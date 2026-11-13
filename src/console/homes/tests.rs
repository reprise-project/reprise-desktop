use smithay::output::Output;

use crate::harness::{Harness, LockScreen, TestClient, Toplevel};
use crate::raster::Rect;
use crate::snap::ZoneLayout;
use crate::window::WindowId;

fn remotes(h: &Harness) -> Vec<Output> {
    crate::virtualout::remote_outputs(&h.state)
}

fn frame(h: &Harness, w: WindowId) -> Rect {
    h.state.win(w).expect("the window").frame_rect()
}

fn centre_on(r: Rect, b: Rect) -> bool {
    b.contains(r.x + r.width / 2, r.y + r.height / 2)
}

/// `r`'s top-left as fractions of `area`.
fn rel(r: Rect, area: Rect) -> (f64, f64) {
    (
        f64::from(r.x - area.x) / f64::from(area.width),
        f64::from(r.y - area.y) / f64::from(area.height),
    )
}

fn near(a: (f64, f64), b: (f64, f64), area: Rect) {
    let px = ((a.0 - b.0) * f64::from(area.width)).abs();
    let py = ((a.1 - b.1) * f64::from(area.height)).abs();
    assert!(px <= 1.0 && py <= 1.0, "{a:?} is {b:?} of {area:?}");
}

/// The point at fractions `f` of `area`.
fn at(area: Rect, f: (f64, f64)) -> (i32, i32) {
    (
        area.x + (f.0 * f64::from(area.width)) as i32,
        area.y + (f.1 * f64::from(area.height)) as i32,
    )
}

fn area(h: &Harness, o: &Output) -> Rect {
    h.state.usable_area(Some(o))
}

fn place(h: &mut Harness, c: &mut TestClient, t: Toplevel, o: &Output, f: (f64, f64)) {
    let (x, y) = at(area(h, o), f);
    h.state.move_to(t.id, x, y);
    h.roundtrip(c);
}

struct Session {
    portal: TestClient,
    lock: LockScreen,
}

fn begin(h: &mut Harness, layout: &[(i32, i32, i32, i32, u32)]) -> Session {
    let mut portal = h.connect_portal();
    let lock = portal.lock_screen();
    lock.set_layout(layout);
    h.roundtrip(&mut portal);
    Session { portal, lock }
}

fn end(h: &mut Harness, mut s: Session, c: &mut TestClient) {
    s.lock.object.destroy();
    h.roundtrip(&mut s.portal);
    h.roundtrip(c);
}

#[test]
fn lock_mode_moves_each_window_to_the_same_relative_place_on_a_remote_screen() {
    let mut h = Harness::new();
    let edp = h.output("eDP-1", 1920, 1080);
    let dp = h.output("DP-2", 1280, 1024);
    let mut c = h.connect();
    let a = h.map_toplevel(&mut c, "a", 400, 300);
    place(&mut h, &mut c, a, &edp, (0.25, 0.25));
    let b = h.map_toplevel(&mut c, "b", 300, 200);
    place(&mut h, &mut c, b, &dp, (0.5, 0.1));
    let hidden = h.map_toplevel(&mut c, "hidden", 300, 200);
    place(&mut h, &mut c, hidden, &edp, (0.6, 0.5));
    h.state.set_minimized(hidden.id, true);
    let away = h.map_toplevel(&mut c, "away", 300, 200);
    place(&mut h, &mut c, away, &dp, (0.2, 0.6));
    h.state.win_mut(away.id).expect("the window").desktop = 1;
    let before = |h: &Harness, t: Toplevel, o: &Output| rel(frame(h, t.id), area(h, o));
    let (ra, rb) = (before(&h, a, &edp), before(&h, b, &dp));
    let width = frame(&h, a.id).width;
    let (rh, rw) = (before(&h, hidden, &edp), before(&h, away, &dp));
    let s = begin(
        &mut h,
        &[(0, 0, 2560, 1440, 100), (2560, 0, 1920, 1080, 100)],
    );
    h.roundtrip(&mut c);
    let r = remotes(&h);
    let on = |h: &Harness, t: Toplevel, o: &Output, f: (f64, f64)| {
        let fr = frame(h, t.id);
        assert!(
            centre_on(fr, h.state.layout_box(Some(o))),
            "{fr:?} on {}",
            o.name()
        );
        near(rel(fr, area(h, o)), f, area(h, o));
    };
    on(&h, a, &r[0], ra);
    on(&h, b, &r[1], rb);
    on(&h, hidden, &r[0], rh);
    on(&h, away, &r[1], rw);
    assert_eq!(frame(&h, a.id).width, width, "the size stays");
    near(ra, (0.25, 0.25), area(&h, &r[0]));
    end(&mut h, s, &mut c);
}

