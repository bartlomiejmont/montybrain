//! Defence in depth: before a mutation writes a file, the would-be contents are
//! parsed again and their structure is compared with the original. The change must
//! be exactly what the operation intended (for example: one task more, nothing else
//! structural changed); otherwise the write is refused.

use crate::doc::{Doc, Outline, heading_level_1};
use crate::files::{FileKind, PROJECT_KEYS, TOPIC_KEYS, UPDATED_KEY, journal_lines, task_blocks};
use crate::frontmatter::Frontmatter;

/// The structural skeleton of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Shape {
    /// Top-level frontmatter keys in order; `None` if there is no frontmatter.
    fm_keys: Option<Vec<String>>,
    /// The `# H1` text before the first section.
    h1: Option<String>,
    /// `##` section names, trimmed and lowercased.
    sections: Vec<String>,
    /// Task ids in file order.
    tasks: Vec<Option<String>>,
    /// Number of journal entries.
    journal: usize,
}

impl Shape {
    pub fn of(text: &str, kind: FileKind) -> Shape {
        let doc = Doc::parse(text);
        let o = Outline::of(&doc);
        Shape {
            fm_keys: Frontmatter::of(&doc, &o)
                .map(|f| f.entries.into_iter().map(|e| e.key).collect()),
            h1: o
                .h1
                .and_then(|h| heading_level_1(doc.text(h)).map(str::to_string)),
            sections: o.sections.iter().map(|s| s.name.to_lowercase()).collect(),
            tasks: task_blocks(&doc, kind)
                .0
                .into_iter()
                .map(|b| b.parsed.id)
                .collect(),
            journal: journal_lines(&doc).0.len(),
        }
    }
}

/// What an operation is allowed to change, beyond MontyBrain's own frontmatter keys
/// and adding a missing `## Zadania` / `## Dziennik` section.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Expect<'a> {
    /// Exactly this task is added.
    pub add_task: Option<&'a str>,
    /// Exactly these tasks are added at the end, in this order (a new file).
    pub add_tasks: &'a [String],
    /// Exactly this task is removed.
    pub remove_task: Option<&'a str>,
    /// Exactly one journal entry is added.
    pub add_journal: bool,
    /// Exactly one journal entry is removed.
    pub remove_journal: bool,
    /// The `# H1` title may change (it must still exist).
    pub title: bool,
    /// Task ids may change, but not the number of tasks (`normalize`).
    pub renumber: bool,
}

fn own_keys(kind: FileKind) -> &'static [&'static str] {
    match kind {
        FileKind::Project => PROJECT_KEYS,
        FileKind::Topic => TOPIC_KEYS,
        FileKind::Inbox => &[],
    }
}

/// `after` is `before` with exactly one `Some(id)` inserted.
fn is_insertion(before: &[Option<String>], after: &[Option<String>], id: &str) -> bool {
    after.len() == before.len() + 1
        && (0..after.len()).any(|k| {
            after[k].as_deref() == Some(id)
                && after[..k].iter().chain(&after[k + 1..]).eq(before.iter())
        })
}

