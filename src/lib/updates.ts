import { t as tr, getLanguage, getLocale, message } from "../i18n";
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
  const f = getLanguage() === "en" ? (n === 1 ? "one" : "many") : pluralForm(n);
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
  const what = run.request ? tr("{0} wykonuje zlecenie „{1}”", who, shorten(run.request)) : tr("{0} sprawdza projekt", who);
  return `${what}${meta ? ` · ${meta}` : ""}`;
}

/** What the sources line says about the history (review head, idle hint). */
export function historyLine(note: string | null | undefined, where: "review" | "idle"): string | null {
  if (!note) return null;
  if (note === HISTORY_OFF_NOTE) return tr(note);
  return where === "review" ? tr("Źródła: repozytoria i historia rozmów. {0}", message(note)) : tr("Historia rozmów: {0}", message(note));
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
    const all = tr("{0} zmian", plural(c.total, tr("propozycja"), tr("propozycje"), tr("propozycji")));
    if (c.topics === 0) return all;
    if (c.topics === c.total) return plural(c.topics, tr("nowy temat"), tr("nowe tematy"), tr("nowych tematów"));
    return tr("{0}, w tym {1}", all, plural(c.topics, tr("nowy temat"), tr("nowe tematy"), tr("nowych tematów")));
  }
  const decided = c.total - c.pending;
  return tr("{0} z {1} rozstrzygnięte · {2}", decided, c.total, plural(c.pending, tr("niezastosowana"), tr("niezastosowane"), tr("niezastosowanych")));
}

/** Status line during review: what is left to decide. */
export function reviewStatus(c: Counts): string {
  if (c.pending === c.total) return tr("Zaznacz, co zastosować.");
  const left = { one: tr("Została"), few: tr("Zostały"), many: tr("Zostało") }[pluralForm(c.pending)];
  const head = c.applied > 0 ? tr("Zastosowano {0}. ", c.applied) : "";
  return `${head}${left} ${plural(c.pending, tr("propozycja"), tr("propozycje"), tr("propozycji"))}.`;
}

/** The secondary button never says more than it does. `null`: nothing left to decide. */
export function secondaryLabel(c: Counts): string | null {
  if (c.pending === 0) return null;
  if (c.pending === c.total) return tr("Odrzuć wszystkie");
  return tr("Odrzuć pozostałe ({0})", c.pending);
}

/** One line once everything is decided: "Zastosowano 3 zmiany · pominięto 1". */
export function receiptText(c: Counts): string {
  const parts: string[] = [];
  const others = c.applied - c.topicsApplied;
  if (c.topicsApplied) parts.push(tr("Dodano {0}", plural(c.topicsApplied, tr("temat"), tr("tematy"), tr("tematów"))));
  if (others) parts.push(`${c.topicsApplied ? tr("zastosowano") : tr("Zastosowano")} ${plural(others, tr("zmianę"), tr("zmiany"), tr("zmian"))}`);
  if (c.stale) parts.push(tr("pominięto {0}", c.stale));
  if (c.failed) parts.push(tr("{0} z błędem", c.failed));
  if (c.rejected) parts.push(c.applied || c.stale || c.failed ? tr("odrzucono {0}", c.rejected) : tr("Odrzucono {0}", plural(c.rejected, tr("propozycję"), tr("propozycje"), tr("propozycji"))));
  return parts.join(" · ") || tr("Brak zmian");
}

/** "Czekają 4 propozycje. Automatyczne sprawdzenie wstrzymane." */
export function heldText(pending: number): string {
  const verb = pluralForm(pending) === "few" ? tr("Czekają") : tr("Czeka");
  return tr("{0} {1}. Automatyczne sprawdzenie wstrzymane.", verb, plural(pending, tr("propozycja"), tr("propozycje"), tr("propozycji")));
}

