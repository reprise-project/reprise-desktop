use super::*;

#[test]
fn set_check_moves_a_radios_own_dot_only() {
    let mut f = radio_form();
    f.set_check("up", true);
    f.set_check("down", false);
    assert_eq!(f.checked("up"), Some(true));
    assert_eq!(f.checked("down"), Some(false));
    assert_eq!(f.checked("all"), Some(true), "another group stays");
}

fn checked(f: &Form, id: &str) -> bool {
    matches!(
        f.item(id).map(|i| &i.control),
        Some(Control::Radio { checked: true, .. })
    )
}

#[test]
fn positioned_constructors_take_the_row_and_button_heights() {
    assert_eq!(
        Item::label_at("l", 5, 6, 70, "A").rect,
        Rect::new(5, 6, 70, 13)
    );
    let v = Item::value_at("v", 5, 6, 70, "1");
    assert_eq!(v.rect, Rect::new(5, 6, 70, 13));
    assert!(matches!(
        v.control,
        Control::Label {
            align: Align::Right,
            ..
        }
    ));
    let p = Item::lines_at("p", 5, 6, 70, &["a", "b", "c"]);
    assert_eq!(p.rect, Rect::new(5, 6, 70, 39), "three 13 px rows");
    let b = Item::button_at("b", 5, 6, 75, "OK", true);
    assert_eq!(b.rect, Rect::new(5, 6, 75, 23));
    assert!(matches!(b.control, Control::Button { default: false, .. }));
}

#[test]
fn picking_a_radio_clears_its_group_and_leaves_others_alone() {
    let mut f = radio_form();
    f.focus_id("up");
    assert_eq!(f.key(Key::Space), Event::Changed("up"));
    assert!(checked(&f, "up"));
    assert!(!checked(&f, "down"), "the group is exclusive");
    assert!(checked(&f, "all"), "the other group is untouched");
}

#[test]
fn arrows_walk_within_a_group_and_stop_at_its_ends() {
    let mut f = radio_form();
    f.focus_id("up");
    // "down" is already this group's checked radio, so landing on it
    // mutates nothing: Redraw, not Changed.
    assert_eq!(f.key(Key::Down), Event::Redraw);
    assert_eq!(f.focus, f.index_of("down"));
    assert!(checked(&f, "down"));
    assert_eq!(f.key(Key::Down), Event::None);
    assert_eq!(f.focus, f.index_of("down"));
    assert!(checked(&f, "all"), "the next group is untouched");
    assert_eq!(f.key(Key::Up), Event::Changed("up"));
    assert_eq!(f.focus, f.index_of("up"));
    assert!(checked(&f, "up"));
    assert!(!checked(&f, "down"));
    assert_eq!(f.key(Key::Up), Event::None);
    assert_eq!(f.focus, f.index_of("up"));
    assert!(checked(&f, "up"));
}

#[test]
fn tab_treats_a_group_as_one_stop_and_enters_at_the_checked_radio() {
    let mut f = radio_form();
    // "down" is checked, so it is the group's stop, not "up".
    assert_eq!(f.focus, f.index_of("down"));
    assert_eq!(f.key(Key::Tab { back: false }), Event::Redraw);
    assert_eq!(f.focus, f.index_of("all"), "skips the rest of the group");
    assert_eq!(f.key(Key::Tab { back: false }), Event::Redraw);
    assert_eq!(f.focus, f.index_of("ok"));
}

#[test]
fn tab_without_the_wrap_stops_at_the_ends() {
    let mut f = radio_form();
    while f.tab_no_wrap(false) {}
    let last = f.focus;
    assert_eq!(last, f.index_of("ok"));
    assert!(!f.tab_no_wrap(false));
    assert_eq!(f.focus, last);
    while f.tab_no_wrap(true) {}
    assert_eq!(f.focus, f.index_of("down"), "the checked radio is the stop");
    assert!(!f.tab_no_wrap(true));
    assert!(f.focus_edge(true));
    assert_eq!(f.focus, last);
    assert!(f.focus_edge(false));
    assert_eq!(f.focus, f.index_of("down"));
}

