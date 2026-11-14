//! The session manager against a fake CLI (a shell script speaking the
//! Claude Code stream-json protocol as described in `claude.rs`).

use super::*;
use std::os::unix::fs::PermissionsExt;

#[derive(Default)]
struct TestSink {
    views: Mutex<Vec<SessionView>>,
    answers: Mutex<Vec<String>>,
}

impl SessionSink for TestSink {
    fn changed(&self, v: &SessionView) {
        self.views.lock().unwrap().push(v.clone());
    }
    fn answer(&self, _v: &SessionView, text: &str) -> Result<usize, String> {
        self.answers.lock().unwrap().push(text.to_string());
        Ok(2)
    }
}

const FAKE: &str = r##"#!/bin/bash
read -r init
read -r user
echo "$user" > "$OUT/user"
echo '{"type":"system","subtype":"init","model":"fake","mcp_servers":[]}'
echo '{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"Zaczynam, token sk-ant-abcdefghijklmnopqrstuvwxyz0123456789"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"osascript -e x"}}]}}'
echo '{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"osascript -e x"},"tool_use_id":"t1"}}'
echo '{"type":"control_request","request_id":"r2","request":{"subtype":"can_use_tool","tool_name":"mcp__montybrain__create_task","input":{"title":"x"},"tool_use_id":"t2"}}'
echo '{"type":"control_request","request_id":"r3","request":{"subtype":"can_use_tool","tool_name":"Write","input":{"file_path":"VAULT/skrzynka.md","content":"x"},"tool_use_id":"t3"}}'
read -r a1; echo "$a1" > "$OUT/a1"
read -r a2; echo "$a2" > "$OUT/a2"
read -r a3; echo "$a3" > "$OUT/a3"
echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"wynik","is_error":false}]}}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"Gotowe\n```montybrain-zmiany\n{\"changes\":[]}\n```"}'
read -r follow; echo "$follow" > "$OUT/follow"
echo '{"type":"control_request","request_id":"r4","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"osascript -e y"},"tool_use_id":"t4"}}'
read -r a4; echo "$a4" > "$OUT/a4"
echo '{"type":"result","subtype":"success","is_error":false,"result":"ok"}'
sleep 30
"##;

