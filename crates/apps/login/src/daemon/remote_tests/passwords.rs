use super::*;

fn nla_user(p: &StoredPassword) -> NlaUser {
    NlaUser {
        user: p.user.clone(),
        nt_hash: p.nt_hash.clone(),
    }
}

fn nla_users(users: &[&StoredPassword]) -> Effect {
    front(FrontEvent::Passwords {
        users: users.iter().map(|p| nla_user(p)).collect(),
    })
}

fn listed_look_up(p: &StoredPassword, generation: u32) -> Effect {
    Effect::LookUp {
        key: LookUpKey::Listed {
            uid: p.uid,
            generation,
        },
        user: p.user.clone(),
        group: "reprise-remote".into(),
    }
}

fn listed(p: &StoredPassword, generation: u32, found: Option<u32>, member: bool) -> Input {
    Input::Member {
        key: LookUpKey::Listed {
            uid: p.uid,
            generation,
        },
        uid: found,
        member,
        failed: false,
    }
}

fn listed_failed(p: &StoredPassword, generation: u32) -> Input {
    Input::Member {
        key: LookUpKey::Listed {
            uid: p.uid,
            generation,
        },
        uid: None,
        member: false,
        failed: true,
    }
}

fn relist_arm(secs: u64) -> Effect {
    Effect::Arm {
        timer: Timer::Relist,
        after: Duration::from_secs(secs),
    }
}

#[test]
fn the_front_gets_only_remote_group_members_passwords() {
    let mut run = remote_run(enabled());
    let anna = stored("anna", ANNA, "Remote1!");
    let bert = stored("bert", BERT, "Another1!");
    let root = stored("root", 0, "Rooted1!");
    let all = vec![anna.clone(), bert.clone(), root.clone()];
    assert_eq!(run.d.set_remote_passwords(all.clone()), []);
    assert_eq!(run.on(Input::FrontLinked), []);
    assert_eq!(
        run.on(Input::Front(FrontRequest::Hello)),
        [
            nla_users(&[]),
            listed_look_up(&anna, 1),
            listed_look_up(&bert, 1)
        ],
        "root is never looked up"
    );
    let added = run.on(listed(&anna, 1, Some(ANNA), true));
    assert_eq!(
        logs(&added),
        ["anna is in reprise-remote; the front takes their Remote Desktop password"]
    );
    assert_eq!(quiet(added), [nla_users(&[&anna])]);
    assert_eq!(run.on(listed(&root, 1, Some(0), true)), [], "root never");
    assert_eq!(quiet(run.on(listed(&bert, 1, Some(BERT), false))), []);
    assert_eq!(
        run.d.set_remote_passwords(all),
        [nla_users(&[&anna])],
        "a store sends the members' only"
    );
    let shown = format!("{anna:?}");
    assert!(!shown.contains(&anna.nt_hash[..8]), "{shown}");
}

#[test]
fn a_password_whose_name_has_another_uid_is_dropped_from_the_store_at_once() {
    let mut run = remote_run(enabled());
    let anna = stored("anna", ANNA, "Remote1!");
    let bert = stored("bert", BERT, "Another1!");
    run.d.set_remote_passwords(vec![anna.clone(), bert.clone()]);
    run.on(Input::FrontLinked);
    run.on(Input::Front(FrontRequest::Hello));
    quiet(run.on(listed(&anna, 1, Some(ANNA), true)));
    assert_eq!(
        quiet(run.on(listed(&bert, 1, Some(CARL), true))),
        [Effect::StorePasswords(vec![anna.clone()])],
        "the name is another uid's now"
    );
    assert_eq!(run.on(Input::Stored { ok: true }), []);
    assert_eq!(run.d.remote.passwords, vec![anna.clone()]);
}

