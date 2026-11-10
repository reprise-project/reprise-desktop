//! The login screen's side of reprise-login's conversation: which request each reply calls for.
//! Pure, so the order of prompts, notes and results is tested without a socket.

use reprisekit::logon::{self, Logon, Prompt, Purpose};
use repriselogin::greeter::{AuthKind, ErrorKind, GreeterReply, GreeterRequest, PowerAction};
use zeroize::Zeroizing;

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Send(GreeterRequest),
    /// A question after the password, for the field to take.
    Ask(Prompt),
    Note(String),
    Fail(Failure),
    /// The session started: the greeter has done its job.
    Started,
    /// Turning off or restarting needs an administrator, since others are logged on.
    Admin(logon::PowerAction),
    /// An administrator's power request succeeded; the computer is going down.
    GoingDown,
    /// A cancelled login ended; nothing is in progress.
    Idle,
}

/// What the screen says when a request fails.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// The name or password is wrong.
    Incorrect,
    /// PAM's own words: its notes from this login, or else its refusal of the account.
    Pam(String),
    /// The login could not run; the daemon's reason is for the journal.
    Failed,
    /// Turning off or restarting is refused: others are logged on, or the account may not.
    PowerRefused,
    PowerFailed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    Authenticating,
    Starting,
    /// An administrator's power request succeeded; nothing more is sent, and only the power
    /// action's failure is heard.
    GoingDown,
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
    /// This login's info and error notes, cleaned like the lock screen's.
    notes: Vec<String>,
    /// The conversation in progress is a `PowerAs`'s, so its success turns the computer off.
    admin: bool,
}

impl Conversation {
    /// Whether a new login may start.
    pub fn idle(&self) -> bool {
        self.phase == Phase::Idle && !self.power
    }

    /// `user` logs on, or under [`Purpose::Admin`] asks for its power action as an administrator;
    /// either way the conversation runs as a login's does.
    pub fn submit(
        &mut self,
        purpose: Purpose,
        user: String,
        password: Zeroizing<String>,
    ) -> GreeterRequest {
        self.phase = Phase::Authenticating;
        self.password = Some(password);
        self.asked = true;
        self.notes.clear();
        match purpose {
            Purpose::Login => {
                self.admin = false;
                GreeterRequest::CreateSession { user }
            }
            Purpose::Unlock { .. } => unreachable!("the greeter's screen is never a lock"),
            Purpose::Admin(action) => {
                self.admin = true;
                GreeterRequest::PowerAs {
                    action: wire(action),
                    user,
                }
            }
        }
    }

    /// The screen's submit for `user`: the waiting prompt's answer, else a new login or power
    /// request. The password field is wiped and the screen busy once a request goes out.
    pub fn on_submit(&mut self, m: &mut Logon, user: String) -> Option<GreeterRequest> {
        let text = Zeroizing::new(m.password.text.clone());
        let req = if m.prompt.is_some() {
            self.answer(text)
        } else if self.idle() {
            Some(self.submit(m.purpose, user, text))
        } else {
            None
        }?;
        m.sent();
        Some(req)
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
        if matches!(self.phase, Phase::Starting | Phase::GoingDown) || self.power {
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
            // An administrator's success the held requests crossed: the computer is going down.
            (Phase::Cancelling, GreeterReply::Success) if answered && self.admin => {
                self.held.clear();
                self.power = false;
                self.phase = Phase::GoingDown;
                vec![Step::GoingDown]
            }
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
                    repriselogin::add_prompt_lines(&mut self.notes, &text);
                    vec![
                        Step::Note(text),
                        Step::Send(GreeterRequest::PostAuthResponse { response: None }),
                    ]
                }
            },
            (Phase::Authenticating, GreeterReply::Success) if self.admin => {
                self.phase = Phase::GoingDown;
                self.password = None;
                vec![Step::GoingDown]
            }
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
                vec![Step::Fail(self.failure(kind, &text))]
            }
            // The power action the administrator's success started failed.
            (Phase::GoingDown, GreeterReply::Error { .. }) => {
                self.phase = Phase::Idle;
                vec![Step::Fail(Failure::PowerFailed)]
            }
            (Phase::Cancelling, GreeterReply::Success | GreeterReply::Error { .. }) => {
                self.phase = Phase::Idle;
                vec![Step::Idle]
            }
            (Phase::Idle, GreeterReply::Success) if self.power => {
                self.power = false;
                Vec::new()
            }
            (Phase::Idle, GreeterReply::AdminNeeded { action }) if self.power => {
                self.power = false;
                vec![Step::Admin(kit(action))]
            }
            (Phase::Idle, GreeterReply::Error { kind, .. }) if self.power => {
                self.power = false;
                vec![Step::Fail(match kind {
                    ErrorKind::Refused => Failure::PowerRefused,
                    _ => Failure::PowerFailed,
                })]
            }
            _ => Vec::new(),
        }
    }

    /// A failed login's words: PAM's notes when it sent any, as the lock screen shows them.
    fn failure(&self, kind: ErrorKind, text: &str) -> Failure {
        let notes = (!self.notes.is_empty()).then(|| self.notes.join("\n"));
        match kind {
            ErrorKind::Refused if self.admin => Failure::PowerRefused,
            ErrorKind::Failed if self.admin => Failure::PowerFailed,
            ErrorKind::Auth => notes.map_or(Failure::Incorrect, Failure::Pam),
            ErrorKind::Account => {
                Failure::Pam(notes.unwrap_or_else(|| repriselogin::clean_prompt(text).join("\n")))
            }
            ErrorKind::Refused | ErrorKind::Failed => Failure::Failed,
        }
    }
}

