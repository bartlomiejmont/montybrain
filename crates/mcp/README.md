# morwisko-mcp

Stdio [MCP](https://modelcontextprotocol.io) server that lets coding agents (Claude Code, Codex)
read and update the user's Morwisko vault: projects, topics, tasks and journal, stored as
Markdown files. It builds on `morwisko-core` and uses the official Rust SDK `rmcp` 3.10.

On every tool call the server opens the vault (`$MORWISKO_DIR`, default `~/Morwisko`), runs
`normalize()` so hand-written tasks get ids (skipped in read-only mode, see below), and loads a fresh snapshot, so it always sees edits
made by the user or by the desktop app. stdout is the protocol channel; logs go to stderr
(`RUST_LOG=debug` for more). There are deliberately no delete tools in this version.

## Read-only mode

When the environment variable `MORWISKO_READONLY` is `1` or `true` (case-insensitive), the server
writes nothing: `create_task`, `update_task`, `create_topic` and `log_update` return a tool error
starting with `read-only run:` before touching the vault, and the read tools skip `normalize()`
(hand-written tasks without an id are shown as they are instead of getting one). The server
`instructions` say so, so the agent knows not to try. The Morwisko desktop app uses this for its
report-only background runs, e.g. for Codex:
`codex exec -c 'mcp_servers.morwisko.env.MORWISKO_READONLY="1"' …`.

## Tools

| tool | purpose |
|------|---------|
| `overview` `{project?}` | Session-start briefing: date, projects and topics, a Today block (overdue / due today / in progress / blocked), inbox count, recent journal. Every section has an item and a character budget; cut sections end with `… and N more (use list_tasks with …)`. |
| `get_topic` `{project, topic, include_done?, tasks_limit?, tasks_offset?, journal_limit?, journal_offset?}` | Full topic: status, repo, summary, tasks, journal. 50 tasks and 30 journal entries per page by default (max 500); the output says how to get the next page. |
| `list_tasks` `{project?, topic?, inbox_only?, status?[], due_from?, due_to?, include_done?, limit?, offset?}` | Filtered task list, 50 per page by default (max 500); `… and N more (pass offset=…)` when cut. |
| `search` `{query, limit?, offset?}` | Search tasks, topics and journal (case and diacritics insensitive). 20 results per kind by default (max 500); `limit`/`offset` apply to each kind. |
| `create_task` `{title, project?, topic?, status?, priority?, due?, links?[], notes?}` | No project: inbox; project only: project-level task. |
| `update_task` `{id, title?, status?, priority?, due?, clear_due?, add_links?[], remove_links?[], notes?, move_to?{inbox?, project?, topic?}}` | Edit, change status, complete or move a task. Edits and the move are one core transaction (`update_and_move_task`): both happen or neither does. |
| `create_topic` `{project, title, status?, repo?, summary?}` | New topic in an existing project. |
| `log_update` `{project, topic, text, status?, summary?, author?}` | Append a journal entry, optionally changing topic status/summary in the same write. Author defaults to `agent`. |

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
cargo build --release -p morwisko-mcp     # binary: target/release/morwisko-mcp
cargo test -p morwisko-mcp                # spawns the binary against a temp vault
```

## Registration

Use the absolute path of the built binary. The server needs no arguments; set `MORWISKO_DIR`
only if the vault is not `~/Morwisko`.

Claude Code (syntax checked against `claude mcp add --help`; the command itself was not run):

```sh
claude mcp add --scope user morwisko -- /ABS/PATH/TO/morwisko-mcp
# with a custom vault directory:
claude mcp add --scope user -e MORWISKO_DIR=/path/to/vault morwisko -- /ABS/PATH/TO/morwisko-mcp
```

Codex (`codex mcp add` syntax checked against `codex mcp add --help`; the command itself was not run):

```sh
codex mcp add morwisko -- /ABS/PATH/TO/morwisko-mcp
codex mcp add --env MORWISKO_DIR=/path/to/vault morwisko -- /ABS/PATH/TO/morwisko-mcp
```

Equivalent `~/.codex/config.toml` entry (table layout written from memory of the Codex docs,
not verified against this Codex version):

```toml
[mcp_servers.morwisko]
command = "/ABS/PATH/TO/morwisko-mcp"
# env = { MORWISKO_DIR = "/path/to/vault" }
```
