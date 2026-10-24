use super::*;

#[test]
fn classic_dialog_has_a_close_button_and_a_focus_ring() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let d = Dialog::confirm_delete("x", std::path::Path::new("/x"), false, false, false);
    let l = DialogLayout::compute(&theme, &text, &d);
    let close = l.close.unwrap();
    assert_eq!(
        close,
        Rect::new(l.caption.right() - 18, l.caption.y + 2, 16, 14)
    );
    assert_eq!(
        l.hit(&d, close.x + 5, close.y + 5, &text, theme.fonts.size_px),
        Some(DialogHit::Close)
    );
    assert_eq!(
        l.hit(
            &d,
            l.caption.x + 20,
            l.caption.y + 5,
            &text,
            theme.fonts.size_px
        ),
        Some(DialogHit::Caption)
    );
    assert_eq!(
        l.hit(
            &d,
            l.caption.x + 20,
            l.caption.bottom() + 4,
            &text,
            theme.fonts.size_px
        ),
        None,
        "below the caption is the body, not a grip"
    );
    assert!(l.help.is_none(), "only Run has the ? button");
    let c = draw(
        &theme,
        &text,
        &l,
        None,
        None,
        &[],
        &[],
        None,
        &d,
        true,
        None,
        Cues::ALL,
        true,
        None,
    );
    let cs = &theme.colors;
    assert_eq!(c.get(close.x, close.y + 7), cs.button_hilight.argb());
    assert_eq!(c.get(close.x + 7, close.y + 6), cs.button_text.argb());
    let yes = l.buttons[0].inset(4);
    assert_ne!(c.get(yes.x, yes.y), cs.button_text.argb());
    assert_eq!(c.get(yes.x + 1, yes.y), cs.button_text.argb());
    assert_ne!(c.get(yes.x + 2, yes.y), cs.button_text.argb());
    let no = l.buttons[1].inset(4);
    assert_ne!(c.get(no.x, no.y), cs.button_text.argb());
    let run = Dialog::run(&[]);
    let rl = DialogLayout::compute(&theme, &text, &run);
    let rc = draw(
        &theme,
        &text,
        &rl,
        None,
        None,
        &[],
        &[],
        None,
        &run,
        true,
        None,
        Cues::ALL,
        true,
        None,
    );
    let ok = rl.buttons[0].inset(4);
    assert_ne!(rc.get(ok.x + 1, ok.y), cs.button_text.argb());
    assert!(
        DialogLayout::compute(&theme, &text, &Dialog::log_off())
            .close
            .is_none()
    );
}

#[test]
fn a_classic_button_lights_under_the_pointer_and_a_leave_clears_it() {
    let mut d = Dialog::log_off();
    let on = Some(DialogHit::Button(1));
    assert!(d.hover(on, (0, 0), true));
    assert_eq!(d.lit(), Some((1, Interaction::Hot)));
    assert!(!d.hover(on, (0, 0), true), "still on it");
    assert!(
        d.hover(Some(DialogHit::Button(0)), (0, 0), true),
        "onto a greyed one"
    );
    assert_eq!(d.lit(), None, "a greyed button does not light");
    d.hover(on, (0, 0), true);
    assert!(d.leave(true));
    assert_eq!(d.lit(), None);
    let mut m = Dialog::end_program("foot", 4321);
    assert!(
        !m.hover(Some(DialogHit::Button(0)), (0, 0), false),
        "a Classic message box has nothing to redraw"
    );
}

#[test]
fn the_help_box_lights_hot_and_shows_the_press_until_the_release() {
    let mut d = Dialog::log_off();
    let help = Some(DialogHit::Help);
    assert!(d.hover(help, (0, 0), true));
    assert_eq!(d.help_box.of(()), Interaction::Hot);
    d.press_help();
    assert_eq!(d.help_box.of(()), Interaction::Pressed);
    assert!(d.drag_help(false), "dragged off");
    assert_eq!(d.help_box.of(()), Interaction::Idle);
    assert!(d.drag_help(true), "back on");
    assert!(d.release_help());
    assert_eq!(d.help_box.of(()), Interaction::Hot);
    assert!(d.leave(true));
    assert_eq!(d.help_box.of(()), Interaction::Idle);
}

