use super::*;

#[test]
fn the_daemon_starts_the_front_on_its_port_or_frees_the_port_at_start() {
    let mut run = Run::machine(config(MachineRemote {
        port: 3390,
        ..enabled()
    }));
    run.recover(vec![]);
    let effects = run.d.start(run.now, &run.world);
    assert_eq!(
        effects[0],
        Effect::Front(FrontUnit::Start { port: 3390 }),
        "{effects:?}"
    );
    assert_eq!(effects[1..], greeter_spawn(1));
    let mut run = Run::machine(config(MachineRemote::default()));
    run.recover(vec![]);
    let effects = run.d.start(run.now, &run.world);
    assert_eq!(
        effects[0],
        Effect::Front(FrontUnit::Stop),
        "a port held since boot is freed while remote logins are off: {effects:?}"
    );
    assert_eq!(effects[1..], greeter_spawn(1));
}

#[test]
fn turning_remote_on_starts_the_front_and_links_no_console_session_without_a_client() {
    let mut run = anna_recovered(MachineRemote::default(), "c1", Mode::Login.service());
    assert_eq!(
        run.on(Input::ConfigReloaded(config(enabled()))),
        [Effect::Front(FrontUnit::Start { port: 3389 })]
    );
}

#[test]
fn a_reload_while_remote_logins_stay_off_frees_the_port() {
    let mut run = anna_recovered(MachineRemote::default(), "c1", Mode::Login.service());
    assert_eq!(
        run.on(Input::ConfigReloaded(config(MachineRemote::default()))),
        [Effect::Front(FrontUnit::Stop)],
        "a socket started by hand is stopped"
    );
}

#[test]
fn a_reload_while_remote_logins_stay_on_starts_the_front_again() {
    let mut run = anna_recovered(enabled(), "c1", Mode::Login.service());
    assert_eq!(
        run.on(Input::ConfigReloaded(config(enabled()))),
        [Effect::Front(FrontUnit::Start { port: 3389 })],
        "a port another program held at the last start is tried again"
    );
}

#[test]
fn turning_remote_off_stops_the_front_and_denies_pending_logins_but_keeps_connected_clients() {
    let remote = MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    };
    let mut run = remote_run(remote.clone());
    run.anna_in_background(1, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    run.on(ready(1, "anna"));
    run.on(Input::Forwarded { conn: 1, ok: true });
    assert_eq!(
        run.viewers(ANNA, 1),
        [Effect::Inhibit(Inhibit::SleepAndIdle)]
    );
    assert_eq!(run.on(tls_from(7, 2, "bert", "pw")), looked_up(7, "bert"));
    run.on(tls_from(8, 3, "carl", "pw"));
    assert_eq!(
        run.on(member(8, CARL)),
        remote_spawn(3, Mode::Remote, "carl", 3)
    );
    run.on(tls_from(9, 4, "dora", "pw"));
    let unknown = Input::Member {
        key: LookUpKey::Conn(9),
        uid: None,
        member: false,
        failed: false,
    };
    assert_eq!(quiet(run.on(unknown)), []);

    let off = MachineRemote {
        enabled: false,
        ..remote
    };
    let effects = run.on(Input::ConfigReloaded(config(off)));
    assert_eq!(
        quiet(effects),
        [
            Effect::Front(FrontUnit::Stop),
            front(FrontEvent::Denied {
                conn: 7,
                backoff_s: 0
            }),
            Effect::Kill(WorkerId(3)),
            front(FrontEvent::Denied {
                conn: 8,
                backoff_s: 0
            }),
            front(FrontEvent::Denied {
                conn: 9,
                backoff_s: 0
            }),
            Effect::CloseIntakes {
                keep: BTreeSet::from([ANNA])
            },
        ],
        "anna's client keeps the inhibitor and arms no Disconnected timer"
    );
    let gen1 = Timer::Disconnected {
        id: "c2".into(),
        generation: 1,
    };
    assert_eq!(run.fire(gen1), []);
    assert_eq!(run.on(member(7, BERT)), [], "a denied login is forgotten");
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(9)), []);
    assert_eq!(quiet(run.on(tls(10, "anna", "pw"))), armed(10));
    assert_eq!(
        run.after(REFUSE_DELAY).fire(Timer::Refuse(10)),
        denied(10, 0),
        "logins stay off"
    );
    assert_eq!(
        run.viewers(ANNA, 0),
        [disconnect_arm("c2", 2, 30), Effect::Inhibit(Inhibit::Idle)],
        "the kept link still reports"
    );
}

#[test]
fn a_port_change_moves_the_held_port() {
    let mut run = remote_run(enabled());
    let moved = MachineRemote {
        port: 3390,
        ..enabled()
    };
    assert_eq!(
        run.on(Input::ConfigReloaded(config(moved.clone()))),
        [Effect::Front(FrontUnit::Start { port: 3390 })]
    );
    let more = MachineRemote {
        max_sessions: 2,
        ..moved
    };
    assert_eq!(
        run.on(Input::ConfigReloaded(config(more))),
        [Effect::Front(FrontUnit::Start { port: 3390 })],
        "other remote settings apply to the next login"
    );
    let mut run = remote_run(MachineRemote::default());
    let moved_off = MachineRemote {
        port: 3390,
        ..MachineRemote::default()
    };
    assert_eq!(
        run.on(Input::ConfigReloaded(config(moved_off))),
        [Effect::Front(FrontUnit::Stop)],
        "no port is held while remote logins are off"
    );
}
