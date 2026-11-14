//! The in-memory view of the vault that the window renders.
//!
//! `VaultState` owns the [`Vault`] and the latest [`Snapshot`]. Every reload
//! (watcher event or own mutation) runs `normalize()` + `load()` and replaces the
//! snapshot; the files on disk stay the single source of truth.

use std::sync::{Arc, Mutex};

use montybrain_core::chrono::{NaiveDate, NaiveDateTime};
use montybrain_core::{JournalEntry, Snapshot, TodayItem, Vault};
use serde::Serialize;

use crate::lock;

/// How many journal entries the "Dziś" view gets.
pub const RECENT_JOURNAL_LIMIT: usize = 30;

#[derive(Debug, Clone)]
struct Current {
    snapshot: Arc<Snapshot>,
    version: u64,
    load_error: Option<String>,
}

pub struct VaultState {
    vault: Vault,
    current: Mutex<Current>,
    /// Serializes reloads so an older load can never replace a newer one.
    reload_lock: Mutex<()>,
}

/// What `get_state` returns: the snapshot plus the derived lists the views need.
#[derive(Debug, Clone, Serialize)]
pub struct AppView {
    /// Increases every time the loaded snapshot changes.
    pub version: u64,
    /// Local date used for `today_items`.
    pub today: NaiveDate,
    /// Local "now" (minute precision), for relative times in the UI.
    #[serde(with = "montybrain_core::minute")]
    pub now: NaiveDateTime,
    pub snapshot: Arc<Snapshot>,
    pub today_items: Vec<TodayItem>,
    pub recent_journal: Vec<JournalEntry>,
    /// Set when the last reload failed (the previous snapshot is kept).
    pub load_error: Option<String>,
}

impl VaultState {
    /// Normalizes and loads the vault once. A failed load is kept as `load_error`
    /// with an empty snapshot, so the window can still open and show it.
    pub fn open(vault: Vault) -> Self {
        let (snapshot, load_error) = read(&vault);
        let snapshot = snapshot.unwrap_or_else(|| Snapshot {
            root: vault.root().to_path_buf(),
            ..Default::default()
        });
        VaultState {
            vault,
            current: Mutex::new(Current {
                snapshot: Arc::new(snapshot),
                version: 1,
                load_error,
            }),
            reload_lock: Mutex::new(()),
        }
    }

    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    pub fn version(&self) -> u64 {
        lock(&self.current).version
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        lock(&self.current).snapshot.clone()
    }

    /// Re-reads the vault. Returns the new version when anything visible changed,
    /// `None` when the files produced the same snapshot as before.
    pub fn reload(&self) -> Option<u64> {
        let _guard = lock(&self.reload_lock);
        let (snapshot, load_error) = read(&self.vault);
        let mut cur = lock(&self.current);
        let snapshot_changed = snapshot.as_ref().is_some_and(|s| *s != *cur.snapshot);
        if !snapshot_changed && load_error == cur.load_error {
            return None;
        }
        if let Some(s) = snapshot {
            cur.snapshot = Arc::new(s);
        }
        cur.load_error = load_error;
        cur.version += 1;
        Some(cur.version)
    }

    pub fn view(&self, today: NaiveDate, now: NaiveDateTime) -> AppView {
        let cur = lock(&self.current).clone();
        AppView {
            version: cur.version,
            today,
            now,
            today_items: cur.snapshot.today(today),
            recent_journal: cur.snapshot.recent_journal(RECENT_JOURNAL_LIMIT),
            snapshot: cur.snapshot,
            load_error: cur.load_error,
        }
    }
}

/// `normalize()` (best effort: a failure is logged, loading continues) + `load()`.
fn read(vault: &Vault) -> (Option<Snapshot>, Option<String>) {
    match vault.normalize() {
        Ok(r) if r.ids_assigned > 0 => crate::log(&format!(
            "normalize: {} id(s) assigned in {} file(s)",
            r.ids_assigned,
            r.files_changed.len()
        )),
        Ok(_) => {}
        Err(e) => crate::log(&format!("normalize failed: {e}")),
    }
    match vault.load() {
        Ok(s) => (Some(s), None),
        Err(e) => {
            crate::log(&format!("load failed: {e}"));
            (None, Some(e.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use montybrain_core::{NewTask, TaskLocation, chrono::NaiveDate};

    fn date(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn reload_bumps_version_only_on_change() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::init(dir.path()).unwrap();
        let st = VaultState::open(vault.clone());
        assert_eq!(st.version(), 1);
        assert_eq!(st.reload(), None, "nothing changed on disk");

        vault
            .create_task(
                &TaskLocation::Inbox,
                NewTask {
                    title: "Kupić mleko".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(st.reload(), Some(2));
        assert_eq!(st.snapshot().tasks.len(), 1);
        assert_eq!(st.reload(), None);
    }

    #[test]
    fn reload_normalizes_hand_written_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::init(dir.path()).unwrap();
        let st = VaultState::open(vault);
        std::fs::write(dir.path().join("skrzynka.md"), "- [ ] Ręcznie dopisane\n").unwrap();
        assert!(st.reload().is_some());
        let snap = st.snapshot();
        assert_eq!(snap.tasks.len(), 1);
        assert!(snap.tasks[0].id.is_some(), "normalize gave the task an id");
        let on_disk = std::fs::read_to_string(dir.path().join("skrzynka.md")).unwrap();
        assert!(on_disk.contains(" ^"));
    }

    #[test]
    fn missing_root_is_reported_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let vault = Vault::init(&root).unwrap();
        let st = VaultState::open(vault);
        std::fs::remove_dir_all(&root).unwrap();
        assert!(st.reload().is_some());
        let view = st.view(
            date("2026-11-13"),
            date("2026-11-13").and_hms_opt(9, 0, 0).unwrap(),
        );
        assert!(view.load_error.is_some());
        // Coming back clears the error.
        Vault::init(&root).unwrap();
        assert!(st.reload().is_some());
        assert!(
            st.view(
                date("2026-11-13"),
                date("2026-11-13").and_hms_opt(9, 0, 0).unwrap()
            )
            .load_error
            .is_none()
        );
    }

    #[test]
    fn view_serializes_with_expected_keys() {
        let dir = tempfile::tempdir().unwrap();
        let st = VaultState::open(Vault::init(dir.path()).unwrap());
        let v = st.view(
            date("2026-11-13"),
            date("2026-11-13").and_hms_opt(14, 2, 0).unwrap(),
        );
        let json = serde_json::to_value(&v).unwrap();
        for key in [
            "version",
            "today",
            "now",
            "snapshot",
            "today_items",
            "recent_journal",
            "load_error",
        ] {
            assert!(json.get(key).is_some(), "missing {key}");
        }
        assert_eq!(json["now"], "2026-11-13T14:02");
        assert_eq!(json["today"], "2026-11-13");
    }
}
