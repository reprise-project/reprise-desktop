use super::*;

#[test]
fn a_background_session_ending_leaves_the_console_alone() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    assert_eq!(
        run.removed("c2", Some("g1")),
        [Effect::Inhibit(Inhibit::Nothing)],
        "no greeter, switch or notice"
    );
    assert_eq!(run.exited(2), [Effect::Disallocate(7)]);
    let mut run = anna_recovered(enabled(), "g1", Mode::Remote.service());
    assert_eq!(
        run.removed("c1", Some("g1")),
        [Effect::Disallocate(7), Effect::Inhibit(Inhibit::Nothing)]
    );
}

#[test]
fn a_background_session_switched_to_on_the_console_becomes_the_consoles() {
    let remote = MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    };
    let mut run = remote_run(remote);
    run.anna_in_background(7, 2);
    assert_eq!(
        quiet(run.on(Input::ConsoleOnVt(7))),
        [Effect::Inhibit(Inhibit::Nothing)]
    );
    assert_eq!(
        run.d.sessions.by_id("c2").map(|e| e.background),
        Some(false)
    );
    let disconnected = Timer::Disconnected {
        id: "c2".into(),
        generation: 1,
    };
    assert_eq!(run.fire(disconnected), [], "someone uses it at the console");
    run.viewers(ANNA, 1);
    assert_eq!(
        run.viewers(ANNA, 0),
        [Effect::Inhibit(Inhibit::Nothing)],
        "no Disconnected timer"
    );
    let effects = run.removed("c2", None);
    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::SwitchTo(GREETER_VT),
                Effect::Reply(GreeterReply::Greeting(_))
            ]
        ),
        "the login screen comes back to the console: {effects:?}"
    );
}

#[test]
fn a_restart_keeps_each_session_background_or_console_as_it_was() {
    let remote = MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    };
    let mut run = remote_run(remote.clone());
    run.anna_in_background(7, 2);
    assert_eq!(run.d.background_ids(), BTreeSet::from(["c2".to_string()]));
    run.on(Input::ConsoleOnVt(7));
    assert_eq!(run.d.background_ids(), BTreeSet::new());

    let restarted = |before: Option<BTreeSet<String>>| {
        let mut run = Run::machine(config(remote.clone()));
        run.d.background_before(before);
        let found = vec![
            logind("c2", "anna", ANNA, 7, "user", "reprise-login-remote"),
            logind("c3", "bert", BERT, 8, "user", "reprise-login"),
        ];
        run.d.recover(found, Some("g1"));
        run
    };
    let background = |run: &Run, id: &str| run.d.sessions.by_id(id).map(|e| e.background);
    let mut run = restarted(Some(BTreeSet::new()));
    assert_eq!(
        background(&run, "c2"),
        Some(false),
        "anna used it at the console"
    );
    run.on(Input::IntakeOpen { uid: ANNA });
    let effects = run.viewers(ANNA, 0);
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Arm { .. })),
        "{effects:?}"
    );
    let run = restarted(Some(BTreeSet::from(["c3".to_string()])));
    assert_eq!(background(&run, "c2"), Some(false));
    assert_eq!(
        background(&run, "c3"),
        Some(true),
        "bert's was reached remotely"
    );
    let run = restarted(None);
    assert_eq!(
        (background(&run, "c2"), background(&run, "c3")),
        (Some(true), Some(false)),
        "with no record, a remote login's session off the seat is a background one"
    );
}

#[test]
fn the_inhibitor_follows_viewers_and_background_sessions() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    assert_eq!(
        run.viewers(ANNA, 1),
        [Effect::Inhibit(Inhibit::SleepAndIdle)]
    );
    assert_eq!(run.viewers(ANNA, 2), []);
    assert_eq!(run.viewers(ANNA, 0), [Effect::Inhibit(Inhibit::Idle)]);
    assert_eq!(
        run.removed("c2", Some("g1")),
        [Effect::Inhibit(Inhibit::Nothing)]
    );
    let mut run = anna_recovered(enabled(), "c1", Mode::Login.service());
    assert_eq!(
        run.viewers(ANNA, 1),
        [Effect::Inhibit(Inhibit::SleepAndIdle)],
        "viewers of the console's session hold off sleep too"
    );
    assert_eq!(run.viewers(ANNA, 0), [Effect::Inhibit(Inhibit::Nothing)]);
}

