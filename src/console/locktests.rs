use std::process::Command;
use std::time::{Duration, Instant};

use smithay::input::keyboard::xkb::{self, compose};

use super::Side;
use super::tests::{in_lock_mode, in_lock_mode_with, under_lock_layer};
use crate::compose::Compose;
use crate::config::Config;
use crate::harness::Harness;
use crate::inputproto::InputSource;
use crate::lock;
use crate::lockfinger::{self, PasswordFor};
use crate::lockview::{self, Helpers};
use crate::startmenu::StartRow;

const E: u32 = 18;
pub(super) const ENTER: u32 = 28;
const L: u32 = 38;
const APOSTROPHE: u32 = 40;
const X: u32 = 45;
const SUPER: u32 = 125;
/// "hunter2" as evdev codes.
pub(super) const HUNTER2: [u32; 7] = [35, 22, 49, 20, 18, 19, 3];

fn covers(h: &Harness, side: Side) -> Vec<String> {
    (h.state.lock.on(side).iter())
        .flat_map(|l| l.covered())
        .collect()
}

fn field(h: &Harness, side: Side) -> String {
    let l = h.state.lock.on(side).expect("the side is locked");
    l.input().to_owned()
}

pub(super) fn sh(script: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(script);
    c
}

/// The password conversation takes "hunter2"; no fingerprint conversation runs.
pub(super) fn hunter2(h: &mut Harness) {
    h.state.lock.helpers = Helpers {
        password: |_, _| sh(r#"read p; [ "$p" = hunter2 ] && echo '= 0' || echo '= 7'"#),
        finger: |_| None,
        ..Helpers::default()
    };
}

pub(super) fn tap(h: &mut Harness, codes: &[u32]) {
    for &c in codes {
        h.key(c, true);
        h.key(c, false);
    }
}

fn remote_submit(h: &mut Harness, password: &str) {
    h.from(InputSource::Remote, |s| lockview::type_text(s, password));
    h.remote_key(ENTER, true);
    h.remote_key(ENTER, false);
}

fn verifying(h: &Harness, side: Side) -> bool {
    h.state.lock.on(side).is_some_and(|l| l.verifying())
}

/// Turn the loop until neither side is verifying (or 5 s pass).
pub(super) fn settle(h: &mut Harness) {
    let end = Instant::now() + Duration::from_secs(5);
    while verifying(h, Side::Console) || verifying(h, Side::Remote) {
        assert!(Instant::now() < end, "the checks end");
        h.run_for(5);
    }
}

fn message(h: &Harness, side: Side) -> Option<String> {
    h.state.lock.on(side)?.message()
}

fn assert_remote_locked(h: &Harness) {
    assert!(h.state.desktop_locked(), "the remote side is locked");
    assert_eq!(covers(h, Side::Remote), ["REMOTE-1"]);
    assert_eq!(covers(h, Side::Console), ["eDP-1"]);
}

#[test]
fn the_remote_idle_lock_covers_the_remote_screens_only() {
    let mut config = Config::default();
    config.idle.lock_after = 1;
    config.idle.lock_fade = 0;
    config.idle.screen_off_after = 0;
    config.idle.suspend_after = 0;
    config.idle.blank_after = 0;
    let (mut h, _portal, _lock) = in_lock_mode_with(config);
    assert!(!h.state.desktop_locked());
    h.run_for(1500);
    assert_remote_locked(&h);
    let remote = &crate::virtualout::remote_outputs(&h.state)[0];
    let area = reprisescene::output_rect(remote).expect("in the layout");
    let leaves = h.state.scene.leaves_in(area);
    assert!(
        leaves.iter().any(|&n| under_lock_layer(&h, n)),
        "a capture of REMOTE-1 shows its cover"
    );
}

#[test]
fn start_lock_from_the_remote_locks_the_remote_side() {
    let (mut h, _portal, _lock) = in_lock_mode();
    h.from(InputSource::Remote, |s| s.start_row(StartRow::Lock));
    assert_remote_locked(&h);
}

#[test]
fn the_remote_lock_key_locks_the_remote_side() {
    let (mut h, _portal, _lock) = in_lock_mode();
    h.remote_key(SUPER, true);
    h.remote_key(L, true);
    h.remote_key(L, false);
    h.remote_key(SUPER, false);
    assert_remote_locked(&h);
}

#[test]
fn the_remote_password_unlocks_the_remote_side_and_not_the_console() {
    let (mut h, _portal, _lock) = in_lock_mode();
    hunter2(&mut h);
    h.from(InputSource::Remote, lockview::lock_desktop);
    h.from(InputSource::Remote, |s| lockview::type_text(s, "hunter2"));
    assert_eq!(field(&h, Side::Remote), "hunter2");
    assert_eq!(field(&h, Side::Console), "", "the console's field is apart");
    h.remote_key(ENTER, true);
    h.remote_key(ENTER, false);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while h.state.desktop_locked() {
        assert!(std::time::Instant::now() < end, "the remote side unlocks");
        h.run_for(5);
    }
    assert!(h.state.console_locked(), "the console stays locked");
    assert_eq!(covers(&h, Side::Console), ["eDP-1"]);
    let console = h.state.lock.console.as_ref().unwrap();
    assert!(!console.verifying() && console.pending.is_none());
}

#[test]
fn a_remote_password_that_must_be_changed_unlocks_with_a_notice_and_no_back_off() {
    let (mut h, _portal, _lock) = in_lock_mode();
    h.state.lock.helpers = Helpers {
        password: |_, _| sh(r#"read p; [ "$p" = hunter2 ] && echo '= 12' || echo '= 7'"#),
        finger: |_| None,
        ..Helpers::default()
    };
    for _ in 0..5 {
        h.from(InputSource::Remote, lockview::lock_desktop);
        remote_submit(&mut h, "hunter2");
        settle(&mut h);
        assert!(!h.state.desktop_locked(), "the remote side unlocks");
    }
    assert!(h.state.console_locked(), "the console stays locked");
    assert!(
        crate::notifyview::balloon_up(&h.state),
        "a notice says the password has expired"
    );
    assert!(
        (h.state.lock.remote_backoff.wait(Instant::now())).is_zero(),
        "no unlock counts as a failure"
    );
}

#[test]
fn lock_mode_publishes_only_the_remote_sides_lock() {
    let (mut h, _portal, _lock) = in_lock_mode();
    let (_tx, _wake, heard) = crate::tray::headless(&mut h.state);
    let sent = |h: &mut Harness| -> Vec<bool> {
        h.dispatch();
        (std::iter::from_fn(|| heard.try_recv().ok()))
            .filter_map(|c| match c {
                crate::tray::TrayCommand::Locked(l) => Some(l),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        sent(&mut h),
        [false],
        "unlocked while only the console is locked"
    );
    lockview::lock_remote(&mut h.state);
    assert_eq!(sent(&mut h), [true]);
    assert!(sent(&mut h).is_empty(), "once per change");
}

#[test]
fn leaving_lock_mode_drops_the_remote_lock() {
    let (mut h, mut portal, lock) = in_lock_mode();
    lockview::lock_remote(&mut h.state);
    assert!(h.state.lock.remote.is_some());
    lock.object.destroy();
    h.roundtrip(&mut portal);
    assert!(!super::remote_side(&h.state), "lock mode ends");
    assert!(h.state.lock.remote.is_none(), "the remote lock goes");
    assert!(h.state.console_locked(), "the console lock stays");
    assert_eq!(covers(&h, Side::Console), ["eDP-1"]);
}

#[test]
fn a_physical_dead_key_never_combines_with_a_remote_key() {
    let mut config = Config::default();
    config.keyboard.layout = "us".into();
    config.keyboard.variant = "intl".into();
    let (mut h, _portal, _lock) = in_lock_mode_with(config);
    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let table = compose::Table::new_from_buffer(
        &ctx,
        "<dead_acute> <e> : \"é\"\n",
        "C",
        compose::FORMAT_TEXT_V1,
        compose::COMPILE_NO_FLAGS,
    )
    .expect("the table compiles");
    h.state.lock.console.as_mut().unwrap().view.compose = Some(Compose::over(&table));
    h.state.compose = Some(table);
    h.from(InputSource::Remote, lockview::lock_desktop);
    h.key(APOSTROPHE, true);
    h.key(APOSTROPHE, false);
    h.remote_key(E, true);
    h.remote_key(E, false);
    assert_eq!(field(&h, Side::Remote), "e", "the remote types a plain e");
    assert_eq!(field(&h, Side::Console), "", "the dead key waits");
    h.key(E, true);
    h.key(E, false);
    assert_eq!(field(&h, Side::Console), "é");
}

#[test]
fn the_remote_screen_checks_its_password_under_the_remote_service() {
    let rdp = lockfinger::password_command("alice", PasswordFor::RemoteDesktop);
    let args: Vec<_> = rdp.get_args().collect();
    assert_eq!(args, ["--rdp-password-conversation=alice"]);
    let (mut h, _portal, _lock) = in_lock_mode();
    h.state.lock.helpers = Helpers {
        password: |_, purpose| match purpose {
            PasswordFor::RemoteDesktop => {
                sh(r#"read p; [ "$p" = hunter2 ] && echo '= 0' || echo '= 7'"#)
            }
            PasswordFor::Unlock => sh("read p; echo '= 7'"),
        },
        finger: |_| None,
        ..Helpers::default()
    };
    h.from(InputSource::Remote, lockview::lock_desktop);
    remote_submit(&mut h, "hunter2");
    settle(&mut h);
    assert!(
        !h.state.desktop_locked(),
        "the remote service takes the password"
    );
}

#[test]
fn remote_unlock_failures_back_off() {
    let (mut h, _portal, _lock) = in_lock_mode();
    hunter2(&mut h);
    h.from(InputSource::Remote, lockview::lock_desktop);
    for _ in 0..4 {
        remote_submit(&mut h, "nope");
        settle(&mut h);
        assert_eq!(message(&h, Side::Remote).as_deref(), Some(lock::INCORRECT));
    }
    remote_submit(&mut h, "hunter2");
    let remote = h
        .state
        .lock
        .remote
        .as_ref()
        .expect("the remote side stays locked");
    assert!(remote.pending.is_none(), "the fifth try is not checked");
    assert_eq!(
        message(&h, Side::Remote).as_deref(),
        Some("Too many incorrect passwords. Wait 2 seconds and retry.")
    );
    tap(&mut h, &[X, ENTER]);
    settle(&mut h);
    assert_eq!(
        message(&h, Side::Console).as_deref(),
        Some(lock::INCORRECT),
        "the console's check runs at once"
    );
}

#[test]
fn the_console_password_ends_lock_mode_and_the_remote_lock_with_it() {
    let (mut h, _portal, _lock) = in_lock_mode();
    hunter2(&mut h);
    h.from(InputSource::Remote, lockview::lock_desktop);
    tap(&mut h, &HUNTER2);
    tap(&mut h, &[ENTER]);
    settle(&mut h);
    assert!(!h.state.console_locked(), "the console unlocks");
    assert!(!super::remote_side(&h.state), "lock mode ends");
    assert!(h.state.lock.remote.is_none(), "the remote lock goes");
    assert!(!h.state.desktop_locked());
}

/// The file both sides' checks wait for before they answer.
fn verdict_gate() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("reprise-verdict-gate-{}", std::process::id()))
}

#[test]
fn both_sides_verifying_at_once_each_get_their_own_verdict() {
    let (mut h, _portal, _lock) = in_lock_mode();
    let _ = std::fs::remove_file(verdict_gate());
    h.state.lock.helpers = Helpers {
        password: |_, _| {
            let gate = verdict_gate();
            sh(&format!(
                r#"read p; i=0; while [ ! -e '{}' ] && [ $i -lt 1000 ]; do sleep 0.01; i=$((i+1)); done; [ "$p" = hunter2 ] && echo '= 0' || echo '= 7'"#,
                gate.display()
            ))
        },
        finger: |_| None,
        ..Helpers::default()
    };
    h.from(InputSource::Remote, lockview::lock_desktop);
    remote_submit(&mut h, "hunter2");
    tap(&mut h, &[X, ENTER]);
    assert!(verifying(&h, Side::Remote) && verifying(&h, Side::Console));
    std::fs::write(verdict_gate(), b"").unwrap();
    settle(&mut h);
    let _ = std::fs::remove_file(verdict_gate());
    assert!(!h.state.desktop_locked(), "the remote side unlocks");
    assert!(h.state.console_locked(), "the console stays locked");
    assert_eq!(message(&h, Side::Console).as_deref(), Some(lock::INCORRECT));
}

#[test]
fn a_physical_lock_key_in_lock_mode_keeps_the_console_screen_and_leaves_the_remote_side_alone() {
    let (mut h, _portal, _lock) = in_lock_mode();
    h.key(SUPER, true);
    h.key(L, true);
    h.key(L, false);
    h.key(SUPER, false);
    let console = h
        .state
        .lock
        .console
        .as_ref()
        .expect("the console stays locked");
    assert!(console.screen_shown(), "the console's screen is up");
    assert_eq!(covers(&h, Side::Console), ["eDP-1"]);
    assert!(!h.state.desktop_locked(), "the remote side stays unlocked");
    assert!(covers(&h, Side::Remote).is_empty());
}

/// Each of `codes` pressed and released through the keys object `keys`.
fn keys_tap(h: &mut Harness, keys: crate::remotekeys::KeysId, codes: &[u32]) {
    for &c in codes {
        h.state.keys_key(keys, c, true);
        h.state.keys_key(keys, c, false);
    }
}

#[test]
fn the_servers_keys_object_types_into_the_remote_lock() {
    let (mut h, _portal, _lock) = in_lock_mode();
    h.from(InputSource::Remote, lockview::lock_desktop);
    let server = h.state.keys_create(None);
    keys_tap(&mut h, server, &HUNTER2);
    assert_eq!(field(&h, Side::Remote), "hunter2");
    assert_eq!(field(&h, Side::Console), "", "the console's field is apart");
}

#[test]
fn the_servers_keys_object_types_into_the_lock_and_an_apps_does_not() {
    let mut h = Harness::with_config(Config::default());
    h.output("eDP-1", 1920, 1080);
    lockview::lock(&mut h.state);
    let server = h.state.keys_create(None);
    let app = h.state.keys_create(Some("org.example.Remote".into()));
    keys_tap(&mut h, server, &HUNTER2);
    keys_tap(&mut h, app, &[X]);
    assert_eq!(field(&h, Side::Console), "hunter2");
}
