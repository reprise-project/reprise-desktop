//! The parent's side of a confined helper program: found beside the running program or installed
//! in /usr/bin, started with an empty environment, fed its request on stdin, its reply read from
//! stdout up to a cap, killed at a deadline that runs from the start or once its caller stops
//! it, and reaped by pid.

use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The most of a helper's reply text, or of its stderr, that reaches a log line or a notice.
pub const TEXT_MAX: usize = 1024;

/// The start of `bytes` as one line of at most [`TEXT_MAX`] bytes: invalid UTF-8 replaced, and
/// control and format characters (newlines, bidi overrides, line and paragraph separators)
/// turned into spaces.
pub fn printable(bytes: &[u8]) -> String {
    let mut line = String::new();
    for c in String::from_utf8_lossy(bytes).chars().map(blank) {
        if line.len() + c.len_utf8() > TEXT_MAX {
            break;
        }
        line.push(c);
    }
    line
}

/// [`printable`] keeping the end of `bytes` rather than the start.
fn printable_end(bytes: &[u8]) -> String {
    let all: Vec<char> = String::from_utf8_lossy(bytes).chars().map(blank).collect();
    let mut len = 0;
    let from = (all.iter().rposition(|c| {
        len += c.len_utf8();
        len > TEXT_MAX
    }))
    .map_or(0, |i| i + 1);
    all[from..].iter().collect()
}

/// A space for `c` when it is a control (Cc), format (Cf), line (Zl) or paragraph (Zp)
/// separator character, else `c`.
fn blank(c: char) -> char {
    let format = matches!(
        c,
        '\u{ad}'
            | '\u{600}'..='\u{605}'
            | '\u{61c}'
            | '\u{6dd}'
            | '\u{70f}'
            | '\u{890}'..='\u{891}'
            | '\u{8e2}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{110bd}'
            | '\u{110cd}'
            | '\u{13430}'..='\u{1343f}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0001}'
            | '\u{e0020}'..='\u{e007f}'
    );
    if c.is_control() || format { ' ' } else { c }
}

/// Helper `name` beside the running program's executable, else in /usr/bin; the error names each
/// path tried and why it was not there.
pub fn find(name: &str) -> Result<PathBuf, String> {
    let beside = std::fs::read_link("/proc/self/exe")
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(name)));
    let mut tried = Vec::new();
    for path in beside.into_iter().chain([Path::new("/usr/bin").join(name)]) {
        match std::fs::metadata(&path) {
            Ok(_) => return Ok(path),
            Err(e) => tried.push(format!("{}: {e}", path.display())),
        }
    }
    Err(format!("no {name} ({})", tried.join("; ")))
}

/// `program` with no environment but LD_LIBRARY_PATH (for build-tree runs), so nothing like
/// GLIBC_TUNABLES reaches the confined process.
pub fn command(program: &Path) -> Command {
    let mut c = Command::new(program);
    c.env_clear();
    if let Some(dirs) = std::env::var_os("LD_LIBRARY_PATH") {
        c.env("LD_LIBRARY_PATH", dirs);
    }
    c
}

/// How much of a helper's stdout [`run`] reads.
#[derive(Clone, Copy, Debug)]
pub struct ReplyCap {
    /// The most bytes read; a helper writing more is killed.
    pub max: usize,
    /// The bytes a reply's length can be told from.
    pub head: usize,
    /// The reply's whole length from its first `head` bytes, None when they do not tell; the
    /// buffer is sized from it once, bounded by `max`.
    pub len_of: fn(&[u8]) -> Option<usize>,
}

impl ReplyCap {
    /// At most `max` bytes, read into a buffer that grows as they come.
    pub fn growing(max: usize) -> ReplyCap {
        ReplyCap {
            max,
            head: 0,
            len_of: |_| None,
        }
    }
}

/// How a [`run`] helper ended.
#[derive(Debug)]
pub struct Ran {
    pub status: io::Result<ExitStatus>,
    /// Its stdout, at most the cap's `max` and one byte more.
    pub reply: Vec<u8>,
    /// It wrote past the cap and was killed.
    pub past_cap: bool,
    /// It ran past the deadline and was killed.
    pub late: bool,
    /// Its caller stopped it ([`run_until`]) and it was killed.
    pub stopped: bool,
    /// The last [`TEXT_MAX`] bytes of its stderr, [`printable`].
    pub said: String,
}

