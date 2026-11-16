//! Tauri commands: thin wrappers over `montybrain-core`.
//!
//! Core does synchronous file I/O, so every command is `async` and runs its work
//! on a blocking thread; nothing touches the main (UI) thread. Errors cross the
//! IPC boundary as strings (core's `Error` display text).
//!
//! After a successful mutation the snapshot is reloaded right away (so a
//! `get_state` issued after the promise resolves already sees it) and
//! `vault-changed` is emitted. The watcher reports the same write a moment later;
//! that reload finds nothing new and stays silent.

use montybrain_core::{
    JournalAdded, NewJournalEntry, NewProject, NewTask, NewTopic, Project, ProjectPatch, SearchHit,
    Task, TaskLocation, TaskPatch, Topic, TopicPatch, Vault, local_today,
};
use tauri::{AppHandle, Manager, Runtime, WebviewWindow};
use tauri_plugin_opener::OpenerExt;

use crate::command_line;
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
    F: FnOnce(&Vault) -> montybrain_core::Result<T> + Send + 'static,
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

// --- the command field (src-tauri/src/command_line.rs) ------------------------

/// The live reading of the field: plan, problems, choices, duplicate hint and
/// the completions for the caret. Writes nothing.
#[tauri::command]
pub async fn command_preview<R: Runtime>(
    app: AppHandle<R>,
    input: command_line::FieldInput,
) -> CmdResult<command_line::FieldPreview> {
    blocking(move || {
        let state = app.state::<VaultState>();
        Ok(command_line::preview(
            &state.snapshot(),
            state.vault(),
            &input,
            local_today(),
        ))
    })
    .await
}

/// Writes a previewed plan, only while the data it was based on is unchanged.
#[tauri::command]
pub async fn command_apply<R: Runtime>(
    app: AppHandle<R>,
    input: command_line::FieldInput,
    plan: montybrain_core::command::Plan,
) -> CmdResult<command_line::ApplyResult> {
    blocking(move || {
        let state = app.state::<VaultState>();
        let undo = app.state::<command_line::CommandUndo>();
        let r = command_line::apply(state.vault(), &undo, &input, &plan, local_today());
        if matches!(r, command_line::ApplyResult::Written { .. })
            && let Some(version) = state.reload()
        {
            emit_changed(&app, version, ChangeSource::Mutation, Vec::new());
        }
        Ok(r)
    })
    .await
}

