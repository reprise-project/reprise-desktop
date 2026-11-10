//! The lock screen's PAM password check, run in a helper child beside the fingerprint's, and what
//! a fingerprint line does to the screen. The verifier zeroes the copies of the password it owns.

use reprisekit::logon::Logon;
use repriselogin::add_prompt_lines;
use reprisepam::ffi;
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// Shown after PAM rejects the password without saying why.
pub const INCORRECT: &str = "The password is incorrect. Please retry.";
/// Shown when the password could not be checked at all (the helper did not start, died or hung).
pub const UNAVAILABLE: &str = "The password could not be checked. Please retry.";
/// The notice after an unlock with a password that must be changed.
pub const EXPIRED_SUMMARY: &str = "Your password has expired";
pub const EXPIRED_BODY: &str =
    "Change it now. The next login to a new session will ask for a new password.";
/// Shown when the fingerprint conversation ended with PAM_MAXTRIES: too
/// many fingers did not match.
pub const FINGER_TOO_MANY: &str = "Too many fingerprint attempts. Type your password to unlock.";

/// The longest answer the password conversation reads, in bytes: libpam's `PAM_MAX_RESP_SIZE`.
/// The lock's field stops there.
pub const INPUT_CAPACITY: usize = reprisekit::logon::UNLOCK_MAX;

pub use reprisekit::logon::Prompt;

/// Apply one line of the fingerprint conversation to the lock screen `m`. Returns true only for
/// PAM's success, which unlocks. The typed password is left alone: a finger that did not match
/// says nothing about it.
pub fn finger(m: &mut Logon, line: crate::lockfinger::Line) -> bool {
    use crate::lockfinger::Line;
    match line {
        Line::Info(t) => m.hint = Some(t),
        Line::Error(t) => m.info(t),
        Line::Prompt(_) => {}
        Line::Result(rc) => {
            m.hint = None;
            if rc == ffi::PAM_SUCCESS {
                return true;
            }
            if rc == ffi::PAM_MAXTRIES {
                m.info(FINGER_TOO_MANY.into());
            }
        }
    }
    false
}

/// A verifier's answer: `Ok` to unlock, or `Err` with the text PAM sent with the rejection (empty
/// for a plain wrong password), cleaned by `repriselogin::add_prompt_lines`.
pub type Verdict = Result<Passed, Vec<String>>;

/// A right password; `Expired` when the account step says it must be changed at the next login.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Passed {
    Current,
    Expired,
}

/// Answers a PAM prompt after the password: its text and whether it
/// echoes, to the answer, or None to end the conversation.
pub type Ask<'a> = dyn FnMut(&str, bool) -> Option<Zeroizing<String>> + 'a;

pub trait Verifier {
    fn verify(&self, user: &str, password: &str, ask: &mut Ask<'_>) -> Verdict;
}

pub fn failure_text(notices: &[String]) -> String {
    if notices.is_empty() {
        INCORRECT.into()
    } else {
        notices.join("\n")
    }
}

/// Failed checks before the back-off starts, its longest wait, and how long after its last
/// failure it is forgotten.
const FREE_FAILURES: u32 = 3;
const MAX_WAIT: Duration = Duration::from_secs(60);
const FORGET: Duration = Duration::from_secs(1200);

/// Failed Unlock checks: after the fourth each next check waits, from 2 s doubling up to `MAX_WAIT`,
/// until a success or `FORGET` without a failure. A check counts as failed from its start.
#[derive(Debug, Default)]
pub struct Backoff {
    failures: u32,
    last: Option<Instant>,
}

impl Backoff {
    /// How long the next check still waits at `now`.
    pub fn wait(&self, now: Instant) -> Duration {
        let Some(last) = self.last.filter(|&l| now.duration_since(l) < FORGET) else {
            return Duration::ZERO;
        };
        (last + wait_after(self.failures)).saturating_duration_since(now)
    }

    pub fn failed(&mut self, now: Instant) {
        if self.last.is_none_or(|l| now.duration_since(l) >= FORGET) {
            self.failures = 0;
        }
        self.failures += 1;
        self.last = Some(now);
    }