/// Runs `c` as a helper: `input` to its stdin, its stdout read under `cap`. A helper still
/// running `deadline` after it started, or writing past `cap`, is killed. It dies with the
/// calling thread, and starts with every fd above 2 closed and no signal blocked. The error is
/// the spawn's.
pub fn run(c: Command, input: impl Read, cap: ReplyCap, deadline: Duration) -> io::Result<Ran> {
    run_until(c, input, cap, deadline, &|| false)
}

/// How often [`run_until`] asks whether its helper is still wanted.
const STOP_POLL: Duration = Duration::from_millis(50);

/// [`run`], also killing the helper once `stop` returns true.
pub fn run_until(
    mut c: Command,
    mut input: impl Read,
    cap: ReplyCap,
    deadline: Duration,
    stop: &(dyn Fn() -> bool + Sync),
) -> io::Result<Ran> {
    c.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: the closure runs between fork and exec; sigemptyset, pthread_sigmask, signal,
    // prctl and close_range are async-signal-safe and nothing allocates.
    unsafe {
        c.pre_exec(|| {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::pthread_sigmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            // Close-on-exec rather than closed: std reports a failed exec on a descriptor above 2.
            libc::close_range(
                3,
                libc::c_uint::MAX,
                libc::CLOSE_RANGE_CLOEXEC as libc::c_int,
            );
            Ok(())
        });
    }
    let mut child = c.spawn()?;
    let pid = child.id() as libc::pid_t;
    let (stdin, mut stdout, stderr) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take());
    let (done, finished) = mpsc::channel::<()>();
    let (reply, past_cap, said, (late, stopped)) = std::thread::scope(|scope| {
        let watch = std::thread::Builder::new()
            .name("helper-watch".into())
            .spawn_scoped(scope, move || {
                let end = Instant::now() + deadline;
                let ended = loop {
                    let left = end.saturating_duration_since(Instant::now());
                    if finished.recv_timeout(left.min(STOP_POLL))
                        != Err(mpsc::RecvTimeoutError::Timeout)
                    {
                        break (false, false);
                    }
                    let ended = (Instant::now() >= end, stop());
                    if ended != (false, false) {
                        break ended;
                    }
                };
                if ended != (false, false) {
                    // SAFETY: a plain signal; the child is reaped only after this thread is
                    // joined, so `pid` is still the child's.
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                }
                ended
            });
        if watch.is_err() {
            // SAFETY: as above, the child is not reaped yet.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        let said = stderr.and_then(|err| {
            std::thread::Builder::new()
                .name("helper-stderr".into())
                .spawn_scoped(scope, move || tail_of(err, TEXT_MAX))
                .ok()
        });
        // A helper reads all of its input before it writes, so neither pipe can fill up the other.
        if let Some(mut stdin) = stdin {
            let _ = io::copy(&mut input, &mut stdin);
        }
        let reply = stdout
            .as_mut()
            .map_or_else(Vec::new, |out| read_reply(out, cap));
        let past_cap = reply.len() > cap.max;
        if past_cap {
            // SAFETY: as above, the child is not reaped yet.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        // Closed only after the kill, so a helper past the cap dies of SIGKILL, never SIGPIPE.
        drop(stdout);
        exited(pid);
        drop(done);
        let ended = watch.map_or((false, false), |w| w.join().unwrap_or((false, false)));
        let said = said.and_then(|t| t.join().ok()).unwrap_or_default();
        (reply, past_cap, said, ended)
    });
    Ok(Ran {
        status: child.wait(),
        reply,
        past_cap,
        late,
        stopped,
        said,
    })
}

