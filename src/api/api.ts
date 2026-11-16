// The one typed surface every view uses to talk to the backend.

import type {
  Captured,
  CommandApplyResult,
  CommandPlan,
  CommandUndoReport,
  FieldInput,
  FieldPreview,
  InboxAgentAnswer,
  InboxAgentCheck,
  SessionAnswer,
  SessionStartRequest,
  SessionView,
  InboxAgentRequest,
  InboxSuggestion,
  ApplyReport,
  AppSettings,
  AutoUpdate,
  ChangeEdit,
  DismissReport,
  JoinReport,
  ProjectDrift,
  WaitView,
  AppView,
  JournalAdded,
  NewJournalEntry,
  NewProject,
  NewTask,
  NewTopic,
  Project,
  ProjectPatch,
  ProjectUpdateView,
  ProviderInfo,
  Recipe,
  RunEvent,
  RunInfo,
  RunRequest,
  SearchHit,
  SettingsPatch,
  Task,
  TaskLocation,
  TaskPatch,
  Topic,
  TopicPatch,
  UndoReport,
  UpdateSources,
  VaultChanged,
} from "./types";

export interface MontyBrainApi {
  /** "tauri" inside the app, "mock" in a plain browser (`pnpm dev`). */
  readonly mode: "tauri" | "mock";

  getState(): Promise<AppView>;
  /** Subscribes to `vault-changed`; resolves to an unsubscribe function. */
  onChange(cb: (e: VaultChanged) => void): Promise<() => void>;
  search(query: string): Promise<SearchHit[]>;

  createProject(input: NewProject): Promise<Project>;
  updateProject(slug: string, patch: ProjectPatch): Promise<Project>;
  archiveProject(slug: string): Promise<Project>;

  createTopic(project: string, input: NewTopic): Promise<Topic>;
  updateTopic(project: string, topic: string, patch: TopicPatch): Promise<Topic>;
  deleteTopic(project: string, topic: string): Promise<Topic>;
  addJournalEntry(project: string, topic: string, entry: NewJournalEntry): Promise<JournalAdded>;

  createTask(location: TaskLocation, input: NewTask): Promise<Task>;
  updateTask(id: string, patch: TaskPatch): Promise<Task>;
  moveTask(id: string, to: TaskLocation): Promise<Task>;
  deleteTask(id: string): Promise<Task>;

  // --- the command field (Skrzynka, quick capture, ⌘K) ---
  /** The live reading: plan, problems, choices, duplicate hint, completions. Writes nothing. */
  commandPreview(input: FieldInput): Promise<FieldPreview>;
  /** Writes a previewed plan, only while the data it was based on is unchanged. */
  commandApply(input: FieldInput, plan: CommandPlan): Promise<CommandApplyResult>;
  /** Takes back a command write (`token`, else the newest one from `source`, e.g. `inbox`). */
  commandUndo(token: number | null, source: string | null): Promise<CommandUndoReport>;
  /** A plain new task from a "dodaj zadanie" field (title with optional `!`/`@`). */
  captureTask(location: TaskLocation, text: string): Promise<Captured>;
  /** One local destination suggestion per open inbox entry. */
  inboxSuggestions(): Promise<InboxSuggestion[]>;
  /** Runs the Skrzynka agent once with the chosen entries (writes nothing). */
  inboxAgent(request: InboxAgentRequest): Promise<RunInfo>;
  /** The command of a finished single-entry Skrzynka run. */
  inboxAgentAnswer(runId: string): Promise<InboxAgentAnswer>;
  /** Whether the Terminal Skrzynka session wrote its answer; if so it becomes the proposal to tick. */
  inboxAgentCheck(runId: string): Promise<InboxAgentCheck>;
  /** The answer of a Terminal Skrzynka session, pasted by the user. */
  inboxAgentPaste(runId: string, text: string): Promise<InboxAgentCheck>;

  /** Starts the in-app agent session ("Moje narzędzia"): one at a time. */
  agentSessionStart(request: SessionStartRequest): Promise<SessionView>;
  /** The live (or last ended) session. */
  agentSessionGet(): Promise<SessionView | null>;
  /** A follow-up message to the agent. */
  agentSessionSend(text: string): Promise<void>;
  /** The answer to a permission card (or a question card with `answers`). */
  agentSessionAnswer(entry: number, answer: SessionAnswer, answers?: Record<string, string>): Promise<void>;
  /** Stops the current turn; the session stays. */
  agentSessionInterrupt(): Promise<void>;
  /** Ends the session (the process is killed). */
  agentSessionStop(): Promise<void>;
  /** Forgets an ended session. */
  agentSessionClose(): Promise<void>;
  /** The session changed (the whole view). */
  onAgentSession(cb: (v: SessionView) => void): Promise<() => void>;

  openUrl(url: string): Promise<void>;
  /** Reveal a vault file (path relative to the vault root) in Finder. */
  revealPath(path: string): Promise<void>;
  /** Reveal the vault folder itself in Finder. */
  revealVault(): Promise<void>;

