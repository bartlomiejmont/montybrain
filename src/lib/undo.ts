// What the app says and offers after an undo, and when `u` means "undo" in
// Skrzynka. Pure, so the decisions are testable without a DOM.

import type { CommandUndoReport } from "../api/types";

export interface UndoView {
  text: string;
  /** Token to retry with (some steps failed and stay undoable), else null. */
  retry: number | null;
  /** Something failed: keep the window / control open. */
  failed: boolean;
}

export function describeUndo(r: CommandUndoReport, token: number | null): UndoView {
  const failed = r.items.filter((i) => i.status === "failed").length;
  const left = r.items.filter((i) => i.status !== "undone").length;
  if (failed > 0) {
    return { text: `Nie udało się cofnąć wszystkiego w „${r.reading}” (${failed}).`, retry: r.retry ?? token, failed: true };
  }
  if (left > 0) return { text: `Cofnięto częściowo „${r.reading}”: ${left} zmieniło się w międzyczasie i zostało.`, retry: null, failed: false };
  return { text: `Cofnięto: ${r.reading}`, retry: null, failed: false };
}

/** `u` in Skrzynka: a plain key press, not while typing, not with a dialog open. */
export function isUndoKey(e: { key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; defaultPrevented: boolean }, typing: boolean, dialogOpen: boolean): boolean {
  return e.key === "u" && !e.metaKey && !e.ctrlKey && !e.altKey && !e.defaultPrevented && !typing && !dialogOpen;
}
