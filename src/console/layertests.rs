use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::Layer, zwlr_layer_surface_v1::KeyboardInteractivity,
};

use super::inputtests::keys_heard;
use super::locktests::{ENTER, HUNTER2, hunter2, settle, tap};
use super::remote_side;
use super::tests::under_lock_layer;
use crate::harness::{Harness, LockScreen, TestClient, Toplevel};

const A: u32 = 30;

/// eDP-1 at 0,0 with the password "hunter2"; no lock mode yet.
fn computer() -> Harness {
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    hunter2(&mut h);
    h
}

/// Lock mode with REMOTE-1 at 1920,0 and REMOTE-2 at 3200,0, 1280x720 each.
fn begin(h: &mut Harness) -> (TestClient, LockScreen) {
    let mut portal = h.connect_portal();
    let lock = portal.lock_screen();
    lock.set_layout(&[(0, 0, 1280, 720, 100), (1280, 0, 1280, 720, 100)]);
    h.roundtrip(&mut portal);
    assert!(remote_side(&h.state));
    (portal, lock)
}

/// The output and size of the layer surface made last.
fn last_layer(h: &Harness) -> (String, (i32, i32)) {
    let l = h
        .state
        .layers_shell
        .surfaces
        .last()
        .expect("a layer surface");
    let size = l.surface.with_pending_state(|s| s.size.unwrap_or_default());
    (l.output.name(), (size.w, size.h))
}

fn last_layer_wl(h: &Harness) -> WlSurface {
    let l = h
        .state
        .layers_shell
        .surfaces
        .last()
        .expect("a layer surface");
    l.surface.wl_surface().clone()
}

fn keyboard(h: &Harness) -> Option<WlSurface> {
    h.state.seat.get_keyboard()?.current_focus()
}

fn pointer(h: &Harness) -> Option<WlSurface> {
    h.state
        .seat
        .get_pointer()?
        .current_focus()?
        .surface()
        .cloned()
}

/// A window on REMOTE-1 the remote side clicked, so it holds the keyboard.
fn remote_window(h: &mut Harness, c: &mut TestClient) -> (Toplevel, WlSurface) {
    let t = h.map_toplevel(c, "test.app", 400, 300);
    h.state.move_to(t.id, 2100, 200);
    h.roundtrip(c);
    h.remote_click(2300.0, 350.0);
    h.roundtrip(c);
    let s = h.state.win(t.id).unwrap().wl_surface().clone();
    assert_eq!(
        keyboard(h),
        Some(s.clone()),
        "the window takes the keyboard"
    );
    (t, s)
}

#[test]
fn in_lock_mode_a_layer_surface_naming_a_console_output_opens_under_the_remote_pointer() {
    let mut h = computer();
    let mut early = h.connect();
    let edp = h.bound_output(&early, "eDP-1");
    let (_portal, _lock) = begin(&mut h);
    h.remote_pointer_to(3500.0, 300.0);
    h.map_exclusive_layer_on(&mut early, Some(&edp), Layer::Top);
    assert_eq!(last_layer(&h), ("REMOTE-2".into(), (1280, 720)));

    let mut late = h.connect();
    let remote_1 = h.bound_output(&late, "REMOTE-1");
    h.map_exclusive_layer_on(&mut late, Some(&remote_1), Layer::Top);
    assert_eq!(
        last_layer(&h).0,
        "REMOTE-1",
        "a named Remote output keeps it"
    );
}

#[test]
fn in_lock_mode_a_layer_surface_naming_no_output_opens_under_the_remote_pointer() {
    let mut h = computer();
    let (_portal, _lock) = begin(&mut h);
    h.remote_pointer_to(3500.0, 300.0);
    let mut c = h.connect();
    h.map_exclusive_layer(&mut c, Layer::Top);
    assert_eq!(last_layer(&h).0, "REMOTE-2");
}

#[test]
fn a_layer_surface_naming_a_console_output_keeps_it_outside_lock_mode() {
    let mut h = computer();
    h.output("HDMI-A-1", 1280, 1024);
    h.pointer_to(2000.0, 100.0);
    let mut c = h.connect();
    let edp = h.bound_output(&c, "eDP-1");
    h.map_exclusive_layer_on(&mut c, Some(&edp), Layer::Top);
    assert_eq!(last_layer(&h), ("eDP-1".into(), (1920, 1080)));
}

#[test]
fn a_console_layer_surface_from_before_lock_mode_never_takes_the_remote_keyboard() {
    let mut h = computer();
    let mut console = h.connect();
    let (wl, console_layer) = h.map_exclusive_layer_on(&mut console, None, Layer::Overlay);
    let held = last_layer_wl(&h);
    assert_eq!(keyboard(&h), Some(held.clone()), "it holds the keyboard");
    let (_portal, _lock) = begin(&mut h);
    assert_ne!(keyboard(&h), Some(held.clone()), "lock mode takes it");

    let mut c = h.connect();
    let (_, window) = remote_window(&mut h, &mut c);
    let mut applet = h.connect();
    h.map_flyout(&mut applet);
    applet.hang_up();
    h.dispatch();
    assert_eq!(
        keyboard(&h),
        Some(window.clone()),
        "a remote flyout closing hands the keyboard back to the window"
    );

    console_layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    wl.commit();
    h.roundtrip(&mut console);
    console_layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    wl.commit();
    h.roundtrip(&mut console);
    assert_eq!(
        keyboard(&h),
        Some(window.clone()),
        "asking for the keyboard again takes nothing"
    );

    console.side.key_log.clear();
    c.side.key_log.clear();
    h.remote_key(A, true);
    h.remote_key(A, false);
    h.roundtrip(&mut c);
    h.roundtrip(&mut console);
    assert!(
        console.side.key_log.is_empty(),
        "{:?}",
        console.side.key_log
    );
    assert_eq!(keys_heard(&c), [(A, true), (A, false)]);
}

