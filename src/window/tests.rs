#[test]
fn a_batch_draws_each_frame_once_at_the_outer_end() {
    use super::{FrameBatch, WindowId};
    let (a, b) = (WindowId::next(), WindowId::next());
    let mut batch = FrameBatch::default();
    assert!(!batch.put_off(a), "outside a batch a frame draws at once");
    assert!(batch.begin());
    assert!(batch.put_off(a) && batch.put_off(b) && batch.put_off(a));
    assert!(
        !batch.begin(),
        "a nested batch leaves the drawing to the outer one"
    );
    assert!(batch.put_off(b));
    assert_eq!(batch.end(), vec![a, b]);
    assert!(!batch.put_off(a), "the batch is closed after its end");
}

#[test]
fn a_frame_never_starts_above_a_top_bars_foot() {
    use super::below_top_bar;
    use reprisekit::taskbar::Edge;
    let out = crate::raster::Rect::new(0, 0, 800, 600);
    let top = crate::barplace::place(out, Edge::Top, 30);
    assert_eq!(below_top_bar(5, &top), 30);
    assert_eq!(below_top_bar(120, &top), 120);
    let bottom = crate::barplace::place(out, Edge::Bottom, 30);
    assert_eq!(below_top_bar(-4, &bottom), -4);
}

#[test]
fn a_rect_keeps_its_size_when_a_top_bar_pushes_it_down() {
    use super::rect_below_top_bar;
    use crate::raster::Rect;
    use reprisekit::taskbar::Edge;
    let out = Rect::new(0, 0, 800, 600);
    let r = Rect::new(40, 5, 300, 200);
    let top = crate::barplace::place(out, Edge::Top, 30);
    assert_eq!(rect_below_top_bar(r, &top), Rect::new(40, 30, 300, 200));
    let low = Rect::new(40, 90, 300, 200);
    assert_eq!(rect_below_top_bar(low, &top), low);
    for edge in [Edge::Bottom, Edge::Left, Edge::Right] {
        let p = crate::barplace::place(out, edge, 30);
        assert_eq!(rect_below_top_bar(r, &p), r, "{edge:?}");
    }
}

#[test]
fn a_frame_may_start_under_an_auto_hidden_top_bars_reveal_line() {
    use super::below_top_bar;
    use reprisekit::taskbar::Edge;
    let out = crate::raster::Rect::new(0, 0, 800, 600);
    for hide in [0.0, 1.0] {
        let top = crate::barplace::place_hiding(out, Edge::Top, 30, Some(hide));
        assert_eq!(
            below_top_bar(0, &top),
            2,
            "only the reveal line, shown or hidden ({hide})"
        );
    }
}

#[test]
fn molten_stacks_the_shadow_over_the_glass() {
    use super::draw::stack_shadow;
    use crate::compositor::DeskScene;
    use reprisescene::NodeId;
    let mut scene = DeskScene::new();
    let chrome = scene.tree_create(scene.root());
    let shadow = std::array::from_fn(|_| scene.buffer_create(chrome));
    let backdrop = scene.tree_create(chrome);
    let frame: [_; 4] = std::array::from_fn(|_| scene.buffer_create(chrome));
    let ids = |shadow_first: bool| -> Vec<NodeId> {
        let s = shadow.iter().map(|&b| NodeId::from(b));
        let f = frame.iter().map(|&b| NodeId::from(b));
        let b = std::iter::once(NodeId::from(backdrop));
        if shadow_first {
            s.chain(b).chain(f).collect()
        } else {
            b.chain(s).chain(f).collect()
        }
    };
    assert_eq!(
        scene.children(chrome),
        ids(true),
        "made with the shadow lowest"
    );
    stack_shadow(&mut scene, &shadow, backdrop, true);
    assert_eq!(
        scene.children(chrome),
        ids(false),
        "Molten: over the backdrop"
    );
    stack_shadow(&mut scene, &shadow, backdrop, true);
    assert_eq!(scene.children(chrome), ids(false), "again: unchanged");
    stack_shadow(&mut scene, &shadow, backdrop, false);
    assert_eq!(scene.children(chrome), ids(true), "Frosted: under it again");
}

