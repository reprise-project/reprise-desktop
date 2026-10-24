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
