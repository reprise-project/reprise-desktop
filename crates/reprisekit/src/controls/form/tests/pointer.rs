use super::*;

/// [`form`] with a trackbar under its buttons.
fn lit_form() -> Form {
    let mut f = form();
    f.items.push(Item::new(
        "track",
        Rect::new(10, 150, 100, trackbar::HEIGHT),
        Control::TrackBar(TrackBar::new(3, 10)),
    ));
    f
}

/// A point on `part` of item `i`, scanning the item's rect.
fn point_on(f: &Form, i: usize, part: Part) -> (i32, i32) {
    let r = f.items[i].rect;
    (r.y..r.bottom())
        .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
        .find(|&(x, y)| at(f, x, y).is_some_and(|h| h.item == i && h.part == part))
        .expect("the part is on the item")
}

#[test]
fn under_blue_a_motion_lights_the_control_under_it_and_a_leave_clears_it() {
    let mut f = lit_form();
    drawn(&f, &blue());
    let (ok, check) = (f.index_of("ok").unwrap(), f.index_of("check").unwrap());
    assert_eq!(f.motion(20, 120, at(&f, 20, 120)), Event::Redraw);
    assert_eq!(f.lit(ok, Light::Body), Interaction::Hot);
    assert_eq!(
        f.motion(30, 121, at(&f, 30, 121)),
        Event::None,
        "a move within the button changes nothing"
    );
    assert_eq!(f.motion(60, 65, at(&f, 60, 65)), Event::Redraw);
    assert_eq!(f.lit(check, Light::Body), Interaction::Hot);
    assert_eq!(f.lit(ok, Light::Body), Interaction::Idle);
    assert_eq!(f.leave(), Event::Redraw);
    assert_eq!(f.lit(check, Light::Body), Interaction::Idle);
    assert_eq!(f.leave(), Event::None, "nothing left to clear");
}

/// An editable Blue combo lights its drop button only from the button:
/// with the pointer on its field the button draws at rest, on the
/// button it draws hot.
#[test]
fn an_editable_blue_combo_lights_its_drop_only_from_the_drop() {
    let mut cb = Combo::new(&["a", "b"], 0, 4);
    cb.edit = Some(TextEdit::new("text", false));
    let mut f = Form::new(vec![Item::new(
        "c",
        Rect::new(10, 10, 140, combo::COMBO_H),
        Control::Combo(cb),
    )]);
    let theme = blue();
    drawn(&f, &theme);
    let lay = combo::layout(
        match &f.items[0].control {
            Control::Combo(c) => c,
            _ => unreachable!(),
        },
        f.items[0].rect,
        &theme.look.controls(),
    );
    let drop_at = |ptr| {
        let mut c = Canvas::new(200, 200);
        c.fill_rect(c.bounds(), theme.colors.window);
        crate::look::widgets::draw_combo_drop(&mut c, &theme.paint(), lay.drop, true, ptr, true);
        c
    };
    let same = |a: &Canvas, b: &Canvas| {
        let r = lay.drop;
        (r.y..r.bottom()).all(|y| (r.x..r.right()).all(|x| a.get(x, y) == b.get(x, y)))
    };
    let (fx, fy) = point_on(&f, 0, Part::Combo(combo::Hit::Field));
    f.motion(fx, fy, at(&f, fx, fy));
    assert_eq!(f.lit(0, Light::Field), Interaction::Hot, "the field lights");
    assert!(
        same(&drawn(&f, &theme), &drop_at(Interaction::Idle)),
        "with the pointer on the field the drop draws Normal"
    );
    let (dx, dy) = point_on(&f, 0, Part::Combo(combo::Hit::Drop));
    f.motion(dx, dy, at(&f, dx, dy));
    assert!(
        same(&drawn(&f, &theme), &drop_at(Interaction::Hot)),
        "with the pointer on the drop it draws Hot"
    );
}

