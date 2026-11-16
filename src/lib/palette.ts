import { t as tr } from "../i18n";
// Result model of the ⌘K palette: search hits grouped for display plus
// navigation targets. Pure, so ordering and limits are testable.

import type { JournalEntry, SearchHit, Snapshot, Task, Topic } from "../api/types";
import { href } from "../router/registry";
import { partitionProjects } from "./group";
import { fold } from "./text";

export type PaletteGroup = "nav" | "task" | "topic" | "journal" | "new";

export const GROUP_LABEL: Record<PaletteGroup, string> = {
  get nav() { return tr("Przejdź do"); },
  get task() { return tr("Zadania"); },
  get topic() { return tr("Tematy"); },
  get journal() { return tr("Dziennik"); },
  get new() { return tr("Nowe"); },
};

export type PaletteItem =
  | { group: "nav"; key: string; label: string; to: string; color: string | null }
  | { group: "task"; key: string; task: Task }
  | { group: "topic"; key: string; topic: Topic }
  | { group: "journal"; key: string; entry: JournalEntry }
  /** The query as a new task or command (read by the backend, written after confirmation). */
  | { group: "new"; key: "new"; text: string };

export const GROUP_LIMIT: Record<PaletteGroup, number> = { nav: 5, task: 8, topic: 6, journal: 6, new: 1 };

export function navTargets(snap: Snapshot): PaletteItem[] {
  const items: PaletteItem[] = [
    { group: "nav", key: "nav:dzis", label: tr("Dziś"), to: href("dzis"), color: null },
    { group: "nav", key: "nav:skrzynka", label: tr("Skrzynka"), to: href("skrzynka"), color: null },
    { group: "nav", key: "nav:tablica", label: tr("Tablica"), to: href("tablica"), color: null },
    { group: "nav", key: "nav:mapa", label: tr("Mapa"), to: href("mapa"), color: null },
    { group: "nav", key: "nav:akcje", label: tr("Akcje"), to: href("akcje"), color: null },
    { group: "nav", key: "nav:ustawienia", label: tr("Ustawienia"), to: href("ustawienia"), color: null },
  ];
  for (const p of partitionProjects(snap.projects).visible) {
    items.push({
      group: "nav",
      key: `nav:p:${p.slug}`,
      label: p.name,
      to: href("p/:project", { project: p.slug }),
      color: p.color,
    });
  }
  return items;
}

function taskKey(t: Task): string {
  if (t.id) return `task:${t.id}`;
  const l = t.location;
  return `task:${l.kind === "inbox" ? "inbox" : l.kind === "project" ? l.project : `${l.project}/${l.topic}`}:${t.line}`;
}

function findTask(snap: Snapshot, t: Task): Task | undefined {
  if (t.id) return snap.tasks.find((x) => x.id === t.id);
  // No id yet: only the same line of the same file is the same task.
  return snap.tasks.find((x) => x.id === null && x.line === t.line && JSON.stringify(x.location) === JSON.stringify(t.location));
}

function findTopic(snap: Snapshot, project: string, slug: string): Topic | undefined {
  return snap.topics.find((t) => t.project === project && t.slug === slug);
}

function findEntry(snap: Snapshot, e: JournalEntry): JournalEntry | undefined {
  // Entries have no id; lines shift when entries are added. Match on content:
  // an entry that is gone (or was rewritten) is no longer a valid result.
  return snap.journal.find((x) => x.project === e.project && x.topic === e.topic && x.at === e.at && x.text === e.text);
}

/**
 * `item` as it is in `snap` (fresh location, title, status), or null when it
 * no longer exists there (deleted task, topic, project or journal entry).
 */
export function resolveItem(snap: Snapshot, item: PaletteItem): PaletteItem | null {
  switch (item.group) {
    case "new":
      return item;
    case "nav": {
      const slug = item.key.startsWith("nav:p:") ? item.key.slice("nav:p:".length) : null;
      return !slug || snap.projects.some((p) => p.slug === slug) ? item : null;
    }
    case "task": {
      const task = findTask(snap, item.task);
      return task ? { group: "task", key: taskKey(task), task } : null;
    }
    case "topic": {
      const topic = findTopic(snap, item.topic.project, item.topic.slug);
      return topic ? { ...item, topic } : null;
    }
    case "journal": {
      const entry = findTopic(snap, item.entry.project, item.entry.topic) ? findEntry(snap, item.entry) : undefined;
      return entry ? { ...item, entry } : null;
    }
  }
}

/** Items for the current query: navigation when empty, else grouped hits (+ matching targets). */
export function buildItems(snap: Snapshot, query: string, hits: SearchHit[]): PaletteItem[] {
  if (!query.trim()) return navTargets(snap);
  const q = fold(query).split(/\s+/).filter(Boolean);
  const nav = navTargets(snap)
    .filter((n) => n.group === "nav" && q.every((t) => fold(n.label).includes(t)))
    .slice(0, GROUP_LIMIT.nav);
  const tasks: PaletteItem[] = [];
  const topics: PaletteItem[] = [];
  const journal: PaletteItem[] = [];
  // Hits may come from an older snapshot than `snap` (the search reruns after a
  // change, but asynchronously): show each one as it is now, skip the gone ones.
  for (const h of hits) {
    if (h.kind === "task" && tasks.length < GROUP_LIMIT.task) {
      const item = resolveItem(snap, { group: "task", key: taskKey(h.task), task: h.task });
      if (item) tasks.push(item);
    } else if (h.kind === "topic" && topics.length < GROUP_LIMIT.topic) {
      const item = resolveItem(snap, { group: "topic", key: `topic:${h.topic.project}/${h.topic.slug}`, topic: h.topic });
      if (item) topics.push(item);
    } else if (h.kind === "journal" && journal.length < GROUP_LIMIT.journal) {
      const e = h.entry;
      const item = resolveItem(snap, { group: "journal", key: `j:${e.project}/${e.topic}/${e.at}/${e.line}`, entry: e });
      if (item) journal.push(item);
    }
  }
  // Last: the query itself as a new task or command.
  return [...tasks, ...topics, ...journal, ...nav, { group: "new", key: "new", text: query.trim() }];
}

/**
 * Index to highlight after the items changed: the top for a new query,
 * otherwise the same item as before (by key) when it is still there.
 */
export function nextActive(prev: { items: PaletteItem[]; active: number; query: string }, items: PaletteItem[], query: string): number {
  if (prev.query.trim() !== query.trim()) return 0;
  const key = prev.items[prev.active]?.key;
  const i = key === undefined ? -1 : items.findIndex((x) => x.key === key);
  return i < 0 ? 0 : i;
}
