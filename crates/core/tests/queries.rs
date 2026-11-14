mod common;

use common::*;
use morwisko_core::*;

fn setup() -> (tempfile::TempDir, Vault) {
    let (dir, v) = vault();
    write(&v, "whenubuy/projekt.md", PROJECT);
    write(&v, "whenubuy/tematy/ab-testy.md", &fixture("topic_spec.md"));
    write(
        &v,
        "stary/projekt.md",
        "---\nnazwa: Stary\nstan: archiwum\n---\n## Zadania\n- [/] Zapomniane ^old001 @2026-01-01\n",
    );
    write(
        &v,
        "skrzynka.md",
        "- [ ] Zadzwonić do księgowej @2026-11-14T16:00 ^inb001\n- [ ] Przeczytać artykuł ^inb002\n- [ ] Zaległe @2026-11-01 !niski ^inb003\n- [x] Zrobione @2026-11-01 ✓2026-11-02 ^inb004\n",
    );
    (dir, v)
}

fn ids(tasks: &[Task]) -> Vec<&str> {
    tasks.iter().map(|t| t.id.as_deref().unwrap()).collect()
}

#[test]
fn filters() {
    let (_d, v) = setup();
    let s = v.load().unwrap();
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    let all_open = s.tasks(&TaskFilter::default());
    assert!(!ids(&all_open).contains(&"inb004"));
    assert!(!ids(&all_open).contains(&"a1b2c3"));
    let with_done = s.tasks(&TaskFilter {
        include_done: true,
        ..Default::default()
    });
    assert_eq!(with_done.len(), all_open.len() + 2);

    let wub = s.tasks(&TaskFilter {
        project: Some("whenubuy".into()),
        ..Default::default()
    });
    assert_eq!(ids(&wub), vec!["k3f9a2", "d4e5f6", "g7h8i9", "j1k2l3"]);
    let topic = s.tasks(&TaskFilter {
        project: Some("whenubuy".into()),
        topic: Some("ab-testy".into()),
        statuses: vec![TaskStatus::Done, TaskStatus::Blocked],
        ..Default::default()
    });
    assert_eq!(ids(&topic), vec!["a1b2c3", "j1k2l3"]);
    let inbox = s.tasks(&TaskFilter {
        inbox_only: true,
        ..Default::default()
    });
    assert_eq!(ids(&inbox), vec!["inb001", "inb002", "inb003"]);
    let due = s.tasks(&TaskFilter {
        due_from: Some(date("2026-11-10")),
        due_to: Some(date("2026-11-20")),
        ..Default::default()
    });
    assert_eq!(ids(&due), vec!["inb001", "g7h8i9"]);
}

#[test]
fn today_view() {
    let (_d, v) = setup();
    let s = v.load().unwrap();
    let today = s.today(date("2026-11-14"));
    let got: Vec<(TodayReason, &str)> = today
        .iter()
        .map(|i| (i.reason, i.task.id.as_deref().unwrap()))
        .collect();
    assert_eq!(
        got,
        vec![
            (TodayReason::Overdue, "inb003"),
            (TodayReason::DueToday, "inb001"),
            (TodayReason::Doing, "d4e5f6"),
            (TodayReason::Blocked, "j1k2l3"),
        ]
    );
}

#[test]
fn search_is_case_and_diacritic_insensitive() {
    let (_d, v) = setup();
    let s = v.load().unwrap();
    let hits = s.search("KSIEGOWEJ zadzwonic");
    assert_eq!(hits.len(), 1);
    assert!(matches!(&hits[0], SearchHit::Task { task } if task.id.as_deref() == Some("inb001")));
    let hits = s.search("free-form note");
    assert!(matches!(&hits[0], SearchHit::Task { task } if task.id.as_deref() == Some("g7h8i9")));
    let hits = s.search("current state");
    assert!(matches!(&hits[0], SearchHit::Topic { topic } if topic.slug == "ab-testy"));
    let hits = s.search("mr opened");
    assert!(matches!(&hits[0], SearchHit::Journal { entry } if entry.author == "agent"));
    assert!(s.search("   ").is_empty());
    assert!(s.search("nic-takiego-nie-ma").is_empty());
    let json = serde_json::to_string(&s.search("A/B")).unwrap();
    assert!(json.contains("\"kind\":\"topic\""));
}

#[test]
fn recent_journal_across_vault() {
    let (_d, v) = setup();
    v.create_topic(
        "whenubuy",
        NewTopic {
            title: "Drugi".into(),
            ..Default::default()
        },
    )
    .unwrap();
    v.add_journal_entry(
        "whenubuy",
        "drugi",
        NewJournalEntry {
            author: "agent".into(),
            text: "Najnowszy".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let s = v.load().unwrap();
    let recent = s.recent_journal(2);
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].text, "Najnowszy");
    assert_eq!(recent[0].topic, "drugi");
    assert_eq!(recent[1].text, "Tests green, MR opened.");
    assert_eq!(s.recent_journal(100).len(), 3);
}

#[test]
fn snapshot_serializes_to_json_and_back() {
    let (_d, v) = setup();
    let s = v.load().unwrap();
    let json = serde_json::to_string(&s).unwrap();
    assert!(json.contains("\"status\":\"in_progress\""));
    assert!(json.contains("\"due\":\"2026-11-14T16:00\""));
    assert!(json.contains("\"location\":{\"kind\":\"inbox\"}"));
    assert!(json.contains("\"updated\":\"2026-11-13T14:02\""));
    let back: Snapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(back, s);
}

#[test]
fn init_and_open() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    assert!(matches!(Vault::open(&root), Err(Error::VaultNotFound(_))));
    let v = Vault::init(&root).unwrap();
    assert!(root.join("skrzynka.md").is_file());
    assert!(root.join(".morwisko").is_dir());
    write(&v, "skrzynka.md", "- [ ] keep ^kkkkk1\n");
    Vault::init(&root).unwrap();
    assert_eq!(
        read(&v, "skrzynka.md"),
        "- [ ] keep ^kkkkk1\n",
        "init never overwrites"
    );
    let s = v.load().unwrap();
    assert_eq!(s.tasks.len(), 1);
    assert!(s.projects.is_empty());
    // Plain directories without projekt.md / tematy are not projects.
    std::fs::create_dir_all(root.join("random-dir")).unwrap();
    std::fs::create_dir_all(root.join("orphan/tematy")).unwrap();
    let s = v.load().unwrap();
    assert_eq!(s.projects.len(), 1);
    assert_eq!(s.projects[0].slug, "orphan");
    assert!(
        s.warnings
            .iter()
            .any(|w| w.message.contains("no readable projekt.md"))
    );
}