#[test]
fn under_classic_hover_asks_no_redraw_but_a_held_button_dragged_off_does() {
    let mut f = lit_form();
    drawn(&f, &Theme::default());
    assert_eq!(f.motion(20, 120, at(&f, 20, 120)), Event::None);
    assert_eq!(f.leave(), Event::None);
    let ok = at(&f, 20, 120);
    assert_eq!(f.click(ok), Event::Redraw);
    assert_eq!(f.motion(300, 300, None), Event::Redraw, "dragged off");
    assert_eq!(f.motion(20, 120, ok), Event::Redraw, "back on");
    assert_eq!(f.release(), Event::Pressed("ok"));
}

/// Hot outranks the default ring; a held button dragged off draws as
/// at rest, and pushed again back on.
#[test]
fn a_blue_form_draws_the_hot_button_and_a_held_one_dragged_off_at_rest() {
    let theme = blue();
    let mut f = lit_form();
    let rest = drawn(&f, &theme);
    let ok = at(&f, 20, 120);
    f.motion(20, 120, ok);
    let hot = drawn(&f, &theme);
    for (x, y) in [(7, 2), (3, 10)] {
        let (h, r) = (hot.get(10 + x, 110 + y), rest.get(10 + x, 110 + y));
        assert_ne!(h, r, "the hot ring lights ({x}, {y})");
    }
    f.click(ok);
    let held = drawn(&f, &theme);
    let (h, r) = (held.get(10 + 10, 110 + 3), rest.get(10 + 10, 110 + 3));
    assert!(h & 0xFF < r & 0xFF, "the pushed face is darker at its top");
    f.motion(300, 300, None);
    let off = drawn(&f, &theme);
    let r = f.items[f.index_of("ok").unwrap()].rect;
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            assert_eq!(off.get(x, y), rest.get(x, y), "({x}, {y}) is at rest");
        }
    }
    f.motion(20, 120, ok);
    assert!(drawn(&f, &theme) == held, "back on, it is pushed again");
}

/// A Blue check box lights from its label; held, it draws its Pressed
/// frame with no tick yet, and the release over it ticks it and leaves
/// it hot.
#[test]
fn a_blue_check_box_draws_pressed_while_held_and_ticks_on_the_release() {
    let theme = blue();
    let mut f = lit_form();
    drawn(&f, &theme);
    use crate::style::{Check, Part, State};
    let box_px = |c: &Canvas, x: i32, y: i32| c.get(10 + x, 60 + y);
    let frame = |check: Check, state: State, x: i32, y: i32| {
        let mut c = Canvas::new(13, 13);
        let r = Rect::new(0, 0, 13, 13);
        crate::blue::style().draw_check(&mut c, r, Part::CheckBox, check, state, 1);
        c.get(x, y)
    };
    f.motion(60, 65, at(&f, 60, 65));
    let hot = drawn(&f, &theme);
    let lit = frame(Check::Unchecked, State::Hot, 1, 1);
    assert_eq!(box_px(&hot, 1, 1), lit, "the orange ring");
    assert_eq!(
        box_px(&hot, 6, 6),
        frame(Check::Unchecked, State::Hot, 6, 6),
        "no tick yet"
    );
    assert_eq!(f.click(at(&f, 60, 65)), Event::Redraw);
    let held = drawn(&f, &theme);
    assert_eq!(
        box_px(&held, 1, 1),
        frame(Check::Unchecked, State::Pressed, 1, 1),
        "the pressed well"
    );
    assert_eq!(
        box_px(&held, 6, 6),
        frame(Check::Unchecked, State::Pressed, 6, 6),
        "the pressed fill, no tick"
    );
    assert_eq!(f.release(), Event::Changed("check"));
    let after = drawn(&f, &theme);
    assert_eq!(box_px(&after, 1, 1), lit, "hot again");
    let tick = crate::blue::Variant::Blue
        .scheme()
        .check_box
        .normal_checked
        .mark;
    assert_eq!(
        box_px(&after, 6, 7),
        0xFF00_0000 | tick.unwrap().argb(),
        "the tick"
    );
}

