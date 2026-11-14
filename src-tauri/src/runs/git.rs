//! The read-only git snapshot of a project's repositories (`postep` recipe,
//! background mode). The app collects it itself, so the agent needs no shell
//! and no file access; the snapshot goes into the prompt as delimited data.
//!
//! Every git call has fixed arguments (no shell) and is hardened so that a
//! repository's own configuration cannot make git run commands:
//! `--no-pager`, `--no-optional-locks`, `core.fsmonitor=false`,
//! `core.hooksPath=/dev/null`, `log.showSignature=false`, filter drivers
//! (`filter.<name>.clean/smudge/process`, which `git status` runs for files
//! whose stat data changed) blanked out by name, submodules ignored, a
//! scrubbed environment and stdin `/dev/null`. Each call has a deadline (the
//! process group is killed when it passes) and capped output; one repository
//! failing only adds a note.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use super::proc::{Output, run_bounded};
use super::providers::SYSTEM_PATH;

pub const GIT_TIMEOUT: Duration = Duration::from_secs(5);
/// The whole snapshot stops collecting after this.
pub const SNAPSHOT_BUDGET: Duration = Duration::from_secs(60);
/// Raw output read per git call (the rest is drained and dropped).
const READ_CAP: usize = 64 * 1024;
const STDERR_CAP: usize = 2 * 1024;

/// Output limits of a snapshot.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Repositories (existing directories) that git is run in.
    pub max_repos: usize,
    pub repo_bytes: usize,
    pub total_bytes: usize,
    pub status_lines: usize,
    pub branch_lines: usize,
    /// Lines of `git worktree list --porcelain` (about four per worktree).
    pub worktree_lines: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_repos: 12,
            repo_bytes: 8 * 1024,
            total_bytes: 48 * 1024,
            status_lines: 40,
            branch_lines: 20,
            worktree_lines: 60,
        }
    }
}

/// Settings fixed for every git call; part of every invocation.
pub const HARDENING: &[&str] = &[
    "core.fsmonitor=false",
    "core.hooksPath=/dev/null",
    "log.showSignature=false",
    // No transport may start: a repository with a promisor remote would
    // otherwise fetch missing objects and run its configured ssh command.
    "protocol.allow=never",
    "color.ui=never",
    "color.status=never",
    "color.branch=never",
];

/// Where git is looked up, in order. `/usr/bin/git` is only a shim that pops
/// up the "install developer tools" dialog when they are missing, so it is
/// used only when a developer directory was selected explicitly.
pub fn git_candidates() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = [
        "/opt/homebrew/bin/git",
        "/usr/local/bin/git",
        "/Library/Developer/CommandLineTools/usr/bin/git",
        "/Applications/Xcode.app/Contents/Developer/usr/bin/git",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    if Path::new("/var/db/xcode_select_link").exists() {
        v.push("/usr/bin/git".into());
    }
    v
}

pub fn find_git(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| is_executable(p)).cloned()
}

fn is_executable(p: &Path) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// One repository entry of a project, as written in the vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRef {
    /// `project / repo name`.
    pub label: String,
    /// `~/` expanded.
    pub path: PathBuf,
}

pub struct Collector<'a> {
    pub git: Option<&'a Path>,
    pub home: &'a Path,
    pub timeout: Duration,
    pub budget: Duration,
    pub limits: Limits,
    pub cancelled: &'a dyn Fn() -> bool,
}

#[derive(Debug, Default)]
pub struct GitSnapshot {
    /// The data block for the prompt.
    pub text: String,
    /// Problems, for the run log.
    pub notes: Vec<String>,
    /// Repositories whose state was collected.
    pub collected: usize,
    pub cancelled: bool,
}

/// The linked worktrees listed in a snapshot text (`git worktree list
/// --porcelain` sections), with the repository name of their section
/// (`=== project / repo ===`). Includes each repository's main worktree.
pub fn worktrees(text: &str) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut repo = String::new();
    let mut inside = false;
    for line in text.lines() {
        if let Some(h) = line
            .strip_prefix("=== ")
            .and_then(|h| h.strip_suffix(" ==="))
        {
            repo = h.rsplit_once(" / ").map_or(h, |(_, r)| r).to_string();
            inside = false;
        } else if line.starts_with("$ ") {
            inside = line.starts_with("$ git worktree list");
        } else if inside && let Some(p) = line.strip_prefix("worktree ") {
            let p = PathBuf::from(p);
            if p.is_absolute() && !out.iter().any(|(_, x)| x == &p) {
                out.push((repo.clone(), p));
            }
        }
    }
    out
}

