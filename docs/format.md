# MontyBrain on-disk format

MontyBrain keeps everything in plain Markdown files inside one directory, the **vault**.
People and coding agents may edit these files by hand at any time. The
`montybrain-core` crate reads them tolerantly and writes them back **losslessly**:
parsing and re-serialising an untouched file gives back the same bytes, and a change
only rewrites the lines it is about (plus `zaktualizowano`, see below).

The words in the files are Polish. Identifiers in code and JSON are English (see
`core-api.md`).

---

## 1. Vault layout

The vault root is `$MONTYBRAIN_DIR` if that is set and not empty, else `$MORWISKO_DIR`
(the name before the rename), else `~/MontyBrain` if it exists, else `~/Morwisko` if it
exists, else `~/MontyBrain` (created). Nothing is ever moved or copied.

```
<vault>/
  skrzynka.md                 inbox: tasks that belong to no project
  <project-slug>/
    projekt.md                project metadata + project-level tasks
    tematy/
      <topic-slug>.md         one file per topic
  .montybrain/
    lock                      advisory lock file (see §9)
```

A vault created before the rename has `.morwisko/` instead of `.montybrain/` and keeps
using it: MontyBrain uses `.montybrain/` when it exists, else an existing `.morwisko/`,
and creates `.montybrain/` only in a vault that has neither. Both are skipped by the
loader and the watcher.

How the loader finds things:

* **Projects**: every top-level directory whose name does not start with `.` and that
  contains `projekt.md` or a `tematy/` directory. The directory name is the
  project's **slug**, which is its stable id. Other directories are ignored. A project
  directory that has `tematy/` but no `projekt.md` still loads, named after its slug,
  with a warning.
* **Topics**: every `tematy/*.md` file whose name does not start with `.`. The file
  stem is the topic's **slug**, its stable id inside the project. Subdirectories of
  `tematy/` are ignored.
* `skrzynka.md` may be missing; it is created when the first inbox task is added.
* Hidden entries (`.git`, `.obsidian`, `.montybrain`, …) are never read.
* Only an unreadable vault root makes loading fail. Every other discovery problem is a
  **warning** and the rest of the vault still loads: a project directory that cannot
  be read, a `tematy` that is a file instead of a directory or cannot be read, a broken
  symbolic link, a name that is not valid UTF-8, an unreadable file.

Renaming a project (`nazwa`) or a topic (`# H1`) does **not** rename its directory or
file. Slugs never change after creation. To change one, rename the file or directory
by hand.

## 2. General text rules

* Encoding: UTF-8. A UTF-8 BOM is kept if there is one. A file that is not valid
  UTF-8 is skipped with a warning, and mutations on it fail with
  `Error::Unparseable`.
* Line endings: LF and CRLF are both kept exactly. Lines that MontyBrain adds use the
  file's **first** line ending (LF if the file has none yet). MontyBrain never creates
  mixed line endings in a file that did not have them: lines moved in from another
  file (§7.3) take the destination's line ending, and only a destination without any
  line ending yet (empty, or one unterminated line) takes the moved block's.
* Whether the file ends with a newline is kept, even when lines are added at the
  end.
* Lines MontyBrain does not understand are never removed or reordered. When a file
  has a problem, the loader reports a **warning** (file, 1-based line, message) and
  keeps going. One bad line never stops the rest of the vault from loading.

### 2.1 What a written value may contain

Every value MontyBrain writes stays inside its own field: it can never create or change
structure elsewhere (a task, a heading or section, a journal entry, a frontmatter key
or delimiter, or a code fence that hides what follows). Input that would do so is
**rejected** with `InvalidInput` naming the field (`title`, `links[0]`, `summary`, …)
and the reason; it is not silently rewritten.

| field | stored as | rule |
|-------|-----------|------|
| task title, topic title, project name | one line | Whitespace runs, including line breaks, collapse to one space. Other control characters are rejected. Must not be empty. A task title must not end in something that reads back as a token (§7.2). |
| journal author | one line | `·` and control characters become spaces, whitespace collapses; empty becomes `ja`. |
| colour, repository `nazwa` / `sciezka` | one frontmatter value | Trimmed. Line breaks, other control characters and U+2028/U+2029 are rejected. A repository `nazwa` must not be empty. Values are quoted and escaped as needed (§3). |
| link | one indented line `- <url>` | Must be a single bare URL (`scheme://…` with no whitespace). Anything else is rejected. Empty strings are skipped. |
| task notes, journal text | indented lines below the item | Multi-line. `\r\n` counts as a line break; a lone `\r`, other control characters (tab is fine) and U+2028/U+2029 are rejected. Every line is written indented, so it can contain anything else (`## …`, `- [ ] …`, fences): an indented line is never structure, and a fence inside an item stays inside it (§6). |
| topic summary, project description | column-0 lines between the title and the first `##` | Same character rules as notes. Trimmed, and trailing whitespace is removed from each line. In addition, a line outside a code fence must not be a level-1 or level-2 heading (`#`, `# …`, `##`, `## …`; use `###` or deeper), the first line must not be `---`, and every code fence opened in the text must be closed in it. Checklists, `###` headings and balanced fences are fine. |

