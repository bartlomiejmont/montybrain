# `morwisko-core` API guide

This guide is for the agents building the Tauri app (`src-tauri`) and the MCP server
(`crates/mcp`). The file format is described in [`format.md`](format.md). Everything
below is re-exported from the crate root: `use morwisko_core::*;`.

```toml
# Cargo.toml of a consumer inside the workspace
morwisko-core = { workspace = true }                       # MCP server
morwisko-core = { workspace = true, features = ["watch"] } # desktop app (file watcher)
```

The crate re-exports `chrono` as `morwisko_core::chrono`, so you get the same
`NaiveDate` and `NaiveDateTime` types. The API is synchronous. Each call does a little
file I/O, so in async code (Tauri commands, an MCP runtime) run it on a blocking
thread (`spawn_blocking`).

## Opening a vault

```rust
Vault::default_root() -> Result<PathBuf>   // $MORWISKO_DIR (if non-empty) else ~/Morwisko
Vault::open(root: impl Into<PathBuf>) -> Result<Vault>   // must exist → else Error::VaultNotFound
Vault::open_default() -> Result<Vault>
Vault::init(root: impl Into<PathBuf>) -> Result<Vault>   // mkdir -p root + .morwisko/, create empty skrzynka.md if missing; never overwrites
Vault::init_default() -> Result<Vault>
vault.root() -> &Path
vault.with_clock(impl Fn() -> NaiveDateTime + Send + Sync + 'static) -> Vault  // tests: fixed "now"
vault.now() -> NaiveDateTime                                                    // local time, minute precision
```

`Vault` is `Clone + Send + Sync` and holds no open files. Keep one per process. You
can also create one per thread; locking is between file handles, not per `Vault`.

**On startup, call `init` and then `normalize`**, so that hand-written tasks get ids:

```rust
let vault = Vault::init_default()?;
let report = vault.normalize()?;            // NormalizeReport { ids_assigned, files_changed }
let snap = vault.load()?;
```

## Loading and the snapshot

`vault.load() -> Result<Snapshot>` reads the whole vault without taking the lock. It
fails only if the root is missing or unreadable. Any problem inside a file, and any
discovery problem (an unreadable project directory or `tematy/`, a `tematy` that is a
file, a broken symlink, a non-UTF-8 name), becomes a
`LoadWarning { path, line: Option<usize /*1-based*/>, message }` in
`snapshot.warnings`; the rest of the vault still loads. Warnings whose `path` is a
directory (`"locked"`, `"p/tematy"`) have no line.

```rust
pub struct Snapshot {
    pub root: PathBuf,
    pub projects: Vec<Project>,      // sorted by slug
    pub topics: Vec<Topic>,          // by project slug, then topic slug
    pub tasks: Vec<Task>,            // file order: inbox, then per project: projekt.md, topics
    pub journal: Vec<JournalEntry>,  // file order
    pub warnings: Vec<LoadWarning>,
}
```

Model types. All of them are `Serialize + Deserialize + Clone + Debug + PartialEq`.

| type | fields |
|------|--------|
| `Project` | `slug, name, color: Option<String>, state: ProjectState, repos: Vec<Repo>, description, path` |
| `Repo` | `name, path` (`nazwa`, `sciezka`) |
| `Topic` | `project, slug, title, status: TopicStatus, repo: Option<String>, summary, created: Option<NaiveDate>, updated: Option<NaiveDateTime>, path` |
| `Task` | `id: Option<String>, title, status: TaskStatus, priority: Priority, due: Option<Due>, done: Option<NaiveDate>, links: Vec<String>, notes: String, location: TaskLocation, line` |
| `JournalEntry` | `project, topic, at: NaiveDateTime, author, text, line` |
| `TaskLocation` | `Inbox` \| `Project { project }` \| `Topic { project, topic }`; `.project()`, `.topic()`, `.rel_path()` |
| `Due` | `{ date: NaiveDate, time: Option<NaiveTime> }`, implements `FromStr` and `Display` (`"2026-11-20"`, `"2026-11-20T14:00"`) |

