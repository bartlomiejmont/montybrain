import { t as tr, message } from "../i18n";
// Skrzynka: one command field on top, the entries below, each with one local
// destination suggestion ("?" when the match is weak), keyboard flow and
// multi-select. Commands and suggestions are read and written by the backend
// (montybrain_core::command); the agent (`?…`) runs only on request, with the
// chosen entries and their local candidates as data.
//
// Keys on the list (not while typing): ↑↓ or j/k move, Enter takes the
// suggestion (to the confirm step), `>` picks a place, `t` makes a topic,
// `/` starts a command for the focused entry (`/czeka …` with no subject
// applies to it), Space selects, `u` undoes the last command, `?` asks the
// agent for a command, Esc clears the selection.

import { ChevronRight } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api, mockCommandState, MOCK_COMMAND_TEXT } from "../api";
import type { AgentAsk, InboxAgentTools, InboxSuggestion, ProviderId, RunInfo, Task } from "../api/types";
import { AgentField, MODE_SHORTCUT, ModeSwitch, type AgentFieldHandle, type FieldMode } from "../components/AgentField";
import { CommandField, type CommandFieldHandle } from "../components/CommandField";
import { agentEntries, contextLabel, MY_TOOLS_CONSENT_KEY, planAsk, PROVIDER_NAME } from "../lib/agentMode";
import { TaskRow } from "../components/TaskRow";
import { Button, useStoredFlag } from "../components/ui";
import { pickProvider } from "../lib/actions";
import { describeError } from "../lib/errors";
import { describeUndo, isUndoKey } from "../lib/undo";
import { splitDone, tasksAt } from "../lib/group";
import { readPref, writePref } from "../store/ui";
import { runStore, useRuns } from "../store/runs";
import { sessionStore } from "../store/session";
import { updateStore, useProviders, useUpdates } from "../store/updates";
import { useVault, useView } from "../store/vault";
import { Review } from "./project/UpdateStrip";
import "./inbox.css";

const INBOX = { kind: "inbox" } as const;
const PROVIDER_PREF = "montybrain.aktualizacja.provider";
/** The proposals of the Skrzynka agent live under this "project" key (src-tauri proposal::INBOX). */
const INBOX_KEY = "";

const isTyping = (el: EventTarget | null) =>
  el instanceof HTMLElement && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);

/** Mock URL states for screenshots: `?field=agent`, `&tools=mine`, `&consent=1`, `&terminal=1`. */
function urlParam(name: string): string | null {
  return typeof window === "undefined" ? null : new URLSearchParams(window.location.search).get(name);
}

/** How often a Terminal session's answer file is looked for. */
const TERMINAL_POLL_MS = 4000;

function initialSelection(): string[] {
  const state = mockCommandState();
  if (state === "topic" || state === "partial") return MOCK_COMMAND_TEXT[state].selection ?? [];
  const q = typeof window === "undefined" ? null : new URLSearchParams(window.location.search).get("select");
  return q ? q.split(",").filter(Boolean) : [];
}

function Suggestion({ s, onAccept, onAsk }: { s: InboxSuggestion | undefined; onAccept: () => void; onAsk: () => void }) {
  if (!s) return <span className="ibx-sugg muted" />;
  if (s.best) {
    return (
      <button
        type="button"
        className="ibx-sugg"
        title={tr("Lokalna podpowiedź (wynik {0}). Enter: przenieś, z potwierdzeniem.", s.best.score.toFixed(1))}
        onClick={(e) => {
          e.stopPropagation();
          onAccept();
        }}
      >
        <span className="ibx-arrow" aria-hidden>
          →
        </span>
        <span className="ibx-sugg-label">{s.best.place.label}</span>
      </button>
    );
  }
  const near = s.candidates.map((c) => c.place.label).join(", ");
  return (
    <button
      type="button"
      className="ibx-sugg weak"
      title={near ? tr("Słabe dopasowanie: {0}. Kliknij, żeby zapytać agenta.", near) : tr("Nic lokalnie nie pasuje. Kliknij, żeby zapytać agenta.")}
      aria-label={tr("Słabe dopasowanie: zapytaj agenta")}
      onClick={(e) => {
        e.stopPropagation();
        onAsk();
      }}
    >
      ?
    </button>
  );
}

