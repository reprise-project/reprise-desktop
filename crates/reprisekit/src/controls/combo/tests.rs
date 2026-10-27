use super::*;
use crate::look::classic_paint;
use crate::theme::{Color, FontConfig, Theme};

fn combo() -> Combo {
    Combo::new(&["Reprise Classic", "Slate", "Harbor"], 0, 8)
}

#[test]
fn a_value_the_list_lacks_shows_as_one_added_row() {
    let mut c = combo();
    c.select_text("Gone");
    c.select_text("Also gone");
    let rows: Vec<&str> = c.items.iter().map(|i| i.text.as_str()).collect();
    assert_eq!(rows, ["Reprise Classic", "Slate", "Harbor", "Also gone"]);
    assert_eq!(c.selected, 3);
    c.select_text("Slate");
    assert_eq!(c.items.len(), 3, "a listed value drops the added row");
    assert_eq!(c.selected, 1);
}

#[test]
fn a_popped_list_draws_alone_as_draw_list_does_and_moves_where_it_was_put() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let field = Rect::new(10, 10, 120, COMBO_H);
    let cm = ControlMetrics::CLASSIC;
    let mut c = combo();
    let mut p = Popped {
        combo: &mut c,
        field,
        cm,
        px: 11,
    };
    assert!(p.spots().is_empty());
    p.combo.toggle();
    let lay = layout(p.combo, field, &cm);
    assert_eq!(
        p.spots(),
        [Spot {
            rect: lay.list,
            anchor: Anchor::Below(field)
        }]
    );
    let mut whole = Canvas::new(200, 120);
    let paint = classic_paint(&theme.colors, 1);
    let items = p.combo.items.clone();
    draw_list(
        &mut whole,
        &text,
        &paint,
        &items,
        p.combo.open,
        0,
        8,
        lay.list,
        None,
        None,
        11,
    );
    let r = lay.list;
    let mut own = Canvas::new(r.width as u32, r.height as u32);
    p.draw_level(0, &mut own, (-r.x, -r.y), &theme, &text);
    for y in 0..r.height {
        for x in 0..r.width {
            assert_eq!(own.get(x, y), whole.get(r.x + x, r.y + y), "({x}, {y})");
        }
    }
    p.host();
    p.place(0, (300, 5));
    assert!(c.hosted);
    let lay = layout(&c, field, &cm);
    assert_eq!((lay.list.x, lay.list.y), (300, 5));
    assert_eq!(hit(&c, &lay, 305, 5 + 1 + c.row_h(&cm)), Some(Hit::Row(1)));
    c.toggle();
    c.toggle();
    assert_eq!(c.list_at, None, "a list opened again hangs under its field");
}

