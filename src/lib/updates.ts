// Pure logic of the project update strip („Aktualizacja"): which state to show,
// the status lines, change descriptions, plurals and the toasts.

import type {
  ApplyItem,
  AutoUpdate,
  IsoMinute,
  ProjectUpdateView,
  ProposedChange,
  ProviderId,
  ProviderInfo,
  Recipe,
  RunInfo,
  StoredChange,
  StoredProposal,
  UndoResult,
  UpdateResult,
} from "../api/types";
import { daysBetween, toIsoDate } from "./dates";
import { TASK_STATUS_LABEL, TOPIC_STATUS_LABEL } from "./labels";

export const UPDATE_RECIPE = "aktualizacja";
/** A free-form request ("Zlecenie"): the same review flow, never scheduled. */
export const REQUEST_RECIPE = "zlecenie";
/** Said by the backend when a request ran with the history source off (runs/runner.rs). */
export const HISTORY_OFF_NOTE = "Historia rozmów jest wyłączona dla tego projektu (Źródła).";

export const PROVIDER_LABEL: Record<ProviderId, string> = { claude: "Claude Code", codex: "Codex" };

/** Which of the three Polish plural forms `n` takes: 1 → one, 2–4 (not 12–14) → few, else many. */
export function pluralForm(n: number): "one" | "few" | "many" {
  if (n === 1) return "one";
  return n % 10 >= 2 && n % 10 <= 4 && !(n % 100 >= 12 && n % 100 <= 14) ? "few" : "many";
}

/** "1 zmiana", "3 zmiany", "5 zmian", "22 zmiany". */
export function plural(n: number, one: string, few: string, many: string): string {
  const f = pluralForm(n);
  return `${n} ${f === "one" ? one : f === "few" ? few : many}`;
}

/** The running background `aktualizacja` or `zlecenie` run of `project`, if any. */
export function updateRunFor(runs: readonly RunInfo[], project: string): RunInfo | null {
  return runs.find((r) => (r.recipe === UPDATE_RECIPE || r.recipe === REQUEST_RECIPE) && r.mode === "background" && r.status === "running" && r.project === project) ?? null;
}

/** One line, at most `max` characters: "rozpisz temat płatności na podstawie…". */
export function shorten(text: string, max = 60): string {
  const line = text.split(/\s+/).filter(Boolean).join(" ");
  if (line.length <= max) return line;
  const cut = line.slice(0, max);
  const space = cut.lastIndexOf(" ");
  return `${(space > max * 0.6 ? cut.slice(0, space) : cut).replace(/[\s,.;:–-]+$/, "")}…`;
}

/** The status of a running check or request. */
export function runningStatus(run: Pick<RunInfo, "provider" | "request" | "model" | "effort">): string {
  const meta = agentMeta(run);
  const who = PROVIDER_LABEL[run.provider];
  const what = run.request ? `${who} wykonuje zlecenie „${shorten(run.request)}”` : `${who} sprawdza projekt`;
  return `${what}${meta ? ` · ${meta}` : ""}`;
}

/** What the sources line says about the history (review head, idle hint). */
export function historyLine(note: string | null | undefined, where: "review" | "idle"): string | null {
  if (!note) return null;
  if (note === HISTORY_OFF_NOTE) return note;
  return where === "review" ? `Źródła: repozytoria i historia rozmów. ${note}` : `Historia rozmów: ${note}`;
}

/** The strip is shown only where there is something to compare: an active project with repositories. */
export function showStrip(project: { state: string; repos: readonly unknown[] }): boolean {
  return project.state !== "archived" && project.repos.length > 0;
}

export type Strip =
  | { kind: "running"; run: RunInfo }
  /** A proposal with undecided changes. */
  | { kind: "review"; proposal: StoredProposal }
  /** Every change of the last proposal is decided: one receipt line. */
  | { kind: "receipt"; proposal: StoredProposal }
  | { kind: "idle" };

export function stripState(view: ProjectUpdateView | null, run: RunInfo | null): Strip {
  if (run) return { kind: "running", run };
  const p = view?.proposal;
  if (p && !p.problem && p.changes.length > 0) {
    return p.changes.some((c) => !c.outcome) ? { kind: "review", proposal: p } : { kind: "receipt", proposal: p };
  }
  return { kind: "idle" };
}

export interface Counts {
  total: number;
  /** Proposed new topics, and how many of them were applied. */
  topics: number;
  topicsApplied: number;
  pending: number;
  applied: number;
  stale: number;
  failed: number;
  rejected: number;
}

