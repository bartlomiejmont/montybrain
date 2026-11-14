import { describe, expect, it } from "vitest";

import type { Snapshot, Task, TaskLocation, Topic } from "../api/types";
import { addDays, formatAge, formatDue, formatStamp, isOverdue } from "./dates";
import { countTasks, groupToday, journalOf, moveTargets, sortTopics, splitDone, taskDraftKey } from "./group";
import { parseQuickAdd } from "./quickadd";
import { fold, linkLabel, plural, slugify, uniqueSlug } from "./text";

const TODAY = "2026-11-13"; // a Friday

function task(id: string, location: TaskLocation, extra: Partial<Task> = {}): Task {
  return {
    id,
    title: id,
    status: "todo",
    priority: "normal",
    due: null,
    done: null,
    links: [],
    notes: "",
    location,
    line: 1,
    ...extra,
  };
}

function topic(project: string, slug: string, extra: Partial<Topic> = {}): Topic {
  return {
    project,
    slug,
    title: slug,
    status: "planned",
    repo: null,
    summary: "",
    created: null,
    updated: null,
    path: `${project}/tematy/${slug}.md`,
    ...extra,
  };
}

describe("text", () => {
  it("folds Polish diacritics", () => {
    expect(fold("Zażółć GĘŚLĄ jaźń Łódź")).toBe("zazolc gesla jazn lodz");
  });
  it("slugifies like core", () => {
    expect(slugify("Płatności: Stripe → Przelewy24")).toBe("platnosci-stripe-przelewy24");
    expect(slugify("  ***  ")).toBe("bez-nazwy");
    expect(slugify("a".repeat(30) + " " + "b".repeat(40)).length).toBeLessThanOrEqual(60);
    expect(uniqueSlug("x", (s) => s === "x" || s === "x-2")).toBe("x-3");
  });
  it("labels links", () => {
    expect(linkLabel("https://gitlab.com/g/r/-/merge_requests/42")).toBe("gitlab.com !42");
    expect(linkLabel("https://github.com/o/r/pull/7")).toBe("github.com #7");
    expect(linkLabel("https://www.example.com/")).toBe("example.com");
  });
  it("pluralizes in Polish", () => {
    expect([1, 2, 5, 12, 22, 25].map((n) => plural(n, "zadanie", "zadania", "zadań"))).toEqual([
      "zadanie",
      "zadania",
      "zadań",
      "zadań",
      "zadania",
      "zadań",
    ]);
  });
});

describe("dates", () => {
  it("formats due dates relative to today", () => {
    expect(formatDue(TODAY, TODAY)).toBe("dziś");
    expect(formatDue(`${addDays(TODAY, 1)}T09:30`, TODAY)).toBe("jutro 09:30");
    expect(formatDue(addDays(TODAY, -1), TODAY)).toBe("wczoraj");
    expect(formatDue("2026-12-24", TODAY)).toMatch(/^24 gru/);
    expect(formatDue("2027-01-02", TODAY)).toMatch(/2027/);
  });
  it("detects overdue by date only (as core does)", () => {
    expect(isOverdue("2026-11-12T23:59", TODAY)).toBe(true);
    expect(isOverdue("2026-11-13T00:01", TODAY)).toBe(false);
    expect(isOverdue(null, TODAY)).toBe(false);
  });
  it("formats stamps and ages", () => {
    expect(formatStamp("2026-11-13T14:02", TODAY)).toBe("14:02");
    expect(formatStamp("2026-11-12T09:30", TODAY)).toBe("wczoraj 09:30");
    expect(formatAge("2026-11-10T10:00", TODAY)).toBe("3 dni temu");
    expect(addDays("2026-12-31", 1)).toBe("2027-01-01");
  });
});

describe("quick add", () => {
  it("parses trailing priority and due tokens", () => {
    expect(parseQuickAdd("Zrobić MR !wysoki @jutro", TODAY)).toEqual({
      title: "Zrobić MR",
      priority: "high",
      due: "2026-11-14",
    });
    expect(parseQuickAdd("Spotkanie @2026-11-20T14:00", TODAY)).toEqual({ title: "Spotkanie", due: "2026-11-20T14:00" });
    expect(parseQuickAdd("Coś @dziś !niski", TODAY)).toEqual({ title: "Coś", priority: "low", due: TODAY });
  });
  it("leaves non-tokens and lone tokens in the title", () => {
    expect(parseQuickAdd("Napraw @mention parsing", TODAY)).toEqual({ title: "Napraw @mention parsing" });
    expect(parseQuickAdd("Data @2026-02-30", TODAY)).toEqual({ title: "Data @2026-02-30" });
    expect(parseQuickAdd("!wysoki", TODAY)).toEqual({ title: "!wysoki" });
    expect(parseQuickAdd("Dwa @jutro @dziś", TODAY)).toEqual({ title: "Dwa @jutro", due: TODAY });
  });
});

