import { describe, expect, it } from "vitest";

import { buildFixtures } from "../api/fixtures";
import type { Snapshot, Task, TaskStatus, Topic } from "../api/types";
import {
  ALL_PROJECTS,
  DONE_CAP,
  TOPIC_ARCHIVE_DAYS,
  TOPIC_BOARD_COLUMNS,
  TASK_COLUMNS,
  adjacentColumn,
  beginMove,
  capColumn,
  createSerialQueue,
  moveOverrides,
  settleMove,
  effectiveScope,
  firstLine,
  groupTasks,
  groupTopics,
  taskLanes,
  topicMoveColumns,
  type PendingMoves,
} from "./board";
import { buildItems } from "./palette";

const snap: Snapshot = buildFixtures("2026-11-13", "2026-11-13T10:00");
const ids = (ts: Task[]) => ts.map((t) => t.id);

describe("board grouping", () => {
  it("puts every project task in its status column and skips the inbox", () => {
    const cols = groupTasks(snap, ALL_PROJECTS);
    const all = TASK_COLUMNS.flatMap((c) => cols[c]);
    const active = new Set(snap.projects.filter((p) => p.state === "active").map((p) => p.slug));
    expect(all.length).toBe(snap.tasks.filter((t) => t.location.kind !== "inbox" && active.has(t.location.project)).length);
    for (const c of TASK_COLUMNS) for (const t of cols[c]) expect(t.status).toBe(c);
  });

  it("sorts open cards by priority, then due date, then file order", () => {
    const todo = groupTasks(snap, "whenubuy").todo;
    const rank = todo.map((t) => `${t.priority}|${t.due ?? "-"}`);
    // normal+due before normal without due; low last
    const firstLow = todo.findIndex((t) => t.priority === "low");
    expect(firstLow).toBeGreaterThan(0);
    expect(todo.slice(firstLow).every((t) => t.priority === "low")).toBe(true);
    expect(rank.length).toBeGreaterThan(2);
    const dues = todo.filter((t) => t.priority === "normal" && t.due).map((t) => t.due!);
    expect(dues).toEqual([...dues].sort());
  });

  it("sorts done cards newest first and caps the list", () => {
    const done = groupTasks(snap, "whenubuy").done;
    expect(done.map((t) => t.done)).toEqual([...done.map((t) => t.done)].sort().reverse());
    const many = Array.from({ length: DONE_CAP + 7 }, (_, i) => i);
    expect(capColumn(many, true)).toEqual({ shown: many.slice(0, DONE_CAP), hidden: 7 });
    expect(capColumn(many, false).hidden).toBe(0);
  });

  it("scopes to one project and falls back to all active ones for an unknown slug", () => {
    const one = groupTasks(snap, "montybrain");
    expect(TASK_COLUMNS.flatMap((c) => one[c]).every((t) => t.location.kind !== "inbox" && t.location.project === "montybrain")).toBe(true);
    expect(effectiveScope(snap, "nie-ma")).toBe(ALL_PROJECTS);
    expect(effectiveScope(snap, "montybrain")).toBe("montybrain");
  });

  it("shows a pending move in the new column at once; a rolled back move returns", () => {
    const t = groupTasks(snap, ALL_PROJECTS).todo[0]!;
    const moved = groupTasks(snap, ALL_PROJECTS, new Map([[t.id!, "doing" as const]]));
    expect(ids(moved.todo)).not.toContain(t.id);
    expect(ids(moved.doing)).toContain(t.id);
    expect(moved.doing.find((x) => x.id === t.id)!.status).toBe("doing");
    // optimistic "done" has no stamp yet and is the newest
    const done = groupTasks(snap, ALL_PROJECTS, new Map([[t.id!, "done" as const]])).done;
    expect(done[0]!.id).toBe(t.id);
    // override removed -> original grouping
    expect(ids(groupTasks(snap, ALL_PROJECTS).todo)).toContain(t.id);
  });

  it("groups topics by status, most recently updated first", () => {
    const cols = groupTopics(snap, "whenubuy", "2026-11-13");
    expect(cols.in_progress.map((t) => t.slug)).toContain("ab-testy-cen");
    expect(cols.archive.map((t) => t.slug)).toEqual(["onboarding"]);
    const moved = groupTopics(snap, "whenubuy", "2026-11-13", new Map([["whenubuy/ab-testy-cen", "waiting" as const]]));
    expect(moved.waiting.map((t) => t.slug)).toContain("ab-testy-cen");
  });

  it("finds neighbouring columns for keyboard moves", () => {
    expect(adjacentColumn(TASK_COLUMNS, "todo", 1)).toBe("doing");
    expect(adjacentColumn(TASK_COLUMNS, "todo", -1)).toBeNull();
    expect(adjacentColumn(TASK_COLUMNS, "done", 1)).toBeNull();
    expect(firstLine("\n  pierwsza\ndruga")).toBe("pierwsza");
  });
});