export function counts(p: StoredProposal): Counts {
  const c: Counts = { total: p.changes.length, topics: 0, topicsApplied: 0, pending: 0, applied: 0, stale: 0, failed: 0, rejected: 0 };
  for (const sc of p.changes) {
    if (!sc.outcome) c.pending++;
    else c[sc.outcome.status]++;
    if (sc.change.kind === "new_topic") {
      c.topics++;
      if (sc.outcome?.status === "applied") c.topicsApplied++;
    }
  }
  return c;
}

/** Heading of the review: "4 propozycje zmian" or, after a partial apply, "3 z 4 zastosowane · 1 niezastosowana". */
export function reviewHeading(c: Counts): string {
  if (c.pending === c.total) {
    const all = `${plural(c.total, "propozycja", "propozycje", "propozycji")} zmian`;
    if (c.topics === 0) return all;
    if (c.topics === c.total) return plural(c.topics, "nowy temat", "nowe tematy", "nowych tematów");
    return `${all}, w tym ${plural(c.topics, "nowy temat", "nowe tematy", "nowych tematów")}`;
  }
  const decided = c.total - c.pending;
  return `${decided} z ${c.total} rozstrzygnięte · ${plural(c.pending, "niezastosowana", "niezastosowane", "niezastosowanych")}`;
}

/** Status line during review: what is left to decide. */
export function reviewStatus(c: Counts): string {
  if (c.pending === c.total) return "Zaznacz, co zastosować.";
  const left = { one: "Została", few: "Zostały", many: "Zostało" }[pluralForm(c.pending)];
  const head = c.applied > 0 ? `Zastosowano ${c.applied}. ` : "";
  return `${head}${left} ${plural(c.pending, "propozycja", "propozycje", "propozycji")}.`;
}

/** The secondary button never says more than it does. `null`: nothing left to decide. */
export function secondaryLabel(c: Counts): string | null {
  if (c.pending === 0) return null;
  if (c.pending === c.total) return "Odrzuć wszystkie";
  return `Odrzuć pozostałe (${c.pending})`;
}

/** One line once everything is decided: "Zastosowano 3 zmiany · pominięto 1". */
export function receiptText(c: Counts): string {
  const parts: string[] = [];
  const others = c.applied - c.topicsApplied;
  if (c.topicsApplied) parts.push(`Dodano ${plural(c.topicsApplied, "temat", "tematy", "tematów")}`);
  if (others) parts.push(`${c.topicsApplied ? "zastosowano" : "Zastosowano"} ${plural(others, "zmianę", "zmiany", "zmian")}`);
  if (c.stale) parts.push(`pominięto ${c.stale}`);
  if (c.failed) parts.push(`${c.failed} z błędem`);
  if (c.rejected) parts.push(c.applied || c.stale || c.failed ? `odrzucono ${c.rejected}` : `Odrzucono ${plural(c.rejected, "propozycję", "propozycje", "propozycji")}`);
  return parts.join(" · ") || "Brak zmian";
}

/** "Czekają 4 propozycje. Automatyczne sprawdzenie wstrzymane." */
export function heldText(pending: number): string {
  const verb = pluralForm(pending) === "few" ? "Czekają" : "Czeka";
  return `${verb} ${plural(pending, "propozycja", "propozycje", "propozycji")}. Automatyczne sprawdzenie wstrzymane.`;
}

/** The question before a manual re-check (or a request) replaces undecided proposals. */
export function recheckQuestion(pending: number, request = false, agent: ProviderId | null = null): string {
  const who = agent ? ` (${PROVIDER_LABEL[agent]})` : "";
  return `${request ? "Zlecenie" : "Nowe sprawdzenie"}${who} zastąpi ${plural(pending, "nieprzejrzaną propozycję", "nieprzejrzane propozycje", "nieprzejrzanych propozycji")}. Uruchomić?`;
}

const MONTHS = ["sty", "lut", "mar", "kwi", "maj", "cze", "lip", "sie", "wrz", "paź", "lis", "gru"];

/** "dziś 14:02", "wczoraj 09:05", "12 lis 14:02", "jutro 09:00". */
export function formatWhen(at: IsoMinute, now: Date): string {
  const [date = at, time = ""] = at.split("T");
  const diff = daysBetween(toIsoDate(now), date);
  const t = time.slice(0, 5);
  if (diff === 0) return `dziś ${t}`;
  if (diff === -1) return `wczoraj ${t}`;
  if (diff === 1) return `jutro ${t}`;
  const [, m, d] = date.split("-").map(Number);
  return `${d} ${MONTHS[(m ?? 1) - 1]} ${t}`;
}

