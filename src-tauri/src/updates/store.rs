//! What is remembered per project about updates (`updates.json` in the config
//! dir, never in the vault): the schedule clock, the input fingerprint of the
//! last completed check, the last result and the latest proposal with what was
//! applied of it. Survives app restarts.

use std::collections::BTreeMap;
use std::path::Path;

use morwisko_core::chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

use super::schedule::{AutoUpdate, UpdateSources, next_due};
use crate::runs::proposal::{Change, Dropped, Proposal};
use crate::runs::{ProviderId, Trigger};

pub const STATE_FILE: &str = "updates.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    Manual,
    Auto,
}

impl From<Trigger> for TriggerKind {
    fn from(t: Trigger) -> Self {
        match t {
            Trigger::Manual => TriggerKind::Manual,
            Trigger::Auto => TriggerKind::Auto,
        }
    }
}

/// The outcome of the last check of a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// `count` changes proposed and waiting for review.
    Proposed {
        count: usize,
    },
    /// The agent proposed nothing.
    NoChanges,
    /// Changes of the proposal were applied (`count` in total so far).
    Applied {
        count: usize,
    },
    /// Automatic check skipped: nothing changed since the last check.
    Unchanged,
    /// The run succeeded but its answer had no usable proposal.
    NoProposal {
        message: String,
    },
    Failed {
        message: String,
        /// The CLI reported a usage / rate limit ([`is_limit_error`]).
        #[serde(default)]
        limit: bool,
        /// The CLI did not accept the model set in Settings
        /// ([`is_model_rejection`]); the message's first line says so.
        #[serde(default)]
        model: bool,
    },
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateResult {
    #[serde(with = "morwisko_core::minute")]
    pub at: NaiveDateTime,
    pub provider: Option<ProviderId>,
    pub trigger: TriggerKind,
    pub outcome: Outcome,
    /// What the history source contributed to that check, when it was on.
    #[serde(default)]
    pub history_note: Option<String>,
    /// The user's request when the run was a `zlecenie`, not a check.
    #[serde(default)]
    pub request: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    Applied,
    /// The target changed after the proposal ([`crate::runs::proposal::STALE`]).
    Stale,
    Failed,
    /// The user decided not to apply it.
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeOutcome {
    pub status: ChangeStatus,
    pub message: String,
    #[serde(with = "morwisko_core::minute")]
    pub at: NaiveDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredChange {
    pub change: Change,
    /// What happened when it was applied; `None` = not applied yet.
    pub outcome: Option<ChangeOutcome>,
    /// The user corrected the agent's version before applying it.
    #[serde(default)]
    pub edited: bool,
}

impl StoredChange {
    pub fn applied(&self) -> bool {
        self.outcome
            .as_ref()
            .is_some_and(|o| o.status == ChangeStatus::Applied)
    }
}

/// The proposal of one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredProposal {
    pub run_id: String,
    pub provider: ProviderId,
    pub trigger: TriggerKind,
    #[serde(with = "morwisko_core::minute")]
    pub created_at: NaiveDateTime,
    /// The agent's report (Markdown, the block removed).
    pub report: String,
    pub changes: Vec<StoredChange>,
    pub dropped: Vec<Dropped>,
    pub dropped_more: usize,
    /// No usable proposal: why.
    pub problem: Option<String>,
    /// What the history source contributed, when it was on.
    #[serde(default)]
    pub history_note: Option<String>,
    /// The user's request when the run was a `zlecenie`, not a check.
    #[serde(default)]
    pub request: Option<String>,
}

impl StoredProposal {
    pub fn from_proposal(
        p: Proposal,
        run_id: &str,
        provider: ProviderId,
        trigger: TriggerKind,
        at: NaiveDateTime,
    ) -> Self {
        StoredProposal {
            run_id: run_id.into(),
            provider,
            trigger,
            created_at: at,
            report: p.report,
            changes: p
                .changes
                .into_iter()
                .map(|change| StoredChange {
                    change,
                    outcome: None,
                    edited: false,
                })
                .collect(),
            dropped: p.dropped,
            dropped_more: p.dropped_more,
            problem: p.problem,
            history_note: None,
            request: None,
        }
    }

    /// Changes not applied (nor tried) yet.
    pub fn pending(&self) -> usize {
        self.changes.iter().filter(|c| c.outcome.is_none()).count()
    }

    pub fn applied(&self) -> usize {
        self.changes.iter().filter(|c| c.applied()).count()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectUpdate {
    /// The schedule clock: start of the last check (manual or automatic run,
    /// or a skipped automatic check).
    #[serde(with = "opt_minute")]
    pub clock: Option<NaiveDateTime>,
    /// Input fingerprint of the last check that produced a proposal.
    pub fingerprint: Option<String>,
    pub last: Option<UpdateResult>,
    pub proposal: Option<StoredProposal>,
    /// What the last apply click wrote, until it is undone or replaced.
    pub undo: Option<super::apply::UndoRecord>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateStore {
    pub projects: BTreeMap<String, ProjectUpdate>,
}

impl UpdateStore {
    /// Missing → empty; unreadable → empty + a log line (overwritten on the next save).
    pub fn load(path: &Path) -> UpdateStore {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                crate::log(&format!("updates: ignoring {}: {e}", path.display()));
                UpdateStore::default()
            }),
            Err(_) => UpdateStore::default(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        json.push('\n');
        crate::settings::write_atomic(path, json.as_bytes())
    }

    pub fn project(&mut self, slug: &str) -> &mut ProjectUpdate {
        self.projects.entry(slug.to_string()).or_default()
    }

    /// The project holding the proposal of `run_id`.
    pub fn find_run(&mut self, run_id: &str) -> Option<(&String, &mut ProjectUpdate)> {
        self.projects
            .iter_mut()
            .find(|(_, p)| p.proposal.as_ref().is_some_and(|x| x.run_id == run_id))
    }
}

/// What a finished `aktualizacja` or `zlecenie` run means for its project.
pub struct Finished<'a> {
    pub run_id: &'a str,
    pub provider: ProviderId,
    pub trigger: TriggerKind,
    pub started_at: NaiveDateTime,
    pub now: NaiveDateTime,
    pub fingerprint: Option<String>,
    pub history_note: Option<String>,
    /// The request of a `zlecenie` run: it neither counts as a check for the
    /// schedule nor sets the skip-if-unchanged fingerprint.
    pub request: Option<String>,
}

/// How a run ended, as far as updates care.
pub enum RunEnd {
    /// Succeeded, with its parsed answer.
    Parsed(Proposal),
    Failed(String),
    Cancelled,
}

/// Records a finished run in the project's state. Returns the stored
/// proposal's pending count when there is a new proposal with changes.
pub fn record_finished(st: &mut ProjectUpdate, f: &Finished, end: RunEnd) -> Option<usize> {
    // A manual check resets the schedule clock (an automatic one set it when
    // it started); a request is not a check.
    if f.trigger == TriggerKind::Manual && f.request.is_none() && !matches!(end, RunEnd::Cancelled)
    {
        st.clock = Some(st.clock.map_or(f.started_at, |c| c.max(f.started_at)));
    }
    let result = |outcome| UpdateResult {
        at: f.now,
        provider: Some(f.provider),
        trigger: f.trigger,
        outcome,
        history_note: f.history_note.clone(),
        request: f.request.clone(),
    };
    match end {
        RunEnd::Failed(message) => {
            let limit = is_limit_error(&message);
            let model = is_model_rejection(&message);
            st.last = Some(result(Outcome::Failed {
                message,
                limit,
                model,
            }));
            None
        }
        RunEnd::Cancelled => {
            st.last = Some(result(Outcome::Cancelled));
            None
        }
        RunEnd::Parsed(p) => {
            let mut stored =
                StoredProposal::from_proposal(p, f.run_id, f.provider, f.trigger, f.now);
            stored.history_note = f.history_note.clone();
            stored.request = f.request.clone();
            let outcome = match (&stored.problem, stored.changes.len()) {
                (Some(m), _) => Outcome::NoProposal { message: m.clone() },
                (None, 0) => Outcome::NoChanges,
                (None, n) => Outcome::Proposed { count: n },
            };
            if stored.problem.is_none() && f.request.is_none() {
                st.fingerprint = f.fingerprint.clone();
            }
            let pending = (stored.problem.is_none() && !stored.changes.is_empty())
                .then_some(stored.changes.len());
            st.last = Some(result(outcome));
            st.proposal = Some(stored);
            pending
        }
    }
}

/// Replaces the undecided change `index` of `p` with the user's correction,
/// validated like agent text ([`crate::runs::proposal::edit_change`]); on
/// error `p` is unchanged.
pub fn edit_change(
    p: &mut StoredProposal,
    index: usize,
    edit: &crate::runs::proposal::ChangeEdit,
    snap: &morwisko_core::Snapshot,
    project: &str,
) -> Result<(), String> {
    let sc = p.changes.get(index).ok_or("Nie ma takiej propozycji.")?;
    if sc.outcome.is_some() {
        return Err("Tej zmiany nie można już poprawić: została rozstrzygnięta.".into());
    }
    let others: Vec<&Change> = p
        .changes
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != index)
        .map(|(_, c)| &c.change)
        .collect();
    let edited = crate::runs::proposal::edit_change(&sc.change, edit, &others, snap, project)?;
    let sc = &mut p.changes[index];
    if edited != sc.change {
        sc.change = edited;
        sc.edited = true;
    }
    Ok(())
}

/// "Dołącz do tematu…": the undecided new topic `index` of `p` becomes a
/// journal entry of the existing topic `topic` plus its tasks there
/// ([`crate::runs::proposal::join_change`]); the entry takes its place, the
/// tasks are added at the end (indices of the other rows stay). Returns the
/// tasks left out because the topic has them already. On error `p` is
/// unchanged.
pub fn join_change(
    p: &mut StoredProposal,
    index: usize,
    topic: &str,
    snap: &morwisko_core::Snapshot,
    project: &str,
) -> Result<Vec<String>, String> {
    let sc = p.changes.get(index).ok_or("Nie ma takiej propozycji.")?;
    if sc.outcome.is_some() {
        return Err("Tej zmiany nie można już zmienić: została rozstrzygnięta.".into());
    }
    crate::runs::proposal::check_current(&sc.change, snap, project)
        .map_err(|e| format!("Nie można już zmienić tej propozycji: {e}."))?;
    let others: Vec<&Change> = p
        .changes
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != index)
        .map(|(_, c)| &c.change)
        .collect();
    let (mut joined, skipped) =
        crate::runs::proposal::join_change(&sc.change, topic, &others, snap, project)?;
    let limit = crate::runs::proposal::MAX_CHANGES + crate::runs::proposal::MAX_TOPIC_TASKS;
    if p.changes.len() + joined.len() - 1 > limit {
        return Err(format!("Za dużo zmian po dołączeniu (limit {limit})."));
    }
    let entry = joined.remove(0);
    p.changes[index] = StoredChange {
        change: entry,
        outcome: None,
        edited: true,
    };
    p.changes
        .extend(joined.into_iter().map(|change| StoredChange {
            change,
            outcome: None,
            edited: true,
        }));
    Ok(skipped)
}

