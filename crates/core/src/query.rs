//! In-memory snapshot of a vault and read-only queries over it.

use std::path::PathBuf;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::model::*;
use crate::slug::fold;

/// Everything loaded from a vault at one point in time. Produced by
/// [`crate::Vault::load`]; never written back.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub root: PathBuf,
    /// Sorted by slug.
    pub projects: Vec<Project>,
    /// Sorted by project slug, then topic slug.
    pub topics: Vec<Topic>,
    /// File order: inbox first, then per project: `projekt.md`, then topics.
    pub tasks: Vec<Task>,
    /// File order (each topic newest first, as written).
    pub journal: Vec<JournalEntry>,
    pub warnings: Vec<LoadWarning>,
}

/// Filter for [`Snapshot::tasks`]. All set criteria must match.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskFilter {
    /// Tasks of this project: its `projekt.md` and all its topics.
    pub project: Option<String>,
    /// Tasks of this topic (combine with `project`; alone it matches the topic slug
    /// in any project).
    pub topic: Option<String>,
    /// Only inbox tasks.
    pub inbox_only: bool,
    /// Allowed statuses; empty = any.
    pub statuses: Vec<TaskStatus>,
    /// Inclusive lower bound on the due date. Tasks without a due date are
    /// excluded when either bound is set.
    pub due_from: Option<NaiveDate>,
    /// Inclusive upper bound on the due date.
    pub due_to: Option<NaiveDate>,
    /// Include done tasks (also included when `statuses` contains `done`).
    pub include_done: bool,
}

impl TaskFilter {
    pub fn matches(&self, t: &Task) -> bool {
        if self.inbox_only && t.location != TaskLocation::Inbox {
            return false;
        }
        if let Some(p) = &self.project
            && t.location.project() != Some(p.as_str())
        {
            return false;
        }
        if let Some(tp) = &self.topic
            && t.location.topic() != Some(tp.as_str())
        {
            return false;
        }
        if !self.statuses.is_empty() && !self.statuses.contains(&t.status) {
            return false;
        }
        if t.status.is_done() && !self.include_done && !self.statuses.contains(&TaskStatus::Done) {
            return false;
        }
        if self.due_from.is_some() || self.due_to.is_some() {
            let Some(due) = t.due else { return false };
            if self.due_from.is_some_and(|f| due.date < f)
                || self.due_to.is_some_and(|to| due.date > to)
            {
                return false;
            }
        }
        true
    }
}

/// Why a task is in the "today" view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodayReason {
    Overdue,
    DueToday,
    Doing,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodayItem {
    pub reason: TodayReason,
    pub task: Task,
}

/// One search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SearchHit {
    Task { task: Task },
    Topic { topic: Topic },
    Journal { entry: JournalEntry },
}

fn matches_all(terms: &[String], fields: &[&str]) -> bool {
    let hay = fold(&fields.join("\n"));
    terms.iter().all(|t| hay.contains(t.as_str()))
}

impl Snapshot {
    pub fn project(&self, slug: &str) -> Option<&Project> {
        self.projects.iter().find(|p| p.slug == slug)
    }

    pub fn topic(&self, project: &str, slug: &str) -> Option<&Topic> {
        self.topics
            .iter()
            .find(|t| t.project == project && t.slug == slug)
    }

    pub fn topics_of<'a>(&'a self, project: &'a str) -> impl Iterator<Item = &'a Topic> + 'a {
        self.topics.iter().filter(move |t| t.project == project)
    }

    /// Task by id (`^` prefix optional).
    pub fn task(&self, id: &str) -> Option<&Task> {
        let id = id.trim_start_matches('^');
        self.tasks.iter().find(|t| t.id.as_deref() == Some(id))
    }

    /// Tasks matching `filter`, in snapshot (file) order.
    pub fn tasks(&self, filter: &TaskFilter) -> Vec<Task> {
        self.tasks
            .iter()
            .filter(|t| filter.matches(t))
            .cloned()
            .collect()
    }

    /// Journal of one topic, newest first.
    pub fn journal_of(&self, project: &str, topic: &str) -> Vec<JournalEntry> {
        let mut v: Vec<JournalEntry> = self
            .journal
            .iter()
            .filter(|j| j.project == project && j.topic == topic)
            .cloned()
            .collect();
        v.sort_by(|a, b| b.at.cmp(&a.at).then(a.line.cmp(&b.line)));
        v
    }

    /// The most recent `limit` journal entries across the vault, newest first.
    pub fn recent_journal(&self, limit: usize) -> Vec<JournalEntry> {
        let mut v = self.journal.clone();
        v.sort_by(|a, b| {
            b.at.cmp(&a.at)
                .then_with(|| a.project.cmp(&b.project))
                .then_with(|| a.topic.cmp(&b.topic))
                .then(a.line.cmp(&b.line))
        });
        v.truncate(limit);
        v
    }

    /// Open tasks that need attention on `today`: overdue, due today, doing,
    /// blocked. Each task appears once, under the first matching reason in that
    /// order. Done tasks and tasks of archived projects are excluded. Sorted by
    /// reason, then due date/time, then priority.
    pub fn today(&self, today: NaiveDate) -> Vec<TodayItem> {
        let archived = |t: &Task| {
            t.location
                .project()
                .and_then(|p| self.project(p))
                .is_some_and(|p| p.state == ProjectState::Archived)
        };
        let mut items: Vec<TodayItem> = self
            .tasks
            .iter()
            .filter(|t| !t.status.is_done() && !archived(t))
            .filter_map(|t| {
                let reason = match (t.due, t.status) {
                    (Some(d), _) if d.date < today => TodayReason::Overdue,
                    (Some(d), _) if d.date == today => TodayReason::DueToday,
                    (_, TaskStatus::Doing) => TodayReason::Doing,
                    (_, TaskStatus::Blocked) => TodayReason::Blocked,
                    _ => return None,
                };
                Some(TodayItem {
                    reason,
                    task: t.clone(),
                })
            })
            .collect();
        items.sort_by(|a, b| {
            a.reason
                .cmp(&b.reason)
                .then_with(|| match (a.task.due, b.task.due) {
                    (Some(x), Some(y)) => x.cmp(&y),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                })
                .then(a.task.priority.rank().cmp(&b.task.priority.rank()))
        });
        items
    }

    /// Case- and diacritic-insensitive search. Every whitespace-separated term
    /// must occur in the item. Searches task titles + notes, topic titles +
    /// summaries, journal text + author. Results: tasks, then topics, then
    /// journal entries (newest first).
    pub fn search(&self, query: &str) -> Vec<SearchHit> {
        let terms: Vec<String> = fold(query).split_whitespace().map(str::to_string).collect();
        if terms.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<SearchHit> = Vec::new();
        hits.extend(
            self.tasks
                .iter()
                .filter(|t| matches_all(&terms, &[&t.title, &t.notes]))
                .map(|t| SearchHit::Task { task: t.clone() }),
        );
        hits.extend(
            self.topics
                .iter()
                .filter(|t| matches_all(&terms, &[&t.title, &t.summary]))
                .map(|t| SearchHit::Topic { topic: t.clone() }),
        );
        let mut journal: Vec<&JournalEntry> = self
            .journal
            .iter()
            .filter(|j| matches_all(&terms, &[&j.text, &j.author]))
            .collect();
        journal.sort_by_key(|j| std::cmp::Reverse(j.at));
        hits.extend(
            journal
                .into_iter()
                .map(|j| SearchHit::Journal { entry: j.clone() }),
        );
        hits
    }
}
