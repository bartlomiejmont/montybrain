//! The output contract of the `aktualizacja` recipe and its validation.
//!
//! A background `aktualizacja` run is read-only: the agent writes a short
//! report and ends its answer with exactly one fenced block whose info string
//! is [`FENCE_INFO`], holding JSON:
//!
//! ```text
//! {"changes": [
//!   {"kind": "new_topic", "title": "<cel>", "status": "in_progress", "summary": "<gdzie jesteśmy>",
//!    "repo": "<repozytorium>", "text": "<dowody>", "tasks": [{"title": "<…>", "status": "todo"}],
//!    "reason": "<skąd>"},
//!   {"kind": "task_status", "task": "<id>", "status": "done", "reason": "<dowód>"},
//!   {"kind": "topic_log", "topic": "<slug>", "text": "<wpis>", "status": "waiting", "summary": "<opis>"},
//!   {"kind": "new_task", "title": "<tytuł>", "topic": "<slug>", "reason": "<skąd>"}
//! ]}
//! ```
//!
//! The app, not the agent, applies accepted changes through `montybrain-core`.
//! Everything in the answer is untrusted:
//!
//! * a missing, duplicated, unclosed, oversized or malformed block is "no
//!   usable proposal" with a Polish message ([`Proposal::problem`]), never a
//!   panic;
//! * every item is checked on its own (unknown fields, kinds and statuses,
//!   caps on count and text lengths, ids and topics that must exist **in this
//!   project**, tasks that must be open, the same free-text rules core applies
//!   before writing, see [`montybrain_core::check`]); a bad item is dropped with
//!   a visible reason ([`Dropped`]) instead of failing the whole proposal;
//! * each kept [`Change`] records the state it was based on, so applying it
//!   later can detect that its target changed in the meantime ([`check_current`]).

use montybrain_core::{
    Snapshot, Task, TaskLocation, TaskStatus, Topic, TopicStatus, check, fold, is_valid_id, slugify,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The info string of the fenced block with the proposed changes.
pub const FENCE_INFO: &str = "montybrain-zmiany";
/// The pre-rename info string, still accepted (prompts ask for [`FENCE_INFO`]).
pub const LEGACY_FENCE_INFO: &str = "morwisko-zmiany";
/// The proposals of the Skrzynka agent are stored under this "project" key:
/// their changes are about inbox entries and may point at any project.
pub const INBOX: &str = "";
/// Entries in one "topic from entries" change.
pub const MAX_TOPIC_ENTRIES: usize = 20;
/// Changes per proposal; further items are dropped.
pub const MAX_CHANGES: usize = 20;
/// A larger block is not parsed at all.
pub const MAX_BLOCK_BYTES: usize = 128 * 1024;
/// New topics per proposal (the agent is asked for fewer, larger ones).
pub const MAX_NEW_TOPICS: usize = 8;
/// Initial tasks of one new topic.
pub const MAX_TOPIC_TASKS: usize = 6;
/// New tasks outside any topic per proposal.
pub const MAX_LOOSE_TASKS: usize = 3;
pub const MAX_REASON_CHARS: usize = 600;
pub const MAX_TITLE_CHARS: usize = 200;
pub const MAX_LOG_CHARS: usize = 2_000;
pub const MAX_SUMMARY_CHARS: usize = 1_500;
/// Dropped items listed per proposal (the rest are counted only).
const MAX_DROPPED: usize = 40;

/// The message of a change whose target changed after the proposal was made.
pub const STALE: &str = "zmienione w międzyczasie";

/// One validated change, with the state it was based on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    RemoveTask {
        task: String,
        title: String,
        reason: String,
        /// Full base: do not remove a task that was edited or moved.
        base: Task,
    },
    TaskStatus {
        task: String,
        /// For display.
        title: String,
        /// The task's status when the proposal was made (the base).
        from: TaskStatus,
        to: TaskStatus,
        reason: String,
    },
    TopicLog {
        topic: String,
        /// For display.
        topic_title: String,
        /// As core will store it.
        text: String,
        /// `None` when not given or equal to the base.
        status: Option<TopicStatus>,
        summary: Option<String>,
        /// The topic's status and summary when the proposal was made.
        base_status: TopicStatus,
        base_summary: String,
        /// Branches the agent says belong to the topic (stored as links on apply).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        branches: Vec<BranchRef>,
    },
    NewTask {
        title: String,
        /// `None`: project level.
        topic: Option<String>,
        topic_title: Option<String>,
        reason: String,
    },
    /// A new topic (an "epic"): one stream of work, with its first journal
    /// entry and its first tasks, written as one file.
    NewTopic {
        title: String,
        /// The slug it will get (`slugify(title)`); the base of the stale check.
        slug: String,
        status: TopicStatus,
        summary: String,
        /// The main repository (a repository name of the project).
        repo: Option<String>,
        /// The first journal entry (the evidence).
        text: String,
        tasks: Vec<TopicTask>,
        reason: String,
        /// Branches of the topic (stored as links on apply).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        branches: Vec<BranchRef>,
    },
    /// Skrzynka: move an inbox entry (id, links and notes kept).
    MoveTask {
        task: String,
        title: String,
        /// Where it was when proposed (the base).
        from: TaskLocation,
        to: TaskLocation,
        /// "WhenUBuy / Checkout".
        to_label: String,
        reason: String,
    },
    /// Skrzynka: one new topic in `project` made of inbox entries.
    TopicFromTasks {
        project: String,
        project_name: String,
        title: String,
        /// The slug it will get; the base of the stale check.
        slug: String,
        summary: String,
        tasks: Vec<EntryRef>,
        reason: String,
    },
}

/// An inbox entry named by a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryRef {
    pub id: String,
    pub title: String,
    /// Where it was when proposed.
    pub from: TaskLocation,
}

/// A branch of one of the project's repositories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchRef {
    pub repo: String,
    pub branch: String,
}

/// At most this many branches per change.
pub const MAX_BRANCHES: usize = 6;

/// A plausible git branch name (no option-like or path-escaping forms).
pub fn valid_branch_name(b: &str) -> bool {
    !b.is_empty()
        && b.chars().count() <= 200
        && !b.starts_with(['-', '/', '.'])
        && !b.ends_with(['/', '.'])
        && !b.ends_with(".lock")
        && !b.contains("..")
        && !b.contains("//")
        && b.chars()
            .all(|c| c.is_alphanumeric() || "/_.-+@#".contains(c))
}

/// The optional `branches` of an item: each `{repo, branch}` with a repository
/// of the project and a plausible branch name.
fn parse_branches(
    v: Option<&Value>,
    snap: &Snapshot,
    project: &str,
) -> Result<Vec<BranchRef>, String> {
    let items = match v {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(a)) => a,
        Some(_) => return Err("Gałęzie: zły format.".into()),
    };
    let p = snap.project(project).ok_or("Nie ma takiego projektu.")?;
    let mut out: Vec<BranchRef> = Vec::new();
    // More than the limit: the first ones are kept.
    for item in items.iter().take(MAX_BRANCHES) {
        let o = item.as_object().ok_or("Gałąź ma zły format.")?;
        let f = Fields { obj: o };
        f.only(&["repo", "branch"])?;
        let repo = f.str("repo")?.trim();
        let branch = f.str("branch")?.trim();
        let branch = branch.strip_prefix("origin/").unwrap_or(branch);
        let known = p
            .repos
            .iter()
            .find(|r| r.name == repo)
            .ok_or("Gałąź: projekt nie ma repozytorium o tej nazwie.")?;
        if !valid_branch_name(branch) {
            return Err(format!(
                "Nieprawidłowa nazwa gałęzi „{}”.",
                clip_label(branch, 40)
            ));
        }
        let b = BranchRef {
            repo: known.name.clone(),
            branch: branch.to_string(),
        };
        if !out.contains(&b) {
            out.push(b);
        }
    }
    Ok(out)
}

/// An initial task of a proposed new topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicTask {
    pub title: String,
    pub status: TaskStatus,
}

impl Change {
    pub fn location(&self, project: &str) -> TaskLocation {
        if project == INBOX && matches!(self, Change::NewTask { .. } | Change::TaskStatus { .. }) {
            return TaskLocation::Inbox;
        }
        match self {
            Change::MoveTask { to, .. } => to.clone(),
            Change::TopicFromTasks { project, slug, .. } => TaskLocation::Topic {
                project: project.clone(),
                topic: slug.clone(),
            },
            Change::NewTask {
                topic: Some(topic), ..
            } => TaskLocation::Topic {
                project: project.into(),
                topic: topic.clone(),
            },
            Change::NewTask { topic: None, .. } => TaskLocation::Project {
                project: project.into(),
            },
            Change::TopicLog { topic, .. } => TaskLocation::Topic {
                project: project.into(),
                topic: topic.clone(),
            },
            Change::NewTopic { slug, .. } => TaskLocation::Topic {
                project: project.into(),
                topic: slug.clone(),
            },
            Change::TaskStatus { .. } | Change::RemoveTask { .. } => TaskLocation::Project {
                project: project.into(),
            },
        }
    }

    fn is_loose_task(&self) -> bool {
        matches!(self, Change::NewTask { topic: None, .. })
    }
}

/// An item of the block that was not kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dropped {
    /// Position in the block's `changes` array (0-based).
    pub index: usize,
    /// What the item seemed to be about (kind, id, title…), for display.
    pub label: String,
    pub reason: String,
}

/// The parsed answer of an `aktualizacja` run.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Proposal {
    /// The answer without the block (Markdown, for display).
    pub report: String,
    pub changes: Vec<Change>,
    pub dropped: Vec<Dropped>,
    /// Dropped items beyond the listed ones.
    pub dropped_more: usize,
    /// Set when there is no usable proposal (block missing, duplicated,
    /// malformed…); `changes` is empty then.
    pub problem: Option<String>,
}

pub fn same_task(a: &Task, b: &Task) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    a.line = 0;
    b.line = 0;
    a == b
}

/// Journal refresh proposals cannot escape the topic.
/// Cross-project changes remain forbidden by the shared parser.
pub fn parse_journal(
    answer: &str,
    snap: &Snapshot,
    project: &str,
    topic: &str,
    note: &str,
) -> Proposal {
    parse_scoped(answer, snap, project, Some((topic, note)))
}

fn journal_change(
    change: Change,
    snap: &Snapshot,
    _project: &str,
    topic: &str,
    note: &str,
) -> Result<Change, String> {
    let target = match &change {
        Change::TaskStatus { task, .. } | Change::RemoveTask { task, .. } => {
            snap.task(task).and_then(|t| match &t.location {
                TaskLocation::Topic { topic, .. } => Some(topic.as_str()),
                _ => None,
            })
        }
        Change::TopicLog { topic, .. } => Some(topic.as_str()),
        Change::NewTask { topic, .. } => topic.as_deref(),
        _ => None,
    };
    if matches!(&change, Change::TopicLog { text, status, summary, .. } if (status.is_none() && summary.is_none()) || text.trim() == note.trim())
    {
        return Err("Wpis jest już zapisany; agent ma proponować zmiany statusu lub opisu bez powtarzania notatki.".into());
    }
    if !target.is_some_and(|t| t == topic) {
        return Err(
            "Odświeżenie dotyczy wyłącznie tego tematu; pominięto zmianę poza zakresem.".into(),
        );
    }
    Ok(change)
}

// --- extraction -----------------------------------------------------------------

/// A fence opening/closing line: the fence char, its length and the info string.
fn fence_line(line: &str) -> Option<(char, usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let c = rest.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    let n = rest.chars().take_while(|&x| x == c).count();
    if n < 3 {
        return None;
    }
    let info = rest[n..].trim();
    // A backtick fence's info string cannot contain backticks (CommonMark).
    if c == '`' && info.contains('`') {
        return None;
    }
    Some((c, n, info))
}

