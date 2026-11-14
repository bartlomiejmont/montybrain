//! What an in-app agent session may do without asking, what it may never do,
//! and what the person decides on a card. Pure: no process, no UI.
//!
//! * MontyBrain's read tools are allowed without asking (the session is
//!   started with `montybrain-mcp` read-only anyway).
//! * MontyBrain's writing tools, any file write inside the vault and any shell
//!   command that names the vault are denied outright: they are never shown
//!   as a card, so they cannot be approved by mistake.
//! * Everything else is a card: "Zezwól", "Zezwól na tę sesję" (remembered
//!   for the rest of this session under the request's key), "Odmów".

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use super::super::command::{MONTYBRAIN_READ_TOOLS, MONTYBRAIN_WRITING_TOOLS};

/// One permission request, as the provider adaptor describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    /// The tool's name as the CLI gives it (`Bash`, `mcp__montybrain__search`,
    /// `command`, `fileChange`, `mcp:computer-use`…).
    pub tool: String,
    /// What "Zezwól na tę sesję" remembers (`Bash:osascript`, `mcp:computer-use`).
    pub key: String,
    /// One line for the card.
    pub title: String,
    /// The details (command, files, arguments), already shortened.
    pub detail: String,
    /// Files the request would write, when known.
    pub paths: Vec<String>,
    /// Free text of the request (a shell command), checked for the vault path.
    pub text: String,
    /// The provider can be told "for this session" (or we remember it).
    pub can_session: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Allowed without asking (MontyBrain read tools, an earlier "na tę sesję").
    Allow,
    /// Never allowed; the reason is shown, there is no card.
    Deny(String),
    /// A card for the person.
    Ask,
}

pub const DENY_MONTYBRAIN: &str =
    "MontyBrain jest dla agenta tylko do odczytu: zmiany wracają jako propozycje do zaznaczenia.";
pub const DENY_VAULT: &str = "Pliki MontyBrain są dla agenta tylko do odczytu: zmiany wracają jako propozycje do zaznaczenia.";

#[derive(Debug, Clone, Default)]
pub struct Gate {
    vault: PathBuf,
    session: BTreeSet<String>,
}

/// `p` without `.`/`..` (lexically), so `/v/../v/x` and `/v/x` compare equal.
fn normal(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn is_montybrain_tool(tool: &str) -> bool {
    tool.starts_with("mcp__montybrain__")
        || tool == "mcp:montybrain"
        || tool.starts_with("mcp:montybrain/")
}

impl Gate {
    pub fn new(vault: &Path) -> Gate {
        Gate {
            vault: normal(vault),
            session: BTreeSet::new(),
        }
    }

    fn in_vault(&self, path: &str) -> bool {
        let p = path.trim();
        if p.is_empty() || self.vault.as_os_str().is_empty() {
            return false;
        }
        // Claude Code permission rules write absolute paths as `//…`.
        let p = if let Some(rest) = p.strip_prefix("//") {
            format!("/{rest}")
        } else {
            p.to_string()
        };
        normal(Path::new(&p)).starts_with(&self.vault)
    }

    fn names_vault(&self, text: &str) -> bool {
        let v = self.vault.to_string_lossy();
        !v.is_empty() && v != "/" && text.contains(v.as_ref())
    }

    pub fn check(&self, ask: &Ask) -> Verdict {
        if MONTYBRAIN_READ_TOOLS.contains(&ask.tool.as_str()) {
            return Verdict::Allow;
        }
        if MONTYBRAIN_WRITING_TOOLS.contains(&ask.tool.as_str()) || is_montybrain_tool(&ask.tool) {
            return Verdict::Deny(DENY_MONTYBRAIN.into());
        }
        if ask.paths.iter().any(|p| self.in_vault(p)) || self.names_vault(&ask.text) {
            return Verdict::Deny(DENY_VAULT.into());
        }
        if ask.can_session && self.session.contains(&ask.key) {
            return Verdict::Allow;
        }
        Verdict::Ask
    }

    /// "Zezwól na tę sesję": only for something that would be a card.
    pub fn allow_for_session(&mut self, ask: &Ask) -> Result<(), String> {
        match self.check(ask) {
            Verdict::Deny(why) => Err(why),
            _ if !ask.can_session => Err("Na to można zezwolić tylko raz.".into()),
            _ => {
                self.session.insert(ask.key.clone());
                Ok(())
            }
        }
    }

    pub fn remembered(&self) -> Vec<String> {
        self.session.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(tool: &str, key: &str) -> Ask {
        Ask {
            tool: tool.into(),
            key: key.into(),
            title: tool.into(),
            detail: String::new(),
            paths: vec![],
            text: String::new(),
            can_session: true,
        }
    }

    #[test]
    fn montybrain_reads_pass_and_writes_never_do() {
        let mut g = Gate::new(Path::new("/Users/a/Morwisko"));
        assert_eq!(
            g.check(&ask("mcp__montybrain__search", "x")),
            Verdict::Allow
        );
        for t in MONTYBRAIN_WRITING_TOOLS {
            let a = ask(t, t);
            assert!(matches!(g.check(&a), Verdict::Deny(_)), "{t}");
            // Not approvable, not even "for this session".
            assert!(g.allow_for_session(&a).is_err());
            assert!(matches!(g.check(&a), Verdict::Deny(_)));
        }
        assert!(matches!(
            g.check(&ask("mcp:montybrain", "mcp:montybrain")),
            Verdict::Deny(_)
        ));
        assert!(matches!(
            g.check(&ask("mcp__montybrain__future_tool", "y")),
            Verdict::Deny(_)
        ));
    }

    #[test]
    fn vault_files_and_commands_naming_the_vault_are_denied() {
        let mut g = Gate::new(Path::new("/Users/a/Morwisko/"));
        let mut w = ask("Write", "Write");
        w.paths = vec!["/Users/a/Morwisko/firma/projekt.md".into()];
        assert_eq!(g.check(&w), Verdict::Deny(DENY_VAULT.into()));
        w.paths = vec!["/Users/a/Morwisko/../Morwisko/x.md".into()];
        assert!(matches!(g.check(&w), Verdict::Deny(_)));
        w.paths = vec!["//Users/a/Morwisko/x.md".into()];
        assert!(matches!(g.check(&w), Verdict::Deny(_)));
        assert!(g.allow_for_session(&w).is_err());
        w.paths = vec!["/Users/a/MorwiskoInne/x.md".into()];
        assert_eq!(g.check(&w), Verdict::Ask);
        let mut b = ask("Bash", "Bash:sed");
        b.text = "sed -i '' s/a/b/ /Users/a/Morwisko/skrzynka.md".into();
        assert!(matches!(g.check(&b), Verdict::Deny(_)));
    }

    #[test]
    fn for_this_session_remembers_the_key_only() {
        let mut g = Gate::new(Path::new("/v"));
        let a = ask("Bash", "Bash:osascript");
        assert_eq!(g.check(&a), Verdict::Ask);
        g.allow_for_session(&a).unwrap();
        assert_eq!(g.check(&a), Verdict::Allow);
        assert_eq!(g.check(&ask("Bash", "Bash:rm")), Verdict::Ask);
        let mut once = ask("x", "x");
        once.can_session = false;
        assert!(g.allow_for_session(&once).is_err());
        assert_eq!(g.remembered(), ["Bash:osascript"]);
    }
}
