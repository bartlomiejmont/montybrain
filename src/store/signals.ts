// Drift signals (`list_drift` + `drift-changed`) and the waiting schedule
// (`list_waiting` + `waiting-changed`), shared by Dziś and the project view.
// Both re-load when the vault changes (topics, statuses and journals feed them).

import { useSyncExternalStore } from "react";

import { api } from "../api";
import type { MorwiskoApi } from "../api/api";
import type { ProjectDrift, WaitView } from "../api/types";

export interface ListSnapshot<T> {
  loaded: boolean;
  list: T[];
}

function createListStore<T>(load: () => Promise<T[]>, listen: (cb: () => void) => Promise<() => void>, onVault: (cb: () => void) => Promise<() => void>) {
  let snap: ListSnapshot<T> = { loaded: false, list: [] };
  const subs = new Set<() => void>();
  let started: Promise<void> | null = null;
  let seq = 0;
  const set = (list: T[]) => {
    snap = { loaded: true, list };
    subs.forEach((f) => f());
  };
  const reload = async () => {
    const mine = ++seq;
    const list = await load();
    if (mine === seq) set(list);
  };
  return {
    getSnapshot: () => snap,
    subscribe(f: () => void) {
      subs.add(f);
      return () => void subs.delete(f);
    },
    start(): Promise<void> {
      started ??= (async () => {
        await listen(() => void reload().catch(() => {}));
        await onVault(() => void reload().catch(() => {}));
        await reload();
      })().catch((e) => {
        started = null;
        throw e;
      });
      return started;
    },
    reload,
    set,
  };
}

export function createDriftStore(a: Pick<MorwiskoApi, "listDrift" | "onDriftChanged" | "onChange">) {
  return createListStore<ProjectDrift>(() => a.listDrift(), (cb) => a.onDriftChanged(cb), (cb) => a.onChange(() => cb()));
}

export function createWaitingStore(a: Pick<MorwiskoApi, "listWaiting" | "onWaitingChanged" | "onChange">) {
  return createListStore<WaitView>(() => a.listWaiting(), (cb) => a.onWaitingChanged(cb), (cb) => a.onChange(() => cb()));
}

export const driftStore = createDriftStore(api);
export const waitingStore = createWaitingStore(api);

export function useDrift(): ListSnapshot<ProjectDrift> {
  return useSyncExternalStore(driftStore.subscribe, driftStore.getSnapshot);
}

export function useWaiting(): ListSnapshot<WaitView> {
  return useSyncExternalStore(waitingStore.subscribe, waitingStore.getSnapshot);
}

// The default "Przypomnij za" (Settings → waiting_days), read once and on settings changes.
let waitingDays = 2;
let daysLoaded: Promise<void> | null = null;
const daysSubs = new Set<() => void>();
function loadDays() {
  daysLoaded ??= api
    .getSettings()
    .then((s) => {
      waitingDays = s.waiting_days ?? 2;
      daysSubs.forEach((f) => f());
      void api.onSettingsChange((s2) => {
        waitingDays = s2.waiting_days ?? 2;
        daysSubs.forEach((f) => f());
      });
    })
    .catch(() => {
      daysLoaded = null;
    });
}

export function useWaitingDays(): number {
  loadDays();
  return useSyncExternalStore(
    (f) => {
      daysSubs.add(f);
      return () => void daysSubs.delete(f);
    },
    () => waitingDays,
  );
}
