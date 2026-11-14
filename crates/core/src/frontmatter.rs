//! Minimal, line-preserving YAML frontmatter handling.
//!
//! Supported subset (see `docs/format.md`): top-level `key: scalar` lines (plain,
//! `"double"` or `'single'` quoted, optional trailing `# comment`), and the
//! `repozytoria` block sequence of `nazwa`/`sciezka` mappings. Everything else
//! (unknown keys, comments, blank lines, nested structures) is preserved verbatim
//! because edits only ever replace the lines of the key being changed.

use crate::doc::{Doc, Outline, Src};
use crate::model::Repo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FmEntry {
    pub key: String,
    /// Line index of the `key:` line.
    pub line: usize,
    /// Exclusive end of the entry (includes indented / `-` continuation lines).
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frontmatter {
    pub open: usize,
    pub close: usize,
    pub entries: Vec<FmEntry>,
}

/// Byte index of the mapping colon in a `key: value` line (colon followed by
/// whitespace or end of line), if the line is a top-level key line.
fn key_colon(text: &str) -> Option<usize> {
    if text.is_empty() || text.starts_with([' ', '\t', '#', '-']) {
        return None;
    }
    let bytes = text.as_bytes();
    (0..bytes.len()).find(|&i| {
        bytes[i] == b':' && (i + 1 == bytes.len() || bytes[i + 1] == b' ' || bytes[i + 1] == b'\t')
    })
}

impl Frontmatter {
    pub fn of(doc: &Doc, outline: &Outline) -> Option<Frontmatter> {
        let (open, close) = outline.frontmatter?;
        let mut entries = Vec::new();
        let mut i = open + 1;
        while i < close {
            let t = doc.text(i);
            if let Some(colon) = key_colon(t) {
                let key = t[..colon].trim().to_string();
                // Continuation: indented lines, `-` items, and blank/comment lines
                // when followed by more continuation.
                let mut end = i + 1;
                let mut j = i + 1;
                while j < close {
                    let tj = doc.text(j);
                    let trimmed = tj.trim();
                    if trimmed.is_empty()
                        || (trimmed.starts_with('#') && !tj.starts_with([' ', '\t']))
                    {
                        j += 1;
                        continue;
                    }
                    if tj.starts_with([' ', '\t', '-']) {
                        j += 1;
                        end = j;
                    } else {
                        break;
                    }
                }
                entries.push(FmEntry { key, line: i, end });
                i = end;
            } else {
                i += 1;
            }
        }
        Some(Frontmatter {
            open,
            close,
            entries,
        })
    }

    /// The first entry of `key`. It is authoritative: later entries with the same key
    /// are duplicates, which are kept as they are but never read or written.
    pub fn entry(&self, key: &str) -> Option<&FmEntry> {
        self.entries.iter().find(|e| e.key == key)
    }

    /// The entries of `key` after the first one.
    pub fn duplicates<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a FmEntry> {
        self.entries.iter().filter(move |e| e.key == key).skip(1)
    }

    pub fn has_duplicate(&self, key: &str) -> bool {
        self.duplicates(key).next().is_some()
    }

    /// Raw value text after the colon of `key` (untrimmed), if present.
    pub fn raw_value<'a>(&self, doc: &'a Doc, key: &str) -> Option<&'a str> {
        let e = self.entry(key)?;
        let t = doc.text(e.line);
        let colon = key_colon(t)?;
        Some(&t[colon + 1..])
    }

    /// Line index of the first non-blank line that continues `key` (an indented
    /// line or a `-` item below it), if any.
    pub fn continuation(&self, doc: &Doc, key: &str) -> Option<usize> {
        let e = self.entry(key)?;
        (e.line + 1..e.end).find(|&i| {
            let t = doc.text(i);
            !t.trim().is_empty() && t.starts_with([' ', '\t', '-'])
        })
    }

    /// Parsed scalar value of `key`. `Ok(None)` for absent/empty/null.
    pub fn scalar(&self, doc: &Doc, key: &str) -> Result<Option<String>, String> {
        match self.raw_value(doc, key) {
            None => Ok(None),
            Some(raw) => parse_scalar(raw).map(|(v, _)| v),
        }
    }
}

/// YAML separation whitespace (space and tab).
const WS: [char; 2] = [' ', '\t'];

/// Byte offset where a comment starts in `s` (text after a colon or a value): a `#`
/// at the very start or right after a space or tab. The one rule for scalar values
/// and the `repozytoria:` header, for reading and writing.
fn comment_start(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    (0..b.len()).find(|&i| b[i] == b'#' && (i == 0 || b[i - 1] == b' ' || b[i - 1] == b'\t'))
}

/// The value of a `repozytoria:` line (`raw` is the text after the colon), shared by
/// the reader and the writer: `Ok((empty_flow, comment))` when it is empty or `[]`,
/// optionally followed by whitespace and a comment (`comment` starts at the `#`, or
/// is empty); `Err(value)` for anything else (a flow list, a scalar).
fn repos_header(raw: &str) -> Result<(bool, &str), String> {
    let body = raw.trim_start_matches(WS);
    if comment_start(body) == Some(0) || body.is_empty() {
        return Ok((false, body));
    }
    if let Some(rest) = body.strip_prefix("[]") {
        let after = rest.trim_start_matches(WS);
        // A comment needs whitespace before it: `[]#x` is not `[]` plus a comment.
        if after.is_empty() || (after.starts_with('#') && after.len() < rest.len()) {
            return Ok((true, after));
        }
    }
    let value = &body[..comment_start(body).unwrap_or(body.len())];
    Err(value.trim_end_matches(WS).to_string())
}

