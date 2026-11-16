//! Read-only CLI handshakes; never create a thread or send a user/model turn.
//! Checked against codex-cli 0.169.0 generate-ts + live responses and Claude
//! Code 2.1.325's SDK initialize response. Raw output never reaches the UI.
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use super::{
    ProviderId,
    session::telemetry::{LimitSnapshot, codex_limits},
};
use serde_json::{Value, json};

pub const CODEX_FALLBACK: &[&str] = &[
    "gpt-6.1-sol",
    "gpt-6-astra",
    "gpt-6.1-terra",
    "gpt-6-sol",
    "gpt-6-luna",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
];
pub const CLAUDE_FALLBACK: &[&str] = &["opus", "fable", "sonnet", "haiku"];

pub fn fallback(id: ProviderId) -> Vec<String> {
    match id {
        ProviderId::Codex => CODEX_FALLBACK,
        ProviderId::Claude => CLAUDE_FALLBACK,
    }
    .iter()
    .map(|s| (*s).into())
    .collect()
}

struct Client {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<Value>,
    deadline: Instant,
}
impl Drop for Client {
    fn drop(&mut self) {
        super::runner::signal_group(self.child.id(), super::runner::Sig::Kill);
        let _ = self.child.wait();
    }
}
impl Client {
    fn start(path: &Path, id: ProviderId, env: &[(OsString, OsString)]) -> Result<Self, String> {
        let mut cmd = Command::new(path);
        match id {
            ProviderId::Codex => {
                cmd.arg("app-server");
            }
            ProviderId::Claude => {
                cmd.args([
                    "-p",
                    "--input-format",
                    "stream-json",
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--safe-mode",
                    "--strict-mcp-config",
                    "--mcp-config",
                    "{\"mcpServers\":{}}",
                ]);
            }
        }
        let mut child = cmd
            .current_dir(std::env::temp_dir())
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|_| "Nie udało się uruchomić odczytu CLI.")?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, lines) = mpsc::sync_channel(16);
        std::thread::spawn(move || {
            // Cap the entire probe, including a malformed unterminated line.
            let reader = BufReader::new(stdout.take(2 * 1024 * 1024));
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if let Ok(v) = serde_json::from_str(&line) {
                    if tx.send(v).is_err() {
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            lines,
            deadline: Instant::now() + Duration::from_secs(8),
        })
    }
    fn send(&mut self, v: Value) -> Result<(), String> {
        writeln!(self.stdin, "{v}")
            .and_then(|_| self.stdin.flush())
            .map_err(|_| "CLI przerwało odczyt statusu.".into())
    }
    fn response(&self, id: Value, claude: bool) -> Result<Value, String> {
        loop {
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            let v = self
                .lines
                .recv_timeout(remaining)
                .map_err(|_| "CLI nie odpowiedziało na odczyt statusu w ciągu 8 s.")?;
            if claude {
                if v.pointer("/response/request_id") == Some(&id) {
                    if v.pointer("/response/subtype").and_then(Value::as_str) == Some("success") {
                        return Ok(v["response"]["response"].clone());
                    }
                    return Err("CLI nie udostępniło listy modeli.".into());
                }
            } else if v.get("id") == Some(&id) {
                return v
                    .get("result")
                    .cloned()
                    .ok_or_else(|| "CLI nie udostępniło danych konta.".into());
            }
        }
    }
}

pub struct Status {
    pub models: Vec<String>,
    pub limits: Result<LimitSnapshot, String>,
}