  // --- app settings & native integration ---
  getSettings(): Promise<AppSettings>;
  updateSettings(patch: SettingsPatch): Promise<AppSettings>;
  /** Registers a new global shortcut; rejects (keeping the old one) when it is taken or invalid. */
  setShortcut(shortcut: string): Promise<AppSettings>;
  /**
   * Suspends the global shortcut while a new one is being recorded (`true`) or
   * resumes it (`false`). Resuming rejects when the combination can no longer be
   * registered; the shortcut is then inactive and `shortcut_error` says why.
   */
  pauseShortcut(paused: boolean): Promise<AppSettings>;
  setAutostart(enabled: boolean): Promise<AppSettings>;
  /** Subscribes to `settings-changed`; resolves to an unsubscribe function. */
  onSettingsChange(cb: (s: AppSettings) => void): Promise<() => void>;

  // --- quick-capture window (src/quick) ---
  /** Hides the quick-capture window and gives focus back to the previous app. */
  quickClose(): Promise<void>;
  /** Fits the quick-capture window to its content height (CSS px). */
  quickResize(height: number): Promise<void>;
  /** Fires each time the quick-capture window is shown. */
  onQuickShow(cb: () => void): Promise<() => void>;

  // --- agent actions („Akcje") ---
  listRecipes(): Promise<Recipe[]>;
  listProviders(refresh?: boolean): Promise<ProviderInfo[]>;
  /** Background: rejects while another background run is running. Terminal: resolves at once ("opened"). */
  startRun(request: RunRequest): Promise<RunInfo>;
  cancelRun(runId: string): Promise<RunInfo>;
  /** Newest first, the last 20 (kept in memory by the backend). */
  listRuns(): Promise<RunInfo[]>;
  /** Subscribes to `run-event` (one new log line); resolves to an unsubscribe function. */
  onRunEvent(cb: (e: RunEvent) => void): Promise<() => void>;
  /** Subscribes to `run-updated` (RunInfo with `events: []`); resolves to an unsubscribe function. */
  onRunUpdated(cb: (r: RunInfo) => void): Promise<() => void>;

  // --- project updates („Aktualizacja"; started with startRun({recipe: "aktualizacja", …})) ---
  /** Every project with an update setting or state. */
  listUpdates(): Promise<ProjectUpdateView[]>;
  /** Applies the selected changes (0-based) of a run's proposal; per-change results. */
  applyUpdate(runId: string, indices: number[]): Promise<ApplyReport>;
  /** Takes back one apply click (`token` from `applyUpdate`), only where it is still what was written. */
  undoUpdate(runId: string, token: string): Promise<UndoReport>;
  /** Rejects the undecided changes of a run's proposal (nothing is written; it stays as the last result). */
  dismissUpdate(runId: string): Promise<DismissReport>;
  /** "Dołącz do tematu…": a proposed new topic becomes an entry (and tasks) of an existing topic. */
  joinUpdate(runId: string, index: number, topic: string): Promise<JoinReport>;
  /** Drift signals of every active project (last collection + the vault now). */
  listDrift(): Promise<ProjectDrift[]>;
  /** Asks for a collection now; false when one is already running. */
  refreshDrift(): Promise<boolean>;
  linkBranch(project: string, repo: string, branch: string, topic: string): Promise<ProjectDrift[]>;
  ignoreBranch(project: string, repo: string, branch: string): Promise<ProjectDrift[]>;
  /** Subscribes to `drift-changed` (then call `listDrift`). */
  onDriftChanged(cb: () => void): Promise<() => void>;
  /** Waiting topics and blocked tasks with their check schedule. */
  listWaiting(): Promise<WaitView[]>;
  setWaiting(key: string, days: number): Promise<WaitView[]>;
  confirmWaiting(key: string): Promise<WaitView[]>;
  onWaitingChanged(cb: () => void): Promise<() => void>;
  /** Corrects one undecided change before applying (validated like the agent's text). */
  editUpdate(runId: string, index: number, edit: ChangeEdit): Promise<ProjectUpdateView[]>;
  setAutoUpdate(project: string, config: AutoUpdate): Promise<ProjectUpdateView[]>;
  /** Sets where a project's update looks (conversation history on/off, window). */
  setUpdateSources(project: string, sources: UpdateSources): Promise<ProjectUpdateView[]>;
  /** Subscribes to `updates-changed` (every project's view). */
  onUpdatesChanged(cb: (u: ProjectUpdateView[]) => void): Promise<() => void>;
}

/** Name of the backend event (src-tauri/src/lib.rs `VAULT_CHANGED`). */
export const VAULT_CHANGED_EVENT = "vault-changed";
/** src-tauri/src/native.rs `SETTINGS_CHANGED`. */
export const SETTINGS_CHANGED_EVENT = "settings-changed";
/** src-tauri/src/native.rs `QUICK_SHOW`. */
export const QUICK_SHOW_EVENT = "quick-show";
/** src-tauri `run-event`. */
export const RUN_EVENT_EVENT = "run-event";
/** src-tauri runs::session::SESSION_UPDATED */
export const AGENT_SESSION_EVENT = "agent-session";
/** src-tauri `run-updated`. */
export const RUN_UPDATED_EVENT = "run-updated";
/** src-tauri/src/updates `UPDATES_CHANGED`. */
export const UPDATES_CHANGED_EVENT = "updates-changed";
export const DRIFT_CHANGED_EVENT = "drift-changed";
export const WAITING_CHANGED_EVENT = "waiting-changed";