/** The next automatic check: "wkrótce" when due, else like [`formatWhen`]. */
export function formatNext(next: IsoMinute, now: Date): string {
  const [d = "", t = "00:00"] = next.split("T");
  const [y, m, day] = d.split("-").map(Number);
  const [h, min] = t.split(":").map(Number);
  const at = new Date(y ?? 1970, (m ?? 1) - 1, day ?? 1, h ?? 0, min ?? 0);
  return at.getTime() - now.getTime() <= 60_000 ? "wkrótce" : formatWhen(next, now);
}

export interface StatusLine {
  text: string;
  /** Error states: said in words, not by colour only. */
  error: boolean;
  /** Raw technical text (CLI message, exit code), shown collapsed. */
  detail: string | null;
}

/** A provider other than `p` that is available, if any. */
export function otherAvailable(providers: readonly ProviderInfo[] | null, p: ProviderId | null): ProviderId | null {
  return providers?.find((x) => x.id !== p && x.available)?.id ?? null;
}

/** The idle status line from the last result (`hasTopics`: the project has topics already). */
export function statusLine(last: UpdateResult | null, now: Date, providers: readonly ProviderInfo[] | null = null, hasTopics = true): StatusLine {
  if (!last) {
    return {
      text: hasTopics
        ? "Porówna repozytoria z tematami i zadaniami i zaproponuje zmiany."
        : "Znajdzie w repozytoriach tematy, nad którymi trwa praca, i zaproponuje je do dodania.",
      error: false,
      detail: null,
    };
  }
  const when = formatWhen(last.at, now);
  const who = last.provider ? ` · ${PROVIDER_LABEL[last.provider]}` : "";
  const how = last.trigger === "auto" ? "automatycznie " : "";
  const o = last.outcome;
  const limited = o.kind === "failed" && (o.model || (o.limit && last.provider));
  if (last.request && !limited) return requestLine(last, when, who);
  const checked = `Sprawdzono ${how}${when}${who}`;
  switch (o.kind) {
    case "proposed":
      return { text: `${checked} · ${plural(o.count, "propozycja", "propozycje", "propozycji")}`, error: false, detail: null };
    case "no_changes":
      return { text: `${checked} · bez zmian`, error: false, detail: null };
    case "applied":
      return { text: `${checked} · zastosowano ${plural(o.count, "zmianę", "zmiany", "zmian")}`, error: false, detail: null };
    case "unchanged":
      return { text: `${when.charAt(0).toUpperCase()}${when.slice(1)} · bez zmian od ostatniego sprawdzenia`, error: false, detail: null };
    case "no_proposal":
      return { text: `${checked} · agent nie podał propozycji w wymaganym formacie`, error: true, detail: o.message };
    case "failed": {
      if (o.model) {
        // The runner put the plain sentence first; the CLI's text follows.
        const [first = "", ...rest] = o.message.split("\n");
        return { text: first, error: true, detail: rest.join("\n").trim() || o.message };
      }
      if (o.limit && last.provider) {
        const other = otherAvailable(providers, last.provider);
        const way = other ? `Spróbuj później albo wybierz ${PROVIDER_LABEL[other]}.` : "Spróbuj później.";
        return { text: `Limit ${PROVIDER_LABEL[last.provider]} wykorzystany. ${way}`, error: true, detail: o.message };
      }
      return { text: `Sprawdzanie ${how}${when} nie powiodło się${who}`, error: true, detail: o.message };
    }
    case "cancelled":
      return { text: `Przerwano ${when}${who}`, error: false, detail: null };
  }
}

/** The idle status line after a request ("Zlecenie · dziś 14:02 · Codex · 3 propozycje"). */
function requestLine(last: UpdateResult, when: string, who: string): StatusLine {
  const head = `Zlecenie · ${when}${who}`;
  const o = last.outcome;
  switch (o.kind) {
    case "proposed":
      return { text: `${head} · ${plural(o.count, "propozycja", "propozycje", "propozycji")}`, error: false, detail: null };
    case "no_changes":
    case "unchanged":
      return { text: `${head} · odpowiedź bez zmian w Morwisku`, error: false, detail: null };
    case "applied":
      return { text: `${head} · zastosowano ${plural(o.count, "zmianę", "zmiany", "zmian")}`, error: false, detail: null };
    case "no_proposal":
      return { text: `${head} · agent nie podał propozycji w wymaganym formacie`, error: true, detail: o.message };
    case "failed":
      return { text: `Zlecenie ${when} nie powiodło się${who}`, error: true, detail: o.message };
    case "cancelled":
      return { text: `Przerwano zlecenie ${when}${who}`, error: false, detail: null };
  }
}

