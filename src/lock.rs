//! Built-in lock screen: the dialog's state machine, the "Unlock Computer"
//! dialog layout and drawing, and the PAM password check, which runs in a
//! helper child beside the fingerprint's (see `lockfinger.rs`). Nothing in
//! here touches the scene or the seat; `lockview.rs` does that.
//!
//! `LockState` and the verifier zero the copies they own. libpam wipes its
//! own, except in modules that copy the answer elsewhere.

use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::{Color, Metrics, Theme};
use reprisekit::classic::indicator;
use reprisekit::frame::{ButtonSet, Chrome, FrameGeometry, FrameState};
use reprisekit::look::Look;
use reprisekit::look::widgets::{draw_button, draw_edit_field};
use reprisekit::pointer::Interaction;
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use zeroize::{Zeroize, Zeroizing};

/// Shown after PAM rejects the password without saying why.
pub const INCORRECT: &str = "The password is incorrect. Please retry.";
/// Shown when the password could not be checked at all (the helper did not start, died or hung).
pub const UNAVAILABLE: &str = "The password could not be checked. Please retry.";
/// Shown on the message area's first line while Caps Lock is on.
pub const CAPS_LOCK: &str = "Caps Lock is on.";
/// Shown when the fingerprint conversation ended with PAM_MAXTRIES: too
/// many fingers did not match.
pub const FINGER_TOO_MANY: &str = "Too many fingerprint attempts. Type your password to unlock.";

/// Lines the message area holds. PAM text past them is not kept.
const MESSAGE_LINES: usize = 2;
/// Characters kept of one PAM text line.
const NOTICE_CHARS: usize = 200;

/// Password field reservation in bytes, see `LockState::new`. Typing past
/// it is refused. 512 is libpam's `PAM_MAX_RESP_SIZE`.
pub const INPUT_CAPACITY: usize = 512;

/// A key the lock dialog cares about. Text is whatever xkb produced for the
/// key in the current layout, so the password follows the configured keymap;
/// it is borrowed so no copy of a password character outlives the press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockKey<'a> {
    Text(&'a str),
    Backspace,
    Enter,
    Escape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockButton {
    Ok,
    Cancel,
}

/// The dialog's state. `input` is the password typed so far; it is zeroed
/// whenever it is cleared. `verifying` gates every edit while the password
/// helper is busy. `prompt` is a further PAM prompt (a one-time code) the
/// field answers in place of the password. `finger` is the fingerprint
/// conversation's standing prompt (its last PAM info text) while it runs,
/// shown when there is no message. `caps_lock` mirrors the keyboard's;
/// `layout` is the active keyboard layout's short name while more than one
/// is configured.
#[derive(Default)]
pub struct LockState {
    pub input: String,
    pub message: Option<String>,
    pub verifying: bool,
    pub prompt: Option<Prompt>,
    pub finger: Option<String>,
    pub caps_lock: bool,
    pub layout: Option<String>,
}

/// A PAM prompt after the password: its text labels the field, and `echo`
/// shows the answer as typed instead of as stars.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub text: String,
    pub echo: bool,
}

/// By hand so a `{:?}` in a log line never carries the password: only
/// its length is shown.
impl std::fmt::Debug for LockState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LockState")
            .field(
                "input",
                &format_args!("<{} chars>", self.input.chars().count()),
            )
            .field("message", &self.message)
            .field("verifying", &self.verifying)
            .field("prompt", &self.prompt)
            .field("finger", &self.finger)
            .field("caps_lock", &self.caps_lock)
            .field("layout", &self.layout)
            .finish()
    }
}

impl LockState {
    pub fn new() -> LockState {
        LockState {
            // Reserve up front and never grow: `zeroize` can only wipe the
            // live allocation, and a grown `String` leaves the old buffer,
            // holding a prefix of the password, on the heap. Zeroing keeps
            // the capacity, so this one reservation covers every retry.
            input: String::with_capacity(INPUT_CAPACITY),
            ..LockState::default()
        }
    }

    /// Wipe the password. `Zeroize for String` clears the length and keeps the
    /// capacity, so no separate `clear()` is needed.
    fn clear_input(&mut self) {
        self.input.zeroize();
    }

    pub fn reset(&mut self) {
        self.clear_input();
        self.message = None;
        self.verifying = false;
        self.prompt = None;
    }

    /// The conversation asks `prompt`: the field empties and answers it.
    pub fn ask(&mut self, prompt: Prompt) {
        self.clear_input();
        self.message = None;
        self.verifying = false;
        self.prompt = Some(prompt);
    }

    pub fn take_input(&mut self) -> Zeroizing<String> {
        Zeroizing::new(std::mem::replace(
            &mut self.input,
            String::with_capacity(INPUT_CAPACITY),
        ))
    }

    pub fn key(&mut self, key: LockKey<'_>) -> bool {
        if self.verifying {
            return false;
        }
        match key {
            LockKey::Text(s) => {
                if self.input.len() + s.len() <= INPUT_CAPACITY {
                    self.input.push_str(s);
                }
                false
            }
            LockKey::Backspace => {
                self.input.pop();
                false
            }
            LockKey::Enter => !self.input.is_empty(),
            LockKey::Escape => {
                self.clear_input();
                self.message = None;
                self.prompt = None;
                false
            }
        }
    }

    pub fn button(&mut self, b: LockButton) -> bool {
        if self.verifying {
            return false;
        }
        match b {
            LockButton::Ok => !self.input.is_empty(),
            LockButton::Cancel => {
                self.clear_input();
                self.prompt = None;
                false
            }
        }
    }

    pub fn fail_with(&mut self, message: &str) {
        self.clear_input();
        self.message = Some(message.into());
        self.verifying = false;
        self.prompt = None;
    }

