use super::*;

#[test]
fn run_opens_by_the_start_button() {
    use reprisekit::taskbar::Edge;
    let area = Rect::new(0, 0, 1920, 1050);
    assert_eq!(by_start(area, Edge::Bottom, (347, 179)), (0, 1050 - 179));
    let area = Rect::new(0, 30, 1920, 1050);
    assert_eq!(by_start(area, Edge::Top, (347, 179)), (0, 30));
    let area = Rect::new(60, 0, 1860, 1080);
    assert_eq!(
        by_start(area, Edge::Left, (347, 179)),
        (60, 0),
        "Start tops a left bar"
    );
    let area = Rect::new(0, 0, 1860, 1080);
    assert_eq!(by_start(area, Edge::Right, (347, 179)), (1860 - 347, 0));
}

#[test]
fn run_beside_a_right_bar_stays_on_a_narrow_output() {
    use reprisekit::taskbar::Edge;
    let area = Rect::new(1920, 0, 260, 1080);
    assert_eq!(by_start(area, Edge::Right, (347, 179)), (1920, 0));
}

#[test]
fn runs_browse_keeps_3_px_off_the_screens_edge_on_starts_side() {
    use reprisekit::taskbar::Edge;
    let area = Rect::new(0, 0, 1920, 1050);
    assert_eq!(browse_by_start(area, Edge::Bottom, (400, 300)), (3, 750));
    let area = Rect::new(0, 0, 1860, 1080);
    assert_eq!(
        browse_by_start(area, Edge::Right, (400, 300)),
        (1860 - 400 - 3, 0)
    );
}

/// A command longer than the box scrolls with the caret; `draw` keeps `Field::first` in step.
#[test]
fn a_long_run_command_scrolls_instead_of_overflowing_the_field() {
    let theme = Theme::default();
    let Some(text) = tahoma() else { return };
    let px = theme.fonts.size_px;
    let mut d = Dialog::run(&[]);
    let long = "/usr/local/bin/some-very-long-command --with --plenty --of --flags";
    d.key(Key::Edit(EditKey::Insert(long.into())));
    let l = DialogLayout::compute(&theme, &text, &d);
    let _ = draw(
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

    let f = d.field.as_ref().unwrap();
    let from = f.combo.first.get();
    assert!(from > 0, "the value scrolled to follow the caret");
    let field = l.field.unwrap();
    let drop = l.drop.unwrap();
    let inner = Rect::new(
        field.x + combo::EDIT_TEXT_X,
        field.y,
        drop.x - 2 - (field.x + combo::EDIT_TEXT_X),
        field.height,
    );
    assert!(text.measure(&f.edit().unwrap().text[from..], false, px) <= inner.width);
    assert!(
        text.measure(
            &f.edit().unwrap().text[from..f.edit().unwrap().caret()],
            false,
            px
        ) <= inner.width
    );

    d.key(Key::Edit(crate::textedit::EditKey::Home { select: false }));
    let _ = draw(
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
    assert_eq!(
        d.field.as_ref().unwrap().combo.first.get(),
        0,
        "the caret at the start shows the start of the command"
    );
}

#[test]
fn run_drop_button_has_the_combo_bevel() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let Some(text) = tahoma() else { return };
    let d = Dialog::run(&history());
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
        Cues::NONE,
        true,
        None,
    );
    let r = l.drop.expect("a history draws the drop button");
    assert_eq!(c.get(r.x, r.y), cs.button_face.argb(), "outer top-left");
    assert_eq!(
        c.get(r.x + 1, r.y + 1),
        cs.button_hilight.argb(),
        "inner top-left"
    );
    let (x1, y1) = (r.right() - 1, r.bottom() - 1);
    assert_eq!(
        c.get(x1, y1),
        cs.button_dk_shadow.argb(),
        "outer bottom-right"
    );
    assert_eq!(
        c.get(x1 - 1, y1 - 1),
        cs.button_shadow.argb(),
        "inner bottom-right"
    );
}

