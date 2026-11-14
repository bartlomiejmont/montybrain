//! Applying accepted changes of a proposal to the vault, through the core API
//! (every mutation takes the vault lock, validates the text again and
//! verifies the structure of what it writes), and undoing one apply click.
//!
//! Before each change the vault is re-read and the change is checked against
//! the state it was based on ([`proposal::check_current`]); a change whose
//! target changed in the meantime is skipped as stale. A change is applied at
//! most once: its outcome is recorded in the proposal.
//!
//! Undo ([`undo`]) takes back only what one click wrote, and only what is
//! still exactly what was written; each check runs inside the vault lock
//! (core's guarded mutations `update_task_if`, `delete_task_if`,
//! `revert_journal_entry`). Anything that changed since is left alone. Each
//! write records the file's `zaktualizowano` before and after, so a full undo
//! puts the old value back ([`Vault::restoring`]) and the file reads as
//! untouched; writes are taken back in reverse order.

use montybrain_core::chrono::NaiveDateTime;
use montybrain_core::{
    JournalEntry, JournalRevert, NewJournalEntry, NewTask, NewTopic, Priority, Stamp, Task,
    TaskLocation, TaskPatch, TaskStatus, TopicStatus, Vault,
};
use serde::{Deserialize, Serialize};

use super::store::{ChangeOutcome, ChangeStatus, StoredProposal};
use crate::drift::links::BranchKey;
use crate::runs::ProviderId;
use crate::runs::proposal::{self, Change};

/// The journal author of entries an agent proposed.
pub fn author(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Claude => "claude-code",
        ProviderId::Codex => "codex",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyStatus {
    Applied,
    /// The target changed since the proposal.
    Stale,
    /// Applied before; not applied again.
    Already,
    Failed,
    /// No change with this index.
    Missing,
}

/// The result of one requested change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApplyItem {
    pub index: usize,
    pub status: ApplyStatus,
    pub message: String,
}

/// What one applied change wrote, enough to take it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UndoItem {
    TaskStatus {
        index: usize,
        task: String,
        /// The status before.
        from: TaskStatus,
        /// The status written.
        to: TaskStatus,
        /// The file's `zaktualizowano` before / as written.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stamp: Option<Stamp>,
    },
    NewTask {
        index: usize,
        task: String,
        title: String,
        location: TaskLocation,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stamp: Option<Stamp>,
    },
    TopicLog {
        index: usize,
        topic: String,
        entry: JournalEntry,
        /// `(written, before)`.
        status: Option<(TopicStatus, TopicStatus)>,
        summary: Option<(String, String)>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stamp: Option<Stamp>,
        /// Branch links stored with it (`.montybrain/links.json`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        links: Vec<proposal::BranchRef>,
    },
    NewTopic {
        index: usize,
        /// The slug the topic got.
        topic: String,
        /// [`proposal::content_hash`] of the file as written.
        hash: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        links: Vec<proposal::BranchRef>,
    },
    /// "Odrzuć wszystkie / pozostałe": nothing was written; undoing makes the
    /// change undecided again (its user corrections kept).
    Rejected { index: usize },
}

impl UndoItem {
    fn set_stamp(&mut self, value: Option<Stamp>) {
        match self {
            UndoItem::TaskStatus { stamp, .. }
            | UndoItem::NewTask { stamp, .. }
            | UndoItem::TopicLog { stamp, .. } => *stamp = value,
            UndoItem::NewTopic { .. } | UndoItem::Rejected { .. } => {}
        }
    }

    fn stamp(&self) -> Option<Stamp> {
        match self {
            UndoItem::TaskStatus { stamp, .. }
            | UndoItem::NewTask { stamp, .. }
            | UndoItem::TopicLog { stamp, .. } => stamp.clone(),
            UndoItem::NewTopic { .. } | UndoItem::Rejected { .. } => None,
        }
    }

    /// Whether undoing it writes to the vault (a rejection does not).
    pub fn writes(&self) -> bool {
        !matches!(self, UndoItem::Rejected { .. })
    }

    pub fn index(&self) -> usize {
        match self {
            UndoItem::TaskStatus { index, .. }
            | UndoItem::NewTask { index, .. }
            | UndoItem::TopicLog { index, .. }
            | UndoItem::NewTopic { index, .. }
            | UndoItem::Rejected { index } => *index,
        }
    }
}

/// The writes of one apply click (kept until undone or replaced).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoRecord {
    pub token: String,
    pub run_id: String,
    #[serde(with = "montybrain_core::minute")]
    pub at: NaiveDateTime,
    pub items: Vec<UndoItem>,
}

/// Stores the branches a topic change names as links to its topic, and keeps
/// them on the undo item (an undo removes them while they still point there).
fn link_branches(vault: &Vault, project: &str, change: &Change, undo: &mut UndoItem) {
    let (branches, topic, links) = match (change, undo) {
        (Change::TopicLog { branches, .. }, UndoItem::TopicLog { topic, links, .. })
        | (Change::NewTopic { branches, .. }, UndoItem::NewTopic { topic, links, .. }) => {
            (branches, topic.clone(), links)
        }
        _ => return,
    };
    if branches.is_empty() {
        return;
    }
    let r = crate::drift::links::update(vault.root(), |s| {
        let mut changed = false;
        for b in branches {
            changed |= s.link(&BranchKey::new(project, &b.repo, &b.branch), &topic);
        }
        changed
    });
    match r {
        Ok(_) => *links = branches.clone(),
        Err(e) => crate::log(&format!("links: {e}")),
    }
}

fn unlink_branches(vault: &Vault, project: &str, item: &UndoItem) {
    let (topic, links) = match item {
        UndoItem::TopicLog { topic, links, .. } | UndoItem::NewTopic { topic, links, .. } => {
            (topic, links)
        }
        _ => return,
    };
    if links.is_empty() {
        return;
    }
    if let Err(e) = crate::drift::links::update(vault.root(), |s| {
        let mut changed = false;
        for b in links {
            changed |= s.unlink(&BranchKey::new(project, &b.repo, &b.branch), topic);
        }
        changed
    }) {
        crate::log(&format!("links: {e}"));
    }
}

