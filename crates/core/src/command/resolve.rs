//! Names to places. After ` > `:
//!
//! 1. an exact slug wins (`projekt`, `projekt/temat`, or a topic slug);
//! 2. then a match ignoring case and Polish characters on slugs and names;
//! 3. a word that matches a project and a topic means the project;
//! 4. two hits, or none: nothing is written and the choices are listed;
//! 5. no hit, but exactly one name one edit away: offered as a typo
//!    ("literówka"), still to be confirmed.
//!
//! Archived projects and done topics are found only by their exact slug.

use serde::{Deserialize, Serialize};

use crate::model::{ProjectState, Task, TaskLocation, TaskStatus, TopicStatus};
use crate::query::Snapshot;
use crate::refs::RefTarget;
use crate::slug::{fold, slugify};

/// A place a task can go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    pub location: TaskLocation,
    /// "Skrzynka", "WhenUBuy", "WhenUBuy / Checkout".
    pub label: String,
    /// What to type after ` > ` to get exactly this place.
    pub text: String,
}

/// One option offered when a name is ambiguous, unknown or a typo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub label: String,
    /// The replacement for the span the choice belongs to.
    pub insert: String,
    /// `project`, `topic`, `inbox` or `task`.
    pub kind: String,
    /// One edit away from what was typed.
    pub typo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved<T> {
    One(T),
    /// Exactly one name one edit away.
    Typo(T, Choice),
    Ambiguous(Vec<Choice>),
    Unknown(Vec<Choice>),
}

pub fn inbox_place() -> Place {
    Place {
        location: TaskLocation::Inbox,
        label: "Skrzynka".into(),
        text: "skrzynka".into(),
    }
}

pub fn project_place(snap: &Snapshot, slug: &str) -> Place {
    let name = snap.project(slug).map_or(slug, |p| p.name.as_str());
    Place {
        location: TaskLocation::Project {
            project: slug.to_string(),
        },
        label: name.to_string(),
        text: slug.to_string(),
    }
}

pub fn topic_place(snap: &Snapshot, project: &str, topic: &str) -> Place {
    let pname = snap.project(project).map_or(project, |p| p.name.as_str());
    let tname = snap
        .topic(project, topic)
        .map_or(topic, |t| t.title.as_str());
    Place {
        location: TaskLocation::Topic {
            project: project.to_string(),
            topic: topic.to_string(),
        },
        label: format!("{pname} / {tname}"),
        text: format!("{project}/{topic}"),
    }
}

pub fn place_of(snap: &Snapshot, loc: &TaskLocation) -> Place {
    match loc {
        TaskLocation::Inbox => inbox_place(),
        TaskLocation::Project { project } => project_place(snap, project),
        TaskLocation::Topic { project, topic } => topic_place(snap, project, topic),
    }
}

/// Levenshtein distance is at most one (on chars).
pub fn within_one(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a == b {
        return true;
    }
    let (s, l) = if a.len() <= b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    if l.len() - s.len() > 1 {
        return false;
    }
    let mut i = 0;
    while i < s.len() && s[i] == l[i] {
        i += 1;
    }
    if s.len() == l.len() {
        s[i + 1..] == l[i + 1..]
    } else {
        s[i..] == l[i + 1..]
    }
}

/// Keys a thing can be typed as: its slug and its name, folded.
fn keys(slug: &str, name: &str) -> [String; 3] {
    [fold(slug), fold(name), slugify(name)]
}

fn folded_match(input: &str, slug: &str, name: &str) -> bool {
    let k = fold(input.trim());
    let ks = slugify(input);
    keys(slug, name).contains(&k) || slug == ks
}

fn typo_match(input: &str, slug: &str, name: &str) -> bool {
    let k = fold(input.trim());
    // Very short words are too easy to hit by accident.
    k.chars().count() >= 4 && keys(slug, name).iter().any(|x| within_one(&k, x))
}