/** The question before a manual re-check (or a request) replaces undecided proposals. */
export function recheckQuestion(pending: number, request = false, agent: ProviderId | null = null): string {
  const who = agent ? ` (${PROVIDER_LABEL[agent]})` : "";
  return tr("{0}{1} zastąpi {2}. Uruchomić?", request ? tr("Zlecenie") : tr("Nowe sprawdzenie"), who, plural(pending, tr("nieprzejrzaną propozycję"), tr("nieprzejrzane propozycje"), tr("nieprzejrzanych propozycji")));
}



/** "dziś 14:02", "wczoraj 09:05", "12 lis 14:02", "jutro 09:00". */
export function formatWhen(at: IsoMinute, now: Date): string {
  const [date = at, time = ""] = at.split("T");
  const diff = daysBetween(toIsoDate(now), date);
  const t = time.slice(0, 5);
  if (diff === 0) return tr("dziś {0}", t);
  if (diff === -1) return tr("wczoraj {0}", t);
  if (diff === 1) return tr("jutro {0}", t);
  const [, m, d] = date.split("-").map(Number);
  const month = new Intl.DateTimeFormat(getLocale(), { month: "short" }).format(new Date(2000, (m ?? 1) - 1, 1)).replace(/\.$/, "");
  return `${d} ${month} ${t}`;
}

/** The next automatic check: "wkrótce" when due, else like [`formatWhen`]. */
export function formatNext(next: IsoMinute, now: Date): string {
  const [d = "", t = "00:00"] = next.split("T");
  const [y, m, day] = d.split("-").map(Number);
  const [h, min] = t.split(":").map(Number);
  const at = new Date(y ?? 1970, (m ?? 1) - 1, day ?? 1, h ?? 0, min ?? 0);
  return at.getTime() - now.getTime() <= 60_000 ? tr("wkrótce") : formatWhen(next, now);
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
        ? tr("Porówna repozytoria z tematami i zadaniami i zaproponuje zmiany.")
        : tr("Znajdzie w repozytoriach tematy, nad którymi trwa praca, i zaproponuje je do dodania."),
      error: false,
      detail: null,
    };
  }
  const when = formatWhen(last.at, now);
  const who = last.provider ? ` · ${PROVIDER_LABEL[last.provider]}` : "";
  const how = last.trigger === "auto" ? tr("automatycznie ") : "";
  const o = last.outcome;
  const limited = o.kind === "failed" && (o.model || (o.limit && last.provider));
  if (last.request && !limited) return requestLine(last, when, who);
  const checked = tr("Sprawdzono {0}{1}{2}", how, when, who);
  switch (o.kind) {
    case "proposed":
      return { text: `${checked} · ${plural(o.count, tr("propozycja"), tr("propozycje"), tr("propozycji"))}`, error: false, detail: null };
    case "no_changes":
      return { text: tr("{0} · bez zmian", checked), error: false, detail: null };
    case "applied":
      return { text: tr("{0} · zastosowano {1}", checked, plural(o.count, tr("zmianę"), tr("zmiany"), tr("zmian"))), error: false, detail: null };
    case "unchanged":
      return { text: tr("{0}{1} · bez zmian od ostatniego sprawdzenia", when.charAt(0).toUpperCase(), when.slice(1)), error: false, detail: null };
    case "no_proposal":
      return { text: tr("{0} · agent nie podał propozycji w wymaganym formacie", checked), error: true, detail: o.message };
    case "failed": {
      if (o.model) {
        // The runner put the plain sentence first; the CLI's text follows.
        const [first = "", ...rest] = o.message.split("\n");
        return { text: message(first), error: true, detail: rest.join("\n").trim() || o.message };
      }
      if (o.limit && last.provider) {
        const other = otherAvailable(providers, last.provider);
        const way = other ? tr("Spróbuj później albo wybierz {0}.", PROVIDER_LABEL[other]) : tr("Spróbuj później.");
        return { text: tr("Limit {0} wykorzystany. {1}", PROVIDER_LABEL[last.provider], way), error: true, detail: o.message };
      }
      return { text: tr("Sprawdzanie {0}{1} nie powiodło się{2}", how, when, who), error: true, detail: o.message };
    }
    case "cancelled":
      return { text: tr("Przerwano {0}{1}", when, who), error: false, detail: null };
  }
}

