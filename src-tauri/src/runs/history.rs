//! Conversation history of Claude Code and Codex as a second source for the
//! project update: what the user asked their coding agents (intent), next to
//! the repository state (results).
//!
//! Collected locally by the app, never by a model:
//!
//! * stores: Claude Code `~/.claude/projects/<cwd-slug>/<session>.jsonl`,
//!   Codex `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` (roots overridable
//!   with [`ENV_CLAUDE`] / [`ENV_CODEX`]);
//! * only sessions modified within the window whose working directory is
//!   inside one of the project's repositories or their worktrees
//!   (component-wise, so `/repo-other` is not inside `/repo`);
//! * only what the human typed: assistant text, tool results, sidechain and
//!   meta lines, harness-injected blocks (system reminders, command wrappers,
//!   task notifications, compaction summaries, Codex environment/instruction
//!   blocks) are dropped, and so are sessions recognisably started by a program
//!   or another agent (see [`claude_line_kept`], [`codex_session_kept`]);
//! * bounded: a streaming reader with a per-line cap that skips oversized
//!   lines without buffering them, a prefix check before any JSON parsing,
//!   regular files only (symlinks are not followed), a cap on files examined
//!   and an overall time budget (on exhaustion: what was found, plus a note);
//! * redacted ([`redact`]) before anything leaves the app.
//!
//! Nothing read from the files is executed or interpreted beyond JSON parsing.

use std::cmp::Reverse;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use montybrain_core::chrono::{DateTime, Local, NaiveDate, NaiveDateTime};
use serde_json::Value;

use super::runner::{Chunk, read_line_capped};

pub const ENV_CLAUDE: &str = "MONTYBRAIN_CLAUDE_HISTORY_DIR";
pub const ENV_CODEX: &str = "MONTYBRAIN_CODEX_HISTORY_DIR";
/// Pre-rename names of [`ENV_CLAUDE`] / [`ENV_CODEX`] (the new ones win).
pub const LEGACY_ENV_CLAUDE: &str = "MORWISKO_CLAUDE_HISTORY_DIR";
pub const LEGACY_ENV_CODEX: &str = "MORWISKO_CODEX_HISTORY_DIR";
/// What replaces anything that looks like a secret.
pub const SECRET: &str = "[usunięto: wygląda na sekret]";
pub const EMAIL: &str = "[usunięto: e-mail]";
pub const HISTORY_BUDGET: Duration = Duration::from_secs(20);

/// Where the two stores are.
#[derive(Debug, Clone)]
pub struct Roots {
    pub claude: PathBuf,
    pub codex: PathBuf,
}

impl Roots {
    /// The env overrides when set (tests, dev runs), else the CLIs' defaults.
    pub fn from_env(home: &Path) -> Roots {
        Roots::from_vars(home, |var| std::env::var_os(var))
    }

    fn from_vars(home: &Path, var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Roots {
        let pick = |new: &str, legacy: &str, default: PathBuf| {
            crate::settings::env_with_legacy(var(new), var(legacy)).map_or(default, PathBuf::from)
        };
        Roots {
            claude: pick(ENV_CLAUDE, LEGACY_ENV_CLAUDE, home.join(".claude/projects")),
            codex: pick(ENV_CODEX, LEGACY_ENV_CODEX, home.join(".codex/sessions")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub line_bytes: usize,
    pub max_files: usize,
    pub first_chars: usize,
    pub later_chars: usize,
    pub prompts_per_session: usize,
    pub max_sessions: usize,
    pub total_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            line_bytes: 256 * 1024,
            max_files: 400,
            first_chars: 1200,
            later_chars: 400,
            prompts_per_session: 12,
            max_sessions: 40,
            total_bytes: 48 * 1024,
        }
    }
}

/// A directory sessions may belong to (a repository or one of its worktrees),
/// with the repository name shown in the digest.
#[derive(Debug, Clone)]
pub struct Place {
    pub path: PathBuf,
    pub repo: String,
}

pub struct Request<'a> {
    pub roots: &'a Roots,
    pub home: &'a Path,
    pub places: &'a [Place],
    pub now: SystemTime,
    pub days: u16,
    pub budget: Duration,
    pub limits: Limits,
    /// The text of a request (`zlecenie`): sessions and prompts are then
    /// ranked by how many of its significant words they mention instead of
    /// by recency alone.
    pub focus: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Claude,
    Codex,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Tool::Claude => "Claude Code",
            Tool::Codex => "Codex",
        }
    }
}

#[derive(Debug, Clone)]
struct Session {
    tool: Tool,
    at: NaiveDateTime,
    repo: String,
    branch: Option<String>,
    id: String,
    prompts: Vec<String>,
}

/// The digest of the history of one project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Digest {
    /// The data block for the prompt (redacted); empty when nothing was found.
    pub text: String,
    pub claude: usize,
    pub codex: usize,
    pub omitted_sessions: usize,
    pub omitted_prompts: usize,
    /// Problems (budget, unreadable store), for the run log.
    pub notes: Vec<String>,
}

impl Digest {
    pub fn sessions(&self) -> usize {
        self.claude + self.codex
    }

    /// "Zebrano 12 sesji (Claude Code 9, Codex 3), 31 KB." or the empty case.
    pub fn summary(&self, days: u16) -> String {
        if self.sessions() == 0 {
            return format!("Brak sesji z ostatnich {days} dni w repozytoriach projektu.");
        }
        format!(
            "Zebrano {} (Claude Code {}, Codex {}), {} KB.",
            plural_sessions(self.sessions()),
            self.claude,
            self.codex,
            self.text.len().div_ceil(1024)
        )
    }
}

fn pl(n: usize, one: &str, few: &str, many: &str) -> String {
    let w = if n == 1 {
        one
    } else if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) {
        few
    } else {
        many
    };
    format!("{n} {w}")
}

fn plural_sessions(n: usize) -> String {
    let w = if n == 1 {
        "sesję"
    } else if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) {
        "sesje"
    } else {
        "sesji"
    };
    format!("{n} {w}")
}

// --- matching ------------------------------------------------------------------------

/// The place `cwd` is inside (component-wise, raw or canonical paths).
fn place_of<'p>(cwd: &str, places: &'p [Place]) -> Option<&'p Place> {
    let raw = Path::new(cwd);
    if !raw.is_absolute() {
        return None;
    }
    let canon = std::fs::canonicalize(raw).ok();
    places.iter().find(|p| {
        let pc = std::fs::canonicalize(&p.path).ok();
        let cands = [Some(raw), canon.as_deref()];
        let dirs = [Some(p.path.as_path()), pc.as_deref()];
        cands
            .iter()
            .flatten()
            .any(|c| dirs.iter().flatten().any(|d| c.starts_with(d)))
    })
}

