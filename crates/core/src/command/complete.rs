//! Autocomplete for the word under the caret: after ` > ` (places), `[[`
//! (topics), `^` (tasks), `!` (priorities), `@` (dates), `/` (commands) and a
//! leading `?` (agent requests). Offsets are UTF-16 code units.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::model::{ProjectState, TaskStatus, TopicStatus};
use crate::query::Snapshot;
use crate::slug::fold;

use super::dates::{describe_due, parse_due_word};
use super::marks::mark_ranges;
use super::parse::{Cmd, Span, Tok, text_slice, tokenize};
use super::resolve::{place_of, search, topic_place};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub label: String,
    /// Replaces the span.
    pub insert: String,
    pub detail: String,
    /// `inbox`, `project`, `topic`, `task`, `priority`, `date`, `command`
    /// or `agent`: topics and tasks are told apart in the list.
    #[serde(default)]
    pub kind: String,
    /// The parts of `label` that match what was typed (UTF-16 ranges).
    #[serde(default)]
    pub marks: Vec<Span>,
    /// The same for `detail` (a task's id or place).
    #[serde(default)]
    pub detail_marks: Vec<Span>,
}

fn item(
    label: impl Into<String>,
    insert: impl Into<String>,
    detail: impl Into<String>,
    kind: &str,
) -> Item {
    Item {
        label: label.into(),
        insert: insert.into(),
        detail: detail.into(),
        kind: kind.into(),
        marks: Vec::new(),
        detail_marks: Vec::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completions {
    pub span: Span,
    /// `place`, `topic`, `task`, `ref` (a task or topic by name after
    /// `/czeka`, `/powiaz`, or as the subject), `priority`, `date`, `command`
    /// or `agent`.
    pub kind: String,
    pub items: Vec<Item>,
}

const LIMIT: usize = 8;

fn matches(hay: &str, needle: &str) -> bool {
    let n = fold(needle);
    n.is_empty()
        || fold(hay)
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| w.starts_with(&n))
        || fold(hay).contains(&n)
}

fn project_items(snap: &Snapshot, partial: &str) -> Vec<Item> {
    let mut out = Vec::new();
    if matches("skrzynka", partial) {
        out.push(item("Skrzynka", "skrzynka", "bez projektu", "inbox"));
    }
    for p in snap
        .projects
        .iter()
        .filter(|p| p.state != ProjectState::Archived)
    {
        if matches(&format!("{} {}", p.slug, p.name), partial) {
            out.push(item(p.name.clone(), p.slug.clone(), "projekt", "project"));
        }
    }
    out
}

/// Open topics of active projects matching `partial` (`projekt/…` narrows).
fn topic_items(snap: &Snapshot, partial: &str) -> Vec<Item> {
    let (proj, rest) = project_prefix(partial).unwrap_or(("", partial));
    let mut out = Vec::new();
    for t in snap.topics.iter().filter(|t| {
        t.status != TopicStatus::Done
            && snap
                .project(&t.project)
                .is_some_and(|p| p.state != ProjectState::Archived)
    }) {
        let in_project = proj.is_empty() || fold(&t.project).starts_with(&fold(proj));
        let hay = format!(
            "{} {} {}",
            t.slug,
            t.title,
            if proj.is_empty() { &t.project } else { "" }
        );
        if in_project && matches(&hay, rest) {
            let pl = topic_place(snap, &t.project, &t.slug);
            out.push(item(pl.label, pl.text, "temat", "topic"));
        }
    }
    out
}

/// `projekt/…` (a slug before the slash), not a name with a slash in it.
fn project_prefix(partial: &str) -> Option<(&str, &str)> {
    partial
        .split_once('/')
        .filter(|(p, _)| !p.is_empty() && !p.contains(char::is_whitespace))
}

/// Places after ` > `: at most [`MAX_PROJECTS`] projects first, the rest
/// topics, so projects never crowd the topics out (filtered before the limit).
fn places(snap: &Snapshot, partial: &str) -> Vec<Item> {
    let projects = if project_prefix(partial).is_some() {
        Vec::new()
    } else {
        project_items(snap, partial)
    };
    let topics = topic_items(snap, partial);
    let keep = if topics.is_empty() {
        LIMIT
    } else {
        MAX_PROJECTS
    };
    let mut out: Vec<Item> = projects.iter().take(keep).cloned().collect();
    out.extend(topics);
    out.truncate(LIMIT);
    if out.len() < LIMIT {
        out.extend(projects.into_iter().skip(keep).take(LIMIT - out.len()));
    }
    out
}

const MAX_PROJECTS: usize = 3;

/// Topics only, filtered before the limit (projects never crowd them out).
fn topics(snap: &Snapshot, partial: &str) -> Vec<Item> {
    let mut out: Vec<Item> = topic_items(snap, partial)
        .into_iter()
        .map(|i| Item {
            insert: format!("[[{}]]", i.insert),
            ..i
        })
        .collect();
    out.truncate(LIMIT);
    out
}

fn tasks(snap: &Snapshot, partial: &str) -> Vec<Item> {
    let mut open: Vec<_> = snap
        .tasks
        .iter()
        .filter(|t| t.status != TaskStatus::Done)
        .filter_map(|t| {
            let id = t.id.as_ref()?;
            (id.starts_with(partial) || matches(&t.title, partial)).then(|| {
                item(
                    t.title.clone(),
                    format!("^{id}"),
                    format!("^{id} · {}", place_of(snap, &t.location).label),
                    "task",
                )
            })
        })
        .collect();
    open.truncate(LIMIT);
    open
}

/// Completions for the word that ends at `caret` (`None`: nothing to offer),
/// with the matching parts of each label marked.
pub fn complete(
    snap: &Snapshot,
    text: &str,
    caret: usize,
    today: NaiveDate,
) -> Option<Completions> {
    let mut c = complete_raw(snap, text, caret, today)?;
    let typed = text_slice(
        text,
        Span {
            from: c.span.from,
            to: caret.min(c.span.to).max(c.span.from),
        },
    );
    for i in &mut c.items {
        i.marks = mark_ranges(&i.label, &typed);
        if i.kind == "task" {
            i.detail_marks = mark_ranges(&i.detail, &typed);
        }
    }
    Some(c)
}

/// Tasks and topics named by `query` (plain words), for `/czeka`, `/powiaz`
/// and the subject: what to insert is `^id` or `[[projekt/temat]]`.
fn found(snap: &Snapshot, query: &str) -> Vec<Item> {
    search(snap, query, &[])
        .into_iter()
        .take(LIMIT)
        .map(|h| {
            let insert = h.insert();
            let detail = match h.kind {
                "task" => h.place.clone(),
                _ => format!("{} · {}", h.place, insert),
            };
            item(h.title, insert, detail, h.kind)
        })
        .collect()
}

fn complete_raw(
    snap: &Snapshot,
    text: &str,
    caret: usize,
    today: NaiveDate,
) -> Option<Completions> {
    let toks = tokenize(text);
    // The word under the caret (or an empty one right after a space).
    let cur = toks
        .iter()
        .position(|t| t.span.from <= caret && caret <= t.span.to);
    let (span, word) = match cur {
        Some(i) => {
            let t = &toks[i];
            // Only the part before the caret is the prefix.
            let prefix_len = caret - t.span.from;
            let prefix: String = {
                let mut s = String::new();
                let mut n = 0;
                for c in t.raw.chars() {
                    if n >= prefix_len {
                        break;
                    }
                    s.push(c);
                    n += c.len_utf16();
                }
                s
            };
            (t.span, prefix)
        }
        None => (
            Span {
                from: caret,
                to: caret,
            },
            String::new(),
        ),
    };
    let earlier: Vec<&super::parse::Token> = toks
        .iter()
        .filter(|t| t.span != span && t.span.to <= span.from)
        .collect();
    let before = earlier.last();
    let is_first = earlier.is_empty();

    if is_first && word.starts_with('?') {
        let items = [
            ("?rozpisz", "rozpisz wpis na kroki"),
            ("?dopasuj", "miejsca dla wpisów"),
        ]
        .iter()
        .filter(|(w, _)| w.starts_with(&fold(&word)))
        .map(|(w, d)| item(*w, format!("{w} "), *d, "agent"))
        .collect::<Vec<_>>();
        return (!items.is_empty()).then(|| Completions {
            span,
            kind: "agent".into(),
            items,
        });
    }
    // After ` > ` (attributes between are skipped; a name may have several words).
    let _ = before;
    if !word.starts_with(['!', '@', '[', '^', '/'])
        && let Some(arrow) = earlier.iter().rposition(|t| t.tok == Tok::Arrow)
        && !earlier[arrow..]
            .iter()
            .any(|t| matches!(t.tok, Tok::Cmd(_)))
    {
        let dest: Vec<&super::parse::Token> = earlier[arrow + 1..]
            .iter()
            .copied()
            .filter(|t| matches!(t.tok, Tok::Word(_)))
            .collect();
        // The words of the name typed so far (attributes left out), then the
        // part of the current word before the caret.
        let mut parts: Vec<String> = dest.iter().map(|t| t.raw.clone()).collect();
        if !word.is_empty() {
            parts.push(word.clone());
        }
        let partial = parts.join(" ");
        let from = dest.first().map_or(span.from, |t| t.span.from);
        // Only a contiguous name: attributes between its words end it.
        let contiguous = dest.first().is_none_or(|first| {
            !earlier
                .iter()
                .any(|t| t.span.from > first.span.from && !matches!(t.tok, Tok::Word(_)))
        });
        let (from, partial) = if contiguous {
            (from, partial)
        } else {
            (span.from, word.clone())
        };
        let items = places(snap, &partial);
        return (!items.is_empty()).then(|| Completions {
            span: Span { from, to: span.to },
            kind: "place".into(),
            items,
        });
    }
    // Plain words after `/czeka` or `/powiaz`: tasks and topics by name.
    let plain = |w: &str| !w.starts_with(['!', '@', '[', '^', '/', '>']);
    if plain(&word)
        && let Some(cmd) = earlier
            .iter()
            .rposition(|t| matches!(t.tok, Tok::Cmd(Cmd::Czeka | Cmd::Powiaz) | Tok::Arrow))
        && matches!(earlier[cmd].tok, Tok::Cmd(_))
    {
        let words: Vec<&super::parse::Token> = earlier[cmd + 1..].to_vec();
        // Only a run of plain words since the command (a reference ends it).
        let run_from = words
            .iter()
            .rposition(|t| !matches!(t.tok, Tok::Word(_)) || !plain(&t.raw))
            .map_or(0, |i| i + 1);
        let words = &words[run_from..];
        let mut parts: Vec<String> = words.iter().map(|t| t.raw.clone()).collect();
        if !word.is_empty() {
            parts.push(word.clone());
        }
        let query = parts.join(" ");
        if query.trim().is_empty() {
            return None;
        }
        let from = words.first().map_or(span.from, |t| t.span.from);
        let items = found(snap, &query);
        return (!items.is_empty()).then(|| Completions {
            span: Span { from, to: span.to },
            kind: "ref".into(),
            items,
        });
    }
    // The subject by name: plain words at the start, before `/czeka` or
    // `/powiaz` (no ` > `), with the caret in them.
    if plain(&word)
        && !word.is_empty()
        && earlier
            .iter()
            .all(|t| matches!(t.tok, Tok::Word(_)) && plain(&t.raw))
        && toks
            .iter()
            .any(|t| t.span.from >= span.to && matches!(t.tok, Tok::Cmd(Cmd::Czeka | Cmd::Powiaz)))
        && !toks.iter().any(|t| t.tok == Tok::Arrow)
    {
        let head_end = toks
            .iter()
            .position(|t| matches!(t.tok, Tok::Cmd(_)))
            .map_or(span.to, |i| toks[..i].last().map_or(span.to, |t| t.span.to));
        let mut parts: Vec<String> = earlier.iter().map(|t| t.raw.clone()).collect();
        parts.push(word.clone());
        let query = parts.join(" ");
        let from = earlier.first().map_or(span.from, |t| t.span.from);
        let items = found(snap, &query);
        return (!items.is_empty()).then(|| Completions {
            span: Span { from, to: head_end },
            kind: "ref".into(),
            items,
        });
    }
    if let Some(rest) = word.strip_prefix("[[") {
        let rest = rest.trim_end_matches(']');
        let items = topics(snap, rest);
        return (!items.is_empty()).then(|| Completions {
            span,
            kind: "topic".into(),
            items,
        });
    }
    if let Some(rest) = word.strip_prefix('^') {
        let items = tasks(snap, rest);
        return (!items.is_empty()).then(|| Completions {
            span,
            kind: "task".into(),
            items,
        });
    }
    if let Some(rest) = word.strip_prefix('!') {
        let items = [
            ("!high", "high priority"),
            ("!low", "low priority"),
            ("!normal", "normal priority"),
            ("!wysoki", "priorytet wysoki"),
            ("!niski", "priorytet niski"),
            ("!normalny", "priorytet normalny"),
        ]
        .iter()
        .filter(|(w, _)| w[1..].starts_with(&fold(rest)))
        .map(|(w, d)| item(*w, *w, *d, "priority"))
        .collect::<Vec<_>>();
        return (!items.is_empty()).then(|| Completions {
            span,
            kind: "priority".into(),
            items,
        });
    }
    if let Some(rest) = word.strip_prefix('@') {
        let words = [
            "today", "tomorrow", "mon", "tue", "wed", "thu", "fri", "sat", "sun", "+1w",
            "dzis", "jutro", "pojutrze", "pn", "wt", "sr", "cz", "pt", "sb", "nd", "+3d", "+1t",
            "+2t",
        ];
        let items = words
            .iter()
            .filter(|w| w.starts_with(&fold(rest)))
            .filter_map(|w| {
                let due = parse_due_word(w)?.resolve(today);
                Some(item(
                    format!("@{w}"),
                    format!("@{w}"),
                    format!("{} · {due}", describe_due(&due, today)),
                    "date",
                ))
            })
            .collect::<Vec<_>>();
        return (!items.is_empty()).then(|| Completions {
            span,
            kind: "date".into(),
            items,
        });
    }
    if word.starts_with('/') {
        let items = [
            ("/topic", "new topic"),
            ("/wait", "add a waiting relation"),
            ("/link", "add a related link"),
            ("/temat", "nowy temat"),
            ("/czeka", "dopisz „Czeka na:”"),
            ("/powiaz", "dopisz „Powiązane:”"),
        ]
        .iter()
        .filter(|(w, _)| w.starts_with(&fold(&word)))
        .map(|(w, d)| item(*w, format!("{w} "), *d, "command"))
        .collect::<Vec<_>>();
        return (!items.is_empty()).then(|| Completions {
            span,
            kind: "command".into(),
            items,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::resolve::tests::snap;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 14).unwrap()
    }

    fn c(text: &str) -> Option<Completions> {
        let caret = text.chars().map(char::len_utf16).sum();
        complete(&snap(), text, caret, today())
    }

    #[test]
    fn after_the_arrow() {
        let r = c("Kupić > when").unwrap();
        assert_eq!(r.kind, "place");
        assert_eq!(r.span, Span { from: 8, to: 12 });
        assert!(r.items.iter().any(|i| i.insert == "whenubuy"));
        assert!(r.items.iter().any(|i| i.insert == "whenubuy/checkout"));
        let r = c("Kupić > ").unwrap();
        assert_eq!(r.span, Span { from: 8, to: 8 });
        assert!(r.items.len() > 3);
        let r = c("x > whenubuy/pła").unwrap();
        assert_eq!(r.items[0].insert, "whenubuy/platnosci");
        // Archived projects and done topics are not offered.
        assert!(!c("x > sta").is_some_and(|r| r.items.iter().any(|i| i.insert.contains("sta"))));
    }

    #[test]
    fn topic_references_are_not_crowded_out_by_projects() {
        let mut s = snap();
        for i in 0..12 {
            s.projects.push(crate::model::Project {
                slug: format!("krs{i}"),
                name: format!("KRS {i}"),
                color: None,
                state: ProjectState::Active,
                repos: vec![],
                description: String::new(),
                path: format!("krs{i}/projekt.md").into(),
            });
        }
        let caret = "^a /czeka [[kr".chars().map(char::len_utf16).sum();
        let r = complete(&s, "^a /czeka [[kr", caret, today()).unwrap();
        assert_eq!(r.items[0].insert, "[[firma/krs]]");
        assert!(r.items.iter().all(|i| i.detail == "temat"));
    }

    #[test]
    fn places_after_the_arrow_with_names_attributes_and_many_projects() {
        let mut s = snap();
        s.topics.push(crate::model::Topic {
            project: "whenubuy".into(),
            slug: "testy-a-b".into(),
            title: "Testy A/B".into(),
            status: TopicStatus::InProgress,
            repo: None,
            summary: String::new(),
            created: None,
            updated: None,
            path: "whenubuy/tematy/testy-a-b.md".into(),
        });
        let at = |t: &str| complete(&s, t, t.chars().map(char::len_utf16).sum(), today());
        // A multi-word name, after a space and in the middle of a word.
        let r = at("x > Testy ").unwrap();
        assert_eq!(r.items[0].insert, "whenubuy/testy-a-b");
        assert_eq!(r.span, Span { from: 4, to: 10 });
        let r = at("x > Testy A/").unwrap();
        assert_eq!(r.items[0].insert, "whenubuy/testy-a-b");
        // An attribute after the arrow does not end place completion.
        let r = at("x > !wysoki when").unwrap();
        assert!(r.items.iter().any(|i| i.insert == "whenubuy"));
        assert_eq!(r.span.from, 12);
        // Many matching projects do not crowd the topics out.
        for i in 0..12 {
            s.projects.push(crate::model::Project {
                slug: format!("check{i}"),
                name: format!("Check {i}"),
                color: None,
                state: ProjectState::Active,
                repos: vec![],
                description: String::new(),
                path: format!("check{i}/projekt.md").into(),
            });
        }
        let r = complete(&s, "x > check", 9, today()).unwrap();
        // Both checkout topics are there; the free places go to more projects.
        assert_eq!(
            r.items.iter().filter(|i| i.detail == "temat").count(),
            2,
            "{r:?}"
        );
        assert_eq!(r.items.len(), 8);
        assert!(r.items[..5].iter().filter(|i| i.detail == "temat").count() == 2);
    }

    fn snap_with_tasks() -> Snapshot {
        let mut s = snap();
        s.tasks = vec![crate::model::Task {
            id: Some("a1b2c3".into()),
            title: "Umowa z dostawcą".into(),
            status: TaskStatus::Todo,
            priority: crate::model::Priority::Normal,
            due: None,
            done: None,
            links: vec![],
            notes: String::new(),
            location: crate::model::TaskLocation::Inbox,
            line: 1,
        }];
        s
    }

    #[test]
    fn references_by_name_after_czeka_and_powiaz() {
        let s = snap_with_tasks();
        let at = |t: &str| complete(&s, t, t.encode_utf16().count(), today());
        let r = at("^a1b2c3 /czeka płat").unwrap();
        assert_eq!(r.kind, "ref");
        assert_eq!(r.span, Span { from: 15, to: 19 });
        assert_eq!(r.items[0].insert, "[[whenubuy/platnosci]]");
        assert_eq!(r.items[0].kind, "topic");
        assert_eq!(r.items[0].marks, [Span { from: 0, to: 4 }]);
        // Several words; tasks with their place.
        let r = at("/powiaz umowa z dos").unwrap();
        assert_eq!(r.span.from, 8);
        assert_eq!(r.items[0].insert, "^a1b2c3");
        assert_eq!(r.items[0].kind, "task");
        assert_eq!(r.items[0].detail, "Skrzynka");
        // A typo still finds it, with the matching letters marked.
        let r = at("/czeka dostwca").unwrap();
        assert_eq!(r.items[0].insert, "^a1b2c3");
        assert!(!r.items[0].marks.is_empty());
        // Nothing typed yet: no list (the hint says what to type).
        assert!(at("^a1b2c3 /czeka ").is_none());
    }

    #[test]
    fn the_subject_by_name() {
        let s = snap_with_tasks();
        let text = "Umo /czeka [[firma/krs]]";
        let r = complete(&s, text, 3, today()).unwrap();
        assert_eq!(r.kind, "ref");
        assert_eq!(r.span, Span { from: 0, to: 3 });
        assert_eq!(r.items[0].insert, "^a1b2c3");
        // A plain title (no /czeka): no list.
        assert!(complete(&s, "Umo", 3, today()).is_none());
    }

    #[test]
    fn matches_are_marked() {
        let r = c("x > whenubuy/pła").unwrap();
        assert_eq!(r.items[0].label, "WhenUBuy / Płatności");
        assert_eq!(
            r.items[0].marks,
            [Span { from: 0, to: 8 }, Span { from: 11, to: 14 }]
        );
        let r = c("/cz").unwrap();
        assert_eq!(r.items[0].marks, [Span { from: 1, to: 3 }]);
    }

    #[test]
    fn triggers() {
        assert_eq!(
            c("^a1 /czeka [[kr").unwrap().items[0].insert,
            "[[firma/krs]]"
        );
        assert_eq!(c("x !wy").unwrap().items[0].insert, "!wysoki");
        let d = c("x @ju").unwrap();
        assert_eq!(d.items[0].insert, "@jutro");
        assert!(d.items[0].detail.contains("2026-11-15"));
        assert_eq!(c("/te").unwrap().items[0].insert, "/temat ");
        assert_eq!(c("^a /cz").unwrap().items[0].insert, "/czeka ");
        assert_eq!(c("?ro").unwrap().items[0].insert, "?rozpisz ");
        assert!(c("Zwykły tytuł").is_none());
    }
}