describe("topic archive", () => {
  const today = "2026-11-15";
  const topic = (slug: string, patch: Partial<Topic> = {}): Topic => ({
    ...snap.topics[0]!, project: "whenubuy", slug, title: slug, status: "done",
    updated: "2026-11-08T23:59", created: "2026-10-01", ...patch,
  });
  const snapshot = (...topics: Topic[]): Snapshot => ({ ...snap, topics });

  it("splits at exactly seven calendar days using the supplied today", () => {
    expect(TOPIC_ARCHIVE_DAYS).toBe(7);
    const data = snapshot(topic("boundary"), topic("recent", { updated: "2026-11-09T00:00" }), topic("future", { updated: "2026-11-16T00:00" }), topic("active", { status: "in_progress" }));
    const cols = groupTopics(data, ALL_PROJECTS, today);
    expect(cols.archive.map((t) => t.slug)).toEqual(["boundary"]);
    expect(cols.done.map((t) => t.slug)).toEqual(["future", "recent"]);
    expect(cols.in_progress.map((t) => t.slug)).toEqual(["active"]);
    expect(groupTopics(data, ALL_PROJECTS, "2026-11-14").archive).toHaveLength(0);
    expect(TOPIC_BOARD_COLUMNS.flatMap((c) => cols[c])).toHaveLength(data.topics.length);
    expect(data.topics[0]!.status).toBe("done");
  });

  it("uses created if updated is missing and keeps undated topics visible in done", () => {
    const cols = groupTopics(snapshot(topic("created", { updated: null, created: "2026-11-08" }), topic("undated", { updated: null, created: null }), topic("edited", { updated: "2026-11-14T12:00" })), ALL_PROJECTS, today);
    expect(cols.archive.map((t) => t.slug)).toEqual(["created"]);
    expect(cols.done.map((t) => t.slug)).toEqual(["edited", "undated"]);
  });

  it("counts calendar days across daylight saving changes", () => {
    const cols = groupTopics(snapshot(topic("dst", { updated: "2026-10-24T23:59" })), ALL_PROJECTS, "2026-10-31");
    expect(cols.archive.map((t) => t.slug)).toEqual(["dst"]);
  });

  it("moves archived cards out optimistically and restores them on rollback", () => {
    const data = snapshot(topic("old"));
    const cols = groupTopics(data, ALL_PROJECTS, today, new Map([["whenubuy/old", "waiting"]]));
    expect(cols.archive).toHaveLength(0);
    expect(cols.waiting[0]?.status).toBe("waiting");
    expect(groupTopics(data, ALL_PROJECTS, today).archive[0]).toBe(data.topics[0]);
  });

  it("keeps a new optimistic completion in done despite an old source timestamp", () => {
    const data = snapshot(topic("stale", { status: "planned" }), topic("recent", { updated: "2026-11-14T12:00" }));
    const cols = groupTopics(data, ALL_PROJECTS, today, new Map([["whenubuy/stale", "done"]]));
    expect(cols.archive).toHaveLength(0);
    expect(cols.done.map((t) => t.slug)).toEqual(["stale", "recent"]);
    expect(data.topics[0]!.updated).toBe("2026-11-08T23:59");
    const archived = snapshot(topic("unchanged"));
    expect(groupTopics(archived, ALL_PROJECTS, today, new Map([["whenubuy/unchanged", "done"]])).archive).toHaveLength(1);
  });

  it("caps archive and done independently, newest first, keeping counts and scope", () => {
    const topics = Array.from({ length: DONE_CAP + 3 }, (_, i) => topic(`old-${i}`, { updated: `2026-11-08T${String(i % 24).padStart(2, "0")}:00` }));
    const recent = topics.map((t) => ({ ...t, slug: `new-${t.slug}`, updated: t.updated!.replace("11-08", "11-14") }));
    const cols = groupTopics(snapshot(...topics, ...recent, topic("outside", { project: "other" })), "whenubuy", today);
    for (const c of ["done", "archive"] as const) {
      const capped = capColumn(cols[c], true);
      expect(capped.shown).toHaveLength(DONE_CAP);
      expect(capped.hidden).toBe(3);
      expect(cols[c].map((t) => t.updated)).toEqual(cols[c].map((t) => t.updated).sort().reverse());
      expect(cols[c].every((t) => t.project === "whenubuy")).toBe(true);
    }
  });

  it("never offers archive as a move, or done for an already archived topic", () => {
    expect(topicMoveColumns("planned")).toContain("done");
    expect(topicMoveColumns("planned")).not.toContain("archive");
    expect(topicMoveColumns("archive")).toEqual(["planned", "in_progress", "blocked", "waiting"]);
    expect(adjacentColumn<string>([...topicMoveColumns("archive"), "archive"], "archive", -1)).toBe("waiting");
    expect(adjacentColumn<string>([...topicMoveColumns("archive"), "archive"], "archive", 1)).toBeNull();
  });
});

