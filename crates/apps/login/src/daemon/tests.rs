use repriselogin::frame::{MAX_FRAME, encode};
use repriselogin::greeter::{AuthKind, ErrorKind, Greeting, Note};
use repriselogin::{GREETER_USER, Interface};
use zeroize::Zeroizing;

use super::*;

const GREETER: WorkerId = WorkerId(1);

fn user(name: &str) -> UserEntry {
    UserEntry {
        name: name.into(),
        real_name: name.to_uppercase(),
        picture: None,
        note: None,
    }
}

fn world() -> World {
    World {
        users: vec![user("anna"), user("bert")],
        keyboard: Keyboard {
            layout: "de".into(),
            ..Keyboard::default()
        },
        kernel_open: 0,
    }
}

fn logind(id: &str, user: &str, uid: u32, vt: u16, class: &str, service: &str) -> LogindSession {
    LogindSession {
        id: id.into(),
        user: user.into(),
        uid,
        vt,
        seat: "seat0".into(),
        class: class.into(),
        service: service.into(),
    }
}

fn anna_on_7() -> LogindSession {
    logind("c1", "anna", 1000, 7, "user", "reprise-login")
}

fn greeter_spawn(w: u32) -> Vec<Effect> {
    vec![
        Effect::Spawn {
            worker: WorkerId(w),
            job: Job {
                mode: Mode::Greeter,
                user: GREETER_USER.into(),
            },
        },
        Effect::ToWorker {
            worker: WorkerId(w),
            msg: ToWorker::Start { vt: 1 },
        },
    ]
}

fn spawn(w: u32, mode: Mode, user: &str) -> Vec<Effect> {
    vec![Effect::Spawn {
        worker: WorkerId(w),
        job: Job {
            mode,
            user: user.into(),
        },
    }]
}

fn reply(r: GreeterReply) -> Vec<Effect> {
    vec![Effect::Reply(r)]
}

fn error(kind: ErrorKind, text: &str) -> GreeterReply {
    GreeterReply::Error {
        kind,
        text: text.into(),
    }
}

fn secret(text: &str) -> FromWorker {
    FromWorker::Prompt {
        kind: AuthKind::Secret,
        text: text.into(),
    }
}

fn answer(text: &str) -> GreeterRequest {
    GreeterRequest::PostAuthResponse {
        response: Some(Zeroizing::new(text.into())),
    }
}

fn create(user: &str) -> GreeterRequest {
    GreeterRequest::CreateSession { user: user.into() }
}

fn started(id: &str, uid: u32) -> FromWorker {
    FromWorker::Started {
        session_id: id.into(),
        uid,
    }
}

fn is_refused(effects: &[Effect]) -> bool {
    matches!(
        effects,
        [Effect::Reply(GreeterReply::Error {
            kind: ErrorKind::Refused,
            ..
        })]
    )
}

/// A daemon driven by hand, with a clock the test moves.
struct Run {
    d: Daemon,
    now: Instant,
    world: World,
}

impl Run {
    fn new(config: LoginConfig, remembered: Remembered) -> Run {
        Run {
            d: Daemon::new(config, remembered),
            now: Instant::now(),
            world: world(),
        }
    }

    /// Recovers nothing and starts, which brings up the greeter as worker 1.
    fn booted() -> Run {
        let mut run = Run::new(LoginConfig::default(), Remembered::default());
        assert_eq!(run.recover(vec![]), vec![]);
        assert_eq!(run.start(), greeter_spawn(1));
        run
    }

    fn recover(&mut self, found: Vec<LogindSession>) -> Vec<Effect> {
        self.d.recover(self.now, found)
    }

    fn start(&mut self) -> Vec<Effect> {
        self.d.start(self.now, &self.world)
    }

    fn on(&mut self, input: Input) -> Vec<Effect> {
        self.d.handle(self.now, &self.world, input)
    }

    fn greeter(&mut self, request: GreeterRequest) -> Vec<Effect> {
        self.on(Input::GreeterRequest(request))
    }

