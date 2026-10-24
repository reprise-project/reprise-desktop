use super::*;
use crate::dialog::welcome::WELCOME_TITLE;

#[test]
fn log_off_and_turn_off_answer_enter_and_access_keys() {
    let mut d = Dialog::log_off();
    assert_eq!(d.key(Key::Enter), Outcome::Act(Action::Exit));
    assert_eq!(
        d.key(Key::Edit(EditKey::Insert("l".into()))),
        Outcome::Act(Action::Exit)
    );
    assert_eq!(
        d.key(Key::Edit(EditKey::Insert("z".into()))),
        Outcome::Nothing
    );
    let mut t = Dialog::turn_off(&power());
    assert_eq!(
        t.key(Key::Enter),
        Outcome::EndSession(Goal::TurnOff, "systemctl poweroff".into())
    );
    assert_eq!(
        t.key(Key::Edit(EditKey::Insert("S".into()))),
        Outcome::StandBy("systemctl suspend".into())
    );
    assert_eq!(
        t.key(Key::Edit(EditKey::Insert("r".into()))),
        Outcome::EndSession(Goal::Restart, "systemctl reboot".into())
    );
    assert_eq!(t.press(3), Outcome::Close);
    assert_eq!(t.press(9), Outcome::Nothing);
}

#[test]
fn welcome_panels_lay_out_their_cells() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let turn_off = Dialog::turn_off(&power());
    let t = DialogLayout::compute(&theme, &text, &turn_off);
    assert_eq!(t.size, (314, 200));
    assert_eq!(t.caption, Rect::new(1, 1, 312, 43));
    assert!(t.lines.is_empty() && t.icon.is_none());
    let centres: Vec<i32> = t.buttons[..3].iter().map(|r| r.x + r.width / 2).collect();
    assert_eq!(centres, vec![71, 157, 242]);
    assert_eq!(t.buttons[0], Rect::new(29, 73, 85, 64));
    assert_eq!(t.buttons[3], Rect::new(243, 168, 60, 20));
    assert_eq!(
        t.hit(&turn_off, 156, 100, &text, theme.fonts.size_px),
        Some(DialogHit::Button(1))
    );
    assert_eq!(
        t.hit(&turn_off, 250, 175, &text, theme.fonts.size_px),
        Some(DialogHit::Button(3))
    );
    assert_eq!(t.hit(&turn_off, 5, 100, &text, theme.fonts.size_px), None);
    let l = DialogLayout::compute(&theme, &text, &Dialog::log_off());
    let centres: Vec<i32> = l.buttons[..2].iter().map(|r| r.x + r.width / 2).collect();
    assert_eq!(centres, vec![100, 215]);
    assert_eq!(l.buttons[2], t.buttons[3]);
}

#[test]
fn welcome_gradient_varies_across_and_down() {
    const PIXELS: [(i32, i32, u32); 18] = [
        (1, 46, 0x5a7dde),
        (162, 46, 0x93b2f4),
        (158, 60, 0x94b3f4),
        (157, 155, 0x84a4ee),
        (41, 150, 0x6284e1),
        (196, 62, 0x97b5f5),
        (249, 150, 0x7f9fec),
        (270, 50, 0x7b9beb),
        (310, 60, 0x698be4),
        (20, 70, 0x5d80df),
        (300, 146, 0x6b8de5),
        (100, 60, 0x7d9deb),
        (229, 143, 0x87a7ef),
        (5, 156, 0x0f40a4),
        (157, 156, 0x1444a6),
        (300, 156, 0x0f40a4),
        (157, 157, 0x013399),
        (300, 157, 0x003399),
    ];
    for d in [Dialog::log_off(), Dialog::turn_off(&power())] {
        let (c, _) = welcome_drawn(&d, None, Cues::NONE);
        for (x, y, rgb) in PIXELS {
            assert_eq!(
                c.get(x, y) & 0xff_ffff,
                rgb,
                "{} at ({x}, {y}) is {rgb:06X}",
                d.title
            );
        }
    }
}