/// Claude Code's directory name for a working directory.
fn claude_slug(p: &Path) -> String {
    p.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

// --- reading ------------------------------------------------------------------------

/// A regular file (not a symlink) and its modification time.
fn regular_file(p: &Path) -> Option<SystemTime> {
    let m = std::fs::symlink_metadata(p).ok()?;
    (m.file_type().is_file())
        .then(|| m.modified().ok())
        .flatten()
}

fn real_dir(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_dir())
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|it| it.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Calls `f` for each line (at most `cap` bytes; longer lines are skipped
/// without being buffered) until it returns `false`.
fn each_line(path: &Path, cap: usize, mut f: impl FnMut(&[u8]) -> bool) {
    let Ok(file) = std::fs::File::open(path) else {
        return;
    };
    let mut r = BufReader::new(file);
    let mut buf = Vec::new();
    loop {
        match read_line_capped(&mut r, &mut buf, cap) {
            Ok(Chunk::Line) => {
                if !f(&buf) {
                    return;
                }
            }
            Ok(Chunk::Oversized(_)) => {}
            Ok(Chunk::Eof) | Err(_) => return,
        }
    }
}

fn head_contains(line: &[u8], needle: &[u8], within: usize) -> bool {
    let h = &line[..line.len().min(within)];
    h.windows(needle.len()).any(|w| w == needle)
}

fn local_time(ts: &str) -> Option<NaiveDateTime> {
    DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|t| t.with_timezone(&Local).naive_local())
}

fn mtime_local(t: SystemTime) -> NaiveDateTime {
    DateTime::<Local>::from(t).naive_local()
}

/// Text blocks Claude Code's harness puts into user turns (dropped whole).
const CLAUDE_INJECTED: &[&str] = &[
    "<task-notification",
    "<local-command-caveat",
    "<local-command-stdout",
    "<local-command-stderr",
    "<user-prompt-submit-hook",
    "<bash-input",
    "<bash-stdout",
    "<bash-stderr",
    "Caveat: The messages below were generated",
    "[Request interrupted",
    "This session is being continued from a previous conversation",
];

/// Removes every `<tag>…</tag>` block (the harness's system reminders).
fn strip_blocks(text: &str, tag: &str) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find(&open) {
        out.push_str(&rest[..i]);
        match rest[i..].find(&close) {
            Some(j) => rest = &rest[i + j + close.len()..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let i = s.find(open)? + open.len();
    let j = s[i..].find(close)?;
    Some(&s[i..i + j])
}

/// What the human typed in one Claude Code text part, or `None`.
pub fn clean_claude_text(text: &str) -> Option<String> {
    let t = strip_blocks(text, "system-reminder");
    let t = t.trim();
    if t.is_empty() || CLAUDE_INJECTED.iter().any(|p| t.starts_with(p)) {
        return None;
    }
    // A slash command: keep the command and its arguments.
    if let Some(name) = between(t, "<command-name>", "</command-name>") {
        let args = between(t, "<command-args>", "</command-args>")
            .unwrap_or("")
            .trim();
        let s = format!("{} {args}", name.trim());
        let s = s.trim();
        return (!s.is_empty()).then(|| s.to_string());
    }
    if t.starts_with("<command-message>") {
        return None;
    }
    Some(t.to_string())
}

/// Whether a Claude Code `user` line is the human's own turn.
pub fn claude_line_kept(d: &Value) -> bool {
    let flag = |k: &str| d.get(k).and_then(Value::as_bool).unwrap_or(false);
    if flag("isSidechain")
        || flag("isMeta")
        || flag("isCompactSummary")
        || flag("isVisibleInTranscriptOnly")
    {
        return false;
    }
    // Turns that came from a background task or from another agent.
    !matches!(
        d.get("turnOrigin").and_then(Value::as_str),
        Some("task_notification" | "peer")
    )
}

/// Whether a Claude Code session was started headless by a program (`claude
/// -p`, e.g. the app's own runs): those prompts were not typed by the user.
fn claude_session_headless(d: &Value) -> bool {
    d.get("entrypoint").and_then(Value::as_str) == Some("sdk-cli")
}

fn claude_texts(d: &Value) -> Vec<String> {
    match d.pointer("/message/content") {
        Some(Value::String(s)) => clean_claude_text(s).into_iter().collect(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .filter_map(clean_claude_text)
            .collect(),
        _ => Vec::new(),
    }
}

fn read_claude(path: &Path, req: &Request, mtime: SystemTime) -> Option<Session> {
    let mut session: Option<Session> = None;
    let mut skip = false;
    each_line(path, req.limits.line_bytes, |line| {
        // Cheap prefix check: `type` is among the first keys of a user line.
        if !head_contains(line, br#""type":"user""#, 600) {
            return true;
        }
        let Ok(d) = serde_json::from_slice::<Value>(line) else {
            return true;
        };
        if d.get("type").and_then(Value::as_str) != Some("user") {
            return true;
        }
        if session.is_none() {
            if claude_session_headless(&d) {
                skip = true;
                return false;
            }
            let Some(place) = d
                .get("cwd")
                .and_then(Value::as_str)
                .and_then(|c| place_of(c, req.places))
            else {
                skip = true;
                return false;
            };
            session = Some(Session {
                tool: Tool::Claude,
                at: d
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(local_time)
                    .unwrap_or_else(|| mtime_local(mtime)),
                repo: place.repo.clone(),
                branch: d
                    .get("gitBranch")
                    .and_then(Value::as_str)
                    .filter(|b| !b.is_empty() && *b != "HEAD")
                    .map(String::from),
                id: d
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .chars()
                    .take(8)
                    .collect(),
                prompts: Vec::new(),
            });
        }
        if claude_line_kept(&d)
            && let Some(s) = session.as_mut()
        {
            s.prompts.extend(claude_texts(&d));
        }
        true
    });
    if skip {
        return None;
    }
    session.filter(|s| !s.prompts.is_empty())
}

/// Codex blocks injected into user turns (environment, instructions).
fn codex_injected(t: &str) -> bool {
    if t.starts_with("# AGENTS.md instructions") || t.starts_with("<permissions") {
        return true;
    }
    // A whole `<tag>…</tag>` block.
    let Some(rest) = t.strip_prefix('<') else {
        return false;
    };
    let tag: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    !tag.is_empty()
        && rest[tag.len()..].starts_with('>')
        && t.trim_end().ends_with(&format!("</{tag}>"))
}

/// Whether a Codex session (its `session_meta.payload`) was started by the
/// user, not by a program (`codex exec`) or by another agent (subagent,
/// review thread).
pub fn codex_session_kept(meta: &Value) -> bool {
    let s = |k: &str| meta.get(k).and_then(Value::as_str);
    if s("originator") == Some("codex_exec") || s("source") == Some("exec") {
        return false;
    }
    if meta.get("source").is_some_and(Value::is_object) {
        return false;
    }
    if meta.get("parent_thread_id").is_some_and(|v| !v.is_null()) {
        return false;
    }
    match meta.get("thread_source") {
        None | Some(Value::Null) => true,
        Some(v) => v.as_str() == Some("user"),
    }
}

fn read_codex(path: &Path, req: &Request, mtime: SystemTime) -> Option<Session> {
    let mut session: Option<Session> = None;
    let mut skip = false;
    let mut first = true;
    each_line(path, req.limits.line_bytes, |line| {
        if first {
            first = false;
            let meta = serde_json::from_slice::<Value>(line).ok();
            let Some(p) = meta
                .as_ref()
                .filter(|d| d.get("type").and_then(Value::as_str) == Some("session_meta"))
                .and_then(|d| d.get("payload"))
            else {
                skip = true;
                return false;
            };
            if !codex_session_kept(p) {
                skip = true;
                return false;
            }
            let Some(place) = p
                .get("cwd")
                .and_then(Value::as_str)
                .and_then(|c| place_of(c, req.places))
            else {
                skip = true;
                return false;
            };
            session = Some(Session {
                tool: Tool::Codex,
                at: p
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(local_time)
                    .unwrap_or_else(|| mtime_local(mtime)),
                repo: place.repo.clone(),
                branch: p
                    .pointer("/git/branch")
                    .and_then(Value::as_str)
                    .filter(|b| !b.is_empty())
                    .map(String::from),
                id: p
                    .get("id")
                    .or_else(|| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .chars()
                    .take(8)
                    .collect(),
                prompts: Vec::new(),
            });
            return true;
        }
        if !head_contains(line, br#""response_item""#, 200)
            || !head_contains(line, br#""role":"user""#, 400)
        {
            return true;
        }
        let Ok(d) = serde_json::from_slice::<Value>(line) else {
            return true;
        };
        let Some(p) = d.get("payload") else {
            return true;
        };
        if p.get("type").and_then(Value::as_str) != Some("message")
            || p.get("role").and_then(Value::as_str) != Some("user")
        {
            return true;
        }
        if let (Some(s), Some(Value::Array(parts))) = (session.as_mut(), p.get("content")) {
            for part in parts {
                if part.get("type").and_then(Value::as_str) != Some("input_text") {
                    continue;
                }
                let Some(t) = part.get("text").and_then(Value::as_str) else {
                    continue;
                };
                let t = t.trim();
                if !t.is_empty() && !codex_injected(t) {
                    s.prompts.push(t.to_string());
                }
            }
        }
        true
    });
    if skip {
        return None;
    }
    session.filter(|s| !s.prompts.is_empty())
}

// --- collecting -----------------------------------------------------------------------

struct Candidate {
    tool: Tool,
    path: PathBuf,
    mtime: SystemTime,
}

fn claude_candidates(req: &Request, cutoff: SystemTime, out: &mut Vec<Candidate>) {
    let slugs: Vec<String> = req
        .places
        .iter()
        .flat_map(|p| {
            let mut v = vec![claude_slug(&p.path)];
            if let Ok(c) = std::fs::canonicalize(&p.path) {
                v.push(claude_slug(&c));
            }
            v
        })
        .map(|s| s.chars().take(150).collect())
        .collect();
    for dir in entries(&req.roots.claude) {
        let Some(name) = dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // A prefilter only (the real check is the recorded cwd); long names may
        // be shortened by Claude Code, so those are always examined.
        if name.len() < 150 && !slugs.iter().any(|s| name.starts_with(s.as_str())) {
            continue;
        }
        if !real_dir(&dir) {
            continue;
        }
        for f in entries(&dir) {
            if f.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if let Some(m) = regular_file(&f).filter(|m| *m >= cutoff) {
                out.push(Candidate {
                    tool: Tool::Claude,
                    path: f,
                    mtime: m,
                });
            }
        }
    }
}

fn codex_candidates(req: &Request, cutoff: SystemTime, out: &mut Vec<Candidate>) {
    let cutoff_day = mtime_local(cutoff).date() - montybrain_core::chrono::TimeDelta::days(1);
    let num = |p: &Path| {
        p.file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.parse::<u32>().ok())
    };
    for y in entries(&req.roots.codex) {
        let Some(yy) = num(&y).filter(|_| real_dir(&y)) else {
            continue;
        };
        for m in entries(&y) {
            let Some(mm) = num(&m).filter(|_| real_dir(&m)) else {
                continue;
            };
            for d in entries(&m) {
                let Some(dd) = num(&d).filter(|_| real_dir(&d)) else {
                    continue;
                };
                let Some(day) = NaiveDate::from_ymd_opt(yy as i32, mm, dd) else {
                    continue;
                };
                if day < cutoff_day {
                    continue;
                }
                for f in entries(&d) {
                    let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if !(name.starts_with("rollout-") && name.ends_with(".jsonl")) {
                        continue;
                    }
                    if let Some(t) = regular_file(&f).filter(|t| *t >= cutoff) {
                        out.push(Candidate {
                            tool: Tool::Codex,
                            path: f,
                            mtime: t,
                        });
                    }
                }
            }
        }
    }
}

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max).collect();
    t.push('…');
    t
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Collects and digests the history for the places of one project.
pub fn collect(req: &Request) -> Digest {
    let started = Instant::now();
    let mut digest = Digest::default();
    if req.places.is_empty() {
        return digest;
    }
    let cutoff = req
        .now
        .checked_sub(Duration::from_secs(u64::from(req.days) * 86_400))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut cands = Vec::new();
    claude_candidates(req, cutoff, &mut cands);
    codex_candidates(req, cutoff, &mut cands);
    cands.sort_by_key(|c| Reverse(c.mtime));
    if cands.len() > req.limits.max_files {
        digest.notes.push(format!(
            "Historia: przejrzano {} najnowszych plików z {}.",
            req.limits.max_files,
            cands.len()
        ));
        cands.truncate(req.limits.max_files);
    }
    let mut sessions: Vec<Session> = Vec::new();
    for c in &cands {
        if started.elapsed() >= req.budget {
            digest.notes.push(format!(
                "Przerwano zbieranie historii po {} s; część sesji pominięto.",
                req.budget.as_secs()
            ));
            break;
        }
        let s = match c.tool {
            Tool::Claude => read_claude(&c.path, req, c.mtime),
            Tool::Codex => read_codex(&c.path, req, c.mtime),
        };
        sessions.extend(s);
    }
    build(&mut digest, sessions, req);
    digest
}

// --- request focus ----------------------------------------------------------------------

/// Words of a request that say what to do or where to look, not what about.
const FOCUS_STOPWORDS: &[&str] = &[
    "ale",
    "albo",
    "dla",
    "do",
    "jak",
    "jest",
    "jego",
    "jej",
    "ich",
    "lub",
    "mam",
    "mnie",
    "moj",
    "moja",
    "moje",
    "mojego",
    "na",
    "nad",
    "nie",
    "od",
    "oraz",
    "po",
    "pod",
    "przez",
    "przy",
    "sie",
    "tak",
    "ten",
    "ta",
    "te",
    "tego",
    "tej",
    "tym",
    "to",
    "tu",
    "co",
    "czy",
    "ze",
    "za",
    "jaki",
    "jakie",
    "wszystko",
    "wszystkie",
    "wszystkich",
    "ktore",
    "ktory",
    "ktora",
    "teraz",
    "potem",
    "znajdz",
    "poszukaj",
    "wyszukaj",
    "sprawdz",
    "napisz",
    "rozpisz",
    "opisz",
    "zrob",
    "dodaj",
    "zaktualizuj",
    "podsumuj",
    "przygotuj",
    "zbierz",
    "uzupelnij",
    "zapisz",
    "przejrzyj",
    "informacje",
    "informacji",
    "informacja",
    "temat",
    "tematu",
    "tematy",
    "tematow",
    "temacie",
    "zadanie",
    "zadania",
    "zadan",
    "podstawie",
    "historii",
    "historia",
    "historie",
    "rozmow",
    "rozmowy",
    "sesji",
    "sesje",
    "poczta",
    "poczty",
    "poczcie",
    "mail",
    "maila",
    "maile",
    "mailach",
    "mailu",
    "slack",
    "slacka",
    "slacku",
    "t3",
    "code",
    "claude",
    "codex",
    "montybrain",
    "montybraina",
    "montybrainie",
    "montybrainem",
    // The pre-rename name, still found in older sessions.
    "morwisko",
    "morwisku",
    "morwiska",
    "projekt",
    "projektu",
    "projekcie",
    "the",
    "and",
    "for",
    "about",
    "find",
    "write",
    "topic",
    "with",
    "from",
];

/// Lower case without Polish diacritics (and the common Latin ones).
pub fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'ą' | 'á' | 'à' | 'ä' | 'â' => 'a',
            'ć' | 'č' | 'ç' => 'c',
            'ę' | 'é' | 'è' | 'ë' | 'ê' | 'ě' => 'e',
            'ł' => 'l',
            'ń' | 'ň' | 'ñ' => 'n',
            'ó' | 'ö' | 'ô' | 'ò' => 'o',
            'ś' | 'š' => 's',
            'ź' | 'ż' | 'ž' => 'z',
            'í' | 'ï' => 'i',
            'ú' | 'ü' | 'ů' => 'u',
            'ý' => 'y',
            'ř' => 'r',
            other => other,
        })
        .collect()
}

fn words(folded: &str) -> impl Iterator<Item = &str> {
    folded
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
}

/// The significant words of a request as prefixes that also match their
/// inflected forms ("płatności" → "platnos", which "płatność" and
/// "płatnościami" start with): short words whole, longer ones without their
/// last two letters (at least 4, at most 8 letters kept).
pub fn focus_stems(request: &str) -> Vec<String> {
    let folded = fold(request);
    let mut out: Vec<String> = Vec::new();
    for w in words(&folded) {
        let n = w.chars().count();
        if n < 3 || FOCUS_STOPWORDS.contains(&w) {
            continue;
        }
        let stem: String = if n <= 4 {
            w.to_string()
        } else {
            w.chars().take((n - 2).clamp(4, 8)).collect()
        };
        if !out.contains(&stem) {
            out.push(stem);
        }
        if out.len() == 24 {
            break;
        }
    }
    out
}

/// How many distinct `stems` the words of `text` start with.
pub fn focus_score(text: &str, stems: &[String]) -> usize {
    if stems.is_empty() {
        return 0;
    }
    let folded = fold(text);
    let mut hit = vec![false; stems.len()];
    for w in words(&folded) {
        for (i, s) in stems.iter().enumerate() {
            if !hit[i] && w.starts_with(s.as_str()) {
                hit[i] = true;
            }
        }
    }
    hit.into_iter().filter(|h| *h).count()
}

/// Which prompts of a long session to keep (indices, in order): the first
/// (the goal) and the most recent ones, or, with a focus, the ones that
/// mention the request most (recency breaks ties; without any match the
/// recent ones).
fn kept_prompts(prompts: &[String], keep: usize, stems: &[String]) -> Vec<usize> {
    let n = prompts.len();
    if n <= keep {
        return (0..n).collect();
    }
    let tail = keep - 1;
    let scores: Vec<usize> = prompts.iter().map(|p| focus_score(p, stems)).collect();
    let mut rest: Vec<usize> = (1..n).collect();
    if scores[1..].iter().any(|s| *s > 0) {
        rest.sort_by_key(|i| (Reverse(scores[*i]), Reverse(*i)));
        rest.truncate(tail);
        rest.sort_unstable();
    } else {
        rest = (n - tail..n).collect();
    }
    std::iter::once(0).chain(rest).collect()
}

fn build(digest: &mut Digest, mut sessions: Vec<Session>, req: &Request) {
    let l = req.limits;
    let stems = req.focus.map(focus_stems).unwrap_or_default();
    if stems.is_empty() {
        sessions.sort_by_key(|s| Reverse(s.at));
    } else {
        // The sessions that mention the request most, then the most recent.
        let score = |s: &Session| {
            let all = s.prompts.join("\n");
            let per_prompt = s
                .prompts
                .iter()
                .filter(|p| focus_score(p, &stems) > 0)
                .count();
            (focus_score(&all, &stems), per_prompt)
        };
        let mut scored: Vec<((usize, usize), Session)> =
            sessions.into_iter().map(|s| (score(&s), s)).collect();
        scored.sort_by_key(|(k, s)| (Reverse(*k), Reverse(s.at)));
        sessions = scored.into_iter().map(|(_, s)| s).collect();
    }
    if sessions.len() > l.max_sessions {
        digest.omitted_sessions += sessions.len() - l.max_sessions;
        sessions.truncate(l.max_sessions);
    }
    let mut blocks: Vec<(NaiveDateTime, String, Tool)> = Vec::new();
    let mut total = 0usize;
    for (i, s) in sessions.iter().enumerate() {
        let n = s.prompts.len();
        let keep: Vec<(usize, &String)> = kept_prompts(&s.prompts, l.prompts_per_session, &stems)
            .into_iter()
            .map(|i| (i, &s.prompts[i]))
            .collect();
        let dropped = n - keep.len();
        let mut b = format!(
            "=== {} · {} · {}{} · sesja {} ===\n",
            s.tool.name(),
            s.at.format("%Y-%m-%d %H:%M"),
            s.repo,
            s.branch
                .as_ref()
                .map(|b| format!(" · gałąź {b}"))
                .unwrap_or_default(),
            if s.id.is_empty() { "?" } else { &s.id }
        );
        for (k, (idx, p)) in keep.iter().enumerate() {
            let max = if *idx == 0 {
                l.first_chars
            } else {
                l.later_chars
            };
            let text = cut(&one_line(&redact(p, req.home)), max);
            b.push_str(&format!("{}. {text}\n", k + 1));
            if k == 0 && dropped > 0 {
                let what = if stems.is_empty() {
                    pl(
                        dropped,
                        "kolejne polecenie",
                        "kolejne polecenia",
                        "kolejnych poleceń",
                    )
                } else {
                    pl(
                        dropped,
                        "polecenie mniej pasujące do zlecenia",
                        "polecenia mniej pasujące do zlecenia",
                        "poleceń mniej pasujących do zlecenia",
                    )
                };
                b.push_str(&format!("(pominięto {what})\n"));
            }
        }
        if total + b.len() + 1 > l.total_bytes {
            digest.omitted_sessions += sessions.len() - i;
            break;
        }
        total += b.len() + 1;
        digest.omitted_prompts += dropped;
        match s.tool {
            Tool::Claude => digest.claude += 1,
            Tool::Codex => digest.codex += 1,
        }
        blocks.push((s.at, b, s.tool));
    }
    // Oldest first, as the work happened.
    blocks.sort_by_key(|(at, _, _)| *at);
    let mut text = blocks
        .into_iter()
        .map(|(_, b, _)| b)
        .collect::<Vec<_>>()
        .join("\n");
    if digest.omitted_sessions > 0 || digest.omitted_prompts > 0 {
        text.push_str(&format!(
            "\n(Pominięto: {}, {} w środku długich sesji.)\n",
            if stems.is_empty() {
                pl(
                    digest.omitted_sessions,
                    "starszą sesję",
                    "starsze sesje",
                    "starszych sesji",
                )
            } else {
                pl(
                    digest.omitted_sessions,
                    "mniej pasującą sesję",
                    "mniej pasujące sesje",
                    "mniej pasujących sesji",
                )
            },
            pl(digest.omitted_prompts, "polecenie", "polecenia", "poleceń")
        ));
    }
    digest.text = text;
}

// --- redaction ------------------------------------------------------------------------

const KEY_PREFIXES: &[&str] = &[
    "sk-",
    "sk_",
    "ghp_",
    "gho_",
    "ghs_",
    "ghu_",
    "github_pat_",
    "glpat-",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xoxr-",
    "xoxs-",
    "AKIA",
    "ASIA",
    "AIza",
    "pk_live_",
    "rk_live_",
    "sk_live_",
    "npm_",
];

const ASSIGN_KEYS: &[&str] = &[
    "password",
    "passwd",
    "pwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "api-key",
    "access_key",
    "client_secret",
    "private_key",
    "haslo",
    "hasło",
];

fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '~' | '+' | '/' | '=' | '-' | '@' | '%')
}

