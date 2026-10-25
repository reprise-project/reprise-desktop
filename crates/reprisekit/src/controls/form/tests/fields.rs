use super::*;

#[test]
fn typing_reaches_a_focused_edit_instead_of_an_access_key() {
    let mut f = edit_form();
    assert_eq!(f.focus, Some(1), "the edit is the first focusable item");
    assert_eq!(f.key(Key::Char('o')), Event::Changed("find"));
    assert_eq!(f.key(Key::Char('k')), Event::Changed("find"));
    assert!(matches!(&f.items[1].control, Control::Edit { edit, .. } if edit.text == "ok"));
    assert_eq!(f.focus, Some(1));
}

#[test]
fn edit_text_reads_an_edit_and_is_empty_for_anything_else() {
    let mut f = edit_form();
    f.key(Key::Char('o'));
    assert_eq!(f.edit_text("find"), "o");
    assert_eq!(f.edit_text("lbl"), "", "a label is not an edit");
    assert_eq!(f.edit_text("missing"), "");
}

#[test]
fn a_paste_into_a_field_takes_its_first_line_only() {
    let mut f = edit_form();
    assert_eq!(f.paste("pw\nsecond".into()), Event::Changed("find"));
    assert!(matches!(&f.items[1].control, Control::Edit { edit, .. } if edit.text == "pw"));
    f.focus_id("ok");
    assert_eq!(f.paste("x".into()), Event::None, "a button takes no paste");
}

/// Alt+letter is an access key even while a field has the focus
/// (Alt+B on Run presses Browse...), so it never types into it.
#[test]
fn an_access_key_skips_a_focused_edit() {
    let mut f = edit_form();
    if let Control::Button { label, .. } = &mut f.items[2].control {
        *label = "&OK".into();
    }
    f.key(Key::Char('x'));
    assert_eq!(f.key(Key::Access('O')), Event::Pressed("ok"));
    assert!(matches!(&f.items[1].control, Control::Edit { edit, .. } if edit.text == "x"));
    assert_eq!(f.key(Key::Access('q')), Event::None);
    assert!(matches!(&f.items[1].control, Control::Edit { edit, .. } if edit.text == "x"));
}

#[test]
fn edit_keys_reach_the_field_and_dialog_keys_still_do_not() {
    let mut f = edit_form();
    f.key(Key::Char('a'));
    f.key(Key::Char('b'));
    assert_eq!(
        f.key(Key::Edit(EditKey::Backspace { word: false })),
        Event::Changed("find")
    );
    assert!(matches!(&f.items[1].control, Control::Edit { edit, .. } if edit.text == "a"));
    assert_eq!(
        f.key(Key::Edit(EditKey::Home { select: true })),
        Event::Changed("find")
    );
    assert!(matches!(&f.items[1].control, Control::Edit { edit, .. } if edit.has_selection()));
    f.key(Key::Edit(EditKey::End { select: false }));
    assert_eq!(f.key(Key::Space), Event::Changed("find"));
    assert!(matches!(&f.items[1].control, Control::Edit { edit, .. } if edit.text == "a "));
    assert_eq!(f.key(Key::Enter), Event::Default);
    assert_eq!(f.key(Key::Escape), Event::Cancel);
    f.focus_id("ok");
    assert_eq!(f.key(Key::Char('n')), Event::Redraw);
    assert_eq!(f.focus, Some(1));
}