/// Writes both panels to `target/log-off.png` and `target/turn-off.png`.
#[test]
#[ignore]
fn render_the_welcome_panels() {
    for (d, name) in [
        (Dialog::log_off(), "log-off"),
        (Dialog::turn_off(&power()), "turn-off"),
    ] {
        let (c, _) = welcome_drawn(&d, None, Cues::NONE);
        let path = format!("{}/target/{name}.png", env!("CARGO_MANIFEST_DIR"));
        std::fs::write(&path, reprisekit::raster::to_png(&c)).unwrap();
    }
}

/// At 200% text the Turn Off panel doubles: its title, cells and Cancel grow, the gradient
/// and the lines under it stretch over the grown band, and the icons keep their 32 px while
/// the labels grow under them.
#[test]
fn the_welcome_panel_grows_with_the_text_size() {
    let theme = Theme::default().with_text_size(200);
    let u = theme.text;
    let text = TextRenderer::load(&theme.fonts);
    let d = Dialog::turn_off(&power());
    let l = DialogLayout::compute(&theme, &text, &d);
    assert_eq!(l.size, (628, 400));
    assert_eq!(l.caption, u.rect(Rect::new(1, 1, 312, 43)));
    assert_eq!(l.buttons[0], u.rect(Rect::new(29, 73, 85, 64)));
    assert_eq!(l.buttons[3], u.rect(Rect::new(243, 168, 60, 20)));
    let icon = Canvas::new(ICON_PX as u32, ICON_PX as u32);
    let mut icon = icon;
    icon.fill_rect(icon.bounds(), Color::rgb(1, 2, 3));
    let c = draw(
        &theme,
        &text,
        &l,
        None,
        None,
        &[Some(icon.clone()), Some(icon.clone()), Some(icon), None],
        &[],
        None,
        &d,
        true,
        None,
        Cues::ALL,
        true,
        None,
    );
    assert_eq!((c.width as i32, c.height as i32), l.size);
    // The band runs from under the separator's two rows to the lines at 2 x 156.
    let band_top = l.caption.bottom() + 2;
    assert_eq!(c.get(3, band_top), panel_gradient(0, 0).argb());
    assert_eq!(c.get(3, 311), panel_gradient(0, 109).argb());
    assert_eq!(c.get(3, 312), PANEL_LINE_RUNS[0][0].0.argb());
    assert_eq!(c.get(3, 313), PANEL_LINE_RUNS[1][0].0.argb());
    let (bw, bh) = (l.caption.width, 312 - band_top);
    let at = panel_gradient(366 * PANEL_W / bw, 100 * PANEL_H / bh);
    assert_eq!(
        c.get(1 + 366, band_top + 100),
        at.argb(),
        "the gradient stretched"
    );
    let cell = l.buttons[2];
    let shown = (cell.y..cell.bottom())
        .filter(|&y| c.get(cell.x + cell.width / 2, y) == Color::rgb(1, 2, 3).argb())
        .count();
    assert_eq!(shown, ICON_PX as usize, "the icon keeps its 32 px");
    let white = Color::rgb(255, 255, 255).argb();
    let label_rows: Vec<i32> = (cell.y..cell.bottom())
        .filter(|&y| (cell.x..cell.right()).any(|x| c.get(x, y) == white))
        .collect();
    let (top, bottom) = (label_rows[0], *label_rows.last().unwrap());
    assert!(
        bottom - top >= 12,
        "the label's ink grows, rows {top}..={bottom}"
    );
    assert!(bottom < cell.bottom(), "and stays in its cell");
}

#[test]
fn welcome_panel_draws_bands_gradient_and_labels() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let d = Dialog::log_off();
    let l = DialogLayout::compute(&theme, &text, &d);
    let c = draw(
        &theme,
        &text,
        &l,
        None,
        None,
        &[None, None, None],
        &[],
        None,
        &d,
        true,
        None,
        Cues::ALL,
        true,
        None,
    );
    assert_eq!(c.get(0, 0), Color::rgb(0, 0, 0).argb());
    assert_eq!(c.get(5, 5), PANEL_BAND.argb());
    assert_eq!(c.get(5, 190), PANEL_BAND.argb());
    let count = |r: Rect, color: Color| {
        (r.x..r.right())
            .flat_map(|x| (r.y..r.bottom()).map(move |y| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == color.argb())
            .count()
    };
    let label = |r: Rect| Rect::new(r.x, r.y + 44, r.width, 20);
    assert!(count(label(l.buttons[0]), PANEL_DISABLED) > 10);
    assert_eq!(count(label(l.buttons[0]), Color::rgb(255, 255, 255)), 0);
    assert!(count(label(l.buttons[1]), Color::rgb(255, 255, 255)) > 10);
    let cancel = l.buttons[2];
    assert_eq!(
        c.get(cancel.x, cancel.y + 9),
        theme.colors.button_hilight.argb()
    );
    assert_eq!(
        c.get(cancel.x + 5, cancel.y + 3),
        theme.colors.button_face.argb()
    );
}

