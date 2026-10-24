use super::*;

#[test]
fn a_window_without_an_icon_shows_a_light_glyph_on_the_dark_glass_bar() {
    let settings = crate::glass::Settings {
        mode: crate::glass::Mode::Dark,
        ..crate::glass::Settings::default()
    };
    let theme =
        crate::interfaces::theme_with(Interface::Glass, crate::blue::Variant::default(), &settings);
    let glyph = icon::chrome_fallback(&theme, 16);
    assert_eq!(glyph.get(8, 10) & 0xFF_FFFF, 0xFF_FFFF, "the white body");
    assert_eq!(glyph.get(1, 8) & 0xFF_FFFF, 0xEF_EEEF, "the EFEEEF frame");
    let s = theme.look.style().unwrap();
    let counts = TaskbarCounts {
        tasks: 1,
        ..TaskbarCounts::default()
    };
    let layout = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40);
    let h = layout.height as u32;
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let draw = |st: &TaskbarState| {
        let mut c = Canvas::new(800, h);
        draw_taskbar(&mut c, s, &theme, &text, &start, &layout, st);
        c
    };
    let bare = state_with_tasks(&["One"]);
    let mut given = state_with_tasks(&["One"]);
    given.tasks[0].icon = Some(icon::chrome_fallback(&theme, s.taskbar.task_icon).into());
    assert!(
        draw(&bare).pixels == draw(&given).pixels,
        "the bar draws the light glyph"
    );
}

fn glass_stacks(s: &Style, height: i32) -> (Canvas, TaskbarLayout) {
    let theme = theme_for(Interface::Glass);
    let metrics = crate::theme::Metrics {
        taskbar_height: height,
        ..theme.metrics.clone()
    };
    let counts = TaskbarCounts {
        tasks: 3,
        ..TaskbarCounts::default()
    };
    let mut layout = TaskbarLayout::compute(&metrics, &s.taskbar, 1920, true, counts, 60);
    layout.lay_tasks(
        &metrics,
        &s.taskbar,
        &[1, 2, 3],
        crate::taskbar::TaskRow::Icons,
    );
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let mut st = state_with_tasks(&["", "", ""]);
    for t in &mut st.tasks {
        t.icon = Some(Canvas::new(16, 16).into());
    }
    let mut c = Canvas::new(1920, height as u32);
    draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);
    (c, layout)
}

#[test]
fn a_glass_button_for_several_windows_ends_in_stacked_edges() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let (c, l) = glass_stacks(s, 40);
    for (i, edges) in [0, 1, 2].into_iter().enumerate() {
        let r = l.tasks[i];
        assert_eq!(r.width, 60, "slot {i} keeps its 60 px");
        let mut want = Canvas::new(1920, 40);
        s.draw(
            &mut want,
            Rect::new(0, 0, 1920, 40),
            Part::Taskbar,
            State::Normal,
            1,
        );
        let front = Rect::new(r.x, r.y, 60 - 4 * edges, r.height);
        s.draw(&mut want, front, Part::TaskButton, State::Normal, 1);
        for k in 0..edges {
            let e = Rect::new(front.right() + 4 * k, r.y, 4, r.height);
            s.draw_nth(&mut want, e, Part::TaskStack, State::Normal, k as usize, 1);
        }
        for y in 0..40 {
            for x in r.x..r.right() {
                assert_eq!(c.get(x, y), want.get(x, y), "{} windows: ({x}, {y})", i + 1);
            }
        }
    }
    let at = |i: usize, dx: i32| over_grey(c.get(l.tasks[i].right() - dx, 20));
    assert_eq!(
        at(0, 2),
        [180, 187, 197],
        "one window: the button's light line"
    );
    assert_eq!(at(0, 1), [42, 57, 77], "one window: the button's dark line");
    assert_eq!(
        at(1, 5),
        [42, 57, 77],
        "two windows: the button's dark line, 4 px in"
    );
    assert_eq!(
        at(1, 2),
        [143, 154, 168],
        "two windows: the edge's light line"
    );
    assert_eq!(at(1, 1), [67, 82, 102], "two windows: the edge's dark line");
    assert_eq!(
        at(2, 6),
        [143, 154, 168],
        "three windows: the nearer light line"
    );
    assert_eq!(
        at(2, 2),
        [118, 131, 149],
        "three windows: the farther light line"
    );
    assert_eq!(
        at(2, 1),
        [84, 99, 119],
        "three windows: the farther dark line"
    );
}

fn glass_stacks_in(
    active: [bool; 3],
    hot: Option<usize>,
    held: Option<usize>,
) -> (Canvas, TaskbarLayout) {
    glass_stacks_glowing(active, hot, held, None, 1)
}

fn glass_stacks_glowing(
    active: [bool; 3],
    hot: Option<usize>,
    held: Option<usize>,
    glow: Option<crate::theme::Color>,
    scale: i32,
) -> (Canvas, TaskbarLayout) {
    glass_row(&[1, 2, 3], active, hot, held, glow, scale)
}

fn glass_row(
    stacks: &[u8],
    active: [bool; 3],
    hot: Option<usize>,
    held: Option<usize>,
    glow: Option<crate::theme::Color>,
    scale: i32,
) -> (Canvas, TaskbarLayout) {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    glass_row_in(s, stacks, active, (hot, held, None), glow, scale)
}

fn glass_row_in(
    s: &Style,
    stacks: &[u8],
    active: [bool; 3],
    lit: (Option<usize>, Option<usize>, Option<i32>),
    glow: Option<crate::theme::Color>,
    scale: i32,
) -> (Canvas, TaskbarLayout) {
    let (hot, held, x) = lit;
    let theme = theme_for(Interface::Glass).scaled(scale);
    let counts = TaskbarCounts {
        tasks: 3,
        ..TaskbarCounts::default()
    };
    let base = theme_for(Interface::Glass);
    let mut layout = TaskbarLayout::compute(&base.metrics, &s.taskbar, 1920, true, counts, 60);
    layout.lay_tasks(
        &base.metrics,
        &s.taskbar,
        stacks,
        crate::taskbar::TaskRow::Icons,
    );
    let layout = layout.scaled(scale);
    let text = TextRenderer::load(&theme.fonts);
    let start = TextRenderer::load(&s.start_text.font());
    let mut st = state_with_tasks(&["", "", ""]);
    let px = (s.taskbar.task_icon * scale) as u32;
    let mut white = Canvas::new(px, px);
    white.fill_rect(white.bounds(), crate::theme::Color::rgb(0xFF, 0xFF, 0xFF));
    for (t, a) in st.tasks.iter_mut().zip(active) {
        t.icon = Some(white.clone().into());
        t.active = a;
        t.glow = glow;
    }
    st.task_hot = hot;
    st.task_pressed = held;
    st.task_hot_x = x;
    let mut c = Canvas::new(layout.width as u32, layout.height as u32);
    draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);
    (c, layout)
}

fn glass_unlit_spot() -> Style {
    glass_unlit_spot_at(1)
}

fn glass_unlit_spot_at(sc: i32) -> Style {
    let mut s = crate::glass::build(&Default::default());
    if sc > 1 {
        let at = crate::glass::style(&Default::default()).at_scale(sc);
        for part in [
            Part::Taskbar,
            Part::TrayArea,
            Part::LabelRow,
            Part::TaskButton,
            Part::TaskStack,
            Part::TaskLabel,
            Part::TaskLabelEdge,
            Part::SlotEdge,
            Part::SlotLight,
            Part::SlotBand,
            Part::PagerTile,
            Part::ShowDesktop,
        ] {
            s = s.with_part(part, at.part(part).unwrap().clone());
        }
        s.task_spot = at.task_spot.clone();
    }
    let mut spot = s.task_spot().expect("Glass buttons light a spot").clone();
    spot.fit.halo.peak = 0.0;
    spot.fit.core.peak = 0.0;
    s.with_task_spot(Some(spot))
}