/// The single command of a `skrzynka-komenda` answer: the first non-empty
/// line inside the first block with info string `info`, and the text outside
/// the blocks (the agent's one-sentence reason).
pub fn command_block(answer: &str, info: &str) -> (String, Option<String>) {
    let lines: Vec<&str> = answer
        .split('\n')
        .map(|l| l.trim_end_matches('\r'))
        .collect();
    let mut outside: Vec<&str> = Vec::new();
    let mut found: Option<String> = None;
    let mut i = 0;
    while i < lines.len() {
        match fence_line(lines[i]) {
            Some((c, n, inf)) => {
                let close = (i + 1..lines.len()).find(|&j| {
                    fence_line(lines[j])
                        .is_some_and(|(c2, n2, i2)| c2 == c && n2 >= n && i2.is_empty())
                });
                let end = close.unwrap_or(lines.len());
                if inf == info && found.is_none() {
                    found = lines[i + 1..end]
                        .iter()
                        .map(|l| l.trim())
                        .find(|l| !l.is_empty())
                        .map(str::to_string);
                } else if inf != info {
                    outside.extend_from_slice(&lines[i..end.min(lines.len())]);
                }
                i = end + 1;
            }
            None => {
                outside.push(lines[i]);
                i += 1;
            }
        }
    }
    (outside.join("\n").trim().to_string(), found)
}

/// The answer split into the report and the proposal block(s).
struct Split {
    report: String,
    blocks: Vec<String>,
    unclosed: bool,
}

fn split(answer: &str) -> Split {
    let lines: Vec<&str> = answer
        .split('\n')
        .map(|l| l.trim_end_matches('\r'))
        .collect();
    let mut report: Vec<&str> = Vec::new();
    let mut blocks = Vec::new();
    let mut unclosed = false;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let Some((c, n, info)) = fence_line(line) else {
            report.push(line);
            i += 1;
            continue;
        };
        // Find the closing fence: same char, at least as long, no info.
        let close = (i + 1..lines.len()).find(|&j| {
            fence_line(lines[j])
                .is_some_and(|(c2, n2, info2)| c2 == c && n2 >= n && info2.is_empty())
        });
        let ours = info == FENCE_INFO || info == LEGACY_FENCE_INFO;
        match close {
            Some(j) => {
                if ours {
                    blocks.push(lines[i + 1..j].join("\n"));
                } else {
                    report.extend_from_slice(&lines[i..=j]);
                }
                i = j + 1;
            }
            None => {
                if ours {
                    unclosed = true;
                } else {
                    report.extend_from_slice(&lines[i..]);
                }
                break;
            }
        }
    }
    Split {
        report: report.join("\n").trim().to_string(),
        blocks,
        unclosed,
    }
}

// --- validation -------------------------------------------------------------------

fn chars(s: &str) -> usize {
    s.chars().count()
}

/// The Polish name of a field, for messages ("Uzasadnienie zawiera…").
fn field_name(field: &str) -> &str {
    match field {
        "reason" => "Uzasadnienie",
        "title" => "Tytuł",
        "text" => "Wpis do dziennika",
        "summary" => "Opis tematu",
        "task" => "Zadanie",
        "topic" => "Temat",
        "status" => "Status",
        "kind" => "Rodzaj zmiany",
        other => other,
    }
}

/// A short single-line text shown to the user only (never written to the
/// vault): whitespace collapsed, no control characters, not empty, capped.
fn display_line(field: &str, s: &str, max: usize) -> Result<String, String> {
    // Control characters are dropped (display only), not a reason to drop the change.
    let s: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.is_empty() {
        return Err(format!("{}: puste.", field_name(field)));
    }
    // Display-only text: a long one is shortened, never a reason to drop the
    // whole change.
    if chars(&s) > max {
        let mut cut: String = s.chars().take(max - 1).collect();
        cut.push('…');
        return Ok(cut);
    }
    Ok(s)
}

/// When an item has no usable `reason` (display only).
pub const NO_REASON: &str = "(agent nie podał uzasadnienia)";

/// The `reason` of an item: optional and display only, so a missing, empty or
/// malformed one is replaced, a long one shortened.
fn reason_of(f: &Fields) -> String {
    match f.obj.get("reason") {
        Some(Value::String(s)) => {
            display_line("reason", s, MAX_REASON_CHARS).unwrap_or_else(|_| NO_REASON.into())
        }
        _ => NO_REASON.into(),
    }
}

/// Text that is written (a title, a summary, a journal entry) but harmless
/// when shortened: cut at `max` characters with "…" instead of dropping the
/// change (the user sees it in the review and can edit it). Structure checks
/// (core `check::*`) still run on the result.
fn shortened(s: &str, max: usize) -> String {
    if chars(s) <= max {
        return s.to_string();
    }
    let mut cut: String = s
        .chars()
        .take(max - 1)
        .collect::<String>()
        .trim_end()
        .to_string();
    cut.push('…');
    cut
}

fn capped(field: &str, s: &str, max: usize) -> Result<(), String> {
    let n = chars(s);
    if n > max {
        return Err(format!(
            "{}: za długie ({n} znaków, limit {max}).",
            field_name(field)
        ));
    }
    Ok(())
}

/// Core's rejection message, shortened for the UI.
fn core_rejects(field: &str, e: montybrain_core::Error) -> String {
    let msg = e.to_string();
    let msg = msg.strip_prefix("invalid input: ").unwrap_or(&msg);
    format!(
        "{} nie może zostać zapisany w tej postaci: naruszałby format pliku MontyBrain ({msg}).",
        field_name(field)
    )
}

struct Fields<'a> {
    obj: &'a Map<String, Value>,
}

impl<'a> Fields<'a> {
    /// Fields beyond `allowed` are ignored (nothing reads them, nothing writes
    /// them): an extra field is no reason to drop a change.
    fn only(&self, allowed: &[&str]) -> Result<(), String> {
        let _ = allowed;
        Ok(())
    }

    fn str_opt(&self, key: &str) -> Result<Option<&'a str>, String> {
        match self.obj.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.as_str())),
            Some(_) => Err(format!("{}: zły format.", field_name(key))),
        }
    }

    fn str(&self, key: &str) -> Result<&'a str, String> {
        self.str_opt(key)?
            .ok_or_else(|| format!("Brakuje pola „{key}”."))
    }
}

fn clip_label(s: &str, max: usize) -> String {
    let one: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let one = one.split_whitespace().collect::<Vec<_>>().join(" ");
    if chars(&one) <= max {
        one
    } else {
        format!("{}…", one.chars().take(max).collect::<String>())
    }
}

/// What a raw item seems to be about, for the dropped list: names the task or
/// topic of this project when it exists, never an id or a slug.
fn label_of(v: &Value, snap: &Snapshot, project: &str) -> String {
    let get = |k: &str| v.get(k).and_then(Value::as_str);
    match get("kind") {
        Some("remove_task") => get("task")
            .and_then(|id| snap.task(id.trim().trim_start_matches('^')))
            .filter(|t| task_in_project(t, project))
            .map_or_else(
                || "Usunięcie zadania".into(),
                |t| format!("Usunięcie zadania „{}”", clip_label(&t.title, 60)),
            ),
        Some("task_status") => get("task")
            .map(|t| t.trim().trim_start_matches('^'))
            .and_then(|id| snap.task(id))
            .filter(|t| task_in_project(t, project))
            .map_or_else(
                || "Status zadania".into(),
                |t| format!("Status zadania „{}”", clip_label(&t.title, 60)),
            ),
        Some("topic_log") => get("topic")
            .and_then(|slug| topic_of(snap, project, slug.trim()))
            .map_or_else(
                || "Wpis w dzienniku".into(),
                |t| format!("Wpis w dzienniku „{}”", clip_label(&t.title, 60)),
            ),
        Some("new_task") => format!(
            "Nowe zadanie {}",
            get("title").map_or_else(|| "?".into(), |t| format!("„{}”", clip_label(t, 60)))
        ),
        Some("new_topic") => format!(
            "Nowy temat {}",
            get("title").map_or_else(|| "?".into(), |t| format!("„{}”", clip_label(t, 60)))
        ),
        Some("move_task") => get("task")
            .map(|t| t.trim().trim_start_matches('^'))
            .and_then(|id| snap.task(id))
            .map_or_else(
                || "Przeniesienie wpisu".into(),
                |t| format!("Przeniesienie „{}”", clip_label(&t.title, 60)),
            ),
        Some("topic_from_tasks") => format!(
            "Temat z wpisów {}",
            get("title").map_or_else(|| "?".into(), |t| format!("„{}”", clip_label(t, 60)))
        ),
        Some(k) => format!("Zmiana „{}”", clip_label(k, 30)),
        None => "Zmiana bez rodzaju".into(),
    }
}

fn open_status(s: TaskStatus) -> bool {
    s != TaskStatus::Done
}

fn task_in_project(t: &Task, project: &str) -> bool {
    if project == INBOX {
        return t.location == TaskLocation::Inbox;
    }
    t.location.project() == Some(project)
}

/// An open inbox entry with this id.
fn inbox_entry<'s>(snap: &'s Snapshot, raw: &str) -> Result<&'s Task, String> {
    let id = raw.trim().trim_start_matches('^');
    if !is_valid_id(id) {
        return Err("Nieprawidłowy identyfikator zadania.".into());
    }
    let t = snap.task(id).ok_or("Nie ma takiego wpisu.")?;
    if t.location != TaskLocation::Inbox {
        return Err("Tego wpisu nie ma już w Skrzynce.".into());
    }
    if !open_status(t.status) {
        return Err("Wpis jest już zrobiony.".into());
    }
    Ok(t)
}

fn topic_of<'s>(snap: &'s Snapshot, project: &str, slug: &str) -> Option<&'s Topic> {
    snap.topics
        .iter()
        .find(|t| t.project == project && t.slug == slug)
}

/// A task with this title (case and diacritics ignored) at `loc`.
fn same_title_at<'s>(snap: &'s Snapshot, loc: &TaskLocation, title: &str) -> Option<&'s Task> {
    let want = fold(title);
    snap.tasks
        .iter()
        .find(|t| &t.location == loc && fold(&t.title) == want)
}

/// A status word as agents write it: case, `-`/`_`/spaces and Polish forms.
fn status_key(v: &str) -> String {
    fold(v).chars().filter(|c| c.is_alphanumeric()).collect()
}

fn parse_task_status(v: &str) -> Result<TaskStatus, String> {
    match status_key(v).as_str() {
        "todo" | "open" | "dozrobienia" | "otwarte" | "nowe" => return Ok(TaskStatus::Todo),
        "doing" | "inprogress" | "wtoku" | "wtrakcie" | "started" => return Ok(TaskStatus::Doing),
        "done" | "completed" | "complete" | "closed" | "zrobione" | "zrobiony" | "gotowe" => {
            return Ok(TaskStatus::Done);
        }
        "blocked" | "zablokowane" | "zablokowany" | "waiting" | "czeka" => {
            return Ok(TaskStatus::Blocked);
        }
        _ => {}
    }
    serde_json::from_value(Value::String(v.into())).map_err(|_| {
        format!(
            "Agent podał nieznany status zadania „{}”.",
            clip_label(v, 20)
        )
    })
}

fn parse_topic_status(v: &str) -> Result<TopicStatus, String> {
    match status_key(v).as_str() {
        "planned" | "planowany" | "todo" | "plan" => return Ok(TopicStatus::Planned),
        "inprogress" | "wtoku" | "doing" | "active" | "aktywny" => {
            return Ok(TopicStatus::InProgress);
        }
        "blocked" | "zablokowany" => return Ok(TopicStatus::Blocked),
        "waiting" | "czeka" | "oczekuje" => return Ok(TopicStatus::Waiting),
        "done" | "zrobiony" | "zrobione" | "completed" | "closed" => return Ok(TopicStatus::Done),
        _ => {}
    }
    serde_json::from_value(Value::String(v.into())).map_err(|_| {
        format!(
            "Agent podał nieznany status tematu „{}”.",
            clip_label(v, 20)
        )
    })
}

/// Validates one raw item against the snapshot the run started from.
fn validate_item(v: &Value, snap: &Snapshot, project: &str) -> Result<Change, String> {
    validate_item_as(v, snap, project, true)
}

