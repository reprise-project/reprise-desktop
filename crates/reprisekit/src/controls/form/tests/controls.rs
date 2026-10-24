use super::*;

/// A check box's label rect starts at the scaled label x and is the
/// text's device width plus 2 px per scale step, so at scale 2 the
/// focus ring ends 5 px past the 22 px text, not twice its width out.
#[test]
fn a_check_boxs_focus_ring_at_scale_two_spans_the_scaled_label() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let cs = &theme.colors;
    let label = "Display delete confirmation";
    let mut f = Form::new(vec![Item::check("c", 10, 10, label)]);
    f.focus = Some(0);
    // The ring's rightmost inked column at scale `s`.
    let right = |s: i32| {
        let mut c = Canvas::new(600, 80);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(
            &mut c,
            &text,
            &classic_paint(cs, s),
            &f,
            true,
            11 * s as u32,
        );
        (0..600)
            .rev()
            .find(|&x| (0..80).any(|y| c.get(x, y) != cs.button_face.argb()))
            .expect("the check box draws")
    };
    // The ring runs from 1 px left of the label rect to 1 px past it;
    // the rect starts at the scaled label x and is the text plus 2 px.
    let x = 10 + classic_paint(cs, 1).look.controls().check_label;
    for s in [1, 2] {
        let want = s * x + text.measure(label, false, 11 * s as u32) + 3 * s - 1;
        assert_eq!(right(s), want, "the ring ends at x {want} at scale {s}");
    }
}

#[test]
fn a_disabled_check_greys_its_box_not_just_its_label() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let cs = &theme.colors;
    let f = Form::new(vec![Item::new(
        "c",
        Rect::new(10, 10, 13, 13),
        Control::Check {
            label: "Do not move files to the Recycle Bin".into(),
            checked: false,
            enabled: false,
            mixed: None,
        },
    )]);
    let mut c = Canvas::new(240, 40);
    c.fill_rect(c.bounds(), cs.button_face);
    draw(&mut c, &text, &classic_paint(cs, 1), &f, true, 11);
    // Face inside the sunken edge, where an enabled box is window.
    assert_eq!(c.get(13, 13), cs.button_face.argb());
}

#[test]
fn set_track_clamps_to_the_last_tick() {
    let mut f = Form::new(vec![Item::new(
        "t",
        Rect::new(10, 10, 71, trackbar::HEIGHT),
        Control::TrackBar(TrackBar::new(5, 10)),
    )]);
    f.set_track("t", 99);
    let Some(Control::TrackBar(t)) = f.item("t").map(|i| &i.control) else {
        panic!("the only item is the trackbar")
    };
    assert_eq!(t.pos, 9, "the last of 10 ticks");
}

#[test]
fn a_disabled_trackbar_is_not_a_tab_stop() {
    let mut tb = TrackBar::new(5, 10);
    tb.enabled = false;
    let f = Form::new(vec![
        Item::new(
            "t",
            Rect::new(10, 10, 71, trackbar::HEIGHT),
            Control::TrackBar(tb),
        ),
        Item::new(
            "ok",
            Rect::new(10, 50, 75, 23),
            Control::Button {
                label: "OK".into(),
                enabled: true,
                default: true,
            },
        ),
    ]);
    assert_eq!(f.focus, Some(1), "the trackbar is skipped");
}

#[test]
fn an_editing_key_outside_a_field_drives_the_control_it_stood_for() {
    // Arrows reach the form as EditKeys, so a focused combo must
    // still step: the form degrades what is not going into a field.
    let (mut flat, mut rich) = (form(), form());
    flat.focus_id("combo");
    rich.focus_id("combo");
    assert_eq!(flat.key(Key::Right), Event::Changed("combo"));
    assert_eq!(
        rich.key(Key::Edit(EditKey::Right {
            select: false,
            word: false
        })),
        Event::Changed("combo")
    );
    assert_eq!(flat.key(Key::Left), Event::Changed("combo"));
    assert_eq!(
        rich.key(Key::Edit(EditKey::Left {
            select: false,
            word: false
        })),
        Event::Changed("combo")
    );
    assert_eq!(flat, rich, "the degrade left the form in the same state");
    // An editing key with no dialog meaning stays dropped.
    assert_eq!(
        rich.key(Key::Edit(EditKey::Delete { word: false })),
        Event::None
    );
}