#[test]
fn a_blue_dialog_caption_shows_its_caption_icon_as_given() {
    let blue = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
    let text = TextRenderer::load(&blue.fonts);
    let d = crate::binprops::dialog(
        crate::binprops::BinSheet::new(
            Default::default(),
            vec![crate::binprops::Drive {
                key: "home".into(),
                label: "Local Disk (/home)".into(),
                size: 21_365_735_424,
            }],
        ),
        (0, 0),
        "user-trash",
    );
    let l = DialogLayout::compute(&blue, &text, &d);
    let mut big = Canvas::new(32, 32);
    big.fill_rect(big.bounds(), Color::rgb(0x00, 0xFF, 0x00));
    let mut small = Canvas::new(16, 16);
    small.fill_rect(small.bounds(), Color::rgb(0xFF, 0x00, 0xFF));
    let c = draw(
        &blue,
        &text,
        &l,
        Some(&big),
        Some(&small),
        &[],
        &[],
        None,
        &d,
        false,
        Lit::default(),
        Cues::NONE,
        true,
        None,
    );
    let caption = l.frame.expect("a styled dialog has a frame").caption;
    let count = |argb: u32| {
        (caption.y..caption.bottom())
            .flat_map(|y| (caption.x..caption.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == argb)
            .count()
    };
    assert_eq!(count(0xFFFF_00FF), 256, "the 16x16 caption icon, unscaled");
    assert_eq!(count(0xFF00_FF00), 0, "no trace of the 32 px icon");
}

#[test]
fn a_blue_form_dialog_lights_its_tab_and_control_and_a_leave_clears_them() {
    use reprisekit::controls::form::Light;
    let blue = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
    let text = TextRenderer::load(&blue.fonts);
    let px = blue.fonts.size_px;
    let mut d = crate::binprops::dialog(
        crate::binprops::BinSheet::new(
            Default::default(),
            vec![crate::binprops::Drive {
                key: "home".into(),
                label: "Local Disk (/home)".into(),
                size: 21_365_735_424,
            }],
        ),
        (0, 0),
        "user-trash",
    );
    let l = DialogLayout::compute(&blue, &text, &d);
    draw(
        &blue,
        &text,
        &l,
        None,
        None,
        &[],
        &[],
        None,
        &d,
        false,
        None,
        Cues::NONE,
        true,
        None,
    );
    assert!(d.hover(Some(DialogHit::Tab(1)), (0, 0), true));
    let tabs = |d: &Dialog| d.form().unwrap().sheet().unwrap().tabs.hot;
    assert_eq!(tabs(&d), Some(1));
    let page = d.form().unwrap().open_page().unwrap();
    let ok = page.index_of("ok").unwrap();
    let r = page.rect("ok").unwrap();
    let (x, y) = (r.x + 5, r.y + 5);
    let over = l.hit(&d, x, y, &text, px);
    assert!(matches!(over, Some(DialogHit::Control(_))));
    assert!(d.hover(over, (x, y), true));
    let lit = |d: &Dialog| d.form().unwrap().open_page().unwrap().lit(ok, Light::Body);
    assert_eq!(lit(&d), Interaction::Hot);
    assert_eq!(tabs(&d), None, "off the tab");
    assert!(d.leave(true));
    assert_eq!(lit(&d), Interaction::Idle);
}

#[test]
fn a_button_acts_on_the_release_over_it() {
    let mut d = Dialog::log_off();
    assert!(!d.press_button(0), "disabled");
    assert_eq!(d.lit(), None);
    assert!(d.press_button(1));
    assert_eq!(d.lit(), Some((1, Interaction::Pressed)));
    assert!(d.drag_button(None), "left the button");
    assert_eq!(d.lit(), None);
    assert!(!d.drag_button(None), "still off it");
    assert_eq!(d.release_button(None), Some(Outcome::Redraw));
    assert_eq!(d.release_button(None), None, "nothing held");
    assert!(d.press_button(1));
    assert!(d.drag_button(Some(2)));
    assert!(d.drag_button(Some(1)));
    assert_eq!(d.release_button(Some(1)), Some(Outcome::Act(Action::Exit)));
}

#[test]
fn a_cancelled_press_does_not_act_on_the_release() {
    let mut d = Dialog::log_off();
    assert!(d.press_button(1));
    d.press_help();
    assert_eq!(d.cancel_press(), Outcome::Redraw);
    assert_eq!(d.lit(), None, "drawn released");
    assert!(!d.help_box.holding(), "the ? box lets go too");
    assert_eq!(d.release_button(Some(1)), None, "nothing held");
    assert_eq!(d.release(None), Outcome::Nothing);
}

#[test]
fn an_inactive_dialog_has_a_grey_caption_and_no_default_ring() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let d = Dialog::run(&[]);
    let l = DialogLayout::compute(&theme, &text, &d);
    let c = draw(
        &theme,
        &text,
        &l,
        None,
        None,
        &[],
        &[],
        None,
        &d,
        false,
        None,
        Cues::ALL,
        false,
        None,
    );
    assert_eq!(
        c.get(l.caption.x, l.caption.bottom() - 2),
        Color::rgb(128, 128, 128).argb()
    );
    let ok = l.buttons[0];
    assert_eq!(
        c.get(ok.x, ok.y + ok.height / 2),
        theme.colors.button_hilight.argb(),
        "OK's left edge is the hilight, not the black ring"
    );
}

