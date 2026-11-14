//! The reference convention of „Mapa" (`[[slug]]`, `[[projekt/slug]]`, `^id`,
//! a `Czeka na: …` / `Powiązane: …` line) is free text for core: summaries
//! and task notes that carry it are accepted, read back as written, and
//! adding or removing one such line through the normal update path leaves
//! the rest of the file byte for byte as it was.

mod common;

use common::*;
use montybrain_core::*;

const TOPIC: &str = "whenubuy/tematy/ab-testy.md";

fn setup() -> (tempfile::TempDir, Vault) {
    let (dir, v) = vault();
    write(&v, "whenubuy/projekt.md", PROJECT);
    write(&v, TOPIC, &fixture("topic_spec.md"));
    (dir, v)
}

#[test]
fn references_in_a_summary_are_plain_text() {
    let (_d, v) = setup();
    let summary = "Wariant B na 20% ruchu, patrz [[checkout]] i [[montybrain/mapa]].\nCzeka na: [[whenubuy/platnosci]]\nPowiązane: ^d4e5f6";
    let t = v
        .update_topic(
            "whenubuy",
            "ab-testy",
            TopicPatch {
                summary: Some(summary.into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(t.summary, summary);
    let snap = v.load().unwrap();
    assert_eq!(snap.topic("whenubuy", "ab-testy").unwrap().summary, summary);
    // `^d4e5f6` in the summary is text, not a second copy of the task.
    assert_eq!(
        snap.tasks
            .iter()
            .filter(|t| t.id.as_deref() == Some("d4e5f6"))
            .count(),
        1
    );
    assert!(snap.warnings.is_empty(), "{:?}", snap.warnings);
}

#[test]
fn appending_and_removing_one_line_keeps_the_rest_byte_exact() {
    let (_d, v) = setup();
    let before_file = read(&v, TOPIC);
    let before = v
        .load()
        .unwrap()
        .topic("whenubuy", "ab-testy")
        .unwrap()
        .summary
        .clone();
    // What the map's "Połącz z…" sends: the summary plus one standalone line.
    let added = format!("{before}\nCzeka na: [[montybrain/mapa]]");
    v.update_topic(
        "whenubuy",
        "ab-testy",
        TopicPatch {
            summary: Some(added.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let with_line = read(&v, TOPIC);
    assert_eq!(
        v.load()
            .unwrap()
            .topic("whenubuy", "ab-testy")
            .unwrap()
            .summary,
        added
    );
    // Only the line was inserted (and `zaktualizowano` bumped).
    let strip = |s: &str| {
        s.lines()
            .filter(|l| !l.starts_with("zaktualizowano:"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let stripped = strip(&before_file);
    let mut expected: Vec<&str> = stripped.lines().collect();
    let last_summary_line = before.lines().last().unwrap();
    let at = expected
        .iter()
        .position(|l| *l == last_summary_line)
        .unwrap();
    expected.insert(at + 1, "Czeka na: [[montybrain/mapa]]");
    assert_eq!(strip(&with_line), expected.join("\n"));
    // Removing exactly that line: the file is the original again (bar the date).
    v.update_topic(
        "whenubuy",
        "ab-testy",
        TopicPatch {
            summary: Some(before.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(strip(&read(&v, TOPIC)), strip(&before_file));
}

#[test]
fn references_in_task_notes_round_trip() {
    let (_d, v) = setup();
    let loc = TaskLocation::Topic {
        project: "whenubuy".into(),
        topic: "ab-testy".into(),
    };
    let notes = "Czeka na: ^d4e5f6\nPowiązane: [[montybrain/mapa]] i [[checkout]]";
    let t = v
        .create_task(
            &loc,
            NewTask {
                title: "Z odwołaniami".into(),
                notes: notes.into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(t.notes, notes);
    let file = read(&v, TOPIC);
    let snap = v.load().unwrap();
    let back = snap.tasks.iter().find(|x| x.id == t.id).unwrap();
    assert_eq!(back.notes, notes);
    // The referenced id stays one task; nothing is reinterpreted.
    assert_eq!(
        snap.tasks
            .iter()
            .filter(|t| t.id.as_deref() == Some("d4e5f6"))
            .count(),
        1
    );
    // A no-op update writes nothing: the file round-trips byte for byte.
    v.update_task(
        t.id.as_deref().unwrap(),
        TaskPatch {
            notes: Some(notes.into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(read(&v, TOPIC), file);
}
