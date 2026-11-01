use super::*;

fn frame(x: i32) -> Remembered {
    Remembered {
        output: "DP-1".into(),
        x,
        y: 20,
        width: 640,
        height: 480,
        maximized: false,
        text_size: 100,
    }
}

fn kept() -> KeptFrames {
    KeptFrames::empty(PathBuf::from("/nowhere"))
}

#[test]
fn frames_are_kept_per_set_of_screens_and_read_back_from_the_text() {
    let mut k = kept();
    assert!(k.store("eDP-1", "org.example.A", frame(10)));
    assert!(k.store("Dell U2720Q + eDP-1", "org.example.A", frame(900)));
    assert_eq!(k.get("eDP-1", "org.example.A"), Some(&frame(10)));
    assert_eq!(
        k.get("Dell U2720Q + eDP-1", "org.example.A"),
        Some(&frame(900))
    );
    assert_eq!(k.get("eDP-1", "org.example.B"), None);
    let back = KeptFrames::parse(PathBuf::from("/nowhere"), &k.text());
    assert_eq!(back.get("eDP-1", "org.example.A"), Some(&frame(10)));
    assert_eq!(back.clock, 2, "the stamps come back");
}

#[test]
fn storing_the_newest_frame_again_needs_no_write() {
    let mut k = kept();
    assert!(k.store("s", "a", frame(1)));
    assert!(!k.store("s", "a", frame(1)));
    assert!(k.store("s", "b", frame(1)));
    assert!(k.store("s", "a", frame(1)), "a becomes the newest again");
}

#[test]
fn the_least_recently_stored_app_goes_past_the_cap() {
    let mut k = kept();
    for i in 0..APPS_MAX {
        k.store("s", &format!("app{i}"), frame(1));
    }
    k.store("s", "app0", frame(2));
    k.store("s", "new", frame(1));
    assert_eq!(k.sets["s"].len(), APPS_MAX);
    assert!(k.get("s", "app0").is_some(), "stored again, so kept");
    assert!(k.get("s", "app1").is_none(), "the oldest went");
}

#[test]
fn the_set_stored_to_least_recently_goes_past_the_cap() {
    let mut k = kept();
    for i in 0..=SETS_MAX {
        k.store(&format!("set{i}"), "a", frame(1));
    }
    assert_eq!(k.sets.len(), SETS_MAX);
    assert!(k.get("set0", "a").is_none());
    assert!(k.get(&format!("set{SETS_MAX}"), "a").is_some());
}

#[test]
fn an_entry_that_does_not_read_is_dropped_alone() {
    let text = "[s.good]\noutput = \"DP-1\"\nx = 1\ny = 2\nwidth = 3\nheight = 4\nused = 7\n\
                [s.bad]\nx = \"left\"\n";
    let k = KeptFrames::parse(PathBuf::from("/nowhere"), text);
    assert!(k.get("s", "good").is_some());
    assert!(k.get("s", "bad").is_none());
    assert_eq!(k.clock, 7);
    assert!(
        KeptFrames::parse(PathBuf::from("/nowhere"), "not toml [")
            .sets
            .is_empty()
    );
}

use crate::harness::{Harness, TestClient, Toplevel};

const APP: &str = "org.example.Notes";

fn one_screen() -> Harness {
    let mut h = Harness::new();
    h.output("TEST-1", 1920, 1080);
    h
}

fn close(h: &mut Harness, c: &mut TestClient, t: Toplevel) {
    c.destroy(t);
    h.roundtrip(c);
    assert!(h.state.win(t.id).is_none(), "the window went");
}

/// Opens `APP`, gives it frame `(x, y, w, h)` and closes it.
fn leave_at(h: &mut Harness, c: &mut TestClient, (x, y): (i32, i32), (w, hh): (i32, i32)) {
    let t = h.map_toplevel(c, APP, 300, 200);
    h.resize_self(c, t, w, hh);
    h.state.move_to(t.id, x, y);
    close(h, c, t);
}

