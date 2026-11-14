// Pure logic of the kanban view: columns, grouping, sorting, status moves.

import type { Snapshot, Task, TaskStatus, Topic, TopicStatus } from "../api/types";

export type BoardMode = "tasks" | "topics";
/** "all" = every active project, otherwise a project slug. */
export type BoardScope = string;
export const ALL_PROJECTS = "all";

export const TASK_COLUMNS: TaskStatus[] = ["todo", "doing", "blocked", "done"];
export const TOPIC_COLUMNS: TopicStatus[] = ["planned", "in_progress", "blocked", "waiting", "done"];

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
): Grouped<TaskStatus, Task> {
  const slugs = scopeSlugs(snap, scope);
  const out = empty<TaskStatus, Task>(TASK_COLUMNS);
  const indexed = new Map<Task, number>();
  snap.tasks.forEach((t, i) => {
    if (t.location.kind === "inbox" || !slugs.has(t.location.project)) return;
    const status = (t.id && overrides.get(t.id)) || t.status;
    const shown = status === t.status ? t : { ...t, status };
    indexed.set(shown, i);
    out[status].push(shown);
  });
  for (const col of TASK_COLUMNS) out[col].sort((a, b) => compareTasks(a, b, indexed.get(a), indexed.get(b)));
  return out;
}

/** Topic columns, most recently updated first. */
export function groupTopics(
  snap: Snapshot,
  scope: BoardScope,
  overrides: ReadonlyMap<string, TopicStatus> = new Map(),
): Grouped<TopicStatus, Topic> {
  const slugs = scopeSlugs(snap, scope);
  const out = empty<TopicStatus, Topic>(TOPIC_COLUMNS);
  snap.topics.forEach((t) => {
    if (!slugs.has(t.project)) return;
    const status = overrides.get(topicId(t)) ?? t.status;
    out[status].push(status === t.status ? t : { ...t, status });
  });
  for (const col of TOPIC_COLUMNS) {
    out[col].sort(
      (a, b) =>
        (b.updated ?? b.created ?? "").localeCompare(a.updated ?? a.created ?? "") || a.title.localeCompare(b.title, "pl"),
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
export interface PendingMove<S extends string> {
  op: number;
  to: S;
}
export type PendingMoves<S extends string> = ReadonlyMap<string, PendingMove<S>>;

/** Card `id` is being moved to `to` by operation `op` (replaces an older pending move). */
export function beginMove<S extends string>(m: PendingMoves<S>, id: string, op: number, to: S): PendingMoves<S> {
  return new Map(m).set(id, { op, to });
}

/**
 * Operation `op` settled (succeeded or failed). Only its own override is
 * cleared: a newer move of the same card stays. Returns `m` when unchanged.
 */
export function settleMove<S extends string>(m: PendingMoves<S>, id: string, op: number): PendingMoves<S> {
  if (m.get(id)?.op !== op) return m;
  const n = new Map(m);
  n.delete(id);
  return n;
}

/** Card id → status to show, for `groupTasks` / `groupTopics`. */
export function moveOverrides<S extends string>(m: PendingMoves<S>): Map<string, S> {
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