/// Under Glass a form dialog's default button draws as a plain one
/// while the dialog is inactive: a 707070 top line, not 3C7EB1.
#[test]
fn an_inactive_glass_form_dialog_draws_its_default_button_plain() {
    use reprisekit::controls::form::Control;
    let theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
    let text = TextRenderer::load(&FontConfig::default());
    let d = Dialog::advanced_attributes(
        Subject::File,
        None,
        &text,
        theme.fonts.size_px,
        TextUnits::ONE,
    );
    let l = DialogLayout::compute(&theme, &text, &d);
    let ok = d
        .form()
        .and_then(|f| f.open_page())
        .and_then(|form| {
            let it = form
                .items
                .iter()
                .find(|it| matches!(it.control, Control::Button { default: true, .. }))?;
            form.rect(it.id)
        })
        .expect("a default button");
    let top = |active| {
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            false,
            None,
            Cues::NONE,
            active,
            None,
        );
        c.get(ok.x + 10, ok.y + 1)
    };
    assert_eq!(top(true), 0xFF3B_76AD, "active, the default frame");
    assert_eq!(top(false), 0xFF6D_6D6D, "inactive, a plain button");
}

#[test]
fn a_fixed_dialogs_border_sizes_nothing() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let l = DialogLayout::compute(&theme, &text, &chooser());
    assert_eq!(l.sizing_edges(1, 100, &theme.metrics), None);
    assert_eq!(l.sizing_edges(1, 1, &theme.metrics), None);
}

#[test]
fn a_blue_dialog_is_framed_by_the_style() {
    use reprisekit::style::{Part, State};
    let classic = Theme::default();
    let blue = blue();
    let text = TextRenderer::load(&FontConfig::default());
    let px = blue.fonts.size_px;
    let d = Dialog::confirm_delete("x", std::path::Path::new("/x"), false, false, false);
    let cl = DialogLayout::compute(&classic, &text, &d);
    let bl = DialogLayout::compute(&blue, &text, &d);
    assert_eq!(bl.caption, Rect::new(3, 3, cl.caption.width, 26));
    assert_eq!(bl.size, (cl.size.0, cl.size.1 + 8));
    let down = |r: Rect| Rect::new(r.x, r.y + 8, r.width, r.height);
    assert_eq!(bl.icon, cl.icon.map(down));
    assert_eq!(
        bl.buttons,
        cl.buttons.iter().copied().map(down).collect::<Vec<_>>()
    );
    assert_eq!(bl.lines[0].0, down(cl.lines[0].0));
    let close = bl.close.unwrap();
    assert_eq!(close, Rect::new(bl.caption.right() - 23, 5, 21, 21));
    assert_eq!(
        bl.hit(&d, close.x + 10, close.y + 10, &text, px),
        Some(DialogHit::Close)
    );
    assert_eq!(
        bl.hit(&d, 40, bl.caption.bottom() - 1, &text, px),
        Some(DialogHit::Caption)
    );

    let style = blue.look.style().unwrap();
    let top = Rect::new(0, 0, bl.size.0, bl.caption.bottom());
    for (active, state) in [(true, State::Normal), (false, State::Inactive)] {
        let c = draw(
            &blue,
            &text,
            &bl,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            false,
            None,
            Cues::NONE,
            active,
            None,
        );
        let mut want = Canvas::new(bl.size.0 as u32, top.height as u32);
        style.draw(&mut want, top, Part::Caption, state, 1);
        assert_eq!(c.get(12, 2), want.get(12, 2), "active {active}");
        assert_eq!(c.get(22, 15), want.get(22, 15), "active {active}");
        assert_eq!(
            c.get(20, bl.caption.bottom() + 2),
            blue.colors.button_face.argb(),
            "the body is button face"
        );
    }
}