pub fn read(path: &Path, id: ProviderId, env: &[(OsString, OsString)]) -> Result<Status, String> {
    let mut client = Client::start(path, id, env)?;
    if id == ProviderId::Claude {
        client.send(json!({"type":"control_request","request_id":"models","request":{"subtype":"initialize"}}))?;
        let result = client.response(json!("models"), true)?;
        let models = models(&result, id);
        drop(client);
        return Ok(Status {
            models,
            limits: read_claude_usage(path, env),
        });
    }
    client.send(json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"montybrain_status","version":env!("CARGO_PKG_VERSION")}}}))?;
    client.response(json!(1), false)?;
    client.send(json!({"method":"initialized"}))?;
    client.send(json!({"id":2,"method":"account/rateLimits/read"}))?;
    let limits = client.response(json!(2), false).and_then(|v| {
        account_limits(&v).ok_or_else(|| "CLI nie podało wartości limitów konta.".into())
    });
    let mut catalog = Vec::new();
    let mut cursor = Value::Null;
    // Bounded pagination, under the same overall deadline.
    for page in 0..10 {
        let request_id = page + 3;
        client.send(json!({"id":request_id,"method":"model/list","params":{"limit":100,"cursor":cursor,"includeHidden":false}}))?;
        let Ok(v) = client.response(json!(request_id), false) else {
            break;
        };
        for model in models(&v, id) {
            if !catalog.contains(&model) {
                catalog.push(model);
            }
        }
        cursor = v["nextCursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    Ok(Status {
        models: catalog,
        limits,
    })
}

/// The installed CLI exposes /usage as a non-interactive local command.
/// --max-turns 0 is an additional guard: this probe must never run inference.
fn read_claude_usage(path: &Path, env: &[(OsString, OsString)]) -> Result<LimitSnapshot, String> {
    let mut cmd = Command::new(path);
    cmd.args([
        "-p",
        "/usage",
        "--max-turns",
        "0",
        "--output-format",
        "json",
        "--safe-mode",
        "--strict-mcp-config",
        "--mcp-config",
        "{\"mcpServers\":{}}",
        "--tools",
        "",
    ])
    .current_dir(std::env::temp_dir())
    .env_clear()
    .envs(env.iter().map(|(k, v)| (k, v)));
    let out =
        super::proc::run_bounded(cmd, Duration::from_secs(8), 256 * 1024, 4 * 1024, &|| false)
            .map_err(|_| "Nie udało się odczytać /usage w Claude Code.")?;
    if !out.success() || out.stdout_truncated {
        return Err("Claude Code nie udostępniło stanu limitu przez /usage. Ostatni raport z sesji pozostaje dostępny.".into());
    }
    serde_json::from_slice::<Value>(&out.stdout).ok().and_then(|v| claude_usage(&v))
        .ok_or_else(|| "Claude Code nie podało rozpoznawalnych limitów w /usage. Stan z sesji pojawi się po raporcie CLI.".into())
}

pub fn claude_usage(v: &Value) -> Option<LimitSnapshot> {
    if v["local_command"] != "usage" || v["is_error"] != false || v["num_turns"] != 0 {
        return None;
    }
    let text = v["result"].as_str()?;
    let mut limits = Vec::new();
    for line in text.lines() {
        let Some((name, value)) = line.split_once(": ") else {
            continue;
        };
        let (id, minutes) = match name {
            "Current session" => ("claude:five_hour".to_string(), 300),
            "Current week (all models)" => ("claude:seven_day".to_string(), 10080),
            name if name.starts_with("Current week (") && name.ends_with(')') => {
                let model = name[14..name.len() - 1].trim_end_matches(" only");
                if !matches!(
                    model.to_lowercase().as_str(),
                    "opus" | "sonnet" | "haiku" | "fable"
                ) {
                    continue;
                }
                (format!("claude:seven_day_{}", model.to_lowercase()), 10080)
            }
            _ => continue,
        };
        let Some((percent, suffix)) = value.split_once("% used") else {
            continue;
        };
        let Some(used) = percent
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
        else {
            continue;
        };
        let reset = suffix
            .trim()
            .strip_prefix("· resets ")
            .and_then(claude_reset_label);
        if limits
            .iter()
            .any(|l: &super::session::telemetry::Limit| l.id == id)
        {
            continue;
        }
        limits.push(super::session::telemetry::Limit {
            id,
            used_percent: used,
            reported_at: super::session::telemetry::timestamp(),
            window_minutes: Some(minutes),
            resets_at: None,
            reset_label: reset,
        });
    }
    (!limits.is_empty()).then(|| LimitSnapshot::new(limits, None))
}