/// Cuts `s` to at most `max` bytes on a char boundary.
fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Untrusted bytes as text: control characters (ANSI escapes included) are
/// replaced, only newlines and tabs stay.
fn clean(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .map(|c| {
            if c == '\n' || c == '\t' || !c.is_control() {
                c
            } else {
                '\u{FFFD}'
            }
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn human(d: Duration) -> String {
    if d < Duration::from_secs(1) {
        format!("{} ms", d.as_millis())
    } else {
        format!("{} s", d.as_secs())
    }
}

impl Collector<'_> {
    fn git(
        &self,
        git: &Path,
        dir: &Path,
        extra: &[String],
        args: &[&str],
    ) -> Result<Output, String> {
        let mut cmd = Command::new(git);
        cmd.arg("--no-pager").arg("--no-optional-locks");
        for c in HARDENING
            .iter()
            .copied()
            .chain(extra.iter().map(String::as_str))
        {
            cmd.arg("-c").arg(c);
        }
        cmd.args(args)
            .current_dir(dir)
            .env_clear()
            .env("HOME", self.home)
            .env("PATH", SYSTEM_PATH)
            .env("LC_ALL", "C")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_NO_LAZY_FETCH", "1")
            .env("GIT_SSH_COMMAND", "/usr/bin/false")
            .env("GIT_SSH", "/usr/bin/false")
            .env("GIT_PAGER", "cat")
            .env("PAGER", "cat");
        run_bounded(cmd, self.timeout, READ_CAP, STDERR_CAP, self.cancelled)
            .map_err(|e| format!("nie udało się uruchomić gita: {e}"))
    }

    /// Why a git call did not give usable output.
    fn failure(&self, o: &Output) -> String {
        if o.timed_out {
            return format!("git nie odpowiedział w {}", human(self.timeout));
        }
        if o.cancelled {
            return "przerwano".into();
        }
        let first = clean(&o.stderr);
        let first = first.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        let code = o
            .status
            .and_then(|s| s.code())
            .map_or_else(|| "przerwany".to_string(), |c| format!("kod {c}"));
        if first.is_empty() {
            format!("git: {code}")
        } else {
            format!("git: {code}: {}", cut(first, 200))
        }
    }

    /// Filter driver names configured for `dir` (any scope); `Err` when one
    /// cannot be overridden on the command line.
    fn filter_overrides(&self, git: &Path, dir: &Path) -> Result<Vec<String>, String> {
        let o = self.git(
            git,
            dir,
            &[],
            &[
                "config",
                "--null",
                "--name-only",
                "--get-regexp",
                r"^filter\.",
            ],
        )?;
        // Exit code 1: no such keys.
        if !o.success() && o.status.and_then(|s| s.code()) != Some(1) {
            return Err(self.failure(&o));
        }
        if o.stdout_truncated {
            return Err("za dużo wpisów filter.* w konfiguracji".into());
        }
        let mut names: Vec<String> = Vec::new();
        for key in o.stdout.split(|&b| b == 0).filter(|k| !k.is_empty()) {
            // A lossy conversion would change the name, and the override below
            // would then miss the real filter: refuse instead.
            let key = std::str::from_utf8(key)
                .map_err(|_| "nazwa filtra w konfiguracji nie jest poprawnym UTF-8".to_string())?;
            let name = key
                .strip_prefix("filter.")
                .and_then(|rest| rest.rsplit_once('.'))
                .map(|(name, _)| name.to_string())
                .ok_or_else(|| "nietypowy wpis filter.* w konfiguracji".to_string())?;
            if name.contains(['=', '\n', '\0']) {
                return Err("nietypowa nazwa filtra w konfiguracji".into());
            }
            if !names.contains(&name) {
                names.push(name);
            }
        }
        Ok(names
            .iter()
            .flat_map(|n| {
                [
                    format!("filter.{n}.clean="),
                    format!("filter.{n}.smudge="),
                    format!("filter.{n}.process="),
                    format!("filter.{n}.required=false"),
                ]
            })
            .collect())
    }

    /// `Err` when git's working tree for `dir` is not `dir` itself (for example
    /// `core.worktree` in the repository's own config points elsewhere):
    /// `status` would then list files from outside the selected repository.
    fn worktree_is_dir(&self, git: &Path, dir: &Path) -> Result<(), String> {
        let o = self.git(git, dir, &[], &["rev-parse", "--show-toplevel"])?;
        if !o.success() {
            return Err(self.failure(&o));
        }
        let top = std::str::from_utf8(&o.stdout)
            .map_err(|_| "ścieżka katalogu roboczego nie jest poprawnym UTF-8".to_string())?;
        let top = top.strip_suffix('\n').unwrap_or(top);
        match (std::fs::canonicalize(top), std::fs::canonicalize(dir)) {
            (Ok(a), Ok(b)) if a == b => Ok(()),
            _ => Err("katalog roboczy repozytorium wskazuje poza podaną ścieżkę".into()),
        }
    }

    /// One command's output, line-capped, or the reason it failed.
    fn command(
        &self,
        git: &Path,
        dir: &Path,
        extra: &[String],
        label: &str,
        args: &[&str],
        max_lines: Option<usize>,
    ) -> String {
        let mut out = format!("\n$ {label}\n");
        match self.git(git, dir, extra, args) {
            Err(e) => out.push_str(&format!("(nie udało się: {e})\n")),
            Ok(o) if !o.success() => {
                out.push_str(&format!("(nie udało się: {})\n", self.failure(&o)));
            }
            Ok(o) => {
                let text = clean(&o.stdout);
                let lines: Vec<&str> = text.lines().collect();
                let keep = max_lines.unwrap_or(usize::MAX).min(lines.len());
                if lines.is_empty() {
                    out.push_str("(brak)\n");
                }
                for l in &lines[..keep] {
                    out.push_str(l);
                    out.push('\n');
                }
                if keep < lines.len() {
                    out.push_str(&format!("… (+{} linii pominięto)\n", lines.len() - keep));
                } else if o.stdout_truncated {
                    out.push_str("… (obcięto)\n");
                }
            }
        }
        out
    }

    /// The section of one existing directory, or why it was skipped.
    fn repo(&self, git: &Path, dir: &Path) -> Result<String, String> {
        // Only a repository's own root is read: `.git` must be a real directory
        // here. A `.git` file (`gitdir: …`, as in linked worktrees) or a symlink
        // would make git read history from somewhere else.
        match std::fs::symlink_metadata(dir.join(".git")) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => {
                return Err(
                    "`.git` nie jest tu katalogiem (powiązane drzewo robocze albo przekierowanie); \
                     wskaż główny katalog repozytorium"
                        .into(),
                );
            }
            Err(_) => return Err("to nie jest repozytorium git z katalogiem roboczym".into()),
        }
        let o = self.git(git, dir, &[], &["rev-parse", "--is-inside-work-tree"])?;
        if !o.success() || clean(&o.stdout) != "true" {
            return Err(if o.timed_out || o.cancelled {
                self.failure(&o)
            } else {
                "to nie jest repozytorium git z katalogiem roboczym".into()
            });
        }
        let mut out = String::new();
        match self
            .worktree_is_dir(git, dir)
            .and_then(|()| self.filter_overrides(git, dir))
        {
            Ok(filters) => {
                out += &self.command(
                    git,
                    dir,
                    &filters,
                    "git status --short --branch",
                    &[
                        "status",
                        "--short",
                        "--branch",
                        "--ignore-submodules=all",
                        "--untracked-files=normal",
                    ],
                    Some(self.limits.status_lines),
                )
            }
            Err(e) => out.push_str(&format!(
                "\n$ git status --short --branch\n(pominięto: {e})\n"
            )),
        }
        out += &self.command(
            git,
            dir,
            &[],
            "git log --oneline -15",
            &["log", "--no-color", "--oneline", "-15"],
            None,
        );
        out += &self.command(
            git,
            dir,
            &[],
            "git log --all --since=14.days (hash, data, autor, opis)",
            &[
                "log",
                "--no-color",
                "--all",
                "--since=14.days",
                "--format=%h %ad %an %s",
                "--date=short",
                "-40",
            ],
            None,
        );
        out += &self.command(
            git,
            dir,
            &[],
            "git branch -a --sort=-committerdate (gałąź, data ostatniego commita)",
            &[
                "branch",
                "--no-color",
                "-a",
                "--sort=-committerdate",
                "--format=%(refname:short) %(committerdate:short)",
            ],
            Some(self.limits.branch_lines),
        );
        // Linked worktrees (agents work in them): which branch is checked out
        // where. Read from the main repository only; git is never run inside
        // the worktree directories, so their uncommitted changes stay unseen.
        out += &self.command(
            git,
            dir,
            &[],
            "git worktree list --porcelain (drzewa robocze: ścieżka, HEAD, gałąź)",
            &["worktree", "list", "--porcelain"],
            Some(self.limits.worktree_lines),
        );
        Ok(out)
    }

    /// Commits on local branch `branch` of the repository at `dir` newer than
    /// `since` (`git rev-list --count`), hardened like the snapshot. `None`
    /// when it cannot be told (not a repository root, an odd branch name, git
    /// failed).
    pub fn commits_since(&self, dir: &Path, branch: &str, since: &str) -> Option<usize> {
        let git = self.git?;
        let safe = |s: &str| {
            !s.is_empty()
                && !s.starts_with('-')
                && !s.contains("..")
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "/_.-".contains(c))
        };
        let since_ok = since
            .chars()
            .all(|c| c.is_ascii_digit() || "-:T ".contains(c));
        if !safe(branch) || !since_ok || !dir.is_absolute() {
            return None;
        }
        // Same rule as the snapshot: only a repository's own root.
        if !std::fs::symlink_metadata(dir.join(".git")).is_ok_and(|m| m.is_dir()) {
            return None;
        }
        let since_arg = format!("--since={since}");
        let rev = format!("refs/heads/{branch}");
        let o = self
            .git(
                git,
                dir,
                &[],
                &["rev-list", "--count", &since_arg, &rev, "--"],
            )
            .ok()?;
        if !o.success() {
            return None;
        }
        clean(&o.stdout).trim().parse().ok()
    }

    /// Collects the snapshot of `repos` (in order, at most `max_repos`).
    pub fn collect(&self, repos: &[RepoRef]) -> GitSnapshot {
        let mut snap = GitSnapshot::default();
        if repos.is_empty() {
            snap.text = "Wybrane projekty nie mają w MontyBrain przypisanych repozytoriów.".into();
            return snap;
        }
        let Some(git) = self.git else {
            let msg = "Nie znaleziono programu git (szukano w /opt/homebrew/bin, /usr/local/bin \
i w narzędziach deweloperskich Apple); stanu repozytoriów nie zebrano.";
            snap.text = msg.into();
            snap.notes.push(msg.into());
            return snap;
        };
        let started = Instant::now();
        let mut checked = 0usize;
        let mut text = String::new();
        for (i, r) in repos.iter().enumerate() {
            if (self.cancelled)() {
                snap.cancelled = true;
                break;
            }
            if started.elapsed() >= self.budget {
                let msg = format!(
                    "Przerwano zbieranie stanu repozytoriów po {}; pominięto {}.",
                    human(self.budget),
                    repos.len() - i
                );
                text.push_str(&format!("\n{msg}\n"));
                snap.notes.push(msg);
                break;
            }
            let head = format!("=== {} ===\nŚcieżka: {}\n", r.label, r.path.display());
            let skip = if !r.path.is_absolute() {
                Some("ścieżka nie jest bezwzględna".to_string())
            } else if !r.path.exists() {
                Some("ścieżka nie istnieje".to_string())
            } else if !r.path.is_dir() {
                Some("to nie jest katalog".to_string())
            } else if checked >= self.limits.max_repos {
                Some(format!(
                    "limit {} repozytoriów na jedną akcję",
                    self.limits.max_repos
                ))
            } else {
                None
            };
            let section = match skip {
                Some(why) => Err(why),
                None => {
                    checked += 1;
                    self.repo(git, &r.path)
                }
            };
            let section = match section {
                Ok(body) => {
                    snap.collected += 1;
                    let mut s = format!("{head}{body}");
                    if s.len() > self.limits.repo_bytes {
                        let marker = format!(
                            "\n[… obcięto: limit {} KB na repozytorium]\n",
                            self.limits.repo_bytes / 1024
                        );
                        s = format!(
                            "{}{marker}",
                            cut(&s, self.limits.repo_bytes.saturating_sub(marker.len()))
                        );
                    }
                    s
                }
                Err(why) => {
                    snap.notes.push(format!("{}: {why}", r.label));
                    format!("{head}(pominięto: {why})\n")
                }
            };
            let sep = if text.is_empty() { "" } else { "\n" };
            if text.len() + sep.len() + section.len() > self.limits.total_bytes {
                let marker = format!(
                    "\n[… obcięto: limit {} KB na cały stan repozytoriów; pominięto {}]\n",
                    self.limits.total_bytes / 1024,
                    repos.len() - i
                );
                let room = self
                    .limits
                    .total_bytes
                    .saturating_sub(text.len() + sep.len() + marker.len());
                text.push_str(sep);
                text.push_str(cut(&section, room));
                text.push_str(&marker);
                snap.notes.push(format!(
                    "Stan repozytoriów obcięto do {} KB.",
                    self.limits.total_bytes / 1024
                ));
                break;
            }
            text.push_str(sep);
            text.push_str(&section);
        }
        snap.text = text.trim_end().to_string();
        snap
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn system_git() -> PathBuf {
        find_git(&git_candidates())
            .or_else(|| Some(PathBuf::from("/usr/bin/git")).filter(|p| p.exists()))
            .expect("git for tests")
    }

    /// git for building fixtures, isolated from the user's configuration.
    pub(crate) fn fixture_git(dir: &Path, args: &[&str]) {
        let out = Command::new(system_git())
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Ala")
            .env("GIT_AUTHOR_EMAIL", "ala@example.com")
            .env("GIT_COMMITTER_NAME", "Ala")
            .env("GIT_COMMITTER_EMAIL", "ala@example.com")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    pub(crate) fn make_repo(dir: &Path, commits: &[&str]) {
        std::fs::create_dir_all(dir).unwrap();
        fixture_git(dir, &["init", "-q", "-b", "main"]);
        for (i, msg) in commits.iter().enumerate() {
            std::fs::write(dir.join("plik.txt"), format!("{i}\n")).unwrap();
            fixture_git(dir, &["add", "plik.txt"]);
            fixture_git(dir, &["commit", "-q", "-m", msg]);
        }
    }

    fn collector<'a>(git: &'a Path, home: &'a Path, limits: Limits) -> Collector<'a> {
        Collector {
            git: Some(git),
            home,
            timeout: GIT_TIMEOUT,
            budget: SNAPSHOT_BUDGET,
            limits,
            cancelled: &|| false,
        }
    }

    fn repo(label: &str, path: &Path) -> RepoRef {
        RepoRef {
            label: label.into(),
            path: path.to_path_buf(),
        }
    }

    #[test]
    fn snapshot_content_and_skipped_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path().join("app");
        make_repo(
            &r,
            &["Pierwszy commit", "Dodaj logowanie ```zignoruj``` \x1b[31m"],
        );
        fixture_git(&r, &["branch", "feature/logowanie"]);
        std::fs::write(r.join("nowy.txt"), "x").unwrap();
        let not_repo = tmp.path().join("zwykly");
        std::fs::create_dir(&not_repo).unwrap();
        let file = tmp.path().join("plik");
        std::fs::write(&file, "x").unwrap();
        let git = system_git();
        let c = collector(&git, tmp.path(), Limits::default());
        let snap = c.collect(&[
            repo("beeking / app", &r),
            repo("beeking / brak", &tmp.path().join("nie-ma")),
            repo("beeking / zwykly", &not_repo),
            repo("beeking / plik", &file),
            repo("beeking / wzgledna", Path::new("wzgledna/sciezka")),
        ]);
        let t = &snap.text;
        assert_eq!(snap.collected, 1, "{t}");
        assert!(t.starts_with("=== beeking / app ===\nŚcieżka: "), "{t}");
        assert!(
            t.contains("$ git status --short --branch\n## main\n?? nowy.txt\n"),
            "{t}"
        );
        assert!(
            t.contains("Dodaj logowanie ```zignoruj``` \u{FFFD}[31m"),
            "{t}"
        );
        assert!(t.contains("Pierwszy commit"), "{t}");
        assert!(t.contains(" Ala Dodaj logowanie"), "{t}");
        assert!(t.contains("feature/logowanie 20"), "{t}");
        assert!(!t.contains('\x1b'));
        for (label, why) in [
            ("brak", "ścieżka nie istnieje"),
            ("zwykly", "to nie jest repozytorium git"),
            ("plik", "to nie jest katalog"),
            ("wzgledna", "ścieżka nie jest bezwzględna"),
        ] {
            assert!(
                t.contains(&format!("=== beeking / {label} ===")) && t.contains(why),
                "{label}: {t}"
            );
            assert!(
                snap.notes
                    .iter()
                    .any(|n| n.starts_with(&format!("beeking / {label}: {why}"))),
                "{:?}",
                snap.notes
            );
        }
        // Nothing was written into the repository by the snapshot.
        assert!(!r.join(".git/index.lock").exists());

        let empty = c.collect(&[]);
        assert!(empty.text.contains("nie mają"));
        let no_git = Collector {
            git: None,
            ..collector(&git, tmp.path(), Limits::default())
        };
        let s = no_git.collect(&[repo("a", &r)]);
        assert!(s.text.contains("Nie znaleziono programu git") && s.notes.len() == 1);
    }

    /// Linked worktrees show up as data from the main repository; the
    /// worktree directory itself is never a repository root for the snapshot.
    #[test]
    fn worktrees_are_listed_from_the_main_repository() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path().join("app");
        make_repo(&r, &["Pierwszy commit"]);
        let wt = tmp.path().join("app-feature");
        fixture_git(
            &r,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature/platnosci",
                wt.to_str().unwrap(),
            ],
        );
        std::fs::write(wt.join("niewidoczne.txt"), "x").unwrap();
        let git = system_git();
        let c = collector(&git, tmp.path(), Limits::default());
        let snap = c.collect(&[repo("p / app", &r), repo("p / worktree", &wt)]);
        let t = &snap.text;
        assert!(t.contains("$ git worktree list --porcelain"), "{t}");
        let wt_real = wt.canonicalize().unwrap();
        assert!(
            t.contains(&format!("worktree {}", wt_real.display()))
                || t.contains(&format!("worktree {}", wt.display())),
            "{t}"
        );
        assert!(t.contains("branch refs/heads/feature/platnosci"), "{t}");
        // The worktree paths are read back from the text for the history source.
        let wts: Vec<PathBuf> = worktrees(t)
            .into_iter()
            .map(|(r, p)| {
                assert_eq!(r, "app");
                p
            })
            .collect();
        assert!(wts.iter().any(|p| p.ends_with("app-feature")), "{wts:?}");
        assert!(wts.iter().any(|p| p.ends_with("app")), "{wts:?}");
        assert!(worktrees("=== p / a ===\n$ git log\nworktree /nie/z/tej/sekcji\n").is_empty());
        // The worktree's own uncommitted file is not visible, and the
        // worktree path itself is refused as a repository root.
        assert!(!t.contains("niewidoczne.txt"), "{t}");
        assert!(
            t.contains("=== p / worktree ===") && t.contains("`.git` nie jest tu katalogiem"),
            "{t}"
        );
        assert_eq!(snap.collected, 1);
    }

    #[test]
    fn caps() {
        let tmp = tempfile::tempdir().unwrap();
        let long = "x".repeat(300);
        let msgs: Vec<String> = (0..30).map(|i| format!("commit {i} {long}")).collect();
        let msgs: Vec<&str> = msgs.iter().map(String::as_str).collect();
        let a = tmp.path().join("a");
        make_repo(&a, &msgs);
        for i in 0..60 {
            std::fs::write(a.join(format!("u{i:02}.txt")), "x").unwrap();
        }
        for i in 0..30 {
            fixture_git(&a, &["branch", &format!("b{i:02}")]);
        }
        let git = system_git();

        // Line caps of status and branches.
        let big = Limits {
            repo_bytes: 1 << 20,
            total_bytes: 1 << 20,
            ..Limits::default()
        };
        let t = collector(&git, tmp.path(), big)
            .collect(&[repo("p / a", &a)])
            .text;
        let status = t
            .split("$ git status")
            .nth(1)
            .unwrap()
            .split("\n$ ")
            .next()
            .unwrap();
        // 40 lines: the branch header and 39 untracked files.
        assert!(status.contains("\n## main\n"), "{status}");
        assert_eq!(
            status.lines().filter(|l| l.starts_with("?? ")).count(),
            39,
            "{status}"
        );
        assert!(status.contains("… (+21 linii pominięto)"), "{status}");
        let branches = t
            .split("$ git branch")
            .nth(1)
            .unwrap()
            .split("\n$ ")
            .next()
            .unwrap();
        assert_eq!(branches.lines().filter(|l| l.contains(" 20")).count(), 20);
        assert!(branches.contains("… (+11 linii pominięto)"), "{branches}");
        let log15 = t
            .split("$ git log --oneline -15\n")
            .nth(1)
            .unwrap()
            .split("\n$ ")
            .next()
            .unwrap();
        assert_eq!(log15.lines().count(), 15);

        // Per-repo and total byte caps.
        let limits = Limits {
            repo_bytes: 6 * 1024,
            total_bytes: 15 * 1024,
            max_repos: 3,
            ..Limits::default()
        };
        let c = collector(&git, tmp.path(), limits);
        let s = c.collect(&[repo("p / a", &a), repo("p / b", &a)]);
        let first = s.text.split("=== p / b").next().unwrap();
        assert!(first.len() <= 6 * 1024 + 1, "{}", first.len());
        assert!(first.contains("[… obcięto: limit 6 KB na repozytorium]"));
        let many: Vec<RepoRef> = (0..5).map(|i| repo(&format!("p / r{i}"), &a)).collect();
        let s = c.collect(&many);
        assert!(s.text.len() <= 15 * 1024, "{}", s.text.len());
        assert!(s.text.contains("limit 15 KB na cały stan"), "{}", s.text);
        assert_eq!(s.collected, 3);

        // At most `max_repos` repositories get git.
        let limits = Limits {
            max_repos: 2,
            ..big
        };
        let s = collector(&git, tmp.path(), limits).collect(&many);
        assert_eq!(s.collected, 2);
        assert_eq!(
            s.text.matches("limit 2 repozytoriów").count(),
            3,
            "{}",
            s.text
        );
    }

    /// The repository's own configuration must not make the snapshot run
    /// anything: an fsmonitor hook and a clean filter both touch a marker.
    #[test]
    fn repository_config_cannot_run_commands() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path().join("zla");
        make_repo(&r, &["init"]);
        let marker = tmp.path().join("MARKER");
        let hook = tmp.path().join("hook.sh");
        std::fs::write(
            &hook,
            format!("#!/bin/sh\ntouch '{}'\ncat\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let hook_s = hook.to_str().unwrap();
        fixture_git(&r, &["config", "core.fsmonitor", hook_s]);
        fixture_git(&r, &["config", "filter.zly.clean", hook_s]);
        fixture_git(&r, &["config", "filter.zly.process", hook_s]);
        fixture_git(&r, &["config", "filter.zly.required", "true"]);
        fixture_git(&r, &["config", "core.pager", hook_s]);
        fixture_git(&r, &["config", "log.showSignature", "true"]);
        fixture_git(&r, &["config", "gpg.program", hook_s]);
        std::fs::write(r.join(".git/info/attributes"), "* filter=zly\n").unwrap();
        for name in [
            "post-index-change",
            "post-checkout",
            "reference-transaction",
        ] {
            let p = r.join(".git/hooks").join(name);
            std::fs::write(&p, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let git = system_git();
        let dirty = |n: i64| {
            // A new mtime makes git re-read (and filter) the file.
            let f = std::fs::File::options()
                .write(true)
                .open(r.join("plik.txt"))
                .unwrap();
            f.set_modified(
                std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000 + n as u64),
            )
            .unwrap();
        };

        // The fixture is armed: plain `git status` runs the repository's
        // commands (and then fails on the bogus filter protocol).
        dirty(1);
        Command::new(&git)
            .args(["status", "--short"])
            .current_dir(&r)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(marker.exists(), "fixture did not trigger");
        std::fs::remove_file(&marker).unwrap();
        // Only fsmonitor blocked (what a naive hardening does): the filter still runs.
        dirty(2);
        Command::new(&git)
            .args(["-c", "core.fsmonitor=false", "status", "--short"])
            .current_dir(&r)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(marker.exists(), "the filter fixture did not trigger");
        std::fs::remove_file(&marker).unwrap();

        dirty(3);
        let s = collector(&git, tmp.path(), Limits::default()).collect(&[repo("p / zla", &r)]);
        assert_eq!(s.collected, 1, "{}", s.text);
        assert!(s.text.contains("## main"), "{}", s.text);
        assert!(s.text.contains("init"), "{}", s.text);
        assert!(
            !marker.exists(),
            "the snapshot ran a repository-configured command"
        );
        assert!(!r.join(".git/index.lock").exists());
    }

    /// A filter whose name is not valid UTF-8 cannot be blanked on the command
    /// line, so `status` (the only call that runs filters) is skipped.
    #[test]
    fn non_utf8_filter_name_skips_status() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path().join("zla");
        make_repo(&r, &["init"]);
        let marker = tmp.path().join("MARKER");
        let hook = tmp.path().join("hook.sh");
        std::fs::write(
            &hook,
            format!("#!/bin/sh\ntouch '{}'\ncat\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut cfg = std::fs::File::options()
            .append(true)
            .open(r.join(".git/config"))
            .unwrap();
        cfg.write_all(b"[filter \"evil\xff\"]\n\tclean = ").unwrap();
        cfg.write_all(hook.to_str().unwrap().as_bytes()).unwrap();
        cfg.write_all(b"\n").unwrap();
        drop(cfg);
        std::fs::write(r.join(".git/info/attributes"), b"* filter=evil\xff\n").unwrap();
        let git = system_git();
        let dirty = |n: u64| {
            let f = std::fs::File::options()
                .write(true)
                .open(r.join("plik.txt"))
                .unwrap();
            f.set_modified(
                std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000 + n),
            )
            .unwrap();
        };

        // The fixture is armed: plain `git status` runs the filter.
        dirty(1);
        Command::new(&git)
            .args(["status", "--short"])
            .current_dir(&r)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(marker.exists(), "fixture did not trigger");
        std::fs::remove_file(&marker).unwrap();

        dirty(2);
        let s = collector(&git, tmp.path(), Limits::default()).collect(&[repo("p / zla", &r)]);
        assert!(!marker.exists(), "the snapshot ran the repository's filter");
        assert!(s.text.contains("(pominięto: nazwa filtra"), "{}", s.text);
        // The rest of the snapshot is still there.
        assert!(s.text.contains("init"), "{}", s.text);
    }

    /// A promisor remote plus a missing object makes plain git fetch lazily and
    /// run the repository's `core.sshCommand`; the snapshot must not.
    #[test]
    fn missing_promisor_objects_do_not_start_a_transport() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path().join("repo");
        make_repo(&r, &["init"]);
        let marker = tmp.path().join("MARKER");
        let ssh = tmp.path().join("ssh.sh");
        std::fs::write(
            &ssh,
            format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
        fixture_git(
            &r,
            &["config", "remote.origin.url", "ssh://example.invalid/repo"],
        );
        fixture_git(&r, &["config", "remote.origin.promisor", "true"]);
        fixture_git(
            &r,
            &["config", "remote.origin.partialclonefilter", "blob:none"],
        );
        fixture_git(&r, &["config", "core.sshCommand", ssh.to_str().unwrap()]);
        std::fs::write(
            r.join(".git/refs/heads/main"),
            "1234567890123456789012345678901234567890\n",
        )
        .unwrap();
        let git = system_git();

        // The fixture is armed: plain `git log` runs the ssh command.
        Command::new(&git)
            .args(["log", "--oneline", "-15"])
            .current_dir(&r)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .unwrap();
        assert!(marker.exists(), "fixture did not trigger");
        std::fs::remove_file(&marker).unwrap();

        let s = collector(&git, tmp.path(), Limits::default()).collect(&[repo("p / repo", &r)]);
        assert!(
            !marker.exists(),
            "the snapshot started a transport: {}",
            s.text
        );
    }

    /// A `.git` file pointing at another repository must not make the snapshot
    /// read that repository's history.
    #[test]
    fn gitdir_file_is_not_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let b = tmp.path().join("b");
        make_repo(&b, &["PRIVATE_SUBJECT"]);
        let a = tmp.path().join("a");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(
            a.join(".git"),
            format!("gitdir: {}\n", b.join(".git").display()),
        )
        .unwrap();
        let git = system_git();

        // The fixture is armed: plain `git log` in `a` shows `b`'s history.
        let plain = Command::new(&git)
            .args(["log", "--oneline", "-15"])
            .current_dir(&a)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&plain.stdout).contains("PRIVATE_SUBJECT"),
            "fixture did not trigger: {plain:?}"
        );

        let s = collector(&git, tmp.path(), Limits::default()).collect(&[repo("p / a", &a)]);
        assert!(!s.text.contains("PRIVATE_SUBJECT"), "{}", s.text);
        assert!(s.text.contains("nie jest tu katalogiem"), "{}", s.text);
    }

    /// `core.worktree` pointing outside the selected directory must not make
    /// `status` list files from there.
    #[test]
    fn redirected_worktree_does_not_leak_other_files() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path().join("repo");
        make_repo(&r, &["init"]);
        std::fs::write(tmp.path().join("PRIVATE_FILENAME"), "x").unwrap();
        fixture_git(
            &r,
            &["config", "core.worktree", tmp.path().to_str().unwrap()],
        );
        let git = system_git();

        // The fixture is armed: plain `git status` names the outside file.
        let plain = Command::new(&git)
            .args(["status", "--short", "--untracked-files=normal"])
            .current_dir(&r)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&plain.stdout).contains("PRIVATE_FILENAME"),
            "fixture did not trigger: {plain:?}"
        );

        let s = collector(&git, tmp.path(), Limits::default()).collect(&[repo("p / repo", &r)]);
        assert!(!s.text.contains("PRIVATE_FILENAME"), "{}", s.text);
        assert!(s.text.contains("poza podaną ścieżkę"), "{}", s.text);
        assert!(s.text.contains("init"), "{}", s.text);
    }

    /// A git that never answers: each call is killed at the deadline, the
    /// repository is skipped with a note, the rest of the snapshot goes on.
    #[test]
    fn timeouts_and_cancel() {
        let tmp = tempfile::tempdir().unwrap();
        let pids = tmp.path().join("pids");
        let fake = tmp.path().join("git");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\necho $$ >> '{}'\nexec sleep 30\n",
                pids.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        // The collector only asks git about directories that have a `.git` directory.
        std::fs::create_dir_all(a.join(".git")).unwrap();
        std::fs::create_dir_all(b.join(".git")).unwrap();
        let c = Collector {
            timeout: Duration::from_millis(1500),
            ..collector(&fake, tmp.path(), Limits::default())
        };
        let t = Instant::now();
        let s = c.collect(&[repo("p / a", &a), repo("p / b", &b)]);
        assert!(t.elapsed() < Duration::from_secs(8), "{:?}", t.elapsed());
        assert_eq!(s.collected, 0);
        assert_eq!(s.notes.len(), 2, "{:?}", s.notes);
        assert!(
            s.notes[0].contains("git nie odpowiedział w 1 s"),
            "{:?}",
            s.notes
        );
        for pid in std::fs::read_to_string(&pids).unwrap().lines() {
            let pid = rustix::process::Pid::from_raw(pid.parse().unwrap()).unwrap();
            assert!(
                rustix::process::test_kill_process(pid).is_err(),
                "{pid:?} alive"
            );
        }

        // The budget for the whole snapshot.
        let c = Collector {
            timeout: Duration::from_millis(300),
            budget: Duration::from_millis(100),
            ..collector(&fake, tmp.path(), Limits::default())
        };
        let s = c.collect(&[repo("p / a", &a), repo("p / b", &b)]);
        assert!(
            s.text.contains("Przerwano zbieranie") && s.text.contains("pominięto 1"),
            "{}",
            s.text
        );

        // Cancel stops at once.
        let flag = std::sync::atomic::AtomicBool::new(false);
        let cancel = || flag.load(std::sync::atomic::Ordering::SeqCst);
        let c = Collector {
            cancelled: &cancel,
            ..collector(&fake, tmp.path(), Limits::default())
        };
        let t = Instant::now();
        std::thread::scope(|sc| {
            sc.spawn(|| {
                std::thread::sleep(Duration::from_millis(200));
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            let s = c.collect(&[repo("p / a", &a), repo("p / b", &b)]);
            assert!(s.cancelled);
        });
        assert!(t.elapsed() < Duration::from_secs(3));
    }
}