Enums:

| enum | variants | file words |
|------|----------|------------|
| `TaskStatus` | `Todo, Doing, Done, Blocked` | `[ ] [/] [x] [!]` |
| `Priority` | `High, Normal, Low` | `!wysoki`, none, `!niski` |
| `ProjectState` | `Active, Paused, Archived` | `aktywny wstrzymany archiwum` |
| `TopicStatus` | `Planned, InProgress, Blocked, Waiting, Done` | `planowany w-toku zablokowany czeka zrobiony` |

**JSON shape**

* Enums serialize as snake_case English (`"in_progress"`, `"high"`). They also
  deserialize from the Polish file words (`"w-toku"`, `"wysoki"`, `"archiwum"`), so
  MCP tools can accept either.
* `TaskLocation` → `{"kind":"inbox"}`, `{"kind":"project","project":"x"}`,
  `{"kind":"topic","project":"x","topic":"y"}`.
* Dates → `"2026-11-13"`. Timestamps (`Topic.updated`, `JournalEntry.at`) →
  `"2026-11-13T14:02"`. `Due` → `"2026-11-20"` or `"2026-11-20T14:00"`.
* Paths are relative to the vault root, with `/`.
* `Task.id` is `null` only for tasks that have not been normalized yet. They cannot
  be changed by id.
* `Task.line` and `JournalEntry.line` are 1-based line numbers, useful for "open in
  editor" (`vault.root().join(task.location.rel_path())`).

### Queries (on `Snapshot`)

```rust
snap.project(slug: &str) -> Option<&Project>
snap.topic(project: &str, slug: &str) -> Option<&Topic>
snap.topics_of(project: &str) -> impl Iterator<Item = &Topic>
snap.task(id: &str) -> Option<&Task>                          // "^" prefix optional
snap.tasks(filter: &TaskFilter) -> Vec<Task>                  // file order
snap.journal_of(project: &str, topic: &str) -> Vec<JournalEntry>   // newest first
snap.recent_journal(limit: usize) -> Vec<JournalEntry>        // across the vault, newest first
snap.today(today: NaiveDate) -> Vec<TodayItem>                // use morwisko_core::local_today()
snap.search(query: &str) -> Vec<SearchHit>
```

```rust
#[serde(default)]
pub struct TaskFilter {
    pub project: Option<String>,   // projekt.md + all its topics
    pub topic: Option<String>,     // with `project`: that topic; alone: that slug in any project
    pub inbox_only: bool,
    pub statuses: Vec<TaskStatus>, // empty = any
    pub due_from: Option<NaiveDate>, // inclusive; when either bound is set, tasks without due are excluded
    pub due_to: Option<NaiveDate>,   // inclusive
    pub include_done: bool,        // done tasks also included if `statuses` contains Done
}
```

* `today` returns `TodayItem { reason: TodayReason, task }`. `TodayReason` is
  `Overdue | DueToday | Doing | Blocked`. Each open task appears once, under the first
  reason that applies in that order. Done tasks and tasks of archived projects are
  left out. The result is sorted by reason, then due date and time, then priority.
* `search` ignores case and Polish diacritics, and every whitespace-separated term
  must match. It searches task title + notes, topic title + summary, and journal
  text + author. Results come as `SearchHit::Task { task }`, then
  `SearchHit::Topic { topic }`, then `SearchHit::Journal { entry }` (newest first).
  JSON: `{"kind":"task","task":{…}}`.

## Mutations (on `Vault`)

Every mutation takes the vault lock, re-reads the affected files, applies the change,
writes atomically and releases the lock. It returns the entity **as now written to
disk**. Only the lines involved change, plus `zaktualizowano` in topic files. A
mutation that changes nothing writes nothing. Your snapshot is not updated: call
`load()` again, or rely on the watcher.

