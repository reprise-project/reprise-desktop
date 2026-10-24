use super::*;

/// A greyed Blue edit fills with what sits behind it inside its line: the page's own
/// pixels on a page, the face on the dialog.
#[test]
fn a_greyed_blue_edit_shows_the_page_or_the_face_behind_it() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let text = TextRenderer::load(&theme.fonts);
    let px = theme.fonts.size_px;
    let p = theme.paint();
    let face = theme.colors.button_face;
    let page = Rect::new(9, 49, 386, 495);
    let greyed = |id, r| {
        Item::new(
            id,
            r,
            Control::Edit {
                edit: TextEdit::new("", false),
                first: std::cell::Cell::new(0),
                enabled: false,
            },
        )
    };
    let high = Rect::new(33, 100, 185, 23);
    let low = Rect::new(24, 512, 173, 20);
    let form = Form::new(vec![greyed("high", high), greyed("low", low)]);
    let mut bare = Canvas::new(400, 560);
    bare.fill_rect(bare.bounds(), face);
    crate::look::widgets::draw_page(&mut bare, &p, page);
    let mut c = bare.clone();
    draw(&mut c, &text, &p.on_page(page), &form, false, px);
    for (r, top, bottom) in [
        (high, 0xFFFB_FCFB, 0xFFFA_FBFA),
        (low, 0xFFF5_F3ED, 0xFFF5_F3ED),
    ] {
        let inside = r.inset(1);
        for y in inside.y..inside.bottom() {
            for x in inside.x..inside.right() {
                assert_eq!(c.get(x, y), bare.get(x, y), "({x}, {y}) is the page");
            }
        }
        assert_eq!(
            c.get(inside.x + 5, inside.y),
            top,
            "{r:?}: the page's top row"
        );
        let last = inside.bottom() - 1;
        assert_eq!(
            c.get(inside.x + 5, last),
            bottom,
            "{r:?}: the page's bottom row"
        );
        assert_eq!(
            c.get(r.x, r.y + 10),
            crate::blue::Variant::Blue
                .scheme()
                .edit_border
                .normal
                .rim
                .argb(),
            "{r:?}: the field's line"
        );
    }
    let mut dialog = Canvas::new(400, 560);
    dialog.fill_rect(dialog.bounds(), face);
    draw(&mut dialog, &text, &p, &form, false, px);
    let inside = high.inset(1);
    for y in inside.y..inside.bottom() {
        for x in inside.x..inside.right() {
            assert_eq!(dialog.get(x, y), face.argb(), "({x}, {y}) is the face");
        }
    }
}

/// In Blue a radio shares a check box's column: for a box at x 10 and
/// a radio item at x 11 (where Classic's circle is, a pixel right of
/// the box), the circle starts at x 10 too, and "Read-only" and "Up"
/// both ink their first column 16 px past it, at 26.
#[test]
fn a_blue_check_box_and_radio_share_a_column() {
    let text = TextRenderer::load(&FontConfig::default());
    if !crate::text::test_tahoma() {
        return;
    }
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let form = Form::new(vec![
        Item::check("ro", 10, 10, "&Read-only"),
        Item::radio("up", 11, 40, "&Up", "dir"),
    ]);
    let mut c = Canvas::new(100, 60);
    c.fill_rect(c.bounds(), cs.button_face);
    draw(&mut c, &text, &theme.paint(), &form, false, px);
    let ink = |rows: std::ops::Range<i32>| {
        (24..60).find(|&x| rows.clone().any(|y| c.get(x, y) == cs.button_text.argb()))
    };
    let left =
        |y0: i32| (0..24).find(|&x| (y0..y0 + 13).any(|y| c.get(x, y) != cs.button_face.argb()));
    assert_eq!(left(10), Some(10), "the box starts at x 10");
    assert_eq!(left(40), Some(10), "the circle starts at x 10");
    assert_eq!(ink(8..26), Some(26), "the check box label inks from x 26");
    assert_eq!(ink(38..56), Some(26), "the radio label inks from x 26");
}

