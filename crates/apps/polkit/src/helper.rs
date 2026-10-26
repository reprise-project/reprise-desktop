//! polkit's agent helper (`polkit-agent-helper-1`) and its line protocol. It runs PAM and tells
//! polkitd the result; we feed it the cookie, user and password, one line each, and read its
//! prompts back. Since polkit 125 it is socket-activated at `/run/polkit/agent-helper.socket`;
//! older installs have it setuid root, spawned by the agent. After the header lines both speak
//! the same prompts.
//!
//! A conversation starts when the dialog opens. With an fprintd finger enrolled it reaches the
//! helper at once, so a `pam_fprintd` PAM stack can take the finger (a match ends it with
//! SUCCESS); the password prompt waits for the dialog's OK. Without a finger it reaches the
//! helper only on OK, with the password: hanging up at the password prompt (Cancel) fails PAM
//! and pam_faillock counts it against the account.

use reprisebus::mutex::lock;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zeroize::Zeroizing;

/// One line from the helper, its `g_strescape` undone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// A prompt whose answer must not be shown: the password.
    PromptEchoOff(String),
    /// A prompt whose answer may be shown: the user name.
    PromptEchoOn(String),
    Error(String),
    Info(String),
    Success,
    Failure,
}

/// Undo `g_strescape`: C escapes and octal `\ooo` bytes (how non-ASCII arrives, as UTF-8
/// bytes). An octal escape past 0o377 becomes `?`.
pub fn unescape(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let Some(&next) = bytes.get(i + 1) else {
            out.push(b'\\');
            break;
        };
        i += 2;
        match next {
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'b' => out.push(0x08),
            b'f' => out.push(0x0c),
            b'v' => out.push(0x0b),
            b'"' => out.push(b'"'),
            b'\\' => out.push(b'\\'),
            b'0'..=b'7' => {
                let mut v = u32::from(next - b'0');
                let mut n = 1;
                while n < 3 && i < bytes.len() && (b'0'..=b'7').contains(&bytes[i]) {
                    v = v * 8 + u32::from(bytes[i] - b'0');
                    i += 1;
                    n += 1;
                }
                out.push(u8::try_from(v).unwrap_or(b'?'));
            }
            other => {
                out.push(b'\\');
                out.push(other);
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One helper line, newline and escapes removed. `None` for a line the
/// protocol does not have, which ends the session as a failure.
pub fn parse_line(raw: &str) -> Option<Line> {
    let line = raw.strip_suffix('\n').unwrap_or(raw);
    let prompt = |prefix: &str| line.strip_prefix(prefix).map(unescape);
    if let Some(t) = prompt("PAM_PROMPT_ECHO_OFF ") {
        Some(Line::PromptEchoOff(t))
    } else if let Some(t) = prompt("PAM_PROMPT_ECHO_ON ") {
        Some(Line::PromptEchoOn(t))
    } else if let Some(t) = prompt("PAM_ERROR_MSG ") {
        Some(Line::Error(t))
    } else if let Some(t) = prompt("PAM_TEXT_INFO ") {
        Some(Line::Info(t))
    } else if line.trim() == "SUCCESS" {
        Some(Line::Success)
    } else if line.trim() == "FAILURE" {
        Some(Line::Failure)
    } else {
        None
    }
}

const SOCKET: &str = "/run/polkit/agent-helper.socket";
const SETUID_HELPER: &str = "/usr/lib/polkit-1/polkit-agent-helper-1";
/// How long a helper may stay silent (socket) or run (spawned) while we wait on it, so a wedged
/// helper cannot hold the thread. Time waiting for the user's typing or finger does not count.
const HELPER_TIMEOUT: Duration = Duration::from_secs(120);

/// How one attempt ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The helper has already told polkitd.
    Success,
    Failed,
    /// The helper could not be reached or spawned at all, or hung up
    /// before it had a password or said FAILURE, so no attempt was
    /// judged: this consumes no retry.
    Unreachable,
}

/// What one attempt tells the dialog, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Report {
    Info(String),
    Error(String),
    /// The helper asks a hidden question (the password, then any second factor), whose text
    /// this is, and waits for [`Session::answer`]. Text before the first belonged to an earlier
    /// step of the PAM stack (the fingerprint reader's, say).
    Prompt(String),
    Done(Verdict),
}

/// Run one PAM conversation over an open transport whose header lines
/// (user, cookie) have been written. Each hidden prompt reports
/// [`Report::Prompt`] and takes a fresh answer from `password`, which
/// blocks until the dialog has one. The user name answers a shown prompt.
/// `password` returning None, or a line outside the protocol, ends the
/// conversation. Each answer is wiped once written. `deadline` hears
/// whether the next read waits on the helper (true) or on the user's
/// finger (false): an info line before any hidden prompt is the
/// fingerprint reader's.
pub fn converse<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    user: &str,
    password: &mut dyn FnMut() -> Option<Zeroizing<String>>,
    deadline: &mut dyn FnMut(bool),
    report: &mut dyn FnMut(Report),
) {
    let mut line = String::new();
    let mut answered = false;
    let mut timed = true;
    let verdict = loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break None,
            Ok(_) => {}
        }
        let parsed = parse_line(&line);
        let on = !(matches!(parsed, Some(Line::Info(_))) && !answered);
        if on != timed {
            timed = on;
            deadline(on);
        }
        match parsed {
            Some(Line::PromptEchoOff(t)) => {
                report(Report::Prompt(t.trim_end().to_string()));
                let Some(p) = password() else {
                    break None;
                };
                answered = true;
                if writer.write_all(p.as_bytes()).is_err() || writer.write_all(b"\n").is_err() {
                    break None;
                }
                let _ = writer.flush();
            }
            Some(Line::PromptEchoOn(_)) => {
                if writeln!(writer, "{user}").is_err() {
                    break None;
                }
                let _ = writer.flush();
            }
            Some(Line::Error(t)) => report(Report::Error(t.trim_end().to_string())),
            Some(Line::Info(t)) => report(Report::Info(t.trim_end().to_string())),
            Some(Line::Success) => break Some(Verdict::Success),
            Some(Line::Failure) | None => break Some(Verdict::Failed),
        }
    };
    // A hang-up after the password went out judged it, as far as we can tell.
    let verdict = verdict.unwrap_or(if answered {
        Verdict::Failed
    } else {
        Verdict::Unreachable
    });
    report(Report::Done(verdict));
}