/** "wyłączona", "co 6 h", "codziennie 09:00" (the one format everywhere). */
export function autoLabel(cfg: AutoUpdate): string {
  const s = cfg.schedule;
  if (s.kind === "off") return "wyłączona";
  if (s.kind === "every") return `co ${s.hours} h`;
  return `codziennie ${String(s.hour).padStart(2, "0")}:00`;
}

export interface ChangeText {
  /** What kind of change, in words. */
  kind: string;
  title: string;
  /** What will change ("w toku → zrobione"). */
  what: string | null;
  /** The agent's evidence / the journal text. */
  detail: string;
  /** The full proposed topic summary, if any (shown, never approved blind). */
  summary: string | null;
  /** Topic-level change (new topic, topic journal): shown first, as the primary items. */
  topic: boolean;
  /** Label of `detail` when it is not the evidence sentence (e.g. the first journal entry). */
  detailLabel: string | null;
  /** Initial tasks of a new topic. */
  tasks: { title: string; status: string }[];
}

const lower = (s: string) => s.charAt(0).toLowerCase() + s.slice(1);

export function describeChange(c: ProposedChange): ChangeText {
  switch (c.kind) {
    case "new_topic":
      return {
        kind: "Nowy temat",
        title: c.title,
        what: [lower(TOPIC_STATUS_LABEL[c.status]), c.repo ? `repozytorium ${c.repo}` : null].filter(Boolean).join(" · "),
        detail: c.text,
        summary: c.summary,
        topic: true,
        detailLabel: "Pierwszy wpis w dzienniku",
        tasks: c.tasks.map((t) => ({ title: t.title, status: lower(TASK_STATUS_LABEL[t.status]) })),
      };
    case "task_status":
      return {
        kind: "Status zadania",
        title: c.title,
        what: `${lower(TASK_STATUS_LABEL[c.from])} → ${lower(TASK_STATUS_LABEL[c.to])}`,
        detail: c.reason,
        summary: null,
        topic: false,
        detailLabel: null,
        tasks: [],
      };
    case "topic_log":
      return {
        kind: c.status ? "Dziennik i status tematu" : "Wpis w dzienniku",
        title: c.topic_title,
        what: c.status ? `${lower(TOPIC_STATUS_LABEL[c.base_status])} → ${lower(TOPIC_STATUS_LABEL[c.status])}` : null,
        detail: c.text,
        summary: c.summary,
        topic: true,
        detailLabel: null,
        tasks: [],
      };
    case "new_task":
      return {
        kind: "Nowe zadanie",
        title: c.title,
        what: c.topic_title ? `w temacie ${c.topic_title}` : "poza tematami",
        detail: c.reason,
        summary: null,
        topic: false,
        detailLabel: null,
        tasks: [],
      };
  }
}

/** Indices of the topic-level changes (first, primary) and of the task changes. */
export function groupChanges(p: StoredProposal): { topics: number[]; tasks: number[] } {
  const topics: number[] = [];
  const tasks: number[] = [];
  p.changes.forEach((c, i) => (c.change.kind === "new_topic" || c.change.kind === "topic_log" ? topics : tasks).push(i));
  return { topics, tasks };
}

/** Changes checked by default: those not decided yet. */
export function defaultSelection(p: StoredProposal): number[] {
  return p.changes.flatMap((c, i) => (c.outcome ? [] : [i]));
}

/** The toast after applying (the undo action is a separate button). */
export function applySummary(items: readonly ApplyItem[]): string {
  const applied = items.filter((i) => i.status === "applied").length;
  const stale = items.filter((i) => i.status === "stale").length;
  const failed = items.filter((i) => i.status === "failed" || i.status === "missing").length;
  const parts: string[] = [];
  parts.push(applied ? `Zastosowano ${plural(applied, "zmianę", "zmiany", "zmian")}` : "Nic nie zastosowano");
  if (stale) parts.push(`pominięto ${stale} (zmienione w międzyczasie)`);
  if (failed) parts.push(`${failed} z błędem`);
  return `${parts.join("; ")}.`;
}