#[test]
fn the_shadow_cache_draws_each_key_once() {
    use super::ShadowCache;
    use reprisekit::glass::shadow;
    use std::rc::Rc;
    let mut cache = ShadowCache::default();
    let a = cache.pieces(1, true, false);
    assert!(
        Rc::ptr_eq(&a, &cache.pieces(1, true, false)),
        "same key: one drawing"
    );
    let keys = [(1, false, false), (2, true, false), (1, true, true)];
    for (scale, active, square) in keys {
        assert!(
            !Rc::ptr_eq(&a, &cache.pieces(scale, active, square)),
            "{scale} {active} {square}: a drawing of its own"
        );
    }
    assert_eq!(cache.0.len(), 4, "four keys, four drawings");
    let sq = cache.pieces(1, true, true);
    for i in 0..8 {
        assert_eq!(
            a[i].pixels,
            shadow::piece(true, i, 1).pixels,
            "rounded piece {i}"
        );
        assert_eq!(
            sq[i].pixels,
            reprisekit::glass::square::shadow_piece(true, i, 1).pixels,
            "square piece {i}"
        );
    }
    let hi = cache.pieces(2, true, false);
    assert_eq!(
        (hi[0].width, hi[0].height),
        (a[0].width * 2, a[0].height * 2),
        "scale 2 draws at twice the size"
    );
}

#[test]
fn x11_geometry_saturates() {
    assert_eq!(super::x11_geometry(10, -20, 300, 200), (10, -20, 300, 200));
    assert_eq!(
        super::x11_geometry(40_000, -40_000, 0, 70_000),
        (i16::MAX, i16::MIN, 1, u16::MAX)
    );
}

#[test]
fn client_extent_stays_within_1_to_32767() {
    assert_eq!(super::client_extent(640, 480), (640, 480));
    assert_eq!(super::client_extent(0, -5), (1, 1));
    assert_eq!(super::client_extent(i32::MAX, 40_000), (32767, 32767));
}

#[test]
fn client_limits_keep_zero_max_unbounded_and_cap_at_32767() {
    let limits = |min, max| super::SizeLimits { min, max };
    assert_eq!(
        super::client_limits((320, 240, 640, 480)),
        limits((320, 240), (Some(640), Some(480)))
    );
    assert_eq!(
        super::client_limits((0, -1, 0, -5)),
        limits((1, 1), (None, None))
    );
    assert_eq!(
        super::client_limits((i32::MAX, i32::MAX, i32::MAX, 40_000)),
        limits((32767, 32767), (Some(32767), Some(32767)))
    );
}

use super::draw::{client_glass_rects, client_holes};
use super::owners::{modal_end, owner_chain_has, walk_owned};
use super::place::{grown_back_inside, retheme_back_inside};
use super::*;
use crate::raster::Rect;
use crate::theme::Metrics;
use reprisekit::classic::frame::Edges;

#[test]
fn the_frames_glass_leaves_clear_only_the_client_outside_its_blur_region() {
    let client = Rect::new(8, 30, 400, 300);
    assert_eq!(
        client_holes(client, &[]),
        vec![client],
        "no region: the whole client"
    );
    let holes = client_holes(client, &[Rect::new(0, 0, 400, 78)]);
    assert_eq!(holes, vec![Rect::new(8, 108, 400, 222)], "below the band");
    let holes = client_holes(client, &[Rect::new(0, 0, 400, 300)]);
    assert!(holes.is_empty(), "a region over all of it: {holes:?}");
}

#[test]
fn a_huge_clients_full_blur_region_draws_glass_no_larger_than_the_layout() {
    let client = (32767, 32767);
    let rects = client_glass_rects(client, &[Rect::new(0, 0, 32767, 32767)], (1920, 1080));
    assert_eq!(rects, vec![Rect::new(0, 0, 1920, 1080)]);
    let rects = client_glass_rects(
        (400, 300),
        &[
            Rect::new(0, 0, 400, 78),
            Rect::new(390, 290, 50, 50),
            Rect::new(500, 0, 10, 10),
        ],
        (1920, 1080),
    );
    assert_eq!(
        rects,
        vec![Rect::new(0, 0, 400, 78), Rect::new(390, 290, 10, 10)],
        "clipped to the client, the rect outside it dropped"
    );
}

