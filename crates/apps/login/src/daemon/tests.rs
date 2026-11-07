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
        user_sessions: 0,
        login_sessions: Vec::new(),
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
    let mut effects = spawn(w, Mode::Greeter, GREETER_USER);
    effects.extend(start_on(w, 1));
    effects
}

/// Worker `w`'s spawn; a login's comes with its `AUTH_WAIT`.
fn spawn(w: u32, mode: Mode, user: &str) -> Vec<Effect> {
    let mut effects = vec![Effect::Spawn {
        worker: WorkerId(w),
        job: Job {
            mode,
            user: user.into(),
            autologin_user: None,
            greeter_socket: false,
        },
    }];
    if let Mode::Login | Mode::Reauth = mode {
        effects.push(Effect::Arm {
            timer: Timer::Auth(WorkerId(w)),
            after: AUTH_WAIT,
        });
    }
    effects
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

fn authed(user: &str, uid: u32) -> FromWorker {
    FromWorker::Authenticated {
        user: user.into(),
        uid,
    }
}

fn start_on(w: u32, vt: u16) -> Vec<Effect> {
    vec![
        Effect::ToWorker {
            worker: WorkerId(w),
            msg: ToWorker::Start { vt },
        },
        Effect::Arm {
            timer: Timer::Start(WorkerId(w)),
            after: START_WAIT,
        },
    ]
}

fn unlock(id: &str, generation: u32) -> Vec<Effect> {
    vec![
        Effect::UnlockSession(id.into()),
        Effect::Arm {
            timer: Timer::UnlockAck(generation),
            after: UNLOCK_WAIT,
        },
    ]
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
        Run::up(LoginConfig::default(), Remembered::default())
    }

    /// Like `booted`, with `config` and `remembered`.
    fn up(config: LoginConfig, remembered: Remembered) -> Run {
        let mut run = Run::new(config, remembered);
        assert_eq!(run.recover(vec![]), vec![]);
        assert_eq!(run.start(), greeter_spawn(1));
        run
    }

    /// Recovers anna's session on VT 7 and starts; her Switch User brings up the greeter as
    /// worker 1.
    fn anna_switched(remembered: Remembered) -> Run {
        let mut run = Run::new(LoginConfig::default(), remembered);
        run.recover(vec![anna_on_7()]);
        assert_eq!(run.start(), vec![]);
        assert_eq!(
            run.session("c1", SessionRequest::SwitchUser),
            greeter_spawn(1)
        );
        run
    }

    /// Recovers `found` with c1 on screen.
    fn recover(&mut self, found: Vec<LogindSession>) -> Vec<Effect> {
        self.d.recover(self.now, found, Some("c1"))
    }

    fn start(&mut self) -> Vec<Effect> {
        self.d.start(self.now, &self.world)
    }

    fn on(&mut self, input: Input) -> Vec<Effect> {
        self.d.handle(self.now, &self.world, input)
    }

    /// A request from the running greeter, or from worker 1 with none running.
    fn greeter(&mut self, request: GreeterRequest) -> Vec<Effect> {
        self.on(Input::GreeterRequest(self.greeter_worker(), request))
    }

    fn greeter_gone(&mut self) -> Vec<Effect> {
        self.on(Input::GreeterGone(self.greeter_worker()))
    }

    fn greeter_worker(&self) -> WorkerId {
        self.d.greeter.as_ref().map_or(GREETER, |g| g.worker)
    }

    fn worker(&mut self, w: u32, msg: FromWorker) -> Vec<Effect> {
        self.on(Input::Worker(WorkerId(w), msg))
    }

    fn exited(&mut self, w: u32) -> Vec<Effect> {
        self.on(Input::WorkerExited(WorkerId(w)))
    }

    /// A request from session `id` while it is on screen.
    fn session(&mut self, id: &str, request: SessionRequest) -> Vec<Effect> {
        self.on(Input::Session {
            id: id.into(),
            request,
            seat_active: Some(id.into()),
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
            self.worker(w, authed(user, uid)),
            reply(GreeterReply::Success)
        );
        assert_eq!(self.greeter(GreeterRequest::StartSession), start_on(w, vt));
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
fn only_a_greeter_job_carries_greeter_socket_and_only_when_set() {
    let config = LoginConfig {
        greeter_socket: true,
        ..LoginConfig::default()
    };
    let mut run = Run::new(config, Remembered::default());
    match run.start().as_slice() {
        [Effect::Spawn { job, .. }, ..] => assert!(job.greeter_socket, "{job:?}"),
        other => panic!("{other:?}"),
    }
    match run.greeter(create("anna")).as_slice() {
        [Effect::Spawn { job, .. }, ..] => assert!(!job.greeter_socket, "{job:?}"),
        other => panic!("{other:?}"),
    }
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    match run.start().as_slice() {
        [Effect::Spawn { job, .. }, ..] => assert!(!job.greeter_socket, "{job:?}"),
        other => panic!("{other:?}"),
    }
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
                    autologin_user: Some("anna".into()),
                    greeter_socket: false,
                },
            },
            Effect::ToWorker {
                worker: WorkerId(1),
                msg: ToWorker::Start { vt: 7 },
            },
            Effect::Arm {
                timer: Timer::Start(WorkerId(1)),
                after: START_WAIT,
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
        matches!(&effects[0], Effect::Log(log::Level::Info, text) if text.contains("root")),
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
    let mut run = Run::up(LoginConfig::default(), remembered.clone());
    assert_eq!(run.greeting().interface, Interface::Blue);

    let config = LoginConfig {
        interface: Some(Interface::Glass),
        ..LoginConfig::default()
    };
    let mut run = Run::up(config, remembered);
    assert_eq!(run.greeting().interface, Interface::Glass);

    let mut run = Run::booted();
    assert_eq!(run.greeting().interface, Interface::Classic);
}

#[test]
fn the_greeting_marks_users_with_a_session_logged_on() {
    let remembered = Remembered {
        last_user: Some("bert".into()),
        last_interface: None,
    };
    let mut run = Run::anna_switched(remembered);
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
        run.worker(2, authed("anna", 1000)),
        reply(GreeterReply::Success)
    );
    assert_eq!(run.greeter(GreeterRequest::StartSession), start_on(2, 7));
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
        run.exited(1),
        vec![],
        "the leaving greeter is not respawned"
    );

    assert_eq!(
        run.session("c2", SessionRequest::SwitchUser),
        greeter_spawn(3)
    );
    assert_eq!(run.exited(3), greeter_spawn(4));
    assert_eq!(
        run.exited(4),
        greeter_spawn(5),
        "the leaving greeter's exit counted no failure"
    );
}

