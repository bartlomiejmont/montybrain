import { t as tr } from "../i18n";
// The command field's decisions, pure (no React, no timers, no backend):
// what Enter, Tab, the arrows, Esc and the mouse do, given the text on screen
// and the readings that came back for it. CommandField runs the effects.
//
// Rules:
// * Enter always confirms the reading on screen: the first Enter shows the
//   plan, the second writes (plain capture writes at once). It accepts a
//   suggestion only when the user moved the highlight (arrows or mouse), or
//   when the text is still being typed (`typing`: then the first row is
//   highlighted and Enter takes it).
// * Tab accepts a suggestion (the highlighted one, else the first).
// * One list at a time, all handled alike: the suggestions for the word under
//   the caret (places after ` > `, `/` commands, tasks and topics by name…),
//   else the choices of the reading (several places or items fit, a typo).
//   ↑/↓ move, Enter/Tab accept, the mouse highlights what it hovers and takes
//   what it clicks, Esc closes the list first and steps back second.
// * A suggestion equal to the word already typed is not offered.
// * Readings belong to the text they were computed for, suggestions to the
//   text and caret: an older answer is never applied to newer text. Until the
//   new one arrives the old one stays on screen, dimmed and inert (no
//   flicker, no jump).
// * Enter before the reading of the current text arrived is not dropped: it
//   asks for that reading at once and acts when it comes.

import type { AgentAsk, CommandPlan, CommandPreview, Completions, Span } from "../api/types";

export interface FieldState {
  text: string;
  caret: number;
  /** Bumped on every change of text or caret. */
  rev: number;
  /** The last reading (it may belong to older text: see `readingShown`). */
  preview: CommandPreview | null;
  /** The last suggestions and the revision (text + caret) they belong to. */
  completions: { rev: number; value: Completions | null } | null;
  /** Index the user moved to with the arrows or the mouse; null: not moved. */
  highlight: number | null;
  /** Esc closed the list for this text. */
  closed: boolean;
  phase: "edit" | "confirm";
  /** Enter was pressed; waiting for the reading of the current text. */
  waiting: boolean;
  busy: boolean;
}

/** One row of the list under the field: a suggestion or a choice. */
export interface ListItem {
  key: string;
  label: string;
  /** Matched parts of `label` (UTF-16 ranges). */
  marks: Span[];
  detail: string;
  detailMarks: Span[];
  /** Replaces `span` of the text. */
  insert: string;
  span: Span;
  /** `project`, `topic`, `task`, `inbox`, `command`, `date`… */
  kind: string;
  typo: boolean;
}

export interface FieldList {
  /** `completions`: for the word under the caret; `choices`: of the reading. */
  source: "completions" | "choices";
  items: ListItem[];
  /** Belongs to older text: shown dimmed, keys and clicks do nothing. */
  stale: boolean;
}

export type FieldEvent =
  | { type: "edit"; text: string; caret: number }
  | { type: "caret"; caret: number }
  | { type: "reading"; rev: number; text: string; preview: CommandPreview; completions: Completions | null }
  | { type: "arrow"; dir: 1 | -1 }
  | { type: "hover"; index: number }
  | { type: "pick"; index: number }
  | { type: "tab" }
  | { type: "enter" }
  | { type: "escape" }
  | { type: "written" }
  | { type: "failed"; keepConfirm?: boolean }
  | { type: "stale"; preview: CommandPreview };

export type FieldEffect =
  | { kind: "fetch"; rev: number; text: string; caret: number; now: boolean }
  | { kind: "apply"; plan: CommandPlan; text: string }
  | { kind: "agent"; ask: AgentAsk; text: string }
  | { kind: "problem"; message: string }
  | { kind: "escape" }
  | { kind: "arrowOut" }
  | { kind: "caret"; caret: number };

export function initialField(text = "", phase: "edit" | "confirm" = "edit"): FieldState {
  return { text, caret: text.length, rev: 0, preview: null, completions: null, highlight: null, closed: false, phase, waiting: false, busy: false };
}

/** The reading of the text on screen, if it arrived. */
export function currentReading(s: FieldState): CommandPreview | null {
  return s.preview && s.preview.text === s.text ? s.preview : null;
}

