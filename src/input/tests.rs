use super::button::{DesktopLeft, caption_double, desktop_left, press_closes_start_panel};
use super::move_resize::top_below_top_bar;
use super::*;
use crate::window::SizeLimits;
use reprisekit::pointer::DoubleClick;

#[test]
fn a_named_cursor_change_asks_only_the_outputs_showing_the_cursor() {
    let mut h = crate::harness::Harness::new();
    h.output("TEST-1", 1024, 768);
    let b = h.output("TEST-2", 1024, 768);
    let b_at = reprisescene::output_rect(&b).expect("TEST-2's place").loc;
    h.pointer_to(f64::from(b_at.x + 100), f64::from(b_at.y + 100));
    h.state.zoom(crate::magnifier::ZoomStep::In);
    crate::backend::take_asks();
    h.state.set_cursor("move");
    assert_eq!(
        crate::backend::take_asks(),
        vec![("TEST-2".to_string(), crate::backend::Ask::Cursor)],
        "only the zoomed TEST-2 follows the cursor; no output has drawn it yet"
    );
}

#[test]
fn caption_presses_pair_on_one_part_within_the_configured_time_and_slop() {
    let m = reprisekit::theme::Metrics {
        double_click_ms: 900,
        ..Default::default()
    };
    let w = WindowId::next();
    let mut clicks = DoubleClick::default();
    let mut press =
        |hit, button, x, time| caption_double(&mut clicks, w, hit, button, m.click(x, 10, time));
    assert!(!press(FrameHit::Title, BTN_LEFT, 100, 1000));
    assert!(press(FrameHit::Title, BTN_LEFT, 104, 1800), "800 ms, 4 px");
    assert!(!press(FrameHit::Title, BTN_LEFT, 100, 3000));
    assert!(!press(FrameHit::Icon, BTN_LEFT, 100, 3100), "another part");
    assert!(!press(FrameHit::Icon, BTN_RIGHT, 100, 3200));
    assert!(
        !press(FrameHit::Icon, BTN_LEFT, 100, 3300),
        "a right press between"
    );
    assert!(
        press(FrameHit::Icon, BTN_LEFT, 100, 3400),
        "the icon pairs too"
    );
    assert!(!press(FrameHit::Title, BTN_LEFT, 100, 5000));
    assert!(!press(FrameHit::Title, BTN_LEFT, 105, 5100), "5 px off");
}

#[test]
fn a_cancel_on_another_window_keeps_the_caption_click_pairing() {
    let m = reprisekit::theme::Metrics::default();
    let (w, other) = (WindowId::next(), WindowId::next());
    let mut clicks = DoubleClick::default();
    caption_double(
        &mut clicks,
        w,
        FrameHit::Title,
        BTN_LEFT,
        m.click(100, 10, 1000),
    );
    caption_cancelled(&mut clicks, other);
    assert!(
        caption_double(
            &mut clicks,
            w,
            FrameHit::Title,
            BTN_LEFT,
            m.click(100, 10, 1100)
        ),
        "another window's cancel leaves the first click"
    );
    caption_double(
        &mut clicks,
        w,
        FrameHit::Title,
        BTN_LEFT,
        m.click(100, 10, 3000),
    );
    caption_cancelled(&mut clicks, w);
    assert!(
        !caption_double(
            &mut clicks,
            w,
            FrameHit::Title,
            BTN_LEFT,
            m.click(100, 10, 3100)
        ),
        "its own window's cancel forgets it"
    );
}

#[test]
fn a_drop_restores_to_the_rect_from_before_the_drag() {
    // Grabbed far right and dropped at the left edge: the frame at
    // the release starts at -700.
    let found = Rect::new(400, 300, 800, 600);
    let free = MoveStart {
        rect: found,
        maximized: false,
        restore: None,
    };
    assert_eq!(free.restore_rect(None), found, "the rect the drag found");
    let pre_snap = Rect::new(100, 120, 640, 480);
    assert_eq!(
        free.restore_rect(Some(pre_snap)),
        pre_snap,
        "the size from before the snap"
    );
    let maximized = MoveStart {
        rect: Rect::new(0, 0, 1920, 1032),
        maximized: true,
        restore: Some(pre_snap),
    };
    assert_eq!(
        maximized.restore_rect(None),
        pre_snap,
        "the rect a restore from maximized went to"
    );
}

