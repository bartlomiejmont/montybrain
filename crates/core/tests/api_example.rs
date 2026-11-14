//! The usage example from docs/core-api.md, run against a temp vault.

use montybrain_core::*;

fn assert_send_sync<T: Send + Sync + Clone>() {}

#[test]
fn docs_example_runs() -> Result<()> {
    assert_send_sync::<Vault>();
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(dir.path().join("MontyBrain"))?;
    vault.normalize()?;

    let p = vault.create_project(NewProject {
        name: "WhenUBuy".into(),
        color: Some("#a3e635".into()),
        repos: vec![Repo {
            name: "whenubuy-backend".into(),
            path: "/Users/x/whenubuy-backend".into(),
        }],
        ..Default::default()
    })?;
    let t = vault.create_topic(
        &p.slug,
        NewTopic {
            title: "A/B testy cen".into(),
            status: TopicStatus::InProgress,
            repo: Some("whenubuy-backend".into()),
            ..Default::default()
        },
    )?;
    let loc = TaskLocation::Topic {
        project: p.slug.clone(),
        topic: t.slug.clone(),
    };
    let task = vault.create_task(
        &loc,
        NewTask {
            title: "Open MR".into(),
            priority: Priority::High,
            due: Some("2026-11-20T14:00".parse().unwrap()),
            ..Default::default()
        },
    )?;
    let id = task.id.clone().unwrap();
    let done = vault.update_task(
        &id,
        TaskPatch {
            status: Some(TaskStatus::Done),
            add_links: vec!["https://gitlab.com/g/r/-/merge_requests/42".into()],
            ..Default::default()
        },
    )?;
    assert_eq!(done.done, Some(local_today()));
    let added = vault.add_journal_entry(
        &p.slug,
        &t.slug,
        NewJournalEntry {
            author: "agent".into(),
            text: "Tests green, MR opened.".into(),
            status: Some(TopicStatus::Waiting),
            ..Default::default()
        },
    )?;
    assert_eq!(added.topic.status, TopicStatus::Waiting);

    let snap = vault.load()?;
    assert!(snap.warnings.is_empty(), "{:?}", snap.warnings);
    assert!(snap.today(local_today()).is_empty());
    assert_eq!(snap.task(&id).unwrap().links.len(), 1);
    assert_eq!(snap.recent_journal(10).len(), 1);

    // JSON inputs as an MCP server / Tauri command would receive them.
    let patch: TaskPatch =
        serde_json::from_str(r#"{"status":"todo","due":null,"notes":"z JSON"}"#).unwrap();
    let t2 = vault.update_task(&format!("^{id}"), patch)?;
    assert_eq!(t2.done, None);
    assert_eq!(t2.due, None);
    assert_eq!(t2.notes, "z JSON");
    let loc: TaskLocation = serde_json::from_str(r#"{"kind":"inbox"}"#).unwrap();
    let moved = vault.move_task(&id, &loc)?;
    assert_eq!(moved.location, TaskLocation::Inbox);
    let err = serde_json::to_string(&vault.delete_task("nope00").unwrap_err()).unwrap();
    assert_eq!(err, "\"task not found: ^nope00\"");
    Ok(())
}