fn brightest(lit: &Canvas, bare: &Canvas, x0: i32, cols: std::ops::Range<i32>, y: i32) -> i32 {
    let sum = |c: &Canvas, x: i32| over_grey(c.get(x0 + x, y)).iter().sum::<i32>();
    let lift: Vec<(i32, i32)> = cols.map(|x| (x, sum(lit, x) - sum(bare, x))).collect();
    let top = lift.iter().map(|&(_, l)| l).max().expect("some columns");
    let tied: Vec<i32> = lift
        .iter()
        .filter(|&&(_, l)| l == top)
        .map(|&(x, _)| x)
        .collect();
    tied[tied.len() / 2]
}

fn glass_labelled(
    stacks: &[u8],
    active: &[bool],
    hot: Option<usize>,
    held: Option<usize>,
    glow: Option<crate::theme::Color>,
    scale: i32,
) -> (Canvas, TaskbarLayout) {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    glass_labelled_in(s, stacks, active, (hot, held, None), glow, scale)
}

fn glass_labelled_in(
    s: &Style,
    stacks: &[u8],
    active: &[bool],
    lit: (Option<usize>, Option<usize>, Option<i32>),
    glow: Option<crate::theme::Color>,
    scale: i32,
) -> (Canvas, TaskbarLayout) {
    let (hot, held, x) = lit;
    labelled_row(s, stacks, scale, |st, _| {
        for (t, &a) in st.tasks.iter_mut().zip(active) {
            t.active = a;
            t.glow = glow;
        }
        st.task_hot = hot;
        st.task_pressed = held;
        st.task_hot_x = x;
    })
}

fn labelled_row(
    s: &Style,
    stacks: &[u8],
    scale: i32,
    edit: impl FnOnce(&mut TaskbarState, &mut TaskbarLayout),
) -> (Canvas, TaskbarLayout) {
    let theme = theme_for(Interface::Glass).scaled(scale);
    let base = theme_for(Interface::Glass);
    let counts = TaskbarCounts {
        tasks: stacks.len(),
        ..TaskbarCounts::default()
    };
    let mut layout = TaskbarLayout::compute(&base.metrics, &s.taskbar, 1920, true, counts, 60);
    layout.lay_tasks(
        &base.metrics,
        &s.taskbar,
        stacks,
        crate::taskbar::TaskRow::AtLeast(62),
    );
    let mut layout = layout.scaled(scale);
    let text = TextRenderer::load(&theme.fonts);
    let start = TextRenderer::load(&s.start_text.font());
    let mut st = state_with_tasks(&vec![""; stacks.len()]);
    let px = 16 * scale as u32;
    for t in st.tasks.iter_mut() {
        t.icon = Some(Canvas::new(px, px).into());
    }
    edit(&mut st, &mut layout);
    let mut c = Canvas::new(layout.width as u32, layout.height as u32);
    draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);
    (c, layout)
}

#[test]
fn a_sliding_glass_button_draws_off_its_slot_with_its_stacked_edges() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    for scale in [1, 2] {
        let (c, _) = labelled_row(s, &[1, 2], scale, |st, _| st.task_shift = vec![0, 30]);
        let (want, _) = labelled_row(s, &[1, 2], scale, |_, l| l.tasks[1].x += 30 * scale);
        assert_eq!(
            same_columns(&c, &want, 0..c.width as i32),
            Ok(()),
            "scale {scale}: the group draws 30 px right, as in a slot there"
        );
    }
}

#[test]
fn a_floated_glass_button_draws_lit_over_the_row_and_leaves_its_slot_empty() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let (_, l) = labelled_row(s, &[1, 1, 1], 1, |_, _| {});
    let (x1, x2) = (l.tasks[1].x, l.tasks[2].x);
    let (c, _) = labelled_row(s, &[1, 1, 1], 1, |st, _| st.task_float = Some((2, x1)));
    let (want, _) = labelled_row(s, &[1, 1, 1], 1, |st, _| {
        st.task_shift = vec![0, 0, x1 - x2];
        st.task_hot = Some(2);
    });
    assert_eq!(same_columns(&c, &want, 0..1920), Ok(()));
    let veil = over_veil(s, &l);
    let slot = l.tasks[2];
    assert_eq!(
        same_columns(&c, &veil, slot.x..slot.right()),
        Ok(()),
        "the floated button's slot is empty"
    );
    let (first, _) = labelled_row(s, &[1, 1, 1], 1, |st, _| st.task_float = Some((0, x1)));
    let slot = l.tasks[0];
    assert_eq!(same_columns(&first, &veil, slot.x..slot.right()), Ok(()));
    let r = label_frame(l.tasks[1], 1);
    assert_eq!(same_columns(&first, &c, r.x..r.right()), Ok(()));
}

type Slot = (State, (State, usize), (State, usize));

fn labelled_want(s: &Style, l: &TaskbarLayout, slots: &[Slot], sc: i32) -> Canvas {
    let mut c = over_veil_at(s, l, sc);
    for (r, &(st, left, right)) in l.tasks.iter().zip(slots) {
        label_in(&mut c, s, label_frame(*r, sc), st, sc);
        for ((est, n), x) in [(left, r.x), (right, r.right() - 3 * sc)] {
            let at = Rect::new(x, r.y, 3 * sc, r.height);
            s.draw_nth(&mut c, at, Part::TaskLabelEdge, est, n, sc);
        }
    }
    c
}

#[test]
fn a_labelled_glass_row_draws_each_frame_between_its_pairs() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let n = State::Normal;
    let rest = |st: State| (st, (st, EDGE_LEFT), (st, EDGE_RIGHT));
    let first = (n, (n, EDGE_ROW_START), (n, EDGE_RIGHT));
    let last = (n, (n, EDGE_LEFT), (n, EDGE_ROW_END));
    let beside = |st: State| {
        [
            (n, (n, EDGE_ROW_START), (st, EDGE_PREV_RIGHT)),
            rest(st),
            (n, (st, EDGE_NEXT_LEFT), (n, EDGE_ROW_END)),
        ]
    };
    let c = State::Checked;
    let [before, lit, after] = beside(State::Hot);
    let [_, held, held_after] = beside(State::Pressed);
    type Case = ([bool; 3], Option<usize>, Option<usize>, [Slot; 3]);
    let cases: [Case; 9] = [
        (
            [true, false, false],
            Some(1),
            None,
            [(c, (n, EDGE_ROW_START), (c, EDGE_RIGHT)), lit, after],
        ),
        (
            [false, false, true],
            Some(1),
            None,
            [before, lit, (c, (c, EDGE_LEFT), (n, EDGE_ROW_END))],
        ),
        (
            [true, false, false],
            None,
            Some(1),
            [(c, (n, EDGE_ROW_START), (c, EDGE_RIGHT)), held, held_after],
        ),
        ([false; 3], None, None, [first, rest(n), last]),
        ([false; 3], Some(1), None, beside(State::Hot)),
        (
            [false, true, false],
            None,
            None,
            [first, rest(State::Checked), last],
        ),
        (
            [false, true, false],
            Some(1),
            None,
            beside(State::CheckedHot),
        ),
        ([false; 3], None, Some(1), beside(State::Pressed)),
        (
            [false, true, false],
            Some(1),
            Some(1),
            beside(State::Pressed),
        ),
    ];
    let (s1, s2) = glass_at_1_and_2();
    for (active, hot, held, slots) in cases {
        for (sc, st) in [(1, &s1), (2, &s2)] {
            let lit = (hot, held, None);
            let (c, l) = glass_labelled_in(st, &[1, 1, 1], &active, lit, None, sc);
            let want = labelled_want(st, &l, &slots, sc);
            let row = l.tasks[0].x..l.tasks[2].right();
            assert_eq!(
                same_columns(&c, &want, row),
                Ok(()),
                "scale {sc}: active {active:?} hot {hot:?} held {held:?}"
            );
        }
    }
    let edge = s.part(Part::TaskLabelEdge).unwrap();
    let label = s.part(Part::TaskLabel).unwrap();
    for (a, b) in [(State::Pressed, State::CheckedHot)] {
        let (fa, fb) = (label.frame(a), label.frame(b));
        assert_eq!(
            pixels(&label.atlas, fa),
            pixels(&label.atlas, fb),
            "a held frame is the active lit one"
        );
        for k in 0..4 {
            let (ea, eb) = (edge.frame_nth(a, k), edge.frame_nth(b, k));
            assert_eq!(
                pixels(&edge.atlas, ea),
                pixels(&edge.atlas, eb),
                "held pair {k}"
            );
        }
    }
}

