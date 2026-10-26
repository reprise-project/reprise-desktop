//! Passwords: crypt(3) hashing for AccountsService's `SetPassword`, and
//! changing your own through `passwd` on a pseudo-terminal (`passwd`
//! checks the current password itself; AccountsService would want an
//! administrator's).

use std::ffi::CStr;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

#[link(name = "crypt")]
unsafe extern "C" {
    fn crypt_gensalt_ra(
        prefix: *const libc::c_char,
        count: libc::c_ulong,
        rbytes: *const libc::c_char,
        nrbytes: libc::c_int,
    ) -> *mut libc::c_char;
    fn crypt_ra(
        phrase: *const libc::c_char,
        setting: *const libc::c_char,
        data: *mut *mut libc::c_void,
        size: *mut libc::c_int,
    ) -> *mut libc::c_char;
}

pub fn crypt(password: &str) -> Result<String, String> {
    if password.contains('\0') {
        return Err("The password holds a NUL.".into());
    }
    let mut phrase = Zeroizing::new(Vec::with_capacity(password.len() + 1));
    phrase.extend_from_slice(password.as_bytes());
    phrase.push(0);
    // SAFETY: a null prefix picks the default method and null random
    // bytes make libcrypt draw its own; the result is malloc'd (or null)
    // and freed below.
    let setting = unsafe { crypt_gensalt_ra(std::ptr::null(), 0, std::ptr::null(), 0) };
    if setting.is_null() {
        return Err("No password hashing method is available.".into());
    }
    let mut data: *mut libc::c_void = std::ptr::null_mut();
    let mut size: libc::c_int = 0;
    // SAFETY: both strings are NUL-terminated and live across the call;
    // crypt_ra allocates `data`, which holds the result until freed.
    let out = unsafe { crypt_ra(phrase.as_ptr().cast(), setting, &mut data, &mut size) };
    let hash = if out.is_null() {
        None
    } else {
        // SAFETY: a non-null result is a NUL-terminated string in `data`.
        let s = unsafe { CStr::from_ptr(out) };
        Some(s.to_string_lossy().into_owned())
    };
    // SAFETY: both were allocated by libcrypt with malloc; `data` is `size` bytes, its work
    // area holding state derived from the phrase.
    unsafe {
        libc::free(setting.cast());
        if !data.is_null() {
            libc::explicit_bzero(data, size.max(0) as usize);
        }
        libc::free(data);
    }
    match hash {
        Some(h) if !h.starts_with('*') => Ok(h),
        _ => Err("The password could not be hashed.".into()),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Wait,
    /// Type answer `i` (0 the current password, 1 and 2 the new).
    Answer(usize),
    /// A prompt past the three answers: `passwd` refused the new password; the text is why.
    Refused(String),
}

/// The talk with `passwd`: three prompts (current, new, new again), each a line ending in a colon.
/// Other output between prompts is kept for the error box.
#[derive(Default)]
pub struct Dialogue {
    answered: usize,
    pending: String,
    pub said: Vec<String>,
}

impl Dialogue {
    pub fn feed(&mut self, out: &str) -> Step {
        self.pending.push_str(&out.replace('\r', ""));
        let tail = self.pending.trim_end();
        if !tail.ends_with(':') {
            return Step::Wait;
        }
        let mut lines: Vec<String> = self.pending.lines().map(str::to_string).collect();
        lines.pop();
        self.said
            .extend(lines.into_iter().filter(|l| !l.trim().is_empty()));
        self.pending.clear();
        let i = self.answered;
        self.answered += 1;
        if i < 3 {
            Step::Answer(i)
        } else {
            Step::Refused(self.why())
        }
    }

    pub fn why(&self) -> String {
        let all = || {
            self.said
                .iter()
                .map(String::as_str)
                .chain(self.pending.lines())
        };
        if all().any(|l| {
            l.contains("Authentication token manipulation") || l.contains("Authentication failure")
        }) {
            return "The current password is not correct.".into();
        }
        let last = self
            .said
            .iter()
            .chain(
                self.pending
                    .lines()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
                    .iter(),
            )
            .rfind(|l| !l.trim().is_empty() && !l.starts_with("Changing password"))
            .cloned()
            .unwrap_or_default();
        let last = last.trim();
        let last = last.strip_prefix("passwd: ").unwrap_or(last);
        let last = last.strip_prefix("BAD PASSWORD: ").unwrap_or(last);
        if last.is_empty() {
            "The password was not changed.".into()
        } else {
            let mut s = last.to_string();
            if let Some(c) = s.get(0..1) {
                s.replace_range(0..1, &c.to_uppercase());
            }
            if !s.ends_with('.') {
                s.push('.');
            }
            s
        }
    }
}

const QUIET: Duration = Duration::from_secs(20);

pub fn change_own(current: &str, new: &str) -> Result<(), String> {
    talk(Command::new("passwd"), current, new)
}

fn talk(mut cmd: Command, current: &str, new: &str) -> Result<(), String> {
    let (master, slave) = open_pty().map_err(|e| format!("No terminal for passwd: {e}"))?;
    let tty = |fd: &OwnedFd| fd.try_clone().map(Stdio::from);
    cmd.env("LC_ALL", "C")
        .stdin(tty(&slave).map_err(|e| e.to_string())?)
        .stdout(tty(&slave).map_err(|e| e.to_string())?)
        .stderr(tty(&slave).map_err(|e| e.to_string())?);
    // SAFETY: setsid and the ioctl are async-signal-safe; the child makes the pty its
    // controlling terminal, as passwd's PAM conversation expects.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("passwd could not start: {e}"))?;
    // The command keeps its copies of the slave until dropped; any left open here
    // stop the master hanging up when passwd exits.
    drop(cmd);
    drop(slave);
    let answers = [current, new, new];
    let mut talk = Dialogue::default();
    let mut file = std::fs::File::from(master);
    let mut buf = [0u8; 512];
    let mut last = Instant::now();
    let refused = loop {
        let mut pfd = libc::pollfd {
            fd: file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one pollfd on a descriptor this function owns.
        let n = unsafe { libc::poll(&mut pfd, 1, 500) };
        if n == 0 {
            if matches!(child.try_wait(), Ok(Some(_))) {
                break None;
            }
            if last.elapsed() > QUIET {
                break Some("passwd stopped answering.".to_string());
            }
            continue;
        }
        let read = match file.read(&mut buf) {
            Ok(0) | Err(_) => break None,
            Ok(r) => r,
        };
        last = Instant::now();
        match talk.feed(&String::from_utf8_lossy(&buf[..read])) {
            Step::Wait => {}
            Step::Answer(i) => {
                let mut line = Zeroizing::new(Vec::with_capacity(answers[i].len() + 1));
                line.extend_from_slice(answers[i].as_bytes());
                line.push(b'\n');
                if file.write_all(&line).is_err() {
                    break None;
                }
            }
            Step::Refused(why) => break Some(why),
        }
    };
    if refused.is_some() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    match refused {
        Some(why) => Err(why),
        None if status.success() => Ok(()),
        None => Err(talk.why()),
    }
}

fn open_pty() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let (mut m, mut s) = (-1, -1);
    // SAFETY: openpty writes two descriptors into the two locals; the
    // name, termios and window size pointers may be null.
    let rc = unsafe {
        libc::openpty(
            &mut m,
            &mut s,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if rc < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: both descriptors were just opened and are owned here.
    Ok(unsafe { (OwnedFd::from_raw_fd(m), OwnedFd::from_raw_fd(s)) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hash_is_a_crypt_string_that_checks_out() {
        let h = crypt("s3cret pass").unwrap();
        assert!(h.starts_with('$'), "{h}");
        assert_ne!(
            h,
            crypt("s3cret pass").unwrap(),
            "each hash gets its own salt"
        );
    }

    /// A stand-in for passwd: three prompts, then success when the current password is "old" and the new one matches twice.
    fn fake_passwd() -> Command {
        let mut c = Command::new("sh");
        c.arg("-c").arg(
            "printf 'Changing password for jo.\\nCurrent password: '; read a; \
             printf 'New password: '; read b; printf 'Retype new password: '; read c; \
             if [ \"$a\" = old ] && [ \"$b\" = \"$c\" ]; then \
               echo 'passwd: password updated successfully'; \
             else echo 'passwd: Authentication token manipulation error'; exit 10; fi",
        );
        c
    }

    #[test]
    fn a_pty_talk_ends_when_the_program_exits() {
        let start = Instant::now();
        assert_eq!(talk(fake_passwd(), "old", "new pw"), Ok(()));
        assert_eq!(
            talk(fake_passwd(), "wrong", "new pw"),
            Err("The current password is not correct.".into())
        );
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "no wait for the quiet timeout"
        );
    }

    #[test]
    fn the_three_prompts_are_answered_in_order() {
        let mut d = Dialogue::default();
        assert_eq!(d.feed("Changing password for jo.\r\n"), Step::Wait);
        assert_eq!(d.feed("Current pass"), Step::Wait);
        assert_eq!(d.feed("word: "), Step::Answer(0));
        assert_eq!(d.feed("\r\nNew password: "), Step::Answer(1));
        assert_eq!(d.feed("\r\nRetype new password: "), Step::Answer(2));
        assert_eq!(
            d.feed("\r\npasswd: password updated successfully\r\n"),
            Step::Wait
        );
    }

    #[test]
    fn a_fourth_prompt_is_a_refusal_with_its_reason() {
        let mut d = Dialogue::default();
        d.feed("Current password: ");
        d.feed("\nNew password: ");
        d.feed("\nRetype new password: ");
        let step =
            d.feed("\nBAD PASSWORD: The password is shorter than 8 characters\nNew password: ");
        assert_eq!(
            step,
            Step::Refused("The password is shorter than 8 characters.".into())
        );
    }

    #[test]
    fn a_wrong_current_password_reads_as_passwd_says_it() {
        let mut d = Dialogue::default();
        d.feed("Current password: ");
        d.feed("\npasswd: Authentication token manipulation error\npasswd: password unchanged\n");
        assert_eq!(d.why(), "The current password is not correct.");
        let mut d = Dialogue::default();
        assert_eq!(d.why(), "The password was not changed.");
        d.feed("Current password: ");
        d.feed("\npasswd: Have exhausted maximum number of retries for service\n");
        assert_eq!(
            d.why(),
            "Have exhausted maximum number of retries for service."
        );
    }
}