#[test]
fn a_disconnected_background_session_ends_after_end_disconnected_after() {
    let remote = MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    };
    let disconnected = |id: &str, generation| Timer::Disconnected {
        id: id.into(),
        generation,
    };
    let arm = |generation| disconnect_arm("c2", generation, 30);
    let mut run = remote_run(remote.clone());
    run.anna_in_background(7, 2);
    run.gone(7);
    run.viewers(ANNA, 1);
    assert_eq!(
        run.viewers(ANNA, 0),
        [arm(2), Effect::Inhibit(Inhibit::Idle)]
    );
    assert_eq!(
        quiet(run.fire(disconnected("c2", 2))),
        [Effect::TerminateSession("c2".into())]
    );

    let mut run = remote_run(remote.clone());
    run.anna_in_background(7, 2);
    run.gone(7);
    run.viewers(ANNA, 1);
    run.viewers(ANNA, 0);
    run.viewers(ANNA, 1);
    assert_eq!(
        run.viewers(ANNA, 0),
        [arm(3), Effect::Inhibit(Inhibit::Idle)]
    );
    assert_eq!(run.fire(disconnected("c2", 1)), [], "a stale generation");
    assert_eq!(run.fire(disconnected("c2", 2)), [], "a stale generation");
    assert_eq!(
        quiet(run.fire(disconnected("c2", 3))),
        [Effect::TerminateSession("c2".into())]
    );

    let mut run = remote_run(remote.clone());
    run.anna_in_background(7, 2);
    run.gone(7);
    assert_eq!(
        quiet(run.fire(disconnected("c2", 1))),
        [Effect::TerminateSession("c2".into())],
        "a session no client ever reached ends too"
    );

    let mut run = Run::machine(config(remote.clone()));
    let found = vec![logind(
        "c1",
        "anna",
        ANNA,
        7,
        "user",
        "reprise-login-remote",
    )];
    assert_eq!(
        run.d.recover(found, Some("g1")),
        [
            Effect::OpenIntake { uid: ANNA },
            Effect::Inhibit(Inhibit::Idle),
        ]
    );
    run.on(Input::IntakeOpen { uid: ANNA });
    assert_eq!(run.viewers(ANNA, 0), [disconnect_arm("c1", 1, 30)]);
    assert_eq!(
        quiet(run.fire(disconnected("c1", 1))),
        [Effect::TerminateSession("c1".into())],
        "a recovered background session with no client ends"
    );

    let mut run = anna_recovered(remote, "c1", Mode::Login.service());
    run.viewers(ANNA, 1);
    assert_eq!(
        run.viewers(ANNA, 0),
        [Effect::Inhibit(Inhibit::Nothing)],
        "a console session arms nothing"
    );
}

fn ending_after(min: Option<u32>) -> Input {
    Input::ConfigReloaded(config(MachineRemote {
        end_disconnected_after_min: min,
        ..enabled()
    }))
}

#[test]
fn never_cancels_an_armed_disconnected_timer_and_a_value_re_arms_it_from_the_disconnect() {
    let mut run = anna_unviewed_for_30();
    assert_eq!(
        run.after(Duration::from_secs(600)).on(ending_after(None)),
        [front_on()]
    );
    let gen1 = Timer::Disconnected {
        id: "c2".into(),
        generation: 1,
    };
    assert_eq!(run.after(Duration::from_secs(1200)).fire(gen1), []);
    assert_eq!(
        run.after(Duration::from_secs(60))
            .on(ending_after(Some(40))),
        [
            Effect::Arm {
                timer: Timer::Disconnected {
                    id: "c2".into(),
                    generation: 2
                },
                after: Duration::from_secs(40 * 60 - 1860),
            },
            front_on()
        ],
        "40 minutes from the disconnect, not from the reload"
    );
}

