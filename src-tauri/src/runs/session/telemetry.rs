//! Structured CLI reports. Missing values stay unknown; cumulative reports
//! replace previous totals (adding them would count earlier turns twice).
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Telemetry {
    pub connected: bool,
    pub model: Option<String>,
    pub usage: Option<Usage>,
    pub limits: Vec<Limit>,
    pub limits_at: Option<u64>,
    pub limit_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LimitSnapshot {
    pub limits: Vec<Limit>,
    pub checked_at: u64,
    pub status: Option<String>,
}
pub fn timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
impl LimitSnapshot {
    pub fn new(limits: Vec<Limit>, status: Option<String>) -> Self {
        Self {
            limits,
            checked_at: timestamp(),
            status,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Usage {
    /// Includes cached input; cache counters are subsets of this value.
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    /// CLI estimate, never an invoice or subscription balance.
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Limit {
    pub id: String,
    pub used_percent: f64,
    pub reported_at: u64,
    pub window_minutes: Option<u64>,
    pub resets_at: Option<u64>,
    /// CLI /usage prints a local date and zone rather than an epoch. Preserve
    /// that precision instead of guessing its year or timezone offset.
    pub reset_label: Option<String>,
}

fn count(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(Value::as_u64)
}

impl Usage {
    pub fn claude(v: &Value) -> Option<Self> {
        let u = &v["usage"];
        let cached = count(u, "cache_read_input_tokens");
        let write = count(u, "cache_creation_input_tokens");
        let input = count(u, "input_tokens").map(|n| {
            n.saturating_add(cached.unwrap_or(0))
                .saturating_add(write.unwrap_or(0))
        });
        let result = Self {
            input_tokens: input,
            output_tokens: count(u, "output_tokens"),
            cached_input_tokens: cached,
            cache_write_tokens: write,
            cost_usd: v
                .get("total_cost_usd")
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && *n >= 0.0),
        };
        (result.input_tokens.is_some()
            || result.output_tokens.is_some()
            || result.cost_usd.is_some())
        .then_some(result)
    }

    pub fn codex(v: &Value) -> Option<Self> {
        let u = v.pointer("/tokenUsage/total")?;
        let result = Self {
            input_tokens: count(u, "inputTokens"),
            output_tokens: count(u, "outputTokens"),
            cached_input_tokens: count(u, "cachedInputTokens"),
            cache_write_tokens: count(u, "cacheWriteInputTokens"),
            cost_usd: None,
        };
        (result.input_tokens.is_some() || result.output_tokens.is_some()).then_some(result)
    }
}

pub fn codex_limits(p: &Value) -> Vec<Limit> {
    let r = &p["rateLimits"];
    let quota = r.get("limitId").and_then(Value::as_str).unwrap_or("codex");
    ["primary", "secondary"]
        .iter()
        .filter_map(|key| {
            let w = &r[key];
            let used = w.get("usedPercent")?.as_f64()?;
            if !used.is_finite() || !(0.0..=100.0).contains(&used) {
                return None;
            }
            Some(Limit {
                id: format!("{quota}:{key}"),
                used_percent: used,
                reported_at: timestamp(),
                window_minutes: count(w, "windowDurationMins"),
                resets_at: count(w, "resetsAt"),
                reset_label: None,
            })
        })
        .collect()
}

/// Wire fields confirmed in the installed Claude 2.1.325 schema (including
/// unifiedWindows), and anthropics/claude-agent-sdk-python message_parser.py.
pub fn claude_limits(v: &Value) -> (Vec<Limit>, Option<String>) {
    let r = &v["rate_limit_info"];
    let status = match r["status"].as_str() {
        Some("allowed") => Some("W granicach limitu"),
        Some("allowed_warning") => Some("Zbliżasz się do limitu"),
        Some("rejected") => Some("Limit osiągnięty"),
        _ => None,
    }
    .map(String::from);
    let window = |id: &str, w: &Value| -> Option<Limit> {
        let used = w["utilization"].as_f64()?;
        if !used.is_finite() || !(0.0..=1.0).contains(&used) {
            return None;
        }
        let minutes = match id {
            "five_hour" => Some(300),
            "seven_day" | "seven_day_opus" | "seven_day_sonnet" | "seven_day_overage_included" => {
                Some(10080)
            }
            "overage" => None,
            _ => return None,
        };
        Some(Limit {
            id: format!("claude:{id}"),
            used_percent: used * 100.0,
            reported_at: timestamp(),
            window_minutes: minutes,
            resets_at: count(w, "resetsAt"),
            reset_label: None,
        })
    };
    let mut limits: Vec<Limit> = r["unifiedWindows"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(id, w)| window(id, w))
        .collect();
    if let Some(id) = r["rateLimitType"].as_str() {
        if !limits.iter().any(|l| l.id == format!("claude:{id}")) {
            if let Some(limit) = window(id, r) {
                limits.push(limit);
            }
        }
    }
    (limits, status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_events_preserve_unknown_usage_and_parse_real_fractional_windows() {
        let (limits, state) = claude_limits(&json!({"rate_limit_info":{"status":"allowed"}}));
        assert!(limits.is_empty());
        assert_eq!(state.as_deref(), Some("W granicach limitu"));
        let (limits, _) = claude_limits(
            &json!({"rate_limit_info":{"status":"allowed_warning","rateLimitType":"five_hour","utilization":0.8,"unifiedWindows":{"five_hour":{"utilization":0.8,"resetsAt":1800000000},"seven_day":{"utilization":0.3,"resetsAt":1800500000}}}}),
        );
        assert_eq!(limits.len(), 2);
        assert_eq!(limits[0].used_percent, 80.0);
        assert_eq!(limits[0].window_minutes, Some(300));
        assert!(
            claude_limits(
                &json!({"rate_limit_info":{"rateLimitType":"five_hour","utilization":1.5}})
            )
            .0
            .is_empty()
        );
        assert_eq!(claude_limits(&json!({})), (vec![], None));
    }

    #[test]
    fn missing_is_not_zero_and_caches_are_part_of_input() {
        assert_eq!(Usage::claude(&json!({"usage":{}})), None);
        assert_eq!(Usage::codex(&json!({"tokenUsage":{"total":{}}})), None);
        let u = Usage::claude(&json!({"usage":{"input_tokens":10,"output_tokens":0,"cache_read_input_tokens":20,"cache_creation_input_tokens":3},"total_cost_usd":0.0})).unwrap();
        assert_eq!(u.input_tokens, Some(33));
        assert_eq!(u.output_tokens, Some(0));
        assert_eq!(u.cost_usd, Some(0.0));
        let u = Usage::codex(&json!({"tokenUsage":{"total":{"inputTokens":100,"cachedInputTokens":60,"outputTokens":5,"reasoningOutputTokens":3}}})).unwrap();
        assert_eq!(u.input_tokens, Some(100));
        assert_eq!(u.output_tokens, Some(5));
        assert_eq!(u.cost_usd, None);
    }

    #[test]
    fn limits_ignore_missing_and_malformed_windows() {
        assert!(codex_limits(&json!({"rateLimits":{"primary":{"usedPercent":-1}}})).is_empty());
        let limits = codex_limits(
            &json!({"rateLimits":{"limitId":"codex","primary":{"usedPercent":25,"windowDurationMins":300,"resetsAt":1800000000},"secondary":null}}),
        );
        assert_eq!(limits.len(), 1);
        assert_eq!(limits[0].used_percent, 25.0);
        assert_eq!(limits[0].window_minutes, Some(300));
    }
}