/// Ends a conversation from outside its thread, even one blocked reading a helper waiting on
/// the fingerprint reader: the thread registers how (shut the socket, kill the child) once it
/// has a transport; a stop that came first runs it then.
#[derive(Default)]
struct Stop(Mutex<StopState>);

#[derive(Default)]
enum StopState {
    #[default]
    Armed,
    Hooked(Box<dyn FnOnce() + Send>),
    Fired,
}

impl Stop {
    fn on_stop(&self, f: Box<dyn FnOnce() + Send>) {
        let mut s = lock(&self.0);
        if matches!(*s, StopState::Fired) {
            drop(s);
            f();
        } else {
            *s = StopState::Hooked(f);
        }
    }

    fn stopped(&self) -> bool {
        matches!(*lock(&self.0), StopState::Fired)
    }

    fn fire(&self) {
        let prev = std::mem::replace(&mut *lock(&self.0), StopState::Fired);
        if let StopState::Hooked(f) = prev {
            f();
        }
    }
}

/// A conversation running on its own thread. Dropping it ends the
/// conversation: a password still waiting in it is wiped, and the
/// helper is hung up on.
pub struct Session {
    password: Sender<Zeroizing<String>>,
    stop: Arc<Stop>,
}

impl Session {
    /// Hand over the answer for the hidden prompt that came or the one to
    /// come. False when the conversation is already over.
    pub fn answer(&self, password: Zeroizing<String>) -> bool {
        self.password.send(password).is_ok()
    }
}

#[cfg(test)]
impl Session {
    /// A session with no helper behind it, and the end its passwords
    /// arrive at.
    pub fn detached() -> (Session, Receiver<Zeroizing<String>>) {
        let (password, rx) = mpsc::channel();
        let stop = Arc::new(Stop::default());
        (Session { password, stop }, rx)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.fire();
    }
}

/// A connected helper to read lines from and write ours to, header lines sent.
enum Transport {
    Socket(UnixStream),
    Child(std::process::Child),
}

/// Reach the helper: the systemd socket (polkit 125+), else the setuid binary. Writes the
/// header lines (the socket wants the user first; the child takes it as an argument). Paths are
/// parameters so tests can stand in.
fn connect_at(
    socket: &Path,
    helper: &Path,
    user: &str,
    cookie: &str,
) -> std::io::Result<Transport> {
    match UnixStream::connect(socket) {
        Ok(mut s) => {
            s.set_read_timeout(Some(HELPER_TIMEOUT))?;
            write!(s, "{user}\n{cookie}\n")?;
            Ok(Transport::Socket(s))
        }
        // Only a missing socket means an older polkit: from 125 on the
        // binary is not setuid, so spawning it would only fail PAM.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::info!(
                "no {} ({e}); spawning {}",
                socket.display(),
                helper.display()
            );
            let mut child = std::process::Command::new(helper)
                .arg(user)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()?;
            let stdin = child.stdin.as_mut().ok_or(std::io::ErrorKind::BrokenPipe)?;
            writeln!(stdin, "{cookie}")?;
            Ok(Transport::Child(child))
        }
        Err(e) => {
            log::warn!("cannot use {} ({e})", socket.display());
            Err(e)
        }
    }
}