#[test]
fn a_login_whose_account_has_a_session_resumes_it_without_starting_one() {
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    run.recover(vec![anna_on_7()]);
    assert_eq!(
        run.session("c1", SessionRequest::SwitchUser),
        greeter_spawn(1)
    );
    assert_eq!(run.greeter(create("Anna")), spawn(2, Mode::Login, "Anna"));
    assert_eq!(
        run.worker(2, authed("anna", 1000)),
        vec![
            Effect::Kill(WorkerId(2)),
            Effect::Reply(GreeterReply::Success)
        ],
        "uid 1000 already has session c1"
    );
    assert_eq!(run.greeter(GreeterRequest::StartSession), unlock("c1", 1));
    assert_eq!(run.exited(2), vec![]);
    assert_eq!(
        run.on(Input::Unlocked { id: "c1".into() }),
        vec![Effect::SwitchTo(7), Effect::Reply(GreeterReply::Success)]
    );
    assert_eq!(run.exited(1), vec![], "the greeter left");
}

#[test]
fn a_started_session_whose_uid_has_one_already_is_ended_and_that_one_resumed() {
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    run.recover(vec![anna_on_7()]);
    run.session("c1", SessionRequest::SwitchUser);
    run.greeter(create("bert"));
    run.worker(2, authed("bert", 1001));
    assert_eq!(run.greeter(GreeterRequest::StartSession), start_on(2, 8));
    let mut ends_c2 = vec![Effect::TerminateSession("c2".into())];
    ends_c2.extend(unlock("c1", 1));
    assert_eq!(
        run.worker(2, started("c2", 1000)),
        ends_c2,
        "uid 1000 already has session c1"
    );
    assert_eq!(
        run.on(Input::Unlocked { id: "c1".into() }),
        vec![Effect::SwitchTo(7), Effect::Reply(GreeterReply::Success)]
    );
    assert_eq!(run.removed("c2", Some("c1")), vec![]);
    assert_eq!(run.exited(2), vec![Effect::Disallocate(8)]);
    assert_eq!(run.exited(1), vec![], "the greeter left");
    assert_eq!(
        run.session("c1", SessionRequest::SwitchUser),
        greeter_spawn(3),
        "c1 is still in the table"
    );
}

#[test]
fn the_table_keeps_the_name_pam_resolved() {
    let mut run = Run::booted();
    run.greeter(create("ANNA"));
    assert_eq!(
        run.worker(2, authed("anna", 1000)),
        reply(GreeterReply::Success)
    );
    run.greeter(GreeterRequest::StartSession);
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
    let notes: Vec<_> = run.greeting().users.iter().map(|u| u.note).collect();
    assert_eq!(notes, [Some(Note::LoggedOn), None]);
    assert_eq!(run.greeter(create("anna")), spawn(3, Mode::Reauth, "anna"));
}

