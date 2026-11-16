import { t as tr } from "../i18n";
// "Dziś" as an overview of topics: which topics need attention, which are in
// progress, which wait, and the few tasks outside topics that are urgent.
// Pure: the snapshot, the waiting schedule, the drift signals and the
// pending proposals in, sections out. A topic appears once, in the highest
// section that applies, with all its reasons.

import type { ProjectDrift, ProjectUpdateView, Snapshot, Task, Topic, TopicStatus, WaitView } from "../api/types";
import { daysBetween, formatAge, shortDate } from "./dates";
import { journalOf } from "./group";
import { plural } from "./text";

/** An in-progress topic with no journal entry for this many days needs attention. */
export const STALE_DAYS = 7;
/** New commits on the default branches since the last check that are worth a word. */
export const COMMITS_HINT = 10;

export interface Reason {
  /** Machine kind, for tests and styling. */
  kind: "blocked" | "check_due" | "stale" | "overdue" | "due_today" | "spread" | "moved" | "task_check" | "proposals" | "unknown_branches" | "commits" | "active" | "waiting";
  text: string;
}

export interface TopicRow {
  kind: "topic";
  key: string;
  project: string;
  projectName: string;
  topic: Topic;
  /** The summary's first sentence. */
  sentence: string;
  last: { at: string; author: string } | null;
  reasons: Reason[];
  openTasks: number;
  wait: WaitView | null;
}

export interface ProjectRow {
  kind: "project";
  key: string;
  project: string;
  projectName: string;
  reasons: Reason[];
  /** Proposals waiting for review (opens the project's review). */
  pending: number;
  /** Has drift signals (opens the drift list). */
  drift: boolean;
}

export interface TaskRow {
  kind: "task";
  key: string;
  task: Task;
  reasons: Reason[];
  wait: WaitView | null;
}

export type Row = TopicRow | ProjectRow | TaskRow;

export interface TodaySections {
  attention: Row[];
  active: TopicRow[];
  waiting: (TopicRow | TaskRow)[];
  /** Project-level and inbox tasks that are overdue, due today or in progress. */
  loose: Task[];
}

export interface TodayInput {
  snapshot: Snapshot;
  today: string;
  waiting: WaitView[];
  drift: ProjectDrift[];
  updates: ProjectUpdateView[];
}

const WAITING: TopicStatus[] = ["waiting", "blocked"];

/** The first sentence of a summary (at most ~140 characters). */
export function firstSentence(text: string): string {
  const flat = text.replace(/\s+/g, " ").trim();
  const m = /^(.+?[.!?])(\s|$)/.exec(flat);
  const s = m ? m[1]! : flat;
  return s.length > 140 ? `${s.slice(0, 139).trimEnd()}…` : s;
}

const taskCount = (n: number) => `${n} ${plural(n, tr("zadanie"), tr("zadania"), tr("zadań"))}`;

function dueDate(t: Task): string | null {
  return t.due ? t.due.slice(0, 10) : null;
}

/** "Czeka na: weryfikacja konta · sprawdzimy 16 lis" / "Zablokowany · sprawdzimy jutro". */
export function waitText(w: WaitView, today: string): string {
  const head = w.reason ? `${w.status === "blocked" ? tr("Blokuje") : tr("Czeka na")}: ${w.reason}` : w.status === "blocked" ? tr("Zablokowany") : tr("Czeka");
  const diff = daysBetween(today, w.next);
  const when = diff <= 0 ? tr("do sprawdzenia") : diff === 1 ? tr("sprawdzimy jutro") : tr("sprawdzimy {0}", shortDate(w.next, today));
  return `${head} · ${when}`;
}

/** "Czeka od 3 dni — czy coś się ruszyło?" */
function dueText(w: WaitView, today: string): string {
  const days = Math.max(0, daysBetween(w.since.slice(0, 10), today));
  const since = days <= 0 ? tr("od dziś") : days === 1 ? tr("od wczoraj") : tr("od {0} dni", days);
  return tr("{0} {1}{2} — czy coś się ruszyło?", w.status === "blocked" ? tr("Zablokowany") : tr("Czeka"), since, w.reason ? ` (${w.reason})` : "");
}

