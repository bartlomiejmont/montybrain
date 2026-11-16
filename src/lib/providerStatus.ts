import { t as tr, getLocale } from "../i18n";
import type { ProviderInfo, SessionView } from "../api/types";

export function providerStatus(p: ProviderInfo | undefined): { label: string; tone: string } {
  if (!p) return { label: tr("Sprawdzam…"), tone: "unknown" };
  if (!p.available) return { label: tr("Brak instalacji"), tone: "error" };
  if (p.problem) return { label: tr("Problem z CLI"), tone: "error" };
  if (p.auth.status === "logged_out") return { label: tr("Wylogowany"), tone: "error" };
  if (p.auth.status === "logged_in") return { label: tr("Zalogowany"), tone: "ok" };
  return { label: tr("Logowanie niepotwierdzone"), tone: "unknown" };
}

export function sessionConnection(view: SessionView): string {
  if (["ended", "failed", "stopped"].includes(view.status)) return tr("Rozłączono");
  return view.telemetry.connected ? tr("Połączono z CLI") : tr("Łączenie…");
}

const tokens = () => new Intl.NumberFormat(getLocale());
export const formatTokens = (n: number | null) => n === null ? tr("Brak danych") : tokens().format(n);
export const formatCost = (n: number) => new Intl.NumberFormat(getLocale(), { style: "currency", currency: "USD", minimumFractionDigits: 2, maximumFractionDigits: 4 }).format(n);

export function limitWindow(minutes: number | null): string {
  if (minutes === null) return "Limit";
  if (minutes % 1440 === 0) return tr("{0} dni", minutes / 1440);
  if (minutes % 60 === 0) return tr("{0} godz.", minutes / 60);
  return tr("{0} min", minutes);
}
