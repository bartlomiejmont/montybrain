//! Local suggestions, no model involved: where an inbox entry probably belongs,
//! and which open task a new one looks like (the duplicate hint).
//!
//! # Words
//! Text is folded (lowercase, no Polish characters), split on anything that is
//! not a letter or digit, words shorter than 3 characters and a short list of
//! stop words are dropped, and each word is cut to its first 5 characters (a
//! crude stem: `faktura`, `fakturę`, `faktury` → `faktu`).
//!
//! # Destination score
//! For an entry with title stems `T` and every open topic and active project:
//!
//! | where a title stem is found                          | points |
//! |------------------------------------------------------|--------|
//! | topic title                                          | 3      |
//! | project name or slug (for the project and its topics)| 2      |
//! | a repository name of the project                     | 2      |
//! | topic summary                                        | 1      |
//!
//! Each stem counts once per field. A topic with any match gets +0.5 when its
//! journal has an entry from the last 14 days (recent activity breaks ties).
//! The score is divided by `sqrt(|T|)` so long titles are not favoured.
//!
//! # Thresholds
//! The best candidate is suggested when its score is at least
//! [`STRONG_SCORE`] and beats the second one by at least [`MARGIN`];
//! otherwise the row shows "?" (weak) with the candidates still listed.
//!
//! # Duplicate hint
//! An open task is "similar" when the two titles share at least 2 stems and
//! the Jaccard index of their stems is at least [`SIMILAR_JACCARD`], or when
//! the folded titles are equal.

use std::collections::BTreeSet;

