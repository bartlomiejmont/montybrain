mod common;

use common::*;
use morwisko_core::*;

const CRLF_PROJECT: &str = "---\r\nnazwa: Okna CRLF\r\nstan: wstrzymany\r\n---\r\nOpis\r\n\r\n## Zadania\r\n- [ ] Pierwsze ^ccccc1\r\n- [ ] Drugie ^ccccc2\r\n  - https://example.com/crlf\r\n";

const BOM_INBOX: &str = "\u{feff}- [ ] Z BOM ^ddddd1\n";

fn messy_vault() -> (tempfile::TempDir, Vault) {
    let (dir, v) = vault();
    write(&v, "whenubuy/projekt.md", PROJECT);
    write(&v, "whenubuy/tematy/ab-testy.md", &fixture("topic_spec.md"));
    write(&v, "whenubuy/tematy/messy.md", &fixture("topic_messy.md"));
    write(&v, "whenubuy/tematy/pusty.md", &fixture("empty.md"));
    write(&v, "skrzynka.md", &fixture("inbox_messy.md"));
    write(&v, "okna/projekt.md", CRLF_PROJECT);
    write(&v, "inny/projekt.md", "---\nnazwa: Inny\n---\n");
    (dir, v)
}

const FILES: &[&str] = &[
    "whenubuy/projekt.md",
    "whenubuy/tematy/ab-testy.md",
    "whenubuy/tematy/messy.md",
    "whenubuy/tematy/pusty.md",
    "skrzynka.md",
    "okna/projekt.md",
];

fn contents(v: &Vault) -> Vec<String> {
    FILES.iter().map(|f| read(v, f)).collect()
}

#[test]
fn load_and_unrelated_mutations_leave_files_byte_identical() {
    let (_d, v) = messy_vault();
    let before = contents(&v);
    let snap = v.load().unwrap();
    assert_eq!(snap.projects.len(), 3);
    // Mutations elsewhere in the vault must not touch any of these files.
    let t = v
        .create_task(
            &TaskLocation::Project {
                project: "inny".into(),
            },
            NewTask {
                title: "Gdzie indziej".into(),
                ..Default::default()
            },
        )
        .unwrap();
    v.update_task(
        t.id.as_deref().unwrap(),
        TaskPatch {
            status: Some(TaskStatus::Done),
            ..Default::default()
        },
    )
    .unwrap();
    v.create_topic(
        "inny",
        NewTopic {
            title: "Temat".into(),
            ..Default::default()
        },
    )
    .unwrap();
    v.load().unwrap();
    assert_eq!(contents(&v), before);
}

#[test]
fn messy_topic_is_parsed_tolerantly() {
    let (_d, v) = messy_vault();
    let snap = v.load().unwrap();
    let topic = snap.topic("whenubuy", "messy").unwrap();
    assert_eq!(topic.title, "Messy   topic");
    assert_eq!(topic.status, TopicStatus::Waiting);
    assert_eq!(topic.created, Some(date("2026-11-01")));
    assert_eq!(topic.updated, None);
    assert!(
        topic
            .summary
            .starts_with("Summary line one.\n   indented summary line")
    );
    assert!(topic.summary.contains("## Not a section"));

    let tasks = snap.tasks(&TaskFilter {
        project: Some("whenubuy".into()),
        topic: Some("messy".into()),
        include_done: true,
        ..Default::default()
    });
    let titles: Vec<_> = tasks.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(
        titles,
        vec![
            "Odd   spacing   task",
            "Uppercase done",
            "No id here",
            "Bad date @2026-02-30",
            "Time due",
            "Task under sub heading",
        ]
    );
    assert_eq!(tasks[0].id.as_deref(), Some("aaaaa1"));
    assert_eq!(tasks[1].status, TaskStatus::Done);
    assert_eq!(tasks[1].done, Some(date("2026-11-10")));
    assert_eq!(tasks[2].id, None);
    assert_eq!(tasks[2].priority, Priority::Low);
    assert_eq!(tasks[3].id.as_deref(), Some("aaaaa3"));
    assert_eq!(tasks[3].links, vec!["http://example.com/a"]);
    assert_eq!(tasks[3].notes, "tab-indented note\n\nnote after blank line");
    assert_eq!(tasks[4].due.unwrap().to_string(), "2026-11-20T09:30");
    assert_eq!(tasks[4].notes, "tab note");
    assert!(
        snap.task("nottsk").is_none(),
        "tasks in unknown sections are ignored"
    );
    assert!(
        snap.task("zzzzzz").is_none(),
        "tasks in code fences are ignored"
    );

    let journal = snap.journal_of("whenubuy", "messy");
    assert_eq!(journal.len(), 3);
    assert_eq!(journal[0].text, "Entry with\ncontinuation line");
    assert_eq!(journal[1].author, "ja");
    assert_eq!(journal[1].text, "tight separators");
    assert_eq!(journal[2].text, "extra space after bullet");

    let msgs: Vec<String> = snap
        .warnings
        .iter()
        .filter(|w| w.path.ends_with("messy.md"))
        .map(|w| format!("{:?}: {}", w.line, w.message))
        .collect();
    let joined = msgs.join("\n");
    assert!(joined.contains("unknown task status `[?]`"), "{joined}");
    assert!(joined.contains("empty checkbox"), "{joined}");
    assert!(
        joined.contains("invalid due date `@2026-02-30`"),
        "{joined}"
    );
    assert!(
        joined.contains("not a journal entry") || joined.contains("journal entry must look like"),
        "{joined}"
    );
    assert!(joined.contains("1 task(s) without ^id"), "{joined}");
    // line numbers are 1-based
    assert!(msgs.iter().any(|m| m.starts_with("Some(29)")), "{joined}");
}