#[test]
fn an_editing_key_reaches_an_open_drop_list() {
    // The popup guard returns before the focused control is
    // consulted, so the degrade has to happen above it.
    let (mut flat, mut rich) = (form(), form());
    flat.focus_id("combo");
    rich.focus_id("combo");
    flat.key(Key::Drop);
    rich.key(Key::Drop);
    assert_eq!(flat.key(Key::End), Event::Redraw);
    assert_eq!(
        rich.key(Key::Edit(EditKey::End { select: false })),
        Event::Redraw
    );
    assert!(
        matches!(&rich.items[1].control, Control::Combo(c) if c.open == Some(2)),
        "End walked the open list to its last row"
    );
    assert_eq!(flat, rich);
}

#[test]
fn tab_walks_the_focusable_items_and_keys_reach_the_focused_one() {
    let mut f = form();
    assert_eq!(f.focus, Some(1), "the label is skipped");
    assert_eq!(f.key(Key::Tab { back: false }), Event::Redraw);
    assert_eq!(f.focus, Some(2));
    assert_eq!(f.key(Key::Space), Event::Changed("check"));
    assert!(matches!(
        f.items[2].control,
        Control::Check { checked: true, .. }
    ));
    assert_eq!(f.key(Key::Tab { back: true }), Event::Redraw);
    assert_eq!(f.key(Key::Down), Event::Changed("combo"));
    assert_eq!(f.key(Key::Drop), Event::Redraw);
    assert_eq!(f.key(Key::Down), Event::Redraw, "moves the open highlight");
    assert_eq!(f.key(Key::Enter), Event::Changed("combo"));
    assert!(
        matches!(&f.items[1].control, Control::Combo(c) if c.selected == 2 && c.open.is_none())
    );
    assert_eq!(f.key(Key::Enter), Event::Default);
    assert_eq!(f.key(Key::Escape), Event::Cancel);
    f.focus = Some(4);
    assert_eq!(f.key(Key::Char('p')), Event::Changed("check"));
    assert_eq!(f.key(Key::Char('T')), Event::Redraw);
    assert_eq!(f.focus, Some(1));
    f.focus_id("spin");
    assert_eq!(f.key(Key::Char('4')), Event::Changed("spin"));
    assert!(matches!(&f.items[3].control, Control::UpDown(u) if u.value() == 4));
    assert_eq!(f.key(Key::Up), Event::Changed("spin"));
    f.focus_id("cancel");
    assert_eq!(f.key(Key::Enter), Event::Pressed("cancel"));
}

#[test]
fn a_spinner_takes_a_right_press_and_the_clipboard_keys() {
    let mut f = form();
    let at = |at| {
        Some(Hit {
            item: 3,
            part: Part::UpDown(updown::Hit::Edit(at)),
            x: 0,
        })
    };
    assert_eq!(f.right_press(at(1)), Event::Redraw);
    assert_eq!(f.focus, Some(3), "the press focuses the spinner");
    assert_eq!(
        spin(&f).edit().map(|e| e.selection()),
        Some(0..2),
        "the number focus selected stays selected under the press"
    );
    let Some(FocusedEdit::Line(e)) = f.focused_edit() else {
        panic!("the spinner's digits are the focused edit");
    };
    assert_eq!(FocusedEdit::Line(e).copy_text().as_deref(), Some("10"));
    assert_eq!(f.key(Key::Edit(EditKey::Copy)), Event::Redraw);
    assert_eq!(f.key(Key::Edit(EditKey::Cut)), Event::Changed("spin"));
    assert_eq!(spin(&f).shown(), "");
    assert_eq!(f.paste(" 42\n".into()), Event::Changed("spin"));
    assert_eq!(spin(&f).value(), 42);
    assert_eq!(
        f.paste("abc".into()),
        Event::Redraw,
        "only a number goes in"
    );
    assert_eq!(spin(&f).value(), 42);
    assert_eq!(f.key(Key::Edit(EditKey::Undo)), Event::Changed("spin"));
    assert_eq!(spin(&f).shown(), "");
    assert_eq!(f.right_press(at(0)), Event::Redraw);
    assert_eq!(
        spin(&f).edit().map(|e| (e.selection(), e.caret())),
        Some((0..0, 0)),
        "a press off the selection moves the caret"
    );
}