/** The idle status line after a request ("Zlecenie · dziś 14:02 · Codex · 3 propozycje"). */
function requestLine(last: UpdateResult, when: string, who: string): StatusLine {
  const head = tr("Zlecenie · {0}{1}", when, who);
  const o = last.outcome;
  switch (o.kind) {
    case "proposed":
      return { text: `${head} · ${plural(o.count, tr("propozycja"), tr("propozycje"), tr("propozycji"))}`, error: false, detail: null };
    case "no_changes":
    case "unchanged":
      return { text: tr("{0} · odpowiedź bez zmian w MontyBrain", head), error: false, detail: null };
    case "applied":
      return { text: tr("{0} · zastosowano {1}", head, plural(o.count, tr("zmianę"), tr("zmiany"), tr("zmian"))), error: false, detail: null };
    case "no_proposal":
      return { text: tr("{0} · agent nie podał propozycji w wymaganym formacie", head), error: true, detail: o.message };
    case "failed":
      return { text: tr("Zlecenie {0} nie powiodło się{1}", when, who), error: true, detail: o.message };
    case "cancelled":
      return { text: tr("Przerwano zlecenie {0}{1}", when, who), error: false, detail: null };
  }
}

/** "wyłączona", "co 6 h", "codziennie 09:00" (the one format everywhere). */
export function autoLabel(cfg: AutoUpdate): string {
  const s = cfg.schedule;
  if (s.kind === "off") return tr("wyłączona");
  if (s.kind === "every") return tr("co {0} h", s.hours);
  return tr("codziennie {0}:00", String(s.hour).padStart(2, "0"));
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
        kind: tr("Nowy temat"),
        title: c.title,
        what: [lower(TOPIC_STATUS_LABEL[c.status]), c.repo ? tr("repozytorium {0}", c.repo) : null].filter(Boolean).join(" · "),
        detail: c.text,
        summary: c.summary,
        topic: true,
        detailLabel: tr("Pierwszy wpis w dzienniku"),
        tasks: c.tasks.map((t) => ({ title: t.title, status: lower(TASK_STATUS_LABEL[t.status]) })),
      };
    case "remove_task":
      return { kind: tr("Usunięcie zadania"), title: c.title, what: tr("Usunie zadanie wraz z notatkami i powiązaniami"), detail: c.reason, summary: null, topic: false, detailLabel: null, tasks: [] };
    case "task_status":
      return {
        kind: tr("Status zadania"),
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
        kind: c.status ? tr("Dziennik i status tematu") : tr("Wpis w dzienniku"),
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
        kind: tr("Nowe zadanie"),
        title: c.title,
        what: c.topic_title ? tr("w temacie {0}", c.topic_title) : tr("poza tematami"),
        detail: c.reason,
        summary: null,
        topic: false,
        detailLabel: null,
        tasks: [],
      };
    case "move_task":
      return {
        kind: tr("Przeniesienie wpisu"),
        title: c.title,
        what: tr("do: {0}", c.to_label),
        detail: c.reason,
        summary: null,
        topic: false,
        detailLabel: null,
        tasks: [],
      };
    case "topic_from_tasks":
      return {
        kind: tr("Temat z wpisów"),
        title: c.title,
        what: tr("w projekcie {0} · {1} (z identyfikatorami i notatkami)", c.project_name, plural(c.tasks.length, tr("wpis"), tr("wpisy"), tr("wpisów"))),
        detail: c.reason,
        summary: c.summary || null,
        topic: true,
        detailLabel: null,
        tasks: c.tasks.map((t) => ({ title: t.title, status: `^${t.id}` })),
      };
  }
}

/** Indices of the topic-level changes (first, primary) and of the task changes. */
export function groupChanges(p: StoredProposal): { topics: number[]; tasks: number[] } {
  const topics: number[] = [];
  const tasks: number[] = [];
  p.changes.forEach((c, i) => (c.change.kind === "new_topic" || c.change.kind === "topic_log" || c.change.kind === "topic_from_tasks" ? topics : tasks).push(i));
  return { topics, tasks };
}

