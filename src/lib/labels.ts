// Polish UI labels for the core enums.

import type { Priority, ProjectState, TaskStatus, TodayReason, TopicStatus } from "../api/types";

export const TASK_STATUSES: TaskStatus[] = ["todo", "doing", "done", "blocked"];

export const TASK_STATUS_LABEL: Record<TaskStatus, string> = {
  todo: "Do zrobienia",
  doing: "W toku",
  done: "Zrobione",
  blocked: "Zablokowane",
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
  high: "Wysoki",
  normal: "Normalny",
  low: "Niski",
};

export const TOPIC_STATUSES: TopicStatus[] = ["planned", "in_progress", "blocked", "waiting", "done"];

export const TOPIC_STATUS_LABEL: Record<TopicStatus, string> = {
  planned: "Planowany",
  in_progress: "W toku",
  blocked: "Zablokowany",
  waiting: "Czeka",
  done: "Zrobiony",
};

export const PROJECT_STATES: ProjectState[] = ["active", "paused", "archived"];

export const PROJECT_STATE_LABEL: Record<ProjectState, string> = {
  active: "Aktywny",
  paused: "Wstrzymany",
  archived: "Archiwum",
};

export const TODAY_REASONS: TodayReason[] = ["overdue", "due_today", "doing", "blocked"];

export const TODAY_REASON_LABEL: Record<TodayReason, string> = {
  overdue: "Po terminie",
  due_today: "Na dziś",
  doing: "W toku",
  blocked: "Zablokowane",
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