    fn worker(&mut self, w: u32, msg: FromWorker) -> Vec<Effect> {
        self.on(Input::Worker(WorkerId(w), msg))
    }

    fn exited(&mut self, w: u32, clean: bool) -> Vec<Effect> {
        self.on(Input::WorkerExited {
            worker: WorkerId(w),
            clean,
        })
    }

    fn session(&mut self, id: &str, request: SessionRequest) -> Vec<Effect> {
        self.on(Input::Session {
            id: id.into(),
            request,
        })
    }

    fn removed(&mut self, id: &str, seat_active: Option<&str>) -> Vec<Effect> {
        self.on(Input::SessionRemoved {
            id: id.into(),
            seat_active: seat_active.map(Into::into),
        })
    }

    fn greeting(&mut self) -> Greeting {
        match self.greeter(GreeterRequest::Hello).as_slice() {
            [Effect::Reply(GreeterReply::Greeting(g))] => g.clone(),
            other => panic!("Hello is answered with one Greeting, got {other:?}"),
        }
    }

    /// Logs `user` in through worker `w` from the greeter and its session `id` starts on `vt`.
    fn log_in(&mut self, w: u32, user: &str, uid: u32, id: &str, vt: u16) {
        assert_eq!(self.greeter(create(user)), spawn(w, Mode::Login, user));
        assert_eq!(
            self.worker(w, FromWorker::Authenticated),
            reply(GreeterReply::Success)
        );
        assert_eq!(
            self.greeter(GreeterRequest::StartSession),
            vec![Effect::ToWorker {
                worker: WorkerId(w),
                msg: ToWorker::Start { vt },
            }]
        );
        let effects = self.worker(w, started(id, uid));
        assert_eq!(
            effects[0],
            Effect::Reply(GreeterReply::Success),
            "{effects:?}"
        );
    }
}

#[test]
fn boot_with_no_session_starts_the_greeter_on_vt_1() {
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    assert_eq!(run.recover(vec![]), vec![]);
    assert_eq!(run.start(), greeter_spawn(1));
}

#[test]
fn boot_with_autologin_starts_that_user_on_vt_7_without_a_greeter() {
    let config = LoginConfig {
        autologin_user: Some("anna".into()),
        ..LoginConfig::default()
    };
    let mut run = Run::new(config.clone(), Remembered::default());
    assert_eq!(run.recover(vec![]), vec![]);
    assert_eq!(
        run.start(),
        vec![
            Effect::Spawn {
                worker: WorkerId(1),
                job: Job {
                    mode: Mode::Autologin,
                    user: "anna".into(),
                },
            },
            Effect::ToWorker {
                worker: WorkerId(1),
                msg: ToWorker::Start { vt: 7 },
            },
        ]
    );

    let mut run = Run::new(config, Remembered::default());
    let found = logind("c1", "anna", 1000, 7, "user", "reprise-login-autologin");
    assert_eq!(run.recover(vec![found]), vec![]);
    assert_eq!(run.start(), vec![], "the recovered session stays on screen");
}

#[test]
fn autologin_of_an_account_the_screen_would_not_list_shows_the_greeter() {
    let config = LoginConfig {
        autologin_user: Some("root".into()),
        ..LoginConfig::default()
    };
    let mut run = Run::new(config, Remembered::default());
    let effects = run.start();
    assert!(
        matches!(&effects[0], Effect::Log(text) if text.contains("root")),
        "{effects:?}"
    );
    assert_eq!(effects[1..], greeter_spawn(1)[..]);
}

#[test]
fn hello_greets_with_the_configured_interface_else_the_last_one() {
    let remembered = Remembered {
        last_user: None,
        last_interface: Some(Interface::Blue),
    };
    let mut run = Run::new(LoginConfig::default(), remembered.clone());
    assert_eq!(run.greeting().interface, Interface::Blue);

    let config = LoginConfig {
        interface: Some(Interface::Glass),
        ..LoginConfig::default()
    };
    let mut run = Run::new(config, remembered);
    assert_eq!(run.greeting().interface, Interface::Glass);

    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    assert_eq!(run.greeting().interface, Interface::Classic);
}

