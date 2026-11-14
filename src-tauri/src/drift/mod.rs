//! Drift between the repositories and the map of topics, without a model.
//!
//! The app collects the same hardened git snapshot an update run uses
//! ([`crate::runs::git`]) for every active project with repositories: shortly
//! after launch, then about every [`INTERVAL`] while running, and on demand. It
//! keeps what it saw ([`ProjectObs`], `drift.json` in the config dir) and
//! derives signals locally ([`analyse`]):
//!
//! * **unknown branches**: active branches that match no topic;
//! * **one topic, many branches**: a topic matched by [`SPREAD_BRANCHES`] or
//!   more active branches, or by branches in several repositories;
//! * **repository moved, topic did not**: a matched branch has commits newer
//!   than the topic's last journal entry;
//! * **since the last check**: new commits on the default branches since the
//!   project's last update check.
//!
//! A branch is matched to a topic ([`match_branch`]) by an explicit link
//! ([`links`], `.montybrain/links.json` in the vault), else by its name appearing
//! in the topic's summary or journal, else by shared word stems with the
//! topic's title and slug. Nothing here starts an agent.

pub mod links;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use montybrain_core::chrono::{NaiveDate, NaiveDateTime, TimeDelta};
use montybrain_core::{ProjectState, Snapshot, fold};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use links::{BranchKey, LinkStore};

pub const FILE: &str = "drift.json";
/// Tauri event after a collection (payload: nothing; call `list_drift`).
pub const DRIFT_CHANGED: &str = "drift-changed";
/// First collection after launch.
pub const FIRST: Duration = Duration::from_secs(45);
/// Between collections while the app runs.
pub const INTERVAL: Duration = Duration::from_secs(30 * 60);
/// A branch with a commit this recent (or checked out in a worktree) is active.
pub const ACTIVE_DAYS: i64 = 14;
/// This many active branches of one topic make a "spread" signal.
pub const SPREAD_BRANCHES: usize = 3;

/// Branch names that are a repository's trunk, never a stream of work.
const TRUNKS: &[&str] = &["main", "master", "develop", "dev", "trunk", "HEAD"];

// --- observations ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchObs {
    pub name: String,
    /// Date of its last commit.
    pub last_commit: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeObs {
    pub path: String,
    pub branch: Option<String>,
}

/// One repository as the snapshot showed it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoObs {
    pub repo: String,
    pub path: String,
    /// The snapshot could read it.
    pub ok: bool,
    pub branches: Vec<BranchObs>,
    /// Linked worktrees (the main one excluded).
    pub worktrees: Vec<WorktreeObs>,
    pub default_branch: Option<String>,
    /// Commits on the default branch since the last check (when known).
    pub new_commits: Option<usize>,
}

/// One project's last collection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectObs {
    #[serde(with = "montybrain_core::minute")]
    pub collected_at: NaiveDateTime,
    /// The check `new_commits` counts from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    pub repos: Vec<RepoObs>,
}

/// `drift.json`: the last collection per project and when each branch or
/// worktree was last seen (`project|repo|branch` → minute).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DriftFile {
    pub projects: BTreeMap<String, ProjectObs>,
    pub seen: BTreeMap<String, String>,
}