#[test]
fn run_highlights_its_command_13_rows_tall_from_6_px_in() {
    let theme = Theme::default();
    let Some(text) = tahoma() else { return };
    let d = Dialog::run(&history());
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
        true,
        None,
        Cues::NONE,
        true,
        None,
    );
    let f = l.field.unwrap();
    let hl = theme.colors.hilight.argb();
    let row = f.y + 3;
    let first = (f.x..f.right()).find(|&x| c.get(x, row) == hl);
    assert_eq!(first, Some(f.x + 6), "the highlight's first column");
    let rows: Vec<i32> = (f.y..f.bottom())
        .filter(|&y| (f.x..f.right()).any(|x| c.get(x, y) == hl))
        .collect();
    assert_eq!(rows, (f.y + 3..f.y + 16).collect::<Vec<_>>(), "13 rows");
    assert_eq!(
        l.hit(&d, f.x + 10, f.y + 5, &text, theme.fonts.size_px),
        Some(DialogHit::Field(4))
    );
}

#[test]
fn run_dialog_edits_and_runs_the_field() {
    let mut d = Dialog::run(&[]);
    assert_eq!(
        d.key(Key::Edit(EditKey::Insert("b".into()))),
        Outcome::Redraw
    );
    assert_eq!(d.press(2), Outcome::BrowseRun);
    assert_eq!(d.set_field("'/usr/bin/foot'"), Outcome::Redraw);
    assert_eq!(d.key(Key::Enter), Outcome::Run("'/usr/bin/foot'".into()));
    let mut d = Dialog::run(&[]);
    assert_eq!(
        d.key(Key::Enter),
        Outcome::Nothing,
        "empty field stays open"
    );
    assert_eq!(
        d.key(Key::Edit(EditKey::Insert("xt".into()))),
        Outcome::Redraw
    );
    assert_eq!(
        d.key(Key::Edit(EditKey::Insert("erm".into()))),
        Outcome::Redraw
    );
    assert_eq!(d.key(Key::Backspace), Outcome::Redraw);
    assert_eq!(d.key(Key::Backspace), Outcome::Redraw);
    assert_eq!(
        d.key(Key::Edit(EditKey::Insert("rm ".into()))),
        Outcome::Redraw
    );
    assert_eq!(d.key(Key::Enter), Outcome::Run("xterm".into()));
    assert_eq!(d.press(1), Outcome::Close);
    assert_eq!(d.key(Key::Escape), Outcome::Close);
    let mut empty = Dialog::run(&[]);
    assert_eq!(empty.key(Key::Backspace), Outcome::Nothing);
}

#[test]
fn the_wheel_scrolls_run_s_open_list_wherever_the_pointer_is() {
    let mut d = Dialog::run(&history());
    assert_eq!(d.wheel(None, 3), Outcome::Nothing, "the list is closed");
    d.click(Some(DialogHit::Drop));
    let top = |d: &Dialog| d.field.as_ref().unwrap().combo.top;
    assert_eq!(d.wheel(Some(DialogHit::Button(0)), 3), Outcome::Redraw);
    assert_eq!(top(&d), 2, "five commands, three rows: the last two");
    assert_eq!(d.wheel(None, 3), Outcome::Nothing, "at the bottom");
    assert_eq!(d.wheel(None, -3), Outcome::Redraw);
    assert_eq!(top(&d), 0);
    assert_eq!(d.field.as_ref().unwrap().combo.text(), "notepad");
}

#[test]
fn a_readers_focus_on_a_button_moves_the_keyboard_without_pressing_it() {
    let Some(text) = tahoma() else { return };
    let theme = Theme::default();
    let px = theme.fonts.size_px;
    let middle = |r: Rect| (r.x + r.width / 2, r.y + r.height / 2);
    let mut run = Dialog::run(&history());
    let l = DialogLayout::compute(&theme, &text, &run);
    let (x, y) = middle(l.buttons[1]);
    assert_eq!(run.focus_at(&l, x, y, &text, px), Outcome::Redraw);
    assert_eq!(run.button_focus, Some(1), "Cancel wears the focus");
    assert_eq!(run.focus_at(&l, x, y, &text, px), Outcome::Nothing);
    assert_eq!(run.key(Key::Tab { back: false }), Outcome::Redraw);
    assert_eq!(run.button_focus, Some(2), "Tab goes on from Cancel");

    let path = std::path::Path::new("/srv/a.txt");
    let mut ask = Dialog::confirm_delete("a.txt", path, false, false, false);
    let l = DialogLayout::compute(&theme, &text, &ask);
    let no = ask.buttons.iter().position(|b| !b.default).unwrap();
    let (x, y) = middle(l.buttons[no]);
    assert_eq!(ask.focus_at(&l, x, y, &text, px), Outcome::Redraw);
    let defaults: Vec<bool> = ask.buttons.iter().map(|b| b.default).collect();
    assert!(defaults[no] && defaults.iter().filter(|&&d| d).count() == 1);
}