/** The toast after a rejection was taken back. */
export function restoreSummary(items: readonly UndoResult[]): string {
  const back = items.filter((i) => i.status === "undone").length;
  if (back === 0) return "Nic nie przywrócono: propozycje zmieniły się w międzyczasie.";
  return `Przywrócono do przeglądu ${plural(back, "propozycję", "propozycje", "propozycji")}.`;
}

/** The toast after "Odrzuć wszystkie / pozostałe" (with a "Cofnij" action). */
export function rejectSummary(count: number): string {
  return `Odrzucono ${plural(count, "propozycję", "propozycje", "propozycji")}.`;
}

/** The toast after an undo: "Cofnięto 3 zmiany." or "2 cofnięte, 1 zmienione w międzyczasie." */
export function undoSummary(items: readonly UndoResult[]): string {
  const undone = items.filter((i) => i.status === "undone").length;
  const partial = items.filter((i) => i.status === "partial").length;
  const changed = items.filter((i) => i.status === "changed").length;
  const failed = items.filter((i) => i.status === "failed").length;
  if (undone === items.length && undone > 0) return `Cofnięto ${plural(undone, "zmianę", "zmiany", "zmian")}.`;
  const parts: string[] = [];
  if (undone) parts.push(`${undone} ${undone === 1 ? "cofnięta" : "cofnięte"}`);
  if (partial) parts.push(`${partial} cofnięte częściowo`);
  if (changed) parts.push(`${changed} zmienione w międzyczasie`);
  if (failed) parts.push(`${failed} z błędem`);
  return `${parts.join(", ") || "Nic nie cofnięto"}.`;
}

export const OUTCOME_LABEL = { applied: "Zastosowano", stale: "Pominięto", failed: "Błąd", rejected: "Odrzucono" } as const;
export const OUTCOME_MARK = { applied: "✓", stale: "–", failed: "✕", rejected: "×" } as const;

/** Hours offered for "every N hours" (a stored other value is offered too). */
export function hourChoices(current: number | null): number[] {
  const base = [1, 2, 3, 4, 6, 8, 12, 24];
  return current && !base.includes(current) ? [...base, current].sort((a, b) => a - b) : base;
}

