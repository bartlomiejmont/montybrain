//! Markdown rendering of vault entities, aimed at an LLM reader.

use std::fmt::Write as _;
use std::path::Path;

use morwisko_core::chrono::NaiveDate;
use morwisko_core::{
    JournalEntry, Priority, Project, SearchHit, Snapshot, Task, TaskLocation, TodayReason, Topic,
    TopicStatus, minute,
};

pub fn topic_status(s: TopicStatus) -> &'static str {
    match s {
        TopicStatus::Planned => "planned",
        TopicStatus::InProgress => "in_progress",
        TopicStatus::Blocked => "blocked",
        TopicStatus::Waiting => "waiting",
        TopicStatus::Done => "done",
    }
}

pub fn location(l: &TaskLocation) -> String {
    match l {
        TaskLocation::Inbox => "inbox".to_string(),
        TaskLocation::Project { project } => project.clone(),
        TaskLocation::Topic { project, topic } => format!("{project} / {topic}"),
    }
}

pub fn first_line(s: &str, max: usize) -> String {
    let line = s
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    truncate(line, max)
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", cut.trim_end())
    }
}

// ---------------------------------------------------------------------------
// Output budgets
// ---------------------------------------------------------------------------

/// Longest task title shown.
const TITLE_MAX: usize = 200;
/// Longest journal author shown.
const AUTHOR_MAX: usize = 60;

/// Item and character budget for one output section. Items are taken in order
/// until either budget is exhausted; everything after that is only counted, and
/// [`Bounded::finish`] writes a "… and N more (hint)" line.
pub struct Bounded {
    lines: Vec<String>,
    chars: usize,
    max_items: usize,
    max_chars: usize,
    skipped: usize,
}

impl Bounded {
    pub fn new(max_items: usize, max_chars: usize) -> Self {
        Bounded {
            lines: Vec::new(),
            chars: 0,
            max_items,
            max_chars,
            skipped: 0,
        }
    }

    pub fn push(&mut self, item: String) {
        let len = item.chars().count() + 1;
        if self.skipped == 0
            && self.lines.len() < self.max_items
            && (self.lines.is_empty() || self.chars + len <= self.max_chars)
        {
            self.chars += len;
            self.lines.push(item);
        } else {
            self.skipped += 1;
        }
    }

    /// Write the kept items (one per line) and the "… and N more" line.
    pub fn finish(self, out: &mut String, hint: &str) {
        for l in self.lines {
            out.push_str(&l);
            out.push('\n');
        }
        if self.skipped > 0 {
            let _ = writeln!(out, "… and {} more ({hint})", self.skipped);
        }
    }
}

/// Notes and links shown per task.
#[derive(Clone, Copy)]
pub struct NoteBudget {
    pub lines: usize,
    pub chars: usize,
    pub links: usize,
}

/// For results of a single write (create/update): the task as written.
pub const NOTES_FULL: NoteBudget = NoteBudget {
    lines: 40,
    chars: 4000,
    links: 30,
};
/// For lists (get_topic, search, project overview).
pub const NOTES_LIST: NoteBudget = NoteBudget {
    lines: 12,
    chars: 1200,
    links: 10,
};
/// For the dense project overview.
pub const NOTES_BRIEF: NoteBudget = NoteBudget {
    lines: 3,
    chars: 300,
    links: 3,
};

/// `- [ ] Title !high @2026-11-20 ^id · project / topic`
pub fn task_line(t: &Task) -> String {
    let mut s = format!(
        "- [{}] {}",
        t.status.checkbox(),
        truncate(&t.title, TITLE_MAX)
    );
    match t.priority {
        Priority::High => s.push_str(" !high"),
        Priority::Low => s.push_str(" !low"),
        Priority::Normal => {}
    }
    if let Some(d) = t.due {
        let _ = write!(s, " @{d}");
    }
    if let Some(d) = t.done {
        let _ = write!(s, " ✓{d}");
    }
    if let Some(id) = &t.id {
        let _ = write!(s, " ^{id}");
    }
    let _ = write!(s, " · {}", location(&t.location));
    s
}

