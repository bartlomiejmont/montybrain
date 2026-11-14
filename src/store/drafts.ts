// Drafts that survive external refreshes (and remounts). Keyed state lives in a
// module-level store, so a field keeps its text when its component is replaced
// (a task moving between lists, a topic between "active" and "done").

import { useCallback, useLayoutEffect, useRef, useSyncExternalStore, type FocusEvent } from "react";

import {
  ackSave,
  adoptStored,
  cancelEdit,
  freshDraft,
  isDirty,
  leaveDraft,
  requestSave,
  showsConflict,
  startSave,
  syncDraft,
  takeQueued,
  target,
  withFocus,
  type DraftState,
} from "../lib/draft";

/** A draft whose field disappeared is only reused by a remount within this window. */
export const REMOUNT_WINDOW_MS = 3000;

/** Sends one field's value; resolves to the value as written, `undefined` on failure. */
export type SendSave = () => Promise<string | undefined>;

/**
 * The store behind `useDraft`. `now` and `schedule` are injectable for tests.
 * Abandoned drafts (field unmounted while dirty or saving) are kept for
 * `REMOUNT_WINDOW_MS`, then swept; one with a save in flight is kept until the
 * save (and any write queued after it) has its result.
 */
export function createDraftStore(
  now: () => number = () => Date.now(),
  schedule: (f: () => void, ms: number) => void = (f, ms) => void setTimeout(f, ms),
) {
  const states = new Map<string, DraftState>();
  const leftAt = new Map<string, number>();
  /** How many mounted fields use each key (the task title is in the row and the detail). */
  const holders = new Map<string, number>();
  /** The call that sends a field's queued value (see `requestSave`). */
  const queuedCalls = new Map<string, SendSave>();
  /** Explicit-save forms with a submission in flight (by draft prefix). */
  const busy = new Set<string>();
  const subs = new Set<() => void>();
  let sweepScheduled = false;
  let lastOp = 0;

  const emit = () => subs.forEach((f) => f());
  const expired = (key: string) => {
    const t = leftAt.get(key);
    return t !== undefined && now() - t > REMOUNT_WINDOW_MS && !states.get(key)?.pending;
  };
  /** Sends `value`; resolves once its result has been applied to the draft. */
  const dispatch = (key: string, s: DraftState, value: string, send: SendSave): Promise<void> => {
    const op = ++lastOp;
    store.put(key, startSave(s, op, value));
    return send().then(
      (w) => settle(key, op, w),
      () => settle(key, op, undefined),
    );
  };
  const settle = (key: string, op: number, written: string | undefined) => {
    const cur = states.get(key);
    if (!cur) return void queuedCalls.delete(key); // dropped meanwhile (editor closed)
    const { state, send: next } = takeQueued(ackSave(cur, op, written));
    const call = next === null ? undefined : queuedCalls.get(key);
    if (state.queued === null && !state.pending) queuedCalls.delete(key);
    if (next !== null && call) void dispatch(key, state, next, call);
    else store.put(key, state);
  };
  const scheduleSweep = () => {
    if (sweepScheduled || leftAt.size === 0) return;
    sweepScheduled = true;
    schedule(() => {
      sweepScheduled = false;
      store.sweep();
      scheduleSweep();
    }, REMOUNT_WINDOW_MS + 100);
  };

  const store = {
    subscribe(f: () => void) {
      subs.add(f);
      return () => void subs.delete(f);
    },
    /** The live draft for `key`; an expired abandoned one is not returned. */
    get(key: string): DraftState | undefined {
      return expired(key) ? undefined : states.get(key);
    },
    put(key: string, s: DraftState) {
      if (states.get(key) === s) return;
      states.set(key, s);
      emit();
    },
    /** A field with this key is mounted. */
    entered(key: string) {
      holders.set(key, (holders.get(key) ?? 0) + 1);
      if (expired(key)) states.delete(key);
      leftAt.delete(key);
    },
    /** A field with this key unmounted; the draft is abandoned when it was the last one. */
    left(key: string) {
      const n = (holders.get(key) ?? 1) - 1;
      if (n > 0) return void holders.set(key, n);
      holders.delete(key);
      const s = states.get(key);
      if (s && (isDirty(s) || s.pending || s.queued !== null)) {
        leftAt.set(key, now());
        scheduleSweep();
      } else {
        states.delete(key);
        leftAt.delete(key);
      }
    },
    /** Frees abandoned drafts older than the remount window (not while saving). */
    sweep() {
      for (const key of [...leftAt.keys()]) {
        if (!expired(key)) continue;
        leftAt.delete(key);
        states.delete(key);
        queuedCalls.delete(key);
      }
    },
    /** Drops every draft whose key starts with `prefix`. */
    drop(prefix: string) {
      for (const k of [...states.keys()]) if (k.startsWith(prefix)) states.delete(k);
      for (const k of [...leftAt.keys()]) if (k.startsWith(prefix)) leftAt.delete(k);
      for (const k of [...queuedCalls.keys()]) if (k.startsWith(prefix)) queuedCalls.delete(k);
      emit();
    },
    /**
     * The user asks to save `value` for `key` (`s` is its current state). Sent
     * now, or — while a save of this field is in flight — after its result
     * (only the latest such request is kept). Not sent when it is already what
     * will be stored. Resolves once a write sent now has its result applied
     * (at once when nothing was sent now).
     */
    save(key: string, s: DraftState, value: string, send: SendSave): Promise<void> {
      const { state, send: now } = requestSave(s, value);
      if (state.queued === null) queuedCalls.delete(key);
      else queuedCalls.set(key, send);
      if (now) return dispatch(key, state, value, send);
      store.put(key, state);
      return Promise.resolve();
    },
    isBusy: (key: string) => busy.has(key),
    /** Starts a submission of the form `key`; false when one is already running. */
    begin(key: string): boolean {
      if (busy.has(key)) return false;
      busy.add(key);
      emit();
      return true;
    },
    end(key: string) {
      if (busy.delete(key)) emit();
    },
    size: () => states.size,
    leftCount: () => leftAt.size,
  };
  return store;
}