/// A tree's thumb arms a drag, and the sweep scrolls the tree by
/// where the thumb lands, the way a list box's does.
#[test]
fn a_trees_thumb_drags() {
    use crate::controls::treeview::{self, Node, TreeView};
    let t = tr();
    let mut root = Node::new("root", true);
    root.children = (0..30)
        .map(|i| Node::new(&format!("n{i}"), false))
        .collect();
    root.expanded = true;
    let tv = TreeView::new(vec![root], treeview::Style::Classic);
    let rect = Rect::new(0, 0, 150, 100);
    let mut f = Form::new(vec![Item::new("tree", rect, Control::Tree(tv))]);
    let (sb, rows, grab_y) = {
        let Control::Tree(tv) = &f.items[0].control else {
            panic!()
        };
        let lay = treeview::layout_without_text(tv, rect, &ControlMetrics::CLASSIC);
        let sb = lay.scrollbar.expect("31 rows overflow 100 px");
        let p =
            crate::classic::widgets::scrollbar_parts(sb, 1, 0, lay.rows, 31, false, lay.thumb_min);
        (sb, lay.rows, p.thumb.expect("a thumb").y + 2)
    };
    let h = f.hit(sb.x + 8, grab_y, &t, PX).expect("the thumb");
    assert_eq!(
        h.part,
        Part::Tree(treeview::Hit::VBar(crate::classic::widgets::BarHit::Thumb(
            2
        )))
    );
    assert_eq!(f.click(Some(h)), Event::Redraw);
    let held = match f.drag {
        Some(Drag::Bar { item: 0, thumb }) => thumb.grab(Axis::Vertical),
        _ => None,
    };
    assert_eq!(held, Some(2), "the tree's thumb, grabbed 2 px in");
    assert_eq!(f.motion(sb.x + 8, sb.bottom(), None), Event::Redraw);
    let Control::Tree(tv) = &f.items[0].control else {
        panic!()
    };
    assert_eq!(tv.top, 31 - rows, "dragged to the end: the last page");
    f.release();
    assert_eq!(f.drag, None);
}

#[test]
fn clicking_an_edit_places_the_caret_and_dragging_extends_it() {
    let t = tr();
    let mut f = edit_form();
    let Control::Edit { edit, .. } = &mut f.items[1].control else {
        panic!("the fixture's second item is the edit")
    };
    *edit = TextEdit::new("Hello world", false);
    // The pen starts FIELD_PAD inside the frame, so 30 px along the
    // text area lands somewhere in the middle of the value.
    let x = 80 + crate::classic::widgets::FIELD_PAD + 30;
    let h = f.hit(x, 15, &t, PX).expect("the edit is hit");
    assert_eq!(h.item, 1);
    let Part::Edit(at) = h.part else {
        panic!("{:?} is not an edit hit", h.part)
    };
    assert_eq!(at, crate::textedit::index_at(&t, "Hello world", 30, PX));
    assert!(at > 0 && at < "Hello world".len(), "a boundary inside it");
    assert!(f.hit(300, 300, &t, PX).is_none());

    assert_eq!(f.click(Some(h)), Event::Redraw);
    let Control::Edit { edit, .. } = &f.items[1].control else {
        panic!()
    };
    assert_eq!(
        edit.caret(),
        at,
        "the click put the caret under the pointer"
    );
    assert!(!edit.has_selection(), "and dropped the selection");
    assert_eq!(f.drag, Some(Drag::Edit(1)), "the press armed a sweep");

    let back = 80 + crate::classic::widgets::FIELD_PAD + 10;
    let over = f.hit(back, 15, &t, PX);
    let Some(Part::Edit(to)) = over.map(|h| h.part) else {
        panic!("the drag is still over the edit")
    };
    assert!(to < at);
    assert_eq!(f.motion(back, 15, over), Event::Redraw);
    let Control::Edit { edit, .. } = &f.items[1].control else {
        panic!()
    };
    assert_eq!(edit.selection(), to..at, "the drag swept back to `to`");
}

#[test]
fn a_right_press_focuses_a_field_and_keeps_a_selection_under_it() {
    let t = tr();
    let mut f = edit_form();
    let Control::Edit { edit, .. } = &mut f.items[1].control else {
        panic!("the fixture's second item is an edit")
    };
    *edit = TextEdit::new("Hello world", false);
    edit.set_selection(0, 5);
    f.set_focus(Some(2));
    let at = |f: &Form, dx: i32| f.hit(80 + crate::classic::widgets::FIELD_PAD + dx, 15, &t, PX);

    let inside = at(&f, 8);
    assert_eq!(f.right_press(inside), Event::Redraw);
    assert_eq!(f.focus, Some(1), "the field takes the focus");
    assert_eq!(
        field(&f).0.selection(),
        0..5,
        "a press inside the selection keeps it"
    );
    assert_eq!(f.drag, None, "a right press starts no drag");

    let outside = at(&f, 60);
    let Some(Hit {
        part: Part::Edit(i),
        ..
    }) = outside
    else {
        panic!("the press lands in the field");
    };
    f.right_press(outside);
    assert!(
        !field(&f).0.has_selection(),
        "a press off the selection drops it"
    );
    assert_eq!(field(&f).0.caret(), i);

    let ok = f.hit(20, 50, &t, PX);
    assert_eq!(f.right_press(ok), Event::None, "a button is not a field");
    assert_eq!(f.focus, Some(1));
}