#[test]
fn a_reauth_that_resolves_to_an_account_without_a_session_unlocks_nothing() {
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    run.recover(vec![anna_on_7()]);
    run.session("c1", SessionRequest::SwitchUser);
    assert_eq!(run.greeter(create("anna")), spawn(2, Mode::Reauth, "anna"));
    assert_eq!(
        run.worker(2, authed("bert", 1001)),
        reply(error(ErrorKind::Failed, NO_SESSION))
    );
    let effects = run.greeter(GreeterRequest::StartSession);
    assert!(is_refused(&effects), "{effects:?}");
    assert_eq!(run.exited(2), vec![]);
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
        run.worker(2, authed("anna", 1000)),
        reply(GreeterReply::Success)
    );
    assert_eq!(run.exited(2), vec![]);
    assert_eq!(run.greeter(GreeterRequest::StartSession), unlock("c1", 1));
    assert_eq!(
        run.on(Input::Unlocked { id: "c1".into() }),
        vec![Effect::SwitchTo(7), Effect::Reply(GreeterReply::Success)]
    );
    assert_eq!(run.on(Input::Timer(Timer::UnlockAck(1))), vec![]);
    assert_eq!(run.exited(1), vec![], "the greeter left after the resume");

    assert_eq!(
        run.session("c1", SessionRequest::SwitchUser),
        greeter_spawn(3)
    );
    assert_eq!(run.greeter(create("anna")), spawn(4, Mode::Reauth, "anna"));
    run.worker(4, authed("anna", 1000));
    assert_eq!(run.greeter(GreeterRequest::StartSession), unlock("c1", 2));
    assert_eq!(
        run.on(Input::Timer(Timer::UnlockAck(1))),
        vec![],
        "the first resume's timer is stale"
    );
    let effects = run.on(Input::Timer(Timer::UnlockAck(2)));
    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::Log(..),
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
    assert_eq!(run.exited(2), vec![]);
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
        run.greeter_gone(),
        vec![Effect::Kill(WorkerId(2)), Effect::Kill(GREETER)]
    );
    assert_eq!(run.worker(2, secret("Password:")), vec![]);
    assert_eq!(run.exited(2), vec![]);
    assert_eq!(run.exited(1), greeter_spawn(3));
    assert_eq!(run.greeter(create("anna")), spawn(4, Mode::Login, "anna"));
    run.greeter_gone();
    assert_eq!(run.exited(3), greeter_spawn(5));
    assert_eq!(
        run.exited(5)[0],
        Effect::SwitchTo(2),
        "the first death counted"
    );
}

#[test]
fn three_greeter_exits_in_a_row_switch_to_the_text_login_vt() {
    let mut run = Run::booted();
    assert_eq!(run.exited(1), greeter_spawn(2));
    assert_eq!(run.exited(2), greeter_spawn(3));
    let effects = run.exited(3);
    assert!(
        matches!(effects.as_slice(), [Effect::SwitchTo(2), Effect::Log(..)]),
        "{effects:?}"
    );
}

#[test]
fn a_session_ending_while_active_brings_the_greeter_back() {
    let mut run = Run::booted();
    run.log_in(2, "anna", 1000, "c2", 7);
    run.exited(1);
    run.now += EARLY_END * 2;
    assert_eq!(run.removed("c2", None), greeter_spawn(3));
    assert_eq!(run.exited(2), vec![Effect::Disallocate(7)]);

    run.log_in(4, "anna", 1000, "c4", 7);
    run.exited(3);
    assert_eq!(
        run.session("c4", SessionRequest::SwitchUser),
        greeter_spawn(5)
    );
    run.log_in(6, "bert", 1001, "c6", 8);
    run.exited(5);
    run.now += EARLY_END * 2;
    assert_eq!(run.removed("c4", Some("c6")), vec![]);
    assert_eq!(run.exited(4), vec![Effect::Disallocate(7)]);
}

#[test]
fn a_session_ending_soon_after_start_shows_a_notice() {
    let mut run = Run::booted();
    run.log_in(2, "anna", 1000, "c2", 7);
    run.exited(1);
    run.now += EARLY_END / 2;
    assert_eq!(run.removed("c2", None), greeter_spawn(3));
    assert_eq!(run.greeting().notice, Some(Notice::SessionEndedEarly));
    assert_eq!(run.greeting().notice, None, "the notice shows once");
}