MontyBrain also checks every write before it happens: it parses the would-be file again
and compares its structure with the file on disk. Only the intended change may differ
(for example, creating a task: exactly one more task with the new id, the same other
tasks, sections, journal entries and non-MontyBrain frontmatter keys; a missing
`## Zadania`/`## Dziennik` may be added; MontyBrain's own keys may change). Otherwise the
mutation fails with `InvalidInput` ("refusing to write …; nothing was changed") and
nothing is written.

## 3. Frontmatter

An optional YAML block at the very top of the file, between a line `---` and the next
line `---` (or `...`). If the opening `---` is never closed, the block is ignored as
frontmatter (the text stays as it is) and a warning is reported.

MontyBrain supports this YAML subset:

* `key: value` lines starting in column 0. Values can be:
  * plain: `stan: aktywny`. A `#` after a space or a tab starts a comment
    (`stan: aktywny # x` and `stan: aktywny<TAB># x` both read as `aktywny`; `a#b` is
    one value). The reader and the writer share this rule, also for `repozytoria`.
  * double-quoted: `kolor: "#a3e635"`, with escapes `\"` `\\` `\n` `\t` `\r` `\/` `\0`
  * single-quoted: `nazwa: 'it''s'`, where `''` is a literal `'`
  * empty, `~` or `null`: no value.
* The `repozytoria` block list (see §4).
* Comments (`# …`), blank lines, unknown keys and nested structures under unknown
  keys are kept byte for byte. MontyBrain does not interpret them.
* **A repeated key.** If one of MontyBrain's keys occurs more than once (for example
  two `stan:` lines, or two `repozytoria:` blocks), the **first** occurrence is the
  one that is read and written. Every later occurrence gives a warning ("duplicate
  frontmatter key") on its line and is kept byte for byte, but never read or changed.
  An edit never lets a duplicate take over: clearing a duplicated scalar empties the
  first line (`kolor:`) instead of deleting it, and clearing a duplicated
  `repozytoria` leaves `repozytoria: []` (§4).

When MontyBrain writes a key:

* An existing key keeps its position, the text before its colon, the spacing after
  the colon and any trailing `# comment`. Only the value changes, and only the key's
  own line is rewritten: `stan: aktywny   # x` becomes `stan: archiwum   # x`, and
  `stan:   # x` becomes `stan: archiwum   # x`.
* Lines below one of MontyBrain's scalar keys (indented continuation lines, or
  comments followed by them) are not supported. They are kept byte for byte, never
  removed by an update, and reported as a warning on the key's line.
* A new key goes after the keys that come before it in the canonical order
  (`projekt.md`: `nazwa, kolor, stan, repozytoria`; topic:
  `status, repo, utworzono, zaktualizowano`). If none of those keys exist, it goes
  at the end of the block. If the file has no frontmatter, a new block is created
  at the top.
* Removing a value (for example clearing `kolor` or `repo`) deletes the key's line.
  If the line has a trailing comment or continuation lines, or the key occurs again
  further down, the value is emptied instead (`kolor:   # x`, which reads as no
  value), so nothing is lost and setting the value again gives back the same line.
* After the change, the file is read again and each written value must read back as
  given; otherwise nothing is written.
* Values are written plain when that is unambiguous. Otherwise they are
  double-quoted. A value is quoted when it is empty, starts with a YAML indicator
  (such as `#`, `-`, `[`, `"`), contains `: ` or ` #`, has leading or trailing
  spaces, or looks like a number, boolean or null.

Block scalars (`|`, `>`) are not supported for MontyBrain's own keys. If one is used,
the value is reported as a warning and ignored.

## 4. `projekt.md`

