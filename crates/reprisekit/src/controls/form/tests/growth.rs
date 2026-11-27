use super::*;

/// At 200% text a growing picture's 48 px canvas room doubles inside its 2 px frame; a plain
/// picture keeps its size; both move with the text.
#[test]
fn a_growing_picture_doubles_inside_its_kept_frame() {
    let picture = |id, y| {
        Item::new(
            id,
            Rect::new(10, y, 52, 52),
            Control::Picture {
                canvas: None,
                frame: 2,
            },
        )
    };
    let f = Form::new(vec![picture("grows", 20).grows(), picture("keeps", 80)]);
    f.drawn_with(&Theme::default().with_text_size(200).paint());
    assert_eq!(f.rect("grows"), Some(Rect::new(20, 40, 100, 100)));
    assert_eq!(f.rect("keeps"), Some(Rect::new(20, 160, 52, 52)));
    f.drawn_with(&Theme::default().paint());
    assert_eq!(f.rect("grows"), Some(Rect::new(10, 20, 52, 52)), "at 100%");
}

/// The form [`grown_form_draws_rects_with_the_text_and_keeps_glyphs`] draws at each size.
fn sized_form() -> Form {
    let mut f = Form::new(vec![
        Item::group("g", Rect::new(6, 10, 200, 120), "Group"),
        Item::label_at("l", 16, 26, 60, "&Name:"),
        Item::edit("e", Rect::new(80, 22, 110, 21), "text", true),
        Item::check("c", 16, 52, "Check"),
        Item::radio("r", 16, 72, "Radio", "rg"),
        Item::lines_at("ls", 16, 90, 180, &["one", "two"]),
        Item::picture("p", Rect::new(210, 10, 32, 32), Some(Canvas::new(32, 32))),
        Item::combo("cb", 16, 140, 100, &["a", "b", "c"], 3),
        Item::button("ok", Rect::new(130, 140, 75, 23), "OK", true, false),
    ]);
    f.set_check("c", true);
    f
}

/// At 150% and 200% every control's rect grows with the text from its 100% place (both
/// edges, so neighbours stay apart), a picture keeps its size, a check box's 13 px box grows by
/// the text's whole multiple (twice at 200%) centred on its taller row, and the open combo's rows
/// grow; 100% is the rect as laid out.
#[test]
fn grown_form_draws_rects_with_the_text_and_keeps_glyphs() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let text = TextRenderer::load(&FontConfig::default());
    for (pct, px) in [(100, 11), (150, 17), (200, 22)] {
        let u = TextUnits::new(px, 11);
        let mut f = sized_form();
        let mut c = Canvas::new(500, 400);
        c.fill_rect(c.bounds(), cs.button_face);
        let p = Paint {
            text: u,
            ..classic_paint(cs, 1)
        };
        draw(&mut c, &text, &p, &f, false, px);
        for id in ["e", "ok", "g", "cb"] {
            let design = f.item(id).unwrap().rect;
            assert_eq!(f.rect(id), Some(u.rect(design)), "{id} at {pct}%");
        }
        let ok = f.rect("ok").unwrap();
        assert_eq!(
            ok,
            Rect::new(
                u.len(130),
                u.len(140),
                u.len(205) - u.len(130),
                u.len(163) - u.len(140)
            )
        );
        assert_eq!(
            c.get(ok.right() - 1, ok.bottom() - 1),
            cs.button_dk_shadow.argb(),
            "{pct}%: 1 px bevel at the grown corner"
        );
        assert_eq!(
            c.get(ok.right() - 2, ok.bottom() - 2),
            cs.button_shadow.argb(),
            "{pct}%"
        );
        assert_eq!(f.rect("p").unwrap().width, 32, "the picture keeps its size");

        // The check box: 13 px times the text's whole multiple, centred down the row.
        let row = f.rect("c").unwrap();
        assert_eq!(row.height, u.len(52 + CHECK) - u.len(52));
        let side = u.small() * CHECK;
        let by = row.y + (row.height - side) / 2;
        assert_eq!(
            c.get(row.x, by),
            cs.button_shadow.argb(),
            "{pct}%: the box's top-left"
        );
        assert_eq!(
            c.get(row.x + side - 1, by + side - 1),
            cs.button_hilight.argb(),
            "{pct}%: its bottom-right"
        );
        assert_eq!(
            c.get(row.x + side - 1, by + side),
            cs.button_face.argb(),
            "{pct}%: {side} px tall"
        );
        assert_eq!(
            c.get(row.x + side, by + side - 1),
            cs.button_face.argb(),
            "{pct}%: {side} px wide"
        );

        // A click on the grown button's far corner lands on it.
        let hit = f.hit(ok.right() - 1, ok.bottom() - 1, &text, px);
        assert_eq!(hit.map(|h| h.item), f.index_of("ok"), "{pct}%");

        // The open combo's rows hold a line of the grown text.
        f.focus_id("cb");
        f.key(Key::Drop);
        draw(&mut c, &text, &p, &f, false, px);
        let Control::Combo(cb) = &f.item("cb").unwrap().control else {
            unreachable!()
        };
        let lay = combo::layout(cb, f.rect("cb").unwrap(), &f.controls());
        assert_eq!(lay.row_h, u.len(13), "{pct}%");
        assert_eq!(lay.list.height, 3 * u.len(13) + 2);
        let x = lay.list.right() - 3;
        let lit = (lay.list.y..lay.list.bottom())
            .filter(|&y| c.get(x, y) == cs.hilight.argb())
            .count();
        assert_eq!(
            lit as i32,
            u.len(13),
            "{pct}%: the picked row is drawn a grown row tall"
        );
    }
}