#[test]
fn run_s_tab_walks_the_field_and_its_buttons() {
    let mut d = Dialog::run(&history());
    let tab = |d: &mut Dialog, back| d.key(Key::Tab { back });
    assert_eq!(d.button_focus, None, "Run opens on the field");
    assert_eq!(tab(&mut d, false), Outcome::Redraw);
    assert_eq!(d.button_focus, Some(0), "Tab moves to OK");
    tab(&mut d, false);
    assert_eq!(d.button_focus, Some(1), "then Cancel");
    tab(&mut d, false);
    assert_eq!(d.button_focus, Some(2), "then Browse...");
    tab(&mut d, false);
    assert_eq!(d.button_focus, None, "and round to the field");
    assert_eq!(tab(&mut d, true), Outcome::Redraw);
    assert_eq!(
        d.button_focus,
        Some(2),
        "Shift+Tab from the field is Browse..."
    );
    tab(&mut d, true);
    assert_eq!(d.button_focus, Some(1), "then Cancel");
    let before = d.field.clone();
    assert_eq!(
        d.key(Key::Edit(EditKey::Insert(" ".into()))),
        Outcome::Close,
        "Space presses Cancel"
    );
    assert_eq!(d.field, before, "the space stayed out of the field");
    assert_eq!(d.key(Key::Enter), Outcome::Close, "Enter presses Cancel");
    tab(&mut d, false);
    assert_eq!(
        d.key(Key::Down),
        Outcome::Nothing,
        "Down leaves the history alone"
    );
    assert_eq!(d.field, before, "the field is unchanged");
    assert_eq!(
        d.key(Key::Enter),
        Outcome::BrowseRun,
        "Enter presses Browse..."
    );
    assert_eq!(d.button_focus, None);
    d.key(Key::Edit(EditKey::End { select: false }));
    d.key(Key::Edit(EditKey::Insert(" x".into())));
    assert_eq!(d.key(Key::Enter), Outcome::Run("notepad x".into()));
}

#[test]
fn run_s_arrows_walk_its_buttons_and_stop_at_the_ends() {
    let mut d = Dialog::run(&history());
    let left = || {
        Key::Edit(EditKey::Left {
            select: false,
            word: false,
        })
    };
    let right = || {
        Key::Edit(EditKey::Right {
            select: false,
            word: false,
        })
    };
    let before = d.field.clone();
    assert_eq!(d.key(Key::Down), Outcome::Redraw);
    assert_ne!(d.field, before, "Down on the field steps the history");
    assert_eq!(d.button_focus, None, "the field keeps the focus");
    let before = d.field.clone();
    d.key(left());
    assert_ne!(d.field, before, "Left on the field moves the caret");
    assert_eq!(d.button_focus, None, "the field keeps the focus");
    d.key(Key::Tab { back: false });
    assert_eq!(d.button_focus, Some(0), "Tab moves to OK");
    let field = d.field.clone();
    assert_eq!(d.key(right()), Outcome::Redraw);
    assert_eq!(d.button_focus, Some(1), "Right moves to Cancel");
    assert_eq!(d.key(Key::Down), Outcome::Redraw);
    assert_eq!(d.button_focus, Some(2), "Down moves to Browse...");
    assert_eq!(d.key(Key::Down), Outcome::Nothing);
    assert_eq!(d.button_focus, Some(2), "Down stops at Browse...");
    assert_eq!(d.key(right()), Outcome::Nothing);
    assert_eq!(d.button_focus, Some(2), "Right stops at Browse...");
    d.key(Key::Up);
    assert_eq!(d.button_focus, Some(1), "Up moves to Cancel");
    d.key(left());
    assert_eq!(d.button_focus, Some(0), "Left moves to OK");
    assert_eq!(d.key(Key::Up), Outcome::Nothing);
    assert_eq!(d.key(left()), Outcome::Nothing);
    assert_eq!(d.button_focus, Some(0), "Up and Left stop at OK");
    assert_eq!(d.field, field, "the field is unchanged");
}