#[test]
fn screens_past_the_remote_count_send_their_windows_to_the_remote_primary() {
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    let dp = h.output("DP-2", 1280, 1024);
    let mut c = h.connect();
    let b = h.map_toplevel(&mut c, "b", 300, 200);
    place(&mut h, &mut c, b, &dp, (0.5, 0.5));
    let _s = begin(&mut h, &[(0, 0, 1920, 1080, 100)]);
    h.roundtrip(&mut c);
    let r = remotes(&h);
    let f = frame(&h, b.id);
    assert!(centre_on(f, h.state.layout_box(Some(&r[0]))), "{f:?}");
    near(rel(f, area(&h, &r[0])), (0.5, 0.5), area(&h, &r[0]));
}

fn halves() -> ZoneLayout {
    ZoneLayout::parse("1|1").expect("a layout")
}

#[test]
fn leaving_lock_mode_brings_each_window_back_where_it_was() {
    let mut h = Harness::new();
    let edp = h.output("eDP-1", 1920, 1080);
    let dp = h.output("DP-2", 1280, 1024);
    let mut c = h.connect();
    let normal = h.map_toplevel(&mut c, "normal", 500, 400);
    place(&mut h, &mut c, normal, &dp, (0.3, 0.2));
    let max = h.map_toplevel(&mut c, "max", 500, 400);
    place(&mut h, &mut c, max, &edp, (0.1, 0.1));
    h.state.set_maximized(max.id, true);
    let full = h.map_toplevel(&mut c, "full", 500, 400);
    place(&mut h, &mut c, full, &dp, (0.2, 0.4));
    h.state.set_fullscreen(full.id, true);
    let zoned = h.map_toplevel(&mut c, "zoned", 500, 400);
    let d = area(&h, &dp);
    let zone = Rect::new(d.x + d.width / 2, d.y, d.width / 2, d.height);
    h.state.snap_to_zone(zoned.id, zone);
    let left = h.map_toplevel(&mut c, "left", 500, 400);
    let right = h.map_toplevel(&mut c, "right", 500, 400);
    h.state.apply_layout(&edp, halves(), false, left.id, 0);
    h.state.apply_layout(&edp, halves(), false, right.id, 1);
    h.roundtrip(&mut c);
    assert!(h.state.wm.snap.shares_group(left.id, right.id));
    let ws = [normal, max, full, zoned, left, right].map(|t| t.id);
    let kept = |h: &Harness| {
        ws.map(|w| {
            let win = h.state.win(w).expect("the window");
            (
                win.frame_rect(),
                win.saved,
                win.saved_fullscreen,
                win.snapped,
            )
        })
    };
    let home = kept(&h);

    let s = begin(&mut h, &[(0, 0, 2560, 1440, 100)]);
    h.roundtrip(&mut c);
    let r = remotes(&h)[0].clone();
    let rbox = h.state.layout_box(Some(&r));
    for w in ws {
        assert!(
            centre_on(frame(&h, w), rbox),
            "{:?} is on the remote",
            frame(&h, w)
        );
    }
    assert_eq!(frame(&h, max.id), area(&h, &r), "maximized on the remote");
    assert_eq!(frame(&h, full.id), rbox, "full screen on the remote");
    let ra = area(&h, &r);
    assert_eq!(
        frame(&h, left.id),
        Rect::new(ra.x, ra.y, ra.width / 2, ra.height),
        "the left zone of the remote"
    );
    assert!(h.state.wm.snap.shares_group(left.id, right.id));
    assert_eq!(
        frame(&h, zoned.id),
        Rect::new(ra.x + ra.width / 2, ra.y, ra.width / 2, ra.height),
        "the same zone of the remote"
    );
    assert!(h.state.win(max.id).is_some_and(|w| w.maximized));

    end(&mut h, s, &mut c);
    assert_eq!(kept(&h), home);
    assert!(h.state.win(max.id).is_some_and(|w| w.maximized));
    assert!(h.state.win(full.id).is_some_and(|w| w.fullscreen));
    assert!(h.state.wm.snap.shares_group(left.id, right.id));
    for w in ws {
        assert!(
            h.state.win(w).is_some_and(|w| w.home.is_none()),
            "homes are spent"
        );
    }
}

