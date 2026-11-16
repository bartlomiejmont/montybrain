import { getLocale, t as tr } from "../i18n";
// Pure helpers that slice the snapshot for the views. No React here, so they are
// easy to test and reuse in later views (kanban, map).

import type {
  JournalEntry,
  Project,
  Snapshot,
  Task,
  TaskLocation,
  TodayItem,
  TodayReason,
  Topic,
  TopicStatus,
} from "../api/types";
import { TODAY_REASONS } from "./labels";

export interface Counts {
  open: number;
  total: number;
}

export function locationKey(loc: TaskLocation): string {
  switch (loc.kind) {
    case "inbox":
      return "inbox";
    case "project":
      return `p:${loc.project}`;
    case "topic":
      return `t:${loc.project}/${loc.topic}`;
  }
}

/**
 * Key for drafts of one task's fields: its id, so a draft survives the task
 * moving to another list. A task without an id yet (its fields are read-only)
 * is identified by its file and line.
 */
export function taskDraftKey(task: Task): string {
  return task.id ? `task:${task.id}` : `task@${locationKey(task.location)}#${task.line}`;
}

export function sameLocation(a: TaskLocation, b: TaskLocation): boolean {
  return locationKey(a) === locationKey(b);
}

export function topicKey(project: string, topic: string): string {
  return `${project}/${topic}`;
}

/** Tasks living exactly at `loc` (file order). */
export function tasksAt(snap: Snapshot, loc: TaskLocation): Task[] {
  const key = locationKey(loc);
  return snap.tasks.filter((t) => locationKey(t.location) === key);
}

function bump(map: Map<string, Counts>, key: string, done: boolean) {
  const c = map.get(key) ?? { open: 0, total: 0 };
  c.total += 1;
  if (!done) c.open += 1;
  map.set(key, c);
}

export interface TaskCounts {
  inbox: Counts;
  /** By project slug: projekt.md + all topics. */
  project: Map<string, Counts>;
  /** By `project/topic`. */
  topic: Map<string, Counts>;
}

export function countTasks(snap: Snapshot): TaskCounts {
  const inbox: Counts = { open: 0, total: 0 };
  const project = new Map<string, Counts>();
  const topic = new Map<string, Counts>();
  for (const t of snap.tasks) {
    const done = t.status === "done";
    const loc = t.location;
    if (loc.kind === "inbox") {
      inbox.total += 1;
      if (!done) inbox.open += 1;
      continue;
    }
    bump(project, loc.project, done);
    if (loc.kind === "topic") bump(topic, topicKey(loc.project, loc.topic), done);
  }
  return { inbox, project, topic };
}

/** Open tasks first in file order, done tasks newest-completed first. */
export function splitDone(tasks: Task[]): { open: Task[]; done: Task[] } {
  const open = tasks.filter((t) => t.status !== "done");
  const done = tasks
    .filter((t) => t.status === "done")
    .map((t, i) => ({ t, i }))
    .sort((a, b) => (b.t.done ?? "").localeCompare(a.t.done ?? "") || a.i - b.i)
    .map(({ t }) => t);
  return { open, done };
}

const TOPIC_ORDER: Record<TopicStatus, number> = {
  in_progress: 0,
  blocked: 1,
  waiting: 2,
  planned: 3,
  done: 4,
};

/** Active work first (w toku, zablokowany, czeka, planowany, zrobiony), then most recently updated. */
export function sortTopics(topics: Topic[]): Topic[] {
  return [...topics].sort(
    (a, b) =>
      TOPIC_ORDER[a.status] - TOPIC_ORDER[b.status] ||
      (b.updated ?? b.created ?? "").localeCompare(a.updated ?? a.created ?? "") ||
      a.title.localeCompare(b.title, getLocale()),
  );
}

export interface TodaySection {
  reason: TodayReason;
  items: TodayItem[];
}

/** Core already sorts; this only buckets, keeping core's order and dropping empty sections. */
export function groupToday(items: TodayItem[]): TodaySection[] {
  return TODAY_REASONS.map((reason) => ({
    reason,
    items: items.filter((i) => i.reason === reason),
  })).filter((s) => s.items.length > 0);
}

/** Visible projects for the sidebar: non-archived sorted by name, archived separately. */
export function partitionProjects(projects: Project[]): { visible: Project[]; archived: Project[] } {
  const byName = (a: Project, b: Project) => a.name.localeCompare(b.name, getLocale());
  return {
    visible: projects.filter((p) => p.state !== "archived").sort(byName),
    archived: projects.filter((p) => p.state === "archived").sort(byName),
  };
}

export interface MoveTarget {
  key: string;
  label: string;
  group: string;
  location: TaskLocation;
}

/** Every place a task can be moved to (archived projects and done topics excluded unless current). */
export function moveTargets(snap: Snapshot, current?: TaskLocation): MoveTarget[] {
  const out: MoveTarget[] = [{ key: "inbox", label: tr("Skrzynka"), group: "", location: { kind: "inbox" } }];
  const currentProject = current && current.kind !== "inbox" ? current.project : null;
  for (const p of partitionProjects(snap.projects).visible.concat(
    snap.projects.filter((p) => p.state === "archived" && p.slug === currentProject),
  )) {
    const projectLoc: TaskLocation = { kind: "project", project: p.slug };
    out.push({ key: locationKey(projectLoc), label: tr("{0} — ogólne", p.name), group: p.name, location: projectLoc });
    const topics = snap.topics.filter(
      (t) =>
        t.project === p.slug &&
        (t.status !== "done" || (current?.kind === "topic" && current.project === p.slug && current.topic === t.slug)),
    );
    for (const t of sortTopics(topics)) {
      const loc: TaskLocation = { kind: "topic", project: p.slug, topic: t.slug };
      out.push({ key: locationKey(loc), label: t.title, group: p.name, location: loc });
    }
  }
  return out;
}

export interface TaskContext {
  project: Project | null;
  topic: Topic | null;
}

export function taskContext(snap: Snapshot, loc: TaskLocation): TaskContext {
  if (loc.kind === "inbox") return { project: null, topic: null };
  const project = snap.projects.find((p) => p.slug === loc.project) ?? null;
  const topic =
    loc.kind === "topic" ? (snap.topics.find((t) => t.project === loc.project && t.slug === loc.topic) ?? null) : null;
  return { project, topic };
}

/** Journal of one topic, newest first (same order as core's `journal_of`). */
export function journalOf(snap: Snapshot, project: string, topic: string): JournalEntry[] {
  return snap.journal
    .filter((j) => j.project === project && j.topic === topic)
    .map((j, i) => ({ j, i }))
    .sort((a, b) => b.j.at.localeCompare(a.j.at) || a.j.line - b.j.line || a.i - b.i)
    .map(({ j }) => j);
}

/** Path of the file holding tasks at `loc`, relative to the vault root. */
export function locationPath(loc: TaskLocation): string {
  switch (loc.kind) {
    case "inbox":
      return "skrzynka.md";
    case "project":
      return `${loc.project}/projekt.md`;
    case "topic":
      return `${loc.project}/tematy/${loc.topic}.md`;
  }
}