#[test]
fn empty_and_bom_and_crlf_files_load() {
    let (_d, v) = messy_vault();
    write(&v, "skrzynka.md", BOM_INBOX);
    let snap = v.load().unwrap();
    let pusty = snap.topic("whenubuy", "pusty").unwrap();
    assert_eq!(pusty.title, "pusty");
    assert_eq!(pusty.status, TopicStatus::Planned);
    let okna = snap.project("okna").unwrap();
    assert_eq!(okna.name, "Okna CRLF");
    assert_eq!(okna.state, ProjectState::Paused);
    assert_eq!(okna.description, "Opis");
    let t = snap.task("ccccc2").unwrap();
    assert_eq!(t.links, vec!["https://example.com/crlf"]);
    assert_eq!(snap.task("ddddd1").unwrap().title, "Z BOM");
}

#[test]
fn inbox_tasks_outside_other_sections() {
    let (_d, v) = messy_vault();
    let snap = v.load().unwrap();
    let inbox = snap.tasks(&TaskFilter {
        inbox_only: true,
        ..Default::default()
    });
    let titles: Vec<_> = inbox.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(
        titles,
        vec!["Inbox task", "Hand written without id", "Under heading"]
    );
    assert_eq!(inbox[1].links, vec!["https://example.org/x"]);
}

#[test]
fn normalize_assigns_ids_with_minimal_diff_and_is_idempotent() {
    let (_d, v) = messy_vault();
    let before_topic = read(&v, "whenubuy/tematy/messy.md");
    let before_inbox = read(&v, "skrzynka.md");
    let before_spec = read(&v, "whenubuy/tematy/ab-testy.md");
    let report = v.normalize().unwrap();
    assert_eq!(report.ids_assigned, 2);
    let mut changed: Vec<String> = report
        .files_changed
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    changed.sort();
    assert_eq!(changed, vec!["skrzynka.md", "whenubuy/tematy/messy.md"]);

    let snap = v.load().unwrap();
    let no_id = snap.tasks.iter().find(|t| t.title == "No id here").unwrap();
    let id = no_id.id.clone().unwrap();
    assert!(is_valid_id(&id));
    assert_eq!(
        diff(&before_topic, &read(&v, "whenubuy/tematy/messy.md")),
        vec![
            "-- [ ] No id here !niski".to_string(),
            format!("+- [ ] No id here !niski ^{id}")
        ]
    );
    let hw = snap
        .tasks
        .iter()
        .find(|t| t.title == "Hand written without id")
        .unwrap();
    let id2 = hw.id.clone().unwrap();
    assert_eq!(
        diff(&before_inbox, &read(&v, "skrzynka.md")),
        vec![
            "-- [ ] Hand written without id".to_string(),
            format!("+- [ ] Hand written without id ^{id2}")
        ]
    );
    assert_eq!(read(&v, "whenubuy/tematy/ab-testy.md"), before_spec);
    assert!(
        !snap
            .warnings
            .iter()
            .any(|w| w.message.contains("without ^id"))
    );

    let again = v.normalize().unwrap();
    assert_eq!(again, NormalizeReport::default());
}

#[test]
fn normalize_fixes_duplicate_ids() {
    let (_d, v) = vault();
    write(&v, "skrzynka.md", "- [ ] A ^dup111\n- [ ] B ^dup111\n");
    let snap = v.load().unwrap();
    assert!(
        snap.warnings
            .iter()
            .any(|w| w.message.contains("duplicate task id"))
    );
    let r = v.normalize().unwrap();
    assert_eq!(r.ids_assigned, 1);
    let snap = v.load().unwrap();
    assert_eq!(snap.tasks[0].id.as_deref(), Some("dup111"));
    assert_ne!(snap.tasks[1].id.as_deref(), Some("dup111"));
    assert!(snap.warnings.is_empty(), "{:?}", snap.warnings);
}