/** Undecided non-destructive changes start checked; removals need a manual tick. */
export function defaultSelection(p: StoredProposal): number[] {
  return p.changes.flatMap((c, i) => (c.outcome || c.change.kind === "remove_task" ? [] : [i]));
}

/** All actionable rows, including removals that start unchecked. */
export function undecidedChanges(p: StoredProposal): number[] {
  return p.changes.flatMap((c, i) => c.outcome ? [] : [i]);
}

/** The toast after applying (the undo action is a separate button). */
export function applySummary(items: readonly ApplyItem[]): string {
  const applied = items.filter((i) => i.status === "applied").length;
  const stale = items.filter((i) => i.status === "stale").length;
  const failed = items.filter((i) => i.status === "failed" || i.status === "missing").length;
  const parts: string[] = [];
  parts.push(applied ? tr("Zastosowano {0}", plural(applied, tr("zmianę"), tr("zmiany"), tr("zmian"))) : tr("Nic nie zastosowano"));
  if (stale) parts.push(tr("pominięto {0} (zmienione w międzyczasie)", stale));
  if (failed) parts.push(tr("{0} z błędem", failed));
  return `${parts.join("; ")}.`;
}

/** The toast after a rejection was taken back. */
export function restoreSummary(items: readonly UndoResult[]): string {
  const back = items.filter((i) => i.status === "undone").length;
  if (back === 0) return tr("Nic nie przywrócono: propozycje zmieniły się w międzyczasie.");
  return tr("Przywrócono do przeglądu {0}.", plural(back, tr("propozycję"), tr("propozycje"), tr("propozycji")));
}

/** The toast after "Odrzuć wszystkie / pozostałe" (with a "Cofnij" action). */
export function rejectSummary(count: number): string {
  return tr("Odrzucono {0}.", plural(count, tr("propozycję"), tr("propozycje"), tr("propozycji")));
}

/** The toast after an undo: "Cofnięto 3 zmiany." or "2 cofnięte, 1 zmienione w międzyczasie." */
export function undoSummary(items: readonly UndoResult[]): string {
  const undone = items.filter((i) => i.status === "undone").length;
  const partial = items.filter((i) => i.status === "partial").length;
  const changed = items.filter((i) => i.status === "changed").length;
  const failed = items.filter((i) => i.status === "failed").length;
  if (undone === items.length && undone > 0) return tr("Cofnięto {0}.", plural(undone, tr("zmianę"), tr("zmiany"), tr("zmian")));
  const parts: string[] = [];
  if (undone) parts.push(`${undone} ${undone === 1 ? tr("cofnięta") : tr("cofnięte")}`);
  if (partial) parts.push(tr("{0} cofnięte częściowo", partial));
  if (changed) parts.push(tr("{0} zmienione w międzyczasie", changed));
  if (failed) parts.push(tr("{0} z błędem", failed));
  return `${parts.join(", ") || tr("Nic nie cofnięto")}.`;
}

export const OUTCOME_LABEL = { get applied() { return tr("Zastosowano"); }, get stale() { return tr("Pominięto"); }, get failed() { return tr("Błąd"); }, get rejected() { return tr("Odrzucono"); } } as const;
export const OUTCOME_MARK = { applied: "✓", stale: "–", failed: "✕", rejected: "×" } as const;

/** Hours offered for "every N hours" (a stored other value is offered too). */
export function hourChoices(current: number | null): number[] {
  const base = [1, 2, 3, 4, 6, 8, 12, 24];
  return current && !base.includes(current) ? [...base, current].sort((a, b) => a - b) : base;
}