/// Logs in for anna and bert, with bert's account not found in round 1.
fn bert_missing_once() -> (Run, StoredPassword, StoredPassword) {
    let mut run = remote_run(enabled());
    let anna = stored("anna", ANNA, "Remote1!");
    let bert = stored("bert", BERT, "Another1!");
    run.d.set_remote_passwords(vec![anna.clone(), bert.clone()]);
    run.on(Input::FrontLinked);
    run.on(Input::Front(FrontRequest::Hello));
    quiet(run.on(listed(&anna, 1, Some(ANNA), true)));
    assert_eq!(
        quiet(run.on(listed(&bert, 1, None, false))),
        [Effect::Arm {
            timer: Timer::GoneCheck,
            after: GONE_CHECK,
        }],
        "one round that misses the account writes nothing"
    );
    assert_eq!(run.d.remote.passwords, [anna.clone(), bert.clone()]);
    (run, anna, bert)
}

#[test]
fn a_password_whose_account_is_gone_is_dropped_when_a_later_round_misses_it_again() {
    let (mut run, anna, bert) = bert_missing_once();
    assert_eq!(
        run.after(GONE_CHECK).fire(Timer::GoneCheck),
        [listed_look_up(&anna, 2), listed_look_up(&bert, 2)]
    );
    quiet(run.on(listed(&anna, 2, Some(ANNA), true)));
    assert_eq!(
        quiet(run.on(listed(&bert, 2, None, false))),
        [Effect::StorePasswords(vec![anna.clone()])]
    );
    assert_eq!(run.on(Input::Stored { ok: true }), []);
    assert_eq!(run.d.remote.passwords, [anna]);
}

#[test]
fn a_missing_account_found_again_keeps_its_password() {
    let (mut run, anna, bert) = bert_missing_once();
    run.on(Input::ConfigReloaded(config(enabled())));
    quiet(run.on(listed(&anna, 2, Some(ANNA), true)));
    quiet(run.on(listed(&bert, 2, Some(BERT), true)));
    run.after(GONE_CHECK);
    run.on(Input::ConfigReloaded(config(enabled())));
    quiet(run.on(listed(&anna, 3, Some(ANNA), true)));
    let missed = quiet(run.on(listed(&bert, 3, None, false)));
    assert!(
        !missed
            .iter()
            .any(|e| matches!(e, Effect::StorePasswords(_))),
        "a round that found the account starts the wait again: {missed:?}"
    );
    assert_eq!(run.d.remote.passwords, [anna, bert]);
}

#[test]
fn a_missing_account_is_not_dropped_by_a_round_too_soon_or_with_a_failed_look_up() {
    let (mut run, anna, bert) = bert_missing_once();
    run.on(Input::ConfigReloaded(config(enabled())));
    quiet(run.on(listed(&anna, 2, Some(ANNA), true)));
    assert_eq!(quiet(run.on(listed(&bert, 2, None, false))), [], "too soon");
    run.after(GONE_CHECK).fire(Timer::GoneCheck);
    quiet(run.on(listed_failed(&anna, 3)));
    let missed = quiet(run.on(listed(&bert, 3, None, false)));
    assert!(
        !missed
            .iter()
            .any(|e| matches!(e, Effect::StorePasswords(_))),
        "a round with a failed look-up proves nothing: {missed:?}"
    );
    assert_eq!(run.d.remote.passwords, [anna, bert]);
}

#[test]
fn a_password_dropped_while_a_change_is_stored_stays_dropped() {
    let mut run = remote_run(enabled());
    let anna = stored("anna", ANNA, "Remote1!");
    let bert = stored("bert", BERT, "Another1!");
    run.d.set_remote_passwords(vec![anna.clone(), bert.clone()]);
    run.on(Input::FrontLinked);
    run.on(Input::Front(FrontRequest::Hello));
    let new = stored("anna", ANNA, "Changed1!");
    run.on(Input::Account {
        link: 7,
        uid: ANNA,
        user: "anna".into(),
        request: repriselogin::account::AccountRequest::SetRemotePassword {
            login_password: Zeroizing::new("login".into()),
            remote_password: Zeroizing::new("Changed1!".into()),
        },
    });
    run.on(Input::Member {
        key: LookUpKey::Account(7),
        uid: Some(ANNA),
        member: true,
        failed: false,
    });
    run.worker(2, secret("Password: "));
    assert_eq!(
        quiet(run.worker(2, authed("anna", ANNA))),
        [Effect::StorePasswords(vec![bert.clone(), new.clone()])]
    );
    assert_eq!(
        quiet(run.on(listed(&bert, 1, Some(CARL), true))),
        [],
        "one write at a time"
    );
    let effects = quiet(run.on(Input::Stored { ok: true }));
    assert_eq!(
        effects.last(),
        Some(&Effect::StorePasswords(vec![new.clone()])),
        "{effects:?}"
    );
    assert_eq!(run.d.remote.passwords, [new]);
}

