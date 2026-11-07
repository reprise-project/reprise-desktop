//! The login screen's side of reprise-login's conversation: which request each reply calls for.
//! Pure, so the order of prompts, notes and results is tested without a socket.

use reprisekit::logon::Prompt;
use repriselogin::greeter::{AuthKind, ErrorKind, GreeterReply, GreeterRequest, PowerAction};
use zeroize::Zeroizing;

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Send(GreeterRequest),
    /// A question after the password, for the field to take.
    Ask(Prompt),
    Note(String),
    Fail(ErrorKind, String),
    /// The session started: the greeter has done its job.
    Started,
    /// A cancelled login ended; nothing is in progress.
    Idle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    Authenticating,
    Starting,
    /// The login was cancelled; what it still sends is dropped until the cancel's reply.
    Cancelling,
}

#[derive(Default)]
pub struct Conversation {
    phase: Phase,
    /// Answers the first secret prompt, then is gone.
    password: Option<Zeroizing<String>>,
    /// A `CreateSession` or `PostAuthResponse` went out and its reply has not come.
    asked: bool,
    /// Requests held while `asked`: a cancel sent then could not be told from the reply it
    /// crosses, since both may be `Success`.
    held: Vec<GreeterRequest>,
    /// A `Power` request waits for its result. It goes out only while no login runs, so a
    /// result in `Idle` is its.
    power: bool,
}

impl Conversation {
    /// Whether a new login may start.
    pub fn idle(&self) -> bool {
        self.phase == Phase::Idle && !self.power
    }

    pub fn submit(&mut self, user: String, password: Zeroizing<String>) -> GreeterRequest {
        self.phase = Phase::Authenticating;
        self.password = Some(password);
        self.asked = true;
        GreeterRequest::CreateSession { user }
    }

    /// The answer to the prompt `Step::Ask` showed; None once that login is cancelled or over.
    pub fn answer(&mut self, text: Zeroizing<String>) -> Option<GreeterRequest> {
        if self.phase != Phase::Authenticating || self.asked {
            return None;
        }
        self.asked = true;
        Some(GreeterRequest::PostAuthResponse {
            response: Some(text),
        })
    }

    /// Drops the login in progress; None while there is none, it is already starting, or a
    /// reply is in flight (the cancel then goes out after it).
    pub fn cancel(&mut self) -> Option<GreeterRequest> {
        if self.phase != Phase::Authenticating {
            return None;
        }
        self.phase = Phase::Cancelling;
        self.password = None;
        self.send(GreeterRequest::CancelSession)
    }

    /// The requests that ask for `action`: a login still authenticating is cancelled first, and
    /// one that is starting keeps the machine up.
    pub fn power(&mut self, action: PowerAction) -> Vec<GreeterRequest> {
        if self.phase == Phase::Starting || self.power {
            return Vec::new();
        }
        let mut out: Vec<GreeterRequest> = self.cancel().into_iter().collect();
        self.power = true;
        out.extend(self.send(GreeterRequest::Power { action }));
        out
    }

    /// `req` now, or held until the reply in flight comes.
    fn send(&mut self, req: GreeterRequest) -> Option<GreeterRequest> {
        if self.asked {
            self.held.push(req);
            return None;
        }
        Some(req)
    }