/// A grown form grows its calendar, clock and tree with the text: their rects grow from the
/// form's origin, a day is hit at its grown cell, and the tree's rows take the grown pitch.
#[test]
fn a_grown_form_grows_its_calendar_clock_and_tree() {
    let text = TextRenderer::load(&FontConfig::default());
    let cs = &Theme::default().colors;
    let tv = TreeView::new(
        vec![
            treeview::Node::new("One", false),
            treeview::Node::new("Two", false),
        ],
        treeview::Style::Plain,
    );
    let mut f = Form::new(vec![
        Item::new(
            "cal",
            Rect::new(27, 87, calendar::W, calendar::H),
            Control::Calendar(Calendar::new(2026, 9, Some(24), 0)),
        ),
        Item::new(
            "clock",
            Rect::new(225, 68, 132, 100),
            Control::Clock(clockface::Face::default()),
        ),
        Item::new("tree", Rect::new(10, 230, 200, 60), Control::Tree(tv)),
    ]);
    let u = TextUnits::new(22, 11);
    let p = Paint {
        text: u,
        ..classic_paint(cs, 1)
    };
    let mut c = Canvas::new(800, 700);
    draw(&mut c, &text, &p, &f, false, 22);
    for id in ["cal", "clock", "tree"] {
        let design = f.item(id).unwrap().rect;
        assert_eq!(f.rect(id), Some(u.rect(design)), "{id}");
    }
    // The 1st of September 2026: row 0, column 2 of the grown grid.
    let cal = f.rect("cal").unwrap();
    let lay = calendar::layout(cal, &f.controls());
    let first = lay.cell(2);
    let hit = f.hit(first.x + 3, first.y + 3, &text, 22);
    assert_eq!(f.click(hit), Event::Changed("cal"));
    let tree = f.rect("tree").unwrap();
    let row_h = u.len(18);
    let hit = f.hit(tree.x + 40, tree.y + row_h + 2, &text, 22);
    assert!(
        matches!(
            hit,
            Some(Hit {
                part: Part::Tree(treeview::Hit::Row(1)),
                ..
            })
        ),
        "the second row a grown row down, got {hit:?}"
    );
}

#[test]
fn a_grown_rule_grows_along_its_length_only() {
    let text = TextRenderer::load(&FontConfig::default());
    let f = Form::new(vec![
        Item::rule("across", Rect::new(10, 100, 300, 1)),
        Item::rule("down", Rect::new(20, 10, 2, 60)),
    ]);
    let theme = Theme::default();
    let p = Paint {
        text: TextUnits::new(22, 11),
        ..classic_paint(&theme.colors, 1)
    };
    draw(&mut Canvas::new(700, 300), &text, &p, &f, false, 22);
    assert_eq!(f.rect("across"), Some(Rect::new(20, 200, 600, 1)));
    assert_eq!(f.rect("down"), Some(Rect::new(40, 20, 2, 120)));
}

