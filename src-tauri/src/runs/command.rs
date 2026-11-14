//! argv and environment of a background run. Every element comes from the
//! recipe table, the resolved provider, the app's own directories and the
//! vault root, or validated parameters; the prompt is always one single argv
//! element. No shell is involved.
//!
//! Isolation of a background run:
//! * Claude Code gets no built-in tools at all (no Bash, no file tools): only
//!   the Morwisko MCP tools are allowed, and `--strict-mcp-config` with an
//!   app-written config attaches `morwisko-mcp` as the only MCP server.
//! * Codex ignores the user's `config.toml` (`--ignore-user-config`, so none
//!   of the user's other MCP servers are attached) and gets `morwisko-mcp`
//!   through `-c` overrides.
//! * Read-only recipes run `morwisko-mcp` with `MORWISKO_READONLY=1`.
//! * The repository state the `postep` recipe needs is collected by the app
//!   itself (see [`super::git`]) and passed in the prompt.

use std::ffi::OsString;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use morwisko_core::Project;

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

/// Read-only switch of `morwisko-mcp`.
pub const ENV_READONLY: &str = "MORWISKO_READONLY";
/// Vault root of `morwisko-mcp`.
pub const ENV_VAULT: &str = "MORWISKO_DIR";
/// The MCP server binary.
pub const MCP_BINARY: &str = "morwisko-mcp";

pub const MORWISKO_READ_TOOLS: &[&str] = &[
    "mcp__morwisko__overview",
    "mcp__morwisko__list_tasks",
    "mcp__morwisko__get_topic",
    "mcp__morwisko__search",
];