#[test]
fn a_second_press_ends_every_gesture_a_button_holds() {
    let scene = crate::compositor::DeskScene::new();
    let bar = scene.root();
    let held = [
        Interaction::ArrowPress { bar },
        Interaction::QuickPress {
            bar,
            index: 0,
            p0: (0.0, 0.0),
            grab: (0, 0),
            dragging: false,
        },
        Interaction::TaskPress {
            bar,
            key: crate::taskbar::TaskKey::Window(WindowId::next()),
            along0: 0.0,
            grab: 0,
            dragging: false,
        },
        Interaction::Band {
            bar,
            band: crate::taskbar::Band::Tasks,
            dx: 0,
        },
        Interaction::NotifyPress,
        Interaction::DialogPress,
        Interaction::Marquee,
        Interaction::Stir,
        Interaction::DisplaysSlide,
        Interaction::BarWidth { bar, grab: 0 },
        Interaction::Drag {
            x0: 0.0,
            y0: 0.0,
            started: false,
        },
        Interaction::Resize {
            window: WindowId::next(),
            edges: Edges::LEFT,
            start: Rect::new(0, 0, 10, 10),
            cx: 0.0,
            cy: 0.0,
            keyboard: None,
        },
    ];
    for i in held {
        assert!(i.ends_on_press(), "{i:?} ends");
    }
    let keyboard = Interaction::Resize {
        window: WindowId::next(),
        edges: Edges::NONE,
        start: Rect::new(0, 0, 10, 10),
        cx: 0.0,
        cy: 0.0,
        keyboard: Some(Rect::new(0, 0, 10, 10)),
    };
    assert!(
        !keyboard.ends_on_press(),
        "a window-menu size commits on a press"
    );
    assert!(!Interaction::Idle.ends_on_press());
}

#[test]
fn fluid_press_on_empty_desktop_stirs_not_marquee() {
    assert_eq!(desktop_left(true, None, false), DesktopLeft::Stir);
}

#[test]
fn fluid_double_click_on_empty_desktop_clears() {
    assert_eq!(desktop_left(true, None, true), DesktopLeft::Clear);
}

#[test]
fn fluid_press_on_icon_selects_as_before() {
    assert_eq!(desktop_left(true, Some(2), false), DesktopLeft::Icon);
    assert_eq!(desktop_left(true, Some(2), true), DesktopLeft::Icon);
}

#[test]
fn a_press_where_no_fluid_takes_the_stir_marquees() {
    assert_eq!(desktop_left(false, None, false), DesktopLeft::Marquee);
    assert_eq!(desktop_left(false, None, true), DesktopLeft::Marquee);
    assert_eq!(desktop_left(false, Some(0), false), DesktopLeft::Icon);
}

#[test]
fn the_pointer_stays_on_the_outputs() {
    let area = Rectangle::new((0, 0).into(), (1920, 1080).into());
    assert_eq!(clamp_to((-5.0, 20.0).into(), area), (0.0, 20.0).into());
    assert_eq!(
        clamp_to((5000.0, 5000.0).into(), area),
        (1919.0, 1079.0).into()
    );
    assert_eq!(clamp_to((10.5, 7.25).into(), area), (10.5, 7.25).into());
}

#[test]
fn a_press_on_a_popup_keeps_the_start_panel_and_one_on_neither_closes_it() {
    assert!(!press_closes_start_panel(&Hit::Menu {
        depth: 0,
        row: Some(1)
    }));
    assert!(!press_closes_start_panel(&Hit::StartPanel { spot: None }));
    assert!(press_closes_start_panel(&Hit::Nothing));
    assert!(press_closes_start_panel(&Hit::Desktop { index: None }));
}

use crate::theme::Metrics;

fn frame_pad(m: &Metrics) -> (i32, i32) {
    (2 * m.frame_width, 2 * m.frame_width + m.caption_height)
}

#[test]
fn resize_rect_left_edge_reanchors_at_max_width() {
    let m = Metrics::default();
    let start = Rect::new(100, 50, 208, 126);
    let limits = SizeLimits {
        min: (1, 1),
        max: (Some(150), None),
    };
    let r = resize_rect(start, Edges::LEFT, -50, 0, limits, frame_pad(&m));
    assert_eq!(r.right(), start.right());
    assert_eq!(r.width, 150 + 2 * m.frame_width);
}

#[test]
fn resize_rect_top_edge_reanchors_at_max_height() {
    let m = Metrics::default();
    let start = Rect::new(100, 50, 208, 126);
    let limits = SizeLimits {
        min: (1, 1),
        max: (None, Some(80)),
    };
    let r = resize_rect(start, Edges::TOP, 0, -80, limits, frame_pad(&m));
    assert_eq!(r.bottom(), start.bottom());
    assert_eq!(r.height, 80 + 2 * m.frame_width + m.caption_height);
}

