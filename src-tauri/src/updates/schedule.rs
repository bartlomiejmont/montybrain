//! Automatic project update: the per-project setting and the pure schedule
//! decision ("which project, if any, is due now?").

use std::collections::BTreeMap;

use montybrain_core::chrono::{NaiveDateTime, NaiveTime, TimeDelta};
use serde::{Deserialize, Serialize};

use crate::runs::ProviderId;

/// Longest "every N hours" interval.
pub const MAX_HOURS: u32 = 7 * 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    #[default]
    Off,
    /// Every `hours` hours (at least 1), counted from the last check.
    Every { hours: u32 },
    /// Once a day, the first check at or after `hour`:00 (local time).
    Daily { hour: u8 },
}

/// What to do with the proposal of an automatic run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnResult {
    /// Keep it for review in the project view (default).
    #[default]
    Propose,
    /// Apply every valid change right away.
    Apply,
}

/// Per-project automatic update setting (`settings.json`, key `auto_update`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AutoUpdate {
    pub schedule: Schedule,
    pub provider: ProviderId,
    pub on_result: OnResult,
}

impl Default for AutoUpdate {
    fn default() -> Self {
        AutoUpdate {
            schedule: Schedule::Off,
            provider: ProviderId::Claude,
            on_result: OnResult::Propose,
        }
    }
}

impl AutoUpdate {
    pub fn validate(&self) -> Result<(), String> {
        match self.schedule {
            Schedule::Every { hours } if !(1..=MAX_HOURS).contains(&hours) => Err(format!(
                "Nieprawidłowy odstęp: {hours} h (dozwolone 1–{MAX_HOURS})."
            )),
            Schedule::Daily { hour } if hour > 23 => Err(format!("Nieprawidłowa godzina: {hour}.")),
            _ => Ok(()),
        }
    }

    pub fn enabled(&self) -> bool {
        self.schedule != Schedule::Off
    }
}

/// The windows (days) offered for the history source.
pub const HISTORY_DAYS: &[u16] = &[7, 14, 30];

/// Per-project sources of the update (`settings.json`, key `update_sources`):
/// the repositories always; the conversation history of Claude Code and
/// Codex only when turned on (default off).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UpdateSources {
    pub history: bool,
    pub history_days: u16,
}

impl Default for UpdateSources {
    fn default() -> Self {
        UpdateSources {
            history: false,
            history_days: 14,
        }
    }
}

impl UpdateSources {
    pub fn validate(&self) -> Result<(), String> {
        if HISTORY_DAYS.contains(&self.history_days) {
            Ok(())
        } else {
            Err(format!(
                "Nieprawidłowe okno historii: {} dni (dozwolone: 7, 14, 30).",
                self.history_days
            ))
        }
    }
}

/// When the next automatic check is due, given the last check (`clock`: the
/// start of the last run, manual or automatic, or of a skipped check).
/// `None` when the schedule is off. A time at or before `now` means due.
pub fn next_due(
    cfg: &AutoUpdate,
    clock: Option<NaiveDateTime>,
    now: NaiveDateTime,
) -> Option<NaiveDateTime> {
    match cfg.schedule {
        Schedule::Off => None,
        Schedule::Every { hours } => Some(match clock {
            Some(c) => c + TimeDelta::hours(i64::from(hours.max(1))),
            // Never checked: due now.
            None => now,
        }),
        Schedule::Daily { hour } => {
            let at =
                NaiveTime::from_hms_opt(u32::from(hour.min(23)), 0, 0).unwrap_or(NaiveTime::MIN);
            let today = now.date().and_time(at);
            // The most recent slot at or before now, and the one after it.
            let (last_slot, next_slot) = if now >= today {
                (today, today + TimeDelta::days(1))
            } else {
                (today - TimeDelta::days(1), today)
            };
            Some(match clock {
                // Not checked since the last slot: that slot is due (once).
                Some(c) if c < last_slot => last_slot,
                Some(_) => next_slot,
                None if now >= today => today,
                None => next_slot,
            })
        }
    }
}

/// One project as the scheduler sees it.
#[derive(Debug, Clone)]
pub struct Candidate<'a> {
    pub project: &'a str,
    pub cfg: &'a AutoUpdate,
    pub clock: Option<NaiveDateTime>,
}

/// The project to check now: the most overdue enabled one (ties by slug), or
/// none when nothing is due. Only active projects are passed in.
pub fn pick<'a>(now: NaiveDateTime, candidates: &[Candidate<'a>]) -> Option<&'a str> {
    candidates
        .iter()
        .filter_map(|c| {
            next_due(c.cfg, c.clock, now)
                .filter(|due| *due <= now)
                .map(|due| (due, c.project))
        })
        .min()
        .map(|(_, p)| p)
}