#[test]
fn a_session_that_fails_to_start_returns_to_the_login_screen() {
    let mut run = Run::booted();
    run.greeter(create("anna"));
    run.worker(2, authed("anna", 1000));
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
fn switch_user_from_a_session_off_screen_is_ignored() {
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    run.recover(vec![
        anna_on_7(),
        logind("c2", "bert", 1001, 8, "user", "reprise-login"),
    ]);
    assert_eq!(run.start(), vec![]);
    for seat_active in [Some("c2".to_string()), None] {
        let effects = run.on(Input::Session {
            id: "c1".into(),
            request: SessionRequest::SwitchUser,
            seat_active,
        });
        assert_eq!(effects, vec![]);
    }
    assert_eq!(
        run.session("c2", SessionRequest::SwitchUser),
        greeter_spawn(1)
    );
}

#[test]
fn a_removal_that_still_names_the_removed_session_active_brings_the_greeter() {
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    run.recover(vec![anna_on_7()]);
    assert_eq!(run.start(), vec![]);
    let effects = run.removed("c1", Some("c1"));
    assert_eq!(effects[0], Effect::Disallocate(7));
    assert_eq!(effects[1..], greeter_spawn(1)[..]);
}

#[test]
fn power_with_another_session_running_is_refused() {
    let mut run = Run::anna_switched(Remembered::default());
    let effects = run.greeter(GreeterRequest::Power {
        action: PowerAction::PowerOff,
    });
    assert!(is_refused(&effects), "{effects:?}");
}

#[test]
fn power_with_no_session_powers_off_and_a_failure_is_logged() {
    let mut run = Run::booted();
    assert_eq!(
        run.greeter(GreeterRequest::Power {
            action: PowerAction::PowerOff,
        }),
        vec![Effect::Power(PowerAction::PowerOff)]
    );
    assert_eq!(
        run.on(Input::PowerDone(Err("masked".into()))),
        vec![
            Effect::Log(Level::Warn, "turning off or restarting: masked".into()),
            Effect::Reply(error(ErrorKind::Failed, POWER_FAILED)),
        ]
    );
}

#[test]
fn hello_from_a_session_remembers_its_interface() {
    let mut run = Run::anna_switched(Remembered::default());
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
    assert_eq!(run.worker(2, authed("anna", 1000)), vec![]);
    assert_eq!(run.worker(2, started("c2", 1000)), vec![]);
    assert_eq!(run.exited(2), vec![]);
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
        vec![
            Effect::LockSession("c3".into()),
            Effect::TerminateSession("c5".into())
        ],
        "bert's session is off screen; carl's text login is not reprise-login's"
    );
    assert_eq!(
        run.start(),
        vec![],
        "the greeter waits for the old one to go"
    );
    assert_eq!(run.removed("c5", None), greeter_spawn(1));
    let notes: Vec<_> = run.greeting().users.iter().map(|u| u.note).collect();
    assert_eq!(notes, [Some(Note::LoggedOn), Some(Note::LoggedOn), None]);
    assert_eq!(run.greeter(create("anna")), spawn(2, Mode::Reauth, "anna"));
    assert_eq!(
        run.removed("c3", Some("c5")),
        vec![Effect::Disallocate(8)],
        "no worker of this daemon holds a recovered session's VT"
    );

    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    let orphan = logind(
        "c5",
        GREETER_USER,
        960,
        1,
        "greeter",
        "reprise-login-greeter",
    );
    run.recover(vec![anna_on_7(), orphan]);
    assert_eq!(run.start(), vec![]);
    assert_eq!(
        run.removed("c5", Some("c1")),
        vec![],
        "anna's session holds the seat"
    );
    assert_eq!(
        run.session("c1", SessionRequest::SwitchUser),
        greeter_spawn(1)
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

#[test]
fn a_resume_whose_session_ends_mid_conversation_kills_its_worker() {
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    run.recover(vec![anna_on_7()]);
    run.session("c1", SessionRequest::SwitchUser);
    assert_eq!(run.greeter(create("anna")), spawn(2, Mode::Reauth, "anna"));
    run.worker(2, secret("Password:"));
    assert_eq!(
        run.removed("c1", None),
        vec![
            Effect::Kill(WorkerId(2)),
            Effect::Reply(error(ErrorKind::Failed, SESSION_ENDED)),
            Effect::Disallocate(7),
            Effect::SwitchTo(1),
        ]
    );
    assert_eq!(run.greeter(create("anna")), spawn(3, Mode::Login, "anna"));
}

#[test]
fn start_session_with_no_free_vt_fails_and_kills_the_worker() {
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    run.recover(
        (16..=63)
            .map(|vt| {
                let (id, name) = (format!("c{vt}"), format!("u{vt}"));
                logind(
                    &id,
                    &name,
                    2000 + u32::from(vt),
                    vt,
                    "user",
                    "reprise-login",
                )
            })
            .collect(),
    );
    run.world.kernel_open = 0xff80;
    run.session("c16", SessionRequest::SwitchUser);
    run.greeter(create("anna"));
    run.worker(2, authed("anna", 1000));
    assert_eq!(
        run.greeter(GreeterRequest::StartSession),
        vec![
            Effect::Kill(WorkerId(2)),
            Effect::Reply(error(ErrorKind::Failed, NO_VT)),
        ]
    );
    assert_eq!(run.greeter(create("anna")), spawn(3, Mode::Login, "anna"));
}

#[test]
fn a_session_removed_before_its_started_is_handled_as_ended() {
    let mut run = Run::booted();
    run.greeter(create("anna"));
    run.worker(2, authed("anna", 1000));
    assert_eq!(run.greeter(GreeterRequest::StartSession), start_on(2, 7));
    assert_eq!(run.removed("c2", None), vec![]);
    assert_eq!(
        run.worker(2, started("c2", 1000)),
        vec![
            Effect::Reply(error(ErrorKind::Failed, SESSION_ENDED)),
            Effect::SwitchTo(1),
        ]
    );
    let notes: Vec<_> = run.greeting().users.iter().map(|u| u.note).collect();
    assert_eq!(notes, [None, None], "c2 never entered the table");
    assert_eq!(run.greeter(create("anna")), spawn(3, Mode::Login, "anna"));
    assert_eq!(run.exited(2), vec![Effect::Disallocate(7)]);

    let config = LoginConfig {
        autologin_user: Some("anna".into()),
        ..LoginConfig::default()
    };
    let mut run = Run::new(config, Remembered::default());
    run.start();
    assert_eq!(run.removed("c1", None), vec![]);
    assert_eq!(run.worker(1, started("c1", 1000)), greeter_spawn(2));
    assert_eq!(run.greeting().notice, Some(Notice::SessionEndedEarly));
}

#[test]
fn a_session_removed_after_its_worker_exited_frees_its_vt_once() {
    let mut run = Run::booted();
    run.log_in(2, "anna", 1000, "c2", 7);
    run.exited(1);
    assert_eq!(run.exited(2), vec![Effect::Disallocate(7)]);
    run.now += EARLY_END * 2;
    assert_eq!(run.removed("c2", None), greeter_spawn(3));
}

#[test]
fn a_leaving_greeter_that_lingers_is_killed_when_the_login_screen_is_wanted() {
    let mut run = Run::booted();
    run.log_in(2, "anna", 1000, "c2", 7);
    assert_eq!(
        run.session("c2", SessionRequest::SwitchUser),
        vec![Effect::Kill(GREETER)]
    );
    assert_eq!(run.session("c2", SessionRequest::SwitchUser), vec![]);
    assert_eq!(run.exited(1), greeter_spawn(3));
}

#[test]
fn a_start_finishing_after_its_greeter_died_tells_the_next_greeter_nothing() {
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    run.recover(vec![anna_on_7()]);
    run.session("c1", SessionRequest::SwitchUser);
    run.greeter(create("bert"));
    run.worker(2, authed("bert", 1001));
    assert_eq!(run.greeter(GreeterRequest::StartSession), start_on(2, 8));
    assert_eq!(run.greeter_gone(), vec![Effect::Kill(GREETER)]);
    assert_eq!(
        run.exited(1),
        vec![],
        "no new greeter while bert's session starts"
    );
    assert_eq!(
        run.session("c1", SessionRequest::SwitchUser),
        greeter_spawn(3)
    );
    assert_eq!(
        run.worker(2, started("c2", 1001)),
        vec![Effect::Remember(Remembered {
            last_user: Some("bert".into()),
            last_interface: None,
        })],
        "greeter 3 asked for no session"
    );
    assert_eq!(run.exited(3), greeter_spawn(4), "greeter 3 was not leaving");
}

#[test]
fn a_started_before_start_ends_that_session() {
    let mut run = Run::booted();
    run.greeter(create("anna"));
    run.worker(2, authed("anna", 1000));
    let effects = run.worker(2, started("c2", 1000));
    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::Log(..),
                Effect::TerminateSession(id),
                Effect::Kill(WorkerId(2)),
                Effect::Reply(GreeterReply::Error {
                    kind: ErrorKind::Failed,
                    ..
                }),
            ] if id == "c2"
        ),
        "{effects:?}"
    );
    assert_eq!(run.exited(2), vec![]);
    assert_eq!(run.greeter(create("anna")), spawn(3, Mode::Login, "anna"));
}

