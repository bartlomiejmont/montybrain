//! Public data model. Every type here is `Serialize + Deserialize + Clone` because it
//! crosses the Tauri IPC boundary and MCP JSON.
//!
//! JSON conventions:
//! * enums serialize as English `snake_case` strings (`"in_progress"`), but also
//!   *deserialize* from the Polish words used in the files (`"w-toku"`);
//! * dates are `"YYYY-MM-DD"`, timestamps are minute precision `"YYYY-MM-DDTHH:MM"`,
//!   a [`Due`] is `"YYYY-MM-DD"` or `"YYYY-MM-DDTHH:MM"`;
//! * paths are relative to the vault root, with `/` separators.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Task status, stored as the checkbox character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// `[ ]`
    #[default]
    Todo,
    /// `[/]`
    Doing,
    /// `[x]` (also `[X]` when reading)
    Done,
    /// `[!]`
    Blocked,
}

impl TaskStatus {
    pub fn from_checkbox(c: char) -> Option<Self> {
        match c {
            ' ' => Some(Self::Todo),
            '/' => Some(Self::Doing),
            'x' | 'X' => Some(Self::Done),
            '!' => Some(Self::Blocked),
            _ => None,
        }
    }

    pub fn checkbox(self) -> char {
        match self {
            Self::Todo => ' ',
            Self::Doing => '/',
            Self::Done => 'x',
            Self::Blocked => '!',
        }
    }

    pub fn is_done(self) -> bool {
        self == Self::Done
    }
}

/// Task priority: `!wysoki`, (absent), `!niski`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    #[serde(alias = "wysoki")]
    High,
    #[default]
    #[serde(alias = "normalny")]
    Normal,
    #[serde(alias = "niski")]
    Low,
}

impl Priority {
    /// The inline token (without `!`), or `None` for normal priority.
    pub fn token(self) -> Option<&'static str> {
        match self {
            Self::High => Some("wysoki"),
            Self::Normal => None,
            Self::Low => Some("niski"),
        }
    }

    /// Sort key: high first.
    pub fn rank(self) -> u8 {
        match self {
            Self::High => 0,
            Self::Normal => 1,
            Self::Low => 2,
        }
    }
}

/// `stan:` of a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProjectState {
    #[default]
    #[serde(alias = "aktywny")]
    Active,
    #[serde(alias = "wstrzymany")]
    Paused,
    #[serde(alias = "archiwum")]
    Archived,
}

impl ProjectState {
    pub fn file_value(self) -> &'static str {
        match self {
            Self::Active => "aktywny",
            Self::Paused => "wstrzymany",
            Self::Archived => "archiwum",
        }
    }

    pub fn from_file_value(s: &str) -> Option<Self> {
        match s {
            "aktywny" => Some(Self::Active),
            "wstrzymany" => Some(Self::Paused),
            "archiwum" => Some(Self::Archived),
            _ => None,
        }
    }
}

/// `status:` of a topic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TopicStatus {
    #[default]
    #[serde(alias = "planowany")]
    Planned,
    #[serde(alias = "w-toku")]
    InProgress,
    #[serde(alias = "zablokowany")]
    Blocked,
    #[serde(alias = "czeka")]
    Waiting,
    #[serde(alias = "zrobiony")]
    Done,
}

impl TopicStatus {
    pub fn file_value(self) -> &'static str {
        match self {
            Self::Planned => "planowany",
            Self::InProgress => "w-toku",
            Self::Blocked => "zablokowany",
            Self::Waiting => "czeka",
            Self::Done => "zrobiony",
        }
    }

    pub fn from_file_value(s: &str) -> Option<Self> {
        match s {
            "planowany" => Some(Self::Planned),
            "w-toku" => Some(Self::InProgress),
            "zablokowany" => Some(Self::Blocked),
            "czeka" => Some(Self::Waiting),
            "zrobiony" => Some(Self::Done),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Due date
// ---------------------------------------------------------------------------

/// Due date with optional time of day (`@2026-11-20` / `@2026-11-20T14:00`).
/// Serializes as the token text without `@`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Due {
    pub date: NaiveDate,
    pub time: Option<NaiveTime>,
}

impl Due {
    pub fn date(date: NaiveDate) -> Self {
        Due { date, time: None }
    }

    pub fn at(date: NaiveDate, time: NaiveTime) -> Self {
        Due {
            date,
            time: Some(time),
        }
    }
}

impl fmt::Display for Due {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.date.format("%Y-%m-%d"))?;
        if let Some(t) = self.time {
            write!(f, "T{}", t.format("%H:%M"))?;
        }
        Ok(())
    }
}

