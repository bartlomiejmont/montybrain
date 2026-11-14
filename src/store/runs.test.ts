import { describe, expect, it, vi } from "vitest";

import { createMockRuns } from "../api/mock-runs";
import { createRunStore } from "./runs";

// The store module imports the real `api` (browser globals are not needed: isTauri() is false in node).
const req = (text = "x", mode: "background" | "terminal" = "background") => ({ recipe: "zadania", provider: "claude" as const, mode, params: { text, project: "" } });

describe("run store with mock backend", () => {
  it("streams a background run to completion and refuses a second one", async () => {
    vi.useFakeTimers();
    const mock = createMockRuns({ stepMs: 10 });
    const store = createRunStore(mock);
    await store.start();
    const run = await store.startRun(req());
    expect(store.getSnapshot().active?.id).toBe(run.id);
    await expect(store.startRun(req())).rejects.toThrow(/w toku/);
    await vi.advanceTimersByTimeAsync(500);
    const done = store.getSnapshot().data.runs[run.id]!;
    expect(done.status).toBe("succeeded");
    expect(done.result).toContain("Dodałem");
    const seqs = done.events.map((e) => e.seq);
    expect(seqs).toEqual([...seqs].sort((a, b) => a - b));
    expect(new Set(seqs).size).toBe(seqs.length);
    expect(store.getSnapshot().active).toBeNull();
    vi.useRealTimers();
  });

  it("cancels and fails", async () => {
    vi.useFakeTimers();
    const mock = createMockRuns({ stepMs: 10 });
    const store = createRunStore(mock);
    await store.start();
    const a = await store.startRun(req());
    await vi.advanceTimersByTimeAsync(25);
    await store.cancel(a.id);
    await vi.advanceTimersByTimeAsync(200);
    expect(store.getSnapshot().data.runs[a.id]!.status).toBe("cancelled");
    expect(store.getSnapshot().data.runs[a.id]!.events.length).toBeGreaterThan(0);
    const b = await store.startRun(req("to ma fail"));
    await vi.advanceTimersByTimeAsync(200);
    const failed = store.getSnapshot().data.runs[b.id]!;
    expect(failed.status).toBe("failed");
    expect(failed.error).toBeTruthy();
    vi.useRealTimers();
  });

  it("terminal run returns 'opened' and rejects bad input", async () => {
    const mock = createMockRuns({ stepMs: 10, codex: "missing" });
    const store = createRunStore(mock);
    const r = await store.startRun({ recipe: "poczta", provider: "claude", mode: "terminal", params: { text: "", period: "tydzien" } });
    expect(r).toMatchObject({ mode: "terminal", status: "succeeded" });
    await expect(store.startRun({ recipe: "poczta", provider: "claude", mode: "background", params: {} })).rejects.toThrow();
    await expect(store.startRun({ ...req(), provider: "codex" })).rejects.toThrow(/niedostępny/);
  });
});
