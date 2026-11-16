import { t as tr } from "../i18n";
// One app-wide conversation; composing another request never changes its context.
import { useSyncExternalStore } from "react";

import { api, mockSessionState } from "../api";
import type { MontyBrainApi } from "../api/api";
import type { SessionAnswer, SessionStartRequest, SessionView, Topic } from "../api/types";
import { isLive, sessionRequest, type SessionDraft } from "../lib/session";

export interface SessionSnapshot {
  view: SessionView | null;
  open: boolean;
  draft: SessionDraft | null;
  composing: boolean;
  context: SessionDraft | null;
  busy: boolean;
}

type SessionApi = Pick<MontyBrainApi,
  "agentSessionStart" | "agentSessionGet" | "agentSessionSend" | "agentSessionAnswer" | "agentSessionInterrupt" | "agentSessionStop" | "agentSessionClose" | "onAgentSession"
>;

export function createSessionStore(api: SessionApi, openAtStart = false) {
  let snap: SessionSnapshot = { view: null, open: openAtStart, draft: null, composing: false, context: null, busy: false };
  const subs = new Set<() => void>();
  let started: Promise<void> | null = null;
  let revision = 0;
  let unsubscribe: (() => void) | null = null;
  const set = (patch: Partial<SessionSnapshot>) => {
    snap = { ...snap, ...patch };
    subs.forEach((f) => f());
  };
  const take = (v: SessionView | null) => {
    const cur = snap.view;
    if (v && cur && v.id === cur.id && v.entries.length < cur.entries.length && v.status === cur.status) return;
    revision++;
    set({ view: v });
  };
  function start(): Promise<void> {
    started ??= (async () => {
      unsubscribe ??= await api.onAgentSession(take);
      const before = revision;
      const current = await api.agentSessionGet();
      if (revision === before) take(current);
    })().catch((e) => { started = null; throw e; });
    return started;
  }
  return {
    getSnapshot: () => snap,
    subscribe(f: () => void) { subs.add(f); return () => void subs.delete(f); },
    start,
    prepare(draft: SessionDraft) {
      if (snap.busy) return;
      set({ draft, composing: true, open: true });
    },
    editDraft(patch: Partial<SessionDraft>) {
      if (snap.draft && !snap.busy) set({ draft: { ...snap.draft, ...patch } });
    },
    conversation: () => set({ composing: false }),
    compose(defaults: SessionDraft) {
      set({ draft: snap.draft ?? defaults, composing: true, open: true });
    },
    async begin(r: SessionStartRequest, context?: SessionDraft, replace = false): Promise<SessionView> {
      if (snap.busy) throw new Error(tr("Uruchamianie sesji już trwa."));
      set({ busy: true });
      try {
        await start();
        const current = await api.agentSessionGet();
        take(current);
        if (current && isLive(current.status)) {
          if (!replace) throw new Error(tr("Inna sesja już działa. Wróć do niej albo zakończ ją przed uruchomieniem nowej."));
          await api.agentSessionStop();
          take(await api.agentSessionGet());
        }
        const v = await api.agentSessionStart(r);
        take(v);
        set({ open: true, composing: false, draft: null, context: context ?? null });
        return v;
      } finally {
        set({ busy: false });
      }
    },
    open: () => set({ open: true }),
    hide: () => set({ open: false }),
    send: (text: string) => api.agentSessionSend(text),
    answer: (entry: number, a: SessionAnswer, answers?: Record<string, string>) => api.agentSessionAnswer(entry, a, answers),
    interrupt: () => api.agentSessionInterrupt(),
    stop: () => api.agentSessionStop(),
    async dismiss() {
      await api.agentSessionClose();
      set({ view: null, context: null, composing: !!snap.draft });
    },
  };
}

export const sessionStore = createSessionStore(api, mockSessionState() !== null);
export function useSession(): SessionSnapshot {
  return useSyncExternalStore(sessionStore.subscribe, sessionStore.getSnapshot, sessionStore.getSnapshot);
}

export function askAboutTopic(topic: Topic) {
  sessionStore.prepare({ scope: "project", project: topic.project, topic: { slug: topic.slug, title: topic.title }, text: "", origin: window.location.hash });
}

export async function startSessionDraft(draft: SessionDraft, provider: SessionStartRequest["provider"], replace: boolean) {
  return sessionStore.begin(sessionRequest(draft, provider), draft, replace);
}