#[test]
fn run_s_field_has_the_focus_after_browse_returns() {
    let mut d = Dialog::run(&[]);
    d.key(Key::Edit(EditKey::Insert("b".into())));
    for _ in 0..3 {
        d.key(Key::Tab { back: false });
    }
    assert_eq!(d.button_focus, Some(2), "Browse... has the focus");
    assert_eq!(d.key(Key::Enter), Outcome::BrowseRun);
    assert_eq!(
        d.button_focus, None,
        "the field has the focus once the picker is cancelled"
    );
    assert_eq!(d.key(Key::Enter), Outcome::Run("b".into()), "Enter is OK");
    let mut d = Dialog::run(&[]);
    d.key(Key::Tab { back: true });
    assert_eq!(d.key(Key::Space), Outcome::BrowseRun);
    d.set_field("'/usr/bin/foot'");
    assert_eq!(
        d.button_focus, None,
        "the field has the focus once the pick lands"
    );
    assert_eq!(
        d.key(Key::Enter),
        Outcome::Run("'/usr/bin/foot'".into()),
        "Enter runs the picked path"
    );
}

#[test]
fn run_s_focused_button_wears_the_ring_and_the_field_hides_its_caret() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let mut d = Dialog::run(&[]);
    d.key(Key::Edit(EditKey::Insert("ab".into())));
    d.key(Key::Tab { back: false });
    d.key(Key::Tab { back: false });
    assert_eq!(d.button_focus, Some(1), "Cancel has the focus");
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
        true,
        None,
        Cues::ALL,
        true,
        None,
    );
    let cs = &theme.colors;
    let mid = |r: Rect| (r.x, r.y + r.height / 2);
    let (ok, cancel) = (l.buttons[0], l.buttons[1]);
    assert_eq!(
        c.get(mid(cancel).0, mid(cancel).1),
        cs.button_text.argb(),
        "Cancel's left edge is the black ring"
    );
    assert_eq!(
        c.get(mid(ok).0, mid(ok).1),
        cs.button_hilight.argb(),
        "OK's left edge is the plain white edge"
    );
    let dots = cancel.inset(4);
    assert!(
        (dots.x..dots.right()).any(|x| c.get(x, dots.y) == cs.button_text.argb()),
        "Cancel wears the focus dots"
    );
    let f = l.field.unwrap();
    let inner = f.inset(3);
    let caret_x = f.x + combo::EDIT_TEXT_X + text.measure("ab", false, theme.fonts.size_px);
    assert_eq!(
        c.get(caret_x, inner.y + inner.height / 2),
        cs.window.argb(),
        "the field shows no caret"
    );
}

#[test]
fn run_focuses_its_labelled_field_and_names_its_buttons() {
    use reprisekit::access::accesskit::Role;
    let (t, focus) = tree_of(&Dialog::run(&history()));
    assert_eq!(t.get(focus).unwrap().role(), Role::TextInput);
    let combo = t.nodes().find(|(_, n)| n.role() == Role::EditableComboBox);
    let (_, combo) = combo.expect("the field is an editable combo");
    let label = t.get(combo.labelled_by()[0]).unwrap();
    assert_eq!(label.value(), Some("Open:"), "the field is labelled");
    for name in ["OK", "Cancel", "Browse..."] {
        assert!(
            t.nodes().any(|(_, n)| n.label() == Some(name)),
            "{name} is a button"
        );
    }
}

