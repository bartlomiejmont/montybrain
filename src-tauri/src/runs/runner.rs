//! The run store and the background process lifecycle.
//!
//! A background run is the resolved CLI spawned directly (no shell) in its own
//! process group, stdin `/dev/null`, cwd = the runs dir, with a constructed
//! PATH and the API-key variables removed, attached to `montybrain-mcp` only
//! (see [`command`]). Before the CLI starts, the run's own thread collects the
//! git snapshot a `postep` run needs ([`super::git`]). stdout is parsed line by
//! line (each line bounded) into events; the tail of stderr is kept for the
//! error message. At most one background run at a time; a hard timeout,
//! `cancel` and app quit stop the whole process group (SIGTERM, then SIGKILL
//! after a grace period).

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use montybrain_core::{Project, Snapshot};

use super::agent::{self, AgentChoice};
use super::command::{McpConfigFile, McpServer};
use super::parse::{Outcome, Parser, clip};
use super::providers::{self, Resolver};
use super::recipes::{self, Plan, RecipeId};
use super::{
    EventKind, ProviderId, RunEvent, RunInfo, RunMode, RunRequest, RunStatus, command, git,
    history, now_string, proposal, terminal,
};

pub const MAX_RUNS: usize = 20;
pub const MAX_EVENTS: usize = 2_000;
pub const MAX_EVENT_BYTES: usize = 200 * 1024;
/// One event's text is cut at this many bytes.
pub const MAX_EVENT_TEXT: usize = 16 * 1024;
pub const MAX_RESULT_BYTES: usize = 200 * 1024;
const STDERR_TAIL_BYTES: usize = 4 * 1024;
const STDERR_TAIL_LINES: usize = 15;
pub const RUN_TIMEOUT: Duration = Duration::from_secs(15 * 60);
pub const KILL_GRACE: Duration = Duration::from_secs(5);
/// How long the run waits for stdout to drain after the process exited.
const DRAIN_WAIT: Duration = Duration::from_secs(2);
/// A stdout line longer than this is skipped (not buffered) with a note.
pub const MAX_LINE_BYTES: usize = 256 * 1024;
/// Notes about skipped long lines per run; further ones are skipped silently.
const MAX_LONG_LINE_NOTES: usize = 5;

/// One `\n`-terminated piece of a stream, read with a bounded buffer.
#[derive(Debug, PartialEq, Eq)]
pub enum Chunk {
    /// `buf` holds the line (at most `max` bytes, delimiter included).
    Line,
    /// The line was longer than `max`: `buf` is empty and the whole line
    /// (this many bytes) was consumed and dropped.
    Oversized(usize),
    Eof,
}

/// Like `read_until(b'\n')`, but never holds more than `max` bytes: the rest
/// of an oversized line is read and discarded up to its delimiter.
pub fn read_line_capped<R: BufRead>(
    r: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
) -> std::io::Result<Chunk> {
    buf.clear();
    let mut total = 0usize;
    let mut over = false;
    loop {
        let avail = match r.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if avail.is_empty() {
            return Ok(match (total, over) {
                (0, _) => Chunk::Eof,
                (n, true) => Chunk::Oversized(n),
                (_, false) => Chunk::Line,
            });
        }
        let (n, done) = match avail.iter().position(|&b| b == b'\n') {
            Some(i) => (i + 1, true),
            None => (avail.len(), false),
        };
        if !over {
            if buf.len() + n > max {
                over = true;
                *buf = Vec::new();
            } else {
                buf.extend_from_slice(&avail[..n]);
            }
        }
        total += n;
        r.consume(n);
        if done {
            return Ok(if over {
                Chunk::Oversized(total)
            } else {
                Chunk::Line
            });
        }
    }
}

/// Receives run events (the app forwards them to the window).
pub trait RunSink: Send + Sync {
    fn event(&self, e: &RunEvent);
    /// `info.events` is always empty here.
    fn updated(&self, info: &RunInfo);
    /// A background run ended (once, after `updated`, outside the store lock,
    /// on the run's own thread). `info` carries its result.
    fn finished(&self, _info: &RunInfo, _ctx: &RunContext) {}
}

/// Who started a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Trigger {
    #[default]
    Manual,
    /// The automatic project update ([`crate::updates`]).
    Auto,
}

/// What the app hands a run besides the request; never comes from the webview.
#[derive(Debug, Clone, Default)]
pub struct StartExtra {
    /// The vault as the run starts: the base an `aktualizacja` proposal is
    /// validated against.
    pub base: Option<Arc<Snapshot>>,
    /// A git snapshot the caller already collected (the scheduler does, to
    /// decide whether anything changed); used instead of collecting again.
    pub repo_state: Option<String>,
    pub trigger: Trigger,
    /// Model / effort of the provider (Settings → "Agenci"); default = none.
    pub agent: AgentChoice,
    /// Conversation history as a second source (`aktualizacja`, background).
    pub history: Option<HistoryOptions>,
}

/// The history source of one run.
#[derive(Debug, Clone, Default)]
pub struct HistoryOptions {
    /// The window in days.
    pub days: u16,
    /// A digest the caller already collected (the scheduler, for its
    /// fingerprint); used instead of collecting again.
    pub prepared: Option<history::Digest>,
    /// The request of a `zlecenie` run: the digest is ranked by it.
    pub focus: Option<String>,
}

/// About a finished background run, for [`RunSink::finished`].
#[derive(Debug, Clone, Default)]
pub struct RunContext {
    pub project: Option<String>,
    pub base: Option<Arc<Snapshot>>,
    /// [`proposal::fingerprint`] of the run's input (`aktualizacja` only).
    pub fingerprint: Option<String>,
    pub trigger: Trigger,
    /// What the history source contributed ("Zebrano 12 sesji …"), when on.
    pub history_note: Option<String>,
    /// The text of a `zlecenie` run.
    pub request: Option<String>,
}

/// The request text of a `zlecenie` plan.
fn request_text(plan: &Plan) -> Option<String> {
    (plan.recipe.id == RecipeId::Zlecenie)
        .then(|| plan.params.text.clone())
        .flatten()
}

pub struct RunnerConfig {
    pub home: PathBuf,
    /// cwd of background runs.
    pub runs_dir: PathBuf,
    /// Launcher scripts of terminal runs.
    pub terminal_dir: PathBuf,
    pub timeout: Duration,
    pub grace: Duration,
    /// Program + args that open a launcher script (the script path is appended).
    pub open_cmd: Vec<OsString>,
    /// Where `claude` / `codex` are looked up before PATH.
    pub search_dirs: Vec<PathBuf>,
    /// PATH of the app, searched after `search_dirs` (and `mcp_dirs`).
    pub path_var: Option<OsString>,
    /// Where `montybrain-mcp` is looked up before PATH.
    pub mcp_dirs: Vec<PathBuf>,
    /// Fixed locations git is taken from for the repository snapshot.
    pub git_candidates: Vec<PathBuf>,
    pub git_timeout: Duration,
    pub snapshot_budget: Duration,
    pub max_events: usize,
    pub max_event_bytes: usize,
    /// The conversation-history stores ([`history::Roots`]).
    pub history_roots: history::Roots,
    pub history_budget: Duration,
}

impl RunnerConfig {
    pub fn new(home: PathBuf, runs_dir: PathBuf, terminal_dir: PathBuf) -> Self {
        RunnerConfig {
            search_dirs: providers::candidate_dirs(&home),
            mcp_dirs: providers::mcp_candidate_dirs(&home),
            runs_dir,
            terminal_dir,
            timeout: RUN_TIMEOUT,
            grace: KILL_GRACE,
            open_cmd: vec!["/usr/bin/open".into(), "-a".into(), "Terminal".into()],
            path_var: std::env::var_os("PATH"),
            git_candidates: git::git_candidates(),
            git_timeout: git::GIT_TIMEOUT,
            snapshot_budget: git::SNAPSHOT_BUDGET,
            max_events: MAX_EVENTS,
            max_event_bytes: MAX_EVENT_BYTES,
            history_roots: history::Roots {
                claude: home.join(".claude/projects"),
                codex: home.join(".codex/sessions"),
            },
            history_budget: history::HISTORY_BUDGET,
            home,
        }
    }
}

// --- process groups -------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub enum Sig {
    Term,
    Kill,
}

/// Sends `sig` to the process group `pgid`; false when it no longer exists.
pub fn signal_group(pgid: u32, sig: Sig) -> bool {
    use rustix::process::{Pid, Signal, kill_process_group};
    let Some(pid) = i32::try_from(pgid).ok().and_then(Pid::from_raw) else {
        return false;
    };
    let sig = match sig {
        Sig::Term => Signal::TERM,
        Sig::Kill => Signal::KILL,
    };
    kill_process_group(pid, sig).is_ok()
}

fn group_alive(pgid: u32) -> bool {
    use rustix::process::{Pid, test_kill_process_group};
    i32::try_from(pgid)
        .ok()
        .and_then(Pid::from_raw)
        .is_some_and(|pid| test_kill_process_group(pid).is_ok())
}

/// SIGTERM to the group, SIGKILL to whatever is left after `grace`.
pub fn terminate_group(pgid: u32, grace: Duration) {
    if !signal_group(pgid, Sig::Term) {
        return;
    }
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if !group_alive(pgid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    if signal_group(pgid, Sig::Kill) {
        crate::log(&format!(
            "runs: process group {pgid} killed after {grace:?}"
        ));
    }
}

// --- store ------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    Cancel,
    Timeout,
    Shutdown,
}

struct Active {
    run_id: String,
    /// Set once the process is spawned.
    pgid: Option<u32>,
    stop: Option<Stop>,
}

struct Record {
    /// Always with empty `events`; they live in `events` below.
    info: RunInfo,
    /// Background runs only.
    ctx: Option<RunContext>,
    events: VecDeque<RunEvent>,
    bytes: usize,
    dropped: u64,
    next_seq: u64,
}

impl Record {
    fn with_events(&self) -> RunInfo {
        let mut info = self.info.clone();
        if self.dropped > 0 {
            info.events.push(RunEvent {
                run_id: info.id.clone(),
                seq: 0,
                at: self
                    .events
                    .front()
                    .map_or_else(|| info.started_at.clone(), |e| e.at.clone()),
                kind: EventKind::Note,
                text: format!(
                    "… pominięto {} wcześniejszych wpisów (limit dziennika akcji).",
                    self.dropped
                ),
            });
        }
        info.events.extend(self.events.iter().cloned());
        info
    }
}

#[derive(Default)]
struct Store {
    /// Oldest first.
    runs: VecDeque<Record>,
    active: Option<Active>,
    counter: u64,
}

impl Store {
    fn get_mut(&mut self, id: &str) -> Option<&mut Record> {
        self.runs.iter_mut().find(|r| r.info.id == id)
    }

