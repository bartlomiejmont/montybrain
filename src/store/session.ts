// The in-app agent session lives here, outside any view: the backend event is
// subscribed once, so the session keeps streaming while the user switches
// views; the panel is an overlay any view (and the sidebar indicator) opens.

import { useSyncExternalStore } from "react";

import { api, mockSessionState } from "../api";
import type { MontyBrainApi } from "../api/api";
import type { SessionAnswer, SessionStartRequest, SessionView } from "../api/types";

export interface SessionSnapshot {
  view: SessionView | null;
  /** The panel is open. */
  open: boolean;
}

type SessionApi = Pick<
  MontyBrainApi,
  "agentSessionStart" | "agentSessionGet" | "agentSessionSend" | "agentSessionAnswer" | "agentSessionInterrupt" | "agentSessionStop" | "agentSessionClose" | "onAgentSession"
>;

export function createSessionStore(api: SessionApi, openAtStart = false) {
  let snap: SessionSnapshot = { view: null, open: openAtStart };
  const subs = new Set<() => void>();
  let started: Promise<void> | null = null;
  const set = (patch: Partial<SessionSnapshot>) => {
    snap = { ...snap, ...patch };
    subs.forEach((f) => f());
  };
  // Views can arrive out of order across the event and a command's result:
  // an older one (fewer entries of the same session) never replaces a newer one.
  const take = (v: SessionView | null) => {
    const cur = snap.view;
    if (v && cur && v.id === cur.id && v.entries.length < cur.entries.length && v.status === cur.status) return;
    set({ view: v });
  };

  return {
    getSnapshot: () => snap,
    subscribe(f: () => void) {
      subs.add(f);
      return () => void subs.delete(f);
    },
    /** Idempotent: subscribes to the event, then reads the current session. */
    start(): Promise<void> {
      started ??= (async () => {
        await api.onAgentSession((v) => take(v));
        take(await api.agentSessionGet());
      })().catch((e) => {
        started = null;
        throw e;
      });
      return started;
    },
    async begin(r: SessionStartRequest): Promise<SessionView> {
      const v = await api.agentSessionStart(r);
      take(v);
      set({ open: true });
      return v;
    },
    open: () => set({ open: true }),
    hide: () => set({ open: false }),
    send: (text: string) => api.agentSessionSend(text),
    answer: (entry: number, a: SessionAnswer, answers?: Record<string, string>) => api.agentSessionAnswer(entry, a, answers),
    interrupt: () => api.agentSessionInterrupt(),
    stop: () => api.agentSessionStop(),
    /** "Zamknij" on an ended session: forgotten, the panel closes. */
    async dismiss() {
      await api.agentSessionClose();
      set({ view: null, open: false });
    },
  };
}

export const sessionStore = createSessionStore(api, mockSessionState() !== null);

export function useSession(): SessionSnapshot {
  return useSyncExternalStore(sessionStore.subscribe, sessionStore.getSnapshot, sessionStore.getSnapshot);
}
