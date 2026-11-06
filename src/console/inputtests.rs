use smithay::backend::input::{Axis, AxisSource, InputTime};
use smithay::input::pointer::AxisFrame;

use super::tests::{click_lock_cancel, in_lock_mode, in_lock_mode_with, lock_input};
use crate::config::{Config, PowerAction};
use crate::harness::{Harness, KeyHeard, TestClient};
use crate::inputproto::InputSource;
use crate::window::WindowId;

const SHIFT: u32 = 42;
const A: u32 = 30;
const ENTER: u32 = 28;
const POWER: u32 = 116;
const BTN_LEFT: u32 = 0x110;
const KP6: u32 = 77;

/// A window on REMOTE-1 with the keyboard, its client's key log cleared.
fn remote_window(h: &mut Harness) -> (TestClient, WindowId) {
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "test.app", 400, 300);
    h.state.move_to(t.id, 2100, 200);
    let s = h.state.win(t.id).unwrap().wl_surface().clone();
    h.state.enter_keyboard(Some(s));
    h.roundtrip(&mut c);
    c.side.key_log.clear();
    (c, t.id)
}

fn keys_heard(c: &TestClient) -> Vec<(u32, bool)> {
    (c.side.key_log.iter())
        .filter_map(|k| match *k {
            KeyHeard::Key(code, down) => Some((code, down)),
            KeyHeard::Mods(_) => None,
        })
        .collect()
}

fn notch() -> AxisFrame {
    AxisFrame::new(InputTime::now())
        .source(AxisSource::Wheel)
        .value(Axis::Vertical, 15.0)
        .v120(Axis::Vertical, 120)
}

fn lock_mode(h: &mut Harness) -> (TestClient, crate::harness::LockScreen) {
    let mut portal = h.connect_portal();
    let lock = portal.lock_screen();
    lock.set_layout(&[(0, 0, 1280, 720, 100)]);
    h.roundtrip(&mut portal);
    assert!(super::remote_side(&h.state));
    (portal, lock)
}

#[test]
fn a_physical_key_held_across_lock_mode_start_never_reaches_the_remote() {
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "test.app", 400, 300);
    let s = h.state.win(t.id).unwrap().wl_surface().clone();
    h.state.enter_keyboard(Some(s.clone()));
    h.key(SHIFT, true);
    h.roundtrip(&mut c);
    let (_portal, _lock) = lock_mode(&mut h);
    h.state.move_to(t.id, 2100, 200);
    h.state.enter_keyboard(Some(s));
    h.roundtrip(&mut c);
    assert!(
        c.side.enter_keys.is_empty(),
        "the window enters with no key held"
    );
    c.side.key_log.clear();
    h.key(SHIFT, false);
    h.remote_key(A, true);
    h.remote_key(A, false);
    h.roundtrip(&mut c);
    assert_eq!(keys_heard(&c), [(A, true), (A, false)]);
    assert!(
        !(c.side.key_log.iter()).any(|k| matches!(k, KeyHeard::Mods(m) if m & 1 != 0)),
        "no Shift reaches the window: {:?}",
        c.side.key_log
    );
    let seat = h.state.seat.get_keyboard().unwrap();
    assert!(!seat.modifier_state().shift, "the seat holds no Shift");
}

#[test]
fn in_lock_mode_physical_typing_reaches_the_console_dialog_only() {
    let (mut h, _portal, _lock) = in_lock_mode();
    let (mut c, _) = remote_window(&mut h);
    h.key(SHIFT, true);
    h.key(A, true);
    h.key(A, false);
    h.key(SHIFT, false);
    h.key(A, true);
    h.key(A, false);
    h.roundtrip(&mut c);
    assert_eq!(lock_input(&h), "Aa");
    assert!(c.side.key_log.is_empty(), "heard {:?}", c.side.key_log);
}

#[test]
fn in_lock_mode_remote_keys_reach_the_focused_window_while_the_console_is_locked() {
    let (mut h, _portal, _lock) = in_lock_mode();
    let (mut c, _) = remote_window(&mut h);
    h.remote_key(A, true);
    h.remote_key(A, false);
    h.roundtrip(&mut c);
    assert_eq!(keys_heard(&c), [(A, true), (A, false)]);
    assert!(h.state.console_locked());
    assert_eq!(lock_input(&h), "");
}