/// At 200% a growing icon list's cells and its bar double, 72x88 and 32 px, and its height takes
/// them: the bar sits as far above the frame's foot as at 100%.
#[test]
fn a_growing_icon_list_doubles_its_cells() {
    let text = TextRenderer::load(&FontConfig::default());
    let icons = iconlist::IconList::new(vec!["a".into(); 8], vec![None; 8], Some(0));
    let r = Rect::new(10, 20, 76, 199);
    let f = Form::new(vec![
        Item::new("icons", r, Control::IconList(Box::new(icons))).grows(),
    ]);
    let theme = Theme::default();
    let u = TextUnits::new(22, 11);
    let p = Paint {
        text: u,
        ..classic_paint(&theme.colors, 1)
    };
    draw(&mut Canvas::new(400, 500), &text, &p, &f, false, 22);
    let grown = f.rect("icons").unwrap();
    assert_eq!(
        grown,
        Rect::new(20, 40, 152, 199 + 4 * 44 + 16),
        "its bar 16 px thicker"
    );
    let cm = ControlMetrics::CLASSIC.grown(u);
    let lay = icon_list_layout(true, grown, &cm);
    assert_eq!((lay.cell, lay.icon, lay.cols), ((72, 88), (4, 12), 2));
    let at_100 = iconlist::layout(r, &ControlMetrics::CLASSIC);
    assert_eq!(
        grown.bottom() - lay.bar.bottom(),
        r.bottom() - at_100.bar.bottom()
    );
    let Control::IconList(l) = &f.items[0].control else {
        unreachable!()
    };
    assert_eq!(
        l.cell_rect(&lay, 5),
        Some(Rect::new(22 + 72, 42 + 88, 72, 88))
    );
}

#[test]
fn a_grown_icon_list_widens_and_keeps_its_bar_at_its_foot() {
    let text = TextRenderer::load(&FontConfig::default());
    let icons = iconlist::IconList::new(vec!["a".into(); 12], vec![None; 12], Some(0));
    let r = Rect::new(10, 20, 76, 199);
    let f = Form::new(vec![Item::new(
        "icons",
        r,
        Control::IconList(Box::new(icons)),
    )]);
    let theme = Theme::default();
    let p = Paint {
        text: TextUnits::new(22, 11),
        ..classic_paint(&theme.colors, 1)
    };
    draw(&mut Canvas::new(400, 300), &text, &p, &f, false, 22);
    let grown = f.rect("icons").unwrap();
    assert_eq!(
        grown,
        Rect::new(20, 40, 152, 199 + 16),
        "taller by the bar's growth"
    );
    let at_100 = iconlist::layout(r, &ControlMetrics::CLASSIC);
    let lay = iconlist::layout(grown, &ControlMetrics::CLASSIC.grown(p.text));
    assert_eq!(lay.bar.bottom() - grown.y, at_100.bar.bottom() - r.y + 16);
    assert_eq!(
        grown.bottom() - lay.bar.bottom(),
        r.bottom() - at_100.bar.bottom()
    );
    assert_eq!(lay.cols, 4, "the width shows more columns");
}

#[test]
fn change_icon_drop_keeps_a_shown_pick_in_view_at_grown_text() {
    use crate::controls::filesheet::{
        CHANGE_ICON_LIST, change_icon_drop, change_icon_form, change_icon_list_mut,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let names: Vec<String> = (0..120).map(|i| format!("i{i}")).collect();
    let list = iconlist::IconList::new(names, vec![None; 120], Some(4 * 17));
    let mut f = change_icon_form("x", list, false);
    let theme = Theme::default();
    let u = TextUnits::new(22, 11);
    let p = Paint {
        text: u,
        ..classic_paint(&theme.colors, 1)
    };
    draw(&mut Canvas::new(900, 900), &text, &p, &f, false, 22);
    let grown = f.rect(CHANGE_ICON_LIST).unwrap();
    let lay = icon_list_layout(false, grown, &ControlMetrics::CLASSIC.grown(u));
    assert_eq!(lay.cols, 14, "the grown list shows 14 columns");
    change_icon_list_mut(&mut f).unwrap().left = 5;
    let l = change_icon_list_mut(&mut f).unwrap().clone();
    assert!(
        l.cell_rect(&lay, l.selected.unwrap()).is_some(),
        "shown before the drop"
    );
    let gone: Vec<String> = (0..60).map(|i| format!("i{i}")).collect();
    change_icon_drop(&mut f, &gone);
    let l = change_icon_list_mut(&mut f).unwrap().clone();
    assert!(
        l.cell_rect(&lay, l.selected.unwrap()).is_some(),
        "still shown after the drop"
    );
    assert_eq!(
        l.left + lay.cols,
        l.columns(),
        "no blank columns past the last icon"
    );
}

/// A form drawn at an origin keeps that offset as it is and grows only its own layout from
/// there.
#[test]
fn a_grown_form_keeps_its_origin() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let text = TextRenderer::load(&FontConfig::default());
    let mut f = sized_form();
    f.origin = (3, 21);
    let u = TextUnits::new(17, 11);
    let p = Paint {
        text: u,
        ..classic_paint(cs, 1)
    };
    draw(&mut Canvas::new(500, 400), &text, &p, &f, false, 17);
    let r = f.rect("ok").unwrap();
    assert_eq!((r.x, r.y), (3 + u.len(130), 21 + u.len(140)));
}