    /// Apply one line of the fingerprint conversation. Returns true only
    /// for PAM's success, which unlocks. The typed password is left alone:
    /// a finger that did not match says nothing about it.
    pub fn finger(&mut self, line: crate::lockfinger::Line) -> bool {
        use crate::lockfinger::Line;
        match line {
            Line::Info(t) => self.finger = Some(t),
            Line::Error(t) => self.message = Some(t),
            Line::Prompt(_) => {}
            Line::Result(rc) => {
                self.finger = None;
                if rc == ffi::PAM_SUCCESS {
                    return true;
                }
                if rc == ffi::PAM_MAXTRIES {
                    self.message = Some(FINGER_TOO_MANY.into());
                }
            }
        }
        false
    }
}

/// A verifier's answer: `Ok` to unlock, or `Err` with the text PAM sent
/// along with the rejection (empty for a plain wrong password), already
/// cleaned by `add_notice`.
pub type Verdict = Result<(), Vec<String>>;

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

/// Add one PAM message to `notices`: a line per non-blank line of `raw`,
/// control characters turned to spaces, each cut to `NOTICE_CHARS`, and
/// at most `MESSAGE_LINES` lines in all.
pub fn add_notice(notices: &mut Vec<String>, raw: &str) {
    for line in raw.lines() {
        if notices.len() >= MESSAGE_LINES {
            return;
        }
        let line: String = line
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let line = line.trim();
        if !line.is_empty() {
            notices.push(line.chars().take(NOTICE_CHARS).collect());
        }
    }
}

pub fn clean_notice(raw: &str) -> Vec<String> {
    let mut v = Vec::new();
    add_notice(&mut v, raw);
    v
}

#[allow(non_camel_case_types)]
pub(crate) mod ffi {
    use std::ffi::{c_char, c_int, c_void};

    pub const PAM_SUCCESS: c_int = 0;
    pub const PAM_BUF_ERR: c_int = 5;
    pub const PAM_AUTH_ERR: c_int = 7;
    pub const PAM_AUTHINFO_UNAVAIL: c_int = 9;
    pub const PAM_MAXTRIES: c_int = 11;
    pub const PAM_CONV_ERR: c_int = 19;

    pub const PAM_PROMPT_ECHO_OFF: c_int = 1;
    pub const PAM_PROMPT_ECHO_ON: c_int = 2;
    pub const PAM_ERROR_MSG: c_int = 3;
    pub const PAM_TEXT_INFO: c_int = 4;

    pub const PAM_FAIL_DELAY: c_int = 10;

    #[repr(C)]
    pub struct pam_handle_t {
        _opaque: [u8; 0],
    }

    #[repr(C)]
    pub struct pam_message {
        pub msg_style: c_int,
        pub msg: *const c_char,
    }

    #[repr(C)]
    pub struct pam_response {
        pub resp: *mut c_char,
        pub resp_retcode: c_int,
    }

    pub type ConvFn = unsafe extern "C" fn(
        c_int,
        *mut *const pam_message,
        *mut *mut pam_response,
        *mut c_void,
    ) -> c_int;

    #[repr(C)]
    pub struct pam_conv {
        pub conv: Option<ConvFn>,
        pub appdata_ptr: *mut c_void,
    }

    #[link(name = "pam")]
    unsafe extern "C" {
        pub fn pam_start(
            service_name: *const c_char,
            user: *const c_char,
            pam_conversation: *const pam_conv,
            pamh: *mut *mut pam_handle_t,
        ) -> c_int;
        pub fn pam_authenticate(pamh: *mut pam_handle_t, flags: c_int) -> c_int;
        pub fn pam_end(pamh: *mut pam_handle_t, pam_status: c_int) -> c_int;
        pub fn pam_strerror(pamh: *mut pam_handle_t, errnum: c_int) -> *const c_char;
        pub fn pam_set_item(
            pamh: *mut pam_handle_t,
            item_type: c_int,
            item: *const c_void,
        ) -> c_int;
    }
}

/// PAM through libpam. The service is `reprise` when our own file is
/// installed under `/etc/pam.d`, otherwise `login`, which every
/// distribution ships, so an unpackaged build can still be unlocked (on
/// Arch that stack runs pam_nologin in `auth`, so /run/nologin blocks it).
pub struct PamVerifier;

impl PamVerifier {
    pub fn service() -> &'static str {
        if Path::new("/etc/pam.d/reprise").exists() {
            "reprise"
        } else {
            "login"
        }
    }
}

impl Verifier for PamVerifier {
    fn verify(&self, user: &str, password: &str, ask: &mut Ask<'_>) -> Verdict {
        // pam_authenticate alone, as swaylock and hyprlock do: pam_acct_mgmt
        // (an expired password, /run/nologin, a time rule) would fail after
        // the right password and read as "incorrect", with the live
        // session behind it unreachable.
        let service = PamVerifier::service();
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
        // SAFETY: `handle` is the live handle pam_start just returned.
        let rc = unsafe { ffi::pam_authenticate(handle, 0) };
        if rc != ffi::PAM_SUCCESS {
            log::warn!(
                "pam: authentication failed for {user}: {} {:?}",
                pam_text(handle, rc),
                creds.notices.borrow()
            );
        }
        // SAFETY: `handle` is live and not used after this call.
        unsafe { ffi::pam_end(handle, rc) };
        if rc == ffi::PAM_SUCCESS {
            Ok(())
        } else {
            Err(creds.notices.take())
        }
    }
}