/// Check boxes and radios act on the release, as push buttons do, in
/// every look: the press holds one, a release over it toggles it, a
/// release after dragging off changes nothing, and dragging back on
/// arms it again. A radio already picked stays picked; a greyed box
/// takes neither the press nor the release.
#[test]
fn a_check_box_and_a_radio_toggle_on_the_release_over_them_in_every_look() {
    use crate::interface::Interface;
    for look in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = crate::interfaces::theme_for(look);
        let mut f = Form::new(vec![
            Item::check("check", 10, 10, "&Check"),
            Item::radio("up", 10, 40, "&Up", "dir"),
            Item::radio("down", 10, 70, "&Down", "dir").checked(true),
        ]);
        drawn(&f, &theme);
        for id in ["check", "up"] {
            let i = f.index_of(id).unwrap();
            let (x, y) = point_on(&f, i, Part::Body);
            let on = at(&f, x, y);
            let was = f.checked(id).unwrap();
            assert_eq!(f.click(on), Event::Redraw, "{look:?} {id}: held");
            assert_eq!(f.lit(i, Light::Body), Interaction::Pressed);
            assert_eq!(f.checked(id), Some(was), "{look:?} {id}: the press waits");
            f.motion(300, 300, None);
            assert_eq!(f.lit(i, Light::Body), Interaction::Idle, "dragged off");
            assert_eq!(f.release(), Event::Redraw, "{look:?} {id}: released off");
            assert_eq!(f.checked(id), Some(was), "{look:?} {id}: unchanged");
            f.click(on);
            f.motion(300, 300, None);
            f.motion(x, y, on);
            assert_eq!(f.lit(i, Light::Body), Interaction::Pressed, "back on");
            assert_eq!(f.release(), Event::Changed(id), "{look:?} {id}");
            assert_eq!(f.checked(id), Some(!was), "{look:?} {id}: toggled");
            assert_eq!(f.lit(i, Light::Body), Interaction::Hot, "{look:?} {id}");
        }
        assert_eq!(f.checked("down"), Some(false), "{look:?}: up cleared down");
        let up = at(&f, 12, 42);
        f.click(up);
        assert_eq!(f.release(), Event::Redraw, "{look:?}: up is already picked");
        assert_eq!(f.checked("up"), Some(true), "{look:?}: up stays picked");
        f.set_enabled("check", false);
        let check = at(&f, 12, 12);
        assert_eq!(f.click(check), Event::None, "{look:?}: greyed");
        assert!(!f.holding(), "{look:?}: a greyed box is not held");
        assert_eq!(f.release(), Event::None, "{look:?}: greyed");
        assert_eq!(
            f.checked("check"),
            Some(true),
            "{look:?}: greyed, unchanged"
        );
    }
}

/// A spinner's button lights alone, hot then pressed while held.
#[test]
fn a_blue_spin_button_lights_hot_and_pressed() {
    let theme = blue();
    let mut f = lit_form();
    drawn(&f, &theme);
    let i = f.index_of("spin").unwrap();
    let (x, y) = point_on(&f, i, Part::UpDown(updown::Hit::Up));
    assert_eq!(f.motion(x, y, at(&f, x, y)), Event::Redraw);
    assert_eq!(f.spin_lit(i), Some((Arrow::Up, Interaction::Hot)));
    let up = match &f.items[i].control {
        Control::UpDown(u) => {
            updown::layout_for(u, f.items[i].rect, &theme.look.controls(), TextUnits::ONE).up
        }
        _ => unreachable!("spin is an up-down"),
    };
    let edge = |state| {
        let st = crate::blue::style();
        let img = st.part(crate::style::Part::SpinUp).unwrap();
        let f = img.frame(state);
        img.atlas.get(f.x, f.y + 3)
    };
    let hot = drawn(&f, &theme);
    assert_eq!(
        hot.get(up.x, up.y + 3),
        edge(crate::style::State::Hot),
        "the hot button's edge"
    );
    f.click(at(&f, x, y));
    assert_eq!(f.spin_lit(i), Some((Arrow::Up, Interaction::Pressed)));
    let held = drawn(&f, &theme);
    assert_eq!(
        held.get(up.x, up.y + 3),
        edge(crate::style::State::Pressed),
        "the held button's edge"
    );
    assert_eq!(f.release(), Event::Redraw, "back to hot");
    assert_eq!(f.spin_lit(i), Some((Arrow::Up, Interaction::Hot)));
}