impl DriftFile {
    pub fn load(path: &Path) -> DriftFile {
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

    /// Records a collection (and when its branches and worktrees were seen).
    pub fn record(&mut self, project: &str, obs: ProjectObs) {
        let at = montybrain_core::minute::format(&obs.collected_at);
        for r in obs.repos.iter().filter(|r| r.ok) {
            for b in &r.branches {
                self.seen
                    .insert(seen_key(project, &r.repo, &b.name), at.clone());
            }
            for w in &r.worktrees {
                self.seen
                    .insert(seen_key(project, &r.repo, &w.path), at.clone());
            }
        }
        self.projects.insert(project.to_string(), obs);
    }

    /// Projects no longer active lose their observations.
    pub fn retain(&mut self, active: &[String]) {
        self.projects.retain(|k, _| active.contains(k));
        self.seen
            .retain(|k, _| active.iter().any(|p| k.starts_with(&format!("{p}|"))));
    }
}

pub fn seen_key(project: &str, repo: &str, what: &str) -> String {
    format!("{project}|{repo}|{what}")
}

/// A remote-tracking name as the local branch it mirrors (`origin/x` → `x`).
fn local_name(name: &str) -> &str {
    for remote in ["origin/", "upstream/", "remotes/origin/"] {
        if let Some(rest) = name.strip_prefix(remote) {
            return rest;
        }
    }
    name
}

pub fn is_trunk(name: &str) -> bool {
    TRUNKS.contains(&name)
}

/// The repositories of a snapshot text ([`crate::runs::git::Collector::collect`]):
/// branches with their last commit date, linked worktrees, the default branch.
pub fn parse_snapshot(text: &str) -> Vec<RepoObs> {
    let mut out: Vec<RepoObs> = Vec::new();
    let mut section = "";
    let mut wt: Option<WorktreeObs> = None;
    let mut first_wt = true;
    let flush = |out: &mut Vec<RepoObs>, wt: &mut Option<WorktreeObs>, first: &mut bool| {
        if let (Some(w), Some(r)) = (wt.take(), out.last_mut()) {
            if !*first {
                r.worktrees.push(w);
            }
            *first = false;
        }
    };
    for line in text.lines() {
        if let Some(h) = line
            .strip_prefix("=== ")
            .and_then(|h| h.strip_suffix(" ==="))
        {
            flush(&mut out, &mut wt, &mut first_wt);
            let repo = h.rsplit_once(" / ").map_or(h, |(_, r)| r).to_string();
            out.push(RepoObs {
                repo,
                ok: true,
                ..Default::default()
            });
            section = "";
            first_wt = true;
            continue;
        }
        let Some(r) = out.last_mut() else { continue };
        if let Some(p) = line
            .strip_prefix("Ścieżka: ")
            .filter(|_| section.is_empty())
        {
            r.path = p.to_string();
            continue;
        }
        if line.starts_with("(pominięto: ") && section.is_empty() {
            r.ok = false;
            continue;
        }
        if let Some(cmd) = line.strip_prefix("$ ") {
            flush(&mut out, &mut wt, &mut first_wt);
            section = if cmd.starts_with("git branch") {
                "branch"
            } else if cmd.starts_with("git worktree") {
                "worktree"
            } else {
                "other"
            };
            continue;
        }
        match section {
            "branch" => {
                let Some((name, date)) = line.rsplit_once(' ') else {
                    continue;
                };
                let Ok(date) = NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
                    continue;
                };
                let name = local_name(name.trim());
                if name.is_empty() || name == "origin" || name.contains(' ') {
                    continue;
                }
                match r.branches.iter_mut().find(|b| b.name == name) {
                    Some(b) => b.last_commit = b.last_commit.max(date),
                    None => r.branches.push(BranchObs {
                        name: name.to_string(),
                        last_commit: date,
                    }),
                }
            }
            "worktree" => {
                if let Some(p) = line.strip_prefix("worktree ") {
                    flush(&mut out, &mut wt, &mut first_wt);
                    wt = Some(WorktreeObs {
                        path: p.to_string(),
                        branch: None,
                    });
                } else if let (Some(b), Some(w)) = (line.strip_prefix("branch "), wt.as_mut()) {
                    w.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
                }
            }
            _ => {}
        }
    }
    flush(&mut out, &mut wt, &mut first_wt);
    for r in &mut out {
        r.default_branch = ["main", "master", "trunk", "develop"]
            .iter()
            .find(|d| r.branches.iter().any(|b| b.name == **d))
            .map(|d| d.to_string());
    }
    out
}

// --- matching ------------------------------------------------------------------------------

/// Words in branch names that say what kind of work, not which.
const BRANCH_NOISE: &[&str] = &[
    "feature",
    "feat",
    "fix",
    "bugfix",
    "hotfix",
    "wip",
    "chore",
    "refactor",
    "release",
    "test",
    "tests",
    "tmp",
    "temp",
    "dev",
    "draft",
    "spike",
    "poc",
    "experiment",
    "update",
    "add",
    "remove",
    "new",
    "old",
    "codex",
    "claude",
    "agent",
    "task",
    "issue",
    "the",
    "and",
    "for",
    "with",
    "into",
    "from",
];

/// Short Polish words in topic titles that say nothing about the topic.
const TITLE_NOISE: &[&str] = &[
    "dla", "oraz", "przez", "jako", "nowy", "nowa", "nowe", "temat", "projekt", "zadania", "and",
    "the", "for",
];

/// A ticket number or hash-like token (`abc-123`, `123`, `jira42`, `a1b2c3d`).
fn is_ticket(w: &str) -> bool {
    let digits = w.chars().filter(char::is_ascii_digit).count();
    if w.len() >= 6 && digits >= 2 && w.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    digits > 0 && (digits * 2 >= w.len() || w.chars().last().is_some_and(|c| c.is_ascii_digit()))
}