#[test]
fn resizing_the_remote_screen_keeps_windows_in_their_relative_place() {
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    let mut c = h.connect();
    let a = h.map_toplevel(&mut c, "a", 400, 300);
    let b = h.map_toplevel(&mut c, "b", 400, 300);
    let max = h.map_toplevel(&mut c, "max", 400, 300);
    let mut s = begin(
        &mut h,
        &[(0, 0, 1920, 1080, 100), (1920, 0, 1920, 1080, 100)],
    );
    let r = remotes(&h);
    let to = |h: &mut Harness, c: &mut TestClient, t: Toplevel, o: &Output, f: (f64, f64)| {
        let (x, y) = at(h.state.layout_box(Some(o)), f);
        h.state.move_to(t.id, x, y);
        h.roundtrip(c);
    };
    to(&mut h, &mut c, a, &r[0], (0.25, 0.25));
    to(&mut h, &mut c, b, &r[1], (0.5, 0.125));
    to(&mut h, &mut c, max, &r[1], (0.25, 0.25));
    h.state.set_maximized(max.id, true);
    h.roundtrip(&mut c);
    for (w0, h0, w1, h1) in [
        (2560, 1440, 1920, 1080),
        (1280, 720, 1920, 1080),
        (2560, 1440, 2560, 1440),
        (1920, 1080, 1280, 1024),
    ] {
        s.lock
            .set_layout(&[(0, 0, w0, h0, 100), (w0, 0, w1, h1, 100)]);
        h.roundtrip(&mut s.portal);
        h.roundtrip(&mut c);
        let boxes = [0, 1].map(|i| h.state.layout_box(Some(&r[i])));
        assert_eq!(boxes[0].width, w0, "the remote screen took the new size");
        assert_eq!(rel(frame(&h, a.id), boxes[0]), (0.25, 0.25), "{w0}x{h0}");
        assert_eq!(rel(frame(&h, b.id), boxes[1]), (0.5, 0.125), "{w1}x{h1}");
        assert_eq!(
            frame(&h, max.id),
            area(&h, &r[1]),
            "maximized on its own screen"
        );
    }
}

#[test]
fn resizing_the_remote_screen_keeps_a_zoned_window_filling_its_zone() {
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "zoned", 400, 300);
    let mut s = begin(&mut h, &[(0, 0, 1920, 1080, 100)]);
    let r = remotes(&h)[0].clone();
    let right = |a: Rect| Rect::new(a.x + a.width / 2, a.y, a.width / 2, a.height);
    h.state.snap_to_zone(t.id, right(area(&h, &r)));
    h.roundtrip(&mut c);
    s.lock.set_layout(&[(0, 0, 2560, 1440, 100)]);
    h.roundtrip(&mut s.portal);
    h.roundtrip(&mut c);
    assert_eq!(
        frame(&h, t.id),
        right(area(&h, &r)),
        "the right half of the remote's work area"
    );
}

