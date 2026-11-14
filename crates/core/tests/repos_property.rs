//! Randomized (seeded, deterministic) property test of repository list editing
//! (`docs/format.md` §4, "Changing the list").
//!
//! Each case generates a hand-written-looking `repozytoria` list (unknown keys, nested
//! lists, comments inside and between items, trailing comments, blank lines, quoted
//! values, items without `nazwa`, mixed indentation, a duplicate key, CRLF) and a new
//! list derived from it (renames, path changes, removals, additions, reorders, all
//! combined). Invariants:
//!
//! * the save succeeds, or fails without touching the file;
//! * on success the file reads back as exactly the requested list, with no new
//!   warning, and saving the same list again changes nothing;
//! * the lines before and after the `repozytoria` entry are byte for byte the same;
//! * list-level text and items without `nazwa` are kept; every other item is kept
//!   whole or removed whole, and exactly `min(items, repos)` items are kept (an item
//!   is only dropped when the new list is shorter).
//!
//! Every comment and unknown line carries a unique token `Q<n>Z`, so "kept" is checked
//! by looking the tokens up in the result.

mod common;

use common::*;
use montybrain_core::*;

const PF: &str = "p/projekt.md";
/// Seeds `0..CASES`; `REPOS_PROPERTY_CASES=<n>` runs more.
const CASES: u64 = 600;

const NAMES: &[&str] = &["a", "b", "c", "d", "e", "x y", "#h", "it's"];
const PATHS: &[&str] = &["/a", "/b", "/c", "/d", "/p q", ""];

struct Gen {
    rng: fastrand::Rng,
    tok: usize,
}

/// One line of an item after its dash line.
enum L {
    /// `key: value` (or a comment) at the item's key column.
    Key(String),
    /// A line nested below the previous `Key` (two columns deeper).
    Nested(String),
    /// A comment at the dash's column (list-level indentation inside the item).
    DashComment(String),
    Blank,
}

struct Item {
    /// `(nazwa, sciezka)` as read, `None` for an item without `nazwa`.
    repo: Option<Repo>,
    lines: Vec<String>,
    tokens: Vec<String>,
}

impl Gen {
    fn token(&mut self) -> String {
        self.tok += 1;
        format!("Q{}Z", self.tok)
    }

    fn chance(&mut self, p: f64) -> bool {
        self.rng.f64() < p
    }

    fn pick<'a>(&mut self, xs: &'a [&'a str]) -> &'a str {
        xs[self.rng.usize(..xs.len())]
    }

    fn quoted(&mut self, v: &str) -> String {
        let plain_ok = !v.is_empty()
            && v.chars()
                .all(|c| c.is_ascii_alphanumeric() || "/ _".contains(c))
            && !v.ends_with(' ')
            && !matches!(v, "y" | "n");
        match self.rng.u8(..3) {
            0 if plain_ok => v.to_string(),
            1 if !v.contains('\'') => format!("'{v}'"),
            _ => format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\"")),
        }
    }

    fn comment(&mut self, tokens: &mut Vec<String>) -> String {
        if self.chance(0.3) {
            let t = self.token();
            let sep = if self.chance(0.3) { "\t" } else { "   " };
            tokens.push(t.clone());
            format!("{sep}# {t}")
        } else {
            String::new()
        }
    }

    fn item(&mut self, indent: &str) -> Item {
        let mut tokens = Vec::new();
        let name = self.chance(0.9).then(|| self.pick(NAMES).to_string());
        let path = self.chance(0.75).then(|| self.pick(PATHS).to_string());
        let mut lines: Vec<L> = Vec::new();
        if let Some(n) = &name {
            let v = self.quoted(n);
            let c = self.comment(&mut tokens);
            lines.push(L::Key(format!("nazwa: {v}{c}")));
        }
        if let Some(p) = &path {
            let v = self.quoted(p);
            let c = self.comment(&mut tokens);
            lines.push(L::Key(format!("sciezka: {v}{c}")));
        }
        if self.chance(0.4) {
            self.rng.shuffle(&mut lines);
        }
        for _ in 0..self.rng.usize(0..4) {
            let t = self.token();
            tokens.push(t.clone());
            let at = self.rng.usize(0..=lines.len());
            match self.rng.u8(..5) {
                0 => lines.insert(at, L::Key(format!("k{t}: v"))),
                1 => {
                    let t2 = self.token();
                    tokens.push(t2.clone());
                    lines.insert(at, L::Nested(format!("- {t2}")));
                    lines.insert(at, L::Key(format!("k{t}:")));
                }
                2 => lines.insert(at, L::Key(format!("# {t}"))),
                3 => lines.insert(at, L::DashComment(format!("# {t}"))),
                _ => {
                    tokens.pop();
                    lines.insert(at, L::Blank);
                }
            }
        }
        // A nested line needs a key right above it; drop strays the shuffle made.
        let mut fixed: Vec<L> = Vec::new();
        for l in lines {
            let ok = match &l {
                L::Nested(_) => {
                    matches!(fixed.last(), Some(L::Key(k) | L::Nested(k)) if !k.starts_with("nazwa") && !k.starts_with("sciezka") && !k.starts_with('#'))
                }
                _ => true,
            };
            if ok {
                fixed.push(l);
            } else if let L::Nested(t) = &l {
                tokens.retain(|x| !t.contains(x.as_str()));
            }
        }
        // Dash-column comments and blank lines belong to the item only when a
        // non-comment line of the item follows them (`docs/format.md` §4): drop the
        // ones after the last such line.
        let last = fixed.iter().rposition(|l| {
            matches!(l, L::Nested(_)) || matches!(l, L::Key(k) if !k.starts_with('#'))
        });
        let mut lines = Vec::new();
        for (i, l) in fixed.into_iter().enumerate() {
            if last.is_none_or(|last| i > last) && matches!(l, L::DashComment(_) | L::Blank) {
                if let L::DashComment(t) = &l {
                    tokens.retain(|x| !t.contains(x.as_str()));
                }
                continue;
            }
            lines.push(l);
        }
        if !lines
            .iter()
            .any(|l| matches!(l, L::Key(k) if !k.starts_with('#')))
        {
            let t = self.token();
            tokens.push(t.clone());
            lines.push(L::Key(format!("k{t}: v")));
        }
        // Dash line: a key on it, or a bare dash.
        let first_key = matches!(lines.first(), Some(L::Key(k)) if !k.starts_with('#'));
        let bare = !first_key || self.chance(0.2);
        let gap = if self.chance(0.2) { "   " } else { " " };
        let key_col = if bare {
            format!("{indent}{}", if self.chance(0.5) { "  " } else { "    " })
        } else {
            format!("{indent} {gap}")
        };
        let mut out = Vec::new();
        let mut rest = lines.into_iter();
        if bare {
            out.push(format!("{indent}-"));
        } else if let Some(L::Key(k)) = rest.next() {
            out.push(format!("{indent}-{gap}{k}"));
        }
        for l in rest {
            out.push(match l {
                L::Key(k) => format!("{key_col}{k}"),
                L::Nested(k) => format!("{key_col}  {k}"),
                L::DashComment(c) => format!("{indent}{c}"),
                L::Blank => String::new(),
            });
        }
        Item {
            repo: name.map(|n| Repo {
                name: n,
                path: path.unwrap_or_default(),
            }),
            lines: out,
            tokens,
        }
    }
}

