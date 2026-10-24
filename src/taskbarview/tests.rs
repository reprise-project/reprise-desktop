use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn a_full_glow_cache_drops_the_least_recently_used_icon() {
    static WORKED_OUT: AtomicUsize = AtomicUsize::new(0);
    fn colour(_: &Canvas) -> Option<Color> {
        WORKED_OUT.fetch_add(1, Ordering::Relaxed);
        None
    }
    let glow = TaskGlow {
        colour,
        label_edges: None,
    };
    let icon = |i: usize| {
        let mut c = Canvas::new(1, 1);
        c.pixels[0] = i as u32;
        c
    };
    let ask = |i: usize| {
        let before = WORKED_OUT.load(Ordering::Relaxed);
        glow_colour(glow, &icon(i));
        WORKED_OUT.load(Ordering::Relaxed) > before
    };
    for i in 0..GLOW_KEEP {
        assert!(ask(i), "icon {i} is new");
    }
    assert!(!ask(0), "the first icon is kept");
    assert!(ask(GLOW_KEEP), "one past the cap");
    assert!(!ask(0), "the first icon, asked for since, is still kept");
    assert!(ask(1), "the second, the least recently used, went");
}

#[test]
fn a_top_bars_held_orb_stands_over_the_menu_below_the_bar() {
    use reprisekit::taskbar::Edge;
    let style = reprisekit::glass::small_bar_style(&Default::default());
    let theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
    let m = Metrics {
        taskbar_height: reprisekit::glass::SMALL_TASKBAR_HEIGHT,
        ..theme.metrics
    };
    let l = TaskbarLayout::compute(&m, &style.taskbar, 800, true, TaskbarCounts::default(), 40)
        .on_edge(Edge::Top);
    assert_eq!(
        l.start_orb(false),
        Some(Rect::new(0, 30, 54, 13)),
        "at rest the orb's buffer is the 13 rows below the bar"
    );
    assert!(
        !stands_over_menu(&l, false, false),
        "at rest it stays with the bar"
    );
    assert!(
        stands_over_menu(&l, true, false),
        "held it stands over the menu"
    );
    assert!(
        !stands_over_menu(&l, true, true),
        "under a fullscreen window it stays with the bar"
    );
}

#[test]
fn a_bar_moved_under_a_still_pointer_lights_only_the_cell_under_it() {
    let mut h = crate::harness::Harness::new();
    h.output("TEST-1", 1024, 768);
    h.pointer_to(4.0, 764.0);
    assert!(h.state.bars[0].start_hot, "Start under the pointer");
    let mut top = h.state.config.clone();
    top.taskbar.position = reprisekit::taskbar::Edge::Top;
    crate::configwatch::reload(&mut h.state, top);
    h.dispatch();
    assert!(
        !h.state.bars[0].start_hot,
        "Start moved out from under the pointer"
    );
    h.pointer_to(4.0, 4.0);
    let mut bottom = h.state.config.clone();
    bottom.taskbar.position = reprisekit::taskbar::Edge::Bottom;
    crate::configwatch::reload(&mut h.state, bottom);
    h.dispatch();
    assert!(!h.state.bars[0].start_hot, "Start moved away again");
    let mut top = h.state.config.clone();
    top.taskbar.position = reprisekit::taskbar::Edge::Top;
    crate::configwatch::reload(&mut h.state, top);
    h.dispatch();
    assert!(
        h.state.bars[0].start_hot,
        "Start moved in under the still pointer"
    );
}