#[test]
fn a_reader_scrolls_a_dropped_row_in_and_the_highlight_stays() {
    use crate::access::{Published, ROOT, Tree};
    let names: Vec<String> = (0..30).map(|i| format!("Row {i}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut c = Combo::new(&names, 0, 8);
    let text = TextRenderer::load(&FontConfig::default());
    let lay = layout(
        &c,
        Rect::new(10, 10, 120, COMBO_H),
        &ControlMetrics::CLASSIC,
    );
    let tree_of = |c: &Combo| {
        let mut tree = Tree::window("", (200, 300));
        access(c, &lay, &text, 11, &mut tree, ROOT, "combo");
        tree
    };
    let tree = tree_of(&c);
    let row = tree.find(Role::ListBoxOption, "Row 20").unwrap();
    assert_eq!(
        tree.get(row).unwrap().bounds(),
        None,
        "a closed list has no rows to show"
    );
    assert!(c.toggle());
    let tree = tree_of(&c);
    let shown = tree.find(Role::ListBoxOption, "Row 7").unwrap();
    let row = tree.find(Role::ListBoxOption, "Row 20").unwrap();
    let mut p = Published::default();
    p.update(tree);
    assert_eq!(p.reveal(shown), None);
    assert_eq!(p.reveal(row).map(|r| r.row), Some(20));
    assert!(c.reveal(20));
    assert_eq!(c.top, 13, "row 20 is the last row shown");
    assert_eq!(c.open, Some(0), "the highlight stays");
}

#[test]
fn layout_puts_the_button_inside_the_edge_and_the_list_under_the_field() {
    let c = combo();
    let l = layout(
        &c,
        Rect::new(24, 119, 192, COMBO_H),
        &ControlMetrics::CLASSIC,
    );
    assert_eq!(l.drop, Rect::new(198, 121, 16, 17));
    assert_eq!(l.highlight, Rect::new(27, 122, 171, 15));
    assert_eq!(l.text.x, 28);
    assert_eq!(l.list, Rect::new(24, 140, 192, 3 * 13 + 2));
    assert!(l.scrollbar.is_none());
    let mut many = Combo::new(&["a"; 30], 0, 8);
    let l = layout(
        &many,
        Rect::new(0, 0, 100, COMBO_H),
        &ControlMetrics::CLASSIC,
    );
    assert_eq!(l.list.height, 8 * 13 + 2);
    assert_eq!(l.scrollbar, Some(Rect::new(83, 22, 16, 104)));
    // Hits: the list only counts while open.
    assert_eq!(hit(&many, &l, 90, 30), None);
    assert_eq!(hit(&many, &l, 90, 10), Some(Hit::Drop));
    assert_eq!(hit(&many, &l, 10, 10), Some(Hit::Field));
    assert!(many.toggle());
    assert_eq!(hit(&many, &l, 10, 22 + 13 + 3), Some(Hit::Row(1)));
    assert_eq!(hit(&many, &l, 90, 25), Some(Hit::ScrollUp));
    assert_eq!(hit(&many, &l, 90, 122), Some(Hit::ScrollDown));
    // 30 items, 8 in view: a 19 px thumb at the top of a 72 px
    // track, and the rest of the track pages.
    assert_eq!(hit(&many, &l, 90, 45), Some(Hit::Thumb(7)));
    assert_eq!(hit(&many, &l, 90, 70), Some(Hit::PageDown));
    // Dragging the thumb by that grab point walks the whole range.
    assert_eq!(top_at(&many, &l, 45, 7), 0);
    assert_eq!(top_at(&many, &l, 98, 7), 22);
    many.top = 22;
    assert_eq!(hit(&many, &l, 90, 45), Some(Hit::PageUp));
}

/// Blue's combo at the Themes tab's 192x21: a 15 px drop button with
/// a white column on each side, and the picked item's highlight
/// ending 3 px short of the button.
#[test]
fn a_blue_combo_drop_is_15_px_and_3_px_clear_of_the_highlight() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let cm = theme.look.controls();
    let text = TextRenderer::load(&FontConfig::default());
    let cs = &theme.colors;
    let c = combo();
    let field = Rect::new(3, 3, 192, COMBO_H);
    let l = layout(&c, field, &cm);
    assert_eq!(l.drop, Rect::new(178, 5, 15, 17));
    assert_eq!(l.highlight, Rect::new(6, 6, 169, 15));
    assert_eq!(hit(&c, &l, 178, 12), Some(Hit::Drop));
    assert_eq!(hit(&c, &l, 177, 12), Some(Hit::Field));
    let mut canvas = Canvas::new(198, 27);
    canvas.fill_rect(canvas.bounds(), cs.button_face);
    let p = theme.paint();
    draw(
        &mut canvas,
        &text,
        &p,
        &c,
        &l,
        true,
        true,
        Caret::Unfocused,
        ComboLit::IDLE,
        11,
    );
    let line = crate::blue::Variant::Blue
        .scheme()
        .edit_border
        .normal
        .rim
        .argb();
    let white = cs.window.argb();
    let dots = [cs.hilight.inverted().argb(), cs.button_text.argb()];
    for y in 6..21 {
        assert!(
            dots.contains(&canvas.get(174, y)),
            "(174, {y}) is a focus dot on the highlight's last column"
        );
        if (7..20).contains(&y) {
            assert_eq!(canvas.get(173, y), cs.hilight.argb(), "(173, {y})");
        }
        for x in 175..178 {
            assert_eq!(canvas.get(x, y), white, "({x}, {y}) is white");
        }
        assert_eq!(canvas.get(193, y), white, "(193, {y}) is white");
        assert_eq!(canvas.get(194, y), line, "(194, {y}) is the field line");
    }
    let mut drop = Canvas::new(198, 27);
    // The button's round corners let the white well through.
    drop.fill_rect(drop.bounds(), cs.window);
    crate::look::widgets::draw_combo_drop(&mut drop, &p, l.drop, true, Interaction::Idle, false);
    for y in 5..22 {
        for x in 178..193 {
            assert_eq!(canvas.get(x, y), drop.get(x, y), "drop art at ({x}, {y})");
        }
    }
}

/// Blue's greyed combo at the Effects dialog's 120x21: a #C9C7BA line,
/// a #F5F4EA ring inside it around the text part, face inside that,
/// and the greyed drop button in a white well.
#[test]
fn a_greyed_blue_combo_has_a_c9c7ba_line_and_its_button_a_white_well() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let text = TextRenderer::load(&FontConfig::default());
    let cs = &theme.colors;
    let mut c = combo();
    c.enabled = false;
    let l = layout(&c, Rect::new(3, 3, 120, COMBO_H), &theme.look.controls());
    assert_eq!(l.drop, Rect::new(106, 5, 15, 17));
    let mut canvas = Canvas::new(126, 27);
    canvas.fill_rect(canvas.bounds(), cs.button_face);
    let p = theme.paint();
    draw(
        &mut canvas,
        &text,
        &p,
        &c,
        &l,
        false,
        false,
        Caret::Unfocused,
        ComboLit::IDLE,
        11,
    );
    let greyed = crate::blue::Variant::Blue.scheme().edit_border.disabled;
    let (line, ring) = (greyed.rim.argb(), greyed.inner.argb());
    let (face, white) = (cs.button_face.argb(), cs.window.argb());
    for x in 3..123 {
        assert_eq!(canvas.get(x, 3), line, "top line at x {x}");
        assert_eq!(canvas.get(x, 23), line, "bottom line at x {x}");
        let inside = if x == 3 || x == 122 {
            line
        } else if x < 105 {
            ring
        } else {
            white
        };
        assert_eq!(canvas.get(x, 4), inside, "row 4 at x {x}");
        assert_eq!(canvas.get(x, 22), inside, "row 22 at x {x}");
    }
    for y in 4..23 {
        assert_eq!(canvas.get(3, y), line, "left line at y {y}");
        assert_eq!(canvas.get(122, y), line, "right line at y {y}");
        assert_eq!(canvas.get(4, y), ring, "left ring at y {y}");
        assert_eq!(canvas.get(104, y), ring, "the ring's right side at y {y}");
        assert_eq!(canvas.get(105, y), white, "the well's left side at y {y}");
        assert_eq!(canvas.get(121, y), white, "the well's right side at y {y}");
    }
    assert_eq!(canvas.get(5, 5), face, "face inside the ring");
    assert_eq!(canvas.get(103, 21), face, "face inside the ring");
    let mut drop = Canvas::new(126, 27);
    crate::look::widgets::draw_combo_drop(&mut drop, &p, l.drop, false, Interaction::Idle, false);
    for y in 5..22 {
        for x in 106..121 {
            // The button's round corners let what lies under it through.
            if (x - 106).min(120 - x) < 2 && (y - 5).min(21 - y) < 2 {
                continue;
            }
            assert_eq!(
                canvas.get(x, y),
                drop.get(x, y),
                "greyed drop at ({x}, {y})"
            );
        }
    }
}

