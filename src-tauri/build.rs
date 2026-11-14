//! Declaring the app commands makes Tauri check every app command against the
//! capabilities (`capabilities/*.json`), so each window gets only what it
//! lists. Keep this list in sync with `generate_handler!` in `src/lib.rs`.

/// Commands of the main window and the quick-capture window.
const SHARED: &[&str] = &[
    "get_state",
    "search",
    "create_project",
    "update_project",
    "archive_project",
    "create_topic",
    "update_topic",
    "delete_topic",
    "add_journal_entry",
    "create_task",
    "update_task",
    "move_task",
    "delete_task",
    "open_url",
    "reveal_path",
    "reveal_vault",
    "get_settings",
    "update_settings",
    "set_shortcut",
    "pause_shortcut",
    "set_autostart",
    "quick_close",
    "quick_resize",
];

/// Agent runs and project updates: main window only.
const RUNS: &[&str] = &[
    "list_recipes",
    "list_providers",
    "start_run",
    "cancel_run",
    "list_runs",
    "list_updates",
    "apply_update",
    "undo_update",
    "dismiss_update",
    "edit_update",
    "join_update",
    "list_drift",
    "refresh_drift",
    "link_branch",
    "ignore_branch",
    "list_waiting",
    "set_waiting",
    "confirm_waiting",
    "set_auto_update",
    "set_update_sources",
];

fn main() {
    let all: &'static [&'static str] = Box::leak([SHARED, RUNS].concat().into_boxed_slice());
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(all)),
    )
    .expect("tauri build");
}