/// At most `cap.max` bytes of `out` and one more.
fn read_reply(out: impl Read, cap: ReplyCap) -> Vec<u8> {
    let mut out = out.take(cap.max as u64 + 1);
    let mut reply = Vec::new();
    let _ = (&mut out).take(cap.head as u64).read_to_end(&mut reply);
    if let Some(len) = (cap.len_of)(&reply) {
        let len = len.min(cap.max + 1);
        let _ = reply.try_reserve_exact(len.saturating_sub(reply.len()));
    }
    let _ = out.read_to_end(&mut reply);
    reply
}

/// Blocks until child `pid` has exited, leaving it to be reaped.
fn exited(pid: libc::pid_t) {
    loop {
        // SAFETY: info is a live siginfo_t; WNOWAIT leaves the child unreaped.
        let waited = unsafe {
            let mut info: libc::siginfo_t = std::mem::zeroed();
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if waited == 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return;
        }
    }
}

/// The last `keep` bytes `r` gives before it ends, [`printable`] from the end.
fn tail_of(mut r: impl Read, keep: usize) -> String {
    let mut tail = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        match r.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                tail.extend_from_slice(&chunk[..n]);
                if tail.len() > keep {
                    tail.drain(..tail.len() - keep);
                }
            }
        }
    }
    printable_end(&tail)
}

