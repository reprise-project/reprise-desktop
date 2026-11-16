use std::ffi::{CStr, CString, c_int, c_void};

use repriselogin::clean_prompt;
use repriselogin::greeter::AuthKind;
use zeroize::{Zeroize, Zeroizing};

use crate::ffi;

/// Answers one PAM message, its text cleaned by `repriselogin::clean_prompt` with the lines
/// joined by `\n`. Info and Error return `None`; `None` for a Visible or Secret prompt ends the
/// conversation.
pub trait Converse {
    fn prompt(&mut self, kind: AuthKind, text: &str) -> Option<Zeroizing<String>>;
}

impl<C: Converse + ?Sized> Converse for &mut C {
    fn prompt(&mut self, kind: AuthKind, text: &str) -> Option<Zeroizing<String>> {
        (**self).prompt(kind, text)
    }
}

/// Runs `conv` through the callback libpam calls, over `msgs` (style, text) as one round, and
/// returns the answers libpam would get or the callback's error code. For tests of a `Converse`.
pub fn run_conversation(
    conv: &mut dyn Converse,
    msgs: &[(c_int, &str)],
) -> Result<Vec<Option<Zeroizing<String>>>, c_int> {
    let texts: Vec<CString> = msgs
        .iter()
        .map(|(_, t)| CString::new(*t).unwrap_or_default())
        .collect();
    let messages: Vec<ffi::pam_message> = msgs
        .iter()
        .zip(&texts)
        .map(|((style, _), t)| ffi::pam_message {
            msg_style: *style,
            msg: t.as_ptr(),
        })
        .collect();
    let mut ptrs: Vec<*const ffi::pam_message> = messages.iter().map(|m| m as *const _).collect();
    let mut out: *mut ffi::pam_response = std::ptr::null_mut();
    let mut conv: Box<dyn Converse + '_> = Box::new(conv);
    // SAFETY: `ptrs` holds `msgs.len()` live messages, `out` is a live result slot and `conv`
    // is a live boxed `Converse` that outlives the call.
    let rc = unsafe {
        converse(
            ptrs.len() as c_int,
            ptrs.as_mut_ptr(),
            &mut out,
            (&mut conv as *mut Box<dyn Converse + '_>).cast(),
        )
    };
    if rc != ffi::PAM_SUCCESS {
        return Err(rc);
    }
    // SAFETY: on success `out` is the callback's calloc'd array of `msgs.len()` answers, each
    // null or a calloc'd NUL-terminated string, owned here as libpam would own it.
    unsafe {
        let answers = (0..msgs.len())
            .map(|i| {
                let r = (*out.add(i)).resp;
                (!r.is_null())
                    .then(|| Zeroizing::new(CStr::from_ptr(r).to_string_lossy().into_owned()))
            })
            .collect();
        free_answers(out, msgs.len());
        Ok(answers)
    }
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

    /// Records every message and answers prompts from `answers` in order.
    struct Fake {
        seen: Vec<(AuthKind, String)>,
        answers: Vec<Option<&'static str>>,
    }

    impl Converse for Fake {
        fn prompt(&mut self, kind: AuthKind, text: &str) -> Option<Zeroizing<String>> {
            self.seen.push((kind, text.to_owned()));
            match kind {
                AuthKind::Visible | AuthKind::Secret => {
                    self.answers.remove(0).map(|a| Zeroizing::new(a.to_owned()))
                }
                AuthKind::Info | AuthKind::Error => None,
            }
        }
    }

    type Answers = Result<Vec<Option<String>>, c_int>;

    fn run(msgs: &[(c_int, &str)], answers: Vec<Option<&'static str>>) -> (Answers, Fake) {
        let mut fake = Fake {
            seen: Vec::new(),
            answers,
        };
        let out = run_conversation(&mut fake, msgs);
        let out = out.map(|a| a.into_iter().map(|a| a.map(|a| a.to_string())).collect());
        (out, fake)
    }

    #[test]
    fn echo_off_prompt_gets_the_answer() {
        let (answers, fake) = run(
            &[(ffi::PAM_PROMPT_ECHO_OFF, "Password: ")],
            vec![Some("pw")],
        );
        assert_eq!(answers, Ok(vec![Some("pw".to_string())]));
        assert_eq!(fake.seen, [(AuthKind::Secret, "Password:".to_string())]);
    }

    #[test]
    fn info_and_error_are_relayed_without_an_answer() {
        let (answers, fake) = run(
            &[
                (ffi::PAM_TEXT_INFO, "Place your finger"),
                (ffi::PAM_ERROR_MSG, "Account\x1b expires\nin 2 days"),
            ],
            vec![],
        );
        assert_eq!(answers, Ok(vec![None, None]));
        assert_eq!(
            fake.seen,
            [
                (AuthKind::Info, "Place your finger".to_string()),
                (AuthKind::Error, "Account  expires\nin 2 days".to_string()),
            ]
        );
    }

    #[test]
    fn a_refused_prompt_fails_the_conversation_with_no_responses() {
        let (answers, fake) = run(
            &[
                (ffi::PAM_PROMPT_ECHO_ON, "login:"),
                (ffi::PAM_PROMPT_ECHO_OFF, "Password:"),
            ],
            vec![Some("anna"), None],
        );
        assert_eq!(answers, Err(ffi::PAM_CONV_ERR));
        assert_eq!(fake.seen.len(), 2);
    }

    #[test]
    fn an_unknown_message_style_ends_the_conversation() {
        let (answers, fake) = run(&[(7, "binary")], vec![]);
        assert_eq!(answers, Err(ffi::PAM_CONV_ERR));
        assert!(fake.seen.is_empty());
    }

    #[test]
    fn a_round_past_pam_max_num_msg_is_refused_unread() {
        let msgs = vec![(ffi::PAM_TEXT_INFO, "note"); ffi::PAM_MAX_NUM_MSG as usize + 1];
        let (answers, fake) = run(&msgs, vec![]);
        assert_eq!(answers, Err(ffi::PAM_CONV_ERR));
        assert!(fake.seen.is_empty());
        let (answers, _) = run(&msgs[1..], vec![]);
        assert!(answers.is_ok(), "PAM_MAX_NUM_MSG messages are one round");
    }
}