#[test]
fn a_login_reaching_an_unviewed_background_session_restarts_its_time() {
    let mut run = anna_unviewed_for_30();
    run.on(Input::IntakeOpen { uid: ANNA });
    run.after(Duration::from_secs(29 * 60));
    assert_eq!(
        run.verify(8, 3, "anna", ANNA),
        [
            front(FrontEvent::Verified { conn: 8 }),
            to_worker(3, ToWorker::Cancel),
            disconnect_arm("c2", 2, 30),
            front(FrontEvent::Accepted { conn: 8 }),
        ]
    );
    let gen1 = Timer::Disconnected {
        id: "c2".into(),
        generation: 1,
    };
    assert_eq!(run.after(Duration::from_secs(60)).fire(gen1), []);
}

#[test]
fn a_disconnect_time_running_out_waits_for_a_login_in_flight_to_the_session() {
    let disconnected = |generation| Timer::Disconnected {
        id: "c2".into(),
        generation,
    };
    let held = || Effect::Arm {
        timer: disconnected(2),
        after: HANDOVER_DEADLINE,
    };
    let mut run = anna_unviewed_for_30();
    run.after(Duration::from_secs(30 * 60 - 1));
    run.on(tls(8, "anna", "pw"));
    run.on(member(8, ANNA));
    assert_eq!(
        run.after(Duration::from_secs(1)).fire(disconnected(1)),
        [held()],
        "the PAM check holds it"
    );
    run.worker(3, failed(ErrorKind::Failed, "wrong password"));
    assert_eq!(
        quiet(run.after(HANDOVER_DEADLINE).fire(disconnected(2))),
        [Effect::TerminateSession("c2".into())],
        "a refused login holds it no longer"
    );

    let mut run = anna_unviewed_for_30();
    run.after(Duration::from_secs(30 * 60));
    run.on(tls(8, "anna", "pw"));
    run.on(member(8, ANNA));
    run.worker(3, secret("Password: "));
    assert_eq!(run.fire(disconnected(1)), [held()]);
    run.worker(3, authed("anna", ANNA));
    assert_eq!(
        run.after(HANDOVER_DEADLINE).fire(disconnected(2)),
        [],
        "the verified login restarted its time"
    );
}

#[test]
fn a_failed_end_of_an_unviewed_background_session_is_tried_again_unless_a_client_came() {
    let disconnected = |generation| Timer::Disconnected {
        id: "c2".into(),
        generation,
    };
    let failed = || Input::TerminateFailed {
        id: "c2".into(),
        seat_active: None,
    };
    let mut run = anna_unviewed_for_30();
    let end = quiet(
        run.after(Duration::from_secs(30 * 60))
            .fire(disconnected(1)),
    );
    assert_eq!(end, [Effect::TerminateSession("c2".into())]);
    assert_eq!(
        run.on(failed()),
        [Effect::Arm {
            timer: disconnected(2),
            after: TERMINATE_RETRY,
        }]
    );
    assert_eq!(
        quiet(run.after(TERMINATE_RETRY).fire(disconnected(2))),
        [Effect::TerminateSession("c2".into())]
    );
    assert_eq!(
        run.on(failed()),
        [Effect::Arm {
            timer: disconnected(3),
            after: TERMINATE_RETRY * 2,
        }]
    );
    run.on(Input::IntakeOpen { uid: ANNA });
    run.viewers(ANNA, 1);
    assert_eq!(
        quiet(run.after(TERMINATE_RETRY * 2).fire(disconnected(3))),
        [],
        "a client connected meanwhile"
    );
}