#[derive(Clone)]
struct Cand {
    place: Place,
    kind: &'static str,
    slug: String,
    name: String,
    /// Active project / open topic.
    live: bool,
}

fn project_cands(snap: &Snapshot) -> Vec<Cand> {
    snap.projects
        .iter()
        .map(|p| Cand {
            place: project_place(snap, &p.slug),
            kind: "project",
            slug: p.slug.clone(),
            name: p.name.clone(),
            live: p.state != ProjectState::Archived,
        })
        .collect()
}

fn topic_cands(snap: &Snapshot, project: Option<&str>) -> Vec<Cand> {
    snap.topics
        .iter()
        .filter(|t| project.is_none_or(|p| t.project == p))
        .map(|t| Cand {
            place: topic_place(snap, &t.project, &t.slug),
            kind: "topic",
            slug: t.slug.clone(),
            name: t.title.clone(),
            live: t.status != TopicStatus::Done
                && snap
                    .project(&t.project)
                    .is_some_and(|p| p.state != ProjectState::Archived),
        })
        .collect()
}

fn choice(c: &Cand, typo: bool) -> Choice {
    Choice {
        label: c.place.label.clone(),
        insert: c.place.text.clone(),
        kind: c.kind.into(),
        typo,
    }
}

/// Projects win over topics; one hit is the answer, several are ambiguous.
fn decide(projects: Vec<&Cand>, topics: Vec<&Cand>) -> Option<Resolved<Place>> {
    match (projects.as_slice(), topics.as_slice()) {
        ([one], _) => Some(Resolved::One(one.place.clone())),
        ([_, _, ..], _) => Some(Resolved::Ambiguous(
            projects.iter().map(|c| choice(c, false)).collect(),
        )),
        ([], [one]) => Some(Resolved::One(one.place.clone())),
        ([], [_, _, ..]) => Some(Resolved::Ambiguous(
            topics.iter().map(|c| choice(c, false)).collect(),
        )),
        ([], []) => None,
    }
}

/// The closest few places for an unknown name (substring of slug or name first).
fn nearest(cands: &[Cand], input: &str) -> Vec<Choice> {
    let k = fold(input.trim());
    let mut scored: Vec<(usize, &Cand)> = cands
        .iter()
        .filter(|c| c.live)
        .filter_map(|c| {
            let hay = format!("{} {}", fold(&c.slug), fold(&c.name));
            let first = k.split(['/', ' ', '-']).find(|w| w.len() >= 3);
            if hay.contains(&k) {
                Some((0, c))
            } else if first.is_some_and(|w| hay.contains(w)) {
                Some((1, c))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.place.label.cmp(&b.1.place.label))
    });
    scored
        .into_iter()
        .take(8)
        .map(|(_, c)| choice(c, false))
        .collect()
}

fn tiers(projects: &[Cand], topics: &[Cand], input: &str) -> Resolved<Place> {
    let exact = input.trim();
    let p1: Vec<&Cand> = projects.iter().filter(|c| c.slug == exact).collect();
    let t1: Vec<&Cand> = topics.iter().filter(|c| c.slug == exact).collect();
    if let Some(r) = decide(p1, t1) {
        return r;
    }
    let p2: Vec<&Cand> = projects
        .iter()
        .filter(|c| c.live && folded_match(input, &c.slug, &c.name))
        .collect();
    let t2: Vec<&Cand> = topics
        .iter()
        .filter(|c| c.live && folded_match(input, &c.slug, &c.name))
        .collect();
    if let Some(r) = decide(p2, t2) {
        return r;
    }
    let typos: Vec<&Cand> = projects
        .iter()
        .chain(topics)
        .filter(|c| c.live && typo_match(input, &c.slug, &c.name))
        .collect();
    if let [one] = typos.as_slice() {
        return Resolved::Typo(one.place.clone(), choice(one, true));
    }
    let all: Vec<Cand> = projects.iter().chain(topics).cloned().collect();
    if typos.len() > 1 {
        return Resolved::Unknown(typos.iter().map(|c| choice(c, true)).collect());
    }
    Resolved::Unknown(nearest(&all, input))
}

