use super::*;

fn unread(id: &str, uid: u32, seat: &str) -> Unread {
    Unread {
        id: id.into(),
        uid,
        user: String::new(),
        seat: seat.into(),
    }
}

#[test]
fn a_session_logind_could_not_describe_at_recovery_gets_no_session_beside_it_until_it_ends() {
    let mut run = Run::machine(config(enabled()));
    run.d.recover_unread(vec![unread("c1", ANNA, "seat0")]);
    assert_eq!(run.d.recover(vec![], None), vec![]);
    run.start();
    let effects = run.verify(7, 2, "anna", ANNA);
    assert!(
        (effects.iter())
            .any(|e| matches!(e, Effect::Log(Level::Warn, text) if text.contains("session c1"))),
        "{effects:?}"
    );
    assert_eq!(
        quiet(effects),
        [
            front(FrontEvent::Verified { conn: 7 }),
            to_worker(2, ToWorker::Cancel),
        ]
    );
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), denied(7, 0));
    run.removed("c1", Some("g1"));
    run.anna_in_background(8, 3);
}

fn autologin_anna() -> MachineConfig {
    MachineConfig {
        login: repriselogin::machine::LoginConfig {
            autologin_user: Some("anna".into()),
            ..Default::default()
        },
        ..MachineConfig::default()
    }
}

#[test]
fn a_session_logind_could_not_describe_at_recovery_counts_as_running_at_start() {
    let mut run = Run::machine(autologin_anna());
    run.d.recover_unread(vec![unread("c1", ANNA, "seat0")]);
    assert_eq!(run.recover(vec![]), []);
    assert_eq!(
        run.start(),
        [],
        "the seat shows a session, and autologin waits"
    );

    let mut run = Run::machine(autologin_anna());
    run.d.recover_unread(vec![unread("c1", ANNA, "seat0")]);
    assert_eq!(run.d.recover(vec![], None), []);
    assert_eq!(run.start(), greeter_spawn(1), "no autologin beside it");
}

#[test]
fn a_console_login_for_a_user_whose_session_logind_could_not_describe_starts_none() {
    let mut run = Run::machine(MachineConfig::default());
    run.d.recover_unread(vec![unread("c1", ANNA, "seat0")]);
    run.d.recover(vec![], None);
    assert_eq!(run.start(), greeter_spawn(1));
    assert_eq!(run.greeter(create("anna")), spawn(2, Mode::Login, "anna"));
    let effects = quiet(run.worker(2, authed("anna", ANNA)));
    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::Kill(WorkerId(2)),
                Effect::Reply(GreeterReply::Error {
                    kind: ErrorKind::Failed,
                    ..
                })
            ]
        ),
        "{effects:?}"
    );
}

#[test]
fn a_session_read_after_recovery_never_replaces_its_users_session_in_the_table() {
    let mut run = Run::machine(MachineConfig::default());
    run.d.recover_unread(vec![unread("c1", ANNA, "seat0")]);
    let c9 = logind("c9", "anna", ANNA, 8, "user", "reprise-login");
    run.d.recover(vec![c9], Some("c9"));
    run.start();
    let read = Input::SessionRead {
        session: logind("c1", "anna", ANNA, 7, "user", "reprise-login"),
        seat_active: Some("c9".into()),
    };
    let effects = run.on(read);
    assert!(
        (effects.iter())
            .any(|e| matches!(e, Effect::Log(Level::Warn, text) if text.contains("c1"))),
        "{effects:?}"
    );
    assert_eq!(
        run.d.sessions.by_uid(ANNA).map(|e| e.id.as_str()),
        Some("c9")
    );
    assert!(run.d.sessions.by_id("c1").is_none());
}

#[test]
fn an_unread_session_on_screen_that_ends_brings_the_login_screen() {
    let mut run = Run::machine(MachineConfig::default());
    run.d.recover_unread(vec![unread("c1", ANNA, "seat0")]);
    assert_eq!(run.recover(vec![]), []);
    assert_eq!(run.start(), []);
    assert_eq!(run.removed("c1", None), greeter_spawn(1));
}

#[test]
fn an_unread_session_on_screen_read_as_closing_brings_the_login_screen() {
    let mut run = Run::machine(MachineConfig::default());
    run.d.recover_unread(vec![unread("c1", ANNA, "seat0")]);
    assert_eq!(run.recover(vec![]), []);
    assert_eq!(run.start(), []);
    let read = Input::SessionRead {
        session: closing(logind("c1", "anna", ANNA, 7, "user", "reprise-login")),
        seat_active: Some("c1".into()),
    };
    assert_eq!(run.on(read), greeter_spawn(1));
    assert_eq!(
        run.removed("c1", None),
        [],
        "the login screen is already up"
    );
}

