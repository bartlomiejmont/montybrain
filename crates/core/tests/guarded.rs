//! Guarded mutations: undo only while the target is still what was written.

mod common;

use common::*;
use morwisko_core::*;

const TOPIC: &str = "whenubuy/tematy/ab-testy.md";
const UPD_LINE: &str = "zaktualizowano: 2026-11-14T09:15";

fn setup() -> (tempfile::TempDir, Vault) {
    let (dir, v) = vault();
    write(&v, "whenubuy/projekt.md", PROJECT);
    write(&v, TOPIC, &fixture("topic_spec.md"));
    (dir, v)
}

/// The file with its `zaktualizowano` line normalised (the only line an
/// undo cannot take back).
fn without_updated(s: &str) -> String {
    s.lines()
        .map(|l| {
            if l.starts_with("zaktualizowano:") {
                UPD_LINE
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn update_task_if_writes_only_when_the_guard_holds() {
    let (_d, v) = setup();
    let before = read(&v, TOPIC);
    let patch = || TaskPatch {
        status: Some(TaskStatus::Todo),
        ..Default::default()
    };
    // d4e5f6 is "doing": a guard expecting "done" refuses, nothing is written.
    let r = v
        .update_task_if("d4e5f6", |t| t.status == TaskStatus::Done, patch())
        .unwrap();
    assert_eq!(r, None);
    assert_eq!(read(&v, TOPIC), before);
    let r = v
        .update_task_if("d4e5f6", |t| t.status == TaskStatus::Doing, patch())
        .unwrap();
    assert_eq!(r.unwrap().status, TaskStatus::Todo);
    assert!(v.update_task_if("zzzzzz", |_| true, patch()).is_err());
}

#[test]
fn delete_task_if_removes_an_untouched_task_byte_exactly() {
    let (_d, v) = setup();
    let before = read(&v, TOPIC);
    let loc = TaskLocation::Topic {
        project: "whenubuy".into(),
        topic: "ab-testy".into(),
    };
    let created = v
        .create_task(
            &loc,
            NewTask {
                title: "Nowe".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let id = created.id.clone().unwrap();
    let untouched =
        |t: &Task| t.title == created.title && t.status == created.status && t.notes.is_empty();
    // Edited meanwhile: left alone.
    v.update_task(
        &id,
        TaskPatch {
            notes: Some("dopisane".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(v.delete_task_if(&id, untouched).unwrap(), None);
    assert!(v.load().unwrap().task(&id).is_some());
    v.update_task(
        &id,
        TaskPatch {
            notes: Some(String::new()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(v.delete_task_if(&id, untouched).unwrap().is_some());
    assert_eq!(without_updated(&read(&v, TOPIC)), without_updated(&before));
}

fn added(v: &Vault, status: Option<TopicStatus>, summary: Option<&str>) -> JournalAdded {
    v.add_journal_entry(
        "whenubuy",
        "ab-testy",
        NewJournalEntry {
            author: "codex".into(),
            text: "Webhook gotowy.\nZostały testy.".into(),
            status,
            summary: summary.map(String::from),
        },
    )
    .unwrap()
}

#[test]
fn revert_journal_entry_takes_back_entry_status_and_summary() {
    let (_d, v) = setup();
    let before = read(&v, TOPIC);
    let old = v
        .load()
        .unwrap()
        .topic("whenubuy", "ab-testy")
        .unwrap()
        .clone();
    let a = added(&v, Some(TopicStatus::Waiting), Some("Nowy stan."));
    let r = v
        .revert_journal_entry(
            "whenubuy",
            "ab-testy",
            JournalRevert {
                entry: a.entry.clone(),
                status: Some((TopicStatus::Waiting, old.status)),
                summary: Some(("Nowy stan.".into(), old.summary.clone())),
            },
        )
        .unwrap();
    assert!(
        r.entry_removed && r.status_restored && r.summary_restored,
        "{r:?}"
    );
    // Byte-exact apart from `zaktualizowano`.
    assert_eq!(without_updated(&read(&v, TOPIC)), without_updated(&before));
    // Again: nothing left to take back, nothing written.
    let now = read(&v, TOPIC);
    let r = v
        .revert_journal_entry(
            "whenubuy",
            "ab-testy",
            JournalRevert {
                entry: a.entry,
                status: Some((TopicStatus::Waiting, old.status)),
                summary: None,
            },
        )
        .unwrap();
    assert!(!r.entry_removed && !r.status_restored);
    assert_eq!(read(&v, TOPIC), now);
}

#[test]
fn revert_journal_entry_leaves_what_changed_meanwhile() {
    let (_d, v) = setup();
    let old = v
        .load()
        .unwrap()
        .topic("whenubuy", "ab-testy")
        .unwrap()
        .clone();
    let a = added(&v, Some(TopicStatus::Waiting), Some("Nowy stan."));
    // The user then set another status and a later entry came in.
    v.update_topic(
        "whenubuy",
        "ab-testy",
        TopicPatch {
            status: Some(TopicStatus::Blocked),
            ..Default::default()
        },
    )
    .unwrap();
    v.add_journal_entry(
        "whenubuy",
        "ab-testy",
        NewJournalEntry {
            author: "ja".into(),
            text: "Później.".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let r = v
        .revert_journal_entry(
            "whenubuy",
            "ab-testy",
            JournalRevert {
                entry: a.entry.clone(),
                status: Some((TopicStatus::Waiting, old.status)),
                summary: Some(("Nowy stan.".into(), old.summary.clone())),
            },
        )
        .unwrap();
    assert!(
        r.entry_removed && !r.status_restored && r.summary_restored,
        "{r:?}"
    );
    let snap = v.load().unwrap();
    let t = snap.topic("whenubuy", "ab-testy").unwrap();
    assert_eq!(t.status, TopicStatus::Blocked);
    let journal = snap.journal_of("whenubuy", "ab-testy");
    assert_eq!(journal.len(), 3, "the later entry and the old ones stay");
    assert!(journal.iter().all(|j| j.text != a.entry.text));

    // An edited entry is not "exactly what was written": left alone.
    let b = added(&v, None, None);
    let path = v.root().join(TOPIC);
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("Zostały testy.", "Zostały testy BLIK.");
    std::fs::write(&path, text).unwrap();
    let r = v
        .revert_journal_entry(
            "whenubuy",
            "ab-testy",
            JournalRevert {
                entry: b.entry,
                status: None,
                summary: None,
            },
        )
        .unwrap();
    assert!(!r.entry_removed);
}

fn topic_with(v: &Vault) -> TopicCreated {
    v.create_topic_with(
        "whenubuy",
        NewTopic {
            title: "Płatności BLIK".into(),
            status: TopicStatus::InProgress,
            repo: Some("whenubuy-backend".into()),
            summary: "Integracja BLIK w toku (wnioskowane z gałęzi).".into(),
        },
        Some(NewJournalEntry {
            author: "codex".into(),
            text: "Gałąź feature/blik, 5 commitów od 10 lis.".into(),
            ..Default::default()
        }),
        vec![
            NewTask {
                title: "Testy sandbox".into(),
                ..Default::default()
            },
            NewTask {
                title: "Webhook".into(),
                status: TaskStatus::Done,
                ..Default::default()
            },
        ],
    )
    .unwrap()
}

#[test]
fn create_topic_with_writes_one_complete_file() {
    let (_d, v) = setup();
    let c = topic_with(&v);
    assert_eq!(c.topic.slug, "platnosci-blik");
    let rel = "whenubuy/tematy/platnosci-blik.md";
    let file = read(&v, rel);
    assert_eq!(
        file, c.contents,
        "the returned contents are what is on disk"
    );
    let ids: Vec<String> = c.tasks.iter().map(|t| t.id.clone().unwrap()).collect();
    assert_eq!(
        file,
        format!(
            "---\nstatus: w-toku\nrepo: whenubuy-backend\nutworzono: 2026-11-14\nzaktualizowano: 2026-11-14T09:15\n---\n\
# Płatności BLIK\n\nIntegracja BLIK w toku (wnioskowane z gałęzi).\n\n## Zadania\n- [ ] Testy sandbox ^{}\n\
- [x] Webhook ✓2026-11-14 ^{}\n\n## Dziennik\n- 2026-11-14 09:15 · codex · Gałąź feature/blik, 5 commitów od 10 lis.\n",
            ids[0], ids[1]
        )
    );
    let snap = v.load().unwrap();
    assert!(snap.warnings.is_empty(), "{:?}", snap.warnings);
    assert_eq!(
        snap.journal_of("whenubuy", "platnosci-blik")[0].author,
        "codex"
    );
    assert_eq!(
        c.entry.unwrap().text,
        "Gałąź feature/blik, 5 commitów od 10 lis."
    );

    // Nothing is written when any part is invalid.
    let bad = v.create_topic_with(
        "whenubuy",
        NewTopic {
            title: "Zły temat".into(),
            summary: "## Zadania\n- [ ] wstrzyknięte".into(),
            ..Default::default()
        },
        None,
        vec![],
    );
    assert!(bad.is_err());
    let bad_task = v.create_topic_with(
        "whenubuy",
        NewTopic {
            title: "Zły temat".into(),
            ..Default::default()
        },
        None,
        vec![NewTask {
            title: "x @2026-11-20".into(),
            ..Default::default()
        }],
    );
    assert!(bad_task.is_err());
    let bad_repo = v.create_topic_with(
        "whenubuy",
        NewTopic {
            title: "Zły temat".into(),
            repo: Some("nie-ma".into()),
            ..Default::default()
        },
        None,
        vec![],
    );
    assert!(bad_repo.is_err());
    assert!(!v.root().join("whenubuy/tematy/zly-temat.md").exists());
}

#[test]
fn delete_topic_if_removes_only_an_untouched_file() {
    let (_d, v) = setup();
    let c = topic_with(&v);
    let same = |text: &str| text == c.contents;
    // Changed meanwhile (a task ticked): left alone.
    let id = c.tasks[0].id.clone().unwrap();
    v.update_task(
        &id,
        TaskPatch {
            status: Some(TaskStatus::Doing),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        v.delete_topic_if("whenubuy", "platnosci-blik", same)
            .unwrap(),
        None
    );
    assert!(v.root().join("whenubuy/tematy/platnosci-blik.md").exists());
    // Untouched: removed.
    let d = topic_with(&v);
    assert_eq!(d.topic.slug, "platnosci-blik-2");
    let same2 = |text: &str| text == d.contents;
    assert!(
        v.delete_topic_if("whenubuy", "platnosci-blik-2", same2)
            .unwrap()
            .is_some()
    );
    assert!(
        !v.root()
            .join("whenubuy/tematy/platnosci-blik-2.md")
            .exists()
    );
    assert!(
        v.delete_topic_if("whenubuy", "platnosci-blik-2", |_| true)
            .is_err()
    );
}

fn loc() -> TaskLocation {
    TaskLocation::Topic {
        project: "whenubuy".into(),
        topic: "ab-testy".into(),
    }
}

#[test]
fn an_undo_can_put_back_the_old_zaktualizowano() {
    let (_d, v) = setup();
    let before = read(&v, TOPIC);
    let old_stamp = v.stamp_of(&loc()).unwrap().unwrap();
    assert!(before.contains(&format!("zaktualizowano: {old_stamp}")));

    // Journal entry with status and summary, taken back: byte-identical.
    let old = v
        .load()
        .unwrap()
        .topic("whenubuy", "ab-testy")
        .unwrap()
        .clone();
    let a = added(&v, Some(TopicStatus::Waiting), Some("Nowy stan."));
    let written = v.stamp_of(&loc()).unwrap().unwrap();
    assert_ne!(written, old_stamp, "the write bumped it");
    let stamp = Stamp {
        written: written.clone(),
        before: old_stamp.clone(),
    };
    let r = v
        .restoring(Some(stamp.clone()))
        .revert_journal_entry(
            "whenubuy",
            "ab-testy",
            JournalRevert {
                entry: a.entry,
                status: Some((TopicStatus::Waiting, old.status)),
                summary: Some(("Nowy stan.".into(), old.summary.clone())),
            },
        )
        .unwrap();
    assert!(r.entry_removed && r.status_restored && r.summary_restored);
    assert_eq!(read(&v, TOPIC), before);

    // A task status in the topic file, taken back: byte-identical.
    let s = v.stamp_of_task("d4e5f6").unwrap().unwrap();
    v.update_task(
        "d4e5f6",
        TaskPatch {
            status: Some(TaskStatus::Done),
            ..Default::default()
        },
    )
    .unwrap();
    let stamp = Stamp {
        written: v.stamp_of_task("d4e5f6").unwrap().unwrap(),
        before: s,
    };
    v.restoring(Some(stamp))
        .update_task_if(
            "d4e5f6",
            |t| t.status == TaskStatus::Done,
            TaskPatch {
                status: Some(TaskStatus::Doing),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(read(&v, TOPIC), before);

    // A task added to the topic, taken back: byte-identical.
    let t = v
        .create_task(
            &loc(),
            NewTask {
                title: "Nowe".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let stamp = Stamp {
        written: v.stamp_of(&loc()).unwrap().unwrap(),
        before: old_stamp.clone(),
    };
    v.restoring(Some(stamp))
        .delete_task_if(t.id.as_deref().unwrap(), |_| true)
        .unwrap()
        .unwrap();
    assert_eq!(read(&v, TOPIC), before);
}

#[test]
fn the_old_zaktualizowano_comes_back_only_while_nothing_else_wrote_the_file() {
    let (dir, v) = setup();
    let old_stamp = v.stamp_of(&loc()).unwrap().unwrap();
    let a = added(&v, None, None);
    let written = v.stamp_of(&loc()).unwrap().unwrap();
    // Someone else writes the file a minute later (another entry).
    let later = Vault::open(dir.path())
        .unwrap()
        .with_clock(|| now() + chrono::TimeDelta::minutes(5));
    later
        .add_journal_entry(
            "whenubuy",
            "ab-testy",
            NewJournalEntry {
                author: "ja".into(),
                text: "Ręcznie.".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let bumped = v.stamp_of(&loc()).unwrap().unwrap();
    assert_ne!(bumped, written);
    let r = v
        .restoring(Some(Stamp {
            written,
            before: old_stamp.clone(),
        }))
        .revert_journal_entry(
            "whenubuy",
            "ab-testy",
            JournalRevert {
                entry: a.entry,
                status: None,
                summary: None,
            },
        )
        .unwrap();
    assert!(r.entry_removed);
    let now_stamp = v.stamp_of(&loc()).unwrap().unwrap();
    assert_ne!(
        now_stamp, old_stamp,
        "not put back over someone else's write"
    );
}
