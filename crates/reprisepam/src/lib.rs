#[allow(non_camel_case_types)]
pub mod ffi {
    use std::ffi::{c_char, c_int, c_void};

    pub const PAM_SUCCESS: c_int = 0;
    pub const PAM_SYSTEM_ERR: c_int = 4;
    pub const PAM_BUF_ERR: c_int = 5;
    pub const PAM_AUTH_ERR: c_int = 7;
    pub const PAM_AUTHINFO_UNAVAIL: c_int = 9;
    pub const PAM_MAXTRIES: c_int = 11;
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
    pub const PAM_RHOST: c_int = 4;
    pub const PAM_FAIL_DELAY: c_int = 10;

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

#[cfg(test)]
mod tests {
    use super::ffi::*;

    #[test]
    fn the_constants_are_linux_pams() {
        assert_eq!(PAM_SUCCESS, 0);
        assert_eq!(PAM_BUF_ERR, 5);
        assert_eq!(PAM_AUTH_ERR, 7);
        assert_eq!(PAM_AUTHINFO_UNAVAIL, 9);
        assert_eq!(PAM_MAXTRIES, 11);
        assert_eq!(PAM_CONV_ERR, 19);
        assert_eq!(PAM_PROMPT_ECHO_OFF, 1);
        assert_eq!(PAM_PROMPT_ECHO_ON, 2);
        assert_eq!(PAM_ERROR_MSG, 3);
        assert_eq!(PAM_TEXT_INFO, 4);
        assert_eq!(PAM_FAIL_DELAY, 10);
    }
}