/// Settings entries that are invalid are dropped (they read as "off").
pub fn sanitize(map: BTreeMap<String, AutoUpdate>) -> BTreeMap<String, AutoUpdate> {
    map.into_iter()
        .filter(|(slug, cfg)| !slug.is_empty() && cfg.validate().is_ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        montybrain_core::minute::parse(s).unwrap()
    }

    fn every(hours: u32) -> AutoUpdate {
        AutoUpdate {
            schedule: Schedule::Every { hours },
            ..Default::default()
        }
    }

    fn daily(hour: u8) -> AutoUpdate {
        AutoUpdate {
            schedule: Schedule::Daily { hour },
            ..Default::default()
        }
    }

    #[test]
    fn every_n_hours() {
        let now = at("2026-11-14T12:00");
        assert_eq!(next_due(&AutoUpdate::default(), None, now), None);
        assert_eq!(
            next_due(&every(6), None, now),
            Some(now),
            "never checked → due"
        );
        assert_eq!(
            next_due(&every(6), Some(at("2026-11-14T08:00")), now),
            Some(at("2026-11-14T14:00"))
        );
        assert_eq!(
            next_due(&every(1), Some(at("2026-11-14T11:00")), now),
            Some(now)
        );
    }

    #[test]
    fn daily_at_hour() {
        let d = daily(9);
        // Before the slot, never checked: due at 9 today.
        assert_eq!(
            next_due(&d, None, at("2026-11-14T08:00")),
            Some(at("2026-11-14T09:00"))
        );
        // After the slot, never checked: due (once).
        assert_eq!(
            next_due(&d, None, at("2026-11-14T15:00")),
            Some(at("2026-11-14T09:00"))
        );
        // Checked yesterday: today's slot is due; app started late → overdue.
        assert_eq!(
            next_due(&d, Some(at("2026-11-13T09:05")), at("2026-11-14T15:00")),
            Some(at("2026-11-14T09:00"))
        );
        // Checked after today's slot (also a manual run): tomorrow.
        assert_eq!(
            next_due(&d, Some(at("2026-11-14T09:01")), at("2026-11-14T15:00")),
            Some(at("2026-11-15T09:00"))
        );
        // A manual run before the slot does not count for today's slot.
        assert_eq!(
            next_due(&d, Some(at("2026-11-14T08:30")), at("2026-11-14T08:45")),
            Some(at("2026-11-14T09:00"))
        );
        // Days without the app: one check, not one per missed day.
        assert_eq!(
            next_due(&d, Some(at("2026-11-01T09:00")), at("2026-11-14T10:00")),
            Some(at("2026-11-14T09:00"))
        );
        // Before today's slot, last check two days ago: yesterday's slot is due.
        assert_eq!(
            next_due(&d, Some(at("2026-11-12T09:00")), at("2026-11-14T07:00")),
            Some(at("2026-11-13T09:00"))
        );
    }

    #[test]
    fn pick_the_most_overdue_only() {
        let now = at("2026-11-14T12:00");
        let (a, b, c, off) = (every(6), every(2), daily(20), AutoUpdate::default());
        let cands = [
            Candidate {
                project: "a",
                cfg: &a,
                clock: Some(at("2026-11-14T05:00")),
            }, // due 11:00
            Candidate {
                project: "b",
                cfg: &b,
                clock: Some(at("2026-11-14T08:00")),
            }, // due 10:00
            Candidate {
                project: "c",
                cfg: &c,
                clock: None,
            }, // due 20:00
            Candidate {
                project: "d",
                cfg: &off,
                clock: None,
            },
        ];
        assert_eq!(pick(now, &cands), Some("b"));
        assert_eq!(pick(at("2026-11-14T09:00"), &cands), None);
        assert_eq!(pick(now, &[]), None);
    }

    #[test]
    fn validation_and_json() {
        assert!(every(0).validate().is_err());
        assert!(every(MAX_HOURS + 1).validate().is_err());
        assert!(every(1).validate().is_ok());
        assert!(daily(24).validate().is_err());
        assert!(daily(0).validate().is_ok());
        let cfg = AutoUpdate {
            schedule: Schedule::Every { hours: 6 },
            provider: ProviderId::Codex,
            on_result: OnResult::Apply,
        };
        let json = serde_json::to_value(cfg).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"schedule":{"kind":"every","hours":6},"provider":"codex","on_result":"apply"})
        );
        assert_eq!(serde_json::from_value::<AutoUpdate>(json).unwrap(), cfg);
        assert_eq!(
            serde_json::from_value::<AutoUpdate>(serde_json::json!({})).unwrap(),
            AutoUpdate::default()
        );
        assert!(serde_json::from_value::<AutoUpdate>(serde_json::json!({"x":1})).is_err());
        let src = UpdateSources {
            history: true,
            history_days: 30,
        };
        let json = serde_json::to_value(src).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"history": true, "history_days": 30})
        );
        assert_eq!(serde_json::from_value::<UpdateSources>(json).unwrap(), src);
        assert_eq!(UpdateSources::default().history_days, 14);
        assert!(!UpdateSources::default().history);
        assert!(
            UpdateSources {
                history: true,
                history_days: 10
            }
            .validate()
            .is_err()
        );
        let map = BTreeMap::from([
            ("ok".to_string(), every(3)),
            ("zle".to_string(), every(0)),
            (String::new(), every(3)),
        ]);
        assert_eq!(sanitize(map).keys().collect::<Vec<_>>(), ["ok"]);
    }
}
