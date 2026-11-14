//! End-to-end tests: spawn the real `montybrain-mcp` binary on a temp vault and speak
//! JSON-RPC over stdio.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use montybrain_core::*;
use serde_json::{Value, json};
use tempfile::TempDir;

struct Client {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    next_id: u64,
    root: PathBuf,
    _dir: TempDir,
}

struct Reply {
    text: String,
    is_error: bool,
}

impl Client {
    fn start(seed: impl FnOnce(&Path)) -> Client {
        Client::start_with_env(seed, &[])
    }

    fn start_with_env(seed: impl FnOnce(&Path), env: &[(&str, &str)]) -> Client {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        std::fs::create_dir_all(&root).unwrap();
        seed(&root);
        Client::spawn(dir, root, env)
    }

    /// Starts the server on `root` (which may be missing) inside `dir`.
    fn spawn(dir: TempDir, root: PathBuf, env: &[(&str, &str)]) -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_montybrain-mcp"))
            .env("MONTYBRAIN_DIR", &root)
            .env_remove("MONTYBRAIN_READONLY")
            .envs(env.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let v: Value = serde_json::from_str(&line).unwrap_or_else(|e| {
                    panic!("stdout must carry only JSON-RPC, got {line:?}: {e}")
                });
                if tx.send(v).is_err() {
                    break;
                }
            }
        });
        let mut c = Client {
            child,
            stdin,
            rx,
            next_id: 0,
            root,
            _dir: dir,
        };
        let init = c.request(
            "initialize",
            json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "montybrain");
        c.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        c
    }
}

impl Client {
    fn send(&mut self, v: Value) {
        writeln!(self.stdin, "{v}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        loop {
            let v = self
                .rx
                .recv_timeout(Duration::from_secs(30))
                .expect("server reply");
            if v.get("id") == Some(&json!(id)) {
                return v;
            }
        }
    }

