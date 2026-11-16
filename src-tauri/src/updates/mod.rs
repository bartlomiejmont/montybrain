//! Project updates: what happens with the proposal of an `aktualizacja` run,
//! and the automatic update schedule.
//!
//! * A background `aktualizacja` run (manual from the project view or Akcje,
//!   or automatic) is read-only; its answer ends with a block of proposed
//!   changes ([`crate::runs::proposal`]). When the run ends, [`run_finished`]
//!   parses and validates it against the vault as the run started and stores
//!   it per project ([`store`], `updates.json` in the config dir).
//! * The user applies selected changes ([`apply_changes`], Tauri command
//!   `apply_update`); the app writes them through `montybrain-core`
//!   ([`apply`]), skipping changes whose target changed in the meantime and
//!   never applying a change twice.
//! * Automatic update ([`schedule`], setting `auto_update` per project in
//!   `settings.json`, default off): a scheduler thread checks once a minute
//!   while the app runs (first after [`STARTUP_DELAY`]); it picks the most
//!   overdue project, skips it without calling the agent when the input
//!   fingerprint (git snapshot + the project's vault data) equals the one of
//!   the last completed check, never starts while another background run is
//!   running, and after a failure waits a full interval. In "apply" mode all
//!   valid changes are applied at once; either way a notification tells the
//!   user.
//!
//! Every change of this state is sent to the window as [`UPDATES_CHANGED`].

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use montybrain_core::chrono::{Local, NaiveDateTime, Timelike};
use montybrain_core::{ProjectState, Snapshot};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::runs::proposal;
use crate::runs::{
    HistoryOptions, RunContext, RunInfo, RunMode, RunRequest, RunStatus, Runs, StartExtra, Trigger,
};
use crate::state::VaultState;

pub mod apply;
pub mod journal;
pub mod schedule;
pub mod store;

use apply::ApplyItem;
use schedule::{AutoUpdate, Candidate, OnResult, UpdateSources};
use store::{Finished, Outcome, ProjectUpdateView, RunEnd, TriggerKind, UpdateResult, UpdateStore};

/// Emitted with every project's [`ProjectUpdateView`] whenever one changes.
pub const UPDATES_CHANGED: &str = "updates-changed";
/// The recipe whose runs produce proposals.
pub const RECIPE: &str = "aktualizacja";
/// The recipe of a free-form request: the same review flow, never scheduled.
pub const REQUEST_RECIPE: &str = "zlecenie";
/// The first automatic check after the app started.
pub const STARTUP_DELAY: Duration = Duration::from_secs(90);

/// Managed state: the persisted update store.
pub struct Updates {
    path: PathBuf,
    store: Mutex<UpdateStore>,
}

impl Updates {
    pub fn new(config_dir: &std::path::Path) -> Self {
        let path = config_dir.join(store::STATE_FILE);
        Updates {
            store: Mutex::new(UpdateStore::load(&path)),
            path,
        }
    }

    fn save(&self, store: &UpdateStore) {
        if let Err(e) = store.save(&self.path) {
            crate::log(&format!(
                "updates: cannot save {}: {e}",
                self.path.display()
            ));
        }
    }
}

/// `n` with the Polish plural form: (1, "zmiana", "zmiany", "zmian").
pub fn plural(n: usize, one: &str, few: &str, many: &str) -> String {
    let word = if n == 1 {
        one
    } else if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) {
        few
    } else {
        many
    };
    format!("{n} {word}")
}

pub fn local_now() -> NaiveDateTime {
    crate::reminders::local_now()
}

/// `RunInfo.started_at` (`YYYY-MM-DDTHH:MM:SS`) at minute precision.
fn run_time(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .and_then(|t| t.with_second(0))
}

// --- automatic tick (pure decision + injected effects) ---------------------------

