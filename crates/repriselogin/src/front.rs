//! Messages between reprise-login and its RDP front, reprise-login-rdp, over `FRONT_SOCKET`.

use std::fmt;
use std::net::SocketAddr;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::handover::HandOver;

pub const FRONT_SOCKET: &str = "/run/reprise-login/front.sock";
pub const FRONT_USER: &str = "reprise-login-rdp";
/// The front's certificate fingerprint while it runs, in its runtime directory.
pub const FINGERPRINT: &str = "/run/reprise-login-rdp/fingerprint";

/// `Password` came in the Client Info PDU over TLS; `Delegated` is the password CredSSP
/// delegated after Network Level Authentication.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Secret {
    Password(Zeroizing<String>),
    Delegated(Zeroizing<String>),
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Secret::Password(_) => f.write_str("Password(<redacted>)"),
            Secret::Delegated(_) => f.write_str("Delegated(<redacted>)"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FrontRequest {
    Hello,
    Authenticate {
        conn: u64,
        peer: SocketAddr,
        user: String,
        secret: Secret,
        /// Time since the TCP accept.
        elapsed_ms: u32,
    },
    /// Travels with the client's socket.
    Ready {
        conn: u64,
        handover: Box<HandOver>,
    },
    Gone {
        conn: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FrontEvent {
    Passwords { users: Vec<NlaUser> },
    Verified { conn: u64 },
    Accepted { conn: u64 },
    Denied { conn: u64, backoff_s: u32 },
}

/// `nt_hash` is 32 uppercase hex characters.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NlaUser {
    pub user: String,
    pub nt_hash: Zeroizing<String>,
}

impl fmt::Debug for NlaUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NlaUser")
            .field("user", &self.user)
            .field("nt_hash", &format_args!("<redacted>"))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{decode, encode};

    fn body<T: Serialize>(m: &T) -> String {
        String::from_utf8(encode(m).unwrap()[4..].to_vec()).unwrap()
    }

    fn authenticate(secret: Secret) -> FrontRequest {
        FrontRequest::Authenticate {
            conn: 7,
            peer: "[2001:db8::1]:3389".parse().unwrap(),
            user: "anna".into(),
            secret,
            elapsed_ms: 120,
        }
    }

    #[test]
    fn wire_shapes_are_pinned() {
        assert_eq!(
            body(&FrontEvent::Denied {
                conn: 7,
                backoff_s: 4
            }),
            r#"{"type":"denied","conn":7,"backoff_s":4}"#
        );
        for (secret, kind) in [
            (
                Secret::Password(Zeroizing::new("hunter2".into())),
                "password",
            ),
            (
                Secret::Delegated(Zeroizing::new("hunter2".into())),
                "delegated",
            ),
        ] {
            let sent = authenticate(secret);
            let json = body(&sent);
            assert_eq!(
                json,
                format!(
                    r#"{{"type":"authenticate","conn":7,"peer":"[2001:db8::1]:3389","user":"anna","secret":{{"{kind}":"hunter2"}},"elapsed_ms":120}}"#
                )
            );
            assert_eq!(decode::<FrontRequest>(json.as_bytes()).unwrap(), sent);
        }
    }

    #[test]
    fn debug_of_an_authenticate_never_shows_the_secret() {
        for secret in [
            Secret::Password(Zeroizing::new("hunter2".into())),
            Secret::Delegated(Zeroizing::new("hunter2".into())),
        ] {
            let shown = format!("{:?}", authenticate(secret));
            assert!(!shown.contains("hunter2"), "{shown}");
            assert!(shown.contains("<redacted>"), "{shown}");
        }
    }

    #[test]
    fn debug_of_the_passwords_never_shows_a_hash() {
        let m = FrontEvent::Passwords {
            users: vec![NlaUser {
                user: "anna".into(),
                nt_hash: Zeroizing::new("8846F7EAEE8FB117AD06BDD830B7586C".into()),
            }],
        };
        let shown = format!("{m:?}");
        assert!(!shown.contains("8846"), "{shown}");
        assert!(shown.contains(r#"user: "anna""#), "{shown}");
    }
}