    fn call(&mut self, name: &str, args: Value) -> Reply {
        let v = self.request("tools/call", json!({"name":name,"arguments":args}));
        assert!(v.get("error").is_none(), "protocol error for {name}: {v}");
        let r = &v["result"];
        Reply {
            text: r["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            is_error: r["isError"].as_bool().unwrap_or(false),
        }
    }

    fn ok(&mut self, name: &str, args: Value) -> String {
        let r = self.call(name, args);
        assert!(!r.is_error, "{name} failed: {}", r.text);
        r.text
    }

    fn err(&mut self, name: &str, args: Value) -> String {
        let r = self.call(name, args);
        assert!(r.is_error, "{name} should have failed, got: {}", r.text);
        r.text
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn seed(root: &Path) {
    let v = Vault::init(root).unwrap();
    let p = v
        .create_project(NewProject {
            name: "WhenUBuy".into(),
            repos: vec![Repo {
                name: "whenubuy-backend".into(),
                path: "/x/whenubuy-backend".into(),
            }],
            ..Default::default()
        })
        .unwrap();
    v.create_topic(
        &p.slug,
        NewTopic {
            title: "Śledzenie błędów".into(),
            status: TopicStatus::InProgress,
            summary: "Integracja z Sentry.\nDruga linia.".into(),
            repo: Some("whenubuy-backend".into()),
        },
    )
    .unwrap();
    v.create_topic(
        &p.slug,
        NewTopic {
            title: "Testy A/B".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let d = v
        .create_project(NewProject {
            name: "Dom".into(),
            ..Default::default()
        })
        .unwrap();
    v.create_topic(
        &d.slug,
        NewTopic {
            title: "Remont".into(),
            ..Default::default()
        },
    )
    .unwrap();
    // A hand-written task without an id, in the inbox.
    let inbox = root.join("skrzynka.md");
    let mut s = std::fs::read_to_string(&inbox).unwrap();
    if !s.ends_with('\n') && !s.is_empty() {
        s.push('\n');
    }
    s.push_str("- [ ] Ręcznie dopisane zadanie\n");
    std::fs::write(inbox, s).unwrap();
}

fn id_of(text: &str) -> String {
    let i = text.find(" ^").expect("task id in output") + 2;
    text[i..i + 6].to_string()
}

#[test]
fn handshake_and_tool_list() {
    let mut c = Client::start(seed);
    let init = c.request(
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}),
    );
    let instr = init["result"]["instructions"].as_str().unwrap();
    assert!(instr.contains("overview") && instr.contains("log_update"));

    let list = c.request("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().unwrap();
    let mut names: Vec<_> = tools
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "create_task",
            "create_topic",
            "get_topic",
            "list_tasks",
            "log_update",
            "overview",
            "search",
            "update_task"
        ]
    );
    for t in tools {
        let name = t["name"].as_str().unwrap();
        assert!(
            t["description"].as_str().unwrap().len() > 40,
            "{name} description"
        );
        let schema = &t["inputSchema"];
        assert_eq!(schema["type"], "object", "{name}");
        let props = schema["properties"].as_object().unwrap();
        assert!(!props.is_empty(), "{name} has properties");
        for (k, p) in props {
            assert!(
                p["description"].is_string(),
                "{name}.{k} lacks a description: {p}"
            );
        }
    }
    let update = tools.iter().find(|t| t["name"] == "update_task").unwrap();
    assert_eq!(update["inputSchema"]["required"], json!(["id"]));
}

#[test]
fn full_flow() {
    let mut c = Client::start(seed);
    let today = local_today();

    // --- overview: hand-written task got an id (normalize ran); counts and topics shown.
    let o = c.ok("overview", json!({}));
    assert!(o.contains(&today.format("%Y-%m-%d").to_string()), "{o}");
    assert!(o.contains("## Inbox\n1 open task"), "{o}");
    assert!(
        o.contains("Śledzenie błędów") && o.contains("Integracja z Sentry."),
        "{o}"
    );
    assert!(
        !o.contains("Druga linia"),
        "only the first summary line: {o}"
    );
    assert!(
        c.read("skrzynka.md").contains(" ^"),
        "normalize assigned an id on disk"
    );

    // --- create_task: inbox, name-based project/topic (diacritics-insensitive), Polish words.
    let inbox = c.ok(
        "create_task",
        json!({"title":"Zadzwonić do klienta","priority":"wysoki","due":"2020-01-01"}),
    );
    assert!(
        inbox.contains("- [ ] Zadzwonić do klienta !high @2020-01-01") && inbox.contains("· inbox"),
        "{inbox}"
    );
    assert!(
        c.read("skrzynka.md")
            .contains("Zadzwonić do klienta !wysoki @2020-01-01")
    );

    let t1 = c.ok(
        "create_task",
        json!({"title":"Podpiąć Sentry","project":"whenubuy","topic":"sledzenie BLEDOW","status":"w-toku",
               "links":["https://gitlab.com/g/r/-/merge_requests/1"],"notes":"Zacząć od backendu"}),
    );
    assert!(
        t1.contains("- [/] Podpiąć Sentry") && t1.contains("· whenubuy / sledzenie-bledow"),
        "{t1}"
    );
    assert!(
        t1.contains("link: https://gitlab.com/g/r/-/merge_requests/1")
            && t1.contains("> Zacząć od backendu"),
        "{t1}"
    );
    let id1 = id_of(&t1);
    let topic_file = "whenubuy/tematy/sledzenie-bledow.md";
    assert!(c.read(topic_file).contains("- [/] Podpiąć Sentry"));

    let t2 = c.ok(
        "create_task",
        json!({"title":"Projektowy","project":"WhenUBuy","status":"blocked"}),
    );
    assert!(
        t2.contains("- [!] Projektowy") && t2.contains("· whenubuy\n"),
        "{t2}"
    );
    assert!(c.read("whenubuy/projekt.md").contains("- [!] Projektowy"));
    let id2 = id_of(&t2);

    // --- errors: unknown project lists the choices; topic without project; bad status; bad date.
    let e = c.err("create_task", json!({"title":"x","project":"nie-ma"}));
    assert!(
        e.contains("Unknown project 'nie-ma'")
            && e.contains("whenubuy (WhenUBuy)")
            && e.contains("dom (Dom)"),
        "{e}"
    );
    let e = c.err("get_topic", json!({"project":"dom","topic":"nope"}));
    assert!(e.contains("Valid topics: remont (Remont)"), "{e}");
    let e = c.err("create_task", json!({"title":"x","topic":"remont"}));
    assert!(e.contains("needs `project`"), "{e}");
    let e = c.err("create_task", json!({"title":"x","status":"kaput"}));
    assert!(e.contains("Unknown task status"), "{e}");
    let e = c.err("create_task", json!({"title":"x","due":"jutro"}));
    assert!(e.contains("Invalid date"), "{e}");
    let e = c.err("update_task", json!({"id":"zzzzzz","status":"done"}));
    assert!(e.contains("No task with id 'zzzzzz'"), "{e}");
    let e = c.err(
        "update_task",
        json!({"id":id1,"due":"2026-01-01","clear_due":true}),
    );
    assert!(e.contains("not both"), "{e}");

    // --- overview today block and project detail.
    let o = c.ok("overview", json!({}));
    assert!(o.contains("Overdue:\n- [ ] Zadzwonić do klienta"), "{o}");
    assert!(o.contains("In progress:\n- [/] Podpiąć Sentry"), "{o}");
    assert!(o.contains("Blocked:\n- [!] Projektowy"), "{o}");
    let od = c.ok("overview", json!({"project":"Dom"}));
    assert!(od.contains("Remont") && !od.contains("Sentry"), "{od}");
    let od = c.ok("overview", json!({"project":"whenubuy"}));
    assert!(od.contains("Integracja z Sentry.\nDruga linia."), "{od}");

    // --- update_task: English status, add link, clear due, id with ^.
    let u = c.ok("update_task", json!({"id":format!("^{id1}"),"status":"done","add_links":["https://example.com/thread/9"],"priority":"low"}));
    assert!(
        u.contains("- [x] Podpiąć Sentry !low") && u.contains(&format!("✓{today}")),
        "{u}"
    );
    assert!(u.contains("link: https://example.com/thread/9"), "{u}");
    let disk = c.read(topic_file);
    assert!(
        disk.contains("- [x] Podpiąć Sentry") && disk.contains("https://example.com/thread/9"),
        "{disk}"
    );

    // --- update_task: set then clear a due date.
    let u = c.ok(
        "update_task",
        json!({"id":id2,"due":"2026-12-24T18:30","status":"doing"}),
    );
    assert!(u.contains("@2026-12-24T18:30"), "{u}");
    assert!(c.read("whenubuy/projekt.md").contains("@2026-12-24T18:30"));
    let u = c.ok("update_task", json!({"id":id2,"clear_due":true}));
    assert!(!u.contains("@2026"), "{u}");
    assert!(!c.read("whenubuy/projekt.md").contains("@2026-12-24"));

    // --- move: project-level task into a topic (topic only, project inferred), then to the inbox.
    let m = c.ok(
        "update_task",
        json!({"id":id2,"move_to":{"topic":"Testy A/B"}}),
    );
    assert!(
        m.contains("Moved from whenubuy to whenubuy / testy-a-b")
            && m.contains("· whenubuy / testy-a-b"),
        "{m}"
    );
    assert!(
        c.read("whenubuy/tematy/testy-a-b.md")
            .contains("Projektowy")
    );
    assert!(!c.read("whenubuy/projekt.md").contains("Projektowy"));
    let m = c.ok("update_task", json!({"id":id2,"move_to":{"inbox":true}}));
    assert!(m.contains("· inbox"), "{m}");
    assert!(c.read("skrzynka.md").contains("Projektowy"));
    let e = c.err(
        "update_task",
        json!({"id":id2,"move_to":{"topic":"remont"}}),
    );
    assert!(e.contains("inbox"), "{e}");
    let m = c.ok(
        "update_task",
        json!({"id":id2,"move_to":{"project":"dom","topic":"remont"},"title":"Zamówić płytki"}),
    );
    assert!(
        m.contains("Zamówić płytki") && m.contains("· dom / remont"),
        "{m}"
    );
    assert!(c.read("dom/tematy/remont.md").contains("Zamówić płytki"));

    // --- list_tasks.
    let l = c.ok("list_tasks", json!({"inbox_only":true}));
    assert!(
        l.contains("Zadzwonić do klienta")
            && l.contains("Ręcznie dopisane")
            && !l.contains("płytki"),
        "{l}"
    );
    let l = c.ok("list_tasks", json!({"project":"WhenUBuy"}));
    assert!(!l.contains("Podpiąć Sentry"), "done hidden by default: {l}");
    let l = c.ok(
        "list_tasks",
        json!({"project":"WhenUBuy","include_done":true}),
    );
    assert!(l.contains("Podpiąć Sentry"), "{l}");
    let l = c.ok("list_tasks", json!({"status":["done"]}));
    assert!(
        l.starts_with("1 task(s)") && l.contains("Podpiąć Sentry"),
        "{l}"
    );
    let l = c.ok(
        "list_tasks",
        json!({"topic":"remont","status":["w-toku","blocked"]}),
    );
    assert!(l.starts_with("1 task(s)") && l.contains("płytki"), "{l}");
    let l = c.ok(
        "list_tasks",
        json!({"due_from":"2019-01-01","due_to":"2020-12-31"}),
    );
    assert!(
        l.starts_with("1 task(s)") && l.contains("Zadzwonić do klienta"),
        "{l}"
    );

    // --- create_topic.
    let ct = c.ok("create_topic", json!({"project":"WhenUBuy","title":"Migracja bazy","status":"czeka","repo":"WHENUBUY-backend","summary":"Czekamy na DBA."}));
    assert!(
        ct.contains("whenubuy/migracja-bazy")
            && ct.contains("waiting")
            && ct.contains("repo whenubuy-backend"),
        "{ct}"
    );
    let f = c.read("whenubuy/tematy/migracja-bazy.md");
    assert!(
        f.contains("status: czeka")
            && f.contains("repo: whenubuy-backend")
            && f.contains("Czekamy na DBA."),
        "{f}"
    );
    let e = c.err(
        "create_topic",
        json!({"project":"whenubuy","title":"Z","repo":"nope"}),
    );
    assert!(e.contains("Valid repos: whenubuy-backend"), "{e}");

    // --- log_update: default author, status + summary in one write.
    let lu = c.ok(
        "log_update",
        json!({"project":"WhenUBuy","topic":"migracja bazy","text":"DBA potwierdził okno migracji.","status":"w-toku","summary":"Okno migracji w piątek."}),
    );
    assert!(
        lu.contains("· agent ·")
            && lu.contains("DBA potwierdził okno migracji.")
            && lu.contains("in_progress"),
        "{lu}"
    );
    let f = c.read("whenubuy/tematy/migracja-bazy.md");
    assert!(
        f.contains("status: w-toku")
            && f.contains("Okno migracji w piątek.")
            && f.contains("· agent · DBA potwierdził okno migracji."),
        "{f}"
    );
    let e = c.err(
        "log_update",
        json!({"project":"whenubuy","topic":"migracja bazy","text":"  "}),
    );
    assert!(
        e.to_lowercase().contains("text")
            || e.to_lowercase().contains("empty")
            || e.contains("journal"),
        "{e}"
    );

    // --- get_topic.
    let g = c.ok(
        "get_topic",
        json!({"project":"whenubuy","topic":"Śledzenie błędów","include_done":true}),
    );
    assert!(
        g.contains("Status: in_progress")
            && g.contains("Repo: whenubuy-backend")
            && g.contains("Repo path: /x/whenubuy-backend"),
        "{g}"
    );
    assert!(
        g.contains("- [x] Podpiąć Sentry") && g.contains("Druga linia."),
        "{g}"
    );
    let g2 = c.ok(
        "get_topic",
        json!({"project":"whenubuy","topic":"sledzenie-bledow"}),
    );
    assert!(
        !g2.contains("Podpiąć Sentry") && g2.contains("1 done hidden"),
        "{g2}"
    );
    let g3 = c.ok(
        "get_topic",
        json!({"project":"whenubuy","topic":"migracja-bazy"}),
    );
    assert!(g3.contains("DBA potwierdził okno migracji."), "{g3}");

    // --- search (diacritics-insensitive; tasks, topics, journal).
    let s = c.ok("search", json!({"query":"migracji"}));
    assert!(s.contains("## Topics") && s.contains("## Journal"), "{s}");
    let s = c.ok("search", json!({"query":"plytki"}));
    assert!(
        s.contains("## Tasks") && s.contains("Zamówić płytki"),
        "{s}"
    );
    let s = c.ok("search", json!({"query":"nic-takiego-nie-ma"}));
    assert!(s.starts_with("0 hit(s)"), "{s}");
    c.err("search", json!({"query":" "}));
}

#[test]
fn survives_garbage_arguments() {
    let mut c = Client::start(seed);
    // Wrong types are protocol-level invalid params, but the server must stay alive.
    let v = c.request(
        "tools/call",
        json!({"name":"overview","arguments":{"project":42}}),
    );
    assert!(
        v.get("error").is_some() || v["result"]["isError"] == true,
        "{v}"
    );
    let v = c.request("tools/call", json!({"name":"nope","arguments":{}}));
    assert!(
        v.get("error").is_some() || v["result"]["isError"] == true,
        "{v}"
    );
    let o = c.ok("overview", json!({}));
    assert!(o.contains("# MontyBrain overview"));
}

// ---------------------------------------------------------------------------
// Review regressions
// ---------------------------------------------------------------------------

#[test]
fn structure_injection_is_refused() {
    let mut c = Client::start(seed);
    c.ok("overview", json!({})); // normalize assigns the hand-written task's id
    let inbox = c.read("skrzynka.md");
    let e = c.err(
        "create_task",
        json!({"title":"Real","links":["https://example.com\n- [ ] Forged"]}),
    );
    assert!(
        e.contains("links[0]") && e.contains("not a bare URL"),
        "{e}"
    );
    assert_eq!(c.read("skrzynka.md"), inbox, "nothing written");
    let id = id_of(&c.ok("list_tasks", json!({"inbox_only":true})));
    let e = c.err(
        "update_task",
        json!({"id":id,"add_links":["https://example.com\n- [ ] Forged"]}),
    );
    assert!(e.contains("add_links[0]"), "{e}");
    assert_eq!(c.read("skrzynka.md"), inbox);

    let forged = "Hello\n## Zadania\n- [ ] Summary forged";
    let e = c.err(
        "create_topic",
        json!({"project":"whenubuy","title":"Evil","summary":forged}),
    );
    assert!(e.contains("summary") && e.contains("heading"), "{e}");
    assert!(!c.root.join("whenubuy/tematy/evil.md").exists());
    let topic = c.read("whenubuy/tematy/testy-a-b.md");
    let e = c.err(
        "log_update",
        json!({"project":"whenubuy","topic":"testy-a-b","text":"ok","summary":forged}),
    );
    assert!(e.contains("summary"), "{e}");
    assert_eq!(c.read("whenubuy/tematy/testy-a-b.md"), topic);
    let l = c.ok("list_tasks", json!({}));
    assert!(!l.contains("Forged") && !l.contains("forged"), "{l}");
}

#[test]
fn update_with_failing_move_changes_nothing() {
    let mut c = Client::start(|root| {
        seed(root);
        std::fs::create_dir_all(root.join("zepsuty")).unwrap();
        std::fs::write(root.join("zepsuty/projekt.md"), [0xff, 0xfe, b'\n']).unwrap();
    });
    let t = c.ok(
        "create_task",
        json!({"title":"Do przeniesienia","project":"whenubuy"}),
    );
    let id = id_of(&t);
    let before = c.read("whenubuy/projekt.md");
    let e = c.err(
        "update_task",
        json!({"id":id,"title":"Nowy tytuł","status":"done","move_to":{"project":"zepsuty"}}),
    );
    assert!(e.contains("Nothing was changed"), "{e}");
    assert_eq!(c.read("whenubuy/projekt.md"), before, "edit not applied");
    let ok = c.ok(
        "update_task",
        json!({"id":id,"title":"Nowy tytuł","move_to":{"project":"dom"}}),
    );
    assert!(
        ok.contains("Updated and moved") && ok.contains("Nowy tytuł") && ok.contains("· dom\n"),
        "{ok}"
    );
    assert!(!c.read("whenubuy/projekt.md").contains("przeniesienia"));
}

#[test]
fn exact_slug_wins_over_display_names() {
    let mut c = Client::start(|root| {
        let v = Vault::init(root).unwrap();
        // Project `a` is named "A", project `b` is named "a".
        for (slug, name) in [("a", "A"), ("b", "a")] {
            std::fs::create_dir_all(root.join(slug).join("tematy")).unwrap();
            std::fs::write(
                root.join(slug).join("projekt.md"),
                format!("---\nnazwa: {name}\n---\n## Zadania\n"),
            )
            .unwrap();
        }
        // In `a`: topic `x` titled "X" and topic `y` titled "x".
        for (slug, title) in [("x", "X"), ("y", "x")] {
            std::fs::write(
                root.join("a/tematy").join(format!("{slug}.md")),
                format!("# {title}\n\n## Zadania\n\n## Dziennik\n"),
            )
            .unwrap();
        }
        drop(v);
    });
    let t = c.ok("create_task", json!({"title":"Do a","project":"a"}));
    assert!(t.contains("· a\n"), "{t}");
    let t = c.ok("create_task", json!({"title":"Do b","project":"b"}));
    assert!(t.contains("· b\n"), "{t}");
    let g = c.ok("get_topic", json!({"project":"a","topic":"x"}));
    assert!(g.contains("(`a/x`)"), "{g}");
    let g = c.ok("get_topic", json!({"project":"a","topic":"y"}));
    assert!(g.contains("(`a/y`)"), "{g}");
    // A folded name that matches two projects is still ambiguous.
    let e = c.err("create_task", json!({"title":"?","project":" A "}));
    assert!(e.contains("Ambiguous project"), "{e}");
}

fn big_seed(root: &Path) {
    let v = Vault::init(root).unwrap();
    let mut inbox = String::new();
    for i in 0..10_000 {
        inbox.push_str(&format!("- [ ] Zaległe zadanie {i} @2020-01-01 ^{i:06}\n"));
    }
    std::fs::write(root.join("skrzynka.md"), inbox).unwrap();
    let p = v
        .create_project(NewProject {
            name: "Duży".into(),
            ..Default::default()
        })
        .unwrap();
    let huge_notes: String = (0..5_000).map(|i| format!("linia notatki {i}\n")).collect();
    v.create_task(
        &TaskLocation::Project {
            project: p.slug.clone(),
        },
        NewTask {
            title: "Z ogromną notatką".into(),
            notes: huge_notes,
            ..Default::default()
        },
    )
    .unwrap();
    let t = v
        .create_topic(
            &p.slug,
            NewTopic {
                title: "Temat".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let path = root.join(&t.path);
    let mut s = std::fs::read_to_string(&path).unwrap();
    let mut tasks = String::new();
    let mut journal = String::new();
    for i in 0..1_000 {
        tasks.push_str(&format!("- [ ] Tematowe {i} ^t{i:05}\n"));
        journal.push_str(&format!("- 2026-01-01 10:00 · ja · wpis {i}\n"));
    }
    s = s.replace("## Zadania\n", &format!("## Zadania\n{tasks}"));
    s.push_str(&journal);
    std::fs::write(path, s).unwrap();
}

#[test]
fn outputs_are_bounded_and_pageable() {
    let mut c = Client::start(big_seed);
    let o = c.ok("overview", json!({}));
    assert!(o.len() < 20_000, "overview is {} bytes", o.len());
    assert!(o.contains("… and 99"), "{o}");
    assert!(o.contains("list_tasks"), "{o}");

    let o = c.ok("overview", json!({"project":"duzy"}));
    assert!(o.len() < 20_000, "project overview is {} bytes", o.len());
    assert!(!o.contains("linia notatki 4999"), "notes are cut");
    assert!(o.contains("more note line"), "{o}");

    let l = c.ok("list_tasks", json!({"inbox_only":true}));
    assert!(l.starts_with("10000 task(s)"), "{}", &l[..200]);
    assert!(l.len() < 20_000, "list_tasks is {} bytes", l.len());
    assert!(l.contains("offset=50"), "{}", &l[l.len() - 300..]);
    let page = c.ok(
        "list_tasks",
        json!({"inbox_only":true,"offset":9_990,"limit":20}),
    );
    assert!(
        page.contains("Zaległe zadanie 9999") && !page.contains("Zaległe zadanie 9989 "),
        "{page}"
    );

    let s = c.ok("search", json!({"query":"zadanie"}));
    assert!(s.len() < 20_000, "search is {} bytes", s.len());
    assert!(s.contains("offset=20"), "{s}");
    let s = c.ok(
        "search",
        json!({"query":"zadanie","offset":9_995,"limit":10}),
    );
    assert!(s.contains("Zaległe zadanie 9999"), "{s}");

    let g = c.ok("get_topic", json!({"project":"duzy","topic":"temat"}));
    assert!(g.len() < 30_000, "get_topic is {} bytes", g.len());
    assert!(
        g.contains("tasks_offset=") && g.contains("journal_offset="),
        "{g}"
    );
    let g = c.ok(
        "get_topic",
        json!({"project":"duzy","topic":"temat","tasks_offset":995,"journal_limit":1}),
    );
    assert!(
        g.contains("Tematowe 999 ") && !g.contains("Tematowe 994 "),
        "{g}"
    );
}

/// Every file under `root` with its bytes, for "nothing was written" checks.
fn tree(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                out.push((p.strip_prefix(root).unwrap().to_path_buf(), Vec::new()));
                stack.push(p);
            } else {
                let bytes = std::fs::read(&p).unwrap();
                out.push((p.strip_prefix(root).unwrap().to_path_buf(), bytes));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn read_only_mode_refuses_writes_and_writes_nothing() {
    for value in ["1", "true", " TRUE "] {
        let mut c = Client::start_with_env(seed, &[("MONTYBRAIN_READONLY", value)]);
        let before = tree(&c.root);

        let init = c.request(
            "initialize",
            json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}),
        );
        let instr = init["result"]["instructions"].as_str().unwrap();
        assert!(instr.contains("READ-ONLY RUN"), "{instr}");

        // Read tools work as before (minus normalize: the hand-written task keeps no id).
        let o = c.ok("overview", json!({}));
        assert!(o.contains("Śledzenie błędów"), "{o}");
        let l = c.ok("list_tasks", json!({"inbox_only": true}));
        assert!(l.contains("Ręcznie dopisane zadanie"), "{l}");
        assert!(c.ok("search", json!({"query":"sentry"})).contains("Sentry"));
        assert!(
            c.ok(
                "get_topic",
                json!({"project":"whenubuy","topic":"sledzenie-bledow"})
            )
            .contains("Integracja z Sentry.")
        );

        let writes = [
            ("create_task", json!({"title":"Nie wolno"})),
            ("update_task", json!({"id":"zzzzzz","status":"done"})),
            ("create_topic", json!({"project":"dom","title":"Nowy"})),
            (
                "log_update",
                json!({"project":"dom","topic":"remont","text":"wpis"}),
            ),
        ];
        for (tool, args) in writes {
            let e = c.err(tool, args);
            assert!(
                e.starts_with("read-only run:") && e.contains(tool),
                "{tool}: {e}"
            );
            assert!(e.contains("Nothing was changed"), "{tool}: {e}");
        }
        // Refused before any validation: even invalid-but-parseable input gets the same answer.
        let e = c.err("create_task", json!({"title":"x","status":"kaput"}));
        assert!(e.starts_with("read-only run:"), "{e}");

        assert_eq!(tree(&c.root), before, "read-only run changed the vault");
        assert!(
            !c.read("skrzynka.md").contains(" ^"),
            "normalize must not run"
        );
    }
}

#[test]
fn read_only_off_values_keep_writes_enabled() {
    for value in ["0", "", "false", "yes"] {
        let mut c = Client::start_with_env(seed, &[("MONTYBRAIN_READONLY", value)]);
        let init = c.request(
            "initialize",
            json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}),
        );
        assert!(
            !init["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("READ-ONLY"),
            "{value}"
        );
        let t = c.ok("create_task", json!({"title":"Wolno"}));
        assert!(t.contains("Wolno"), "{value}: {t}");
    }
}

/// Every path under `dir` (directories included), relative and sorted.
fn all_paths(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            out.push(p.strip_prefix(dir).unwrap().to_path_buf());
            if p.is_dir() {
                stack.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn read_only_start_creates_nothing() {
    // An existing but empty vault directory: no skeleton (`.montybrain/`,
    // `skrzynka.md`) is created, and reads work on the empty vault.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).unwrap();
    let mut c = Client::spawn(dir, root.clone(), &[("MONTYBRAIN_READONLY", "1")]);
    let o = c.ok("overview", json!({}));
    assert!(!o.is_empty());
    c.ok("list_tasks", json!({}));
    c.ok("search", json!({"query":"x"}));
    assert!(
        c.err("create_task", json!({"title":"x"}))
            .starts_with("read-only run:")
    );
    assert_eq!(
        all_paths(&root),
        Vec::<PathBuf>::new(),
        "read-only start wrote files"
    );

    // A missing vault: the server still starts, creates nothing, and every
    // tool answers with a clear error.
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_path_buf();
    let root = base.join("brak");
    let mut c = Client::spawn(dir, root.clone(), &[("MONTYBRAIN_READONLY", "1")]);
    for (tool, args) in [
        ("overview", json!({})),
        ("list_tasks", json!({})),
        ("search", json!({"query":"x"})),
        ("get_topic", json!({"project":"a","topic":"b"})),
    ] {
        let e = c.err(tool, args);
        assert!(
            e.contains("vault not found") && e.contains(root.to_str().unwrap()),
            "{tool}: {e}"
        );
    }
    assert!(
        c.err("create_task", json!({"title":"x"}))
            .starts_with("read-only run:")
    );
    assert!(
        !root.exists(),
        "read-only start created the vault directory"
    );
    assert_eq!(all_paths(&base), Vec::<PathBuf>::new());
}
