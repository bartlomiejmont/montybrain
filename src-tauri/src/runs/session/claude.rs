//! Claude Code as an in-app session: `claude -p --input-format stream-json
//! --output-format stream-json --verbose --permission-prompt-tool stdio`.
//!
//! One JSON object per line both ways. What this relies on (checked against
//! `claude --help` 2.1.325 and the protocol strings in the installed binary;
//! the exact message shapes are not documented in `--help`, so the fixtures
//! in the tests are written by hand from those strings and marked GUESS
//! where a field is assumed):
//!
//! * stdin, a user turn: `{"type":"user","message":{"role":"user","content":[{"type":"text","text":…}]},"parent_tool_use_id":null,"session_id":""}`;
//! * stdin, control requests: `{"type":"control_request","request_id":…,"request":{"subtype":"initialize"|"interrupt"}}`;
//! * stdout: `system/init`, `assistant` (content blocks `text`, `tool_use`),
//!   `user` (blocks `tool_result`), `result` (end of a turn, `result` text);
//! * stdout, a permission request (because of `--permission-prompt-tool
//!   stdio`): `{"type":"control_request","request_id":…,"request":{"subtype":"can_use_tool","tool_name":…,"input":{…},"tool_use_id":…,"permission_suggestions":[…],"blocked_path":…}}`,
//!   answered on stdin with `{"type":"control_response","response":{"subtype":"success","request_id":…,"response":{"behavior":"allow","updatedInput":{…}}}}`
//!   or `{"behavior":"deny","message":…}`;
//! * stdout, `control_cancel_request` withdraws a pending request.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::gate::Ask;
use super::proto::{
    Act, DETAIL_MAX, Decision, Question, ReqRef, Request, args_detail, args_summary, clip,
    command_key, one_line, result_text, written_paths,
};

#[derive(Debug, Default)]
pub struct Claude {
    next: u64,
}

fn line(v: Value) -> String {
    v.to_string()
}

impl Claude {
    fn req_id(&mut self, what: &str) -> String {
        self.next += 1;
        format!("mb-{what}-{}", self.next)
    }

    /// What to send right after the start: the handshake and the first turn.
    pub fn start(&mut self, prompt: &str) -> Vec<String> {
        let id = self.req_id("init");
        vec![
            line(
                json!({"type": "control_request", "request_id": id, "request": {"subtype": "initialize", "hooks": null}}),
            ),
            self.user(prompt),
        ]
    }

    /// A follow-up message (also while the agent works: Claude Code queues it).
    pub fn user(&mut self, text: &str) -> String {
        line(json!({
            "type": "user",
            "message": {"role": "user", "content": [{"type": "text", "text": text}]},
            "parent_tool_use_id": null,
            "session_id": "",
        }))
    }

    pub fn interrupt(&mut self) -> Vec<String> {
        let id = self.req_id("interrupt");
        vec![line(
            json!({"type": "control_request", "request_id": id, "request": {"subtype": "interrupt"}}),
        )]
    }

