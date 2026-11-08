use super::*;

#[test]
fn a_lost_intake_link_reopens_and_keeps_its_viewers_until_the_new_link_reports() {
    let remote = MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    };
    let mut run = remote_run(remote);
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    run.viewers(ANNA, 1);
    assert_eq!(
        run.on(Input::IntakeLost { uid: ANNA }),
        [Effect::ReopenIntake { uid: ANNA }]
    );
    assert_eq!(
        run.fire(Timer::Disconnected {
            id: "c2".into(),
            generation: 1
        }),
        [],
        "the client is still counted"
    );
    assert_eq!(run.on(Input::IntakeOpen { uid: ANNA }), []);
    assert_eq!(run.viewers(ANNA, 1), []);
    assert_eq!(
        run.on(Input::IntakeLost { uid: ANNA }),
        [Effect::ReopenIntake { uid: ANNA }],
        "a link lost after its reopen opens again"
    );
}

#[test]
fn a_reopened_link_lost_before_it_reports_gives_up() {
    let remote = MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    };
    let mut run = remote_run(remote);
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    run.viewers(ANNA, 1);
    run.on(Input::IntakeLost { uid: ANNA });
    assert_eq!(run.on(Input::IntakeOpen { uid: ANNA }), []);
    assert_eq!(
        run.on(Input::IntakeLost { uid: ANNA }),
        [disconnect_arm("c2", 2, 30), Effect::Inhibit(Inhibit::Idle)],
        "no second reopen"
    );
}

#[test]
fn a_reopen_that_gives_up_zeroes_its_viewers_and_the_next_login_opens_it() {
    let remote = MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    };
    let mut run = remote_run(remote);
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    run.viewers(ANNA, 1);
    run.on(Input::IntakeLost { uid: ANNA });
    assert_eq!(
        run.on(Input::IntakeLost { uid: ANNA }),
        [disconnect_arm("c2", 2, 30), Effect::Inhibit(Inhibit::Idle)]
    );
    assert_eq!(
        run.verify(8, 3, "anna", ANNA),
        [
            front(FrontEvent::Verified { conn: 8 }),
            to_worker(3, ToWorker::Cancel),
            disconnect_arm("c2", 3, 30),
            Effect::OpenIntake { uid: ANNA },
        ]
    );
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    assert_eq!(
        run.verify(8, 3, "anna", ANNA),
        [
            front(FrontEvent::Verified { conn: 8 }),
            to_worker(3, ToWorker::Cancel),
            front(FrontEvent::Accepted { conn: 8 }),
        ],
        "an open link takes the login at once"
    );
}

#[test]
fn a_link_lost_with_no_viewers_is_not_reopened() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    run.viewers(ANNA, 1);
    run.viewers(ANNA, 0);
    assert_eq!(run.on(Input::IntakeLost { uid: ANNA }), []);
    assert_eq!(
        run.verify(8, 3, "anna", ANNA),
        [
            front(FrontEvent::Verified { conn: 8 }),
            to_worker(3, ToWorker::Cancel),
            Effect::OpenIntake { uid: ANNA },
        ],
        "the next login opens it"
    );
}

#[test]
fn a_login_waiting_on_a_reopen_that_gives_up_is_refused() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    run.viewers(ANNA, 1);
    run.on(Input::IntakeLost { uid: ANNA });
    assert_eq!(
        run.verify(8, 3, "anna", ANNA),
        [
            front(FrontEvent::Verified { conn: 8 }),
            to_worker(3, ToWorker::Cancel),
            Effect::OpenIntake { uid: ANNA },
        ]
    );
    run.after(REFUSE_DELAY);
    let effects = run.on(Input::IntakeLost { uid: ANNA });
    assert!(
        !(effects.iter()).any(|e| matches!(e, Effect::Log(Level::Warn, _))),
        "{effects:?}"
    );
    assert!(
        effects.contains(&front(FrontEvent::Denied {
            conn: 8,
            backoff_s: 0
        })),
        "{effects:?}"
    );
    assert_eq!(run.fire(Timer::Handover(8)), [], "nothing left to deny");
}

#[test]
fn an_intake_open_for_a_uid_without_a_session_marks_nothing() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    run.removed("c2", Some("g1"));
    assert_eq!(run.on(Input::IntakeOpen { uid: ANNA }), []);
    run.anna_in_background(8, 3);
}

