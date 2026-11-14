//! Codex as an in-app session: `codex app-server` (JSON-RPC over stdio, one
//! JSON object per line, no `"jsonrpc"` field).
//!
//! The method and type names come from the schema the installed binary
//! prints itself (`codex app-server generate-ts`, codex-cli 0.169.0):
//!
//! * client → server: `initialize {clientInfo, capabilities}`, then the
//!   `initialized` notification; `thread/start {cwd, approvalPolicy, sandbox}`
//!   → `{thread: {id}}`; `turn/start {threadId, input: [{type: "text", text,
//!   text_elements: []}]}` → `{turn: {id}}`; `turn/steer {threadId, input,
//!   expectedTurnId}` (a message while a turn runs); `turn/interrupt
//!   {threadId, turnId}`;
//! * server → client notifications: `turn/started`, `item/started`,
//!   `item/agentMessage/delta`, `item/completed` (items `agentMessage`,
//!   `commandExecution`, `mcpToolCall`, `fileChange`, `webSearch`…),
//!   `turn/completed {turn: {status, error}}`, `error`;
//! * server → client requests (answered with `{id, result}`):
//!   `item/commandExecution/requestApproval` → `{decision: accept |
//!   acceptForSession | decline}`; `item/fileChange/requestApproval` (same);
//!   `item/permissions/requestApproval` → `{permissions, scope: turn |
//!   session}`; `mcpServer/elicitation/request` → `{action: accept | decline,
//!   content, _meta}`; `item/tool/requestUserInput` → `{answers: {id:
//!   {answers: [..]}}}`; the legacy `execCommandApproval` /
//!   `applyPatchApproval` → `{decision: approved | approved_for_session |
//!   {denied: {rejection}}}`.
//!
//! GUESS: which of these requests Codex uses for computer-use tools (an MCP
//! elicitation is assumed for MCP tool approvals); the shape of the
//! elicitation `content` for an approval form (an empty object is sent).

use std::collections::HashMap;

use serde_json::{Value, json};

use super::gate::Ask;
use super::proto::{
    Act, DETAIL_MAX, Decision, Question, ReqRef, Request, args_detail, args_summary, clip,
    command_key, one_line, result_text,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Initialize,
    ThreadStart,
    TurnStart,
    TurnSteer,
    Interrupt,
}

#[derive(Debug, Default)]
pub struct Codex {
    next: u64,
    pending: HashMap<u64, Pending>,
    thread: Option<String>,
    turn: Option<String>,
    /// Messages typed before the thread exists.
    queued: Vec<String>,
    cwd: String,
    /// Paths of `fileChange` items (the approval request names only the item).
    item_paths: HashMap<String, Vec<String>>,
    /// The last agent message of the current turn (its final answer).
    last_text: String,
    texts: HashMap<String, String>,
}

fn input(text: &str) -> Value {
    json!([{"type": "text", "text": text, "text_elements": []}])
}

impl Codex {
    pub fn new(cwd: &str) -> Codex {
        Codex {
            cwd: cwd.to_string(),
            ..Default::default()
        }
    }

    fn request(&mut self, method: &str, params: Value, what: Pending) -> String {
        self.next += 1;
        self.pending.insert(self.next, what);
        json!({"id": self.next, "method": method, "params": params}).to_string()
    }

    pub fn start(&mut self, prompt: &str) -> Vec<String> {
        self.queued.push(prompt.to_string());
        vec![self.request(
            "initialize",
            json!({
                "clientInfo": {"name": "montybrain", "title": "MontyBrain", "version": env!("CARGO_PKG_VERSION")},
                "capabilities": {"experimentalApi": false, "requestAttestation": false},
            }),
            Pending::Initialize,
        )]
    }

