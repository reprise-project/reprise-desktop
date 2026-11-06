use super::*;
use crate::harness::{Harness, Heard, TestClient};

fn remote_names(h: &Harness) -> Vec<String> {
    (crate::virtualout::remote_outputs(&h.state).iter())
        .map(|o| o.name())
        .collect()
}

fn output_names(h: &Harness) -> Vec<String> {
    h.state.scene.outputs().iter().map(|o| o.name()).collect()
}

fn unplug(h: &mut Harness, o: &Output) {
    h.state.scene.remove_output(o);
    h.state.output_removed(o);
    h.dispatch();
}

fn size_and_scale(o: &Output) -> ((i32, i32), f64) {
    let mode = o.current_mode().expect("a mode");
    (
        (mode.size.w, mode.size.h),
        o.current_scale().fractional_scale(),
    )
}

#[test]
fn the_first_lock_screen_object_creates_its_screens_and_locks_the_console() {
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    let mut portal = h.connect_portal();
    let lock = portal.lock_screen();
    lock.set_layout(&[(0, 0, 2560, 1440, 100), (2560, 0, 1920, 1080, 100)]);
    h.roundtrip(&mut portal);
    assert_eq!(lock.take_screens(), ["REMOTE-1", "REMOTE-2"]);
    assert_eq!(remote_names(&h), ["REMOTE-1", "REMOTE-2"]);
    assert!(h.state.is_locked(), "the console is locked");
    assert!(remote_side(&h.state), "the remote side holds the desktop");
    let remotes = crate::virtualout::remote_outputs(&h.state);
    assert_eq!(size_and_scale(&remotes[0]), ((2560, 1440), 1.0));
    assert_eq!(remotes[0].current_location(), (1920, 0).into());
    assert_eq!(remotes[1].current_location(), (4480, 0).into());
    assert_eq!(side_of(&remotes[0]), Side::Remote);
    assert_eq!(side_of(&h.state.scene.outputs()[0]), Side::Console);
}

#[test]
fn lock_mode_lasts_until_the_last_object_goes() {
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    let mut portal = h.connect_portal();
    let first = portal.lock_screen();
    let second = portal.lock_screen();
    first.set_layout(&[(0, 0, 1920, 1080, 100)]);
    second.set_layout(&[(0, 0, 1920, 1080, 100)]);
    h.roundtrip(&mut portal);
    first.object.destroy();
    h.roundtrip(&mut portal);
    assert!(remote_side(&h.state), "the second object holds lock mode");
    assert_eq!(remote_names(&h), ["REMOTE-1"]);
    second.object.destroy();
    h.roundtrip(&mut portal);
    assert!(!remote_side(&h.state));
    assert!(remote_names(&h).is_empty(), "the remote screens go");
    assert_eq!(output_names(&h), ["eDP-1"]);
    assert!(h.state.is_locked(), "the console lock stays");
}

#[test]
fn a_second_objects_layout_is_ignored() {
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    let mut portal = h.connect_portal();
    let first = portal.lock_screen();
    first.set_layout(&[(0, 0, 1920, 1080, 100)]);
    h.roundtrip(&mut portal);
    first.take_screens();
    let second = portal.lock_screen();
    second.set_layout(&[(0, 0, 1280, 720, 100), (1280, 0, 1280, 720, 100)]);
    h.roundtrip(&mut portal);
    assert_eq!(
        second.take_screens(),
        ["REMOTE-1"],
        "it hears the screens there are"
    );
    assert!(first.heard().is_empty(), "nothing changed for the first");
    assert_eq!(remote_names(&h), ["REMOTE-1"]);
    let remote = &crate::virtualout::remote_outputs(&h.state)[0];
    assert_eq!(size_and_scale(remote), ((1920, 1080), 1.0));
    second.set_layout(&[(0, 0, 1280, 720, 100)]);
    h.roundtrip(&mut portal);
    assert_eq!(size_and_scale(remote), ((1920, 1080), 1.0));
}

#[test]
fn the_first_objects_new_layout_resizes_adds_and_removes_screens() {
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    let mut portal = h.connect_portal();
    let first = portal.lock_screen();
    let second = portal.lock_screen();
    first.set_layout(&[(0, 0, 1920, 1080, 100)]);
    second.set_layout(&[(0, 0, 1920, 1080, 100)]);
    h.roundtrip(&mut portal);
    first.take_screens();
    second.take_screens();
    first.set_layout(&[(0, 0, 2560, 1440, 150), (2560, 0, 1920, 1080, 100)]);
    h.roundtrip(&mut portal);
    assert_eq!(first.take_screens(), ["REMOTE-1", "REMOTE-2"]);
    assert_eq!(second.take_screens(), ["REMOTE-1", "REMOTE-2"]);
    let remotes = crate::virtualout::remote_outputs(&h.state);
    assert_eq!(size_and_scale(&remotes[0]), ((2560, 1440), 1.5));
    assert_eq!(remotes[1].current_location(), (1920 + 1707, 0).into());
    first.set_layout(&[(0, 0, 2560, 1440, 150)]);
    h.roundtrip(&mut portal);
    assert_eq!(first.take_screens(), ["REMOTE-1"]);
    assert_eq!(remote_names(&h), ["REMOTE-1"]);
    first.set_layout(&[(0, 0, 2560, 1440, 150)]);
    h.roundtrip(&mut portal);
    assert!(first.take_screens().is_empty(), "an unchanged layout");
}

