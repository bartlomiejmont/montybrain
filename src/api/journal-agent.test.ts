import { afterEach, describe, expect, it, vi } from "vitest";
import { createMockApi } from "./mock";
import { newJournalEntries } from "../lib/journalAgent";
import { defaultSelection } from "../lib/updates";

const project = "whenubuy";
const topic = "platnosci-stripe-przelewy24";
const req = () => ({ recipe: "zlecenie", provider: "codex" as const, mode: "background" as const, params: { project, topic } });
afterEach(() => vi.useRealTimers());

describe("journal refresh shared proposal", () => {
  it("adds entries independently, refreshes saved entries, blocks replacement, applies and undoes", async () => {
    vi.useFakeTimers();
    const api = createMockApi({ runs: { stepMs: 5 } });
    const text = "  Etap gotowy.\nWebhook zbędny.  ";
    const saved = await api.addJournalEntry(project, topic, { text, author: "ja" });
    expect(saved.entry.text).toBe(text.trim()); // existing journal serialization
    expect(await api.listRuns()).toEqual([]); // plain save never starts an agent
    const before = api.snapshotForTests();
    const run = await api.startRun(req());
    expect(run.topic).toBe(topic);
    await vi.advanceTimersByTimeAsync(200);
    let view = (await api.listUpdates()).find((v) => v.project === project)!;
    const p = view.proposal!;
    expect(p.topic).toBe(topic);
    expect(p.request).toContain(saved.entry.text);
    expect(p.request).toContain(saved.entry.at);
    expect(p.request).not.toContain("Widok Dziś");
    expect(newJournalEntries(api.snapshotForTests().journal.filter((j) => j.project === project && j.topic === topic), view.journal_refresh?.[topic])).toEqual([]);
    expect(p.changes.map((c) => c.change.kind)).toEqual(["topic_log", "new_task", "task_status", "remove_task"]);
    await expect(api.startRun(req())).rejects.toThrow(/nierozstrzygnięte/);
    const removed = p.changes[3]!.change;
    if (removed.kind !== "remove_task") throw new Error("missing removal");
    const applied = await api.applyUpdate(run.id, defaultSelection(p));
    expect(applied.items.every((i) => i.status === "applied")).toBe(true);
    expect(api.snapshotForTests().tasks.some((t) => t.id === removed.task)).toBe(true);
    const undone = await api.undoUpdate(run.id, applied.undo!);
    expect(undone.items.every((i) => i.status === "undone")).toBe(true);
    expect(api.snapshotForTests().tasks).toEqual(before.tasks);
    const all = await api.applyUpdate(run.id, [0, 1, 2, 3]);
    expect(all.items.every((i) => i.status === "applied")).toBe(true);
    expect(api.snapshotForTests().tasks.some((t) => t.id === removed.task)).toBe(false);
    await api.undoUpdate(run.id, all.undo!);
    expect(api.snapshotForTests().tasks).toEqual(before.tasks);
    expect(api.snapshotForTests().journal).toEqual(before.journal);
    view = (await api.listUpdates()).find((v) => v.project === project)!;
    expect(view.pending).toBe(4);
    expect(newJournalEntries(api.snapshotForTests().journal.filter((j) => j.project === project && j.topic === topic), view.journal_refresh?.[topic])).toEqual([]);
  });

  it.each(["awaria", "bez zmian", "cancel"])("keeps the note for %s", async (text) => {
    vi.useFakeTimers();
    const api = createMockApi({ runs: { stepMs: 5 } });
    await api.addJournalEntry(project, topic, { text });
    const run = await api.startRun(req());
    if (text === "cancel") await api.cancelRun(run.id);
    await vi.advanceTimersByTimeAsync(200);
    const view = (await api.listUpdates()).find((v) => v.project === project)!;
    expect(view.last?.outcome.kind).toBe(text === "awaria" ? "failed" : text === "cancel" ? "cancelled" : "no_changes");
    expect(api.snapshotForTests().journal.filter((j) => j.text === text)).toHaveLength(1);
    const entries = api.snapshotForTests().journal.filter((j) => j.project === project && j.topic === topic);
    expect(newJournalEntries(entries, view.journal_refresh?.[topic]).some((e) => e.text === text)).toBe(text !== "bez zmian");
    expect(!!view.journal_refresh?.[topic]).toBe(text === "bez zmian");
  });

  it("captures only start-time entries and preserves the marker across apply and undo", async () => {
    vi.useFakeTimers();
    const api = createMockApi({ runs: { stepMs: 5 } });
    await api.addJournalEntry(project, topic, { text: "Etap gotowy" });
    const run = await api.startRun(req());
    await api.addJournalEntry(project, topic, { text: "Wpis podczas analizy" });
    await vi.advanceTimersByTimeAsync(200);
    const view = (await api.listUpdates()).find((v) => v.project === project)!;
    expect(view.proposal?.request).not.toContain("Wpis podczas analizy");
    const marker = view.journal_refresh![topic];
    const fresh = () => newJournalEntries(api.snapshotForTests().journal.filter((e) => e.project === project && e.topic === topic), marker);
    expect(fresh().map((e) => e.text)).toEqual(["Wpis podczas analizy"]);
    const applied = await api.applyUpdate(run.id, defaultSelection(view.proposal!));
    expect(fresh().map((e) => e.text)).toEqual(["Wpis podczas analizy"]);
    await api.undoUpdate(run.id, applied.undo!);
    expect(fresh().map((e) => e.text)).toEqual(["Wpis podczas analizy"]);
  });

  it("refuses a refresh with no new user entries", async () => {
    vi.useFakeTimers();
    const api = createMockApi({ runs: { stepMs: 5 } });
    await api.addJournalEntry(project, topic, { text: "bez zmian" });
    await api.startRun(req());
    await vi.advanceTimersByTimeAsync(200);
    await expect(api.startRun(req())).rejects.toThrow(/Brak nowych wpisów/);
  });

  it("gives multiple notes saved in the same minute distinct render identities", async () => {
    const api = createMockApi();
    const first = await api.addJournalEntry(project, topic, { text: "Pierwszy" });
    const second = await api.addJournalEntry(project, topic, { text: "Drugi" });
    expect(first.entry.line).not.toBe(second.entry.line);
  });
});
