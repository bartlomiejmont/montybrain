//! JSONL output of `claude -p --output-format stream-json --verbose` and
//! `codex exec --json`, turned into log lines and a final outcome.
//! Unknown event types are ignored; unparseable lines become (a few) notes.

use serde_json::Value;

use super::{EventKind, ProviderId};

const TOOL_ARGS_CHARS: usize = 160;
const COMMAND_CHARS: usize = 200;
const NOTE_CHARS: usize = 300;
/// After this many unparseable lines the rest are skipped silently.
const MAX_GARBAGE_NOTES: usize = 5;
/// Bounds of what the parser keeps across lines.
pub const MAX_FINAL_CHARS: usize = 200_000;
pub const MAX_ERROR_CHARS: usize = 4_000;
/// Codex item ids remembered for de-duplicating `item.started` / `item.completed`.
const MAX_SHOWN_IDS: usize = 256;

pub type Line = (EventKind, String);

/// What the stream said about the end of the run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The final answer (Claude: `result.result`; Codex: last `agent_message`).
    pub final_text: Option<String>,
    /// The CLI reported a failure.
    pub failed: bool,
    /// The CLI's own error message, if it gave one.
    pub cli_error: Option<String>,
}

/// Cuts `s` to `max` characters, marking the cut.
pub fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    match s.char_indices().nth(max) {
        None => s.to_string(),
        Some((i, _)) => format!("{}…", &s[..i]),
    }
}

/// One line of text (newlines folded into spaces), clipped.
fn one_line(s: &str, max: usize) -> String {
    clip(&s.split_whitespace().collect::<Vec<_>>().join(" "), max)
}

/// `name {"compact":"args"}`, or just `name` for empty arguments.
fn tool_line(name: &str, args: Option<&Value>) -> String {
    let args = match args {
        None | Some(Value::Null) => String::new(),
        Some(Value::Object(m)) if m.is_empty() => String::new(),
        Some(Value::String(s)) if s.trim().is_empty() || s.trim() == "{}" => String::new(),
        // Some CLIs pass arguments as a JSON string.
        Some(Value::String(s)) => serde_json::from_str::<Value>(s)
            .map(|v| v.to_string())
            .unwrap_or_else(|_| s.clone()),
        Some(v) => v.to_string(),
    };
    if args.is_empty() {
        name.to_string()
    } else {
        format!("{name} {}", one_line(&args, TOOL_ARGS_CHARS))
    }
}

