use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::*;
use crate::dialog::Style;
use crate::harness::Harness;
use crate::lockview::Origin;
use reprisekit::controls::endsession::Goal;
use reprisekit::powerpanel::UpdatesBox;

/// A PackageKit that records its calls, answers its status with `status`, Trigger with `answer`
/// and Cancel with `cancel`, and the computer going down (logind or systemd) while `down`. It
/// records the prompts' withdrawal too.
struct Fake {
    status: Option<Status>,
    answer: Answer,
    cancel: Answer,
    down: AtomicBool,
    calls: Mutex<Vec<String>>,
}

impl Fake {
    /// An update prepared, `triggered` elsewhere or not, and Trigger and Cancel answering `answer`.
    fn new(triggered: bool, answer: Answer) -> Arc<Fake> {
        let status = Some(Status {
            prepared: true,
            triggered,
        });
        Fake::with(status, answer.clone(), answer)
    }

    fn with(status: Option<Status>, answer: Answer, cancel: Answer) -> Arc<Fake> {
        Arc::new(Fake {
            status,
            answer,
            cancel,
            down: AtomicBool::new(false),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn call(&self, what: &str) {
        self.calls.lock().unwrap().push(what.into());
    }
}

impl Bus for Fake {
    fn status(&self) -> Option<Status> {
        self.call("status");
        self.status
    }
    fn trigger(&self, after: After, _: bool) -> Answer {
        self.call(after.action());
        self.answer.clone()
    }
    fn cancel(&self, _: bool) -> Answer {
        self.call("cancel");
        self.cancel.clone()
    }
    fn shutting_down(&self) -> bool {
        self.call("down");
        self.down.load(Ordering::SeqCst)
    }
    fn withdraw(&self) {
        self.call("withdraw");
    }
}

type Ran = (Result<Option<&'static str>, Held<&'static str>>, bool);

/// [`around`] for attempt 1 on `fake` with a power action that fails when `power_fails`, while
/// `still` holds; the answer and whether the power action ran.
fn run(fake: &Fake, updates: Updates, still: &dyn Fn() -> bool, power_fails: bool) -> Ran {
    run_on(fake, &Made::default(), updates, still, || {
        power_fails.then_some("refused")
    })
}

fn run_on(
    fake: &Fake,
    made: &Made,
    updates: Updates,
    still: &dyn Fn() -> bool,
    power: impl FnOnce() -> Option<&'static str>,
) -> Ran {
    let ran = Cell::new(false);
    let answer = around(fake, made, 1, updates, true, still, || {
        ran.set(true);
        power()
    });
    (answer, ran.get())
}

#[test]
fn install_triggers_just_before_the_power_action_and_takes_it_back_when_that_fails() {
    let fake = Fake::new(false, Ok(()));
    let made = Made::default();
    let went = run_on(&fake, &made, Updates::Install, &|| true, || None);
    assert_eq!(went, (Ok(None), true));
    assert_eq!(
        fake.calls(),
        ["status", "reboot"],
        "restarts after the install"
    );
    assert_eq!(
        made.by(),
        None,
        "a trigger whose action went through is not taken back later"
    );
    let fake = Fake::new(false, Ok(()));
    let failed = run(&fake, Updates::Install, &|| true, true);
    assert_eq!(failed, (Ok(Some("refused")), true));
    assert_eq!(fake.calls(), ["status", "reboot", "down", "cancel"]);
}

#[test]
fn install_over_a_trigger_made_elsewhere_neither_triggers_again_nor_takes_it_back() {
    let fake = Fake::new(true, Ok(()));
    let failed = run(&fake, Updates::Install, &|| true, true);
    assert_eq!(failed, (Ok(Some("refused")), true));
    assert_eq!(fake.calls(), ["status"], "its action stays as it was set");
}

#[test]
fn install_with_nothing_prepared_any_more_runs_a_plain_power_action() {
    let stale = Some(Status::default());
    let fake = Fake::with(stale, Err(Some("Prepared update not found".into())), Ok(()));
    assert_eq!(
        run(&fake, Updates::Install, &|| true, false),
        (Ok(None), true)
    );
    assert_eq!(fake.calls(), ["status"]);
}

#[test]
fn a_status_that_does_not_come_counts_a_trigger_as_another_programs() {
    let fake = Fake::with(None, Ok(()), Ok(()));
    let failed = run(&fake, Updates::Install, &|| true, true);
    assert_eq!(failed, (Ok(Some("refused")), true));
    assert_eq!(fake.calls(), ["status", "reboot"], "not taken back");
    let fake = Fake::with(None, Ok(()), Ok(()));
    assert_eq!(run(&fake, Updates::Skip, &|| true, false), (Ok(None), true));
    assert_eq!(
        fake.calls(),
        ["status", "cancel"],
        "an untick still cancels"
    );
}

#[test]
fn a_trigger_that_will_not_come_back_after_a_failed_action_is_told() {
    let ready = Some(Status {
        prepared: true,
        triggered: false,
    });
    let fake = Fake::with(ready, Ok(()), Err(Some("Not allowed.".into())));
    let failed = run(&fake, Updates::Install, &|| true, true);
    assert_eq!(failed, (Err(Held::Stays("refused")), true));
    assert_eq!(fake.calls(), ["status", "reboot", "down", "cancel"]);
}

#[test]
fn an_attempt_given_up_on_triggers_nothing_or_takes_its_trigger_back() {
    let fake = Fake::new(false, Ok(()));
    let gone = run(&fake, Updates::Install, &|| false, false);
    assert_eq!(gone, (Err(Held::Dismissed), false));
    assert!(fake.calls().is_empty());
    let fake = Fake::new(false, Ok(()));
    let asked = Cell::new(0);
    let still = || {
        asked.set(asked.get() + 1);
        asked.get() == 1
    };
    let late = run(&fake, Updates::Install, &still, false);
    assert_eq!(
        late,
        (Err(Held::Dismissed), false),
        "given up during Trigger"
    );
    assert_eq!(fake.calls(), ["status", "reboot", "down", "cancel"]);
    let prepared = Some(Status {
        prepared: true,
        triggered: false,
    });
    let stuck = Fake::with(prepared, Ok(()), Err(Some("no".into())));
    asked.set(0);
    let stays = run(&stuck, Updates::Install, &still, false);
    assert_eq!(
        stays,
        (Err(Held::GivenUpStays), false),
        "a trigger that would not come back is told"
    );
}

#[test]
fn a_trigger_left_while_its_action_runs_is_taken_back_by_the_next_attempt_or_the_end() {
    let fake = Fake::new(false, Ok(()));
    let made = Made::default();
    let next = || {
        // Attempt 2 (a critical battery's Turn Off) starts while attempt 1's Reboot waits.
        assert_eq!(made.by(), Some(1));
        let mine = around(&*fake, &made, 2, Updates::Leave, false, &|| true, || {
            None::<()>
        });
        assert_eq!(mine, Ok(None));
        Some("given up")
    };
    let first = run_on(&fake, &made, Updates::Install, &|| true, next);
    assert_eq!(first, (Ok(Some("given up")), true));
    assert_eq!(
        fake.calls(),
        ["status", "reboot", "down", "cancel"],
        "taken back once, by attempt 2"
    );
    let fake = Fake::new(false, Ok(()));
    let made = Made::default();
    let end = || {
        made.take_back(&*fake, None);
        Some("given up")
    };
    let first = run_on(&fake, &made, Updates::Install, &|| true, end);
    assert_eq!(first, (Ok(Some("given up")), true));
    assert_eq!(
        fake.calls(),
        ["status", "reboot", "down", "cancel"],
        "Log Off took it back"
    );
}

#[test]
fn a_left_trigger_that_will_not_come_back_is_told_when_the_next_action_fails() {
    let next = |updates, triggered, power_fails: bool| {
        let fake = Fake::new(triggered, Err(Some("Not allowed.".into())));
        let made = Made::default();
        made.set(Some(1));
        let power = || power_fails.then_some("refused");
        (
            around(&*fake, &made, 2, updates, false, &|| true, power),
            made.by(),
        )
    };
    assert_eq!(
        next(Updates::Leave, true, true),
        (Err(Held::Stays("refused")), None)
    );
    assert_eq!(next(Updates::Leave, true, false), (Ok(None), None));
    assert_eq!(
        next(Updates::Install, true, true),
        (Err(Held::Stays("refused")), None),
        "set to install already, by attempt 1"
    );
    let fake = Fake::with(None, Ok(()), Ok(()));
    let made = Made::default();
    made.set(Some(1));
    let failed = around(&*fake, &made, 2, Updates::Leave, false, &|| true, || {
        Some("refused")
    });
    assert_eq!(failed, Ok(Some("refused")), "taken back");
}

#[test]
fn a_trigger_handed_over_after_a_late_action_is_taken_back_by_nothing() {
    let fake = Fake::new(true, Ok(()));
    let made = Made::default();
    made.set(Some(1));
    assert_eq!(made.hand_over(2), Some(false), "not attempt 2's");
    let held = made.locked();
    assert_eq!(made.hand_over(1), None, "a Cancel holds the lock");
    drop(held);
    assert_eq!(made.hand_over(1), Some(true));
    assert_eq!(made.take_back(&*fake, None), Left::Nothing, "Log Off");
    let next = around(&*fake, &made, 2, Updates::Leave, false, &|| true, || {
        None::<()>
    });
    assert_eq!(next, Ok(None));
    assert!(fake.calls().is_empty(), "asked nothing, cancelled nothing");
}

#[test]
fn the_session_ending_during_a_shutdown_keeps_a_trigger_an_attempt_left() {
    let fake = Fake::new(false, Ok(()));
    let made = Made::default();
    made.take_back(&*fake, None);
    assert!(fake.calls().is_empty(), "no trigger left, nothing asked");
    *made.lock() = Some(1);
    made.take_back(&*fake, None);
    assert_eq!(fake.calls(), ["down", "cancel"], "taken back outside one");
    let fake = Fake::new(false, Ok(()));
    fake.down.store(true, Ordering::SeqCst);
    *made.lock() = Some(1);
    made.take_back(&*fake, None);
    assert_eq!(
        fake.calls(),
        ["down"],
        "the update installs at this restart"
    );
    assert_eq!(made.by(), None);
}

#[test]
fn a_power_command_the_shutdown_kills_keeps_its_trigger() {
    let fake = Fake::new(false, Ok(()));
    let killed = || {
        fake.down.store(true, Ordering::SeqCst);
        Some("signal 15")
    };
    let failed = run_on(&fake, &Made::default(), Updates::Install, &|| true, killed);
    assert_eq!(failed, (Err(Held::Stays("signal 15")), true));
    assert_eq!(fake.calls(), ["status", "reboot", "down"]);
}

#[test]
fn a_trigger_that_does_not_go_through_runs_no_power_action() {
    let refused = Fake::new(false, Err(Some("Not allowed.".into())));
    let failed = run(&refused, Updates::Install, &|| true, false);
    assert_eq!(failed, (Err(Held::NotSet("Not allowed.".into())), false));
    let dismissed = Fake::new(false, Err(None));
    let failed = run(&dismissed, Updates::Install, &|| true, false);
    assert_eq!(failed, (Err(Held::Dismissed), false));
}

#[test]
fn skip_takes_back_only_a_trigger_made_elsewhere_and_one_that_stays_holds_the_action() {
    let fake = Fake::new(true, Ok(()));
    assert_eq!(run(&fake, Updates::Skip, &|| true, false), (Ok(None), true));
    assert_eq!(fake.calls(), ["status", "cancel"]);
    let fake = Fake::new(false, Ok(()));
    assert_eq!(run(&fake, Updates::Skip, &|| true, false), (Ok(None), true));
    assert_eq!(fake.calls(), ["status"], "nothing to take back");
    let fake = Fake::new(true, Err(Some("Not allowed.".into())));
    let held = run(&fake, Updates::Skip, &|| true, false);
    assert_eq!(held, (Err(Held::StillSet), false));
}

#[test]
fn leave_asks_packagekit_nothing() {
    let fake = Fake::new(true, Ok(()));
    assert!(run(&fake, Updates::Leave, &|| true, true).1);
    assert!(fake.calls().is_empty());
}

/// A harness on `fake` whose jobs run on threads, so their answers land later as they do live.
fn harness(fake: &Arc<Fake>) -> Harness {
    let mut h = Harness::new();
    h.output("TEST-1", 1024, 768);
    crate::fsjobs::setup(&mut h.state);
    h.state.offline = Offline::on(fake.clone());
    h
}

fn panel_updates(h: &Harness) -> Option<UpdatesBox> {
    match &h.state.dialogs.views.last()?.dialog.style {
        Style::Welcome(w) => w.panel.updates,
        _ => None,
    }
}

#[test]
fn a_box_that_arrives_after_the_panel_opened_shows_unticked_and_the_next_panel_ticked() {
    let fake = Fake::new(false, Ok(()));
    let mut h = harness(&fake);
    crate::power::run_action(
        &mut h.state,
        crate::config::PowerAction::Ask,
        Origin::Desktop,
    );
    assert_eq!(panel_updates(&h), None, "nothing known yet");
    h.until("the answer", |s| s.offline.known.prepared);
    let late = panel_updates(&h).map(|b| b.checked);
    assert_eq!(late, Some(false), "late, so unticked");
    h.state.close_dialog_at(0);
    h.state.blue_turn_off();
    let next = panel_updates(&h).map(|b| b.checked);
    assert_eq!(next, Some(true), "from its first frame");
    assert_eq!(
        fake.calls(),
        ["status"],
        "a fresh answer is not asked again"
    );
}

#[test]
fn nothing_prepared_leaves_the_panel_and_the_log_off_panel_alone() {
    struct Nothing;
    impl Bus for Nothing {
        fn status(&self) -> Option<Status> {
            Some(Status::default())
        }
        fn trigger(&self, _: After, _: bool) -> Answer {
            Ok(())
        }
        fn cancel(&self, _: bool) -> Answer {
            Ok(())
        }
        fn shutting_down(&self) -> bool {
            false
        }
    }
    let mut h = Harness::new();
    h.output("TEST-1", 1024, 768);
    crate::fsjobs::setup(&mut h.state);
    h.state.offline = Offline::on(Arc::new(Nothing));
    h.state.offline.known.prepared = true;
    h.state.blue_turn_off();
    h.until("the answer", |s| !s.offline.known.prepared);
    assert_eq!(
        panel_updates(&h),
        None,
        "hidden once PackageKit says nothing is prepared"
    );
    h.state.close_dialog_at(0);
    h.state.offline.known.prepared = true;
    h.state.run_action(&crate::config::Action::LogOff, 0);
    assert_eq!(panel_updates(&h), None, "Log Off has none");
}

/// Wait for the power attempt a session end starts, and for its answer.
fn answered(h: &mut Harness) {
    h.until("the attempt", |s| s.powering.busy());
    h.until("its answer", |s| !s.powering.busy());
    crate::crash::not_ending();
}

#[test]
fn a_turn_off_that_installs_triggers_at_its_power_action_and_runs_the_poweroff_command() {
    let fake = Fake::new(false, Ok(()));
    let mut h = harness(&fake);
    let install = Updates::Install;
    crate::sessionend::begin_updates(&mut h.state, Goal::TurnOff, "false".into(), install);
    answered(&mut h);
    assert_eq!(
        fake.calls(),
        ["status", "reboot", "down", "cancel"],
        "taken back"
    );
    let d = &h.state.dialogs.views.last().unwrap().dialog;
    assert_eq!(
        d.title, "Turn Off Computer",
        "the poweroff command's refusal"
    );
}

#[test]
fn a_power_key_turn_off_leaves_a_trigger_made_elsewhere() {
    let fake = Fake::new(true, Ok(()));
    let mut h = harness(&fake);
    h.state.config.power.poweroff = "true".into();
    crate::power::run_action(
        &mut h.state,
        crate::config::PowerAction::TurnOff,
        Origin::Desktop,
    );
    answered(&mut h);
    assert!(fake.calls().is_empty());
}

#[test]
fn an_unticked_trigger_that_will_not_cancel_asks_whether_to_turn_off_anyway() {
    let fake = Fake::new(true, Err(Some("Not allowed.".into())));
    let mut h = harness(&fake);
    let skip = Updates::Skip;
    crate::sessionend::begin_updates(&mut h.state, Goal::TurnOff, "true".into(), skip);
    answered(&mut h);
    assert_eq!(fake.calls(), ["status", "cancel"]);
    let d = &h.state.dialogs.views.last().unwrap().dialog;
    assert!(
        d.text
            .starts_with("Updates will install the next time the computer starts"),
        "{}",
        d.text
    );
    let again = d.buttons.iter().find_map(|b| match &b.action {
        crate::dialog::DialogAction::PowerAnyway(a) => Some(a.clone()),
        _ => None,
    });
    let again = again.expect("Turn Off Anyway");
    assert_eq!(again.updates, Updates::Leave, "anyway leaves the trigger");
    assert_eq!(again.cmd, "true");
}

#[test]
fn a_turn_off_whose_prepared_update_went_stale_powers_off_plainly() {
    let stale = Some(Status::default());
    let fake = Fake::with(stale, Err(Some("Prepared update not found".into())), Ok(()));
    let mut h = harness(&fake);
    let install = Updates::Install;
    crate::sessionend::begin_updates(&mut h.state, Goal::TurnOff, "true".into(), install);
    answered(&mut h);
    assert_eq!(fake.calls(), ["status"]);
    assert!(h.state.dialogs.views.is_empty(), "no refusal");
}

#[test]
fn log_off_takes_back_a_trigger_an_attempt_it_gave_up_on_left() {
    let fake = Fake::new(false, Ok(()));
    let mut h = harness(&fake);
    *h.state.offline.made.lock() = Some(7);
    crate::sessionend::begin(&mut h.state, Goal::LogOff, None, Origin::Desktop);
    h.until("the take-back", |_| fake.calls() == ["down", "cancel"]);
    assert_eq!(h.state.offline.made.by(), None);
}

/// A Trigger whose password prompt waits on this compositor's loop: holds `made`'s lock until
/// told to let go, then leaves attempt 7's trigger behind.
fn hold(made: &Made) -> (std::sync::mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let made = made.clone();
    let (held, holding) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let trigger = std::thread::spawn(move || {
        let mut lock = made.lock();
        held.send(()).unwrap();
        let _ = released.recv_timeout(Duration::from_secs(10));
        *lock = Some(7);
    });
    holding.recv().unwrap();
    (release, trigger)
}

#[test]
fn log_off_keeps_the_loop_turning_while_a_trigger_holds_the_lock_then_takes_back_what_it_left() {
    let fake = Fake::new(false, Ok(()));
    let mut h = harness(&fake);
    let (release, trigger) = hold(&h.state.offline.made);
    let started = Instant::now();
    crate::sessionend::begin(&mut h.state, Goal::LogOff, None, Origin::Desktop);
    h.until("the take-back under way", |s| s.offline.ends.is_some());
    let took = started.elapsed();
    let bound = Duration::from_secs(5);
    assert!(
        took < bound,
        "the loop turns on past the end within {bound:?}, took {took:?}"
    );
    assert!(
        fake.calls().is_empty(),
        "nothing asked while the lock is held"
    );
    release.send(()).unwrap();
    trigger.join().unwrap();
    h.until("the end", |s| s.offline.ends.is_none());
    assert_eq!(fake.calls(), ["down", "cancel"]);
    assert_eq!(h.state.offline.made.by(), None);
}

#[test]
fn a_take_back_past_its_deadline_ends_the_session_once() {
    static ENDS: AtomicUsize = AtomicUsize::new(0);
    let fake = Fake::new(false, Ok(()));
    let mut h = harness(&fake);
    let (release, trigger) = hold(&h.state.offline.made);
    take_back_within(&mut h.state, Duration::from_millis(100), |_| {
        ENDS.fetch_add(1, Ordering::SeqCst);
    });
    h.until("the deadline", |_| ENDS.load(Ordering::SeqCst) == 1);
    release.send(()).unwrap();
    trigger.join().unwrap();
    h.until("the take-back", |_| fake.calls() == ["down", "cancel"]);
    let after = Instant::now();
    h.until("its answer", |_| {
        after.elapsed() > Duration::from_millis(200)
    });
    assert_eq!(ENDS.load(Ordering::SeqCst), 1, "the session ends once");
}

#[test]
fn a_take_back_that_panics_still_ends_the_session() {
    struct Panics;
    impl Bus for Panics {
        fn status(&self) -> Option<Status> {
            None
        }
        fn trigger(&self, _: After, _: bool) -> Answer {
            Ok(())
        }
        fn cancel(&self, _: bool) -> Answer {
            Ok(())
        }
        fn shutting_down(&self) -> bool {
            panic!("a bus read that panics");
        }
    }
    static ENDS: AtomicUsize = AtomicUsize::new(0);
    let mut h = Harness::new();
    crate::fsjobs::setup(&mut h.state);
    h.state.offline = Offline::on(Arc::new(Panics));
    h.state.offline.made.set(Some(7));
    take_back_within(&mut h.state, Duration::from_secs(60), |_| {
        ENDS.fetch_add(1, Ordering::SeqCst);
    });
    h.until("the end", |_| ENDS.load(Ordering::SeqCst) == 1);
}

#[test]
fn a_session_end_that_gives_up_the_power_action_withdraws_its_prompts_and_runs_no_action() {
    type End = fn(&mut State);
    let ends: [(&str, End); 2] = [
        ("quit", crate::sessionend::quit),
        ("log-off", |s| {
            crate::sessionend::begin(s, Goal::LogOff, None, Origin::Desktop)
        }),
    ];
    for (how, end) in ends {
        let fake = Fake::new(false, Ok(()));
        let mut h = harness(&fake);
        let ran =
            std::env::temp_dir().join(format!("reprise-given-up-{}-{how}", std::process::id()));
        let _ = std::fs::remove_file(&ran);
        let stand_by = format!("touch '{}'", ran.display());
        let attempt_done = |_: &State| Arc::strong_count(&fake) == 2;
        crate::sessionend::sleep(
            &mut h.state,
            crate::sessionend::Act::StandBy,
            stand_by.clone(),
        );
        h.until("the Stand By command", |_| ran.exists());
        h.until("its attempt's thread", attempt_done);
        std::fs::remove_file(&ran).unwrap();
        // The attempt's thread waits on the lock as on a password prompt.
        let (release, trigger) = hold(&h.state.offline.made);
        crate::sessionend::sleep(&mut h.state, crate::sessionend::Act::StandBy, stand_by);
        h.until("the attempt", |s| s.powering.busy());
        end(&mut h.state);
        h.until("the withdrawal", |_| {
            fake.calls().iter().any(|c| c == "withdraw")
        });
        release.send(()).unwrap();
        trigger.join().unwrap();
        h.until("its attempt's thread", attempt_done);
        assert!(!ran.exists(), "{how}: the Stand By command does not run");
        crate::crash::not_ending();
    }
}

#[test]
fn a_trigger_that_appears_between_the_shutdown_read_and_the_lock_is_read_under_the_lock() {
    let fake = Fake::new(false, Ok(()));
    fake.down.store(true, Ordering::SeqCst);
    let made = Made::default();
    made.set(Some(7));
    let first = Cell::new(true);
    let appears = |_| !first.replace(false);
    let (held, down) = lock_down(&made, &*fake, appears);
    assert!(down, "the computer is going down");
    assert_eq!(*held, Some(7));
    drop(held);
    assert_eq!(fake.calls(), ["down"], "read once, under the lock");
    let (held, down) = lock_down(&made, &*fake, |_| false);
    drop(held);
    assert!(!down);
    assert_eq!(fake.calls(), ["down"], "no trigger, nothing read");
}
