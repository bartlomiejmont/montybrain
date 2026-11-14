// „Aktualizacja": the update strip under the project header. Starts the
// read-only `aktualizacja` recipe for this project ("Sprawdź projekt") or a
// free-form request (`zlecenie`, the input under the bar), shows its progress,
// the proposed changes for review (the app writes the accepted ones, with a
// guarded undo), a one-line receipt once everything is decided, and the opt-in
// automatic update. Other actions for the project are in its menu.
// State: store/updates.ts (backend: src-tauri/src/updates/).

import { ChevronDown, ChevronRight, MoreHorizontal, RefreshCw } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";

import {
  api,
  type ChangeEdit,
  type AutoUpdate,
  type Project,
  type ProjectUpdateView,
  type ProviderId,
  type ProviderInfo,
  type RunMode,
  type StoredChange,
  type StoredProposal,
  type TaskStatus,
  type TopicStatus,
  type UpdateSchedule,
  type UpdateSources,
} from "../../api";
import { Menu, type MenuItem } from "../../components/Menu";
import { Button, Segmented } from "../../components/ui";
import { formatElapsed, pickProvider, runElapsedMs } from "../../lib/actions";
import { describeError } from "../../lib/errors";
import { TASK_STATUSES, TASK_STATUS_LABEL, TOPIC_STATUSES, TOPIC_STATUS_LABEL } from "../../lib/labels";
import {
  HISTORY_CONSENT,
  HISTORY_OFF_NOTE,
  OUTCOME_LABEL,
  OUTCOME_MARK,
  PROVIDER_LABEL,
  REQUEST_RECIPE,
  UPDATE_RECIPE,
  applySummary,
  counts,
  decidedLine,
  defaultSelection,
  describeChange,
  formatNext,
  formatWhen,
  groupChanges,
  heldText,
  historyLine,
  nextSelection,
  progressText,
  receiptLine,
  requestNote,
  historyToggle,
  hourChoices,
  panelLabel,
  plural,
  recheckQuestion,
  rejectSummary,
  restoreSummary,
  reviewHeading,
  reviewStatus,
  runningStatus,
  secondaryLabel,
  statusLine,
  stripState,
  undoSummary,
  updateRunFor,
} from "../../lib/updates";
import { navigate } from "../../router/router";
import { readPref, writePref } from "../../store/ui";
import { runStore, useRuns } from "../../store/runs";
import { updateStore, useProviders, useUpdates } from "../../store/updates";
import { useVault, useView } from "../../store/vault";
import { Markdown } from "../actions/Markdown";
import { driftLine, driftRequest, similarTopic } from "../../lib/drift";
import { driftStore, useDrift } from "../../store/signals";
import { presetAction } from "../actions/preset";
import "./update.css";

const PROVIDER_PREF = "montybrain.aktualizacja.provider";
const DEFAULT_AUTO: AutoUpdate = { schedule: { kind: "off" }, provider: "claude", on_result: "propose" };
const DEFAULT_SOURCES: UpdateSources = { history: false, history_days: 14 };
/** Typed requests per project: survive navigating away and back (module scope, like Akcje's form). */
const requestMemory = new Map<string, string>();

/** Re-renders every second while `on`. */
function useNow(on: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!on) return;
    setNow(Date.now());
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [on]);
  return now;
}

function Disclosure({ label, open, onToggle, controls, className }: { label: string; open: boolean; onToggle: () => void; controls: string; className?: string }) {
  return (
    <button type="button" className={`upd-disclosure ${className ?? ""}`} aria-expanded={open} aria-controls={controls} onClick={onToggle}>
      <ChevronRight size={12} className="chev" aria-hidden />
      {label}
    </button>
  );
}

/** Inline two-step confirmation (the app's ConfirmButton pattern, with a longer question). */
function InlineConfirm({ question, confirmLabel, onConfirm, onCancel, danger }: { question: ReactNode; confirmLabel: string; onConfirm: () => void; onCancel: () => void; danger?: boolean }) {
  const ref = useRef<HTMLButtonElement>(null);
  useEffect(() => ref.current?.focus(), []);
  return (
    <div className="upd-confirm" role="group" aria-label="Potwierdzenie" onKeyDown={(e) => e.key === "Escape" && (e.stopPropagation(), onCancel())}>
      <p className="upd-confirm-q">{question}</p>
      <div className="upd-confirm-actions">
        <Button ref={ref} variant={danger ? "danger" : "ghost"} onClick={onConfirm}>
          {confirmLabel}
        </Button>
        <Button variant="quiet" onClick={onCancel}>
          Anuluj
        </Button>
      </div>
    </div>
  );
}

