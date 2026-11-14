//! The [`Vault`] handle: path resolution, loading, locking and all mutations.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{NaiveDateTime, Timelike};

use crate::doc::{Doc, Eol};
use crate::error::{Error, Result};
use crate::files::{
    FileKind, PROJECT_KEYS, TOPIC_KEYS, date_str, insert_journal_lines, insert_task_lines,
    journal_lines, keys_before, parse_inbox, parse_project, parse_topic, set_h1, set_text_region,
    set_updated, task_blocks, touch_updated, updated_value,
};
use crate::frontmatter::{encode_scalar, set_repos, set_scalar};
use crate::id::generate_id;
use crate::journal::{clean_author, render_entry};
use crate::model::*;
use crate::query::Snapshot;
use crate::slug::{is_safe_component, slugify, unique_slug};
use crate::task::{TaskBlock, TaskLine, parse_task_line, rewrite_continuation};
use crate::validate::{Multi, link, multiline, one_line, strict_line};
use crate::verify::{Expect, check};

/// Environment variable overriding the vault location.
pub const ENV_VAR: &str = "MONTYBRAIN_DIR";
/// The pre-rename name of [`ENV_VAR`], still honoured when [`ENV_VAR`] is unset.
pub const LEGACY_ENV_VAR: &str = "MORWISKO_DIR";
/// Inbox file name.
pub const INBOX_FILE: &str = "skrzynka.md";
/// Project metadata file name inside a project directory.
pub const PROJECT_FILE: &str = "projekt.md";
/// Directory holding topic files inside a project directory.
pub const TOPICS_DIR: &str = "tematy";
/// Hidden directory for MontyBrain's own bookkeeping (lock file, links).
/// See [`meta_dir`] for which one a given vault uses.
pub const META_DIR: &str = ".montybrain";
/// The pre-rename bookkeeping directory. A vault that already has it keeps
/// using it (and never gets a [`META_DIR`] created next to it), so the app,
/// the MCP server and an older binary still running all lock the same file.
pub const LEGACY_META_DIR: &str = ".morwisko";
/// Advisory lock file inside the vault's meta directory.
pub const LOCK_FILE: &str = "lock";
/// Suffix of temporary files written during atomic saves.
pub const TMP_SUFFIX: &str = ".montybrain-tmp";
/// [`TMP_SUFFIX`] of older binaries (ignored by the watcher like our own).
pub const LEGACY_TMP_SUFFIX: &str = ".morwisko-tmp";

/// The bookkeeping directory of the vault at `root`: [`META_DIR`] when it
/// exists, else an existing [`LEGACY_META_DIR`], else [`META_DIR`] (created
/// on first use). Nothing is ever moved or renamed.
pub fn meta_dir(root: &Path) -> PathBuf {
    let new = root.join(META_DIR);
    if new.is_dir() {
        return new;
    }
    let legacy = root.join(LEGACY_META_DIR);
    if legacy.is_dir() { legacy } else { new }
}

type Clock = Arc<dyn Fn() -> NaiveDateTime + Send + Sync>;

/// Handle to a vault directory. Cheap to clone; holds no open files.
///
/// Reads ([`Vault::load`]) are lock-free. Every mutation takes the vault-wide
/// advisory lock, re-reads the affected files from disk, applies the change,
/// writes atomically (temp file + rename) and releases the lock.
#[derive(Clone)]
pub struct Vault {
    root: PathBuf,
    clock: Clock,
    /// See [`Vault::restoring`].
    restore: Option<Stamp>,
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vault")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

fn local_now() -> NaiveDateTime {
    chrono::Local::now().naive_local()
}

fn truncate_minute(dt: NaiveDateTime) -> NaiveDateTime {
    dt.with_second(0)
        .and_then(|d| d.with_nanosecond(0))
        .unwrap_or(dt)
}

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

/// Read a file as a [`Doc`]. `Ok(None)` if it does not exist.
fn read_doc(path: &Path) -> Result<Option<(Doc, String)>> {
    match fs::read(path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(s) => Ok(Some((Doc::parse(&s), s))),
            Err(_) => Err(Error::Unparseable {
                path: path.to_path_buf(),
                message: "file is not valid UTF-8".into(),
            }),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(path, e)),
    }
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `contents` to `path` atomically: temp file in the same directory, fsync,
/// rename over the target.
pub(crate) fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{name}.{}.{n}{TMP_SUFFIX}", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut f = File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
        if let Ok(meta) = fs::metadata(path) {
            let _ = fs::set_permissions(&tmp, meta.permissions());
        }
        fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(Error::io(path, e));
    }
    Ok(())
}

/// The result of a mutation without a guard.
fn guarded<T>(v: Option<T>) -> T {
    v.expect("a mutation without a guard always returns its entity")
}

fn rel_str(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn location_path(loc: &TaskLocation) -> PathBuf {
    loc.rel_path()
}

fn location_kind(loc: &TaskLocation) -> FileKind {
    match loc {
        TaskLocation::Inbox => FileKind::Inbox,
        TaskLocation::Project { .. } => FileKind::Project,
        TaskLocation::Topic { .. } => FileKind::Topic,
    }
}

fn check_component(what: &str, s: &str) -> Result<()> {
    if is_safe_component(s) {
        Ok(())
    } else {
        Err(Error::invalid(format!("invalid {what} slug: {s:?}")))
    }
}

fn check_location(loc: &TaskLocation) -> Result<()> {
    if let Some(p) = loc.project() {
        check_component("project", p)?;
    }
    if let Some(t) = loc.topic() {
        check_component("topic", t)?;
    }
    Ok(())
}

/// Validated links (empty strings are skipped).
fn links(field: &str, links: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for (i, l) in links.iter().enumerate() {
        if let Some(l) = link(&format!("{field}[{i}]"), l)? {
            out.push(l);
        }
    }
    Ok(out)
}

/// Validated repository list.
fn repos(field: &str, repos: &[Repo]) -> Result<Vec<Repo>> {
    repos
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let name = strict_line(&format!("{field}[{i}].name"), &r.name)?;
            if name.is_empty() {
                return Err(Error::invalid(format!(
                    "{field}[{i}].name: must not be empty"
                )));
            }
            strict_line(&format!("{field}[{i}].path"), &r.path)?;
            // Values are stored as given (quoted when needed); only checked here.
            Ok(r.clone())
        })
        .collect()
}

/// Validated colour; empty means none.
fn color(c: &str) -> Result<Option<String>> {
    let c = strict_line("color", c)?;
    Ok((!c.is_empty()).then_some(c))
}

/// Directory layout discovered on disk.
struct Layout {
    projects: Vec<ProjectDir>,
    /// Discovery problems (unreadable directories, broken links, …).
    warnings: Vec<LoadWarning>,
}

struct ProjectDir {
    slug: String,
    has_project_file: bool,
    topics: Vec<String>,
}

fn warn(warnings: &mut Vec<LoadWarning>, rel: impl Into<PathBuf>, message: String) {
    warnings.push(LoadWarning {
        path: rel.into(),
        line: None,
        message,
    });
}

/// Directory entries sorted by name. Entries that cannot be read become warnings.
fn sorted_entries(
    dir: &Path,
    rel: &Path,
    warnings: &mut Vec<LoadWarning>,
) -> std::io::Result<Vec<fs::DirEntry>> {
    let mut v = Vec::new();
    for e in fs::read_dir(dir)? {
        match e {
            Ok(e) => v.push(e),
            Err(err) => warn(
                warnings,
                rel,
                format!("cannot read a directory entry: {err}"),
            ),
        }
    }
    v.sort_by_key(|e| e.file_name());
    Ok(v)
}

