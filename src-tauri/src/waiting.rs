//! Waiting items: topics with status Czeka or Zablokowany and blocked tasks.
//! Each gets a check schedule (app state, `waiting.json` next to
//! `reminders.json`): when it is due, the item rises to "Wymaga uwagi" in Dziś
//! and one notification at the digest hour sums up everything that is due
//! ([`note`], through the reminders scheduler and its clock).
//!
//! The schedule follows the vault ([`reconcile`]): an item that becomes
//! waiting (pill, editor, an agent, a hand edit) gets the default interval;
//! one that leaves the waiting status loses its schedule. "Na co czeka?" is
//! not stored here: it is a journal entry ("Czeka: …") of the topic, or a line
//! in the task's notes, so it lives in Markdown where agents see it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use montybrain_core::chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};
use montybrain_core::{ProjectState, Snapshot, TaskStatus, TopicStatus};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::reminders::{Config, Fired, Note, NoteKind};

pub const FILE: &str = "waiting.json";
pub const WAITING_CHANGED: &str = "waiting-changed";
pub const DEFAULT_DAYS: u8 = 2;
/// Intervals offered ("Przypomnij za").
pub const CHOICES: &[u8] = &[1, 2, 3, 7];

/// Prefixes of the reason entry / note line.
pub const TOPIC_WAITING_PREFIX: &str = "Czeka: ";
pub const TOPIC_BLOCKED_PREFIX: &str = "Zablokowany: ";
pub const TASK_BLOCKED_PREFIX: &str = "Zablokowane: ";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitCheck {
    /// The day the item is due for a check.
    pub next: NaiveDate,
    pub interval_days: u8,
    /// When the user last said "Nadal czeka" (minute).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed: Option<String>,
    /// When it started waiting (as far as the app knows; minute).
    pub since: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WaitStore {
    /// `topic:<project>/<slug>` or `task:<id>` → schedule.
    pub items: BTreeMap<String, WaitCheck>,
}

impl WaitStore {
    pub fn load(path: &Path) -> WaitStore {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        json.push('\n');
        crate::settings::write_atomic(path, json.as_bytes())
    }
}

pub fn topic_key(project: &str, slug: &str) -> String {
    format!("topic:{project}/{slug}")
}

pub fn task_key(id: &str) -> String {
    format!("task:{id}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitKind {
    Topic,
    Task,
}

/// A waiting item of the vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WaitTarget {
    pub key: String,
    pub kind: WaitKind,
    pub project: Option<String>,
    pub topic: Option<String>,
    pub task: Option<String>,
    pub title: String,
    /// `waiting` / `blocked`.
    pub status: String,
    /// What it waits for, from the newest "Czeka: …" / "Zablokowany: …"
    /// journal entry (topic) or notes line (task).
    pub reason: Option<String>,
}

fn strip_reason(text: &str) -> Option<String> {
    for p in [
        TOPIC_WAITING_PREFIX,
        TOPIC_BLOCKED_PREFIX,
        TASK_BLOCKED_PREFIX,
    ] {
        if let Some(rest) = text.trim().strip_prefix(p) {
            let r = rest.lines().next().unwrap_or("").trim();
            if !r.is_empty() {
                return Some(r.to_string());
            }
        }
    }
    None
}

/// The waiting items of active projects (and blocked inbox tasks).
pub fn targets(snap: &Snapshot) -> Vec<WaitTarget> {
    let active = |p: &str| {
        snap.project(p)
            .is_some_and(|x| x.state == ProjectState::Active)
    };
    let mut out = Vec::new();
    for t in &snap.topics {
        let status = match t.status {
            TopicStatus::Waiting => "waiting",
            TopicStatus::Blocked => "blocked",
            _ => continue,
        };
        if !active(&t.project) {
            continue;
        }
        let reason = snap
            .journal_of(&t.project, &t.slug)
            .iter()
            .find_map(|j| strip_reason(&j.text));
        out.push(WaitTarget {
            key: topic_key(&t.project, &t.slug),
            kind: WaitKind::Topic,
            project: Some(t.project.clone()),
            topic: Some(t.slug.clone()),
            task: None,
            title: t.title.clone(),
            status: status.into(),
            reason,
        });
    }
    for t in &snap.tasks {
        let Some(id) = t.id.as_deref() else { continue };
        if t.status != TaskStatus::Blocked || t.location.project().is_some_and(|p| !active(p)) {
            continue;
        }
        out.push(WaitTarget {
            key: task_key(id),
            kind: WaitKind::Task,
            project: t.location.project().map(str::to_string),
            topic: t.location.topic().map(str::to_string),
            task: Some(id.to_string()),
            title: t.title.clone(),
            status: "blocked".into(),
            reason: t.notes.lines().rev().find_map(strip_reason),
        });
    }
    out
}

/// Follows the vault: new waiting items get `default_days`, items that left
/// the waiting status lose their schedule. Returns whether anything changed.
pub fn reconcile(
    store: &mut WaitStore,
    snap: &Snapshot,
    now: NaiveDateTime,
    default_days: u8,
) -> bool {
    let live = targets(snap);
    let before = store.items.len();
    let mut changed = false;
    store.items.retain(|k, _| live.iter().any(|t| &t.key == k));
    changed |= store.items.len() != before;
    for t in &live {
        if !store.items.contains_key(&t.key) {
            store.items.insert(t.key.clone(), fresh(now, default_days));
            changed = true;
        }
    }
    changed
}

fn fresh(now: NaiveDateTime, days: u8) -> WaitCheck {
    WaitCheck {
        next: now.date() + TimeDelta::days(i64::from(days)),
        interval_days: days,
        confirmed: None,
        since: montybrain_core::minute::format(&now),
    }
}

pub fn valid_days(days: u8) -> Result<u8, String> {
    if CHOICES.contains(&days) {
        Ok(days)
    } else {
        Err(format!(
            "Nieprawidłowy odstęp przypomnień: {days} dni (dozwolone 1, 2, 3, 7)."
        ))
    }
}

/// "Przypomnij za" chosen in the waiting step: next check `days` from now.
pub fn schedule(
    store: &mut WaitStore,
    key: &str,
    days: u8,
    now: NaiveDateTime,
) -> Result<(), String> {
    let days = valid_days(days)?;
    let since = store
        .items
        .get(key)
        .map(|c| c.since.clone())
        .unwrap_or_else(|| montybrain_core::minute::format(&now));
    store.items.insert(
        key.to_string(),
        WaitCheck {
            since,
            ..fresh(now, days)
        },
    );
    Ok(())
}

/// "Nadal czeka": the next check one interval from today; the confirmation is recorded.
pub fn confirm(store: &mut WaitStore, key: &str, now: NaiveDateTime) -> Result<(), String> {
    let c = store
        .items
        .get_mut(key)
        .ok_or("Ta pozycja już nie czeka.")?;
    c.next = now.date() + TimeDelta::days(i64::from(c.interval_days));
    c.confirmed = Some(montybrain_core::minute::format(&now));
    Ok(())
}

/// What the window gets per waiting item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WaitView {
    #[serde(flatten)]
    pub target: WaitTarget,
    pub next: NaiveDate,
    pub interval_days: u8,
    pub confirmed: Option<String>,
    pub since: String,
    /// The check is due (today or earlier).
    pub due: bool,
}

