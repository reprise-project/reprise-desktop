use super::*;

#[test]
fn a_tls_login_checks_the_group_then_pam_then_starts_a_background_session() {
    let mut run = remote_run(enabled());
    assert_eq!(run.on(tls(7, "anna", "pw")), looked_up(7, "anna"));
    assert_eq!(
        run.on(member(7, ANNA)),
        remote_spawn(2, Mode::Remote, "anna", 1)
    );
    assert_eq!(
        run.worker(2, secret("Password: ")),
        [to_worker(2, password("pw"))]
    );
    let mut want = vec![front(FrontEvent::Verified { conn: 7 })];
    want.extend(background_start(2, 7));
    assert_eq!(run.worker(2, authed("anna", ANNA)), want);
    assert_eq!(
        run.worker(2, started("c2", ANNA)),
        [
            Effect::OpenIntake { uid: ANNA },
            Effect::Inhibit(Inhibit::Idle)
        ]
    );
    assert_eq!(
        run.d.sessions.by_uid(ANNA),
        Some(&SessionEntry {
            id: "c2".into(),
            user: "anna".into(),
            uid: ANNA,
            vt: 7,
            started: Some(run.now),
            background: true,
            interface: None,
        })
    );
    assert_eq!(
        run.on(Input::IntakeOpen { uid: ANNA }),
        [front(FrontEvent::Accepted { conn: 7 })]
    );
    assert_eq!(
        run.on(ready(7, "anna")),
        [Effect::Forward { conn: 7, uid: ANNA }]
    );
    assert_eq!(run.on(Input::Forwarded { conn: 7, ok: true }), []);
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), []);
    assert_eq!(run.after(HANDOVER_DEADLINE).fire(Timer::Handover(7)), []);
}

#[test]
fn only_a_remote_start_runs_its_session_in_the_background() {
    let background = |effects: &[Effect]| {
        effects.iter().find_map(|e| match e {
            Effect::ToWorker {
                msg: ToWorker::Start { background, .. },
                ..
            } => Some(*background),
            _ => None,
        })
    };
    let mut run = remote_run(enabled());
    assert_eq!(run.greeter(create("bert")), spawn(2, Mode::Login, "bert"));
    run.worker(2, authed("bert", BERT));
    let console = run.greeter(GreeterRequest::StartSession);
    assert_eq!(background(&console), Some(false), "{console:?}");
    let remote = run.verify(7, 3, "anna", ANNA);
    assert_eq!(background(&remote), Some(true), "{remote:?}");
}

#[test]
fn a_background_start_never_picks_the_vt_on_screen() {
    let mut run = remote_run(enabled());
    run.world.active_vt = 7;
    run.on(tls(7, "anna", "pw"));
    run.on(member(7, ANNA));
    run.worker(2, secret("Password: "));
    let mut want = vec![front(FrontEvent::Verified { conn: 7 })];
    want.extend(background_start(2, 8));
    assert_eq!(run.worker(2, authed("anna", ANNA)), want);
}

#[test]
fn a_second_prompt_from_pam_fails_the_remote_login() {
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "pw"));
    run.on(member(7, ANNA));
    run.worker(2, secret("Password: "));
    let effects = run.worker(2, secret("New password: "));
    assert_eq!(quiet(effects), [Effect::Kill(WorkerId(2))]);
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), denied(7, 0));
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "pw"));
    run.on(member(7, ANNA));
    let visible = FromWorker::Prompt {
        kind: AuthKind::Visible,
        text: "login: ".into(),
    };
    assert_eq!(quiet(run.worker(2, visible)), [Effect::Kill(WorkerId(2))]);
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), denied(7, 0));
}

