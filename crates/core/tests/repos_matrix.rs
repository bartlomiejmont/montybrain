//! Repository list editing (`repozytoria`, `docs/format.md` §4): every operation the
//! API allows (rename, change a path, add, remove, reorder, replace the whole list,
//! clear) against the shapes a hand-edited list takes. Each cell checks the exact
//! resulting text, that the untouched items are byte for byte the same, that the
//! file reads back as the requested list, that repeating the update changes
//! nothing, and that no new warning appears. Every cell also runs with CRLF.

mod common;

use common::*;
use montybrain_core::*;

struct Case {
    name: &'static str,
    /// The `repozytoria` entry (between `nazwa: P` and `stan: aktywny`).
    src: &'static str,
    set: &'static [(&'static str, &'static str)],
    want: &'static str,
    /// Item blocks of `src` that must come out unchanged.
    keep: &'static [&'static str],
}

const PF: &str = "p/projekt.md";

fn file(entry: &str) -> String {
    format!("---\nnazwa: P\n{entry}stan: aktywny\n---\n## Zadania\n")
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

fn run(c: &Case, crlf: bool) {
    let conv = |s: &str| {
        if crlf {
            s.replace('\n', "\r\n")
        } else {
            s.to_string()
        }
    };
    let tag = format!("{}{}", c.name, if crlf { " (CRLF)" } else { "" });
    let (_d, v) = vault();
    let src = conv(&file(c.src));
    write(&v, PF, &src);
    let warned = warnings(&v);
    let set: Vec<Repo> = c
        .set
        .iter()
        .map(|(n, p)| Repo {
            name: (*n).into(),
            path: (*p).into(),
        })
        .collect();
    let patch = || ProjectPatch {
        repos: Some(set.clone()),
        ..Default::default()
    };
    let p = v
        .update_project("p", patch())
        .unwrap_or_else(|e| panic!("{tag}: {e:?}"));
    let got = read(&v, PF);
    assert_eq!(got, conv(&file(c.want)), "{tag}: resulting text");
    for block in c.keep {
        let block = conv(block);
        assert!(
            src.contains(&block),
            "{tag}: bad case, {block:?} not in src"
        );
        assert!(got.contains(&block), "{tag}: item changed: {block:?}");
    }
    assert_eq!(p.repos, set, "{tag}: returned list");
    assert_eq!(
        v.load().unwrap().project("p").unwrap().repos,
        set,
        "{tag}: reloaded list"
    );
    let new_warnings: Vec<String> = warnings(&v)
        .into_iter()
        .filter(|w| !warned.contains(w))
        .collect();
    assert!(
        new_warnings.is_empty(),
        "{tag}: new warnings {new_warnings:?}"
    );
    v.update_project("p", patch()).unwrap();
    assert_eq!(read(&v, PF), got, "{tag}: second update is not a no-op");
}

fn run_all(cases: &[Case]) {
    for c in cases {
        run(c, false);
        run(c, true);
    }
}

// ---------------------------------------------------------------------------
// Unknown keys (and a nested list) inside items
// ---------------------------------------------------------------------------

const EXTRA: &str = "\
repozytoria:
  - nazwa: a
    sciezka: /a
    branch: main
  - nazwa: b
    sciezka: /b
    remote: origin
    tags:
      - x
";
const EXTRA_A: &str = "  - nazwa: a\n    sciezka: /a\n    branch: main\n";
const EXTRA_B: &str = "  - nazwa: b\n    sciezka: /b\n    remote: origin\n    tags:\n      - x\n";

#[test]
fn unknown_item_keys() {
    run_all(&[
        Case {
            name: "extra/rename",
            src: EXTRA,
            set: &[("a", "/a"), ("c", "/b")],
            want: "repozytoria:\n  - nazwa: a\n    sciezka: /a\n    branch: main\n  - nazwa: c\n    sciezka: /b\n    remote: origin\n    tags:\n      - x\n",
            keep: &[EXTRA_A],
        },
        Case {
            name: "extra/path",
            src: EXTRA,
            set: &[("a", "/a2"), ("b", "/b")],
            want: "repozytoria:\n  - nazwa: a\n    sciezka: /a2\n    branch: main\n  - nazwa: b\n    sciezka: /b\n    remote: origin\n    tags:\n      - x\n",
            keep: &[EXTRA_B],
        },
        Case {
            name: "extra/add",
            src: EXTRA,
            set: &[("a", "/a"), ("b", "/b"), ("c", "/c")],
            want: "repozytoria:\n  - nazwa: a\n    sciezka: /a\n    branch: main\n  - nazwa: b\n    sciezka: /b\n    remote: origin\n    tags:\n      - x\n  - nazwa: c\n    sciezka: /c\n",
            keep: &[EXTRA_A, EXTRA_B],
        },
        Case {
            name: "extra/remove",
            src: EXTRA,
            set: &[("b", "/b")],
            want: "repozytoria:\n  - nazwa: b\n    sciezka: /b\n    remote: origin\n    tags:\n      - x\n",
            keep: &[EXTRA_B],
        },
        Case {
            name: "extra/reorder",
            src: EXTRA,
            set: &[("b", "/b"), ("a", "/a")],
            want: "repozytoria:\n  - nazwa: b\n    sciezka: /b\n    remote: origin\n    tags:\n      - x\n  - nazwa: a\n    sciezka: /a\n    branch: main\n",
            keep: &[EXTRA_A, EXTRA_B],
        },
        Case {
            // Replacing every repo is read as editing each one in place. (`y` is
            // quoted: plain, it would read as a YAML 1.1 boolean.)
            name: "extra/replace",
            src: EXTRA,
            set: &[("x", "/x"), ("y", "/y")],
            want: "repozytoria:\n  - nazwa: x\n    sciezka: /x\n    branch: main\n  - nazwa: \"y\"\n    sciezka: /y\n    remote: origin\n    tags:\n      - x\n",
            keep: &[],
        },
        Case {
            name: "extra/clear",
            src: EXTRA,
            set: &[],
            want: "",
            keep: &[],
        },
    ]);
}

// ---------------------------------------------------------------------------
// Comments before, between, after and inside items; trailing comments on fields
// ---------------------------------------------------------------------------

const COMMENTS: &str = "\
repozytoria:   # repos
  # before
  - nazwa: a   # name a
    # inside a
    sciezka: /a   # path a
  # between
  - nazwa: b
  # list-level comment inside b
    sciezka: /b
  # after
";
const COMMENTS_A: &str = "  - nazwa: a   # name a\n    # inside a\n    sciezka: /a   # path a\n";
const COMMENTS_B: &str = "  - nazwa: b\n  # list-level comment inside b\n    sciezka: /b\n";

#[test]
fn comments() {
    run_all(&[
        Case {
            name: "comments/rename",
            src: COMMENTS,
            set: &[("c", "/a"), ("b", "/b")],
            want: "repozytoria:   # repos\n  # before\n  - nazwa: c   # name a\n    # inside a\n    sciezka: /a   # path a\n  # between\n  - nazwa: b\n  # list-level comment inside b\n    sciezka: /b\n  # after\n",
            keep: &[COMMENTS_B],
        },
        Case {
            name: "comments/path behind a list-level comment",
            src: COMMENTS,
            set: &[("a", "/a"), ("b", "/b2")],
            want: "repozytoria:   # repos\n  # before\n  - nazwa: a   # name a\n    # inside a\n    sciezka: /a   # path a\n  # between\n  - nazwa: b\n  # list-level comment inside b\n    sciezka: /b2\n  # after\n",
            keep: &[COMMENTS_A],
        },
        Case {
            name: "comments/rename and path with trailing comments",
            src: COMMENTS,
            set: &[("x y: z", "/new"), ("b", "/b")],
            want: "repozytoria:   # repos\n  # before\n  - nazwa: \"x y: z\"   # name a\n    # inside a\n    sciezka: /new   # path a\n  # between\n  - nazwa: b\n  # list-level comment inside b\n    sciezka: /b\n  # after\n",
            keep: &[COMMENTS_B],
        },
        Case {
            name: "comments/add",
            src: COMMENTS,
            set: &[("a", "/a"), ("b", "/b"), ("c", "/c")],
            want: "repozytoria:   # repos\n  # before\n  - nazwa: a   # name a\n    # inside a\n    sciezka: /a   # path a\n  # between\n  - nazwa: b\n  # list-level comment inside b\n    sciezka: /b\n  - nazwa: c\n    sciezka: /c\n  # after\n",
            keep: &[COMMENTS_A, COMMENTS_B],
        },
        Case {
            name: "comments/add first",
            src: COMMENTS,
            set: &[("c", "/c"), ("a", "/a"), ("b", "/b")],
            want: "repozytoria:   # repos\n  # before\n  - nazwa: c\n    sciezka: /c\n  - nazwa: a   # name a\n    # inside a\n    sciezka: /a   # path a\n  # between\n  - nazwa: b\n  # list-level comment inside b\n    sciezka: /b\n  # after\n",
            keep: &[COMMENTS_A, COMMENTS_B],
        },
        Case {
            name: "comments/remove",
            src: COMMENTS,
            set: &[("b", "/b")],
            want: "repozytoria:   # repos\n  # before\n  # between\n  - nazwa: b\n  # list-level comment inside b\n    sciezka: /b\n  # after\n",
            keep: &[COMMENTS_B],
        },
        Case {
            name: "comments/reorder",
            src: COMMENTS,
            set: &[("b", "/b"), ("a", "/a")],
            want: "repozytoria:   # repos\n  # before\n  - nazwa: b\n  # list-level comment inside b\n    sciezka: /b\n  # between\n  - nazwa: a   # name a\n    # inside a\n    sciezka: /a   # path a\n  # after\n",
            keep: &[COMMENTS_A, COMMENTS_B],
        },
        Case {
            name: "comments/replace",
            src: COMMENTS,
            set: &[("x", "/x")],
            want: "repozytoria:   # repos\n  # before\n  - nazwa: x   # name a\n    # inside a\n    sciezka: /x   # path a\n  # between\n  # after\n",
            keep: &[],
        },
        Case {
            name: "comments/clear",
            src: COMMENTS,
            set: &[],
            want: "repozytoria:   # repos\n  # before\n  # between\n  # after\n",
            keep: &[],
        },
    ]);
}

// ---------------------------------------------------------------------------
// Blank lines between items
// ---------------------------------------------------------------------------

const BLANKS: &str = "\
repozytoria:

  - nazwa: a
    sciezka: /a

  - nazwa: b
    sciezka: /b

  - nazwa: c
    sciezka: /c
";

#[test]
fn blank_lines() {
    run_all(&[
        Case {
            name: "blanks/remove middle",
            src: BLANKS,
            set: &[("a", "/a"), ("c", "/c")],
            want: "repozytoria:\n\n  - nazwa: a\n    sciezka: /a\n\n  - nazwa: c\n    sciezka: /c\n",
            keep: &[
                "  - nazwa: a\n    sciezka: /a\n",
                "  - nazwa: c\n    sciezka: /c\n",
            ],
        },
        Case {
            name: "blanks/remove first",
            src: BLANKS,
            set: &[("b", "/b"), ("c", "/c")],
            want: "repozytoria:\n\n  - nazwa: b\n    sciezka: /b\n\n  - nazwa: c\n    sciezka: /c\n",
            keep: &[],
        },
        Case {
            name: "blanks/remove last",
            src: BLANKS,
            set: &[("a", "/a"), ("b", "/b")],
            want: "repozytoria:\n\n  - nazwa: a\n    sciezka: /a\n\n  - nazwa: b\n    sciezka: /b\n",
            keep: &[],
        },
        Case {
            name: "blanks/remove two",
            src: BLANKS,
            set: &[("b", "/b")],
            want: "repozytoria:\n\n  - nazwa: b\n    sciezka: /b\n",
            keep: &[],
        },
        Case {
            name: "blanks/path",
            src: BLANKS,
            set: &[("a", "/a"), ("b", "/B"), ("c", "/c")],
            want: "repozytoria:\n\n  - nazwa: a\n    sciezka: /a\n\n  - nazwa: b\n    sciezka: /B\n\n  - nazwa: c\n    sciezka: /c\n",
            keep: &[],
        },
        Case {
            name: "blanks/add",
            src: BLANKS,
            set: &[("a", "/a"), ("b", "/b"), ("c", "/c"), ("d", "/d")],
            want: "repozytoria:\n\n  - nazwa: a\n    sciezka: /a\n\n  - nazwa: b\n    sciezka: /b\n\n  - nazwa: c\n    sciezka: /c\n  - nazwa: d\n    sciezka: /d\n",
            keep: &[],
        },
        Case {
            name: "blanks/reorder",
            src: BLANKS,
            set: &[("c", "/c"), ("a", "/a"), ("b", "/b")],
            want: "repozytoria:\n\n  - nazwa: c\n    sciezka: /c\n\n  - nazwa: a\n    sciezka: /a\n\n  - nazwa: b\n    sciezka: /b\n",
            keep: &[],
        },
        Case {
            name: "blanks/clear",
            src: BLANKS,
            set: &[],
            want: "",
            keep: &[],
        },
    ]);
}

// ---------------------------------------------------------------------------
// Indentation widths
// ---------------------------------------------------------------------------

const INDENT0: &str = "repozytoria:\n- nazwa: a\n  sciezka: /a\n- nazwa: b\n  sciezka: /b\n";
const INDENT4: &str = "repozytoria:\n    -   nazwa: a\n        sciezka: /a\n        x: 1\n";
const BARE_DASH: &str = "repozytoria:\n  -\n    nazwa: a\n    note: keep\n";

#[test]
fn indentation() {
    run_all(&[
        Case {
            name: "indent0/add",
            src: INDENT0,
            set: &[("a", "/a"), ("b", "/b"), ("c", "/c")],
            want: "repozytoria:\n- nazwa: a\n  sciezka: /a\n- nazwa: b\n  sciezka: /b\n- nazwa: c\n  sciezka: /c\n",
            keep: &[],
        },
        Case {
            name: "indent0/rename and path",
            src: INDENT0,
            set: &[("a", "/a"), ("c", "/c")],
            want: "repozytoria:\n- nazwa: a\n  sciezka: /a\n- nazwa: c\n  sciezka: /c\n",
            keep: &["- nazwa: a\n  sciezka: /a\n"],
        },
        Case {
            name: "indent0/remove",
            src: INDENT0,
            set: &[("b", "/b")],
            want: "repozytoria:\n- nazwa: b\n  sciezka: /b\n",
            keep: &["- nazwa: b\n  sciezka: /b\n"],
        },
        Case {
            name: "indent4/add",
            src: INDENT4,
            set: &[("a", "/a"), ("b", "/b")],
            want: "repozytoria:\n    -   nazwa: a\n        sciezka: /a\n        x: 1\n    -   nazwa: b\n        sciezka: /b\n",
            keep: &["    -   nazwa: a\n        sciezka: /a\n        x: 1\n"],
        },
        Case {
            name: "indent4/path",
            src: INDENT4,
            set: &[("a", "/z")],
            want: "repozytoria:\n    -   nazwa: a\n        sciezka: /z\n        x: 1\n",
            keep: &[],
        },
        Case {
            name: "bare dash/add path",
            src: BARE_DASH,
            set: &[("a", "/a")],
            want: "repozytoria:\n  -\n    nazwa: a\n    sciezka: /a\n    note: keep\n",
            keep: &[],
        },
        Case {
            name: "bare dash/add item",
            src: BARE_DASH,
            set: &[("a", ""), ("b", "/b")],
            want: "repozytoria:\n  -\n    nazwa: a\n    note: keep\n  - nazwa: b\n    sciezka: /b\n",
            keep: &["  -\n    nazwa: a\n    note: keep\n"],
        },
    ]);
}

// ---------------------------------------------------------------------------
// Quoted values, `sciezka` before `nazwa`, an item without `sciezka`
// ---------------------------------------------------------------------------

const QUOTED: &str = "\
repozytoria:
  - nazwa: \"a\"   # quoted
    sciezka: '/my path'  # single
  - nazwa: 'b'
    sciezka: \"/b\"
";
const PATH_FIRST: &str = "\
repozytoria:
  - sciezka: /a   # p
    nazwa: a
    note: keep
";
const NO_PATH: &str = "\
repozytoria:
  - nazwa: a   # c
    note: keep
  - nazwa: b
    sciezka: /b
";

#[test]
fn values_and_field_order() {
    run_all(&[
        Case {
            name: "quoted/rename",
            src: QUOTED,
            set: &[("a2", "/my path"), ("b", "/b")],
            want: "repozytoria:\n  - nazwa: a2   # quoted\n    sciezka: '/my path'  # single\n  - nazwa: 'b'\n    sciezka: \"/b\"\n",
            keep: &["  - nazwa: 'b'\n    sciezka: \"/b\"\n"],
        },
        Case {
            name: "quoted/path needing quotes",
            src: QUOTED,
            set: &[("a", "#x"), ("b", "/b")],
            want: "repozytoria:\n  - nazwa: \"a\"   # quoted\n    sciezka: \"#x\"  # single\n  - nazwa: 'b'\n    sciezka: \"/b\"\n",
            keep: &[],
        },
        Case {
            name: "quoted/empty path",
            src: QUOTED,
            set: &[("a", ""), ("b", "/b")],
            want: "repozytoria:\n  - nazwa: \"a\"   # quoted\n    sciezka: \"\"  # single\n  - nazwa: 'b'\n    sciezka: \"/b\"\n",
            keep: &[],
        },
        Case {
            name: "quoted/reorder",
            src: QUOTED,
            set: &[("b", "/b"), ("a", "/my path")],
            want: "repozytoria:\n  - nazwa: 'b'\n    sciezka: \"/b\"\n  - nazwa: \"a\"   # quoted\n    sciezka: '/my path'  # single\n",
            keep: &[],
        },
        Case {
            name: "path first/rename",
            src: PATH_FIRST,
            set: &[("b", "/a")],
            want: "repozytoria:\n  - sciezka: /a   # p\n    nazwa: b\n    note: keep\n",
            keep: &[],
        },
        Case {
            name: "path first/path",
            src: PATH_FIRST,
            set: &[("a", "/x")],
            want: "repozytoria:\n  - sciezka: /x   # p\n    nazwa: a\n    note: keep\n",
            keep: &[],
        },
        Case {
            name: "path first/add",
            src: PATH_FIRST,
            set: &[("a", "/a"), ("c", "/c")],
            want: "repozytoria:\n  - sciezka: /a   # p\n    nazwa: a\n    note: keep\n  - nazwa: c\n    sciezka: /c\n",
            keep: &["  - sciezka: /a   # p\n    nazwa: a\n    note: keep\n"],
        },
        Case {
            name: "no path/set path",
            src: NO_PATH,
            set: &[("a", "/a"), ("b", "/b")],
            want: "repozytoria:\n  - nazwa: a   # c\n    sciezka: /a\n    note: keep\n  - nazwa: b\n    sciezka: /b\n",
            keep: &["  - nazwa: b\n    sciezka: /b\n"],
        },
        Case {
            name: "no path/rename",
            src: NO_PATH,
            set: &[("z", ""), ("b", "/b")],
            want: "repozytoria:\n  - nazwa: z   # c\n    note: keep\n  - nazwa: b\n    sciezka: /b\n",
            keep: &[],
        },
        Case {
            name: "no path/remove other",
            src: NO_PATH,
            set: &[("a", "")],
            want: "repozytoria:\n  - nazwa: a   # c\n    note: keep\n",
            keep: &["  - nazwa: a   # c\n    note: keep\n"],
        },
    ]);
}

// ---------------------------------------------------------------------------
// Duplicate names
// ---------------------------------------------------------------------------

const DUPS: &str = "\
repozytoria:
  - nazwa: a
    sciezka: /1
    n: one
  - nazwa: a
    sciezka: /2
    n: two
";
const DUP1: &str = "  - nazwa: a\n    sciezka: /1\n    n: one\n";
const DUP2: &str = "  - nazwa: a\n    sciezka: /2\n    n: two\n";

#[test]
fn duplicate_names() {
    run_all(&[
        Case {
            name: "dups/remove first",
            src: DUPS,
            set: &[("a", "/2")],
            want: "repozytoria:\n  - nazwa: a\n    sciezka: /2\n    n: two\n",
            keep: &[DUP2],
        },
        Case {
            name: "dups/remove second",
            src: DUPS,
            set: &[("a", "/1")],
            want: "repozytoria:\n  - nazwa: a\n    sciezka: /1\n    n: one\n",
            keep: &[DUP1],
        },
        Case {
            name: "dups/path of second",
            src: DUPS,
            set: &[("a", "/1"), ("a", "/3")],
            want: "repozytoria:\n  - nazwa: a\n    sciezka: /1\n    n: one\n  - nazwa: a\n    sciezka: /3\n    n: two\n",
            keep: &[DUP1],
        },
        Case {
            name: "dups/both paths (list order)",
            src: DUPS,
            set: &[("a", "/x"), ("a", "/y")],
            want: "repozytoria:\n  - nazwa: a\n    sciezka: /x\n    n: one\n  - nazwa: a\n    sciezka: /y\n    n: two\n",
            keep: &[],
        },
        Case {
            name: "dups/swap",
            src: DUPS,
            set: &[("a", "/2"), ("a", "/1")],
            want: "repozytoria:\n  - nazwa: a\n    sciezka: /2\n    n: two\n  - nazwa: a\n    sciezka: /1\n    n: one\n",
            keep: &[DUP1, DUP2],
        },
        Case {
            name: "dups/add third",
            src: DUPS,
            set: &[("a", "/1"), ("a", "/2"), ("a", "/3")],
            want: "repozytoria:\n  - nazwa: a\n    sciezka: /1\n    n: one\n  - nazwa: a\n    sciezka: /2\n    n: two\n  - nazwa: a\n    sciezka: /3\n",
            keep: &[DUP1, DUP2],
        },
        Case {
            name: "dups/rename second",
            src: DUPS,
            set: &[("a", "/1"), ("b", "/2")],
            want: "repozytoria:\n  - nazwa: a\n    sciezka: /1\n    n: one\n  - nazwa: b\n    sciezka: /2\n    n: two\n",
            keep: &[DUP1],
        },
    ]);
}

// ---------------------------------------------------------------------------
// Odd but tolerated lists: comment splitting an item, an item without `nazwa`,
// a line at the dash's indentation, an empty commented list
// ---------------------------------------------------------------------------

const ODD: &str = "\
repozytoria:
  - nazwa: a
  # list comment
    sciezka: /a
  - sciezka: /orphan
    note: no name
  - nazwa: b
    sciezka: /b
  stray: line
";
const ODD_ORPHAN: &str = "  - sciezka: /orphan\n    note: no name\n";

#[test]
fn odd_lists() {
    run_all(&[
        Case {
            name: "odd/path behind a comment",
            src: ODD,
            set: &[("a", "/a2"), ("b", "/b")],
            want: "repozytoria:\n  - nazwa: a\n  # list comment\n    sciezka: /a2\n  - sciezka: /orphan\n    note: no name\n  - nazwa: b\n    sciezka: /b\n  stray: line\n",
            keep: &[ODD_ORPHAN, "  - nazwa: b\n    sciezka: /b\n"],
        },
        Case {
            name: "odd/remove",
            src: ODD,
            set: &[("b", "/b")],
            want: "repozytoria:\n  - sciezka: /orphan\n    note: no name\n  - nazwa: b\n    sciezka: /b\n  stray: line\n",
            keep: &[ODD_ORPHAN],
        },
        Case {
            name: "odd/add",
            src: ODD,
            set: &[("a", "/a"), ("b", "/b"), ("c", "/c")],
            want: "repozytoria:\n  - nazwa: a\n  # list comment\n    sciezka: /a\n  - sciezka: /orphan\n    note: no name\n  - nazwa: b\n    sciezka: /b\n  - nazwa: c\n    sciezka: /c\n  stray: line\n",
            keep: &[ODD_ORPHAN],
        },
        Case {
            name: "odd/rename",
            src: ODD,
            set: &[("a", "/a"), ("c", "/b")],
            want: "repozytoria:\n  - nazwa: a\n  # list comment\n    sciezka: /a\n  - sciezka: /orphan\n    note: no name\n  - nazwa: c\n    sciezka: /b\n  stray: line\n",
            keep: &[ODD_ORPHAN],
        },
        Case {
            name: "odd/reorder",
            src: ODD,
            set: &[("b", "/b"), ("a", "/a")],
            want: "repozytoria:\n  - nazwa: b\n    sciezka: /b\n  - sciezka: /orphan\n    note: no name\n  - nazwa: a\n  # list comment\n    sciezka: /a\n  stray: line\n",
            keep: &[ODD_ORPHAN],
        },
        Case {
            name: "odd/clear",
            src: ODD,
            set: &[],
            want: "repozytoria:\n  - sciezka: /orphan\n    note: no name\n  stray: line\n",
            keep: &[ODD_ORPHAN],
        },
        Case {
            name: "empty commented/add",
            src: "repozytoria:   # none yet\n  # add repos below\n",
            set: &[("a", "/a")],
            want: "repozytoria:   # none yet\n  # add repos below\n  - nazwa: a\n    sciezka: /a\n",
            keep: &[],
        },
    ]);
}

#[test]
fn odd_lines_are_reported() {
    let (_d, v) = vault();
    write(&v, PF, &file(ODD));
    let snap = v.load().unwrap();
    assert_eq!(
        snap.project("p").unwrap().repos,
        vec![
            Repo {
                name: "a".into(),
                path: "/a".into()
            },
            Repo {
                name: "b".into(),
                path: "/b".into()
            }
        ]
    );
    let lines: Vec<(Option<usize>, bool)> = snap
        .warnings
        .iter()
        .map(|w| (w.line, w.message.contains("without `nazwa`")))
        .collect();
    // 1-based: the orphan item (line 7) and the stray line (line 11).
    assert!(lines.contains(&(Some(7), true)), "{:?}", snap.warnings);
    assert!(
        snap.warnings
            .iter()
            .any(|w| w.line == Some(11) && w.message.contains("not inside an item")),
        "{:?}",
        snap.warnings
    );
}

// ---------------------------------------------------------------------------
// Review round 3
// ---------------------------------------------------------------------------

/// Q1: a rename combined with a reorder keeps the renamed item's other lines.
const PRECIOUS: &str = "\
repozytoria:
  - nazwa: a
    sciezka: /a
    precious: KEEP
  - nazwa: b
    sciezka: /b
";

#[test]
fn rename_with_reorder_keeps_unknown_lines() {
    run_all(&[
        Case {
            // The reviewer's reproduction: `c` continues `a` (same `sciezka`).
            name: "precious/rename and reorder",
            src: PRECIOUS,
            set: &[("b", "/b"), ("c", "/a")],
            want: "repozytoria:\n  - nazwa: b\n    sciezka: /b\n  - nazwa: c\n    sciezka: /a\n    precious: KEEP\n",
            keep: &["  - nazwa: b\n    sciezka: /b\n"],
        },
        Case {
            // Nothing ties `c` to `a`, but a repository is left over and so is an
            // item: pairing them keeps the item's lines instead of deleting them.
            name: "precious/rename, new path and reorder",
            src: PRECIOUS,
            set: &[("b", "/b"), ("c", "/c")],
            want: "repozytoria:\n  - nazwa: b\n    sciezka: /b\n  - nazwa: c\n    sciezka: /c\n    precious: KEEP\n",
            keep: &["  - nazwa: b\n    sciezka: /b\n"],
        },
        Case {
            name: "precious/leftover item and repo plus an addition",
            src: PRECIOUS,
            set: &[("b", "/b"), ("c", "/c"), ("d", "/d")],
            want: "repozytoria:\n  - nazwa: b\n    sciezka: /b\n  - nazwa: c\n    sciezka: /c\n    precious: KEEP\n  - nazwa: d\n    sciezka: /d\n",
            keep: &["  - nazwa: b\n    sciezka: /b\n"],
        },
        Case {
            // Strictly shorter: only now an item is dropped.
            name: "precious/shorter",
            src: PRECIOUS,
            set: &[("b", "/b")],
            want: "repozytoria:\n  - nazwa: b\n    sciezka: /b\n",
            keep: &["  - nazwa: b\n    sciezka: /b\n"],
        },
        Case {
            // A unique unchanged path wins over the place in the list.
            name: "precious/path decides between two renames",
            src: PRECIOUS,
            set: &[("y", "/b"), ("x", "/a")],
            want: "repozytoria:\n  - nazwa: \"y\"\n    sciezka: /b\n  - nazwa: x\n    sciezka: /a\n    precious: KEEP\n",
            keep: &[],
        },
    ]);
}

/// Q2: items with different indentation can be reordered; a moved item takes the
/// indentation of the place it moves to, keeping its inner layout.
const MIXED: &str = "\
repozytoria:
  - nazwa: a
    sciezka: /a
    # inside a
    tags:
      - x
- nazwa: b
  sciezka: /b
";

#[test]
fn mixed_indentation() {
    run_all(&[
        Case {
            name: "mixed/reorder",
            src: MIXED,
            set: &[("b", "/b"), ("a", "/a")],
            want: "repozytoria:\n  - nazwa: b\n    sciezka: /b\n- nazwa: a\n  sciezka: /a\n  # inside a\n  tags:\n    - x\n",
            keep: &[],
        },
        Case {
            name: "mixed/add after the outdented item",
            src: MIXED,
            set: &[("a", "/a"), ("b", "/b"), ("c", "/c")],
            want: "repozytoria:\n  - nazwa: a\n    sciezka: /a\n    # inside a\n    tags:\n      - x\n- nazwa: b\n  sciezka: /b\n- nazwa: c\n  sciezka: /c\n",
            keep: &["  - nazwa: a\n    sciezka: /a\n    # inside a\n    tags:\n      - x\n"],
        },
        Case {
            name: "mixed/reorder and add",
            src: MIXED,
            set: &[("c", "/c"), ("b", "/b"), ("a", "/a")],
            want: "repozytoria:\n  - nazwa: c\n    sciezka: /c\n  - nazwa: b\n    sciezka: /b\n- nazwa: a\n  sciezka: /a\n  # inside a\n  tags:\n    - x\n",
            keep: &[],
        },
        Case {
            name: "mixed/reorder deeper",
            src: "repozytoria:\n    - nazwa: a\n      sciezka: /a\n  - nazwa: b\n    note: keep\n- nazwa: c\n",
            set: &[("c", ""), ("a", "/a"), ("b", "")],
            want: "repozytoria:\n    - nazwa: c\n  - nazwa: a\n    sciezka: /a\n- nazwa: b\n  note: keep\n",
            keep: &[],
        },
    ]);
}

/// Q3: a repeated `repozytoria` key. The first one is authoritative; the second is
/// kept (and reported) and must never take over.
const DUP_KEY: &str = "\
repozytoria:
  - nazwa: a
    sciezka: /a
  - nazwa: b
    sciezka: /b
repozytoria:
  - nazwa: z
    sciezka: /z
";
const DUP_KEY_SECOND: &str = "repozytoria:\n  - nazwa: z\n    sciezka: /z\n";

#[test]
fn duplicate_repos_key() {
    run_all(&[
        Case {
            name: "dup key/clear",
            src: DUP_KEY,
            set: &[],
            want: "repozytoria: []\nrepozytoria:\n  - nazwa: z\n    sciezka: /z\n",
            keep: &[DUP_KEY_SECOND],
        },
        Case {
            name: "dup key/remove",
            src: DUP_KEY,
            set: &[("b", "/b")],
            want: "repozytoria:\n  - nazwa: b\n    sciezka: /b\nrepozytoria:\n  - nazwa: z\n    sciezka: /z\n",
            keep: &[DUP_KEY_SECOND],
        },
        Case {
            name: "dup key/add",
            src: DUP_KEY,
            set: &[("a", "/a"), ("b", "/b"), ("c", "/c")],
            want: "repozytoria:\n  - nazwa: a\n    sciezka: /a\n  - nazwa: b\n    sciezka: /b\n  - nazwa: c\n    sciezka: /c\nrepozytoria:\n  - nazwa: z\n    sciezka: /z\n",
            keep: &[DUP_KEY_SECOND],
        },
        Case {
            name: "dup key/rename and reorder",
            src: DUP_KEY,
            set: &[("b", "/b"), ("c", "/a")],
            want: "repozytoria:\n  - nazwa: b\n    sciezka: /b\n  - nazwa: c\n    sciezka: /a\nrepozytoria:\n  - nazwa: z\n    sciezka: /z\n",
            keep: &[DUP_KEY_SECOND],
        },
        Case {
            name: "dup key/first empty, add",
            src: "repozytoria: []\nrepozytoria:\n  - nazwa: z\n    sciezka: /z\n",
            set: &[("a", "/a")],
            want: "repozytoria:\n  - nazwa: a\n    sciezka: /a\nrepozytoria:\n  - nazwa: z\n    sciezka: /z\n",
            keep: &[DUP_KEY_SECOND],
        },
        Case {
            name: "dup key/first bare, add",
            src: "repozytoria:\nrepozytoria:\n  - nazwa: z\n    sciezka: /z\n",
            set: &[("a", "/a")],
            want: "repozytoria:\n  - nazwa: a\n    sciezka: /a\nrepozytoria:\n  - nazwa: z\n    sciezka: /z\n",
            keep: &[DUP_KEY_SECOND],
        },
    ]);
}

#[test]
fn duplicate_repos_key_is_reported() {
    let (_d, v) = vault();
    write(&v, PF, &file(DUP_KEY));
    let snap = v.load().unwrap();
    assert_eq!(
        snap.project("p").unwrap().repos,
        vec![
            Repo {
                name: "a".into(),
                path: "/a".into()
            },
            Repo {
                name: "b".into(),
                path: "/b".into()
            }
        ]
    );
    // 1-based line 8: the second `repozytoria:`.
    assert!(
        snap.warnings
            .iter()
            .any(|w| w.line == Some(8) && w.message.contains("duplicate")),
        "{:?}",
        snap.warnings
    );
}

/// Q4: a tab before the comment of `repozytoria: []` is a comment separator for the
/// reader as it is for the writer.
#[test]
fn empty_flow_list_with_tab_comment() {
    for src in [
        "repozytoria: []\t# keep\n",
        "repozytoria:\t[] # keep\n",
        "repozytoria:\t# keep\n",
    ] {
        let (_d, v) = vault();
        write(&v, PF, &file(src));
        let snap = v.load().unwrap();
        assert!(snap.warnings.is_empty(), "{src:?}: {:?}", snap.warnings);
        assert!(snap.project("p").unwrap().repos.is_empty());
    }
    // `[]#x` is not a comment (YAML needs whitespace before `#`): both sides refuse.
    let (_d, v) = vault();
    let src = file("repozytoria: []#x\n");
    write(&v, PF, &src);
    assert!(
        v.load()
            .unwrap()
            .warnings
            .iter()
            .any(|w| w.message.contains("unsupported")),
    );
    let err = v
        .update_project(
            "p",
            ProjectPatch {
                repos: Some(vec![Repo {
                    name: "a".into(),
                    path: "/a".into(),
                }]),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(matches!(err, Error::Unparseable { .. }), "{err:?}");
    assert_eq!(read(&v, PF), src);
    run_all(&[
        Case {
            name: "tab comment/add",
            src: "repozytoria: []\t# keep\n",
            set: &[("a", "/a")],
            want: "repozytoria:\t# keep\n  - nazwa: a\n    sciezka: /a\n",
            keep: &[],
        },
        Case {
            name: "tab comment/clear",
            src: "repozytoria:\t# keep\n  - nazwa: a\n    sciezka: /a\n",
            set: &[],
            want: "repozytoria:\t# keep\n",
            keep: &[],
        },
    ]);
}

/// Moving items keeps each line's own ending, even in a file with mixed endings.
#[test]
fn reorder_keeps_each_lines_own_ending() {
    let (_d, v) = vault();
    let src = "---\r\nrepozytoria:\r\n  - nazwa: a\r\n    sciezka: /a\r\n  - nazwa: b\n    sciezka: /b\n---\n";
    write(&v, PF, src);
    v.update_project(
        "p",
        ProjectPatch {
            repos: Some(vec![
                Repo {
                    name: "b".into(),
                    path: "/b".into(),
                },
                Repo {
                    name: "a".into(),
                    path: "/a".into(),
                },
            ]),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        read(&v, PF),
        "---\r\nrepozytoria:\r\n  - nazwa: b\n    sciezka: /b\n  - nazwa: a\r\n    sciezka: /a\r\n---\n"
    );
}