#[test]
fn modal_end_follows_the_chain_to_its_last_dialog() {
    let modal = |w: u32| match w {
        0 => Some(1),
        1 => Some(2),
        _ => None,
    };
    assert_eq!(modal_end(0, modal), 2);
    assert_eq!(modal_end(1, modal), 2);
    assert_eq!(modal_end(3, modal), 3);
}

#[test]
fn modal_end_stops_on_a_parent_cycle() {
    let modal = |w: u32| Some(if w == 2 { 1 } else { w + 1 });
    assert_eq!(modal_end(0, modal), 2);
}

#[test]
fn owner_chain_has_every_owner_up_the_chain_and_no_other() {
    let parent = |w: u32| match w {
        2 => Some(1),
        1 => Some(0),
        _ => None,
    };
    assert!(owner_chain_has(2, 1, parent), "expected 1 to own 2");
    assert!(owner_chain_has(2, 0, parent), "expected 0 to own 2");
    assert!(!owner_chain_has(1, 2, parent), "expected 2 not to own 1");
    assert!(!owner_chain_has(2, 3, parent), "expected 3 not to own 2");
    assert!(
        !owner_chain_has(2, 2, parent),
        "expected 2 not to own itself"
    );
}

#[test]
fn owner_chain_has_ends_on_a_parent_cycle() {
    let parent = |w: u32| Some(if w == 2 { 1 } else { w + 1 });
    assert!(!owner_chain_has(0, 7, parent), "expected 7 not to own 0");
}

#[test]
fn walk_owned_visits_owners_first_and_passes_their_state_down() {
    let kids = |w: usize| match w {
        0 => vec![1, 4],
        1 => vec![2, 3],
        _ => vec![],
    };
    let mut seen = Vec::new();
    walk_owned(0, 0, kids, |w, depth| {
        seen.push((w, depth));
        depth + 1
    });
    assert_eq!(seen, [(1, 0), (2, 1), (3, 1), (4, 0)]);
}

#[test]
fn walk_owned_visits_each_window_of_a_parent_cycle_once() {
    let kids = |w: usize| match w {
        0 => vec![1],
        1 => vec![2],
        _ => vec![0, 1],
    };
    let mut seen = Vec::new();
    walk_owned(0, (), kids, |w, ()| seen.push(w));
    assert_eq!(seen, [1, 2]);
}

#[test]
fn walk_owned_follows_a_100000_deep_chain() {
    let depth = 100_000;
    let mut last = 0;
    walk_owned(
        0,
        (),
        |w: usize| if w < depth { vec![w + 1] } else { vec![] },
        |w, ()| last = w,
    );
    assert_eq!(last, depth);
}

#[test]
fn chrome_follows_ownership_and_size_hints() {
    use reprisekit::classic::frame::{ButtonSet, Chrome};
    let none = DialogTag::None;
    assert_eq!(chrome_for(false, true, true, none), Chrome::WINDOW);
    assert_eq!(chrome_for(true, false, true, none), Chrome::DIALOG);
    let c = chrome_for(true, true, true, none);
    assert_eq!(c.buttons, ButtonSet::Close);
    assert!(c.resizable);
    let c = chrome_for(false, false, true, none);
    assert_eq!(c.buttons, ButtonSet::MinMaxClose);
    assert!(!c.resizable);
    assert!(!chrome_for(false, true, false, none).decorated);
}

#[test]
fn a_tagged_dialog_gets_its_caption_owned_or_not() {
    use reprisekit::classic::frame::Chrome;
    let (help, prompt) = (DialogTag::Help, DialogTag::Prompt);
    assert_eq!(chrome_for(false, false, true, help), Chrome::HELP_DIALOG);
    assert_eq!(chrome_for(true, false, true, help), Chrome::HELP_DIALOG);
    assert!(!chrome_for(false, false, false, help).decorated);
    assert_eq!(chrome_for(false, false, true, prompt), Chrome::PROMPT);
    assert_eq!(chrome_for(true, false, true, prompt), Chrome::PROMPT);
    assert_eq!(DialogTag::from_tag("prompt"), prompt);
    assert_eq!(DialogTag::from_tag("help-dialog"), help);
    assert_eq!(DialogTag::from_tag("dialog"), DialogTag::None);
}