/// Compare the structure of `original` and `new`. `Err` describes the unexpected
/// difference.
pub(crate) fn check(
    original: &str,
    new: &str,
    kind: FileKind,
    e: &Expect<'_>,
) -> Result<(), String> {
    let before = Shape::of(original, kind);
    let after = Shape::of(new, kind);
    let own = |k: &String| own_keys(kind).contains(&k.as_str()) || k == UPDATED_KEY;

    // Frontmatter: only MontyBrain's keys may come or go; other keys stay in order.
    match (&before.fm_keys, &after.fm_keys) {
        (Some(_), None) => return Err("the frontmatter would disappear".into()),
        (b, Some(a)) => {
            let other_before: Vec<&String> = b.iter().flatten().filter(|k| !own(k)).collect();
            let other_after: Vec<&String> = a.iter().filter(|k| !own(k)).collect();
            if other_before != other_after {
                return Err(format!(
                    "the frontmatter keys would change from {other_before:?} to {other_after:?}"
                ));
            }
        }
        (None, None) => {}
    }

    // Title.
    if e.title {
        if after.h1.is_none() {
            return Err("the `# ` title would disappear".into());
        }
    } else if before.h1 != after.h1 {
        return Err(format!(
            "the `# ` title would change from {:?} to {:?}",
            before.h1, after.h1
        ));
    }

    // Sections: existing ones stay in order; only missing known sections are added.
    let mut it = before.sections.iter().peekable();
    for s in &after.sections {
        if it.peek() == Some(&s) {
            it.next();
        } else if s != "zadania" && s != "dziennik" {
            return Err(format!("a `## {s}` section would appear"));
        }
    }
    if let Some(s) = it.next() {
        return Err(format!("the `## {s}` section would disappear"));
    }

    // Tasks.
    let mut expected = before.tasks.clone();
    if let Some(id) = e.remove_task {
        match expected.iter().position(|t| t.as_deref() == Some(id)) {
            Some(p) => {
                expected.remove(p);
            }
            None => return Err(format!("task ^{id} is not in the file")),
        }
    }
    let tasks_ok = if !e.add_tasks.is_empty() {
        let mut want = expected.clone();
        want.extend(e.add_tasks.iter().cloned().map(Some));
        after.tasks == want
    } else if e.renumber {
        after.tasks.len() == expected.len()
    } else if let Some(id) = e.add_task {
        is_insertion(&expected, &after.tasks, id)
    } else {
        after.tasks == expected
    };
    if !tasks_ok {
        return Err(format!(
            "the task list would change unexpectedly ({} task(s) before, {} after)",
            before.tasks.len(),
            after.tasks.len()
        ));
    }

    // Journal.
    let want = (before.journal + usize::from(e.add_journal))
        .checked_sub(usize::from(e.remove_journal))
        .ok_or("there is no journal entry to remove")?;
    if after.journal != want {
        return Err(format!(
            "the journal would have {} entr(ies) instead of {want}",
            after.journal
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "---\nstatus: w-toku\nx: 1\n---\n# T\n\n## Zadania\n- [ ] a ^aaaaa1\n\n## Dziennik\n- 2026-01-01 10:00 · ja · j\n";

    fn ok(new: &str, e: Expect<'_>) -> Result<(), String> {
        check(T, new, FileKind::Topic, &e)
    }

    #[test]
    fn accepts_intended_changes() {
        assert!(ok(T, Expect::default()).is_ok());
        let added = T.replace("^aaaaa1\n", "^aaaaa1\n- [ ] b ^bbbbb1\n");
        assert!(
            ok(
                &added,
                Expect {
                    add_task: Some("bbbbb1"),
                    ..Default::default()
                }
            )
            .is_ok()
        );
        let removed = T.replace("- [ ] a ^aaaaa1\n", "");
        assert!(
            ok(
                &removed,
                Expect {
                    remove_task: Some("aaaaa1"),
                    ..Default::default()
                }
            )
            .is_ok()
        );
        let touched = T.replace(
            "x: 1\n",
            "x: 1\nzaktualizowano: 2026-01-01T10:00\nrepo: r\n",
        );
        assert!(ok(&touched, Expect::default()).is_ok());
        let journal = T.replace(
            "## Dziennik\n",
            "## Dziennik\n- 2026-01-02 10:00 · ja · k\n",
        );
        assert!(
            ok(
                &journal,
                Expect {
                    add_journal: true,
                    ..Default::default()
                }
            )
            .is_ok()
        );
        let retitled = T.replace("# T\n", "# U\n");
        assert!(
            ok(
                &retitled,
                Expect {
                    title: true,
                    ..Default::default()
                }
            )
            .is_ok()
        );
        let from_empty = "## Zadania\n- [ ] a ^aaaaa1\n";
        assert!(
            check(
                "",
                from_empty,
                FileKind::Project,
                &Expect {
                    add_task: Some("aaaaa1"),
                    ..Default::default()
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_structural_side_effects() {
        // A forged extra task.
        let forged = T.replace("^aaaaa1\n", "^aaaaa1\n- [ ] b ^bbbbb1\n- [ ] forged\n");
        assert!(
            ok(
                &forged,
                Expect {
                    add_task: Some("bbbbb1"),
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert!(ok(&forged, Expect::default()).is_err());
        // A new section.
        let section = T.replace("# T\n", "# T\n## Inne\n");
        assert!(ok(&section, Expect::default()).is_err());
        // A fence swallowing the journal.
        let swallowed = T.replace("\n## Dziennik", "```\n## Dziennik");
        assert!(ok(&swallowed, Expect::default()).is_err());
        // A foreign frontmatter key.
        let key = T.replace("x: 1\n", "x: 1\nevil: 1\n");
        assert!(ok(&key, Expect::default()).is_err());
        // An extra journal entry, or a changed title.
        let journal = T.replace(
            "## Dziennik\n",
            "## Dziennik\n- 2026-01-02 10:00 · ja · k\n",
        );
        assert!(ok(&journal, Expect::default()).is_err());
        assert!(ok(&T.replace("# T\n", "# U\n"), Expect::default()).is_err());
    }
}