    /// The answer to a `can_use_tool` request.
    pub fn answer(&mut self, req: &ReqRef, d: &Decision) -> Vec<String> {
        let input = req.data.get("input").cloned().unwrap_or_else(|| json!({}));
        let response = match d {
            Decision::Allow => json!({"behavior": "allow", "updatedInput": input}),
            Decision::AllowSession => {
                // GUESS: the CLI's own suggestions (session-scoped rules) are
                // accepted back as `updatedPermissions`; we also remember the
                // key ourselves, so later requests are answered without a card.
                let mut r = json!({"behavior": "allow", "updatedInput": input});
                if let Some(Value::Array(s)) = req.data.get("suggestions")
                    && !s.is_empty()
                {
                    r["updatedPermissions"] = Value::Array(
                        s.iter()
                            .cloned()
                            .map(|mut x| {
                                if x.is_object() {
                                    x["destination"] = json!("session");
                                }
                                x
                            })
                            .collect(),
                    );
                }
                r
            }
            Decision::Deny(why) => json!({"behavior": "deny", "message": why}),
            Decision::Answers(_) => json!({"behavior": "deny", "message": "Brak odpowiedzi."}),
        };
        vec![line(json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": req.id, "response": response},
        }))]
    }

    /// One stdout line.
    pub fn on_line(&mut self, raw: &str, tools: &mut HashMap<String, String>) -> Vec<Act> {
        let Ok(v) = serde_json::from_str::<Value>(raw.trim()) else {
            return Vec::new();
        };
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("");
        match s("type") {
            "rate_limit_event" => {
                let (limits, status) = super::telemetry::claude_limits(&v);
                let mut out = Vec::new();
                if !limits.is_empty() { out.push(Act::Limits(limits)); }
                if let Some(status) = status { out.push(Act::LimitStatus(status)); }
                out
            }
            "system" if s("subtype") == "init" => {
                let model = s("model");
                let failed: Vec<&str> = v
                    .get("mcp_servers")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter(|m| {
                                m.get("status")
                                    .and_then(Value::as_str)
                                    .is_some_and(|st| st != "connected")
                            })
                            .filter_map(|m| m.get("name").and_then(Value::as_str))
                            .collect()
                    })
                    .unwrap_or_default();
                let mut text = format!(
                    "Claude Code połączony{}.",
                    if model.is_empty() {
                        String::new()
                    } else {
                        format!(" (model {model})")
                    }
                );
                if !failed.is_empty() {
                    text.push_str(&format!(
                        " Serwery MCP bez połączenia: {}.",
                        failed.join(", ")
                    ));
                }
                vec![
                    Act::Notice { text, error: false },
                    Act::Connected((!model.is_empty()).then(|| clip(model, 200))),
                ]
            }
            "assistant" => {
                let mut out = Vec::new();
                let msg = v.get("message").cloned().unwrap_or_default();
                let mid = msg
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("m")
                    .to_string();
                for (i, b) in msg
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    match b.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            let t = b.get("text").and_then(Value::as_str).unwrap_or("");
                            if !t.trim().is_empty() {
                                out.push(Act::Text {
                                    key: format!("{mid}:{i}"),
                                    text: t.to_string(),
                                    append: false,
                                });
                            }
                        }
                        Some("tool_use") => {
                            let id = b
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string();
                            let name = b
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("?")
                                .to_string();
                            let input = b.get("input").cloned().unwrap_or_default();
                            tools.insert(id.clone(), name.clone());
                            out.push(Act::Tool {
                                key: id,
                                name,
                                summary: args_summary(&input),
                                detail: args_detail(&input),
                            });
                        }
                        _ => {}
                    }
                }
                out
            }
            "user" => v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
                .map(|b| Act::ToolDone {
                    key: b
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    ok: !b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                    output: result_text(b.get("content").unwrap_or(&Value::Null)),
                })
                .collect(),
            "result" => {
                let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                let text = v.get("result").and_then(Value::as_str).map(str::to_string);
                let error = is_error.then(|| match s("subtype") {
                    "" | "success" => text
                        .clone()
                        .unwrap_or_else(|| "Agent zakończył z błędem.".into()),
                    sub => format!("Agent zakończył z błędem ({sub})."),
                });
                let mut out = vec![Act::TurnDone {
                    text: if is_error { None } else { text },
                    error,
                }];
                if let Some(usage) = super::telemetry::Usage::claude(&v) {
                    out.push(Act::Usage(usage));
                }
                out
            }
            "control_request" => {
                let id = v.get("request_id").cloned().unwrap_or(Value::Null);
                let r = v.get("request").cloned().unwrap_or_default();
                let sub = r.get("subtype").and_then(Value::as_str).unwrap_or("");
                if sub != "can_use_tool" {
                    // Hooks, elicitation dialogs…: not offered by this host.
                    return vec![Act::Write(line(json!({
                        "type": "control_response",
                        "response": {"subtype": "error", "request_id": id, "error": format!("MontyBrain nie obsługuje żądania {sub}.")},
                    })))];
                }
                vec![Act::Ask(Box::new(permission(id, &r)))]
            }
            "control_cancel_request" => vec![Act::Cancel {
                id: v.get("request_id").cloned().unwrap_or(Value::Null),
            }],
            "control_response" => {
                let r = v.get("response").cloned().unwrap_or_default();
                if r.get("subtype").and_then(Value::as_str) == Some("error") {
                    let e = r.get("error").and_then(Value::as_str).unwrap_or("błąd");
                    vec![Act::Notice {
                        text: format!("Claude Code odrzucił polecenie: {}", clip(e, 300)),
                        error: true,
                    }]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }
}