#[test]
fn the_welcome_separator_fades_toward_its_ends() {
    for runs in SEPARATOR_RUNS {
        let width: i32 = runs.iter().map(|&(_, n)| n as i32).sum();
        assert_eq!(width, WELCOME_W - 2 * WELCOME_BORDER);
    }
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let d = Dialog::turn_off(&power());
    let l = DialogLayout::compute(&theme, &text, &d);
    let c = draw(
        &theme,
        &text,
        &l,
        None,
        None,
        &[None, None, None],
        &[],
        None,
        &d,
        true,
        None,
        Cues::NONE,
        true,
        None,
    );
    let rgb = |x, y| c.get(x, y) & 0x00ff_ffff;
    assert_eq!(rgb(5, 43), 0x003399, "the title band above");
    for (x, want) in [
        (1, 0x3c64b6),
        (10, 0x4a6fba),
        (60, 0x9db5df),
        (113, 0xd5e4f8),
        (114, 0xd8e6fa),
        (154, 0xd8e6fa),
        (155, 0xd5e4f8),
        (230, 0xbccfed),
        (304, 0x4a6fba),
        (305, 0x3c64b6),
        (312, 0x3c64b6),
    ] {
        assert_eq!(rgb(x, 44), want, "row 44, column {x}");
    }
    for (x, want) in [
        (1, 0x3c64b6),
        (2, 0x3053b5),
        (10, 0x3c64b6),
        (134, 0xd8e6fa),
        (311, 0x3c64b6),
        (312, 0x3053b5),
    ] {
        assert_eq!(rgb(x, 45), want, "row 45, column {x}");
    }
    assert_eq!(rgb(0, 44), 0x000000, "the border");
    assert_eq!(rgb(313, 45), 0x000000, "the border");
}

#[test]
fn the_welcome_labels_sit_under_their_icons_and_underline_with_the_cues() {
    let Some(text) = tahoma() else {
        return;
    };
    let theme = Theme::default();
    let d = Dialog::turn_off(&power());
    let l = DialogLayout::compute(&theme, &text, &d);
    let red = Color::rgb(255, 0, 0);
    let mut icon = Canvas::new(32, 32);
    icon.fill_rect(icon.bounds(), red);
    let icons = [Some(icon.clone()), Some(icon.clone()), Some(icon), None];
    let drawn = |cues: Cues| {
        draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &icons,
            &[],
            None,
            &d,
            true,
            None,
            cues,
            true,
            None,
        )
    };
    let ink = |c: &Canvas, r: Rect, hit: &dyn Fn(u32) -> bool| {
        let pts: Vec<(i32, i32)> = (r.y..r.bottom())
            .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| hit(c.get(x, y)))
            .collect();
        let xs = pts.iter().map(|p| p.0);
        let ys = pts.iter().map(|p| p.1);
        (xs.clone().min(), ys.clone().min(), xs.max(), ys.max())
    };
    let white = |p: u32| p == Color::rgb(255, 255, 255).argb();
    let label = |i: usize| {
        let r = l.buttons[i];
        Rect::new(r.x, r.y + 41, r.width, r.height - 41)
    };
    let plain = drawn(Cues::NONE);
    assert_eq!(
        ink(&plain, label(1), &white),
        (Some(133), Some(122), Some(179), Some(130)),
        "Turn Off without its underline"
    );
    assert_eq!(
        ink(&plain, label(2), &white),
        (Some(221), Some(123), Some(262), Some(130)),
        "Restart without its underline"
    );
    let cued = drawn(Cues::ALL);
    assert_eq!(
        ink(&cued, label(1), &white),
        (Some(133), Some(122), Some(179), Some(132)),
        "Turn Off with the u underlined on row 132"
    );
    let is_red = |p: u32| p == red.argb();
    let lefts: Vec<Option<i32>> = (0..3)
        .map(|i| ink(&plain, l.buttons[i], &is_red).0)
        .collect();
    assert_eq!(
        lefts,
        [Some(55), Some(141), Some(226)],
        "the icons' left columns"
    );
    assert_eq!(
        ink(&plain, l.buttons[1], &is_red).1,
        Some(81),
        "the icons' top row"
    );
    let black = |p: u32| p == theme.colors.button_text.argb();
    let cancel = l.buttons[3];
    assert_eq!(cancel, Rect::new(243, 168, 60, 20));
    for c in [&plain, &cued] {
        let (x0, y0, _, y1) = ink(c, cancel.inset(2), &black);
        assert_eq!(
            (x0, y0, y1),
            (Some(257), Some(173), Some(181)),
            "Cancel's text, never underlined"
        );
    }
}