const drafts = createDraftStore();

/** Drops every draft whose key starts with `prefix` (editor closed / saved). */
export function dropDrafts(prefix: string) {
  drafts.drop(prefix);
}

export interface DraftHandle {
  key: string;
  draft: string;
  setDraft(v: string): void;
  dirty: boolean;
  /** A save of this field is waiting for its result. */
  saving: boolean;
  /** The stored value changed while this draft is being edited. */
  elsewhere: boolean;
  onFocus(): void;
  /** Leaves the field without saving (explicit-save forms). */
  onBlur(): void;
  /**
   * Leaves the field (autosave forms): reconciles with the current stored value
   * and returns the text the user changed, or null when they changed nothing
   * (judged against what will be stored once the save in flight lands).
   */
  leave(): string | null;
  /** Like `leave` but keeps the focus state: what an explicit "Save" should send for this field. */
  edited(): string | null;
  /** What this field will hold once the writes in flight land (compare `edited()` with this). */
  stored(): string;
  /** The current state (for decisions made after an await). */
  state(): DraftState;
  /**
   * Saves `v`: `call` sends it — now, or after the result of a save of this
   * field already in flight (only the latest request waits; see `requestSave`).
   * Not sent when `v` is already what will be stored. The save is acknowledged
   * by its own result: `written` picks this field's stored value from it;
   * `undefined` means it failed. Resolves once a write sent now has its
   * result applied (at once when nothing was sent now).
   */
  save<T>(v: string, call: () => Promise<T | undefined>, written: (result: T) => string): Promise<void>;
  /** Take the stored value instead of the draft (the value being saved, while saving). */
  adoptStored(keepFocus?: boolean): void;
  /** Drop only what was typed in this edit; writes in flight or queued stay (see `cancelEdit`). */
  cancelEdit(keepFocus?: boolean): void;
}