#[test]
fn a_radios_access_key_picks_it() {
    let mut f = radio_form();
    f.focus_id("ok");
    assert_eq!(f.key(Key::Char('u')), Event::Changed("up"));
    assert!(checked(&f, "up"));
    assert_eq!(f.focus, f.index_of("up"));
}

#[test]
fn a_radios_label_is_part_of_its_hit_area() {
    let f = radio_form();
    assert_eq!(f.hit(12, 12, &tr(), PX).map(|h| h.item), f.index_of("up"));
    assert_eq!(f.hit(30, 12, &tr(), PX).map(|h| h.item), f.index_of("up"));
}

#[test]
fn a_disabled_radio_is_skipped_by_tab_and_arrows() {
    let mut f = radio_form();
    if let Control::Radio { enabled, .. } = &mut f.items[2].control {
        *enabled = false;
    }
    f.focus_id("down");
    // "all" is disabled, so Down stays put rather than landing on it.
    assert_eq!(f.key(Key::Down), Event::None);
    assert_eq!(f.focus, f.index_of("down"));
}

#[test]
fn clicking_a_radio_picks_it_and_clears_its_sibling() {
    let mut f = radio_form();
    let h = f.hit(12, 12, &tr(), PX);
    assert_eq!(h.map(|h| h.item), f.index_of("up"));
    assert_eq!(f.click(h), Event::Redraw, "the press only holds it");
    assert!(!checked(&f, "up"));
    assert_eq!(f.release(), Event::Changed("up"));
    assert!(checked(&f, "up"));
    assert!(!checked(&f, "down"), "the group is exclusive");
    assert!(checked(&f, "all"), "the other group is untouched");
    assert_eq!(f.focus, f.index_of("up"));
}

#[test]
fn clicking_an_already_checked_radio_changes_nothing() {
    let mut f = radio_form();
    let h = f.hit(30, 38, &tr(), PX);
    assert_eq!(h.map(|h| h.item), f.index_of("down"));
    // "down" is already this group's pick, so nothing mutates.
    assert_eq!(f.click(h), Event::Redraw);
    assert_eq!(f.release(), Event::Redraw);
    assert!(checked(&f, "down"));
    assert!(!checked(&f, "up"));
}

#[test]
fn clicking_a_disabled_radio_does_nothing() {
    let mut f = radio_form();
    if let Control::Radio { enabled, .. } = &mut f.items[2].control {
        *enabled = false;
    }
    f.focus_id("down");
    let before = f.focus;
    let h = f.hit(12, 62, &tr(), PX);
    assert_eq!(h.map(|h| h.item), f.index_of("all"), "it is still hit");
    assert_eq!(f.click(h), Event::None);
    assert!(!f.holding(), "a disabled radio is not held");
    assert_eq!(f.release(), Event::None);
    assert!(checked(&f, "all"));
    assert!(checked(&f, "down"));
    assert!(!checked(&f, "up"));
    assert_eq!(before, f.focus, "a disabled radio does not take focus");
}

/// A group with one enabled radio has nowhere to step: the walk is
/// confined to the group and does not wrap, so both arrows run off
/// an end and do nothing at all.
#[test]
fn an_arrow_on_a_lone_radio_does_nothing() {
    let mut f = Form::new(vec![
        Item::radio("solo", 10, 10, "&Solo", "only").checked(true),
        Item::new(
            "ok",
            Rect::new(10, 40, 75, 23),
            Control::Button {
                label: "OK".into(),
                enabled: true,
                default: true,
            },
        ),
    ]);
    f.focus_id("solo");
    assert_eq!(f.key(Key::Down), Event::None);
    assert_eq!(f.key(Key::Up), Event::None);
    assert_eq!(f.focus, f.index_of("solo"));
    assert!(checked(&f, "solo"));
}

/// A save prompt's row: Yes is the default, a greyed button sits
/// between it and No, and a combo stands before the row.
fn button_row_form() -> Form {
    Form::new(vec![
        Item::new(
            "combo",
            Rect::new(10, 10, 150, 21),
            Control::Combo(Combo::new(&["Alpha", "Beta", "Gamma"], 0, 8)),
        ),
        Item::button("yes", Rect::new(10, 40, 75, 23), "&Yes", true, true),
        Item::button("greyed", Rect::new(90, 40, 75, 23), "&Later", false, false),
        Item::button("no", Rect::new(170, 40, 75, 23), "&No", true, false),
        Item::button("cancel", Rect::new(250, 40, 75, 23), "Cancel", true, false),
    ])
}

