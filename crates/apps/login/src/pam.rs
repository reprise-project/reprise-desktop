//! libpam through one handle, from `pam_start` to `pam_end`, and its conversation callback.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fmt;

use repriselogin::clean_prompt;
use repriselogin::greeter::AuthKind;
use zeroize::{Zeroize, Zeroizing};

#[allow(non_camel_case_types)]
pub(crate) mod ffi {
    use std::ffi::{c_char, c_int, c_void};

    pub const PAM_SUCCESS: c_int = 0;
    pub const PAM_SYSTEM_ERR: c_int = 4;
    pub const PAM_BUF_ERR: c_int = 5;
    pub const PAM_NEW_AUTHTOK_REQD: c_int = 12;
    pub const PAM_CONV_ERR: c_int = 19;

    pub const PAM_PROMPT_ECHO_OFF: c_int = 1;
    pub const PAM_PROMPT_ECHO_ON: c_int = 2;
    pub const PAM_ERROR_MSG: c_int = 3;
    pub const PAM_TEXT_INFO: c_int = 4;
    /// Linux-PAM's `PAM_MAX_NUM_MSG`.
    pub const PAM_MAX_NUM_MSG: c_int = 32;

    pub const PAM_USER: c_int = 2;
    pub const PAM_TTY: c_int = 3;

    pub const PAM_ESTABLISH_CRED: c_int = 0x2;
    pub const PAM_DELETE_CRED: c_int = 0x4;
    pub const PAM_CHANGE_EXPIRED_AUTHTOK: c_int = 0x20;

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
        pub fn pam_end(pamh: *mut pam_handle_t, pam_status: c_int) -> c_int;
        pub fn pam_authenticate(pamh: *mut pam_handle_t, flags: c_int) -> c_int;
        pub fn pam_acct_mgmt(pamh: *mut pam_handle_t, flags: c_int) -> c_int;
        pub fn pam_chauthtok(pamh: *mut pam_handle_t, flags: c_int) -> c_int;
        pub fn pam_setcred(pamh: *mut pam_handle_t, flags: c_int) -> c_int;
        pub fn pam_open_session(pamh: *mut pam_handle_t, flags: c_int) -> c_int;
        pub fn pam_close_session(pamh: *mut pam_handle_t, flags: c_int) -> c_int;
        pub fn pam_set_item(
            pamh: *mut pam_handle_t,
            item_type: c_int,
            item: *const c_void,
        ) -> c_int;
        pub fn pam_get_item(
            pamh: *const pam_handle_t,
            item_type: c_int,
            item: *mut *const c_void,
        ) -> c_int;
        pub fn pam_putenv(pamh: *mut pam_handle_t, name_value: *const c_char) -> c_int;
        pub fn pam_getenvlist(pamh: *mut pam_handle_t) -> *mut *mut c_char;
        pub fn pam_strerror(pamh: *mut pam_handle_t, errnum: c_int) -> *const c_char;
    }
}

/// Answers one PAM message. Info and Error return `None`; `None` for a
/// Visible or Secret prompt ends the conversation.
pub trait Converse {
    fn prompt(&mut self, kind: AuthKind, text: &str) -> Option<Zeroizing<String>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PamError {
    pub code: i32,
    pub text: String,
}

impl fmt::Display for PamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (PAM code {})", self.text, self.code)
    }
}

impl std::error::Error for PamError {}

/// The two calls of the account step, so `account_steps` runs without libpam in tests.
pub trait PamOps {
    fn acct_mgmt(&mut self) -> c_int;
    fn chauthtok(&mut self, flags: c_int) -> c_int;
}

/// `pam_acct_mgmt`, then on `PAM_NEW_AUTHTOK_REQD` `pam_chauthtok` for the expired password.
pub fn account_steps(ops: &mut impl PamOps) -> c_int {
    match ops.acct_mgmt() {
        ffi::PAM_NEW_AUTHTOK_REQD => ops.chauthtok(ffi::PAM_CHANGE_EXPIRED_AUTHTOK),
        rc => rc,
    }
}