fn welcome_drawn(d: &Dialog, lit: Lit<usize>, cues: Cues) -> (Canvas, DialogLayout) {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let mut icon = Canvas::new(32, 32);
    icon.fill_rect(icon.bounds(), Color::rgb(138, 82, 2));
    let icons: Vec<_> = d
        .buttons
        .iter()
        .map(|b| b.icon.is_some().then(|| icon.clone()))
        .collect();
    let l = DialogLayout::compute(&theme, &text, d);
    let c = draw(
        &theme,
        &text,
        &l,
        None,
        None,
        &icons,
        &[],
        None,
        d,
        true,
        lit,
        cues,
        true,
        None,
    );
    (c, l)
}

#[test]
fn under_glass_the_welcome_panels_are_framed_dialogs() {
    let theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
    let text = TextRenderer::load(&theme.fonts);
    let orange = Color::rgb(138, 82, 2);
    let mut icon = Canvas::new(32, 32);
    icon.fill_rect(icon.bounds(), orange);
    for (d, icons) in [(Dialog::log_off(), 2), (Dialog::turn_off(&power()), 3)] {
        let l = DialogLayout::compute(&theme, &text, &d);
        let g = l.frame.as_ref().expect("a frame");
        let client = g.client;
        assert_eq!(client.width, GLASS_WELCOME_W, "{}", d.title);
        let row = &l.buttons[..icons];
        for r in row {
            assert_eq!((r.width, r.height), GLASS_WELCOME_CELL);
            assert_eq!(r.y, client.y + GLASS_WELCOME_TOP);
        }
        let (first, last) = (row[0], row[icons - 1]);
        assert_eq!(
            first.x - client.x,
            client.right() - last.right(),
            "{}: the row is centred",
            d.title
        );
        let cancel = l.buttons[icons];
        assert_eq!((cancel.width, cancel.height), (BUTTON_W, BUTTON_H));
        assert_eq!(client.right() - cancel.right(), GLASS_WELCOME_MARGIN);
        assert_eq!(client.bottom() - cancel.bottom(), GLASS_WELCOME_MARGIN);
        assert_eq!(cancel.y - last.bottom(), GLASS_WELCOME_TOP);
        assert!(
            DialogLayout::compute(&Theme::default(), &text, &d)
                .frame
                .is_none(),
            "Classic keeps the panel"
        );
        let pics: Vec<_> = d
            .buttons
            .iter()
            .map(|b| b.icon.is_some().then(|| icon.clone()))
            .collect();
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &pics,
            &[],
            None,
            &d,
            true,
            None,
            Cues::NONE,
            true,
            None,
        );
        for (i, r) in row.iter().enumerate() {
            let mid = c.get(r.x + r.width / 2, r.y + WELCOME_ICON_Y + ICON_PX / 2);
            if d.buttons[i].enabled {
                assert_eq!(mid, orange.argb(), "{}: icon {i}", d.title);
            } else {
                assert_ne!(mid, orange.argb(), "{}: icon {i} is faded", d.title);
            }
        }
        let st = theme.look.style().unwrap();
        let frame = |state| {
            let mut f = Canvas::new(GLASS_WELCOME_CELL.0 as u32, GLASS_WELCOME_CELL.1 as u32);
            f.fill_rect(f.bounds(), theme.colors.button_face);
            let r = Rect::new(0, 0, GLASS_WELCOME_CELL.0, GLASS_WELCOME_CELL.1);
            st.draw(&mut f, r, reprisekit::style::Part::PushButton, state, 1);
            f.get(GLASS_WELCOME_CELL.0 / 2, 1)
        };
        for (i, r) in row.iter().enumerate() {
            let b = &d.buttons[i];
            let want = if !b.enabled {
                reprisekit::style::State::Disabled
            } else if b.default {
                reprisekit::style::State::Default
            } else {
                reprisekit::style::State::Normal
            };
            assert_eq!(
                c.get(r.x + r.width / 2, r.y + 1),
                frame(want),
                "{}: button {i} in its {want:?} frame",
                d.title
            );
        }
    }
}