#[test]
fn a_fixed_dialog_has_the_close_box_alone() {
    use reprisekit::classic::frame::{ButtonSet, Chrome};
    let fixed = DialogTag::from_tag("fixed-dialog");
    assert_eq!(fixed, DialogTag::Fixed);
    let c = chrome_for(false, false, true, fixed);
    assert_eq!(c, Chrome::DIALOG);
    assert_eq!(c.buttons, ButtonSet::Close);
}

#[test]
fn cascade_steps_and_wraps() {
    let m = Metrics::default();
    let area = Rect::new(100, 50, 800, 600);
    assert_eq!(cascade_position(0, area, &m), (132, 82));
    assert_eq!(cascade_position(1, area, &m), (156, 106));
    assert_eq!(cascade_position(10, area, &m), (132, 82));
    let small = Rect::new(0, 0, 300, 200);
    let (x, y) = cascade_position(9, small, &m);
    assert!(x < 300 && y + m.caption_height + 2 * m.frame_width <= 200);
}

#[test]
fn cascade_uses_configured_metrics() {
    let m = Metrics {
        cascade_origin: 10,
        cascade_step: 5,
        ..Default::default()
    };
    let area = Rect::new(0, 0, 800, 600);
    assert_eq!(cascade_position(0, area, &m), (10, 10));
    assert_eq!(cascade_position(3, area, &m), (25, 25));
    assert_eq!(cascade_position(10, area, &m), (10, 10));
}

#[test]
fn an_apps_next_window_cascades_from_its_newest_and_wraps_at_the_edge() {
    let m = Metrics::default();
    let area = Rect::new(0, 0, 1920, 1050);
    assert_eq!(cascade_from((300, 200), (640, 480), area, &m), (324, 224));
    assert_eq!(
        cascade_from((1270, 200), (640, 480), area, &m),
        (32, 32),
        "past the right edge"
    );
    assert_eq!(
        cascade_from((300, 560), (640, 480), area, &m),
        (32, 32),
        "past the bottom"
    );
}

#[test]
fn a_remembered_frame_is_kept_inside_the_work_area() {
    let area = Rect::new(1920, 0, 1280, 994);
    assert_eq!(keep_inside((2000, 100), (640, 480), area), (2000, 100));
    assert_eq!(keep_inside((3000, 900), (640, 480), area), (2560, 514));
    assert_eq!(keep_inside((-50, -20), (640, 480), area), (1920, 0));
    assert_eq!(keep_inside((2000, 100), (2000, 480), area), (1920, 100));
    assert_eq!(fit_size((2000, 600), area), (1280, 600));
    assert_eq!(fit_size((800, 1200), area), (800, 994));
}

#[test]
fn a_remembered_frame_reopens_at_the_same_size() {
    let m = Metrics::default();
    let screen = Rect::new(0, 0, 1920, 1080);
    let area = Rect::new(0, 0, 1920, 1050);
    let want = crate::windowrules::Remembered {
        output: "Virtual-1".into(),
        x: 32,
        y: 32,
        width: 831,
        height: 678,
        maximized: false,
        text_size: 100,
    }
    .want(true, 100);
    for decorated in [false, true] {
        let chrome = chrome_for(false, true, decorated, DialogTag::None);
        let (w, h) = first_configure_size(&m, &want, OPEN, screen, area, chrome)
            .expect("a remembered frame asks for a size");
        assert_eq!(
            FrameGeometry::layout(&m, w, h, false, chrome).total,
            (831, 678),
            "decorated: {decorated}"
        );
    }
    let csd = chrome_for(false, true, false, DialogTag::None);
    assert_eq!(
        first_configure_size(&m, &want, OPEN, screen, area, csd),
        Some((831, 678))
    );
}

const OPEN: SizeLimits = SizeLimits {
    min: (1, 1),
    max: (None, None),
};