/// A greyed 75 px Position combo on a 386x495
/// page: the C9C7BA line, the F5F4EA ring, then one ring of the page
/// itself around a face interior, on the left and right of the text
/// part and above and below it. On the dialog that page ring is face.
#[test]
fn a_greyed_blue_combo_on_a_page_rings_its_face_with_the_page() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let text = TextRenderer::load(&FontConfig::default());
    let face = theme.colors.button_face;
    let p = theme.paint();
    let page = Rect::new(9, 49, 386, 495);
    let mut c = Combo::new(&["Center", "Tile", "Stretch"], 2, 8);
    c.enabled = false;
    let l = layout(&c, Rect::new(306, 308, 75, COMBO_H), &theme.look.controls());
    let mut bare = Canvas::new(400, 560);
    bare.fill_rect(bare.bounds(), face);
    crate::look::widgets::draw_page(&mut bare, &p, page);
    let mut on_page = bare.clone();
    let mut on_dialog = Canvas::new(400, 560);
    on_dialog.fill_rect(on_dialog.bounds(), face);
    for (canvas, paint) in [(&mut on_page, p.on_page(page)), (&mut on_dialog, p)] {
        draw(
            canvas,
            &text,
            &paint,
            &c,
            &l,
            false,
            false,
            Caret::Unfocused,
            ComboLit::IDLE,
            11,
        );
    }
    let greyed = crate::blue::Variant::Blue.scheme().edit_border.disabled;
    let (line, ring, white, facev) = (
        greyed.rim.argb(),
        greyed.inner.argb(),
        0xFFFF_FFFF,
        face.argb(),
    );
    let well = l.drop.x - 1;
    // Across the text part, row by row down its left edge: line,
    // ring, page, face.
    assert_eq!(l.field.bottom(), 329);
    for y in 311..326 {
        let row: Vec<u32> = (306..310).map(|x| on_page.get(x, y)).collect();
        assert_eq!(
            row,
            [line, ring, bare.get(308, y), facev],
            "left edge at y {y}"
        );
        let row: Vec<u32> = (well - 3..=well).map(|x| on_page.get(x, y)).collect();
        assert_eq!(
            row,
            [facev, bare.get(well - 2, y), ring, white],
            "right of the text at y {y}"
        );
        assert_eq!(
            on_dialog.get(308, y),
            facev,
            "on the dialog the ring is face at y {y}"
        );
    }
    for x in 308..well - 1 {
        let col: Vec<u32> = (308..311).map(|y| on_page.get(x, y)).collect();
        assert_eq!(col, [line, ring, bare.get(x, 310)], "top edge at x {x}");
        let col: Vec<u32> = (326..329).map(|y| on_page.get(x, y)).collect();
        assert_eq!(col, [bare.get(x, 326), ring, line], "bottom edge at x {x}");
    }
    // A value longer than the field leaves the ring's right column to
    // the page: "Reprise Classic" in combos 50 to 80 px wide, so some
    // glyph's ink reaches the text rect's last column.
    let mut long = Combo::new(&["Reprise Classic"], 0, 8);
    long.enabled = false;
    for w in 50..80 {
        let l = layout(
            &long,
            Rect::new(306, 308, w, COMBO_H),
            &theme.look.controls(),
        );
        let mut c = bare.clone();
        draw(
            &mut c,
            &text,
            &p.on_page(page),
            &long,
            &l,
            false,
            false,
            Caret::Unfocused,
            ComboLit::IDLE,
            11,
        );
        let ring_x = l.drop.x - 3;
        for y in 311..326 {
            assert_eq!(
                c.get(ring_x, y),
                bare.get(ring_x, y),
                "{w} px: the ring at y {y} is the page"
            );
        }
    }
    assert_eq!(
        on_page.get(308, 320),
        0xFFF6_F4EE,
        "the page's own pixel at row 320"
    );
}