describe("group", () => {
  const inbox: TaskLocation = { kind: "inbox" };
  const tp: TaskLocation = { kind: "topic", project: "p", topic: "t" };
  const snap: Snapshot = {
    root: "/v",
    projects: [
      { slug: "p", name: "Projekt", color: null, state: "active", repos: [], description: "", path: "p/projekt.md" },
      { slug: "old", name: "Stary", color: null, state: "archived", repos: [], description: "", path: "old/projekt.md" },
    ],
    topics: [
      topic("p", "t", { status: "in_progress" }),
      topic("p", "done-topic", { status: "done" }),
      topic("old", "x"),
    ],
    tasks: [
      task("a", inbox),
      task("b", inbox, { status: "done", done: "2026-11-01" }),
      task("c", tp),
      task("d", tp, { status: "done", done: "2026-11-10" }),
      task("e", { kind: "project", project: "p" }),
    ],
    journal: [
      { project: "p", topic: "t", at: "2026-11-12T10:00", author: "ja", text: "starszy", line: 9 },
      { project: "p", topic: "t", at: "2026-11-13T10:00", author: "ja", text: "nowszy", line: 8 },
    ],
    warnings: [],
  };

  it("counts open/total per container", () => {
    const c = countTasks(snap);
    expect(c.inbox).toEqual({ open: 1, total: 2 });
    expect(c.project.get("p")).toEqual({ open: 2, total: 3 });
    expect(c.topic.get("p/t")).toEqual({ open: 1, total: 2 });
  });

  it("splits done tasks, newest completion first", () => {
    const { open, done } = splitDone(snap.tasks);
    expect(open.map((t) => t.id)).toEqual(["a", "c", "e"]);
    expect(done.map((t) => t.id)).toEqual(["d", "b"]);
  });

  it("sorts topics by activity", () => {
    const sorted = sortTopics([
      topic("p", "plan", { status: "planned", updated: "2026-11-13T10:00" }),
      topic("p", "done", { status: "done" }),
      topic("p", "old-work", { status: "in_progress", updated: "2026-11-01T10:00" }),
      topic("p", "new-work", { status: "in_progress", updated: "2026-11-12T10:00" }),
      topic("p", "stuck", { status: "blocked" }),
    ]);
    expect(sorted.map((t) => t.slug)).toEqual(["new-work", "old-work", "stuck", "plan", "done"]);
  });

  it("buckets today items in reason order and drops empty sections", () => {
    const sections = groupToday([
      { reason: "doing", task: task("x", inbox) },
      { reason: "overdue", task: task("y", inbox) },
      { reason: "doing", task: task("z", inbox) },
    ]);
    expect(sections.map((s) => [s.reason, s.items.map((i) => i.task.id)])).toEqual([
      ["overdue", ["y"]],
      ["doing", ["x", "z"]],
    ]);
  });

  it("lists move targets without archived projects or done topics", () => {
    expect(moveTargets(snap).map((t) => t.key)).toEqual(["inbox", "p:p", "t:p/t"]);
    // ...unless the task already lives there
    expect(moveTargets(snap, { kind: "topic", project: "old", topic: "x" }).map((t) => t.key)).toContain("t:old/x");
  });

  it("orders a topic journal newest first", () => {
    expect(journalOf(snap, "p", "t").map((j) => j.text)).toEqual(["nowszy", "starszy"]);
  });
});

// N3: drafts of a task's fields (title, notes, deadline, the link being typed)
// are keyed by the task, not by the component instance or list position.
describe("task draft key", () => {
  const inTopic: TaskLocation = { kind: "topic", project: "p", topic: "a" };
  const elsewhere: TaskLocation = { kind: "topic", project: "p", topic: "b" };

  it("stays the same when the task moves to another list", () => {
    expect(taskDraftKey(task("x1", inTopic, { line: 3 }))).toBe(taskDraftKey(task("x1", elsewhere, { line: 9 })));
  });

  it("tells apart tasks without an id at the same line of different files", () => {
    const a = task("", inTopic, { id: null, line: 3 });
    const b = task("", elsewhere, { id: null, line: 3 });
    expect(taskDraftKey(a)).not.toBe(taskDraftKey(b));
    expect(taskDraftKey(a)).not.toBe(taskDraftKey(task("", { kind: "project", project: "p" }, { id: null, line: 3 })));
  });
});
