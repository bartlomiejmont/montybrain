use super::links::{BranchKey, LinkStore};
use super::*;
use morwisko_core::{JournalEntry, Project, Repo, Topic, TopicStatus};

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn at(s: &str) -> NaiveDateTime {
    morwisko_core::minute::parse(s).unwrap()
}

const SNAPSHOT: &str = "=== beeking / app ===
Ścieżka: /r/app

$ git status --short --branch
## feature/pos-offline

$ git log --oneline -15
abc123 Kolejka

$ git log --all --since=14.days (hash, data, autor, opis)
abc123 2026-11-13 Ala Kolejka

$ git branch -a --sort=-committerdate (gałąź, data ostatniego commita)
feature/pos-offline 2026-11-13
main 2026-11-10
origin/feature/pos-offline 2026-11-12
origin/HEAD 2026-11-10
feat/JIRA-123-platnosci-blik 2026-11-12
fix/typo 2026-11-11
old/stara-galaz 2026-09-01

$ git worktree list --porcelain (drzewa robocze: ścieżka, HEAD, gałąź)
worktree /r/app
HEAD abc123
branch refs/heads/main

worktree /r/app-pos
HEAD def456
branch refs/heads/feature/pos-offline

=== beeking / api ===
Ścieżka: /r/api
(pominięto: ścieżka nie istnieje)
";

#[test]
fn snapshot_text_is_parsed_into_branches_and_worktrees() {
    let repos = parse_snapshot(SNAPSHOT);
    assert_eq!(repos.len(), 2);
    let app = &repos[0];
    assert_eq!(
        (app.repo.as_str(), app.path.as_str(), app.ok),
        ("app", "/r/app", true)
    );
    let names: Vec<&str> = app.branches.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "feature/pos-offline",
            "main",
            "HEAD",
            "feat/JIRA-123-platnosci-blik",
            "fix/typo",
            "old/stara-galaz"
        ]
    );
    // The remote copy is folded into the local branch, keeping the newer date.
    assert_eq!(app.branches[0].last_commit, d("2026-11-13"));
    assert_eq!(app.default_branch.as_deref(), Some("main"));
    // The main worktree is not listed; the linked one is, with its branch.
    assert_eq!(
        app.worktrees,
        vec![WorktreeObs {
            path: "/r/app-pos".into(),
            branch: Some("feature/pos-offline".into())
        }]
    );
    assert!(!repos[1].ok);
}

#[test]
fn branch_words_drop_kinds_and_tickets() {
    assert_eq!(
        branch_words("feat/JIRA-123-platnosci-blik"),
        ["jira", "platnosci", "blik"]
    );
    assert_eq!(branch_words("fix/typo"), ["typo"]);
    assert!(branch_words("feature/wip-123").is_empty());
    assert!(branch_words("codex/task-42").is_empty());
    assert_eq!(branch_words("Płatności_KARTĄ"), ["platnosci", "karta"]);
    assert!(branch_words("a1b2c3d").is_empty(), "a hash-like name");
}

fn topic(slug: &str, title: &str, text: &str, last: Option<&str>) -> TopicInfo {
    TopicInfo {
        slug: slug.into(),
        title: title.into(),
        text: fold(text),
        last_entry: last.map(d),
    }
}

fn topics() -> Vec<TopicInfo> {
    vec![
        topic(
            "platnosci-blik",
            "Płatności BLIK",
            "Testy na sandboksie.",
            Some("2026-11-10"),
        ),
        topic(
            "aplikacja-kasowa",
            "Aplikacja kasowa offline",
            "Gałąź feature/pos-offline w drzewie roboczym.",
            Some("2026-11-12"),
        ),
        topic("platnosci-karta", "Płatności kartą", "", None),
        topic(
            "migracja",
            "Migracja bazy na Postgres 17",
            "",
            Some("2026-11-01"),
        ),
    ]
}