```markdown
---
nazwa: WhenUBuy
kolor: "#a3e635"
stan: aktywny            # aktywny | wstrzymany | archiwum
repozytoria:
  - nazwa: whenubuy-backend
    sciezka: /Users/x/Projects/WhenUBuy/whenubuy-backend
---
Free-text description (optional).

## Zadania
- [ ] Project-level task ^k3f9a2
```

| key           | meaning                                   | default if missing |
|---------------|-------------------------------------------|--------------------|
| `nazwa`       | display name                              | the slug           |
| `kolor`       | colour, e.g. `"#a3e635"` (always quote `#`!) | none            |
| `stan`        | `aktywny` \| `wstrzymany` \| `archiwum`   | `aktywny`; an unknown value gives a warning |
| `repozytoria` | list of `{nazwa, sciezka}`                | empty              |

* `repozytoria` must be a block list: each item starts with `-`, and its keys are
  `nazwa` and `sciezka` (in any order). Unknown item keys, nested values and comments
  inside an item are kept. `repozytoria: []` means an empty list. The `repozytoria:`
  line may end with a comment after a space or a tab (`repozytoria: []<TAB># none`);
  `[]#x` is not `[]` plus a comment. Flow lists (`[{…}]`) and other values are not
  supported and give a warning. An item without `nazwa` gives a warning and is
  ignored; an item without `sciezka` has an empty path.
* **Which lines belong to an item.** An item starts at its `-` line and owns every
  following line up to the first line that is neither blank nor a comment and is not
  indented deeper than the `-` (that line is the next item or list-level text).
  Comments and blank lines *between* the item's lines belong to the item whatever
  their indentation; comments *after* its last line belong to it only if they are
  indented deeper than the `-`. Inside an item, lines at the key column (the column
  of the first key) are its fields; deeper lines and nested `-` lists belong to the
  field above them. So an item is never indented deeper than the item before it
  (it would belong to that item); items may be indented less, or differently from
  each other.
* Warnings (the lines are kept and ignored): a line in the list that is not inside an
  item (for example `sciezka:` at the `-`'s indentation), a field line that is not
  `key: value`, a value that cannot be parsed, a repeated `nazwa`/`sciezka` in one
  item (the last one counts), and indented lines below `nazwa`/`sciezka`.
* **Changing the list.** The API takes the whole new list. Each repository in it is
  matched to at most one existing item (items without `nazwa` are never matched).
  The passes run in this order, each only over what is still unmatched:
  1. an item with the same `nazwa` and `sciezka`; then one with the same `nazwa`.
     With duplicate names, list order decides (the first such repository takes the
     first such item);
  2. an item with the same non-empty `sciezka`, when exactly one unmatched item and
     exactly one unmatched repository have that path (a rename that kept its path,
     wherever it moved in the list);
  3. an item from the same place in the list, that is, between the items matched to
     the repository's neighbours in the new list (one with the same non-empty
     `sciezka` first, then in order);
  4. any item still unmatched: the remaining repositories (in list order) take the
     remaining items (in file order).

  Passes 2–4 read a changed repository as a **rename** or edit of an existing item.
  A whole-list update cannot tell "rename `a` to `b`" from "remove `a`, add `b`", so
  MontyBrain always picks the reading that keeps the item's other lines. The result:
  **an item is removed only when the new list is shorter than the list of named
  items**, and then exactly as many items as it is shorter (those that no
  repository matched). A repository is **new** only when every named item is
  already matched. Replacing every repository edits the existing items in place.
* What the change writes:
  * A matched item keeps all its lines. Only the value of its `nazwa` and `sciezka`
    lines changes, as for scalar keys (§3: position, spacing and trailing comment
    kept; quoting as needed). A missing `sciezka` is added right below `nazwa`, at
    the item's key column (not when the path is empty).
  * The matched items fill their own places in the order of the new list, so a
    reorder moves whole items and leaves list-level text where it is. Each place
    keeps its indentation: an item that moves to a place whose `-` is indented
    differently is re-indented as a block (every line by the same amount, blank
    lines untouched), so its inner layout is unchanged and it cannot end up nested
    in its neighbour. Otherwise its lines are kept byte for byte.
  * A removed item loses its lines, including comments inside it, plus one blank line
    that separated it from its neighbours.
  * A new item goes right after the item before it in the new list (a new first
    item goes right before the first kept item; with no kept item, after the last
    item or at the end of the list). It is written as `- nazwa: …` / `sciezka: …`
    shaped like the item whose place it follows (the first kept place for a new
    first item, the last item when none is kept; two spaces if there is no item).
  * The `repozytoria:` line, including a trailing `# comment`, is kept. `[]` is
    removed from it when the first item is added.
  * If `repozytoria` occurs more than once, only the first block is changed (§3).
    Emptying it then keeps the key as `repozytoria: []` (or keeps it as it is when
    it has a comment or other text), so the duplicate never takes over.
  * After the change the file is read again and must give exactly the requested
    list; otherwise the mutation fails and nothing is written.
  * List-level text stays where it is: comments and blank lines before, between or
    after items, items without `nazwa`, and lines outside any item.
  * Lines keep their own line endings, also when a reorder moves them; new lines use
    the file's line ending (§2).
  * An empty list removes the key, unless the key line has a comment or the list has
    other text, which are then kept (the key then reads as an empty list).
  * A value that is not a block list (a flow list `[{…}]`, a scalar) cannot be
    changed without losing it: the mutation fails with `Error::Unparseable` and the
    file is not touched.