/// In Glass too a radio shares a check box's column: for a box at x 10
/// and a radio item at x 11, the 12 px circle starts at x 10, and
/// "Read-only" and "Up" both ink their first column 16 px past it.
#[test]
fn a_glass_check_box_and_radio_share_a_column() {
    let text = TextRenderer::load(&FontConfig::default());
    if !crate::text::test_tahoma() {
        return;
    }
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let form = Form::new(vec![
        Item::check("ro", 10, 10, "&Read-only"),
        Item::radio("up", 11, 40, "&Up", "dir"),
    ]);
    let mut c = Canvas::new(100, 60);
    c.fill_rect(c.bounds(), cs.button_face);
    draw(&mut c, &text, &theme.paint(), &form, false, px);
    let ink = |rows: std::ops::Range<i32>| {
        (24..60).find(|&x| rows.clone().any(|y| c.get(x, y) == cs.button_text.argb()))
    };
    let left =
        |y0: i32| (0..24).find(|&x| (y0..y0 + 13).any(|y| c.get(x, y) != cs.button_face.argb()));
    assert_eq!(left(10), Some(10), "the box starts at x 10");
    assert_eq!(left(40), Some(10), "the circle starts at x 10");
    assert_eq!(ink(8..26), Some(26), "the check box label inks from x 26");
    assert_eq!(ink(38..56), Some(26), "the radio label inks from x 26");
}