    /// A message: a new turn, or added to the running one.
    pub fn user(&mut self, text: &str) -> Vec<String> {
        let Some(thread) = self.thread.clone() else {
            self.queued.push(text.to_string());
            return Vec::new();
        };
        match self.turn.clone() {
            Some(turn) => vec![self.request(
                "turn/steer",
                json!({"threadId": thread, "input": input(text), "expectedTurnId": turn}),
                Pending::TurnSteer,
            )],
            None => vec![self.turn_start(&thread, text)],
        }
    }

    fn turn_start(&mut self, thread: &str, text: &str) -> String {
        self.request(
            "turn/start",
            json!({"threadId": thread, "input": input(text)}),
            Pending::TurnStart,
        )
    }

    pub fn interrupt(&mut self) -> Vec<String> {
        match (self.thread.clone(), self.turn.clone()) {
            (Some(t), Some(turn)) => vec![self.request(
                "turn/interrupt",
                json!({"threadId": t, "turnId": turn}),
                Pending::Interrupt,
            )],
            _ => Vec::new(),
        }
    }

    pub fn answer(&mut self, req: &ReqRef, d: &Decision) -> Vec<String> {
        let allow = matches!(d, Decision::Allow | Decision::AllowSession);
        let session = matches!(d, Decision::AllowSession);
        let result = match req.method.as_str() {
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => json!({
                "decision": if session { "acceptForSession" } else if allow { "accept" } else { "decline" },
            }),
            "item/permissions/requestApproval" => {
                let asked = req
                    .data
                    .get("permissions")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let granted = if allow { strip_nulls(asked) } else { json!({}) };
                json!({"permissions": granted, "scope": if session { "session" } else { "turn" }})
            }
            "mcpServer/elicitation/request" => {
                if allow {
                    let content = if req.data.get("mode").and_then(Value::as_str) == Some("url") {
                        Value::Null
                    } else {
                        json!({})
                    };
                    json!({"action": "accept", "content": content, "_meta": null})
                } else {
                    json!({"action": "decline", "content": null, "_meta": null})
                }
            }
            "item/tool/requestUserInput" => {
                let mut answers = serde_json::Map::new();
                if let Decision::Answers(list) = d {
                    for (q, a) in list {
                        answers.insert(q.clone(), json!({"answers": [a]}));
                    }
                }
                json!({"answers": answers})
            }
            "execCommandApproval" | "applyPatchApproval" => json!({
                "decision": match d {
                    Decision::AllowSession => json!("approved_for_session"),
                    Decision::Allow => json!("approved"),
                    Decision::Deny(why) => json!({"denied": {"rejection": why}}),
                    Decision::Answers(_) => json!("abort"),
                },
            }),
            _ => {
                return vec![error_reply(
                    &req.id,
                    "MontyBrain nie obsługuje tego żądania.",
                )];
            }
        };
        vec![json!({"id": req.id, "result": result}).to_string()]
    }

    pub fn on_line(&mut self, raw: &str) -> Vec<Act> {
        let Ok(v) = serde_json::from_str::<Value>(raw.trim()) else {
            return Vec::new();
        };
        let method = v.get("method").and_then(Value::as_str);
        let id = v.get("id");
        match (method, id) {
            // A response to one of our requests.
            (None, Some(id)) => self.response(id, &v),
            // A request from the server.
            (Some(m), Some(id)) => {
                self.server_request(m, id.clone(), v.get("params").cloned().unwrap_or_default())
            }
            (Some(m), None) => self.notification(m, v.get("params").cloned().unwrap_or_default()),
            _ => Vec::new(),
        }
    }