#[test]
fn a_classic_combo_frame_is_the_field_frame_in_both_states() {
    let theme = Theme::default();
    let p = classic_paint(&theme.colors, 1);
    let r = Rect::new(2, 2, 60, 21);
    for (enabled, fill) in [(true, p.cs.window), (false, p.cs.button_face)] {
        let (mut a, mut b) = (Canvas::new(64, 25), Canvas::new(64, 25));
        let look = ComboLook {
            enabled,
            focused: false,
            ptr: Interaction::Idle,
            editable: false,
        };
        draw_combo_frame(&mut a, &p, r, Rect::new(44, 4, 16, 17), fill, look);
        crate::classic::widgets::draw_field_frame(&mut b, &p, r, fill);
        assert!(
            a == b,
            "Classic's combo frame (enabled {enabled}) is its sunken edge"
        );
    }
}

#[test]
fn an_editable_combo_starts_its_text_6_px_into_the_field() {
    let mut c = Combo::new(&["notepad"], 0, 3);
    c.edit = Some(TextEdit::new("notepad", false));
    let field = Rect::new(24, 119, 192, COMBO_H);
    let l = layout(&c, field, &ControlMetrics::CLASSIC);
    assert_eq!(l.text, Rect::new(30, 122, 166, 15));
}

/// An editable combo over a history: Down from typed text starts at
/// the newest item, a step or a pick puts the item's text in the
/// field selected, the list opens on the item the field holds, and
/// the frame's bottom pixel under the last row in view is no row.
#[test]
fn an_editable_combo_steps_its_field_through_the_items() {
    let mut c = Combo::new(&["notepad", "mspaint", "calc", "cmd"], 0, 3);
    c.edit = Some(TextEdit::new("typed", false));
    let text = |c: &Combo| c.edit.as_ref().unwrap().text.clone();
    assert!(!c.step(false), "Up from typed text does nothing");
    assert!(c.step(true));
    assert_eq!(text(&c), "notepad");
    assert!(c.edit.as_ref().unwrap().all_selected());
    assert!(c.step(true));
    assert_eq!(text(&c), "mspaint");
    assert!(c.toggle());
    assert_eq!((c.open, c.top), (Some(1), 0));
    c.step(true);
    c.step(true);
    assert_eq!((c.open, c.top, text(&c)), (Some(3), 1, "cmd".to_string()));
    let lay = layout(&c, Rect::new(0, 0, 100, COMBO_H), &ControlMetrics::CLASSIC);
    assert_eq!(hit(&c, &lay, 5, lay.list.bottom() - 2), Some(Hit::Row(3)));
    assert_eq!(hit(&c, &lay, 5, lay.list.bottom() - 1), None);
    assert!(c.pick(0));
    assert_eq!((c.open, text(&c)), (None, "notepad".to_string()));
}