#[test]
fn the_greeting_marks_users_with_a_session_logged_on() {
    let remembered = Remembered {
        last_user: Some("bert".into()),
        last_interface: None,
    };
    let mut run = Run::new(LoginConfig::default(), remembered);
    run.recover(vec![anna_on_7()]);
    assert_eq!(
        run.greeting(),
        Greeting {
            interface: Interface::Classic,
            show_users: true,
            users: vec![
                UserEntry {
                    note: Some(Note::LoggedOn),
                    ..user("anna")
                },
                user("bert"),
            ],
            last_user: Some("bert".into()),
            keyboard: world().keyboard,
            notice: None,
        }
    );
}

#[test]
fn a_huge_user_list_still_encodes() {
    let mut run = Run::booted();
    run.world.users = (0..5000)
        .map(|n| UserEntry {
            real_name: "\u{1}".repeat(400),
            picture: Some(format!("/var/lib/AccountsService/icons/user{n}").into()),
            ..user(&format!("user{n}"))
        })
        .collect();
    let greeting = run.greeting();
    assert!(!greeting.users.is_empty(), "some users still fit");
    assert!(
        greeting.users.len() <= MAX_USERS,
        "{}",
        greeting.users.len()
    );
    let frame = encode(&GreeterReply::Greeting(greeting)).expect("the greeting fits one frame");
    assert!(frame.len() <= 4 + MAX_FRAME);
}

#[test]
fn a_login_for_a_new_user_starts_a_session_on_the_lowest_free_vt() {
    let mut run = Run::booted();
    assert_eq!(run.greeter(create("anna")), spawn(2, Mode::Login, "anna"));
    assert_eq!(
        run.worker(2, secret("Password:")),
        reply(GreeterReply::AuthMessage {
            kind: AuthKind::Secret,
            text: "Password:".into(),
        })
    );
    assert_eq!(
        run.greeter(answer("hunter2")),
        vec![Effect::ToWorker {
            worker: WorkerId(2),
            msg: ToWorker::Answer(Some(Zeroizing::new("hunter2".into()))),
        }]
    );
    assert_eq!(
        run.worker(2, FromWorker::Authenticated),
        reply(GreeterReply::Success)
    );
    assert_eq!(
        run.greeter(GreeterRequest::StartSession),
        vec![Effect::ToWorker {
            worker: WorkerId(2),
            msg: ToWorker::Start { vt: 7 },
        }]
    );
    assert_eq!(
        run.worker(2, started("c2", 1000)),
        vec![
            Effect::Reply(GreeterReply::Success),
            Effect::Remember(Remembered {
                last_user: Some("anna".into()),
                last_interface: None,
            }),
        ]
    );
    assert_eq!(
        run.exited(1, false),
        vec![],
        "the leaving greeter is not respawned"
    );

    assert_eq!(
        run.session("c2", SessionRequest::SwitchUser),
        greeter_spawn(3)
    );
    assert_eq!(run.exited(3, false), greeter_spawn(4));
    assert_eq!(
        run.exited(4, false),
        greeter_spawn(5),
        "the leaving greeter's exit counted no failure"
    );
}

#[test]
fn a_login_whose_account_has_a_session_resumes_that_one() {
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    run.recover(vec![anna_on_7()]);
    assert_eq!(
        run.session("c1", SessionRequest::SwitchUser),
        greeter_spawn(1)
    );
    run.greeter(create("Anna"));
    run.worker(2, FromWorker::Authenticated);
    run.greeter(GreeterRequest::StartSession);
    assert_eq!(
        run.worker(2, started("c2", 1000)),
        vec![
            Effect::TerminateSession("c2".into()),
            Effect::UnlockSession("c1".into()),
            Effect::Arm {
                timer: Timer::UnlockAck("c1".into()),
                after: UNLOCK_WAIT,
            },
        ],
        "the account's uid already has session c1"
    );
    assert_eq!(
        run.on(Input::Unlocked { id: "c1".into() }),
        vec![Effect::SwitchTo(7), Effect::Reply(GreeterReply::Success)]
    );
    assert_eq!(run.removed("c2", Some("c1")), vec![]);
    assert_eq!(run.exited(2, false), vec![Effect::Disallocate(8)]);
    assert_eq!(run.exited(1, false), vec![], "the greeter left");
    assert_eq!(
        run.session("c1", SessionRequest::SwitchUser),
        greeter_spawn(3),
        "c1 is still in the table"
    );
}

