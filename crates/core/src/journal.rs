//! Journal lines: `- YYYY-MM-DD HH:MM · author · text`.

use std::ops::Range;

use chrono::NaiveDateTime;

use crate::doc::Doc;
use crate::model::{parse_date_strict, parse_time_strict};

pub(crate) const SEP: char = '·';

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JournalLine {
    pub line: usize,
    pub end: usize,
    pub at: NaiveDateTime,
    pub author: String,
    pub text: String,
}

/// `None` = not a list item; `Some(Err)` = a list item that is not a valid entry.
pub(crate) fn parse_entry_line(
    text: &str,
) -> Option<Result<(NaiveDateTime, String, String), String>> {
    let rest = text
        .strip_prefix("- ")
        .or_else(|| text.strip_prefix("* "))?;
    let rest = rest.trim_start();
    let bad = || {
        Some(Err(
            "journal entry must look like `- YYYY-MM-DD HH:MM · author · text`".to_string(),
        ))
    };
    let (Some(date), Some(sep), Some(time)) = (rest.get(..10), rest.get(10..11), rest.get(11..16))
    else {
        return bad();
    };
    let (Some(date), Some(time)) = (parse_date_strict(date), parse_time_strict(time)) else {
        return bad();
    };
    if sep != " " && sep != "T" {
        return bad();
    }
    let tail = rest[16..].trim();
    let Some(tail) = tail.strip_prefix(SEP) else {
        if tail.is_empty() {
            return Some(Ok((date.and_time(time), String::new(), String::new())));
        }
        return bad();
    };
    let (author, body) = match tail.split_once(SEP) {
        Some((a, b)) => (a.trim(), b.trim()),
        None => ("", tail.trim()),
    };
    Some(Ok((
        date.and_time(time),
        author.to_string(),
        body.to_string(),
    )))
}

/// Scan a `## Dziennik` section body. `code` marks lines in fenced code blocks
/// ([`crate::doc::Outline::code`]); they are skipped.
pub(crate) fn scan_journal(
    doc: &Doc,
    range: Range<usize>,
    code: &[bool],
) -> (Vec<JournalLine>, Vec<(usize, String)>) {
    let mut out = Vec::new();
    let mut problems = Vec::new();
    let mut i = range.start;
    while i < range.end {
        if code[i] || doc.is_indented(i) || doc.is_blank(i) {
            i += 1;
            continue;
        }
        let t = doc.text(i);
        match parse_entry_line(t) {
            None => i += 1,
            Some(Err(msg)) => {
                problems.push((i, msg));
                i += 1;
            }
            Some(Ok((at, author, text))) => {
                let end = doc.continuation_end(i + 1, range.end);
                let mut full = text;
                for k in i + 1..end {
                    full.push('\n');
                    full.push_str(doc.text(k).trim());
                }
                out.push(JournalLine {
                    line: i,
                    end,
                    at,
                    author,
                    text: full.trim_end().to_string(),
                });
                i = end.max(i + 1);
            }
        }
    }
    (out, problems)
}

pub(crate) fn clean_author(author: &str) -> String {
    let a: String = author
        .chars()
        .map(|c| if c == SEP || c.is_control() { ' ' } else { c })
        .collect();
    let a = a.split_whitespace().collect::<Vec<_>>().join(" ");
    if a.is_empty() { "ja".to_string() } else { a }
}

/// Render an entry: first line plus indented continuation lines.
pub(crate) fn render_entry(at: &NaiveDateTime, author: &str, text: &str) -> Vec<String> {
    let mut lines = text.trim().lines();
    let first = lines.next().unwrap_or("").trim();
    let mut out = vec![
        format!(
            "- {} {SEP} {} {SEP} {}",
            at.format("%Y-%m-%d %H:%M"),
            author,
            first
        )
        .trim_end()
        .to_string(),
    ];
    for l in lines {
        if l.trim().is_empty() {
            out.push(String::new());
        } else {
            out.push(format!("  {}", l.trim()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_entries() {
        let (at, a, t) = parse_entry_line("- 2026-11-13 14:02 · agent · Tests green, MR opened.")
            .unwrap()
            .unwrap();
        assert_eq!(at.to_string(), "2026-11-13 14:02:00");
        assert_eq!(a, "agent");
        assert_eq!(t, "Tests green, MR opened.");
        let (_, a, t) = parse_entry_line("-   2026-11-13 14:02·ja·x · y")
            .unwrap()
            .unwrap();
        assert_eq!((a.as_str(), t.as_str()), ("ja", "x · y"));
        let (_, a, t) = parse_entry_line("- 2026-11-13 14:02 · only text")
            .unwrap()
            .unwrap();
        assert_eq!((a.as_str(), t.as_str()), ("", "only text"));
        assert!(parse_entry_line("- yesterday · ja · x").unwrap().is_err());
        assert!(
            parse_entry_line("- 2026-11-13 25:02 · ja · x")
                .unwrap()
                .is_err()
        );
        assert!(
            parse_entry_line("- 2026-11-13 14:02 ja x")
                .unwrap()
                .is_err()
        );
        assert!(parse_entry_line("plain").is_none());
    }

    #[test]
    fn scan_with_continuation() {
        let doc = Doc::parse(
            "- 2026-11-13 14:02 · ja · a\n  more\n- bad\n\n- 2026-11-12 09:30 · agent · b\n",
        );
        let (entries, problems) = scan_journal(&doc, 0..doc.len(), &vec![false; doc.len()]);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "a\nmore");
        assert_eq!(entries[0].end, 2);
        assert_eq!(problems.len(), 1);
    }

    #[test]
    fn render() {
        let at = NaiveDateTime::parse_from_str("2026-11-13 14:02", "%Y-%m-%d %H:%M").unwrap();
        assert_eq!(
            render_entry(&at, "ja", "x\n\ny"),
            vec!["- 2026-11-13 14:02 · ja · x", "", "  y"]
        );
        assert_eq!(clean_author(" a·b \n"), "a b");
        assert_eq!(clean_author(""), "ja");
    }
}