#[test]
fn a_spinner_left_and_tabbed_back_to_replaces_its_number() {
    let mut f = form();
    f.focus_id("spin");
    f.key(Key::Char('3'));
    f.key(Key::Char('0'));
    assert_eq!(spin(&f).value(), 30);
    f.key(Key::Tab { back: false });
    assert!(!spin(&f).editing(), "leaving drops the typed digits");
    f.key(Key::Tab { back: true });
    assert_eq!(f.key(Key::Char('1')), Event::Changed("spin"));
    assert_eq!((spin(&f).value(), spin(&f).shown().as_str()), (1, "1"));
    // Digits past the range clamp, and leaving shows the clamped value.
    f.key(Key::Char('2'));
    f.key(Key::Char('3'));
    assert_eq!((spin(&f).value(), spin(&f).shown().as_str()), (99, "123"));
    f.set_focus(None);
    assert_eq!(spin(&f).shown(), "99");
}

#[test]
fn tabbing_into_a_spinner_selects_its_number() {
    let mut f = form();
    f.focus_id("check");
    f.key(Key::Tab { back: false });
    assert_eq!(f.focus(), f.index_of("spin"));
    assert!(spin(&f).editing());
    // A host's editing keys: the digit replaces the selection, Left
    // steps the caret back over it, and the next digit goes in there.
    let ins = |s: &str| Key::Edit(EditKey::Insert(s.into()));
    assert_eq!(f.key(ins("5")), Event::Changed("spin"));
    assert_eq!(f.key(ins("7")), Event::Changed("spin"));
    let left = EditKey::Left {
        select: false,
        word: false,
    };
    assert_eq!(f.key(Key::Edit(left)), Event::Redraw);
    assert_eq!(f.key(ins("1")), Event::Changed("spin"));
    assert_eq!(spin(&f).shown(), "517");
    assert_eq!(spin(&f).value(), 99);
    // A letter is no digit: it is the check box's access key.
    assert_eq!(f.key(ins("p")), Event::Changed("check"));
}

#[test]
fn clicking_a_spinner_puts_the_caret_under_the_pointer() {
    let mut f = form();
    // The field's white part runs from x 12; "10" sits right-aligned
    // against x 41, so x 13 is before its first digit.
    let h = f.hit(13, 90, &tr(), PX).unwrap();
    assert_eq!(h.part, Part::UpDown(updown::Hit::Edit(0)));
    assert_eq!(f.click(Some(h)), Event::Redraw);
    assert_eq!(f.focus(), f.index_of("spin"));
    f.key(Key::Char('2'));
    assert_eq!(spin(&f).shown(), "210", "typed in front, not over it");
    // Its up arrow steps the value and selects it for the next digit.
    let up = f.hit(50, 83, &tr(), PX).unwrap();
    assert_eq!(f.click(Some(up)), Event::Redraw, "already at the top");
    f.focus_id("ok");
    f.click(Some(up));
    assert_eq!(spin(&f).shown(), "99");
    f.key(Key::Char('4'));
    assert_eq!(spin(&f).value(), 4);
}

