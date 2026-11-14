// A task's deadline as one draft string: "YYYY-MM-DD", "YYYY-MM-DDTHH:MM" or ""
// (the stored `due` form). While the user retypes the date the draft may hold
// only a time ("T14:00"), so the time is not lost on the way.

import type { DueString } from "../api/types";

export function fromDue(due: DueString | null): string {
  return due ?? "";
}

export function deadlineParts(draft: string): { date: string; time: string } {
  const [date = "", time = ""] = draft.split("T");
  return { date, time };
}

function join(date: string, time: string): string {
  return time ? `${date}T${time}` : date;
}

export function withDate(draft: string, date: string): string {
  return join(date, deadlineParts(draft).time);
}

export function withTime(draft: string, time: string): string {
  return join(deadlineParts(draft).date, time);
}

/** The `due` to store for a draft; no date means no deadline. */
export function deadlineToSave(draft: string): DueString | null {
  const { date, time } = deadlineParts(draft);
  return date ? join(date, time) : null;
}
