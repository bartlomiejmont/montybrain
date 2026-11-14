//! Global shortcut strings.
//!
//! Stored form: modifiers in the order `Ctrl+Alt+Shift+Cmd`, then one key named
//! like `KeyboardEvent.code` without the `Key`/`Digit` prefix (`"Ctrl+Alt+Space"`,
//! `"Cmd+Shift+K"`, `"Alt+F5"`). This is also what `global-hotkey` parses.

use tauri_plugin_global_shortcut::Shortcut;

const MODIFIERS: [(&str, &[&str]); 4] = [
    ("Ctrl", &["CTRL", "CONTROL"]),
    ("Alt", &["ALT", "OPTION"]),
    ("Shift", &["SHIFT"]),
    ("Cmd", &["CMD", "COMMAND", "SUPER", "META"]),
];

/// Validates and normalizes a shortcut. A shortcut needs Ctrl, Alt or Cmd
/// (Shift alone would swallow normal typing), except for the function keys.
pub fn canonical(input: &str) -> Result<String, String> {
    let bad = || format!("Nieprawidłowy skrót: {input}");
    let tokens: Vec<&str> = input.split('+').map(str::trim).collect();
    let (key, mods) = tokens.split_last().ok_or_else(bad)?;
    let mut on = [false; 4];
    for m in mods {
        let up = m.to_ascii_uppercase();
        let i = MODIFIERS
            .iter()
            .position(|(_, names)| names.contains(&up.as_str()))
            .ok_or_else(bad)?;
        on[i] = true;
    }
    let key = normalize_key(key).ok_or_else(bad)?;
    let is_fn_key =
        key.len() > 1 && key.starts_with('F') && key[1..].chars().all(|c| c.is_ascii_digit());
    if !(on[0] || on[1] || on[3] || is_fn_key) {
        return Err("Skrót musi zawierać ⌃, ⌥ lub ⌘.".into());
    }
    let mut parts: Vec<&str> = MODIFIERS
        .iter()
        .zip(on)
        .filter(|(_, on)| *on)
        .map(|((name, _), _)| *name)
        .collect();
    parts.push(&key);
    let out = parts.join("+");
    out.parse::<Shortcut>().map_err(|_| bad())?;
    Ok(out)
}

/// "Ctrl+Alt+Space" → "⌃⌥Space" (macOS menu style).
pub fn pretty(canonical: &str) -> String {
    canonical
        .split('+')
        .map(|t| match t {
            "Ctrl" => "⌃",
            "Alt" => "⌥",
            "Shift" => "⇧",
            "Cmd" => "⌘",
            "Space" => "Spacja",
            other => other,
        })
        .collect()
}

fn normalize_key(key: &str) -> Option<String> {
    if key.is_empty() {
        return None;
    }
    let lower = key.to_ascii_lowercase();
    let k = if key.len() == 4 && lower.starts_with("key") {
        &key[3..]
    } else if key.len() == 6 && lower.starts_with("digit") {
        &key[5..]
    } else {
        key
    };
    let mods: Vec<&str> = MODIFIERS
        .iter()
        .flat_map(|(_, n)| n.iter().copied())
        .collect();
    if mods.contains(&k.to_ascii_uppercase().as_str()) {
        return None;
    }
    let mut chars = k.chars();
    let first = chars.next()?;
    Some(if k.len() == 1 {
        first.to_ascii_uppercase().to_string()
    } else {
        first.to_ascii_uppercase().to_string() + chars.as_str()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_forms() {
        assert_eq!(canonical("Ctrl+Alt+Space").unwrap(), "Ctrl+Alt+Space");
        assert_eq!(canonical("alt+control+space").unwrap(), "Ctrl+Alt+Space");
        assert_eq!(canonical("cmd+shift+keyk").unwrap(), "Shift+Cmd+K");
        assert_eq!(canonical("Shift+Cmd+KeyK").unwrap(), "Shift+Cmd+K");
        assert_eq!(canonical("Super+Digit1").unwrap(), "Cmd+1");
        assert_eq!(canonical("Option+ArrowUp").unwrap(), "Alt+ArrowUp");
        assert_eq!(canonical("F5").unwrap(), "F5");
        assert_eq!(canonical("Shift+F12").unwrap(), "Shift+F12");
    }

    #[test]
    fn pretty_names() {
        assert_eq!(pretty("Ctrl+Alt+Space"), "⌃⌥Spacja");
        assert_eq!(pretty("Shift+Cmd+K"), "⇧⌘K");
    }

    #[test]
    fn rejects() {
        assert!(canonical("").is_err());
        assert!(canonical("K").is_err(), "no modifier");
        assert!(canonical("Shift+K").is_err(), "shift alone");
        assert!(canonical("Ctrl+Alt").is_err(), "no key");
        assert!(canonical("Ctrl+Hyper+K").is_err());
        assert!(canonical("Ctrl+NoSuchKey").is_err());
        assert!(canonical("Ctrl++").is_err());
    }
}