/// Parse a YAML scalar from the text after `key:`. Returns the value (`None` for
/// empty/null) and the byte offset in `raw` where the value ends (start of the
/// trailing whitespace/comment).
pub(crate) fn parse_scalar(raw: &str) -> Result<(Option<String>, usize), String> {
    let lead = raw.len() - raw.trim_start().len();
    let s = &raw[lead..];
    let check_tail = |tail: &str| -> Result<(), String> {
        let t = tail.trim_start();
        if t.is_empty() || t.starts_with('#') {
            Ok(())
        } else {
            Err(format!("unexpected text after quoted value: {t}"))
        }
    };
    if s.is_empty() || s.starts_with('#') {
        return Ok((None, lead));
    }
    if let Some(body) = s.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = body.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => {
                    let end = lead + 1 + i + 1;
                    check_tail(&raw[end..])?;
                    return Ok((Some(out), end));
                }
                '\\' => match chars.next() {
                    Some((_, 'n')) => out.push('\n'),
                    Some((_, 't')) => out.push('\t'),
                    Some((_, 'r')) => out.push('\r'),
                    Some((_, '0')) => out.push('\0'),
                    Some((_, '"')) => out.push('"'),
                    Some((_, '\\')) => out.push('\\'),
                    Some((_, '/')) => out.push('/'),
                    Some((_, other)) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => return Err("unterminated escape in quoted value".into()),
                },
                c => out.push(c),
            }
        }
        return Err("unterminated double-quoted value".into());
    }
    if let Some(body) = s.strip_prefix('\'') {
        let mut out = String::new();
        let mut chars = body.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '\'' {
                if let Some((_, '\'')) = chars.peek() {
                    chars.next();
                    out.push('\'');
                    continue;
                }
                let end = lead + 1 + i + 1;
                check_tail(&raw[end..])?;
                return Ok((Some(out), end));
            }
            out.push(c);
        }
        return Err("unterminated single-quoted value".into());
    }
    if s.starts_with(['|', '>']) {
        return Err("block scalars are not supported here".into());
    }
    // Plain scalar: ends at ` #` / `\t#` or end of line.
    let cut = comment_start(s).unwrap_or(s.len());
    let value = s[..cut].trim_end();
    let end = lead + value.len();
    if value.is_empty() || value == "~" || value == "null" || value == "Null" || value == "NULL" {
        return Ok((None, end));
    }
    Ok((Some(value.to_string()), end))
}

/// Encode a string as a YAML scalar: plain when unambiguous, double-quoted
/// otherwise.
pub(crate) fn encode_scalar(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let special_word = matches!(
        lower.as_str(),
        "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~" | "y" | "n"
    );
    let numeric = s.parse::<f64>().is_ok()
        || (s.starts_with(|c: char| c.is_ascii_digit())
            && s.chars().all(|c| c.is_ascii_digit() || "-:._".contains(c)));
    let needs_quotes = s.is_empty()
        || special_word
        || numeric
        || s.trim() != s
        || s.starts_with([
            '-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%',
            '@', '`',
        ])
        || s.contains(": ")
        || s.contains(" #")
        || s.contains('\t')
        || s.ends_with(':')
        || s.chars().any(char::is_control);
    if !needs_quotes {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The text after a scalar value on its line (whitespace and a `# comment`), given
/// the text after the colon. Empty if the value cannot be parsed (it is replaced as a
/// whole).
fn value_suffix(raw: &str) -> &str {
    match parse_scalar(raw) {
        Ok((_, end)) => &raw[end..],
        Err(_) => "",
    }
}

/// `line` with the scalar value after byte `at` (just after the colon) replaced by
/// `enc` (already encoded), or emptied when `enc` is `None`. The whitespace between
/// the colon and the value and a trailing comment (with the spacing before it) are
/// kept, so clearing and setting again gives back the same line. A comment never
/// ends up glued to the colon or the value, where it would read back as part of them.
fn replace_value(line: &str, at: usize, enc: Option<&str>) -> String {
    let raw = &line[at..];
    let body = raw.trim_start();
    let (lead, suffix) = if body.is_empty() || body.starts_with('#') {
        // No value yet: everything after the colon is spacing and comment.
        (" ", raw)
    } else {
        (&raw[..raw.len() - body.len()], value_suffix(raw))
    };
    let sep = if suffix.starts_with('#') { " " } else { "" };
    match enc {
        Some(enc) => format!("{}{lead}{enc}{sep}{suffix}", &line[..at]),
        None => format!("{}{sep}{suffix}", &line[..at]),
    }
}

/// Set (`Some(raw)`, already encoded) or remove (`None`) a top-level scalar key.
///
/// Only the key's own line is ever changed: an existing key keeps its position, the
/// text before the colon, the spacing after it and any trailing comment. Lines that
/// follow the key (indented continuation lines, comments) are never touched; such
/// continuation content under one of Morwisko's scalar keys is unsupported and
/// reported as a load warning. Removing a key deletes its line, unless the line has
/// a trailing comment or continuation lines: then its value is emptied (`key:`, comment
/// kept), so neither is lost. It is emptied too when the key occurs again further
/// down: the first occurrence is the one that counts, and deleting it would let the
/// duplicate take over. A new key is inserted after the last present key from
/// `after` (or at the end of the frontmatter); a frontmatter block is created if the
/// file has none.
pub(crate) fn set_scalar(doc: &mut Doc, key: &str, value: Option<&str>, after: &[&str]) {
    let outline = Outline::of(doc);
    let Some(fm) = Frontmatter::of(doc, &outline) else {
        if let Some(v) = value {
            doc.insert(0, vec!["---".into(), format!("{key}: {v}"), "---".into()]);
        }
        return;
    };
    if let Some(e) = fm.entry(key).cloned() {
        let t = doc.text(e.line);
        let colon = key_colon(t).expect("entry line has a colon");
        if value.is_none()
            && e.end == e.line + 1
            && value_suffix(&t[colon + 1..]).trim().is_empty()
            && !fm.has_duplicate(key)
        {
            doc.remove(e.line..e.line + 1);
            return;
        }
        let new = replace_value(t, colon + 1, value);
        if doc.text(e.line) != new {
            doc.set_text(e.line, new);
        }
        return;
    }
    let Some(v) = value else { return };
    let at = after
        .iter()
        .filter_map(|k| fm.entry(k))
        .map(|e| e.end)
        .max()
        .unwrap_or(fm.close);
    doc.insert(at, vec![format!("{key}: {v}")]);
}

// ---------------------------------------------------------------------------
// repozytoria
// ---------------------------------------------------------------------------

pub(crate) const REPOS_KEY: &str = "repozytoria";

/// A recognised field (`nazwa` or `sciezka`) of a repository item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepoField {
    /// Line of the `key: value`.
    pub line: usize,
    /// Byte offset in that line just after the key's colon.
    pub value_at: usize,
    /// Exclusive end of the field: its line plus the lines nested below it.
    pub end: usize,
}

/// One `- …` item of the `repozytoria` list.
///
/// An item owns its `-` line and every following line up to the first non-blank,
/// non-comment line that is not indented deeper than the `-` (which is list-level
/// text or the next item). Comments and blank lines between its lines belong to it,
/// whatever their indentation; comments after its last line belong to it only when
/// they are indented deeper than the `-`. Inside the item, lines at the key column
/// are fields; deeper lines (and `-` lines) are nested under the field above them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepoItem {
    pub name: Option<String>,
    pub path: Option<String>,
    /// Line range the item owns.
    pub start: usize,
    pub end: usize,
    /// Leading whitespace before the dash.
    pub indent: String,
    /// Whitespace between the dash and the first key on the dash line, if the dash
    /// line holds a key.
    pub dash_gap: Option<String>,
    /// Leading whitespace of a field on a line of its own.
    pub key_prefix: Option<String>,
    /// The fields that give `name` and `path` (the last one when a key repeats).
    pub name_field: Option<RepoField>,
    pub path_field: Option<RepoField>,
}

