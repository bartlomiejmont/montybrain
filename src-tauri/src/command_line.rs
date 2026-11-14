//! The command field of Skrzynka, the quick-capture window and ⌘K: preview,
//! apply (only a previewed plan, only while its data is unchanged), undo, and
//! the local inbox suggestions. The grammar lives in
//! `montybrain_core::command`; nothing here parses text.

use std::sync::Mutex;

use montybrain_core::command::{
    self, Applied, Completions, Context, InboxSuggestion, Plan, Preview, Refusal, State,
    UndoOutcome, UndoStatus, UndoStep,
};
use montybrain_core::{Snapshot, Task, TaskLocation, Vault};
use serde::{Deserialize, Serialize};

use crate::lock;

/// How many writes can be taken back (newest first).
const UNDO_KEEP: usize = 20;

/// Undo records of command writes, in memory only (not after a restart).
#[derive(Default)]
pub struct CommandUndo {
    inner: Mutex<UndoList>,
}

#[derive(Default)]
struct UndoList {
    next: u64,
    items: Vec<Record>,
}

#[derive(Debug, Clone)]
struct Record {
    token: u64,
    /// Where the write came from (`inbox`, `quick`, `palette`, `field`).
    source: String,
    reading: String,
    steps: Vec<UndoStep>,
}

impl CommandUndo {
    /// Stores the steps of one write; returns its token (`None` when nothing to undo).
    pub fn push(&self, source: &str, reading: &str, steps: Vec<UndoStep>) -> Option<u64> {
        if steps.is_empty() {
            return None;
        }
        let mut l = lock(&self.inner);
        l.next += 1;
        let token = l.next;
        l.items.push(Record {
            token,
            source: source.to_string(),
            reading: reading.to_string(),
            steps,
        });
        let over = l.items.len().saturating_sub(UNDO_KEEP);
        l.items.drain(..over);
        Some(token)
    }

    /// A copy of the record `token`, or of the newest one from `source`
    /// (any source when `None`). The record stays until [`CommandUndo::resolve`].
    fn peek(&self, token: Option<u64>, source: Option<&str>) -> Option<Record> {
        let l = lock(&self.inner);
        match token {
            Some(t) => l.items.iter().find(|r| r.token == t).cloned(),
            None => l
                .items
                .iter()
                .rev()
                .find(|r| source.is_none_or(|s| r.source == s))
                .cloned(),
        }
    }

    /// After an undo of `token`: the steps that failed stay (so it can be
    /// retried); a record with nothing left is removed.
    fn resolve(&self, token: u64, left: Vec<UndoStep>) {
        let mut l = lock(&self.inner);
        if let Some(i) = l.items.iter().position(|r| r.token == token) {
            if left.is_empty() {
                l.items.remove(i);
            } else {
                l.items[i].steps = left;
            }
        }
    }
}

/// What the field sends with the text.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FieldInput {
    pub text: String,
    /// Caret position in UTF-16 units, for autocomplete.
    pub caret: Option<usize>,
    /// Selected inbox entries (Skrzynka).
    pub selection: Vec<String>,
    /// The focused inbox entry (Skrzynka): the subject of `/czeka …` when
    /// nothing is selected.
    pub focus: Option<String>,
    /// Where a plain new task goes (default: the inbox).
    pub location: Option<TaskLocation>,
    /// Which field it is (`inbox`, `quick`, `palette`): undo with `u` in
    /// Skrzynka takes back only Skrzynka's own writes.
    pub source: Option<String>,
}

impl FieldInput {
    fn source(&self) -> &str {
        self.source.as_deref().unwrap_or("field")
    }
}