#[test]
fn a_glass_dialog_wears_the_window_frame_with_a_lone_close_box() {
    let classic = Theme::default();
    let glass = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
    let text = TextRenderer::load(&FontConfig::default());
    let px = glass.fonts.size_px;
    let d = Dialog::confirm_delete("x", std::path::Path::new("/x"), false, false, false);
    let cl = DialogLayout::compute(&classic, &text, &d);
    let gl = DialogLayout::compute(&glass, &text, &d);
    let (w, h) = gl.size;
    assert_eq!((w, h), (cl.size.0 + 10, cl.size.1 + 14));
    let f = gl.frame.unwrap();
    assert_eq!(f.frame, 8);
    assert_eq!(f.client, Rect::new(8, 30, w - 16, h - 38));
    assert_eq!(gl.close, Some(Rect::new(w - 55, 1, 49, 20)));
    assert_eq!(f.buttons[2], Rect::new(w - 55, 1, 49, 20));
    let down = |r: Rect| Rect::new(r.x + 5, r.y + 9, r.width, r.height);
    assert_eq!(gl.icon, cl.icon.map(down));
    assert_eq!(
        gl.hit(&d, w - 30, 10, &text, px),
        Some(DialogHit::Close),
        "the close box"
    );
    assert_eq!(
        gl.hit(&d, w - 70, 10, &text, px),
        Some(DialogHit::Caption),
        "left of the close box is caption"
    );
    let c = draw(
        &glass,
        &text,
        &gl,
        None,
        None,
        &[],
        &[],
        None,
        &d,
        false,
        None,
        Cues::NONE,
        true,
        None,
    );
    for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
        assert_eq!(c.get(x, y) >> 24, 0, "corner ({x}, {y}) is clear");
    }
    let band = c.get(w / 2, 15) >> 24;
    assert!(band > 0 && band < 0xFF, "the top band is glass: {band:#x}");
    assert!(
        c.get(0, 15) >> 24 > band,
        "the outline is denser than glass"
    );
    assert_eq!(
        c.get(20, f.client.y + 2),
        glass.colors.button_face.argb(),
        "the body is opaque button face"
    );
}

#[test]
fn a_blue_dialogs_close_box_draws_hot() {
    use reprisekit::style::{Part, State};
    let blue = blue();
    let text = TextRenderer::load(&FontConfig::default());
    let mut d = Dialog::confirm_delete("x", std::path::Path::new("/x"), false, false, false);
    d.caption_hot = Some(CaptionButton::Close);
    let l = DialogLayout::compute(&blue, &text, &d);
    let c = draw(
        &blue,
        &text,
        &l,
        None,
        None,
        &[],
        &[],
        None,
        &d,
        false,
        None,
        Cues::NONE,
        true,
        None,
    );
    let r = l.close.unwrap();
    let mut want = Canvas::new(l.size.0 as u32, l.size.1 as u32);
    let style = blue.look.style().unwrap();
    style.draw(&mut want, r, Part::CloseButton, State::Hot, 1);
    assert_eq!(c.get(r.x + 2, r.y + 3), want.get(r.x + 2, r.y + 3));
    let mut normal = Canvas::new(l.size.0 as u32, l.size.1 as u32);
    style.draw(&mut normal, r, Part::CloseButton, State::Normal, 1);
    assert_ne!(want.get(r.x + 2, r.y + 3), normal.get(r.x + 2, r.y + 3));
}

#[test]
fn the_close_box_closes_only_on_a_release_over_it() {
    let mut d = Dialog::log_off();
    d.press_caption(CaptionButton::Close);
    assert_eq!(d.caption_box.of(CaptionButton::Close), Interaction::Pressed);
    assert!(d.drag_caption(None), "dragged off");
    assert_eq!(d.caption_box.of(CaptionButton::Close), Interaction::Idle);
    assert!(
        d.release_caption().is_none(),
        "a release off the box does not close"
    );
    assert!(!d.caption_box.holding());
    d.press_caption(CaptionButton::Close);
    assert!(!d.drag_caption(Some(DialogHit::Close)), "still on it");
    assert!(d.release_caption().is_some(), "a release on the box closes");
    d.press_caption(CaptionButton::Close);
    d.cancel_press();
    assert!(
        !d.caption_box.holding(),
        "a taken pointer lets go of the close box"
    );
}