#[test]
fn cancel_is_refused_once_the_session_starts_or_unlocks() {
    let mut run = Run::booted();
    run.greeter(create("anna"));
    run.worker(2, authed("anna", 1000));
    run.greeter(GreeterRequest::StartSession);
    let effects = run.greeter(GreeterRequest::CancelSession);
    assert!(is_refused(&effects), "{effects:?}");

    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    run.recover(vec![anna_on_7()]);
    run.session("c1", SessionRequest::SwitchUser);
    run.greeter(create("anna"));
    run.worker(2, authed("anna", 1000));
    run.greeter(GreeterRequest::StartSession);
    let effects = run.greeter(GreeterRequest::CancelSession);
    assert!(is_refused(&effects), "{effects:?}");
}

#[test]
fn a_start_that_never_reports_started_times_out() {
    let mut run = Run::booted();
    run.greeter(create("anna"));
    run.worker(2, authed("anna", 1000));
    run.greeter(GreeterRequest::StartSession);
    let effects = run.on(Input::Timer(Timer::Start(WorkerId(2))));
    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::Kill(WorkerId(2)),
                Effect::Reply(GreeterReply::Error {
                    kind: ErrorKind::Failed,
                    ..
                }),
                Effect::SwitchTo(1),
            ]
        ),
        "{effects:?}"
    );
    assert_eq!(run.greeter(create("bert")), spawn(3, Mode::Login, "bert"));
    assert_eq!(run.on(Input::Timer(Timer::Start(WorkerId(2)))), vec![]);
    assert_eq!(run.exited(2), vec![Effect::Disallocate(7)]);

    let mut run = Run::booted();
    run.log_in(2, "anna", 1000, "c2", 7);
    assert_eq!(
        run.on(Input::Timer(Timer::Start(WorkerId(2)))),
        vec![],
        "c2 started in time"
    );
}

#[test]
fn an_autologin_that_never_starts_times_out_to_the_login_screen() {
    let config = LoginConfig {
        autologin_user: Some("anna".into()),
        ..LoginConfig::default()
    };
    let mut run = Run::new(config, Remembered::default());
    run.start();
    let effects = run.on(Input::Timer(Timer::Start(WorkerId(1))));
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::Kill(WorkerId(1)), Effect::Log(..), ..]
        ),
        "{effects:?}"
    );
    assert_eq!(
        effects[2..],
        greeter_spawn(2)[..],
        "the login screen comes up without waiting for the worker"
    );
    assert_eq!(run.exited(1), vec![Effect::Disallocate(7)]);
    assert_eq!(run.exited(2), greeter_spawn(3));
}

#[test]
fn power_with_a_text_or_ssh_login_running_is_refused() {
    let mut run = Run::booted();
    run.world.user_sessions = 1;
    let effects = run.greeter(GreeterRequest::Power {
        action: PowerAction::Reboot,
    });
    assert!(is_refused(&effects), "{effects:?}");
}

