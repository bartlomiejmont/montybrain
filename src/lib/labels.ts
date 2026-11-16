import { t as tr } from "../i18n";
// Polish UI labels for the core enums.

import type { Priority, ProjectState, TaskStatus, TodayReason, TopicStatus } from "../api/types";

export const TASK_STATUSES: TaskStatus[] = ["todo", "doing", "done", "blocked"];

export const TASK_STATUS_LABEL: Record<TaskStatus, string> = {
  get todo() { return tr("Do zrobienia"); },
  get doing() { return tr("W toku"); },
  get done() { return tr("Zrobione"); },
  get blocked() { return tr("Zablokowane"); },
};

/** Keyboard shortcut (on a focused task row) per status. */
export const TASK_STATUS_KEY: Record<TaskStatus, string> = {
  todo: "1",
  doing: "2",
  done: "3",
  blocked: "4",
};

export const PRIORITIES: Priority[] = ["high", "normal", "low"];

export const PRIORITY_LABEL: Record<Priority, string> = {
  get high() { return tr("Wysoki"); },
  get normal() { return tr("Normalny"); },
  get low() { return tr("Niski"); },
};

export const TOPIC_STATUSES: TopicStatus[] = ["planned", "in_progress", "blocked", "waiting", "done"];

export const TOPIC_STATUS_LABEL: Record<TopicStatus, string> = {
  get planned() { return tr("Planowany"); },
  get in_progress() { return tr("W toku"); },
  get blocked() { return tr("Zablokowany"); },
  get waiting() { return tr("Czeka"); },
  get done() { return tr("Zrobiony"); },
};

export const PROJECT_STATES: ProjectState[] = ["active", "paused", "archived"];

export const PROJECT_STATE_LABEL: Record<ProjectState, string> = {
  get active() { return tr("Aktywny"); },
  get paused() { return tr("Wstrzymany"); },
  get archived() { return tr("Archiwum"); },
};

export const TODAY_REASONS: TodayReason[] = ["overdue", "due_today", "doing", "blocked"];

export const TODAY_REASON_LABEL: Record<TodayReason, string> = {
  get overdue() { return tr("Po terminie"); },
  get due_today() { return tr("Na dziś"); },
  get doing() { return tr("W toku"); },
  get blocked() { return tr("Zablokowane"); },
};

/** Muted, distinguishable project colours for the picker. */
export const PROJECT_COLORS = [
  "#a3e635",
  "#4ade80",
  "#2dd4bf",
  "#60a5fa",
  "#818cf8",
  "#c084fc",
  "#f472b6",
  "#f87171",
  "#fb923c",
  "#facc15",
];
