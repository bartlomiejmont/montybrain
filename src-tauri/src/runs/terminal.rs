//! Terminal mode: the same task as an interactive session in Terminal.app.
//!
//! The prompt goes to a file; a small launcher script (`.command`, 0700) reads
//! it with `"$(cat …)"` and execs the CLI with it as one argument, so no user
//! text is ever part of shell syntax. Only app-generated paths (and the repo
//! path of a project) appear in the script, single-quoted.

use std::ffi::OsString;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use super::command::SCRUBBED_ENV;

/// Launch files older than this are removed before a new launch.
pub const KEEP_FILES_FOR: Duration = Duration::from_secs(60 * 60);

/// `'…'` with embedded single quotes as `'\''`: safe for any string without NUL.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn utf8(p: &Path) -> Result<&str, String> {
    let s = p
        .to_str()
        .ok_or_else(|| format!("Ścieżka nie jest w UTF-8: {}", p.display()))?;
    if s.contains('\0') {
        return Err(format!("Nieprawidłowa ścieżka: {}", p.display()));
    }
    Ok(s)
}

/// The launcher script. `args` (model / effort flags, validated by
/// [`super::agent`]) go between the program and the prompt, each single-quoted.
pub fn script(
    binary: &Path,
    args: &[String],
    prompt_file: &Path,
    cwd: &Path,
    path: &str,
) -> Result<String, String> {
    let mut bin = shell_quote(utf8(binary)?);
    for a in args {
        if a.contains('\0') || a.contains('\n') {
            return Err("Nieprawidłowy argument agenta.".into());
        }
        bin.push(' ');
        bin.push_str(&shell_quote(a));
    }
    let prompt = shell_quote(utf8(prompt_file)?);
    let cwd = shell_quote(utf8(cwd)?);
    let path = shell_quote(path);
    let unset = SCRUBBED_ENV.join(" ");
    Ok(format!(
        "#!/bin/bash\n\
# MontyBrain: sesja agenta otwarta z aplikacji. Plik tymczasowy, można go usunąć.\n\
export PATH={path}\":$PATH\"\n\
unset {unset} MONTYBRAIN_READONLY MORWISKO_READONLY\n\
cd {cwd} 2>/dev/null || cd \"$HOME\" || exit 1\n\
prompt=\"$(cat {prompt})\" || exit 1\n\
rm -f {prompt}\n\
exec {bin} \"$prompt\"\n"
    ))
}

/// Removes our old launch files.
fn cleanup(dir: &Path, keep: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for e in entries.flatten() {
        let p = e.path();
        let ours = p
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".command") || n.ends_with(".prompt.txt"));
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > keep);
        if ours && old {
            let _ = std::fs::remove_file(&p);
        }
    }
}