fn str_at<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn nonempty(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub struct Parser {
    provider: ProviderId,
    outcome: Outcome,
    garbage: usize,
    /// Codex: ids of items already shown at `item.started` (the most recent ones).
    shown: std::collections::VecDeque<String>,
    /// Codex: last top-level `error` message (used only when the run fails).
    last_error: Option<String>,
}

impl Parser {
    pub fn new(provider: ProviderId) -> Self {
        Parser {
            provider,
            outcome: Outcome::default(),
            garbage: 0,
            shown: Default::default(),
            last_error: None,
        }
    }

    pub fn outcome(&self) -> Outcome {
        let mut o = self.outcome.clone();
        if o.cli_error.is_none() {
            o.cli_error = self.last_error.clone();
        }
        o
    }

    fn set_final(&mut self, text: Option<String>) {
        self.outcome.final_text = text.map(|t| clip(&t, MAX_FINAL_CHARS));
    }

    fn set_cli_error(&mut self, msg: String) {
        self.outcome.cli_error = Some(clip(&msg, MAX_ERROR_CHARS));
    }

    /// Parses one stdout line.
    pub fn line(&mut self, line: &str) -> Vec<Line> {
        let line = line.trim();
        if line.is_empty() {
            return Vec::new();
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v @ Value::Object(_)) => v,
            _ => {
                self.garbage += 1;
                return if self.garbage <= MAX_GARBAGE_NOTES {
                    vec![(
                        EventKind::Note,
                        format!("Pominięto linię spoza JSON: {}", one_line(line, 120)),
                    )]
                } else {
                    Vec::new()
                };
            }
        };
        match self.provider {
            ProviderId::Claude => self.claude(&v),
            ProviderId::Codex => self.codex(&v),
        }
    }

    // --- Claude Code -----------------------------------------------------------

    fn claude(&mut self, v: &Value) -> Vec<Line> {
        let mut out = Vec::new();
        match str_at(v, "type") {
            Some("system") if str_at(v, "subtype") == Some("init") => {
                let mut s = String::from("Sesja Claude Code");
                if let Some(m) = str_at(v, "model") {
                    s.push_str(&format!(", model {m}"));
                }
                out.push((EventKind::Note, s));
                let montybrain = v
                    .get("mcp_servers")
                    .and_then(Value::as_array)
                    .and_then(|a| a.iter().find(|s| str_at(s, "name") == Some("montybrain")));
                out.push((
                    EventKind::Note,
                    match montybrain {
                        Some(s) => format!(
                            "Serwer MCP montybrain: {}",
                            str_at(s, "status").unwrap_or("?")
                        ),
                        None => "Serwer MCP montybrain nie jest skonfigurowany w Claude Code."
                            .to_string(),
                    },
                ));
            }
            Some("assistant") => {
                for block in content(v) {
                    match str_at(block, "type") {
                        Some("text") => {
                            if let Some(t) = nonempty(str_at(block, "text")) {
                                out.push((EventKind::Text, t));
                            }
                        }
                        Some("tool_use") => {
                            let name = str_at(block, "name").unwrap_or("?");
                            out.push((EventKind::Tool, claude_tool(name, block.get("input"))));
                        }
                        _ => {}
                    }
                }
            }
            Some("user") => {
                for block in content(v) {
                    if str_at(block, "type") == Some("tool_result")
                        && block.get("is_error").and_then(Value::as_bool) == Some(true)
                    {
                        let text = tool_result_text(block.get("content"));
                        out.push((
                            EventKind::Note,
                            format!("Narzędzie zwróciło błąd: {}", one_line(&text, NOTE_CHARS)),
                        ));
                    }
                }
            }
            Some("result") => {
                let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                let subtype = str_at(v, "subtype").unwrap_or("");
                let result = nonempty(str_at(v, "result"));
                if let Some(denials) = v.get("permission_denials").and_then(Value::as_array)
                    && !denials.is_empty()
                {
                    let list: Vec<String> = denials
                        .iter()
                        .map(|d| {
                            claude_tool(str_at(d, "tool_name").unwrap_or("?"), d.get("tool_input"))
                        })
                        .collect();
                    out.push((
                        EventKind::Note,
                        clip(&format!("Odmówiono uprawnień: {}", list.join("; ")), 1000),
                    ));
                }
                if !is_error && subtype == "success" {
                    self.set_final(result);
                } else {
                    self.outcome.failed = true;
                    let errors: Vec<String> = v
                        .get("errors")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(|e| nonempty(e.as_str())).collect())
                        .unwrap_or_default();
                    self.set_cli_error(match (result, errors.is_empty()) {
                        (Some(r), _) => r,
                        (None, false) => errors.join("\n"),
                        (None, true) if subtype == "error_max_turns" => {
                            "Claude Code przekroczył limit tur (error_max_turns).".into()
                        }
                        (None, true) => format!("Claude Code zgłosił błąd ({subtype})."),
                    });
                }
            }
            _ => {}
        }
        out
    }

    // --- Codex ---------------------------------------------------------------

    fn codex(&mut self, v: &Value) -> Vec<Line> {
        let mut out = Vec::new();
        match str_at(v, "type") {
            Some("thread.started") => out.push((
                EventKind::Note,
                format!("Sesja Codex {}", str_at(v, "thread_id").unwrap_or_default())
                    .trim()
                    .to_string(),
            )),
            Some("turn.failed") => {
                let msg = v
                    .get("error")
                    .and_then(|e| nonempty(str_at(e, "message")))
                    .unwrap_or_else(|| "Codex przerwał turę.".into());
                self.outcome.failed = true;
                self.set_cli_error(msg.clone());
                out.push((EventKind::Error, msg));
            }
            Some("error") => {
                if let Some(msg) = nonempty(str_at(v, "message")) {
                    self.last_error = Some(clip(&msg, MAX_ERROR_CHARS));
                    out.push((EventKind::Error, msg));
                }
            }
            Some(kind @ ("item.started" | "item.completed")) => {
                let Some(item) = v.get("item") else {
                    return out;
                };
                let started = kind == "item.started";
                let id = str_at(item, "id").unwrap_or_default().to_string();
                match str_at(item, "type") {
                    Some("agent_message") if !started => {
                        if let Some(t) = nonempty(str_at(item, "text")) {
                            self.set_final(Some(t.clone()));
                            out.push((EventKind::Text, t));
                        }
                    }
                    Some(t @ ("command_execution" | "mcp_tool_call" | "web_search")) => {
                        let already = !id.is_empty() && self.shown.contains(&id);
                        if !already {
                            out.push((EventKind::Tool, codex_tool(t, item)));
                            if !id.is_empty() {
                                if self.shown.len() >= MAX_SHOWN_IDS {
                                    self.shown.pop_front();
                                }
                                self.shown.push_back(id);
                            }
                        }
                        if !started && let Some(problem) = codex_item_failure(t, item) {
                            out.push((EventKind::Note, problem));
                        }
                    }
                    Some("error") if !started => {
                        if let Some(m) = nonempty(str_at(item, "message")) {
                            out.push((
                                EventKind::Note,
                                format!("Ostrzeżenie: {}", one_line(&m, NOTE_CHARS)),
                            ));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        out
    }
}

fn content(v: &Value) -> impl Iterator<Item = &Value> {
    v.get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn tool_result_text(c: Option<&Value>) -> String {
    match c {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|b| str_at(b, "text"))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn claude_tool(name: &str, input: Option<&Value>) -> String {
    if name == "Bash"
        && let Some(cmd) = input.and_then(|i| str_at(i, "command"))
    {
        return format!("Bash: {}", one_line(cmd, COMMAND_CHARS));
    }
    // Our tools under the server name, or the pre-rename `morwisko` one
    // (still registered in a CLI that was not reinstalled).
    let short = name
        .strip_prefix("mcp__montybrain__")
        .or_else(|| name.strip_prefix("mcp__morwisko__"))
        .unwrap_or(name);
    tool_line(short, input)
}

fn codex_tool(kind: &str, item: &Value) -> String {
    match kind {
        "command_execution" => format!(
            "$ {}",
            one_line(str_at(item, "command").unwrap_or("?"), COMMAND_CHARS)
        ),
        "mcp_tool_call" => {
            let tool = str_at(item, "tool").unwrap_or("?");
            let name = match str_at(item, "server") {
                Some("montybrain" | "morwisko") | None => tool.to_string(),
                Some(server) => format!("{server}/{tool}"),
            };
            tool_line(&name, item.get("arguments"))
        }
        _ => tool_line("web_search", item.get("query")),
    }
}

/// A note for a finished tool item that did not succeed.
fn codex_item_failure(kind: &str, item: &Value) -> Option<String> {
    match kind {
        "command_execution" => {
            let code = item.get("exit_code").and_then(Value::as_i64);
            let failed = str_at(item, "status") == Some("failed") || code.is_some_and(|c| c != 0);
            failed.then(|| match code {
                Some(c) => format!("Polecenie zakończone kodem {c}"),
                None => "Polecenie nie powiodło się".to_string(),
            })
        }
        "mcp_tool_call" => {
            let msg = item.get("error").and_then(|e| match e {
                Value::Null => None,
                Value::String(s) => nonempty(Some(s)),
                other => nonempty(str_at(other, "message")).or_else(|| Some(other.to_string())),
            });
            let failed = str_at(item, "status") == Some("failed") || msg.is_some();
            failed.then(|| {
                format!(
                    "Narzędzie {} zwróciło błąd: {}",
                    str_at(item, "tool").unwrap_or("?"),
                    one_line(msg.as_deref().unwrap_or("bez opisu"), NOTE_CHARS)
                )
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use EventKind::*;

    fn run(provider: ProviderId, lines: &[&str]) -> (Vec<Line>, Outcome) {
        let mut p = Parser::new(provider);
        let mut out = Vec::new();
        for l in lines {
            out.extend(p.line(l));
        }
        (out, p.outcome())
    }

    #[test]
    fn tools_of_the_pre_rename_server_name_are_shortened_too() {
        assert_eq!(claude_tool("mcp__morwisko__overview", None), "overview");
        assert_eq!(claude_tool("mcp__montybrain__overview", None), "overview");
        assert_eq!(
            codex_tool(
                "mcp_tool_call",
                &serde_json::json!({"server": "morwisko", "tool": "search"})
            ),
            "search"
        );
    }

    #[test]
    fn claude_success_stream() {
        let (lines, outcome) = run(
            ProviderId::Claude,
            &[
                r#"{"type":"system","subtype":"init","model":"claude-x","mcp_servers":[{"name":"gmail","status":"failed"},{"name":"montybrain","status":"connected"}],"tools":["mcp__montybrain__overview"]}"#,
                r#"{"type":"rate_limit_event","rate_limit_info":{}}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"hm"},{"type":"text","text":"Sprawdzam."},{"type":"tool_use","id":"t1","name":"mcp__montybrain__list_tasks","input":{"project":"beeking"}}]}}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t0","name":"mcp__montybrain__overview","input":{}}]}}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"git log\n--oneline -15","description":"x"}}]}}"#,
                r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"3 task(s)"}]}}"#,
                r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","is_error":true,"content":[{"type":"text","text":"Permission denied"}]}]}}"#,
                "",
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Raport:\nWszystko gra."}]}}"#,
                r#"{"type":"result","subtype":"success","is_error":false,"result":"Raport:\nWszystko gra.","num_turns":4,"permission_denials":[{"tool_name":"Bash","tool_use_id":"t2","tool_input":{"command":"git -C /x log"}}]}"#,
            ],
        );
        assert_eq!(
            lines,
            vec![
                (Note, "Sesja Claude Code, model claude-x".to_string()),
                (Note, "Serwer MCP montybrain: connected".to_string()),
                (Text, "Sprawdzam.".to_string()),
                (Tool, r#"list_tasks {"project":"beeking"}"#.to_string()),
                (Tool, "overview".to_string()),
                (Tool, "Bash: git log --oneline -15".to_string()),
                (
                    Note,
                    "Narzędzie zwróciło błąd: Permission denied".to_string()
                ),
                (Text, "Raport:\nWszystko gra.".to_string()),
                (Note, "Odmówiono uprawnień: Bash: git -C /x log".to_string()),
            ]
        );
        assert_eq!(
            outcome,
            Outcome {
                final_text: Some("Raport:\nWszystko gra.".into()),
                failed: false,
                cli_error: None,
            }
        );
    }

    #[test]
    fn claude_failures() {
        let (_, o) = run(
            ProviderId::Claude,
            &[
                r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}"#,
            ],
        );
        assert!(o.failed);
        assert_eq!(
            o.cli_error.as_deref(),
            Some("Not logged in · Please run /login")
        );
        assert_eq!(o.final_text, None);

        let (_, o) = run(
            ProviderId::Claude,
            &[r#"{"type":"result","subtype":"error_max_turns","is_error":true}"#],
        );
        assert!(o.cli_error.unwrap().contains("error_max_turns"));
        let (_, o) = run(
            ProviderId::Claude,
            &[
                r#"{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["boom"]}"#,
            ],
        );
        assert_eq!(o.cli_error.as_deref(), Some("boom"));

        let (lines, o) = run(
            ProviderId::Claude,
            &[r#"{"type":"system","subtype":"init","mcp_servers":[]}"#],
        );
        assert_eq!(o, Outcome::default());
        assert!(lines[1].1.contains("nie jest skonfigurowany"));
    }

    #[test]
    fn codex_stream_with_warnings() {
        let (lines, outcome) = run(
            ProviderId::Codex,
            &[
                r#"{"type":"thread.started","thread_id":"019a-abc"}"#,
                r#"{"type":"turn.started"}"#,
                r#"{"type":"item.completed","item":{"id":"item_0","type":"error","message":"`experimental_use_rmcp_client` is deprecated"}}"#,
                r#"{"type":"item.completed","item":{"id":"item_1","type":"reasoning","text":"thinking"}}"#,
                r#"{"type":"item.started","item":{"id":"item_2","type":"mcp_tool_call","server":"montybrain","tool":"overview","arguments":{},"result":null,"error":null,"status":"in_progress"}}"#,
                r#"{"type":"item.completed","item":{"id":"item_2","type":"mcp_tool_call","server":"montybrain","tool":"overview","arguments":{},"result":{"content":[]},"error":null,"status":"completed"}}"#,
                r#"{"type":"item.started","item":{"id":"item_3","type":"mcp_tool_call","server":"montybrain","tool":"list_tasks","arguments":{"project":"beeking"},"status":"in_progress"}}"#,
                r#"{"type":"item.completed","item":{"id":"item_3","type":"mcp_tool_call","server":"montybrain","tool":"list_tasks","arguments":{"project":"beeking"},"error":{"message":"Unknown project"},"status":"failed"}}"#,
                r#"{"type":"item.completed","item":{"id":"item_4","type":"mcp_tool_call","server":"gmail","tool":"search","arguments":"{\"q\":\"x\"}","status":"completed"}}"#,
                r#"{"type":"item.started","item":{"id":"item_5","type":"command_execution","command":"/bin/zsh -lc 'git log --oneline'","aggregated_output":"","status":"in_progress"}}"#,
                r#"{"type":"item.completed","item":{"id":"item_5","type":"command_execution","command":"/bin/zsh -lc 'git log --oneline'","aggregated_output":"fatal","exit_code":128,"status":"failed"}}"#,
                r#"{"type":"item.completed","item":{"id":"item_6","type":"agent_message","text":"Najpierw sprawdzę zadania."}}"#,
                "Reading additional input from stdin...",
                r#"{"type":"item.completed","item":{"id":"item_7","type":"agent_message","text":"Raport:\nGotowe."}}"#,
                r#"{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":2}}"#,
            ],
        );
        assert_eq!(
            lines,
            vec![
                (Note, "Sesja Codex 019a-abc".to_string()),
                (
                    Note,
                    "Ostrzeżenie: `experimental_use_rmcp_client` is deprecated".to_string()
                ),
                (Tool, "overview".to_string()),
                (Tool, r#"list_tasks {"project":"beeking"}"#.to_string()),
                (
                    Note,
                    "Narzędzie list_tasks zwróciło błąd: Unknown project".to_string()
                ),
                (Tool, r#"gmail/search {"q":"x"}"#.to_string()),
                (Tool, "$ /bin/zsh -lc 'git log --oneline'".to_string()),
                (Note, "Polecenie zakończone kodem 128".to_string()),
                (Text, "Najpierw sprawdzę zadania.".to_string()),
                (
                    Note,
                    "Pominięto linię spoza JSON: Reading additional input from stdin..."
                        .to_string()
                ),
                (Text, "Raport:\nGotowe.".to_string()),
            ]
        );
        assert_eq!(
            outcome,
            Outcome {
                final_text: Some("Raport:\nGotowe.".into()),
                failed: false,
                cli_error: None,
            }
        );
    }

    #[test]
    fn codex_failed_turn() {
        let (lines, o) = run(
            ProviderId::Codex,
            &[
                r#"{"type":"thread.started","thread_id":"t"}"#,
                r#"{"type":"error","message":"stream disconnected; retrying 1/5"}"#,
                r#"{"type":"turn.failed","error":{"message":"You've hit your usage limit."}}"#,
            ],
        );
        assert!(o.failed);
        assert_eq!(o.cli_error.as_deref(), Some("You've hit your usage limit."));
        assert_eq!(
            lines.last().unwrap(),
            &(Error, "You've hit your usage limit.".to_string())
        );

        // A top-level error alone is not a failure, but is the message if the run fails.
        let (_, o) = run(
            ProviderId::Codex,
            &[r#"{"type":"error","message":"auth required"}"#],
        );
        assert!(!o.failed);
        assert_eq!(o.cli_error.as_deref(), Some("auth required"));
        assert_eq!(o.final_text, None);
    }

    #[test]
    fn retained_strings_are_bounded() {
        let big = "ą".repeat(MAX_FINAL_CHARS + 10);
        let mut p = Parser::new(ProviderId::Codex);
        p.line(
            &serde_json::json!({"type":"item.completed","item":{"id":"m","type":"agent_message","text":big}})
                .to_string(),
        );
        p.line(&serde_json::json!({"type":"error","message":"e".repeat(10_000)}).to_string());
        for i in 0..1000 {
            p.line(&format!(
                r#"{{"type":"item.started","item":{{"id":"i{i}","type":"mcp_tool_call","tool":"overview","arguments":{{}}}}}}"#
            ));
        }
        assert!(p.shown.len() <= MAX_SHOWN_IDS);
        let o = p.outcome();
        assert_eq!(o.final_text.unwrap().chars().count(), MAX_FINAL_CHARS + 1);
        assert_eq!(o.cli_error.unwrap().chars().count(), MAX_ERROR_CHARS + 1);

        let mut p = Parser::new(ProviderId::Claude);
        p.line(
            &serde_json::json!({"type":"result","subtype":"x","is_error":true,"result":"r".repeat(10_000)})
                .to_string(),
        );
        assert_eq!(
            p.outcome().cli_error.unwrap().chars().count(),
            MAX_ERROR_CHARS + 1
        );
    }

    #[test]
    fn garbage_is_bounded_and_clipping_is_char_safe() {
        let mut p = Parser::new(ProviderId::Claude);
        let notes: usize = (0..20).map(|_| p.line("nie json").len()).sum();
        assert_eq!(notes, MAX_GARBAGE_NOTES);
        assert!(
            p.line("[1,2]").is_empty(),
            "non-object JSON counts as garbage"
        );
        assert_eq!(clip("ąęść", 2), "ąę…");
        assert_eq!(clip("ab", 2), "ab");
        let long = format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"mcp__montybrain__search","input":{{"query":"{}"}}}}]}}}}"#,
            "x".repeat(500)
        );
        let l = Parser::new(ProviderId::Claude).line(&long);
        assert!(l[0].1.starts_with("search {\"query\":\"xxx") && l[0].1.ends_with('…'));
        assert!(l[0].1.chars().count() <= "search ".len() + TOOL_ARGS_CHARS + 1);
    }
}
