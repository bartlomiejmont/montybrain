//! Tauri commands: thin wrappers over `morwisko-core`.
//!
//! Core does synchronous file I/O, so every command is `async` and runs its work
//! on a blocking thread; nothing touches the main (UI) thread. Errors cross the
//! IPC boundary as strings (core's `Error` display text).
//!
//! After a successful mutation the snapshot is reloaded right away (so a
//! `get_state` issued after the promise resolves already sees it) and
//! `vault-changed` is emitted. The watcher reports the same write a moment later;
//! that reload finds nothing new and stays silent.

use morwisko_core::{
    JournalAdded, NewJournalEntry, NewProject, NewTask, NewTopic, Project, ProjectPatch, SearchHit,
    Task, TaskLocation, TaskPatch, Topic, TopicPatch, Vault, local_today,
};
use tauri::{AppHandle, Manager, Runtime, WebviewWindow};
use tauri_plugin_opener::OpenerExt;

use crate::native::{self, SettingsView};
use crate::paths::{checked_url, resolve_in_vault};
use crate::runs::{ProviderInfo, Recipe, RunInfo, RunRequest, Runs, StartExtra};
use crate::settings::SettingsPatch;
use crate::state::{AppView, VaultState};
use crate::updates::schedule::{AutoUpdate, UpdateSources};
use crate::updates::store::ProjectUpdateView;
use crate::updates::{ApplyReport, UndoReport};
use crate::{ChangeSource, emit_changed};

type CmdResult<T> = Result<T, String>;

async fn blocking<T, F>(f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> CmdResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Błąd wewnętrzny: {e}"))?
}

/// Runs a core mutation off the main thread, then reloads + emits on success.
async fn mutate<R, T, F>(app: AppHandle<R>, f: F) -> CmdResult<T>
where
    R: Runtime,
    T: Send + 'static,
    F: FnOnce(&Vault) -> morwisko_core::Result<T> + Send + 'static,
{
    blocking(move || {
        let state = app.state::<VaultState>();
        let out = f(state.vault()).map_err(|e| e.to_string())?;
        if let Some(version) = state.reload() {
            emit_changed(&app, version, ChangeSource::Mutation, Vec::new());
        }
        Ok(out)
    })
    .await
}

#[tauri::command]
pub async fn get_state<R: Runtime>(app: AppHandle<R>) -> CmdResult<AppView> {
    blocking(move || {
        let state = app.state::<VaultState>();
        Ok(state.view(local_today(), state.vault().now()))
    })
    .await
}

#[tauri::command]
pub async fn search<R: Runtime>(app: AppHandle<R>, query: String) -> CmdResult<Vec<SearchHit>> {
    blocking(move || Ok(app.state::<VaultState>().snapshot().search(&query))).await
}

// --- projects ---------------------------------------------------------------

#[tauri::command]
pub async fn create_project<R: Runtime>(
    app: AppHandle<R>,
    input: NewProject,
) -> CmdResult<Project> {
    mutate(app, move |v| v.create_project(input)).await
}

#[tauri::command]
pub async fn update_project<R: Runtime>(
    app: AppHandle<R>,
    slug: String,
    patch: ProjectPatch,
) -> CmdResult<Project> {
    mutate(app, move |v| v.update_project(&slug, patch)).await
}

#[tauri::command]
pub async fn archive_project<R: Runtime>(app: AppHandle<R>, slug: String) -> CmdResult<Project> {
    mutate(app, move |v| v.archive_project(&slug)).await
}

// --- topics -----------------------------------------------------------------

#[tauri::command]
pub async fn create_topic<R: Runtime>(
    app: AppHandle<R>,
    project: String,
    input: NewTopic,
) -> CmdResult<Topic> {
    mutate(app, move |v| v.create_topic(&project, input)).await
}

#[tauri::command]
pub async fn update_topic<R: Runtime>(
    app: AppHandle<R>,
    project: String,
    topic: String,
    patch: TopicPatch,
) -> CmdResult<Topic> {
    mutate(app, move |v| v.update_topic(&project, &topic, patch)).await
}

#[tauri::command]
pub async fn delete_topic<R: Runtime>(
    app: AppHandle<R>,
    project: String,
    topic: String,
) -> CmdResult<Topic> {
    mutate(app, move |v| v.delete_topic(&project, &topic)).await
}

#[tauri::command]
pub async fn add_journal_entry<R: Runtime>(
    app: AppHandle<R>,
    project: String,
    topic: String,
    entry: NewJournalEntry,
) -> CmdResult<JournalAdded> {
    mutate(app, move |v| v.add_journal_entry(&project, &topic, entry)).await
}

