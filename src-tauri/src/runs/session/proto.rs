//! What the two provider adaptors ([`super::claude`], [`super::codex`]) turn
//! their CLI's messages into, and what they are asked to send back.

use serde_json::Value;

use super::gate::Ask;

/// The provider's handle of one pending request (opaque to the session).
#[derive(Debug, Clone, PartialEq)]
pub struct ReqRef {
    /// The CLI's request id (Claude: `request_id`; Codex: the JSON-RPC id).
    pub id: Value,
    /// Codex: the method; Claude: `can_use_tool`.
    pub method: String,
    /// What the answer needs back (Claude: the tool input; Codex: the params).
    pub data: Value,
}

/// A question the agent asks (Codex `item/tool/requestUserInput`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Question {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<String>,
}

/// A request the person (or the gate) answers.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub req: ReqRef,
    pub ask: Ask,
    /// Questions instead of a yes/no permission.
    pub questions: Vec<Question>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    AllowSession,
    Deny(String),
    /// Answers to [`Request::questions`] (by question id).
    Answers(Vec<(String, String)>),
}

/// One thing that happened in the CLI's output.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// A line to write to the CLI's stdin (protocol housekeeping).
    Write(String),
    /// Assistant text: `key` names the message; `append` adds a streamed piece.
    Text {
        key: String,
        text: String,
        append: bool,
    },
    /// A tool call started (one compact line, details expandable).
    Tool {
        key: String,
        name: String,
        summary: String,
        detail: String,
    },
    /// A tool call ended.
    ToolDone {
        key: String,
        ok: bool,
        output: String,
    },
    /// A permission request or a question.
    Ask(Box<Request>),
    /// The CLI withdrew a pending request (interrupt, timeout).
    Cancel {
        id: Value,
    },
    TurnStarted,
    /// The turn ended; `text` is its final answer.
    TurnDone {
        text: Option<String>,
        error: Option<String>,
    },
    Notice {
        text: String,
        error: bool,
    },
}

pub const DETAIL_MAX: usize = 4_000;
pub const OUTPUT_MAX: usize = 1_500;
pub const SUMMARY_MAX: usize = 140;

/// `s` cut at `max` characters with "…".
pub fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// One line: whitespace collapsed.
pub fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A short summary of tool arguments: the telling field when there is one
/// (a command, a path, a URL, a query), else the compact JSON.
pub fn args_summary(input: &Value) -> String {
    for k in [
        "command",
        "cmd",
        "file_path",
        "notebook_path",
        "path",
        "url",
        "query",
        "pattern",
        "description",
        "text",
        "action",
    ] {
        match input.get(k) {
            Some(Value::String(s)) if !s.trim().is_empty() => {
                return clip(&one_line(s), SUMMARY_MAX);
            }
            Some(Value::Array(a)) if k == "command" => {
                let parts: Vec<&str> = a.iter().filter_map(Value::as_str).collect();
                if !parts.is_empty() {
                    return clip(&one_line(&parts.join(" ")), SUMMARY_MAX);
                }
            }
            _ => {}
        }
    }
    match input {
        Value::Null => String::new(),
        Value::Object(o) if o.is_empty() => String::new(),
        v => clip(&one_line(&v.to_string()), SUMMARY_MAX),
    }
}

/// Arguments for the expandable part: pretty JSON, shortened.
pub fn args_detail(input: &Value) -> String {
    match input {
        Value::Null => String::new(),
        Value::String(s) => clip(s, DETAIL_MAX),
        v => clip(
            &serde_json::to_string_pretty(v).unwrap_or_default(),
            DETAIL_MAX,
        ),
    }
}

/// Text of a tool result (`content` as a string or a list of text blocks).
pub fn result_text(v: &Value) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|i| match i {
                Value::String(s) => Some(s.clone()),
                Value::Object(o) => o
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        o.get("type")
                            .and_then(Value::as_str)
                            .map(|t| format!("[{t}]"))
                    }),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    clip(s.trim(), OUTPUT_MAX)
}

/// The paths a file tool would write (`file_path`, `notebook_path`, `path`, `edits[].file_path`).
pub fn written_paths(input: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for k in ["file_path", "notebook_path", "path"] {
        if let Some(s) = input.get(k).and_then(Value::as_str) {
            out.push(s.to_string());
        }
    }
    if let Some(Value::Array(edits)) = input.get("edits") {
        for e in edits {
            if let Some(s) = e.get("file_path").and_then(Value::as_str) {
                out.push(s.to_string());
            }
        }
    }
    out
}

/// What "for this session" remembers for a shell command: its program.
pub fn command_key(prefix: &str, command: &str) -> String {
    let first = command
        .split_whitespace()
        .find(|w| !w.contains('='))
        .unwrap_or("")
        .rsplit('/')
        .next()
        .unwrap_or("");
    format!("{prefix}:{first}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summaries_pick_the_telling_field() {
        assert_eq!(
            args_summary(&json!({"command": "ls -la\n/tmp"})),
            "ls -la /tmp"
        );
        assert_eq!(
            args_summary(&json!({"file_path": "/a/b.md", "content": "x"})),
            "/a/b.md"
        );
        assert_eq!(
            args_summary(&json!({"command": ["bash", "-lc", "echo hi"]})),
            "bash -lc echo hi"
        );
        assert_eq!(args_summary(&json!({})), "");
        assert!(args_summary(&json!({"x": "y".repeat(500)})).ends_with('…'));
        assert_eq!(
            result_text(&json!([{"type": "text", "text": "ok"}, {"type": "image"}])),
            "ok\n[image]"
        );
        assert_eq!(
            command_key("Bash", "FOO=1 /usr/bin/osascript -e x"),
            "Bash:osascript"
        );
        assert_eq!(
            written_paths(&json!({"edits": [{"file_path": "/x"}], "file_path": "/y"})),
            ["/y", "/x"]
        );
    }
}