#[test]
fn a_remote_screen_going_away_keeps_its_maximized_window_on_the_remote_side_and_its_home() {
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    let dp = h.output("DP-2", 1280, 1024);
    let mut c = h.connect();
    let max = h.map_toplevel(&mut c, "max", 500, 400);
    place(&mut h, &mut c, max, &dp, (0.1, 0.1));
    h.state.set_maximized(max.id, true);
    let normal = h.map_toplevel(&mut c, "normal", 300, 200);
    place(&mut h, &mut c, normal, &dp, (0.25, 0.25));
    let mut s = begin(
        &mut h,
        &[(0, 0, 1920, 1080, 100), (1920, 0, 2560, 1440, 100)],
    );
    h.roundtrip(&mut c);
    let r = remotes(&h);
    assert_eq!(frame(&h, max.id), area(&h, &r[1]), "maximized on REMOTE-2");
    let on_second = rel(frame(&h, normal.id), h.state.layout_box(Some(&r[1])));
    s.lock.set_layout(&[(0, 0, 1920, 1080, 100)]);
    h.roundtrip(&mut s.portal);
    h.roundtrip(&mut c);
    assert_eq!(
        frame(&h, max.id),
        area(&h, &r[0]),
        "maximized on the remote primary"
    );
    let win = h.state.win(max.id).expect("the window");
    assert!(
        win.maximized && win.home.is_some(),
        "still maximized, its home kept"
    );
    let first = h.state.layout_box(Some(&r[0]));
    near(rel(frame(&h, normal.id), first), on_second, first);
    end(&mut h, s, &mut c);
    assert_eq!(frame(&h, max.id), area(&h, &dp), "maximized on DP-2 again");
    near(
        rel(frame(&h, normal.id), area(&h, &dp)),
        (0.25, 0.25),
        area(&h, &dp),
    );
}

#[test]
fn lock_mode_keeps_the_last_session_and_writes_no_open_here_rule() {
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    h.state.config.session.restore = true;
    h.state.app_index = vec![
        reprisefs::apps::parse_desktop(
            "[Desktop Entry]\nType=Application\nName=app\nExec=app\n",
            "app.desktop",
        )
        .expect("an entry"),
    ];
    let _s = begin(&mut h, &[(0, 0, 1920, 1080, 100)]);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "app", 400, 300);
    crate::session::save(&mut h.state);
    assert!(
        h.state.session.apps.is_empty(),
        "{:?}",
        h.state.session.apps
    );
    assert_eq!(
        h.state.open_here_state(t.id),
        crate::winmenu::OpenHereState::Greyed
    );
    h.state.toggle_open_here(t.id);
    assert!(!crate::windowrules::has_open_here(
        &h.state.config.window_rules,
        "app"
    ));
}

#[test]
fn new_windows_in_lock_mode_open_on_a_remote_screen() {
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    h.pointer_to(400.0, 300.0);
    let _s = begin(&mut h, &[(0, 0, 1920, 1080, 100)]);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "new", 400, 300);
    let r = remotes(&h);
    let f = frame(&h, t.id);
    assert!(centre_on(f, h.state.layout_box(Some(&r[0]))), "{f:?}");
}

#[test]
fn a_window_opened_in_lock_mode_goes_to_the_console_primary_after() {
    let mut h = Harness::new();
    let edp = h.output("eDP-1", 1920, 1080);
    h.output("DP-2", 1280, 1024);
    let s = begin(
        &mut h,
        &[(0, 0, 1920, 1080, 100), (1920, 0, 2560, 1440, 100)],
    );
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "new", 400, 300);
    let r = remotes(&h);
    place(&mut h, &mut c, t, &r[1], (0.25, 0.25));
    end(&mut h, s, &mut c);
    let f = frame(&h, t.id);
    assert!(centre_on(f, h.state.layout_box(Some(&edp))), "{f:?}");
    near(rel(f, area(&h, &edp)), (0.25, 0.25), area(&h, &edp));
}

#[test]
fn desktop_icons_and_notifications_follow_the_remote_primary_in_lock_mode() {
    let mut h = Harness::new();
    let edp = h.output("eDP-1", 1920, 1080);
    let mut c = h.connect();
    let s = begin(
        &mut h,
        &[(0, 0, 2560, 1440, 100), (2560, 0, 1920, 1080, 100)],
    );
    let r = remotes(&h);
    assert_eq!(h.state.first_output(), Some(r[0].clone()));
    assert_eq!(h.state.desktop_area(), area(&h, &r[0]), "the icons' area");
    let n = crate::tray::Notification::new(1, "n".into(), "x".into());
    crate::notifyview::show(&mut h.state, n);
    let rbox = h.state.layout_box(Some(&r[0]));
    let balloons = crate::notifyview::balloon_origins(&h.state);
    assert!(!balloons.is_empty(), "a balloon shows");
    for (x, y) in balloons {
        assert!(rbox.contains(x, y), "the balloon at {x},{y} is on {rbox:?}");
    }
    end(&mut h, s, &mut c);
    assert_eq!(h.state.first_output(), Some(edp.clone()));
    assert_eq!(h.state.desktop_area(), area(&h, &edp));
}

