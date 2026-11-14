// The command field: one input for Skrzynka, the quick-capture window and the
// "Nowe…" row of ⌘K. The text is read by the backend (montybrain_core::command:
// `commandPreview`); nothing here parses it. Under the field: one fixed-height
// line with the live reading (a neutral hint while a word is still being
// typed), then one list (suggestions for the word under the caret, or the
// choices of the reading) with the matched letters marked, warnings and the
// duplicate hint; the plan on the confirm step and, after a write, the echo of
// the exact lines. In Skrzynka the list floats over the entries, so nothing
// below moves while typing; the last reading stays on screen (dimmed) until
// the next one arrives, so nothing blinks.
//
// Enter on plain capture saves at once; anything else shows the plan first and
// writes on the second Enter (Esc goes back). A write is undoable from the
// toast ("Cofnij") and with `u` in Skrzynka.

import { forwardRef, useCallback, useEffect, useId, useImperativeHandle, useRef, useState, type ReactNode } from "react";

import { api } from "../api";
import type { AgentAsk, CommandApplied, CommandPlan, LineWritten, Span, TaskLocation } from "../api/types";
import { describeError } from "../lib/errors";
import { describeUndo } from "../lib/undo";
import { activeIndex, currentReading, fieldList, initialField, readingShown, stepField, type FieldEffect, type FieldEvent, type FieldState, type ListItem } from "../lib/fieldMachine";
import { useOptionalVault } from "../store/vault";
import "./command.css";

export type FieldVariant = "inbox" | "quick" | "palette";

export interface CommandFieldHandle {
  /** Replaces the text (caret at `caret` or the end); `submit` acts as Enter once the reading arrives. */
  setText(text: string, opts?: { caret?: number; submit?: boolean; focus?: boolean }): void;
  focus(): void;
  text(): string;
}

export interface Written {
  reading: string;
  echo: LineWritten[];
  undo: number | null;
}

/** Which field a write came from: undo with `u` in Skrzynka takes back only its own. */
export type FieldSource = "inbox" | "quick" | "palette";

const ECHO_MS = 6000;
const DEBOUNCE_MS = 60;

/**
 * A write that failed and could not be fully taken back (rare: plans are all
 * or nothing): what is still written, why, and a way to undo it.
 */
export function Partial({ applied, onUndo }: { applied: CommandApplied; total: number; onUndo?: () => void }) {
  const left = applied.left_written ?? [];
  return (
    <div className="cf-partial" role="alert">
      <p className="cf-partial-head">Zapis się nie udał: {applied.error}</p>
      {left.length > 0 ? (
        <>
          <p className="cf-partial-foot">Nie wszystko dało się cofnąć. Zostało zapisane:</p>
          <ul className="cf-partial-list done" aria-label="Zostało zapisane">
            {left.map((x, i) => (
              <li key={i}>✓ {x}</li>
            ))}
          </ul>
        </>
      ) : (
        <p className="cf-partial-foot">Nic nie zostało zapisane.</p>
      )}
      {onUndo && left.length > 0 && (
        <button type="button" className="cf-link" onClick={onUndo}>
          Cofnij to, co zostało
        </button>
      )}
      <p className="cf-partial-foot">Tekst został w polu.</p>
    </div>
  );
}

/** The exact Markdown lines of a plan (or an echo), monospaced. */
function Lines({ lines, muted }: { lines: string[]; muted?: boolean }) {
  return <pre className={`cf-lines${muted ? " muted" : ""}`}>{lines.join("\n")}</pre>;
}

export function PlanView({ plan, compact }: { plan: CommandPlan; compact?: boolean }) {
  const steps = compact ? plan.steps.slice(0, 3) : plan.steps;
  return (
    <ol className="cf-plan" aria-label="Plan zapisu">
      {steps.map((s, i) => (
        <li key={i} className="cf-step">
          <p className="cf-step-summary">{s.summary}</p>
          <p className="cf-file">{s.file}</p>
          {s.before && s.before.length > 0 && <Lines lines={s.before} muted />}
          <Lines lines={s.lines} />
        </li>
      ))}
      {compact && plan.steps.length > steps.length && <li className="cf-more">… i jeszcze {plan.steps.length - steps.length}</li>}
    </ol>
  );
}

