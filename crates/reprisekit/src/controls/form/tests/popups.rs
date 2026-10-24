use super::*;

/// A closed combo still lists its rows with the picked one selected,
/// which is where a reader takes the combo's value from.
#[test]
fn a_closed_combo_lists_its_rows_with_the_pick_selected() {
    use crate::access::{ROOT, Tree};
    let mut item = Item::combo("renderer", 10, 10, 120, &["auto", "gles2", "vulkan"], 3);
    if let Control::Combo(c) = &mut item.control {
        c.selected = 1;
    }
    let form = Form::new(vec![item]);
    let mut tree = Tree::window("T", (200, 100));
    let focus = form.access(&mut tree, ROOT, &tr(), 11).expect("focus");
    assert_eq!(tree.get(focus).unwrap().role(), Role::ComboBox);
    let picked: Vec<_> = tree
        .nodes()
        .filter(|(_, n)| n.role() == Role::ListBoxOption && n.is_selected() == Some(true))
        .map(|(_, n)| n.label().unwrap_or_default().to_string())
        .collect();
    assert_eq!(picked, ["gles2"]);
}

/// A notch over a focused closed combo picks the next item down or up;
/// past either end, or without the focus, it does nothing.
#[test]
fn the_wheel_over_a_focused_closed_combo_steps_its_pick() {
    let items = ["a", "b", "c", "d"];
    let mut f = Form::new(vec![
        Item::combo("c", 10, 10, 120, &items, 3),
        Item::check("check", 10, 50, "Check"),
    ]);
    f.focus_id("check");
    let on = at(&f, 20, 15);
    assert_eq!(f.wheel(on, WHEEL_ROWS), Event::None, "not focused");
    assert_eq!(f.combo_selected("c"), Some(0));
    f.focus_id("c");
    assert_eq!(f.wheel(on, WHEEL_ROWS), Event::Changed("c"));
    assert_eq!(f.combo_selected("c"), Some(1));
    assert_eq!(f.wheel(on, 2 * WHEEL_ROWS), Event::Changed("c"));
    assert_eq!(f.combo_selected("c"), Some(3), "two notches, two items");
    assert_eq!(f.wheel(on, WHEEL_ROWS), Event::None, "already last");
    assert_eq!(f.wheel(on, -WHEEL_ROWS), Event::Changed("c"));
    assert_eq!(f.combo_selected("c"), Some(2));
    assert_eq!(f.wheel(None, WHEEL_ROWS), Event::None, "off the combo");
}

/// With the list open the wheel scrolls it, wherever the pointer is,
/// and leaves the pick alone.
#[test]
fn a_hosted_combo_list_draws_alone_as_in_the_form_and_is_hit_where_it_was_put() {
    use crate::controls::hosted::{Anchor, Hosted};
    let theme = Theme::default();
    let t = tr();
    let items = ["a", "b", "c", "d", "e", "f"];
    let mut f = Form::new(vec![Item::combo("c", 10, 10, 120, &items, 3)]);
    assert!(f.spots().is_empty(), "nothing to show while closed");
    f.click(at(&f, 20, 15));
    let p = classic_paint(&theme.colors, 1);
    let mut whole = Canvas::new(200, 120);
    draw(&mut whole, &t, &p, &f, false, PX);
    let spots = f.spots();
    assert_eq!(spots.len(), 1);
    assert_eq!(spots[0].anchor, Anchor::Below(f.rect("c").unwrap()));
    let r = spots[0].rect;
    let mut own = Canvas::new(r.width as u32, r.height as u32);
    f.draw_level(0, &mut own, (-r.x, -r.y), &theme, &t);
    for y in 0..r.height {
        for x in 0..r.width {
            assert_eq!(own.get(x, y), whole.get(r.x + x, r.y + y), "({x}, {y})");
        }
    }
    f.host();
    let mut bare = Canvas::new(200, 120);
    draw(&mut bare, &t, &p, &f, false, PX);
    assert_eq!(
        bare.get(r.x + 5, r.y + 5),
        0,
        "the form leaves the list out"
    );
    f.place(0, (300, 400));
    let row = (r.height - 2) / 3;
    f.click(at(&f, 310, 401 + row + row / 2));
    let Control::Combo(c) = &f.items[0].control else {
        unreachable!()
    };
    assert_eq!(
        (c.selected, c.open),
        (1, None),
        "the second row, where it was put"
    );
}