impl FieldInput {
    pub fn context<'a>(
        &self,
        vault: Option<&'a Vault>,
        today: montybrain_core::chrono::NaiveDate,
    ) -> Context<'a> {
        Context {
            today,
            selection: self.selection.clone(),
            focus: self.focus.clone(),
            // Only a preview reads half-typed words as typing; a write never does.
            caret: vault.and(self.caret),
            default_location: self.location.clone().unwrap_or(TaskLocation::Inbox),
            vault,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldPreview {
    pub preview: Preview,
    pub completions: Option<Completions>,
}

pub fn preview(
    snap: &Snapshot,
    vault: &Vault,
    input: &FieldInput,
    today: montybrain_core::chrono::NaiveDate,
) -> FieldPreview {
    let ctx = input.context(Some(vault), today);
    let preview = command::preview(snap, &input.text, &ctx);
    let completions = input
        .caret
        .and_then(|c| command::complete(snap, &input.text, c, today));
    FieldPreview {
        preview,
        completions,
    }
}

/// `command_apply` result.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApplyResult {
    Written {
        reading: String,
        applied: Applied,
        /// For `command_undo` (absent when nothing was written).
        undo: Option<u64>,
    },
    /// The data changed since the preview: confirm the new plan.
    Stale {
        message: String,
        preview: Box<Preview>,
    },
    /// Not writable (problems, choices, an agent request).
    Refused {
        message: String,
        preview: Option<Box<Preview>>,
    },
}

