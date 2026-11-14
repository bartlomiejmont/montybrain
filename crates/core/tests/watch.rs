#![cfg(feature = "watch")]

mod common;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::*;
use montybrain_core::*;

fn wait_for(rx: &mpsc::Receiver<Vec<PathBuf>>, pred: impl Fn(&PathBuf) -> bool) -> Vec<PathBuf> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        if let Ok(paths) = rx.recv_timeout(Duration::from_millis(200)) {
            let hit = paths.iter().any(&pred);
            seen.extend(paths);
            if hit {
                return seen;
            }
        }
    }
    panic!("no matching change event; saw {seen:?}");
}

#[test]
fn watcher_reports_changes_and_ignores_meta() {
    let (_d, v) = vault();
    let (tx, rx) = mpsc::channel();
    let _w = v
        .watch(Duration::from_millis(100), move |paths| {
            let _ = tx.send(paths);
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let t = v
        .create_task(
            &TaskLocation::Inbox,
            NewTask {
                title: "watched".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let seen = wait_for(&rx, |p| p == &PathBuf::from(INBOX_FILE));
    assert!(t.id.is_some());
    for p in &seen {
        assert!(!is_ignored(p), "ignored path reported: {p:?}");
        assert!(!p.starts_with(META_DIR), "{p:?}");
    }
    // A hand edit in a new project folder is reported too.
    write(&v, "nowy/projekt.md", "---\nnazwa: Nowy\n---\n");
    wait_for(&rx, |p| p.starts_with("nowy"));
}
