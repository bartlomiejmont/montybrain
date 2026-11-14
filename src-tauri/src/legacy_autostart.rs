//! The login item of the app before the rename.
//!
//! Autostart is a LaunchAgent named after the product
//! (`~/Library/LaunchAgents/<productName>.plist`, written by
//! `tauri-plugin-autostart`). The pre-rename app wrote `Morwisko.plist`,
//! pointing at `…/Morwisko.app/Contents/MacOS/morwisko-desktop`. The new name
//! gets a new file, so without cleanup the old one would keep starting the
//! old app (or point at nothing once it is deleted).
//!
//! The old file is removed only when it is ours: label `Morwisko` and a
//! program whose file name is `morwisko-desktop`. Anything else is left alone.

use std::path::{Path, PathBuf};

/// The old LaunchAgent's label and file stem.
pub const LEGACY_LABEL: &str = "Morwisko";
/// The old app's executable name.
pub const LEGACY_BINARY: &str = "morwisko-desktop";

/// `~/Library/LaunchAgents/Morwisko.plist`.
pub fn legacy_plist(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents")
        .join(format!("{LEGACY_LABEL}.plist"))
}

/// The first `<string>` after `<key>{key}</key>`.
fn string_after_key<'a>(plist: &'a str, key: &str) -> Option<&'a str> {
    let rest = &plist[plist.find(&format!("<key>{key}</key>"))?..];
    let start = rest.find("<string>")? + "<string>".len();
    let len = rest[start..].find("</string>")?;
    Some(rest[start..start + len].trim())
}

/// Whether a LaunchAgent plist is the one the pre-rename app wrote.
pub fn is_ours(plist: &str) -> bool {
    string_after_key(plist, "Label") == Some(LEGACY_LABEL)
        && string_after_key(plist, "ProgramArguments")
            .and_then(|p| Path::new(p).file_name())
            .is_some_and(|n| n == LEGACY_BINARY)
}

/// Whether `~/Library/LaunchAgents/Morwisko.plist` exists and is ours.
pub fn present(home: &Path) -> bool {
    std::fs::read_to_string(legacy_plist(home)).is_ok_and(|t| is_ours(&t))
}

/// Removes the old login item when it is ours. `Ok(true)` = removed.
pub fn remove(home: &Path) -> std::io::Result<bool> {
    if !present(home) {
        return Ok(false);
    }
    std::fs::remove_file(legacy_plist(home))?;
    Ok(true)
}

/// Whether the running executable is inside an app bundle (not a dev build),
/// the only case where startup carries an old login item over.
pub fn runs_from_bundle(exe: &Path) -> bool {
    exe.to_str()
        .is_some_and(|p| p.contains(".app/Contents/MacOS/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plist(label: &str, program: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n  <dict>\n  \
<key>Label</key>\n  <string>{label}</string>\n  <key>ProgramArguments</key>\n  \
<array><string>{program}</string><string>--hidden</string></array>\n  \
<key>RunAtLoad</key>\n  <true/>\n  </dict>\n</plist>"
        )
    }

    #[test]
    fn recognises_only_the_old_apps_login_item() {
        assert!(is_ours(&plist(
            "Morwisko",
            "/Applications/Morwisko.app/Contents/MacOS/morwisko-desktop"
        )));
        // Moved elsewhere: still the old binary.
        assert!(is_ours(&plist(
            "Morwisko",
            "/Users/x/Apps/Morwisko.app/Contents/MacOS/morwisko-desktop"
        )));
        assert!(!is_ours(&plist(
            "Morwisko",
            "/Applications/Other.app/Contents/MacOS/other"
        )));
        assert!(!is_ours(&plist(
            "Something",
            "/Applications/Morwisko.app/Contents/MacOS/morwisko-desktop"
        )));
        assert!(!is_ours("not a plist"));
        assert!(!is_ours(""));
    }

    #[test]
    fn removes_the_old_item_only_when_it_is_ours() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("Library/LaunchAgents");
        std::fs::create_dir_all(&dir).unwrap();
        // Absent: nothing to do.
        assert!(!remove(home.path()).unwrap());

        // Someone else's file under the same name stays.
        let foreign = plist("Morwisko", "/usr/local/bin/something-else");
        std::fs::write(dir.join("Morwisko.plist"), &foreign).unwrap();
        assert!(!present(home.path()));
        assert!(!remove(home.path()).unwrap());
        assert_eq!(
            std::fs::read_to_string(dir.join("Morwisko.plist")).unwrap(),
            foreign
        );

        // Ours goes; other agents are untouched.
        std::fs::write(
            dir.join("Morwisko.plist"),
            plist(
                "Morwisko",
                "/Applications/Morwisko.app/Contents/MacOS/morwisko-desktop",
            ),
        )
        .unwrap();
        std::fs::write(dir.join("MontyBrain.plist"), "x").unwrap();
        assert!(present(home.path()));
        assert!(remove(home.path()).unwrap());
        assert!(!dir.join("Morwisko.plist").exists());
        assert!(dir.join("MontyBrain.plist").exists());
        assert!(!remove(home.path()).unwrap());
    }

    #[test]
    fn bundle_detection() {
        assert!(runs_from_bundle(Path::new(
            "/Applications/MontyBrain.app/Contents/MacOS/montybrain-desktop"
        )));
        assert!(!runs_from_bundle(Path::new(
            "/Users/x/Projects/m/target/debug/montybrain-desktop"
        )));
    }
}