impl FromStr for Due {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (d, t) = match s.split_once('T') {
            Some((d, t)) => (d, Some(t)),
            None => (s, None),
        };
        let date = parse_date_strict(d).ok_or_else(|| format!("invalid date: {s}"))?;
        let time = match t {
            None => None,
            Some(t) => Some(parse_time_strict(t).ok_or_else(|| format!("invalid time: {s}"))?),
        };
        Ok(Due { date, time })
    }
}

impl Serialize for Due {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Due {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Strict `YYYY-MM-DD` (exactly 10 chars).
pub(crate) fn parse_date_strict(s: &str) -> Option<NaiveDate> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    if !b
        .iter()
        .enumerate()
        .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return None;
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// Strict `HH:MM` (exactly 5 chars).
pub(crate) fn parse_time_strict(s: &str) -> Option<NaiveTime> {
    let b = s.as_bytes();
    if b.len() != 5 || b[2] != b':' || !b[0..2].iter().chain(&b[3..5]).all(u8::is_ascii_digit) {
        return None;
    }
    NaiveTime::parse_from_str(s, "%H:%M").ok()
}

/// Minute-precision timestamp serde (`"YYYY-MM-DDTHH:MM"`). Deserialization also
/// accepts seconds and a space separator.
pub mod minute {
    use super::*;

    pub fn format(dt: &NaiveDateTime) -> String {
        dt.format("%Y-%m-%dT%H:%M").to_string()
    }

    pub fn parse(s: &str) -> Option<NaiveDateTime> {
        let s = s.trim();
        for fmt in [
            "%Y-%m-%dT%H:%M",
            "%Y-%m-%d %H:%M",
            "%Y-%m-%dT%H:%M:%S",
            "%Y-%m-%d %H:%M:%S",
        ] {
            if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
                return Some(dt);
            }
        }
        parse_date_strict(s).map(|d| d.and_time(NaiveTime::MIN))
    }

    pub fn serialize<S: Serializer>(dt: &NaiveDateTime, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format(dt))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveDateTime, D::Error> {
        let s = String::deserialize(d)?;
        parse(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid timestamp: {s}")))
    }

    pub mod option {
        use super::*;

        pub fn serialize<S: Serializer>(
            dt: &Option<NaiveDateTime>,
            s: S,
        ) -> Result<S::Ok, S::Error> {
            match dt {
                Some(dt) => s.serialize_str(&format(dt)),
                None => s.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            d: D,
        ) -> Result<Option<NaiveDateTime>, D::Error> {
            match Option::<String>::deserialize(d)? {
                None => Ok(None),
                Some(s) => parse(&s)
                    .map(Some)
                    .ok_or_else(|| serde::de::Error::custom(format!("invalid timestamp: {s}"))),
            }
        }
    }
}

/// Deserialize helper for "double option" patch fields: absent = `None` (keep),
/// `null` = `Some(None)` (clear), value = `Some(Some(v))` (set).
fn double_option<'de, T, D>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

// ---------------------------------------------------------------------------
// Entities
// ---------------------------------------------------------------------------

/// Where a task lives.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskLocation {
    /// `skrzynka.md`
    Inbox,
    /// `<project>/projekt.md`
    Project { project: String },
    /// `<project>/tematy/<topic>.md`
    Topic { project: String, topic: String },
}

impl TaskLocation {
    pub fn project(&self) -> Option<&str> {
        match self {
            TaskLocation::Inbox => None,
            TaskLocation::Project { project } | TaskLocation::Topic { project, .. } => {
                Some(project)
            }
        }
    }

    pub fn topic(&self) -> Option<&str> {
        match self {
            TaskLocation::Topic { topic, .. } => Some(topic),
            _ => None,
        }
    }