#[test]
fn run_s_drop_button_draws_held_until_the_release() {
    let mut d = Dialog::run(&["a".into()]);
    assert_eq!(d.click(Some(DialogHit::Drop)), Outcome::Redraw);
    assert_eq!(d.field_lit.of(FieldPart::Drop), Interaction::Pressed);
    assert!(d.drag_drop(None), "dragged off");
    assert_eq!(d.field_lit.of(FieldPart::Drop), Interaction::Idle);
    assert!(d.drag_drop(Some(DialogHit::Drop)), "back on");
    assert_eq!(d.release(Some(DialogHit::Drop)), Outcome::Redraw);
    assert!(!d.field_lit.holding());
    assert_eq!(d.field_lit.of(FieldPart::Drop), Interaction::Hot);
}

#[test]
fn run_s_field_lights_its_frame_from_either_part_and_its_button_alone() {
    let mut d = Dialog::run(&[]);
    assert!(d.hover(Some(DialogHit::Field(3)), (0, 0), true));
    assert_eq!(d.field_lit.of(FieldPart::Edit), Interaction::Hot);
    assert_eq!(d.field_lit.of(FieldPart::Drop), Interaction::Idle);
    assert!(d.hover(Some(DialogHit::Drop), (0, 0), true));
    assert_eq!(d.field_lit.of(FieldPart::Drop), Interaction::Hot);
    assert!(d.leave(true));
    assert_eq!(d.field_lit.lit(), None);
}

#[test]
fn default_button_has_a_black_ring_and_field_shows_a_caret() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let mut d = Dialog::run(&[]);
    d.key(Key::Edit(EditKey::Insert("ab".into())));
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
        true,
        None,
        Cues::ALL,
        true,
        None,
    );
    let black = theme.colors.button_text.argb();
    let ok = l.buttons[0];
    assert_eq!(c.get(ok.x, ok.y + ok.height / 2), black);
    assert_eq!(
        c.get(ok.x + 1, ok.y + ok.height / 2),
        theme.colors.button_hilight.argb()
    );
    let cancel = l.buttons[1];
    assert_eq!(
        c.get(cancel.x, cancel.y + cancel.height / 2),
        theme.colors.button_hilight.argb()
    );
    let f = l.field.unwrap();
    let inner = f.inset(3);
    let caret_x = f.x + combo::EDIT_TEXT_X + text.measure("ab", false, theme.fonts.size_px);
    assert_eq!(
        c.get(caret_x, inner.y + inner.height / 2),
        theme.colors.button_text.argb()
    );
    assert_eq!(
        c.get(l.drop.unwrap().x - 3, inner.y + 2),
        theme.colors.window.argb()
    );
}

/// A blink repaints the field over the caret-off raster; the result
/// is the caret-on raster, pixel for pixel, and painting the field
/// again with the caret off gets the caret-off one back.
#[test]
fn a_blink_over_the_kept_raster_matches_a_full_draw() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let mut d = Dialog::run(&history());
    d.key(Key::Edit(EditKey::Insert("notepad".into())));
    let l = DialogLayout::compute(&theme, &text, &d);
    let full = |caret: bool| {
        draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            caret,
            None,
            Cues::ALL,
            true,
            None,
        )
    };
    let off = full(false);
    let on = full(true);
    assert_ne!(off, on, "the caret shows");
    let mut c = off.clone();
    draw_field(&mut c, &theme, &text, &l, &d, true);
    assert_eq!(c, on);
    draw_field(&mut c, &theme, &text, &l, &d, false);
    assert_eq!(c, off);
}

/// The open drop list's bottom frame pixel sits under the third row:
/// it is no row, rather than the hidden fourth one.
#[test]
fn the_run_lists_bottom_frame_row_hits_no_row() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let mut d = Dialog::run(&history());
    d.key(Key::Drop);
    let l = DialogLayout::compute(&theme, &text, &d);
    let list = l.list.unwrap();
    assert_eq!(
        l.hit(&d, list.x + 5, list.bottom() - 2, &text, px),
        Some(DialogHit::Row(2))
    );
    assert_eq!(l.hit(&d, list.x + 5, list.bottom() - 1, &text, px), None);
}

