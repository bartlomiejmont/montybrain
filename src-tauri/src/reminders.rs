//! Due reminders.
//!
//! * A task with a due **time** gets one notification at that time (title = task
//!   title, body = project / topic).
//! * Tasks due today without a time plus overdue tasks are summed up in one
//!   daily digest at `digest_hour` (or soon after start, when the app starts
//!   later that day and today's digest has not been shown yet).
//!
//! The decision is the pure function [`plan`]: snapshot + now + what was already
//! notified → what to show. The [`Fired`] set is persisted in the config dir
//! (`reminders.json`), keyed so that rescheduling a task re-arms it
//! (`due:<id>@<due>`), and pruned after [`KEEP_DAYS`].
//!
//! The scheduler thread ([`start`]) runs [`plan`] at every full minute and
//! whenever it is poked (vault or settings changed).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use montybrain_core::chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Timelike};
use montybrain_core::{ProjectState, Snapshot, Task, TaskLocation, TaskStatus};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

pub const STATE_FILE: &str = "reminders.json";
/// A timed reminder still fires when the app notices it this late (app was
/// not running, Mac was asleep). Older ones are left to the digest.
pub const LATE_WINDOW: TimeDelta = TimeDelta::hours(12);
/// Fired entries older than this are dropped from `reminders.json`.
pub const KEEP_DAYS: i64 = 7;
/// More timed reminders than this at once are merged into one notification.
pub const MAX_SEPARATE: usize = 3;
/// How many titles a summary body lists.
const BODY_TITLES: usize = 3;
/// Delay before the first check after start ("shortly after start").
const FIRST_CHECK: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub enabled: bool,
    pub digest_hour: u8,
}

/// What has already been notified: key → when.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fired {
    #[serde(default)]
    pub fired: BTreeMap<String, String>,
}

impl Fired {
    pub fn contains(&self, key: &str) -> bool {
        self.fired.contains_key(key)
    }

    pub fn insert(&mut self, key: String, at: NaiveDateTime) {
        self.fired.insert(key, montybrain_core::minute::format(&at));
    }

    /// Drops entries fired more than [`KEEP_DAYS`] ago (or unreadable ones).
    /// Safe: a timed key can only fire within [`LATE_WINDOW`] of its due time,
    /// and a digest key only on its own day. Returns whether anything changed.
    pub fn prune(&mut self, now: NaiveDateTime) -> bool {
        let cutoff = now - TimeDelta::days(KEEP_DAYS);
        let before = self.fired.len();
        self.fired
            .retain(|_, at| montybrain_core::minute::parse(at).is_some_and(|t| t >= cutoff));
        self.fired.len() != before
    }

    pub fn load(path: &Path) -> Fired {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                crate::log(&format!("reminders: ignoring {}: {e}", path.display()));
                Fired::default()
            }),
            Err(_) => Fired::default(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        json.push('\n');
        crate::settings::write_atomic(path, json.as_bytes())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    Due,
    DueBatch,
    Digest,
    /// Waiting items due for a check ([`crate::waiting::note`]).
    Waiting,
}

/// One system notification to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Note {
    pub kind: NoteKind,
    pub title: String,
    pub body: String,
    /// Keys to record as fired once shown.
    pub keys: Vec<String>,
}

/// Result of [`plan`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub notes: Vec<Note>,
    /// Keys to record without showing anything (an empty digest).
    pub silent: Vec<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty() && self.silent.is_empty()
    }
}

pub fn due_key(task_id: &str, due: &montybrain_core::Due) -> String {
    format!("due:{task_id}@{due}")
}

pub fn digest_key(day: NaiveDate) -> String {
    format!("digest:{day}")
}