#[test]
fn a_labelled_glass_row_ends_at_a_pinned_slot_and_at_stacked_edges() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let n = State::Normal;
    let (start, end) = ((n, EDGE_ROW_START), (n, EDGE_ROW_END));
    let (c, l) = glass_labelled(&[1, 0, 1, 1], &[false; 4], None, None, None, 1);
    let want = labelled_want(
        s,
        &l,
        &[
            (n, start, end),
            (n, start, start),
            (n, start, (n, EDGE_RIGHT)),
            (n, (n, EDGE_LEFT), end),
        ],
        1,
    );
    for i in [0, 2, 3] {
        let r = l.tasks[i];
        assert_eq!(same_columns(&c, &want, r.x..r.right()), Ok(()), "slot {i}");
    }
    let (c, l) = glass_labelled(&[2, 1], &[false; 2], None, None, None, 1);
    let (front, next) = (l.tasks[0], l.tasks[1]);
    let mut want = over_veil(s, &l);
    let f = Rect::new(front.x, front.y, front.width - 4, front.height);
    s.draw(&mut want, label_frame(f, 1), Part::TaskLabel, n, 1);
    s.draw_nth(
        &mut want,
        Rect::new(f.right() - 3, 0, 3, 40),
        Part::TaskLabelEdge,
        n,
        EDGE_ROW_END,
        1,
    );
    s.draw_nth(
        &mut want,
        Rect::new(next.x, 0, 3, 40),
        Part::TaskLabelEdge,
        n,
        EDGE_ROW_START,
        1,
    );
    assert_eq!(
        same_columns(&c, &want, f.right() - 3..f.right()),
        Ok(()),
        "the group's front ends the run"
    );
    assert_eq!(
        same_columns(&c, &want, next.x..next.x + 3),
        Ok(()),
        "the next button starts one"
    );
}

#[test]
fn a_lit_labelled_glass_button_glows_in_its_icons_colour() {
    let cyan = crate::theme::Color::rgb(113, 164, 168);
    let unlit = glass_unlit_spot();
    let base = &unlit.task_spot().unwrap().labelled.as_ref().unwrap().base;
    for (active, hot, held, state) in [
        ([false; 3], Some(1), None, State::Hot),
        ([false, true, false], Some(1), None, State::CheckedHot),
        ([false; 3], None, Some(1), State::Pressed),
    ] {
        let (c, l) = glass_labelled_in(
            &unlit,
            &[1, 1, 1],
            &active,
            (hot, held, None),
            Some(cyan),
            1,
        );
        let r = label_frame(l.tasks[1], 1);
        let mut want = over_veil(&unlit, &l);
        draw_image(&mut want, base, r, state, 1);
        assert_eq!(same_columns(&c, &want, r.x..r.right()), Ok(()), "{state:?}");
        let (c, _) = glass_labelled(&[1, 1, 1], &active, hot, held, Some(cyan), 1);
        let (plain, _) = glass_labelled(&[1, 1, 1], &active, hot, held, None, 1);
        let [red, _, blue] = over_grey(c.get(r.x + 80, 36));
        let [pred, _, pblue] = over_grey(plain.get(r.x + 80, 36));
        assert!(
            blue - red > pblue - pred,
            "{state:?}: the foot glow leans to cyan ({red} {blue} against {pred} {pblue})"
        );
    }
}

#[test]
fn a_lit_labelled_glass_buttons_pairs_glow_with_it() {
    let cyan = crate::theme::Color::rgb(113, 164, 168);
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let (glowing, edges) = (
        s.task_glow.unwrap().label_edges.unwrap(),
        s.part(Part::TaskLabelEdge).unwrap(),
    );
    for (active, hot, held, state) in [
        ([false; 3], Some(1), None, State::Hot),
        ([false, true, false], Some(1), None, State::CheckedHot),
        ([false; 3], None, Some(1), State::Pressed),
    ] {
        let (c, l) = glass_labelled(&[1, 1, 1], &active, hot, held, Some(cyan), 1);
        let (plain, _) = glass_labelled(&[1, 1, 1], &active, hot, held, None, 1);
        let (before, lit, after) = (l.tasks[0], l.tasks[1], l.tasks[2]);
        let own = [(lit.x, EDGE_LEFT), (lit.right() - 3, EDGE_RIGHT)];
        let given = [
            (before.right() - 3, EDGE_PREV_RIGHT),
            (after.x, EDGE_NEXT_LEFT),
        ];
        let pairs = [own[0], own[1], given[0], given[1]];
        let mut want = over_veil(s, &l);
        for (x, n) in own {
            let one = glowing(edges, state, n, cyan);
            draw_image_nth(&mut want, &one, Rect::new(x, 0, 3, 40), state, 0, 1);
        }
        for (x, n) in given {
            s.draw_nth(
                &mut want,
                Rect::new(x, 0, 3, 40),
                Part::TaskLabelEdge,
                state,
                n,
                1,
            );
        }
        let lean = |c: &Canvas| -> i32 {
            let mut sum = 0;
            for (x, _) in pairs {
                for y in 0..40 {
                    for dx in 0..3 {
                        let [red, _, blue] = over_grey(c.get(x + dx, y));
                        sum += blue - red;
                    }
                }
            }
            sum
        };
        for (x, n) in pairs {
            assert_eq!(
                same_columns(&c, &want, x..x + 3),
                Ok(()),
                "{state:?} pair {n} at {x}"
            );
        }
        assert!(
            lean(&c) > lean(&plain),
            "{state:?}: the pairs lean to cyan ({} against {})",
            lean(&c),
            lean(&plain)
        );
    }
}

#[test]
fn labelled_glass_parts_keep_their_top_and_bottom_rows_on_a_bar_of_another_height() {
    let cyan = crate::theme::Color::rgb(113, 164, 168);
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let lit = s.task_spot().unwrap().labelled.as_ref().unwrap();
    let (base, gain, glowing, edges) = (
        &lit.base,
        &lit.gain,
        s.task_glow.unwrap().label_edges.unwrap(),
        s.part(Part::TaskLabelEdge).unwrap(),
    );
    let all = [
        State::Normal,
        State::Hot,
        State::Checked,
        State::CheckedHot,
        State::Pressed,
    ];
    type Piece<'a> = (String, Box<dyn Fn(&mut Canvas, i32) + 'a>);
    let mut pieces: Vec<Piece> = vec![(
        "veil".into(),
        Box::new(|c, h| s.draw(c, Rect::new(0, 0, 160, h), Part::LabelRow, State::Normal, 1)),
    )];
    for st in all {
        pieces.push((
            format!("{st:?} frame"),
            Box::new(move |c, h| s.draw(c, Rect::new(0, 0, 154, h), Part::TaskLabel, st, 1)),
        ));
        for n in 0..4 {
            pieces.push((
                format!("{st:?} pair {n}"),
                Box::new(move |c, h| {
                    s.draw_nth(c, Rect::new(0, 0, 3, h), Part::TaskLabelEdge, st, n, 1)
                }),
            ));
        }
    }
    for st in [State::Hot, State::CheckedHot, State::Pressed] {
        pieces.push((
            format!("{st:?} frame under the spot"),
            Box::new(move |c, h| draw_image(c, base, Rect::new(0, 0, 154, h), st, 1)),
        ));
        pieces.push((
            format!("{st:?} spot's gain"),
            Box::new(move |c, h| draw_image(c, gain, Rect::new(0, 0, 154, h), st, 1)),
        ));
        for n in [EDGE_LEFT, EDGE_RIGHT] {
            let one = glowing(edges, st, n, cyan);
            pieces.push((
                format!("glowing {st:?} pair {n}"),
                Box::new(move |c, h| draw_image(c, &one, Rect::new(0, 0, 3, h), st, 1)),
            ));
        }
    }
    for (what, draw) in &pieces {
        let mut at40 = Canvas::new(160, 40);
        draw(&mut at40, 40);
        for h in [22, 34, 48] {
            let mut c = Canvas::new(160, h as u32);
            draw(&mut c, h);
            let rows = [
                (0, 0),
                (1, 1),
                (2, 2),
                (h - 3, 37),
                (h - 2, 38),
                (h - 1, 39),
            ];
            for (y, y40) in rows {
                for x in 0..160 {
                    assert_eq!(
                        c.get(x, y),
                        at40.get(x, y40),
                        "{what} at {h} px: ({x}, {y}) is its row {y40} at 40 px"
                    );
                }
            }
        }
    }
}