    fn get(&self, id: &str) -> Option<&Record> {
        self.runs.iter().find(|r| r.info.id == id)
    }
}

/// Cuts `s` to at most `max` bytes on a char boundary, marking the cut.
fn clip_bytes(s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    let mut end = max.saturating_sub('…'.len_utf8());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

fn human_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s} s")
    } else {
        format!("{} min {} s", s / 60, s % 60)
    }
}

/// The last lines of stderr, trimmed.
fn tail_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(STDERR_TAIL_LINES);
    lines[start..].join("\n")
}

/// What the skip-if-unchanged fingerprint covers besides the vault: the git
/// snapshot and, when the history source is on, its digest.
pub fn fingerprint_input(repo_state: &str, digest: Option<&history::Digest>) -> String {
    match digest {
        Some(d) => format!("{repo_state}\u{0}historia\u{0}{}", d.text),
        None => repo_state.to_string(),
    }
}

/// Recipes whose background run gets the app's git snapshot in the prompt.
fn needs_repo_state(id: RecipeId) -> bool {
    matches!(
        id,
        RecipeId::Postep | RecipeId::Aktualizacja | RecipeId::Zlecenie
    )
}

/// The history window of a request: wider than a check, ranked by the request.
pub const REQUEST_HISTORY_DAYS: u16 = 30;

/// Said in the log, the strip and the receipt of a request run when the
/// project's history source is off.
pub const REQUEST_HISTORY_OFF_NOTE: &str =
    "Historia rozmów jest wyłączona dla tego projektu (Źródła).";

/// What the run thread of a background run needs.
struct Job {
    id: String,
    plan: Plan,
    /// Collected by the caller already ([`StartExtra::repo_state`]).
    repo_state: Option<String>,
    agent: AgentChoice,
    history: Option<HistoryOptions>,
    /// The project's repositories, where its sessions may have run.
    places: Vec<history::Place>,
    binary: PathBuf,
    mcp: PathBuf,
    vault_root: PathBuf,
    repos: Vec<git::RepoRef>,
    cwd: PathBuf,
}

pub struct RunManager {
    cfg: RunnerConfig,
    sink: Arc<dyn RunSink>,
    store: Mutex<Store>,
    resolver: Resolver,
    parent_env: Vec<(OsString, OsString)>,
}

impl RunManager {
    /// `parent_env` is the environment children start from (scrubbed per run).
    pub fn new(
        cfg: RunnerConfig,
        sink: Arc<dyn RunSink>,
        parent_env: Vec<(OsString, OsString)>,
    ) -> Self {
        let version_env =
            command::child_env(parent_env.clone(), providers::child_path(&cfg.home), false);
        let resolver = Resolver::new(cfg.search_dirs.clone(), cfg.path_var.clone(), version_env);
        RunManager {
            cfg,
            sink,
            store: Mutex::new(Store::default()),
            resolver,
            parent_env,
        }
    }

    pub fn providers(&self) -> Vec<super::ProviderInfo> {
        self.resolver.list()
    }

    /// Newest first, with events.
    pub fn list(&self) -> Vec<RunInfo> {
        let st = crate::lock(&self.store);
        st.runs.iter().rev().map(Record::with_events).collect()
    }

    pub fn get(&self, id: &str) -> Option<RunInfo> {
        crate::lock(&self.store).get(id).map(Record::with_events)
    }

    /// Whether a background run is running (only one at a time).
    pub fn is_busy(&self) -> bool {
        crate::lock(&self.store).active.is_some()
    }

    // --- store operations (events are emitted under the lock, in seq order) ---

    fn new_record(
        &self,
        st: &mut Store,
        plan: &Plan,
        ctx: Option<RunContext>,
        agent: &AgentChoice,
    ) -> RunInfo {
        let agent = agent.validated(plan.provider).unwrap_or_default();
        st.counter += 1;
        let started_at = now_string();
        let id = format!(
            "{}-{}",
            started_at.replace([':', '-'], "").replace('T', "-"),
            st.counter
        );
        let info = RunInfo {
            id,
            recipe: plan.recipe.key.into(),
            recipe_title: plan.recipe.title.into(),
            provider: plan.provider,
            mode: plan.mode,
            project: plan.params.project.clone(),
            request: request_text(plan),
            model: agent.model,
            effort: agent.effort,
            status: RunStatus::Running,
            started_at,
            finished_at: None,
            result: None,
            error: None,
            events: Vec::new(),
        };
        st.runs.push_back(Record {
            info: info.clone(),
            ctx,
            events: VecDeque::new(),
            bytes: 0,
            dropped: 0,
            next_seq: 0,
        });
        while st.runs.len() > MAX_RUNS {
            match st
                .runs
                .iter()
                .position(|r| r.info.status != RunStatus::Running)
            {
                Some(i) => {
                    st.runs.remove(i);
                }
                None => break,
            }
        }
        info
    }

    /// Appends a log line to a run and emits it.
    fn push(&self, id: &str, kind: EventKind, text: String) {
        let mut st = crate::lock(&self.store);
        let Some(rec) = st.get_mut(id) else {
            return;
        };
        rec.next_seq += 1;
        let ev = RunEvent {
            run_id: id.to_string(),
            seq: rec.next_seq,
            at: now_string(),
            kind,
            text: clip_bytes(text, MAX_EVENT_TEXT),
        };
        rec.bytes += ev.text.len();
        rec.events.push_back(ev.clone());
        loop {
            let marker = usize::from(rec.dropped > 0);
            let over = rec.events.len() + marker > self.cfg.max_events
                || rec.bytes > self.cfg.max_event_bytes;
            if !over || rec.events.len() <= 1 {
                break;
            }
            if let Some(old) = rec.events.pop_front() {
                rec.bytes -= old.text.len();
                rec.dropped += 1;
            }
        }
        self.sink.event(&ev);
    }

    /// Sets the final state of a run (once) and emits `run-updated`.
    fn finish(
        &self,
        id: &str,
        status: RunStatus,
        result: Option<String>,
        error: Option<String>,
    ) -> bool {
        let (info, ctx) = {
            let mut st = crate::lock(&self.store);
            if st.active.as_ref().is_some_and(|a| a.run_id == id) {
                st.active = None;
            }
            let Some(rec) = st.get_mut(id) else {
                return false;
            };
            if rec.info.status != RunStatus::Running {
                return false;
            }
            rec.info.status = status;
            rec.info.finished_at = Some(now_string());
            rec.info.result = result.map(|r| clip_bytes(r, MAX_RESULT_BYTES));
            rec.info.error = error;
            let info = rec.info.clone();
            self.sink.updated(&info);
            (info, rec.ctx.clone())
        };
        if let Some(ctx) = ctx {
            self.sink.finished(&info, &ctx);
        }
        true
    }

    /// Sets the input fingerprint of a running background run.
    fn set_fingerprint(&self, id: &str, fp: String) {
        if let Some(ctx) = crate::lock(&self.store)
            .get_mut(id)
            .and_then(|r| r.ctx.as_mut())
        {
            ctx.fingerprint = Some(fp);
        }
    }

    fn context(&self, id: &str) -> Option<RunContext> {
        crate::lock(&self.store).get(id).and_then(|r| r.ctx.clone())
    }

    fn last_event(&self, id: &str) -> Option<RunEvent> {
        crate::lock(&self.store)
            .get(id)
            .and_then(|r| r.events.back().cloned())
    }

    // --- starting --------------------------------------------------------------

    /// Validates and starts a run. Errors before a run exists (validation,
    /// missing CLI or `montybrain-mcp`, another background run) are `Err`; a
    /// run that could not be spawned or opened ends as `failed`. `vault_root`
    /// is the vault the app uses (handed to `montybrain-mcp`).
    pub fn start(
        self: &Arc<Self>,
        req: &RunRequest,
        projects: &[Project],
        vault_root: &Path,
    ) -> Result<RunInfo, String> {
        self.start_with(req, projects, vault_root, StartExtra::default())
    }

    /// [`Self::start`] with what the app knows besides the request.
    pub fn start_with(
        self: &Arc<Self>,
        req: &RunRequest,
        projects: &[Project],
        vault_root: &Path,
        extra: StartExtra,
    ) -> Result<RunInfo, String> {
        let plan = recipes::validate(req, projects)?;
        let binary = self.resolver.resolve(plan.provider)?;
        match plan.mode {
            RunMode::Background => self.start_background(plan, binary, projects, vault_root, extra),
            RunMode::Terminal => {
                let prompt = recipes::prompt(&plan, None);
                self.start_terminal(&plan, &binary, &prompt, projects, &extra.agent)
            }
        }
    }

    /// `montybrain-mcp`, looked up like the providers.
    fn resolve_mcp(&self) -> Result<PathBuf, String> {
        providers::find_in(
            command::MCP_BINARY,
            &self.cfg.mcp_dirs,
            self.cfg.path_var.as_deref(),
        )
        .ok_or_else(|| {
            format!(
                "Nie znaleziono serwera MCP `{}` (szukano w ~/.local/bin, obok aplikacji i w PATH). \
Akcje w tle łączą się z MontyBrain tylko przez niego: uruchom `./install.sh` w katalogu \
repozytorium MontyBrain i spróbuj ponownie.",
                command::MCP_BINARY
            )
        })
    }

