// Mirrors of the JSON shapes produced by `montybrain-core` (see docs/core-api.md)
// and by the app's own commands (src-tauri/src/state.rs). One place, on purpose.

/** "YYYY-MM-DD" */
export type IsoDate = string;
/** "YYYY-MM-DDTHH:MM" (minute precision, local time) */
export type IsoMinute = string;
/** "YYYY-MM-DD" or "YYYY-MM-DDTHH:MM" */
export type DueString = string;

export type TaskStatus = "todo" | "doing" | "done" | "blocked";
export type Priority = "high" | "normal" | "low";
export type ProjectState = "active" | "paused" | "archived";
export type TopicStatus = "planned" | "in_progress" | "blocked" | "waiting" | "done";
export type TodayReason = "overdue" | "due_today" | "doing" | "blocked";

export type TaskLocation =
  | { kind: "inbox" }
  | { kind: "project"; project: string }
  | { kind: "topic"; project: string; topic: string };

export interface Task {
  id: string | null;
  title: string;
  status: TaskStatus;
  priority: Priority;
  due: DueString | null;
  done: IsoDate | null;
  links: string[];
  notes: string;
  location: TaskLocation;
  line: number;
}

export interface Repo {
  name: string;
  path: string;
}

export interface Project {
  slug: string;
  name: string;
  color: string | null;
  state: ProjectState;
  repos: Repo[];
  description: string;
  path: string;
}

export interface Topic {
  project: string;
  slug: string;
  title: string;
  status: TopicStatus;
  repo: string | null;
  summary: string;
  created: IsoDate | null;
  updated: IsoMinute | null;
  path: string;
}

export interface JournalEntry {
  project: string;
  topic: string;
  at: IsoMinute;
  author: string;
  text: string;
  line: number;
}

export interface LoadWarning {
  path: string;
  line: number | null;
  message: string;
}

export interface Snapshot {
  root: string;
  projects: Project[];
  topics: Topic[];
  tasks: Task[];
  journal: JournalEntry[];
  warnings: LoadWarning[];
}

export interface TodayItem {
  reason: TodayReason;
  task: Task;
}

export type SearchHit =
  | { kind: "task"; task: Task }
  | { kind: "topic"; topic: Topic }
  | { kind: "journal"; entry: JournalEntry };

/** `get_state` result (src-tauri/src/state.rs `AppView`). */
export interface AppView {
  version: number;
  today: IsoDate;
  now: IsoMinute;
  snapshot: Snapshot;
  today_items: TodayItem[];
  recent_journal: JournalEntry[];
  load_error: string | null;
}

/** Payload of the `vault-changed` event. */
export interface VaultChanged {
  version: number;
  source: "watcher" | "mutation";
  paths: string[];
}

// --- mutation inputs (all fields optional on the Rust side via #[serde(default)]) ---

export interface NewProject {
  name: string;
  color?: string | null;
  state?: ProjectState;
  repos?: Repo[];
  description?: string;
}

/** Absent key = keep; `null` on `color` = clear. */
export interface ProjectPatch {
  name?: string;
  color?: string | null;
  state?: ProjectState;
  repos?: Repo[];
  description?: string;
}

export interface NewTopic {
  title: string;
  status?: TopicStatus;
  repo?: string | null;
  summary?: string;
}

/** Absent key = keep; `null` on `repo` = clear. */
export interface TopicPatch {
  title?: string;
  status?: TopicStatus;
  repo?: string | null;
  summary?: string;
}

export interface NewJournalEntry {
  author?: string;
  text: string;
  status?: TopicStatus | null;
  summary?: string | null;
}

export interface JournalAdded {
  topic: Topic;
  entry: JournalEntry;
}

export interface NewTask {
  title: string;
  status?: TaskStatus;
  priority?: Priority;
  due?: DueString | null;
  links?: string[];
  notes?: string;
}