```rust
// projects
fn create_project(&self, new: NewProject) -> Result<Project>
fn update_project(&self, slug: &str, patch: ProjectPatch) -> Result<Project>
fn archive_project(&self, slug: &str) -> Result<Project>            // stan: archiwum

// topics
fn create_topic(&self, project: &str, new: NewTopic) -> Result<Topic>
fn update_topic(&self, project: &str, topic: &str, patch: TopicPatch) -> Result<Topic>
fn delete_topic(&self, project: &str, topic: &str) -> Result<Topic>  // returns the deleted topic
fn create_topic_with(&self, project: &str, new: NewTopic, entry: Option<NewJournalEntry>, tasks: Vec<NewTask>)
    -> Result<TopicCreated>        // topic + first journal entry + first tasks as ONE new file, written once;
                                   // TopicCreated { topic, tasks, entry, contents /* the file as written */ }
fn delete_topic_if(&self, project: &str, topic: &str, guard: impl Fn(&str) -> bool) -> Result<Option<Topic>>
                                   // removes the file only if `guard(current contents)` holds inside the lock
                                   // (undo: "still exactly what was written"); Ok(None) = left alone
fn add_journal_entry(&self, project: &str, topic: &str, entry: NewJournalEntry) -> Result<JournalAdded>

// tasks
fn create_task(&self, location: &TaskLocation, new: NewTask) -> Result<Task>
fn update_task(&self, id: &str, patch: TaskPatch) -> Result<Task>
fn move_task(&self, id: &str, to: &TaskLocation) -> Result<Task>     // keeps id, links, notes
fn update_and_move_task(&self, id: &str, patch: TaskPatch, to: &TaskLocation) -> Result<Task>
                                                                     // edit + move, all or nothing
fn delete_task(&self, id: &str) -> Result<Task>                      // returns the deleted task

// guarded (for undo): the guard runs on the task as read inside the lock;
// Ok(None) = guard did not hold, nothing written
fn update_task_if(&self, id: &str, guard: impl Fn(&Task) -> bool, patch: TaskPatch) -> Result<Option<Task>>
fn delete_task_if(&self, id: &str, guard: impl Fn(&Task) -> bool) -> Result<Option<Task>>
fn revert_journal_entry(&self, project: &str, topic: &str, revert: JournalRevert) -> Result<JournalReverted>
   // removes the newest entry equal to revert.entry (at, author, text) and restores
   // status / summary (`(written, before)`) only where they are still `written`;
   // one write, verified like every mutation (exactly one journal entry fewer)

// `zaktualizowano` for undo: record it around a write, then undo through
// `restoring(Some(Stamp { written, before }))`, whose writes put `before` back
// instead of bumping, but only while the file still has `written`
fn stamp_of(&self, location: &TaskLocation) -> Result<Option<String>>
fn stamp_of_task(&self, id: &str) -> Result<Option<String>>
fn restoring(&self, stamp: Option<Stamp>) -> Vault

// maintenance
fn normalize(&self) -> Result<NormalizeReport>
```

Inputs. All of them derive `Default` and use `#[serde(default)]`, so JSON callers can
leave out any field.

```rust
pub struct NewProject  { name: String, color: Option<String>, state: ProjectState, repos: Vec<Repo>, description: String }
pub struct ProjectPatch { name: Option<String>, color: Option<Option<String>>, state: Option<ProjectState>,
                          repos: Option<Vec<Repo>> /* replaces the list */, description: Option<String> }
pub struct NewTopic   { title: String, status: TopicStatus, repo: Option<String>, summary: String }
pub struct TopicPatch { title: Option<String>, status: Option<TopicStatus>, repo: Option<Option<String>>, summary: Option<String> }
pub struct NewJournalEntry { author: String /* "" → "ja" */, text: String,
                             status: Option<TopicStatus>, summary: Option<String> }  // optional same-write topic changes
pub struct JournalAdded { topic: Topic, entry: JournalEntry }
pub struct NewTask   { title: String, status: TaskStatus, priority: Priority, due: Option<Due>, links: Vec<String>, notes: String }
pub struct TaskPatch { title: Option<String>, status: Option<TaskStatus>, priority: Option<Priority>,
                       due: Option<Option<Due>>, add_links: Vec<String>, remove_links: Vec<String>,
                       notes: Option<String> /* replaces all notes; "" clears */ }
pub struct NormalizeReport { ids_assigned: usize, files_changed: Vec<PathBuf> }
```