#[test]
fn in_lock_mode_the_physical_pointer_stays_on_the_physical_outputs() {
    let (mut h, _portal, _lock) = in_lock_mode();
    let seat = h.state.pointer_location();
    h.pointer_to(2500.0, 300.0);
    assert_eq!(h.state.console.pointer, (1919.0, 300.0).into());
    assert_eq!(h.state.pointer_location(), seat, "the seat pointer stays");
}

#[test]
fn in_lock_mode_the_remote_pointer_stays_on_the_remote_screens() {
    let (mut h, _portal, _lock) = in_lock_mode();
    let console = h.state.console.pointer;
    h.remote_pointer_to(100.0, 100.0);
    assert_eq!(h.state.pointer_location(), (1920.0, 100.0).into());
    assert_eq!(
        h.state.console.pointer, console,
        "the computer's pointer stays"
    );
}

#[test]
fn in_lock_mode_eis_regions_cover_the_remote_screens_only() {
    let names = |h: &Harness| -> Vec<String> {
        (h.state.eis_regions().into_iter())
            .map(|(name, ..)| name)
            .collect()
    };
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    assert_eq!(names(&h), ["eDP-1"]);
    let (_portal, _lock) = lock_mode(&mut h);
    assert_eq!(names(&h), ["REMOTE-1"]);
}

#[test]
fn remote_activity_does_not_wake_the_physical_monitors_in_lock_mode() {
    let mut config = Config::default();
    config.idle.screen_off_after = 2;
    config.idle.suspend_after = 0;
    config.idle.blank_after = 0;
    config.idle.lock_after = 0;
    let (mut h, _portal, _lock) = in_lock_mode_with(config);
    h.run_for(2500);
    assert!(
        crate::idle::console_dark(&h.state),
        "the panel goes off at 2 s"
    );
    assert!(h.state.idle.steps.screen_off, "so do the remote screens");
    h.from(InputSource::Remote, crate::idle::notify_activity);
    assert!(!h.state.idle.steps.screen_off, "the remote screens wake");
    assert!(crate::idle::console_dark(&h.state), "the panel stays off");
    h.from(InputSource::Physical, crate::idle::notify_activity);
    assert!(
        !crate::idle::console_dark(&h.state),
        "the computer's input wakes it"
    );
}

#[test]
fn in_lock_mode_the_computers_input_holds_off_only_its_own_screen_off() {
    let mut config = Config::default();
    config.idle.screen_off_after = 2;
    config.idle.suspend_after = 0;
    config.idle.blank_after = 0;
    config.idle.lock_after = 0;
    let (mut h, _portal, _lock) = in_lock_mode_with(config);
    h.run_for(1500);
    h.from(InputSource::Physical, crate::idle::notify_activity);
    h.run_for(1000);
    assert!(!crate::idle::console_dark(&h.state), "the panel stays on");
    assert!(h.state.idle.steps.screen_off, "the remote screens go off");
}

#[test]
fn outside_lock_mode_eis_keys_still_unlock_the_lock_dialog() {
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    crate::lockview::lock(&mut h.state);
    h.remote_key(A, true);
    h.remote_key(A, false);
    assert_eq!(lock_input(&h), "a");
    h.remote_key(ENTER, true);
    let l = h.state.lock.active.as_ref().expect("locked");
    assert!(l.state.verifying, "Enter hands the password to the check");
}

#[test]
fn in_lock_mode_the_remote_wheel_switches_desktops_and_the_physical_one_does_not() {
    let (mut h, _portal, _lock) = in_lock_mode_with(Config {
        base: repriseconf::config::Config {
            workspaces: 4,
            wheel_on_desktop: true,
            ..Default::default()
        },
        ..Default::default()
    });
    h.remote_pointer_to(2500.0, 300.0);
    h.from(InputSource::Physical, |s| s.handle_axis(notch()));
    assert_eq!(h.state.desktops.current, 0, "the first desktop stays");
    h.from(InputSource::Remote, |s| s.handle_axis(notch()));
    assert_eq!(h.state.desktops.current, 1);
}

#[test]
fn in_lock_mode_the_remote_hot_corner_opens_the_overview_and_the_physical_one_does_not() {
    let (mut h, _portal, _lock) = in_lock_mode();
    let (_c, _) = remote_window(&mut h);
    h.state.config.hot_corner = crate::config::HotCorner::TopLeft;
    h.pointer_to(5.0, 5.0);
    h.pointer_to(0.0, 0.0);
    assert!(h.state.overview.view.is_none());
    h.remote_pointer_to(1925.0, 5.0);
    h.remote_pointer_to(1920.0, 0.0);
    assert!(
        h.state.overview.view.is_some(),
        "the remote side's corner opens it"
    );
}