fn tokens(s: &str) -> Vec<String> {
    fold(s)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// The significant words of a branch name (folded; trunk/noise words and
/// ticket numbers dropped).
pub fn branch_words(name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in tokens(name) {
        if w.chars().count() < 3 || BRANCH_NOISE.contains(&w.as_str()) || is_ticket(&w) {
            continue;
        }
        if !out.contains(&w) {
            out.push(w);
        }
    }
    out
}

/// The significant words of a topic's title and slug.
pub fn topic_words(title: &str, slug: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in tokens(title).into_iter().chain(tokens(slug)) {
        if w.chars().count() < 3 || TITLE_NOISE.contains(&w.as_str()) || is_ticket(&w) {
            continue;
        }
        if !out.contains(&w) {
            out.push(w);
        }
    }
    out
}

/// A word's stem: whole when short, else without its last two letters (at
/// least 4 kept), so inflected and derived forms share it.
fn stem(w: &str) -> String {
    let n = w.chars().count();
    if n <= 4 {
        w.to_string()
    } else {
        w.chars().take((n - 2).clamp(4, 8)).collect()
    }
}

/// Whether two significant words count as the same word.
fn same_word(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    // `w` starts with stem `st`: a short stem only for a short word
    // ("bazy" fits "baza"-like forms, not "bazylia").
    let fits = |w: &str, st: &str| {
        let (n, k) = (w.chars().count(), st.chars().count());
        k >= 4 && w.starts_with(st) && (k >= 5 || n <= k + 2)
    };
    let (sa, sb) = (stem(a), stem(b));
    fits(b, &sa) || fits(a, &sb)
}

/// Whether `text` names the branch `name` as a whole (not as the start of a
/// longer branch name: `feature/pos` is not in `feature/pos-offline`).
fn mentions_name(text: &str, name: &str) -> bool {
    let part = |c: char| c.is_alphanumeric() || "-/_.".contains(c);
    text.match_indices(name).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + name.len()..].chars().next();
        // A trailing period ends a sentence, not the name.
        let after_ok = match after {
            None => true,
            Some('.') => text[i + name.len() + 1..]
                .chars()
                .next()
                .is_none_or(|c| !part(c)),
            Some(c) => !part(c),
        };
        before.is_none_or(|c| !part(c)) && after_ok
    })
}

/// What a topic offers to matching.
#[derive(Debug, Clone)]
pub struct TopicInfo {
    pub slug: String,
    pub title: String,
    /// Folded summary and journal texts.
    pub text: String,
    pub last_entry: Option<NaiveDate>,
}