/// A held trackbar thumb stays pressed wherever the pointer drags it.
#[test]
fn a_held_thumb_stays_pressed_off_it_until_the_release() {
    let mut f = lit_form();
    drawn(&f, &blue());
    let i = f.index_of("track").unwrap();
    let (x, y) = point_on(&f, i, Part::TrackBar(trackbar::Hit::Thumb));
    f.motion(x, y, at(&f, x, y));
    assert_eq!(f.lit(i, Light::Thumb), Interaction::Hot);
    f.click(at(&f, x, y));
    assert_eq!(f.lit(i, Light::Thumb), Interaction::Pressed);
    f.motion(x, 300, None);
    assert_eq!(
        f.lit(i, Light::Thumb),
        Interaction::Pressed,
        "off the thumb"
    );
    f.release();
    assert_eq!(f.lit(i, Light::Thumb), Interaction::Idle, "released off it");
}

#[test]
fn a_trackbar_thumb_moved_by_keys_or_the_host_follows_the_pointer() {
    let mut f = lit_form();
    drawn(&f, &blue());
    let i = f.index_of("track").unwrap();
    f.set_focus(Some(i));
    let (x, y) = point_on(&f, i, Part::TrackBar(trackbar::Hit::Thumb));
    f.motion(x, y, at(&f, x, y));
    assert_eq!(f.lit(i, Light::Thumb), Interaction::Hot);
    // A position is 9 or 10 px; the pointer is on the thumb's left
    // column.
    assert_eq!(f.key(Key::Right), Event::Changed("track"));
    assert_eq!(f.lit(i, Light::Thumb), Interaction::Idle, "moved off it");
    assert_eq!(f.key(Key::Left), Event::Changed("track"));
    assert_eq!(f.lit(i, Light::Thumb), Interaction::Hot, "back under it");
    f.set_track("track", 5);
    assert_eq!(f.lit(i, Light::Thumb), Interaction::Idle);
    f.set_track("track", 3);
    assert_eq!(f.lit(i, Light::Thumb), Interaction::Hot);
    f.leave();
    f.set_track("track", 4);
    f.set_track("track", 3);
    assert_eq!(f.lit(i, Light::Thumb), Interaction::Idle);
}

#[test]
fn carry_keeps_the_controls_and_focus_and_takes_the_new_text() {
    let built = |line: &str| {
        Form::new(vec![
            Item::label_at("line", 10, 10, 200, line),
            Item::check("check", 10, 30, "&Remember"),
            Item::button("ok", Rect::new(10, 60, 75, 23), "OK", true, true),
        ])
    };
    let mut old = built("One long line");
    old.set_check("check", true);
    old.focus_id("ok");
    let mut new = built("One long");
    new.carry(&old);
    assert_eq!(new.checked("check"), Some(true));
    assert_eq!(new.focus(), new.index_of("ok"));
    assert_eq!(
        new.item("line").map(|i| &i.control),
        built("One long").item("line").map(|i| &i.control)
    );
}

