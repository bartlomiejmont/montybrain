//! Explicit branch ↔ topic links and ignored branches, in a sidecar file of
//! the vault (`.montybrain/links.json`, or `.morwisko/links.json` in a vault
//! that predates the rename; the loader and the watcher skip both), so the topic file format stays as it is and the links travel
//! with the vault. Written by applying proposals that name `branches`, by
//! "Przypisz do tematu…" and by "Ignoruj".

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

pub const FILE: &str = "links.json";

/// Serialises read-modify-write of the file within the app.
static WRITE: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BranchKey {
    pub project: String,
    pub repo: String,
    pub branch: String,
}

impl BranchKey {
    pub fn new(project: &str, repo: &str, branch: &str) -> BranchKey {
        BranchKey {
            project: project.into(),
            repo: repo.into(),
            branch: branch.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub project: String,
    pub repo: String,
    pub branch: String,
    pub topic: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LinkStore {
    pub links: Vec<Link>,
    pub ignored: Vec<BranchKey>,
}

impl LinkStore {
    fn key_of(l: &Link) -> BranchKey {
        BranchKey::new(&l.project, &l.repo, &l.branch)
    }

    pub fn topic_of(&self, key: &BranchKey) -> Option<&str> {
        self.links
            .iter()
            .find(|l| Self::key_of(l) == *key)
            .map(|l| l.topic.as_str())
    }

    pub fn is_ignored(&self, key: &BranchKey) -> bool {
        self.ignored.contains(key)
    }

    pub fn of_project(&self, project: &str) -> Vec<Link> {
        self.links
            .iter()
            .filter(|l| l.project == project)
            .cloned()
            .collect()
    }

    /// Links `key` to `topic` (replacing another link of it, un-ignoring it).
    /// Returns whether anything changed.
    pub fn link(&mut self, key: &BranchKey, topic: &str) -> bool {
        let before = self.clone();
        self.links.retain(|l| Self::key_of(l) != *key);
        self.ignored.retain(|k| k != key);
        self.links.push(Link {
            project: key.project.clone(),
            repo: key.repo.clone(),
            branch: key.branch.clone(),
            topic: topic.into(),
        });
        self.links
            .sort_by(|a, b| Self::key_of(a).cmp(&Self::key_of(b)));
        *self != before
    }

    /// Removes the link of `key` while it still points at `topic` (an undo).
    pub fn unlink(&mut self, key: &BranchKey, topic: &str) -> bool {
        let n = self.links.len();
        self.links
            .retain(|l| !(Self::key_of(l) == *key && l.topic == topic));
        self.links.len() != n
    }

    /// Hides `key` from the "unknown branches" signal (and drops its link).
    pub fn ignore(&mut self, key: &BranchKey) -> bool {
        let before = self.clone();
        self.links.retain(|l| Self::key_of(l) != *key);
        if !self.ignored.contains(key) {
            self.ignored.push(key.clone());
            self.ignored.sort();
        }
        *self != before
    }
}

/// In the vault's meta directory: `.montybrain/`, or the legacy `.morwisko/`
/// of a vault that already has one (see [`montybrain_core::meta_dir`]).
pub fn path(vault_root: &Path) -> PathBuf {
    montybrain_core::meta_dir(vault_root).join(FILE)
}

/// Missing or unreadable → empty (an unreadable file is logged and left alone
/// until the next write replaces it).
pub fn load(vault_root: &Path) -> LinkStore {
    let p = path(vault_root);
    match std::fs::read_to_string(&p) {
        Ok(t) => serde_json::from_str(&t).unwrap_or_else(|e| {
            crate::log(&format!("links: ignoring {}: {e}", p.display()));
            LinkStore::default()
        }),
        Err(_) => LinkStore::default(),
    }
}

/// Loads, changes with `f` and saves (when `f` says something changed).
pub fn update(vault_root: &Path, f: impl FnOnce(&mut LinkStore) -> bool) -> Result<bool, String> {
    let _g = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let mut store = load(vault_root);
    if !f(&mut store) {
        return Ok(false);
    }
    let p = path(vault_root);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Nie udało się zapisać powiązań: {e}"))?;
    }
    let mut json = serde_json::to_string_pretty(&store).map_err(|e| e.to_string())?;
    json.push('\n');
    crate::settings::write_atomic(&p, json.as_bytes())
        .map_err(|e| format!("Nie udało się zapisać powiązań: {e}"))?;
    Ok(true)
}