/// [`validate_item`]; `lenient` (agent answers): repairable text is
/// shortened or normalised instead of dropping the change. The user's own
/// corrections are checked strictly (they are told what is wrong).
fn validate_item_as(
    v: &Value,
    snap: &Snapshot,
    project: &str,
    lenient: bool,
) -> Result<Change, String> {
    let fit = |field: &str, s: &str, max: usize| -> Result<String, String> {
        if lenient {
            Ok(shortened(s, max))
        } else {
            capped(field, s, max).map(|()| s.to_string())
        }
    };
    let title_fit = |s: &str| -> Result<String, String> {
        let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
        fit("title", &one, MAX_TITLE_CHARS)
    };
    let obj = v.as_object().ok_or("Zmiana ma zły format.")?;
    let f = Fields { obj };
    let kind = f.str("kind")?;
    let inbox = project == INBOX;
    match (inbox, kind) {
        (false, "move_task" | "topic_from_tasks") => {
            return Err("Ten rodzaj zmiany jest tylko dla Skrzynki.".into());
        }
        (true, "topic_log" | "new_topic") => {
            return Err(
                "W Skrzynce agent proponuje przeniesienia, tematy z wpisów i nowe zadania.".into(),
            );
        }
        _ => {}
    }
    match kind {
        "remove_task" => {
            if inbox {
                return Err("Usuwanie zadań jest dostępne tylko w projekcie.".into());
            }
            let id = f.str("task")?.trim().trim_start_matches('^');
            if !is_valid_id(id) {
                return Err("Nieprawidłowy identyfikator zadania.".into());
            }
            let task = snap
                .task(id)
                .ok_or("Nie ma już takiego zadania w projekcie.")?;
            if !task_in_project(task, project) {
                return Err("Zadanie nie należy do tego projektu.".into());
            }
            Ok(Change::RemoveTask {
                task: id.into(),
                title: task.title.clone(),
                reason: reason_of(&f),
                base: task.clone(),
            })
        }
        "move_task" => {
            f.only(&["kind", "task", "to", "reason"])?;
            let t = inbox_entry(snap, f.str("task")?)?;
            let reason = reason_of(&f);
            let to_raw = f.str("to")?.trim();
            use montybrain_core::command::resolve::{Resolved, resolve_place};
            let place = match resolve_place(snap, to_raw) {
                Resolved::One(p) => p,
                Resolved::Typo(p, _) => {
                    return Err(format!(
                        "Nie ma miejsca „{}” (może {}?).",
                        clip_label(to_raw, 40),
                        p.text
                    ));
                }
                Resolved::Ambiguous(_) => {
                    return Err(format!(
                        "„{}” pasuje do kilku miejsc.",
                        clip_label(to_raw, 40)
                    ));
                }
                Resolved::Unknown(_) => {
                    return Err(format!("Nie ma miejsca „{}”.", clip_label(to_raw, 40)));
                }
            };
            if place.location == TaskLocation::Inbox {
                return Err("Wpis jest już w Skrzynce.".into());
            }
            Ok(Change::MoveTask {
                task: t.id.clone().unwrap_or_default(),
                title: t.title.clone(),
                from: t.location.clone(),
                to: place.location,
                to_label: place.label,
                reason,
            })
        }
        "topic_from_tasks" => {
            f.only(&["kind", "project", "title", "summary", "tasks", "reason"])?;
            let slug_in = f.str("project")?.trim();
            let p = snap
                .project(slug_in)
                .ok_or_else(|| format!("Nie ma projektu „{}”.", clip_label(slug_in, 40)))?;
            let raw_title = title_fit(f.str("title")?)?;
            let title = check::topic_title(&raw_title).map_err(|e| core_rejects("title", e))?;
            let slug = slugify(&title);
            let summary = match f.str_opt("summary")? {
                Some(s) => check::summary(&fit("summary", s, MAX_SUMMARY_CHARS)?)
                    .map_err(|e| core_rejects("summary", e))?,
                None => String::new(),
            };
            let reason = reason_of(&f);
            let ids = match obj.get("tasks") {
                Some(Value::Array(a)) if !a.is_empty() => a,
                _ => {
                    return Err(
                        "Temat z wpisów potrzebuje listy „tasks” z identyfikatorami.".into(),
                    );
                }
            };
            if ids.len() > MAX_TOPIC_ENTRIES {
                return Err(format!(
                    "Za dużo wpisów ({}, limit {MAX_TOPIC_ENTRIES}).",
                    ids.len()
                ));
            }
            let mut tasks: Vec<EntryRef> = Vec::new();
            for v in ids {
                let raw = v.as_str().ok_or("Wpis tematu ma zły format.")?;
                let t = inbox_entry(snap, raw)?;
                let id = t.id.clone().unwrap_or_default();
                // A repeated entry is counted once.
                if tasks.iter().any(|e| e.id == id) {
                    continue;
                }
                tasks.push(EntryRef {
                    id,
                    title: t.title.clone(),
                    from: t.location.clone(),
                });
            }
            if topic_exists(snap, &p.slug, &slug, &title) {
                return Err(TOPIC_EXISTS.into());
            }
            Ok(Change::TopicFromTasks {
                project: p.slug.clone(),
                project_name: p.name.clone(),
                title,
                slug,
                summary,
                tasks,
                reason,
            })
        }
        "task_status" => {
            f.only(&["kind", "task", "status", "reason"])?;
            let id = f.str("task")?.trim().trim_start_matches('^');
            if !is_valid_id(id) {
                return Err("Nieprawidłowy identyfikator zadania.".into());
            }
            let to = parse_task_status(f.str("status")?.trim())?;
            let reason = reason_of(&f);
            let task = snap
                .task(id)
                .ok_or("Nie ma już takiego zadania w projekcie.")?;
            if !task_in_project(task, project) {
                return Err(if inbox {
                    "Tego zadania nie ma w Skrzynce.".into()
                } else {
                    "Zadanie nie należy do tego projektu.".into()
                });
            }
            if !open_status(task.status) {
                return Err("Zadanie jest już zrobione.".into());
            }
            if task.status == to {
                return Err("Zadanie ma już ten status.".into());
            }
            Ok(Change::TaskStatus {
                task: id.to_string(),
                title: task.title.clone(),
                from: task.status,
                to,
                reason,
            })
        }
        "topic_log" => {
            f.only(&["kind", "topic", "text", "status", "summary", "branches"])?;
            let branches = parse_branches(obj.get("branches"), snap, project)?;
            let slug = f.str("topic")?.trim();
            let text = fit("text", f.str("text")?, MAX_LOG_CHARS)?;
            let text = check::journal_text(&text).map_err(|e| core_rejects("text", e))?;
            let status = f
                .str_opt("status")?
                .map(|s| parse_topic_status(s.trim()))
                .transpose()?;
            let summary = match f.str_opt("summary")? {
                Some(s) => Some(
                    check::summary(&fit("summary", s, MAX_SUMMARY_CHARS)?)
                        .map_err(|e| core_rejects("summary", e))?,
                ),
                None => None,
            };
            let topic = topic_of(snap, project, slug)
                .ok_or_else(|| "W tym projekcie nie ma takiego tematu.".to_string())?;
            Ok(Change::TopicLog {
                topic: topic.slug.clone(),
                topic_title: topic.title.clone(),
                text,
                status: status.filter(|s| *s != topic.status),
                summary: summary.filter(|s| *s != topic.summary),
                base_status: topic.status,
                base_summary: topic.summary.clone(),
                branches,
            })
        }
        "new_task" => {
            f.only(&["kind", "title", "topic", "reason"])?;
            let raw_title = title_fit(f.str("title")?)?;
            let title = check::task_title(&raw_title).map_err(|e| core_rejects("title", e))?;
            let reason = reason_of(&f);
            let topic = match f.str_opt("topic")?.map(str::trim).filter(|s| !s.is_empty()) {
                Some(_) if inbox => {
                    return Err(
                        "Nowe zadanie ze Skrzynki trafia do Skrzynki: bez pola „topic”.".into(),
                    );
                }
                Some(slug) => Some(
                    topic_of(snap, project, slug)
                        .ok_or_else(|| "W tym projekcie nie ma takiego tematu.".to_string())?,
                ),
                None => None,
            };
            let change = Change::NewTask {
                title,
                topic: topic.map(|t| t.slug.clone()),
                topic_title: topic.map(|t| t.title.clone()),
                reason,
            };
            if let Change::NewTask { title, .. } = &change
                && same_title_at(snap, &change.location(project), title).is_some()
            {
                return Err("Takie zadanie już istnieje w tym miejscu.".into());
            }
            Ok(change)
        }
        "new_topic" => {
            f.only(&[
                "kind", "title", "status", "summary", "repo", "text", "tasks", "reason", "branches",
            ])?;
            let branches = parse_branches(obj.get("branches"), snap, project)?;
            let raw_title = title_fit(f.str("title")?)?;
            let title = check::topic_title(&raw_title).map_err(|e| core_rejects("title", e))?;
            let slug = slugify(&title);
            if slug.is_empty() {
                return Err("Z tego tytułu nie da się utworzyć nazwy pliku tematu.".into());
            }
            // A missing status is "planned", like a new topic in the app.
            let status = match f
                .str_opt("status")?
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(st) => parse_topic_status(st)?,
                None => TopicStatus::Planned,
            };
            let raw_summary = fit("summary", f.str("summary")?, MAX_SUMMARY_CHARS)?;
            let summary = check::summary(&raw_summary).map_err(|e| core_rejects("summary", e))?;
            let text = fit("text", f.str("text")?, MAX_LOG_CHARS)?;
            let text = check::journal_text(&text).map_err(|e| core_rejects("text", e))?;
            let reason = reason_of(&f);
            let repo = match f.str_opt("repo")?.map(str::trim).filter(|r| !r.is_empty()) {
                Some(r) => {
                    let p = snap.project(project).ok_or("Nie ma takiego projektu.")?;
                    let known = p
                        .repos
                        .iter()
                        .find(|x| x.name == r)
                        .ok_or("Projekt nie ma repozytorium o tej nazwie.")?;
                    Some(known.name.clone())
                }
                None => None,
            };
            let tasks = match obj.get("tasks") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(items)) => {
                    if !lenient && items.len() > MAX_TOPIC_TASKS {
                        return Err(format!(
                            "Za dużo zadań w temacie ({}, limit {MAX_TOPIC_TASKS}).",
                            items.len()
                        ));
                    }
                    // More than the limit (agent text): the first ones are kept.
                    let mut out: Vec<TopicTask> = Vec::new();
                    for item in items {
                        if out.len() >= MAX_TOPIC_TASKS {
                            break;
                        }
                        // A bare string is a title.
                        let (raw, st) = match item {
                            Value::String(s) => (s.as_str(), None),
                            Value::Object(t) => {
                                let tf = Fields { obj: t };
                                (tf.str("title")?, tf.str_opt("status")?)
                            }
                            _ => return Err("Zadanie tematu ma zły format.".into()),
                        };
                        let t_title = check::task_title(&title_fit(raw)?)
                            .map_err(|e| core_rejects("title", e))?;
                        let t_status = match st.map(str::trim) {
                            None | Some("") => TaskStatus::Todo,
                            Some(st) => match parse_task_status(st)? {
                                TaskStatus::Blocked => {
                                    return Err(
                                        "Zadanie nowego tematu może mieć status todo, doing albo done."
                                            .into(),
                                    );
                                }
                                other => other,
                            },
                        };
                        // A repeated task is listed once (agent text).
                        if out.iter().any(|x| fold(&x.title) == fold(&t_title)) {
                            if lenient {
                                continue;
                            }
                            return Err("Powtórzone zadanie w temacie.".into());
                        }
                        out.push(TopicTask {
                            title: t_title,
                            status: t_status,
                        });
                    }
                    out
                }
                Some(_) => return Err("Zadania: zły format.".into()),
            };
            if topic_exists(snap, project, &slug, &title) {
                return Err(TOPIC_EXISTS.into());
            }
            Ok(Change::NewTopic {
                title,
                slug,
                status,
                summary,
                repo,
                text,
                tasks,
                reason,
                branches,
            })
        }
        other => Err(format!(
            "Agent podał nieznany rodzaj zmiany „{}”.",
            clip_label(other, 30)
        )),
    }
}

/// The reason a new topic is dropped as a duplicate.
const TOPIC_EXISTS: &str = "Taki temat już istnieje.";

/// A topic of `project` with this slug or this title (case and diacritics ignored).
fn topic_exists(snap: &Snapshot, project: &str, slug: &str, title: &str) -> bool {
    let want = fold(title);
    snap.topics_of(project)
        .any(|t| t.slug == slug || fold(&t.title) == want)
}