/// Metadata following symlinks; a broken symlink or an unreadable path becomes a
/// warning and `None`.
fn stat(path: &Path, rel: &Path, warnings: &mut Vec<LoadWarning>) -> Option<fs::Metadata> {
    match fs::metadata(path) {
        Ok(m) => Some(m),
        Err(e) if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) => {
            warn(
                warnings,
                rel,
                format!("broken symbolic link ({e}); skipped"),
            );
            None
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            warn(warnings, rel, format!("cannot access: {e}; skipped"));
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Lock + transaction
// ---------------------------------------------------------------------------

struct LockGuard {
    /// Released in reverse order of acquisition.
    files: Vec<File>,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        for file in self.files.iter().rev() {
            let _ = file.unlock();
        }
    }
}

fn lock_file_in(dir: &Path) -> Result<File> {
    fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    let path = dir.join(LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| Error::io(&path, e))?;
    file.lock().map_err(|e| Error::io(&path, e))?;
    Ok(file)
}

/// State of one locked mutation.
struct Txn<'a> {
    vault: &'a Vault,
    _guard: LockGuard,
    now: NaiveDateTime,
}

impl Txn<'_> {
    /// Bump `zaktualizowano`, or put back the value of [`Vault::restoring`]
    /// while the file still has the value that write set.
    fn stamp(&self, doc: &mut Doc, kind: FileKind) {
        match &self.vault.restore {
            Some(r) if updated_value(doc).as_deref() == Some(r.written.as_str()) => {
                set_updated(doc, &r.before)
            }
            _ => touch_updated(doc, kind, &self.now),
        }
    }
}

impl Txn<'_> {
    fn abs(&self, rel: &Path) -> PathBuf {
        self.vault.root.join(rel)
    }

    fn read(&self, rel: &Path) -> Result<Option<(Doc, String)>> {
        read_doc(&self.abs(rel))
    }

    /// Prepare writing `doc` if it differs from `original`: bump `zaktualizowano`
    /// when `touch` is set, then re-parse the would-be file and check that its
    /// structure changed exactly as `expect` says. Nothing is written yet. `doc` is
    /// replaced by the re-parsed contents, so read-backs see what will be on disk.
    fn prepare(
        &self,
        rel: &Path,
        kind: FileKind,
        original: &str,
        doc: &mut Doc,
        touch: bool,
        expect: &Expect<'_>,
    ) -> Result<Option<Pending>> {
        if doc.render() == original {
            return Ok(None);
        }
        if doc.has_embedded_newline() {
            return Err(Error::invalid(format!(
                "refusing to write {}: a value contains a line break; nothing was changed",
                rel_str(rel)
            )));
        }
        if touch {
            self.stamp(doc, kind);
        }
        let contents = doc.render();
        check(original, &contents, kind, expect).map_err(|why| {
            Error::invalid(format!(
                "refusing to write {}: {why}; nothing was changed",
                rel_str(rel)
            ))
        })?;
        *doc = Doc::parse(&contents);
        Ok(Some(Pending {
            path: self.abs(rel),
            contents,
        }))
    }

    /// Write a prepared file. Returns whether anything was written.
    fn write(&self, pending: Option<Pending>) -> Result<bool> {
        match pending {
            Some(p) => write_atomic(&p.path, &p.contents).map(|()| true),
            None => Ok(false),
        }
    }

    /// [`Txn::prepare`] + [`Txn::write`] for a single-file mutation.
    fn commit(
        &self,
        rel: &Path,
        kind: FileKind,
        original: &str,
        doc: &mut Doc,
        touch: bool,
        expect: &Expect<'_>,
    ) -> Result<bool> {
        let p = self.prepare(rel, kind, original, doc, touch, expect)?;
        self.write(p)
    }

    /// All files that can contain tasks, in deterministic order.
    fn task_files(&self) -> Result<Vec<(PathBuf, FileKind, TaskLocation)>> {
        let mut out = vec![(
            PathBuf::from(INBOX_FILE),
            FileKind::Inbox,
            TaskLocation::Inbox,
        )];
        for p in self.vault.layout()?.projects {
            if p.has_project_file {
                let loc = TaskLocation::Project {
                    project: p.slug.clone(),
                };
                out.push((location_path(&loc), FileKind::Project, loc));
            }
            for t in p.topics {
                let loc = TaskLocation::Topic {
                    project: p.slug.clone(),
                    topic: t,
                };
                out.push((location_path(&loc), FileKind::Topic, loc));
            }
        }
        Ok(out)
    }

    /// Every task id currently used in the vault.
    fn all_ids(&self) -> Result<HashSet<String>> {
        let mut ids = HashSet::new();
        for (rel, kind, _) in self.task_files()? {
            if let Ok(Some((doc, _))) = self.read(&rel) {
                let (blocks, _) = task_blocks(&doc, kind);
                ids.extend(blocks.into_iter().filter_map(|b| b.parsed.id));
            }
        }
        Ok(ids)
    }

    fn find_task(&self, id: &str) -> Result<Found> {
        let id = id.trim().trim_start_matches('^');
        for (rel, kind, loc) in self.task_files()? {
            let Ok(Some((doc, original))) = self.read(&rel) else {
                continue;
            };
            let (blocks, _) = task_blocks(&doc, kind);
            if let Some(block) = blocks
                .into_iter()
                .find(|b| b.parsed.id.as_deref() == Some(id))
            {
                return Ok(Found {
                    rel,
                    kind,
                    loc,
                    doc,
                    original,
                    block,
                });
            }
        }
        Err(Error::TaskNotFound(id.to_string()))
    }

    /// Read the file for `loc`, which must exist (the inbox is created on demand).
    fn read_location(&self, loc: &TaskLocation) -> Result<(Doc, String)> {
        check_location(loc)?;
        let rel = location_path(loc);
        match self.read(&rel)? {
            Some(x) => Ok(x),
            None => match loc {
                TaskLocation::Inbox => Ok((Doc::default(), String::new())),
                TaskLocation::Project { project } => {
                    if self.abs(Path::new(project)).is_dir() {
                        Ok((Doc::default(), String::new()))
                    } else {
                        Err(Error::ProjectNotFound(project.clone()))
                    }
                }
                TaskLocation::Topic { project, topic } => Err(Error::TopicNotFound {
                    project: project.clone(),
                    topic: topic.clone(),
                }),
            },
        }
    }
}

/// A verified file write that has not happened yet.
#[derive(Debug)]
struct Pending {
    path: PathBuf,
    contents: String,
}

struct Found {
    rel: PathBuf,
    kind: FileKind,
    loc: TaskLocation,
    doc: Doc,
    original: String,
    block: TaskBlock,
}

fn task_by_id(doc: &Doc, kind: FileKind, loc: &TaskLocation, id: &str) -> Option<Task> {
    let (blocks, _) = task_blocks(doc, kind);
    blocks
        .iter()
        .find(|b| b.parsed.id.as_deref() == Some(id))
        .map(|b| b.to_task(loc.clone()))
}

/// Verify that `text` parses back to exactly the fields of `line`. A title that
/// ends with something token-like (e.g. `Meet @2026-11-20` on a task without a due
/// date) would otherwise silently turn into a token on the next read.
fn verify_line(text: &str, line: &TaskLine) -> Result<()> {
    match parse_task_line(text) {
        Some(Ok(back))
            if back.title == line.title
                && back.status == line.status
                && back.priority == line.priority
                && back.due == line.due
                && back.done == line.done
                && back.id == line.id =>
        {
            Ok(())
        }
        _ => Err(Error::invalid(format!(
            "task title {:?} cannot be stored unambiguously (it ends with something that looks like an inline token)",
            line.title
        ))),
    }
}

/// A written field must read back as what was asked for.
fn read_back(field: &str, got: &str, want: &str) -> Result<()> {
    if got == want {
        Ok(())
    } else {
        Err(Error::invalid(format!(
            "{field}: cannot be stored unambiguously (it would read back as {got:?})"
        )))
    }
}

/// Every added link must read back as a link of the task.
fn check_links(task: &Task, added: &[String]) -> Result<()> {
    match added.iter().find(|l| !task.links.contains(l)) {
        Some(l) => Err(Error::invalid(format!(
            "links: {l:?} cannot be stored as a link"
        ))),
        None => Ok(()),
    }
}

pub(crate) fn render_checked(line: &TaskLine) -> Result<String> {
    let text = line.render();
    verify_line(&text, line)?;
    Ok(text)
}

