//! The lock screen's PAM conversations, each in a child of the compositor binary so no PAM module runs in the compositor and the lock can end one at any time: the fingerprint's (`--fingerprint-conversation USER`) and the password's (`--password-conversation USER`, the password on stdin).
//! A child writes `I <info>` and `E <error>` lines and `= <pam_authenticate rc>` last; only `= 0` unlocks.

use crate::lock::{self, Verdict, Verifier, ffi};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const SERVICE: &str = "reprise-fingerprint";
pub const FLAG: &str = "--fingerprint-conversation";
pub const PASSWORD_FLAG: &str = "--password-conversation";
/// Time an ended conversation gets to release the reader after SIGINT before it is killed.
pub const RELEASE_GRACE: Duration = Duration::from_millis(500);

/// Ended conversations still running, unreaped so their pids stay theirs, and when each is killed.
static ENDING: Mutex<Vec<(libc::pid_t, Instant)>> = Mutex::new(Vec::new());

/// Without the service libpam would fall back to the `other` service.
pub fn service_installed() -> bool {
    Path::new("/etc/pam.d").join(SERVICE).exists()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    Info(String),
    Error(String),
    Result(c_int),
}

pub fn parse(line: &str) -> Option<Line> {
    if let Some(t) = line.strip_prefix("I ") {
        Some(Line::Info(t.to_string()))
    } else if let Some(t) = line.strip_prefix("E ") {
        Some(Line::Error(t.to_string()))
    } else if let Some(rc) = line.strip_prefix("= ") {
        rc.parse().ok().map(Line::Result)
    } else {
        None
    }
}

/// The fingerprint conversation's command for `user`; None when its PAM service is not installed.
pub fn finger_command(user: &str) -> Option<Command> {
    if !service_installed() {
        return None;
    }
    let mut c = Command::new("/proc/self/exe");
    c.arg0("reprise").arg(format!("{FLAG}={user}"));
    Some(c)
}

pub fn password_command(user: &str) -> Command {
    let mut c = Command::new("/proc/self/exe");
    c.arg0("reprise").arg(format!("{PASSWORD_FLAG}={user}"));
    c
}

/// Folds the password conversation's lines into its verdict: the text lines are the notices of a rejection.
#[derive(Default)]
pub struct PasswordAnswer {
    notices: Vec<String>,
}

impl PasswordAnswer {
    /// The verdict, once the result line arrives.
    pub fn line(&mut self, line: Line) -> Option<Verdict> {
        match line {
            Line::Info(t) | Line::Error(t) => {
                lock::add_notice(&mut self.notices, &t);
                None
            }
            Line::Result(ffi::PAM_SUCCESS) => Some(Ok(())),
            Line::Result(_) => Some(Err(std::mem::take(&mut self.notices))),
        }
    }
}

pub struct Conversation {
    child: Child,
    stdout: ChildStdout,
    buf: Vec<u8>,
}

impl Conversation {
    pub fn start(c: Command) -> std::io::Result<Conversation> {
        Conversation::spawn(c, None)
    }

    /// Start `c` and hand it `password` on stdin, which then closes: never in argv or the environment.
    pub fn start_password(c: Command, password: &[u8]) -> std::io::Result<Conversation> {
        Conversation::spawn(c, Some(password))
    }

    /// Spawn `c` as the child: stdout piped, stdin piped for `input` or else /dev/null, empty signal mask, SIGINT at its default action (an ignored SIGINT survives exec) and SIGKILL when the compositor dies, so a crash never leaves the reader claimed.
    fn spawn(mut c: Command, input: Option<&[u8]>) -> std::io::Result<Conversation> {
        c.stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped());
        // SAFETY: the closure runs between fork and exec; sigemptyset,
        // pthread_sigmask, signal and prctl are async-signal-safe and
        // nothing allocates.
        unsafe {
            c.pre_exec(|| {
                let mut set: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut set);
                libc::pthread_sigmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
                libc::signal(libc::SIGINT, libc::SIG_DFL);
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
        let mut child = c.spawn()?;
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::ErrorKind::BrokenPipe.into());
        };
        if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
            // A blocking write that cannot block: a fresh pipe holds far more than
            // `lock::INPUT_CAPACITY`. Dropping `stdin` closes it, which ends the child's read.
            if let Err(e) = stdin.write_all(input) {
                log::warn!("lock: cannot hand the password to its conversation: {e}");
            }
        }
        // SAFETY: `stdout` is an open pipe we own; F_GETFL/F_SETFL take no
        // pointers.
        unsafe {
            let fd = stdout.as_raw_fd();
            let flags = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        Ok(Conversation {
            child,
            stdout,
            buf: Vec::new(),
        })
    }

    /// SIGKILL the child, for a conversation that hangs; dropping it reaps it.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }

    #[cfg(test)]
    pub fn pid(&self) -> libc::pid_t {
        self.child.id() as libc::pid_t
    }

    pub fn fd(&self) -> RawFd {
        self.stdout.as_raw_fd()
    }

    pub fn read(&mut self) -> (Vec<Line>, bool) {
        let mut eof = false;
        let mut chunk = [0u8; 1024];
        loop {
            match self.stdout.read(&mut chunk) {
                Ok(0) => {
                    eof = true;
                    break;
                }
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    eof = true;
                    break;
                }
            }
        }
        let mut lines = Vec::new();
        while let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = self.buf.drain(..=i).collect();
            if let Some(l) = parse(String::from_utf8_lossy(&raw[..i]).as_ref()) {
                lines.push(l);
            }
        }
        (lines, eof)
    }
}