#[test]
fn clicks_land_on_parts_and_popups_cover_the_rest() {
    let mut f = form();
    let h = f.hit(20, 65, &tr(), PX).unwrap();
    assert_eq!((h.item, h.part), (2, Part::Body));
    assert_eq!(f.click(Some(h)), Event::Redraw);
    assert_eq!(f.release(), Event::Changed("check"));
    let h = f.hit(60, 65, &tr(), PX).unwrap();
    assert_eq!(h.item, 2);
    let drop = f.hit(150, 40, &tr(), PX).unwrap();
    assert_eq!(drop.part, Part::Combo(combo::Hit::Drop));
    assert_eq!(f.click(Some(drop)), Event::Redraw);
    let row = f.hit(20, 51 + 1 + 13 + 5, &tr(), PX).unwrap();
    assert_eq!((row.item, row.part), (1, Part::Combo(combo::Hit::Row(1))));
    assert_eq!(f.click(Some(row)), Event::Changed("combo"));
    f.key(Key::Drop);
    assert_eq!(f.click(None), Event::Redraw);
    let ok = f.hit(20, 120, &tr(), PX).unwrap();
    assert_eq!(f.click(Some(ok)), Event::Redraw);
    assert_eq!(f.held_item(), Some(ok.item));
    assert_eq!(f.lit(ok.item, Light::Body), Interaction::Pressed);
    assert_eq!(f.release(), Event::Pressed("ok"));
    assert!(!f.holding());
    assert_eq!(f.click(Some(ok)), Event::Redraw);
    assert_eq!(f.motion(300, 300, None), Event::Redraw);
    assert_eq!(f.held_item(), Some(ok.item));
    assert_eq!(f.lit(ok.item, Light::Body), Interaction::Idle);
    assert_eq!(f.release(), Event::Redraw);
    assert!(!f.holding());
}

#[test]
fn a_rule_is_a_solid_shadow_rect_nothing_can_hit() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let cs = &theme.colors;
    let f = Form::new(vec![Item::new(
        "rule",
        Rect::new(20, 10, 2, 16),
        Control::Rule,
    )]);
    let mut c = Canvas::new(60, 40);
    c.fill_rect(c.bounds(), cs.button_face);
    draw(&mut c, &text, &classic_paint(cs, 1), &f, false, 11);
    assert_eq!(c.get(20, 10), cs.button_shadow.argb());
    assert_eq!(c.get(21, 25), cs.button_shadow.argb());
    assert_eq!(c.get(22, 25), cs.button_face.argb(), "2 px wide");
    assert_eq!(c.get(20, 26), cs.button_face.argb(), "16 px tall");
    assert_eq!(f.hit(21, 20, &text, 11), None, "a rule is not a target");
}

#[test]
fn draws_focus_and_the_default_ring() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let cs = &theme.colors;
    let mut f = form();
    let mut c = Canvas::new(200, 150);
    c.fill_rect(c.bounds(), cs.button_face);
    draw(&mut c, &text, &classic_paint(cs, 1), &f, true, 11);
    // OK wears the default ring while the combo has the focus.
    assert_eq!(c.get(10, 115), cs.button_text.argb());
    assert_eq!(c.get(90, 115), cs.button_hilight.argb());
    f.focus_id("cancel");
    let mut c2 = Canvas::new(200, 150);
    draw(&mut c2, &text, &classic_paint(cs, 1), &f, true, 11);
    assert_eq!(c2.get(90, 115), cs.button_text.argb());
    assert_eq!(c2.get(10, 115), cs.button_hilight.argb());
    // The dotted rectangle stays hidden until the keyboard is used,
    // so the same focus without cues keeps the ring and drops the
    // dots.
    let ring = Rect::new(
        90,
        110,
        crate::classic::widgets::BUTTON_W,
        crate::classic::widgets::BUTTON_H,
    )
    .inset(4);
    let dotted =
        |c: &Canvas| (ring.x..ring.right()).any(|x| c.get(x, ring.y) == cs.button_text.argb());
    assert!(dotted(&c2), "with cues Cancel is dotted");
    let mut c3 = Canvas::new(200, 150);
    c3.fill_rect(c3.bounds(), cs.button_face);
    draw(&mut c3, &text, &classic_paint(cs, 1), &f, false, 11);
    assert_eq!(c3.get(90, 115), cs.button_text.argb(), "the ring stays");
    assert!(!dotted(&c3), "the dots wait for a key");
}