fn glass_at_1_and_2() -> (std::sync::Arc<Style>, std::sync::Arc<Style>) {
    let one = crate::glass::style(&Default::default());
    let two = one.at_scale(2);
    (one, two)
}

#[test]
fn labelled_glass_buttons_over_grey_land_on_their_colours() {
    let (c, l) = glass_labelled(&[1, 1, 1], &[false, true, false], None, None, None, 1);
    let (lit, _) = glass_labelled(&[1, 1, 1], &[false; 3], Some(1), None, None, 1);
    let bar = over_veil(theme_for(Interface::Glass).look.style().unwrap(), &l);
    let at = |c: &Canvas, i: usize, x: i32, y: i32| over_grey(c.get(l.tasks[i].x + x, y));
    let got = [
        ("veil", over_grey(bar.get(1000, 20))),
        ("veil's top lines", over_grey(bar.get(1000, 1))),
        ("first button's dark line", at(&c, 0, 0, 20)),
        ("first button's light line", at(&c, 0, 1, 20)),
        ("sheen", at(&c, 0, 20, 5)),
        ("plain glass", at(&c, 0, 150, 20)),
        ("the line two buttons share", at(&c, 0, 159, 20)),
        ("active frame", at(&c, 1, 150, 20)),
        ("active foot", at(&c, 1, 80, 38)),
        ("lit foot", at(&lit, 1, 80, 36)),
        ("beside the lit one", at(&lit, 0, 158, 20)),
        ("last button's light line", at(&c, 2, 158, 20)),
        ("last button's dark line", at(&c, 2, 159, 20)),
    ];
    let want: [[i32; 3]; 13] = [
        [75, 88, 103],
        [166, 177, 188],
        [34, 51, 70],
        [130, 142, 154],
        [190, 196, 203],
        [99, 114, 129],
        [80, 96, 112],
        [156, 165, 175],
        [219, 222, 226],
        [224, 224, 225],
        [228, 230, 233],
        [132, 144, 156],
        [35, 52, 71],
    ];
    for ((what, rgb), want) in got.into_iter().zip(want) {
        assert!(
            (0..3).all(|i| (rgb[i] - want[i]).abs() <= 5),
            "{what} over grey is {want:?} within 5, got {rgb:?}"
        );
    }
}

#[test]
fn a_held_glass_button_is_its_lit_active_frame_its_icon_a_pixel_in() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let (lit, l) = glass_stacks_in([true, false, false], Some(0), None);
    let r = l.tasks[0];
    for active in [true, false] {
        let (held, _) = glass_stacks_in([active, false, false], None, Some(0));
        for (x, y) in [(2, 1), (30, 2), (55, 20), (30, 37)] {
            assert_eq!(
                held.get(r.x + x, r.y + y),
                lit.get(r.x + x, r.y + y),
                "active {active}: the frame at ({x}, {y})"
            );
        }
        let white = 0xFFFF_FFFF;
        assert_eq!(lit.get(r.x + 14, r.y + 4), white);
        assert_ne!(held.get(r.x + 14, r.y + 4), white, "active {active}");
        assert_eq!(held.get(r.x + 15, r.y + 5), white, "active {active}");
        assert_eq!(held.get(r.x + 46, r.y + 36), white, "active {active}");
    }
    assert!(
        s.part(Part::TaskButton)
            .unwrap()
            .states
            .contains(&State::Pressed),
        "the button has a held frame"
    );
}

#[test]
fn stacked_glass_edges_take_the_front_buttons_state() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let edges = |c: &Canvas, l: &TaskbarLayout| {
        let r = l.tasks[2];
        (0..8)
            .flat_map(|x| (0..40).map(move |y| (x, y)))
            .map(|(x, y)| c.get(r.x + 52 + x, r.y + y))
            .collect::<Vec<u32>>()
    };
    let want = |state: State| {
        let (_, l) = glass_stacks_in([false; 3], None, None);
        let r = l.tasks[2];
        let mut bar = Canvas::new(1920, 40);
        s.draw(
            &mut bar,
            Rect::new(0, 0, 1920, 40),
            Part::Taskbar,
            State::Normal,
            1,
        );
        for k in 0..2 {
            let e = Rect::new(r.x + 52 + 4 * k, r.y, 4, 40);
            s.draw_nth(&mut bar, e, Part::TaskStack, state, k as usize, 1);
        }
        edges(&bar, &l)
    };
    let cases = [
        ([false; 3], None, None, State::Normal),
        ([false; 3], Some(2), None, State::Hot),
        ([false, false, true], None, None, State::Checked),
        ([false, false, true], Some(2), None, State::CheckedHot),
        ([false; 3], None, Some(2), State::Pressed),
    ];
    let plain = crate::glass::build(&Default::default()).with_task_spot(None);
    let mut seen = Vec::new();
    for (active, hot, held, state) in cases {
        let (c, l) = glass_row_in(&plain, &[1, 2, 3], active, (hot, held, None), None, 1);
        let got = edges(&c, &l);
        assert_eq!(got, want(state), "{state:?} edges");
        seen.push(got);
    }
    assert_ne!(seen[0], seen[1], "lit edges differ from resting ones");
    assert_ne!(seen[0], seen[2], "active edges differ from resting ones");
    assert_eq!(seen[3], seen[4], "held edges are the active, lit pair");
}

#[test]
fn an_idle_pin_draws_no_spot_where_its_slot_lights() {
    let glass = || crate::glass::build(&Default::default());
    assert!(task_draws_spot(&glass(), false), "a running button");
    assert!(
        !task_draws_spot(&glass(), true),
        "an idle pin draws its slot light"
    );
    let unlit = glass().without_part(Part::SlotLight);
    assert!(
        task_draws_spot(&unlit, true),
        "no slot light: the idle pin's button lights"
    );
    let plain = glass().with_task_spot(None);
    assert!(!task_draws_spot(&plain, false), "a style without the spot");
}

#[test]
fn lit_and_held_glass_edges_draw_their_pairs_under_the_spot() {
    let (_, s2) = glass_at_1_and_2();
    let one = crate::glass::build(&Default::default());
    for (sc, s) in [(1, &one), (2, &*s2)] {
        lit_edges_at(s, sc);
    }
}