/// The `zaktualizowano` of the file `change` writes (none for a new topic:
/// its undo removes the file).
fn file_stamp(vault: &Vault, project: &str, change: &Change) -> Option<String> {
    match change {
        Change::TaskStatus { task, .. } => vault.stamp_of_task(task).ok().flatten(),
        Change::TopicLog { .. } | Change::NewTask { .. } => {
            vault.stamp_of(&change.location(project)).ok().flatten()
        }
        Change::NewTopic { .. } => None,
    }
}

fn run_change(
    vault: &Vault,
    project: &str,
    author: &str,
    change: &Change,
    index: usize,
) -> Result<(String, UndoItem), String> {
    match change {
        Change::TaskStatus { task, to, from, .. } => vault
            .update_task(
                task,
                TaskPatch {
                    status: Some(*to),
                    ..Default::default()
                },
            )
            .map(|t| {
                (
                    format!("Zadanie „{}” zmienione.", t.title),
                    UndoItem::TaskStatus {
                        index,
                        task: task.clone(),
                        from: *from,
                        to: t.status,
                        stamp: None,
                    },
                )
            }),
        Change::TopicLog {
            topic,
            text,
            status,
            summary,
            base_status,
            base_summary,
            ..
        } => vault
            .add_journal_entry(
                project,
                topic,
                NewJournalEntry {
                    author: author.into(),
                    text: text.clone(),
                    status: *status,
                    summary: summary.clone(),
                },
            )
            .map(|j| {
                (
                    format!("Dopisano wpis w temacie „{}”.", j.topic.title),
                    UndoItem::TopicLog {
                        index,
                        topic: topic.clone(),
                        status: status.map(|_| (j.topic.status, *base_status)),
                        summary: summary
                            .as_ref()
                            .map(|_| (j.topic.summary.clone(), base_summary.clone())),
                        entry: j.entry,
                        stamp: None,
                        links: Vec::new(),
                    },
                )
            }),
        Change::NewTopic {
            title,
            status,
            summary,
            repo,
            text,
            tasks,
            ..
        } => vault
            .create_topic_with(
                project,
                NewTopic {
                    title: title.clone(),
                    status: *status,
                    repo: repo.clone(),
                    summary: summary.clone(),
                },
                Some(NewJournalEntry {
                    author: author.into(),
                    text: text.clone(),
                    ..Default::default()
                }),
                tasks
                    .iter()
                    .map(|t| NewTask {
                        title: t.title.clone(),
                        status: t.status,
                        ..Default::default()
                    })
                    .collect(),
            )
            .map(|c| {
                (
                    format!("Dodano temat „{}”.", c.topic.title),
                    UndoItem::NewTopic {
                        index,
                        hash: proposal::content_hash(&c.contents),
                        topic: c.topic.slug,
                        links: Vec::new(),
                    },
                )
            }),
        Change::NewTask { title, .. } => {
            let location = change.location(project);
            vault
                .create_task(
                    &location,
                    NewTask {
                        title: title.clone(),
                        ..Default::default()
                    },
                )
                .map(|t| {
                    (
                        format!("Dodano zadanie „{}”.", t.title),
                        UndoItem::NewTask {
                            index,
                            task: t.id.clone().unwrap_or_default(),
                            title: t.title,
                            location,
                            stamp: None,
                        },
                    )
                })
        }
    }
    .map_err(|e| e.to_string())
}