/// One PAM transaction. `Drop` ends it with the last call's status.
pub struct Pam {
    handle: *mut ffi::pam_handle_t,
    last: c_int,
    _conv: Box<ffi::pam_conv>,
    _converse: Box<Box<dyn Converse>>,
}

impl Pam {
    pub fn start(service: &str, user: &str, conv: Box<dyn Converse>) -> Result<Pam, PamError> {
        let cservice = c_string(service)?;
        let cuser = c_string(user)?;
        let mut converse_box = Box::new(conv);
        let pam_conv = Box::new(ffi::pam_conv {
            conv: Some(converse),
            appdata_ptr: &mut *converse_box as *mut Box<dyn Converse> as *mut c_void,
        });
        let mut handle = std::ptr::null_mut();
        // SAFETY: the strings are NUL-terminated and `pam_conv` and the
        // `Converse` it points at are boxed and live as long as the handle.
        let rc =
            unsafe { ffi::pam_start(cservice.as_ptr(), cuser.as_ptr(), &*pam_conv, &mut handle) };
        if rc != ffi::PAM_SUCCESS || handle.is_null() {
            return Err(PamError {
                code: rc,
                text: format!("cannot start PAM service {service} (code {rc})"),
            });
        }
        Ok(Pam {
            handle,
            last: rc,
            _conv: pam_conv,
            _converse: converse_box,
        })
    }

    pub fn set_tty(&mut self, tty: &str) -> Result<(), PamError> {
        let tty = c_string(tty)?;
        // SAFETY: the handle is live; libpam copies the string.
        let rc = unsafe { ffi::pam_set_item(self.handle, ffi::PAM_TTY, tty.as_ptr().cast()) };
        self.check(rc)
    }

    /// `PAM_USER`, which a module may have changed during `authenticate`.
    pub fn user(&self) -> Option<String> {
        let mut item: *const c_void = std::ptr::null();
        // SAFETY: the handle is live and `item` is a local out pointer.
        let rc = unsafe { ffi::pam_get_item(self.handle, ffi::PAM_USER, &mut item) };
        if rc != ffi::PAM_SUCCESS || item.is_null() {
            return None;
        }
        // SAFETY: PAM_USER is a NUL-terminated string owned by the handle.
        let user = unsafe { CStr::from_ptr(item.cast::<c_char>()) };
        user.to_str().ok().map(str::to_owned)
    }

    pub fn putenv(&mut self, key: &str, value: &str) -> Result<(), PamError> {
        let pair = c_string(&format!("{key}={value}"))?;
        // SAFETY: the handle is live; libpam copies the string.
        let rc = unsafe { ffi::pam_putenv(self.handle, pair.as_ptr()) };
        self.check(rc)
    }

    pub fn authenticate(&mut self) -> Result<(), PamError> {
        // SAFETY: the handle is live.
        let rc = unsafe { ffi::pam_authenticate(self.handle, 0) };
        self.check(rc)
    }

    /// The account stack; an expired password is changed through the conversation.
    pub fn account(&mut self) -> Result<(), PamError> {
        let rc = account_steps(self);
        self.check(rc)
    }

    pub fn establish_cred(&mut self) -> Result<(), PamError> {
        // SAFETY: the handle is live.
        let rc = unsafe { ffi::pam_setcred(self.handle, ffi::PAM_ESTABLISH_CRED) };
        self.check(rc)
    }

    pub fn open_session(&mut self) -> Result<(), PamError> {
        // SAFETY: the handle is live.
        let rc = unsafe { ffi::pam_open_session(self.handle, 0) };
        self.check(rc)
    }

    pub fn close_session(&mut self) -> Result<(), PamError> {
        // SAFETY: the handle is live.
        let rc = unsafe { ffi::pam_close_session(self.handle, 0) };
        self.check(rc)
    }

    pub fn delete_cred(&mut self) -> Result<(), PamError> {
        // SAFETY: the handle is live.
        let rc = unsafe { ffi::pam_setcred(self.handle, ffi::PAM_DELETE_CRED) };
        self.check(rc)
    }