fn lit_edges_at(s: &Style, sc: i32) {
    let cyan = crate::theme::Color::rgb(113, 164, 168);
    let unlit = glass_unlit_spot_at(sc);
    let pairs = &unlit.task_spot().unwrap().edges.as_ref().unwrap().base;
    let mut bar = Canvas::new(1920 * sc as u32, 40 * sc as u32);
    s.draw(
        &mut bar,
        Rect::new(0, 0, 1920 * sc, 40 * sc),
        Part::Taskbar,
        State::Normal,
        sc,
    );
    let cases = [
        ([false; 3], Some(2), None, State::Hot),
        ([false, false, true], Some(2), None, State::CheckedHot),
        ([false; 3], None, Some(2), State::Pressed),
    ];
    for (active, hot, held, state) in cases {
        let lit = (hot, held, None);
        let (c, l) = glass_row_in(&unlit, &[1, 2, 3], active, lit, Some(cyan), sc);
        let r = l.tasks[2];
        for k in 0..2 {
            let f = pairs.frame_nth(state, k);
            for (x, y) in [(0, 5), (1, 20), (2, 30), (3, 38)] {
                for (i, j) in (0..sc).flat_map(|i| (0..sc).map(move |j| (i, j))) {
                    let (dx, dy) = (sc * x + i, sc * y + j);
                    let at = r.x + sc * (52 + 4 * k as i32) + dx;
                    let mut want = Canvas::new(1, 1);
                    want.put_raw(0, 0, bar.get(at, r.y + dy));
                    let mut edge = Canvas::new(1, 1);
                    edge.put_raw(0, 0, pairs.atlas.get(f.x + dx, f.y + dy));
                    want.blit_over(0, 0, &edge);
                    assert_eq!(
                        c.get(at, r.y + dy),
                        want.get(0, 0),
                        "scale {sc}, {state:?} edge {k} at ({dx}, {dy}): the pair over the bar"
                    );
                }
            }
        }
        let foot = |x: i32| {
            let lit = (hot, held, Some(x));
            let (c, l) = glass_row_in(s, &[1, 2, 3], active, lit, Some(cyan), sc);
            let r = l.tasks[2];
            (52 * sc..60 * sc)
                .map(|dx| over_grey(c.get(r.x + dx, 34 * sc)).iter().sum::<i32>())
                .sum::<i32>()
        };
        assert!(
            foot(59) > foot(0),
            "scale {sc}, {state:?}: the edges light under the pointer"
        );
    }
}

fn glass_bar_with_idle_pin(hover: Hover, scale: i32) -> (Canvas, Rect) {
    let (hot, held) = hover.hot_held();
    let (c, l) = glass_row(&[0, 1, 1], [false; 3], hot, held, None, scale);
    (c, l.tasks[0])
}

/// What an idle pinned slot `r` lit in `state` draws at scale `sc` on a `w` x `h` bar through
/// `s`: the bar, the band (if held) between the 2 px edges, the edges at both ends, the `light`
/// (design px) foot light, then a `px` white icon at `icon`.
#[allow(clippy::too_many_arguments)]
fn idle_pin_want(
    s: &Style,
    (w, h): (i32, i32),
    r: Rect,
    state: State,
    light: (i32, i32),
    px: i32,
    icon: (i32, i32),
    sc: i32,
) -> Canvas {
    let mut c = Canvas::new(w as u32, h as u32);
    s.draw(
        &mut c,
        Rect::new(0, 0, w, h),
        Part::Taskbar,
        State::Normal,
        sc,
    );
    let e = 2 * sc;
    if state == State::Pressed {
        let band = Rect::new(r.x + e, r.y, r.width - 2 * e, r.height);
        s.draw(&mut c, band, Part::SlotBand, State::Pressed, sc);
    }
    for x in [r.x, r.right() - e] {
        s.draw(
            &mut c,
            Rect::new(x, r.y, e, r.height),
            Part::SlotEdge,
            state,
            sc,
        );
    }
    let foot = Rect::new(
        r.x + sc * ((r.width / sc - light.0) / 2),
        r.bottom() - light.1 * sc,
        light.0 * sc,
        light.1 * sc,
    );
    s.draw(&mut c, foot, Part::SlotLight, state, sc);
    c.fill_rect(
        Rect::new(icon.0, icon.1, px, px),
        crate::theme::Color::rgb(0xFF, 0xFF, 0xFF),
    );
    c
}

#[test]
fn a_hot_idle_glass_pin_draws_its_edges_and_foot_light_only() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let (canvas, r) = glass_bar_with_idle_pin(Hover::Hot, 1);
    let rest = glass_bar_with_idle_pin(Hover::None, 1).0;
    let mid = r.y + r.height / 2;
    assert_ne!(canvas.get(r.x, mid), rest.get(r.x, mid), "left edge");
    assert_ne!(
        canvas.get(r.right() - 1, mid),
        rest.get(r.right() - 1, mid),
        "right edge"
    );
    assert_eq!(
        canvas.get(r.x + 4, r.y + 2),
        rest.get(r.x + 4, r.y + 2),
        "no frame"
    );
    let foot = (r.x + r.width / 2, r.bottom() - 2);
    assert!(
        lum(canvas.get(foot.0, foot.1)) > lum(rest.get(foot.0, foot.1)) + 20,
        "foot light"
    );
    let want = idle_pin_want(
        s,
        (1920, 40),
        r,
        State::Hot,
        (60, 7),
        32,
        (r.x + 14, r.y + 4),
        1,
    );
    assert_eq!(same_columns(&canvas, &want, r.x..r.right()), Ok(()));
}

#[test]
fn a_lit_idle_glass_pin_at_scale_2_draws_its_pieces_painted_at_2() {
    let (_, s2) = glass_at_1_and_2();
    for (hover, state) in [(Hover::Hot, State::Hot), (Hover::Held, State::Pressed)] {
        let (hot, held) = hover.hot_held();
        let (c, l) = glass_row_in(&s2, &[0, 1, 1], [false; 3], (hot, held, None), None, 2);
        let r = l.tasks[0];
        let o = if state == State::Pressed { 2 } else { 0 };
        let icon = (r.x + 28 + o, r.y + 8 + o);
        let want = idle_pin_want(&s2, (3840, 80), r, state, (60, 7), 64, icon, 2);
        assert_eq!(same_columns(&c, &want, r.x..r.right()), Ok(()), "{hover:?}");
    }
}

#[test]
fn a_held_idle_glass_pin_lights_its_slot_and_nudges_its_icon() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let (held, r) = glass_bar_with_idle_pin(Hover::Held, 1);
    let hot = glass_bar_with_idle_pin(Hover::Hot, 1).0;
    let rest = glass_bar_with_idle_pin(Hover::None, 1).0;
    let (x, y) = (r.x + 7, r.y + r.height / 2);
    assert!(
        lum(held.get(x, y)) < lum(rest.get(x, y)) - 20,
        "the band darkens the slot"
    );
    let white = 0xFFFF_FFFF;
    assert_eq!(hot.get(r.x + 14, r.y + 4), white, "hot icon's top left");
    assert_ne!(held.get(r.x + 14, r.y + 4), white, "held icon moved off it");
    for (dx, dy) in [(14, 4), (45, 4), (14, 35), (45, 35)] {
        assert_eq!(
            held.get(r.x + dx + 1, r.y + dy + 1),
            hot.get(r.x + dx, r.y + dy),
            "icon corner ({dx}, {dy}) a pixel right and down"
        );
    }
    let want = idle_pin_want(
        s,
        (1920, 40),
        r,
        State::Pressed,
        (60, 7),
        32,
        (r.x + 15, r.y + 5),
        1,
    );
    assert_eq!(same_columns(&held, &want, r.x..r.right()), Ok(()));
    let cyan = crate::theme::Color::rgb(113, 164, 168);
    let (glowing, _) = glass_row(&[0, 1, 1], [false; 3], None, Some(0), Some(cyan), 1);
    assert_eq!(same_columns(&glowing, &held, r.x..r.right()), Ok(()));
}

#[test]
fn a_lit_idle_glass_pin_on_the_small_bar_draws_the_small_pieces() {
    let small = crate::glass::small_bar_style(&Default::default());
    for sc in [1, 2] {
        small_idle_pin_at(&small.at_scale(sc), sc);
    }
}