* **Description**: the body text between the frontmatter and the first `##` heading,
  trimmed. If there is a `# H1` line before the first `##`, the description is the
  text after the H1, and the H1 is kept.
* **Tasks**: `## Zadania` (see §7).

## 5. Topic file `tematy/<slug>.md`

```markdown
---
status: w-toku           # planowany | w-toku | zablokowany | czeka | zrobiony
repo: whenubuy-backend   # optional, name of one of the project's repos
utworzono: 2026-11-13
zaktualizowano: 2026-11-13T14:02
---
# Topic title

Free text between the H1 and the first `##` = the topic's "current state" summary.

## Zadania
- [ ] Todo task !wysoki @2026-11-20 ^g7h8i9

## Dziennik
- 2026-11-13 14:02 · agent · Tests green, MR opened.
```

| key              | meaning                                                              |
|------------------|----------------------------------------------------------------------|
| `status`         | `planowany` \| `w-toku` \| `zablokowany` \| `czeka` \| `zrobiony` (default `planowany`; an unknown value gives a warning) |
| `repo`           | optional; must be the `nazwa` of one of the project's repos (warning otherwise) |
| `utworzono`      | creation date `YYYY-MM-DD`                                           |
| `zaktualizowano` | last change, `YYYY-MM-DDTHH:MM` (local time). A date alone or `YYYY-MM-DD HH:MM[:SS]` is accepted when reading |

* **Title**: the first `# ` heading before any `##` section. If there is none, the
  title is the slug. Setting a title replaces that line, or inserts `# Title` right
  after the frontmatter.
* **Summary** ("current state"): everything between the H1 and the first `##`
  heading, trimmed. When it is rewritten, the region becomes: blank line, summary,
  blank line (when a section follows). Lines that did not change are not touched.
  What a summary may contain is in §2.1.
* **`zaktualizowano`** is set to "now" by every mutation that changes a topic file
  (task changes, journal entries, metadata). It is inserted if missing. In
  `projekt.md` and `skrzynka.md` it is updated only if the key is already there.
  `normalize` (§8) never changes it.
* New topic files are created as:

  ```markdown
  ---
  status: planowany
  repo: …                       (only if given)
  utworzono: 2026-11-14
  zaktualizowano: 2026-11-14T09:15
  ---
  # Title

  Summary (only if given)

  ## Zadania

  ## Dziennik
  ```

## 6. Sections

* A section starts at a line `## Name` (exactly two `#`, then a space) and runs to the
  next `## ` line or the end of the file. `###` and deeper headings do not start a
  section; they belong to the section they are in.
