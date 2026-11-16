//! Topic journal cursors belong to updates.json, never to the vault.
use std::collections::BTreeMap;

use montybrain_core::{JournalEntry, Snapshot};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalRefreshMarker {
    pub at: String,
    /// Content identities with multiplicity. Line numbers are deliberately absent:
    /// adding/removing Markdown above a journal entry must not make it new.
    pub seen: BTreeMap<String, usize>,
}

#[derive(Debug, Clone)]
pub struct JournalRefreshInput {
    pub entries: Vec<JournalEntry>,
    pub marker: JournalRefreshMarker,
}

pub fn key(e: &JournalEntry) -> String {
    serde_json::to_string(&(montybrain_core::minute::format(&e.at), &e.author, &e.text)).unwrap()
}

pub fn user_entries(snap: &Snapshot, project: &str, topic: &str) -> Vec<JournalEntry> {
    let mut entries: Vec<_> = snap
        .journal
        .iter()
        .filter(|e| {
            e.project == project
                && e.topic == topic
                && !matches!(e.author.as_str(), "codex" | "claude-code")
        })
        .cloned()
        .collect();
    entries.sort_by_key(|e| (e.at, std::cmp::Reverse(e.line)));
    entries
}

pub fn prepare(
    snap: &Snapshot,
    project: &str,
    topic: &str,
    previous: Option<&JournalRefreshMarker>,
) -> Result<JournalRefreshInput, String> {
    let user = user_entries(snap, project, topic);
    let mut seen = BTreeMap::new();
    for e in &user {
        *seen.entry(key(e)).or_insert(0) += 1;
    }
    let entries = if let Some(previous) = previous {
        let mut remaining = previous.seen.clone();
        user.iter()
            .filter(|e| {
                let count = remaining.entry(key(e)).or_default();
                if *count > 0 {
                    *count -= 1;
                    false
                } else {
                    true
                }
            })
            .cloned()
            .collect::<Vec<_>>()
    } else {
        user.iter()
            .skip(user.len().saturating_sub(5))
            .cloned()
            .collect()
    };
    let newest = entries.last().ok_or("Brak nowych wpisów.")?;
    // Preserve the high-water timestamp if a hand edit makes an older entry new.
    let at = previous
        .map(|p| p.at.clone())
        .unwrap_or_default()
        .max(montybrain_core::minute::format(&newest.at));
    Ok(JournalRefreshInput {
        entries,
        marker: JournalRefreshMarker { at, seen },
    })
}