#[test]
fn a_hosted_colour_popup_draws_alone_as_in_the_form_and_is_hit_where_it_was_put() {
    use crate::controls::hosted::{Anchor, Hosted};
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    let t = tr();
    for look in [Interface::Classic, Interface::Blue] {
        let theme = theme_for(look);
        let red = Color::rgb(255, 0, 0);
        let blue = Color::rgb(0, 0, 255);
        let palette = vec![red, blue, Color::rgb(0, 255, 0)];
        let button = Rect::new(10, 10, 75, 21);
        let mut f = Form::new(vec![Item::new(
            "c",
            button,
            Control::Color(ColorButton::new(red, palette)),
        )]);
        assert!(f.spots().is_empty(), "nothing to show while closed");
        f.click(at(&f, 20, 15));
        let p = theme.paint();
        let mut whole = Canvas::new(200, 200);
        draw(&mut whole, &t, &p, &f, false, PX);
        let spots = f.spots();
        assert_eq!(spots.len(), 1);
        assert_eq!(
            spots[0].anchor,
            Anchor::Below(Rect::new(10, 10, 98, 21)),
            "{look:?}: the popup hangs 21 px under the button's top"
        );
        let r = spots[0].rect;
        assert_eq!(r, Rect::new(10, 31, 98, 151), "{look:?}");
        let mut own = Canvas::new(r.width as u32, r.height as u32);
        f.draw_level(0, &mut own, (-r.x, -r.y), &theme, &t);
        for y in 0..r.height {
            for x in 0..r.width {
                assert_eq!(
                    own.get(x, y),
                    whole.get(r.x + x, r.y + y),
                    "{look:?} ({x}, {y})"
                );
            }
        }
        f.host();
        let mut bare = Canvas::new(200, 200);
        draw(&mut bare, &t, &p, &f, false, PX);
        assert_eq!(
            bare.get(r.x + 30, r.y + 30),
            0,
            "{look:?}: the form leaves the popup out"
        );
        f.place(0, (300, 400));
        // The second swatch: 5 + 23 px in, 5 px down from the popup's top.
        f.click(at(&f, 300 + 5 + 23 + 9, 400 + 5 + 9));
        let Control::Color(b) = &f.items[0].control else {
            unreachable!()
        };
        assert_eq!(
            (b.color, b.open),
            (blue, false),
            "{look:?}: the second swatch, where it was put"
        );
    }
}

#[test]
fn the_wheel_scrolls_an_open_combo_list() {
    let items = ["a", "b", "c", "d", "e", "f"];
    let mut f = Form::new(vec![Item::combo("c", 10, 10, 120, &items, 3)]);
    let on = at(&f, 20, 15);
    f.click(on);
    assert!(f.popup_open());
    assert_eq!(f.wheel(None, 2), Event::Redraw);
    let Control::Combo(c) = &f.items[0].control else {
        unreachable!()
    };
    assert_eq!((c.top, c.selected), (2, 0));
    assert_eq!(f.wheel(None, WHEEL_ROWS), Event::Redraw);
    assert_eq!(f.wheel(None, WHEEL_ROWS), Event::None, "at the bottom");
    assert!(f.popup_open(), "the list stays open");
}

/// An open combo's highlight follows the pointer down its list, past
/// a greyed row, and the pick stays until a click.
#[test]
fn an_open_combo_highlight_follows_the_pointer() {
    let mut item = Item::combo("c", 10, 10, 120, &["a", "b", "c", "d"], 4);
    if let Control::Combo(c) = &mut item.control {
        c.items[2].greyed = true;
    }
    let mut f = Form::new(vec![item]);
    f.click(at(&f, 20, 15));
    let open = |f: &Form| match &f.items[0].control {
        Control::Combo(c) => (c.open, c.selected),
        _ => unreachable!(),
    };
    assert_eq!(open(&f), (Some(0), 0));
    // Classic's 13 px rows start under the list's top line at 32.
    let row = |r: i32| (20, 32 + 13 * r + 6);
    let (x, y) = row(1);
    assert_eq!(f.motion(x, y, at(&f, x, y)), Event::Redraw);
    assert_eq!(open(&f), (Some(1), 0));
    assert_eq!(
        f.motion(x, y + 1, at(&f, x, y + 1)),
        Event::None,
        "same row"
    );
    let (x, y) = row(2);
    f.motion(x, y, at(&f, x, y));
    assert_eq!(open(&f), (Some(1), 0), "the greyed row is passed over");
    let (x, y) = row(3);
    f.motion(x, y, at(&f, x, y));
    assert_eq!(open(&f), (Some(3), 0));
    assert_eq!(f.click(at(&f, x, y)), Event::Changed("c"));
    assert_eq!(open(&f), (None, 3));
}