#[test]
fn keys_pick_directly_when_closed_and_move_the_highlight_when_open() {
    let mut c = combo();
    assert!(c.step(true));
    assert_eq!(c.selected, 1);
    assert!(!c.step(false) || c.selected == 0);
    assert!(c.jump(true));
    assert_eq!(c.selected, 2);
    assert!(c.toggle());
    assert_eq!(c.open, Some(2));
    assert!(!c.step(false));
    assert_eq!((c.open, c.selected), (Some(1), 2));
    assert!(c.take());
    assert_eq!((c.open, c.selected), (None, 1));
    c.enabled = false;
    assert!(!c.toggle());
    assert!(!c.step(true));
}

#[test]
fn a_greyed_row_is_skipped_refused_and_drawn_grey() {
    let mut c = combo();
    c.items[1].greyed = true;
    assert!(c.step(true));
    assert_eq!(c.selected, 2, "Down steps over the greyed row");
    assert!(c.step(false));
    assert_eq!(c.selected, 0, "and Up back over it");
    c.items[0].greyed = true;
    c.selected = 2;
    assert!(!c.jump(false), "Home stops at the first row not greyed");
    assert_eq!(c.selected, 2);
    assert!(c.toggle());
    assert!(!c.pick(1));
    assert_eq!((c.open, c.selected), (Some(2), 2), "the list stays open");

    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let cs = &theme.colors;
    let rows = c.items.len();
    let list = Rect::new(
        0,
        0,
        120,
        rows as i32 * ControlMetrics::CLASSIC.combo_row + 2,
    );
    let mut canvas = Canvas::new(120, list.height as u32);
    draw_list(
        &mut canvas,
        &text,
        &classic_paint(cs, 1),
        &c.items,
        None,
        0,
        rows,
        list,
        None,
        None,
        11,
    );
    let inks = |row: i32| {
        let r = Rect::new(
            1,
            1 + row * ControlMetrics::CLASSIC.combo_row,
            118,
            ControlMetrics::CLASSIC.combo_row,
        );
        (r.x..r.right())
            .flat_map(|x| (r.y..r.bottom()).map(move |y| (x, y)))
            .map(|(x, y)| canvas.get(x, y))
            .filter(|&p| p != cs.window.argb())
            .collect::<std::collections::HashSet<_>>()
    };
    assert!(inks(1).contains(&cs.gray_text.argb()));
    assert!(!inks(1).contains(&cs.button_text.argb()));
    assert!(inks(2).contains(&cs.button_text.argb()));
}

#[test]
fn draws_the_field_highlight_and_button_pixels() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let cs = &theme.colors;
    let c = combo();
    let field = Rect::new(24, 119, 192, COMBO_H);
    let l = layout(&c, field, &ControlMetrics::CLASSIC);
    let mut canvas = Canvas::new(240, 160);
    canvas.fill_rect(canvas.bounds(), cs.button_face);
    draw(
        &mut canvas,
        &text,
        &classic_paint(cs, 1),
        &c,
        &l,
        true,
        true,
        Caret::Unfocused,
        ComboLit::IDLE,
        11,
    );
    // The sunken edge, the highlight with its dots, the raised button.
    assert_eq!(canvas.get(24, 119), cs.button_shadow.argb());
    assert_eq!(canvas.get(25, 120), cs.button_dk_shadow.argb());
    assert_eq!(canvas.get(26, 121), cs.window.argb());
    assert_eq!(canvas.get(27, 122), Color::rgb(245, 219, 149).argb());
    assert_eq!(canvas.get(28, 122), cs.button_text.argb());
    assert_eq!(canvas.get(100, 130), cs.hilight.argb());
    assert_eq!(canvas.get(198, 121), cs.button_face.argb());
    assert_eq!(canvas.get(199, 122), cs.button_hilight.argb());
    assert_eq!(canvas.get(213, 121), cs.button_dk_shadow.argb());
    assert_eq!(canvas.get(212, 136), cs.button_shadow.argb());
    assert_eq!(canvas.get(205, 127), cs.button_text.argb(), "arrow tip row");
    assert_eq!(canvas.get(215, 130), cs.button_hilight.argb());
    // Without the focus the item is plain; disabled it is face and etched.
    let mut plain = Canvas::new(240, 160);
    draw(
        &mut plain,
        &text,
        &classic_paint(cs, 1),
        &c,
        &l,
        false,
        false,
        Caret::Unfocused,
        ComboLit::IDLE,
        11,
    );
    assert_eq!(plain.get(100, 130), cs.window.argb());
    let mut off = c.clone();
    off.enabled = false;
    let mut dis = Canvas::new(240, 160);
    draw(
        &mut dis,
        &text,
        &classic_paint(cs, 1),
        &off,
        &l,
        false,
        false,
        Caret::Unfocused,
        ComboLit::IDLE,
        11,
    );
    assert_eq!(dis.get(180, 130), cs.button_face.argb());
    assert_eq!(dis.get(205, 127), cs.button_shadow.argb());
    assert_eq!(
        dis.get(209, 128),
        cs.button_hilight.argb(),
        "greyed arrow copy"
    );
    // Focused by a click: the highlight stays, the dots do not. They
    // wait until the keyboard is used, so the highlight's top edge
    // runs solid.
    let mut nocue = Canvas::new(240, 160);
    nocue.fill_rect(nocue.bounds(), cs.button_face);
    draw(
        &mut nocue,
        &text,
        &classic_paint(cs, 1),
        &c,
        &l,
        true,
        false,
        Caret::Unfocused,
        ComboLit::IDLE,
        11,
    );
    assert_eq!(nocue.get(100, 130), cs.hilight.argb(), "still highlighted");
    let edge = l.highlight;
    for x in edge.x..edge.right() {
        assert_eq!(
            nocue.get(x, edge.y),
            cs.hilight.argb(),
            "no dot at {x} on the highlight's top edge"
        );
    }
    // With the cue the same edge is dotted.
    assert!((edge.x..edge.right()).any(|x| canvas.get(x, edge.y) == cs.button_text.argb()));
}

