import { t as tr, getLocale } from "../i18n";
// Pure logic of the „Akcje" view: parameter values, validation, request building,
// provider choice and time formatting.

import type { ProviderId, ProviderInfo, Recipe, RecipeParam, RunMode, RunRequest } from "../api/types";

export type ParamValues = Record<string, string>;

/** Starting values: the recipe's defaults, `""` (all projects) / "dzis" / "" otherwise. */
export function initialValues(recipe: Recipe): ParamValues {
  const v: ParamValues = {};
  for (const p of recipe.params) {
    v[p.key] = p.default ?? (p.kind === "period" ? "dzis" : "");
  }
  return v;
}

function check(p: RecipeParam, raw: string | undefined, projects: readonly string[] | null): string | null {
  const value = (raw ?? "").trim();
  if (p.kind === "period") {
    if (value !== "dzis" && value !== "tydzien") return tr("Wybierz okres: {0}.", p.label);
    return null;
  }
  if (p.kind === "project") {
    if (value && projects && !projects.includes(value)) return tr("Nie ma takiego projektu: {0}.", value);
    if (!value && p.required) return tr("Uzupełnij pole: {0}.", p.label);
    return null;
  }
  if (!value && p.required) return tr("Uzupełnij pole: {0}.", p.label);
  return null;
}

/** key → message for every invalid param (empty object = valid). */
export function validateParams(recipe: Recipe, values: ParamValues, projects: readonly string[] | null = null): Record<string, string> {
  const errors: Record<string, string> = {};
  for (const p of recipe.params) {
    const msg = check(p, values[p.key], projects);
    if (msg) errors[p.key] = msg;
  }
  return errors;
}

/** Every param of the recipe is sent (trimmed); unknown keys are never sent. */
export function buildRequest(recipe: Recipe, provider: ProviderId, mode: RunMode, values: ParamValues): RunRequest {
  const params: Record<string, string> = {};
  for (const p of recipe.params) params[p.key] = (values[p.key] ?? "").trim();
  return { recipe: recipe.id, provider, mode, params };
}

/** Stored choice if still available, else the first available provider, else null. */
export function pickProvider(providers: readonly ProviderInfo[], stored: string | null): ProviderId | null {
  const ok = providers.filter((p) => p.available);
  const hit = ok.find((p) => p.id === stored);
  return (hit ?? ok[0])?.id ?? null;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** "0:07", "12:03", "1:02:03". Negative / NaN → "0:00". */
export function formatElapsed(ms: number): string {
  const total = Number.isFinite(ms) && ms > 0 ? Math.floor(ms / 1000) : 0;
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

/** Parses the backend's local "YYYY-MM-DDTHH:MM:SS" (no zone) as local time. */
export function parseLocal(s: string): number {
  const m = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::(\d{2}))?/.exec(s);
  if (!m) return NaN;
  return new Date(+m[1]!, +m[2]! - 1, +m[3]!, +m[4]!, +m[5]!, +(m[6] ?? 0)).getTime();
}

/** Elapsed ms of a run: to `finished_at`, or to `now` while running. */
export function runElapsedMs(startedAt: string, finishedAt: string | null, now: number): number {
  const start = parseLocal(startedAt);
  const end = finishedAt ? parseLocal(finishedAt) : now;
  return end - start;
}

/** "14:05" for today, "12 lis 14:05" otherwise (local). */
export function formatRunTime(at: string, now: Date = new Date()): string {
  const ms = parseLocal(at);
  if (Number.isNaN(ms)) return at;
  const d = new Date(ms);
  const hm = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  if (d.toDateString() === now.toDateString()) return hm;
  const month = new Intl.DateTimeFormat(getLocale(), { month: "short" }).format(d).replace(/\.$/, "");
  return `${d.getDate()} ${month} ${hm}`;
}

export const MODE_LABEL: Record<RunMode, string> = { get background() { return tr("W tle"); }, terminal: "Terminal" };
export const STATUS_LABEL = {
  get running() { return tr("W toku"); },
  get succeeded() { return tr("Zakończone"); },
  get failed() { return tr("Błąd"); },
  get cancelled() { return tr("Przerwane"); },
} as const;
