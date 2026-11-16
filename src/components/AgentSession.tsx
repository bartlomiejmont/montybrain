import { t as tr } from "../i18n";
import { lockInert } from "../lib/inert";
// The in-app agent session ("Moje narzędzia"), beside the current workspace:
// the conversation, one compact line per tool call (expandable), the
// permission requests as cards (Zezwól / Zezwól na tę sesję / Odmów), a box
// for follow-up messages, "Przerwij" and "Zakończ". The session runs in the
// backend (src-tauri runs/session); closing the panel does not stop it, the
// sidebar indicator brings it back.

import { ArrowDown, ArrowUp, Bot, Check, ChevronRight, CircleAlert, LoaderCircle, Lock, MoreHorizontal, Plus, Square, X } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";

import type { SessionEntry, SessionView } from "../api/types";
import { describeError } from "../lib/errors";
import { CARD_STATE_LABEL, STATUS_LABEL, canInterrupt, canSend, cardActions, displayText, indicator, isLive, pendingCards, providerName, proposalPath, toolLabel, completeAnswers, scopeLabel, proposalPending, groupSessionEntries, type TimelineEntry, type ToolEntry, type SessionDraft, type Card } from "../lib/session";
import { links, navigate, useRoute } from "../router/router";
import { sessionStore, useSession } from "../store/session";
import { useOptionalVault } from "../store/vault";
import { updateStore, useUpdates } from "../store/updates";
import { SessionStart } from "./SessionStart";
import { AgentTelemetry } from "./AgentTelemetry";
import { Markdown } from "../views/actions/Markdown";
import { Menu } from "./Menu";
import { useAutosizeTextarea } from "./ui";
import "./session.css";

function ToolStatus({ status }: { status: ToolEntry["status"] }) {
  const Icon = status === "running" ? LoaderCircle : status === "failed" ? CircleAlert : Check;
  return <span className={`ags-tool-st st-${status}`} role="img" aria-label={status === "running" ? tr("W toku") : status === "failed" ? tr("Błąd") : tr("Gotowe")}><Icon size={13} className={status === "running" ? "ags-spin" : undefined} aria-hidden /></span>;
}

function WorkGroup({ group }: { group: Extract<TimelineEntry, { kind: "work" }> }) {
  const [open, setOpen] = useState(false);
  return <li className={`ags-work st-${group.status}`}>
    <button type="button" className="ags-work-head" aria-expanded={open} onClick={() => setOpen(!open)}>
      <ChevronRight size={12} className={`chev${open ? " open" : ""}`} aria-hidden />
      <ToolStatus status={group.status} />
      <span>{tr("Wywołania:")} {group.tools.length}</span>
      <span className="ags-work-latest">{toolLabel(group.latest.name)}{group.latest.summary && ` · ${group.latest.summary}`}</span>
    </button>
    {open && <ol className="ags-tools" aria-label={tr("Wywołania narzędzi")}>{group.tools.map((e) => <ToolLine key={e.id} e={e} />)}</ol>}
  </li>;
}

