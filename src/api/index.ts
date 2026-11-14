// Picks the backend: real Tauri commands inside the app, the in-memory mock in a
// plain browser. Everything else imports `api` from here.

import { isTauri } from "@tauri-apps/api/core";

import type { MontyBrainApi } from "./api";
import { createMockApi } from "./mock";
import { UPDATE_SCENARIOS, type UpdateScenario } from "./mock-updates";
import { createTauriApi } from "./tauri";

export type { MontyBrainApi } from "./api";
export * from "./types";

// Browser-mock switches for testing the „Akcje" view: ?codex=off (Codex unavailable),
// ?step=150 (ms between scripted agent log lines). Project update states for
// screenshots (project „whenubuy”): ?update=never|running|proposals|applied|
// nochanges|error|unavailable|auto|pending|request|requestrun (see api/mock-updates.ts).
const query = () => new URLSearchParams(typeof window === "undefined" ? "" : window.location.search);

/** A browser-mock URL switch (`null` inside the app), e.g. `?recipe=aktualizacja` for Akcje. */
export function mockQuery(name: string): string | null {
  return isTauri() ? null : query().get(name);
}

/** The `?update=` scenario of the browser mock (never set inside the app). */
export function mockUpdateScenario(): UpdateScenario | null {
  if (isTauri()) return null;
  const v = query().get("update");
  return UPDATE_SCENARIOS.includes(v as UpdateScenario) ? (v as UpdateScenario) : null;
}

function mockMap(): "demo" | "large" | undefined {
  const v = query().get("map");
  return v === "demo" || v === "large" ? v : undefined;
}

function mockRunOptions() {
  const q = query();
  const step = Number(q.get("step"));
  const scenario = mockUpdateScenario();
  return {
    codex: q.get("codex") === "off" || scenario === "unavailable" ? ("missing" as const) : ("ok" as const),
    claude: scenario === "unavailable" ? ("missing" as const) : ("ok" as const),
    ...(step >= 0 && q.has("step") ? { stepMs: step } : {}),
  };
}

export const api: MontyBrainApi = isTauri()
  ? createTauriApi()
  : createMockApi({ latencyMs: 40, runs: mockRunOptions(), updateScenario: mockUpdateScenario() ?? undefined, drift: query().get("drift") === "none" ? "none" : undefined, today: query().get("today") === "full" ? "full" : undefined, map: mockMap() });