    pub fn succeeded(&mut self) {
        *self = Backoff::default();
    }
}

fn wait_after(failures: u32) -> Duration {
    match failures.checked_sub(FREE_FAILURES) {
        None | Some(0) => Duration::ZERO,
        Some(n) => Duration::from_secs(1u64 << n.min(6)).min(MAX_WAIT),
    }
}

/// Shown when a check is refused for `wait`, rounded up to whole seconds.
pub fn backoff_text(wait: Duration) -> String {
    let secs = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
    let unit = if secs == 1 { "second" } else { "seconds" };
    format!("Too many incorrect passwords. Wait {secs} {unit} and retry.")
}

/// PAM through libpam with `service`; with `account`, its account stack too, which passes a
/// password that must be changed as [`Passed::Expired`].
pub struct PamVerifier {
    pub service: &'static str,
    pub account: bool,
}

impl PamVerifier {
    /// The lock screen's service: `reprise` when our own file is installed
    /// under `/etc/pam.d`, otherwise `login`, which every distribution
    /// ships, so an unpackaged build can still be unlocked (on Arch that
    /// stack runs pam_nologin in `auth`, so /run/nologin blocks it).
    pub fn lock_service() -> &'static str {
        if Path::new("/etc/pam.d/reprise").exists() {
            "reprise"
        } else {
            "login"
        }
    }
}

impl Verifier for PamVerifier {
    fn verify(&self, user: &str, password: &str, ask: &mut Ask<'_>) -> Verdict {
        // The console's unlock runs no account stack: an expired password, /run/nologin or a time
        // rule would fail after the right password and leave the live session behind it unreachable.
        let service = self.service;
        let (Ok(cservice), Ok(cuser)) = (CString::new(service), CString::new(user)) else {
            return Err(Vec::new());
        };
        let mut creds = Credentials::new(cuser, password, ask);
        let conv = ffi::pam_conv {
            conv: Some(converse),
            appdata_ptr: &mut creds as *mut Credentials<'_> as *mut c_void,
        };
        let mut handle: *mut ffi::pam_handle_t = std::ptr::null_mut();
        // SAFETY: every pointer is to a live local or a NUL-terminated
        // CString; `creds` outlives the handle, which pam_end closes below.
        let rc =
            unsafe { ffi::pam_start(cservice.as_ptr(), creds.user.as_ptr(), &conv, &mut handle) };
        if rc != ffi::PAM_SUCCESS || handle.is_null() {
            log::error!(
                "pam: cannot start service {service}: {}",
                pam_text(handle, rc)
            );
            return Err(Vec::new());
        }
        let checked = authenticate_then_account(
            self.account,
            // SAFETY: `handle` is the live handle pam_start just returned.
            || unsafe { ffi::pam_authenticate(handle, 0) },
            // SAFETY: as above; pam_authenticate succeeded on it.
            || unsafe { ffi::pam_acct_mgmt(handle, 0) },
        );
        let rc = match checked {
            Ok(Passed::Current) => ffi::PAM_SUCCESS,
            Ok(Passed::Expired) => ffi::PAM_NEW_AUTHTOK_REQD,
            Err(rc) => rc,
        };
        if checked.is_err() {
            log::warn!(
                "pam: refused {user}: {} {:?}",
                pam_text(handle, rc),
                creds.notices.borrow()
            );
        }
        // SAFETY: `handle` is live and not used after this call.
        unsafe { ffi::pam_end(handle, rc) };
        checked.map_err(|_| creds.notices.take())
    }
}

/// `authenticate`, then with `account` `acct_mgmt`: the first failure's code, or the password's
/// standing.
fn authenticate_then_account(
    account: bool,
    authenticate: impl FnOnce() -> c_int,
    acct_mgmt: impl FnOnce() -> c_int,
) -> Result<Passed, c_int> {
    match authenticate() {
        ffi::PAM_SUCCESS if account => match acct_mgmt() {
            ffi::PAM_SUCCESS => Ok(Passed::Current),
            // The password is right and only its age is at fault; the session it unlocks runs.
            ffi::PAM_NEW_AUTHTOK_REQD => Ok(Passed::Expired),
            rc => Err(rc),
        },
        ffi::PAM_SUCCESS => Ok(Passed::Current),
        rc => Err(rc),
    }
}

