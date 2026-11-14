//! Task lines (`- [ ] title !prio @due ✓done ^id`) and their indented blocks.

use std::ops::Range;

use chrono::NaiveDate;

use crate::doc::Doc;
#[cfg(test)]
use crate::doc::Outline;
use crate::id::is_valid_id;
use crate::model::{Due, Priority, Task, TaskLocation, TaskStatus, parse_date_strict};

pub(crate) const DONE_MARK: char = '✓';

/// A parsed task line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskLine {
    pub bullet: char,
    pub status: TaskStatus,
    pub title: String,
    pub priority: Priority,
    pub due: Option<Due>,
    pub done: Option<NaiveDate>,
    pub id: Option<String>,
    /// Non-fatal token problems (e.g. `@2026-02-30`), the text stays in the title.
    pub problems: Vec<String>,
}

impl TaskLine {
    pub fn render(&self) -> String {
        let mut out = format!("{} [{}]", self.bullet, self.status.checkbox());
        let mut push = |s: &str| {
            out.push(' ');
            out.push_str(s);
        };
        if !self.title.is_empty() {
            push(&self.title);
        }
        if let Some(p) = self.priority.token() {
            push(&format!("!{p}"));
        }
        if let Some(d) = &self.due {
            push(&format!("@{d}"));
        }
        if let Some(d) = &self.done {
            push(&format!("{DONE_MARK}{}", d.format("%Y-%m-%d")));
        }
        if let Some(id) = &self.id {
            push(&format!("^{id}"));
        }
        out
    }

    /// Same title and tokens, i.e. only the checkbox may differ.
    fn same_tokens(&self, other: &TaskLine) -> bool {
        self.title == other.title
            && self.priority == other.priority
            && self.due == other.due
            && self.done == other.done
            && self.id == other.id
    }