#[test]
fn a_login_for_a_user_with_a_session_hands_over_to_it_without_starting_one() {
    let mut run = anna_recovered(enabled(), "g1", Mode::Login.service());
    let effects = run.verify(7, 1, "anna", ANNA);
    assert_eq!(
        effects,
        [
            front(FrontEvent::Verified { conn: 7 }),
            to_worker(1, ToWorker::Cancel),
            Effect::OpenIntake { uid: ANNA },
        ]
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

fn authed_expired(user: &str, uid: u32) -> FromWorker {
    FromWorker::Authenticated {
        user: user.into(),
        uid,
        expired: true,
    }
}

#[test]
fn an_expired_password_reconnects_to_the_running_session() {
    let mut run = anna_recovered(enabled(), "g1", Mode::Login.service());
    run.on(tls(7, "anna", "pw"));
    run.on(member(7, ANNA));
    run.worker(1, secret("Password: "));
    assert_eq!(
        run.worker(1, authed_expired("anna", ANNA)),
        [
            front(FrontEvent::Verified { conn: 7 }),
            to_worker(1, ToWorker::Cancel),
            Effect::OpenIntake { uid: ANNA },
        ]
    );
}

#[test]
fn an_expired_password_starts_no_session_and_counts_as_a_failure() {
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "pw"));
    run.on(member(7, ANNA));
    run.worker(2, secret("Password: "));
    let effects = run.worker(2, authed_expired("anna", ANNA));
    assert_eq!(quiet(effects), [Effect::Kill(WorkerId(2))]);
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), denied(7, 0));
    for conn in 8..=10 {
        let w = conn as u32 - 5;
        run.on(tls(conn, "anna", "pw"));
        run.on(member(conn, ANNA));
        run.worker(w, secret("Password: "));
        run.worker(w, authed_expired("anna", ANNA));
    }
    assert_eq!(
        quiet(run.on(tls(11, "anna", "pw"))),
        armed(11),
        "the fifth login after four refusals is refused without a look-up"
    );
}

#[test]
fn a_login_for_the_console_users_session_hands_over_to_it() {
    let mut run = Run::machine(config(enabled()));
    let found = vec![logind("c1", "anna", ANNA, 7, "user", "reprise-login")];
    assert_eq!(
        run.d.recover(found, Some("c1")),
        [],
        "the session on the console is neither locked, a background one nor linked"
    );
    let mut effects = run.verify(7, 1, "anna", ANNA);
    assert_eq!(
        effects,
        [
            front(FrontEvent::Verified { conn: 7 }),
            to_worker(1, ToWorker::Cancel),
            Effect::OpenIntake { uid: ANNA },
        ]
    );
    effects.extend(run.on(Input::IntakeOpen { uid: ANNA }));
    effects.extend(run.on(ready(7, "anna")));
    assert_eq!(
        effects[3..],
        [
            front(FrontEvent::Accepted { conn: 7 }),
            Effect::Forward { conn: 7, uid: ANNA },
        ]
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::UnlockSession(_) | Effect::SwitchTo(_))),
        "{effects:?}"
    );
}

#[test]
fn two_logins_for_one_user_while_their_session_starts_share_it() {
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "pw"));
    run.on(tls_from(8, 2, "anna", "pw"));
    run.on(member(7, ANNA));
    assert_eq!(
        run.on(member(8, ANNA)),
        remote_spawn(3, Mode::Remote, "anna", 2)
    );
    run.worker(2, secret("Password: "));
    run.worker(3, secret("Password: "));
    let mut want = vec![front(FrontEvent::Verified { conn: 7 })];
    want.extend(background_start(2, 7));
    assert_eq!(run.worker(2, authed("anna", ANNA)), want);
    assert_eq!(
        run.worker(3, authed("anna", ANNA)),
        [
            front(FrontEvent::Verified { conn: 8 }),
            to_worker(3, ToWorker::Cancel)
        ],
        "the second login waits for the first one's start"
    );
    assert_eq!(
        run.worker(2, started("c2", ANNA)),
        [
            Effect::OpenIntake { uid: ANNA },
            Effect::Inhibit(Inhibit::Idle)
        ]
    );
    assert_eq!(
        run.on(Input::IntakeOpen { uid: ANNA }),
        [
            front(FrontEvent::Accepted { conn: 7 }),
            front(FrontEvent::Accepted { conn: 8 })
        ]
    );
    assert_eq!(
        run.on(ready(8, "anna")),
        [Effect::Forward { conn: 8, uid: ANNA }]
    );
    assert_eq!(
        run.on(ready(7, "anna")),
        [Effect::Forward { conn: 7, uid: ANNA }]
    );
}

