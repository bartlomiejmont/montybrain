//! App settings: how the app behaves on this machine, not vault data.
//!
//! Stored as `settings.json` in the config dir (`$MONTYBRAIN_CONFIG_DIR` when set,
//! else the platform app-config dir, e.g. `~/Library/Application Support/
//! pl.montybrain.desktop`). A missing or unreadable file means defaults; the file
//! is rewritten atomically on every change.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use montybrain_core::TaskLocation;
use serde::{Deserialize, Deserializer, Serialize};

use crate::runs::agent::Agents;
use crate::updates::schedule::{self, AutoUpdate, UpdateSources};

/// Env var that overrides the config dir (tests, dev runs).
pub const ENV_CONFIG_DIR: &str = "MONTYBRAIN_CONFIG_DIR";
/// The pre-rename name of [`ENV_CONFIG_DIR`], honoured when that is unset.
pub const LEGACY_ENV_CONFIG_DIR: &str = "MORWISKO_CONFIG_DIR";
/// The config dir name (bundle identifier) before the rename.
pub const LEGACY_IDENTIFIER: &str = "pl.morwisko.desktop";
/// Files copied once from the old config dir. Not caches (run files live in
/// the cache dir and are left behind). `settings.json` goes last: its
/// presence marks the migration done, so an interrupted copy is redone.
pub const MIGRATED_FILES: [&str; 5] = [
    "updates.json",
    "reminders.json",
    "waiting.json",
    "drift.json",
    SETTINGS_FILE,
];
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

/// The first non-empty of a new env variable and its legacy name (the new one
/// wins).
pub fn env_with_legacy(
    value: Option<std::ffi::OsString>,
    legacy: Option<std::ffi::OsString>,
) -> Option<std::ffi::OsString> {
    value
        .filter(|v| !v.is_empty())
        .or_else(|| legacy.filter(|v| !v.is_empty()))
}

/// `$MONTYBRAIN_CONFIG_DIR`, else `$MORWISKO_CONFIG_DIR` (non-empty only).
pub fn config_dir_override() -> Option<PathBuf> {
    env_with_legacy(
        std::env::var_os(ENV_CONFIG_DIR),
        std::env::var_os(LEGACY_ENV_CONFIG_DIR),
    )
    .map(PathBuf::from)
}

/// The pre-rename config dir next to `new_dir`
/// (`…/Application Support/pl.morwisko.desktop`).
pub fn legacy_config_dir(new_dir: &Path) -> Option<PathBuf> {
    new_dir.parent().map(|p| p.join(LEGACY_IDENTIFIER))
}

