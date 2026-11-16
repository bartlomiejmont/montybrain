import { getLocale } from "../i18n";
// Pure logic of the kanban view: columns, grouping, sorting, status moves.

import type { Snapshot, Task, TaskLocation, TaskStatus, Topic, TopicStatus } from "../api/types";
import { daysBetween } from "./dates";

export type BoardMode = "tasks" | "topics";
/** "all" = every active project, otherwise a project slug. */
export type BoardScope = string;
export const ALL_PROJECTS = "all";

export const TASK_COLUMNS: TaskStatus[] = ["todo", "doing", "blocked", "done"];
export const TOPIC_COLUMNS: TopicStatus[] = ["planned", "in_progress", "blocked", "waiting", "done"];
export type TopicColumn = TopicStatus | "archive";
export const TOPIC_BOARD_COLUMNS: TopicColumn[] = [...TOPIC_COLUMNS, "archive"];
export const TOPIC_ARCHIVE_DAYS = 7;

/** Archive is derived, never a status to write. Archived → done is a no-op. */
export function topicMoveColumns(from: TopicColumn): TopicStatus[] {
  return TOPIC_COLUMNS.filter((col) => from !== "archive" || col !== "done");
}

/** How many cards an expanded "done" column shows. */
export const DONE_CAP = 25;

const PRIORITY_RANK = { high: 0, normal: 1, low: 2 } as const;

/** Projects shown for a scope; an unknown slug falls back to all active projects. */
export function scopeSlugs(snap: Snapshot, scope: BoardScope): Set<string> {
  const known = snap.projects.find((p) => p.slug === scope && p.state !== "archived");
  if (known) return new Set([known.slug]);
  return new Set(snap.projects.filter((p) => p.state === "active").map((p) => p.slug));
}

export function effectiveScope(snap: Snapshot, scope: BoardScope): BoardScope {
  return snap.projects.some((p) => p.slug === scope && p.state !== "archived") ? scope : ALL_PROJECTS;
}

export type Grouped<S extends string, T> = Record<S, T[]>;

function empty<S extends string, T>(columns: S[]): Grouped<S, T> {
  return Object.fromEntries(columns.map((c) => [c, [] as T[]])) as Grouped<S, T>;
}

/** Open cards: priority, then due date (none last), then file order. Done: newest first. */
export function compareTasks(a: Task, b: Task, ia = 0, ib = 0): number {
  if (a.status === "done" && b.status === "done") {
    // A card just moved to "done" has no stamp yet: it is the newest.
    return (b.done ?? "9999").localeCompare(a.done ?? "9999") || ia - ib;
  }
  const p = PRIORITY_RANK[a.priority] - PRIORITY_RANK[b.priority];
  if (p) return p;
  if (a.due && b.due && a.due !== b.due) return a.due < b.due ? -1 : 1;
  if (a.due && !b.due) return -1;
  if (!a.due && b.due) return 1;
  return ia - ib;
}

/**
 * Task columns. `overrides` maps task id → status shown optimistically while a
 * move is in flight. Inbox tasks have no project and are not on the board.
 */
export function groupTasks(
  snap: Snapshot,
  scope: BoardScope,
  overrides: ReadonlyMap<string, TaskStatus> = new Map(),
  locations: ReadonlyMap<string, TaskLocation> = new Map(),
): Grouped<TaskStatus, Task> {
  const slugs = scopeSlugs(snap, scope);
  const out = empty<TaskStatus, Task>(TASK_COLUMNS);
  const indexed = new Map<Task, number>();
  snap.tasks.forEach((t, i) => {
    const location = (t.id && locations.get(t.id)) || t.location;
    if (location.kind === "inbox" || !slugs.has(location.project)) return;
    const status = (t.id && overrides.get(t.id)) || t.status;
    const shown = status === t.status && location === t.location ? t : { ...t, status, location };
    indexed.set(shown, i);
    out[status].push(shown);
  });
  for (const col of TASK_COLUMNS) out[col].sort((a, b) => compareTasks(a, b, indexed.get(a), indexed.get(b)));
  return out;
}

export interface TaskLane {
  key: string;
  topic: Topic | null;
  columns: Grouped<TaskStatus, Task>;
  total: number;
  hiddenDone: number;
}

/** One continuous row per shared topic. Single tasks have no topic enclosure.
 * Counts use all scoped tasks; hiding/capping done never breaks a shared topic.
 */