struct Case {
    text: String,
    prefix: String,
    suffix: String,
    old: Vec<Repo>,
    /// Tokens of named items, per item.
    items: Vec<Vec<String>>,
    /// Tokens that must always survive (list-level text, items without `nazwa`).
    always: Vec<String>,
    new: Vec<Repo>,
}

fn generate(seed: u64) -> Case {
    let mut g = Gen {
        rng: fastrand::Rng::with_seed(seed),
        tok: 0,
    };
    let n = g.rng.usize(0..5);
    let mixed = g.chance(0.3);
    let base = g.pick(&["", "  ", "    "]).to_string();
    let mut indents: Vec<String> = (0..n)
        .map(|_| {
            if mixed {
                g.pick(&["", "  ", "    ", "   "]).to_string()
            } else {
                base.clone()
            }
        })
        .collect();
    // An item indented deeper than the one before it would be nested in it.
    indents.sort_by_key(|i| std::cmp::Reverse(i.len()));
    let mut always = Vec::new();
    let mut entry = Vec::new();
    let present = n > 0 || g.chance(0.8);
    if present {
        let flow = if n == 0 && g.chance(0.4) {
            if g.chance(0.5) { " []" } else { "\t[]" }
        } else {
            ""
        };
        let c = g.comment(&mut always);
        entry.push(format!("repozytoria:{flow}{c}"));
    }
    let mut items = Vec::new();
    let mut old = Vec::new();
    for (i, indent) in indents.iter().enumerate() {
        if i > 0 && g.chance(0.3) {
            entry.push(String::new());
        }
        if g.chance(0.25) {
            let t = g.token();
            always.push(t.clone());
            entry.push(format!("{indent}# {t}"));
        }
        let it = g.item(indent);
        entry.extend(it.lines);
        match it.repo {
            Some(r) => {
                old.push(r);
                items.push(it.tokens);
            }
            None => always.extend(it.tokens),
        }
    }
    if n > 0 && g.chance(0.25) {
        let t = g.token();
        always.push(t.clone());
        entry.push(format!("{}# {t}", indents[n - 1]));
    }
    // Without the key, a new one goes after `stan` (canonical order), so the text
    // around the entry is split there.
    let (prefix, mut suffix) = if present {
        ("---\nnazwa: P\n", "stan: aktywny\n".to_string())
    } else {
        ("---\nnazwa: P\nstan: aktywny\n", String::new())
    };
    let prefix = prefix.to_string();
    // A duplicate key further down (only behind a first one, which is authoritative).
    if present && g.chance(0.15) {
        suffix.push_str("repozytoria:\n  - nazwa: zz\n    sciezka: /zz\n");
    }
    suffix.push_str("---\n## Zadania\n");
    let mut text = prefix.clone();
    for l in &entry {
        text.push_str(l);
        text.push('\n');
    }
    text.push_str(&suffix);

    // The new list.
    let mut new: Vec<Repo> = Vec::new();
    for r in &old {
        if g.chance(0.15) {
            continue;
        }
        let mut r = r.clone();
        if g.chance(0.25) {
            r.name = g.pick(NAMES).to_string();
        }
        if g.chance(0.25) {
            r.path = g.pick(PATHS).to_string();
        }
        new.push(r);
    }
    if g.chance(0.35) {
        g.rng.shuffle(&mut new);
    }
    if g.chance(0.35) {
        for _ in 0..g.rng.usize(1..3) {
            let at = g.rng.usize(0..=new.len());
            let r = Repo {
                name: g.pick(NAMES).to_string(),
                path: g.pick(PATHS).to_string(),
            };
            new.insert(at, r);
        }
    }
    let crlf = g.chance(0.3);
    let conv = |s: String| if crlf { s.replace('\n', "\r\n") } else { s };
    Case {
        text: conv(text),
        prefix: conv(prefix),
        suffix: conv(suffix),
        old,
        items,
        always,
        new,
    }
}