/// The key under which two changes count as the same target.
fn target_key(c: &Change) -> String {
    match c {
        Change::MoveTask { task, .. } => format!("task:{task}"),
        Change::TopicFromTasks { project, slug, .. } => format!("topic:{project}/{slug}"),
        Change::NewTopic { slug, .. } => format!("topic:{slug}"),
        Change::TaskStatus { task, .. } | Change::RemoveTask { task, .. } => format!("task:{task}"),
        Change::TopicLog { topic, .. } => format!("log:{topic}"),
        Change::NewTask { title, topic, .. } => {
            format!("new:{}:{}", topic.as_deref().unwrap_or(""), fold(title))
        }
    }
}

fn no_proposal(report: String, problem: impl Into<String>) -> Proposal {
    Proposal {
        report,
        problem: Some(problem.into()),
        ..Default::default()
    }
}

/// Parses and validates the final answer of an `aktualizacja` run for
/// `project`, against `snap` (the vault as the run started).
pub fn parse(answer: &str, snap: &Snapshot, project: &str) -> Proposal {
    parse_scoped(answer, snap, project, None)
}

fn parse_scoped(
    answer: &str,
    snap: &Snapshot,
    project: &str,
    scope: Option<(&str, &str)>,
) -> Proposal {
    let Split {
        report,
        blocks,
        unclosed,
    } = split(answer);
    if unclosed {
        return no_proposal(
            report,
            "Blok z propozycjami zmian nie jest zamknięty; odpowiedź agenta mogła zostać ucięta.",
        );
    }
    let block = match blocks.as_slice() {
        [] => {
            return no_proposal(
                report,
                format!(
                    "Agent nie dołączył bloku `{FENCE_INFO}` z propozycjami zmian, więc nie ma czego zastosować."
                ),
            );
        }
        [one] => one,
        many => {
            return no_proposal(
                report,
                format!(
                    "Agent dołączył {} bloki z propozycjami zamiast jednego; żaden nie został użyty.",
                    many.len()
                ),
            );
        }
    };
    if block.len() > MAX_BLOCK_BYTES {
        return no_proposal(
            report,
            format!(
                "Blok z propozycjami jest za duży ({} KB, limit {} KB).",
                block.len().div_ceil(1024),
                MAX_BLOCK_BYTES / 1024
            ),
        );
    }
    let value: Value = match serde_json::from_str(block) {
        Ok(v) => v,
        Err(e) => {
            return no_proposal(
                report,
                format!("Blok z propozycjami nie jest poprawnym JSON-em ({e})."),
            );
        }
    };
    let Some(obj) = value.as_object() else {
        return no_proposal(report, "Blok z propozycjami musi zawierać obiekt JSON.");
    };
    if let Some(k) = obj.keys().find(|k| k.as_str() != "changes") {
        return no_proposal(
            report,
            format!(
                "Blok z propozycjami ma nieznane pole „{}”.",
                clip_label(k, 40)
            ),
        );
    }
    let Some(items) = obj.get("changes").and_then(Value::as_array) else {
        return no_proposal(report, "Blok z propozycjami nie ma listy „changes”.");
    };

    let mut out = Proposal {
        report,
        ..Default::default()
    };
    let mut seen: Vec<String> = Vec::new();
    let drop = |out: &mut Proposal, index: usize, label: String, reason: String| {
        if out.dropped.len() < MAX_DROPPED {
            out.dropped.push(Dropped {
                index,
                label,
                reason,
            });
        } else {
            out.dropped_more += 1;
        }
    };
    let mut new_topics = 0usize;
    let mut loose = 0usize;
    for (i, item) in items.iter().enumerate() {
        if i >= MAX_CHANGES {
            drop(
                &mut out,
                i,
                label_of(item, snap, project),
                format!("Ponad limit {MAX_CHANGES} zmian na raz."),
            );
            continue;
        }
        match validate_item(item, snap, project).and_then(|c| match scope {
            Some((topic, note)) => journal_change(c, snap, project, topic, note),
            None => Ok(c),
        }) {
            Ok(change) => {
                let key = target_key(&change);
                // An entry may be named by one change only.
                let entries: Vec<String> = match &change {
                    Change::TopicFromTasks { tasks, .. } => {
                        tasks.iter().map(|e| format!("task:{}", e.id)).collect()
                    }
                    _ => Vec::new(),
                };
                if entries.iter().any(|k| seen.contains(k)) {
                    drop(
                        &mut out,
                        i,
                        label_of(item, snap, project),
                        "Wpis jest już w innej propozycji tej listy.".into(),
                    );
                    continue;
                }
                seen.extend(entries);
                let same_title = matches!(&change, Change::NewTopic { title, .. }
                    if out.changes.iter().any(|c| matches!(c, Change::NewTopic { title: t, .. } if fold(t) == fold(title))));
                if matches!(change, Change::NewTopic { .. }) && (seen.contains(&key) || same_title)
                {
                    drop(
                        &mut out,
                        i,
                        label_of(item, snap, project),
                        TOPIC_EXISTS.into(),
                    );
                } else if seen.contains(&key) {
                    drop(
                        &mut out,
                        i,
                        label_of(item, snap, project),
                        "Powtórzona zmiana tego samego zadania lub tematu.".into(),
                    );
                } else if matches!(change, Change::NewTopic { .. }) && new_topics >= MAX_NEW_TOPICS
                {
                    drop(
                        &mut out,
                        i,
                        label_of(item, snap, project),
                        format!("Ponad limit {MAX_NEW_TOPICS} nowych tematów na raz."),
                    );
                } else if change.is_loose_task() && loose >= MAX_LOOSE_TASKS {
                    drop(
                        &mut out,
                        i,
                        label_of(item, snap, project),
                        format!(
                            "Ponad limit {MAX_LOOSE_TASKS} zadań poza tematami; zadania powinny należeć do tematów."
                        ),
                    );
                } else {
                    if matches!(change, Change::NewTopic { .. }) {
                        new_topics += 1;
                    }
                    if change.is_loose_task() {
                        loose += 1;
                    }
                    seen.push(key);
                    out.changes.push(change);
                }
            }
            Err(reason) => drop(&mut out, i, label_of(item, snap, project), reason),
        }
    }
    out
}

// --- the user's corrections -----------------------------------------------------

/// What the user may change in an undecided proposed change before applying
/// it (Review → "Edytuj"): only what that kind writes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChangeEdit {
    NewTopic {
        title: String,
        status: String,
        summary: String,
        text: String,
        /// One per proposed task, in order: kept or left out, title, status.
        tasks: Vec<TopicTaskEdit>,
    },
    TopicLog {
        /// `None`: the status does not change.
        status: Option<String>,
        /// The summary after the write (equal to the current one = no change).
        summary: String,
        text: String,
    },
    TaskStatus {
        status: String,
    },
    NewTask {
        title: String,
    },
}

/// A proposed task of a new topic as the user corrected it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TopicTaskEdit {
    /// `false` leaves the task out.
    pub keep: bool,
    pub title: String,
    pub status: String,
}

/// `change` corrected by the user, validated like agent text: the same core
/// checks and caps as [`parse`], the same duplicate rules against the other
/// changes of the proposal (`others`) and the vault (`snap`, as it is now).
/// The target must still be as the proposal saw it (else [`STALE`]); the
/// base of the stale check is kept.
pub fn edit_change(
    change: &Change,
    edit: &ChangeEdit,
    others: &[&Change],
    snap: &Snapshot,
    project: &str,
) -> Result<Change, String> {
    check_current(change, snap, project)
        .map_err(|e| format!("Nie można już poprawić tej zmiany: {e}."))?;
    let raw = match (change, edit) {
        (
            Change::NewTopic {
                repo,
                tasks,
                reason,
                branches,
                ..
            },
            ChangeEdit::NewTopic {
                title,
                status,
                summary,
                text,
                tasks: edited,
            },
        ) => {
            if edited.len() != tasks.len() {
                return Err("Lista zadań tematu nie zgadza się z propozycją.".into());
            }
            // The kept tasks go through the same checks as the agent's.
            let kept: Vec<Value> = edited
                .iter()
                .filter(|t| t.keep)
                .map(|t| serde_json::json!({"title": t.title, "status": t.status}))
                .collect();
            serde_json::json!({"kind": "new_topic", "title": title, "status": status,
                "summary": summary, "repo": repo, "text": text, "tasks": kept, "reason": reason,
                "branches": branches})
        }
        (
            Change::TopicLog {
                topic, branches, ..
            },
            ChangeEdit::TopicLog {
                status,
                summary,
                text,
            },
        ) => {
            let mut v = serde_json::json!({"kind": "topic_log", "topic": topic, "text": text,
                "summary": summary, "branches": branches});
            if let Some(st) = status.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                v["status"] = Value::String(st.into());
            }
            v
        }
        (Change::TaskStatus { task, reason, .. }, ChangeEdit::TaskStatus { status }) => {
            serde_json::json!({"kind": "task_status", "task": task, "status": status, "reason": reason})
        }
        (Change::NewTask { topic, reason, .. }, ChangeEdit::NewTask { title }) => {
            serde_json::json!({"kind": "new_task", "title": title, "topic": topic, "reason": reason})
        }
        _ => return Err("Ta poprawka nie pasuje do rodzaju zmiany.".into()),
    };
    let edited = validate_item_as(&raw, snap, project, false)?;
    let key = target_key(&edited);
    for o in others {
        let same_topic_title = matches!((&edited, o), (Change::NewTopic { title: a, .. }, Change::NewTopic { title: b, .. }) if fold(a) == fold(b));
        if target_key(o) == key || same_topic_title {
            return Err(match edited {
                Change::NewTopic { .. } => {
                    "Taki temat jest już w innej propozycji tej listy.".into()
                }
                _ => "Takie zadanie jest już w innej propozycji tej listy.".into(),
            });
        }
    }
    Ok(edited)
}

/// A proposed new topic folded into the existing topic `topic` ("Dołącz do
/// tematu…"): its first journal entry as a `topic_log` of that topic (status
/// and summary unchanged), its tasks as `new_task`s there, its branches with
/// the entry. Each is validated like agent text against `snap` (the vault now)
/// and the other changes of the proposal. Tasks that already exist in the
/// topic are left out and named in the second value.
pub fn join_change(
    change: &Change,
    topic: &str,
    others: &[&Change],
    snap: &Snapshot,
    project: &str,
) -> Result<(Vec<Change>, Vec<String>), String> {
    let Change::NewTopic {
        text,
        tasks,
        reason,
        branches,
        ..
    } = change
    else {
        return Err("Dołączyć można tylko propozycję nowego tematu.".into());
    };
    let target = topic_of(snap, project, topic)
        .ok_or_else(|| "W tym projekcie nie ma takiego tematu.".to_string())?;
    let log = validate_item(
        &serde_json::json!({"kind": "topic_log", "topic": target.slug, "text": text,
            "branches": branches}),
        snap,
        project,
    )?;
    let mut out = vec![log];
    let mut skipped = Vec::new();
    for t in tasks {
        let raw = serde_json::json!({"kind": "new_task", "title": t.title, "topic": target.slug,
            "reason": reason});
        match validate_item(&raw, snap, project) {
            Ok(c) => {
                if !out.iter().any(|o| target_key(o) == target_key(&c)) {
                    out.push(c);
                }
            }
            Err(e) if e.contains("już istnieje") => skipped.push(t.title.clone()),
            Err(e) => return Err(e),
        }
    }
    for c in &out {
        if let Some(o) = others.iter().find(|o| target_key(o) == target_key(c)) {
            return Err(match o {
                Change::TopicLog { .. } => format!(
                    "Na liście jest już wpis do tematu „{}”: popraw tamten wpis zamiast dołączać.",
                    target.title
                ),
                _ => "Takie zadanie jest już w innej propozycji tej listy.".into(),
            });
        }
    }
    Ok((out, skipped))
}