#[test]
fn branches_are_matched_by_link_then_mention_then_words() {
    let t = topics();
    let mut links = LinkStore::default();
    let m = |b: &str, links: &LinkStore| match_branch("beeking", "app", b, &t, links);
    // The name written in the journal wins over words.
    assert_eq!(
        m("feature/pos-offline", &links),
        Some(("aplikacja-kasowa".into(), MatchKind::Mention))
    );
    // Two words in common beat one ("platnosci" alone fits two topics).
    assert_eq!(
        m("feat/JIRA-123-platnosci-blik", &links),
        Some(("platnosci-blik".into(), MatchKind::Words))
    );
    // Inflected and derived forms share a stem.
    assert_eq!(
        m("feature/migracje-postgresa", &links),
        Some(("migracja".into(), MatchKind::Words))
    );
    assert_eq!(
        m("karty-platnicze", &links),
        Some(("platnosci-karta".into(), MatchKind::Words))
    );
    // Traps: one shared word fitting two topics, noise only, short words, unrelated words.
    assert_eq!(m("feature/platnosci", &links), None, "ambiguous");
    assert_eq!(m("fix/wip-123", &links), None);
    assert_eq!(
        m("feature/pos", &links),
        None,
        "a short word is not a mention"
    );
    assert_eq!(m("feature/loyalty", &links), None);
    assert_eq!(
        m("feature/bazylia", &links),
        None,
        "a stem must be a real prefix"
    );
    // An explicit link wins over everything (and an unknown topic is ignored).
    links.link(
        &BranchKey::new("beeking", "app", "feature/platnosci"),
        "platnosci-karta",
    );
    assert_eq!(
        m("feature/platnosci", &links),
        Some(("platnosci-karta".into(), MatchKind::Link))
    );
    links.link(
        &BranchKey::new("beeking", "app", "feature/loyalty"),
        "nie-ma",
    );
    assert_eq!(m("feature/loyalty", &links), None);
    // Links are per repository.
    assert_eq!(
        match_branch("beeking", "api", "feature/platnosci", &t, &links),
        None
    );
}

fn snap() -> Snapshot {
    let topic = |slug: &str, title: &str, summary: &str| Topic {
        project: "beeking".into(),
        slug: slug.into(),
        title: title.into(),
        status: TopicStatus::InProgress,
        repo: None,
        summary: summary.into(),
        created: None,
        updated: None,
        path: format!("beeking/tematy/{slug}.md").into(),
    };
    Snapshot {
        projects: vec![Project {
            slug: "beeking".into(),
            name: "Beeking".into(),
            color: None,
            state: ProjectState::Active,
            repos: vec![
                Repo {
                    name: "app".into(),
                    path: "/r/app".into(),
                },
                Repo {
                    name: "api".into(),
                    path: "/r/api".into(),
                },
            ],
            description: String::new(),
            path: "beeking/projekt.md".into(),
        }],
        topics: vec![
            topic("platnosci-blik", "Płatności BLIK", ""),
            topic(
                "aplikacja-kasowa",
                "Aplikacja kasowa offline",
                "feature/pos-offline",
            ),
        ],
        journal: vec![JournalEntry {
            project: "beeking".into(),
            topic: "aplikacja-kasowa".into(),
            at: at("2026-11-12T10:00"),
            author: "ja".into(),
            text: "Kolejka transakcji.".into(),
            line: 1,
        }],
        ..Default::default()
    }
}

fn obs() -> ProjectObs {
    let b = |name: &str, date: &str| BranchObs {
        name: name.into(),
        last_commit: d(date),
    };
    ProjectObs {
        collected_at: at("2026-11-14T10:00"),
        since: Some("2026-11-12T09:00".into()),
        repos: vec![
            RepoObs {
                repo: "app".into(),
                path: "/r/app".into(),
                ok: true,
                branches: vec![
                    b("main", "2026-11-13"),
                    b("feature/pos-offline", "2026-11-13"),
                    b("feature/pos-sync", "2026-11-11"),
                    b("feature/loyalty", "2026-11-12"),
                    b("experiment/stare", "2026-09-01"),
                    b("platnosci-blik-e2e", "2026-11-09"),
                ],
                worktrees: vec![WorktreeObs {
                    path: "/r/app-old".into(),
                    branch: Some("experiment/stare".into()),
                }],
                default_branch: Some("main".into()),
                new_commits: Some(4),
            },
            RepoObs {
                repo: "api".into(),
                path: "/r/api".into(),
                ok: true,
                branches: vec![b("main", "2026-11-12"), b("blik-webhook", "2026-11-13")],
                worktrees: vec![],
                default_branch: Some("main".into()),
                new_commits: Some(8),
            },
        ],
    }
}