#[test]
fn an_orphan_removed_while_an_unknown_session_holds_the_seat_still_brings_the_greeter() {
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    let orphan = logind(
        "c5",
        GREETER_USER,
        960,
        1,
        "greeter",
        "reprise-login-greeter",
    );
    run.recover(vec![orphan]);
    assert_eq!(run.start(), vec![]);
    assert_eq!(
        run.removed("c5", Some("tty2-session")),
        greeter_spawn(1),
        "a text login is not a session of this daemon"
    );
}

#[test]
fn a_restart_over_the_login_screen_still_logs_the_autologin_user_in() {
    let config = LoginConfig {
        autologin_user: Some("anna".into()),
        ..LoginConfig::default()
    };
    let mut run = Run::new(config, Remembered::default());
    let orphan = logind(
        "c5",
        GREETER_USER,
        960,
        1,
        "greeter",
        "reprise-login-greeter",
    );
    run.recover(vec![orphan]);
    assert_eq!(run.start(), vec![], "the old login screen goes first");
    let effects = run.removed("c5", None);
    assert!(
        matches!(
            effects.first(),
            Some(Effect::Spawn {
                job: Job {
                    mode: Mode::Autologin,
                    ..
                },
                ..
            })
        ),
        "{effects:?}"
    );
}

#[test]
fn a_restart_with_sessions_and_no_session_on_screen_brings_the_login_screen() {
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    assert_eq!(
        run.d.recover(run.now, vec![anna_on_7()], None),
        vec![Effect::LockSession("c1".into())]
    );
    assert_eq!(run.start(), greeter_spawn(1));
}

#[test]
fn the_greeter_waits_for_an_orphan_whatever_asks_for_it() {
    let orphan = || {
        logind(
            "c5",
            GREETER_USER,
            960,
            1,
            "greeter",
            "reprise-login-greeter",
        )
    };
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    run.recover(vec![anna_on_7(), orphan()]);
    assert_eq!(run.start(), vec![]);
    assert_eq!(run.session("c1", SessionRequest::SwitchUser), vec![]);
    assert_eq!(
        run.removed("c5", Some("c1")),
        greeter_spawn(1),
        "c1 asked for the login screen"
    );

    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    run.recover(vec![anna_on_7(), orphan()]);
    run.start();
    assert_eq!(run.removed("c1", None), vec![Effect::Disallocate(7)]);
    assert_eq!(run.removed("c5", None), greeter_spawn(1));
}

#[test]
fn a_late_session_of_a_timed_out_start_leaves_the_next_starting_login_on_its_vt() {
    let mut run = Run::booted();
    run.greeter(create("anna"));
    run.worker(2, authed("anna", 1000));
    run.greeter(GreeterRequest::StartSession);
    run.on(Input::Timer(Timer::Start(WorkerId(2))));
    run.greeter(create("bob"));
    run.worker(3, authed("bob", 1001));
    run.greeter(GreeterRequest::StartSession);
    let effects = run.worker(2, started("c2", 1000));
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::Log(..), Effect::TerminateSession(id)] if id == "c2"
        ),
        "bob's start keeps the console: {effects:?}"
    );
}

#[test]
fn a_timed_out_start_whose_session_comes_up_late_returns_to_the_login_screen() {
    let mut run = Run::booted();
    run.greeter(create("anna"));
    run.worker(2, authed("anna", 1000));
    run.greeter(GreeterRequest::StartSession);
    run.on(Input::Timer(Timer::Start(WorkerId(2))));
    let effects = run.worker(2, started("c2", 1000));
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::Log(..), Effect::TerminateSession(id), Effect::SwitchTo(1)] if id == "c2"
        ),
        "{effects:?}"
    );
    let notes: Vec<_> = run.greeting().users.iter().map(|u| u.note).collect();
    assert_eq!(notes, [None, None], "c2 never entered the table");
    assert_eq!(run.exited(2), vec![Effect::Disallocate(7)]);

    let mut run = Run::booted();
    run.greeter(create("anna"));
    run.worker(2, authed("anna", 1000));
    run.greeter(GreeterRequest::StartSession);
    run.on(Input::Timer(Timer::Start(WorkerId(2))));
    assert_eq!(run.removed("c9", Some("c8")), vec![]);
    assert_eq!(
        run.removed("c2", None),
        vec![Effect::SwitchTo(1)],
        "the timed-out worker's session left the seat empty"
    );
    run.exited(2);
    assert_eq!(
        run.removed("c3", None),
        vec![],
        "no worker timed out any more"
    );
}

#[test]
fn a_greeter_that_never_starts_times_out_and_counts_as_failed() {
    let mut run = Run::booted();
    let effects = run.on(Input::Timer(Timer::Start(GREETER)));
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::Kill(GREETER), Effect::Log(..), ..]
        ),
        "{effects:?}"
    );
    assert_eq!(effects[2..], greeter_spawn(2)[..]);
    assert_eq!(
        run.worker(1, started("c1", 960)),
        vec![
            Effect::Log(
                log::Level::Warn,
                "session c1 opened after its start timed out; ending it".into()
            ),
            Effect::TerminateSession("c1".into()),
        ]
    );
    assert_eq!(run.exited(1), vec![]);
    run.on(Input::Timer(Timer::Start(WorkerId(2))));
    let effects = run.on(Input::Timer(Timer::Start(WorkerId(3))));
    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::Kill(WorkerId(3)),
                Effect::Log(..),
                Effect::SwitchTo(2),
                Effect::Log(..)
            ]
        ),
        "{effects:?}"
    );
    assert_eq!(
        run.removed("c9", None),
        vec![],
        "the text login stays: a stalled greeter holds no VT but 1"
    );

    let mut run = Run::booted();
    assert_eq!(run.worker(1, started("c1", 960)), vec![]);
    assert_eq!(run.on(Input::Timer(Timer::Start(GREETER))), vec![]);
}