`Option<Option<T>>` fields work like this: `None` (JSON key absent) means keep,
`Some(None)` (JSON `null`) means clear, and `Some(Some(v))` means set.

Behaviour worth knowing:

* **Validation and verification.** Every free-text input is validated before the lock
  is taken, with the rules in [`format.md` §2.1](format.md): no value can create or
  change structure outside its own field. A violation is `InvalidInput` whose message
  starts with the field name (`title: …`, `links[0]: …`, `add_links[1]: …`,
  `notes: …`, `text: …`, `summary: …`, `description: …`, `color: …`,
  `repos[0].name: …`). In particular:
  * links (`NewTask.links`, `TaskPatch.add_links`) must be bare URLs (`scheme://…`, no
    whitespace); empty strings are skipped;
  * `summary` / `description` must not contain `#`/`##` heading lines (outside code
    fences), must not start with `---` and must close every code fence they open;
  * multi-line text accepts `\n` and `\r\n`, but no lone `\r` or other control
    characters (tab is fine);
  * colour and repository name/path must be single lines without control characters;
    a repository name must not be empty (also in `create_project` now).
  Then, inside the lock, the would-be file is parsed again and its structure must
  differ from the file on disk exactly as the operation intends (one task more, one
  journal entry more, …). If not, the mutation fails with
  `InvalidInput("refusing to write <path>: …; nothing was changed")`. All files of a
  move are verified before the first write.
* `update_and_move_task(id, patch, to)` validates the patch and reads (validates) the
  destination before anything is written, then writes the destination and the
  source. Any error except an `Io` failure during the writes means nothing changed.
  If `to` is the task's current location it is `update_task`. `update_task` and
  `move_task` are the special cases with no move / an empty patch.
* `update_project` with `repos` on a file whose `repozytoria` is not a block list
  (for example a flow list `[{…}]`) fails with `Unparseable` instead of overwriting it.
  List comments and unsupported list lines are kept. The new list is matched to the
  existing items by name, then by place (a repository that took the place of an
  unmatched item is that item renamed), and a kept item changes only its
  `nazwa`/`sciezka` values; its other lines stay as they are
  ([`format.md` §4](format.md)).
* Setting a frontmatter value rewrites only that key's line; continuation lines under
  it are kept ([`format.md` §3](format.md)).
* A moved task block takes the destination file's line ending.

* Slugs come from the name or title (`slugify`), with `-2`, `-3`, … added on
  collision. They never change afterwards, even if you rename. Use `Project.slug`
  and `Topic.slug` as ids.
* Titles and names are collapsed to one line (whitespace normalised). An empty
  title or name is rejected with `InvalidInput`.
* A task title that would read back differently (for example ending in `@2026-11-20`
  with no due date) is rejected with `InvalidInput`.
* `status: Done` stamps `✓<today>` when a task becomes done. Any other status
  removes it. A task created as done is stamped too.
* `create_topic` and `update_topic` check that `repo` is one of the project's repos
  (`InvalidInput` if not).
