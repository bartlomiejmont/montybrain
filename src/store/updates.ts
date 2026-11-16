// Project update state (`list_updates` + `updates-changed`), outside any view so
// the sidebar marker and the project view share it. Also a small cache of the
// agent CLIs (`list_providers`) for the update strip.

import { useSyncExternalStore } from "react";

import { api } from "../api";
import type { MontyBrainApi } from "../api/api";
import type { ProjectUpdateView, ProviderInfo } from "../api/types";

export interface UpdatesSnapshot {
  loaded: boolean;
  byProject: Record<string, ProjectUpdateView>;
}

export function createUpdateStore(api: Pick<MontyBrainApi, "listUpdates" | "onUpdatesChanged" | "listProviders">) {
  let snap: UpdatesSnapshot = { loaded: false, byProject: {} };
  let providers: ProviderInfo[] | null = null;
  let providersAt = 0;
  const subs = new Set<() => void>();
  let started: Promise<void> | null = null;
  let loadingProviders: Promise<void> | null = null;
  let loadingProvidersForced = false;
  const emit = () => subs.forEach((f) => f());

  function set(list: ProjectUpdateView[]) {
    snap = { loaded: true, byProject: Object.fromEntries(list.map((v) => [v.project, v])) };
    emit();
  }

  return {
    getSnapshot: () => snap,
    getProviders: () => providers,
    subscribe(f: () => void) {
      subs.add(f);
      return () => void subs.delete(f);
    },
    /** Idempotent: subscribes to `updates-changed`, then loads the list. */
    start(): Promise<void> {
      started ??= (async () => {
        await api.onUpdatesChanged(set);
        set(await api.listUpdates());
      })().catch((e) => {
        started = null;
        throw e;
      });
      return started;
    },
    /** A newer list from a command's response. */
    set,
    /** Loads the providers when unknown, older than a minute, or `force`d (re-probe). */
    loadProviders: function loadProviders(force = false): Promise<void> {
      if (loadingProviders) {
        if (force && !loadingProvidersForced) {
          return loadingProviders.catch(() => {}).then(() => loadProviders(true));
        }
        return loadingProviders;
      }
      if (!force && providers && Date.now() - providersAt < 60_000) return Promise.resolve();
      loadingProvidersForced = force;
      loadingProviders ??= api
        .listProviders(force)
        .then((p) => {
          providers = p;
          providersAt = Date.now();
          emit();
        })
        .finally(() => {
          loadingProviders = null;
        });
      return loadingProviders;
    },
  };
}

export type UpdateStore = ReturnType<typeof createUpdateStore>;

export const updateStore: UpdateStore = createUpdateStore(api);

export function useUpdates(): UpdatesSnapshot {
  return useSyncExternalStore(updateStore.subscribe, updateStore.getSnapshot);
}

export function useProviders(): ProviderInfo[] | null {
  return useSyncExternalStore(updateStore.subscribe, updateStore.getProviders);
}