#[test]
fn a_click_at_scale_2_lands_on_the_character_drawn_there() {
    let text = TextRenderer::load(&FontConfig::default());
    let (s, px) = (2, 22);
    let mut c = combo();
    c.edit = Some(TextEdit::new(
        "C:\\Documents and Settings\\All Users",
        false,
    ));
    let lay = layout(&c, Rect::new(0, 0, 120, COMBO_H), &ControlMetrics::CLASSIC);
    let edit = c.edit.as_ref().unwrap();
    // The caret at the end scrolls the start out of view.
    let tr = lay.text.scaled(s);
    let first = edit_first_visible(&text, edit, tr.width, px, 0);
    assert!(first > 0);
    let k = first + 3;
    let dx = tr.x + text.measure(&edit.text[first..k], false, px);
    assert_eq!(index_at(&c, &lay, &text, px, s, dx / s), k);
}

#[test]
fn a_shell_row_highlight_ends_one_px_past_its_label_at_scale_2() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let cs = &theme.colors;
    let (s, px) = (2, 22);
    let items = [ListItem::shell("Harbor", Some(Canvas::new(32, 32)), 0)];
    let mut c = Canvas::new(400, 40);
    let list = Rect::new(0, 0, 200, SHELL_ROW_H + 2);
    draw_list(
        &mut c,
        &text,
        &classic_paint(cs, s),
        &items,
        Some(0),
        0,
        1,
        list,
        None,
        None,
        px,
    );
    // The label starts at the frame, the icon inset and the icon with
    // its gap: (1 + 3 + 16 + 4) px, scaled.
    let tx = (1 + LIST_ICON_X + 16 + ICON_GAP) * s;
    let right = tx + text.measure("Harbor", false, px) + s;
    let mid = SHELL_ROW_H * s / 2;
    assert_ne!(
        c.get(right - 1, mid),
        cs.window.argb(),
        "the ring on the highlight's last column"
    );
    assert_eq!(
        c.get(right, mid),
        cs.window.argb(),
        "white past the highlight"
    );
}

