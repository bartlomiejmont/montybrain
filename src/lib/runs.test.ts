import { describe, expect, it } from "vitest";

import type { RunEvent, RunInfo } from "../api/types";
import { MAX_EVENTS, MAX_RUNS, activeBackgroundRun, initialRunsState, mergeEvents, runsReducer, sortRuns } from "./runs";

const ev = (seq: number, run_id = "r1", text = `l${seq}`): RunEvent => ({ run_id, seq, at: "2026-11-13T10:00:00", kind: "text", text });
const run = (id: string, over: Partial<RunInfo> = {}): RunInfo => ({
  id,
  recipe: "postep",
  recipe_title: "Otwarte zadania i postęp",
  provider: "claude",
  mode: "background",
  project: null,
  model: null,
  effort: null,
  status: "running",
  started_at: "2026-11-13T10:00:00",
  finished_at: null,
  result: null,
  error: null,
  events: [],
  ...over,
});
const apply = (...a: Parameters<typeof runsReducer>[1][]) => a.reduce(runsReducer, initialRunsState);

describe("runs reducer", () => {
  it("orders events by seq and drops duplicates", () => {
    const s = apply({ type: "updated", run: run("r1") }, { type: "event", event: ev(2) }, { type: "event", event: ev(1) }, { type: "event", event: ev(2) });
    expect(s.runs.r1!.events.map((e) => e.seq)).toEqual([1, 2]);
  });
  it("an update without events does not wipe the log", () => {
    const s = apply({ type: "updated", run: run("r1") }, { type: "event", event: ev(1) }, { type: "updated", run: run("r1", { status: "succeeded", result: "ok", finished_at: "2026-11-13T10:01:00" }) });
    expect(s.runs.r1!.events).toHaveLength(1);
    expect(s.runs.r1!.status).toBe("succeeded");
    expect(s.runs.r1!.result).toBe("ok");
  });
  it("buffers events that arrive before the run", () => {
    const s = apply({ type: "event", event: ev(2) }, { type: "event", event: ev(1) }, { type: "updated", run: run("r1") });
    expect(s.runs.r1!.events.map((e) => e.seq)).toEqual([1, 2]);
    expect(s.pending).toEqual({});
  });
  it("a snapshot merges its events with live ones", () => {
    const s = apply({ type: "updated", run: run("r1") }, { type: "event", event: ev(3) }, { type: "snapshot", runs: [run("r1", { events: [ev(1), ev(2), ev(3)] })] });
    expect(s.runs.r1!.events.map((e) => e.seq)).toEqual([1, 2, 3]);
  });
  it("a stale running update does not reopen a finished run", () => {
    const s = apply({ type: "updated", run: run("r1", { status: "failed", error: "x" }) }, { type: "updated", run: run("r1") });
    expect(s.runs.r1!.status).toBe("failed");
    expect(s.runs.r1!.error).toBe("x");
  });
  it("caps events, dropping the oldest", () => {
    const many = Array.from({ length: MAX_EVENTS + 50 }, (_, i) => ev(i + 1));
    const out = mergeEvents([], many);
    expect(out).toHaveLength(MAX_EVENTS);
    expect(out[0]!.seq).toBe(51);
    const s = apply({ type: "updated", run: run("r1", { events: many }) }, { type: "event", event: ev(MAX_EVENTS + 51) });
    expect(s.runs.r1!.events).toHaveLength(MAX_EVENTS);
    expect(s.runs.r1!.events.at(-1)!.seq).toBe(MAX_EVENTS + 51);
  });
  it("caps runs but keeps running ones, newest first", () => {
    const runs = Array.from({ length: MAX_RUNS + 3 }, (_, i) => run(`r${i}`, { started_at: `2026-11-13T10:${String(i).padStart(2, "0")}:00`, status: i === 0 ? "running" : "succeeded" }));
    const s = apply({ type: "snapshot", runs });
    const sorted = sortRuns(s);
    expect(sorted.length).toBe(MAX_RUNS + 1);
    expect(sorted[0]!.id).toBe(`r${MAX_RUNS + 2}`);
    expect(activeBackgroundRun(s)!.id).toBe("r0");
  });
  it("ties on start time keep the later-seen run first", () => {
    const s = apply({ type: "updated", run: run("a") }, { type: "updated", run: run("b") });
    expect(sortRuns(s).map((r) => r.id)).toEqual(["b", "a"]);
  });
  it("ignores a duplicated event without changing identity", () => {
    const s1 = apply({ type: "updated", run: run("r1") }, { type: "event", event: ev(1) });
    expect(runsReducer(s1, { type: "event", event: ev(1) })).toBe(s1);
  });
});
