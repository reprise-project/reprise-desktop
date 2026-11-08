//! What reprise-login hands a user's reprise-rdp over its hand-over socket: an authenticated
//! connection's socket, the RDP exchange so far and the TLS state to install in the kernel.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// The socket the user manager listens on for hand-overs, inside the user's runtime directory.
pub fn intake_path(uid: u32) -> PathBuf {
    PathBuf::from(format!("/run/user/{uid}/reprise-rdp/handover.sock"))
}

/// One step of the connection sequence the front ran, replayed by the server that takes over.
/// A `Pdu` can be the Client Info PDU, which carries the password.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Pdu {
        #[serde(with = "crate::frame::hex")]
        input: Zeroizing<Vec<u8>>,
        #[serde(with = "crate::frame::hex")]
        output: Zeroizing<Vec<u8>>,
    },
    SecurityUpgradeDone,
    CredsspDone {
        user: String,
        domain: Option<String>,
    },
}

impl fmt::Debug for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Step::Pdu { input, output } => f
                .debug_struct("Pdu")
                .field("input_len", &input.len())
                .field("output_len", &output.len())
                .finish(),
            Step::SecurityUpgradeDone => f.write_str("SecurityUpgradeDone"),
            Step::CredsspDone { user, domain } => f
                .debug_struct("CredsspDone")
                .field("user", user)
                .field("domain", domain)
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TlsVersion {
    Tls12,
    Tls13,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cipher {
    Aes128Gcm,
    Aes256Gcm,
    Chacha20Poly1305,
}

/// One direction's record keys. `iv` is the full 12 bytes; `secret` is the TLS 1.3 traffic
/// secret, which a key update derives the next keys from.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectionKeys {
    pub seq: u64,
    #[serde(with = "crate::frame::hex")]
    pub key: Zeroizing<Vec<u8>>,
    #[serde(with = "crate::frame::hex")]
    pub iv: Zeroizing<Vec<u8>>,
    #[serde(with = "crate::frame::hex::option")]
    pub secret: Option<Zeroizing<Vec<u8>>>,
}

impl fmt::Debug for DirectionKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirectionKeys")
            .field("seq", &self.seq)
            .field("key", &format_args!("<redacted>"))
            .field("iv", &format_args!("<redacted>"))
            .field(
                "secret",
                &match self.secret {
                    Some(_) => format_args!("<redacted>"),
                    None => format_args!("None"),
                },
            )
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsState {
    pub version: TlsVersion,
    pub cipher: Cipher,
    pub tx: DirectionKeys,
    pub rx: DirectionKeys,
    pub record_limit: u64,
}

impl fmt::Debug for TlsState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsState")
            .field("version", &self.version)
            .field("cipher", &self.cipher)
            .field("tx", &self.tx)
            .field("rx", &self.rx)
            .field("record_limit", &self.record_limit)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandOver {
    /// The front's number for the connection, unique while the front runs.
    pub conn: u64,
    pub user: String,
    pub peer: SocketAddr,
    /// The negotiated security protocol, as in the X.224 Connection Confirm.
    pub security: u32,
    pub steps: Vec<Step>,
    pub tls: TlsState,
}

impl fmt::Debug for HandOver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HandOver")
            .field("conn", &self.conn)
            .field("user", &self.user)
            .field("peer", &self.peer)
            .field("security", &self.security)
            .field("steps", &self.steps.len())
            .field("tls", &self.tls)
            .finish()
    }
}

/// reprise-login to a user's reprise-rdp. `HandOver` always travels with exactly one fd, the
/// client's socket.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntakeMessage {
    HandOver(Box<HandOver>),
}