/// One-time copy of the old app's state: when `new_dir` has no
/// `settings.json` and `old_dir` exists, copies [`MIGRATED_FILES`] that exist
/// there and not yet here (writing default settings when the old app had
/// none, which marks the migration done). `old_dir` is only read. Returns
/// the names copied (empty = nothing done: already migrated or no old dir).
pub fn migrate_config(new_dir: &Path, old_dir: &Path) -> io::Result<Vec<&'static str>> {
    if new_dir.join(SETTINGS_FILE).exists() || !old_dir.is_dir() || new_dir == old_dir {
        return Ok(Vec::new());
    }
    let mut copied = Vec::new();
    for name in MIGRATED_FILES {
        let (from, to) = (old_dir.join(name), new_dir.join(name));
        if !from.is_file() || to.exists() {
            continue;
        }
        let bytes = std::fs::read(&from)?;
        write_atomic(&to, &bytes)?;
        copied.push(name);
    }
    // The old app never saved settings: write the defaults (what a missing
    // file means anyway) so the migration counts as done and is not redone.
    if !new_dir.join(SETTINGS_FILE).exists() {
        Settings::default().save(&new_dir.join(SETTINGS_FILE))?;
    }
    Ok(copied)
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
    fn env_with_legacy_prefers_the_new_name() {
        assert_eq!(env_with_legacy(None, None), None);
        assert_eq!(
            env_with_legacy(Some("/new".into()), Some("/old".into())),
            Some("/new".into())
        );
        assert_eq!(
            env_with_legacy(None, Some("/old".into())),
            Some("/old".into())
        );
        assert_eq!(
            env_with_legacy(Some("".into()), Some("/old".into())),
            Some("/old".into())
        );
        assert_eq!(env_with_legacy(Some("".into()), Some("".into())), None);
    }

    fn old_config(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        for name in MIGRATED_FILES {
            std::fs::write(dir.join(name), format!("{{\"from\":\"{name}\"}}\n")).unwrap();
        }
        // Not migrated: a cache-like extra and the atomic-save temp.
        std::fs::write(dir.join("other.json"), "{}").unwrap();
        std::fs::create_dir_all(dir.join("cache")).unwrap();
    }

    fn listing(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                let bytes = if e.path().is_file() {
                    std::fs::read(e.path()).unwrap()
                } else {
                    Vec::new()
                };
                (e.file_name().to_string_lossy().into_owned(), bytes)
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn legacy_config_dir_is_next_to_the_new_one() {
        assert_eq!(
            legacy_config_dir(Path::new("/L/Application Support/pl.montybrain.desktop")),
            Some(PathBuf::from("/L/Application Support/pl.morwisko.desktop"))
        );
    }

    #[test]
    fn migrate_config_copies_once_and_leaves_the_old_dir_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("pl.morwisko.desktop");
        let new = tmp.path().join("pl.montybrain.desktop");
        old_config(&old);
        let before = listing(&old);

        let copied = migrate_config(&new, &old).unwrap();
        assert_eq!(copied, MIGRATED_FILES.to_vec());
        for name in MIGRATED_FILES {
            assert_eq!(
                std::fs::read(new.join(name)).unwrap(),
                std::fs::read(old.join(name)).unwrap()
            );
        }
        assert!(!new.join("other.json").exists());
        assert!(!new.join("cache").exists());
        assert_eq!(listing(&old), before, "the old dir is untouched");

        // Already migrated: a later start changes nothing, even after the
        // new app wrote its own settings and the old one changed.
        std::fs::write(new.join(SETTINGS_FILE), "{\"mine\":1}\n").unwrap();
        std::fs::write(old.join("waiting.json"), "{\"newer\":1}\n").unwrap();
        std::fs::remove_file(new.join("drift.json")).unwrap();
        assert!(migrate_config(&new, &old).unwrap().is_empty());
        assert_eq!(
            std::fs::read_to_string(new.join(SETTINGS_FILE)).unwrap(),
            "{\"mine\":1}\n"
        );
        assert!(!new.join("drift.json").exists());
        assert_ne!(
            std::fs::read(new.join("waiting.json")).unwrap(),
            std::fs::read(old.join("waiting.json")).unwrap()
        );
    }

    #[test]
    fn migrate_config_without_an_old_dir_does_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let new = tmp.path().join("pl.montybrain.desktop");
        let old = tmp.path().join("pl.morwisko.desktop");
        assert!(migrate_config(&new, &old).unwrap().is_empty());
        assert!(!new.exists(), "a fresh install creates nothing here");
    }

    #[test]
    fn migrate_config_copies_what_exists_and_marks_itself_done() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let new = tmp.path().join("new");
        // The old app never saved settings; it has a waiting list only.
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("waiting.json"), "{\"w\":1}\n").unwrap();
        std::fs::write(old.join("drift.json"), "{\"d\":1}\n").unwrap();
        // An interrupted earlier attempt already copied drift.json and the
        // new app changed it since: not overwritten.
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("drift.json"), "{\"kept\":1}\n").unwrap();
        let copied = migrate_config(&new, &old).unwrap();
        assert_eq!(copied, vec!["waiting.json"]);
        assert_eq!(
            std::fs::read_to_string(new.join("drift.json")).unwrap(),
            "{\"kept\":1}\n"
        );
        // Default settings mark it done: nothing is copied again.
        assert_eq!(
            Settings::load(&new.join(SETTINGS_FILE)),
            Settings::default()
        );
        std::fs::remove_file(new.join("waiting.json")).unwrap();
        assert!(migrate_config(&new, &old).unwrap().is_empty());
        assert!(!new.join("waiting.json").exists());
        assert!(
            !old.join(SETTINGS_FILE).exists(),
            "the old dir is only read"
        );
        // Same dir is a no-op.
        assert!(migrate_config(&old, &old).unwrap().is_empty());
    }
}