/// The effects an automatic tick needs; the app's are in [`TauriEnv`].
pub trait AutoEnv {
    fn busy(&self) -> bool;
    fn snapshot(&self) -> Arc<Snapshot>;
    /// The read-only git snapshot of the project's repositories.
    fn repo_state(&self, snap: &Snapshot, project: &str) -> String;
    /// The conversation-history digest of the project (history source on).
    fn history(
        &self,
        snap: &Snapshot,
        project: &str,
        repo_state: &str,
        days: u16,
    ) -> crate::runs::history::Digest;
    /// Starts the automatic background run; its id.
    fn start(
        &self,
        project: &str,
        cfg: &AutoUpdate,
        base: Arc<Snapshot>,
        repo_state: String,
        history: Option<HistoryOptions>,
    ) -> Result<String, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickOutcome {
    Idle,
    /// Something is due, but another background run is running.
    Busy(String),
    /// Nothing changed since the last check; the agent was not called.
    Unchanged(String),
    Started {
        project: String,
        run_id: String,
    },
    StartFailed {
        project: String,
        error: String,
    },
}

/// One scheduler pass at `now`.
pub fn auto_tick(
    env: &dyn AutoEnv,
    store: &Mutex<UpdateStore>,
    auto: &BTreeMap<String, AutoUpdate>,
    sources: &BTreeMap<String, UpdateSources>,
    now: NaiveDateTime,
) -> TickOutcome {
    let snap = env.snapshot();
    let due = {
        let st = crate::lock(store);
        let cands: Vec<Candidate> = auto
            .iter()
            .filter(|(slug, cfg)| {
                cfg.enabled()
                    && snap.project(slug).is_some_and(|p| {
                        p.state == ProjectState::Active && !p.repos.is_empty()
                    })
                    // Proposals waiting for review are never overwritten by
                    // an automatic check.
                    && !st
                        .projects
                        .get(slug.as_str())
                        .and_then(|p| p.proposal.as_ref())
                        .is_some_and(|p| p.pending() > 0)
            })
            .map(|(slug, cfg)| Candidate {
                project: slug,
                cfg,
                clock: st.projects.get(slug.as_str()).and_then(|p| p.clock),
            })
            .collect();
        schedule::pick(now, &cands).map(str::to_string)
    };
    let Some(project) = due else {
        return TickOutcome::Idle;
    };
    if env.busy() {
        return TickOutcome::Busy(project);
    }
    let Some(cfg) = auto.get(&project).copied() else {
        return TickOutcome::Idle;
    };
    let repo_state = env.repo_state(&snap, &project);
    let src = sources.get(&project).copied().unwrap_or_default();
    let digest = src
        .history
        .then(|| env.history(&snap, &project, &repo_state, src.history_days));
    let input = crate::runs::runner::fingerprint_input(&repo_state, digest.as_ref());
    let fp = proposal::fingerprint(&input, Some(&snap), &project);
    {
        let mut st = crate::lock(store);
        let p = st.project(&project);
        if p.fingerprint.as_deref() == Some(fp.as_str()) {
            store::record_unchanged(p, cfg.provider, now);
            return TickOutcome::Unchanged(project);
        }
        // Set before starting: the check counts from now, whatever happens.
        p.clock = Some(now);
    }
    let history = digest.map(|d| HistoryOptions {
        days: src.history_days,
        prepared: Some(d),
        focus: None,
    });
    match env.start(&project, &cfg, snap, repo_state, history) {
        Ok(run_id) => TickOutcome::Started { project, run_id },
        Err(error) => {
            store::record_start_failure(
                crate::lock(store).project(&project),
                cfg.provider,
                now,
                error.clone(),
            );
            TickOutcome::StartFailed { project, error }
        }
    }
}

// --- Tauri glue -------------------------------------------------------------------

fn auto_settings<R: Runtime>(app: &AppHandle<R>) -> BTreeMap<String, AutoUpdate> {
    app.try_state::<crate::native::Native>()
        .map(|n| n.settings().auto_update)
        .unwrap_or_default()
}

fn source_settings<R: Runtime>(app: &AppHandle<R>) -> BTreeMap<String, UpdateSources> {
    app.try_state::<crate::native::Native>()
        .map(|n| n.settings().update_sources)
        .unwrap_or_default()
}

/// The model / effort of `provider` from Settings → "Agenci".
pub fn agent_choice<R: Runtime>(
    app: &AppHandle<R>,
    provider: crate::runs::ProviderId,
) -> crate::runs::agent::AgentChoice {
    app.try_state::<crate::native::Native>()
        .map(|n| n.settings().agents.get(provider).clone())
        .unwrap_or_default()
}

/// The history source of `project` for a run (`None` when off).
pub fn history_options<R: Runtime>(app: &AppHandle<R>, project: &str) -> Option<HistoryOptions> {
    let src = source_settings(app)
        .get(project)
        .copied()
        .unwrap_or_default();
    src.history.then_some(HistoryOptions {
        days: src.history_days,
        prepared: None,
        focus: None,
    })
}

/// When `project` was last checked (the schedule clock: a manual or
/// automatic check, or a skipped automatic one), `YYYY-MM-DDTHH:MM`.
pub fn last_check<R: Runtime>(app: &AppHandle<R>, project: &str) -> Option<String> {
    let u = app.try_state::<Updates>()?;
    let st = crate::lock(&u.store);
    st.projects
        .get(project)
        .and_then(|p| p.clock)
        .map(|c| montybrain_core::minute::format(&c))
}

/// Every project's view, for `list_updates` and [`UPDATES_CHANGED`].
pub fn list<R: Runtime>(app: &AppHandle<R>) -> Vec<ProjectUpdateView> {
    let Some(u) = app.try_state::<Updates>() else {
        return Vec::new();
    };
    let st = crate::lock(&u.store).clone();
    store::views(&st, &auto_settings(app), &source_settings(app), local_now())
}

pub fn emit_changed<R: Runtime>(app: &AppHandle<R>) {
    if let Err(e) = app.emit(UPDATES_CHANGED, list(app)) {
        crate::log(&format!("emit {UPDATES_CHANGED} failed: {e}"));
    }
}

fn notify<R: Runtime>(app: &AppHandle<R>, title: &str, body: &str) {
    crate::log(&format!("updates: notify {title} — {body}"));
    if let Err(e) = crate::notifier::deliver(app, title, body) {
        crate::log(&format!("updates: notification failed: {e}"));
    }
}

fn project_name(snap: &Snapshot, slug: &str) -> String {
    snap.project(slug)
        .map_or_else(|| slug.to_string(), |p| p.name.clone())
}

/// Reloads the vault after the app wrote it and tells the window.
fn vault_written<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<VaultState>();
    if let Some(version) = state.reload() {
        crate::emit_changed(app, version, crate::ChangeSource::Mutation, Vec::new());
    }
}

