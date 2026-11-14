//! Which characters of a label matched what was typed, for marking them in
//! the lists of the command field. Matching ignores case and Polish characters
//! (`fold`) and tolerates one typo in words of four letters or more, like the
//! search; the ranges are of the original label, in UTF-16 code units.

use crate::slug::fold;

use super::parse::Span;
use super::resolve::within_one;

/// One folded character and the original character it came from.
struct Folded {
    chars: Vec<char>,
    /// UTF-16 span in the original text of each folded char.
    spans: Vec<Span>,
}

fn folded(text: &str) -> Folded {
    let mut chars = Vec::new();
    let mut spans = Vec::new();
    let mut pos = 0usize;
    for c in text.chars() {
        let len = c.len_utf16();
        let span = Span {
            from: pos,
            to: pos + len,
        };
        for f in fold(&c.to_string()).chars() {
            chars.push(f);
            spans.push(span);
        }
        pos += len;
    }
    Folded { chars, spans }
}

/// The words of what was typed, folded (any non-alphanumeric splits).
pub fn terms(query: &str) -> Vec<Vec<char>> {
    fold(query)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.chars().collect())
        .collect()
}

fn find(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

/// Word runs (alphanumeric) of the folded label: `[start, end)` char indices.
fn words(chars: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in chars.iter().enumerate() {
        match (c.is_alphanumeric(), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, chars.len()));
    }
    out
}

/// `term` against the start of `word` with at most one edit: the char ranges
/// of `word` (relative) that line up with `term`.
fn fuzzy(term: &[char], word: &[char]) -> Option<Vec<(usize, usize)>> {
    if term.len() < 4 {
        return None;
    }
    let lens = [
        term.len(),
        term.len() + 1,
        term.len().saturating_sub(1),
        word.len(),
    ];
    for &n in &lens {
        if n == 0 || n > word.len() {
            continue;
        }
        let cand: String = word[..n].iter().collect();
        let t: String = term.iter().collect();
        if within_one(&t, &cand) {
            let w = &word[..n];
            let p = term.iter().zip(w).take_while(|(a, b)| a == b).count();
            let s = term
                .iter()
                .rev()
                .zip(w.iter().rev())
                .take_while(|(a, b)| a == b)
                .count()
                .min(n - p.min(n));
            let mut out = Vec::new();
            if p > 0 {
                out.push((0, p));
            }
            if s > 0 {
                out.push((n - s, n));
            }
            return Some(out);
        }
    }
    None
}

/// The ranges of `label` (UTF-16) that match the words of `query`: a word
/// start first, then anywhere, then one typo away. Merged and sorted.
pub fn mark_ranges(label: &str, query: &str) -> Vec<Span> {
    let ts = terms(query);
    if ts.is_empty() || label.is_empty() {
        return Vec::new();
    }
    let f = folded(label);
    let ws = words(&f.chars);
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for t in &ts {
        if let Some(&(s, _)) = ws
            .iter()
            .find(|(s, e)| e - s >= t.len() && f.chars[*s..s + t.len()] == **t)
        {
            ranges.push((s, s + t.len()));
            continue;
        }
        if let Some(i) = find(&f.chars, t, 0) {
            ranges.push((i, i + t.len()));
            continue;
        }
        for &(s, e) in &ws {
            if let Some(parts) = fuzzy(t, &f.chars[s..e]) {
                ranges.extend(parts.into_iter().map(|(a, b)| (s + a, s + b)));
                break;
            }
        }
    }
    let mut spans: Vec<Span> = ranges
        .into_iter()
        .filter(|(a, b)| a < b)
        .map(|(a, b)| Span {
            from: f.spans[a].from,
            to: f.spans[b - 1].to,
        })
        .collect();
    spans.sort_by_key(|s| (s.from, s.to));
    let mut out: Vec<Span> = Vec::new();
    for s in spans {
        match out.last_mut() {
            Some(last) if s.from <= last.to => last.to = last.to.max(s.to),
            _ => out.push(s),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked(label: &str, query: &str) -> Vec<String> {
        let units: Vec<u16> = label.encode_utf16().collect();
        mark_ranges(label, query)
            .into_iter()
            .map(|s| String::from_utf16(&units[s.from..s.to]).unwrap())
            .collect()
    }

    #[test]
    fn word_starts_case_and_polish_characters() {
        assert_eq!(
            marked("WhenUBuy / Płatności", "whenubuy/pła"),
            ["WhenUBuy", "Pła"]
        );
        assert_eq!(marked("Mapa myśli", "mapa-mysl"), ["Mapa", "myśl"]);
        assert_eq!(marked("Umowa z dostawcą", "DOSTAWCA"), ["dostawcą"]);
        assert_eq!(marked("/czeka", "/cz"), ["cz"]);
        // Inside a word when no word starts with it.
        assert_eq!(marked("Checkout", "kout"), ["kout"]);
    }

    #[test]
    fn a_typo_marks_the_letters_that_line_up() {
        // "dostwca": one letter missing in "dostawca".
        assert_eq!(marked("Umowa z dostawcą", "dostwca"), ["dost", "wcą"]);
        // A typo while typing: a prefix one edit away.
        assert_eq!(marked("Płatności", "platm"), ["Płat"]);
        assert!(marked("Firma", "xyz").is_empty());
    }

    #[test]
    fn utf16_offsets() {
        assert_eq!(mark_ranges("😀 Kot", "kot"), [Span { from: 3, to: 6 }]);
    }
}