/// Takes back a command write (`token`; else the newest one from `source`,
/// e.g. `inbox` for `u` in Skrzynka).
#[tauri::command]
pub async fn command_undo<R: Runtime>(
    app: AppHandle<R>,
    token: Option<u64>,
    source: Option<String>,
) -> CmdResult<command_line::UndoReport> {
    blocking(move || {
        let state = app.state::<VaultState>();
        let undo = app.state::<command_line::CommandUndo>();
        let r = command_line::undo(state.vault(), &undo, token, source.as_deref())?;
        if let Some(version) = state.reload() {
            emit_changed(&app, version, ChangeSource::Mutation, Vec::new());
        }
        Ok(r)
    })
    .await
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Captured {
    pub task: Task,
    pub undo: Option<u64>,
}

/// A plain new task from a "dodaj zadanie" field (title, optional `!`/`@`).
#[tauri::command]
pub async fn capture_task<R: Runtime>(
    app: AppHandle<R>,
    location: TaskLocation,
    text: String,
) -> CmdResult<Captured> {
    blocking(move || {
        let state = app.state::<VaultState>();
        let undo = app.state::<command_line::CommandUndo>();
        let (task, token) =
            command_line::capture(state.vault(), &undo, location, &text, local_today())?;
        if let Some(version) = state.reload() {
            emit_changed(&app, version, ChangeSource::Mutation, Vec::new());
        }
        Ok(Captured { task, undo: token })
    })
    .await
}

/// One local destination suggestion per open inbox entry (no model involved).
#[tauri::command]
pub async fn inbox_suggestions<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
) -> CmdResult<Vec<montybrain_core::command::InboxSuggestion>> {
    main_window_only(&window)?;
    blocking(move || {
        let snap = app.state::<VaultState>().snapshot();
        Ok(command_line::suggestions(&snap, local_today()))
    })
    .await
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct InboxAgentRequest {
    /// The text after `?` (for `?rozpisz` / `?dopasuj` the verb's instruction).
    #[serde(default)]
    pub text: String,
    /// Chosen entries; empty = every open inbox entry (batch only).
    #[serde(default)]
    pub selection: Vec<String>,
    /// `command`: one entry, the answer is a command for the field;
    /// `proposals`: ticked proposals in the review.
    pub mode: String,
    pub provider: String,
    /// `montybrain` (default): in the background, MontyBrain read-only and
    /// nothing else; `mine`: an interactive session in Terminal with the
    /// user's own tools, MontyBrain read-only, the answer through a file
    /// (always proposals).
    #[serde(default)]
    pub tools: Option<String>,
}

/// Runs the Skrzynka agent once, on request, with only the chosen entries and
/// their top local candidates as data. It writes nothing.
#[tauri::command]
pub async fn inbox_agent<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    request: InboxAgentRequest,
) -> CmdResult<RunInfo> {
    main_window_only(&window)?;
    blocking(move || {
        let state = app.state::<VaultState>();
        let snapshot = state.snapshot();
        let mine = request.tools.as_deref() == Some("mine");
        let single = request.mode == "command" && !mine;
        if single && request.selection.len() != 1 {
            return Err("Komendę od agenta można dostać dla jednego wpisu naraz.".into());
        }
        let data = command_line::agent_data(&snapshot, &request.selection, local_today())?;
        let recipe = if single {
            crate::runs::recipes::INBOX_COMMAND_RECIPE
        } else {
            crate::runs::recipes::INBOX_RECIPE
        };
        let run = RunRequest {
            recipe: recipe.into(),
            provider: request.provider.clone(),
            mode: if mine { "terminal" } else { "background" }.into(),
            params: [
                ("text".to_string(), request.text.clone()),
                ("dane".to_string(), data),
            ]
            .into_iter()
            .collect(),
        };
        let provider = crate::runs::ProviderId::parse(&request.provider)?;
        let extra = StartExtra {
            base: Some(snapshot.clone()),
            agent: crate::updates::agent_choice(&app, provider),
            ..Default::default()
        };
        app.state::<Runs>()
            .0
            .start_with(&run, &snapshot.projects, state.vault().root(), extra)
    })
    .await
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InboxAgentAnswer {
    /// The command to put into the field (to be previewed and confirmed).
    pub command: Option<String>,
    /// The agent's reason, or why there is no command.
    pub note: String,
}

/// The command of a finished single-entry Skrzynka run.
#[tauri::command]
pub async fn inbox_agent_answer<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    run_id: String,
) -> CmdResult<InboxAgentAnswer> {
    main_window_only(&window)?;
    blocking(move || {
        let info = app
            .state::<Runs>()
            .0
            .get(&run_id)
            .ok_or("Nie ma takiego uruchomienia.")?;
        if info.recipe != crate::runs::recipes::INBOX_COMMAND_RECIPE {
            return Err("To nie jest odpowiedź dla wpisu ze Skrzynki.".into());
        }
        match info.status {
            crate::runs::RunStatus::Succeeded => {}
            crate::runs::RunStatus::Running => return Err("Agent jeszcze pracuje.".into()),
            _ => {
                return Err(info
                    .error
                    .unwrap_or_else(|| "Agent nie odpowiedział.".into()));
            }
        }
        let answer = info.result.unwrap_or_default();
        let (note, command) = crate::runs::proposal::command_block(
            &answer,
            crate::runs::recipes::INBOX_COMMAND_FENCE,
        );
        // Only a single line, and never another agent request.
        let command = command.filter(|c| !c.starts_with('?') && !c.contains('\n'));
        let note = if note.is_empty() && command.is_none() {
            "Agent nie zaproponował komendy.".into()
        } else {
            note
        };
        Ok(InboxAgentAnswer { command, note })
    })
    .await
}

/// `inbox_agent_check` / `inbox_agent_paste` result.
#[derive(Debug, Clone, serde::Serialize)]
pub struct InboxAgentCheck {
    /// `waiting`: no answer file yet; `received`: the proposal is in the review.
    pub status: String,
    /// How many changes the answer proposes (`received`).
    pub count: usize,
}

/// The Terminal Skrzynka run `run_id`, if it is one.
fn terminal_inbox_run<R: Runtime>(app: &AppHandle<R>, run_id: &str) -> Result<RunInfo, String> {
    let info = app
        .state::<Runs>()
        .0
        .get(run_id)
        .ok_or("Nie ma takiej sesji agenta.")?;
    if info.recipe != crate::runs::recipes::INBOX_RECIPE
        || info.mode != crate::runs::RunMode::Terminal
    {
        return Err("To nie jest sesja agenta Skrzynki w Terminalu.".into());
    }
    Ok(info)
}