#[test]
fn grown_shell_rows_grow_and_centre_their_icons() {
    let text = TextRenderer::load(&FontConfig::default());
    let red = Color::rgb(0xFF, 0, 0);
    let mut icon = Canvas::new(16, 16);
    icon.fill_rect(Rect::new(0, 0, 16, 16), red);
    let items = [
        ListItem::shell("Desktop", Some(icon.clone()), 0),
        ListItem::shell("Documents", Some(icon), 1),
    ];
    for (pct, row_h) in [(100, 16), (150, 25), (200, 32)] {
        let theme = Theme::default().with_text_size(pct);
        let combo = Combo::with_items(items.to_vec(), 0, 4);
        assert_eq!(combo.row_h(&theme.controls()), row_h, "{pct}%");
        let list = Rect::new(0, 0, 200, 2 * row_h + 2);
        let mut c = Canvas::new(200, list.height as u32);
        let p = Paint {
            text: theme.text,
            ..classic_paint(&theme.colors, 1)
        };
        let px = theme.fonts.size_px;
        draw_list(
            &mut c,
            &text,
            &p,
            &items,
            Some(1),
            0,
            2,
            list,
            None,
            None,
            px,
        );
        let x = 1 + LIST_ICON_X + INDENT_STEP;
        let dy = (row_h - 16) / 2;
        let top = 1 + row_h + dy;
        assert_eq!(c.get(x, top), red.argb(), "{pct}%: icon top");
        assert_eq!(c.get(x, top + 15), red.argb(), "{pct}%: icon bottom");
        if dy > 0 {
            assert_ne!(c.get(x, top - 1), red.argb(), "{pct}%: above the icon");
            assert_ne!(c.get(x, top + 16), red.argb(), "{pct}%: under the icon");
        }
        let tx = x + 16 + ICON_GAP;
        let tw = text.measure("Documents", false, px);
        let lit: Vec<i32> = (0..list.height)
            .filter(|&y| (tx..tx + tw).any(|x| c.get(x, y) == theme.colors.hilight.argb()))
            .collect();
        assert_eq!(lit.first(), Some(&(2 + row_h)), "{pct}%: highlight top");
        assert_eq!(lit.last(), Some(&(2 * row_h)), "{pct}%: highlight bottom");
    }
}

/// A shell list whose icons are 32 px (a grown Look in folder) draws them whole in 32 px
/// rows with the labels past them; 16 px icons keep their 16 px column.
#[test]
fn a_shell_list_widens_its_icon_column_for_larger_icons() {
    let text = TextRenderer::load(&FontConfig::default());
    let red = Color::rgb(0xFF, 0, 0);
    let theme = Theme::default().with_text_size(200);
    let p = Paint {
        text: theme.text,
        ..classic_paint(&theme.colors, 1)
    };
    let px = theme.fonts.size_px;
    for side in [16, 32] {
        let mut icon = Canvas::new(side as u32, side as u32);
        icon.fill_rect(Rect::new(0, 0, side, side), red);
        let items = [ListItem::shell("W", Some(icon), 0)];
        let list = Rect::new(0, 0, 200, 32 + 2);
        let mut c = Canvas::new(200, list.height as u32);
        draw_list(&mut c, &text, &p, &items, None, 0, 1, list, None, None, px);
        let reds: Vec<i32> = (0..200)
            .filter(|&x| (0..list.height).any(|y| c.get(x, y) == red.argb()))
            .collect();
        let x = 1 + LIST_ICON_X;
        let left = x;
        assert_eq!(reds.first(), Some(&left), "{side} px: icon left");
        assert_eq!(
            reds.last(),
            Some(&(left + side - 1)),
            "{side} px: icon right"
        );
        let ink = (left + side..199).find(|&x| {
            (1..list.height - 1).any(|y| c.get(x, y) == theme.colors.button_text.argb())
        });
        assert!(
            ink.is_some_and(|i| i >= x + side + ICON_GAP),
            "{side} px: the label starts past the icon, got {ink:?}"
        );
    }
}

fn run_box(value: &str) -> (Combo, Layout) {
    let mut c = combo();
    c.edit = Some(TextEdit::new(value, false));
    let lay = layout(&c, Rect::new(0, 0, 120, COMBO_H), &ControlMetrics::CLASSIC);
    (c, lay)
}

#[test]
fn text_dropped_on_an_editable_field_lands_at_the_drop_caret_selected() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let (mut c, lay) = run_box("foot");
    assert!(
        !c.drag_over(&lay, lay.drop.x + 2, 10),
        "the drop button takes nothing"
    );
    assert!(c.drop_x.is_none());
    let x = lay.text.x + text.measure("fo", false, px);
    assert!(c.drag_over(&lay, x, 10));
    assert!(c.drop_text(
        &lay,
        &crate::textedit::DropLine::Text("X".into()),
        &text,
        px
    ));
    let edit = c.edit.as_ref().unwrap();
    assert_eq!(edit.text, "foXot");
    assert_eq!(edit.selected_text(), "X");
    assert!(c.drop_x.is_none(), "the drop ends the drag");
}

#[test]
fn a_plain_open_or_greyed_combo_takes_no_drop() {
    let lay = layout(
        &combo(),
        Rect::new(0, 0, 120, COMBO_H),
        &ControlMetrics::CLASSIC,
    );
    assert!(!combo().drag_over(&lay, 20, 10), "a pick list has no caret");
    let (mut c, lay) = run_box("x");
    c.open = Some(0);
    assert!(!c.drag_over(&lay, 20, 10));
    let (mut c, lay) = run_box("x");
    c.enabled = false;
    assert!(!c.drag_over(&lay, 20, 10));
}

