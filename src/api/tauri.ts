import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import {
  QUICK_SHOW_EVENT,
  RUN_EVENT_EVENT,
  RUN_UPDATED_EVENT,
  SETTINGS_CHANGED_EVENT,
  UPDATES_CHANGED_EVENT,
  VAULT_CHANGED_EVENT,
  DRIFT_CHANGED_EVENT,
  WAITING_CHANGED_EVENT,
  type MorwiskoApi,
} from "./api";
import type { AppSettings, ProjectUpdateView, RunEvent, RunInfo, VaultChanged } from "./types";

/** Backend access through Tauri commands (src-tauri/src/commands.rs). */
export function createTauriApi(): MorwiskoApi {
  return {
    mode: "tauri",
    getState: () => invoke("get_state"),
    onChange: (cb) => listen<VaultChanged>(VAULT_CHANGED_EVENT, (e) => cb(e.payload)),
    search: (query) => invoke("search", { query }),

    createProject: (input) => invoke("create_project", { input }),
    updateProject: (slug, patch) => invoke("update_project", { slug, patch }),
    archiveProject: (slug) => invoke("archive_project", { slug }),

    createTopic: (project, input) => invoke("create_topic", { project, input }),
    updateTopic: (project, topic, patch) => invoke("update_topic", { project, topic, patch }),
    deleteTopic: (project, topic) => invoke("delete_topic", { project, topic }),
    addJournalEntry: (project, topic, entry) => invoke("add_journal_entry", { project, topic, entry }),

    createTask: (location, input) => invoke("create_task", { location, input }),
    updateTask: (id, patch) => invoke("update_task", { id, patch }),
    moveTask: (id, to) => invoke("move_task", { id, to }),
    deleteTask: (id) => invoke("delete_task", { id }),

    openUrl: (url) => invoke("open_url", { url }),
    revealPath: (path) => invoke("reveal_path", { path }),
    revealVault: () => invoke("reveal_vault"),

    getSettings: () => invoke("get_settings"),
    updateSettings: (patch) => invoke("update_settings", { patch }),
    setShortcut: (shortcut) => invoke("set_shortcut", { shortcut }),
    pauseShortcut: (paused) => invoke("pause_shortcut", { paused }),
    setAutostart: (enabled) => invoke("set_autostart", { enabled }),
    onSettingsChange: (cb) => listen<AppSettings>(SETTINGS_CHANGED_EVENT, (e) => cb(e.payload)),

    quickClose: () => invoke("quick_close"),
    quickResize: (height) => invoke("quick_resize", { height }),
    onQuickShow: (cb) => listen(QUICK_SHOW_EVENT, () => cb()),

    listRecipes: () => invoke("list_recipes"),
    listProviders: () => invoke("list_providers"),
    startRun: (request) => invoke("start_run", { request }),
    // Tauri maps JS camelCase keys to Rust snake_case parameters; the contract
    // names the parameter `run_id`, so both spellings are sent (extra keys are ignored).
    cancelRun: (runId) => invoke("cancel_run", { runId, run_id: runId }),
    listRuns: () => invoke("list_runs"),
    onRunEvent: (cb) => listen<RunEvent>(RUN_EVENT_EVENT, (e) => cb(e.payload)),
    onRunUpdated: (cb) => listen<RunInfo>(RUN_UPDATED_EVENT, (e) => cb(e.payload)),

    listUpdates: () => invoke("list_updates"),
    applyUpdate: (runId, indices) => invoke("apply_update", { runId, indices }),
    undoUpdate: (runId, token) => invoke("undo_update", { runId, token }),
    dismissUpdate: (runId) => invoke("dismiss_update", { runId }),
    editUpdate: (runId, index, edit) => invoke("edit_update", { runId, index, edit }),
    joinUpdate: (runId, index, topic) => invoke("join_update", { runId, index, topic }),
    listDrift: () => invoke("list_drift"),
    refreshDrift: () => invoke("refresh_drift"),
    linkBranch: (project, repo, branch, topic) => invoke("link_branch", { project, repo, branch, topic }),
    ignoreBranch: (project, repo, branch) => invoke("ignore_branch", { project, repo, branch }),
    onDriftChanged: (cb) => listen(DRIFT_CHANGED_EVENT, () => cb()),
    listWaiting: () => invoke("list_waiting"),
    setWaiting: (key, days) => invoke("set_waiting", { key, days }),
    confirmWaiting: (key) => invoke("confirm_waiting", { key }),
    onWaitingChanged: (cb) => listen(WAITING_CHANGED_EVENT, () => cb()),
    setAutoUpdate: (project, config) => invoke("set_auto_update", { project, config }),
    setUpdateSources: (project, sources) => invoke("set_update_sources", { project, sources }),
    onUpdatesChanged: (cb) => listen<ProjectUpdateView[]>(UPDATES_CHANGED_EVENT, (e) => cb(e.payload)),
  };
}