    /// The PAM environment, in libpam's order.
    pub fn env(&self) -> Vec<(String, String)> {
        // SAFETY: the handle is live; the list is ours to free.
        let list = unsafe { ffi::pam_getenvlist(self.handle) };
        if list.is_null() {
            return Vec::new();
        }
        let mut env = Vec::new();
        // SAFETY: `list` is a NULL-terminated array of malloc'd
        // NUL-terminated strings; each is freed once, then the array.
        unsafe {
            let mut i = 0;
            while !(*list.add(i)).is_null() {
                let entry = *list.add(i);
                if let Some((k, v)) = CStr::from_ptr(entry)
                    .to_str()
                    .ok()
                    .and_then(|s| s.split_once('='))
                {
                    env.push((k.to_owned(), v.to_owned()));
                }
                libc::free(entry.cast());
                i += 1;
            }
            libc::free(list.cast());
        }
        env
    }

    fn check(&mut self, rc: c_int) -> Result<(), PamError> {
        self.last = rc;
        if rc == ffi::PAM_SUCCESS {
            return Ok(());
        }
        // SAFETY: the handle is live; pam_strerror returns a static string.
        let text = unsafe { CStr::from_ptr(ffi::pam_strerror(self.handle, rc)) };
        Err(PamError {
            code: rc,
            text: text.to_string_lossy().into_owned(),
        })
    }
}

impl PamOps for Pam {
    fn acct_mgmt(&mut self) -> c_int {
        // SAFETY: the handle is live.
        unsafe { ffi::pam_acct_mgmt(self.handle, 0) }
    }

    fn chauthtok(&mut self, flags: c_int) -> c_int {
        // SAFETY: the handle is live.
        unsafe { ffi::pam_chauthtok(self.handle, flags) }
    }
}

impl Drop for Pam {
    fn drop(&mut self) {
        // SAFETY: the handle is live and not used after this call.
        unsafe { ffi::pam_end(self.handle, self.last) };
    }
}

fn c_string(s: &str) -> Result<CString, PamError> {
    CString::new(s).map_err(|_| PamError {
        code: ffi::PAM_SYSTEM_ERR,
        text: format!("{s:?} holds a NUL byte"),
    })
}

/// libpam's conversation callback.
///
/// # Safety
/// `data` is a live `*mut Box<dyn Converse>`, and `msgs` and `out` are what
/// libpam passes: `n` message pointers and a result slot.
pub(crate) unsafe extern "C" fn converse(
    n: c_int,
    msgs: *mut *const ffi::pam_message,
    out: *mut *mut ffi::pam_response,
    data: *mut c_void,
) -> c_int {
    if n <= 0 || n > ffi::PAM_MAX_NUM_MSG || msgs.is_null() || out.is_null() || data.is_null() {
        return ffi::PAM_CONV_ERR;
    }
    // SAFETY: `data` is the `appdata_ptr` `Pam::start` registered, a live boxed `Converse`
    // that only this call uses while it runs.
    let conv = unsafe { &mut *(data as *mut Box<dyn Converse>) };
    let n = n as usize;
    // SAFETY: plain allocation; the result is null-checked below.
    let resp = unsafe { libc::calloc(n, std::mem::size_of::<ffi::pam_response>()) }
        as *mut ffi::pam_response;
    if resp.is_null() {
        return ffi::PAM_BUF_ERR;
    }
    for i in 0..n {
        // Linux-PAM hands an array of message pointers.
        // SAFETY: libpam passes `n` live messages behind `msgs`.
        let m = unsafe { &**msgs.add(i) };
        let kind = match m.msg_style {
            ffi::PAM_PROMPT_ECHO_OFF => AuthKind::Secret,
            ffi::PAM_PROMPT_ECHO_ON => AuthKind::Visible,
            ffi::PAM_ERROR_MSG => AuthKind::Error,
            ffi::PAM_TEXT_INFO => AuthKind::Info,
            _ => {
                // SAFETY: the first `i` answers are null or calloc'd by this loop.
                unsafe { free_answers(resp, i) };
                return ffi::PAM_CONV_ERR;
            }
        };
        let text = match m.msg.is_null() {
            true => String::new(),
            // SAFETY: libpam's message text is a NUL-terminated string.
            false => clean_prompt(&unsafe { CStr::from_ptr(m.msg) }.to_string_lossy()).join("\n"),
        };
        let answer = conv.prompt(kind, &text);
        if matches!(kind, AuthKind::Info | AuthKind::Error) {
            continue;
        }
        let Some(answer) = answer else {
            // SAFETY: the first `i` answers are null or calloc'd by this loop.
            unsafe { free_answers(resp, i) };
            return ffi::PAM_CONV_ERR;
        };
        // SAFETY: plain allocation; the result is null-checked below.
        let buf = unsafe { libc::calloc(answer.len() + 1, 1) } as *mut u8;
        if buf.is_null() {
            // SAFETY: the first `i` answers are null or calloc'd by this loop.
            unsafe { free_answers(resp, i) };
            return ffi::PAM_BUF_ERR;
        }
        // SAFETY: `buf` holds `answer.len() + 1` bytes and `i < n` is inside `resp`.
        unsafe {
            std::ptr::copy_nonoverlapping(answer.as_ptr(), buf, answer.len());
            (*resp.add(i)).resp = buf.cast();
        }
    }
    // SAFETY: `out` is libpam's non-null result slot; libpam frees the array.
    unsafe { *out = resp };
    ffi::PAM_SUCCESS
}

