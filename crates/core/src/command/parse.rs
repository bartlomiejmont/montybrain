//! The command language, syntax only (no vault): one line of text into
//! [`Syntax`]. Spans are in UTF-16 code units, the unit of a JavaScript string
//! index, so the app can underline and replace parts of the field.
//!
//! ```text
//! Zadzwonić do sądu @jutro !wysoki            new task (Skrzynka)
//! Podpiąć karty > whenubuy/checkout           new task in a topic
//! [/] Dokończyć opis > mapa-mysli             new task, doing
//! /temat Płatności mobilne > whenubuy         new topic
//! /temat ^a1b2c3 > firma                      topic from an entry
//! ^a1b2c3 > whenubuy/checkout                 move
//! ^a1b2c3 [x] !wysoki @2026-11-20             status, priority, due
//! ^a1b2c3 /czeka ^c3d4e5                      "Czeka na: …" line
//! [[whenubuy/checkout]] /powiaz [[firma/krs]] "Powiązane: …" line in a summary
//! ?rozpisz ^a1b2c3                            request to the agent
//! ```
//!
//! Rules: ` > ` must be a word of its own (`a>b` is text); `\>` and a leading
//! `\?` are a literal `>` / `?`. In a new task's title only trailing `!prio`
//! and `@date` words count (like in the files), and `@` before something that
//! is not a date stays text. Unknown `/words` are text.

use crate::id::is_valid_id;
use crate::model::{Priority, TaskStatus};
use crate::slug::fold;

use super::dates::{DueWord, looks_like_bad_date, parse_due_word};