#[test]
fn a_session_left_out_of_the_table_on_screen_that_ends_brings_the_login_screen() {
    let mut run = Run::machine(MachineConfig::default());
    run.d.recover_unread(vec![unread("c1", ANNA, "seat0")]);
    let c9 = logind("c9", "anna", ANNA, 8, "user", "reprise-login");
    run.d.recover(vec![c9], Some("c1"));
    assert_eq!(run.start(), []);
    let read = Input::SessionRead {
        session: logind("c1", "anna", ANNA, 7, "user", "reprise-login"),
        seat_active: Some("c1".into()),
    };
    run.on(read);
    assert!(run.d.sessions.by_id("c1").is_none());
    assert!(run.d.acts_on_removal("c1"));
    assert_eq!(run.removed("c1", None), greeter_spawn(1));
}

#[test]
fn only_seat0_sessions_logind_could_not_describe_at_recovery_hold_back_remote_logins() {
    let mut run = Run::machine(config(enabled()));
    let kept = run
        .d
        .recover_unread(vec![unread("c1", ANNA, ""), unread("c3", BERT, "seat0")]);
    assert_eq!(kept, [unread("c3", BERT, "seat0")]);
    run.d.recover(vec![], None);
    run.start();
    run.anna_in_background(7, 2);
}

#[test]
fn a_seat0_session_read_after_recovery_joins_the_table_and_is_linked() {
    let mut run = Run::machine(config(enabled()));
    run.d.recover_unread(vec![unread("c1", ANNA, "seat0")]);
    run.d.recover(vec![], None);
    run.start();
    let read = |id: &str| Input::SessionRead {
        session: logind(id, "anna", ANNA, 7, "user", "reprise-login-remote"),
        seat_active: Some("g1".into()),
    };
    assert_eq!(
        run.on(read("c1")),
        [
            Effect::OpenIntake { uid: ANNA },
            Effect::Inhibit(Inhibit::Idle),
        ]
    );
    assert_eq!(run.d.sessions.by_id("c1").map(|e| e.background), Some(true));
    assert_eq!(run.on(read("c1")), [], "a second read changes nothing");
    assert_eq!(
        run.on(Input::IntakeOpen { uid: ANNA }),
        [],
        "nothing waits on the link"
    );
    assert_eq!(
        quiet(run.verify(7, 2, "anna", ANNA)),
        [
            front(FrontEvent::Verified { conn: 7 }),
            to_worker(2, ToWorker::Cancel),
            front(FrontEvent::Accepted { conn: 7 }),
        ]
    );
}

#[test]
fn a_session_read_after_recovery_that_is_no_seat0_user_session_stops_holding_back_logins() {
    let mut run = Run::machine(config(enabled()));
    run.d.recover_unread(vec![unread("c1", ANNA, "seat0")]);
    run.d.recover(vec![], None);
    run.start();
    let session = logind("c1", "anna", ANNA, 0, "manager", "systemd-user");
    let read = Input::SessionRead {
        session,
        seat_active: Some("g1".into()),
    };
    assert_eq!(run.on(read), []);
    assert!(run.d.sessions.by_id("c1").is_none());
    run.anna_in_background(7, 2);
}

#[test]
fn recovery_marks_sessions_off_the_seat_background_and_links_only_them() {
    let mut run = Run::machine(config(enabled()));
    let found = vec![
        logind("c1", "anna", ANNA, 7, "user", "reprise-login-remote"),
        logind("c3", "bert", BERT, 8, "user", "reprise-login"),
        logind("c4", "carl", CARL, 9, "user", "reprise-login"),
    ];
    assert_eq!(
        run.d.recover(found, Some("c3")),
        [
            Effect::OpenIntake { uid: ANNA },
            Effect::Inhibit(Inhibit::Idle),
        ]
    );
    let background = |id| run.d.sessions.by_id(id).map(|e| e.background);
    assert_eq!(background("c1"), Some(true));
    assert_eq!(background("c3"), Some(false));
    assert_eq!(
        background("c4"),
        Some(false),
        "a console session off the seat is no background one"
    );
}