#[test]
fn a_remembered_frame_under_the_minimum_reopens_at_the_minimum() {
    let m = Metrics::default();
    let screen = Rect::new(0, 0, 1920, 1080);
    let area = Rect::new(0, 0, 1920, 1050);
    let remembered = |width, height| {
        crate::windowrules::Remembered {
            output: "Virtual-1".into(),
            x: 32,
            y: 32,
            width,
            height,
            maximized: false,
            text_size: 100,
        }
        .want(true, 100)
    };
    let limits = SizeLimits {
        min: (544, 528),
        max: (None, None),
    };
    let csd = chrome_for(false, true, false, DialogTag::None);
    let size = |want| first_configure_size(&m, &want, limits, screen, area, csd);
    assert_eq!(size(remembered(760, 500)), Some((760, 528)));
    assert_eq!(size(remembered(400, 300)), Some((544, 528)));
    assert_eq!(size(remembered(2400, 1400)), Some((1920, 1050)));
    let tall = SizeLimits {
        min: (1100, 1300),
        max: (None, None),
    };
    assert_eq!(
        first_configure_size(&m, &remembered(760, 500), tall, screen, area, csd),
        Some((1100, 1050)),
        "a minimum past the work area opens as tall as the area"
    );
    let decorated = chrome_for(false, true, true, DialogTag::None);
    assert_eq!(
        first_configure_size(&m, &remembered(760, 500), limits, screen, area, decorated)
            .map(|s| s.1),
        Some(528),
        "the minimum is the client's, inside the frame"
    );
}

/// A window growing with the text size past the taskbar moves up as far as it overran; one
/// the user left partly off screen, or one that shrank, stays put.
#[test]
fn a_window_that_grew_past_the_work_area_moves_back_in() {
    let area = Rect::new(0, 0, 1920, 1050);
    let was = Rect::new(640, 160, 620, 700);
    let now = Rect::new(640, 160, 820, 960);
    assert_eq!(grown_back_inside(was, now, area), Some((640, 90)));
    let fits = Rect::new(640, 160, 700, 800);
    assert_eq!(grown_back_inside(was, fits, area), None);
    let off = Rect::new(1500, 160, 620, 700);
    assert_eq!(
        grown_back_inside(off, Rect { width: 820, ..off }, area),
        None
    );
    assert_eq!(grown_back_inside(now, was, area), None);
}

#[test]
fn a_window_on_screen_comes_back_inside_only_after_its_frame_or_the_taskbar_grows() {
    let screen = Rect::new(0, 0, 1920, 1080);
    let old = Rect::new(0, 0, 1920, 1050);
    let area = Rect::new(0, 0, 1920, 1027);
    let was = Rect::new(312, 567, 375, 460);
    let grown = Rect::new(312, 567, 735, 920);
    assert_eq!(
        retheme_back_inside(was, grown, screen, area, area),
        Some((312, 107)),
        "the grown sheet ends on the taskbar's top"
    );
    let low = Rect::new(156, 557, 688, 480);
    assert_eq!(
        retheme_back_inside(low, low, screen, old, area),
        Some((156, 547)),
        "a window the taller taskbar now covers moves up"
    );
    assert_eq!(
        retheme_back_inside(low, low, screen, area, area),
        None,
        "one left over an unchanged taskbar stays"
    );
    assert_eq!(retheme_back_inside(was, was, screen, area, area), None);
    let off = Rect::new(1500, 567, 620, 460);
    assert_eq!(
        retheme_back_inside(off, Rect { height: 920, ..off }, screen, area, area),
        None,
        "one left partly off screen stays"
    );
    let tall = Rect::new(0, 100, 600, 1200);
    assert_eq!(
        retheme_back_inside(Rect::new(0, 100, 600, 500), tall, screen, area, area),
        Some((0, 0)),
        "one taller than the work area keeps its caption on screen"
    );
}

#[test]
fn a_dialog_centres_on_its_owner() {
    let parent = Rect::new(100, 100, 400, 300);
    let area = Rect::new(0, 0, 1920, 1030);
    assert_eq!(centre_on(parent, (200, 100), area), (200, 200));
}