/// Whether the Terminal Skrzynka session `run_id` wrote its answer file;
/// if so it becomes the Skrzynka's proposal (to tick) and the file is removed.
#[tauri::command]
pub async fn inbox_agent_check<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    run_id: String,
) -> CmdResult<InboxAgentCheck> {
    main_window_only(&window)?;
    blocking(move || {
        let info = terminal_inbox_run(&app, &run_id)?;
        let path = app.state::<Runs>().0.answer_path(&run_id);
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            return Ok(InboxAgentCheck {
                status: "waiting".into(),
                count: 0,
            });
        };
        if !meta.is_file() || meta.len() > 512 * 1024 {
            let _ = std::fs::remove_file(&path);
            return Err(
                "Plik odpowiedzi agenta jest nieprawidłowy (za duży albo nie jest plikiem).".into(),
            );
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("Nie udało się odczytać odpowiedzi agenta: {e}"))?;
        // The agent may still be writing it: an answer without its block waits.
        if !text.contains(crate::runs::proposal::FENCE_INFO) {
            return Ok(InboxAgentCheck {
                status: "waiting".into(),
                count: 0,
            });
        }
        let r = crate::updates::inbox_answer(&app, &run_id, info.provider, &text);
        let _ = std::fs::remove_file(&path);
        let count = r?;
        Ok(InboxAgentCheck {
            status: "received".into(),
            count,
        })
    })
    .await
}

/// The answer of a Terminal Skrzynka session pasted by the user (when the
/// agent could not write the file): it becomes the Skrzynka's proposal.
#[tauri::command]
pub async fn inbox_agent_paste<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    run_id: String,
    text: String,
) -> CmdResult<InboxAgentCheck> {
    main_window_only(&window)?;
    blocking(move || {
        if text.len() > 512 * 1024 {
            return Err("Wklejona odpowiedź jest za długa.".into());
        }
        let info = terminal_inbox_run(&app, &run_id)?;
        let count = crate::updates::inbox_answer(&app, &run_id, info.provider, &text)?;
        let _ = std::fs::remove_file(app.state::<Runs>().0.answer_path(&run_id));
        Ok(InboxAgentCheck {
            status: "received".into(),
            count,
        })
    })
    .await
}

/// `agent_session_start`: what the in-app session is about.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SessionRequest {
    /// `inbox` (Skrzynka) or `project` ("Zlecenie" of `project`).
    pub scope: String,
    #[serde(default)]
    pub project: Option<String>,
    pub provider: String,
    pub text: String,
    /// Skrzynka: the chosen entries (empty: every open entry).
    #[serde(default)]
    pub selection: Vec<String>,
}

fn sessions<R: Runtime>(app: &AppHandle<R>) -> std::sync::Arc<crate::runs::session::Sessions> {
    app.state::<crate::runs::AgentSessions>().0.clone()
}

/// Starts the in-app agent session ("Moje narzędzia" inside the app).
#[tauri::command]
pub async fn agent_session_start<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    request: SessionRequest,
) -> CmdResult<crate::runs::session::SessionView> {
    main_window_only(&window)?;
    blocking(move || {
        use crate::runs::recipes::{MAX_TEXT_CHARS, session_prompt_inbox, session_prompt_request};
        use crate::runs::session::Scope;
        let text = request.text.trim().to_string();
        if text.chars().count() > MAX_TEXT_CHARS {
            return Err("Prośba jest za długa.".into());
        }
        let provider = crate::runs::ProviderId::parse(&request.provider)?;
        let state = app.state::<VaultState>();
        let snapshot = state.snapshot();
        let (scope, prompt) = match request.scope.as_str() {
            "inbox" => {
                let data = command_line::agent_data(&snapshot, &request.selection, local_today())?;
                (Scope::Inbox, session_prompt_inbox(&text, &data))
            }
            "project" => {
                let slug = request.project.clone().unwrap_or_default();
                if snapshot.project(&slug).is_none() {
                    return Err("Nie ma takiego projektu.".into());
                }
                if text.is_empty() {
                    return Err("Wpisz zlecenie.".into());
                }
                (
                    Scope::Project {
                        project: slug.clone(),
                    },
                    session_prompt_request(&slug, &text),
                )
            }
            other => return Err(format!("Nieznany zakres sesji: {other}")),
        };
        let shown = if text.is_empty() {
            "Porządek w Skrzynce".to_string()
        } else {
            text.clone()
        };
        let launch = app.state::<Runs>().0.session_launch(
            provider,
            scope,
            &shown,
            crate::runs::recipes::prompt_language(prompt, app.state::<crate::native::Native>().settings().language),
            &snapshot.projects,
            state.vault().root(),
            &crate::updates::agent_choice(&app, provider),
        )?;
        sessions(&app).start(launch)
    })
    .await
}