/// Marks every undecided change of `p` as rejected; returns how many.
pub fn reject_pending(p: &mut StoredProposal, now: NaiveDateTime) -> usize {
    let mut n = 0;
    for c in p.changes.iter_mut().filter(|c| c.outcome.is_none()) {
        c.outcome = Some(ChangeOutcome {
            status: ChangeStatus::Rejected,
            message: "Odrzucone.".into(),
            at: now,
        });
        n += 1;
    }
    n
}

/// Whether a failure says the CLI did not accept the configured model (the
/// runner puts [`crate::runs::agent::model_rejected_message`] first).
pub fn is_model_rejection(message: &str) -> bool {
    message
        .lines()
        .next()
        .is_some_and(|l| l.contains("nie przyjął modelu"))
}

/// Whether a CLI error is a usage / rate limit (Claude Code: "usage limit
/// reached", "rate limit"; Codex: "You have hit your usage limit", HTTP 429).
pub fn is_limit_error(message: &str) -> bool {
    let m = message.to_lowercase();
    [
        "usage limit",
        "rate limit",
        "rate_limit",
        "limit reached",
        "limit will reset",
        "hit your limit",
        "quota",
        "too many requests",
        "429",
        "out of credits",
        "credit balance is too low",
    ]
    .iter()
    .any(|p| m.contains(p))
}