#[test]
fn a_login_for_a_user_with_a_session_unlocks_and_switches_to_it() {
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    run.recover(vec![anna_on_7()]);
    assert_eq!(
        run.session("c1", SessionRequest::SwitchUser),
        greeter_spawn(1)
    );

    assert_eq!(run.greeter(create("anna")), spawn(2, Mode::Reauth, "anna"));
    run.worker(2, secret("Password:"));
    run.greeter(answer("hunter2"));
    assert_eq!(
        run.worker(2, FromWorker::Authenticated),
        reply(GreeterReply::Success)
    );
    assert_eq!(run.exited(2, true), vec![]);
    let unlock = vec![
        Effect::UnlockSession("c1".into()),
        Effect::Arm {
            timer: Timer::UnlockAck("c1".into()),
            after: UNLOCK_WAIT,
        },
    ];
    assert_eq!(run.greeter(GreeterRequest::StartSession), unlock);
    assert_eq!(
        run.on(Input::Unlocked { id: "c1".into() }),
        vec![Effect::SwitchTo(7), Effect::Reply(GreeterReply::Success)]
    );
    assert_eq!(run.on(Input::Timer(Timer::UnlockAck("c1".into()))), vec![]);
    assert_eq!(
        run.exited(1, false),
        vec![],
        "the greeter left after the resume"
    );

    assert_eq!(
        run.session("c1", SessionRequest::SwitchUser),
        greeter_spawn(3)
    );
    assert_eq!(run.greeter(create("anna")), spawn(4, Mode::Reauth, "anna"));
    run.worker(4, FromWorker::Authenticated);
    assert_eq!(run.greeter(GreeterRequest::StartSession), unlock);
    let effects = run.on(Input::Timer(Timer::UnlockAck("c1".into())));
    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::Log(_),
                Effect::SwitchTo(7),
                Effect::Reply(GreeterReply::Success)
            ]
        ),
        "{effects:?}"
    );
    assert_eq!(run.on(Input::Unlocked { id: "c1".into() }), vec![]);
}

#[test]
fn a_wrong_password_reports_an_auth_error_and_the_greeter_can_retry() {
    let mut run = Run::booted();
    run.greeter(create("anna"));
    run.worker(2, secret("Password:"));
    run.greeter(answer("wrong"));
    assert_eq!(
        run.worker(
            2,
            FromWorker::Failed {
                kind: ErrorKind::Auth,
                text: "Authentication failure".into(),
            }
        ),
        reply(error(ErrorKind::Auth, "Authentication failure"))
    );
    assert_eq!(run.exited(2, false), vec![]);
    assert_eq!(run.greeter(create("anna")), spawn(3, Mode::Login, "anna"));
}

#[test]
fn every_prompt_gets_one_answer() {
    let mut run = Run::booted();
    run.greeter(create("anna"));
    for kind in [AuthKind::Info, AuthKind::Error] {
        assert_eq!(
            run.worker(
                2,
                FromWorker::Prompt {
                    kind,
                    text: "Your password expires soon".into(),
                }
            ),
            reply(GreeterReply::AuthMessage {
                kind,
                text: "Your password expires soon".into(),
            })
        );
        assert_eq!(
            run.greeter(GreeterRequest::PostAuthResponse { response: None }),
            vec![Effect::ToWorker {
                worker: WorkerId(2),
                msg: ToWorker::Answer(None),
            }]
        );
    }
    assert!(
        is_refused(&run.greeter(GreeterRequest::PostAuthResponse { response: None })),
        "no second answer to one prompt"
    );
    assert_eq!(
        run.worker(1, secret("Password:")),
        vec![Effect::ToWorker {
            worker: GREETER,
            msg: ToWorker::Answer(None),
        }],
        "the greeter's worker has nobody to ask"
    );
}