/// What the conversation answers with, across every `converse` call of
/// one `pam_authenticate`. The password is zeroed with it. `notices`
/// collects the error and info text PAM sends for the screen.
struct Credentials<'a> {
    user: CString,
    password: Zeroizing<Vec<u8>>,
    notices: RefCell<Vec<String>>,
    /// A prompt has been answered.
    prompted: Cell<bool>,
    /// The password has been handed out.
    password_sent: Cell<bool>,
    ask: RefCell<&'a mut Ask<'a>>,
}

impl<'a> Credentials<'a> {
    fn new(user: CString, password: &str, ask: &'a mut Ask<'a>) -> Credentials<'a> {
        Credentials {
            user,
            password: Zeroizing::new(password.as_bytes().to_vec()),
            notices: RefCell::new(Vec::new()),
            prompted: Cell::new(false),
            password_sent: Cell::new(false),
            ask: RefCell::new(ask),
        }
    }

    /// The answer to the next prompt: the login name for a login prompt
    /// that comes first, the password for the first blind one, and `ask`'s
    /// answer to any further one (a one-time code, a second password).
    fn answer(&self, echo: bool, text: &str) -> Option<Zeroizing<Vec<u8>>> {
        let first = !self.prompted.replace(true);
        if echo && first && is_login_prompt(text) {
            return Some(Zeroizing::new(self.user.as_bytes().to_vec()));
        }
        if !echo && !self.password_sent.replace(true) {
            return Some(self.password.clone());
        }
        let a = (self.ask.borrow_mut())(text, echo)?;
        Some(Zeroizing::new(a.as_bytes().to_vec()))
    }
}

/// libpam's own user prompt is "login:"; modules ask "Username:".
fn is_login_prompt(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    ["login:", "username:", "user name:"].contains(&t.as_str())
}

fn pam_text(handle: *mut ffi::pam_handle_t, rc: c_int) -> String {
    if handle.is_null() {
        return format!("code {rc}");
    }
    // SAFETY: pam_strerror returns a static string for any code.
    unsafe { CStr::from_ptr(ffi::pam_strerror(handle, rc)) }
        .to_string_lossy()
        .into_owned()
}

