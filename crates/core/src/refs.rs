//! References between topics and tasks (docs/format.md §11a): `[[slug]]` (a
//! topic of the same project), `[[projekt/slug]]` (any project), `^id` (a
//! task). A line that starts with `Czeka na:` makes its references directed
//! ("waits for"); any other reference is "related". Read only from topic
//! summaries and task notes; fenced code and inline code are skipped; the
//! journal is never read.
//!
//! This is the reading the app's map uses (`src/lib/refs.ts`); the MCP
//! `connections` tool and the command language (`/czeka`, `/powiaz`) use it
//! from here. Pure.

use std::collections::{BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

use crate::id::is_valid_id;
use crate::model::{Task, TaskLocation};
use crate::query::Snapshot;

/// Prefix of a "waits for" line.
pub const WAITS_PREFIX: &str = "Czeka na:";
/// Prefix of a "related" line.
pub const RELATED_PREFIX: &str = "Powiązane:";

/// What a reference points at.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RefTarget {
    Topic { project: String, slug: String },
    Task { id: String },
}

impl RefTarget {
    /// As written in a connection line: always project-qualified for topics.
    pub fn text(&self) -> String {
        match self {
            RefTarget::Topic { project, slug } => format!("[[{project}/{slug}]]"),
            RefTarget::Task { id } => format!("^{id}"),
        }
    }

    /// The graph key ([`NodeKey`]).
    pub fn key(&self) -> String {
        match self {
            RefTarget::Topic { project, slug } => topic_key(project, slug),
            RefTarget::Task { id } => format!("k:{id}"),
        }
    }
}

/// One reference found in a text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ref {
    pub target: RefTarget,
    /// On a `Czeka na:` line.
    pub waits: bool,
    /// The reference is the whole line (`Powiązane: …` / `Czeka na: …`).
    pub standalone: bool,
    /// 0-based line in the text.
    pub line: usize,
}

fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--")
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The fence opened or closed by a trimmed line (`(char, run length)`).
fn fence_marker(trimmed: &str) -> Option<(char, usize)> {
    let c = trimmed.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    let n = trimmed.chars().take_while(|&x| x == c).count();
    (n >= 3).then_some((c, n))
}

/// `line` with inline code spans (`` `…` ``) blanked out (same length in chars).
fn blank_inline_code(line: &str) -> Vec<char> {
    let mut chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`'
            && let Some(len) = chars[i + 1..].iter().position(|&c| c == '`')
        {
            let end = i + 1 + len;
            for c in &mut chars[i..=end] {
                *c = ' ';
            }
            i = end + 1;
            continue;
        }
        i += 1;
    }
    chars
}

/// Every reference in `text` (fenced code skipped). `project` is the project the
/// text belongs to (for `[[slug]]`); `None` for the inbox.
pub fn parse_refs(text: &str, project: Option<&str>) -> Vec<Ref> {
    let mut out = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for (line_no, raw) in text.split('\n').enumerate() {
        let trimmed = raw.trim();
        let marker = fence_marker(trimmed);
        if let Some((c, n)) = fence {
            if marker.is_some_and(|(c2, n2)| c2 == c && n2 >= n) {
                fence = None;
            }
            continue;
        }
        if marker.is_some() {
            fence = marker;
            continue;
        }
        let plain = blank_inline_code(raw);
        let plain_str: String = plain.iter().collect();
        let plain_trim = plain_str.trim();
        let waits_line = plain_trim.starts_with(WAITS_PREFIX);
        // (target, start, end) in char positions.
        let mut found: Vec<(RefTarget, usize, usize)> = Vec::new();
        // Wikilinks.
        let mut i = 0;
        while i + 1 < plain.len() {
            if plain[i] == '[' && plain[i + 1] == '[' {
                let mut j = i + 2;
                let mut ok = false;
                while j < plain.len() {
                    if plain[j] == '\n' {
                        break;
                    }
                    if plain[j] == ']' {
                        ok = j + 1 < plain.len() && plain[j + 1] == ']';
                        break;
                    }
                    j += 1;
                }
                if ok {
                    let inner: String = plain[i + 2..j].iter().collect();
                    let body = inner.split('|').next().unwrap_or("").trim().to_string();
                    let has_body = !inner.starts_with('|') && !body.is_empty();
                    if has_body {
                        let (p, slug) = match body.split_once('/') {
                            Some((p, s)) => (Some(p.trim().to_string()), s.trim().to_string()),
                            None => (project.map(str::to_string), body.clone()),
                        };
                        if let Some(p) = p
                            && is_slug(&slug)
                            && is_slug(&p)
                        {
                            found.push((RefTarget::Topic { project: p, slug }, i, j + 2));
                        }
                    }
                    i = j + 2;
                    continue;
                }
            }
            i += 1;
        }
        // Task ids: `^xxxxxx` at the start or after whitespace / ( , ; :
        for k in 0..plain.len() {
            if plain[k] != '^' {
                continue;
            }
            let before_ok = k == 0 || {
                let b = plain[k - 1];
                b.is_whitespace() || matches!(b, '(' | ',' | ';' | ':')
            };
            if !before_ok || k + 7 > plain.len() {
                continue;
            }
            let id: String = plain[k + 1..k + 7].iter().collect();
            let after_ok = plain
                .get(k + 7)
                .is_none_or(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit()));
            if after_ok && is_valid_id(&id) {
                found.push((RefTarget::Task { id }, k, k + 7));
            }
        }
        found.sort_by_key(|f| f.1);
        let standalone = found.len() == 1 && {
            let (_, s, e) = &found[0];
            let r: String = plain[*s..*e].iter().collect();
            let r = r.trim().to_string();
            [WAITS_PREFIX, RELATED_PREFIX].iter().any(|prefix| {
                plain_trim
                    .strip_prefix(prefix)
                    .is_some_and(|body| body.trim() == r)
            })
        };
        for (target, _, _) in found {
            out.push(Ref {
                target,
                waits: waits_line,
                standalone,
                line: line_no,
            });
        }
    }
    out
}

