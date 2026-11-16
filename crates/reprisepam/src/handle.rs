use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fmt;

use crate::conv::{Converse, converse};
use crate::ffi;

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

/// One PAM transaction, its conversation borrowed for `'a`. `Drop` ends it with the last call's
/// status.
pub struct Pam<'a> {
    handle: *mut ffi::pam_handle_t,
    last: c_int,
    _conv: Box<ffi::pam_conv>,
    /// The `appdata_ptr` libpam hands back to `converse`, freed in `Drop` after `pam_end`.
    converse: *mut Box<dyn Converse + 'a>,
}

impl<'a> Pam<'a> {
    pub fn start(
        service: &str,
        user: &str,
        conv: Box<dyn Converse + 'a>,
    ) -> Result<Pam<'a>, PamError> {
        let cservice = c_string(service, "service name")?;
        let cuser = c_string(user, "user name")?;
        let appdata = Box::into_raw(Box::new(conv));
        let pam_conv = Box::new(ffi::pam_conv {
            conv: Some(converse),
            appdata_ptr: appdata.cast(),
        });
        let mut handle = std::ptr::null_mut();
        // SAFETY: the strings are NUL-terminated and `pam_conv` and the
        // `Converse` it points at are boxed and live as long as the handle.
        let rc =
            unsafe { ffi::pam_start(cservice.as_ptr(), cuser.as_ptr(), &*pam_conv, &mut handle) };
        if rc != ffi::PAM_SUCCESS || handle.is_null() {
            // SAFETY: `appdata` came from `Box::into_raw` above and libpam holds no handle.
            drop(unsafe { Box::from_raw(appdata) });
            return Err(PamError {
                code: rc,
                text: format!("cannot start PAM service {service} (code {rc})"),
            });
        }
        Ok(Pam {
            handle,
            last: rc,
            _conv: pam_conv,
            converse: appdata,
        })
    }

    pub fn set_tty(&mut self, tty: &str) -> Result<(), PamError> {
        let tty = c_string(tty, "tty")?;
        // SAFETY: the handle is live; libpam copies the string.
        let rc = unsafe { ffi::pam_set_item(self.handle, ffi::PAM_TTY, tty.as_ptr().cast()) };
        self.check(rc)
    }

    /// `PAM_RHOST`, the remote client's address; `None` clears it.
    pub fn set_item_rhost(&mut self, rhost: Option<&str>) -> Result<(), PamError> {
        let rhost = rhost.map(|r| c_string(r, "remote host")).transpose()?;
        let item = rhost
            .as_ref()
            .map_or(std::ptr::null(), |r| r.as_ptr().cast());
        // SAFETY: the handle is live; libpam copies the string, and a NULL item clears it.
        let rc = unsafe { ffi::pam_set_item(self.handle, ffi::PAM_RHOST, item) };
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
        let pair = c_string(&format!("{key}={value}"), "environment variable")?;
        // SAFETY: the handle is live; libpam copies the string.
        let rc = unsafe { ffi::pam_putenv(self.handle, pair.as_ptr()) };
        self.check(rc)
    }

    pub fn authenticate(&mut self) -> Result<(), PamError> {
        // SAFETY: the handle is live.
        let rc = unsafe { ffi::pam_authenticate(self.handle, 0) };
        self.check(rc)
    }

    pub fn acct_mgmt(&mut self) -> Result<(), PamError> {
        // SAFETY: the handle is live.
        let rc = unsafe { ffi::pam_acct_mgmt(self.handle, 0) };
        self.check(rc)
    }

    pub fn chauthtok(&mut self, flags: c_int) -> Result<(), PamError> {
        // SAFETY: the handle is live.
        let rc = unsafe { ffi::pam_chauthtok(self.handle, flags) };
        self.check(rc)
    }

    /// Clears libpam's delay after a failed `authenticate`, for a caller that never retries on
    /// this handle.
    pub fn skip_fail_delay(&mut self) -> Result<(), PamError> {
        let delay: unsafe extern "C" fn(c_int, libc::c_uint, *mut c_void) = no_delay;
        // SAFETY: the handle is live, and PAM_FAIL_DELAY takes a function of `no_delay`'s
        // signature, which libpam calls instead of sleeping.
        let rc =
            unsafe { ffi::pam_set_item(self.handle, ffi::PAM_FAIL_DELAY, delay as *const c_void) };
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

impl Drop for Pam<'_> {
    fn drop(&mut self) {
        // SAFETY: the handle is live and not used after this call; `converse` came from
        // `Box::into_raw` and libpam no longer calls back once `pam_end` returns.
        unsafe {
            ffi::pam_end(self.handle, self.last);
            drop(Box::from_raw(self.converse));
        }
    }
}

/// `s` as a C string. The error names `what` and never echoes `s`, which may be a password
/// typed into the user name field.
fn c_string(s: &str, what: &str) -> Result<CString, PamError> {
    CString::new(s).map_err(|_| PamError {
        code: ffi::PAM_SYSTEM_ERR,
        text: format!("the {what} holds a NUL byte"),
    })
}

unsafe extern "C" fn no_delay(_retval: c_int, _usec: libc::c_uint, _data: *mut c_void) {}