#[test]
fn an_ownerless_dialog_centres_on_the_usable_area() {
    let area = Rect::new(0, 50, 1920, 1000);
    assert_eq!(centre_on(area, (378, 201), area), (771, 449));
}

#[test]
fn a_dialog_is_pushed_back_inside_the_usable_area() {
    let area = Rect::new(0, 0, 1920, 1030);
    let parent = Rect::new(1800, 980, 120, 50);
    let (x, y) = centre_on(parent, (400, 200), area);
    assert!(x + 400 <= area.right(), "x {x} runs off the right");
    assert!(y + 200 <= area.bottom(), "y {y} runs off the bottom");
    let parent = Rect::new(-50, -40, 100, 80);
    let (x, y) = centre_on(parent, (400, 200), area);
    assert_eq!((x, y), (area.x, area.y));
}

#[test]
fn a_dialog_larger_than_the_screen_starts_at_the_corner() {
    let area = Rect::new(0, 0, 300, 200);
    let parent = Rect::new(0, 0, 300, 200);
    assert_eq!(centre_on(parent, (800, 600), area), (area.x, area.y));
}

#[test]
fn an_anchor_waits_for_the_last_size_asked_for() {
    let (s5, s7) = (Some(Serial::from(5)), Some(Serial::from(7)));
    assert!(
        !anchor_done(s7, s5, true),
        "an older size landing keeps the anchor for the newer one"
    );
    assert!(anchor_done(s7, s7, true), "the last size landed");
    assert!(anchor_done(s7, Some(Serial::from(8)), false));
    assert!(!anchor_done(s7, None, true), "nothing acked yet");
    assert!(anchor_done(None, None, true), "an X11 window's new size");
    assert!(!anchor_done(None, None, false));
}

#[test]
fn anchored_position_pins_the_opposite_edge() {
    let right_bottom = (300, 200);
    let a = ResizeAnchor {
        edges: Edges::LEFT,
        right: right_bottom.0,
        bottom: right_bottom.1,
    };
    assert_eq!(anchored_position(&a, (120, 80), (50, 60)), (180, 60));
    let a = ResizeAnchor {
        edges: Edges::TOP,
        right: right_bottom.0,
        bottom: right_bottom.1,
    };
    assert_eq!(anchored_position(&a, (120, 80), (50, 60)), (50, 120));
    let a = ResizeAnchor {
        edges: Edges::LEFT | Edges::TOP,
        right: right_bottom.0,
        bottom: right_bottom.1,
    };
    assert_eq!(anchored_position(&a, (120, 80), (50, 60)), (180, 120));
    let a = ResizeAnchor {
        edges: Edges::RIGHT,
        right: right_bottom.0,
        bottom: right_bottom.1,
    };
    assert_eq!(anchored_position(&a, (120, 80), (50, 60)), (50, 60));
}

/// The layout rect of a 100x60 menu, and the usable area, once a toplevel asks for it to drop
/// from its client's bottom-left corner (or to open leftwards from its top-left one when
/// `leftwards`) with the taskbar on `edge` and the client 40 (80) px from the usable area's
/// edge: far enough that the menu would fit on the output over the bar.
fn menu_by_the_bar(
    edge: reprisekit::taskbar::Edge,
    leftwards: bool,
) -> (crate::raster::Rect, crate::raster::Rect) {
    use crate::raster::Rect;
    use smithay::desktop::{PopupKind, PopupManager};
    use wayland_protocols::xdg::shell::client::xdg_positioner::{
        Anchor, ConstraintAdjustment, Gravity,
    };
    let mut config = crate::config::Config::default();
    config.taskbar.position = edge;
    let mut h = crate::harness::Harness::with_config(config);
    let out = h.output("TEST-1", 1024, 768);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "test.app", 300, 200);
    let area = h.state.usable_area(Some(&out));
    let win = h.state.win(t.id).unwrap();
    let (cx, cy) = win.client_origin();
    let (fx, fy) = (win.x, win.y);
    let (x, y) = if leftwards {
        (area.x + 80 + fx - cx, fy)
    } else {
        (fx, area.y + area.height - 240 + fy - cy)
    };
    h.state.move_to(t.id, x, y);
    h.dispatch();
    h.map_popup_with(&mut c, t, 100, 60, |p| {
        let adjust = ConstraintAdjustment::FlipX
            | ConstraintAdjustment::FlipY
            | ConstraintAdjustment::SlideX
            | ConstraintAdjustment::SlideY;
        if leftwards {
            p.set_anchor_rect(0, 0, 1, 20);
            p.set_anchor(Anchor::TopLeft);
            p.set_gravity(Gravity::BottomLeft);
        } else {
            p.set_anchor_rect(0, 180, 40, 20);
            p.set_anchor(Anchor::BottomLeft);
            p.set_gravity(Gravity::BottomRight);
        }
        p.set_constraint_adjustment(adjust);
    });
    let win = h.state.win(t.id).unwrap();
    let (cx, cy) = win.client_origin();
    let open: Vec<_> = PopupManager::popups_for_surface(win.wl_surface()).collect();
    assert_eq!(open.len(), 1, "the menu is open");
    let PopupKind::Xdg(popup) = &open[0].0 else {
        panic!("an xdg popup");
    };
    let g = popup.with_pending_state(|s| s.geometry);
    let menu = Rect::new(cx + g.loc.x, cy + g.loc.y, g.size.w, g.size.h);
    (menu, area)
}

