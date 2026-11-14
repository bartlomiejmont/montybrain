// Vault state for the whole UI. The backend (files + watcher) is the source of
// truth: after every mutation and every `vault-changed` event we re-fetch the
// state and keep only the newest version.

import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { api, type AppView, type MorwiskoApi } from "../api";
import { toIsoDate } from "../lib/dates";
import { describeError } from "../lib/errors";

export interface ToastAction {
  label: string;
  run: () => void;
}

export interface Toast {
  id: number;
  text: string;
  /** One action button (e.g. "Cofnij"); the toast then stays longer. */
  action?: ToastAction;
}

interface VaultContextValue {
  view: AppView | null;
  /** Fatal error of the very first load. */
  fatal: string | null;
  api: MorwiskoApi;
  refresh(): Promise<void>;
  /** Runs a backend call; on error shows a toast and resolves to `undefined`. */
  run<T>(call: (api: MorwiskoApi) => Promise<T>): Promise<T | undefined>;
  toasts: Toast[];
  notify(text: string, action?: ToastAction): void;
  dismiss(id: number): void;
}

const VaultContext = createContext<VaultContextValue | null>(null);

export function VaultProvider({ children }: { children: ReactNode }) {
  const [view, setView] = useState<AppView | null>(null);
  const [fatal, setFatal] = useState<string | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const latest = useRef(0);
  const toastId = useRef(0);

  const notify = useCallback((text: string, action?: ToastAction) => {
    const id = ++toastId.current;
    setToasts((t) => [...t.slice(-2), { id, text, action }]);
    window.setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), action ? 12000 : 6000);
  }, []);

  const dismiss = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);

  const refresh = useCallback(async () => {
    try {
      const next = await api.getState();
      // Responses can arrive out of order; never go back to an older version
      // (same version is accepted so a new local day re-computes `today`).
      if (next.version < latest.current) return;
      latest.current = next.version;
      setView(next);
      setFatal(null);
    } catch (e) {
      if (latest.current === 0) setFatal(describeError(e));
      else notify(describeError(e));
    }
  }, [notify]);

  const run = useCallback(
    async <T,>(call: (api: MorwiskoApi) => Promise<T>): Promise<T | undefined> => {
      try {
        const out = await call(api);
        await refresh();
        return out;
      } catch (e) {
        notify(describeError(e));
        await refresh();
        return undefined;
      }
    },
    [notify, refresh],
  );

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    api
      .onChange((e) => {
        if (e.version > latest.current) void refresh();
      })
      .then((u) => {
        if (disposed) u();
        else unlisten = u;
      });
    void refresh();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [refresh]);

  // Browser mock only: a handle to simulate an external change (an agent or an
  // editor writing a file) from the console, e.g.
  //   __morwisko.api.updateTask("g7h8i9", { notes: "from outside" })
  useEffect(() => {
    if (api.mode !== "mock") return;
    (window as unknown as { __morwisko?: unknown }).__morwisko = { api, refresh };
  }, [refresh]);

  // A new local day changes "Dziś" without any file change.
  useEffect(() => {
    const timer = window.setInterval(() => {
      if (view && toIsoDate(new Date()) !== view.today) void refresh();
    }, 60_000);
    return () => window.clearInterval(timer);
  }, [view, refresh]);

  const value = useMemo<VaultContextValue>(
    () => ({ view, fatal, api, refresh, run, toasts, notify, dismiss }),
    [view, fatal, refresh, run, toasts, notify, dismiss],
  );
  return <VaultContext.Provider value={value}>{children}</VaultContext.Provider>;
}

export function useVault(): VaultContextValue {
  const ctx = useContext(VaultContext);
  if (!ctx) throw new Error("useVault outside VaultProvider");
  return ctx;
}

/** The loaded view; only call below the loading gate in <App>. */
export function useView(): AppView {
  const { view } = useVault();
  if (!view) throw new Error("vault not loaded yet");
  return view;
}