#[test]
fn signals_of_a_project() {
    let mut links = LinkStore::default();
    links.link(
        &BranchKey::new("beeking", "app", "feature/pos-sync"),
        "aplikacja-kasowa",
    );
    links.link(
        &BranchKey::new("beeking", "app", "feature/pos-offline"),
        "aplikacja-kasowa",
    );
    let o = obs();
    let r = analyse(
        "beeking",
        Some(&o),
        &BTreeMap::new(),
        &snap(),
        &links,
        d("2026-11-14"),
    );
    // Trunks are not listed; an old branch checked out in a worktree is active.
    assert_eq!(r.branches.len(), 6, "{:#?}", r.branches);
    let stare = r
        .branches
        .iter()
        .find(|b| b.branch == "experiment/stare")
        .unwrap();
    assert!(stare.active && stare.worktree.as_deref() == Some("/r/app-old"));
    // Unknown: active and unmatched.
    let unknown: Vec<&str> = r.unknown.iter().map(|b| b.branch.as_str()).collect();
    assert_eq!(unknown, ["feature/loyalty", "experiment/stare"]);
    // Spread: "Płatności BLIK" by branches in two repositories.
    assert_eq!(
        r.spread,
        vec![Spread {
            topic: "platnosci-blik".into(),
            branches: 2,
            repos: 2
        }]
    );
    // Moved: a matched branch newer than the topic's last entry (none for BLIK, 13.11 > 12.11 for POS).
    let moved: Vec<(&str, &str)> = r
        .moved
        .iter()
        .map(|m| (m.topic.as_str(), m.branch.as_str()))
        .collect();
    assert_eq!(
        moved,
        [
            ("aplikacja-kasowa", "feature/pos-offline"),
            ("platnosci-blik", "blik-webhook")
        ]
    );
    assert_eq!(
        (r.new_commits, r.since.as_deref()),
        (Some(12), Some("2026-11-12T09:00"))
    );

    // Ignoring hides a branch from "unknown"; three active branches of one topic spread.
    links.ignore(&BranchKey::new("beeking", "app", "feature/loyalty"));
    links.link(
        &BranchKey::new("beeking", "app", "experiment/stare"),
        "aplikacja-kasowa",
    );
    let r = analyse(
        "beeking",
        Some(&o),
        &BTreeMap::new(),
        &snap(),
        &links,
        d("2026-11-14"),
    );
    assert!(r.unknown.is_empty(), "{:?}", r.unknown);
    assert!(
        r.spread
            .iter()
            .any(|s| s.topic == "aplikacja-kasowa" && s.branches == 3 && s.repos == 1)
    );
    // Not collected yet: nothing to say.
    let none = analyse(
        "beeking",
        None,
        &BTreeMap::new(),
        &snap(),
        &links,
        d("2026-11-14"),
    );
    assert!(none.branches.is_empty() && none.collected_at.is_none());
}

#[test]
fn link_store_round_trip_and_rules() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".morwisko")).unwrap();
    let k = BranchKey::new("beeking", "app", "feature/x");
    assert!(links::update(dir.path(), |s| s.link(&k, "a")).unwrap());
    assert!(
        !links::update(dir.path(), |s| s.link(&k, "a")).unwrap(),
        "no change, no write"
    );
    let s = links::load(dir.path());
    assert_eq!(s.topic_of(&k), Some("a"));
    // Relinking replaces; ignoring drops the link; linking again un-ignores.
    links::update(dir.path(), |s| s.link(&k, "b")).unwrap();
    assert_eq!(links::load(dir.path()).links.len(), 1);
    links::update(dir.path(), |s| s.ignore(&k)).unwrap();
    let s = links::load(dir.path());
    assert!(s.is_ignored(&k) && s.topic_of(&k).is_none());
    links::update(dir.path(), |s| s.link(&k, "b")).unwrap();
    assert!(!links::load(dir.path()).is_ignored(&k));
    // An undo removes the link only while it points at its topic.
    links::update(dir.path(), |s| s.unlink(&k, "a")).unwrap();
    assert_eq!(links::load(dir.path()).topic_of(&k), Some("b"));
    links::update(dir.path(), |s| s.unlink(&k, "b")).unwrap();
    assert!(links::load(dir.path()).topic_of(&k).is_none());
    // The file lives in .morwisko/ (skipped by the loader and the watcher).
    assert!(dir.path().join(".morwisko/links.json").is_file());
    assert!(morwisko_core::is_ignored(std::path::Path::new(
        ".morwisko/links.json"
    )));
    // Unreadable → empty.
    std::fs::write(dir.path().join(".morwisko/links.json"), "nie json").unwrap();
    assert_eq!(links::load(dir.path()), LinkStore::default());
}

#[test]
fn drift_file_records_when_things_were_seen() {
    let mut f = DriftFile::default();
    f.record("beeking", obs());
    assert_eq!(
        f.seen
            .get("beeking|app|feature/loyalty")
            .map(String::as_str),
        Some("2026-11-14T10:00")
    );
    assert_eq!(
        f.seen.get("beeking|app|/r/app-old").map(String::as_str),
        Some("2026-11-14T10:00")
    );
    f.retain(&["dom".to_string()]);
    assert!(f.projects.is_empty() && f.seen.is_empty());
}