    fn response(&mut self, id: &Value, v: &Value) -> Vec<Act> {
        let Some(what) = id.as_u64().and_then(|n| self.pending.remove(&n)) else {
            return Vec::new();
        };
        if let Some(e) = v.get("error") {
            let msg = e.get("message").and_then(Value::as_str).unwrap_or("błąd");
            let fatal = matches!(
                what,
                Pending::Initialize | Pending::ThreadStart | Pending::TurnStart
            );
            let mut out = vec![Act::Notice {
                text: format!("Codex odrzucił polecenie: {}", clip(msg, 300)),
                error: true,
            }];
            if fatal {
                out.push(Act::TurnDone {
                    text: None,
                    error: Some(clip(msg, 300)),
                });
            }
            return out;
        }
        let r = v.get("result").cloned().unwrap_or_default();
        match what {
            Pending::Initialize => {
                let start = self.request(
                    "thread/start",
                    json!({"cwd": self.cwd, "approvalPolicy": "on-request", "sandbox": "workspace-write"}),
                    Pending::ThreadStart,
                );
                vec![
                    Act::Write(json!({"method": "initialized"}).to_string()),
                    Act::Write(start),
                ]
            }
            Pending::ThreadStart => {
                let Some(thread) = r
                    .pointer("/thread/id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                else {
                    return vec![Act::TurnDone {
                        text: None,
                        error: Some("Codex nie założył wątku.".into()),
                    }];
                };
                self.thread = Some(thread.clone());
                let model = r.get("model").and_then(Value::as_str).unwrap_or("");
                let mut out = vec![Act::Notice {
                    text: format!(
                        "Codex połączony{}.",
                        if model.is_empty() {
                            String::new()
                        } else {
                            format!(" (model {model})")
                        }
                    ),
                    error: false,
                }];
                let queued = std::mem::take(&mut self.queued);
                if !queued.is_empty() {
                    out.push(Act::Write(self.turn_start(&thread, &queued.join("\n\n"))));
                }
                out
            }
            Pending::TurnStart => {
                if let Some(t) = r.pointer("/turn/id").and_then(Value::as_str) {
                    self.turn = Some(t.to_string());
                }
                Vec::new()
            }
            Pending::TurnSteer | Pending::Interrupt => Vec::new(),
        }
    }

