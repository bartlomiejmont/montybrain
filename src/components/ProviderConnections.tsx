import { t as tr } from "../i18n";
import { useEffect, useState } from "react";
import type { ProviderInfo } from "../api/types";
import { describeError } from "../lib/errors";
import { providerStatus } from "../lib/providerStatus";
import { useSession } from "../store/session";
import { updateStore, useProviders } from "../store/updates";
import { AgentLimits, AgentTelemetry } from "./AgentTelemetry";

function State({ provider }: { provider?: ProviderInfo }) {
  const state = providerStatus(provider);
  return <span className={`provider-state ${state.tone}`}>{state.label}</span>;
}

export function ProviderSummary() {
  const providers = useProviders();
  const [error, setError] = useState(false);
  useEffect(() => {
    const load = () => void updateStore.loadProviders().then(() => setError(false), () => setError(true));
    load();
    const timer = window.setInterval(load, 60_000);
    window.addEventListener("focus", load);
    return () => { window.clearInterval(timer); window.removeEventListener("focus", load); };
  }, []);
  return <a className="provider-summary" href="#/ustawienia" aria-label={tr("Połączenia agentów — szczegóły w ustawieniach")}>
    {(["claude", "codex"] as const).map((id) => {
      const p = providers?.find((p) => p.id === id);
      return <span key={id}>{id === "claude" ? "Claude" : "Codex"} · {error ? <span>{tr("Nie udało się odświeżyć")}</span> : <State provider={p} />}</span>;
    })}
  </a>;
}

export function ProviderConnections() {
  const providers = useProviders();
  const { view } = useSession();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function refresh() {
    setBusy(true);
    setError("");
    try { await updateStore.loadProviders(true); }
    catch (e) { setError(describeError(e)); }
    finally { setBusy(false); }
  }
  useEffect(() => { void refresh(); }, []);
  return <section className="group" aria-labelledby="set-g-connections" aria-busy={busy}>
    <div className="set-connections-head">
      <h2 className="group-head" id="set-g-connections">{tr("Połączenia agentów")}</h2>
      <button type="button" className="btn btn-quiet" disabled={busy} onClick={() => void refresh()}>{busy ? tr("Sprawdzam…") : tr("Odśwież status")}</button>
    </div>
    <p className="set-hint">{tr("Logowanie sprawdzamy w lokalnym CLI. Połączenie z CLI i użyty model potwierdza uruchomiona sesja.")}</p>
    {error && <p className="set-note danger" role="alert">{tr("Nie udało się odświeżyć. Wyświetlone dane mogą być nieaktualne.")} {error}</p>}
    {(["claude", "codex"] as const).map((id) => {
      const p = providers?.find((p) => p.id === id);
      const sessionLimits = view?.provider === id && view.telemetry.limits_at ? { limits: view.telemetry.limits, checked_at: view.telemetry.limits_at, status: view.telemetry.limit_status ?? null } : null;
      const limits = sessionLimits && sessionLimits.checked_at >= (p?.limits?.checked_at ?? 0) ? sessionLimits : p?.limits;
      return <div className="set-connection" key={id}>
        <div className="set-row"><span className="set-label">{id === "claude" ? "Claude Code" : "Codex"}</span><State provider={p} /></div>
        {p && <>
          <dl className="set-connection-facts">
            <div><dt>{tr("Logowanie")}</dt><dd>{[p.auth.method, p.auth.plan].filter(Boolean).join(" · ") || tr("Brak danych")}</dd></div>
            <div><dt>{tr("Wersja CLI")}</dt><dd>{p.version ?? tr("Brak danych")}</dd></div>
            <div><dt>{tr("Program")}</dt><dd className="set-path">{p.path ?? tr("Nie znaleziono")}</dd></div>
          </dl>
          {(p.problem || p.auth.problem) && <p className="set-note danger">{describeError(p.problem || p.auth.problem)}</p>}
          {p.available && p.auth.status === "logged_out" && <p className="set-hint">{tr("Zaloguj się w terminalu:")} <code>{id === "claude" ? "claude auth login" : "codex login"}</code>{tr(", potem odśwież status.")}</p>}
        </>}
        <AgentLimits snapshot={limits} />
        {p?.limits_problem && <p className="set-hint set-foot">{p.limits_problem}{limits && tr(" Wyświetlamy ostatni zapisany raport.")}</p>}
        {!limits && id === "claude" && !p?.limits_problem && <p className="set-hint set-foot">{tr("Claude Code nie podało limitu w /usage ani w sesji. Odśwież status po zalogowaniu w CLI.")}</p>}
        {view?.provider === id ? <>
          <p className="set-hint set-foot">{["ended", "failed", "stopped"].includes(view.status) ? tr("Ostatnia sesja w MontyBrain") : tr("Bieżąca sesja w MontyBrain")}</p>
          <AgentTelemetry view={view} showLimits={false} />
        </> : <p className="set-hint set-foot">{tr("Model i zużycie sesji: brak raportu.")}</p>}
      </div>;
    })}
  </section>;
}
