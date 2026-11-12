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
        h.until("the titles drawn", |s| s.title_draws_at_once(t.id));
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
        h.until("the titles drawn", |s| s.title_draws_at_once(t.id));
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
    fn each_bar_draws_once_per_window_or_desktop_change() {
        let (mut h, mut c, one) = two_outputs("[workspaces]\ncount = 2");
        let two = h.map_titled(&mut c, "other", "two", (300, 200));
        h.dispatch();
        let draws = |h: &Harness| h.state.bars.iter().map(|b| b.draws).collect::<Vec<_>>();
        let once = |h: &mut Harness, what: &str, f: &mut dyn FnMut(&mut Harness)| {
            let was = draws(h);
            f(h);
            h.dispatch();
            let now = draws(h);
            let per: Vec<usize> = now.iter().zip(&was).map(|(n, w)| n - w).collect();
            assert_eq!(per, [1, 1], "{what}: draws per bar");
        };
        assert_eq!(h.state.focused, Some(two.id));
        once(&mut h, "minimize", &mut |h| {
            h.state.set_minimized(two.id, true)
        });
        assert_eq!(h.state.focused, Some(one.id), "the focus moved on");
        once(&mut h, "restore", &mut |h| {
            h.state.set_minimized(two.id, false)
        });
        once(&mut h, "move to a desktop", &mut |h| {
            h.state.move_to_desktop(&[two.id], 1)
        });
        assert_eq!(h.state.focused, Some(one.id), "the focus moved on");
        once(&mut h, "switch desktop", &mut |h| h.state.switch_desktop(1));
        assert_eq!(
            h.state.focused,
            Some(two.id),
            "the desktop's window took focus"
        );
        once(&mut h, "switch back", &mut |h| h.state.switch_desktop(0));
        let mut three = None;
        once(&mut h, "map", &mut |h| {
            three = Some(h.map_titled(&mut c, "third", "three", (300, 200)));
        });
        let three = three.expect("mapped");
        assert_eq!(h.state.focused, Some(three.id));
        once(&mut h, "close", &mut |h| {
            c.destroy(three);
            h.roundtrip(&mut c);
        });
        assert_eq!(h.state.focused, Some(one.id), "the focus moved on");
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

#[test]
fn a_redraw_draws_again_only_the_cells_that_changed() {
    for interface in ["classic", "blue", "glass"] {
        let toml = format!("[appearance]\ninterface = \"{interface}\"\n");
        let mut h =
            crate::harness::Harness::with_config(crate::config::Config::parse(&toml).unwrap());
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let one = h.map_titled(&mut c, "editor", "one", (300, 200));
        h.map_titled(&mut c, "other", "two", (300, 200));
        h.dispatch();
        let redraw = |h: &mut crate::harness::Harness| {
            h.state.bars[0].drawn_parts.clear();
            h.state.redraw_taskbars();
            let b = &h.state.bars[0];
            let d = b.drawn.as_ref().expect("drawn");
            let mut want = Canvas::new(d.canvas.width, d.canvas.height);
            let theme = h.state.config.theme.scaled(d.scale);
            reprisekit::look::draw_taskbar(
                &mut want,
                &theme,
                h.state.text.for_output(d.scale),
                h.state.start_text.for_output(d.scale),
                &d.layout,
                &d.state,
            );
            assert!(
                d.canvas.pixels == want.pixels,
                "{interface}: the bar as a full draw gives it"
            );
            (b.drawn_parts.clone(), d.layout.clone())
        };
        let (parts, _) = redraw(&mut h);
        assert_eq!(parts, [None], "{interface}: nothing changed");
        crate::taskbar::TEST_CLOCK.set(crate::taskbar::TEST_CLOCK.get() + 60);
        let (parts, l) = redraw(&mut h);
        let clock = |r: &Rect| r.x <= l.clock.x && r.right() >= l.clock.right();
        match parts[..] {
            [Some(r)] => assert!(clock(&r) && r.x > l.tasks[1].right(), "{interface}: {r:?}"),
            _ => panic!("{interface}: the clock turned: {parts:?}"),
        }
        h.state.notify.log.dnd = true;
        redraw(&mut h);
        h.state.notify.log.unseen = 2;
        let (parts, l) = redraw(&mut h);
        match parts[..] {
            [Some(r)] => assert!(
                r.x <= l.tray[0].x && r.x > l.tasks[1].right(),
                "{interface}: {r:?}"
            ),
            _ => panic!("{interface}: the bell's count changed: {parts:?}"),
        }
        let (parts, _) = redraw(&mut h);
        assert_eq!(parts, [None], "{interface}: the bell drawn the same");
        h.state.bars[0].task_hot = Some(TaskKey::Window(one.id));
        let (parts, l) = redraw(&mut h);
        match parts[..] {
            [Some(r)] => assert!(
                r.x <= l.tasks[0].x && r.right() < l.clock.x,
                "{interface}: {r:?}"
            ),
            _ => panic!("{interface}: the first button lit: {parts:?}"),
        }
    }
}

/// The time of one bar redraw per look and scale: with nothing changed, the pointer moving between
/// task buttons, the clock turning, a tray cell redrawn, and a bar drawn from scratch (as a button
/// added or closed draws it). Run with `--ignored --nocapture` under `--profile vm`;
/// `BAR_COST=glass:2` keeps one look and scale, `N` times as many rounds.
#[test]
#[ignore]
fn bar_redraw_cost() {
    use crate::harness::Harness;
    use smithay::output::Scale;
    let only = std::env::var("BAR_COST").ok();
    let n: u32 = if only.is_some() { 1000 } else { 100 };
    for look in ["classic", "blue", "glass"] {
        for (w, hh, scale) in [(1920, 1080, 1.0), (3840, 2160, 2.0)] {
            if only
                .as_ref()
                .is_some_and(|o| *o != format!("{look}:{scale}"))
            {
                continue;
            }
            let text = format!("[appearance]\ninterface = \"{look}\"\n");
            let mut h = Harness::with_config(crate::config::Config::parse(&text).unwrap());
            for name in ["TEST-1", "TEST-2"] {
                let o = h.output(name, w, hh);
                o.change_current_state(None, None, Some(Scale::Fractional(scale)), None);
            }
            h.state.update_layout();
            let mut c = h.connect();
            let one = h.map_titled(&mut c, "editor", "one", (300, 200));
            let two = h.map_titled(&mut c, "other", "two", (300, 200));
            h.state.notify.log.dnd = true;
            h.run_for(300);
            let bars = h.state.bars.len() as u32;
            let time = |h: &mut Harness, f: &mut dyn FnMut(&mut Harness, u32)| {
                let t = std::time::Instant::now();
                for k in 0..n {
                    f(h, k);
                    h.state.redraw_taskbars();
                }
                t.elapsed() / (n * bars)
            };
            let same = time(&mut h, &mut |_, _| {});
            let hover = time(&mut h, &mut |h, k| {
                let id = if k % 2 == 0 { one.id } else { two.id };
                for b in &mut h.state.bars {
                    b.task_hot = Some(TaskKey::Window(id));
                    b.task_hot_x = Some(20 + (k % 7) as i32);
                }
            });
            for b in &mut h.state.bars {
                (b.task_hot, b.task_hot_x) = (None, None);
            }
            let clock = time(&mut h, &mut |_, _| {
                crate::taskbar::TEST_CLOCK.set(crate::taskbar::TEST_CLOCK.get() + 60);
            });
            let tray = time(&mut h, &mut |h, k| {
                h.state.notify.log.unseen = 1 + (k % 2) as usize;
            });
            let full = time(&mut h, &mut |h, _| {
                for b in &mut h.state.bars {
                    b.drawn = None;
                }
            });
            eprintln!(
                "{look} x{scale}: unchanged {same:?}, hover {hover:?}, clock {clock:?}, \
                 tray {tray:?}, full {full:?}"
            );
        }
    }
}