#[test]
fn start_session_without_a_worker_says_the_login_ended() {
    let mut run = Run::booted();
    run.greeter(create("anna"));
    run.worker(2, authed("anna", 1000));
    run.d.conversation.as_mut().expect("conversation").worker = None;
    assert_eq!(
        run.greeter(GreeterRequest::StartSession),
        reply(error(ErrorKind::Failed, WORKER_ENDED))
    );
}

#[test]
fn greeter_requests_while_no_greeter_runs_are_ignored() {
    let config = LoginConfig {
        autologin_user: Some("anna".into()),
        ..LoginConfig::default()
    };
    let mut run = Run::new(config, Remembered::default());
    run.recover(vec![]);
    run.start();
    assert_eq!(run.greeter(create("bert")), vec![]);
    assert_eq!(run.greeter(GreeterRequest::Hello), vec![]);
}

#[test]
fn a_greeters_gone_arriving_after_its_exit_spares_the_next_greeter() {
    let mut run = Run::booted();
    assert_eq!(run.exited(1), greeter_spawn(2));
    assert_eq!(run.on(Input::GreeterGone(GREETER)), vec![]);
    assert_eq!(
        run.on(Input::GreeterRequest(GREETER, create("anna"))),
        vec![]
    );
    assert_eq!(run.greeter(create("anna")), spawn(3, Mode::Login, "anna"));
}

#[test]
fn an_account_resolving_to_uid_0_fails_like_a_wrong_password() {
    let mut run = Run::booted();
    assert_eq!(run.greeter(create("toor")), spawn(2, Mode::Login, "toor"));
    assert_eq!(
        run.worker(2, authed("root", 0)),
        vec![
            Effect::Log(log::Level::Warn, "a login as root was refused".into()),
            Effect::Kill(WorkerId(2)),
            Effect::Reply(error(ErrorKind::Auth, AUTH_FAILED)),
        ]
    );
    assert_eq!(
        run.greeter(GreeterRequest::StartSession),
        reply(error(ErrorKind::Refused, NOT_AUTHENTICATED))
    );
}

#[test]
fn autologin_never_unlocks_a_session_its_user_already_has() {
    let config = LoginConfig {
        autologin_user: Some("anna".into()),
        ..LoginConfig::default()
    };
    let mut run = Run::new(config, Remembered::default());
    run.recover(vec![]);
    run.start();
    run.d.sessions.insert(SessionEntry {
        id: "c1".into(),
        user: "anna".into(),
        uid: 1000,
        vt: 8,
        started: None,
    });
    let effects = run.worker(1, started("c2", 1000));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::UnlockSession(_))),
        "{effects:?}"
    );
    assert_eq!(effects[0], Effect::TerminateSession("c2".into()));
    assert_eq!(effects[2..], greeter_spawn(2)[..], "{effects:?}");
    assert!(run.d.conversation.is_none());
}

#[test]
fn a_login_the_greeter_leaves_unanswered_for_auth_wait_is_cancelled() {
    let mut run = Run::booted();
    run.greeter(create("anna"));
    run.worker(2, secret("Password:"));
    run.now += AUTH_WAIT / 2;
    run.greeter(answer("hunter2"));
    run.now += AUTH_WAIT / 2;
    assert_eq!(
        run.on(Input::Timer(Timer::Auth(WorkerId(2)))),
        vec![Effect::Arm {
            timer: Timer::Auth(WorkerId(2)),
            after: AUTH_WAIT / 2,
        }],
        "an answer restarts the wait"
    );
    run.now += AUTH_WAIT / 2;
    assert_eq!(
        run.on(Input::Timer(Timer::Auth(WorkerId(2)))),
        vec![
            Effect::Kill(WorkerId(2)),
            Effect::Reply(error(ErrorKind::Failed, AUTH_TIMED_OUT)),
        ]
    );
    assert_eq!(run.greeter(create("bert")), spawn(3, Mode::Login, "bert"));
}

#[test]
fn the_auth_wait_ends_with_authentication() {
    let mut run = Run::booted();
    run.greeter(create("anna"));
    run.worker(2, authed("anna", 1000));
    run.now += AUTH_WAIT * 2;
    assert_eq!(run.on(Input::Timer(Timer::Auth(WorkerId(2)))), vec![]);
}