#[test]
fn the_passwords_dropped_at_start_are_written_back() {
    let mut run = remote_run(enabled());
    let anna = stored("anna", ANNA, "Remote1!");
    run.d.set_remote_passwords(vec![anna.clone()]);
    assert_eq!(
        run.d.rewrite_passwords(),
        [Effect::StorePasswords(vec![anna])]
    );
}

#[test]
fn a_user_put_back_in_the_remote_group_gets_their_password_to_the_front_again() {
    let mut run = remote_run(enabled());
    let anna = stored("anna", ANNA, "Remote1!");
    run.d.set_remote_passwords(vec![anna.clone()]);
    run.on(Input::FrontLinked);
    run.on(Input::Front(FrontRequest::Hello));
    assert_eq!(
        quiet(run.on(listed(&anna, 1, Some(ANNA), true))),
        [nla_users(&[&anna])]
    );

    assert_eq!(
        run.on(Input::ConfigReloaded(config(enabled()))),
        [
            Effect::Front(FrontUnit::Start { port: 3389 }),
            listed_look_up(&anna, 2)
        ],
        "a reload reads the group again"
    );
    let dropped = run.on(listed(&anna, 2, Some(ANNA), false));
    assert_eq!(
        logs(&dropped),
        ["anna is not in reprise-remote; the front leaves out their Remote Desktop password"]
    );
    assert_eq!(quiet(dropped), [nla_users(&[])], "taken out of the group");
    assert_eq!(run.on(nla(7, "anna", "Remote1!")), looked_up(7, "anna"));
    assert_eq!(
        quiet(run.on(Input::Member {
            key: LookUpKey::Conn(7),
            uid: Some(ANNA),
            member: false,
            failed: false,
        })),
        [],
        "a login that agrees with the list looks nobody up"
    );

    assert_eq!(run.on(tls(8, "anna", "pw")), looked_up(8, "anna"));
    let mut want = vec![listed_look_up(&anna, 3)];
    want.extend(remote_spawn(2, Mode::Remote, "anna", 1));
    assert_eq!(
        run.on(member(8, ANNA)),
        want,
        "a login finding a member the list leaves out reads the group again"
    );
    assert_eq!(
        run.on(listed(&anna, 2, Some(ANNA), false)),
        [],
        "a stale answer"
    );
    assert_eq!(
        quiet(run.on(listed(&anna, 3, Some(ANNA), true))),
        [nla_users(&[&anna])]
    );
}

#[test]
fn a_user_put_back_in_the_remote_group_reaches_the_front_when_the_group_file_is_written() {
    let mut run = remote_run(enabled());
    let anna = stored("anna", ANNA, "Remote1!");
    run.d.set_remote_passwords(vec![anna.clone()]);
    run.on(Input::FrontLinked);
    run.on(Input::Front(FrontRequest::Hello));
    assert_eq!(quiet(run.on(listed(&anna, 1, Some(ANNA), false))), []);
    assert_eq!(
        run.on(Input::AccountsChanged),
        [listed_look_up(&anna, 2)],
        "a written group file reads the group again"
    );
    let added = run.on(listed(&anna, 2, Some(ANNA), true));
    assert_eq!(
        logs(&added),
        ["anna is in reprise-remote; the front takes their Remote Desktop password"]
    );
    assert_eq!(quiet(added), [nla_users(&[&anna])]);
}