* Slugs passed in are checked: they cannot be empty, start with `.` or contain `/`
  or `\` (`InvalidInput`). This protects against path traversal from MCP input.
* Task ids may be given with or without `^`.
* `create_task` in the inbox creates `skrzynka.md` if it is missing. A project
  directory without `projekt.md` gets one when it is first written to.

## Errors

`morwisko_core::Error` (`thiserror`). It serializes as its display string, so it can
be returned from a Tauri command as is.

| variant | when |
|---------|------|
| `Io { path, source }` | filesystem failure |
| `VaultNotFound(PathBuf)` | `open`/`load`/mutation on a root that does not exist |
| `NoVaultDir` | neither `MORWISKO_DIR` nor `HOME` is available |
| `ProjectNotFound(String)` | unknown project slug |
| `TopicNotFound { project, topic }` | unknown topic |
| `TaskNotFound(String)` | no task with that id |
| `InvalidInput(String)` | empty title, bad slug, unknown repo, ambiguous title, empty journal text, a value that would create structure (`format.md` §2.1), a write whose structure check failed |
| `Unparseable { path, message }` | the file to edit is not valid UTF-8, or `repozytoria` is not a block list |

`Result<T> = std::result::Result<T, Error>`.

## File watcher (feature `watch`)

```rust
fn watch(&self, debounce: Duration, on_change: impl FnMut(Vec<PathBuf>) + Send + 'static)
    -> Result<VaultWatcher>
// DEFAULT_DEBOUNCE = 300 ms; drop the VaultWatcher to stop.
// is_ignored(rel: &Path) -> bool   — the filter that is applied
```

The watcher uses `notify` through `notify-debouncer-mini` and watches recursively.
`on_change` runs on a background thread with the deduplicated paths that changed,
relative to the vault root. It ignores `.morwisko/**`, `*.morwisko-tmp` (Morwisko's
own temp files), `*~`, `*.swp`, `*.swx`, `*.tmp`, `.#*`, `.DS_Store` and vim's
`4913`. Morwisko's own writes **are** reported, because the final rename is a real
change. A typical app handler: reload with `load()` (and `normalize()` if you want
hand-added tasks to get ids right away), then emit an event to the UI. Normalize
writes only when something changed, so it cannot cause a loop.

## Example

```rust
use morwisko_core::*;

fn demo() -> Result<()> {
    let vault = Vault::init_default()?;
    vault.normalize()?;

    let p = vault.create_project(NewProject {
        name: "WhenUBuy".into(),
        color: Some("#a3e635".into()),
        repos: vec![Repo { name: "whenubuy-backend".into(), path: "/Users/x/whenubuy-backend".into() }],
        ..Default::default()
    })?;
    let t = vault.create_topic(&p.slug, NewTopic {
        title: "A/B testy cen".into(),
        status: TopicStatus::InProgress,
        repo: Some("whenubuy-backend".into()),
        ..Default::default()
    })?;
    let loc = TaskLocation::Topic { project: p.slug.clone(), topic: t.slug.clone() };
    let task = vault.create_task(&loc, NewTask {
        title: "Open MR".into(),
        priority: Priority::High,
        due: Some("2026-11-20T14:00".parse().unwrap()),
        ..Default::default()
    })?;
    let id = task.id.clone().unwrap();
    vault.update_task(&id, TaskPatch {
        status: Some(TaskStatus::Done),
        add_links: vec!["https://gitlab.com/g/r/-/merge_requests/42".into()],
        ..Default::default()
    })?;
    vault.add_journal_entry(&p.slug, &t.slug, NewJournalEntry {
        author: "agent".into(),
        text: "Tests green, MR opened.".into(),
        status: Some(TopicStatus::Waiting),
        ..Default::default()
    })?;

    let snap = vault.load()?;
    for item in snap.today(local_today()) {
        println!("{:?}: {}", item.reason, item.task.title);
    }
    for w in &snap.warnings {
        eprintln!("{}:{:?}: {}", w.path.display(), w.line, w.message);
    }
    Ok(())
}
```

## Other public helpers

* `slugify(&str) -> String`, `unique_slug(base, taken: impl FnMut(&str) -> bool) -> String`
* `fold(&str) -> String`: lowercases and transliterates diacritics (the search uses it)
* `is_valid_id(&str) -> bool`, `ID_LEN`
* `check::{task_title, topic_title, journal_text, summary}`: the free-text validation a mutation
  runs before it takes the lock, without writing (returns the stored form or the same
  `InvalidInput`). For rejecting untrusted text early, e.g. an agent's proposed
  changes shown for review; the mutation still validates and verifies again.
* `local_today() -> NaiveDate`
* Constants: `ENV_VAR`, `INBOX_FILE`, `PROJECT_FILE`, `TOPICS_DIR`, `META_DIR`,
  `LOCK_FILE`, `TMP_SUFFIX`
* `minute::{format, parse}`: minute-precision timestamp helpers (also used as serde
  `with` modules)
