//! Finding `claude` / `codex`. An app started from the Dock has a minimal PATH,
//! so a fixed list of install locations is searched first, then PATH.

use std::ffi::{OsStr, OsString};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::proc::run_bounded;
use super::{ProviderId, ProviderInfo};

/// System dirs appended to the constructed PATH of child processes.
pub const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// Install locations searched before PATH, in order.
pub fn candidate_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".cargo/bin"),
    ]
}

/// Where `montybrain-mcp` is looked up before PATH: where `install.sh` puts
/// it, then next to the app's own executable (for a future bundled copy).
pub fn mcp_candidate_dirs(home: &Path) -> Vec<PathBuf> {
    let mut v = vec![home.join(".local/bin")];
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        v.push(dir);
    }
    v
}

/// PATH given to children: the candidate dirs, then the system dirs.
pub fn child_path(home: &Path) -> OsString {
    let mut out = OsString::new();
    for d in candidate_dirs(home) {
        out.push(d.as_os_str());
        out.push(":");
    }
    out.push(SYSTEM_PATH);
    out
}

fn is_executable(p: &Path) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The first executable `name` in `dirs`, then in `path_var`.
pub fn find_in(name: &str, dirs: &[PathBuf], path_var: Option<&OsStr>) -> Option<PathBuf> {
    let from_path = path_var
        .map(|p| std::env::split_paths(p).collect::<Vec<_>>())
        .unwrap_or_default();
    dirs.iter()
        .chain(from_path.iter())
        .filter(|d| d.is_absolute())
        .map(|d| d.join(name))
        .find(|p| is_executable(p))
}

/// How long `list_providers` reuses a result.
const CACHE_TTL: Duration = Duration::from_secs(30);
/// `--version` must answer within this.
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Resolver {
    dirs: Vec<PathBuf>,
    path_var: Option<OsString>,
    /// Environment for `--version` (already scrubbed, with the child PATH).
    env: Vec<(OsString, OsString)>,
    cache: Mutex<Option<(Instant, Vec<ProviderInfo>)>>,
}

impl Resolver {
    pub fn new(
        dirs: Vec<PathBuf>,
        path_var: Option<OsString>,
        env: Vec<(OsString, OsString)>,
    ) -> Self {
        Resolver {
            dirs,
            path_var,
            env,
            cache: Mutex::new(None),
        }
    }

    pub fn resolve(&self, id: ProviderId) -> Result<PathBuf, String> {
        find_in(id.binary(), &self.dirs, self.path_var.as_deref()).ok_or_else(|| missing(id))
    }

    /// Provider status, cached for a short while.
    pub fn list(&self) -> Vec<ProviderInfo> {
        if let Some((at, list)) = crate::lock(&self.cache).as_ref()
            && at.elapsed() < CACHE_TTL
        {
            return list.clone();
        }
        let list: Vec<ProviderInfo> = ProviderId::ALL.iter().map(|&id| self.info(id)).collect();
        *crate::lock(&self.cache) = Some((Instant::now(), list.clone()));
        list
    }

    fn info(&self, id: ProviderId) -> ProviderInfo {
        let mut info = ProviderInfo {
            id,
            name: id.name().into(),
            available: false,
            path: None,
            version: None,
            problem: None,
        };
        match self.resolve(id) {
            Err(problem) => info.problem = Some(problem),
            Ok(path) => {
                info.available = true;
                info.path = Some(path.display().to_string());
                match version(&path, &self.env, VERSION_TIMEOUT) {
                    Ok(v) => info.version = Some(v),
                    Err(e) => {
                        info.problem =
                            Some(format!("`{} --version` nie zadziałało: {e}", id.binary()))
                    }
                }
            }
        }
        info
    }
}

fn missing(id: ProviderId) -> String {
    let how = match id {
        ProviderId::Claude => {
            "Zainstaluj Claude Code i zaloguj się kontem z subskrypcją (`claude`, potem /login)."
        }
        ProviderId::Codex => "Zainstaluj Codex CLI i zaloguj się kontem ChatGPT (`codex login`).",
    };
    format!(
        "Nie znaleziono programu `{}` (szukano w ~/.local/bin, /opt/homebrew/bin, /usr/local/bin, \
~/.cargo/bin i w PATH). {how}",
        id.binary()
    )
}

