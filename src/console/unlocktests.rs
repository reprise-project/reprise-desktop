use reis::event::EiEvent;
use smithay::backend::input::{ButtonState, InputTime};

use super::inputtests::keys_heard;
use super::locktests::{ENTER, HUNTER2, hunter2, settle, sh, tap};
use super::remote_side;
use crate::config::{Action, Config};
use crate::desktops::Step;
use crate::harness::{EiPeer, Harness, Heard, KeyHeard, LockScreen, TestClient, Toplevel};
use crate::input::{BTN_LEFT, Interaction};
use crate::inputproto::InputSource;
use crate::inputproto::proto::reprise_lock_screen_v1::Reason;
use crate::raster::Rect;
use crate::window::WindowId;

const SHIFT: u32 = 42;
const A: u32 = 30;
const EDP: Rect = Rect::new(0, 0, 1920, 1080);

/// eDP-1 with the password "hunter2" and no fingerprint conversation; no lock mode yet.
fn computer() -> Harness {
    let mut h = Harness::with_config(Config::default());
    h.output("eDP-1", 1920, 1080);
    hunter2(&mut h);
    h
}

fn begin(h: &mut Harness) -> (TestClient, LockScreen) {
    let mut portal = h.connect_portal();
    let lock = portal.lock_screen();
    lock.set_layout(&[(0, 0, 1280, 720, 100)]);
    h.roundtrip(&mut portal);
    assert!(remote_side(&h.state));
    (portal, lock)
}

/// The password typed on the computer's own keyboard, its check run.
fn unlock_at_the_computer(h: &mut Harness) {
    tap(h, &HUNTER2);
    tap(h, &[ENTER]);
    settle(h);
}

fn ended(lock: &LockScreen) -> Vec<u32> {
    (lock.heard().into_iter())
        .filter_map(|e| match e {
            Heard::Ended(r) => Some(r),
            _ => None,
        })
        .collect()
}

fn frame(h: &Harness, w: WindowId) -> Rect {
    h.state.win(w).expect("the window").frame_rect()
}

fn on_edp(h: &Harness, w: WindowId) -> bool {
    let r = frame(h, w);
    EDP.contains(r.x + r.width / 2, r.y + r.height / 2)
}

fn centre(h: &Harness, w: WindowId) -> (f64, f64) {
    let r = frame(h, w);
    (f64::from(r.x + r.width / 2), f64::from(r.y + r.height / 2))
}

fn mapped(h: &mut Harness, c: &mut TestClient, x: i32, y: i32) -> Toplevel {
    let t = h.map_toplevel(c, "test.app", 400, 300);
    h.state.move_to(t.id, x, y);
    h.roundtrip(c);
    t
}

fn keyboard_window(h: &Harness) -> Option<WindowId> {
    let s = h.state.seat.get_keyboard()?.current_focus()?;
    h.state.window_for_surface(&s)
}

fn assert_lock_mode_over(h: &Harness) {
    assert!(!remote_side(&h.state), "lock mode ends");
    assert!(
        crate::virtualout::remote_outputs(&h.state).is_empty(),
        "the remote screens go"
    );
}