    /// Compute the new text for a task line, touching as little as possible:
    /// a pure status change only swaps the checkbox character.
    pub fn rewrite(original_text: &str, old: &TaskLine, new: &TaskLine) -> String {
        if old == new {
            return original_text.to_string();
        }
        if old.same_tokens(new) && old.bullet == new.bullet {
            let mut s = String::with_capacity(original_text.len());
            s.push_str(&original_text[..3]);
            s.push(new.status.checkbox());
            let cb_len = original_text[3..].chars().next().map_or(0, char::len_utf8);
            s.push_str(&original_text[3 + cb_len..]);
            return s;
        }
        new.render()
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Tok {
    Priority(Priority),
    Due(Due),
    Done(NaiveDate),
    Id,
}

/// Classify one whitespace-separated word. `Ok(None)` = not a token,
/// `Err` = looks like a token but is invalid.
fn classify(word: &str) -> Result<Option<Tok>, String> {
    if let Some(rest) = word.strip_prefix('^') {
        if is_valid_id(rest) {
            return Ok(Some(Tok::Id));
        }
        // Only id-like words (4–8 alphanumerics) are reported; anything else is
        // ordinary title text.
        if (4..=8).contains(&rest.len()) && rest.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(format!(
                "invalid task id `{word}` (expected 6 lowercase base36 chars)"
            ));
        }
        return Ok(None);
    }
    if let Some(rest) = word.strip_prefix('!') {
        return Ok(match rest {
            "wysoki" => Some(Tok::Priority(Priority::High)),
            "niski" => Some(Tok::Priority(Priority::Low)),
            "normalny" => Some(Tok::Priority(Priority::Normal)),
            _ => None,
        });
    }
    if let Some(rest) = word.strip_prefix('@') {
        let date_like = rest.len() >= 5
            && rest.as_bytes()[..4].iter().all(u8::is_ascii_digit)
            && rest.as_bytes()[4] == b'-';
        if date_like {
            return rest.parse::<Due>().map(|d| Some(Tok::Due(d))).map_err(|_| {
                format!("invalid due date `{word}` (expected @YYYY-MM-DD or @YYYY-MM-DDTHH:MM)")
            });
        }
        return Ok(None);
    }
    if let Some(rest) = word.strip_prefix(DONE_MARK) {
        return parse_date_strict(rest)
            .map(|d| Some(Tok::Done(d)))
            .ok_or_else(|| format!("invalid completion date `{word}` (expected ✓YYYY-MM-DD)"));
    }
    Ok(None)
}

/// Parse a task line. `None` = not a checkbox item at all; `Some(Err)` = looks like
/// one (`- [`) but is malformed.
pub(crate) fn parse_task_line(text: &str) -> Option<Result<TaskLine, String>> {
    let mut chars = text.chars();
    let bullet = chars.next()?;
    if !matches!(bullet, '-' | '*' | '+') {
        return None;
    }
    let after_bullet = &text[1..];
    let rest = after_bullet.strip_prefix(' ')?;
    let rest = rest.strip_prefix('[')?;
    let mut rc = rest.chars();
    let cb = match rc.next() {
        Some(c) => c,
        None => return Some(Err("unterminated checkbox".into())),
    };
    if cb == ']' {
        return Some(Err("empty checkbox `[]` (use `[ ]`)".into()));
    }
    let after_cb = &rest[cb.len_utf8()..];
    let Some(after) = after_cb.strip_prefix(']') else {
        return Some(Err("malformed checkbox".into()));
    };
    let Some(status) = TaskStatus::from_checkbox(cb) else {
        return Some(Err(format!(
            "unknown task status `[{cb}]` (use [ ], [/], [x] or [!])"
        )));
    };
    if !after.is_empty() && !after.starts_with([' ', '\t']) {
        return Some(Err("missing space after checkbox".into()));
    }
    let body = after.trim();

    // Tokens are taken from the end of the line, right to left; the first word that
    // is not a (new kind of) token ends the token run.
    let mut priority = Priority::Normal;
    let mut due = None;
    let mut done = None;
    let mut id = None;
    let mut seen: Vec<std::mem::Discriminant<Tok>> = Vec::new();
    let mut problems = Vec::new();
    let mut title_end = body.len();
    loop {
        let head = body[..title_end].trim_end();
        let start = head.rfind([' ', '\t']).map_or(0, |i| i + 1);
        let word = &head[start..];
        if word.is_empty() {
            title_end = head.len();
            break;
        }
        match classify(word) {
            Ok(Some(tok)) if !seen.contains(&std::mem::discriminant(&tok)) => {
                seen.push(std::mem::discriminant(&tok));
                match tok {
                    Tok::Priority(p) => priority = p,
                    Tok::Due(d) => due = Some(d),
                    Tok::Done(d) => done = Some(d),
                    Tok::Id => id = Some(word[1..].to_string()),
                }
                title_end = start;
            }
            Ok(_) => {
                title_end = head.len();
                break;
            }
            Err(msg) => {
                problems.push(msg);
                title_end = head.len();
                break;
            }
        }
    }
    Some(Ok(TaskLine {
        bullet,
        status,
        title: body[..title_end].trim_end().to_string(),
        priority,
        due,
        done,
        id,
        problems,
    }))
}

// ---------------------------------------------------------------------------
// Blocks
// ---------------------------------------------------------------------------

/// True if `s` is a bare URL (`scheme://…` without whitespace).
pub(crate) fn is_url(s: &str) -> bool {
    let Some(pos) = s.find("://") else {
        return false;
    };
    let scheme = &s[..pos];
    !scheme.is_empty()
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        && s.len() > pos + 3
        && !s.contains(char::is_whitespace)
}

/// The URL on a continuation line, if the line is a link line.
fn line_link(text: &str) -> Option<&str> {
    let t = text.trim();
    let t = t
        .strip_prefix("- ")
        .or_else(|| t.strip_prefix("* "))
        .or_else(|| t.strip_prefix("+ "))
        .map_or(t, str::trim);
    is_url(t).then_some(t)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskBlock {
    /// Line index of the task line.
    pub line: usize,
    /// Exclusive end of the block (task line + continuation).
    pub end: usize,
    pub parsed: TaskLine,
    pub links: Vec<String>,
    pub notes: String,
    /// Indentation of the first continuation line (`"  "` if none).
    pub base_indent: String,
}

impl TaskBlock {
    pub fn range(&self) -> Range<usize> {
        self.line..self.end
    }

    pub fn to_task(&self, location: TaskLocation) -> Task {
        Task {
            id: self.parsed.id.clone(),
            title: self.parsed.title.clone(),
            status: self.parsed.status,
            priority: self.parsed.priority,
            due: self.parsed.due,
            done: self.parsed.done,
            links: self.links.clone(),
            notes: self.notes.clone(),
            location,
            line: self.line + 1,
        }
    }
}

fn note_text<'a>(text: &'a str, base: &str) -> &'a str {
    if text.trim().is_empty() {
        return "";
    }
    text.strip_prefix(base).unwrap_or_else(|| text.trim_start())
}