/// A greyed plain static label (`Control::Label`, not a check box or
/// radio) keeps the etched pair under Blue, the same as Classic: a
/// white copy a pixel down and right under the grey.
#[test]
fn a_greyed_blue_static_label_is_etched() {
    let text = TextRenderer::load(&FontConfig::default());
    if !crate::text::test_tahoma() {
        return;
    }
    let form = Form::new(vec![
        Item::label_at("wait", 10, 10, 70, "Wait:").enabled(false),
    ]);
    let label = Rect::new(10, 10, 70, LINE_H);
    let count = |c: &Canvas, ink: u32| {
        (label.y..label.bottom())
            .flat_map(|y| (label.x..label.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == ink)
            .count()
    };
    let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let mut c = Canvas::new(100, 40);
    c.fill_rect(c.bounds(), blue.colors.button_face);
    draw(&mut c, &text, &blue.paint(), &form, false, PX);
    assert!(count(&c, 0xFFFF_FFFF) > 0, "white pixels sit at +1,+1");
}

/// A greyed Blue check box inks its label once, flat in the greyed ink, with
/// no white copy under it; the same box in Classic keeps the white
/// copy a pixel down and right.
#[test]
fn a_greyed_blue_check_box_label_is_flat() {
    let text = TextRenderer::load(&FontConfig::default());
    if !crate::text::test_tahoma() {
        return;
    }
    let form = Form::new(vec![Item::check("ro", 10, 10, "&Read-only").enabled(false)]);
    let label = Rect::new(24, 8, 70, 18);
    let count = |c: &Canvas, ink: u32| {
        (label.y..label.bottom())
            .flat_map(|y| (label.x..label.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == ink)
            .count()
    };
    let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let mut c = Canvas::new(100, 40);
    c.fill_rect(c.bounds(), blue.colors.button_face);
    draw(&mut c, &text, &blue.paint(), &form, false, PX);
    let grey = crate::blue::Variant::Blue.scheme().inks.greyed_label.argb();
    assert!(count(&c, grey) > 0, "the label inks the greyed label ink");
    assert_eq!(count(&c, 0xFFFF_FFFF), 0, "no white pixel under the label");
    let classic = Theme::default();
    let mut c = Canvas::new(100, 40);
    c.fill_rect(c.bounds(), classic.colors.button_face);
    draw(&mut c, &text, &classic.paint(), &form, false, PX);
    assert!(
        count(&c, classic.colors.button_hilight.argb()) > 0,
        "Classic keeps the white copy"
    );
}

/// A greyed Blue radio button inks its label once, flat in the greyed ink,
/// the same ink the art uses for a greyed check box label.
#[test]
fn a_greyed_blue_radio_label_is_flat() {
    let text = TextRenderer::load(&FontConfig::default());
    if !crate::text::test_tahoma() {
        return;
    }
    let form = Form::new(vec![Item::radio("up", 10, 10, "&Up", "dir").enabled(false)]);
    let label = Rect::new(26, 8, 70, 18);
    let count = |c: &Canvas, ink: u32| {
        (label.y..label.bottom())
            .flat_map(|y| (label.x..label.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == ink)
            .count()
    };
    let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let mut c = Canvas::new(100, 40);
    c.fill_rect(c.bounds(), blue.colors.button_face);
    draw(&mut c, &text, &blue.paint(), &form, false, PX);
    let grey = crate::blue::Variant::Blue.scheme().inks.greyed_label.argb();
    assert!(count(&c, grey) > 0, "the label inks the greyed label ink");
    assert_eq!(count(&c, 0xFFFF_FFFF), 0, "no white pixel under the label");
}

/// A greyed Glass check box and radio ink their labels once, flat in
/// 6B6B6B, with no white copy under them.
#[test]
fn greyed_glass_check_box_and_radio_labels_are_flat_6b6b6b() {
    let text = TextRenderer::load(&FontConfig::default());
    if !crate::text::test_tahoma() {
        return;
    }
    let form = Form::new(vec![
        Item::check("ro", 10, 10, "&Read-only").enabled(false),
        Item::radio("up", 10, 40, "&Up", "dir").enabled(false),
    ]);
    let glass = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let mut c = Canvas::new(100, 70);
    c.fill_rect(c.bounds(), glass.colors.button_face);
    draw(&mut c, &text, &glass.paint(), &form, false, PX);
    for (what, label) in [
        ("check box", Rect::new(24, 8, 70, 18)),
        ("radio", Rect::new(26, 38, 70, 18)),
    ] {
        let count = |ink: u32| {
            (label.y..label.bottom())
                .flat_map(|y| (label.x..label.right()).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == ink)
                .count()
        };
        assert!(count(0xFF6B_6B6B) > 0, "the {what} label inks 6B6B6B");
        assert_eq!(
            count(0xFFFF_FFFF),
            0,
            "no white pixel under the {what} label"
        );
    }
}

/// A greyed three-state box in its indeterminate state draws the
/// look's greyed mixed box: Blue's CAC8BB square 3 px inside the box.
#[test]
fn a_greyed_mixed_blue_check_box_shows_its_square() {
    let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let mut form = Form::new(vec![Item::check("ro", 10, 10, "&Read-only").enabled(false)]);
    if let Some(Control::Check { mixed, .. }) = form.item_mut("ro").map(|i| &mut i.control) {
        *mixed = Some(true);
    }
    let mut c = Canvas::new(100, 40);
    draw(&mut c, &tr(), &blue.paint(), &form, false, PX);
    let faces = &crate::blue::Variant::Blue.scheme().check_box;
    let grey = 0xFF00_0000 | faces.disabled_mixed.rim.argb();
    let mark = 0xFF00_0000 | faces.disabled_mixed.mark.unwrap().argb();
    assert_eq!(c.get(10, 10), grey, "the greyed frame");
    assert_eq!(c.get(13, 13), mark, "the square starts 3 px in");
    assert_ne!(c.get(12, 12), mark, "the well round the square");
}

/// A Blue radio's label is centred on its 13 px circle, as a check
/// box's is on its box: "&Up" inks its last row and its underline on
/// the same rows below either item's top.
#[test]
fn a_blue_radio_label_sits_on_the_check_box_labels_row() {
    let text = tr();
    if !crate::text::test_tahoma() {
        return;
    }
    let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let cs = &blue.colors;
    let form = Form::new(vec![
        Item::check("c", 10, 10, "&Up"),
        Item::radio("r", 10, 40, "&Up", "g"),
    ]);
    let mut c = Canvas::new(100, 70);
    c.fill_rect(c.bounds(), cs.button_face);
    draw(&mut c, &text, &blue.paint(), &form, false, PX);
    let last_ink = |top: i32| {
        (top..top + 20)
            .filter(|&y| (26..60).any(|x| c.get(x, y) == cs.button_text.argb()))
            .max()
            .map(|y| y - top)
    };
    assert!(last_ink(10).is_some(), "the check box label inks");
    assert_eq!(
        last_ink(40),
        last_ink(10),
        "the radio label's last row matches the check box label's"
    );
}

/// Two Glass samples whose looks share a name but not their art (the
/// glass in another colour) compare equal as controls, yet the second
/// renders afresh: a sample is kept only while its style is the same.
#[test]
fn a_sample_in_other_glass_art_renders_afresh() {
    use crate::glass;
    use crate::interface::Interface;
    let t = tr();
    let sky = crate::interfaces::theme_for(Interface::Glass);
    let ruby = Theme {
        look: Look::Styled(glass::style(&glass::Settings {
            color: glass::swatch("ruby").expect("ruby is a swatch"),
            ..glass::Settings::default()
        })),
        ..sky.clone()
    };
    let sample = |theme: Theme| Control::Sample {
        themes: false,
        theme,
        icon: None,
    };
    assert_eq!(sample(sky.clone()), sample(ruby.clone()), "one look name");
    let mut f = Form::new(vec![Item::new(
        "sample",
        Rect::new(0, 0, 350, 188),
        sample(sky),
    )]);
    let shot = |f: &Form| {
        let mut c = Canvas::new(360, 190);
        draw(
            &mut c,
            &t,
            &classic_paint(&Theme::default().colors, 1),
            f,
            false,
            PX,
        );
        c
    };
    let first = shot(&f);
    f.items[0].control = sample(ruby);
    let second = shot(&f);
    assert_eq!(f.rendered.renders.get(), 2, "the ruby sample renders");
    assert_ne!(first.pixels, second.pixels, "the ruby glass shows");
}

/// A monitor's fitted picture and bezel and a sample render once per
/// input: a redraw with the same control and scale, such as a caret
/// blink, blits what the first draw rendered, and a changed control or
/// scale renders afresh.
#[test]
fn monitor_and_sample_render_once_per_input() {
    let t = tr();
    let theme = Theme::default();
    let cs = &theme.colors;
    let mut pic = Canvas::new(40, 30);
    pic.fill_rect(pic.bounds(), Color::rgb(0, 128, 0));
    let mut f = Form::new(vec![
        Item::new(
            "monitor",
            Rect::new(0, 0, 191, 168),
            Control::Monitor(Screen::Picture(pic)),
        ),
        Item::new(
            "sample",
            Rect::new(200, 0, 350, 188),
            Control::Sample {
                themes: false,
                theme: Theme::default(),
                icon: None,
            },
        ),
    ]);
    let shot = |f: &Form, s: i32| {
        let mut c = Canvas::new(560 * s as u32, 190 * s as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &t, &classic_paint(cs, s), f, false, PX);
        c
    };
    let first = shot(&f, 1);
    assert_eq!(f.rendered.renders.get(), 2, "one render per control");
    f.caret = !f.caret;
    assert_eq!(shot(&f, 1).pixels, first.pixels, "a redraw draws the same");
    assert_eq!(f.rendered.renders.get(), 2, "a redraw renders nothing");

    shot(&f, 2);
    assert_eq!(f.rendered.renders.get(), 4, "a new scale renders both");
    let Some(Item {
        control: Control::Sample { theme, .. },
        ..
    }) = f.item_mut("sample")
    else {
        panic!("the sample is the second item")
    };
    theme.colors.active_title = Color::rgb(128, 0, 0);
    let changed = shot(&f, 2);
    assert_eq!(
        f.rendered.renders.get(),
        5,
        "a new theme renders the sample"
    );
    assert_eq!(
        changed.pixels,
        shot(&Form::new(f.items.clone()), 2).pixels,
        "the kept sample draws what a fresh render does"
    );
}
