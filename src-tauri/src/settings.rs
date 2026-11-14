//! App settings: how the app behaves on this machine, not vault data.
//!
//! Stored as `settings.json` in the config dir (`$MORWISKO_CONFIG_DIR` when set,
//! else the platform app-config dir, e.g. `~/Library/Application Support/
//! pl.morwisko.desktop`). A missing or unreadable file means defaults; the file
//! is rewritten atomically on every change.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use morwisko_core::TaskLocation;
use serde::{Deserialize, Deserializer, Serialize};

use crate::runs::agent::Agents;
use crate::updates::schedule::{self, AutoUpdate, UpdateSources};

/// Env var that overrides the config dir (tests, dev runs).
pub const ENV_CONFIG_DIR: &str = "MORWISKO_CONFIG_DIR";
pub const SETTINGS_FILE: &str = "settings.json";
pub const DEFAULT_SHORTCUT: &str = "Ctrl+Alt+Space";
pub const DEFAULT_DIGEST_HOUR: u8 = 9;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Global quick-capture shortcut, e.g. `"Ctrl+Alt+Space"` (modifiers in the
    /// order Ctrl, Alt, Shift, Cmd, then a `KeyboardEvent.code`-style key).
    pub shortcut: String,
    /// Due-time reminders and the daily digest.
    pub reminders: bool,
    /// Hour (0–23, local time) of the daily digest.
    pub digest_hour: u8,
    /// Last destination picked in the quick-capture window.
    pub quick_destination: TaskLocation,
    /// Automatic project update per project slug (absent = off). An invalid
    /// entry is dropped on load instead of making the whole file unreadable.
    #[serde(deserialize_with = "lenient_auto_update")]
    pub auto_update: BTreeMap<String, AutoUpdate>,
    /// Sources of the project update per project slug (absent = repositories
    /// only). Invalid entries are dropped on load.
    #[serde(deserialize_with = "lenient_sources")]
    pub update_sources: BTreeMap<String, UpdateSources>,
    /// Model and effort per agent; an invalid stored value reads as default.
    #[serde(deserialize_with = "lenient_agents")]
    pub agents: Agents,
    /// Default days between checks of a waiting item (1, 2, 3 or 7).
    pub waiting_days: u8,
}

fn lenient_sources<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, UpdateSources>, D::Error> {
    let raw = match serde_json::Value::deserialize(d)? {
        serde_json::Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    Ok(raw
        .into_iter()
        .filter_map(|(k, v)| {
            let s: UpdateSources = serde_json::from_value(v).ok()?;
            (!k.is_empty() && s.validate().is_ok()).then_some((k, s))
        })
        .collect())
}

fn lenient_agents<'de, D: Deserializer<'de>>(d: D) -> Result<Agents, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value::<Agents>(v)
        .ok()
        .and_then(|a| a.validated().ok())
        .unwrap_or_else(|| {
            crate::log("settings: ignoring an invalid `agents` entry");
            Agents::default()
        }))
}

fn lenient_auto_update<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, AutoUpdate>, D::Error> {
    let raw = match serde_json::Value::deserialize(d)? {
        serde_json::Value::Object(m) => m,
        _ => {
            crate::log("settings: ignoring auto_update (not an object)");
            serde_json::Map::new()
        }
    };
    Ok(schedule::sanitize(
        raw.into_iter()
            .filter_map(|(k, v)| match serde_json::from_value::<AutoUpdate>(v) {
                Ok(cfg) => Some((k, cfg)),
                Err(e) => {
                    crate::log(&format!("settings: ignoring auto_update.{k}: {e}"));
                    None
                }
            })
            .collect(),
    ))
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            shortcut: DEFAULT_SHORTCUT.into(),
            reminders: true,
            digest_hour: DEFAULT_DIGEST_HOUR,
            quick_destination: TaskLocation::Inbox,
            auto_update: BTreeMap::new(),
            update_sources: BTreeMap::new(),
            agents: Agents::default(),
            waiting_days: crate::waiting::DEFAULT_DAYS,
        }
    }
}

/// `update_settings` input; absent keys are kept.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SettingsPatch {
    pub reminders: Option<bool>,
    pub digest_hour: Option<u8>,
    pub quick_destination: Option<TaskLocation>,
    /// Replaces both agents' model / effort (validated).
    pub agents: Option<Agents>,
    pub waiting_days: Option<u8>,
}