fn trim_blank_lines(lines: &[&str]) -> String {
    let start = lines.iter().position(|l| !l.trim().is_empty());
    let end = lines.iter().rposition(|l| !l.trim().is_empty());
    match (start, end) {
        (Some(s), Some(e)) => lines[s..=e].join("\n"),
        _ => String::new(),
    }
}

/// Scan `range` for task lines. `code` is [`crate::doc::Outline::code`]: lines in
/// fenced code blocks are skipped. A task line starts in column 0; indented lines
/// are never tasks (they belong to the block of the item above). Returns the task
/// blocks and problems `(line index, message)` for malformed checkbox lines /
/// invalid tokens.
pub(crate) fn scan_tasks(
    doc: &Doc,
    range: Range<usize>,
    code: &[bool],
) -> (Vec<TaskBlock>, Vec<(usize, String)>) {
    let mut blocks = Vec::new();
    let mut problems = Vec::new();
    let mut i = range.start;
    while i < range.end {
        let t = doc.text(i);
        if code[i] || doc.is_indented(i) {
            i += 1;
            continue;
        }
        match parse_task_line(t) {
            None => i += 1,
            Some(Err(msg)) => {
                problems.push((i, format!("malformed task line ignored: {msg}")));
                i = doc.continuation_end(i + 1, range.end).max(i + 1);
            }
            Some(Ok(parsed)) => {
                for p in &parsed.problems {
                    problems.push((i, p.clone()));
                }
                let end = doc.continuation_end(i + 1, range.end);
                let base_indent = (i + 1..end)
                    .find(|&k| !doc.is_blank(k))
                    .map(|k| {
                        let tk = doc.text(k);
                        tk[..tk.len() - tk.trim_start().len()].to_string()
                    })
                    .unwrap_or_else(|| "  ".to_string());
                let mut links = Vec::new();
                let mut notes: Vec<&str> = Vec::new();
                for k in i + 1..end {
                    let tk = doc.text(k);
                    match line_link(tk) {
                        Some(url) => links.push(url.to_string()),
                        None => notes.push(note_text(tk, &base_indent)),
                    }
                }
                blocks.push(TaskBlock {
                    line: i,
                    end,
                    parsed,
                    links,
                    notes: trim_blank_lines(&notes),
                    base_indent,
                });
                i = end.max(i + 1);
            }
        }
    }
    (blocks, problems)
}