/// Decides which notifications to show now. Pure: no clock, no I/O.
pub fn plan(snap: &Snapshot, now: NaiveDateTime, fired: &Fired, cfg: Config) -> Plan {
    let mut out = Plan::default();
    if !cfg.enabled {
        return out;
    }
    let today = now.date();
    let live: Vec<&Task> = snap
        .tasks
        .iter()
        .filter(|t| t.status != TaskStatus::Done && !in_archived_project(snap, t))
        .collect();

    // Timed reminders.
    let mut timed: Vec<(NaiveDateTime, &Task, String)> = live
        .iter()
        .filter_map(|t| {
            let id = t.id.as_deref()?;
            let due = t.due?;
            let at = due.date.and_time(due.time?);
            let key = due_key(id, &due);
            (at <= now && now - at < LATE_WINDOW && !fired.contains(&key)).then_some((at, *t, key))
        })
        .collect();
    timed.sort_by_key(|(at, _, _)| *at);
    if timed.len() > MAX_SEPARATE {
        let titles: Vec<&str> = timed.iter().map(|(_, t, _)| t.title.as_str()).collect();
        out.notes.push(Note {
            kind: NoteKind::DueBatch,
            title: format!("{} – termin minął", count_tasks(timed.len())),
            body: list_titles(&titles),
            keys: timed.into_iter().map(|(_, _, k)| k).collect(),
        });
    } else {
        for (at, t, key) in timed {
            let mut body = context(snap, &t.location);
            if now - at > TimeDelta::minutes(10) {
                body.push_str(&format!(" · termin {}", at.format("%H:%M")));
            }
            out.notes.push(Note {
                kind: NoteKind::Due,
                title: t.title.clone(),
                body,
                keys: vec![key],
            });
        }
    }

    // Daily digest.
    let digest_at =
        NaiveTime::from_hms_opt(u32::from(cfg.digest_hour.min(23)), 0, 0).unwrap_or(NaiveTime::MIN);
    let key = digest_key(today);
    if now.time() >= digest_at && !fired.contains(&key) {
        let mut overdue: Vec<&Task> = Vec::new();
        let mut due_today: Vec<&Task> = Vec::new();
        for t in &live {
            match t.due {
                Some(d) if d.date < today => overdue.push(t),
                Some(d) if d.date == today && d.time.is_none() => due_today.push(t),
                _ => {}
            }
        }
        if overdue.is_empty() && due_today.is_empty() {
            out.silent.push(key);
        } else {
            overdue.sort_by_key(|t| (t.due, priority_rank(t)));
            due_today.sort_by_key(|t| priority_rank(t));
            let title = match (overdue.len(), due_today.len()) {
                (0, n) => format!("Na dziś: {}", count_tasks(n)),
                (o, 0) => format!("Po terminie: {}", count_tasks(o)),
                (o, n) => format!("Na dziś: {} · po terminie: {o}", count_tasks(n)),
            };
            let titles: Vec<&str> = overdue
                .iter()
                .chain(due_today.iter())
                .map(|t| t.title.as_str())
                .collect();
            out.notes.push(Note {
                kind: NoteKind::Digest,
                title,
                body: list_titles(&titles),
                keys: vec![key],
            });
        }
    }
    out
}

fn priority_rank(t: &Task) -> u8 {
    match t.priority {
        montybrain_core::Priority::High => 0,
        montybrain_core::Priority::Normal => 1,
        montybrain_core::Priority::Low => 2,
    }
}

fn in_archived_project(snap: &Snapshot, t: &Task) -> bool {
    t.location
        .project()
        .and_then(|p| snap.project(p))
        .is_some_and(|p| p.state == ProjectState::Archived)
}

/// "WhenUBuy / A/B testy cen", "WhenUBuy", "Skrzynka".
pub fn context(snap: &Snapshot, loc: &TaskLocation) -> String {
    match loc {
        TaskLocation::Inbox => "Skrzynka".into(),
        TaskLocation::Project { project } => project_name(snap, project),
        TaskLocation::Topic { project, topic } => {
            let t = snap
                .topic(project, topic)
                .map_or_else(|| topic.clone(), |t| t.title.clone());
            format!("{} / {t}", project_name(snap, project))
        }
    }
}

fn project_name(snap: &Snapshot, slug: &str) -> String {
    snap.project(slug)
        .map_or_else(|| slug.to_string(), |p| p.name.clone())
}

/// "1 zadanie", "3 zadania", "5 zadań", "12 zadań", "22 zadania".
pub fn count_tasks(n: usize) -> String {
    let word = if n == 1 {
        "zadanie"
    } else if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) {
        "zadania"
    } else {
        "zadań"
    };
    format!("{n} {word}")
}

fn list_titles(titles: &[&str]) -> String {
    let mut body = titles
        .iter()
        .take(BODY_TITLES)
        .copied()
        .collect::<Vec<_>>()
        .join(" · ");
    if titles.len() > BODY_TITLES {
        body.push_str(&format!(" · i {} więcej", titles.len() - BODY_TITLES));
    }
    body
}

// --- scheduler --------------------------------------------------------------

