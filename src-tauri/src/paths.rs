//! Input checks for the commands that hand things to the OS (browser, Finder).
//! Vault files can be written by agents, so links and paths are validated
//! before they leave the app.

use std::path::{Component, Path, PathBuf};

/// URL schemes the app opens in the default handler.
const ALLOWED_SCHEMES: &[&str] = &["http", "https", "mailto"];

/// Returns the trimmed URL when its scheme is allowed.
pub fn checked_url(url: &str) -> Result<&str, String> {
    let url = url.trim();
    let scheme = url
        .split_once(':')
        .map(|(s, _)| s.to_ascii_lowercase())
        .filter(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        })
        .ok_or_else(|| format!("To nie jest adres URL: {url}"))?;
    if !ALLOWED_SCHEMES.contains(&scheme.as_str()) {
        return Err(format!("Nieobsługiwany rodzaj linku ({scheme}:)"));
    }
    if url.chars().any(char::is_whitespace) {
        return Err("Adres URL zawiera spacje".into());
    }
    Ok(url)
}

/// Resolves a vault-relative path (as found in the snapshot: `a/tematy/b.md`) to
/// an existing file or directory inside `root`.
///
/// Both the root and the target are canonicalized (symlinks resolved) and
/// containment is checked on the canonical paths, so a symlink inside the vault
/// cannot lead outside it. The returned path is the canonical one: hand exactly
/// that to the OS, not the path that was checked textually. A symlinked vault
/// root itself keeps working.
///
/// Known limitation (accepted for a single-user local app): this is a
/// validate-then-use check. Between the canonicalization here and the OS
/// opening the returned path, an ancestor directory could be swapped for a
/// symlink by something that can already write inside the vault. Closing that
/// would need an fd-based open (`openat` with `O_NOFOLLOW` per component),
/// which the "reveal / open in default app" hand-off does not allow.
pub fn resolve_in_vault(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel_path = Path::new(rel);
    let safe = !rel.is_empty()
        && rel_path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if !safe {
        return Err(format!("Nieprawidłowa ścieżka: {rel}"));
    }
    let root = root
        .canonicalize()
        .map_err(|e| format!("Nie można otworzyć sejfu: {e}"))?;
    // Fails for a missing file and for a dangling symlink.
    let full = root
        .join(rel_path)
        .canonicalize()
        .map_err(|_| format!("Plik nie istnieje: {rel}"))?;
    if !full.starts_with(&root) {
        return Err(format!("Ścieżka poza sejfem: {rel}"));
    }
    Ok(full)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(
            checked_url(" https://gitlab.com/a/-/merge_requests/1 "),
            Ok("https://gitlab.com/a/-/merge_requests/1")
        );
        assert!(checked_url("HTTP://example.com").is_ok());
        assert!(checked_url("mailto:ja@example.com").is_ok());
        assert!(checked_url("file:///etc/passwd").is_err());
        assert!(checked_url("javascript:alert(1)").is_err());
        assert!(checked_url("vscode://file/x").is_err());
        assert!(checked_url("nie-link").is_err());
        assert!(checked_url("https://a b").is_err());
    }

    #[test]
    fn vault_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("p/tematy")).unwrap();
        std::fs::write(dir.path().join("p/tematy/t.md"), "").unwrap();
        assert!(resolve_in_vault(dir.path(), "p/tematy/t.md").is_ok());
        assert!(resolve_in_vault(dir.path(), "p").is_ok());
        assert!(resolve_in_vault(dir.path(), "p/brak.md").is_err());
        assert!(resolve_in_vault(dir.path(), "../etc/passwd").is_err());
        assert!(resolve_in_vault(dir.path(), "p/../../x").is_err());
        assert!(resolve_in_vault(dir.path(), "/etc/passwd").is_err());
        assert!(resolve_in_vault(dir.path(), "").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape_the_vault() {
        use std::os::unix::fs::symlink;
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("outside.txt"), "x").unwrap();
        let vault = tempfile::tempdir().unwrap();
        std::fs::write(vault.path().join("in.md"), "").unwrap();
        symlink(outside.path(), vault.path().join("escape")).unwrap();
        symlink(
            outside.path().join("outside.txt"),
            vault.path().join("file"),
        )
        .unwrap();
        symlink(vault.path().join("nope"), vault.path().join("dangling")).unwrap();
        symlink(vault.path().join("in.md"), vault.path().join("inner")).unwrap();

        assert!(resolve_in_vault(vault.path(), "escape").is_err());
        assert!(resolve_in_vault(vault.path(), "escape/outside.txt").is_err());
        assert!(resolve_in_vault(vault.path(), "file").is_err());
        assert!(resolve_in_vault(vault.path(), "dangling").is_err());
        // A link that stays inside is fine and resolves to the real file.
        let inner = resolve_in_vault(vault.path(), "inner").unwrap();
        assert_eq!(inner, vault.path().canonicalize().unwrap().join("in.md"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_vault_root_keeps_working() {
        use std::os::unix::fs::symlink;
        let real = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(real.path().join("p")).unwrap();
        std::fs::write(real.path().join("p/t.md"), "").unwrap();
        let holder = tempfile::tempdir().unwrap();
        let link = holder.path().join("vault-link");
        symlink(real.path(), &link).unwrap();

        let got = resolve_in_vault(&link, "p/t.md").unwrap();
        assert_eq!(got, real.path().canonicalize().unwrap().join("p/t.md"));
        assert!(resolve_in_vault(&link, "../x").is_err());
    }
}