fn small_idle_pin_at(s: &Style, sc: i32) {
    let theme = theme_for(Interface::Glass);
    let h = crate::glass::SMALL_TASKBAR_HEIGHT;
    let metrics = crate::theme::Metrics {
        taskbar_height: h,
        ..theme.metrics.clone()
    };
    let counts = TaskbarCounts {
        tasks: 2,
        ..TaskbarCounts::default()
    };
    let mut layout = TaskbarLayout::compute(&metrics, &s.taskbar, 1920, true, counts, 60);
    layout.lay_tasks(
        &metrics,
        &s.taskbar,
        &[0, 1],
        crate::taskbar::TaskRow::Icons,
    );
    assert_eq!(layout.tasks[0].width, 44, "the small bar's slot");
    let layout = layout.scaled(sc);
    let theme = theme.scaled(sc);
    let text = TextRenderer::load(&theme.fonts);
    let start = TextRenderer::load(&s.start_text.font());
    let px = s.taskbar.task_icon * sc;
    let r = layout.tasks[0];
    let (w, h) = (layout.width, layout.height);
    for (hover, state) in [(Hover::Hot, State::Hot), (Hover::Held, State::Pressed)] {
        let mut st = state_with_tasks(&["", ""]);
        for t in &mut st.tasks {
            t.icon = Some(solid_icon(crate::theme::Color::rgb(0xFF, 0xFF, 0xFF), px).into());
        }
        (st.task_hot, st.task_pressed) = hover.hot_held();
        let mut c = Canvas::new(w as u32, h as u32);
        draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);
        let o = sc * i32::from(state == State::Pressed);
        let icon = (r.x + (r.width - px) / 2 + o, r.y + (h - px) / 2 + o);
        let want = idle_pin_want(s, (w, h), r, state, (44, 7), px, icon, sc);
        assert_eq!(
            same_columns(&c, &want, r.x..r.right()),
            Ok(()),
            "scale {sc}: {hover:?}"
        );
    }
}

fn small_bar_in(state: &str) -> Canvas {
    let s = crate::glass::small_bar_style(&Default::default());
    let theme = theme_for(Interface::Glass);
    let metrics = crate::theme::Metrics {
        taskbar_height: crate::glass::SMALL_TASKBAR_HEIGHT,
        ..theme.metrics.clone()
    };
    let counts = TaskbarCounts {
        tasks: 5,
        show_desktop: true,
        ..TaskbarCounts::default()
    };
    let mut layout = TaskbarLayout::compute(&metrics, &s.taskbar, 1920, true, counts, 60);
    layout.lay_tasks(
        &metrics,
        &s.taskbar,
        &[0, 1, 0, 3, 1],
        crate::taskbar::TaskRow::Icons,
    );
    let mut st = state_with_tasks(&["", "", "", "", ""]);
    st.start_icon = Some(clear_start_logo());
    for t in &mut st.tasks {
        t.icon = Some(Canvas::new(16, 16).into());
    }
    let calc_active = matches!(state, "active_calc" | "hot_active_calc");
    st.tasks[3].active = !calc_active;
    st.tasks[4].active = calc_active;
    st.start_hot = state == "orb_hot";
    st.start_pressed = state == "orb_pressed";
    st.show_desktop_hot = state == "strip_hot";
    st.task_hot = match state {
        "hot_explorer" => Some(1),
        "hot_active_calc" => Some(4),
        _ => None,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let mut c = Canvas::new(1920, 30);
    draw_taskbar(&mut c, &s, &theme, &text, &start, &layout, &st);
    let r = layout.start_orb(st.start_pressed).unwrap();
    let mut orb = Canvas::new(r.width as u32, r.height as u32);
    draw_start_above(&mut orb, &s, &layout, &st, 1);
    c.blit_over(r.x, r.y, &orb);
    c
}

#[test]
fn the_small_glass_bar_lights_its_buttons_under_the_spot() {
    let spot: [(&str, i32, i32, [u8; 3]); 6] = [
        ("hot_explorer", 107, 2, [214, 217, 221]),
        ("hot_explorer", 127, 27, [233, 233, 233]),
        ("hot_explorer", 146, 15, [202, 205, 209]),
        ("hot_active_calc", 245, 2, [242, 242, 243]),
        ("hot_active_calc", 265, 27, [242, 242, 242]),
        ("hot_active_calc", 284, 15, [218, 219, 221]),
    ];
    let points = spot.map(|(state, x, y, want)| (state, x, y, want, 1));
    let mut drawn: Vec<(&str, Canvas)> = Vec::new();
    for (state, x, y, want, within) in points {
        if !drawn.iter().any(|(s, _)| *s == state) {
            drawn.push((state, small_bar_in(state)));
        }
        let c = &drawn.iter().find(|(s, _)| *s == state).unwrap().1;
        let got = over_grey(c.get(x, y));
        for (g, w) in got.iter().zip(want) {
            assert!(
                (g - i32::from(w)).abs() <= within,
                "{state}: ({x}, {y}) over grey is {want:?} within {within}, got {got:?}"
            );
        }
    }
    assert_eq!(drawn.len(), 2, "both lit states have points");
}

#[test]
fn an_icon_only_glass_button_centres_its_icon() {
    for (s, height, px) in [
        (crate::glass::style(&Default::default()), 40, 32),
        (crate::glass::small_bar_style(&Default::default()), 30, 16),
    ] {
        assert_eq!(s.taskbar.task_icon, px, "{height} px bar's icon size");
        let magenta = solid_icon(crate::theme::Color::rgb(0xFF, 0, 0xFF), px);
        let ink = magenta.get(0, 0);
        let theme = theme_for(Interface::Glass);
        let metrics = crate::theme::Metrics {
            taskbar_height: height,
            ..theme.metrics.clone()
        };
        let counts = TaskbarCounts {
            tasks: 2,
            ..TaskbarCounts::default()
        };
        let mut layout = TaskbarLayout::compute(&metrics, &s.taskbar, 1920, true, counts, 60);
        layout.lay_tasks(
            &metrics,
            &s.taskbar,
            &[1, 3],
            crate::taskbar::TaskRow::Icons,
        );
        let mut st = state_with_tasks(&["", ""]);
        for t in &mut st.tasks {
            t.icon = Some(magenta.clone().into());
        }
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let mut c = Canvas::new(1920, height as u32);
        draw_taskbar(&mut c, &s, &theme, &text, &start, &layout, &st);
        for (r, front) in [(layout.tasks[0], 0), (layout.tasks[1], 8)] {
            let w = r.width - front;
            let (x, y) = (r.x + (w - px) / 2, r.y + (height - px) / 2);
            assert_eq!(c.get(x, y), ink, "{height} px bar: top-left at ({x}, {y})");
            let far = px - 1;
            assert_eq!(
                c.get(x + far, y + far),
                ink,
                "{height} px bar: bottom-right"
            );
            assert_ne!(c.get(x - 1, y), ink, "{height} px bar: left of the icon");
            assert_ne!(c.get(x + px, y), ink, "{height} px bar: right of the icon");
        }
    }
}

#[test]
fn a_labelled_glass_buttons_icon_centres_in_its_32_px_box_10_px_in() {
    let s = crate::glass::style(&Default::default());
    let magenta = solid_icon(crate::theme::Color::rgb(0xFF, 0, 0xFF), 32);
    let ink = magenta.get(0, 0);
    let theme = theme_for(Interface::Glass);
    let counts = TaskbarCounts {
        tasks: 2,
        ..TaskbarCounts::default()
    };
    let mut layout = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 1920, true, counts, 60);
    layout.lay_tasks(
        &theme.metrics,
        &s.taskbar,
        &[1, 1],
        crate::taskbar::TaskRow::AtLeast(62),
    );
    let mut st = state_with_tasks(&["", "Notes"]);
    for t in &mut st.tasks {
        t.icon = Some(magenta.clone().into());
    }
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let mut c = Canvas::new(1920, 40);
    draw_taskbar(&mut c, &s, &theme, &text, &start, &layout, &st);
    for (i, r) in layout.tasks.iter().enumerate() {
        assert!(r.width > 100, "button {i} is a labelled one");
        let (x, y) = (r.x + 10, r.y + 4);
        assert_eq!(
            c.get(x, y),
            ink,
            "button {i}: icon's top-left at ({x}, {y})"
        );
        assert_ne!(c.get(x - 1, y), ink, "button {i}: left of the icon");
    }
}