#[test]
fn a_new_end_after_a_client_came_and_left_counts_its_failures_afresh() {
    let failed = || Input::TerminateFailed {
        id: "c2".into(),
        seat_active: None,
    };
    let fire_armed = |run: &mut Run, effects: Vec<Effect>| -> Vec<Effect> {
        let armed = effects.iter().find_map(|e| match e {
            Effect::Arm { timer, after } => Some((timer.clone(), *after)),
            _ => None,
        });
        let (timer, after) = armed.unwrap_or_else(|| panic!("a timer armed, got {effects:?}"));
        quiet(run.after(after).fire(timer))
    };
    let mut run = anna_unviewed_for_30();
    let ended = |effects: &[Effect]| effects == [Effect::TerminateSession("c2".into())];
    let end = quiet(
        run.after(Duration::from_secs(30 * 60))
            .fire(Timer::Disconnected {
                id: "c2".into(),
                generation: 1,
            }),
    );
    assert!(ended(&end), "{end:?}");
    let retry = run.on(failed());
    assert!(ended(&fire_armed(&mut run, retry)));
    run.on(failed());
    run.on(Input::IntakeOpen { uid: ANNA });
    run.viewers(ANNA, 1);
    let left = quiet(run.viewers(ANNA, 0));
    assert!(ended(&fire_armed(&mut run, left)));
    assert_eq!(
        quiet(run.on(failed())).as_slice(),
        [Effect::Arm {
            timer: Timer::Disconnected {
                id: "c2".into(),
                generation: 5,
            },
            after: TERMINATE_RETRY,
        }],
        "the first retry of a new end waits the first wait"
    );
}

#[test]
fn a_shorter_end_disconnected_after_ends_a_session_past_it_at_once() {
    let gen2 = || Timer::Disconnected {
        id: "c2".into(),
        generation: 2,
    };
    let mut run = anna_unviewed_for_30();
    assert_eq!(
        run.after(Duration::from_secs(600))
            .on(ending_after(Some(5))),
        [
            Effect::Arm {
                timer: gen2(),
                after: Duration::ZERO,
            },
            front_on()
        ]
    );
    assert_eq!(
        quiet(run.fire(gen2())),
        [Effect::TerminateSession("c2".into())]
    );
    let mut run = anna_unviewed_for_30();
    assert_eq!(
        run.after(Duration::from_secs(60)).on(ending_after(Some(5))),
        [
            Effect::Arm {
                timer: Timer::Disconnected {
                    id: "c2".into(),
                    generation: 2
                },
                after: Duration::from_secs(240),
            },
            front_on()
        ]
    );
    let gen1 = Timer::Disconnected {
        id: "c2".into(),
        generation: 1,
    };
    assert_eq!(run.fire(gen1), [], "the old timer is stale");
}

#[test]
fn a_shorter_end_disconnected_after_never_ends_the_session_the_console_shows() {
    let mut run = anna_unviewed_for_30();
    run.world.active_vt = 7;
    let effects = run
        .after(Duration::from_secs(600))
        .on(ending_after(Some(5)));
    assert!(
        !effects.contains(&Effect::TerminateSession("c2".into())),
        "{effects:?}"
    );
    let gen2 = Timer::Disconnected {
        id: "c2".into(),
        generation: 2,
    };
    assert_eq!(quiet(run.fire(gen2)), [Effect::Inhibit(Inhibit::Nothing)]);
    assert!(!run.d.sessions.by_id("c2").unwrap().background);
}

#[test]
fn a_longer_end_disconnected_after_re_arms_from_the_disconnect() {
    let mut run = anna_unviewed_for_30();
    assert_eq!(
        run.after(Duration::from_secs(600))
            .on(ending_after(Some(60))),
        [
            Effect::Arm {
                timer: Timer::Disconnected {
                    id: "c2".into(),
                    generation: 2
                },
                after: Duration::from_secs(50 * 60),
            },
            front_on()
        ]
    );
    let gen1 = Timer::Disconnected {
        id: "c2".into(),
        generation: 1,
    };
    assert_eq!(run.after(Duration::from_secs(1200)).fire(gen1), []);
    let gen2 = Timer::Disconnected {
        id: "c2".into(),
        generation: 2,
    };
    assert_eq!(
        quiet(run.after(Duration::from_secs(1800)).fire(gen2)),
        [Effect::TerminateSession("c2".into())]
    );
    assert_eq!(
        run.after(Duration::from_secs(1)).on(ending_after(Some(60))),
        [front_on()],
        "an unchanged value re-arms nothing"
    );
}