fn welcome_icons(d: &Dialog, lit: Lit<usize>) -> Vec<u32> {
    let (c, l) = welcome_drawn(d, lit, Cues::NONE);
    let n = d.buttons.iter().filter(|b| b.icon.is_some()).count();
    l.buttons[..n]
        .iter()
        .map(|r| c.get(r.x + r.width / 2, r.y + WELCOME_ICON_Y + ICON_PX / 2))
        .collect()
}

#[test]
fn the_hovered_and_focused_welcome_icons_draw_hot() {
    let cold = Color::rgb(138, 82, 2).argb();
    let hot = Color::rgb(163, 97, 2).argb();
    let log_off = Dialog::log_off();
    let faded = welcome_icons(&log_off, None)[0];
    assert_ne!(faded, cold);
    assert_eq!(
        welcome_icons(&log_off, None),
        [faded, hot],
        "Log Off opens with the focus"
    );
    assert_eq!(
        welcome_icons(&log_off, Some((0, Interaction::Hot))),
        [faded, hot],
        "the greyed Switch User stays faded under the pointer"
    );
    let turn_off = Dialog::turn_off(&power());
    assert_eq!(
        welcome_icons(&turn_off, None),
        [cold, hot, cold],
        "Turn Off opens with the focus"
    );
    assert_eq!(
        welcome_icons(&turn_off, Some((2, Interaction::Hot))),
        [cold, hot, hot],
        "Restart under the pointer, Turn Off still focused"
    );
    let mut d = Dialog::turn_off(&power());
    d.key(Key::Right);
    assert_eq!(
        welcome_icons(&d, None),
        [cold, cold, hot],
        "Restart focused"
    );
    assert_eq!(
        welcome_icons(&d, Some((1, Interaction::Hot))),
        [cold, hot, hot],
        "Turn Off under the pointer, Restart still focused"
    );
    d.key(Key::Right);
    assert_eq!(
        welcome_icons(&d, None),
        [cold, cold, cold],
        "Cancel focused"
    );
    assert_eq!(
        welcome_icons(&d, Some((0, Interaction::Hot))),
        [hot, cold, cold],
        "Stand By under the pointer, Cancel focused"
    );
    let mut d = Dialog::turn_off(&power());
    assert!(d.hover(Some(DialogHit::Button(2)), (0, 0), false));
    assert_eq!(d.lit(), Some((2, Interaction::Hot)));
    assert!(d.leave(false));
    assert_eq!(d.buttons.iter().position(|b| b.default), Some(1));
    assert!(!Dialog::run(&[]).hover(Some(DialogHit::Button(0)), (0, 0), false));
}