#[test]
fn a_menu_dropping_onto_the_bottom_taskbar_opens_above_its_anchor() {
    let (menu, area) = menu_by_the_bar(reprisekit::taskbar::Edge::Bottom, false);
    assert!(area.height < 768, "the taskbar takes a strip: {area:?}");
    assert_eq!(
        menu.y + menu.height,
        area.y + area.height - 60,
        "the menu flips up to end at its anchor's top: {menu:?} in {area:?}"
    );
}

#[test]
fn a_menu_opening_onto_a_left_taskbar_stays_right_of_it() {
    let (menu, area) = menu_by_the_bar(reprisekit::taskbar::Edge::Left, true);
    assert!(area.x >= 20, "the taskbar takes a strip: {area:?}");
    assert!(
        menu.x >= area.x,
        "the menu starts right of the bar at x >= {}: {menu:?}",
        area.x
    );
}

/// Two mapped toplevels on one output with the desktop holding the keyboard; the second's
/// window marked override-redirect when `menu`.
fn behind_the_desktop(
    menu: bool,
) -> (
    crate::harness::Harness,
    crate::harness::TestClient,
    crate::harness::Toplevel,
    crate::harness::Toplevel,
) {
    let mut h = crate::harness::Harness::new();
    h.output("TEST-1", 1024, 768);
    let mut c = h.connect();
    let a = h.map_toplevel(&mut c, "test.a", 300, 200);
    let b = h.map_toplevel(&mut c, "test.b", 100, 80);
    h.state.focus_desktop();
    assert_eq!(h.state.focused, None);
    if let Some(w) = h.state.win_mut(b.id) {
        w.override_redirect = menu;
    }
    (h, c, a, b)
}

#[test]
fn a_closing_override_redirect_window_leaves_the_focus_where_it_is() {
    let (mut h, mut c, a, b) = behind_the_desktop(true);
    c.destroy(b);
    h.roundtrip(&mut c);
    assert!(h.state.win(b.id).is_none(), "the window went");
    assert_eq!(h.state.focused, None, "the desktop keeps the keyboard");
    assert!(h.state.win(a.id).is_some());
}

#[test]
fn a_closing_window_hands_the_focus_to_the_top_one() {
    let (mut h, mut c, a, b) = behind_the_desktop(false);
    c.destroy(b);
    h.roundtrip(&mut c);
    assert!(h.state.win(b.id).is_none());
    assert_eq!(h.state.focused, Some(a.id));
}