#[test]
fn in_lock_mode_a_remote_click_reaches_the_window_and_a_physical_one_the_lock() {
    let (mut h, _portal, _lock) = in_lock_mode();
    let (mut c, _) = remote_window(&mut h);
    h.remote_click(2200.0, 300.0);
    h.roundtrip(&mut c);
    assert_eq!(c.side.buttons, [(BTN_LEFT, true), (BTN_LEFT, false)]);
    click_lock_cancel(&mut h);
    h.roundtrip(&mut c);
    assert_eq!(lock_input(&h), "", "Cancel cleared the field");
    assert_eq!(c.side.buttons.len(), 2, "the window heard no more");
}

#[test]
fn in_lock_mode_the_remote_power_key_asks_the_desktop_and_the_physical_one_does_not() {
    let mut config = Config::default();
    config.power.power_button = PowerAction::Ask;
    let (mut h, _portal, _lock) = in_lock_mode_with(config.clone());
    h.key(POWER, true);
    h.key(POWER, false);
    assert!(
        h.state.active_dialog_id().is_none(),
        "the console stays locked"
    );
    h.remote_key(POWER, true);
    h.remote_key(POWER, false);
    assert!(
        h.state.active_dialog_id().is_some(),
        "the Turn Off dialog opens"
    );
}

#[test]
fn in_lock_mode_a_tablet_pad_chord_types_into_the_console_and_the_client_hears_nothing() {
    let (mut h, _portal, _lock) = in_lock_mode();
    let (mut c, _) = remote_window(&mut h);
    let codes = h.state.pad_chord_down(0, crate::keys::KEY_a);
    h.state.pad_chord_up(codes);
    h.roundtrip(&mut c);
    assert_eq!(lock_input(&h), "a");
    assert!(c.side.key_log.is_empty(), "heard {:?}", c.side.key_log);
}

#[test]
fn a_mouse_keys_direction_held_at_lock_mode_start_stops_moving_the_remote_pointer() {
    let mut config = Config::default();
    config.accessibility.mouse_keys = true;
    config.accessibility.mouse_keys_num_lock = false;
    let mut h = Harness::with_config(config);
    h.output("eDP-1", 1920, 1080);
    h.state.mousekeys_configured();
    h.pointer_to(960.0, 540.0);
    h.key(KP6, true);
    let (_portal, _lock) = lock_mode(&mut h);
    let at = h.state.pointer_location();
    h.run_for(600);
    assert_eq!(h.state.pointer_location(), at, "the remote pointer stays");
}

#[test]
fn a_sticky_modifier_latched_at_lock_mode_start_never_reaches_the_remote() {
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    h.state.config.accessibility.sticky_keys = true;
    h.state.aids_configured();
    h.key(SHIFT, true);
    h.key(SHIFT, false);
    assert_ne!(h.state.xkb_mask().unwrap().0.latched, 0, "Shift latched");
    let (_portal, _lock) = lock_mode(&mut h);
    let seat = h.state.seat.get_keyboard().unwrap();
    assert!(!seat.modifier_state().shift, "the seat holds no Shift");
    let (mut c, _) = remote_window(&mut h);
    h.remote_key(A, true);
    h.remote_key(A, false);
    h.roundtrip(&mut c);
    assert_eq!(keys_heard(&c), [(A, true), (A, false)]);
    assert!(
        !(c.side.key_log.iter()).any(|k| matches!(k, KeyHeard::Mods(m) if m & 1 != 0)),
        "no Shift reaches the window: {:?}",
        c.side.key_log
    );
}

#[test]
fn a_button_held_under_the_console_lock_is_released_when_lock_mode_starts() {
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    crate::lockview::lock(&mut h.state);
    h.pointer_to(100.0, 100.0);
    h.button(BTN_LEFT, true);
    let (_portal, _lock) = lock_mode(&mut h);
    h.button(BTN_LEFT, false);
    let (mut c, _) = remote_window(&mut h);
    h.remote_click(2200.0, 300.0);
    h.roundtrip(&mut c);
    assert_eq!(c.side.buttons, [(BTN_LEFT, true), (BTN_LEFT, false)]);
}