fn frame_of(h: &Harness, t: Toplevel) -> crate::raster::Rect {
    h.state.win(t.id).unwrap().frame_rect()
}

/// The frame around the client size the compositor last asked of `t`.
fn asked(h: &Harness, t: Toplevel) -> (i32, i32) {
    let win = h.state.win(t.id).unwrap();
    let size = win.xdg().and_then(|x| x.with_pending_state(|s| s.size));
    let (cw, ch) = size.map_or((0, 0), |s| (s.w, s.h));
    let m = &h.state.config.theme.metrics;
    reprisekit::classic::frame::FrameGeometry::layout(m, cw, ch, false, win.chrome()).total
}

#[test]
fn a_program_reopens_where_and_at_the_size_its_last_window_closed() {
    let mut h = one_screen();
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, APP, 300, 200);
    let first = frame_of(&h, t);
    h.resize_self(&mut c, t, 500, 400);
    let left = frame_of(&h, t);
    h.state.move_to(t.id, 400, 300);
    close(&mut h, &mut c, t);
    let t = h.map_toplevel(&mut c, APP, 300, 200);
    let r = frame_of(&h, t);
    assert_eq!((r.x, r.y), (400, 300));
    assert_ne!((first.x, first.y), (400, 300), "not where a new one opens");
    assert_eq!(
        asked(&h, t),
        (left.width, left.height),
        "the size it was left at"
    );
    let again = h.map_toplevel(&mut c, APP, 300, 200);
    let r2 = frame_of(&h, again);
    assert!(
        r2.x > r.x && r2.y > r.y,
        "a second window cascades from the first: {r2:?} after {r:?}"
    );
}

#[test]
fn a_maximized_program_reopens_maximized_on_the_screen_it_was_on() {
    let mut h = one_screen();
    let two = h.output("TEST-2", 1280, 1024);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, APP, 300, 200);
    let b = h.state.layout_box(Some(&two));
    h.state.move_to(t.id, b.x + 50, b.y + 60);
    h.state.set_maximized(t.id, true);
    h.roundtrip(&mut c);
    close(&mut h, &mut c, t);
    h.pointer_to(10.0, 10.0);
    let t = h.map_toplevel(&mut c, APP, 300, 200);
    h.roundtrip(&mut c);
    let win = h.state.win(t.id).unwrap();
    assert!(win.maximized, "maximized again");
    let r = win.frame_rect();
    assert!(
        b.contains(r.x + r.width / 2, r.y + r.height / 2),
        "on TEST-2: {r:?}"
    );
    let saved = win.saved.expect("a frame to restore to");
    assert_eq!(
        (saved.x, saved.y),
        (b.x + 50, b.y + 60),
        "restores where it was"
    );
}

#[test]
fn each_set_of_screens_keeps_its_own_place() {
    let mut h = one_screen();
    let mut c = h.connect();
    leave_at(&mut h, &mut c, (400, 300), (500, 400));
    let undocked = h.state.output_set();
    let two = h.output("TEST-2", 1280, 1024);
    assert_ne!(h.state.output_set(), undocked);
    let t = h.map_toplevel(&mut c, APP, 300, 200);
    assert_ne!(
        (frame_of(&h, t).x, frame_of(&h, t).y),
        (400, 300),
        "nothing kept for two screens yet"
    );
    let b = h.state.layout_box(Some(&two));
    h.state.move_to(t.id, b.x + 100, b.y + 100);
    close(&mut h, &mut c, t);
    h.state.scene.remove_output(&two);
    h.state.output_removed(&two);
    let t = h.map_toplevel(&mut c, APP, 300, 200);
    assert_eq!((frame_of(&h, t).x, frame_of(&h, t).y), (400, 300));
    close(&mut h, &mut c, t);
    let two = h.output("TEST-2", 1280, 1024);
    let b = h.state.layout_box(Some(&two));
    let t = h.map_toplevel(&mut c, APP, 300, 200);
    assert_eq!(
        (frame_of(&h, t).x, frame_of(&h, t).y),
        (b.x + 100, b.y + 100),
        "docked again: the docked place"
    );
}