/// A `can_use_tool` request as a card (or a gate decision).
fn permission(id: Value, r: &Value) -> Request {
    let tool = r
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_string();
    let input = r.get("input").cloned().unwrap_or_else(|| json!({}));
    let shown = r
        .get("title")
        .or_else(|| r.get("display_name"))
        .and_then(Value::as_str)
        .unwrap_or(&tool)
        .to_string();
    let command = input
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let key = if tool == "Bash" {
        command_key("Bash", &command)
    } else {
        tool.clone()
    };
    let mut paths = if matches!(
        tool.as_str(),
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit"
    ) {
        written_paths(&input)
    } else {
        Vec::new()
    };
    if let Some(p) = r.get("blocked_path").and_then(Value::as_str) {
        paths.push(p.to_string());
    }
    let mut detail = String::new();
    if let Some(d) = r.get("description").and_then(Value::as_str) {
        detail.push_str(&one_line(d));
        detail.push('\n');
    }
    detail.push_str(&args_detail(&input));
    Request {
        req: ReqRef {
            id,
            method: "can_use_tool".into(),
            data: json!({"input": input, "suggestions": r.get("permission_suggestions").cloned().unwrap_or(Value::Null)}),
        },
        ask: Ask {
            title: format!("{shown}: {}", args_summary(&input))
                .trim_end_matches(": ")
                .to_string(),
            tool,
            key,
            detail: clip(detail.trim(), DETAIL_MAX),
            paths,
            text: command,
            can_session: true,
        },
        questions: Vec::<Question>::new(),
    }
}