describe("task topic enclosures", () => {
  it("keeps tasks of an archived topic in the usual task lane and status columns", () => {
    const topic = { ...snap.topics[0]!, status: "done" as const, updated: "2026-10-01T12:00" };
    const location = { kind: "topic" as const, project: topic.project, topic: topic.slug };
    const tasks = ["todo", "done"].map((status, i) => ({ ...snap.tasks[0]!, id: `archived${i}`, location, status: status as TaskStatus }));
    const data = { ...snap, topics: [topic], tasks };
    expect(groupTopics(data, ALL_PROJECTS, "2026-11-15").archive).toEqual([topic]);
    const lanes = taskLanes(groupTasks(data, ALL_PROJECTS), data.topics, true);
    expect(lanes).toHaveLength(1);
    expect(lanes[0]!.topic).toBe(topic);
    expect(lanes[0]!.columns.todo).toEqual([tasks[0]]);
    expect(lanes[0]!.columns.done).toEqual([tasks[1]]);
  });

  it("keeps topic order when moving cards between status columns", () => {
    const before = taskLanes(groupTasks(snap, ALL_PROJECTS), snap.topics, true).map((l) => l.key);
    const task = snap.tasks.find((t) => t.location.kind === "topic" && t.status === "todo")!;
    const after = taskLanes(groupTasks(snap, ALL_PROJECTS, new Map([[task.id!, "done"]])), snap.topics, true).map((l) => l.key);
    expect(after).toEqual(before);
  });

  it("shows a pending topic change in its destination group and removes it from the source", () => {
    const task = snap.tasks.find((t) => t.location.kind === "topic")!;
    const target = snap.topics.find((t) => t.project !== (task.location.kind === "topic" ? task.location.project : ""))!;
    const location = { kind: "topic" as const, project: target.project, topic: target.slug };
    const cols = groupTasks(snap, ALL_PROJECTS, new Map([[task.id!, "doing"]]), new Map([[task.id!, location]]));
    const all = TASK_COLUMNS.flatMap((c) => cols[c]);
    expect(all.filter((t) => t.id === task.id)).toHaveLength(1);
    expect(all.find((t) => t.id === task.id)).toMatchObject({ location, status: "doing" });
    expect(TASK_COLUMNS.flatMap((c) => groupTasks(snap, location.project, new Map(), new Map([[task.id!, location]]))[c]).map((t) => t.id)).toContain(task.id);
  });

  it("keeps a topic in one lane across status columns, including a pending move", () => {
    const cols = groupTasks(snap, "whenubuy");
    const task = cols.todo.find((t) => t.location.kind === "topic")!;
    const location = task.location;
    const topic = location.kind === "topic" ? snap.topics.find((t) => t.project === location.project && t.slug === location.topic) : null;
    expect(topic).toBeTruthy();
    const moved = groupTasks(snap, "whenubuy", new Map([[task.id!, "doing" as const]]));
    const lanes = taskLanes(moved, snap.topics, true);
    const lane = lanes.find((l) => l.topic === topic)!;
    expect(lane.columns.doing.map((t) => t.id)).toContain(task.id);
    expect(lane.columns.todo.map((t) => t.id)).not.toContain(task.id);
    expect(lanes.filter((l) => l.topic === topic)).toHaveLength(1);
  });

  it("encloses only topics with at least two scoped tasks, never lone or general tasks", () => {
    const topic = snap.topics[0]!;
    const task = { ...snap.tasks[0]!, location: { kind: "topic" as const, project: topic.project, topic: topic.slug } };
    const general = { ...task, id: "other1", location: { kind: "project" as const, project: topic.project } };
    const one = { todo: [task, general], doing: [], blocked: [], done: [] };
    expect(taskLanes(one, [topic], false).map((l) => l.topic)).toEqual([null]);
    const two = { ...one, doing: [{ ...task, id: "other2", status: "doing" as const }] };
    const lanes = taskLanes(two, [topic], false);
    expect(lanes[0]!.topic).toBe(topic);
    expect(lanes[0]!.total).toBe(2);
    expect(lanes[1]!.columns.todo).toEqual([general]);
  });

  it("preserves every visible card exactly once and accounts for hidden done cards", () => {
    const cols = groupTasks(snap, ALL_PROJECTS);
    const expanded = taskLanes(cols, snap.topics, true);
    const collapsed = taskLanes(cols, snap.topics, false);
    const cards = (lanes: ReturnType<typeof taskLanes>) => lanes.flatMap((l) => TASK_COLUMNS.flatMap((c) => l.columns[c]));
    expect(new Set(cards(expanded)).size).toBe(cards(expanded).length);
    expect(cards(expanded)).toHaveLength(TASK_COLUMNS.flatMap((c) => cols[c]).length);
    expect(cards(collapsed)).toHaveLength(cols.todo.length + cols.doing.length + cols.blocked.length);
    expect(cards(collapsed).some((t) => t.status === "done")).toBe(false);
    for (const lane of collapsed) expect(lane.total).toBe(TASK_COLUMNS.flatMap((c) => lane.columns[c]).length + lane.hiddenDone);
  });

  it("never combines identical topic slugs from different projects", () => {
    const topic = snap.topics[0]!;
    const other = { ...topic, project: "other-project" };
    const tasks = [topic, other].flatMap((t, i) => [0, 1].map((n) => ({ ...snap.tasks[0]!, id: `task${i}${n}`, status: "todo" as const, location: { kind: "topic" as const, project: t.project, topic: t.slug } })));
    const lanes = taskLanes({ todo: tasks, doing: [], blocked: [], done: [] }, [topic, other], false);
    expect(lanes).toHaveLength(2);
    expect(lanes.map((l) => l.total)).toEqual([2, 2]);
    expect(lanes.map((l) => l.key)).toEqual([`${topic.project}/${topic.slug}`, `${other.project}/${other.slug}`]);
  });

  it("caps completed cards globally while keeping them in the shared topic enclosure", () => {
    const topic = snap.topics[0]!;
    const task = { ...snap.tasks[0]!, status: "todo" as const, location: { kind: "topic" as const, project: topic.project, topic: topic.slug } };
    const done = Array.from({ length: DONE_CAP + 5 }, (_, n) => ({ ...task, id: `done${n}`, status: "done" as const }));
    const lanes = taskLanes({ todo: [task], doing: [], blocked: [], done }, [topic], true);
    expect(lanes).toHaveLength(1);
    expect(lanes[0]).toMatchObject({ topic, total: DONE_CAP + 6, hiddenDone: 5 });
    expect(lanes[0]!.columns.done).toHaveLength(DONE_CAP);
  });
});