#[test]
fn greeter_gone_mid_conversation_kills_the_worker() {
    let mut run = Run::booted();
    run.greeter(create("anna"));
    run.worker(2, secret("Password:"));
    assert_eq!(
        run.on(Input::GreeterGone),
        vec![Effect::Kill(WorkerId(2)), Effect::Kill(GREETER)]
    );
    assert_eq!(run.worker(2, secret("Password:")), vec![]);
    assert_eq!(run.exited(2, false), vec![]);
    assert_eq!(run.exited(1, false), greeter_spawn(3));
    assert_eq!(run.greeter(create("anna")), spawn(4, Mode::Login, "anna"));
    run.on(Input::GreeterGone);
    assert_eq!(run.exited(3, false), greeter_spawn(5));
    assert_eq!(
        run.exited(5, false)[0],
        Effect::SwitchTo(2),
        "the first death counted"
    );
}

#[test]
fn three_greeter_crashes_in_a_row_switch_to_the_text_login_vt() {
    let mut run = Run::booted();
    assert_eq!(run.exited(1, false), greeter_spawn(2));
    assert_eq!(run.exited(2, false), greeter_spawn(3));
    let effects = run.exited(3, false);
    assert!(
        matches!(effects.as_slice(), [Effect::SwitchTo(2), Effect::Log(_)]),
        "{effects:?}"
    );
}

#[test]
fn a_session_ending_while_active_brings_the_greeter_back() {
    let mut run = Run::booted();
    run.log_in(2, "anna", 1000, "c2", 7);
    run.exited(1, true);
    run.now += EARLY_END * 2;
    assert_eq!(run.removed("c2", None), greeter_spawn(3));
    assert_eq!(run.exited(2, true), vec![Effect::Disallocate(7)]);

    run.log_in(4, "anna", 1000, "c4", 7);
    run.exited(3, true);
    assert_eq!(
        run.session("c4", SessionRequest::SwitchUser),
        greeter_spawn(5)
    );
    run.log_in(6, "bert", 1001, "c6", 8);
    run.exited(5, true);
    run.now += EARLY_END * 2;
    assert_eq!(run.removed("c4", Some("c6")), vec![]);
    assert_eq!(run.exited(4, true), vec![Effect::Disallocate(7)]);
}

#[test]
fn a_session_ending_soon_after_start_shows_a_notice() {
    let mut run = Run::booted();
    run.log_in(2, "anna", 1000, "c2", 7);
    run.exited(1, true);
    run.now += EARLY_END / 2;
    assert_eq!(run.removed("c2", None), greeter_spawn(3));
    assert_eq!(run.greeting().notice, Some(Notice::SessionEndedEarly));
    assert_eq!(run.greeting().notice, None, "the notice shows once");
}

#[test]
fn a_session_that_fails_to_start_returns_to_the_login_screen() {
    let mut run = Run::booted();
    run.greeter(create("anna"));
    run.worker(2, FromWorker::Authenticated);
    run.greeter(GreeterRequest::StartSession);
    assert_eq!(
        run.worker(
            2,
            FromWorker::Failed {
                kind: ErrorKind::Failed,
                text: "no session id".into(),
            }
        ),
        vec![
            Effect::Reply(error(ErrorKind::Failed, "no session id")),
            Effect::SwitchTo(1),
        ]
    );
    assert_eq!(run.greeter(create("anna")), spawn(3, Mode::Login, "anna"));
}

#[test]
fn switch_user_from_a_session_starts_the_greeter() {
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    run.recover(vec![anna_on_7()]);
    assert_eq!(run.start(), vec![]);
    assert_eq!(
        run.session("c1", SessionRequest::SwitchUser),
        greeter_spawn(1)
    );
    assert_eq!(
        run.session("c1", SessionRequest::SwitchUser),
        vec![Effect::SwitchTo(1)]
    );
}