impl Settings {
    /// Applies a patch; returns whether anything changed.
    pub fn apply(&mut self, patch: SettingsPatch) -> Result<bool, String> {
        let before = self.clone();
        if let Some(h) = patch.digest_hour {
            if h > 23 {
                return Err(format!("Nieprawidłowa godzina: {h}"));
            }
            self.digest_hour = h;
        }
        if let Some(r) = patch.reminders {
            self.reminders = r;
        }
        if let Some(d) = patch.quick_destination {
            self.quick_destination = d;
        }
        if let Some(a) = patch.agents {
            self.agents = a.validated()?;
        }
        if let Some(d) = patch.waiting_days {
            self.waiting_days = crate::waiting::valid_days(d)?;
        }
        Ok(*self != before)
    }

    /// Sets (or, when off with default choices, removes) the automatic
    /// update of `project`; returns whether anything changed.
    pub fn set_auto_update(&mut self, project: &str, cfg: AutoUpdate) -> Result<bool, String> {
        cfg.validate()?;
        let before = self.auto_update.get(project).copied();
        if cfg == AutoUpdate::default() {
            self.auto_update.remove(project);
        } else {
            self.auto_update.insert(project.to_string(), cfg);
        }
        Ok(self.auto_update.get(project).copied() != before)
    }

    /// Sets (or, when default, removes) the update sources of `project`.
    pub fn set_update_sources(
        &mut self,
        project: &str,
        src: UpdateSources,
    ) -> Result<bool, String> {
        src.validate()?;
        let before = self.update_sources.get(project).copied();
        if src == UpdateSources::default() {
            self.update_sources.remove(project);
        } else {
            self.update_sources.insert(project.to_string(), src);
        }
        Ok(self.update_sources.get(project).copied() != before)
    }

    /// Reads the settings file. Missing → defaults; unreadable → defaults + a
    /// log line (the broken file is left as is until the next save).
    pub fn load(path: &Path) -> Settings {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Settings::default(),
            Err(e) => {
                crate::log(&format!("settings: cannot read {}: {e}", path.display()));
                return Settings::default();
            }
        };
        match serde_json::from_str::<Settings>(&text) {
            Ok(mut s) => {
                if s.digest_hour > 23 {
                    s.digest_hour = DEFAULT_DIGEST_HOUR;
                }
                if s.shortcut.trim().is_empty() {
                    s.shortcut = DEFAULT_SHORTCUT.into();
                }
                s
            }
            Err(e) => {
                crate::log(&format!("settings: ignoring {}: {e}", path.display()));
                Settings::default()
            }
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        json.push('\n');
        write_atomic(path, json.as_bytes())
    }
}

/// The config dir: `$MORWISKO_CONFIG_DIR` if non-empty, else `fallback`
/// (the platform app-config dir).
pub fn resolve_config_dir(env_value: Option<std::ffi::OsString>, fallback: PathBuf) -> PathBuf {
    match env_value {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => fallback,
    }
}

