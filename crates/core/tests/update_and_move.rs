//! M3: `Vault::update_and_move_task` validates everything and applies the edit and
//! the move in one locked transaction.

mod common;

use common::*;
use morwisko_core::*;

fn proj_loc(p: &str) -> TaskLocation {
    TaskLocation::Project { project: p.into() }
}

fn topic_setup() -> (tempfile::TempDir, Vault) {
    let (d, v) = vault();
    write(&v, "whenubuy/projekt.md", PROJECT);
    write(&v, "whenubuy/tematy/ab-testy.md", &fixture("topic_spec.md"));
    (d, v)
}

// ---------------------------------------------------------------------------
// M3: edit + move in one transaction
// ---------------------------------------------------------------------------

#[test]
fn m3_update_and_move_is_all_or_nothing() {
    let (_d, v) = topic_setup();
    std::fs::create_dir_all(v.root().join("bad")).unwrap();
    std::fs::write(v.root().join("bad/projekt.md"), [0xff, 0xfe]).unwrap();
    let topic = read(&v, "whenubuy/tematy/ab-testy.md");
    let err = v
        .update_and_move_task(
            "g7h8i9",
            TaskPatch {
                title: Some("Renamed".into()),
                ..Default::default()
            },
            &proj_loc("bad"),
        )
        .unwrap_err();
    assert!(matches!(err, Error::Unparseable { .. }), "{err:?}");
    assert_eq!(
        read(&v, "whenubuy/tematy/ab-testy.md"),
        topic,
        "nothing written"
    );
    let err = v
        .update_and_move_task(
            "g7h8i9",
            TaskPatch {
                title: Some("Renamed".into()),
                ..Default::default()
            },
            &TaskLocation::Topic {
                project: "whenubuy".into(),
                topic: "nope".into(),
            },
        )
        .unwrap_err();
    assert!(matches!(err, Error::TopicNotFound { .. }), "{err:?}");
    assert_eq!(read(&v, "whenubuy/tematy/ab-testy.md"), topic);
    // Invalid patch: nothing moves.
    let err = v
        .update_and_move_task(
            "g7h8i9",
            TaskPatch {
                add_links: vec!["nope".into()],
                ..Default::default()
            },
            &TaskLocation::Inbox,
        )
        .unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));
    assert_eq!(read(&v, "whenubuy/tematy/ab-testy.md"), topic);

    let t = v
        .update_and_move_task(
            "g7h8i9",
            TaskPatch {
                title: Some("Renamed".into()),
                status: Some(TaskStatus::Doing),
                add_links: vec!["https://x.y/2".into()],
                ..Default::default()
            },
            &TaskLocation::Inbox,
        )
        .unwrap();
    assert_eq!(t.location, TaskLocation::Inbox);
    assert_eq!(t.title, "Renamed");
    assert_eq!(t.status, TaskStatus::Doing);
    assert_eq!(t.links.len(), 2);
    assert_eq!(
        read(&v, "skrzynka.md"),
        "- [/] Renamed !wysoki @2026-11-20 ^g7h8i9\n  - https://gitlab.com/group/repo/-/merge_requests/42\n  - https://x.y/2\n  - any other indented line is a free-form note belonging to the task\n"
    );
    assert!(!read(&v, "whenubuy/tematy/ab-testy.md").contains("g7h8i9"));
    // Same location: plain update.
    let t = v
        .update_and_move_task(
            "g7h8i9",
            TaskPatch {
                status: Some(TaskStatus::Todo),
                ..Default::default()
            },
            &TaskLocation::Inbox,
        )
        .unwrap();
    assert_eq!(t.status, TaskStatus::Todo);
}