export function taskLanes(columns: Grouped<TaskStatus, Task>, topics: Topic[], doneOpen: boolean): TaskLane[] {
  const all = TASK_COLUMNS.flatMap((c) => columns[c]);
  const byTopic = new Map(topics.map((t) => [topicId(t), t]));
  const keyOf = (t: Task) => t.location.kind === "topic" ? `${t.location.project}/${t.location.topic}` : null;
  const totals = new Map<string, number>();
  for (const task of all) {
    const key = keyOf(task);
    if (key) totals.set(key, (totals.get(key) ?? 0) + 1);
  }
  const lanes = new Map<string, TaskLane>();
  const shownDone = new Set(doneOpen ? capColumn(columns.done, true).shown : []);
  for (const task of all) {
    const key = keyOf(task);
    const topic = key && (totals.get(key) ?? 0) >= 2 ? byTopic.get(key) ?? null : null;
    const laneKey = topic ? topicId(topic) : "ungrouped";
    let lane = lanes.get(laneKey);
    if (!lane) {
      lane = { key: laneKey, topic, columns: empty(TASK_COLUMNS), total: 0, hiddenDone: 0 };
      lanes.set(laneKey, lane);
    }
    lane.total++;
    if (task.status === "done" && !shownDone.has(task)) lane.hiddenDone++;
    else lane.columns[task.status].push(task);
  }
  const order = new Map(topics.map((t, i) => [topicId(t), i]));
  return [...lanes.values()].filter((l) => TASK_COLUMNS.some((c) => l.columns[c].length)).sort((a, b) => (order.get(a.key) ?? Number.MAX_SAFE_INTEGER) - (order.get(b.key) ?? Number.MAX_SAFE_INTEGER));
}

/** Topic columns, newest first. No completion stamp exists on Topic, so use
 * updated (then created), in calendar days relative to the app's today.
 * Editing a done topic therefore returns it to done for another seven days.
 */
export function groupTopics(
  snap: Snapshot,
  scope: BoardScope,
  today: string,
  overrides: ReadonlyMap<string, TopicStatus> = new Map(),
): Grouped<TopicColumn, Topic> {
  const slugs = scopeSlugs(snap, scope);
  const out = empty<TopicColumn, Topic>(TOPIC_BOARD_COLUMNS);
  snap.topics.forEach((t) => {
    if (!slugs.has(t.project)) return;
    const status = overrides.get(topicId(t)) ?? t.status;
    // A newly completed optimistic card must not inherit its old update date.
    const shown = status === t.status ? t : { ...t, status, updated: `${today}T00:00` };
    const stamp = shown.updated ?? shown.created;
    const archived = status === "done" && stamp !== null && daysBetween(stamp.slice(0, 10), today) >= TOPIC_ARCHIVE_DAYS;
    out[archived ? "archive" : status].push(shown);
  });
  for (const col of TOPIC_BOARD_COLUMNS) {
    out[col].sort(
      (a, b) =>
        (b.updated ?? b.created ?? "").localeCompare(a.updated ?? a.created ?? "") || a.title.localeCompare(b.title, getLocale()),
    );
  }
  return out;
}

export function topicId(t: Pick<Topic, "project" | "slug">): string {
  return `${t.project}/${t.slug}`;
}

/** Visible cards of a column: all, or the newest `DONE_CAP` of a "done" column. */
export function capColumn<T>(items: T[], isDone: boolean): { shown: T[]; hidden: number } {
  if (!isDone || items.length <= DONE_CAP) return { shown: items, hidden: 0 };
  return { shown: items.slice(0, DONE_CAP), hidden: items.length - DONE_CAP };
}

/** Neighbouring column for keyboard moves; null at the edges. */
export function adjacentColumn<S extends string>(columns: readonly S[], from: S, dir: 1 | -1): S | null {
  const i = columns.indexOf(from);
  return columns[i + dir] ?? null;
}

/** First line of a topic summary, for the card. */
export function firstLine(text: string): string {
  return text.split("\n").find((l) => l.trim())?.trim() ?? "";
}

// ----- optimistic moves -----

/** A move shown before the backend confirms it; `op` identifies the request. */
export interface PendingMove<S> {
  op: number;
  to: S;
}
export type PendingMoves<S> = ReadonlyMap<string, PendingMove<S>>;

/** Card `id` is being moved to `to` by operation `op` (replaces an older pending move). */
export function beginMove<S>(m: PendingMoves<S>, id: string, op: number, to: S): PendingMoves<S> {
  return new Map(m).set(id, { op, to });
}

/**
 * Operation `op` settled (succeeded or failed). Only its own override is
 * cleared: a newer move of the same card stays. Returns `m` when unchanged.
 */
export function settleMove<S>(m: PendingMoves<S>, id: string, op: number): PendingMoves<S> {
  if (m.get(id)?.op !== op) return m;
  const n = new Map(m);
  n.delete(id);
  return n;
}

/** Card id → status to show, for `groupTasks` / `groupTopics`. */
export function moveOverrides<S>(m: PendingMoves<S>): Map<string, S> {
  return new Map([...m].map(([id, p]) => [id, p.to]));
}

/**
 * Runs tasks with the same key one after another (in call order), tasks with
 * different keys in parallel. A failed task does not stop the next one.
 */
export function createSerialQueue() {
  const tails = new Map<string, Promise<unknown>>();
  return function enqueue<T>(key: string, task: () => Promise<T>): Promise<T> {
    const prev = tails.get(key) ?? Promise.resolve();
    const run = prev.then(task, task);
    const tail = run.catch(() => undefined);
    tails.set(key, tail);
    void tail.then(() => {
      if (tails.get(key) === tail) tails.delete(key);
    });
    return run;
  };
}
