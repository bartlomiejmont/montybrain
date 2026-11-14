// Quick-add syntax for the "dodaj zadanie" inputs: trailing `!wysoki`/`!niski`
// and `@dziś`/`@jutro`/`@pojutrze`/`@YYYY-MM-DD[THH:MM]` become fields instead
// of title text (core would reject a title ending in a due token anyway).

import type { NewTask, Priority } from "../api/types";
import { addDays } from "./dates";
import { fold, oneLine } from "./text";

const PRIORITY_WORDS: Record<string, Priority> = {
  wysoki: "high",
  w: "high",
  pilne: "high",
  niski: "low",
  n: "low",
};

const RELATIVE_DAYS: Record<string, number> = { dzis: 0, jutro: 1, pojutrze: 2 };

function parseDueWord(word: string, today: string): string | null {
  const m = word.match(/^(\d{4}-\d{2}-\d{2})(?:T(\d{2}:\d{2}))?$/);
  if (m) {
    const [y, mo, d] = m[1]!.split("-").map(Number);
    const probe = new Date(y!, mo! - 1, d!);
    if (probe.getMonth() !== mo! - 1 || probe.getDate() !== d) return null;
    if (m[2]) {
      const [h, mi] = m[2].split(":").map(Number);
      if (h! > 23 || mi! > 59) return null;
    }
    return m[2] ? `${m[1]}T${m[2]}` : m[1]!;
  }
  const rel = RELATIVE_DAYS[fold(word)];
  return rel === undefined ? null : addDays(today, rel);
}

/** Parses "Zrobić MR !wysoki @jutro" into a NewTask. */
export function parseQuickAdd(input: string, today: string): NewTask {
  const words = oneLine(input).split(" ");
  let priority: Priority | undefined;
  let due: string | undefined;
  while (words.length > 1) {
    const last = words[words.length - 1]!;
    if (last.startsWith("!") && priority === undefined) {
      const p = PRIORITY_WORDS[fold(last.slice(1))];
      if (!p) break;
      priority = p;
    } else if (last.startsWith("@") && due === undefined) {
      const d = parseDueWord(last.slice(1), today);
      if (!d) break;
      due = d;
    } else {
      break;
    }
    words.pop();
  }
  const task: NewTask = { title: words.join(" ") };
  if (priority) task.priority = priority;
  if (due) task.due = due;
  return task;
}