/// Rewrite the continuation lines of `block`: remove/add links and optionally
/// replace all notes. Link lines and their order are otherwise preserved.
pub(crate) fn rewrite_continuation(
    doc: &mut Doc,
    block: &TaskBlock,
    add_links: &[String],
    remove_links: &[String],
    notes: Option<&str>,
) {
    let base = block.base_indent.clone();
    let old: Vec<String> = (block.line + 1..block.end)
        .map(|k| doc.text(k).to_string())
        .collect();
    let mut new: Vec<String> = Vec::with_capacity(old.len());
    let mut notes_at: Option<usize> = None;
    for line in &old {
        match line_link(line) {
            Some(url) if remove_links.iter().any(|r| r.trim() == url) => {}
            Some(_) => new.push(line.clone()),
            None if notes.is_some() => {
                if notes_at.is_none() && !line.trim().is_empty() {
                    notes_at = Some(new.len());
                }
            }
            None => new.push(line.clone()),
        }
    }
    if let Some(text) = notes {
        let text_lines: Vec<&str> = text.lines().collect();
        let trimmed = trim_blank_lines(&text_lines);
        let note_lines: Vec<String> = if trimmed.is_empty() {
            Vec::new()
        } else {
            trimmed
                .lines()
                .map(|l| {
                    if l.trim().is_empty() {
                        String::new()
                    } else {
                        format!("{base}{l}")
                    }
                })
                .collect()
        };
        let at = notes_at.unwrap_or(new.len()).min(new.len());
        new.splice(at..at, note_lines);
    }
    for url in add_links {
        let url = url.trim();
        if url.is_empty() {
            continue;
        }
        let exists = new.iter().any(|l| line_link(l) == Some(url));
        if exists {
            continue;
        }
        let at = new
            .iter()
            .rposition(|l| line_link(l).is_some())
            .map_or(0, |p| p + 1);
        new.insert(at, format!("{base}- {url}"));
    }
    // Drop blank lines that ended up at the end of the block (they would no longer
    // belong to it and would only add noise). A block never ends in a blank line.
    while new.last().is_some_and(|l| l.trim().is_empty()) {
        new.pop();
    }
    if new != old {
        doc.splice(block.line + 1..block.end, new);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> TaskLine {
        parse_task_line(s).unwrap().unwrap()
    }

    fn d(s: &str) -> NaiveDate {
        parse_date_strict(s).unwrap()
    }

    #[test]
    fn statuses() {
        assert_eq!(p("- [ ] a").status, TaskStatus::Todo);
        assert_eq!(p("- [/] a").status, TaskStatus::Doing);
        assert_eq!(p("- [x] a").status, TaskStatus::Done);
        assert_eq!(p("- [X] a").status, TaskStatus::Done);
        assert_eq!(p("- [!] a").status, TaskStatus::Blocked);
        assert_eq!(p("* [ ] a").bullet, '*');
        assert_eq!(p("+ [ ] a").bullet, '+');
        assert_eq!(p("- [ ]").title, "");
    }

    #[test]
    fn not_tasks_and_malformed() {
        assert!(parse_task_line("- plain bullet").is_none());
        assert!(parse_task_line("text").is_none());
        assert!(parse_task_line("").is_none());
        assert!(parse_task_line("-[ ] a").is_none());
        assert!(parse_task_line("- [?] a").unwrap().is_err());
        assert!(parse_task_line("- [] a").unwrap().is_err());
        assert!(parse_task_line("- [ a").unwrap().is_err());
        assert!(parse_task_line("- [ ]a").unwrap().is_err());
        assert!(parse_task_line("- [").unwrap().is_err());
    }

    #[test]
    fn tokens_any_order() {
        let t = p("- [ ] Todo task !wysoki @2026-11-20 ^g7h8i9");
        assert_eq!(t.title, "Todo task");
        assert_eq!(t.priority, Priority::High);
        assert_eq!(t.due, Some(Due::date(d("2026-11-20"))));
        assert_eq!(t.id.as_deref(), Some("g7h8i9"));
        let u = p("- [ ] Todo task ^g7h8i9 @2026-11-20 !wysoki");
        assert_eq!(
            (u.title.as_str(), u.priority, u.due, u.id.clone()),
            ("Todo task", t.priority, t.due, t.id.clone())
        );
        let t = p("- [x] Done ✓2026-11-12 ^a1b2c3");
        assert_eq!(t.done, Some(d("2026-11-12")));
        let t = p("- [ ] Low !niski @2026-11-20T14:30");
        assert_eq!(t.priority, Priority::Low);
        assert_eq!(t.due.unwrap().to_string(), "2026-11-20T14:30");
        assert!(t.id.is_none());
        let t = p("- [ ] Normal !normalny");
        assert_eq!(t.priority, Priority::Normal);
        assert_eq!(t.title, "Normal");
    }

    #[test]
    fn tokens_only_at_end() {
        let t = p("- [ ] Fix @mention and ^abc123 parsing");
        assert_eq!(t.title, "Fix @mention and ^abc123 parsing");
        assert!(t.id.is_none());
        let t = p("- [ ] Hey !important ^abc123");
        assert_eq!(t.title, "Hey !important");
        assert_eq!(t.id.as_deref(), Some("abc123"));
        // duplicate kind stops the run
        let t = p("- [ ] A @2026-01-01 @2026-01-02");
        assert_eq!(t.title, "A @2026-01-01");
        assert_eq!(t.due.unwrap().to_string(), "2026-01-02");
        // extra whitespace
        let t = p("- [ ]   Spaced   title    ^abc123   ");
        assert_eq!(t.title, "Spaced   title");
        assert_eq!(t.id.as_deref(), Some("abc123"));
        let t = p("- [ ] ^abc123");
        assert_eq!(t.title, "");
        assert_eq!(t.id.as_deref(), Some("abc123"));
    }

    #[test]
    fn invalid_tokens_stay_in_title_with_problem() {
        let t = p("- [ ] A @2026-02-30");
        assert_eq!(t.title, "A @2026-02-30");
        assert_eq!(t.due, None);
        assert_eq!(t.problems.len(), 1);
        let t = p("- [ ] A ^ABC123");
        assert_eq!(t.title, "A ^ABC123");
        assert_eq!(t.problems.len(), 1);
        let t = p("- [ ] A ✓wczoraj");
        assert_eq!(t.problems.len(), 1);
        let t = p("- [ ] A ^abc12");
        assert!(t.id.is_none());
        assert_eq!(t.problems.len(), 1);
        // Not token-like enough to warn about.
        let t = p("- [ ] Ping @2x and ^ab");
        assert_eq!(t.title, "Ping @2x and ^ab");
        assert!(t.problems.is_empty());
    }

    #[test]
    fn render_canonical_order() {
        let t = p("- [ ] Todo task ^g7h8i9 @2026-11-20 !wysoki");
        assert_eq!(t.render(), "- [ ] Todo task !wysoki @2026-11-20 ^g7h8i9");
        let t = p("* [x] Done ^a1b2c3 ✓2026-11-12");
        assert_eq!(t.render(), "* [x] Done ✓2026-11-12 ^a1b2c3");
    }

    #[test]
    fn rewrite_status_only_swaps_char() {
        let orig = "- [ ]  odd   spacing  ^abc123";
        let old = p(orig);
        let mut new = old.clone();
        new.status = TaskStatus::Doing;
        assert_eq!(
            TaskLine::rewrite(orig, &old, &new),
            "- [/]  odd   spacing  ^abc123"
        );
    }

    #[test]
    fn urls() {
        assert!(is_url("https://gitlab.com/group/repo/-/merge_requests/42"));
        assert!(is_url("http://x.y"));
        assert!(!is_url("https://"));
        assert!(!is_url("see https://x.y"));
        assert!(!is_url("foo"));
        assert_eq!(line_link("  - https://a.b/c"), Some("https://a.b/c"));
        assert_eq!(line_link("    https://a.b/c  "), Some("https://a.b/c"));
        assert_eq!(line_link("  - note https://a.b/c"), None);
    }

    #[test]
    fn blocks_links_notes() {
        let doc = Doc::parse(
            "- [ ] a ^aaaaaa\n  - https://x.y/1\n  - note one\n    deeper\n\n  after blank\n\n- [ ] b\nplain\n  - [?] c\n",
        );
        let (blocks, problems) = scan_tasks(&doc, 0..doc.len(), &Outline::of(&doc).code);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].range(), 0..6);
        assert_eq!(blocks[0].links, vec!["https://x.y/1"]);
        assert_eq!(blocks[0].notes, "- note one\n  deeper\n\nafter blank");
        assert_eq!(blocks[1].range(), 7..8);
        assert_eq!(blocks[1].notes, "");
    }

    #[test]
    fn rewrite_block() {
        let src = "- [ ] a\n  - https://x.y/1\n  - old note\n- [ ] b\n";
        let mut doc = Doc::parse(src);
        let (blocks, _) = scan_tasks(&doc, 0..doc.len(), &Outline::of(&doc).code);
        rewrite_continuation(
            &mut doc,
            &blocks[0],
            &["https://x.y/2".into()],
            &[],
            Some("new\n  indented"),
        );
        assert_eq!(
            doc.render(),
            "- [ ] a\n  - https://x.y/1\n  - https://x.y/2\n  new\n    indented\n- [ ] b\n"
        );
        let (blocks, _) = scan_tasks(&doc, 0..doc.len(), &Outline::of(&doc).code);
        rewrite_continuation(
            &mut doc,
            &blocks[0],
            &[],
            &["https://x.y/1".into()],
            Some(""),
        );
        assert_eq!(doc.render(), "- [ ] a\n  - https://x.y/2\n- [ ] b\n");
        let (blocks, _) = scan_tasks(&doc, 0..doc.len(), &Outline::of(&doc).code);
        rewrite_continuation(&mut doc, &blocks[1], &["https://z".into()], &[], None);
        assert_eq!(
            doc.render(),
            "- [ ] a\n  - https://x.y/2\n- [ ] b\n  - https://z\n"
        );
    }
}