#[test]
fn a_recovered_switched_away_console_session_holds_no_inhibitor_and_never_ends_unviewed() {
    let remote = MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    };
    let mut run = Run::machine(config(remote));
    run.d
        .viewed_before(Some(BTreeSet::from(["c1".to_string()])));
    let found = vec![logind("c1", "anna", ANNA, 7, "user", "reprise-login")];
    assert_eq!(
        run.d.recover(found, Some("g1")),
        [link_look_up(ANNA, "anna")]
    );
    assert_eq!(
        run.viewers(ANNA, 1),
        [Effect::Inhibit(Inhibit::SleepAndIdle)]
    );
    assert_eq!(
        run.viewers(ANNA, 0),
        [Effect::Inhibit(Inhibit::Nothing)],
        "no Disconnected timer"
    );
}

#[test]
fn with_remote_logins_off_recovery_links_only_background_sessions() {
    let mut run = Run::machine(config(MachineRemote::default()));
    let found = vec![
        logind("c1", "anna", ANNA, 7, "user", "reprise-login-remote-nla"),
        logind("c3", "bert", BERT, 8, "user", "reprise-login"),
    ];
    assert_eq!(
        run.d.recover(found, Some("g1")),
        [
            Effect::OpenIntake { uid: ANNA },
            Effect::Inhibit(Inhibit::Idle),
        ]
    );
}

/// Recovers anna's console session `c1` on screen and bert's `c3`, which the last run counted a
/// client in.
fn bert_viewed_at_restart(remote: MachineRemote) -> (Run, Vec<Effect>) {
    let mut run = Run::machine(config(remote));
    run.d
        .viewed_before(Some(BTreeSet::from(["c3".to_string()])));
    let found = vec![
        logind("c1", "anna", ANNA, 7, "user", Mode::Login.service()),
        logind("c3", "bert", BERT, 8, "user", Mode::Login.service()),
    ];
    let effects = run.d.recover(found, Some("c1"));
    (run, effects)
}

#[test]
fn a_console_session_with_a_client_at_a_restart_is_linked_with_remote_logins_off() {
    let mut run = anna_recovered(enabled(), "c1", Mode::Login.service());
    run.on(link_member(ANNA, Some(ANNA), true));
    run.on(Input::IntakeOpen { uid: ANNA });
    run.viewers(ANNA, 1);
    assert_eq!(run.d.viewed_ids(), BTreeSet::from(["c1".to_string()]));
    run.viewers(ANNA, 0);
    assert_eq!(run.d.viewed_ids(), BTreeSet::new());

    let (mut run, effects) = bert_viewed_at_restart(MachineRemote::default());
    assert_eq!(
        effects,
        [Effect::OpenIntake { uid: BERT }],
        "only the session that had a client is linked"
    );
    assert_eq!(run.on(Input::IntakeOpen { uid: BERT }), []);
    assert_eq!(
        run.viewers(BERT, 1),
        [Effect::Inhibit(Inhibit::SleepAndIdle)],
        "the client still connected keeps the computer awake"
    );
}

#[test]
fn a_session_linked_at_a_restart_stays_listed_and_reopens_once_until_its_link_reports() {
    let (mut run, _) = bert_viewed_at_restart(MachineRemote::default());
    assert_eq!(
        run.d.viewed_ids(),
        BTreeSet::from(["c3".to_string()]),
        "listed for a second restart before the link reports"
    );
    assert_eq!(
        run.on(Input::IntakeLost { uid: BERT }),
        [Effect::ReopenIntake { uid: BERT }]
    );
    assert_eq!(run.d.viewed_ids(), BTreeSet::from(["c3".to_string()]));
    assert_eq!(run.on(Input::IntakeOpen { uid: BERT }), []);
    run.viewers(BERT, 0);
    assert_eq!(run.d.viewed_ids(), BTreeSet::new());
}

#[test]
fn a_session_linked_at_a_restart_is_not_linked_again_when_remote_logins_turn_on_after_its_link_ended()
 {
    let (mut run, _) = bert_viewed_at_restart(MachineRemote::default());
    run.on(Input::IntakeOpen { uid: BERT });
    run.viewers(BERT, 1);
    run.viewers(BERT, 0);
    run.on(Input::IntakeLost { uid: BERT });
    let effects = run.on(Input::ConfigReloaded(config(enabled())));
    assert_eq!(
        effects,
        [Effect::Front(FrontUnit::Start { port: 3389 })],
        "its list entry is used up"
    );
}