#[test]
fn a_held_close_box_draws_pushed_only_while_the_pointer_is_on_it() {
    let text = TextRenderer::load(&FontConfig::default());
    let classic = Theme::default();
    let glass = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
    for theme in [&classic, &blue(), &glass] {
        let mut d = Dialog::confirm_delete("x", std::path::Path::new("/x"), false, false, false);
        d.caption_hot = Some(CaptionButton::Close);
        let l = DialogLayout::compute(theme, &text, &d);
        let shot = |d: &Dialog| {
            draw(
                theme,
                &text,
                &l,
                None,
                None,
                &[],
                &[],
                None,
                d,
                true,
                None,
                Cues::NONE,
                true,
                None,
            )
        };
        let r = l.close.unwrap();
        let pixels = |c: &Canvas| -> Vec<u32> {
            (r.y..r.bottom())
                .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
                .map(|(x, y)| c.get(x, y))
                .collect()
        };
        let plain = {
            let mut cold = d.clone();
            cold.caption_hot = None;
            pixels(&shot(&cold))
        };
        let hot = pixels(&shot(&d));
        d.press_caption(CaptionButton::Close);
        let held = pixels(&shot(&d));
        assert_ne!(
            held, hot,
            "{:?}: the held close box draws pushed",
            theme.look
        );
        d.drag_caption(None);
        assert_eq!(
            pixels(&shot(&d)),
            plain,
            "{:?}: dragged off, it draws plain",
            theme.look
        );
    }
    let mut d = Dialog::confirm_delete("x", std::path::Path::new("/x"), false, false, false);
    let l = DialogLayout::compute(&classic, &text, &d);
    d.press_caption(CaptionButton::Close);
    let c = draw(
        &classic,
        &text,
        &l,
        None,
        None,
        &[],
        &[],
        None,
        &d,
        true,
        None,
        Cues::NONE,
        true,
        None,
    );
    let r = l.close.unwrap();
    let cs = &classic.colors;
    assert_eq!(
        c.get(r.x, r.y),
        cs.button_shadow.argb(),
        "sunken: shadow outer top-left"
    );
    assert_eq!(c.get(r.x + 1, r.y + 1), cs.button_dk_shadow.argb());
    assert_eq!(
        c.get(r.right() - 1, r.bottom() - 1),
        cs.button_hilight.argb()
    );
}

#[test]
fn a_form_dialog_fits_the_frame_it_is_shown_in() {
    use reprisekit::controls::filesheet::NAME;
    let classic = Theme::default();
    let blue = blue();
    let text = TextRenderer::load(&FontConfig::default());
    let px = classic.fonts.size_px;
    let mut d = Dialog::properties(&file_info(), "text-x-generic", (0, 0));
    d.fit(&classic, &text);
    let cl = DialogLayout::compute(&classic, &text, &d);
    let name = sheet_form(&d).rect(NAME).unwrap();
    d.fit(&blue, &text);
    d.fit(&blue, &text);
    let bl = DialogLayout::compute(&blue, &text, &d);
    assert_eq!(bl.size, (cl.size.0, cl.size.1 + 8));
    assert_eq!(bl.caption.height, 26);
    let page = cl.page.unwrap();
    assert_eq!(
        bl.page,
        Some(Rect::new(page.x, page.y + 8, page.width, page.height))
    );
    assert_eq!(bl.tabs[0].y, cl.tabs[0].y + 8);
    let moved = sheet_form(&d).rect(NAME).unwrap();
    assert_eq!(
        moved,
        Rect::new(name.x, name.y + 8, name.width, name.height)
    );
    assert!(matches!(
        bl.hit(&d, moved.x + 3, moved.y + 3, &text, px),
        Some(DialogHit::Control(_))
    ));
    d.fit(&classic, &text);
    assert_eq!(DialogLayout::compute(&classic, &text, &d), cl);
    assert_eq!(sheet_form(&d).rect(NAME).unwrap(), name);
}