/** The answer of a proposing recipe without its machine-readable `morwisko-zmiany` block. */
export function stripProposalBlock(text: string): string {
  const lines = text.split("\n");
  const out: string[] = [];
  for (let i = 0; i < lines.length; i++) {
    const m = /^ {0,3}(`{3,}|~{3,})\s*morwisko-zmiany\s*$/.exec(lines[i]!);
    if (!m) {
      out.push(lines[i]!);
      continue;
    }
    const fence = m[1]!;
    let j = i + 1;
    while (j < lines.length && !new RegExp(`^ {0,3}${fence[0] === "`" ? "`" : "~"}{${fence.length},}\\s*$`).test(lines[j]!)) j++;
    i = j;
  }
  return out.join("\n").trim();
}

/** Validation of the project of a recipe that compares repositories. */
export function projectProblem(recipe: Pick<Recipe, "needs_repos">, project: { name: string; repos: readonly unknown[] } | undefined): string | null {
  if (!recipe.needs_repos || !project) return null;
  return project.repos.length === 0 ? `Projekt „${project.name}” nie ma repozytoriów, więc nie ma z czym porównać zadań. Dodaj repozytorium w edycji projektu.` : null;
}

/** "opus · wysiłek high" for a run's meta, or `null` when both are the CLI's default. */
export function agentMeta(choice: { model: string | null; effort: string | null }): string | null {
  const parts = [choice.model, choice.effort ? `wysiłek ${choice.effort}` : null].filter(Boolean);
  return parts.length ? parts.join(" · ") : null;
}

/** The bar button's short label: "Auto: co 6 h · historia". */
export function panelLabel(cfg: AutoUpdate, history: boolean): string {
  const sources = history ? "repozytoria i historia" : "repozytoria";
  if (cfg.schedule.kind === "off") return `Źródła: ${sources} · auto wył.`;
  return `Źródła: ${sources} · auto ${autoLabel(cfg)}${cfg.on_result === "apply" ? " · bez pytania" : ""}`;
}

/** What the agent of a request can see, one line under the request field. */
export function requestNote(history: boolean): string {
  return history ? "Agent widzi Morwisko, repozytoria i historię rozmów; bez poczty i Slacka." : "Agent widzi Morwisko i repozytoria; bez historii rozmów, poczty i Slacka.";
}

/**
 * The review's selection after its outcomes changed: the user's ticks stay on
 * rows still undecided; only rows that became undecided again (an undo) are
 * ticked anew.
 */
export function nextSelection(prevUndecided: readonly number[], undecided: readonly number[], selected: ReadonlySet<number>): Set<number> {
  const before = new Set(prevUndecided);
  return new Set(undecided.filter((i) => selected.has(i) || !before.has(i)));
}

/** One line for a decided row of a partial review: "Nowy temat · Program lojalnościowy · Zastosowano". */
export function decidedLine(sc: StoredChange): string {
  const t = describeChange(sc.change);
  const o = sc.outcome;
  const what = o ? OUTCOME_LABEL[o.status] : "Do decyzji";
  return `${t.kind} · ${t.title} · ${what}`;
}

/** The receipt with who and when: "Dziś 12:10 · Codex · dodano 1 temat · zastosowano 3 zmiany · odrzucono 1". */
export function receiptLine(p: StoredProposal, now: Date): string {
  const when = formatWhen(p.created_at, now);
  const head = `${p.request ? "Zlecenie · " : ""}${when} · ${PROVIDER_LABEL[p.provider]}`;
  const body = receiptText(counts(p));
  const line = `${head} · ${body.charAt(0).toLowerCase()}${body.slice(1)}`;
  return line.charAt(0).toUpperCase() + line.slice(1);
}

/** The value of `"key":"…"` in a (possibly cut) tool-call line. */
function argOf(line: string, key: string): string | null {
  const m = new RegExp(`"${key}"\\s*:\\s*"((?:[^"\\\\]|\\\\.)*)`).exec(line);
  return m ? m[1]!.replace(/\\"/g, '"') : null;
}

/**
 * The strip's progress line: a Polish sentence per step instead of the raw
 * tool call (that stays in the Akcje log). `topicTitle` names a topic slug.
 */
export function progressText(e: { kind: string; text: string }, topicTitle: (slug: string) => string | null = () => null): string {
  if (e.kind === "text") return "Agent analizuje i pisze odpowiedź…";
  if (e.kind !== "tool") return e.text;
  const name = (e.text.split(/[\s{]/, 1)[0] ?? "").replace(/^mcp__morwisko__/, "").replace(/^morwisko\//, "");
  const topic = argOf(e.text, "topic");
  const named = topic ? `„${topicTitle(topic) ?? topic}”` : null;
  switch (name) {
    case "overview":
      return "Czytam przegląd projektu";
    case "get_topic":
      return named ? `Czytam temat ${named}` : "Czytam temat";
    case "list_tasks":
      return named ? `Czytam zadania tematu ${named}` : "Czytam listę zadań";
    case "search": {
      const q = argOf(e.text, "query");
      return q ? `Szukam „${shorten(q, 40)}” w Morwisku` : "Szukam w Morwisku";
    }
    case "recent_journal":
      return "Czytam dziennik";
    case "get_task":
      return "Czytam zadanie";
    case "log_update":
    case "update_task":
    case "create_task":
    case "create_topic":
      return "Agent próbował zapisać w Morwisku (w tym trybie zablokowane)";
    default:
      return name.startsWith("Bash") || name.startsWith("$") ? "Agent próbował uruchomić polecenie (w tym trybie zablokowane)" : `Używam narzędzia ${name || "agenta"}`;
  }
}

/** The consent sentence for turning the history source on. */
export const HISTORY_CONSENT =
  "Treść twoich poleceń z sesji Claude Code i Codexa w repozytoriach tego projektu trafi do agenta, który robi sprawdzenie, także do drugiego dostawcy. Morwisko wycina fragmenty wyglądające na hasła i klucze, ale nie daje gwarancji.";

/**
 * The history toggle: turning it on only asks for consent (nothing is saved
 * until `confirmed`); turning it off saves at once.
 */
export function historyToggle(
  src: { history: boolean; history_days: number },
  on: boolean,
  confirmed = false,
): { ask: boolean; save: { history: boolean; history_days: number } | null } {
  if (!on) return { ask: false, save: src.history ? { ...src, history: false } : null };
  if (src.history) return { ask: false, save: null };
  return confirmed ? { ask: false, save: { ...src, history: true } } : { ask: true, save: null };
}