fn entropy(s: &str) -> f64 {
    let mut counts = [0usize; 128];
    let mut n = 0usize;
    for b in s.bytes().filter(u8::is_ascii) {
        counts[b as usize] += 1;
        n += 1;
    }
    if n == 0 {
        return 0.0;
    }
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n as f64;
            -p * p.log2()
        })
        .sum()
}

fn is_email(t: &str) -> bool {
    let Some((local, domain)) = t.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._%+-".contains(c))
        && domain.contains('.')
        && !domain.starts_with('.')
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))
        && domain
            .rsplit('.')
            .next()
            .is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()))
}

fn looks_secret(t: &str) -> bool {
    let core = t.trim_matches(|c: char| matches!(c, '.' | ',' | ';' | ':'));
    if KEY_PREFIXES.iter().any(|p| core.starts_with(p)) && core.len() >= 20 {
        return true;
    }
    // Long high-entropy strings (not URLs or paths, not git hashes).
    if core.len() >= 32 && !core.contains("://") && !core.starts_with('/') && !core.starts_with('~')
    {
        let hex = core.chars().all(|c| c.is_ascii_hexdigit());
        let letters = core.chars().any(|c| c.is_ascii_alphabetic());
        let digits = core.chars().any(|c| c.is_ascii_digit());
        if !hex && letters && digits && entropy(core) > 4.0 {
            return true;
        }
    }
    false
}