#[test]
fn every_refusal_waits_the_refuse_delay() {
    let not_member = Input::Member {
        key: LookUpKey::Conn(7),
        uid: Some(ANNA),
        member: false,
        failed: false,
    };
    let unknown = Input::Member {
        key: LookUpKey::Conn(7),
        uid: None,
        member: false,
        failed: false,
    };
    let root = Input::Member {
        key: LookUpKey::Conn(7),
        uid: Some(0),
        member: true,
        failed: false,
    };
    for answer in [not_member, unknown, root] {
        let mut run = remote_run(enabled());
        assert_eq!(run.on(tls(7, "anna", "pw")), looked_up(7, "anna"));
        assert_eq!(quiet(run.on(answer)), []);
        assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), denied(7, 0));
    }
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "wrong"));
    run.on(member(7, ANNA));
    run.worker(2, secret("Password: "));
    run.after(Duration::from_secs(1));
    let refused = run.worker(2, failed(ErrorKind::Auth, AUTH_FAILED));
    assert_eq!(quiet(refused), [], "no Denied before REFUSE_DELAY");
    assert_eq!(
        run.after(REFUSE_DELAY - Duration::from_secs(1))
            .fire(Timer::Refuse(7)),
        denied(7, 0)
    );
}

#[test]
fn a_refusal_decided_after_the_refuse_delay_goes_out_at_once_and_is_logged_as_slow() {
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "wrong"));
    run.on(member(7, ANNA));
    run.worker(2, secret("Password: "));
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), []);
    let effects = run
        .after(Duration::from_secs(1))
        .worker(2, failed(ErrorKind::Auth, AUTH_FAILED));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Log(Level::Warn, text) if text.contains("slow"))),
        "{effects:?}"
    );
    assert_eq!(quiet(effects), denied(7, 0));
}

#[test]
fn tls_failures_back_off_the_address_and_a_pause_stops_every_tls_login() {
    let mut run = remote_run(enabled());
    let not_member = |conn| Input::Member {
        key: LookUpKey::Conn(conn),
        uid: Some(ANNA),
        member: false,
        failed: false,
    };
    for conn in 1..=4 {
        assert_eq!(run.on(tls(conn, "anna", "pw")), looked_up(conn, "anna"));
        run.on(not_member(conn));
    }
    assert_eq!(
        quiet(run.on(tls(5, "anna", "pw"))),
        armed(5),
        "the fifth login after four failures is refused without a look-up"
    );
    match run.after(REFUSE_DELAY).fire(Timer::Refuse(5)).as_slice() {
        [Effect::ToFront(FrontEvent::Denied { conn: 5, backoff_s })] => assert!(*backoff_s > 0),
        other => panic!("{other:?}"),
    }

    let mut run = remote_run(enabled());
    for n in 1..=PAUSE_AFTER as u8 {
        let conn = u64::from(n);
        assert_eq!(
            run.on(tls_from(conn, n, "anna", "pw")),
            looked_up(conn, "anna")
        );
        run.on(not_member(conn));
        run.after(Duration::from_secs(30));
    }
    let effects = run.on(tls_from(11, 11, "anna", "pw"));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Log(Level::Warn, text) if text.contains("paused"))),
        "{effects:?}"
    );
    assert_eq!(
        quiet(effects),
        armed(11),
        "a pause refuses an address that never failed"
    );
    assert_eq!(
        run.after(REFUSE_DELAY).fire(Timer::Refuse(11)),
        denied(11, 0)
    );
}