/// A note's rows keep every descender at a text size whose rows grow by a fraction of a pixel:
/// each row is at least the font's line tall, live or greyed, at scale 1 and 2.
#[test]
fn a_grown_notes_rows_keep_their_descenders() {
    crate::pins::fonts();
    let t = tr();
    let cs = &Theme::default().colors;
    let lines = [
        "gypsy jogging",
        "happy puppy",
        "quietly typing",
        "yoga pigs",
        "gulps jumpy",
    ];
    let mut cut = Vec::new();
    for (pct, px) in [(100, 11), (125, 14), (150, 17)] {
        for s in [1, 2] {
            for on in [true, false] {
                let f = Form::new(vec![Item::lines_at("n", 8, 7, 160, &lines).enabled(on)]);
                let p = Paint {
                    text: TextUnits::new(px, 11),
                    ..classic_paint(cs, s)
                };
                let mut c = Canvas::new(400 * s as u32, 200 * s as u32);
                let ((), cuts) =
                    crate::pins::clipped_ink(|| draw(&mut c, &t, &p, &f, false, px * s as u32));
                cut.extend(cuts.into_iter().map(|c| (pct, s, on, c.text, c.rows)));
            }
        }
    }
    assert_eq!(
        cut,
        Vec::new(),
        "(text size, scale, live, line, ink rows cut)"
    );
}

/// A label, a check box's and a radio's label, a group title and a group box's label keep every
/// descender at 100%, 125% and 150% text, live or greyed, at scale 1 and 2, on every row a
/// fractional text size rounds short.
#[test]
fn grown_labels_keep_their_descenders() {
    crate::pins::fonts();
    let t = tr();
    let cs = &Theme::default().colors;
    let mut cut = Vec::new();
    for (pct, px) in [(100, 11), (125, 14), (150, 17)] {
        for s in [1, 2] {
            for on in [true, false] {
                for dy in 0..4 {
                    let f = Form::new(vec![
                        Item::group("g", Rect::new(6, 10 + dy, 220, 150), "Group gypsy"),
                        Item::group_title("t", 16, 26 + dy, 180, "Title gypsy"),
                        Item::label_at("l", 16, 44 + dy, 180, "&Label gypsy").enabled(on),
                        Item::check("c", 16, 62 + dy, "&Check gypsy").enabled(on),
                        Item::radio("r", 16, 80 + dy, "&Radio gypsy", "rg").enabled(on),
                    ]);
                    let p = Paint {
                        text: TextUnits::new(px, 11),
                        ..classic_paint(cs, s)
                    };
                    let mut c = Canvas::new(500 * s as u32, 400 * s as u32);
                    let ((), cuts) =
                        crate::pins::clipped_ink(|| draw(&mut c, &t, &p, &f, false, px * s as u32));
                    cut.extend(cuts.into_iter().map(|c| (pct, s, on, dy, c.text, c.rows)));
                }
            }
        }
    }
    assert_eq!(
        cut,
        Vec::new(),
        "(text size, scale, live, rows down, label, ink rows cut)"
    );
}