// --- tasks ------------------------------------------------------------------

#[tauri::command]
pub async fn create_task<R: Runtime>(
    app: AppHandle<R>,
    location: TaskLocation,
    input: NewTask,
) -> CmdResult<Task> {
    mutate(app, move |v| v.create_task(&location, input)).await
}

#[tauri::command]
pub async fn update_task<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    patch: TaskPatch,
) -> CmdResult<Task> {
    mutate(app, move |v| v.update_task(&id, patch)).await
}

#[tauri::command]
pub async fn move_task<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    to: TaskLocation,
) -> CmdResult<Task> {
    mutate(app, move |v| v.move_task(&id, &to)).await
}

#[tauri::command]
pub async fn delete_task<R: Runtime>(app: AppHandle<R>, id: String) -> CmdResult<Task> {
    mutate(app, move |v| v.delete_task(&id)).await
}

// --- OS integration ---------------------------------------------------------

/// Opens an http(s)/mailto link in the default browser / mail app.
#[tauri::command]
pub async fn open_url<R: Runtime>(app: AppHandle<R>, url: String) -> CmdResult<()> {
    let url = checked_url(&url)?.to_string();
    blocking(move || {
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|e| e.to_string())
    })
    .await
}

/// Reveals a vault file or directory (path relative to the vault root) in Finder.
#[tauri::command]
pub async fn reveal_path<R: Runtime>(app: AppHandle<R>, path: String) -> CmdResult<()> {
    blocking(move || {
        let root = app.state::<VaultState>().vault().root().to_path_buf();
        let full = resolve_in_vault(&root, &path)?;
        app.opener()
            .reveal_item_in_dir(full)
            .map_err(|e| e.to_string())
    })
    .await
}

/// Reveals the vault folder itself in Finder.
#[tauri::command]
pub async fn reveal_vault<R: Runtime>(app: AppHandle<R>) -> CmdResult<()> {
    blocking(move || {
        let root = app.state::<VaultState>().vault().root().to_path_buf();
        app.opener()
            .reveal_item_in_dir(root)
            .map_err(|e| e.to_string())
    })
    .await
}

// --- settings & native integration (src-tauri/src/native.rs) ----------------

#[tauri::command]
pub async fn get_settings<R: Runtime>(app: AppHandle<R>) -> CmdResult<SettingsView> {
    blocking(move || Ok(native::settings_view(&app))).await
}

#[tauri::command]
pub async fn update_settings<R: Runtime>(
    app: AppHandle<R>,
    patch: SettingsPatch,
) -> CmdResult<SettingsView> {
    blocking(move || native::update_settings(&app, patch)).await
}

/// Registers a new global shortcut (`"Ctrl+Alt+Space"`); the old one stays on failure.
#[tauri::command]
pub async fn set_shortcut<R: Runtime>(
    app: AppHandle<R>,
    shortcut: String,
) -> CmdResult<SettingsView> {
    blocking(move || native::set_shortcut(&app, &shortcut)).await
}

/// Suspends the global shortcut while Settings records a new combination.
#[tauri::command]
pub async fn pause_shortcut<R: Runtime>(
    app: AppHandle<R>,
    paused: bool,
) -> CmdResult<SettingsView> {
    blocking(move || native::pause_shortcut(&app, paused)).await
}

#[tauri::command]
pub async fn set_autostart<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> CmdResult<SettingsView> {
    blocking(move || native::set_autostart(&app, enabled)).await
}

/// Closes the quick-capture window and returns focus where it came from.
#[tauri::command]
pub async fn quick_close<R: Runtime>(app: AppHandle<R>) -> CmdResult<()> {
    blocking(move || {
        native::hide_quick(&app);
        Ok(())
    })
    .await
}

/// Fits the quick-capture window to its content (CSS px).
#[tauri::command]
pub async fn quick_resize<R: Runtime>(app: AppHandle<R>, height: f64) -> CmdResult<()> {
    blocking(move || {
        native::resize_quick(&app, height);
        Ok(())
    })
    .await
}

// --- actions: agent runs (src-tauri/src/runs/) -------------------------------

/// Agent runs are started from the main window only (the capabilities say so
/// too; this is the second lock).
fn main_window_only<R: Runtime>(window: &WebviewWindow<R>) -> CmdResult<()> {
    if window.label() == native::MAIN {
        Ok(())
    } else {
        Err("Akcje są dostępne tylko w głównym oknie.".into())
    }
}