    /// Path of the file holding these tasks, relative to the vault root
    /// (`skrzynka.md`, `<p>/projekt.md`, `<p>/tematy/<t>.md`).
    pub fn rel_path(&self) -> PathBuf {
        match self {
            TaskLocation::Inbox => PathBuf::from(crate::vault::INBOX_FILE),
            TaskLocation::Project { project } => {
                PathBuf::from(format!("{project}/{}", crate::vault::PROJECT_FILE))
            }
            TaskLocation::Topic { project, topic } => {
                PathBuf::from(format!("{project}/{}/{topic}.md", crate::vault::TOPICS_DIR))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Stable 6-char base36 id (`^xxxxxx`). `None` only for hand-written tasks that
    /// have not been through [`crate::Vault::normalize`] yet; such tasks cannot be
    /// mutated by id.
    pub id: Option<String>,
    pub title: String,
    pub status: TaskStatus,
    pub priority: Priority,
    pub due: Option<Due>,
    /// Completion date (`✓YYYY-MM-DD`).
    pub done: Option<NaiveDate>,
    /// Indented lines under the task whose content is a bare URL.
    pub links: Vec<String>,
    /// All other indented lines under the task, joined with `\n`, with the first
    /// level of indentation removed. Empty string when there are none.
    pub notes: String,
    pub location: TaskLocation,
    /// 1-based line number of the task line in its file (informational).
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repo {
    /// `nazwa`
    pub name: String,
    /// `sciezka`
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// Directory name; the stable identifier of the project.
    pub slug: String,
    /// `nazwa` (falls back to the slug if missing).
    pub name: String,
    /// `kolor`, e.g. `"#a3e635"`.
    pub color: Option<String>,
    /// `stan`
    pub state: ProjectState,
    /// `repozytoria`
    pub repos: Vec<Repo>,
    /// Body text between the frontmatter and the first `##` heading, trimmed.
    pub description: String,
    /// `<slug>/projekt.md`
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Topic {
    pub project: String,
    /// File stem; the stable identifier of the topic within its project.
    pub slug: String,
    /// The `# H1` (falls back to the slug if missing).
    pub title: String,
    pub status: TopicStatus,
    /// Name of one of the project's repos.
    pub repo: Option<String>,
    /// Text between the H1 and the first `##` heading, trimmed.
    pub summary: String,
    /// `utworzono`
    pub created: Option<NaiveDate>,
    /// `zaktualizowano`
    #[serde(with = "minute::option")]
    pub updated: Option<NaiveDateTime>,
    /// `<project>/tematy/<slug>.md`
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub project: String,
    pub topic: String,
    #[serde(with = "minute")]
    pub at: NaiveDateTime,
    pub author: String,
    /// Entry text; indented continuation lines are joined with `\n`.
    pub text: String,
    /// 1-based line number (informational).
    pub line: usize,
}

/// A non-fatal problem found while loading. The vault keeps loading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadWarning {
    /// Relative to the vault root.
    pub path: PathBuf,
    /// 1-based line, when the problem is tied to one line.
    pub line: Option<usize>,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Mutation inputs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NewProject {
    pub name: String,
    pub color: Option<String>,
    pub state: ProjectState,
    pub repos: Vec<Repo>,
    pub description: String,
}

/// Fields left as `None` are not touched.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectPatch {
    pub name: Option<String>,
    /// `Some(None)` / JSON `null` removes the color.
    #[serde(
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub color: Option<Option<String>>,
    pub state: Option<ProjectState>,
    /// Replaces the whole repo list.
    pub repos: Option<Vec<Repo>>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NewTopic {
    pub title: String,
    pub status: TopicStatus,
    pub repo: Option<String>,
    pub summary: String,
}

/// Fields left as `None` are not touched.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TopicPatch {
    pub title: Option<String>,
    pub status: Option<TopicStatus>,
    /// `Some(None)` / JSON `null` removes the repo.
    #[serde(
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub repo: Option<Option<String>>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NewJournalEntry {
    /// Free text, e.g. `ja`, `agent`, or an agent/thread name. Empty → `ja`.
    pub author: String,
    pub text: String,
    /// Optionally change the topic status in the same write.
    pub status: Option<TopicStatus>,
    /// Optionally replace the topic summary in the same write.
    pub summary: Option<String>,
}

/// Result of [`crate::Vault::add_journal_entry`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalAdded {
    pub topic: Topic,
    pub entry: JournalEntry,
}

/// What [`crate::Vault::create_topic_with`] wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicCreated {
    pub topic: Topic,
    pub tasks: Vec<Task>,
    pub entry: Option<JournalEntry>,
    /// The file as written (for a later "still untouched?" check).
    pub contents: String,
}

/// Input of [`crate::Vault::revert_journal_entry`]: what one
/// `add_journal_entry` wrote, to take back while it is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalRevert {
    /// The entry as written (`at`, `author` and `text` are matched).
    pub entry: JournalEntry,
    /// `(written, before)`: restore `before` if the status is still `written`.
    pub status: Option<(TopicStatus, TopicStatus)>,
    /// `(written, before)`: restore `before` if the summary is still `written`.
    pub summary: Option<(String, String)>,
}

/// The `zaktualizowano` value a write replaced (`before`) and the one it set
/// (`written`), so that an undo can put the old value back instead of
/// bumping it: see [`crate::Vault::restoring`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    pub written: String,
    pub before: String,
}

/// What [`crate::Vault::revert_journal_entry`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalReverted {
    pub entry_removed: bool,
    pub status_restored: bool,
    pub summary_restored: bool,
    /// The topic as now written.
    pub topic: Option<Topic>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NewTask {
    pub title: String,
    pub status: TaskStatus,
    pub priority: Priority,
    pub due: Option<Due>,
    pub links: Vec<String>,
    pub notes: String,
}

/// Fields left as `None` / empty are not touched.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskPatch {
    pub title: Option<String>,
    /// Becoming `done` adds `✓<today>`; leaving `done` removes it.
    pub status: Option<TaskStatus>,
    pub priority: Option<Priority>,
    /// `Some(None)` / JSON `null` clears the due date.
    #[serde(
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub due: Option<Option<Due>>,
    /// URLs to add (duplicates of existing links are ignored).
    pub add_links: Vec<String>,
    /// URLs to remove (exact match).
    pub remove_links: Vec<String>,
    /// Replaces all note lines (links are kept). Empty string removes all notes.
    pub notes: Option<String>,
}

/// Result of [`crate::Vault::normalize`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizeReport {
    /// Number of tasks that received a new id (missing or duplicate).
    pub ids_assigned: usize,
    /// Files rewritten (relative paths).
    pub files_changed: Vec<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_roundtrip() {
        let d: Due = "2026-11-20".parse().unwrap();
        assert_eq!(d.time, None);
        assert_eq!(d.to_string(), "2026-11-20");
        let d: Due = "2026-11-20T09:05".parse().unwrap();
        assert_eq!(d.to_string(), "2026-11-20T09:05");
        assert!("2026-11-31".parse::<Due>().is_err());
        assert!("2026-11-20T25:00".parse::<Due>().is_err());
        assert!("2026-11-20T9:00".parse::<Due>().is_err());
        assert!("26-11-20".parse::<Due>().is_err());
    }

