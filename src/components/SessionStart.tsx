import { t as tr } from "../i18n";
import { useEffect, useLayoutEffect, useState } from "react";

import { ArrowUp, X } from "lucide-react";
import { useAutosizeTextarea } from "./ui";

import type { ProviderId, SessionView } from "../api/types";
import { pickProvider } from "../lib/actions";
import { MY_TOOLS_CONSENT, MY_TOOLS_CONSENT_KEY } from "../lib/agentMode";
import { describeError } from "../lib/errors";
import { isLive, providerName, scopeLabel, type SessionDraft } from "../lib/session";
import { links, navigate } from "../router/router";
import { sessionStore, startSessionDraft } from "../store/session";
import { readPref, writePref } from "../store/ui";
import { updateStore, useProviders, useUpdates } from "../store/updates";
import { useView } from "../store/vault";

const PROVIDER_PREF = "montybrain.aktualizacja.provider";

export function SessionStart({ draft, current, busy, onError }: { draft: SessionDraft; current: SessionView | null; busy: boolean; onError: (text: string) => void }) {
  const { snapshot } = useView();
  const input = useAutosizeTextarea(draft.text);
  useLayoutEffect(() => { input.current?.focus(); }, [draft.project, draft.topic?.slug]);
  const providers = useProviders();
  const updates = useUpdates();
  const [agreed, setAgreed] = useState(() => readPref(MY_TOOLS_CONSENT_KEY) === "1");
  const [checking, setChecking] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const projects = snapshot.projects;
  const project = projects.find((p) => p.slug === draft.project);
  const chosen = providers ? pickProvider(providers, draft.provider ?? readPref(PROVIDER_PREF)) : null;
  const target = updates.byProject[draft.scope === "inbox" ? "" : draft.project ?? ""];
  const pending = target?.pending ?? 0;
  const live = !!current && isLive(current.status);
  const validScope = draft.scope === "inbox" || !!project;
  const ready = !busy && !checking && !loadError && !!chosen && validScope && !!draft.text.trim() && agreed && pending === 0;

  async function load() {
    setChecking(true);
    setLoadError(null);
    try { await Promise.all([updateStore.loadProviders(true), updateStore.start(), sessionStore.start()]); }
    catch (e) { setLoadError(describeError(e)); }
    finally { setChecking(false); }
  }
  useEffect(() => { void load(); }, []);

  async function submit() {
    if (!ready || !chosen) return;
    onError("");
    try {
      writePref(PROVIDER_PREF, chosen);
      writePref(MY_TOOLS_CONSENT_KEY, "1");
      await startSessionDraft(draft, chosen, live);
    } catch (e) { onError(describeError(e)); }
  }
  const review = () => {
    navigate(draft.scope === "inbox" ? links.inbox() : links.project(draft.project!));
    sessionStore.hide();
  };

  return (
    <form className="ags-start" aria-busy={busy} onSubmit={(e) => { e.preventDefault(); void submit(); }}>
      {current && <div className="ags-conflict" role="status">
        <p>{live ? tr("Inna sesja już działa") : tr("Masz poprzednią rozmowę")} · {scopeLabel(current.scope, projects)}.</p>
        <p>{live ? tr("Uruchomienie nowej zakończy obecną sesję. Nowa prośba pozostanie tutaj, jeśli wrócisz do rozmowy.") : tr("Nowa sesja zastąpi tę rozmowę. Zapisane propozycje pozostają w swoim miejscu.")}</p>
        <button type="button" className="btn btn-secondary" disabled={busy} onClick={() => sessionStore.conversation()}>{tr("Pokaż")} {live ? tr("aktywną sesję") : tr("poprzednią rozmowę")}</button>
      </div>}
      {(loadError || (!checking && !chosen)) && <div className="ags-start-note" role="alert">
        <p>{loadError ?? tr("Nie znaleziono Claude Code ani Codexa. Sprawdź instalację i logowanie.")}</p>
        <button type="button" className="btn btn-secondary" disabled={checking} onClick={() => void load()}>{tr("Sprawdź ponownie")}</button>
      </div>}
      {busy && <p className="ags-start-note" role="status">{tr("Uruchamiam sesję…")}</p>}
      {!validScope && <p role="alert">{tr("Projekt nie jest już dostępny. Wybierz zakres.")}</p>}
      <div className="composer">
        <label className="sr-only" htmlFor="session-request">{tr("Co mam sprawdzić?")}</label>
        <textarea id="session-request" ref={input} data-session-focus className="field ags-start-input" rows={5} maxLength={18000} value={draft.text} disabled={busy}
          placeholder={draft.topic ? tr("Co sprawdzić w tym temacie?") : tr("Co mam sprawdzić? Np. wiadomości od księgowej i zadania, które z nich wynikają…")}
          onChange={(e) => sessionStore.editDraft({ text: e.target.value })}
          onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); void submit(); } }} />
        <div className="composer-row">
          <div className="composer-context">
            <label className="sr-only" htmlFor="session-scope">{tr("Zakres propozycji")}</label>
            <select id="session-scope" aria-label={tr("Zakres propozycji")} className="control-chip" value={draft.scope === "inbox" ? "" : draft.project ?? ""} disabled={busy} onChange={(e) => sessionStore.editDraft({ scope: e.target.value ? "project" : "inbox", project: e.target.value || undefined, topic: undefined, selection: undefined })}>
              <option value="">{tr("Skrzynka")}</option>
              {projects.map((p) => <option key={p.slug} value={p.slug}>{p.name}{p.state === "archived" ? tr(" · archiwum") : ""}</option>)}
            </select>
            <label className="sr-only" htmlFor="session-provider">Agent</label>
            <select id="session-provider" aria-label="Agent" className="control-chip" value={chosen ?? ""} disabled={busy || checking || !chosen} onChange={(e) => sessionStore.editDraft({ provider: e.target.value as ProviderId })}>
              {!chosen && <option value="">{checking ? tr("Sprawdzam dostępność…") : tr("Brak dostępnego agenta")}</option>}
              {(providers ?? []).map((p) => <option key={p.id} value={p.id} disabled={!p.available}>{providerName(p.id)}{!p.available ? tr(" · niedostępny") : ""}</option>)}
            </select>
            {draft.topic && <button type="button" className="btn btn-secondary ags-topic-chip" title={tr("Usuń kontekst tematu: {0}", draft.topic.title)} aria-label={tr("Usuń kontekst tematu: {0}", draft.topic.title)} disabled={busy} onClick={() => sessionStore.editDraft({ topic: undefined })}><span>{draft.topic.title}</span><X size={12} aria-hidden /></button>}
          </div>
          <button type="submit" className="btn btn-primary btn-round" aria-label={live ? tr("Zakończ i uruchom nową") : tr("Uruchom sesję")} title={live ? tr("Zakończ i uruchom nową") : tr("Uruchom sesję (Enter)")} disabled={!ready}><ArrowUp size={16} aria-hidden /></button>
        </div>
      </div>
      {draft.scope === "inbox" && <p className="ags-start-note">{tr("Kontekst:")} {draft.selection?.length ? tr("wybrane wpisy ({0})", draft.selection.length) : tr("otwarte wpisy Skrzynki")}.</p>}
      <p className="ags-start-note">{tr("Enter: uruchom · Shift+Enter: nowa linia. Sesja używa twoich skonfigurowanych narzędzi. Zmiany w MontyBrain wrócą jako propozycje.")}</p>
      {!agreed && <label className="ags-consent"><input type="checkbox" checked={agreed} onChange={(e) => setAgreed(e.target.checked)} /> <span>{MY_TOOLS_CONSENT}</span></label>}
      {pending > 0 && <div className="ags-conflict" role="status"><p>{tr("W tym miejscu czekają propozycje (")}{pending}{tr("). Przejrzyj je przed nową sesją, żeby kolejny wynik ich nie zastąpił.")}</p><button type="button" className="btn btn-secondary" onClick={review}>{tr("Przejrzyj propozycje")}</button></div>}
    </form>
  );
}