/// A host rebuilds the open page's `Form` on every model change. `carry_pointer` puts back
/// what the old one held, so a held check box, spin button or thumb keeps drawing pressed
/// and its release still resolves.
#[test]
fn carry_pointer_keeps_a_held_check_spin_and_thumb_pressed_across_a_rebuild() {
    let theme = blue();

    let mut f = lit_form();
    drawn(&f, &theme);
    let check = f.index_of("check").unwrap();
    assert_eq!(f.click(at(&f, 60, 65)), Event::Redraw);
    let mut rebuilt = lit_form();
    rebuilt.carry_pointer(&f);
    drawn(&rebuilt, &theme);
    assert_eq!(rebuilt.held_item(), Some(check));
    assert_eq!(rebuilt.lit(check, Light::Body), Interaction::Pressed);
    assert_eq!(rebuilt.release(), Event::Changed("check"));
    assert_eq!(rebuilt.checked("check"), Some(true));
    assert_eq!(rebuilt.lit(check, Light::Body), Interaction::Hot);

    let mut f = lit_form();
    drawn(&f, &theme);
    let spin_i = f.index_of("spin").unwrap();
    let (x, y) = point_on(&f, spin_i, Part::UpDown(updown::Hit::Up));
    f.motion(x, y, at(&f, x, y));
    assert_eq!(f.click(at(&f, x, y)), Event::Changed("spin"));
    assert_eq!(spin(&f).value(), 11);
    // A real host rebuilds the page from the model the press just
    // changed, so the rebuilt spin already reads the stepped value.
    let mut rebuilt = lit_form();
    rebuilt.items[spin_i].control = f.items[spin_i].control.clone();
    rebuilt.carry_pointer(&f);
    drawn(&rebuilt, &theme);
    assert_eq!(
        rebuilt.spin_lit(spin_i),
        Some((Arrow::Up, Interaction::Pressed))
    );
    assert_eq!(rebuilt.release(), Event::Redraw, "back to hot");
    assert_eq!(
        rebuilt.spin_lit(spin_i),
        Some((Arrow::Up, Interaction::Hot))
    );
    assert_eq!(spin(&rebuilt).value(), 11, "release did not step it again");

    let mut f = lit_form();
    drawn(&f, &theme);
    let track = f.index_of("track").unwrap();
    let (x, y) = point_on(&f, track, Part::TrackBar(trackbar::Hit::Thumb));
    f.motion(x, y, at(&f, x, y));
    f.click(at(&f, x, y));
    f.motion(x, 300, None);
    assert_eq!(
        f.lit(track, Light::Thumb),
        Interaction::Pressed,
        "off it, still sticky"
    );
    let mut rebuilt = lit_form();
    rebuilt.carry_pointer(&f);
    drawn(&rebuilt, &theme);
    assert_eq!(
        rebuilt.lit(track, Light::Thumb),
        Interaction::Pressed,
        "sticky pressed survives the rebuild"
    );
    rebuilt.release();
    assert_eq!(
        rebuilt.lit(track, Light::Thumb),
        Interaction::Idle,
        "released off it"
    );
}

/// A page rebuilt under the pointer (a keystroke in one of its
/// edits) keeps a hot edit and an editable combo's hot field lit.
#[test]
fn carry_pointer_keeps_a_hot_edit_and_an_editable_combo_field_lit() {
    let form = || {
        let mut cb = Combo::new(&["a"], 0, 4);
        cb.edit = Some(TextEdit::new("text", false));
        Form::new(vec![
            Item::edit("edit", Rect::new(10, 10, 100, 21), "x", true),
            Item::new(
                "combo",
                Rect::new(10, 40, 140, combo::COMBO_H),
                Control::Combo(cb),
            ),
        ])
    };
    let theme = blue();
    for (id, part, light) in [
        ("edit", Part::Edit(0), Light::Body),
        ("combo", Part::Combo(combo::Hit::Field), Light::Field),
    ] {
        let mut f = form();
        drawn(&f, &theme);
        let i = f.index_of(id).unwrap();
        let (x, y) = (f.items[i].rect.x + 20, f.items[i].rect.y + 10);
        let hit = at(&f, x, y);
        assert_eq!(
            hit.map(|h| (h.item, std::mem::discriminant(&h.part))),
            Some((i, std::mem::discriminant(&part))),
            "{id}: the point is on its field"
        );
        f.motion(x, y, hit);
        assert_eq!(f.lit(i, light), Interaction::Hot, "{id} lights");
        let mut rebuilt = form();
        rebuilt.carry_pointer(&f);
        assert_eq!(
            rebuilt.lit(i, light),
            Interaction::Hot,
            "{id} stays hot across the rebuild"
        );
    }
}

/// A rebuild that drops the held item (its id is gone, or its part no
/// longer exists on it) leaves the new form with nothing held, rather
/// than pointing the pressed look at a different control.
#[test]
fn carry_pointer_drops_a_held_item_the_rebuild_no_longer_has() {
    let mut f = lit_form();
    drawn(&f, &blue());
    assert_eq!(f.click(at(&f, 20, 120)), Event::Redraw, "ok held");
    assert_eq!(f.held_item(), f.index_of("ok"));

    let mut rebuilt = Form::new(
        lit_form()
            .items
            .into_iter()
            .filter(|it| it.id != "ok")
            .collect(),
    );
    rebuilt.carry_pointer(&f);
    assert!(!rebuilt.holding(), "the held button no longer exists");
    assert_eq!(rebuilt.pointer.hot(), None);
}