#[test]
fn tls_and_nla_logins_back_off_and_pause_each_on_their_own_failures() {
    let not_member = |conn| Input::Member {
        key: LookUpKey::Conn(conn),
        uid: Some(ANNA),
        member: false,
        failed: false,
    };
    type Login = fn(u64, u8, &str, &str) -> Input;
    let kinds: [(&str, Login, Login); 2] =
        [("TLS", tls_from, nla_from), ("NLA", nla_from, tls_from)];
    for (kind, failing, other) in kinds {
        let mut run = remote_run(enabled());
        for conn in 1..=4 {
            assert_eq!(
                run.on(failing(conn, 1, "anna", "pw")),
                looked_up(conn, "anna")
            );
            run.on(not_member(conn));
        }
        assert_eq!(
            quiet(run.on(failing(5, 1, "anna", "pw"))),
            armed(5),
            "{kind} failures back the address off for {kind}"
        );
        assert_eq!(
            run.on(other(6, 1, "anna", "pw")),
            looked_up(6, "anna"),
            "{kind} failures leave the address's other kind of login alone"
        );

        let mut run = remote_run(enabled());
        for n in 1..=PAUSE_AFTER as u8 {
            let conn = u64::from(n);
            assert_eq!(
                run.on(failing(conn, n, "anna", "pw")),
                looked_up(conn, "anna")
            );
            run.on(not_member(conn));
            run.after(Duration::from_secs(30));
        }
        assert_eq!(
            quiet(run.on(failing(11, 11, "anna", "pw"))),
            armed(11),
            "{kind} failures pause {kind}"
        );
        assert_eq!(
            run.on(other(12, 12, "anna", "pw")),
            looked_up(12, "anna"),
            "a {kind} pause leaves the other kind of login alone"
        );
    }
}

#[test]
fn max_sessions_refuses_a_new_session_but_not_an_existing_one() {
    let remote = MachineRemote {
        max_sessions: 1,
        ..enabled()
    };
    let mut run = anna_recovered(remote, "c1", Mode::Login.service());
    let effects = run.verify(7, 1, "carl", CARL);
    assert_eq!(
        quiet(effects),
        [
            front(FrontEvent::Verified { conn: 7 }),
            to_worker(1, ToWorker::Cancel)
        ]
    );
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), denied(7, 0));
    assert_eq!(
        run.verify(8, 2, "anna", ANNA),
        [
            front(FrontEvent::Verified { conn: 8 }),
            to_worker(2, ToWorker::Cancel),
            Effect::OpenIntake { uid: ANNA },
        ]
    );
}

#[test]
fn a_hand_over_not_taken_by_the_deadline_is_denied_and_the_session_kept() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    let effects = run.after(HANDOVER_DEADLINE).fire(Timer::Handover(7));
    assert_eq!(
        quiet(effects),
        [
            front(FrontEvent::Denied {
                conn: 7,
                backoff_s: 0
            }),
            Effect::DropConn { conn: 7 },
        ]
    );
    assert!(run.d.sessions.by_id("c2").is_some(), "the session stays");
    assert_eq!(run.on(Input::IntakeOpen { uid: ANNA }), []);
    assert_eq!(
        run.on(ready(7, "anna")).last(),
        Some(&Effect::DropConn { conn: 7 })
    );
}

#[test]
fn a_login_whose_authenticate_comes_late_still_gets_the_whole_hand_over_deadline() {
    let mut run = remote_run(enabled());
    run.after(Duration::from_secs(20));
    assert_eq!(run.on(tls(7, "anna", "pw")), looked_up(7, "anna"));
    let early = HANDOVER_DEADLINE - Duration::from_millis(100);
    assert_eq!(
        run.after(early).fire(Timer::Handover(7)),
        [Effect::Arm {
            timer: Timer::Handover(7),
            after: Duration::from_millis(100),
        }]
    );
}

#[test]
fn nla_verifies_the_delegated_password_against_the_stored_hash() {
    let mut run = remote_run(enabled());
    assert_eq!(
        run.d
            .set_remote_passwords(vec![stored("anna", ANNA, "Remote1!")]),
        []
    );
    assert_eq!(run.on(nla(7, "anna", "Remote1!")), looked_up(7, "anna"));
    assert_eq!(
        run.on(member(7, ANNA)),
        remote_spawn(2, Mode::RemoteNla, "anna", 1),
        "a match spawns the account check, and no answer goes to it"
    );
    assert_eq!(run.on(nla(8, "anna", "remote1!")), looked_up(8, "anna"));
    assert_eq!(quiet(run.on(member(8, ANNA))), [], "a mismatch");
    assert_eq!(run.on(nla(9, "carl", "Remote1!")), looked_up(9, "carl"));
    assert_eq!(quiet(run.on(member(9, CARL))), [], "no stored hash");
    run.after(REFUSE_DELAY);
    assert_eq!(run.fire(Timer::Refuse(8)), denied(8, 0));
    assert_eq!(run.fire(Timer::Refuse(9)), denied(9, 0));
    let mut want = vec![front(FrontEvent::Verified { conn: 7 })];
    want.extend(background_start(2, 7));
    assert_eq!(run.worker(2, authed("anna", ANNA)), want);
}