/// Continuation lines of a new task. `links` and `notes` are already validated.
fn continuation_lines(links: &[String], notes: &str) -> Vec<String> {
    let mut out: Vec<String> = links.iter().map(|l| format!("  - {l}")).collect();
    let notes_lines: Vec<&str> = notes.trim_matches('\n').lines().collect();
    let start = notes_lines.iter().position(|l| !l.trim().is_empty());
    let end = notes_lines.iter().rposition(|l| !l.trim().is_empty());
    if let (Some(s), Some(e)) = (start, end) {
        for l in &notes_lines[s..=e] {
            out.push(if l.trim().is_empty() {
                String::new()
            } else {
                format!("  {}", l.trim_end())
            });
        }
    }
    out
}

/// Default vault folder name in the home directory.
pub const DEFAULT_DIR_NAME: &str = "MontyBrain";
/// The pre-rename default folder, used when it exists and [`DEFAULT_DIR_NAME`] does not.
pub const LEGACY_DIR_NAME: &str = "Morwisko";

/// `env` (new variable), else `legacy_env`, else `~/MontyBrain` if it exists,
/// else `~/Morwisko` if it exists, else `~/MontyBrain` (to be created).
fn resolve_root(
    env: Option<std::ffi::OsString>,
    legacy_env: Option<std::ffi::OsString>,
    home: Option<PathBuf>,
    exists: impl Fn(&Path) -> bool,
) -> Result<PathBuf> {
    if let Some(dir) = env
        .filter(|v| !v.is_empty())
        .or_else(|| legacy_env.filter(|v| !v.is_empty()))
    {
        return Ok(PathBuf::from(dir));
    }
    let home = home.ok_or(Error::NoVaultDir)?;
    let new = home.join(DEFAULT_DIR_NAME);
    if exists(&new) {
        return Ok(new);
    }
    let legacy = home.join(LEGACY_DIR_NAME);
    Ok(if exists(&legacy) { legacy } else { new })
}

// ---------------------------------------------------------------------------
// Vault
// ---------------------------------------------------------------------------

impl Vault {
    /// `$MONTYBRAIN_DIR` if set (and non-empty), else `$MORWISKO_DIR`, else
    /// `~/MontyBrain` if it exists, else `~/Morwisko` if it exists, else
    /// `~/MontyBrain`. Nothing is moved or copied.
    pub fn default_root() -> Result<PathBuf> {
        resolve_root(
            std::env::var_os(ENV_VAR),
            std::env::var_os(LEGACY_ENV_VAR),
            std::env::home_dir(),
            |p| p.is_dir(),
        )
    }

    /// Open an existing vault directory (nothing is created).
    pub fn open(root: impl Into<PathBuf>) -> Result<Vault> {
        let root = root.into();
        if !root.is_dir() {
            return Err(Error::VaultNotFound(root));
        }
        Ok(Vault {
            root,
            clock: Arc::new(local_now),
            restore: None,
        })
    }

    /// Open the vault at [`Vault::default_root`].
    pub fn open_default() -> Result<Vault> {
        Self::open(Self::default_root()?)
    }