#[test]
fn arrows_walk_the_push_buttons_skip_greyed_ones_and_stop_at_the_ends() {
    let mut f = button_row_form();
    f.focus_id("yes");
    assert_eq!(
        f.key(Key::Left),
        Event::None,
        "Left on the first button stops"
    );
    assert_eq!(f.key(Key::Up), Event::None, "Up on the first button stops");
    assert_eq!(f.focus, f.index_of("yes"), "the focus stays on Yes");
    assert_eq!(f.key(Key::Right), Event::Redraw);
    assert_eq!(
        f.focus,
        f.index_of("no"),
        "Right skips the greyed button to No"
    );
    assert_eq!(f.key(Key::Down), Event::Redraw);
    assert_eq!(f.focus, f.index_of("cancel"), "Down moves on to Cancel");
    assert_eq!(
        f.key(Key::Right),
        Event::None,
        "Right on the last button stops"
    );
    assert_eq!(
        f.key(Key::Down),
        Event::None,
        "Down on the last button stops"
    );
    assert_eq!(f.focus, f.index_of("cancel"), "the focus stays on Cancel");
    assert_eq!(f.key(Key::Up), Event::Redraw);
    assert_eq!(f.focus, f.index_of("no"), "Up moves back to No");
    assert_eq!(f.key(Key::Left), Event::Redraw);
    assert_eq!(
        f.focus,
        f.index_of("yes"),
        "Left moves back to Yes, not the combo"
    );
    let right = Key::Edit(EditKey::Right {
        select: false,
        word: false,
    });
    assert_eq!(f.key(right), Event::Redraw);
    assert_eq!(f.focus, f.index_of("no"), "an editing Right moves to No");
    assert_eq!(f.key(Key::Enter), Event::Pressed("no"), "Enter presses No");
}

#[test]
fn an_arrow_on_a_combo_still_steps_the_combo() {
    let mut f = button_row_form();
    f.focus_id("combo");
    assert_eq!(f.key(Key::Down), Event::Changed("combo"));
    assert_eq!(f.focus, f.index_of("combo"), "the focus stays on the combo");
    assert_eq!(f.key(Key::Right), Event::Changed("combo"));
    assert_eq!(f.focus, f.index_of("combo"), "the focus stays on the combo");
}

#[test]
fn an_arrow_to_a_button_takes_the_default_ring_from_the_default() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let cs = &theme.colors;
    let mut f = form();
    f.focus_id("ok");
    assert_eq!(f.key(Key::Right), Event::Redraw);
    assert_eq!(
        f.focus,
        f.index_of("cancel"),
        "Right moves from OK to Cancel"
    );
    let mut c = Canvas::new(200, 150);
    c.fill_rect(c.bounds(), cs.button_face);
    draw(&mut c, &text, &classic_paint(cs, 1), &f, true, 11);
    assert_eq!(
        c.get(90, 115),
        cs.button_text.argb(),
        "Cancel's left edge is the black ring"
    );
    assert_eq!(
        c.get(10, 115),
        cs.button_hilight.argb(),
        "OK's left edge is the plain white edge"
    );
}

/// Space works the focused control even when the host sends editing
/// keys: it arrives as a one-character insert, and the general arm
/// in `flat_key` would degrade it to `Key::Char(' ')`, which the
/// access-key search then swallows.
#[test]
fn space_as_an_editing_key_still_works_the_focused_control() {
    let mut f = radio_form();
    f.focus_id("up");
    assert!(!checked(&f, "up"));
    assert_eq!(
        f.key(Key::Edit(EditKey::Insert(" ".into()))),
        Event::Changed("up")
    );
    assert!(checked(&f, "up"), "space picked the focused radio");

    // A space in a focused edit field is still a space: it never
    // reaches flat_key at all.
    let mut e = edit_form();
    assert_eq!(
        e.key(Key::Edit(EditKey::Insert(" ".into()))),
        Event::Changed("find")
    );
    assert!(matches!(&e.items[1].control, Control::Edit { edit, .. } if edit.text == " "));
}