#[test]
fn hidden_access_keys_drop_only_the_underline_in_every_look() {
    use crate::interface::Interface;
    let text = TextRenderer::load(&FontConfig::default());
    const LABELS: [&str; 6] = [
        "&Name:",
        "&Group",
        "&Check",
        "&Radio",
        "&Browse...",
        "&Greyed",
    ];
    // Only the label at `marked` keeps its access key.
    let form = |marked: usize| {
        let l = |k: usize| match k == marked {
            true => LABELS[k].to_string(),
            false => LABELS[k].replace('&', ""),
        };
        Form::new(vec![
            Item::label("label", Rect::new(10, 6, 80, 13), &l(0)),
            Item::group("group", Rect::new(6, 26, 180, 66), &l(1)),
            Item::check("check", 16, 42, &l(2)),
            Item::radio("radio", 16, 62, &l(3), "r"),
            Item::button("on", Rect::new(10, 100, 75, 23), &l(4), true, false),
            Item::button("off", Rect::new(95, 100, 75, 23), &l(5), false, false),
        ])
    };
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = crate::interfaces::theme_for(i);
        let drawn = |f: &Form, keys: bool| {
            let p = Paint {
                access_keys: keys,
                ..theme.paint()
            };
            let mut c = Canvas::new(200, 130);
            c.fill_rect(c.bounds(), p.cs.button_face);
            draw(&mut c, &text, &p, f, false, 11);
            c
        };
        for (k, label) in LABELS.iter().enumerate() {
            let f = form(k);
            let (hidden, shown) = (drawn(&f, false), drawn(&f, true));
            let rows: std::collections::BTreeSet<i32> = (0..130)
                .flat_map(|y| (0..200).map(move |x| (x, y)))
                .filter(|&(x, y)| hidden.get(x, y) != shown.get(x, y))
                .map(|(_, y)| y)
                .collect();
            // An etched (greyed Classic) underline takes its highlight's row too.
            let (first, last) = (rows.first().copied(), rows.last().copied());
            assert!(
                first.is_some_and(|f| last.is_some_and(|l| l - f <= 1)),
                "{i:?}: {label} differs by its underline alone, hidden or shown: rows {rows:?}"
            );
        }
    }
}

#[test]
fn an_edit_fills_by_its_state_and_blinks_only_when_focused() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let cs = &theme.colors;
    let mut f = edit_form();
    // The field is (80, 10, 100, 20) and the pen starts FIELD_PAD
    // inside it, so an empty field's caret is the column at x 83 and
    // (120, 20) is clear interior. The text is empty, so no glyph can
    // land on either sample.
    let (caret_x, caret_y) = (83, 15);
    let (fill_x, fill_y) = (120, 20);
    let shot = |f: &Form| {
        let mut c = Canvas::new(200, 80);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &text, &classic_paint(cs, 1), f, true, 11);
        c
    };
    assert_eq!(f.focus, Some(1));
    assert!(f.caret, "a new form starts with the caret shown");
    let c = shot(&f);
    assert_eq!(c.get(fill_x, fill_y), cs.window.argb());
    assert_eq!(c.get(caret_x, caret_y), cs.button_text.argb());
    f.caret = false;
    let c = shot(&f);
    assert_eq!(c.get(caret_x, caret_y), cs.window.argb(), "no caret");
    assert_eq!(c.get(fill_x, fill_y), cs.window.argb());
    f.caret = true;
    f.focus_id("ok");
    let c = shot(&f);
    assert_eq!(
        c.get(caret_x, caret_y),
        cs.window.argb(),
        "an unfocused field does not blink"
    );
    if let Control::Edit { enabled, .. } = &mut f.items[1].control {
        *enabled = false;
    }
    f.focus = Some(1);
    let c = shot(&f);
    assert_eq!(c.get(fill_x, fill_y), cs.button_face.argb());
    assert_eq!(
        c.get(caret_x, caret_y),
        cs.button_face.argb(),
        "a disabled field does not blink"
    );
}

