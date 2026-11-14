//! Executing a previewed plan, and taking it back.
//!
//! [`run`] re-reads the vault, builds the plan again from the same text and
//! refuses when it differs from the one the user confirmed (the data changed
//! in between). Then each step goes through the existing core mutations, each
//! one guarded by what the plan saw (status, priority, due date, place), so a
//! change made in the meantime is never overwritten.
//!
//! What an undo takes back (each step only while it is still exactly as
//! written; anything changed since is left alone and reported):
//! * a created task is deleted;
//! * an edit puts back the status, priority, due date and place (a task moved
//!   back goes to the end of its old file's tasks; a task that was done and is
//!   made done again gets today's `✓` date);
//! * a created topic file is removed;
//! * a topic made from entries is removed and its tasks go back to where they
//!   were;
//! * an appended connection line is removed.
//!
//! `zaktualizowano` is put back for single-file steps (a created task, an edit
//! without a move, an appended line) while the file still has the value the
//! step set; moves and topics leave it as the undo sets it.

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::model::{NewTopic, Stamp, Task, TaskLocation, TaskPatch};
use crate::vault::{LineWritten, Vault};

use super::plan::{Context, Fields, LineTarget, Op, Plan, Preview, State, TaskBasis, preview};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UndoStep {
    CreatedTask {
        task: Task,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stamp: Option<Stamp>,
    },
    EditedTask {
        id: String,
        title: String,
        from: TaskLocation,
        to: TaskLocation,
        before: Fields,
        after: Fields,
        /// The task as the edit left it: the undo runs only while it is still
        /// exactly this (title, links and notes included).
        written: Task,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stamp: Option<Stamp>,
    },
    CreatedTopic {
        project: String,
        slug: String,
        title: String,
        contents: String,
    },
    TopicFromTasks {
        project: String,
        slug: String,
        title: String,
        contents: String,
        origins: Vec<(String, TaskLocation)>,
    },
    AppendedLine {
        target: LineTarget,
        title: String,
        line: String,
        /// The task's notes / the topic's summary as the append left them.
        expected: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stamp: Option<Stamp>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepResult {
    pub summary: String,
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applied {
    pub steps: Vec<StepResult>,
    /// The exact lines written, with their files (the echo).
    pub echo: Vec<LineWritten>,
    /// What [`undo`] needs, in the order written.
    pub undo: Vec<UndoStep>,
    /// The first failure, in Polish (later steps were not run).
    pub error: Option<String>,
    /// Summaries of the steps not run after a failure.
    #[serde(default)]
    pub remaining: Vec<String>,
    /// After a failure the steps already written were taken back: nothing of
    /// this plan is in the vault.
    #[serde(default)]
    pub rolled_back: bool,
    /// After a failure, the steps that could not be taken back (still written);
    /// `undo` then holds exactly what is left to take back.
    #[serde(default)]
    pub left_written: Vec<String>,
    /// The step summary of each `undo` entry (for the rollback report).
    #[serde(skip)]
    undo_for: Vec<String>,
}

/// A core error as one Polish sentence for the user (the technical detail in
/// parentheses where there is one).
pub fn polish_error(e: &Error) -> String {
    match e {
        Error::TaskNotFound(id) => format!("Nie ma już zadania ^{id}."),
        Error::TopicNotFound { project, topic } => format!("Nie ma już tematu {project}/{topic}."),
        Error::ProjectNotFound(p) => format!("Nie ma projektu {p}."),
        Error::VaultNotFound(p) => format!("Nie ma folderu danych {}.", p.display()),
        Error::NoVaultDir => "Nie wiadomo, gdzie jest folder danych.".into(),
        Error::Io { path, source } => {
            format!(
                "Błąd zapisu pliku {} ({source}); część zmian mogła się zapisać.",
                path.display()
            )
        }
        Error::Unparseable { path, message } => {
            format!(
                "Pliku {} nie da się bezpiecznie edytować ({message}).",
                path.display()
            )
        }
        Error::InvalidInput(m) => {
            let detail = m.as_str();
            if detail.contains("a task with id") && detail.contains("already exists") {
                "To zadanie już istnieje: ten plan został już zapisany.".into()
            } else if detail.contains("a topic file") && detail.contains("already exists") {
                "Taki plik tematu już istnieje (ktoś go założył po podglądzie).".into()
            } else if detail.contains("would not read it back as a connection") {
                "Linii powiązania nie da się dopisać tak, żeby była czytana jako powiązanie (otwarty blok kodu?).".into()
            } else if detail.contains("cannot be stored unambiguously") {
                format!("Tego tekstu nie da się zapisać jednoznacznie ({detail}).")
            } else {
                format!("Zapis odrzucony, żeby nie zepsuć pliku ({detail}).")
            }
        }
    }
}

/// Why [`run`] wrote nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Refusal {
    /// The text cannot be written as it is (problems, choices, an agent request).
    Invalid {
        preview: Box<Preview>,
    },
    /// The plan rests on a typo correction and the caller asked for exact names.
    Typo {
        preview: Box<Preview>,
    },
    /// The data changed since the preview: here is the new one.
    Stale {
        preview: Box<Preview>,
    },
    Failed {
        message: String,
    },
}

impl Refusal {
    /// One line for the user / an agent.
    pub fn message(&self) -> String {
        match self {
            Refusal::Invalid { preview } => {
                if preview.state == State::Agent {
                    "a request to the agent (`?…`) cannot be executed here".into()
                } else if !preview.problems.is_empty() {
                    preview.problems.join(" ")
                } else {
                    "there is nothing to write".into()
                }
            }
            Refusal::Typo { preview } => {
                format!("{} Write the exact slug.", preview.warnings.join(" "))
            }
            Refusal::Stale { .. } => {
                "Dane zmieniły się od podglądu: sprawdź plan jeszcze raz.".into()
            }
            Refusal::Failed { message } => message.clone(),
        }
    }
}

/// What `run` wrote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub preview: Preview,
    pub applied: Applied,
}

/// Preview `text` against the vault as it is now and write it when it is
/// writable and, if `expected` is given, still the same plan. `exact`: refuse
/// a plan that rests on a typo correction (agents must name things exactly).
pub fn run(
    vault: &Vault,
    text: &str,
    ctx: &Context,
    expected: Option<&Plan>,
    exact: bool,
) -> Result<Outcome, Refusal> {
    let snap = vault.load().map_err(|e| Refusal::Failed {
        message: e.to_string(),
    })?;
    let ctx = Context {
        vault: Some(vault),
        ..ctx.clone()
    };
    let p = preview(&snap, text, &ctx);
    let Some(plan) = p.plan.clone() else {
        return Err(Refusal::Invalid {
            preview: Box::new(p),
        });
    };
    if exact && p.typo {
        return Err(Refusal::Typo {
            preview: Box::new(p),
        });
    }
    if let Some(e) = expected
        && *e != plan
    {
        return Err(Refusal::Stale {
            preview: Box::new(p),
        });
    }
    let applied = apply(vault, &plan);
    Ok(Outcome {
        preview: p,
        applied,
    })
}

fn file_line(vault: &Vault, loc: &TaskLocation, line: usize) -> LineWritten {
    let file = loc.rel_path();
    let text = std::fs::read_to_string(vault.root().join(&file))
        .ok()
        .and_then(|t| t.lines().nth(line.saturating_sub(1)).map(str::to_string))
        .unwrap_or_default();
    LineWritten { file, line, text }
}

fn stamp(before: Option<String>, after: Option<String>) -> Option<Stamp> {
    match (before, after) {
        (Some(before), Some(written)) if before != written => Some(Stamp { written, before }),
        _ => None,
    }
}

fn changed_err(what: &str) -> String {
    format!("{what} zmieniło się od podglądu; pominięte.")
}

/// Run every step of `plan` in order, all or nothing: at the first failure the
/// steps already written are taken back (the guarded undo, newest first). Only
/// when that rollback cannot complete does something stay written; it is
/// listed in [`Applied::left_written`] and `undo` holds what is left.
pub fn apply(vault: &Vault, plan: &Plan) -> Applied {
    apply_with(vault, plan, &|_| {})
}

/// [`apply`] with a hook after each written step (tests simulate other
/// writers between the steps).
fn apply_with(vault: &Vault, plan: &Plan, after_step: &dyn Fn(usize)) -> Applied {
    let mut out = apply_steps(vault, plan, after_step);
    if out.error.is_some() {
        let written = out.undo_for.clone();
        if written.is_empty() {
            out.rolled_back = true;
            out.undo.clear();
            out.echo.clear();
            return out;
        }
        let outcomes = undo(vault, &out.undo);
        // `undo` answers newest first; pair with the steps in that order.
        let mut left_steps = Vec::new();
        let mut left = Vec::new();
        for ((u, o), summary) in out
            .undo
            .iter()
            .rev()
            .zip(&outcomes)
            .zip(written.iter().rev())
        {
            if o.status != UndoStatus::Undone {
                left_steps.push(u.clone());
                left.push(format!("{summary} ({})", o.message));
            }
        }
        left_steps.reverse();
        left.reverse();
        out.undo_for = Vec::new();
        out.rolled_back = left.is_empty();
        out.left_written = left;
        out.undo = left_steps;
        if out.rolled_back {
            out.echo.clear();
        }
    }
    out
}

/// The steps in order; stops at the first failure (no rollback).
fn apply_steps(vault: &Vault, plan: &Plan, after_step: &dyn Fn(usize)) -> Applied {
    let mut out = Applied::default();
    for (n, step) in plan.steps.iter().enumerate() {
        let undo_before = out.undo.len();
        let res: Result<String, String> = (|| match &step.op {
            Op::CreateTask { location, task, id } => {
                let before = vault.stamp_of(location).ok().flatten();
                let t = vault
                    .create_task_with_id(location, task.clone(), id)
                    .map_err(|e| polish_error(&e))?;
                let after = vault.stamp_of(location).ok().flatten();
                out.echo.push(file_line(vault, location, t.line));
                for k in 1..=task.notes.lines().count() {
                    out.echo
                        .push(file_line(vault, location, t.line + t.links.len() + k));
                }
                let msg = format!("Dodano „{}”.", t.title);
                out.undo.push(UndoStep::CreatedTask {
                    task: t,
                    stamp: stamp(before, after),
                });
                Ok(msg)
            }
            Op::EditTask {
                id,
                title,
                from,
                to,
                before,
                after,
                basis,
            } => {
                let patch = TaskPatch {
                    status: (after.status != before.status).then_some(after.status),
                    priority: (after.priority != before.priority).then_some(after.priority),
                    due: (after.due != before.due).then_some(after.due),
                    ..Default::default()
                };
                let s_before = (from == to)
                    .then(|| vault.stamp_of(from).ok().flatten())
                    .flatten();
                let guard = |t: &Task| {
                    t.location == *from && Fields::of(t) == *before && TaskBasis::of(t) == *basis
                };
                let t = vault
                    .update_and_move_task_if(id, guard, patch, to)
                    .map_err(|e| match e {
                        Error::TaskNotFound(_) => changed_err("Zadanie"),
                        e => polish_error(&e),
                    })?
                    .ok_or_else(|| changed_err("Zadanie"))?;
                let s_after = (from == to)
                    .then(|| vault.stamp_of(to).ok().flatten())
                    .flatten();
                out.echo.push(file_line(vault, to, t.line));
                out.undo.push(UndoStep::EditedTask {
                    id: id.clone(),
                    title: title.clone(),
                    from: from.clone(),
                    to: to.clone(),
                    before: *before,
                    after: Fields::of(&t),
                    written: t.clone(),
                    stamp: stamp(s_before, s_after),
                });
                Ok(format!("Zmieniono „{title}”."))
            }
            Op::CreateTopic {
                project,
                title,
                slug,
                summary,
                tasks,
            } => {
                let new = NewTopic {
                    title: title.clone(),
                    summary: summary.clone(),
                    ..Default::default()
                };
                if tasks.is_empty() {
                    // Exactly the previewed file; refused if it appeared meanwhile.
                    let c = vault
                        .create_topic_exact(project, slug, new)
                        .map_err(|e| polish_error(&e))?;
                    out.echo.push(LineWritten {
                        file: c.topic.path.clone(),
                        line: c
                            .contents
                            .lines()
                            .position(|l| l.starts_with("# "))
                            .map_or(1, |i| i + 1),
                        text: format!("# {}", c.topic.title),
                    });
                    out.undo.push(UndoStep::CreatedTopic {
                        project: project.clone(),
                        slug: c.topic.slug.clone(),
                        title: c.topic.title.clone(),
                        contents: c.contents,
                    });
                    Ok(format!("Dodano temat „{}”.", c.topic.title))
                } else {
                    // Every entry must still be where and what the plan saw:
                    // checked inside the lock that moves them.
                    let ids: Vec<String> = tasks.iter().map(|e| e.id.clone()).collect();
                    let guard = |t: &Task| {
                        tasks.iter().any(|e| {
                            Some(&e.id) == t.id.as_ref()
                                && e.from == t.location
                                && e.fields == Fields::of(t)
                        })
                    };
                    let made = vault
                        .create_topic_from_tasks_if(project, new, &ids, Some(slug), guard)
                        .map_err(|e| match e {
                            Error::TaskNotFound(id) => changed_err(&format!("Zadanie ^{id}")),
                            e => polish_error(&e),
                        })?
                        .ok_or_else(|| changed_err("Jeden z wpisów"))?;
                    let loc = TaskLocation::Topic {
                        project: project.clone(),
                        topic: made.topic.slug.clone(),
                    };
                    out.echo.push(LineWritten {
                        file: made.topic.path.clone(),
                        line: made
                            .contents
                            .lines()
                            .position(|l| l.starts_with("# "))
                            .map_or(1, |i| i + 1),
                        text: format!("# {}", made.topic.title),
                    });
                    for t in &made.tasks {
                        out.echo.push(file_line(vault, &loc, t.line));
                    }
                    let msg = format!(
                        "Dodano temat „{}” z {} zadaniami.",
                        made.topic.title,
                        made.tasks.len()
                    );
                    out.undo.push(UndoStep::TopicFromTasks {
                        project: project.clone(),
                        slug: made.topic.slug.clone(),
                        title: made.topic.title.clone(),
                        contents: made.contents,
                        origins: made.origins,
                    });
                    Ok(msg)
                }
            }
            Op::AppendLine {
                target,
                title,
                line,
                already,
                basis,
            } => {
                if *already {
                    return Ok(format!("„{title}”: ta linia już jest."));
                }
                match target {
                    LineTarget::Task { id } => {
                        let loc = vault
                            .load()
                            .ok()
                            .and_then(|s| s.task(id).map(|t| t.location.clone()));
                        let before = loc.as_ref().and_then(|l| vault.stamp_of(l).ok().flatten());
                        let (t, w) = vault
                            .append_task_note_line_if(id, line, |t| t.notes == *basis)
                            .map_err(|e| match e {
                                Error::TaskNotFound(_) => changed_err("Zadanie"),
                                e => polish_error(&e),
                            })?
                            .ok_or_else(|| changed_err("Zadanie"))?;
                        let after = vault.stamp_of(&t.location).ok().flatten();
                        out.echo.push(w);
                        out.undo.push(UndoStep::AppendedLine {
                            target: target.clone(),
                            title: title.clone(),
                            line: line.clone(),
                            expected: t.notes.clone(),
                            stamp: stamp(before, after),
                        });
                    }
                    LineTarget::Topic { project, slug } => {
                        let loc = TaskLocation::Topic {
                            project: project.clone(),
                            topic: slug.clone(),
                        };
                        let before = vault.stamp_of(&loc).ok().flatten();
                        let (topic, w) = vault
                            .append_summary_line_if(project, slug, line, |t| t.summary == *basis)
                            .map_err(|e| match e {
                                Error::TopicNotFound { .. } => changed_err("Temat"),
                                e => polish_error(&e),
                            })?
                            .ok_or_else(|| changed_err("Opis tematu"))?;
                        let after = vault.stamp_of(&loc).ok().flatten();
                        out.echo.push(w);
                        out.undo.push(UndoStep::AppendedLine {
                            target: target.clone(),
                            title: title.clone(),
                            line: line.clone(),
                            expected: topic.summary,
                            stamp: stamp(before, after),
                        });
                    }
                }
                Ok(format!("„{title}”: dopisano „{line}”."))
            }
        })();
        for _ in undo_before..out.undo.len() {
            out.undo_for.push(step.summary.clone());
        }
        match res {
            Ok(message) => {
                out.steps.push(StepResult {
                    summary: step.summary.clone(),
                    ok: true,
                    message,
                });
                after_step(n);
            }
            Err(message) => {
                out.steps.push(StepResult {
                    summary: step.summary.clone(),
                    ok: false,
                    message: message.clone(),
                });
                out.error = Some(message);
                out.remaining = plan.steps[n + 1..]
                    .iter()
                    .map(|s| s.summary.clone())
                    .collect();
                break;
            }
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoStatus {
    Undone,
    /// Changed in the meantime: left alone.
    Changed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoOutcome {
    pub status: UndoStatus,
    pub message: String,
}

/// Everything but the line number.
fn same_task(a: &Task, b: &Task) -> bool {
    a.id == b.id
        && a.title == b.title
        && a.status == b.status
        && a.priority == b.priority
        && a.due == b.due
        && a.done == b.done
        && a.links == b.links
        && a.notes == b.notes
        && a.location == b.location
}

fn task_as_created(t: &Task, created: &Task) -> bool {
    t.title == created.title
        && t.status == created.status
        && t.priority == created.priority
        && t.due == created.due
        && t.links == created.links
        && t.notes == created.notes
        && t.location == created.location
}

/// Take back `steps` (newest first), each only while it is unchanged.
pub fn undo(vault: &Vault, steps: &[UndoStep]) -> Vec<UndoOutcome> {
    let done = |m: String| UndoOutcome {
        status: UndoStatus::Undone,
        message: m,
    };
    let changed = |what: &str| UndoOutcome {
        status: UndoStatus::Changed,
        message: format!("{what} zmieniło się w międzyczasie; zostawione bez zmian."),
    };
    let failed = |e: Error| UndoOutcome {
        status: UndoStatus::Failed,
        message: polish_error(&e),
    };
    steps
        .iter()
        .rev()
        .map(|step| match step {
            UndoStep::CreatedTask { task, stamp } => {
                let id = task.id.clone().unwrap_or_default();
                match vault
                    .restoring(stamp.clone())
                    .delete_task_if(&id, |t| task_as_created(t, task))
                {
                    Ok(Some(_)) => done(format!("Usunięto dodane „{}”.", task.title)),
                    Ok(None) | Err(Error::TaskNotFound(_)) => {
                        changed(&format!("Zadanie „{}”", task.title))
                    }
                    Err(e) => failed(e),
                }
            }
            UndoStep::EditedTask {
                id,
                title,
                from,
                to,
                before,
                after,
                written,
                stamp,
            } => {
                let patch = TaskPatch {
                    status: (after.status != before.status).then_some(before.status),
                    priority: (after.priority != before.priority).then_some(before.priority),
                    due: (after.due != before.due).then_some(before.due),
                    ..Default::default()
                };
                let v = if from == to {
                    vault.restoring(stamp.clone())
                } else {
                    vault.clone()
                };
                match v.update_and_move_task_if(
                    id,
                    |t| t.location == *to && Fields::of(t) == *after && same_task(t, written),
                    patch,
                    from,
                ) {
                    Ok(Some(_)) => done(format!("Przywrócono „{title}”.")),
                    Ok(None) | Err(Error::TaskNotFound(_)) => {
                        changed(&format!("Zadanie „{title}”"))
                    }
                    Err(e) => failed(e),
                }
            }
            UndoStep::CreatedTopic {
                project,
                slug,
                title,
                contents,
            } => match vault.delete_topic_if(project, slug, |c| c == contents) {
                Ok(Some(_)) => done(format!("Usunięto dodany temat „{title}”.")),
                Ok(None) | Err(Error::TopicNotFound { .. }) => changed(&format!("Temat „{title}”")),
                Err(e) => failed(e),
            },
            UndoStep::TopicFromTasks {
                project,
                slug,
                title,
                contents,
                origins,
            } => match vault.dissolve_topic_if(project, slug, contents, origins) {
                Ok(Some(back)) => done(format!(
                    "Usunięto temat „{title}”; {} wróciło na miejsce.",
                    super::plan::tasks_word(back.len())
                )),
                Ok(None) | Err(Error::TopicNotFound { .. }) => changed(&format!("Temat „{title}”")),
                Err(e) => failed(e),
            },
            UndoStep::AppendedLine {
                target,
                title,
                line,
                expected,
                stamp,
            } => {
                let v = vault.restoring(stamp.clone());
                let r = match target {
                    LineTarget::Task { id } => {
                        v.remove_task_note_line_if(id, line, |t| t.notes == *expected)
                    }
                    LineTarget::Topic { project, slug } => {
                        v.remove_summary_line_if(project, slug, line, |t| t.summary == *expected)
                    }
                };
                match r {
                    Ok(true) => done(format!("Usunięto linię „{line}” z „{title}”.")),
                    Ok(false) | Err(Error::TaskNotFound(_)) | Err(Error::TopicNotFound { .. }) => {
                        changed(&format!("„{title}”"))
                    }
                    Err(e) => failed(e),
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NewProject, NewTask, NewTopic as NT, TaskStatus};
    use chrono::NaiveDate;
    use std::fs;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 14).unwrap()
    }

    fn vault() -> (tempfile::TempDir, Vault, String) {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::init(dir.path())
            .unwrap()
            .with_clock(|| today().and_hms_opt(10, 0, 0).unwrap());
        for name in ["Firma", "WhenUBuy"] {
            v.create_project(NewProject {
                name: name.into(),
                ..Default::default()
            })
            .unwrap();
        }
        v.create_topic(
            "whenubuy",
            NT {
                title: "Checkout".into(),
                ..Default::default()
            },
        )
        .unwrap();
        v.create_topic(
            "firma",
            NT {
                title: "KRS".into(),
                summary: "Sąd.".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let id = v
            .create_task(
                &TaskLocation::Inbox,
                NewTask {
                    title: "Umowa".into(),
                    notes: "Wersja 2.".into(),
                    ..Default::default()
                },
            )
            .unwrap()
            .id
            .unwrap();
        (dir, v, id)
    }

    fn ctx() -> Context<'static> {
        Context::new(today())
    }

    fn files(dir: &std::path::Path) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                {
                    continue;
                }
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push((
                        p.strip_prefix(dir).unwrap().display().to_string(),
                        fs::read_to_string(&p).unwrap(),
                    ));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn plain_capture_writes_echoes_and_undoes() {
        let (dir, v, _) = vault();
        let before = files(dir.path());
        let o = run(&v, "Zadzwonić do sądu @jutro !wysoki", &ctx(), None, true).unwrap();
        assert_eq!(o.preview.state, State::Capture);
        assert!(o.applied.error.is_none());
        let echo = &o.applied.echo[0];
        assert_eq!(echo.file, std::path::PathBuf::from("skrzynka.md"));
        assert!(
            echo.text
                .starts_with("- [ ] Zadzwonić do sądu !wysoki @2026-11-15 ^"),
            "{}",
            echo.text
        );
        let u = undo(&v, &o.applied.undo);
        assert_eq!(u[0].status, UndoStatus::Undone);
        assert_eq!(files(dir.path()), before);
    }

    #[test]
    fn a_stale_plan_is_refused() {
        let (_dir, v, id) = vault();
        let snap = v.load().unwrap();
        let p = preview(
            &snap,
            &format!("^{id} [x]"),
            &Context {
                vault: Some(&v),
                ..ctx()
            },
        );
        let plan = p.plan.unwrap();
        // Someone else changes the task in between.
        v.update_task(
            &id,
            TaskPatch {
                status: Some(TaskStatus::Doing),
                ..Default::default()
            },
        )
        .unwrap();
        match run(&v, &format!("^{id} [x]"), &ctx(), Some(&plan), false) {
            Err(Refusal::Stale { preview }) => assert!(preview.plan.is_some()),
            o => panic!("{o:?}"),
        }
        assert_eq!(
            v.load().unwrap().task(&id).unwrap().status,
            TaskStatus::Doing
        );
        // The same plan again (nothing changed) goes through.
        let snap = v.load().unwrap();
        let plan = preview(
            &snap,
            &format!("^{id} [x]"),
            &Context {
                vault: Some(&v),
                ..ctx()
            },
        )
        .plan
        .unwrap();
        let o = run(&v, &format!("^{id} [x]"), &ctx(), Some(&plan), false).unwrap();
        assert!(o.applied.error.is_none());
        assert_eq!(
            v.load().unwrap().task(&id).unwrap().status,
            TaskStatus::Done
        );
    }

    #[test]
    fn move_edit_lines_and_undo_round_trip() {
        let (dir, v, id) = vault();
        let before = files(dir.path());
        let o = run(
            &v,
            &format!("^{id} !wysoki @pt > whenubuy/checkout /czeka [[firma/krs]]"),
            &ctx(),
            None,
            true,
        )
        .unwrap();
        assert!(o.applied.error.is_none(), "{:?}", o.applied);
        let s = v.load().unwrap();
        let t = s.task(&id).unwrap();
        assert_eq!(t.location.topic(), Some("checkout"));
        assert_eq!(t.notes, "Wersja 2.\nCzeka na: [[firma/krs]]");
        assert_eq!(t.due.unwrap().to_string(), "2026-11-20");
        assert_eq!(o.applied.echo.len(), 2);
        assert_eq!(o.applied.echo[1].text, "  Czeka na: [[firma/krs]]");
        let u = undo(&v, &o.applied.undo);
        assert!(u.iter().all(|x| x.status == UndoStatus::Undone), "{u:?}");
        let s = v.load().unwrap();
        let t = s.task(&id).unwrap();
        assert_eq!(t.location, TaskLocation::Inbox);
        assert_eq!(t.notes, "Wersja 2.");
        // Same files apart from `zaktualizowano` of the topic the task visited.
        let after = files(dir.path());
        let strip = |v: Vec<(String, String)>| {
            v.into_iter()
                .map(|(p, t)| {
                    (
                        p,
                        t.lines()
                            .filter(|l| !l.starts_with("zaktualizowano"))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(strip(after), strip(before));
    }

    #[test]
    fn undo_leaves_what_changed() {
        let (_dir, v, id) = vault();
        let o = run(&v, &format!("^{id} [x]"), &ctx(), None, true).unwrap();
        v.update_task(
            &id,
            TaskPatch {
                status: Some(TaskStatus::Blocked),
                ..Default::default()
            },
        )
        .unwrap();
        let u = undo(&v, &o.applied.undo);
        assert_eq!(u[0].status, UndoStatus::Changed);
        assert_eq!(
            v.load().unwrap().task(&id).unwrap().status,
            TaskStatus::Blocked
        );
        // A created task that was edited stays.
        let o = run(&v, "Nowe", &ctx(), None, true).unwrap();
        let UndoStep::CreatedTask { task, .. } = &o.applied.undo[0] else {
            panic!()
        };
        let nid = task.id.clone().unwrap();
        v.update_task(
            &nid,
            TaskPatch {
                title: Some("Nowe!".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(undo(&v, &o.applied.undo)[0].status, UndoStatus::Changed);
        assert!(v.load().unwrap().task(&nid).is_some());
    }

    #[test]
    fn topics_from_entries_and_bulk_moves() {
        let (dir, v, id) = vault();
        let id2 = v
            .create_task(
                &TaskLocation::Inbox,
                NewTask {
                    title: "Aneks".into(),
                    ..Default::default()
                },
            )
            .unwrap()
            .id
            .unwrap();
        let before = files(dir.path());
        let c = Context {
            selection: vec![id.clone(), id2.clone()],
            ..ctx()
        };
        let o = run(&v, "/temat Biuro > firma", &c, None, true).unwrap();
        assert!(o.applied.error.is_none(), "{:?}", o.applied);
        let s = v.load().unwrap();
        assert_eq!(s.task(&id).unwrap().location.topic(), Some("biuro"));
        assert_eq!(s.task(&id).unwrap().notes, "Wersja 2.");
        assert_eq!(o.applied.echo[0].text, "# Biuro");
        let u = undo(&v, &o.applied.undo);
        assert_eq!(u[0].status, UndoStatus::Undone, "{u:?}");
        assert!(!dir.path().join("firma/tematy/biuro.md").exists());
        let _ = before;
        // Bulk move of the selection.
        let o = run(&v, "> whenubuy", &c, None, true).unwrap();
        assert_eq!(o.applied.steps.len(), 2);
        let s = v.load().unwrap();
        assert!(s.task(&id).unwrap().location.project() == Some("whenubuy"));
        assert!(s.task(&id2).unwrap().location.project() == Some("whenubuy"));
        let u = undo(&v, &o.applied.undo);
        assert!(u.iter().all(|x| x.status == UndoStatus::Undone));
        // A new topic, then its undo.
        let o = run(
            &v,
            "/temat Płatności > whenubuy /powiaz [[firma/krs]]",
            &ctx(),
            None,
            true,
        )
        .unwrap();
        let s = v.load().unwrap();
        assert_eq!(
            s.topic("whenubuy", "platnosci").unwrap().summary,
            "Powiązane: [[firma/krs]]"
        );
        assert_eq!(undo(&v, &o.applied.undo)[0].status, UndoStatus::Undone);
        assert!(v.load().unwrap().topic("whenubuy", "platnosci").is_none());
        // Topic summary line and its undo.
        let o = run(
            &v,
            "[[firma/krs]] /czeka [[whenubuy/checkout]]",
            &ctx(),
            None,
            true,
        )
        .unwrap();
        assert_eq!(
            v.load().unwrap().topic("firma", "krs").unwrap().summary,
            "Sąd.\nCzeka na: [[whenubuy/checkout]]"
        );
        assert_eq!(undo(&v, &o.applied.undo)[0].status, UndoStatus::Undone);
        assert_eq!(
            v.load().unwrap().topic("firma", "krs").unwrap().summary,
            "Sąd."
        );
    }

    #[test]
    fn undo_of_an_edit_refuses_after_any_later_change() {
        let (_dir, v, id) = vault();
        let o = run(&v, &format!("^{id} !wysoki"), &ctx(), None, true).unwrap();
        // Same status, priority and due, but another title: not ours to restore.
        v.update_task(
            &id,
            TaskPatch {
                title: Some("Umowa najmu".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let u = undo(&v, &o.applied.undo);
        assert_eq!(u[0].status, UndoStatus::Changed, "{u:?}");
        assert_eq!(
            v.load().unwrap().task(&id).unwrap().priority,
            crate::model::Priority::High
        );
        // Notes too.
        let o = run(&v, &format!("^{id} @jutro"), &ctx(), None, true).unwrap();
        v.update_task(
            &id,
            TaskPatch {
                notes: Some("Inna notatka.".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(undo(&v, &o.applied.undo)[0].status, UndoStatus::Changed);
    }

    #[test]
    fn undo_of_an_appended_line_refuses_after_the_text_changed() {
        let (_dir, v, id) = vault();
        let o = run(
            &v,
            &format!("^{id} /czeka [[firma/krs]]"),
            &ctx(),
            None,
            true,
        )
        .unwrap();
        v.update_task(
            &id,
            TaskPatch {
                notes: Some("Wersja 2.\nCzeka na: [[firma/krs]]\nDopisane ręcznie.".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let u = undo(&v, &o.applied.undo);
        assert_eq!(u[0].status, UndoStatus::Changed, "{u:?}");
        assert!(
            v.load()
                .unwrap()
                .task(&id)
                .unwrap()
                .notes
                .contains("Czeka na: [[firma/krs]]")
        );
        // A topic summary the same way.
        let o = run(
            &v,
            "[[firma/krs]] /powiaz [[whenubuy/checkout]]",
            &ctx(),
            None,
            true,
        )
        .unwrap();
        v.update_topic(
            "firma",
            "krs",
            crate::model::TopicPatch {
                summary: Some("Sąd.\nPowiązane: [[whenubuy/checkout]]\nI coś.".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(undo(&v, &o.applied.undo)[0].status, UndoStatus::Changed);
    }

    #[test]
    fn a_topic_from_entries_checks_its_entries_inside_the_lock() {
        let (dir, v, id) = vault();
        let c = Context {
            selection: vec![id.clone()],
            ..ctx()
        };
        let snap = v.load().unwrap();
        let plan = preview(
            &snap,
            "/temat Biuro > firma",
            &Context {
                vault: Some(&v),
                ..c.clone()
            },
        )
        .plan
        .unwrap();
        // The entry changes after the preview; applying that old plan directly.
        v.update_task(
            &id,
            TaskPatch {
                status: Some(TaskStatus::Doing),
                ..Default::default()
            },
        )
        .unwrap();
        let a = apply(&v, &plan);
        assert!(
            a.error
                .as_deref()
                .is_some_and(|e| e.contains("zmieniło się")),
            "{a:?}"
        );
        assert!(!dir.path().join("firma/tematy/biuro.md").exists());
        assert_eq!(
            v.load().unwrap().task(&id).unwrap().location,
            TaskLocation::Inbox
        );
    }

    #[test]
    fn a_failure_mid_way_reports_done_and_remaining_steps_in_polish() {
        let (_dir, v, id) = vault();
        let id2 = v
            .create_task(
                &TaskLocation::Inbox,
                NewTask {
                    title: "Aneks".into(),
                    ..Default::default()
                },
            )
            .unwrap()
            .id
            .unwrap();
        let c = Context {
            selection: vec![id.clone(), id2.clone()],
            ..ctx()
        };
        let snap = v.load().unwrap();
        let plan = preview(
            &snap,
            "> firma",
            &Context {
                vault: Some(&v),
                ..c.clone()
            },
        )
        .plan
        .unwrap();
        // The second entry disappears before its step runs.
        v.delete_task(&id2).unwrap();
        let a = apply(&v, &plan);
        assert_eq!(a.steps.len(), 2);
        assert!(a.steps[0].ok && !a.steps[1].ok);
        assert!(
            a.error
                .as_deref()
                .is_some_and(|e| e.contains("zmieniło się")),
            "{a:?}"
        );
        assert!(a.remaining.is_empty());
        // All or nothing: the first move was taken back.
        assert!(
            a.rolled_back && a.left_written.is_empty() && a.undo.is_empty() && a.echo.is_empty()
        );
        assert_eq!(
            v.load().unwrap().task(&id).unwrap().location,
            TaskLocation::Inbox
        );
        let e = polish_error(&Error::TaskNotFound("abc123".into()));
        assert_eq!(e, "Nie ma już zadania ^abc123.");
    }

    #[test]
    fn a_rollback_that_cannot_complete_says_what_stays_written() {
        let (_dir, v, id) = vault();
        let id2 = v
            .create_task(
                &TaskLocation::Inbox,
                NewTask {
                    title: "Aneks".into(),
                    ..Default::default()
                },
            )
            .unwrap()
            .id
            .unwrap();
        let c = Context {
            selection: vec![id.clone(), id2.clone()],
            ..ctx()
        };
        let snap = v.load().unwrap();
        let plan = preview(
            &snap,
            "> firma",
            &Context {
                vault: Some(&v),
                ..c
            },
        )
        .plan
        .unwrap();
        // After the first move someone edits that task (so its undo must not
        // touch it) and deletes the second one (so the second step fails).
        let hook = |n: usize| {
            if n == 0 {
                v.update_task(
                    &id,
                    TaskPatch {
                        title: Some("Umowa (zmieniona)".into()),
                        ..Default::default()
                    },
                )
                .unwrap();
                v.delete_task(&id2).unwrap();
            }
        };
        let a = apply_with(&v, &plan, &hook);
        assert!(!a.rolled_back, "{a:?}");
        assert_eq!(a.left_written.len(), 1, "{a:?}");
        assert!(a.left_written[0].contains("Umowa"), "{a:?}");
        assert_eq!(a.undo.len(), 1);
        let t = v.load().unwrap().task(&id).unwrap().clone();
        assert_eq!(t.location.project(), Some("firma"));
    }

    #[test]
    fn a_new_task_plan_writes_its_previewed_id_once() {
        let (_dir, v, _) = vault();
        let snap = v.load().unwrap();
        let p = preview(
            &snap,
            "Zadzwonić do sądu",
            &Context {
                vault: Some(&v),
                ..ctx()
            },
        );
        let plan = p.plan.unwrap();
        let Op::CreateTask { id, .. } = &plan.steps[0].op else {
            panic!()
        };
        let a = apply(&v, &plan);
        assert!(a.error.is_none(), "{a:?}");
        assert_eq!(
            v.load().unwrap().task(id).unwrap().title,
            "Zadzwonić do sądu"
        );
        // The same plan again: refused, nothing written.
        let n = v.load().unwrap().tasks.len();
        let again = apply(&v, &plan);
        assert!(
            again
                .error
                .as_deref()
                .is_some_and(|e| e.contains("już zapisany")),
            "{again:?}"
        );
        assert!(again.rolled_back);
        assert_eq!(v.load().unwrap().tasks.len(), n);
        // Through `run` with the old plan: stale (the next preview picks another id).
        assert!(matches!(
            run(&v, "Zadzwonić do sądu", &ctx(), Some(&plan), true),
            Err(Refusal::Stale { .. })
        ));
    }

    #[test]
    fn a_new_topic_is_written_at_exactly_the_previewed_path() {
        let (dir, v, _) = vault();
        let snap = v.load().unwrap();
        let plan = preview(
            &snap,
            "/temat Biuro > firma",
            &Context {
                vault: Some(&v),
                ..ctx()
            },
        )
        .plan
        .unwrap();
        assert_eq!(plan.steps[0].file, "firma/tematy/biuro.md");
        // Someone takes biuro.md in between.
        v.create_topic(
            "firma",
            NT {
                title: "Biuro".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let a = apply(&v, &plan);
        assert!(a.error.is_some() && a.rolled_back, "{a:?}");
        assert!(!dir.path().join("firma/tematy/biuro-2.md").exists());
    }

    #[test]
    fn edits_and_appended_lines_depend_on_what_the_preview_saw() {
        let (_dir, v, id) = vault();
        let c = Context {
            vault: Some(&v),
            ..ctx()
        };
        let plan = preview(&v.load().unwrap(), &format!("^{id} !wysoki"), &c)
            .plan
            .unwrap();
        let line = preview(
            &v.load().unwrap(),
            &format!("^{id} /czeka [[firma/krs]]"),
            &c,
        )
        .plan
        .unwrap();
        let topic = preview(
            &v.load().unwrap(),
            "[[firma/krs]] /powiaz [[whenubuy/checkout]]",
            &c,
        )
        .plan
        .unwrap();
        // Notes and the summary change after the previews.
        v.update_task(
            &id,
            TaskPatch {
                notes: Some("Nowe notatki.".into()),
                ..Default::default()
            },
        )
        .unwrap();
        v.update_topic(
            "firma",
            "krs",
            crate::model::TopicPatch {
                summary: Some("Inny opis.".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let after = preview(&v.load().unwrap(), &format!("^{id} !wysoki"), &c)
            .plan
            .unwrap();
        assert_ne!(
            super::super::plan::plan_token(&after),
            super::super::plan::plan_token(&plan)
        );
        for p in [&plan, &line, &topic] {
            let a = apply(&v, p);
            assert!(
                a.error
                    .as_deref()
                    .is_some_and(|e| e.contains("zmieniło się")),
                "{a:?}"
            );
        }
        let t = v.load().unwrap().task(&id).unwrap().clone();
        assert_eq!(t.priority, crate::model::Priority::Normal);
        assert_eq!(t.notes, "Nowe notatki.");
        assert_eq!(
            v.load().unwrap().topic("firma", "krs").unwrap().summary,
            "Inny opis."
        );
    }

    #[test]
    fn plans_survive_the_json_round_trip_of_the_app() {
        let (_dir, v, id) = vault();
        let snap = v.load().unwrap();
        let c = Context {
            vault: Some(&v),
            selection: vec![id.clone()],
            ..ctx()
        };
        for text in [
            "Nowe @jutro !wysoki",
            "[/] Coś > whenubuy/checkout /czeka [[firma/krs]]",
            &format!("^{id} [x] @pt > firma"),
            &format!("^{id} /powiaz [[firma/krs]]"),
            "[[firma/krs]] /czeka [[whenubuy/checkout]]",
            "/temat Biuro > firma",
        ] {
            let p = preview(&snap, text, &c);
            let plan = p.plan.unwrap_or_else(|| panic!("{text}: {:?}", p.problems));
            let json = serde_json::to_string(&plan).unwrap();
            let back: Plan = serde_json::from_str(&json).unwrap();
            assert_eq!(back, plan, "{text}");
        }
    }

    #[test]
    fn typos_are_refused_when_exact_names_are_required() {
        let (_dir, v, _) = vault();
        assert!(matches!(
            run(&v, "x > whenbuy", &ctx(), None, true),
            Err(Refusal::Typo { .. })
        ));
        let o = run(&v, "x > whenbuy", &ctx(), None, false).unwrap();
        assert!(o.applied.error.is_none());
        assert!(matches!(
            run(&v, "?dopasuj", &ctx(), None, false),
            Err(Refusal::Invalid { .. })
        ));
        assert!(matches!(
            run(&v, "x > nieznane", &ctx(), None, false),
            Err(Refusal::Invalid { .. })
        ));
    }
}