    #[test]
    fn enums_accept_polish_on_deserialize() {
        let s: TopicStatus = serde_json::from_str("\"w-toku\"").unwrap();
        assert_eq!(s, TopicStatus::InProgress);
        let s: TopicStatus = serde_json::from_str("\"in_progress\"").unwrap();
        assert_eq!(s, TopicStatus::InProgress);
        assert_eq!(serde_json::to_string(&s).unwrap(), "\"in_progress\"");
        let p: Priority = serde_json::from_str("\"wysoki\"").unwrap();
        assert_eq!(p, Priority::High);
        let st: ProjectState = serde_json::from_str("\"archiwum\"").unwrap();
        assert_eq!(st, ProjectState::Archived);
    }

    #[test]
    fn task_patch_double_option() {
        let p: TaskPatch = serde_json::from_str("{}").unwrap();
        assert_eq!(p.due, None);
        let p: TaskPatch = serde_json::from_str("{\"due\":null}").unwrap();
        assert_eq!(p.due, Some(None));
        let p: TaskPatch = serde_json::from_str("{\"due\":\"2026-01-02\"}").unwrap();
        assert_eq!(
            p.due,
            Some(Some(Due::date(
                NaiveDate::from_ymd_opt(2026, 1, 2).unwrap()
            )))
        );
    }

    #[test]
    fn location_json_shape() {
        let l = TaskLocation::Topic {
            project: "a".into(),
            topic: "b".into(),
        };
        assert_eq!(
            serde_json::to_string(&l).unwrap(),
            r#"{"kind":"topic","project":"a","topic":"b"}"#
        );
        let i: TaskLocation = serde_json::from_str(r#"{"kind":"inbox"}"#).unwrap();
        assert_eq!(i, TaskLocation::Inbox);
    }

    #[test]
    fn minute_parse() {
        assert!(minute::parse("2026-11-13T14:02").is_some());
        assert!(minute::parse("2026-11-13 14:02").is_some());
        assert!(minute::parse("2026-11-13").is_some());
        assert!(minute::parse("nope").is_none());
    }
}