/// A program draws its form on a canvas at the output's scale: a label as wide as its ink at
/// scale 1 keeps all of it at scale 2, in Tahoma and in DejaVu Sans.
#[test]
fn labels_fitted_to_their_ink_keep_it_on_a_scale_2_canvas() {
    crate::pins::fonts();
    let cs = &Theme::default().colors;
    let words = [
        "Documents icon from",
        "Click OK",
        "Status: Idle",
        "Default",
        "Memory",
        "Wireless network",
        "Share this folder",
        "Save As",
        "Format",
        "Read-only",
        "Advanced",
        "Archive",
        "Size on disk",
        "Type of file",
        "Opens with",
        "Network",
    ];
    let mut cut = Vec::new();
    for family in ["Tahoma", "DejaVu Sans"] {
        let t = TextRenderer::load(&FontConfig {
            family: vec![family.into()],
            ..FontConfig::default()
        });
        let px = 11;
        for w in words {
            let f = Form::new(vec![Item::label_at(
                "l",
                16,
                44,
                t.ink_width(w, false, px),
                w,
            )]);
            let p = Paint {
                text: TextUnits::new(px, 11),
                ..classic_paint(cs, 1)
            };
            for s in [1, 2] {
                let mut c = Canvas::with_scale(300, 100, s);
                let ((), cuts) = crate::pins::clipped_ink(|| draw(&mut c, &t, &p, &f, false, px));
                assert!(
                    f.rendered.ellipsized.borrow().is_empty(),
                    "{family}: {w:?} fits its ink"
                );
                cut.extend(cuts.into_iter().map(|c| (family, s, c.text, c.cols)));
            }
        }
    }
    assert_eq!(cut, Vec::new(), "(font, scale, label, ink columns cut)");
}

/// A label, a check box's and a radio's label, a group title, a group box's label and a note
/// keep a `|` whole: Tahoma's hangs a row past the font's descent at 14 and 22 px. At every text
/// size, drawn at scale 2 by the paint and by the canvas.
#[test]
fn a_bar_hanging_past_the_descent_keeps_its_ink() {
    crate::pins::fonts();
    let t = tr();
    let cs = &Theme::default().colors;
    let mut cut = Vec::new();
    for pct in [100, 125, 150, 175, 200] {
        let px = crate::theme::text_px(11, pct);
        for (s, by_canvas) in [(1, false), (2, false), (2, true)] {
            let f = Form::new(vec![
                Item::group("g", Rect::new(6, 10, 400, 200), "Group 1|1"),
                Item::group_title("t", 16, 30, 380, "Title 1|1"),
                Item::label_at("l", 16, 50, 380, "Color|Solid"),
                Item::check("c", 16, 70, "Check 1|1"),
                Item::radio("r", 16, 90, "Radio 1|1", "rg"),
                Item::lines_at("n", 16, 110, 380, &["e.g. 1|1 2|1 1:2|1:2"]),
            ]);
            let (ps, cs_) = if by_canvas { (1, s) } else { (s, 1) };
            let p = Paint {
                text: TextUnits::new(px, 11),
                ..classic_paint(cs, ps)
            };
            let mut c = Canvas::with_scale(900 * ps as u32, 600 * ps as u32, cs_);
            let ((), cuts) =
                crate::pins::clipped_ink(|| draw(&mut c, &t, &p, &f, false, px * ps as u32));
            cut.extend(
                cuts.into_iter()
                    .map(|c| (pct, s, by_canvas, c.text, c.rows)),
            );
        }
    }
    assert_eq!(
        cut,
        Vec::new(),
        "(text size, scale, by the canvas, line, ink rows cut)"
    );
}