#[test]
fn a_blue_client_area_starts_under_the_styles_caption() {
    let blue = blue();
    let text = TextRenderer::load(&FontConfig::default());
    let browse = Dialog::browse(crate::filepick::FilePicker::run_command(
        &std::env::temp_dir(),
        crate::filepick::Icons::default(),
        true,
    ));
    let l = DialogLayout::compute(&blue, &text, &browse);
    assert_eq!(l.page, Some(Rect::new(4, 30, 555, 451)));
    assert_eq!(l.size, (563, 485));
    assert_eq!(l.caption, Rect::new(4, 4, 555, 26));
    let l = DialogLayout::compute(&blue, &text, &chooser());
    assert_eq!(l.page.map(|p| (p.x, p.y)), Some((3, 29)));
    assert_eq!(l.size, (CHOOSER_W, CHOOSER_H + 8));
}

#[test]
fn the_copy_dialog_answers_its_keys_and_close_box_as_the_put_controls() {
    use reprisekit::controls::copyprogress::{CopyProgress, Kind};
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let cp = CopyProgress::new(Kind::Copy, "Downloads", "KINGSTON", Color::rgb(0, 0, 0));
    let mut d = Dialog::copy_progress(cp, "/media/KINGSTON".into());
    d.fit(&theme, &text);
    assert_eq!(d.title, "Copying...");
    assert_eq!(d.key(Key::Access('p')), Outcome::CopyPause(true));
    assert_eq!(d.key(Key::Escape), Outcome::CopyCancel);

    let layout = DialogLayout::compute(&theme, &text, &d);
    let client = layout.page.expect("a client");
    assert_eq!((client.width, client.height), d.copy().unwrap().cp.client());
    let px = theme.fonts.size_px;
    let cancel = (client.right() - 20, client.bottom() - 15);
    let hit = layout.hit(&d, cancel.0, cancel.1, &text, px);
    assert!(matches!(hit, Some(DialogHit::Control(_))), "{hit:?}");
    d.click(hit);
    assert_eq!(d.release(hit), Outcome::CopyCancel);
}

#[test]
fn a_windowed_dialog_minimizes_from_the_box_left_of_close() {
    use reprisekit::controls::copyprogress::{CopyProgress, Kind};
    let text = TextRenderer::load(&FontConfig::default());
    let glass = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
    for theme in [Theme::default(), glass] {
        let cp = CopyProgress::new(Kind::Copy, "Downloads", "KINGSTON", Color::rgb(0, 0, 0));
        let mut d = Dialog::copy_progress(cp, "/media/KINGSTON".into());
        d.fit(&theme, &text);
        let l = DialogLayout::compute(&theme, &text, &d);
        let g = l.frame.expect("a frame");
        let min = l.minimize.expect("a minimize box");
        assert_eq!(min, g.button_rect(CaptionButton::Minimize));
        assert!(min.right() <= l.close.unwrap().x + 1, "left of close");
        let px = theme.fonts.size_px;
        let hit = l.hit(&d, min.x + 2, min.y + 2, &text, px);
        assert_eq!(hit, Some(DialogHit::Minimize));
        d.press_caption(CaptionButton::Minimize);
        assert!(d.drag_caption(None), "dragged off");
        assert!(d.drag_caption(hit), "back on");
        assert_eq!(d.release_caption(), Some(CaptionButton::Minimize));

        let run = DialogLayout::compute(&theme, &text, &Dialog::run(&[]));
        assert_eq!(
            run.minimize, None,
            "a dialog over the windows has no minimize box"
        );
    }
}

#[test]
fn the_copy_dialog_is_a_windowed_task_with_the_jobs_progress() {
    use reprisekit::controls::copyprogress::{CopyProgress, Kind, Phase, Status};
    let mut cp = CopyProgress::new(Kind::Copy, "Downloads", "KINGSTON", Color::rgb(0, 0, 0));
    let st = Status {
        phase: Phase::Copying,
        paused: true,
        bytes: 25,
        total_bytes: 100,
        ..Status::default()
    };
    cp.update(st, 0);
    let d = Dialog::copy_progress(cp, "/media/KINGSTON".into());
    assert!(d.windowed());
    assert!(d.on_taskbar());
    assert_eq!(d.task_icon(), Some("system-file-manager"));
    let p = d.task_progress().expect("a copy's progress");
    assert_eq!((p.fraction, p.paused), (0.25, true));
    assert_eq!(Dialog::end_program("foot", 4321).task_progress(), None);
}