#[test]
fn a_delegated_password_stored_under_another_name_for_the_uid_is_refused() {
    let mut run = remote_run(enabled());
    run.d
        .set_remote_passwords(vec![stored("alice", ANNA, "Remote1!")]);
    assert_eq!(run.on(nla(7, "anna", "Remote1!")), looked_up(7, "anna"));
    assert_eq!(quiet(run.on(member(7, ANNA))), []);
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), denied(7, 0));
}

#[test]
fn nla_into_a_running_session_runs_the_account_check_first() {
    let mut run = anna_recovered(enabled(), "c1", Mode::Login.service());
    run.d
        .set_remote_passwords(vec![stored("anna", ANNA, "Remote1!")]);
    run.on(nla(7, "anna", "Remote1!"));
    assert_eq!(
        run.on(member(7, ANNA)),
        remote_spawn(1, Mode::RemoteNla, "anna", 1)
    );
    assert_eq!(
        run.worker(1, authed("anna", ANNA)),
        [
            front(FrontEvent::Verified { conn: 7 }),
            to_worker(1, ToWorker::Cancel),
            Effect::OpenIntake { uid: ANNA },
        ]
    );
    run.on(nla(8, "anna", "Remote1!"));
    run.on(member(8, ANNA));
    let expired = run.worker(2, failed(ErrorKind::Account, "the account has expired"));
    assert_eq!(quiet(expired), [], "a refused account is never verified");
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(8)), denied(8, 0));
}

#[test]
fn remote_logins_while_disabled_are_denied() {
    let mut run = remote_run(MachineRemote::default());
    assert_eq!(quiet(run.on(tls(7, "anna", "pw"))), armed(7));
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), denied(7, 0));
}

#[test]
fn more_than_max_remote_checks_at_once_are_denied() {
    let mut run = remote_run(enabled());
    for conn in 1..=MAX_REMOTE_CHECKS as u64 {
        assert_eq!(
            run.on(tls_from(conn, conn as u8, "anna", "pw")),
            looked_up(conn, "anna")
        );
    }
    assert_eq!(quiet(run.on(tls_from(9, 9, "anna", "pw"))), armed(9));
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(9)), denied(9, 0));
    run.on(member(1, ANNA));
    run.worker(2, secret("Password: "));
    run.worker(2, authed("anna", ANNA));
    assert_eq!(
        run.on(tls_from(10, 10, "anna", "pw")),
        looked_up(10, "anna"),
        "a verified login is no longer a check"
    );
}

#[test]
fn a_front_holding_max_logins_is_not_heard_until_one_ends() {
    let mut run = remote_run(enabled());
    for conn in 0..MAX_LOGINS as u64 {
        run.on(tls_from(conn, conn as u8, "Not A Name", "pw"));
    }
    assert_eq!(
        quiet(run.on(tls_from(900, 1, "anna", "pw"))),
        vec![],
        "a login past the cap is dropped unanswered"
    );
    run.after(REFUSE_DELAY).fire(Timer::Refuse(0));
    assert_eq!(quiet(run.on(tls_from(901, 1, "anna", "pw"))), armed(901));
}

#[test]
fn a_login_the_front_gave_up_holds_its_place_until_its_hand_over_deadline() {
    let mut run = remote_run(enabled());
    for conn in 0..MAX_LOGINS as u64 {
        run.on(tls_from(conn, 1, "anna", "pw"));
        run.gone(conn);
    }
    assert_eq!(
        quiet(run.on(tls_from(900, 1, "anna", "pw"))),
        [],
        "the logins the front gave up still count, and so do their timers"
    );
    run.after(REFUSE_DELAY);
    for conn in 0..MAX_LOGINS as u64 {
        assert_eq!(run.fire(Timer::Refuse(conn)), []);
    }
    run.after(HANDOVER_DEADLINE - REFUSE_DELAY);
    for conn in 0..MAX_LOGINS as u64 {
        assert_eq!(run.fire(Timer::Handover(conn)), []);
    }
    assert_eq!(
        quiet(run.on(tls_from(901, 1, "anna", "pw"))),
        looked_up(901, "anna")
    );
}