#[test]
fn a_failed_hand_over_opens_the_link_again_for_the_next_login() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    run.on(ready(7, "anna"));
    run.on(Input::Forwarded { conn: 7, ok: false });
    assert_eq!(
        run.verify(8, 3, "anna", ANNA),
        [
            front(FrontEvent::Verified { conn: 8 }),
            to_worker(3, ToWorker::Cancel),
            Effect::OpenIntake { uid: ANNA },
        ]
    );
}

#[test]
fn a_ready_for_a_login_whose_session_ended_is_denied_and_not_forwarded() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    run.removed("c2", Some("g1"));
    assert_eq!(
        quiet(run.on(ready(7, "anna"))),
        [
            front(FrontEvent::Denied {
                conn: 7,
                backoff_s: 0
            }),
            Effect::DropConn { conn: 7 },
        ]
    );
    assert_eq!(
        run.after(HANDOVER_DEADLINE).fire(Timer::Handover(7)),
        [],
        "nothing left to deny"
    );
}

#[test]
fn a_login_waiting_as_a_viewed_link_drops_hurries_its_reopen() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    run.viewers(ANNA, 1);
    run.on(ready(7, "anna"));
    run.on(Input::Forwarded { conn: 7, ok: false });
    run.verify(8, 3, "anna", ANNA);
    assert_eq!(
        run.on(Input::IntakeLost { uid: ANNA }),
        [
            Effect::ReopenIntake { uid: ANNA },
            Effect::OpenIntake { uid: ANNA },
        ]
    );
}

#[test]
fn a_link_closing_as_its_unviewed_session_ends_logs_and_arms_nothing() {
    let mut run = anna_unviewed_for_30();
    run.on(Input::IntakeOpen { uid: ANNA });
    run.viewers(ANNA, 0);
    let ended = run
        .after(Duration::from_secs(30 * 60))
        .fire(Timer::Disconnected {
            id: "c2".into(),
            generation: 1,
        });
    assert_eq!(quiet(ended), [Effect::TerminateSession("c2".into())]);
    assert_eq!(run.on(Input::IntakeLost { uid: ANNA }), []);
}

#[test]
fn a_lost_link_of_a_uid_with_no_session_is_not_reopened() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    run.removed("c2", Some("g1"));
    assert_eq!(run.on(Input::IntakeLost { uid: ANNA }), []);
}

#[test]
fn viewers_for_a_uid_with_no_session_are_ignored() {
    let mut run = remote_run(enabled());
    assert_eq!(run.viewers(ANNA, 1), []);
    run.anna_in_background(7, 2);
}

#[test]
fn recovery_opens_links_only_for_remote_group_members() {
    let mut run = Run::machine(config(enabled()));
    let found = vec![
        logind("c1", "anna", ANNA, 7, "user", "reprise-login"),
        logind("c3", "carl", CARL, 8, "user", "reprise-login"),
    ];
    assert_eq!(
        run.d.recover(found, Some("c1")),
        [link_look_up(ANNA, "anna"), link_look_up(CARL, "carl")]
    );
    assert_eq!(run.on(link_member(CARL, Some(CARL), false)), []);
    assert_eq!(
        run.on(link_member(ANNA, Some(ANNA), true)),
        [Effect::OpenIntake { uid: ANNA }]
    );
    assert_eq!(
        run.on(link_member(CARL, Some(BERT), true)),
        [],
        "the name now names another account"
    );
}

#[test]
fn a_link_look_up_that_fails_still_opens_the_link() {
    let mut run = Run::machine(config(enabled()));
    let found = vec![logind("c1", "anna", ANNA, 7, "user", "reprise-login")];
    run.d.recover(found, Some("c1"));
    assert_eq!(
        run.on(link_member(ANNA, None, false)),
        [Effect::OpenIntake { uid: ANNA }]
    );
}

#[test]
fn a_link_look_up_answered_after_its_session_ended_opens_nothing() {
    let mut run = Run::machine(config(enabled()));
    let found = vec![logind("c1", "anna", ANNA, 7, "user", "reprise-login")];
    run.d.recover(found, Some("g1"));
    run.removed("c1", Some("g1"));
    assert_eq!(run.on(link_member(ANNA, Some(ANNA), true)), []);
}