    /// Create the vault skeleton if missing (`<root>/`, `skrzynka.md` and the
    /// [`meta_dir`], `.montybrain/` for a new vault) and open it. Never
    /// touches existing files.
    pub fn init(root: impl Into<PathBuf>) -> Result<Vault> {
        let root = root.into();
        fs::create_dir_all(meta_dir(&root)).map_err(|e| Error::io(&root, e))?;
        let inbox = root.join(INBOX_FILE);
        match OpenOptions::new().write(true).create_new(true).open(&inbox) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(Error::io(&inbox, e)),
        }
        Self::open(root)
    }

    /// [`Vault::init`] at [`Vault::default_root`].
    pub fn init_default() -> Result<Vault> {
        Self::init(Self::default_root()?)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Replace the clock used for `zaktualizowano`, journal timestamps and `✓`
    /// dates (local time by default). Mostly for tests.
    pub fn with_clock(
        mut self,
        clock: impl Fn() -> NaiveDateTime + Send + Sync + 'static,
    ) -> Vault {
        self.clock = Arc::new(clock);
        self
    }

    /// This vault for an undo: a write that would bump `zaktualizowano` puts
    /// back `stamp.before` instead, but only while the file's value is still
    /// `stamp.written` (nothing else wrote the file since); otherwise it bumps
    /// as usual. `None` = the normal behaviour.
    pub fn restoring(&self, stamp: Option<Stamp>) -> Vault {
        Vault {
            restore: stamp,
            ..self.clone()
        }
    }

    /// The raw `zaktualizowano` of the file holding `location` (`None` when
    /// the file or the value is missing). With [`Vault::stamp_of_task`], what
    /// an apply records for [`Vault::restoring`].
    pub fn stamp_of(&self, location: &TaskLocation) -> Result<Option<String>> {
        check_location(location)?;
        Ok(
            read_doc(&self.root.join(location_path(location)))?
                .and_then(|(d, _)| updated_value(&d)),
        )
    }

    /// [`Vault::stamp_of`] the file holding task `id`.
    pub fn stamp_of_task(&self, id: &str) -> Result<Option<String>> {
        let txn = self.txn()?;
        let found = txn.find_task(id)?;
        Ok(updated_value(&found.doc))
    }

    /// Current time according to the vault clock, truncated to the minute.
    pub fn now(&self) -> NaiveDateTime {
        truncate_minute((self.clock)())
    }

    /// Lock the vault for one mutation. The lock file lives in
    /// [`meta_dir`]; when a vault has both directories (only possible by
    /// hand), the legacy lock is taken first as well, so an older binary
    /// that only knows `.morwisko/lock` is still excluded. Every process
    /// takes the locks in the same order, so this cannot deadlock.
    fn lock(&self) -> Result<LockGuard> {
        let primary = meta_dir(&self.root);
        let legacy = self.root.join(LEGACY_META_DIR);
        let mut files = Vec::with_capacity(2);
        if primary != legacy && legacy.is_dir() {
            files.push(lock_file_in(&legacy)?);
        }
        files.push(lock_file_in(&primary)?);
        Ok(LockGuard { files })
    }

    fn txn(&self) -> Result<Txn<'_>> {
        if !self.root.is_dir() {
            return Err(Error::VaultNotFound(self.root.clone()));
        }
        Ok(Txn {
            vault: self,
            _guard: self.lock()?,
            now: self.now(),
        })
    }

    /// Discover projects and topics. Only an unreadable vault root is an error;
    /// every other problem (a `tematy` that is not a directory, a directory that
    /// cannot be read, a broken symlink, a non-UTF-8 name) becomes a warning and the
    /// rest of the vault is still discovered.
    fn layout(&self) -> Result<Layout> {
        let mut warnings = Vec::new();
        let mut projects = Vec::new();
        let entries = sorted_entries(&self.root, Path::new("."), &mut warnings)
            .map_err(|e| Error::io(&self.root, e))?;
        for e in entries {
            let Some(name) = e.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let dir = e.path();
            let rel = PathBuf::from(&name);
            if !stat(&dir, &rel, &mut warnings).is_some_and(|m| m.is_dir()) {
                continue;
            }
            // `projekt.md`: a permission problem here means the whole directory
            // cannot be read; report it instead of silently skipping the project.
            let has_project_file = match fs::metadata(dir.join(PROJECT_FILE)) {
                Ok(m) => m.is_file(),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
                Err(err) => {
                    if fs::symlink_metadata(dir.join(PROJECT_FILE)).is_ok() {
                        warn(
                            &mut warnings,
                            rel.join(PROJECT_FILE),
                            format!("cannot access: {err}; skipped"),
                        );
                        false
                    } else {
                        warn(
                            &mut warnings,
                            &rel,
                            format!("cannot access directory: {err}; skipped"),
                        );
                        continue;
                    }
                }
            };
            let topics_rel = rel.join(TOPICS_DIR);
            let topics_dir = dir.join(TOPICS_DIR);
            let topics_meta = fs::metadata(&topics_dir);
            let has_topics_dir = topics_meta.as_ref().is_ok_and(|m| m.is_dir());
            if !has_project_file && !has_topics_dir {
                continue;
            }
            let mut topics = Vec::new();
            match topics_meta {
                Ok(m) if m.is_dir() => {
                    match sorted_entries(&topics_dir, &topics_rel, &mut warnings) {
                        Ok(entries) => {
                            for t in entries {
                                let Some(n) = t.file_name().to_str().map(str::to_string) else {
                                    let p = t.path();
                                    warn(
                                        &mut warnings,
                                        p.strip_prefix(&self.root).unwrap_or(&p),
                                        "file name is not valid UTF-8; skipped".into(),
                                    );
                                    continue;
                                };
                                if n.starts_with('.') {
                                    continue;
                                }
                                let Some(stem) = n.strip_suffix(".md").filter(|s| !s.is_empty())
                                else {
                                    continue;
                                };
                                let trel = topics_rel.join(&n);
                                if stat(&t.path(), &trel, &mut warnings)
                                    .is_some_and(|m| m.is_file())
                                {
                                    topics.push(stem.to_string());
                                }
                            }
                        }
                        Err(err) => warn(
                            &mut warnings,
                            &topics_rel,
                            format!("cannot read directory: {err}; topics not loaded"),
                        ),
                    }
                }
                Ok(_) => warn(
                    &mut warnings,
                    &topics_rel,
                    format!("`{TOPICS_DIR}` is not a directory; topics not loaded"),
                ),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => warn(
                    &mut warnings,
                    &topics_rel,
                    format!("cannot access: {err}; topics not loaded"),
                ),
            }
            projects.push(ProjectDir {
                slug: name,
                has_project_file,
                topics,
            });
        }
        Ok(Layout { projects, warnings })
    }

    // -----------------------------------------------------------------------
    // Load
    // -----------------------------------------------------------------------

    /// Load the whole vault into a [`Snapshot`]. Per-file problems become
    /// warnings; only a missing/unreadable vault root is an error.
    pub fn load(&self) -> Result<Snapshot> {
        if !self.root.is_dir() {
            return Err(Error::VaultNotFound(self.root.clone()));
        }
        let mut snap = Snapshot {
            root: self.root.clone(),
            ..Snapshot::default()
        };
        let mut warnings: Vec<LoadWarning> = Vec::new();
        let push_problems =
            |rel: &Path, problems: Vec<(Option<usize>, String)>, w: &mut Vec<LoadWarning>| {
                w.extend(problems.into_iter().map(|(line, message)| LoadWarning {
                    path: rel.to_path_buf(),
                    line: line.map(|l| l + 1),
                    message,
                }));
            };

        let read = |rel: &Path, w: &mut Vec<LoadWarning>| -> Option<Doc> {
            match read_doc(&self.root.join(rel)) {
                Ok(Some((doc, _))) => Some(doc),
                Ok(None) => None,
                Err(e) => {
                    w.push(LoadWarning {
                        path: rel.to_path_buf(),
                        line: None,
                        message: format!("file skipped: {e}"),
                    });
                    None
                }
            }
        };

        let inbox_rel = PathBuf::from(INBOX_FILE);
        if let Some(doc) = read(&inbox_rel, &mut warnings) {
            let (tasks, problems) = parse_inbox(&doc);
            snap.tasks.extend(tasks);
            push_problems(&inbox_rel, problems, &mut warnings);
        }

        let layout = self.layout()?;
        warnings.extend(layout.warnings);
        for p in layout.projects {
            let rel = location_path(&TaskLocation::Project {
                project: p.slug.clone(),
            });
            let project = if p.has_project_file {
                read(&rel, &mut warnings).map(|doc| {
                    let parsed = parse_project(&p.slug, &rel, &doc);
                    snap.tasks.extend(parsed.tasks);
                    push_problems(&rel, parsed.problems, &mut warnings);
                    parsed.project
                })
            } else {
                None
            };
            let project = project.unwrap_or_else(|| {
                warnings.push(LoadWarning {
                    path: PathBuf::from(&p.slug),
                    line: None,
                    message: format!("project directory has no readable {PROJECT_FILE}"),
                });
                Project {
                    slug: p.slug.clone(),
                    name: p.slug.clone(),
                    color: None,
                    state: ProjectState::Active,
                    repos: Vec::new(),
                    description: String::new(),
                    path: rel.clone(),
                }
            });
            for t in &p.topics {
                let trel = location_path(&TaskLocation::Topic {
                    project: p.slug.clone(),
                    topic: t.clone(),
                });
                let Some(doc) = read(&trel, &mut warnings) else {
                    continue;
                };
                let parsed = parse_topic(&p.slug, t, &trel, &doc);
                if let Some(repo) = &parsed.topic.repo
                    && !project.repos.iter().any(|r| &r.name == repo)
                {
                    warnings.push(LoadWarning {
                        path: trel.clone(),
                        line: None,
                        message: format!(
                            "repo `{repo}` is not listed in the project's `repozytoria`"
                        ),
                    });
                }
                snap.tasks.extend(parsed.tasks);
                snap.journal.extend(parsed.journal);
                push_problems(&trel, parsed.problems, &mut warnings);
                snap.topics.push(parsed.topic);
            }
            snap.projects.push(project);
        }

        // Id checks.
        let mut seen: HashMap<String, PathBuf> = HashMap::new();
        let mut missing: HashMap<PathBuf, usize> = HashMap::new();
        for t in &snap.tasks {
            let rel = location_path(&t.location);
            match &t.id {
                None => *missing.entry(rel).or_default() += 1,
                Some(id) => {
                    if let Some(first) = seen.get(id) {
                        warnings.push(LoadWarning {
                            path: rel,
                            line: Some(t.line),
                            message: format!(
                                "duplicate task id ^{id} (also in {}); run normalize",
                                rel_str(first)
                            ),
                        });
                    } else {
                        seen.insert(id.clone(), rel);
                    }
                }
            }
        }
        let mut missing: Vec<_> = missing.into_iter().collect();
        missing.sort();
        for (rel, n) in missing {
            warnings.push(LoadWarning {
                path: rel,
                line: None,
                message: format!("{n} task(s) without ^id; run normalize"),
            });
        }
        snap.warnings = warnings;
        Ok(snap)
    }

    // -----------------------------------------------------------------------
    // Normalize
    // -----------------------------------------------------------------------

    /// Assign ids to tasks that have none (or share a duplicate id) and write the
    /// affected files back under the lock. Files that need no change are not
    /// written. `zaktualizowano` is not bumped by normalization.
    pub fn normalize(&self) -> Result<NormalizeReport> {
        let txn = self.txn()?;
        let files = txn.task_files()?;
        let mut docs = Vec::new();
        let mut taken: HashSet<String> = HashSet::new();
        for (rel, kind, _) in files {
            let Ok(Some((doc, original))) = txn.read(&rel) else {
                continue;
            };
            let (blocks, _) = task_blocks(&doc, kind);
            taken.extend(blocks.iter().filter_map(|b| b.parsed.id.clone()));
            docs.push((rel, kind, doc, original));
        }
        let mut first_seen: HashSet<String> = HashSet::new();
        let mut report = NormalizeReport::default();
        for (rel, kind, mut doc, original) in docs {
            let (blocks, _) = task_blocks(&doc, kind);
            for b in blocks {
                let needs = match &b.parsed.id {
                    None => true,
                    Some(id) => !first_seen.insert(id.clone()),
                };
                if !needs {
                    continue;
                }
                let id = generate_id(&mut taken);
                let text = doc.text(b.line).to_string();
                let new_text = if b.parsed.id.is_none() {
                    format!("{} ^{id}", text.trim_end())
                } else {
                    let mut new = b.parsed.clone();
                    new.id = Some(id);
                    TaskLine::rewrite(&text, &b.parsed, &new)
                };
                doc.set_text(b.line, new_text);
                report.ids_assigned += 1;
            }
            let renumber = Expect {
                renumber: true,
                ..Default::default()
            };
            if txn.commit(&rel, kind, &original, &mut doc, false, &renumber)? {
                report.files_changed.push(rel);
            }
        }
        Ok(report)
    }

    // -----------------------------------------------------------------------
    // Projects
    // -----------------------------------------------------------------------

    fn read_project(&self, txn: &Txn<'_>, slug: &str) -> Result<(PathBuf, Doc, String)> {
        check_component("project", slug)?;
        let rel = location_path(&TaskLocation::Project {
            project: slug.to_string(),
        });
        match txn.read(&rel)? {
            Some((doc, original)) => Ok((rel, doc, original)),
            None if self.root.join(slug).is_dir() => Ok((rel, Doc::default(), String::new())),
            None => Err(Error::ProjectNotFound(slug.to_string())),
        }
    }

    /// Create `<slug>/projekt.md` and `<slug>/tematy/`. The slug is derived from
    /// the name (`-2`, `-3`… on collision).
    pub fn create_project(&self, new: NewProject) -> Result<Project> {
        let name = one_line("name", &new.name)?;
        let color = new.color.as_deref().map(color).transpose()?.flatten();
        let new_repos = repos("repos", &new.repos)?;
        let desc = multiline("description", &new.description, Multi::Region)?;
        let txn = self.txn()?;
        let slug = unique_slug(&slugify(&name), |s| self.root.join(s).exists());
        let dir = self.root.join(&slug);
        let mut text = format!("---\nnazwa: {}\n", encode_scalar(&name));
        if let Some(c) = &color {
            text.push_str(&format!("kolor: {}\n", encode_scalar(c)));
        }
        text.push_str(&format!("stan: {}\n", new.state.file_value()));
        text.push_str("---\n");
        let desc = desc.trim();
        if !desc.is_empty() {
            text.push_str(desc);
            text.push_str("\n\n");
        }
        text.push_str("## Zadania\n");
        let mut doc = Doc::parse(&text);
        set_repos(
            &mut doc,
            &new_repos,
            keys_before(PROJECT_KEYS, "repozytoria"),
        )
        .map_err(Error::invalid)?;
        let rel = PathBuf::from(format!("{slug}/{PROJECT_FILE}"));
        let p = txn.prepare(
            &rel,
            FileKind::Project,
            "",
            &mut doc,
            false,
            &Expect::default(),
        )?;
        let project = parse_project(&slug, &rel, &doc).project;
        read_back("description", &project.description, desc)?;
        if project.repos != new_repos {
            return Err(Error::invalid("repos: could not be stored unambiguously"));
        }
        fs::create_dir_all(dir.join(TOPICS_DIR)).map_err(|e| Error::io(&dir, e))?;
        txn.write(p)?;
        Ok(project)
    }

    /// Update project metadata/description. Only the lines of changed fields are
    /// rewritten.
    pub fn update_project(&self, slug: &str, patch: ProjectPatch) -> Result<Project> {
        let name = patch
            .name
            .as_deref()
            .map(|n| one_line("name", n))
            .transpose()?;
        let new_color = match &patch.color {
            None => None,
            Some(None) => Some(None),
            Some(Some(c)) => Some(color(c)?),
        };
        let new_repos = patch
            .repos
            .as_deref()
            .map(|r| repos("repos", r))
            .transpose()?;
        let desc = patch
            .description
            .as_deref()
            .map(|d| multiline("description", d, Multi::Region))
            .transpose()?;
        let txn = self.txn()?;
        let (rel, mut doc, original) = self.read_project(&txn, slug)?;
        if let Some(name) = &name {
            set_scalar(&mut doc, "nazwa", Some(&encode_scalar(name)), &[]);
        }
        if let Some(color) = &new_color {
            let v = color.as_deref().map(encode_scalar);
            set_scalar(
                &mut doc,
                "kolor",
                v.as_deref(),
                keys_before(PROJECT_KEYS, "kolor"),
            );
        }
        if let Some(state) = patch.state {
            set_scalar(
                &mut doc,
                "stan",
                Some(state.file_value()),
                keys_before(PROJECT_KEYS, "stan"),
            );
        }
        if let Some(repos) = &new_repos {
            set_repos(&mut doc, repos, keys_before(PROJECT_KEYS, "repozytoria")).map_err(
                |message| Error::Unparseable {
                    path: txn.abs(&rel),
                    message,
                },
            )?;
        }
        if let Some(desc) = &desc {
            set_text_region(&mut doc, desc);
        }
        let p = txn.prepare(
            &rel,
            FileKind::Project,
            &original,
            &mut doc,
            true,
            &Expect::default(),
        )?;
        let project = parse_project(slug, &rel, &doc).project;
        if let Some(desc) = &desc {
            read_back("description", &project.description, desc)?;
        }
        if new_repos.as_ref().is_some_and(|r| *r != project.repos) {
            return Err(Error::invalid("repos: could not be stored unambiguously"));
        }
        if let Some(name) = &name {
            read_back("name", &project.name, name)?;
        }
        if new_color.as_ref().is_some_and(|c| *c != project.color) {
            return Err(Error::invalid("color: could not be stored unambiguously"));
        }
        if patch.state.is_some_and(|s| s != project.state) {
            return Err(Error::invalid("state: could not be stored unambiguously"));
        }
        txn.write(p)?;
        Ok(project)
    }

    /// Set `stan: archiwum`.
    pub fn archive_project(&self, slug: &str) -> Result<Project> {
        self.update_project(
            slug,
            ProjectPatch {
                state: Some(ProjectState::Archived),
                ..Default::default()
            },
        )
    }

    // -----------------------------------------------------------------------
    // Topics
    // -----------------------------------------------------------------------

    fn check_repo(&self, txn: &Txn<'_>, project: &str, repo: &str) -> Result<()> {
        let (rel, doc, _) = self.read_project(txn, project)?;
        let p = parse_project(project, &rel, &doc).project;
        if p.repos.iter().any(|r| r.name == repo) {
            Ok(())
        } else {
            Err(Error::invalid(format!(
                "project `{project}` has no repository named `{repo}`"
            )))
        }
    }

    fn read_topic(
        &self,
        txn: &Txn<'_>,
        project: &str,
        topic: &str,
    ) -> Result<(PathBuf, Doc, String)> {
        let loc = TaskLocation::Topic {
            project: project.to_string(),
            topic: topic.to_string(),
        };
        check_location(&loc)?;
        let rel = location_path(&loc);
        match txn.read(&rel)? {
            Some((doc, original)) => Ok((rel, doc, original)),
            None => Err(Error::TopicNotFound {
                project: project.to_string(),
                topic: topic.to_string(),
            }),
        }
    }

    /// Create `<project>/tematy/<slug>.md`; the slug is derived from the title.
    pub fn create_topic(&self, project: &str, new: NewTopic) -> Result<Topic> {
        self.create_topic_with(project, new, None, Vec::new())
            .map(|c| c.topic)
    }

    /// Create a topic together with its first journal entry and its first
    /// tasks, as one new file written once: either all of it is written or
    /// nothing. Everything is validated like [`Vault::create_topic`],
    /// [`Vault::add_journal_entry`] and [`Vault::create_task`] do, and the
    /// would-be file is verified (exactly these tasks, at most this entry).
    pub fn create_topic_with(
        &self,
        project: &str,
        new: NewTopic,
        entry: Option<NewJournalEntry>,
        tasks: Vec<NewTask>,
    ) -> Result<TopicCreated> {
        let title = one_line("title", &new.title)?;
        let summary = multiline("summary", &new.summary, Multi::Region)?;
        let entry_text = match &entry {
            Some(e) => {
                let t = multiline("text", &e.text, Multi::Indented)?;
                if t.trim().is_empty() {
                    return Err(Error::invalid("text: journal text must not be empty"));
                }
                Some(t)
            }
            None => None,
        };
        let mut task_lines_in = Vec::new();
        for (i, t) in tasks.iter().enumerate() {
            let field = format!("tasks[{i}].title");
            task_lines_in.push((one_line(&field, &t.title)?, t));
        }
        let txn = self.txn()?;
        self.read_project(&txn, project)?;
        let repo = new.repo.as_deref().map(str::trim).filter(|r| !r.is_empty());
        if let Some(r) = repo {
            self.check_repo(&txn, project, r)?;
        }
        let topics_dir = self.root.join(project).join(TOPICS_DIR);
        let slug = unique_slug(&slugify(&title), |s| {
            topics_dir.join(format!("{s}.md")).exists()
        });
        let mut text = format!("---\nstatus: {}\n", new.status.file_value());
        if let Some(r) = repo {
            text.push_str(&format!("repo: {}\n", encode_scalar(r)));
        }
        text.push_str(&format!(
            "utworzono: {}\nzaktualizowano: {}\n---\n# {title}\n\n",
            date_str(&txn.now.date()),
            minute::format(&txn.now)
        ));
        let summary = summary.trim();
        if !summary.is_empty() {
            text.push_str(summary);
            text.push_str("\n\n");
        }
        text.push_str("## Zadania\n\n## Dziennik\n");
        let mut doc = Doc::parse(&text);

        let mut ids = Vec::new();
        if !task_lines_in.is_empty() {
            let mut taken = txn.all_ids()?;
            let mut lines = Vec::new();
            for (t_title, t) in &task_lines_in {
                let id = generate_id(&mut taken);
                let line = TaskLine {
                    bullet: '-',
                    status: t.status,
                    title: t_title.clone(),
                    priority: t.priority,
                    due: t.due,
                    done: t.status.is_done().then(|| txn.now.date()),
                    id: Some(id.clone()),
                    problems: Vec::new(),
                };
                lines.push(render_checked(&line)?);
                ids.push(id);
            }
            insert_task_lines(&mut doc, FileKind::Topic, lines);
        }
        if let (Some(e), Some(t)) = (&entry, &entry_text) {
            insert_journal_lines(
                &mut doc,
                render_entry(&txn.now, &clean_author(&e.author), t),
            );
        }

        let rel = PathBuf::from(format!("{project}/{TOPICS_DIR}/{slug}.md"));
        let expect = Expect {
            title: true,
            add_tasks: &ids,
            add_journal: entry.is_some(),
            ..Default::default()
        };
        let p = txn.prepare(&rel, FileKind::Topic, "", &mut doc, false, &expect)?;
        let parsed = parse_topic(project, &slug, &rel, &doc);
        let topic = parsed.topic;
        read_back("title", &topic.title, &title)?;
        read_back("summary", &topic.summary, summary)?;
        let loc = TaskLocation::Topic {
            project: project.to_string(),
            topic: slug.clone(),
        };
        let mut created = Vec::new();
        for (id, (t_title, _)) in ids.iter().zip(&task_lines_in) {
            let t = task_by_id(&doc, FileKind::Topic, &loc, id)
                .ok_or_else(|| Error::invalid("task could not be read back"))?;
            read_back("title", &t.title, t_title)?;
            created.push(t);
        }
        let entry = match entry_text {
            Some(_) => {
                let e = parsed
                    .journal
                    .into_iter()
                    .next()
                    .ok_or_else(|| Error::invalid("journal entry could not be read back"))?;
                Some(e)
            }
            None => None,
        };
        let contents = doc.render();
        fs::create_dir_all(&topics_dir).map_err(|e| Error::io(&topics_dir, e))?;
        txn.write(p)?;
        Ok(TopicCreated {
            topic,
            tasks: created,
            entry,
            contents,
        })
    }

    /// Update title (`# H1`), status, repo and/or summary. The file name (slug)
    /// never changes.
    pub fn update_topic(&self, project: &str, topic: &str, patch: TopicPatch) -> Result<Topic> {
        let title = patch
            .title
            .as_deref()
            .map(|t| one_line("title", t))
            .transpose()?;
        let summary = patch
            .summary
            .as_deref()
            .map(|s| multiline("summary", s, Multi::Region))
            .transpose()?;
        let txn = self.txn()?;
        let (rel, mut doc, original) = self.read_topic(&txn, project, topic)?;
        if let Some(title) = &title {
            set_h1(&mut doc, title);
        }
        if let Some(status) = patch.status {
            set_scalar(&mut doc, "status", Some(status.file_value()), &[]);
        }
        if let Some(repo) = &patch.repo {
            let repo = repo.as_deref().map(str::trim).filter(|r| !r.is_empty());
            if let Some(r) = repo {
                self.check_repo(&txn, project, r)?;
            }
            let enc = repo.map(encode_scalar);
            set_scalar(
                &mut doc,
                "repo",
                enc.as_deref(),
                keys_before(TOPIC_KEYS, "repo"),
            );
        }
        if let Some(summary) = &summary {
            set_text_region(&mut doc, summary);
        }
        let expect = Expect {
            title: title.is_some(),
            ..Default::default()
        };
        let p = txn.prepare(&rel, FileKind::Topic, &original, &mut doc, true, &expect)?;
        let parsed = parse_topic(project, topic, &rel, &doc).topic;
        if let Some(title) = &title {
            read_back("title", &parsed.title, title)?;
        }
        if let Some(summary) = &summary {
            read_back("summary", &parsed.summary, summary)?;
        }
        if patch.status.is_some_and(|s| s != parsed.status) {
            return Err(Error::invalid("status: could not be stored unambiguously"));
        }
        if let Some(repo) = &patch.repo {
            let want = repo.as_deref().map(str::trim).filter(|r| !r.is_empty());
            if parsed.repo.as_deref() != want {
                return Err(Error::invalid("repo: could not be stored unambiguously"));
            }
        }
        txn.write(p)?;
        Ok(parsed)
    }

    /// [`Vault::delete_topic`], but only if `guard` holds for the file's
    /// current contents, read inside the lock (e.g. "still exactly what was
    /// written"); otherwise nothing is removed and `Ok(None)` is returned.
    pub fn delete_topic_if(
        &self,
        project: &str,
        topic: &str,
        guard: impl Fn(&str) -> bool,
    ) -> Result<Option<Topic>> {
        let txn = self.txn()?;
        let (rel, doc, original) = self.read_topic(&txn, project, topic)?;
        if !guard(&original) {
            return Ok(None);
        }
        let parsed = parse_topic(project, topic, &rel, &doc).topic;
        let abs = txn.abs(&rel);
        fs::remove_file(&abs).map_err(|e| Error::io(&abs, e))?;
        Ok(Some(parsed))
    }

    /// Delete a topic file (with its tasks and journal). Returns the topic as it was.
    pub fn delete_topic(&self, project: &str, topic: &str) -> Result<Topic> {
        let txn = self.txn()?;
        let (rel, doc, _) = self.read_topic(&txn, project, topic)?;
        let parsed = parse_topic(project, topic, &rel, &doc).topic;
        let abs = txn.abs(&rel);
        fs::remove_file(&abs).map_err(|e| Error::io(&abs, e))?;
        Ok(parsed)
    }

    /// Prepend a journal entry (newest first), optionally changing the topic
    /// status and/or summary in the same write.
    pub fn add_journal_entry(
        &self,
        project: &str,
        topic: &str,
        entry: NewJournalEntry,
    ) -> Result<JournalAdded> {
        let text = multiline("text", &entry.text, Multi::Indented)?;
        if text.trim().is_empty() {
            return Err(Error::invalid("text: journal text must not be empty"));
        }
        let summary = entry
            .summary
            .as_deref()
            .map(|s| multiline("summary", s, Multi::Region))
            .transpose()?;
        let txn = self.txn()?;
        let (rel, mut doc, original) = self.read_topic(&txn, project, topic)?;
        let author = clean_author(&entry.author);
        insert_journal_lines(&mut doc, render_entry(&txn.now, &author, &text));
        if let Some(status) = entry.status {
            set_scalar(&mut doc, "status", Some(status.file_value()), &[]);
        }
        if let Some(summary) = &summary {
            set_text_region(&mut doc, summary);
        }
        let expect = Expect {
            add_journal: true,
            ..Default::default()
        };
        let p = txn.prepare(&rel, FileKind::Topic, &original, &mut doc, true, &expect)?;
        let parsed = parse_topic(project, topic, &rel, &doc);
        if let Some(summary) = &summary {
            read_back("summary", &parsed.topic.summary, summary)?;
        }
        let first = journal_lines(&doc).0.into_iter().next();
        let entry = parsed
            .journal
            .into_iter()
            .find(|j| first.as_ref().is_some_and(|f| f.line + 1 == j.line))
            .ok_or_else(|| Error::invalid("journal entry could not be read back"))?;
        txn.write(p)?;
        Ok(JournalAdded {
            topic: parsed.topic,
            entry,
        })
    }

    /// Undo of [`Vault::add_journal_entry`], guarded: removes the newest
    /// entry that is still exactly `revert.entry` (same minute, author and
    /// text), and restores the topic status / summary to their previous value
    /// only where they are still what was written. Each part is checked inside
    /// the lock and done in one write; a part that no longer matches is left
    /// alone and reported as not done. Only the entry's own lines (and the
    /// changed status / summary, plus `zaktualizowano`) change.
    pub fn revert_journal_entry(
        &self,
        project: &str,
        topic: &str,
        revert: JournalRevert,
    ) -> Result<JournalReverted> {
        let restore_summary = revert
            .summary
            .as_ref()
            .map(|(_, before)| multiline("summary", before, Multi::Region))
            .transpose()?;
        let txn = self.txn()?;
        let (rel, mut doc, original) = self.read_topic(&txn, project, topic)?;
        let current = parse_topic(project, topic, &rel, &doc).topic;
        let mut out = JournalReverted::default();
        let (entries, _) = journal_lines(&doc);
        let target = entries.iter().find(|e| {
            e.at == revert.entry.at
                && e.author == revert.entry.author
                && e.text == revert.entry.text
        });
        if let Some(e) = target {
            doc.remove(e.line..e.end);
            out.entry_removed = true;
        }
        if let Some((written, before)) = revert.status
            && current.status == written
        {
            set_scalar(&mut doc, "status", Some(before.file_value()), &[]);
            out.status_restored = true;
        }
        if let (Some((written, _)), Some(before)) = (&revert.summary, &restore_summary)
            && current.summary == *written
        {
            set_text_region(&mut doc, before);
            out.summary_restored = true;
        }
        let expect = Expect {
            remove_journal: out.entry_removed,
            ..Default::default()
        };
        let p = txn.prepare(&rel, FileKind::Topic, &original, &mut doc, true, &expect)?;
        let parsed = parse_topic(project, topic, &rel, &doc).topic;
        if let Some(before) = restore_summary.as_ref().filter(|_| out.summary_restored) {
            read_back("summary", &parsed.summary, before)?;
        }
        txn.write(p)?;
        out.topic = Some(parsed);
        Ok(out)
    }

    // -----------------------------------------------------------------------
    // Tasks
    // -----------------------------------------------------------------------

    /// Append a task to the inbox, a project or a topic. A fresh vault-unique id
    /// is assigned. Creating a task as `done` stamps `✓<today>`.
    pub fn create_task(&self, location: &TaskLocation, new: NewTask) -> Result<Task> {
        let title = one_line("title", &new.title)?;
        let new_links = links("links", &new.links)?;
        let notes = multiline("notes", &new.notes, Multi::Indented)?;
        let txn = self.txn()?;
        let (mut doc, original) = txn.read_location(location)?;
        let id = generate_id(&mut txn.all_ids()?);
        let line = TaskLine {
            bullet: '-',
            status: new.status,
            title,
            priority: new.priority,
            due: new.due,
            done: new.status.is_done().then(|| txn.now.date()),
            id: Some(id.clone()),
            problems: Vec::new(),
        };
        let mut lines = vec![render_checked(&line)?];
        lines.extend(continuation_lines(&new_links, &notes));
        let kind = location_kind(location);
        insert_task_lines(&mut doc, kind, lines);
        let rel = location_path(location);
        let expect = Expect {
            add_task: Some(&id),
            ..Default::default()
        };
        let p = txn.prepare(&rel, kind, &original, &mut doc, true, &expect)?;
        let task = task_by_id(&doc, kind, location, &id)
            .ok_or_else(|| Error::invalid("task could not be read back"))?;
        check_links(&task, &new_links)?;
        txn.write(p)?;
        Ok(task)
    }

    /// Update a task by id. Only the task's own lines change (plus
    /// `zaktualizowano` in topic files).
    pub fn update_task(&self, id: &str, patch: TaskPatch) -> Result<Task> {
        self.edit_task(id, patch, None, None).map(guarded)
    }

    /// [`Vault::update_task`], but only if the task as read inside the lock
    /// satisfies `guard`; otherwise nothing is written and `Ok(None)` is
    /// returned. For undoing a change only while it is still what was written.
    pub fn update_task_if(
        &self,
        id: &str,
        guard: impl Fn(&Task) -> bool,
        patch: TaskPatch,
    ) -> Result<Option<Task>> {
        self.edit_task(id, patch, None, Some(&guard))
    }

    /// Move a task (with its links/notes) to another container, keeping its id.
    /// The destination is written before the source, so a crash in between can
    /// duplicate but never lose the task.
    pub fn move_task(&self, id: &str, to: &TaskLocation) -> Result<Task> {
        self.edit_task(id, TaskPatch::default(), Some(to), None)
            .map(guarded)
    }

    /// Apply `patch` and move the task to `to`, in one locked transaction. The patch
    /// and the destination are validated and both files are prepared and verified
    /// before anything is written, so an error means nothing changed (except for an
    /// I/O failure between the destination and the source write, which can leave
    /// the task in both files; `normalize` then reports the duplicate id). If `to` is
    /// the task's current location this is [`Vault::update_task`].
    pub fn update_and_move_task(
        &self,
        id: &str,
        patch: TaskPatch,
        to: &TaskLocation,
    ) -> Result<Task> {
        self.edit_task(id, patch, Some(to), None).map(guarded)
    }

    /// `Ok(None)` only when `guard` is given and does not hold.
    fn edit_task(
        &self,
        id: &str,
        patch: TaskPatch,
        to: Option<&TaskLocation>,
        guard: Option<&dyn Fn(&Task) -> bool>,
    ) -> Result<Option<Task>> {
        let title = patch
            .title
            .as_deref()
            .map(|t| one_line("title", t))
            .transpose()?;
        let add_links = links("add_links", &patch.add_links)?;
        let notes = patch
            .notes
            .as_deref()
            .map(|n| multiline("notes", n, Multi::Indented))
            .transpose()?;
        if let Some(to) = to {
            check_location(to)?;
        }
        let txn = self.txn()?;
        let Found {
            rel,
            kind,
            loc,
            mut doc,
            original,
            block,
        } = txn.find_task(id)?;
        if guard.is_some_and(|g| !g(&block.to_task(loc.clone()))) {
            return Ok(None);
        }
        let task_id = block.parsed.id.clone().unwrap_or_default();
        // The destination is read (and so validated) before anything is written.
        let dest = match to {
            Some(to) if *to != loc => Some((to, txn.read_location(to)?)),
            _ => None,
        };

        // Edit the task line.
        let old = &block.parsed;
        let mut new = old.clone();
        new.problems.clear();
        if let Some(title) = title {
            new.title = title;
        }
        if let Some(status) = patch.status {
            new.status = status;
            if status.is_done() && !old.status.is_done() {
                new.done = Some(old.done.unwrap_or(txn.now.date()));
            } else if !status.is_done() {
                new.done = None;
            }
        }
        if let Some(p) = patch.priority {
            new.priority = p;
        }
        if let Some(due) = patch.due {
            new.due = due;
        }
        let text = doc.text(block.line).to_string();
        let mut old_cmp = old.clone();
        old_cmp.problems.clear();
        if new != old_cmp {
            let new_text = TaskLine::rewrite(&text, &old_cmp, &new);
            verify_line(&new_text, &new)?;
            doc.set_text(block.line, new_text);
        }
        rewrite_continuation(
            &mut doc,
            &block,
            &add_links,
            &patch.remove_links,
            notes.as_deref(),
        );

        let Some((to, (mut dest_doc, dest_original))) = dest else {
            txn.commit(&rel, kind, &original, &mut doc, true, &Expect::default())?;
            let task =
                task_by_id(&doc, kind, &loc, &task_id).ok_or(Error::TaskNotFound(task_id))?;
            check_links(&task, &add_links)?;
            return Ok(Some(task));
        };

        // Move the (edited) block.
        let (blocks, _) = task_blocks(&doc, kind);
        let moved = blocks
            .into_iter()
            .find(|b| b.parsed.id.as_deref() == Some(task_id.as_str()))
            .ok_or_else(|| Error::TaskNotFound(task_id.clone()))?;
        let lines: Vec<String> = moved.range().map(|i| doc.text(i).to_string()).collect();
        if !dest_doc.has_newline() {
            // A file without line endings of its own takes the block's.
            let eol = match doc.eol(moved.line) {
                Eol::None => doc.newline(),
                e => e,
            };
            dest_doc.set_fallback_newline(eol);
        }
        let dest_kind = location_kind(to);
        insert_task_lines(&mut dest_doc, dest_kind, lines);
        doc.remove(moved.range());
        let dest_rel = location_path(to);
        let add = Expect {
            add_task: Some(&task_id),
            ..Default::default()
        };
        let remove = Expect {
            remove_task: Some(&task_id),
            ..Default::default()
        };
        let pd = txn.prepare(
            &dest_rel,
            dest_kind,
            &dest_original,
            &mut dest_doc,
            true,
            &add,
        )?;
        let ps = txn.prepare(&rel, kind, &original, &mut doc, true, &remove)?;
        let task = task_by_id(&dest_doc, dest_kind, to, &task_id)
            .ok_or_else(|| Error::TaskNotFound(task_id.clone()))?;
        check_links(&task, &add_links)?;
        txn.write(pd)?;
        txn.write(ps)?;
        Ok(Some(task))
    }

    /// Delete a task (with its links/notes). Returns the task as it was.
    pub fn delete_task(&self, id: &str) -> Result<Task> {
        self.delete_task_guarded(id, None).map(guarded)
    }

    /// [`Vault::delete_task`], but only if the task as read inside the lock
    /// satisfies `guard` (e.g. "still exactly as created"); otherwise nothing
    /// is written and `Ok(None)` is returned.
    pub fn delete_task_if(&self, id: &str, guard: impl Fn(&Task) -> bool) -> Result<Option<Task>> {
        self.delete_task_guarded(id, Some(&guard))
    }

    fn delete_task_guarded(
        &self,
        id: &str,
        guard: Option<&dyn Fn(&Task) -> bool>,
    ) -> Result<Option<Task>> {
        let txn = self.txn()?;
        let Found {
            rel,
            kind,
            loc,
            mut doc,
            original,
            block,
        } = txn.find_task(id)?;
        let task = block.to_task(loc);
        if guard.is_some_and(|g| !g(&task)) {
            return Ok(None);
        }
        let task_id = block.parsed.id.clone().unwrap_or_default();
        doc.remove(block.range());
        let expect = Expect {
            remove_task: Some(&task_id),
            ..Default::default()
        };
        txn.commit(&rel, kind, &original, &mut doc, true, &expect)?;
        Ok(Some(task))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_resolution() {
        let home = Some(PathBuf::from("/home/u"));
        let none = |_: &Path| false;
        let new = PathBuf::from("/home/u/MontyBrain");
        let old = PathBuf::from("/home/u/Morwisko");
        // The new variable wins over the legacy one; empty values are unset.
        assert_eq!(
            resolve_root(
                Some("/data/v".into()),
                Some("/old".into()),
                home.clone(),
                none
            )
            .unwrap(),
            PathBuf::from("/data/v")
        );
        assert_eq!(
            resolve_root(Some("".into()), Some("/old".into()), home.clone(), none).unwrap(),
            PathBuf::from("/old")
        );
        assert_eq!(
            resolve_root(None, Some("".into()), home.clone(), none).unwrap(),
            new
        );
        // Neither folder exists: the new one (to be created).
        assert_eq!(resolve_root(None, None, home.clone(), none).unwrap(), new);
        // Only the old folder exists: keep using it.
        let only_old = |p: &Path| p == Path::new("/home/u/Morwisko");
        assert_eq!(
            resolve_root(None, None, home.clone(), only_old).unwrap(),
            old
        );
        // Both exist: the new one wins.
        let both = |_: &Path| true;
        assert_eq!(resolve_root(None, None, home.clone(), both).unwrap(), new);
        // A variable still wins over an existing folder.
        assert_eq!(
            resolve_root(None, Some("/old".into()), home, both).unwrap(),
            PathBuf::from("/old")
        );
        assert!(matches!(
            resolve_root(None, None, None, none),
            Err(Error::NoVaultDir)
        ));
    }

    #[test]
    fn meta_dir_keeps_a_legacy_vault_on_its_old_directory() {
        // A new vault gets `.montybrain/`.
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::init(dir.path()).unwrap();
        assert!(dir.path().join(META_DIR).is_dir());
        assert!(!dir.path().join(LEGACY_META_DIR).exists());
        drop(v.lock().unwrap());
        assert!(dir.path().join(META_DIR).join(LOCK_FILE).is_file());

        // An existing vault with `.morwisko/` keeps it: init and locking
        // never create `.montybrain/` next to it.
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(LEGACY_META_DIR)).unwrap();
        assert_eq!(meta_dir(dir.path()), dir.path().join(LEGACY_META_DIR));
        let v = Vault::init(dir.path()).unwrap();
        let guard = v.lock().unwrap();
        assert_eq!(guard.files.len(), 1);
        drop(guard);
        assert!(!dir.path().join(META_DIR).exists());
        assert!(dir.path().join(LEGACY_META_DIR).join(LOCK_FILE).is_file());

        // Both present (only by hand): `.montybrain/` is the meta dir, and
        // the legacy lock is taken too.
        fs::create_dir_all(dir.path().join(META_DIR)).unwrap();
        assert_eq!(meta_dir(dir.path()), dir.path().join(META_DIR));
        let guard = v.lock().unwrap();
        assert_eq!(guard.files.len(), 2);
        drop(guard);
        assert!(dir.path().join(META_DIR).join(LOCK_FILE).is_file());
    }

    /// What an older binary does: lock `<root>/.morwisko/lock` on its own.
    fn hold_legacy_lock(root: &Path) -> File {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join(LEGACY_META_DIR).join(LOCK_FILE))
            .unwrap();
        file.lock().unwrap();
        file
    }

    fn locks_while_held(v: Vault, held: File) -> bool {
        let (tx, rx) = std::sync::mpsc::channel();
        let t = std::thread::spawn(move || {
            let _g = v.lock().unwrap();
            tx.send(()).unwrap();
        });
        let early = rx
            .recv_timeout(std::time::Duration::from_millis(300))
            .is_ok();
        held.unlock().unwrap();
        if !early {
            rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        }
        t.join().unwrap();
        early
    }

    #[test]
    fn a_legacy_lock_held_by_an_older_binary_excludes_us() {
        // Legacy vault: we lock the very same file.
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(LEGACY_META_DIR)).unwrap();
        let v = Vault::init(dir.path()).unwrap();
        let held = hold_legacy_lock(dir.path());
        assert!(!locks_while_held(v, held));

        // Both directories: the legacy lock is still honoured.
        fs::create_dir_all(dir.path().join(META_DIR)).unwrap();
        let v = Vault::open(dir.path()).unwrap();
        let held = hold_legacy_lock(dir.path());
        assert!(!locks_while_held(v, held));
    }

    #[test]
    fn prepare_refuses_embedded_newlines_and_structural_surprises() {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::init(dir.path()).unwrap();
        let txn = v.txn().unwrap();
        let original = "## Zadania\n- [ ] a ^aaaaa1\n";
        let rel = Path::new("skrzynka.md");
        // A line whose text carries a line break.
        let mut doc = Doc::parse(original);
        doc.set_text(1, "- [ ] a ^aaaaa1\n  - https://x.y".into());
        let err = txn
            .prepare(
                rel,
                FileKind::Inbox,
                original,
                &mut doc,
                false,
                &Expect::default(),
            )
            .unwrap_err();
        assert!(err.to_string().contains("line break"), "{err}");
        // A change that adds a task the operation did not ask for.
        let mut doc = Doc::parse(original);
        doc.insert(2, vec!["- [ ] forged".into()]);
        let err = txn
            .prepare(
                rel,
                FileKind::Inbox,
                original,
                &mut doc,
                false,
                &Expect::default(),
            )
            .unwrap_err();
        assert!(err.to_string().contains("task list"), "{err}");
        assert_eq!(fs::read_to_string(dir.path().join(rel)).unwrap(), "");
    }

    #[test]
    fn atomic_write_replaces_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.md");
        write_atomic(&p, "one").unwrap();
        write_atomic(&p, "two").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "two");
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
    }
}