#[test]
fn a_narrow_labelled_glass_button_keeps_its_icon_inside_it() {
    let s = crate::glass::style(&Default::default());
    let magenta = solid_icon(crate::theme::Color::rgb(0xFF, 0, 0xFF), 32);
    let ink = magenta.get(0, 0);
    let theme = theme_for(Interface::Glass);
    let counts = TaskbarCounts {
        tasks: 4,
        ..TaskbarCounts::default()
    };
    let mut layout = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 1920, true, counts, 60);
    layout.lay_tasks(
        &theme.metrics,
        &s.taskbar,
        &[1, 1, 1, 1],
        crate::taskbar::TaskRow::Shared,
    );
    layout.tasks = [(100, 36), (140, 12), (156, 48), (220, 2)]
        .map(|(x, w)| Rect::new(x, 0, w, 40))
        .to_vec();
    let mut st = state_with_tasks(&["", "", "", ""]);
    for t in &mut st.tasks {
        t.icon = Some(magenta.clone().into());
    }
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let mut c = Canvas::new(1920, 40);
    draw_taskbar(&mut c, &s, &theme, &text, &start, &layout, &st);
    let inked = |r: &Rect| pixels(&c, *r).iter().filter(|&&p| p == ink).count();
    let counts: Vec<usize> = layout.tasks.iter().map(inked).collect();
    assert_eq!(
        counts,
        [1024, 0, 1024, 0],
        "whole icons, each inside its button"
    );
    assert_eq!(c.get(102, 4), ink, "the narrow button's icon is centred");
    assert_ne!(c.get(101, 4), ink, "left of the centred icon");
    assert_eq!(c.get(156 + 10, 4), ink, "the wide one's sits in its box");
    let bar = over_veil(&s, &layout);
    for x in [219, 222] {
        assert_eq!(
            same_columns(&c, &bar, x..x + 1),
            Ok(()),
            "a 2 px button's pairs stay inside it"
        );
    }
}

#[test]
fn a_squeezed_small_bar_button_centres_its_icon() {
    let magenta = solid_icon(crate::theme::Color::rgb(0xFF, 0, 0xFF), 16);
    let ink = magenta.get(0, 0);
    let s = crate::glass::small_bar_style(&Default::default());
    let theme = theme_for(Interface::Glass);
    let metrics = crate::theme::Metrics {
        taskbar_height: 30,
        ..theme.metrics.clone()
    };
    let counts = TaskbarCounts {
        tasks: 3,
        ..TaskbarCounts::default()
    };
    let mut layout = TaskbarLayout::compute(&metrics, &s.taskbar, 1920, true, counts, 60);
    layout.lay_tasks(
        &metrics,
        &s.taskbar,
        &[1, 1, 1],
        crate::taskbar::TaskRow::Shared,
    );
    layout.tasks = [(100, 20), (120, 12), (132, 48)]
        .map(|(x, w)| Rect::new(x, 0, w, 30))
        .to_vec();
    let mut st = state_with_tasks(&["", "", ""]);
    for t in &mut st.tasks {
        t.icon = Some(magenta.clone().into());
    }
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let mut c = Canvas::new(1920, 30);
    draw_taskbar(&mut c, &s, &theme, &text, &start, &layout, &st);
    let inked = |r: &Rect| pixels(&c, *r).iter().filter(|&&p| p == ink).count();
    let counts: Vec<usize> = layout.tasks.iter().map(inked).collect();
    assert_eq!(counts, [256, 0, 256], "whole icons, each inside its button");
    assert_eq!(c.get(102, 7), ink, "the 20 px button's icon is centred");
    assert_ne!(c.get(101, 7), ink, "left of the centred icon");
    let lb = s.taskbar.labelled;
    let x = 132 + lb.icon_x + (lb.icon_box - 16) / 2;
    assert_eq!(c.get(x, 7), ink, "the wide one's sits in its box");
}

#[test]
fn classic_and_blue_have_no_stacked_edges() {
    for i in [Interface::Classic, Interface::Blue] {
        let theme = theme_for(i);
        if let Some(s) = theme.look.style() {
            assert!(s.part(Part::TaskStack).is_none(), "{i:?} stacks nothing");
        }
    }
}

#[test]
fn a_hot_glass_button_glows_in_its_icons_colour() {
    use crate::theme::Color;
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let glow = s.task_glow.expect("Glass buttons glow in colour");
    let yellow = solid_icon(Color::rgb(0xF0, 0xC8, 0x40), 32);
    let blue = solid_icon(Color::rgb(0x30, 0x70, 0xD0), 32);
    let hot = |icon: &Canvas, with_glow: bool| {
        let mut st = state_with_tasks(&["One", "Two"]);
        st.tasks[1].active = true;
        for t in &mut st.tasks {
            t.icon = Some(icon.clone().into());
            t.glow = if with_glow { (glow.colour)(icon) } else { None };
        }
        st.task_hot_x = Some(100);
        let (a, l) = {
            st.task_hot = Some(0);
            draw_glass(s, 40, &st)
        };
        st.task_hot = Some(1);
        let (b, _) = draw_glass(s, 40, &st);
        let at = |c: &Canvas, r: Rect| over_grey(c.get(r.x + 100, r.y + 34));
        (at(&a, l.tasks[0]), at(&b, l.tasks[1]))
    };
    let grey = hot(&yellow, false);
    let (y_hot, y_checked) = hot(&yellow, true);
    let (b_hot, b_checked) = hot(&blue, true);
    for ([r, g, b], w) in [(y_hot, grey.0), (y_checked, grey.1)] {
        assert!(
            r > b + 20 && g > b + 10,
            "yellow icon: ({r}, {g}, {b}), no colour {w:?}"
        );
    }
    for ([r, g, b], w) in [(b_hot, grey.0), (b_checked, grey.1)] {
        assert!(
            b > g && b - r > w[2] - w[0] + 15,
            "blue icon: ({r}, {g}, {b}), no colour {w:?}"
        );
    }
}

#[test]
fn the_spot_follows_the_pointer() {
    // The third button, clear of the denser glass under the orb, so
    // the bar under it is even. At scale 2, over the frames painted at
    // 2, it lies where it does at 1, in device px twice over.
    let (_, s2) = glass_at_1_and_2();
    let one = crate::glass::build(&Default::default());
    let yellow = crate::theme::Color::rgb(0xF0, 0xC8, 0x40);
    for (sc, s) in [(1, &one), (2, &*s2)] {
        let unlit = glass_unlit_spot_at(sc);
        for x in [6, 30, 54] {
            let lit = (Some(2), None, Some(x));
            let (c, l) = glass_row_in(s, &[1, 1, 1], [false; 3], lit, Some(yellow), sc);
            let (bare, _) = glass_row_in(&unlit, &[1, 1, 1], [false; 3], lit, Some(yellow), sc);
            let got = brightest(&c, &bare, l.tasks[2].x, 3 * sc..57 * sc, 36 * sc);
            assert!(
                (got / sc - x.clamp(3, 56)).abs() <= 3,
                "scale {sc}, the pointer at {x}: the foot is brightest at {got}"
            );
        }
    }
}

#[test]
fn the_spot_stays_inside_the_frame() {
    let s = crate::glass::build(&Default::default());
    let unlit = glass_unlit_spot();
    let spot = s.task_spot().unwrap();
    let yellow = crate::theme::Color::rgb(0xF0, 0xC8, 0x40);
    let states = [
        (State::Hot, false, None),
        (State::CheckedHot, true, None),
        (State::Pressed, false, Some(0)),
    ];
    for scale in [1, 2] {
        for (state, active, held) in states {
            for (labelled, stacks) in [(false, [1, 1, 1]), (false, [2, 1, 1]), (true, [1, 1, 1])] {
                for x in [0, 59] {
                    let lit = (Some(0), held, Some(x));
                    let act = [active, false, false];
                    let row = |st: &Style| {
                        if labelled {
                            glass_labelled_in(st, &stacks, &act, lit, Some(yellow), scale)
                        } else {
                            glass_row_in(st, &stacks, act, lit, Some(yellow), scale)
                        }
                    };
                    let (c, l) = row(&s);
                    let (bare, _) = row(&unlit);
                    let what = format!(
                        "{state:?}, labelled {labelled}, scale {scale}, stacks {stacks:?}, pointer at {x}"
                    );
                    let mut frame = Canvas::new(l.width as u32, l.height as u32);
                    let mut edges = Vec::new();
                    if labelled {
                        let f = spot.labelled.as_ref().unwrap();
                        draw_image(
                            &mut frame,
                            &f.base,
                            label_frame(l.tasks[0], scale),
                            state,
                            scale,
                        );
                    } else {
                        let front;
                        (front, edges) = stacked(&s, l.tasks[0], l.stacks.first().copied(), scale);
                        draw_image(&mut frame, &spot.button.base, front, state, scale);
                        let pairs = &spot.edges.as_ref().unwrap().base;
                        for (k, e) in edges.iter().enumerate() {
                            draw_image_nth(&mut frame, pairs, *e, state, k, scale);
                        }
                    }
                    let mut lit_px = 0;
                    for y in 0..l.height {
                        for px in 0..l.width {
                            if c.get(px, y) != bare.get(px, y) {
                                lit_px += 1;
                                assert_ne!(
                                    frame.get(px, y) >> 24,
                                    0,
                                    "{what}: ({px}, {y}) lies outside the frame"
                                );
                            }
                        }
                    }
                    assert!(lit_px > 0, "{what}: the spot shows");
                    if stacks[0] == 2 {
                        let e = edges[0];
                        let edge_lit = (e.x..e.right())
                            .any(|px| (0..l.height).any(|y| c.get(px, y) != bare.get(px, y)));
                        assert!(edge_lit, "{what}: the spot lights the stacked edge");
                    }
                }
            }
        }
    }
}

