use super::*;
use crate::harness::Harness;

fn names(h: &Harness) -> Vec<String> {
    h.state.scene.outputs().iter().map(|o| o.name()).collect()
}

/// A monitor leaving the layout, as `backend::disable` and `leave_layout` take it out.
fn unplug(h: &mut Harness, o: &Output) {
    h.state.scene.remove_output(o);
    h.state.output_removed(o);
    h.dispatch();
}

#[test]
fn the_standby_exists_only_with_remote_desktop_on_and_no_monitor() {
    assert!(wanted(true, 0));
    assert!(!wanted(true, 1));
    assert!(!wanted(true, 2));
    assert!(!wanted(false, 0));
    assert!(!wanted(false, 1));
}

#[test]
fn the_standby_comes_when_wanted_and_goes_when_not() {
    assert_eq!(change(false, true), Change::Add);
    assert_eq!(change(true, false), Change::Remove);
    assert_eq!(change(true, true), Change::Keep);
    assert_eq!(change(false, false), Change::Keep);
}

#[test]
fn the_standby_runs_as_the_last_monitor_else_the_configured_primary_else_1080p() {
    assert_eq!(spec(None, &[]), FALLBACK);
    let out = |name: &str, primary: bool, mode: &str| OutputConfig {
        name: name.into(),
        scale: 1.5,
        mode: Some(crate::config::OutputMode::parse(mode).unwrap()),
        adaptive_sync: None,
        transform: Default::default(),
        position: Some((100, 0)),
        primary,
    };
    let outputs = [
        out("DP-1", false, "2560x1440"),
        out("HDMI-A-1", true, "3840x2160@30"),
    ];
    let configured = spec(None, &outputs);
    assert_eq!(configured.size, (3840, 2160));
    assert_eq!(configured.refresh, 30_000);
    assert_eq!(configured.scale, 1.5);
    assert_eq!(configured.position, (100, 0));
    assert_eq!(spec(None, &outputs[..1]).size, (2560, 1440));
    let last = Spec {
        size: (1280, 1024),
        ..FALLBACK
    };
    assert_eq!(spec(Some(last), &outputs), last);
}

#[test]
fn turning_remote_desktop_on_with_no_monitor_adds_the_standby_and_off_removes_it() {
    let mut h = Harness::new();
    rdp_changed(&mut h.state, true);
    h.dispatch();
    assert_eq!(names(&h), ["HEADLESS-1"]);
    let o = h.state.scene.outputs()[0].clone();
    assert!(crate::backend::is_virtual(&o));
    assert_eq!(o.current_mode().map(|m| m.size), Some((1920, 1080).into()));
    assert_eq!(h.state.layout_box(Some(&o)).x, 0);
    assert!(!h.state.bars.is_empty(), "the standby has a taskbar");
    rdp_changed(&mut h.state, false);
    h.dispatch();
    assert!(names(&h).is_empty(), "no output once Remote Desktop is off");
}

#[test]
fn remote_desktop_with_a_monitor_adds_no_standby() {
    let mut h = Harness::new();
    h.output("DP-1", 1920, 1080);
    rdp_changed(&mut h.state, true);
    h.dispatch();
    assert_eq!(names(&h), ["DP-1"]);
}

#[test]
fn the_last_monitor_leaving_with_remote_desktop_off_leaves_no_output() {
    let mut h = Harness::new();
    let dp = h.output("DP-1", 1920, 1080);
    unplug(&mut h, &dp);
    assert!(names(&h).is_empty());
}

#[test]
fn the_last_monitor_leaving_hands_its_mode_and_place_to_the_standby_and_windows_stay() {
    let mut h = Harness::new();
    let dp = h.output("DP-1", 2560, 1440);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "test.app", 300, 200);
    h.state.move_to(t.id, 700, 400);
    rdp_changed(&mut h.state, true);
    h.dispatch();
    assert_eq!(names(&h), ["DP-1"]);
    unplug(&mut h, &dp);
    assert_eq!(names(&h), ["HEADLESS-1"]);
    let standby = h.state.scene.outputs()[0].clone();
    assert_eq!(
        standby.current_mode().map(|m| m.size),
        Some((2560, 1440).into())
    );
    let r = h.state.win(t.id).unwrap().frame_rect();
    assert_eq!((r.x, r.y), (700, 400), "the window keeps its place");
}

#[test]
fn a_monitor_coming_in_replaces_the_standby_and_windows_keep_their_place() {
    let mut h = Harness::new();
    rdp_changed(&mut h.state, true);
    h.dispatch();
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "test.app", 300, 200);
    h.state.move_to(t.id, 640, 360);
    let before = h.state.win(t.id).unwrap().frame_rect();
    h.output("DP-1", 1920, 1080);
    assert_eq!(
        names(&h),
        ["DP-1"],
        "the monitor alone, no standby beside it"
    );
    let after = h.state.win(t.id).unwrap().frame_rect();
    assert_eq!((after.x, after.y), (before.x, before.y));
    let dp = h.state.scene.outputs()[0].clone();
    unplug(&mut h, &dp);
    assert_eq!(
        names(&h),
        ["HEADLESS-2"],
        "the standby is back once the monitor goes"
    );
}

#[test]
fn output_management_does_not_list_the_standby() {
    let mut h = Harness::new();
    rdp_changed(&mut h.state, true);
    h.dispatch();
    assert_eq!(names(&h), ["HEADLESS-1"]);
    assert!(
        h.state.outputs_mgmt.head_names().is_empty(),
        "Settings > Displays reads output management, which has no head for the standby"
    );
    assert!(crate::backend::heads(&h.state).is_empty());
}

#[test]
fn a_lock_on_the_standby_is_confirmed_once_its_clock_shows_the_cover() {
    let mut h = Harness::new();
    rdp_changed(&mut h.state, true);
    h.dispatch();
    let mut c = h.connect();
    let (_lock, _surface) = h.map_lock_surface(&mut c);
    assert!(h.state.is_locked());
    h.run_for(1100);
    assert!(
        crate::sessionlock::confirmed(&mut h.state),
        "the standby's frame counts as the cover shown"
    );
}

fn reports(changes: usize, owned: bool, delivered: usize) -> (Vec<bool>, bool) {
    let mut seen = Vec::new();
    let ended = zbus::block_on(report_owner(
        futures_util::stream::iter(vec![(); changes]),
        || std::future::ready(owned),
        |on| {
            seen.push(on);
            seen.len() <= delivered
        },
    ));
    (seen, ended)
}

#[test]
fn the_bus_watcher_turns_remote_desktop_off_when_it_stops_following() {
    assert_eq!(
        reports(1, true, usize::MAX),
        (vec![true, true, false], true)
    );
    assert_eq!(
        reports(0, true, usize::MAX),
        (vec![true, false], true),
        "an empty stream (no match rule) still ends in off"
    );
    assert_eq!(
        reports(1, false, usize::MAX),
        (vec![false, false], true),
        "off stays off without a second report"
    );
    assert_eq!(
        reports(3, true, 1),
        (vec![true, true], false),
        "a gone compositor hears nothing more"
    );
}