#[test]
fn a_top_edge_drag_stops_at_a_top_bars_foot() {
    use reprisekit::taskbar::Edge;
    let out = Rect::new(0, 0, 800, 600);
    let top = crate::barplace::place(out, Edge::Top, 30);
    let r = Rect::new(100, 10, 200, 190);
    assert_eq!(
        top_below_top_bar(r, Edges::TOP | Edges::LEFT, &top),
        Rect::new(100, 30, 200, 170)
    );
    let low = Rect::new(100, 40, 200, 160);
    assert_eq!(top_below_top_bar(low, Edges::TOP, &top), low);
    assert_eq!(top_below_top_bar(r, Edges::BOTTOM, &top), r);
    let bottom = crate::barplace::place(out, Edge::Bottom, 30);
    assert_eq!(top_below_top_bar(r, Edges::TOP, &bottom), r);
}

#[test]
fn is_resize_of_only_matches_its_own_window() {
    let a = WindowId::next();
    let b = WindowId::next();
    let r = Interaction::Resize {
        window: a,
        edges: Edges::LEFT,
        start: Rect::new(0, 0, 10, 10),
        cx: 0.0,
        cy: 0.0,
        keyboard: None,
    };
    assert!(r.is_resize_of(a));
    assert!(!r.is_resize_of(b));
    assert!(!Interaction::Idle.is_resize_of(a));
    assert!(
        !Interaction::Move {
            window: a,
            dx: 0.0,
            dy: 0.0,
            origin: None,
            was_snapped: None,
            start: MoveStart::default(),
            press: (0.0, 0.0),
            snap: None,
            armed: false,
            bar: false,
        }
        .is_resize_of(a)
    );
    assert!(
        !Interaction::PressButton {
            window: a,
            button: CaptionButton::Close
        }
        .is_resize_of(a)
    );
}

#[test]
fn resize_rect_takes_client_limits_at_i32_max() {
    let m = Metrics::default();
    let start = Rect::new(100, 50, 208, 126);
    let pad = frame_pad(&m);
    let limits = crate::window::client_limits((i32::MAX, i32::MAX, i32::MAX, i32::MAX));
    let r = resize_rect(start, Edges::LEFT, 10, 0, limits, pad);
    assert_eq!(r.width, 32767 + pad.0);
    assert_eq!(r.right(), start.right());
}

#[test]
fn resize_rect_right_edge_min_width_keeps_x() {
    let m = Metrics::default();
    let start = Rect::new(100, 50, 208, 126);
    let limits = SizeLimits {
        min: (300, 1),
        max: (None, None),
    };
    let r = resize_rect(start, Edges::RIGHT, -100, 0, limits, frame_pad(&m));
    assert_eq!(r.x, start.x);
    assert_eq!(r.width, 300 + 2 * m.frame_width);
}

/// A Classic window on TEST-1 of two outputs with its close button held down; the asks made so
/// far are taken.
fn held_close() -> (
    crate::harness::Harness,
    crate::harness::TestClient,
    WindowId,
) {
    let mut h = crate::harness::Harness::new();
    h.output("TEST-1", 1024, 768);
    h.output("TEST-2", 1024, 768);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "test.app", 300, 200);
    let w = h.state.win(t.id).expect("the window");
    let b = w.geom.button_rect(CaptionButton::Close);
    let (x, y) = (w.x + b.x + b.width / 2, w.y + b.y + b.height / 2);
    h.pointer_to(f64::from(x), f64::from(y));
    h.button(crate::harness::LEFT, true);
    assert!(matches!(
        h.state.interaction,
        Interaction::PressButton {
            button: CaptionButton::Close,
            ..
        }
    ));
    crate::backend::take_asks();
    (h, c, t.id)
}

fn scene_asks(asked: &[(String, crate::backend::Ask)]) -> Vec<&str> {
    (asked.iter())
        .filter(|(_, a)| *a == crate::backend::Ask::Scene)
        .map(|(o, _)| o.as_str())
        .collect()
}

#[test]
fn a_held_caption_button_left_by_the_pointer_asks_its_output_for_a_frame() {
    let (mut h, _c, w) = held_close();
    let r = h.state.win(w).expect("the window").frame_rect();
    h.pointer_to(f64::from(r.x + r.width / 2), f64::from(r.y + r.height / 2));
    assert_eq!(h.state.win(w).and_then(|w| w.pressed), None);
    let asked = crate::backend::take_asks();
    assert_eq!(scene_asks(&asked), ["TEST-1"], "{asked:?}");
}

#[test]
fn a_cancelled_caption_press_asks_its_output_for_a_frame() {
    let (mut h, _c, w) = held_close();
    h.state.cancel_interaction();
    assert_eq!(h.state.win(w).and_then(|w| w.pressed), None);
    let asked = crate::backend::take_asks();
    assert_eq!(scene_asks(&asked), ["TEST-1"], "{asked:?}");
}
