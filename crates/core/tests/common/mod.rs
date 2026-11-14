#![allow(dead_code)]

use std::fs;
use std::path::Path;

use morwisko_core::Vault;
use morwisko_core::chrono::{NaiveDate, NaiveDateTime};

pub const NOW: &str = "2026-11-14 09:15";

pub fn now() -> NaiveDateTime {
    NaiveDateTime::parse_from_str(NOW, "%Y-%m-%d %H:%M").unwrap()
}

pub fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

/// A fresh vault in a temp dir with a fixed clock.
pub fn vault() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap().with_clock(now);
    (dir, v)
}

pub fn write(v: &Vault, rel: &str, contents: &str) {
    let p = v.root().join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, contents).unwrap();
}

pub fn read(v: &Vault, rel: &str) -> String {
    fs::read_to_string(v.root().join(rel)).unwrap()
}

pub fn fixture(name: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// Minimal project with two repos.
pub const PROJECT: &str = "---\nnazwa: WhenUBuy\nkolor: \"#a3e635\"\nstan: aktywny\nrepozytoria:\n  - nazwa: whenubuy-backend\n    sciezka: /Users/x/Projects/WhenUBuy/whenubuy-backend\n  - nazwa: whenubuy-mobile\n    sciezka: /Users/x/Projects/WhenUBuy/whenubuy-mobile\n---\nOpis projektu.\n\n## Zadania\n- [ ] Project-level task ^k3f9a2\n";

/// Line diff (LCS based). Lines are split on `\n` keeping `\r`, so CRLF shows up.
/// Returns `-old` / `+new` entries in order; unchanged lines are omitted.
pub fn diff(before: &str, after: &str) -> Vec<String> {
    let a: Vec<&str> = before.split('\n').collect();
    let b: Vec<&str> = after.split('\n').collect();
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            i += 1;
            j += 1;
        } else if i < n && (j == m || lcs[i + 1][j] >= lcs[i][j + 1]) {
            out.push(format!("-{}", a[i]));
            i += 1;
        } else {
            out.push(format!("+{}", b[j]));
            j += 1;
        }
    }
    out
}