#[test]
fn a_console_panels_menu_never_takes_the_remote_keyboard_with_its_grab() {
    let mut h = computer();
    let mut console = h.connect();
    let panel = h.map_panel(&mut console);
    assert_eq!(last_layer(&h).0, "eDP-1");
    let (_portal, _lock) = begin(&mut h);
    let mut c = h.connect();
    let (_, window) = remote_window(&mut h, &mut c);
    let menu = h.panel_popup(&mut console, &panel, 120, 90, true);
    assert_eq!(
        keyboard(&h),
        Some(window.clone()),
        "the menu's grab takes nothing"
    );

    console.side.key_log.clear();
    c.side.key_log.clear();
    h.remote_key(A, true);
    h.remote_key(A, false);
    h.roundtrip(&mut c);
    h.roundtrip(&mut console);
    assert!(
        console.side.key_log.is_empty(),
        "{:?}",
        console.side.key_log
    );
    assert_eq!(keys_heard(&c), [(A, true), (A, false)]);

    console.hide_popup(&menu);
    h.roundtrip(&mut console);
    assert_eq!(
        keyboard(&h),
        Some(window),
        "the menu closing hands its panel nothing"
    );
}

#[test]
fn a_console_layer_surface_mapped_under_the_console_lock_leaves_the_remote_side_the_keyboard() {
    let mut h = computer();
    crate::lockview::lock(&mut h.state);
    let mut console = h.connect();
    h.map_exclusive_layer(&mut console, Layer::Overlay);
    assert_eq!(last_layer(&h).0, "eDP-1");
    let (_portal, _lock) = begin(&mut h);
    let mut c = h.connect();
    remote_window(&mut h, &mut c);
}

#[test]
fn a_console_layer_surface_from_before_lock_mode_gets_the_keyboard_back_when_it_ends() {
    let mut h = computer();
    let mut console = h.connect();
    h.map_exclusive_layer(&mut console, Layer::Overlay);
    let held = last_layer_wl(&h);
    let (_portal, _lock) = begin(&mut h);
    tap(&mut h, &HUNTER2);
    tap(&mut h, &[ENTER]);
    settle(&mut h);
    assert!(!remote_side(&h.state), "the console unlock ends lock mode");
    assert!(console.side.layers_closed.is_empty(), "it stays open");
    assert_eq!(last_layer(&h).0, "eDP-1");
    assert_eq!(keyboard(&h), Some(held));
}

#[test]
fn a_console_layer_surface_gets_the_keyboard_back_over_the_window_the_remote_side_used() {
    let mut h = computer();
    let mut console = h.connect();
    h.map_exclusive_layer(&mut console, Layer::Overlay);
    let held = last_layer_wl(&h);
    let (_portal, _lock) = begin(&mut h);
    let mut c = h.connect();
    remote_window(&mut h, &mut c);
    tap(&mut h, &HUNTER2);
    tap(&mut h, &[ENTER]);
    settle(&mut h);
    assert!(!remote_side(&h.state), "the console unlock ends lock mode");
    assert_eq!(keyboard(&h), Some(held));
}

#[test]
fn a_console_layer_surface_gets_no_remote_pointer() {
    let mut h = computer();
    h.pointer_to(100.0, 100.0);
    let mut console = h.connect();
    h.map_exclusive_layer(&mut console, Layer::Overlay);
    let held = last_layer_wl(&h);
    assert_eq!(pointer(&h), Some(held.clone()), "it has the pointer");
    let (_portal, _lock) = begin(&mut h);
    assert_ne!(pointer(&h), Some(held.clone()), "lock mode takes it");
    for (x, y) in [(100.0, 100.0), (1919.0, 500.0), (2000.0, 100.0)] {
        h.remote_pointer_to(x, y);
        assert_ne!(pointer(&h), Some(held.clone()), "at {x},{y}");
    }
}

#[test]
fn in_lock_mode_the_console_cover_draws_over_its_layer_surfaces() {
    let mut h = computer();
    let mut console = h.connect();
    h.map_exclusive_layer(&mut console, Layer::Overlay);
    let (_portal, _lock) = begin(&mut h);
    let l = h
        .state
        .layers_shell
        .surfaces
        .last()
        .expect("a layer surface");
    assert!(
        l.mapped && l.output.name() == "eDP-1",
        "it stays mapped on eDP-1"
    );
    let area = reprisescene::output_rect(&h.state.scene.outputs()[0]).unwrap();
    let leaves = h.state.scene.leaves_in(area);
    assert!(
        leaves.first().is_some_and(|&n| under_lock_layer(&h, n)),
        "the frontmost leaf on eDP-1 is the cover"
    );
}