pub fn views(store: &WaitStore, snap: &Snapshot, now: NaiveDateTime) -> Vec<WaitView> {
    targets(snap)
        .into_iter()
        .filter_map(|t| {
            let c = store.items.get(&t.key)?;
            Some(WaitView {
                due: c.next <= now.date(),
                next: c.next,
                interval_days: c.interval_days,
                confirmed: c.confirmed.clone(),
                since: c.since.clone(),
                target: t,
            })
        })
        .collect()
}

pub fn note_key(day: NaiveDate) -> String {
    format!("waiting:{day}")
}

/// "1 rzecz czeka", "3 rzeczy czekają", "5 rzeczy czeka".
pub fn count_things(n: usize) -> String {
    let few = (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100));
    match (n, few) {
        (1, _) => "1 rzecz czeka".into(),
        (_, true) => format!("{n} rzeczy czekają"),
        _ => format!("{n} rzeczy czeka"),
    }
}

/// The one notification for every due check, at the digest hour, once a day.
pub fn note(due_titles: &[String], now: NaiveDateTime, fired: &Fired, cfg: Config) -> Option<Note> {
    if !cfg.enabled || due_titles.is_empty() {
        return None;
    }
    let at =
        NaiveTime::from_hms_opt(u32::from(cfg.digest_hour.min(23)), 0, 0).unwrap_or(NaiveTime::MIN);
    let key = note_key(now.date());
    if now.time() < at || fired.contains(&key) {
        return None;
    }
    let mut body = due_titles
        .iter()
        .take(3)
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ");
    if due_titles.len() > 3 {
        body.push_str(&format!(" · i {} więcej", due_titles.len() - 3));
    }
    Some(Note {
        kind: NoteKind::Waiting,
        title: format!(
            "{}. Sprawdź, czy coś się ruszyło.",
            count_things(due_titles.len())
        ),
        body,
        keys: vec![key],
    })
}