#[test]
fn power_with_another_session_running_is_refused() {
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    run.recover(vec![anna_on_7()]);
    let effects = run.greeter(GreeterRequest::Power {
        action: PowerAction::PowerOff,
    });
    assert!(is_refused(&effects), "{effects:?}");
}

#[test]
fn power_with_no_session_powers_off() {
    let mut run = Run::booted();
    assert_eq!(
        run.greeter(GreeterRequest::Power {
            action: PowerAction::PowerOff,
        }),
        vec![Effect::Power(PowerAction::PowerOff)]
    );
    assert_eq!(
        run.on(Input::PowerDone(Err("masked".into()))),
        reply(error(ErrorKind::Failed, "masked"))
    );
}

#[test]
fn hello_from_a_session_remembers_its_interface() {
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    run.recover(vec![anna_on_7()]);
    let hello = SessionRequest::Hello {
        interface: Interface::Glass,
    };
    assert_eq!(
        run.session("c1", hello.clone()),
        vec![Effect::Remember(Remembered {
            last_user: None,
            last_interface: Some(Interface::Glass),
        })]
    );
    assert_eq!(run.session("c1", hello), vec![], "nothing changed");
    assert_eq!(run.greeting().interface, Interface::Glass);
}

#[test]
fn one_conversation_at_a_time() {
    let mut run = Run::booted();
    assert_eq!(run.greeter(create("anna")), spawn(2, Mode::Login, "anna"));
    let effects = run.greeter(create("bert"));
    assert!(is_refused(&effects), "{effects:?}");
    assert_eq!(
        run.greeter(GreeterRequest::CancelSession),
        vec![
            Effect::Kill(WorkerId(2)),
            Effect::Reply(GreeterReply::Success)
        ]
    );
    assert_eq!(run.greeter(create("bert")), spawn(3, Mode::Login, "bert"));
}

#[test]
fn a_cancelled_workers_messages_are_ignored() {
    let mut run = Run::booted();
    run.greeter(create("anna"));
    run.greeter(GreeterRequest::CancelSession);
    assert_eq!(run.worker(2, secret("Password:")), vec![]);
    assert_eq!(run.worker(2, FromWorker::Authenticated), vec![]);
    assert_eq!(run.worker(2, started("c2", 1000)), vec![]);
    assert_eq!(run.exited(2, false), vec![]);
}

#[test]
fn recovery_rebuilds_the_table_and_terminates_an_orphaned_greeter() {
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    run.world.users.push(user("carl"));
    let found = vec![
        anna_on_7(),
        logind("c3", "bert", 1001, 8, "user", "reprise-login-autologin"),
        logind("c4", "carl", 1002, 2, "user", "login"),
        logind(
            "c5",
            GREETER_USER,
            960,
            1,
            "greeter",
            "reprise-login-greeter",
        ),
        logind("c6", "sddm", 961, 1, "greeter", "sddm-greeter"),
        LogindSession {
            seat: "seat1".into(),
            ..logind("c7", "carl", 1002, 9, "user", "reprise-login")
        },
    ];
    assert_eq!(
        run.recover(found),
        vec![Effect::TerminateSession("c5".into())]
    );
    assert_eq!(
        run.start(),
        vec![],
        "the greeter waits for the old one to go"
    );
    let notes: Vec<_> = run.greeting().users.iter().map(|u| u.note).collect();
    assert_eq!(notes, [Some(Note::LoggedOn), Some(Note::LoggedOn), None]);
    assert_eq!(run.removed("c5", None), greeter_spawn(1));
    assert_eq!(run.greeter(create("anna")), spawn(2, Mode::Reauth, "anna"));
    assert_eq!(
        run.removed("c3", Some("c5")),
        vec![Effect::Disallocate(8)],
        "no worker of this daemon holds a recovered session's VT"
    );
}

#[test]
fn a_session_request_from_an_unknown_session_is_ignored() {
    let mut run = Run::booted();
    assert_eq!(run.session("c9", SessionRequest::SwitchUser), vec![]);
    let hello = SessionRequest::Hello {
        interface: Interface::Blue,
    };
    assert_eq!(run.session("c9", hello), vec![]);
}