fn keep(h: &mut Harness, output: &str, x: i32, y: i32) {
    let set = h.state.output_set();
    h.state.wm.kept.store(
        &set,
        APP,
        Remembered {
            output: output.into(),
            x,
            y,
            width: 500,
            height: 400,
            maximized: false,
            text_size: 100,
        },
    );
}

#[test]
fn a_kept_place_off_every_screen_opens_at_the_default_place() {
    let mut h = one_screen();
    let mut c = h.connect();
    let fresh = h.map_toplevel(&mut c, "org.example.Other", 300, 200);
    let default = (frame_of(&h, fresh).x, frame_of(&h, fresh).y);
    close(&mut h, &mut c, fresh);
    for (output, x, y, why) in [
        ("TEST-1", 2500, 100, "right of the screen"),
        ("TEST-1", 100, -200, "above the screen"),
        ("GONE-1", 100, 100, "on a screen not in the layout"),
    ] {
        keep(&mut h, output, x, y);
        let t = h.map_toplevel(&mut c, APP, 300, 200);
        let r = frame_of(&h, t);
        assert_eq!((r.x, r.y), default, "{why}");
        assert_eq!(asked(&h, t), (500, 400), "{why}: the kept size stays");
        close(&mut h, &mut c, t);
    }
}

#[test]
fn a_kept_place_with_its_title_bar_on_screen_moves_inside_the_work_area() {
    let mut h = one_screen();
    let mut c = h.connect();
    keep(&mut h, "TEST-1", 1700, 900);
    let t = h.map_toplevel(&mut c, APP, 300, 200);
    let r = frame_of(&h, t);
    let area = h.state.usable_area(h.state.scene.outputs().first());
    assert_eq!((r.x, r.y), (area.right() - 500, area.bottom() - 400));
}

#[test]
fn a_window_rule_place_and_size_beat_the_kept_ones() {
    let config = crate::config::Config {
        window_rules: vec![crate::windowrules::WindowRule {
            app_id: APP.into(),
            position: Some((40, 50)),
            size: Some((700, 500)),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut h = Harness::with_config(config);
    h.output("TEST-1", 1920, 1080);
    let mut c = h.connect();
    leave_at(&mut h, &mut c, (400, 300), (500, 400));
    let t = h.map_toplevel(&mut c, APP, 300, 200);
    let r = frame_of(&h, t);
    assert_eq!((r.x, r.y), (40, 50));
    assert_eq!(asked(&h, t), (700, 500));
}

#[test]
#[expect(clippy::field_reassign_with_default)]
fn with_remembering_off_nothing_is_kept_or_used() {
    let mut config = crate::config::Config::default();
    config.remember_windows = false;
    let mut h = Harness::with_config(config);
    h.output("TEST-1", 1920, 1080);
    let mut c = h.connect();
    leave_at(&mut h, &mut c, (400, 300), (500, 400));
    assert_eq!(h.state.wm.kept.get(&h.state.output_set(), APP), None);
    keep(&mut h, "TEST-1", 400, 300);
    let t = h.map_toplevel(&mut c, APP, 300, 200);
    assert_ne!((frame_of(&h, t).x, frame_of(&h, t).y), (400, 300));
}

#[test]
fn forgetting_drops_every_kept_place() {
    let mut h = one_screen();
    let mut c = h.connect();
    leave_at(&mut h, &mut c, (400, 300), (500, 400));
    h.state.forget_window_frames();
    assert_eq!(h.state.wm.kept.get(&h.state.output_set(), APP), None);
    let t = h.map_toplevel(&mut c, APP, 300, 200);
    assert_ne!((frame_of(&h, t).x, frame_of(&h, t).y), (400, 300));
}