fn indent_len(t: &str) -> usize {
    t.len() - t.trim_start().len()
}

/// Text after the dash of a `-` item line (`trimmed` has no leading whitespace).
fn dash_rest(trimmed: &str) -> Option<&str> {
    trimmed
        .strip_prefix('-')
        .filter(|r| r.is_empty() || r.starts_with([' ', '\t']))
}

fn is_blank_or_comment(t: &str) -> bool {
    let t = t.trim_start();
    t.is_empty() || t.starts_with('#')
}

/// Exclusive end of the lines owned by the item whose `-` line is `start` (see
/// [`RepoItem`]). `limit` is the end of the `repozytoria` entry.
fn item_end(doc: &Doc, start: usize, limit: usize) -> usize {
    let dash = indent_len(doc.text(start));
    let mut end = start + 1;
    let mut i = start + 1;
    while i < limit {
        let t = doc.text(i);
        if is_blank_or_comment(t) {
            i += 1;
        } else if indent_len(t) > dash {
            i += 1;
            end = i;
        } else {
            break;
        }
    }
    // Only blank and comment lines are left before `i`; keep the deeper comments.
    (end..i)
        .filter(|&j| !doc.is_blank(j))
        .take_while(|&j| indent_len(doc.text(j)) > dash)
        .last()
        .map_or(end, |j| j + 1)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Last {
    None,
    Name,
    Path,
    Other,
}

fn parse_item(
    doc: &Doc,
    start: usize,
    limit: usize,
    problems: &mut Vec<(usize, String)>,
) -> RepoItem {
    let end = item_end(doc, start, limit);
    let dash_line = doc.text(start);
    let dash = indent_len(dash_line);
    let rest = dash_rest(&dash_line[dash..]).unwrap_or("");
    let first = rest.trim_start();
    let gap = &rest[..rest.len() - first.len()];
    let mut item = RepoItem {
        name: None,
        path: None,
        start,
        end,
        indent: dash_line[..dash].to_string(),
        dash_gap: None,
        key_prefix: None,
        name_field: None,
        path_field: None,
    };
    let mut key_col = None;
    let mut last = Last::None;
    let mut nested_reported = [false; 2];
    let mut lines: Vec<(usize, usize)> = Vec::new();
    if !first.is_empty() && !first.starts_with('#') {
        key_col = Some(dash + 1 + gap.len());
        item.dash_gap = Some(gap.to_string());
        item.key_prefix = Some(format!("{} {gap}", item.indent));
        lines.push((start, dash + 1 + gap.len()));
    }
    for i in start + 1..end {
        let t = doc.text(i);
        if !is_blank_or_comment(t) {
            lines.push((i, indent_len(t)));
        }
    }
    for (i, at) in lines {
        let t = doc.text(i);
        let content = &t[at..];
        let col = *key_col.get_or_insert_with(|| {
            item.key_prefix = Some(t[..at].to_string());
            at
        });
        if i != start && (at > col || dash_rest(content).is_some()) {
            // Nested under the field above.
            let (field, k, key) = match last {
                Last::Name => (&mut item.name_field, 0, "nazwa"),
                Last::Path => (&mut item.path_field, 1, "sciezka"),
                _ => continue,
            };
            if let Some(f) = field.as_mut() {
                f.end = i + 1;
            }
            if !nested_reported[k] {
                nested_reported[k] = true;
                let line = field.as_ref().map_or(i, |f| f.line);
                problems.push((
                    line,
                    format!(
                        "`{REPOS_KEY}`: the indented lines below `{key}` are not supported; they are kept but ignored"
                    ),
                ));
            }
            continue;
        }
        let Some(colon) = key_colon(content) else {
            problems.push((i, format!("expected `key: value` in `{REPOS_KEY}`")));
            last = Last::Other;
            continue;
        };
        let key = content[..colon].trim();
        let (value, field, which) = match key {
            "nazwa" => (&mut item.name, &mut item.name_field, Last::Name),
            "sciezka" => (&mut item.path, &mut item.path_field, Last::Path),
            _ => {
                last = Last::Other;
                continue;
            }
        };
        if field.is_some() {
            problems.push((
                i,
                format!("duplicate `{key}` in a `{REPOS_KEY}` item; the last one is used"),
            ));
        }
        *field = Some(RepoField {
            line: i,
            value_at: at + colon + 1,
            end: i + 1,
        });
        *value = match parse_scalar(&content[colon + 1..]) {
            Ok((v, _)) => v,
            Err(msg) => {
                problems.push((i, msg));
                None
            }
        };
        last = which;
    }
    item
}

/// Parse the `repozytoria` entry. Returns items and per-line problems
/// `(line index, message)`.
pub(crate) fn parse_repos(doc: &Doc, fm: &Frontmatter) -> (Vec<RepoItem>, Vec<(usize, String)>) {
    let mut items: Vec<RepoItem> = Vec::new();
    let mut problems = Vec::new();
    let Some(e) = fm.entry(REPOS_KEY) else {
        return (items, problems);
    };
    if let Some(Err(v)) = fm.raw_value(doc, REPOS_KEY).map(repos_header) {
        problems.push((
            e.line,
            format!("unsupported `{REPOS_KEY}` value (use a block list): {v}"),
        ));
        return (items, problems);
    }
    let mut i = e.line + 1;
    while i < e.end {
        let t = doc.text(i);
        if is_blank_or_comment(t) {
            i += 1;
        } else if dash_rest(t.trim_start()).is_some() {
            let item = parse_item(doc, i, e.end, &mut problems);
            i = item.end;
            items.push(item);
        } else {
            problems.push((
                i,
                format!(
                    "unexpected line in `{REPOS_KEY}`: it is not inside an item (indent it deeper than the item's `-`); kept but ignored"
                ),
            ));
            i += 1;
        }
    }
    for it in &items {
        if it.name.is_none() {
            problems.push((it.start, "repository entry without `nazwa`".into()));
        }
    }
    (items, problems)
}

pub(crate) fn repos_from_items(items: &[RepoItem]) -> Vec<Repo> {
    items
        .iter()
        .filter_map(|it| {
            Some(Repo {
                name: it.name.clone()?,
                path: it.path.clone().unwrap_or_default(),
            })
        })
        .collect()
}

/// Which existing item (index into `items`) each repo of the new list continues.
///
/// 1. An unmatched item with the same `nazwa` and `sciezka`, else one with the same
///    `nazwa` (so with duplicate names, list order decides).
/// 2. A repo with a non-empty `sciezka` that exactly one unmatched item has, when no
///    other unmatched repo has it too: a rename that kept its path, wherever it moved.
/// 3. A repo left over takes an unmatched named item from the same place: between the
///    items matched to its neighbours in the new list. One with the same non-empty
///    `sciezka` first, then in order.
/// 4. Repos still left over take the items still left over, in order (list order for
///    the repos, file order for the items).
///
/// Steps 2–4 read a changed repo as a rename (or a rename plus a path change): the
/// list alone cannot tell it from "remove one, add another", and keeping the item's
/// other lines loses nothing. So a repo is new only when every named item is matched,
/// and items are removed only when the new list is shorter than the item list.
fn match_repos(items: &[RepoItem], repos: &[Repo]) -> Vec<Option<usize>> {
    let n = repos.len();
    let mut to: Vec<Option<usize>> = vec![None; n];
    let mut used: Vec<bool> = items.iter().map(|it| it.name.is_none()).collect();
    let path_of = |k: usize| items[k].path.as_deref().unwrap_or("");
    let take = |to: &mut Vec<Option<usize>>, used: &mut Vec<bool>, j: usize, k: usize| {
        to[j] = Some(k);
        used[k] = true;
    };
    for exact in [true, false] {
        for (j, r) in repos.iter().enumerate() {
            if to[j].is_some() {
                continue;
            }
            let found = (0..items.len()).find(|&k| {
                !used[k]
                    && items[k].name.as_deref() == Some(r.name.as_str())
                    && (!exact || path_of(k) == r.path)
            });
            if let Some(k) = found {
                take(&mut to, &mut used, j, k);
            }
        }
    }
    for j in 0..n {
        let path = repos[j].path.as_str();
        if to[j].is_some() || path.is_empty() {
            continue;
        }
        let rivals = (0..n).filter(|&x| to[x].is_none() && repos[x].path == path);
        let mut cands = (0..items.len()).filter(|&k| !used[k] && path_of(k) == path);
        if let (Some(k), None, 1) = (cands.next(), cands.next(), rivals.count()) {
            take(&mut to, &mut used, j, k);
        }
    }
    let mut j = 0;
    while j < n {
        if to[j].is_some() {
            j += 1;
            continue;
        }
        let run_end = (j..n).find(|&x| to[x].is_some()).unwrap_or(n);
        let lo = j.checked_sub(1).and_then(|p| to[p]);
        let hi = to.get(run_end).copied().flatten();
        let in_place = |k: usize| {
            let after = lo.is_none_or(|l| k > l);
            // With a reordered list the neighbours give no upper bound.
            let before = match (lo, hi) {
                (Some(l), Some(h)) if h < l => true,
                (_, Some(h)) => k < h,
                (_, None) => true,
            };
            after && before
        };
        let mut cands: Vec<usize> = (0..items.len())
            .filter(|&k| !used[k] && in_place(k))
            .collect();
        for (m, r) in to[j..run_end].iter_mut().zip(&repos[j..run_end]) {
            if r.path.is_empty() {
                continue;
            }
            if let Some(p) = cands.iter().position(|&k| path_of(k) == r.path) {
                *m = Some(cands.remove(p));
            }
        }
        for m in &mut to[j..run_end] {
            if m.is_none() && !cands.is_empty() {
                *m = Some(cands.remove(0));
            }
        }
        for k in to[j..run_end].iter().flatten() {
            used[*k] = true;
        }
        j = run_end;
    }
    let mut left = (0..items.len()).filter(|&k| !used[k]);
    for m in to.iter_mut().filter(|m| m.is_none()) {
        match left.next() {
            Some(k) => *m = Some(k),
            None => break,
        }
    }
    to
}

/// `(dash prefix, key prefix)` of new lines shaped like item `it`: `- nazwa: …` and
/// `sciezka: …` at its indentation and key column.
fn item_shape(it: &RepoItem) -> (String, String) {
    let gap = it.dash_gap.clone().unwrap_or_else(|| {
        let col = it.key_prefix.as_ref().map_or(0, String::len);
        " ".repeat(col.saturating_sub(it.indent.len() + 1).max(1))
    });
    (
        format!("{}-{gap}", it.indent),
        format!("{} {gap}", it.indent),
    )
}

/// `text` (a line of an item whose dash was indented by `from`) moved to an item
/// place indented by `to`. The indentation changes by the same amount on every line,
/// so the item's inner layout (key column, nested lines, comments) stays the same
/// relative to its dash. Blank lines are kept.
fn reindent(text: &str, from: &str, to: &str) -> String {
    if text.trim().is_empty() {
        return text.to_string();
    }
    if let Some(rest) = text.strip_prefix(from) {
        return format!("{to}{rest}");
    }
    // A line less indented than the dash (only a comment can be) or indented with
    // other whitespace characters: shift it by the byte difference.
    if to.len() >= from.len() {
        format!("{}{text}", " ".repeat(to.len() - from.len()))
    } else {
        let ws = text.len() - text.trim_start_matches(WS).len();
        text[ws.min(from.len() - to.len())..].to_string()
    }
}

/// The lines of item `it` continuing as repo `r`: only the value of its `nazwa` and
/// `sciezka` lines changes (a missing `sciezka` is added below the `nazwa`).
fn edit_item(doc: &Doc, it: &RepoItem, r: &Repo) -> Vec<Src> {
    let mut out: Vec<Src> = (it.start..it.end).map(Src::Old).collect();
    let mut set = |f: &RepoField, v: &str| {
        let text = replace_value(doc.text(f.line), f.value_at, Some(&encode_scalar(v)));
        if text != doc.text(f.line) {
            out[f.line - it.start] = Src::Edit(f.line, text);
        }
    };
    if it.name.as_deref() != Some(r.name.as_str())
        && let Some(f) = &it.name_field
    {
        set(f, &r.name);
    }
    if it.path.as_deref().unwrap_or("") != r.path {
        match &it.path_field {
            Some(f) => set(f, &r.path),
            None => {
                let at = it.name_field.as_ref().map_or(it.end, |f| f.end);
                let prefix = it.key_prefix.clone().unwrap_or_else(|| item_shape(it).1);
                out.insert(
                    at - it.start,
                    Src::New(format!("{prefix}sciezka: {}", encode_scalar(&r.path))),
                );
            }
        }
    }
    out
}

/// Replace the repo list with minimal edits.
///
/// The `repozytoria:` line (including a trailing comment) and every list-level line
/// (comments and blank lines before, between or after items, unsupported lines,
/// items without `nazwa`) stay where they are. Repos of the new list are matched to
/// existing items ([`match_repos`]); a matched item keeps all its lines, and only the
/// values of its `nazwa`/`sciezka` lines change. The kept items fill their own places
/// in the order of the new list (so a reorder moves whole items and leaves list-level
/// text in place); an item moved to a place indented differently is re-indented as a
/// block. Removed items lose their lines (and one blank separator line). New items go
/// right after the item before them in the new list (the first ones before the first
/// kept item, or after the last item when none is kept), shaped like the item whose
/// place they follow. An empty list removes the key unless it carries a comment or
/// other text, which are then kept, or the key occurs again further down (then it
/// stays, as `[]`, so the duplicate cannot take over).
///
/// Returns `Err(message)` when the existing value is not a block list (for example a
/// flow list `[{…}]`), which Morwisko cannot edit without losing it.
pub(crate) fn set_repos(doc: &mut Doc, repos: &[Repo], after: &[&str]) -> Result<(), String> {
    let outline = Outline::of(doc);
    let fm = Frontmatter::of(doc, &outline);
    let default_item = |r: &Repo| {
        vec![
            format!("  - nazwa: {}", encode_scalar(&r.name)),
            format!("    sciezka: {}", encode_scalar(&r.path)),
        ]
    };
    let Some(fm) = fm else {
        if !repos.is_empty() {
            let mut lines = vec!["---".to_string(), format!("{REPOS_KEY}:")];
            lines.extend(repos.iter().flat_map(default_item));
            lines.push("---".into());
            doc.insert(0, lines);
        }
        return Ok(());
    };
    let Some(e) = fm.entry(REPOS_KEY).cloned() else {
        if !repos.is_empty() {
            let at = after
                .iter()
                .filter_map(|k| fm.entry(k))
                .map(|e| e.end)
                .max()
                .unwrap_or(fm.close);
            let mut lines = vec![format!("{REPOS_KEY}:")];
            lines.extend(repos.iter().flat_map(default_item));
            doc.insert(at, lines);
        }
        return Ok(());
    };

    // Header: `repozytoria:` with an optional `[]` and an optional comment.
    let header = doc.text(e.line).to_string();
    let colon = key_colon(&header).expect("entry line has a colon");
    let (empty_flow, comment) = repos_header(&header[colon + 1..]).map_err(|v| {
        format!(
            "`{REPOS_KEY}` is not a block list (`{v}`); Morwisko cannot change it without losing it, edit it by hand"
        )
    })?;

    let (items, _) = parse_repos(doc, &fm);
    if repos_from_items(&items) == repos {
        return Ok(());
    }

    // A new item is shaped like the item whose place it follows (the first place for
    // new first items; the last item when no item is kept), so it lines up with its
    // neighbours whatever their indentation.
    let new_item = |shape: Option<&RepoItem>, r: &Repo| {
        let (dash_prefix, key_prefix) =
            shape.map_or_else(|| ("  - ".to_string(), "    ".to_string()), item_shape);
        vec![
            Src::New(format!("{dash_prefix}nazwa: {}", encode_scalar(&r.name))),
            Src::New(format!("{key_prefix}sciezka: {}", encode_scalar(&r.path))),
        ]
    };
    let text_of = |s: &Src| match s {
        Src::Old(l) => doc.text(*l).to_string(),
        Src::Edit(_, t) | Src::New(t) => t.clone(),
    };

    // The kept items, in file order, are the places; they are filled in list order.
    // Each place keeps its indentation: an item moved into it is re-indented, so the
    // list keeps the structure it had (`item_end` never lets a deeper item follow).
    let to = match_repos(&items, repos);
    let mut places: Vec<usize> = to.iter().flatten().copied().collect();
    places.sort_unstable();
    let place_item = |p: usize| &items[places[p]];
    let mut fill: Vec<Vec<Src>> = vec![Vec::new(); places.len()];
    let mut front: Vec<Src> = Vec::new();
    let mut cur: Option<usize> = None;
    let mut next_place = 0;
    for (r, m) in repos.iter().zip(&to) {
        match m {
            Some(k) => {
                let (it, dest) = (&items[*k], &place_item(next_place).indent);
                let mut lines = edit_item(doc, it, r);
                if it.indent != *dest {
                    for s in &mut lines {
                        let moved = reindent(&text_of(s), &it.indent, dest);
                        *s = match s {
                            Src::Old(l) | Src::Edit(l, _) if *doc.text(*l) == moved => Src::Old(*l),
                            Src::Old(l) | Src::Edit(l, _) => Src::Edit(*l, moved),
                            Src::New(_) => Src::New(moved),
                        };
                    }
                }
                fill[next_place] = lines;
                cur = Some(next_place);
                next_place += 1;
            }
            None => match cur {
                Some(p) => fill[p].extend(new_item(Some(place_item(p)), r)),
                None if places.is_empty() => {
                    front.extend(new_item(items.iter().max_by_key(|it| it.start), r));
                }
                None => front.extend(new_item(Some(place_item(0)), r)),
            },
        }
    }
    if let Some(first) = fill.first_mut() {
        front.append(first);
        *first = front;
        front = Vec::new();
    }

    // The entry's lines with the items replaced. `None` marks a removed item.
    let mut pieces: Vec<Option<Src>> = Vec::new();
    let mut after_items = None;
    let mut i = e.line + 1;
    while i < e.end {
        let Some(k) = items.iter().position(|it| it.start == i) else {
            pieces.push(Some(Src::Old(i)));
            i += 1;
            continue;
        };
        let it = &items[k];
        if let Some(p) = places.iter().position(|&x| x == k) {
            pieces.extend(std::mem::take(&mut fill[p]).into_iter().map(Some));
        } else if it.name.is_some() {
            pieces.push(None);
        } else {
            pieces.extend((it.start..it.end).map(|l| Some(Src::Old(l))));
        }
        after_items = Some(pieces.len());
        i = it.end;
    }
    if !front.is_empty() {
        let at = after_items.unwrap_or(pieces.len());
        pieces.splice(at..at, front.into_iter().map(Some));
    }

    // A removed item takes one blank separator line with it.
    let is_blank = |p: &Option<Src>| p.as_ref().is_some_and(|s| text_of(s).trim().is_empty());
    let mut drop = vec![false; pieces.len()];
    for p in 0..pieces.len() {
        if pieces[p].is_some() {
            continue;
        }
        let live = |q: &usize| pieces[*q].is_some() && !drop[*q];
        let prev = (0..p).rev().find(live);
        let next = (p + 1..pieces.len()).find(live);
        match next {
            Some(q) if is_blank(&pieces[q]) && prev.is_none_or(|q| is_blank(&pieces[q])) => {
                drop[q] = true;
            }
            None => {
                if let Some(q) = prev.filter(|q| is_blank(&pieces[*q])) {
                    drop[q] = true;
                }
            }
            _ => {}
        }
    }
    let body: Vec<Src> = pieces
        .into_iter()
        .zip(drop)
        .filter_map(|(p, d)| if d { None } else { p })
        .collect();

    let mut lines = Vec::new();
    if repos.is_empty() && comment.is_empty() && body.iter().all(|s| text_of(s).trim().is_empty()) {
        if !fm.has_duplicate(REPOS_KEY) {
            doc.remove(e.line..e.end);
            return Ok(());
        }
        // Removing the key would let its duplicate below take over: keep it as an
        // explicit empty list instead.
        lines.push(if empty_flow {
            Src::Old(e.line)
        } else {
            Src::Edit(e.line, format!("{} []", &header[..colon + 1]))
        });
    } else {
        lines.push(if empty_flow && !repos.is_empty() {
            Src::Edit(
                e.line,
                format!(
                    "{}{}",
                    &header[..colon + 1],
                    &header[colon + 1..].trim_start_matches(WS)[2..]
                ),
            )
        } else {
            Src::Old(e.line)
        });
        lines.extend(body);
    }
    let unchanged = lines.len() == e.end - e.line
        && lines
            .iter()
            .enumerate()
            .all(|(n, s)| *s == Src::Old(e.line + n));
    if !unchanged {
        doc.rearrange(e.line..e.end, lines);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fm(doc: &Doc) -> Frontmatter {
        Frontmatter::of(doc, &Outline::of(doc)).unwrap()
    }

    #[test]
    fn scalars() {
        assert_eq!(
            parse_scalar(" aktywny").unwrap().0.as_deref(),
            Some("aktywny")
        );
        assert_eq!(
            parse_scalar(" aktywny   # komentarz").unwrap().0.as_deref(),
            Some("aktywny")
        );
        assert_eq!(
            parse_scalar(" \"#a3e635\"").unwrap().0.as_deref(),
            Some("#a3e635")
        );
        assert_eq!(
            parse_scalar(" '#a3e635' # c").unwrap().0.as_deref(),
            Some("#a3e635")
        );
        assert_eq!(parse_scalar(" 'it''s'").unwrap().0.as_deref(), Some("it's"));
        assert_eq!(
            parse_scalar(" \"a \\\"b\\\" \\\\ c\"")
                .unwrap()
                .0
                .as_deref(),
            Some("a \"b\" \\ c")
        );
        assert_eq!(parse_scalar(" #a3e635").unwrap().0, None);
        assert_eq!(parse_scalar("").unwrap().0, None);
        assert_eq!(parse_scalar(" ~").unwrap().0, None);
        assert_eq!(parse_scalar(" a#b").unwrap().0.as_deref(), Some("a#b"));
        assert!(parse_scalar(" \"open").is_err());
        assert!(parse_scalar(" \"x\" y").is_err());
        assert!(parse_scalar(" |").is_err());
    }

    #[test]
    fn encode() {
        assert_eq!(encode_scalar("WhenUBuy"), "WhenUBuy");
        assert_eq!(encode_scalar("#a3e635"), "\"#a3e635\"");
        assert_eq!(encode_scalar("a: b"), "\"a: b\"");
        assert_eq!(encode_scalar("2026"), "\"2026\"");
        assert_eq!(encode_scalar("yes"), "\"yes\"");
        assert_eq!(encode_scalar(""), "\"\"");
        assert_eq!(encode_scalar("Zażółć"), "Zażółć");
        assert_eq!(encode_scalar("/Users/x/Projects"), "/Users/x/Projects");
        for s in ["a \"q\" \\", "x\ny", "#", " lead", "-x", "trail:"] {
            let enc = encode_scalar(s);
            assert_eq!(
                parse_scalar(&format!(" {enc}")).unwrap().0.as_deref(),
                Some(s),
                "{enc}"
            );
        }
    }

    #[test]
    fn entries_and_continuations() {
        let d = Doc::parse(
            "---\na: 1\n# comment\nrepozytoria:\n  - nazwa: x\n    sciezka: /p\n\n  - nazwa: y\nb: 2\n---\n",
        );
        let f = fm(&d);
        let keys: Vec<_> = f
            .entries
            .iter()
            .map(|e| (e.key.as_str(), e.line, e.end))
            .collect();
        assert_eq!(keys, vec![("a", 1, 2), ("repozytoria", 3, 8), ("b", 8, 9)]);
        let (items, problems) = parse_repos(&d, &f);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            repos_from_items(&items),
            vec![
                Repo {
                    name: "x".into(),
                    path: "/p".into()
                },
                Repo {
                    name: "y".into(),
                    path: "".into()
                }
            ]
        );
    }

    #[test]
    fn set_scalar_keeps_comment_and_position() {
        let mut d = Doc::parse("---\nstan: aktywny   # aktywny | archiwum\nnazwa: X\n---\n");
        set_scalar(&mut d, "stan", Some("archiwum"), &[]);
        assert_eq!(
            d.render(),
            "---\nstan: archiwum   # aktywny | archiwum\nnazwa: X\n---\n"
        );
        set_scalar(&mut d, "kolor", Some("\"#fff\""), &["nazwa"]);
        assert_eq!(
            d.render(),
            "---\nstan: archiwum   # aktywny | archiwum\nnazwa: X\nkolor: \"#fff\"\n---\n"
        );
        set_scalar(&mut d, "nazwa", None, &[]);
        assert_eq!(
            d.render(),
            "---\nstan: archiwum   # aktywny | archiwum\nkolor: \"#fff\"\n---\n"
        );
        let mut d = Doc::parse("# T\n");
        set_scalar(&mut d, "status", Some("w-toku"), &[]);
        assert_eq!(d.render(), "---\nstatus: w-toku\n---\n# T\n");
    }

    #[test]
    fn set_repos_reuses_lines() {
        let src = "---\nnazwa: X\nrepozytoria:\n  - nazwa: a   # main\n    sciezka: /a\n    extra: keep\n  - nazwa: b\n    sciezka: /b\n---\n";
        let mut d = Doc::parse(src);
        let a = Repo {
            name: "a".into(),
            path: "/a".into(),
        };
        let c = Repo {
            name: "c".into(),
            path: "/c d".into(),
        };
        set_repos(&mut d, &[a.clone(), c], &[]).unwrap();
        assert_eq!(
            d.render(),
            "---\nnazwa: X\nrepozytoria:\n  - nazwa: a   # main\n    sciezka: /a\n    extra: keep\n  - nazwa: c\n    sciezka: /c d\n---\n"
        );
        set_repos(&mut d, &[], &[]).unwrap();
        assert_eq!(d.render(), "---\nnazwa: X\n---\n");
        set_repos(&mut d, &[a], &["nazwa"]).unwrap();
        assert_eq!(
            d.render(),
            "---\nnazwa: X\nrepozytoria:\n  - nazwa: a\n    sciezka: /a\n---\n"
        );
    }

    #[test]
    fn set_repos_reorders_into_existing_places() {
        let src = "---\nrepozytoria:\n  # first\n  - nazwa: a\n    sciezka: /a   # a!\n  # second\n  - nazwa: b\n    sciezka: /b\n---\n";
        let mut d = Doc::parse(src);
        let r = |n: &str, p: &str| Repo {
            name: n.into(),
            path: p.into(),
        };
        set_repos(&mut d, &[r("b", "/b"), r("a", "/a")], &[]).unwrap();
        assert_eq!(
            d.render(),
            "---\nrepozytoria:\n  # first\n  - nazwa: b\n    sciezka: /b\n  # second\n  - nazwa: a\n    sciezka: /a   # a!\n---\n"
        );
        let (items, _) = parse_repos(&d, &fm(&d));
        assert_eq!(repos_from_items(&items), vec![r("b", "/b"), r("a", "/a")]);
    }

    #[test]
    fn nested_lines_belong_to_their_item() {
        // A nested list under an unknown key is not a list of items, and a comment
        // at the dash's indentation does not end the item.
        let d = Doc::parse(
            "---\nrepozytoria:\n  - nazwa: a\n    extra:\n      - nazwa: ghost\n  # c\n    sciezka: /a\n  - nazwa: b\n---\n",
        );
        let (items, problems) = parse_repos(&d, &fm(&d));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            repos_from_items(&items),
            vec![
                Repo {
                    name: "a".into(),
                    path: "/a".into()
                },
                Repo {
                    name: "b".into(),
                    path: "".into()
                }
            ]
        );
        assert_eq!((items[0].start, items[0].end), (2, 7));
        // Lines below a field, duplicates and lines outside any item are reported.
        let d = Doc::parse(
            "---\nrepozytoria:\n  - nazwa: a\n    sciezka: /x\n      more\n    sciezka: /y\n  sciezka: /z\n---\n",
        );
        let (items, problems) = parse_repos(&d, &fm(&d));
        assert_eq!(items[0].path.as_deref(), Some("/y"));
        let lines: Vec<usize> = problems.iter().map(|p| p.0).collect();
        assert_eq!(lines, vec![3, 5, 6], "{problems:?}");
    }

    #[test]
    fn replace_value_keeps_spacing_and_comment() {
        assert_eq!(replace_value("k: a   # c", 2, Some("b")), "k: b   # c");
        assert_eq!(replace_value("k:   # c", 2, Some("b")), "k: b   # c");
        assert_eq!(replace_value("k: # c", 2, Some("b")), "k: b # c");
        assert_eq!(replace_value("k:", 2, Some("b")), "k: b");
        assert_eq!(replace_value("k:  \"a\"# c", 2, Some("b")), "k:  b # c");
        assert_eq!(replace_value("k: a   # c", 2, None), "k:   # c");
        assert_eq!(replace_value("k:   # c", 2, None), "k:   # c");
        assert_eq!(replace_value("k: \"open", 2, Some("b")), "k: b");
    }

    #[test]
    fn repos_flow_empty_and_unsupported() {
        let d = Doc::parse("---\nrepozytoria: []\n---\n");
        let (items, problems) = parse_repos(&d, &fm(&d));
        assert!(items.is_empty() && problems.is_empty());
        let d = Doc::parse("---\nrepozytoria: [{nazwa: a}]\n---\n");
        let (_, problems) = parse_repos(&d, &fm(&d));
        assert_eq!(problems.len(), 1);
    }
}