function ToolLine({ e }: { e: Extract<SessionEntry, { kind: "tool" }> }) {
  const [open, setOpen] = useState(false);
  const hasMore = !!(e.detail || e.output);
  return (
    <li className={`ags-tool st-${e.status}`}>
      <button type="button" className="ags-tool-head" aria-expanded={hasMore ? open : undefined} disabled={!hasMore} onClick={() => setOpen(!open)}>
        <ChevronRight size={11} className={`chev${open ? " open" : ""}`} aria-hidden />
        <span className="ags-tool-name">{toolLabel(e.name)}</span>
        {e.summary && <span className="ags-tool-sum">{e.summary}</span>}
        <ToolStatus status={e.status} />
      </button>
      {open && (
        <div className="ags-tool-more">
          {e.detail && <pre className="ags-pre">{e.detail}</pre>}
          {e.output && (
            <>
              <p className="ags-tool-label">{tr("Wynik")}</p>
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
  const first = useRef<HTMLButtonElement | HTMLInputElement>(null);
  const [answers, setAnswers] = useState<Record<string, string>>({});
  const complete = completeAnswers(c, answers);
  useEffect(() => {
    if (autoFocus) first.current?.focus();
  }, [autoFocus]);
  async function act(answer: "allow" | "allow_session" | "deny", answers?: Record<string, string>) {
    setBusy(true);
    onError("");
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
    <li className="ags-card pending" role="group" aria-label={tr("Prośba o zgodę: {0}", c.title)}>
      <p className="ags-card-ask">{c.questions.length > 0 ? tr("Agent pyta") : tr("Agent prosi o zgodę")}</p>
      <p className="ags-card-title strong">{c.title}</p>
      {c.detail && <pre className="ags-pre">{c.detail}</pre>}
      {c.questions.map((q, qi) => (
        <div key={q.id} className="ags-question">
          {q.header && <p className="ags-q-head">{q.header}</p>}
          <p className="ags-q-text">{q.question}</p>
          <div className="ags-actions">
            {q.options.map((o, i) => (
              <button key={o} ref={qi === 0 && i === 0 ? (el) => { first.current = el; } : undefined} type="button" className="btn btn-secondary" disabled={busy} aria-pressed={answers[q.id] === o} onClick={() => setAnswers((a) => ({ ...a, [q.id]: o }))}>
                {o}
              </button>
            ))}
          </div>
          <input className="field ags-answer" aria-label={tr("Twoja odpowiedź: {0}", q.question)} placeholder={tr("Wybierz opcję lub wpisz własną odpowiedź")}
            ref={qi === 0 && q.options.length === 0 ? (el) => { first.current = el; } : undefined}
            disabled={busy} value={answers[q.id] ?? ""} onChange={(e) => setAnswers((a) => ({ ...a, [q.id]: e.target.value }))} />
        </div>
      ))}
      <div className="ags-actions">
        {actions.map((a, i) => (
          <button key={a.answer} ref={i === 0 && c.questions.length === 0 ? (el) => { first.current = el; } : undefined} type="button" className={`btn ${a.answer === "allow" ? "btn-primary" : "btn-secondary"}`} disabled={busy} onClick={() => void act(a.answer)}>
            {a.label}
          </button>
        ))}
        {c.questions.length > 0 && <button type="button" className="btn btn-primary" disabled={busy || !complete} onClick={() => complete && void act("allow", complete)}>{tr("Odpowiedz")}</button>}
        {c.questions.length > 0 && (
          <button type="button" className="btn btn-secondary" disabled={busy} onClick={() => void act("deny")}>
            {tr("Nie odpowiadaj")}</button>
        )}
      </div>
    </li>
  );
}

function Entries({ view, onError }: { view: SessionView; onError: (m: string) => void }) {
  return (
    <ol className="ags-entries" aria-label={tr("Przebieg sesji")}>
      {groupSessionEntries(view.entries).map((e) => {
        switch (e.kind) {
          case "user":
            return (
              <li key={e.id} className="ags-user" aria-label={tr("Twoja wiadomość")}>
                <p className="ags-user-text">{e.text}</p>
              </li>
            );
          case "assistant":
            return (
              <li key={e.id} className="ags-assistant">
                <Markdown text={displayText(e.text)} />
              </li>
            );
          case "work":
            return <WorkGroup key={e.id} group={e} />;
          case "permission":
            return e.state === "pending" ? null : <CardView key={e.id} c={e} onError={onError} autoFocus={false} />;
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
  const { view, open, draft, composing, context, busy } = useSession();
  const vault = useOptionalVault();
  const route = useRoute();
  const updates = useUpdates();
  const [text, setText] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [sending, setSending] = useState(false);
  const [unread, setUnread] = useState(false);
  const [menuAnchor, setMenuAnchor] = useState<HTMLElement | null>(null);
  const [error, setError] = useState("");
  const [narrow, setNarrow] = useState(() => window.matchMedia("(max-width: 1279px)").matches);
  const bodyRef = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const panelRef = useRef<HTMLElement>(null);
  const projects = vault?.view?.snapshot.projects ?? [];
  const topic = vault?.view?.snapshot.topics.find((t) => t.project === route.params.project && t.slug === route.query.get("temat"));
  const defaults: SessionDraft = { scope: route.params.project ? "project" : "inbox", project: route.params.project, topic: topic ? { slug: topic.slug, title: topic.title } : undefined, text: "", origin: window.location.hash };
  const form = composing || !view;
  const inputRef = useAutosizeTextarea(text, open && !form);
  const pending = proposalPending(view, updates.byProject, updates.loaded);

  useEffect(() => {
    void sessionStore.start().catch((e) => setError(describeError(e))).finally(() => setLoaded(true));
    void updateStore.start().catch((e) => setError(describeError(e)));
    const media = window.matchMedia("(max-width: 1279px)");
    const resize = () => setNarrow(media.matches);
    media.addEventListener("change", resize);
    return () => media.removeEventListener("change", resize);
  }, []);
  useEffect(() => { setText(""); setError(""); setUnread(false); stick.current = true; }, [view?.id]);
  useEffect(() => {
    if (loaded && open && form && !draft) sessionStore.compose(defaults);
  }, [loaded, open, form, draft]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && !e.altKey && e.key.toLowerCase() === "j" && !document.querySelector(".palette-overlay")) {
        // A palette or another modal owns its keyboard until it closes.
        if (document.querySelector('[aria-modal="true"]:not(.ags)')) return;
        e.preventDefault();
        if (sessionStore.getSnapshot().open) sessionStore.hide();
        else sessionStore.open();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  useLayoutEffect(() => {
    if (!open) return;
    const before = document.activeElement as HTMLElement | null;
    return () => {
      if (before?.isConnected) requestAnimationFrame(() => before.focus());
    };
  }, [open]);
  useLayoutEffect(() => {
    if (!open || !loaded) return;
    const frame = requestAnimationFrame(() => {
      (panelRef.current?.querySelector<HTMLElement>(".ags-pending button:not(:disabled), .ags-pending input:not(:disabled)") ?? panelRef.current?.querySelector<HTMLElement>("[data-session-focus]:not(:disabled)") ?? panelRef.current?.querySelector<HTMLElement>("textarea:not(:disabled), button:not(:disabled)") ?? panelRef.current)?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [open, form, loaded]);
  useEffect(() => {
    if (!open || !narrow) return;
    const background = [...document.querySelectorAll<HTMLElement>(".app > .sidebar, .app > .main")];
    return lockInert(background);
  }, [open, narrow]);
  useLayoutEffect(() => {
    const el = bodyRef.current;
    if (el && !form) {
      if (stick.current) { el.scrollTop = el.scrollHeight; setUnread(false); }
      else setUnread(true);
    }
  }, [view?.entries.length, view?.entries[view.entries.length - 1], open, form]);
  useLayoutEffect(() => {
    if (form && bodyRef.current) bodyRef.current.scrollTop = 0;
  }, [form, draft?.topic?.slug]);

  if (!open) return null;
  const live = !!view && isLive(view.status);
  const cards = pendingCards(view);
  const interruptible = canInterrupt(view);
  const working = view?.status === "working" || view?.status === "starting";
  const label = view ? scopeLabel(view.scope, projects) : "";
  const run = (f: () => Promise<void>) => () => void f().catch((e) => setError(describeError(e)));
  async function send() {
    const t = text.trim();
    if (!t || !canSend(view) || sending) return;
    setSending(true);
    stick.current = true;
    setError("");
    try { await sessionStore.send(t); setText(""); stick.current = true; }
    catch (e) { setError(describeError(e)); }
    finally { setSending(false); }
  }
  const review = () => {
    if (!view) return;
    const where = proposalPath(view);
    navigate("inbox" in where ? links.inbox() : links.project(where.project));
    sessionStore.hide();
  };
  return <>
    {narrow && <div className="ags-backdrop" aria-hidden onClick={() => sessionStore.hide()} />}
    <aside ref={panelRef} className="ags" role="dialog" aria-modal={narrow || undefined} aria-label={tr("Sesja agenta")} tabIndex={-1}
      onKeyDown={(e) => {
        if (e.key === "Escape") { e.stopPropagation(); sessionStore.hide(); }
        if (narrow && e.key === "Tab") {
          const els = [...e.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], [tabindex="0"]')].filter((el) => el.getClientRects().length > 0);
          const first = els[0], last = els.at(-1);
          if (!first) { e.preventDefault(); e.currentTarget.focus(); }
          else if (e.shiftKey && (document.activeElement === first || document.activeElement === e.currentTarget)) { e.preventDefault(); last?.focus(); }
          else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
        }
      }}>
      <header className="ags-head">
        <span className={`ags-status st-${form ? "idle" : view!.status}`} title={form ? tr("Nowa sesja") : STATUS_LABEL[view!.status]} aria-live="polite">
          <span className="ags-dot" aria-hidden /><span className="sr-only">{form ? tr("Nowa sesja") : STATUS_LABEL[view!.status]}</span>
        </span>
        <h2 className="ags-title" title={form ? tr("Nowa sesja agenta") : context?.text || view!.request}>{form ? tr("Nowa sesja") : context?.text || view!.request}</h2>
        {!form && view && <span className="ags-provider">{providerName(view.provider)}</span>}
        <div className="ags-head-actions">
          {!form && view && <>
            <button type="button" className="icon-btn" title={tr("Nowa sesja")} aria-label={tr("Nowa sesja")} disabled={busy} onClick={() => sessionStore.compose(defaults)}><Plus size={16} aria-hidden /></button>
            <button type="button" className="icon-btn" title={tr("Opcje sesji")} aria-label={tr("Opcje sesji")} aria-haspopup="menu" aria-expanded={!!menuAnchor} onClick={(e) => setMenuAnchor(e.currentTarget)}><MoreHorizontal size={16} aria-hidden /></button>
          </>}
          <button type="button" className="icon-btn" aria-label={tr("Schowaj panel (sesja działa dalej)")} title={tr("Schowaj (Esc) — sesja działa dalej")} onClick={() => sessionStore.hide()}><X size={16} aria-hidden /></button>
        </div>
      </header>
      {!form && view && <div className="ags-context-line"><span title={`${label}${context?.topic ? ` · ${context.topic.title}` : ""}`}>{label}{context?.topic && ` · ${context.topic.title}`}</span>{context?.origin && <button type="button" className="btn btn-quiet" title={tr("Wróć do miejsca rozpoczęcia")} onClick={() => { navigate(context.origin!); sessionStore.hide(); }}>{tr("Wróć do źródła")}</button>}</div>}
      {menuAnchor && view && <Menu portalRoot={panelRef.current ?? undefined} anchor={menuAnchor} label={tr("Opcje sesji")} onClose={() => setMenuAnchor(null)} items={[
        ...(view.remembered.length ? [{ key: "consent", label: tr("Zgody: {0}", view.remembered.map(toolLabel).join(", ")), disabled: true, onSelect: () => {} }] : []),
        { key: "end", label: live ? tr("Zakończ sesję") : tr("Wyczyść rozmowę"), disabled: busy, onSelect: run(live ? sessionStore.stop : sessionStore.dismiss) },
      ]} />}
      {!form && view && <AgentTelemetry view={view} collapsible />}
      <div className="ags-body" ref={bodyRef} onScroll={(e) => { const el = e.currentTarget; stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40; if (stick.current) setUnread(false); }}>
        {form && draft ? <SessionStart draft={draft} current={view} busy={busy} onError={setError} /> : view && <>
          <Entries key={view.id} view={view} onError={setError} />
          {working && <p className="ags-working" role="status"><LoaderCircle size={14} className="ags-spin" aria-hidden />{tr("Agent pracuje…")}</p>}
          {view.error && <p className="ags-error" role="alert">{describeError(view.error)}</p>}
        </>}
      </div>
      {error && <p className="ags-error ags-action-error" role="alert">{error}</p>}
      {!form && view?.proposal && <div className="ags-proposal" role="status">
        <span>{view.proposal.count === 0 ? tr("Odpowiedź gotowa (bez propozycji zmian).") : pending > 0 ? tr("Propozycje do przejrzenia: {0} · {1}", pending, label) : tr("Propozycje nie oczekują już na decyzję · {0}", label)}</span>
        {view.proposal.count > 0 && <button type="button" className="btn btn-primary" onClick={review}>{pending > 0 ? tr("Przejrzyj zmiany") : tr("Pokaż wynik")}</button>}
      </div>}
      {!form && <div className="ags-foot">
        {unread && <button type="button" className="btn btn-secondary ags-latest" onClick={() => { const el = bodyRef.current; if (el) el.scrollTop = el.scrollHeight; stick.current = true; setUnread(false); }}><ArrowDown size={12} aria-hidden />{tr("Do najnowszych")}</button>}
        {cards.length > 0 && <div className="ags-pending">
          {cards.length > 1 && <p className="ags-start-note">{tr("1 z")} {cards.length}  {tr("oczekujących próśb")}</p>}
          <ol className="ags-docked" aria-label={tr("Oczekuje na twoją odpowiedź")}><CardView key={cards[0]!.id} c={cards[0]!} onError={setError} autoFocus={true} /></ol>
        </div>}
        <div className="composer">
          <textarea ref={inputRef} data-session-focus className="field" rows={2} value={text} disabled={!live || sending} aria-label={tr("Wiadomość do agenta")} placeholder={live ? tr("Napisz do agenta…") : tr("Sesja się zakończyła.")}
            onChange={(e) => setText(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); void send(); } }} />
          <div className="composer-row">
            <div className="composer-context"><span className="control-chip" title={label}>{label}</span>{view && <span className="control-chip">{providerName(view.provider)}</span>}</div>
            <button type="button" className="btn btn-primary btn-round" disabled={interruptible ? busy : working || !live || sending || !text.trim()} aria-label={interruptible ? tr("Przerwij") : tr("Wyślij")} title={interruptible ? tr("Przerwij pracę agenta") : tr("Wyślij (Enter)")} onClick={interruptible ? run(sessionStore.interrupt) : () => void send()}>{interruptible ? <Square size={14} aria-hidden /> : <ArrowUp size={16} aria-hidden />}</button>
          </div>
        </div>
        <p className="ags-start-note ags-composer-note">{tr("Enter: wyślij · Shift+Enter: nowa linia · Zmiany wracają jako propozycje.")}</p>
      </div>}
    </aside>
  </>;
}

export function SessionIndicator() {
  const { view, open } = useSession();
  const updates = useUpdates();
  const pending = proposalPending(view, updates.byProject, updates.loaded);
  const i = indicator(view, pending);
  return <button type="button" className={`ags-indicator${i?.attention ? " attention" : ""}`} aria-expanded={open} title={tr("Otwórz panel agenta (⌘⇧J / Ctrl+Shift+J)")} onClick={() => sessionStore.open()}>
    <Bot size={14} aria-hidden />{i?.text ?? "Agent"}
  </button>;
}
