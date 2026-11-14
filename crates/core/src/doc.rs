//! Lossless line-based document model.
//!
//! A [`Doc`] stores every line of a file verbatim together with its own line
//! terminator, plus the UTF-8 BOM flag. Serializing an unmodified `Doc` reproduces
//! the input byte for byte. All structural understanding (frontmatter, headings,
//! tasks…) is computed as a *view* over the lines ([`Outline`]); edits are line
//! splices, so untouched lines are never re-rendered.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Eol {
    None,
    #[default]
    Lf,
    CrLf,
}

impl Eol {
    fn as_str(self) -> &'static str {
        match self {
            Eol::None => "",
            Eol::Lf => "\n",
            Eol::CrLf => "\r\n",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Line {
    pub text: String,
    pub eol: Eol,
}

/// Where a line passed to [`Doc::rearrange`] comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Src {
    /// Existing line `i`, unchanged.
    Old(usize),
    /// Existing line `i` with new text (it keeps its line ending).
    Edit(usize, String),
    /// A new line.
    New(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Doc {
    bom: bool,
    lines: Vec<Line>,
    /// Line terminator for new lines when the file has none yet (`\n` by default).
    fallback: Eol,
}

impl Doc {
    pub fn parse(input: &str) -> Doc {
        let (bom, input) = match input.strip_prefix('\u{feff}') {
            Some(rest) => (true, rest),
            None => (false, input),
        };
        let lines = input
            .split_inclusive('\n')
            .map(|raw| {
                if let Some(t) = raw.strip_suffix("\r\n") {
                    Line {
                        text: t.to_string(),
                        eol: Eol::CrLf,
                    }
                } else if let Some(t) = raw.strip_suffix('\n') {
                    Line {
                        text: t.to_string(),
                        eol: Eol::Lf,
                    }
                } else {
                    Line {
                        text: raw.to_string(),
                        eol: Eol::None,
                    }
                }
            })
            .collect();
        Doc {
            bom,
            lines,
            fallback: Eol::Lf,
        }
    }

    pub fn render(&self) -> String {
        let mut out = String::with_capacity(self.lines.iter().map(|l| l.text.len() + 2).sum());
        if self.bom {
            out.push('\u{feff}');
        }
        for l in &self.lines {
            out.push_str(&l.text);
            out.push_str(l.eol.as_str());
        }
        out
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn text(&self, i: usize) -> &str {
        &self.lines[i].text
    }

    /// Line terminator used for new lines: the first terminator found in the file,
    /// else the fallback (`\n` unless [`Doc::set_fallback_newline`] was called).
    pub fn newline(&self) -> Eol {
        self.lines
            .iter()
            .map(|l| l.eol)
            .find(|e| *e != Eol::None)
            .unwrap_or(self.fallback)
    }

    /// True if at least one line has a terminator.
    pub fn has_newline(&self) -> bool {
        self.lines.iter().any(|l| l.eol != Eol::None)
    }

    /// Terminator of line `i` (`None` for an unterminated last line).
    pub fn eol(&self, i: usize) -> Eol {
        self.lines[i].eol
    }

    /// Line terminator to use for new lines while the file has none of its own.
    pub fn set_fallback_newline(&mut self, eol: Eol) {
        if eol != Eol::None {
            self.fallback = eol;
        }
    }

    /// True if a line's text contains a line break (`\n`), which would turn one
    /// logical line into several when written. Never true for a parsed file; a
    /// guard against values that slipped through validation.
    pub fn has_embedded_newline(&self) -> bool {
        self.lines.iter().any(|l| l.text.contains('\n'))
    }

    pub fn set_text(&mut self, i: usize, text: String) {
        self.lines[i].text = text;
    }

    /// Replace `range` with `new` lines. New lines use [`Doc::newline`]. The
    /// "no trailing newline" state of the file is preserved when the splice touches
    /// the end of the file.
    pub fn splice(&mut self, range: Range<usize>, new: Vec<String>) {
        let nl = self.newline();
        let at_end = range.end == self.lines.len();
        let unterminated = self.lines.last().is_some_and(|l| l.eol == Eol::None);
        let new_lines = new.into_iter().map(|text| Line { text, eol: nl });
        self.lines.splice(range, new_lines);
        if at_end && unterminated {
            let n = self.lines.len();
            for (i, l) in self.lines.iter_mut().enumerate() {
                if i + 1 < n && l.eol == Eol::None {
                    l.eol = nl;
                }
            }
            if let Some(last) = self.lines.last_mut() {
                last.eol = Eol::None;
            }
        }
    }

    /// Like [`Doc::splice`], but leaves lines that are identical at the start and end
    /// of the range untouched, so the diff is as small as possible.
    pub fn splice_min(&mut self, range: Range<usize>, mut new: Vec<String>) {
        let (mut start, mut end) = (range.start, range.end);
        let mut head = 0;
        while start < end && head < new.len() && self.text(start) == new[head] {
            start += 1;
            head += 1;
        }
        let mut tail = new.len();
        while start < end && tail > head && self.text(end - 1) == new[tail - 1] {
            end -= 1;
            tail -= 1;
        }
        if start == end && head == tail {
            return;
        }
        new.truncate(tail);
        new.drain(..head);
        self.splice(start..end, new);
    }

    /// Replace `range` with `new`, where each new line is an existing line kept as it
    /// is ([`Src::Old`]), an existing line with new text but its own terminator
    /// ([`Src::Edit`]), or a new line ([`Src::New`], terminated like [`Doc::splice`]).
    /// Kept lines keep their bytes, including their line ending, even when they move.
    pub fn rearrange(&mut self, range: Range<usize>, new: Vec<Src>) {
        if range.end == self.lines.len() {
            // The end-of-file rules live in `splice`.
            let texts = new
                .into_iter()
                .map(|s| match s {
                    Src::Old(i) => self.text(i).to_string(),
                    Src::Edit(_, t) | Src::New(t) => t,
                })
                .collect();
            self.splice_min(range, texts);
            return;
        }
        let nl = self.newline();
        let eol = |e: Eol| if e == Eol::None { nl } else { e };
        let lines: Vec<Line> = new
            .into_iter()
            .map(|s| match s {
                Src::Old(i) => Line {
                    text: self.lines[i].text.clone(),
                    eol: eol(self.lines[i].eol),
                },
                Src::Edit(i, text) => Line {
                    text,
                    eol: eol(self.lines[i].eol),
                },
                Src::New(text) => Line { text, eol: nl },
            })
            .collect();
        self.lines.splice(range, lines);
    }

    pub fn insert(&mut self, at: usize, new: Vec<String>) {
        self.splice(at..at, new);
    }

    pub fn remove(&mut self, range: Range<usize>) {
        self.splice(range, Vec::new());
    }

    pub fn is_blank(&self, i: usize) -> bool {
        self.lines[i].text.trim().is_empty()
    }

    pub fn is_indented(&self, i: usize) -> bool {
        self.lines[i].text.starts_with([' ', '\t'])
    }

    /// End (exclusive) of the indented continuation block that starts at `first`
    /// (the line after a list item). Indented non-blank lines belong to the block;
    /// blank lines belong to it only when more indented lines follow. Stops at
    /// `limit`.
    pub fn continuation_end(&self, first: usize, limit: usize) -> usize {
        let mut end = first;
        let mut i = first;
        while i < limit {
            if self.is_blank(i) {
                i += 1;
                continue;
            }
            if self.is_indented(i) {
                i += 1;
                end = i;
            } else {
                break;
            }
        }
        end
    }
}

// ---------------------------------------------------------------------------
// Outline
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Section {
    /// Line index of the `## ` heading.
    pub heading: usize,
    /// Heading text, trimmed.
    pub name: String,
    /// Exclusive end (next `##` heading or end of file).
    pub end: usize,
}

impl Section {
    pub fn content(&self) -> Range<usize> {
        self.heading + 1..self.end
    }

    pub fn is(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Outline {
    /// Per line: true if the line is part of a top-level fenced code block (its
    /// delimiters included). Such lines are never headings, tasks or journal entries.
    pub code: Vec<bool>,
    /// `(opening, closing)` line indices of the `---` frontmatter fences.
    pub frontmatter: Option<(usize, usize)>,
    /// The file starts with `---` but has no closing fence.
    pub frontmatter_unclosed: bool,
    /// First line after the frontmatter.
    pub body_start: usize,
    /// Line index of the first `# ` heading before any `##` section.
    pub h1: Option<usize>,
    pub sections: Vec<Section>,
    pub len: usize,
}

// ---------------------------------------------------------------------------
// Fenced code blocks
// ---------------------------------------------------------------------------

/// `(delimiter byte, run length)` if `text` opens a fenced code block: at most 3
/// spaces of indentation, then at least three backticks or tildes. The info string
/// of a backtick fence may not contain a backtick (CommonMark).
fn fence_open(text: &str) -> Option<(u8, usize)> {
    let t = text.trim_start_matches(' ');
    if text.len() - t.len() > 3 {
        return None;
    }
    let ch = *t.as_bytes().first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let run = t.bytes().take_while(|&b| b == ch).count();
    if run < 3 || (ch == b'`' && t[run..].contains('`')) {
        return None;
    }
    Some((ch, run))
}

/// True if `text` closes a fence opened with `run` × `ch`: at most 3 spaces of
/// indentation, at least `run` of the same character, then only whitespace.
fn fence_closes(text: &str, ch: u8, run: usize) -> bool {
    let t = text.trim_start_matches(' ');
    if text.len() - t.len() > 3 {
        return false;
    }
    let n = t.bytes().take_while(|&b| b == ch).count();
    n >= run && t[n..].trim().is_empty()
}

/// A column-0 list item marker (`-`, `*`, `+`, `1.`, `1)`) followed by whitespace
/// or the end of the line.
fn is_list_item(text: &str) -> bool {
    let b = text.as_bytes();
    let n = match b.first() {
        Some(b'-' | b'*' | b'+') => 1,
        Some(c) if c.is_ascii_digit() => {
            let d = b.iter().take_while(|c| c.is_ascii_digit()).count();
            if d > 9 || !matches!(b.get(d), Some(b'.' | b')')) {
                return false;
            }
            d + 1
        }
        _ => return false,
    };
    matches!(b.get(n), None | Some(b' ' | b'\t'))
}

/// The one fence parser used for sections, tasks, journal entries and input
/// validation. Feed lines in order; [`FenceScanner::feed`] says whether a line is
/// code.
///
/// Rules (a CommonMark subset tailored to MontyBrain's line-based structure, where
/// every structural line starts in column 0):
/// * a fence opens on a line with at most 3 spaces of indentation and at least three
///   backticks or tildes, and closes on a line (again at most 3 spaces) made of at
///   least as many of the same character; an unclosed fence runs to the end of the
///   file;
/// * a fence marker on an indented line that continues a list item (a task, a
///   journal entry or any other `-`/`*`/`+`/`1.` item) belongs to that item: it
///   never hides anything, because the item ends at the next column-0 line anyway.
#[derive(Debug, Default, Clone)]
pub(crate) struct FenceScanner {
    open: Option<(u8, usize)>,
    in_item: bool,
}

impl FenceScanner {
    /// Feed the next line. Returns true if the line is code (a delimiter or inside
    /// a top-level fence).
    pub fn feed(&mut self, text: &str) -> bool {
        if let Some((ch, run)) = self.open {
            if fence_closes(text, ch, run) {
                self.open = None;
            }
            return true;
        }
        if text.trim().is_empty() {
            return false;
        }
        let indented = text.starts_with([' ', '\t']);
        if !indented {
            self.in_item = is_list_item(text);
        } else if self.in_item {
            return false;
        }
        match fence_open(text) {
            Some(f) => {
                self.open = Some(f);
                self.in_item = false;
                true
            }
            None => false,
        }
    }

    /// True while a top-level fence is open (it was not closed yet).
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }
}

pub(crate) fn heading_level_2(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("##")?;
    if rest.starts_with('#') {
        return None;
    }
    if rest.is_empty() {
        return Some("");
    }
    if rest.starts_with([' ', '\t']) {
        return Some(rest.trim());
    }
    None
}

pub(crate) fn heading_level_1(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('#')?;
    if rest.starts_with('#') {
        return None;
    }
    if rest.is_empty() || rest.starts_with([' ', '\t']) {
        return Some(rest.trim());
    }
    None
}

impl Outline {
    pub fn of(doc: &Doc) -> Outline {
        let n = doc.len();
        let mut frontmatter = None;
        let mut unclosed = false;
        if n > 0 && doc.text(0).trim_end() == "---" {
            match (1..n).find(|&i| matches!(doc.text(i).trim_end(), "---" | "...")) {
                Some(close) => frontmatter = Some((0, close)),
                None => unclosed = true,
            }
        }
        let body_start = frontmatter.map_or(0, |(_, c)| c + 1);
        let mut h1 = None;
        let mut sections: Vec<Section> = Vec::new();
        let mut code = vec![false; n];
        let mut fences = FenceScanner::default();
        for (i, is_code) in code.iter_mut().enumerate().skip(body_start) {
            let t = doc.text(i);
            if fences.feed(t) {
                *is_code = true;
                continue;
            }
            if let Some(name) = heading_level_2(t) {
                if let Some(last) = sections.last_mut() {
                    last.end = i;
                }
                sections.push(Section {
                    heading: i,
                    name: name.to_string(),
                    end: n,
                });
            } else if sections.is_empty() && h1.is_none() && heading_level_1(t).is_some() {
                h1 = Some(i);
            }
        }
        Outline {
            code,
            frontmatter,
            frontmatter_unclosed: unclosed,
            body_start,
            h1,
            sections,
            len: n,
        }
    }

    /// Start of the first `##` section, or end of file.
    pub fn first_section_start(&self) -> usize {
        self.sections.first().map_or(self.len, |s| s.heading)
    }

    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.is(name))
    }
}

/// Join lines `range` of `doc`, trimmed of surrounding blank lines/whitespace.
pub(crate) fn joined_trimmed(doc: &Doc, range: Range<usize>) -> String {
    let lines: Vec<&str> = range.map(|i| doc.text(i)).collect();
    lines.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_variants() {
        for s in [
            "",
            "\n",
            "a",
            "a\n",
            "a\r\nb\r\n",
            "a\r\nb",
            "a\nb\r\nc",
            "\u{feff}---\nx: 1\n---\n",
            "\n\n\n",
            "a\rb\n",
        ] {
            assert_eq!(Doc::parse(s).render(), s, "{s:?}");
        }
    }

    #[test]
    fn splice_preserves_missing_trailing_newline() {
        let mut d = Doc::parse("a\nb");
        d.insert(2, vec!["c".into()]);
        assert_eq!(d.render(), "a\nb\nc");
        d.remove(2..3);
        assert_eq!(d.render(), "a\nb");
        d.remove(1..2);
        assert_eq!(d.render(), "a");
        let mut d = Doc::parse("a\r\nb\r\n");
        d.insert(1, vec!["x".into()]);
        assert_eq!(d.render(), "a\r\nx\r\nb\r\n");
        let mut d = Doc::parse("");
        d.insert(0, vec!["x".into()]);
        assert_eq!(d.render(), "x\n");
    }

    #[test]
    fn outline_basic() {
        let d = Doc::parse(
            "---\na: 1\n---\n# T\n\ns\n```\n## not\n```\n## Zadania\n- [ ] x\n## Dziennik\n",
        );
        let o = Outline::of(&d);
        assert_eq!(o.frontmatter, Some((0, 2)));
        assert_eq!(o.h1, Some(3));
        assert_eq!(o.sections.len(), 2);
        assert_eq!(o.sections[0].name, "Zadania");
        assert_eq!(o.sections[0].content(), 10..11);
        assert!(o.section("zadania").is_some());
    }

    #[test]
    fn outline_unclosed_frontmatter() {
        let d = Doc::parse("---\na: 1\n# T\n");
        let o = Outline::of(&d);
        assert!(o.frontmatter.is_none());
        assert!(o.frontmatter_unclosed);
    }

    #[test]
    fn fences_track_delimiter_and_length() {
        let code = |s: &str| Outline::of(&Doc::parse(s)).code;
        // 4-backtick fence with an inner 3-backtick line.
        assert_eq!(
            code("````\n```\nx\n```\n````\ny\n"),
            vec![true, true, true, true, true, false]
        );
        // Tilde fence is not closed by backticks or by a shorter run.
        assert_eq!(
            code("~~~~\n```\n~~~\n~~~~~ \nz\n"),
            vec![true, true, true, true, false]
        );
        // Indentation: up to 3 spaces opens; 4 does not; closers need <= 3 too.
        assert_eq!(
            code("   ```\na\n    ```\n```\nb\n"),
            vec![true, true, true, true, false]
        );
        assert_eq!(code("    ```\n## H\n"), vec![false, false]);
        // Backtick info string may not contain a backtick.
        assert_eq!(code("``` a`b\n## H\n"), vec![false, false]);
        // A fence inside a list item's continuation never hides column-0 lines.
        assert_eq!(
            code("- [ ] a\n  ```\n## H\n- [ ] b\n  ```\n"),
            vec![false, false, false, false, false]
        );
        // ...but after a paragraph line an indented fence is a real fence.
        assert_eq!(code("text\n  ```\n## H\n"), vec![false, true, true]);
        // Unclosed fence runs to the end of the file.
        assert_eq!(code("```\n## H\n"), vec![true, true]);
        // Frontmatter is not scanned.
        assert_eq!(
            code("---\na: ```\n---\n## H\n"),
            vec![false, false, false, false]
        );
    }

    #[test]
    fn list_items() {
        for s in ["- a", "-", "* [ ] x", "+\tb", "1. x", "12) y"] {
            assert!(is_list_item(s), "{s}");
        }
        for s in ["---", "-x", "1.x", "a", "", "#"] {
            assert!(!is_list_item(s), "{s}");
        }
    }

    #[test]
    fn continuation() {
        let d = Doc::parse("- [ ] a\n  n1\n\n  n2\n\n- [ ] b\n");
        assert_eq!(d.continuation_end(1, d.len()), 4);
    }
}