#[test]
fn the_saved_display_layout_never_records_a_remote_screen() {
    let mut h = Harness::new();
    h.head("eDP-1", 1920, 1080);
    let _s = begin(&mut h, &[(0, 0, 2560, 1440, 100)]);
    h.state.outputs_reloaded();
    h.dispatch();
    crate::state::flush();
    assert_eq!(h.state.saved_primary(), Some("eDP-1"));
    let r = remotes(&h);
    assert_eq!(h.state.saved_layout_of(&r[0]), None);
    let file = std::fs::read_to_string(&h.state.saved_layouts.path).expect("the saved layouts");
    assert!(!file.contains("REMOTE"), "{file}");
}

#[test]
fn a_session_with_no_monitor_yet_takes_its_windows_to_the_first_monitor_at_their_relative_places() {
    let mut h = Harness::new();
    let mut c = h.connect();
    let s = begin(&mut h, &[(0, 0, 1024, 768, 100)]);
    h.roundtrip(&mut c);
    let r = remotes(&h)[0].clone();
    let t = h.map_toplevel(&mut c, "t", 400, 300);
    place(&mut h, &mut c, t, &r, (0.25, 0.25));
    let from = rel(frame(&h, t.id), area(&h, &r));
    let width = frame(&h, t.id).width;
    end(&mut h, s, &mut c);

    let monitor = h.output("Virtual-1", 1920, 1080);
    h.roundtrip(&mut c);
    near(
        rel(frame(&h, t.id), area(&h, &monitor)),
        from,
        area(&h, &monitor),
    );
    assert_eq!(frame(&h, t.id).width, width, "the size stays");
}

/// A session with no monitor whose window `t` lock mode left on a 1024x768 client screen at
/// (0.25, 0.25) of its work area; that place, as `rel` gives it.
fn held_window(h: &mut Harness, c: &mut TestClient) -> (Toplevel, (f64, f64)) {
    let s = begin(h, &[(0, 0, 1024, 768, 100)]);
    h.roundtrip(c);
    let r = remotes(h)[0].clone();
    let t = h.map_toplevel(c, "t", 400, 300);
    place(h, c, t, &r, (0.25, 0.25));
    let from = rel(frame(h, t.id), area(h, &r));
    end(h, s, c);
    (t, from)
}

#[test]
fn a_window_back_from_lock_mode_stays_put_when_another_monitor_comes() {
    let mut h = Harness::new();
    let mut c = h.connect();
    let (t, _) = held_window(&mut h, &mut c);
    let s = begin(&mut h, &[(0, 0, 1024, 768, 100)]);
    h.roundtrip(&mut c);
    let monitor = h.output("HDMI-A-1", 1920, 1080);
    h.roundtrip(&mut c);
    end(&mut h, s, &mut c);
    let back = frame(&h, t.id);
    assert!(
        centre_on(back, h.state.layout_box(Some(&monitor))),
        "{back:?}"
    );

    h.output("DP-1", 1280, 1024);
    h.roundtrip(&mut c);
    assert_eq!(frame(&h, t.id), back, "the window stays");
}