/// Whether `change` can still be applied to the vault as it is now (`snap`):
/// `Err` with [`STALE`] (plus a detail) when its target changed since the
/// proposal was made.
pub fn check_current(change: &Change, snap: &Snapshot, project: &str) -> Result<(), String> {
    let stale = |why: &str| Err(format!("{STALE} ({why})"));
    let place_exists = |loc: &TaskLocation| match loc {
        TaskLocation::Inbox => true,
        TaskLocation::Project { project } => snap.project(project).is_some(),
        TaskLocation::Topic { project, topic } => topic_of(snap, project, topic).is_some(),
    };
    match change {
        Change::RemoveTask { task, base, .. } => match snap.task(task) {
            Some(t) if same_task(t, base) && task_in_project(t, project) => Ok(()),
            _ => stale("zadanie zmieniono, przeniesiono lub usunięto"),
        },
        Change::MoveTask { task, from, to, .. } => match snap.task(task) {
            None => stale("wpisu już nie ma"),
            Some(t) if t.location != *from => stale("wpis przeniesiono"),
            Some(t) if !open_status(t.status) => stale("wpis jest już zrobiony"),
            Some(_) if !place_exists(to) => stale("miejsca docelowego już nie ma"),
            Some(_) => Ok(()),
        },
        Change::TopicFromTasks {
            project: p,
            slug,
            title,
            tasks,
            ..
        } => {
            if snap.project(p).is_none() {
                return stale("projektu już nie ma");
            }
            if topic_exists(snap, p, slug, title) {
                return stale("taki temat już jest");
            }
            for e in tasks {
                if snap.task(&e.id).map(|t| &t.location) != Some(&e.from) {
                    return stale(&format!("wpis „{}” przeniesiono", clip_label(&e.title, 40)));
                }
            }
            Ok(())
        }
        Change::TaskStatus { task, from, .. } => match snap.task(task) {
            None => stale("zadania już nie ma"),
            Some(t) if !task_in_project(t, project) => stale("zadanie przeniesiono"),
            Some(t) if t.status != *from => stale("status zadania się zmienił"),
            Some(_) => Ok(()),
        },
        Change::TopicLog {
            topic,
            base_status,
            base_summary,
            ..
        } => match topic_of(snap, project, topic) {
            None => stale("tematu już nie ma"),
            Some(t) if t.status != *base_status => stale("status tematu się zmienił"),
            Some(t) if t.summary != *base_summary => stale("opis tematu się zmienił"),
            Some(_) => Ok(()),
        },
        Change::NewTopic {
            slug, title, repo, ..
        } => {
            if topic_exists(snap, project, slug, title) {
                return stale("taki temat już jest");
            }
            if let Some(r) = repo
                && !snap
                    .project(project)
                    .is_some_and(|p| p.repos.iter().any(|x| &x.name == r))
            {
                return stale("repozytorium zniknęło z projektu");
            }
            Ok(())
        }
        Change::NewTask { topic, title, .. } => {
            if let Some(slug) = topic
                && topic_of(snap, project, slug).is_none()
            {
                return stale("tematu już nie ma");
            }
            if same_title_at(snap, &change.location(project), title).is_some() {
                return stale("takie zadanie już jest");
            }
            Ok(())
        }
    }
}

// --- input fingerprint --------------------------------------------------------------

/// FNV-1a, 64 bit: stable across builds (unlike `DefaultHasher`), no dependency.
struct Fnv(u64);