#[test]
fn a_login_for_the_conn_of_one_given_up_takes_its_place_and_timers() {
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "pw"));
    run.gone(7);
    let effects = run.after(Duration::from_secs(1)).on(tls(7, "anna", "pw"));
    assert_eq!(
        effects,
        [Effect::LookUp {
            key: LookUpKey::Conn(7),
            user: "anna".into(),
            group: "reprise-remote".into(),
        }]
    );
    assert_eq!(
        run.after(Duration::from_secs(7)).fire(Timer::Handover(7)),
        [Effect::Arm {
            timer: Timer::Handover(7),
            after: Duration::from_secs(1),
        }]
    );
}

#[test]
fn a_login_given_up_during_its_look_up_counts_among_the_checks_until_it_answers() {
    let mut run = remote_run(enabled());
    let looks_up =
        |effects: &[Effect]| (effects.iter()).any(|e| matches!(e, Effect::LookUp { .. }));
    for from in 1..=MAX_REMOTE_CHECKS as u8 {
        assert!(looks_up(&run.on(tls_from(7, from, "anna", "pw"))));
        run.gone(7);
    }
    assert!(
        !looks_up(&run.on(tls_from(7, 10, "anna", "pw"))),
        "the look-ups of the logins given up still run"
    );
    run.gone(7);
    run.on(Input::Member {
        key: LookUpKey::Conn(7),
        uid: Some(ANNA),
        member: true,
        failed: false,
    });
    assert!(looks_up(&run.on(tls_from(8, 11, "anna", "pw"))));
}

#[test]
fn a_login_taking_the_place_of_one_given_up_within_its_refuse_delay_is_denied_on_time() {
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "pw"));
    run.gone(7);
    assert_eq!(
        run.after(Duration::from_secs(2)).on(tls(7, "anna", "pw")),
        [Effect::LookUp {
            key: LookUpKey::Conn(7),
            user: "anna".into(),
            group: "reprise-remote".into(),
        }]
    );
    assert_eq!(
        run.after(Duration::from_secs(1)).fire(Timer::Refuse(7)),
        [Effect::Arm {
            timer: Timer::Refuse(7),
            after: Duration::from_secs(2),
        }],
        "the timer it took over waits on for the new login"
    );
    let not_member = Input::Member {
        key: LookUpKey::Conn(7),
        uid: Some(ANNA),
        member: false,
        failed: false,
    };
    assert_eq!(quiet(run.on(not_member)), []);
    let denied = quiet(run.after(Duration::from_secs(2)).fire(Timer::Refuse(7)));
    assert!(
        matches!(
            denied.as_slice(),
            [Effect::ToFront(FrontEvent::Denied { conn: 7, .. })]
        ),
        "{denied:?}"
    );
}

#[test]
fn a_login_taking_the_place_of_one_given_up_past_its_refuse_delay_is_denied_on_time() {
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "pw"));
    run.gone(7);
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), []);
    let refused = quiet(
        run.after(Duration::from_secs(1))
            .on(tls(7, "Not A Name", "pw")),
    );
    assert_eq!(
        refused,
        [Effect::Arm {
            timer: Timer::Refuse(7),
            after: REFUSE_DELAY,
        }]
    );
    let denied = quiet(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)));
    assert!(
        matches!(
            denied.as_slice(),
            [Effect::ToFront(FrontEvent::Denied { conn: 7, .. })]
        ),
        "{denied:?}"
    );
}

#[test]
fn a_refusal_before_any_check_is_logged_once_a_minute() {
    let logs = |effects: &[Effect]| {
        (effects.iter())
            .filter(|e| matches!(e, Effect::Log(_, text) if text.contains("was refused")))
            .count()
    };
    let mut run = remote_run(MachineRemote::default());
    assert_eq!(logs(&run.on(tls(1, "anna", "pw"))), 1);
    let mut held = 0;
    for conn in 2..50 {
        held += logs(
            &run.after(Duration::from_millis(100))
                .on(tls(conn, "anna", "pw")),
        );
    }
    assert_eq!(held, 0, "the rest of the minute is counted");
    let effects = run.after(Duration::from_secs(60)).on(tls(50, "anna", "pw"));
    assert!(
        (effects.iter()).any(|e| matches!(e, Effect::Log(_, text) if text.contains("48 more"))),
        "{effects:?}"
    );
}