#[test]
fn a_snap_group_waiting_for_a_monitor_fills_its_zones_there() {
    let mut h = Harness::new();
    let mut c = h.connect();
    let s = begin(&mut h, &[(0, 0, 1024, 768, 100)]);
    h.roundtrip(&mut c);
    let r = remotes(&h)[0].clone();
    let left = h.map_toplevel(&mut c, "left", 300, 200);
    let right = h.map_toplevel(&mut c, "right", 300, 200);
    h.state.apply_layout(&r, halves(), false, left.id, 0);
    h.state.apply_layout(&r, halves(), false, right.id, 1);
    h.roundtrip(&mut c);
    assert!(h.state.wm.snap.shares_group(left.id, right.id));
    end(&mut h, s, &mut c);

    let monitor = h.output("HDMI-A-1", 1920, 1080);
    h.roundtrip(&mut c);
    let a = area(&h, &monitor);
    assert_eq!(
        frame(&h, left.id),
        Rect::new(a.x, a.y, a.width / 2, a.height),
        "the left half of the monitor"
    );
    assert!(centre_on(
        frame(&h, right.id),
        Rect::new(a.x + a.width / 2, a.y, a.width / 2, a.height)
    ));
    assert_eq!(h.state.wm.snap.group_output(left.id), Some("HDMI-A-1"));
}

#[test]
fn a_waiting_window_takes_its_relative_place_on_a_monitor_placed_off_the_origin() {
    let mut h = Harness::new();
    let mut c = h.connect();
    let (t, from) = held_window(&mut h, &mut c);

    let monitor = h.add_output("HDMI-A-1", 1920, 1080, Some((1920, 0)));
    h.dispatch();
    h.roundtrip(&mut c);
    assert_eq!(h.state.layout_box(Some(&monitor)).x, 1920);
    near(
        rel(frame(&h, t.id), area(&h, &monitor)),
        from,
        area(&h, &monitor),
    );
}

/// `[[outputs]]` making `name` the primary.
fn primary(name: &str) -> crate::config::OutputConfig {
    crate::config::OutputConfig {
        name: name.into(),
        scale: 1.0,
        mode: None,
        adaptive_sync: None,
        transform: Default::default(),
        position: None,
        primary: true,
        icc_profile: None,
    }
}

#[test]
fn waiting_windows_go_to_the_console_primary_of_the_monitors_that_come_together() {
    let mut h = Harness::new();
    h.state.config.outputs = vec![primary("HDMI-A-1")];
    let mut c = h.connect();
    let (t, from) = held_window(&mut h, &mut c);

    h.add_output("DP-1", 1280, 1024, None);
    let primary = h.add_output("HDMI-A-1", 1920, 1080, None);
    h.dispatch();
    h.roundtrip(&mut c);
    let f = frame(&h, t.id);
    assert!(centre_on(f, h.state.layout_box(Some(&primary))), "{f:?}");
    near(rel(f, area(&h, &primary)), from, area(&h, &primary));
}

#[test]
fn a_window_back_from_lock_mode_lands_on_the_standby_and_a_monitor_takes_it_in_place() {
    let mut h = Harness::new();
    crate::standby::rdp_changed(&mut h.state, true);
    let mut c = h.connect();
    let s = begin(&mut h, &[(0, 0, 2560, 1440, 100)]);
    h.roundtrip(&mut c);
    let r = remotes(&h)[0].clone();
    let t = h.map_toplevel(&mut c, "t", 400, 300);
    place(&mut h, &mut c, t, &r, (0.75, 0.5));
    let from = rel(frame(&h, t.id), area(&h, &r));
    end(&mut h, s, &mut c);
    let standby = (h.state.scene.outputs().iter())
        .find(|o| crate::backend::is_virtual(o))
        .cloned()
        .expect("the standby holds the desktop");
    let f = frame(&h, t.id);
    assert!(centre_on(f, h.state.layout_box(Some(&standby))), "{f:?}");
    near(rel(f, area(&h, &standby)), from, area(&h, &standby));

    h.output("HDMI-A-1", 1920, 1080);
    h.roundtrip(&mut c);
    assert_eq!(frame(&h, t.id), f, "the window stays where it is");
}