#[test]
fn an_orphan_that_does_not_end_is_retried_then_given_up_on() {
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    let orphan = logind(
        "c5",
        GREETER_USER,
        960,
        1,
        "greeter",
        "reprise-login-greeter",
    );
    assert_eq!(
        run.recover(vec![orphan]),
        vec![Effect::TerminateSession("c5".into())]
    );
    assert_eq!(run.start(), vec![]);
    let failed = || Input::TerminateFailed {
        id: "c5".into(),
        seat_active: None,
    };
    for n in 0..TERMINATE_TRIES - 1 {
        assert_eq!(
            run.on(failed()),
            vec![Effect::Arm {
                timer: Timer::Terminate("c5".into()),
                after: TERMINATE_RETRY * 2u32.pow(n),
            }]
        );
        assert_eq!(
            run.on(Input::Timer(Timer::Terminate("c5".into()))),
            vec![Effect::TerminateSession("c5".into())]
        );
    }
    let effects = run.on(failed());
    assert!(
        matches!(&effects[0], Effect::Log(log::Level::Warn, text) if text.contains("c5")),
        "{effects:?}"
    );
    assert_eq!(effects[1..], greeter_spawn(1)[..]);
    assert_eq!(run.removed("c5", None), vec![], "its late removal");
    assert_eq!(run.on(Input::Timer(Timer::Terminate("c5".into()))), vec![]);
}

#[test]
fn a_resume_whose_switch_fails_locks_the_session_again_and_keeps_the_login_screen() {
    let mut run = Run::anna_switched(Remembered::default());
    assert_eq!(run.greeter(create("anna")), spawn(2, Mode::Reauth, "anna"));
    run.worker(2, authed("anna", 1000));
    assert_eq!(run.greeter(GreeterRequest::StartSession), unlock("c1", 1));
    assert_eq!(
        run.on(Input::Unlocked { id: "c1".into() }),
        vec![Effect::SwitchTo(7), Effect::Reply(GreeterReply::Success)]
    );
    assert_eq!(
        run.on(Input::SwitchFailed { vt: 7 }),
        vec![Effect::LockSession("c1".into()), Effect::Kill(WorkerId(1)),],
        "the leaving greeter is replaced"
    );
    assert_eq!(run.exited(1), greeter_spawn(3));
    assert_eq!(
        run.on(Input::SwitchFailed { vt: 9 }),
        vec![],
        "no session on VT 9"
    );
}

#[test]
fn recovery_locks_every_session_that_is_not_on_screen() {
    let found = || {
        vec![
            anna_on_7(),
            logind("c2", "bert", 1001, 8, "user", "reprise-login"),
        ]
    };
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    assert_eq!(
        run.d.recover(run.now, found(), Some("c1")),
        vec![Effect::LockSession("c2".into())]
    );
    let mut run = Run::new(LoginConfig::default(), Remembered::default());
    assert_eq!(
        run.d.recover(run.now, found(), None),
        vec![
            Effect::LockSession("c1".into()),
            Effect::LockSession("c2".into())
        ]
    );
}

#[test]
fn the_console_moving_to_vt_1_with_no_login_screen_brings_one() {
    let mut run = Run::booted();
    run.log_in(2, "anna", 1000, "c1", 7);
    assert_eq!(
        run.on(Input::ConsoleOnGreeterVt),
        vec![Effect::Kill(WorkerId(1))],
        "the leaving greeter is replaced"
    );
    assert_eq!(run.exited(1), greeter_spawn(3));
    assert_eq!(run.on(Input::ConsoleOnGreeterVt), vec![], "one runs");

    let mut run = Run::booted();
    run.log_in(2, "anna", 1000, "c1", 7);
    assert_eq!(run.exited(1), vec![], "the greeter left");
    assert_eq!(run.on(Input::ConsoleOnGreeterVt), greeter_spawn(3));
}

#[test]
fn the_console_on_vt_1_while_autologin_starts_brings_no_login_screen() {
    let config = LoginConfig {
        autologin_user: Some("anna".into()),
        ..LoginConfig::default()
    };
    let mut run = Run::new(config, Remembered::default());
    run.recover(vec![]);
    run.start();
    assert_eq!(run.on(Input::ConsoleOnGreeterVt), vec![]);
}

#[test]
fn a_login_first_ends_its_users_sessions_the_table_does_not_hold() {
    let ends = || {
        vec![
            Effect::Log(
                Level::Info,
                "ending anna's leftover session c9 before a new one".into(),
            ),
            Effect::StopSession("c9".into()),
        ]
    };
    let mut run = Run::booted();
    run.world.login_sessions = vec![
        logind("c9", "anna", 1000, 8, "user", "reprise-login"),
        logind("c8", "bert", 1001, 9, "user", "reprise-login"),
        logind("c7", "anna", 1000, 0, "user", "sshd"),
    ];
    assert_eq!(run.greeter(create("anna")), spawn(2, Mode::Login, "anna"));
    run.worker(2, authed("anna", 1000));
    let mut expected = ends();
    expected.extend(start_on(2, 7));
    assert_eq!(run.greeter(GreeterRequest::StartSession), expected);

    let config = LoginConfig {
        autologin_user: Some("anna".into()),
        ..LoginConfig::default()
    };
    let mut run = Run::new(config, Remembered::default());
    run.world.login_sessions = vec![logind(
        "c9",
        "anna",
        1000,
        8,
        "user",
        "reprise-login-autologin",
    )];
    let started = run.start();
    assert_eq!(started[1..3], ends()[..], "{started:?}");
    assert!(matches!(started[3], Effect::ToWorker { .. }), "{started:?}");
}