function AutoPanel({
  project,
  view,
  providers,
  ready,
  focusHistory = false,
  onClose,
}: {
  project: Project;
  view: ProjectUpdateView | null;
  providers: ProviderInfo[] | null;
  ready: boolean;
  /** Opened from "Włącz historię": focus goes to the history toggle. */
  focusHistory?: boolean;
  onClose: () => void;
}) {
  const { notify } = useVault();
  const cfg = view?.auto ?? DEFAULT_AUTO;
  const src = view?.sources ?? DEFAULT_SOURCES;
  const [busy, setBusy] = useState(false);
  const [confirmApply, setConfirmApply] = useState(false);
  const [confirmHistory, setConfirmHistory] = useState(false);
  const saveSources = async (next: UpdateSources) => {
    setBusy(true);
    try {
      updateStore.set(await api.setUpdateSources(project.slug, next));
    } catch (e) {
      notify(describeError(e));
    } finally {
      setBusy(false);
    }
  };
  const ref = useRef<HTMLDivElement>(null);
  const historyRow = useRef<HTMLDivElement>(null);
  const resultRow = useRef<HTMLDivElement>(null);
  const focused = useRef(false);
  /** Focus back on the control that opened a confirmation (its checked option, else its first). */
  const focusIn = (row: RefObject<HTMLDivElement | null>, prefer?: string) =>
    requestAnimationFrame(() => {
      const el = row.current;
      (
        (prefer ? el?.querySelector<HTMLElement>(prefer) : null) ??
        el?.querySelector<HTMLElement>("button[aria-checked='true']:not(:disabled)") ??
        el?.querySelector<HTMLElement>("button:not(:disabled)")
      )?.focus();
    });
  // Focus moves into the panel once its data is there (opened from Settings,
  // the list may still be loading).
  useEffect(() => {
    if (!ready || focused.current) return;
    focused.current = true;
    if (focusHistory) {
      focusIn(historyRow, "button[aria-checked='false']:not(:disabled)");
      return;
    }
    const el = ref.current;
    (el?.querySelector<HTMLElement>("button[aria-checked='true']:not(:disabled)") ?? el?.querySelector<HTMLElement>("button:not(:disabled)"))?.focus();
  }, [ready]);
  const save = async (next: AutoUpdate) => {
    setBusy(true);
    try {
      updateStore.set(await api.setAutoUpdate(project.slug, next));
    } catch (e) {
      notify(describeError(e));
    } finally {
      setBusy(false);
    }
  };
  const s = cfg.schedule;
  const setSchedule = (schedule: UpdateSchedule) => void save({ ...cfg, schedule });
  const saved = providers?.find((p) => p.id === cfg.provider);
  const savedMissing = cfg.schedule.kind !== "off" && saved && !saved.available;
  const id = `upd-auto-${project.slug}`;
  return (
    <div
      ref={ref}
      className="upd-auto"
      id={id}
      role="group"
      aria-label="Źródła i automatyczna aktualizacja"
      onKeyDown={(e) => {
        if (e.key === "Escape" && !confirmApply && !confirmHistory) {
          e.stopPropagation();
          onClose();
        }
      }}
    >
      <h3 className="upd-panel-head" id={`upd-src-${project.slug}`}>
        Źródła
      </h3>
      <div className="upd-auto-row">
        <span className="upd-auto-label">Repozytoria</span>
        <div className="upd-auto-controls">
          <span className="upd-auto-static">zawsze: commity, gałęzie, drzewa robocze</span>
        </div>
      </div>
      <div className="upd-auto-row" ref={historyRow}>
        <span className="upd-auto-label">Historia rozmów</span>
        <div className="upd-auto-controls">
          <Segmented<"off" | "on">
            label="Historia rozmów (Claude Code, Codex)"
            value={src.history ? "on" : "off"}
            disabled={busy || confirmHistory}
            onChange={(v) => {
              const t = historyToggle(src, v === "on");
              if (t.ask) setConfirmHistory(true);
              if (t.save) void saveSources(t.save);
            }}
            options={[
              { value: "off", label: "Wyłączona" },
              { value: "on", label: "Claude Code, Codex" },
            ]}
          />
          {src.history && (
            <select
              className="field select upd-select"
              aria-label="Okno historii rozmów"
              value={src.history_days}
              disabled={busy}
              onChange={(e) => void saveSources({ ...src, history_days: Number(e.target.value) })}
            >
              {[7, 14, 30].map((d) => (
                <option key={d} value={d}>
                  ostatnie {d} dni
                </option>
              ))}
            </select>
          )}
          {src.history && !confirmHistory && (
            <p className="upd-auto-warn">Polecenia z sesji w repozytoriach projektu idą do agenta razem ze stanem repozytoriów, po wycięciu fragmentów wyglądających na sekrety.</p>
          )}
        </div>
      </div>
      {confirmHistory && (
        <InlineConfirm
          question={HISTORY_CONSENT}
          confirmLabel="Włącz historię"
          onConfirm={() => {
            setConfirmHistory(false);
            const t = historyToggle(src, true, true);
            // Back on the toggle once it is enabled again (saving disables it).
            if (t.save) void saveSources(t.save).then(() => focusIn(historyRow));
          }}
          onCancel={() => {
            setConfirmHistory(false);
            focusIn(historyRow);
          }}
        />
      )}
      <h3 className="upd-panel-head">Automatycznie</h3>
      <p className="upd-auto-note">Każde sprawdzenie zużywa limit wybranej subskrypcji. Działa tylko, gdy MontyBrain jest uruchomiony.</p>
      <div className="upd-auto-row">
        <span className="upd-auto-label">Harmonogram</span>
        <div className="upd-auto-controls">
          <Segmented<UpdateSchedule["kind"]>
            label="Harmonogram"
            value={s.kind}
            disabled={busy}
            onChange={(k) => setSchedule(k === "off" ? { kind: "off" } : k === "every" ? { kind: "every", hours: 6 } : { kind: "daily", hour: 9 })}
            options={[
              { value: "off", label: "Wyłączona" },
              { value: "every", label: "Co kilka godzin" },
              { value: "daily", label: "Codziennie" },
            ]}
          />
          {s.kind === "every" && (
            <select className="field select upd-select" aria-label="Odstęp między sprawdzeniami" value={s.hours} disabled={busy} onChange={(e) => setSchedule({ kind: "every", hours: Number(e.target.value) })}>
              {hourChoices(s.hours).map((h) => (
                <option key={h} value={h}>
                  co {h} h
                </option>
              ))}
            </select>
          )}
          {s.kind === "daily" && (
            <select className="field select upd-select" aria-label="Godzina sprawdzenia" value={s.hour} disabled={busy} onChange={(e) => setSchedule({ kind: "daily", hour: Number(e.target.value) })}>
              {Array.from({ length: 24 }, (_, h) => (
                <option key={h} value={h}>
                  {String(h).padStart(2, "0")}:00
                </option>
              ))}
            </select>
          )}
        </div>
      </div>
      <div className="upd-auto-row">
        <span className="upd-auto-label">Subskrypcja</span>
        <div className="upd-auto-controls">
          <Segmented<ProviderId>
            label="Subskrypcja automatycznej aktualizacji"
            value={cfg.provider}
            disabled={busy}
            onChange={(provider) => void save({ ...cfg, provider })}
            options={(providers ?? []).map((p) => ({
              value: p.id,
              label: PROVIDER_LABEL[p.id],
              disabled: !p.available,
              hint: p.available ? undefined : (p.problem ?? "Niedostępny"),
            }))}
          />
          {savedMissing && (
            <p className="upd-auto-warn" role="status">
              {PROVIDER_LABEL[cfg.provider]} jest niedostępny — zainstaluj CLI lub wybierz inną subskrypcję.
            </p>
          )}
        </div>
      </div>
      <div className="upd-auto-row" ref={resultRow}>
        <span className="upd-auto-label">Wynik</span>
        <div className="upd-auto-controls">
          <Segmented<AutoUpdate["on_result"]>
            label="Co zrobić z wynikiem"
            value={cfg.on_result}
            disabled={busy || confirmApply}
            onChange={(on_result) => (on_result === "apply" ? setConfirmApply(true) : void save({ ...cfg, on_result }))}
            options={[
              { value: "propose", label: "Pokaż propozycje" },
              { value: "apply", label: "Zapisuj bez pytania" },
            ]}
          />
          {cfg.on_result === "apply" && !confirmApply && <p className="upd-auto-warn">Zmiany trafią do zadań i dziennika, zanim je zobaczysz.</p>}
        </div>
      </div>
      {confirmApply && (
        <InlineConfirm
          danger
          question="Zapisywać zmiany bez pytania? Agent będzie sam zmieniać zadania i tematy w tym projekcie. Cofnięcie będzie ręczne."
          confirmLabel="Zapisuj bez pytania"
          onConfirm={() => {
            setConfirmApply(false);
            void save({ ...cfg, on_result: "apply" }).then(() => focusIn(resultRow));
          }}
          onCancel={() => {
            setConfirmApply(false);
            focusIn(resultRow);
          }}
        />
      )}
      {view?.held ? (
        <p className="upd-auto-next">{heldText(view.pending)}</p>
      ) : (
        view?.next_due && <p className="upd-auto-next">Następne sprawdzenie: {formatNext(view.next_due, new Date())}.</p>
      )}
    </div>
  );
}