/** The answer of a proposing recipe without its machine-readable `montybrain-zmiany` block. */
export function stripProposalBlock(text: string): string {
  const lines = text.split("\n");
  const out: string[] = [];
  for (let i = 0; i < lines.length; i++) {
    const m = /^ {0,3}(`{3,}|~{3,})\s*montybrain-zmiany\s*$/.exec(lines[i]!);
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
  return project.repos.length === 0 ? tr("Projekt „{0}” nie ma repozytoriów, więc nie ma z czym porównać zadań. Dodaj repozytorium w edycji projektu.", project.name) : null;
}

/** "opus · wysiłek high" for a run's meta, or `null` when both are the CLI's default. */
export function agentMeta(choice: { model: string | null; effort: string | null }): string | null {
  const parts = [choice.model, choice.effort ? tr("wysiłek {0}", choice.effort) : null].filter(Boolean);
  return parts.length ? parts.join(" · ") : null;
}

/** The bar button's short label: "Auto: co 6 h · historia". */
export function panelLabel(cfg: AutoUpdate, history: boolean): string {
  const sources = history ? tr("repozytoria i historia") : tr("repozytoria");
  if (cfg.schedule.kind === "off") return tr("Źródła: {0} · auto wył.", sources);
  return tr("Źródła: {0} · auto {1}{2}", sources, autoLabel(cfg), cfg.on_result === "apply" ? tr(" · bez pytania") : "");
}

/** What the agent of a request can see, one line under the request field. */
export function requestNote(history: boolean): string {
  return history ? tr("Agent widzi MontyBrain, repozytoria i historię rozmów; bez poczty i Slacka.") : tr("Agent widzi MontyBrain i repozytoria; bez historii rozmów, poczty i Slacka.");
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
  const what = o ? OUTCOME_LABEL[o.status] : tr("Do decyzji");
  return `${t.kind} · ${t.title} · ${what}`;
}

/** The receipt with who and when: "Dziś 12:10 · Codex · dodano 1 temat · zastosowano 3 zmiany · odrzucono 1". */
export function receiptLine(p: StoredProposal, now: Date): string {
  const when = formatWhen(p.created_at, now);
  const head = `${p.topic ? tr("Odświeżenie · ") : p.request ? tr("Zlecenie · ") : ""}${when} · ${PROVIDER_LABEL[p.provider]}`;
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
  if (e.kind === "text") return tr("Agent analizuje i pisze odpowiedź…");
  if (e.kind !== "tool") return e.text;
  const name = (e.text.split(/[\s{]/, 1)[0] ?? "").replace(/^mcp__montybrain__/, "").replace(/^montybrain\//, "");
  const topic = argOf(e.text, "topic");
  const named = topic ? `„${topicTitle(topic) ?? topic}”` : null;
  switch (name) {
    case "overview":
      return tr("Czytam przegląd projektu");
    case "get_topic":
      return named ? tr("Czytam temat {0}", named) : tr("Czytam temat");
    case "list_tasks":
      return named ? tr("Czytam zadania tematu {0}", named) : tr("Czytam listę zadań");
    case "search": {
      const q = argOf(e.text, "query");
      return q ? tr("Szukam „{0}” w MontyBrain", shorten(q, 40)) : tr("Szukam w MontyBrain");
    }
    case "recent_journal":
      return tr("Czytam dziennik");
    case "get_task":
      return tr("Czytam zadanie");
    case "log_update":
    case "update_task":
    case "create_task":
    case "create_topic":
      return tr("Agent próbował zapisać w MontyBrain (w tym trybie zablokowane)");
    default:
      return name.startsWith("Bash") || name.startsWith("$") ? tr("Agent próbował uruchomić polecenie (w tym trybie zablokowane)") : tr("Używam narzędzia {0}", name || tr("agenta"));
  }
}

/** The consent sentence for turning the history source on. */
export const HISTORY_CONSENT =
  "Treść twoich poleceń z sesji Claude Code i Codexa w repozytoriach tego projektu trafi do agenta, który robi sprawdzenie, także do drugiego dostawcy. MontyBrain wycina fragmenty wyglądające na hasła i klucze, ale nie daje gwarancji.";

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
