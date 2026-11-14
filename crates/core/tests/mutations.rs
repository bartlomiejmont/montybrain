//! Every mutation is checked against the exact line diff it produces.

mod common;

use common::*;
use montybrain_core::*;

const TOPIC: &str = "whenubuy/tematy/ab-testy.md";
const UPD_OLD: &str = "-zaktualizowano: 2026-11-13T14:02";
const UPD_NEW: &str = "+zaktualizowano: 2026-11-14T09:15";

fn setup() -> (tempfile::TempDir, Vault) {
    let (dir, v) = vault();
    write(&v, "whenubuy/projekt.md", PROJECT);
    write(&v, TOPIC, &fixture("topic_spec.md"));
    (dir, v)
}

fn topic_loc() -> TaskLocation {
    TaskLocation::Topic {
        project: "whenubuy".into(),
        topic: "ab-testy".into(),
    }
}

/// Run `f` and return the diff it caused in `rel`.
fn diff_of(v: &Vault, rel: &str, f: impl FnOnce(&Vault)) -> Vec<String> {
    let before = read(v, rel);
    f(v);
    diff(&before, &read(v, rel))
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[test]
fn complete_task_adds_done_date() {
    let (_d, v) = setup();
    let d = diff_of(&v, TOPIC, |v| {
        let t = v
            .update_task(
                "g7h8i9",
                TaskPatch {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(t.done, Some(date("2026-11-14")));
        assert_eq!(t.status, TaskStatus::Done);
    });
    assert_eq!(
        d,
        s(&[
            UPD_OLD,
            UPD_NEW,
            "-- [ ] Todo task !wysoki @2026-11-20 ^g7h8i9",
            "+- [x] Todo task !wysoki @2026-11-20 ✓2026-11-14 ^g7h8i9",
        ])
    );
}

#[test]
fn reopen_task_removes_done_date() {
    let (_d, v) = setup();
    let d = diff_of(&v, TOPIC, |v| {
        let t = v
            .update_task(
                "^a1b2c3",
                TaskPatch {
                    status: Some(TaskStatus::Todo),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(t.done, None);
    });
    assert_eq!(
        d,
        s(&[
            UPD_OLD,
            UPD_NEW,
            "-- [x] Done task ✓2026-11-12 ^a1b2c3",
            "+- [ ] Done task ^a1b2c3",
        ])
    );
}

#[test]
fn status_change_only_swaps_checkbox() {
    let (_d, v) = vault();
    write(&v, "skrzynka.md", "* [ ]   odd    spacing  ^aaaaa1   \n");
    let d = diff_of(&v, "skrzynka.md", |v| {
        v.update_task(
            "aaaaa1",
            TaskPatch {
                status: Some(TaskStatus::Blocked),
                ..Default::default()
            },
        )
        .unwrap();
    });
    // No zaktualizowano in the inbox (key absent → not added).
    assert_eq!(
        d,
        s(&[
            "-* [ ]   odd    spacing  ^aaaaa1   ",
            "+* [!]   odd    spacing  ^aaaaa1   "
        ])
    );
}

#[test]
fn update_title_priority_due() {
    let (_d, v) = setup();
    let d = diff_of(&v, TOPIC, |v| {
        let t = v
            .update_task(
                "g7h8i9",
                TaskPatch {
                    title: Some("Renamed  task\n".into()),
                    priority: Some(Priority::Low),
                    due: Some(Some("2026-12-01T10:00".parse().unwrap())),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(t.title, "Renamed task");
        assert_eq!(t.links.len(), 1, "links are kept");
    });
    assert_eq!(
        d,
        s(&[
            UPD_OLD,
            UPD_NEW,
            "-- [ ] Todo task !wysoki @2026-11-20 ^g7h8i9",
            "+- [ ] Renamed task !niski @2026-12-01T10:00 ^g7h8i9",
        ])
    );
    let d = diff_of(&v, TOPIC, |v| {
        v.update_task(
            "g7h8i9",
            TaskPatch {
                due: Some(None),
                priority: Some(Priority::Normal),
                ..Default::default()
            },
        )
        .unwrap();
    });
    assert_eq!(
        d,
        s(&[
            "-- [ ] Renamed task !niski @2026-12-01T10:00 ^g7h8i9",
            "+- [ ] Renamed task ^g7h8i9",
        ])
    );
}

#[test]
fn ambiguous_title_is_rejected() {
    let (_d, v) = setup();
    let before = read(&v, TOPIC);
    let err = v
        .update_task(
            "j1k2l3",
            TaskPatch {
                title: Some("Meet @2026-11-20".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)), "{err}");
    assert_eq!(read(&v, TOPIC), before);
    // With a due date present the trailing `@…` in the title is unambiguous.
    let t = v
        .update_task(
            "g7h8i9",
            TaskPatch {
                title: Some("Meet @2026-11-20".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(t.title, "Meet @2026-11-20");
    // …but then clearing the due date would make it ambiguous.
    let err = v
        .update_task(
            "g7h8i9",
            TaskPatch {
                due: Some(None),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)), "{err}");
}

#[test]
fn links_and_notes() {
    let (_d, v) = setup();
    let d = diff_of(&v, TOPIC, |v| {
        let t = v
            .update_task(
                "g7h8i9",
                TaskPatch {
                    add_links: vec!["https://gitlab.com/x/-/merge_requests/43".into()],
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(t.links.len(), 2);
    });
    assert_eq!(
        d,
        s(&[
            UPD_OLD,
            UPD_NEW,
            "+  - https://gitlab.com/x/-/merge_requests/43"
        ])
    );
    let d = diff_of(&v, TOPIC, |v| {
        v.update_task(
            "g7h8i9",
            TaskPatch {
                remove_links: vec!["https://gitlab.com/group/repo/-/merge_requests/42".into()],
                notes: Some("New note\n  nested".into()),
                ..Default::default()
            },
        )
        .unwrap();
    });
    assert_eq!(
        d,
        s(&[
            "-  - https://gitlab.com/group/repo/-/merge_requests/42",
            "-  - any other indented line is a free-form note belonging to the task",
            "+  New note",
            "+    nested",
        ])
    );
    let t = v.load().unwrap().task("g7h8i9").unwrap().clone();
    assert_eq!(t.links, vec!["https://gitlab.com/x/-/merge_requests/43"]);
    assert_eq!(t.notes, "New note\n  nested");
    // notes on a task that had none
    let d = diff_of(&v, TOPIC, |v| {
        v.update_task(
            "j1k2l3",
            TaskPatch {
                notes: Some("czeka na review".into()),
                add_links: vec!["https://x.y/1".into()],
                ..Default::default()
            },
        )
        .unwrap();
    });
    assert_eq!(d, s(&["+  - https://x.y/1", "+  czeka na review"]));
}

#[test]
fn create_task_in_topic_appends_to_list() {
    let (_d, v) = setup();
    let mut id = String::new();
    let d = diff_of(&v, TOPIC, |v| {
        let t = v
            .create_task(
                &topic_loc(),
                NewTask {
                    title: "Nowe zadanie".into(),
                    priority: Priority::High,
                    due: Some("2026-11-30".parse().unwrap()),
                    links: vec!["https://example.com/mr/1".into()],
                    notes: "notatka".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        id = t.id.clone().unwrap();
        assert!(is_valid_id(&id));
        assert_eq!(t.location, topic_loc());
        assert_eq!(t.links, vec!["https://example.com/mr/1"]);
        assert_eq!(t.notes, "notatka");
    });
    assert_eq!(
        d,
        vec![
            UPD_OLD.to_string(),
            UPD_NEW.to_string(),
            format!("+- [ ] Nowe zadanie !wysoki @2026-11-30 ^{id}"),
            "+  - https://example.com/mr/1".to_string(),
            "+  notatka".to_string(),
        ]
    );
    let after = read(&v, TOPIC);
    assert!(after.contains(&format!(
        "- [!] Blocked task ^j1k2l3\n- [ ] Nowe zadanie !wysoki @2026-11-30 ^{id}\n"
    )));
}

#[test]
fn create_task_done_and_in_inbox_and_project() {
    let (_d, v) = setup();
    let t = v
        .create_task(
            &TaskLocation::Inbox,
            NewTask {
                title: "Kupić mleko".into(),
                status: TaskStatus::Done,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        read(&v, "skrzynka.md"),
        format!(
            "- [x] Kupić mleko ✓2026-11-14 ^{}\n",
            t.id.as_ref().unwrap()
        )
    );
    let d = diff_of(&v, "whenubuy/projekt.md", |v| {
        v.create_task(
            &TaskLocation::Project {
                project: "whenubuy".into(),
            },
            NewTask {
                title: "Projektowe".into(),
                ..Default::default()
            },
        )
        .unwrap();
    });
    assert_eq!(d.len(), 1);
    assert!(d[0].starts_with("+- [ ] Projektowe ^"));
    assert!(matches!(
        v.create_task(
            &TaskLocation::Project {
                project: "nie-ma".into()
            },
            NewTask {
                title: "x".into(),
                ..Default::default()
            }
        ),
        Err(Error::ProjectNotFound(_))
    ));
    assert!(matches!(
        v.create_task(
            &TaskLocation::Project {
                project: "../etc".into()
            },
            NewTask {
                title: "x".into(),
                ..Default::default()
            }
        ),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        v.create_task(&TaskLocation::Inbox, NewTask::default()),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn move_task_keeps_id_and_block() {
    let (_d, v) = setup();
    let topic_before = read(&v, TOPIC);
    let t = v.move_task("g7h8i9", &TaskLocation::Inbox).unwrap();
    assert_eq!(t.id.as_deref(), Some("g7h8i9"));
    assert_eq!(t.location, TaskLocation::Inbox);
    assert_eq!(t.links.len(), 1);
    assert_eq!(
        diff(&topic_before, &read(&v, TOPIC)),
        s(&[
            UPD_OLD,
            UPD_NEW,
            "-- [ ] Todo task !wysoki @2026-11-20 ^g7h8i9",
            "-  - https://gitlab.com/group/repo/-/merge_requests/42",
            "-  - any other indented line is a free-form note belonging to the task",
        ])
    );
    assert_eq!(
        read(&v, "skrzynka.md"),
        "- [ ] Todo task !wysoki @2026-11-20 ^g7h8i9\n  - https://gitlab.com/group/repo/-/merge_requests/42\n  - any other indented line is a free-form note belonging to the task\n"
    );
    // and into the project file
    let d = diff_of(&v, "whenubuy/projekt.md", |v| {
        v.move_task(
            "g7h8i9",
            &TaskLocation::Project {
                project: "whenubuy".into(),
            },
        )
        .unwrap();
    });
    assert_eq!(d.len(), 3);
    assert_eq!(read(&v, "skrzynka.md"), "");
    // moving to the same place is a no-op
    let before = read(&v, "whenubuy/projekt.md");
    v.move_task(
        "g7h8i9",
        &TaskLocation::Project {
            project: "whenubuy".into(),
        },
    )
    .unwrap();
    assert_eq!(read(&v, "whenubuy/projekt.md"), before);
    let snap = v.load().unwrap();
    assert_eq!(
        snap.tasks
            .iter()
            .filter(|t| t.id.as_deref() == Some("g7h8i9"))
            .count(),
        1
    );
}

#[test]
fn delete_task_removes_block() {
    let (_d, v) = setup();
    let d = diff_of(&v, TOPIC, |v| {
        let t = v.delete_task("g7h8i9").unwrap();
        assert_eq!(t.title, "Todo task");
    });
    assert_eq!(
        d,
        s(&[
            UPD_OLD,
            UPD_NEW,
            "-- [ ] Todo task !wysoki @2026-11-20 ^g7h8i9",
            "-  - https://gitlab.com/group/repo/-/merge_requests/42",
            "-  - any other indented line is a free-form note belonging to the task",
        ])
    );
    assert!(matches!(
        v.delete_task("g7h8i9"),
        Err(Error::TaskNotFound(_))
    ));
}

#[test]
fn journal_entry_with_status_and_summary() {
    let (_d, v) = setup();
    let d = diff_of(&v, TOPIC, |v| {
        let r = v
            .add_journal_entry(
                "whenubuy",
                "ab-testy",
                NewJournalEntry {
                    author: "claude · wątek 7".into(),
                    text: "Wyniki gotowe.\nDruga linia".into(),
                    status: Some(TopicStatus::Waiting),
                    summary: Some("Czekamy na decyzję.".into()),
                },
            )
            .unwrap();
        assert_eq!(r.entry.author, "claude wątek 7");
        assert_eq!(r.entry.text, "Wyniki gotowe.\nDruga linia");
        assert_eq!(r.topic.status, TopicStatus::Waiting);
        assert_eq!(r.topic.summary, "Czekamy na decyzję.");
    });
    assert_eq!(
        d,
        s(&[
            "-status: w-toku           # planowany | w-toku | zablokowany | czeka | zrobiony",
            "+status: czeka           # planowany | w-toku | zablokowany | czeka | zrobiony",
            UPD_OLD,
            UPD_NEW,
            "-Free text between the H1 and the first `##` = the topic's \"current state\" summary.",
            "+Czekamy na decyzję.",
            "+- 2026-11-14 09:15 · claude wątek 7 · Wyniki gotowe.",
            "+  Druga linia",
        ])
    );
    let snap = v.load().unwrap();
    let j = snap.journal_of("whenubuy", "ab-testy");
    assert_eq!(j.len(), 3);
    assert_eq!(j[0].text, "Wyniki gotowe.\nDruga linia");
    assert!(matches!(
        v.add_journal_entry("whenubuy", "ab-testy", NewJournalEntry::default()),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn update_topic_fields() {
    let (_d, v) = setup();
    let d = diff_of(&v, TOPIC, |v| {
        let t = v
            .update_topic(
                "whenubuy",
                "ab-testy",
                TopicPatch {
                    title: Some("Nowy tytuł".into()),
                    repo: Some(Some("whenubuy-mobile".into())),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(t.title, "Nowy tytuł");
        assert_eq!(t.slug, "ab-testy");
        assert_eq!(t.repo.as_deref(), Some("whenubuy-mobile"));
    });
    assert_eq!(
        d,
        s(&[
            "-repo: whenubuy-backend   # optional, name of one of the project's repos",
            "+repo: whenubuy-mobile   # optional, name of one of the project's repos",
            UPD_OLD,
            UPD_NEW,
            "-# A/B testy cen",
            "+# Nowy tytuł",
        ])
    );
    let d = diff_of(&v, TOPIC, |v| {
        v.update_topic(
            "whenubuy",
            "ab-testy",
            TopicPatch {
                repo: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
    });
    // Clearing a key whose line has a comment empties the value and keeps the comment.
    assert_eq!(
        d,
        s(&[
            "-repo: whenubuy-mobile   # optional, name of one of the project's repos",
            "+repo:   # optional, name of one of the project's repos",
        ])
    );
    let err = v
        .update_topic(
            "whenubuy",
            "ab-testy",
            TopicPatch {
                repo: Some(Some("nope".into())),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));
    // re-adding the repo fills the kept line, comment and spacing included
    let d = diff_of(&v, TOPIC, |v| {
        v.update_topic(
            "whenubuy",
            "ab-testy",
            TopicPatch {
                repo: Some(Some("whenubuy-backend".into())),
                ..Default::default()
            },
        )
        .unwrap();
    });
    assert_eq!(
        d,
        s(&[
            "-repo:   # optional, name of one of the project's repos",
            "+repo: whenubuy-backend   # optional, name of one of the project's repos",
        ])
    );
    assert!(read(&v, TOPIC).starts_with("---\nstatus: w-toku           # planowany | w-toku | zablokowany | czeka | zrobiony\nrepo: whenubuy-backend   # optional, name of one of the project's repos\n"));
    // no-op patch writes nothing
    let before = read(&v, TOPIC);
    v.update_topic("whenubuy", "ab-testy", TopicPatch::default())
        .unwrap();
    assert_eq!(read(&v, TOPIC), before);
}

#[test]
fn create_topic_exact_content_and_slug_collision() {
    let (_d, v) = setup();
    let t = v
        .create_topic(
            "whenubuy",
            NewTopic {
                title: "Płatności: Stripe → Przelewy24".into(),
                status: TopicStatus::InProgress,
                repo: Some("whenubuy-backend".into()),
                summary: "Migracja bramki.".into(),
            },
        )
        .unwrap();
    assert_eq!(t.slug, "platnosci-stripe-przelewy24");
    assert_eq!(
        read(&v, "whenubuy/tematy/platnosci-stripe-przelewy24.md"),
        "---\nstatus: w-toku\nrepo: whenubuy-backend\nutworzono: 2026-11-14\nzaktualizowano: 2026-11-14T09:15\n---\n# Płatności: Stripe → Przelewy24\n\nMigracja bramki.\n\n## Zadania\n\n## Dziennik\n"
    );
    let t2 = v
        .create_topic(
            "whenubuy",
            NewTopic {
                title: "Płatności Stripe Przelewy24".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(t2.slug, "platnosci-stripe-przelewy24-2");
    assert_eq!(t2.status, TopicStatus::Planned);
    let snap = v.load().unwrap();
    assert!(snap.warnings.is_empty(), "{:?}", snap.warnings);
    // first task + journal go into the empty sections
    let task = v
        .create_task(
            &TaskLocation::Topic {
                project: "whenubuy".into(),
                topic: t2.slug.clone(),
            },
            NewTask {
                title: "a".into(),
                ..Default::default()
            },
        )
        .unwrap();
    v.add_journal_entry(
        "whenubuy",
        &t2.slug,
        NewJournalEntry {
            author: "ja".into(),
            text: "start".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        read(&v, "whenubuy/tematy/platnosci-stripe-przelewy24-2.md"),
        format!(
            "---\nstatus: planowany\nutworzono: 2026-11-14\nzaktualizowano: 2026-11-14T09:15\n---\n# Płatności Stripe Przelewy24\n\n## Zadania\n- [ ] a ^{}\n\n## Dziennik\n- 2026-11-14 09:15 · ja · start\n",
            task.id.unwrap()
        )
    );
    assert!(matches!(
        v.create_topic(
            "nie-ma",
            NewTopic {
                title: "x".into(),
                ..Default::default()
            }
        ),
        Err(Error::ProjectNotFound(_))
    ));
}

#[test]
fn delete_topic_removes_file() {
    let (_d, v) = setup();
    let t = v.delete_topic("whenubuy", "ab-testy").unwrap();
    assert_eq!(t.title, "A/B testy cen");
    assert!(!v.root().join(TOPIC).exists());
    assert!(matches!(
        v.delete_topic("whenubuy", "ab-testy"),
        Err(Error::TopicNotFound { .. })
    ));
}

#[test]
fn create_project_exact_content() {
    let (_d, v) = vault();
    let p = v
        .create_project(NewProject {
            name: "Żółta Łódź".into(),
            color: Some("#a3e635".into()),
            repos: vec![Repo {
                name: "lodz-app".into(),
                path: "/Users/x/Projects/lodz app".into(),
            }],
            description: "Opis łodzi.".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(p.slug, "zolta-lodz");
    assert_eq!(p.state, ProjectState::Active);
    assert_eq!(
        read(&v, "zolta-lodz/projekt.md"),
        "---\nnazwa: Żółta Łódź\nkolor: \"#a3e635\"\nstan: aktywny\nrepozytoria:\n  - nazwa: lodz-app\n    sciezka: /Users/x/Projects/lodz app\n---\nOpis łodzi.\n\n## Zadania\n"
    );
    assert!(v.root().join("zolta-lodz/tematy").is_dir());
    let p2 = v
        .create_project(NewProject {
            name: "zolta lodz".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(p2.slug, "zolta-lodz-2");
    assert_eq!(
        read(&v, "zolta-lodz-2/projekt.md"),
        "---\nnazwa: zolta lodz\nstan: aktywny\n---\n## Zadania\n"
    );
    let snap = v.load().unwrap();
    assert_eq!(snap.projects.len(), 2);
    assert!(snap.warnings.is_empty(), "{:?}", snap.warnings);
}

#[test]
fn update_and_archive_project() {
    let (_d, v) = setup();
    let pf = "whenubuy/projekt.md";
    let d = diff_of(&v, pf, |v| {
        let p = v
            .update_project(
                "whenubuy",
                ProjectPatch {
                    name: Some("WhenUBuy 2".into()),
                    color: Some(None),
                    repos: Some(vec![
                        Repo {
                            name: "whenubuy-backend".into(),
                            path: "/Users/x/Projects/WhenUBuy/whenubuy-backend".into(),
                        },
                        Repo {
                            name: "whenubuy-infra".into(),
                            path: "/Users/x/infra".into(),
                        },
                    ]),
                    description: Some("Nowy opis.\nDruga linia.".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(p.repos.len(), 2);
        assert_eq!(p.color, None);
    });
    assert_eq!(
        d,
        s(&[
            "-nazwa: WhenUBuy",
            "-kolor: \"#a3e635\"",
            "+nazwa: WhenUBuy 2",
            "-  - nazwa: whenubuy-mobile",
            "-    sciezka: /Users/x/Projects/WhenUBuy/whenubuy-mobile",
            "+  - nazwa: whenubuy-infra",
            "+    sciezka: /Users/x/infra",
            "-Opis projektu.",
            "+Nowy opis.",
            "+Druga linia.",
        ])
    );
    let d = diff_of(&v, pf, |v| {
        let p = v.archive_project("whenubuy").unwrap();
        assert_eq!(p.state, ProjectState::Archived);
    });
    assert_eq!(d, s(&["-stan: aktywny", "+stan: archiwum"]));
    let d = diff_of(&v, pf, |v| {
        v.update_project(
            "whenubuy",
            ProjectPatch {
                color: Some(Some("#000000".into())),
                ..Default::default()
            },
        )
        .unwrap();
    });
    assert_eq!(d, s(&["+kolor: \"#000000\""]));
    assert!(
        read(&v, pf).starts_with("---\nnazwa: WhenUBuy 2\nkolor: \"#000000\"\nstan: archiwum\n")
    );
}

#[test]
fn mutations_on_missing_entities() {
    let (_d, v) = setup();
    assert!(matches!(
        v.update_task("zzzzzz", TaskPatch::default()),
        Err(Error::TaskNotFound(_))
    ));
    assert!(matches!(
        v.update_topic("whenubuy", "nope", TopicPatch::default()),
        Err(Error::TopicNotFound { .. })
    ));
    assert!(matches!(
        v.update_project("nope", ProjectPatch::default()),
        Err(Error::ProjectNotFound(_))
    ));
    assert!(matches!(
        v.move_task("zzzzzz", &TaskLocation::Inbox),
        Err(Error::TaskNotFound(_))
    ));
    assert!(matches!(
        v.move_task(
            "g7h8i9",
            &TaskLocation::Topic {
                project: "whenubuy".into(),
                topic: "nope".into()
            }
        ),
        Err(Error::TopicNotFound { .. })
    ));
    // failed move changed nothing
    assert!(v.load().unwrap().task("g7h8i9").is_some());
}