fn claude_reset_label(text: &str) -> Option<String> {
    // The CLI formats e.g. "Nov 16 at 1:19am (Europe/Warsaw)". No year is
    // supplied. Keep a neutral numeric date, time and the original zone.
    let fields: Vec<_> = text.split_whitespace().collect();
    if fields.len() != 5 || fields[2] != "at" {
        return None;
    }
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|m| *m == fields[0])?
        + 1;
    let day: u32 = fields[1].parse().ok()?;
    if !(1..=31).contains(&day) {
        return None;
    }
    let (time, pm) = fields[3]
        .strip_suffix("am")
        .map(|s| (s, false))
        .or_else(|| fields[3].strip_suffix("pm").map(|s| (s, true)))?;
    let (hour, minute) = time.split_once(':')?;
    let hour: u32 = hour.parse().ok()?;
    let minute: u32 = minute.parse().ok()?;
    if !(1..=12).contains(&hour) || minute > 59 {
        return None;
    }
    let zone = fields[4];
    if !zone.starts_with('(')
        || !zone.ends_with(')')
        || zone.len() > 80
        || !zone
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "()/+-_:".contains(c))
    {
        return None;
    }
    Some(format!(
        "{day:02}.{month:02}, {:02}:{minute:02} {zone}",
        hour % 12 + if pm { 12 } else { 0 }
    ))
}

pub fn models(v: &Value, id: ProviderId) -> Vec<String> {
    let (key, field) = match id {
        ProviderId::Codex => ("data", "model"),
        ProviderId::Claude => ("models", "value"),
    };
    let mut out = Vec::new();
    for m in v[key].as_array().into_iter().flatten() {
        if m["hidden"].as_bool() == Some(true) {
            continue;
        }
        if let Some(s) = m[field].as_str().filter(|s| *s != "default") {
            if let Ok(Some(s)) = super::agent::check_model(id, s) {
                if !out.contains(&s) {
                    out.push(s);
                }
            }
        }
    }
    out
}

