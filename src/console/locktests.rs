use std::process::Command;

use smithay::input::keyboard::xkb::{self, compose};

use super::Side;
use super::tests::{in_lock_mode, in_lock_mode_with, under_lock_layer};
use crate::compose::Compose;
use crate::config::Config;
use crate::harness::Harness;
use crate::inputproto::InputSource;
use crate::lockview::{self, Helpers};
use crate::startmenu::StartRow;

const E: u32 = 18;
const ENTER: u32 = 28;
const L: u32 = 38;
const APOSTROPHE: u32 = 40;
const SUPER: u32 = 125;

fn covers(h: &Harness, side: Side) -> Vec<String> {
    (h.state.lock.on(side).iter())
        .flat_map(|l| l.outputs.iter().map(|lo| lo.output.name()))
        .collect()
}

fn field(h: &Harness, side: Side) -> String {
    let l = h.state.lock.on(side).expect("the side is locked");
    l.state.input.clone()
}

fn sh(script: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(script);
    c
}

/// The password conversation takes "hunter2"; no fingerprint conversation runs.
fn hunter2(h: &mut Harness) {
    h.state.lock.helpers = Helpers {
        password: |_| sh(r#"read p; [ "$p" = hunter2 ] && echo '= 0' || echo '= 7'"#),
        finger: |_| None,
        ..Helpers::default()
    };
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
    assert!(!console.state.verifying && console.pending.is_none());
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
    h.state.lock.console.as_mut().unwrap().compose = Some(Compose::over(&table));
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