#[test]
fn a_snap_group_whose_monitor_left_fills_its_zones_on_the_remote_screen() {
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    let hdmi = h.output("HDMI-A-1", 1920, 1080);
    let mut c = h.connect();
    let left = h.map_toplevel(&mut c, "left", 300, 200);
    let right = h.map_toplevel(&mut c, "right", 300, 200);
    h.state.apply_layout(&hdmi, halves(), false, left.id, 0);
    h.state.apply_layout(&hdmi, halves(), false, right.id, 1);
    h.roundtrip(&mut c);
    h.state.scene.remove_output(&hdmi);
    h.state.output_removed(&hdmi);
    h.roundtrip(&mut c);
    assert_eq!(h.state.wm.snap.group_output(left.id), Some("HDMI-A-1"));

    let _s = begin(&mut h, &[(0, 0, 1024, 768, 100)]);
    h.roundtrip(&mut c);
    let r = remotes(&h)[0].clone();
    assert_eq!(
        h.state.wm.snap.group_output(left.id),
        Some(r.name().as_str())
    );
    let a = area(&h, &r);
    assert_eq!(
        frame(&h, left.id),
        Rect::new(a.x, a.y, a.width / 2, a.height),
        "the left half of the client's screen"
    );
}

#[test]
fn waiting_windows_wait_for_a_deferred_monitor_enable_and_go_to_the_console_primary() {
    let mut h = Harness::new();
    h.state.config.outputs = vec![primary("HDMI-A-1")];
    let mut c = h.connect();
    let (t, from) = held_window(&mut h, &mut c);

    h.state.test_outputs_pending = true;
    h.output("DP-1", 1280, 1024);
    h.roundtrip(&mut c);
    h.state.test_outputs_pending = false;
    let primary = h.output("HDMI-A-1", 1920, 1080);
    h.roundtrip(&mut c);
    let f = frame(&h, t.id);
    assert!(centre_on(f, h.state.layout_box(Some(&primary))), "{f:?}");
    near(rel(f, area(&h, &primary)), from, area(&h, &primary));
}

#[test]
fn a_window_whose_monitor_left_in_lock_mode_goes_home_when_it_comes_back() {
    let mut h = Harness::new();
    let edp = h.output("eDP-1", 1920, 1080);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "t", 500, 400);
    place(&mut h, &mut c, t, &edp, (0.3, 0.2));
    let home = frame(&h, t.id);
    let s = begin(&mut h, &[(0, 0, 1024, 768, 100)]);
    h.roundtrip(&mut c);
    let r = remotes(&h)[0].clone();
    place(&mut h, &mut c, t, &r, (0.5, 0.4));
    h.state.scene.remove_output(&edp);
    h.state.output_removed(&edp);
    end(&mut h, s, &mut c);
    assert!(
        h.state.win(t.id).is_some_and(|w| w.home.is_none()),
        "homes are spent"
    );

    h.output("eDP-1", 1920, 1080);
    h.roundtrip(&mut c);
    assert_eq!(frame(&h, t.id), home, "back at its home");
}

#[test]
fn a_window_waiting_for_its_monitor_keeps_its_home_through_a_reconnect() {
    let mut h = Harness::new();
    let edp = h.output("eDP-1", 1920, 1080);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "t", 500, 400);
    place(&mut h, &mut c, t, &edp, (0.3, 0.2));
    let home = frame(&h, t.id);
    let s = begin(&mut h, &[(0, 0, 1024, 768, 100)]);
    h.roundtrip(&mut c);
    let r = remotes(&h)[0].clone();
    place(&mut h, &mut c, t, &r, (0.5, 0.4));
    h.state.scene.remove_output(&edp);
    h.state.output_removed(&edp);
    end(&mut h, s, &mut c);

    let s = begin(&mut h, &[(0, 0, 1024, 768, 100)]);
    h.roundtrip(&mut c);
    h.output("eDP-1", 1920, 1080);
    h.roundtrip(&mut c);
    end(&mut h, s, &mut c);
    assert_eq!(frame(&h, t.id), home, "back at its home");
}

#[test]
fn a_background_session_takes_its_windows_to_its_first_monitor_at_their_relative_places() {
    let mut h = Harness::new();
    crate::standby::rdp_changed(&mut h.state, true);
    crate::lockview::seat_away(&mut h.state, true);
    let mut c = h.connect();
    let s = begin(&mut h, &[(0, 0, 1024, 768, 100)]);
    h.roundtrip(&mut c);
    let r = remotes(&h)[0].clone();
    let t = h.map_toplevel(&mut c, "t", 200, 150);
    place(&mut h, &mut c, t, &r, (0.75, 0.5));
    let from = rel(frame(&h, t.id), area(&h, &r));
    end(&mut h, s, &mut c);

    h.state.test_outputs_pending = true;
    crate::lockview::seat_away(&mut h.state, false);
    h.roundtrip(&mut c);
    let monitor = h.output("eDP-1", 1440, 900);
    h.state.test_outputs_pending = false;
    h.roundtrip(&mut c);
    let f = frame(&h, t.id);
    assert!(centre_on(f, h.state.layout_box(Some(&monitor))), "{f:?}");
    near(rel(f, area(&h, &monitor)), from, area(&h, &monitor));
}

