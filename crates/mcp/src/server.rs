//! MCP tool definitions.

use morwisko_core::{
    NewJournalEntry, NewTask, NewTopic, Priority, Snapshot, TaskFilter, TaskLocation, TaskPatch,
    Vault,
};
use rmcp::{
    ErrorData, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    schemars::{self, JsonSchema},
    tool, tool_handler, tool_router,
};
use serde::Deserialize;

use crate::render;
use crate::resolve::{
    nonempty, parse_date, parse_due, parse_priority, parse_task_status, parse_topic_status,
    resolve_project, resolve_topic, resolve_topic_anywhere,
};

const INSTRUCTIONS: &str = "\
Morwisko is the user's local command center: projects contain topics, topics and projects contain \
tasks, and every topic has a status, a short summary and a journal. Everything is stored as Markdown \
files that the user also edits by hand and sees in the Morwisko desktop app. \
Etiquette: (1) call `overview` at the start of work to see what is going on today. \
(2) After meaningful progress on a topic, call `log_update` with a short, factual entry written in \
Polish (the user's language); use its `status`/`summary` to keep the topic current. \
(3) Mark tasks `doing` when you start and `done` when finished via `update_task`. \
(4) Create tasks (`create_task`) for follow-ups you discover instead of keeping them in your head; \
tasks without a project go to the inbox. \
(5) Attach merge-request / thread / issue URLs to tasks with `add_links`. \
(6) Do not restructure the user's projects or topics unasked: no new projects, no mass moves or \
renames. Tool results are Markdown; `^id` is a task id and `project / topic` its location. \
Project and topic arguments accept the slug or the display name.";

/// Appended to [`INSTRUCTIONS`] when the server runs read-only.
const READ_ONLY_INSTRUCTIONS: &str = "\n\nREAD-ONLY RUN: this session was started with \
MORWISKO_READONLY set. Only `overview`, `get_topic`, `list_tasks` and `search` work; `create_task`, \
`update_task`, `create_topic` and `log_update` are refused and nothing in the vault can be changed. \
Do not try to write: report what you would change instead.";

/// Env var that switches the server to read-only mode (`1` or `true`).
pub const ENV_READONLY: &str = "MORWISKO_READONLY";

/// Whether an `MORWISKO_READONLY` value turns read-only mode on: `1` or `true`
/// (case-insensitive, surrounding whitespace ignored). Anything else is off.
pub fn read_only_from_env(value: Option<&str>) -> bool {
    value.is_some_and(|v| {
        let v = v.trim();
        v == "1" || v.eq_ignore_ascii_case("true")
    })
}

/// Opens the vault for a read-only session without creating anything. The
/// error is the message every tool returns when there is no vault.
pub fn open_read_only(root: morwisko_core::Result<std::path::PathBuf>) -> Result<Vault, String> {
    let root = root.map_err(|e| format!("Morwisko vault not found: {e}."))?;
    Vault::open(&root).map_err(|_| {
        format!(
            "Morwisko vault not found at {}. This read-only session does not create it: open the \
Morwisko app once (or point MORWISKO_DIR at the vault) and try again. Nothing was read or changed.",
            root.display()
        )
    })
}

#[derive(Clone)]
pub struct Morwisko {
    /// `Err` (read-only sessions only): there is no vault; every tool returns this message.
    vault: Result<Vault, String>,
    /// Write tools refuse and read tools skip `normalize()`: nothing is written.
    read_only: bool,
}

type ToolResult = Result<CallToolResult, ErrorData>;

fn ce(e: morwisko_core::Error) -> String {
    match e {
        morwisko_core::Error::TaskNotFound(id) => format!(
            "No task with id '{id}'. Find ids with list_tasks or search (ids look like ^k3f9a2)."
        ),
        other => other.to_string(),
    }
}

/// Error of a mutation. Core validates everything and verifies the result before
/// writing, so any error other than an I/O failure means nothing was written.
fn mutation_err(e: morwisko_core::Error) -> String {
    match e {
        morwisko_core::Error::Io { .. } => format!(
            "{} The write failed part-way, so the change may be partially applied: check with \
list_tasks, search or get_topic before retrying.",
            ce(e)
        ),
        other => format!("Nothing was changed: {}", ce(other)),
    }
}

/// Default and maximum page sizes.
const LIST_LIMIT: usize = 50;
const SEARCH_LIMIT: usize = 20;
const TOPIC_TASKS_LIMIT: usize = 50;
const TOPIC_JOURNAL_LIMIT: usize = 30;
const MAX_LIMIT: usize = 500;