impl TopicInfo {
    pub fn of(snap: &Snapshot, project: &str) -> Vec<TopicInfo> {
        snap.topics_of(project)
            .map(|t| {
                let journal = snap.journal_of(project, &t.slug);
                let mut text = fold(&t.summary);
                for j in &journal {
                    text.push('\n');
                    text.push_str(&fold(&j.text));
                }
                TopicInfo {
                    slug: t.slug.clone(),
                    title: t.title.clone(),
                    text,
                    last_entry: journal.iter().map(|j| j.at.date()).max(),
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    /// Linked by the agent or the user.
    Link,
    /// Its name appears in the topic's summary or journal.
    Mention,
    /// Shared words with the topic's title.
    Words,
}

/// The topic `branch` (of `repo` in `project`) belongs to, and why; `None`
/// when no topic or more than one fits equally well.
pub fn match_branch(
    project: &str,
    repo: &str,
    branch: &str,
    topics: &[TopicInfo],
    links: &LinkStore,
) -> Option<(String, MatchKind)> {
    if let Some(t) = links.topic_of(&BranchKey::new(project, repo, branch))
        && topics.iter().any(|x| x.slug == t)
    {
        return Some((t.to_string(), MatchKind::Link));
    }
    // The full name in a summary or journal (long enough not to be a common word).
    let name = fold(branch);
    if name.chars().count() >= 6 && name.contains(['/', '-', '_']) {
        let mentions: Vec<&TopicInfo> = topics
            .iter()
            .filter(|t| mentions_name(&t.text, &name))
            .collect();
        if let [one] = mentions.as_slice() {
            return Some((one.slug.clone(), MatchKind::Mention));
        }
        if mentions.len() > 1 {
            // The most recently written one.
            if let Some(t) = mentions.iter().max_by_key(|t| t.last_entry) {
                return Some((t.slug.clone(), MatchKind::Mention));
            }
        }
    }
    let words = branch_words(branch);
    if words.is_empty() {
        return None;
    }
    let mut best: Option<(&TopicInfo, usize)> = None;
    let mut tie = false;
    for t in topics {
        let tw = topic_words(&t.title, &t.slug);
        let score = words
            .iter()
            .filter(|w| tw.iter().any(|x| same_word(w, x)))
            .count();
        if score == 0 {
            continue;
        }
        match best {
            Some((_, s)) if score < s => {}
            Some((_, s)) if score == s => tie = true,
            _ => {
                best = Some((t, score));
                tie = false;
            }
        }
    }
    match best {
        Some((t, _)) if !tie => Some((t.slug.clone(), MatchKind::Words)),
        _ => None,
    }
}

// --- signals ---------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchView {
    pub repo: String,
    pub branch: String,
    pub last_commit: NaiveDate,
    /// The worktree it is checked out in, if any.
    pub worktree: Option<String>,
    pub topic: Option<String>,
    pub matched_by: Option<MatchKind>,
    pub active: bool,
    pub ignored: bool,
    /// When the app last saw it (minute).
    pub seen: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Spread {
    pub topic: String,
    pub branches: usize,
    pub repos: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Moved {
    pub topic: String,
    pub repo: String,
    pub branch: String,
    pub last_commit: NaiveDate,
    pub last_entry: Option<NaiveDate>,
}

/// What `list_drift` returns per project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectDrift {
    pub project: String,
    pub collected_at: Option<String>,
    /// Every non-trunk branch seen, matched or not.
    pub branches: Vec<BranchView>,
    /// Active, unmatched, not ignored.
    pub unknown: Vec<BranchView>,
    pub spread: Vec<Spread>,
    pub moved: Vec<Moved>,
    /// New commits on the default branches since `since` (the last check).
    pub new_commits: Option<usize>,
    pub since: Option<String>,
    /// Explicit links of this project (repo, branch, topic).
    pub links: Vec<links::Link>,
}

/// Derives the signals of one project from its last collection, the vault
/// as it is now, and the links / ignores.
pub fn analyse(
    project: &str,
    obs: Option<&ProjectObs>,
    seen: &BTreeMap<String, String>,
    snap: &Snapshot,
    store: &LinkStore,
    today: NaiveDate,
) -> ProjectDrift {
    let topics = TopicInfo::of(snap, project);
    let mut out = ProjectDrift {
        project: project.to_string(),
        collected_at: obs.map(|o| montybrain_core::minute::format(&o.collected_at)),
        branches: Vec::new(),
        unknown: Vec::new(),
        spread: Vec::new(),
        moved: Vec::new(),
        new_commits: None,
        since: obs.and_then(|o| o.since.clone()),
        links: store.of_project(project),
    };
    let Some(obs) = obs else { return out };
    let cutoff = today - TimeDelta::days(ACTIVE_DAYS);
    let mut commits = None::<usize>;
    for r in obs.repos.iter().filter(|r| r.ok) {
        if let Some(n) = r.new_commits {
            commits = Some(commits.unwrap_or(0) + n);
        }
        for b in &r.branches {
            if is_trunk(&b.name) || Some(&b.name) == r.default_branch.as_ref() {
                continue;
            }
            let worktree = r
                .worktrees
                .iter()
                .find(|w| w.branch.as_deref() == Some(b.name.as_str()))
                .map(|w| w.path.clone());
            let m = match_branch(project, &r.repo, &b.name, &topics, store);
            out.branches.push(BranchView {
                repo: r.repo.clone(),
                branch: b.name.clone(),
                last_commit: b.last_commit,
                active: b.last_commit >= cutoff || worktree.is_some(),
                worktree,
                topic: m.as_ref().map(|(t, _)| t.clone()),
                matched_by: m.map(|(_, k)| k),
                ignored: store.is_ignored(&BranchKey::new(project, &r.repo, &b.name)),
                seen: seen.get(&seen_key(project, &r.repo, &b.name)).cloned(),
            });
        }
    }
    out.new_commits = commits;
    out.unknown = out
        .branches
        .iter()
        .filter(|b| b.active && b.topic.is_none() && !b.ignored)
        .cloned()
        .collect();
    // Per topic: its active branches and repositories.
    let mut per: BTreeMap<&str, Vec<&BranchView>> = BTreeMap::new();
    for b in out.branches.iter().filter(|b| b.active) {
        if let Some(t) = &b.topic {
            per.entry(t.as_str()).or_default().push(b);
        }
    }
    for (topic, bs) in &per {
        let mut repos: Vec<&str> = bs.iter().map(|b| b.repo.as_str()).collect();
        repos.sort_unstable();
        repos.dedup();
        if bs.len() >= SPREAD_BRANCHES || repos.len() >= 2 {
            out.spread.push(Spread {
                topic: topic.to_string(),
                branches: bs.len(),
                repos: repos.len(),
            });
        }
    }
    for b in out.branches.iter().filter(|b| b.active) {
        let Some(t) = topics.iter().find(|t| Some(&t.slug) == b.topic.as_ref()) else {
            continue;
        };
        if t.last_entry.is_none_or(|e| b.last_commit > e)
            && !out
                .moved
                .iter()
                .any(|m| m.topic == t.slug && m.last_commit >= b.last_commit)
        {
            out.moved.retain(|m| m.topic != t.slug);
            out.moved.push(Moved {
                topic: t.slug.clone(),
                repo: b.repo.clone(),
                branch: b.branch.clone(),
                last_commit: b.last_commit,
                last_entry: t.last_entry,
            });
        }
    }
    out
}

// --- collection (Tauri glue) -----------------------------------------------------------------

/// Managed state: the observations and the collector thread's handle.
pub struct Drift {
    file: Mutex<DriftFile>,
    path: PathBuf,
    running: AtomicBool,
    tx: Mutex<Sender<()>>,
}

impl Drift {
    /// Asks for a collection now (ignored while one is running).
    pub fn poke(&self) {
        let _ = crate::lock(&self.tx).send(());
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

pub fn path(config_dir: &Path) -> PathBuf {
    config_dir.join(FILE)
}

/// Starts the collector thread: first run after [`FIRST`], then every
/// [`INTERVAL`] or when poked. One collection at a time; it runs on its own
/// thread and never holds the vault or the store locks while git runs.
pub fn start<R: Runtime>(app: &AppHandle<R>, config_dir: &Path) {
    let (tx, rx) = mpsc::channel::<()>();
    let path = path(config_dir);
    app.manage(Drift {
        file: Mutex::new(DriftFile::load(&path)),
        path,
        running: AtomicBool::new(false),
        tx: Mutex::new(tx),
    });
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("montybrain-drift".into())
        .spawn(move || {
            let mut wait = FIRST;
            loop {
                match rx.recv_timeout(wait) {
                    Ok(()) => while rx.try_recv().is_ok() {},
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
                collect(&app);
                wait = INTERVAL;
            }
        });
    if let Err(e) = spawned {
        crate::log(&format!("drift: collector failed to start: {e}"));
    }
}

fn collect<R: Runtime>(app: &AppHandle<R>) {
    let Some(drift) = app.try_state::<Drift>() else {
        return;
    };
    if drift.running.swap(true, Ordering::SeqCst) {
        return;
    }
    let snap = app.state::<crate::state::VaultState>().snapshot();
    let runs = app.state::<crate::runs::Runs>().0.clone();
    let active: Vec<String> = snap
        .projects
        .iter()
        .filter(|p| p.state == ProjectState::Active && !p.repos.is_empty())
        .map(|p| p.slug.clone())
        .collect();
    for slug in &active {
        let since = crate::updates::last_check(app, slug);
        let repos = runs.drift_obs(&snap.projects, slug, since.as_deref());
        let obs = ProjectObs {
            collected_at: crate::reminders::local_now(),
            since,
            repos,
        };
        crate::lock(&drift.file).record(slug, obs);
    }
    {
        let mut f = crate::lock(&drift.file);
        f.retain(&active);
        if let Err(e) = f.save(&drift.path) {
            crate::log(&format!("drift: cannot save {}: {e}", drift.path.display()));
        }
    }
    drift.running.store(false, Ordering::SeqCst);
    if let Err(e) = app.emit(DRIFT_CHANGED, ()) {
        crate::log(&format!("emit {DRIFT_CHANGED} failed: {e}"));
    }
}

/// Every active project's drift, from the last collection and the vault now.
pub fn list<R: Runtime>(app: &AppHandle<R>) -> Vec<ProjectDrift> {
    let Some(drift) = app.try_state::<Drift>() else {
        return Vec::new();
    };
    let state = app.state::<crate::state::VaultState>();
    let snap = state.snapshot();
    let store = links::load(state.vault().root());
    let file = crate::lock(&drift.file).clone();
    let today = crate::reminders::local_now().date();
    snap.projects
        .iter()
        .filter(|p| p.state == ProjectState::Active && !p.repos.is_empty())
        .map(|p| {
            analyse(
                &p.slug,
                file.projects.get(&p.slug),
                &file.seen,
                &snap,
                &store,
                today,
            )
        })
        .collect()
}

pub fn emit_changed<R: Runtime>(app: &AppHandle<R>) {
    if let Err(e) = app.emit(DRIFT_CHANGED, ()) {
        crate::log(&format!("emit {DRIFT_CHANGED} failed: {e}"));
    }
}

#[cfg(test)]
mod tests;
