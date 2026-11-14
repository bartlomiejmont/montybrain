// Pure state of agent runs: what the store keeps and how backend messages fold
// into it. Messages may arrive out of order, twice, or before the run is known.

import type { RunEvent, RunInfo } from "../api/types";

/** Per run; the backend caps too, the oldest lines are dropped first. */
export const MAX_EVENTS = 1500;
/** The backend keeps 20; so do we. */
export const MAX_RUNS = 20;
/** Events that arrived before their run (kept per run id, bounded). */
const MAX_PENDING_RUNS = 20;

export interface RunsState {
  runs: Record<string, RunInfo>;
  /** Insertion counter per run, the tie-breaker for equal start times. */
  seen: Record<string, number>;
  counter: number;
  /** Events for runs not known yet. */
  pending: Record<string, RunEvent[]>;
}

export type RunsAction =
  | { type: "event"; event: RunEvent }
  | { type: "updated"; run: RunInfo }
  | { type: "snapshot"; runs: RunInfo[] };

export const initialRunsState: RunsState = { runs: {}, seen: {}, counter: 0, pending: {} };

/** Merges by `seq` (duplicates: the later copy wins), sorted ascending, capped. */
export function mergeEvents(a: readonly RunEvent[], b: readonly RunEvent[], cap = MAX_EVENTS): RunEvent[] {
  if (b.length === 0 && a.length <= cap) return a as RunEvent[];
  const bySeq = new Map<number, RunEvent>();
  for (const e of a) bySeq.set(e.seq, e);
  for (const e of b) bySeq.set(e.seq, e);
  const all = [...bySeq.values()].sort((x, y) => x.seq - y.seq);
  return all.length > cap ? all.slice(all.length - cap) : all;
}

const isTerminal = (s: RunInfo["status"]) => s !== "running";

function upsert(state: RunsState, incoming: RunInfo): RunsState {
  const prev = state.runs[incoming.id];
  const pending = state.pending[incoming.id] ?? [];
  let next: RunInfo;
  if (!prev) {
    next = { ...incoming, events: mergeEvents(incoming.events ?? [], pending) };
  } else {
    // A late "running" update must not reopen a finished run.
    const stale = isTerminal(prev.status) && !isTerminal(incoming.status);
    const base = stale ? prev : incoming;
    next = { ...base, events: mergeEvents(prev.events, incoming.events ?? []) };
  }
  const { [incoming.id]: _drop, ...restPending } = state.pending;
  void _drop;
  const seen = prev ? state.seen : { ...state.seen, [incoming.id]: state.counter };
  return trimRuns({ ...state, runs: { ...state.runs, [incoming.id]: next }, seen, counter: prev ? state.counter : state.counter + 1, pending: restPending });
}

function trimRuns(state: RunsState): RunsState {
  const ids = Object.keys(state.runs);
  if (ids.length <= MAX_RUNS) return state;
  const sorted = sortRuns(state);
  const keep = new Set(sorted.slice(0, MAX_RUNS).map((r) => r.id));
  for (const r of sorted) if (r.status === "running") keep.add(r.id);
  const runs: Record<string, RunInfo> = {};
  const seen: Record<string, number> = {};
  for (const id of keep) {
    runs[id] = state.runs[id]!;
    seen[id] = state.seen[id]!;
  }
  return { ...state, runs, seen };
}

export function runsReducer(state: RunsState, action: RunsAction): RunsState {
  switch (action.type) {
    case "event": {
      const e = action.event;
      const run = state.runs[e.run_id];
      if (!run) {
        const list = mergeEvents(state.pending[e.run_id] ?? [], [e]);
        let pending = { ...state.pending, [e.run_id]: list };
        const keys = Object.keys(pending);
        if (keys.length > MAX_PENDING_RUNS) {
          const { [keys[0]!]: _d, ...rest } = pending;
          void _d;
          pending = rest;
        }
        return { ...state, pending };
      }
      if (run.events.some((x) => x.seq === e.seq)) {
        // duplicate delivery: keep the log as is
        return state;
      }
      return { ...state, runs: { ...state.runs, [run.id]: { ...run, events: mergeEvents(run.events, [e]) } } };
    }
    case "updated":
      return upsert(state, action.run);
    case "snapshot":
      return action.runs.reduce(upsert, state);
  }
}

/** Newest first (by start time; ties: the one seen later). */
export function sortRuns(state: RunsState): RunInfo[] {
  return Object.values(state.runs).sort(
    (a, b) => b.started_at.localeCompare(a.started_at) || (state.seen[b.id] ?? 0) - (state.seen[a.id] ?? 0),
  );
}

export function activeBackgroundRun(state: RunsState): RunInfo | null {
  return Object.values(state.runs).find((r) => r.mode === "background" && r.status === "running") ?? null;
}