/// An Open With list: a letter jumps to the next row starting with
/// it (the focus stays on the list), Enter is the default button and
/// a double click on a row is Enter.
#[test]
fn a_list_box_takes_type_ahead_and_a_double_click_is_the_default() {
    let names = ["Write", "Notepad", "Picture Editor"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut f = Form::new(vec![
        Item::new(
            "apps",
            Rect::new(10, 10, 100, 60),
            Control::ListBox {
                list: ListBox::new(names, Some(0)),
                icons: vec![None; 3],
            },
        ),
        Item::new(
            "ok",
            Rect::new(10, 80, 75, 23),
            Control::Button {
                label: "OK".into(),
                enabled: true,
                default: true,
            },
        ),
    ]);
    f.focus_id("apps");
    assert_eq!(f.key(Key::Char('p')), Event::Changed("apps"));
    let list = |f: &Form| match &f.items[0].control {
        Control::ListBox { list, .. } => list.selected,
        _ => unreachable!(),
    };
    assert_eq!(list(&f), Some(2));
    assert_eq!(f.key(Key::Char('z')), Event::None);
    assert_eq!(f.key(Key::Enter), Event::Default);
    let row = f.hit(30, 12 + 17, &tr(), PX).unwrap();
    assert_eq!(row.part, Part::List(listbox::Hit::Row(1)));
    assert_eq!(f.double_click(Some(row)), Event::Default);
    assert_eq!(list(&f), Some(1));
}

#[test]
fn a_readers_focus_moves_to_a_button_or_check_box_without_acting() {
    let mut f = Form::new(vec![
        Item::new(
            "wrap",
            Rect::new(10, 10, 100, 17),
            Control::Check {
                label: "Wrap".into(),
                checked: false,
                enabled: true,
                mixed: None,
            },
        ),
        Item::new(
            "ok",
            Rect::new(10, 40, 75, 23),
            Control::Button {
                label: "OK".into(),
                enabled: true,
                default: true,
            },
        ),
        Item::new(
            "off",
            Rect::new(10, 70, 75, 23),
            Control::Button {
                label: "Apply".into(),
                enabled: false,
                default: false,
            },
        ),
    ]);
    f.focus_id("wrap");
    assert_eq!(f.focus_at(40, 50, &tr(), PX), Event::Redraw);
    assert_eq!(f.focus(), Some(1));
    assert_eq!(f.focus_at(40, 50, &tr(), PX), Event::None, "already there");
    assert_eq!(
        f.focus_at(40, 80, &tr(), PX),
        Event::None,
        "a greyed button"
    );
    assert_eq!(f.focus(), Some(1));
    assert_eq!(f.focus_at(14, 18, &tr(), PX), Event::Redraw);
    assert_eq!(f.focus(), Some(0));
    assert!(
        matches!(f.items[0].control, Control::Check { checked: false, .. }),
        "the box is not flipped"
    );
}

#[test]
fn a_reader_scrolls_a_list_box_row_in_without_picking_it() {
    let names = (0..20).map(|i| format!("Row {i}")).collect();
    let mut f = Form::new(vec![Item::new(
        "rows",
        Rect::new(10, 10, 100, 60),
        Control::ListBox {
            list: ListBox::new(names, Some(0)),
            icons: vec![None; 20],
        },
    )]);
    use crate::access::{Published, ROOT, Tree};
    let mut tree = Tree::window("", (200, 100));
    f.access(&mut tree, ROOT, &tr(), PX);
    let shown = tree.find(Role::ListBoxOption, "Row 1").unwrap();
    let below = tree.find(Role::ListBoxOption, "Row 12").unwrap();
    let mut p = Published::default();
    p.update(tree);
    assert_eq!(p.reveal(shown), None, "a row in view needs no scroll");
    assert_eq!(p.target(below), None, "a row out of view takes no click");
    let reveal = p.reveal(below).expect("a row below the frame");
    assert_eq!(reveal.row, 12);
    let (x, y) = (reveal.at.0 as i32, reveal.at.1 as i32);
    assert_eq!(f.reveal(x, y, reveal.row, &tr(), PX), Event::Redraw);
    let list = match &f.items[0].control {
        Control::ListBox { list, .. } => list,
        _ => unreachable!(),
    };
    let visible = listbox::layout(list, f.at(&f.items[0]), &f.rendered.controls.get()).visible;
    assert_eq!(list.top + visible, 13, "row 12 is the last row shown");
    assert_eq!(list.selected, Some(0), "the pick stays");
    assert_eq!(f.reveal(x, y, 12, &tr(), PX), Event::None);
}

/// A list with check boxes: a click on a row's box selects the row
/// and flips it, a click on its label only selects, and Space flips
/// the selected row.
#[test]
fn a_checked_list_box_flips_on_its_box_and_on_space() {
    let names = ["Volume Control", "Firefox"].map(String::from).to_vec();
    let mut list = ListBox::new(names, Some(0));
    list.checks = vec![true, true];
    let mut f = Form::new(vec![Item::new(
        "shown",
        Rect::new(10, 10, 150, 60),
        Control::ListBox {
            list,
            icons: vec![None; 2],
        },
    )]);
    f.focus_id("shown");
    let state = |f: &Form| match &f.items[0].control {
        Control::ListBox { list, .. } => (list.selected, list.checks.clone()),
        _ => unreachable!(),
    };
    let box1 = f.hit(12 + 3 + 5, 12 + 17 + 2 + 5, &tr(), PX).unwrap();
    assert_eq!(box1.part, Part::List(listbox::Hit::Check(1)));
    assert_eq!(f.click(Some(box1)), Event::Changed("shown"));
    assert_eq!(state(&f), (Some(1), vec![true, false]));
    let label0 = f.hit(60, 12 + 5, &tr(), PX).unwrap();
    assert_eq!(f.click(Some(label0)), Event::Changed("shown"));
    assert_eq!(state(&f), (Some(0), vec![true, false]), "only selected");
    assert_eq!(f.key(Key::Space), Event::Changed("shown"));
    assert_eq!(state(&f), (Some(0), vec![false, false]));
}

#[test]
fn the_calendar_and_time_field_take_clicks_and_keys() {
    let text = TextRenderer::load(&FontConfig::default());
    let mut f = Form::new(vec![
        Item::new(
            "cal",
            Rect::new(27, 87, calendar::W, calendar::H),
            Control::Calendar(Calendar::new(2026, 9, Some(24), 0)),
        ),
        Item::new(
            "time",
            Rect::new(249, 198, 103, 20),
            Control::Time(TimeField::new(23, 41, 24, true)),
        ),
    ]);
    let day = |f: &Form| match &f.items[0].control {
        Control::Calendar(c) => c.day,
        _ => None,
    };
    // The 1st of September 2026 is a Tuesday: row 0, column 2.
    let hit = f.hit(84, 112, &text, 11);
    assert_eq!(f.click(hit), Event::Changed("cal"));
    assert_eq!(day(&f), Some(1));
    assert_eq!(f.key(Key::Down), Event::Changed("cal"));
    assert_eq!(day(&f), Some(8));
    let hit = f.hit(340, 200, &text, 11);
    assert_eq!(f.click(hit), Event::Changed("time"));
    assert_eq!(f.focus, Some(1));
    let hour = |f: &Form| match &f.items[1].control {
        Control::Time(t) => t.h,
        _ => 99,
    };
    assert_eq!(hour(&f), 12, "11 PM steps to 12 PM, inside its half");
    assert_eq!(f.key(Key::Char('3')), Event::Changed("time"));
    assert_eq!(hour(&f), 15);
    f.set_enabled("cal", false);
    let hit = f.hit(84, 130, &text, 11);
    assert_eq!(f.click(hit), Event::None, "a greyed calendar picks nothing");
}

/// The tree a form gives a screen reader: its group holds the radios
/// inside it, a field is labelled by the text before it, labels lose
/// their `&`, a check box carries its state and the focused field is
/// the focus.
#[test]
fn a_forms_tree_nests_labels_and_reports_state_and_focus() {
    use crate::access::{ROOT, Tree};
    let mut form = Form::new(vec![
        Item::label_at("namelabel", 10, 10, 80, "&Name:"),
        Item::edit("name", Rect::new(10, 26, 120, 20), "abc", true),
        Item::group("dir", Rect::new(6, 50, 200, 60), "Direction"),
        Item::radio("up", 16, 70, "&Up", "dir").checked(true),
        Item::check("case", 10, 120, "Match &case").checked(true),
        Item::button("ok", Rect::new(10, 140, 75, 23), "OK", false, true),
    ]);
    form.focus_id("name");
    let text = tr();
    let mut tree = Tree::window("Find", (220, 170));
    let focus = form.access(&mut tree, ROOT, &text, 11);

    let edit = tree
        .nodes()
        .find(|(_, n)| n.role() == Role::TextInput)
        .map(|(id, _)| id)
        .expect("a text input");
    assert_eq!(focus, Some(edit), "the focused field is the focus");
    let label = tree.get(edit).unwrap().labelled_by()[0];
    assert_eq!(tree.get(label).unwrap().value(), Some("Name:"));

    let group = tree.find(Role::Group, "Direction").expect("the group");
    let up = tree.find(Role::RadioButton, "Up").expect("the radio");
    assert!(tree.get(group).unwrap().children().contains(&up));
    assert_eq!(
        tree.get(up).unwrap().toggled(),
        Some(accesskit::Toggled::True)
    );

    let case = tree.find(Role::CheckBox, "Match case").expect("the check");
    assert_eq!(
        tree.get(case).unwrap().toggled(),
        Some(accesskit::Toggled::True)
    );
    assert!(tree.find(Role::DefaultButton, "OK").is_some());
}

/// A notch over a Change Icon list scrolls it one column, down to the
/// right, held to the ends; the pick stays.
#[test]
fn the_wheel_scrolls_an_icon_list_a_column_a_notch() {
    let n = 40;
    let icons = iconlist::IconList::new(
        (0..n).map(|k| format!("i{k}")).collect(),
        vec![None; n],
        Some(0),
    );
    let r = Rect::new(0, 0, 267, 198);
    let mut f = Form::new(vec![Item::new(
        "icons",
        r,
        Control::IconList(Box::new(icons)),
    )]);
    let on = at(&f, 20, 20);
    let left = |f: &Form| match &f.items[0].control {
        Control::IconList(l) => (l.left, l.selected),
        _ => unreachable!(),
    };
    assert_eq!(f.wheel(on, WHEEL_ROWS), Event::Redraw);
    assert_eq!(left(&f), (1, Some(0)), "one notch, one column");
    assert_eq!(f.wheel(on, 2 * WHEEL_ROWS), Event::Redraw);
    assert_eq!(left(&f), (3, Some(0)), "ten columns, seven in view");
    assert_eq!(f.wheel(on, WHEEL_ROWS), Event::None, "at the last column");
    assert_eq!(f.wheel(on, -WHEEL_ROWS), Event::Redraw);
    assert_eq!(left(&f), (2, Some(0)));
    assert_eq!(f.wheel(None, -WHEEL_ROWS), Event::None, "off the list");
}