fn field(f: &Form) -> (&TextEdit, usize) {
    match &f.items[1].control {
        Control::Edit { edit, first, .. } => (edit, first.get()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_drag_over_a_field_drops_its_text_at_the_drop_caret() {
    let t = tr();
    let mut f = edit_form();
    f.paste("hello world".into());
    f.set_focus(None);
    let inner_x = 80 + crate::classic::widgets::FIELD_PAD;
    let x = inner_x + t.measure("hello", false, PX);
    assert!(!f.drag_over(30, 15, &t, PX), "a label takes no drop");
    assert!(f.drag_over(x, 15, &t, PX));
    assert_eq!(
        f.drop_text(&crate::textedit::DropLine::Text(" big".into()), &t, PX),
        Event::Changed("find")
    );
    assert_eq!(f.focus(), Some(1), "the field takes the focus");
    assert_eq!(field(&f).0.text, "hello big world");
    assert_eq!(field(&f).0.selected_text(), " big");
    assert_eq!(
        f.drop_text(&crate::textedit::DropLine::Text("x".into()), &t, PX),
        Event::None,
        "the drop ended the drag"
    );
}

#[test]
fn a_masked_or_greyed_field_takes_no_drop() {
    let t = tr();
    let mut f = edit_form();
    f.items[1].control = Control::Edit {
        edit: TextEdit::secret(),
        first: Cell::new(0),
        enabled: true,
    };
    assert!(!f.drag_over(120, 15, &t, PX));
    let mut f = edit_form();
    f.set_enabled("find", false);
    assert!(!f.drag_over(120, 15, &t, PX));
    assert!(!f.drag_leave());
}

#[test]
fn a_drag_resting_at_a_fields_edge_scrolls_it_without_moving_the_caret() {
    let t = tr();
    let theme = Theme::default();
    let mut f = edit_form();
    f.paste("abcdefghijklmnopqrstuvwxyz0123456789".into());
    let mut c = Canvas::new(200, 80);
    draw(&mut c, &t, &classic_paint(&theme.colors, 1), &f, true, PX);
    let end = field(&f).1;
    assert!(end > 0, "the caret at the end scrolled the value");
    assert!(f.drag_over(130, 15, &t, PX));
    assert_eq!(f.drag_scroll(&t, PX), None, "the middle rests still");
    assert!(f.drag_over(82, 15, &t, PX));
    assert_eq!(
        f.drag_scroll(&t, PX),
        Some(end - 1),
        "nothing moves before the tick"
    );
    for k in 1..=3 {
        assert!(f.drag_tick(&t, PX));
        draw(&mut c, &t, &classic_paint(&theme.colors, 1), &f, true, PX);
        assert_eq!(field(&f).1, end - k, "the draw keeps the scrolled view");
    }
    let caret = field(&f).0.caret();
    assert!(f.drag_over(178, 15, &t, PX));
    assert!(f.drag_tick(&t, PX));
    assert_eq!(field(&f).1, end - 2);
    assert!(f.drag_leave());
    assert!(!f.drag_tick(&t, PX), "no drag, no step");
    draw(&mut c, &t, &classic_paint(&theme.colors, 1), &f, true, PX);
    assert_eq!(
        field(&f).1,
        end,
        "without a drag the caret pins the view again"
    );
    assert_eq!(field(&f).0.caret(), caret);
}

#[test]
fn the_drop_caret_is_a_bar_where_the_text_would_land() {
    let t = tr();
    let theme = Theme::default();
    let cs = &theme.colors;
    let mut f = edit_form();
    f.paste("hello world".into());
    f.set_focus(None);
    let inner_x = 80 + crate::classic::widgets::FIELD_PAD;
    let x = inner_x + t.measure("hello", false, PX);
    let shot = |f: &Form| {
        let mut c = Canvas::new(200, 40);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &t, &classic_paint(cs, 1), f, true, PX);
        c
    };
    let before = shot(&f);
    f.drag_over(x, 15, &t, PX);
    let after = shot(&f);
    let changed: Vec<(i32, i32)> = (0..40)
        .flat_map(|y| (0..200).map(move |x| (x, y)))
        .filter(|&(x, y)| before.get(x, y) != after.get(x, y))
        .collect();
    assert!(!changed.is_empty());
    assert!(
        changed.iter().all(|&(cx, _)| cx == x),
        "one column at the drop caret: {changed:?}"
    );
    assert!(
        changed
            .iter()
            .all(|&(cx, cy)| after.get(cx, cy) == cs.button_text.argb())
    );
    assert!(changed.len() >= 8, "a line tall");
}

/// `first` is the field's scroll offset and `draw` owns it: typing
/// past the right edge has to bring the caret back into view on the
/// next frame, with no host bookkeeping in between.
#[test]
fn typing_past_the_right_edge_scrolls_the_value_into_view() {
    let t = tr();
    let theme = Theme::default();
    let cs = &theme.colors;
    let mut f = edit_form();
    for ch in "abcdefghijklmnopqrstuvwxyz0123456789".chars() {
        f.key(Key::Char(ch));
    }
    let first = |f: &Form| match &f.items[1].control {
        Control::Edit { first, .. } => first.get(),
        other => panic!("{other:?}"),
    };
    assert_eq!(first(&f), 0, "nothing has drawn yet");

    let mut c = Canvas::new(200, 80);
    c.fill_rect(c.bounds(), cs.button_face);
    draw(&mut c, &t, &classic_paint(cs, 1), &f, true, PX);
    let from = first(&f);
    assert!(from > 0, "the value scrolled to follow the caret");
    // What shows fits the text area, caret included.
    let Control::Edit { edit, .. } = &f.items[1].control else {
        panic!()
    };
    let inner_w = f.items[1].rect.width - 2 * crate::classic::widgets::FIELD_PAD;
    assert!(t.measure(&edit.text[from..edit.caret()], false, PX) <= inner_w);

    f.key(Key::Edit(EditKey::Home { select: false }));
    draw(&mut c, &t, &classic_paint(cs, 1), &f, true, PX);
    assert_eq!(first(&f), 0, "the caret at the start shows the start");
}

/// A focused password field shows no preedit and hands an input
/// method no text, only that it is secret.
#[test]
fn a_masked_field_reports_a_secret_spot_and_shows_no_preedit() {
    let t = tr();
    let theme = Theme::default();
    let cs = &theme.colors;
    let mut f = edit_form();
    let Control::Edit { edit, .. } = &mut f.items[1].control else {
        panic!("the fixture's second item is the edit")
    };
    *edit = TextEdit::secret();
    edit.paste("abc", &[]);
    assert!(f.focused_edit().is_some(), "the edit has the focus");
    let shot = |preedit| {
        let mut c = Canvas::new(200, 80);
        c.set_preedit(preedit);
        draw(&mut c, &t, &classic_paint(cs, 1), &f, false, PX);
        c
    };
    let mut with = shot(crate::textedit::Preedit::new("nihao".into(), 5, 5));
    assert_eq!(with.pixels, shot(None).pixels);
    let spot = with.take_spot().expect("a focused field reports");
    assert!(spot.secret);
    assert_eq!(spot.surrounding, None);
}

/// A masked field draws bullets instead of its value: two values of
/// the same length draw the same pixels, and masking one of them
/// changes what it draws.
#[test]
fn masked_edit_draws_bullets() {
    if !crate::text::test_tahoma() {
        return;
    }
    let t = tr();
    let theme = Theme::default();
    let cs = &theme.colors;
    let shot = |value: &str, masked: bool| {
        let mut f = edit_form();
        let Control::Edit { edit, .. } = &mut f.items[1].control else {
            panic!("the fixture's second item is the edit")
        };
        *edit = if masked {
            TextEdit::secret()
        } else {
            TextEdit::new("", false)
        };
        edit.paste(value, &[]);
        // The caret blink would differ with the bullets' width.
        f.caret = false;
        let mut c = Canvas::new(200, 80);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &t, &classic_paint(cs, 1), &f, false, PX);
        c
    };
    assert_eq!(
        shot("abc", true).pixels,
        shot("xyz", true).pixels,
        "the value does not show through the mask"
    );
    assert_ne!(
        shot("abc", true).pixels,
        shot("abc", false).pixels,
        "and the bullets are not the value"
    );
    // Something is drawn: an empty field would pass both of those.
    let bullets = shot("abc", true);
    let field = edit_form().items[1].rect;
    let ink = (field.x + 4..field.right() - 4)
        .flat_map(|x| (field.y + 4..field.bottom() - 4).map(move |y| (x, y)))
        .filter(|&(x, y)| bullets.get(x, y) == cs.button_text.argb())
        .count();
    assert!(ink > 20, "three bullets' worth of ink, not {ink} px");
}

/// An empty field with a cue draws it in grey text; typing replaces it,
/// and a masked field shows its cue rather than bullets.
#[test]
fn an_empty_edit_draws_its_cue_in_grey() {
    if !crate::text::test_tahoma() {
        return;
    }
    let t = tr();
    let theme = Theme::default();
    let cs = &theme.colors;
    let field = edit_form().items[1].rect;
    let ink = |secret: bool, value: &str| {
        let mut f = edit_form();
        let Control::Edit { edit, .. } = &mut f.items[1].control else {
            panic!("the fixture's second item is the edit")
        };
        *edit = if secret {
            TextEdit::secret()
        } else {
            TextEdit::new("", false)
        };
        edit.set_cue("Unchanged");
        edit.paste(value, &[]);
        f.caret = false;
        let mut c = Canvas::new(200, 80);
        draw(&mut c, &t, &classic_paint(cs, 1), &f, false, PX);
        let count = |colour: Color| {
            (field.x + 3..field.right() - 3)
                .flat_map(|x| (field.y + 3..field.bottom() - 3).map(move |y| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == colour.argb())
                .count()
        };
        (count(cs.gray_text), count(cs.button_text))
    };
    let (grey, black) = ink(false, "");
    assert!(grey > 20, "the cue is drawn in grey, {grey} px");
    assert_eq!(black, 0, "and nothing in the text colour");
    assert_eq!(ink(true, "").0, grey, "a masked field shows the same cue");
    let (grey, black) = ink(false, "abc");
    assert_eq!(grey, 0, "a value hides the cue");
    assert!(black > 10, "and is drawn itself, {black} px");
}

#[test]
fn an_edit_cue_is_its_placeholder() {
    let mut f = edit_form();
    let Control::Edit { edit, .. } = &mut f.items[1].control else {
        panic!("the fixture's second item is the edit")
    };
    edit.set_cue("Unchanged");
    let mut tree = crate::access::Tree::window("Find", (220, 170));
    f.access(&mut tree, crate::access::ROOT, &tr(), 11);
    let node = tree
        .nodes()
        .find(|(_, n)| n.role() == Role::TextInput)
        .map(|(_, n)| n.clone())
        .expect("the edit has a node");
    assert_eq!(node.placeholder(), Some("Unchanged"));
}

#[test]
fn a_disabled_edit_is_skipped_by_tab() {
    let mut f = edit_form();
    if let Control::Edit { enabled, .. } = &mut f.items[1].control {
        *enabled = false;
    }
    let mut f = Form::new(f.items);
    assert_eq!(f.focus, Some(2), "focus starts on OK");
    assert_eq!(f.key(Key::Tab { back: false }), Event::Redraw);
    assert_eq!(f.focus, Some(2), "OK is the only stop");
}

/// `hit` must not report `Part::Edit` for a disabled field, nor measure text for it;
/// Replace greys both its fields until it has a query.
#[test]
fn a_disabled_edit_hits_as_body() {
    let t = tr();
    let mut f = edit_form();
    let r = f.items[1].rect;
    let (x, y) = (r.x + r.width / 2, r.y + r.height / 2);
    let live = f.hit(x, y, &t, PX).expect("the field is under the pointer");
    assert!(
        matches!(live.part, Part::Edit(_)),
        "an enabled field still reports the byte under the pointer"
    );
    if let Control::Edit { enabled, .. } = &mut f.items[1].control {
        *enabled = false;
    }
    let dead = f
        .hit(x, y, &t, PX)
        .expect("the rect is still under the pointer");
    assert_eq!(dead.item, 1, "the same item is still hit");
    assert_eq!(dead.part, Part::Body);
}
