// The in-app agent session ("Moje narzędzia"), as an overlay over any view:
// the conversation, one compact line per tool call (expandable), the
// permission requests as cards (Zezwól / Zezwól na tę sesję / Odmów), a box
// for follow-up messages, "Przerwij" and "Zakończ". The session runs in the
// backend (src-tauri runs/session); closing the panel does not stop it, the
// sidebar indicator brings it back.

import { Bot, ChevronRight, Lock, X } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";

import type { SessionEntry, SessionView } from "../api/types";
import { describeError } from "../lib/errors";
import { CARD_STATE_LABEL, STATUS_LABEL, canInterrupt, canSend, cardActions, displayText, indicator, isLive, pendingCards, providerName, proposalPath, toolLabel, type Card } from "../lib/session";
import { links, navigate } from "../router/router";
import { sessionStore, useSession } from "../store/session";
import { useOptionalVault } from "../store/vault";
import { Markdown } from "../views/actions/Markdown";
import "./session.css";

function ToolLine({ e }: { e: Extract<SessionEntry, { kind: "tool" }> }) {
  const [open, setOpen] = useState(false);
  const hasMore = !!(e.detail || e.output);
  return (
    <li className={`ags-tool st-${e.status}`}>
      <button type="button" className="ags-tool-head" aria-expanded={hasMore ? open : undefined} disabled={!hasMore} onClick={() => setOpen(!open)}>
        <ChevronRight size={11} className={`chev${open ? " open" : ""}`} aria-hidden />
        <span className="ags-tool-name">{toolLabel(e.name)}</span>
        {e.summary && <span className="ags-tool-sum">{e.summary}</span>}
        <span className="ags-tool-st">{e.status === "running" ? "…" : e.status === "failed" ? "błąd" : "✓"}</span>
      </button>
      {open && (
        <div className="ags-tool-more">
          {e.detail && <pre className="ags-pre">{e.detail}</pre>}
          {e.output && (
            <>
              <p className="ags-tool-label">Wynik</p>
              <pre className="ags-pre">{e.output}</pre>
            </>
          )}
        </div>
      )}
    </li>
  );
}

function CardView({ c, onError, autoFocus }: { c: Card; onError: (m: string) => void; autoFocus: boolean }) {
  const [busy, setBusy] = useState(false);
  const [open, setOpen] = useState(c.state === "pending");
  const first = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (autoFocus) first.current?.focus();
  }, [autoFocus]);
  async function act(answer: "allow" | "allow_session" | "deny", answers?: Record<string, string>) {
    setBusy(true);
    try {
      await sessionStore.answer(c.id, answer, answers);
    } catch (e) {
      onError(describeError(e));
    } finally {
      setBusy(false);
    }
  }
  if (c.state === "auto_denied") {
    return (
      <li className="ags-card locked">
        <p className="ags-card-line">
          <Lock size={11} aria-hidden /> <span className="ags-card-title">{c.title}</span>
        </p>
        <p className="ags-card-note">{c.note}</p>
      </li>
    );
  }
  if (c.state !== "pending") {
    return (
      <li className={`ags-card done st-${c.state}`}>
        <button type="button" className="ags-card-line" aria-expanded={open} onClick={() => setOpen(!open)}>
          <span className="ags-card-state">{CARD_STATE_LABEL[c.state]}</span> <span className="ags-card-title">{c.title}</span>
          {c.note && c.questions.length > 0 && <span className="ags-card-note-inline"> → {c.note}</span>}
        </button>
        {open && c.detail && <pre className="ags-pre">{c.detail}</pre>}
      </li>
    );
  }
  const actions = cardActions(c);
  return (
    <li className="ags-card pending" role="group" aria-label={`Prośba o zgodę: ${c.title}`}>
      <p className="ags-card-ask">{c.questions.length > 0 ? "Agent pyta" : "Agent prosi o zgodę"}</p>
      <p className="ags-card-title strong">{c.title}</p>
      {c.detail && <pre className="ags-pre">{c.detail}</pre>}
      {c.questions.map((q, qi) => (
        <div key={q.id} className="ags-question">
          {q.header && <p className="ags-q-head">{q.header}</p>}
          <p className="ags-q-text">{q.question}</p>
          <div className="ags-actions">
            {q.options.map((o, i) => (
              <button key={o} ref={qi === 0 && i === 0 ? first : undefined} type="button" className="ags-btn" disabled={busy} onClick={() => void act("allow", { [q.id]: o })}>
                {o}
              </button>
            ))}
          </div>
        </div>
      ))}
      <div className="ags-actions">
        {actions.map((a, i) => (
          <button key={a.answer} ref={i === 0 && c.questions.length === 0 ? first : undefined} type="button" className={`ags-btn${a.answer === "allow" ? " primary" : ""}`} disabled={busy} onClick={() => void act(a.answer)}>
            {a.label}
          </button>
        ))}
        {c.questions.length > 0 && (
          <button type="button" className="ags-btn" disabled={busy} onClick={() => void act("deny")}>
            Nie odpowiadaj
          </button>
        )}
      </div>
    </li>
  );
}

