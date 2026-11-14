//! Regression tests for the review findings (C1–C5, I1–I2, M3). Each test reproduces
//! the reported problem; see `docs/format.md` for the rules they pin down.

mod common;

use common::*;
use montybrain_core::*;

fn invalid(e: Error) -> String {
    match e {
        Error::InvalidInput(m) => m,
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

fn proj_loc(p: &str) -> TaskLocation {
    TaskLocation::Project { project: p.into() }
}

// ---------------------------------------------------------------------------
// C1: fences
// ---------------------------------------------------------------------------

#[test]
fn c1_nested_fence_example_is_not_a_task() {
    let (_d, v) = vault();
    let src = "---\nnazwa: P\n---\n## Zadania\n- [ ] Real ^real01\n\n````markdown\n```\n- [ ] example ^abc123\n## Not a section\n```\n````\n- [ ] After ^after1\n\n## Dziennik\n";
    write(&v, "p/projekt.md", src);
    let snap = v.load().unwrap();
    assert!(
        snap.task("abc123").is_none(),
        "task inside a 4-backtick fence"
    );
    assert!(snap.task("real01").is_some());
    assert!(snap.task("after1").is_some(), "task after the closed fence");
    assert!(matches!(
        v.delete_task("abc123"),
        Err(Error::TaskNotFound(_))
    ));
    assert_eq!(v.normalize().unwrap().ids_assigned, 0);
    assert_eq!(read(&v, "p/projekt.md"), src);
}

#[test]
fn c1_tilde_fence_ignores_backtick_lines_and_short_closers() {
    let (_d, v) = vault();
    let src = "## Zadania\n~~~~\n```\n- [ ] hidden one ^hid001\n~~~\n- [ ] hidden two ^hid002\n~~~~~\n- [ ] Visible ^vis001\n";
    write(&v, "p/projekt.md", src);
    let snap = v.load().unwrap();
    assert!(snap.task("hid001").is_none());
    assert!(snap.task("hid002").is_none(), "`~~~` cannot close `~~~~`");
    assert!(snap.task("vis001").is_some());
}

#[test]
fn c1_deeply_indented_fence_marker_does_not_hide_tasks() {
    let (_d, v) = vault();
    // An 8-space ``` line is not a fence (CommonMark: at most 3 spaces).
    write(
        &v,
        "p/projekt.md",
        "## Zadania\nSome text\n        ```\n- [ ] Visible ^vis001\n",
    );
    let snap = v.load().unwrap();
    assert!(snap.task("vis001").is_some());
}

#[test]
fn c1_fence_inside_task_notes_does_not_swallow_the_rest_of_the_file() {
    let (_d, v) = vault();
    let src = "# T\n\n## Zadania\n- [ ] A ^aaaaa1\n  ```\n- [ ] B ^bbbbb1\n\n## Dziennik\n- 2026-11-13 14:02 · ja · x\n";
    write(&v, "p/projekt.md", "---\nnazwa: P\n---\n");
    write(&v, "p/tematy/t.md", src);
    let snap = v.load().unwrap();
    assert!(snap.task("bbbbb1").is_some());
    assert_eq!(
        snap.journal_of("p", "t").len(),
        1,
        "## Dziennik still found"
    );
}

// ---------------------------------------------------------------------------
// C2: scalar continuation lines
// ---------------------------------------------------------------------------

#[test]
fn c2_archive_keeps_comment_and_continuation_after_scalar() {
    let (_d, v) = vault();
    let pf = "p/projekt.md";
    let src = "---\nnazwa: P\nstan: aktywny\n# keep comment\n  unknown: precious\nkolor: \"#fff\"   # c\n  more: stuff\n---\n## Zadania\n";
    write(&v, pf, src);
    let snap = v.load().unwrap();
    assert!(
        snap.warnings
            .iter()
            .any(|w| w.line == Some(3) && w.message.contains("stan")),
        "unsupported continuation is reported: {:?}",
        snap.warnings
    );
    v.archive_project("p").unwrap();
    assert_eq!(
        diff(src, &read(&v, pf)),
        vec!["-stan: aktywny".to_string(), "+stan: archiwum".to_string()]
    );
    // Clearing a key that has continuation lines empties the value but keeps them.
    v.update_project(
        "p",
        ProjectPatch {
            color: Some(None),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        read(&v, pf),
        "---\nnazwa: P\nstan: archiwum\n# keep comment\n  unknown: precious\nkolor:   # c\n  more: stuff\n---\n## Zadania\n"
    );
    assert_eq!(v.load().unwrap().project("p").unwrap().color, None);
}

/// Writing a value into an empty key that has a comment keeps the comment a comment
/// (it used to give `stan: archiwum# …`, which reads back as one plain value), and
/// clearing a key keeps its comment.
#[test]
fn c2_value_and_comment_stay_apart() {
    let (_d, v) = vault();
    let pf = "p/projekt.md";
    let src = "---\nnazwa: P\nkolor:   # brand colour\nstan: # aktywny | archiwum\n---\n";
    write(&v, pf, src);
    let p = v.archive_project("p").unwrap();
    assert_eq!(p.state, ProjectState::Archived);
    let p = v
        .update_project(
            "p",
            ProjectPatch {
                color: Some(Some("#a3e635".into())),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(p.color.as_deref(), Some("#a3e635"));
    assert_eq!(
        read(&v, pf),
        "---\nnazwa: P\nkolor: \"#a3e635\"   # brand colour\nstan: archiwum # aktywny | archiwum\n---\n"
    );
    v.update_project(
        "p",
        ProjectPatch {
            color: Some(None),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        read(&v, pf),
        "---\nnazwa: P\nkolor:   # brand colour\nstan: archiwum # aktywny | archiwum\n---\n"
    );
    let snap = v.load().unwrap();
    assert_eq!(snap.project("p").unwrap().color, None);
    assert!(snap.warnings.is_empty(), "{:?}", snap.warnings);
}

// ---------------------------------------------------------------------------
// C3: repository list comments
// ---------------------------------------------------------------------------

fn repo(n: &str, p: &str) -> Repo {
    Repo {
        name: n.into(),
        path: p.into(),
    }
}

#[test]
fn c3_adding_a_repo_keeps_list_comments() {
    let (_d, v) = vault();
    let pf = "p/projekt.md";
    let src = "---\nnazwa: P\nrepozytoria: # important\n  # keep this explanation\n  - nazwa: a\n    sciezka: /a\n  # between items\n  - nazwa: b\n    sciezka: /b\n  weird: unsupported\nstan: aktywny\n---\n## Zadania\n";
    write(&v, pf, src);
    v.update_project(
        "p",
        ProjectPatch {
            repos: Some(vec![repo("a", "/a"), repo("b", "/b"), repo("c", "/c")]),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        diff(src, &read(&v, pf)),
        vec!["+  - nazwa: c".to_string(), "+    sciezka: /c".to_string()]
    );
    // Removing and replacing keeps the comments too.
    v.update_project(
        "p",
        ProjectPatch {
            repos: Some(vec![repo("b", "/b2")]),
            ..Default::default()
        },
    )
    .unwrap();
    let after = read(&v, pf);
    for keep in [
        "repozytoria: # important\n",
        "  # keep this explanation\n",
        "  # between items\n",
        "  weird: unsupported\n",
    ] {
        assert!(after.contains(keep), "lost {keep:?}:\n{after}");
    }
    let p = v.load().unwrap().project("p").unwrap().clone();
    assert_eq!(p.repos, vec![repo("b", "/b2")]);
    // Emptying the list keeps the commented header.
    v.update_project(
        "p",
        ProjectPatch {
            repos: Some(vec![]),
            ..Default::default()
        },
    )
    .unwrap();
    let after = read(&v, pf);
    assert!(after.contains("repozytoria: # important\n  # keep this explanation\n"));
    assert!(v.load().unwrap().project("p").unwrap().repos.is_empty());
}

#[test]
fn c3_empty_flow_list_becomes_block_list_and_flow_items_are_refused() {
    let (_d, v) = vault();
    let pf = "p/projekt.md";
    write(&v, pf, "---\nrepozytoria: []   # none yet\n---\n");
    v.update_project(
        "p",
        ProjectPatch {
            repos: Some(vec![repo("a", "/a")]),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        read(&v, pf),
        "---\nrepozytoria:   # none yet\n  - nazwa: a\n    sciezka: /a\n---\n"
    );
    let src = "---\nrepozytoria: [{nazwa: a, sciezka: /a}]\n---\n";
    write(&v, pf, src);
    let err = v
        .update_project(
            "p",
            ProjectPatch {
                repos: Some(vec![repo("b", "/b")]),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(matches!(err, Error::Unparseable { .. }), "{err:?}");
    assert_eq!(read(&v, pf), src);
}

fn set_repos_of(v: &Vault, repos: Vec<Repo>) -> Result<Project> {
    v.update_project(
        "p",
        ProjectPatch {
            repos: Some(repos),
            ..Default::default()
        },
    )
}

/// R1: editing a repository edits its `nazwa`/`sciezka` lines in place and keeps
/// every other line of the item.
#[test]
fn c3_editing_a_repo_path_keeps_unknown_item_lines() {
    let (_d, v) = vault();
    let pf = "p/projekt.md";
    let src = "---\nnazwa: P\nrepozytoria:\n  - nazwa: main\n    sciezka: /old\n    branch: important\n    # keep me\n---\n## Zadania\n";
    write(&v, pf, src);
    let p = set_repos_of(&v, vec![repo("main", "/new")]).unwrap();
    assert_eq!(p.repos, vec![repo("main", "/new")]);
    assert_eq!(
        read(&v, pf),
        "---\nnazwa: P\nrepozytoria:\n  - nazwa: main\n    sciezka: /new\n    branch: important\n    # keep me\n---\n## Zadania\n"
    );
}

/// R2: a list-level comment inside an item does not end the item, so its later
/// fields can be edited.
#[test]
fn c3_comment_inside_an_item_does_not_split_it() {
    let (_d, v) = vault();
    let pf = "p/projekt.md";
    let src = "---\nrepozytoria:\n  - nazwa: main\n  # list comment\n    sciezka: /old\n---\n";
    write(&v, pf, src);
    assert_eq!(
        v.load().unwrap().project("p").unwrap().repos,
        vec![repo("main", "/old")]
    );
    let p = set_repos_of(&v, vec![repo("main", "/new")]).unwrap();
    assert_eq!(p.repos, vec![repo("main", "/new")]);
    assert_eq!(
        read(&v, pf),
        "---\nrepozytoria:\n  - nazwa: main\n  # list comment\n    sciezka: /new\n---\n"
    );
}

// ---------------------------------------------------------------------------
// C4: discovery problems are warnings
// ---------------------------------------------------------------------------

#[test]
fn c4_tematy_as_a_file_is_a_warning() {
    let (_d, v) = vault();
    write(
        &v,
        "p/projekt.md",
        "---\nnazwa: P\n---\n## Zadania\n- [ ] a ^ppppp1\n",
    );
    write(&v, "p/tematy", "not a directory");
    write(&v, "q/projekt.md", "---\nnazwa: Q\n---\n");
    let snap = v.load().unwrap();
    assert!(snap.project("p").is_some() && snap.project("q").is_some());
    assert!(snap.task("ppppp1").is_some());
    assert!(
        snap.warnings
            .iter()
            .any(|w| w.path.ends_with("p/tematy") && w.message.contains("not a directory")),
        "{:?}",
        snap.warnings
    );
    // Mutations (which discover files the same way) still work.
    v.create_task(
        &TaskLocation::Inbox,
        NewTask {
            title: "x".into(),
            ..Default::default()
        },
    )
    .unwrap();
    v.normalize().unwrap();
}

#[cfg(unix)]
#[test]
fn c4_unreadable_project_and_broken_symlink_are_warnings() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (_d, v) = vault();
    write(&v, "ok/projekt.md", "---\nnazwa: Ok\n---\n");
    write(&v, "locked/projekt.md", "---\nnazwa: Locked\n---\n");
    write(&v, "locked2/projekt.md", "---\nnazwa: Locked2\n---\n");
    write(&v, "locked2/tematy/t.md", "# T\n");
    std::fs::create_dir_all(v.root().join("ok/tematy")).unwrap();
    symlink(
        v.root().join("nowhere.md"),
        v.root().join("ok/tematy/broken.md"),
    )
    .unwrap();
    let set = |rel: &str, mode: u32| {
        std::fs::set_permissions(v.root().join(rel), std::fs::Permissions::from_mode(mode))
            .unwrap();
    };
    set("locked", 0o000);
    set("locked2/tematy", 0o000);
    let snap = v.load();
    set("locked", 0o755);
    set("locked2/tematy", 0o755);
    let snap = snap.expect("one unreadable project must not fail the load");
    assert!(snap.project("ok").is_some());
    let msgs: Vec<String> = snap
        .warnings
        .iter()
        .map(|w| format!("{}: {}", w.path.display(), w.message))
        .collect();
    let joined = msgs.join("\n");
    assert!(
        msgs.iter().any(|m| m.starts_with("locked:")),
        "unreadable project dir reported: {joined}"
    );
    assert!(
        msgs.iter().any(|m| m.starts_with("locked2/tematy:")),
        "unreadable tematy reported: {joined}"
    );
    assert!(
        msgs.iter()
            .any(|m| m.starts_with("ok/tematy/broken.md:") && m.contains("symbolic link")),
        "broken symlink reported: {joined}"
    );
}

// ---------------------------------------------------------------------------
// C5: line endings of moved blocks
// ---------------------------------------------------------------------------

const CRLF_PROJECT: &str = "---\r\nnazwa: C\r\n---\r\n## Zadania\r\n- [ ] Crlf ^ccccc1\r\n  - https://example.com/x\r\n  note\r\n";

#[test]
fn c5_moved_block_takes_destination_line_endings() {
    let (_d, v) = vault();
    write(&v, "c/projekt.md", CRLF_PROJECT);
    write(&v, "skrzynka.md", "- [ ] Lf ^lllll1\n");
    v.move_task("ccccc1", &TaskLocation::Inbox).unwrap();
    assert_eq!(
        read(&v, "skrzynka.md"),
        "- [ ] Lf ^lllll1\n- [ ] Crlf ^ccccc1\n  - https://example.com/x\n  note\n"
    );
    assert_eq!(
        read(&v, "c/projekt.md"),
        "---\r\nnazwa: C\r\n---\r\n## Zadania\r\n"
    );
    // LF block into a CRLF file becomes CRLF.
    v.move_task("lllll1", &proj_loc("c")).unwrap();
    let c = read(&v, "c/projekt.md");
    assert!(c.ends_with("## Zadania\r\n- [ ] Lf ^lllll1\r\n"), "{c:?}");
}

#[test]
fn c5_block_moved_into_a_file_without_line_endings_keeps_its_own() {
    let (_d, v) = vault();
    write(&v, "c/projekt.md", CRLF_PROJECT);
    write(&v, "skrzynka.md", "");
    v.move_task("ccccc1", &TaskLocation::Inbox).unwrap();
    assert_eq!(
        read(&v, "skrzynka.md"),
        "- [ ] Crlf ^ccccc1\r\n  - https://example.com/x\r\n  note\r\n"
    );
}

// ---------------------------------------------------------------------------
// Q3: a repeated frontmatter key. The first occurrence is authoritative; the
// duplicate is kept byte for byte, reported, and never takes over.
// ---------------------------------------------------------------------------

fn has_duplicate_warning(v: &Vault, line: usize, key: &str) -> bool {
    v.load()
        .unwrap()
        .warnings
        .iter()
        .any(|w| w.line == Some(line) && w.message.contains("duplicate") && w.message.contains(key))
}

#[test]
fn q3_duplicate_project_scalar_keys() {
    let (_d, v) = vault();
    let pf = "p/projekt.md";
    let src = "---\nnazwa: P\nkolor: \"#111111\"\nstan: aktywny\nkolor: \"#222222\"\nstan: wstrzymany\n---\n## Zadania\n";
    write(&v, pf, src);
    let snap = v.load().unwrap();
    let p = snap.project("p").unwrap();
    assert_eq!(
        (p.color.as_deref(), p.state),
        (Some("#111111"), ProjectState::Active)
    );
    assert!(has_duplicate_warning(&v, 5, "kolor"));
    assert!(has_duplicate_warning(&v, 6, "stan"));
    // Clearing the first `kolor` must not let the second one take over.
    let p = v
        .update_project(
            "p",
            ProjectPatch {
                color: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(p.color, None);
    assert_eq!(
        read(&v, pf),
        "---\nnazwa: P\nkolor:\nstan: aktywny\nkolor: \"#222222\"\nstan: wstrzymany\n---\n## Zadania\n"
    );
    assert_eq!(v.load().unwrap().project("p").unwrap().color, None);
    assert!(has_duplicate_warning(&v, 5, "kolor"));
    // Setting it again and changing the state edit the first occurrences.
    let p = v
        .update_project(
            "p",
            ProjectPatch {
                color: Some(Some("#333333".into())),
                state: Some(ProjectState::Archived),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        (p.color.as_deref(), p.state),
        (Some("#333333"), ProjectState::Archived)
    );
    assert_eq!(
        read(&v, pf),
        "---\nnazwa: P\nkolor: \"#333333\"\nstan: archiwum\nkolor: \"#222222\"\nstan: wstrzymany\n---\n## Zadania\n"
    );
    let p = v.load().unwrap().project("p").unwrap().clone();
    assert_eq!(
        (p.color.as_deref(), p.state),
        (Some("#333333"), ProjectState::Archived)
    );
}

#[test]
fn q3_duplicate_topic_scalar_keys() {
    let (_d, v) = vault();
    write(
        &v,
        "p/projekt.md",
        "---\nnazwa: P\nrepozytoria:\n  - nazwa: r1\n    sciezka: /1\n  - nazwa: r2\n    sciezka: /2\n---\n",
    );
    let tf = "p/tematy/t.md";
    let src = "---\nstatus: w-toku\nrepo: r1\nstatus: czeka\nrepo: r2\nzaktualizowano: 2026-11-01T10:00\n---\n# T\n";
    write(&v, tf, src);
    let snap = v.load().unwrap();
    let t = snap.topics.iter().find(|t| t.slug == "t").unwrap();
    assert_eq!(
        (t.status, t.repo.as_deref()),
        (TopicStatus::InProgress, Some("r1"))
    );
    assert!(has_duplicate_warning(&v, 4, "status"));
    assert!(has_duplicate_warning(&v, 5, "repo"));
    let t = v
        .update_topic(
            "p",
            "t",
            TopicPatch {
                repo: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(t.repo, None);
    let t = v
        .update_topic(
            "p",
            "t",
            TopicPatch {
                status: Some(TopicStatus::Done),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!((t.status, t.repo.as_deref()), (TopicStatus::Done, None));
    assert_eq!(
        read(&v, tf),
        "---\nstatus: zrobiony\nrepo:\nstatus: czeka\nrepo: r2\nzaktualizowano: 2026-11-14T09:15\n---\n# T\n"
    );
    let snap = v.load().unwrap();
    let t = snap.topics.iter().find(|t| t.slug == "t").unwrap();
    assert_eq!((t.status, t.repo.as_deref()), (TopicStatus::Done, None));
    let t = v
        .update_topic(
            "p",
            "t",
            TopicPatch {
                repo: Some(Some("r2".into())),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(t.repo.as_deref(), Some("r2"));
    assert!(read(&v, tf).starts_with("---\nstatus: zrobiony\nrepo: r2\nstatus: czeka\nrepo: r2\n"));
}

/// Q4: a tab separates a scalar value from its comment, for reading and writing.
#[test]
fn q4_tab_before_scalar_comment() {
    let (_d, v) = vault();
    let pf = "p/projekt.md";
    write(
        &v,
        pf,
        "---\nnazwa: P\nkolor: \"#111111\"\t# brand\nstan: aktywny\t# state\n---\n",
    );
    let snap = v.load().unwrap();
    assert!(snap.warnings.is_empty(), "{:?}", snap.warnings);
    let p = snap.project("p").unwrap();
    assert_eq!(
        (p.color.as_deref(), p.state),
        (Some("#111111"), ProjectState::Active)
    );
    v.update_project(
        "p",
        ProjectPatch {
            color: Some(None),
            state: Some(ProjectState::Paused),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        read(&v, pf),
        "---\nnazwa: P\nkolor:\t# brand\nstan: wstrzymany\t# state\n---\n"
    );
    let p = v
        .update_project(
            "p",
            ProjectPatch {
                color: Some(Some("#222222".into())),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(p.color.as_deref(), Some("#222222"));
    assert_eq!(
        read(&v, pf),
        "---\nnazwa: P\nkolor: \"#222222\"\t# brand\nstan: wstrzymany\t# state\n---\n"
    );
}

// ---------------------------------------------------------------------------
// I1 / I2: structure injection through free-text fields
// ---------------------------------------------------------------------------

fn topic_setup() -> (tempfile::TempDir, Vault) {
    let (d, v) = vault();
    write(&v, "whenubuy/projekt.md", PROJECT);
    write(&v, "whenubuy/tematy/ab-testy.md", &fixture("topic_spec.md"));
    (d, v)
}

fn task_count(v: &Vault) -> usize {
    v.load().unwrap().tasks.len()
}

#[test]
fn i1_link_with_newline_cannot_forge_a_task() {
    let (_d, v) = topic_setup();
    let before = task_count(&v);
    let inbox = read(&v, "skrzynka.md");
    let err = v
        .create_task(
            &TaskLocation::Inbox,
            NewTask {
                title: "Real".into(),
                links: vec!["https://example.com\n- [ ] Forged".into()],
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(invalid(err).contains("links"));
    assert_eq!(read(&v, "skrzynka.md"), inbox);
    let topic = read(&v, "whenubuy/tematy/ab-testy.md");
    let err = v
        .update_task(
            "g7h8i9",
            TaskPatch {
                add_links: vec!["https://example.com\n- [ ] Forged ^forged".into()],
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(invalid(err).contains("add_links"));
    assert_eq!(read(&v, "whenubuy/tematy/ab-testy.md"), topic);
    // Not a URL at all.
    let err = v
        .create_task(
            &TaskLocation::Inbox,
            NewTask {
                title: "Real".into(),
                links: vec!["see the MR".into()],
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(invalid(err).contains("links"));
    assert_eq!(task_count(&v), before);
}

#[test]
fn i2_summary_cannot_create_sections_or_tasks() {
    let (_d, v) = topic_setup();
    let forged = "Hello\n## Zadania\n- [ ] Summary forged";
    let err = v
        .create_topic(
            "whenubuy",
            NewTopic {
                title: "Evil".into(),
                summary: forged.into(),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(invalid(err).contains("summary"));
    assert!(!v.root().join("whenubuy/tematy/evil.md").exists());
    let topic = read(&v, "whenubuy/tematy/ab-testy.md");
    let err = v
        .add_journal_entry(
            "whenubuy",
            "ab-testy",
            NewJournalEntry {
                text: "ok".into(),
                summary: Some(forged.into()),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(invalid(err).contains("summary"));
    let err = v
        .update_topic(
            "whenubuy",
            "ab-testy",
            TopicPatch {
                summary: Some("x\n```\nswallow everything".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(invalid(err).contains("fence"));
    let err = v
        .update_topic(
            "whenubuy",
            "ab-testy",
            TopicPatch {
                summary: Some("# Another title".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(invalid(err).contains("heading"));
    assert_eq!(read(&v, "whenubuy/tematy/ab-testy.md"), topic);
    // Balanced fences, deeper headings and checklists are fine in a summary.
    let ok = "Stan:\n### Szczegóły\n```\n## not a section\n```\n- [ ] just a checklist item";
    let t = v
        .update_topic(
            "whenubuy",
            "ab-testy",
            TopicPatch {
                summary: Some(ok.into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(t.summary, ok);
    let snap = v.load().unwrap();
    assert_eq!(snap.journal_of("whenubuy", "ab-testy").len(), 2);
    assert_eq!(
        snap.tasks(&TaskFilter {
            topic: Some("ab-testy".into()),
            include_done: true,
            ..Default::default()
        })
        .len(),
        4
    );
}

#[test]
fn i2_description_cannot_create_frontmatter_title_or_sections() {
    let (_d, v) = topic_setup();
    for bad in [
        "Opis\n## Zadania\n- [ ] forged",
        "# Title",
        "---\nnazwa: Evil\n---",
        "```\nunclosed",
    ] {
        let err = v
            .create_project(NewProject {
                name: "X".into(),
                description: bad.into(),
                ..Default::default()
            })
            .unwrap_err();
        assert!(invalid(err).contains("description"), "{bad:?}");
    }
    assert!(!v.root().join("x").exists());
    // A project directory without projekt.md: the description would be line 1.
    std::fs::create_dir_all(v.root().join("bare/tematy")).unwrap();
    let err = v
        .update_project(
            "bare",
            ProjectPatch {
                description: Some("---\nnazwa: Evil\n---".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(invalid(err).contains("description"));
    assert!(!v.root().join("bare/projekt.md").exists());
}

#[test]
fn i2_notes_and_journal_text_stay_inside_their_block() {
    let (_d, v) = topic_setup();
    let before = v.load().unwrap();
    v.update_task(
        "g7h8i9",
        TaskPatch {
            notes: Some("```\n## Dziennik\n- [ ] not a task ^zzzzz9".into()),
            ..Default::default()
        },
    )
    .unwrap();
    v.add_journal_entry(
        "whenubuy",
        "ab-testy",
        NewJournalEntry {
            author: "agent\n## Zadania".into(),
            text: "first\n```\n## Zadania\n- [ ] forged ^zzzzz8\n- 2026-01-01 10:00 · x · forged entry"
                .into(),
            ..Default::default()
        },
    )
    .unwrap();
    let after = v.load().unwrap();
    assert_eq!(after.tasks.len(), before.tasks.len());
    assert!(after.task("zzzzz9").is_none() && after.task("zzzzz8").is_none());
    let j = after.journal_of("whenubuy", "ab-testy");
    assert_eq!(j.len(), 3, "exactly one new entry");
    assert_eq!(j[0].author, "agent ## Zadania");
    assert!(
        after.warnings.is_empty(),
        "no structural damage: {:?}",
        after.warnings
    );
    // The following topic structure is still intact: a new task lands in Zadania.
    let t = v
        .create_task(
            &TaskLocation::Topic {
                project: "whenubuy".into(),
                topic: "ab-testy".into(),
            },
            NewTask {
                title: "after".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(v.load().unwrap().task(t.id.as_deref().unwrap()).is_some());
}

#[test]
fn i2_single_line_fields_reject_control_characters_and_line_breaks() {
    let (_d, v) = topic_setup();
    let err = v
        .update_project(
            "whenubuy",
            ProjectPatch {
                repos: Some(vec![repo("evil\nstan: archiwum", "/x")]),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(invalid(err).contains("repos"));
    let err = v
        .update_project(
            "whenubuy",
            ProjectPatch {
                repos: Some(vec![repo("ok", "/x\n---")]),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(invalid(err).contains("repos"));
    let err = v
        .create_project(NewProject {
            name: "R".into(),
            repos: vec![repo("", "/x")],
            ..Default::default()
        })
        .unwrap_err();
    assert!(invalid(err).contains("repos"));
    let err = v
        .update_project(
            "whenubuy",
            ProjectPatch {
                color: Some(Some("#fff\nstan: archiwum".into())),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(invalid(err).contains("color"));
    let err = v
        .create_task(
            &TaskLocation::Inbox,
            NewTask {
                title: "bell\u{7}".into(),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(invalid(err).contains("title"));
    let err = v
        .update_task(
            "g7h8i9",
            TaskPatch {
                notes: Some("a\rb".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(invalid(err).contains("notes"));
    // Titles and names are still collapsed to one line (documented behaviour).
    let p = v
        .update_project(
            "whenubuy",
            ProjectPatch {
                name: Some("Name\n---\nstan: archiwum".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(p.name, "Name --- stan: archiwum");
    assert_eq!(p.state, ProjectState::Active);
    let snap = v.load().unwrap();
    assert!(snap.warnings.is_empty(), "{:?}", snap.warnings);
}

#[test]
fn i2_crlf_in_multiline_input_is_a_line_break() {
    let (_d, v) = topic_setup();
    let t = v
        .update_topic(
            "whenubuy",
            "ab-testy",
            TopicPatch {
                summary: Some("one\r\ntwo".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(t.summary, "one\ntwo");
}

#[test]
fn i2_region_text_is_stored_with_trimmed_line_ends() {
    let (_d, v) = topic_setup();
    // A Markdown hard break (two trailing spaces) is accepted; line ends are trimmed
    // when the region is written, as before.
    let p = v
        .update_project(
            "whenubuy",
            ProjectPatch {
                description: Some("Linia  \nDruga".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(p.description, "Linia\nDruga");
    let p = v
        .create_project(NewProject {
            name: "Nowy".into(),
            description: "  Opis  \n\n  wcięty  ".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(p.description, "Opis\n\n  wcięty");
}