pub const MORWISKO_WRITE_TOOLS: &[&str] = &[
    "mcp__morwisko__create_task",
    "mcp__morwisko__update_task",
    "mcp__morwisko__create_topic",
    "mcp__morwisko__log_update",
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

/// Claude Code `--allowedTools` for a background run: Morwisko tools only.
pub fn claude_allowed_tools(plan: &Plan) -> Vec<&'static str> {
    let mut tools = MORWISKO_READ_TOOLS.to_vec();
    if plan.writes() {
        tools.extend_from_slice(MORWISKO_WRITE_TOOLS);
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
        None => p.state == morwisko_core::ProjectState::Active,
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

/// The `morwisko-mcp` server a background run attaches.
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

/// The `-c` values that attach `morwisko-mcp` to a Codex run.
pub fn codex_mcp_overrides(plan: &Plan, mcp: &McpServer) -> Result<Vec<String>, String> {
    let mut v = vec![
        format!(
            "mcp_servers.morwisko.command={}",
            toml_string(utf8(mcp.binary)?)
        ),
        format!(
            "mcp_servers.morwisko.env.{ENV_VAULT}={}",
            toml_string(utf8(mcp.vault_root)?)
        ),
    ];
    if plan.read_only() {
        v.push(format!("mcp_servers.morwisko.env.{ENV_READONLY}=\"1\""));
    }
    Ok(v)
}

/// The `--mcp-config` JSON of a Claude Code run.
pub fn claude_mcp_config(plan: &Plan, mcp: &McpServer) -> Result<String, String> {
    let mut env = serde_json::Map::new();
    env.insert(ENV_VAULT.into(), utf8(mcp.vault_root)?.into());
    if plan.read_only() {
        env.insert(ENV_READONLY.into(), "1".into());
    }
    let v = serde_json::json!({
        "mcpServers": {
            "morwisko": {
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
            a.push("--allowedTools".into());
            a.extend(claude_allowed_tools(plan).into_iter().map(String::from));
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
            for o in codex_mcp_overrides(plan, mcp)? {
                a.extend(["-c".into(), o]);
            }
            a.extend(extra);
            a.push(prompt.into());
        }
    }
    Ok(a)
}

/// The full environment of a child: the parent's minus [`SCRUBBED_ENV`] (and
/// any inherited read-only switch), with `PATH` replaced and, for runs that
/// must not write, `MORWISKO_READONLY=1` (passed on to `morwisko-mcp` by CLIs
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
            k != "PATH" && k != ENV_READONLY && !SCRUBBED_ENV.contains(&k.as_ref())
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
    use morwisko_core::{ProjectState, Repo};

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
            binary: Path::new("/h/.local/bin/morwisko-mcp"),
            vault_root: Path::new("/h/Morwisko"),
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
            "mcp__morwisko__overview",
            "mcp__morwisko__list_tasks",
            "mcp__morwisko__get_topic",
            "mcp__morwisko__search",
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
            "mcp__morwisko__create_task",
            "mcp__morwisko__update_task",
            "mcp__morwisko__create_topic",
            "mcp__morwisko__log_update",
        ]);
        let z = plan("zadania", "claude", &[("text", "x")], &ps);
        assert_eq!(args(&z), write);
        // No shell, no file tools, no extra directories, ever.
        for p in [&z, &plan("postep", "claude", &[], &ps)] {
            let a = args(p);
            let allowed = &a[a.iter().position(|x| x == "--allowedTools").unwrap() + 1..];
            assert!(
                allowed.iter().all(|t| t.starts_with("mcp__morwisko__")),
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
            "mcp_servers.morwisko.command=\"/h/.local/bin/morwisko-mcp\"",
            "-c",
            "mcp_servers.morwisko.env.MORWISKO_DIR=\"/h/Morwisko\"",
            "-c",
            "mcp_servers.morwisko.env.MORWISKO_READONLY=\"1\"",
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
                "mcp_servers.morwisko.command=\"/h/.local/bin/morwisko-mcp\"",
                "-c",
                "mcp_servers.morwisko.env.MORWISKO_DIR=\"/h/Morwisko\"",
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
            binary: Path::new("/Users/a b/\"q\"/back\\slash/morwisko-mcp"),
            vault_root: Path::new("/Users/a b/Mój \"vault\"\\x"),
        };
        let mut table = toml::Table::new();
        for o in codex_mcp_overrides(&p, &mcp).unwrap() {
            let (key, value) = o.split_once('=').unwrap();
            let v: toml::Table = toml::from_str(&format!("v = {value}")).unwrap();
            table.insert(key.to_string(), v["v"].clone());
        }
        assert_eq!(
            table["mcp_servers.morwisko.command"].as_str(),
            mcp.binary.to_str()
        );
        assert_eq!(
            table["mcp_servers.morwisko.env.MORWISKO_DIR"].as_str(),
            mcp.vault_root.to_str()
        );
        assert_eq!(
            table["mcp_servers.morwisko.env.MORWISKO_READONLY"].as_str(),
            Some("1")
        );
        assert_eq!(table.len(), 3);
    }

    #[test]
    fn claude_mcp_config_content_and_file() {
        let ps = projects(Path::new("/x"), Path::new("/y"));
        let mcp = McpServer {
            binary: Path::new("/h/.local/bin/morwisko-mcp"),
            vault_root: Path::new("/h/Mój \"vault\""),
        };
        let json = |p: &Plan| -> serde_json::Value {
            serde_json::from_str(&claude_mcp_config(p, &mcp).unwrap()).unwrap()
        };
        assert_eq!(
            json(&plan("raport", "claude", &[], &ps)),
            serde_json::json!({"mcpServers":{"morwisko":{
                "command":"/h/.local/bin/morwisko-mcp","args":[],
                "env":{"MORWISKO_DIR":"/h/Mój \"vault\"","MORWISKO_READONLY":"1"}}}})
        );
        assert_eq!(
            json(&plan("zadania", "claude", &[("text", "x")], &ps))["mcpServers"]["morwisko"]["env"],
            serde_json::json!({"MORWISKO_DIR":"/h/Mój \"vault\""})
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
        let hostile = "--dangerously-skip-permissions\n$(rm -rf ~) `id` \"'\n/morwisko-poczta";
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
        assert!(ro.contains(&("MORWISKO_READONLY".into(), "1".into())));
        assert!(!ro.iter().any(|(k, _)| k == "ANTHROPIC_API_KEY"));
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