/** Inline correction of one undecided change: only what that kind writes; saved through `edit_update`. */
function ChangeEditor({ sc, index, runId, onDone }: { sc: StoredChange; index: number; runId: string; onDone: (saved: boolean) => void }) {
  const c = sc.change;
  const { notify } = useVault();
  const [title, setTitle] = useState(c.kind === "new_topic" || c.kind === "new_task" ? c.title : "");
  const [topicStatus, setTopicStatus] = useState<TopicStatus | "">(c.kind === "new_topic" ? c.status : c.kind === "topic_log" ? (c.status ?? "") : "");
  const [summary, setSummary] = useState(c.kind === "new_topic" ? c.summary : c.kind === "topic_log" ? (c.summary ?? c.base_summary) : "");
  const [text, setText] = useState(c.kind === "new_topic" || c.kind === "topic_log" ? c.text : "");
  const [tasks, setTasks] = useState(c.kind === "new_topic" ? c.tasks.map((t) => ({ keep: true, title: t.title, status: t.status })) : []);
  const setTask = (k: number, patch: Partial<(typeof tasks)[number]>) => setTasks(tasks.map((t, j) => (j === k ? { ...t, ...patch } : t)));
  const [taskStatus, setTaskStatus] = useState<TaskStatus>(c.kind === "task_status" ? c.to : "todo");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const first = useRef<HTMLInputElement & HTMLSelectElement>(null);
  useEffect(() => first.current?.focus(), []);
  const id = `upd-e-${runId}-${index}`;

  const edit = (): ChangeEdit => {
    switch (c.kind) {
      case "new_topic":
        return { kind: "new_topic", title, status: (topicStatus || c.status) as TopicStatus, summary, text, tasks };
      case "topic_log":
        return { kind: "topic_log", status: topicStatus || null, summary, text };
      case "task_status":
        return { kind: "task_status", status: taskStatus };
      case "new_task":
        return { kind: "new_task", title };
    }
  };
  async function save() {
    setBusy(true);
    setError(null);
    try {
      updateStore.set(await api.editUpdate(runId, index, edit()));
      notify("Zapisano poprawkę propozycji.");
      onDone(true);
    } catch (e) {
      setError(describeError(e));
      setBusy(false);
    }
  }
  const field = (label: string, key: string, control: ReactNode) => (
    <div className="upd-edit-field">
      <label className="upd-edit-label" htmlFor={`${id}-${key}`}>
        {label}
      </label>
      {control}
    </div>
  );
  return (
    <form
      className="upd-edit"
      aria-label="Popraw propozycję"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onDone(false);
        }
      }}
    >
      {(c.kind === "new_topic" || c.kind === "new_task") &&
        field("Tytuł", "title", <input ref={first} id={`${id}-title`} className="field" value={title} onChange={(e) => setTitle(e.target.value)} />)}
      {(c.kind === "new_topic" || c.kind === "topic_log") &&
        field(
          "Status tematu",
          "status",
          <select
            ref={c.kind === "topic_log" ? first : undefined}
            id={`${id}-status`}
            className="field select"
            value={topicStatus}
            onChange={(e) => setTopicStatus(e.target.value as TopicStatus | "")}
          >
            {c.kind === "topic_log" && <option value="">bez zmiany ({TOPIC_STATUS_LABEL[c.base_status].toLowerCase()})</option>}
            {TOPIC_STATUSES.map((st) => (
              <option key={st} value={st}>
                {TOPIC_STATUS_LABEL[st]}
              </option>
            ))}
          </select>,
        )}
      {(c.kind === "new_topic" || c.kind === "topic_log") &&
        field("Opis stanu", "summary", <textarea id={`${id}-summary`} className="field" rows={3} value={summary} onChange={(e) => setSummary(e.target.value)} />)}
      {(c.kind === "new_topic" || c.kind === "topic_log") &&
        field(
          c.kind === "new_topic" ? "Pierwszy wpis w dzienniku" : "Wpis w dzienniku",
          "text",
          <textarea id={`${id}-text`} className="field" rows={2} value={text} onChange={(e) => setText(e.target.value)} />,
        )}
      {c.kind === "new_topic" && c.tasks.length > 0 && (
        <fieldset className="upd-edit-tasks">
          <legend className="upd-edit-label">Zadania tematu</legend>
          {tasks.map((t, k) => (
            <div key={k} className={`upd-edit-task${t.keep ? "" : " off"}`}>
              <input type="checkbox" checked={t.keep} aria-label={`Dodaj zadanie ${k + 1}`} onChange={() => setTask(k, { keep: !t.keep })} />
              <input
                className="field"
                aria-label={`Tytuł zadania ${k + 1}`}
                value={t.title}
                disabled={!t.keep}
                onChange={(e) => setTask(k, { title: e.target.value })}
              />
              <select
                className="field select"
                aria-label={`Status zadania ${k + 1}`}
                value={t.status}
                disabled={!t.keep}
                onChange={(e) => setTask(k, { status: e.target.value as TaskStatus })}
              >
                {(["todo", "doing", "done"] as const).map((st) => (
                  <option key={st} value={st}>
                    {TASK_STATUS_LABEL[st]}
                  </option>
                ))}
              </select>
            </div>
          ))}
        </fieldset>
      )}
      {c.kind === "task_status" &&
        field(
          `Status zadania (teraz: ${TASK_STATUS_LABEL[c.from].toLowerCase()})`,
          "task",
          <select ref={first} id={`${id}-task`} className="field select" value={taskStatus} onChange={(e) => setTaskStatus(e.target.value as TaskStatus)}>
            {TASK_STATUSES.filter((st) => st !== c.from).map((st) => (
              <option key={st} value={st}>
                {TASK_STATUS_LABEL[st]}
              </option>
            ))}
          </select>,
        )}
      {error && (
        <p className="upd-edit-error" role="alert">
          {error}
        </p>
      )}
      <div className="upd-edit-actions">
        <Button type="submit" variant="ghost" disabled={busy}>
          {busy ? "Zapisuję…" : "Zapisz poprawkę"}
        </Button>
        <Button variant="quiet" disabled={busy} onClick={() => onDone(false)}>
          Anuluj
        </Button>
      </div>
    </form>
  );
}

/** The proposed topic summary; "Porównaj opis" shows it next to the current one (each once). */
function SummaryCompare({ id, now, after }: { id: string; now: string; after: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="upd-change-summary">
      {!open && (
        <>
          <span className="upd-change-summary-label">Nowy opis tematu</span>
          <p>{after || "(pusty)"}</p>
        </>
      )}
      <Disclosure label={open ? "Zwiń porównanie" : "Porównaj opis"} open={open} onToggle={() => setOpen(!open)} controls={id} />
      {open && (
        <div className="upd-compare" id={id}>
          <div>
            <span className="upd-change-summary-label">Obecnie</span>
            <p>{now || "(pusty)"}</p>
          </div>
          <div>
            <span className="upd-change-summary-label">Po zapisie</span>
            <p>{after || "(pusty)"}</p>
          </div>
        </div>
      )}
    </div>
  );
}

function ChangeRow({
  project,
  sc,
  index,
  runId,
  selected,
  onToggle,
  busy,
  compact = false,
  editable = false,
  proposalTopics = [],
}: {
  project: string;
  /** Topics with a journal entry elsewhere on this proposal. */
  proposalTopics?: string[];
  sc: StoredChange;
  index: number;
  runId: string;
  selected?: boolean;
  onToggle?: () => void;
  busy?: boolean;
  /** A decided row in a partial review: one line, details on demand. */
  compact?: boolean;
  /** Undecided rows of a review can be corrected ("Edytuj"). */
  editable?: boolean;
}) {
  const t = describeChange(sc.change);
  const o = sc.outcome;
  const inputId = `upd-c-${runId}-${index}`;
  const selectable = !o && onToggle;
  const [editing, setEditing] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const editBtn = useRef<HTMLButtonElement>(null);
  const view = useView();
  if (compact && o && !expanded) {
    return (
      <li className={`upd-change is-compact st-${o.status}`}>
        <span className={`upd-mark st-${o.status}`} aria-hidden>
          {OUTCOME_MARK[o.status]}
        </span>
        <p className="upd-compact-line">
          {decidedLine(sc)}
          {sc.edited && <span className="upd-edited"> · zmienione przez ciebie</span>}{" "}
          <button type="button" className="upd-link upd-compact-more" aria-expanded={false} onClick={() => setExpanded(true)}>
            Szczegóły
          </button>
        </p>
      </li>
    );
  }
  const c = sc.change;
  const { notify } = useVault();
  const drift = useDrift();
  const [joinMenu, setJoinMenu] = useState<HTMLElement | null>(null);
  // Topics that already have an entry on this list are not offered (that entry is corrected instead).
  const logged = new Set(proposalTopics);
  const joinTargets = view.snapshot.topics.filter((x) => x.project === project && x.status !== "done" && !logged.has(x.slug));
  const similar = c.kind === "new_topic" ? similarTopic(c, joinTargets, drift.list.find((d) => d.project === project)?.links ?? []) : null;
  async function join(topic: string) {
    try {
      const r = await api.joinUpdate(runId, index, topic);
      updateStore.set(r.updates);
      const title = joinTargets.find((x) => x.slug === topic)?.title ?? topic;
      notify(`Dołączono do tematu „${title}”.${r.skipped.length ? ` Pominięto ${r.skipped.length} (już są w temacie).` : ""}`);
    } catch (e) {
      notify(describeError(e));
    }
  }
  const currentSummary = c.kind === "topic_log" ? (view.snapshot.topics.find((x) => x.project === project && x.slug === c.topic)?.summary ?? c.base_summary) : "";
  const closeEditor = () => {
    setEditing(false);
    requestAnimationFrame(() => editBtn.current?.focus());
  };
  return (
    <li className={`upd-change${t.topic ? " is-topic" : ""}${o ? ` st-${o.status}` : ""}`}>
      {o ? (
        <span className={`upd-mark st-${o.status}`} aria-hidden>
          {OUTCOME_MARK[o.status]}
        </span>
      ) : selectable ? (
        <input id={inputId} type="checkbox" className="upd-check" checked={!!selected} disabled={busy || editing} onChange={onToggle} aria-describedby={`${inputId}-d`} />
      ) : (
        <span className="upd-mark" aria-hidden />
      )}
      <label htmlFor={selectable ? inputId : undefined} className="upd-change-main">
        <span className="upd-change-line">
          <span className="upd-change-kind">{t.kind}</span>
          <span className="upd-change-title">{t.title}</span>
          {t.what && <span className="upd-change-what">{t.what}</span>}
          {sc.edited && <span className="upd-edited">zmienione przez ciebie</span>}
        </span>
      </label>
      {editing ? (
        <div className="upd-change-body">
          <ChangeEditor sc={sc} index={index} runId={runId} onDone={closeEditor} />
        </div>
      ) : (
        <div className="upd-change-body" id={`${inputId}-d`}>
          {sc.change.kind === "new_topic" ? (
            <>
              <div className="upd-change-summary">
                <span className="upd-change-summary-label">Opis stanu</span>
                <p>{t.summary || "(pusty)"}</p>
              </div>
              <div className="upd-change-summary">
                <span className="upd-change-summary-label">{t.detailLabel}</span>
                <p>{t.detail}</p>
              </div>
              {t.tasks.length > 0 && (
                <ul className="upd-topic-tasks" aria-label={`Zadania tematu ${t.title}`}>
                  {t.tasks.map((x, k) => (
                    <li key={k}>
                      <span className="upd-topic-task-title">{x.title}</span> <span className="upd-topic-task-status">{x.status}</span>
                    </li>
                  ))}
                </ul>
              )}
              <p className="upd-change-detail">{sc.change.reason}</p>
            </>
          ) : (
            <>
              <p className="upd-change-detail">{t.detail}</p>
              {t.summary !== null && <SummaryCompare id={`${inputId}-cmp`} now={currentSummary} after={t.summary} />}
            </>
          )}
          {(c.kind === "new_topic" || c.kind === "topic_log") && (c.branches?.length ?? 0) > 0 && (
            <p className="upd-change-detail upd-branches">Gałęzie: {c.branches!.map((b) => `${b.repo}/${b.branch}`).join(", ")}</p>
          )}
          {editable && !o && (
            <span className="upd-row-actions">
              <button ref={editBtn} type="button" className="upd-link upd-edit-btn" disabled={busy} onClick={() => setEditing(true)}>
                Edytuj
              </button>
              {c.kind === "new_topic" && joinTargets.length > 0 && (
                <button
                  type="button"
                  className="upd-link upd-edit-btn"
                  disabled={busy}
                  aria-haspopup="menu"
                  aria-expanded={joinMenu !== null}
                  onClick={(e) => setJoinMenu(joinMenu ? null : e.currentTarget)}
                >
                  Dołącz do tematu…
                </button>
              )}
            </span>
          )}
          {editable && !o && similar && (
            <p className="upd-similar">
              Podobny do: „{similar.title}”.{" "}
              <button type="button" className="upd-link" disabled={busy} onClick={() => void join(similar.slug)}>
                Dołączyć?
              </button>
            </p>
          )}
          {joinMenu && (
            <Menu
              anchor={joinMenu}
              label="Dołącz do tematu"
              minWidth={240}
              onClose={() => setJoinMenu(null)}
              items={joinTargets.map((t) => ({ key: t.slug, label: t.title, onSelect: () => void join(t.slug) }))}
            />
          )}
        </div>
      )}
      {o && (
        <p className="upd-outcome">
          {OUTCOME_LABEL[o.status]}
          {(o.status === "stale" || o.status === "failed") && `: ${o.message}`}
          {o.status === "applied" && o.message.startsWith("Cofnięto częściowo") && ` · ${o.message}`}
          {compact && (
            <>
              {" "}
              <button type="button" className="upd-link" aria-expanded onClick={() => setExpanded(false)}>
                Zwiń
              </button>
            </>
          )}
        </p>
      )}
    </li>
  );
}