#[test]
fn run_opens_on_the_last_command_selected_and_typing_replaces_it() {
    let mut d = Dialog::run(&history());
    let f = d.field.as_ref().unwrap();
    assert_eq!(f.edit().unwrap().text, "notepad");
    assert!(f.edit().unwrap().all_selected());
    assert_eq!(
        d.key(Key::Edit(EditKey::Insert("x".into()))),
        Outcome::Redraw
    );
    let f = d.field.as_ref().unwrap();
    assert_eq!(
        (
            f.edit().unwrap().text.as_str(),
            f.edit().unwrap().has_selection()
        ),
        ("x", false)
    );
    let mut d = Dialog::run(&history());
    assert_eq!(d.key(Key::Backspace), Outcome::Redraw);
    assert_eq!(d.field.as_ref().unwrap().combo.text(), "");
    let mut d = Dialog::run(&history());
    assert_eq!(d.key(Key::Enter), Outcome::Run("notepad".into()));
}

#[test]
fn up_and_down_walk_the_history_and_f4_opens_the_list() {
    let mut d = Dialog::run(&history());
    let value = |d: &Dialog| d.field.as_ref().unwrap().combo.text().to_string();
    assert_eq!(d.key(Key::Up), Outcome::Nothing, "newest already");
    d.key(Key::Down);
    assert_eq!(value(&d), "mspaint");
    d.key(Key::Up);
    assert_eq!(value(&d), "notepad");
    d.key(Key::Edit(EditKey::Insert("z".into())));
    d.key(Key::Down);
    assert_eq!(value(&d), "notepad");
    assert_eq!(d.key(Key::Drop), Outcome::Redraw);
    let field = |d: &Dialog| d.field.clone().unwrap();
    assert_eq!((field(&d).combo.open, field(&d).combo.top), (Some(0), 0));
    for _ in 0..3 {
        d.key(Key::Down);
    }
    assert_eq!((field(&d).combo.open, field(&d).combo.top), (Some(3), 1));
    assert_eq!(value(&d), "cmd");
    assert_eq!(d.key(Key::Enter), Outcome::Redraw);
    assert_eq!(field(&d).combo.open, None);
    assert!(field(&d).edit().unwrap().all_selected());
    d.key(Key::Drop);
    assert_eq!(d.key(Key::Escape), Outcome::Redraw);
    assert_eq!(d.key(Key::Escape), Outcome::Close);
    let mut plain = Dialog::run(&[]);
    assert_eq!(plain.key(Key::Drop), Outcome::Nothing);
    assert_eq!(plain.key(Key::Down), Outcome::Nothing);
}

#[test]
fn remember_moves_a_repeat_to_the_front_and_caps_the_list() {
    let mut h = history();
    remember(&mut h, "calc");
    assert_eq!(h[..3], ["calc", "notepad", "mspaint"]);
    assert_eq!(h.len(), 5);
    for i in 0..40 {
        remember(&mut h, &format!("cmd{i}"));
    }
    assert_eq!(h.len(), HISTORY_MAX);
    assert_eq!(h[0], "cmd39");
}

#[test]
fn a_press_on_the_drop_lists_thumb_keeps_the_list_open() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let mut d = Dialog::run(&history());
    let l = DialogLayout::compute(&theme, &text, &d);
    d.click(Some(DialogHit::Drop));
    let sb = l.scrollbar(&d).unwrap();
    let thumb = l.hit(&d, sb.x + 8, sb.y + sb.height / 2, &text, px);
    assert!(matches!(thumb, Some(DialogHit::ListBar(BarHit::Thumb(_)))));
    d.click(thumb);
    assert!(
        d.field.as_ref().unwrap().combo.open.is_some(),
        "the list stays open"
    );
    // The thumb is grabbed until the release. Three rows leave a 7 px
    // track that is all thumb, so the drag holds the list where it is.
    assert!(d.holding_thumb());
    assert_eq!(d.drag_list(&l, sb.bottom() + 200), Outcome::Nothing);
    assert_eq!(d.field.as_ref().unwrap().combo.top, 0);
    assert_eq!(d.release(None), Outcome::Redraw);
    assert!(!d.holding_thumb());
    assert!(d.field.as_ref().unwrap().combo.open.is_some());
}

