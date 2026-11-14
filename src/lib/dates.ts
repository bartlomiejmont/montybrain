// Local-date helpers. Dates travel as strings ("YYYY-MM-DD", "YYYY-MM-DDTHH:MM");
// arithmetic is done on plain calendar dates, never on UTC instants.

import type { DueString, IsoDate, IsoMinute } from "../api/types";

const pad = (n: number) => String(n).padStart(2, "0");

export function toIsoDate(d: Date): IsoDate {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

export function toIsoMinute(d: Date): IsoMinute {
  return `${toIsoDate(d)}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** Calendar date → local Date at midnight. */
export function parseDate(s: IsoDate): Date {
  const [y, m, d] = s.split("-").map(Number);
  return new Date(y ?? 1970, (m ?? 1) - 1, d ?? 1);
}

export function addDays(s: IsoDate, n: number): IsoDate {
  const d = parseDate(s);
  d.setDate(d.getDate() + n);
  return toIsoDate(d);
}

/** Whole days from `a` to `b` (b - a). */
export function daysBetween(a: IsoDate, b: IsoDate): number {
  return Math.round((parseDate(b).getTime() - parseDate(a).getTime()) / 86_400_000);
}

export interface DueParts {
  date: IsoDate;
  time: string | null;
}

export function splitDue(due: DueString): DueParts {
  const [date, time] = due.split("T");
  return { date: date ?? due, time: time ?? null };
}

export function joinDue(date: IsoDate, time: string | null | undefined): DueString {
  return time ? `${date}T${time}` : date;
}

const fmtDayMonth = new Intl.DateTimeFormat("pl-PL", { day: "numeric", month: "short" });
const fmtDayMonthYear = new Intl.DateTimeFormat("pl-PL", { day: "numeric", month: "short", year: "numeric" });
const fmtWeekdayShort = new Intl.DateTimeFormat("pl-PL", { weekday: "short" });
const fmtLong = new Intl.DateTimeFormat("pl-PL", { weekday: "long", day: "numeric", month: "long" });

export function shortDate(date: IsoDate, today: IsoDate): string {
  const d = parseDate(date);
  const sameYear = date.slice(0, 4) === today.slice(0, 4);
  return (sameYear ? fmtDayMonth : fmtDayMonthYear).format(d).replace(/\.$/, "");
}

/** "dziś", "jutro", "wczoraj", "pt", "20 lis" (+ " 14:00"). */
export function formatDue(due: DueString, today: IsoDate): string {
  const { date, time } = splitDue(due);
  const diff = daysBetween(today, date);
  let label: string;
  if (diff === 0) label = "dziś";
  else if (diff === 1) label = "jutro";
  else if (diff === -1) label = "wczoraj";
  else if (diff > 1 && diff < 7) label = fmtWeekdayShort.format(parseDate(date)).replace(/\.$/, "");
  else label = shortDate(date, today);
  return time ? `${label} ${time}` : label;
}

/** Overdue in core's sense: the due date is before today. */
export function isOverdue(due: DueString | null, today: IsoDate): boolean {
  return due !== null && splitDue(due).date < today;
}

/** "14:02" today, "wczoraj 09:30", otherwise "12 lis 09:30". */
export function formatStamp(at: IsoMinute, today: IsoDate): string {
  const [date = at, time = ""] = at.split("T");
  const diff = daysBetween(today, date);
  if (diff === 0) return time;
  if (diff === -1) return `wczoraj ${time}`;
  return `${shortDate(date, today)} ${time}`.trim();
}

/** Coarse age for "updated" labels: "dziś", "wczoraj", "3 dni temu", "12 lis". */
export function formatAge(at: IsoMinute | IsoDate, today: IsoDate): string {
  const date = at.slice(0, 10);
  const diff = daysBetween(date, today);
  if (diff <= 0) return "dziś";
  if (diff === 1) return "wczoraj";
  if (diff < 7) return `${diff} dni temu`;
  return shortDate(date, today);
}

/** "piątek, 13 listopada" */
export function formatLongDate(today: IsoDate): string {
  return fmtLong.format(parseDate(today));
}