fn warnings(v: &Vault) -> Vec<String> {
    let mut w: Vec<String> = v
        .load()
        .unwrap()
        .warnings
        .iter()
        .map(|w| w.message.clone())
        .collect();
    w.sort();
    w.dedup();
    w
}

/// Runs one case; `Ok(false)` if the save was refused (file untouched).
fn check(seed: u64) -> Result<bool, String> {
    let c = generate(seed);
    let (_d, v) = vault();
    write(&v, PF, &c.text);
    let loaded = v.load().unwrap().project("p").unwrap().repos.clone();
    if loaded != c.old {
        return Err(format!(
            "generator/parser disagree: {loaded:?} != {:?}",
            c.old
        ));
    }
    let warned = warnings(&v);
    let patch = || ProjectPatch {
        repos: Some(c.new.clone()),
        ..Default::default()
    };
    let p = match v.update_project("p", patch()) {
        Ok(p) => p,
        Err(e) => {
            if read(&v, PF) != c.text {
                return Err(format!("failed save changed the file: {e}"));
            }
            return Ok(false);
        }
    };
    let got = read(&v, PF);
    let ctx = || format!("\n--- before\n{}\n--- after\n{got}", c.text);
    if p.repos != c.new {
        return Err(format!("returned {:?}{}", p.repos, ctx()));
    }
    let reloaded = v.load().unwrap().project("p").unwrap().repos.clone();
    if reloaded != c.new {
        return Err(format!("reads back as {reloaded:?}{}", ctx()));
    }
    if !got.starts_with(&c.prefix) || !got.ends_with(&c.suffix) {
        return Err(format!("text outside the entry changed{}", ctx()));
    }
    if let Some(t) = c.always.iter().find(|t| !got.contains(t.as_str())) {
        return Err(format!("list-level text {t} lost{}", ctx()));
    }
    let mut kept = 0;
    for toks in &c.items {
        let n = toks.iter().filter(|t| got.contains(t.as_str())).count();
        if n != 0 && n != toks.len() {
            return Err(format!("item {toks:?} partly lost{}", ctx()));
        }
        // An item without tokens cannot be observed; count it as kept when possible.
        kept += usize::from(n == toks.len() && !toks.is_empty());
    }
    let observable = c.items.iter().filter(|t| !t.is_empty()).count();
    let dropped_max = c.old.len().saturating_sub(c.new.len());
    if observable - kept > dropped_max {
        return Err(format!(
            "{} item(s) dropped, but the list is only {dropped_max} shorter{}",
            observable - kept,
            ctx()
        ));
    }
    let new_warnings: Vec<String> = warnings(&v)
        .into_iter()
        .filter(|w| !warned.contains(w))
        .collect();
    if !new_warnings.is_empty() {
        return Err(format!("new warnings {new_warnings:?}{}", ctx()));
    }
    v.update_project("p", patch())
        .map_err(|e| format!("second save failed: {e}{}", ctx()))?;
    if read(&v, PF) != got {
        return Err(format!("second save is not a no-op{}", ctx()));
    }
    Ok(true)
}

#[test]
fn random_list_edits_keep_the_invariants() {
    let mut refused = Vec::new();
    let cases = std::env::var("REPOS_PROPERTY_CASES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(CASES);
    if let Ok(s) = std::env::var("REPOS_PROPERTY_SHOW") {
        let c = generate(s.parse().unwrap());
        println!("{}\n=> {:?}", c.text, c.new);
    }
    for seed in 0..cases {
        match check(seed) {
            Ok(true) => {}
            Ok(false) => refused.push(seed),
            Err(msg) => panic!("seed {seed}: {msg}"),
        }
    }
    // Every generated list is one MontyBrain supports, so no save may be refused.
    assert!(refused.is_empty(), "refused seeds: {refused:?}");
}