/** The answer to a request, on top (clamped; "Pokaż całość"). */
function AnswerBox({ text }: { text: string }) {
  const [full, setFull] = useState(false);
  const long = text.length > 600 || text.split("\n").length > 8;
  return (
    <div className="upd-answer">
      <span className="upd-change-summary-label">Odpowiedź agenta</span>
      <div className={`upd-answer-body${long && !full ? " clamped" : ""}`}>
        <Markdown text={text} />
      </div>
      {long && (
        <button type="button" className="upd-link" aria-expanded={full} onClick={() => setFull(!full)}>
          {full ? "Zwiń" : "Pokaż całość"}
        </button>
      )}
    </div>
  );
}

function ReportBox({ id, text }: { id: string; text: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="upd-report">
      <Disclosure label="Raport agenta" open={open} onToggle={() => setOpen(!open)} controls={id} />
      {open && (
        <div className="upd-report-body" id={id}>
          <Markdown text={text} />
        </div>
      )}
    </div>
  );
}

/** The request a result answers, quoted above it. */
function RequestQuote({ text }: { text: string }) {
  return (
    <p className="upd-request">
      <span className="upd-request-label">Zlecenie</span> <q className="upd-request-text">{text}</q>
    </p>
  );
}

/** The request input under the bar: Enter (or "Zleć") runs it in the background with the chosen agent. */
function RequestInput({
  project,
  chosen,
  blocked,
  pending,
  history,
  collapsed = false,
  onStart,
  onEnableHistory,
}: {
  project: Project;
  chosen: ProviderId | null;
  /** Why it cannot run now (another background action), if so. */
  blocked: string | null;
  /** Undecided proposals a background request would replace. */
  pending: number;
  history: boolean;
  /** While a review is open: a quiet "Nowe zlecenie…" that opens the field. */
  collapsed?: boolean;
  onStart: (text: string, mode: RunMode) => Promise<boolean>;
  onEnableHistory: () => void;
}) {
  const slug = project.slug;
  const [text, setTextState] = useState(() => requestMemory.get(slug) ?? "");
  const [open, setOpen] = useState(() => !collapsed || requestMemory.has(slug));
  const [focused, setFocused] = useState(false);
  const [busy, setBusy] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);
  const opener = useRef<HTMLButtonElement>(null);
  const wasOpened = useRef(false);
  const setText = (v: string) => {
    setTextState(v);
    if (v) requestMemory.set(slug, v);
    else requestMemory.delete(slug);
  };
  useEffect(() => {
    if (!collapsed) setOpen(true);
  }, [collapsed]);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight + 2, 180)}px`;
  }, [text, open]);
  useEffect(() => {
    if (open && wasOpened.current) ref.current?.focus();
  }, [open]);
  const ready = text.trim() !== "" && chosen !== null && !busy;
  async function go(mode: RunMode, confirmed = false) {
    const t = text.trim();
    if (!t || !chosen || busy || (mode === "background" && blocked)) return;
    if (mode === "background" && pending > 0 && !confirmed) return setConfirm(true);
    setConfirm(false);
    setBusy(true);
    const ok = await onStart(t, mode);
    setBusy(false);
    if (ok) setText("");
    requestAnimationFrame(() => ref.current?.focus());
  }
  const id = `upd-ask-${slug}`;
  if (collapsed && !open) {
    return (
      <button
        ref={opener}
        type="button"
        className="btn btn-quiet upd-ask-open"
        onClick={() => {
          wasOpened.current = true;
          setOpen(true);
        }}
      >
        Nowe zlecenie…
      </button>
    );
  }
  return (
    <div className={`upd-ask-wrap${collapsed ? " in-review" : ""}`}>
      <div className="upd-ask">
        <label className="sr-only" htmlFor={id}>
          Zlecenie dla agenta
        </label>
        <textarea
          ref={ref}
          id={id}
          className="field upd-ask-input"
          rows={1}
          value={text}
          disabled={busy}
          placeholder="Zleć agentowi, np. „rozpisz temat X na podstawie historii rozmów”"
          aria-describedby={`${id}-note`}
          onChange={(e) => setText(e.target.value)}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void go("background");
            } else if (e.key === "Escape") {
              e.stopPropagation();
              if (collapsed && !text) {
                setOpen(false);
                requestAnimationFrame(() => opener.current?.focus());
              } else e.currentTarget.blur();
            }
          }}
        />
        <Button
          variant="ghost"
          className="upd-ask-go"
          disabled={!ready || !!blocked}
          title={blocked ?? (chosen ? `${PROVIDER_LABEL[chosen]} wykona zlecenie w tle; zmiany zaproponuje do akceptacji` : "Żaden agent nie jest dostępny")}
          onClick={() => void go("background")}
        >
          {busy ? "Uruchamiam…" : "Zleć"}
        </Button>
        <button
          type="button"
          className="upd-link upd-ask-term"
          disabled={!ready}
          title={chosen ? `Sesja ${PROVIDER_LABEL[chosen]} w Terminalu, z twoimi narzędziami; zapis po potwierdzeniu` : undefined}
          onClick={() => void go("terminal")}
        >
          w Terminalu
        </button>
      </div>
      <p className="upd-ask-hint" id={`${id}-note`}>
        {requestNote(history)}
        {!history && (
          <>
            {" "}
            <button type="button" className="upd-link" onClick={onEnableHistory}>
              Włącz historię
            </button>
          </>
        )}
        {focused && <span className="upd-ask-keys"> Enter: zleć, Shift+Enter: nowa linia.</span>}
      </p>
      {confirm && (
        <InlineConfirm
          question={recheckQuestion(pending, true, chosen)}
          confirmLabel="Uruchom"
          onConfirm={() => void go("background", true)}
          onCancel={() => {
            setConfirm(false);
            requestAnimationFrame(() => ref.current?.focus());
          }}
        />
      )}
    </div>
  );
}

/**
 * The drift line ("Rozjazd: …") and its list: branches outside topics with
 * "Przypisz do tematu…" / "Ignoruj", topics spread over many branches, topics
 * whose repository moved; "Zapytaj agenta" starts a request about them (the
 * only part that uses a subscription).
 */
function DriftPanel({
  project,
  open,
  onToggle,
  onAsk,
  askDisabled,
}: {
  project: Project;
  open: boolean;
  onToggle: () => void;
  onAsk: (text: string) => void;
  askDisabled: string | null;
}) {
  const { notify } = useVault();
  const view = useView();
  const drift = useDrift();
  const [menu, setMenu] = useState<{ anchor: HTMLElement; repo: string; branch: string } | null>(null);
  const d = drift.list.find((x) => x.project === project.slug) ?? null;
  const topics = view.snapshot.topics.filter((t) => t.project === project.slug && t.status !== "done");
  const titleOf = (slug: string) => view.snapshot.topics.find((t) => t.project === project.slug && t.slug === slug)?.title ?? slug;
  const line = driftLine(d, titleOf);
  if (!d || !line) return null;
  const id = `upd-drift-${project.slug}`;
  const act = async (call: () => Promise<ProjectDriftList>, done: string) => {
    try {
      driftStore.set(await call());
      notify(done);
    } catch (e) {
      notify(describeError(e));
    }
  };
  return (
    <div className="upd-drift">
      <Disclosure className="upd-drift-toggle" label={line} open={open} onToggle={onToggle} controls={id} />
      {open && (
        <div className="upd-drift-body" id={id}>
          {d.unknown.length > 0 && (
            <>
              <h4 className="upd-group-head">Gałęzie bez tematu</h4>
              <ul className="upd-drift-list">
                {d.unknown.map((b) => (
                  <li key={`${b.repo}/${b.branch}`} className="upd-drift-item">
                    <span className="upd-drift-name">
                      <span className="upd-drift-repo">{b.repo}</span> {b.branch}
                    </span>
                    <span className="upd-drift-meta">
                      ostatni commit {formatWhen(`${b.last_commit}T00:00`, new Date()).replace(/ 00:00$/, "")}
                      {b.worktree ? ` · drzewo robocze ${b.worktree}` : ""}
                    </span>
                    <span className="upd-drift-actions">
                      <button
                        type="button"
                        className="upd-link"
                        aria-haspopup="menu"
                        disabled={topics.length === 0}
                        onClick={(e) => setMenu({ anchor: e.currentTarget, repo: b.repo, branch: b.branch })}
                      >
                        Przypisz do tematu…
                      </button>
                      <button
                        type="button"
                        className="upd-link"
                        onClick={() => void act(() => api.ignoreBranch(project.slug, b.repo, b.branch), `Gałąź ${b.branch} nie będzie już zgłaszana.`)}
                      >
                        Ignoruj
                      </button>
                    </span>
                  </li>
                ))}
              </ul>
            </>
          )}
          {d.spread.length > 0 && (
            <>
              <h4 className="upd-group-head">Dużo rozgałęzień jednego tematu</h4>
              <ul className="upd-drift-list">
                {d.spread.map((x) => (
                  <li key={x.topic} className="upd-drift-item">
                    <span className="upd-drift-name">„{titleOf(x.topic)}”</span>
                    <span className="upd-drift-meta">
                      {d.branches
                        .filter((b) => b.topic === x.topic && b.active)
                        .map((b) => `${b.repo}/${b.branch}`)
                        .join(", ")}
                    </span>
                  </li>
                ))}
              </ul>
            </>
          )}
          {d.moved.length > 0 && (
            <>
              <h4 className="upd-group-head">Repozytorium ruszyło, temat nie</h4>
              <ul className="upd-drift-list">
                {d.moved.map((m) => (
                  <li key={m.topic} className="upd-drift-item">
                    <span className="upd-drift-name">„{titleOf(m.topic)}”</span>
                    <span className="upd-drift-meta">
                      {m.repo}/{m.branch}: commit {m.last_commit}, ostatni wpis {m.last_entry ?? "brak"}
                    </span>
                  </li>
                ))}
              </ul>
            </>
          )}
          <div className="upd-drift-foot">
            <Button variant="ghost" disabled={!!askDisabled} title={askDisabled ?? "Agent sprawdzi te gałęzie i zaproponuje tematy (zużywa limit subskrypcji)"} onClick={() => onAsk(driftRequest(d, titleOf))}>
              Zapytaj agenta
            </Button>
            <button type="button" className="upd-link" onClick={() => void api.refreshDrift().then((ok) => notify(ok ? "Sprawdzam repozytoria…" : "Sprawdzanie repozytoriów już trwa."))}>
              Sprawdź repozytoria teraz
            </button>
            {d.collected_at && <span className="upd-drift-meta">zebrano {formatWhen(d.collected_at, new Date())}, bez agenta</span>}
          </div>
          {menu && (
            <Menu
              anchor={menu.anchor}
              label="Przypisz do tematu"
              minWidth={240}
              onClose={() => setMenu(null)}
              items={topics.map((t) => ({
                key: t.slug,
                label: t.title,
                onSelect: () => void act(() => api.linkBranch(project.slug, menu.repo, menu.branch, t.slug), `Przypisano ${menu.branch} do „${t.title}”.`),
              }))}
            />
          )}
        </div>
      )}
    </div>
  );
}

type ProjectDriftList = Awaited<ReturnType<typeof api.listDrift>>;

function DroppedBox({ proposal }: { proposal: StoredProposal }) {
  const [open, setOpen] = useState(false);
  const n = proposal.dropped.length + proposal.dropped_more;
  if (n === 0) return null;
  const id = `upd-dropped-${proposal.run_id}`;
  return (
    <div className="upd-dropped">
      <Disclosure label={`Nie da się zapisać (${n})`} open={open} onToggle={() => setOpen(!open)} controls={id} />
      {open && (
        <ul className="upd-dropped-list" id={id}>
          {proposal.dropped.map((d) => (
            <li key={d.index}>
              {d.label}: {d.reason}
            </li>
          ))}
          {proposal.dropped_more > 0 && <li>… i {proposal.dropped_more} więcej</li>}
        </ul>
      )}
    </div>
  );
}

function Review({
  project,
  proposal,
  autoFocus,
  onApplied,
  onDecided,
  onUndone,
  extra,
}: {
  project: string;
  proposal: StoredProposal;
  autoFocus: boolean;
  /** After an apply click (focus moves to the status line / "Cofnij zapis"). */
  onApplied: () => void;
  onDecided: () => void;
  /** After an undo from the toast (focus moves to the review heading). */
  onUndone: () => void;
  /** Quiet things next to the actions ("Nowe zlecenie…"). */
  extra?: ReactNode;
}) {
  const { notify } = useVault();
  const [selected, setSelected] = useState<Set<number>>(() => new Set(defaultSelection(proposal)));
  const [busy, setBusy] = useState<"apply" | "dismiss" | null>(null);
  const headRef = useRef<HTMLHeadingElement>(null);
  const id = proposal.run_id;
  const c = counts(proposal);

  useEffect(() => {
    if (autoFocus) headRef.current?.focus();
  }, [autoFocus]);
  // Outcomes arrived (applied, undone): the user's ticks stay as they were;
  // only rows that became undecided again (an undo) are ticked.
  const undecidedKey = defaultSelection(proposal).join(",");
  const prevUndecided = useRef(defaultSelection(proposal));
  useEffect(() => {
    const undecided = defaultSelection(proposal);
    setSelected((cur) => nextSelection(prevUndecided.current, undecided, cur));
    prevUndecided.current = undecided;
  }, [undecidedKey]);

  const undecided = defaultSelection(proposal);
  const partial = c.pending > 0 && c.pending < c.total;
  const toggle = (i: number) =>
    setSelected((cur) => {
      const next = new Set(cur);
      if (next.has(i)) next.delete(i);
      else next.add(i);
      return next;
    });
  const allOn = undecided.every((i) => selected.has(i));

  async function undo(token: string, rejection = false) {
    try {
      const r = await api.undoUpdate(id, token);
      updateStore.set(r.updates);
      notify(rejection ? restoreSummary(r.items) : undoSummary(r.items));
      onUndone();
    } catch (e) {
      notify(describeError(e));
    }
  }
  async function apply() {
    setBusy("apply");
    try {
      const report = await api.applyUpdate(id, [...selected].filter((i) => undecided.includes(i)).sort((a, b) => a - b));
      updateStore.set(report.updates);
      const token = report.undo;
      notify(applySummary(report.items), token ? { label: "Cofnij", run: () => void undo(token) } : undefined);
      onApplied();
    } catch (e) {
      notify(describeError(e));
    } finally {
      setBusy(null);
    }
  }
  async function dismiss() {
    setBusy("dismiss");
    try {
      const r = await api.dismissUpdate(id);
      updateStore.set(r.updates);
      const token = r.undo;
      // Undoable, with the user's corrections kept: no confirmation needed.
      if (r.count > 0) notify(rejectSummary(r.count), token ? { label: "Cofnij", run: () => void undo(token, true) } : undefined);
      onDecided();
    } catch (e) {
      notify(describeError(e));
      setBusy(null);
    }
  }

  const secondary = secondaryLabel(c);
  const chosen = undecided.filter((i) => selected.has(i)).length;

  return (
    <div className="upd-review">
      <div className="upd-review-head">
        <h3 ref={headRef} tabIndex={-1} className="upd-review-title" id={`upd-review-${id}`}>
          {reviewHeading(c)}
          <span className="upd-review-meta">
            {" "}
            · {PROVIDER_LABEL[proposal.provider]}
            {proposal.trigger === "auto" ? " · automatycznie" : ""} · {formatWhen(proposal.created_at, new Date())}
          </span>
        </h3>
        {proposal.request && <RequestQuote text={proposal.request} />}
        {proposal.history_note && <p className="upd-history-note">{historyLine(proposal.history_note, "review")}</p>}
        {undecided.length > 3 && (
          <button type="button" className="upd-link" onClick={() => setSelected(allOn ? new Set() : new Set(undecided))}>
            {allOn ? "Odznacz wszystkie" : "Zaznacz wszystkie"}
          </button>
        )}
      </div>
      {proposal.request && proposal.report && <AnswerBox text={proposal.report} />}
      <GroupedChanges
        proposal={proposal}
        labelledBy={`upd-review-${id}`}
        row={(i) => (
          <ChangeRow
            key={i}
            project={project}
            sc={proposal.changes[i]!}
            index={i}
            runId={id}
            selected={selected.has(i)}
            onToggle={() => toggle(i)}
            busy={busy !== null}
            compact={partial}
            editable
            proposalTopics={proposal.changes.flatMap((x) => (x.change.kind === "topic_log" ? [x.change.topic] : []))}
          />
        )}
      />

      <DroppedBox proposal={proposal} />
      {!proposal.request && proposal.report && <ReportBox id={`upd-report-${id}`} text={proposal.report} />}

      <div className="upd-review-actions">
        {c.pending > 0 && (
          <Button variant="primary" disabled={chosen === 0 || busy !== null} aria-busy={busy === "apply" || undefined} onClick={() => void apply()}>
            {busy === "apply" ? "Zapisuję…" : `Zastosuj zaznaczone (${chosen})`}
          </Button>
        )}
        {secondary && (
          <Button variant="quiet" disabled={busy !== null} onClick={() => void dismiss()}>
            {secondary}
          </Button>
        )}
        {extra}
      </div>
    </div>
  );
}

/** Topics first (the primary items), then tasks; quiet headings when both are present. */
function GroupedChanges({ proposal, row, labelledBy, label }: { proposal: StoredProposal; row: (i: number) => ReactNode; labelledBy?: string; label?: string }) {
  const g = groupChanges(proposal);
  const both = g.topics.length > 0 && g.tasks.length > 0;
  if (!both) {
    return (
      <ul className="upd-changes" aria-labelledby={labelledBy} aria-label={label}>
        {[...g.topics, ...g.tasks].map(row)}
      </ul>
    );
  }
  const base = labelledBy ?? `upd-g-${proposal.run_id}`;
  return (
    <div className="upd-groups" role="group" aria-labelledby={labelledBy} aria-label={label}>
      <h4 className="upd-group-head" id={`${base}-topics`}>
        Tematy
      </h4>
      <ul className="upd-changes" aria-labelledby={`${base}-topics`}>
        {g.topics.map(row)}
      </ul>
      <h4 className="upd-group-head" id={`${base}-tasks`}>
        Zadania
      </h4>
      <ul className="upd-changes" aria-labelledby={`${base}-tasks`}>
        {g.tasks.map(row)}
      </ul>
    </div>
  );
}

/** The last result, reopened from the receipt line (rows with outcomes + the report). */
function ResultDetails({ project, proposal }: { project: string; proposal: StoredProposal }) {
  return (
    <div className="upd-review" id={`upd-details-${proposal.run_id}`}>
      {proposal.request && <RequestQuote text={proposal.request} />}
      {proposal.request && proposal.report && <AnswerBox text={proposal.report} />}
      <GroupedChanges
        proposal={proposal}
        label="Wynik ostatniej aktualizacji"
        row={(i) => <ChangeRow key={i} project={project} sc={proposal.changes[i]!} index={i} runId={proposal.run_id} />}
      />
      <DroppedBox proposal={proposal} />
      {!proposal.request && proposal.report && <ReportBox id={`upd-report-${proposal.run_id}`} text={proposal.report} />}
    </div>
  );
}

export function UpdateStrip({ project, openAuto = false, openDrift = false }: { project: Project; openAuto?: boolean; openDrift?: boolean }) {
  const { notify } = useVault();
  const runs = useRuns();
  const updates = useUpdates();
  const providers = useProviders();
  const view = updates.byProject[project.slug] ?? null;
  const run = updateRunFor(runs.list, project.slug);
  const strip = stripState(view, run);
  const running = strip.kind === "running";
  const now = useNow(running);
  const [provider, setProvider] = useState<ProviderId | null>(() => (readPref(PROVIDER_PREF) as ProviderId | null) ?? null);
  const [busy, setBusy] = useState(false);
  const [menu, setMenu] = useState<HTMLElement | null>(null);
  const [providerMenu, setProviderMenu] = useState<HTMLElement | null>(null);
  const [autoOpen, setAutoOpen] = useState(openAuto);
  const [autoFocusHistory, setAutoFocusHistory] = useState(false);
  const [driftOpen, setDriftOpen] = useState(openDrift);
  const [askDrift, setAskDrift] = useState<string | null>(null);
  const [confirmRecheck, setConfirmRecheck] = useState(false);
  const [showDetails, setShowDetails] = useState(false);
  const [showTech, setShowTech] = useState(false);
  const [announce, setAnnounce] = useState("");
  const [focusReview, setFocusReview] = useState<string | null>(null);
  const sectionRef = useRef<HTMLElement>(null);
  const autoBtnRef = useRef<HTMLButtonElement>(null);
  // The run watched here and the last result before it.
  const watched = useRef<{ run: string; lastAt: string | null } | null>(null);

  useEffect(() => {
    void updateStore.start().catch(() => {});
    void updateStore.loadProviders().catch(() => {});
    void runStore.start().catch(() => {});
    void driftStore.start().catch(() => {});
  }, []);
  useEffect(() => {
    if (openAuto) setAutoOpen(true);
  }, [openAuto]);

  const runId = strip.kind === "running" ? strip.run.id : null;
  const proposalRun = view?.proposal?.run_id ?? null;
  const lastAt = view?.last?.at ?? null;
  useEffect(() => {
    if (runId) {
      if (watched.current?.run !== runId) watched.current = { run: runId, lastAt };
      return;
    }
    const w = watched.current;
    if (!w) return;
    if (strip.kind === "review" && proposalRun === w.run) {
      const c = counts(strip.proposal);
      setAnnounce(`Gotowe: ${plural(c.total, "propozycja", "propozycje", "propozycji")} zmian.`);
      // Focus moves to the result only from inside the strip or from nowhere in particular.
      const active = document.activeElement;
      if (!active || active === document.body || sectionRef.current?.contains(active)) setFocusReview(w.run);
      watched.current = null;
    } else if (lastAt !== w.lastAt) {
      const last = view?.last;
      const what = last?.request ? "Zlecenie" : "Sprawdzanie";
      setAnnounce(
        last?.outcome.kind === "failed"
          ? `${what} nie powiodło się.`
          : last?.outcome.kind === "cancelled"
            ? `${what} przerwane.`
            : last?.request
              ? "Gotowe: odpowiedź bez zmian w MontyBrain."
              : "Gotowe: bez zmian.",
      );
      watched.current = null;
    }
  }, [runId, strip.kind, proposalRun, lastAt]);

  const chosen = providers ? pickProvider(providers, provider) : null;
  const available = providers?.some((p) => p.available) ?? true;
  const auto = view?.auto ?? DEFAULT_AUTO;
  const vaultView = useView();
  const hasTopics = vaultView.snapshot.topics.some((t) => t.project === project.slug);
  const line = statusLine(view?.last ?? null, new Date(now), providers, hasTopics);
  const elsewhere = runs.active && !running ? runs.active : null;
  const pending = view?.pending ?? 0;

  // After an apply click the button is gone or disabled: "Cofnij zapis", else the status line.
  const focusAfterApply = () =>
    requestAnimationFrame(() => requestAnimationFrame(() => {
      const root = sectionRef.current;
      for (const sel of [".upd-undo", ".upd-status"]) {
        const el = root?.querySelector<HTMLElement>(sel);
        if (el && !(el as HTMLButtonElement).disabled) return el.focus();
      }
    }));
  /** Focus on the first match of `selectors` that is there and enabled (after the next render). */
  const focusFirst = (...selectors: string[]) =>
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        const root = sectionRef.current;
        for (const sel of selectors) {
          const el = root?.querySelector<HTMLElement>(sel);
          if (el && !(el as HTMLButtonElement).disabled) return el.focus();
        }
      }),
    );
  // After an undo the rows are back for review: its heading, else the status line.
  const focusAfterUndo = () => focusFirst(".upd-review-title", ".upd-status");
  // A run started or stopped: the controls that started it are gone or disabled.
  const focusStatus = () => focusFirst(".upd-status", ".upd-main");
  const openHistory = () => {
    setAutoFocusHistory(true);
    if (autoOpen) setAutoOpen(false);
    requestAnimationFrame(() => setAutoOpen(true));
  };
  const focusStrip = () =>
    requestAnimationFrame(() => {
      const root = sectionRef.current;
      for (const sel of [".upd-receipt-toggle", ".upd-main", ".upd-title-focus"]) {
        const el = root?.querySelector<HTMLElement>(sel);
        if (el && !(el as HTMLButtonElement).disabled) return el.focus();
      }
    });

  async function start(mode: "background" | "terminal") {
    if (!chosen) return;
    setBusy(true);
    try {
      const r = await runStore.startRun({ recipe: UPDATE_RECIPE, provider: chosen, mode, params: { project: project.slug } });
      if (r.mode === "terminal") notify(r.status === "failed" ? (r.error ?? "Nie udało się otworzyć Terminala.") : `Otwarto sesję ${PROVIDER_LABEL[chosen]} w Terminalu.`);
      else {
        setShowDetails(false);
        focusStatus();
      }
    } catch (e) {
      notify(describeError(e));
    } finally {
      setBusy(false);
    }
  }
  async function startRequest(text: string, mode: RunMode): Promise<boolean> {
    if (!chosen) return false;
    try {
      const r = await runStore.startRun({ recipe: REQUEST_RECIPE, provider: chosen, mode, params: { project: project.slug, text } });
      if (r.mode === "terminal") {
        if (r.status === "failed") {
          notify(r.error ?? "Nie udało się otworzyć Terminala.");
          return false;
        }
        notify(`Otwarto sesję ${PROVIDER_LABEL[chosen]} w Terminalu.`);
      } else {
        setShowDetails(false);
        focusStatus();
      }
      return true;
    } catch (e) {
      notify(describeError(e));
      return false;
    }
  }
  async function cancel() {
    if (strip.kind !== "running") return;
    try {
      await runStore.cancel(strip.run.id);
      focusStatus();
    } catch (e) {
      notify(describeError(e));
    }
  }
  // The last apply click stays undoable from the strip, not only from its toast
  // (which is gone after a few seconds, and never shown for automatic writes).
  const undoToken = view?.undo ?? null;
  const undoRun = view?.proposal?.run_id ?? null;
  async function undoLast() {
    if (!undoToken || !undoRun) return;
    setBusy(true);
    try {
      const rejection = view?.undo_rejection ?? false;
      const r = await api.undoUpdate(undoRun, undoToken);
      updateStore.set(r.updates);
      notify(rejection ? restoreSummary(r.items) : undoSummary(r.items));
      focusAfterUndo();
    } catch (e) {
      notify(describeError(e));
    } finally {
      setBusy(false);
    }
  }
  const goTo = (recipe: string, params: Record<string, string>) => {
    presetAction(recipe, params);
    navigate("#/akcje");
  };
  const closeAuto = () => {
    setAutoOpen(false);
    setAutoFocusHistory(false);
    requestAnimationFrame(() => autoBtnRef.current?.focus());
  };

  const menuItems: MenuItem[] = [
    ...(strip.kind === "review"
      ? [
          { key: "h-upd", label: "Aktualizacja", header: true },
          { key: "recheck", label: chosen ? `Sprawdź ponownie (${PROVIDER_LABEL[chosen]})…` : "Sprawdź ponownie…", disabled: !chosen || !!elsewhere, onSelect: () => setConfirmRecheck(true) },
        ]
      : []),
    { key: "h-more", label: "Otwórz w Akcjach", header: true },
    { key: "postep", label: "Raport postępu…", onSelect: () => goTo("postep", { project: project.slug }) },
    { key: "raport", label: "Raport z dnia lub tygodnia…", onSelect: () => goTo("raport", { project: project.slug }) },
    { key: "zadania", label: "Zadania z notatek…", onSelect: () => goTo("zadania", { project: project.slug }) },
    { key: "h-term", label: "Od razu", header: true },
    {
      key: "term",
      label: "Sprawdź w Terminalu",
      hint: chosen ? PROVIDER_LABEL[chosen] : undefined,
      disabled: !chosen,
      title: chosen ? undefined : "Żaden agent nie jest dostępny",
      onSelect: () => void start("terminal"),
    },
  ];
  const providerItems: MenuItem[] = (providers ?? []).map((p) => ({
    key: p.id,
    label: PROVIDER_LABEL[p.id],
    checked: chosen === p.id,
    disabled: !p.available,
    hint: p.available ? undefined : "niedostępny",
    title: p.available ? undefined : (p.problem ?? "Niedostępny"),
    onSelect: () => {
      setProvider(p.id);
      writePref(PROVIDER_PREF, p.id);
    },
  }));

  const topicTitle = (slug: string) => vaultView.snapshot.topics.find((t) => t.project === project.slug && t.slug === slug)?.title ?? null;
  const requestInput = (collapsed: boolean) => (
    <RequestInput
      key={project.slug}
      project={project}
      chosen={chosen}
      blocked={elsewhere ? `W tle trwa już akcja „${elsewhere.recipe_title}”.` : null}
      pending={pending}
      history={view?.sources.history ?? false}
      collapsed={collapsed}
      onStart={startRequest}
      onEnableHistory={openHistory}
    />
  );
  const lastEvent = strip.kind === "running" ? strip.run.events[strip.run.events.length - 1] : undefined;
  const idleReport = strip.kind === "idle" && view?.proposal?.report ? view.proposal : null;
  const c = strip.kind === "review" || strip.kind === "receipt" ? counts(strip.proposal) : null;

  let status: ReactNode;
  let statusError = false;
  if (strip.kind === "running") status = runningStatus(strip.run);
  else if (strip.kind === "review") status = reviewStatus(c!);
  else if (strip.kind === "receipt") status = receiptLine(strip.proposal, new Date(now));
  else if (!available) {
    status = "Agent niedostępny";
    statusError = true;
  } else {
    status = line.text;
    statusError = line.error;
  }

  return (
    <section
      ref={sectionRef}
      className="upd"
      aria-labelledby={`upd-title-${project.slug}`}
      onKeyDown={(e) => {
        // Escape closes the sources panel from anywhere in the strip (inner
        // confirmations, menus and editors stop it first).
        if (e.key === "Escape" && autoOpen && !e.defaultPrevented) {
          e.stopPropagation();
          closeAuto();
        }
      }}
    >
      <div className="upd-bar">
        <div className="upd-text">
          <RefreshCw size={13} className={`upd-icon${running ? " spin" : ""}`} aria-hidden />
          <h2 className="upd-title upd-title-focus" id={`upd-title-${project.slug}`} tabIndex={-1}>
            Aktualizacja
          </h2>
          <p className={`upd-status${statusError ? " error" : ""}`} aria-live="polite" tabIndex={-1}>
            {status}
          </p>
          {strip.kind === "running" && <time className="upd-clock">{formatElapsed(runElapsedMs(strip.run.started_at, null, now))}</time>}
          {strip.kind === "receipt" && (
            <Disclosure
              className="upd-receipt-toggle"
              label={showDetails ? "Ukryj szczegóły" : "Pokaż szczegóły"}
              open={showDetails}
              onToggle={() => setShowDetails(!showDetails)}
              controls={`upd-details-${strip.proposal.run_id}`}
            />
          )}
          {undoToken && (strip.kind === "receipt" || strip.kind === "review") && (
            <button
              type="button"
              className="upd-link upd-undo"
              disabled={busy}
              title={view?.undo_rejection ? "Przywraca odrzucone propozycje do przeglądu, z twoimi poprawkami" : "Cofa ostatni zapis, o ile nic się od tego czasu nie zmieniło"}
              onClick={() => void undoLast()}
            >
              {view?.undo_rejection ? "Cofnij odrzucenie" : "Cofnij zapis"}
            </button>
          )}
        </div>
        <div className="upd-actions">
          <Button ref={autoBtnRef} variant="quiet" className="upd-auto-btn" aria-expanded={autoOpen} aria-controls={`upd-auto-${project.slug}`} onClick={() => (autoOpen ? closeAuto() : setAutoOpen(true))}>
            {panelLabel(auto, view?.sources.history ?? false)}
          </Button>
          {running ? (
            <Button variant="danger" onClick={() => void cancel()}>
              Przerwij
            </Button>
          ) : strip.kind !== "review" ? (
            <>
              {providers && providers.length > 0 && (
                <button
                  type="button"
                  className="btn btn-quiet upd-provider"
                  aria-haspopup="menu"
                  aria-expanded={providerMenu !== null}
                  aria-label={`Subskrypcja: ${chosen ? PROVIDER_LABEL[chosen] : "brak"}. Zmień`}
                  disabled={!available}
                  onClick={(e) => setProviderMenu(providerMenu ? null : e.currentTarget)}
                >
                  {chosen ? PROVIDER_LABEL[chosen] : "Brak agenta"}
                  <ChevronDown size={12} aria-hidden />
                </button>
              )}
              <Button
                variant="ghost"
                className="upd-main"
                disabled={!chosen || busy || !!elsewhere}
                title={elsewhere ? `W tle trwa już akcja „${elsewhere.recipe_title}”.` : chosen ? `${PROVIDER_LABEL[chosen]} sprawdzi projekt w tle, tylko do odczytu` : undefined}
                onClick={() => void start("background")}
              >
                {busy ? "Uruchamiam…" : "Sprawdź projekt"}
              </Button>
            </>
          ) : null}
          <button
            type="button"
            className="icon-btn"
            aria-label="Więcej akcji dla projektu"
            title="Więcej akcji dla projektu"
            aria-haspopup="menu"
            aria-expanded={menu !== null}
            onClick={(e) => setMenu(menu ? null : e.currentTarget)}
          >
            <MoreHorizontal size={14} />
          </button>
        </div>
      </div>

      {strip.kind !== "running" && (
        <DriftPanel
          project={project}
          open={driftOpen}
          onToggle={() => setDriftOpen(!driftOpen)}
          askDisabled={!chosen ? "Żaden agent nie jest dostępny" : elsewhere ? `W tle trwa już akcja „${elsewhere.recipe_title}”.` : null}
          onAsk={(text) => (pending > 0 ? setAskDrift(text) : void startRequest(text, "background"))}
        />
      )}
      {askDrift && (
        <InlineConfirm
          question={recheckQuestion(pending, true, chosen)}
          confirmLabel="Uruchom"
          onConfirm={() => {
            const t = askDrift;
            setAskDrift(null);
            void startRequest(t, "background");
          }}
          onCancel={() => setAskDrift(null)}
        />
      )}
      {strip.kind !== "running" && strip.kind !== "review" && available && requestInput(false)}
      {strip.kind === "running" && (
        <div className="upd-run">
          <p className="upd-run-line" aria-live="polite">
            {lastEvent ? progressText(lastEvent, topicTitle) : "Czekam na pierwsze komunikaty agenta…"}
          </p>
          <a
            href="#/akcje"
            className="upd-link"
            onClick={() => {
              // Akcje opens on this run's recipe (its parameters filled in) with the run selected.
              presetAction(strip.run.recipe, { project: project.slug, ...(strip.run.request ? { text: strip.run.request } : {}) });
              runStore.select(strip.run.id);
            }}
          >
            Otwórz przebieg w Akcjach →
          </a>
        </div>
      )}
      {strip.kind === "idle" && !available && (
        <div className="upd-sub">
          <p className="upd-sub-text">Nie znaleziono Claude Code ani Codex. Zainstaluj jedno z nich i zaloguj się na swoją subskrypcję.</p>
          <button type="button" className="upd-link" onClick={() => void updateStore.loadProviders(true).catch((e) => notify(describeError(e)))}>
            Sprawdź ponownie
          </button>
        </div>
      )}
      {strip.kind === "idle" && available && line.detail && (
        <div className="upd-sub">
          <Disclosure label="Szczegóły techniczne" open={showTech} onToggle={() => setShowTech(!showTech)} controls={`upd-tech-${project.slug}`} />
          {showTech && (
            <pre className="upd-tech" id={`upd-tech-${project.slug}`}>
              {line.detail}
            </pre>
          )}
        </div>
      )}
      {strip.kind === "idle" && view?.last?.history_note && (
        <p className="upd-hint">
          {historyLine(view.last.history_note, "idle")}
          {view.last.history_note === HISTORY_OFF_NOTE && (
            <>
              {" "}
              <button type="button" className="upd-link" onClick={openHistory}>
                Włącz historię
              </button>
            </>
          )}
        </p>
      )}
      {strip.kind === "idle" && elsewhere && <p className="upd-hint">W tle trwa już akcja „{elsewhere.recipe_title}”. Sprawdzenie uruchomisz po jej zakończeniu.</p>}
      {strip.kind === "review" && view?.held && !autoOpen && <p className="upd-hint">{heldText(pending)}</p>}

      {confirmRecheck && strip.kind === "review" && (
        <InlineConfirm
          question={recheckQuestion(pending, false, chosen)}
          confirmLabel="Uruchom"
          onConfirm={() => {
            setConfirmRecheck(false);
            void start("background");
          }}
          onCancel={() => {
            setConfirmRecheck(false);
            focusStrip();
          }}
        />
      )}

      {autoOpen && <AutoPanel project={project} view={view} providers={providers} ready={updates.loaded} focusHistory={autoFocusHistory} onClose={closeAuto} />}

      {strip.kind === "review" && (
        <Review
          key={strip.proposal.run_id}
          project={project.slug}
          proposal={strip.proposal}
          autoFocus={focusReview === strip.proposal.run_id}
          onApplied={focusAfterApply}
          onDecided={focusStrip}
          onUndone={focusAfterUndo}
          extra={available ? requestInput(true) : null}
        />
      )}
      {strip.kind === "receipt" && showDetails && <ResultDetails project={project.slug} proposal={strip.proposal} />}
      {idleReport?.request && <RequestQuote text={idleReport.request} />}
      {idleReport?.request ? (
        <div className="upd-idle-answer">
          <AnswerBox key={idleReport.run_id} text={idleReport.report} />
        </div>
      ) : (
        idleReport && <ReportBox key={idleReport.run_id} id={`upd-report-${project.slug}`} text={idleReport.report} />
      )}

      <p className="sr-only" aria-live="polite">
        {announce}
      </p>

      {menu && <Menu anchor={menu} items={menuItems} onClose={() => setMenu(null)} label="Więcej akcji dla projektu" minWidth={250} />}
      {providerMenu && <Menu anchor={providerMenu} items={providerItems} onClose={() => setProviderMenu(null)} label="Subskrypcja" minWidth={200} />}
    </section>
  );
}