    fn notification(&mut self, m: &str, p: Value) -> Vec<Act> {
        match m {
            "turn/started" => {
                self.turn = p
                    .pointer("/turn/id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or(self.turn.take());
                self.last_text.clear();
                vec![Act::TurnStarted]
            }
            "turn/completed" => {
                self.turn = None;
                let status = p
                    .pointer("/turn/status")
                    .and_then(Value::as_str)
                    .unwrap_or("completed");
                let error = p
                    .pointer("/turn/error/message")
                    .and_then(Value::as_str)
                    .map(|e| clip(e, 300));
                let text = std::mem::take(&mut self.last_text);
                match status {
                    "completed" => vec![Act::TurnDone {
                        text: Some(text).filter(|t| !t.is_empty()),
                        error: None,
                    }],
                    "interrupted" => vec![Act::TurnDone {
                        text: None,
                        error: Some(error.unwrap_or_else(|| "Przerwano.".into())),
                    }],
                    _ => vec![Act::TurnDone {
                        text: None,
                        error: Some(error.unwrap_or_else(|| "Tura się nie udała.".into())),
                    }],
                }
            }
            "item/agentMessage/delta" => {
                let key = p
                    .get("itemId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let delta = p
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.texts.entry(key.clone()).or_default().push_str(&delta);
                vec![Act::Text {
                    key,
                    text: delta,
                    append: true,
                }]
            }
            "item/started" | "item/completed" => self.item(
                m == "item/completed",
                p.get("item").cloned().unwrap_or_default(),
            ),
            "error" => {
                let msg = p
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("błąd");
                let retry = p.get("willRetry").and_then(Value::as_bool).unwrap_or(false);
                vec![Act::Notice {
                    text: format!(
                        "{}{}",
                        clip(msg, 300),
                        if retry { " (ponawiam)" } else { "" }
                    ),
                    error: !retry,
                }]
            }
            _ => Vec::new(),
        }
    }

    fn item(&mut self, done: bool, it: Value) -> Vec<Act> {
        let s = |k: &str| it.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let id = s("id");
        match s("type").as_str() {
            "agentMessage" => {
                if !done {
                    return Vec::new();
                }
                let text = s("text");
                self.texts.remove(&id);
                if text.trim().is_empty() {
                    return Vec::new();
                }
                self.last_text = text.clone();
                vec![Act::Text {
                    key: id,
                    text,
                    append: false,
                }]
            }
            "commandExecution" => {
                let command = s("command");
                if done {
                    let code = it.get("exitCode").and_then(Value::as_i64);
                    let ok = s("status") == "completed" && code.is_none_or(|c| c == 0);
                    let out = it
                        .get("aggregatedOutput")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    vec![Act::ToolDone {
                        key: id,
                        ok,
                        output: clip(out.trim(), super::proto::OUTPUT_MAX),
                    }]
                } else {
                    vec![Act::Tool {
                        key: id,
                        name: "Polecenie".into(),
                        summary: clip(&one_line(&command), super::proto::SUMMARY_MAX),
                        detail: format!("{}\n(w: {})", clip(&command, DETAIL_MAX), s("cwd")),
                    }]
                }
            }
            "mcpToolCall" => {
                let name = format!("{}/{}", s("server"), s("tool"));
                let args = it.get("arguments").cloned().unwrap_or_default();
                if done {
                    let err = it.pointer("/error/message").and_then(Value::as_str);
                    let out = match err {
                        Some(e) => clip(e, super::proto::OUTPUT_MAX),
                        None => result_text(it.pointer("/result/content").unwrap_or(&Value::Null)),
                    };
                    vec![Act::ToolDone {
                        key: id,
                        ok: err.is_none() && s("status") != "failed",
                        output: out,
                    }]
                } else {
                    vec![Act::Tool {
                        key: id,
                        name,
                        summary: args_summary(&args),
                        detail: args_detail(&args),
                    }]
                }
            }
            "fileChange" => {
                let paths: Vec<String> = it
                    .get("changes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c.get("path").and_then(Value::as_str).map(str::to_string))
                    .collect();
                self.item_paths.insert(id.clone(), paths.clone());
                if done {
                    vec![Act::ToolDone {
                        key: id,
                        ok: s("status") == "completed",
                        output: String::new(),
                    }]
                } else {
                    vec![Act::Tool {
                        key: id,
                        name: "Zmiana plików".into(),
                        summary: clip(&paths.join(", "), super::proto::SUMMARY_MAX),
                        detail: paths.join("\n"),
                    }]
                }
            }
            "webSearch" if !done => {
                vec![Act::Tool {
                    key: id,
                    name: "Wyszukiwanie".into(),
                    summary: clip(&s("query"), super::proto::SUMMARY_MAX),
                    detail: String::new(),
                }]
            }
            "webSearch" => vec![Act::ToolDone {
                key: id,
                ok: true,
                output: String::new(),
            }],
            "dynamicToolCall" | "collabAgentToolCall" if !done => {
                vec![Act::Tool {
                    key: id,
                    name: s("tool"),
                    summary: String::new(),
                    detail: args_detail(it.get("arguments").unwrap_or(&Value::Null)),
                }]
            }
            _ => Vec::new(),
        }
    }

    fn server_request(&mut self, m: &str, id: Value, p: Value) -> Vec<Act> {
        let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let reason = s("reason");
        let with_reason = |d: String| {
            if reason.is_empty() {
                d
            } else {
                format!("{}\n{d}", one_line(&reason))
            }
        };
        let mk = |ask: Ask, data: Value, questions: Vec<Question>| {
            vec![Act::Ask(Box::new(Request {
                req: ReqRef {
                    id: id.clone(),
                    method: m.to_string(),
                    data,
                },
                ask,
                questions,
            }))]
        };
        match m {
            "item/commandExecution/requestApproval" | "execCommandApproval" => {
                let command = match p.get("command") {
                    Some(Value::Array(a)) => a
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" "),
                    Some(Value::String(c)) => c.clone(),
                    _ => String::new(),
                };
                mk(
                    Ask {
                        tool: "command".into(),
                        key: command_key("command", &command),
                        title: format!(
                            "Polecenie: {}",
                            clip(&one_line(&command), super::proto::SUMMARY_MAX)
                        ),
                        detail: with_reason(format!(
                            "{}\n(w: {})",
                            clip(&command, DETAIL_MAX),
                            s("cwd")
                        )),
                        paths: Vec::new(),
                        text: command,
                        can_session: true,
                    },
                    p.clone(),
                    Vec::new(),
                )
            }
            "item/fileChange/requestApproval" | "applyPatchApproval" => {
                let mut paths = self
                    .item_paths
                    .get(&s("itemId"))
                    .cloned()
                    .unwrap_or_default();
                if let Some(Value::Object(changes)) = p.get("fileChanges") {
                    paths.extend(changes.keys().cloned());
                }
                if let Some(root) = p.get("grantRoot").and_then(Value::as_str) {
                    paths.push(root.to_string());
                }
                mk(
                    Ask {
                        tool: "fileChange".into(),
                        key: "fileChange".into(),
                        title: format!(
                            "Zmiana plików: {}",
                            clip(&paths.join(", "), super::proto::SUMMARY_MAX)
                        ),
                        detail: with_reason(paths.join("\n")),
                        paths,
                        text: String::new(),
                        can_session: true,
                    },
                    p.clone(),
                    Vec::new(),
                )
            }
            "item/permissions/requestApproval" => {
                let perms = p.get("permissions").cloned().unwrap_or_default();
                let mut paths = Vec::new();
                collect_strings(perms.get("fileSystem").unwrap_or(&Value::Null), &mut paths);
                mk(
                    Ask {
                        tool: "permissions".into(),
                        key: "permissions".into(),
                        title: "Dodatkowe uprawnienia".into(),
                        detail: with_reason(args_detail(&perms)),
                        paths,
                        text: String::new(),
                        can_session: true,
                    },
                    p.clone(),
                    Vec::new(),
                )
            }
            "mcpServer/elicitation/request" => {
                let server = s("serverName");
                let mut detail = one_line(&s("message"));
                if let Some(u) = p.get("url").and_then(Value::as_str) {
                    detail.push_str(&format!("\n{u}"));
                }
                mk(
                    Ask {
                        tool: format!("mcp:{server}"),
                        key: format!("mcp:{server}"),
                        title: format!(
                            "{server}: {}",
                            clip(&one_line(&s("message")), super::proto::SUMMARY_MAX)
                        ),
                        detail: clip(&detail, DETAIL_MAX),
                        paths: Vec::new(),
                        text: String::new(),
                        can_session: true,
                    },
                    p.clone(),
                    Vec::new(),
                )
            }
            "item/tool/requestUserInput" => {
                let questions: Vec<Question> = p
                    .get("questions")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|q| Question {
                        id: q
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        header: q
                            .get("header")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        question: q
                            .get("question")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        options: q
                            .get("options")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(|o| {
                                o.get("label").and_then(Value::as_str).map(str::to_string)
                            })
                            .collect(),
                    })
                    .collect();
                let title = questions
                    .first()
                    .map(|q| one_line(&q.question))
                    .unwrap_or_else(|| "Pytanie agenta".into());
                mk(
                    Ask {
                        tool: "question".into(),
                        key: "question".into(),
                        title: clip(&title, super::proto::SUMMARY_MAX),
                        detail: String::new(),
                        paths: Vec::new(),
                        text: String::new(),
                        can_session: false,
                    },
                    p.clone(),
                    questions,
                )
            }
            // Tools we did not register, auth refresh, attestation: not ours.
            _ => vec![Act::Write(error_reply(
                &id,
                "MontyBrain nie obsługuje tego żądania.",
            ))],
        }
    }
}

fn error_reply(id: &Value, msg: &str) -> String {
    json!({"id": id, "error": {"code": -32601, "message": msg}}).to_string()
}

fn strip_nulls(v: Value) -> Value {
    match v {
        Value::Object(o) => Value::Object(
            o.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, strip_nulls(v)))
                .collect(),
        ),
        other => other,
    }
}