/// Handle for waking the scheduler thread.
pub struct Scheduler {
    tx: Mutex<Sender<()>>,
}

impl Scheduler {
    /// Asks for a check now (vault or settings changed).
    pub fn poke(&self) {
        let _ = crate::lock(&self.tx).send(());
    }
}

pub fn state_path(config_dir: &Path) -> PathBuf {
    config_dir.join(STATE_FILE)
}

/// Starts the scheduler thread and manages a [`Scheduler`] handle.
pub fn start<R: Runtime>(app: &AppHandle<R>, config_dir: PathBuf) {
    let (tx, rx) = mpsc::channel::<()>();
    app.manage(Scheduler { tx: Mutex::new(tx) });
    let app = app.clone();
    let path = state_path(&config_dir);
    let spawned = std::thread::Builder::new()
        .name("montybrain-reminders".into())
        .spawn(move || {
            let mut tracker = Tracker::new(Fired::load(&path));
            let mut wait = FIRST_CHECK;
            loop {
                match rx.recv_timeout(wait) {
                    // Coalesce bursts of pokes.
                    Ok(()) => while rx.try_recv().is_ok() {},
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
                tick(&app, &path, &mut tracker);
                wait = until_next_minute();
            }
        });
    if let Err(e) = spawned {
        crate::log(&format!("reminders: scheduler failed to start: {e}"));
    }
}

/// Sleep until just after the next full minute (due times have minute precision).
fn until_next_minute() -> Duration {
    let now = Local::now();
    let into_minute = Duration::from_secs(u64::from(now.second()))
        + Duration::from_nanos(u64::from(now.nanosecond() % 1_000_000_000));
    Duration::from_secs(60)
        .saturating_sub(into_minute)
        .max(Duration::from_millis(200))
        + Duration::from_millis(300)
}

fn tick<R: Runtime>(app: &AppHandle<R>, path: &Path, tracker: &mut Tracker) {
    // The tray shows "today", which changes at midnight without any file change.
    crate::tray::refresh(app);

    let now = local_now();
    let cfg = crate::native::reminder_config(app);
    let snap = app.state::<crate::state::VaultState>().snapshot();
    tracker.waiting = crate::waiting::tick(app, &snap, now);
    tracker.tick(&snap, now, cfg, &mut |note| show(app, note), &mut |fired| {
        fired.save(path)
    });
}

/// A note whose delivery failed: when to try again and how often we tried.
#[derive(Debug, Clone, Copy)]
struct Retry {
    attempts: u32,
    next_at: NaiveDateTime,
}

/// Delivery attempts per note before it is given up for this run of the app.
pub const MAX_ATTEMPTS: u32 = 6;

/// Wait before attempt `n + 1` after `n` failures: 1, 2, 4, 8, 16 minutes.
fn backoff(attempts: u32) -> TimeDelta {
    TimeDelta::minutes(1 << attempts.saturating_sub(1).min(5))
}

/// The scheduler's state: what was notified, whether `reminders.json` is behind
/// memory, and which notes are being retried.
///
/// * A key is recorded as fired only after the notification was delivered; a
///   failed delivery is retried on later ticks with [`backoff`], at most
///   [`MAX_ATTEMPTS`] times (then it is dropped for this run, not recorded, so
///   a restart may try again; the late window bounds it anyway).
/// * `dirty` stays set until a save succeeds, so idle ticks retry the save.
#[derive(Debug, Default)]
pub struct Tracker {
    fired: Fired,
    dirty: bool,
    retries: BTreeMap<String, Retry>,
    abandoned: BTreeSet<String>,
    /// Titles of the waiting items due for a check (set before each tick).
    pub waiting: Vec<String>,
}

impl Tracker {
    pub fn new(fired: Fired) -> Self {
        Tracker {
            fired,
            ..Default::default()
        }
    }

    /// One scheduler pass. `deliver` shows a note; `save` persists the fired set.
    pub fn tick(
        &mut self,
        snap: &Snapshot,
        now: NaiveDateTime,
        cfg: Config,
        deliver: &mut dyn FnMut(&Note) -> Result<(), String>,
        save: &mut dyn FnMut(&Fired) -> std::io::Result<()>,
    ) {
        let mut plan = plan(snap, now, &self.fired, cfg);
        plan.notes
            .extend(crate::waiting::note(&self.waiting, now, &self.fired, cfg));
        self.dirty |= self.fired.prune(now);

        // Forget retry state of notes that are no longer planned (done, moved).
        let planned: BTreeSet<String> = plan.notes.iter().map(note_id).collect();
        self.retries.retain(|id, _| planned.contains(id));
        self.abandoned.retain(|id| planned.contains(id));

        for note in &plan.notes {
            let id = note_id(note);
            if self.abandoned.contains(&id)
                || self.retries.get(&id).is_some_and(|r| now < r.next_at)
            {
                continue;
            }
            crate::log(&format!(
                "reminder [{:?}] {} — {}",
                note.kind, note.title, note.body
            ));
            match deliver(note) {
                Ok(()) => {
                    self.retries.remove(&id);
                    for k in &note.keys {
                        self.fired.insert(k.clone(), now);
                    }
                    self.dirty = true;
                }
                Err(e) => {
                    let attempts = self.retries.get(&id).map_or(0, |r| r.attempts) + 1;
                    if attempts >= MAX_ATTEMPTS {
                        crate::log(&format!(
                            "notification failed ({e}); giving up on \"{}\" after {attempts} attempts",
                            note.title
                        ));
                        self.retries.remove(&id);
                        self.abandoned.insert(id);
                    } else {
                        crate::log(&format!(
                            "notification failed ({e}); attempt {attempts}/{MAX_ATTEMPTS} for \"{}\"",
                            note.title
                        ));
                        self.retries.insert(
                            id,
                            Retry {
                                attempts,
                                next_at: now + backoff(attempts),
                            },
                        );
                    }
                }
            }
        }
        for k in plan.silent {
            crate::log(&format!("reminder: {k} (nothing to report)"));
            self.fired.insert(k, now);
            self.dirty = true;
        }
        if self.dirty {
            match save(&self.fired) {
                Ok(()) => self.dirty = false,
                Err(e) => crate::log(&format!("reminders: cannot save state: {e} (will retry)")),
            }
        }
    }
}

fn note_id(note: &Note) -> String {
    note.keys.join("|")
}

pub fn local_now() -> NaiveDateTime {
    let now = Local::now().naive_local();
    now.with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(now)
}

fn show<R: Runtime>(app: &AppHandle<R>, note: &Note) -> Result<(), String> {
    crate::notifier::deliver(app, &note.title, &note.body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use montybrain_core::{Due, Priority, Project, Topic, TopicStatus};
    use std::collections::HashMap;

    fn keys(plan: &Plan) -> HashMap<&str, NoteKind> {
        plan.notes
            .iter()
            .flat_map(|n| n.keys.iter().map(move |k| (k.as_str(), n.kind)))
            .collect()
    }

    fn at(s: &str) -> NaiveDateTime {
        montybrain_core::minute::parse(s).unwrap()
    }

    fn task(id: &str, title: &str, due: Option<&str>, loc: TaskLocation) -> Task {
        Task {
            id: Some(id.into()),
            title: title.into(),
            status: TaskStatus::Todo,
            priority: Priority::Normal,
            due: due.map(|d| d.parse::<Due>().unwrap()),
            done: None,
            links: vec![],
            notes: String::new(),
            location: loc,
            line: 1,
        }
    }

    fn snap(tasks: Vec<Task>) -> Snapshot {
        Snapshot {
            projects: vec![
                Project {
                    slug: "wub".into(),
                    name: "WhenUBuy".into(),
                    color: None,
                    state: ProjectState::Active,
                    repos: vec![],
                    description: String::new(),
                    path: "wub/projekt.md".into(),
                },
                Project {
                    slug: "stare".into(),
                    name: "Stare".into(),
                    color: None,
                    state: ProjectState::Archived,
                    repos: vec![],
                    description: String::new(),
                    path: "stare/projekt.md".into(),
                },
            ],
            topics: vec![Topic {
                project: "wub".into(),
                slug: "ab".into(),
                title: "A/B testy".into(),
                status: TopicStatus::InProgress,
                repo: None,
                summary: String::new(),
                created: None,
                updated: None,
                path: "wub/tematy/ab.md".into(),
            }],
            tasks,
            ..Default::default()
        }
    }

    const ON: Config = Config {
        enabled: true,
        digest_hour: 9,
    };

    fn topic() -> TaskLocation {
        TaskLocation::Topic {
            project: "wub".into(),
            topic: "ab".into(),
        }
    }

    #[test]
    fn timed_reminder_fires_at_due_time_once() {
        let s = snap(vec![task(
            "aaaaaa",
            "Otworzyć MR",
            Some("2026-11-13T14:00"),
            topic(),
        )]);
        let mut fired = Fired::default();
        fired.insert(
            digest_key(at("2026-11-13T00:00").date()),
            at("2026-11-13T09:00"),
        );

        assert!(plan(&s, at("2026-11-13T13:59"), &fired, ON).is_empty());

        let p = plan(&s, at("2026-11-13T14:00"), &fired, ON);
        assert_eq!(p.notes.len(), 1);
        let n = &p.notes[0];
        assert_eq!(n.kind, NoteKind::Due);
        assert_eq!(n.title, "Otworzyć MR");
        assert_eq!(n.body, "WhenUBuy / A/B testy");
        assert_eq!(n.keys, vec!["due:aaaaaa@2026-11-13T14:00".to_string()]);

        fired.insert(n.keys[0].clone(), at("2026-11-13T14:00"));
        assert!(plan(&s, at("2026-11-13T14:01"), &fired, ON).is_empty());
    }

    #[test]
    fn rescheduling_rearms_and_done_never_fires() {
        let mut fired = Fired::default();
        fired.insert(
            digest_key(at("2026-11-13T00:00").date()),
            at("2026-11-13T09:00"),
        );
        fired.insert("due:aaaaaa@2026-11-13T14:00".into(), at("2026-11-13T14:00"));

        let moved = snap(vec![task(
            "aaaaaa",
            "Otworzyć MR",
            Some("2026-11-13T16:30"),
            TaskLocation::Inbox,
        )]);
        assert!(plan(&moved, at("2026-11-13T16:29"), &fired, ON).is_empty());
        let p = plan(&moved, at("2026-11-13T16:30"), &fired, ON);
        assert_eq!(p.notes.len(), 1);
        assert_eq!(p.notes[0].body, "Skrzynka");

        let mut done = moved.clone();
        done.tasks[0].status = TaskStatus::Done;
        assert!(plan(&done, at("2026-11-13T16:30"), &fired, ON).is_empty());
    }

    #[test]
    fn late_window_and_late_suffix() {
        let s = snap(vec![task(
            "aaaaaa",
            "Zadzwonić",
            Some("2026-11-13T08:00"),
            TaskLocation::Project {
                project: "wub".into(),
            },
        )]);
        let mut fired = Fired::default();
        fired.insert(
            digest_key(at("2026-11-13T00:00").date()),
            at("2026-11-13T09:00"),
        );
        let p = plan(&s, at("2026-11-13T10:30"), &fired, ON);
        assert_eq!(p.notes[0].body, "WhenUBuy · termin 08:00");
        // 12 h later it is only the digest's business.
        assert!(plan(&s, at("2026-11-13T20:00"), &fired, ON).is_empty());
    }

    #[test]
    fn archived_projects_disabled_and_unnormalized_are_skipped() {
        let mut no_id = task("x", "Bez id", Some("2026-11-13T14:00"), TaskLocation::Inbox);
        no_id.id = None;
        let s = snap(vec![
            task(
                "bbbbbb",
                "W archiwum",
                Some("2026-11-13T14:00"),
                TaskLocation::Project {
                    project: "stare".into(),
                },
            ),
            no_id,
        ]);
        let mut fired = Fired::default();
        fired.insert(
            digest_key(at("2026-11-13T00:00").date()),
            at("2026-11-13T09:00"),
        );
        assert!(plan(&s, at("2026-11-13T14:00"), &fired, ON).is_empty());

        let live = snap(vec![task(
            "cccccc",
            "A",
            Some("2026-11-13T14:00"),
            TaskLocation::Inbox,
        )]);
        let off = Config {
            enabled: false,
            ..ON
        };
        assert!(plan(&live, at("2026-11-13T14:00"), &Fired::default(), off).is_empty());
    }

    #[test]
    fn many_at_once_are_merged() {
        let tasks = (0..5)
            .map(|i| {
                task(
                    &format!("t{i}aaaa"),
                    &format!("Zadanie {i}"),
                    Some("2026-11-13T14:00"),
                    TaskLocation::Inbox,
                )
            })
            .collect();
        let s = snap(tasks);
        let mut fired = Fired::default();
        fired.insert(
            digest_key(at("2026-11-13T00:00").date()),
            at("2026-11-13T09:00"),
        );
        let p = plan(&s, at("2026-11-13T14:00"), &fired, ON);
        assert_eq!(p.notes.len(), 1);
        assert_eq!(p.notes[0].kind, NoteKind::DueBatch);
        assert_eq!(p.notes[0].title, "5 zadań – termin minął");
        assert_eq!(p.notes[0].keys.len(), 5);
        assert!(p.notes[0].body.ends_with("i 2 więcej"));
    }

    #[test]
    fn digest_at_hour_with_overdue_and_today() {
        let s = snap(vec![
            task("d1aaaa", "Na dziś", Some("2026-11-13"), TaskLocation::Inbox),
            task(
                "d2aaaa",
                "Wczoraj",
                Some("2026-11-12T10:00"),
                TaskLocation::Inbox,
            ),
            task(
                "d3aaaa",
                "Dziś o 15",
                Some("2026-11-13T15:00"),
                TaskLocation::Inbox,
            ),
            task("d4aaaa", "Jutro", Some("2026-11-14"), TaskLocation::Inbox),
            task("d5aaaa", "Bez terminu", None, TaskLocation::Inbox),
        ]);
        let fired = Fired::default();
        assert!(plan(&s, at("2026-11-13T08:59"), &fired, ON).is_empty());
        let p = plan(&s, at("2026-11-13T09:00"), &fired, ON);
        assert_eq!(p.notes.len(), 1);
        let n = &p.notes[0];
        assert_eq!(n.kind, NoteKind::Digest);
        assert_eq!(n.title, "Na dziś: 1 zadanie · po terminie: 1");
        assert_eq!(n.body, "Wczoraj · Na dziś");
        assert_eq!(n.keys, vec!["digest:2026-11-13".to_string()]);
    }

    #[test]
    fn digest_after_late_start_once_per_day_and_silent_when_empty() {
        let s = snap(vec![task(
            "d1aaaa",
            "Na dziś",
            Some("2026-11-13"),
            TaskLocation::Inbox,
        )]);
        let mut fired = Fired::default();
        // App started at 17:40, digest hour 9 → shown now.
        let p = plan(&s, at("2026-11-13T17:40"), &fired, ON);
        assert_eq!(p.notes.len(), 1);
        fired.insert(p.notes[0].keys[0].clone(), at("2026-11-13T17:40"));
        assert!(plan(&s, at("2026-11-13T17:41"), &fired, ON).is_empty());
        // Next day: new digest (yesterday's task is now overdue).
        let p = plan(&s, at("2026-11-14T09:00"), &fired, ON);
        assert_eq!(p.notes[0].title, "Po terminie: 1 zadanie");

        let empty = snap(vec![]);
        let p = plan(&empty, at("2026-11-13T09:00"), &Fired::default(), ON);
        assert!(p.notes.is_empty());
        assert_eq!(p.silent, vec!["digest:2026-11-13".to_string()]);

        let custom = Config {
            enabled: true,
            digest_hour: 18,
        };
        assert!(plan(&s, at("2026-11-13T17:59"), &Fired::default(), custom).is_empty());
        assert_eq!(
            plan(&s, at("2026-11-13T18:00"), &Fired::default(), custom)
                .notes
                .len(),
            1
        );
    }

    #[test]
    fn prune_drops_old_entries_only() {
        let mut f = Fired::default();
        f.insert("due:old@2026-11-01T10:00".into(), at("2026-11-01T10:00"));
        f.insert("digest:2026-11-12".into(), at("2026-11-12T09:00"));
        f.fired.insert("broken".into(), "nie data".into());
        assert!(f.prune(at("2026-11-13T09:00")));
        assert_eq!(
            f.fired.keys().collect::<Vec<_>>(),
            vec!["digest:2026-11-12"]
        );
        assert!(!f.prune(at("2026-11-13T09:00")));
    }

    #[test]
    fn fired_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = state_path(dir.path());
        assert_eq!(Fired::load(&path), Fired::default());
        let mut f = Fired::default();
        f.insert("digest:2026-11-13".into(), at("2026-11-13T09:00"));
        f.save(&path).unwrap();
        assert_eq!(Fired::load(&path), f);
    }

    #[test]
    fn polish_plurals() {
        assert_eq!(count_tasks(1), "1 zadanie");
        assert_eq!(count_tasks(2), "2 zadania");
        assert_eq!(count_tasks(5), "5 zadań");
        assert_eq!(count_tasks(12), "12 zadań");
        assert_eq!(count_tasks(22), "22 zadania");
        assert_eq!(count_tasks(0), "0 zadań");
    }

    #[test]
    fn keys_helper() {
        let s = snap(vec![task(
            "aaaaaa",
            "A",
            Some("2026-11-13T14:00"),
            TaskLocation::Inbox,
        )]);
        let p = plan(&s, at("2026-11-13T14:00"), &Fired::default(), ON);
        let k = keys(&p);
        assert_eq!(k.get("due:aaaaaa@2026-11-13T14:00"), Some(&NoteKind::Due));
        assert_eq!(
            k.get("digest:2026-11-13"),
            None,
            "nothing due without time → silent"
        );
        assert_eq!(p.silent, vec!["digest:2026-11-13".to_string()]);
    }

    // --- delivery / persistence ------------------------------------------------

    fn one_due() -> Snapshot {
        snap(vec![task(
            "aaaaaa",
            "Otworzyć MR",
            Some("2026-11-13T14:00"),
            TaskLocation::Inbox,
        )])
    }

    fn digest_done() -> Fired {
        let mut f = Fired::default();
        f.insert("digest:2026-11-13".into(), at("2026-11-13T09:00"));
        f
    }

    #[test]
    fn failed_delivery_is_not_marked_fired_and_is_retried_with_backoff() {
        let s = one_due();
        let mut t = Tracker::new(digest_done());
        let mut shown = 0;
        let mut fail = true;
        let mut saves = 0;
        let mut run = |t: &mut Tracker, when: &str, fail: bool| {
            t.tick(
                &s,
                at(when),
                ON,
                &mut |_| {
                    if fail {
                        Err("denied".into())
                    } else {
                        shown += 1;
                        Ok(())
                    }
                },
                &mut |_| {
                    saves += 1;
                    Ok(())
                },
            );
        };
        run(&mut t, "2026-11-13T14:00", fail);
        assert!(!t.fired.contains("due:aaaaaa@2026-11-13T14:00"));
        // Inside the backoff window nothing is attempted (would show if it were).
        fail = false;
        run(&mut t, "2026-11-13T14:00", fail);
        assert!(!t.fired.contains("due:aaaaaa@2026-11-13T14:00"));
        run(&mut t, "2026-11-13T14:01", fail);
        assert!(t.fired.contains("due:aaaaaa@2026-11-13T14:00"));
        run(&mut t, "2026-11-13T14:02", fail);
        assert_eq!(shown, 1);
        assert_eq!(saves, 1, "saved once, after the successful delivery");
    }

    #[test]
    fn permanently_failing_delivery_gives_up() {
        let s = one_due();
        let mut t = Tracker::new(digest_done());
        let mut attempts = 0;
        for m in 0..120 {
            let now = at("2026-11-13T14:00") + TimeDelta::minutes(m);
            t.tick(
                &s,
                now,
                ON,
                &mut |_| {
                    attempts += 1;
                    Err("denied".into())
                },
                &mut |_| Ok(()),
            );
        }
        assert_eq!(attempts, MAX_ATTEMPTS);
        assert!(!t.fired.contains("due:aaaaaa@2026-11-13T14:00"));
    }

    #[test]
    fn failed_save_is_retried_on_idle_ticks_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = state_path(dir.path());
        let s = one_due();
        let mut shown = 0;
        let mut disk_ok = false;
        let mut t = Tracker::new(digest_done());
        let mut tick = |t: &mut Tracker, when: &str, disk_ok: bool| {
            t.tick(
                &s,
                at(when),
                ON,
                &mut |_| {
                    shown += 1;
                    Ok(())
                },
                &mut |f| {
                    if disk_ok {
                        f.save(&path)
                    } else {
                        Err(std::io::Error::other("disk full"))
                    }
                },
            );
        };
        tick(&mut t, "2026-11-13T14:00", disk_ok);
        assert!(!path.exists());
        // Idle ticks: nothing new to fire, still failing.
        tick(&mut t, "2026-11-13T14:01", disk_ok);
        assert!(!path.exists());
        // Disk recovers: the next idle tick persists.
        disk_ok = true;
        tick(&mut t, "2026-11-13T14:02", disk_ok);
        assert!(path.exists());

        // Restart within the late window: loaded state, no duplicate.
        let mut t2 = Tracker::new(Fired::load(&path));
        tick(&mut t2, "2026-11-13T14:05", disk_ok);
        assert_eq!(shown, 1);
    }
}