    pub fn on_reply(&mut self, reply: GreeterReply) -> Vec<Step> {
        if matches!(reply, GreeterReply::Greeting(_)) {
            return Vec::new();
        }
        let answered = std::mem::take(&mut self.asked);
        match (self.phase, reply) {
            // The reply the held cancel waited out; after an error there is no login to cancel.
            (Phase::Cancelling, reply) if answered => {
                let mut held = std::mem::take(&mut self.held);
                let mut out = Vec::new();
                if matches!(reply, GreeterReply::Error { .. }) {
                    self.phase = Phase::Idle;
                    held.retain(|r| *r != GreeterRequest::CancelSession);
                    out.push(Step::Idle);
                }
                out.extend(held.into_iter().map(Step::Send));
                out
            }
            (Phase::Authenticating, GreeterReply::AuthMessage { kind, text }) => match kind {
                AuthKind::Secret | AuthKind::Visible => {
                    let password = (kind == AuthKind::Secret)
                        .then(|| self.password.take())
                        .flatten();
                    match password {
                        Some(p) => {
                            self.asked = true;
                            vec![Step::Send(GreeterRequest::PostAuthResponse {
                                response: Some(p),
                            })]
                        }
                        None => vec![Step::Ask(Prompt {
                            text,
                            echo: kind == AuthKind::Visible,
                        })],
                    }
                }
                AuthKind::Info | AuthKind::Error => {
                    self.asked = true;
                    vec![
                        Step::Note(text),
                        Step::Send(GreeterRequest::PostAuthResponse { response: None }),
                    ]
                }
            },
            (Phase::Authenticating, GreeterReply::Success) => {
                self.phase = Phase::Starting;
                self.password = None;
                vec![Step::Send(GreeterRequest::StartSession)]
            }
            (Phase::Starting, GreeterReply::Success) => {
                self.phase = Phase::Idle;
                vec![Step::Started]
            }
            (Phase::Authenticating | Phase::Starting, GreeterReply::Error { kind, text }) => {
                self.phase = Phase::Idle;
                self.password = None;
                vec![Step::Fail(kind, text)]
            }
            (Phase::Cancelling, GreeterReply::Success | GreeterReply::Error { .. }) => {
                self.phase = Phase::Idle;
                vec![Step::Idle]
            }
            (Phase::Idle, GreeterReply::Success) if self.power => {
                self.power = false;
                Vec::new()
            }
            (Phase::Idle, GreeterReply::Error { kind, text }) if self.power => {
                self.power = false;
                vec![Step::Fail(kind, text)]
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(kind: AuthKind, text: &str) -> GreeterReply {
        GreeterReply::AuthMessage {
            kind,
            text: text.into(),
        }
    }

    fn submitted() -> Conversation {
        let mut c = Conversation::default();
        let req = c.submit("anna".into(), Zeroizing::new("hunter2".into()));
        assert_eq!(
            req,
            GreeterRequest::CreateSession {
                user: "anna".into()
            }
        );
        c
    }

    fn answer(text: &str) -> Step {
        Step::Send(GreeterRequest::PostAuthResponse {
            response: Some(Zeroizing::new(text.into())),
        })
    }

    #[test]
    fn the_first_secret_prompt_is_answered_with_the_typed_password() {
        let mut c = submitted();
        assert_eq!(
            c.on_reply(prompt(AuthKind::Secret, "Password:")),
            vec![answer("hunter2")]
        );
    }

    #[test]
    fn a_second_secret_prompt_relabels_the_field() {
        let mut c = submitted();
        c.on_reply(prompt(AuthKind::Secret, "Password:"));
        assert_eq!(
            c.on_reply(prompt(AuthKind::Secret, "Token:")),
            vec![Step::Ask(Prompt {
                text: "Token:".into(),
                echo: false
            })]
        );
        assert_eq!(
            c.on_reply(prompt(AuthKind::Visible, "Code:")),
            vec![Step::Ask(Prompt {
                text: "Code:".into(),
                echo: true
            })]
        );
        assert_eq!(
            c.answer(Zeroizing::new("123".into())),
            Some(GreeterRequest::PostAuthResponse {
                response: Some(Zeroizing::new("123".into()))
            })
        );
    }

    #[test]
    fn info_is_shown_and_acknowledged() {
        let mut c = submitted();
        assert_eq!(
            c.on_reply(prompt(AuthKind::Info, "Password expires in 3 days")),
            vec![
                Step::Note("Password expires in 3 days".into()),
                Step::Send(GreeterRequest::PostAuthResponse { response: None })
            ]
        );
        assert_eq!(
            c.on_reply(prompt(AuthKind::Secret, "Password:")),
            vec![answer("hunter2")],
            "a note does not use up the password"
        );
    }

    #[test]
    fn success_after_auth_starts_the_session() {
        let mut c = submitted();
        c.on_reply(prompt(AuthKind::Secret, "Password:"));
        assert_eq!(
            c.on_reply(GreeterReply::Success),
            vec![Step::Send(GreeterRequest::StartSession)]
        );
        assert_eq!(c.on_reply(GreeterReply::Success), vec![Step::Started]);
    }

    #[test]
    fn an_auth_error_clears_the_password_and_shows_the_text() {
        let mut c = submitted();
        let error = GreeterReply::Error {
            kind: ErrorKind::Auth,
            text: "Authentication failure".into(),
        };
        assert_eq!(
            c.on_reply(error),
            vec![Step::Fail(ErrorKind::Auth, "Authentication failure".into())]
        );
        assert!(c.idle(), "the conversation is back to idle");
        assert_eq!(
            c.on_reply(prompt(AuthKind::Secret, "Password:")),
            vec![],
            "nothing answers a prompt once the login failed"
        );
    }

    /// A login past its password, asking `Token:`: no reply is in flight.
    fn asking() -> Conversation {
        let mut c = submitted();
        c.on_reply(prompt(AuthKind::Secret, "Password:"));
        c.on_reply(prompt(AuthKind::Secret, "Token:"));
        c
    }

    #[test]
    fn power_during_a_login_cancels_it_first_and_its_refusal_fails() {
        let mut c = asking();
        assert_eq!(
            c.power(PowerAction::PowerOff),
            vec![
                GreeterRequest::CancelSession,
                GreeterRequest::Power {
                    action: PowerAction::PowerOff
                }
            ]
        );
        assert_eq!(
            c.answer(Zeroizing::new("123".into())),
            None,
            "the cancelled prompt takes no answer"
        );
        assert_eq!(c.on_reply(GreeterReply::Success), vec![Step::Idle]);
        assert!(!c.idle(), "the power request is still out");
        assert_eq!(
            c.on_reply(GreeterReply::Error {
                kind: ErrorKind::Refused,
                text: "other users are logged on".into()
            }),
            vec![Step::Fail(
                ErrorKind::Refused,
                "other users are logged on".into()
            )]
        );
        assert!(c.idle());
    }

    #[test]
    fn a_cancel_goes_out_after_the_reply_it_would_cross() {
        let mut c = submitted();
        c.on_reply(prompt(AuthKind::Secret, "Password:"));
        assert_eq!(c.cancel(), None, "the answer's reply is still to come");
        assert_eq!(
            c.on_reply(GreeterReply::Success),
            vec![Step::Send(GreeterRequest::CancelSession)],
            "the login's success is not the cancel's"
        );
        assert!(!c.idle());
        assert_eq!(c.on_reply(GreeterReply::Success), vec![Step::Idle]);
        assert!(c.idle());
        c.submit("bob".into(), Zeroizing::new("pw".into()));
        assert_eq!(
            c.on_reply(GreeterReply::Success),
            vec![Step::Send(GreeterRequest::StartSession)],
            "the next login's success is its own"
        );
    }

    #[test]
    fn power_during_a_reply_in_flight_waits_for_it() {
        let mut c = submitted();
        assert_eq!(c.power(PowerAction::Reboot), vec![]);
        assert_eq!(
            c.on_reply(prompt(AuthKind::Secret, "Password:")),
            vec![
                Step::Send(GreeterRequest::CancelSession),
                Step::Send(GreeterRequest::Power {
                    action: PowerAction::Reboot
                })
            ],
            "the prompt goes unanswered"
        );
        assert_eq!(c.on_reply(GreeterReply::Success), vec![Step::Idle]);
        assert_eq!(c.on_reply(GreeterReply::Success), vec![]);
        assert!(c.idle());
    }

    #[test]
    fn a_login_failing_before_its_cancel_went_out_needs_none() {
        let mut c = submitted();
        assert_eq!(c.cancel(), None);
        assert_eq!(
            c.on_reply(GreeterReply::Error {
                kind: ErrorKind::Auth,
                text: "Authentication failure".into()
            }),
            vec![Step::Idle]
        );
        assert!(c.idle());
    }

    #[test]
    fn a_starting_session_keeps_the_machine_up() {
        let mut c = submitted();
        c.on_reply(GreeterReply::Success);
        assert_eq!(c.power(PowerAction::Reboot), vec![]);
        assert_eq!(c.cancel(), None);
    }
}