#[test]
fn keys_walk_a_welcome_panels_focus_and_cancel_shows_it() {
    let focused = |d: &Dialog| d.buttons.iter().position(|b| b.default);
    let mut d = Dialog::log_off();
    assert_eq!(focused(&d), Some(1), "Log Off holds the focus");
    assert_eq!(d.key(Key::Left), Outcome::Nothing, "Switch User is greyed");
    assert_eq!(d.key(Key::Right), Outcome::Redraw);
    assert_eq!(focused(&d), Some(2), "Right goes on to Cancel");
    assert_eq!(d.key(Key::Right), Outcome::Nothing, "and stops there");
    assert_eq!(d.key(Key::Tab { back: false }), Outcome::Redraw);
    assert_eq!(
        focused(&d),
        Some(1),
        "Tab wraps past the greyed Switch User"
    );

    let mut d = Dialog::turn_off(&power());
    let order: Vec<_> = (0..4)
        .map(|_| {
            d.key(Key::Tab { back: false });
            focused(&d)
        })
        .collect();
    assert_eq!(order, [Some(2), Some(3), Some(0), Some(1)]);
    d.key(Key::Left);
    assert_eq!(focused(&d), Some(0), "Left from Turn Off to Stand By");

    let cancel = |d: &Dialog, cues: Cues| {
        let (c, l) = welcome_drawn(d, None, cues);
        let r = l.buttons[3];
        (c.get(r.x, r.y + r.height / 2), c.get(r.x + 4, r.y + 5))
    };
    let black = Color::rgb(0, 0, 0).argb();
    let face = Theme::default().colors.button_face.argb();
    let d = Dialog::turn_off(&power());
    assert_ne!(cancel(&d, Cues::ALL).0, black, "Cancel unfocused");
    assert_eq!(cancel(&d, Cues::ALL).1, face);
    let mut d = Dialog::turn_off(&power());
    d.key(Key::Tab { back: true });
    d.key(Key::Tab { back: true });
    assert_eq!(focused(&d), Some(3), "Shift+Tab wraps round to Cancel");
    assert_eq!(cancel(&d, Cues::NONE), (black, face), "focused, no cues");
    assert_eq!(cancel(&d, Cues::ALL), (black, black), "focused, with cues");
}

#[test]
fn the_welcome_title_asks_for_franklin_gothic_medium_then_trebuchet_ms() {
    let spec = welcome_title_text();
    assert_eq!(
        spec.family[..2],
        ["Franklin Gothic Medium", "Trebuchet MS"],
        "Franklin Gothic Medium first, Trebuchet MS as its fallback"
    );
    assert_eq!(spec.family_sizes, [("Franklin Gothic Medium".into(), 20)]);
    assert_eq!(spec.size_px, 18, "the fallback's size");
    assert!(spec.bold && !spec.italic);
}

#[test]
fn the_welcome_title_sets_in_franklin_gothic_medium_at_20_px() {
    let installed = WELCOME_TITLE.with(|t| t.has_face("Franklin Gothic Medium"));
    let Some(text) = tahoma().filter(|_| installed) else {
        return;
    };
    let theme = Theme::default();
    let d = Dialog::turn_off(&power());
    let l = DialogLayout::compute(&theme, &text, &d);
    let c = draw(
        &theme,
        &text,
        &l,
        None,
        None,
        &[None, None, None, None],
        &[],
        None,
        &d,
        true,
        None,
        Cues::ALL,
        true,
        None,
    );
    let band = PANEL_BAND.argb();
    let ink: Vec<(i32, i32)> = (1..44)
        .flat_map(|y| (1..250).map(move |x| (x, y)))
        .filter(|&(x, y)| c.get(x, y) != band)
        .collect();
    let xs = ink.iter().map(|p| p.0);
    assert_eq!((xs.clone().min(), xs.max()), (Some(13), Some(164)));
    // The T's crossbar is the title's first full row, and the last row
    // with more than the p's descender is the baseline's.
    let row = |y: i32| ink.iter().filter(|p| p.1 == y).count();
    let rows: Vec<i32> = (1..44).filter(|&y| row(y) > 4).collect();
    assert_eq!((rows.first(), rows.last()), (Some(&15), Some(&28)));
}

#[test]
fn welcome_buttons_tip_their_description_but_not_when_greyed() {
    let d = Dialog::turn_off(&power());
    let (title, body) = d.tip(1).unwrap();
    assert_eq!(title, "Turn Off");
    assert!(body.starts_with("Shuts down Reprise"));
    assert!(d.tip(3).is_none(), "Cancel has no tip");
    let l = Dialog::log_off();
    assert!(l.tip(0).is_none(), "greyed Switch User has none");
    assert!(l.tip(1).is_some());
    assert!(Dialog::run(&[]).tip(0).is_none());
}