#[tauri::command]
pub async fn agent_session_get<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
) -> CmdResult<Option<crate::runs::session::SessionView>> {
    main_window_only(&window)?;
    Ok(sessions(&app).view())
}

#[tauri::command]
pub async fn agent_session_send<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    text: String,
) -> CmdResult<()> {
    main_window_only(&window)?;
    blocking(move || sessions(&app).send(&text)).await
}

/// The answer to a card: `allow`, `allow_session` or `deny`; `answers` for
/// a question card (`{question id: chosen label}`).
#[tauri::command]
pub async fn agent_session_answer<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    entry: u64,
    answer: crate::runs::session::Answer,
    answers: Option<serde_json::Value>,
) -> CmdResult<()> {
    main_window_only(&window)?;
    blocking(move || {
        let list = answers
            .as_ref()
            .map(crate::runs::session::parse_answers)
            .unwrap_or_default();
        sessions(&app).answer(entry, answer, list)
    })
    .await
}

#[tauri::command]
pub async fn agent_session_interrupt<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
) -> CmdResult<()> {
    main_window_only(&window)?;
    blocking(move || sessions(&app).interrupt()).await
}

#[tauri::command]
pub async fn agent_session_stop<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
) -> CmdResult<()> {
    main_window_only(&window)?;
    blocking(move || sessions(&app).stop()).await
}

#[tauri::command]
pub async fn agent_session_close<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
) -> CmdResult<()> {
    main_window_only(&window)?;
    blocking(move || sessions(&app).close()).await
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
    refresh: Option<bool>,
) -> CmdResult<Vec<ProviderInfo>> {
    main_window_only(&window)?;
    blocking(move || {
        let runs = app.state::<Runs>();
        Ok(if refresh.unwrap_or(false) {
            runs.0.refresh_providers()
        } else {
            runs.0.providers()
        })
    })
    .await
}

#[tauri::command]
pub async fn start_run<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    mut request: RunRequest,
) -> CmdResult<RunInfo> {
    main_window_only(&window)?;
    blocking(move || {
        let state = app.state::<VaultState>();
        let snapshot = state.snapshot();
        let runs = app.state::<Runs>().0.clone();
        let projects = snapshot.projects.clone();
        let provider = crate::runs::ProviderId::parse(&request.provider)?;
        let mut journal_refresh = None;
        if let Some(topic) = request.params.get("topic") {
            if request.recipe != "zlecenie" || request.mode != "background" {
                return Err("Odświeżanie tematu działa tylko jako zlecenie w tle.".into());
            }
            let project = request.params.get("project").ok_or("Brak projektu.")?;
            crate::runs::recipes::journal_context(&snapshot, project, topic, &[])?;
            let views = crate::updates::list(&app);
            if views.iter().any(|v| &v.project == project && v.pending > 0) {
                return Err(
                    "Projekt ma nierozstrzygnięte propozycje. Najpierw przejrzyj zmiany.".into(),
                );
            }
            let previous = views.iter()
                .find(|v| &v.project == project)
                .and_then(|v| v.journal_refresh.get(topic));
            let input = crate::updates::journal::prepare(&snapshot, project, topic, previous)?;
            request.params.insert("text".into(), "Odśwież temat z dziennika".into());
            journal_refresh = Some(input);
        }
        let history = if request.params.contains_key("topic") {
            None
        } else {
            request.params.get("project")
                .and_then(|p| crate::updates::history_options(&app, p.trim()))
        };
        // The vault as the run starts: the base of an `aktualizacja` proposal;
        // the model / effort and the history source come from Settings.
        let extra = StartExtra {
            journal_refresh,
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
/// through `montybrain-core`; each change gets its own result.
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
    snap: &montybrain_core::Snapshot,
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