/// `[from, to)` in UTF-16 code units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Span {
    pub from: usize,
    pub to: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Temat,
    Czeka,
    Powiaz,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tok {
    Status(TaskStatus),
    Priority(Priority),
    Due(DueWord),
    Id(String),
    /// `[[…]]`, the inside trimmed.
    TopicRef(String),
    Arrow,
    Cmd(Cmd),
    Word(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub tok: Tok,
    /// As typed.
    pub raw: String,
    pub span: Span,
}

pub fn priority_word(w: &str) -> Option<Priority> {
    match fold(w).as_str() {
        "wysoki" | "w" | "pilne" | "pilny" | "high" | "urgent" => Some(Priority::High),
        "niski" | "n" | "low" => Some(Priority::Low),
        "normalny" | "normal" => Some(Priority::Normal),
        _ => None,
    }
}

fn status_word(w: &str) -> Option<TaskStatus> {
    match w {
        "[ ]" => Some(TaskStatus::Todo),
        "[/]" => Some(TaskStatus::Doing),
        "[x]" | "[X]" => Some(TaskStatus::Done),
        "[!]" => Some(TaskStatus::Blocked),
        _ => None,
    }
}

fn cmd_word(w: &str) -> Option<Cmd> {
    match fold(w).as_str() {
        "/temat" | "/topic" => Some(Cmd::Temat),
        "/czeka" | "/wait" => Some(Cmd::Czeka),
        "/powiaz" | "/powiazane" | "/link" | "/related" => Some(Cmd::Powiaz),
        _ => None,
    }
}

fn classify(raw: &str) -> Tok {
    if raw == ">" {
        return Tok::Arrow;
    }
    if let Some(s) = status_word(raw) {
        return Tok::Status(s);
    }
    if let Some(c) = cmd_word(raw) {
        return Tok::Cmd(c);
    }
    if let Some(body) = raw.strip_prefix("[[").and_then(|r| r.strip_suffix("]]")) {
        let body = body.trim();
        if !body.is_empty() && !body.contains('[') && !body.contains(']') {
            return Tok::TopicRef(body.to_string());
        }
    }
    if let Some(rest) = raw.strip_prefix('^')
        && is_valid_id(rest)
    {
        return Tok::Id(rest.to_string());
    }
    if let Some(rest) = raw.strip_prefix('!')
        && let Some(p) = priority_word(rest)
    {
        return Tok::Priority(p);
    }
    if let Some(rest) = raw.strip_prefix('@')
        && let Some(d) = parse_due_word(rest)
    {
        return Tok::Due(d);
    }
    let text = raw.replace("\\>", ">");
    Tok::Word(text)
}

/// Words with their spans. `[ ]` (one word for the "todo" box) and `[[a b]]`
/// (a reference with spaces) are joined.
pub fn tokenize(text: &str) -> Vec<Token> {
    // Split on whitespace, keeping UTF-16 offsets.
    let mut words: Vec<(String, Span)> = Vec::new();
    let mut cur = String::new();
    let mut start = 0usize;
    let mut pos = 0usize;
    for c in text.chars() {
        let len = c.len_utf16();
        if c.is_whitespace() {
            if !cur.is_empty() {
                words.push((
                    std::mem::take(&mut cur),
                    Span {
                        from: start,
                        to: pos,
                    },
                ));
            }
        } else {
            if cur.is_empty() {
                start = pos;
            }
            cur.push(c);
        }
        pos += len;
    }
    if !cur.is_empty() {
        words.push((
            cur,
            Span {
                from: start,
                to: pos,
            },
        ));
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let (w, span) = &words[i];
        if w == "[" && words.get(i + 1).is_some_and(|(n, _)| n == "]") {
            let to = words[i + 1].1.to;
            out.push(Token {
                tok: Tok::Status(TaskStatus::Todo),
                raw: "[ ]".into(),
                span: Span {
                    from: span.from,
                    to,
                },
            });
            i += 2;
            continue;
        }
        if w.starts_with("[[")
            && !w.contains("]]")
            && let Some(k) = (i + 1..words.len()).find(|&k| words[k].0.contains("]]"))
        {
            let raw = text_slice(
                text,
                Span {
                    from: span.from,
                    to: words[k].1.to,
                },
            );
            let tok = classify(&raw);
            if matches!(tok, Tok::TopicRef(_)) {
                out.push(Token {
                    tok,
                    raw,
                    span: Span {
                        from: span.from,
                        to: words[k].1.to,
                    },
                });
                i = k + 1;
                continue;
            }
        }
        out.push(Token {
            tok: classify(w),
            raw: w.clone(),
            span: *span,
        });
        i += 1;
    }
    out
}

/// The part of `text` covered by `span` (UTF-16 units).
pub fn text_slice(text: &str, span: Span) -> String {
    let mut out = String::new();
    let mut pos = 0;
    for c in text.chars() {
        if pos >= span.from && pos < span.to {
            out.push(c);
        }
        pos += c.len_utf16();
    }
    out
}

/// A reference after `/czeka` or `/powiaz` (or the subject of an edit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefTok {
    Task(String),
    Topic(String),
    /// Plain words: a task or topic found by its name (`/czeka umowa`).
    Search(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefArg {
    pub r: RefTok,
    pub span: Span,
}

/// Destination text after ` > ` (words joined by single spaces).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestArg {
    pub text: String,
    pub span: Span,
}

/// What the tail segments of any form can carry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tail {
    pub dest: Option<DestArg>,
    pub waits: Vec<RefArg>,
    pub related: Vec<RefArg>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attrs {
    pub status: Option<(TaskStatus, Span)>,
    pub priority: Option<(Priority, Span)>,
    pub due: Option<(DueWord, Span)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentVerb {
    Rozpisz,
    Dopasuj,
    Free,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    /// `^id …` (several ids: a bulk edit), or the selection in the app.
    Tasks(Vec<(String, Span)>),
    Topic(String, Span),
    /// Plain words before `/czeka` or `/powiaz` (and no ` > `): an existing
    /// task or topic found by its name. Nothing found: a new task.
    Search(String, Span),
    /// Nothing written before the first `>` or `/command`: the entries
    /// selected in the app, else the focused one.
    Selection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Syntax {
    Empty,
    Agent {
        verb: AgentVerb,
        text: String,
        ids: Vec<String>,
    },
    NewTask {
        title: String,
        attrs: Attrs,
        tail: Tail,
    },
    NewTopic {
        title: String,
        ids: Vec<(String, Span)>,
        tail: Tail,
    },
    Edit {
        subject: Subject,
        attrs: Attrs,
        tail: Tail,
    },
}

/// The syntax tree plus problems that make it unusable (nothing can be
/// written) and warnings (shown, but it can still be written).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub syntax: Syntax,
    pub problems: Vec<String>,
    pub warnings: Vec<String>,
}

fn put<T>(slot: &mut Option<(T, Span)>, v: T, span: Span, what: &str, problems: &mut Vec<String>) {
    if slot.is_some() {
        problems.push(format!("{what} podano dwa razy."));
    } else {
        *slot = Some((v, span));
    }
}

/// Attribute tokens anywhere in `toks` (edit subjects, the destination segment).
/// Returns the words that are not attributes.
fn take_attrs(toks: &[Token], attrs: &mut Attrs, problems: &mut Vec<String>) -> Vec<Token> {
    let mut rest = Vec::new();
    for t in toks {
        match &t.tok {
            Tok::Status(s) => put(&mut attrs.status, *s, t.span, "Status", problems),
            Tok::Priority(p) => put(&mut attrs.priority, *p, t.span, "Priorytet", problems),
            Tok::Due(d) => put(&mut attrs.due, *d, t.span, "Termin", problems),
            _ => rest.push(t.clone()),
        }
    }
    rest
}

fn warn_lookalikes(words: &[Token], warnings: &mut Vec<String>) {
    for w in words {
        if let Some(rest) = w.raw.strip_prefix('@')
            && looks_like_bad_date(rest)
        {
            warnings.push(format!(
                "„{}” nie jest poprawną datą, więc zostaje w tytule.",
                w.raw
            ));
        }
        if let Some(rest) = w.raw.strip_prefix('^')
            && rest.len() >= 4
            && rest.len() <= 8
            && rest.chars().all(|c| c.is_ascii_alphanumeric())
            && !is_valid_id(rest)
        {
            warnings.push(format!(
                "„{}” nie jest identyfikatorem zadania (6 małych liter lub cyfr), więc zostaje w tytule.",
                w.raw
            ));
        }
    }
}

/// Split the tail (`> dest`, `/czeka …`, `/powiaz …`) off the tokens.
fn parse_tail(toks: &[Token], tail: &mut Tail, attrs: &mut Attrs, problems: &mut Vec<String>) {
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        let end = (i + 1..toks.len())
            .find(|&k| matches!(toks[k].tok, Tok::Arrow | Tok::Cmd(_)))
            .unwrap_or(toks.len());
        let seg = &toks[i + 1..end];
        match &t.tok {
            Tok::Arrow => {
                let words = take_attrs(seg, attrs, problems);
                if tail.dest.is_some() {
                    problems.push("Miejsce docelowe (` > `) może być tylko jedno.".into());
                } else if words.is_empty() {
                    problems.push(
                        "Po ` > ` wpisz projekt albo temat, np. `> whenubuy/checkout`.".into(),
                    );
                } else {
                    let span = Span {
                        from: words[0].span.from,
                        to: words[words.len() - 1].span.to,
                    };
                    let text = words
                        .iter()
                        .map(|w| w.raw.replace("\\>", ">"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    tail.dest = Some(DestArg { text, span });
                }
            }
            Tok::Cmd(c @ (Cmd::Czeka | Cmd::Powiaz)) => {
                let list = if *c == Cmd::Czeka {
                    &mut tail.waits
                } else {
                    &mut tail.related
                };
                let name = if *c == Cmd::Czeka {
                    "/czeka"
                } else {
                    "/powiaz"
                };
                let seg = take_attrs(seg, attrs, problems);
                if seg.is_empty() {
                    problems.push(format!(
                        "Po {name} podaj zadanie albo temat: jego nazwę, `^id` albo `[[projekt/temat]]`."
                    ));
                }
                // Plain words in a row are one search (`/czeka umowa z dostawcą`).
                let mut words: Vec<&Token> = Vec::new();
                let flush = |words: &mut Vec<&Token>, list: &mut Vec<RefArg>| {
                    if let (Some(first), Some(last)) = (words.first(), words.last()) {
                        let text = words
                            .iter()
                            .map(|w| match &w.tok {
                                Tok::Word(s) => s.clone(),
                                _ => w.raw.clone(),
                            })
                            .collect::<Vec<_>>()
                            .join(" ");
                        list.push(RefArg {
                            r: RefTok::Search(text),
                            span: Span {
                                from: first.span.from,
                                to: last.span.to,
                            },
                        });
                    }
                    words.clear();
                };
                for w in &seg {
                    match &w.tok {
                        Tok::Id(id) => {
                            flush(&mut words, list);
                            list.push(RefArg {
                                r: RefTok::Task(id.clone()),
                                span: w.span,
                            })
                        }
                        Tok::TopicRef(b) => {
                            flush(&mut words, list);
                            list.push(RefArg {
                                r: RefTok::Topic(b.clone()),
                                span: w.span,
                            })
                        }
                        Tok::Word(_) if w.raw.starts_with('^') || w.raw.starts_with("[[") => {
                            flush(&mut words, list);
                            problems.push(format!(
                                "„{}” nie jest odwołaniem: zadanie to `^` i 6 znaków, temat to `[[projekt/temat]]`.",
                                w.raw
                            ))
                        }
                        _ => words.push(w),
                    }
                }
                flush(&mut words, list);
            }
            Tok::Cmd(Cmd::Temat) => {
                problems.push("`/temat` musi być na początku.".into());
            }
            _ => unreachable!("the tail starts at an arrow or a command"),
        }
        i = end;
    }
}

/// Parse one line. Line breaks are a problem (a command is one line).
pub fn parse(text: &str) -> Parsed {
    let mut problems = Vec::new();
    let mut warnings = Vec::new();
    if text.contains('\n') || text.contains('\r') {
        problems.push("Komenda to jedna linia.".into());
    }
    let trimmed = text.trim_start();
    if trimmed.trim().is_empty() {
        return Parsed {
            syntax: Syntax::Empty,
            problems,
            warnings,
        };
    }
    if let Some(rest) = trimmed.strip_prefix('?') {
        let rest = rest.trim();
        let (first, after) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let verb = match fold(first).as_str() {
            "rozpisz" => AgentVerb::Rozpisz,
            "dopasuj" => AgentVerb::Dopasuj,
            _ => AgentVerb::Free,
        };
        let body = if verb == AgentVerb::Free {
            rest
        } else {
            after.trim()
        };
        let ids = tokenize(body)
            .into_iter()
            .filter_map(|t| match t.tok {
                Tok::Id(id) => Some(id),
                _ => None,
            })
            .collect();
        return Parsed {
            syntax: Syntax::Agent {
                verb,
                text: body.to_string(),
                ids,
            },
            problems,
            warnings,
        };
    }
    let mut toks = tokenize(text);
    // `\?` at the very start is a literal question mark.
    if let Some(first) = toks.first_mut()
        && let Some(rest) = first.raw.strip_prefix("\\?")
    {
        first.raw = format!("?{rest}");
        first.tok = Tok::Word(first.raw.clone());
    }
    let head_end = toks
        .iter()
        .position(|t| matches!(t.tok, Tok::Arrow | Tok::Cmd(Cmd::Czeka | Cmd::Powiaz)))
        .unwrap_or(toks.len());
    let (head, rest) = toks.split_at(head_end);
    let mut tail = Tail::default();
    let mut attrs = Attrs::default();

    // /temat
    if matches!(head.first().map(|t| &t.tok), Some(Tok::Cmd(Cmd::Temat))) {
        parse_tail(rest, &mut tail, &mut attrs, &mut problems);
        let mut ids = Vec::new();
        let mut words = Vec::new();
        for t in &head[1..] {
            match &t.tok {
                Tok::Id(id) => ids.push((id.clone(), t.span)),
                Tok::Status(_) | Tok::Priority(_) | Tok::Due(_) => problems.push(format!(
                    "„{}” nie pasuje do tematu: status, priorytet i termin mają zadania.",
                    t.raw
                )),
                _ => words.push(t.clone()),
            }
        }
        if attrs != Attrs::default() {
            problems.push("Status, priorytet i termin mają zadania, nie tematy.".into());
        }
        let title = words
            .iter()
            .map(|w| match &w.tok {
                Tok::Word(s) => s.clone(),
                _ => w.raw.clone(),
            })
            .collect::<Vec<_>>()
            .join(" ");
        return Parsed {
            syntax: Syntax::NewTopic { title, ids, tail },
            problems,
            warnings,
        };
    }

    parse_tail(rest, &mut tail, &mut attrs, &mut problems);

    // Edits: `^id…`, `[[topic]]`, or nothing before the tail (the selection).
    let subject = match head.first().map(|t| &t.tok) {
        None => Some(Subject::Selection),
        Some(Tok::Id(_)) => {
            let ids: Vec<(String, Span)> = head
                .iter()
                .map_while(|t| match &t.tok {
                    Tok::Id(id) => Some((id.clone(), t.span)),
                    _ => None,
                })
                .collect();
            Some(Subject::Tasks(ids))
        }
        Some(Tok::TopicRef(b)) => Some(Subject::Topic(b.clone(), head[0].span)),
        Some(Tok::Word(_))
            if tail.dest.is_none() && (!tail.waits.is_empty() || !tail.related.is_empty()) =>
        {
            // Trailing attributes belong to the found task (as in a title).
            let mut words: Vec<Token> = head.to_vec();
            while let Some(last) = words.last() {
                match &last.tok {
                    Tok::Priority(p) if attrs.priority.is_none() => {
                        attrs.priority = Some((*p, last.span))
                    }
                    Tok::Due(d) if attrs.due.is_none() => attrs.due = Some((*d, last.span)),
                    Tok::Status(s) if attrs.status.is_none() => {
                        attrs.status = Some((*s, last.span))
                    }
                    _ => break,
                }
                words.pop();
            }
            let query = words
                .iter()
                .map(|w| match &w.tok {
                    Tok::Word(s) => s.clone(),
                    _ => w.raw.clone(),
                })
                .collect::<Vec<_>>()
                .join(" ");
            let span = Span {
                from: words[0].span.from,
                to: words[words.len() - 1].span.to,
            };
            return Parsed {
                syntax: Syntax::Edit {
                    subject: Subject::Search(query, span),
                    attrs,
                    tail,
                },
                problems,
                warnings,
            };
        }
        _ => None,
    };
    if let Some(subject) = subject {
        let skip = match &subject {
            Subject::Tasks(ids) => ids.len(),
            Subject::Topic(..) => 1,
            Subject::Search(..) | Subject::Selection => 0,
        };
        let words = take_attrs(&head[skip..], &mut attrs, &mut problems);
        if !words.is_empty() {
            let text = words
                .iter()
                .map(|w| w.raw.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            problems.push(format!(
                "Nie wiem, co zrobić z „{text}”. Przy istniejącym zadaniu wpisz ` > miejsce`, status, \
`!priorytet`, `@termin`, `/czeka` albo `/powiaz`; tytuł zmienisz w szczegółach zadania."
            ));
        }
        if let Subject::Topic(..) = subject
            && attrs != Attrs::default()
        {
            problems.push("Status, priorytet i termin mają zadania, nie tematy.".into());
        }
        if let Subject::Topic(..) = subject
            && tail.dest.is_some()
        {
            problems.push("Tematu nie da się przenieść komendą.".into());
        }
        return Parsed {
            syntax: Syntax::Edit {
                subject,
                attrs,
                tail,
            },
            problems,
            warnings,
        };
    }

    // A new task: optional leading status, title, trailing !prio / @date.
    let mut words: Vec<Token> = head.to_vec();
    if let Some(Tok::Status(s)) = words.first().map(|t| t.tok.clone()) {
        attrs.status = Some((s, words[0].span));
        words.remove(0);
    }
    while let Some(last) = words.last() {
        match &last.tok {
            Tok::Priority(p) if attrs.priority.is_none() => attrs.priority = Some((*p, last.span)),
            Tok::Due(d) if attrs.due.is_none() => attrs.due = Some((*d, last.span)),
            _ => break,
        }
        words.pop();
    }
    warn_lookalikes(&words, &mut warnings);
    let title = words
        .iter()
        .map(|w| match &w.tok {
            Tok::Word(s) => s.clone(),
            _ => w.raw.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        // Only attributes (`[x]`, `!wysoki @jutro`): they apply to the selection.
        return Parsed {
            syntax: Syntax::Edit {
                subject: Subject::Selection,
                attrs,
                tail,
            },
            problems,
            warnings,
        };
    }
    Parsed {
        syntax: Syntax::NewTask { title, attrs, tail },
        problems,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_task(text: &str) -> (String, Attrs, Tail) {
        match parse(text).syntax {
            Syntax::NewTask { title, attrs, tail } => (title, attrs, tail),
            other => panic!("{text}: {other:?}"),
        }
    }

    #[test]
    fn spans_are_utf16() {
        let t = tokenize("Zadzwonić 😀 > x");
        assert_eq!(t[0].span, Span { from: 0, to: 9 });
        assert_eq!(t[1].span, Span { from: 10, to: 12 });
        assert_eq!(t[2].tok, Tok::Arrow);
        assert_eq!(t[2].span, Span { from: 13, to: 14 });
        assert_eq!(text_slice("Zadzwonić 😀 > x", t[1].span), "😀");
    }

    #[test]
    fn plain_capture() {
        let (title, a, tail) = new_task("Zadzwonić do sądu @jutro !wysoki");
        assert_eq!(title, "Zadzwonić do sądu");
        assert_eq!(a.priority.map(|p| p.0), Some(Priority::High));
        assert_eq!(a.due.map(|d| d.0), Some(DueWord::Days(1)));
        assert_eq!(tail, Tail::default());
        // The old quick-add cases (src/lib/quickadd.ts, now removed).
        let (title, a, _) = new_task("Spotkanie @2026-11-20T14:00");
        assert_eq!(title, "Spotkanie");
        assert_eq!(
            a.due.map(|d| d.0),
            Some(DueWord::Absolute("2026-11-20T14:00".parse().unwrap()))
        );
        let (title, a, _) = new_task("Dwa @jutro @dziś");
        assert_eq!(title, "Dwa @jutro");
        assert_eq!(a.due.map(|d| d.0), Some(DueWord::Days(0)));
        let (title, a, _) = new_task("Coś @dziś !niski");
        assert_eq!(title, "Coś");
        assert_eq!(a.priority.map(|p| p.0), Some(Priority::Low));
        // Aliases from the old quick-add stay.
        let (_, a, _) = new_task("x !pilne");
        assert_eq!(a.priority.map(|p| p.0), Some(Priority::High));
    }

    #[test]
    fn title_text_that_looks_like_syntax() {
        assert_eq!(new_task("Fix @mention parsing").0, "Fix @mention parsing");
        assert_eq!(new_task("Kurs 2>1 i a>b").0, "Kurs 2>1 i a>b");
        assert_eq!(new_task("Ścieżka /etc/hosts").0, "Ścieżka /etc/hosts");
        assert_eq!(new_task("Hura! Spotkanie @ania").0, "Hura! Spotkanie @ania");
        assert_eq!(new_task("Plan Q1 \\> Q2").0, "Plan Q1 > Q2");
        assert_eq!(new_task("\\?Co dalej").0, "?Co dalej");
        // Only trailing tokens count: `@jutro` in the middle is text.
        let (title, a, _) = new_task("Spotkanie @jutro z Anią");
        assert_eq!(title, "Spotkanie @jutro z Anią");
        assert!(a.due.is_none());
        // An invalid date stays text, with a warning.
        let p = parse("Meet @2026-02-30");
        assert!(matches!(&p.syntax, Syntax::NewTask { title, .. } if title == "Meet @2026-02-30"));
        assert_eq!(p.warnings.len(), 1);
        // Attributes alone apply to the selection.
        assert!(matches!(
            parse("!wysoki @jutro").syntax,
            Syntax::Edit {
                subject: Subject::Selection,
                ..
            }
        ));
    }

    #[test]
    fn routing_and_status() {
        let (title, _, tail) = new_task("Podpiąć karty > whenubuy/checkout");
        assert_eq!(title, "Podpiąć karty");
        assert_eq!(tail.dest.unwrap().text, "whenubuy/checkout");
        let (title, a, tail) = new_task("[/] Dokończyć opis > mapa-mysli");
        assert_eq!(title, "Dokończyć opis");
        assert_eq!(a.status.map(|s| s.0), Some(TaskStatus::Doing));
        assert_eq!(tail.dest.unwrap().text, "mapa-mysli");
        let (title, a, _) = new_task("[ ] Zwykłe");
        assert_eq!(title, "Zwykłe");
        assert_eq!(a.status.map(|s| s.0), Some(TaskStatus::Todo));
        // Attributes after the destination count too.
        let (_, a, tail) = new_task("Kupić farbę > remont-kuchni !wysoki");
        assert_eq!(tail.dest.unwrap().text, "remont-kuchni");
        assert_eq!(a.priority.map(|p| p.0), Some(Priority::High));
        // A multi-word destination (a display name).
        let (_, _, tail) = new_task("x > Remont kuchni");
        assert_eq!(tail.dest.unwrap().text, "Remont kuchni");
    }

    #[test]
    fn topics_edits_and_references() {
        match parse("/temat Płatności mobilne > whenubuy").syntax {
            Syntax::NewTopic { title, ids, tail } => {
                assert_eq!(title, "Płatności mobilne");
                assert!(ids.is_empty());
                assert_eq!(tail.dest.unwrap().text, "whenubuy");
            }
            o => panic!("{o:?}"),
        }
        match parse("/temat ^a1b2c3 > firma").syntax {
            Syntax::NewTopic { title, ids, .. } => {
                assert_eq!(title, "");
                assert_eq!(ids[0].0, "a1b2c3");
            }
            o => panic!("{o:?}"),
        }
        match parse("^a1b2c3 > whenubuy/checkout").syntax {
            Syntax::Edit {
                subject: Subject::Tasks(ids),
                tail,
                ..
            } => {
                assert_eq!(ids[0].0, "a1b2c3");
                assert_eq!(tail.dest.unwrap().text, "whenubuy/checkout");
            }
            o => panic!("{o:?}"),
        }
        match parse("^a1b2c3 [x]").syntax {
            Syntax::Edit { attrs, .. } => assert_eq!(attrs.status.unwrap().0, TaskStatus::Done),
            o => panic!("{o:?}"),
        }
        match parse("^a1b2c3 !wysoki @2026-11-20").syntax {
            Syntax::Edit { attrs, .. } => {
                assert_eq!(attrs.priority.unwrap().0, Priority::High);
                assert!(matches!(attrs.due.unwrap().0, DueWord::Absolute(_)));
            }
            o => panic!("{o:?}"),
        }
        match parse("^a1b2c3 /czeka ^c3d4e5").syntax {
            Syntax::Edit { tail, .. } => assert_eq!(tail.waits[0].r, RefTok::Task("c3d4e5".into())),
            o => panic!("{o:?}"),
        }
        match parse("[[whenubuy/checkout]] /czeka [[firma/krs]]").syntax {
            Syntax::Edit {
                subject: Subject::Topic(b, _),
                tail,
                ..
            } => {
                assert_eq!(b, "whenubuy/checkout");
                assert_eq!(tail.waits[0].r, RefTok::Topic("firma/krs".into()));
            }
            o => panic!("{o:?}"),
        }
        match parse("^a1b2c3 /powiaz [[whenubuy/platnosci]]").syntax {
            Syntax::Edit { tail, .. } => {
                assert_eq!(
                    tail.related[0].r,
                    RefTok::Topic("whenubuy/platnosci".into())
                )
            }
            o => panic!("{o:?}"),
        }
        // Selection forms.
        assert!(matches!(
            parse("> whenubuy").syntax,
            Syntax::Edit {
                subject: Subject::Selection,
                ..
            }
        ));
        // Bulk ids.
        match parse("^aaaaa1 ^bbbbb2 > firma").syntax {
            Syntax::Edit {
                subject: Subject::Tasks(ids),
                ..
            } => assert_eq!(ids.len(), 2),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn agent_requests() {
        assert_eq!(
            parse("?rozpisz ^a1b2c3").syntax,
            Syntax::Agent {
                verb: AgentVerb::Rozpisz,
                text: "^a1b2c3".into(),
                ids: vec!["a1b2c3".into()]
            }
        );
        assert!(matches!(
            parse("?dopasuj").syntax,
            Syntax::Agent {
                verb: AgentVerb::Dopasuj,
                ..
            }
        ));
        assert!(matches!(
            parse("? gdzie to wrzucić").syntax,
            Syntax::Agent {
                verb: AgentVerb::Free,
                ..
            }
        ));
    }

    #[test]
    fn problems() {
        assert!(!parse("^a1b2c3 Nowy tytuł").problems.is_empty());
        assert!(!parse("x > a > b").problems.is_empty());
        assert!(!parse("x >").problems.is_empty());
        assert!(!parse("^a1b2c3 /czeka ^a1").problems.is_empty());
        assert!(!parse("^a1b2c3 !wysoki !niski").problems.is_empty());
        assert!(!parse("a\nb").problems.is_empty());
        assert!(!parse("/temat X > p !wysoki").problems.is_empty());
        assert!(parse("").syntax == Syntax::Empty);
    }
}