#[test]
fn a_drag_resting_at_the_fields_edge_scrolls_it_and_the_drop_caret_shows() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let (mut c, lay) = run_box("/usr/share/applications/org.example.Long.desktop");
    let shot = |c: &Combo| {
        let mut canvas = Canvas::new(120, COMBO_H as u32);
        let caret = Caret::new(false, true);
        draw(
            &mut canvas,
            &text,
            &classic_paint(cs, 1),
            c,
            &lay,
            false,
            false,
            caret,
            ComboLit::IDLE,
            px,
        );
        canvas
    };
    shot(&c);
    let end = c.first.get();
    assert!(end > 0, "the caret at the end scrolled the start away");
    assert!(c.drag_over(&lay, lay.text.x + 1, 10));
    assert_eq!(c.drag_scroll(&lay, &text, px), Some(end - 1));
    assert!(c.drag_tick(&lay, &text, px));
    let before = shot(&c);
    assert_eq!(c.first.get(), end - 1, "the draw keeps the scrolled view");
    let x = lay.text.x + 30;
    c.drag_over(&lay, x, 10);
    let after = shot(&c);
    c.drag_leave();
    let plain = shot(&c);
    assert_eq!(
        c.first.get(),
        end,
        "without a drag the caret pins the view again"
    );
    let changed: Vec<(i32, i32)> = (0..COMBO_H)
        .flat_map(|y| (0..120).map(move |x| (x, y)))
        .filter(|&(x, y)| before.get(x, y) != after.get(x, y))
        .collect();
    let mut cols: Vec<i32> = changed.iter().map(|&(x, _)| x).collect();
    cols.dedup();
    cols.sort();
    cols.dedup();
    assert_eq!(
        cols.len(),
        2,
        "the drop caret moved from one column to another: {cols:?}"
    );
    assert_eq!(
        cols[0], lay.text.x,
        "the field's left edge, where it rested"
    );
    assert!(
        (x - 4..=x + 4).contains(&cols[1]),
        "near the pointer: {cols:?}"
    );
    let bar = changed.iter().filter(|&&(cx, _)| cx == cols[1]);
    assert!(
        bar.clone()
            .all(|&(cx, cy)| after.get(cx, cy) == cs.button_text.argb())
    );
    assert!(bar.count() >= 8, "a line tall");
    assert_ne!(plain.pixels, after.pixels);
}

/// Whether the ink on `back` in `r` ends in dots: the last 7 columns that ink anything (two
/// dots) ink at most two rows, where a letter cut at the edge would ink many.
fn ends_in_dots(c: &Canvas, r: Rect, back: u32) -> bool {
    let inked: Vec<(i32, i32)> = (r.x..r.right())
        .flat_map(|x| (r.y..r.bottom()).map(move |y| (x, y)))
        .filter(|&(x, y)| c.get(x, y) != back)
        .collect();
    let last = inked.iter().map(|p| p.0).max().unwrap_or(r.x);
    let rows: std::collections::HashSet<i32> = inked
        .iter()
        .filter(|p| p.0 > last - 7)
        .map(|p| p.1)
        .collect();
    !inked.is_empty() && rows.len() <= 2
}

#[test]
fn a_value_or_row_wider_than_its_room_ends_in_an_ellipsis() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let cs = &theme.colors;
    let long = "Images (*.png, *.jpg, *.jpeg, *.gif, *.bmp, *.webp, *.svg)";
    let c = Combo::new(&[long, "All Files"], 0, 2);
    let l = layout(&c, Rect::new(0, 0, 160, COMBO_H), &ControlMetrics::CLASSIC);
    let mut canvas = Canvas::new(160, COMBO_H as u32);
    let p = classic_paint(cs, 1);
    draw(
        &mut canvas,
        &text,
        &p,
        &c,
        &l,
        false,
        true,
        Caret::Unfocused,
        ComboLit::IDLE,
        11,
    );
    assert!(ends_in_dots(&canvas, l.text, cs.window.argb()), "the field");

    let list = Rect::new(0, 0, 160, 2 * ControlMetrics::CLASSIC.combo_row + 2);
    let mut canvas = Canvas::new(160, list.height as u32);
    draw_list(
        &mut canvas,
        &text,
        &p,
        &c.items,
        None,
        0,
        2,
        list,
        None,
        None,
        11,
    );
    let first = Rect::new(1, 1, 158, ControlMetrics::CLASSIC.combo_row);
    assert!(
        ends_in_dots(&canvas, first, cs.window.argb()),
        "the first row"
    );
}