/// A canvas at 1x drawn at scale 2 with twice the px widens a label wider than its rect, and a
/// button wider than its label room, as scale 1 does, by as much as the text drawn at twice the
/// px needs, so neither is shortened.
#[test]
fn a_form_fits_its_text_at_scale_1_and_2() {
    crate::pins::fonts();
    let t = tr();
    let f = Form::new(vec![
        Item::label_at("l", 10, 14, 30, "A long label:"),
        Item::edit("e", Rect::new(44, 10, 120, 21), "", true),
        Item::button(
            "b",
            Rect::new(10, 40, 40, 23),
            "Restore Defaults",
            false,
            false,
        ),
    ]);
    let laid = |s: i32| {
        let theme = Theme::default().scaled(s);
        let mut c = Canvas::new(200 * s as u32, 80 * s as u32);
        let ((), short) = crate::pins::shortened_text(|| {
            draw(&mut c, &t, &theme.paint(), &f, false, theme.fonts.size_px)
        });
        assert_eq!(short, Vec::new(), "nothing shortened at scale {s}");
        ["l", "e", "b"].map(|id| f.rect(id).unwrap())
    };
    let [l, e, b] = laid(1);
    assert_ne!(e, Rect::new(44, 10, 120, 21), "the label moves the field");
    assert_ne!(b, Rect::new(10, 40, 40, 23), "the button widens");
    let [l2, e2, b2] = laid(2);
    assert!(
        l2.width >= l.width && b2.width >= b.width,
        "as wide at scale 2"
    );
    assert_eq!(
        e2.right(),
        e.right(),
        "the field gives up width on its left"
    );
}

/// A label, a check box's and a radio's label whose first glyph inks left of its pen (DejaVu
/// Sans' J) move right to keep that ink, drawn at scale 2 by the paint and by the canvas;
/// Tahoma's J starts at its pen.
#[test]
fn a_label_keeps_a_first_glyph_that_hooks_left_of_its_pen() {
    crate::pins::fonts();
    assert_eq!(tr().ink_before("Join it", false, 11), 0);
    let t = TextRenderer::load(&FontConfig {
        family: vec!["DejaVu Sans".into()],
        ..FontConfig::default()
    });
    assert!(t.ink_before("Join it", false, 11) > 0);
    let cs = &Theme::default().colors;
    let f = Form::new(vec![
        Item::radio("r", 16, 20, "&Join it", "g"),
        Item::check("c", 16, 40, "Jobs").enabled(false),
        Item::label_at("l", 16, 60, 60, "Jo Doe"),
    ]);
    let mut cut = Vec::new();
    for s in [1, 2] {
        let by_paint = (
            classic_paint(cs, s),
            Canvas::new(300 * s as u32, 100 * s as u32),
        );
        let by_canvas = (classic_paint(cs, 1), Canvas::with_scale(300, 100, s));
        for (how, (p, mut c)) in [("paint", by_paint), ("canvas", by_canvas)] {
            let px = 11 * p.s as u32;
            let ((), cuts) = crate::pins::clipped_ink(|| draw(&mut c, &t, &p, &f, false, px));
            cut.extend(cuts.into_iter().map(|c| (s, how, c.text, c.cols)));
        }
    }
    assert_eq!(cut, Vec::new(), "(scale, drawn by, label, ink columns cut)");
}

/// At 200% text the Appearance sample's box doubles with the form and its windows fill it, in
/// each look at scale 1 and 2.
#[test]
fn the_appearance_sample_grows_with_the_text_in_every_look() {
    use crate::interface::Interface;
    let mut got = Vec::new();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = crate::interfaces::theme_for(i).with_text_size(200);
        let f = Form::new(vec![Item::new(
            "sample",
            Rect::new(0, 0, 350, 188),
            Control::Sample {
                themes: false,
                theme: theme.clone(),
                icon: None,
            },
        )]);
        for s in [1, 2] {
            let theme = theme.scaled(s);
            let text = TextRenderer::load(&theme.fonts);
            let mut c = Canvas::new(700 * s as u32, 376 * s as u32);
            draw(
                &mut c,
                &text,
                &theme.paint(),
                &f,
                false,
                theme.fonts.size_px,
            );
            assert_eq!(f.rect("sample"), Some(Rect::new(0, 0, 700, 376)), "{i:?}");
            let corner = (696 * s, 372 * s);
            assert_eq!(
                c.get(corner.0, corner.1),
                theme.colors.background.argb(),
                "{i:?} @{s}: the desktop reaches the box's bottom right"
            );
            got.push((format!("{i:?} {s}"), crate::pins::fnv(&c)));
        }
    }
    crate::pins::check("form/sample_text_200", &got);
}

