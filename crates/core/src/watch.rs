//! Debounced file watcher (feature `watch`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, Debouncer, new_debouncer};

use crate::error::{Error, Result};
use crate::vault::{LEGACY_META_DIR, LEGACY_TMP_SUFFIX, META_DIR, TMP_SUFFIX, Vault};

/// Default debounce interval.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(300);

/// Keeps watching while alive; drop it to stop.
pub struct VaultWatcher {
    _debouncer: Debouncer<RecommendedWatcher>,
}

impl std::fmt::Debug for VaultWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultWatcher").finish_non_exhaustive()
    }
}

/// True for paths the watcher should not report: anything under `.montybrain/`
/// or the legacy `.morwisko/`, atomic-save temp files (ours and an older
/// binary's), editor swap/backup files and `.DS_Store`.
pub fn is_ignored(rel: &Path) -> bool {
    if rel
        .components()
        .next()
        .is_some_and(|first| first.as_os_str() == META_DIR || first.as_os_str() == LEGACY_META_DIR)
    {
        return true;
    }
    let Some(name) = rel.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    name.ends_with(TMP_SUFFIX)
        || name.ends_with(LEGACY_TMP_SUFFIX)
        || name.ends_with('~')
        || name.ends_with(".swp")
        || name.ends_with(".swx")
        || name.ends_with(".tmp")
        || name.starts_with(".#")
        || name == ".DS_Store"
        || name == "4913" // vim's write-test file
}

impl Vault {
    /// Watch the vault recursively. `on_change` is called (on a background
    /// thread) with the de-duplicated list of changed paths, relative to the vault
    /// root, after changes settle for `debounce`. Ignored paths (see
    /// [`is_ignored`]) never trigger a call. Own writes are reported too.
    pub fn watch(
        &self,
        debounce: Duration,
        mut on_change: impl FnMut(Vec<PathBuf>) + Send + 'static,
    ) -> Result<VaultWatcher> {
        let root = self.root().to_path_buf();
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        let (r1, r2) = (root.clone(), canonical.clone());
        let mut debouncer = new_debouncer(debounce, move |res: DebounceEventResult| {
            let Ok(events) = res else { return };
            let mut paths: Vec<PathBuf> = events
                .into_iter()
                .filter_map(|e| {
                    e.path
                        .strip_prefix(&r2)
                        .or_else(|_| e.path.strip_prefix(&r1))
                        .map(Path::to_path_buf)
                        .ok()
                })
                .filter(|p| !p.as_os_str().is_empty() && !is_ignored(p))
                .collect();
            paths.sort();
            paths.dedup();
            if !paths.is_empty() {
                on_change(paths);
            }
        })
        .map_err(|e| Error::io(&root, std::io::Error::other(e)))?;
        debouncer
            .watcher()
            .watch(&canonical, RecursiveMode::Recursive)
            .map_err(|e| Error::io(&root, std::io::Error::other(e)))?;
        Ok(VaultWatcher {
            _debouncer: debouncer,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignore_rules() {
        assert!(is_ignored(Path::new(".montybrain/lock")));
        assert!(is_ignored(Path::new(".morwisko/lock")));
        assert!(is_ignored(Path::new(".morwisko/links.json")));
        assert!(is_ignored(Path::new("p/tematy/.a.md.123.0.morwisko-tmp")));
        assert!(is_ignored(Path::new("p/tematy/.a.md.123.0.montybrain-tmp")));
        assert!(is_ignored(Path::new("p/tematy/a.md~")));
        assert!(is_ignored(Path::new("p/.a.md.swp")));
        assert!(!is_ignored(Path::new("p/tematy/a.md")));
        assert!(!is_ignored(Path::new("skrzynka.md")));
        assert!(!is_ignored(Path::new("nowy-projekt")));
    }
}