/// Wipes and frees the first `i` answers, then the array.
///
/// # Safety
/// `resp` is a calloc'd array not yet handed to libpam, and each of its
/// first `i` answers is null or a calloc'd NUL-terminated buffer.
unsafe fn free_answers(resp: *mut ffi::pam_response, i: usize) {
    // SAFETY: the caller's contract.
    unsafe {
        for j in 0..i {
            let r = (*resp.add(j)).resp;
            if !r.is_null() {
                std::slice::from_raw_parts_mut(r.cast::<u8>(), libc::strlen(r)).zeroize();
                libc::free(r.cast());
            }
        }
        libc::free(resp.cast());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    const PAM_AUTH_ERR: c_int = 7;

    type Seen = Rc<RefCell<Vec<(AuthKind, String)>>>;
    type Responses = Vec<Option<String>>;

    /// Records every message and answers prompts from `answers` in order.
    struct Fake {
        seen: Seen,
        answers: Vec<Option<&'static str>>,
    }

    impl Converse for Fake {
        fn prompt(&mut self, kind: AuthKind, text: &str) -> Option<Zeroizing<String>> {
            self.seen.borrow_mut().push((kind, text.to_owned()));
            match kind {
                AuthKind::Visible | AuthKind::Secret => {
                    self.answers.remove(0).map(|a| Zeroizing::new(a.to_owned()))
                }
                AuthKind::Info | AuthKind::Error => None,
            }
        }
    }

    /// Runs `converse` over `msgs` with a `Fake`; the responses are read and freed.
    fn run(
        msgs: &[(c_int, &str)],
        answers: Vec<Option<&'static str>>,
    ) -> (c_int, Option<Responses>, Vec<(AuthKind, String)>) {
        let seen = Seen::default();
        let mut conv: Box<dyn Converse> = Box::new(Fake {
            seen: seen.clone(),
            answers,
        });
        let texts: Vec<CString> = msgs
            .iter()
            .map(|(_, t)| CString::new(*t).unwrap())
            .collect();
        let messages: Vec<ffi::pam_message> = msgs
            .iter()
            .zip(&texts)
            .map(|((style, _), t)| ffi::pam_message {
                msg_style: *style,
                msg: t.as_ptr(),
            })
            .collect();
        let mut ptrs: Vec<*const ffi::pam_message> = messages
            .iter()
            .map(|m| m as *const ffi::pam_message)
            .collect();
        let mut out: *mut ffi::pam_response = std::ptr::null_mut();
        // SAFETY: `ptrs` holds `msgs.len()` live messages and `conv` outlives the call.
        let rc = unsafe {
            converse(
                ptrs.len() as c_int,
                ptrs.as_mut_ptr(),
                &mut out,
                &mut conv as *mut Box<dyn Converse> as *mut c_void,
            )
        };
        let responses = (!out.is_null()).then(|| {
            // SAFETY: on success `out` is a calloc'd array of `msgs.len()`
            // responses, each null or a NUL-terminated calloc'd string.
            unsafe {
                let r = (0..msgs.len())
                    .map(|i| {
                        let p = (*out.add(i)).resp;
                        (!p.is_null()).then(|| CStr::from_ptr(p).to_str().unwrap().to_owned())
                    })
                    .collect();
                free_answers(out, msgs.len());
                r
            }
        });
        let seen = seen.borrow().clone();
        (rc, responses, seen)
    }

    #[test]
    fn echo_off_prompt_gets_the_answer() {
        let (rc, responses, seen) = run(
            &[(ffi::PAM_PROMPT_ECHO_OFF, "Password: ")],
            vec![Some("pw")],
        );
        assert_eq!(rc, ffi::PAM_SUCCESS);
        assert_eq!(responses, Some(vec![Some("pw".to_string())]));
        assert_eq!(seen, [(AuthKind::Secret, "Password:".to_string())]);
    }

    #[test]
    fn info_and_error_are_relayed_without_an_answer() {
        let (rc, responses, seen) = run(
            &[
                (ffi::PAM_TEXT_INFO, "Place your finger"),
                (ffi::PAM_ERROR_MSG, "Account\x1b expires\nin 2 days"),
            ],
            vec![],
        );
        assert_eq!(rc, ffi::PAM_SUCCESS);
        assert_eq!(responses, Some(vec![None, None]));
        assert_eq!(
            seen,
            [
                (AuthKind::Info, "Place your finger".to_string()),
                (AuthKind::Error, "Account  expires\nin 2 days".to_string()),
            ]
        );
    }

    #[test]
    fn a_refused_prompt_returns_conv_err_and_frees_earlier_answers() {
        let (rc, responses, seen) = run(
            &[
                (ffi::PAM_PROMPT_ECHO_ON, "login:"),
                (ffi::PAM_PROMPT_ECHO_OFF, "Password:"),
            ],
            vec![Some("anna"), None],
        );
        assert_eq!(rc, ffi::PAM_CONV_ERR);
        assert_eq!(responses, None);
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn an_unknown_message_style_ends_the_conversation() {
        let (rc, responses, seen) = run(&[(7, "binary")], vec![]);
        assert_eq!(rc, ffi::PAM_CONV_ERR);
        assert_eq!(responses, None);
        assert!(seen.is_empty());
    }

    struct FakeOps {
        acct: Vec<c_int>,
        chauthtok_flags: Vec<c_int>,
        chauthtok_rc: c_int,
    }

    impl PamOps for FakeOps {
        fn acct_mgmt(&mut self) -> c_int {
            self.acct.remove(0)
        }

        fn chauthtok(&mut self, flags: c_int) -> c_int {
            self.chauthtok_flags.push(flags);
            self.chauthtok_rc
        }
    }

    #[test]
    fn an_expired_password_runs_chauthtok() {
        let mut ops = FakeOps {
            acct: vec![ffi::PAM_NEW_AUTHTOK_REQD],
            chauthtok_flags: vec![],
            chauthtok_rc: ffi::PAM_SUCCESS,
        };
        assert_eq!(account_steps(&mut ops), ffi::PAM_SUCCESS);
        assert_eq!(ops.chauthtok_flags, [ffi::PAM_CHANGE_EXPIRED_AUTHTOK]);

        ops.acct = vec![ffi::PAM_NEW_AUTHTOK_REQD];
        ops.chauthtok_rc = PAM_AUTH_ERR;
        assert_eq!(account_steps(&mut ops), PAM_AUTH_ERR);
    }

    #[test]
    fn a_refused_account_changes_no_password() {
        let mut ops = FakeOps {
            acct: vec![PAM_AUTH_ERR],
            chauthtok_flags: vec![],
            chauthtok_rc: ffi::PAM_SUCCESS,
        };
        assert_eq!(account_steps(&mut ops), PAM_AUTH_ERR);
        assert!(ops.chauthtok_flags.is_empty());
    }
}
