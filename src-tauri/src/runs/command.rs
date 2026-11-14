//! argv and environment of a background run. Every element comes from the
//! recipe table, the resolved provider, the app's own directories and the
//! vault root, or validated parameters; the prompt is always one single argv
//! element. No shell is involved.
//!
//! Isolation of a background run:
//! * Claude Code gets no built-in tools at all (no Bash, no file tools): only
//!   the MontyBrain MCP tools are allowed, and `--strict-mcp-config` with an
//!   app-written config attaches `montybrain-mcp` as the only MCP server.
//! * Codex ignores the user's `config.toml` (`--ignore-user-config`, so none
//!   of the user's other MCP servers are attached) and gets `montybrain-mcp`
//!   through `-c` overrides.
//! * Read-only recipes run `montybrain-mcp` with `MONTYBRAIN_READONLY=1`.
//! * The repository state the `postep` recipe needs is collected by the app
//!   itself (see [`super::git`]) and passed in the prompt.

use std::ffi::OsString;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use montybrain_core::Project;

use super::ProviderId;
use super::agent::AgentChoice;
use super::git::RepoRef;
use super::recipes::Plan;

/// Removed from every child environment so the CLIs use the subscription login
/// (and do not think they run inside another agent session).
pub const SCRUBBED_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    // Markers of a parent Codex sandbox (only present when the app itself was
    // started from inside a Codex session, e.g. in development).
    "CODEX_SANDBOX",
    "CODEX_SANDBOX_NETWORK_DISABLED",
];

/// Read-only switch of `montybrain-mcp`.
pub const ENV_READONLY: &str = "MONTYBRAIN_READONLY";
/// Its pre-rename name (still honoured by `montybrain-mcp`): never inherited
/// by a child either, so a stray value cannot flip a writing run.
pub const LEGACY_ENV_READONLY: &str = "MORWISKO_READONLY";
/// Vault root of `montybrain-mcp`.
pub const ENV_VAULT: &str = "MONTYBRAIN_DIR";
/// The MCP server binary.
pub const MCP_BINARY: &str = "montybrain-mcp";

pub const MONTYBRAIN_READ_TOOLS: &[&str] = &[
    "mcp__montybrain__overview",
    "mcp__montybrain__list_tasks",
    "mcp__montybrain__get_topic",
    "mcp__montybrain__search",
    "mcp__montybrain__connections",
];

pub const MONTYBRAIN_WRITE_TOOLS: &[&str] = &[
    "mcp__montybrain__create_task",
    "mcp__montybrain__update_task",
    "mcp__montybrain__create_topic",
    "mcp__montybrain__log_update",
];

/// Built-in Claude Code tools denied explicitly. With `dontAsk` anything not
/// in `--allowedTools` is denied already; the deny rules also beat allow rules
/// the user may have in their own Claude Code settings.
pub const CLAUDE_DENIED_TOOLS: &[&str] = &[
    "Bash",
    "Read",
    "Write",
    "Edit",
    "NotebookEdit",
    "Glob",
    "Grep",
    "WebFetch",
    "WebSearch",
];

/// Claude Code `--allowedTools` for a background run: MontyBrain tools only.
pub fn claude_allowed_tools(plan: &Plan) -> Vec<&'static str> {
    if !plan.recipe.vault_tools {
        return Vec::new();
    }
    let mut tools = MONTYBRAIN_READ_TOOLS.to_vec();
    if plan.writes() {
        tools.extend_from_slice(MONTYBRAIN_WRITE_TOOLS);
    }
    tools
}

/// Expands a leading `~/` (repo paths are often written that way).
fn expand_home(p: &str, home: &Path) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if p == "~" => home.to_path_buf(),
        None => PathBuf::from(p),
    }
}

fn selected<'a>(
    projects: &'a [Project],
    project: Option<&'a str>,
) -> impl Iterator<Item = &'a Project> {
    projects.iter().filter(move |p| match project {
        Some(slug) => p.slug == slug,
        None => p.state == montybrain_core::ProjectState::Active,
    })
}

/// Existing repo directories of the selected project (or of every active
/// project when none is selected), absolute, deduplicated, in vault order.
pub fn repo_dirs(projects: &[Project], project: Option<&str>, home: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for p in selected(projects, project) {
        for r in &p.repos {
            let path = expand_home(r.path.trim(), home);
            if path.is_absolute()
                && path.is_dir()
                && path.to_str().is_some()
                && !out.contains(&path)
            {
                out.push(path);
            }
        }
    }
    out
}