/** Absent key = keep; `null` on `due` = clear; `notes: ""` clears notes. */
export interface TaskPatch {
  title?: string;
  status?: TaskStatus;
  priority?: Priority;
  due?: DueString | null;
  add_links?: string[];
  remove_links?: string[];
  notes?: string;
}

// --- app settings (src-tauri/src/native.rs `SettingsView`, settings.rs) ---

/** `get_settings` result and `settings-changed` payload. */
export interface AppSettings {
  /** Global quick-capture shortcut, e.g. "Ctrl+Alt+Space" (see api/shortcut.ts). */
  shortcut: string;
  /** Why the saved shortcut is not active (taken by another app, …). */
  shortcut_error: string | null;
  /** Due-time reminders + daily digest on/off. */
  reminders: boolean;
  /** Hour (0–23) of the daily digest. */
  digest_hour: number;
  /** Start at login (hidden, in the menu bar). */
  autostart: boolean;
  autostart_error: string | null;
  /** Last destination of the quick-capture window. */
  quick_destination: TaskLocation;
  /** Automatic project update per project slug (entries that are not the default "off"). */
  auto_update: Record<string, AutoUpdate>;
  /** Update sources per project slug (entries that are not the default: repositories only). */
  update_sources: Record<string, UpdateSources>;
  /** Model and effort per agent (`null` = the CLI's default). */
  agents: Agents;
  /** Model names to suggest (Claude: aliases; Codex: its own model catalog). */
  agent_suggestions: { claude: string[]; codex: string[] };
  /** Default days between checks of a waiting item (1, 2, 3 or 7). */
  waiting_days: number;
  /** Absolute vault folder (read-only). */
  vault_path: string;
  /** Absolute folder of settings.json / reminders.json. */
  config_path: string;
  /** App version. */
  version: string;
}

/** Absent key = keep. */
export interface SettingsPatch {
  reminders?: boolean;
  digest_hour?: number;
  quick_destination?: TaskLocation;
  /** Replaces both agents' model / effort (validated by the backend). */
  agents?: Agents;
  waiting_days?: number;
}

export interface AgentChoice {
  model: string | null;
  effort: string | null;
}

export interface Agents {
  claude: AgentChoice;
  codex: AgentChoice;
}

/** Effort values per provider (src-tauri/src/runs/agent.rs). */
export const AGENT_EFFORTS: Record<"claude" | "codex", string[]> = {
  claude: ["low", "medium", "high", "xhigh", "max"],
  codex: ["low", "medium", "high", "xhigh", "max", "ultra"],
};

/** Per-project sources of the update (settings.json `update_sources`). */
export interface UpdateSources {
  /** Conversation history of Claude Code and Codex (off by default; needs consent). */
  history: boolean;
  /** 7, 14 or 30. */
  history_days: number;
}

// --- agent actions („Akcje"); contract shared with the Rust backend (snake_case JSON) ---

export type ProviderId = "claude" | "codex";
export type RunMode = "background" | "terminal";
export type RunStatus = "running" | "succeeded" | "failed" | "cancelled";

export interface ProviderInfo {
  id: ProviderId;
  name: string;
  available: boolean;
  path: string | null;
  version: string | null;
  problem: string | null;
}

export interface RecipeParam {
  key: string;
  label: string;
  /** project → slug or "" (all active); period → "dzis" | "tydzien"; text → free text. */
  kind: "project" | "period" | "text";
  required: boolean;
  placeholder: string | null;
  default: string | null;
}

export interface Recipe {
  id: string;
  title: string;
  description: string;
  skill: string;
  modes: RunMode[];
  /** In the background the agent writes to MontyBrain without asking. */
  background_writes: boolean;
  /** The background run proposes changes that are written after review (needs a project with repositories). */
  proposes: boolean;
  /** The project must have repositories (the update compares them with the topics). */
  needs_repos: boolean;
  params: RecipeParam[];
}

export interface RunRequest {
  recipe: string;
  provider: ProviderId;
  mode: RunMode;
  params: Record<string, string>;
}

