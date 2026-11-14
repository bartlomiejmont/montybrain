//! Short helper processes (`--version` probes, the git snapshot) with one
//! deadline for both process completion and output drainage.
//!
//! The process runs in its own process group with stdin `/dev/null`. stdout
//! and stderr are read on helper threads into capped buffers (the rest is
//! drained and discarded, so the child never blocks on a full pipe). When the
//! deadline passes, or `cancelled` says so, the whole group is SIGKILLed and
//! the call returns what was read so far. Reader threads are never joined: one
//! still blocked on a pipe held open by a process that left the group ends
//! with that process, without holding up the caller.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use super::runner::{Sig, signal_group};

/// How long readers get to see EOF after the group was killed.
const AFTER_KILL: Duration = Duration::from_millis(200);
const POLL: Duration = Duration::from_millis(10);

#[derive(Debug, Default)]
pub struct Output {
    /// `None` when the process could not be reaped.
    pub status: Option<ExitStatus>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// stdout had more than the cap.
    pub stdout_truncated: bool,
    /// The deadline passed (the group was killed).
    pub timed_out: bool,
    /// `cancelled` returned true (the group was killed).
    pub cancelled: bool,
}

impl Output {
    pub fn success(&self) -> bool {
        !self.timed_out && !self.cancelled && self.status.is_some_and(|s| s.success())
    }
}

#[derive(Default)]
struct Capture {
    bytes: Vec<u8>,
    truncated: bool,
}

fn reader(
    mut src: impl Read + Send + 'static,
    cap: usize,
    done: mpsc::Sender<()>,
) -> Arc<Mutex<Capture>> {
    let buf = Arc::new(Mutex::new(Capture::default()));
    let out = Arc::clone(&buf);
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match src.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let mut c = crate::lock(&buf);
                    let room = cap.saturating_sub(c.bytes.len());
                    if n > room {
                        c.truncated = true;
                    }
                    c.bytes.extend_from_slice(&chunk[..n.min(room)]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        drop(done);
    });
    out
}

fn take(c: &Mutex<Capture>) -> (Vec<u8>, bool) {
    let mut c = crate::lock(c);
    (std::mem::take(&mut c.bytes), c.truncated)
}

/// Runs `cmd` (stdio, process group and stdin are set here) under `timeout`.
pub fn run_bounded(
    mut cmd: Command,
    timeout: Duration,
    max_stdout: usize,
    max_stderr: usize,
    cancelled: &dyn Fn() -> bool,
) -> std::io::Result<Output> {
    let deadline = Instant::now() + timeout;
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let pgid = child.id();
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let out = reader(
        child.stdout.take().expect("piped stdout"),
        max_stdout,
        done_tx.clone(),
    );
    let err = reader(
        child.stderr.take().expect("piped stderr"),
        max_stderr,
        done_tx,
    );

    let mut o = Output::default();
    // Both readers hit EOF once every sender is dropped.
    let mut drained = false;
    loop {
        if o.status.is_none() {
            match child.try_wait() {
                Ok(s) => o.status = s,
                Err(_) => break,
            }
        }
        if !drained {
            match done_rx.recv_timeout(POLL) {
                Err(mpsc::RecvTimeoutError::Disconnected) => drained = true,
                Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        } else {
            std::thread::sleep(POLL);
        }
        if o.status.is_some() && drained {
            break;
        }
        if cancelled() {
            o.cancelled = true;
            break;
        }
        if Instant::now() >= deadline {
            o.timed_out = true;
            break;
        }
    }
    if o.timed_out || o.cancelled || o.status.is_none() {
        signal_group(pgid, Sig::Kill);
        // The leader may have moved to another group; SIGKILL it directly too.
        let _ = child.kill();
        if o.status.is_none() {
            // Bounded: the process was just SIGKILLed.
            o.status = child.wait().ok();
        }
        let until = Instant::now() + AFTER_KILL;
        while !drained && Instant::now() < until {
            if let Err(mpsc::RecvTimeoutError::Disconnected) = done_rx.recv_timeout(POLL) {
                drained = true;
            }
        }
    }
    (o.stdout, o.stdout_truncated) = take(&out);
    o.stderr = take(&err).0;
    Ok(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(body: &str) -> Command {
        let mut c = Command::new("/bin/sh");
        c.arg("-c").arg(body);
        c
    }

    #[test]
    fn output_caps_and_status() {
        let o = run_bounded(
            sh("head -c 100000 /dev/zero | tr '\\0' x; echo err >&2; exit 3"),
            Duration::from_secs(10),
            1000,
            100,
            &|| false,
        )
        .unwrap();
        assert_eq!(o.stdout.len(), 1000);
        assert!(o.stdout_truncated && !o.timed_out && !o.success());
        assert_eq!(o.status.unwrap().code(), Some(3));
        assert_eq!(o.stderr, b"err\n");
    }

    #[test]
    fn deadline_and_cancel_kill_the_group() {
        let t = Instant::now();
        let o = run_bounded(
            sh("echo partial; sleep 20"),
            Duration::from_millis(300),
            1000,
            100,
            &|| false,
        )
        .unwrap();
        assert!(o.timed_out && !o.success());
        assert_eq!(o.stdout, b"partial\n");
        assert!(t.elapsed() < Duration::from_secs(3));

        let t = Instant::now();
        let o = run_bounded(sh("sleep 20"), Duration::from_secs(30), 10, 10, &|| {
            t.elapsed() > Duration::from_millis(200)
        })
        .unwrap();
        assert!(o.cancelled && !o.timed_out);
        assert!(t.elapsed() < Duration::from_secs(3));
    }
}