pub fn request(input: &JournalRefreshInput) -> String {
    let entries = input
        .entries
        .iter()
        .map(|e| {
            format!(
                "[{}] {}\n{}",
                montybrain_core::minute::format(&e.at),
                e.author,
                e.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "Odświeżenie tematu · {}\n\n{entries}",
        super::plural(
            input.entries.len(),
            "nowy wpis",
            "nowe wpisy",
            "nowych wpisów"
        )
    )
}

/// Failures, cancellation and malformed answers never consume journal entries.
pub fn advance(
    state: &mut super::store::ProjectUpdate,
    topic: &str,
    input: &JournalRefreshInput,
    end: &super::store::RunEnd,
) {
    if matches!(end, super::store::RunEnd::Parsed(p) if p.problem.is_none()) {
        state
            .journal_refresh
            .insert(topic.into(), input.marker.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::proposal;
    use crate::updates::store::{ProjectUpdate, RunEnd, UpdateStore};

    fn snap() -> Snapshot {
        let mut snap = proposal::tests::snap();
        snap.journal = (0..7)
            .map(|i| JournalEntry {
                project: "beeking".into(),
                topic: "platnosci".into(),
                at: montybrain_core::minute::parse("2026-11-15T15:00").unwrap(),
                author: "ja".into(),
                text: format!("Wpis {i}"),
                line: 100 - i,
            })
            .collect();
        snap
    }

    #[test]
    fn first_use_last_five_user_entries_and_request_order() {
        let mut snap = snap();
        for author in ["codex", "claude-code"] {
            let mut e = snap.journal[0].clone();
            e.author = author.into();
            e.line = 1;
            snap.journal.push(e);
        }
        let input = prepare(&snap, "beeking", "platnosci", None).unwrap();
        assert_eq!(
            input
                .entries
                .iter()
                .map(|e| e.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Wpis 2", "Wpis 3", "Wpis 4", "Wpis 5", "Wpis 6"]
        );
        let request = request(&input);
        assert!(request.starts_with("Odświeżenie tematu · 5 nowych wpisów"));
        assert!(request.find("Wpis 2") < request.find("Wpis 6"));
        assert!(request.contains("[2026-11-15T15:00] ja"));
        assert!(!request.contains("codex"));
    }

    #[test]
    fn shifting_lines_removing_entries_editing_and_same_minute_duplicates() {
        let mut snap = snap();
        let marker = prepare(&snap, "beeking", "platnosci", None).unwrap().marker;
        snap.journal.iter_mut().for_each(|e| e.line += 50);
        assert!(prepare(&snap, "beeking", "platnosci", Some(&marker)).is_err());
        snap.journal.remove(0);
        assert!(prepare(&snap, "beeking", "platnosci", Some(&marker)).is_err());
        snap.journal[0].text = "Ręcznie poprawiony".into();
        let mut duplicate = snap.journal[1].clone();
        duplicate.line = 1;
        snap.journal.push(duplicate);
        let input = prepare(&snap, "beeking", "platnosci", Some(&marker)).unwrap();
        assert_eq!(input.entries.len(), 2);
        assert_eq!(input.entries[0].text, "Ręcznie poprawiony");
        assert_eq!(input.entries[1].text, "Wpis 2");
    }

    #[test]
    fn advances_only_for_usable_results_and_uses_start_snapshot() {
        let mut snap = snap();
        let input = prepare(&snap, "beeking", "platnosci", None).unwrap();
        let mut state = ProjectUpdate::default();
        let parsed = |json| {
            proposal::parse(
                &format!("```{}\n{json}\n```", proposal::FENCE_INFO),
                &snap,
                "beeking",
            )
        };
        for end in [
            RunEnd::Failed("x".into()),
            RunEnd::Cancelled,
            RunEnd::Parsed(parsed("not json")),
        ] {
            advance(&mut state, "platnosci", &input, &end);
            assert!(state.journal_refresh.is_empty());
        }
        for json in [
            r#"{"changes":[]}"#,
            r#"{"changes":[{"kind":"task_status","task":"aaaaa1","status":"done","reason":"r"}]}"#,
        ] {
            advance(
                &mut state,
                "platnosci",
                &input,
                &RunEnd::Parsed(parsed(json)),
            );
            assert_eq!(state.journal_refresh["platnosci"], input.marker);
        }
        let mut added = snap.journal[0].clone();
        added.text = "Podczas analizy".into();
        added.line = 1;
        snap.journal.push(added);
        let next = prepare(
            &snap,
            "beeking",
            "platnosci",
            state.journal_refresh.get("platnosci"),
        )
        .unwrap();
        assert_eq!(next.entries.len(), 1);
        assert_eq!(next.entries[0].text, "Podczas analizy");
    }

    #[test]
    fn marker_roundtrips_in_update_state_and_old_state_defaults() {
        let input = prepare(&snap(), "beeking", "platnosci", None).unwrap();
        let mut store = UpdateStore::default();
        store
            .project("beeking")
            .journal_refresh
            .insert("platnosci".into(), input.marker.clone());
        let json = serde_json::to_string(&store).unwrap();
        let restored: UpdateStore = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, store);
        assert_eq!(
            key(&input.entries[0]),
            r#"["2026-11-15T15:00","ja","Wpis 2"]"#
        );
        let old: UpdateStore = serde_json::from_str(r#"{"projects":{"beeking":{}}}"#).unwrap();
        assert!(old.projects["beeking"].journal_refresh.is_empty());
    }
}