/// Applies the changes `indices` (0-based, in the order given, duplicates
/// once) of `proposal` for `project`, recording each outcome in it. Returns
/// the per-change results and what was written (for [`undo`]).
pub fn apply(
    vault: &Vault,
    project: &str,
    proposal: &mut StoredProposal,
    indices: &[usize],
    now: NaiveDateTime,
) -> (Vec<ApplyItem>, Vec<UndoItem>) {
    let author = author(proposal.provider);
    let mut done: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    let mut written = Vec::new();
    for &index in indices {
        if done.contains(&index) {
            continue;
        }
        done.push(index);
        let item = |status, message: String| ApplyItem {
            index,
            status,
            message,
        };
        let Some(stored) = proposal.changes.get_mut(index) else {
            out.push(item(
                ApplyStatus::Missing,
                format!("Nie ma zmiany nr {}.", index + 1),
            ));
            continue;
        };
        if stored.applied() {
            out.push(item(ApplyStatus::Already, "Już zastosowane.".into()));
            continue;
        }
        let (status, message) = match vault.load() {
            Err(e) => (
                ChangeStatus::Failed,
                format!("Nie udało się wczytać MontyBrain: {e}"),
            ),
            Ok(snap) => match proposal::check_current(&stored.change, &snap, project) {
                Err(stale) => (ChangeStatus::Stale, stale),
                Ok(()) => {
                    let before = file_stamp(vault, project, &stored.change);
                    match run_change(vault, project, author, &stored.change, index) {
                        Ok((msg, mut undo)) => {
                            let after = file_stamp(vault, project, &stored.change);
                            link_branches(vault, project, &stored.change, &mut undo);
                            if let (Some(before), Some(written)) = (before, after)
                                && before != written
                            {
                                undo.set_stamp(Some(Stamp { written, before }));
                            }
                            written.push(undo);
                            (ChangeStatus::Applied, msg)
                        }
                        Err(e) => (ChangeStatus::Failed, e),
                    }
                }
            },
        };
        stored.outcome = Some(ChangeOutcome {
            status,
            message: message.clone(),
            at: now,
        });
        out.push(item(
            match status {
                ChangeStatus::Applied => ApplyStatus::Applied,
                ChangeStatus::Stale => ApplyStatus::Stale,
                ChangeStatus::Failed | ChangeStatus::Rejected => ApplyStatus::Failed,
            },
            message,
        ));
    }
    (out, written)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoStatus {
    /// Everything this change wrote was taken back.
    Undone,
    /// Part of it was taken back; the rest changed in the meantime.
    Partial,
    /// It changed in the meantime: left alone.
    Changed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UndoResult {
    pub index: usize,
    pub status: UndoStatus,
    pub message: String,
}

/// A task created by an apply is "untouched" while it is still exactly as created.
fn untouched(t: &Task, title: &str, location: &TaskLocation) -> bool {
    t.title == title
        && t.status == TaskStatus::Todo
        && t.priority == Priority::Normal
        && t.due.is_none()
        && t.links.is_empty()
        && t.notes.trim().is_empty()
        && t.location == *location
}

fn undo_one(vault: &Vault, project: &str, item: &UndoItem) -> (UndoStatus, String) {
    let changed = |what: &str| {
        (
            UndoStatus::Changed,
            format!("{what} zmieniło się w międzyczasie; zostawione bez zmian."),
        )
    };
    let failed = |e: montybrain_core::Error| (UndoStatus::Failed, e.to_string());
    match item {
        UndoItem::TaskStatus { task, from, to, .. } => {
            let patch = TaskPatch {
                status: Some(*from),
                ..Default::default()
            };
            match vault.update_task_if(task, |t| t.status == *to, patch) {
                Ok(Some(_)) => (UndoStatus::Undone, "Przywrócono poprzedni status.".into()),
                Ok(None) => changed("Zadanie"),
                Err(montybrain_core::Error::TaskNotFound(_)) => changed("Zadanie"),
                Err(e) => failed(e),
            }
        }
        UndoItem::NewTopic { topic, hash, .. } => {
            match vault
                .delete_topic_if(project, topic, |text| proposal::content_hash(text) == *hash)
            {
                Ok(Some(_)) => (UndoStatus::Undone, "Usunięto dodany temat.".into()),
                Ok(None) | Err(montybrain_core::Error::TopicNotFound { .. }) => changed("Temat"),
                Err(e) => failed(e),
            }
        }
        UndoItem::NewTask {
            task,
            title,
            location,
            ..
        } => match vault.delete_task_if(task, |t| untouched(t, title, location)) {
            Ok(Some(_)) => (UndoStatus::Undone, "Usunięto dodane zadanie.".into()),
            Ok(None) | Err(montybrain_core::Error::TaskNotFound(_)) => changed("Zadanie"),
            Err(e) => failed(e),
        },
        UndoItem::Rejected { .. } => (UndoStatus::Changed, String::new()),
        UndoItem::TopicLog {
            topic,
            entry,
            status,
            summary,
            ..
        } => {
            let revert = JournalRevert {
                entry: entry.clone(),
                status: *status,
                summary: summary.clone(),
            };
            match vault.revert_journal_entry(project, topic, revert) {
                Ok(r) => {
                    let wanted = 1 + usize::from(status.is_some()) + usize::from(summary.is_some());
                    let got = usize::from(r.entry_removed)
                        + usize::from(r.status_restored)
                        + usize::from(r.summary_restored);
                    if got == wanted {
                        (UndoStatus::Undone, "Usunięto wpis z dziennika.".into())
                    } else if got == 0 {
                        changed("Temat")
                    } else {
                        let mut left = Vec::new();
                        if !r.entry_removed {
                            left.push("wpis");
                        }
                        if status.is_some() && !r.status_restored {
                            left.push("status");
                        }
                        if summary.is_some() && !r.summary_restored {
                            left.push("opis");
                        }
                        (
                            UndoStatus::Partial,
                            format!(
                                "Cofnięto częściowo; zmienione w międzyczasie i zostawione: {}.",
                                left.join(", ")
                            ),
                        )
                    }
                }
                Err(montybrain_core::Error::TopicNotFound { .. }) => changed("Temat"),
                Err(e) => failed(e),
            }
        }
    }
}

/// The rejection of the undecided changes of `p` as an undo record's items
/// (marks them rejected).
pub fn reject(p: &mut StoredProposal, now: NaiveDateTime) -> Vec<UndoItem> {
    let mut out = Vec::new();
    for (index, c) in p.changes.iter_mut().enumerate() {
        if c.outcome.is_none() {
            c.outcome = Some(ChangeOutcome {
                status: ChangeStatus::Rejected,
                message: "Odrzucone.".into(),
                at: now,
            });
            out.push(UndoItem::Rejected { index });
        }
    }
    out
}

/// Takes back what `record` wrote, where it still matches. Changes fully
/// taken back become undecided again in `proposal` (a rejection: while the
/// change is still rejected).
pub fn undo(
    vault: &Vault,
    project: &str,
    record: &UndoRecord,
    mut proposal: Option<&mut StoredProposal>,
) -> Vec<UndoResult> {
    let rejected_now = |index: usize, p: &Option<&mut StoredProposal>| {
        p.as_ref()
            .filter(|p| p.run_id == record.run_id)
            .is_some_and(|p| {
                p.changes
                    .get(index)
                    .and_then(|c| c.outcome.as_ref())
                    .is_some_and(|o| o.status == ChangeStatus::Rejected)
            })
    };
    let rejected: Vec<(usize, bool)> = record
        .items
        .iter()
        .filter_map(|i| match i {
            UndoItem::Rejected { index } => Some((*index, rejected_now(*index, &proposal))),
            _ => None,
        })
        .collect();
    let proposal = proposal.take();
    // Newest write first, so each one finds the file as it left it (and the
    // `zaktualizowano` it set).
    let mut results: Vec<UndoResult> = record
        .items
        .iter()
        .rev()
        .map(|item| {
            let (status, message) = match item {
                UndoItem::Rejected { index } => {
                    if rejected.contains(&(*index, true)) {
                        (UndoStatus::Undone, "Przywrócono propozycję.".into())
                    } else {
                        (
                            UndoStatus::Changed,
                            "Propozycja nie jest już odrzucona.".into(),
                        )
                    }
                }
                _ => {
                    let r = undo_one(&vault.restoring(item.stamp()), project, item);
                    if matches!(r.0, UndoStatus::Undone | UndoStatus::Partial) {
                        unlink_branches(vault, project, item);
                    }
                    r
                }
            };
            UndoResult {
                index: item.index(),
                status,
                message,
            }
        })
        .collect();
    results.reverse();
    if let Some(p) = proposal.filter(|p| p.run_id == record.run_id) {
        for r in &results {
            if let Some(c) = p.changes.get_mut(r.index) {
                match r.status {
                    UndoStatus::Undone => c.outcome = None,
                    UndoStatus::Partial => {
                        if let Some(o) = c.outcome.as_mut() {
                            o.message = r.message.clone();
                        }
                    }
                    UndoStatus::Changed | UndoStatus::Failed => {}
                }
            }
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updates::store::TriggerKind;
    use montybrain_core::{NewProject, NewTopic, TaskLocation, TaskStatus, TopicStatus};

    struct Fixture {
        _dir: tempfile::TempDir,
        vault: Vault,
        open: String,
        other: String,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::init(dir.path()).unwrap();
        vault
            .create_project(NewProject {
                name: "Beeking".into(),
                ..Default::default()
            })
            .unwrap();
        vault
            .create_project(NewProject {
                name: "Dom".into(),
                ..Default::default()
            })
            .unwrap();
        vault
            .create_topic(
                "beeking",
                NewTopic {
                    title: "Płatności".into(),
                    status: TopicStatus::InProgress,
                    summary: "Stan.".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let loc = TaskLocation::Topic {
            project: "beeking".into(),
            topic: "platnosci".into(),
        };
        let open = vault
            .create_task(
                &loc,
                NewTask {
                    title: "Webhook zwrotów".into(),
                    status: TaskStatus::Doing,
                    ..Default::default()
                },
            )
            .unwrap()
            .id
            .unwrap();
        let other = vault
            .create_task(
                &TaskLocation::Project {
                    project: "dom".into(),
                },
                NewTask {
                    title: "Blat".into(),
                    ..Default::default()
                },
            )
            .unwrap()
            .id
            .unwrap();
        Fixture {
            _dir: dir,
            vault,
            open,
            other,
        }
    }

    fn at(s: &str) -> NaiveDateTime {
        montybrain_core::minute::parse(s).unwrap()
    }

    fn proposal_for(f: &Fixture, provider: ProviderId) -> StoredProposal {
        let json = format!(
            r#"{{"changes":[
  {{"kind":"task_status","task":"{}","status":"done","reason":"Commit abc."}},
  {{"kind":"topic_log","topic":"platnosci","text":"Zwroty gotowe.","status":"waiting"}},
  {{"kind":"new_task","title":"Testy BLIK","topic":"platnosci","reason":"Brak testów."}},
  {{"kind":"task_status","task":"{}","status":"done","reason":"Spoza projektu."}}
]}}"#,
            f.open, f.other
        );
        let answer = format!("Raport.\n```{}\n{json}\n```", proposal::FENCE_INFO);
        let snap = f.vault.load().unwrap();
        let p = proposal::parse(&answer, &snap, "beeking");
        assert_eq!(p.changes.len(), 3, "{p:?}");
        assert_eq!(p.dropped.len(), 1, "the other project's task is dropped");
        StoredProposal::from_proposal(
            p,
            "r1",
            provider,
            TriggerKind::Manual,
            at("2026-11-14T14:00"),
        )
    }

    #[test]
    fn applies_selected_changes_once_with_the_provider_as_author() {
        let f = fixture();
        let mut p = proposal_for(&f, ProviderId::Codex);
        let now = at("2026-11-14T14:05");
        let (r, _) = apply(&f.vault, "beeking", &mut p, &[1, 0, 0, 9], now);
        let statuses: Vec<_> = r.iter().map(|i| (i.index, i.status)).collect();
        assert_eq!(
            statuses,
            [
                (1, ApplyStatus::Applied),
                (0, ApplyStatus::Applied),
                (9, ApplyStatus::Missing)
            ]
        );
        let snap = f.vault.load().unwrap();
        assert_eq!(snap.task(&f.open).unwrap().status, TaskStatus::Done);
        let topic = snap.topic("beeking", "platnosci").unwrap();
        assert_eq!(topic.status, TopicStatus::Waiting);
        let entry = &snap.journal_of("beeking", "platnosci")[0];
        assert_eq!(
            (entry.author.as_str(), entry.text.as_str()),
            ("codex", "Zwroty gotowe.")
        );
        assert_eq!(p.applied(), 2);
        assert_eq!(p.pending(), 1);

        // Again: already applied, nothing written twice.
        let (r, _) = apply(&f.vault, "beeking", &mut p, &[0, 1, 2], now);
        assert_eq!(
            r.iter().map(|i| i.status).collect::<Vec<_>>(),
            [
                ApplyStatus::Already,
                ApplyStatus::Already,
                ApplyStatus::Applied
            ]
        );
        let snap = f.vault.load().unwrap();
        assert_eq!(snap.journal_of("beeking", "platnosci").len(), 1);
        let new: Vec<_> = snap
            .tasks
            .iter()
            .filter(|t| t.title == "Testy BLIK")
            .collect();
        assert_eq!(new.len(), 1);
        assert_eq!(
            apply(&f.vault, "beeking", &mut p, &[2], now).0[0].status,
            ApplyStatus::Already
        );
        assert_eq!(author(ProviderId::Claude), "claude-code");
    }

    #[test]
    fn changes_whose_target_changed_are_skipped() {
        let f = fixture();
        let mut p = proposal_for(&f, ProviderId::Claude);
        // Meanwhile: the task got blocked, the topic summary was edited, and
        // someone added the proposed task by hand.
        f.vault
            .update_task(
                &f.open,
                TaskPatch {
                    status: Some(TaskStatus::Blocked),
                    ..Default::default()
                },
            )
            .unwrap();
        f.vault
            .update_topic(
                "beeking",
                "platnosci",
                montybrain_core::TopicPatch {
                    summary: Some("Nowy stan.".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        f.vault
            .create_task(
                &TaskLocation::Topic {
                    project: "beeking".into(),
                    topic: "platnosci".into(),
                },
                NewTask {
                    title: "testy blik".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let (r, _) = apply(
            &f.vault,
            "beeking",
            &mut p,
            &[0, 1, 2],
            at("2026-11-14T15:00"),
        );
        assert!(r.iter().all(|i| i.status == ApplyStatus::Stale), "{r:#?}");
        assert!(r.iter().all(|i| i.message.starts_with(proposal::STALE)));
        let snap = f.vault.load().unwrap();
        assert_eq!(snap.task(&f.open).unwrap().status, TaskStatus::Blocked);
        assert!(snap.journal_of("beeking", "platnosci").is_empty());
        assert_eq!(p.applied(), 0);
        assert_eq!(p.pending(), 0, "outcomes recorded");
        assert!(
            p.changes
                .iter()
                .all(|c| c.outcome.as_ref().unwrap().status == ChangeStatus::Stale)
        );
    }

    #[test]
    fn core_still_validates_what_it_writes() {
        let f = fixture();
        let mut p = proposal_for(&f, ProviderId::Claude);
        // Tampered stored proposal (e.g. a hand-edited updates.json): core refuses.
        if let Change::TopicLog { summary, .. } = &mut p.changes[1].change {
            *summary = Some("a\n## Zadania\n- [ ] x".into());
        }
        let (r, _) = apply(&f.vault, "beeking", &mut p, &[1], at("2026-11-14T15:00"));
        assert_eq!(r[0].status, ApplyStatus::Failed, "{r:?}");
        assert!(r[0].message.contains("summary"), "{}", r[0].message);
        let snap = f.vault.load().unwrap();
        assert!(snap.journal_of("beeking", "platnosci").is_empty());
    }

    fn record(items: Vec<UndoItem>) -> UndoRecord {
        UndoRecord {
            token: "r1#1".into(),
            run_id: "r1".into(),
            at: at("2026-11-14T14:05"),
            items,
        }
    }

    #[test]
    fn undo_takes_back_each_kind_exactly() {
        let f = fixture();
        let dir = f.vault.root().to_path_buf();
        let read_all = || {
            let mut v: Vec<(String, String)> = Vec::new();
            for rel in ["beeking/projekt.md", "beeking/tematy/platnosci.md"] {
                let text = std::fs::read_to_string(dir.join(rel)).unwrap();
                // `zaktualizowano` is the one line an undo does not restore.
                let text: Vec<&str> = text
                    .lines()
                    .filter(|l| !l.starts_with("zaktualizowano:"))
                    .collect();
                v.push((rel.into(), text.join("\n")));
            }
            v
        };
        let before = read_all();
        let mut p = proposal_for(&f, ProviderId::Codex);
        let (_, written) = apply(
            &f.vault,
            "beeking",
            &mut p,
            &[0, 1, 2],
            at("2026-11-14T14:05"),
        );
        assert_eq!(written.len(), 3);
        assert_eq!(p.applied(), 3);
        let r = undo(&f.vault, "beeking", &record(written), Some(&mut p));
        assert!(r.iter().all(|x| x.status == UndoStatus::Undone), "{r:#?}");
        assert_eq!(read_all(), before, "the vault is as before the click");
        assert_eq!(p.pending(), 3, "undone changes can be decided again");
        let snap = f.vault.load().unwrap();
        assert_eq!(snap.task(&f.open).unwrap().status, TaskStatus::Doing);
        assert_eq!(
            snap.topic("beeking", "platnosci").unwrap().status,
            TopicStatus::InProgress
        );
    }

    #[test]
    fn undo_leaves_what_changed_in_the_meantime() {
        let f = fixture();
        let mut p = proposal_for(&f, ProviderId::Claude);
        let (_, written) = apply(
            &f.vault,
            "beeking",
            &mut p,
            &[0, 1, 2],
            at("2026-11-14T14:05"),
        );
        let snap = f.vault.load().unwrap();
        let new_id = snap
            .tasks
            .iter()
            .find(|t| t.title == "Testy BLIK")
            .and_then(|t| t.id.clone())
            .unwrap();
        // Meanwhile: the task got blocked, the new task got a note, the topic
        // got another status (the entry itself is untouched).
        let patch = |status| TaskPatch {
            status: Some(status),
            ..Default::default()
        };
        f.vault
            .update_task(&f.open, patch(TaskStatus::Blocked))
            .unwrap();
        f.vault
            .update_task(
                &new_id,
                TaskPatch {
                    notes: Some("ręczna notatka".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        f.vault
            .update_topic(
                "beeking",
                "platnosci",
                montybrain_core::TopicPatch {
                    status: Some(TopicStatus::Blocked),
                    ..Default::default()
                },
            )
            .unwrap();
        let r = undo(&f.vault, "beeking", &record(written), Some(&mut p));
        let st: Vec<_> = r.iter().map(|x| (x.index, x.status)).collect();
        assert_eq!(
            st,
            [
                (0, UndoStatus::Changed),
                (1, UndoStatus::Partial),
                (2, UndoStatus::Changed)
            ],
            "{r:#?}"
        );
        assert!(r[1].message.contains("status"), "{}", r[1].message);
        let snap = f.vault.load().unwrap();
        assert_eq!(snap.task(&f.open).unwrap().status, TaskStatus::Blocked);
        assert!(snap.task(&new_id).is_some());
        assert!(
            snap.journal_of("beeking", "platnosci").is_empty(),
            "the untouched entry went"
        );
        assert_eq!(
            snap.topic("beeking", "platnosci").unwrap().status,
            TopicStatus::Blocked
        );
        // Nothing fully undone: those changes stay applied.
        assert_eq!(p.applied(), 3);
    }

    fn topic_proposal(f: &Fixture) -> StoredProposal {
        let json = r#"{"changes":[{"kind":"new_topic","title":"Integracja BLIK","status":"in_progress","summary":"Gałąź w toku (wniosek).","text":"feature/blik: 3 commity od 12 lis.","tasks":[{"title":"Testy sandbox","status":"todo"},{"title":"Webhook","status":"done"}],"reason":"Aktywna gałąź."}]}"#;
        let answer = format!("Raport.\n```{}\n{json}\n```", proposal::FENCE_INFO);
        let p = proposal::parse(&answer, &f.vault.load().unwrap(), "beeking");
        assert_eq!(p.changes.len(), 1, "{p:?}");
        StoredProposal::from_proposal(
            p,
            "r2",
            ProviderId::Codex,
            TriggerKind::Manual,
            at("2026-11-14T14:00"),
        )
    }

    #[test]
    fn a_new_topic_is_written_as_one_file_and_undone_while_untouched() {
        let f = fixture();
        let mut p = topic_proposal(&f);
        let (r, written) = apply(&f.vault, "beeking", &mut p, &[0], at("2026-11-14T14:05"));
        assert_eq!(r[0].status, ApplyStatus::Applied, "{r:?}");
        let snap = f.vault.load().unwrap();
        let t = snap.topic("beeking", "integracja-blik").unwrap();
        assert_eq!(
            (t.status, t.summary.as_str()),
            (TopicStatus::InProgress, "Gałąź w toku (wniosek).")
        );
        let tasks: Vec<_> = snap
            .tasks
            .iter()
            .filter(|t| t.location.topic() == Some("integracja-blik"))
            .map(|t| (t.title.as_str(), t.status))
            .collect();
        assert_eq!(
            tasks,
            [
                ("Testy sandbox", TaskStatus::Todo),
                ("Webhook", TaskStatus::Done)
            ]
        );
        let j = snap.journal_of("beeking", "integracja-blik");
        assert_eq!((j.len(), j[0].author.as_str()), (1, "codex"));

        let r = undo(&f.vault, "beeking", &record_of(written), Some(&mut p));
        assert_eq!(r[0].status, UndoStatus::Undone, "{r:?}");
        assert!(
            f.vault
                .load()
                .unwrap()
                .topic("beeking", "integracja-blik")
                .is_none()
        );
        assert_eq!(p.pending(), 1, "can be decided again");

        // Applied again, then touched by hand: the undo leaves it alone.
        let (_, written) = apply(&f.vault, "beeking", &mut p, &[0], at("2026-11-14T14:10"));
        let snap = f.vault.load().unwrap();
        let id = snap
            .tasks
            .iter()
            .find(|t| t.title == "Testy sandbox")
            .and_then(|t| t.id.clone())
            .unwrap();
        f.vault
            .update_task(
                &id,
                TaskPatch {
                    status: Some(TaskStatus::Doing),
                    ..Default::default()
                },
            )
            .unwrap();
        let r = undo(&f.vault, "beeking", &record_of(written), Some(&mut p));
        assert_eq!(r[0].status, UndoStatus::Changed, "{r:?}");
        assert!(r[0].message.contains("zmieniło się w międzyczasie"));
        assert!(
            f.vault
                .load()
                .unwrap()
                .topic("beeking", "integracja-blik")
                .is_some()
        );
    }

    #[test]
    fn a_new_topic_is_skipped_when_such_a_topic_appeared() {
        let f = fixture();
        let mut p = topic_proposal(&f);
        f.vault
            .create_topic(
                "beeking",
                NewTopic {
                    title: "integracja blik".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let (r, written) = apply(&f.vault, "beeking", &mut p, &[0], at("2026-11-14T14:05"));
        assert_eq!(r[0].status, ApplyStatus::Stale, "{r:?}");
        assert!(written.is_empty());
        assert_eq!(
            f.vault.load().unwrap().topics_of("beeking").count(),
            2,
            "nothing extra written"
        );
    }

    #[test]
    fn a_corrected_change_is_validated_shown_applied_and_undone_as_corrected() {
        use crate::runs::proposal::ChangeEdit;
        use crate::updates::store::edit_change;
        let f = fixture();
        let mut p = topic_proposal(&f);
        let snap = f.vault.load().unwrap();
        // A rejected correction leaves the proposal as it was.
        let before = p.clone();
        let tasks = |keep: Vec<bool>| -> Vec<proposal::TopicTaskEdit> {
            keep.into_iter()
                .zip([("Testy sandbox", "todo"), ("Webhook", "done")])
                .map(|(keep, (title, status))| proposal::TopicTaskEdit {
                    keep,
                    title: title.into(),
                    status: status.into(),
                })
                .collect()
        };
        let bad = |title: &str, status: &str, keep: Vec<bool>| ChangeEdit::NewTopic {
            title: title.into(),
            status: status.into(),
            summary: "Opis.".into(),
            text: "Wpis.".into(),
            tasks: tasks(keep),
        };
        let with_tasks = |t: Vec<(&str, &str)>| ChangeEdit::NewTopic {
            title: "Integracja BLIK".into(),
            status: "planned".into(),
            summary: "Opis.".into(),
            text: "Wpis.".into(),
            tasks: t
                .into_iter()
                .map(|(title, status)| proposal::TopicTaskEdit {
                    keep: true,
                    title: title.into(),
                    status: status.into(),
                })
                .collect(),
        };
        for (edit, want) in [
            (
                bad("Płatności", "in_progress", vec![true, true]),
                "Taki temat już istnieje",
            ),
            (
                bad("Integracja BLIK", "gotowe", vec![true, true]),
                "nieznany status",
            ),
            (
                bad("Integracja BLIK", "planned", vec![true]),
                "nie zgadza się",
            ),
            (bad(&"x".repeat(400), "planned", vec![true, true]), ""),
            (
                with_tasks(vec![("Testy", "blocked"), ("Webhook", "done")]),
                "todo, doing albo done",
            ),
            (
                with_tasks(vec![("Testy", "todo"), ("testy", "done")]),
                "Powtórzone zadanie",
            ),
            (with_tasks(vec![("  ", "todo"), ("Webhook", "done")]), ""),
            (ChangeEdit::NewTask { title: "x".into() }, "nie pasuje"),
        ] {
            let e =
                edit_change(&mut p, 0, &edit, &snap, "beeking").expect_err(&format!("{edit:?}"));
            assert!(e.contains(want), "{e}");
            assert_eq!(p, before);
        }
        // A valid one: retitled, another status and summary, one task left out.
        let good = ChangeEdit::NewTopic {
            title: "Płatności BLIK".into(),
            status: "planned".into(),
            summary: "Poprawiony opis.".into(),
            text: "Poprawiony wpis.".into(),
            tasks: vec![
                proposal::TopicTaskEdit {
                    keep: true,
                    title: "Testy na sandboksie".into(),
                    status: "doing".into(),
                },
                proposal::TopicTaskEdit {
                    keep: false,
                    title: "Webhook".into(),
                    status: "done".into(),
                },
            ],
        };
        edit_change(&mut p, 0, &good, &snap, "beeking").unwrap();
        assert!(p.changes[0].edited);
        let proposal::Change::NewTopic {
            slug,
            status,
            tasks,
            reason,
            ..
        } = &p.changes[0].change
        else {
            panic!("kind kept")
        };
        assert_eq!(
            (slug.as_str(), *status, tasks.len(), reason.as_str()),
            ("platnosci-blik", TopicStatus::Planned, 1, "Aktywna gałąź.")
        );
        assert_eq!(
            (tasks[0].title.as_str(), tasks[0].status),
            ("Testy na sandboksie", TaskStatus::Doing)
        );
        // It survives a save and load.
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<StoredProposal>(&json).unwrap(), p);

        let (r, written) = apply(&f.vault, "beeking", &mut p, &[0], at("2026-11-14T14:05"));
        assert_eq!(r[0].status, ApplyStatus::Applied, "{r:?}");
        let snap = f.vault.load().unwrap();
        let t = snap.topic("beeking", "platnosci-blik").unwrap();
        assert_eq!(
            (t.status, t.summary.as_str()),
            (TopicStatus::Planned, "Poprawiony opis.")
        );
        assert_eq!(
            snap.journal_of("beeking", "platnosci-blik")[0].text,
            "Poprawiony wpis."
        );
        assert_eq!(
            snap.tasks
                .iter()
                .filter(|t| t.location.topic() == Some("platnosci-blik"))
                .count(),
            1
        );
        // Decided: no more corrections.
        assert!(
            edit_change(&mut p, 0, &good, &snap, "beeking")
                .unwrap_err()
                .contains("rozstrzygnięta")
        );
        let r = undo(&f.vault, "beeking", &record_of(written), Some(&mut p));
        assert_eq!(r[0].status, UndoStatus::Undone, "{r:?}");
        assert!(
            f.vault
                .load()
                .unwrap()
                .topic("beeking", "platnosci-blik")
                .is_none()
        );
        assert!(p.changes[0].edited, "the corrected version stays");
    }

    #[test]
    fn corrections_of_the_other_kinds() {
        use crate::runs::proposal::ChangeEdit;
        use crate::updates::store::edit_change;
        let f = fixture();
        let mut p = proposal_for(&f, ProviderId::Claude);
        let snap = f.vault.load().unwrap();
        // task_status: another status; the same as now is refused.
        edit_change(
            &mut p,
            0,
            &ChangeEdit::TaskStatus {
                status: "blocked".into(),
            },
            &snap,
            "beeking",
        )
        .unwrap();
        assert!(matches!(
            p.changes[0].change,
            proposal::Change::TaskStatus {
                to: TaskStatus::Blocked,
                ..
            }
        ));
        assert!(
            edit_change(
                &mut p,
                0,
                &ChangeEdit::TaskStatus {
                    status: "doing".into()
                },
                &snap,
                "beeking"
            )
            .unwrap_err()
            .contains("ma już ten status")
        );
        // topic_log: keep the entry, no status change, a summary equal to the current one = no summary change.
        let log = ChangeEdit::TopicLog {
            status: None,
            summary: "Stan.".into(),
            text: "Zwroty gotowe, bez zmiany statusu.".into(),
        };
        edit_change(&mut p, 1, &log, &snap, "beeking").unwrap();
        assert!(
            matches!(&p.changes[1].change, proposal::Change::TopicLog { status: None, summary: None, text, .. } if text.contains("bez zmiany"))
        );
        // new_task: an empty title is refused, a duplicate of an existing task too.
        assert!(
            edit_change(
                &mut p,
                2,
                &ChangeEdit::NewTask { title: "  ".into() },
                &snap,
                "beeking"
            )
            .is_err()
        );
        assert!(
            edit_change(
                &mut p,
                2,
                &ChangeEdit::NewTask {
                    title: "webhook zwrotów".into()
                },
                &snap,
                "beeking"
            )
            .unwrap_err()
            .contains("już istnieje")
        );
        // The target changed since the proposal: no corrections.
        f.vault
            .update_task(
                &f.open,
                TaskPatch {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .unwrap();
        let now = f.vault.load().unwrap();
        let e = edit_change(
            &mut p,
            0,
            &ChangeEdit::TaskStatus {
                status: "todo".into(),
            },
            &now,
            "beeking",
        )
        .unwrap_err();
        assert!(e.contains("zmienione w międzyczasie"), "{e}");
    }

    #[test]
    fn apply_then_undo_leaves_the_files_byte_identical() {
        let f = fixture();
        let mut p = proposal_for(&f, ProviderId::Codex);
        // A later clock: every write bumps `zaktualizowano` to another minute.
        let later = f.vault.clone().with_clock(|| at("2031-01-02T03:04"));
        let topic = f.vault.root().join("beeking/tematy/platnosci.md");
        let before = std::fs::read_to_string(&topic).unwrap();
        let (r, written) = apply(
            &later,
            "beeking",
            &mut p,
            &[0, 1, 2],
            at("2031-01-02T03:04"),
        );
        assert!(r.iter().all(|i| i.status == ApplyStatus::Applied), "{r:?}");
        let after = std::fs::read_to_string(&topic).unwrap();
        assert!(
            after.contains("zaktualizowano: 2031-01-02T03:04"),
            "{after}"
        );
        let rec = UndoRecord {
            token: "t".into(),
            run_id: "r1".into(),
            at: at("2031-01-02T03:05"),
            items: written,
        };
        let r = undo(&later, "beeking", &rec, Some(&mut p));
        assert!(r.iter().all(|i| i.status == UndoStatus::Undone), "{r:?}");
        assert_eq!(std::fs::read_to_string(&topic).unwrap(), before);
        // The stamps are kept with the undo record (updates.json).
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"stamp\""), "{json}");
        assert_eq!(serde_json::from_str::<UndoRecord>(&json).unwrap(), rec);
    }

    #[test]
    fn the_date_is_not_put_back_over_a_later_edit() {
        let f = fixture();
        let mut p = proposal_for(&f, ProviderId::Codex);
        let later = f.vault.clone().with_clock(|| at("2031-01-02T03:04"));
        let (_, written) = apply(&later, "beeking", &mut p, &[1], at("2031-01-02T03:04"));
        // Someone edits the topic afterwards (another minute).
        f.vault
            .clone()
            .with_clock(|| at("2031-01-02T04:00"))
            .update_topic(
                "beeking",
                "platnosci",
                montybrain_core::TopicPatch {
                    title: Some("Płatności i zwroty".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let rec = UndoRecord {
            token: "t".into(),
            run_id: "r1".into(),
            at: at("2031-01-02T03:05"),
            items: written,
        };
        let undo_clock = f.vault.clone().with_clock(|| at("2031-01-02T05:00"));
        undo(&undo_clock, "beeking", &rec, Some(&mut p));
        let text =
            std::fs::read_to_string(f.vault.root().join("beeking/tematy/platnosci.md")).unwrap();
        // Bumped like any write, not put back to the pre-apply value.
        assert!(text.contains("zaktualizowano: 2031-01-02T05:00"), "{text}");
        assert!(text.contains("Płatności i zwroty"));
    }

    #[test]
    fn branches_become_links_on_apply_and_go_with_the_undo() {
        let f = fixture();
        f.vault
            .update_project(
                "beeking",
                montybrain_core::ProjectPatch {
                    repos: Some(vec![montybrain_core::Repo {
                        name: "app".into(),
                        path: "/r/app".into(),
                    }]),
                    ..Default::default()
                },
            )
            .unwrap();
        let json = r#"{"changes":[{"kind":"new_topic","title":"Kasa offline","status":"in_progress","summary":"S.","text":"T.","reason":"r","branches":[{"repo":"app","branch":"feature/pos-offline"}]},{"kind":"topic_log","topic":"platnosci","text":"Wpis.","branches":[{"repo":"app","branch":"blik-e2e"}]}]}"#;
        let answer = format!("R.\n```{}\n{json}\n```", proposal::FENCE_INFO);
        let p = proposal::parse(&answer, &f.vault.load().unwrap(), "beeking");
        assert_eq!(p.changes.len(), 2, "{:?}", p.dropped);
        let mut p = StoredProposal::from_proposal(
            p,
            "r2",
            ProviderId::Codex,
            TriggerKind::Manual,
            at("2026-11-14T14:00"),
        );
        let (r, written) = apply(&f.vault, "beeking", &mut p, &[0, 1], at("2026-11-14T14:05"));
        assert!(r.iter().all(|i| i.status == ApplyStatus::Applied), "{r:?}");
        let links = crate::drift::links::load(f.vault.root());
        assert_eq!(
            links.topic_of(&BranchKey::new("beeking", "app", "feature/pos-offline")),
            Some("kasa-offline")
        );
        assert_eq!(
            links.topic_of(&BranchKey::new("beeking", "app", "blik-e2e")),
            Some("platnosci")
        );
        let r = undo(&f.vault, "beeking", &record_of(written), Some(&mut p));
        assert!(r.iter().all(|i| i.status == UndoStatus::Undone), "{r:?}");
        assert!(crate::drift::links::load(f.vault.root()).links.is_empty());
    }

    #[test]
    fn a_joined_new_topic_is_applied_and_undone_as_an_entry_and_tasks() {
        use crate::updates::store::join_change;
        let f = fixture();
        let mut p = topic_proposal(&f);
        let snap = f.vault.load().unwrap();
        let skipped = join_change(&mut p, 0, "platnosci", &snap, "beeking").unwrap();
        assert!(skipped.is_empty());
        assert_eq!(p.changes.len(), 3);
        assert!(p.changes.iter().all(|c| c.edited));
        assert!(
            matches!(&p.changes[0].change, proposal::Change::TopicLog { topic, .. } if topic == "platnosci")
        );
        // Decided rows cannot be joined; a topic_log cannot be joined either.
        assert!(join_change(&mut p, 0, "platnosci", &snap, "beeking").is_err());
        let (r, written) = apply(
            &f.vault,
            "beeking",
            &mut p,
            &[0, 1, 2],
            at("2026-11-14T14:05"),
        );
        assert!(r.iter().all(|i| i.status == ApplyStatus::Applied), "{r:?}");
        let snap = f.vault.load().unwrap();
        assert!(
            snap.topic("beeking", "integracja-blik").is_none(),
            "no new topic"
        );
        assert_eq!(
            snap.journal_of("beeking", "platnosci")[0].text,
            "feature/blik: 3 commity od 12 lis."
        );
        let titles: Vec<&str> = snap
            .tasks
            .iter()
            .filter(|t| t.location.topic() == Some("platnosci"))
            .map(|t| t.title.as_str())
            .collect();
        assert!(
            titles.contains(&"Testy sandbox") && titles.contains(&"Webhook"),
            "{titles:?}"
        );
        let r = undo(&f.vault, "beeking", &record_of(written), Some(&mut p));
        assert!(r.iter().all(|i| i.status == UndoStatus::Undone), "{r:?}");
        assert_eq!(p.pending(), 3);
    }

    fn record_of(items: Vec<UndoItem>) -> UndoRecord {
        UndoRecord {
            token: "t".into(),
            run_id: "r2".into(),
            at: at("2026-11-14T14:06"),
            items,
        }
    }
}