* **Fenced code blocks.** Lines inside a fenced code block (delimiters included) are
  never headings, tasks or journal entries. One parser decides this for sections,
  tasks, journal entries and input validation:
  * A fence **opens** on a line indented by at most 3 spaces that starts with at least
    three backticks or at least three tildes. The info string after backticks must
    not contain a backtick.
  * It **closes** on a line indented by at most 3 spaces that consists of the same
    character, at least as many times as the opening run, followed only by
    whitespace. So a fence opened with four backticks is not closed by three, and a
    tilde fence is not closed by backticks. An unclosed fence runs to the end of the
    file.
  * A fence marker on an **indented line that continues a list item** (a task's
    notes, a journal entry's continuation, or any `-`/`*`/`+`/`1.` item) belongs to
    that item and never hides anything: every structural line starts in column 0, and
    a column-0 line ends the item anyway. After a non-list line (a paragraph, a
    heading), a marker indented by 1–3 spaces opens a real fence, as in CommonMark.
  * A line indented by 4 or more spaces (or a tab) is never a fence marker.
* Structural lines (headings, task lines, journal entries) always start in column 0.
* Section names are compared after trimming, ignoring ASCII case. MontyBrain knows
  `Zadania` (tasks) and `Dziennik` (journal). All other sections are kept and
  ignored. Checkbox lines in other sections are **not** tasks.
* If there are several `## Zadania` sections, they are all read. New tasks go after
  the last task of the file.

## 7. Tasks

### 7.1 Task line

```
- [ ] Title words … !wysoki @2026-11-20T14:00 ✓2026-11-12 ^g7h8i9
```

Grammar of one task line:

1. Column 0: a bullet `-`, `*` or `+`, then exactly one space. Any indentation (a
   space or a tab) means the line is not a task: an indented checkbox line is a note
   of the task above it, or plain text. A checkbox line inside a fenced code block
   (§6) is not a task either.
2. A checkbox `[c]` where `c` is:

   | `c`        | status   |
   |------------|----------|
   | space      | todo     |
   | `/`        | doing    |
   | `x` or `X` | done (written as `x`) |
   | `!`        | blocked  |

3. Then the end of the line, or whitespace followed by the title and tokens.

A line that starts like a checkbox (`- [`) but breaks this grammar is **kept as is**,
is not loaded as a task, and gives a warning. Examples: `- [?] x`, `- [] x`,
`- [ ]x`. Plain bullets (`- text`) are silently ignored.

### 7.2 Inline tokens

Tokens are read **from the end of the line, right to left**. Reading stops at the
first word that is not a token, or at a second token of a kind that was already
read. Everything before that point is the title, with trailing whitespace trimmed.
So `Fix @mention parsing` stays a title, and only trailing tokens count.

| token                | meaning |
|----------------------|---------|
| `!wysoki` / `!niski` | priority high / low. No token means normal. `!normalny` is also accepted as normal and is never written. |
| `@YYYY-MM-DD`        | due date |
| `@YYYY-MM-DDTHH:MM`  | due date with a time (used for reminders) |
| `✓YYYY-MM-DD`        | completion date (U+2713). Added automatically when a task becomes done and removed when it is reopened. Loading never adds or removes it. |
| `^xxxxxx`            | stable task id: exactly 6 characters from `[0-9a-z]` |

* When tokens are read, their order does not matter. When MontyBrain **writes** a line,
  the canonical order is `title !prio @due ✓done ^id`, with single spaces.
* Words that look like tokens but are invalid, such as `@2026-02-30`, `^ABC123`,
  `^abc12` or `✓wczoraj`, stop the token run and stay in the title. A warning is
  reported. Only words that really look like a token are reported (`@` followed by
  `DDDD-`, `^` followed by 4–8 alphanumerics, any `✓…`), so `@2x` or `^ab` in a title
  are not reported.
* **Ambiguous titles**: MontyBrain refuses (`InvalidInput`) to write a title that would
  read back as something else. For example, the title `Meet @2026-11-20` on a task
  without a due date would be read back as title `Meet` with a due date. The same
  title is fine on a task that already has a due date, because the second `@…` stops
  the token run.
* Changing only the status changes only the checkbox character, so unusual spacing
  in the line is kept. Any other change to the line rewrites it in canonical form,
  keeping the original bullet character.

### 7.3 Task block: links and notes

The indented lines right below a task line belong to that task. An indented line
starts with a space or a tab. A blank line belongs to the block only if more
indented lines follow it, so a block never ends with a blank line. The first
non-indented, non-blank line ends the block.

Each line in the block is either:

* a **link**: after trimming and removing an optional `- `, `* ` or `+ ` bullet, the
  content is a single bare URL (`scheme://…` with no whitespace); or
* a **note**: any other line.

`notes` in the model is all note lines joined with `\n`. Each line has the block's
base indentation removed: the leading whitespace of the first non-blank line in the
block. Deeper indentation and bullets stay part of the note text. Leading and
trailing empty note lines are trimmed.

Writing:

* Added links become `<base>- <url>` right after the last existing link, or right
  below the task line if there are no links. Duplicate URLs are skipped. A link must
  be a single bare URL (§2.1).
* Removing a link deletes its line.
* Setting notes deletes all note lines, including blank ones, and writes the new
  text where the first old note was (or at the end of the block). Each line becomes
  `<base><line>`, and empty lines stay empty. `<base>` is the block's base
  indentation, or two spaces. A note line that consists only of a URL reads back as
  a link.
* Moving a task moves its whole block: the text of every line is kept byte for byte,
  and the lines take the destination file's line ending (§2), so the destination
  never ends up with mixed line endings. Deleting a task deletes the whole block.
* Notes may span several lines and contain any text (§2.1); every note line is
  written indented, so a note can never create a task, heading or journal entry.

### 7.4 Where tasks live and where new ones go

| file          | tasks are read from |
|---------------|---------------------|
| `skrzynka.md` | the body before the first `##` heading, plus every `## Zadania` section. Tasks in other sections are not inbox tasks. |
| `projekt.md`  | `## Zadania` sections only |
| topic file    | `## Zadania` sections only |

A new or moved task is added after the last task block of the file. If the file has
no task yet:

* if a `## Zadania` section exists, the task goes right below its heading;
* in the inbox, it goes at the end of the text before the first `##`, with a blank
  line kept before that heading;
* otherwise a `## Zadania` section is created: before `## Dziennik` in a topic,
  at the end of the file in `projekt.md`.

## 8. Task ids and `normalize`

* Every task should have an id `^xxxxxx`: 6 lowercase base36 characters, unique
  across the whole vault. Ids are random and are never reused on purpose. Moving a
  task keeps its id.
* A hand-written task without an id still loads (with `id: null` and a warning), but
  it cannot be changed by id until it has one.
* `Vault::normalize()` takes the lock, gives a fresh id to every task without one
  (by appending ` ^xxxxxx` to its line) and to every task whose id is a duplicate of
  an earlier one (in vault order: inbox, then projects by slug, then `projekt.md`
  before topics, then topics by slug). It writes only the files it changed. The app
  and the MCP server call it on open, and may call it after a file-change event.

## 9. Journal (`## Dziennik`)

```
- 2026-11-13 14:02 · agent · Tests green, MR opened.
  optional continuation line(s), indented
- 2026-11-12 09:30 · ja · Started A/B tests.
```

* Grammar: a bullet `- ` or `* ` in column 0, then `YYYY-MM-DD HH:MM` (a `T` between
  date and time is also accepted), then `·` (U+00B7), the author, `·`, and the text.
  Spaces around `·` are optional. Only the first two `·` split the line, so the text
  may contain `·`. With a single `·`, the author is empty and everything after it
  is the text.
* Author: free text, for example `ja`, `agent` or an agent or thread name. When
  MontyBrain writes an author, it removes `·` and newlines, collapses whitespace, and
  uses `ja` if the result is empty.
* Indented lines after an entry continue its text (joined with `\n`, each line
  trimmed). When MontyBrain writes a multi-line text, every line after the first is
  written indented by two spaces, so the text can never start a new entry, section or
  task (§2.1).
* Entries are newest first. A new entry goes before the first existing entry. If
  there are no entries, it goes right below the heading. A missing `## Dziennik`
  section is added at the end of the file.
* Other bullet lines in the section give a warning and are kept. Non-bullet text is
  kept silently.

## 10. Slugs

Projects and topics get a slug from their name or title when they are created:

1. Polish letters are transliterated (`ą→a ć→c ę→e ł→l ń→n ó→o ś→s ź→z ż→z`, upper
   case too), and so are common Latin diacritics (`é→e`, `ü→u`, `ß→ss`, …).
2. The result is lowercased. Each run of other characters becomes a single `-`.
   Leading and trailing `-` are removed.
3. The slug is cut to at most 60 characters, at a `-` when possible.
4. An empty result becomes `bez-nazwy`.
5. If the slug is already taken (an existing directory, or `<slug>.md` in `tematy/`),
   `-2`, `-3`, … is appended.

Example: `Płatności: Stripe → Przelewy24` → `platnosci-stripe-przelewy24`.

## 11. Concurrency protocol (for every writer)

The desktop app and the MCP server are separate processes that write the same
files. Every MontyBrain mutation:

1. takes an **exclusive advisory lock** on `<vault>/.montybrain/lock` (or
   `<vault>/.morwisko/lock` in a vault from before the rename, §1; when a vault has both
   directories, both locks, the legacy one first) (`flock`-style, through Rust's
   `File::lock`), waiting if another writer holds it;
2. **re-reads** the affected files from disk;
3. applies the change in memory and **verifies** it: every affected file is parsed
   again and its structure must have changed exactly as intended (§2.1). All files
   of the mutation are verified before the first one is written;
4. writes each file **atomically**: it writes a temp file
   `.<name>.<pid>.<n>.montybrain-tmp` in the same directory, fsyncs it, copies the old
   file's permissions to it, and renames it over the original;
5. releases the lock.

A move writes the destination before the source. A crash between the two writes
can therefore duplicate a task, which `normalize` then reports as a duplicate id,
but it can never lose one.

Readers do not take the lock. The atomic rename means a reader always sees either
the old or the new file, never a half-written one.

People editing by hand do not take the lock. Editors usually save in a fraction of
a second, so the remaining risk is small: MontyBrain re-reads inside the lock, and a
hand save that lands between MontyBrain's read and its rename can be overwritten. A
script or agent that edits files directly should take the same lock (for example
`flock <vault>/.montybrain/lock <command>`) or go through the MCP server.

## 11a. References between topics and tasks (a convention on free text)

The app's map ("Mapa") reads connections from free text. Core treats them as
ordinary text: nothing below changes the structure of a file, and a summary or
note with references round-trips byte for byte.

* `[[slug]]`: a topic of the same project; `[[projekt/slug]]`: a topic of any
  project (the same slug may exist in two projects); `^id`: a task.
* Read only from a topic's summary (the text under the title) and from task
  notes. Fenced code blocks and inline code are skipped. The journal is never
  read: an entry is history.
* A line that starts with `Czeka na:` makes its references directed: "this
  topic / task waits for that one". Any other reference is "related"
  (undirected).
* The app writes a connection as one standalone line at the end of the summary,
  `Powiązane: [[projekt/slug]]` or `Czeka na: [[projekt/slug]]`, and removes
  exactly that line again; a reference inside prose is only changed by editing
  the text.
* A reference to something that does not exist is shown as a warning, never as
  a node. Use slugs: a title change keeps the slug, `^id` survives moving a
  task; only renaming a file breaks a reference. A task without an id cannot be
  referenced.

## 12. Notes for agents editing files by hand

* Keep a task on one line: `- [ ] Title !prio @due ^id`. Leave out `^id` if you don't
  have one; `normalize` adds it. Never make up an id that may already exist.
* Put links and notes on indented lines (two spaces) directly below the task.
* Add a journal entry as the **first** bullet under `## Dziennik`:
  `- YYYY-MM-DD HH:MM · <your name> · <what happened>`.
* Update `zaktualizowano: YYYY-MM-DDTHH:MM` when you change a topic.
* Quote values that start with `#` in frontmatter (`kolor: "#a3e635"`).
* Do not rename `tematy/*.md` files or project directories unless you mean to change
  the slug. Slugs are ids that agents and the app use.
* Avoid ending a title with something that looks like a token (`@2026-…`, `!wysoki`,
  `^abc123`, `✓…`).

## 13. The command language (input only)

Skrzynka, the quick-capture window, the "Nowe" row of ⌘K and the MCP `command`
tool read one line of text with one parser (`montybrain_core::command`). Nothing
of the syntax below reaches the files except what it means: tasks, topics and
lines written in the formats above.

```text
Zadzwonić do sądu @jutro !wysoki            task in Skrzynka
Podpiąć karty > whenubuy/checkout           task in a topic
Sprawdzić fakturę > whenubuy                project-level task
[/] Dokończyć opis > mapa-mysli             doing; a bare topic slug when it is unique
/temat Płatności mobilne > whenubuy         new topic
/temat ^a1b2c3 > firma                      topic from an entry (the entry becomes its task)
^a1b2c3 > whenubuy/checkout                 move
^a1b2c3 [x]                                 status
^a1b2c3 !wysoki @2026-11-20                 priority and due date
^a1b2c3 /czeka ^c3d4e5                      adds the line "Czeka na: ^c3d4e5" to the notes
[[whenubuy/checkout]] /czeka [[firma/krs]]  adds "Czeka na: [[firma/krs]]" to the summary
^a1b2c3 /powiaz [[whenubuy/platnosci]]      adds "Powiązane: [[whenubuy/platnosci]]"
^a1b2c3 /czeka odpis z sądu                 the same, the task or topic found by its name
umowa z dostawcą /powiaz płatności          subject and reference by name (no ` > `)
[ ] Umowa z dostawcą /powiaz ^a1b2c3        `[ ]` first: always a new task
?rozpisz ^a1b2c3                            a request to the app's agent
?dopasuj                                    the agent proposes places (selection or all of Skrzynka)
```

* `!priorytet`, `@termin`, `^id`, `[[projekt/temat]]` and the boxes `[ ] [/] [x] [!]`
  mean what they mean in files (§7). In a new task's title only trailing `!…` and
  `@…` words count, read right to left like in a task line; `@` before something that
  is not a date stays in the title (`Fix @mention`).
* Relative dates are written as plain dates: `@dzis`/`@dziś`, `@jutro`, `@pojutrze`,
  `@pn` … `@nd` (the next such day after today; on a Monday `@pn` is in a week),
  `@+3d` (days), `@+2t` (weeks, up to 3650 days). Absolute: `@YYYY-MM-DD[THH:MM]`.
* ` > ` must be a word of its own (`a>b` is text); `\>` is a literal `>`, a leading
  `\?` a literal `?`. Unknown `/words` are text.
* After ` > `: `skrzynka`, `projekt`, `projekt/temat` or a topic. An exact slug wins;
  then slugs and names are compared ignoring case and Polish characters; a word that
  is a project and a topic means the project. Two hits or none: nothing is written
  and the choices are listed. A name one edit away from exactly one place is offered
  as a typo ("literówka") and still has to be confirmed (the MCP tool never applies
  it). Archived projects and done topics are found only by their exact slug.
* Text that starts with `^id` (or several) edits those tasks; text that starts with
  ` > `, `/czeka`, `/powiaz` or only attributes applies to the entries selected in
  Skrzynka, or, with none selected, to the entry focused in its list.
  `/temat Nazwa > projekt` with a selection makes one topic of the selected
  entries; ids, links and notes are kept (whole blocks move).
* After `/czeka` and `/powiaz` a task or topic can be named by plain words instead
  of `^id` / `[[…]]`; likewise plain words at the start of a line that has
  `/czeka` or `/powiaz` and no ` > ` name the task or topic to change. The words are
  searched in open tasks (title) and open topics of active projects (title, slug),
  ignoring case and Polish characters; the project's (and topic's) name may narrow
  them (`umowa firma`), and a word of four letters or more may be one edit off. The
  whole name first, then names whose words start with the words typed, then one
  typo away: exactly one hit of the best kind is used (a typo still has to be
  confirmed and is never applied by the MCP tool); several: nothing is written and
  they are listed (kind, place, `^id` or `[[projekt/temat]]`). A subject that finds
  nothing makes a new task with that title (said in the preview); a reference that
  finds nothing is a problem. A leading box (`[ ] Tytuł /powiaz …`) keeps it a new
  task. Done tasks and topics are referred to by `^id` / `[[…]]`.