#[test]
fn text_dropped_on_runs_field_lands_at_the_drop_caret_and_takes_the_keyboard() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let mut d = Dialog::run(&[]);
    d.set_field("foot");
    d.button_focus = Some(0);
    let l = DialogLayout::compute(&theme, &text, &d);
    let field = l.field.unwrap();
    let ok = l.buttons[0];
    assert!(
        !d.drag_over(&l, ok.x + 5, ok.y + 5, &text, px),
        "a button takes no drop"
    );
    let x = field.x + combo::EDIT_TEXT_X + text.measure("fo", false, px);
    assert!(d.drag_over(&l, x, field.y + 5, &text, px));
    assert_eq!(
        d.drop_text(&l, &crate::textedit::DropLine::Text("X".into()), &text, px),
        Outcome::Redraw
    );
    let edit = d.field.as_ref().unwrap().edit().unwrap();
    assert_eq!(edit.text, "foXot");
    assert_eq!(edit.selected_text(), "X");
    assert_eq!(d.button_focus, None, "the field has the keyboard");
    assert_eq!(
        d.drop_text(&l, &crate::textedit::DropLine::Text("Y".into()), &text, px),
        Outcome::Nothing,
        "the drop ended the drag"
    );
}

#[test]
fn combo_layout_and_clicks() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let mut d = Dialog::run(&history());
    let l = DialogLayout::compute(&theme, &text, &d);
    let field = l.field.unwrap();
    assert_eq!(field.height, 21);
    let drop = l.drop.unwrap();
    assert_eq!(drop, Rect::new(field.right() - 18, field.y + 2, 16, 17));
    let list = l.list.unwrap();
    assert_eq!(list, Rect::new(field.x, field.bottom(), field.width, 41));
    assert!(list.bottom() <= l.size.1, "the list fits on the dialog");
    assert_eq!(
        l.hit(&d, drop.x + 8, drop.y + 8, &text, theme.fonts.size_px),
        Some(DialogHit::Drop)
    );
    let ok = l.buttons[0];
    assert_eq!(
        l.hit(&d, ok.x + 5, ok.y + 5, &text, theme.fonts.size_px),
        Some(DialogHit::Button(0))
    );
    assert_eq!(
        l.hit(&d, field.x + 5, field.y + 5, &text, theme.fonts.size_px),
        Some(DialogHit::Field(5 - combo::EDIT_TEXT_X))
    );
    assert_eq!(d.click(Some(DialogHit::Drop)), Outcome::Redraw);
    assert!(d.field.as_ref().unwrap().combo.open.is_some());
    assert_eq!(
        l.hit(
            &d,
            list.x + 5,
            list.y + 1 + 13 + 6,
            &text,
            theme.fonts.size_px
        ),
        Some(DialogHit::Row(1))
    );
    let sb = l.scrollbar(&d).unwrap();
    assert_eq!(sb, Rect::new(list.right() - 17, list.y + 1, 16, 39));
    assert_eq!(
        l.hit(&d, sb.x + 8, sb.y + 3, &text, theme.fonts.size_px),
        Some(DialogHit::ListBar(BarHit::Back))
    );
    assert_eq!(
        l.hit(&d, sb.x + 8, sb.bottom() - 3, &text, theme.fonts.size_px),
        Some(DialogHit::ListBar(BarHit::Forward))
    );
    // The 7 px track between the arrows is all thumb, so the hit is
    // never a page here; a page click still moves three rows of five.
    assert_eq!(
        l.hit(
            &d,
            sb.x + 8,
            sb.y + ControlMetrics::CLASSIC.scroll + 3,
            &text,
            theme.fonts.size_px
        ),
        Some(DialogHit::ListBar(BarHit::Thumb(3)))
    );
    assert_eq!(
        d.click(Some(DialogHit::ListBar(BarHit::PageForward))),
        Outcome::Redraw
    );
    assert_eq!(d.field.as_ref().unwrap().combo.top, 2);
    assert_eq!(
        d.click(Some(DialogHit::ListBar(BarHit::PageBack))),
        Outcome::Redraw
    );
    assert_eq!(d.field.as_ref().unwrap().combo.top, 0);
    assert_eq!(
        d.click(Some(DialogHit::ListBar(BarHit::Forward))),
        Outcome::Redraw
    );
    assert_eq!(d.field.as_ref().unwrap().combo.top, 1);
    assert_eq!(
        l.hit(&d, list.x + 5, list.y + 5, &text, theme.fonts.size_px),
        Some(DialogHit::Row(1))
    );
    assert_eq!(d.click(Some(DialogHit::Row(2))), Outcome::Redraw);
    let f = d.field.as_ref().unwrap();
    assert_eq!(
        (
            f.edit().unwrap().text.as_str(),
            f.combo.open,
            f.edit().unwrap().all_selected()
        ),
        ("calc", None, true)
    );
    d.click(Some(DialogHit::Drop));
    assert_eq!(d.click(None), Outcome::Redraw);
    assert_eq!(d.field.as_ref().unwrap().combo.open, None);
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    assert_eq!(d.press_field(&text, px, 1000, false), Outcome::Redraw);
    let f = |d: &Dialog| d.field.as_ref().unwrap().edit().unwrap().clone();
    assert!(!f(&d).has_selection());
    assert_eq!(f(&d).caret(), f(&d).text.len());
    assert_eq!(d.drag_field(&text, px, 0), Outcome::Redraw);
    assert_eq!(f(&d).selected_text(), f(&d).text);
    assert_eq!(d.double_click_field(&text, px, 2), Outcome::Redraw);
    assert_eq!(f(&d).selected_text(), f(&d).text);
    assert_eq!(d.paste("ls -l\nrm"), Outcome::Redraw);
    assert_eq!(f(&d).text, "ls -l");
}