impl Fnv {
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

fn fnv(parts: &[&[u8]]) -> u64 {
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    for p in parts {
        h.write(p);
    }
    h.0
}

/// A stable hash of a file's contents (the "still untouched?" check of undo).
pub fn content_hash(text: &str) -> String {
    format!("{:016x}", fnv(&[text.as_bytes()]))
}

/// The project's data as loaded from the vault (`projekt.md` and its topic
/// files: project, topics, tasks, journal), hashed.
pub fn vault_fingerprint(snap: &Snapshot, project: &str) -> String {
    let data = serde_json::json!({
        "project": snap.project(project),
        "topics": snap.topics_of(project).collect::<Vec<_>>(),
        "tasks": snap.tasks.iter().filter(|t| t.location.project() == Some(project)).collect::<Vec<_>>(),
        "journal": snap.journal.iter().filter(|j| j.project == project).collect::<Vec<_>>(),
    });
    format!("{:016x}", fnv(&[data.to_string().as_bytes()]))
}

/// Fingerprint of an `aktualizacja` run's input: the git snapshot of the
/// project's repositories plus [`vault_fingerprint`], as
/// `fnv1a64:<git>/<vault>`. Equal fingerprints mean the agent would look at
/// the same thing again.
pub fn fingerprint(repo_state: &str, snap: Option<&Snapshot>, project: &str) -> String {
    let git = format!("{:016x}", fnv(&[repo_state.as_bytes()]));
    let vault = snap.map_or_else(|| "-".to_string(), |s| vault_fingerprint(s, project));
    format!("{FINGERPRINT_PREFIX}{git}/{vault}")
}

const FINGERPRINT_PREFIX: &str = "fnv1a64:";

/// `fp` with its vault part replaced by `vault`, when its vault part is
/// `before` (the app wrote the vault itself and nothing else changed it).
pub fn refresh_vault_part(fp: &str, before: &str, vault: &str) -> Option<String> {
    let (git, old) = fp.strip_prefix(FINGERPRINT_PREFIX)?.split_once('/')?;
    (old == before).then(|| format!("{FINGERPRINT_PREFIX}{git}/{vault}"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use montybrain_core::{Priority, Project, ProjectState};

    pub(crate) fn task(id: &str, title: &str, status: TaskStatus, loc: TaskLocation) -> Task {
        Task {
            id: Some(id.into()),
            title: title.into(),
            status,
            priority: Priority::Normal,
            due: None,
            done: None,
            links: vec![],
            notes: String::new(),
            location: loc,
            line: 1,
        }
    }

    fn tp(project: &str, topic: &str) -> TaskLocation {
        TaskLocation::Topic {
            project: project.into(),
            topic: topic.into(),
        }
    }

    pub(crate) fn snap() -> Snapshot {
        let project = |slug: &str| Project {
            slug: slug.into(),
            name: slug.to_uppercase(),
            color: None,
            state: ProjectState::Active,
            repos: vec![montybrain_core::Repo {
                name: "app".into(),
                path: format!("/r/{slug}"),
            }],
            description: String::new(),
            path: format!("{slug}/projekt.md").into(),
        };
        let topic = |p: &str, slug: &str, status| Topic {
            project: p.into(),
            slug: slug.into(),
            title: format!("Temat {slug}"),
            status,
            repo: None,
            summary: "Stan.".into(),
            created: None,
            updated: None,
            path: format!("{p}/tematy/{slug}.md").into(),
        };
        Snapshot {
            projects: vec![project("beeking"), project("dom")],
            topics: vec![
                topic("beeking", "platnosci", TopicStatus::InProgress),
                topic("dom", "kuchnia", TopicStatus::Planned),
            ],
            tasks: vec![
                task(
                    "aaaaa1",
                    "Webhook zwrotów",
                    TaskStatus::Doing,
                    tp("beeking", "platnosci"),
                ),
                task(
                    "aaaaa2",
                    "Testy BLIK",
                    TaskStatus::Todo,
                    tp("beeking", "platnosci"),
                ),
                task(
                    "aaaaa3",
                    "Stare",
                    TaskStatus::Done,
                    TaskLocation::Project {
                        project: "beeking".into(),
                    },
                ),
                task("bbbbb1", "Blat", TaskStatus::Todo, tp("dom", "kuchnia")),
                task(
                    "ccccc1",
                    "Ze skrzynki",
                    TaskStatus::Todo,
                    TaskLocation::Inbox,
                ),
            ],
            ..Default::default()
        }
    }

    fn answer(json: &str) -> String {
        format!("Raport: webhook gotowy.\n\n```{FENCE_INFO}\n{json}\n```\n")
    }

    fn inbox_snap() -> Snapshot {
        let mut s = snap();
        s.tasks.push(task(
            "ccccc2",
            "Płytki do łazienki",
            TaskStatus::Todo,
            TaskLocation::Inbox,
        ));
        s.tasks.push(task(
            "ccccc3",
            "Fuga",
            TaskStatus::Todo,
            TaskLocation::Inbox,
        ));
        s.tasks.push(task(
            "ccccc4",
            "Zrobione",
            TaskStatus::Done,
            TaskLocation::Inbox,
        ));
        s
    }

    #[test]
    fn inbox_proposals_have_their_own_kinds_and_rules() {
        let s = inbox_snap();
        let json = r#"{"changes":[
  {"kind":"move_task","task":"ccccc1","to":"dom/kuchnia","reason":"Kuchnia."},
  {"kind":"topic_from_tasks","project":"dom","title":"Łazienka","summary":"Remont.","tasks":["ccccc2","^ccccc3"],"reason":"Jeden remont."},
  {"kind":"new_task","title":"Zmierzyć ścianę","reason":"Z wpisu."},
  {"kind":"topic_from_tasks","project":"dom","title":"Inny","tasks":["ccccc1"],"reason":"r"},
  {"kind":"new_task","title":"W temacie","topic":"kuchnia","reason":"r"},
  {"kind":"topic_log","topic":"kuchnia","text":"t"},
  {"kind":"move_task","task":"aaaaa2","to":"dom","reason":"Nie ze skrzynki."},
  {"kind":"move_task","task":"ccccc4","to":"dom","reason":"Zrobione."},
  {"kind":"move_task","task":"ccccc2","to":"nie-ma","reason":"r"},
  {"kind":"topic_from_tasks","project":"dom","title":"Kuchnia","tasks":["ccccc3"],"reason":"r"}
]}"#;
        let p = parse(&answer(json), &s, INBOX);
        assert_eq!(p.changes.len(), 3, "{p:#?}");
        match &p.changes[0] {
            Change::MoveTask {
                task,
                from,
                to,
                to_label,
                ..
            } => {
                assert_eq!(task, "ccccc1");
                assert_eq!(*from, TaskLocation::Inbox);
                assert_eq!(*to, tp("dom", "kuchnia"));
                assert_eq!(to_label, "DOM / Temat kuchnia");
            }
            o => panic!("{o:?}"),
        }
        match &p.changes[1] {
            Change::TopicFromTasks {
                project,
                slug,
                tasks,
                ..
            } => {
                assert_eq!(project, "dom");
                assert_eq!(slug, "lazienka");
                assert_eq!(
                    tasks.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
                    ["ccccc2", "ccccc3"]
                );
            }
            o => panic!("{o:?}"),
        }
        assert_eq!(p.changes[2].location(INBOX), TaskLocation::Inbox);
        let reasons: Vec<&str> = p.dropped.iter().map(|d| d.reason.as_str()).collect();
        assert!(
            reasons.iter().any(|r| r.contains("innej propozycji")),
            "{reasons:?}"
        );
        assert!(
            reasons.iter().any(|r| r.contains("bez pola „topic”")),
            "{reasons:?}"
        );
        assert!(
            reasons
                .iter()
                .any(|r| r.contains("W Skrzynce agent proponuje")),
            "{reasons:?}"
        );
        assert!(
            reasons.iter().any(|r| r.contains("nie ma już w Skrzynce")),
            "{reasons:?}"
        );
        assert!(
            reasons.iter().any(|r| r.contains("już zrobiony")),
            "{reasons:?}"
        );
        assert!(
            reasons.iter().any(|r| r.contains("Nie ma miejsca")),
            "{reasons:?}"
        );
        // A project's own proposal cannot carry these kinds.
        let p = parse(
            &answer(
                r#"{"changes":[{"kind":"move_task","task":"ccccc1","to":"dom","reason":"r"}]}"#,
            ),
            &s,
            "beeking",
        );
        assert!(p.changes.is_empty());
        assert!(p.dropped[0].reason.contains("tylko dla Skrzynki"));
        // Stale when an entry moved meanwhile.
        let mut later = s.clone();
        later
            .tasks
            .iter_mut()
            .find(|t| t.id.as_deref() == Some("ccccc2"))
            .unwrap()
            .location = TaskLocation::Project {
            project: "dom".into(),
        };
        let parsed = parse(&answer(json), &s, INBOX);
        assert!(check_current(&parsed.changes[1], &later, INBOX).is_err());
        assert!(check_current(&parsed.changes[0], &later, INBOX).is_ok());
    }

    #[test]
    fn the_command_block_of_a_single_entry_answer() {
        let (why, cmd) = command_block(
            "Pasuje do checkoutu.\n\n```montybrain-komenda\n\n^a1b2c3 > whenubuy/checkout\n```\n",
            "montybrain-komenda",
        );
        assert_eq!(why, "Pasuje do checkoutu.");
        assert_eq!(cmd.as_deref(), Some("^a1b2c3 > whenubuy/checkout"));
        let (_, none) = command_block(
            "Nic nie pasuje.\n```montybrain-komenda\n```",
            "montybrain-komenda",
        );
        assert!(none.is_none());
        let (_, none) = command_block("Bez bloku.", "montybrain-komenda");
        assert!(none.is_none());
    }

    #[test]
    fn the_legacy_fence_info_is_still_accepted() {
        let text = "Raport.\n\n```morwisko-zmiany\n{\"changes\":[]}\n```\n";
        let s = split(text);
        assert_eq!(s.report, "Raport.");
        assert_eq!(s.blocks, vec!["{\"changes\":[]}".to_string()]);
        assert!(!s.unclosed);
        // The same through the full parser.
        let json = r#"{"changes": [{"kind": "task_status", "task": "^aaaaa1", "status": "done", "reason": "Commit abc."}]}"#;
        let p = parse(
            &format!("Raport.\n\n```morwisko-zmiany\n{json}\n```\n"),
            &snap(),
            "beeking",
        );
        assert_eq!(p.problem, None);
        assert_eq!(p.report, "Raport.");
        assert_eq!(p.changes.len(), 1);
    }

    fn parse_json(json: &str) -> Proposal {
        parse(&answer(json), &snap(), "beeking")
    }

    #[test]
    fn a_good_proposal() {
        let p = parse_json(
            r#"{"changes": [
  {"kind": "task_status", "task": "^aaaaa1", "status": "done", "reason": "Commit abc „Zwroty” w repo backend."},
  {"kind": "topic_log", "topic": "platnosci", "text": "Zwroty zmergowane.\nZostały testy BLIK.", "status": "waiting", "summary": "Zwroty działają."},
  {"kind": "new_task", "title": "Opisać zwroty w README", "topic": "platnosci", "reason": "Brak dokumentacji."},
  {"kind": "new_task", "title": "Ogólne", "reason": "Na poziomie projektu."}
]}"#,
        );
        assert_eq!(p.problem, None);
        assert_eq!(p.report, "Raport: webhook gotowy.");
        assert!(p.dropped.is_empty(), "{:?}", p.dropped);
        assert_eq!(
            p.changes[0],
            Change::TaskStatus {
                task: "aaaaa1".into(),
                title: "Webhook zwrotów".into(),
                from: TaskStatus::Doing,
                to: TaskStatus::Done,
                reason: "Commit abc „Zwroty” w repo backend.".into(),
            }
        );
        assert_eq!(
            p.changes[1],
            Change::TopicLog {
                topic: "platnosci".into(),
                topic_title: "Temat platnosci".into(),
                text: "Zwroty zmergowane.\nZostały testy BLIK.".into(),
                status: Some(TopicStatus::Waiting),
                summary: Some("Zwroty działają.".into()),
                base_status: TopicStatus::InProgress,
                base_summary: "Stan.".into(),
                branches: vec![],
            }
        );
        assert_eq!(
            p.changes[3].location("beeking"),
            TaskLocation::Project {
                project: "beeking".into()
            }
        );
        // The stored form round-trips.
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["changes"][0]["kind"], "task_status");
        assert_eq!(json["changes"][0]["from"], "doing");
        assert_eq!(serde_json::from_value::<Proposal>(json).unwrap(), p);
    }

    #[test]
    fn empty_proposal_and_no_op_fields() {
        let p = parse_json(r#"{"changes": []}"#);
        assert_eq!(p.problem, None);
        assert!(p.changes.is_empty());
        // A status / summary equal to the base is no change.
        let p = parse_json(
            r#"{"changes":[{"kind":"topic_log","topic":"platnosci","text":"x","status":"w-toku","summary":"Stan."}]}"#,
        );
        let Change::TopicLog {
            status, summary, ..
        } = &p.changes[0]
        else {
            panic!()
        };
        assert_eq!((status, summary), (&None, &None));
    }

    #[test]
    fn missing_duplicated_unclosed_or_malformed_blocks_are_no_proposal() {
        let s = snap();
        let no_block = parse(
            "Tylko raport.\n\n```json\n{\"changes\":[]}\n```",
            &s,
            "beeking",
        );
        assert!(no_block.problem.unwrap().contains("nie dołączył bloku"));
        // The report keeps other code blocks.
        assert!(no_block.report.contains("```json"));

        let two = format!(
            "{}\n{}",
            answer(r#"{"changes":[]}"#),
            answer(r#"{"changes":[]}"#)
        );
        let p = parse(&two, &s, "beeking");
        assert!(p.problem.unwrap().contains("2 bloki"));
        assert!(p.changes.is_empty());

        let cut = format!("Raport\n```{FENCE_INFO}\n{{\"changes\": [");
        let p = parse(&cut, &s, "beeking");
        assert!(p.problem.unwrap().contains("nie jest zamknięty"));
        assert_eq!(p.report, "Raport");

        for (json, want) in [
            ("{nie json", "poprawnym JSON"),
            ("[1,2]", "obiekt JSON"),
            (r#"{"changes": [], "extra": 1}"#, "nieznane pole „extra”"),
            (r#"{"zmiany": []}"#, "nieznane pole"),
            (r#"{"changes": {}}"#, "listy „changes”"),
            ("{}", "listy „changes”"),
        ] {
            let p = parse_json(json);
            assert!(
                p.problem.as_deref().unwrap_or("").contains(want),
                "{json}: {p:?}"
            );
        }
        let huge = format!(
            r#"{{"changes": [], "x": "{}"}}"#,
            "a".repeat(MAX_BLOCK_BYTES)
        );
        assert!(parse_json(&huge).problem.unwrap().contains("za duży"));
        // Empty answer.
        assert!(parse("", &s, "beeking").problem.is_some());
    }

    #[test]
    fn a_block_quoted_inside_another_fence_does_not_count() {
        let inner = format!(
            "Przykład:\n````markdown\n```{FENCE_INFO}\n{{\"changes\":[]}}\n```\n````\n\n```{FENCE_INFO}\n{{\"changes\":[]}}\n```"
        );
        let p = parse(&inner, &snap(), "beeking");
        assert_eq!(p.problem, None, "{p:?}");
        assert!(p.report.contains("````markdown"));
        // Tilde fences and a longer closing fence work as well.
        let tilde = format!("R\n~~~~ {FENCE_INFO}\n{{\"changes\":[]}}\n~~~~~\n");
        assert_eq!(parse(&tilde, &snap(), "beeking").problem, None);
    }

    fn dropped_reasons(json: &str) -> Vec<String> {
        let p = parse_json(json);
        assert_eq!(p.problem, None, "{p:?}");
        p.dropped
            .iter()
            .map(|d| format!("{}: {}", d.label, d.reason))
            .collect()
    }

    #[test]
    fn bad_items_are_dropped_one_by_one_with_a_reason() {
        let r = dropped_reasons(
            r#"{"changes": [
  {"kind": "task_status", "task": "aaaaa2", "status": "doing", "reason": "Gałąź feature/blik."},
  {"kind": "task_status", "task": "bbbbb1", "status": "done", "reason": "x"},
  {"kind": "task_status", "task": "ccccc1", "status": "done", "reason": "x"},
  {"kind": "task_status", "task": "aaaaa3", "status": "todo", "reason": "x"},
  {"kind": "task_status", "task": "zzzzz9", "status": "done", "reason": "x"},
  {"kind": "task_status", "task": "../../etc", "status": "done", "reason": "x"},
  {"kind": "task_status", "task": "aaaaa1", "status": "finished", "reason": "x"},
  {"kind": "task_status", "task": "aaaaa1", "status": "doing", "reason": "x"},
  {"kind": "task_status", "task": "aaaaa1", "status": "done", "reason": "x", "force": true},
  {"kind": "task_status", "task": "aaaaa1", "status": "done"},
  {"kind": "task_status", "task": "aaaaa1", "status": 3, "reason": "x"},
  {"kind": "delete_task", "task": "aaaaa1"},
  {"task": "aaaaa1"},
  "tekst",
  {"kind": "topic_log", "topic": "kuchnia", "text": "x"},
  {"kind": "topic_log", "topic": "platnosci", "text": "x", "status": "skonczony"},
  {"kind": "new_task", "title": "Testy BLIK", "topic": "platnosci", "reason": "dup"},
  {"kind": "task_status", "task": "aaaaa2", "status": "done", "reason": "Drugi raz to samo zadanie."}
]}"#,
        );
        let want = [
            "Status zadania: Zadanie nie należy do tego projektu.",
            "Status zadania: Zadanie nie należy do tego projektu.",
            "Status zadania „Stare”: Zadanie jest już zrobione.",
            "Status zadania: Nie ma już takiego zadania w projekcie.",
            "Status zadania: Nieprawidłowy identyfikator zadania.",
            "Status zadania „Webhook zwrotów”: Agent podał nieznany status zadania „finished”.",
            "Status zadania „Webhook zwrotów”: Zadanie ma już ten status.",
            // An extra field ("force") is ignored and a missing reason replaced:
            // that change is kept, so the next one is a repeat.
            "Status zadania „Webhook zwrotów”: Powtórzona zmiana tego samego zadania lub tematu.",
            "Status zadania „Webhook zwrotów”: Status: zły format.",
            "Zmiana „delete_task”: Agent podał nieznany rodzaj zmiany „delete_task”.",
            "Zmiana bez rodzaju: Brakuje pola „kind”.",
            "Zmiana bez rodzaju: Zmiana ma zły format.",
            "Wpis w dzienniku: W tym projekcie nie ma takiego tematu.",
            "Wpis w dzienniku „Temat platnosci”: Agent podał nieznany status tematu „skonczony”.",
            "Nowe zadanie „Testy BLIK”: Takie zadanie już istnieje w tym miejscu.",
            "Status zadania „Testy BLIK”: Powtórzona zmiana tego samego zadania lub tematu.",
        ];
        assert_eq!(r, want);
        let p = parse_json(
            r#"{"changes":[{"kind":"task_status","task":"aaaaa2","status":"doing","reason":"ok"}]}"#,
        );
        assert_eq!(p.changes.len(), 1);
    }

    /// Agent text that would create Markdown structure is refused by the same
    /// rules core applies before writing; display-only text is flattened.
    #[test]
    fn injected_markdown_structure_is_refused() {
        let r = dropped_reasons(
            r#"{"changes": [
  {"kind": "topic_log", "topic": "platnosci", "text": "ok", "summary": "Stan\n## Zadania\n- [ ] Wstrzyknięte ^zzzzzz"},
  {"kind": "topic_log", "topic": "platnosci", "text": "ok", "summary": "---\nstatus: zrobiony"},
  {"kind": "topic_log", "topic": "platnosci", "text": "ok", "summary": "a\n```\nreszta pliku"},
  {"kind": "topic_log", "topic": "platnosci", "text": "a\rb"},
  {"kind": "new_task", "title": "Zrobić @2026-11-20", "reason": "x"},
  {"kind": "new_task", "title": "Pilne !wysoki", "reason": "x"},
  {"kind": "new_task", "title": "x", "reason": "a\u001bb"}
]}"#,
        );
        assert_eq!(r.len(), 6, "{r:#?}");
        assert!(
            r[0].contains("Opis tematu nie może zostać zapisany")
                && r[0].contains("level-2 heading"),
            "{}",
            r[0]
        );
        assert!(r[1].contains("---"), "{}", r[1]);
        assert!(r[2].contains("code fence"), "{}", r[2]);
        assert!(r[3].contains("Wpis do dziennika"), "{}", r[3]);
        assert!(r[4].contains("Tytuł") && r[5].contains("Tytuł"));
        // A control character in display-only text is dropped, not the change.
        let p = parse_json(r#"{"changes":[{"kind":"new_task","title":"x","reason":"a\u001bb"}]}"#);
        let Change::NewTask { reason, .. } = &p.changes[0] else {
            panic!()
        };
        assert_eq!(reason, "a b");

        // A journal text is written indented: structure-looking lines are kept
        // as text (core verifies the write again when it is applied).
        let p = parse_json(
            r###"{"changes":[{"kind":"topic_log","topic":"platnosci","text":"## Zadania\n- [ ] x ^zzzzzz"}]}"###,
        );
        assert_eq!(p.changes.len(), 1);
        // A multi-line title collapses to one line, as core stores it.
        let p = parse_json(
            r#"{"changes":[{"kind":"new_task","title":"Pierwsza\n- [ ] druga","reason":"r\nr"}]}"#,
        );
        let Change::NewTask { title, reason, .. } = &p.changes[0] else {
            panic!()
        };
        assert_eq!(
            (title.as_str(), reason.as_str()),
            ("Pierwsza - [ ] druga", "r r")
        );
    }

    #[test]
    fn repairable_text_is_normalised_not_dropped() {
        let p = parse_json(
            r#"{"changes": [
  {"kind": "task_status", "task": "^aaaaa1", "status": "Zrobione"},
  {"kind": "topic_log", "topic": "platnosci", "text": "t", "status": "W toku"},
  {"kind": "new_topic", "title": "Bez statusu", "summary": "S.", "text": "T.", "reason": "", "tasks": ["Pierwsze", "Pierwsze", {"title": "Drugie"}]}
]}"#,
        );
        assert_eq!(p.changes.len(), 3, "{:?}", p.dropped);
        assert!(
            matches!(&p.changes[0], Change::TaskStatus { to: TaskStatus::Done, reason, .. } if reason == NO_REASON)
        );
        let Change::NewTopic {
            status,
            tasks,
            reason,
            ..
        } = &p.changes[2]
        else {
            panic!()
        };
        assert_eq!(*status, TopicStatus::Planned);
        assert_eq!(
            tasks.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            ["Pierwsze", "Drugie"]
        );
        assert_eq!(reason, NO_REASON);
        // What protects the vault stays hard: unknown ids, places, kinds, structure.
        let r = dropped_reasons(
            r#"{"changes": [
  {"kind": "task_status", "task": "../../etc", "status": "done"},
  {"kind": "delete_task", "task": "aaaaa1"},
  {"kind": "new_task", "title": "Zrobić @2026-11-20"}
]}"#,
        );
        assert_eq!(r.len(), 3, "{r:#?}");
    }

    #[test]
    fn caps_on_count_and_lengths() {
        let item = |i: usize| {
            format!(
                r#"{{"kind":"new_task","title":"Zadanie {i}","topic":"platnosci","reason":"r"}}"#
            )
        };
        let items: Vec<String> = (0..MAX_CHANGES + 5).map(item).collect();
        let p = parse_json(&format!(r#"{{"changes":[{}]}}"#, items.join(",")));
        assert_eq!(p.changes.len(), MAX_CHANGES);
        assert_eq!(p.dropped.len(), 5);
        assert!(p.dropped.iter().all(|d| d.reason.contains("Ponad limit")));
        assert_eq!(p.dropped[0].index, MAX_CHANGES);

        let long = |n: usize| "ą".repeat(n);
        let r = dropped_reasons(&format!(
            r#"{{"changes":[
  {{"kind":"new_task","title":"{}","reason":"r"}},
  {{"kind":"new_task","title":"t","reason":"{}"}},
  {{"kind":"topic_log","topic":"platnosci","text":"{}"}},
  {{"kind":"topic_log","topic":"platnosci","text":"x","summary":"{}"}}
]}}"#,
            long(MAX_TITLE_CHARS + 1),
            long(MAX_REASON_CHARS + 1),
            long(MAX_LOG_CHARS + 1),
            long(MAX_SUMMARY_CHARS + 1)
        ));
        // Long titles, reasons, entries and summaries are shortened with "…",
        // not a reason to drop the change (the second log of one topic is a repeat).
        assert!(r.iter().all(|x| !x.contains("za długie")), "{r:#?}");
        let p = parse_json(&format!(
            r#"{{"changes":[{{"kind":"new_task","title":"{}","reason":"{}"}}]}}"#,
            long(MAX_TITLE_CHARS + 50),
            long(MAX_REASON_CHARS + 50)
        ));
        let Change::NewTask { title, reason, .. } = &p.changes[0] else {
            panic!("{p:?}")
        };
        assert_eq!(title.chars().count(), MAX_TITLE_CHARS);
        assert!(title.ends_with('…') && reason.ends_with('…'));
        assert_eq!(reason.chars().count(), MAX_REASON_CHARS);
        // Exactly at the limits is fine.
        let p = parse_json(&format!(
            r#"{{"changes":[{{"kind":"new_task","title":"{}","reason":"{}"}}]}}"#,
            long(MAX_TITLE_CHARS),
            long(MAX_REASON_CHARS)
        ));
        assert_eq!(p.changes.len(), 1, "{p:?}");

        // Many bad items: the dropped list is bounded.
        let bad: Vec<String> = (0..200).map(|_| r#"{"kind":"x"}"#.to_string()).collect();
        let p = parse_json(&format!(r#"{{"changes":[{}]}}"#, bad.join(",")));
        assert_eq!(p.dropped.len(), MAX_DROPPED);
        assert_eq!(p.dropped_more, 200 - MAX_DROPPED);
    }

    #[test]
    fn staleness() {
        let s = snap();
        let p = parse_json(
            r#"{"changes": [
  {"kind": "task_status", "task": "aaaaa1", "status": "done", "reason": "r"},
  {"kind": "topic_log", "topic": "platnosci", "text": "t", "status": "waiting"},
  {"kind": "new_task", "title": "Nowe", "topic": "platnosci", "reason": "r"}
]}"#,
        );
        for c in &p.changes {
            assert_eq!(check_current(c, &s, "beeking"), Ok(()));
        }
        let mut changed = s.clone();
        changed.tasks[0].status = TaskStatus::Blocked;
        changed.topics[0].summary = "Inny".into();
        changed.tasks.push(task(
            "ddddd1",
            "nowe",
            TaskStatus::Todo,
            tp("beeking", "platnosci"),
        ));
        for c in &p.changes {
            let e = check_current(c, &changed, "beeking").unwrap_err();
            assert!(e.starts_with(STALE), "{e}");
        }
        let mut gone = s.clone();
        gone.tasks.remove(0);
        gone.topics.remove(0);
        for c in &p.changes {
            assert!(check_current(c, &gone, "beeking").is_err());
        }
        // Moved to another project.
        let mut moved = s.clone();
        moved.tasks[0].location = tp("dom", "kuchnia");
        assert!(
            check_current(&p.changes[0], &moved, "beeking")
                .unwrap_err()
                .contains("przeniesiono")
        );
    }

    #[test]
    fn fingerprint_covers_git_and_this_projects_data_only() {
        let s = snap();
        let fp = fingerprint("git A", Some(&s), "beeking");
        assert!(
            fp.starts_with("fnv1a64:") && fp.len() == 8 + 16 + 1 + 16,
            "{fp}"
        );
        assert_eq!(fp, fingerprint("git A", Some(&s.clone()), "beeking"));
        assert_ne!(fp, fingerprint("git B", Some(&s), "beeking"));
        let mut t = s.clone();
        t.tasks[1].status = TaskStatus::Doing;
        assert_ne!(fp, fingerprint("git A", Some(&t), "beeking"));
        // Another project's change does not count.
        let mut other = s.clone();
        other.tasks[3].title = "Inny blat".into();
        other.tasks[4].title = "Inna skrzynka".into();
        assert_eq!(fp, fingerprint("git A", Some(&other), "beeking"));
        // Known value (FNV-1a of the empty input): stable across builds.
        assert_eq!(fingerprint("", None, "x"), "fnv1a64:cbf29ce484222325/-");
        // The app's own write refreshes the vault half only if nothing else changed.
        let before = vault_fingerprint(&s, "beeking");
        let after = vault_fingerprint(&t, "beeking");
        let refreshed = refresh_vault_part(&fp, &before, &after).unwrap();
        assert_eq!(refreshed, fingerprint("git A", Some(&t), "beeking"));
        assert_eq!(refresh_vault_part(&fp, &after, &before), None);
        assert_eq!(refresh_vault_part("zle", &before, &after), None);
    }

    #[test]
    fn moved_task_is_stale() {
        let s = snap();
        let p = parse_json(
            r#"{"changes":[{"kind":"task_status","task":"aaaaa1","status":"done","reason":"r"}]}"#,
        );
        let mut moved = s.clone();
        moved.tasks[0].location = tp("dom", "kuchnia");
        assert!(
            check_current(&p.changes[0], &moved, "beeking")
                .unwrap_err()
                .contains("przeniesiono")
        );
    }

    fn topic_json(title: &str, extra: &str) -> String {
        format!(
            r#"{{"kind":"new_topic","title":"{title}","status":"in_progress","summary":"Integracja w toku (wniosek z gałęzi).","repo":"app","text":"Gałąź feature/x, 4 commity od 10 lis.","tasks":[{{"title":"Testy","status":"todo"}},{{"title":"Webhook","status":"done"}}],"reason":"Aktywna gałąź."{extra}}}"#
        )
    }

    #[test]
    fn new_topics_are_validated() {
        let p = parse_json(&format!(
            r#"{{"changes":[{}]}}"#,
            topic_json("Płatności BLIK", "")
        ));
        assert_eq!(p.problem, None);
        assert!(p.dropped.is_empty(), "{:?}", p.dropped);
        assert_eq!(
            p.changes[0],
            Change::NewTopic {
                title: "Płatności BLIK".into(),
                slug: "platnosci-blik".into(),
                status: TopicStatus::InProgress,
                summary: "Integracja w toku (wniosek z gałęzi).".into(),
                repo: Some("app".into()),
                text: "Gałąź feature/x, 4 commity od 10 lis.".into(),
                tasks: vec![
                    TopicTask {
                        title: "Testy".into(),
                        status: TaskStatus::Todo
                    },
                    TopicTask {
                        title: "Webhook".into(),
                        status: TaskStatus::Done
                    },
                ],
                reason: "Aktywna gałąź.".into(),
                branches: vec![],
            }
        );
        let json = serde_json::to_value(&p.changes[0]).unwrap();
        assert_eq!(json["kind"], "new_topic");
        assert_eq!(json["tasks"][1]["status"], "done");
        // Without repo and tasks.
        let minimal = r#"{"changes":[{"kind":"new_topic","title":"Migracja bazy","status":"done","summary":"Zrobione.","text":"Zmergowane 2 lis.","reason":"r"}]}"#;
        let p = parse_json(minimal);
        assert!(
            matches!(&p.changes[0], Change::NewTopic { repo: None, tasks, .. } if tasks.is_empty()),
            "{p:?}"
        );
    }

    #[test]
    fn branches_on_topic_changes() {
        let p = parse_json(
            r#"{"changes":[
  {"kind":"new_topic","title":"Kasa offline","status":"in_progress","summary":"S.","text":"T.","reason":"r","branches":[{"repo":"app","branch":"origin/feature/pos-offline"},{"repo":"app","branch":"feature/pos-offline"}]},
  {"kind":"topic_log","topic":"platnosci","text":"W.","branches":[{"repo":"app","branch":"blik-e2e"}]},
  {"kind":"topic_log","topic":"platnosci","text":"Dwa.","branches":[{"repo":"api","branch":"x"}]},
  {"kind":"new_topic","title":"Zła gałąź","status":"planned","summary":"S.","text":"T.","reason":"r","branches":[{"repo":"app","branch":"--upload-pack=x"}]},
  {"kind":"new_topic","title":"Za dużo","status":"planned","summary":"S.","text":"T.","reason":"r","branches":[{"repo":"app","branch":"a"},{"repo":"app","branch":"b"},{"repo":"app","branch":"c"},{"repo":"app","branch":"d"},{"repo":"app","branch":"e"},{"repo":"app","branch":"f"},{"repo":"app","branch":"g"}]},
  {"kind":"new_topic","title":"Pola","status":"planned","summary":"S.","text":"T.","reason":"r","branches":[{"repo":"app","branch":"a","sha":"1"}]}
]}"#,
        );
        // Too many branches: the first six kept; an extra field ignored.
        assert_eq!(p.changes.len(), 4, "{:?}", p.dropped);
        assert!(
            matches!(&p.changes[2], Change::NewTopic { branches, .. } if branches.len() == MAX_BRANCHES)
        );
        let Change::NewTopic { branches, .. } = &p.changes[0] else {
            panic!()
        };
        assert_eq!(
            branches,
            &vec![BranchRef {
                repo: "app".into(),
                branch: "feature/pos-offline".into()
            }],
            "origin/ stripped, duplicates once"
        );
        assert!(matches!(&p.changes[1], Change::TopicLog { branches, .. } if branches.len() == 1));
        let reasons: Vec<&str> = p.dropped.iter().map(|d| d.reason.as_str()).collect();
        assert_eq!(
            reasons,
            [
                "Gałąź: projekt nie ma repozytorium o tej nazwie.",
                "Nieprawidłowa nazwa gałęzi „--upload-pack=x”.",
            ]
        );
        // Without branches the JSON shape is unchanged (no empty list written).
        let plain =
            parse_json(r#"{"changes":[{"kind":"topic_log","topic":"platnosci","text":"W."}]}"#);
        assert!(
            serde_json::to_value(&plain.changes[0])
                .unwrap()
                .get("branches")
                .is_none()
        );
        for (b, ok) in [
            ("feature/x", true),
            ("a..b", false),
            ("-x", false),
            ("x.lock", false),
            ("a b", false),
            ("ąę/x", true),
        ] {
            assert_eq!(valid_branch_name(b), ok, "{b}");
        }
    }

    #[test]
    fn a_new_topic_joins_an_existing_one() {
        let s = snap();
        let p = parse_json(
            r#"{"changes":[{"kind":"new_topic","title":"Webhook i testy","status":"in_progress","summary":"S.","text":"Gałąź blik-e2e, 3 commity.","reason":"Aktywna gałąź.","tasks":[{"title":"Nowe testy","status":"doing"},{"title":"testy blik","status":"todo"}],"branches":[{"repo":"app","branch":"blik-e2e"}]}]}"#,
        );
        let (out, skipped) = join_change(&p.changes[0], "platnosci", &[], &s, "beeking").unwrap();
        assert_eq!(skipped, ["testy blik"], "already in the topic");
        assert_eq!(out.len(), 2);
        assert!(
            matches!(&out[0], Change::TopicLog { topic, text, status: None, summary: None, branches, .. }
            if topic == "platnosci" && text.starts_with("Gałąź blik-e2e") && branches.len() == 1)
        );
        assert!(
            matches!(&out[1], Change::NewTask { title, topic: Some(t), .. } if title == "Nowe testy" && t == "platnosci")
        );
        // Another entry for that topic on the list: refused.
        let other = Change::TopicLog {
            topic: "platnosci".into(),
            topic_title: "Temat platnosci".into(),
            text: "x".into(),
            status: None,
            summary: None,
            base_status: TopicStatus::InProgress,
            base_summary: "Stan.".into(),
            branches: vec![],
        };
        assert!(
            join_change(&p.changes[0], "platnosci", &[&other], &s, "beeking")
                .unwrap_err()
                .contains("popraw tamten wpis")
        );
        assert!(join_change(&p.changes[0], "nie-ma", &[], &s, "beeking").is_err());
        assert!(
            join_change(&other, "platnosci", &[], &s, "beeking").is_err(),
            "only a new topic"
        );
    }

    #[test]
    fn bad_new_topics_are_dropped_with_plain_reasons() {
        let six: Vec<String> = (0..MAX_TOPIC_TASKS + 1)
            .map(|i| format!(r#"{{"title":"Z{i}"}}"#))
            .collect();
        let r = dropped_reasons(&format!(
            r#"{{"changes":[
  {},
  {},
  {},
  {},
  {},
  {},
  {},
  {},
  {}
]}}"#,
            // Same slug as the existing topic "platnosci" / same title (diacritics, case).
            topic_json("Płatności", ""),
            topic_json("TEMAT PLATNOSCI", ""),
            topic_json("Nowy temat", ""),
            topic_json("nowy  TEMAT", ""),
            topic_json("Inne repo", r#","repo2":"x""#),
            topic_json("Zły status", "").replace(r#""status":"in_progress""#, r#""status":"skonczony""#),
            topic_json("Obce repo", "").replace(r#""repo":"app""#, r#""repo":"nie-ma""#),
            topic_json("Za dużo zadań", "").replace(
                r#""tasks":[{"title":"Testy","status":"todo"},{"title":"Webhook","status":"done"}]"#,
                &format!(r#""tasks":[{}]"#, six.join(","))
            ),
            topic_json("Zablokowane zadanie", "").replace(r#""status":"done"}]"#, r#""status":"blocked"}]"#),
        ));
        assert_eq!(
            r,
            [
                "Nowy temat „Płatności”: Taki temat już istnieje.",
                "Nowy temat „TEMAT PLATNOSCI”: Taki temat już istnieje.",
                "Nowy temat „nowy TEMAT”: Taki temat już istnieje.",
                "Nowy temat „Zły status”: Agent podał nieznany status tematu „skonczony”.",
                "Nowy temat „Obce repo”: Projekt nie ma repozytorium o tej nazwie.",
                "Nowy temat „Zablokowane zadanie”: Zadanie nowego tematu może mieć status todo, doing albo done.",
            ]
        );
        // Kept, repaired: an extra field ignored, the first six of seven tasks.
        let p = parse_json(&format!(
            r#"{{"changes":[{},{}]}}"#,
            topic_json("Inne repo", r#","repo2":"x""#),
            topic_json("Za dużo zadań", "").replace(
                r#""tasks":[{"title":"Testy","status":"todo"},{"title":"Webhook","status":"done"}]"#,
                &format!(r#""tasks":[{}]"#, six.join(","))
            ),
        ));
        assert_eq!(p.changes.len(), 2, "{:?}", p.dropped);
        assert!(
            matches!(&p.changes[1], Change::NewTopic { tasks, .. } if tasks.len() == MAX_TOPIC_TASKS)
        );
        // Hostile text: structure in the summary, a token-like task title, oversized text.
        let r = dropped_reasons(&format!(
            r#"{{"changes":[{},{},{},{}]}}"#,
            topic_json("Wstrzyknięcie", "").replace(
                "Integracja w toku (wniosek z gałęzi).",
                r"Stan\n## Zadania\n- [ ] obce ^zzzzzz"
            ),
            topic_json("Token", "").replace(r#""title":"Testy""#, r#""title":"Testy @2026-11-20""#),
            topic_json("Długi", "").replace(
                "Gałąź feature/x, 4 commity od 10 lis.",
                &"ą".repeat(MAX_LOG_CHARS + 1)
            ),
            topic_json(r"Dwie\nlinie # tytuł", ""),
        ));
        // The oversized journal entry is shortened, the others still refused.
        assert_eq!(r.len(), 2, "{r:#?}");
        assert!(
            r[0].contains("Opis tematu nie może zostać zapisany"),
            "{}",
            r[0]
        );
        assert!(r[1].contains("Tytuł"), "{}", r[1]);
        // The multi-line title is collapsed to one line, as core stores it.
        let p = parse_json(&format!(
            r#"{{"changes":[{}]}}"#,
            topic_json(r"Dwie\nlinie", "")
        ));
        assert!(matches!(&p.changes[0], Change::NewTopic { title, .. } if title == "Dwie linie"));
    }

    #[test]
    fn topic_caps_and_loose_tasks() {
        let topics: Vec<String> = (0..MAX_NEW_TOPICS + 2)
            .map(|i| topic_json(&format!("Temat {i}"), ""))
            .collect();
        let loose: Vec<String> = (0..MAX_LOOSE_TASKS + 2)
            .map(|i| format!(r#"{{"kind":"new_task","title":"Luźne {i}","reason":"r"}}"#))
            .collect();
        let in_topic =
            r#"{"kind":"new_task","title":"W temacie","topic":"platnosci","reason":"r"}"#;
        let p = parse_json(&format!(
            r#"{{"changes":[{},{},{in_topic}]}}"#,
            topics.join(","),
            loose.join(",")
        ));
        let kinds = |k: fn(&Change) -> bool| p.changes.iter().filter(|c| k(c)).count();
        assert_eq!(
            kinds(|c| matches!(c, Change::NewTopic { .. })),
            MAX_NEW_TOPICS
        );
        assert_eq!(kinds(|c| c.is_loose_task()), MAX_LOOSE_TASKS);
        assert_eq!(
            kinds(|c| matches!(c, Change::NewTask { topic: Some(_), .. })),
            1
        );
        assert!(
            p.dropped
                .iter()
                .filter(|d| d.reason.contains("nowych tematów"))
                .count()
                == 2
        );
        assert!(
            p.dropped
                .iter()
                .filter(|d| d.reason.contains("poza tematami"))
                .count()
                == 2
        );
        // A task for a topic proposed in the same block cannot be placed: it belongs in `tasks`.
        let p = parse_json(&format!(
            r#"{{"changes":[{},{{"kind":"new_task","title":"X","topic":"nowy","reason":"r"}}]}}"#,
            topic_json("Nowy", "")
        ));
        assert_eq!(p.changes.len(), 1);
        assert_eq!(
            p.dropped[0].reason,
            "W tym projekcie nie ma takiego tematu."
        );
    }

    #[test]
    fn a_new_topic_goes_stale_when_such_a_topic_appeared() {
        let p = parse_json(&format!(
            r#"{{"changes":[{}]}}"#,
            topic_json("Płatności BLIK", "")
        ));
        let c = &p.changes[0];
        let s = snap();
        assert_eq!(check_current(c, &s, "beeking"), Ok(()));
        let mut appeared = s.clone();
        let mut t = appeared.topics[0].clone();
        t.slug = "inny-slug".into();
        t.title = "płatności blik".into();
        appeared.topics.push(t);
        assert!(
            check_current(c, &appeared, "beeking")
                .unwrap_err()
                .contains("taki temat już jest")
        );
        let mut no_repo = s.clone();
        no_repo.projects[0].repos.clear();
        assert!(
            check_current(c, &no_repo, "beeking")
                .unwrap_err()
                .contains("repozytorium")
        );
    }
    #[test]
    fn journal_removals_validate_ids_project_scope_duplicates_and_staleness() {
        let s = snap();
        let json = r#"{"changes":[
          {"kind":"remove_task","task":"aaaaa1","reason":"Nieaktualne."},
          {"kind":"remove_task","task":"zzzzzz","reason":"Nieznane."},
          {"kind":"remove_task","task":"bbbbb1","reason":"Inny projekt."},
          {"kind":"task_status","task":"aaaaa1","status":"done","reason":"Powtórka."},
          {"kind":"remove_task","task":"aaaaa3","reason":"Poza tematem."}
        ]}"#;
        let p = parse_journal(
            &answer(json),
            &s,
            "beeking",
            "platnosci",
            "Usuń nieaktualne zadania.",
        );
        assert_eq!(p.changes.len(), 1);
        assert_eq!(p.dropped.len(), 4);
        assert!(p.dropped.iter().any(|d| d.reason.contains("Nie ma już")));
        assert!(p.dropped.iter().any(|d| d.reason.contains("nie należy")));
        assert!(p.dropped.iter().any(|d| d.reason.contains("zakresem")));
        let mut later = s.clone();
        later.tasks[0].line = 99;
        assert!(check_current(&p.changes[0], &later, "beeking").is_ok());
        later.tasks[0].notes = "Ręczna zmiana".into();
        assert!(check_current(&p.changes[0], &later, "beeking").is_err());
    }

    #[test]
    fn journal_scope_stays_in_topic_even_when_entries_name_another_topic() {
        let mut s = snap();
        let mut other = s.topics[0].clone();
        other.slug = "inne".into();
        other.title = "Inny temat".into();
        s.topics.push(other);
        let json = r#"{"changes":[{"kind":"new_task","topic":"inne","title":"Krok","reason":"r"},
          {"kind":"topic_log","topic":"platnosci","text":"Już zapisano","status":"waiting"},
          {"kind":"topic_log","topic":"inne","text":"Sam wpis"}] }"#;
        let p = parse_journal(&answer(json), &s, "beeking", "platnosci", "Już zapisano");
        assert!(p.changes.is_empty());
        assert_eq!(p.dropped.len(), 3);
        let p = parse_journal(
            &answer(json),
            &s,
            "beeking",
            "platnosci",
            "Inny temat wymaga kroku",
        );
        assert_eq!(p.changes.len(), 1);
        assert!(matches!(&p.changes[0], Change::TopicLog { topic, .. } if topic == "platnosci"));
    }
}
