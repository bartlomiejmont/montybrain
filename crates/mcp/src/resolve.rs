//! Argument resolution and parsing: names to slugs, English/Polish words to enums.

use morwisko_core::chrono::NaiveDate;
use morwisko_core::{
    Due, Priority, Project, Snapshot, TaskStatus, Topic, TopicStatus, fold, slugify,
};

/// `Some("")` and `Some("  ")` count as absent: LLM callers often send empty strings.
pub fn nonempty(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn key(s: &str) -> String {
    fold(s.trim())
}

/// Two-tier match: an exact slug wins outright; otherwise slugs, display names and
/// the slugified input are compared case- and diacritics-insensitively.
fn pick<'a, T>(
    items: impl Iterator<Item = &'a T> + Clone,
    input: &str,
    slug: impl Fn(&T) -> &str,
    name: impl Fn(&T) -> &str,
) -> Vec<&'a T> {
    let exact = input.trim();
    let by_slug: Vec<&T> = items.clone().filter(|x| slug(x) == exact).collect();
    if !by_slug.is_empty() {
        return by_slug;
    }
    let k = key(input);
    let slug_k = slugify(input);
    items
        .filter(|x| key(slug(x)) == k || key(name(x)) == k || slug(x) == slug_k)
        .collect()
}

pub fn resolve_project<'a>(snap: &'a Snapshot, input: &str) -> Result<&'a Project, String> {
    let hits = pick(snap.projects.iter(), input, |p| &p.slug, |p| &p.name);
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(format!(
            "Unknown project '{input}'. Valid projects: {}.",
            list_or_none(
                snap.projects
                    .iter()
                    .map(|p| format!("{} ({})", p.slug, p.name))
            )
        )),
        many => Err(format!(
            "Ambiguous project '{input}'. Matches: {}. Pass the exact slug.",
            many.iter()
                .map(|p| format!("{} ({})", p.slug, p.name))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

pub fn resolve_topic<'a>(
    snap: &'a Snapshot,
    project: &Project,
    input: &str,
) -> Result<&'a Topic, String> {
    let in_project = snap.topics.iter().filter(|t| t.project == project.slug);
    let hits = pick(in_project, input, |t| &t.slug, |t| &t.title);
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(format!(
            "Unknown topic '{input}' in project '{}'. Valid topics: {}.",
            project.slug,
            list_or_none(
                snap.topics
                    .iter()
                    .filter(|t| t.project == project.slug)
                    .map(|t| format!("{} ({})", t.slug, t.title))
            )
        )),
        many => Err(format!(
            "Ambiguous topic '{input}' in project '{}'. Matches: {}. Pass the exact slug.",
            project.slug,
            many.iter()
                .map(|t| format!("{} ({})", t.slug, t.title))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Resolve a topic given without a project, searching every project.
pub fn resolve_topic_anywhere<'a>(snap: &'a Snapshot, input: &str) -> Result<&'a Topic, String> {
    let hits = pick(snap.topics.iter(), input, |t| &t.slug, |t| &t.title);
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(format!(
            "Unknown topic '{input}'. Pass `project` too. Valid topics: {}.",
            list_or_none(
                snap.topics
                    .iter()
                    .map(|t| format!("{}/{} ({})", t.project, t.slug, t.title))
            )
        )),
        many => Err(format!(
            "Topic '{input}' matches several topics: {}. Pass `project` as well.",
            many.iter()
                .map(|t| format!("{}/{}", t.project, t.slug))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn list_or_none(items: impl Iterator<Item = String>) -> String {
    let v: Vec<String> = items.collect();
    if v.is_empty() {
        "(none)".to_string()
    } else {
        v.join(", ")
    }
}

fn word(s: &str) -> String {
    key(s).replace([' ', '-'], "_")
}

pub fn parse_task_status(s: &str) -> Result<TaskStatus, String> {
    match word(s).as_str() {
        "todo" | "to_do" | "open" | "do_zrobienia" | "planned" => Ok(TaskStatus::Todo),
        "doing" | "in_progress" | "inprogress" | "w_toku" | "started" => Ok(TaskStatus::Doing),
        "done" | "x" | "zrobione" | "zrobiony" | "zrobione." | "complete" | "completed" => {
            Ok(TaskStatus::Done)
        }
        "blocked" | "zablokowane" | "zablokowany" => Ok(TaskStatus::Blocked),
        _ => Err(format!(
            "Unknown task status '{s}'. Use one of: todo, doing, done, blocked (Polish: w-toku, zrobione, zablokowane)."
        )),
    }
}

pub fn parse_priority(s: &str) -> Result<Priority, String> {
    match word(s).as_str() {
        "high" | "wysoki" | "wysoka" => Ok(Priority::High),
        "normal" | "normalny" | "normalna" | "medium" => Ok(Priority::Normal),
        "low" | "niski" | "niska" => Ok(Priority::Low),
        _ => Err(format!(
            "Unknown priority '{s}'. Use one of: high, normal, low (Polish: wysoki, normalny, niski)."
        )),
    }
}

pub fn parse_topic_status(s: &str) -> Result<TopicStatus, String> {
    match word(s).as_str() {
        "planned" | "planowany" => Ok(TopicStatus::Planned),
        "in_progress" | "inprogress" | "doing" | "w_toku" => Ok(TopicStatus::InProgress),
        "blocked" | "zablokowany" => Ok(TopicStatus::Blocked),
        "waiting" | "czeka" => Ok(TopicStatus::Waiting),
        "done" | "zrobiony" | "zrobione" => Ok(TopicStatus::Done),
        _ => Err(format!(
            "Unknown topic status '{s}'. Use one of: planned, in_progress, blocked, waiting, done (Polish: planowany, w-toku, zablokowany, czeka, zrobiony)."
        )),
    }
}

pub fn parse_due(s: &str) -> Result<Due, String> {
    s.trim()
        .parse::<Due>()
        .map_err(|_| format!("Invalid date '{s}'. Use YYYY-MM-DD or YYYY-MM-DDTHH:MM."))
}

/// A date bound; a full due value is accepted and reduced to its date.
pub fn parse_date(s: &str) -> Result<NaiveDate, String> {
    parse_due(s).map(|d| d.date)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use morwisko_core::{ProjectState, Topic, TopicStatus};

    use super::*;

    fn project(slug: &str, name: &str) -> Project {
        Project {
            slug: slug.into(),
            name: name.into(),
            color: None,
            state: ProjectState::Active,
            repos: Vec::new(),
            description: String::new(),
            path: PathBuf::from(format!("{slug}/projekt.md")),
        }
    }

    fn topic(project: &str, slug: &str, title: &str) -> Topic {
        Topic {
            project: project.into(),
            slug: slug.into(),
            title: title.into(),
            status: TopicStatus::Planned,
            repo: None,
            summary: String::new(),
            created: None,
            updated: None,
            path: PathBuf::from(format!("{project}/tematy/{slug}.md")),
        }
    }

    fn snap() -> Snapshot {
        Snapshot {
            projects: vec![project("a", "A"), project("b", "a"), project("dom", "Dom")],
            topics: vec![
                topic("a", "x", "X"),
                topic("a", "y", "x"),
                topic("b", "x", "Other"),
                topic("dom", "remont", "Remont łazienki"),
            ],
            ..Snapshot::default()
        }
    }

    #[test]
    fn exact_slug_beats_display_name() {
        let s = snap();
        assert_eq!(resolve_project(&s, "a").unwrap().slug, "a");
        assert_eq!(resolve_project(&s, " b ").unwrap().slug, "b");
        // Folded input matches the name "a" and the slug/name "A": ambiguous.
        let e = resolve_project(&s, "A").unwrap_err();
        assert!(e.contains("Ambiguous") && e.contains("exact slug"), "{e}");
        assert_eq!(resolve_project(&s, "DOM").unwrap().slug, "dom");

        let a = resolve_project(&s, "a").unwrap();
        assert_eq!(resolve_topic(&s, a, "x").unwrap().slug, "x");
        assert_eq!(resolve_topic(&s, a, "y").unwrap().slug, "y");
        assert!(resolve_topic(&s, a, "X").unwrap_err().contains("Ambiguous"));
        let dom = resolve_project(&s, "dom").unwrap();
        assert_eq!(
            resolve_topic(&s, dom, "remont lazienki").unwrap().slug,
            "remont"
        );
        // Anywhere: an exact slug in two projects is still ambiguous.
        assert!(
            resolve_topic_anywhere(&s, "x")
                .unwrap_err()
                .contains("several")
        );
        assert_eq!(resolve_topic_anywhere(&s, "y").unwrap().project, "a");
    }
}