#[tauri::command]
pub async fn list_recipes<R: Runtime>(window: WebviewWindow<R>) -> CmdResult<Vec<Recipe>> {
    main_window_only(&window)?;
    Ok(crate::runs::recipes::list())
}

/// Finds `claude` / `codex` and asks for their version (cached briefly).
#[tauri::command]
pub async fn list_providers<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
) -> CmdResult<Vec<ProviderInfo>> {
    main_window_only(&window)?;
    blocking(move || Ok(app.state::<Runs>().0.providers())).await
}

#[tauri::command]
pub async fn start_run<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    request: RunRequest,
) -> CmdResult<RunInfo> {
    main_window_only(&window)?;
    blocking(move || {
        let state = app.state::<VaultState>();
        let snapshot = state.snapshot();
        let runs = app.state::<Runs>().0.clone();
        let projects = snapshot.projects.clone();
        let provider = crate::runs::ProviderId::parse(&request.provider)?;
        let history = request
            .params
            .get("project")
            .and_then(|p| crate::updates::history_options(&app, p.trim()));
        // The vault as the run starts: the base of an `aktualizacja` proposal;
        // the model / effort and the history source come from Settings.
        let extra = StartExtra {
            base: Some(snapshot),
            agent: crate::updates::agent_choice(&app, provider),
            history,
            ..Default::default()
        };
        runs.start_with(&request, &projects, state.vault().root(), extra)
    })
    .await
}

/// Stops a background run (SIGTERM, then SIGKILL to its process group) and
/// returns it once it ended (or after a bounded wait).
#[tauri::command]
pub async fn cancel_run<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    run_id: String,
) -> CmdResult<RunInfo> {
    main_window_only(&window)?;
    blocking(move || app.state::<Runs>().0.cancel(&run_id)).await
}

/// The last runs, newest first, with their logs.
#[tauri::command]
pub async fn list_runs<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
) -> CmdResult<Vec<RunInfo>> {
    main_window_only(&window)?;
    blocking(move || Ok(app.state::<Runs>().0.list())).await
}

// --- project updates (src-tauri/src/updates/) ---------------------------------

/// Every project's update state: setting, last result, pending proposal.
#[tauri::command]
pub async fn list_updates<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
) -> CmdResult<Vec<ProjectUpdateView>> {
    main_window_only(&window)?;
    blocking(move || Ok(crate::updates::list(&app))).await
}

/// Applies the selected changes (0-based indices) of the proposal of a run,
/// through `morwisko-core`; each change gets its own result.
#[tauri::command]
pub async fn apply_update<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    run_id: String,
    indices: Vec<usize>,
) -> CmdResult<ApplyReport> {
    main_window_only(&window)?;
    if indices.len() > 4 * crate::runs::proposal::MAX_CHANGES {
        return Err("Za dużo zmian naraz.".into());
    }
    blocking(move || crate::updates::apply_changes(&app, &run_id, &indices)).await
}

/// Takes back the writes of one apply click (`token` from `apply_update`),
/// only where they are still what was written.
#[tauri::command]
pub async fn undo_update<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    run_id: String,
    token: String,
) -> CmdResult<UndoReport> {
    main_window_only(&window)?;
    blocking(move || crate::updates::undo_changes(&app, &run_id, &token)).await
}

/// Rejects the undecided changes of a run's proposal (nothing is written to the vault).
#[tauri::command]
pub async fn dismiss_update<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    run_id: String,
) -> CmdResult<crate::updates::DismissReport> {
    main_window_only(&window)?;
    blocking(move || crate::updates::dismiss(&app, &run_id)).await
}

/// Corrects one undecided change of a run's proposal before it is applied
/// (validated like the agent's text); returns the updated views.
#[tauri::command]
pub async fn edit_update<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    run_id: String,
    index: usize,
    edit: crate::runs::proposal::ChangeEdit,
) -> CmdResult<Vec<ProjectUpdateView>> {
    main_window_only(&window)?;
    blocking(move || crate::updates::edit_change(&app, &run_id, index, &edit)).await
}

/// Sets the automatic update of a project.
#[tauri::command]
pub async fn set_auto_update<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    project: String,
    config: AutoUpdate,
) -> CmdResult<Vec<ProjectUpdateView>> {
    main_window_only(&window)?;
    blocking(move || crate::updates::set_auto(&app, &project, config)).await
}

/// Sets the sources of a project's update (history of conversations on/off, window).
#[tauri::command]
pub async fn set_update_sources<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    project: String,
    sources: UpdateSources,
) -> CmdResult<Vec<ProjectUpdateView>> {
    main_window_only(&window)?;
    blocking(move || crate::updates::set_sources(&app, &project, sources)).await
}