/// libpam's conversation callback. Each answer is a malloc'd copy libpam
/// frees itself; the password copy is one more place the secret sits
/// until then (see the module doc).
unsafe extern "C" fn converse(
    n: c_int,
    msgs: *mut *const ffi::pam_message,
    out: *mut *mut ffi::pam_response,
    data: *mut c_void,
) -> c_int {
    if n <= 0 || msgs.is_null() || out.is_null() || data.is_null() {
        return ffi::PAM_CONV_ERR;
    }
    // SAFETY: `data` is the `appdata_ptr` from `verify`: a live `Credentials`
    // that outlives the PAM handle.
    let creds = unsafe { &*(data as *const Credentials<'_>) };
    let n = n as usize;
    // SAFETY: plain allocation; the result is null-checked below.
    let resp = unsafe { libc::calloc(n, std::mem::size_of::<ffi::pam_response>()) }
        as *mut ffi::pam_response;
    if resp.is_null() {
        return ffi::PAM_BUF_ERR;
    }
    for i in 0..n {
        // Linux-PAM hands an array of messages; `msgs` points at its first.
        // SAFETY: libpam passes `n` (> 0) live messages behind non-null `msgs`.
        let m = unsafe { &**msgs.add(i) };
        let answer = match m.msg_style {
            style @ (ffi::PAM_PROMPT_ECHO_ON | ffi::PAM_PROMPT_ECHO_OFF) => {
                let text = match m.msg.is_null() {
                    true => Default::default(),
                    // SAFETY: libpam's message text is a NUL-terminated string.
                    false => unsafe { CStr::from_ptr(m.msg) }.to_string_lossy(),
                };
                let Some(a) = creds.answer(style == ffi::PAM_PROMPT_ECHO_ON, &text) else {
                    log::warn!("pam: no answer for a further prompt, ending the conversation");
                    // SAFETY: the first `i` answers are null or calloc'd
                    // buffers from this loop, none handed to libpam.
                    unsafe { free_answers(resp, i) };
                    return ffi::PAM_CONV_ERR;
                };
                Some(a)
            }
            ffi::PAM_ERROR_MSG | ffi::PAM_TEXT_INFO if !m.msg.is_null() => {
                // SAFETY: libpam's message text is a NUL-terminated string.
                let text = unsafe { CStr::from_ptr(m.msg) }.to_string_lossy();
                add_prompt_lines(&mut creds.notices.borrow_mut(), &text);
                None
            }
            _ => None,
        };
        if let Some(a) = answer {
            // SAFETY: plain allocation; the result is null-checked below.
            let buf = unsafe { libc::calloc(a.len() + 1, 1) } as *mut u8;
            if buf.is_null() {
                // SAFETY: the first `i` answers are null or calloc'd
                // buffers from this loop, none handed to libpam.
                unsafe { free_answers(resp, i) };
                return ffi::PAM_BUF_ERR;
            }
            // SAFETY: `buf` holds `a.len() + 1` bytes and `i < n` is in bounds
            // of `resp`.
            unsafe {
                std::ptr::copy_nonoverlapping(a.as_ptr(), buf, a.len());
                (*resp.add(i)).resp = buf as *mut c_char;
            }
        }
    }
    // SAFETY: `out` is non-null (checked above) and is libpam's result slot.
    unsafe { *out = resp };
    ffi::PAM_SUCCESS
}

/// # Safety
/// `resp` is the array `converse` calloc'd, not yet handed to libpam, and
/// each of its first `i` answers is null or a calloc'd buffer.
unsafe fn free_answers(resp: *mut ffi::pam_response, i: usize) {
    // SAFETY: the caller's contract.
    unsafe {
        for j in 0..i {
            libc::free((*resp.add(j)).resp as *mut c_void);
        }
        libc::free(resp as *mut c_void);
    }
}

/// Login name of the user running the compositor: the passwd entry for
/// the uid, else `$USER`, else `$LOGNAME`, else "". The lookup may block on
/// nsswitch, so `run` makes it at startup and a name found is kept for
/// every later call. An empty name means PAM can never accept anything, so
/// `lockview::lock` logs it as an error before engaging.
pub fn current_user() -> String {
    static FOUND: OnceLock<String> = OnceLock::new();
    if let Some(name) = FOUND.get() {
        return name.clone();
    }
    let name = pick_user(
        passwd_name(),
        std::env::var("USER").ok(),
        std::env::var("LOGNAME").ok(),
    );
    if !name.is_empty() {
        let _ = FOUND.set(name.clone());
    }
    name
}

/// The full name the lock's tile shows, from the passwd entry; looked up once, at startup, as
/// [`current_user`] is. Empty when there is none.
pub fn real_name() -> String {
    static FOUND: OnceLock<String> = OnceLock::new();
    let found = FOUND.get_or_init(|| {
        // SAFETY: getuid has no preconditions.
        let uid = unsafe { libc::getuid() };
        reprisefs::sysinfo::full_name(uid).unwrap_or_default()
    });
    found.clone()
}

/// Where the login screen lists `user` among `source`'s accounts; 0 when it isn't listed.
pub fn listed_at(source: repriselogin::users::Source, user: &str) -> usize {
    (source.listed().iter())
        .position(|e| e.name == user)
        .unwrap_or(0)
}

fn pick_user(passwd: Option<String>, user: Option<String>, logname: Option<String>) -> String {
    [passwd, user, logname]
        .into_iter()
        .flatten()
        .find(|s| !s.is_empty())
        .unwrap_or_default()
}

/// The uid's login name from the passwd database. The buffer grows on
/// ERANGE (a long LDAP entry) up to 64 KiB.
fn passwd_name() -> Option<String> {
    let mut buf = vec![0u8; 1024];
    loop {
        // SAFETY: `libc::passwd` is a plain C struct, all-zeroes is a valid value.
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: getpwuid_r only writes through the three pointers we pass,
        // each of which points at a live local. `buf` outlives the
        // `pwd.pw_name` read below, which points into it.
        let rc = unsafe {
            libc::getpwuid_r(
                libc::getuid(),
                &mut pwd,
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.len(),
                &mut result,
            )
        };
        if rc == libc::ERANGE && buf.len() < 64 * 1024 {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        if rc != 0 || result.is_null() || pwd.pw_name.is_null() {
            return None;
        }
        // SAFETY: getpwuid_r succeeded, so `pw_name` is a NUL-terminated
        // string in `buf`, which is still in scope.
        let name = unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) }
            .to_string_lossy()
            .into_owned();
        return Some(name);
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn the_backoff_doubles_from_the_fourth_failure_to_a_minute_and_forgets_after_twenty_minutes() {
        let t0 = Instant::now();
        let mut b = Backoff::default();
        let waits: Vec<u64> = (0..10)
            .map(|_| {
                b.failed(t0);
                b.wait(t0).as_secs()
            })
            .collect();
        assert_eq!(waits, [0, 0, 0, 2, 4, 8, 16, 32, 60, 60]);
        assert_eq!(b.wait(t0 + Duration::from_secs(59)).as_secs(), 1);
        let later = t0 + Duration::from_secs(1200);
        assert_eq!(b.wait(later), Duration::ZERO);
        b.failed(later);
        assert_eq!(b.wait(later), Duration::ZERO, "the count starts over");
        b.succeeded();
        assert_eq!(b.failures, 0);
    }

    #[test]
    fn the_backoff_text_rounds_up_to_whole_seconds() {
        assert_eq!(
            backoff_text(Duration::from_millis(1500)),
            "Too many incorrect passwords. Wait 2 seconds and retry."
        );
        assert!(backoff_text(Duration::from_millis(200)).contains("Wait 1 second and"));
    }

    use super::*;

    #[test]
    fn the_account_stack_runs_only_when_asked_and_after_a_good_password() {
        let account_ran = std::cell::Cell::new(false);
        let acct = |rc| {
            let account_ran = &account_ran;
            move || {
                account_ran.set(true);
                rc
            }
        };
        const ACCT_EXPIRED: c_int = 13;
        let ok = || ffi::PAM_SUCCESS;
        assert_eq!(
            authenticate_then_account(true, ok, acct(ACCT_EXPIRED)),
            Err(ACCT_EXPIRED)
        );
        assert!(account_ran.take());
        assert_eq!(
            authenticate_then_account(true, ok, acct(ffi::PAM_SUCCESS)),
            Ok(Passed::Current)
        );
        assert!(account_ran.take());
        let wrong = || ffi::PAM_AUTH_ERR;
        assert_eq!(
            authenticate_then_account(true, wrong, acct(ffi::PAM_SUCCESS)),
            Err(ffi::PAM_AUTH_ERR)
        );
        assert!(!account_ran.take(), "a wrong password ends it");
        assert_eq!(
            authenticate_then_account(false, ok, acct(ACCT_EXPIRED)),
            Ok(Passed::Current),
            "unlocking ignores the account"
        );
        assert!(!account_ran.take());
    }

    #[test]
    fn a_right_password_that_must_be_changed_unlocks_and_says_so() {
        let ok = || ffi::PAM_SUCCESS;
        let expired = || ffi::PAM_NEW_AUTHTOK_REQD;
        assert_eq!(
            authenticate_then_account(true, ok, expired),
            Ok(Passed::Expired)
        );
        assert_eq!(
            authenticate_then_account(true, expired, || ffi::PAM_SUCCESS),
            Err(ffi::PAM_NEW_AUTHTOK_REQD),
            "only the account step's word counts"
        );
        assert_eq!(
            authenticate_then_account(true, || ffi::PAM_AUTH_ERR, expired),
            Err(ffi::PAM_AUTH_ERR)
        );
    }

    fn screen() -> Logon {
        let user = reprisekit::logon::LogonUser {
            name: "someone".into(),
            real_name: String::new(),
            picture: None,
            note: None,
            stock: 0,
        };
        Logon::locked(reprisekit::interface::Interface::Classic, user, false, true)
    }

    #[test]
    fn the_finger_prompt_shows_until_pam_answers_and_only_success_unlocks() {
        use crate::lockfinger::Line;
        let mut m = screen();
        m.key(reprisekit::logon::LogonKey::Text("hunt"));
        let prompt = "Place your finger on the fingerprint reader";
        assert!(!finger(&mut m, Line::Info(prompt.into())));
        assert_eq!(m.said().as_deref(), Some(prompt));
        assert_eq!(m.message, None);
        assert!(!finger(
            &mut m,
            Line::Error("Failed to match fingerprint".into())
        ));
        assert_eq!(m.message.as_deref(), Some("Failed to match fingerprint"));
        assert_eq!(m.password.text, "hunt", "a miss keeps what was typed");
        m.message = None;
        assert_eq!(m.said().as_deref(), Some(prompt), "the prompt shows again");
        assert!(finger(&mut m, Line::Result(ffi::PAM_SUCCESS)));
    }

    #[test]
    fn a_failed_finger_conversation_never_unlocks_and_leaves_the_password() {
        use crate::lockfinger::Line;
        for rc in [
            ffi::PAM_AUTHINFO_UNAVAIL,
            7, // PAM_AUTH_ERR: pam_faillock's preauth on a locked account
            ffi::PAM_CONV_ERR,
            -1,
        ] {
            let mut m = typed_screen();
            finger(&mut m, Line::Info("Swipe your finger".into()));
            finger(&mut m, Line::Error("Failed to match fingerprint".into()));
            m.message = None;
            assert!(
                !finger(&mut m, Line::Result(rc)),
                "PAM code {rc} does not unlock"
            );
            assert_eq!(m.hint, None);
            assert_eq!(m.message, None, "code {rc} adds no text of its own");
            assert_eq!(m.password.text, "hunter", "code {rc} leaves the password");
        }
        let mut m = typed_screen();
        finger(&mut m, Line::Info("Swipe your finger".into()));
        assert!(!finger(&mut m, Line::Result(ffi::PAM_MAXTRIES)));
        assert_eq!(m.hint, None);
        assert_eq!(m.message.as_deref(), Some(FINGER_TOO_MANY));
        assert_eq!(m.password.text, "hunter", "too many fingers leave it too");
    }

    /// The screen with "hunter" typed into its field.
    fn typed_screen() -> Logon {
        let mut m = screen();
        m.password
            .key(reprisekit::textedit::EditKey::Insert("hunter".into()));
        m
    }

    #[test]
    fn the_lock_finds_its_user_where_the_login_screen_lists_it() {
        let at = |user| listed_at(repriselogin::users::sample(), user);
        assert_eq!(at("zoe"), 2);
        assert_eq!(at("anna"), 0);
        assert_eq!(at("nobody-listed"), 0);
    }

    #[test]
    fn current_user_is_non_empty() {
        assert!(!current_user().is_empty());
        assert_eq!(passwd_name().as_deref(), Some(current_user().as_str()));
    }

    #[test]
    fn the_environment_stands_in_for_a_missing_passwd_entry() {
        let s = |v: &str| Some(v.to_string());
        assert_eq!(pick_user(s("pw"), s("env"), s("log")), "pw");
        assert_eq!(pick_user(None, s("env"), s("log")), "env");
        assert_eq!(pick_user(Some(String::new()), None, s("log")), "log");
        assert_eq!(pick_user(None, None, None), "");
    }

    fn creds<'a>(ask: &'a mut Ask<'a>) -> Credentials<'a> {
        Credentials::new(CString::new("someone").unwrap(), "hunter2", ask)
    }

    fn no_answer(_: &str, _: bool) -> Option<Zeroizing<String>> {
        None
    }

    fn msg(msg_style: c_int, text: &CStr) -> ffi::pam_message {
        ffi::pam_message {
            msg_style,
            msg: text.as_ptr(),
        }
    }

    fn ask(creds: &mut Credentials<'_>, msgs: &[ffi::pam_message]) -> (c_int, Vec<Option<String>>) {
        let mut ptrs: Vec<*const ffi::pam_message> = msgs.iter().map(|m| m as *const _).collect();
        let mut out: *mut ffi::pam_response = std::ptr::null_mut();
        // SAFETY: the arguments are what libpam passes: `ptrs.len()` live
        // messages, a live result slot and a live `Credentials`.
        let rc = unsafe {
            converse(
                ptrs.len() as c_int,
                ptrs.as_mut_ptr(),
                &mut out,
                creds as *mut Credentials<'_> as *mut c_void,
            )
        };
        if out.is_null() {
            return (rc, Vec::new());
        }
        // SAFETY: a non-null `out` is `converse`'s calloc'd array of
        // `ptrs.len()` answers, each null or a calloc'd NUL-terminated
        // string, and this caller owns it as libpam would.
        let answers = unsafe {
            let answers = (0..ptrs.len())
                .map(|i| {
                    let r = (*out.add(i)).resp;
                    let a = (!r.is_null()).then(|| CStr::from_ptr(r).to_str().unwrap().to_owned());
                    libc::free(r as *mut c_void);
                    a
                })
                .collect();
            libc::free(out as *mut c_void);
            answers
        };
        (rc, answers)
    }

    /// The conversation answers an echo prompt with the login, a blind
    /// one with the password, and keeps error and info text unanswered.
    #[test]
    fn converse_answers_the_prompts_and_keeps_the_notices() {
        let mut none = no_answer;
        let mut creds = creds(&mut none);
        let msgs = [
            msg(ffi::PAM_PROMPT_ECHO_ON, c"login:"),
            msg(ffi::PAM_PROMPT_ECHO_OFF, c"Password:"),
            msg(ffi::PAM_TEXT_INFO, c"note"),
            msg(ffi::PAM_ERROR_MSG, c"Account locked."),
        ];
        let (rc, answers) = ask(&mut creds, &msgs);
        assert_eq!(rc, ffi::PAM_SUCCESS);
        assert_eq!(
            answers,
            [Some("someone".into()), Some("hunter2".into()), None, None]
        );
        assert_eq!(creds.notices.take(), ["note", "Account locked."]);
    }

    #[test]
    fn converse_asks_for_an_echoed_first_prompt_that_is_no_login_prompt() {
        let mut asked = Vec::new();
        let mut answer = |text: &str, echo: bool| {
            asked.push((text.to_string(), echo));
            Some(Zeroizing::new("123456".to_string()))
        };
        let mut creds = creds(&mut answer);
        let msgs = [
            msg(ffi::PAM_PROMPT_ECHO_ON, c"Verification code: "),
            msg(ffi::PAM_PROMPT_ECHO_OFF, c"Password:"),
        ];
        let (rc, answers) = ask(&mut creds, &msgs);
        assert_eq!(rc, ffi::PAM_SUCCESS);
        assert_eq!(answers, [Some("123456".into()), Some("hunter2".into())]);
        drop(creds);
        assert_eq!(asked, [("Verification code: ".to_string(), true)]);
    }

    /// A prompt after the password that `ask` has no answer for ends the
    /// conversation with PAM_CONV_ERR instead of getting the password or
    /// the login name.
    #[test]
    fn converse_refuses_a_prompt_after_the_password_without_an_answer() {
        for second in [ffi::PAM_PROMPT_ECHO_OFF, ffi::PAM_PROMPT_ECHO_ON] {
            let mut none = no_answer;
            let mut creds = creds(&mut none);
            let (rc, answers) = ask(&mut creds, &[msg(ffi::PAM_PROMPT_ECHO_OFF, c"Password:")]);
            assert_eq!(rc, ffi::PAM_SUCCESS);
            assert_eq!(answers, [Some("hunter2".into())]);
            let (rc, answers) = ask(&mut creds, &[msg(second, c"One-time code:")]);
            assert_eq!(rc, ffi::PAM_CONV_ERR);
            assert_eq!(answers, []);
        }
        // Within one round too, and after the login name.
        let mut none = no_answer;
        let mut creds = creds(&mut none);
        let msgs = [
            msg(ffi::PAM_PROMPT_ECHO_ON, c"login:"),
            msg(ffi::PAM_PROMPT_ECHO_ON, c"Code:"),
        ];
        assert_eq!(ask(&mut creds, &msgs), (ffi::PAM_CONV_ERR, Vec::new()));
    }

    /// Each prompt after the password goes to `ask` with its text and echo,
    /// and gets `ask`'s answer.
    #[test]
    fn converse_asks_for_each_further_prompt() {
        let mut asked = Vec::new();
        let mut answer = |text: &str, echo: bool| {
            asked.push((text.to_string(), echo));
            Some(Zeroizing::new(format!("answer {}", asked.len())))
        };
        let mut creds = creds(&mut answer);
        let msgs = [
            msg(ffi::PAM_PROMPT_ECHO_OFF, c"Password:"),
            msg(ffi::PAM_PROMPT_ECHO_ON, c"Verification code:"),
        ];
        let (rc, answers) = ask(&mut creds, &msgs);
        assert_eq!(rc, ffi::PAM_SUCCESS);
        assert_eq!(answers, [Some("hunter2".into()), Some("answer 1".into())]);
        let (rc, answers) = ask(&mut creds, &[msg(ffi::PAM_PROMPT_ECHO_OFF, c"PIN:")]);
        assert_eq!(rc, ffi::PAM_SUCCESS);
        assert_eq!(answers, [Some("answer 2".into())]);
        drop(creds);
        assert_eq!(
            asked,
            [("Verification code:".into(), true), ("PIN:".into(), false)]
        );
    }

    #[test]
    fn a_rejection_shows_what_pam_sent_else_the_incorrect_text() {
        assert_eq!(failure_text(&[]), INCORRECT);
        let locked = ["Account locked.".to_string(), "(10 minutes left)".into()];
        assert_eq!(failure_text(&locked), "Account locked.\n(10 minutes left)");
    }

    #[test]
    fn pam_service_prefers_our_own_file() {
        let s = PamVerifier::lock_service();
        assert!(s == "reprise" || s == "login");
        if std::path::Path::new("/etc/pam.d/reprise").exists() {
            assert_eq!(s, "reprise");
        } else {
            assert_eq!(s, "login");
        }
    }

    #[test]
    fn libpam_starts_a_handle_through_the_declared_bindings() {
        let service = CString::new(PamVerifier::lock_service()).unwrap();
        let user = CString::new("nobody").unwrap();
        let conv = ffi::pam_conv {
            conv: Some(converse),
            appdata_ptr: std::ptr::null_mut(),
        };
        let mut handle: *mut ffi::pam_handle_t = std::ptr::null_mut();
        // SAFETY: live CStrings and locals; `handle` is closed below and
        // never authenticated, so `converse` is not called.
        let rc = unsafe { ffi::pam_start(service.as_ptr(), user.as_ptr(), &conv, &mut handle) };
        assert_eq!(rc, ffi::PAM_SUCCESS, "pam_start returns PAM_SUCCESS");
        assert!(!handle.is_null(), "pam_start fills in a handle");
        let text = pam_text(handle, ffi::PAM_SUCCESS);
        assert!(
            !text.is_empty() && text != "code 0",
            "pam_strerror names the code"
        );
        // SAFETY: `handle` is live and not used after this call.
        assert_eq!(unsafe { ffi::pam_end(handle, rc) }, ffi::PAM_SUCCESS);
    }
}
