//! In-app agent sessions ("Moje narzędzia" inside MontyBrain): Claude Code or
//! Codex run as a child process with the user's own configuration, driven
//! over a structured stdio protocol instead of a terminal. The app shows the
//! conversation, a compact line per tool call and the permission requests as
//! cards; the person answers, writes follow-ups, interrupts or ends it.
//!
//! * [`claude`] / [`codex`]: the two protocols (lines in, [`proto::Act`]s out);
//! * [`gate`]: what is allowed without asking, what is never allowed
//!   (MontyBrain writes, vault files), what becomes a card;
//! * this module: the one live session, its process, its view for the window.
//!
//! MontyBrain stays read-only for the agent: `montybrain-mcp` runs with
//! `MONTYBRAIN_READONLY=1`, its writing tools are denied by name, vault edits
//! are denied, and the gate refuses them again before any card is shown. The
//! agent's final `montybrain-zmiany` block becomes the usual proposal to tick
//! ([`SessionSink::answer`]). One session at a time; it lives in memory,
//! survives switching views and is killed when the app quits.

pub mod claude;
pub mod codex;
pub mod gate;
pub mod proto;

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ProviderId;
use super::history::redact;
use gate::{Gate, Verdict};
use proto::{Act, Decision, Question, Request};

/// Emitted with the whole [`SessionView`] whenever it changes.
pub const SESSION_UPDATED: &str = "agent-session";
/// Entries kept in the view (the oldest go first).
const MAX_ENTRIES: usize = 400;
/// A longer stdout line is skipped (never buffered whole).
const MAX_LINE: usize = 8 * 1024 * 1024;
/// Streamed text is sent to the window at most this often.
const STREAM_EMIT: Duration = Duration::from_millis(120);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    /// Skrzynka: proposals about inbox entries.
    Inbox,
    /// A project's "Zlecenie": proposals for that project.
    Project { project: String },
}