/// What the spawned helper's watchdog hears.
enum Watch {
    /// We wait on the user, not the helper: the deadline is off.
    Pause,
    /// We wait on the helper again: a fresh deadline.
    Resume,
    Kill,
}

/// [`converse`] with a spawned helper, which is killed if it keeps us
/// waiting past `timeout` (a pipe has no read timeout) or when `stop`
/// fires.
fn converse_child(
    mut child: std::process::Child,
    user: &str,
    password: &mut dyn FnMut() -> Option<Zeroizing<String>>,
    timeout: Duration,
    stop: &Stop,
    report: &mut dyn FnMut(Report),
) {
    let (Some(stdout), Some(stdin)) = (child.stdout.take(), child.stdin.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        report(Report::Done(Verdict::Unreachable));
        return;
    };
    let (watch_tx, watch_rx) = mpsc::channel::<Watch>();
    let kill = watch_tx.clone();
    stop.on_stop(Box::new(move || {
        let _ = kill.send(Watch::Kill);
    }));
    let child = &mut child;
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let mut paused = false;
            // Killing closes the helper's stdout, which ends the read in
            // `converse`; every sender gone means the conversation is over.
            loop {
                let next = if paused {
                    watch_rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
                } else {
                    watch_rx.recv_timeout(timeout)
                };
                match next {
                    Ok(Watch::Pause) => paused = true,
                    Ok(Watch::Resume) => paused = false,
                    Ok(Watch::Kill) | Err(RecvTimeoutError::Timeout) => {
                        let _ = child.kill();
                        break;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            let _ = child.wait();
        });
        let mut paused_password = || {
            let _ = watch_tx.send(Watch::Pause);
            let p = password();
            let _ = watch_tx.send(Watch::Resume);
            p
        };
        let mut deadline = |on: bool| {
            let _ = watch_tx.send(if on { Watch::Resume } else { Watch::Pause });
        };
        converse(
            BufReader::new(stdout),
            stdin,
            user,
            &mut paused_password,
            &mut deadline,
            report,
        );
        drop(watch_tx);
        // The stop hook holds the last sender; drop it too, or the
        // watchdog waits on it.
        stop.on_stop(Box::new(|| {}));
    });
}

/// Start a conversation for `user` on its own thread: connect, converse,
/// send every `Report` down `tx` and wake the app after each. None when
/// the thread could not start.
pub fn authenticate(
    user: String,
    cookie: String,
    tx: Sender<Report>,
    waker: repriseapp::Waker,
) -> Option<Session> {
    authenticate_at(
        PathBuf::from(SOCKET),
        PathBuf::from(SETUID_HELPER),
        reprisebus::fprint::has_finger,
        user,
        cookie,
        tx,
        Box::new(move || waker.wake()),
    )
}

/// [`authenticate`] with the socket, the binary and the finger check
/// given, so a test can stand in for all three.
#[allow(clippy::too_many_arguments)]
fn authenticate_at(
    socket: PathBuf,
    helper: PathBuf,
    finger: fn(&str) -> bool,
    user: String,
    cookie: String,
    tx: Sender<Report>,
    wake: Box<dyn Fn() + Send>,
) -> Option<Session> {
    let (password_tx, password_rx) = mpsc::channel::<Zeroizing<String>>();
    let stop = Arc::new(Stop::default());
    let thread_stop = stop.clone();
    let spawned = std::thread::Builder::new()
        .name("reprise-polkit-helper".into())
        .spawn(move || {
            let mut report = |r: Report| {
                if tx.send(r).is_ok() {
                    wake();
                }
            };
            let early = finger(&user);
            run(
                &socket,
                &helper,
                early,
                &user,
                &cookie,
                password_rx,
                &thread_stop,
                &mut report,
            );
        });
    match spawned {
        Ok(_) => Some(Session {
            password: password_tx,
            stop,
        }),
        Err(e) => {
            log::error!("cannot start the helper thread: {e}");
            None
        }
    }
}