#[test]
fn a_window_moved_to_an_output_with_another_work_area_hears_its_bounds() {
    let mut h = crate::harness::Harness::new();
    h.output("TEST-1", 1024, 768);
    h.output("TEST-2", 1920, 1080);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "test.app", 300, 200);
    h.roundtrip(&mut c);
    let first = c.side.shells[t.shell].bounds;
    assert!(
        first.0 > 0 && first.0 < 1024,
        "bounds on the first output: {first:?}"
    );
    let two = h.state.scene.outputs()[1].clone();
    let b = h.state.layout_box(Some(&two));
    h.state.move_to(t.id, b.x + 100, b.y + 100);
    h.roundtrip(&mut c);
    let moved = c.side.shells[t.shell].bounds;
    assert!(
        moved.0 > 1024 && moved.1 > first.1,
        "bounds on the larger output: {moved:?}"
    );
    h.state.move_to(t.id, 100, 100);
    h.roundtrip(&mut c);
    assert_eq!(
        c.side.shells[t.shell].bounds, first,
        "and back on the first"
    );
}

#[test]
fn a_window_hears_new_bounds_when_the_taskbar_moves_or_hides() {
    let mut h = crate::harness::Harness::new();
    h.output("TEST-1", 1024, 768);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "test.app", 300, 200);
    h.roundtrip(&mut c);
    let bottom = c.side.shells[t.shell].bounds;
    let mut cfg = h.state.config.clone();
    cfg.taskbar.position = reprisekit::taskbar::Edge::Left;
    crate::configwatch::reload(&mut h.state, cfg);
    h.roundtrip(&mut c);
    let left = c.side.shells[t.shell].bounds;
    assert!(
        left.0 < bottom.0 && left.1 > bottom.1,
        "a side bar takes width and gives height back: {bottom:?} -> {left:?}"
    );
    let mut cfg = h.state.config.clone();
    cfg.taskbar.auto_hide = true;
    crate::configwatch::reload(&mut h.state, cfg);
    h.roundtrip(&mut c);
    let hidden = c.side.shells[t.shell].bounds;
    assert!(
        hidden.0 > left.0 && hidden.1 == left.1,
        "a hiding bar gives its width back: {left:?} -> {hidden:?}"
    );
}

#[test]
fn a_window_hears_new_bounds_when_its_output_changes_mode_or_scale() {
    use smithay::output::{Mode, Scale};
    let mut h = crate::harness::Harness::new();
    let o = h.output("TEST-1", 1024, 768);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "test.app", 300, 200);
    h.roundtrip(&mut c);
    let first = c.side.shells[t.shell].bounds;
    let mode = Mode {
        size: (1920, 1080).into(),
        refresh: 60_000,
    };
    o.change_current_state(Some(mode), None, None, None);
    h.state.update_layout();
    h.roundtrip(&mut c);
    let wide = c.side.shells[t.shell].bounds;
    assert!(
        wide.0 > first.0 && wide.1 > first.1,
        "a larger mode gives larger bounds: {first:?} -> {wide:?}"
    );
    o.change_current_state(None, None, Some(Scale::Fractional(2.0)), None);
    h.state.update_layout();
    h.roundtrip(&mut c);
    let scaled = c.side.shells[t.shell].bounds;
    assert!(
        scaled.0 < first.0 && scaled.1 < first.1,
        "scale 2 halves the work area: {wide:?} -> {scaled:?}"
    );
}

#[test]
fn a_window_that_sized_itself_keeps_that_size_when_restored() {
    let mut h = crate::harness::Harness::new();
    h.output("TEST-1", 1024, 768);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "test.app", 300, 200);
    h.state.request_size(t.id, 400, 300);
    h.roundtrip(&mut c);
    assert_eq!(c.side.shells[t.shell].size, (400, 300), "the asked size");
    h.resize_self(&mut c, t, 350, 250);
    h.state.set_minimized(t.id, true);
    h.roundtrip(&mut c);
    h.state.set_minimized(t.id, false);
    h.roundtrip(&mut c);
    assert_eq!(
        c.side.shells[t.shell].size,
        (350, 250),
        "minimize and restore keep the size the program took"
    );
}

#[test]
fn a_client_name_is_read_up_to_1024_chars() {
    use super::{NAME_CAP, capped};
    assert_eq!(NAME_CAP, 1024);
    let long: String = "中".repeat(3000);
    assert_eq!(capped(long).chars().count(), 1024);
    assert_eq!(capped("Notepad".into()), "Notepad");
    assert_eq!(capped("a".repeat(1024)).len(), 1024);
}