    fn start_background(
        self: &Arc<Self>,
        plan: Plan,
        binary: PathBuf,
        projects: &[Project],
        vault_root: &Path,
        extra: StartExtra,
    ) -> Result<RunInfo, String> {
        let mcp = self.resolve_mcp()?;
        let vault_root = std::path::absolute(vault_root)
            .map_err(|e| format!("Nieprawidłowy katalog MontyBrain: {e}"))?;
        if vault_root.to_str().is_none() {
            return Err(format!(
                "Ścieżka MontyBrain nie jest w UTF-8: {}",
                vault_root.display()
            ));
        }
        let cwd = self.cfg.runs_dir.clone();
        std::fs::create_dir_all(&cwd)
            .and_then(|()| std::fs::set_permissions(&cwd, std::fs::Permissions::from_mode(0o700)))
            .map_err(|e| format!("Nie udało się przygotować katalogu akcji: {e}"))?;
        let repos = if needs_repo_state(plan.recipe.id) && extra.repo_state.is_none() {
            command::repo_refs(projects, plan.params.project.as_deref(), &self.cfg.home)
        } else {
            Vec::new()
        };
        let ctx = RunContext {
            project: plan.params.project.clone(),
            base: extra.base,
            fingerprint: None,
            trigger: extra.trigger,
            history_note: None,
            request: request_text(&plan),
        };
        let history = match plan.recipe.id {
            RecipeId::Aktualizacja => extra.history,
            // A request looks further back, ranked by what it asks for.
            RecipeId::Zlecenie => extra.history.map(|_| HistoryOptions {
                days: REQUEST_HISTORY_DAYS,
                prepared: None,
                focus: plan.params.text.clone(),
            }),
            _ => None,
        };
        let places = match (&history, &plan.params.project) {
            (Some(_), Some(slug)) => command::repo_refs(projects, Some(slug), &self.cfg.home)
                .into_iter()
                .map(|r| history::Place {
                    repo: r
                        .label
                        .rsplit_once(" / ")
                        .map_or(r.label.as_str(), |(_, n)| n)
                        .to_string(),
                    path: r.path,
                })
                .collect(),
            _ => Vec::new(),
        };

        let info = {
            let mut st = crate::lock(&self.store);
            if let Some(a) = &st.active {
                let title = st
                    .get(&a.run_id)
                    .map_or("inna akcja", |r| r.info.recipe_title.as_str());
                return Err(format!(
                    "W tle trwa już akcja „{title}”. Poczekaj na jej koniec albo ją anuluj."
                ));
            }
            let info = self.new_record(&mut st, &plan, Some(ctx), &extra.agent);
            st.active = Some(Active {
                run_id: info.id.clone(),
                pgid: None,
                stop: None,
            });
            self.sink.updated(&info);
            info
        };
        let id = info.id.clone();
        let job = Job {
            id: id.clone(),
            plan,
            repo_state: extra.repo_state,
            agent: extra.agent,
            history,
            places,
            binary,
            mcp,
            vault_root,
            repos,
            cwd,
        };
        let mgr = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name(format!("run-{id}"))
            .spawn(move || mgr.run_background(job));
        if let Err(e) = spawned {
            self.fail(&id, format!("Błąd wewnętrzny: {e}"));
        }
        Ok(self.get(&id).unwrap_or(info))
    }

    /// The stop requested for the active run `id`, if any.
    fn stop_requested(&self, id: &str) -> Option<Stop> {
        crate::lock(&self.store)
            .active
            .as_ref()
            .filter(|a| a.run_id == id)
            .and_then(|a| a.stop)
    }

    fn fail(&self, id: &str, msg: String) {
        self.push(id, EventKind::Error, msg.clone());
        self.finish(id, RunStatus::Failed, None, Some(msg));
    }