#[test]
fn a_monitor_hotplug_during_lock_mode_keeps_the_remote_screens_and_adds_no_standby() {
    let mut h = Harness::new();
    let edp = h.output("eDP-1", 1920, 1080);
    crate::standby::rdp_changed(&mut h.state, true);
    let mut app = h.connect();
    let t = h.map_toplevel(&mut app, "test.app", 400, 300);
    let mut portal = h.connect_portal();
    let lock = portal.lock_screen();
    lock.set_layout(&[(0, 0, 1920, 1080, 100)]);
    h.roundtrip(&mut portal);
    let remote = crate::virtualout::remote_outputs(&h.state)[0].clone();
    let at = |h: &Harness| {
        let f = h.state.win(t.id).expect("the window").frame_rect();
        let o = remote.current_location();
        (f.x - o.x, f.y - o.y)
    };
    h.state.move_to(t.id, 1920 + 100, 50);
    let before = at(&h);
    let dp = h.output("DP-2", 1280, 1024);
    h.dispatch();
    assert_eq!(remote_names(&h), ["REMOTE-1"]);
    assert_eq!(at(&h), before, "the window stays put on its remote screen");
    unplug(&mut h, &dp);
    unplug(&mut h, &edp);
    assert_eq!(output_names(&h), ["REMOTE-1"], "no standby screen");
    assert_eq!(at(&h), before);
    assert!(remote_side(&h.state));
}

#[test]
fn lock_mode_is_unavailable_while_a_locker_is_set() {
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    h.state.config.power.locker = "swaylock".into();
    let mut portal = h.connect_portal();
    let lock = portal.lock_screen();
    h.roundtrip(&mut portal);
    assert_eq!(
        lock.heard(),
        [Heard::Ended(2)],
        "ended(unavailable) at once"
    );
    lock.set_layout(&[(0, 0, 1920, 1080, 100)]);
    h.roundtrip(&mut portal);
    assert_eq!(lock.heard(), [Heard::Ended(2)]);
    assert!(remote_names(&h).is_empty());
    assert!(!h.state.is_locked());
    assert!(!remote_side(&h.state));
}

#[test]
fn only_the_portal_may_bind_lock_screen() {
    let mut h = Harness::new();
    let app: TestClient = h.connect();
    assert_eq!(app.global_version("reprise_input_v1"), None);
    let portal = h.connect_portal();
    assert_eq!(portal.global_version("reprise_input_v1"), Some(9));
}

#[test]
fn the_standby_goes_when_lock_mode_begins() {
    let mut h = Harness::new();
    crate::standby::rdp_changed(&mut h.state, true);
    h.dispatch();
    assert_eq!(output_names(&h), ["HEADLESS-1"]);
    let mut portal = h.connect_portal();
    let lock = portal.lock_screen();
    lock.set_layout(&[(0, 0, 1920, 1080, 100)]);
    h.roundtrip(&mut portal);
    assert_eq!(output_names(&h), ["REMOTE-1"]);
    let remote = &h.state.scene.outputs()[0];
    assert_eq!(remote.current_location(), (0, 0).into());
}

#[test]
fn the_standby_comes_back_when_lock_mode_ends_with_no_monitor() {
    let mut h = Harness::new();
    let edp = h.output("eDP-1", 1920, 1080);
    crate::standby::rdp_changed(&mut h.state, true);
    let mut portal = h.connect_portal();
    let lock = portal.lock_screen();
    lock.set_layout(&[(0, 0, 1920, 1080, 100)]);
    h.roundtrip(&mut portal);
    unplug(&mut h, &edp);
    assert_eq!(output_names(&h), ["REMOTE-1"]);
    lock.object.destroy();
    h.roundtrip(&mut portal);
    assert_eq!(output_names(&h), ["HEADLESS-1"], "never zero outputs");
}

#[test]
fn monitors_meeting_edge_to_edge_still_meet_in_layout_units() {
    let m = |x, y, width, height, scale| RemoteMonitor {
        x,
        y,
        width,
        height,
        scale,
    };
    let specs = specs(&[
        m(0, 0, 3840, 2160, 200),
        m(3840, 0, 1920, 1080, 100),
        m(0, 2160, 1280, 1024, 130),
    ]);
    let got: Vec<_> = specs
        .iter()
        .map(|s| (s.size, s.scale, s.position))
        .collect();
    assert_eq!(
        got,
        [
            ((3840, 2160), 2.0, (0, 0)),
            ((1920, 1080), 1.0, (1920, 0)),
            ((1280, 1024), 1.25, (0, 1080)),
        ]
    );
}

#[test]
fn a_layout_of_no_whole_entry_or_no_area_is_ignored() {
    let entry = |w: i32, h: i32| -> Vec<u8> {
        [0, 0, w, h]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .chain(100u32.to_ne_bytes())
            .collect()
    };
    let one = parse_layout(&entry(1920, 1080)).expect("one monitor");
    assert_eq!(
        (one[0].width, one[0].height, one[0].scale),
        (1920, 1080, 100)
    );
    assert_eq!(parse_layout(&[]), None, "no monitor");
    assert_eq!(
        parse_layout(&entry(1920, 1080)[..19]),
        None,
        "a partial entry"
    );
    assert_eq!(parse_layout(&entry(0, 1080)), None, "no width");
    assert_eq!(parse_layout(&entry(1920, 16385)), None, "too tall");
    assert_eq!(parse_layout(&entry(64, 64).repeat(17)), None, "17 monitors");
    assert_eq!(
        parse_layout(&entry(64, 64).repeat(16)).map(|l| l.len()),
        Some(16)
    );
}
