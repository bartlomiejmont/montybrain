//! Model and effort per agent (Settings → "Agenci"): validation, the argv
//! additions for both CLIs, and recognising a rejected model.
//!
//! The model is user input that ends up in argv: it is checked strictly (short,
//! a conservative character set, no leading dash) and always passed as its own
//! argv element (background) or single-quoted (Terminal launcher). Effort is an
//! enum per provider. Empty means "default", i.e. no flag at all.
//!
//! Verified flags: `claude --model <alias-or-id> --effort <low|medium|high|xhigh|max>`
//! (`claude --help`, 2.1.325); `codex exec -m <MODEL>` (`codex exec --help`,
//! 0.169.0) and `-c model_reasoning_effort="<value>"`, whose values come from
//! the `supported_reasoning_levels` of the CLI's own model catalog
//! (`~/.codex/models_cache.json`, written by codex-cli 0.169.0).

use serde::{Deserialize, Serialize};

use super::ProviderId;
use super::command::toml_string;

pub const CLAUDE_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
pub const CODEX_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultra"];
pub const MAX_MODEL_CHARS: usize = 64;

/// The model and effort of one provider; `None` = the CLI's default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentChoice {
    pub model: Option<String>,
    pub effort: Option<String>,
}

/// Settings key `agents`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Agents {
    pub claude: AgentChoice,
    pub codex: AgentChoice,
}

impl Agents {
    pub fn get(&self, p: ProviderId) -> &AgentChoice {
        match p {
            ProviderId::Claude => &self.claude,
            ProviderId::Codex => &self.codex,
        }
    }

    /// Both choices validated and normalised (trimmed, empty → `None`).
    pub fn validated(&self) -> Result<Agents, String> {
        Ok(Agents {
            claude: self.claude.validated(ProviderId::Claude)?,
            codex: self.codex.validated(ProviderId::Codex)?,
        })
    }
}

fn display(p: ProviderId) -> &'static str {
    match p {
        ProviderId::Claude => "Claude Code",
        ProviderId::Codex => "Codex",
    }
}

pub fn efforts(p: ProviderId) -> &'static [&'static str] {
    match p {
        ProviderId::Claude => CLAUDE_EFFORTS,
        ProviderId::Codex => CODEX_EFFORTS,
    }
}

/// A model name: trimmed; empty → `None`; at most [`MAX_MODEL_CHARS`] of
/// `A–Z a–z 0–9 . _ - : / @ [ ]`, not starting with `-`.
pub fn check_model(p: ProviderId, s: &str) -> Result<Option<String>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let bad = || {
        format!(
            "Nieprawidłowa nazwa modelu dla {}: „{}”. Dozwolone są litery, cyfry i znaki . _ - : / @ [ ] (najwyżej {MAX_MODEL_CHARS}).",
            display(p),
            s.chars().take(MAX_MODEL_CHARS).collect::<String>()
        )
    };
    if s.chars().count() > MAX_MODEL_CHARS || s.starts_with('-') {
        return Err(bad());
    }
    let ok = s.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/' | '@' | '[' | ']')
    });
    if !ok {
        return Err(bad());
    }
    Ok(Some(s.to_string()))
}

pub fn check_effort(p: ProviderId, s: &str) -> Result<Option<String>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    if efforts(p).contains(&s) {
        Ok(Some(s.to_string()))
    } else {
        Err(format!(
            "Nieznany poziom wysiłku dla {}: „{}” (dozwolone: {}).",
            display(p),
            s.chars().take(20).collect::<String>(),
            efforts(p).join(", ")
        ))
    }
}

impl AgentChoice {
    pub fn validated(&self, p: ProviderId) -> Result<AgentChoice, String> {
        Ok(AgentChoice {
            model: match &self.model {
                Some(m) => check_model(p, m)?,
                None => None,
            },
            effort: match &self.effort {
                Some(e) => check_effort(p, e)?,
                None => None,
            },
        })
    }

    pub fn is_default(&self) -> bool {
        self.model.is_none() && self.effort.is_none()
    }
}

/// The argv additions (background and Terminal). Values were validated, but
/// the choice is validated again here: an invalid one adds nothing.
pub fn args(p: ProviderId, choice: &AgentChoice) -> Vec<String> {
    let Ok(c) = choice.validated(p) else {
        return Vec::new();
    };
    let mut a = Vec::new();
    match p {
        ProviderId::Claude => {
            if let Some(m) = c.model {
                a.extend(["--model".to_string(), m]);
            }
            if let Some(e) = c.effort {
                a.extend(["--effort".to_string(), e]);
            }
        }
        ProviderId::Codex => {
            if let Some(m) = c.model {
                a.extend(["-m".to_string(), m]);
            }
            if let Some(e) = c.effort {
                a.extend([
                    "-c".to_string(),
                    format!("model_reasoning_effort={}", toml_string(&e)),
                ]);
            }
        }
    }
    a
}