/// Task line plus its links and notes, indented, within `budget`.
pub fn task_detail(t: &Task, budget: NoteBudget) -> String {
    let mut s = task_line(t);
    for l in t.links.iter().take(budget.links) {
        let _ = write!(s, "\n  - link: {}", truncate(l, 300));
    }
    if t.links.len() > budget.links {
        let _ = write!(
            s,
            "\n  - … and {} more link(s)",
            t.links.len() - budget.links
        );
    }
    let notes: Vec<&str> = t.notes.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut chars = 0;
    let mut shown = 0;
    for n in &notes {
        let n = n.trim_end();
        let len = n.chars().count();
        if shown >= budget.lines || (shown > 0 && chars + len > budget.chars) {
            break;
        }
        chars += len;
        shown += 1;
        let _ = write!(s, "\n  > {}", truncate(n, budget.chars));
    }
    if shown < notes.len() {
        let _ = write!(
            s,
            "\n  > … {} more note line(s) (full notes in the file)",
            notes.len() - shown
        );
    }
    s
}

pub fn journal_line(e: &JournalEntry, with_location: bool) -> String {
    let loc = if with_location {
        format!(" · {} / {}", e.project, e.topic)
    } else {
        String::new()
    };
    format!(
        "- {} · {}{loc} · {}",
        minute::format(&e.at),
        truncate(&e.author, AUTHOR_MAX),
        e.text.replace('\n', " / ")
    )
}

/// [`journal_line`] with the entry text cut to `max` characters.
pub fn journal_line_cut(e: &JournalEntry, with_location: bool, max: usize) -> String {
    let mut e = e.clone();
    e.text = truncate(&e.text.replace('\n', " / "), max);
    journal_line(&e, with_location)
}

pub fn abs_path(root: &Path, rel: &Path) -> String {
    root.join(rel).display().to_string()
}

fn open_count(snap: &Snapshot, loc: &TaskLocation) -> usize {
    snap.tasks
        .iter()
        .filter(|t| &t.location == loc && !t.status.is_done())
        .count()
}

fn topic_loc(t: &Topic) -> TaskLocation {
    TaskLocation::Topic {
        project: t.project.clone(),
        topic: t.slug.clone(),
    }
}

fn warnings_block(snap: &Snapshot, out: &mut String) {
    if snap.warnings.is_empty() {
        return;
    }
    let _ = writeln!(
        out,
        "\n## Warnings ({} problems in vault files)",
        snap.warnings.len()
    );
    let mut b = Bounded::new(5, 1500);
    for w in &snap.warnings {
        let line = w.line.map(|l| format!(":{l}")).unwrap_or_default();
        b.push(format!(
            "- {}{line}: {}",
            w.path.display(),
            truncate(&w.message, 200)
        ));
    }
    b.finish(out, "see the files or the desktop app");
}

/// `key=value` arguments that narrow `list_tasks` to one project, if any.
fn project_arg(project: Option<&Project>) -> String {
    project
        .map(|p| format!("project=\"{}\", ", p.slug))
        .unwrap_or_default()
}

