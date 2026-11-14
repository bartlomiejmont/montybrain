# montybrain-mcp

Stdio [MCP](https://modelcontextprotocol.io) server that lets coding agents (Claude Code, Codex)
read and update the user's MontyBrain vault: projects, topics, tasks and journal, stored as
Markdown files. It builds on `montybrain-core` and uses the official Rust SDK `rmcp` 3.10.

On every tool call the server opens the vault (`$MONTYBRAIN_DIR`, default `~/MontyBrain`, or `~/Morwisko` when only that exists; the
pre-rename `MORWISKO_DIR` / `MORWISKO_READONLY` still work), runs
`normalize()` so hand-written tasks get ids (skipped in read-only mode, see below), and loads a fresh snapshot, so it always sees edits
made by the user or by the desktop app. stdout is the protocol channel; logs go to stderr
(`RUST_LOG=debug` for more). There are deliberately no delete tools in this version.

## Read-only mode

When the environment variable `MONTYBRAIN_READONLY` is `1` or `true` (case-insensitive), the server
writes nothing: `create_task`, `update_task`, `create_topic` and `log_update` return a tool error
starting with `read-only run:` before touching the vault, and the read tools skip `normalize()`
(hand-written tasks without an id are shown as they are instead of getting one). The server
`instructions` say so, so the agent knows not to try. The MontyBrain desktop app uses this for its
report-only background runs, e.g. for Codex:
`codex exec -c 'mcp_servers.montybrain.env.MONTYBRAIN_READONLY="1"' …`.

## Tools

| tool | purpose |
|------|---------|
| `overview` `{project?}` | Session-start briefing: date, projects and topics, a Today block (overdue / due today / in progress / blocked), inbox count, recent journal. Every section has an item and a character budget; cut sections end with `… and N more (use list_tasks with …)`. |
| `get_topic` `{project, topic, include_done?, tasks_limit?, tasks_offset?, journal_limit?, journal_offset?}` | Full topic: status, repo, summary, tasks, journal. 50 tasks and 30 journal entries per page by default (max 500); the output says how to get the next page. |
| `list_tasks` `{project?, topic?, inbox_only?, status?[], due_from?, due_to?, include_done?, limit?, offset?}` | Filtered task list, 50 per page by default (max 500); `… and N more (pass offset=…)` when cut. |
| `search` `{query, limit?, offset?}` | Search tasks, topics and journal (case and diacritics insensitive). 20 results per kind by default (max 500); `limit`/`offset` apply to each kind. |
| `create_task` `{title, project?, topic?, status?, priority?, due?, links?[], notes?, waits_for?[], related?[]}` | No project: inbox; project only: project-level task. `waits_for` / `related` (`^id` or `[[project/topic]]`) add one `Czeka na: …` / `Powiązane: …` line each to the notes. |
| `update_task` `{id, title?, status?, priority?, due?, clear_due?, add_links?[], remove_links?[], notes?, move_to?{inbox?, project?, topic?}, waits_for?[], related?[]}` | Edit, change status, complete or move a task. Edits and the move are one core transaction (`update_and_move_task`): both happen or neither does. `waits_for` / `related` then append one line each (already present lines are skipped; the other note lines are not touched). |
| `create_topic` `{project, title, status?, repo?, summary?, waits_for?[], related?[]}` | New topic in an existing project; `waits_for` / `related` end the summary with one line each. |
| `command` `{text, apply?, plan?}` | One line of the command language (below), in two calls. Without `apply`: a preview — the plan with each file and the exact Markdown lines (a new task already with the id it will get), warnings, similar open tasks, and a `plan` token in the structured result (`structuredContent.plan`; the prose repeats it, but use the field). Nothing is written, not even ids. With `apply: true` and that token: the plan is rebuilt and written only when its token is the same — the token covers the new task's id, the new topic's exact path, and everything an edit or an appended line depends on (the task's title, links, notes; the topic's summary), all re-checked inside the lock while writing. Otherwise nothing is written and the new preview is returned; `apply: true` without a token writes nothing and returns the preview. Applying the same token twice is refused (the id exists). A plan is written all or nothing: if a step fails, the steps already written are taken back. Every error starts with `Nothing was changed:` — except one that starts with `The write failed and could not be fully taken back`, which lists exactly what is still written. A typo correction is never applied (send the exact slug); `?` requests are refused (the server never calls a model). In read-only runs only the preview works. |
| `connections` `{ref}` | Connections of one task (`^id`; six lowercase characters without `^` are a task id when such a task exists, else a topic slug) or topic (`[[project/topic]]`, `project/topic`, a unique topic slug), read like the app's map: what it waits for, what waits for it, related items, the "waits for" chain (cycles marked) and dangling references. |
| `log_update` `{project, topic, text, status?, summary?, author?}` | Append a journal entry, optionally changing topic status/summary in the same write. Author defaults to `agent`. |

The command language (full rules in [`docs/format.md` §13](../../docs/format.md)):

```text
Zadzwonić do sądu @jutro !wysoki            task in the inbox
Podpiąć karty > whenubuy/checkout           task in a topic
[/] Dokończyć opis > mapa-mysli             doing, topic found by its unique slug
/temat Płatności mobilne > whenubuy         new topic
/temat ^a1b2c3 > firma                      topic from an existing entry
^a1b2c3 [x] !wysoki @2026-11-20 > whenubuy  status, priority, due date, move
^a1b2c3 /czeka ^c3d4e5                      "Czeka na: ^c3d4e5" in the notes
[[whenubuy/checkout]] /powiaz [[firma/krs]] "Powiązane: [[firma/krs]]" in the summary
^a1b2c3 /czeka odpis z sądu                 the same, the task or topic found by its name
umowa z dostawcą /powiaz płatności          subject and reference by name (no ` > `)
```

After `/czeka` / `/powiaz`, and at the start of a line that has one of them and no ` > `,
plain words name an open task or topic (title, slug, project name; case, Polish characters
and one typo ignored). One hit is used; several are an error listing their `^id` /
`[[project/topic]]`; a subject that finds nothing becomes a new task (`[ ] Title …` always
does).

Conventions: `project` / `topic` accept the slug or the display name. An exact slug match
always wins; otherwise slugs, names and the slugified input are compared case and diacritics
insensitively. On no match or ambiguity the tool returns an error that lists the valid choices
(pass the exact slug to disambiguate).

Long text is cut with explicit markers: task titles at 200 characters, notes at a few lines per
task in lists (`> … N more note line(s)`), journal entries at 140–1500 characters depending on
the tool. The full text is always in the file named in the output.

Errors of the writing tools start with `Nothing was changed:` when the vault was not touched
(validation errors, unknown targets, unreadable files, refused structure changes). Values that
would create structure in the Markdown files are refused: links must be bare URLs, a topic
`summary` must not contain `#`/`##` headings or an unclosed code fence (see
`docs/format.md` §2.1). Only an I/O failure in the middle of a write says that the change may be
partially applied.
Task status: `todo, doing, done, blocked`. Topic status: `planned, in_progress, blocked, waiting,
done`. Priority: `high, normal, low`. The Polish file words (`w-toku`, `wysoki`, `czeka`, ...) are
accepted too. Dates: `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`.

## Build

```sh
cargo build --release -p montybrain-mcp     # binary: target/release/montybrain-mcp
cargo test -p montybrain-mcp                # spawns the binary against a temp vault
```

## Registration

Use the absolute path of the built binary. The server needs no arguments; set `MONTYBRAIN_DIR`
only if the vault is not `~/MontyBrain`.

Claude Code (syntax checked against `claude mcp add --help`; the command itself was not run):

```sh
claude mcp add --scope user montybrain -- /ABS/PATH/TO/montybrain-mcp
# with a custom vault directory:
claude mcp add --scope user -e MONTYBRAIN_DIR=/path/to/vault montybrain -- /ABS/PATH/TO/montybrain-mcp
```

Codex (`codex mcp add` syntax checked against `codex mcp add --help`; the command itself was not run):

```sh
codex mcp add montybrain -- /ABS/PATH/TO/montybrain-mcp
codex mcp add --env MONTYBRAIN_DIR=/path/to/vault montybrain -- /ABS/PATH/TO/montybrain-mcp
```

Equivalent `~/.codex/config.toml` entry (table layout written from memory of the Codex docs,
not verified against this Codex version):

```toml
[mcp_servers.montybrain]
command = "/ABS/PATH/TO/montybrain-mcp"
# env = { MONTYBRAIN_DIR = "/path/to/vault" }
```