export function Echo({ echo, onUndo }: { echo: LineWritten[]; onUndo?: () => void }) {
  if (echo.length === 0) return null;
  const files = [...new Set(echo.map((e) => e.file))];
  return (
    <div className="cf-echo" role="status">
      <p className="cf-echo-head">
        Zapisano w {files.map((f, i) => (
          <span key={f}>
            {i > 0 && ", "}
            <span className="cf-file-inline">{f}</span>
          </span>
        ))}
      </p>
      <Lines lines={echo.map((e) => e.text)} />
      {onUndo && (
        <button type="button" className="cf-link cf-echo-undo" onClick={onUndo}>
          Cofnij
        </button>
      )}
    </div>
  );
}

/** `text` with the `marks` ranges (UTF-16) marked: bold, in the accent colour. */
export function Marked({ text, marks }: { text: string; marks?: Span[] }) {
  if (!marks || marks.length === 0) return <>{text}</>;
  const out: ReactNode[] = [];
  let at = 0;
  marks.forEach((m, i) => {
    const from = Math.max(at, Math.min(m.from, text.length));
    const to = Math.min(m.to, text.length);
    if (to <= from) return;
    if (from > at) out.push(text.slice(at, from));
    out.push(
      <mark key={i} className="cf-mark">
        {text.slice(from, to)}
      </mark>,
    );
    at = to;
  });
  if (at < text.length) out.push(text.slice(at));
  return <>{out}</>;
}

const KIND_TAG: Record<string, string> = { topic: "temat", task: "zadanie", project: "projekt", inbox: "skrzynka" };

function ListRow({ item, index, active, id, onHover, onPick }: { item: ListItem; index: number; active: boolean; id: string; onHover: (i: number) => void; onPick: (i: number) => void }) {
  const tag = item.typo ? "literówka" : KIND_TAG[item.kind];
  return (
    <li
      id={id}
      data-row={index}
      role="option"
      aria-selected={active}
      className={`cf-opt${active ? " active" : ""}`}
      // Only a real mouse move highlights: a list appearing under a resting pointer does not.
      onMouseMove={() => onHover(index)}
      onMouseDown={(e) => {
        e.preventDefault();
        onPick(index);
      }}
    >
      {tag && <span className={`cf-kind kind-${item.typo ? "typo" : item.kind}`}>{tag}</span>}
      <span className="cf-opt-label">
        <Marked text={item.label} marks={item.marks} />
      </span>
      {item.detail && (
        <span className="cf-opt-detail">
          <Marked text={item.detail} marks={item.detailMarks} />
        </span>
      )}
    </li>
  );
}

export const CommandField = forwardRef<
  CommandFieldHandle,
  {
    variant?: FieldVariant;
    /** Which field this is (undo scope). */
    source?: FieldSource;
    /** Where a plain new task goes (default: the inbox). */
    location?: TaskLocation | null;
    selection?: string[];
    /** The focused entry (Skrzynka): the subject of `/czeka …` when nothing is selected. */
    focus?: string | null;
    placeholder?: string;
    autoFocus?: boolean;
    initialText?: string;
    /** Act as Enter once, as soon as the first reading arrives. */
    autoSubmit?: boolean;
    /** Start on the confirm step (mock URL states for screenshots). */
    startConfirmed?: boolean;
    /** Show this echo at first (mock URL states). */
    initialEcho?: LineWritten[];
    /** Show this partial write at first (mock URL states). */
    initialPartial?: { applied: CommandApplied; total: number; undo?: number | null };
    /** A line above the list, e.g. the agent's reason for an inserted command. */
    note?: ReactNode;
    /** Controls at the end of the input row (Skrzynka: the Komenda | Agent switch). */
    controls?: ReactNode;
    onEscape?: () => void;
    /** A complete write (not after a partial one: the field then keeps its text). */
    onWritten?: (w: Written) => void;
    /** A `?…` request (only Skrzynka runs the agent). */
    onAgent?: (ask: AgentAsk, text: string) => void;
    /** ArrowDown with nothing to complete (Skrzynka: into the list). */
    onArrowDown?: () => void;
    onTextChange?: (text: string) => void;
    /** Keys the field does not use (Skrzynka: ⌘J for the agent). */
    onKeyDownExtra?: (e: React.KeyboardEvent<HTMLInputElement>) => void;
    disabled?: boolean;
  }
