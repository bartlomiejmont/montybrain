//! Pre-flight checks: the same free-text validation a mutation runs before it
//! takes the lock, without writing anything. For callers that want to reject
//! untrusted text early (e.g. an agent's proposed changes shown for review)
//! and still have the mutation validate and verify it again when it runs.
//!
//! Each function returns the value as the mutation would store it, or the same
//! [`Error::InvalidInput`](crate::Error::InvalidInput) the mutation would fail with.

use crate::error::{Error, Result};
use crate::model::{Priority, TaskStatus};
use crate::task::TaskLine;
use crate::validate::{Multi, multiline, one_line};

/// A task title as `create_task` / `update_task` take it: collapsed to one
/// line, not empty, and reading back as the same title (a title ending in
/// something that looks like a token is rejected).
pub fn task_title(title: &str) -> Result<String> {
    let title = one_line("title", title)?;
    let line = TaskLine {
        bullet: '-',
        status: TaskStatus::Todo,
        title: title.clone(),
        priority: Priority::Normal,
        due: None,
        done: None,
        id: Some("aaaaaa".into()),
        problems: Vec::new(),
    };
    crate::vault::render_checked(&line)?;
    Ok(title)
}

/// A topic title as `create_topic` takes it: collapsed to one line, not
/// empty, without `[` or `]`.
pub fn topic_title(title: &str) -> Result<String> {
    crate::validate::topic_title_line("title", title)
}

/// The text of a journal entry (`add_journal_entry`): multi-line, written
/// indented below the entry, not empty.
pub fn journal_text(text: &str) -> Result<String> {
    let text = multiline("text", text, Multi::Indented)?;
    if text.trim().is_empty() {
        return Err(Error::invalid("text: journal text must not be empty"));
    }
    Ok(text)
}

/// A topic summary (`update_topic`, `add_journal_entry`): no headings that
/// would start a section, no leading `---`, no unclosed code fence.
pub fn summary(text: &str) -> Result<String> {
    multiline("summary", text, Multi::Region)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_rules_as_the_mutations() {
        assert_eq!(task_title("  Zrobić \n MR ").unwrap(), "Zrobić MR");
        assert!(task_title(" ").is_err());
        assert!(task_title("a\u{1b}b").is_err());
        // Would read back as a due date / priority / id token.
        for t in ["Wysłać @2026-11-20", "Pilne !wysoki", "x ✓2026-11-01"] {
            assert!(task_title(t).is_err(), "{t}");
        }
        // Not the last token, so it stays title text (as in `create_task`).
        assert_eq!(task_title("Coś ^abcdef").unwrap(), "Coś ^abcdef");
        assert_eq!(journal_text("a\r\nb").unwrap(), "a\nb");
        assert!(journal_text(" \n ").is_err());
        assert!(journal_text("a\rb").is_err());
        // Indented text may look like structure: it cannot escape its block.
        assert!(journal_text("## Zadania\n- [ ] x ^aaaaaa").is_ok());
        assert!(summary("ok\n### niżej").is_ok());
        for bad in ["a\n## Zadania", "# Tytuł", "---\nx: 1", "a\n```\nb"] {
            assert!(summary(bad).is_err(), "{bad:?}");
        }
    }
}