export function InboxView() {
  const view = useView();
  const { notify } = useVault();
  const tasks = tasksAt(view.snapshot, INBOX);
  const { open, done } = splitDone(tasks);
  const mock = mockCommandState();
  const [selected, setSelected] = useState<string[]>(initialSelection);
  const [showDone, setShowDone] = useStoredFlag("montybrain.done.inbox", false);
  /** The last Skrzynka write, undoable from the "Cofnij" next to it until undone. */
  const [lastWrite, setLastWrite] = useState<{ reading: string; token: number | null } | null>(
    mock === "echo" ? { reading: tr("Nowe zadanie „Podpiąć zapisane karty” w: WhenUBuy / Płatności: Stripe → Przelewy24 · priorytet wysoki"), token: 1 } : null,
  );
  const [cursor, setCursor] = useState<string | null>(null);
  const [suggestions, setSuggestions] = useState<Map<string, InboxSuggestion>>(new Map());
  const [agentRun, setAgentRun] = useState<{ id: string; mode: "command" | "proposals" } | null>(null);
  const [agentNote, setAgentNote] = useState<string | null>(mock === "agent" ? tr("Agent: wpis dotyczy webhooka zwrotów, który jest w temacie płatności (Stripe → Przelewy24).") : null);
  const field = useRef<CommandFieldHandle>(null);
  const agentField = useRef<AgentFieldHandle>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const providers = useProviders();
  // The field's mode (Komenda | Agent) and, for the agent, the provider and
  // what it may reach. "Moje narzędzia" is per request: back to "Tylko
  // MontyBrain" after each one.
  const [fieldMode, setFieldMode] = useState<FieldMode>(urlParam("field") === "agent" ? "agent" : "command");
  const [agentText, setAgentText] = useState("");
  const [providerPref, setProviderPref] = useState<string | null>(() => readPref(PROVIDER_PREF));
  const [tools, setTools] = useState<InboxAgentTools>(urlParam("tools") === "mine" ? "mine" : "montybrain");
  const [consentPending, setConsentPending] = useState(urlParam("consent") === "1");
  /** A Terminal session ("Moje narzędzia") whose answer is awaited. */
  const [terminalRun, setTerminalRun] = useState<{ id: string; provider: ProviderId } | null>(urlParam("terminal") === "1" ? { id: "mock-terminal", provider: "claude" } : null);
  const [pasting, setPasting] = useState(false);
  const [pasteText, setPasteText] = useState("");
  const runs = useRuns();
  const updates = useUpdates();
  const proposal = updates.byProject[INBOX_KEY]?.proposal ?? null;
  const pendingProposal = proposal && proposal.changes.some((c) => !c.outcome) ? proposal : null;

  useEffect(() => {
    void updateStore.start().catch(() => {});
    void updateStore.loadProviders().catch(() => {});
    void runStore.start().catch(() => {});
  }, []);

  // Suggestions follow the vault.
  useEffect(() => {
    let live = true;
    api
      .inboxSuggestions()
      .then((list) => live && setSuggestions(new Map(list.map((s) => [s.id, s]))))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [view.version]);

  // Selected entries that left the inbox are no longer selected.
  const openIds = useMemo(() => new Set(open.flatMap((t) => (t.id ? [t.id] : []))), [open]);
  useEffect(() => {
    setSelected((s) => (s.every((id) => openIds.has(id)) ? s : s.filter((id) => openIds.has(id))));
  }, [openIds]);

  const toggle = useCallback((id: string) => setSelected((s) => (s.includes(id) ? s.filter((x) => x !== id) : [...s, id])), []);

  const focusRow = (id: string | null) => {
    setCursor(id);
    if (id) requestAnimationFrame(() => listRef.current?.querySelector<HTMLElement>(`[data-inbox-row="${id}"]`)?.focus());
  };

  const accept = (t: Task) => {
    const s = t.id ? suggestions.get(t.id) : undefined;
    if (!t.id) return;
    if (s?.best) field.current?.setText(`^${t.id} > ${s.best.place.text}`, { submit: true });
    else field.current?.setText(`^${t.id} > `);
  };

  const provider = (): ProviderId | null => pickProvider(providers ?? [], providerPref);

  async function askAgent(mode: "command" | "proposals", text: string, ids: string[], with_: InboxAgentTools = "montybrain") {
    const p = provider();
    if (!p) {
      notify(tr("Żaden agent nie jest dostępny (Claude Code ani Codex)."));
      return;
    }
    try {
      const run = await api.inboxAgent({ text, selection: ids, mode, provider: p, tools: with_ });
      if (with_ === "mine") {
        if (run.status === "failed") notify(run.error ?? tr("Nie udało się otworzyć sesji w Terminalu."));
        else {
          setTerminalRun({ id: run.id, provider: p });
          notify(tr("Otwarto sesję {0} w Terminalu. Propozycje wrócą do Skrzynki.", PROVIDER_NAME[p]));
        }
        return;
      }
      setAgentRun({ id: run.id, mode });
      if (run.status !== "running") void finished(run, mode);
    } catch (e) {
      notify(describeError(e));
    }
  }

  // A Terminal session: look for its answer file every few seconds.
  useEffect(() => {
    if (!terminalRun || terminalRun.id === "mock-terminal") return;
    let live = true;
    const tick = async () => {
      try {
        const r = await api.inboxAgentCheck(terminalRun.id);
        if (!live || r.status !== "received") return;
        setTerminalRun(null);
        setPasting(false);
        notify(r.count === 0 ? tr("Agent odpowiedział bez propozycji zmian.") : tr("Agent przysłał propozycje ({0}): zaznacz, co zapisać.", r.count));
      } catch (e) {
        if (!live) return;
        setTerminalRun(null);
        notify(describeError(e));
      }
    };
    const t = window.setInterval(() => void tick(), TERMINAL_POLL_MS);
    void tick();
    return () => {
      live = false;
      window.clearInterval(t);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [terminalRun?.id]);

  async function pasteAnswer() {
    if (!terminalRun) return;
    try {
      const r = await api.inboxAgentPaste(terminalRun.id, pasteText);
      setTerminalRun(null);
      setPasting(false);
      setPasteText("");
      notify(r.count === 0 ? tr("Odpowiedź wczytana: bez propozycji zmian.") : tr("Wczytano propozycje ({0}): zaznacz, co zapisać.", r.count));
    } catch (e) {
      notify(describeError(e));
    }
  }

  const switchMode = (m: FieldMode, text = "") => {
    setFieldMode(m);
    if (m === "agent") {
      setAgentText(text);
      requestAnimationFrame(() => agentField.current?.focus());
    } else {
      setConsentPending(false);
      requestAnimationFrame(() => field.current?.focus());
    }
  };

  // ⌘J (Ctrl+J) flips Komenda | Agent, also while typing in the field.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey || e.key.toLowerCase() !== "j") return;
      if (document.querySelector('[aria-modal="true"]')) return;
      e.preventDefault();
      setFieldMode((m) => {
        const next = m === "agent" ? "command" : "agent";
        if (next === "command") setConsentPending(false);
        requestAnimationFrame(() => (next === "agent" ? agentField.current?.focus() : field.current?.focus()));
        return next;
      });
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const chooseTools = (t: InboxAgentTools) => {
    if (t === "mine" && readPref(MY_TOOLS_CONSENT_KEY) !== "1") {
      setConsentPending(true);
      return;
    }
    setTools(t);
  };

  const submitAgent = (text: string, terminal = false) => {
    const ids = agentEntries(selected, focused?.id ?? null);
    const plan = planAsk(text, ids.length, tools);
    if (tools === "mine" && !terminal) {
      // The in-app session: the conversation and the permission cards in the panel.
      const p = provider();
      if (!p) notify(tr("Żaden agent nie jest dostępny (Claude Code ani Codex)."));
      else sessionStore.prepare({ scope: "inbox", provider: p, text: plan.text, selection: ids, origin: window.location.hash });
    } else void askAgent(plan.mode, plan.text, ids, tools);
    // Per request: the user's own tools are off again for the next one.
    setTools("montybrain");
  };

  async function finished(run: RunInfo, mode: "command" | "proposals") {
    setAgentRun(null);
    if (run.status !== "succeeded") {
      notify(run.error ?? tr("Agent nie dokończył pracy."));
      return;
    }
    if (mode === "command") {
      try {
        const a = await api.inboxAgentAnswer(run.id);
        if (a.command) {
          setAgentNote(tr("Agent: {0}", a.note));
          field.current?.setText(a.command);
        } else notify(a.note);
      } catch (e) {
        notify(describeError(e));
      }
    }
  }

  // Follow the agent run to its end.
  useEffect(() => {
    if (!agentRun) return;
    const run = runs.list.find((r) => r.id === agentRun.id);
    if (run && run.status !== "running") void finished(run, agentRun.mode);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runs, agentRun]);

  const onAgent = (ask: AgentAsk) => {
    const ids = [...new Set([...ask.ids, ...agentEntries(selected, focused?.id ?? null)])];
    const single = ask.verb === "free" && ids.length === 1;
    const text = ask.verb === "rozpisz" ? tr("Rozpisz te wpisy na konkretne kroki. {0}", ask.text).trim() : ask.verb === "dopasuj" ? tr("Zaproponuj miejsca dla tych wpisów. {0}", ask.text).trim() : ask.text;
    void askAgent(single ? "command" : "proposals", text, ids);
  };

  const onListKey = (e: React.KeyboardEvent) => {
    if (isTyping(e.target) || e.metaKey || e.ctrlKey || e.altKey) return;
    // Anywhere inside a row (its checkbox, buttons, the task line); fields keep
    // their keys (isTyping), and Enter / Space on a button keep their meaning.
    const el = e.target as HTMLElement;
    const row = el.closest?.("[data-inbox-row]") as HTMLElement | null;
    if (!row) return;
    const onRowItself = el === row || el.matches?.("[data-task-row]");
    if ((e.key === "Enter" || e.key === " ") && !onRowItself) return;
    const rowId = row.getAttribute("data-inbox-row");
    const ids = open.flatMap((t) => (t.id ? [t.id] : []));
    const here = rowId ?? cursor;
    const i = here ? ids.indexOf(here) : -1;
    const t = open.find((x) => x.id === here);
    const k = e.key;
    if (k === "ArrowDown" || k === "j") focusRow(ids[Math.min(ids.length - 1, i + 1)] ?? null);
    else if (k === "ArrowUp" || k === "k") {
      if (i <= 0) field.current?.focus();
      else focusRow(ids[i - 1] ?? null);
    } else if (k === "Enter" && t) accept(t);
    else if (k === ">") {
      if (selected.length > 0) field.current?.setText("> ");
      else if (t?.id) field.current?.setText(`^${t.id} > `);
    } else if (k === "t") {
      if (selected.length > 1) field.current?.setText("/temat  > ", { caret: 7 });
      else if (t?.id) field.current?.setText(`/temat ^${t.id} > `);
    } else if (k === "/" && t?.id) {
      // A command for this entry: `/czeka …`, `/powiaz …` apply to it.
      setCursor(t.id);
      field.current?.setText("/");
    } else if (k === " " && t?.id) toggle(t.id);
    else if (k === "?" && t?.id) void askAgent("command", "", [t.id]);
    else if (k === "Escape" && selected.length > 0) setSelected([]);
    else return;
    e.preventDefault();
    e.stopPropagation();
  };

  // `u` anywhere in the view (not while typing, not with a dialog open): the
  // last Skrzynka write, also when its entry already left the list.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!isUndoKey(e, isTyping(document.activeElement), !!document.querySelector('[aria-modal="true"]'))) return;
      e.preventDefault();
      void undoLast();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function undoLast(token: number | null = null) {
    try {
      // Only Skrzynka's own writes (not the quick window's or ⌘K's).
      const r = await api.commandUndo(token, token === null ? "inbox" : null);
      const v = describeUndo(r, token);
      notify(v.failed ? tr("{0} Naciśnij u albo „Cofnij”, żeby spróbować jeszcze raz.", v.text) : v.text);
      setLastWrite(v.failed ? { reading: r.reading, token: v.retry } : null);
    } catch (e) {
      notify(describeError(e));
      // Nothing left to undo (taken back elsewhere, or older than the last 20).
      setLastWrite(null);
    }
  }

  const running = agentRun && runs.list.find((r) => r.id === agentRun.id)?.status === "running";
  // The focused entry (↑↓ on the list): what `/czeka …` with no subject applies to.
  const focused = cursor ? open.find((t) => t.id === cursor) ?? null : null;
  const mockText = mock && mock !== "echo" ? MOCK_COMMAND_TEXT[mock].text : "";

  return (
    <div className="view inbox-view">
      <header className="view-head">
        <h1 className="view-title">{tr("Skrzynka")}</h1>
        <p className="view-sub">
          {tr("Zapisz myśl lub zadanie. Kliknij wpis, żeby zobaczyć szczegóły i przypisać go do projektu lub tematu.")}</p>
      </header>

      <details className="ibx-help"><summary>{tr("Komendy i pomoc agenta")}</summary><p>{tr("Możesz też użyć komend:")} <code>{tr("> miejsce")}</code>  {tr("przenosi wpis,")} <code>{tr("/temat")}</code>  {tr("tworzy temat,")} <code>{tr("/czeka")}</code>  {tr("ustawia oczekiwanie,")} <code>{tr("/powiaz")}</code>  {tr("dodaje powiązanie. Agenta włączysz przełącznikiem przy polu, skrótem")} <kbd>{MODE_SHORTCUT}</kbd>  {tr("lub znakiem")} <code>?</code>  {tr("na początku.")}</p></details>

      <section className="ibx-field" aria-label={tr("Nowe zadanie albo komenda")}>
        {fieldMode === "agent" ? (
          <AgentField
            ref={agentField}
            providers={providers ?? []}
            provider={provider()}
            onProvider={(p) => {
              writePref(PROVIDER_PREF, p);
              setProviderPref(p);
            }}
            tools={tools}
            onTools={chooseTools}
            consentPending={consentPending}
            onConsent={(ok) => {
              setConsentPending(false);
              if (ok) {
                writePref(MY_TOOLS_CONSENT_KEY, "1");
                setTools("mine");
              }
              requestAnimationFrame(() => agentField.current?.focus());
            }}
            context={contextLabel(selected.length, selected.length ? null : (focused?.title ?? null), open.filter((t) => t.id).length)}
            busy={!!running}
            initialText={agentText}
            onSubmit={(t) => submitAgent(t)}
            onTerminal={(t) => submitAgent(t, true)}
            onMode={(m) => switchMode(m)}
          />
        ) : (
        <CommandField
          ref={field}
          variant="inbox"
          source="inbox"
          selection={selected}
          focus={focused?.id ?? null}
          placeholder={
            selected.length
              ? tr("Komenda dla zaznaczonych ({0}): > miejsce, /temat Nazwa > projekt, [x]…", selected.length)
              : focused
                ? tr("Dodaj do skrzynki albo komenda dla „{0}”: /czeka, /powiaz, > miejsce…", focused.title)
                : tr("Dodaj do skrzynki albo wpisz komendę…")
          }
          autoFocus
          initialText={mockText}
          startConfirmed={mock === "confirm" || mock === "topic"}
          initialPartial={
            mock === "partial"
              ? {
                  total: 2,
                  undo: 1,
                  applied: {
                    steps: [
                      { summary: tr("„Webhook zwrotów zwraca 500 na produkcji”: przeniesie z: Skrzynka do: WhenUBuy / Płatności: Stripe → Przelewy24"), ok: true, message: "" },
                      { summary: tr("„Opisać instalację serwera MCP w README”: przeniesie z: Skrzynka do: WhenUBuy / Płatności: Stripe → Przelewy24"), ok: false, message: "" },
                    ],
                    echo: [],
                    error: tr("Zadanie zmieniło się od podglądu; pominięte."),
                    remaining: [],
                    rolled_back: false,
                    left_written: [
                      tr("„Webhook zwrotów zwraca 500 na produkcji”: przeniesie z: Skrzynka do: WhenUBuy / Płatności: Stripe → Przelewy24 (Zadanie „Webhook zwrotów zwraca 500 na produkcji” zmieniło się w międzyczasie; zostawione bez zmian.)"),
                    ],
                  },
                }
              : undefined
          }
          initialEcho={
            mock === "echo"
              ? [{ file: "whenubuy/tematy/platnosci-stripe-przelewy24.md", line: 14, text: "- [ ] Podpiąć zapisane karty !wysoki ^q8w7e6" }]
              : undefined
          }
          note={agentNote}
          controls={<ModeSwitch mode="command" onChange={(m) => switchMode(m)} />}
          onTextChange={(t) => {
            if (agentNote) setAgentNote(null);
            // `?` at the start flips to the agent (the rest of the text goes along).
            if (t.startsWith("?")) switchMode("agent", t.slice(1).trimStart());
          }}
          onAgent={onAgent}
          // Esc in an empty field forgets the focused entry (the implied subject).
          onEscape={() => setCursor(null)}
          onArrowDown={() => focusRow(open.find((t) => t.id)?.id ?? null)}
          onWritten={(w) => {
            setAgentNote(null);
            if (w.undo !== null) setLastWrite({ reading: w.reading, token: w.undo });
          }}
        />
        )}
      </section>

      {terminalRun && (
        <div className="ibx-terminal" role="status">
          <p className="ibx-terminal-text">
            {tr("Sesja")} {PROVIDER_NAME[terminalRun.provider]}  {tr("jest otwarta w Terminalu. Gdy agent skończy, zapisze odpowiedź do pliku, a propozycje pojawią się tutaj do zaznaczenia (sprawdzam co kilka sekund). Jeśli agent pokazał odpowiedź, ale nie zapisał pliku, wklej ją.")}</p>
          {pasting ? (
            <div className="ibx-paste">
              <textarea
                className="ibx-paste-text"
                value={pasteText}
                autoFocus
                aria-label={tr("Odpowiedź agenta z Terminala")}
                placeholder={tr("Wklej całą odpowiedź agenta, razem z blokiem montybrain-zmiany…")}
                onChange={(e) => setPasteText(e.target.value)}
              />
              <div className="ibx-paste-actions">
                <Button variant="quiet" disabled={!pasteText.trim()} onClick={() => void pasteAnswer()}>
                  {tr("Wczytaj propozycje")}</Button>
                <Button variant="quiet" onClick={() => setPasting(false)}>
                  {tr("Anuluj")}</Button>
              </div>
            </div>
          ) : (
            <div className="ibx-paste-actions">
              <Button variant="quiet" onClick={() => setPasting(true)}>
                {tr("Wklej odpowiedź…")}</Button>
              <Button variant="quiet" onClick={() => setTerminalRun(null)}>
                {tr("Przestań czekać")}</Button>
            </div>
          )}
        </div>
      )}

      {lastWrite && (
        <p className="ibx-last" role="status">
          <span className="ibx-last-text">{tr("Ostatni zapis:")} {message(lastWrite.reading)}</span>
          <button type="button" className="cf-link" onClick={() => void undoLast(lastWrite.token)}>
            {tr("Cofnij")} <kbd>u</kbd>
          </button>
        </p>
      )}

      {selected.length > 0 && (
        <div className="ibx-bar" role="toolbar" aria-label={tr("Zaznaczone wpisy")}>
          <span className="ibx-bar-count">{tr("Zaznaczone:")} {selected.length}</span>
          <Button variant="quiet" onClick={() => field.current?.setText("> ")}>
            {tr("Przenieś wszystkie…")} <kbd>&gt;</kbd>
          </Button>
          <Button variant="quiet" disabled={selected.length < 2} onClick={() => field.current?.setText("/temat  > ", { caret: 7 })}>
            {tr("Jeden temat z wpisów…")} <kbd>t</kbd>
          </Button>
          <Button variant="quiet" onClick={() => void askAgent("proposals", tr("Zaproponuj miejsca dla tych wpisów."), selected)}>
            {tr("Zapytaj agenta")}</Button>
          <Button variant="quiet" onClick={() => setSelected([])}>
            {tr("Odznacz")} <kbd>esc</kbd>
          </Button>
        </div>
      )}

      {running && (
        <p className="ibx-agent" role="status">
          {tr("Agent pracuje nad")} {agentRun?.mode === "command" ? tr("komendą dla wpisu") : tr("propozycjami")}{tr("… Dostaje tylko wybrane wpisy i ich podpowiedzi.")}</p>
      )}

      {pendingProposal && (
        <section className="ibx-review" aria-label={tr("Propozycje agenta")}>
          <Review project={INBOX_KEY} proposal={pendingProposal} autoFocus={false} onApplied={() => {}} onDecided={() => {}} onUndone={() => {}} />
        </section>
      )}

      <section className="group" aria-label={tr("Wpisy w skrzynce")}>
        <div className="ibx-list" ref={listRef} onKeyDownCapture={onListKey}>
          {open.length === 0 && (
            <div className="empty compact">
              <p className="empty-title">{tr("Skrzynka pusta.")}</p>
              <p className="empty-text">{tr("Zapisz tu cokolwiek przyjdzie do głowy — porządek zrobisz później.")}</p>
            </div>
          )}
          {open.map((t) => {
            const id = t.id;
            const sel = !!id && selected.includes(id);
            return (
              <div
                key={id ?? `l:${t.line}`}
                className={`ibx-row${cursor === id ? " active" : ""}${sel ? " selected" : ""}`}
                data-inbox-row={id ?? undefined}
                tabIndex={-1}
                onFocus={() => setCursor(id)}
              >
                <button
                  type="button"
                  className="ibx-check"
                  role="checkbox"
                  aria-checked={sel}
                  aria-label={tr("Zaznacz: {0}", t.title)}
                  disabled={!id}
                  onClick={() => id && toggle(id)}
                >
                  {sel && (
                    <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden>
                      <path d="M2.5 6.2l2.3 2.3 4.7-5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
                    </svg>
                  )}
                </button>
                <div className="ibx-task">
                  <TaskRow task={t} />
                </div>
                <Suggestion s={id ? suggestions.get(id) : undefined} onAccept={() => accept(t)} onAsk={() => id && void askAgent("command", "", [id])} />
              </div>
            );
          })}
        </div>
        {done.length > 0 && (
          <div className="done-group">
            <button type="button" className="done-toggle" aria-expanded={showDone} onClick={() => setShowDone(!showDone)}>
              <ChevronRight size={12} className="chev" />
              {tr("Zrobione")} <span className="count">{done.length}</span>
            </button>
            {showDone && done.map((t) => <TaskRow key={t.id ?? `d:${t.line}`} task={t} />)}
          </div>
        )}
      </section>
      {open.length > 0 && <details className="ibx-help ibx-shortcuts"><summary>{tr("Skróty klawiaturowe")}</summary><p className="ibx-keys">
        <kbd>↑</kbd>
        <kbd>↓</kbd>  {tr("wybór ·")} <kbd>↵</kbd>  {tr("przyjmij podpowiedź ·")} <kbd>&gt;</kbd>  {tr("miejsce ·")} <kbd>t</kbd>  {tr("temat ·")} <kbd>/</kbd>  {tr("komenda dla wpisu ·")} <kbd>{tr("spacja")}</kbd>  {tr("zaznacz ·")} <kbd>u</kbd>  {tr("cofnij ·")} <kbd>?</kbd> agent
      </p></details>}
    </div>
  );
}
