//! File-level views (inbox, `projekt.md`, topic files) and structural edits.

use std::path::Path;

use chrono::{NaiveDate, NaiveDateTime};

use crate::doc::{Doc, Outline, heading_level_1, joined_trimmed};
use crate::frontmatter::{Frontmatter, parse_repos, repos_from_items, set_scalar};
use crate::journal::{JournalLine, scan_journal};
use crate::model::{
    JournalEntry, Project, ProjectState, Task, TaskLocation, Topic, TopicStatus, minute,
    parse_date_strict,
};
use crate::task::{TaskBlock, scan_tasks};

pub(crate) const TASKS_SECTION: &str = "Zadania";
pub(crate) const JOURNAL_SECTION: &str = "Dziennik";

pub(crate) const PROJECT_KEYS: &[&str] = &["nazwa", "kolor", "stan", "repozytoria"];
pub(crate) const TOPIC_KEYS: &[&str] = &["status", "repo", "utworzono", "zaktualizowano"];
pub(crate) const UPDATED_KEY: &str = "zaktualizowano";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileKind {
    Inbox,
    Project,
    Topic,
}

/// Problems found while parsing: `(0-based line index or None, message)`.
pub(crate) type Problems = Vec<(Option<usize>, String)>;

fn lift(problems: Vec<(usize, String)>) -> impl Iterator<Item = (Option<usize>, String)> {
    problems.into_iter().map(|(l, m)| (Some(l), m))
}

