//! Syntax + snapshot → [`Preview`]: what would be created or changed, where,
//! the exact Markdown lines, warnings, and choices when a name is ambiguous.
//! Pure apart from reading the task lines of the vault (when one is given) so
//! that the lines shown are the lines on disk.

use std::collections::BTreeSet;
use std::path::Path;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::model::{Due, NewTask, Priority, Task, TaskLocation, TaskStatus};
use crate::query::Snapshot;
use crate::refs::{RefTarget, connection_line, has_connection_line};
use crate::slug::{slugify, unique_slug};
use crate::task::{TaskLine, parse_task_line};
use crate::vault::Vault;

use super::dates::describe_due;
use super::marks::mark_ranges;
use super::parse::{
    AgentVerb, Attrs, Cmd, DestArg, RefArg, RefTok, Span, Subject, Syntax, Tail, Tok, parse,
    text_slice, tokenize,
};
use super::resolve::{
    Choice, Hit, Place, Resolved, find_task, place_of, project_place, resolve_place,
    resolve_search, resolve_topic_ref,
};
use super::suggest::{Similar, similar_tasks};

/// What the app passes along with the text.
#[derive(Debug, Clone)]
pub struct Context<'a> {
    pub today: NaiveDate,
    /// Tasks selected in the app (Skrzynka): the subject of a command that
    /// starts with ` > ` or `/…`, and the entries of `/temat Nazwa > projekt`.
    pub selection: Vec<String>,
    /// The entry focused in the app's list (Skrzynka): the subject when
    /// nothing is selected.
    pub focus: Option<String>,
    /// Where the caret is (UTF-16), while the text is being typed: a token
    /// still being typed there reads as [`State::Typing`], not as an error.
    /// `None` (MCP, apply): every problem counts.
    pub caret: Option<usize>,
    /// Where a new task goes without ` > ` (the inbox, or the quick-capture
    /// window's destination).
    pub default_location: TaskLocation,
    /// For reading the exact task lines (optional; canonical lines without).
    pub vault: Option<&'a Vault>,
}

impl Context<'_> {
    pub fn new(today: NaiveDate) -> Context<'static> {
        Context {
            today,
            selection: Vec::new(),
            focus: None,
            caret: None,
            default_location: TaskLocation::Inbox,
            vault: None,
        }
    }
}

/// Status, priority and due date of a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub status: TaskStatus,
    pub priority: Priority,
    pub due: Option<Due>,
}

impl Fields {
    pub fn of(t: &Task) -> Fields {
        Fields {
            status: t.status,
            priority: t.priority,
            due: t.due,
        }
    }
}