function Entries({ view, onError }: { view: SessionView; onError: (m: string) => void }) {
  const firstPending = pendingCards(view)[0]?.id ?? null;
  return (
    <ol className="ags-entries" aria-label="Przebieg sesji">
      {view.entries.map((e) => {
        switch (e.kind) {
          case "user":
            return (
              <li key={e.id} className="ags-user">
                <span className="ags-who">Ty</span>
                <p className="ags-user-text">{e.text}</p>
              </li>
            );
          case "assistant":
            return (
              <li key={e.id} className="ags-assistant">
                <Markdown text={displayText(e.text)} />
              </li>
            );
          case "tool":
            return <ToolLine key={e.id} e={e} />;
          case "permission":
            return <CardView key={e.id} c={e} onError={onError} autoFocus={e.id === firstPending} />;
          case "notice":
            return (
              <li key={e.id} className={`ags-notice${e.error ? " error" : ""}`}>
                {e.text}
              </li>
            );
        }
      })}
    </ol>
  );
}

export function AgentSessionPanel() {
  const { view, open } = useSession();
  const vault = useOptionalVault();
  const notify = vault?.notify ?? (() => {});
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const bodyRef = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const panelRef = useRef<HTMLElement>(null);

  useEffect(() => {
    void sessionStore.start().catch(() => {});
  }, []);

  // Follow the conversation unless the user scrolled up to read.
  useLayoutEffect(() => {
    const el = bodyRef.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [view?.entries.length, view?.entries[view.entries.length - 1], open]);

  if (!open || !view) return null;
  const live = isLive(view.status);
  const onError = (m: string) => notify(m);

  async function send() {
    const t = text.trim();
    if (!t || !canSend(view)) return;
    setSending(true);
    try {
      await sessionStore.send(t);
      setText("");
      stick.current = true;
    } catch (e) {
      notify(describeError(e));
    } finally {
      setSending(false);
    }
  }
  const run = (f: () => Promise<void>) => () => void f().catch((e) => notify(describeError(e)));
  const where = proposalPath(view);

  return (
    <aside
      ref={panelRef}
      className="ags"
      role="dialog"
      aria-label="Sesja agenta"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          sessionStore.hide();
        }
      }}
    >
      <header className="ags-head">
        <div className="ags-head-main">
          <p className="ags-eyebrow">
            <Bot size={12} aria-hidden /> Sesja agenta · {providerName(view.provider)} · moje narzędzia
          </p>
          <p className="ags-title" title={view.request}>
            {view.request}
          </p>
          <p className={`ags-status st-${view.status}`} aria-live="polite">
            <span className="ags-dot" aria-hidden />
            {STATUS_LABEL[view.status]}
            {view.remembered.length > 0 && <span className="ags-remembered"> · zgoda na tę sesję: {view.remembered.map(toolLabel).join(", ")}</span>}
          </p>
        </div>
        <div className="ags-head-actions">
          {canInterrupt(view) && (
            <button type="button" className="ags-btn" onClick={run(sessionStore.interrupt)}>
              Przerwij
            </button>
          )}
          {live ? (
            <button type="button" className="ags-btn danger" onClick={run(sessionStore.stop)}>
              Zakończ
            </button>
          ) : (
            <button type="button" className="ags-btn" onClick={run(sessionStore.dismiss)}>
              Zamknij sesję
            </button>
          )}
          <button type="button" className="ags-icon" aria-label="Schowaj panel (sesja działa dalej)" title="Schowaj (esc) — sesja działa dalej" onClick={() => sessionStore.hide()}>
            <X size={14} />
          </button>
        </div>
      </header>
      <p className="ags-reach">MontyBrain jest dla agenta tylko do odczytu; inne narzędzia pytają o zgodę tutaj. Zmiany wracają jako propozycje do zaznaczenia.</p>
      <div
        className="ags-body"
        ref={bodyRef}
        onScroll={(e) => {
          const el = e.currentTarget;
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
        }}
      >
        <Entries view={view} onError={onError} />
        {view.error && (
          <p className="ags-error" role="alert">
            {view.error}
          </p>
        )}
      </div>
      {view.proposal && (
        <div className="ags-proposal" role="status">
          <span>{view.proposal.count === 0 ? "Agent odpowiedział bez propozycji zmian." : `Propozycje (${view.proposal.count}) czekają ${"inbox" in where ? "w Skrzynce" : "w widoku projektu"}.`}</span>
          {view.proposal.count > 0 && (
            <button
              type="button"
              className="ags-btn primary"
              onClick={() => {
                navigate("inbox" in where ? links.inbox() : links.project(where.project));
                sessionStore.hide();
              }}
            >
              Pokaż propozycje
            </button>
          )}
        </div>
      )}
      <div className="ags-foot">
        <textarea
          className="field ags-input"
          rows={2}
          value={text}
          disabled={!live || sending}
          aria-label="Wiadomość do agenta"
          placeholder={live ? "Napisz do agenta: odpowiedź, poprawka albo kolejna prośba… (Enter: wyślij, Shift+Enter: nowa linia)" : "Sesja się zakończyła."}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void send();
            }
          }}
        />
        <button type="button" className="ags-btn primary" disabled={!live || sending || !text.trim()} onClick={() => void send()}>
          Wyślij
        </button>
      </div>
    </aside>
  );
}

/** The sidebar line while a session exists: opens the panel. */
export function SessionIndicator() {
  const { view, open } = useSession();
  useEffect(() => {
    void sessionStore.start().catch(() => {});
  }, []);
  const i = indicator(view);
  if (!i || open) return null;
  return (
    <button type="button" className={`ags-indicator${i.attention ? " attention" : ""}`} onClick={() => sessionStore.open()}>
      <span className="ags-dot" aria-hidden />
      {i.text}
    </button>
  );
}