/// Writes via a temp file + rename so a crash never leaves half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join(SETTINGS_FILE);
        assert_eq!(Settings::load(&path), Settings::default());
        let s = Settings {
            shortcut: "Cmd+Shift+KeyK".into(),
            reminders: false,
            digest_hour: 7,
            quick_destination: TaskLocation::Topic {
                project: "p".into(),
                topic: "t".into(),
            },
            auto_update: BTreeMap::from([(
                "beeking".to_string(),
                AutoUpdate {
                    schedule: schedule::Schedule::Daily { hour: 7 },
                    provider: crate::runs::ProviderId::Codex,
                    on_result: schedule::OnResult::Apply,
                },
            )]),
            update_sources: BTreeMap::from([(
                "beeking".to_string(),
                UpdateSources {
                    history: true,
                    history_days: 7,
                },
            )]),
            agents: Agents {
                claude: crate::runs::agent::AgentChoice {
                    model: Some("opus".into()),
                    effort: Some("high".into()),
                },
                codex: crate::runs::agent::AgentChoice {
                    model: None,
                    effort: Some("xhigh".into()),
                },
            },
            waiting_days: 3,
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["quick_destination"]["kind"], "topic");
        assert_eq!(json["digest_hour"], 7);
        assert_eq!(
            json["auto_update"]["beeking"],
            serde_json::json!({"schedule":{"kind":"daily","hour":7},"provider":"codex","on_result":"apply"})
        );
    }

    #[test]
    fn agents_and_sources_are_lenient_validated_and_settable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        std::fs::write(
            &path,
            r#"{"reminders": false,
                "agents": {"claude": {"model": "--yolo"}},
                "update_sources": {"ok": {"history": true, "history_days": 30}, "zle": {"history_days": 5}, "x": 1}}"#,
        )
        .unwrap();
        let s = Settings::load(&path);
        assert!(!s.reminders);
        assert_eq!(
            s.agents,
            Agents::default(),
            "an invalid model reads as default"
        );
        assert_eq!(s.update_sources.keys().collect::<Vec<_>>(), ["ok"]);

        let mut s = Settings::default();
        let bad = Agents {
            codex: crate::runs::agent::AgentChoice {
                model: None,
                effort: Some("minimal".into()),
            },
            ..Default::default()
        };
        assert!(
            s.apply(SettingsPatch {
                agents: Some(bad),
                ..Default::default()
            })
            .is_err()
        );
        let good = Agents {
            claude: crate::runs::agent::AgentChoice {
                model: Some("  sonnet ".into()),
                effort: Some("".into()),
            },
            ..Default::default()
        };
        assert_eq!(
            s.apply(SettingsPatch {
                agents: Some(good),
                ..Default::default()
            }),
            Ok(true)
        );
        assert_eq!(s.agents.claude.model.as_deref(), Some("sonnet"));
        assert_eq!(s.agents.claude.effort, None);

        let on = UpdateSources {
            history: true,
            history_days: 7,
        };
        assert_eq!(s.set_update_sources("p", on), Ok(true));
        assert_eq!(s.set_update_sources("p", on), Ok(false));
        assert!(
            s.set_update_sources(
                "p",
                UpdateSources {
                    history: true,
                    history_days: 3
                }
            )
            .is_err()
        );
        assert_eq!(
            s.set_update_sources("p", UpdateSources::default()),
            Ok(true)
        );
        assert!(s.update_sources.is_empty());
    }

    #[test]
    fn auto_update_entries_are_lenient_and_settable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        std::fs::write(
            &path,
            r#"{"reminders": false, "auto_update": {
                "ok": {"schedule": {"kind": "every", "hours": 6}, "provider": "codex"},
                "zly-dostawca": {"provider": "gpt"},
                "zero": {"schedule": {"kind": "every", "hours": 0}},
                "nieznane": {"x": 1}
            }}"#,
        )
        .unwrap();
        let s = Settings::load(&path);
        assert!(!s.reminders, "the rest of the file is still read");
        assert_eq!(s.auto_update.keys().collect::<Vec<_>>(), ["ok"]);
        assert_eq!(s.auto_update["ok"].on_result, schedule::OnResult::Propose);
        std::fs::write(&path, r#"{"auto_update": 5}"#).unwrap();
        assert!(Settings::load(&path).auto_update.is_empty());

        let mut s = Settings::default();
        let on = AutoUpdate {
            schedule: schedule::Schedule::Every { hours: 2 },
            ..Default::default()
        };
        assert_eq!(s.set_auto_update("p", on), Ok(true));
        assert_eq!(s.set_auto_update("p", on), Ok(false));
        let bad = AutoUpdate {
            schedule: schedule::Schedule::Every { hours: 0 },
            ..Default::default()
        };
        assert!(s.set_auto_update("p", bad).is_err());
        assert_eq!(s.auto_update["p"], on);
        assert_eq!(s.set_auto_update("p", AutoUpdate::default()), Ok(true));
        assert!(s.auto_update.is_empty());
    }

    #[test]
    fn partial_and_broken_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        std::fs::write(
            &path,
            r#"{"reminders": false, "digest_hour": 99, "shortcut": ""}"#,
        )
        .unwrap();
        let s = Settings::load(&path);
        assert!(!s.reminders);
        assert_eq!(s.digest_hour, DEFAULT_DIGEST_HOUR);
        assert_eq!(s.shortcut, DEFAULT_SHORTCUT);
        std::fs::write(&path, "nie json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
    }

    #[test]
    fn patch() {
        let mut s = Settings::default();
        assert_eq!(s.apply(SettingsPatch::default()), Ok(false));
        assert_eq!(
            s.apply(SettingsPatch {
                digest_hour: Some(8),
                ..Default::default()
            }),
            Ok(true)
        );
        assert_eq!(s.digest_hour, 8);
        assert!(
            s.apply(SettingsPatch {
                digest_hour: Some(24),
                ..Default::default()
            })
            .is_err()
        );
        assert_eq!(s.digest_hour, 8);
    }

    #[test]
    fn config_dir_override() {
        let fb = PathBuf::from("/fallback");
        assert_eq!(resolve_config_dir(None, fb.clone()), fb);
        assert_eq!(resolve_config_dir(Some("".into()), fb.clone()), fb);
        assert_eq!(
            resolve_config_dir(Some("/tmp/x".into()), fb),
            PathBuf::from("/tmp/x")
        );
    }
}