/// argv (without the program) of a session. The prompt is not in argv: it is
/// the first user message on stdin.
pub fn args(mcp_config: &str, vault: &str, extra: &[String]) -> Vec<String> {
    let vault = vault.trim_end_matches('/');
    let mut a: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--permission-prompt-tool",
        "stdio",
        "--permission-mode",
        "default",
    ]
    .map(String::from)
    .to_vec();
    a.extend(extra.iter().cloned());
    a.push("--mcp-config".into());
    a.push(mcp_config.into());
    a.push("--disallowedTools".into());
    a.extend(
        super::super::command::MONTYBRAIN_WRITING_TOOLS
            .iter()
            .map(|t| t.to_string()),
    );
    a.push(format!("Edit(/{vault}/**)"));
    a.push(format!("Write(/{vault}/**)"));
    a.push("--allowedTools".into());
    a.extend(
        super::super::command::MONTYBRAIN_READ_TOOLS
            .iter()
            .map(|t| t.to_string()),
    );
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-written from the protocol strings of Claude Code 2.1.325. GUESS:
    /// the exact set of fields of `system/init` and `result`.
    const FIXTURE: &str = r##"{"type":"system","subtype":"init","session_id":"s1","model":"claude-x","mcp_servers":[{"name":"montybrain","status":"connected"},{"name":"slack","status":"failed"}]}
{"type":"assistant","message":{"id":"msg_1","role":"assistant","content":[{"type":"text","text":"Sprawdzę kanał."},{"type":"tool_use","id":"toolu_1","name":"mcp__slack__read_channel","input":{"channel":"#zespol","limit":20}}]},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"control_request","request_id":"req-7","request":{"subtype":"can_use_tool","tool_name":"mcp__slack__read_channel","display_name":"slack - read_channel","input":{"channel":"#zespol","limit":20},"tool_use_id":"toolu_1","permission_suggestions":[{"type":"addRules","rules":[{"toolName":"mcp__slack__read_channel"}],"behavior":"allow","destination":"localSettings"}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"text","text":"3 wiadomości"}],"is_error":false}]}}
{"type":"control_request","request_id":"req-8","request":{"subtype":"elicitation","message":"?"}}
{"type":"control_cancel_request","request_id":"req-9"}
{"type":"result","subtype":"success","is_error":false,"result":"Gotowe.\n```montybrain-zmiany\n{\"changes\": []}\n```","session_id":"s1"}
nie-json"##;

    #[test]
    fn parses_a_session_transcript() {
        let mut c = Claude::default();
        let mut tools = HashMap::new();
        let acts: Vec<Act> = FIXTURE
            .lines()
            .flat_map(|l| c.on_line(l, &mut tools))
            .collect();
        assert!(
            acts.iter()
                .any(|a| matches!(a, Act::Connected(Some(m)) if m == "claude-x"))
        );
        let acts: Vec<_> = acts
            .into_iter()
            .filter(|a| !matches!(a, Act::Connected(_)))
            .collect();
        assert!(
            matches!(&acts[0], Act::Notice { text, .. } if text.contains("claude-x") && text.contains("slack"))
        );
        assert!(
            matches!(&acts[1], Act::Text { key, text, .. } if key == "msg_1:0" && text == "Sprawdzę kanał.")
        );
        assert!(
            matches!(&acts[2], Act::Tool { key, name, summary, .. } if key == "toolu_1" && name == "mcp__slack__read_channel" && summary.contains("zespol"))
        );
        let Act::Ask(r) = &acts[3] else {
            panic!("{:?}", acts[3])
        };
        assert_eq!(r.req.id, json!("req-7"));
        assert_eq!(r.ask.tool, "mcp__slack__read_channel");
        assert!(
            r.ask.title.starts_with("slack - read_channel"),
            "{}",
            r.ask.title
        );
        assert!(
            matches!(&acts[4], Act::ToolDone { ok: true, output, .. } if output == "3 wiadomości")
        );
        // An unsupported control request is answered with an error at once.
        assert!(
            matches!(&acts[5], Act::Write(l) if l.contains("\"error\"") && l.contains("req-8"))
        );
        assert!(matches!(&acts[6], Act::Cancel { id } if id == &json!("req-9")));
        assert!(
            matches!(&acts[7], Act::TurnDone { text: Some(t), error: None } if t.contains("montybrain-zmiany"))
        );
        assert_eq!(acts.len(), 8);
    }

    #[test]
    fn answers_and_messages_have_the_documented_shape() {
        let mut c = Claude::default();
        let start = c.start("Zadanie");
        let init: Value = serde_json::from_str(&start[0]).unwrap();
        assert_eq!(init["request"]["subtype"], "initialize");
        let user: Value = serde_json::from_str(&start[1]).unwrap();
        assert_eq!(user["message"]["content"][0]["text"], "Zadanie");
        let mut tools = HashMap::new();
        let Act::Ask(r) = c
            .on_line(FIXTURE.lines().nth(2).unwrap(), &mut tools)
            .remove(0)
        else {
            panic!()
        };
        let allow: Value = serde_json::from_str(&c.answer(&r.req, &Decision::Allow)[0]).unwrap();
        assert_eq!(allow["response"]["request_id"], "req-7");
        assert_eq!(allow["response"]["response"]["behavior"], "allow");
        assert_eq!(
            allow["response"]["response"]["updatedInput"]["channel"],
            "#zespol"
        );
        let session: Value =
            serde_json::from_str(&c.answer(&r.req, &Decision::AllowSession)[0]).unwrap();
        assert_eq!(
            session["response"]["response"]["updatedPermissions"][0]["destination"],
            "session"
        );
        let deny: Value =
            serde_json::from_str(&c.answer(&r.req, &Decision::Deny("nie".into()))[0]).unwrap();
        assert_eq!(
            deny["response"]["response"],
            json!({"behavior": "deny", "message": "nie"})
        );
        let int: Value = serde_json::from_str(&c.interrupt()[0]).unwrap();
        assert_eq!(int["request"]["subtype"], "interrupt");
    }

    #[test]
    fn file_writes_and_commands_carry_what_the_gate_checks() {
        let mut c = Claude::default();
        let mut tools = HashMap::new();
        let l = r##"{"type":"control_request","request_id":"r","request":{"subtype":"can_use_tool","tool_name":"Write","input":{"file_path":"/v/skrzynka.md","content":"x"},"tool_use_id":"t"}}"##;
        let Act::Ask(r) = c.on_line(l, &mut tools).remove(0) else {
            panic!()
        };
        assert_eq!(r.ask.paths, ["/v/skrzynka.md"]);
        let l = r##"{"type":"control_request","request_id":"r","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"osascript -e 'tell app \"Slack\"'"},"tool_use_id":"t"}}"##;
        let Act::Ask(r) = c.on_line(l, &mut tools).remove(0) else {
            panic!()
        };
        assert_eq!(r.ask.key, "Bash:osascript");
        assert!(r.ask.text.contains("Slack"));
    }

    #[test]
    fn argv_denies_writes_and_allows_reads() {
        let a = args(
            "/c/s.mcp.json",
            "/Users/a/Mój vault/",
            &["--model".into(), "opus".into()],
        );
        assert!(
            a.windows(2)
                .any(|w| w[0] == "--permission-prompt-tool" && w[1] == "stdio")
        );
        assert!(a.contains(&"Edit(//Users/a/Mój vault/**)".to_string()));
        let deny = a.iter().position(|x| x == "--disallowedTools").unwrap();
        let allow = a.iter().position(|x| x == "--allowedTools").unwrap();
        assert!(a[deny..allow].contains(&"mcp__montybrain__create_task".to_string()));
        assert!(a[allow..].contains(&"mcp__montybrain__search".to_string()));
        assert!(
            !a.iter()
                .any(|x| x == "--strict-mcp-config" || x.contains("dangerously"))
        );
    }
}