/// A user's reprise-rdp to reprise-login.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntakeReport {
    /// The clients now connected to the session, sent on every change.
    Viewers { count: u32 },
    /// The server's accept loop took hand-over `conn`; a stopping server sends none.
    Took { conn: u64 },
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::frame::{decode, encode};

    fn body<T: Serialize>(m: &T) -> String {
        String::from_utf8(encode(m).unwrap()[4..].to_vec()).unwrap()
    }

    fn keys(byte: u8) -> DirectionKeys {
        DirectionKeys {
            seq: 3,
            key: Zeroizing::new(vec![byte; 32]),
            iv: Zeroizing::new(vec![byte; 12]),
            secret: Some(Zeroizing::new(vec![205; 48])),
        }
    }

    pub(crate) fn hand_over() -> HandOver {
        HandOver {
            conn: 9,
            user: "anna".into(),
            peer: "192.0.2.1:50000".parse().unwrap(),
            security: 2,
            steps: vec![
                Step::Pdu {
                    input: Zeroizing::new(vec![171; 4]),
                    output: Zeroizing::new(vec![205; 4]),
                },
                Step::SecurityUpgradeDone,
                Step::CredsspDone {
                    user: "anna".into(),
                    domain: None,
                },
            ],
            tls: TlsState {
                version: TlsVersion::Tls13,
                cipher: Cipher::Aes256Gcm,
                tx: keys(171),
                rx: keys(171),
                record_limit: 1 << 24,
            },
        }
    }

    #[test]
    fn wire_shapes_are_pinned() {
        assert_eq!(
            body(&Step::Pdu {
                input: Zeroizing::new(vec![1, 0xAB]),
                output: Zeroizing::new(vec![])
            }),
            r#"{"pdu":{"input":"01ab","output":""}}"#
        );
        assert_eq!(
            body(&Step::SecurityUpgradeDone),
            r#""security_upgrade_done""#
        );
        assert_eq!(
            body(&IntakeReport::Viewers { count: 2 }),
            r#"{"viewers":{"count":2}}"#
        );
        assert_eq!(
            body(&IntakeReport::Took { conn: 9 }),
            r#"{"took":{"conn":9}}"#
        );
    }

    #[test]
    fn a_hand_over_round_trips_with_its_secrets_in_hex() {
        let sent = IntakeMessage::HandOver(Box::new(hand_over()));
        let keys = format!(
            r#"{{"seq":3,"key":"{}","iv":"{}","secret":"{}"}}"#,
            "ab".repeat(32),
            "ab".repeat(12),
            "cd".repeat(48)
        );
        assert_eq!(
            body(&sent),
            format!(
                r#"{{"hand_over":{{"conn":9,"user":"anna","peer":"192.0.2.1:50000","security":2,"steps":[{{"pdu":{{"input":"abababab","output":"cdcdcdcd"}}}},"security_upgrade_done",{{"credssp_done":{{"user":"anna","domain":null}}}}],"tls":{{"version":"tls13","cipher":"aes256_gcm","tx":{keys},"rx":{keys},"record_limit":16777216}}}}}}"#
            )
        );
        assert_eq!(
            decode::<IntakeMessage>(body(&sent).as_bytes()).unwrap(),
            sent
        );
    }

    #[test]
    fn a_tls12_hand_over_round_trips_without_a_traffic_secret() {
        let mut handover = hand_over();
        handover.tls.version = TlsVersion::Tls12;
        handover.tls.tx.secret = None;
        handover.tls.rx.secret = None;
        let sent = IntakeMessage::HandOver(Box::new(handover));
        let json = body(&sent);
        assert!(json.contains(r#","secret":null}"#), "{json}");
        assert_eq!(decode::<IntakeMessage>(json.as_bytes()).unwrap(), sent);
    }

    #[test]
    fn debug_of_a_hand_over_never_shows_keys_or_secrets() {
        let shown = format!("{:?}", IntakeMessage::HandOver(Box::new(hand_over())));
        assert!(!shown.contains("171"), "{shown}");
        assert!(!shown.contains("205"), "{shown}");
        assert!(shown.contains("steps: 3"), "{shown}");
        assert!(shown.contains(r#"user: "anna""#), "{shown}");
    }

    #[test]
    fn debug_of_a_pdu_step_shows_only_lengths() {
        let step = Step::Pdu {
            input: Zeroizing::new(b"hunter2".to_vec()),
            output: Zeroizing::new(vec![171; 3]),
        };
        assert_eq!(format!("{step:?}"), "Pdu { input_len: 7, output_len: 3 }");
    }

    #[test]
    fn intake_path_is_in_the_users_runtime_dir() {
        assert_eq!(
            intake_path(1000),
            PathBuf::from("/run/user/1000/reprise-rdp/handover.sock")
        );
    }
}