pub fn account_limits(v: &Value) -> Option<LimitSnapshot> {
    let mut limits = Vec::new();
    if let Some(buckets) = v["rateLimitsByLimitId"].as_object() {
        for (id, bucket) in buckets {
            let mut bucket = bucket.clone();
            if bucket["limitId"].is_null() {
                bucket["limitId"] = json!(id);
            }
            limits.extend(codex_limits(&json!({"rateLimits":bucket})));
        }
    }
    if limits.is_empty() {
        limits = codex_limits(v);
    }
    (!limits.is_empty()).then(|| LimitSnapshot::new(limits, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn claude_usage_accepts_only_local_zero_turn_reports_and_known_quota_lines() {
        let mut v = json!({"local_command":"usage","is_error":false,"num_turns":0,"result":"Current session: 5% used · resets Nov 16 at 1:19am (Europe/Warsaw)\nCurrent week (all models): 96% used · resets Nov 16 at 2:59am (Europe/Warsaw)\nCurrent week (Fable): 71% used · resets Nov 16 at 2:59am (Europe/Warsaw)\nLast 24h · 99 requests\n  86% Start fresh"});
        let snapshot = claude_usage(&v).unwrap();
        let sonnet = json!({"local_command":"usage","is_error":false,"num_turns":0,"result":"Current week (Sonnet only): 42% used"});
        assert_eq!(
            claude_usage(&sonnet).unwrap().limits[0].id,
            "claude:seven_day_sonnet"
        );
        assert_eq!(snapshot.limits.len(), 3);
        assert_eq!(snapshot.limits[0].window_minutes, Some(300));
        assert_eq!(
            snapshot.limits[0].reset_label.as_deref(),
            Some("16.11, 01:19 (Europe/Warsaw)")
        );
        assert_eq!(snapshot.limits[2].id, "claude:seven_day_fable");
        assert_eq!(snapshot.limits[2].used_percent, 71.0);
        assert!(snapshot.limits[0].resets_at.is_none());
        v["num_turns"] = json!(1);
        assert!(claude_usage(&v).is_none());
        v["num_turns"] = json!(0);
        v["local_command"] = json!(null);
        assert!(claude_usage(&v).is_none());
        v["local_command"] = json!("usage");
        v["result"] = json!("Current session: 101% used\nCurrent week (all models): NaN% used");
        assert!(claude_usage(&v).is_none());
        assert_eq!(
            claude_reset_label("Dec 31 at 12:00am (UTC)"),
            Some("31.12, 00:00 (UTC)".into())
        );
        assert_eq!(
            claude_reset_label("Jan 1 at 12:01pm (UTC)"),
            Some("01.01, 12:01 (UTC)".into())
        );
        assert!(claude_reset_label("unknown").is_none());
        assert!(claude_reset_label("Jan 32 at 1:00pm (UTC)").is_none());
    }
    #[test]
    fn probe_uses_only_handshake_and_read_methods_and_follows_catalog_pages() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("codex");
        std::fs::write(&script, r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) echo '{"id":1,"result":{}}' ;;
    *'"method":"initialized"'*) : ;;
    *'"method":"account/rateLimits/read"'*) echo '{"id":2,"result":{"rateLimits":{"primary":{"usedPercent":12,"windowDurationMins":300}}}}' ;;
    *'"method":"model/list"'*'"id":3'*|*'"id":3'*'"method":"model/list"'*) echo '{"id":3,"result":{"data":[{"model":"gpt-6-sol"}],"nextCursor":"page-2"}}' ;;
    *'"method":"model/list"'*'"id":4'*|*'"id":4'*'"method":"model/list"'*) echo '{"id":4,"result":{"data":[{"model":"gpt-6-luna"}],"nextCursor":null}}' ;;
    *) exit 1 ;;
  esac
done
"#).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let result = read(&script, ProviderId::Codex, &[]).unwrap();
        assert_eq!(result.models, ["gpt-6-sol", "gpt-6-luna"]);
        assert_eq!(result.limits.unwrap().limits[0].used_percent, 12.0);
    }
    #[test]
    fn catalogs_exclude_hidden_invalid_duplicate_and_cli_default() {
        assert_eq!(
            models(
                &json!({"data":[{"model":"gpt-6-sol"},{"model":"hidden","hidden":true},{"model":"gpt-6-sol"},{"model":"bad\nname"},{}]}),
                ProviderId::Codex
            ),
            ["gpt-6-sol"]
        );
        assert_eq!(
            models(
                &json!({"models":[{"value":"default"},{"value":"opus"},{"value":"custom-model"}]}),
                ProviderId::Claude
            ),
            ["opus", "custom-model"]
        );
    }
    #[test]
    fn account_limits_use_all_buckets_without_duplicating_legacy_view() {
        let w =
            json!({"primary":{"usedPercent":25,"windowDurationMins":300,"resetsAt":1800000000}});
        let result =
            account_limits(&json!({"rateLimits":w,"rateLimitsByLimitId":{"codex":w,"other":w}}))
                .unwrap();
        assert_eq!(result.limits.len(), 2);
        assert!(result.limits.iter().any(|l| l.id == "other:primary"));
        assert!(result.checked_at > 0);
        assert!(account_limits(&json!({"rateLimits":{"primary":{"usedPercent":200}}})).is_none());
        assert!(account_limits(&json!({})).is_none());
        assert_eq!(
            account_limits(&json!({"rateLimits":w}))
                .unwrap()
                .limits
                .len(),
            1
        );
    }
}
