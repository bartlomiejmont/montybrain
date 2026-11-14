// Saving Settings → "Agenci": one field at a time, in order, each merged into
// the latest saved settings (a model typed and then an effort picked right
// away must both stick), and a "Zapisano" for every save that succeeded.

import type { AgentChoice, Agents, ProviderId } from "../api/types";

export interface AgentSaver {
  /** The settings changed elsewhere (another window, a reload). */
  sync(agents: Agents): void;
  /** Saves `patch` for `provider`, after the saves already queued; resolves to the saved settings. */
  save(provider: ProviderId, patch: Partial<AgentChoice>): Promise<Agents>;
}

export function createAgentSaver(send: (next: Agents) => Promise<Agents>, initial: Agents): AgentSaver {
  let latest = initial;
  let chain: Promise<unknown> = Promise.resolve();
  return {
    sync(agents) {
      latest = agents;
    },
    save(provider, patch) {
      const run = chain.then(async () => {
        const next: Agents = { ...latest, [provider]: { ...latest[provider], ...patch } };
        latest = await send(next);
        return latest;
      });
      chain = run.catch(() => {});
      return run;
    },
  };
}

/** The model field's value to save, or `null` when it is what is stored (nothing to send). */
export function modelToSave(draft: string, stored: string | null): string | null | undefined {
  const v = draft.trim() || null;
  return v === stored ? undefined : v;
}
