//! The account step on reprisepam's handle.

use std::ffi::c_int;

use reprisepam::ffi;
pub use reprisepam::{Converse, PamError};

/// A transaction whose conversation lives as long as it does.
pub type Pam = reprisepam::Pam<'static>;

/// The two calls of the account step, so `account_steps` runs without libpam in tests.
pub trait PamOps {
    fn acct_mgmt(&mut self) -> Result<(), PamError>;
    fn chauthtok(&mut self, flags: c_int) -> Result<(), PamError>;
}

/// `pam_acct_mgmt`, then on `PAM_NEW_AUTHTOK_REQD` with `change` `pam_chauthtok` for the expired
/// password; without, that error is the answer.
pub fn account_steps(ops: &mut impl PamOps, change: bool) -> Result<(), PamError> {
    match ops.acct_mgmt() {
        Err(e) if change && e.code == ffi::PAM_NEW_AUTHTOK_REQD => {
            ops.chauthtok(ffi::PAM_CHANGE_EXPIRED_AUTHTOK)
        }
        r => r,
    }
}

impl PamOps for Pam {
    fn acct_mgmt(&mut self) -> Result<(), PamError> {
        Pam::acct_mgmt(self)
    }

    fn chauthtok(&mut self, flags: c_int) -> Result<(), PamError> {
        Pam::chauthtok(self, flags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAM_AUTH_ERR: c_int = 7;

    struct FakeOps {
        acct: Vec<c_int>,
        chauthtok_flags: Vec<c_int>,
        chauthtok_rc: c_int,
    }

    fn status(code: c_int) -> Result<(), PamError> {
        match code {
            ffi::PAM_SUCCESS => Ok(()),
            code => Err(PamError {
                code,
                text: String::new(),
            }),
        }
    }

    impl PamOps for FakeOps {
        fn acct_mgmt(&mut self) -> Result<(), PamError> {
            status(self.acct.remove(0))
        }

        fn chauthtok(&mut self, flags: c_int) -> Result<(), PamError> {
            self.chauthtok_flags.push(flags);
            status(self.chauthtok_rc)
        }
    }

    #[test]
    fn an_expired_password_runs_chauthtok() {
        let mut ops = FakeOps {
            acct: vec![ffi::PAM_NEW_AUTHTOK_REQD],
            chauthtok_flags: vec![],
            chauthtok_rc: ffi::PAM_SUCCESS,
        };
        assert_eq!(account_steps(&mut ops, true), Ok(()));
        assert_eq!(ops.chauthtok_flags, [ffi::PAM_CHANGE_EXPIRED_AUTHTOK]);

        ops.acct = vec![ffi::PAM_NEW_AUTHTOK_REQD];
        ops.chauthtok_rc = PAM_AUTH_ERR;
        assert_eq!(account_steps(&mut ops, true), status(PAM_AUTH_ERR));
    }

    #[test]
    fn without_a_change_an_expired_password_is_reported() {
        let mut ops = FakeOps {
            acct: vec![ffi::PAM_NEW_AUTHTOK_REQD],
            chauthtok_flags: vec![],
            chauthtok_rc: ffi::PAM_SUCCESS,
        };
        assert_eq!(
            account_steps(&mut ops, false),
            status(ffi::PAM_NEW_AUTHTOK_REQD)
        );
        assert!(ops.chauthtok_flags.is_empty());
    }

    #[test]
    fn a_refused_account_changes_no_password() {
        let mut ops = FakeOps {
            acct: vec![PAM_AUTH_ERR],
            chauthtok_flags: vec![],
            chauthtok_rc: ffi::PAM_SUCCESS,
        };
        assert_eq!(account_steps(&mut ops, true), status(PAM_AUTH_ERR));
        assert!(ops.chauthtok_flags.is_empty());
    }
}