export function buildToday({ snapshot: snap, today, waiting, drift, updates }: TodayInput): TodaySections {
  const active = new Set(snap.projects.filter((p) => p.state === "active").map((p) => p.slug));
  const projectName = (slug: string) => snap.projects.find((p) => p.slug === slug)?.name ?? slug;
  const waitOf = new Map(waiting.map((w) => [w.key, w]));
  const driftOf = new Map(drift.map((d) => [d.project, d]));

  const attention: Row[] = [];
  const activeRows: TopicRow[] = [];
  const waitingRows: (TopicRow | TaskRow)[] = [];

  // Project-level: proposals waiting, branches outside topics, new commits.
  for (const p of snap.projects.filter((x) => active.has(x.slug))) {
    const reasons: Reason[] = [];
    const pending = updates.find((u) => u.project === p.slug)?.pending ?? 0;
    if (pending > 0) reasons.push({ kind: "proposals", text: `${pending} ${plural(pending, tr("propozycja czeka"), tr("propozycje czekają"), tr("propozycji czeka"))}` });
    const d = driftOf.get(p.slug);
    if (d && d.unknown.length > 0) reasons.push({ kind: "unknown_branches", text: tr("{0} {1} bez tematu", d.unknown.length, plural(d.unknown.length, tr("gałąź"), tr("gałęzie"), tr("gałęzi"))) });
    if (reasons.length && d && (d.new_commits ?? 0) >= COMMITS_HINT) reasons.push({ kind: "commits", text: tr("{0} nowych commitów od sprawdzenia", d.new_commits) });
    if (reasons.length) attention.push({ kind: "project", key: `project:${p.slug}`, project: p.slug, projectName: p.name, reasons, pending, drift: !!d && d.unknown.length > 0 });
  }

  // Tasks per topic (open ones), for reasons and counts.
  const openByTopic = new Map<string, Task[]>();
  for (const t of snap.tasks) {
    if (t.status === "done" || t.location.kind !== "topic") continue;
    const k = `${t.location.project}/${t.location.topic}`;
    openByTopic.set(k, [...(openByTopic.get(k) ?? []), t]);
  }

  const rank = (r: Row) => {
    const kinds = new Set(r.reasons.map((x) => x.kind));
    if (r.kind === "project") return 0;
    if (kinds.has("overdue")) return 1;
    if (kinds.has("check_due") || kinds.has("blocked") || kinds.has("task_check")) return 2;
    if (kinds.has("due_today")) return 3;
    return 4;
  };

  for (const topic of snap.topics) {
    if (!active.has(topic.project) || topic.status === "done") continue;
    const key = `${topic.project}/${topic.slug}`;
    const tasks = openByTopic.get(key) ?? [];
    const journal = journalOf(snap, topic.project, topic.slug);
    const last = journal[0] ? { at: journal[0].at, author: journal[0].author } : null;
    const wait = waitOf.get(`topic:${topic.project}/${topic.slug}`) ?? null;
    const reasons: Reason[] = [];

    if (WAITING.includes(topic.status) && wait?.due) reasons.push({ kind: "check_due", text: dueText(wait, today) });
    if (topic.status === "in_progress") {
      const since = last ? daysBetween(last.at.slice(0, 10), today) : null;
      if (since === null || since >= STALE_DAYS) reasons.push({ kind: "stale", text: since === null ? tr("Bez wpisu w dzienniku") : tr("Bez wpisu od {0} dni", since) });
    }
    const overdue = tasks.filter((t) => (dueDate(t) ?? "9999") < today).length;
    const dueToday = tasks.filter((t) => dueDate(t) === today).length;
    if (overdue) reasons.push({ kind: "overdue", text: tr("{0} po terminie", taskCount(overdue)) });
    if (dueToday) reasons.push({ kind: "due_today", text: tr("{0} na dziś", taskCount(dueToday)) });
    for (const t of tasks) {
      const w = t.id ? waitOf.get(`task:${t.id}`) : undefined;
      if (w?.due) reasons.push({ kind: "task_check", text: tr("Zadanie „{0}” zablokowane — czy coś się ruszyło?", t.title) });
    }
    const d = driftOf.get(topic.project);
    const spread = d?.spread.find((s) => s.topic === topic.slug);
    if (spread) {
      const repos = spread.repos > 1 ? tr(" w {0} repozytoriach", spread.repos) : "";
      reasons.push({ kind: "spread", text: `${spread.branches} ${plural(spread.branches, tr("gałąź"), tr("gałęzie"), tr("gałęzi"))}${repos}` });
    }
    const moved = d?.moved.find((m) => m.topic === topic.slug);
    if (moved) reasons.push({ kind: "moved", text: tr("Repozytorium ruszyło ({0}, {1}), dziennik nie", moved.branch, formatAge(moved.last_commit, today)) });

    const row: TopicRow = {
      kind: "topic",
      key: `topic:${key}`,
      project: topic.project,
      projectName: projectName(topic.project),
      topic,
      sentence: firstSentence(topic.summary),
      last,
      reasons,
      openTasks: tasks.length,
      wait,
    };
    if (reasons.length) {
      attention.push(row);
    } else if (topic.status === "in_progress") {
      row.reasons = [{ kind: "active", text: last ? tr("Ostatni wpis {0}", formatAge(last.at, today)) : tr("W toku") }];
      activeRows.push(row);
    } else if (WAITING.includes(topic.status)) {
      row.reasons = [{ kind: "waiting", text: wait ? waitText(wait, today) : topic.status === "blocked" ? tr("Zablokowany") : tr("Czeka") }];
      waitingRows.push(row);
    }
  }

  // Blocked tasks outside topics: attention when their check is due, else waiting.
  const loose: Task[] = [];
  for (const t of snap.tasks) {
    if (t.status === "done" || t.location.kind === "topic") continue;
    if (t.location.kind === "project" && !active.has(t.location.project)) continue;
    const w = t.id ? (waitOf.get(`task:${t.id}`) ?? null) : null;
    if (t.status === "blocked") {
      const row: TaskRow = { kind: "task", key: `task:${t.id ?? t.title}`, task: t, wait: w, reasons: [] };
      if (w?.due) {
        row.reasons = [{ kind: "task_check", text: dueText(w, today) }];
        attention.push(row);
      } else {
        row.reasons = [{ kind: "waiting", text: w ? waitText(w, today) : tr("Zablokowane") }];
        waitingRows.push(row);
      }
      continue;
    }
    const due = dueDate(t);
    if (t.status === "doing" || (due !== null && due <= today)) loose.push(t);
  }

  const recent = (r: TopicRow) => r.last?.at ?? r.topic.updated ?? "";
  attention.sort((a, b) => rank(a) - rank(b) || (b.kind === "topic" ? recent(b) : "").localeCompare(a.kind === "topic" ? recent(a) : ""));
  activeRows.sort((a, b) => recent(b).localeCompare(recent(a)));
  waitingRows.sort((a, b) => (a.wait?.next ?? "9999").localeCompare(b.wait?.next ?? "9999"));
  loose.sort((a, b) => (dueDate(a) ?? "9999").localeCompare(dueDate(b) ?? "9999"));
  return { attention, active: activeRows, waiting: waitingRows, loose };
}

/** Items counted as "Wymaga uwagi". */
export function attentionCount(s: TodaySections): number {
  return s.attention.length;
}
