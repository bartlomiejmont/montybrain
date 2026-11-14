//! Several independent `Vault` handles (as the app and the MCP server would have)
//! mutating the same files concurrently must not lose anything.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Barrier};
use std::thread;

use common::*;
use morwisko_core::*;

const THREADS: usize = 8;
const PER_THREAD: usize = 15;

#[test]
fn concurrent_mutations_lose_nothing() {
    let (_d, setup) = vault();
    write(&setup, "p/projekt.md", "---\nnazwa: P\n---\n## Zadania\n");
    setup
        .create_topic(
            "p",
            NewTopic {
                title: "Wspólny".into(),
                ..Default::default()
            },
        )
        .unwrap();
    // One pre-existing task per thread, toggled concurrently by its owner.
    let mut own = Vec::new();
    for i in 0..THREADS {
        let t = setup
            .create_task(
                &TaskLocation::Topic {
                    project: "p".into(),
                    topic: "wspolny".into(),
                },
                NewTask {
                    title: format!("own {i}"),
                    ..Default::default()
                },
            )
            .unwrap();
        own.push(t.id.unwrap());
    }

    let root = setup.root().to_path_buf();
    let barrier = Arc::new(Barrier::new(THREADS));
    let handles: Vec<_> = (0..THREADS)
        .map(|i| {
            let root = root.clone();
            let barrier = barrier.clone();
            let own_id = own[i].clone();
            thread::spawn(move || {
                // Separate handle per thread, like separate processes.
                let v = Vault::open(&root).unwrap();
                barrier.wait();
                let mut created = Vec::new();
                for k in 0..PER_THREAD {
                    let loc = match k % 3 {
                        0 => TaskLocation::Inbox,
                        1 => TaskLocation::Project {
                            project: "p".into(),
                        },
                        _ => TaskLocation::Topic {
                            project: "p".into(),
                            topic: "wspolny".into(),
                        },
                    };
                    let t = v
                        .create_task(
                            &loc,
                            NewTask {
                                title: format!("t{i}-{k}"),
                                ..Default::default()
                            },
                        )
                        .unwrap();
                    created.push(t.id.unwrap());
                    if k % 5 == 0 {
                        v.add_journal_entry(
                            "p",
                            "wspolny",
                            NewJournalEntry {
                                author: format!("thread-{i}"),
                                text: format!("entry {k}"),
                                ..Default::default()
                            },
                        )
                        .unwrap();
                    }
                }
                v.update_task(
                    &own_id,
                    TaskPatch {
                        status: Some(TaskStatus::Done),
                        notes: Some(format!("closed by {i}")),
                        ..Default::default()
                    },
                )
                .unwrap();
                // Move one of our tasks around, then delete another.
                v.move_task(
                    &created[0],
                    &TaskLocation::Project {
                        project: "p".into(),
                    },
                )
                .unwrap();
                v.delete_task(&created[1]).unwrap();
                created
            })
        })
        .collect();
    let created: Vec<Vec<String>> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    let snap = setup.load().unwrap();
    assert!(snap.warnings.is_empty(), "{:?}", snap.warnings);
    let ids: Vec<&str> = snap.tasks.iter().filter_map(|t| t.id.as_deref()).collect();
    let unique: HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(ids.len(), unique.len(), "ids must be unique");
    assert_eq!(snap.tasks.len(), THREADS + THREADS * (PER_THREAD - 1));
    for (i, list) in created.iter().enumerate() {
        let moved = snap.task(&list[0]).unwrap();
        assert_eq!(
            moved.location,
            TaskLocation::Project {
                project: "p".into()
            }
        );
        assert_eq!(moved.title, format!("t{i}-0"));
        assert!(snap.task(&list[1]).is_none());
        for (k, id) in list.iter().enumerate().skip(2) {
            assert_eq!(snap.task(id).unwrap().title, format!("t{i}-{k}"));
        }
        let o = snap.task(&own[i]).unwrap();
        assert_eq!(o.status, TaskStatus::Done);
        assert_eq!(o.notes, format!("closed by {i}"));
    }
    let journal = snap.journal_of("p", "wspolny");
    assert_eq!(journal.len(), THREADS * PER_THREAD.div_ceil(5));
    // No temp files left behind.
    for entry in walk(&root) {
        assert!(!entry.ends_with(TMP_SUFFIX), "{entry}");
    }
}

fn walk(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p.display().to_string());
        }
    }
    out
}
