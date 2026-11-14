// Small UI state that must survive an external refresh and component remounts:
// which tasks are expanded / being renamed, and what to reveal after a jump
// from the board or the palette. Module-level, so a task that moves between
// lists (and is re-created by React) keeps its expanded detail.

import { useCallback, useSyncExternalStore } from "react";

type Listener = () => void;

function createSet() {
  let items: ReadonlySet<string> = new Set();
  const subs = new Set<Listener>();
  const emit = () => subs.forEach((f) => f());
  return {
    subscribe: (f: Listener) => (subs.add(f), () => void subs.delete(f)),
    has: (k: string) => items.has(k),
    set(k: string, on: boolean) {
      if (items.has(k) === on) return;
      const n = new Set(items);
      if (on) n.add(k);
      else n.delete(k);
      items = n;
      emit();
    },
    keepOnly(keep: string | null) {
      if (items.size === 0) return;
      items = new Set(keep && items.has(keep) ? [keep] : []);
      emit();
    },
  };
}

const expandedTasks = createSet();
const renamingTasks = createSet();
const editingTopics = createSet();

/** Expanded / renaming state of a task, by id (null id = no shared state). */
export function useTaskFlag(
  kind: "expanded" | "renaming" | "topic-editing",
  id: string | null,
): [boolean, (v: boolean) => void] {
  const store = kind === "expanded" ? expandedTasks : kind === "renaming" ? renamingTasks : editingTopics;
  const on = useSyncExternalStore(store.subscribe, () => (id ? store.has(id) : false));
  const set = useCallback((v: boolean) => void (id && store.set(id, v)), [id, store]);
  return [on, set];
}

// --- reveal: "show me this task / topic" ---------------------------------

export interface Reveal {
  nonce: number;
  at: number;
  task: string | null;
  project: string | null;
  topic: string | null;
}

let pendingNav = false;
let reveal: Reveal = { nonce: 0, at: 0, task: null, project: null, topic: null };
const revealSubs = new Set<Listener>();

function setReveal(r: Omit<Reveal, "nonce" | "at">) {
  reveal = { ...r, nonce: reveal.nonce + 1, at: Date.now() };
  revealSubs.forEach((f) => f());
}

/** Expands the task and asks the target view to open its topic and scroll to it. */
export function revealTask(id: string | null, project: string | null, topic: string | null) {
  if (id) expandedTasks.set(id, true);
  pendingNav = true;
  window.setTimeout(() => (pendingNav = false), 800);
  setReveal({ task: id, project, topic });
}

const handled = new Set<string>();
/**
 * True once per reveal and consumer (`who`), and only shortly after the jump,
 * so a later remount does not scroll and flash again.
 */
export function takeReveal(r: Reveal, who: string): boolean {
  const k = `${r.nonce}:${who}`;
  if (handled.has(k) || Date.now() - r.at > 2500) return false;
  handled.add(k);
  return true;
}

export function revealTopic(project: string, topic: string) {
  setReveal({ task: null, project, topic });
}

export function useReveal(): Reveal {
  return useSyncExternalStore(
    (f) => (revealSubs.add(f), () => void revealSubs.delete(f)),
    () => reveal,
  );
}

// Navigating to another view collapses what was expanded there (as before),
// except the task that is being revealed. Registered before React listens.
if (typeof window !== "undefined") {
  window.addEventListener("hashchange", () => {
    expandedTasks.keepOnly(pendingNav ? reveal.task : null);
    pendingNav = false;
    renamingTasks.keepOnly(null);
    editingTopics.keepOnly(null);
  });
}

// --- stored preferences (per viewer, best effort) ------------------------

export function readPref(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function writePref(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    /* storage unavailable */
  }
}