impl Scope {
    /// The key the proposal is stored under ([`super::proposal::INBOX`] or the slug).
    pub fn key(&self) -> String {
        match self {
            Scope::Inbox => super::proposal::INBOX.to_string(),
            Scope::Project { project } => project.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Starting,
    /// The agent works.
    Working,
    /// A card waits for the person.
    NeedsYou,
    /// The turn ended; a follow-up starts the next one.
    Idle,
    /// The process ended by itself.
    Ended,
    Failed,
    /// Ended by the person (or the app quitting).
    Stopped,
}

impl Status {
    pub fn live(self) -> bool {
        matches!(
            self,
            Status::Starting | Status::Working | Status::NeedsYou | Status::Idle
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermState {
    Pending,
    Allowed,
    AllowedSession,
    Denied,
    /// Allowed without a card (an earlier "na tę sesję").
    AutoAllowed,
    /// Never allowed (MontyBrain writes, vault files): no buttons.
    AutoDenied,
    /// The agent withdrew it (interrupt, end of the session).
    Withdrawn,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entry {
    User {
        id: u64,
        text: String,
    },
    Assistant {
        id: u64,
        text: String,
    },
    Tool {
        id: u64,
        name: String,
        summary: String,
        detail: String,
        status: ToolStatus,
        output: String,
    },
    Permission {
        id: u64,
        title: String,
        detail: String,
        tool: String,
        state: PermState,
        can_session: bool,
        /// Why it was denied without a card, or the answer given.
        note: String,
        questions: Vec<Question>,
    },
    Notice {
        id: u64,
        text: String,
        error: bool,
    },
}

impl Entry {
    fn id(&self) -> u64 {
        match self {
            Entry::User { id, .. }
            | Entry::Assistant { id, .. }
            | Entry::Tool { id, .. }
            | Entry::Permission { id, .. }
            | Entry::Notice { id, .. } => *id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProposalNote {
    /// Changes proposed (0: an answer without changes).
    pub count: usize,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionView {
    pub id: String,
    pub provider: ProviderId,
    pub scope: Scope,
    /// The request it started with (shown as the title).
    pub request: String,
    pub status: Status,
    pub entries: Vec<Entry>,
    pub started_at: String,
    pub ended_at: Option<String>,
    /// The last proposal taken from the agent's answer.
    pub proposal: Option<ProposalNote>,
    pub error: Option<String>,
    /// Keys allowed "na tę sesję".
    pub remembered: Vec<String>,
}

/// Where a session's changes and answers go (the Tauri side, or a test).
pub trait SessionSink: Send + Sync {
    fn changed(&self, view: &SessionView);
    /// The agent's answer with a `montybrain-zmiany` block: stored as the
    /// proposal of `view.scope`; returns how many changes it proposes.
    fn answer(&self, view: &SessionView, text: &str) -> Result<usize, String>;
}

/// Everything needed to start one session (built by [`super::RunManager`]).
#[derive(Debug, Clone)]
pub struct Launch {
    pub provider: ProviderId,
    pub scope: Scope,
    pub request: String,
    pub prompt: String,
    pub binary: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub vault: PathBuf,
    pub home: PathBuf,
    pub id: String,
}

enum Proto {
    Claude(claude::Claude),
    Codex(Box<codex::Codex>),
}

impl Proto {
    fn on_line(&mut self, line: &str, tools: &mut HashMap<String, String>) -> Vec<Act> {
        match self {
            Proto::Claude(c) => c.on_line(line, tools),
            Proto::Codex(c) => c.on_line(line),
        }
    }
    fn user(&mut self, text: &str) -> Vec<String> {
        match self {
            Proto::Claude(c) => vec![c.user(text)],
            Proto::Codex(c) => c.user(text),
        }
    }
    fn answer(&mut self, r: &Request, d: &Decision) -> Vec<String> {
        match self {
            Proto::Claude(c) => c.answer(&r.req, d),
            Proto::Codex(c) => c.answer(&r.req, d),
        }
    }
    fn interrupt(&mut self) -> Vec<String> {
        match self {
            Proto::Claude(c) => c.interrupt(),
            Proto::Codex(c) => c.interrupt(),
        }
    }
}

struct Live {
    view: SessionView,
    stdin: Option<ChildStdin>,
    pgid: u32,
    proto: Proto,
    gate: Gate,
    home: PathBuf,
    /// Cards waiting for the person: entry id → the request.
    pending: HashMap<u64, Request>,
    /// Assistant / tool entries by the CLI's key.
    keyed: HashMap<String, u64>,
    tools: HashMap<String, String>,
    next: u64,
    last_emit: Instant,
    stderr: Vec<String>,
    stderr_done: bool,
}

impl Live {
    fn push(&mut self, e: Entry) -> u64 {
        let id = e.id();
        self.view.entries.push(e);
        let over = self.view.entries.len().saturating_sub(MAX_ENTRIES);
        if over > 0 {
            self.view.entries.drain(..over);
        }
        id
    }

    fn new_id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    fn entry_mut(&mut self, id: u64) -> Option<&mut Entry> {
        self.view.entries.iter_mut().find(|e| e.id() == id)
    }

    fn write(&mut self, lines: &[String]) {
        let Some(stdin) = self.stdin.as_mut() else {
            return;
        };
        for l in lines {
            if stdin
                .write_all(l.as_bytes())
                .and_then(|()| stdin.write_all(b"\n"))
                .and_then(|()| stdin.flush())
                .is_err()
            {
                self.stdin = None;
                return;
            }
        }
    }

    fn clean(&self, s: &str) -> String {
        redact(s, &self.home)
    }

    fn refresh_status(&mut self) {
        if !self.view.status.live() {
            return;
        }
        if !self.pending.is_empty() {
            self.view.status = Status::NeedsYou;
        } else if self.view.status == Status::NeedsYou {
            self.view.status = Status::Working;
        }
    }

    /// Applies one act; returns the answer text when a turn ended with one.
    fn apply(&mut self, act: Act) -> Option<String> {
        match act {
            Act::Write(l) => self.write(&[l]),
            Act::Text { key, text, append } => {
                let text = if append { text } else { self.clean(&text) };
                match self.keyed.get(&key).copied() {
                    Some(id) => {
                        if let Some(Entry::Assistant { text: t, .. }) = self.entry_mut(id) {
                            if append {
                                t.push_str(&text);
                            } else {
                                *t = text;
                            }
                        }
                    }
                    None => {
                        let id = self.new_id();
                        self.keyed.insert(key, id);
                        self.push(Entry::Assistant { id, text });
                    }
                }
                if self.view.status == Status::Idle || self.view.status == Status::Starting {
                    self.view.status = Status::Working;
                }
            }
            Act::Tool {
                key,
                name,
                summary,
                detail,
            } => {
                let id = self.new_id();
                self.keyed.insert(key, id);
                let (summary, detail) = (self.clean(&summary), self.clean(&detail));
                self.push(Entry::Tool {
                    id,
                    name,
                    summary,
                    detail,
                    status: ToolStatus::Running,
                    output: String::new(),
                });
            }
            Act::ToolDone { key, ok, output } => {
                let output = self.clean(&output);
                if let Some(id) = self.keyed.get(&key).copied()
                    && let Some(Entry::Tool {
                        status, output: o, ..
                    }) = self.entry_mut(id)
                {
                    *status = if ok {
                        ToolStatus::Done
                    } else {
                        ToolStatus::Failed
                    };
                    *o = output;
                }
            }
            Act::Ask(r) => self.ask(*r),
            Act::Cancel { id } => {
                let gone: Vec<u64> = self
                    .pending
                    .iter()
                    .filter(|(_, r)| r.req.id == id)
                    .map(|(e, _)| *e)
                    .collect();
                for e in gone {
                    self.pending.remove(&e);
                    if let Some(Entry::Permission { state, .. }) = self.entry_mut(e) {
                        *state = PermState::Withdrawn;
                    }
                }
            }
            Act::TurnStarted => self.view.status = Status::Working,
            Act::TurnDone { text, error } => {
                if self.view.status.live() {
                    self.view.status = Status::Idle;
                }
                if let Some(e) = error {
                    let id = self.new_id();
                    let text = self.clean(&e);
                    self.push(Entry::Notice {
                        id,
                        text,
                        error: true,
                    });
                }
                if let Some(t) = text.filter(|t| {
                    t.contains(super::proposal::FENCE_INFO)
                        || t.contains(super::proposal::LEGACY_FENCE_INFO)
                }) {
                    return Some(self.clean(&t));
                }
            }
            Act::Notice { text, error } => {
                let id = self.new_id();
                let text = self.clean(&text);
                self.push(Entry::Notice { id, text, error });
            }
        }
        None
    }

    fn ask(&mut self, r: Request) {
        let id = self.new_id();
        let (title, detail) = (self.clean(&r.ask.title), self.clean(&r.ask.detail));
        let tool = r.ask.tool.clone();
        let can_session = r.ask.can_session;
        let verdict = if r.questions.is_empty() {
            self.gate.check(&r.ask)
        } else {
            Verdict::Ask
        };
        match verdict {
            Verdict::Allow => {
                let lines = self.proto.answer(&r, &Decision::Allow);
                self.write(&lines);
                // MontyBrain's read tools need no line; a remembered key gets one.
                if !super::command::MONTYBRAIN_READ_TOOLS.contains(&tool.as_str()) {
                    self.push(Entry::Permission {
                        id,
                        title,
                        detail,
                        tool,
                        state: PermState::AutoAllowed,
                        can_session,
                        note: "Zezwolono wcześniej na tę sesję.".into(),
                        questions: vec![],
                    });
                }
            }
            Verdict::Deny(why) => {
                let lines = self.proto.answer(&r, &Decision::Deny(why.clone()));
                self.write(&lines);
                self.push(Entry::Permission {
                    id,
                    title,
                    detail,
                    tool,
                    state: PermState::AutoDenied,
                    can_session: false,
                    note: why,
                    questions: vec![],
                });
            }
            Verdict::Ask => {
                let questions = r.questions.clone();
                self.pending.insert(id, r);
                self.push(Entry::Permission {
                    id,
                    title,
                    detail,
                    tool,
                    state: PermState::Pending,
                    can_session,
                    note: String::new(),
                    questions,
                });
            }
        }
        self.refresh_status();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    Allow,
    AllowSession,
    Deny,
}

/// The one session (live or the last ended one, until closed).
pub struct Sessions {
    inner: Arc<Mutex<Option<Live>>>,
    sink: Arc<dyn SessionSink>,
}

fn now() -> String {
    super::now_string()
}

impl Sessions {
    pub fn new(sink: Arc<dyn SessionSink>) -> Sessions {
        Sessions {
            inner: Arc::new(Mutex::new(None)),
            sink,
        }
    }

    pub fn view(&self) -> Option<SessionView> {
        crate::lock(&self.inner).as_ref().map(|l| l.view.clone())
    }

    /// Starts `launch`; refused while another session is live.
    pub fn start(&self, launch: Launch) -> Result<SessionView, String> {
        let mut guard = crate::lock(&self.inner);
        if guard.as_ref().is_some_and(|l| l.view.status.live()) {
            return Err("Sesja agenta już trwa: zakończ ją, zanim zaczniesz nową.".into());
        }
        let mut child = Command::new(&launch.binary)
            .args(&launch.args)
            .current_dir(&launch.cwd)
            .env_clear()
            .envs(launch.env.iter().cloned())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .map_err(|e| format!("Nie udało się uruchomić {}: {e}", launch.provider.name()))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or("Brak wyjścia procesu agenta.")?;
        let stderr = child.stderr.take();
        let pgid = child.id();
        let mut proto = match launch.provider {
            ProviderId::Claude => Proto::Claude(claude::Claude::default()),
            ProviderId::Codex => {
                Proto::Codex(Box::new(codex::Codex::new(&launch.cwd.to_string_lossy())))
            }
        };
        let first = match &mut proto {
            Proto::Claude(c) => c.start(&launch.prompt),
            Proto::Codex(c) => c.start(&launch.prompt),
        };
        let mut live = Live {
            view: SessionView {
                id: launch.id.clone(),
                provider: launch.provider,
                scope: launch.scope.clone(),
                request: launch.request.clone(),
                status: Status::Starting,
                entries: Vec::new(),
                started_at: now(),
                ended_at: None,
                proposal: None,
                error: None,
                remembered: Vec::new(),
            },
            stdin,
            pgid,
            proto,
            gate: Gate::new(&launch.vault),
            home: launch.home.clone(),
            pending: HashMap::new(),
            keyed: HashMap::new(),
            tools: HashMap::new(),
            next: 0,
            last_emit: Instant::now(),
            stderr: Vec::new(),
            stderr_done: stderr.is_none(),
        };
        let id = live.new_id();
        let shown = live.clean(&launch.request);
        live.push(Entry::User { id, text: shown });
        live.write(&first);
        let view = live.view.clone();
        *guard = Some(live);
        drop(guard);

        if let Some(mut err) = stderr {
            let inner = self.inner.clone();
            let sid = launch.id.clone();
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = (&mut err).take(256 * 1024).read_to_end(&mut buf);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(l) = crate::lock(&inner).as_mut().filter(|l| l.view.id == sid) {
                    l.stderr = text
                        .lines()
                        .rev()
                        .take(20)
                        .map(|s| l.clean(s))
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    l.stderr_done = true;
                }
            });
        }
        let inner = self.inner.clone();
        let sink = self.sink.clone();
        let sid = launch.id;
        std::thread::spawn(move || read_loop(inner, sink, sid, stdout, child));
        self.sink.changed(&view);
        Ok(view)
    }

    fn with_live<T>(&self, f: impl FnOnce(&mut Live) -> Result<T, String>) -> Result<T, String> {
        let mut g = crate::lock(&self.inner);
        let l = g.as_mut().ok_or("Nie ma sesji agenta.")?;
        if !l.view.status.live() {
            return Err("Sesja agenta już się zakończyła.".into());
        }
        let r = f(l)?;
        l.view.remembered = l.gate.remembered();
        let v = l.view.clone();
        drop(g);
        self.sink.changed(&v);
        Ok(r)
    }

    /// A follow-up message (an answer, a correction, the next request).
    pub fn send(&self, text: &str) -> Result<(), String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("Wpisz wiadomość.".into());
        }
        if text.chars().count() > super::recipes::MAX_TEXT_CHARS {
            return Err("Wiadomość jest za długa.".into());
        }
        self.with_live(|l| {
            let lines = l.proto.user(text);
            l.write(&lines);
            let id = l.new_id();
            let shown = l.clean(text);
            l.push(Entry::User { id, text: shown });
            if l.view.status != Status::NeedsYou {
                l.view.status = Status::Working;
            }
            Ok(())
        })
    }

    /// The person's answer to the card `entry`.
    pub fn answer(
        &self,
        entry: u64,
        answer: Answer,
        answers: Vec<(String, String)>,
    ) -> Result<(), String> {
        self.with_live(|l| {
            let Some(r) = l.pending.remove(&entry) else {
                return Err("Na to pytanie nie można już odpowiedzieć.".into());
            };
            let decision = if !r.questions.is_empty() {
                match answer {
                    Answer::Deny => Decision::Answers(Vec::new()),
                    _ => Decision::Answers(answers.clone()),
                }
            } else {
                match answer {
                    Answer::Allow => Decision::Allow,
                    Answer::AllowSession => {
                        if let Err(e) = l.gate.allow_for_session(&r.ask) {
                            l.pending.insert(entry, r);
                            return Err(e);
                        }
                        Decision::AllowSession
                    }
                    Answer::Deny => Decision::Deny("Użytkownik odmówił.".into()),
                }
            };
            // Defence in depth: a request the gate denies is never allowed.
            if matches!(decision, Decision::Allow | Decision::AllowSession)
                && matches!(l.gate.check(&r.ask), Verdict::Deny(_))
            {
                return Err(gate::DENY_MONTYBRAIN.into());
            }
            let lines = l.proto.answer(&r, &decision);
            l.write(&lines);
            let key = r.ask.key.clone();
            if let Some(Entry::Permission { state, note, .. }) = l.entry_mut(entry) {
                *state = match answer {
                    Answer::Allow => PermState::Allowed,
                    Answer::AllowSession => PermState::AllowedSession,
                    Answer::Deny => PermState::Denied,
                };
                if let Decision::Answers(a) = &decision {
                    *note = a
                        .iter()
                        .map(|(_, v)| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                }
            }
            // "Na tę sesję" also answers the other cards with the same key.
            if answer == Answer::AllowSession {
                let same: Vec<u64> = l
                    .pending
                    .iter()
                    .filter(|(_, x)| x.ask.key == key && x.ask.can_session)
                    .map(|(e, _)| *e)
                    .collect();
                for e in same {
                    if let Some(x) = l.pending.remove(&e) {
                        let lines = l.proto.answer(&x, &Decision::Allow);
                        l.write(&lines);
                        if let Some(Entry::Permission { state, .. }) = l.entry_mut(e) {
                            *state = PermState::AutoAllowed;
                        }
                    }
                }
            }
            l.refresh_status();
            Ok(())
        })
    }

    /// Stops the current turn; the session stays (Claude Code, Codex).
    pub fn interrupt(&self) -> Result<(), String> {
        self.with_live(|l| {
            let lines = l.proto.interrupt();
            l.write(&lines);
            let id = l.new_id();
            l.push(Entry::Notice {
                id,
                text: "Przerwano na prośbę użytkownika.".into(),
                error: false,
            });
            Ok(())
        })
    }

    /// Ends the session: the whole process group is killed.
    pub fn stop(&self) -> Result<(), String> {
        let v = {
            let mut g = crate::lock(&self.inner);
            let l = g.as_mut().ok_or("Nie ma sesji agenta.")?;
            if !l.view.status.live() {
                return Ok(());
            }
            l.view.status = Status::Stopped;
            l.view.ended_at = Some(now());
            for (e, _) in std::mem::take(&mut l.pending) {
                if let Some(Entry::Permission { state, .. }) = l.entry_mut(e) {
                    *state = PermState::Withdrawn;
                }
            }
            l.stdin = None;
            (l.view.clone(), l.pgid)
        };
        super::runner::terminate_group(v.1, Duration::from_millis(800));
        self.sink.changed(&v.0);
        Ok(())
    }

    /// Forgets an ended session (the panel's "Zamknij").
    pub fn close(&self) -> Result<(), String> {
        let mut g = crate::lock(&self.inner);
        if g.as_ref().is_some_and(|l| l.view.status.live()) {
            return Err("Sesja jeszcze trwa: najpierw ją zakończ.".into());
        }
        *g = None;
        Ok(())
    }

    /// On quit.
    pub fn shutdown(&self) {
        let _ = self.stop();
    }
}

fn read_loop(
    inner: Arc<Mutex<Option<Live>>>,
    sink: Arc<dyn SessionSink>,
    sid: String,
    stdout: std::process::ChildStdout,
    mut child: Child,
) {
    let mut reader = BufReader::new(stdout);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        let n = match (&mut reader)
            .take(MAX_LINE as u64 + 1)
            .read_until(b'\n', &mut buf)
        {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if n > MAX_LINE && !buf.ends_with(b"\n") {
            // Skip the rest of an oversized line.
            let mut sink_buf = Vec::new();
            let _ = reader.read_until(b'\n', &mut sink_buf);
            continue;
        }
        let line = String::from_utf8_lossy(&buf).to_string();
        let mut answer: Option<(SessionView, String)> = None;
        let mut emit: Option<SessionView> = None;
        {
            let mut g = crate::lock(&inner);
            let Some(l) = g.as_mut().filter(|l| l.view.id == sid) else {
                break;
            };
            if !l.view.status.live() {
                continue;
            }
            let mut tools = std::mem::take(&mut l.tools);
            let acts = l.proto.on_line(&line, &mut tools);
            l.tools = tools;
            if acts.is_empty() {
                continue;
            }
            let streamed = acts
                .iter()
                .all(|a| matches!(a, Act::Text { append: true, .. }));
            for a in acts {
                if let Some(t) = l.apply(a) {
                    answer = Some((l.view.clone(), t));
                }
            }
            l.view.remembered = l.gate.remembered();
            if !streamed || l.last_emit.elapsed() >= STREAM_EMIT {
                l.last_emit = Instant::now();
                emit = Some(l.view.clone());
            }
        }
        if let Some((view, text)) = answer {
            let note = sink.answer(&view, &text);
            let mut g = crate::lock(&inner);
            if let Some(l) = g.as_mut().filter(|l| l.view.id == sid) {
                let id = l.new_id();
                match note {
                    Ok(count) => {
                        l.view.proposal = Some(ProposalNote { count, at: now() });
                        let text = match (count, &l.view.scope) {
                            (0, _) => "Agent odpowiedział bez propozycji zmian.".to_string(),
                            (n, Scope::Inbox) => {
                                format!("Propozycje ({n}) czekają w Skrzynce: zaznacz, co zapisać.")
                            }
                            (n, Scope::Project { .. }) => format!(
                                "Propozycje ({n}) czekają w widoku projektu: zaznacz, co zapisać."
                            ),
                        };
                        l.push(Entry::Notice {
                            id,
                            text,
                            error: false,
                        });
                    }
                    Err(e) => {
                        l.push(Entry::Notice {
                            id,
                            text: e,
                            error: true,
                        });
                    }
                }
                emit = Some(l.view.clone());
            }
        }
        if let Some(v) = emit {
            sink.changed(&v);
        }
    }
    // End of output: the process is done (or closed its stdout).
    let status = child.wait().ok();
    // Give the stderr reader a moment to finish (the error message).
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline
        && crate::lock(&inner)
            .as_ref()
            .is_some_and(|l| l.view.id == sid && !l.stderr_done)
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    let v = {
        let mut g = crate::lock(&inner);
        let Some(l) = g.as_mut().filter(|l| l.view.id == sid) else {
            return;
        };
        if l.view.status.live() {
            let ok = status.is_some_and(|s| s.success());
            l.view.status = if ok { Status::Ended } else { Status::Failed };
            l.view.ended_at = Some(now());
            if !ok {
                let tail = l.stderr.join("\n");
                l.view.error = Some(if tail.trim().is_empty() {
                    "Proces agenta zakończył się błędem.".into()
                } else {
                    proto::clip(tail.trim(), 1_500)
                });
            }
            for (e, _) in std::mem::take(&mut l.pending) {
                if let Some(Entry::Permission { state, .. }) = l.entry_mut(e) {
                    *state = PermState::Withdrawn;
                }
            }
        }
        l.stdin = None;
        l.view.clone()
    };
    sink.changed(&v);
}

/// The answers to a question card: `{question id: chosen label}`.
pub fn parse_answers(v: &Value) -> Vec<(String, String)> {
    v.as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// A private directory (0700) for one session's files.
pub fn private_dir(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)))
        .map_err(|e| format!("Nie udało się przygotować sesji agenta: {e}"))
}

#[cfg(test)]
mod tests;