/// The session-start briefing. `project` limits it to one project, with more detail.
pub fn overview(snap: &Snapshot, today: NaiveDate, project: Option<&Project>) -> String {
    let mut out = format!("# Morwisko overview · {}\n", today.format("%Y-%m-%d, %A"));

    // Today block.
    let items: Vec<_> = snap
        .today(today)
        .into_iter()
        .filter(|i| project.is_none_or(|p| i.task.location.project() == Some(p.slug.as_str())))
        .collect();
    out.push_str("\n## Today\n");
    if items.is_empty() {
        out.push_str("Nothing overdue, due today, in progress or blocked.\n");
    }
    for (reason, label) in [
        (TodayReason::Overdue, "Overdue"),
        (TodayReason::DueToday, "Due today"),
        (TodayReason::Doing, "In progress"),
        (TodayReason::Blocked, "Blocked"),
    ] {
        let group: Vec<_> = items.iter().filter(|i| i.reason == reason).collect();
        if group.is_empty() {
            continue;
        }
        let _ = writeln!(out, "{label}:");
        let mut b = Bounded::new(10, 3000);
        for i in group {
            b.push(task_line(&i.task));
        }
        let pa = project_arg(project);
        let hint = match reason {
            TodayReason::Overdue => format!(
                "use list_tasks with {pa}due_to=\"{}\"",
                today.pred_opt().unwrap_or(today)
            ),
            TodayReason::DueToday => {
                format!("use list_tasks with {pa}due_from=\"{today}\", due_to=\"{today}\"")
            }
            TodayReason::Doing => format!("use list_tasks with {pa}status=[\"doing\"]"),
            TodayReason::Blocked => format!("use list_tasks with {pa}status=[\"blocked\"]"),
        };
        b.finish(&mut out, &hint);
    }

    match project {
        None => {
            let inbox = open_count(snap, &TaskLocation::Inbox);
            let _ = writeln!(
                out,
                "\n## Inbox\n{inbox} open task(s){}",
                if inbox > 0 {
                    " (list_tasks with inbox_only)."
                } else {
                    "."
                }
            );
            out.push_str("\n## Projects\n");
            let active: Vec<_> = snap
                .projects
                .iter()
                .filter(|p| p.state == morwisko_core::ProjectState::Active)
                .collect();
            if active.is_empty() {
                out.push_str("(no active projects)\n");
            }
            let mut projects = Bounded::new(20, 9000);
            for p in active {
                let mut out = String::new();
                let _ = write!(out, "### {} (`{}`)", truncate(&p.name, 100), p.slug);
                if !p.repos.is_empty() {
                    let names: Vec<_> = p.repos.iter().take(8).map(|r| r.name.as_str()).collect();
                    let more = if p.repos.len() > 8 { ", …" } else { "" };
                    let _ = write!(out, " · repos: {}{more}", names.join(", "));
                }
                out.push('\n');
                let own = open_count(
                    snap,
                    &TaskLocation::Project {
                        project: p.slug.clone(),
                    },
                );
                if own > 0 {
                    let _ = writeln!(out, "- project-level tasks: {own} open");
                }
                let mut hidden_done = 0;
                let mut topics = Bounded::new(12, 2500);
                for t in snap.topics_of(&p.slug) {
                    if t.status == TopicStatus::Done {
                        hidden_done += 1;
                        continue;
                    }
                    let summary = first_line(&t.summary, 110);
                    let sep = if summary.is_empty() {
                        String::new()
                    } else {
                        format!(" — {summary}")
                    };
                    topics.push(format!(
                        "- {} (`{}`) · {} · {} open{sep}",
                        truncate(&t.title, 100),
                        t.slug,
                        topic_status(t.status),
                        open_count(snap, &topic_loc(t)),
                    ));
                }
                topics.finish(
                    &mut out,
                    &format!("use overview with project=\"{}\"", p.slug),
                );
                if hidden_done > 0 {
                    let _ = writeln!(out, "- ({hidden_done} done topic(s) hidden)");
                }
                projects.push(out.trim_end().to_string());
            }
            projects.finish(&mut out, "use overview with project=\"<slug>\"");
            let others: Vec<_> = snap
                .projects
                .iter()
                .filter(|p| p.state == morwisko_core::ProjectState::Paused)
                .map(|p| p.slug.as_str())
                .collect();
            if !others.is_empty() {
                let more = if others.len() > 30 {
                    format!(", … and {} more", others.len() - 30)
                } else {
                    String::new()
                };
                let _ = writeln!(
                    out,
                    "\nPaused projects: {}{more}",
                    others
                        .iter()
                        .take(30)
                        .copied()
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            out.push_str("\n## Recent journal\n");
            let recent = snap.recent_journal(10);
            if recent.is_empty() {
                out.push_str("(empty)\n");
            }
            for e in recent {
                let _ = writeln!(out, "{}", journal_line_cut(&e, true, 140));
            }
        }
        Some(p) => {
            let _ = write!(
                out,
                "\n## {} (`{}`) · {:?}",
                truncate(&p.name, 100),
                p.slug,
                p.state
            );
            out.push('\n');
            let mut repos = Bounded::new(20, 3000);
            for r in &p.repos {
                repos.push(format!(
                    "- repo `{}`: {}",
                    truncate(&r.name, 100),
                    truncate(&r.path, 300)
                ));
            }
            repos.finish(&mut out, "see projekt.md");
            if !p.description.trim().is_empty() {
                let _ = writeln!(out, "\n{}", truncate(p.description.trim(), 600));
            }
            let own: Vec<_> = snap
                .tasks
                .iter()
                .filter(|t| {
                    t.location
                        == TaskLocation::Project {
                            project: p.slug.clone(),
                        }
                })
                .filter(|t| !t.status.is_done())
                .collect();
            if !own.is_empty() {
                out.push_str("\n### Project-level tasks\n");
                let mut b = Bounded::new(15, 5000);
                for t in own {
                    b.push(task_detail(t, NOTES_BRIEF));
                }
                b.finish(
                    &mut out,
                    &format!("use list_tasks with project=\"{}\"", p.slug),
                );
            }
            out.push_str("\n### Topics\n");
            let mut topics = Bounded::new(25, 10_000);
            for t in snap.topics_of(&p.slug) {
                let mut out = String::new();
                let repo = t
                    .repo
                    .as_deref()
                    .map(|r| format!(" · repo {}", truncate(r, 100)))
                    .unwrap_or_default();
                let _ = writeln!(
                    out,
                    "\n#### {} (`{}`) · {}{repo} · updated {}",
                    truncate(&t.title, 100),
                    t.slug,
                    topic_status(t.status),
                    t.updated
                        .map(|u| minute::format(&u))
                        .unwrap_or_else(|| "?".into()),
                );
                if !t.summary.trim().is_empty() {
                    let _ = writeln!(out, "{}", truncate(t.summary.trim(), 500));
                }
                let mut tasks = Bounded::new(8, 2000);
                for task in snap
                    .tasks
                    .iter()
                    .filter(|x| x.location == topic_loc(t) && !x.status.is_done())
                {
                    tasks.push(task_detail(task, NOTES_BRIEF));
                }
                tasks.finish(
                    &mut out,
                    &format!(
                        "use get_topic with project=\"{}\", topic=\"{}\"",
                        p.slug, t.slug
                    ),
                );
                topics.push(out.trim_end().to_string());
            }
            topics.finish(&mut out, "use get_topic for the others");
            out.push_str("\n### Recent journal\n");
            let entries: Vec<_> = snap
                .recent_journal(usize::MAX)
                .into_iter()
                .filter(|e| e.project == p.slug)
                .take(10)
                .collect();
            if entries.is_empty() {
                out.push_str("(empty)\n");
            }
            for e in entries {
                let _ = writeln!(out, "{}", journal_line_cut(&e, true, 300));
            }
        }
    }
    warnings_block(snap, &mut out);
    out
}

/// Paging of the two lists in [`topic_full`].
#[derive(Clone, Copy)]
pub struct TopicPaging {
    pub tasks_offset: usize,
    pub tasks_limit: usize,
    pub journal_offset: usize,
    pub journal_limit: usize,
}

/// "showing a–b of n" for a page.
fn showing(offset: usize, shown: usize, total: usize) -> String {
    if shown == 0 {
        format!("none shown of {total}")
    } else {
        format!("showing {}–{} of {total}", offset + 1, offset + shown)
    }
}

/// The topic summary is shown in full up to this many characters.
const SUMMARY_MAX: usize = 4000;
/// Each journal entry in `get_topic` is cut to this many characters.
const JOURNAL_ENTRY_MAX: usize = 1500;

pub fn topic_full(
    snap: &Snapshot,
    t: &Topic,
    include_done: bool,
    root: &Path,
    paging: TopicPaging,
) -> String {
    let mut out = format!(
        "# {} / {} (`{}/{}`)\n",
        t.project,
        truncate(&t.title, 200),
        t.project,
        t.slug
    );
    let _ = writeln!(
        out,
        "Status: {} · Repo: {} · Created: {} · Updated: {}\nFile: {}",
        topic_status(t.status),
        t.repo.as_deref().unwrap_or("-"),
        t.created
            .map(|d| d.to_string())
            .unwrap_or_else(|| "?".into()),
        t.updated
            .map(|u| minute::format(&u))
            .unwrap_or_else(|| "?".into()),
        abs_path(root, &t.path),
    );
    if let Some(repo) = t.repo.as_deref()
        && let Some(r) = snap
            .project(&t.project)
            .and_then(|p| p.repos.iter().find(|r| r.name == repo))
    {
        let _ = writeln!(out, "Repo path: {}", truncate(&r.path, 300));
    }
    out.push_str("\n## Summary\n");
    let summary = t.summary.trim();
    if summary.is_empty() {
        out.push_str("(empty)");
    } else if summary.chars().count() > SUMMARY_MAX {
        out.push_str(&truncate(summary, SUMMARY_MAX));
        out.push_str("\n(summary truncated; the full text is in the file)");
    } else {
        out.push_str(summary);
    }
    let loc = topic_loc(t);
    let all: Vec<_> = snap.tasks.iter().filter(|x| x.location == loc).collect();
    let shown: Vec<_> = all
        .iter()
        .filter(|x| include_done || !x.status.is_done())
        .collect();
    let hidden = all.len() - shown.len();
    let page: Vec<_> = shown
        .iter()
        .skip(paging.tasks_offset)
        .take(paging.tasks_limit)
        .collect();
    let _ = write!(
        out,
        "\n\n## Tasks ({}",
        showing(paging.tasks_offset, page.len(), shown.len())
    );
    if hidden > 0 {
        let _ = write!(out, "; {hidden} done hidden, pass include_done");
    }
    out.push_str(")\n");
    for x in &page {
        let _ = writeln!(out, "{}", task_detail(x, NOTES_LIST));
    }
    let next = paging.tasks_offset + page.len();
    if next < shown.len() {
        let _ = writeln!(
            out,
            "… and {} more task(s) (pass tasks_offset={next} to see the next page)",
            shown.len() - next
        );
    }
    let j = snap.journal_of(&t.project, &t.slug);
    let jpage: Vec<_> = j
        .iter()
        .skip(paging.journal_offset)
        .take(paging.journal_limit)
        .collect();
    let _ = writeln!(
        out,
        "\n## Journal (newest first; {})",
        showing(paging.journal_offset, jpage.len(), j.len())
    );
    if j.is_empty() {
        out.push_str("(empty)\n");
    }
    for e in &jpage {
        let _ = writeln!(out, "{}", journal_line_cut(e, false, JOURNAL_ENTRY_MAX));
    }
    let next = paging.journal_offset + jpage.len();
    if next < j.len() {
        let _ = writeln!(
            out,
            "… and {} more entr(ies) (pass journal_offset={next} to see older ones)",
            j.len() - next
        );
    }
    out
}

pub fn topic_brief(t: &Topic, root: &Path) -> String {
    format!(
        "- {} (`{}/{}`) · {}{}\n  summary: {}\n  file: {}",
        truncate(&t.title, 200),
        t.project,
        t.slug,
        topic_status(t.status),
        t.repo
            .as_deref()
            .map(|r| format!(" · repo {r}"))
            .unwrap_or_default(),
        if t.summary.trim().is_empty() {
            "(empty)".to_string()
        } else {
            first_line(&t.summary, 200)
        },
        abs_path(root, &t.path),
    )
}

/// Search results; `offset` and `limit` apply to each kind (tasks, topics,
/// journal entries) separately.
pub fn search_results(hits: &[SearchHit], root: &Path, offset: usize, limit: usize) -> String {
    let mut tasks = Vec::new();
    let mut topics = Vec::new();
    let mut journal = Vec::new();
    for h in hits {
        match h {
            SearchHit::Task { task } => tasks.push(task),
            SearchHit::Topic { topic } => topics.push(topic),
            SearchHit::Journal { entry } => journal.push(entry),
        }
    }
    let mut out = format!(
        "{} hit(s): {} task(s), {} topic(s), {} journal entr(ies)\n",
        hits.len(),
        tasks.len(),
        topics.len(),
        journal.len()
    );
    fn section<T>(
        out: &mut String,
        title: &str,
        items: &[T],
        offset: usize,
        limit: usize,
        render: impl Fn(&T) -> String,
    ) {
        if items.is_empty() {
            return;
        }
        let page: Vec<&T> = items.iter().skip(offset).take(limit).collect();
        let _ = writeln!(
            out,
            "\n## {title} ({})",
            showing(offset, page.len(), items.len())
        );
        for x in &page {
            let _ = writeln!(out, "{}", render(x));
        }
        let next = offset + page.len();
        if next < items.len() {
            let _ = writeln!(
                out,
                "… and {} more (pass offset={next} to see the next page)",
                items.len() - next
            );
        }
    }
    section(&mut out, "Tasks", &tasks, offset, limit, |t| {
        task_detail(t, NOTES_BRIEF)
    });
    section(&mut out, "Topics", &topics, offset, limit, |t| {
        topic_brief(t, root)
    });
    section(
        &mut out,
        "Journal (newest first)",
        &journal,
        offset,
        limit,
        |e| journal_line_cut(e, true, 300),
    );
    out
}