#[test]
fn a_ready_for_another_user_is_dropped() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    assert_eq!(
        quiet(run.on(ready(7, "bert"))),
        [Effect::DropConn { conn: 7 }]
    );
    assert_eq!(
        quiet(run.on(ready(7, "anna"))),
        [Effect::DropConn { conn: 7 }]
    );
}

#[test]
fn a_ready_for_a_conn_never_accepted_is_dropped() {
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "pw"));
    assert_eq!(
        quiet(run.on(ready(7, "anna"))),
        [Effect::DropConn { conn: 7 }]
    );
    assert_eq!(quiet(run.on(member(7, ANNA))), [], "the login is forgotten");
    assert_eq!(
        quiet(run.on(ready(9, "anna"))),
        [Effect::DropConn { conn: 9 }]
    );
}

#[test]
fn a_user_name_no_account_can_have_is_refused_without_a_look_up() {
    let long = "a".repeat(33);
    for (conn, name) in [
        (1, "Anna"),
        (2, "1anna"),
        (3, "an na"),
        (4, ""),
        (5, long.as_str()),
    ] {
        let mut run = remote_run(enabled());
        assert_eq!(
            quiet(run.on(tls(conn, name, "pw"))),
            armed(conn),
            "{name:?}"
        );
        assert_eq!(
            run.after(REFUSE_DELAY).fire(Timer::Refuse(conn)),
            denied(conn, 0)
        );
    }
    let mut run = remote_run(enabled());
    let name = format!("_{}", "a.b-c_9".repeat(5))[..32].to_string();
    assert_eq!(run.on(tls(1, &name, "pw")), looked_up(1, &name));
}

#[test]
fn a_lost_front_kills_unstarted_checks_and_forgets_its_conns() {
    let mut run = remote_run(enabled());
    run.on(Input::FrontLinked);
    let mut want = vec![front(FrontEvent::Verified { conn: 7 })];
    want.extend(background_start(2, 7));
    assert_eq!(run.verify(7, 2, "anna", ANNA), want);
    run.on(tls_from(8, 2, "bert", "pw"));
    run.on(member(8, BERT));
    run.on(tls_from(9, 3, "carl", "pw"));
    assert_eq!(run.on(Input::FrontLost), [Effect::Kill(WorkerId(3))]);
    assert_eq!(
        run.worker(2, started("c2", ANNA)),
        [
            Effect::OpenIntake { uid: ANNA },
            Effect::Inhibit(Inhibit::Idle)
        ],
        "a start in flight goes on"
    );
    assert_eq!(run.on(Input::IntakeOpen { uid: ANNA }), []);
    assert_eq!(quiet(run.on(member(9, CARL))), []);
    run.after(HANDOVER_DEADLINE);
    for conn in [7, 8, 9] {
        assert_eq!(run.fire(Timer::Refuse(conn)), []);
        assert_eq!(run.fire(Timer::Handover(conn)), []);
    }
    run.on(Input::FrontLinked);
    assert_eq!(
        run.on(tls(7, "carl", "pw")),
        looked_up(7, "carl"),
        "a new front's conn 7 is a new login"
    );
}

#[test]
fn a_lost_front_drops_the_hand_overs_in_flight() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    assert_eq!(
        run.on(ready(7, "anna")),
        [Effect::Forward { conn: 7, uid: ANNA }]
    );
    assert_eq!(run.on(Input::FrontLost), [Effect::DropConn { conn: 7 }]);
    assert_eq!(
        run.on(Input::Forwarded { conn: 7, ok: true }),
        [],
        "a late result is forgotten"
    );
}

