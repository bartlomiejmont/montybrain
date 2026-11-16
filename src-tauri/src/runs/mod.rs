//! Actions: agent runs of the MontyBrain skills on the user's own subscription
//! (Claude Code or Codex CLI, logged in with a subscription, never an API key).
//!
//! The webview only ever sends a recipe id, a provider, a mode and validated
//! parameters ([`RunRequest`]); prompts, argv and executables are built here:
//!
//! * [`recipes`]: the fixed recipe table, parameter validation, prompt building;
//! * [`providers`]: finding `claude` / `codex` (a GUI app has a minimal PATH);
//! * [`agent`]: model and effort per agent (Settings → "Agenci");
//! * [`command`]: argv and environment of a background run;
//! * [`history`]: the conversation history of Claude Code and Codex as a
//!   second source for `aktualizacja` (collected locally, redacted);
//! * [`parse`]: JSONL output of both CLIs → log lines + final answer;
//! * [`proposal`]: the output contract of the `aktualizacja` recipe (a fenced
//!   JSON block of proposed changes) and its strict validation;
//! * [`runner`]: the run store, the background process lifecycle (process
//!   group, timeout, cancel) and the events sent to the window;
//! * [`terminal`]: the interactive variant, a launcher script opened in Terminal.
//!
//! What happens with a finished `aktualizacja` proposal (storing, applying,
//! the automatic schedule) lives in [`crate::updates`].
//!
//! There is deliberately no generic "run a command" entry point.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};

pub mod agent;
pub mod cli_status;
pub mod command;
pub mod git;
pub mod history;
pub mod parse;
pub mod proc;
pub mod proposal;
pub mod providers;
pub mod recipes;
pub mod runner;
pub mod session;
pub mod terminal;

pub use runner::{
    HistoryOptions, RunContext, RunManager, RunSink, RunnerConfig, StartExtra, Trigger,
};

/// Emitted with a [`RunEvent`] for every new log line of a run.
pub const RUN_EVENT: &str = "run-event";
/// Emitted with a [`RunInfo`] (its `events` empty) when a run starts or ends.
pub const RUN_UPDATED: &str = "run-updated";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Claude,
    Codex,
}

impl ProviderId {
    pub const ALL: [ProviderId; 2] = [ProviderId::Claude, ProviderId::Codex];

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "claude" => Ok(ProviderId::Claude),
            "codex" => Ok(ProviderId::Codex),
            other => Err(format!("Nieznany dostawca: {other}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ProviderId::Claude => "claude",
            ProviderId::Codex => "codex",
        }
    }

    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            ProviderId::Claude => "Claude Code",
            ProviderId::Codex => "Codex CLI",
        }
    }

    /// Executable name looked up by [`providers`].
    pub fn binary(self) -> &'static str {
        self.as_str()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    Background,
    Terminal,
}

