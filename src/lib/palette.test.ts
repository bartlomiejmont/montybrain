import { describe, expect, it } from "vitest";

import { buildFixtures } from "../api/fixtures";
import { computeSearch } from "../api/mock";
import type { Snapshot, Task } from "../api/types";
import { buildItems, nextActive, resolveItem, type PaletteItem } from "./palette";

const before: Snapshot = buildFixtures("2026-11-13", "2026-11-13T10:00");

/** Two tasks in topics; after an external change one moved to another topic, one is gone. */
function scenario() {
  const inTopics = before.tasks.filter((t) => t.id && t.location.kind === "topic");
  const moved = inTopics[0]!;
  const deleted = inTopics.find((t) => t.id !== moved.id)!;
  const target = before.topics.find(
    (tp) => moved.location.kind === "topic" && !(tp.project === moved.location.project && tp.slug === moved.location.topic),
  )!;
  const after: Snapshot = structuredClone(before);
  after.tasks = after.tasks
    .filter((t) => t.id !== deleted.id)
    .map((t) => (t.id === moved.id ? { ...t, location: { kind: "topic", project: target.project, topic: target.slug } } : t));
  const hits = [moved, deleted].map((task) => ({ kind: "task" as const, task }));
  return { moved, deleted, target, after, hits };
}

const taskOf = (i: PaletteItem | null | undefined): Task | undefined => (i && i.group === "task" ? i.task : undefined);

// F5: results follow the latest snapshot; activation resolves against it.
describe("palette results after an external change", () => {
  it("shows the task where it is now and drops a deleted one", () => {
    const { moved, deleted, target, after, hits } = scenario();
    const items = buildItems(after, "x", hits);
    const tasks = items.map(taskOf).filter(Boolean) as Task[];
    expect(tasks.map((t) => t.id)).toEqual([moved.id]);
    expect(tasks[0]!.location).toEqual({ kind: "topic", project: target.project, topic: target.slug });
    expect(tasks.some((t) => t.id === deleted.id)).toBe(false);
  });

  it("resolves the chosen item against the latest snapshot at activation", () => {
    const { moved, deleted, target, after, hits } = scenario();
    const stale = buildItems(before, "x", hits);
    const pick = (id: string | null) => stale.find((i) => taskOf(i)?.id === id)!;
    expect(taskOf(resolveItem(after, pick(moved.id)))?.location).toEqual({ kind: "topic", project: target.project, topic: target.slug });
    expect(resolveItem(after, pick(deleted.id))).toBeNull();
  });

  it("resolves topics and journal entries, null when their topic is gone", () => {
    const topic = before.topics[0]!;
    const entry = before.journal.find((j) => j.project === topic.project && j.topic === topic.slug);
    const after = structuredClone(before);
    after.topics = after.topics.filter((t) => !(t.project === topic.project && t.slug === topic.slug));
    const items = buildItems(before, "x", [
      { kind: "topic", topic },
      ...(entry ? [{ kind: "journal" as const, entry }] : []),
    ]);
    for (const i of items) if (i.group !== "nav" && i.group !== "new") expect(resolveItem(after, i)).toBeNull();
    expect(buildItems(after, "x", [{ kind: "topic", topic }]).some((i) => i.group === "topic")).toBe(false);
    const renamed = structuredClone(before);
    renamed.topics[0]!.title = "Nowy tytuł";
    expect(resolveItem(renamed, items[0]!)).toMatchObject({ group: "topic", topic: { title: "Nowy tytuł" } });
  });

  // N4: deleting only the entry (its topic stays) must drop it from the results.
  it("drops a deleted journal entry whose topic still exists", () => {
    const entry = before.journal[0]!;
    const after = structuredClone(before);
    after.journal = after.journal.filter(
      (j) => !(j.project === entry.project && j.topic === entry.topic && j.at === entry.at && j.text === entry.text),
    );
    expect(after.topics.some((t) => t.project === entry.project && t.slug === entry.topic)).toBe(true);
    const stale = buildItems(before, "x", [{ kind: "journal", entry }]);
    expect(stale.filter((i) => i.group === "journal")).toHaveLength(1);
    // activation of a result shown before the deletion
    expect(resolveItem(after, stale[0]!)).toBeNull();
    // a search hit from an older snapshot is not shown
    expect(buildItems(after, "x", [{ kind: "journal", entry }]).some((i) => i.group === "journal")).toBe(false);
    // an entry that is still there resolves to its current object
    const kept = after.journal[0]!;
    const item = buildItems(before, "x", [{ kind: "journal", entry: kept }])[0]!;
    expect(resolveItem(after, item)).toMatchObject({ group: "journal", entry: kept });
  });

  it("keeps the selection on the same item when results refresh", () => {
    const { after } = scenario();
    const q = "do";
    const old = buildItems(before, q, computeSearch(before, q));
    const fresh = buildItems(after, q, computeSearch(after, q));
    expect(old.length).toBeGreaterThan(1);
    const chosen = old.length - 1;
    const kept = nextActive({ items: old, active: chosen, query: q }, fresh, q);
    expect(fresh[kept]?.key).toBe(old[chosen]!.key);
    // the query changed: start from the top
    expect(nextActive({ items: old, active: chosen, query: q }, fresh, `${q}x`)).toBe(0);
    // the chosen item vanished: fall back to the top
    expect(nextActive({ items: old, active: chosen, query: q }, fresh.slice(0, 1).concat([]), q)).toBe(0);
  });

  it("ends with the query as a new task or command", () => {
    const snap = structuredClone(before);
    const items = buildItems(snap, "Kupić mleko", []);
    expect(items.at(-1)).toEqual({ group: "new", key: "new", text: "Kupić mleko" });
    expect(buildItems(snap, "", []).some((i) => i.group === "new")).toBe(false);
  });
});