/** The reading to show: the current one, else the last one (dimmed) while the new one is on its way. */
export function readingShown(s: FieldState): { preview: CommandPreview; stale: boolean } | null {
  if (!s.preview) return null;
  return { preview: s.preview, stale: s.preview.text !== s.text };
}

const PLACE_KINDS = new Set(["project", "topic", "inbox"]);

/** The grey text right of a suggestion: what gets inserted for a place, else the backend's detail. */
function detailOf(i: Completions["items"][number], kind: string): { text: string; marks: Span[] } {
  const place = PLACE_KINDS.has(kind);
  if (place && (i.detail === "projekt" || i.detail === "temat" || i.detail === "bez projektu")) return { text: i.insert !== i.label ? i.insert : "", marks: [] };
  if (i.insert !== i.label && !place && kind !== "task" && kind !== "topic") return { text: [i.insert.trim(), i.detail].filter(Boolean).join(" · "), marks: [] };
  return { text: i.detail, marks: i.detail_marks ?? [] };
}

function fromCompletions(c: Completions): ListItem[] {
  return c.items.map((i, n) => {
    const kind = i.kind ?? c.kind;
    const d = detailOf(i, kind);
    return {
      key: `c:${n}:${i.insert}`,
      label: i.label,
      marks: i.marks ?? [],
      detail: d.text,
      detailMarks: d.marks,
      insert: i.insert,
      span: c.span,
      kind,
      typo: false,
    };
  });
}

function fromChoices(p: CommandPreview): ListItem[] {
  return p.choices.map((c, n) => ({
    key: `x:${n}:${c.choice.insert}`,
    label: c.choice.label,
    marks: c.marks ?? [],
    detail: c.choice.insert.trim(),
    detailMarks: [],
    insert: c.choice.insert,
    span: c.span,
    kind: c.choice.kind,
    typo: c.choice.typo,
  }));
}

/** The list under the field, if any (possibly stale: see `FieldList.stale`). */
export function fieldList(s: FieldState): FieldList | null {
  if (s.closed || s.busy) return null;
  const c = s.completions;
  if (s.phase === "edit" && c?.value) {
    // A suggestion equal to the word typed is no suggestion.
    const typed = s.text.slice(c.value.span.from, c.value.span.to);
    const items = fromCompletions(c.value).filter((i) => i.insert.trim() !== typed);
    if (items.length > 0) return { source: "completions", items, stale: c.rev !== s.rev };
  }
  const p = s.preview;
  if (p && p.choices.length > 0) return { source: "choices", items: fromChoices(p), stale: p.text !== s.text };
  return null;
}

/** The rows keys and clicks act on: the list, when it is not stale. */
export function visibleItems(s: FieldState): ListItem[] {
  const l = fieldList(s);
  return l && !l.stale ? l.items : [];
}

/** The highlighted row: where the user moved, else the first while typing. */
export function activeIndex(s: FieldState): number | null {
  const items = visibleItems(s);
  if (items.length === 0) return null;
  if (s.highlight !== null && s.highlight < items.length) return s.highlight;
  return currentReading(s)?.state === "typing" ? 0 : null;
}

function edited(s: FieldState, text: string, caret: number, now = false): { state: FieldState; effects: FieldEffect[] } {
  const rev = s.rev + 1;
  const textChanged = text !== s.text;
  return {
    state: {
      ...s,
      text,
      caret,
      rev,
      highlight: null,
      closed: textChanged ? false : s.closed,
      phase: textChanged ? "edit" : s.phase,
      // Typing after Enter cancels that Enter: it was about older text.
      waiting: textChanged ? false : s.waiting,
    },
    effects: [{ kind: "fetch", rev, text, caret, now }],
  };
}

function accept(s: FieldState, item: ListItem): { state: FieldState; effects: FieldEffect[] } {
  const len = s.text.length;
  // A choice at the very end appends (e.g. ` > projekt` after `/temat Nazwa`).
  const from = Math.min(item.span.from, len);
  const to = Math.min(item.span.to, len);
  const text = s.text.slice(0, from) + item.insert + s.text.slice(to);
  const caret = from + item.insert.length;
  const out = edited(s, text, caret);
  out.effects.push({ kind: "caret", caret });
  return out;
}