/// An automatic check that found nothing changed.
pub fn record_unchanged(st: &mut ProjectUpdate, provider: ProviderId, now: NaiveDateTime) {
    st.clock = Some(now);
    st.last = Some(UpdateResult {
        at: now,
        provider: Some(provider),
        trigger: TriggerKind::Auto,
        outcome: Outcome::Unchanged,
        history_note: None,
        request: None,
    });
}

/// An automatic check that could not start (provider missing, …): the error
/// is shown and the next attempt waits a full interval.
pub fn record_start_failure(
    st: &mut ProjectUpdate,
    provider: ProviderId,
    now: NaiveDateTime,
    message: String,
) {
    st.clock = Some(now);
    st.last = Some(UpdateResult {
        at: now,
        provider: Some(provider),
        trigger: TriggerKind::Auto,
        outcome: Outcome::Failed {
            limit: is_limit_error(&message),
            model: is_model_rejection(&message),
            message,
        },
        history_note: None,
        request: None,
    });
}

/// What the window gets per project (`list_updates`, `updates-changed`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectUpdateView {
    pub project: String,
    pub auto: AutoUpdate,
    /// Where the update looks (repositories always; history when on).
    pub sources: UpdateSources,
    /// Next automatic check (`null` when off).
    #[serde(with = "opt_minute")]
    pub next_due: Option<NaiveDateTime>,
    pub last: Option<UpdateResult>,
    pub proposal: Option<StoredProposal>,
    /// Changes of the proposal still waiting for review.
    pub pending: usize,
    /// Automatic checks are held back while proposals wait for review.
    pub held: bool,
    /// Token of the last apply click on this proposal that can still be undone.
    pub undo: Option<String>,
    /// That last action was a rejection ("Cofnij odrzucenie"), not a write.
    pub undo_rejection: bool,
}