#[test]
fn with_remote_logins_on_a_restart_looks_up_the_group_for_a_session_that_had_a_client() {
    let (run, effects) = bert_viewed_at_restart(enabled());
    assert_eq!(
        effects,
        [link_look_up(BERT, "bert")],
        "anna's session had no client"
    );
    assert_eq!(run.d.viewed_ids(), BTreeSet::new());
}

#[test]
fn with_remote_logins_on_a_restart_links_no_console_session_without_a_client() {
    let mut run = Run::machine(config(enabled()));
    run.d.recover_unread(vec![unread("c5", CARL, "seat0")]);
    let found = vec![
        logind("c1", "anna", ANNA, 7, "user", Mode::Login.service()),
        logind("c3", "bert", BERT, 8, "user", Mode::Login.service()),
    ];
    let linking = |e: &Effect| matches!(e, Effect::LookUp { .. } | Effect::OpenIntake { .. });
    let effects = run.d.recover(found, Some("c1"));
    assert!(!effects.iter().any(linking), "{effects:?}");
    let read = Input::SessionRead {
        session: logind("c5", "carl", CARL, 9, "user", Mode::Login.service()),
        seat_active: Some("c1".into()),
    };
    let effects = run.on(read);
    assert!(
        !effects.iter().any(linking),
        "a session read late: {effects:?}"
    );
}

#[test]
fn a_session_with_a_client_read_after_a_restart_is_linked_with_remote_logins_off() {
    let mut run = Run::machine(config(MachineRemote::default()));
    run.d
        .viewed_before(Some(BTreeSet::from(["c3".to_string()])));
    run.d.recover_unread(vec![unread("c3", BERT, "seat0")]);
    run.d.recover(vec![], Some("g1"));
    assert_eq!(run.d.viewed_ids(), BTreeSet::from(["c3".to_string()]));
    let read = Input::SessionRead {
        session: logind("c3", "bert", BERT, 8, "user", Mode::Login.service()),
        seat_active: Some("g1".into()),
    };
    assert_eq!(run.on(read), [Effect::OpenIntake { uid: BERT }]);
    assert_eq!(run.d.viewed_ids(), BTreeSet::from(["c3".to_string()]));
}

#[test]
fn a_recovered_background_session_outside_the_group_is_linked_without_a_look_up() {
    let mut run = Run::machine(config(enabled()));
    let found = vec![logind(
        "c1",
        "anna",
        ANNA,
        7,
        "user",
        "reprise-login-remote",
    )];
    let effects = run.d.recover(found, Some("g1"));
    assert!(
        !(effects.iter()).any(|e| matches!(e, Effect::LookUp { .. })),
        "{effects:?}"
    );
    assert!(
        effects.contains(&Effect::OpenIntake { uid: ANNA }),
        "{effects:?}"
    );
}

#[test]
fn a_recovered_background_session_ends_once_its_server_reports_no_client_or_never_reports() {
    let remote = MachineRemote {
        end_disconnected_after_min: Some(30),
        ..MachineRemote::default()
    };
    let mut run = Run::machine(config(remote));
    let found = vec![logind(
        "c1",
        "anna",
        ANNA,
        7,
        "user",
        "reprise-login-remote",
    )];
    run.d.recover(found, Some("g1"));
    assert_eq!(run.on(Input::IntakeOpen { uid: ANNA }), []);
    assert_eq!(
        run.viewers(ANNA, 1),
        [Effect::Inhibit(Inhibit::SleepAndIdle)],
        "a client still connected keeps it"
    );
    assert_eq!(
        run.after(Duration::from_secs(10)).viewers(ANNA, 0),
        [disconnect_arm("c1", 1, 30), Effect::Inhibit(Inhibit::Idle)]
    );

    let remote = MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    };
    let mut run = Run::machine(config(remote));
    let found = vec![logind(
        "c1",
        "anna",
        ANNA,
        7,
        "user",
        "reprise-login-remote",
    )];
    run.d.recover(found, Some("g1"));
    let effects = run.on(Input::IntakeLost { uid: ANNA });
    assert!(
        (effects.iter())
            .any(|e| matches!(e, Effect::Log(Level::Warn, text) if text.contains("c1"))),
        "{effects:?}"
    );
    assert_eq!(
        quiet(effects),
        [disconnect_arm("c1", 1, 30)],
        "a server that never reports counts as no client"
    );
}