/// Puts `step` on the screen; `Send` and `Started` are the host's.
pub fn show(m: &mut Logon, step: Step) {
    match step {
        Step::Send(_) | Step::Started => {}
        Step::Ask(prompt) => m.ask(prompt),
        Step::Note(text) => m.info(text),
        Step::Fail(f) => m.fail(match f {
            Failure::Incorrect => logon::text::incorrect(),
            Failure::Pam(text) => text,
            Failure::Failed => logon::text::failed(),
            Failure::PowerRefused => logon::text::power_refused(),
            Failure::PowerFailed => logon::text::power_failed(),
        }),
        Step::Idle => m.set_busy(false),
        Step::Admin(action) => m.admin(action),
        Step::GoingDown => m.going_down(),
    }
}

fn wire(action: logon::PowerAction) -> PowerAction {
    match action {
        logon::PowerAction::PowerOff => PowerAction::PowerOff,
        logon::PowerAction::Reboot => PowerAction::Reboot,
    }
}

fn kit(action: PowerAction) -> logon::PowerAction {
    match action {
        PowerAction::PowerOff => logon::PowerAction::PowerOff,
        PowerAction::Reboot => logon::PowerAction::Reboot,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::interface::Interface;
    use reprisekit::logon::{self, LogonAction, LogonKey};

    fn prompt(kind: AuthKind, text: &str) -> GreeterReply {
        GreeterReply::AuthMessage {
            kind,
            text: text.into(),
        }
    }

    fn submitted() -> Conversation {
        let mut c = Conversation::default();
        let req = c.submit(
            Purpose::Login,
            "anna".into(),
            Zeroizing::new("hunter2".into()),
        );
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

    fn error(kind: ErrorKind, text: &str) -> GreeterReply {
        GreeterReply::Error {
            kind,
            text: text.into(),
        }
    }

    #[test]
    fn an_auth_error_clears_the_password_and_says_incorrect() {
        let mut c = submitted();
        assert_eq!(
            c.on_reply(error(ErrorKind::Auth, "Authentication failure")),
            vec![Step::Fail(Failure::Incorrect)]
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
            vec![Step::Fail(Failure::PowerRefused)]
        );
        assert!(c.idle());
    }

    #[test]
    fn a_failed_power_request_is_a_power_failure() {
        let mut c = Conversation::default();
        c.power(PowerAction::Reboot);
        assert_eq!(
            c.on_reply(GreeterReply::Error {
                kind: ErrorKind::Failed,
                text: "org.freedesktop.DBus.Error.AccessDenied".into()
            }),
            vec![Step::Fail(Failure::PowerFailed)]
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
        c.submit(Purpose::Login, "bob".into(), Zeroizing::new("pw".into()));
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

    #[test]
    fn a_refused_or_failed_login_is_a_fixed_failure() {
        for kind in [ErrorKind::Refused, ErrorKind::Failed] {
            let mut c = submitted();
            c.on_reply(prompt(AuthKind::Info, "Welcome"));
            assert_eq!(
                c.on_reply(error(kind, "another login is in progress")),
                vec![Step::Fail(Failure::Failed)],
                "{kind:?}"
            );
        }
    }

    #[test]
    fn an_auth_error_after_notes_keeps_them() {
        let mut c = submitted();
        c.on_reply(prompt(AuthKind::Secret, "Password:"));
        let locked = "The account is locked due to 3 failed logins.\n\t(5 minutes left to unlock)";
        c.on_reply(prompt(AuthKind::Error, locked));
        assert_eq!(
            c.on_reply(error(ErrorKind::Auth, "Authentication failure")),
            vec![Step::Fail(Failure::Pam(
                "The account is locked due to 3 failed logins.\n(5 minutes left to unlock)".into()
            ))]
        );
        c.submit(Purpose::Login, "anna".into(), Zeroizing::new("pw".into()));
        assert_eq!(
            c.on_reply(error(ErrorKind::Auth, "Authentication failure")),
            vec![Step::Fail(Failure::Incorrect)],
            "the next login starts without them"
        );
    }

    #[test]
    fn an_account_refusal_shows_its_notes_or_else_pams_text() {
        let mut c = submitted();
        c.on_reply(prompt(AuthKind::Secret, "Password:"));
        assert_eq!(
            c.on_reply(error(ErrorKind::Account, "User account has expired")),
            vec![Step::Fail(Failure::Pam("User account has expired".into()))]
        );
        let mut c = submitted();
        let note = "Your account has expired; please contact your system administrator.";
        c.on_reply(prompt(AuthKind::Error, note));
        assert_eq!(
            c.on_reply(error(ErrorKind::Account, "User account has expired")),
            vec![Step::Fail(Failure::Pam(note.into()))]
        );
    }

    /// The screen's model and the conversation, joined as the host joins them.
    struct Screen {
        m: Logon,
        c: Conversation,
    }

    impl Screen {
        fn new() -> Screen {
            Screen {
                m: Logon::new(Interface::Classic, Vec::new(), false, Some("bob")),
                c: Conversation::default(),
            }
        }

        /// The requests `reply` sends; every other step is shown.
        fn reply(&mut self, reply: GreeterReply) -> Vec<GreeterRequest> {
            let mut out = Vec::new();
            for step in self.c.on_reply(reply) {
                match step {
                    Step::Send(req) => out.push(req),
                    step => show(&mut self.m, step),
                }
            }
            out
        }

        /// The request `key` sends.
        fn key(&mut self, key: LogonKey) -> Option<GreeterRequest> {
            match self.m.key(key)? {
                LogonAction::Submit { user } => self.c.on_submit(&mut self.m, user),
                LogonAction::Cancel => self.c.cancel(),
                _ => None,
            }
        }

        fn typed(&mut self, text: &str) {
            assert_eq!(self.key(LogonKey::Text(text)), None);
        }
    }

    /// A power-off refused for others' sessions: the administrator's form is up.
    fn admin_form() -> Screen {
        let mut s = Screen::new();
        assert_eq!(
            s.c.power(PowerAction::PowerOff),
            vec![GreeterRequest::Power {
                action: PowerAction::PowerOff
            }]
        );
        assert_eq!(
            s.reply(GreeterReply::AdminNeeded {
                action: PowerAction::PowerOff
            }),
            vec![]
        );
        s
    }

    #[test]
    fn admin_needed_turns_the_form_to_an_administrators_and_power_as_follows() {
        let mut s = admin_form();
        assert_eq!(s.m.purpose, Purpose::Admin(logon::PowerAction::PowerOff));
        assert_eq!(s.m.name.text, "", "the last user's name is not offered");
        assert!(s.c.idle(), "the refused power request is over");
        s.typed("anna");
        assert_eq!(s.key(LogonKey::Enter), None, "Enter on the name moves on");
        s.typed("hunter2");
        assert_eq!(
            s.key(LogonKey::Enter),
            Some(GreeterRequest::PowerAs {
                action: PowerAction::PowerOff,
                user: "anna".into()
            })
        );
        assert_eq!(
            s.reply(prompt(AuthKind::Secret, "Password:")),
            vec![GreeterRequest::PostAuthResponse {
                response: Some(Zeroizing::new("hunter2".into()))
            }],
            "the typed password answers, as a login's does"
        );
    }

    #[test]
    fn an_administrators_success_shuts_down_and_starts_no_session() {
        let mut s = admin_form();
        s.typed("anna");
        s.key(LogonKey::Enter);
        s.typed("hunter2");
        s.key(LogonKey::Enter);
        s.reply(prompt(AuthKind::Secret, "Password:"));
        assert_eq!(s.reply(GreeterReply::Success), vec![], "no StartSession");
        assert!(s.m.busy, "the form stays busy");
        assert_eq!(s.m.busy_text(), logon::text::shutting_down());
        assert!(!s.c.idle(), "no login starts while the computer goes down");
        assert_eq!(s.c.power(PowerAction::Reboot), vec![]);
        assert_eq!(s.c.cancel(), None);
        assert_eq!(s.reply(GreeterReply::Success), vec![]);
    }

    /// The administrator's form after a power-off's password went out, its reply in flight.
    fn admin_answered() -> Screen {
        let mut s = admin_form();
        s.typed("anna");
        s.key(LogonKey::Enter);
        s.typed("hunter2");
        s.key(LogonKey::Enter);
        s.reply(prompt(AuthKind::Secret, "Password:"));
        s
    }

    #[test]
    fn a_power_off_failing_after_the_administrators_success_gives_the_form_back() {
        let mut s = admin_answered();
        s.reply(GreeterReply::Success);
        assert_eq!(
            s.reply(error(ErrorKind::Failed, "org.freedesktop.login1 failed")),
            vec![]
        );
        assert!(s.c.idle(), "the conversation is over");
        assert!(!s.m.busy, "the form takes input again");
        assert_eq!(s.m.message, Some(logon::text::power_failed()));
        assert_eq!(s.m.busy_text(), logon::text::verifying());
        assert_eq!(s.m.purpose, Purpose::Admin(logon::PowerAction::PowerOff));
        s.typed("hunter2");
        assert_eq!(
            s.key(LogonKey::Enter),
            Some(GreeterRequest::PowerAs {
                action: PowerAction::PowerOff,
                user: "anna".into()
            }),
            "the administrator may try again"
        );
        assert_eq!(s.m.busy_text(), logon::text::verifying());
    }

    #[test]
    fn an_administrators_success_crossing_a_power_request_still_goes_down() {
        let mut s = admin_answered();
        assert_eq!(s.c.power(PowerAction::Reboot), vec![], "held for the reply");
        assert_eq!(
            s.reply(GreeterReply::Success),
            vec![],
            "the held cancel and power request are dropped"
        );
        assert!(s.m.busy);
        assert_eq!(s.m.busy_text(), logon::text::shutting_down());
        assert!(!s.c.idle());
        assert_eq!(s.reply(error(ErrorKind::Failed, "logind")), vec![]);
        assert!(s.c.idle(), "the dropped power request waits for nothing");
        assert_eq!(s.m.message, Some(logon::text::power_failed()));
        s.typed("hunter2");
        assert_eq!(
            s.key(LogonKey::Enter),
            Some(GreeterRequest::PowerAs {
                action: PowerAction::PowerOff,
                user: "anna".into()
            }),
            "the administrator may try again"
        );
    }

    #[test]
    fn a_wrong_administrators_password_keeps_the_form() {
        let mut s = admin_form();
        s.typed("anna");
        s.key(LogonKey::Enter);
        s.typed("wrong");
        s.key(LogonKey::Enter);
        s.reply(prompt(AuthKind::Secret, "Password:"));
        assert_eq!(
            s.reply(error(ErrorKind::Auth, "Authentication failure")),
            vec![]
        );
        assert_eq!(s.m.purpose, Purpose::Admin(logon::PowerAction::PowerOff));
        assert_eq!(s.m.message, Some(logon::text::incorrect()));
        s.typed("hunter2");
        assert_eq!(
            s.key(LogonKey::Enter),
            Some(GreeterRequest::PowerAs {
                action: PowerAction::PowerOff,
                user: "anna".into()
            }),
            "the next try asks again"
        );
        let mut s = admin_form();
        s.c.submit(s.m.purpose, "anna".into(), Zeroizing::new("hunter2".into()));
        assert_eq!(s.reply(error(ErrorKind::Failed, "logind")), vec![]);
        assert_eq!(s.m.message, Some(logon::text::power_failed()));
    }

    #[test]
    fn cancel_leaves_the_administrators_form() {
        let mut s = admin_form();
        s.typed("anna");
        s.key(LogonKey::Enter);
        s.typed("hunter2");
        s.key(LogonKey::Enter);
        s.reply(prompt(AuthKind::Secret, "Password:"));
        s.reply(prompt(AuthKind::Secret, "Token:"));
        assert_eq!(s.key(LogonKey::Escape), Some(GreeterRequest::CancelSession));
        assert_eq!(s.m.purpose, Purpose::Login);
        assert_eq!(s.reply(GreeterReply::Success), vec![]);
        assert!(s.c.idle());
        s.typed("bob");
        s.key(LogonKey::Enter);
        s.typed("pw");
        assert_eq!(
            s.key(LogonKey::Enter),
            Some(GreeterRequest::CreateSession { user: "bob".into() }),
            "the next submit is a login again"
        );

        let mut s = admin_form();
        assert_eq!(
            s.key(LogonKey::Escape),
            None,
            "an empty form has no conversation to cancel"
        );
        assert_eq!(s.m.purpose, Purpose::Login);
    }
}