// --- Tauri glue --------------------------------------------------------------------------

pub struct Waiting {
    pub store: Mutex<WaitStore>,
    path: PathBuf,
}

impl Waiting {
    pub fn save(&self, st: &WaitStore) {
        if let Err(e) = st.save(&self.path) {
            crate::log(&format!(
                "waiting: cannot save {}: {e}",
                self.path.display()
            ));
        }
    }
}

pub fn start<R: Runtime>(app: &AppHandle<R>, config_dir: &Path) {
    let path = config_dir.join(FILE);
    app.manage(Waiting {
        store: Mutex::new(WaitStore::load(&path)),
        path,
    });
}

fn default_days<R: Runtime>(app: &AppHandle<R>) -> u8 {
    app.try_state::<crate::native::Native>()
        .map_or(DEFAULT_DAYS, |n| n.settings().waiting_days)
}

/// Called by the reminders tick: follows the vault and returns the titles of
/// the items due for a check.
pub fn tick<R: Runtime>(app: &AppHandle<R>, snap: &Snapshot, now: NaiveDateTime) -> Vec<String> {
    let Some(w) = app.try_state::<Waiting>() else {
        return Vec::new();
    };
    let mut st = crate::lock(&w.store);
    if reconcile(&mut st, snap, now, default_days(app)) {
        w.save(&st);
        let _ = app.emit(WAITING_CHANGED, ());
    }
    views(&st, snap, now)
        .into_iter()
        .filter(|v| v.due)
        .map(|v| v.target.title)
        .collect()
}

pub fn list<R: Runtime>(app: &AppHandle<R>) -> Vec<WaitView> {
    let Some(w) = app.try_state::<Waiting>() else {
        return Vec::new();
    };
    let snap = app.state::<crate::state::VaultState>().snapshot();
    let now = crate::reminders::local_now();
    let mut st = crate::lock(&w.store);
    if reconcile(&mut st, &snap, now, default_days(app)) {
        w.save(&st);
    }
    views(&st, &snap, now)
}

/// Runs `f` on the store (after following the vault), saves, and returns the views.
pub fn change<R: Runtime>(
    app: &AppHandle<R>,
    f: impl FnOnce(&mut WaitStore, NaiveDateTime) -> Result<(), String>,
) -> Result<Vec<WaitView>, String> {
    let w = app.state::<Waiting>();
    let snap = app.state::<crate::state::VaultState>().snapshot();
    let now = crate::reminders::local_now();
    {
        let mut st = crate::lock(&w.store);
        reconcile(&mut st, &snap, now, default_days(app));
        f(&mut st, now)?;
        w.save(&st);
    }
    let _ = app.emit(WAITING_CHANGED, ());
    Ok(list(app))
}

#[cfg(test)]
mod tests {
    use super::*;
    use montybrain_core::{JournalEntry, Project, Task, TaskLocation, Topic};

    fn at(s: &str) -> NaiveDateTime {
        montybrain_core::minute::parse(s).unwrap()
    }