export interface RunEvent {
  run_id: string;
  seq: number;
  /** "YYYY-MM-DDTHH:MM:SS", local time */
  at: string;
  kind: "note" | "text" | "tool" | "error";
  text: string;
}

export interface RunInfo {
  id: string;
  recipe: string;
  recipe_title: string;
  provider: ProviderId;
  mode: RunMode;
  /** The run's `project` parameter, if any. */
  project: string | null;
  /** The request of a `zlecenie` run (its `text` parameter). */
  request?: string | null;
  /** Model / effort requested in Settings (`null` = the CLI's default). */
  model: string | null;
  effort: string | null;
  status: RunStatus;
  started_at: string;
  finished_at: string | null;
  result: string | null;
  error: string | null;
  events: RunEvent[];
}

// --- project updates („Aktualizacja"; src-tauri/src/updates/, runs/proposal.rs) ---

export type UpdateSchedule = { kind: "off" } | { kind: "every"; hours: number } | { kind: "daily"; hour: number };

/** Per-project automatic update (settings.json `auto_update`); default off. */
export interface AutoUpdate {
  schedule: UpdateSchedule;
  provider: ProviderId;
  /** "propose": keep for review (default); "apply": apply every valid change at once. */
  on_result: "propose" | "apply";
}

/** One validated change; `from` / `base_*` record the state it was based on. */
export type ProposedChange =
  | { kind: "task_status"; task: string; title: string; from: TaskStatus; to: TaskStatus; reason: string }
  | {
      kind: "topic_log";
      topic: string;
      topic_title: string;
      text: string;
      status: TopicStatus | null;
      summary: string | null;
      base_status: TopicStatus;
      base_summary: string;
      /** Branches of the topic named by the agent (stored as links on apply). */
      branches?: BranchRef[];
    }
  | { kind: "new_task"; title: string; topic: string | null; topic_title: string | null; reason: string }
  /** A new topic ("epic") with its first journal entry and first tasks, written as one file. */
  | {
      kind: "new_topic";
      title: string;
      /** The slug it will get. */
      slug: string;
      status: TopicStatus;
      /** "Where we are". */
      summary: string;
      /** Main repository (a repository name of the project). */
      repo: string | null;
      /** First journal entry (the evidence). */
      text: string;
      tasks: { title: string; status: TaskStatus }[];
      reason: string;
      branches?: BranchRef[];
    };

/** A branch of one of the project's repositories. */
export interface BranchRef {
  repo: string;
  branch: string;
}

// --- drift (src-tauri/src/drift/) ---

export type MatchKind = "link" | "mention" | "words";

export interface DriftBranch {
  repo: string;
  branch: string;
  /** Date of its last commit (YYYY-MM-DD). */
  last_commit: string;
  worktree: string | null;
  topic: string | null;
  matched_by: MatchKind | null;
  active: boolean;
  ignored: boolean;
  /** When the app last saw it. */
  seen: string | null;
}

export interface ProjectDrift {
  project: string;
  collected_at: IsoMinute | null;
  branches: DriftBranch[];
  /** Active, unmatched, not ignored. */
  unknown: DriftBranch[];
  spread: { topic: string; branches: number; repos: number }[];
  moved: { topic: string; repo: string; branch: string; last_commit: string; last_entry: string | null }[];
  new_commits: number | null;
  since: IsoMinute | null;
  links: { project: string; repo: string; branch: string; topic: string }[];
}

// --- waiting items (src-tauri/src/waiting.rs) ---

export interface WaitView {
  /** `topic:<project>/<slug>` or `task:<id>`. */
  key: string;
  kind: "topic" | "task";
  project: string | null;
  topic: string | null;
  task: string | null;
  title: string;
  status: "waiting" | "blocked";
  reason: string | null;
  /** Day of the next check (YYYY-MM-DD). */
  next: string;
  interval_days: number;
  confirmed: IsoMinute | null;
  since: IsoMinute;
  due: boolean;
}

