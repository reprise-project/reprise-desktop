//! The lock screen's fingerprint conversation: PAM's `reprise-fingerprint` service in a child process (`--fingerprint-conversation USER`) so the lock can end it at any time. The child writes `I <info>` and `E <error>` lines and `= <pam_authenticate rc>` last; only `= 0` unlocks.

use crate::lock::{self, ffi};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

pub const SERVICE: &str = "reprise-fingerprint";
pub const FLAG: &str = "--fingerprint-conversation";
/// Time an ended conversation gets to release the reader after SIGINT before it is killed.
const RELEASE_GRACE: Duration = Duration::from_millis(500);

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

pub struct Conversation {
    child: Child,
    stdout: ChildStdout,
    buf: Vec<u8>,
}

impl Conversation {
    pub fn start(user: &str) -> std::io::Result<Conversation> {
        let mut c = Command::new("/proc/self/exe");
        c.arg0("reprise").arg(format!("{FLAG}={user}"));
        Conversation::spawn(c)
    }

    /// Spawn `c` as the child: stdout piped, stdin on /dev/null, empty signal mask, SIGINT at its default action (an ignored SIGINT survives exec) and SIGKILL when the compositor dies, so a crash never leaves the reader claimed.
    fn spawn(mut c: Command) -> std::io::Result<Conversation> {
        c.stdin(Stdio::null()).stdout(Stdio::piped());
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

/// SIGINT first: the child blocks it, so pam_fprintd's own signal watch stops verifying and releases the reader before PAM returns. A child still running after `RELEASE_GRACE` is killed; either way it is reaped here.
impl Drop for Conversation {
    fn drop(&mut self) {
        let pid = self.child.id() as libc::pid_t;
        if let Ok(None) = self.child.try_wait() {
            // SAFETY: `pid` is our unreaped child, so it cannot be reused.
            unsafe { libc::kill(pid, libc::SIGINT) };
            let end = Instant::now() + RELEASE_GRACE;
            while Instant::now() < end {
                match self.child.try_wait() {
                    Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                    _ => return,
                }
            }
            log::warn!("lock: the fingerprint conversation did not end; killing it");
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
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
mod tests {
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
        Conversation::spawn(c).expect("sh starts")
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

    #[test]
    fn dropping_interrupts_the_child_and_reaps_it() {
        let mark = std::env::temp_dir().join(format!("reprise-finger-{}", std::process::id()));
        let _ = std::fs::remove_file(&mark);
        let mut c = sh(&format!(
            "trap 'echo released > {}; exit 0' INT; echo 'I ready'; while :; do sleep 0.01; done",
            mark.display()
        ));
        let pid = c.child.id() as libc::pid_t;
        let end = Instant::now() + Duration::from_secs(5);
        while c.read().0.is_empty() {
            assert!(Instant::now() < end, "the child says ready");
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(c);
        assert_eq!(
            std::fs::read_to_string(&mark).ok().as_deref(),
            Some("released\n")
        );
        // SAFETY: signal 0 only checks that the pid exists.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "the child is reaped");
        let _ = std::fs::remove_file(&mark);
    }

    #[test]
    fn a_child_that_ignores_the_interrupt_is_killed() {
        let c = sh("trap '' INT; exec sleep 60");
        let pid = c.child.id() as libc::pid_t;
        std::thread::sleep(Duration::from_millis(50));
        let start = Instant::now();
        drop(c);
        assert!(
            start.elapsed() < RELEASE_GRACE + Duration::from_secs(1),
            "ends shortly after the 500 ms grace"
        );
        // SAFETY: signal 0 only checks that the pid exists.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "the child is reaped");
    }
}
