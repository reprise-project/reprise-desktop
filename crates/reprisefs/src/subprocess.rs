//! A helper program run within a deadline, in a process group of its own that is killed when the
//! program ends or runs out of time, and that dies with the thread that ran it.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitStatus};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(20);
/// How long the pipes get to reach their end once the program and its group are gone, so output
/// written just before the deadline is still read.
const DRAIN: Duration = Duration::from_millis(250);

/// How a [`run`] program ended, with what it wrote to the pipes the caller asked for.
#[derive(Debug)]
pub struct Ran {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug)]
pub enum Error {
    Spawn(std::io::Error),
    /// It, or a process it left holding its pipes, ran past the deadline; its group is killed.
    Timeout,
    Wait(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Spawn(e) | Error::Wait(e) => e.fmt(f),
            Error::Timeout => f.write_str("it did not end in time"),
        }
    }
}

/// Run `cmd` for at most `within`, reading the stdout and stderr it pipes. Whatever the program
/// leaves running in its group is killed once it ends. The program's death signal follows the
/// calling thread, which waits here until it ends.
pub fn run(cmd: &mut Command, within: Duration) -> Result<Ran, Error> {
    let end = Instant::now() + within;
    let parent = std::process::id() as libc::pid_t;
    cmd.process_group(0);
    // SAFETY: prctl and getppid are async-signal-safe, and nothing allocates between fork and
    // exec. A parent gone before prctl leaves the child reparented, which getppid catches.
    unsafe {
        cmd.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(Error::Spawn)?;
    let group = child.id() as libc::pid_t;
    // Each pipe drains on a thread of its own, so a full one cannot hold the program up.
    let stdout = read(child.stdout.take());
    let stderr = read(child.stderr.take());
    let mut ours = true;
    loop {
        match ended(group) {
            Some(false) => {}
            Some(true) => break,
            None => {
                ours = false;
                break;
            }
        }
        if Instant::now() >= end {
            kill_group(group);
            // A program stuck on a dead mount may outlive SIGKILL for a while.
            std::thread::spawn(move || child.wait());
            return Err(Error::Timeout);
        }
        std::thread::sleep(POLL);
    }
    // Unreaped, the program's pid still names its group, so what it left goes with it. Once
    // someone else reaped it, the group may name another's processes.
    if ours {
        kill_group(group);
    }
    let status = child.wait().map_err(Error::Wait)?;
    let left = |rx: mpsc::Receiver<String>| {
        rx.recv_timeout(end.saturating_duration_since(Instant::now()).max(DRAIN))
            .map_err(|_| Error::Timeout)
    };
    Ok(Ran {
        status,
        stdout: left(stdout)?,
        stderr: left(stderr)?,
    })
}

/// `pipe` read to its end on a thread; empty text at once when the caller did not pipe it.
fn read(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    match pipe {
        None => {
            let _ = tx.send(String::new());
        }
        Some(mut pipe) => {
            std::thread::spawn(move || {
                let mut text = String::new();
                let _ = pipe.read_to_string(&mut text);
                let _ = tx.send(text);
            });
        }
    }
    rx
}

/// Whether child `pid` has ended, left unreaped; None when it cannot be waited on (reaped
/// elsewhere), for the wait to report.
fn ended(pid: libc::pid_t) -> Option<bool> {
    // SAFETY: an all-zero siginfo_t is valid, and waitid only writes into it.
    unsafe {
        let mut info: libc::siginfo_t = std::mem::zeroed();
        let flags = libc::WEXITED | libc::WNOHANG | libc::WNOWAIT;
        if libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, flags) != 0 {
            return None;
        }
        Some(info.si_pid() != 0)
    }
}

fn kill_group(group: libc::pid_t) {
    // SAFETY: a plain system call; the group may already be gone.
    unsafe {
        libc::kill(-group, libc::SIGKILL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::path::Path;
    use std::process::Stdio;

    fn sh(script: &str) -> Command {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }

    /// Whether the process whose pid `dir`'s `left` holds is dead within 5 s. A zombie counts:
    /// in a container whose init reaps nothing, the killed orphan stays one.
    fn gone(dir: &Path) -> bool {
        let pid: libc::pid_t = std::fs::read_to_string(dir.join("left"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let started = Instant::now();
        let alive = || {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            let state = stat
                .rsplit_once(')')
                .and_then(|(_, rest)| rest.trim_start().chars().next());
            !matches!(state, None | Some('Z' | 'X'))
        };
        while alive() {
            if started.elapsed() > Duration::from_secs(5) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }

    #[test]
    fn a_program_that_hangs_is_killed_with_what_it_started() {
        let dir = tempfile::tempdir().unwrap();
        let script = format!("sleep 60 & echo $! >{}/left; wait", dir.path().display());
        let started = Instant::now();
        let ran = run(&mut sh(&script), Duration::from_millis(300));
        assert!(matches!(ran, Err(Error::Timeout)), "{ran:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(gone(dir.path()), "its child is killed too");
    }

    #[test]
    fn a_program_that_ends_is_read_and_what_it_left_is_killed() {
        let dir = tempfile::tempdir().unwrap();
        let script = format!(
            "echo out; echo err >&2; sleep 60 & echo $! >{}/left",
            dir.path().display()
        );
        let started = Instant::now();
        let ran = run(&mut sh(&script), Duration::from_secs(30)).unwrap();
        assert!(ran.status.success());
        assert_eq!(
            (ran.stdout.as_str(), ran.stderr.as_str()),
            ("out\n", "err\n")
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the readers do not wait on the pipes the leftover holds"
        );
        assert!(gone(dir.path()), "the leftover is killed");
    }

    #[test]
    fn a_crash_is_its_status_and_a_missing_program_a_spawn_error() {
        let ran = run(
            &mut sh("echo partial; kill -SEGV $$"),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(ran.status.signal(), Some(libc::SIGSEGV));
        let mut missing = Command::new("/nonexistent/program");
        let ran = run(&mut missing, Duration::from_secs(5));
        assert!(matches!(ran, Err(Error::Spawn(_))), "{ran:?}");
    }
}