#[test]
fn a_spot_at_the_middle_lands_on_its_colours() {
    let s = crate::glass::build(&Default::default());
    let glow = crate::theme::Color::rgb(191, 200, 150);
    type Points = [((i32, i32), [i32; 3]); 6];
    let want: [(bool, Points); 2] = [
        (
            false,
            [
                ((30, 36), [236, 233, 206]),
                ((2, 37), [228, 226, 193]),
                ((57, 11), [198, 202, 189]),
                ((10, 34), [216, 214, 177]),
                ((30, 2), [194, 199, 196]),
                ((50, 38), [246, 243, 227]),
            ],
        ),
        (
            true,
            [
                ((30, 36), [245, 241, 225]),
                ((2, 37), [240, 236, 205]),
                ((57, 11), [226, 226, 205]),
                ((10, 34), [232, 229, 194]),
                ((30, 2), [229, 231, 228]),
                ((50, 38), [254, 254, 254]),
            ],
        ),
    ];
    for (active, points) in want {
        let lit = (Some(0), None, Some(30));
        let active = [active, false, false];
        let (c, l) = glass_row_in(&s, &[1, 1, 1], active, lit, Some(glow), 1);
        let r = l.tasks[0];
        for ((x, y), rgb) in points {
            let got = over_grey(c.get(r.x + x, r.y + y));
            assert!(
                (0..3).all(|i| (got[i] - rgb[i]).abs() <= 1),
                "active {}: ({x}, {y}) is {got:?}, want {rgb:?}",
                active[0]
            );
        }
        let middle = (Some(0), None, None);
        let (centred, _) = glass_row_in(&s, &[1, 1, 1], active, middle, Some(glow), 1);
        assert_eq!(
            same_columns(&c, &centred, r.x..r.right()),
            Ok(()),
            "a lit button with no pointer x draws the spot at its middle"
        );
    }
}

#[test]
fn a_labelled_spot_follows_across_the_label() {
    let s = crate::glass::build(&Default::default());
    let unlit = glass_unlit_spot();
    let yellow = crate::theme::Color::rgb(0xF0, 0xC8, 0x40);
    let draw = |s: &Style, x: i32| {
        let lit = (Some(1), None, Some(x));
        glass_labelled_in(s, &[1, 1, 1], &[false; 3], lit, Some(yellow), 1)
    };
    let ((left, l), (right, _)) = (draw(&s, 20), draw(&s, 140));
    let r = l.tasks[1];
    assert_eq!(r.width, 160, "a 160 px labelled button");
    let foot = |c: &Canvas, cols: std::ops::Range<i32>| -> i32 {
        cols.flat_map(|x| (33..37).map(move |y| (x, y)))
            .map(|(x, y)| over_grey(c.get(r.x + x, r.y + y)).iter().sum::<i32>())
            .sum()
    };
    assert!(
        foot(&left, 6..46) > foot(&right, 6..46),
        "the left end lights under the pointer"
    );
    assert!(
        foot(&right, 114..154) > foot(&left, 114..154),
        "the right end lights under it"
    );
    let (bare, _) = draw(&unlit, 140);
    let got = brightest(&right, &bare, r.x, 48..154, 22);
    assert!(
        (got - 140).abs() <= 3,
        "the pointer at 140: the middle row is brightest at {got}"
    );
}

#[test]
fn a_grey_icon_lights_a_neutral_spot() {
    use crate::style::{Layout, SpotFrames};
    use crate::theme::Color;
    let s = crate::glass::build(&Default::default());
    let fit = s.task_spot().unwrap().fit;
    assert_eq!(
        fit.halo.colour(None),
        [136.0; 3],
        "the halo: its white's luma"
    );
    assert_eq!(
        fit.core.colour(None),
        [250.0; 3],
        "the core: its white's luma"
    );
    let grey = Some(Color::rgb(0x77, 0x77, 0x77));
    assert_eq!(fit.halo.colour(grey), [136.0; 3], "a grey glow has no hue");
    let mut gain = Canvas::new(60, 40);
    gain.fill_rect(gain.bounds(), Color::rgb(0, 0, 32));
    let frames = SpotFrames {
        base: PartImage::new(Canvas::new(60, 40), &[State::Hot], Layout::Vertical),
        gain: PartImage::new(gain, &[State::Hot], Layout::Vertical),
    };
    let r = Rect::new(0, 0, 60, 40);
    let lit = |glow: Option<Color>| {
        let span = Span {
            x: 0,
            right: 60,
            pointer: None,
        };
        let spot = Spot {
            fit: &fit,
            span,
            glow,
        };
        let mut c = Canvas::new(60, 40);
        draw_spot(&mut c, &spot, &frames, r, State::Hot, 0, 1);
        c
    };
    let c = lit(None);
    for y in 0..40 {
        for x in 0..60 {
            let px = c.get(x, y);
            let [r, g, b] = [16, 8, 0].map(|sh| (px >> sh) & 0xFF);
            assert!(r == g && g == b, "({x}, {y}) is grey: {px:08x}");
        }
    }
    assert!(c.get(32, 34) >> 24 > 200, "the foot is lit");
    let warm = lit(Some(Color::rgb(0xF0, 0xC8, 0x40))).get(32, 34);
    assert!(
        (warm >> 16 & 0xFF) > (warm & 0xFF) + 20,
        "a yellow icon's spot is warm: {warm:08x}"
    );
}

#[test]
fn a_checked_glass_buttons_icon_and_title_are_not_pushed_in() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    assert!(!s.task_nudge);
    let icon = solid_icon(crate::theme::Color::rgb(0xFF, 0, 0xFF), 32);
    let magenta = icon.get(0, 0);
    let mut st = state_with_tasks(&["Notes", "Notes"]);
    st.tasks[1].active = true;
    for t in &mut st.tasks {
        t.icon = Some(icon.clone().into());
    }
    let (c, l) = draw_glass(s, 40, &st);
    let icon_px = s.taskbar.task_icon;
    for (i, r) in l.tasks.iter().enumerate() {
        let (x, y) = (r.x + 10, r.y + (r.height - icon_px) / 2);
        assert_eq!(c.get(x, y), magenta, "task {i}'s icon at ({x}, {y})");
        assert_ne!(c.get(x - 1, y), magenta, "task {i} left of its icon");
        assert_ne!(c.get(x, y - 1), magenta, "task {i} above its icon");
    }
    let ink = |r: Rect| {
        let white = s.bar_text.argb();
        let inked: Vec<(i32, i32)> = (r.y + 5..r.bottom() - 5)
            .flat_map(|y| (r.x + 30..r.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == white)
            .collect();
        let left = inked.iter().map(|&(x, _)| x - r.x).min();
        let top = inked.iter().map(|&(_, y)| y - r.y).min();
        (left, top)
    };
    assert!(ink(l.tasks[0]).0.is_some(), "the title inks");
    assert_eq!(ink(l.tasks[0]), ink(l.tasks[1]));
}