// F4: each optimistic move has its own operation id; only that operation's
// completion clears it, and moves of one card are sent in order.
describe("optimistic board moves", () => {
  it("completion of an earlier move keeps a newer pending one", () => {
    let m: PendingMoves<TaskStatus> = beginMove(new Map(), "t1", 1, "doing");
    m = beginMove(m, "t1", 2, "blocked");
    m = settleMove(m, "t1", 1); // the "doing" request settles first
    expect(moveOverrides(m).get("t1")).toBe("blocked");
    m = settleMove(m, "t1", 2);
    expect(moveOverrides(m).has("t1")).toBe(false);
  });

  it("failure of an earlier move does not clobber a later one; other cards are untouched", () => {
    let m: PendingMoves<TaskStatus> = beginMove(new Map(), "t1", 1, "doing");
    m = beginMove(m, "t2", 2, "done");
    m = beginMove(m, "t1", 3, "blocked");
    const after = settleMove(m, "t1", 1); // failed or not: it is not the newest for t1
    expect(after).toBe(m);
    expect([...moveOverrides(after)]).toEqual([
      ["t1", "blocked"],
      ["t2", "done"],
    ]);
  });

  it("feeds the grouping like a plain override map", () => {
    const t = groupTasks(snap, ALL_PROJECTS).todo[0]!;
    const m = beginMove(beginMove<TaskStatus>(new Map(), t.id!, 1, "doing"), t.id!, 2, "blocked");
    const cols = groupTasks(snap, ALL_PROJECTS, moveOverrides(settleMove(m, t.id!, 1)));
    expect(ids(cols.blocked)).toContain(t.id);
    expect(ids(cols.doing)).not.toContain(t.id);
  });

  it("runs the moves of one card in order, others in parallel, and survives a failure", async () => {
    const queue = createSerialQueue();
    const log: string[] = [];
    let release!: () => void;
    const first = queue("t1", async () => {
      log.push("doing:start");
      await new Promise<void>((r) => (release = r));
      log.push("doing:end");
      throw new Error("conflict");
    });
    const second = queue("t1", async () => void log.push("blocked"));
    const other = queue("t2", async () => void log.push("t2"));
    await other;
    expect(log).toEqual(["doing:start", "t2"]); // t1's second move waits
    release();
    await expect(first).rejects.toThrow("conflict");
    await second;
    expect(log).toEqual(["doing:start", "t2", "doing:end", "blocked"]);
  });
});

describe("palette items", () => {
  it("lists navigation targets for an empty query", () => {
    const items = buildItems(snap, "  ", []);
    expect(items.map((i) => i.group)).toEqual(items.map(() => "nav"));
    expect(items.slice(0, 3).map((i) => (i.group === "nav" ? i.label : ""))).toEqual(["Dziś", "Skrzynka", "Tablica"]);
  });

  it("groups hits as tasks, topics, journal and limits each group", () => {
    const hits = [
      ...snap.journal.slice(0, 2).map((entry) => ({ kind: "journal" as const, entry })),
      ...snap.topics.map((topic) => ({ kind: "topic" as const, topic })),
      ...snap.tasks.map((task) => ({ kind: "task" as const, task })),
    ];
    const items = buildItems(snap, "x", hits);
    const groups = items.map((i) => i.group);
    expect(groups.indexOf("task")).toBeLessThan(groups.indexOf("topic"));
    expect(groups.indexOf("topic")).toBeLessThan(groups.indexOf("journal"));
    expect(groups.filter((g) => g === "task").length).toBe(8);
  });
});