#[test]
fn a_console_unlock_mid_drag_cancels_it_and_returns_the_windows() {
    let mut h = computer();
    let mut c = h.connect();
    let t = mapped(&mut h, &mut c, 100, 100);
    let (mut portal, lock) = begin(&mut h);
    assert!(!on_edp(&h, t.id), "the window is on REMOTE-1");
    let (x, y) = centre(&h, t.id);
    h.remote_click(x, y);
    h.remote_key(A, true);
    let w = h.state.win(t.id).unwrap();
    let (cap, wx, wy) = (w.geom.caption, w.x, w.y);
    let (cx, cy) = (
        f64::from(wx + cap.x + cap.width / 2),
        f64::from(wy + cap.y + cap.height / 2),
    );
    h.remote_pointer_to(cx, cy);
    h.from(InputSource::Remote, |s| {
        s.handle_button(BTN_LEFT, ButtonState::Pressed, InputTime::now())
    });
    h.remote_pointer_to(cx + 60.0, cy + 40.0);
    assert!(matches!(h.state.interaction, Interaction::Move { .. }));
    h.roundtrip(&mut c);
    c.side.key_log.clear();
    unlock_at_the_computer(&mut h);
    h.roundtrip(&mut portal);
    h.roundtrip(&mut c);
    assert!(!h.state.console_locked());
    assert_lock_mode_over(&h);
    assert_eq!(ended(&lock), [Reason::ConsoleUnlocked as u32]);
    assert!(
        matches!(h.state.interaction, Interaction::Idle),
        "the move is cancelled"
    );
    assert!(on_edp(&h, t.id), "the window is back on eDP-1");
    assert_eq!(keys_heard(&c), [(A, false)], "the remote's key is released");
    let keyboard = h.state.seat.get_keyboard().unwrap();
    assert!(keyboard.pressed_keys().is_empty(), "the seat holds no key");
    assert!(h.state.key_input.held.is_empty(), "no key is held back");
    let pointer = h.state.seat.get_pointer().unwrap();
    assert!(!pointer.is_grabbed(), "no grab is left");
    let at = frame(&h, t.id);
    h.from(InputSource::Remote, |s| {
        s.handle_button(BTN_LEFT, ButtonState::Released, InputTime::now())
    });
    assert!(matches!(h.state.interaction, Interaction::Idle));
    assert_eq!(frame(&h, t.id), at, "the late release moves nothing");
}

#[test]
fn a_console_unlock_sends_ended_to_every_lock_screen_object() {
    let mut h = computer();
    let (mut portal, first) = begin(&mut h);
    let second = portal.lock_screen();
    second.set_layout(&[(0, 0, 1280, 720, 100)]);
    h.roundtrip(&mut portal);
    unlock_at_the_computer(&mut h);
    h.roundtrip(&mut portal);
    let unlocked = Reason::ConsoleUnlocked as u32;
    assert_eq!(ended(&first), [unlocked]);
    assert_eq!(ended(&second), [unlocked]);
    assert_lock_mode_over(&h);
    first.object.destroy();
    second.object.destroy();
    h.roundtrip(&mut portal);
    assert!(!remote_side(&h.state), "the objects going changes nothing");
}

#[test]
fn after_a_console_unlock_the_console_keyboard_reaches_windows_again() {
    let mut h = computer();
    let mut c = h.connect();
    let t = mapped(&mut h, &mut c, 100, 100);
    h.state.focus_window(t.id);
    let (_portal, _lock) = begin(&mut h);
    unlock_at_the_computer(&mut h);
    assert_eq!(keyboard_window(&h), Some(t.id));
    h.roundtrip(&mut c);
    c.side.key_log.clear();
    tap(&mut h, &[A]);
    h.roundtrip(&mut c);
    assert_eq!(keys_heard(&c), [(A, true), (A, false)]);
}

/// A fingerprint read at the computer, its conversation run.
fn finger_at_the_computer(h: &mut Harness) {
    h.state.lock.helpers.finger = |_| Some(sh("echo '= 0'"));
    crate::lockview::start_dialog_finger(&mut h.state);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while h.state.console_locked() {
        assert!(std::time::Instant::now() < end, "the fingerprint is read");
        h.run_for(5);
    }
}

