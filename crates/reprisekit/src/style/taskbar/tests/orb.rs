use super::*;

#[test]
fn the_mark_stands_20_px_on_the_orb_centred_and_a_theme_logo_keeps_its_size() {
    let mut s = (*crate::glass::style(&Default::default())).clone();
    s.start_bloom = None;
    let mut st = state_with_tasks(&[]);
    let (with_mark, layout) = draw_glass(&s, 40, &st);
    st.start_icon = Some(Canvas::new(24, 24).into());
    let (without, _) = draw_glass(&s, 40, &st);
    let disc = layout.start_disc.unwrap_or(layout.start);
    let (cx, cy) = (disc.x + disc.width / 2, disc.y + disc.height / 2);
    let ink: Vec<(i32, i32)> = (0..40)
        .flat_map(|y| (disc.x..disc.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| with_mark.get(x, y) != without.get(x, y))
        .collect();
    let span = |f: fn(&(i32, i32)) -> i32| {
        let v = ink.iter().map(f);
        (v.clone().min().unwrap(), v.max().unwrap())
    };
    let ((x0, x1), (y0, y1)) = (span(|p| p.0), span(|p| p.1));
    assert!(
        x0 >= cx - 10 && x1 < cx + 10,
        "across {x0}..={x1} round {cx}"
    );
    assert!(y0 >= cy - 10 && y1 < cy + 10, "down {y0}..={y1} round {cy}");
    assert!(x1 - x0 >= 15 && y1 - y0 >= 15, "{x0}..={x1}, {y0}..={y1}");
    for d in [1, 2] {
        let l = layout.scaled(d);
        let mut st = state_with_tasks(&[]);
        let i = start_icon_box(&s, &l, &st, d);
        assert_eq!((i.width, i.height), (20 * d, 20 * d), "the mark at {d}");
        let disc = l.start_disc.unwrap_or(l.start);
        assert_eq!(2 * (i.x - disc.x) + i.width, disc.width, "centred at {d}");
        st.start_icon = Some(Canvas::new(24 * d as u32, 24 * d as u32).into());
        let i = start_icon_box(&s, &l, &st, d);
        assert_eq!(i.width, 24 * d, "a theme logo at its own size at {d}");
    }
}

#[test]
fn a_lit_orb_glows_onto_the_bar_from_a_glow_built_once_per_state() {
    let s = crate::glass::build(&Default::default());
    let mut bare = s.clone();
    bare.start_bloom = None;
    let mut st = state_with_tasks(&[]);
    st.start_hot = true;
    let (lit, layout) = draw_glass(&s, 40, &st);
    let (plain, _) = draw_glass(&bare, 40, &st);
    // Left of the disc, on the bar: lighter with the glow, never darker.
    // The disc fills x 9..45 of Start, centred on row 20.
    let (x, y) = (layout.start.x + 8, layout.start.y + 20);
    let ch = |p: u32| [p >> 16, p >> 8, p].map(|c| c & 0xFF);
    let (a, b) = (ch(lit.get(x, y)), ch(plain.get(x, y)));
    assert!(a.iter().zip(&b).all(|(a, b)| a > b), "{a:?} over {b:?}");
    st.orb_glow = 0;
    assert_eq!(draw_glass(&s, 40, &st).0.pixels, plain.pixels, "orb_glow 0");
    st.orb_glow = 100;
    for (hot, pressed, look) in [
        (false, false, State::Normal),
        (true, false, State::Hot),
        (true, true, State::Pressed),
    ] {
        st.start_hot = hot;
        st.start_pressed = pressed;
        draw_glass(&s, 40, &st);
        let r = layout.start_orb(pressed).unwrap();
        let mut orb = Canvas::new(r.width as u32, r.height as u32);
        draw_start_above(&mut orb, &s, &layout, &st, 1);
        let kept = s.start_bloom.as_ref().unwrap();
        let px = start_icon_box(&s, &layout, &st, 1).width;
        kept.get(IconKey::Mark(px), 1, look, 100, || {
            panic!("{look:?} built again")
        });
    }
}

#[test]
fn a_warm_paints_the_orb_glow_drawn_at_scale_1_for_scale_2() {
    let d = crate::glass::Settings::default();
    let s = Arc::new(crate::glass::build(&d).with_rescale(move |_| crate::glass::build(&d)));
    let mut st = state_with_tasks(&[]);
    st.start_hot = true;
    draw_glass(&s, 40, &st);
    let mut logo = Canvas::new(48, 48);
    logo.fill_rect(Rect::new(8, 8, 32, 32), Color::rgb(0xFF, 0x80, 0));
    for icon in [None, Some(Arc::new(logo))] {
        st.start_icon = icon.clone();
        draw_glass(&s, 40, &st);
        let warm = (s.warm(2, icon.clone())).expect("the scale 1 glow is unpainted at scale 2");
        std::thread::spawn(move || warm.paint()).join().unwrap();
        let two = s.at_scale(2);
        let icon = icon.as_deref();
        assert!(
            keeps_orb_bloom(&two, 2, icon, State::Hot, 100),
            "the scale 2 glow round the logo ({}) is painted before the first draw",
            icon.is_some()
        );
        assert!(
            !keeps_orb_bloom(&two, 2, icon, State::Normal, 100),
            "a state not drawn at scale 1 waits for its draw"
        );
    }
}

#[test]
fn the_small_orb_centres_its_icon_on_the_disc_and_draws_its_rows_above() {
    let mut s = (*crate::glass::small_bar_style(&Default::default())).clone();
    s.start_bloom = None;
    let theme = theme_for(Interface::Glass);
    let metrics = crate::theme::Metrics {
        taskbar_height: crate::glass::SMALL_TASKBAR_HEIGHT,
        ..theme.metrics.clone()
    };
    let layout = TaskbarLayout::compute(
        &metrics,
        &s.taskbar,
        1920,
        true,
        TaskbarCounts::default(),
        60,
    );
    let magenta = crate::theme::Color::rgb(0xFF, 0, 0xFF);
    let mut icon = Canvas::new(24, 24);
    icon.fill_rect(Rect::new(0, 0, 24, 24), magenta);
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let bare = {
        let mut c = Canvas::new(1920, 30);
        s.draw(
            &mut c,
            Rect::new(0, 0, 1920, 30),
            Part::Taskbar,
            State::Normal,
            1,
        );
        c
    };
    for (hot, pressed, state) in [
        (false, false, State::Normal),
        (true, false, State::Hot),
        (false, true, State::Pressed),
    ] {
        let mut st = state_with_tasks(&[]);
        st.start_icon = Some(icon.clone().into());
        st.start_hot = hot;
        st.start_pressed = pressed;
        let mut bar = Canvas::new(1920, 30);
        draw_taskbar(&mut bar, &s, &theme, &text, &start, &layout, &st);
        assert_eq!(
            bar.get(26, 13) == bare.get(26, 13),
            pressed,
            "{state:?}: the bar draws the orb unless held"
        );
        let r = layout.start_orb(pressed).unwrap();
        let want = if pressed {
            (0, -13, 54, 43)
        } else {
            (0, -13, 54, 13)
        };
        assert_eq!((r.x, r.y, r.width, r.height), want, "{state:?}: its buffer");
        let mut orb = Canvas::new(r.width as u32, r.height as u32);
        draw_start_above(&mut orb, &s, &layout, &st, 1);
        let mut c = Canvas::new(1920, 43);
        c.blit_canvas(0, 13, &bar);
        c.blit_over(r.x, r.y + 13, &orb);
        let ink = magenta.argb();
        let at = |x, y| c.get(x, y + 13);
        assert_eq!(
            at(15, 1),
            ink,
            "{state:?}: the icon's top-left is at (15, 1)"
        );
        assert_eq!(
            at(38, 24),
            ink,
            "{state:?}: its bottom-right is at (38, 24)"
        );
        assert_ne!(
            at(15, 25),
            ink,
            "{state:?}: (15, 25), under the icon, is not its ink"
        );
        assert_ne!(
            at(15, 0),
            ink,
            "{state:?}: (15, 0), over the icon, is not its ink"
        );

        let p = s.part(Part::StartAbove).unwrap();
        let f = p.frame(state);
        for (x, y) in [(26, 8), (20, 12), (8, 11), (17, 7), (45, 12)] {
            assert_eq!(
                orb.get(x, y),
                p.atlas.get(f.x + x, f.y + y),
                "{state:?}: ({x}, {y}) is the part's pixel there"
            );
        }
    }

    let full = crate::glass::style(&Default::default());
    let l = TaskbarLayout::compute(
        &theme.metrics,
        &full.taskbar,
        1920,
        true,
        TaskbarCounts::default(),
        60,
    );
    assert_eq!(
        l.start_above,
        Some(Rect::new(0, -6, 54, 6)),
        "54x6 above the 40 px bar"
    );
    for (hot, pressed, lit) in [
        (false, false, false),
        (true, false, true),
        (false, true, true),
    ] {
        let mut st = state_with_tasks(&[]);
        st.start_hot = hot;
        st.start_pressed = pressed;
        let r = l.start_orb(pressed).unwrap();
        let mut orb = Canvas::new(r.width as u32, r.height as u32);
        draw_start_above(&mut orb, &full, &l, &st, 1);
        assert_eq!(
            orb.get(17, 4) >> 24 != 0,
            lit,
            "hot {hot}, held {pressed}: a glow at (17, -2) is {lit}"
        );
    }
}

#[test]
fn under_a_top_bar_the_small_orb_hangs_upright_with_its_glow_below() {
    let mut s = (*crate::glass::small_bar_style(&Default::default())).clone();
    s.start_bloom = None;
    let theme = theme_for(Interface::Glass);
    let metrics = crate::theme::Metrics {
        taskbar_height: crate::glass::SMALL_TASKBAR_HEIGHT,
        ..theme.metrics.clone()
    };
    let layout = TaskbarLayout::compute(
        &metrics,
        &s.taskbar,
        1920,
        true,
        TaskbarCounts::default(),
        60,
    )
    .on_edge(Edge::Top);
    let magenta = crate::theme::Color::rgb(0xFF, 0, 0xFF);
    let icon = solid_icon(magenta, 24);
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let button = s.part(Part::StartButton).unwrap();
    let above = s.part(Part::StartAbove).unwrap();
    for (hot, pressed, state) in [
        (false, false, State::Normal),
        (true, false, State::Hot),
        (false, true, State::Pressed),
    ] {
        let mut st = state_with_tasks(&[]);
        st.start_icon = Some(icon.clone().into());
        st.start_hot = hot;
        st.start_pressed = pressed;
        let mut bar = Canvas::new(1920, 30);
        draw_taskbar(&mut bar, &s, &theme, &text, &start, &layout, &st);
        let r = layout.start_orb(pressed).unwrap();
        let mut orb = Canvas::new(r.width as u32, r.height as u32);
        draw_start_above(&mut orb, &s, &layout, &st, 1);
        let mut c = Canvas::new(1920, 43);
        c.blit_over(0, 0, &bar);
        c.blit_over(r.x, r.y, &orb);

        let mut want = Canvas::new(1920, 43);
        let full = Rect::new(0, 0, 1920, 30);
        want.with_rows_flipped(full, |w| s.draw(w, full, Part::Taskbar, State::Normal, 1));
        s.draw(
            &mut want,
            Rect::new(0, -8, 54, 13),
            Part::StartAbove,
            state,
            1,
        );
        s.draw(
            &mut want,
            Rect::new(0, 5, 54, 30),
            Part::StartButton,
            state,
            1,
        );
        want.blit_canvas(15, 6, &icon);
        let (bf, af) = (button.frame(state), above.frame(state));
        for x in 0..54 {
            for y in 0..35 {
                assert_eq!(
                    c.get(x, y),
                    want.get(x, y),
                    "{state:?}: ({x}, {y}) is the orb drawn upright 5 rows lower"
                );
            }
            for y in 30..35 {
                assert_eq!(
                    orb.get(x, y - r.y),
                    button.atlas.get(bf.x + x, bf.y + y - 5),
                    "{state:?}: ({x}, {y}) is the button's row {}",
                    y - 5
                );
            }
            for y in 35..43 {
                assert_eq!(
                    orb.get(x, y - r.y),
                    above.atlas.get(af.x + x, af.y + 42 - y),
                    "{state:?}: ({x}, {y}) is the row {} above turned over",
                    42 - y
                );
            }
        }
        let ink = magenta.argb();
        assert_eq!(
            c.get(15, 6),
            ink,
            "{state:?}: the icon's top-left at (15, 6)"
        );
        assert_eq!(
            c.get(38, 29),
            ink,
            "{state:?}: its bottom-right at (38, 29)"
        );
        assert_ne!(c.get(15, 5), ink, "{state:?}: (15, 5) is over the icon");
        assert_ne!(c.get(15, 30), ink, "{state:?}: (15, 30) is under the icon");
    }

    let mut full = (*crate::glass::style(&Default::default())).clone();
    full.start_bloom = None;
    let l = TaskbarLayout::compute(
        &theme.metrics,
        &full.taskbar,
        1920,
        true,
        TaskbarCounts::default(),
        60,
    )
    .on_edge(Edge::Top);
    let above = full.part(Part::StartAbove).unwrap();
    for (hot, pressed, state) in [
        (false, false, State::Normal),
        (true, false, State::Hot),
        (false, true, State::Pressed),
    ] {
        let mut st = state_with_tasks(&[]);
        st.start_hot = hot;
        st.start_pressed = pressed;
        let r = l.start_orb(pressed).unwrap();
        let mut orb = Canvas::new(r.width as u32, r.height as u32);
        draw_start_above(&mut orb, &full, &l, &st, 1);
        let f = above.frame(state);
        for x in 0..54 {
            for y in 40..46 {
                assert_eq!(
                    orb.get(x, y - r.y),
                    above.atlas.get(f.x + x, f.y + 45 - y),
                    "{state:?}: ({x}, {y}) is the row {} above turned over",
                    45 - y
                );
            }
        }
    }
}