/// The place after ` > `: `skrzynka`, `projekt`, `projekt/temat` or a topic.
pub fn resolve_place(snap: &Snapshot, input: &str) -> Resolved<Place> {
    let input = input.trim();
    if input.contains('/') {
        // A name with a slash in it ("Testy A/B"): an exact folded match on
        // the whole text comes before reading it as `projekt/temat`.
        let projects = project_cands(snap);
        let topics = topic_cands(snap, None);
        let p: Vec<&Cand> = projects
            .iter()
            .filter(|c| c.live && fold(&c.name) == fold(input))
            .collect();
        let t: Vec<&Cand> = topics
            .iter()
            .filter(|c| c.live && fold(&c.name) == fold(input))
            .collect();
        if let Some(r) = decide(p, t) {
            return r;
        }
    }
    if let Some((p, t)) = input.split_once('/') {
        let projects = project_cands(snap);
        let project = match tiers(&projects, &[], p) {
            Resolved::One(pl) => (pl, None),
            Resolved::Typo(pl, c) => (pl, Some(c)),
            Resolved::Ambiguous(c) => return Resolved::Ambiguous(c),
            Resolved::Unknown(c) => return Resolved::Unknown(c),
        };
        let slug = project.0.location.project().unwrap_or_default().to_string();
        let topics = topic_cands(snap, Some(&slug));
        if t.trim().is_empty() {
            return match project.1 {
                None => Resolved::One(project.0),
                Some(c) => Resolved::Typo(project.0, c),
            };
        }
        let fix = |mut c: Choice| {
            // A choice for the whole `projekt/temat` text.
            if c.kind == "topic" && !c.insert.contains('/') {
                c.insert = format!("{slug}/{}", c.insert);
            }
            c
        };
        return match (tiers(&[], &topics, t), project.1) {
            (Resolved::One(pl), None) => Resolved::One(pl),
            (Resolved::One(pl), Some(pc)) => {
                let c = Choice {
                    label: pl.label.clone(),
                    insert: pl.text.clone(),
                    kind: "topic".into(),
                    typo: pc.typo,
                };
                Resolved::Typo(pl, c)
            }
            (Resolved::Typo(pl, c), _) => Resolved::Typo(pl, fix(c)),
            (Resolved::Ambiguous(c), _) => Resolved::Ambiguous(c.into_iter().map(fix).collect()),
            (Resolved::Unknown(c), _) => Resolved::Unknown(c.into_iter().map(fix).collect()),
        };
    }
    let projects = project_cands(snap);
    let topics = topic_cands(snap, None);
    let exact_hit = projects.iter().chain(&topics).any(|c| c.slug == input);
    if !exact_hit && matches!(fold(input).as_str(), "skrzynka" | "inbox") {
        return Resolved::One(inbox_place());
    }
    tiers(&projects, &topics, input)
}

/// A topic reference `[[…]]`: `projekt/temat`, or a bare topic (first in
/// `context` project, then anywhere, by the same tiers).
pub fn resolve_topic_ref(snap: &Snapshot, body: &str, context: Option<&str>) -> Resolved<Place> {
    let body = body.trim();
    if body.contains('/') {
        // Only topics: a project (or a typo of one) is never offered here.
        let topics_only = |cs: Vec<Choice>| cs.into_iter().filter(|c| c.kind == "topic").collect();
        return match resolve_place(snap, body) {
            Resolved::One(p) | Resolved::Typo(p, _) if p.location.topic().is_none() => {
                Resolved::Unknown(Vec::new())
            }
            Resolved::Ambiguous(cs) => Resolved::Ambiguous(topics_only(cs)),
            Resolved::Unknown(cs) => Resolved::Unknown(topics_only(cs)),
            r => r,
        };
    }
    if let Some(ctx) = context {
        let local = topic_cands(snap, Some(ctx));
        if let r @ (Resolved::One(_) | Resolved::Ambiguous(_)) = tiers(&[], &local, body) {
            return r;
        }
    }
    let topics = topic_cands(snap, None);
    tiers(&[], &topics, body)
}

