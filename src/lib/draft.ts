// Draft preservation. A draft is the text the user is editing; `base` is the
// stored value it was started from (or the value our last acknowledged save
// wrote). When the file changes on disk (`incoming`) a clean, unfocused draft
// just follows it; a dirty or focused one is never overwritten.
//
// Saves are acknowledged by their own result (`ackSave` with the operation id
// and the value the backend reports as written), never inferred from "the next
// snapshot that differs": an agent can write the same file at any moment.
//
// Writes of one field are serialised: at most one is in flight; a value the
// user asks to save meanwhile is queued (the latest one wins) and sent once
// the result of the one in flight arrived (`takeQueued`). What the user changed
// is judged against `target`: what the field will hold once our writes land.

export interface PendingSave {
  /** Operation id, increasing per store. */
  op: number;
  /** The value sent to the backend. */
  value: string;
}

export interface DraftState {
  /** The stored value the draft was started from. */
  base: string;
  /** What the user sees in the field. */
  draft: string;
  /** The save in flight, until its own result comes back. */
  pending: PendingSave | null;
  /** A value to save after the one in flight (only while `pending`). */
  queued: string | null;
  /** Highest operation whose successful result has been applied. */
  acked: number;
  focused: boolean;
}

export function freshDraft(value: string): DraftState {
  return { base: value, draft: value, pending: null, queued: null, acked: 0, focused: false };
}

export function isDirty(s: DraftState): boolean {
  return s.draft !== s.base;
}

/** What the field will hold on disk once the writes we sent (or queued) land. */
export function target(s: DraftState): string {
  return s.queued ?? s.pending?.value ?? s.base;
}

/** Nothing typed that is not stored, nothing in flight. */
export function isSettled(s: DraftState): boolean {
  return !isDirty(s) && !s.pending && s.queued === null;
}

/** Applies a (possibly new) stored value. Returns `prev` itself when nothing changes. */
export function syncDraft(prev: DraftState, incoming: string): DraftState {
  if (incoming === prev.base) return prev;
  // While a save is in flight a new stored value may be ours or someone
  // else's; only the save's own result can tell. Keep everything until then.
  if (prev.pending) return prev;
  // The file now says what the field says: nothing to keep apart any more.
  if (incoming === prev.draft) return { ...prev, base: incoming };
  if (prev.focused || isDirty(prev)) return prev;
  return { ...prev, base: incoming, draft: incoming };
}

/** The stored value differs both from what the draft started from and from the draft. */
export function changedElsewhere(s: DraftState, incoming: string): boolean {
  return incoming !== s.base && incoming !== s.draft;
}

/** Show the "changed outside the app" hint (not while our own save is still unanswered). */
export function showsConflict(s: DraftState, incoming: string): boolean {
  return !s.pending && (isDirty(s) || s.focused) && changedElsewhere(s, incoming);
}

export function withFocus(s: DraftState, focused: boolean, incoming: string): DraftState {
  return syncDraft({ ...s, focused }, incoming);
}

/**
 * The user leaves the field. Returns the reconciled state (an untouched field
 * now shows the stored value) and the text to save: the draft when it differs
 * from what will be stored once the writes in flight land (`target`),
 * otherwise null. Text already being saved is not offered again; text changed
 * back to the stored value while a different one is being saved is.
 */
export function leaveDraft(s: DraftState, incoming: string): { state: DraftState; edited: string | null } {
  const state = withFocus(s, false, incoming);
  const edited = state.draft !== target(state) ? state.draft : null;
  return { state, edited };
}

/**
 * The user asks to save `value` (the draft, possibly normalised by the caller).
 * `send` is true when it must be sent now (then call `startSave`); while a save
 * is in flight it is queued instead, replacing an older queued value. A value
 * equal to what will be stored anyway is not sent.
 */
export function requestSave(s: DraftState, value: string): { state: DraftState; send: boolean } {
  const state = { ...s, draft: value };
  if (s.pending) return { state: { ...state, queued: value === s.pending.value ? null : value }, send: false };
  return { state: { ...state, queued: null }, send: value !== s.base };
}

/** A save of `value` was sent as operation `op` (the draft is left as it is). */
export function startSave(s: DraftState, op: number, value: string): DraftState {
  return { ...s, pending: { op, value } };
}

/**
 * The result of operation `op` arrived: `written` is the field's value as the
 * backend wrote it, `undefined` means the save failed (the draft stays).
 */
export function ackSave(s: DraftState, op: number, written: string | undefined): DraftState {
  if (op <= s.acked) return s;
  const mine = s.pending?.op === op;
  if (written === undefined) return mine ? { ...s, pending: null } : s;
  // Our value landed (possibly normalised): it becomes the base. Text typed
  // after the save was sent is kept.
  const draft = mine && s.draft === s.pending!.value ? written : s.draft;
  return { ...s, base: written, draft, pending: mine ? null : s.pending, acked: op };
}

/**
 * After a result: the queued value to send now (then call `startSave`), or
 * null. A queued value that is already stored is dropped.
 */
export function takeQueued(s: DraftState): { state: DraftState; send: string | null } {
  if (s.pending || s.queued === null) return { state: s, send: null };
  const state = { ...s, queued: null };
  return { state, send: s.queued === s.base ? null : s.queued };
}

/** Throw the draft away and take the stored value. */
export function discardDraft(incoming: string, focused = false): DraftState {
  return { ...freshDraft(incoming), focused };
}

/**
 * The user takes the stored value instead of the draft. While a save is in
 * flight that is the value being saved (it stays tracked); otherwise the file's.
 */
export function adoptStored(s: DraftState, incoming: string, focused: boolean): DraftState {
  if (s.pending) return { ...s, draft: s.pending.value, queued: null, focused };
  return { ...discardDraft(incoming, focused), acked: s.acked };
}

/**
 * The user cancels the current edit (Escape, or an invalid value such as an
 * emptied title). Only the text typed in this editing session goes: writes
 * already in flight or queued were submitted earlier and stay, and the field
 * shows what they will store (`target`). With nothing in flight it is the
 * file's value.
 */
export function cancelEdit(s: DraftState, incoming: string, focused: boolean): DraftState {
  if (s.pending) return { ...s, draft: target(s), focused };
  return { ...discardDraft(incoming, focused), acked: s.acked };
}
