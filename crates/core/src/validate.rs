//! Validation of free-text input before MontyBrain writes it into a file.
//!
//! The goal is that no value can create or alter structure outside its own field:
//! no new tasks, headings, journal entries, frontmatter keys or delimiters, and no
//! code fence that would swallow the rest of the file. Input that would do so is
//! rejected with [`Error::InvalidInput`] naming the field and the reason; it is not
//! silently rewritten. The rules are documented in `docs/format.md` §2.1.

use crate::doc::{FenceScanner, heading_level_1, heading_level_2};
use crate::error::{Error, Result};
use crate::task::is_url;

fn bad(field: &str, why: impl std::fmt::Display) -> Error {
    Error::invalid(format!("{field}: {why}"))
}

/// Line and paragraph separators that some editors treat as line breaks.
fn is_separator(c: char) -> bool {
    matches!(c, '\u{2028}' | '\u{2029}')
}

fn describe(c: char) -> String {
    match c {
        '\n' | '\r' => "a line break".into(),
        c => format!("the control character U+{:04X}", c as u32),
    }
}

/// A title or name (task title, topic title, project name). Whitespace runs,
/// including line breaks, collapse to one space (documented behaviour); other
/// control characters are rejected; the result must not be empty.
pub(crate) fn one_line(field: &str, s: &str) -> Result<String> {
    if let Some(c) = s.chars().find(|c| c.is_control() && !c.is_whitespace()) {
        return Err(bad(field, format!("contains {}", describe(c))));
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.is_empty() {
        return Err(bad(field, "must not be empty"));
    }
    Ok(s)
}

/// A strict single-line value (colour, repository name and path): surrounding
/// whitespace is trimmed, line breaks and control characters are rejected.
pub(crate) fn strict_line(field: &str, s: &str) -> Result<String> {
    let t = s.trim();
    if let Some(c) = t.chars().find(|&c| c.is_control() || is_separator(c)) {
        return Err(bad(
            field,
            format!(
                "must be a single line without control characters (found {})",
                describe(c)
            ),
        ));
    }
    Ok(t.to_string())
}

/// A link: a single bare URL (`scheme://…` without whitespace). `Ok(None)` for an
/// empty string, which callers skip.
pub(crate) fn link(field: &str, s: &str) -> Result<Option<String>> {
    let t = s.trim();
    if t.is_empty() {
        return Ok(None);
    }
    if is_url(t) && !t.chars().any(|c| c.is_control() || is_separator(c)) {
        return Ok(Some(t.to_string()));
    }
    Err(bad(
        field,
        format!("{t:?} is not a bare URL (expected scheme://… without spaces or line breaks)"),
    ))
}

/// How multi-line text is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Multi {
    /// Task notes, journal text: every line is written indented below its item,
    /// so no line can start a structure of its own.
    Indented,
    /// Topic summary, project description: lines are written in column 0 between
    /// the title and the first `##` section.
    Region,
}

/// Multi-line text. `\r\n` counts as a line break; any other control character
/// (except tab) and U+2028/U+2029 are rejected. For [`Multi::Region`] text the
/// lines must also not be structure: no `#`/`##` heading, no `---` first line and
/// no unclosed code fence.
///
/// Returns the text as it will be stored: `\r\n` normalised to `\n`; for
/// [`Multi::Region`] also trimmed, with trailing whitespace removed from every line
/// (that is how the region is written and read back).
pub(crate) fn multiline(field: &str, s: &str, kind: Multi) -> Result<String> {
    let s = s.replace("\r\n", "\n");
    for (n, line) in s.split('\n').enumerate() {
        if let Some(c) = line
            .chars()
            .find(|&c| (c.is_control() && c != '\t') || is_separator(c))
        {
            let what = if c == '\r' {
                "a carriage return that is not part of a \\r\\n line break".to_string()
            } else {
                describe(c)
            };
            return Err(bad(field, format!("line {} contains {what}", n + 1)));
        }
    }
    if kind == Multi::Region {
        let region = s
            .trim()
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n");
        check_region(field, &region)?;
        return Ok(region);
    }
    Ok(s)
}

fn check_region(field: &str, text: &str) -> Result<()> {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    if lines.first().is_some_and(|l| *l == "---") {
        return Err(bad(
            field,
            "must not start with a `---` line (it would be read as a frontmatter delimiter)",
        ));
    }
    let mut fences = FenceScanner::default();
    for (n, l) in lines.iter().enumerate() {
        if fences.feed(l) {
            continue;
        }
        if heading_level_2(l).is_some() {
            return Err(bad(
                field,
                format!(
                    "line {} `{l}` is a level-2 heading, which would start a new section; use `###` or deeper",
                    n + 1
                ),
            ));
        }
        if heading_level_1(l).is_some() {
            return Err(bad(
                field,
                format!(
                    "line {} `{l}` is a level-1 heading, which would be read as the title; use `###` or deeper",
                    n + 1
                ),
            ));
        }
    }
    if fences.is_open() {
        return Err(bad(
            field,
            "opens a code fence (``` or ~~~) that is not closed; it would hide the rest of the file",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_fields() {
        assert_eq!(one_line("t", " a \n b\t").unwrap(), "a b");
        assert!(one_line("t", " \n ").is_err());
        assert!(one_line("t", "a\u{1b}b").is_err());
        assert_eq!(strict_line("c", " #fff ").unwrap(), "#fff");
        assert!(strict_line("c", "a\nb").is_err());
        assert!(strict_line("c", "a\u{2028}b").is_err());
        assert_eq!(
            link("l", " https://x.y ").unwrap().as_deref(),
            Some("https://x.y")
        );
        assert_eq!(link("l", "  ").unwrap(), None);
        for bad in [
            "https://x\n- [ ] y",
            "x.y",
            "see https://x.y",
            "https://x\u{0}",
        ] {
            assert!(link("l", bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn multiline_fields() {
        assert_eq!(multiline("n", "a\r\nb", Multi::Indented).unwrap(), "a\nb");
        assert!(multiline("n", "a\rb", Multi::Indented).is_err());
        assert!(multiline("n", "a\u{85}b", Multi::Indented).is_err());
        // Indented text may contain anything line-shaped.
        assert!(multiline("n", "## x\n```\n- [ ] y", Multi::Indented).is_ok());
        let r = |s: &str| multiline("summary", s, Multi::Region);
        assert!(r("ok\n### deeper\n- [ ] item\n```\n## in code\n```").is_ok());
        assert!(r("a\n## Zadania").is_err());
        assert!(r("##").is_err());
        assert!(r("# T").is_err());
        assert!(r("#hashtag").is_ok());
        assert!(r("\n---\nx: 1\n---").is_err());
        assert!(r("a\n---\nb").is_ok());
        assert!(r("a\n~~~\nb").is_err());
        assert!(r("- item\n  ```").is_ok(), "item-local fence");
        assert_eq!(r("\n a  \nb \n\n").unwrap(), "a\nb", "stored form");
    }
}