/// First non-empty line of `<path> --version`. One deadline covers the
/// process and the draining of its output (see [`super::proc`]): a wrapper
/// that exits but leaves a background child holding stdout cannot hang the
/// probe, and whatever is left of the process group is killed at the deadline.
pub fn version(
    path: &Path,
    env: &[(OsString, OsString)],
    timeout: Duration,
) -> Result<String, String> {
    let mut cmd = Command::new(path);
    cmd.arg("--version")
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)));
    let out =
        run_bounded(cmd, timeout, 64 * 1024, 4 * 1024, &|| false).map_err(|e| e.to_string())?;
    match out.status {
        // Killed at the deadline while still running.
        None => return Err(format!("brak odpowiedzi po {} s", timeout.as_secs())),
        Some(s) if out.timed_out && s.signal().is_some() => {
            return Err(format!("brak odpowiedzi po {} s", timeout.as_secs()));
        }
        Some(s) if !s.success() => {
            return Err(format!("kod wyjścia {}", s.code().unwrap_or(-1)));
        }
        // Exited 0 (possibly before a leftover child released stdout).
        Some(_) => {}
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(200).collect())
        .ok_or_else(|| "pusta odpowiedź".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn candidates_and_child_path() {
        let home = Path::new("/Users/x");
        assert_eq!(
            child_path(home),
            OsString::from(
                "/Users/x/.local/bin:/opt/homebrew/bin:/usr/local/bin:/Users/x/.cargo/bin:\
/usr/bin:/bin:/usr/sbin:/sbin"
            )
        );
    }

    #[test]
    fn resolution_order_and_executable_bit() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let on_path = tempfile::tempdir().unwrap();
        // Not executable in the first dir: skipped.
        std::fs::write(a.path().join("claude"), "x").unwrap();
        let in_b = script(b.path(), "claude", "echo b");
        script(on_path.path(), "claude", "echo path");
        let codex = script(on_path.path(), "codex", "echo path");
        let dirs = vec![a.path().to_path_buf(), b.path().to_path_buf()];
        let path_var = std::env::join_paths([Path::new("relative"), on_path.path()]).unwrap();
        assert_eq!(find_in("claude", &dirs, Some(&path_var)), Some(in_b));
        assert_eq!(find_in("codex", &dirs, Some(&path_var)), Some(codex));
        assert_eq!(find_in("codex", &dirs, None), None);
        // A directory named like the binary is not a binary.
        std::fs::create_dir(a.path().join("dir")).unwrap();
        assert_eq!(find_in("dir", &dirs, None), None);
    }

    #[test]
    fn provider_info_and_cache() {
        let dir = tempfile::tempdir().unwrap();
        script(
            dir.path(),
            "claude",
            "echo; echo '  2.1.325 (Claude Code)  '; echo more",
        );
        let r = Resolver::new(vec![dir.path().to_path_buf()], None, vec![]);
        let list = r.list();
        assert_eq!(list.len(), 2);
        let c = &list[0];
        assert_eq!(c.id, ProviderId::Claude);
        assert!(c.available);
        assert_eq!(c.version.as_deref(), Some("2.1.325 (Claude Code)"));
        assert_eq!(c.problem, None);
        assert_eq!(
            c.path.as_deref(),
            Some(dir.path().join("claude").to_str().unwrap())
        );
        let x = &list[1];
        assert!(!x.available && x.path.is_none() && x.version.is_none());
        assert!(x.problem.as_deref().unwrap().contains("codex login"));
        let json = serde_json::to_value(x).unwrap();
        assert_eq!(json["id"], "codex");
        assert_eq!(json["name"], "Codex CLI");

        // Cached: a new binary is not seen until the TTL passes.
        script(dir.path(), "codex", "echo codex-cli 0.169.0");
        assert!(!r.list()[1].available);
        *crate::lock(&r.cache) = None;
        assert_eq!(r.list()[1].version.as_deref(), Some("codex-cli 0.169.0"));
    }

    #[test]
    fn version_failures() {
        let dir = tempfile::tempdir().unwrap();
        let bad = script(dir.path(), "bad", "echo nope >&2; exit 3");
        assert!(
            version(&bad, &[], VERSION_TIMEOUT)
                .unwrap_err()
                .contains('3')
        );
        let slow = script(dir.path(), "slow", "sleep 30");
        let t = Instant::now();
        let e = version(&slow, &[], Duration::from_millis(300)).unwrap_err();
        assert!(e.contains("brak odpowiedzi"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(5));
        let empty = script(dir.path(), "empty", "true");
        assert!(version(&empty, &[], VERSION_TIMEOUT).is_err());
    }

    fn pid_alive(pid: i32) -> bool {
        rustix::process::Pid::from_raw(pid)
            .is_some_and(|p| rustix::process::test_kill_process(p).is_ok())
    }

    /// A wrapper that backgrounds a child inheriting stdout and exits: the
    /// probe must not wait for the child's end of the pipe beyond its deadline,
    /// and the leftover child (same process group) is killed.
    #[test]
    fn version_probe_does_not_hang_on_an_inherited_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("bg.pid");
        let wrapper = script(
            dir.path(),
            "wrapper",
            &format!(
                "sleep 20 &\necho $! > '{}'\necho 'wrapper 1.2.3'\nexit 0",
                pidfile.display()
            ),
        );
        let t = Instant::now();
        // Generous enough for the first exec of a fresh script under a loaded
        // test run (macOS scans it), and still well below the background
        // child's 20 s: finishing before that is what proves there is no hang.
        let v = version(&wrapper, &[], Duration::from_secs(8));
        let took = t.elapsed();
        assert!(took < Duration::from_secs(15), "probe took {took:?}");
        // The wrapper answered and exited 0: its answer counts.
        assert_eq!(v.as_deref(), Ok("wrapper 1.2.3"));
        let pid: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while pid_alive(pid) {
            assert!(Instant::now() < deadline, "background child {pid} survived");
            std::thread::sleep(Duration::from_millis(20));
        }

        // Same, but the wrapper never printed anything: an error, also bounded.
        let silent = script(dir.path(), "silent", "sleep 20 &\nexit 0");
        let t = Instant::now();
        let e = version(&silent, &[], Duration::from_secs(8)).unwrap_err();
        assert!(t.elapsed() < Duration::from_secs(15));
        assert!(e.contains("pusta odpowiedź"), "{e}");
    }
}
