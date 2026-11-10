//! Messages between reprise-login and its greeter over `GREETER_SOCKET`.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::Interface;
use crate::machine::{Background, LoginOutput};

#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GreeterRequest {
    Hello,
    CreateSession {
        user: String,
    },
    /// `None` answers an Info or Error message.
    PostAuthResponse {
        #[serde(with = "crate::frame::hex::text::option")]
        response: Option<Zeroizing<String>>,
    },
    StartSession,
    CancelSession,
    Power {
        action: PowerAction,
    },
    PowerAs {
        action: PowerAction,
        user: String,
    },
}

impl fmt::Debug for GreeterRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GreeterRequest::Hello => f.write_str("Hello"),
            GreeterRequest::CreateSession { user } => {
                f.debug_struct("CreateSession").field("user", user).finish()
            }
            GreeterRequest::PostAuthResponse { response } => {
                let shown = match response {
                    Some(_) => format_args!("<redacted>"),
                    None => format_args!("None"),
                };
                f.debug_struct("PostAuthResponse")
                    .field("response", &shown)
                    .finish()
            }
            GreeterRequest::StartSession => f.write_str("StartSession"),
            GreeterRequest::CancelSession => f.write_str("CancelSession"),
            GreeterRequest::Power { action } => {
                f.debug_struct("Power").field("action", action).finish()
            }
            GreeterRequest::PowerAs { action, user } => f
                .debug_struct("PowerAs")
                .field("action", action)
                .field("user", user)
                .finish(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GreeterReply {
    /// The answer to `Hello`, and again unasked when a running login screen comes forward.
    Greeting(Greeting),
    Success,
    AuthMessage {
        kind: AuthKind,
        text: String,
    },
    Error {
        kind: ErrorKind,
        text: String,
    },
    /// Others are logged on, so `action` waits for an administrator's password.
    AdminNeeded {
        action: PowerAction,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Greeting {
    pub interface: Interface,
    pub show_users: bool,
    pub users: Vec<UserEntry>,
    pub last_user: Option<String>,
    pub keyboard: Keyboard,
    pub notice: Option<Notice>,
    /// `MACHINE_CONFIG`'s `[[outputs]]`: what the login screen runs on each connector.
    #[serde(default)]
    pub outputs: Vec<LoginOutput>,
    #[serde(default)]
    pub background: Option<Background>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserEntry {
    pub name: String,
    pub real_name: String,
    pub picture: Option<PathBuf>,
    pub note: Option<Note>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Note {
    LoggedOn,
    /// Logged on from another computer.
    Remote,
    /// Logged on, with the session locked.
    Locked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Notice {
    SessionEndedEarly,
    /// A session's Switch User brought the screen: it opens on the users to pick from.
    SwitchUser,
}

/// XKB names, as in `XKB_DEFAULT_LAYOUT` and friends; empty means unset.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keyboard {
    pub layout: String,
    pub variant: String,
    pub options: String,
}

/// Visible and Secret want an answer; Info and Error are shown and answered with `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    Visible,
    Secret,
    Info,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Auth,
    /// The account step refused a correct password; the text is PAM's.
    Account,
    Refused,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerAction {
    PowerOff,
    Reboot,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{decode, encode};

    fn body<T: Serialize>(m: &T) -> String {
        String::from_utf8(encode(m).unwrap()[4..].to_vec()).unwrap()
    }

    #[test]
    fn wire_shapes_are_pinned() {
        assert_eq!(
            body(&GreeterRequest::CreateSession {
                user: "anna".into()
            }),
            r#"{"create_session":{"user":"anna"}}"#
        );
        assert_eq!(
            body(&GreeterReply::AuthMessage {
                kind: AuthKind::Secret,
                text: "Password:".into()
            }),
            r#"{"auth_message":{"kind":"secret","text":"Password:"}}"#
        );
        assert_eq!(
            body(&GreeterRequest::Power {
                action: PowerAction::PowerOff
            }),
            r#"{"power":{"action":"power_off"}}"#
        );
        assert_eq!(
            body(&GreeterRequest::PostAuthResponse { response: None }),
            r#"{"post_auth_response":{"response":null}}"#
        );
        assert_eq!(
            body(&GreeterReply::Error {
                kind: ErrorKind::Account,
                text: "User account has expired".into()
            }),
            r#"{"error":{"kind":"account","text":"User account has expired"}}"#
        );
        assert_eq!(
            body(&GreeterReply::AdminNeeded {
                action: PowerAction::PowerOff
            }),
            r#"{"admin_needed":{"action":"power_off"}}"#
        );
        assert_eq!(
            body(&GreeterRequest::PowerAs {
                action: PowerAction::Reboot,
                user: "anna".into()
            }),
            r#"{"power_as":{"action":"reboot","user":"anna"}}"#
        );
        assert_eq!(serde_json::to_string(&Note::Remote).unwrap(), r#""remote""#);
        assert_eq!(serde_json::to_string(&Note::Locked).unwrap(), r#""locked""#);
        assert_eq!(
            serde_json::to_string(&Notice::SwitchUser).unwrap(),
            r#""switch_user""#
        );
    }

    #[test]
    fn a_greeting_round_trips_under_its_tag() {
        let g = Greeting {
            interface: Interface::Classic,
            show_users: false,
            users: vec![],
            last_user: None,
            keyboard: Keyboard::default(),
            notice: Some(Notice::SessionEndedEarly),
            outputs: Vec::new(),
            background: None,
        };
        let json = body(&GreeterReply::Greeting(g.clone()));
        assert!(json.starts_with(r#"{"greeting":{"interface":"classic","#));
        assert!(json.contains(r#""notice":"session_ended_early""#));
        assert_eq!(
            decode::<GreeterReply>(json.as_bytes()).unwrap(),
            GreeterReply::Greeting(g)
        );
    }

    #[test]
    fn a_greeting_carries_the_outputs_and_the_background() {
        use crate::machine::{LoginMode, Transform};
        let g = Greeting {
            interface: Interface::Blue,
            show_users: true,
            users: vec![],
            last_user: None,
            keyboard: Keyboard::default(),
            notice: None,
            outputs: vec![LoginOutput {
                name: "DP-1".into(),
                mode: Some(LoginMode {
                    width: 2560,
                    height: 1440,
                    refresh_mhz: Some(59_951),
                }),
                quarters: 5,
                transform: Transform::Rotate270,
                position: Some((-2560, 0)),
                primary: true,
            }],
            background: Some(Background::Picture),
        };
        let json = body(&GreeterReply::Greeting(g.clone()));
        assert!(
            json.contains(
                r#""outputs":[{"name":"DP-1","mode":{"width":2560,"height":1440,"refresh_mhz":59951},"quarters":5,"transform":"270","position":[-2560,0],"primary":true}]"#
            ),
            "{json}"
        );
        assert!(json.ends_with(r#""background":"picture"}}"#), "{json}");
        assert_eq!(
            decode::<GreeterReply>(json.as_bytes()).unwrap(),
            GreeterReply::Greeting(g)
        );
        let builtin = serde_json::to_string(&Background::Builtin("reprise:tarn/dusk".into()));
        assert_eq!(builtin.unwrap(), r#"{"builtin":"reprise:tarn/dusk"}"#);
    }

    #[test]
    fn a_greeting_without_outputs_or_background_still_decodes() {
        let json = r#"{"greeting":{"interface":"glass","show_users":true,"users":[],"last_user":null,"keyboard":{"layout":"","variant":"","options":""},"notice":null}}"#;
        let GreeterReply::Greeting(g) = decode::<GreeterReply>(json.as_bytes()).unwrap() else {
            panic!("a greeting");
        };
        assert_eq!((g.outputs, g.background), (vec![], None));
    }

    #[test]
    fn a_response_round_trips_in_hex() {
        let sent = GreeterRequest::PostAuthResponse {
            response: Some(Zeroizing::new("hun\"ter2".into())),
        };
        let json = body(&sent);
        assert_eq!(
            json,
            r#"{"post_auth_response":{"response":"68756e2274657232"}}"#
        );
        assert_eq!(decode::<GreeterRequest>(json.as_bytes()).unwrap(), sent);
        let none = GreeterRequest::PostAuthResponse { response: None };
        assert_eq!(
            decode::<GreeterRequest>(body(&none).as_bytes()).unwrap(),
            none
        );
    }

    #[test]
    fn debug_of_a_response_never_shows_it() {
        let m = GreeterRequest::PostAuthResponse {
            response: Some(Zeroizing::new("hunter2".into())),
        };
        let shown = format!("{m:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
        assert_eq!(shown, "PostAuthResponse { response: <redacted> }");
        assert_eq!(
            format!(
                "{:?}",
                GreeterRequest::CreateSession {
                    user: "anna".into()
                }
            ),
            r#"CreateSession { user: "anna" }"#
        );
        assert_eq!(
            format!(
                "{:?}",
                GreeterRequest::PowerAs {
                    action: PowerAction::Reboot,
                    user: "anna".into()
                }
            ),
            r#"PowerAs { action: Reboot, user: "anna" }"#
        );
    }
}