    /// The run's own thread: repository snapshot, prompt, MCP config, spawn,
    /// supervision.
    fn run_background(self: Arc<Self>, job: Job) {
        let Job {
            id,
            plan,
            repo_state,
            agent,
            history,
            places,
            binary,
            mcp,
            vault_root,
            repos,
            cwd,
        } = job;
        let repo_state = if needs_repo_state(plan.recipe.id) {
            Some(repo_state.unwrap_or_else(|| self.snapshot(&id, &repos)))
        } else {
            None
        };
        let digest = match (&history, &repo_state) {
            (Some(h), Some(state)) => Some(self.history(&id, h, &places, state)),
            _ => None,
        };
        if plan.recipe.id == RecipeId::Zlecenie && history.is_none() {
            self.push(&id, EventKind::Note, REQUEST_HISTORY_OFF_NOTE.into());
            if let Some(ctx) = crate::lock(&self.store)
                .get_mut(&id)
                .and_then(|r| r.ctx.as_mut())
            {
                ctx.history_note = Some(REQUEST_HISTORY_OFF_NOTE.into());
            }
        }
        if plan.recipe.id == RecipeId::Aktualizacja
            && let (Some(state), Some(ctx)) = (&repo_state, self.context(&id))
        {
            let project = ctx.project.as_deref().unwrap_or_default();
            let input = fingerprint_input(state, digest.as_ref());
            let fp = proposal::fingerprint(&input, ctx.base.as_deref(), project);
            self.set_fingerprint(&id, fp);
        }
        if self.stop_requested(&id).is_some() {
            self.push(&id, EventKind::Note, "Anulowano.".into());
            self.finish(&id, RunStatus::Cancelled, None, None);
            return;
        }
        let prompt = recipes::prompt_with(
            &plan,
            repo_state.as_deref(),
            digest.as_ref().map(|d| d.text.as_str()),
        );
        let server = McpServer {
            binary: &mcp,
            vault_root: &vault_root,
        };
        let config = match plan.provider {
            ProviderId::Claude => {
                let written = command::claude_mcp_config(&plan, &server).and_then(|c| {
                    McpConfigFile::write(&cwd, &id, &c)
                        .map_err(|e| format!("Nie udało się zapisać konfiguracji MCP: {e}"))
                });
                match written {
                    Ok(f) => Some(f),
                    Err(e) => return self.fail(&id, e),
                }
            }
            ProviderId::Codex => None,
        };
        let args = match command::background_args_with(
            &plan,
            &prompt,
            &cwd,
            &server,
            config.as_ref().map(McpConfigFile::path),
            &agent,
        ) {
            Ok(a) => a,
            Err(e) => return self.fail(&id, e),
        };
        let env = command::child_env(
            self.parent_env.clone(),
            providers::child_path(&self.cfg.home),
            plan.read_only(),
        );
        self.push(
            &id,
            EventKind::Note,
            format!(
                "Uruchamiam {} w tle: {}{}",
                plan.provider.name(),
                binary.display(),
                if plan.read_only() {
                    " (MontyBrain tylko do odczytu)"
                } else {
                    ""
                }
            ),
        );

        let spawned = Command::new(&binary)
            .args(&args)
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k, v)))
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn();
        let mut child = match spawned {
            Ok(c) => c,
            Err(e) => {
                drop(config);
                return self.fail(
                    &id,
                    format!("Nie udało się uruchomić {}: {e}", binary.display()),
                );
            }
        };
        let pgid = child.id();
        let early_stop = {
            let mut st = crate::lock(&self.store);
            match st.active.as_mut() {
                Some(a) if a.run_id == id => {
                    a.pgid = Some(pgid);
                    a.stop.is_some()
                }
                _ => true,
            }
        };
        if early_stop {
            let grace = self.cfg.grace;
            std::thread::spawn(move || terminate_group(pgid, grace));
        }

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let provider = plan.provider;
        Arc::clone(&self).supervise(id, provider, child, pgid, stdout, stderr, config);
    }

    /// The git snapshot of `project`'s repositories, outside any run (the
    /// automatic update compares it before deciding to call the agent).
    pub fn repo_state(&self, projects: &[Project], project: &str) -> String {
        let repos = command::repo_refs(projects, Some(project), &self.cfg.home);
        let git_bin = git::find_git(&self.cfg.git_candidates);
        let never = || false;
        git::Collector {
            git: git_bin.as_deref(),
            home: &self.cfg.home,
            timeout: self.cfg.git_timeout,
            budget: self.cfg.snapshot_budget,
            limits: git::Limits::default(),
            cancelled: &never,
        }
        .collect(&repos)
        .text
    }

    /// What the drift detector keeps of `project`'s repositories: the git
    /// snapshot, parsed, plus the commits on each default branch since
    /// `since` (the last check, `YYYY-MM-DDTHH:MM`).
    pub fn drift_obs(
        &self,
        projects: &[Project],
        project: &str,
        since: Option<&str>,
    ) -> Vec<crate::drift::RepoObs> {
        let repos = command::repo_refs(projects, Some(project), &self.cfg.home);
        let git_bin = git::find_git(&self.cfg.git_candidates);
        let never = || false;
        let collector = git::Collector {
            git: git_bin.as_deref(),
            home: &self.cfg.home,
            timeout: self.cfg.git_timeout,
            budget: self.cfg.snapshot_budget,
            limits: git::Limits::default(),
            cancelled: &never,
        };
        let text = collector.collect(&repos).text;
        let mut obs = crate::drift::parse_snapshot(&text);
        for o in &mut obs {
            if let (Some(since), Some(branch), true) = (since, o.default_branch.as_deref(), o.ok) {
                o.new_commits =
                    collector.commits_since(std::path::Path::new(&o.path), branch, since);
            }
        }
        obs
    }

    /// The history digest of `project` for the scheduler (its fingerprint):
    /// the project's repositories plus the worktrees listed in `state`.
    pub fn history_digest(
        &self,
        projects: &[Project],
        project: &str,
        state: &str,
        days: u16,
    ) -> history::Digest {
        let mut places: Vec<history::Place> =
            command::repo_refs(projects, Some(project), &self.cfg.home)
                .into_iter()
                .map(|r| history::Place {
                    repo: r
                        .label
                        .rsplit_once(" / ")
                        .map_or(r.label.as_str(), |(_, n)| n)
                        .to_string(),
                    path: r.path,
                })
                .collect();
        for (repo, path) in git::worktrees(state) {
            if !places.iter().any(|p| p.path == path) {
                places.push(history::Place { path, repo });
            }
        }
        history::collect(&history::Request {
            roots: &self.cfg.history_roots,
            home: &self.cfg.home,
            places: &places,
            now: std::time::SystemTime::now(),
            days,
            budget: self.cfg.history_budget,
            limits: history::Limits::default(),
            focus: None,
        })
    }

    /// The history digest of a run (prepared by the caller or collected now,
    /// from the project's repositories and the worktrees in `state`), logged.
    fn history(
        &self,
        id: &str,
        h: &HistoryOptions,
        places: &[history::Place],
        state: &str,
    ) -> history::Digest {
        let digest = h.prepared.clone().unwrap_or_else(|| {
            self.push(
                id,
                EventKind::Note,
                "Zbieram historię rozmów (lokalnie, z redakcją)…".into(),
            );
            let mut all = places.to_vec();
            for (repo, path) in git::worktrees(state) {
                if !all.iter().any(|p| p.path == path) {
                    all.push(history::Place { path, repo });
                }
            }
            history::collect(&history::Request {
                roots: &self.cfg.history_roots,
                home: &self.cfg.home,
                places: &all,
                now: std::time::SystemTime::now(),
                days: h.days,
                budget: self.cfg.history_budget,
                limits: history::Limits::default(),
                focus: h.focus.as_deref(),
            })
        });
        for n in &digest.notes {
            self.push(id, EventKind::Note, n.clone());
        }
        let summary = digest.summary(h.days);
        self.push(id, EventKind::Note, format!("Historia rozmów: {summary}"));
        if let Some(ctx) = crate::lock(&self.store)
            .get_mut(id)
            .and_then(|r| r.ctx.as_mut())
        {
            ctx.history_note = Some(summary);
        }
        digest
    }

    /// Collects the git snapshot of `repos`, logging what happened.
    fn snapshot(&self, id: &str, repos: &[git::RepoRef]) -> String {
        if !repos.is_empty() {
            self.push(
                id,
                EventKind::Note,
                format!("Zbieram stan repozytoriów ({}; tylko odczyt)…", repos.len()),
            );
        }
        let git_bin = git::find_git(&self.cfg.git_candidates);
        let cancelled = || self.stop_requested(id).is_some();
        let snap = git::Collector {
            git: git_bin.as_deref(),
            home: &self.cfg.home,
            timeout: self.cfg.git_timeout,
            budget: self.cfg.snapshot_budget,
            limits: git::Limits::default(),
            cancelled: &cancelled,
        }
        .collect(repos);
        for n in &snap.notes {
            self.push(id, EventKind::Note, format!("Stan repozytoriów: {n}"));
        }
        if !repos.is_empty() && !snap.cancelled {
            self.push(
                id,
                EventKind::Note,
                format!(
                    "Zebrano stan {} z {} repozytoriów ({} KB).",
                    snap.collected,
                    repos.len(),
                    snap.text.len().div_ceil(1024)
                ),
            );
        }
        snap.text
    }

    #[allow(clippy::too_many_arguments)]
    fn supervise(
        self: Arc<Self>,
        id: String,
        provider: ProviderId,
        mut child: std::process::Child,
        pgid: u32,
        stdout: Option<std::process::ChildStdout>,
        stderr: Option<std::process::ChildStderr>,
        config: Option<McpConfigFile>,
    ) {
        let started = Instant::now();
        let parser = Arc::new(Mutex::new(Parser::new(provider)));
        let tail = Arc::new(Mutex::new(Vec::<u8>::new()));

        let (out_done_tx, out_done_rx) = mpsc::channel::<()>();
        if let Some(out) = stdout {
            let mgr = Arc::clone(&self);
            let parser = Arc::clone(&parser);
            let id = id.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(out);
                let mut buf = Vec::new();
                let mut long_lines = 0usize;
                loop {
                    match read_line_capped(&mut reader, &mut buf, MAX_LINE_BYTES) {
                        Ok(Chunk::Eof) | Err(_) => break,
                        Ok(Chunk::Oversized(n)) => {
                            long_lines += 1;
                            if long_lines <= MAX_LONG_LINE_NOTES {
                                mgr.push(
                                    &id,
                                    EventKind::Note,
                                    format!(
                                        "Pominięto zbyt długą linię wyjścia ({} KB, limit {} KB).",
                                        n / 1024,
                                        MAX_LINE_BYTES / 1024
                                    ),
                                );
                            }
                        }
                        Ok(Chunk::Line) => {
                            let line = String::from_utf8_lossy(&buf);
                            let lines = crate::lock(&parser).line(&line);
                            for (kind, text) in lines {
                                mgr.push(&id, kind, text);
                            }
                        }
                    }
                }
                let _ = out_done_tx.send(());
            });
        } else {
            drop(out_done_tx);
        }
        if let Some(mut err) = stderr {
            let tail = Arc::clone(&tail);
            std::thread::spawn(move || {
                let mut chunk = [0u8; 4096];
                while let Ok(n) = err.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    let mut t = crate::lock(&tail);
                    t.extend_from_slice(&chunk[..n]);
                    let excess = t.len().saturating_sub(STDERR_TAIL_BYTES);
                    t.drain(..excess);
                }
            });
        }
        // Hard timeout: wakes up early (and does nothing) when the run ends.
        let (timer_tx, timer_rx) = mpsc::channel::<()>();
        {
            let mgr = Arc::clone(&self);
            let id = id.clone();
            let timeout = self.cfg.timeout;
            std::thread::spawn(move || {
                if let Err(mpsc::RecvTimeoutError::Timeout) = timer_rx.recv_timeout(timeout) {
                    mgr.stop(&id, Stop::Timeout, false);
                }
            });
        }

        let status = child.wait();
        drop(timer_tx);
        // Leftovers in the group (e.g. a tool process that outlived the CLI).
        if group_alive(pgid) {
            terminate_group(pgid, self.cfg.grace);
        }
        let _ = out_done_rx.recv_timeout(DRAIN_WAIT);
        // The MCP config is not needed any more; removed before the run is
        // reported as ended.
        drop(config);
        let outcome = crate::lock(&parser).outcome();
        let stderr_tail = tail_text(&crate::lock(&tail));
        let stop = crate::lock(&self.store)
            .active
            .as_ref()
            .filter(|a| a.run_id == id)
            .and_then(|a| a.stop);
        self.conclude(
            &id,
            provider,
            status,
            outcome,
            stderr_tail,
            stop,
            started.elapsed(),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn conclude(
        &self,
        id: &str,
        provider: ProviderId,
        status: std::io::Result<ExitStatus>,
        outcome: Outcome,
        stderr_tail: String,
        stop: Option<Stop>,
        took: Duration,
    ) {
        let name = provider.name();
        match stop {
            Some(Stop::Cancel | Stop::Shutdown) => {
                self.push(id, EventKind::Note, "Anulowano.".into());
                self.finish(id, RunStatus::Cancelled, None, None);
                return;
            }
            Some(Stop::Timeout) => {
                let msg = format!(
                    "Przekroczono limit czasu ({}); proces przerwano.",
                    human_duration(self.cfg.timeout)
                );
                self.push(id, EventKind::Error, msg.clone());
                self.finish(id, RunStatus::Failed, outcome.final_text, Some(msg));
                return;
            }
            None => {}
        }
        let exit_ok = status.as_ref().is_ok_and(ExitStatus::success);
        if exit_ok && !outcome.failed && outcome.final_text.is_some() {
            self.push(
                id,
                EventKind::Note,
                format!("Zakończono ({}).", human_duration(took)),
            );
            self.finish(id, RunStatus::Succeeded, outcome.final_text, None);
            return;
        }
        let exit = match &status {
            Ok(s) if s.success() => None,
            Ok(s) => Some(match (s.code(), s.signal()) {
                (Some(c), _) => format!("{name} zakończył się kodem {c}."),
                (None, Some(sig)) => format!("{name} został przerwany sygnałem {sig}."),
                _ => format!("{name} zakończył się błędem."),
            }),
            Err(e) => Some(format!("Nie udało się poczekać na {name}: {e}")),
        };
        let mut msg = match (&outcome.cli_error, &exit) {
            (Some(cli), Some(exit)) if outcome.failed => format!("{cli}\n({exit})"),
            (Some(cli), None) if outcome.failed => cli.clone(),
            (cli, Some(exit)) => match cli {
                Some(cli) => format!("{exit}\n{cli}"),
                None => exit.clone(),
            },
            (cli, None) => {
                let mut m = format!("{name} zakończył pracę bez odpowiedzi końcowej.");
                if let Some(cli) = cli {
                    m.push('\n');
                    m.push_str(cli);
                }
                m
            }
        };
        if !stderr_tail.is_empty() && !(outcome.failed && outcome.cli_error.is_some()) {
            msg.push_str("\n\nKońcówka stderr:\n");
            msg.push_str(&stderr_tail);
        }
        // A model the CLI did not accept: say so plainly first.
        let model = crate::lock(&self.store)
            .get(id)
            .and_then(|r| r.info.model.clone());
        let msg = match model {
            Some(m) if agent::model_rejected(&m, &msg) => {
                format!("{}\n\n{msg}", agent::model_rejected_message(provider, &m))
            }
            _ => msg,
        };
        let msg = clip(&msg, 4000);
        let repeated = self
            .last_event(id)
            .is_some_and(|e| e.kind == EventKind::Error && msg.starts_with(&e.text));
        if !repeated {
            self.push(id, EventKind::Error, msg.clone());
        }
        self.finish(id, RunStatus::Failed, outcome.final_text, Some(msg));
    }

    // --- stopping ---------------------------------------------------------------

    /// Asks the active run `id` to stop. With `wait`, terminates inline;
    /// otherwise on a helper thread. Returns whether `id` was the active run.
    fn stop(&self, id: &str, why: Stop, wait: bool) -> bool {
        let pgid = {
            let mut st = crate::lock(&self.store);
            let Some(a) = st.active.as_mut().filter(|a| a.run_id == id) else {
                return false;
            };
            if a.stop.is_none() {
                a.stop = Some(why);
            }
            a.pgid
        };
        if let Some(pgid) = pgid {
            let grace = self.cfg.grace;
            if wait {
                terminate_group(pgid, grace);
            } else {
                std::thread::spawn(move || terminate_group(pgid, grace));
            }
        }
        true
    }

    /// Cancels a running background run and waits (bounded) for it to end.
    pub fn cancel(&self, id: &str) -> Result<RunInfo, String> {
        let status = crate::lock(&self.store)
            .get(id)
            .map(|r| r.info.status)
            .ok_or_else(|| format!("Nie ma akcji {id}."))?;
        if status == RunStatus::Running && self.stop(id, Stop::Cancel, false) {
            let deadline = Instant::now() + self.cfg.grace + DRAIN_WAIT + Duration::from_secs(1);
            while Instant::now() < deadline {
                if self.get(id).is_some_and(|r| r.status != RunStatus::Running) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        self.get(id).ok_or_else(|| format!("Nie ma akcji {id}."))
    }

    /// App quit: stops the active run's process group (SIGTERM, then SIGKILL
    /// after `grace`), synchronously.
    pub fn shutdown(&self, grace: Duration) {
        let active = crate::lock(&self.store).active.as_mut().map(|a| {
            a.stop.get_or_insert(Stop::Shutdown);
            a.pgid
        });
        if let Some(Some(pgid)) = active {
            crate::log("runs: stopping the background run on quit");
            terminate_group(pgid, grace);
        }
    }

    // --- terminal ----------------------------------------------------------------

    fn start_terminal(
        &self,
        plan: &Plan,
        binary: &Path,
        prompt: &str,
        projects: &[Project],
        agent: &AgentChoice,
    ) -> Result<RunInfo, String> {
        let cwd = match (&plan.recipe.id, &plan.params.project) {
            (RecipeId::Postep | RecipeId::Aktualizacja | RecipeId::Zlecenie, Some(slug)) => {
                command::repo_dirs(projects, Some(slug), &self.cfg.home)
                    .into_iter()
                    .next()
            }
            _ => None,
        }
        .unwrap_or_else(|| self.cfg.home.clone());
        let path = providers::child_path(&self.cfg.home);
        let path = path.to_str().ok_or("Ścieżka nie jest w UTF-8")?;
        let info = {
            let mut st = crate::lock(&self.store);
            self.new_record(&mut st, plan, None, agent)
        };
        let id = info.id.clone();
        match terminal::launch(
            &self.cfg.terminal_dir,
            &id,
            binary,
            &agent::args(plan.provider, agent),
            prompt,
            &cwd,
            path,
            &self.cfg.open_cmd,
        ) {
            Ok(script) => {
                self.finish(&id, RunStatus::Succeeded, None, None);
                self.push(
                    &id,
                    EventKind::Note,
                    format!(
                        "Otwarto sesję {} w Terminalu (katalog: {}). Dalej pracujesz w oknie Terminala.",
                        plan.provider.name(),
                        cwd.display()
                    ),
                );
                crate::log(&format!("runs: terminal launcher {}", script.display()));
            }
            Err(e) => {
                self.finish(&id, RunStatus::Failed, None, Some(e.clone()));
                self.push(&id, EventKind::Error, e);
            }
        }
        Ok(self.get(&id).unwrap_or(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::recipes::tests::req;
    use crate::runs::{ProviderId, RunMode};
    use montybrain_core::{ProjectState, Repo};

    #[derive(Default)]
    struct Collect {
        events: Mutex<Vec<RunEvent>>,
        updates: Mutex<Vec<RunInfo>>,
        finished: Mutex<Vec<(RunInfo, RunContext)>>,
    }

    impl RunSink for Collect {
        fn event(&self, e: &RunEvent) {
            self.events.lock().unwrap().push(e.clone());
        }
        fn updated(&self, info: &RunInfo) {
            self.updates.lock().unwrap().push(info.clone());
        }
        fn finished(&self, info: &RunInfo, ctx: &RunContext) {
            self.finished
                .lock()
                .unwrap()
                .push((info.clone(), ctx.clone()));
        }
    }

    struct Env {
        tmp: tempfile::TempDir,
        mgr: Arc<RunManager>,
        sink: Arc<Collect>,
        projects: Vec<Project>,
    }

    impl Env {
        fn out(&self) -> PathBuf {
            self.tmp.path().join("out")
        }
        fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.out().join(name)).unwrap_or_default()
        }
        fn wait(&self, id: &str) -> RunInfo {
            let deadline = Instant::now() + Duration::from_secs(40);
            loop {
                let info = self.mgr.get(id).unwrap();
                if info.status != RunStatus::Running {
                    return info;
                }
                assert!(Instant::now() < deadline, "run {id} did not finish");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        fn wait_file(&self, name: &str) -> String {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let s = self.read(name);
                if !s.trim().is_empty() {
                    return s;
                }
                assert!(
                    Instant::now() < deadline,
                    "{name} never written; runs: {:#?}",
                    self.mgr.list()
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        fn start(
            &self,
            recipe: &str,
            provider: &str,
            mode: &str,
            params: &[(&str, &str)],
        ) -> Result<RunInfo, String> {
            self.mgr.start(
                &req(recipe, provider, mode, params),
                &self.projects,
                &self.vault(),
            )
        }
        fn vault(&self) -> PathBuf {
            self.tmp.path().join("vault")
        }
        fn mcp(&self) -> PathBuf {
            self.tmp.path().join("bin/montybrain-mcp")
        }
        /// The MCP config the fake Claude saw, and whether it is gone now.
        fn mcp_config_seen(&self) -> (serde_json::Value, String, bool) {
            let path = self.read("mcp.path");
            let json = serde_json::from_str(&self.read("mcp.json")).unwrap();
            (
                json,
                self.read("mcp.mode").trim().to_string(),
                !Path::new(path.trim()).exists(),
            )
        }
    }

    /// A temp setup whose `claude` / `codex` are the given shell bodies.
    fn setup(claude: &str, codex: &str, tune: impl FnOnce(&mut RunnerConfig)) -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        let out = tmp.path().join("out");
        let repo = tmp.path().join("repo");
        // The repository fixture has a `.git` directory: the snapshot only asks
        // git about real repository roots.
        for d in [&bin, &out, &repo, &repo.join(".git")] {
            std::fs::create_dir_all(d).unwrap();
        }
        // Every fake records argv (NUL-separated), cwd, env and stdin first,
        // and copies the `--mcp-config` file it was given (with its mode).
        let prologue = "for a in \"$@\"; do printf '%s\\0' \"$a\"; done > \"$FAKE_OUT/argv\"\n\
pwd > \"$FAKE_OUT/pwd\"\nenv > \"$FAKE_OUT/env\"\ncat > \"$FAKE_OUT/stdin\"\n\
prev=; for a in \"$@\"; do if [ \"$prev\" = --mcp-config ]; then cp \"$a\" \"$FAKE_OUT/mcp.json\"; \
stat -f %Lp \"$a\" > \"$FAKE_OUT/mcp.mode\"; echo \"$a\" > \"$FAKE_OUT/mcp.path\"; fi; prev=$a; done\n";
        for (name, body) in [("claude", claude), ("codex", codex)] {
            if body.is_empty() {
                continue;
            }
            let p = bin.join(name);
            std::fs::write(&p, format!("#!/bin/bash\n{prologue}{body}\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // Never executed by the fakes; only resolved.
        let mcp = bin.join("montybrain-mcp");
        std::fs::write(&mcp, "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(&mcp, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut cfg = RunnerConfig::new(
            tmp.path().join("home"),
            tmp.path().join("cache/runs"),
            tmp.path().join("cache/terminal"),
        );
        cfg.search_dirs = vec![bin.clone()];
        cfg.mcp_dirs = vec![bin];
        cfg.path_var = None;
        cfg.grace = Duration::from_millis(400);
        cfg.open_cmd = vec!["/usr/bin/true".into()];
        tune(&mut cfg);
        let sink = Arc::new(Collect::default());
        let parent_env = vec![
            ("FAKE_OUT".into(), out.into_os_string()),
            ("ANTHROPIC_API_KEY".into(), "sk-ant-secret".into()),
            ("OPENAI_API_KEY".into(), "sk-secret".into()),
            ("CLAUDECODE".into(), "1".into()),
            ("PATH".into(), "/evil".into()),
        ];
        let mgr = Arc::new(RunManager::new(cfg, sink.clone(), parent_env));
        let projects = vec![Project {
            slug: "beeking".into(),
            name: "BeeKing".into(),
            color: None,
            state: ProjectState::Active,
            repos: vec![Repo {
                name: "app".into(),
                path: repo.to_str().unwrap().into(),
            }],
            description: String::new(),
            path: "beeking/projekt.md".into(),
        }];
        Env {
            tmp,
            mgr,
            sink,
            projects,
        }
    }

    const CLAUDE_OK: &str = r#"cat <<'EOF'
{"type":"system","subtype":"init","model":"m","mcp_servers":[{"name":"montybrain","status":"connected"}]}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__montybrain__overview","input":{}}]}}
nie-json
{"type":"assistant","message":{"content":[{"type":"text","text":"Raport: wszystko gra."}]}}
EOF
sleep 0.2
echo '{"type":"result","subtype":"success","is_error":false,"result":"Raport: wszystko gra."}'
echo 'jakieś ostrzeżenie' >&2"#;

    const LONG: &str = r#"( trap '' TERM; while :; do sleep 0.05; done ) &
echo $! > "$FAKE_OUT/child.pid"
trap '' TERM
echo '{"type":"system","subtype":"init","mcp_servers":[]}'
while :; do sleep 0.05; done"#;

    fn argv(env: &Env) -> Vec<String> {
        env.read("argv")
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()
    }

    fn alive(pid: i32) -> bool {
        rustix::process::Pid::from_raw(pid)
            .is_some_and(|p| rustix::process::test_kill_process(p).is_ok())
    }

    fn wait_dead(pid: i32) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive(pid) {
            assert!(Instant::now() < deadline, "pid {pid} still alive");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn claude_background_success() {
        let env = setup(CLAUDE_OK, "", |_| {});
        let repo = env.tmp.path().join("repo");
        git::tests::make_repo(&repo, &["Dodaj logowanie ```x```"]);
        let started = env
            .start("postep", "claude", "background", &[("project", "beeking")])
            .unwrap();
        assert_eq!(started.status, RunStatus::Running);
        assert_eq!(started.mode, RunMode::Background);
        assert_eq!(started.recipe_title, "Otwarte zadania i postęp");
        let info = env.wait(&started.id);
        assert_eq!(info.status, RunStatus::Succeeded, "{info:?}");
        assert_eq!(info.result.as_deref(), Some("Raport: wszystko gra."));
        assert_eq!(info.error, None);
        assert!(info.finished_at.is_some());

        // Log: snapshot notes, start note, init notes, tool, garbage note,
        // text, end note; no trailing duplicate.
        let kinds: Vec<_> = info
            .events
            .iter()
            .map(|e| (e.kind, e.text.as_str()))
            .collect();
        assert_eq!(
            kinds[..3],
            [
                (
                    EventKind::Note,
                    "Zbieram stan repozytoriów (1; tylko odczyt)…"
                ),
                (EventKind::Note, "Zebrano stan 1 z 1 repozytoriów (1 KB)."),
                (
                    EventKind::Note,
                    format!(
                        "Uruchamiam Claude Code w tle: {} (MontyBrain tylko do odczytu)",
                        env.tmp.path().join("bin/claude").display()
                    )
                    .as_str()
                )
            ],
            "{kinds:?}"
        );
        assert!(kinds.contains(&(EventKind::Tool, "overview")));
        assert_eq!(
            kinds.iter().filter(|(k, _)| *k == EventKind::Text).count(),
            1,
            "{kinds:?}"
        );
        assert!(kinds.last().unwrap().1.starts_with("Zakończono"));
        let seqs: Vec<u64> = info.events.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, (1..=seqs.len() as u64).collect::<Vec<_>>());
        assert!(
            info.events
                .iter()
                .all(|e| e.run_id == info.id && e.at.len() == 19)
        );

        // Emitted: the same events, and two updates without events.
        assert_eq!(*env.sink.events.lock().unwrap(), info.events);
        let updates = env.sink.updates.lock().unwrap().clone();
        assert_eq!(
            updates.iter().map(|u| u.status).collect::<Vec<_>>(),
            [RunStatus::Running, RunStatus::Succeeded]
        );
        assert!(updates.iter().all(|u| u.events.is_empty()));
        let json = serde_json::to_value(&updates[1]).unwrap();
        assert_eq!(json["status"], "succeeded");
        assert_eq!(json["provider"], "claude");
        assert_eq!(json["mode"], "background");
        assert_eq!(json["events"], serde_json::json!([]));

        // argv exactly as built (the prompt carries the repository snapshot);
        // cwd = runs dir; stdin was /dev/null; env scrubbed.
        let runs_dir = env.tmp.path().join("cache/runs");
        let plan = recipes::validate(
            &req("postep", "claude", "background", &[("project", "beeking")]),
            &env.projects,
        )
        .unwrap();
        let a = argv(&env);
        let prompt = &a[1];
        let state = format!("=== beeking / app ===\nŚcieżka: {}\n", repo.display());
        assert!(prompt.contains(recipes::REPO_STATE_INTRO), "{prompt}");
        assert!(prompt.contains(&format!("````text\n{state}")), "{prompt}");
        assert!(
            prompt.contains("$ git status --short --branch\n## main\n"),
            "{prompt}"
        );
        assert!(prompt.contains("Dodaj logowanie ```x```"), "{prompt}");
        let config = runs_dir.join(format!("{}.mcp.json", info.id));
        let expected = command::background_args(
            &plan,
            prompt,
            &runs_dir,
            &McpServer {
                binary: &env.mcp(),
                vault_root: &env.vault(),
            },
            Some(&config),
        )
        .unwrap();
        assert_eq!(a, expected);
        assert!(!a.iter().any(|x| x == "--add-dir"));
        // The MCP config: montybrain-mcp only, read-only, 0600, removed after the run.
        let (json, mode, removed) = env.mcp_config_seen();
        assert_eq!(
            json,
            serde_json::json!({"mcpServers":{"montybrain":{
                "command": env.mcp().to_str().unwrap(), "args": [],
                "env": {"MONTYBRAIN_DIR": env.vault().to_str().unwrap(), "MONTYBRAIN_READONLY": "1"}}}})
        );
        assert_eq!(mode, "600");
        assert!(removed && !config.exists());
        assert_eq!(
            Path::new(env.read("pwd").trim()),
            runs_dir.canonicalize().unwrap()
        );
        assert_eq!(env.read("stdin"), "");
        let child_env = env.read("env");
        assert!(!child_env.contains("sk-ant-secret") && !child_env.contains("sk-secret"));
        assert!(!child_env.lines().any(|l| l.starts_with("CLAUDECODE=")));
        assert!(child_env.contains("MONTYBRAIN_READONLY=1"));
        let path_line = child_env.lines().find(|l| l.starts_with("PATH=")).unwrap();
        assert!(
            path_line.ends_with(":/usr/bin:/bin:/usr/sbin:/sbin") && !path_line.contains("/evil")
        );
        assert_eq!(
            std::fs::metadata(&runs_dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn codex_background_success_and_writing_recipe_env() {
        let codex = r#"cat <<'EOF'
{"type":"thread.started","thread_id":"t1"}
{"type":"item.completed","item":{"id":"i0","type":"error","message":"deprecated config key"}}
{"type":"item.started","item":{"id":"i1","type":"mcp_tool_call","server":"montybrain","tool":"create_task","arguments":{"title":"A"},"status":"in_progress"}}
{"type":"item.completed","item":{"id":"i2","type":"agent_message","text":"Dodano 1 zadanie."}}
{"type":"turn.completed","usage":{}}
EOF
echo 'mcp gmail failed to start' >&2"#;
        let env = setup("", codex, |_| {});
        let id = env
            .start("zadania", "codex", "background", &[("text", "Kupić mleko")])
            .unwrap()
            .id;
        let info = env.wait(&id);
        assert_eq!(info.status, RunStatus::Succeeded, "{info:?}");
        assert_eq!(info.result.as_deref(), Some("Dodano 1 zadanie."));
        assert!(
            info.events
                .iter()
                .any(|e| e.kind == EventKind::Note && e.text.contains("deprecated"))
        );
        assert!(
            info.events
                .iter()
                .any(|e| e.kind == EventKind::Tool && e.text == r#"create_task {"title":"A"}"#)
        );
        let a = argv(&env);
        let runs_dir = env.tmp.path().join("cache/runs");
        assert_eq!(
            a[..a.len() - 1],
            [
                "exec".to_string(),
                "--json".into(),
                "--skip-git-repo-check".into(),
                "--approve-for-me".into(),
                "-C".into(),
                runs_dir.to_str().unwrap().into(),
                "--ignore-user-config".into(),
                "-c".into(),
                format!("mcp_servers.montybrain.command=\"{}\"", env.mcp().display()),
                "-c".into(),
                format!(
                    "mcp_servers.montybrain.env.MONTYBRAIN_DIR=\"{}\"",
                    env.vault().display()
                ),
            ]
        );
        assert!(
            !a.iter().any(|x| x.contains("MONTYBRAIN_READONLY")),
            "writes allowed: {a:?}"
        );
        let prompt = a.last().unwrap();
        assert!(prompt.starts_with("Użyj skilla montybrain-zadania."));
        assert!(prompt.contains(recipes::skill_instructions("montybrain-zadania").unwrap()));
        assert!(!env.read("env").contains("MONTYBRAIN_READONLY"));
        // Codex gets no config file.
        assert!(!env.out().join("mcp.json").exists());
    }

    #[test]
    fn failures_carry_exit_code_stderr_tail_or_cli_message() {
        let failing = r#"for i in $(seq 1 40); do echo "stderr linia $i" >&2; done
echo '{"type":"system","subtype":"init","mcp_servers":[]}'
exit 3"#;
        let env = setup(
            failing,
            r#"echo '{"type":"turn.failed","error":{"message":"You have hit your usage limit."}}'; exit 1"#,
            |_| {},
        );
        let info = env.wait(&env.start("raport", "claude", "background", &[]).unwrap().id);
        assert_eq!(info.status, RunStatus::Failed);
        let e = info.error.unwrap();
        assert!(e.contains("Claude Code zakończył się kodem 3."), "{e}");
        assert!(
            e.contains("stderr linia 40") && !e.contains("stderr linia 20\n"),
            "{e}"
        );
        assert_eq!(info.events.last().unwrap().kind, EventKind::Error);
        let (json, _, removed) = env.mcp_config_seen();
        assert_eq!(
            json["mcpServers"]["montybrain"]["env"]["MONTYBRAIN_READONLY"],
            "1"
        );
        assert!(removed, "MCP config left behind after a failed run");

        let info = env.wait(&env.start("raport", "codex", "background", &[]).unwrap().id);
        assert_eq!(info.status, RunStatus::Failed);
        assert_eq!(
            info.error.as_deref(),
            Some("You have hit your usage limit.\n(Codex CLI zakończył się kodem 1.)")
        );
        // The CLI's message was logged once by the parser, not repeated.
        let errors: Vec<_> = info
            .events
            .iter()
            .filter(|e| e.kind == EventKind::Error)
            .collect();
        assert_eq!(errors.len(), 1, "{errors:?}");
    }

    #[test]
    fn exit_zero_without_answer_fails_and_cli_error_is_used() {
        let env = setup(
            r#"echo '{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}'"#,
            r#"echo '{"type":"turn.completed"}'"#,
            |_| {},
        );
        let info = env.wait(&env.start("raport", "claude", "background", &[]).unwrap().id);
        assert_eq!(info.status, RunStatus::Failed);
        assert_eq!(
            info.error.as_deref(),
            Some("Not logged in · Please run /login")
        );
        let info = env.wait(&env.start("raport", "codex", "background", &[]).unwrap().id);
        assert_eq!(info.status, RunStatus::Failed);
        assert!(info.error.unwrap().contains("bez odpowiedzi końcowej"));
    }

    #[test]
    fn cancel_kills_the_whole_group_and_one_run_at_a_time() {
        let env = setup(LONG, LONG, |_| {});
        let first = env.start("raport", "claude", "background", &[]).unwrap();
        let child: i32 = env.wait_file("child.pid").trim().parse().unwrap();
        assert!(alive(child));

        // A second background run is rejected; a terminal run is fine.
        let e = env.start("postep", "codex", "background", &[]).unwrap_err();
        assert!(
            e.contains("W tle trwa już akcja „Raport z dnia lub tygodnia”"),
            "{e}"
        );
        let t = env.start("raport", "codex", "terminal", &[]).unwrap();
        assert_eq!(t.status, RunStatus::Succeeded);

        let t0 = Instant::now();
        let info = env.mgr.cancel(&first.id).unwrap();
        assert_eq!(info.status, RunStatus::Cancelled, "{info:?}");
        assert!(t0.elapsed() < Duration::from_secs(4));
        assert_eq!(info.error, None);
        assert_eq!(info.events.last().unwrap().text, "Anulowano.");
        // The TERM-ignoring grandchild was SIGKILLed with its group.
        wait_dead(child);
        let (_, _, removed) = env.mcp_config_seen();
        assert!(removed, "MCP config left behind after cancel");

        // Cancelling again just returns the finished run; unknown ids are errors.
        assert_eq!(
            env.mgr.cancel(&first.id).unwrap().status,
            RunStatus::Cancelled
        );
        assert!(env.mgr.cancel("nie-ma").is_err());
        // The slot is free again.
        let second = env.start("raport", "codex", "background", &[]).unwrap();
        env.mgr.cancel(&second.id).unwrap();
        assert_eq!(env.wait(&second.id).status, RunStatus::Cancelled);
    }

    #[test]
    fn timeout_fails_the_run_and_kills_the_group() {
        let env = setup(LONG, "", |c| c.timeout = Duration::from_secs(3));
        let id = env.start("raport", "claude", "background", &[]).unwrap().id;
        let info = env.wait(&id);
        assert_eq!(info.status, RunStatus::Failed);
        assert!(info.error.unwrap().contains("Przekroczono limit czasu"));
        // The first exec of a freshly written script can be slow on macOS (it
        // gets scanned), so under load the timeout may fire before the fake
        // got as far as starting its child; when it did, the child is dead.
        if let Ok(pid) = env.read("child.pid").trim().parse::<i32>() {
            wait_dead(pid);
        }
    }

    #[test]
    fn shutdown_kills_the_running_child() {
        let env = setup(LONG, "", |_| {});
        let id = env.start("raport", "claude", "background", &[]).unwrap().id;
        let child: i32 = env.wait_file("child.pid").trim().parse().unwrap();
        env.mgr.shutdown(Duration::from_millis(200));
        wait_dead(child);
        assert_eq!(env.wait(&id).status, RunStatus::Cancelled);
    }

    #[test]
    fn event_caps_drop_oldest_with_a_marker() {
        let many = r#"for i in $(seq 1 300); do echo "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"linia $i\"}]}}"; done
echo '{"type":"result","subtype":"success","is_error":false,"result":"ok"}'"#;
        let env = setup(many, "", |c| c.max_events = 50);
        let info = env.wait(&env.start("raport", "claude", "background", &[]).unwrap().id);
        assert_eq!(info.status, RunStatus::Succeeded);
        assert_eq!(info.events.len(), 50);
        assert_eq!(info.events[0].seq, 0);
        assert!(
            info.events[0].text.contains("pominięto"),
            "{:?}",
            info.events[0]
        );
        assert!(info.events.iter().any(|e| e.text == "linia 300"));
        assert!(!info.events.iter().any(|e| e.text == "linia 1"));
        // Live events were all emitted.
        assert!(
            env.sink
                .events
                .lock()
                .unwrap()
                .iter()
                .any(|e| e.text == "linia 1")
        );

        let big = r#"for i in $(seq 1 20); do printf '{"type":"assistant","message":{"content":[{"type":"text","text":"%s"}]}}\n' "$(head -c 1000 /dev/zero | tr '\0' 'x')$i"; done
echo '{"type":"result","subtype":"success","is_error":false,"result":"ok"}'"#;
        let env = setup(big, "", |c| c.max_event_bytes = 4_000);
        let info = env.wait(&env.start("raport", "claude", "background", &[]).unwrap().id);
        let bytes: usize = info.events.iter().skip(1).map(|e| e.text.len()).sum();
        assert!(bytes <= 4_000, "{bytes}");
        assert!(info.events[0].text.contains("pominięto"));
    }

    #[test]
    fn oversized_stdout_lines_are_skipped_not_buffered() {
        // 3 MB without a newline inside a JSON line, twice, then a normal answer.
        let huge = r#"for n in 1 2; do printf '{"type":"assistant","message":{"content":[{"type":"text","text":"'; head -c 3000000 /dev/zero | tr '\0' x; printf '"}]}}\n'; done
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"po"}]}}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"ok"}'"#;
        let env = setup(huge, "", |_| {});
        let info = env.wait(&env.start("raport", "claude", "background", &[]).unwrap().id);
        assert_eq!(info.status, RunStatus::Succeeded, "{info:?}");
        assert_eq!(info.result.as_deref(), Some("ok"));
        let notes: Vec<_> = info
            .events
            .iter()
            .filter(|e| e.text.contains("zbyt długą linię"))
            .collect();
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes[0].text.contains("2929 KB"), "{notes:?}");
        assert!(info.events.iter().any(|e| e.text == "po"));
        assert!(!info.events.iter().any(|e| e.text.contains("xxxxxxxx")));
    }

    #[test]
    fn terminal_runs_are_recorded_and_capped() {
        let env = setup("true", "true", |_| {});
        let info = env
            .start("postep", "claude", "terminal", &[("project", "beeking")])
            .unwrap();
        assert_eq!(info.mode, RunMode::Terminal);
        assert_eq!(info.status, RunStatus::Succeeded);
        assert!(info.finished_at.is_some() && info.result.is_none());
        let script = env
            .tmp
            .path()
            .join(format!("cache/terminal/{}.command", info.id));
        let body = std::fs::read_to_string(&script).unwrap();
        let repo = env.tmp.path().join("repo");
        assert!(body.contains(&format!("cd '{}'", repo.display())), "{body}");
        assert!(body.contains("/bin/claude' \"$prompt\""), "{body}");
        let prompt = std::fs::read_to_string(
            env.tmp
                .path()
                .join(format!("cache/terminal/{}.prompt.txt", info.id)),
        )
        .unwrap();
        assert_eq!(
            prompt,
            "/montybrain-postep Sprawdź postęp otwartych zadań w projekcie `beeking`."
        );
        let updates = env.sink.updates.lock().unwrap().clone();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].status, RunStatus::Succeeded);

        // Text never lands in the script.
        let z = env
            .start(
                "zadania",
                "codex",
                "terminal",
                &[("text", "$(rm -rf ~) 'x'")],
            )
            .unwrap();
        let body = std::fs::read_to_string(
            env.tmp
                .path()
                .join(format!("cache/terminal/{}.command", z.id)),
        )
        .unwrap();
        assert!(!body.contains("rm -rf"), "{body}");
        assert!(body.contains(&format!("cd '{}'", env.tmp.path().join("home").display())));

        for _ in 0..25 {
            env.start("poczta", "claude", "terminal", &[]).unwrap();
        }
        let list = env.mgr.list();
        assert_eq!(list.len(), MAX_RUNS);
        assert!(list.windows(2).all(|w| w[0].id != w[1].id));
        assert_eq!(list[0].recipe, "poczta");
        assert!(!list.iter().any(|r| r.id == info.id), "oldest dropped");
    }

    #[test]
    fn terminal_open_failure_is_a_failed_run() {
        let env = setup("true", "", |c| c.open_cmd = vec!["/usr/bin/false".into()]);
        let info = env.start("raport", "claude", "terminal", &[]).unwrap();
        assert_eq!(info.status, RunStatus::Failed);
        assert!(info.error.unwrap().contains("Terminala"));
    }

    #[test]
    fn missing_mcp_server_is_a_clear_error_for_background_runs_only() {
        let env = setup("true", "true", |_| {});
        std::fs::remove_file(env.mcp()).unwrap();
        for provider in ["claude", "codex"] {
            let e = env
                .start("raport", provider, "background", &[])
                .unwrap_err();
            assert!(
                e.contains("Nie znaleziono serwera MCP `montybrain-mcp`")
                    && e.contains("install.sh"),
                "{e}"
            );
        }
        assert!(env.mgr.list().is_empty());
        // Terminal sessions use the agent's own configuration.
        let t = env.start("raport", "claude", "terminal", &[]).unwrap();
        assert_eq!(t.status, RunStatus::Succeeded);
    }

    /// Cancelling while the snapshot is being collected (a git that hangs)
    /// ends the run at once, and the CLI is never started.
    #[test]
    fn cancel_during_the_snapshot() {
        let env = setup(CLAUDE_OK, "", |c| c.git_timeout = Duration::from_secs(20));
        let fake_git = env.tmp.path().join("git");
        std::fs::write(&fake_git, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&fake_git, std::fs::Permissions::from_mode(0o755)).unwrap();
        // RunnerConfig is consumed by the manager; build one with the fake git.
        let env = Env {
            mgr: {
                let mut cfg = RunnerConfig::new(
                    env.tmp.path().join("home"),
                    env.tmp.path().join("cache/runs"),
                    env.tmp.path().join("cache/terminal"),
                );
                cfg.search_dirs = vec![env.tmp.path().join("bin")];
                cfg.mcp_dirs = vec![env.tmp.path().join("bin")];
                cfg.path_var = None;
                cfg.grace = Duration::from_millis(400);
                cfg.git_candidates = vec![fake_git];
                let parent_env = vec![("FAKE_OUT".into(), env.out().into_os_string())];
                Arc::new(RunManager::new(cfg, env.sink.clone(), parent_env))
            },
            ..env
        };
        let id = env
            .start("postep", "claude", "background", &[("project", "beeking")])
            .unwrap()
            .id;
        std::thread::sleep(Duration::from_millis(300));
        let t = Instant::now();
        let info = env.mgr.cancel(&id).unwrap();
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        assert_eq!(info.status, RunStatus::Cancelled, "{info:?}");
        assert!(
            info.events
                .iter()
                .any(|e| e.text.starts_with("Zbieram stan repozytoriów"))
        );
        assert!(!info.events.iter().any(|e| e.text.starts_with("Uruchamiam")));
        std::thread::sleep(Duration::from_millis(200));
        assert!(!env.out().join("argv").exists(), "the CLI was started");
    }

    #[test]
    fn errors_before_a_run_exists() {
        let env = setup("", "", |_| {});
        let e = env
            .start("raport", "claude", "background", &[])
            .unwrap_err();
        assert!(e.contains("Nie znaleziono programu `claude`"), "{e}");
        assert!(
            env.start("raport", "claude", "background", &[("x", "y")])
                .unwrap_err()
                .contains("Nieznany parametr")
        );
        assert!(env.mgr.list().is_empty());
        let p = env.mgr.providers();
        assert_eq!(
            p.iter().map(|p| p.id).collect::<Vec<_>>(),
            [ProviderId::Claude, ProviderId::Codex]
        );
        assert!(p.iter().all(|p| !p.available));
    }

    /// `aktualizacja` in the background: read-only like `postep`, with the
    /// app's git snapshot; the sink's `finished` gets the run's project, base
    /// snapshot and input fingerprint (what `updates` stores the proposal by).
    #[test]
    fn update_run_is_read_only_and_reports_its_context() {
        let block = format!(
            "Raport.\\n\\n```{}\\n{{\\\"changes\\\": []}}\\n```",
            proposal::FENCE_INFO
        );
        let claude = format!(
            r#"echo '{{"type":"system","subtype":"init","mcp_servers":[]}}'
echo '{{"type":"result","subtype":"success","is_error":false,"result":"{block}"}}'"#
        );
        let env = setup(&claude, "", |_| {});
        let repo = env.tmp.path().join("repo");
        git::tests::make_repo(&repo, &["Pierwszy"]);
        let base = Arc::new(Snapshot {
            projects: env.projects.clone(),
            ..Default::default()
        });
        assert!(!env.mgr.is_busy());
        let started = env
            .mgr
            .start_with(
                &req(
                    "aktualizacja",
                    "claude",
                    "background",
                    &[("project", "beeking")],
                ),
                &env.projects,
                &env.vault(),
                StartExtra {
                    base: Some(base.clone()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(started.project.as_deref(), Some("beeking"));
        let info = env.wait(&started.id);
        assert_eq!(info.status, RunStatus::Succeeded, "{info:?}");
        assert!(
            info.result
                .as_deref()
                .unwrap()
                .contains(proposal::FENCE_INFO)
        );
        assert!(!env.mgr.is_busy());
        // Read-only, MontyBrain tools only, the snapshot in the prompt.
        let a = argv(&env);
        assert!(a[1].contains(recipes::UPDATE_NO_WRITE) && a[1].contains("Pierwszy"));
        let allowed = &a[a.iter().position(|x| x == "--allowedTools").unwrap() + 1..];
        assert_eq!(allowed, command::MONTYBRAIN_READ_TOOLS);
        let (json, _, _) = env.mcp_config_seen();
        assert_eq!(
            json["mcpServers"]["montybrain"]["env"]["MONTYBRAIN_READONLY"],
            "1"
        );
        assert!(env.read("env").contains("MONTYBRAIN_READONLY=1"));

        let fin = env.sink.finished.lock().unwrap().clone();
        assert_eq!(fin.len(), 1);
        let (fin_info, ctx) = &fin[0];
        assert_eq!(fin_info.id, info.id);
        assert_eq!(ctx.project.as_deref(), Some("beeking"));
        assert_eq!(ctx.trigger, Trigger::Manual);
        assert!(Arc::ptr_eq(ctx.base.as_ref().unwrap(), &base));
        let fp = ctx.fingerprint.clone().unwrap();
        assert!(fp.starts_with("fnv1a64:"), "{fp}");

        // A prepared snapshot (the scheduler's) is used as is: no collection,
        // same fingerprint for the same input.
        let state = env.mgr.repo_state(&env.projects, "beeking");
        assert!(state.contains("Pierwszy"));
        assert_eq!(fp, proposal::fingerprint(&state, Some(&base), "beeking"));
        let second = env
            .mgr
            .start_with(
                &req(
                    "aktualizacja",
                    "claude",
                    "background",
                    &[("project", "beeking")],
                ),
                &env.projects,
                &env.vault(),
                StartExtra {
                    base: Some(base.clone()),
                    repo_state: Some(state.clone()),
                    trigger: Trigger::Auto,
                    ..Default::default()
                },
            )
            .unwrap();
        let info = env.wait(&second.id);
        assert!(
            !info
                .events
                .iter()
                .any(|e| e.text.starts_with("Zbieram stan")),
            "{:?}",
            info.events
        );
        let fin = env.sink.finished.lock().unwrap().clone();
        assert_eq!(fin[1].1.trigger, Trigger::Auto);
        assert_eq!(fin[1].1.fingerprint.as_deref(), Some(fp.as_str()));
        // Other recipes and terminal runs carry no fingerprint / no finished call.
        let t = env
            .start(
                "aktualizacja",
                "claude",
                "terminal",
                &[("project", "beeking")],
            )
            .unwrap();
        assert_eq!(t.status, RunStatus::Succeeded);
        assert_eq!(env.sink.finished.lock().unwrap().len(), 2);
        let prompt = std::fs::read_to_string(
            env.tmp
                .path()
                .join(format!("cache/terminal/{}.prompt.txt", t.id)),
        )
        .unwrap();
        assert!(prompt.ends_with(recipes::UPDATE_TERMINAL), "{prompt}");
    }

    /// Settings flow into the run: model / effort flags in argv (background
    /// and Terminal), the history digest in its own block, its note in the log
    /// and the context; a rejected model is said plainly.
    #[test]
    fn agent_choice_and_history_reach_the_run() {
        let ok = r#"echo '{"type":"system","subtype":"init","mcp_servers":[]}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"Raport."}'"#;
        let env = setup(ok, "", |_| {});
        let repo = env.tmp.path().join("repo");
        git::tests::make_repo(&repo, &["Pierwszy"]);
        // A synthetic Claude Code session in the repository.
        let home = env.tmp.path().join("home");
        let slug: String = repo
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let dir = home.join(".claude/projects").join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        let line = serde_json::json!({"parentUuid": null, "isSidechain": false, "promptId": "p", "type": "user",
            "message": {"role": "user", "content": "Zaplanuj tryb offline dla kasy"},
            "timestamp": "2026-11-12T10:00:00Z", "cwd": repo.to_str().unwrap(), "sessionId": "abcdef12-0000",
            "entrypoint": "cli"});
        std::fs::write(dir.join("s.jsonl"), format!("{line}\n")).unwrap();
        let agent = AgentChoice {
            model: Some("opus".into()),
            effort: Some("high".into()),
        };
        let started = env
            .mgr
            .start_with(
                &req(
                    "aktualizacja",
                    "claude",
                    "background",
                    &[("project", "beeking")],
                ),
                &env.projects,
                &env.vault(),
                StartExtra {
                    agent: agent.clone(),
                    history: Some(HistoryOptions {
                        days: 14,
                        prepared: None,
                        focus: None,
                    }),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            (started.model.as_deref(), started.effort.as_deref()),
            (Some("opus"), Some("high"))
        );
        let info = env.wait(&started.id);
        assert_eq!(info.status, RunStatus::Succeeded, "{info:?}");
        let a = argv(&env);
        assert_eq!(a[2..6], ["--model", "opus", "--effort", "high"]);
        let prompt = &a[1];
        assert!(prompt.contains(recipes::UPDATE_HISTORY_GUIDE), "{prompt}");
        assert!(
            prompt.contains("1. Zaplanuj tryb offline dla kasy"),
            "{prompt}"
        );
        assert!(prompt.find("Pierwszy").unwrap() < prompt.find("Zaplanuj tryb").unwrap());
        assert!(
            info.events.iter().any(|e| e
                .text
                .starts_with("Historia rozmów: Zebrano 1 sesję (Claude Code 1, Codex 0)")),
            "{:?}",
            info.events
        );
        let fin = env.sink.finished.lock().unwrap().clone();
        let ctx = &fin[0].1;
        assert!(
            ctx.history_note
                .as_deref()
                .unwrap()
                .starts_with("Zebrano 1 sesję")
        );
        // The fingerprint covers the digest: without history it differs.
        let state = env.mgr.repo_state(&env.projects, "beeking");
        assert_ne!(
            ctx.fingerprint.as_deref(),
            Some(proposal::fingerprint(&state, None, "beeking").as_str())
        );

        // Terminal: the same flags, quoted, before the prompt.
        let t = env
            .mgr
            .start_with(
                &req("raport", "claude", "terminal", &[]),
                &env.projects,
                &env.vault(),
                StartExtra {
                    agent,
                    ..Default::default()
                },
            )
            .unwrap();
        let body = std::fs::read_to_string(
            env.tmp
                .path()
                .join(format!("cache/terminal/{}.command", t.id)),
        )
        .unwrap();
        assert!(
            body.contains("/bin/claude' '--model' 'opus' '--effort' 'high' \"$prompt\""),
            "{body}"
        );
    }

    #[test]
    fn a_request_run_gets_its_text_and_a_focused_history() {
        let ok = r#"echo '{"type":"system","subtype":"init","mcp_servers":[]}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"Raport."}'"#;
        let env = setup(ok, "", |_| {});
        let repo = env.tmp.path().join("repo");
        git::tests::make_repo(&repo, &["Pierwszy"]);
        let home = env.tmp.path().join("home");
        let slug: String = repo
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let dir = home.join(".claude/projects").join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text, ts) in [
            ("a", "Zrób płatności kartą w kasie", "2026-10-20T10:00:00Z"),
            ("b", "Popraw README", "2026-11-12T10:00:00Z"),
        ] {
            let line = serde_json::json!({"parentUuid": null, "isSidechain": false, "promptId": "p", "type": "user",
                "message": {"role": "user", "content": text},
                "timestamp": ts, "cwd": repo.to_str().unwrap(), "sessionId": format!("{name}bcdef12-0000"),
                "entrypoint": "cli"});
            std::fs::write(dir.join(format!("{name}.jsonl")), format!("{line}\n")).unwrap();
        }
        let request = "Rozpisz temat płatności kartą";
        let run = |history: Option<HistoryOptions>| {
            let started = env
                .mgr
                .start_with(
                    &req(
                        "zlecenie",
                        "claude",
                        "background",
                        &[("project", "beeking"), ("text", request)],
                    ),
                    &env.projects,
                    &env.vault(),
                    StartExtra {
                        history,
                        ..Default::default()
                    },
                )
                .unwrap();
            assert_eq!(started.request.as_deref(), Some(request));
            let info = env.wait(&started.id);
            assert_eq!(info.status, RunStatus::Succeeded, "{info:?}");
            (info, argv(&env)[1].clone())
        };

        // History off: not read, and said so in the prompt, log and context.
        let (info, prompt) = run(None);
        assert!(
            prompt.contains(&format!("```text\n{request}\n```")),
            "{prompt}"
        );
        assert!(
            prompt.contains("Pierwszy"),
            "the repository snapshot is there"
        );
        assert!(prompt.ends_with(recipes::REQUEST_HISTORY_OFF), "{prompt}");
        assert!(!prompt.contains("płatności kartą w kasie"));
        assert!(
            info.events
                .iter()
                .any(|e| e.text == REQUEST_HISTORY_OFF_NOTE)
        );
        {
            let fin = env.sink.finished.lock().unwrap();
            let ctx = &fin.last().unwrap().1;
            assert_eq!(ctx.history_note.as_deref(), Some(REQUEST_HISTORY_OFF_NOTE));
            assert_eq!(ctx.request.as_deref(), Some(request));
            assert_eq!(ctx.fingerprint, None, "a request is not a check");
        }

        // History on (7 days in Settings): 30 days, the matching session first.
        let (info, prompt) = run(Some(HistoryOptions {
            days: 7,
            prepared: None,
            focus: None,
        }));
        assert!(prompt.contains(recipes::REQUEST_HISTORY_GUIDE), "{prompt}");
        assert!(
            prompt.contains("1. Zrób płatności kartą w kasie"),
            "{prompt}"
        );
        assert!(
            info.events
                .iter()
                .any(|e| e.text.starts_with("Historia rozmów: Zebrano 2 sesje")),
            "{:?}",
            info.events
        );
    }

    #[test]
    fn a_rejected_model_is_said_plainly() {
        let bad = r#"echo '{"type":"result","subtype":"success","is_error":true,"result":"There'"'"'s an issue with the selected model (zly-model). It may not exist or you may not have access to it."}'
exit 1"#;
        let env = setup(bad, "", |_| {});
        let id = env
            .mgr
            .start_with(
                &req("raport", "claude", "background", &[]),
                &env.projects,
                &env.vault(),
                StartExtra {
                    agent: AgentChoice {
                        model: Some("zly-model".into()),
                        effort: None,
                    },
                    ..Default::default()
                },
            )
            .unwrap()
            .id;
        let info = env.wait(&id);
        let e = info.error.unwrap();
        assert!(
            e.starts_with(
                "Claude Code nie przyjął modelu „zly-model”. Zmień go w Ustawieniach.\n\n"
            ),
            "{e}"
        );
        assert!(e.contains("issue with the selected model"), "{e}");
    }

    #[test]
    fn capped_line_reader() {
        let mut data = b"a\n".to_vec();
        data.extend(std::iter::repeat_n(b'x', 1000));
        data.extend_from_slice(b"\nbb\n");
        data.extend(std::iter::repeat_n(b'y', 50));
        // A small BufReader capacity, so long lines span many fill_buf calls.
        let mut r = BufReader::with_capacity(16, std::io::Cursor::new(data));
        let mut buf = Vec::new();
        let mut got = Vec::new();
        loop {
            let c = read_line_capped(&mut r, &mut buf, 20).unwrap();
            assert!(
                buf.len() <= 20 && buf.capacity() <= 64,
                "{}",
                buf.capacity()
            );
            let done = c == Chunk::Eof;
            got.push((c, String::from_utf8_lossy(&buf).into_owned()));
            if done {
                break;
            }
        }
        assert_eq!(
            got,
            [
                (Chunk::Line, "a\n".to_string()),
                (Chunk::Oversized(1001), String::new()),
                (Chunk::Line, "bb\n".to_string()),
                // The last line has no newline and is too long as well.
                (Chunk::Oversized(50), String::new()),
                (Chunk::Eof, String::new()),
            ]
        );
        // Exactly at the cap is fine.
        let mut r = std::io::Cursor::new(b"0123456789".to_vec());
        assert_eq!(read_line_capped(&mut r, &mut buf, 10).unwrap(), Chunk::Line);
        assert_eq!(buf, b"0123456789");
    }

    #[test]
    fn clipping_helpers() {
        assert_eq!(clip_bytes("abc".into(), 10), "abc");
        let s = clip_bytes("ąąąąą".into(), 6);
        assert!(s.len() <= 6 && s.ends_with('…'), "{s}");
        assert_eq!(human_duration(Duration::from_secs(5)), "5 s");
        assert_eq!(human_duration(Duration::from_secs(905)), "15 min 5 s");
        assert_eq!(tail_text(b"a\n\nb\n"), "a\nb");
    }
}