type UnlockPath = (&'static str, fn(&mut Harness));

#[test]
fn every_console_unlock_in_lock_mode_ends_lock_mode() {
    let paths: [UnlockPath; 3] = [
        ("the password", unlock_at_the_computer),
        ("the fingerprint", finger_at_the_computer),
        ("logind", |h| crate::lockview::unlock(&mut h.state)),
    ];
    for (path, unlock) in paths {
        let mut h = computer();
        let (mut portal, lock) = begin(&mut h);
        unlock(&mut h);
        h.roundtrip(&mut portal);
        assert!(!h.state.console_locked(), "{path} unlocks the console");
        assert!(!remote_side(&h.state), "{path} ends lock mode");
        assert_eq!(ended(&lock), [Reason::ConsoleUnlocked as u32], "{path}");
    }
}

#[test]
fn the_computers_input_stays_off_clients_in_lock_mode_even_with_the_console_unlocked() {
    let mut h = computer();
    let (_portal, _lock) = begin(&mut h);
    // Breaks "lock mode means the console is locked" on purpose: each gate checks the source.
    h.state.lock.console = None;
    h.from(InputSource::Physical, |s| {
        assert!(
            s.console_holds_input(),
            "the console keeps the computer's input"
        );
        assert!(!s.clients_may_have_touch(), "no finger reaches a client");
        assert!(!s.clients_may_have_tool(), "no pen reaches a client");
        s.pad_action(&Action::Desktop(Step::Next));
    });
    assert_eq!(h.state.desktops.current, 0, "a pad action runs nothing");
    h.from(InputSource::Remote, |s| {
        assert!(!s.console_holds_input());
        assert!(
            s.clients_may_have_touch(),
            "the remote's fingers reach clients"
        );
    });
}

#[test]
fn keys_held_at_the_computer_across_a_console_unlock_reach_no_client() {
    let mut h = computer();
    let mut c = h.connect();
    let t = mapped(&mut h, &mut c, 100, 100);
    h.state.focus_window(t.id);
    let (_portal, _lock) = begin(&mut h);
    h.key(SHIFT, true);
    crate::lockview::unlock(&mut h.state);
    assert_eq!(keyboard_window(&h), Some(t.id));
    h.roundtrip(&mut c);
    c.side.key_log.clear();
    h.key(SHIFT, false);
    tap(&mut h, &[A]);
    h.roundtrip(&mut c);
    assert_eq!(
        keys_heard(&c),
        [(A, true), (A, false)],
        "no stray Shift release"
    );
    assert!(
        !(c.side.key_log.iter()).any(|k| matches!(k, KeyHeard::Mods(m) if m & 1 != 0)),
        "no Shift is stuck: {:?}",
        c.side.key_log
    );
}

#[test]
fn a_console_unlock_focuses_the_window_the_remote_side_was_using() {
    let mut h = computer();
    let mut c = h.connect();
    let a = mapped(&mut h, &mut c, 100, 100);
    let b = mapped(&mut h, &mut c, 900, 500);
    h.state.focus_window(a.id);
    let (_portal, _lock) = begin(&mut h);
    let (x, y) = centre(&h, b.id);
    h.remote_click(x, y);
    assert_eq!(keyboard_window(&h), Some(b.id));
    unlock_at_the_computer(&mut h);
    assert_eq!(
        keyboard_window(&h),
        Some(b.id),
        "not the window from before lock mode"
    );
    assert_eq!(h.state.focused, Some(b.id));
}

#[test]
fn a_console_unlock_under_the_remote_lock_focuses_what_the_remote_side_had() {
    let mut h = computer();
    let mut c = h.connect();
    let a = mapped(&mut h, &mut c, 100, 100);
    let b = mapped(&mut h, &mut c, 900, 500);
    h.state.focus_window(a.id);
    let (_portal, _lock) = begin(&mut h);
    let (x, y) = centre(&h, b.id);
    h.remote_click(x, y);
    crate::lockview::lock_remote(&mut h.state);
    assert_eq!(keyboard_window(&h), None);
    unlock_at_the_computer(&mut h);
    assert_eq!(keyboard_window(&h), Some(b.id));
}

#[test]
fn a_console_unlock_with_nothing_focused_remotely_gives_back_the_console_focus() {
    let mut h = computer();
    let mut c = h.connect();
    let a = mapped(&mut h, &mut c, 100, 100);
    let _b = mapped(&mut h, &mut c, 900, 500);
    h.state.focus_window(a.id);
    let (_portal, _lock) = begin(&mut h);
    assert_eq!(keyboard_window(&h), None);
    unlock_at_the_computer(&mut h);
    assert_eq!(keyboard_window(&h), Some(a.id));
}

/// Sends a key, an absolute motion to (`x`, `y`) and a click from each of `peer`'s devices.
fn ei_type_and_click(h: &mut Harness, peer: &EiPeer, x: f32, y: f32) {
    use reis::ei;
    let devices = (peer.heard.iter()).filter_map(|e| match e {
        EiEvent::DeviceResumed(r) => Some(r.device.clone()),
        _ => None,
    });
    for d in devices {
        d.device().start_emulating(0, 0);
        if let Some(k) = d.interface::<ei::Keyboard>() {
            k.key(A, ei::keyboard::KeyState::Press);
            k.key(A, ei::keyboard::KeyState::Released);
        }
        if let Some(p) = d.interface::<ei::PointerAbsolute>() {
            p.motion_absolute(x, y);
        }
        if let Some(b) = d.interface::<ei::Button>() {
            b.button(BTN_LEFT, ei::button::ButtonState::Press);
            b.button(BTN_LEFT, ei::button::ButtonState::Released);
        }
        d.device().frame(0, 0);
    }
    let _ = peer.context.flush();
    h.run_for(5);
}

#[test]
fn a_console_unlock_cuts_every_remote_desktop_session_off() {
    let mut h = computer();
    let mut c = h.connect();
    let t = mapped(&mut h, &mut c, 100, 100);
    h.state.focus_window(t.id);
    let (_portal, _lock) = begin(&mut h);
    let mut peer = h.ei_remote_session();
    h.ei_hear(&mut peer);
    let keys = h.state.inputproto.keys.create();
    let (x, y) = centre(&h, t.id);
    h.remote_click(x, y);
    h.state.keys_key(keys, SHIFT, true);
    unlock_at_the_computer(&mut h);
    assert_lock_mode_over(&h);
    assert!(
        h.state.inputproto.eis.get(peer.id).is_none(),
        "the session's EIS client is gone"
    );
    h.ei_hear(&mut peer);
    assert!(
        (peer.heard.iter()).any(|e| matches!(e, EiEvent::Disconnected(_))),
        "the session hears it is disconnected"
    );
    assert_eq!(keyboard_window(&h), Some(t.id));
    h.roundtrip(&mut c);
    c.side.key_log.clear();
    c.side.buttons.clear();
    let pointer = h.state.pointer_location();
    let (x, y) = centre(&h, t.id);
    ei_type_and_click(&mut h, &peer, x as f32, y as f32);
    h.state.keys_key(keys, SHIFT, false);
    h.state.keys_key(keys, A, true);
    h.state.keys_key(keys, A, false);
    h.roundtrip(&mut c);
    assert_eq!(keys_heard(&c), [], "no remote key reaches the window");
    assert_eq!(c.side.buttons, [], "no remote click reaches the window");
    assert_eq!(h.state.pointer_location(), pointer, "the pointer stays put");
}

#[test]
fn a_sender_asked_for_before_the_portal_heard_the_console_unlock_is_refused() {
    use crate::harness::input_client::reprise_input_v1::Kind;
    use crate::inputproto::{DEVICE_KEYBOARD, DEVICE_POINTER};
    use std::io::Read;
    let mut h = computer();
    let (mut portal, lock) = begin(&mut h);
    unlock_at_the_computer(&mut h);
    assert_lock_mode_over(&h);
    let mut theirs = portal.connect_eis(Kind::RemoteSession, DEVICE_KEYBOARD | DEVICE_POINTER);
    h.roundtrip(&mut portal);
    assert_eq!(ended(&lock), [Reason::ConsoleUnlocked as u32]);
    assert!(h.state.inputproto.eis.is_empty(), "no sender is made");
    theirs.set_nonblocking(true).expect("nonblocking");
    assert_eq!(
        theirs.read(&mut [0; 64]).ok(),
        Some(0),
        "the libei client's socket is closed"
    );
    lock.object.destroy();
    h.roundtrip(&mut portal);
    let _theirs = portal.connect_eis(Kind::RemoteSession, DEVICE_KEYBOARD | DEVICE_POINTER);
    h.roundtrip(&mut portal);
    assert!(
        !h.state.inputproto.eis.is_empty(),
        "a sender asked for after the object went is made"
    );
}

#[test]
fn a_locker_taking_the_lock_mid_run_cuts_every_sender_off() {
    let mut h = computer();
    let (mut portal, lock) = begin(&mut h);
    let mut peer = h.ei_remote_session();
    h.ei_hear(&mut peer);
    let keys = h.state.inputproto.keys.create();
    let mut locker = h.connect();
    let (_locked, _surface) = h.map_lock_surface(&mut locker);
    h.roundtrip(&mut portal);
    assert_lock_mode_over(&h);
    assert_eq!(ended(&lock), [Reason::Replaced as u32]);
    assert!(
        h.state.inputproto.eis.get(peer.id).is_none(),
        "the session's EIS client is gone"
    );
    h.ei_hear(&mut peer);
    assert!(
        (peer.heard.iter()).any(|e| matches!(e, EiEvent::Disconnected(_))),
        "the session hears it is disconnected"
    );
    assert!(
        !h.state.inputproto.keys.exists(keys),
        "the keys object is revoked"
    );
}

#[test]
fn a_console_unlock_cuts_a_permitted_remote_desktop_session_off() {
    use crate::eis::Kind;
    use crate::inputproto::{DEVICE_KEYBOARD, DEVICE_POINTER};
    let mut h = computer();
    let mut c = h.connect();
    let t = mapped(&mut h, &mut c, 100, 100);
    h.state.focus_window(t.id);
    let (_portal, _lock) = begin(&mut h);
    let mut peer = h.ei_keyboard_as(Kind::Sender, false, DEVICE_KEYBOARD | DEVICE_POINTER);
    h.ei_hear(&mut peer);
    let capture = h.ei_keyboard(Kind::Receiver);
    unlock_at_the_computer(&mut h);
    assert_lock_mode_over(&h);
    assert!(
        h.state.inputproto.eis.get(peer.id).is_none(),
        "the sender is gone"
    );
    assert!(
        h.state.inputproto.eis.get(capture.id).is_some(),
        "the input capture receiver stays"
    );
    h.ei_hear(&mut peer);
    assert!(
        (peer.heard.iter()).any(|e| matches!(e, EiEvent::Disconnected(_))),
        "the sender hears it is disconnected"
    );
    h.roundtrip(&mut c);
    c.side.key_log.clear();
    c.side.buttons.clear();
    let pointer = h.state.pointer_location();
    let (x, y) = centre(&h, t.id);
    ei_type_and_click(&mut h, &peer, x as f32, y as f32);
    h.roundtrip(&mut c);
    assert_eq!(
        keys_heard(&c),
        [],
        "no key from the sender reaches the window"
    );
    assert_eq!(
        c.side.buttons,
        [],
        "no click from the sender reaches the window"
    );
    assert_eq!(h.state.pointer_location(), pointer, "the pointer stays put");
}

#[test]
fn a_finished_lockers_unlock_leaves_the_console_lock_and_lock_mode_up() {
    let mut h = computer();
    let mut locker = h.connect();
    let (lock, _surface) = h.map_lock_surface(&mut locker);
    h.run_for(2300);
    h.roundtrip(&mut locker);
    crate::lockview::unlock(&mut h.state);
    h.roundtrip(&mut locker);
    assert!(!h.state.console_locked());
    let (mut portal, screen) = begin(&mut h);
    assert!(h.state.console_locked());
    lock.unlock_and_destroy();
    h.roundtrip(&mut locker);
    h.roundtrip(&mut portal);
    assert!(h.state.console_locked(), "the console stays locked");
    assert!(remote_side(&h.state), "lock mode goes on");
    assert!(ended(&screen).is_empty(), "the object hears no end");
}

#[test]
fn a_console_unlock_ends_lock_mode_with_no_lock_screen_object_left() {
    let mut h = computer();
    let (_portal, _lock) = begin(&mut h);
    h.state.console.objects.clear();
    unlock_at_the_computer(&mut h);
    assert!(!h.state.console_locked());
    assert_lock_mode_over(&h);
}

#[test]
fn a_console_unlock_after_the_remote_side_clicked_the_desktop_focuses_the_desktop() {
    let mut h = computer();
    let mut c = h.connect();
    let a = mapped(&mut h, &mut c, 100, 100);
    h.state.focus_window(a.id);
    let (_portal, _lock) = begin(&mut h);
    let remote = crate::virtualout::remote_outputs(&h.state)[0].clone();
    let r = h.state.layout_box(Some(&remote));
    h.remote_click(f64::from(r.x + r.width - 20), f64::from(r.y + 20));
    assert_eq!(h.state.focused, None, "the desktop took the keyboard");
    unlock_at_the_computer(&mut h);
    assert_eq!(keyboard_window(&h), None);
    assert_eq!(h.state.focused, None, "no window is focused");
}

#[test]
fn a_session_that_lost_its_seat_takes_a_client_in_lock_mode_and_brings_its_windows_back() {
    use super::homes::RelRect;
    let mut h = computer();
    let (ours, _daemon) = std::os::unix::net::UnixStream::pair().unwrap();
    crate::loginlink::install(&mut h.state, crate::loginlink::LoginLink::from_stream(ours));
    let edp = h.state.first_output().expect("eDP-1");
    let home_area = h.state.usable_area(Some(&edp));
    let mut c = h.connect();
    let t = h.map_toplevel(
        &mut c,
        "test.app",
        home_area.width / 2,
        home_area.height / 2,
    );
    let (x, y) = (home_area.width / 4, home_area.height / 4);
    h.state.move_to(t.id, home_area.x + x, home_area.y + y);
    h.roundtrip(&mut c);
    let home = frame(&h, t.id);
    let rel = RelRect::of(home, home_area);

    crate::backend::pause_session(&mut h.state);
    assert!(h.state.console_locked(), "a session off the seat locks");
    let mut portal = h.connect_portal();
    let lock = portal.lock_screen();
    lock.set_layout(&[(0, 0, 1600, 900, 100)]);
    h.roundtrip(&mut portal);
    h.roundtrip(&mut c);
    assert!(
        remote_side(&h.state),
        "the client takes the session in lock mode"
    );
    assert!(h.state.console_locked(), "the console stays locked");
    let remote = crate::virtualout::remote_outputs(&h.state);
    let want = rel.at(h.state.usable_area(remote.first()));
    let got = frame(&h, t.id);
    assert!(
        (got.x - want.x).abs() <= 1 && (got.y - want.y).abs() <= 1,
        "{got:?} is at {want:?}'s place on the Remote output"
    );
    assert_eq!(
        (got.width, got.height),
        (home.width, home.height),
        "the size stays"
    );

    crate::power::report(&mut h.state, crate::power::Report::Unlock);
    crate::backend::activate_session(&mut h.state);
    h.roundtrip(&mut portal);
    h.roundtrip(&mut c);
    assert!(!h.state.console_locked());
    assert_lock_mode_over(&h);
    assert_eq!(frame(&h, t.id), home, "the window is back where it was");
}