/**
 * A text draft for the stored `value`. `key` identifies the field; the same key
 * in a remounted component continues the same draft.
 */
export function useDraft(key: string, value: string): DraftHandle {
  const raw = useSyncExternalStore(drafts.subscribe, () => drafts.get(key));
  const state = syncDraft(raw ?? freshDraft(value), value);
  const latest = useRef({ key, value, state });
  latest.current = { key, value, state };

  useLayoutEffect(() => {
    drafts.entered(key);
    return () => drafts.left(key);
  }, [key]);
  useLayoutEffect(() => {
    if (drafts.get(key) !== state) drafts.put(key, state);
  });

  /** The freshest state: the store's, reconciled with the latest stored value. */
  const current = () => {
    const l = latest.current;
    return { l, s: syncDraft(drafts.get(l.key) ?? l.state, l.value) };
  };

  const setDraft = useCallback((v: string) => {
    const { l, s } = current();
    drafts.put(l.key, { ...s, draft: v });
  }, []);
  const onFocus = useCallback(() => {
    const { l, s } = current();
    drafts.put(l.key, withFocus(s, true, l.value));
  }, []);
  const onBlur = useCallback(() => {
    const { l, s } = current();
    drafts.put(l.key, withFocus(s, false, l.value));
  }, []);
  const leave = useCallback(() => {
    const { l, s } = current();
    const { state: next, edited } = leaveDraft(s, l.value);
    drafts.put(l.key, next);
    return edited;
  }, []);
  const edited = useCallback(() => {
    const { l, s } = current();
    return leaveDraft(s, l.value).edited;
  }, []);
  const stored = useCallback(() => target(current().s), []);
  const getState = useCallback(() => current().s, []);
  const save = useCallback(<T,>(v: string, call: () => Promise<T | undefined>, written: (result: T) => string) => {
    const { l, s } = current();
    return drafts.save(l.key, s, v, () => call().then((r) => (r === undefined ? undefined : written(r))));
  }, []);
  const adopt = useCallback((keepFocus?: boolean) => {
    const { l, s } = current();
    drafts.put(l.key, adoptStored(s, l.value, keepFocus ?? s.focused));
  }, []);

  const cancel = useCallback((keepFocus?: boolean) => {
    const { l, s } = current();
    drafts.put(l.key, cancelEdit(s, l.value, keepFocus ?? s.focused));
  }, []);

  return {
    key,
    draft: state.draft,
    setDraft,
    dirty: isDirty(state),
    saving: state.pending !== null,
    elsewhere: showsConflict(state, value),
    onFocus,
    onBlur,
    leave,
    edited,
    stored,
    state: getState,
    save,
    adoptStored: adopt,
    cancelEdit: cancel,
  };
}

/**
 * Submission guard of an explicit-save form (or a submit-on-Enter input),
 * keyed like its drafts so it survives the component being re-created:
 * `begin()` returns false while a submission is already running.
 */
export function useSubmitting(key: string) {
  const busy = useSyncExternalStore(drafts.subscribe, () => drafts.isBusy(key));
  const begin = useCallback(() => drafts.begin(key), [key]);
  const end = useCallback(() => drafts.end(key), [key]);
  const isBusy = useCallback(() => drafts.isBusy(key), [key]);
  return { busy, begin, end, isBusy };
}

/** Attribute that marks a draft's "use the file's version" button (see DraftHint). */
export const ADOPT_ATTR = "data-draft-adopt";

/**
 * True when focus is leaving a field for one of `within` (its own hint button,
 * or a sibling input of the same draft): such a blur must not autosave.
 */
export function focusStaysWith(e: FocusEvent, draft: Pick<DraftHandle, "key">, within: (Element | null)[] = []): boolean {
  const to = e.relatedTarget as Element | null;
  if (!to) return false;
  if (to.closest?.(`[${ADOPT_ATTR}]`)?.getAttribute(ADOPT_ATTR) === draft.key) return true;
  return within.some((el) => el === to);
}