/// Replaces private-key blocks.
fn redact_key_blocks(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("-----BEGIN ") {
        let after = &rest[i..];
        let header_end = after
            .find("-----\n")
            .or_else(|| after[11..].find("-----").map(|j| j + 11));
        if !after[..after.len().min(80)].contains("PRIVATE KEY") || header_end.is_none() {
            out.push_str(&rest[..i + 11]);
            rest = &rest[i + 11..];
            continue;
        }
        out.push_str(&rest[..i]);
        out.push_str(SECRET);
        match after.find("-----END ") {
            Some(j) => {
                let tail = &after[j + 9..];
                let k = tail.find("-----").map_or(tail.len(), |k| k + 5);
                rest = &tail[k..];
            }
            None => rest = "",
        }
    }
    out.push_str(rest);
    out
}

/// Replaces values after `password=`-style keys and after `Bearer`.
fn redact_assignments(text: &str) -> String {
    let lower = text.to_lowercase();
    if lower.len() != text.len() {
        // Non-ASCII case mapping changed lengths: work on chars instead.
        return redact_assignments_chars(text);
    }
    let bytes = text.as_bytes();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let find_value = |from: usize, spans: &mut Vec<(usize, usize)>| {
        let mut i = from;
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
            i += 1;
        }
        if i < bytes.len() && matches!(bytes[i], b'=' | b':') {
            i += 1;
            while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'"' | b'\'') {
                i += 1;
            }
            let start = i;
            while i < bytes.len()
                && !bytes[i].is_ascii_whitespace()
                && !matches!(bytes[i], b'"' | b'\'' | b',' | b';')
            {
                i += 1;
            }
            if i > start {
                spans.push((start, i));
            }
        }
    };
    for key in ASSIGN_KEYS.iter().filter(|k| k.is_ascii()) {
        let mut from = 0;
        while let Some(pos) = lower[from..].find(key) {
            let at = from + pos;
            let end = at + key.len();
            let boundary = at == 0 || !bytes[at - 1].is_ascii_alphanumeric();
            if boundary {
                find_value(end, &mut spans);
            }
            from = end;
        }
    }
    let mut from = 0;
    while let Some(pos) = lower[from..].find("bearer ") {
        let start = from + pos + 7;
        let mut i = start;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && !matches!(bytes[i], b'"' | b'\'')
        {
            i += 1;
        }
        if i - start >= 8 {
            spans.push((start, i));
        }
        from = start;
    }
    spans.sort();
    let mut out = String::new();
    let mut last = 0;
    for (s, e) in spans {
        if s < last {
            continue;
        }
        out.push_str(&text[last..s]);
        out.push_str(SECRET);
        last = e;
    }
    out.push_str(&text[last..]);
    out
}