fn collect_strings(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|x| collect_strings(x, out)),
        Value::Object(o) => o.values().for_each(|x| collect_strings(x, out)),
        _ => {}
    }
}

/// argv (without the program): the app server on stdio, `montybrain-mcp`
/// overridden to the read-only one, model / effort as config overrides.
pub fn args(mcp_overrides: &[String], agent: &[String]) -> Vec<String> {
    let mut a = vec!["app-server".to_string()];
    for o in mcp_overrides {
        a.extend(["-c".to_string(), o.clone()]);
    }
    // `codex app-server` takes no `-m`: the model goes in as `-c model=…`.
    let mut it = agent.iter();
    while let Some(x) = it.next() {
        if x == "-m"
            && let Some(m) = it.next()
        {
            a.extend([
                "-c".to_string(),
                format!("model={}", super::super::command::toml_string(m)),
            ]);
        } else {
            a.push(x.clone());
        }
    }
    a.extend(["--listen".to_string(), "stdio://".to_string()]);
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(c: &mut Codex, text: &str) -> Vec<Act> {
        text.lines().flat_map(|l| c.on_line(l)).collect()
    }

    fn writes(acts: &[Act]) -> Vec<Value> {
        acts.iter()
            .filter_map(|a| match a {
                Act::Write(l) => Some(serde_json::from_str(l).unwrap()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn handshake_thread_and_first_turn() {
        let mut c = Codex::new("/c/sesja");
        let first: Value = serde_json::from_str(&c.start("Zadanie")[0]).unwrap();
        assert_eq!(first["method"], "initialize");
        assert_eq!(first["params"]["clientInfo"]["name"], "montybrain");
        assert!(first.get("jsonrpc").is_none());
        let acts = lines(
            &mut c,
            r##"{"id":1,"result":{"userAgent":"codex","codexHome":"/h","platformFamily":"unix","platformOs":"macos"}}"##,
        );
        let w = writes(&acts);
        assert_eq!(w[0], json!({"method": "initialized"}));
        assert_eq!(w[1]["method"], "thread/start");
        assert_eq!(
            w[1]["params"],
            json!({"cwd": "/c/sesja", "approvalPolicy": "on-request", "sandbox": "workspace-write"})
        );
        // A message typed meanwhile goes into the first turn.
        assert!(c.user("I jeszcze to").is_empty());
        let acts = lines(
            &mut c,
            r##"{"id":2,"result":{"thread":{"id":"th1"},"model":"gpt-x"}}"##,
        );
        assert!(matches!(&acts[0], Act::Notice { text, .. } if text.contains("gpt-x")));
        let w = writes(&acts);
        assert_eq!(w[0]["method"], "turn/start");
        assert_eq!(w[0]["params"]["threadId"], "th1");
        assert_eq!(
            w[0]["params"]["input"][0]["text"],
            "Zadanie\n\nI jeszcze to"
        );
        lines(
            &mut c,
            r##"{"id":3,"result":{"turn":{"id":"tu1","status":"inProgress"}}}"##,
        );
        // While the turn runs a message steers it; Stop interrupts it.
        let s: Value = serde_json::from_str(&c.user("Szybciej")[0]).unwrap();
        assert_eq!(s["method"], "turn/steer");
        assert_eq!(s["params"]["expectedTurnId"], "tu1");
        let i: Value = serde_json::from_str(&c.interrupt()[0]).unwrap();
        assert_eq!(i["params"], json!({"threadId": "th1", "turnId": "tu1"}));
    }

    /// Hand-written from the generated TypeScript schema (ItemStartedNotification,
    /// AgentMessageDeltaNotification, ItemCompletedNotification, TurnCompletedNotification).
    const TURN: &str = r##"{"method":"turn/started","params":{"threadId":"th1","turn":{"id":"tu1","status":"inProgress","items":[]}}}
{"method":"item/started","params":{"threadId":"th1","turnId":"tu1","startedAtMs":1,"item":{"type":"mcpToolCall","id":"i1","server":"computer-use","tool":"screenshot","status":"inProgress","arguments":{"app":"Slack"}}}}
{"method":"item/completed","params":{"threadId":"th1","turnId":"tu1","completedAtMs":2,"item":{"type":"mcpToolCall","id":"i1","server":"computer-use","tool":"screenshot","status":"completed","arguments":{"app":"Slack"},"result":{"content":[{"type":"text","text":"zrzut"}],"structuredContent":null,"_meta":null},"error":null}}}
{"method":"item/started","params":{"threadId":"th1","turnId":"tu1","startedAtMs":3,"item":{"type":"commandExecution","id":"i2","command":"ls","cwd":"/c","status":"inProgress"}}}
{"method":"item/completed","params":{"threadId":"th1","turnId":"tu1","completedAtMs":4,"item":{"type":"commandExecution","id":"i2","command":"ls","cwd":"/c","status":"failed","exitCode":2,"aggregatedOutput":"ls: nie ma"}}}
{"method":"item/agentMessage/delta","params":{"threadId":"th1","turnId":"tu1","itemId":"i3","delta":"Na kanale "}}
{"method":"item/agentMessage/delta","params":{"threadId":"th1","turnId":"tu1","itemId":"i3","delta":"3 wątki."}}
{"method":"item/completed","params":{"threadId":"th1","turnId":"tu1","completedAtMs":5,"item":{"type":"agentMessage","id":"i3","text":"Na kanale 3 wątki.\n```montybrain-zmiany\n{\"changes\":[]}\n```"}}}
{"method":"turn/completed","params":{"threadId":"th1","turn":{"id":"tu1","status":"completed","items":[],"error":null}}}"##;

    #[test]
    fn a_turn_streams_tools_text_and_the_final_answer() {
        let mut c = Codex::new("/c");
        let acts = lines(&mut c, TURN);
        assert!(matches!(acts[0], Act::TurnStarted));
        assert!(
            matches!(&acts[1], Act::Tool { name, summary, .. } if name == "computer-use/screenshot" && summary.contains("Slack"))
        );
        assert!(matches!(&acts[2], Act::ToolDone { ok: true, output, .. } if output == "zrzut"));
        assert!(matches!(&acts[3], Act::Tool { name, .. } if name == "Polecenie"));
        assert!(matches!(&acts[4], Act::ToolDone { ok: false, .. }));
        assert!(matches!(&acts[5], Act::Text { append: true, text, .. } if text == "Na kanale "));
        assert!(matches!(&acts[7], Act::Text { append: false, key, .. } if key == "i3"));
        assert!(
            matches!(&acts[8], Act::TurnDone { text: Some(t), error: None } if t.contains("montybrain-zmiany"))
        );
    }

    #[test]
    fn approvals_become_requests_and_answers_have_the_schema_shape() {
        let mut c = Codex::new("/c");
        let req = |c: &mut Codex, l: &str| match c.on_line(l).remove(0) {
            Act::Ask(r) => *r,
            other => panic!("{other:?}"),
        };
        let r = req(
            &mut c,
            r##"{"id":"s1","method":"item/commandExecution/requestApproval","params":{"kind":"command","threadId":"th1","turnId":"tu1","itemId":"i9","startedAtMs":1,"environmentId":null,"command":"osascript -e 'x'","cwd":"/c","reason":"potrzebne"}}"##,
        );
        assert_eq!(r.ask.key, "command:osascript");
        assert!(r.ask.detail.starts_with("potrzebne"));
        let a: Value = serde_json::from_str(&c.answer(&r.req, &Decision::AllowSession)[0]).unwrap();
        assert_eq!(
            a,
            json!({"id": "s1", "result": {"decision": "acceptForSession"}})
        );
        let a: Value =
            serde_json::from_str(&c.answer(&r.req, &Decision::Deny("nie".into()))[0]).unwrap();
        assert_eq!(a["result"]["decision"], "decline");

        // A file change names its paths through the item seen before.
        c.on_line(r##"{"method":"item/started","params":{"threadId":"th1","turnId":"tu1","startedAtMs":1,"item":{"type":"fileChange","id":"f1","changes":[{"path":"/v/x.md","kind":"update","diff":""}],"status":"inProgress"}}}"##);
        let r = req(
            &mut c,
            r##"{"id":7,"method":"item/fileChange/requestApproval","params":{"threadId":"th1","turnId":"tu1","itemId":"f1","startedAtMs":1}}"##,
        );
        assert_eq!(r.ask.paths, ["/v/x.md"]);

        // GUESS: computer-use (an MCP server) asks through an elicitation.
        let r = req(
            &mut c,
            r##"{"id":8,"method":"mcpServer/elicitation/request","params":{"threadId":"th1","turnId":"tu1","serverName":"computer-use","mode":"form","_meta":null,"message":"Allow click in Slack?","requestedSchema":{"type":"object","properties":{}}}}"##,
        );
        assert_eq!(r.ask.tool, "mcp:computer-use");
        let a: Value = serde_json::from_str(&c.answer(&r.req, &Decision::Allow)[0]).unwrap();
        assert_eq!(
            a["result"],
            json!({"action": "accept", "content": {}, "_meta": null})
        );

        let r = req(
            &mut c,
            r##"{"id":9,"method":"item/permissions/requestApproval","params":{"threadId":"th1","turnId":"tu1","itemId":"p","environmentId":null,"startedAtMs":1,"cwd":"/c","reason":null,"permissions":{"network":{"enabled":true},"fileSystem":null}}}"##,
        );
        let a: Value = serde_json::from_str(&c.answer(&r.req, &Decision::Allow)[0]).unwrap();
        assert_eq!(
            a["result"],
            json!({"permissions": {"network": {"enabled": true}}, "scope": "turn"})
        );

        let r = req(
            &mut c,
            r##"{"id":10,"method":"item/tool/requestUserInput","params":{"threadId":"th1","turnId":"tu1","itemId":"q","isBlocking":true,"autoResolutionMs":null,"questions":[{"id":"k","header":"Kanał","question":"Który kanał?","isOther":false,"isSecret":false,"options":[{"label":"#zespol","description":""},{"label":"#ogolny","description":""}]}]}}"##,
        );
        assert_eq!(r.questions[0].options, ["#zespol", "#ogolny"]);
        let a: Value = serde_json::from_str(
            &c.answer(
                &r.req,
                &Decision::Answers(vec![("k".into(), "#zespol".into())]),
            )[0],
        )
        .unwrap();
        assert_eq!(
            a["result"],
            json!({"answers": {"k": {"answers": ["#zespol"]}}})
        );

        // Requests we cannot serve are answered with an error at once.
        let w =
            c.on_line(r##"{"id":11,"method":"account/chatgptAuthTokens/refresh","params":{}}"##);
        assert!(matches!(&w[0], Act::Write(l) if l.contains("-32601")));
    }

    #[test]
    fn argv_has_the_read_only_server_and_no_model_flag() {
        let a = args(
            &["mcp_servers.montybrain.env.MONTYBRAIN_READONLY=\"1\"".into()],
            &["-m".into(), "gpt-x".into()],
        );
        assert_eq!(a[0], "app-server");
        assert!(
            a.windows(2)
                .any(|w| w[0] == "-c" && w[1].contains("MONTYBRAIN_READONLY"))
        );
        assert!(a.contains(&"model=\"gpt-x\"".to_string()));
        assert!(!a.contains(&"-m".to_string()));
    }
}