/// Every repo entry of the selected project(s), for the git snapshot: also
/// the missing or relative ones (the snapshot notes them), deduplicated by path.
pub fn repo_refs(projects: &[Project], project: Option<&str>, home: &Path) -> Vec<RepoRef> {
    let mut out: Vec<RepoRef> = Vec::new();
    for p in selected(projects, project) {
        for r in &p.repos {
            let path = expand_home(r.path.trim(), home);
            if !out.iter().any(|x| x.path == path) {
                out.push(RepoRef {
                    label: format!("{} / {}", p.slug, r.name),
                    path,
                });
            }
        }
    }
    out
}

/// The `montybrain-mcp` server a background run attaches.
#[derive(Debug, Clone, Copy)]
pub struct McpServer<'a> {
    pub binary: &'a Path,
    /// Absolute vault root the app uses.
    pub vault_root: &'a Path,
}

fn utf8(p: &Path) -> Result<&str, String> {
    p.to_str()
        .filter(|s| !s.contains('\0'))
        .ok_or_else(|| format!("Ścieżka nie jest w UTF-8: {}", p.display()))
}

/// A TOML basic string (`"…"`) holding exactly `s`.
pub fn toml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The `-c` values that attach `montybrain-mcp` to a Codex run.
pub fn codex_mcp_overrides(plan: &Plan, mcp: &McpServer) -> Result<Vec<String>, String> {
    codex_server_overrides(mcp, plan.read_only())
}

pub fn codex_server_overrides(mcp: &McpServer, read_only: bool) -> Result<Vec<String>, String> {
    let mut v = vec![
        format!(
            "mcp_servers.montybrain.command={}",
            toml_string(utf8(mcp.binary)?)
        ),
        format!(
            "mcp_servers.montybrain.env.{ENV_VAULT}={}",
            toml_string(utf8(mcp.vault_root)?)
        ),
    ];
    if read_only {
        v.push(format!("mcp_servers.montybrain.env.{ENV_READONLY}=\"1\""));
    }
    Ok(v)
}

/// The `--mcp-config` JSON of a Claude Code run.
pub fn claude_mcp_config(plan: &Plan, mcp: &McpServer) -> Result<String, String> {
    if !plan.recipe.vault_tools {
        return Ok(serde_json::json!({"mcpServers": {}}).to_string());
    }
    let mut env = serde_json::Map::new();
    env.insert(ENV_VAULT.into(), utf8(mcp.vault_root)?.into());
    if plan.read_only() {
        env.insert(ENV_READONLY.into(), "1".into());
    }
    let v = serde_json::json!({
        "mcpServers": {
            "montybrain": {
                "command": utf8(mcp.binary)?,
                "args": [],
                "env": env,
            }
        }
    });
    Ok(v.to_string())
}

/// Suffix of the per-run MCP config files in the runs dir.
pub const MCP_CONFIG_SUFFIX: &str = ".mcp.json";

/// An MCP config file (mode 0600) that is removed when dropped.
#[derive(Debug)]
pub struct McpConfigFile {
    path: PathBuf,
}