    fn snap(topic_status: TopicStatus, task_status: TaskStatus) -> Snapshot {
        Snapshot {
            projects: vec![Project {
                slug: "dom".into(),
                name: "Dom".into(),
                color: None,
                state: ProjectState::Active,
                repos: vec![],
                description: String::new(),
                path: "dom/projekt.md".into(),
            }],
            topics: vec![Topic {
                project: "dom".into(),
                slug: "kuchnia".into(),
                title: "Kuchnia".into(),
                status: topic_status,
                repo: None,
                summary: String::new(),
                created: None,
                updated: None,
                path: "dom/tematy/kuchnia.md".into(),
            }],
            tasks: vec![Task {
                id: Some("aaaaa1".into()),
                title: "Blat".into(),
                status: task_status,
                priority: Default::default(),
                due: None,
                done: None,
                links: vec![],
                notes: "Zablokowane: czeka na wymiar".into(),
                location: TaskLocation::Project {
                    project: "dom".into(),
                },
                line: 1,
            }],
            journal: vec![JournalEntry {
                project: "dom".into(),
                topic: "kuchnia".into(),
                at: at("2026-11-10T10:00"),
                author: "ja".into(),
                text: "Czeka: wycena od stolarza".into(),
                line: 1,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn schedule_follows_the_vault() {
        let mut st = WaitStore::default();
        let s = snap(TopicStatus::Waiting, TaskStatus::Blocked);
        let now = at("2026-11-14T10:00");
        assert!(reconcile(&mut st, &s, now, 2));
        assert_eq!(st.items.len(), 2);
        let c = &st.items[&topic_key("dom", "kuchnia")];
        assert_eq!(
            (c.next.to_string(), c.interval_days),
            ("2026-11-16".into(), 2)
        );
        assert!(!reconcile(&mut st, &s, now, 2), "stable");
        // Reasons come from the journal / the notes.
        let v = views(&st, &s, now);
        assert_eq!(v[0].target.reason.as_deref(), Some("wycena od stolarza"));
        assert_eq!(v[1].target.reason.as_deref(), Some("czeka na wymiar"));
        assert!(!v[0].due);
        // Leaving the waiting status clears the schedule.
        let moved = snap(TopicStatus::InProgress, TaskStatus::Doing);
        assert!(reconcile(&mut st, &moved, now, 2));
        assert!(st.items.is_empty());
    }

    #[test]
    fn set_confirm_and_due() {
        let mut st = WaitStore::default();
        let s = snap(TopicStatus::Blocked, TaskStatus::Todo);
        let k = topic_key("dom", "kuchnia");
        reconcile(&mut st, &s, at("2026-11-14T10:00"), 2);
        schedule(&mut st, &k, 7, at("2026-11-14T10:00")).unwrap();
        assert_eq!(st.items[&k].next.to_string(), "2026-11-21");
        assert!(schedule(&mut st, &k, 5, at("2026-11-14T10:00")).is_err());
        schedule(&mut st, &k, 1, at("2026-11-14T10:00")).unwrap();
        assert!(views(&st, &s, at("2026-11-15T08:00"))[0].due);
        confirm(&mut st, &k, at("2026-11-15T08:00")).unwrap();
        let c = &st.items[&k];
        assert_eq!(
            (c.next.to_string(), c.confirmed.as_deref()),
            ("2026-11-16".into(), Some("2026-11-15T08:00"))
        );
        assert!(!views(&st, &s, at("2026-11-15T08:00"))[0].due);
        assert!(confirm(&mut st, "topic:dom/nie-ma", at("2026-11-15T08:00")).is_err());
        // Archived projects do not wait.
        let mut archived = s.clone();
        archived.projects[0].state = ProjectState::Archived;
        assert!(targets(&archived).is_empty());
    }

    #[test]
    fn one_notification_for_all_due_checks_at_the_digest_hour() {
        let cfg = Config {
            enabled: true,
            digest_hour: 9,
        };
        let mut fired = Fired::default();
        let titles = vec![
            "Kuchnia".to_string(),
            "Blat".to_string(),
            "Okna".to_string(),
            "Drzwi".to_string(),
        ];
        assert_eq!(
            note(&titles, at("2026-11-14T08:59"), &fired, cfg),
            None,
            "before the hour"
        );
        let n = note(&titles, at("2026-11-14T09:00"), &fired, cfg).unwrap();
        assert_eq!(n.title, "4 rzeczy czekają. Sprawdź, czy coś się ruszyło.");
        assert_eq!(n.body, "Kuchnia · Blat · Okna · i 1 więcej");
        assert_eq!(n.keys, vec!["waiting:2026-11-14".to_string()]);
        fired.insert(n.keys[0].clone(), at("2026-11-14T09:00"));
        assert_eq!(
            note(&titles, at("2026-11-14T15:00"), &fired, cfg),
            None,
            "once a day"
        );
        assert!(note(&titles, at("2026-11-15T09:30"), &fired, cfg).is_some());
        assert_eq!(note(&[], at("2026-11-15T09:30"), &fired, cfg), None);
        assert_eq!(
            note(
                &titles,
                at("2026-11-15T09:30"),
                &fired,
                Config {
                    enabled: false,
                    digest_hour: 9
                }
            ),
            None
        );
        assert_eq!(count_things(1), "1 rzecz czeka");
        assert_eq!(count_things(5), "5 rzeczy czeka");
        assert_eq!(count_things(22), "22 rzeczy czekają");
    }
}