fn wait_for(s: &Sessions, what: &str, f: impl Fn(&SessionView) -> bool) -> SessionView {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(v) = s.view()
            && f(&v)
        {
            return v;
        }
        assert!(Instant::now() < deadline, "{what}: {:#?}", s.view());
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn read(dir: &Path, name: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(t) = std::fs::read_to_string(dir.join(name))
            && !t.trim().is_empty()
        {
            return serde_json::from_str(t.trim()).unwrap();
        }
        assert!(Instant::now() < deadline, "{name} never written");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn pending(v: &SessionView) -> Vec<u64> {
    v.entries
        .iter()
        .filter_map(|e| match e {
            Entry::Permission {
                id,
                state: PermState::Pending,
                ..
            } => Some(*id),
            _ => None,
        })
        .collect()
}

#[test]
fn a_session_round_trip_with_cards_denials_follow_ups_and_stop() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    let vault = tmp.path().join("vault");
    let bin = tmp.path().join("claude");
    std::fs::write(&bin, FAKE.replace("VAULT", vault.to_str().unwrap())).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let sink = Arc::new(TestSink::default());
    let s = Sessions::new(sink.clone());
    let launch = Launch {
        provider: ProviderId::Claude,
        scope: Scope::Inbox,
        request: "Sprawdź Slacka".into(),
        prompt: "PROMPT".into(),
        binary: bin,
        args: vec![],
        cwd: tmp.path().to_path_buf(),
        env: vec![
            ("OUT".into(), out.clone().into_os_string()),
            ("PATH".into(), "/usr/bin:/bin".into()),
        ],
        vault: vault.clone(),
        home: PathBuf::from("/nonexistent-home"),
        id: "s1".into(),
    };
    let v = s.start(launch.clone()).unwrap();
    assert_eq!(v.entries.len(), 1);
    // Only one at a time.
    assert!(s.start(launch).unwrap_err().contains("już trwa"));
    assert_eq!(
        read(&out, "user")["message"]["content"][0]["text"],
        "PROMPT"
    );

    // The Bash call is a card; the MontyBrain write and the vault write never are.
    let v = wait_for(&s, "cards", |v| {
        v.status == Status::NeedsYou
            && v.entries
                .iter()
                .filter(|e| matches!(e, Entry::Permission { .. }))
                .count()
                == 3
    });
    let denied: Vec<&Entry> = v
        .entries
        .iter()
        .filter(|e| {
            matches!(
                e,
                Entry::Permission {
                    state: PermState::AutoDenied,
                    ..
                }
            )
        })
        .collect();
    assert_eq!(denied.len(), 2, "{v:#?}");
    let a1 = read(&out, "a1");
    assert_eq!(a1["response"]["request_id"], "r2");
    assert_eq!(a1["response"]["response"]["behavior"], "deny");
    assert_eq!(read(&out, "a2")["response"]["request_id"], "r3");
    // A denied one cannot be approved from the window.
    let Entry::Permission { id: denied_id, .. } = denied[0] else {
        panic!()
    };
    assert!(s.answer(*denied_id, Answer::Allow, vec![]).is_err());
    // Secrets do not reach the view.
    assert!(!format!("{v:?}").contains("sk-ant-abcdefghij"), "{v:#?}");

    let card = pending(&v)[0];
    s.answer(card, Answer::AllowSession, vec![]).unwrap();
    let a3 = read(&out, "a3");
    assert_eq!(a3["response"]["request_id"], "r1");
    assert_eq!(a3["response"]["response"]["behavior"], "allow");
    assert!(
        s.answer(card, Answer::Allow, vec![]).is_err(),
        "answered twice"
    );

    // The answer with the block becomes a proposal.
    let v = wait_for(&s, "proposal", |v| {
        v.proposal.is_some() && v.status == Status::Idle
    });
    assert_eq!(v.proposal.as_ref().unwrap().count, 2);
    assert!(sink.answers.lock().unwrap()[0].contains("montybrain-zmiany"));
    assert!(v.entries.iter().any(
        |e| matches!(e, Entry::Tool { status: ToolStatus::Done, output, .. } if output == "wynik")
    ));

    // A follow-up, and the remembered "na tę sesję" answers the next one alone.
    s.send("A teraz kanał #ogolny").unwrap();
    assert_eq!(
        read(&out, "follow")["message"]["content"][0]["text"],
        "A teraz kanał #ogolny"
    );
    assert_eq!(
        read(&out, "a4")["response"]["response"]["behavior"],
        "allow"
    );
    let v = wait_for(&s, "second turn", |v| {
        v.status == Status::Idle
            && v.entries.iter().any(|e| {
                matches!(
                    e,
                    Entry::Permission {
                        state: PermState::AutoAllowed,
                        ..
                    }
                )
            })
    });
    assert_eq!(v.remembered, ["Bash:osascript"]);

    s.stop().unwrap();
    let v = s.view().unwrap();
    assert_eq!(v.status, Status::Stopped);
    assert!(s.send("x").is_err());
    s.close().unwrap();
    assert!(s.view().is_none());
}

#[test]
fn a_process_that_dies_is_a_failed_session_with_its_error() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("claude");
    std::fs::write(
        &bin,
        "#!/bin/bash\necho 'auth: nie zalogowano' >&2\nexit 3\n",
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let sink = Arc::new(TestSink::default());
    let s = Sessions::new(sink);
    s.start(Launch {
        provider: ProviderId::Codex,
        scope: Scope::Project {
            project: "p".into(),
        },
        request: "x".into(),
        prompt: "x".into(),
        binary: bin,
        args: vec![],
        cwd: tmp.path().to_path_buf(),
        env: vec![("PATH".into(), "/usr/bin:/bin".into())],
        vault: tmp.path().join("v"),
        home: tmp.path().to_path_buf(),
        id: "s2".into(),
    })
    .unwrap();
    let v = wait_for(&s, "failed", |v| {
        v.status == Status::Failed
            && v.error
                .as_deref()
                .is_some_and(|e| e.contains("nie zalogowano"))
    });
    assert!(v.ended_at.is_some());
}