#[test]
fn a_user_taken_out_of_the_remote_group_leaves_the_front_when_the_group_file_is_written() {
    let mut run = remote_run(enabled());
    let anna = stored("anna", ANNA, "Remote1!");
    run.d.set_remote_passwords(vec![anna.clone()]);
    run.on(Input::FrontLinked);
    run.on(Input::Front(FrontRequest::Hello));
    assert_eq!(
        quiet(run.on(listed(&anna, 1, Some(ANNA), true))),
        [nla_users(&[&anna])]
    );
    assert_eq!(run.on(Input::AccountsChanged), [listed_look_up(&anna, 2)]);
    assert_eq!(
        quiet(run.on(listed(&anna, 2, Some(ANNA), false))),
        [nla_users(&[])]
    );
    assert_eq!(run.on(tls(7, "anna", "pw")), looked_up(7, "anna"));
    assert_eq!(
        quiet(run.on(Input::Member {
            key: LookUpKey::Conn(7),
            uid: Some(ANNA),
            member: false,
            failed: false,
        })),
        [],
        "the login is refused and agrees with the list"
    );
}

#[test]
fn a_session_look_up_that_disagrees_with_the_front_s_list_reads_the_group_again_after_the_round() {
    let mut run = anna_recovered(enabled(), "c1", Mode::Login.service());
    let anna = stored("anna", ANNA, "Remote1!");
    run.d.set_remote_passwords(vec![anna.clone()]);
    run.on(Input::FrontLinked);
    run.on(Input::Front(FrontRequest::Hello));
    assert_eq!(
        run.on(link_member(ANNA, Some(ANNA), true)),
        [Effect::OpenIntake { uid: ANNA }],
        "round 1 is in flight"
    );
    assert_eq!(
        quiet(run.on(listed(&anna, 1, Some(ANNA), true))),
        [nla_users(&[&anna]), listed_look_up(&anna, 2)]
    );
    assert_eq!(quiet(run.on(listed(&anna, 2, Some(ANNA), true))), []);
}

/// Rounds of group look-ups `effects` start, by generation.
fn rounds(effects: &[Effect]) -> BTreeSet<u32> {
    (effects.iter())
        .filter_map(|e| match e {
            Effect::LookUp {
                key: LookUpKey::Listed { generation, .. },
                ..
            } => Some(*generation),
            _ => None,
        })
        .collect()
}

#[test]
fn a_thousand_hellos_on_one_link_start_one_round_of_look_ups() {
    let mut run = remote_run(enabled());
    let anna = stored("anna", ANNA, "Remote1!");
    let bert = stored("bert", BERT, "Another1!");
    run.d.set_remote_passwords(vec![anna.clone(), bert.clone()]);
    run.on(Input::FrontLinked);
    let mut effects = Vec::new();
    for _ in 0..1000 {
        effects.extend(run.on(Input::Front(FrontRequest::Hello)));
    }
    assert!(rounds(&effects).len() <= 2, "{:?}", rounds(&effects));
    let look_ups = (effects.iter()).filter(|e| matches!(e, Effect::LookUp { .. }));
    assert_eq!(look_ups.count(), 2, "one look-up per stored user");
    let answers = (effects.iter()).filter(|e| **e == nla_users(&[]));
    assert_eq!(answers.count(), 1000, "every Hello hears the list");

    run.on(Input::FrontLinked);
    assert_eq!(
        run.on(Input::Front(FrontRequest::Hello)),
        [nla_users(&[])],
        "a new link's Hello waits for the round in flight"
    );
    run.on(listed(&anna, 1, Some(ANNA), false));
    assert_eq!(
        run.on(listed(&bert, 1, Some(BERT), false)),
        [listed_look_up(&anna, 2), listed_look_up(&bert, 2)]
    );
}

#[test]
fn requests_during_a_round_run_one_more_round_after_it() {
    let mut run = remote_run(enabled());
    let anna = stored("anna", ANNA, "Remote1!");
    let bert = stored("bert", BERT, "Another1!");
    run.d.set_remote_passwords(vec![anna.clone(), bert.clone()]);
    run.on(Input::FrontLinked);
    run.on(Input::Front(FrontRequest::Hello));
    for _ in 0..3 {
        assert_eq!(
            run.on(Input::ConfigReloaded(config(enabled()))),
            [Effect::Front(FrontUnit::Start { port: 3389 })]
        );
    }
    assert_eq!(
        quiet(run.on(listed(&anna, 1, Some(ANNA), true))),
        [nla_users(&[&anna])]
    );
    assert_eq!(
        quiet(run.on(listed(&bert, 1, Some(BERT), false))),
        [listed_look_up(&anna, 2), listed_look_up(&bert, 2)]
    );
    run.on(listed(&anna, 2, Some(ANNA), true));
    assert_eq!(
        run.on(listed(&bert, 2, Some(BERT), false)),
        [],
        "no third round"
    );
}