/// SIGINT and no wait: the fingerprint child blocks it, so pam_fprintd's own signal watch stops
/// verifying and releases the reader before PAM returns. A child still running goes on `ENDING` for
/// `reap_ended`.
impl Drop for Conversation {
    fn drop(&mut self) {
        let pid = self.child.id() as libc::pid_t;
        if let Ok(None) = self.child.try_wait() {
            // SAFETY: `pid` is our unreaped child, so it cannot be reused.
            unsafe { libc::kill(pid, libc::SIGINT) };
            crate::mutex::lock(&ENDING).push((pid, Instant::now() + RELEASE_GRACE));
        }
    }
}

/// Reap the ended conversations that are gone and kill those still running past their grace at `now`,
/// leaving them to `spawn::reap_children`. Returns when the next one is due.
pub fn reap_ended(now: Instant) -> Option<Instant> {
    let mut ending = crate::mutex::lock(&ENDING);
    ending.retain(|&(pid, deadline)| {
        let mut status = 0;
        // SAFETY: `status` is a valid out-param; `pid` is our unreaped child.
        if unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) } != 0 {
            return false;
        }
        if now < deadline {
            return true;
        }
        log::warn!("lock: the PAM conversation {pid} did not end; killing it");
        // SAFETY: as above.
        unsafe { libc::kill(pid, libc::SIGKILL) };
        crate::spawn::adopt(pid, "PAM conversation");
        false
    });
    ending.iter().map(|&(_, d)| d).min()
}

/// The password conversation's child: the password from stdin, `lock::PamVerifier`, and the verdict as lines on stdout.
pub fn password_main(user: &str) -> i32 {
    log::set_max_level(log::LevelFilter::Warn);
    // SAFETY: no other thread exists yet; the sigset is a zeroed local. `main` blocked SIGINT,
    // which here is to end the conversation at once.
    unsafe {
        libc::setlocale(libc::LC_ALL, c"".as_ptr());
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::pthread_sigmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
    }
    let (Ok(input), Ok(out)) = (
        std::io::stdin().as_fd().try_clone_to_owned(),
        std::io::stdout().as_fd().try_clone_to_owned(),
    ) else {
        return 1;
    };
    // The lines go out on a copy of stdout and fd 1 points at /dev/null, so nothing a module
    // prints can pass for a line.
    if let Ok(null) = std::fs::File::options().write(true).open("/dev/null") {
        // SAFETY: both fds are open; dup2 replaces fd 1, which nothing here holds a borrow of.
        unsafe { libc::dup2(null.as_raw_fd(), libc::STDOUT_FILENO) };
    }
    // `File`, not `Stdin`: stdin's buffer would keep a copy of the password nobody zeroes.
    password_conversation(
        &lock::PamVerifier,
        user,
        std::fs::File::from(input),
        std::fs::File::from(out),
    );
    0
}

