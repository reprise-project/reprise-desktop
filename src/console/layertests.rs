use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;

use super::locktests::hunter2;
use super::remote_side;
use crate::harness::{Harness, LockScreen, TestClient};

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