/// The small controls a form holds: check boxes in each state, radio buttons, a drop-down list,
/// a spinner, a checked list long enough for its scrollbar, and a tree with expanders.
fn small_controls_form() -> Form {
    let mut list = ListBox::new((1..=8).map(|i| format!("Item {i}")).collect(), Some(1));
    list.checks = vec![true, false, true, false, true, false, true, false];
    let mut open = treeview::Node::new("Open", true);
    open.expanded = true;
    open.children = vec![
        treeview::Node::new("Folded", true),
        treeview::Node::new("Leaf", false),
    ];
    let tree = TreeView::new(
        vec![open, treeview::Node::new("Closed", true)],
        treeview::Style::Classic,
    );
    let mut f = Form::new(vec![
        Item::check("on", 10, 10, "On"),
        Item::check("off", 10, 30, "Off"),
        Item::check("greyed", 10, 50, "Greyed").enabled(false),
        Item::radio("r1", 90, 10, "One", "rg"),
        Item::radio("r2", 90, 30, "Two", "rg"),
        Item::combo("cb", 10, 72, 100, &["Picked", "Other"], 2),
        Item::new(
            "spin",
            Rect::new(120, 72, 50, 20),
            Control::UpDown(UpDown::new(3, 0, 9)),
        ),
        Item::new(
            "list",
            Rect::new(10, 100, 120, 60),
            Control::ListBox {
                list,
                icons: Vec::new(),
            },
        ),
        Item::new("tree", Rect::new(140, 100, 100, 60), Control::Tree(tree)),
    ]);
    f.set_check("on", true);
    f.set_check("r2", true);
    f
}

/// At 200% text the check boxes, radio buttons, drop button, spin arrows, scrollbar and tree
/// expanders draw at twice their 100% size in every look, at scale 1 and 2.
#[test]
fn a_form_grows_its_small_controls_twice_at_200_percent_in_every_look() {
    use crate::interface::Interface;
    crate::pins::fonts();
    let mut got = Vec::new();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        for s in [1, 2] {
            let theme = crate::interfaces::theme_for(i)
                .with_text_size(200)
                .scaled(s);
            let text = TextRenderer::load(&theme.fonts);
            let f = small_controls_form();
            let mut c = Canvas::new(500 * s as u32, 330 * s as u32);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            draw(
                &mut c,
                &text,
                &theme.paint(),
                &f,
                false,
                theme.fonts.size_px,
            );
            let at_100 = theme.look.controls_at_100();
            let cm = f.controls();
            assert_eq!(
                (cm.check, cm.radio, cm.scroll, cm.combo_drop),
                (
                    2 * at_100.check,
                    2 * at_100.radio,
                    2 * at_100.scroll,
                    2 * at_100.combo_drop
                ),
                "{i:?}: the small controls are twice their 100% size"
            );
            got.push((format!("{i:?}@{s}"), crate::pins::fnv(&c)));
        }
    }
    crate::pins::check("form/small_controls_200", &got);
}

/// Classic's check box at 200% text is its 100% box drawn twice as large, pixel for pixel, and its
/// scrollbar is twice as thick, its arrow buttons the 100% ones doubled.
#[test]
fn classic_draws_its_check_box_and_scrollbar_doubled_at_200_percent() {
    use crate::look::widgets::{draw_checkbox, draw_scrollbar};
    let theme = Theme::default();
    let big = theme.with_text_size(200);
    let (p, pb) = (theme.paint(), big.paint());
    let doubled = Paint { s: 2, ..p };
    let (cm, cmb) = (theme.controls(), big.controls());
    assert_eq!(cmb.check, 2 * cm.check, "the box is 26 px");
    assert_eq!(cmb.scroll, 2 * cm.scroll, "the bar is 32 px thick");
    for on in [false, true] {
        let b = Rect::new(0, 0, cmb.check, cmb.check);
        let (mut grown, mut want) = (Canvas::new(26, 26), Canvas::new(26, 26));
        draw_checkbox(&mut grown, &pb, b, on, Interaction::Idle);
        draw_checkbox(&mut want, &doubled, b, on, Interaction::Idle);
        assert!(grown == want, "checked {on}: the 13 px box doubled");
    }
    let sb = Rect::new(0, 0, cmb.scroll, 200);
    let (mut grown, mut want) = (Canvas::new(32, 200), Canvas::new(32, 200));
    draw_scrollbar(&mut grown, &pb, sb, 0, 10, 10, None);
    draw_scrollbar(&mut want, &doubled, sb, 0, 10, 10, None);
    assert!(
        grown == want,
        "a bar with everything in view is the 16 px bar doubled"
    );
    assert_eq!(
        grown.get(31, 31),
        theme.colors.button_dk_shadow.argb(),
        "the up arrow's button is 32 px square"
    );
}

