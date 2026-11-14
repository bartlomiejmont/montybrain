//! Mutations of the command language (`/czeka`, `/powiaz`, "temat z wpisów")
//! and their guarded undos. Each one is a single locked transaction through
//! the same read → edit → verify → atomic write path as every other mutation.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{
    check_links, continuation_lines, links, location_kind, location_path, read_back,
    render_checked, task_by_id,
};
use crate::doc::{Doc, Outline};
use crate::error::{Error, Result};
use crate::files::{
    FileKind, date_str, insert_task_lines, parse_topic, set_text_region, task_blocks, text_region,
};
use crate::frontmatter::encode_scalar;
use crate::model::*;
use crate::refs::{has_connection_line, parse_refs};
use crate::validate::{Multi, multiline, one_line, topic_title_line};
use crate::vault::{TOPICS_DIR, Vault};

/// One line a mutation wrote, for the echo after a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineWritten {
    /// Relative to the vault root.
    pub file: PathBuf,
    /// 1-based.
    pub line: usize,
    pub text: String,
}

/// What [`Vault::create_topic_from_tasks`] wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicFromTasks {
    pub topic: Topic,
    /// The moved tasks as they are now (same ids, links and notes).
    pub tasks: Vec<Task>,
    /// Where each task was before, in the order given.
    pub origins: Vec<(String, TaskLocation)>,
    /// The new file as written (for the undo's "still untouched?" check).
    pub contents: String,
}

/// A connection line must be one standalone `Czeka na:` / `Powiązane:` line.
fn connection(line: &str, project: Option<&str>) -> Result<String> {
    let line = one_line("line", line)?;
    let refs = parse_refs(&line, project);
    if refs.len() != 1 || !refs[0].standalone {
        return Err(Error::invalid(format!(
            "line: {line:?} is not a single `Czeka na:` / `Powiązane:` connection line"
        )));
    }
    Ok(line)
}

fn region_lines(doc: &Doc) -> (std::ops::Range<usize>, Vec<bool>) {
    let o = Outline::of(doc);
    (text_region(&o), o.code)
}

impl Vault {
    /// Append one standalone connection line (`Czeka na: [[p/s]]`,
    /// `Powiązane: ^id`) at the end of a topic's summary. The other summary
    /// lines are not touched. Refused when the line would not read back as that
    /// connection (e.g. inside an unclosed code block).
    pub fn append_summary_line(
        &self,
        project: &str,
        topic: &str,
        line: &str,
    ) -> Result<(Topic, LineWritten)> {
        self.append_summary_line_if(project, topic, line, |_| true)
            .map(|r| r.expect("no guard"))
    }

    /// [`Vault::append_summary_line`], only while the topic as read inside the
    /// lock satisfies `guard` (e.g. "the summary is still what the preview
    /// saw"); otherwise nothing is written and `Ok(None)`.
    pub fn append_summary_line_if(
        &self,
        project: &str,
        topic: &str,
        line: &str,
        guard: impl Fn(&Topic) -> bool,
    ) -> Result<Option<(Topic, LineWritten)>> {
        let line = connection(line, Some(project))?;
        let target = parse_refs(&line, Some(project)).remove(0);
        let txn = self.txn()?;
        let (rel, mut doc, original) = self.read_topic(&txn, project, topic)?;
        if !guard(&parse_topic(project, topic, &rel, &doc).topic) {
            return Ok(None);
        }
        let (region, code) = region_lines(&doc);
        let last = region.clone().rev().find(|&i| !doc.is_blank(i) && !code[i]);
        match last {
            Some(i) => doc.insert(i + 1, vec![line.clone()]),
            None => set_text_region(&mut doc, &line),
        }
        let p = txn.prepare(
            &rel,
            FileKind::Topic,
            &original,
            &mut doc,
            true,
            &Default::default(),
        )?;
        let parsed = parse_topic(project, topic, &rel, &doc).topic;
        if !has_connection_line(&parsed.summary, Some(project), target.waits, &target.target) {
            return Err(Error::invalid(
                "line: the summary would not read it back as a connection (an unclosed code block?)",
            ));
        }
        let (region, _) = region_lines(&doc);
        let at = region
            .rev()
            .find(|&i| doc.text(i).trim() == line)
            .ok_or_else(|| Error::invalid("line could not be read back"))?;
        txn.write(p)?;
        Ok(Some((
            parsed,
            LineWritten {
                file: rel,
                line: at + 1,
                text: line,
            },
        )))
    }