#[test]
fn a_gone_conn_kills_its_worker() {
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "pw"));
    run.on(member(7, ANNA));
    assert_eq!(
        run.on(Input::Front(FrontRequest::Gone { conn: 7 })),
        [Effect::Kill(WorkerId(2))]
    );
    assert_eq!(run.worker(2, secret("Password: ")), []);
    assert_eq!(run.exited(2), []);
    run.after(HANDOVER_DEADLINE);
    assert_eq!(run.fire(Timer::Refuse(7)), []);
    assert_eq!(run.fire(Timer::Handover(7)), []);
}

#[test]
fn a_front_that_links_while_remote_logins_are_off_is_stopped() {
    let mut run = remote_run(MachineRemote::default());
    assert_eq!(run.on(Input::FrontLinked), []);
    assert_eq!(
        run.on(Input::Front(FrontRequest::Hello)),
        [Effect::Front(FrontUnit::Stop)],
        "the unit restarts with reprise-login whatever the settings say"
    );
}

#[test]
fn every_refusal_from_one_address_carries_the_back_off_of_its_authenticate() {
    let backoff = |wrong_password: bool| {
        let mut run = remote_run(enabled());
        for conn in 1..=3 {
            run.on(tls(conn, "anna", "pw"));
            run.on(Input::Member {
                key: LookUpKey::Conn(conn),
                uid: Some(ANNA),
                member: false,
                failed: false,
            });
        }
        run.on(tls(4, "anna", "pw"));
        if wrong_password {
            run.on(member(4, ANNA));
            run.worker(2, secret("Password: "));
            run.after(Duration::from_millis(1500));
            run.worker(2, failed(ErrorKind::Auth, AUTH_FAILED));
            run.after(REFUSE_DELAY - Duration::from_millis(1500));
        } else {
            run.after(Duration::from_millis(50));
            run.on(Input::Member {
                key: LookUpKey::Conn(4),
                uid: Some(ANNA),
                member: false,
                failed: false,
            });
            run.after(REFUSE_DELAY - Duration::from_millis(50));
        }
        match run.fire(Timer::Refuse(4)).as_slice() {
            [Effect::ToFront(FrontEvent::Denied { conn: 4, backoff_s })] => *backoff_s,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(backoff(false), 2, "the fourth failure backs off 2 s");
    assert_eq!(backoff(true), 2, "a wrong password tells the same");
}

#[test]
fn pam_passing_a_tls_login_without_asking_its_password_refuses_it() {
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "pw"));
    run.on(member(7, ANNA));
    assert_eq!(
        quiet(run.worker(2, authed("anna", ANNA))),
        [Effect::Kill(WorkerId(2))]
    );
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), denied(7, 0));
}

#[test]
fn a_forward_never_confirmed_expires_at_the_hand_over_deadline() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    assert_eq!(
        run.on(ready(7, "anna")),
        [Effect::Forward { conn: 7, uid: ANNA }]
    );
    let effects = run.after(HANDOVER_DEADLINE).fire(Timer::Handover(7));
    assert_eq!(quiet(effects), [Effect::DropConn { conn: 7 }]);
    assert_eq!(run.on(Input::Forwarded { conn: 7, ok: true }), []);
    assert_eq!(
        run.on(tls(7, "anna", "pw")),
        looked_up(7, "anna"),
        "conn 7 is free again"
    );
}

#[test]
fn a_reused_conn_id_re_arms_the_earlier_conns_timers() {
    let mut run = remote_run(enabled());
    let not_member = || Input::Member {
        key: LookUpKey::Conn(7),
        uid: Some(ANNA),
        member: false,
        failed: false,
    };
    run.on(tls(7, "anna", "pw"));
    run.on(not_member());
    run.on(Input::FrontLost);
    run.on(Input::FrontLinked);
    run.after(Duration::from_secs(2));
    run.on(tls(7, "anna", "pw"));
    run.on(not_member());
    assert_eq!(
        run.after(Duration::from_secs(1)).fire(Timer::Refuse(7)),
        [Effect::Arm {
            timer: Timer::Refuse(7),
            after: REFUSE_DELAY - Duration::from_secs(1),
        }],
        "the first conn's Refuse at +3 s"
    );
    assert_eq!(
        run.after(Duration::from_secs(5)).fire(Timer::Handover(7)),
        [Effect::Arm {
            timer: Timer::Handover(7),
            after: Duration::from_secs(2),
        }],
        "the first conn's deadline at +8 s"
    );
}