/// The task with this id.
pub fn find_task<'a>(snap: &'a Snapshot, id: &str) -> Option<&'a Task> {
    snap.task(id.trim().trim_start_matches('^'))
}

/// A task or a topic found by plain words (`/czeka umowa`, `Umowa /powiaz …`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub target: RefTarget,
    /// `task` or `topic`.
    pub kind: &'static str,
    pub title: String,
    /// Where it is: a task's place, a topic's project.
    pub place: String,
    /// 0: the whole name, 1: every word starts a word, 2: one typo away.
    pub tier: u8,
}

impl Hit {
    /// What to type for exactly this one: `^id` or `[[projekt/temat]]`.
    pub fn insert(&self) -> String {
        self.target.text()
    }

    pub fn choice(&self) -> Choice {
        Choice {
            label: format!("{} · {}", self.title, self.place),
            insert: self.insert(),
            kind: self.kind.into(),
            typo: self.tier == 2,
        }
    }
}

fn word_list(s: &str) -> Vec<String> {
    fold(s)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// `term` starts `word`, or (four letters or more) is one edit from it or
/// from its start.
fn term_hits(term: &str, word: &str, typo: bool) -> bool {
    if word.starts_with(term) {
        return true;
    }
    if !typo || term.chars().count() < 4 {
        return false;
    }
    let w: Vec<char> = word.chars().collect();
    let n = term.chars().count();
    within_one(term, word)
        || [n, n + 1]
            .iter()
            .filter(|&&k| k < w.len())
            .any(|&k| within_one(term, &w[..k].iter().collect::<String>()))
}

/// The tier of one candidate, or `None`. `own`: words of its name and slug;
/// `around`: words of where it is (project, topic).
fn tier_of(query: &str, exact: &[String], own: &[String], around: &[String]) -> Option<u8> {
    let q = fold(query.trim());
    if exact.contains(&q) {
        return Some(0);
    }
    let terms = word_list(query);
    if terms.is_empty() {
        return None;
    }
    for (tier, typo) in [(1u8, false), (2u8, true)] {
        let mut on_own = false;
        let all = terms.iter().all(|t| {
            if own.iter().any(|w| term_hits(t, w, typo)) {
                on_own = true;
                true
            } else {
                around.iter().any(|w| term_hits(t, w, typo))
            }
        });
        if all && on_own {
            return Some(tier);
        }
    }
    None
}

/// Open tasks and open topics (of active projects) matching `query`: the
/// best first, then topics before tasks, then by name. `exclude`: left out
/// (the subject of the command itself).
pub fn search(snap: &Snapshot, query: &str, exclude: &[RefTarget]) -> Vec<Hit> {
    let mut out = Vec::new();
    for t in snap.topics.iter().filter(|t| {
        t.status != TopicStatus::Done
            && snap
                .project(&t.project)
                .is_some_and(|p| p.state != ProjectState::Archived)
    }) {
        let target = RefTarget::Topic {
            project: t.project.clone(),
            slug: t.slug.clone(),
        };
        if exclude.contains(&target) {
            continue;
        }
        let pname = snap
            .project(&t.project)
            .map_or(t.project.clone(), |p| p.name.clone());
        let exact = [
            t.slug.clone(),
            format!("{}/{}", t.project, t.slug),
            fold(&t.title),
        ];
        let mut own = word_list(&t.title);
        own.extend(word_list(&t.slug));
        let mut around = word_list(&pname);
        around.extend(word_list(&t.project));
        if let Some(tier) = tier_of(query, &exact, &own, &around) {
            out.push(Hit {
                target,
                kind: "topic",
                title: t.title.clone(),
                place: pname,
                tier,
            });
        }
    }
    for t in snap.tasks.iter().filter(|t| t.status != TaskStatus::Done) {
        let Some(id) = &t.id else { continue };
        let target = RefTarget::Task { id: id.clone() };
        if exclude.contains(&target) {
            continue;
        }
        let place = place_of(snap, &t.location);
        let exact = [fold(&t.title)];
        let own = word_list(&t.title);
        let around = word_list(&format!("{} {}", place.label, place.text));
        if let Some(tier) = tier_of(query, &exact, &own, &around) {
            out.push(Hit {
                target,
                kind: "task",
                title: t.title.clone(),
                place: place.label,
                tier,
            });
        }
    }
    out.sort_by(|a, b| {
        a.tier
            .cmp(&b.tier)
            .then_with(|| b.kind.cmp(a.kind))
            .then_with(|| fold(&a.title).cmp(&fold(&b.title)))
            .then_with(|| a.place.cmp(&b.place))
    });
    out
}

/// Plain words naming one task or topic: one hit of the best tier is the
/// answer (a typo when only a typo matched); several are listed (at most 8).
pub fn resolve_search(snap: &Snapshot, query: &str, exclude: &[RefTarget]) -> Resolved<Hit> {
    let hits = search(snap, query, exclude);
    let Some(best) = hits.first().map(|h| h.tier) else {
        return Resolved::Unknown(Vec::new());
    };
    let top: Vec<&Hit> = hits.iter().filter(|h| h.tier == best).collect();
    match top.as_slice() {
        [one] if best < 2 => Resolved::One((*one).clone()),
        [one] => Resolved::Typo((*one).clone(), one.choice()),
        many => Resolved::Ambiguous(many.iter().take(8).map(|h| h.choice()).collect()),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{Project, Topic};

    pub(crate) fn snap() -> Snapshot {
        let p = |slug: &str, name: &str, state| Project {
            slug: slug.into(),
            name: name.into(),
            color: None,
            state,
            repos: vec![],
            description: String::new(),
            path: format!("{slug}/projekt.md").into(),
        };
        let t = |project: &str, slug: &str, title: &str, status| Topic {
            project: project.into(),
            slug: slug.into(),
            title: title.into(),
            status,
            repo: None,
            summary: String::new(),
            created: None,
            updated: None,
            path: format!("{project}/tematy/{slug}.md").into(),
        };
        Snapshot {
            projects: vec![
                p("firma", "Firma", ProjectState::Active),
                p("montybrain", "MontyBrain", ProjectState::Active),
                p("stary", "Stary", ProjectState::Archived),
                p("whenubuy", "WhenUBuy", ProjectState::Active),
            ],
            topics: vec![
                t(
                    "firma",
                    "checkout",
                    "Checkout firmowy",
                    TopicStatus::Planned,
                ),
                t("firma", "krs", "Odpis z KRS", TopicStatus::Waiting),
                t(
                    "montybrain",
                    "mapa-mysli",
                    "Mapa myśli",
                    TopicStatus::InProgress,
                ),
                t(
                    "montybrain",
                    "firma",
                    "Firma w aplikacji",
                    TopicStatus::Planned,
                ),
                t("whenubuy", "checkout", "Checkout", TopicStatus::InProgress),
                t(
                    "whenubuy",
                    "platnosci",
                    "Płatności",
                    TopicStatus::InProgress,
                ),
                t("whenubuy", "stare", "Stare", TopicStatus::Done),
            ],
            ..Default::default()
        }
    }

    fn one(r: Resolved<Place>) -> String {
        match r {
            Resolved::One(p) => p.text,
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn exact_slug_then_folded_names() {
        let s = snap();
        assert_eq!(one(resolve_place(&s, "whenubuy")), "whenubuy");
        assert_eq!(
            one(resolve_place(&s, "whenubuy/checkout")),
            "whenubuy/checkout"
        );
        assert_eq!(
            one(resolve_place(&s, "mapa-mysli")),
            "montybrain/mapa-mysli"
        );
        assert_eq!(one(resolve_place(&s, "WhenUBuy")), "whenubuy");
        assert_eq!(
            one(resolve_place(&s, "Mapa Myśli")),
            "montybrain/mapa-mysli"
        );
        assert_eq!(
            one(resolve_place(&s, "whenubuy/Płatności")),
            "whenubuy/platnosci"
        );
        assert_eq!(one(resolve_place(&s, "skrzynka")), "skrzynka");
        assert_eq!(one(resolve_place(&s, "platnosci")), "whenubuy/platnosci");
    }

    #[test]
    fn a_project_wins_over_a_topic_of_the_same_name() {
        // "firma" is a project and a topic of montybrain.
        assert_eq!(one(resolve_place(&snap(), "firma")), "firma");
    }

    #[test]
    fn collisions_list_choices() {
        match resolve_place(&snap(), "checkout") {
            Resolved::Ambiguous(c) => {
                let ins: Vec<_> = c.iter().map(|c| c.insert.as_str()).collect();
                assert_eq!(ins, ["firma/checkout", "whenubuy/checkout"]);
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn typos_are_offered_once() {
        match resolve_place(&snap(), "whenbuy") {
            Resolved::Typo(p, c) => {
                assert_eq!(p.text, "whenubuy");
                assert!(c.typo);
            }
            o => panic!("{o:?}"),
        }
        match resolve_place(&snap(), "whenubuy/platnosc") {
            Resolved::Typo(p, c) => {
                assert_eq!(p.text, "whenubuy/platnosci");
                assert_eq!(c.insert, "whenubuy/platnosci");
            }
            o => panic!("{o:?}"),
        }
        match resolve_place(&snap(), "whenbuy/checkout") {
            Resolved::Typo(p, _) => assert_eq!(p.text, "whenubuy/checkout"),
            o => panic!("{o:?}"),
        }
        // Short words do not get typo matches.
        assert!(matches!(
            resolve_place(&snap(), "krz"),
            Resolved::Unknown(_)
        ));
        // Unknown: nothing, with near names listed.
        match resolve_place(&snap(), "nieistnieje") {
            Resolved::Unknown(c) => assert!(c.is_empty()),
            o => panic!("{o:?}"),
        }
        match resolve_place(&snap(), "check") {
            Resolved::Unknown(c) => assert_eq!(c.len(), 2),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn archived_and_done_only_by_exact_slug() {
        let s = snap();
        assert_eq!(one(resolve_place(&s, "stary")), "stary");
        assert!(matches!(resolve_place(&s, " stary "), Resolved::One(_)));
        assert!(matches!(resolve_place(&s, "STARY"), Resolved::Unknown(_)));
        assert_eq!(one(resolve_place(&s, "stare")), "whenubuy/stare");
        assert!(matches!(resolve_place(&s, "Stare!"), Resolved::Unknown(_)));
    }

    #[test]
    fn a_title_with_a_slash_is_found_by_its_whole_name() {
        let mut s = snap();
        s.topics.push(Topic {
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
        assert_eq!(one(resolve_place(&s, "Testy A/B")), "whenubuy/testy-a-b");
        assert_eq!(one(resolve_place(&s, "testy a/b")), "whenubuy/testy-a-b");
        // `projekt/temat` still works.
        assert_eq!(
            one(resolve_place(&s, "whenubuy/testy-a-b")),
            "whenubuy/testy-a-b"
        );
    }

    fn with_tasks() -> Snapshot {
        let mut s = snap();
        let task = |id: &str, title: &str, loc: TaskLocation, status| Task {
            id: Some(id.into()),
            title: title.into(),
            status,
            priority: crate::model::Priority::Normal,
            due: None,
            done: None,
            links: vec![],
            notes: String::new(),
            location: loc,
            line: 1,
        };
        s.tasks = vec![
            task(
                "a1b2c3",
                "Umowa z dostawcą",
                TaskLocation::Inbox,
                TaskStatus::Todo,
            ),
            task(
                "c3d4e5",
                "Odpis z sądu",
                TaskLocation::Inbox,
                TaskStatus::Todo,
            ),
            task(
                "e5f6a7",
                "Umowa najmu",
                TaskLocation::Project {
                    project: "firma".into(),
                },
                TaskStatus::Todo,
            ),
            task(
                "f6a7b8",
                "Stara umowa",
                TaskLocation::Inbox,
                TaskStatus::Done,
            ),
        ];
        s
    }

    fn found(r: Resolved<Hit>) -> String {
        match r {
            Resolved::One(h) => h.insert(),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn search_by_plain_words() {
        let s = with_tasks();
        assert_eq!(found(resolve_search(&s, "dostawca", &[])), "^a1b2c3");
        assert_eq!(found(resolve_search(&s, "umowa dost", &[])), "^a1b2c3");
        // The project name narrows.
        assert_eq!(found(resolve_search(&s, "umowa firma", &[])), "^e5f6a7");
        assert_eq!(found(resolve_search(&s, "KRS", &[])), "[[firma/krs]]");
        assert_eq!(found(resolve_search(&s, "firma/krs", &[])), "[[firma/krs]]");
        // Two open ones: listed with kinds and places; done tasks are not offered.
        match resolve_search(&s, "umowa", &[]) {
            Resolved::Ambiguous(cs) => {
                let ins: Vec<_> = cs.iter().map(|c| c.insert.as_str()).collect();
                assert_eq!(ins, ["^e5f6a7", "^a1b2c3"]);
                assert!(cs.iter().all(|c| c.kind == "task"));
                assert!(cs[1].label.contains("Skrzynka"), "{cs:?}");
            }
            o => panic!("{o:?}"),
        }
        // A whole name wins over words that only start names.
        match resolve_search(&s, "checkout", &[]) {
            Resolved::Ambiguous(cs) => assert_eq!(cs.len(), 2),
            o => panic!("{o:?}"),
        }
        // A typo.
        match resolve_search(&s, "dostwca", &[]) {
            Resolved::Typo(h, c) => {
                assert_eq!(h.insert(), "^a1b2c3");
                assert!(c.typo);
            }
            o => panic!("{o:?}"),
        }
        // Excluded: the subject itself.
        let ex = [RefTarget::Task {
            id: "e5f6a7".into(),
        }];
        assert_eq!(found(resolve_search(&s, "umowa", &ex)), "^a1b2c3");
        assert!(matches!(
            resolve_search(&s, "nic takiego", &[]),
            Resolved::Unknown(_)
        ));
        // The location alone is not a match.
        assert!(matches!(
            resolve_search(&s, "skrzynka", &[]),
            Resolved::Unknown(_)
        ));
    }

    #[test]
    fn topic_references() {
        let s = snap();
        assert_eq!(one(resolve_topic_ref(&s, "firma/krs", None)), "firma/krs");
        assert_eq!(
            one(resolve_topic_ref(&s, "checkout", Some("whenubuy"))),
            "whenubuy/checkout"
        );
        assert!(matches!(
            resolve_topic_ref(&s, "checkout", None),
            Resolved::Ambiguous(_)
        ));
        // A project is not a topic.
        assert!(matches!(
            resolve_topic_ref(&s, "whenubuy/", None),
            Resolved::Unknown(_)
        ));
        assert!(within_one("whenbuy", "whenubuy"));
        assert!(within_one("abcd", "abce"));
        assert!(!within_one("abcd", "abef"));
    }
}