    /// Remove the last summary line of a topic that is exactly `line` (outside
    /// code). `Ok(false)` when there is no such line: nothing is written.
    pub fn remove_summary_line(&self, project: &str, topic: &str, line: &str) -> Result<bool> {
        self.remove_summary_line_if(project, topic, line, |_| true)
    }

    /// [`Vault::remove_summary_line`], only while the topic as read inside
    /// the lock satisfies `guard` (e.g. "the summary is still what the append
    /// wrote"); otherwise nothing is written and `Ok(false)`.
    pub fn remove_summary_line_if(
        &self,
        project: &str,
        topic: &str,
        line: &str,
        guard: impl Fn(&Topic) -> bool,
    ) -> Result<bool> {
        let line = line.trim();
        let txn = self.txn()?;
        let (rel, mut doc, original) = self.read_topic(&txn, project, topic)?;
        if !guard(&parse_topic(project, topic, &rel, &doc).topic) {
            return Ok(false);
        }
        let (region, code) = region_lines(&doc);
        let Some(at) = region
            .rev()
            .find(|&i| !code[i] && doc.text(i).trim() == line)
        else {
            return Ok(false);
        };
        doc.remove(at..at + 1);
        let summary_empty = parse_topic(project, topic, &rel, &doc)
            .topic
            .summary
            .is_empty();
        if summary_empty {
            set_text_region(&mut doc, "");
        }
        txn.commit(
            &rel,
            FileKind::Topic,
            &original,
            &mut doc,
            true,
            &Default::default(),
        )?;
        Ok(true)
    }

    /// Append one standalone connection line to a task's notes, as the last
    /// line of its block (indented like the block). Other lines stay as they are.
    pub fn append_task_note_line(&self, id: &str, line: &str) -> Result<(Task, LineWritten)> {
        self.append_task_note_line_if(id, line, |_| true)
            .map(|r| r.expect("no guard"))
    }

    /// [`Vault::append_task_note_line`], only while the task as read inside
    /// the lock satisfies `guard`; otherwise nothing is written and `Ok(None)`.
    pub fn append_task_note_line_if(
        &self,
        id: &str,
        line: &str,
        guard: impl Fn(&Task) -> bool,
    ) -> Result<Option<(Task, LineWritten)>> {
        let txn = self.txn()?;
        let found = txn.find_task(id)?;
        if !guard(&found.block.to_task(found.loc.clone())) {
            return Ok(None);
        }
        let project = found.loc.project().map(str::to_string);
        let line = connection(line, project.as_deref())?;
        let target = parse_refs(&line, project.as_deref()).remove(0);
        let task_id = found.block.parsed.id.clone().unwrap_or_default();
        let mut doc = found.doc;
        let text = format!("{}{line}", found.block.base_indent);
        doc.insert(found.block.end, vec![text.clone()]);
        let p = txn.prepare(
            &found.rel,
            found.kind,
            &found.original,
            &mut doc,
            true,
            &Default::default(),
        )?;
        let task = task_by_id(&doc, found.kind, &found.loc, &task_id)
            .ok_or_else(|| Error::TaskNotFound(task_id.clone()))?;
        if !has_connection_line(
            &task.notes,
            project.as_deref(),
            target.waits,
            &target.target,
        ) {
            return Err(Error::invalid(
                "line: the notes would not read it back as a connection",
            ));
        }
        let (blocks, _) = task_blocks(&doc, found.kind);
        let block = blocks
            .iter()
            .find(|b| b.parsed.id.as_deref() == Some(task_id.as_str()))
            .ok_or_else(|| Error::TaskNotFound(task_id.clone()))?;
        let at = (block.line + 1..block.end)
            .rev()
            .find(|&k| doc.text(k).trim() == line)
            .ok_or_else(|| Error::invalid("line could not be read back"))?;
        txn.write(p)?;
        Ok(Some((
            task,
            LineWritten {
                file: found.rel,
                line: at + 1,
                text,
            },
        )))
    }