/// Read the password (at most `lock::INPUT_CAPACITY` bytes, until EOF) into a buffer zeroed on drop, verify it with `v` and write the verdict's lines to `out`.
fn password_conversation(v: &dyn Verifier, user: &str, mut input: impl Read, mut out: impl Write) {
    let mut buf = zeroize::Zeroizing::new([0u8; lock::INPUT_CAPACITY]);
    let mut n = 0;
    while n < buf.len() {
        match input.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    let verdict = match std::str::from_utf8(&buf[..n]) {
        Ok(password) => v.verify(user, password),
        Err(_) => Err(Vec::new()),
    };
    let mut lines = String::new();
    match &verdict {
        Ok(()) => lines.push_str(&format!("= {}\n", ffi::PAM_SUCCESS)),
        Err(notices) => {
            for t in notices {
                lines.push_str(&format!("E {t}\n"));
            }
            lines.push_str(&format!("= {}\n", ffi::PAM_AUTH_ERR));
        }
    }
    let _ = out.write_all(lines.as_bytes());
    let _ = out.flush();
}

pub fn child_main(user: &str) -> i32 {
    log::set_max_level(log::LevelFilter::Warn);
    // SAFETY: no other thread exists yet; the sigset is a zeroed local.
    unsafe {
        libc::setlocale(libc::LC_ALL, c"".as_ptr());
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
    let rc = if service_installed() && reprisebus::fprint::has_finger(user) {
        authenticate(user)
    } else {
        ffi::PAM_AUTHINFO_UNAVAIL
    };
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "= {rc}");
    let _ = out.flush();
    0
}

/// `pam_authenticate` on `SERVICE` for `user`; a prompt fails it, as there is no password in this conversation.
fn authenticate(user: &str) -> c_int {
    let (Ok(service), Ok(cuser)) = (CString::new(SERVICE), CString::new(user)) else {
        return ffi::PAM_AUTHINFO_UNAVAIL;
    };
    let conv = ffi::pam_conv {
        conv: Some(converse),
        appdata_ptr: std::ptr::null_mut(),
    };
    let mut handle: *mut ffi::pam_handle_t = std::ptr::null_mut();
    // SAFETY: every pointer is to a live local or a NUL-terminated CString;
    // pam_end closes the handle below.
    let rc = unsafe { ffi::pam_start(service.as_ptr(), cuser.as_ptr(), &conv, &mut handle) };
    if rc != ffi::PAM_SUCCESS || handle.is_null() {
        log::error!("pam: cannot start service {SERVICE}: code {rc}");
        return if rc == ffi::PAM_SUCCESS {
            ffi::PAM_AUTHINFO_UNAVAIL
        } else {
            rc
        };
    }
    // A failure is final here (the conversation is not started over), so
    // libpam's delay after one would only hold the lock's SIGINT up.
    // SAFETY: `handle` is the live handle pam_start just returned, and
    // `no_delay` has the signature libpam calls PAM_FAIL_DELAY with.
    unsafe {
        ffi::pam_set_item(handle, ffi::PAM_FAIL_DELAY, no_delay as *const c_void);
    }
    // SAFETY: `handle` is the live handle pam_start just returned.
    let rc = unsafe { ffi::pam_authenticate(handle, 0) };
    // SAFETY: `handle` is live and not used after this call.
    unsafe { ffi::pam_end(handle, rc) };
    rc
}

unsafe extern "C" fn no_delay(_retval: c_int, _usec: libc::c_uint, _data: *mut c_void) {}

/// The child's conversation callback: text lines go out as `I` or `E` lines; a prompt ends the conversation, since there is nothing to answer it with.
unsafe extern "C" fn converse(
    n: c_int,
    msgs: *mut *const ffi::pam_message,
    out: *mut *mut ffi::pam_response,
    _data: *mut c_void,
) -> c_int {
    if n <= 0 || msgs.is_null() || out.is_null() {
        return ffi::PAM_CONV_ERR;
    }
    let mut lines = Vec::new();
    for i in 0..n as usize {
        // SAFETY: libpam passes `n` (> 0) live messages behind non-null `msgs`.
        let m = unsafe { &**msgs.add(i) };
        let tag = match m.msg_style {
            ffi::PAM_TEXT_INFO => 'I',
            ffi::PAM_ERROR_MSG => 'E',
            _ => return ffi::PAM_CONV_ERR,
        };
        if m.msg.is_null() {
            continue;
        }
        // SAFETY: libpam's message text is a NUL-terminated string.
        let text = unsafe { CStr::from_ptr(m.msg as *const c_char) }.to_string_lossy();
        for l in lock::clean_notice(&text) {
            lines.push(format!("{tag} {l}\n"));
        }
    }
    let mut stdout = std::io::stdout().lock();
    for l in lines {
        let _ = stdout.write_all(l.as_bytes());
    }
    let _ = stdout.flush();
    // SAFETY: plain allocation of an all-null answer array, which libpam frees.
    let resp = unsafe { libc::calloc(n as usize, std::mem::size_of::<ffi::pam_response>()) }
        as *mut ffi::pam_response;
    if resp.is_null() {
        return ffi::PAM_BUF_ERR;
    }
    // SAFETY: `out` is non-null (checked above) and is libpam's result slot.
    unsafe { *out = resp };
    ffi::PAM_SUCCESS
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn only_the_three_line_kinds_parse() {
        assert_eq!(
            parse("I Place your finger on the fingerprint reader"),
            Some(Line::Info(
                "Place your finger on the fingerprint reader".into()
            ))
        );
        assert_eq!(
            parse("E Failed to match fingerprint"),
            Some(Line::Error("Failed to match fingerprint".into()))
        );
        assert_eq!(parse("= 0"), Some(Line::Result(0)));
        assert_eq!(parse("= 7"), Some(Line::Result(7)));
        assert_eq!(parse("=0"), None, "the result needs its space");
        assert_eq!(parse("= 0 "), None, "nothing after the code");
        assert_eq!(parse("= ok"), None);
        assert_eq!(parse("OK"), None);
        assert_eq!(parse("I"), None, "a text line needs its space");
    }

    fn sh(script: &str) -> Conversation {
        let mut c = Command::new("sh");
        c.arg("-c").arg(script);
        Conversation::start(c).expect("sh starts")
    }

    struct FakeVerifier(Verdict);
    impl Verifier for FakeVerifier {
        fn verify(&self, user: &str, password: &str) -> Verdict {
            assert_eq!(user, "someone");
            assert_eq!(password, "hunter2");
            self.0.clone()
        }
    }

    #[test]
    fn the_password_conversation_carries_the_verifiers_verdict_back() {
        let locked = vec!["Account locked.".to_string(), "(10 minutes left)".into()];
        for expected in [Ok(()), Err(Vec::new()), Err(locked)] {
            let mut out = Vec::new();
            password_conversation(
                &FakeVerifier(expected.clone()),
                "someone",
                &b"hunter2"[..],
                &mut out,
            );
            let mut answer = PasswordAnswer::default();
            let verdicts: Vec<Verdict> = String::from_utf8(out)
                .unwrap()
                .lines()
                .filter_map(parse)
                .filter_map(|l| answer.line(l))
                .collect();
            assert_eq!(verdicts, [expected]);
        }
    }

    #[test]
    fn the_password_goes_in_on_stdin_and_stdin_closes() {
        let mut c = Command::new("sh");
        c.arg("-c")
            .arg(r#"p=$(cat); [ "$p" = hunter2 ] && echo '= 0' || echo '= 7'"#);
        let mut c = Conversation::start_password(c, b"hunter2").expect("sh starts");
        assert_eq!(read_all(&mut c), [Line::Result(0)]);
    }

    fn read_all(c: &mut Conversation) -> Vec<Line> {
        let end = Instant::now() + Duration::from_secs(5);
        let mut all = Vec::new();
        loop {
            let (lines, eof) = c.read();
            all.extend(lines);
            if eof {
                return all;
            }
            assert!(Instant::now() < end, "the child's stdout closes");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_lines_arrive_in_order_up_to_the_result() {
        let mut c = sh("printf 'I Place your finger\\nE Failed to match\\nnoise\\n= 0\\n'");
        assert_eq!(
            read_all(&mut c),
            [
                Line::Info("Place your finger".into()),
                Line::Error("Failed to match".into()),
                Line::Result(0),
            ]
        );
    }

    #[test]
    fn a_line_split_across_reads_is_joined() {
        let mut c = sh("printf 'I Pla'; sleep 0.1; printf 'ce\\n= 9\\n'");
        assert_eq!(
            read_all(&mut c),
            [Line::Info("Place".into()), Line::Result(9)]
        );
    }

    /// Whether `pid` is gone or a zombie: no longer running.
    pub(crate) fn ended(pid: libc::pid_t) -> bool {
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat
                .rsplit(')')
                .next()
                .is_some_and(|s| s.trim_start().starts_with('Z')),
            Err(_) => true,
        }
    }

    fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !f() {
            assert!(Instant::now() < end, "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn dropping_interrupts_the_child_without_waiting_and_reap_ended_reaps_it() {
        let mark = std::env::temp_dir().join(format!("reprise-finger-{}", std::process::id()));
        let _ = std::fs::remove_file(&mark);
        let mut c = sh(&format!(
            "trap 'echo released > {}; exit 0' INT; echo 'I ready'; while :; do sleep 0.01; done",
            mark.display()
        ));
        let pid = c.pid();
        wait_until("the child says ready", || !c.read().0.is_empty());
        let start = Instant::now();
        drop(c);
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "drop does not wait"
        );
        wait_until("the child hears SIGINT", || {
            std::fs::read_to_string(&mark).ok().as_deref() == Some("released\n")
        });
        wait_until("reap_ended reaps it", || {
            reap_ended(Instant::now());
            // SAFETY: signal 0 only checks that the pid exists.
            unsafe { libc::kill(pid, 0) == -1 }
        });
        let _ = std::fs::remove_file(&mark);
    }

    #[test]
    fn a_child_that_ignores_the_interrupt_is_killed_once_its_grace_is_over() {
        let c = sh("trap '' INT; exec sleep 60");
        let pid = c.pid();
        std::thread::sleep(Duration::from_millis(50));
        let start = Instant::now();
        drop(c);
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "drop does not wait"
        );
        let due = reap_ended(Instant::now());
        assert!(due.is_some(), "the child waits out its grace");
        assert!(!ended(pid), "still running within the grace");
        wait_until("the child is killed", || {
            reap_ended(Instant::now());
            ended(pid)
        });
        assert!(
            start.elapsed() >= RELEASE_GRACE,
            "killed only after its grace"
        );
        crate::spawn::reap_children();
    }
}