* `/czeka` and `/powiaz` append exactly one standalone line (§11a) at the end of the
  summary (topics) or of the notes (tasks); nothing around it is rewritten. An
  identical line already there is not added twice.
* There is no delete and no project creation this way.
* Everything except plain capture (a title with optional `!`/`@`, into Skrzynka or
  the place picked in the quick-capture window) is shown as a plan first (the files
  and the exact lines) and written on confirmation; the plan is rebuilt before
  writing and refused when the data changed meanwhile. The MCP `command` tool does
  the same in two calls: the preview returns a `plan` token, and `apply: true` writes
  only a plan with that token.
* A plan is written all or nothing: when a step fails, the steps already written are
  taken back (each only while unchanged); only if that cannot complete does the app
  or the MCP tool report exactly what stays written.
* A new task's id is chosen with the plan (shown in the preview, written exactly,
  refused if it exists: a plan cannot be applied twice); a new topic is written at
  exactly the previewed path. An edit or an appended line is written only while the
  task (title, links, notes, fields) or the topic summary is still as previewed.
* Topic titles cannot contain `[` or `]` (topics are referenced as
  `[[projekt/slug]]`); existing titles with brackets are referenced by their slug.
* A text with any problem is not a plan: its reading is the problem. Topic titles
  that would read as structure (`## …`, `---`, `- …`, a fence) are refused, and a
  connection line that would not read back as one (notes or a summary ending in an
  unclosed code block) is refused before anything is written.
* `\>` is a literal `>` also in a destination; a name with a slash in it
  ("Testy A/B") is matched as a whole name before being read as `projekt/temat`.
* While a line is being typed (the app passes the caret), a word still being
  typed at the caret (`/`, `/cz`, `^`, `[[kr`, `!`, `@ju`, a lone ` > ` or
  `/czeka`, or a name with no single match yet) reads as "typing": a neutral hint
  instead of an error, and nothing to write. Without a caret (the MCP tool, a
  write) every problem counts.