#[test]
fn crlf_is_preserved_by_mutations() {
    let (_d, v) = messy_vault();
    v.update_task(
        "ccccc1",
        TaskPatch {
            status: Some(TaskStatus::Doing),
            ..Default::default()
        },
    )
    .unwrap();
    v.create_task(
        &TaskLocation::Project {
            project: "okna".into(),
        },
        NewTask {
            title: "Trzecie".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let after = read(&v, "okna/projekt.md");
    let d = diff(CRLF_PROJECT, &after);
    assert_eq!(d.len(), 3, "{d:?}");
    assert_eq!(d[0], "-- [ ] Pierwsze ^ccccc1\r");
    assert_eq!(d[1], "+- [/] Pierwsze ^ccccc1\r");
    assert!(
        d[2].starts_with("+- [ ] Trzecie ^") && d[2].ends_with('\r'),
        "{d:?}"
    );
    assert!(
        !after.replace("\r\n", "").contains('\n'),
        "all line endings stay CRLF"
    );
}

#[test]
fn missing_trailing_newline_is_preserved() {
    let (_d, v) = messy_vault();
    let before = read(&v, "whenubuy/tematy/messy.md");
    assert!(!before.ends_with('\n'));
    let t = v
        .create_task(
            &TaskLocation::Topic {
                project: "whenubuy".into(),
                topic: "messy".into(),
            },
            NewTask {
                title: "Nowe".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let after = read(&v, "whenubuy/tematy/messy.md");
    assert!(!after.ends_with('\n'));
    let id = t.id.unwrap();
    assert_eq!(
        diff(&before, &after),
        vec![
            "+zaktualizowano: 2026-11-14T09:15".to_string(),
            format!("+- [ ] Nowe ^{id}"),
        ]
    );
    // The new task lands right after the last task of `## Zadania`.
    assert!(after.contains(&format!(
        "- [ ] Task under sub heading ^aaaaa5\n- [ ] Nowe ^{id}\n"
    )));

    // Appending at the very end of a file without trailing newline.
    write(&v, "skrzynka.md", "- [ ] last ^eeeee1");
    let t = v
        .create_task(
            &TaskLocation::Inbox,
            NewTask {
                title: "after".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        read(&v, "skrzynka.md"),
        format!("- [ ] last ^eeeee1\n- [ ] after ^{}", t.id.unwrap())
    );
}

#[test]
fn empty_topic_file_gets_minimal_structure() {
    let (_d, v) = messy_vault();
    let t = v
        .create_task(
            &TaskLocation::Topic {
                project: "whenubuy".into(),
                topic: "pusty".into(),
            },
            NewTask {
                title: "x".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        read(&v, "whenubuy/tematy/pusty.md"),
        format!(
            "---\nzaktualizowano: 2026-11-14T09:15\n---\n## Zadania\n- [ ] x ^{}\n",
            t.id.unwrap()
        )
    );
}

#[test]
fn invalid_utf8_file_is_a_warning_not_a_failure() {
    let (_d, v) = messy_vault();
    std::fs::write(
        v.root().join("whenubuy/tematy/binary.md"),
        [0xff, 0xfe, 0x00, 0x41],
    )
    .unwrap();
    let snap = v.load().unwrap();
    assert!(snap.topic("whenubuy", "ab-testy").is_some());
    assert!(snap.topic("whenubuy", "binary").is_none());
    assert!(
        snap.warnings
            .iter()
            .any(|w| w.path.ends_with("binary.md") && w.message.contains("UTF-8"))
    );
    // Mutations elsewhere still work.
    v.update_task(
        "g7h8i9",
        TaskPatch {
            status: Some(TaskStatus::Doing),
            ..Default::default()
        },
    )
    .unwrap();
}

#[test]
fn unclosed_frontmatter_is_kept_and_warned() {
    let (_d, v) = vault();
    let src = "---\nstatus: w-toku\n# Tytuł\n\n## Zadania\n- [ ] a ^fffff1\n";
    write(&v, "p/projekt.md", "---\nnazwa: P\n---\n");
    write(&v, "p/tematy/t.md", src);
    let snap = v.load().unwrap();
    assert!(
        snap.warnings
            .iter()
            .any(|w| w.message.contains("never closed"))
    );
    assert_eq!(snap.topic("p", "t").unwrap().title, "Tytuł");
    assert_eq!(snap.task("fffff1").unwrap().title, "a");
    assert_eq!(read(&v, "p/tematy/t.md"), src);
}