/// Whether a CLI's error text says it did not accept `model` (recognised by
/// wording only; the CLIs have no stable error code for it).
pub fn model_rejected(model: &str, error: &str) -> bool {
    let e = error.to_lowercase();
    let m = model.to_lowercase();
    if e.contains("model_not_found") || e.contains("invalid model") || e.contains("unknown model") {
        return true;
    }
    e.contains(&m)
        && [
            "not found",
            "does not exist",
            "not exist",
            "unsupported",
            "not supported",
            "not available",
            "no access",
            "issue with the selected model",
            "invalid",
            "unknown",
        ]
        .iter()
        .any(|p| e.contains(p))
}

/// The plain first line of a rejected-model failure.
pub fn model_rejected_message(p: ProviderId, model: &str) -> String {
    format!(
        "{} nie przyjął modelu „{model}”. Zmień go w Ustawieniach.",
        display(p)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_names_are_checked_strictly() {
        let p = ProviderId::Claude;
        assert_eq!(check_model(p, "  "), Ok(None));
        for ok in [
            "opus",
            "sonnet",
            "claude-opus-4-5",
            "opus[1m]",
            "gpt-6.1-sol",
            "org/model:latest",
            "a@b",
        ] {
            assert_eq!(check_model(p, ok), Ok(Some(ok.to_string())), "{ok}");
        }
        for bad in [
            "--dangerously-skip-permissions",
            "-m",
            "a b",
            "x;rm -rf ~",
            "$(id)",
            "`id`",
            "a'b",
            "a\"b",
            "a\nb",
            "ą",
            &"a".repeat(MAX_MODEL_CHARS + 1),
        ] {
            assert!(check_model(p, bad).is_err(), "{bad:?}");
        }
        assert!(check_model(p, &"a".repeat(MAX_MODEL_CHARS)).is_ok());
    }

    #[test]
    fn efforts_per_provider() {
        assert_eq!(
            check_effort(ProviderId::Claude, "max"),
            Ok(Some("max".into()))
        );
        assert!(check_effort(ProviderId::Claude, "ultra").is_err());
        assert_eq!(
            check_effort(ProviderId::Codex, "ultra"),
            Ok(Some("ultra".into()))
        );
        assert!(check_effort(ProviderId::Codex, "minimal").is_err());
        assert!(check_effort(ProviderId::Codex, "high\"; x=\"").is_err());
        assert_eq!(check_effort(ProviderId::Codex, ""), Ok(None));
    }

    #[test]
    fn argv_additions() {
        let c = |m: Option<&str>, e: Option<&str>| AgentChoice {
            model: m.map(String::from),
            effort: e.map(String::from),
        };
        assert!(args(ProviderId::Claude, &AgentChoice::default()).is_empty());
        assert_eq!(
            args(ProviderId::Claude, &c(Some("opus"), Some("high"))),
            ["--model", "opus", "--effort", "high"]
        );
        assert_eq!(
            args(ProviderId::Claude, &c(None, Some("max"))),
            ["--effort", "max"]
        );
        assert_eq!(
            args(ProviderId::Codex, &c(Some("gpt-6-sol"), Some("xhigh"))),
            ["-m", "gpt-6-sol", "-c", "model_reasoning_effort=\"xhigh\""]
        );
        // An invalid stored choice (hand-edited settings) adds nothing.
        assert!(args(ProviderId::Codex, &c(Some("--yolo"), None)).is_empty());
        assert!(args(ProviderId::Claude, &c(None, Some("ultra"))).is_empty());
    }

    #[test]
    fn rejected_models_are_recognised() {
        assert!(model_rejected(
            "gpt-9",
            "The model `gpt-9` does not exist or you do not have access to it."
        ));
        assert!(model_rejected(
            "opus-x",
            "There's an issue with the selected model (opus-x). It may not exist or you may not have access to it."
        ));
        assert!(model_rejected(
            "x",
            "{\"error\":{\"code\":\"model_not_found\"}}"
        ));
        assert!(!model_rejected("opus", "You have hit your usage limit."));
        assert!(!model_rejected("opus", "Not logged in · Please run /login"));
        assert_eq!(
            model_rejected_message(ProviderId::Codex, "x"),
            "Codex nie przyjął modelu „x”. Zmień go w Ustawieniach."
        );
        let json = serde_json::to_value(Agents::default()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"claude":{"model":null,"effort":null},"codex":{"model":null,"effort":null}})
        );
    }
}
