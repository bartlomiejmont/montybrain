//! Slugs and diacritic folding.

/// Maximum slug length (bytes; slugs are ASCII).
const MAX_SLUG_LEN: usize = 60;

/// Transliterate one character to ASCII (Polish first, then common Latin letters).
/// Returns `None` for characters that have no ASCII mapping.
fn translit(c: char) -> Option<&'static str> {
    Some(match c {
        'ą' | 'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' => "a",
        'Ą' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' | 'Ă' => "A",
        'ć' | 'ç' | 'č' | 'ĉ' => "c",
        'Ć' | 'Ç' | 'Č' | 'Ĉ' => "C",
        'ę' | 'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ě' => "e",
        'Ę' | 'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ě' => "E",
        'ł' => "l",
        'Ł' => "L",
        'ń' | 'ñ' | 'ň' => "n",
        'Ń' | 'Ñ' | 'Ň' => "N",
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => "o",
        'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' | 'Ø' | 'Ō' | 'Ő' => "O",
        'ś' | 'š' | 'ş' => "s",
        'Ś' | 'Š' | 'Ş' => "S",
        'ź' | 'ż' | 'ž' => "z",
        'Ź' | 'Ż' | 'Ž' => "Z",
        'ì' | 'í' | 'î' | 'ï' => "i",
        'Ì' | 'Í' | 'Î' | 'Ï' => "I",
        'ù' | 'ú' | 'û' | 'ü' | 'ů' | 'ű' => "u",
        'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ů' | 'Ű' => "U",
        'ý' | 'ÿ' => "y",
        'Ý' => "Y",
        'ř' => "r",
        'Ř' => "R",
        'ď' => "d",
        'Ď' => "D",
        'ť' => "t",
        'Ť' => "T",
        'ß' => "ss",
        'æ' => "ae",
        'Æ' => "AE",
        'œ' => "oe",
        'Œ' => "OE",
        _ => return None,
    })
}

/// Lowercase + transliterate diacritics; used for case/diacritic-insensitive search.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match translit(c) {
            Some(t) => out.push_str(&t.to_ascii_lowercase()),
            None => out.extend(c.to_lowercase()),
        }
    }
    out
}

/// Derive a lowercase ASCII kebab-case slug from a name/title.
///
/// Polish (and common Latin) diacritics are transliterated, every other
/// non-alphanumeric run becomes a single `-`, the result is trimmed of `-` and
/// capped at 60 characters. An empty result becomes `bez-nazwy`.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    let mut push = |s: &str, out: &mut String| {
        for c in s.chars() {
            if c.is_ascii_alphanumeric() {
                if pending_dash && !out.is_empty() {
                    out.push('-');
                }
                pending_dash = false;
                out.push(c.to_ascii_lowercase());
            } else {
                pending_dash = true;
            }
        }
    };
    for c in name.chars() {
        if let Some(t) = translit(c) {
            push(t, &mut out);
        } else if c.is_ascii() {
            let mut b = [0u8; 4];
            push(c.encode_utf8(&mut b), &mut out);
        } else {
            // Unmappable non-ASCII characters act as separators.
            push("-", &mut out);
        }
    }
    if out.len() > MAX_SLUG_LEN {
        let cut = out[..MAX_SLUG_LEN]
            .rfind('-')
            .filter(|&i| i >= MAX_SLUG_LEN / 2);
        out.truncate(cut.unwrap_or(MAX_SLUG_LEN));
        while out.ends_with('-') {
            out.pop();
        }
    }
    if out.is_empty() {
        out.push_str("bez-nazwy");
    }
    out
}

/// Pick `base`, `base-2`, `base-3`, … — the first candidate for which `taken`
/// returns false.
pub fn unique_slug(base: &str, mut taken: impl FnMut(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    let mut n = 2usize;
    loop {
        let candidate = format!("{base}-{n}");
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// True if `s` is safe to use as a single path component for a project/topic slug.
pub(crate) fn is_safe_component(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && !s.contains(['/', '\\', '\0'])
        && s != ".."
        && s.len() <= 255
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_polish() {
        assert_eq!(slugify("Zażółć gęślą jaźń"), "zazolc-gesla-jazn");
        assert_eq!(slugify("ŁÓDŹ Śląsk"), "lodz-slask");
        assert_eq!(slugify("WhenUBuy"), "whenubuy");
        assert_eq!(slugify("  A/B testy -- v2!  "), "a-b-testy-v2");
        assert_eq!(slugify("Straße"), "strasse");
    }

    #[test]
    fn slugify_edge_cases() {
        assert_eq!(slugify(""), "bez-nazwy");
        assert_eq!(slugify("!!!"), "bez-nazwy");
        assert_eq!(slugify("日本語"), "bez-nazwy");
        assert_eq!(slugify("abc日本def"), "abc-def");
        let long = "słowo ".repeat(30);
        let s = slugify(&long);
        assert!(s.len() <= 60, "{s}");
        assert!(!s.ends_with('-'));
        assert!(s.starts_with("slowo-slowo"));
    }

    #[test]
    fn unique_slug_appends_counter() {
        let taken = ["a", "a-2"];
        assert_eq!(unique_slug("a", |s| taken.contains(&s)), "a-3");
        assert_eq!(unique_slug("b", |s| taken.contains(&s)), "b");
    }

    #[test]
    fn fold_is_case_and_diacritic_insensitive() {
        assert_eq!(fold("Żółw ŁĄKA"), "zolw laka");
    }

    #[test]
    fn safe_components() {
        assert!(is_safe_component("whenubuy"));
        assert!(!is_safe_component("../x"));
        assert!(!is_safe_component(".montybrain"));
        assert!(!is_safe_component("a/b"));
        assert!(!is_safe_component(""));
    }
}