>(function CommandField(
  {
    variant = "inbox",
    source = "inbox",
    location = null,
    selection = [],
    focus = null,
    placeholder = "Dodaj zadanie albo wpisz komendę…",
    autoFocus,
    initialText = "",
    autoSubmit,
    startConfirmed,
    initialEcho,
    initialPartial,
    note,
    controls,
    onEscape,
    onWritten,
    onAgent,
    onArrowDown,
    onTextChange,
    onKeyDownExtra,
    disabled,
  },
  ref,
) {
  // The quick-capture window has no vault context: it shows the echo itself.
  const vault = useOptionalVault();
  const notify = vault?.notify ?? (() => {});
  const version = vault?.view?.version ?? 0;
  const [fs, setFs] = useState<FieldState>(() => initialField(initialText, startConfirmed ? "confirm" : "edit"));
  const fsRef = useRef(fs);
  const [message, setMessage] = useState<string | null>(null);
  const [echo, setEcho] = useState<LineWritten[] | null>(initialEcho ?? null);
  const [partial, setPartial] = useState<{ applied: CommandApplied; total: number; undo?: number | null } | null>(initialPartial ?? null);
  // The result of an undo started here, with a retry when some of it failed.
  const [undoState, setUndoState] = useState<{ text: string; retry: number | null } | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const timer = useRef<number | undefined>(undefined);
  const echoTimer = useRef<number | undefined>(undefined);
  const autoSubmitPending = useRef(!!autoSubmit);
  const readingId = useId();
  const listId = useId();
  const selKey = selection.join(",");
  const locKey = JSON.stringify(location);
  const ctx = useRef({ selection, focus, location, source, onAgent, onEscape, onArrowDown, onWritten, onTextChange });
  ctx.current = { selection, focus, location, source, onAgent, onEscape, onArrowDown, onWritten, onTextChange };

  const fetchNow = useCallback((rev: number, text: string, caret: number) => {
    const c = ctx.current;
    api
      .commandPreview({ text, caret, selection: c.selection, focus: c.focus, location: c.location, source: c.source })
      .then((r) => dispatch({ type: "reading", rev, text, preview: r.preview, completions: r.completions }))
      .catch((e) => setMessage(describeError(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function run(effects: FieldEffect[]) {
    for (const ef of effects) {
      switch (ef.kind) {
        case "fetch":
          window.clearTimeout(timer.current);
          if (ef.now) fetchNow(ef.rev, ef.text, ef.caret);
          else timer.current = window.setTimeout(() => fetchNow(ef.rev, ef.text, ef.caret), DEBOUNCE_MS);
          break;
        case "apply":
          void apply(ef.plan, ef.text);
          break;
        case "agent":
          if (ctx.current.onAgent) ctx.current.onAgent(ef.ask, ef.text);
          else setMessage("Prośby do agenta działają w Skrzynce.");
          break;
        case "problem":
          setMessage(ef.message);
          break;
        case "escape":
          ctx.current.onEscape?.();
          break;
        case "arrowOut":
          ctx.current.onArrowDown?.();
          break;
        case "caret":
          requestAnimationFrame(() => {
            inputRef.current?.focus();
            inputRef.current?.setSelectionRange(ef.caret, ef.caret);
          });
          break;
      }
    }
  }

  function dispatch(e: FieldEvent) {
    const prev = fsRef.current;
    const { state, effects } = stepField(prev, e);
    fsRef.current = state;
    setFs(state);
    if (state.text !== prev.text) {
      setMessage(null);
      setPartial(null);
      setUndoState(null);
      ctx.current.onTextChange?.(state.text);
    }
    run(effects);
  }

  // First reading, and again when the selection, the focused entry, the place or the vault change.
  useEffect(() => {
    const s = fsRef.current;
    fetchNow(s.rev, s.text, s.caret);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selKey, focus, locKey, fs.phase === "edit" ? version : 0]);

  useEffect(() => () => {
    window.clearTimeout(timer.current);
    window.clearTimeout(echoTimer.current);
  }, []);

  // `autoSubmit` (the ⌘K row): Enter once, as soon as there is a reading
  // (not while a word is still being typed: that Enter would take a suggestion).
  useEffect(() => {
    const r = currentReading(fs);
    if (autoSubmitPending.current && r) {
      autoSubmitPending.current = false;
      if (r.state !== "typing") dispatch({ type: "enter" });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fs.preview]);

  useImperativeHandle(
    ref,
    () => ({
      setText(t, opts) {
        const at = opts?.caret ?? t.length;
        dispatch({ type: "edit", text: t, caret: at });
        if (opts?.submit) dispatch({ type: "enter" });
        if (opts?.focus !== false) {
          requestAnimationFrame(() => {
            const el = inputRef.current;
            if (!el) return;
            el.focus();
            el.setSelectionRange(at, at);
          });
        }
      },
      focus: () => inputRef.current?.focus(),
      text: () => fsRef.current.text,
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  async function apply(plan: CommandPlan, text: string) {
    setMessage(null);
    setPartial(null);
    const c = ctx.current;
    try {
      const r = await api.commandApply({ text, caret: null, selection: c.selection, focus: c.focus, location: c.location, source: c.source }, plan);
      if (r.kind === "written") {
        const token = r.undo;
        if (r.applied.error) {
          // Failed and the rollback could not finish: keep the text, say what is still written.
          setPartial({ applied: r.applied, total: plan.steps.length, undo: token });
          setEcho(null);
          dispatch({ type: "failed" });
          notify(`Zapis się nie udał i nie wszystko dało się cofnąć`, token !== null ? { label: "Cofnij", run: () => void undoWrite(token) } : undefined);
          return;
        }
        setEcho(r.applied.echo);
        window.clearTimeout(echoTimer.current);
        echoTimer.current = window.setTimeout(() => setEcho(null), ECHO_MS);
        dispatch({ type: "written" });
        notify(`Zapisano: ${r.reading}`, token !== null ? { label: "Cofnij", run: () => void undoWrite(token) } : undefined);
        c.onWritten?.({ reading: r.reading, echo: r.applied.echo, undo: token });
      } else if (r.kind === "stale") {
        dispatch({ type: "stale", preview: r.preview });
        setMessage(r.message);
      } else {
        if (r.preview) dispatch({ type: "stale", preview: r.preview });
        dispatch({ type: "failed" });
        setMessage(r.message);
      }
    } catch (e) {
      dispatch({ type: "failed", keepConfirm: true });
      setMessage(describeError(e));
    }
  }

  async function undoWrite(token: number | null) {
    try {
      const r = await api.commandUndo(token, null);
      const { text, retry } = describeUndo(r, token);
      notify(text, retry !== null ? { label: "Spróbuj cofnąć jeszcze raz", run: () => void undoWrite(retry) } : undefined);
      setUndoState({ text, retry });
      setEcho(null);
      if (retry === null) setPartial(null);
    } catch (e) {
      notify(describeError(e));
      setUndoState({ text: describeError(e), retry: token });
    }
  }

  const list = fieldList(fs);
  const active = activeIndex(fs);
  const interactive = !!list && !list.stale;

  // Keep the highlighted row in view.
  useEffect(() => {
    if (active === null) return;
    listRef.current?.querySelector<HTMLElement>(`[data-row="${active}"]`)?.scrollIntoView?.({ block: "nearest" });
  }, [active]);

  function onKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.nativeEvent.isComposing) return;
    onKeyDownExtra?.(e);
    if (e.defaultPrevented) return;
    switch (e.key) {
      case "Enter":
        e.preventDefault();
        if (!disabled) dispatch({ type: "enter" });
        return;
      case "Tab":
        if (list && !e.shiftKey) {
          e.preventDefault();
          dispatch({ type: "tab" });
        }
        return;
      case "ArrowDown":
      case "ArrowUp":
        if (list || (e.key === "ArrowDown" && ctx.current.onArrowDown)) {
          e.preventDefault();
          dispatch({ type: "arrow", dir: e.key === "ArrowDown" ? 1 : -1 });
        }
        return;
      case "Escape": {
        const s = fsRef.current;
        if (list || s.phase === "confirm" || message || echo || partial || ctx.current.onEscape) {
          e.preventDefault();
          e.stopPropagation();
        }
        if (!list && s.phase !== "confirm" && (message || echo || partial)) {
          setMessage(null);
          setEcho(null);
          setPartial(null);
          return;
        }
        dispatch({ type: "escape" });
        return;
      }
    }
  }

  const shownR = readingShown(fs);
  const shown = shownR?.preview ?? null;
  const stale = shownR?.stale ?? false;
  const plan = shown?.plan ?? null;
  const hint = (() => {
    if (fs.busy) return "Zapisuję…";
    if (fs.waiting) return "Czytam…";
    if (fs.phase === "confirm") return "↵ zapisz · esc wróć do edycji";
    if (interactive && active !== null) return "↵ wstaw · ↑↓ wybór · esc zamknij";
    if (interactive) return "Tab wstawia · ↑↓ wybór";
    switch (shown?.state) {
      case "capture":
        return "↵ zapisz";
      case "confirm":
        return "↵ pokaż plan";
      case "agent":
        return onAgent ? "↵ zapytaj agenta" : null;
      default:
        return null;
    }
  })();
  const readingText = (() => {
    if (fs.phase === "confirm" && plan)
      return `Plan zapisu: ${plan.steps.length === 1 ? "1 zmiana" : `${plan.steps.length} ${plan.steps.length < 5 ? "zmiany" : "zmian"}`}${shown?.typo ? " (z poprawioną literówką)" : ""}. Sprawdź i potwierdź.`;
    if (!shown || shown.state === "empty" || !fs.text.trim()) return null;
    return shown.reading;
  })();
  const idle = variant === "quick" ? "↵ zapisz · > miejsce · @termin · !priorytet" : "↵ zapisz · > miejsce · / komendy · @termin · !priorytet";
  const extraProblems = shown && shown.state === "invalid" && !stale ? shown.problems.slice(1) : [];
  const warnings = shown && fs.text.trim() ? shown.warnings : [];
  const similar = shown && fs.phase === "edit" && fs.text.trim() && shown.state !== "invalid" && shown.state !== "typing" ? shown.similar[0] : undefined;
  const hasPanel = !!note || !!list || extraProblems.length > 0 || warnings.length > 0 || !!similar;

  function linkSimilar(id: string) {
    const t = fsRef.current.text.trimEnd();
    // `[ ]` first keeps it a new task (plain words before /powiaz would name an existing one).
    const text = `${/^\[[ /xX!]\]\s/.test(t) ? "" : "[ ] "}${t} /powiaz ^${id}`;
    dispatch({ type: "edit", text, caret: text.length });
    run([{ kind: "caret", caret: text.length }]);
  }

  return (
    <div className={`cf cf-${variant}${fs.phase === "confirm" ? " confirming" : ""}`}>
      <div className="cf-row">
        {variant !== "quick" && (
          <span className="cf-lead" aria-hidden>
            {fs.phase === "confirm" ? "↵" : "+"}
          </span>
        )}
        <input
          ref={inputRef}
          className="cf-input"
          value={fs.text}
          placeholder={placeholder}
          aria-label="Nowe zadanie albo komenda"
          aria-describedby={readingId}
          role="combobox"
          aria-expanded={!!list}
          aria-controls={list ? listId : undefined}
          aria-activedescendant={interactive && active !== null ? `${listId}-${active}` : undefined}
          aria-autocomplete="list"
          spellCheck={false}
          autoComplete="off"
          autoFocus={autoFocus}
          disabled={disabled || fs.busy}
          onChange={(e) => dispatch({ type: "edit", text: e.target.value, caret: e.target.selectionStart ?? e.target.value.length })}
          onSelect={(e) => dispatch({ type: "caret", caret: (e.target as HTMLInputElement).selectionStart ?? fsRef.current.text.length })}
          onKeyDown={onKeyDown}
        />
        {controls}
      </div>
      <div className="cf-under">
        {/* One line, always there: the reading, a neutral hint while typing, or what went wrong. */}
        <p
          id={readingId}
          className={`cf-reading${message ? " state-message" : readingText ? ` state-${shown!.state}` : " idle"}${stale && !message ? " stale" : ""}`}
          aria-live="polite"
          role={message ? "alert" : undefined}
        >
          <span className="cf-reading-text" title={message ?? readingText ?? undefined}>
            {message ?? readingText ?? idle}
          </span>
          {hint && !message && <span className="cf-hint">{hint}</span>}
        </p>
        {hasPanel && (
          <div className={`cf-panel${variant === "inbox" ? " floating" : ""}`}>
            {note && <div className="cf-note">{note}</div>}
            {list && (
              <ul
                ref={listRef}
                id={listId}
                className={`cf-complete${list.stale ? " stale" : ""}`}
                role="listbox"
                aria-label={list.source === "choices" ? (shown?.typo ? "Literówka: poprawka" : "Wybierz jedno") : "Podpowiedzi"}
              >
                {list.items.map((it, i) => (
                  <ListRow
                    key={it.key}
                    item={it}
                    index={i}
                    id={`${listId}-${i}`}
                    active={interactive && i === active}
                    onHover={(n) => interactive && dispatch({ type: "hover", index: n })}
                    onPick={(n) => interactive && dispatch({ type: "pick", index: n })}
                  />
                ))}
                <li className="cf-opt-hint" aria-hidden>
                  ↑↓ wybór · Tab albo ↵ wstawia · esc zamyka
                </li>
              </ul>
            )}
            {extraProblems.length > 0 && (
              <ul className="cf-problems" aria-label="Problemy">
                {extraProblems.map((p, i) => (
                  <li key={i}>{p}</li>
                ))}
              </ul>
            )}
            {warnings.length > 0 && (
              <ul className={`cf-warnings${stale ? " stale" : ""}`}>
                {warnings.map((w, i) => (
                  <li key={i}>{w}</li>
                ))}
              </ul>
            )}
            {similar && (
              <p className={`cf-similar${stale ? " stale" : ""}`}>
                Podobne do <span className="cf-ref">^{similar.id}</span> „{similar.title}” ({similar.place.label}) —{" "}
                <button type="button" className="cf-link" onMouseDown={(e) => e.preventDefault()} onClick={() => linkSimilar(similar.id)}>
                  powiązać?
                </button>
              </p>
            )}
          </div>
        )}
      </div>
      {fs.phase === "confirm" && plan && shown?.state !== "invalid" && <PlanView plan={plan} compact={variant === "quick"} />}
      {partial && (
        <Partial
          applied={partial.applied}
          total={partial.total}
          onUndo={partial.undo !== undefined && partial.undo !== null ? () => void undoWrite(partial.undo!) : undefined}
        />
      )}
      {undoState && (
        <p className="cf-undo-state" role="status">
          {undoState.text}
          {undoState.retry !== null && (
            <>
              {" "}
              <button type="button" className="cf-link" onClick={() => void undoWrite(undoState.retry)}>
                Spróbuj cofnąć jeszcze raz
              </button>
            </>
          )}
        </p>
      )}
      {echo && <Echo echo={echo} />}
    </div>
  );
});
