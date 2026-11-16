import { t as tr } from "../i18n";
import { RefreshCw } from "lucide-react";
import { Button } from "./ui";
import { useEffect, useState } from "react";

import { api, type JournalEntry, type ProviderId } from "../api";
import { pickProvider } from "../lib/actions";
import { describeError } from "../lib/errors";
import { journalAgentBlock, newJournalEntries, journalRefreshLabel } from "../lib/journalAgent";
import { PROVIDER_LABEL, REQUEST_RECIPE, receiptLine, statusLine, undoSummary } from "../lib/updates";
import { links } from "../router/router";
import { runStore, useRuns } from "../store/runs";
import { readPref, writePref } from "../store/ui";
import { updateStore, useProviders, useUpdates } from "../store/updates";
import { useVault } from "../store/vault";
import { DroppedBox, PROVIDER_PREF, ResultDetails, Review } from "../views/project/UpdateStrip";

/** One shared project run/proposal, rendered at the journal's point of use. */
export function useJournalAgent(project: string, topic: string, entries: JournalEntry[]) {
  const updates = useUpdates();
  const providers = useProviders();
  const runs = useRuns();
  const { refresh } = useVault();
  const [provider, setProvider] = useState<ProviderId | null>(() => (readPref(PROVIDER_PREF) as ProviderId | null) ?? null);
  const [error, setError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const view = updates.byProject[project];
  const fresh = newJournalEntries(entries, view?.journal_refresh?.[topic]);
  const chosen = providers ? pickProvider(providers, provider) : null;
  const block = journalAgentBlock(updates.loaded, providers, view?.pending ?? 0, !!runs.active || starting);
  const latest = runs.list.find((r) => r.recipe === REQUEST_RECIPE && r.project === project && r.topic === topic);
  const proposal = view?.proposal?.topic === topic && (!latest || view.proposal.run_id === latest.id) ? view.proposal : null;
  const running = latest?.status === "running" ? latest : null;

  useEffect(() => {
    void Promise.all([updateStore.start(), updateStore.loadProviders(), runStore.start()]).catch((e) => setError(describeError(e)));
  }, []);

  const start = async () => {
    // Another view may have received a proposal since this render.
    const current = updateStore.getSnapshot();
    const reason = journalAgentBlock(current.loaded, updateStore.getProviders(), current.byProject[project]?.pending ?? 0, !!runStore.getSnapshot().active);
    if (reason || !chosen) { setError(reason ?? tr("Brak agenta.")); return; }
    if (!fresh.length) return;
    setStarting(true);
    setError(null);
    try {
      await runStore.startRun({ recipe: REQUEST_RECIPE, provider: chosen, mode: "background", params: { project, topic } });
    } catch (e) {
      setError(tr("Nie udało się uruchomić odświeżenia: {0}", describeError(e)));
    } finally { setStarting(false); }
  };
  const cancel = async () => {
    if (!running) return;
    try { await runStore.cancel(running.id); } catch (e) { setError(describeError(e)); }
  };
  const undo = async () => {
    if (!proposal || !view?.undo) return;
    try {
      const r = await api.undoUpdate(proposal.run_id, view.undo);
      updateStore.set(r.updates);
      await refresh();
      setError(undoSummary(r.items));
    } catch (e) { setError(describeError(e)); }
  };
  const failed = latest?.status === "failed" ? statusLine({ at: latest.finished_at ?? latest.started_at, provider: latest.provider, trigger: "manual", request: latest.request, outcome: { kind: "failed", message: latest.error ?? tr("Nie udało się odświeżyć tematu."), model: /nie przyjął modelu/.test(latest.error ?? ""), limit: /limit|429|quota/i.test(latest.error ?? "") } }, new Date(), providers) : null;
  const controls = <div className="j-refresh-controls">
    <span className="j-refresh-count" role="status">{journalRefreshLabel(fresh.length)}</span>
    <Button variant="ghost" disabled={!!block || !fresh.length} title={block ?? (fresh.length ? tr("Przeanalizuj nowe wpisy i zaproponuj zmiany tematu") : tr("Brak nowych wpisów"))} onClick={() => void start()}><RefreshCw size={13} aria-hidden />{tr("Odśwież agentem")}</Button>
    {providers && providers.length > 0 && <select className="field select j-agent-provider" aria-label={tr("Agent odświeżający temat")} value={chosen ?? ""} onChange={(e) => {
      const p = e.target.value as ProviderId;
      setProvider(p);
      writePref(PROVIDER_PREF, p);
    }}>
      {!chosen && <option value="">{tr("Brak agenta")}</option>}
      {providers.map((p) => <option key={p.id} value={p.id} disabled={!p.available}>{PROVIDER_LABEL[p.id]}{p.available ? "" : tr(" · niedostępny")}</option>)}
    </select>}
  </div>;
  const result = <div className="j-agent" aria-label={tr("Aktualizacja tematu z dziennika")}>
    {block && !running && <p className="j-agent-note">{block} {!!view?.pending && <a className="upd-link" href={links.project(project)}>{tr("Przejrzyj propozycje projektu →")}</a>}</p>}
    {(starting || running) && <p className="j-agent-note" role="status">{tr("Agent analizuje nowe wpisy…")} {running && <button type="button" className="upd-link" onClick={() => void cancel()}>{tr("Przerwij")}</button>}</p>}
    {error && <p className="j-agent-note" role="status">{error}</p>}
    {failed && <div className="j-agent-note error" role="alert"><p>{failed.text}</p>{failed.detail && <p>{failed.detail}</p>}</div>}
    {latest?.status === "cancelled" && <p className="j-agent-note" role="status">{tr("Odświeżanie przerwane. Nowe wpisy czekają na analizę.")}</p>}
    {proposal && !running && <>
      {proposal.problem ? <p className="j-agent-note error" role="alert">{describeError(proposal.problem)}</p> : proposal.changes.length === 0 ? <p className="j-agent-note" role="status">{tr("Agent nie zaproponował zmian.")}</p> : <>
        {view?.undo && <p className="j-agent-note" role="status">{receiptLine(proposal, new Date())} <button type="button" className="upd-link" onClick={() => void undo()}>{view.undo_rejection ? tr("Cofnij odrzucenie") : tr("Cofnij")}</button></p>}
        {proposal.changes.some((c) => !c.outcome)
          ? <Review key={proposal.run_id} project={project} proposal={proposal} autoFocus={false} onApplied={() => { setError(null); void refresh(); }} onDecided={() => { setError(null); void refresh(); }} onUndone={() => void refresh()} />
          : <details className="j-agent-receipt"><summary>{tr("Pokaż rozstrzygnięte zmiany")}</summary><ResultDetails project={project} proposal={proposal} /></details>}
      </>}
      {proposal.changes.length === 0 && <DroppedBox proposal={proposal} />}
    </>}
  </div>;
  return { controls, result };
}