/// An entry a new topic is made of, as the plan saw it (the guard of the move).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub from: TaskLocation,
    pub fields: Fields,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LineTarget {
    Task { id: String },
    Topic { project: String, slug: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Op {
    CreateTask {
        location: TaskLocation,
        task: NewTask,
        /// The id the task will get, chosen with the plan: written exactly,
        /// refused when it exists (so a plan cannot be applied twice).
        id: String,
    },
    /// Fields and/or place of an existing task (one transaction).
    EditTask {
        id: String,
        title: String,
        from: TaskLocation,
        to: TaskLocation,
        before: Fields,
        after: Fields,
        /// The rest of the task as the plan saw it (checked in the lock).
        basis: TaskBasis,
    },
    /// A new topic; with `tasks`, those entries are moved into it.
    CreateTopic {
        project: String,
        title: String,
        slug: String,
        summary: String,
        tasks: Vec<Entry>,
    },
    /// One standalone connection line in a task's notes / a topic's summary.
    AppendLine {
        target: LineTarget,
        title: String,
        line: String,
        /// The exact line is already there: nothing to write.
        already: bool,
        /// The task's notes / the topic's summary as the plan saw them
        /// (checked in the lock).
        basis: String,
    },
}

/// What an edit of a task depends on beyond [`Fields`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskBasis {
    pub title: String,
    pub links: Vec<String>,
    pub notes: String,
    pub done: Option<NaiveDate>,
}

impl TaskBasis {
    pub fn of(t: &Task) -> TaskBasis {
        TaskBasis {
            title: t.title.clone(),
            links: t.links.clone(),
            notes: t.notes.clone(),
            done: t.done,
        }
    }
}

/// The id a new task of this plan gets: derived from `seed` (the text and
/// its context), so previewing the same text against the same vault gives the
/// same id, and the next free one when that id is taken.
pub fn planned_id(seed: &str, taken: &BTreeSet<String>) -> String {
    let mut n: u64 = 0;
    loop {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in seed.bytes().chain(n.to_le_bytes()) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let mut id = String::with_capacity(6);
        for _ in 0..6 {
            let d = (h % 36) as u32;
            id.push(char::from_digit(d, 36).expect("base 36"));
            h /= 36;
        }
        if !taken.contains(&id) {
            return id;
        }
        n += 1;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub op: Op,
    /// Polish, one line.
    pub summary: String,
    /// The file written (relative to the vault).
    pub file: String,
    /// The Markdown lines as they will be (a new task with the id it will get).
    pub lines: Vec<String>,
    /// For edits: the line as it is now.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub before: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub steps: Vec<Step>,
    /// Plain capture (a title with optional `!`/`@`) saves at once; anything
    /// else waits for "Potwierdź".
    pub confirm: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Empty,
    /// Enter saves.
    Capture,
    /// Enter shows the plan, a second Enter writes.
    Confirm,
    /// A request to the agent (`?…`): the app runs it on request.
    Agent,
    /// Nothing can be written (see `problems`, `choices`).
    Invalid,
    /// A word is still being typed at the caret (`/cz`, `^`, ` > ` with
    /// nothing after it, a name with no single match yet): the reading is a
    /// neutral hint, `problems` say what Enter would refuse.
    Typing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpanChoice {
    /// The part of the text the choice replaces.
    pub span: Span,
    pub choice: Choice,
    /// The parts of `choice.label` that match the text in `span` (UTF-16).
    #[serde(default)]
    pub marks: Vec<Span>,
}

fn sc(span: Span, choice: Choice) -> SpanChoice {
    SpanChoice {
        span,
        choice,
        marks: Vec::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentAsk {
    /// `rozpisz`, `dopasuj` or `free`.
    pub verb: String,
    pub text: String,
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preview {
    pub text: String,
    pub state: State,
    /// The live reading under the field.
    pub reading: String,
    pub plan: Option<Plan>,
    pub problems: Vec<String>,
    pub warnings: Vec<String>,
    pub choices: Vec<SpanChoice>,
    /// The plan rests on a typo correction ("literówka").
    pub typo: bool,
    /// Open tasks that look like the new one (duplicate hint).
    pub similar: Vec<Similar>,
    pub agent: Option<AgentAsk>,
}

pub fn status_label(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::Todo => "do zrobienia",
        TaskStatus::Doing => "w toku",
        TaskStatus::Done => "zrobione",
        TaskStatus::Blocked => "zablokowane",
    }
}

pub fn priority_label(p: Priority) -> &'static str {
    match p {
        Priority::High => "wysoki",
        Priority::Normal => "normalny",
        Priority::Low => "niski",
    }
}

/// `1 zadanie`, `2 zadania`, `5 zadań`.
pub fn tasks_word(n: usize) -> String {
    let w = if n == 1 {
        "zadanie"
    } else if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) {
        "zadania"
    } else {
        "zadań"
    };
    format!("{n} {w}")
}

fn rel(loc: &TaskLocation) -> String {
    loc.rel_path().to_string_lossy().replace('\\', "/")
}

/// The line on disk for `task`, when a vault is given and it still says so.
fn disk_line(vault: Option<&Vault>, task: &Task) -> Option<String> {
    let v = vault?;
    let text = std::fs::read_to_string(v.root().join(task.location.rel_path())).ok()?;
    let line = text.lines().nth(task.line.checked_sub(1)?)?.to_string();
    let parsed = parse_task_line(&line)?.ok()?;
    (parsed.id == task.id).then_some(line)
}

fn line_of(task: &Task) -> TaskLine {
    TaskLine {
        bullet: '-',
        status: task.status,
        title: task.title.clone(),
        priority: task.priority,
        due: task.due,
        done: task.done,
        id: task.id.clone(),
        problems: Vec::new(),
    }
}

/// The task line now and after `after` (exactly as core will rewrite it).
fn edited_lines(
    vault: Option<&Vault>,
    task: &Task,
    after: Fields,
    today: NaiveDate,
) -> (String, String) {
    let (text, old) = match disk_line(vault, task) {
        Some(text) => {
            let mut old = parse_task_line(&text)
                .and_then(Result::ok)
                .unwrap_or_else(|| line_of(task));
            old.problems.clear();
            (text, old)
        }
        None => {
            let old = line_of(task);
            (old.render(), old)
        }
    };
    let mut new = old.clone();
    new.status = after.status;
    if after.status.is_done() && !old.status.is_done() {
        new.done = Some(old.done.unwrap_or(today));
    } else if !after.status.is_done() {
        new.done = None;
    }
    new.priority = after.priority;
    new.due = after.due;
    let after_text = TaskLine::rewrite(&text, &old, &new);
    (text, after_text)
}

/// A resolved connection: `(waits, target, label)`.
type Conn = (bool, RefTarget, String);

struct Builder<'a> {
    snap: &'a Snapshot,
    /// The text, for the planned id of a new task.
    seed: &'a str,
    /// Length of the text in UTF-16 units (choices that append go there).
    text_len: usize,
    ctx: &'a Context<'a>,
    problems: Vec<String>,
    warnings: Vec<String>,
    choices: Vec<SpanChoice>,
    typo: bool,
    conn_cache: Vec<(Option<String>, Vec<Conn>)>,
    /// Left out of name searches: the subject of the command.
    exclude: Vec<RefTarget>,
}

enum Found {
    One(Hit),
    Many,
    Nothing,
}

/// How a found task or topic is named in a reading.
fn hit_label(h: &Hit) -> String {
    match h.kind {
        "task" => format!("„{}”", h.title),
        _ => format!("{} / {}", h.place, h.title),
    }
}

impl Builder<'_> {
    fn place(&mut self, dest: &DestArg) -> Option<Place> {
        match resolve_place(self.snap, &dest.text) {
            Resolved::One(p) => Some(p),
            Resolved::Typo(p, c) => {
                self.typo = true;
                self.warnings.push(format!(
                    "„{}” to pewnie literówka: {} ({}).",
                    dest.text, p.label, p.text
                ));
                self.choices.push(sc(dest.span, c));
                Some(p)
            }
            Resolved::Ambiguous(cs) => {
                self.problems.push(format!(
                    "„{}” pasuje do kilku miejsc: {}. Wybierz jedno.",
                    dest.text,
                    cs.iter()
                        .map(|c| c.insert.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                self.choices
                    .extend(cs.into_iter().map(|choice| sc(dest.span, choice)));
                None
            }
            Resolved::Unknown(cs) => {
                self.problems.push(format!(
                    "Nie ma projektu ani tematu „{}”.{}",
                    dest.text,
                    if cs.is_empty() {
                        ""
                    } else {
                        " Może chodzi o jedno z poniższych?"
                    }
                ));
                self.choices
                    .extend(cs.into_iter().map(|choice| sc(dest.span, choice)));
                None
            }
        }
    }

    fn reference(&mut self, r: &RefArg, context: Option<&str>) -> Option<(RefTarget, String)> {
        match &r.r {
            RefTok::Task(id) => match find_task(self.snap, id) {
                Some(t) => Some((RefTarget::Task { id: id.clone() }, format!("„{}”", t.title))),
                None => {
                    self.problems.push(format!("Nie ma zadania ^{id}."));
                    None
                }
            },
            RefTok::Topic(body) => {
                let place = match resolve_topic_ref(self.snap, body, context) {
                    Resolved::One(p) => p,
                    Resolved::Typo(p, c) => {
                        self.typo = true;
                        self.warnings
                            .push(format!("„{body}” to pewnie literówka: {}.", p.text));
                        self.choices.push(sc(
                            r.span,
                            Choice {
                                insert: format!("[[{}]]", c.insert),
                                ..c
                            },
                        ));
                        p
                    }
                    Resolved::Ambiguous(cs) | Resolved::Unknown(cs) => {
                        self.problems.push(if cs.is_empty() {
                            format!("Nie ma tematu „{body}”.")
                        } else {
                            format!("„{body}”: wybierz temat.")
                        });
                        self.choices
                            .extend(cs.into_iter().filter(|c| c.kind == "topic").map(|c| {
                                sc(
                                    r.span,
                                    Choice {
                                        insert: format!("[[{}]]", c.insert),
                                        ..c
                                    },
                                )
                            }));
                        return None;
                    }
                };
                let (project, slug) = match &place.location {
                    TaskLocation::Topic { project, topic } => (project.clone(), topic.clone()),
                    _ => {
                        self.problems.push(format!("„{body}” to nie temat."));
                        return None;
                    }
                };
                Some((RefTarget::Topic { project, slug }, place.label))
            }
            RefTok::Search(q) => match self.found(q, r.span) {
                Found::One(h) => Some((h.target.clone(), hit_label(&h))),
                Found::Many => None,
                Found::Nothing => {
                    self.problems.push(format!(
                        "Nie ma otwartego zadania ani tematu „{q}”. Wpisz inną nazwę, `^id` albo `[[projekt/temat]]`."
                    ));
                    None
                }
            },
        }
    }

    /// Plain words naming one task or topic. Several: a problem and the
    /// choices; a typo: accepted, to be confirmed.
    fn found(&mut self, query: &str, span: Span) -> Found {
        match resolve_search(self.snap, query, &self.exclude) {
            Resolved::One(h) => Found::One(h),
            Resolved::Typo(h, c) => {
                self.typo = true;
                self.warnings
                    .push(format!("„{query}” to pewnie literówka: {}.", hit_label(&h)));
                self.choices.push(sc(span, c));
                Found::One(h)
            }
            Resolved::Ambiguous(cs) => {
                self.problems.push(format!(
                    "„{query}” pasuje do kilku zadań i tematów. Wybierz jedno."
                ));
                self.choices.extend(cs.into_iter().map(|c| sc(span, c)));
                Found::Many
            }
            Resolved::Unknown(_) => Found::Nothing,
        }
    }

    /// Connection lines of the tail: `(waits, target, label)`. Resolved once
    /// per context project (a bulk edit does not repeat problems and choices).
    fn connections(
        &mut self,
        tail: &Tail,
        context: Option<&str>,
    ) -> Vec<(bool, RefTarget, String)> {
        let key = context.map(str::to_string);
        if let Some((_, hit)) = self.conn_cache.iter().find(|(k, _)| *k == key) {
            return hit.clone();
        }
        let out = self.connections_uncached(tail, context);
        self.conn_cache.push((key, out.clone()));
        out
    }

    fn connections_uncached(
        &mut self,
        tail: &Tail,
        context: Option<&str>,
    ) -> Vec<(bool, RefTarget, String)> {
        let mut out = Vec::new();
        for (waits, list) in [(true, &tail.waits), (false, &tail.related)] {
            for r in list {
                if let Some((t, label)) = self.reference(r, context)
                    && !out.iter().any(|(w, x, _)| *w == waits && *x == t)
                {
                    out.push((waits, t, label));
                }
            }
        }
        out
    }

    fn due(&self, attrs: &Attrs) -> Option<Due> {
        attrs.due.map(|(d, _)| d.resolve(self.ctx.today))
    }

    fn describe_attrs(
        &self,
        status: Option<TaskStatus>,
        priority: Option<Priority>,
        due: Option<Due>,
        relative: bool,
    ) -> Vec<String> {
        let mut parts = Vec::new();
        if let Some(s) = status {
            parts.push(status_label(s).to_string());
        }
        if let Some(d) = due {
            let mut s = format!("termin {}", describe_due(&d, self.ctx.today));
            if relative {
                s.push_str(&format!(" ({d})"));
            }
            parts.push(s);
        }
        if let Some(p) = priority {
            parts.push(format!("priorytet {}", priority_label(p)));
        }
        parts
    }
}

fn conn_summary(conns: &[(bool, RefTarget, String)]) -> Vec<String> {
    conns
        .iter()
        .map(|(w, t, label)| {
            format!(
                "{} {} {label}",
                if *w { "czeka na" } else { "powiązane z" },
                t.text()
            )
        })
        .collect()
}

fn new_task(b: &mut Builder, title: &str, attrs: &Attrs, tail: &Tail) -> (Vec<Step>, bool) {
    let place = match &tail.dest {
        Some(d) => b.place(d),
        None => Some(place_of(b.snap, &b.ctx.default_location)),
    };
    let context = place
        .as_ref()
        .and_then(|p| p.location.project().map(str::to_string));
    let conns = b.connections(tail, context.as_deref());
    let confirm = tail.dest.is_some()
        || attrs.status.is_some()
        || !conns.is_empty()
        || !tail.waits.is_empty()
        || !tail.related.is_empty();
    let Some(place) = place else {
        return (Vec::new(), confirm);
    };
    let notes = conns
        .iter()
        .map(|(w, t, _)| connection_line(*w, t))
        .collect::<Vec<_>>()
        .join("\n");
    let task = NewTask {
        title: title.to_string(),
        status: attrs.status.map(|s| s.0).unwrap_or_default(),
        priority: attrs.priority.map(|p| p.0).unwrap_or_default(),
        due: b.due(attrs),
        links: Vec::new(),
        notes,
    };
    let line = match Vault::render_new_task_line(&task, b.ctx.today) {
        Ok(l) => l,
        Err(_) => {
            b.problems.push(format!(
                "Tytuł „{title}” kończy się czymś, co wygląda na znacznik (`^id`, `@data`, `!priorytet`), \
więc nie da się go zapisać jednoznacznie. Zmień końcówkę albo użyj /powiaz."
            ));
            return (Vec::new(), confirm);
        }
    };
    let taken: BTreeSet<String> = b.snap.tasks.iter().filter_map(|t| t.id.clone()).collect();
    let seed = format!(
        "{}|{:?}|{}|{:?}",
        b.seed, b.ctx.default_location, b.ctx.today, b.ctx.selection
    );
    let id = planned_id(&seed, &taken);
    let mut lines = vec![format!("{line} ^{id}")];
    lines.extend(task.notes.lines().map(|l| format!("  {l}")));
    let mut parts = vec![format!("Nowe zadanie „{title}” w: {}", place.label)];
    parts.extend(
        b.describe_attrs(
            attrs.status.map(|s| s.0).filter(|s| *s != TaskStatus::Todo),
            attrs
                .priority
                .map(|p| p.0)
                .filter(|p| *p != Priority::Normal),
            task.due,
            attrs.due.is_some_and(|(d, _)| d.is_relative()),
        ),
    );
    parts.extend(conn_summary(&conns));
    let step = Step {
        file: rel(&place.location),
        op: Op::CreateTask {
            location: place.location,
            task,
            id,
        },
        summary: parts.join(" · "),
        lines,
        before: Vec::new(),
    };
    (vec![step], confirm)
}

fn new_topic(b: &mut Builder, title: &str, ids: &[(String, Span)], tail: &Tail) -> Vec<Step> {
    let mut wanted: Vec<String> = ids.iter().map(|(i, _)| i.clone()).collect();
    if wanted.is_empty() {
        wanted = b.ctx.selection.clone();
    }
    let mut tasks: Vec<&Task> = Vec::new();
    for id in &wanted {
        match find_task(b.snap, id) {
            Some(t) => {
                if !tasks.iter().any(|x| x.id == t.id) {
                    tasks.push(t)
                }
            }
            None => b.problems.push(format!("Nie ma zadania ^{id}.")),
        }
    }
    let project = match &tail.dest {
        None => {
            b.problems
                .push("Temat zakłada się w projekcie: dopisz ` > projekt`.".into());
            for p in b
                .snap
                .projects
                .iter()
                .filter(|p| p.state != crate::model::ProjectState::Archived)
            {
                let sep = if b.seed.ends_with(char::is_whitespace) {
                    ""
                } else {
                    " "
                };
                b.choices.push(sc(
                    Span {
                        from: b.text_len,
                        to: b.text_len,
                    },
                    Choice {
                        label: p.name.clone(),
                        insert: format!("{sep}> {}", p.slug),
                        kind: "project".into(),
                        typo: false,
                    },
                ));
            }
            None
        }
        Some(d) => match b.place(d) {
            Some(p) => match &p.location {
                TaskLocation::Project { project } => Some(project.clone()),
                TaskLocation::Topic { project, .. } => {
                    b.problems.push(format!(
                        "„{}” to temat. Nowy temat zakłada się w projekcie: ` > {project}`.",
                        d.text
                    ));
                    None
                }
                TaskLocation::Inbox => {
                    b.problems
                        .push("Temat zakłada się w projekcie, nie w Skrzynce.".into());
                    None
                }
            },
            None => None,
        },
    };
    let title = if title.trim().is_empty() {
        match tasks.as_slice() {
            [one] => one.title.clone(),
            [] => {
                b.problems.push("Podaj nazwę tematu.".into());
                return Vec::new();
            }
            _ => {
                b.problems
                    .push("Podaj nazwę tematu dla zaznaczonych wpisów.".into());
                return Vec::new();
            }
        }
    } else {
        title.trim().to_string()
    };
    let title = match topic_title(&title) {
        Ok(t) => t,
        Err(why) => {
            b.problems.push(why);
            return Vec::new();
        }
    };
    let Some(project) = project else {
        return Vec::new();
    };
    let conns = b.connections(tail, Some(&project));
    let taken: BTreeSet<&str> = b
        .snap
        .topics
        .iter()
        .filter(|t| t.project == project)
        .map(|t| t.slug.as_str())
        .collect();
    let exists_on_disk = |s: &str| {
        b.ctx.vault.is_some_and(|v| {
            v.root()
                .join(&project)
                .join("tematy")
                .join(format!("{s}.md"))
                .exists()
        })
    };
    let slug = unique_slug(&slugify(&title), |s| taken.contains(s) || exists_on_disk(s));
    let summary = conns
        .iter()
        .map(|(w, t, _)| connection_line(*w, t))
        .collect::<Vec<_>>()
        .join("\n");
    let mut lines = vec![format!("# {title}")];
    if !summary.is_empty() {
        lines.push(String::new());
        lines.extend(summary.lines().map(str::to_string));
    }
    lines.push(String::new());
    lines.push("## Zadania".into());
    for t in &tasks {
        lines.push(disk_line(b.ctx.vault, t).unwrap_or_else(|| line_of(t).render()));
        lines.extend(t.links.iter().map(|l| format!("  - {l}")));
        lines.extend(t.notes.lines().map(|l| format!("  {l}")));
    }
    let pname = project_place(b.snap, &project).label;
    let mut summary_text = format!("Nowy temat „{title}” w: {pname}");
    if !tasks.is_empty() {
        summary_text.push_str(&format!(
            " · przeniesie do niego {} (z identyfikatorami i notatkami)",
            tasks_word(tasks.len())
        ));
    }
    for c in conn_summary(&conns) {
        summary_text.push_str(&format!(" · {c}"));
    }
    if slug != slugify(&title) {
        b.warnings.push(format!(
            "Plik dostanie nazwę {slug}.md (taki temat już jest)."
        ));
    }
    vec![Step {
        file: format!("{project}/tematy/{slug}.md"),
        op: Op::CreateTopic {
            project,
            title,
            slug,
            summary,
            tasks: tasks
                .iter()
                .map(|t| Entry {
                    id: t.id.clone().unwrap_or_default(),
                    from: t.location.clone(),
                    fields: Fields::of(t),
                })
                .collect(),
        },
        summary: summary_text,
        lines,
        before: Vec::new(),
    }]
}

fn edit(b: &mut Builder, subject: &Subject, attrs: &Attrs, tail: &Tail) -> Vec<Step> {
    match subject {
        // A topic: only connection lines.
        Subject::Topic(body, span) => {
            let place = match resolve_topic_ref(b.snap, body, None) {
                Resolved::One(p) => Some(p),
                Resolved::Typo(p, c) => {
                    b.typo = true;
                    b.warnings
                        .push(format!("„{body}” to pewnie literówka: {}.", p.text));
                    b.choices.push(sc(
                        *span,
                        Choice {
                            insert: format!("[[{}]]", c.insert),
                            ..c
                        },
                    ));
                    Some(p)
                }
                Resolved::Ambiguous(cs) | Resolved::Unknown(cs) => {
                    b.problems
                        .push(format!("Nie wiem, który to temat: „{body}”."));
                    b.choices
                        .extend(cs.into_iter().filter(|c| c.kind == "topic").map(|c| {
                            sc(
                                *span,
                                Choice {
                                    insert: format!("[[{}]]", c.insert),
                                    ..c
                                },
                            )
                        }));
                    None
                }
            };
            if tail.waits.is_empty() && tail.related.is_empty() {
                b.problems
                    .push("Przy temacie wpisz /czeka albo /powiaz z odwołaniem.".into());
                return Vec::new();
            }
            let Some(TaskLocation::Topic { project, topic }) = place.map(|p| p.location) else {
                return Vec::new();
            };
            edit_topic(b, &project, &topic, tail)
        }
        Subject::Search(query, span) => match b.found(query, *span) {
            Found::One(h) => match h.target {
                RefTarget::Task { id } => edit_tasks(b, &[id], attrs, tail),
                RefTarget::Topic { project, slug } => {
                    if attrs != &Attrs::default() {
                        b.problems
                            .push("Status, priorytet i termin mają zadania, nie tematy.".into());
                        return Vec::new();
                    }
                    edit_topic(b, &project, &slug, tail)
                }
            },
            Found::Many => Vec::new(),
            Found::Nothing => {
                b.warnings.push(format!(
                    "Nie ma otwartego zadania ani tematu „{query}”, więc powstanie nowe zadanie. Żeby dopisać do istniejącego, wpisz inną nazwę albo `^id`."
                ));
                new_task(b, query, attrs, tail).0
            }
        },
        Subject::Tasks(ids) => {
            let ids: Vec<String> = ids.iter().map(|(i, _)| i.clone()).collect();
            edit_tasks(b, &ids, attrs, tail)
        }
        Subject::Selection => {
            let ids: Vec<String> = if b.ctx.selection.is_empty() {
                b.ctx.focus.iter().cloned().collect()
            } else {
                b.ctx.selection.clone()
            };
            edit_tasks(b, &ids, attrs, tail)
        }
    }
}

/// Connection lines in a topic's summary.
fn edit_topic(b: &mut Builder, project: &str, topic: &str, tail: &Tail) -> Vec<Step> {
    let (project, topic) = (project.to_string(), topic.to_string());
    b.exclude = vec![RefTarget::Topic {
        project: project.clone(),
        slug: topic.clone(),
    }];
    let conns = b.connections(tail, Some(&project));
    let Some(t) = b.snap.topic(&project, &topic) else {
        return Vec::new();
    };
    let self_ref = RefTarget::Topic {
        project: project.clone(),
        slug: topic.clone(),
    };
    let mut steps = Vec::new();
    for (waits, target, label) in conns {
        if target == self_ref {
            b.problems
                .push("Temat nie może czekać na samego siebie ani być z sobą powiązany.".into());
            continue;
        }
        let line = connection_line(waits, &target);
        let already = has_connection_line(&t.summary, Some(&project), waits, &target);
        if !already && !reads_back(&t.summary, Some(&project), waits, &target) {
            b.problems.push(format!(
                "Opis stanu tematu „{}” kończy się otwartym blokiem kodu: dopisana linia nie byłaby powiązaniem. Popraw opis w temacie.",
                t.title
            ));
            continue;
        }
        steps.push(Step {
            file: rel(&TaskLocation::Topic {
                project: project.clone(),
                topic: topic.clone(),
            }),
            summary: format!(
                "Temat „{}”: dopisze w opisie stanu linię „{line}” ({label}){}",
                t.title,
                if already {
                    " — już jest, nic nie zmieni"
                } else {
                    ""
                }
            ),
            op: Op::AppendLine {
                target: LineTarget::Topic {
                    project: project.clone(),
                    slug: topic.clone(),
                },
                title: t.title.clone(),
                line: line.clone(),
                already,
                basis: t.summary.clone(),
            },
            lines: vec![line],
            before: Vec::new(),
        });
    }
    steps
}

/// Fields, place and connection lines of existing tasks.
fn edit_tasks(b: &mut Builder, ids: &[String], attrs: &Attrs, tail: &Tail) -> Vec<Step> {
    b.exclude = ids
        .iter()
        .map(|id| RefTarget::Task { id: id.clone() })
        .collect();
    if ids.is_empty() {
        b.problems
            .push("Zaznacz wpisy na liście albo zacznij od `^id` zadania.".into());
        return Vec::new();
    }
    let mut tasks: Vec<Task> = Vec::new();
    for id in ids {
        match find_task(b.snap, id) {
            Some(t) if !tasks.iter().any(|x| x.id == t.id) => tasks.push(t.clone()),
            Some(_) => {}
            None => b.problems.push(format!("Nie ma zadania ^{id}.")),
        }
    }
    let place = tail.dest.as_ref().and_then(|d| b.place(d));
    if tail.dest.is_some() && place.is_none() {
        return Vec::new();
    }
    if attrs == &Attrs::default()
        && tail.dest.is_none()
        && tail.waits.is_empty()
        && tail.related.is_empty()
    {
        b.problems.push(
            "Co zrobić z zadaniem? Wpisz ` > miejsce`, status (`[x]`), `!priorytet`, `@termin`, `/czeka` albo `/powiaz`."
                .into(),
        );
        return Vec::new();
    }
    let due = b.due(attrs);
    let mut steps = Vec::new();
    for t in &tasks {
        let id = t.id.clone().unwrap_or_default();
        let before = Fields::of(t);
        let after = Fields {
            status: attrs.status.map_or(before.status, |s| s.0),
            priority: attrs.priority.map_or(before.priority, |p| p.0),
            due: if attrs.due.is_some() { due } else { before.due },
        };
        let to = place
            .as_ref()
            .map_or(t.location.clone(), |p| p.location.clone());
        if after != before || to != t.location {
            let (line_before, line_after) = edited_lines(b.ctx.vault, t, after, b.ctx.today);
            let mut parts = Vec::new();
            if to != t.location {
                parts.push(format!(
                    "przeniesie z: {} do: {}",
                    place_of(b.snap, &t.location).label,
                    place_of(b.snap, &to).label
                ));
            }
            parts.extend(b.describe_attrs(
                (after.status != before.status).then_some(after.status),
                (after.priority != before.priority).then_some(after.priority),
                (after.due != before.due).then_some(after.due).flatten(),
                attrs.due.is_some_and(|(d, _)| d.is_relative()),
            ));
            steps.push(Step {
                file: rel(&to),
                op: Op::EditTask {
                    id: id.clone(),
                    title: t.title.clone(),
                    from: t.location.clone(),
                    to,
                    before,
                    after,
                    basis: TaskBasis::of(t),
                },
                summary: format!("„{}”: {}", t.title, parts.join(" · ")),
                lines: vec![line_after],
                before: vec![line_before],
            });
        } else if attrs != &Attrs::default() || place.is_some() {
            b.warnings
                .push(format!("„{}” już tak wygląda: bez zmian.", t.title));
        }
        let project = place
            .as_ref()
            .and_then(|p| p.location.project().map(str::to_string))
            .or_else(|| t.location.project().map(str::to_string));
        let conns = b.connections(tail, project.as_deref());
        for (waits, target, label) in conns {
            if target == (RefTarget::Task { id: id.clone() }) {
                b.problems.push(
                    "Zadanie nie może czekać na samo siebie ani być z sobą powiązane.".into(),
                );
                continue;
            }
            let line = connection_line(waits, &target);
            let already = has_connection_line(&t.notes, t.location.project(), waits, &target);
            if !already && !reads_back(&t.notes, t.location.project(), waits, &target) {
                b.problems.push(format!(
                    "Notatki zadania „{}” kończą się otwartym blokiem kodu: dopisana linia nie byłaby powiązaniem. Popraw notatki w zadaniu.",
                    t.title
                ));
                continue;
            }
            steps.push(Step {
                file: rel(place.as_ref().map_or(&t.location, |p| &p.location)),
                summary: format!(
                    "„{}”: dopisze w notatkach linię „{line}” ({label}){}",
                    t.title,
                    if already {
                        " — już jest, nic nie zmieni"
                    } else {
                        ""
                    }
                ),
                op: Op::AppendLine {
                    target: LineTarget::Task { id: id.clone() },
                    title: t.title.clone(),
                    line: line.clone(),
                    already,
                    basis: t.notes.clone(),
                },
                lines: vec![format!("  {line}")],
                before: Vec::new(),
            });
        }
    }
    steps
}

fn reading(steps: &[Step], snap: &Snapshot) -> String {
    match steps {
        [] => String::new(),
        [one] => one.summary.clone(),
        many => {
            let moves: Vec<&TaskLocation> = many
                .iter()
                .filter_map(|s| match &s.op {
                    Op::EditTask {
                        from,
                        to,
                        before,
                        after,
                        ..
                    } if from != to && before == after => Some(to),
                    _ => None,
                })
                .collect();
            if moves.len() == many.len() && moves.iter().all(|t| *t == moves[0]) {
                return format!(
                    "Przeniesie {} do: {}",
                    tasks_word(many.len()),
                    place_of(snap, moves[0]).label
                );
            }
            let edits = many
                .iter()
                .filter(|s| matches!(s.op, Op::EditTask { .. }))
                .count();
            if edits == many.len() {
                return format!("Zmieni {}", tasks_word(edits));
            }
            format!("{} (i jeszcze {} zmian)", many[0].summary, many.len() - 1)
        }
    }
}

/// A topic title as core will store it, refused when it would read as
/// Markdown structure instead of a title (`## Zadania`, `---`, `- [ ] x`, …).
fn topic_title(title: &str) -> Result<String, String> {
    let t = crate::check::topic_title(title)
        .map_err(|e| format!("Tej nazwy tematu nie da się zapisać ({e})."))?;
    let first = t.chars().next().unwrap_or(' ');
    let only_rule = t.chars().all(|c| matches!(c, '-' | '=' | '*' | '_' | ' '));
    let structural = first == '#'
        || first == '>'
        || only_rule
        || t.starts_with("```")
        || t.starts_with("~~~")
        || t.starts_with("- ")
        || t.starts_with("* ")
        || t.starts_with("+ ");
    if structural {
        return Err(format!(
            "„{t}” wyglądałoby w pliku jak nagłówek, lista albo linia formatowania, a nie nazwa tematu. Zmień początek nazwy."
        ));
    }
    Ok(t)
}

/// Whether `line` appended at the end of `text` reads back as that connection
/// (it does not when the text ends inside an unclosed code block).
fn reads_back(text: &str, project: Option<&str>, waits: bool, target: &RefTarget) -> bool {
    let joined = if text.is_empty() {
        connection_line(waits, target)
    } else {
        format!("{text}\n{}", connection_line(waits, target))
    };
    has_connection_line(&joined, project, waits, target)
}

const HINT_CMD: &str = "Komendy: /temat nowy temat · /czeka czeka na… · /powiaz powiązane z…";
const HINT_TASK: &str = "Zadanie: ^ i identyfikator albo słowa z tytułu, wybierz z listy";
const HINT_TOPIC: &str = "Temat: wpisz nazwę i zamknij ]] albo wybierz z listy";
const HINT_STATUS: &str = "Status: [ ] do zrobienia · [/] w toku · [x] zrobione · [!] zablokowane";
const HINT_PRIORITY: &str = "Priorytet: !wysoki · !normalny · !niski";
const HINT_DUE: &str = "Termin: @dzis, @jutro, @pt, @+3d albo @2026-11-20";
const HINT_DEST: &str = "Dokąd? Wpisz projekt albo temat i wybierz z listy";
const HINT_WAITS: &str =
    "Na co czeka? Wpisz nazwę zadania albo tematu i wybierz z listy (albo ^id, [[projekt/temat]])";
const HINT_RELATED: &str = "Z czym powiązać? Wpisz nazwę zadania albo tematu i wybierz z listy (albo ^id, [[projekt/temat]])";
const HINT_TEMAT: &str = "Nazwa nowego tematu, potem „ > projekt”";
const HINT_TEMAT_DEST: &str = "W którym projekcie? Dopisz „ > projekt” albo wybierz z listy";
const HINT_SUBJECT: &str = "Które zadanie albo temat? Wybierz z listy albo pisz dalej";

fn is_prefix_of_any(typed: &str, words: &[&str]) -> bool {
    let t = crate::slug::fold(typed);
    words.iter().any(|w| w.starts_with(&t))
}

/// The neutral hint for a word still being typed at `caret`, if there is
/// one: a half-typed token (`/cz`, `^`, `[[kr`, `!`, `@ju`, a lone ` > ` or
/// `/czeka`), or, when the text cannot be written (`invalid`), a name being
/// typed where the caret is (a destination, a reference, the subject).
fn typing_hint(text: &str, caret: usize, syntax: &Syntax, invalid: bool) -> Option<String> {
    let toks = tokenize(text);
    let after = |at: usize| toks.iter().any(|t| t.span.from >= at);
    let cmd_hint = |c: &Cmd| match c {
        Cmd::Czeka => HINT_WAITS,
        Cmd::Powiaz => HINT_RELATED,
        Cmd::Temat => HINT_TEMAT,
    };
    let at = toks
        .iter()
        .find(|t| t.span.from < caret && caret <= t.span.to);
    if let Some(t) = at {
        let r = t.raw.as_str();
        let hint = match &t.tok {
            Tok::Word(_) => {
                if let Some(rest) = r.strip_prefix('/') {
                    is_prefix_of_any(rest, &["temat", "czeka", "powiaz", "powiazane"])
                        .then_some(HINT_CMD)
                } else if r.starts_with("[[") {
                    Some(HINT_TOPIC)
                } else if r.starts_with('^') {
                    Some(HINT_TASK)
                } else if r.starts_with('[') && r.chars().count() <= 2 {
                    Some(HINT_STATUS)
                } else if let Some(rest) = r.strip_prefix('!') {
                    is_prefix_of_any(rest, &["wysoki", "niski", "normalny", "pilne"])
                        .then_some(HINT_PRIORITY)
                } else if let Some(rest) = r.strip_prefix('@') {
                    let partial_date = rest
                        .chars()
                        .all(|c| c.is_ascii_digit() || matches!(c, '-' | ':' | '+' | 'T'));
                    (partial_date
                        || is_prefix_of_any(
                            rest,
                            &[
                                "dzis", "jutro", "pojutrze", "pn", "wt", "sr", "cz", "pt", "sb",
                                "nd",
                            ],
                        )
                        || (rest.starts_with('+') && rest.len() <= 4))
                        .then_some(HINT_DUE)
                } else {
                    None
                }
            }
            Tok::Arrow if !after(t.span.to) => Some(HINT_DEST),
            Tok::Cmd(c) if !after(t.span.to) => Some(cmd_hint(c)),
            _ => None,
        };
        if let Some(h) = hint {
            return Some(h.into());
        }
    } else if !after(caret)
        && let Some(last) = toks.iter().rev().find(|t| t.span.to <= caret)
    {
        match &last.tok {
            Tok::Arrow => return Some(HINT_DEST.into()),
            Tok::Cmd(c) => return Some(cmd_hint(c).into()),
            _ => {}
        }
    }
    if !invalid {
        return None;
    }
    let within = |s: Span| s.from <= caret && caret <= s.to;
    let tail = match syntax {
        Syntax::NewTask { tail, .. }
        | Syntax::NewTopic { tail, .. }
        | Syntax::Edit { tail, .. } => tail,
        _ => return None,
    };
    if tail.dest.as_ref().is_some_and(|d| within(d.span)) {
        return Some(HINT_DEST.into());
    }
    if tail.waits.iter().any(|r| within(r.span)) {
        return Some(HINT_WAITS.into());
    }
    if tail.related.iter().any(|r| within(r.span)) {
        return Some(HINT_RELATED.into());
    }
    match syntax {
        Syntax::Edit {
            subject: Subject::Search(_, span),
            ..
        } if within(*span) => Some(HINT_SUBJECT.into()),
        Syntax::NewTopic { tail, .. } if tail.dest.is_none() && !after(caret) => {
            Some(HINT_TEMAT_DEST.into())
        }
        _ => None,
    }
}

/// A stable token for a plan: the MCP `command` tool hands it out with a
/// preview and writes only when the rebuilt plan has the same token.
pub fn plan_token(plan: &Plan) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in format!("{plan:?}").bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn dedup<T: PartialEq>(v: &mut Vec<T>) {
    let mut out: Vec<T> = Vec::with_capacity(v.len());
    for x in v.drain(..) {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    *v = out;
}

/// The live reading of `text` against `snap`.
pub fn preview(snap: &Snapshot, text: &str, ctx: &Context) -> Preview {
    let parsed = parse(text);
    let mut b = Builder {
        snap,
        seed: text,
        text_len: text.chars().map(char::len_utf16).sum(),
        ctx,
        problems: parsed.problems,
        warnings: parsed.warnings,
        choices: Vec::new(),
        typo: false,
        conn_cache: Vec::new(),
        exclude: Vec::new(),
    };
    let mut similar = Vec::new();
    let mut agent = None;
    let mut confirm = true;
    let steps = match &parsed.syntax {
        Syntax::Empty => Vec::new(),
        Syntax::Agent { verb, text, ids } => {
            agent = Some(AgentAsk {
                verb: match verb {
                    AgentVerb::Rozpisz => "rozpisz",
                    AgentVerb::Dopasuj => "dopasuj",
                    AgentVerb::Free => "free",
                }
                .into(),
                text: text.clone(),
                ids: ids.clone(),
            });
            Vec::new()
        }
        Syntax::NewTask { title, attrs, tail } => {
            if !title.is_empty() {
                similar = similar_tasks(snap, title, &[]);
            }
            let (steps, c) = new_task(&mut b, title, attrs, tail);
            confirm = c;
            steps
        }
        Syntax::NewTopic { title, ids, tail } => new_topic(&mut b, title, ids, tail),
        Syntax::Edit {
            subject,
            attrs,
            tail,
        } => edit(&mut b, subject, attrs, tail),
    };
    dedup(&mut b.problems);
    dedup(&mut b.warnings);
    dedup(&mut b.choices);
    for c in &mut b.choices {
        if c.span.from < c.span.to {
            let typed = text_slice(text, c.span);
            c.marks = mark_ranges(&c.choice.label, &typed);
        }
    }
    let mut state = match (&parsed.syntax, b.problems.is_empty()) {
        (Syntax::Empty, _) => State::Empty,
        (Syntax::Agent { .. }, true) => State::Agent,
        (_, false) => State::Invalid,
        _ if steps.is_empty() => State::Invalid,
        _ if confirm || b.typo => State::Confirm,
        _ => State::Capture,
    };
    let typing = ctx
        .caret
        .and_then(|c| typing_hint(text, c, &parsed.syntax, state == State::Invalid));
    if let Some(hint) = &typing
        && state != State::Empty
        && state != State::Agent
    {
        state = State::Typing;
        if b.problems.is_empty() {
            b.problems.push(format!("Dokończ komendę: {hint}"));
        }
    }
    let plan = (matches!(state, State::Capture | State::Confirm)).then(|| Plan {
        confirm: state == State::Confirm,
        steps: steps.clone(),
    });
    let reading = match state {
        State::Empty => String::new(),
        State::Agent => match agent.as_ref().map(|a| a.verb.as_str()) {
            Some("rozpisz") => "Prośba do agenta: rozpisać wpis na kroki (agent nic nie zapisze, dostaniesz propozycje)".into(),
            Some("dopasuj") => "Prośba do agenta: zaproponować miejsca dla wpisów (do zaznaczenia)".into(),
            _ => "Prośba do agenta (nic nie zapisze bez twojego potwierdzenia)".into(),
        },
        State::Typing => typing.unwrap_or_default(),
        // Any problem: the reading is the problem, never the steps.
        State::Invalid => b
            .problems
            .first()
            .cloned()
            .unwrap_or_else(|| "Tego nie da się zapisać.".into()),
        _ => reading(&steps, snap),
    };
    Preview {
        text: text.to_string(),
        state,
        reading,
        plan,
        problems: b.problems,
        warnings: b.warnings,
        choices: b.choices,
        typo: b.typo,
        similar,
        agent,
    }
}

/// Whether `vault`-relative `file` exists (for tests and callers).
pub fn file_exists(vault: &Vault, file: &str) -> bool {
    vault.root().join(Path::new(file)).exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::resolve::tests::snap as base;
    use crate::model::Task;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 14).unwrap()
    }

    fn snap() -> Snapshot {
        let mut s = base();
        let task = |id: &str, title: &str, loc: TaskLocation, notes: &str| Task {
            id: Some(id.into()),
            title: title.into(),
            status: TaskStatus::Todo,
            priority: Priority::Normal,
            due: None,
            done: None,
            links: vec![],
            notes: notes.into(),
            location: loc,
            line: 1,
        };
        s.tasks = vec![
            task(
                "a1b2c3",
                "Umowa z dostawcą",
                TaskLocation::Inbox,
                "Wersja 2.",
            ),
            task("c3d4e5", "Odpis z sądu", TaskLocation::Inbox, ""),
            task(
                "d4e5f6",
                "Podpiąć karty",
                TaskLocation::Topic {
                    project: "whenubuy".into(),
                    topic: "checkout".into(),
                },
                "Czeka na: ^c3d4e5",
            ),
        ];
        s
    }

    fn new_id(s: &Step) -> String {
        match &s.op {
            Op::CreateTask { id, .. } => id.clone(),
            o => panic!("{o:?}"),
        }
    }

    fn pv(text: &str) -> Preview {
        preview(&snap(), text, &Context::new(today()))
    }

    fn only(p: &Preview) -> &Step {
        let plan = p.plan.as_ref().unwrap_or_else(|| panic!("{p:#?}"));
        assert_eq!(plan.steps.len(), 1, "{p:#?}");
        &plan.steps[0]
    }

    #[test]
    fn every_example_of_the_plan() {
        // Zadzwonić do sądu @jutro !wysoki → a task in the inbox, saved on Enter.
        let p = pv("Zadzwonić do sądu @jutro !wysoki");
        assert_eq!(p.state, State::Capture);
        let s = only(&p);
        assert_eq!(s.file, "skrzynka.md");
        assert_eq!(
            s.lines,
            [format!(
                "- [ ] Zadzwonić do sądu !wysoki @2026-11-15 ^{}",
                new_id(s)
            )]
        );
        assert!(
            p.reading.contains("Skrzynka") && p.reading.contains("jutro (2026-11-15)"),
            "{}",
            p.reading
        );

        // Podpiąć karty > whenubuy/checkout → a task in a topic, confirmed.
        let p = pv("Podpiąć nowe karty > whenubuy/checkout");
        assert_eq!(p.state, State::Confirm);
        assert_eq!(only(&p).file, "whenubuy/tematy/checkout.md");

        // Sprawdzić fakturę > whenubuy → project level.
        let p = pv("Sprawdzić fakturę > whenubuy");
        assert_eq!(only(&p).file, "whenubuy/projekt.md");

        // [/] Dokończyć opis > mapa-mysli → doing, topic by its unique slug.
        let p = pv("[/] Dokończyć opis > mapa-mysli");
        let s = only(&p);
        assert_eq!(s.file, "montybrain/tematy/mapa-mysli.md");
        assert_eq!(s.lines[0], format!("- [/] Dokończyć opis ^{}", new_id(s)));

        // /temat Płatności mobilne > whenubuy → a new topic.
        let p = pv("/temat Płatności mobilne > whenubuy");
        let s = only(&p);
        assert_eq!(s.file, "whenubuy/tematy/platnosci-mobilne.md");
        assert_eq!(s.lines[0], "# Płatności mobilne");

        // /temat ^a1b2c3 > firma → the entry becomes its task.
        let p = pv("/temat ^a1b2c3 > firma");
        let s = only(&p);
        assert_eq!(s.file, "firma/tematy/umowa-z-dostawca.md");
        match &s.op {
            Op::CreateTopic { title, tasks, .. } => {
                assert_eq!(title, "Umowa z dostawcą");
                assert_eq!(tasks[0].id, "a1b2c3");
            }
            o => panic!("{o:?}"),
        }
        assert!(s.lines.contains(&"  Wersja 2.".to_string()));

        // ^a1b2c3 > whenubuy/checkout → move.
        let p = pv("^a1b2c3 > whenubuy/checkout");
        match &only(&p).op {
            Op::EditTask {
                from,
                to,
                before,
                after,
                ..
            } => {
                assert_eq!(*from, TaskLocation::Inbox);
                assert_eq!(to.topic(), Some("checkout"));
                assert_eq!(before, after);
            }
            o => panic!("{o:?}"),
        }

        // ^a1b2c3 [x] → status (✓ today).
        let p = pv("^a1b2c3 [x]");
        let s = only(&p);
        assert_eq!(s.lines, ["- [x] Umowa z dostawcą ✓2026-11-14 ^a1b2c3"]);
        assert_eq!(s.before, ["- [ ] Umowa z dostawcą ^a1b2c3"]);

        // ^a1b2c3 !wysoki @2026-11-20 → priority and due.
        let p = pv("^a1b2c3 !wysoki @2026-11-20");
        assert_eq!(
            only(&p).lines,
            ["- [ ] Umowa z dostawcą !wysoki @2026-11-20 ^a1b2c3"]
        );

        // ^a1b2c3 /czeka ^c3d4e5 → one "Czeka na:" line.
        let p = pv("^a1b2c3 /czeka ^c3d4e5");
        let s = only(&p);
        assert_eq!(s.lines, ["  Czeka na: ^c3d4e5"]);
        assert!(matches!(&s.op, Op::AppendLine { already: false, .. }));

        // [[whenubuy/checkout]] /czeka [[firma/krs]] → a line in the summary.
        let p = pv("[[whenubuy/checkout]] /czeka [[firma/krs]]");
        let s = only(&p);
        assert_eq!(s.file, "whenubuy/tematy/checkout.md");
        assert_eq!(s.lines, ["Czeka na: [[firma/krs]]"]);

        // ^a1b2c3 /powiaz [[whenubuy/platnosci]] → "Powiązane:".
        let p = pv("^a1b2c3 /powiaz [[whenubuy/platnosci]]");
        assert_eq!(only(&p).lines, ["  Powiązane: [[whenubuy/platnosci]]"]);

        // ?rozpisz ^a1b2c3 and ?dopasuj → agent, nothing to write.
        let p = pv("?rozpisz ^a1b2c3");
        assert_eq!(p.state, State::Agent);
        assert!(p.plan.is_none());
        assert_eq!(p.agent.as_ref().unwrap().ids, ["a1b2c3"]);
        assert_eq!(pv("?dopasuj").state, State::Agent);
    }

    #[test]
    fn collisions_unknowns_and_typos() {
        let p = pv("Coś > checkout");
        assert_eq!(p.state, State::Invalid);
        assert!(p.plan.is_none());
        let ins: Vec<_> = p.choices.iter().map(|c| c.choice.insert.as_str()).collect();
        assert_eq!(ins, ["firma/checkout", "whenubuy/checkout"]);
        assert_eq!(p.choices[0].span, Span { from: 6, to: 14 });

        let p = pv("Coś > nieznane");
        assert_eq!(p.state, State::Invalid);

        let p = pv("Coś > whenbuy");
        assert_eq!(p.state, State::Confirm);
        assert!(p.typo);
        assert!(p.choices[0].choice.typo);
        assert_eq!(only(&p).file, "whenubuy/projekt.md");

        // A project wins over a topic named the same.
        assert_eq!(only(&pv("x > firma")).file, "firma/projekt.md");
    }

    #[test]
    fn title_text_that_looks_like_syntax_is_kept() {
        let p = pv("Napisać do @ani o a>b");
        assert_eq!(p.state, State::Capture);
        assert_eq!(
            only(&p).lines[0],
            format!("- [ ] Napisać do @ani o a>b ^{}", new_id(only(&p)))
        );
        // A trailing id-like word cannot be stored: refused, nothing written.
        let p = pv("Sprawdzić ^a1b2c3");
        assert!(matches!(p.state, State::Invalid | State::Confirm), "{p:?}");
    }

    #[test]
    fn edits_need_existing_tasks_and_something_to_do() {
        assert_eq!(pv("^zzzzzz [x]").state, State::Invalid);
        assert_eq!(pv("^a1b2c3").state, State::Invalid);
        assert_eq!(pv("^a1b2c3 /czeka ^a1b2c3").state, State::Invalid);
        // Already there: a step that writes nothing.
        let p = pv("^d4e5f6 /czeka ^c3d4e5");
        assert!(matches!(&only(&p).op, Op::AppendLine { already: true, .. }));
    }

    #[test]
    fn bulk_and_selection() {
        let mut ctx = Context::new(today());
        ctx.selection = vec!["a1b2c3".into(), "c3d4e5".into()];
        let p = preview(&snap(), "> firma", &ctx);
        assert_eq!(p.state, State::Confirm);
        assert_eq!(p.plan.as_ref().unwrap().steps.len(), 2);
        assert_eq!(p.reading, "Przeniesie 2 zadania do: Firma");
        let p = preview(&snap(), "/temat Biuro > firma", &ctx);
        match &only(&p).op {
            Op::CreateTopic { tasks, .. } => assert_eq!(tasks.len(), 2),
            o => panic!("{o:?}"),
        }
        let p = preview(&snap(), "/temat > firma", &ctx);
        assert_eq!(p.state, State::Invalid);
        let p = preview(&snap(), "[x]", &ctx);
        assert_eq!(p.plan.as_ref().unwrap().steps.len(), 2);
        // No selection, nothing written before ` > `.
        assert_eq!(pv("> firma").state, State::Invalid);
        assert_eq!(pv("/temat Coś").state, State::Invalid);
        assert_eq!(pv("/temat Coś > whenubuy/checkout").state, State::Invalid);
    }

    #[test]
    fn duplicate_hint_on_a_new_task() {
        let p = pv("Umowa z dostawcą");
        assert_eq!(p.similar[0].id, "a1b2c3");
    }

    #[test]
    fn a_preview_with_a_problem_reads_as_the_problem() {
        let p = pv("a > whenubuy > firma");
        assert_eq!(p.state, State::Invalid);
        assert!(p.plan.is_none());
        assert_eq!(p.reading, p.problems[0]);
        assert!(p.reading.contains("tylko jedno"), "{}", p.reading);
    }

    #[test]
    fn topic_titles_are_validated_in_the_preview() {
        for t in [
            "/temat ## Zadania > firma",
            "/temat --- > firma",
            "/temat - [ ] x > firma",
            "/temat Zły\u{7}tytuł > firma",
        ] {
            let p = pv(t);
            assert_eq!(p.state, State::Invalid, "{t}: {p:?}");
            assert!(p.plan.is_none());
        }
        assert_eq!(pv("/temat C# i .NET > firma").state, State::Confirm);
        // Brackets in a topic title: rejected (topics are referenced as [[…]]).
        let p = pv("/temat Testy [beta] > firma");
        assert_eq!(p.state, State::Invalid);
        assert!(p.reading.contains("[ or ]"), "{}", p.reading);
    }

    #[test]
    fn an_append_that_would_not_read_back_fails_in_the_preview() {
        let mut s = snap();
        s.tasks[0].notes = "Kod:\n```\nlet x = 1;".into();
        let p = preview(&s, "^a1b2c3 /czeka ^c3d4e5", &Context::new(today()));
        assert_eq!(p.state, State::Invalid, "{p:?}");
        assert!(p.reading.contains("otwartym blokiem kodu"), "{}", p.reading);
    }

    #[test]
    fn escaped_arrow_in_a_destination() {
        match crate::command::parse::parse("x > Plan \\> Q2").syntax {
            crate::command::parse::Syntax::NewTask { tail, .. } => {
                assert_eq!(tail.dest.unwrap().text, "Plan > Q2")
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn bulk_problems_and_choices_appear_once() {
        let mut ctx = Context::new(today());
        ctx.selection = vec!["a1b2c3".into(), "c3d4e5".into()];
        let p = preview(&snap(), "/czeka ^zzzzzz", &ctx);
        assert_eq!(
            p.problems.iter().filter(|x| x.contains("^zzzzzz")).count(),
            1,
            "{:?}",
            p.problems
        );
        let p = preview(&snap(), "/powiaz [[checkout]]", &ctx);
        let n = p.choices.len();
        assert_eq!(n, 2, "{:?}", p.choices);
    }

    #[test]
    fn plain_title_into_a_picked_place_saves_at_once() {
        let mut ctx = Context::new(today());
        ctx.default_location = TaskLocation::Topic {
            project: "whenubuy".into(),
            topic: "checkout".into(),
        };
        let p = preview(&snap(), "Zamówić terminal !wysoki", &ctx);
        assert_eq!(p.state, State::Capture);
        assert_eq!(only(&p).file, "whenubuy/tematy/checkout.md");
    }

    #[test]
    fn plan_tokens_follow_the_plan() {
        let a = pv("^a1b2c3 [x]").plan.unwrap();
        let b = pv("^a1b2c3 [x]").plan.unwrap();
        let c = pv("^a1b2c3 [/]").plan.unwrap();
        assert_eq!(plan_token(&a), plan_token(&b));
        assert_ne!(plan_token(&a), plan_token(&c));
    }

    fn typing(text: &str) -> Preview {
        let mut ctx = Context::new(today());
        ctx.caret = Some(text.encode_utf16().count());
        preview(&snap(), text, &ctx)
    }

    #[test]
    fn plain_words_name_the_reference() {
        // One match: accepted.
        let p = pv("^a1b2c3 /czeka odpis sad");
        assert_eq!(only(&p).lines, ["  Czeka na: ^c3d4e5"], "{p:#?}");
        // Topics too, case and Polish characters ignored.
        let p = pv("^a1b2c3 /powiaz PŁATNOŚCI");
        assert_eq!(only(&p).lines, ["  Powiązane: [[whenubuy/platnosci]]"]);
        // A typo: accepted, to be confirmed, flagged.
        let p = pv("^a1b2c3 /czeka platnosco");
        assert!(p.typo, "{p:#?}");
        assert_eq!(
            only(&p).lines,
            ["  Powiązane: [[whenubuy/platnosci]]".replace("Powiązane", "Czeka na")]
        );
        // Several: nothing written, the picker with kinds and places.
        let p = pv("^a1b2c3 /czeka checkout");
        assert_eq!(p.state, State::Invalid);
        assert!(p.plan.is_none());
        let ins: Vec<_> = p.choices.iter().map(|c| c.choice.insert.as_str()).collect();
        assert_eq!(
            ins,
            ["[[whenubuy/checkout]]", "[[firma/checkout]]"],
            "{p:#?}"
        );
        assert!(p.choices.iter().all(|c| c.choice.kind == "topic"));
        assert_eq!(p.choices[0].span, Span { from: 15, to: 23 });
        // The typed word is marked in each label.
        assert_eq!(p.choices[0].marks, [Span { from: 0, to: 8 }]);
        // Nothing: refused.
        let p = pv("^a1b2c3 /czeka zupelnie nic takiego");
        assert_eq!(p.state, State::Invalid);
        // `^id` and `[[…]]` still work next to words.
        let p = pv("^a1b2c3 /czeka ^c3d4e5 [[firma/krs]]");
        assert_eq!(p.plan.unwrap().steps.len(), 2);
    }

    #[test]
    fn plain_words_name_the_subject() {
        // An existing task by its title.
        let p = pv("Umowa /czeka odpis sadu");
        let s = only(&p);
        assert!(
            matches!(&s.op, Op::AppendLine { target: LineTarget::Task { id }, .. } if id == "a1b2c3"),
            "{p:#?}"
        );
        // An existing topic by its name; the subject is not its own reference.
        let p = pv("Płatności /powiaz checkout whenubuy");
        let s = only(&p);
        assert_eq!(s.file, "whenubuy/tematy/platnosci.md");
        assert_eq!(s.lines, ["Powiązane: [[whenubuy/checkout]]"]);
        // Several: the picker for the subject.
        let p = pv("checkout /czeka odpis sadu");
        assert_eq!(p.state, State::Invalid);
        assert_eq!(p.choices.len(), 2);
        assert_eq!(p.choices[0].span, Span { from: 0, to: 8 });
        // Nothing found: a new task, said so.
        let p = pv("Zadzwonić do Ani /czeka odpis sadu");
        assert!(
            matches!(&only(&p).op, Op::CreateTask { task, .. } if task.title == "Zadzwonić do Ani")
        );
        assert!(
            p.warnings.iter().any(|w| w.contains("nowe zadanie")),
            "{p:?}"
        );
        // `[ ]` first: always a new task.
        let p = pv("[ ] Umowa z dostawcą /powiaz ^a1b2c3");
        assert!(matches!(&only(&p).op, Op::CreateTask { .. }), "{p:#?}");
        // With ` > `: a new task, as before.
        let p = pv("Umowa > firma /czeka ^c3d4e5");
        assert!(matches!(&only(&p).op, Op::CreateTask { .. }));
    }

    #[test]
    fn the_focused_entry_is_the_subject_when_nothing_is_selected() {
        let mut ctx = Context::new(today());
        ctx.focus = Some("a1b2c3".into());
        let p = preview(&snap(), "/czeka odpis sadu", &ctx);
        assert!(
            matches!(&only(&p).op, Op::AppendLine { target: LineTarget::Task { id }, .. } if id == "a1b2c3")
        );
        // A selection wins over the focus.
        ctx.selection = vec!["c3d4e5".into()];
        let p = preview(&snap(), "[x]", &ctx);
        assert!(matches!(&only(&p).op, Op::EditTask { id, .. } if id == "c3d4e5"));
    }

    #[test]
    fn half_typed_words_read_as_typing_not_as_errors() {
        for t in [
            "/",
            "/cz",
            "^a1b2c3 /cz",
            "x > ",
            "x >",
            "^",
            "Kupić ^a1",
            "^a1b2c3 /czeka ",
            "^a1b2c3 /czeka",
            "x [[kr",
            "x !wy",
            "x @ju",
            "x @2026-1",
            "/temat ",
            "[",
            "^a1b2c3 /czeka check",
            "x > fir",
            "/temat Płatności",
        ] {
            let p = typing(t);
            assert_eq!(p.state, State::Typing, "{t}: {p:#?}");
            assert!(p.plan.is_none());
            assert!(!p.reading.is_empty());
            assert!(!p.problems.is_empty(), "{t}: Enter needs a reason");
        }
        // The same texts without a caret (MCP, apply) are plain problems.
        assert_eq!(pv("x > ").state, State::Invalid);
        assert_eq!(pv("^a1b2c3 /czeka").state, State::Invalid);
        // Finished words are not typing.
        assert_eq!(typing("Kupić mleko").state, State::Capture);
        assert_eq!(typing("x > firma").state, State::Confirm);
        assert_eq!(typing("x @jutro").state, State::Capture);
        assert_eq!(typing("Fix @mention").state, State::Capture);
        // The choices of a name being typed stay (the picker).
        assert_eq!(typing("x > check").choices.len(), 2);
        assert!(!typing("/temat Płatności").choices.is_empty());
    }

    #[test]
    fn plurals() {
        assert_eq!(tasks_word(1), "1 zadanie");
        assert_eq!(tasks_word(3), "3 zadania");
        assert_eq!(tasks_word(5), "5 zadań");
        assert_eq!(tasks_word(12), "12 zadań");
        assert_eq!(tasks_word(22), "22 zadania");
    }
}