/// "Dołącz do tematu…": a proposed new topic becomes an entry (and tasks) of
/// an existing topic of the project.
#[tauri::command]
pub async fn join_update<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    run_id: String,
    index: usize,
    topic: String,
) -> CmdResult<crate::updates::JoinReport> {
    main_window_only(&window)?;
    blocking(move || crate::updates::join_change(&app, &run_id, index, &topic)).await
}

// --- drift (src-tauri/src/drift/) ------------------------------------------------

/// Every active project's drift signals (last collection + the vault now).
#[tauri::command]
pub async fn list_drift<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
) -> CmdResult<Vec<crate::drift::ProjectDrift>> {
    main_window_only(&window)?;
    blocking(move || Ok(crate::drift::list(&app))).await
}

/// Asks for a collection now (runs in the background; `drift-changed` follows).
#[tauri::command]
pub async fn refresh_drift<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
) -> CmdResult<bool> {
    main_window_only(&window)?;
    let Some(d) = app.try_state::<crate::drift::Drift>() else {
        return Ok(false);
    };
    let running = d.is_running();
    if !running {
        d.poke();
    }
    Ok(!running)
}

fn check_branch(
    snap: &morwisko_core::Snapshot,
    project: &str,
    repo: &str,
    branch: &str,
) -> CmdResult<()> {
    let p = snap
        .project(project)
        .ok_or_else(|| format!("Nie ma projektu „{project}”."))?;
    if !p.repos.iter().any(|r| r.name == repo) {
        return Err(format!("Projekt nie ma repozytorium „{repo}”."));
    }
    if !crate::runs::proposal::valid_branch_name(branch) {
        return Err("Nieprawidłowa nazwa gałęzi.".into());
    }
    Ok(())
}

/// "Przypisz do tematu…": links a branch to a topic of its project.
#[tauri::command]
pub async fn link_branch<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    project: String,
    repo: String,
    branch: String,
    topic: String,
) -> CmdResult<Vec<crate::drift::ProjectDrift>> {
    main_window_only(&window)?;
    blocking(move || {
        let state = app.state::<VaultState>();
        let snap = state.snapshot();
        check_branch(&snap, &project, &repo, &branch)?;
        if snap.topic(&project, &topic).is_none() {
            return Err("W tym projekcie nie ma takiego tematu.".into());
        }
        let key = crate::drift::links::BranchKey::new(&project, &repo, &branch);
        crate::drift::links::update(state.vault().root(), |s| s.link(&key, &topic))?;
        crate::drift::emit_changed(&app);
        Ok(crate::drift::list(&app))
    })
    .await
}

/// "Ignoruj": hides a branch from the unknown-branches signal.
#[tauri::command]
pub async fn ignore_branch<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    project: String,
    repo: String,
    branch: String,
) -> CmdResult<Vec<crate::drift::ProjectDrift>> {
    main_window_only(&window)?;
    blocking(move || {
        let state = app.state::<VaultState>();
        check_branch(&state.snapshot(), &project, &repo, &branch)?;
        let key = crate::drift::links::BranchKey::new(&project, &repo, &branch);
        crate::drift::links::update(state.vault().root(), |s| s.ignore(&key))?;
        crate::drift::emit_changed(&app);
        Ok(crate::drift::list(&app))
    })
    .await
}

// --- waiting items (src-tauri/src/waiting.rs) --------------------------------------

/// Waiting topics and blocked tasks with their check schedule.
#[tauri::command]
pub async fn list_waiting<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
) -> CmdResult<Vec<crate::waiting::WaitView>> {
    main_window_only(&window)?;
    blocking(move || Ok(crate::waiting::list(&app))).await
}

/// "Przypomnij za": the next check of `key` in `days` days.
#[tauri::command]
pub async fn set_waiting<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    key: String,
    days: u8,
) -> CmdResult<Vec<crate::waiting::WaitView>> {
    main_window_only(&window)?;
    blocking(move || {
        crate::waiting::change(&app, |st, now| {
            if !st.items.contains_key(&key) {
                return Err("Ta pozycja nie czeka (zmień najpierw jej status).".into());
            }
            crate::waiting::schedule(st, &key, days, now)
        })
    })
    .await
}

/// "Nadal czeka": the next check one interval from today.
#[tauri::command]
pub async fn confirm_waiting<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    key: String,
) -> CmdResult<Vec<crate::waiting::WaitView>> {
    main_window_only(&window)?;
    blocking(move || crate::waiting::change(&app, |st, now| crate::waiting::confirm(st, &key, now)))
        .await
}