/** `join_update` result. */
export interface JoinReport {
  skipped: string[];
  updates: ProjectUpdateView[];
}

export interface ChangeOutcome {
  /** "rejected": the user decided not to apply it. */
  status: "applied" | "stale" | "failed" | "rejected";
  message: string;
  at: IsoMinute;
}

export interface StoredChange {
  change: ProposedChange;
  /** `null`: not applied yet. */
  outcome: ChangeOutcome | null;
  /** The user corrected the agent's version before applying it. */
  edited?: boolean;
}

/** `edit_update` input: what the user may correct in an undecided change (only what that kind writes). */
export type ChangeEdit =
  /** `tasks`: one per proposed task, in order (`keep: false` leaves it out). */
  | { kind: "new_topic"; title: string; status: TopicStatus; summary: string; text: string; tasks: { keep: boolean; title: string; status: TaskStatus }[] }
  /** `status: null`: the topic status does not change; a summary equal to the current one = no change. */
  | { kind: "topic_log"; status: TopicStatus | null; summary: string; text: string }
  | { kind: "task_status"; status: TaskStatus }
  | { kind: "new_task"; title: string };

export interface DroppedChange {
  index: number;
  label: string;
  reason: string;
}

export interface StoredProposal {
  run_id: string;
  provider: ProviderId;
  trigger: "manual" | "auto";
  created_at: IsoMinute;
  /** The agent's report (Markdown), the block removed. */
  report: string;
  changes: StoredChange[];
  dropped: DroppedChange[];
  dropped_more: number;
  /** No usable proposal: why. */
  problem: string | null;
  /** What the history source contributed, when on. */
  history_note?: string | null;
  /** The user's request when the run was a `zlecenie`, not a check. */
  request?: string | null;
}

export type UpdateOutcome =
  | { kind: "proposed"; count: number }
  | { kind: "no_changes" }
  | { kind: "applied"; count: number }
  | { kind: "unchanged" }
  | { kind: "no_proposal"; message: string }
  /** `limit`: the CLI reported a usage / rate limit; `model`: it did not accept the configured model. */
  | { kind: "failed"; message: string; limit: boolean; model?: boolean }
  | { kind: "cancelled" };

export interface UpdateResult {
  at: IsoMinute;
  provider: ProviderId | null;
  trigger: "manual" | "auto";
  outcome: UpdateOutcome;
  /** What the history source contributed ("Zebrano 12 sesji …"), when on. */
  history_note?: string | null;
  /** The user's request when the run was a `zlecenie`, not a check. */
  request?: string | null;
}

/** `list_updates` item and `updates-changed` payload item. */
export interface ProjectUpdateView {
  project: string;
  auto: AutoUpdate;
  sources: UpdateSources;
  next_due: IsoMinute | null;
  last: UpdateResult | null;
  proposal: StoredProposal | null;
  /** Changes of the proposal still waiting for review. */
  pending: number;
  /** Automatic checks are held back while proposals wait for review. */
  held: boolean;
  /** Token of the last apply click on this proposal that can still be undone. */
  undo: string | null;
  /** That last action was a rejection ("Cofnij odrzucenie"), not a write. */
  undo_rejection?: boolean;
}

/** `dismiss_update` result: the rejection is undoable like an apply click. */
export interface DismissReport {
  count: number;
  undo: string | null;
  updates: ProjectUpdateView[];
}

export interface ApplyItem {
  index: number;
  status: "applied" | "stale" | "already" | "failed" | "missing";
  message: string;
}

/** `apply_update` result. */
export interface ApplyReport {
  project: string;
  items: ApplyItem[];
  /** Token for `undoUpdate` when something was written. */
  undo: string | null;
  updates: ProjectUpdateView[];
}

export interface UndoResult {
  index: number;
  status: "undone" | "partial" | "changed" | "failed";
  message: string;
}

/** `undo_update` result. */
export interface UndoReport {
  project: string;
  items: UndoResult[];
  updates: ProjectUpdateView[];
}