/// Views of every project that has a setting or a state.
pub fn views(
    store: &UpdateStore,
    auto: &BTreeMap<String, AutoUpdate>,
    sources: &BTreeMap<String, UpdateSources>,
    now: NaiveDateTime,
) -> Vec<ProjectUpdateView> {
    let mut slugs: Vec<&String> = store
        .projects
        .keys()
        .chain(auto.keys())
        .chain(sources.keys())
        .collect();
    slugs.sort();
    slugs.dedup();
    slugs
        .into_iter()
        .map(|slug| {
            let st = store.projects.get(slug).cloned().unwrap_or_default();
            let cfg = auto.get(slug).copied().unwrap_or_default();
            let pending = st.proposal.as_ref().map_or(0, StoredProposal::pending);
            let held = cfg.enabled() && pending > 0;
            let (undo, undo_rejection) = match (&st.undo, &st.proposal) {
                (Some(r), Some(p)) if r.run_id == p.run_id => {
                    (Some(r.token.clone()), !r.items.iter().any(|i| i.writes()))
                }
                _ => (None, false),
            };
            ProjectUpdateView {
                project: slug.clone(),
                auto: cfg,
                sources: sources.get(slug).copied().unwrap_or_default(),
                next_due: if held {
                    None
                } else {
                    next_due(&cfg, st.clock, now).map(|d| d.max(now))
                },
                pending,
                held,
                undo,
                undo_rejection,
                last: st.last,
                proposal: st.proposal,
            }
        })
        .collect()
}

