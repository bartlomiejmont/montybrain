import { describe, expect, it } from "vitest";

import { parseHash, matchPattern, href, type ViewDef } from "../router/registry";
import { createMockApi } from "./mock";

// Friday 2026-11-13, 14:02 local time.
const now = () => new Date(2026, 10, 13, 14, 2);

describe("mock api", () => {
  it("serves fixtures with today computed like core", async () => {
    const api = createMockApi({ now });
    const view = await api.getState();
    expect(view.today).toBe("2026-11-13");
    expect(view.now).toBe("2026-11-13T14:02");
    const reasons = view.today_items.map((i) => i.reason);
    // sorted by reason
    expect([...reasons].sort((a, b) => order(a) - order(b))).toEqual(reasons);
    expect(reasons).toContain("overdue");
    expect(reasons).toContain("due_today");
    // archived and done tasks are excluded
    expect(view.today_items.every((i) => i.task.status !== "done")).toBe(true);
    // recent journal: newest first
    const ats = view.recent_journal.map((j) => j.at);
    expect([...ats].sort().reverse()).toEqual(ats);
  });

  it("creates, completes, moves and deletes a task", async () => {
    const api = createMockApi({ now });
    const events: number[] = [];
    await api.onChange((e) => events.push(e.version));

    const t = await api.createTask({ kind: "inbox" }, { title: "  Nowe   zadanie ", priority: "high" });
    expect(t.title).toBe("Nowe zadanie");
    expect(t.id).toMatch(/^[0-9a-z]{6}$/);

    const done = await api.updateTask(t.id!, { status: "done" });
    expect(done.done).toBe("2026-11-13");
    const reopened = await api.updateTask(t.id!, { status: "doing", add_links: ["https://a.pl", "https://a.pl"] });
    expect(reopened.done).toBeNull();
    expect(reopened.links).toEqual(["https://a.pl"]);

    const moved = await api.moveTask(`^${t.id}`, { kind: "topic", project: "whenubuy", topic: "ab-testy-cen" });
    expect(moved.id).toBe(t.id);
    const state = await api.getState();
    const inState = state.snapshot.tasks.find((x) => x.id === t.id)!;
    expect(inState.location).toEqual({ kind: "topic", project: "whenubuy", topic: "ab-testy-cen" });
    // file order: inbox tasks come before project tasks
    const firstNonInbox = state.snapshot.tasks.findIndex((x) => x.location.kind !== "inbox");
    expect(state.snapshot.tasks.slice(firstNonInbox).every((x) => x.location.kind !== "inbox")).toBe(true);
    // topic got touched
    expect(state.snapshot.topics.find((x) => x.slug === "ab-testy-cen")!.updated).toBe("2026-11-13T14:02");

    await api.deleteTask(t.id!);
    expect((await api.getState()).snapshot.tasks.some((x) => x.id === t.id)).toBe(false);

    await new Promise((r) => setTimeout(r, 0));
    expect(events.length).toBe(5);
    expect(events).toEqual([...events].sort((a, b) => a - b));
  });

  it("rejects what core rejects", async () => {
    const api = createMockApi({ now });
    await expect(api.createTask({ kind: "inbox" }, { title: "   " })).rejects.toThrow(/invalid input/);
    await expect(api.createTask({ kind: "inbox" }, { title: "Spotkanie @2026-11-20" })).rejects.toThrow(/invalid input/);
    await expect(api.createTask({ kind: "project", project: "nie-ma" }, { title: "x" })).rejects.toThrow(/project not found/);
    await expect(api.updateTask("zzzzzz", { title: "x" })).rejects.toThrow(/task not found/);
    await expect(api.createTopic("whenubuy", { title: "T", repo: "obce-repo" })).rejects.toThrow(/unknown repo/);
  });

  it("handles projects, topics and the journal", async () => {
    const api = createMockApi({ now });
    const p = await api.createProject({ name: "Morwisko" });
    expect(p.slug).toBe("morwisko-2"); // fixtures already have "morwisko"
    const t = await api.createTopic(p.slug, { title: "Zażółć gęślą jaźń" });
    expect(t.slug).toBe("zazolc-gesla-jazn");
    expect(t.status).toBe("planned");

    const added = await api.addJournalEntry(p.slug, t.slug, { text: "Start", status: "in_progress" });
    expect(added.entry.author).toBe("ja");
    expect(added.topic.status).toBe("in_progress");

    const cleared = await api.updateProject(p.slug, { color: "#123456" }).then(() => api.updateProject(p.slug, { color: null }));
    expect(cleared.color).toBeNull();
    expect((await api.archiveProject(p.slug)).state).toBe("archived");

    await api.createTask({ kind: "topic", project: p.slug, topic: t.slug }, { title: "Zadanie", due: "2026-11-01" });
    const view = await api.getState();
    // tasks of archived projects are not in "today"
    expect(view.today_items.some((i) => i.task.title === "Zadanie")).toBe(false);

    await api.deleteTopic(p.slug, t.slug);
    const after = await api.getState();
    expect(after.snapshot.topics.some((x) => x.slug === t.slug)).toBe(false);
    expect(after.snapshot.journal.some((j) => j.topic === t.slug)).toBe(false);
  });

  it("searches without case or diacritics", async () => {
    const api = createMockApi({ now });
    const hits = await api.search("PLATNOSCI stripe");
    expect(hits.map((h) => h.kind)).toEqual(["topic"]);
    expect(await api.search("   ")).toEqual([]);
  });
});

describe("router registry", () => {
  const Dummy = () => null;
  const defs: ViewDef[] = [
    { id: "today", pattern: "dzis", component: Dummy },
    { id: "project", pattern: "p/:project", component: Dummy },
  ];

  it("matches patterns and decodes params", () => {
    expect(matchPattern("p/:project", "p/when%20u")).toEqual({ project: "when u" });
    expect(matchPattern("p/:project", "p")).toBeNull();
    expect(matchPattern("dzis", "dzis")).toEqual({});
  });

  it("parses hashes with query", () => {
    const r = parseHash("#/p/whenubuy?temat=ab-testy-cen", defs);
    expect(r.view).toBe("project");
    expect(r.params.project).toBe("whenubuy");
    expect(r.query.get("temat")).toBe("ab-testy-cen");
    expect(parseHash("#/nieznane", defs).view).toBeNull();
    expect(parseHash("", defs).view).toBeNull();
  });

  it("builds hrefs", () => {
    expect(href("p/:project", { project: "a b" }, { temat: "x", other: undefined })).toBe("#/p/a%20b?temat=x");
  });
});

function order(r: string): number {
  return ["overdue", "due_today", "doing", "blocked"].indexOf(r);
}
