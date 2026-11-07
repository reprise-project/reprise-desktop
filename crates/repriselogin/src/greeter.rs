//! Messages between reprise-login and its greeter over `GREETER_SOCKET`.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::Interface;

#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GreeterRequest {
    Hello,
    CreateSession {
        user: String,
    },
    /// `None` answers an Info or Error message.
    PostAuthResponse {
        response: Option<Zeroizing<String>>,
    },
    StartSession,
    CancelSession,
    Power {
        action: PowerAction,
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
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Greeting {
    pub interface: Interface,
    pub show_users: bool,
    pub users: Vec<UserEntry>,
    pub last_user: Option<String>,
    pub keyboard: Keyboard,
    pub notice: Option<Notice>,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Notice {
    SessionEndedEarly,
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
            r#"{"type":"create_session","user":"anna"}"#
        );
        assert_eq!(
            body(&GreeterReply::AuthMessage {
                kind: AuthKind::Secret,
                text: "Password:".into()
            }),
            r#"{"type":"auth_message","kind":"secret","text":"Password:"}"#
        );
        assert_eq!(
            body(&GreeterRequest::Power {
                action: PowerAction::PowerOff
            }),
            r#"{"type":"power","action":"power_off"}"#
        );
        assert_eq!(
            body(&GreeterRequest::PostAuthResponse { response: None }),
            r#"{"type":"post_auth_response","response":null}"#
        );
        assert_eq!(
            body(&GreeterReply::Error {
                kind: ErrorKind::Account,
                text: "User account has expired".into()
            }),
            r#"{"type":"error","kind":"account","text":"User account has expired"}"#
        );
    }

    #[test]
    fn a_greeting_carries_its_fields_beside_the_tag() {
        let g = Greeting {
            interface: Interface::Classic,
            show_users: false,
            users: vec![],
            last_user: None,
            keyboard: Keyboard::default(),
            notice: Some(Notice::SessionEndedEarly),
        };
        let json = body(&GreeterReply::Greeting(g.clone()));
        assert!(json.starts_with(r#"{"type":"greeting","interface":"classic","#));
        assert!(json.contains(r#""notice":"session_ended_early""#));
        assert_eq!(
            decode::<GreeterReply>(json.as_bytes()).unwrap(),
            GreeterReply::Greeting(g)
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
    }
}