impl McpConfigFile {
    /// Writes `<dir>/<run_id>.mcp.json`. Leftovers of earlier runs (only one
    /// background run exists at a time) are removed first.
    pub fn write(dir: &Path, run_id: &str, contents: &str) -> std::io::Result<Self> {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let ours = e
                    .file_name()
                    .to_str()
                    .is_some_and(|n| n.ends_with(MCP_CONFIG_SUFFIX));
                if ours {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        let path = dir.join(format!("{run_id}{MCP_CONFIG_SUFFIX}"));
        let file = McpConfigFile { path };
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&file.path)?;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
        Ok(file)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for McpConfigFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// argv (without the program) of a background run. `mcp_config` is the
/// written [`claude_mcp_config`] (Claude Code only).
pub fn background_args(
    plan: &Plan,
    prompt: &str,
    cwd: &Path,
    mcp: &McpServer,
    mcp_config: Option<&Path>,
) -> Result<Vec<String>, String> {
    background_args_with(plan, prompt, cwd, mcp, mcp_config, &AgentChoice::default())
}

/// [`background_args`] with the model / effort of Settings → "Agenci"
/// ([`super::agent::args`]): Claude Code right after the prompt (before the
/// variadic tool lists), Codex right before the prompt.
pub fn background_args_with(
    plan: &Plan,
    prompt: &str,
    cwd: &Path,
    mcp: &McpServer,
    mcp_config: Option<&Path>,
    agent: &AgentChoice,
) -> Result<Vec<String>, String> {
    let extra = super::agent::args(plan.provider, agent);
    let mut a: Vec<String> = Vec::new();
    match plan.provider {
        ProviderId::Claude => {
            let config = mcp_config.ok_or("Błąd wewnętrzny: brak konfiguracji MCP.")?;
            a.extend(["-p".into(), prompt.into()]);
            a.extend(extra);
            a.extend(
                [
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--permission-mode",
                    "dontAsk",
                    "--permission-prompts",
                    "none",
                    "--strict-mcp-config",
                    "--mcp-config",
                ]
                .map(String::from),
            );
            a.push(utf8(config)?.into());
            a.push("--disallowedTools".into());
            a.extend(CLAUDE_DENIED_TOOLS.iter().map(|t| t.to_string()));
            if plan.recipe.vault_tools {
                a.push("--allowedTools".into());
                a.extend(claude_allowed_tools(plan).into_iter().map(String::from));
            } else {
                // No tools at all: the data is in the prompt. The deny rule
                // beats any allow rule in the user's own settings.
                a.push("mcp__montybrain".into());
            }
        }
        ProviderId::Codex => {
            a.extend(["exec", "--json", "--skip-git-repo-check"].map(String::from));
            if plan.writes() {
                // Codex refuses MCP tools not annotated read-only unless
                // approvals are delegated; this cannot be combined with `-s`.
                a.push("--approve-for-me".into());
            } else {
                a.extend(["-s".into(), "read-only".into()]);
            }
            a.push("-C".into());
            a.push(utf8(cwd)?.into());
            a.push("--ignore-user-config".into());
            if plan.recipe.vault_tools {
                for o in codex_mcp_overrides(plan, mcp)? {
                    a.extend(["-c".into(), o]);
                }
            }
            a.extend(extra);
            a.push(prompt.into());
        }
    }
    Ok(a)
}

/// Every MontyBrain tool that can write (also `command`, whose `apply` writes).
pub const MONTYBRAIN_WRITING_TOOLS: &[&str] = &[
    "mcp__montybrain__create_task",
    "mcp__montybrain__update_task",
    "mcp__montybrain__create_topic",
    "mcp__montybrain__log_update",
    "mcp__montybrain__command",
];

/// The `--mcp-config` JSON of a Terminal Skrzynka session: `montybrain-mcp`
/// read-only (it refuses every write).
pub fn read_only_mcp_config(mcp: &McpServer) -> Result<String, String> {
    Ok(serde_json::json!({
        "mcpServers": {
            "montybrain": {
                "command": utf8(mcp.binary)?,
                "args": [],
                "env": {ENV_VAULT: utf8(mcp.vault_root)?, ENV_READONLY: "1"},
            }
        }
    })
    .to_string())
}

/// Options of a Terminal Skrzynka session ("Moje narzędzia"), between the
/// program and the prompt. The user's own configuration stays (their MCP
/// servers, connectors, skills, Codex computer use if they turned it on) and
/// tools ask for approval in the terminal; MontyBrain is attached read-only
/// and cannot be written:
/// * Claude Code: our `montybrain-mcp` with `MONTYBRAIN_READONLY=1` on top of
///   the user's servers, every writing MontyBrain tool denied by name (also a
///   `montybrain` server of the user's own), file edits in the vault denied,
///   the one answer file allowed, and the default permission mode (prompts,
///   whatever the user's default is). `--permission-mode` also ends the
///   variadic lists before the prompt.
/// * Codex: the `montybrain` server overridden to the read-only one, the
///   workspace-write sandbox rooted in the answers directory (shell commands
///   can write only there, not in the vault) and approvals on request.
pub fn inbox_terminal_args(
    provider: ProviderId,
    mcp: &McpServer,
    mcp_config: Option<&Path>,
    answers_dir: &Path,
    answer: &Path,
) -> Result<Vec<String>, String> {
    let vault = utf8(mcp.vault_root)?.trim_end_matches('/');
    let mut a: Vec<String> = Vec::new();
    match provider {
        ProviderId::Claude => {
            let config = mcp_config.ok_or("Błąd wewnętrzny: brak konfiguracji MCP.")?;
            a.push("--mcp-config".into());
            a.push(utf8(config)?.into());
            a.push("--add-dir".into());
            a.push(utf8(answers_dir)?.into());
            // One rule per argv element (paths may contain spaces); `//`
            // starts an absolute path in Claude Code permission rules.
            a.push("--disallowedTools".into());
            a.extend(MONTYBRAIN_WRITING_TOOLS.iter().map(|t| t.to_string()));
            a.push(format!("Edit(/{vault}/**)"));
            a.push(format!("Write(/{vault}/**)"));
            a.push("--allowedTools".into());
            let answer = utf8(answer)?;
            a.push(format!("Edit(/{answer})"));
            a.push(format!("Write(/{answer})"));
            a.extend(["--permission-mode".into(), "default".into()]);
        }
        ProviderId::Codex => {
            for o in codex_server_overrides(mcp, true)? {
                a.extend(["-c".into(), o]);
            }
            a.extend([
                "-s".into(),
                "workspace-write".into(),
                "-a".into(),
                "on-request".into(),
                "-C".into(),
                utf8(answers_dir)?.into(),
            ]);
        }
    }
    Ok(a)
}

/// The full environment of a child: the parent's minus [`SCRUBBED_ENV`] (and
/// any inherited read-only switch), with `PATH` replaced and, for runs that
/// must not write, `MONTYBRAIN_READONLY=1` (passed on to `montybrain-mcp` by CLIs
/// that let MCP servers inherit their environment).
pub fn child_env(
    parent: impl IntoIterator<Item = (OsString, OsString)>,
    path: OsString,
    read_only: bool,
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = parent
        .into_iter()
        .filter(|(k, _)| {
            let k = k.to_string_lossy();
            k != "PATH"
                && k != ENV_READONLY
                && k != LEGACY_ENV_READONLY
                && !SCRUBBED_ENV.contains(&k.as_ref())
        })
        .collect();
    env.push(("PATH".into(), path));
    if read_only {
        env.push((ENV_READONLY.into(), "1".into()));
    }
    env.sort();
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::recipes::{tests::req, validate};
    use montybrain_core::{ProjectState, Repo};

    fn projects(repo_a: &Path, repo_b: &Path) -> Vec<Project> {
        let p = |slug: &str, state, repos: Vec<Repo>| Project {
            slug: slug.into(),
            name: slug.into(),
            color: None,
            state,
            repos,
            description: String::new(),
            path: format!("{slug}/projekt.md").into(),
        };
        let repo = |path: &str| Repo {
            name: "r".into(),
            path: path.into(),
        };
        vec![
            p(
                "beeking",
                ProjectState::Active,
                vec![
                    repo(repo_a.to_str().unwrap()),
                    repo("/nie/istnieje"),
                    repo("wzgledna/sciezka"),
                    repo(repo_a.to_str().unwrap()),
                ],
            ),
            p(
                "dom",
                ProjectState::Active,
                vec![repo(repo_b.to_str().unwrap())],
            ),
            p("stary", ProjectState::Archived, vec![repo("/tmp")]),
        ]
    }

    fn plan(recipe: &str, provider: &str, params: &[(&str, &str)], projects: &[Project]) -> Plan {
        validate(&req(recipe, provider, "background", params), projects).unwrap()
    }

    fn mcp() -> McpServer<'static> {
        McpServer {
            binary: Path::new("/h/.local/bin/montybrain-mcp"),
            vault_root: Path::new("/h/MontyBrain"),
        }
    }

    fn args(plan: &Plan) -> Vec<String> {
        background_args(
            plan,
            "PROMPT",
            Path::new("/c/runs"),
            &mcp(),
            Some(Path::new("/c/runs/r1.mcp.json")),
        )
        .unwrap()
    }

    #[test]
    fn claude_argv_per_recipe() {
        let a = tempfile::tempdir().unwrap();
        let ps = projects(a.path(), a.path());
        let head = [
            "-p",
            "PROMPT",
            "--output-format",
            "stream-json",
            "--verbose",
            "--permission-mode",
            "dontAsk",
            "--permission-prompts",
            "none",
            "--strict-mcp-config",
            "--mcp-config",
            "/c/runs/r1.mcp.json",
            "--disallowedTools",
            "Bash",
            "Read",
            "Write",
            "Edit",
            "NotebookEdit",
            "Glob",
            "Grep",
            "WebFetch",
            "WebSearch",
            "--allowedTools",
            "mcp__montybrain__overview",
            "mcp__montybrain__list_tasks",
            "mcp__montybrain__get_topic",
            "mcp__montybrain__search",
            "mcp__montybrain__connections",
        ];
        for (recipe, params) in [
            ("postep", vec![("project", "beeking")]),
            ("postep", vec![]),
            ("raport", vec![]),
        ] {
            assert_eq!(
                args(&plan(recipe, "claude", &params, &ps)),
                head,
                "{recipe}"
            );
        }
        let mut write = head.to_vec();
        write.extend([
            "mcp__montybrain__create_task",
            "mcp__montybrain__update_task",
            "mcp__montybrain__create_topic",
            "mcp__montybrain__log_update",
        ]);
        let z = plan("zadania", "claude", &[("text", "x")], &ps);
        assert_eq!(args(&z), write);
        // No shell, no file tools, no extra directories, ever.
        for p in [&z, &plan("postep", "claude", &[], &ps)] {
            let a = args(p);
            let allowed = &a[a.iter().position(|x| x == "--allowedTools").unwrap() + 1..];
            assert!(
                allowed.iter().all(|t| t.starts_with("mcp__montybrain__")),
                "{a:?}"
            );
            assert!(!a.iter().any(|x| x == "--add-dir" || x == "--bare"));
        }
        // The config file is required.
        assert!(background_args(&z, "P", Path::new("/c"), &mcp(), None).is_err());
    }

    #[test]
    fn codex_argv_per_recipe() {
        let a = tempfile::tempdir().unwrap();
        let ps = projects(a.path(), a.path());
        let ro = [
            "exec",
            "--json",
            "--skip-git-repo-check",
            "-s",
            "read-only",
            "-C",
            "/c/runs",
            "--ignore-user-config",
            "-c",
            "mcp_servers.montybrain.command=\"/h/.local/bin/montybrain-mcp\"",
            "-c",
            "mcp_servers.montybrain.env.MONTYBRAIN_DIR=\"/h/MontyBrain\"",
            "-c",
            "mcp_servers.montybrain.env.MONTYBRAIN_READONLY=\"1\"",
            "PROMPT",
        ];
        for (recipe, params) in [("postep", vec![("project", "beeking")]), ("raport", vec![])] {
            let p = plan(recipe, "codex", &params, &ps);
            assert_eq!(args(&p), ro, "{recipe}");
        }
        let z = plan("zadania", "codex", &[("text", "x")], &ps);
        assert_eq!(
            args(&z),
            [
                "exec",
                "--json",
                "--skip-git-repo-check",
                "--approve-for-me",
                "-C",
                "/c/runs",
                "--ignore-user-config",
                "-c",
                "mcp_servers.montybrain.command=\"/h/.local/bin/montybrain-mcp\"",
                "-c",
                "mcp_servers.montybrain.env.MONTYBRAIN_DIR=\"/h/MontyBrain\"",
                "PROMPT"
            ]
        );
        // Codex needs no config file.
        assert_eq!(
            background_args(&z, "PROMPT", Path::new("/c/runs"), &mcp(), None).unwrap(),
            args(&z)
        );
    }

    #[test]
    fn agent_flags_in_background_argv() {
        let a = tempfile::tempdir().unwrap();
        let ps = projects(a.path(), a.path());
        let choice = AgentChoice {
            model: Some("opus".into()),
            effort: Some("high".into()),
        };
        let p = plan("raport", "claude", &[], &ps);
        let v = background_args_with(
            &p,
            "PROMPT",
            Path::new("/c"),
            &mcp(),
            Some(Path::new("/c/m.json")),
            &choice,
        )
        .unwrap();
        assert_eq!(
            v[..6],
            ["-p", "PROMPT", "--model", "opus", "--effort", "high"]
        );
        let p = plan("raport", "codex", &[], &ps);
        let choice = AgentChoice {
            model: Some("gpt-6-sol".into()),
            effort: Some("xhigh".into()),
        };
        let v = background_args_with(&p, "PROMPT", Path::new("/c"), &mcp(), None, &choice).unwrap();
        assert_eq!(
            v[v.len() - 5..],
            [
                "-m",
                "gpt-6-sol",
                "-c",
                "model_reasoning_effort=\"xhigh\"",
                "PROMPT"
            ]
        );
        // Default: exactly today's argv.
        assert_eq!(
            background_args_with(
                &p,
                "PROMPT",
                Path::new("/c"),
                &mcp(),
                None,
                &AgentChoice::default()
            )
            .unwrap(),
            background_args(&p, "PROMPT", Path::new("/c"), &mcp(), None).unwrap()
        );
    }

    #[test]
    fn toml_quoting_round_trips() {
        for s in [
            "/Users/x/My Vault",
            "/Users/x/\"cytat\"/a'b",
            r"C:\temp\nie\n",
            "tab\there\nnew\rline \u{1b}[31m \u{7f} ąę ✓",
            "",
            "\"\"\" ''' ]] = # [x]",
        ] {
            let q = toml_string(s);
            let doc = format!("k = {q}");
            let parsed: toml::Table = toml::from_str(&doc).unwrap_or_else(|e| panic!("{doc}: {e}"));
            assert_eq!(parsed["k"].as_str(), Some(s), "{q}");
            assert!(!q.contains('\n') && !q.contains('\r'));
        }
        // As whole `-c key=value` elements, parsed the way Codex does
        // (dotted key, then a TOML value).
        let ps = projects(Path::new("/x"), Path::new("/y"));
        let p = plan("postep", "codex", &[], &ps);
        let mcp = McpServer {
            binary: Path::new("/Users/a b/\"q\"/back\\slash/montybrain-mcp"),
            vault_root: Path::new("/Users/a b/Mój \"vault\"\\x"),
        };
        let mut table = toml::Table::new();
        for o in codex_mcp_overrides(&p, &mcp).unwrap() {
            let (key, value) = o.split_once('=').unwrap();
            let v: toml::Table = toml::from_str(&format!("v = {value}")).unwrap();
            table.insert(key.to_string(), v["v"].clone());
        }
        assert_eq!(
            table["mcp_servers.montybrain.command"].as_str(),
            mcp.binary.to_str()
        );
        assert_eq!(
            table["mcp_servers.montybrain.env.MONTYBRAIN_DIR"].as_str(),
            mcp.vault_root.to_str()
        );
        assert_eq!(
            table["mcp_servers.montybrain.env.MONTYBRAIN_READONLY"].as_str(),
            Some("1")
        );
        assert_eq!(table.len(), 3);
    }

    #[test]
    fn claude_mcp_config_content_and_file() {
        let ps = projects(Path::new("/x"), Path::new("/y"));
        let mcp = McpServer {
            binary: Path::new("/h/.local/bin/montybrain-mcp"),
            vault_root: Path::new("/h/Mój \"vault\""),
        };
        let json = |p: &Plan| -> serde_json::Value {
            serde_json::from_str(&claude_mcp_config(p, &mcp).unwrap()).unwrap()
        };
        assert_eq!(
            json(&plan("raport", "claude", &[], &ps)),
            serde_json::json!({"mcpServers":{"montybrain":{
                "command":"/h/.local/bin/montybrain-mcp","args":[],
                "env":{"MONTYBRAIN_DIR":"/h/Mój \"vault\"","MONTYBRAIN_READONLY":"1"}}}})
        );
        assert_eq!(
            json(&plan("zadania", "claude", &[("text", "x")], &ps))["mcpServers"]["montybrain"]["env"],
            serde_json::json!({"MONTYBRAIN_DIR":"/h/Mój \"vault\""})
        );

        let dir = tempfile::tempdir().unwrap();
        let stale = dir.path().join("old-run.mcp.json");
        std::fs::write(&stale, "{}").unwrap();
        let keep = dir.path().join("other.txt");
        std::fs::write(&keep, "x").unwrap();
        let f = McpConfigFile::write(dir.path(), "r1", "{\"a\":1}").unwrap();
        assert_eq!(f.path(), dir.path().join("r1.mcp.json"));
        assert_eq!(std::fs::read_to_string(f.path()).unwrap(), "{\"a\":1}");
        let mode = std::fs::metadata(f.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(!stale.exists() && keep.exists());
        let path = f.path().to_path_buf();
        drop(f);
        assert!(!path.exists(), "removed on drop");
    }

    #[test]
    fn hostile_text_is_one_argv_element() {
        let ps = projects(Path::new("/x"), Path::new("/y"));
        let hostile = "--dangerously-skip-permissions\n$(rm -rf ~) `id` \"'\n/montybrain-poczta";
        for provider in ["claude", "codex"] {
            let p = plan("zadania", provider, &[("text", hostile)], &ps);
            let prompt = crate::runs::recipes::prompt(&p, None);
            let a = background_args(
                &p,
                &prompt,
                Path::new("/c"),
                &mcp(),
                Some(Path::new("/c/m.json")),
            )
            .unwrap();
            let with_text: Vec<_> = a.iter().filter(|x| x.contains("--dangerously")).collect();
            assert_eq!(with_text, vec![&prompt], "{provider}");
            assert!(!a.iter().any(|x| x == "--dangerously-skip-permissions"));
            // The prompt never starts with '-', so it cannot be read as a flag.
            assert!(!prompt.starts_with('-'));
        }
    }

    #[test]
    fn repo_refs_keep_missing_entries_once() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let ps = projects(a.path(), b.path());
        let refs = repo_refs(&ps, Some("beeking"), Path::new("/h"));
        let got: Vec<(&str, &Path)> = refs
            .iter()
            .map(|r| (r.label.as_str(), r.path.as_path()))
            .collect();
        assert_eq!(
            got,
            [
                ("beeking / r", a.path()),
                ("beeking / r", Path::new("/nie/istnieje")),
                ("beeking / r", Path::new("wzgledna/sciezka")),
            ]
        );
        // No project: active ones only.
        let all = repo_refs(&ps, None, Path::new("/h"));
        assert_eq!(all.len(), 4);
        assert!(!all.iter().any(|r| r.path == Path::new("/tmp")));
    }

    #[test]
    fn env_is_scrubbed() {
        let parent: Vec<(OsString, OsString)> = [
            ("HOME", "/Users/x"),
            ("PATH", "/evil:/usr/bin"),
            ("ANTHROPIC_API_KEY", "sk-ant"),
            ("ANTHROPIC_AUTH_TOKEN", "t"),
            ("CLAUDE_CODE_USE_BEDROCK", "1"),
            ("CLAUDE_CODE_USE_VERTEX", "1"),
            ("OPENAI_API_KEY", "sk"),
            ("CODEX_API_KEY", "c"),
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_ENTRYPOINT", "cli"),
            ("CODEX_SANDBOX", "seatbelt"),
            ("MONTYBRAIN_READONLY", "1"),
            ("MORWISKO_READONLY", "1"),
            ("CLAUDE_CODE_OAUTH_TOKEN", "keep"),
            ("LANG", "pl_PL.UTF-8"),
        ]
        .iter()
        .map(|(k, v)| (OsString::from(k), OsString::from(v)))
        .collect();
        let env = child_env(parent.clone(), "/new/path".into(), false);
        let keys: Vec<_> = env.iter().map(|(k, _)| k.to_str().unwrap()).collect();
        assert_eq!(keys, ["CLAUDE_CODE_OAUTH_TOKEN", "HOME", "LANG", "PATH"]);
        assert!(env.contains(&("PATH".into(), "/new/path".into())));

        let ro = child_env(parent, "/p".into(), true);
        assert!(ro.contains(&("MONTYBRAIN_READONLY".into(), "1".into())));
        assert!(!ro.iter().any(|(k, _)| k == "MORWISKO_READONLY"));
        assert!(!ro.iter().any(|(k, _)| k == "ANTHROPIC_API_KEY"));
    }

    #[test]
    fn inbox_terminal_args_keep_montybrain_read_only() {
        let server = McpServer {
            binary: Path::new("/bin/montybrain-mcp"),
            vault_root: Path::new("/Users/a/Mój vault"),
        };
        let dir = Path::new("/c/terminal dir");
        let answer = Path::new("/c/terminal dir/r1.odpowiedz.md");
        let a = inbox_terminal_args(
            ProviderId::Claude,
            &server,
            Some(Path::new("/c/r1.mcp.json")),
            dir,
            answer,
        )
        .unwrap();
        let deny = a.iter().position(|x| x == "--disallowedTools").unwrap();
        let allow = a.iter().position(|x| x == "--allowedTools").unwrap();
        for t in MONTYBRAIN_WRITING_TOOLS {
            assert!(a[deny..allow].iter().any(|x| x == t), "{t}: {a:?}");
        }
        assert!(
            a[deny..allow].contains(&"Edit(//Users/a/Mój vault/**)".to_string()),
            "{a:?}"
        );
        assert_eq!(a[allow + 1], "Edit(//c/terminal dir/r1.odpowiedz.md)");
        // The variadic lists end before the prompt.
        assert_eq!(a[a.len() - 2..], ["--permission-mode", "default"]);
        let json: serde_json::Value =
            serde_json::from_str(&read_only_mcp_config(&server).unwrap()).unwrap();
        assert_eq!(
            json["mcpServers"]["montybrain"]["env"]["MONTYBRAIN_READONLY"],
            "1"
        );
        assert_eq!(
            json["mcpServers"]["montybrain"]["env"]["MONTYBRAIN_DIR"],
            "/Users/a/Mój vault"
        );
        let c = inbox_terminal_args(ProviderId::Codex, &server, None, dir, answer).unwrap();
        assert!(
            c.contains(&"mcp_servers.montybrain.env.MONTYBRAIN_READONLY=\"1\"".to_string()),
            "{c:?}"
        );
        assert!(
            c.windows(2)
                .any(|w| w[0] == "-s" && w[1] == "workspace-write")
        );
        assert!(
            c.windows(2)
                .any(|w| w[0] == "-C" && w[1] == "/c/terminal dir")
        );
        assert!(
            !c.iter()
                .any(|x| x.contains("danger") || x == "--approve-for-me")
        );
    }

    #[test]
    fn inbox_background_runs_get_montybrain_read_only_only() {
        let plan = super::super::recipes::validate(
            &super::super::recipes::tests::req(
                "skrzynka",
                "claude",
                "background",
                &[("dane", "x")],
            ),
            &super::super::recipes::tests::projects(),
        )
        .unwrap();
        let server = McpServer {
            binary: Path::new("/bin/montybrain-mcp"),
            vault_root: Path::new("/v"),
        };
        let a = background_args(
            &plan,
            "p",
            Path::new("/tmp"),
            &server,
            Some(Path::new("/tmp/c.json")),
        )
        .unwrap();
        // Only the MontyBrain read tools; every built-in tool denied.
        let allow = a.iter().position(|x| x == "--allowedTools").unwrap();
        assert_eq!(
            a[allow + 1..],
            MONTYBRAIN_READ_TOOLS
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
        );
        assert!(
            !a.iter().any(|x| x == "mcp__montybrain__create_task"),
            "{a:?}"
        );
        assert!(a.iter().any(|x| x == "--strict-mcp-config"));
        let cfg: serde_json::Value =
            serde_json::from_str(&claude_mcp_config(&plan, &server).unwrap()).unwrap();
        assert_eq!(
            cfg["mcpServers"]["montybrain"]["env"]["MONTYBRAIN_READONLY"],
            "1"
        );
        let plan = super::super::recipes::validate(
            &super::super::recipes::tests::req("skrzynka", "codex", "background", &[("dane", "x")]),
            &super::super::recipes::tests::projects(),
        )
        .unwrap();
        let a = background_args(&plan, "p", Path::new("/tmp"), &server, None).unwrap();
        assert!(
            a.contains(&"mcp_servers.montybrain.env.MONTYBRAIN_READONLY=\"1\"".to_string()),
            "{a:?}"
        );
        assert!(a.windows(2).any(|w| w[0] == "-s" && w[1] == "read-only"));
        assert!(a.iter().any(|x| x == "--ignore-user-config"));
    }

    #[test]
    fn home_expansion() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join("repo")).unwrap();
        let ps = vec![Project {
            slug: "p".into(),
            name: "p".into(),
            color: None,
            state: ProjectState::Active,
            repos: vec![Repo {
                name: "r".into(),
                path: "~/repo".into(),
            }],
            description: String::new(),
            path: "p/projekt.md".into(),
        }];
        assert_eq!(
            repo_dirs(&ps, Some("p"), home.path()),
            vec![home.path().join("repo")]
        );
        assert!(repo_dirs(&ps, Some("inny"), home.path()).is_empty());
    }
}