#[test]
fn a_reload_that_changes_the_group_drops_the_round_and_the_old_members_at_once() {
    let mut run = remote_run(enabled());
    let anna = stored("anna", ANNA, "Remote1!");
    run.d.set_remote_passwords(vec![anna.clone()]);
    run.on(Input::FrontLinked);
    run.on(Input::Front(FrontRequest::Hello));
    run.on(listed(&anna, 1, Some(ANNA), true));
    run.on(Input::ConfigReloaded(config(enabled())));
    let staff = MachineRemote {
        group: "staff".into(),
        ..enabled()
    };
    assert_eq!(
        run.on(Input::ConfigReloaded(config(staff))),
        [
            Effect::Front(FrontUnit::Start { port: 3389 }),
            nla_users(&[]),
            Effect::LookUp {
                key: LookUpKey::Listed {
                    uid: ANNA,
                    generation: 3,
                },
                user: "anna".into(),
                group: "staff".into(),
            }
        ]
    );
    assert_eq!(
        run.on(listed(&anna, 2, Some(ANNA), true)),
        [],
        "the old group's round is dropped"
    );
    assert_eq!(
        quiet(run.on(listed(&anna, 3, Some(ANNA), true))),
        [nla_users(&[&anna])]
    );
}

#[test]
fn a_member_whose_look_up_failed_is_listed_again_after_a_retry() {
    let mut run = remote_run(enabled());
    let anna = stored("anna", ANNA, "Remote1!");
    run.d.set_remote_passwords(vec![anna.clone()]);
    run.on(Input::FrontLinked);
    run.on(Input::Front(FrontRequest::Hello));
    run.on(listed(&anna, 1, Some(ANNA), true));
    run.on(Input::ConfigReloaded(config(enabled())));

    let failed = run.on(listed_failed(&anna, 2));
    assert_eq!(
        logs(&failed),
        ["anna's group look-up failed; the front leaves out their Remote Desktop password"]
    );
    assert_eq!(
        quiet(failed),
        [nla_users(&[]), relist_arm(30)],
        "fails closed and retries"
    );
    assert_eq!(run.fire(Timer::Relist), [listed_look_up(&anna, 3)]);
    assert_eq!(
        quiet(run.on(listed_failed(&anna, 3))),
        [relist_arm(60)],
        "backs off"
    );
    assert_eq!(run.fire(Timer::Relist), [listed_look_up(&anna, 4)]);
    assert_eq!(
        quiet(run.on(listed(&anna, 4, Some(ANNA), true))),
        [nla_users(&[&anna])]
    );

    run.on(Input::ConfigReloaded(config(enabled())));
    assert_eq!(
        quiet(run.on(listed(&anna, 5, None, false))),
        [
            nla_users(&[]),
            Effect::Arm {
                timer: Timer::GoneCheck,
                after: GONE_CHECK,
            }
        ],
        "a missing account is no failure, so no retry, only the round that may drop it"
    );
}

#[test]
fn a_failed_login_look_up_reads_nobody_again() {
    let mut run = remote_run(enabled());
    let anna = stored("anna", ANNA, "Remote1!");
    run.d.set_remote_passwords(vec![anna.clone()]);
    run.on(Input::FrontLinked);
    run.on(Input::Front(FrontRequest::Hello));
    run.on(listed(&anna, 1, Some(ANNA), true));
    assert_eq!(run.on(tls(7, "anna", "pw")), looked_up(7, "anna"));
    let refused = run.on(Input::Member {
        key: LookUpKey::Conn(7),
        uid: None,
        member: false,
        failed: true,
    });
    assert_eq!(rounds(&refused), BTreeSet::new(), "{refused:?}");
}
