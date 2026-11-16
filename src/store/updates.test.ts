import { describe, expect, it, vi } from "vitest";

import { createMockApi } from "../api/mock";
import { createRunStore } from "./runs";
import { createUpdateStore } from "./updates";

const runReq = { recipe: "aktualizacja", provider: "codex" as const, mode: "background" as const, params: { project: "whenubuy" } };

describe("project updates with the mock backend", () => {
  it("forces a new CLI check even when a cached check is already in flight", async () => {
    const api = createMockApi();
    const providers = await api.listProviders();
    let resolve!: (value: typeof providers) => void;
    const first = new Promise<typeof providers>((done) => { resolve = done; });
    const listProviders = vi.fn().mockReturnValueOnce(first).mockResolvedValue(providers);
    const store = createUpdateStore({ ...api, listProviders });
    const loading = store.loadProviders();
    const refreshing = store.loadProviders(true);
    resolve(providers);
    await Promise.all([loading, refreshing]);
    expect(listProviders.mock.calls).toEqual([[false], [true]]);
    await store.loadProviders();
    expect(listProviders).toHaveBeenCalledTimes(2);
  });

  it("a finished run becomes a proposal; applying writes once and skips stale changes", async () => {
    vi.useFakeTimers();
    const api = createMockApi({ runs: { stepMs: 5 } });
    const runs = createRunStore(api);
    const updates = createUpdateStore(api);
    await runs.start();
    await updates.start();
    expect(updates.getSnapshot().byProject.whenubuy).toBeUndefined();

    const run = await runs.startRun(runReq);
    expect(run.project).toBe("whenubuy");
    await vi.advanceTimersByTimeAsync(200);
    const view = updates.getSnapshot().byProject.whenubuy!;
    expect(view.last?.outcome).toEqual({ kind: "proposed", count: view.proposal!.changes.length });
    expect(view.pending).toBe(view.proposal!.changes.length);
    expect(view.proposal!.run_id).toBe(run.id);
    // Topics first: the new topic leads the mock proposal; the task changes follow.
    expect(view.proposal!.changes[0]!.change.kind).toBe("new_topic");
    const [a, b] = view.proposal!.changes.flatMap((c, i) => (c.change.kind === "task_status" ? [i] : []));
    const first = view.proposal!.changes[a!];
    const second = view.proposal!.changes[b!];

    // Meanwhile the second task changes by hand: its change is stale.
    if (second!.change.kind !== "task_status") throw new Error("fixture");
    await api.updateTask(second!.change.task, { status: "blocked" });

    const report = await api.applyUpdate(run.id, [a!, b!, a!]);
    expect(report.items.map((i) => [i.index, i.status])).toEqual([
      [a, "applied"],
      [b, "stale"],
    ]);
    updates.set(report.updates);
    if (first!.change.kind !== "task_status") throw new Error("fixture");
    const id = first!.change.task;
    expect(api.snapshotForTests().tasks.find((t) => t.id === id)?.status).toBe("done");
    expect(updates.getSnapshot().byProject.whenubuy!.last?.outcome).toEqual({ kind: "applied", count: 1 });

    // At most once.
    const again = await api.applyUpdate(run.id, [a!]);
    expect(again.items[0]!.status).toBe("already");
    expect(again.undo).toBeNull();

    // Undo of that click: the task goes back, the change can be decided again; once only.
    expect(report.undo).toBeTruthy();
    // The view carries the token too, so the strip can offer undo after the toast is gone.
    expect(report.updates.find((v) => v.project === "whenubuy")!.undo).toBe(report.undo);
    const undone = await api.undoUpdate(run.id, report.undo!);
    expect(undone.updates.find((v) => v.project === "whenubuy")!.undo).toBeNull();
    expect(undone.items).toEqual([{ index: a, status: "undone", message: "Przywrócono poprzedni status." }]);
    expect(api.snapshotForTests().tasks.find((t) => t.id === id)?.status).toBe(first!.change.from);
    expect(undone.updates.find((v) => v.project === "whenubuy")!.proposal!.changes[a!]!.outcome).toBeNull();
    await expect(api.undoUpdate(run.id, report.undo!)).rejects.toThrow(/nie można już cofnąć/);

    // Applied again, then changed by hand: the undo leaves it alone.
    const again2 = await api.applyUpdate(run.id, [a!]);
    await api.updateTask(id, { status: "blocked" });
    const left = await api.undoUpdate(run.id, again2.undo!);
    expect(left.items[0]!.status).toBe("changed");
    expect(api.snapshotForTests().tasks.find((t) => t.id === id)?.status).toBe("blocked");

    // Rejecting the rest keeps the proposal as the last result (receipt), nothing pending.
    const before = api.snapshotForTests().topics.find((t) => t.project === "whenubuy" && t.slug === "platnosci-stripe-przelewy24")!;
    const pendingNow = updates.getSnapshot().byProject.whenubuy ? (await api.listUpdates()).find((v) => v.project === "whenubuy")!.pending : 0;
    // Correct one undecided row first: the rejection must keep the correction.
    const pv = (await api.listUpdates()).find((v) => v.project === "whenubuy")!;
    const ni = pv.proposal!.changes.findIndex((c) => !c.outcome && c.change.kind === "new_task");
    await api.editUpdate(run.id, ni, { kind: "new_task", title: "Zwroty w README (poprawione)" });
    const after = await api.dismissUpdate(run.id);
    expect(after.count).toBe(pendingNow);
    const v2 = after.updates.find((v) => v.project === "whenubuy")!;
    expect(v2.pending).toBe(0);
    expect(v2.proposal!.changes.some((c) => c.outcome?.status === "rejected")).toBe(true);
    expect([v2.undo, v2.undo_rejection]).toEqual([after.undo, true]);
    // Undoing the rejection: back under review, the correction intact; once only.
    const back = await api.undoUpdate(run.id, after.undo!);
    const v3 = back.updates.find((v) => v.project === "whenubuy")!;
    expect(v3.pending).toBe(pendingNow);
    expect(v3.proposal!.changes[ni]!.edited).toBe(true);
    expect(v3.proposal!.changes[ni]!.change).toMatchObject({ title: "Zwroty w README (poprawione)" });
    await expect(api.undoUpdate(run.id, after.undo!)).rejects.toThrow(/nie można już cofnąć/);
    expect(api.snapshotForTests().topics.find((t) => t.slug === before.slug)!.updated).toBe(before.updated);
    vi.useRealTimers();
  });

  it("apply then undo puts the topic's date back", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 10, 14, 15, 0));
    const api = createMockApi({ updateScenario: "proposals" });
    const v = (await api.listUpdates()).find((x) => x.project === "whenubuy")!;
    const i = v.proposal!.changes.findIndex((c) => c.change.kind === "topic_log");
    const log = v.proposal!.changes[i]!.change;
    if (log.kind !== "topic_log") throw new Error("fixture");
    const before = api.snapshotForTests().topics.find((t) => t.slug === log.topic)!.updated;
    const r = await api.applyUpdate(v.proposal!.run_id, [i]);
    expect(api.snapshotForTests().topics.find((t) => t.slug === log.topic)!.updated).not.toBe(before);
    await api.undoUpdate(v.proposal!.run_id, r.undo!);
    expect(api.snapshotForTests().topics.find((t) => t.slug === log.topic)!.updated).toBe(before);
    vi.useRealTimers();
  });

  it("an empty project gets new topics; a new topic is written whole and undone while untouched", async () => {
    vi.useFakeTimers();
    const api = createMockApi({ updateScenario: "topics" });
    const v = (await api.listUpdates()).find((x) => x.project === "beeking")!;
    const p = v.proposal!;
    const kinds = p.changes.map((c) => c.change.kind);
    expect(kinds.filter((k) => k === "new_topic").length).toBeGreaterThanOrEqual(3);
    expect(kinds.filter((k) => k === "new_task").length).toBeLessThanOrEqual(1);
    expect(api.snapshotForTests().topics.filter((t) => t.project === "beeking")).toEqual([]);

    const r = await api.applyUpdate(p.run_id, [0]);
    expect(r.items[0]!.status).toBe("applied");
    const snap = api.snapshotForTests();
    const topic = snap.topics.find((t) => t.project === "beeking")!;
    expect(topic.title).toBe("Aplikacja kasowa w trybie offline");
    expect(snap.tasks.filter((t) => t.location.kind === "topic" && t.location.topic === topic.slug).map((t) => t.status)).toEqual(["doing", "todo"]);
    expect(snap.journal.find((j) => j.topic === topic.slug)?.author).toBe("codex");
    const u = await api.undoUpdate(p.run_id, r.undo!);
    expect(u.items[0]!.status).toBe("undone");
    expect(api.snapshotForTests().topics.some((t) => t.slug === topic.slug)).toBe(false);

    // Applied again and touched by hand: left alone. A topic that appeared meanwhile: stale.
    const r2 = await api.applyUpdate(p.run_id, [0]);
    const t2 = api.snapshotForTests().tasks.find((t) => t.title === "Testy na tablecie bez sieci")!;
    await api.updateTask(t2.id!, { status: "doing" });
    expect((await api.undoUpdate(p.run_id, r2.undo!)).items[0]!.status).toBe("changed");
    await api.createTopic("beeking", { title: "Integracja z hurtownią" });
    expect((await api.applyUpdate(p.run_id, [1])).items[0]!.status).toBe("stale");
    vi.useRealTimers();
  });

  it("history source: validated, listed in settings, and its note reaches the proposal", async () => {
    vi.useFakeTimers();
    const api = createMockApi({ runs: { stepMs: 5 } });
    await expect(api.setUpdateSources("whenubuy", { history: true, history_days: 10 })).rejects.toThrow(/okno historii/);
    const list = await api.setUpdateSources("whenubuy", { history: true, history_days: 30 });
    expect(list.find((v) => v.project === "whenubuy")!.sources).toEqual({ history: true, history_days: 30 });
    expect((await api.getSettings()).update_sources.whenubuy).toEqual({ history: true, history_days: 30 });
    const runs = createRunStore(api);
    await runs.start();
    const run = await runs.startRun(runReq);
    await vi.advanceTimersByTimeAsync(200);
    const v = (await api.listUpdates()).find((x) => x.project === "whenubuy")!;
    expect(v.proposal!.history_note).toMatch(/^Zebrano 12 sesji/);
    expect(v.last!.history_note).toMatch(/^Zebrano/);
    expect(v.proposal!.changes[0]!.change).toMatchObject({ kind: "new_topic", status: "planned" });
    expect(run.model).toBeNull();
    // Back to the default: removed from settings.
    await api.setUpdateSources("whenubuy", { history: false, history_days: 14 });
    expect((await api.getSettings()).update_sources).toEqual({});
    vi.useRealTimers();
  });

  it("agents: model and effort are validated and requested on runs", async () => {
    const api = createMockApi({ runs: { stepMs: 5 } });
    await expect(api.updateSettings({ agents: { claude: { model: "--yolo", effort: null }, codex: { model: null, effort: null } } })).rejects.toThrow(/modelu/);
    await expect(api.updateSettings({ agents: { claude: { model: null, effort: "ultra" }, codex: { model: null, effort: null } } })).rejects.toThrow(/wysiłku/);
    const s = await api.updateSettings({ agents: { claude: { model: " opus ", effort: "high" }, codex: { model: null, effort: "ultra" } } });
    expect(s.agents).toEqual({ claude: { model: "opus", effort: "high" }, codex: { model: null, effort: "ultra" } });
    const r = await api.startRun({ recipe: "raport", provider: "claude", mode: "terminal", params: { period: "dzis", project: "" } });
    expect([r.model, r.effort]).toEqual(["opus", "high"]);
  });

  it("validates and stores the automatic update setting; settings show it", async () => {
    const api = createMockApi();
    await expect(api.setAutoUpdate("whenubuy", { schedule: { kind: "every", hours: 0 }, provider: "claude", on_result: "propose" })).rejects.toThrow(/odstęp/);
    await expect(api.setAutoUpdate("nie-ma", { schedule: { kind: "daily", hour: 9 }, provider: "claude", on_result: "propose" })).rejects.toThrow(/projektu/);
    const list = await api.setAutoUpdate("whenubuy", { schedule: { kind: "every", hours: 6 }, provider: "codex", on_result: "apply" });
    const v = list.find((x) => x.project === "whenubuy")!;
    expect(v.auto).toEqual({ schedule: { kind: "every", hours: 6 }, provider: "codex", on_result: "apply" });
    expect(v.next_due).not.toBeNull();
    expect((await api.getSettings()).auto_update.whenubuy?.provider).toBe("codex");
    // Back to the default: removed.
    await api.setAutoUpdate("whenubuy", { schedule: { kind: "off" }, provider: "claude", on_result: "propose" });
    expect((await api.getSettings()).auto_update).toEqual({});
  });

  it("seeds every screenshot scenario", async () => {
    const pending = createMockApi({ updateScenario: "pending" });
    const v = (await pending.listUpdates()).find((x) => x.project === "whenubuy")!;
    expect(v.auto.schedule).toEqual({ kind: "every", hours: 12 });
    expect(v.pending).toBeGreaterThan(0);
    expect(v.proposal?.trigger).toBe("auto");

    const hist = (await createMockApi({ updateScenario: "history" }).listUpdates()).find((x) => x.project === "whenubuy")!;
    expect(hist.sources.history).toBe(true);
    expect(hist.proposal!.history_note).toMatch(/Zebrano/);
    expect(hist.proposal!.changes[0]!.change).toMatchObject({ kind: "new_topic", status: "planned" });
    const running = createMockApi({ updateScenario: "running" });
    const r = (await running.listRuns()).find((x) => x.recipe === "aktualizacja");
    expect(r?.status).toBe("running");
    expect(r?.events.length).toBeGreaterThan(0);

    const applied = createMockApi({ updateScenario: "applied" });
    const partial = (await createMockApi({ updateScenario: "partial" }).listUpdates())[0]!;
    expect(partial.pending).toBe(1);
    const receipt = (await createMockApi({ updateScenario: "receipt" }).listUpdates())[0]!;
    expect(receipt.pending).toBe(0);
    expect(receipt.proposal!.changes.length).toBeGreaterThan(0);
    const held = (await createMockApi({ updateScenario: "pending" }).listUpdates())[0]!;
    expect(held.held && held.next_due === null).toBe(true);
    const a = (await applied.listUpdates())[0]!;
    expect(a.last?.outcome.kind).toBe("applied");
    expect(a.proposal?.changes.some((c) => c.outcome?.status === "stale")).toBe(true);

    for (const [scenario, kind] of [
      ["nochanges", "no_changes"],
      ["error", "failed"],
      ["auto", "unchanged"],
    ] as const) {
      const api = createMockApi({ updateScenario: scenario });
      expect((await api.listUpdates())[0]?.last?.outcome.kind).toBe(kind);
    }
    const none = createMockApi({ updateScenario: "unavailable", runs: { codex: "missing", claude: "missing" } });
    expect((await none.listProviders()).every((p) => !p.available)).toBe(true);
    expect(await createMockApi().listUpdates()).toEqual([]);
  });
});