fn limit(v: Option<usize>, default: usize) -> usize {
    v.unwrap_or(default).clamp(1, MAX_LIMIT)
}

/// Fresh view of the vault: give hand-written tasks ids (unless read-only, which
/// must not write anything), then reload.
fn fresh(v: &Vault, read_only: bool) -> Result<Snapshot, String> {
    if !read_only {
        v.normalize().map_err(ce)?;
    }
    v.load().map_err(ce)
}

fn text(s: String, is_error: bool) -> CallToolResult {
    if is_error {
        CallToolResult::error(vec![ContentBlock::text(s)])
    } else {
        CallToolResult::success(vec![ContentBlock::text(s)])
    }
}

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OverviewArgs {
    /// Limit the briefing to this project (slug or display name) and show more detail
    /// (all open tasks, longer summaries, project journal). Omit for the all-projects overview.
    project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetTopicArgs {
    /// Project slug or display name, e.g. "whenubuy" or "WhenUBuy".
    project: String,
    /// Topic slug or title, matched case- and diacritics-insensitively.
    topic: String,
    /// Also list done tasks (default false: open tasks only).
    include_done: Option<bool>,
    /// Maximum number of tasks shown (default 50, max 500).
    tasks_limit: Option<usize>,
    /// Number of tasks to skip, for paging (default 0).
    tasks_offset: Option<usize>,
    /// Maximum number of journal entries shown, newest first (default 30, max 500).
    journal_limit: Option<usize>,
    /// Number of journal entries to skip, for paging to older entries (default 0).
    journal_offset: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListTasksArgs {
    /// Only tasks of this project (slug or name), including its topics.
    project: Option<String>,
    /// Only tasks of this topic (slug or title). Best combined with `project`.
    topic: Option<String>,
    /// Only inbox tasks (those that belong to no project).
    inbox_only: Option<bool>,
    /// Allowed statuses, any of: todo, doing, done, blocked (Polish word w-toku also accepted). Empty = any open status.
    status: Option<Vec<String>>,
    /// Earliest due date, inclusive: YYYY-MM-DD. When set, tasks without a due date are excluded.
    due_from: Option<String>,
    /// Latest due date, inclusive: YYYY-MM-DD. When set, tasks without a due date are excluded.
    due_to: Option<String>,
    /// Include done tasks (default false).
    include_done: Option<bool>,
    /// Maximum number of tasks shown (default 50, max 500).
    limit: Option<usize>,
    /// Number of tasks to skip, for paging (default 0).
    offset: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// Search text. Every whitespace-separated word must match; case and Polish diacritics are ignored.
    query: String,
    /// Maximum number of results shown per kind (tasks, topics, journal; default 20, max 500).
    limit: Option<usize>,
    /// Number of results to skip per kind, for paging (default 0).
    offset: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateTaskArgs {
    /// Task title, one line.
    title: String,
    /// Project slug or name. Omit (with no topic) to put the task in the inbox.
    project: Option<String>,
    /// Topic slug or title inside `project`. Omit for a project-level task.
    topic: Option<String>,
    /// Initial status: todo (default), doing, done, blocked.
    status: Option<String>,
    /// Priority: high, normal (default), low (Polish: wysoki, niski).
    priority: Option<String>,
    /// Due date: YYYY-MM-DD or YYYY-MM-DDTHH:MM.
    due: Option<String>,
    /// URLs to attach (merge request, issue, thread, doc).
    links: Option<Vec<String>>,
    /// Free-form notes; may span several lines.
    notes: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveTarget {
    /// true: move to the inbox (ignores project/topic).
    inbox: Option<bool>,
    /// Destination project slug or name. Without `topic`: becomes a project-level task.
    project: Option<String>,
    /// Destination topic slug or title. Without `project`, the task's current project is used.
    topic: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateTaskArgs {
    /// Task id, e.g. "k3f9a2" (a leading ^ is fine). Get ids from overview, list_tasks or search.
    id: String,
    /// New title.
    title: Option<String>,
    /// New status: todo, doing, done, blocked. `done` stamps today's completion date.
    status: Option<String>,
    /// New priority: high, normal, low.
    priority: Option<String>,
    /// New due date: YYYY-MM-DD or YYYY-MM-DDTHH:MM. To remove the due date use `clear_due`.
    due: Option<String>,
    /// true: remove the due date.
    clear_due: Option<bool>,
    /// URLs to add to the task (duplicates are ignored).
    add_links: Option<Vec<String>>,
    /// URLs to remove from the task (exact match).
    remove_links: Option<Vec<String>>,
    /// Replaces ALL notes of the task (links are kept). Empty string clears the notes.
    notes: Option<String>,
    /// Move the task elsewhere; id, links and notes are kept.
    move_to: Option<MoveTarget>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateTopicArgs {
    /// Project slug or name.
    project: String,
    /// Topic title.
    title: String,
    /// Initial status: planned (default), in_progress, blocked, waiting, done (Polish: planowany, w-toku, zablokowany, czeka, zrobiony).
    status: Option<String>,
    /// Name of one of the project's repositories (see overview with `project`).
    repo: Option<String>,
    /// Short current-state summary.
    summary: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LogUpdateArgs {
    /// Project slug or name.
    project: String,
    /// Topic slug or title.
    topic: String,
    /// The journal entry: short, factual, in Polish. What was done, decided or found.
    text: String,
    /// Also set the topic status: planned, in_progress, blocked, waiting, done.
    status: Option<String>,
    /// Also replace the topic's summary (the current-state text under the title).
    summary: Option<String>,
    /// Entry author. Default "agent".
    author: Option<String>,
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

impl Morwisko {
    pub fn new(vault: Vault) -> Self {
        Self {
            vault: Ok(vault),
            read_only: false,
        }
    }

    /// A server without a vault (see [`open_read_only`]).
    pub fn unavailable(problem: String) -> Self {
        Self {
            vault: Err(problem),
            read_only: true,
        }
    }

    /// Read-only mode (see [`ENV_READONLY`]).
    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// The refusal a write tool returns in read-only mode; `None` when writes are allowed.
    fn refuse_write(&self, tool: &str) -> Option<CallToolResult> {
        self.read_only.then(|| {
            text(
                format!(
                    "read-only run: `{tool}` is disabled because this session was started with \
{ENV_READONLY}=1. Nothing was changed. Report the change you would make instead of writing it."
                ),
                true,
            )
        })
    }

    /// Run a synchronous vault operation on a blocking thread; `Err(String)` becomes a tool error.
    async fn run<F>(&self, f: F) -> ToolResult
    where
        F: FnOnce(&Vault) -> Result<String, String> + Send + 'static,
    {
        let vault = match &self.vault {
            Ok(v) => v.clone(),
            Err(problem) => return Ok(text(problem.clone(), true)),
        };
        let res = tokio::task::spawn_blocking(move || f(&vault)).await;
        Ok(match res {
            Ok(Ok(s)) => text(s, false),
            Ok(Err(e)) => text(e, true),
            Err(e) => text(format!("Internal error in morwisko-mcp: {e}"), true),
        })
    }
}

#[tool_router]
impl Morwisko {
    #[tool(
        description = "Session-start briefing of the user's work as compact Markdown: today's date, \
projects with their topics (status, one-line summary, open-task count), a Today block (overdue, due \
today, in progress, blocked tasks with ids), the inbox count and the latest journal entries. Call this \
first in every session. Pass `project` for a detailed view of one project.",
        annotations(read_only_hint = true)
    )]
    async fn overview(&self, Parameters(a): Parameters<OverviewArgs>) -> ToolResult {
        let ro = self.read_only;
        self.run(move |v| {
            let snap = fresh(v, ro)?;
            let project = match nonempty(a.project) {
                Some(p) => Some(resolve_project(&snap, &p)?.clone()),
                None => None,
            };
            Ok(render::overview(&snap, v.now().date(), project.as_ref()))
        })
        .await
    }

    #[tool(
        description = "Full view of one topic: status, repo, summary, tasks (open ones unless \
include_done) with links and notes, and the journal, newest first. Long task lists and journals are \
paged (tasks_limit/tasks_offset, journal_limit/journal_offset); the output says when more exist. Use \
before working on a topic.",
        annotations(read_only_hint = true)
    )]
    async fn get_topic(&self, Parameters(a): Parameters<GetTopicArgs>) -> ToolResult {
        let ro = self.read_only;
        self.run(move |v| {
            let snap = fresh(v, ro)?;
            let p = resolve_project(&snap, &a.project)?;
            let t = resolve_topic(&snap, p, &a.topic)?;
            Ok(render::topic_full(
                &snap,
                t,
                a.include_done.unwrap_or(false),
                v.root(),
                render::TopicPaging {
                    tasks_offset: a.tasks_offset.unwrap_or(0),
                    tasks_limit: limit(a.tasks_limit, TOPIC_TASKS_LIMIT),
                    journal_offset: a.journal_offset.unwrap_or(0),
                    journal_limit: limit(a.journal_limit, TOPIC_JOURNAL_LIMIT),
                },
            ))
        })
        .await
    }

    #[tool(
        description = "List tasks with filters: by project, topic, inbox, status, due-date range. \
Open tasks only unless include_done (or status contains done). Paged: 50 tasks by default (limit, \
offset); the output says when more exist. Use to find task ids, to see what is due this week, or to \
review the inbox.",
        annotations(read_only_hint = true)
    )]
    async fn list_tasks(&self, Parameters(a): Parameters<ListTasksArgs>) -> ToolResult {
        let ro = self.read_only;
        self.run(move |v| {
            let snap = fresh(v, ro)?;
            let mut f = TaskFilter {
                inbox_only: a.inbox_only.unwrap_or(false),
                include_done: a.include_done.unwrap_or(false),
                ..Default::default()
            };
            let topic_in = nonempty(a.topic);
            match (nonempty(a.project), topic_in) {
                (Some(p), t) => {
                    let p = resolve_project(&snap, &p)?;
                    f.project = Some(p.slug.clone());
                    if let Some(t) = t {
                        f.topic = Some(resolve_topic(&snap, p, &t)?.slug.clone());
                    }
                }
                (None, Some(t)) => {
                    let t = resolve_topic_anywhere(&snap, &t)?;
                    f.project = Some(t.project.clone());
                    f.topic = Some(t.slug.clone());
                }
                (None, None) => {}
            }
            for s in a.status.unwrap_or_default() {
                f.statuses.push(parse_task_status(&s)?);
            }
            if let Some(d) = nonempty(a.due_from) {
                f.due_from = Some(parse_date(&d)?);
            }
            if let Some(d) = nonempty(a.due_to) {
                f.due_to = Some(parse_date(&d)?);
            }
            let tasks = snap.tasks(&f);
            let offset = a.offset.unwrap_or(0);
            let limit = limit(a.limit, LIST_LIMIT);
            let page: Vec<_> = tasks.iter().skip(offset).take(limit).collect();
            let mut out = format!("{} task(s)", tasks.len());
            if page.len() < tasks.len() {
                if page.is_empty() {
                    out.push_str(&format!("; none at offset {offset}"));
                } else {
                    out.push_str(&format!(
                        "; showing {}–{}",
                        offset + 1,
                        offset + page.len()
                    ));
                }
            }
            out.push('\n');
            for t in &page {
                out.push_str(&render::task_line(t));
                out.push('\n');
            }
            let next = offset + page.len();
            if next < tasks.len() {
                out.push_str(&format!(
                    "… and {} more (pass offset={next} to see the next page, or narrow the filter)\n",
                    tasks.len() - next
                ));
            }
            Ok(out)
        })
        .await
    }

    #[tool(
        description = "Search tasks (title, notes), topics (title, summary) and journal entries \
(text, author) for text. Ignores case and Polish diacritics; all words must match. Shows 20 results per \
kind by default (limit, offset). Use to check whether something was already tracked or decided before \
creating or doing it.",
        annotations(read_only_hint = true)
    )]
    async fn search(&self, Parameters(a): Parameters<SearchArgs>) -> ToolResult {
        let ro = self.read_only;
        self.run(move |v| {
            if a.query.trim().is_empty() {
                return Err("`query` must not be empty.".into());
            }
            let snap = fresh(v, ro)?;
            Ok(render::search_results(
                &snap.search(&a.query),
                v.root(),
                a.offset.unwrap_or(0),
                limit(a.limit, SEARCH_LIMIT),
            ))
        })
        .await
    }

    #[tool(
        description = "Create a task. With no project it goes to the inbox (for things the user \
should triage); with a project and no topic it is a project-level task; with project and topic it \
belongs to that topic. Use for follow-ups you discover. Returns the task with its new id.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn create_task(&self, Parameters(a): Parameters<CreateTaskArgs>) -> ToolResult {
        if let Some(refused) = self.refuse_write("create_task") {
            return Ok(refused);
        }
        self.run(move |v| {
            let snap = fresh(v, false)?;
            let loc = match (nonempty(a.project), nonempty(a.topic)) {
                (None, None) => TaskLocation::Inbox,
                (None, Some(_)) => {
                    return Err("`topic` needs `project`; omit both for an inbox task.".into());
                }
                (Some(p), None) => TaskLocation::Project {
                    project: resolve_project(&snap, &p)?.slug.clone(),
                },
                (Some(p), Some(t)) => {
                    let p = resolve_project(&snap, &p)?;
                    let t = resolve_topic(&snap, p, &t)?;
                    TaskLocation::Topic {
                        project: p.slug.clone(),
                        topic: t.slug.clone(),
                    }
                }
            };
            let new = NewTask {
                title: a.title,
                status: nonempty(a.status)
                    .map(|s| parse_task_status(&s))
                    .transpose()?
                    .unwrap_or_default(),
                priority: nonempty(a.priority)
                    .map(|s| parse_priority(&s))
                    .transpose()?
                    .unwrap_or(Priority::Normal),
                due: nonempty(a.due).map(|s| parse_due(&s)).transpose()?,
                links: a.links.unwrap_or_default(),
                notes: a.notes.unwrap_or_default(),
            };
            let task = v.create_task(&loc, new).map_err(mutation_err)?;
            Ok(format!(
                "Created task:\n{}\nFile: {}",
                render::task_detail(&task, render::NOTES_FULL),
                render::abs_path(v.root(), &loc.rel_path())
            ))
        })
        .await
    }

    #[tool(
        description = "Edit a task by id: title, status (todo/doing/done/blocked), priority, due date \
(or clear_due), add/remove links, notes, or move it (move_to: inbox, project, topic). Use to mark \
progress (doing/done as you go), attach MR or thread URLs via add_links, and reschedule. Only the \
fields you pass change. Edits and the move are applied together or not at all.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    async fn update_task(&self, Parameters(a): Parameters<UpdateTaskArgs>) -> ToolResult {
        if let Some(refused) = self.refuse_write("update_task") {
            return Ok(refused);
        }
        self.run(move |v| {
            let snap = fresh(v, false)?;
            let id = a.id.trim().trim_start_matches('^').to_string();
            let Some(current) = snap.task(&id) else {
                return Err(ce(morwisko_core::Error::TaskNotFound(id)));
            };
            if a.clear_due.unwrap_or(false) && nonempty(a.due.clone()).is_some() {
                return Err("Pass either `due` or `clear_due`, not both.".into());
            }
            let mut patch = TaskPatch {
                title: nonempty(a.title),
                status: nonempty(a.status)
                    .map(|s| parse_task_status(&s))
                    .transpose()?,
                priority: nonempty(a.priority)
                    .map(|s| parse_priority(&s))
                    .transpose()?,
                add_links: a.add_links.unwrap_or_default(),
                remove_links: a.remove_links.unwrap_or_default(),
                notes: a.notes,
                ..Default::default()
            };
            if a.clear_due.unwrap_or(false) {
                patch.due = Some(None);
            } else if let Some(d) = nonempty(a.due) {
                patch.due = Some(Some(parse_due(&d)?));
            }
            let dest = match a.move_to {
                None => None,
                Some(m) => Some(resolve_move(&snap, &current.location, m)?),
            }
            .filter(|d| *d != current.location);
            // One core transaction: the edit and the move are validated (destination
            // included) before anything is written, so they succeed or fail together.
            let (task, heading) = match &dest {
                Some(dest) => {
                    let from = render::location(&current.location);
                    let task = v
                        .update_and_move_task(&id, patch, dest)
                        .map_err(mutation_err)?;
                    let to = render::location(&task.location);
                    (
                        task,
                        format!("Updated and moved task:\nMoved from {from} to {to}.\n"),
                    )
                }
                None => (
                    v.update_task(&id, patch).map_err(mutation_err)?,
                    "Updated task:\n".to_string(),
                ),
            };
            Ok(format!(
                "{heading}{}\nFile: {}",
                render::task_detail(&task, render::NOTES_FULL),
                render::abs_path(v.root(), &task.location.rel_path())
            ))
        })
        .await
    }

    #[tool(
        description = "Create a topic (a unit of work, e.g. a feature or investigation) inside an \
existing project. Only when the user asked for it or it is clearly needed; do not reorganize the \
user's projects unasked.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn create_topic(&self, Parameters(a): Parameters<CreateTopicArgs>) -> ToolResult {
        if let Some(refused) = self.refuse_write("create_topic") {
            return Ok(refused);
        }
        self.run(move |v| {
            let snap = fresh(v, false)?;
            let p = resolve_project(&snap, &a.project)?;
            let repo = match nonempty(a.repo) {
                None => None,
                Some(r) => match p
                    .repos
                    .iter()
                    .find(|x| morwisko_core::fold(&x.name) == morwisko_core::fold(&r))
                {
                    Some(x) => Some(x.name.clone()),
                    None => {
                        let names: Vec<_> = p.repos.iter().map(|x| x.name.as_str()).collect();
                        return Err(format!(
                            "Unknown repo '{r}' in project '{}'. Valid repos: {}.",
                            p.slug,
                            if names.is_empty() {
                                "(none)".to_string()
                            } else {
                                names.join(", ")
                            }
                        ));
                    }
                },
            };
            let new = NewTopic {
                title: a.title,
                status: nonempty(a.status)
                    .map(|s| parse_topic_status(&s))
                    .transpose()?
                    .unwrap_or_default(),
                repo,
                summary: a.summary.unwrap_or_default(),
            };
            let t = v.create_topic(&p.slug, new).map_err(mutation_err)?;
            Ok(format!(
                "Created topic:\n{}",
                render::topic_brief(&t, v.root())
            ))
        })
        .await
    }

    #[tool(
        description = "Append a journal entry to a topic, optionally changing its status and/or \
summary in the same write. Call after meaningful progress, a decision, a blocker or a hand-off. Write \
short, factual Polish text (what was done / decided / what is next). Set `status` when the state \
changed (e.g. waiting for review) and `summary` to keep the topic's current-state text accurate.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn log_update(&self, Parameters(a): Parameters<LogUpdateArgs>) -> ToolResult {
        if let Some(refused) = self.refuse_write("log_update") {
            return Ok(refused);
        }
        self.run(move |v| {
            let snap = fresh(v, false)?;
            let p = resolve_project(&snap, &a.project)?;
            let t = resolve_topic(&snap, p, &a.topic)?;
            let entry = NewJournalEntry {
                author: nonempty(a.author).unwrap_or_else(|| "agent".to_string()),
                text: a.text,
                status: nonempty(a.status)
                    .map(|s| parse_topic_status(&s))
                    .transpose()?,
                summary: a.summary,
            };
            let added = v
                .add_journal_entry(&p.slug, &t.slug, entry)
                .map_err(mutation_err)?;
            Ok(format!(
                "Logged:\n{}\nTopic now:\n{}",
                render::journal_line(&added.entry, true),
                render::topic_brief(&added.topic, v.root())
            ))
        })
        .await
    }
}