fn write_file(path: &Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    let _ = std::fs::remove_file(path);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

/// Writes the prompt + launcher into `dir` and opens the launcher with
/// `open_cmd` (`open -a Terminal`). Returns the script path.
#[allow(clippy::too_many_arguments)]
pub fn launch(
    dir: &Path,
    run_id: &str,
    binary: &Path,
    args: &[String],
    prompt: &str,
    cwd: &Path,
    path: &str,
    open_cmd: &[OsString],
) -> Result<PathBuf, String> {
    let io = |e: std::io::Error| format!("Nie udało się przygotować sesji w Terminalu: {e}");
    std::fs::create_dir_all(dir).map_err(io)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(io)?;
    cleanup(dir, KEEP_FILES_FOR);
    let prompt_file = dir.join(format!("{run_id}.prompt.txt"));
    let script_file = dir.join(format!("{run_id}.command"));
    write_file(&prompt_file, prompt.as_bytes(), 0o600).map_err(io)?;
    let body = script(binary, args, &prompt_file, cwd, path)?;
    write_file(&script_file, body.as_bytes(), 0o700).map_err(io)?;
    let (program, args) = open_cmd
        .split_first()
        .ok_or("Brak polecenia otwierającego Terminal")?;
    let out = Command::new(program)
        .args(args)
        .arg(&script_file)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("Nie udało się otworzyć Terminala: {e}"))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&prompt_file);
        return Err(format!(
            "Nie udało się otworzyć Terminala: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(script_file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("abc"), "'abc'");
        assert_eq!(shell_quote("a'b"), r"'a'\''b'");
        assert_eq!(shell_quote("$(x) `y` \"z\""), "'$(x) `y` \"z\"'");
    }

    #[test]
    fn script_content() {
        let s = script(
            Path::new("/Users/x/.local/bin/claude"),
            &[],
            Path::new("/Users/x/Library/Caches/pl.montybrain.desktop/terminal/r1.prompt.txt"),
            Path::new("/Users/x/Projects/it's mine"),
            "/Users/x/.local/bin:/usr/bin:/bin",
        )
        .unwrap();
        assert_eq!(
            s,
            "#!/bin/bash\n\
# MontyBrain: sesja agenta otwarta z aplikacji. Plik tymczasowy, można go usunąć.\n\
export PATH='/Users/x/.local/bin:/usr/bin:/bin'\":$PATH\"\n\
unset ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN CLAUDE_CODE_USE_BEDROCK CLAUDE_CODE_USE_VERTEX \
OPENAI_API_KEY CODEX_API_KEY CLAUDECODE CLAUDE_CODE_ENTRYPOINT CODEX_SANDBOX \
CODEX_SANDBOX_NETWORK_DISABLED MONTYBRAIN_READONLY MORWISKO_READONLY\n\
cd '/Users/x/Projects/it'\\''s mine' 2>/dev/null || cd \"$HOME\" || exit 1\n\
prompt=\"$(cat '/Users/x/Library/Caches/pl.montybrain.desktop/terminal/r1.prompt.txt')\" || exit 1\n\
rm -f '/Users/x/Library/Caches/pl.montybrain.desktop/terminal/r1.prompt.txt'\n\
exec '/Users/x/.local/bin/claude' \"$prompt\"\n"
        );
    }

    #[test]
    fn agent_flags_are_quoted_between_program_and_prompt() {
        let args = super::super::agent::args(
            crate::runs::ProviderId::Codex,
            &super::super::agent::AgentChoice {
                model: Some("gpt-6-sol".into()),
                effort: Some("xhigh".into()),
            },
        );
        let s = script(
            Path::new("/b/codex"),
            &args,
            Path::new("/t/p.txt"),
            Path::new("/r"),
            "/usr/bin",
        )
        .unwrap();
        assert!(
            s.ends_with("exec '/b/codex' '-m' 'gpt-6-sol' '-c' 'model_reasoning_effort=\"xhigh\"' \"$prompt\"\n"),
            "{s}"
        );
        assert!(
            script(
                Path::new("/b/c"),
                &["a\nb".into()],
                Path::new("/t"),
                Path::new("/r"),
                "/p"
            )
            .is_err()
        );
    }

    /// Runs the real launcher with a fake CLI that records its argv, and an
    /// "open" that just executes the script.
    #[test]
    fn launch_passes_hostile_prompt_verbatim_as_one_argument() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("term inal's");
        let bin_dir = tmp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let out = tmp.path().join("argv.txt");
        let fake = bin_dir.join("claude");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/bash\nprintf '%s' \"$#\" > {o}.n\nprintf '%s' \"$1\" > {o}\npwd > {o}.pwd\nenv > {o}.env\n",
                o = shell_quote(out.to_str().unwrap())
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cwd = tmp.path().join("repo $(x)");
        std::fs::create_dir_all(&cwd).unwrap();

        let canary = tmp.path().join("pwned");
        let prompt = format!(
            "/montybrain-zadania x\n\"; touch {c}; echo \"\n$(touch {c}) `touch {c}` '\\''\n--flag\n\n",
            c = canary.display()
        );
        let script_path = launch(
            &dir,
            "r1",
            &fake,
            &[],
            &prompt,
            &cwd,
            "/usr/bin:/bin",
            &[
                "/usr/bin/env".into(),
                "ANTHROPIC_API_KEY=sk-ant".into(),
                "CLAUDECODE=1".into(),
                "MONTYBRAIN_READONLY=1".into(),
                "/bin/bash".into(),
            ],
        )
        .unwrap();

        // Files and modes.
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&script_path), 0o700);
        let body = std::fs::read_to_string(&script_path).unwrap();
        assert!(
            !body.contains("pwned") && !body.contains("--flag"),
            "{body}"
        );
        assert!(
            !dir.join("r1.prompt.txt").exists(),
            "prompt file removed after reading"
        );

        // `$(cat)` drops trailing newlines only; everything else is verbatim, one argument.
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            prompt.trim_end_matches('\n')
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("argv.txt.n")).unwrap(),
            "1"
        );
        let pwd = std::fs::read_to_string(tmp.path().join("argv.txt.pwd")).unwrap();
        assert_eq!(
            Path::new(pwd.trim()).canonicalize().unwrap(),
            cwd.canonicalize().unwrap()
        );
        assert!(!canary.exists() && !cwd.join("pwned").exists());
        let env = std::fs::read_to_string(tmp.path().join("argv.txt.env")).unwrap();
        for var in [
            "ANTHROPIC_API_KEY",
            "CLAUDECODE",
            "MONTYBRAIN_READONLY",
            "MORWISKO_READONLY",
        ] {
            assert!(
                !env.lines().any(|l| l.starts_with(&format!("{var}="))),
                "{var} leaked"
            );
        }
        assert!(
            env.lines().any(|l| l.starts_with("PATH=/usr/bin:/bin:")),
            "{env}"
        );
    }

    #[test]
    fn launch_reports_open_failure_and_cleans_old_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("t");
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("old.command");
        std::fs::write(&old, "x").unwrap();
        let unrelated = dir.join("keep.txt");
        std::fs::write(&unrelated, "x").unwrap();
        let past = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(past)
            .unwrap();

        let e = launch(
            &dir,
            "r2",
            Path::new("/bin/echo"),
            &[],
            "p",
            tmp.path(),
            "/usr/bin",
            &["/usr/bin/false".into()],
        )
        .unwrap_err();
        assert!(e.contains("Terminala"), "{e}");
        assert!(!old.exists() && unrelated.exists());
        assert!(!dir.join("r2.prompt.txt").exists());
    }
}