#[test]
fn a_window_waiting_for_a_monitor_takes_its_relative_place_on_the_next_remote_screen() {
    let mut h = Harness::new();
    let mut c = h.connect();
    let (t, from) = held_window(&mut h, &mut c);

    let _s = begin(&mut h, &[(0, 0, 2560, 1440, 100)]);
    h.roundtrip(&mut c);
    let r = remotes(&h)[0].clone();
    near(rel(frame(&h, t.id), area(&h, &r)), from, area(&h, &r));
}

/// Two client screens side by side: 1280x720, then 1024x768.
const TWO: [(i32, i32, i32, i32, u32); 2] = [(0, 0, 1280, 720, 100), (1280, 0, 1024, 768, 100)];

#[test]
fn each_window_goes_back_to_the_client_screen_it_left() {
    let mut h = Harness::new();
    let mut c = h.connect();
    let s = begin(&mut h, &TWO);
    h.roundtrip(&mut c);
    let r = remotes(&h);
    let a = h.map_toplevel(&mut c, "a", 300, 200);
    let b = h.map_toplevel(&mut c, "b", 300, 200);
    place(&mut h, &mut c, a, &r[0], (0.25, 0.25));
    place(&mut h, &mut c, b, &r[1], (0.5, 0.4));
    end(&mut h, s, &mut c);
    let monitor = h.output("eDP-1", 1440, 900);
    h.roundtrip(&mut c);
    let on_console = |h: &Harness, t: Toplevel| rel(frame(h, t.id), area(h, &monitor));
    let (fa, fb) = (on_console(&h, a), on_console(&h, b));

    let _s = begin(&mut h, &TWO);
    h.roundtrip(&mut c);
    let r = remotes(&h);
    near(rel(frame(&h, a.id), area(&h, &r[0])), fa, area(&h, &r[0]));
    near(rel(frame(&h, b.id), area(&h, &r[1])), fb, area(&h, &r[1]));
}

#[test]
fn a_window_whose_client_screen_is_gone_goes_to_the_client_primary() {
    let mut h = Harness::new();
    let monitor = h.output("eDP-1", 1440, 900);
    let mut c = h.connect();
    let s = begin(&mut h, &TWO);
    h.roundtrip(&mut c);
    let b = h.map_toplevel(&mut c, "b", 300, 200);
    let r = remotes(&h);
    place(&mut h, &mut c, b, &r[1], (0.5, 0.4));
    end(&mut h, s, &mut c);
    let from = rel(frame(&h, b.id), area(&h, &monitor));

    let _s = begin(&mut h, &[(0, 0, 1024, 768, 100)]);
    h.roundtrip(&mut c);
    let r = remotes(&h)[0].clone();
    near(rel(frame(&h, b.id), area(&h, &r)), from, area(&h, &r));
}

#[test]
fn windows_on_the_standby_take_their_relative_places_on_the_client_screen() {
    let mut h = Harness::new();
    crate::standby::rdp_changed(&mut h.state, true);
    let mut c = h.connect();
    let standby = (h.state.scene.outputs().iter())
        .find(|o| crate::backend::is_virtual(o))
        .cloned()
        .expect("the standby holds the desktop");
    let t = h.map_toplevel(&mut c, "t", 200, 150);
    place(&mut h, &mut c, t, &standby, (0.75, 0.5));
    let from = rel(frame(&h, t.id), area(&h, &standby));

    let _s = begin(&mut h, &[(0, 0, 1024, 768, 100)]);
    h.roundtrip(&mut c);
    let r = remotes(&h)[0].clone();
    near(rel(frame(&h, t.id), area(&h, &r)), from, area(&h, &r));
}
