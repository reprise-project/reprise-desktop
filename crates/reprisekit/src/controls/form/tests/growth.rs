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
/// edges, so neighbours stay apart), a picture keeps its size, a check box keeps its 13 px
/// box centred on its taller row, and the open combo's rows grow; 100% is the rect as laid out.
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

        // The check box: 13 px, its top half the row's growth below the row's.
        let row = f.rect("c").unwrap();
        assert_eq!(row.height, u.len(52 + CHECK) - u.len(52));
        let by = row.y + (row.height - CHECK) / 2;
        assert_eq!(
            c.get(row.x, by),
            cs.button_shadow.argb(),
            "{pct}%: the box's top-left"
        );
        assert_eq!(
            c.get(row.x + 12, by + 12),
            cs.button_hilight.argb(),
            "{pct}%: its bottom-right"
        );
        assert_eq!(
            c.get(row.x + 12, by + 13),
            cs.button_face.argb(),
            "{pct}%: 13 px tall"
        );
        assert_eq!(
            c.get(row.x + 13, by + 12),
            cs.button_face.argb(),
            "{pct}%: 13 px wide"
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

/// At 200% a growing icon list's cells double, 72x88, and its height takes them: the bar
/// sits as far above the frame's foot as at 100%.
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
    assert_eq!(grown, Rect::new(20, 40, 152, 199 + 4 * 44));
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
    assert_eq!(grown, Rect::new(20, 40, 152, 199));
    let at_100 = iconlist::layout(r, &ControlMetrics::CLASSIC);
    let lay = iconlist::layout(grown, &ControlMetrics::CLASSIC);
    assert_eq!(lay.bar.bottom() - grown.y, at_100.bar.bottom() - r.y);
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

/// A canvas at 1x drawn at scale 2 with twice the px lays a label wider than its rect, and a
/// button wider than its label room, out as scale 1 does.
#[test]
fn a_form_fits_its_text_alike_at_scale_1_and_2() {
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
        draw(&mut c, &t, &theme.paint(), &f, false, theme.fonts.size_px);
        ["l", "e", "b"].map(|id| f.rect(id))
    };
    let one = laid(1);
    assert_ne!(
        one[1],
        Some(Rect::new(44, 10, 120, 21)),
        "the label moves the field"
    );
    assert_ne!(one[2], Some(Rect::new(10, 40, 40, 23)), "the button widens");
    assert_eq!(laid(2), one);
}