/** Enter on the reading `p` of the text on screen. */
function decide(s: FieldState, p: CommandPreview): { state: FieldState; effects: FieldEffect[] } {
  switch (p.state) {
    case "empty":
      return { state: s, effects: [] };
    case "capture":
      return p.plan ? { state: { ...s, busy: true }, effects: [{ kind: "apply", plan: p.plan, text: s.text }] } : { state: s, effects: [] };
    case "confirm":
      if (s.phase === "edit") return { state: { ...s, phase: "confirm" }, effects: [] };
      return p.plan ? { state: { ...s, busy: true }, effects: [{ kind: "apply", plan: p.plan, text: s.text }] } : { state: s, effects: [] };
    case "agent":
      return p.agent ? { state: s, effects: [{ kind: "agent", ask: p.agent, text: s.text }] } : { state: s, effects: [] };
    case "typing": {
      // Still being typed: Enter takes the highlighted row (the first by default).
      const items = visibleItems(s);
      const at = activeIndex(s);
      if (at !== null && items[at]) return accept(s, items[at]!);
      return { state: s, effects: [{ kind: "problem", message: p.problems[0] ?? p.reading }] };
    }
    case "invalid":
      return { state: s, effects: [{ kind: "problem", message: p.problems[0] ?? tr("Tego nie da się zapisać.") }] };
  }
}

export function stepField(s: FieldState, e: FieldEvent): { state: FieldState; effects: FieldEffect[] } {
  switch (e.type) {
    case "edit":
      if (e.text === s.text && e.caret === s.caret) return { state: s, effects: [] };
      return edited(s, e.text, e.caret);
    case "caret":
      if (e.caret === s.caret) return { state: s, effects: [] };
      return edited(s, s.text, e.caret);
    case "reading": {
      // A reading of older text is dropped; suggestions only for this exact revision.
      if (e.text !== s.text) return { state: s, effects: [] };
      const state: FieldState = {
        ...s,
        preview: e.preview,
        completions: e.rev === s.rev ? { rev: e.rev, value: e.completions } : s.completions,
      };
      if (s.waiting) return decide({ ...state, waiting: false }, e.preview);
      return { state, effects: [] };
    }
    case "arrow": {
      const l = fieldList(s);
      if (l?.stale) return { state: s, effects: [] }; // the fresh list is on its way
      const items = visibleItems(s);
      if (items.length === 0) return { state: s, effects: e.dir === 1 ? [{ kind: "arrowOut" }] : [] };
      const from = activeIndex(s);
      const at = from === null ? (e.dir === 1 ? 0 : items.length - 1) : (from + e.dir + items.length) % items.length;
      return { state: { ...s, highlight: at }, effects: [] };
    }
    case "hover": {
      const items = visibleItems(s);
      if (!items[e.index] || s.highlight === e.index) return { state: s, effects: [] };
      return { state: { ...s, highlight: e.index }, effects: [] };
    }
    case "pick": {
      const items = visibleItems(s);
      if (!items[e.index] || s.busy) return { state: s, effects: [] };
      return accept(s, items[e.index]!);
    }
    case "tab": {
      const items = visibleItems(s);
      if (items.length === 0) return { state: s, effects: [] };
      return accept(s, items[activeIndex(s) ?? 0]!);
    }
    case "enter": {
      if (s.busy) return { state: s, effects: [] };
      const items = visibleItems(s);
      if (s.highlight !== null && items[s.highlight]) return accept(s, items[s.highlight]!);
      const p = currentReading(s);
      if (p) return decide(s, p);
      // The reading of this text is not here yet: ask for it now, act when it comes.
      return { state: { ...s, waiting: true }, effects: [{ kind: "fetch", rev: s.rev, text: s.text, caret: s.caret, now: true }] };
    }
    case "escape":
      if (fieldList(s)) return { state: { ...s, closed: true, highlight: null }, effects: [] };
      if (s.phase === "confirm") return { state: { ...s, phase: "edit" }, effects: [] };
      return { state: s, effects: [{ kind: "escape" }] };
    case "written": {
      const out = edited({ ...s, busy: false, phase: "edit", preview: null, completions: null }, "", 0, true);
      return out;
    }
    case "failed":
      return { state: { ...s, busy: false, phase: e.keepConfirm ? s.phase : "edit" }, effects: [] };
    case "stale":
      return { state: { ...s, busy: false, preview: e.preview, phase: "confirm" }, effects: [] };
  }
}