/// What the conversation answers with, across every `converse` call of
/// one `pam_authenticate`. The password is zeroed with it. `notices`
/// collects the error and info text PAM sends for the dialog.
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
                add_notice(&mut creds.notices.borrow_mut(), &text);
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
/// the uid, else `$USER`, else `$LOGNAME`, else "". It may block on
/// nsswitch, so `Server::new` calls it once. An empty name means PAM can
/// never accept anything, so `lockview::lock` logs it as an error before
/// engaging.
pub fn current_user() -> String {
    pick_user(
        passwd_name(),
        std::env::var("USER").ok(),
        std::env::var("LOGNAME").ok(),
    )
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogLayout {
    pub size: (i32, i32),
    /// The frame a styled look draws around the body; None under Classic,
    /// which draws its own.
    pub frame: Option<FrameGeometry>,
    pub caption: Rect,
    pub user_box: Rect,
    pub field: Rect,
    pub message: Rect,
    pub ok: Rect,
    pub cancel: Rect,
    /// The keyboard layout badge: the taskbar cell's size, at the message
    /// area's bottom-right, above Cancel.
    pub layout: Rect,
}

/// Unscaled dialog dimensions (logical px), multiplied by the theme scale.
const DIALOG_W: i32 = 360;
const DIALOG_H: i32 = 180;
const PAD: i32 = 12;
const LABEL_W: i32 = 90;
const ROW_H: i32 = 20;
const LINE_H: i32 = 13;
const BUTTON_W: i32 = 75;
const BUTTON_H: i32 = 23;

/// The Unlock dialog's frame in a styled look: a fixed dialog's border and
/// caption, without caption buttons.
const CHROME: Chrome = Chrome {
    buttons: ButtonSet::None,
    ..Chrome::DIALOG
};

/// The dialog in `theme`'s look. Classic fits a window's border and the
/// caption inside `DIALOG_W` x `DIALOG_H`. A styled look puts the style's
/// frame around the client area Classic has under the default metrics, so
/// the body keeps its size and the dialog grows by the frame's difference.
/// The client grows with the text size (`theme.text`); so does Classic's
/// whole dialog, by as much.
pub fn dialog_layout(theme: &Theme) -> DialogLayout {
    let s = theme.scale.max(1);
    let u = theme.text;
    let m = &theme.metrics;
    let d = Metrics::DEFAULT;
    let (cw, ch) = (
        DIALOG_W - 2 * d.frame_width,
        DIALOG_H - 2 * d.frame_width - d.caption_height,
    );
    let grow = (
        (u.len(cw) - cw) * s,
        (u.len(ch) - ch) * s + (m.caption_height - d.caption_height * s).max(0),
    );
    let (frame, caption, client) = match theme.look {
        Look::Classic => {
            let f = m.frame_width;
            let (w, h) = (DIALOG_W * s + grow.0, DIALOG_H * s + grow.1);
            let caption = Rect::new(f, f, w - 2 * f, m.caption_height);
            let client = Rect::new(f, caption.bottom(), w - 2 * f, h - f - caption.bottom());
            (None, caption, client)
        }
        Look::Styled(_) => {
            let g = FrameGeometry::layout(m, u.len(cw) * s, u.len(ch) * s, false, CHROME);
            (Some(g), g.caption, g.client)
        }
    };
    // `v` 100% px from the client's top or left, at the text size, in device px.
    let at = |v: i32| u.len(v) * s;
    let box_x = client.x + at(PAD + LABEL_W);
    let box_w = client.right() - at(PAD) - box_x;
    let row = |top: i32| {
        let top = PAD + top;
        Rect::new(box_x, client.y + at(top), box_w, at(top + ROW_H) - at(top))
    };
    let user_box = row(24);
    let field = row(52);
    let message = Rect::new(
        client.x + at(PAD),
        client.y + at(PAD + 80),
        client.width - 2 * at(PAD),
        at(MESSAGE_LINES as i32 * LINE_H),
    );
    let (bw, bh) = (at(BUTTON_W), at(BUTTON_H));
    let by = client.bottom() - at(PAD) - bh;
    let cancel = Rect::new(client.right() - at(PAD) - bw, by, bw, bh);
    let ok = Rect::new(cancel.x - at(6) - bw, by, bw, bh);
    let badge = theme.icon_size();
    let layout = Rect::new(
        cancel.right() - badge,
        message.bottom() - badge,
        badge,
        badge,
    );
    let size = match frame {
        Some(g) => g.total,
        None => (DIALOG_W * s + grow.0, DIALOG_H * s + grow.1),
    };
    DialogLayout {
        size,
        frame,
        caption,
        user_box,
        field,
        message,
        ok,
        cancel,
        layout,
    }
}

const TITLE: &str = "Unlock Computer";

/// The "Unlock Computer" dialog: a raised window frame with an
/// active caption, two labelled rows, a message line and OK / Cancel.
/// A styled look draws the frame and caption through the style, with
/// `icon` (the lock icon at the theme's icon size) in the caption.
pub fn draw_dialog(
    theme: &Theme,
    text: &TextRenderer,
    icon: Option<&Canvas>,
    user: &str,
    state: &LockState,
) -> Canvas {
    let l = dialog_layout(theme);
    let (w, h) = l.size;
    let mut c = Canvas::new(w.max(1) as u32, h.max(1) as u32);
    match &l.frame {
        Some(g) => {
            let frame = FrameState {
                title: TITLE,
                active: true,
                maximized: false,
                pressed: None,
                glow: reprisekit::frame::Glow::lit(None),
                icon,
                tabbed: false,
            };
            reprisekit::look::draw_dialog_frame(&mut c, theme, text, g, &frame, None);
        }
        None => draw_classic_frame(&mut c, theme, text, &l),
    }
    draw_body(&mut c, theme, text, &l, user, state);
    c
}

fn draw_classic_frame(c: &mut Canvas, theme: &Theme, text: &TextRenderer, l: &DialogLayout) {
    let s = theme.scale.max(1);
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let all = Rect::new(0, 0, l.size.0, l.size.1);
    c.fill_rect(all, cs.button_face);
    c.edge_t(all, EdgeStyle::WindowRaised, cs, s);

    // The caption has no buttons, so the gradient's ramp is the whole
    // band less 4 px (see `reprisekit::classic::widgets::caption_ramp`).
    c.fill_rect(l.caption, cs.gradient_active_title);
    c.hgradient_stepped(
        Rect::new(
            l.caption.x,
            l.caption.y,
            reprisekit::classic::widgets::caption_ramp(l.caption.width, 0, s),
            l.caption.height,
        ),
        cs.active_title,
        cs.gradient_active_title,
    );
    let title = Rect::new(
        l.caption.x + 4 * s,
        l.caption.y,
        l.caption.width - 8 * s,
        l.caption.height,
    );
    text.draw(c, title, TITLE, true, cs.title_text, Align::Left, px);
}

fn draw_body(
    c: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    l: &DialogLayout,
    user: &str,
    state: &LockState,
) {
    let s = theme.scale.max(1);
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let at = |v: i32| theme.text.len(v) * s;
    let label_x = l.caption.x + at(PAD);
    let body_y = l.caption.bottom() + at(PAD);
    let line = Rect::new(label_x, body_y, l.size.0 - 2 * label_x, at(16));
    text.draw(
        c,
        line,
        "This computer is in use and has been locked.",
        false,
        cs.button_text,
        Align::Left,
        px,
    );

    let label = |c: &mut Canvas, r: Rect, s_: &str| {
        text.draw(
            c,
            Rect::new(label_x, r.y, at(LABEL_W), r.height),
            s_,
            false,
            cs.button_text,
            Align::Left,
            px,
        );
    };
    // A styled look frames both fields with its edit border; Classic
    // keeps a thin sunken ring.
    let field = |c: &mut Canvas, r: Rect, fill: Color, focused: bool| {
        if theme.look.style().is_some() {
            draw_edit_field(c, &theme.paint(), r, fill, focused, Interaction::Idle);
        } else {
            c.fill_rect(r, fill);
            c.edge_t(r, EdgeStyle::ThinSunken, cs, s);
        }
    };
    label(c, l.user_box, "User name:");
    // The user name is shown, not typed: its field stays face.
    field(c, l.user_box, cs.button_face, false);
    text.draw(
        c,
        l.user_box.inset(3 * s),
        user,
        false,
        cs.button_text,
        Align::Left,
        px,
    );

    let prompt = state.prompt.as_ref();
    let name = prompt.map_or("Password:", |p| p.text.as_str());
    label(c, l.field, &text.ellipsize(name, at(LABEL_W), false, px));
    field(c, l.field, cs.window, true);
    let shown: std::borrow::Cow<str> = match prompt {
        Some(p) if p.echo => state.input.as_str().into(),
        _ => "*".repeat(state.input.chars().count()).into(),
    };
    text.draw(
        c,
        l.field.inset(3 * s),
        &shown,
        false,
        cs.button_text,
        Align::Left,
        px,
    );

    let msg = if state.verifying {
        Some("Verifying...")
    } else {
        (state.message.as_deref())
            .or(prompt.map(|p| p.text.as_str()))
            .or(state.finger.as_deref())
    };
    let width = match &state.layout {
        Some(_) => l.layout.x - at(6) - l.message.x,
        None => l.message.width,
    };
    let mut lines = Vec::new();
    if state.caps_lock {
        lines.push(text.ellipsize(CAPS_LOCK, width, false, px).into_owned());
    }
    if let Some(msg) = msg {
        let room = MESSAGE_LINES - lines.len();
        lines.extend(message_lines(text, msg, width, px, room));
    }
    for (i, line) in lines.iter().enumerate() {
        let line_h = at(LINE_H);
        let r = Rect::new(l.message.x, l.message.y + i as i32 * line_h, width, line_h);
        text.draw(c, r, line, false, cs.button_text, Align::Left, px);
    }
    if let Some(short) = &state.layout {
        let badge = indicator::draw_layout(text, cs, short, l.layout.width, px);
        c.blit_canvas(l.layout.x, l.layout.y, &badge);
    }

    for (r, caption) in [(l.ok, "OK"), (l.cancel, "Cancel")] {
        draw_button(
            c,
            text,
            &theme.paint(),
            r,
            caption,
            false,
            Interaction::Idle,
            true,
            false,
            px,
        );
    }
}

/// `msg` wrapped to `width`, cut to `room` lines, with "..." closing the
/// last line when text was cut and on any line still too wide.
fn message_lines(text: &TextRenderer, msg: &str, width: i32, px: u32, room: usize) -> Vec<String> {
    let mut lines = crate::text::wrap(text, msg, width, px);
    if lines.len() > room {
        lines.truncate(room);
        let Some(last) = lines.last_mut() else {
            return lines;
        };
        while text.measure(&format!("{last}..."), false, px) > width && last.pop().is_some() {}
        last.truncate(last.trim_end().len());
        last.push_str("...");
    }
    lines
        .iter()
        .map(|l| text.ellipsize(l, width, false, px).into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn debug_redacts_the_password() {
        let mut st = super::LockState::new();
        st.input.push_str("hunter2");
        let shown = format!("{st:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("<7 chars>"), "{shown}");
    }

    use super::*;

    #[test]
    fn typing_backspace_and_escape_edit_the_field() {
        let mut s = LockState::new();
        assert!(!s.key(LockKey::Text("ab")));
        assert!(!s.key(LockKey::Text("c")));
        assert_eq!(s.input, "abc");
        assert!(!s.key(LockKey::Backspace));
        assert_eq!(s.input, "ab");
        s.message = Some(INCORRECT.into());
        assert!(!s.key(LockKey::Escape));
        assert_eq!(s.input, "");
        // Escape also dismisses a stale failure message.
        assert_eq!(s.message, None);
        // Backspace on an empty field is harmless.
        assert!(!s.key(LockKey::Backspace));
    }

    #[test]
    fn enter_and_ok_submit_only_a_non_empty_field() {
        let mut s = LockState::new();
        assert!(!s.key(LockKey::Enter));
        assert!(!s.button(LockButton::Ok));
        s.key(LockKey::Text("x"));
        assert!(s.key(LockKey::Enter));
        assert!(s.button(LockButton::Ok));
        // Cancel clears and never submits.
        assert!(!s.button(LockButton::Cancel));
        assert_eq!(s.input, "");
    }

    #[test]
    fn input_is_ignored_while_verifying() {
        let mut s = LockState::new();
        s.key(LockKey::Text("x"));
        s.verifying = true;
        assert!(!s.key(LockKey::Text("y")));
        assert!(!s.key(LockKey::Enter));
        assert!(!s.button(LockButton::Ok));
        assert_eq!(s.input, "x");
    }

    #[test]
    fn fail_clears_input_and_sets_the_message() {
        let mut s = LockState::new();
        s.key(LockKey::Text("secret"));
        s.verifying = true;
        s.fail_with(INCORRECT);
        assert_eq!(s.input, "");
        assert!(!s.verifying);
        assert_eq!(s.message.as_deref(), Some(INCORRECT));
    }

    #[test]
    fn the_field_keeps_its_reserved_capacity() {
        let mut s = LockState::new();
        assert!(s.input.capacity() >= INPUT_CAPACITY);
        s.key(LockKey::Text(&"x".repeat(300)));
        s.key(LockKey::Escape);
        assert_eq!(s.input, "");
        assert!(s.input.capacity() >= INPUT_CAPACITY);
        s.key(LockKey::Text(&"y".repeat(300)));
        s.fail_with(INCORRECT);
        assert!(s.input.capacity() >= INPUT_CAPACITY);
        s.key(LockKey::Text("hunter2"));
        assert_eq!(*s.take_input(), "hunter2");
        assert!(s.input.capacity() >= INPUT_CAPACITY);
    }

    #[test]
    fn typing_past_the_reservation_is_refused() {
        let mut s = LockState::new();
        let buf = s.input.as_ptr();
        s.key(LockKey::Text(&"x".repeat(INPUT_CAPACITY - 1)));
        // Two bytes would end past the reservation, so none of it is kept.
        s.key(LockKey::Text("\u{e9}"));
        assert_eq!(s.input.len(), INPUT_CAPACITY - 1);
        s.key(LockKey::Text("y"));
        s.key(LockKey::Text("z"));
        assert_eq!(s.input.len(), INPUT_CAPACITY);
        assert!(s.input.ends_with('y'));
        assert_eq!(s.input.as_ptr(), buf);
    }

    #[test]
    fn the_finger_prompt_shows_until_pam_answers_and_only_success_unlocks() {
        use crate::lockfinger::Line;
        let mut s = LockState::new();
        s.key(LockKey::Text("hunt"));
        let prompt = "Place your finger on the fingerprint reader";
        assert!(!s.finger(Line::Info(prompt.into())));
        assert_eq!(s.finger.as_deref(), Some(prompt));
        assert_eq!(s.message, None);
        // A miss says so and keeps what was typed.
        assert!(!s.finger(Line::Error("Failed to match fingerprint".into())));
        assert_eq!(s.message.as_deref(), Some("Failed to match fingerprint"));
        assert_eq!(s.input, "hunt");
        // Escape clears the note; the standing prompt shows again.
        s.key(LockKey::Escape);
        assert_eq!(s.message, None);
        assert_eq!(s.finger.as_deref(), Some(prompt));
        assert!(s.finger(Line::Result(ffi::PAM_SUCCESS)));
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
            let mut s = LockState::new();
            s.finger(Line::Info("Swipe your finger".into()));
            assert!(!s.finger(Line::Result(rc)), "PAM code {rc} does not unlock");
            assert_eq!(s.finger, None);
            assert_eq!(s.message, None, "code {rc} adds no text of its own");
        }
        let mut s = LockState::new();
        s.finger(Line::Info("Swipe your finger".into()));
        assert!(!s.finger(Line::Result(ffi::PAM_MAXTRIES)));
        assert_eq!(s.finger, None);
        assert_eq!(s.message.as_deref(), Some(FINGER_TOO_MANY));
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
    fn pam_text_is_cleaned_and_cut_to_the_message_lines() {
        let mut n = Vec::new();
        add_notice(&mut n, "  \n\tThe account\x1b[31m is locked.\r\n");
        assert_eq!(n, ["The account [31m is locked."]);
        add_notice(&mut n, &"x".repeat(1000));
        assert_eq!(n[1], "x".repeat(NOTICE_CHARS));
        add_notice(&mut n, "past the message area");
        assert_eq!(n.len(), MESSAGE_LINES);
    }

    #[test]
    fn a_rejection_shows_what_pam_sent_else_the_incorrect_text() {
        assert_eq!(failure_text(&[]), INCORRECT);
        let locked = ["Account locked.".to_string(), "(10 minutes left)".into()];
        assert_eq!(failure_text(&locked), "Account locked.\n(10 minutes left)");
    }

    #[test]
    fn pam_service_prefers_our_own_file() {
        let s = PamVerifier::service();
        assert!(s == "reprise" || s == "login");
        if std::path::Path::new("/etc/pam.d/reprise").exists() {
            assert_eq!(s, "reprise");
        } else {
            assert_eq!(s, "login");
        }
    }

    #[test]
    fn libpam_starts_a_handle_through_the_declared_bindings() {
        let service = CString::new(PamVerifier::service()).unwrap();
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

    use crate::raster::Canvas;
    use crate::text::TextRenderer;
    use crate::theme::{FontConfig, Theme};

    #[test]
    fn dialog_layout_scales_with_theme() {
        let t1 = Theme::default();
        let l1 = dialog_layout(&t1);
        assert_eq!(l1.size, (360, 180));
        assert_eq!(l1.ok.width, 75);
        assert_eq!(l1.ok.height, 23);
        assert!(l1.ok.right() + 6 == l1.cancel.x);
        assert!(l1.cancel.right() <= 360 - t1.metrics.frame_width);
        assert!(l1.field.y > l1.user_box.bottom());
        assert!(l1.message.y > l1.field.bottom());
        assert!(l1.ok.y > l1.message.bottom());
        let l2 = dialog_layout(&t1.scaled(2));
        assert_eq!(l2.size, (720, 360));
        assert_eq!(l2.ok, l1.ok.scaled(2));
        assert_eq!(l2.field, l1.field.scaled(2));
    }

    fn has_ink(c: &Canvas, r: crate::raster::Rect, bg: u32) -> bool {
        (r.x..r.right()).any(|x| (r.y..r.bottom()).any(|y| c.get(x, y) != bg))
    }

    #[test]
    fn draw_dialog_shows_caption_and_asterisks() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let l = dialog_layout(&theme);
        let mut s = LockState::new();
        let c = draw_dialog(&theme, &text, None, "someone", &s);
        assert_eq!(c.width as i32, l.size.0);
        assert_eq!(c.height as i32, l.size.1);
        // Caption gradient starts with the active title color.
        assert_eq!(
            c.get(l.caption.x, l.caption.y + 2),
            theme.colors.active_title.argb()
        );
        // Empty field: white interior only.
        let inner = l.field.inset(2);
        assert!(!has_ink(&c, inner, theme.colors.window.argb()));
        s.key(LockKey::Text("abc"));
        let c = draw_dialog(&theme, &text, None, "someone", &s);
        assert!(has_ink(&c, inner, theme.colors.window.argb()));
        // OK button is raised: left column hilight.
        assert_eq!(
            c.get(l.ok.x, l.ok.y + 5),
            theme.colors.button_hilight.argb()
        );
    }

    #[test]
    fn draw_dialog_field_text_clears_the_sunken_ring_at_scale_4() {
        // `edge_t` draws the field's sunken ring `s` px thick; the text rect
        // must inset by `3 * s`, not a fixed 3 px, or at scale >= 4 the text
        // starts on top of the ring instead of inside it.
        let theme = Theme::default().scaled(4);
        let text = TextRenderer::load(&theme.fonts);
        let l = dialog_layout(&theme);
        let s = LockState::new();
        let c = draw_dialog(&theme, &text, None, "someone", &s);
        let scale = theme.scale;
        assert_eq!(
            c.get(l.field.x + 4 * scale, l.field.y + l.field.height / 2),
            theme.colors.window.argb()
        );
    }

    #[test]
    fn a_long_message_is_cut_to_two_lines_that_fit_the_area() {
        let text = TextRenderer::load(&FontConfig::default());
        let w = dialog_layout(&Theme::default()).message.width;
        let one = message_lines(&text, INCORRECT, w, 11, MESSAGE_LINES);
        assert_eq!(one, [INCORRECT]);
        let long = format!("{}\n{}", "word ".repeat(40), "y".repeat(NOTICE_CHARS));
        let lines = message_lines(&text, &long, w, 11, MESSAGE_LINES);
        assert_eq!(lines.len(), MESSAGE_LINES);
        assert!(lines[1].ends_with("..."), "{lines:?}");
        assert!(
            lines.iter().all(|l| text.measure(l, false, 11) <= w),
            "{lines:?}"
        );
    }

    #[test]
    fn two_message_lines_end_above_the_buttons() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let l = dialog_layout(&theme);
        assert!(l.message.bottom() < l.ok.y);
        let mut s = LockState::new();
        s.fail_with("The account is locked due to 3 failed logins.\n(10 minutes left to unlock)");
        let c = draw_dialog(&theme, &text, None, "someone", &s);
        let bg = theme.colors.button_face.argb();
        let half = l.message.height / 2;
        let top = Rect::new(l.message.x, l.message.y, l.message.width, half);
        let bottom = Rect::new(l.message.x, l.message.y + half, l.message.width, half);
        assert!(has_ink(&c, top, bg) && has_ink(&c, bottom, bg));
    }

    #[test]
    fn draw_dialog_message_and_verifying() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let l = dialog_layout(&theme);
        let bg = theme.colors.button_face.argb();
        let s = LockState::new();
        let c = draw_dialog(&theme, &text, None, "someone", &s);
        assert!(!has_ink(&c, l.message, bg));
        let mut s = LockState::new();
        s.fail_with(INCORRECT);
        let c = draw_dialog(&theme, &text, None, "someone", &s);
        assert!(has_ink(&c, l.message, bg));
        let mut s = LockState::new();
        s.verifying = true;
        let c = draw_dialog(&theme, &text, None, "someone", &s);
        assert!(has_ink(&c, l.message, bg));
        // The reader's standing prompt fills an empty message area.
        let mut s = LockState::new();
        s.finger(crate::lockfinger::Line::Info("Place your finger".into()));
        let c = draw_dialog(&theme, &text, None, "someone", &s);
        assert!(has_ink(&c, l.message, bg));
    }

    /// A further prompt labels the field with its text, fills the empty
    /// message area with it, and an echoed one shows the answer as typed.
    #[test]
    fn a_further_prompt_relabels_the_field() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let l = dialog_layout(&theme);
        let s_ = theme.scale;
        let label = Rect::new(l.caption.x + PAD, l.field.y, LABEL_W, l.field.height);
        let pixels = |c: &Canvas, r: Rect| -> Vec<u32> {
            (r.y..r.bottom())
                .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
                .map(|(x, y)| c.get(x, y))
                .collect()
        };
        let mut s = LockState::new();
        s.key(LockKey::Text("1234"));
        let password = draw_dialog(&theme, &text, None, "someone", &s);
        s.ask(Prompt {
            text: "Code:".into(),
            echo: false,
        });
        assert_eq!(s.input, "", "the field is cleared for the answer");
        s.key(LockKey::Text("1234"));
        let blind = draw_dialog(&theme, &text, None, "someone", &s);
        assert_ne!(
            pixels(&password, label),
            pixels(&blind, label),
            "the label is the prompt's"
        );
        let inside = l.field.inset(3 * s_);
        assert_eq!(
            pixels(&password, inside),
            pixels(&blind, inside),
            "a blind answer shows stars"
        );
        assert!(has_ink(&blind, l.message, theme.colors.button_face.argb()));
        s.prompt.as_mut().unwrap().echo = true;
        let echoed = draw_dialog(&theme, &text, None, "someone", &s);
        assert_ne!(
            pixels(&blind, inside),
            pixels(&echoed, inside),
            "an echoed answer shows as typed"
        );
        s.fail_with(INCORRECT);
        assert_eq!(s.prompt, None, "a verdict ends the prompt");
    }

    fn fnv(c: &Canvas) -> u64 {
        [c.width, c.height].iter().chain(c.pixels.iter()).fold(
            0xcbf2_9ce4_8422_2325_u64,
            |h, &p| {
                p.to_le_bytes().iter().fold(h, |h, &b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
                })
            },
        )
    }

    /// Caps Lock puts its notice on the message area's first line, under
    /// the field, and leaves one line to the message.
    #[test]
    fn caps_lock_shows_a_notice_under_the_field() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let l = dialog_layout(&theme);
        let bg = theme.colors.button_face.argb();
        let line_h = LINE_H;
        let first = Rect::new(l.message.x, l.message.y, l.message.width, line_h);
        let second = Rect::new(l.message.x, l.message.y + line_h, l.message.width, line_h);
        let mut s = LockState::new();
        s.caps_lock = true;
        let c = draw_dialog(&theme, &text, None, "someone", &s);
        assert!(has_ink(&c, first, bg) && !has_ink(&c, second, bg));
        s.fail_with(&format!("{}\n{}", "word ".repeat(40), "more"));
        let c = draw_dialog(&theme, &text, None, "someone", &s);
        assert!(has_ink(&c, second, bg), "the message keeps the second line");
        let lines = message_lines(&text, s.message.as_deref().unwrap(), l.message.width, 11, 1);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].ends_with("..."), "{lines:?}");
    }

    #[test]
    fn caps_lock_dialog_pins_at_scale_1_and_2() {
        if !reprisekit::text::test_tahoma() {
            return;
        }
        let got = CAPS_PINS.map(|(scale, _)| {
            let theme = Theme::default().scaled(scale);
            let text = TextRenderer::load(&theme.fonts);
            let mut s = LockState::new();
            s.caps_lock = true;
            s.key(LockKey::Text("abc"));
            (scale, fnv(&draw_dialog(&theme, &text, None, "someone", &s)))
        });
        assert!(
            got == CAPS_PINS,
            "pins changed; now:\n{}",
            got.map(|(s, h)| format!("({s}, {h:#018x}),")).join("\n")
        );
    }

    const CAPS_PINS: [(i32, u64); 2] = [(1, 0x57f830b0b53ffdfc), (2, 0x68c13bf424ad086b)];

    fn blue() -> Theme {
        reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue)
    }

    /// The layout badge sits at the message area's bottom-right, above
    /// Cancel, in the taskbar cell's highlight in every look; the message
    /// wraps short of it.
    #[test]
    fn the_layout_badge_fills_its_corner_only_when_set() {
        assert_eq!(
            dialog_layout(&Theme::default()).layout,
            Rect::new(328, 124, 16, 16)
        );
        let glass = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
        for look in [Theme::default(), blue(), glass] {
            for scale in [1, 2] {
                let theme = look.scaled(scale);
                let text = TextRenderer::load(&theme.fonts);
                let l = dialog_layout(&theme);
                assert_eq!(l.layout.right(), l.cancel.right());
                assert_eq!(l.layout.bottom(), l.message.bottom());
                assert!(l.layout.bottom() <= l.cancel.y);
                let mut s = LockState::new();
                let without = draw_dialog(&theme, &text, None, "someone", &s);
                s.layout = Some("EN".into());
                let with = draw_dialog(&theme, &text, None, "someone", &s);
                let hilight = theme.colors.hilight.argb();
                let r = l.layout;
                for (x, y) in [(r.x, r.y), (r.right() - 1, r.bottom() - 1)] {
                    assert_eq!(with.get(x, y), hilight, "badge corner at {scale}x");
                    assert_ne!(without.get(x, y), hilight, "no badge at {scale}x");
                }
                let outside = (0..with.height as i32)
                    .flat_map(|y| (0..with.width as i32).map(move |x| (x, y)))
                    .filter(|&(x, y)| !r.contains(x, y))
                    .all(|(x, y)| with.get(x, y) == without.get(x, y));
                assert!(outside, "the badge draws inside its rect at {scale}x");
            }
        }
        let theme = Theme::default();
        let text = TextRenderer::load(&theme.fonts);
        let l = dialog_layout(&theme);
        let mut s = LockState::new();
        s.layout = Some("EN".into());
        s.fail_with(&"word ".repeat(80));
        let c = draw_dialog(&theme, &text, None, "someone", &s);
        let gap = Rect::new(l.layout.x - 6, l.message.y, 6, l.message.height);
        assert!(!has_ink(&c, gap, theme.colors.button_face.argb()));
    }

    /// Under Blue the dialog's caption is the style's 26 px band inside a
    /// 3 px border, with no caption buttons, and the body keeps Classic's
    /// client area: it sits 1 px left and 7 px lower in a dialog 2 px
    /// narrower and 6 px taller. Scale 2 doubles every rect.
    #[test]
    fn a_blue_unlock_dialog_takes_the_styles_frame() {
        let classic = dialog_layout(&Theme::default());
        let blue = blue();
        let l = dialog_layout(&blue);
        let frame = l.frame.expect("a styled look lays out a frame");
        assert_eq!(l.size, (358, 186));
        assert_eq!(l.caption, Rect::new(3, 3, 352, 26));
        assert_eq!(frame.client, Rect::new(3, 29, 352, 154));
        assert_eq!(frame.chrome.buttons, ButtonSet::None);
        let mv = |r: Rect| Rect::new(r.x - 1, r.y + 7, r.width, r.height);
        assert_eq!(l.user_box, mv(classic.user_box));
        assert_eq!(l.field, mv(classic.field));
        assert_eq!(l.message, mv(classic.message));
        assert_eq!(l.ok, mv(classic.ok));
        assert_eq!(l.cancel, mv(classic.cancel));
        assert_eq!(classic.frame, None);
        let l2 = dialog_layout(&blue.scaled(2));
        assert_eq!(l2.size, (716, 372));
        assert_eq!(l2.caption, l.caption.scaled(2));
        assert_eq!(l2.ok, l.ok.scaled(2));
        assert_eq!(l2.cancel, l.cancel.scaled(2));
    }

    /// Blue draws the caption band with the style's Caption part, active,
    /// and the body on button face under it, with OK a Blue push button.
    #[test]
    fn a_blue_unlock_caption_is_the_styles_caption() {
        use reprisekit::style::{Part, State};
        let blue = blue();
        let text = TextRenderer::load(&blue.fonts);
        let l = dialog_layout(&blue);
        let c = draw_dialog(&blue, &text, None, "someone", &LockState::new());
        assert_eq!((c.width as i32, c.height as i32), l.size);
        let top = Rect::new(0, 0, l.size.0, l.caption.bottom());
        let mut want = Canvas::new(l.size.0 as u32, top.height as u32);
        blue.look
            .style()
            .unwrap()
            .draw(&mut want, top, Part::Caption, State::Normal, 1);
        // Above the icon, past the title, and near the right end, where a
        // dialog's close box would sit.
        for (x, y) in [(12, 2), (200, 15), (l.caption.right() - 12, 15)] {
            assert_eq!(c.get(x, y), want.get(x, y), "caption pixel ({x}, {y})");
        }
        assert_eq!(
            c.get(l.caption.x + 5, l.caption.bottom() + 2),
            blue.colors.button_face.argb(),
            "the body is button face"
        );
        let mut button = Canvas::new(l.ok.width as u32, l.ok.height as u32);
        let whole = button.bounds();
        button.fill_rect(whole, blue.colors.button_face);
        blue.look
            .style()
            .unwrap()
            .draw(&mut button, whole, Part::PushButton, State::Normal, 1);
        assert_eq!(
            c.get(l.ok.x + 1, l.ok.y + 5),
            button.get(1, 5),
            "OK is the style's push button: its left border"
        );
    }

    /// Under Blue both fields take the style's 1 px edit border:
    /// the password field is window white inside it, the user name field
    /// face. The caption shows the lock icon as given, 16x16.
    #[test]
    fn a_blue_unlock_dialog_frames_its_fields_with_the_edit_border_and_shows_its_icon() {
        let blue = blue();
        let text = TextRenderer::load(&blue.fonts);
        let l = dialog_layout(&blue);
        let line = reprisekit::blue::Variant::Blue
            .scheme()
            .edit_border
            .normal
            .rim
            .argb();
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(icon.bounds(), Color::rgb(0xFF, 0x00, 0xFF));
        let c = draw_dialog(&blue, &text, Some(&icon), "someone", &LockState::new());
        for (r, inside, what) in [
            (l.field, blue.colors.window, "password"),
            (l.user_box, blue.colors.button_face, "user name"),
        ] {
            let y = r.y + r.height / 2;
            assert_eq!(c.get(r.x, y), line, "{what} left edge is the edit line");
            assert_eq!(
                c.get(r.right() - 1, y),
                line,
                "{what} right edge is the edit line"
            );
            assert_eq!(
                c.get(r.x + 1, y),
                inside.argb(),
                "{what} is {:08X} inside the border",
                inside.argb()
            );
        }
        let magenta = (l.caption.y..l.caption.bottom())
            .flat_map(|y| (l.caption.x..l.caption.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == 0xFFFF_00FF)
            .count();
        assert_eq!(magenta, 256, "the caption shows the 16x16 icon unscaled");
    }

    /// Renders the Unlock dialog (and under Blue) to `target/lock-preview.png` for eyeballing: `cargo test render_lock_preview -- --ignored`.
    #[test]
    #[ignore]
    fn render_lock_preview() {
        let blue = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
        for (theme, file) in [
            (Theme::default(), "lock-preview.png"),
            (blue, "lock-blue-preview.png"),
        ] {
            let mut failed = LockState::new();
            failed.key(LockKey::Text("secret"));
            failed.fail_with(INCORRECT);
            let mut caps = LockState::new();
            caps.caps_lock = true;
            caps.fail_with(INCORRECT);
            let empty = LockState::new();
            let canvases: Vec<Canvas> = [(1, &empty), (1, &failed), (1, &caps), (2, &empty)]
                .into_iter()
                .map(|(scale, state)| {
                    let t = theme.scaled(scale);
                    let text = TextRenderer::load(&t.fonts);
                    draw_dialog(&t, &text, None, "someone", state)
                })
                .collect();
            let gap = 8;
            let w = canvases.iter().map(|c| c.width).max().unwrap() as i32 + 2 * gap;
            let h: i32 = canvases.iter().map(|c| c.height as i32 + gap).sum::<i32>() + gap;
            let mut out = Canvas::new(w as u32, h as u32);
            out.fill_rect(out.bounds(), theme.colors.background);
            let mut y = gap;
            for c in &canvases {
                out.blit_canvas(gap, y, c);
                y += c.height as i32 + gap;
            }
            reprisekit::raster::write_preview(&out, file);
        }
    }

    /// At 150% and 200% the dialog's client grows with the text: the rows hold a grown line, the
    /// message clears the buttons, and every part sits inside the dialog. 100% is the 360x180
    /// dialog.
    #[test]
    fn the_dialog_grows_with_the_text_size() {
        assert_eq!(dialog_layout(&Theme::default()).size, (DIALOG_W, DIALOG_H));
        for pct in [150, 200] {
            let theme = Theme::default().with_text_size(pct);
            let l = dialog_layout(&theme);
            let px = theme.fonts.size_px as i32;
            assert!(l.field.height >= px + 4, "{pct}%");
            assert!(l.user_box.bottom() <= l.field.y, "{pct}%");
            assert!(l.field.bottom() <= l.message.y, "{pct}%");
            assert!(
                l.message.bottom() <= l.ok.y,
                "{pct}%: the message clears the buttons"
            );
            assert!(
                l.cancel.right() <= l.size.0 && l.cancel.bottom() <= l.size.1,
                "{pct}%"
            );
            assert!(l.caption.bottom() <= l.user_box.y, "{pct}%");
        }
    }
}