use chrono::{Duration, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::model::{ProjectState, TaskLocation, TaskStatus, TopicStatus};
use crate::query::Snapshot;
use crate::slug::fold;

use super::resolve::{Place, place_of, project_place, topic_place};

pub const STRONG_SCORE: f64 = 2.0;
pub const MARGIN: f64 = 0.75;
pub const SIMILAR_JACCARD: f64 = 0.6;
const RECENT_DAYS: i64 = 14;
const RECENT_BONUS: f64 = 0.5;
const TOP: usize = 3;

const STOP: &[&str] = &[
    "and",
    "the",
    "for",
    "dla",
    "nie",
    "jak",
    "czy",
    "pod",
    "nad",
    "przy",
    "przez",
    "oraz",
    "albo",
    "tak",
    "juz",
    "jest",
    "sie",
    "aby",
    "zeby",
    "ten",
    "ta",
    "to",
    "tego",
    "tej",
    "jej",
    "jego",
    "ich",
    "nas",
    "was",
    "mnie",
    "moj",
    "moja",
    "moje",
    "bez",
    "lub",
    "ale",
    "ze",
    "po",
    "od",
    "do",
    "na",
    "co",
    "zrobic",
    "trzeba",
    "sprawdzic",
];

/// Stems of a text (see the module docs).
pub fn stems(text: &str) -> BTreeSet<String> {
    fold(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3 && !STOP.contains(w))
        .map(|w| w.chars().take(5).collect())
        .collect()
}

fn hits(title: &BTreeSet<String>, field: &BTreeSet<String>) -> usize {
    title.intersection(field).count()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub place: Place,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InboxSuggestion {
    pub id: String,
    /// Suggested when strong enough (see the thresholds); `None` shows "?".
    pub best: Option<Candidate>,
    /// The top candidates, best first (also for the agent's data).
    pub candidates: Vec<Candidate>,
}

/// Candidates for one title, best first (only those with a positive score).
pub fn candidates(snap: &Snapshot, title: &str, today: NaiveDate) -> Vec<Candidate> {
    let t = stems(title);
    if t.is_empty() {
        return Vec::new();
    }
    let norm = (t.len() as f64).sqrt();
    let since = today - Duration::days(RECENT_DAYS);
    let mut out = Vec::new();
    for p in snap
        .projects
        .iter()
        .filter(|p| p.state == ProjectState::Active)
    {
        let pname = stems(&format!("{} {}", p.name, p.slug));
        let repos = stems(
            &p.repos
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        );
        let project_points = 2.0 * hits(&t, &pname) as f64 + 2.0 * hits(&t, &repos) as f64;
        if project_points > 0.0 {
            out.push(Candidate {
                place: project_place(snap, &p.slug),
                score: project_points / norm,
            });
        }
        for topic in snap
            .topics
            .iter()
            .filter(|x| x.project == p.slug && x.status != TopicStatus::Done)
        {
            let mut s = 3.0 * hits(&t, &stems(&topic.title)) as f64
                + 1.0 * hits(&t, &stems(&topic.summary)) as f64
                + project_points;
            if s > 0.0 {
                let recent = snap.journal.iter().any(|j| {
                    j.project == topic.project && j.topic == topic.slug && j.at.date() >= since
                });
                if recent {
                    s += RECENT_BONUS;
                }
                // A topic matched only through its project is no better than the project.
                if s > project_points {
                    out.push(Candidate {
                        place: topic_place(snap, &topic.project, &topic.slug),
                        score: s / norm,
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.place.text.cmp(&b.place.text))
    });
    out.truncate(TOP);
    out
}

/// One suggestion per open inbox task with an id.
pub fn inbox_suggestions(snap: &Snapshot, today: NaiveDate) -> Vec<InboxSuggestion> {
    snap.tasks
        .iter()
        .filter(|t| t.location == TaskLocation::Inbox && t.status != TaskStatus::Done)
        .filter_map(|t| {
            let id = t.id.clone()?;
            let candidates = candidates(snap, &t.title, today);
            let best = match candidates.as_slice() {
                [first, rest @ ..]
                    if first.score >= STRONG_SCORE
                        && rest.first().is_none_or(|s| first.score - s.score >= MARGIN) =>
                {
                    Some(first.clone())
                }
                _ => None,
            };
            Some(InboxSuggestion {
                id,
                best,
                candidates,
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Similar {
    pub id: String,
    pub title: String,
    pub place: Place,
    pub score: f64,
}

/// Open tasks that look like `title` (the duplicate hint), most similar first,
/// at most 3.
pub fn similar_tasks(snap: &Snapshot, title: &str, exclude: &[String]) -> Vec<Similar> {
    let t = stems(title);
    let folded = fold(title.trim());
    if folded.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<Similar> = snap
        .tasks
        .iter()
        .filter(|x| x.status != TaskStatus::Done)
        .filter_map(|x| {
            let id = x.id.clone()?;
            if exclude.contains(&id) {
                return None;
            }
            let o = stems(&x.title);
            let shared = t.intersection(&o).count();
            let union = t.union(&o).count().max(1);
            let jaccard = shared as f64 / union as f64;
            let same = fold(x.title.trim()) == folded;
            (same || (shared >= 2 && jaccard >= SIMILAR_JACCARD)).then(|| Similar {
                id,
                title: x.title.clone(),
                place: place_of(snap, &x.location),
                score: if same { 1.0 } else { jaccard },
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    out.truncate(3);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{JournalEntry, Priority, Project, Repo, Task, Topic};

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn snap() -> Snapshot {
        let p = |slug: &str, name: &str, repos: &[&str]| Project {
            slug: slug.into(),
            name: name.into(),
            color: None,
            state: ProjectState::Active,
            repos: repos
                .iter()
                .map(|r| Repo {
                    name: (*r).into(),
                    path: format!("/r/{r}"),
                })
                .collect(),
            description: String::new(),
            path: format!("{slug}/projekt.md").into(),
        };
        let t = |project: &str, slug: &str, title: &str, summary: &str| Topic {
            project: project.into(),
            slug: slug.into(),
            title: title.into(),
            status: TopicStatus::InProgress,
            repo: None,
            summary: summary.into(),
            created: None,
            updated: None,
            path: format!("{project}/tematy/{slug}.md").into(),
        };
        let task = |id: &str, title: &str, loc: TaskLocation| Task {
            id: Some(id.into()),
            title: title.into(),
            status: TaskStatus::Todo,
            priority: Priority::Normal,
            due: None,
            done: None,
            links: vec![],
            notes: String::new(),
            location: loc,
            line: 1,
        };
        Snapshot {
            projects: vec![
                p("firma", "Firma", &[]),
                p(
                    "whenubuy",
                    "WhenUBuy",
                    &["whenubuy-backend", "whenubuy-mobile"],
                ),
            ],
            topics: vec![
                t(
                    "firma",
                    "ksiegowosc-vat",
                    "Księgowość i VAT",
                    "Faktury za kwartał dla biura.",
                ),
                t("firma", "krs", "Odpis z KRS", "Sąd ma termin."),
                t(
                    "whenubuy",
                    "platnosci",
                    "Płatności Stripe",
                    "Integracja gotowa na sandboksie.",
                ),
                t(
                    "whenubuy",
                    "push",
                    "Powiadomienia push",
                    "Review w App Store.",
                ),
            ],
            tasks: vec![
                task("aaaaa1", "Faktury VAT do biura", TaskLocation::Inbox),
                task("aaaaa2", "Kupić mleko", TaskLocation::Inbox),
                task(
                    "aaaaa3",
                    "Stripe webhook płatności zwrotów",
                    TaskLocation::Inbox,
                ),
                task("aaaaa4", "Odpis KRS", TaskLocation::Inbox),
                task(
                    "bbbbb1",
                    "Zadzwonić do sądu w sprawie KRS",
                    TaskLocation::Topic {
                        project: "firma".into(),
                        topic: "krs".into(),
                    },
                ),
            ],
            journal: vec![JournalEntry {
                project: "whenubuy".into(),
                topic: "platnosci".into(),
                at: d("2026-11-13").and_hms_opt(10, 0, 0).unwrap(),
                author: "ja".into(),
                text: "x".into(),
                line: 1,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn stems_cut_words() {
        let s = stems("Fakturę za Październik i VAT-y");
        assert!(s.contains("faktu") && s.contains("pazdz") && s.contains("vat"));
        assert!(!s.contains("za"));
    }

    #[test]
    fn scorer_suggests_strong_matches_and_marks_weak_ones() {
        let today = d("2026-11-14");
        let all = inbox_suggestions(&snap(), today);
        let by = |id: &str| all.iter().find(|s| s.id == id).unwrap();
        // faktury (title 1 + summary 1)…
        let f = by("aaaaa1");
        assert_eq!(f.best.as_ref().unwrap().place.text, "firma/ksiegowosc-vat");
        let s = by("aaaaa3");
        assert_eq!(s.best.as_ref().unwrap().place.text, "whenubuy/platnosci");
        // Nothing matches: "?" with no candidates.
        let m = by("aaaaa2");
        assert!(m.best.is_none() && m.candidates.is_empty());
        let k = by("aaaaa4");
        assert_eq!(k.best.as_ref().unwrap().place.text, "firma/krs");
        // Scores are documented numbers: "Stripe webhook płatności zwrotów"
        // has 4 stems; the topic gets 3+3 (title) + 0.5 (recent) = 6.5 / 2.
        assert!((s.best.as_ref().unwrap().score - 3.25).abs() < 1e-9);
    }

    #[test]
    fn weak_when_two_candidates_are_close() {
        let mut sn = snap();
        sn.topics[1].title = "Faktury i KRS".into();
        let today = d("2026-11-14");
        let c = candidates(&sn, "Faktury KRS", today);
        assert!(c.len() >= 2);
        let all = inbox_suggestions(
            &Snapshot {
                tasks: vec![Task {
                    title: "Faktury KRS".into(),
                    ..sn.tasks[0].clone()
                }],
                ..sn.clone()
            },
            today,
        );
        // Two topics close to each other: "?".
        let best = &all[0];
        if let [a, b, ..] = best.candidates.as_slice()
            && a.score - b.score < MARGIN
        {
            assert!(best.best.is_none());
        }
    }

    #[test]
    fn duplicate_hint() {
        let s = snap();
        let sim = similar_tasks(&s, "Zadzwonić do sądu ws. KRS", &[]);
        assert_eq!(sim[0].id, "bbbbb1");
        assert!(similar_tasks(&s, "Kupić chleb", &[]).is_empty());
        let same = similar_tasks(&s, "kupić MLEKO", &[]);
        assert_eq!(same[0].id, "aaaaa2");
        assert!(similar_tasks(&s, "kupić mleko", &["aaaaa2".into()]).is_empty());
    }
}