/// Applies changes of the stored proposal of `project` (store lock held by
/// the caller) and updates its last result and fingerprint.
fn apply_locked<R: Runtime>(
    app: &AppHandle<R>,
    st: &mut store::ProjectUpdate,
    project: &str,
    indices: &[usize],
    trigger: TriggerKind,
) -> (Vec<ApplyItem>, Option<String>) {
    let state = app.state::<VaultState>();
    let vault = state.vault();
    let now = local_now();
    let before = vault
        .load()
        .ok()
        .map(|s| proposal::vault_fingerprint(&s, project));
    let Some(prop) = st.proposal.as_mut() else {
        return (Vec::new(), None);
    };
    let automatic = prop.automatic_indices();
    let safe: Vec<_> = indices
        .iter()
        .copied()
        .filter(|i| trigger != TriggerKind::Auto || automatic.contains(i))
        .collect();
    let (items, written) = apply::apply(vault, project, prop, &safe, now);
    let mut token = None;
    if !written.is_empty() {
        let t = format!(
            "{}#{}",
            prop.run_id,
            UNDO_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        st.undo = Some(apply::UndoRecord {
            token: t.clone(),
            run_id: prop.run_id.clone(),
            at: now,
            items: written,
        });
        token = Some(t);
    }
    let Some(prop) = st.proposal.as_mut() else {
        return (items, token);
    };
    let applied_now = items
        .iter()
        .filter(|i| i.status == apply::ApplyStatus::Applied)
        .count();
    if applied_now > 0 {
        let total = prop.applied();
        let provider = prop.provider;
        vault_written(app);
        st.last = Some(UpdateResult {
            at: now,
            provider: Some(provider),
            trigger,
            outcome: Outcome::Applied { count: total },
            history_note: prop.history_note.clone(),
            request: prop.request.clone(),
        });
        // The app wrote the vault itself: if nothing else changed it since the
        // check, the next automatic check need not run just because of that.
        refresh_fingerprint(st, vault, project, before);
    }
    (items, token)
}

/// The app wrote the vault itself: if nothing else changed it since the
/// check, the next automatic check need not run just because of that.
fn refresh_fingerprint(
    st: &mut store::ProjectUpdate,
    vault: &montybrain_core::Vault,
    project: &str,
    before: Option<String>,
) {
    if let (Some(fp), Some(before), Ok(after)) = (&st.fingerprint, before, vault.load()) {
        let after = proposal::vault_fingerprint(&after, project);
        st.fingerprint =
            proposal::refresh_vault_part(fp, &before, &after).or(st.fingerprint.take());
    }
}

static UNDO_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The undo record with `token`, taken out (an undo runs at most once).
pub fn take_undo(st: &mut store::ProjectUpdate, token: &str) -> Result<apply::UndoRecord, String> {
    match &st.undo {
        Some(r) if r.token == token => Ok(st.undo.take().expect("checked")),
        _ => Err("Tej zmiany nie można już cofnąć: została cofnięta albo zastąpiona.".into()),
    }
}

/// `undo_update` result.
#[derive(Debug, Clone, Serialize)]
pub struct UndoReport {
    pub project: String,
    pub items: Vec<apply::UndoResult>,
    pub updates: Vec<ProjectUpdateView>,
}

/// Takes back the writes of the apply click `token` of run `run_id`.
pub fn undo_changes<R: Runtime>(
    app: &AppHandle<R>,
    run_id: &str,
    token: &str,
) -> Result<UndoReport, String> {
    let updates = app.state::<Updates>();
    let (project, items) = {
        let mut st = crate::lock(&updates.store);
        let project = st
            .projects
            .iter()
            .find(|(_, p)| p.undo.as_ref().is_some_and(|u| u.run_id == run_id))
            .map(|(k, _)| k.clone())
            .ok_or("Tej zmiany nie można już cofnąć: została cofnięta albo zastąpiona.")?;
        let p = st.project(&project);
        let record = take_undo(p, token)?;
        let state = app.state::<VaultState>();
        let vault = state.vault();
        let before = vault
            .load()
            .ok()
            .map(|s| proposal::vault_fingerprint(&s, &project));
        let items = apply::undo(vault, &project, &record, p.proposal.as_mut());
        let wrote = record.items.iter().any(apply::UndoItem::writes);
        if !wrote {
            // A rejection taken back: the proposal is under review again.
            if let Some(prop) = &p.proposal
                && let Some(last) = p.last.as_mut()
                && prop.pending() > 0
                && prop.applied() == 0
            {
                last.outcome = Outcome::Proposed {
                    count: prop.pending(),
                };
            }
        } else if items.iter().any(|i| {
            matches!(
                i.status,
                apply::UndoStatus::Undone | apply::UndoStatus::Partial
            )
        }) {
            vault_written(app);
            refresh_fingerprint(p, vault, &project, before);
            if let Some(prop) = &p.proposal {
                let now = local_now();
                let outcome = match prop.applied() {
                    0 => Outcome::Proposed {
                        count: prop.pending(),
                    },
                    n => Outcome::Applied { count: n },
                };
                p.last = Some(UpdateResult {
                    at: now,
                    provider: Some(prop.provider),
                    trigger: TriggerKind::Manual,
                    outcome,
                    history_note: prop.history_note.clone(),
                    request: prop.request.clone(),
                });
            }
        }
        updates.save(&st);
        (project, items)
    };
    emit_changed(app);
    Ok(UndoReport {
        project,
        items,
        updates: list(app),
    })
}

/// Called by the run sink when a background run ended.
pub fn run_finished<R: Runtime>(app: &AppHandle<R>, info: &RunInfo, ctx: &RunContext) {
    let inbox = info.recipe == crate::runs::recipes::INBOX_RECIPE;
    if !(info.recipe == RECIPE || info.recipe == REQUEST_RECIPE || inbox)
        || info.mode != RunMode::Background
    {
        return;
    }
    let project = if inbox {
        proposal::INBOX.to_string()
    } else {
        let Some(project) = ctx.project.clone() else {
            return;
        };
        project
    };
    let Some(updates) = app.try_state::<Updates>() else {
        return;
    };
    let now = local_now();
    let current = app.state::<VaultState>().snapshot();
    let base = ctx.base.clone().unwrap_or_else(|| current.clone());
    let end = match info.status {
        RunStatus::Succeeded => RunEnd::Parsed(if let Some(topic) = &ctx.topic {
            proposal::parse_journal(
                info.result.as_deref().unwrap_or_default(),
                &base,
                &project,
                topic,
                ctx.request.as_deref().unwrap_or_default(),
            )
        } else {
            proposal::parse(info.result.as_deref().unwrap_or_default(), &base, &project)
        }),
        RunStatus::Failed => RunEnd::Failed(
            info.error
                .clone()
                .unwrap_or_else(|| "Uruchomienie nie powiodło się.".into()),
        ),
        RunStatus::Cancelled => RunEnd::Cancelled,
        RunStatus::Running => return,
    };
    let trigger = TriggerKind::from(ctx.trigger);
    let finished = Finished {
        run_id: &info.id,
        provider: info.provider,
        trigger,
        started_at: run_time(&info.started_at).unwrap_or(now),
        now,
        fingerprint: ctx.fingerprint.clone(),
        history_note: ctx.history_note.clone(),
        request: ctx.request.clone(),
    };
    let name = project_name(&current, &project);
    let cfg = auto_settings(app)
        .get(&project)
        .copied()
        .unwrap_or_default();
    let mut note: Option<(String, String)> = None;
    {
        let mut st = crate::lock(&updates.store);
        let p = st.project(&project);
        if let (Some(topic), Some(input)) = (&ctx.topic, &ctx.journal_refresh) {
            journal::advance(p, topic, input, &end);
        }
        let pending = store::record_finished(p, &finished, end);
        if let Some(prop) = p.proposal.as_mut().filter(|p| p.run_id == info.id) {
            prop.topic = ctx.topic.clone();
        }
        if let (Some(n), Trigger::Auto) = (pending, ctx.trigger) {
            match cfg.on_result {
                OnResult::Propose => {
                    note = Some((
                        format!(
                            "{name}: {} zmian",
                            plural(n, "propozycja", "propozycje", "propozycji")
                        ),
                        "Automatyczna aktualizacja czeka na przegląd w widoku projektu.".into(),
                    ));
                }
                OnResult::Apply => {
                    let all: Vec<usize> = (0..n).collect();
                    let (items, _) = apply_locked(app, p, &project, &all, TriggerKind::Auto);
                    let ok = items
                        .iter()
                        .filter(|i| i.status == apply::ApplyStatus::Applied)
                        .count();
                    let skipped = items.len() - ok;
                    let mut body = format!("Automatyczna aktualizacja ({}).", info.provider.name());
                    if skipped > 0 {
                        body.push_str(&format!(" Pominięto: {skipped}."));
                    }
                    note = Some((
                        format!(
                            "{name}: zastosowano {}",
                            plural(ok, "zmianę", "zmiany", "zmian")
                        ),
                        body,
                    ));
                }
            }
        }
        updates.save(&st);
    }
    emit_changed(app);
    // A finished check moves the "since the last check" baseline: collect again.
    if info.recipe == RECIPE
        && let Some(d) = app.try_state::<crate::drift::Drift>()
    {
        d.poke();
    }
    if let Some((title, body)) = note {
        notify(app, &title, &body);
    }
}

/// The answer of a Terminal Skrzynka session ("Moje narzędzia"): the file it
/// wrote or the text the user pasted, parsed like a background answer and kept
/// as the Skrzynka's proposal (to tick, as usual). Nothing is written to the
/// vault here. Returns how many changes it proposes; an answer without a
/// usable block is an error and leaves the current proposal alone.
pub fn inbox_answer<R: Runtime>(
    app: &AppHandle<R>,
    run_id: &str,
    provider: crate::runs::ProviderId,
    answer: &str,
) -> Result<usize, String> {
    agent_answer(
        app,
        proposal::INBOX,
        run_id,
        provider,
        answer,
        "Skrzynka: sesja w Terminalu",
    )
}

/// An agent's answer from outside the background runner (a Terminal session,
/// an in-app session, pasted text) as the proposal of `project` (the inbox
/// key or a project slug), labelled with `request`.
pub fn agent_answer<R: Runtime>(
    app: &AppHandle<R>,
    project: &str,
    run_id: &str,
    provider: crate::runs::ProviderId,
    answer: &str,
    request: &str,
) -> Result<usize, String> {
    let updates = app
        .try_state::<Updates>()
        .ok_or("Propozycje są jeszcze niedostępne.")?;
    let now = local_now();
    let current = app.state::<VaultState>().snapshot();
    if project != proposal::INBOX && current.project(project).is_none() {
        return Err("Nie ma już takiego projektu.".into());
    }
    let parsed = proposal::parse(answer, &current, project);
    if let Some(problem) = &parsed.problem {
        return Err(format!(
            "W odpowiedzi agenta nie ma propozycji do wczytania: {problem}"
        ));
    }
    let n = parsed.changes.len();
    let finished = Finished {
        run_id,
        provider,
        trigger: TriggerKind::Manual,
        started_at: now,
        now,
        fingerprint: None,
        history_note: None,
        request: Some(request.to_string()),
    };
    {
        let mut st = crate::lock(&updates.store);
        let p = st.project(project);
        store::record_finished(p, &finished, RunEnd::Parsed(parsed));
        updates.save(&st);
    }
    emit_changed(app);
    Ok(n)
}

/// `apply_update` result.
#[derive(Debug, Clone, Serialize)]
pub struct ApplyReport {
    pub project: String,
    pub items: Vec<ApplyItem>,
    /// Token for `undo_update` when something was written.
    pub undo: Option<String>,
    pub updates: Vec<ProjectUpdateView>,
}

/// Applies the changes `indices` of the proposal of run `run_id`.
pub fn apply_changes<R: Runtime>(
    app: &AppHandle<R>,
    run_id: &str,
    indices: &[usize],
) -> Result<ApplyReport, String> {
    let updates = app.state::<Updates>();
    let (project, items, undo) = {
        let mut st = crate::lock(&updates.store);
        let (project, p) = st
            .find_run(run_id)
            .ok_or("Tej propozycji już nie ma: została odrzucona albo zastąpiona nowszą.")?;
        let project = project.clone();
        let (items, undo) = apply_locked(app, p, &project, indices, TriggerKind::Manual);
        updates.save(&st);
        (project, items, undo)
    };
    emit_changed(app);
    // Applied changes may have stored branch links.
    crate::drift::emit_changed(app);
    Ok(ApplyReport {
        project,
        items,
        undo,
        updates: list(app),
    })
}

/// `dismiss_update` result.
#[derive(Debug, Clone, Serialize)]
pub struct DismissReport {
    /// How many changes were rejected.
    pub count: usize,
    /// Token for `undo_update` (the rejection is the last action until another one).
    pub undo: Option<String>,
    pub updates: Vec<ProjectUpdateView>,
}

/// Rejects the undecided changes of the proposal of run `run_id` (nothing is
/// written to the vault; the proposal stays as the last result). Undoable
/// like an apply click ([`undo_changes`]): the rows become undecided again,
/// with the user's corrections kept.
pub fn dismiss<R: Runtime>(app: &AppHandle<R>, run_id: &str) -> Result<DismissReport, String> {
    let updates = app.state::<Updates>();
    let (count, undo) = {
        let mut st = crate::lock(&updates.store);
        let mut out = (0, None);
        if let Some((_, p)) = st.find_run(run_id) {
            out = dismiss_locked(p, local_now());
            updates.save(&st);
        }
        out
    };
    emit_changed(app);
    Ok(DismissReport {
        count,
        undo,
        updates: list(app),
    })
}

/// [`dismiss`] on the project's state: rejects, and keeps the rejection as
/// the undoable last action.
pub fn dismiss_locked(p: &mut store::ProjectUpdate, now: NaiveDateTime) -> (usize, Option<String>) {
    let Some(prop) = p.proposal.as_mut() else {
        return (0, None);
    };
    let items = apply::reject(prop, now);
    if items.is_empty() {
        return (0, None);
    }
    let token = format!(
        "{}#{}",
        prop.run_id,
        UNDO_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let count = items.len();
    p.undo = Some(apply::UndoRecord {
        token: token.clone(),
        run_id: prop.run_id.clone(),
        at: now,
        items,
    });
    (count, Some(token))
}

/// Corrects the undecided change `index` of the proposal of run `run_id`
/// against the vault as it is now ([`store::edit_change`]).
pub fn edit_change<R: Runtime>(
    app: &AppHandle<R>,
    run_id: &str,
    index: usize,
    edit: &proposal::ChangeEdit,
) -> Result<Vec<ProjectUpdateView>, String> {
    let updates = app.state::<Updates>();
    let snap = app.state::<VaultState>().snapshot();
    {
        let mut st = crate::lock(&updates.store);
        let (project, p) = st
            .find_run(run_id)
            .ok_or("Tej propozycji już nie ma: została odrzucona albo zastąpiona nowszą.")?;
        let project = project.clone();
        let prop = p.proposal.as_mut().expect("find_run matched a proposal");
        store::edit_change(prop, index, edit, &snap, &project)?;
        updates.save(&st);
    }
    emit_changed(app);
    Ok(list(app))
}

/// `join_update` result.
#[derive(Debug, Clone, Serialize)]
pub struct JoinReport {
    /// Tasks left out because the topic has them already.
    pub skipped: Vec<String>,
    pub updates: Vec<ProjectUpdateView>,
}

/// "Dołącz do tematu…" on the new-topic change `index` of run `run_id`.
pub fn join_change<R: Runtime>(
    app: &AppHandle<R>,
    run_id: &str,
    index: usize,
    topic: &str,
) -> Result<JoinReport, String> {
    let updates = app.state::<Updates>();
    let snap = app.state::<VaultState>().snapshot();
    let skipped = {
        let mut st = crate::lock(&updates.store);
        let (project, p) = st
            .find_run(run_id)
            .ok_or("Tej propozycji już nie ma: została odrzucona albo zastąpiona nowszą.")?;
        let project = project.clone();
        let prop = p.proposal.as_mut().expect("find_run matched a proposal");
        let skipped = store::join_change(prop, index, topic, &snap, &project)?;
        updates.save(&st);
        skipped
    };
    emit_changed(app);
    Ok(JoinReport {
        skipped,
        updates: list(app),
    })
}

/// Sets the automatic update of `project` (it must exist).
pub fn set_auto<R: Runtime>(
    app: &AppHandle<R>,
    project: &str,
    cfg: AutoUpdate,
) -> Result<Vec<ProjectUpdateView>, String> {
    cfg.validate()?;
    if app
        .state::<VaultState>()
        .snapshot()
        .project(project)
        .is_none()
    {
        return Err(format!("Nie ma projektu „{project}”."));
    }
    crate::native::set_auto_update(app, project, cfg)?;
    emit_changed(app);
    Ok(list(app))
}

/// Sets the update sources of `project` (it must exist).
pub fn set_sources<R: Runtime>(
    app: &AppHandle<R>,
    project: &str,
    src: UpdateSources,
) -> Result<Vec<ProjectUpdateView>, String> {
    src.validate()?;
    if app
        .state::<VaultState>()
        .snapshot()
        .project(project)
        .is_none()
    {
        return Err(format!("Nie ma projektu „{project}”."));
    }
    crate::native::set_update_sources(app, project, src)?;
    emit_changed(app);
    Ok(list(app))
}

/// The app's effects for [`auto_tick`].
struct TauriEnv<R: Runtime>(AppHandle<R>);

impl<R: Runtime> AutoEnv for TauriEnv<R> {
    fn busy(&self) -> bool {
        self.0.state::<Runs>().0.is_busy()
    }

    fn snapshot(&self) -> Arc<Snapshot> {
        self.0.state::<VaultState>().snapshot()
    }

    fn repo_state(&self, snap: &Snapshot, project: &str) -> String {
        self.0.state::<Runs>().0.repo_state(&snap.projects, project)
    }

    fn history(
        &self,
        snap: &Snapshot,
        project: &str,
        repo_state: &str,
        days: u16,
    ) -> crate::runs::history::Digest {
        self.0
            .state::<Runs>()
            .0
            .history_digest(&snap.projects, project, repo_state, days)
    }

    fn start(
        &self,
        project: &str,
        cfg: &AutoUpdate,
        base: Arc<Snapshot>,
        repo_state: String,
        history: Option<HistoryOptions>,
    ) -> Result<String, String> {
        let req = RunRequest {
            recipe: RECIPE.into(),
            provider: cfg.provider.as_str().into(),
            mode: "background".into(),
            params: BTreeMap::from([("project".to_string(), project.to_string())]),
        };
        let state = self.0.state::<VaultState>();
        let runs = self.0.state::<Runs>().0.clone();
        let projects = base.projects.clone();
        runs.start_with(
            &req,
            &projects,
            state.vault().root(),
            StartExtra {
                base: Some(base),
                repo_state: Some(repo_state),
                trigger: Trigger::Auto,
                agent: agent_choice(&self.0, cfg.provider),
                history,
                journal_refresh: None,
            },
        )
        .map(|info| info.id)
    }
}

fn until_next_minute() -> Duration {
    let now = Local::now();
    Duration::from_secs(60)
        .saturating_sub(Duration::from_secs(u64::from(now.second())))
        .max(Duration::from_secs(1))
        + Duration::from_millis(500)
}

/// Manages [`Updates`] and starts the scheduler thread. Called once from
/// the native setup, after the run manager exists.
pub fn start<R: Runtime>(app: &AppHandle<R>, config_dir: &std::path::Path) {
    app.manage(Updates::new(config_dir));
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("montybrain-updates".into())
        .spawn(move || {
            std::thread::sleep(STARTUP_DELAY);
            loop {
                tick(&app);
                std::thread::sleep(until_next_minute());
            }
        });
    if let Err(e) = spawned {
        crate::log(&format!("updates: scheduler failed to start: {e}"));
    }
}

fn tick<R: Runtime>(app: &AppHandle<R>) {
    let auto = auto_settings(app);
    if !auto.values().any(AutoUpdate::enabled) {
        return;
    }
    let Some(updates) = app.try_state::<Updates>() else {
        return;
    };
    let outcome = auto_tick(
        &TauriEnv(app.clone()),
        &updates.store,
        &auto,
        &source_settings(app),
        local_now(),
    );
    match &outcome {
        TickOutcome::Idle => return,
        TickOutcome::Busy(p) => {
            crate::log(&format!("updates: {p} is due, another run is busy; later"));
            return;
        }
        other => crate::log(&format!("updates: {other:?}")),
    }
    updates.save(&crate::lock(&updates.store));
    emit_changed(app);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::ProviderId;
    use crate::runs::proposal::tests::snap;
    use crate::updates::schedule::Schedule;
    use std::cell::{Cell, RefCell};

    fn at(s: &str) -> NaiveDateTime {
        montybrain_core::minute::parse(s).unwrap()
    }

    struct Fake {
        snap: Arc<Snapshot>,
        busy: Cell<bool>,
        git: RefCell<String>,
        fail: Cell<bool>,
        started: RefCell<Vec<String>>,
        git_calls: Cell<usize>,
        digest: RefCell<String>,
        history_calls: Cell<usize>,
        last_history: RefCell<Option<HistoryOptions>>,
    }

    impl Fake {
        fn new() -> Self {
            Fake {
                snap: Arc::new(snap()),
                busy: Cell::new(false),
                git: RefCell::new("git A".into()),
                fail: Cell::new(false),
                started: RefCell::new(Vec::new()),
                git_calls: Cell::new(0),
                digest: RefCell::new("sesja A".into()),
                history_calls: Cell::new(0),
                last_history: RefCell::new(None),
            }
        }
    }

    impl AutoEnv for Fake {
        fn busy(&self) -> bool {
            self.busy.get()
        }
        fn snapshot(&self) -> Arc<Snapshot> {
            self.snap.clone()
        }
        fn repo_state(&self, _: &Snapshot, _: &str) -> String {
            self.git_calls.set(self.git_calls.get() + 1);
            self.git.borrow().clone()
        }
        fn history(
            &self,
            _: &Snapshot,
            _: &str,
            _: &str,
            days: u16,
        ) -> crate::runs::history::Digest {
            self.history_calls.set(self.history_calls.get() + 1);
            crate::runs::history::Digest {
                text: format!("{} ({days} dni)", self.digest.borrow()),
                claude: 1,
                ..Default::default()
            }
        }
        fn start(
            &self,
            project: &str,
            _: &AutoUpdate,
            _: Arc<Snapshot>,
            _: String,
            history: Option<HistoryOptions>,
        ) -> Result<String, String> {
            *self.last_history.borrow_mut() = history;
            if self.fail.get() {
                return Err("Nie znaleziono programu `codex`.".into());
            }
            self.started.borrow_mut().push(project.into());
            Ok(format!("run-{}", self.started.borrow().len()))
        }
    }

    fn every(h: u32) -> BTreeMap<String, AutoUpdate> {
        BTreeMap::from([(
            "beeking".to_string(),
            AutoUpdate {
                schedule: Schedule::Every { hours: h },
                provider: ProviderId::Codex,
                ..Default::default()
            },
        )])
    }

    /// The run of `project` completed with this fingerprint (what
    /// `run_finished` records).
    fn complete(store: &Mutex<UpdateStore>, fake: &Fake, when: &str) {
        let fp = proposal::fingerprint(&fake.git.borrow(), Some(&fake.snap), "beeking");
        let mut st = crate::lock(store);
        store::record_finished(
            st.project("beeking"),
            &Finished {
                run_id: "r",
                provider: ProviderId::Codex,
                trigger: TriggerKind::Auto,
                started_at: at(when),
                now: at(when),
                fingerprint: Some(fp),
                history_note: None,
                request: None,
            },
            RunEnd::Parsed(proposal::parse(
                &format!("R\n```{}\n{{\"changes\":[]}}\n```", proposal::FENCE_INFO),
                &fake.snap,
                "beeking",
            )),
        );
    }

    #[test]
    fn overdue_at_launch_runs_once_then_waits_the_interval() {
        let fake = Fake::new();
        let store = Mutex::new(UpdateStore::default());
        crate::lock(&store).project("beeking").clock = Some(at("2026-11-10T08:00"));
        let auto = every(6);
        // Days overdue: one run, not a burst of catch-up runs.
        let r = auto_tick(
            &fake,
            &store,
            &auto,
            &BTreeMap::new(),
            at("2026-11-14T12:00"),
        );
        assert_eq!(
            r,
            TickOutcome::Started {
                project: "beeking".into(),
                run_id: "run-1".into()
            }
        );
        for m in ["12:01", "12:30", "17:59"] {
            let now = at(&format!("2026-11-14T{m}"));
            assert_eq!(
                auto_tick(&fake, &store, &auto, &BTreeMap::new(), now),
                TickOutcome::Idle,
                "{m}"
            );
        }
        assert_eq!(fake.started.borrow().len(), 1);
        complete(&store, &fake, "2026-11-14T12:00");
        // Due again after a full interval; nothing changed → no agent call.
        assert_eq!(
            auto_tick(
                &fake,
                &store,
                &auto,
                &BTreeMap::new(),
                at("2026-11-14T18:00")
            ),
            TickOutcome::Unchanged("beeking".into())
        );
        let st = crate::lock(&store).projects["beeking"].clone();
        assert_eq!(st.last.unwrap().outcome, Outcome::Unchanged);
        assert_eq!(st.clock, Some(at("2026-11-14T18:00")));
        assert_eq!(fake.started.borrow().len(), 1);
        // A change in the repositories: the agent is called.
        *fake.git.borrow_mut() = "git B".into();
        assert_eq!(
            auto_tick(
                &fake,
                &store,
                &auto,
                &BTreeMap::new(),
                at("2026-11-14T23:59")
            ),
            TickOutcome::Idle
        );
        assert!(matches!(
            auto_tick(
                &fake,
                &store,
                &auto,
                &BTreeMap::new(),
                at("2026-11-15T00:00")
            ),
            TickOutcome::Started { .. }
        ));
    }

    #[test]
    fn busy_means_later_without_touching_the_clock() {
        let fake = Fake::new();
        let store = Mutex::new(UpdateStore::default());
        let auto = every(6);
        fake.busy.set(true);
        assert_eq!(
            auto_tick(
                &fake,
                &store,
                &auto,
                &BTreeMap::new(),
                at("2026-11-14T12:00")
            ),
            TickOutcome::Busy("beeking".into())
        );
        assert_eq!(fake.git_calls.get(), 0, "no git work while busy");
        assert_eq!(
            crate::lock(&store)
                .projects
                .get("beeking")
                .and_then(|p| p.clock),
            None
        );
        fake.busy.set(false);
        assert!(matches!(
            auto_tick(
                &fake,
                &store,
                &auto,
                &BTreeMap::new(),
                at("2026-11-14T12:01")
            ),
            TickOutcome::Started { .. }
        ));
    }

    #[test]
    fn a_failure_waits_a_full_interval_and_is_shown() {
        let fake = Fake::new();
        let store = Mutex::new(UpdateStore::default());
        let auto = every(3);
        fake.fail.set(true);
        let r = auto_tick(
            &fake,
            &store,
            &auto,
            &BTreeMap::new(),
            at("2026-11-14T12:00"),
        );
        assert!(matches!(r, TickOutcome::StartFailed { .. }), "{r:?}");
        let last = crate::lock(&store).projects["beeking"]
            .last
            .clone()
            .unwrap();
        assert_eq!(
            last.outcome,
            Outcome::Failed {
                message: "Nie znaleziono programu `codex`.".into(),
                limit: false,
                model: false
            }
        );
        // No retry loop: nothing until the interval passed.
        for m in 1..180 {
            let now = at("2026-11-14T12:00") + montybrain_core::chrono::TimeDelta::minutes(m);
            assert_eq!(
                auto_tick(&fake, &store, &auto, &BTreeMap::new(), now),
                TickOutcome::Idle
            );
        }
        fake.fail.set(false);
        assert!(matches!(
            auto_tick(
                &fake,
                &store,
                &auto,
                &BTreeMap::new(),
                at("2026-11-14T15:00")
            ),
            TickOutcome::Started { .. }
        ));
        // A failed run (as opposed to a failed start) also counts from its start.
        let mut st = crate::lock(&store);
        let p = st.project("beeking");
        store::record_finished(
            p,
            &Finished {
                run_id: "x",
                provider: ProviderId::Codex,
                trigger: TriggerKind::Auto,
                started_at: at("2026-11-14T15:00"),
                now: at("2026-11-14T15:10"),
                fingerprint: None,
                history_note: None,
                request: None,
            },
            RunEnd::Failed("limit".into()),
        );
        assert_eq!(p.clock, Some(at("2026-11-14T15:00")));
        assert_eq!(
            p.fingerprint, None,
            "a failure leaves no fingerprint to skip on"
        );
    }

    #[test]
    fn off_archived_and_unknown_projects_never_run() {
        let mut fake = Fake::new();
        let store = Mutex::new(UpdateStore::default());
        let mut auto = every(1);
        auto.insert("nie-ma".into(), auto["beeking"]);
        auto.get_mut("beeking").unwrap().schedule = Schedule::Off;
        assert_eq!(
            auto_tick(
                &fake,
                &store,
                &auto,
                &BTreeMap::new(),
                at("2026-11-14T12:00")
            ),
            TickOutcome::Idle
        );
        let mut s = snap();
        s.projects[0].state = ProjectState::Archived;
        fake.snap = Arc::new(s);
        let auto = every(1);
        assert_eq!(
            auto_tick(
                &fake,
                &store,
                &auto,
                &BTreeMap::new(),
                at("2026-11-14T12:00")
            ),
            TickOutcome::Idle
        );
    }

    #[test]
    fn a_manual_run_resets_the_clock() {
        let fake = Fake::new();
        let store = Mutex::new(UpdateStore::default());
        let auto = every(6);
        {
            let mut st = crate::lock(&store);
            store::record_finished(
                st.project("beeking"),
                &Finished {
                    run_id: "m",
                    provider: ProviderId::Claude,
                    trigger: TriggerKind::Manual,
                    started_at: at("2026-11-14T11:00"),
                    now: at("2026-11-14T11:05"),
                    fingerprint: None,
                    history_note: None,
                    request: None,
                },
                RunEnd::Failed("x".into()),
            );
        }
        assert_eq!(
            auto_tick(
                &fake,
                &store,
                &auto,
                &BTreeMap::new(),
                at("2026-11-14T16:59")
            ),
            TickOutcome::Idle
        );
        assert!(matches!(
            auto_tick(
                &fake,
                &store,
                &auto,
                &BTreeMap::new(),
                at("2026-11-14T17:00")
            ),
            TickOutcome::Started { .. }
        ));
    }

    #[test]
    fn the_scheduler_ignores_requests() {
        let fake = Fake::new();
        let store = Mutex::new(UpdateStore::default());
        let auto = every(6);
        let tick = |when| auto_tick(&fake, &store, &auto, &BTreeMap::new(), at(when));
        assert!(matches!(
            tick("2026-11-14T10:00"),
            TickOutcome::Started { .. }
        ));
        complete(&store, &fake, "2026-11-14T10:00");
        // A request finished later: it is not a check, so neither the clock
        // nor the skip-if-unchanged fingerprint move.
        {
            let mut st = crate::lock(&store);
            store::record_finished(
                st.project("beeking"),
                &Finished {
                    run_id: "z",
                    provider: ProviderId::Claude,
                    trigger: TriggerKind::Manual,
                    started_at: at("2026-11-14T11:00"),
                    now: at("2026-11-14T11:05"),
                    fingerprint: Some("other".into()),
                    history_note: None,
                    request: Some("rozpisz temat kasy".into()),
                },
                RunEnd::Parsed(proposal::parse(
                    &format!("R\n```{}\n{{\"changes\":[]}}\n```", proposal::FENCE_INFO),
                    &fake.snap,
                    "beeking",
                )),
            );
        }
        assert_eq!(tick("2026-11-14T15:59"), TickOutcome::Idle);
        // Due 6 h after the check (not after the request) and skipped because
        // nothing changed since the check.
        assert_eq!(
            tick("2026-11-14T16:00"),
            TickOutcome::Unchanged("beeking".into())
        );
        assert_eq!(fake.started.borrow().len(), 1, "only the first check ran");
        let st = crate::lock(&store);
        let last = st.projects["beeking"].last.as_ref().unwrap();
        assert_eq!(
            last.trigger,
            TriggerKind::Auto,
            "the skipped check is the last result"
        );
    }

    #[test]
    fn pending_proposals_hold_automatic_checks_back() {
        let mut fake = Fake::new();
        let store = Mutex::new(UpdateStore::default());
        let auto = every(1);
        {
            let mut st = crate::lock(&store);
            let p = st.project("beeking");
            store::record_finished(
                p,
                &Finished {
                    run_id: "r",
                    provider: ProviderId::Codex,
                    trigger: TriggerKind::Auto,
                    started_at: at("2026-11-14T08:00"),
                    now: at("2026-11-14T08:05"),
                    fingerprint: None,
                    history_note: None,
                    request: None,
                },
                RunEnd::Parsed(proposal::parse(
                    &format!(
                        "R\n```{}\n{{\"changes\":[{{\"kind\":\"new_task\",\"title\":\"N\",\"reason\":\"r\"}}]}}\n```",
                        proposal::FENCE_INFO
                    ),
                    &fake.snap,
                    "beeking",
                )),
            );
            p.clock = Some(at("2026-11-14T08:00"));
            assert_eq!(p.proposal.as_ref().unwrap().pending(), 1);
        }
        // Long overdue, but 1 proposal waits: nothing starts, no git work.
        for h in ["10:00", "15:00", "23:00"] {
            let now = at(&format!("2026-11-14T{h}"));
            assert_eq!(
                auto_tick(&fake, &store, &auto, &BTreeMap::new(), now),
                TickOutcome::Idle,
                "{h}"
            );
        }
        assert_eq!(fake.git_calls.get(), 0);
        let views = store::views(
            &crate::lock(&store),
            &auto,
            &BTreeMap::new(),
            at("2026-11-14T23:00"),
        );
        assert!(views[0].held && views[0].next_due.is_none());
        // Decided (rejected): checks resume.
        store::reject_pending(
            crate::lock(&store)
                .project("beeking")
                .proposal
                .as_mut()
                .unwrap(),
            at("2026-11-14T23:01"),
        );
        assert!(matches!(
            auto_tick(
                &fake,
                &store,
                &auto,
                &BTreeMap::new(),
                at("2026-11-14T23:02")
            ),
            TickOutcome::Started { .. }
        ));
        // A project without repositories is never checked automatically.
        let mut s = snap();
        s.projects[0].repos.clear();
        fake.snap = Arc::new(s);
        let store = Mutex::new(UpdateStore::default());
        assert_eq!(
            auto_tick(
                &fake,
                &store,
                &auto,
                &BTreeMap::new(),
                at("2026-11-15T12:00")
            ),
            TickOutcome::Idle
        );
    }

    #[test]
    fn an_undo_runs_once_and_only_for_the_last_click() {
        let mut st = store::ProjectUpdate {
            undo: Some(apply::UndoRecord {
                token: "r#2".into(),
                run_id: "r".into(),
                at: at("2026-11-14T10:00"),
                items: vec![],
            }),
            ..Default::default()
        };
        assert!(take_undo(&mut st, "r#1").is_err(), "an older click");
        assert!(take_undo(&mut st, "r#2").is_ok());
        assert!(take_undo(&mut st, "r#2").is_err(), "undo once");
        assert!(st.undo.is_none());
    }

    #[test]
    fn a_rejection_is_undone_once_with_the_users_corrections_kept() {
        let snap = proposal::tests::snap();
        let parsed = proposal::parse(
            &format!(
                "R\n```{}\n{}\n```",
                proposal::FENCE_INFO,
                r#"{"changes":[{"kind":"new_task","title":"A","topic":"platnosci","reason":"r"},{"kind":"new_task","title":"B","topic":"platnosci","reason":"r"},{"kind":"task_status","task":"aaaaa1","status":"done","reason":"r"}]}"#
            ),
            &snap,
            "beeking",
        );
        let mut prop = store::StoredProposal::from_proposal(
            parsed,
            "r",
            ProviderId::Codex,
            TriggerKind::Manual,
            at("2026-11-14T10:00"),
        );
        // The user corrected one row and decided another one before.
        store::edit_change(
            &mut prop,
            0,
            &proposal::ChangeEdit::NewTask {
                title: "A poprawione".into(),
            },
            &snap,
            "beeking",
        )
        .unwrap();
        prop.changes[2].outcome = Some(store::ChangeOutcome {
            status: store::ChangeStatus::Applied,
            message: "Zastosowano.".into(),
            at: at("2026-11-14T10:01"),
        });
        let mut st = store::ProjectUpdate {
            proposal: Some(prop),
            ..Default::default()
        };
        let (n, token) = dismiss_locked(&mut st, at("2026-11-14T10:02"));
        assert_eq!(n, 2);
        let token = token.unwrap();
        assert_eq!(st.proposal.as_ref().unwrap().pending(), 0);
        let v = store::views(
            &UpdateStore {
                projects: BTreeMap::from([("beeking".to_string(), st.clone())]),
            },
            &BTreeMap::new(),
            &BTreeMap::new(),
            at("2026-11-14T10:03"),
        );
        assert_eq!(
            (v[0].undo.as_deref(), v[0].undo_rejection),
            (Some(token.as_str()), true)
        );

        let record = take_undo(&mut st, &token).unwrap();
        assert!(take_undo(&mut st, &token).is_err(), "at most once");
        let dir = tempfile::tempdir().unwrap();
        let vault = montybrain_core::Vault::init(dir.path()).unwrap();
        let r = apply::undo(&vault, "beeking", &record, st.proposal.as_mut());
        assert!(
            r.iter().all(|i| i.status == apply::UndoStatus::Undone),
            "{r:?}"
        );
        let prop = st.proposal.as_ref().unwrap();
        assert_eq!(prop.pending(), 2);
        assert_eq!(prop.applied(), 1, "the applied row stays applied");
        assert!(prop.changes[0].edited);
        assert!(
            matches!(&prop.changes[0].change, proposal::Change::NewTask { title, .. } if title == "A poprawione")
        );
        // Nothing left to reject twice; nothing to undo.
        assert_eq!(dismiss_locked(&mut st, at("2026-11-14T10:04")).0, 2);
        assert_eq!(dismiss_locked(&mut st, at("2026-11-14T10:05")), (0, None));
    }

    #[test]
    fn the_history_digest_is_part_of_the_fingerprint() {
        let fake = Fake::new();
        let store = Mutex::new(UpdateStore::default());
        let auto = every(6);
        let sources = BTreeMap::from([(
            "beeking".to_string(),
            UpdateSources {
                history: true,
                history_days: 7,
            },
        )]);
        let r = auto_tick(&fake, &store, &auto, &sources, at("2026-11-14T12:00"));
        assert!(matches!(r, TickOutcome::Started { .. }), "{r:?}");
        let h = fake.last_history.borrow().clone().unwrap();
        assert_eq!(h.days, 7);
        assert_eq!(h.prepared.unwrap().text, "sesja A (7 dni)");
        // The run completed with the fingerprint of git + digest.
        let digest = crate::runs::history::Digest {
            text: "sesja A (7 dni)".into(),
            ..Default::default()
        };
        let input = crate::runs::runner::fingerprint_input(&fake.git.borrow(), Some(&digest));
        crate::lock(&store).project("beeking").fingerprint =
            Some(proposal::fingerprint(&input, Some(&fake.snap), "beeking"));
        assert_eq!(
            auto_tick(&fake, &store, &auto, &sources, at("2026-11-14T18:00")),
            TickOutcome::Unchanged("beeking".into())
        );
        // A new session (same repositories) is a change.
        *fake.digest.borrow_mut() = "sesja A, sesja B".into();
        assert!(matches!(
            auto_tick(&fake, &store, &auto, &sources, at("2026-11-15T00:00")),
            TickOutcome::Started { .. }
        ));
        // History off: no digest is collected at all.
        let calls = fake.history_calls.get();
        let store = Mutex::new(UpdateStore::default());
        auto_tick(
            &fake,
            &store,
            &auto,
            &BTreeMap::new(),
            at("2026-11-15T12:00"),
        );
        assert_eq!(fake.history_calls.get(), calls);
        assert!(fake.last_history.borrow().is_none());
    }

    #[test]
    fn helpers() {
        assert_eq!(plural(1, "zmiana", "zmiany", "zmian"), "1 zmiana");
        assert_eq!(plural(3, "zmiana", "zmiany", "zmian"), "3 zmiany");
        assert_eq!(plural(12, "zmiana", "zmiany", "zmian"), "12 zmian");
        assert_eq!(plural(22, "zmiana", "zmiany", "zmian"), "22 zmiany");
        assert_eq!(
            run_time("2026-11-14T14:02:59"),
            Some(at("2026-11-14T14:02"))
        );
        assert_eq!(run_time("zle"), None);
    }
}
