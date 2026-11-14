//! MontyBrain desktop app: a Tauri shell around `montybrain-core`.
//!
//! * On startup: `Vault::init_default()` (honours `$MONTYBRAIN_DIR`), `normalize()`,
//!   `load()`; the snapshot lives in managed [`state::VaultState`].
//! * The core watcher reloads the snapshot on every external change and emits
//!   [`VAULT_CHANGED`] so the window refreshes without user action.
//! * Commands are listed in [`commands`].
//! * Native layer ([`native`]): menu-bar item with the "today" count ([`tray`]),
//!   global quick-capture shortcut + window, due reminders ([`reminders`]),
//!   autostart and app settings ([`settings`], stored outside the vault).
//! * Actions ([`runs`]): agent runs of the MontyBrain skills via Claude Code or
//!   Codex CLI, in the background or in Terminal.
//! * Project updates ([`updates`]): proposals of the `aktualizacja` and `zlecenie` recipes,
//!   applied by the app after review, and the opt-in automatic schedule.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use montybrain_core::{DEFAULT_DEBOUNCE, Vault, VaultWatcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, RunEvent, Runtime};

pub mod commands;
pub mod drift;
pub mod legacy_autostart;
pub mod native;
pub mod notifier;
pub mod paths;
pub mod reminders;
pub mod runs;
pub mod settings;
pub mod shortcut;
pub mod state;
pub mod tray;
pub mod updates;
pub mod waiting;

use state::VaultState;

/// Event emitted to the frontend whenever the loaded snapshot changed.
pub const VAULT_CHANGED: &str = "vault-changed";

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSource {
    /// A file changed on disk (MCP server, editor, our own write seen late).
    Watcher,
    /// A command of this app wrote the vault.
    Mutation,
}

/// Payload of [`VAULT_CHANGED`]. The frontend re-fetches with `get_state`.
#[derive(Debug, Clone, Serialize)]
pub struct VaultChanged {
    pub version: u64,
    pub source: ChangeSource,
    /// Changed paths relative to the vault root (watcher only).
    pub paths: Vec<String>,
}

/// Keeps the file watcher alive for the lifetime of the app.
struct WatcherHandle(#[allow(dead_code)] Mutex<VaultWatcher>);

/// Diagnostics go to stderr (visible when run from a terminal).
pub(crate) fn log(msg: &str) {
    eprintln!("[montybrain] {msg}");
}

/// Locks a mutex, recovering from poisoning (the data is still usable).
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn emit_changed<R: Runtime>(
    app: &AppHandle<R>,
    version: u64,
    source: ChangeSource,
    paths: Vec<PathBuf>,
) {
    let paths: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    log(&format!(
        "{VAULT_CHANGED} v{version} ({source:?}) {}",
        paths.join(", ")
    ));
    let payload = VaultChanged {
        version,
        source,
        paths,
    };
    if let Err(e) = app.emit(VAULT_CHANGED, payload) {
        log(&format!("emit failed: {e}"));
    }
    native::vault_changed(app);
}

fn start_watcher<R: Runtime>(app: &AppHandle<R>, vault: &Vault) {
    let handle = app.clone();
    let watcher = vault.watch(DEFAULT_DEBOUNCE, move |paths| {
        let state = handle.state::<VaultState>();
        match state.reload() {
            Some(version) => emit_changed(&handle, version, ChangeSource::Watcher, paths),
            None => log(&format!(
                "watcher: {} path(s) changed, snapshot unchanged",
                paths.len()
            )),
        }
    });
    match watcher {
        Ok(w) => {
            app.manage(WatcherHandle(Mutex::new(w)));
        }
        Err(e) => log(&format!("file watcher failed to start: {e}")),
    }
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(native::on_shortcut)
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .arg(native::HIDDEN_ARG)
                .build(),
        )
        .setup(|app| {
            let vault = Vault::init_default()?;
            log(&format!("vault: {}", vault.root().display()));
            let state = VaultState::open(vault.clone());
            log(&format!(
                "loaded: {} project(s), {} task(s), {} warning(s)",
                state.snapshot().projects.len(),
                state.snapshot().tasks.len(),
                state.snapshot().warnings.len()
            ));
            app.manage(state);
            start_watcher(app.handle(), &vault);
            runs::init(app.handle());
            native::init(app.handle())?;
            Ok(())
        })
        .on_window_event(native::on_window_event)
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::search,
            commands::create_project,
            commands::update_project,
            commands::archive_project,
            commands::create_topic,
            commands::update_topic,
            commands::delete_topic,
            commands::add_journal_entry,
            commands::create_task,
            commands::update_task,
            commands::move_task,
            commands::delete_task,
            commands::open_url,
            commands::reveal_path,
            commands::reveal_vault,
            commands::get_settings,
            commands::update_settings,
            commands::set_shortcut,
            commands::pause_shortcut,
            commands::set_autostart,
            commands::quick_close,
            commands::quick_resize,
            commands::list_recipes,
            commands::list_providers,
            commands::start_run,
            commands::cancel_run,
            commands::list_runs,
            commands::list_updates,
            commands::apply_update,
            commands::undo_update,
            commands::dismiss_update,
            commands::edit_update,
            commands::join_update,
            commands::list_drift,
            commands::refresh_drift,
            commands::link_branch,
            commands::ignore_branch,
            commands::list_waiting,
            commands::set_waiting,
            commands::confirm_waiting,
            commands::set_auto_update,
            commands::set_update_sources,
        ])
        .build(tauri::generate_context!())
        .expect("error while building MontyBrain");

    app.run(|app, event| {
        // Dock icon clicked (or app re-opened) while the window is hidden.
        if let RunEvent::Reopen { .. } = event {
            log("reopen");
            native::show_main(app);
        }
        if let RunEvent::Exit = event {
            native::release_shortcuts(app);
            runs::shutdown(app);
        }
    });
}
