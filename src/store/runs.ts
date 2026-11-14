// Agent runs live here, outside any view: the two backend events are subscribed
// once (`start()`), so a run keeps streaming while the user is on another page.

import { useSyncExternalStore } from "react";

import { api } from "../api";
import type { MontyBrainApi } from "../api/api";
import type { RunInfo, RunRequest } from "../api/types";
import { activeBackgroundRun, initialRunsState, runsReducer, sortRuns, type RunsAction, type RunsState } from "../lib/runs";

export interface RunsSnapshot {
  data: RunsState;
  /** Explicitly chosen run (a click in the list, or the one just started). */
  selectedId: string | null;
  /** Newest first. */
  list: RunInfo[];
  active: RunInfo | null;
}

export function createRunStore(api: Pick<MontyBrainApi, "onRunEvent" | "onRunUpdated" | "listRuns" | "startRun" | "cancelRun">) {
  let data = initialRunsState;
  let selectedId: string | null = null;
  let snap: RunsSnapshot = build();
  const subs = new Set<() => void>();
  let started: Promise<void> | null = null;
  const offs: (() => void)[] = [];

  function build(): RunsSnapshot {
    return { data, selectedId, list: sortRuns(data), active: activeBackgroundRun(data) };
  }
  function commit() {
    snap = build();
    subs.forEach((f) => f());
  }
  function dispatch(a: RunsAction) {
    const next = runsReducer(data, a);
    if (next === data) return;
    data = next;
    commit();
  }

  return {
    getSnapshot: () => snap,
    subscribe(f: () => void) {
      subs.add(f);
      return () => void subs.delete(f);
    },
    /** Idempotent: subscribes to both events, then loads the backend's list. */
    start(): Promise<void> {
      started ??= (async () => {
        // Subscribe first, list afterwards: nothing between the two is lost
        // (and duplicates are harmless).
        offs.push(await api.onRunEvent((event) => dispatch({ type: "event", event })));
        offs.push(await api.onRunUpdated((run) => dispatch({ type: "updated", run })));
        await this.refresh();
      })().catch((e) => {
        started = null;
        throw e;
      });
      return started;
    },
    async refresh(): Promise<void> {
      dispatch({ type: "snapshot", runs: await api.listRuns() });
    },
    stop() {
      offs.splice(0).forEach((f) => f());
      started = null;
    },
    select(id: string | null) {
      if (selectedId === id) return;
      selectedId = id;
      commit();
    },
    /** Rejects with the backend's message; on success the run becomes selected. */
    async startRun(req: RunRequest): Promise<RunInfo> {
      const run = await api.startRun(req);
      dispatch({ type: "updated", run });
      selectedId = run.id;
      commit();
      return run;
    },
    async cancel(id: string): Promise<void> {
      const run = await api.cancelRun(id);
      dispatch({ type: "updated", run });
    },
  };
}

export type RunStore = ReturnType<typeof createRunStore>;

/** The app-wide store, bound to the real `api` (Tauri or mock). */
export const runStore: RunStore = createRunStore(api);

export function useRuns(): RunsSnapshot {
  return useSyncExternalStore(runStore.subscribe, runStore.getSnapshot);
}