/// Keys of `order` that precede `key` (used to place a newly inserted key).
pub(crate) fn keys_before<'a>(order: &'a [&'a str], key: &str) -> &'a [&'a str] {
    let pos = order.iter().position(|k| *k == key).unwrap_or(order.len());
    &order[..pos]
}

/// All task blocks of a file, in file order.
pub(crate) fn task_blocks(doc: &Doc, kind: FileKind) -> (Vec<TaskBlock>, Vec<(usize, String)>) {
    let o = Outline::of(doc);
    let mut ranges = Vec::new();
    if kind == FileKind::Inbox {
        ranges.push(o.body_start..o.first_section_start());
    }
    for s in o.sections.iter().filter(|s| s.is(TASKS_SECTION)) {
        ranges.push(s.content());
    }
    let mut blocks = Vec::new();
    let mut problems = Vec::new();
    for r in ranges {
        let (b, p) = scan_tasks(doc, r, &o.code);
        blocks.extend(b);
        problems.extend(p);
    }
    (blocks, problems)
}

pub(crate) fn journal_lines(doc: &Doc) -> (Vec<JournalLine>, Vec<(usize, String)>) {
    let o = Outline::of(doc);
    let mut entries = Vec::new();
    let mut problems = Vec::new();
    for s in o.sections.iter().filter(|s| s.is(JOURNAL_SECTION)) {
        let (e, p) = scan_journal(doc, s.content(), &o.code);
        entries.extend(e);
        problems.extend(p);
    }
    (entries, problems)
}

fn common_problems(o: &Outline, problems: &mut Problems) {
    if o.frontmatter_unclosed {
        problems.push((
            Some(0),
            "frontmatter starts with `---` but is never closed; ignored".into(),
        ));
    }
}

fn fm_scalar(
    doc: &Doc,
    fm: Option<&Frontmatter>,
    key: &str,
    problems: &mut Problems,
) -> Option<String> {
    let fm = fm?;
    match fm.scalar(doc, key) {
        Ok(v) => {
            if fm.continuation(doc, key).is_some() {
                let line = fm.entry(key).map(|e| e.line);
                problems.push((
                    line,
                    format!(
                        "frontmatter `{key}`: the indented lines below it are not supported; they are kept but ignored"
                    ),
                ));
            }
            v
        }
        Err(msg) => {
            let line = fm.entry(key).map(|e| e.line);
            problems.push((line, format!("frontmatter `{key}`: {msg}")));
            None
        }
    }
}

/// Report every repeated occurrence of one of MontyBrain's own keys: the first one is
/// read and written, the others are kept as they are and ignored.
fn duplicate_keys(fm: Option<&Frontmatter>, keys: &[&str], problems: &mut Problems) {
    let Some(fm) = fm else { return };
    let updated = (!keys.contains(&UPDATED_KEY)).then_some(UPDATED_KEY);
    for key in keys.iter().copied().chain(updated) {
        for e in fm.duplicates(key) {
            problems.push((
                Some(e.line),
                format!(
                    "duplicate frontmatter key `{key}`: the first one is used; this one is kept but ignored"
                ),
            ));
        }
    }
}

/// Region holding the free text after the H1 (or after the frontmatter if there is
/// no H1) up to the first `##` section.
pub(crate) fn text_region(o: &Outline) -> std::ops::Range<usize> {
    let start = o.h1.map_or(o.body_start, |h| h + 1);
    start..o.first_section_start().max(start)
}

// ---------------------------------------------------------------------------
// Parsing into model types
// ---------------------------------------------------------------------------

pub(crate) struct ParsedProject {
    pub project: Project,
    pub tasks: Vec<Task>,
    pub problems: Problems,
}

pub(crate) fn parse_project(slug: &str, rel: &Path, doc: &Doc) -> ParsedProject {
    let o = Outline::of(doc);
    let mut problems = Problems::new();
    common_problems(&o, &mut problems);
    let fm = Frontmatter::of(doc, &o);
    duplicate_keys(fm.as_ref(), PROJECT_KEYS, &mut problems);
    let name =
        fm_scalar(doc, fm.as_ref(), "nazwa", &mut problems).unwrap_or_else(|| slug.to_string());
    let color = fm_scalar(doc, fm.as_ref(), "kolor", &mut problems);
    let state = match fm_scalar(doc, fm.as_ref(), "stan", &mut problems) {
        None => ProjectState::Active,
        Some(s) => ProjectState::from_file_value(&s).unwrap_or_else(|| {
            let line = fm.as_ref().and_then(|f| f.entry("stan")).map(|e| e.line);
            problems.push((
                line,
                format!("unknown project state `{s}`; treated as `aktywny`"),
            ));
            ProjectState::Active
        }),
    };
    let repos = match fm.as_ref() {
        Some(f) => {
            let (items, p) = parse_repos(doc, f);
            problems.extend(lift(p));
            repos_from_items(&items)
        }
        None => Vec::new(),
    };
    let description = joined_trimmed(doc, text_region(&o));
    let (blocks, p) = task_blocks(doc, FileKind::Project);
    problems.extend(lift(p));
    let loc = TaskLocation::Project {
        project: slug.to_string(),
    };
    ParsedProject {
        project: Project {
            slug: slug.to_string(),
            name,
            color,
            state,
            repos,
            description,
            path: rel.to_path_buf(),
        },
        tasks: blocks.iter().map(|b| b.to_task(loc.clone())).collect(),
        problems,
    }
}

pub(crate) struct ParsedTopic {
    pub topic: Topic,
    pub tasks: Vec<Task>,
    pub journal: Vec<JournalEntry>,
    pub problems: Problems,
}

pub(crate) fn parse_topic(project: &str, slug: &str, rel: &Path, doc: &Doc) -> ParsedTopic {
    let o = Outline::of(doc);
    let mut problems = Problems::new();
    common_problems(&o, &mut problems);
    let fm = Frontmatter::of(doc, &o);
    duplicate_keys(fm.as_ref(), TOPIC_KEYS, &mut problems);
    let status = match fm_scalar(doc, fm.as_ref(), "status", &mut problems) {
        None => TopicStatus::Planned,
        Some(s) => TopicStatus::from_file_value(&s).unwrap_or_else(|| {
            let line = fm.as_ref().and_then(|f| f.entry("status")).map(|e| e.line);
            problems.push((
                line,
                format!("unknown topic status `{s}`; treated as `planowany`"),
            ));
            TopicStatus::Planned
        }),
    };
    let repo = fm_scalar(doc, fm.as_ref(), "repo", &mut problems);
    let created = fm_scalar(doc, fm.as_ref(), "utworzono", &mut problems).and_then(|s| {
        let d = parse_date_strict(s.get(..10).unwrap_or(&s));
        if d.is_none() {
            problems.push((None, format!("invalid `utworzono` date `{s}`")));
        }
        d
    });
    let updated = fm_scalar(doc, fm.as_ref(), UPDATED_KEY, &mut problems).and_then(|s| {
        let d = minute::parse(&s);
        if d.is_none() {
            problems.push((None, format!("invalid `{UPDATED_KEY}` timestamp `{s}`")));
        }
        d
    });
    let title =
        o.h1.and_then(|h| heading_level_1(doc.text(h)))
            .filter(|t| !t.is_empty())
            .map_or_else(|| slug.to_string(), str::to_string);
    let summary = joined_trimmed(doc, text_region(&o));
    let (blocks, p) = task_blocks(doc, FileKind::Topic);
    problems.extend(lift(p));
    let (entries, p) = journal_lines(doc);
    problems.extend(lift(p));
    let loc = TaskLocation::Topic {
        project: project.to_string(),
        topic: slug.to_string(),
    };
    ParsedTopic {
        topic: Topic {
            project: project.to_string(),
            slug: slug.to_string(),
            title,
            status,
            repo,
            summary,
            created,
            updated,
            path: rel.to_path_buf(),
        },
        tasks: blocks.iter().map(|b| b.to_task(loc.clone())).collect(),
        journal: entries
            .into_iter()
            .map(|e| JournalEntry {
                project: project.to_string(),
                topic: slug.to_string(),
                at: e.at,
                author: e.author,
                text: e.text,
                line: e.line + 1,
            })
            .collect(),
        problems,
    }
}

pub(crate) fn parse_inbox(doc: &Doc) -> (Vec<Task>, Problems) {
    let o = Outline::of(doc);
    let mut problems = Problems::new();
    common_problems(&o, &mut problems);
    let (blocks, p) = task_blocks(doc, FileKind::Inbox);
    problems.extend(lift(p));
    (
        blocks
            .iter()
            .map(|b| b.to_task(TaskLocation::Inbox))
            .collect(),
        problems,
    )
}

// ---------------------------------------------------------------------------
// Structural edits
// ---------------------------------------------------------------------------

/// Insert task lines (a task line plus its continuation) at the end of the file's
/// task list, creating a `## Zadania` section if needed.
pub(crate) fn insert_task_lines(doc: &mut Doc, kind: FileKind, lines: Vec<String>) {
    let (blocks, _) = task_blocks(doc, kind);
    if let Some(last) = blocks.iter().map(|b| b.end).max() {
        doc.insert(last, lines);
        return;
    }
    let o = Outline::of(doc);
    if let Some(s) = o.section(TASKS_SECTION) {
        doc.insert(s.heading + 1, lines);
        return;
    }
    if kind == FileKind::Inbox {
        let pre_end = o.first_section_start();
        let mut at = pre_end;
        while at > o.body_start && doc.is_blank(at - 1) {
            at -= 1;
        }
        let mut lines = lines;
        if at == pre_end && pre_end < o.len {
            // directly before a `##` heading: keep a blank line between
            lines.push(String::new());
        }
        doc.insert(at, lines);
        return;
    }
    // Topic / project without a tasks section: add one (before `## Dziennik` in
    // topics, otherwise at the end of the file).
    let journal = if kind == FileKind::Topic {
        o.section(JOURNAL_SECTION).map(|s| s.heading)
    } else {
        None
    };
    let at = journal.unwrap_or(o.len);
    let mut block = Vec::new();
    if at > 0 && !doc.is_blank(at - 1) {
        block.push(String::new());
    }
    block.push(format!("## {TASKS_SECTION}"));
    block.extend(lines);
    if journal.is_some() {
        block.push(String::new());
    }
    doc.insert(at, block);
}

/// Insert journal lines as the newest entry (top of `## Dziennik`).
pub(crate) fn insert_journal_lines(doc: &mut Doc, lines: Vec<String>) {
    let (entries, _) = journal_lines(doc);
    if let Some(first) = entries.first() {
        doc.insert(first.line, lines);
        return;
    }
    let o = Outline::of(doc);
    if let Some(s) = o.section(JOURNAL_SECTION) {
        doc.insert(s.heading + 1, lines);
        return;
    }
    let mut block = Vec::new();
    if o.len > 0 && !doc.is_blank(o.len - 1) {
        block.push(String::new());
    }
    block.push(format!("## {JOURNAL_SECTION}"));
    block.extend(lines);
    doc.insert(o.len, block);
}

/// Replace the free text after the H1 (topic summary / project description).
pub(crate) fn set_text_region(doc: &mut Doc, text: &str) {
    let o = Outline::of(doc);
    let region = text_region(&o);
    let section_follows = region.end < o.len;
    let text = text.trim();
    let mut new = Vec::new();
    if !text.is_empty() {
        if o.h1.is_some() {
            new.push(String::new());
        }
        new.extend(text.lines().map(|l| l.trim_end().to_string()));
    }
    if section_follows && (!new.is_empty() || o.h1.is_some()) {
        new.push(String::new());
    }
    doc.splice_min(region, new);
}

/// Set the `# H1` title (inserting one after the frontmatter if missing).
pub(crate) fn set_h1(doc: &mut Doc, title: &str) {
    let o = Outline::of(doc);
    let line = format!("# {}", title.trim());
    match o.h1 {
        Some(h) => {
            if doc.text(h) != line {
                doc.set_text(h, line);
            }
        }
        None => {
            let mut lines = vec![line];
            if o.body_start < o.len && !doc.is_blank(o.body_start) {
                lines.push(String::new());
            }
            doc.insert(o.body_start, lines);
        }
    }
}

/// Update `zaktualizowano`: always for topics, only if already present elsewhere.
pub(crate) fn touch_updated(doc: &mut Doc, kind: FileKind, now: &NaiveDateTime) {
    let present = {
        let o = Outline::of(doc);
        Frontmatter::of(doc, &o).is_some_and(|f| f.entry(UPDATED_KEY).is_some())
    };
    if kind == FileKind::Topic || present {
        set_scalar(
            doc,
            UPDATED_KEY,
            Some(&minute::format(now)),
            keys_before(TOPIC_KEYS, UPDATED_KEY),
        );
    }
}

/// The raw `zaktualizowano` value of `doc`, if any.
pub(crate) fn updated_value(doc: &Doc) -> Option<String> {
    let o = Outline::of(doc);
    Frontmatter::of(doc, &o).and_then(|f| f.scalar(doc, UPDATED_KEY).ok().flatten())
}

/// Sets `zaktualizowano` to `value` (an undo putting back the old value).
pub(crate) fn set_updated(doc: &mut Doc, value: &str) {
    set_scalar(
        doc,
        UPDATED_KEY,
        Some(value),
        keys_before(TOPIC_KEYS, UPDATED_KEY),
    );
}

pub(crate) fn date_str(d: &NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_task_variants() {
        let mut d = Doc::parse("# T\n\n## Zadania\n\n## Dziennik\n");
        insert_task_lines(&mut d, FileKind::Topic, vec!["- [ ] a".into()]);
        assert_eq!(d.render(), "# T\n\n## Zadania\n- [ ] a\n\n## Dziennik\n");
        insert_task_lines(&mut d, FileKind::Topic, vec!["- [ ] b".into()]);
        assert_eq!(
            d.render(),
            "# T\n\n## Zadania\n- [ ] a\n- [ ] b\n\n## Dziennik\n"
        );

        let mut d = Doc::parse("# T\n\n## Dziennik\n- 2026-01-01 10:00 · ja · x\n");
        insert_task_lines(&mut d, FileKind::Topic, vec!["- [ ] a".into()]);
        assert_eq!(
            d.render(),
            "# T\n\n## Zadania\n- [ ] a\n\n## Dziennik\n- 2026-01-01 10:00 · ja · x\n"
        );

        let mut d = Doc::parse("---\nnazwa: X\n---\nOpis");
        insert_task_lines(&mut d, FileKind::Project, vec!["- [ ] a".into()]);
        assert_eq!(
            d.render(),
            "---\nnazwa: X\n---\nOpis\n\n## Zadania\n- [ ] a"
        );

        let mut d = Doc::parse("");
        insert_task_lines(&mut d, FileKind::Inbox, vec!["- [ ] a".into()]);
        assert_eq!(d.render(), "- [ ] a\n");

        let mut d = Doc::parse("---\nx: 1\n---\n\n## Inne\ntext\n");
        insert_task_lines(&mut d, FileKind::Inbox, vec!["- [ ] a".into()]);
        assert_eq!(d.render(), "---\nx: 1\n---\n- [ ] a\n\n## Inne\ntext\n");
    }

    #[test]
    fn insert_journal_variants() {
        let mut d = Doc::parse("# T\n## Dziennik\n- 2026-01-01 10:00 · ja · old\n");
        insert_journal_lines(&mut d, vec!["- new".into()]);
        assert_eq!(
            d.render(),
            "# T\n## Dziennik\n- new\n- 2026-01-01 10:00 · ja · old\n"
        );
        let mut d = Doc::parse("# T\n");
        insert_journal_lines(&mut d, vec!["- new".into()]);
        assert_eq!(d.render(), "# T\n\n## Dziennik\n- new\n");
    }

    #[test]
    fn text_region_edits() {
        let mut d = Doc::parse("---\na: 1\n---\n# T\n\nold\n\n## Zadania\n");
        set_text_region(&mut d, "new\nlines");
        assert_eq!(
            d.render(),
            "---\na: 1\n---\n# T\n\nnew\nlines\n\n## Zadania\n"
        );
        set_text_region(&mut d, "");
        assert_eq!(d.render(), "---\na: 1\n---\n# T\n\n## Zadania\n");
        let mut d = Doc::parse("---\na: 1\n---\n\n## Zadania\n");
        set_text_region(&mut d, "Opis");
        assert_eq!(d.render(), "---\na: 1\n---\nOpis\n\n## Zadania\n");
        let mut d = Doc::parse("# T");
        set_text_region(&mut d, "s");
        assert_eq!(d.render(), "# T\n\ns");
    }

    #[test]
    fn h1_edits() {
        let mut d = Doc::parse("---\na: 1\n---\n# Old\n");
        set_h1(&mut d, "New");
        assert_eq!(d.render(), "---\na: 1\n---\n# New\n");
        let mut d = Doc::parse("---\na: 1\n---\ntext\n");
        set_h1(&mut d, "New");
        assert_eq!(d.render(), "---\na: 1\n---\n# New\n\ntext\n");
    }
}