/// The standalone connection line for `target` (`Czeka na: [[p/s]]` /
/// `Powiązane: ^id`).
pub fn connection_line(waits: bool, target: &RefTarget) -> String {
    format!(
        "{} {}",
        if waits { WAITS_PREFIX } else { RELATED_PREFIX },
        target.text()
    )
}

/// Whether `text` already has exactly this standalone connection line.
pub fn has_connection_line(
    text: &str,
    project: Option<&str>,
    waits: bool,
    target: &RefTarget,
) -> bool {
    parse_refs(text, project)
        .iter()
        .any(|r| r.standalone && r.waits == waits && r.target == *target)
}

// --- the graph ------------------------------------------------------------------

/// `t:<project>/<slug>` for topics, `k:<id>` for tasks, `k?:<where>#<line>` for
/// a task without an id (a source only).
pub type NodeKey = String;

pub fn topic_key(project: &str, slug: &str) -> NodeKey {
    format!("t:{project}/{slug}")
}

pub fn task_key(t: &Task) -> NodeKey {
    match &t.id {
        Some(id) => format!("k:{id}"),
        None => {
            let place = match &t.location {
                TaskLocation::Inbox => "inbox".to_string(),
                TaskLocation::Project { project } => project.clone(),
                TaskLocation::Topic { project, topic } => format!("{project}/{topic}"),
            };
            format!("k?:{place}#{}", t.line)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Related,
    Waits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: NodeKey,
    pub to: NodeKey,
    pub kind: EdgeKind,
    /// Written as a whole line in the source.
    pub standalone: bool,
    /// `summary` or `notes`.
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dangling {
    pub from: NodeKey,
    /// As written, e.g. `[[whenubuy/nie-ma]]` or `^zz91kq`.
    pub text: String,
    pub waits: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Graph {
    pub edges: Vec<Edge>,
    pub dangling: Vec<Dangling>,
}

fn written(t: &RefTarget, from_project: Option<&str>) -> String {
    match t {
        RefTarget::Task { id } => format!("^{id}"),
        RefTarget::Topic { project, slug } if Some(project.as_str()) == from_project => {
            format!("[[{slug}]]")
        }
        RefTarget::Topic { project, slug } => format!("[[{project}/{slug}]]"),
    }
}

/// Edges from topic summaries and task notes. Self references are dropped;
/// duplicates once (a standalone duplicate makes the edge standalone).
pub fn build_graph(snap: &Snapshot) -> Graph {
    let topics: HashSet<NodeKey> = snap
        .topics
        .iter()
        .map(|t| topic_key(&t.project, &t.slug))
        .collect();
    let tasks: HashSet<NodeKey> = snap
        .tasks
        .iter()
        .filter_map(|t| t.id.as_ref().map(|id| format!("k:{id}")))
        .collect();
    let mut g = Graph::default();
    let mut seen: HashSet<(NodeKey, NodeKey, EdgeKind)> = HashSet::new();
    let mut add =
        |g: &mut Graph, from: NodeKey, project: Option<&str>, text: &str, source: &str| {
            for r in parse_refs(text, project) {
                let to = r.target.key();
                let exists = match r.target {
                    RefTarget::Topic { .. } => topics.contains(&to),
                    RefTarget::Task { .. } => tasks.contains(&to),
                };
                if !exists {
                    g.dangling.push(Dangling {
                        from: from.clone(),
                        text: written(&r.target, project),
                        waits: r.waits,
                    });
                    continue;
                }
                if to == from {
                    continue;
                }
                let kind = if r.waits {
                    EdgeKind::Waits
                } else {
                    EdgeKind::Related
                };
                let id = (from.clone(), to.clone(), kind);
                if seen.contains(&id) {
                    if r.standalone
                        && let Some(e) = g
                            .edges
                            .iter_mut()
                            .find(|e| e.from == from && e.to == to && e.kind == kind)
                    {
                        e.standalone = true;
                    }
                    continue;
                }
                seen.insert(id);
                g.edges.push(Edge {
                    from: from.clone(),
                    to,
                    kind,
                    standalone: r.standalone,
                    source: source.to_string(),
                });
            }
        };
    for t in &snap.topics {
        add(
            &mut g,
            topic_key(&t.project, &t.slug),
            Some(&t.project),
            &t.summary,
            "summary",
        );
    }
    for t in &snap.tasks {
        if !t.notes.is_empty() {
            add(&mut g, task_key(t), t.location.project(), &t.notes, "notes");
        }
    }
    g
}

/// One edge touching a node, seen from that node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Neighbour {
    pub edge: Edge,
    pub other: NodeKey,
    pub outgoing: bool,
}

/// Edges touching `key`: what it waits for, what waits for it, then related;
/// by the other end's key within each group.
pub fn neighbours(g: &Graph, key: &str) -> Vec<Neighbour> {
    let mut out: Vec<Neighbour> = Vec::new();
    for e in &g.edges {
        if e.from == key {
            out.push(Neighbour {
                edge: e.clone(),
                other: e.to.clone(),
                outgoing: true,
            });
        } else if e.to == key {
            out.push(Neighbour {
                edge: e.clone(),
                other: e.from.clone(),
                outgoing: false,
            });
        }
    }
    let rank = |n: &Neighbour| match (n.edge.kind, n.outgoing) {
        (EdgeKind::Waits, true) => 0,
        (EdgeKind::Waits, false) => 1,
        (EdgeKind::Related, _) => 2,
    };
    out.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.other.cmp(&b.other)));
    out
}

/// What `key` waits for, link by link, up to `depth` levels: at each step the
/// first "waits" target in key order; a cycle is cut where it closes.
pub fn wait_chain(g: &Graph, key: &str, depth: usize) -> (Vec<NodeKey>, bool) {
    let mut chain = Vec::new();
    let mut seen: BTreeSet<NodeKey> = BTreeSet::from([key.to_string()]);
    let mut at = key.to_string();
    for _ in 0..depth {
        let next = g
            .edges
            .iter()
            .filter(|e| e.from == at && e.kind == EdgeKind::Waits)
            .map(|e| e.to.clone())
            .min();
        let Some(next) = next else {
            return (chain, false);
        };
        if seen.contains(&next) {
            return (chain, true);
        }
        chain.push(next.clone());
        seen.insert(next.clone());
        at = next;
    }
    (chain, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Priority, TaskStatus, Topic, TopicStatus};

    fn topic(project: &str, slug: &str, summary: &str) -> Topic {
        Topic {
            project: project.into(),
            slug: slug.into(),
            title: slug.into(),
            status: TopicStatus::InProgress,
            repo: None,
            summary: summary.into(),
            created: None,
            updated: None,
            path: format!("{project}/tematy/{slug}.md").into(),
        }
    }

    fn task(id: Option<&str>, project: &str, t: &str, notes: &str) -> Task {
        Task {
            id: id.map(str::to_string),
            title: "x".into(),
            status: TaskStatus::Todo,
            priority: Priority::Normal,
            due: None,
            done: None,
            links: vec![],
            notes: notes.into(),
            location: TaskLocation::Topic {
                project: project.into(),
                topic: t.into(),
            },
            line: 1,
        }
    }

    fn tt(p: &str, s: &str) -> RefTarget {
        RefTarget::Topic {
            project: p.into(),
            slug: s.into(),
        }
    }

    #[test]
    fn reads_every_form_like_the_map() {
        let text = [
            "Zależy od [[checkout]] i od [[montybrain/mapa]], zadanie ^abc123.",
            "Czeka na: [[whenubuy/platnosci]]",
            "Powiązane: [[ab-testy]]",
            "```",
            "[[w-kodzie]] ^zzz999",
            "```",
            "Kod `[[w-linii]]` się nie liczy, ani e-mail a^bc1234 ani [[Zły Slug]].",
        ]
        .join("\n");
        let refs = parse_refs(&text, Some("whenubuy"));
        let got: Vec<_> = refs
            .iter()
            .map(|r| (r.target.clone(), r.waits, r.standalone, r.line))
            .collect();
        assert_eq!(
            got,
            vec![
                (tt("whenubuy", "checkout"), false, false, 0),
                (tt("montybrain", "mapa"), false, false, 0),
                (
                    RefTarget::Task {
                        id: "abc123".into()
                    },
                    false,
                    false,
                    0
                ),
                (tt("whenubuy", "platnosci"), true, true, 1),
                (tt("whenubuy", "ab-testy"), false, true, 2),
            ]
        );
        let r = &parse_refs("Czeka na: odpowiedź z [[a]] w sprawie KRS", Some("p"))[0];
        assert!(r.waits && !r.standalone);
        let r = &parse_refs("Czeka na: ^abc123", Some("p"))[0];
        assert!(r.waits && r.standalone);
        // Inbox text: `[[slug]]` without a project is not a reference.
        assert!(parse_refs("Patrz [[checkout]]", None).is_empty());
        assert_eq!(parse_refs("Patrz [[a/checkout]]", None).len(), 1);
    }

    #[test]
    fn graph_chain_and_neighbours_like_the_map() {
        let snap = Snapshot {
            topics: vec![
                topic("a", "platnosci", "Czeka na: [[b/platnosci]]"),
                topic(
                    "b",
                    "platnosci",
                    "Czeka na: [[c]]\nPowiązane: [[a/platnosci]]",
                ),
                topic("b", "c", "Czeka na: [[b/d]]"),
                topic("b", "d", "Czeka na: [[a/platnosci]]\n[[nie-ma]] i ^zz9999"),
            ],
            tasks: vec![
                task(None, "a", "platnosci", "Powiązane: [[b/c]]"),
                task(Some("aaaaa1"), "b", "c", "Czeka na: ^aaaaa1 (siebie)"),
            ],
            ..Default::default()
        };
        let g = build_graph(&snap);
        let a = topic_key("a", "platnosci");
        let b = topic_key("b", "platnosci");
        let n: Vec<_> = neighbours(&g, &a)
            .into_iter()
            .map(|n| (n.other, n.edge.kind, n.outgoing))
            .collect();
        assert_eq!(
            n,
            vec![
                (b.clone(), EdgeKind::Waits, true),
                (topic_key("b", "d"), EdgeKind::Waits, false),
                (b.clone(), EdgeKind::Related, false),
            ]
        );
        assert_eq!(
            task_key(&task(None, "a", "platnosci", "")),
            "k?:a/platnosci#1"
        );
        let dangling: Vec<_> = g.dangling.iter().map(|d| d.text.as_str()).collect();
        assert_eq!(dangling, ["[[nie-ma]]", "^zz9999"]);
        assert!(
            !g.edges
                .iter()
                .any(|e| e.from == "k:aaaaa1" && e.to == "k:aaaaa1")
        );
        assert_eq!(
            wait_chain(&g, &a, 4),
            (vec![b, topic_key("b", "c"), topic_key("b", "d")], true)
        );
        assert_eq!(wait_chain(&g, &a, 2).0.len(), 2);
    }

    #[test]
    fn connection_lines() {
        let t = tt("firma", "krs");
        assert_eq!(connection_line(true, &t), "Czeka na: [[firma/krs]]");
        let task = RefTarget::Task {
            id: "c3d4e5".into(),
        };
        assert_eq!(connection_line(false, &task), "Powiązane: ^c3d4e5");
        let text = "Opis.\n```\nCzeka na: [[firma/krs]]\n```";
        assert!(!has_connection_line(text, Some("x"), true, &t));
        assert!(has_connection_line(
            &format!("{text}\nCzeka na: [[firma/krs]]"),
            Some("x"),
            true,
            &t
        ));
    }
}