#[test]
fn resuming_a_background_session_at_the_login_screen_makes_it_the_consoles() {
    let remote = MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    };
    let mut run = remote_run(remote);
    run.anna_in_background(7, 2);
    run.viewers(ANNA, 1);
    run.viewers(ANNA, 0);
    assert_eq!(run.greeter(create("anna")), spawn(3, Mode::Reauth, "anna"));
    assert_eq!(
        run.worker(3, authed("anna", ANNA)),
        reply(GreeterReply::Success)
    );
    assert_eq!(run.greeter(GreeterRequest::StartSession), unlock("c2", 1));
    let effects = run.on(Input::Unlocked { id: "c2".into() });
    assert_eq!(effects[0], Effect::SwitchTo(7));
    assert_eq!(effects.last(), Some(&Effect::Inhibit(Inhibit::Nothing)));
    assert_eq!(
        quiet(run.on(ending_after(Some(60)))),
        [front_on()],
        "a console session arms nothing"
    );
    for generation in [1, 2] {
        let disconnected = Timer::Disconnected {
            id: "c2".into(),
            generation,
        };
        assert_eq!(run.fire(disconnected), []);
    }
}

#[test]
fn greeting_notes_a_session_with_remote_viewers_as_remote() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    let notes = |run: &mut Run| -> Vec<_> { run.greeting().users.iter().map(|u| u.note).collect() };
    assert_eq!(notes(&mut run), [Some(Note::LoggedOn), None]);
    run.viewers(ANNA, 1);
    assert_eq!(notes(&mut run), [Some(Note::Remote), None]);
    run.on(Input::IntakeLost { uid: ANNA });
    assert_eq!(
        notes(&mut run),
        [Some(Note::Remote), None],
        "while it reopens"
    );
    run.on(Input::IntakeLost { uid: ANNA });
    assert_eq!(notes(&mut run), [Some(Note::LoggedOn), None]);
}

#[test]
fn the_login_screen_hears_each_change_of_its_notes() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    assert_eq!(run.greeting().users[0].note, Some(Note::LoggedOn));
    let greeted = |effects: Vec<Effect>| {
        effects.into_iter().find_map(|e| match e {
            Effect::Reply(GreeterReply::Greeting(g)) => Some(g.users[0].note),
            _ => None,
        })
    };
    assert_eq!(greeted(run.viewers(ANNA, 1)), Some(Some(Note::Remote)));
    assert_eq!(
        greeted(run.viewers(ANNA, 2)),
        None,
        "the notes stay the same"
    );
    assert_eq!(greeted(run.viewers(ANNA, 0)), Some(Some(Note::LoggedOn)));
}

#[test]
fn a_background_session_removed_before_its_start_reports_leaves_no_entry() {
    let mut run = remote_run(enabled());
    let mut want = vec![front(FrontEvent::Verified { conn: 7 })];
    want.extend(background_start(2, 7));
    assert_eq!(run.verify(7, 2, "anna", ANNA), want);
    assert_eq!(run.removed("c2", Some("g1")), []);
    assert_eq!(
        quiet(run.worker(2, started("c2", ANNA))),
        [],
        "no intake link and no inhibitor"
    );
    assert_eq!(run.d.sessions.by_uid(ANNA), None);
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), denied(7, 0));
    assert_eq!(run.exited(2), [Effect::Disallocate(7)]);
}

#[test]
fn a_background_start_finishing_beside_a_console_session_ends_it_and_hands_over_to_the_console_one()
{
    let mut run = remote_run(enabled());
    let mut want = vec![front(FrontEvent::Verified { conn: 7 })];
    want.extend(background_start(2, 7));
    assert_eq!(run.verify(7, 2, "anna", ANNA), want);
    run.log_in(3, "anna", ANNA, "c3", 8);
    let effects = run.worker(2, started("c2", ANNA));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Log(Level::Warn, m) if m.contains("beside"))),
        "{effects:?}"
    );
    assert_eq!(
        quiet(effects),
        [
            Effect::TerminateSession("c2".into()),
            Effect::OpenIntake { uid: ANNA },
        ]
    );
    assert_eq!(
        run.d.sessions.by_uid(ANNA).map(|e| e.id.as_str()),
        Some("c3")
    );
    assert_eq!(
        run.on(Input::IntakeOpen { uid: ANNA }),
        [front(FrontEvent::Accepted { conn: 7 })]
    );
    assert_eq!(
        run.on(ready(7, "anna")),
        [Effect::Forward { conn: 7, uid: ANNA }]
    );
}
