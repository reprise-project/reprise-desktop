//! Passwords: crypt(3) hashing for AccountsService's `SetPassword`, and
//! changing your own through `passwd` on a pseudo-terminal (`passwd`
//! checks the current password itself; AccountsService would want an
//! administrator's), and asking `passwd -S` whether yours has one.

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

/// Which password a prompt asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Current,
    New,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Wait,
    Answer(Ask),
    /// A prompt out of turn: `passwd` refused the new password, or asked something it is not
    /// given here; the text is why.
    Refused(String),
}

/// The talk with `passwd`, each prompt a line ending in a colon and answered by its text: the
/// current password (skipped when the account has none and PAM allows that), then the new one
/// twice. Other output between prompts is kept for the error box, in buffers wiped when dropped.
#[derive(Default)]
pub struct Dialogue {
    asked_current: bool,
    asked_new: u8,
    pending: Zeroizing<String>,
    said: Zeroizing<Vec<String>>,
}

/// Appends `s` to `buf`, moving to a larger buffer first so no copy is left in a freed one.
fn push_wiped(buf: &mut Zeroizing<String>, s: &str) {
    if buf.capacity() - buf.len() < s.len() {
        let mut grown = String::with_capacity((buf.len() + s.len()).max(2 * buf.capacity()));
        grown.push_str(buf);
        *buf = Zeroizing::new(grown);
    }
    buf.push_str(s);
}

