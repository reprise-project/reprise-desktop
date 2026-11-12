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

#[test]
fn redrawing_the_taskbars_asks_only_the_outputs_whose_bar_changed() {
    let mut h = crate::harness::Harness::new();
    h.output("TEST-1", 1024, 768);
    h.output("TEST-2", 1024, 768);
    h.state.redraw_taskbars();
    crate::backend::take_asks();
    h.state.redraw_taskbars();
    assert_eq!(
        crate::backend::take_asks(),
        vec![],
        "no bar changed, so no output is asked"
    );
    let first = (h.state.bars.iter_mut())
        .find(|b| b.output.name() == "TEST-1")
        .expect("TEST-1's bar");
    first.start_pressed = true;
    h.state.redraw_taskbars();
    let asked = crate::backend::take_asks();
    assert!(!asked.is_empty(), "the pressed Start asks for a frame");
    assert!(
        asked.iter().all(|(o, _)| o == "TEST-1"),
        "only TEST-1, the pressed bar's output: {asked:?}"
    );
}

mod under_a_lock {
    use crate::backend::{take_asks, take_scene_outputs};
    use crate::harness::{Harness, TestClient, Toplevel};

    /// TEST-1 with a window titled "one" on its bar.
    fn desktop() -> (Harness, TestClient, Toplevel) {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.lock.helpers = crate::lockview::Helpers {
            password: |_, _| std::process::Command::new("false"),
            finger: |_| None,
            ..Default::default()
        };
        let mut c = h.connect();
        let t = h.map_titled(&mut c, "editor", "one", (300, 200));
        h.dispatch();
        (h, c, t)
    }

    /// `t`'s client retitles it to `title`, once a title redraw held back has drawn and the
    /// wait after it is over.
    fn retitle(h: &mut Harness, c: &mut TestClient, t: Toplevel, title: &str) {
        h.run_for(2 * crate::retitle::MIN_INTERVAL.as_millis() as u64 + 20);
        take_asks();
        c.side.shells[t.shell].toplevel.set_title(title.into());
        c.side.shells[t.shell].surface.commit();
        h.roundtrip(c);
        h.dispatch();
    }

    fn drawn(h: &Harness) -> &crate::taskbarview::DrawnBar {
        h.state.bars[0].drawn.as_ref().expect("the bar is drawn")
    }

    fn titles(h: &Harness) -> Vec<String> {
        (drawn(h).state.tasks.iter())
            .map(|t| t.title.clone())
            .collect()
    }

    fn lock(h: &mut Harness) {
        crate::lockview::lock(&mut h.state);
        h.dispatch();
        take_asks();
    }

    #[test]
    fn a_retitle_under_the_lock_redraws_the_bar_and_asks_no_frame() {
        let (mut h, mut c, t) = desktop();
        retitle(&mut h, &mut c, t, "two");
        assert_eq!(take_scene_outputs(), ["TEST-1"], "unlocked, the bar shows");
        lock(&mut h);
        retitle(&mut h, &mut c, t, "three");
        assert_eq!(titles(&h), ["three"], "the bar redrew");
        assert_eq!(take_asks(), [], "the cover hides the bar and the caption");
    }

    #[test]
    fn a_clock_tick_under_the_lock_asks_no_frame() {
        let (mut h, _c, _t) = desktop();
        let tick = |h: &mut Harness| {
            crate::taskbar::TEST_CLOCK.set(crate::taskbar::TEST_CLOCK.get() + 60);
            let was = drawn(h).state.clock.clone();
            h.state.redraw_taskbars();
            assert_ne!(drawn(h).state.clock, was, "the clock turned");
            take_scene_outputs()
        };
        assert_eq!(tick(&mut h), ["TEST-1"], "unlocked, the clock shows");
        lock(&mut h);
        assert_eq!(tick(&mut h), Vec::<String>::new());
    }

    #[test]
    fn typing_on_the_lock_screen_asks_for_a_frame() {
        let (mut h, _c, _t) = desktop();
        lock(&mut h);
        crate::lockview::type_text(&mut h.state, "a");
        assert_eq!(take_scene_outputs(), ["TEST-1"]);
    }