fn resolve_move(
    snap: &Snapshot,
    current: &TaskLocation,
    m: MoveTarget,
) -> Result<TaskLocation, String> {
    if m.inbox.unwrap_or(false) {
        return Ok(TaskLocation::Inbox);
    }
    match (nonempty(m.project), nonempty(m.topic)) {
        (None, None) => {
            Err("`move_to` needs `inbox: true`, or `project` (and optionally `topic`).".into())
        }
        (Some(p), None) => Ok(TaskLocation::Project {
            project: resolve_project(snap, &p)?.slug.clone(),
        }),
        (Some(p), Some(t)) => {
            let p = resolve_project(snap, &p)?;
            let t = resolve_topic(snap, p, &t)?;
            Ok(TaskLocation::Topic {
                project: p.slug.clone(),
                topic: t.slug.clone(),
            })
        }
        (None, Some(t)) => match current.project() {
            Some(cp) => {
                let p = resolve_project(snap, cp)?;
                let t = resolve_topic(snap, p, &t)?;
                Ok(TaskLocation::Topic {
                    project: p.slug.clone(),
                    topic: t.slug.clone(),
                })
            }
            None => Err(
                "The task is in the inbox: give `project` as well as `topic` in `move_to`.".into(),
            ),
        },
    }
}

#[tool_handler]
impl ServerHandler for Morwisko {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("morwisko", env!("CARGO_PKG_VERSION"))
                    .with_title("Morwisko command center"),
            )
            .with_instructions(if self.read_only {
                format!("{INSTRUCTIONS}{READ_ONLY_INSTRUCTIONS}")
            } else {
                INSTRUCTIONS.to_string()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::read_only_from_env;

    #[test]
    fn read_only_env_values() {
        for on in ["1", "true", "TRUE", " True ", "1\n"] {
            assert!(read_only_from_env(Some(on)), "{on:?}");
        }
        for off in ["", "0", "false", "yes", "2", "on"] {
            assert!(!read_only_from_env(Some(off)), "{off:?}");
        }
        assert!(!read_only_from_env(None));
    }
}
