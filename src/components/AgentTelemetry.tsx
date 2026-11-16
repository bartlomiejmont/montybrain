import { t as tr, getLocale } from "../i18n";
import { ChevronRight } from "lucide-react";
import { useState } from "react";
import { readPref, writePref } from "../store/ui";
import type { LimitSnapshot, SessionView } from "../api/types";
import { formatCost, formatTokens, limitWindow, sessionConnection } from "../lib/providerStatus";
import "./agent-telemetry.css";

export function AgentTelemetry({ view, collapsible = false, showLimits = true }: { view: SessionView; collapsible?: boolean; showLimits?: boolean }) {
  const { model, usage, limits } = view.telemetry;
  const [expanded, setExpanded] = useState(() => readPref("montybrain.session.telemetry") === "1");
  const tokens = usage?.input_tokens != null && usage.output_tokens != null ? usage.input_tokens + usage.output_tokens : null;
  return <div className={collapsible ? "agent-telemetry-disclosure" : undefined}>
    {collapsible && <button type="button" className="agent-telemetry-toggle" aria-expanded={expanded} onClick={() => {
      setExpanded(!expanded);
      writePref("montybrain.session.telemetry", expanded ? "0" : "1");
    }}>
      <ChevronRight size={12} className={expanded ? "open" : ""} aria-hidden />
      <span>{model ?? tr("Oczekiwanie na model")} · {formatTokens(tokens)}  {tr("tokenów ·")} {usage?.cost_usd != null ? formatCost(usage.cost_usd) : tr("koszt: brak danych")}{limits.length > 0 && view.telemetry.limits_at && tr(" · limit {0}% · stan z {1}", Math.round(Math.max(...limits.map((l) => l.used_percent))), new Date(view.telemetry.limits_at * 1000).toLocaleString(getLocale(), { dateStyle: "short", timeStyle: "short" }))}</span>
    </button>}
    {(!collapsible || expanded) && <div className="agent-telemetry" aria-label={tr("Połączenie i zużycie sesji")}>
    <div className="agent-telemetry-line"><span>{sessionConnection(view)}</span><span>Model: <strong>{model ?? tr("CLI jeszcze nie podało")}</strong></span></div>
    <dl className="agent-usage">
      <div><dt>{tr("Tokeny wejściowe")}</dt><dd>{formatTokens(usage?.input_tokens ?? null)}</dd></div>
      <div><dt>{tr("Tokeny wyjściowe")}</dt><dd>{formatTokens(usage?.output_tokens ?? null)}</dd></div>
      {usage?.cached_input_tokens != null && <div><dt>{tr("W tym z pamięci")}</dt><dd>{formatTokens(usage.cached_input_tokens)}</dd></div>}
      {usage?.cache_write_tokens != null && <div><dt>{tr("W tym zapis do pamięci")}</dt><dd>{formatTokens(usage.cache_write_tokens)}</dd></div>}
      <div><dt>{tr("Szacunek CLI")}</dt><dd>{usage?.cost_usd != null ? formatCost(usage.cost_usd) : tr("Brak danych")}</dd></div>
    </dl>
    <p className="agent-telemetry-note">{usage ? tr("Ostatni raport CLI · łącznie w tej sesji.") : tr("Zużycie pojawi się po raporcie CLI.")}  {tr("Szacunek kosztu nie jest opłatą za subskrypcję.")}</p>
    {showLimits && <AgentLimits snapshot={view.telemetry.limits_at ? { limits, checked_at: view.telemetry.limits_at, status: view.telemetry.limit_status ?? null } : null} />}
  </div>}
  </div>;
}


export function AgentLimits({ snapshot }: { snapshot?: LimitSnapshot | null }) {
  if (!snapshot) return <p className="agent-telemetry-note">{tr("Limit subskrypcji: brak danych z CLI. Tokeny sesji nie określają limitu konta.")}</p>;
  const at = new Date(snapshot.checked_at * 1000);
  return <div className="agent-limits" aria-label={tr("Limity konta")}>
    <p className="agent-telemetry-note">{tr("Limit konta · stan z")} <time dateTime={at.toISOString()} title={at.toLocaleString(getLocale())}>{at.toLocaleDateString(getLocale()) === new Date().toLocaleDateString(getLocale()) ? at.toLocaleTimeString(getLocale(), { hour: "2-digit", minute: "2-digit" }) : at.toLocaleString(getLocale(), { dateStyle: "short", timeStyle: "short" })}</time>{snapshot.status && ` · ${snapshot.status}`}</p>
    {snapshot.limits.map((l) => <div className="agent-limit" key={l.id}>
      <label><span>{limitWindow(l.window_minutes)} · {l.id.startsWith("claude:") ? `Claude${l.id.includes("seven_day_") ? ` · ${l.id.split("_").at(-1)}` : ""}` : l.id.replace(/:(primary|secondary)$/, "")}</span><span>{new Intl.NumberFormat(getLocale(), { maximumFractionDigits: 1 }).format(l.used_percent)}{tr("% wykorzystane")}</span>
        <progress max={100} value={l.used_percent} aria-label={tr("Wykorzystanie limitu {0}", limitWindow(l.window_minutes))} />
      </label>
      {l.reported_at != null && l.reported_at !== snapshot.checked_at && <p className="agent-telemetry-note">{tr("Raport okna:")} {new Date(l.reported_at * 1000).toLocaleString(getLocale())}</p>}
      {l.resets_at == null && l.reset_label && <p className="agent-telemetry-note">{tr("Odnowienie:")} {l.reset_label}</p>}
      {l.resets_at != null && <p className="agent-telemetry-note">{tr("Odnowienie:")} {new Date(l.resets_at * 1000).toLocaleString(getLocale())}</p>}
    </div>)}
    <p className="agent-telemetry-note">{tr("Ostatni raport CLI może obejmować użycie poza MontyBrain.")}{!snapshot.limits.length && tr(" CLI nie podało procentu wykorzystania.")}</p>
  </div>;
}