fn redact_assignments_chars(text: &str) -> String {
    // Rare path (non-ASCII letters whose lowercase has another length): fall
    // back to a per-line check that drops the whole value part of a line with
    // a secret-like key.
    text.lines()
        .map(|l| {
            let low = l.to_lowercase();
            if ASSIGN_KEYS.iter().any(|k| low.contains(k)) && (l.contains('=') || l.contains(':')) {
                let cut = l.find(['=', ':']).unwrap_or(l.len());
                format!("{} {SECRET}", &l[..=cut.min(l.len() - 1)])
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Replaces e-mail addresses and token-like secrets, word by word.
fn redact_tokens(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if word.is_empty() {
            return;
        }
        if word.contains(SECRET) {
            out.push_str(word);
        } else if is_email(word.trim_end_matches(['.', ','])) {
            out.push_str(EMAIL);
            let trail: String = word
                .chars()
                .rev()
                .take_while(|c| *c == '.' || *c == ',')
                .collect();
            out.push_str(&trail);
        } else if looks_secret(word) {
            out.push_str(SECRET);
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in text.chars() {
        if token_char(c) {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// Redaction before anything leaves the app: private keys, `password=`-style
/// values, bearer tokens, common key shapes, long high-entropy strings and
/// e-mail addresses are replaced; the home directory becomes `~`.
pub fn redact(text: &str, home: &Path) -> String {
    let mut t = redact_key_blocks(text);
    t = redact_assignments(&t);
    t = redact_tokens(&t);
    if let Some(h) = home.to_str().filter(|h| h.len() > 1) {
        let h = h.trim_end_matches('/');
        t = t.replace(&format!("{h}/"), "~/");
        if t.ends_with(h) || t.contains(&format!("{h} ")) {
            t = t.replace(h, "~");
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    const HOME: &str = "/Users/ala";

    #[test]
    fn history_roots_honour_new_then_legacy_variables() {
        let home = Path::new(HOME);
        let vars = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| std::ffi::OsString::from(v))
            }
        };
        let r = Roots::from_vars(home, vars(&[]));
        assert_eq!(r.claude, home.join(".claude/projects"));
        assert_eq!(r.codex, home.join(".codex/sessions"));
        let r = Roots::from_vars(
            home,
            vars(&[
                ("MORWISKO_CLAUDE_HISTORY_DIR", "/old/c"),
                ("MORWISKO_CODEX_HISTORY_DIR", "/old/x"),
                ("MONTYBRAIN_CODEX_HISTORY_DIR", "/new/x"),
            ]),
        );
        assert_eq!(r.claude, PathBuf::from("/old/c"));
        assert_eq!(r.codex, PathBuf::from("/new/x"));
    }

    #[test]
    fn redaction_of_synthetic_secrets() {
        let home = Path::new(HOME);
        let cases = [
            (
                "klucz sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWx0123 do API",
                "klucz [usunięto: wygląda na sekret] do API",
            ),
            (
                "token ghp_aBcDeFgHiJkLmNoPqRsTuVwXyZ012345 w repo",
                "token [usunięto: wygląda na sekret] w repo",
            ),
            (
                "AKIAABCDEFGHIJKLMNOP i AIzaSyA-1234567890abcdefghijklmnopqrstu",
                "[usunięto: wygląda na sekret] i [usunięto: wygląda na sekret]",
            ),
            (
                "Authorization: Bearer abc.def-ghi_jkl123",
                "Authorization: Bearer [usunięto: wygląda na sekret]",
            ),
            (
                "ustaw password=Tajne!123 i dalej",
                "ustaw password=[usunięto: wygląda na sekret] i dalej",
            ),
            (
                "DB_PASSWORD: \"x9y8z7\"",
                "DB_PASSWORD: \"[usunięto: wygląda na sekret]\"",
            ),
            (
                "api_key = 'qwerty'",
                "api_key = '[usunięto: wygląda na sekret]'",
            ),
            (
                "napisz do ala.kot@example.com.",
                "napisz do [usunięto: e-mail].",
            ),
            (
                "plik /Users/ala/Projects/app/src i /Users/ala",
                "plik ~/Projects/app/src i ~",
            ),
            (
                "losowe Zm9vYmFyYmF6cXV4MTIzNDU2Nzg5MGFiY2RlZmdo",
                "losowe [usunięto: wygląda na sekret]",
            ),
        ];
        for (input, want) in cases {
            assert_eq!(redact(input, home), want, "{input}");
        }
        let key = "przed\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAA\nAAAA\n-----END OPENSSH PRIVATE KEY-----\npo";
        assert_eq!(redact(key, home), format!("przed\n{SECRET}\npo"));
        // Not secrets: git hashes, URLs, paths, ordinary words, UUID-less text.
        for keep in [
            "commit 3f9a2b1c4d5e6f708192a3b4c5d6e7f8091a2b3c",
            "https://gitlab.com/whenubuy/backend/-/merge_requests/42",
            "/opt/homebrew/bin/git status",
            "Dodaj tryb offline do aplikacji kasowej",
            "tokenizacja tekstu i passwordless login",
        ] {
            assert_eq!(redact(keep, home), keep, "{keep}");
        }
    }

    // --- fixtures (synthetic) ---------------------------------------------------------

    struct Fx {
        _tmp: tempfile::TempDir,
        roots: Roots,
        repo: PathBuf,
        worktree: PathBuf,
    }

    fn fx() -> Fx {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let roots = Roots {
            claude: base.join("claude"),
            codex: base.join("codex"),
        };
        let repo = base.join("repo");
        let worktree = base.join("repo-wt-feature");
        for d in [
            &roots.claude,
            &roots.codex,
            &repo,
            &worktree,
            &base.join("repo-other"),
        ] {
            std::fs::create_dir_all(d).unwrap();
        }
        Fx {
            _tmp: tmp,
            roots,
            repo,
            worktree,
        }
    }

    fn places(f: &Fx) -> Vec<Place> {
        vec![
            Place {
                path: f.repo.clone(),
                repo: "app".into(),
            },
            Place {
                path: f.worktree.clone(),
                repo: "app".into(),
            },
        ]
    }

    fn claude_user(cwd: &Path, ts: &str, content: Value, extra: Value) -> String {
        let mut d = serde_json::json!({
            "parentUuid": null, "isSidechain": false, "promptId": "p", "type": "user",
            "message": {"role": "user", "content": content},
            "uuid": "u", "timestamp": ts, "userType": "external", "entrypoint": "sdk-ts",
            "cwd": cwd.to_str().unwrap(), "sessionId": "c0ffee11-2222-3333-4444-555566667777",
            "gitBranch": "feature/kasa"
        });
        for (k, v) in extra.as_object().unwrap() {
            d[k] = v.clone();
        }
        // The key order of real files: `type` is among the first keys (the
        // reader's prefix check relies on it); serde_json sorts keys.
        let obj = d.as_object_mut().unwrap();
        let mut head = String::from("{");
        for k in ["parentUuid", "isSidechain", "promptId", "type"] {
            let v = obj.remove(k).unwrap();
            head.push_str(&format!("{}:{v},", serde_json::to_string(k).unwrap()));
        }
        let rest = d.to_string();
        format!("{head}{}", &rest[1..])
    }

    fn write_claude(f: &Fx, cwd: &Path, name: &str, lines: &[String]) -> PathBuf {
        let dir = f.roots.claude.join(claude_slug(cwd));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("{name}.jsonl"));
        std::fs::write(&p, lines.join("\n") + "\n").unwrap();
        p
    }

    fn codex_meta(cwd: &Path, extra: Value) -> String {
        let mut p = serde_json::json!({
            "id": "0a1b2c3d-0000-0000-0000-000000000000", "timestamp": "2026-11-12T09:00:00Z",
            "cwd": cwd.to_str().unwrap(), "originator": "T3 Code", "source": "vscode",
            "base_instructions": {"text": "x"}
        });
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        serde_json::json!({"timestamp": "2026-11-12T09:00:00Z", "ordinal": 0, "type": "session_meta", "payload": p}).to_string()
    }

    fn codex_msg(role: &str, text: &str) -> String {
        serde_json::json!({"timestamp": "2026-11-12T09:01:00Z", "type": "response_item",
            "payload": {"type": "message", "role": role, "content": [{"type": "input_text", "text": text}]}})
        .to_string()
    }

    fn write_codex(f: &Fx, day: &str, name: &str, lines: &[String]) -> PathBuf {
        let dir = f.roots.codex.join(day);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("rollout-{name}.jsonl"));
        std::fs::write(&p, lines.join("\n") + "\n").unwrap();
        p
    }

    fn today_dir() -> String {
        Local::now().format("%Y/%m/%d").to_string()
    }

    fn req<'a>(f: &'a Fx, places: &'a [Place], days: u16, limits: Limits) -> Request<'a> {
        Request {
            roots: &f.roots,
            home: Path::new(HOME),
            places,
            now: SystemTime::now(),
            days,
            budget: Duration::from_secs(10),
            limits,
            focus: None,
        }
    }

    #[test]
    fn focus_stems_and_inflected_forms() {
        // Instruction and source words drop out; the rest become prefixes.
        assert_eq!(
            focus_stems(
                "Znajdź informacje o płatnościach kartą na Slacku, w mailach i T3 Code i rozpisz temat"
            ),
            ["platnosc", "kart"]
        );
        assert_eq!(
            focus_stems("Migracja bazy do Postgresa"),
            ["migrac", "bazy", "postgre"]
        );
        assert_eq!(focus_stems("integracja z KSeF"), ["integrac", "ksef"]);
        assert!(focus_stems("rozpisz temat na podstawie historii rozmów").is_empty());
        let stems = focus_stems("płatności kartą");
        for (text, n) in [
            ("Dodaj płatność kartą", 2),
            ("PLATNOSCI karty", 2),
            ("obsługa płatnościami i kartę", 2),
            ("karta podarunkowa", 1),
            ("plan wydań", 0),
            ("kartoteka", 1), // a prefix match, accepted
        ] {
            assert_eq!(focus_score(text, &stems), n, "{text}");
        }
        assert_eq!(fold("Zażółć GĘŚLĄ jaźń"), "zazolc gesla jazn");
    }

    #[test]
    fn focus_ranks_sessions_and_prompts_by_the_request() {
        let f = fx();
        // Newest first by time, but only the oldest is about payments.
        let one = |name: &str, ts: &str, text: &str| {
            write_claude(
                &f,
                &f.repo,
                name,
                &[claude_user(
                    &f.repo,
                    ts,
                    Value::String(text.into()),
                    serde_json::json!({}),
                )],
            );
        };
        one(
            "a",
            "2026-11-01T08:00:00Z",
            "Zrób obsługę płatności kartą w kasie",
        );
        one("b", "2026-11-11T08:00:00Z", "Popraw literówki w README");
        one("c", "2026-11-12T08:00:00Z", "Odśwież zależności");
        one("d", "2026-11-13T08:00:00Z", "Dodaj płatność BLIK obok kart");
        let pl = places(&f);
        let limits = Limits {
            max_sessions: 2,
            ..Limits::default()
        };
        let mut r = req(&f, &pl, 30, limits);
        let plain = collect(&r);
        assert!(
            plain.text.contains("zależności") && !plain.text.contains("kasie"),
            "{}",
            plain.text
        );
        r.focus = Some("Rozpisz temat płatności kartą na podstawie historii rozmów");
        let d = collect(&r);
        assert!(
            d.text.contains("płatności kartą") && d.text.contains("BLIK"),
            "{}",
            d.text
        );
        assert!(!d.text.contains("README") && !d.text.contains("zależności"));
        assert!(d.text.contains("2 mniej pasujące sesje"), "{}", d.text);
        // Oldest first in the digest, whatever the rank.
        assert!(d.text.find("kasie").unwrap() < d.text.find("BLIK").unwrap());
        // Nothing matches: recency, as without a focus.
        r.focus = Some("zupełnie inna sprawa logistyczna");
        assert_eq!(
            collect(&r).text.replace("mniej pasujące", "starsze"),
            plain.text
        );

        // In a long session the matching prompts are kept, not the latest.
        let mut lines = Vec::new();
        for i in 0..10 {
            let text = match i {
                0 => "Cel: kasa fiskalna".to_string(),
                3 => "teraz płatności zbliżeniowe".to_string(),
                5 => "a karty podarunkowe?".to_string(),
                _ => format!("drobiazg {i}"),
            };
            lines.push(claude_user(
                &f.repo,
                &format!("2026-11-13T10:{i:02}:00Z"),
                Value::String(text),
                serde_json::json!({}),
            ));
        }
        write_claude(&f, &f.repo, "long", &lines);
        let limits = Limits {
            prompts_per_session: 4,
            max_sessions: 1,
            ..Limits::default()
        };
        let mut r = req(&f, &pl, 30, limits);
        r.focus = Some("płatności kartą");
        let d = collect(&r);
        let session: Vec<&str> = d.text.lines().take(6).collect();
        assert_eq!(
            session[1..],
            [
                "1. Cel: kasa fiskalna",
                "(pominięto 6 poleceń mniej pasujących do zlecenia)",
                "2. teraz płatności zbliżeniowe",
                "3. a karty podarunkowe?",
                "4. drobiazg 9",
            ],
            "{}",
            d.text
        );
    }

    #[test]
    fn claude_prompts_only_what_the_human_typed() {
        let f = fx();
        let ts = "2026-11-12T10:00:00Z";
        let lines = vec![
            r#"{"type":"queue-operation","operation":"enqueue"}"#.to_string(),
            claude_user(&f.repo, ts, Value::String("Zrób tryb offline w aplikacji kasowej <system-reminder>wstrzyknięte</system-reminder>".into()), serde_json::json!({})),
            serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":"ODPOWIEDŹ AGENTA"}]}}).to_string(),
            claude_user(&f.repo, ts, serde_json::json!([{"type":"tool_result","content":"WYNIK NARZĘDZIA"}]), serde_json::json!({})),
            claude_user(&f.repo, ts, serde_json::json!([{"type":"text","text":"Dodaj kolejkę transakcji"},{"type":"image"}]), serde_json::json!({})),
            claude_user(&f.repo, ts, Value::String("<task-notification><task-id>1</task-id></task-notification>".into()), serde_json::json!({})),
            claude_user(&f.repo, ts, Value::String("PODAGENT".into()), serde_json::json!({"isSidechain": true})),
            claude_user(&f.repo, ts, Value::String("META".into()), serde_json::json!({"isMeta": true})),
            claude_user(&f.repo, ts, Value::String("STRESZCZENIE".into()), serde_json::json!({"isCompactSummary": true})),
            claude_user(&f.repo, ts, Value::String("OD INNEGO AGENTA".into()), serde_json::json!({"turnOrigin": "peer"})),
            claude_user(&f.repo, ts, Value::String("<command-name>/review</command-name>\n<command-message>review</command-message>\n<command-args>gałąź kasy</command-args>".into()), serde_json::json!({})),
            claude_user(&f.repo, ts, Value::String("<local-command-stdout>wynik</local-command-stdout>".into()), serde_json::json!({})),
        ];
        write_claude(&f, &f.repo, "s1", &lines);
        let pl = places(&f);
        let d = collect(&req(&f, &pl, 14, Limits::default()));
        assert_eq!((d.claude, d.codex), (1, 0), "{d:?}");
        assert!(d.text.contains("=== Claude Code · "), "{}", d.text);
        assert!(
            d.text
                .contains(" · app · gałąź feature/kasa · sesja c0ffee11 ==="),
            "{}",
            d.text
        );
        assert!(
            d.text
                .contains("1. Zrób tryb offline w aplikacji kasowej\n"),
            "{}",
            d.text
        );
        assert!(
            d.text.contains("2. Dodaj kolejkę transakcji\n"),
            "{}",
            d.text
        );
        assert!(d.text.contains("3. /review gałąź kasy\n"), "{}", d.text);
        for absent in [
            "wstrzyknięte",
            "ODPOWIEDŹ",
            "WYNIK",
            "task-id",
            "PODAGENT",
            "META",
            "STRESZCZENIE",
            "OD INNEGO",
            "wynik",
        ] {
            assert!(!d.text.contains(absent), "{absent}: {}", d.text);
        }
        assert_eq!(
            d.summary(14),
            format!(
                "Zebrano 1 sesję (Claude Code 1, Codex 0), {} KB.",
                d.text.len().div_ceil(1024)
            )
        );
    }

    #[test]
    fn headless_and_delegated_sessions_are_skipped() {
        let f = fx();
        let ts = "2026-11-12T10:00:00Z";
        write_claude(
            &f,
            &f.repo,
            "headless",
            &[claude_user(
                &f.repo,
                ts,
                Value::String("PROMPT APLIKACJI".into()),
                serde_json::json!({"entrypoint": "sdk-cli"}),
            )],
        );
        let day = today_dir();
        write_codex(
            &f,
            &day,
            "exec",
            &[
                codex_meta(
                    &f.repo,
                    serde_json::json!({"originator": "codex_exec", "source": "exec"}),
                ),
                codex_msg("user", "PROMPT EXEC"),
            ],
        );
        write_codex(
            &f,
            &day,
            "sub",
            &[
                codex_meta(
                    &f.repo,
                    serde_json::json!({"source": {"subagent": {"other": "guardian"}}}),
                ),
                codex_msg("user", "PROMPT PODAGENTA"),
            ],
        );
        write_codex(
            &f,
            &day,
            "child",
            &[
                codex_meta(&f.repo, serde_json::json!({"parent_thread_id": "x"})),
                codex_msg("user", "PROMPT DZIECKA"),
            ],
        );
        write_codex(
            &f,
            &day,
            "review",
            &[
                codex_meta(
                    &f.repo,
                    serde_json::json!({"thread_source": "guardian_review"}),
                ),
                codex_msg("user", "PROMPT RECENZJI"),
            ],
        );
        write_codex(&f, &day, "ok", &[
            codex_meta(&f.worktree.join("src"), serde_json::json!({"git": {"branch": "feature/kasa"}})),
            codex_msg("developer", "INSTRUKCJE"),
            codex_msg("user", "<environment_context>\n  <cwd>/x</cwd>\n</environment_context>"),
            codex_msg("user", "# AGENTS.md instructions for /x\n\n<INSTRUCTIONS>…</INSTRUCTIONS>"),
            codex_msg("user", "Podłącz drukarkę fiskalną"),
            serde_json::json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ODPOWIEDŹ"}]}}).to_string(),
        ]);
        let pl = places(&f);
        let d = collect(&req(&f, &pl, 14, Limits::default()));
        assert_eq!((d.claude, d.codex), (0, 1), "{}", d.text);
        assert!(
            d.text.contains("=== Codex · ")
                && d.text
                    .contains(" · app · gałąź feature/kasa · sesja 0a1b2c3d"),
            "{}",
            d.text
        );
        assert!(
            d.text.contains("1. Podłącz drukarkę fiskalną"),
            "{}",
            d.text
        );
        for absent in [
            "APLIKACJI",
            "EXEC",
            "PODAGENTA",
            "DZIECKA",
            "RECENZJI",
            "INSTRUKCJE",
            "environment_context",
            "AGENTS.md",
            "ODPOWIEDŹ",
        ] {
            assert!(!d.text.contains(absent), "{absent}: {}", d.text);
        }
    }

    #[test]
    fn directory_matching_window_symlinks_and_oversized_lines() {
        let f = fx();
        let ts = "2026-11-12T10:00:00Z";
        let base = f.repo.parent().unwrap().to_path_buf();
        // Inside the repo (subdirectory), inside the worktree, and two traps.
        write_claude(
            &f,
            &f.repo.join("src/ui"),
            "inside",
            &[claude_user(
                &f.repo.join("src/ui"),
                ts,
                Value::String("W REPO".into()),
                serde_json::json!({}),
            )],
        );
        write_claude(
            &f,
            &f.worktree,
            "wt",
            &[claude_user(
                &f.worktree,
                ts,
                Value::String("W WORKTREE".into()),
                serde_json::json!({}),
            )],
        );
        write_claude(
            &f,
            &base.join("repo-other"),
            "trap",
            &[claude_user(
                &base.join("repo-other"),
                ts,
                Value::String("PUŁAPKA PREFIKSU".into()),
                serde_json::json!({}),
            )],
        );
        write_claude(
            &f,
            &base,
            "outside",
            &[claude_user(
                &base,
                ts,
                Value::String("POZA".into()),
                serde_json::json!({}),
            )],
        );
        // Too old (mtime outside the window).
        let old = write_claude(
            &f,
            &f.repo,
            "old",
            &[claude_user(
                &f.repo,
                ts,
                Value::String("STARE".into()),
                serde_json::json!({}),
            )],
        );
        let t = SystemTime::now() - Duration::from_secs(20 * 86_400);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(t)
            .unwrap();
        // A symlinked session file is not followed.
        let target = base.join("poza.jsonl");
        std::fs::write(
            &target,
            claude_user(
                &f.repo,
                ts,
                Value::String("SYMLINK".into()),
                serde_json::json!({}),
            ) + "\n",
        )
        .unwrap();
        symlink(
            &target,
            f.roots.claude.join(claude_slug(&f.repo)).join("link.jsonl"),
        )
        .unwrap();
        // An oversized line (a request blob) is skipped without parsing; the
        // prompt after it still counts.
        let big = format!(
            r#"{{"type":"api-request-blob","data":"{}"}}"#,
            "x".repeat(400_000)
        );
        write_claude(
            &f,
            &f.repo,
            "big",
            &[
                big,
                claude_user(
                    &f.repo,
                    ts,
                    Value::String("PO DUŻEJ LINII".into()),
                    serde_json::json!({}),
                ),
            ],
        );
        let pl = places(&f);
        let d = collect(&req(&f, &pl, 14, Limits::default()));
        for want in ["W REPO", "W WORKTREE", "PO DUŻEJ LINII"] {
            assert!(d.text.contains(want), "{want}: {}", d.text);
        }
        for absent in ["PUŁAPKA", "POZA", "STARE", "SYMLINK"] {
            assert!(!d.text.contains(absent), "{absent}: {}", d.text);
        }
        assert_eq!(d.claude, 3);
        // A 30-day window includes the old one.
        let d30 = collect(&req(&f, &pl, 30, Limits::default()));
        assert!(d30.text.contains("STARE"));
        // No places: nothing is read.
        assert_eq!(
            collect(&req(&f, &[], 14, Limits::default())),
            Digest::default()
        );
        let empty = Digest::default();
        assert_eq!(
            empty.summary(7),
            "Brak sesji z ostatnich 7 dni w repozytoriach projektu."
        );
    }

    #[test]
    fn caps_and_omitted_counts() {
        let f = fx();
        // One session with many prompts: the first and the most recent are kept.
        let mut lines = Vec::new();
        for i in 0..20 {
            let text = if i == 0 {
                "Cel: ".to_string() + &"długi opis ".repeat(200)
            } else {
                format!("polecenie {i} ") + &"x".repeat(600)
            };
            lines.push(claude_user(
                &f.repo,
                &format!("2026-11-12T10:{i:02}:00Z"),
                Value::String(text),
                serde_json::json!({}),
            ));
        }
        write_claude(&f, &f.repo, "long", &lines);
        // Several more sessions.
        for s in 0..6 {
            write_claude(
                &f,
                &f.repo,
                &format!("s{s}"),
                &[claude_user(
                    &f.repo,
                    &format!("2026-11-1{s}T08:00:00Z"),
                    Value::String(format!("sesja numer {s}")),
                    serde_json::json!({}),
                )],
            );
        }
        let pl = places(&f);
        let limits = Limits {
            prompts_per_session: 4,
            max_sessions: 5,
            ..Limits::default()
        };
        let d = collect(&req(&f, &pl, 14, limits));
        assert_eq!(d.sessions(), 5, "{}", d.text);
        assert_eq!(d.omitted_sessions, 2, "{}", d.text);
        assert_eq!(d.omitted_prompts, 16);
        assert!(
            d.text.contains("(pominięto 16 kolejnych poleceń)"),
            "{}",
            d.text
        );
        assert!(
            d.text.contains("polecenie 19") && !d.text.contains("polecenie 5 "),
            "{}",
            d.text
        );
        assert!(
            d.text
                .contains("(Pominięto: 2 starsze sesje, 16 poleceń w środku długich sesji.)")
        );
        // The first prompt is cut at 1200 characters, later ones at 400.
        let first = d.text.lines().find(|l| l.starts_with("1. Cel:")).unwrap();
        assert_eq!(first.chars().count(), 3 + 1200 + 1, "{first}");
        let later = d.text.lines().find(|l| l.contains("polecenie 19")).unwrap();
        assert_eq!(later.chars().count(), 3 + 400 + 1);
        // Total cap.
        let small = Limits {
            total_bytes: 1200,
            ..Limits::default()
        };
        let d = collect(&req(&f, &pl, 14, small));
        assert!(d.text.len() <= 1200 + 200, "{}", d.text.len());
        assert!(d.omitted_sessions > 0);
        // File cap and the time budget leave a note.
        let few = Limits {
            max_files: 2,
            ..Limits::default()
        };
        let d = collect(&req(&f, &pl, 14, few));
        assert!(
            d.notes
                .iter()
                .any(|n| n.contains("przejrzano 2 najnowszych plików")),
            "{:?}",
            d.notes
        );
        let mut r = req(&f, &pl, 14, Limits::default());
        r.budget = Duration::ZERO;
        let d = collect(&r);
        assert_eq!(d.sessions(), 0);
        assert!(
            d.notes
                .iter()
                .any(|n| n.contains("Przerwano zbieranie historii")),
            "{:?}",
            d.notes
        );
    }

    #[test]
    fn secrets_and_home_are_redacted_in_the_digest() {
        let f = fx();
        write_claude(&f, &f.repo, "sec", &[claude_user(
            &f.repo,
            "2026-11-12T10:00:00Z",
            Value::String("użyj klucza sk-proj-ABCDEFGHIJKLMNOPQRSTUVWX12 i napisz do jan@example.org w /Users/ala/Projects".into()),
            serde_json::json!({}),
        )]);
        let pl = places(&f);
        let d = collect(&req(&f, &pl, 14, Limits::default()));
        assert!(
            d.text.contains(SECRET) && d.text.contains(EMAIL) && d.text.contains("~/Projects"),
            "{}",
            d.text
        );
        assert!(!d.text.contains("sk-proj") && !d.text.contains("jan@"));
    }
}