/// At 200% text a form's trackbars, ticked and plain, enabled and greyed, draw their thumbs,
/// channels and ticks twice their 100% size in every look, at scale 1 and 2, and the click on the
/// grown thumb's far corner grabs it.
#[test]
fn a_forms_trackbars_grow_twice_at_200_percent_in_every_look() {
    use crate::interface::Interface;
    crate::pins::fonts();
    let mut got = Vec::new();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        for s in [1, 2] {
            let theme = crate::interfaces::theme_for(i)
                .with_text_size(200)
                .scaled(s);
            let text = TextRenderer::load(&theme.fonts);
            let track = |id, y, tb| {
                Item::new(
                    id,
                    Rect::new(10, y, 120, trackbar::HEIGHT),
                    Control::TrackBar(tb),
                )
            };
            let f = Form::new(vec![
                track("ticked", 10, TrackBar::new(3, 11)),
                track("plain", 40, TrackBar::new(10, 11).untick()),
                track("greyed", 70, TrackBar::new(0, 11)).enabled(false),
            ]);
            let mut c = Canvas::new(280 * s as u32, 200 * s as u32);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            draw(
                &mut c,
                &text,
                &theme.paint(),
                &f,
                false,
                theme.fonts.size_px,
            );
            let r = f.rect("ticked").unwrap();
            let Some(Control::TrackBar(t)) = f.item("ticked").map(|it| &it.control) else {
                unreachable!()
            };
            let lay = f.track_layout(t, r);
            assert_eq!(
                (lay.thumb.width, lay.thumb.height, lay.bounds.height),
                (22, 42, r.height),
                "{i:?}: the thumb is twice its 100% size and the trackbar fills its row"
            );
            let corner = (lay.thumb.right() - 1, lay.thumb.bottom() - 1);
            assert_eq!(
                f.hit(corner.0, corner.1, &text, theme.fonts.size_px)
                    .map(|h| (h.item, h.part)),
                Some((0, Part::TrackBar(trackbar::Hit::Thumb))),
                "{i:?}: the grown thumb's far corner"
            );
            got.push((format!("{i:?}@{s}"), crate::pins::fnv(&c)));
        }
    }
    crate::pins::check("form/trackbars_200", &got);
}

/// A combo sized for its rows at 1x widens in the fit for a longer row, and that row is drawn
/// whole, neither cut nor shortened, at scale 2 by the paint and by the canvas, at every text
/// size.
#[test]
fn a_combo_fitted_to_its_widest_row_keeps_it_at_scale_2() {
    crate::pins::fonts();
    let t = tr();
    let rows = [
        "Every 5 minutes",
        "At log on",
        "Once per startup, at log on",
    ];
    let mut cut = Vec::new();
    for pct in [100, 125, 150, 175, 200] {
        for (s, by_canvas) in [(1, false), (2, false), (2, true)] {
            let mut f = Form::new(vec![
                Item::label_at("l", 10, 14, 60, "Run:"),
                Item::combo("c", 80, 10, 100, &rows, rows.len()),
                Item::edit("e", Rect::new(80, 40, 300, 21), "", true),
            ]);
            if let Some(Control::Combo(c)) = f.item_mut("c").map(|it| &mut it.control) {
                c.selected = 2;
            }
            let (ps, cs) = if by_canvas { (1, s) } else { (s, 1) };
            let theme = Theme::default().with_text_size(pct).scaled(ps);
            let mut c = Canvas::with_scale(600 * ps as u32, 120 * ps as u32, cs);
            let (((), short), cuts) = crate::pins::clipped_ink(|| {
                crate::pins::shortened_text(|| {
                    draw(&mut c, &t, &theme.paint(), &f, false, theme.fonts.size_px)
                })
            });
            cut.extend(cuts.into_iter().map(|c| (pct, s, by_canvas, c.text)));
            cut.extend(short.into_iter().map(|c| (pct, s, by_canvas, c.text)));
        }
    }
    assert_eq!(
        cut,
        Vec::new(),
        "(text size, scale, by the canvas, row cut or shortened)"
    );
}