    /// Remove the last note line of a task that is exactly `line` (trimmed).
    /// `Ok(false)` when there is none.
    pub fn remove_task_note_line(&self, id: &str, line: &str) -> Result<bool> {
        self.remove_task_note_line_if(id, line, |_| true)
    }

    /// [`Vault::remove_task_note_line`], only while the task as read inside the
    /// lock satisfies `guard`; otherwise nothing is written and `Ok(false)`.
    pub fn remove_task_note_line_if(
        &self,
        id: &str,
        line: &str,
        guard: impl Fn(&Task) -> bool,
    ) -> Result<bool> {
        let line = line.trim();
        let txn = self.txn()?;
        let found = txn.find_task(id)?;
        if !guard(&found.block.to_task(found.loc.clone())) {
            return Ok(false);
        }
        let mut doc = found.doc;
        let Some(at) = (found.block.line + 1..found.block.end)
            .rev()
            .find(|&k| doc.text(k).trim() == line)
        else {
            return Ok(false);
        };
        doc.remove(at..at + 1);
        txn.commit(
            &found.rel,
            found.kind,
            &found.original,
            &mut doc,
            true,
            &Default::default(),
        )?;
        Ok(true)
    }

    /// [`Vault::create_task`] with exactly the id `id` (chosen when the plan
    /// was previewed). Refused, with nothing written, when a task with that id
    /// already exists anywhere in the vault: checked inside the lock, so the
    /// same plan cannot be applied twice.
    pub fn create_task_with_id(
        &self,
        location: &TaskLocation,
        new: NewTask,
        id: &str,
    ) -> Result<Task> {
        let id = id.trim().trim_start_matches('^').to_string();
        if !crate::id::is_valid_id(&id) {
            return Err(Error::invalid(format!("id: {id:?} is not a task id")));
        }
        let title = one_line("title", &new.title)?;
        let new_links = links("links", &new.links)?;
        let notes = multiline("notes", &new.notes, Multi::Indented)?;
        let txn = self.txn()?;
        if txn.all_ids()?.contains(&id) {
            return Err(Error::invalid(format!(
                "a task with id ^{id} already exists (this plan was already written?); nothing was written"
            )));
        }
        let (mut doc, original) = txn.read_location(location)?;
        let line = crate::task::TaskLine {
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
        let expect = crate::verify::Expect {
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

    /// [`Vault::update_and_move_task`], but only if the task as read inside the
    /// lock satisfies `guard`; otherwise nothing is written and `Ok(None)`.
    pub fn update_and_move_task_if(
        &self,
        id: &str,
        guard: impl Fn(&Task) -> bool,
        patch: TaskPatch,
        to: &TaskLocation,
    ) -> Result<Option<Task>> {
        self.edit_task(id, patch, Some(to), Some(&guard))
    }

    /// Create a topic in `project` and move the tasks `ids` into it (whole
    /// blocks, byte for byte: ids, links and notes are kept), as one locked
    /// transaction. The new file is written before the sources, so a crash in
    /// between can duplicate a task but never lose one.
    pub fn create_topic_from_tasks(
        &self,
        project: &str,
        new: NewTopic,
        ids: &[String],
    ) -> Result<TopicFromTasks> {
        self.create_topic_from_tasks_if(project, new, ids, None, |_| true)
            .map(|r| r.expect("no guard"))
    }

    /// [`Vault::create_topic_from_tasks`], but only if every task as read
    /// inside the same lock satisfies `guard`; otherwise nothing is written and
    /// `Ok(None)`.
    pub fn create_topic_from_tasks_if(
        &self,
        project: &str,
        new: NewTopic,
        ids: &[String],
        exact_slug: Option<&str>,
        guard: impl Fn(&Task) -> bool,
    ) -> Result<Option<TopicFromTasks>> {
        let title = topic_title_line("title", &new.title)?;
        let summary = multiline("summary", &new.summary, Multi::Region)?;
        let mut wanted: Vec<String> = Vec::new();
        for id in ids {
            let id = id.trim().trim_start_matches('^').to_string();
            if !wanted.contains(&id) {
                wanted.push(id);
            }
        }
        if wanted.is_empty() {
            return Err(Error::invalid("tasks: at least one task is needed"));
        }
        let txn = self.txn()?;
        self.read_project(&txn, project)?;
        let repo = new.repo.as_deref().map(str::trim).filter(|r| !r.is_empty());
        if let Some(r) = repo {
            self.check_repo(&txn, project, r)?;
        }

        // Sources, one document per file.
        struct Source {
            kind: FileKind,
            doc: Doc,
            original: String,
            ids: Vec<String>,
            ranges: Vec<std::ops::Range<usize>>,
        }
        let mut sources: BTreeMap<PathBuf, Source> = BTreeMap::new();
        let mut moved_lines: Vec<String> = Vec::new();
        let mut origins: Vec<(String, TaskLocation)> = Vec::new();
        for id in &wanted {
            let found = txn.find_task(id)?;
            let src = sources.entry(found.rel.clone()).or_insert(Source {
                kind: found.kind,
                doc: found.doc,
                original: found.original,
                ids: Vec::new(),
                ranges: Vec::new(),
            });
            let (blocks, _) = task_blocks(&src.doc, src.kind);
            let block = blocks
                .into_iter()
                .find(|b| b.parsed.id.as_deref() == Some(id.as_str()))
                .ok_or_else(|| Error::TaskNotFound(id.clone()))?;
            if !guard(&block.to_task(found.loc.clone())) {
                return Ok(None);
            }
            moved_lines.extend(block.range().map(|i| src.doc.text(i).to_string()));
            src.ids.push(id.clone());
            src.ranges.push(block.range());
            origins.push((id.clone(), found.loc));
        }

        let topics_dir = self.root.join(project).join(TOPICS_DIR);
        let slug = super::exact_or_unique(&topics_dir, &title, exact_slug)?;
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
        insert_task_lines(&mut doc, FileKind::Topic, moved_lines);
        let rel = PathBuf::from(format!("{project}/{TOPICS_DIR}/{slug}.md"));
        let add = crate::verify::Expect {
            title: true,
            add_tasks: &wanted,
            ..Default::default()
        };
        let pending_new = txn.prepare(&rel, FileKind::Topic, "", &mut doc, false, &add)?;
        let topic = parse_topic(project, &slug, &rel, &doc).topic;
        read_back("title", &topic.title, &title)?;
        read_back("summary", &topic.summary, summary)?;
        let loc = TaskLocation::Topic {
            project: project.to_string(),
            topic: slug.clone(),
        };
        let mut tasks = Vec::new();
        for id in &wanted {
            tasks.push(
                task_by_id(&doc, FileKind::Topic, &loc, id)
                    .ok_or_else(|| Error::invalid("task could not be read back"))?,
            );
        }
        let mut pending_src = Vec::new();
        for (src_rel, mut src) in sources {
            let mut ranges = src.ranges.clone();
            ranges.sort_by_key(|r| std::cmp::Reverse(r.start));
            for r in ranges {
                src.doc.remove(r);
            }
            let remove = crate::verify::Expect {
                remove_tasks: &src.ids,
                ..Default::default()
            };
            pending_src.push(txn.prepare(
                &src_rel,
                src.kind,
                &src.original,
                &mut src.doc,
                true,
                &remove,
            )?);
        }
        let contents = doc.render();
        fs::create_dir_all(&topics_dir).map_err(|e| Error::io(&topics_dir, e))?;
        txn.write(pending_new)?;
        for p in pending_src {
            txn.write(p)?;
        }
        Ok(Some(TopicFromTasks {
            topic,
            tasks,
            origins,
            contents,
        }))
    }

    /// Undo of [`Vault::create_topic_from_tasks`]: only while the topic file is
    /// still exactly `expected`, move each task back to its origin (at the end
    /// of that file's tasks) and remove the topic file, as one locked
    /// transaction (origins are written before the topic file is removed).
    /// `Ok(None)` when the file changed: nothing is touched.
    pub fn dissolve_topic_if(
        &self,
        project: &str,
        topic: &str,
        expected: &str,
        origins: &[(String, TaskLocation)],
    ) -> Result<Option<Vec<Task>>> {
        let txn = self.txn()?;
        let (rel, doc, original) = self.read_topic(&txn, project, topic)?;
        if original != expected {
            return Ok(None);
        }
        let (blocks, _) = task_blocks(&doc, FileKind::Topic);
        let mut dests: BTreeMap<PathBuf, (TaskLocation, Vec<String>, Vec<String>)> =
            BTreeMap::new();
        for (id, loc) in origins {
            let block = blocks
                .iter()
                .find(|b| b.parsed.id.as_deref() == Some(id.as_str()))
                .ok_or_else(|| Error::TaskNotFound(id.clone()))?;
            let entry = dests
                .entry(location_path(loc))
                .or_insert_with(|| (loc.clone(), Vec::new(), Vec::new()));
            entry
                .1
                .extend(block.range().map(|i| doc.text(i).to_string()));
            entry.2.push(id.clone());
        }
        let mut pending = Vec::new();
        let mut back = Vec::new();
        for (dest_rel, (loc, lines, ids)) in &dests {
            let (mut dest_doc, dest_original) = txn.read_location(loc)?;
            let kind = location_kind(loc);
            insert_task_lines(&mut dest_doc, kind, lines.clone());
            let add = crate::verify::Expect {
                add_tasks: ids,
                ..Default::default()
            };
            pending.push(txn.prepare(dest_rel, kind, &dest_original, &mut dest_doc, true, &add)?);
            for id in ids {
                back.push(
                    task_by_id(&dest_doc, kind, loc, id)
                        .ok_or_else(|| Error::invalid("task could not be read back"))?,
                );
            }
        }
        for p in pending {
            txn.write(p)?;
        }
        let abs = txn.abs(&rel);
        fs::remove_file(&abs).map_err(|e| Error::io(&abs, e))?;
        Ok(Some(back))
    }

    /// The task line of a new task as it would be written (id left out: it is
    /// assigned at write time). `Err` when the title cannot be stored
    /// unambiguously, exactly as [`Vault::create_task`] would refuse it.
    pub fn render_new_task_line(new: &NewTask, today: chrono::NaiveDate) -> Result<String> {
        let title = one_line("title", &new.title)?;
        let line = crate::task::TaskLine {
            bullet: '-',
            status: new.status,
            title,
            priority: new.priority,
            due: new.due,
            done: new.status.is_done().then_some(today),
            id: None,
            problems: Vec::new(),
        };
        render_checked(&line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn vault() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::init(dir.path()).unwrap().with_clock(|| {
            NaiveDate::from_ymd_opt(2026, 11, 14)
                .unwrap()
                .and_hms_opt(10, 0, 0)
                .unwrap()
        });
        v.create_project(NewProject {
            name: "Firma".into(),
            ..Default::default()
        })
        .unwrap();
        (dir, v)
    }

    #[test]
    fn summary_lines_are_appended_and_removed_exactly() {
        let (dir, v) = vault();
        let t = v
            .create_topic(
                "firma",
                NewTopic {
                    title: "KRS".into(),
                    summary: "Odpis zamówiony.\n\nSąd ma termin.".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let path = dir.path().join(&t.path);
        let before = fs::read_to_string(&path).unwrap();
        let (_, w) = v
            .append_summary_line("firma", "krs", "Czeka na: [[firma/umowy]]")
            .unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert_eq!(w.text, "Czeka na: [[firma/umowy]]");
        assert_eq!(
            after.lines().nth(w.line - 1),
            Some("Czeka na: [[firma/umowy]]")
        );
        // Only that line is new (and `zaktualizowano`).
        let diff: Vec<_> = after
            .lines()
            .filter(|l| !before.lines().any(|b| b == *l))
            .collect();
        assert_eq!(diff, ["Czeka na: [[firma/umowy]]"]);
        let s = v.load().unwrap();
        assert_eq!(
            s.topic("firma", "krs").unwrap().summary,
            "Odpis zamówiony.\n\nSąd ma termin.\nCzeka na: [[firma/umowy]]"
        );
        assert!(
            v.remove_summary_line("firma", "krs", "Czeka na: [[firma/umowy]]")
                .unwrap()
        );
        assert!(
            !v.remove_summary_line("firma", "krs", "Czeka na: [[firma/umowy]]")
                .unwrap()
        );
        let s = v.load().unwrap();
        assert_eq!(
            s.topic("firma", "krs").unwrap().summary,
            "Odpis zamówiony.\n\nSąd ma termin."
        );
        // Prose is not a connection line.
        assert!(
            v.append_summary_line("firma", "krs", "Patrz [[firma/x]] i [[firma/y]]")
                .is_err()
        );
    }

    #[test]
    fn an_empty_summary_and_its_undo_round_trip() {
        let (dir, v) = vault();
        let t = v
            .create_topic(
                "firma",
                NewTopic {
                    title: "Pusty".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let path = dir.path().join(&t.path);
        let before = fs::read_to_string(&path).unwrap();
        v.append_summary_line("firma", "pusty", "Powiązane: ^abc123")
            .unwrap();
        assert_eq!(
            v.load().unwrap().topic("firma", "pusty").unwrap().summary,
            "Powiązane: ^abc123"
        );
        v.remove_summary_line("firma", "pusty", "Powiązane: ^abc123")
            .unwrap();
        let back = fs::read_to_string(&path).unwrap();
        // Same apart from `zaktualizowano` (same clock here, so identical).
        assert_eq!(back, before);
    }

    #[test]
    fn note_lines_are_appended_and_removed_exactly() {
        let (_dir, v) = vault();
        let t = v
            .create_task(
                &TaskLocation::Inbox,
                NewTask {
                    title: "Zadzwonić".into(),
                    links: vec!["https://x.y/1".into()],
                    notes: "Numer w mailu.".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let id = t.id.unwrap();
        let (task, w) = v.append_task_note_line(&id, "Czeka na: ^c3d4e5").unwrap();
        assert_eq!(task.notes, "Numer w mailu.\nCzeka na: ^c3d4e5");
        assert_eq!(task.links, ["https://x.y/1"]);
        assert_eq!(w.text, "  Czeka na: ^c3d4e5");
        assert!(v.remove_task_note_line(&id, "Czeka na: ^c3d4e5").unwrap());
        let s = v.load().unwrap();
        assert_eq!(s.task(&id).unwrap().notes, "Numer w mailu.");
        assert!(!v.remove_task_note_line(&id, "Czeka na: ^c3d4e5").unwrap());
    }

    #[test]
    fn topic_from_tasks_moves_blocks_and_dissolves_back() {
        let (dir, v) = vault();
        let mk = |title: &str, notes: &str| {
            v.create_task(
                &TaskLocation::Inbox,
                NewTask {
                    title: title.into(),
                    notes: notes.into(),
                    ..Default::default()
                },
            )
            .unwrap()
            .id
            .unwrap()
        };
        let a = mk("Umowa najmu", "Wersja od prawnika.");
        let keep = mk("Mleko", "");
        let b = mk("Aneks", "");
        let inbox = dir.path().join("skrzynka.md");
        let inbox_before = fs::read_to_string(&inbox).unwrap();
        let made = v
            .create_topic_from_tasks(
                "firma",
                NewTopic {
                    title: "Biuro".into(),
                    ..Default::default()
                },
                &[a.clone(), b.clone()],
            )
            .unwrap();
        assert_eq!(made.topic.slug, "biuro");
        let s = v.load().unwrap();
        let ta = s.task(&a).unwrap();
        assert_eq!(ta.notes, "Wersja od prawnika.");
        assert_eq!(ta.location.topic(), Some("biuro"));
        assert_eq!(s.task(&keep).unwrap().location, TaskLocation::Inbox);
        assert_eq!(made.origins.len(), 2);
        // Undo while untouched: back in the inbox, the file is gone.
        let back = v
            .dissolve_topic_if("firma", "biuro", &made.contents, &made.origins)
            .unwrap()
            .unwrap();
        assert_eq!(back.len(), 2);
        assert!(!dir.path().join("firma/tematy/biuro.md").exists());
        let s = v.load().unwrap();
        assert_eq!(s.task(&a).unwrap().location, TaskLocation::Inbox);
        assert_eq!(s.task(&a).unwrap().notes, "Wersja od prawnika.");
        // Order within the inbox: moved ones come after the kept one.
        let now = fs::read_to_string(&inbox).unwrap();
        assert_ne!(now, inbox_before);
        assert_eq!(now.lines().filter(|l| l.starts_with("- [")).count(), 3);
        // A changed topic is left alone.
        let made = v
            .create_topic_from_tasks(
                "firma",
                NewTopic {
                    title: "Biuro".into(),
                    ..Default::default()
                },
                std::slice::from_ref(&a),
            )
            .unwrap();
        v.update_task(
            &a,
            TaskPatch {
                status: Some(TaskStatus::Doing),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            v.dissolve_topic_if("firma", &made.topic.slug, &made.contents, &made.origins)
                .unwrap()
                .is_none()
        );
        // Unknown task: nothing written.
        assert!(
            v.create_topic_from_tasks(
                "firma",
                NewTopic {
                    title: "X".into(),
                    ..Default::default()
                },
                &["zzzzzz".into()]
            )
            .is_err()
        );
        assert!(!dir.path().join("firma/tematy/x.md").exists());
    }

    #[test]
    fn topic_titles_with_brackets_are_refused_with_a_clear_message() {
        let (_dir, v) = vault();
        let e = v
            .create_topic(
                "firma",
                NewTopic {
                    title: "Testy [beta]".into(),
                    ..Default::default()
                },
            )
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("[ or ]") && e.contains("[[project/slug]]"),
            "{e}"
        );
        let t = v
            .create_topic(
                "firma",
                NewTopic {
                    title: "Beta".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(
            v.update_topic(
                "firma",
                &t.slug,
                TopicPatch {
                    title: Some("Beta [2]".into()),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn exact_topic_paths_are_refused_when_taken() {
        let (dir, v) = vault();
        v.create_topic_exact(
            "firma",
            "biuro",
            NewTopic {
                title: "Biuro".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            v.create_topic_exact(
                "firma",
                "biuro",
                NewTopic {
                    title: "Biuro".into(),
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert!(!dir.path().join("firma/tematy/biuro-2.md").exists());
    }

    #[test]
    fn guarded_move() {
        let (_dir, v) = vault();
        let id = v
            .create_task(
                &TaskLocation::Inbox,
                NewTask {
                    title: "a".into(),
                    ..Default::default()
                },
            )
            .unwrap()
            .id
            .unwrap();
        let to = TaskLocation::Project {
            project: "firma".into(),
        };
        assert!(
            v.update_and_move_task_if(
                &id,
                |t| t.location != TaskLocation::Inbox,
                TaskPatch::default(),
                &to
            )
            .unwrap()
            .is_none()
        );
        let moved = v
            .update_and_move_task_if(
                &id,
                |t| t.location == TaskLocation::Inbox,
                TaskPatch::default(),
                &to,
            )
            .unwrap()
            .unwrap();
        assert_eq!(moved.location, to);
    }

    #[test]
    fn new_task_line_preview_matches_create() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 14).unwrap();
        let new = NewTask {
            title: "Zadzwonić do sądu".into(),
            priority: Priority::High,
            due: Some(Due::date(NaiveDate::from_ymd_opt(2026, 11, 15).unwrap())),
            ..Default::default()
        };
        assert_eq!(
            Vault::render_new_task_line(&new, today).unwrap(),
            "- [ ] Zadzwonić do sądu !wysoki @2026-11-15"
        );
        let bad = NewTask {
            title: "Meet @2026-11-20".into(),
            ..Default::default()
        };
        assert!(Vault::render_new_task_line(&bad, today).is_err());
    }
}