impl Dialogue {
    pub fn feed(&mut self, out: &str) -> Step {
        let mut clean = Zeroizing::new(String::with_capacity(out.len()));
        clean.extend(out.chars().filter(|&c| c != '\r'));
        push_wiped(&mut self.pending, &clean);
        let tail = self.pending.trim_end();
        if !tail.ends_with(':') {
            return Step::Wait;
        }
        let mut lines: Zeroizing<Vec<String>> =
            Zeroizing::new(self.pending.lines().map(str::to_string).collect());
        let before_prompt = lines.len().saturating_sub(1);
        self.said.extend(
            lines
                .drain(..before_prompt)
                .filter(|l| !l.trim().is_empty()),
        );
        let prompt = lines.pop().unwrap_or_default().to_lowercase();
        self.pending.clear();
        let (retype, new) = (prompt.contains("retype"), prompt.contains("new"));
        if prompt.contains("current") && !self.asked_current && self.asked_new == 0 {
            self.asked_current = true;
            Step::Answer(Ask::Current)
        } else if retype && self.asked_new == 1 {
            self.asked_new = 2;
            Step::Answer(Ask::New)
        } else if new && !retype && self.asked_new == 0 {
            self.asked_new = 1;
            Step::Answer(Ask::New)
        } else if new && self.asked_new > 0 {
            Step::Refused(self.why())
        } else {
            Step::Refused("The password cannot be changed here.".into())
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
const PASSWD: &str = "/usr/bin/passwd";
/// How long an answer waits for `passwd` to turn echo off after its prompt.
const ECHO_WAIT: Duration = Duration::from_secs(1);

/// Whether your own account has no password, as `passwd -S` (setuid, so it reads your shadow
/// entry) says; false when it cannot tell.
pub fn own_has_none() -> bool {
    let out = Command::new(PASSWD)
        .arg("-S")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => says_none(&String::from_utf8_lossy(&o.stdout)),
        Ok(o) => {
            log::warn!("passwd -S: {}", o.status);
            false
        }
        Err(e) => {
            log::warn!("passwd -S could not start: {e}");
            false
        }
    }
}

/// A `passwd -S` line's second field: `NP` no password, `P` one, `L` locked.
fn says_none(status: &str) -> bool {
    status.split_whitespace().nth(1) == Some("NP")
}

pub fn change_own(current: &str, new: &str) -> Result<(), String> {
    talk(Command::new(PASSWD), current, new)
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
    let mut talk = Dialogue::default();
    let mut file = std::fs::File::from(master);
    let mut buf = Zeroizing::new([0u8; 512]);
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
        let read = match file.read(&mut buf[..]) {
            Ok(0) | Err(_) => break None,
            Ok(r) => r,
        };
        last = Instant::now();
        let text = Zeroizing::new(String::from_utf8_lossy(&buf[..read]).into_owned());
        match talk.feed(&text) {
            Step::Wait => {}
            Step::Answer(ask) => {
                wait_echo_off(&file);
                let answer = match ask {
                    Ask::Current => current,
                    Ask::New => new,
                };
                let mut line = Zeroizing::new(Vec::with_capacity(answer.len() + 1));
                line.extend_from_slice(answer.as_bytes());
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

/// Waits up to [`ECHO_WAIT`] for the terminal to stop echoing: `passwd` turns echo off with
/// `TCSAFLUSH` after its prompt, which echoes and then drops an answer written before then.
fn wait_echo_off(master: &std::fs::File) {
    let end = Instant::now() + ECHO_WAIT;
    loop {
        let mut t = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: tcgetattr fills the termios on success; on a pty master Linux reports the
        // pair's settings, which are the slave's.
        let echo = unsafe {
            libc::tcgetattr(master.as_raw_fd(), t.as_mut_ptr()) == 0
                && t.assume_init().c_lflag & libc::ECHO != 0
        };
        if !echo || Instant::now() >= end {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
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
    fn an_account_without_a_password_gets_the_new_one_twice() {
        let mut c = Command::new("sh");
        c.arg("-c").arg(
            "printf 'Changing password for jo.\\nNew password: '; read b; \
             printf 'Retype new password: '; read c; \
             if [ \"$b\" = 'new pw' ] && [ \"$c\" = 'new pw' ]; then \
               echo 'passwd: password updated successfully'; \
             else echo 'passwd: Authentication token manipulation error'; exit 10; fi",
        );
        assert_eq!(talk(c, "", "new pw"), Ok(()));
    }

    /// A stand-in for passwd that, like PAM's terminal conversation, throws away input that
    /// came before it turned echo off after each prompt.
    fn slow_echo_passwd() -> Command {
        let mut c = Command::new("bash");
        c.arg("-c").arg(
            "ask() { printf '%s' \"$1\"; sleep 0.3; while read -r -t 0.05 _; do :; done; \
                     stty -echo; read -r \"$2\"; stty echo; printf '\\n'; }; \
             ask 'Current password: ' a; ask 'New password: ' b; ask 'Retype new password: ' c; \
             if [ \"$a\" = old ] && [ \"$b\" = \"$c\" ]; then \
               echo 'passwd: password updated successfully'; \
             else echo 'passwd: Authentication token manipulation error'; exit 10; fi",
        );
        c
    }

    #[test]
    fn each_answer_waits_until_passwd_has_turned_echo_off() {
        let start = Instant::now();
        assert_eq!(talk(slow_echo_passwd(), "old", "new pw"), Ok(()));
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
        assert_eq!(d.feed("word: "), Step::Answer(Ask::Current));
        assert_eq!(d.feed("\r\nNew password: "), Step::Answer(Ask::New));
        assert_eq!(d.feed("\r\nRetype new password: "), Step::Answer(Ask::New));
        assert_eq!(
            d.feed("\r\npasswd: password updated successfully\r\n"),
            Step::Wait
        );
    }

    #[test]
    fn an_account_without_a_password_is_asked_only_for_the_new_one() {
        let mut d = Dialogue::default();
        assert_eq!(
            d.feed("Changing password for jo.\nNew password: "),
            Step::Answer(Ask::New)
        );
        assert_eq!(d.feed("\nRetype new password: "), Step::Answer(Ask::New));
        assert_eq!(
            d.feed("\npasswd: password updated successfully\n"),
            Step::Wait
        );
    }

    #[test]
    fn passwd_status_says_none_only_for_np() {
        assert!(says_none("jo NP 1970-01-01 -1 -1 -1 -1\n"));
        for line in [
            "jo P 1970-01-01 -1 -1 -1 -1\n",
            "jo L 2026-11-19 0 99999 7 -1\n",
            "jo NP",
            "",
        ] {
            assert_eq!(says_none(line), line == "jo NP", "{line:?}");
        }
    }

    #[test]
    fn a_prompt_for_anything_else_is_refused_unanswered() {
        let mut d = Dialogue::default();
        assert_eq!(
            d.feed("Verification code: "),
            Step::Refused("The password cannot be changed here.".into())
        );
        let mut d = Dialogue::default();
        d.feed("Current password: ");
        assert_eq!(
            d.feed("\nCurrent password: "),
            Step::Refused("The password cannot be changed here.".into()),
            "the current password is typed once"
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
    fn a_new_password_asked_again_before_the_retype_is_a_refusal_with_its_reason() {
        let mut d = Dialogue::default();
        d.feed("Current password: ");
        d.feed("\nNew password: ");
        assert_eq!(
            d.feed("\nBAD PASSWORD: it is based on a dictionary word\nNew password: "),
            Step::Refused("It is based on a dictionary word.".into())
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