/// The helper thread's body. `early` reaches the helper before the
/// password is in, for a PAM step (the fingerprint reader) that may come
/// first; otherwise nothing happens until the password is, and a session
/// dropped before then leaves no attempt behind.
#[allow(clippy::too_many_arguments)]
fn run(
    socket: &Path,
    helper: &Path,
    early: bool,
    user: &str,
    cookie: &str,
    password_rx: Receiver<Zeroizing<String>>,
    stop: &Stop,
    report: &mut dyn FnMut(Report),
) {
    let mut first = None;
    if !early {
        match password_rx.recv() {
            Ok(p) => first = Some(p),
            Err(_) => return,
        }
    }
    if stop.stopped() {
        return;
    }
    let mut password = || first.take().or_else(|| password_rx.recv().ok());
    match connect_at(socket, helper, user, cookie) {
        Ok(Transport::Socket(s)) => {
            let (reader, hangup) = match (s.try_clone(), s.try_clone()) {
                (Ok(r), Ok(h)) => (BufReader::new(r), h),
                (Err(e), _) | (_, Err(e)) => {
                    report(Report::Error(e.to_string()));
                    report(Report::Done(Verdict::Unreachable));
                    return;
                }
            };
            // Ends a read blocked on the helper; the helper sees the
            // hang-up on its next write.
            stop.on_stop(Box::new(move || {
                let _ = hangup.shutdown(std::net::Shutdown::Both);
            }));
            let timer = s.try_clone().ok();
            let mut deadline = |on: bool| {
                if let Some(t) = &timer {
                    let _ = t.set_read_timeout(on.then_some(HELPER_TIMEOUT));
                }
            };
            converse(reader, s, user, &mut password, &mut deadline, report);
        }
        Ok(Transport::Child(child)) => {
            converse_child(child, user, &mut password, HELPER_TIMEOUT, stop, report);
        }
        Err(e) => {
            report(Report::Error(format!(
                "Cannot reach the polkit helper: {e}"
            )));
            report(Report::Done(Verdict::Unreachable));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape_undoes_g_strescape() {
        assert_eq!(unescape("plain"), "plain");
        assert_eq!(unescape(r#"a\nb\tc\"d\\e"#), "a\nb\tc\"d\\e");
        // Non-ASCII bytes come as octal escapes of the UTF-8 bytes.
        assert_eq!(unescape(r"caf\303\251"), "café");
        // A trailing lone backslash is kept, not dropped.
        assert_eq!(unescape(r"end\"), "end\\");
        // An octal escape past one byte becomes '?'.
        assert_eq!(unescape(r"a\777b"), "a?b");
    }

    #[test]
    fn parse_line_knows_every_prefix() {
        assert_eq!(
            parse_line("PAM_PROMPT_ECHO_OFF Password: "),
            Some(Line::PromptEchoOff("Password: ".into()))
        );
        assert_eq!(
            parse_line("PAM_PROMPT_ECHO_ON login: "),
            Some(Line::PromptEchoOn("login: ".into()))
        );
        assert_eq!(
            parse_line(r"PAM_ERROR_MSG Authentication failure\n"),
            Some(Line::Error("Authentication failure\n".into()))
        );
        assert_eq!(
            parse_line("PAM_TEXT_INFO hello"),
            Some(Line::Info("hello".into()))
        );
        assert_eq!(parse_line("SUCCESS"), Some(Line::Success));
        assert_eq!(parse_line("FAILURE"), Some(Line::Failure));
        assert_eq!(parse_line("SUCCESS\n"), Some(Line::Success));
        assert_eq!(
            parse_line("PAM_PROMPT_ECHO_OFF"),
            None,
            "a prompt needs its space"
        );
        assert_eq!(parse_line("garbage"), None);
        assert_eq!(parse_line("SUCCESSFUL"), None, "not just a prefix match");
    }

    /// Play the helper on one end of a socket pair: read the two
    /// header lines, prompt, read the answer, say `verdict`.
    fn fake_helper(
        sock: UnixStream,
        verdict: &'static str,
    ) -> std::thread::JoinHandle<Vec<String>> {
        std::thread::spawn(move || {
            let mut r = BufReader::new(sock.try_clone().unwrap());
            let mut w = sock;
            let mut got = Vec::new();
            let mut line = String::new();
            for _ in 0..2 {
                line.clear();
                r.read_line(&mut line).unwrap();
                got.push(line.trim_end().to_string());
            }
            w.write_all(b"PAM_TEXT_INFO hi\n").unwrap();
            w.write_all(b"PAM_PROMPT_ECHO_OFF Password: \n").unwrap();
            line.clear();
            r.read_line(&mut line).unwrap();
            got.push(line.trim_end().to_string());
            w.write_all(b"PAM_PROMPT_ECHO_ON login: \n").unwrap();
            line.clear();
            r.read_line(&mut line).unwrap();
            got.push(line.trim_end().to_string());
            w.write_all(verdict.as_bytes()).unwrap();
            w.write_all(b"\n").unwrap();
            got
        })
    }

    /// A password source that answers `p` and counts the asks.
    fn answers(p: &'static str, asked: &mut u32) -> impl FnMut() -> Option<Zeroizing<String>> {
        move || {
            *asked += 1;
            Some(Zeroizing::new(p.to_string()))
        }
    }

    #[test]
    fn converse_answers_the_prompts_and_reports_the_verdict() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let helper = fake_helper(theirs, "SUCCESS");
        let mut reports = Vec::new();
        let reader = BufReader::new(ours.try_clone().unwrap());
        let mut w = ours;
        w.write_all(b"alice\ncookie-1\n").unwrap();
        let mut asked = 0;
        converse(
            reader,
            w,
            "alice",
            &mut answers("hunter2", &mut asked),
            &mut |_| {},
            &mut |r| reports.push(r),
        );
        assert_eq!(
            helper.join().unwrap(),
            ["alice", "cookie-1", "hunter2", "alice"]
        );
        assert_eq!(
            reports,
            [
                Report::Info("hi".into()),
                Report::Prompt("Password:".into()),
                Report::Done(Verdict::Success)
            ]
        );
        assert_eq!(asked, 1);
    }

    #[test]
    fn converse_reports_failure_and_eof_after_the_password_as_failed() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let helper = fake_helper(theirs, "FAILURE");
        let mut reports = Vec::new();
        let reader = BufReader::new(ours.try_clone().unwrap());
        let mut w = ours;
        w.write_all(b"bob\nc\n").unwrap();
        let mut asked = 0;
        converse(
            reader,
            w,
            "bob",
            &mut answers("x", &mut asked),
            &mut |_| {},
            &mut |r| reports.push(r),
        );
        helper.join().unwrap();
        assert_eq!(reports.last(), Some(&Report::Done(Verdict::Failed)));

        // A helper that hangs up after the password is a failure too.
        let (ours, theirs) = UnixStream::pair().unwrap();
        let helper = std::thread::spawn(move || {
            let mut r = BufReader::new(theirs.try_clone().unwrap());
            let mut w = theirs;
            w.write_all(b"PAM_PROMPT_ECHO_OFF Password: \n").unwrap();
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
        });
        let mut reports = Vec::new();
        let reader = BufReader::new(ours.try_clone().unwrap());
        converse(
            reader,
            ours,
            "bob",
            &mut answers("x", &mut asked),
            &mut |_| {},
            &mut |r| reports.push(r),
        );
        helper.join().unwrap();
        assert_eq!(
            reports,
            [
                Report::Prompt("Password:".into()),
                Report::Done(Verdict::Failed)
            ]
        );
    }

    /// With `pam_fprintd` ahead of the password, a finger ends the
    /// conversation before any prompt: no password is asked for.
    #[test]
    fn a_finger_succeeds_without_a_password() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let helper = std::thread::spawn(move || {
            let mut w = theirs;
            w.write_all(b"PAM_TEXT_INFO Place your finger on the fingerprint reader\n")
                .unwrap();
            w.write_all(b"SUCCESS\n").unwrap();
        });
        let mut reports = Vec::new();
        let mut asked = 0;
        converse(
            BufReader::new(ours.try_clone().unwrap()),
            ours,
            "alice",
            &mut answers("unused", &mut asked),
            &mut |_| {},
            &mut |r| reports.push(r),
        );
        helper.join().unwrap();
        assert_eq!(asked, 0);
        assert_eq!(
            reports,
            [
                Report::Info("Place your finger on the fingerprint reader".into()),
                Report::Done(Verdict::Success)
            ]
        );
    }

    /// A second hidden prompt (a second factor) asks the dialog again, with its own text, and
    /// gets the new answer rather than the password.
    #[test]
    fn a_second_prompt_gets_a_fresh_answer() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let helper = std::thread::spawn(move || {
            let mut r = BufReader::new(theirs.try_clone().unwrap());
            let mut w = theirs;
            let mut got = Vec::new();
            for prompt in ["Password: ", "Verification code: "] {
                writeln!(w, "PAM_PROMPT_ECHO_OFF {prompt}").unwrap();
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                got.push(line.trim_end().to_string());
            }
            w.write_all(b"SUCCESS\n").unwrap();
            got
        });
        let mut reports = Vec::new();
        let mut given = ["hunter2", "123456"].into_iter();
        converse(
            BufReader::new(ours.try_clone().unwrap()),
            ours,
            "alice",
            &mut || given.next().map(|a| Zeroizing::new(a.to_string())),
            &mut |_| {},
            &mut |r| reports.push(r),
        );
        assert_eq!(helper.join().unwrap(), ["hunter2", "123456"]);
        assert_eq!(
            reports,
            [
                Report::Prompt("Password:".into()),
                Report::Prompt("Verification code:".into()),
                Report::Done(Verdict::Success)
            ]
        );
    }

    /// The reader's info line before any prompt lifts the deadline; the next line puts it back.
    #[test]
    fn the_deadline_rests_while_the_reader_waits() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let helper = std::thread::spawn(move || {
            let mut r = BufReader::new(theirs.try_clone().unwrap());
            let mut w = theirs;
            w.write_all(b"PAM_TEXT_INFO Place your finger on the fingerprint reader\n")
                .unwrap();
            w.write_all(b"PAM_PROMPT_ECHO_OFF Password: \n").unwrap();
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            w.write_all(b"PAM_TEXT_INFO Account expires soon\nSUCCESS\n")
                .unwrap();
        });
        let mut deadlines = Vec::new();
        let mut asked = 0;
        converse(
            BufReader::new(ours.try_clone().unwrap()),
            ours,
            "alice",
            &mut answers("hunter2", &mut asked),
            &mut |on| deadlines.push(on),
            &mut |_| {},
        );
        helper.join().unwrap();
        assert_eq!(
            deadlines,
            [false, true],
            "an info line after the password waits on the helper"
        );
    }

    /// A hang-up before any password judged nothing, and no password
    /// from the dialog (it closed) ends the conversation the same way.
    #[test]
    fn ending_before_the_password_spends_no_attempt() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        drop(theirs);
        let mut reports = Vec::new();
        let mut asked = 0;
        converse(
            BufReader::new(ours.try_clone().unwrap()),
            ours,
            "bob",
            &mut answers("x", &mut asked),
            &mut |_| {},
            &mut |r| reports.push(r),
        );
        assert_eq!(reports, [Report::Done(Verdict::Unreachable)]);

        let (ours, theirs) = UnixStream::pair().unwrap();
        let mut w = theirs;
        w.write_all(b"PAM_PROMPT_ECHO_OFF Password: \n").unwrap();
        let mut reports = Vec::new();
        converse(
            BufReader::new(ours.try_clone().unwrap()),
            ours,
            "bob",
            &mut || None,
            &mut |_| {},
            &mut |r| reports.push(r),
        );
        assert_eq!(
            reports,
            [
                Report::Prompt("Password:".into()),
                Report::Done(Verdict::Unreachable)
            ]
        );
    }

    /// Dropping the session ends a conversation blocked on a helper
    /// that says nothing (one waiting on the fingerprint reader).
    #[test]
    fn dropping_the_session_hangs_up_on_a_silent_helper() {
        let dir = scratch("stop");
        let sock = dir.join("agent-helper.socket");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let server = std::thread::spawn(move || {
            let (conn, _) = listener.accept().unwrap();
            let mut r = BufReader::new(conn);
            let mut rest = Vec::new();
            // Returns once the agent hangs up.
            let _ = std::io::Read::read_to_end(&mut r, &mut rest);
        });
        let (tx, rx) = mpsc::channel();
        let session = authenticate_at(
            sock,
            dir.join("no-helper"),
            |_| true,
            "alice".into(),
            "c-5".into(),
            tx,
            Box::new(|| {}),
        )
        .expect("the thread starts");
        // Let the thread connect and block in its read, then drop.
        std::thread::sleep(Duration::from_millis(100));
        drop(session);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)),
            Ok(Report::Done(Verdict::Unreachable))
        );
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A session dropped before its thread connects never reaches the
    /// helper at all.
    #[test]
    fn a_stop_before_connecting_reaches_nothing() {
        let stop = Stop::default();
        stop.fire();
        let (_ptx, prx) = mpsc::channel();
        let mut reports = Vec::new();
        run(
            Path::new("/nonexistent/socket"),
            Path::new("/nonexistent/helper"),
            true,
            "alice",
            "c",
            prx,
            &stop,
            &mut |r| reports.push(r),
        );
        assert!(reports.is_empty());
    }

    /// Without a finger the helper is reached only with the password in
    /// hand: a session dropped before OK never connects, so PAM never
    /// sees a hung-up conversation.
    #[test]
    fn without_a_finger_the_helper_waits_for_the_password() {
        let dir = scratch("late");
        let sock = dir.join("agent-helper.socket");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        listener.set_nonblocking(true).unwrap();
        let start = |tx| {
            authenticate_at(
                sock.clone(),
                dir.join("no-helper"),
                |_| false,
                "alice".into(),
                "c-8".into(),
                tx,
                Box::new(|| {}),
            )
            .expect("the thread starts")
        };

        let (tx, rx) = mpsc::channel();
        let session = start(tx);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            listener.accept().map(|_| ()).map_err(|e| e.kind()),
            Err(std::io::ErrorKind::WouldBlock),
            "no connection before the password"
        );
        drop(session);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)),
            Err(RecvTimeoutError::Disconnected),
            "the thread ends with no report"
        );
        assert_eq!(
            listener.accept().map(|_| ()).map_err(|e| e.kind()),
            Err(std::io::ErrorKind::WouldBlock),
            "no connection after the drop"
        );

        let (tx, rx) = mpsc::channel();
        let session = start(tx);
        assert!(session.answer(Zeroizing::new("hunter2".into())));
        listener.set_nonblocking(false).unwrap();
        let (conn, _) = listener.accept().unwrap();
        let mut r = BufReader::new(conn.try_clone().unwrap());
        let mut w = conn;
        let mut got = Vec::new();
        for _ in 0..2 {
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            got.push(line.trim_end().to_string());
        }
        w.write_all(b"PAM_PROMPT_ECHO_OFF Password: \n").unwrap();
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        got.push(line.trim_end().to_string());
        w.write_all(b"SUCCESS\n").unwrap();
        assert_eq!(got, ["alice", "c-8", "hunter2"]);
        let reports: Vec<Report> = (0..2)
            .map(|_| rx.recv_timeout(Duration::from_secs(5)).unwrap())
            .collect();
        assert_eq!(
            reports,
            [
                Report::Prompt("Password:".into()),
                Report::Done(Verdict::Success)
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A scratch directory of our own, since a helper stand-in must be
    /// executable. It sits in /tmp, not $TMPDIR, because a socket path
    /// must fit sun_path's 108 bytes, which a deep TMPDIR does not.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = Path::new("/tmp").join(format!("reprise-polkit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// [`connect_at`] spawning the script `helper`, since `dir` has no
    /// socket. A spawn can fail with ETXTBSY while another test thread
    /// forks between writing its own script and running it: the child
    /// holds that write fd until it execs. That passes, so retry.
    fn spawn_helper(
        dir: &Path,
        helper: &Path,
        user: &str,
        cookie: &str,
    ) -> std::io::Result<Transport> {
        let socket = dir.join("no-such.socket");
        let mut tries = 0;
        loop {
            match connect_at(&socket, helper, user, cookie) {
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 100 => {
                    tries += 1;
                    std::thread::sleep(Duration::from_millis(10));
                }
                other => return other,
            }
        }
    }

    /// Over the socket the helper reads the user and then the cookie,
    /// one line each.
    #[test]
    fn the_socket_gets_the_user_then_the_cookie() {
        let dir = scratch("sock");
        let sock = dir.join("agent-helper.socket");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let server = std::thread::spawn(move || {
            let (conn, _) = listener.accept().unwrap();
            let mut r = BufReader::new(conn);
            let mut a = String::new();
            let mut b = String::new();
            r.read_line(&mut a).unwrap();
            r.read_line(&mut b).unwrap();
            (a, b)
        });
        let t = connect_at(&sock, Path::new("/nonexistent/helper"), "alice", "c-1").unwrap();
        let Transport::Socket(s) = t else {
            panic!("expected the socket");
        };
        assert_eq!(
            s.read_timeout().unwrap(),
            Some(HELPER_TIMEOUT),
            "a silent helper times the read out"
        );
        assert_eq!(
            server.join().unwrap(),
            ("alice\n".to_string(), "c-1\n".to_string())
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Without the socket the setuid helper is spawned with the user
    /// as its argument and gets only the cookie on stdin.
    #[test]
    fn the_spawned_helper_gets_the_user_as_its_argument() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("spawn");
        let helper = dir.join("helper");
        std::fs::write(&helper, "#!/bin/sh\nread c\necho \"$1 $c\"\n").unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let t = spawn_helper(&dir, &helper, "bob", "c-2").unwrap();
        let Transport::Child(mut child) = t else {
            panic!("expected the spawned helper");
        };
        let mut out = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut out)
            .unwrap();
        let _ = child.wait();
        assert_eq!(out, "bob c-2\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A socket that is there but refuses the connection (its unit
    /// stopped) is an error, not a reason to spawn the helper binary.
    #[test]
    fn a_refused_socket_does_not_spawn_the_helper() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("refused");
        let sock = dir.join("agent-helper.socket");
        // A socket inode bound to nothing. A dropped listener would not do: a
        // helper another test thread spawns holds its fd, still listening, until
        // the child execs.
        let path = std::ffi::CString::new(sock.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: `path` is NUL-terminated and outlives the call.
        let rc = unsafe { libc::mknod(path.as_ptr(), libc::S_IFSOCK | 0o600, 0) };
        assert_eq!(rc, 0, "mknod: {}", std::io::Error::last_os_error());
        let helper = dir.join("helper");
        std::fs::write(&helper, "#!/bin/sh\ncat\n").unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = match connect_at(&sock, &helper, "bob", "c-4") {
            Err(e) => e,
            Ok(_) => panic!("expected an error, not a transport"),
        };
        assert_eq!(err.kind(), std::io::ErrorKind::ConnectionRefused);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A spawned helper that never answers is killed at the deadline,
    /// and the attempt ends without spending a retry.
    #[test]
    fn a_wedged_spawned_helper_is_killed_at_the_deadline() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("wedged");
        let helper = dir.join("helper");
        std::fs::write(&helper, "#!/bin/sh\nexec sleep 60\n").unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let Transport::Child(child) = spawn_helper(&dir, &helper, "bob", "c-3").unwrap() else {
            panic!("expected the spawned helper");
        };
        let start = std::time::Instant::now();
        let mut reports = Vec::new();
        let mut asked = 0;
        converse_child(
            child,
            "bob",
            &mut answers("x", &mut asked),
            std::time::Duration::from_millis(200),
            &Stop::default(),
            &mut |r| reports.push(r),
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "the attempt ends at the 200 ms deadline"
        );
        assert_eq!(reports, [Report::Done(Verdict::Unreachable)]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The deadline is for the helper, not the user: a slow typist past
    /// it still gets the password through.
    #[test]
    fn time_spent_typing_does_not_kill_the_spawned_helper() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("typing");
        let helper = dir.join("helper");
        std::fs::write(
            &helper,
            "#!/bin/sh\nread c\necho 'PAM_PROMPT_ECHO_OFF Password: '\nread p\n\
             [ \"$p\" = hunter2 ] && echo SUCCESS || echo FAILURE\n",
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let Transport::Child(child) = spawn_helper(&dir, &helper, "bob", "c-6").unwrap() else {
            panic!("expected the spawned helper");
        };
        let mut reports = Vec::new();
        let mut slow = || {
            std::thread::sleep(Duration::from_millis(400));
            Some(Zeroizing::new("hunter2".to_string()))
        };
        converse_child(
            child,
            "bob",
            &mut slow,
            Duration::from_millis(200),
            &Stop::default(),
            &mut |r| reports.push(r),
        );
        assert_eq!(
            reports,
            [
                Report::Prompt("Password:".into()),
                Report::Done(Verdict::Success)
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A spawned helper waiting on the fingerprint reader past the deadline is not killed.
    #[test]
    fn waiting_on_the_reader_does_not_kill_the_spawned_helper() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("finger");
        let helper = dir.join("helper");
        std::fs::write(
            &helper,
            "#!/bin/sh\nread c\necho 'PAM_TEXT_INFO Place your finger on the reader'\n\
             sleep 0.6\necho SUCCESS\n",
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let Transport::Child(child) = spawn_helper(&dir, &helper, "bob", "c-9").unwrap() else {
            panic!("expected the spawned helper");
        };
        let mut reports = Vec::new();
        converse_child(
            child,
            "bob",
            &mut || None,
            Duration::from_millis(200),
            &Stop::default(),
            &mut |r| reports.push(r),
        );
        assert_eq!(
            reports,
            [
                Report::Info("Place your finger on the reader".into()),
                Report::Done(Verdict::Success)
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A stop kills a spawned helper that is keeping us waiting.
    #[test]
    fn a_stop_kills_the_spawned_helper() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("killed");
        let helper = dir.join("helper");
        std::fs::write(&helper, "#!/bin/sh\nexec sleep 60\n").unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let Transport::Child(child) = spawn_helper(&dir, &helper, "bob", "c-7").unwrap() else {
            panic!("expected the spawned helper");
        };
        let stop = Arc::new(Stop::default());
        let firer = stop.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            firer.fire();
        });
        let start = std::time::Instant::now();
        let mut reports = Vec::new();
        converse_child(
            child,
            "bob",
            &mut || None,
            HELPER_TIMEOUT,
            &stop,
            &mut |r| reports.push(r),
        );
        t.join().unwrap();
        assert!(start.elapsed() < Duration::from_secs(10));
        assert_eq!(reports, [Report::Done(Verdict::Unreachable)]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