#[test]
fn combo_draws_help_button_selection_and_list() {
    let theme = Theme::default();
    let Some(text) = tahoma() else { return };
    let cs = &theme.colors;
    let mut d = Dialog::run(&history());
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
        true,
        None,
        Cues::ALL,
        true,
        None,
    );
    let help = l.help.unwrap();
    assert_eq!(help.right(), l.close.unwrap().x - 2);
    assert_eq!(c.get(help.x + 7, help.y + 2), cs.button_text.argb());
    assert_eq!(c.get(help.x + 7, help.y + 8), cs.button_face.argb());
    let f = l.field.unwrap();
    assert_eq!(c.get(f.x, f.y + 10), cs.button_shadow.argb());
    assert_eq!(c.get(f.x + 1, f.y + 10), cs.button_dk_shadow.argb());
    assert_eq!(c.get(f.x + 5, f.y + 4), cs.window.argb());
    assert_eq!(c.get(f.x + 6, f.y + 4), cs.hilight.argb());
    let dr = l.drop.unwrap();
    assert_eq!(c.get(dr.x + 8, dr.y + 7), cs.button_text.argb());
    assert_eq!(c.get(dr.x + 4, dr.y + 7), cs.button_face.argb());
    let list = l.list.unwrap();
    assert_eq!(c.get(list.x, list.y + 5), cs.button_face.argb());
    d.click(Some(DialogHit::Drop));
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
    assert_eq!(c.get(list.x, list.y + 5), cs.button_text.argb());
    assert_eq!(c.get(list.x + 30, list.y + 6), cs.hilight.argb());
    assert_eq!(c.get(list.x + 2, list.y + 1 + 13 + 6), cs.window.argb());
    let sb = l.scrollbar(&d).unwrap();
    assert_eq!(c.get(sb.x, sb.y), cs.button_face.argb());
    assert_eq!(c.get(sb.x + 1, sb.y + 1), cs.button_hilight.argb());
    assert_eq!(c.get(sb.x + 1, sb.y + 17), cs.button_hilight.argb());
    assert_eq!(c.get(sb.x + 8, sb.y + 20), cs.button_face.argb());
    assert_eq!(c.get(sb.x + 8, sb.y + 16 + 7 + 7), cs.button_text.argb());
}