pub fn apply(
    vault: &Vault,
    undo: &CommandUndo,
    input: &FieldInput,
    plan: &Plan,
    today: montybrain_core::chrono::NaiveDate,
) -> ApplyResult {
    let ctx = input.context(None, today);
    match command::run(vault, &input.text, &ctx, Some(plan), false) {
        // All or nothing: a failure whose written steps were taken back is a refusal.
        Ok(o) if o.applied.error.is_some() && o.applied.rolled_back => ApplyResult::Refused {
            message: format!(
                "Nic nie zostało zapisane: {}",
                o.applied.error.unwrap_or_default()
            ),
            preview: None,
        },
        Ok(o) => {
            // A complete write, or (rarely) a rollback that could not finish:
            // then `undo` holds exactly what is still written.
            let token = undo.push(input.source(), &o.preview.reading, o.applied.undo.clone());
            ApplyResult::Written {
                reading: o.preview.reading,
                applied: o.applied,
                undo: token,
            }
        }
        Err(Refusal::Stale { preview }) => ApplyResult::Stale {
            message: "Dane zmieniły się od podglądu. Sprawdź plan jeszcze raz.".into(),
            preview,
        },
        Err(r @ (Refusal::Invalid { .. } | Refusal::Typo { .. })) => {
            let message = r.message();
            let preview = match r {
                Refusal::Invalid { preview } | Refusal::Typo { preview } => Some(preview),
                _ => None,
            };
            ApplyResult::Refused { message, preview }
        }
        Err(Refusal::Failed { message }) => ApplyResult::Refused {
            message,
            preview: None,
        },
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UndoReport {
    pub reading: String,
    /// Where the undone write came from.
    pub source: String,
    pub items: Vec<UndoOutcome>,
    /// Some steps failed and can be retried with this token.
    pub retry: Option<u64>,
}

/// Takes back the write `token`, or the newest one from `source`. Steps that
/// failed stay undoable (a retry runs only them); steps that changed meanwhile
/// are reported and dropped.
pub fn undo(
    vault: &Vault,
    list: &CommandUndo,
    token: Option<u64>,
    source: Option<&str>,
) -> Result<UndoReport, String> {
    let rec = list.peek(token, source).ok_or("Nie ma już czego cofnąć.")?;
    let items = command::undo(vault, &rec.steps);
    // `command::undo` answers newest first.
    let left: Vec<UndoStep> = rec
        .steps
        .iter()
        .rev()
        .zip(&items)
        .filter(|(_, o)| o.status == UndoStatus::Failed)
        .map(|(s, _)| s.clone())
        .rev()
        .collect();
    let retry = !left.is_empty();
    list.resolve(rec.token, left);
    Ok(UndoReport {
        reading: rec.reading,
        source: rec.source,
        items,
        retry: retry.then_some(rec.token),
    })
}

/// A plain capture into `location` (the "dodaj zadanie" fields of projects and
/// topics): a title with optional trailing `!`/`@`. Anything else is refused
/// with a pointer to Skrzynka / ⌘K, where commands are confirmed.
pub fn capture(
    vault: &Vault,
    undo: &CommandUndo,
    location: TaskLocation,
    text: &str,
    today: montybrain_core::chrono::NaiveDate,
) -> Result<(Task, Option<u64>), String> {
    let input = FieldInput {
        text: text.to_string(),
        location: Some(location),
        ..Default::default()
    };
    let snap = vault.load().map_err(|e| e.to_string())?;
    let p = command::preview(&snap, text, &input.context(Some(vault), today));
    match p.state {
        State::Capture => {}
        State::Confirm | State::Agent => {
            return Err(
                "To jest komenda: wpisz ją w Skrzynce albo w ⌘K, gdzie pokażę plan do potwierdzenia."
                    .into(),
            );
        }
        State::Empty => return Err("Wpisz tytuł zadania.".into()),
        State::Invalid | State::Typing => return Err(p.problems.join(" ")),
    }
    let plan = p.plan.expect("a capture has a plan");
    let o = command::run(vault, text, &input.context(None, today), Some(&plan), false)
        .map_err(|r| r.message())?;
    if let Some(e) = o.applied.error {
        return Err(e);
    }
    let task = o
        .applied
        .undo
        .iter()
        .find_map(|u| match u {
            UndoStep::CreatedTask { task, .. } => Some(task.clone()),
            _ => None,
        })
        .ok_or("Zadanie nie zostało zapisane.")?;
    let token = undo.push("list", &o.preview.reading, o.applied.undo);
    Ok((task, token))
}

pub fn suggestions(
    snap: &Snapshot,
    today: montybrain_core::chrono::NaiveDate,
) -> Vec<InboxSuggestion> {
    command::inbox_suggestions(snap, today)
}

/// At most this many entries go to the agent in one request.
pub const AGENT_MAX_ENTRIES: usize = 40;

/// The data of a Skrzynka agent request: only the chosen entries (or every
/// open inbox entry when none is chosen) with their top local candidates. Not
/// the rest of the vault.
pub fn agent_data(
    snap: &Snapshot,
    ids: &[String],
    today: montybrain_core::chrono::NaiveDate,
) -> Result<String, String> {
    let entries: Vec<&Task> = if ids.is_empty() {
        snap.tasks
            .iter()
            .filter(|t| {
                t.location == TaskLocation::Inbox
                    && t.status != montybrain_core::TaskStatus::Done
                    && t.id.is_some()
            })
            .collect()
    } else {
        ids.iter()
            .map(|id| snap.task(id).ok_or_else(|| format!("Nie ma wpisu ^{id}.")))
            .collect::<Result<_, _>>()?
    };
    if entries.is_empty() {
        // A request can still be about something else (agent mode): say so in the data.
        return Ok("(Skrzynka jest pusta: nie ma wpisów do uporządkowania.)\n".into());
    }
    let mut out = String::new();
    for t in entries.iter().take(AGENT_MAX_ENTRIES) {
        let id = t.id.as_deref().unwrap_or("");
        out.push_str(&format!("- ^{id} „{}”", t.title));
        if let Some(d) = t.due {
            out.push_str(&format!(" (termin {d})"));
        }
        out.push('\n');
        for line in t.notes.lines().filter(|l| !l.trim().is_empty()).take(3) {
            out.push_str(&format!("  notatka: {}\n", line.trim()));
        }
        let cands = command::candidates(snap, &t.title, today);
        if cands.is_empty() {
            out.push_str("  podpowiedzi: brak (nic lokalnie nie pasuje)\n");
        }
        for c in cands {
            out.push_str(&format!(
                "  podpowiedź: {} ({}), wynik {:.1}\n",
                c.place.text, c.place.label, c.score
            ));
        }
    }
    if entries.len() > AGENT_MAX_ENTRIES {
        out.push_str(&format!(
            "(i jeszcze {} wpisów pominiętych)\n",
            entries.len() - AGENT_MAX_ENTRIES
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use montybrain_core::chrono::NaiveDate;
    use montybrain_core::{NewProject, NewTopic};

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 14).unwrap()
    }

    fn vault() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::init(dir.path()).unwrap();
        v.create_project(NewProject {
            name: "Firma".into(),
            ..Default::default()
        })
        .unwrap();
        v.create_topic(
            "firma",
            NewTopic {
                title: "KRS".into(),
                ..Default::default()
            },
        )
        .unwrap();
        (dir, v)
    }

    #[test]
    fn preview_apply_undo_through_the_app_layer() {
        let (_d, v) = vault();
        let list = CommandUndo::default();
        let input = FieldInput {
            text: "Zadzwonić do sądu @jutro > firma/krs".into(),
            caret: Some(5),
            ..Default::default()
        };
        let snap = v.load().unwrap();
        let p = preview(&snap, &v, &input, today());
        assert_eq!(p.preview.state, State::Confirm);
        let plan = p.preview.plan.clone().unwrap();
        let r = apply(&v, &list, &input, &plan, today());
        let ApplyResult::Written {
            undo: Some(token),
            applied,
            ..
        } = r
        else {
            panic!("{r:?}")
        };
        assert!(
            applied.echo[0]
                .text
                .contains("Zadzwonić do sądu @2026-11-15")
        );
        // A second apply of the same text is a different plan now? No: it would
        // create a second task; but the stored undo works once.
        let rep = undo(&v, &list, Some(token), None).unwrap();
        assert_eq!(
            rep.items[0].status,
            montybrain_core::command::UndoStatus::Undone
        );
        assert!(rep.retry.is_none());
        assert!(undo(&v, &list, Some(token), None).is_err());
    }

    #[test]
    fn stale_plans_come_back_with_a_new_preview() {
        let (_d, v) = vault();
        let list = CommandUndo::default();
        let input = FieldInput {
            text: "Coś > firma".into(),
            ..Default::default()
        };
        let snap = v.load().unwrap();
        let mut plan = preview(&snap, &v, &input, today()).preview.plan.unwrap();
        plan.steps[0].summary.push('!');
        assert!(matches!(
            apply(&v, &list, &input, &plan, today()),
            ApplyResult::Stale { .. }
        ));
        assert!(v.load().unwrap().tasks.is_empty());
    }

    #[test]
    fn agent_data_has_only_the_chosen_entries_and_their_candidates() {
        let (_d, v) = vault();
        let a = v
            .create_task(
                &TaskLocation::Inbox,
                montybrain_core::NewTask {
                    title: "Odpis KRS".into(),
                    notes: "Z sądu.".into(),
                    ..Default::default()
                },
            )
            .unwrap()
            .id
            .unwrap();
        v.create_task(
            &TaskLocation::Inbox,
            montybrain_core::NewTask {
                title: "Tajny wpis".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let snap = v.load().unwrap();
        let d = agent_data(&snap, std::slice::from_ref(&a), today()).unwrap();
        assert!(d.contains(&format!("^{a} „Odpis KRS”")), "{d}");
        assert!(d.contains("notatka: Z sądu."), "{d}");
        assert!(d.contains("podpowiedź: firma/krs"), "{d}");
        assert!(!d.contains("Tajny"), "{d}");
        // No selection: every open inbox entry.
        assert!(agent_data(&snap, &[], today()).unwrap().contains("Tajny"));
        assert!(agent_data(&snap, &["zzzzzz".into()], today()).is_err());
    }

    #[test]
    fn capture_takes_plain_text_only() {
        let (_d, v) = vault();
        let list = CommandUndo::default();
        let loc = TaskLocation::Topic {
            project: "firma".into(),
            topic: "krs".into(),
        };
        let (t, token) = capture(&v, &list, loc.clone(), "Pismo do sądu !wysoki", today()).unwrap();
        assert_eq!(t.title, "Pismo do sądu");
        assert_eq!(t.location, loc);
        assert!(token.is_some());
        assert!(capture(&v, &list, loc.clone(), "Pismo > firma", today()).is_err());
        assert!(capture(&v, &list, loc, "?dopasuj", today()).is_err());
        // Newest undo first; the list keeps at most UNDO_KEEP.
        for _ in 0..25 {
            list.push(
                "inbox",
                "x",
                vec![UndoStep::CreatedTopic {
                    project: "p".into(),
                    slug: "s".into(),
                    title: "t".into(),
                    contents: String::new(),
                }],
            );
        }
        assert_eq!(lock(&list.inner).items.len(), UNDO_KEEP);
    }

    #[test]
    fn a_changed_entry_makes_a_bulk_write_stale_with_nothing_written() {
        let (_d, v) = vault();
        let list = CommandUndo::default();
        let a = v
            .create_task(
                &TaskLocation::Inbox,
                montybrain_core::NewTask {
                    title: "A".into(),
                    ..Default::default()
                },
            )
            .unwrap()
            .id
            .unwrap();
        let b = v
            .create_task(
                &TaskLocation::Inbox,
                montybrain_core::NewTask {
                    title: "B".into(),
                    ..Default::default()
                },
            )
            .unwrap()
            .id
            .unwrap();
        let input = FieldInput {
            text: "> firma".into(),
            selection: vec![a.clone(), b.clone()],
            ..Default::default()
        };
        let snap = v.load().unwrap();
        let plan = preview(&snap, &v, &input, today()).preview.plan.unwrap();
        // `b` changes after the preview: run() sees a different plan, stale.
        v.update_task(
            &b,
            montybrain_core::TaskPatch {
                title: Some("B2".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(matches!(
            apply(&v, &list, &input, &plan, today()),
            ApplyResult::Stale { .. }
        ));
        assert_eq!(
            v.load().unwrap().task(&a).unwrap().location,
            TaskLocation::Inbox
        );
    }

    #[test]
    fn undo_is_scoped_to_its_source_and_kept_when_it_fails() {
        let (_d, v) = vault();
        let list = CommandUndo::default();
        // A step that cannot be undone (an invalid slug makes core refuse).
        let broken = UndoStep::CreatedTopic {
            project: "a/b".into(),
            slug: "s".into(),
            title: "t".into(),
            contents: String::new(),
        };
        let quick = list.push("quick", "z okna", vec![broken]).unwrap();
        let inbox_input = FieldInput {
            text: "Coś > firma".into(),
            source: Some("inbox".into()),
            ..Default::default()
        };
        let snap = v.load().unwrap();
        let plan = preview(&snap, &v, &inbox_input, today())
            .preview
            .plan
            .unwrap();
        let ApplyResult::Written { undo: Some(_), .. } =
            apply(&v, &list, &inbox_input, &plan, today())
        else {
            panic!()
        };
        // `u` in Skrzynka: the inbox write, not the newer-or-older quick one.
        let r = undo(&v, &list, None, Some("inbox")).unwrap();
        assert_eq!(r.source, "inbox");
        assert_eq!(r.items[0].status, UndoStatus::Undone);
        assert!(undo(&v, &list, None, Some("inbox")).is_err());
        // A failed undo stays and can be retried.
        let r = undo(&v, &list, None, Some("quick")).unwrap();
        assert_eq!(r.items[0].status, UndoStatus::Failed);
        assert_eq!(r.retry, Some(quick));
        assert!(undo(&v, &list, Some(quick), None).is_ok());
    }
}
