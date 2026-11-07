//! A user's requests about their own Remote Desktop password over `ACCOUNT_SOCKET`.

use std::fmt;

use md4::{Digest, Md4};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const ACCOUNT_SOCKET: &str = "/run/reprise-login/account.sock";
/// Characters a Remote Desktop password has at least.
pub const MIN_REMOTE_PASSWORD: usize = 8;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AccountRequest {
    Status,
    SetRemotePassword {
        login_password: Zeroizing<String>,
        remote_password: Zeroizing<String>,
    },
    ClearRemotePassword {
        login_password: Zeroizing<String>,
    },
}

impl fmt::Debug for AccountRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted = format_args!("<redacted>");
        match self {
            AccountRequest::Status => f.write_str("Status"),
            AccountRequest::SetRemotePassword { .. } => f
                .debug_struct("SetRemotePassword")
                .field("login_password", &redacted)
                .field("remote_password", &redacted)
                .finish(),
            AccountRequest::ClearRemotePassword { .. } => f
                .debug_struct("ClearRemotePassword")
                .field("login_password", &redacted)
                .finish(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AccountReply {
    Status { remote_password: bool },
    Done,
    Refused { reason: AccountRefusal },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountRefusal {
    WrongPassword,
    TooShort,
    SameAsLogin,
    NotRemoteUser,
    Busy,
    Failed,
}

/// The NT hash NTLM checks against: MD4 of the UTF-16LE password, in uppercase hex.
pub fn nt_hash(password: &str) -> Zeroizing<String> {
    let mut utf16 = Zeroizing::new(Vec::with_capacity(password.len() * 2));
    for unit in password.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    let digest = Zeroizing::new(<[u8; 16]>::from(Md4::digest(&*utf16)));
    let mut hex = Zeroizing::new(String::with_capacity(32));
    for b in digest.iter() {
        for nibble in [b >> 4, b & 15] {
            hex.push(char::from(b"0123456789ABCDEF"[usize::from(nibble)]));
        }
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{decode, encode};

    #[test]
    fn nt_hash_of_password_is_the_known_value() {
        assert_eq!(*nt_hash("password"), "8846F7EAEE8FB117AD06BDD830B7586C");
        assert_eq!(*nt_hash(""), "31D6CFE0D16AE931B73C59D7E0C089C0");
    }

    #[test]
    fn nt_hash_encodes_past_the_basic_plane() {
        assert_eq!(nt_hash("é𝄞").len(), 32);
        assert_ne!(nt_hash("é𝄞"), nt_hash("e"));
    }

    #[test]
    fn wire_shapes_are_pinned() {
        let body = |m: &AccountReply| String::from_utf8(encode(m).unwrap()[4..].to_vec()).unwrap();
        assert_eq!(
            body(&AccountReply::Refused {
                reason: AccountRefusal::SameAsLogin
            }),
            r#"{"type":"refused","reason":"same_as_login"}"#
        );
        assert_eq!(
            decode::<AccountRequest>(
                br#"{"type":"clear_remote_password","login_password":"hunter2"}"#
            )
            .unwrap(),
            AccountRequest::ClearRemotePassword {
                login_password: Zeroizing::new("hunter2".into())
            }
        );
    }

    #[test]
    fn debug_of_an_account_request_never_shows_either_password() {
        let set = AccountRequest::SetRemotePassword {
            login_password: Zeroizing::new("hunter2".into()),
            remote_password: Zeroizing::new("swordfish".into()),
        };
        let clear = AccountRequest::ClearRemotePassword {
            login_password: Zeroizing::new("hunter2".into()),
        };
        for shown in [format!("{set:?}"), format!("{clear:?}")] {
            assert!(!shown.contains("hunter2"), "{shown}");
            assert!(!shown.contains("swordfish"), "{shown}");
        }
        assert_eq!(
            format!("{clear:?}"),
            "ClearRemotePassword { login_password: <redacted> }"
        );
    }
}