/// `signal`'s name, as a failure's log line gives it.
pub fn signal_name(signal: i32) -> String {
    let name = match signal {
        libc::SIGSYS => "SIGSYS",
        libc::SIGXCPU => "SIGXCPU",
        libc::SIGKILL => "SIGKILL",
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGABRT => "SIGABRT",
        libc::SIGBUS => "SIGBUS",
        libc::SIGILL => "SIGILL",
        _ => return format!("signal {signal}"),
    };
    name.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::time::Instant;

    fn sh(script: &str) -> Command {
        let mut c = command(Path::new("/bin/sh"));
        c.args(["-c", script]);
        c
    }

    const LONG: Duration = Duration::from_secs(30);

    fn cap(max: usize) -> ReplyCap {
        ReplyCap::growing(max)
    }

    #[test]
    fn a_helper_gets_its_input_and_gives_its_reply_and_stderr() {
        let input = b"ab".chain(&b"c"[..]);
        let ran = run(sh("cat; echo said >&2"), input, cap(16), LONG).unwrap();
        assert!(ran.status.unwrap().success());
        assert_eq!(ran.reply, b"abc");
        assert_eq!(ran.said, "said ");
        assert!(!ran.late && !ran.past_cap);
    }

    #[test]
    fn a_helper_past_its_deadline_is_killed() {
        let start = Instant::now();
        let ran = run(
            sh("exec sleep 30"),
            io::empty(),
            cap(16),
            Duration::from_millis(200),
        );
        let ran = ran.unwrap();
        assert!(ran.late);
        assert_eq!(ran.status.unwrap().signal(), Some(libc::SIGKILL));
        assert!(start.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_stopped_helper_is_killed_before_its_deadline() {
        let start = Instant::now();
        let stop = || start.elapsed() > Duration::from_millis(200);
        let ran = run_until(sh("exec sleep 30"), io::empty(), cap(16), LONG, &stop).unwrap();
        assert!(ran.stopped && !ran.late);
        assert_eq!(ran.status.unwrap().signal(), Some(libc::SIGKILL));
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_helper_that_closes_its_stdout_is_still_killed_at_the_deadline() {
        for script in [
            "exec >&- 2>&-; sleep 30",
            "cat >/dev/null; exec >&-; kill -STOP $$",
            "cat >/dev/null; printf 'reply'; exec >&-; kill -STOP $$",
        ] {
            let start = Instant::now();
            let ran = run(sh(script), io::empty(), cap(16), Duration::from_millis(200)).unwrap();
            assert!(ran.late, "{script}: the wait is bounded by the deadline");
            assert!(start.elapsed() < Duration::from_secs(10), "{script}");
        }
    }

    #[test]
    fn a_reply_past_the_cap_is_cut_off_and_its_helper_killed() {
        let ran = run(sh("yes"), io::empty(), cap(1000), LONG).unwrap();
        assert!(ran.past_cap);
        assert_eq!(ran.reply.len(), 1001);
        assert_eq!(ran.status.unwrap().signal(), Some(libc::SIGKILL));
        let exactly = run(sh("head -c 1000 /dev/zero"), io::empty(), cap(1000), LONG).unwrap();
        assert!(!exactly.past_cap);
        assert_eq!(exactly.reply.len(), 1000);
    }

    #[test]
    fn a_reply_that_states_its_length_is_read_into_a_buffer_of_that_size() {
        let sized = ReplyCap {
            max: 1 << 20,
            head: 2,
            len_of: |head| Some(2 + usize::from(u16::from_le_bytes(head.try_into().ok()?))),
        };
        let ran = run(
            sh(r"printf '\020\047'; head -c 10000 /dev/zero"),
            io::empty(),
            sized,
            LONG,
        );
        let reply = ran.unwrap().reply;
        assert_eq!(reply.len(), 10002);
        assert_eq!(reply.capacity(), 10002, "no growth past the stated length");
        let liar = run(
            sh(r"printf '\002\000'; head -c 100 /dev/zero"),
            io::empty(),
            sized,
            LONG,
        );
        assert_eq!(
            liar.unwrap().reply.len(),
            102,
            "a longer reply is still read to the cap"
        );
    }

    #[test]
    fn a_helper_inherits_no_environment() {
        assert!(std::env::var_os("HOME").is_some());
        assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
        let script =
            r#"printf '%s %s\n' "${HOME-unset}" "${CARGO_MANIFEST_DIR-unset}"; env | wc -c"#;
        let ran = run(sh(script), io::empty(), cap(4096), LONG).unwrap();
        let said = String::from_utf8(ran.reply).unwrap();
        let (unset, size) = said.split_once('\n').unwrap();
        assert_eq!(unset, "unset unset");
        // What the shell sets itself (PWD, SHLVL, _), and LD_LIBRARY_PATH.
        let ld = std::env::var_os("LD_LIBRARY_PATH").map_or(0, |v| v.len() + 17);
        let cwd = std::env::current_dir().unwrap().as_os_str().len();
        let size: usize = size.trim().parse().unwrap();
        assert!(size <= ld + cwd + 48, "{size} bytes of environment");
    }

    #[test]
    fn a_missing_helper_names_each_path_tried() {
        let why = find("reprise-no-such-helper").unwrap_err();
        assert!(why.starts_with("no reprise-no-such-helper ("), "{why}");
        assert!(
            why.contains("/usr/bin/reprise-no-such-helper: No such file or directory"),
            "{why}"
        );
        assert_eq!(why.matches("reprise-no-such-helper:").count(), 2, "{why}");
    }

    #[test]
    fn the_end_of_a_long_stderr_is_kept_on_one_line() {
        let said: Vec<u8> = (0..10_000u32).map(|i| b'a' + (i % 26) as u8).collect();
        let tail = tail_of(said.as_slice(), TEXT_MAX);
        assert_eq!(tail.as_bytes(), &said[said.len() - TEXT_MAX..]);
        let lines = &b"thread panicked\nmemory allocation failed\n"[..];
        assert_eq!(
            tail_of(lines, TEXT_MAX),
            "thread panicked memory allocation failed "
        );
        let tail = tail_of(&[0xff; 2000][..], TEXT_MAX);
        assert_eq!(
            tail,
            "\u{fffd}".repeat(TEXT_MAX / 3),
            "cut after the replacement"
        );
    }

    #[test]
    fn printable_text_is_one_line_of_at_most_1_kib() {
        assert_eq!(
            printable(b"bad\nline\x1b[31m\0\xff"),
            "bad line [31m \u{fffd}"
        );
        let tricks = "a\u{202e}b\u{2028}c\u{2029}d\u{200b}e\u{feff}f\u{2066}g";
        assert_eq!(printable(tricks.as_bytes()), "a b c d e f g");
        let invalid = [0xff; 2000];
        let line = printable(&invalid);
        assert_eq!(
            line,
            "\u{fffd}".repeat(TEXT_MAX / 3),
            "cut after the replacement"
        );
        assert_eq!(printable(&[b'x'; 5000]).len(), TEXT_MAX);
    }
}