impl RunMode {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "background" => Ok(RunMode::Background),
            "terminal" => Ok(RunMode::Terminal),
            other => Err(format!("Nieznany tryb uruchomienia: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Note,
    Text,
    Tool,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProviderInfo {
    pub id: ProviderId,
    pub name: String,
    pub available: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub problem: Option<String>,
    pub auth: providers::AuthInfo,
    pub models: Vec<String>,
    pub limits: Option<session::telemetry::LimitSnapshot>,
    pub limits_problem: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    Project,
    Period,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecipeParam {
    pub key: String,
    pub label: String,
    pub kind: ParamKind,
    pub required: bool,
    pub placeholder: Option<String>,
    #[serde(rename = "default")]
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Recipe {
    pub id: String,
    pub title: String,
    pub description: String,
    pub skill: String,
    pub modes: Vec<RunMode>,
    pub background_writes: bool,
    /// The background run proposes changes for review instead of writing.
    pub proposes: bool,
    /// The project must have repositories (`aktualizacja` compares them).
    pub needs_repos: bool,
    pub params: Vec<RecipeParam>,
}

/// `start_run` input. Provider and mode arrive as strings so that unknown
/// values get a clear message instead of a deserialization error.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RunRequest {
    pub recipe: String,
    pub provider: String,
    pub mode: String,
    /// `null` values count as not given.
    #[serde(default, deserialize_with = "params_without_nulls")]
    pub params: BTreeMap<String, String>,
}

fn params_without_nulls<'de, D>(d: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: Option<BTreeMap<String, Option<String>>> = Deserialize::deserialize(d)?;
    Ok(raw
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k, v)))
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunEvent {
    pub run_id: String,
    pub seq: u64,
    /// Local time, `YYYY-MM-DDTHH:MM:SS`.
    pub at: String,
    pub kind: EventKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunInfo {
    pub id: String,
    pub recipe: String,
    pub recipe_title: String,
    pub provider: ProviderId,
    pub mode: RunMode,
    /// The run's `project` parameter, if any.
    pub project: Option<String>,
    /// The request of a `zlecenie` run (its `text` parameter).
    pub request: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// The model / effort requested in Settings (`None` = the CLI's default).
    pub model: Option<String>,
    pub effort: Option<String>,
    pub status: RunStatus,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub events: Vec<RunEvent>,
}

/// Local "now" in the format of [`RunEvent::at`].
pub fn now_string() -> String {
    montybrain_core::chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string()
}

// --- Tauri glue ---------------------------------------------------------------

/// Managed state: the one run manager of the app.
pub struct Runs(pub Arc<RunManager>);

struct TauriSink<R: Runtime>(AppHandle<R>);

impl<R: Runtime> RunSink for TauriSink<R> {
    fn event(&self, e: &RunEvent) {
        if let Err(err) = self.0.emit(RUN_EVENT, e) {
            crate::log(&format!("emit {RUN_EVENT} failed: {err}"));
        }
    }

    fn updated(&self, info: &RunInfo) {
        crate::log(&format!(
            "run {} ({} / {} / {:?}): {:?}",
            info.id,
            info.recipe,
            info.provider.as_str(),
            info.mode,
            info.status
        ));
        if let Err(err) = self.0.emit(RUN_UPDATED, info) {
            crate::log(&format!("emit {RUN_UPDATED} failed: {err}"));
        }
    }

    fn finished(&self, info: &RunInfo, ctx: &RunContext) {
        crate::updates::run_finished(&self.0, info, ctx);
    }
}

/// Where run files live: `$MONTYBRAIN_CONFIG_DIR/cache` (or the legacy
/// `$MORWISKO_CONFIG_DIR`) when that is set (dev
/// and test runs), else the platform app-cache dir
/// (`~/Library/Caches/pl.montybrain.desktop`).
fn cache_dir<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    match crate::settings::config_dir_override() {
        Some(dir) => dir.join("cache"),
        None => app
            .path()
            .app_cache_dir()
            .unwrap_or_else(|_| std::env::temp_dir().join("montybrain-cache")),
    }
}

/// The in-app agent session (one at a time), see [`session`].
pub struct AgentSessions(pub Arc<session::Sessions>);

struct TauriSessionSink<R: Runtime>(AppHandle<R>);

impl<R: Runtime> session::SessionSink for TauriSessionSink<R> {
    fn changed(&self, v: &session::SessionView) {
        if let Some(at) = v.telemetry.limits_at {
            self.0.state::<Runs>().0.remember_limits(
                v.provider,
                session::telemetry::LimitSnapshot {
                    limits: v.telemetry.limits.clone(),
                    checked_at: at,
                    status: v.telemetry.limit_status.clone(),
                },
            );
        }
        if let Err(err) = self.0.emit(session::SESSION_UPDATED, v) {
            crate::log(&format!("emit {} failed: {err}", session::SESSION_UPDATED));
        }
    }

    fn answer(&self, v: &session::SessionView, text: &str) -> Result<usize, String> {
        let label: String = v.request.chars().take(300).collect();
        crate::updates::agent_answer(&self.0, &v.scope.key(), &v.id, v.provider, text, &label)
    }
}

/// Called once from `setup`.
pub fn init<R: Runtime>(app: &AppHandle<R>) {
    let home = app
        .path()
        .home_dir()
        .ok()
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"));
    let cache = cache_dir(app);
    crate::log(&format!("runs: {}", cache.display()));
    let mut cfg = RunnerConfig::new(home.clone(), cache.join("runs"), cache.join("terminal"));
    // The history stores: the CLIs' defaults, or the env overrides (dev, tests).
    cfg.history_roots = history::Roots::from_env(&home);
    let manager = RunManager::new(
        cfg,
        Arc::new(TauriSink(app.clone())),
        std::env::vars_os().collect(),
    );
    app.manage(Runs(Arc::new(manager)));
    app.manage(AgentSessions(Arc::new(session::Sessions::new(Arc::new(
        TauriSessionSink(app.clone()),
    )))));
}

/// On quit: stop a running background run (its whole process group).
pub fn shutdown<R: Runtime>(app: &AppHandle<R>) {
    if let Some(runs) = app.try_state::<Runs>() {
        runs.0.shutdown(Duration::from_secs(1));
    }
    // The in-app session's process group is killed, never left running.
    if let Some(s) = app.try_state::<AgentSessions>() {
        s.0.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_and_info_json_shapes() {
        let r: RunRequest = serde_json::from_value(serde_json::json!({
            "recipe": "raport", "provider": "codex", "mode": "terminal",
            "params": {"period": "tydzien", "project": null}
        }))
        .unwrap();
        assert_eq!(r.params.len(), 1);
        let r: RunRequest = serde_json::from_value(serde_json::json!({
            "recipe": "raport", "provider": "claude", "mode": "background"
        }))
        .unwrap();
        assert!(r.params.is_empty());
        let r: RunRequest = serde_json::from_value(serde_json::json!({
            "recipe": "raport", "provider": "claude", "mode": "background", "params": null
        }))
        .unwrap();
        assert!(r.params.is_empty());

        let info = RunInfo {
            id: "20261113-120000-1".into(),
            recipe: "raport".into(),
            recipe_title: "Raport z dnia lub tygodnia".into(),
            provider: ProviderId::Codex,
            mode: RunMode::Background,
            project: None,
            request: None,
            topic: None,
            model: None,
            effort: None,
            status: RunStatus::Cancelled,
            started_at: "2026-11-13T12:00:00".into(),
            finished_at: None,
            result: None,
            error: None,
            events: vec![RunEvent {
                run_id: "20261113-120000-1".into(),
                seq: 1,
                at: "2026-11-13T12:00:01".into(),
                kind: EventKind::Tool,
                text: "overview".into(),
            }],
        };
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            serde_json::json!({
                "id": "20261113-120000-1",
                "recipe": "raport",
                "recipe_title": "Raport z dnia lub tygodnia",
                "provider": "codex",
                "mode": "background",
                "project": null,
                "request": null,
                "model": null,
                "effort": null,
                "status": "cancelled",
                "started_at": "2026-11-13T12:00:00",
                "finished_at": null,
                "result": null,
                "error": null,
                "events": [{
                    "run_id": "20261113-120000-1",
                    "seq": 1,
                    "at": "2026-11-13T12:00:01",
                    "kind": "tool",
                    "text": "overview"
                }]
            })
        );
        assert_eq!(
            serde_json::to_value(RunStatus::Succeeded).unwrap(),
            "succeeded"
        );
        assert_eq!(serde_json::to_value(EventKind::Note).unwrap(), "note");
        assert_eq!(serde_json::to_value(RunMode::Terminal).unwrap(), "terminal");
        let t = now_string();
        assert_eq!(t.len(), 19);
        assert_eq!(&t[10..11], "T");
    }
}