/// `Option<NaiveDateTime>` as `"YYYY-MM-DDTHH:MM"` or `null`.
mod opt_minute {
    use morwisko_core::chrono::NaiveDateTime;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<NaiveDateTime>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(t) => s.serialize_str(&morwisko_core::minute::format(t)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<NaiveDateTime>, D::Error> {
        let raw: Option<String> = Option::deserialize(d)?;
        Ok(raw.and_then(|s| morwisko_core::minute::parse(&s)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::proposal;
    use crate::updates::schedule::Schedule;

    fn at(s: &str) -> NaiveDateTime {
        morwisko_core::minute::parse(s).unwrap()
    }

    fn finished(trigger: TriggerKind) -> Finished<'static> {
        Finished {
            run_id: "r1",
            provider: ProviderId::Codex,
            trigger,
            started_at: at("2026-11-14T14:00"),
            now: at("2026-11-14T14:02"),
            fingerprint: Some("fp1".into()),
            history_note: None,
            request: None,
        }
    }

    fn parsed(json: &str) -> Proposal {
        proposal::parse(
            &format!("Raport.\n```{}\n{json}\n```", proposal::FENCE_INFO),
            &proposal::tests::snap(),
            "beeking",
        )
    }

    #[test]
    fn a_request_is_recorded_but_is_not_a_check() {
        let mut st = ProjectUpdate {
            clock: Some(at("2026-11-14T09:00")),
            fingerprint: Some("fp0".into()),
            ..Default::default()
        };
        let mut f = finished(TriggerKind::Manual);
        f.request = Some("Rozpisz temat kasy".into());
        f.history_note = Some("Historia rozmów jest wyłączona dla tego projektu (Źródła).".into());
        let p = parsed(
            r#"{"changes":[{"kind":"task_status","task":"aaaaa1","status":"done","reason":"r"}]}"#,
        );
        assert_eq!(record_finished(&mut st, &f, RunEnd::Parsed(p)), Some(1));
        assert_eq!(st.clock, Some(at("2026-11-14T09:00")));
        assert_eq!(st.fingerprint.as_deref(), Some("fp0"));
        let last = st.last.as_ref().unwrap();
        assert_eq!(last.request.as_deref(), Some("Rozpisz temat kasy"));
        assert_eq!(last.outcome, Outcome::Proposed { count: 1 });
        let prop = st.proposal.as_ref().unwrap();
        assert_eq!(prop.request.as_deref(), Some("Rozpisz temat kasy"));
        assert!(prop.history_note.as_deref().unwrap().contains("wyłączona"));
        // Older files without the field load as checks.
        let old: UpdateResult = serde_json::from_value(serde_json::json!({
            "at": "2026-11-14T10:00", "provider": "codex", "trigger": "manual",
            "outcome": {"kind": "no_changes"}
        }))
        .unwrap();
        assert_eq!(old.request, None);
    }

    #[test]
    fn finished_runs_are_recorded() {
        let mut st = ProjectUpdate::default();
        let p = parsed(
            r#"{"changes":[{"kind":"task_status","task":"aaaaa1","status":"done","reason":"r"},{"kind":"x"}]}"#,
        );
        assert_eq!(
            record_finished(&mut st, &finished(TriggerKind::Manual), RunEnd::Parsed(p)),
            Some(1)
        );
        assert_eq!(
            st.clock,
            Some(at("2026-11-14T14:00")),
            "manual run resets the clock"
        );
        assert_eq!(st.fingerprint.as_deref(), Some("fp1"));
        assert_eq!(
            st.last.as_ref().unwrap().outcome,
            Outcome::Proposed { count: 1 }
        );
        let prop = st.proposal.as_ref().unwrap();
        assert_eq!(
            (prop.pending(), prop.dropped.len(), prop.report.as_str()),
            (1, 1, "Raport.")
        );

        // A failure keeps the previous proposal and fingerprint.
        let mut f = finished(TriggerKind::Auto);
        f.fingerprint = Some("fp2".into());
        assert_eq!(
            record_finished(&mut st, &f, RunEnd::Failed("limit".into())),
            None
        );
        assert_eq!(
            st.last.as_ref().unwrap().outcome,
            Outcome::Failed {
                message: "limit".into(),
                limit: false,
                model: false
            }
        );
        assert!(st.proposal.is_some());
        assert_eq!(st.fingerprint.as_deref(), Some("fp1"));

        // No usable block: the report is kept, the fingerprint is not taken.
        let none = proposal::parse("Tylko raport.", &proposal::tests::snap(), "beeking");
        assert_eq!(record_finished(&mut st, &f, RunEnd::Parsed(none)), None);
        assert!(matches!(
            st.last.as_ref().unwrap().outcome,
            Outcome::NoProposal { .. }
        ));
        assert_eq!(st.proposal.as_ref().unwrap().report, "Tylko raport.");
        assert_eq!(st.fingerprint.as_deref(), Some("fp1"));

        // Nothing proposed.
        assert_eq!(
            record_finished(&mut st, &f, RunEnd::Parsed(parsed(r#"{"changes":[]}"#))),
            None
        );
        assert_eq!(st.last.as_ref().unwrap().outcome, Outcome::NoChanges);
        assert_eq!(st.fingerprint.as_deref(), Some("fp2"));

        // A cancelled manual run does not move the clock.
        let mut st = ProjectUpdate::default();
        record_finished(&mut st, &finished(TriggerKind::Manual), RunEnd::Cancelled);
        assert_eq!(st.clock, None);
        assert_eq!(st.last.unwrap().outcome, Outcome::Cancelled);
    }

    #[test]
    fn limit_errors_are_recognised() {
        for m in [
            "You have hit your usage limit. Try again later.\n(Codex CLI zakończył się kodem 1.)",
            "Claude AI usage limit reached|1731600000",
            "API Error: 429 Too Many Requests",
            "rate_limit_error: Number of request tokens has exceeded your rate limit",
            "Your 5-hour limit will reset at 3pm",
            "Quota exceeded",
        ] {
            assert!(is_limit_error(m), "{m}");
        }
        for m in [
            "Not logged in · Please run /login",
            "Claude Code zakończył się kodem 3.",
            "",
        ] {
            assert!(!is_limit_error(m), "{m}");
        }
        let mut st = ProjectUpdate::default();
        record_finished(
            &mut st,
            &finished(TriggerKind::Manual),
            RunEnd::Failed("You have hit your usage limit.".into()),
        );
        assert!(matches!(
            st.last.unwrap().outcome,
            Outcome::Failed { limit: true, .. }
        ));
        // Older files without the flag still load.
        let old: Outcome =
            serde_json::from_value(serde_json::json!({"kind":"failed","message":"x"})).unwrap();
        assert_eq!(
            old,
            Outcome::Failed {
                message: "x".into(),
                limit: false,
                model: false
            }
        );
    }

    #[test]
    fn rejecting_keeps_the_proposal_as_the_last_result() {
        let mut st = ProjectUpdate::default();
        let p = parsed(
            r#"{"changes":[{"kind":"task_status","task":"aaaaa1","status":"done","reason":"r"},{"kind":"new_task","title":"N","reason":"r"}]}"#,
        );
        record_finished(&mut st, &finished(TriggerKind::Manual), RunEnd::Parsed(p));
        let prop = st.proposal.as_mut().unwrap();
        prop.changes[0].outcome = Some(ChangeOutcome {
            status: ChangeStatus::Applied,
            message: "ok".into(),
            at: at("2026-11-14T14:03"),
        });
        assert_eq!(reject_pending(prop, at("2026-11-14T14:04")), 1);
        assert_eq!(prop.pending(), 0);
        assert_eq!(prop.applied(), 1);
        assert_eq!(
            prop.changes[1].outcome.as_ref().unwrap().status,
            ChangeStatus::Rejected
        );
        assert_eq!(reject_pending(prop, at("2026-11-14T14:05")), 0);
    }

    #[test]
    fn store_round_trip_and_views() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STATE_FILE);
        assert_eq!(UpdateStore::load(&path), UpdateStore::default());
        let mut store = UpdateStore::default();
        let p = parsed(r#"{"changes":[{"kind":"new_task","title":"Nowe","reason":"r"}]}"#);
        record_finished(
            store.project("beeking"),
            &finished(TriggerKind::Auto),
            RunEnd::Parsed(p),
        );
        record_unchanged(
            store.project("dom"),
            ProviderId::Claude,
            at("2026-11-14T15:00"),
        );
        store.save(&path).unwrap();
        let loaded = UpdateStore::load(&path);
        assert_eq!(loaded, store);
        assert!(loaded.clone().find_run("r1").is_some());
        assert!(loaded.clone().find_run("r2").is_none());
        std::fs::write(&path, "nie json").unwrap();
        assert_eq!(UpdateStore::load(&path), UpdateStore::default());

        let auto = BTreeMap::from([
            (
                "dom".to_string(),
                AutoUpdate {
                    schedule: Schedule::Every { hours: 6 },
                    ..Default::default()
                },
            ),
            ("zzz".to_string(), AutoUpdate::default()),
        ]);
        let v = views(&store, &auto, &BTreeMap::new(), at("2026-11-14T16:00"));
        assert_eq!(
            v.iter().map(|v| v.project.as_str()).collect::<Vec<_>>(),
            ["beeking", "dom", "zzz"]
        );
        assert_eq!(v[0].pending, 1);
        assert_eq!(v[0].next_due, None);
        assert!(!v[0].held, "auto off: nothing is held");
        assert_eq!(v[1].next_due, Some(at("2026-11-14T21:00")));
        let json = serde_json::to_value(&v[1]).unwrap();
        assert_eq!(json["next_due"], "2026-11-14T21:00");
        assert_eq!(json["last"]["outcome"]["kind"], "unchanged");
        assert_eq!(json["last"]["trigger"], "auto");
        let json = serde_json::to_value(&v[0]).unwrap();
        assert_eq!(json["proposal"]["changes"][0]["change"]["kind"], "new_task");
        assert!(json["proposal"]["changes"][0]["outcome"].is_null());
    }
}