    #[test]
    fn an_unlock_asks_for_a_frame_and_shows_the_bar_redrawn_under_the_lock() {
        let (mut fresh, mut fc, ft) = desktop();
        retitle(&mut fresh, &mut fc, ft, "three");
        let (mut h, mut c, t) = desktop();
        lock(&mut h);
        retitle(&mut h, &mut c, t, "three");
        crate::lockview::unlock(&mut h.state);
        assert_eq!(take_scene_outputs(), ["TEST-1"]);
        let bar = h.state.bars[0].tree;
        let o = h.state.scene.outputs()[0].clone();
        assert_eq!(
            h.state.outputs_of_nodes(&[bar.into()]),
            [o],
            "nothing covers the bar"
        );
        assert!(
            drawn(&h).canvas.pixels == drawn(&fresh).canvas.pixels,
            "the bar shows the title it took under the lock"
        );
    }
}

mod per_bar {
    use crate::backend::{take_asks, take_scene_outputs};
    use crate::harness::{Harness, TestClient, Toplevel};

    /// TEST-1 and TEST-2 side by side, with a window titled "one" on TEST-1.
    fn two_outputs(config: &str) -> (Harness, TestClient, Toplevel) {
        let config = format!("{config}\n[effects]\nwindow_animation = false\n");
        let mut h = Harness::with_config(crate::config::Config::parse(&config).unwrap());
        h.output("TEST-1", 1024, 768);
        h.output("TEST-2", 1024, 768);
        let mut c = h.connect();
        let t = h.map_titled(&mut c, "editor", "one", (300, 200));
        h.dispatch();
        (h, c, t)
    }

    fn retitle(h: &mut Harness, c: &mut TestClient, t: Toplevel, title: &str) {
        h.run_for(2 * crate::retitle::MIN_INTERVAL.as_millis() as u64 + 20);
        take_asks();
        c.side.shells[t.shell].toplevel.set_title(title.into());
        c.side.shells[t.shell].surface.commit();
        h.roundtrip(c);
        h.dispatch();
    }

    fn titles(h: &Harness, output: &str) -> Vec<String> {
        let bar = (h.state.bars.iter())
            .find(|b| b.output.name() == output)
            .expect("the output's bar");
        let drawn = bar.drawn.as_ref().expect("the bar is drawn");
        (drawn.state.tasks.iter())
            .map(|t| t.title.clone())
            .collect()
    }

    #[test]
    fn a_button_change_asks_no_frame_of_an_auto_hidden_bar() {
        let (mut h, mut c, t) = two_outputs("[taskbar]\nauto_hide = true");
        h.pointer_to(700.0, 767.0);
        h.run_for(6000);
        // The bar's rows on its output, against its thickness.
        let rows = |h: &Harness, o: &str| {
            let out = (h.state.scene.outputs().iter())
                .find(|x| x.name() == o)
                .expect("the output");
            let p = h.state.bar_drawn(out);
            (768 - p.screen_side(), p.thickness)
        };
        let (on, thick) = rows(&h, "TEST-1");
        assert_eq!(on, thick, "the pointer at its edge keeps TEST-1's bar out");
        let (on, thick) = rows(&h, "TEST-2");
        assert!(on < thick, "TEST-2's bar hid: {on} of {thick} rows show");
        retitle(&mut h, &mut c, t, "two");
        assert_eq!(titles(&h, "TEST-2"), ["two"], "the hidden bar redrew");
        assert_eq!(take_scene_outputs(), ["TEST-1"]);
        h.state.set_minimized(t.id, true);
        assert_eq!(take_scene_outputs(), ["TEST-1"]);
    }

    #[test]
    fn retitling_a_window_on_another_desktop_asks_no_frame() {
        let (mut h, mut c, t) = two_outputs("[workspaces]\ncount = 2");
        h.state.move_to_desktop(&[t.id], 1);
        h.dispatch();
        retitle(&mut h, &mut c, t, "two");
        assert_eq!(take_asks(), []);
    }
}
