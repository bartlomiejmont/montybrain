import { describe, expect, it } from "vitest";

import { buildFixtures } from "../api/fixtures";
import type { Snapshot, Task, TaskStatus } from "../api/types";
import {
  ALL_PROJECTS,
  DONE_CAP,
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
    const cols = groupTopics(snap, "whenubuy");
    expect(cols.in_progress.map((t) => t.slug)).toContain("ab-testy-cen");
    expect(cols.done.map((t) => t.slug)).toEqual(["onboarding"]);
    const moved = groupTopics(snap, "whenubuy", new Map([["whenubuy/ab-testy-cen", "waiting" as const]]));
    expect(moved.waiting.map((t) => t.slug)).toContain("ab-testy-cen");
  });

  it("finds neighbouring columns for keyboard moves", () => {
    expect(adjacentColumn(TASK_COLUMNS, "todo", 1)).toBe("doing");
    expect(adjacentColumn(TASK_COLUMNS, "todo", -1)).toBeNull();
    expect(adjacentColumn(TASK_COLUMNS, "done", 1)).toBeNull();
    expect(firstLine("\n  pierwsza\ndruga")).toBe("pierwsza");
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
