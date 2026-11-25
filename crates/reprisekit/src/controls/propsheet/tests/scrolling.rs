use super::*;

/// A 398x424 sheet whose page's controls reach its bottom.
fn tall_sheet() -> PropSheet {
    let page = Form::new(vec![
        Item::check("top", 20, 50, "Top"),
        Item::check("low", 20, 424 - PAGE_BOTTOM - PAGE_EDGE - 13, "Low"),
    ]);
    PropSheet::new(&["General"], vec![page], (398, 424))
}

/// At 150% and 200% a tabbed sheet grows from its origin: the client, the page and the
/// buttons grow with the text, the tabs are as tall as the grown row with the open one
/// running into the page, and a click on a grown button lands on it. 100% is the layout as
/// built.
#[test]
fn a_grown_sheet_lays_its_page_tabs_and_buttons_out_with_the_text() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    for px in [11, 17, 22] {
        let u = TextUnits::new(px, 11);
        let mut s = sheet(&["General", "Sharing"], (398, 424));
        s.place((3, 21));
        let p = Paint {
            text: u,
            ..theme.paint()
        };
        s.drawn_with(&p);
        assert_eq!(s.client(), (u.len(398), u.len(424)));
        let l = s.layout(&text, px);
        let open = l.tabs[0];
        assert_eq!(open.height, u.len(tabstrip::TAB_H) + 2, "{px}");
        assert_eq!(
            open.bottom(),
            l.page.y,
            "{px}: the open tab runs into the page"
        );
        assert_eq!(l.page.x, 3 + u.len(MARGIN));
        assert_eq!(l.page.right(), 3 + u.len(398 - MARGIN));
        assert_eq!(l.apply.right(), 3 + u.len(398 - MARGIN));
        assert_eq!(l.apply.bottom(), 21 + u.len(424 - BUTTON_BOTTOM));
        let apply = s.open().rect(APPLY).unwrap();
        assert_eq!(
            apply, l.apply,
            "{px}: the page's button is where the layout says"
        );
        let mut c = Canvas::new(3 + u.len(398) as u32, 21 + u.len(424) as u32);
        draw(&mut c, &text, &p, &s, false, px);
        let ok = l.ok;
        let hit = s.hit(ok.x + ok.width / 2, ok.y + ok.height / 2, &text, px);
        assert!(matches!(hit, Some(Hit::Control(_))), "{px}");
        assert_eq!(s.click(hit), Event::Redraw);
        assert_eq!(s.release(), Event::Ok, "{px}");
    }
}

/// A list sheet at 150%: the list's rows grow with the text and its frame from the origin.
#[test]
fn a_grown_list_sheet_grows_its_rows() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let u = TextUnits::new(17, 11);
    let s = list_sheet();
    s.drawn_with(&Paint {
        text: u,
        ..theme.paint()
    });
    let l = s.layout(&text, 17);
    let list = l.list.unwrap();
    assert_eq!(list.row_h, u.len(17));
    assert_eq!(list.rows[1].y - list.rows[0].y, u.len(17));
    assert_eq!(list.frame.x, u.len(MARGIN));
    assert_eq!(l.page.x, u.len(pagelist::ADVANCE + MARGIN));
}

/// A 398x424 sheet shown 300 tall at 200%: the buttons keep to the client's bottom, the page
/// ends above them with a scrollbar, a control below the page's end neither draws nor takes
/// the pointer there, and the wheel or Tab onto it scrolls it into view, no further.
#[test]
fn a_short_client_keeps_its_buttons_and_scrolls_the_page() {
    let theme = Theme::default().with_form_text(200);
    let (u, px) = (theme.text, theme.fonts.size_px);
    let text = TextRenderer::load(&FontConfig::default());
    let p = theme.paint();
    let page = Form::new(vec![
        Item::check("top", 20, 50, "Top"),
        Item::check("low", 20, 350, "Low"),
    ]);
    let mut s = PropSheet::new(&["General"], vec![page], (398, 424));
    s.drawn_with(&p);
    assert!(s.fit((398, 300)));
    assert!(!s.fit((398, 300)), "idempotent");
    assert_eq!(
        s.size,
        (398, 424),
        "the pages keep the size they were built for"
    );
    assert_eq!(s.client(), (u.len(398), u.len(300)));
    let l = s.layout(&text, px);
    assert_eq!(l.ok.bottom(), u.len(300 - BUTTON_BOTTOM));
    assert_eq!(l.page.bottom(), u.len(300 - PAGE_BOTTOM));
    let bar = l.bar.expect("a page that shows in part has a scrollbar");
    assert_eq!(bar.right(), l.page.right() - PAGE_EDGE);
    let low = s.open().rect("low").unwrap();
    assert!(low.y > l.page.bottom(), "Low starts below the page's end");
    assert!(
        s.hit(low.x + 4, low.y + low.height / 2, &text, px)
            .is_none(),
        "Low takes no pointer over the buttons' row"
    );
    let mut c = Canvas::new(u.len(398) as u32, u.len(300) as u32);
    draw(&mut c, &text, &p, &s, false, px);

    assert_eq!(s.wheel(None, 100), Event::Redraw);
    assert_eq!(
        s.page_offset(),
        (0, -101),
        "scrolled until Low's bottom meets the page's"
    );
    let low = s.open().rect("low").unwrap();
    assert!(
        low.y >= l.page.y && low.bottom() <= l.page.bottom(),
        "{low:?} shows"
    );
    let hit = s.hit(low.x + 4, low.y + low.height / 2, &text, px);
    assert!(
        matches!(hit, Some(Hit::Control(_))),
        "Low takes the pointer"
    );
    assert_eq!(s.open().rect(OK).unwrap(), l.ok, "the buttons stay put");

    assert_eq!(s.wheel(None, -100), Event::Redraw);
    s.open_mut().focus_id("top");
    s.key(form::Key::Tab { back: false }, None);
    assert_eq!(s.open().focus().map(|i| s.open().items[i].id), Some("low"));
    let low = s.open().rect("low").unwrap();
    assert!(
        low.bottom() <= l.page.bottom(),
        "Tab onto Low brings it into view"
    );

    assert!(s.fit((398, 424)));
    assert_eq!(s.page_offset(), (0, 0), "a whole sheet does not scroll");
    assert_eq!(s.layout(&text, px).bar, None);
}

/// A note whose lines rewrap onto one more line than its rect holds pushes the controls under
/// it down a line, and the page scrolls through that line though the sheet is whole.
#[test]
fn a_rewrapped_note_pushes_the_page_down_and_scrolls_it() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let (px, p) = (theme.fonts.size_px, theme.paint());
    // The second line is the wider, so in any face it no longer fits once the first line's
    // last word wraps onto it.
    let lines = [
        "Sunset and sunrise are worked out",
        "for the time zone's city, not this machine's.",
    ];
    assert!(text.measure(lines[1], false, px) > text.measure(lines[0], false, px));
    let w = text.measure(lines[0], false, px) - 20;
    let page = Form::new(vec![
        Item::lines("note", Rect::new(20, 340, w, 2 * LINE_H), &lines),
        Item::check("low", 20, 373, "Low"),
    ]);
    let mut s = PropSheet::new(&["General"], vec![page], (398, 424));
    s.drawn_with(&p);
    assert!(!s.fit((398, 424)), "the sheet is whole");
    let mut c = Canvas::new(398, 424);
    draw(&mut c, &text, &p, &s, false, px);
    let note = s.open().rect("note").unwrap();
    assert_eq!(note.height, 3 * LINE_H, "the note takes its third line");
    assert_eq!(s.open().rect("low").unwrap().y, 373 + LINE_H);
    assert_eq!(s.open().rect(OK).unwrap(), s.layout(&text, px).ok);
    assert!(s.layout(&text, px).bar.is_some(), "the page scrolls");
    assert_eq!(s.page_span().1 - s.page_span().0, LINE_H);
    assert_eq!(s.wheel(None, 100), Event::Redraw);
    assert_eq!(s.page_offset().1, -LINE_H, "through the pushed line");

    let roomy = Form::new(vec![
        Item::lines("note", Rect::new(20, 340, 360, 2 * LINE_H), &lines),
        Item::check("low", 20, 373, "Low"),
    ]);
    s.set_page(0, roomy);
    draw(&mut c, &text, &p, &s, false, px);
    s.fit((398, 424));
    assert_eq!(s.page_offset().1, 0, "nothing left to scroll");
    assert_eq!(
        s.open().rect("low").unwrap().y,
        373,
        "a note that fits moves nothing"
    );
    assert_eq!(s.layout(&text, px).bar, None);
}

/// A press on the page's scrollbar arrow scrolls a line; its thumb drags the page.
#[test]
fn the_page_scrollbar_scrolls_the_page() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = tall_sheet();
    s.drawn_with(&Theme::default().paint());
    s.fit((398, 300));
    let bar = s.layout(&text, px).bar.unwrap();
    let down = s.hit(bar.x + 2, bar.bottom() - 3, &text, px);
    assert_eq!(down, Some(Hit::PageBar(BarHit::Forward)));
    assert_eq!(s.click(down), Event::Redraw);
    assert_eq!(s.page_offset().1, -LINE_H);
    s.release();
    let parts = s.bar_parts(bar);
    let grab = s.hit(bar.x + 2, parts.thumb.unwrap().y + 2, &text, px);
    assert!(
        matches!(grab, Some(Hit::PageBar(BarHit::Thumb(_)))),
        "{grab:?}"
    );
    s.click(grab);
    assert_eq!(s.motion(bar.x + 2, bar.bottom(), None), Event::Redraw);
    assert_eq!(s.page_offset().1, -124, "dragged to the end");
    s.release();
}

/// The page's arrow held repeats on the host's ticks like the kit's other scrollbars.
#[test]
fn a_held_page_arrow_repeats_on_the_hosts_ticks() {
    use crate::controls::scrollbar::{REPEAT_AFTER, REPEAT_EVERY};
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = tall_sheet();
    s.drawn_with(&Theme::default().paint());
    s.fit((398, 300));
    let bar = s.layout(&text, px).bar.unwrap();
    let (x, y) = (bar.x + 2, bar.bottom() - 3);
    let down = s.hit(x, y, &text, px);
    s.motion(x, y, down);
    assert!(!s.ticking());
    s.click(down);
    assert!(s.ticking());
    assert_eq!(s.page_offset().1, -LINE_H);
    let t0 = std::time::Instant::now();
    assert_eq!(s.tick(t0), Event::None, "the pause starts");
    assert_eq!(s.tick(t0 + REPEAT_AFTER), Event::Redraw);
    assert_eq!(s.page_offset().1, -2 * LINE_H);
    assert_eq!(s.tick(t0 + REPEAT_AFTER + REPEAT_EVERY), Event::Redraw);
    assert_eq!(s.page_offset().1, -3 * LINE_H);
    s.motion(x - 40, y, None);
    assert_eq!(
        s.tick(t0 + REPEAT_AFTER + 2 * REPEAT_EVERY),
        Event::None,
        "off the arrow"
    );
    s.release();
    assert!(!s.ticking());
}

/// A 398x424 sheet shown 300 wide at 200%: the buttons keep to the client's bottom-right
/// corner, the page ends at its right edge with a bar along its bottom, and the bar's arrow,
/// or Tab onto a control past that edge, scrolls the page sideways.
#[test]
fn a_narrow_client_keeps_its_buttons_and_scrolls_the_page_sideways() {
    let theme = Theme::default().with_form_text(200);
    let (u, px) = (theme.text, theme.fonts.size_px);
    let text = TextRenderer::load(&FontConfig::default());
    let p = theme.paint();
    let page = Form::new(vec![
        Item::check("left", 20, 50, "Left"),
        Item::button("right", Rect::new(300, 50, 75, 23), "Right", true, false),
    ]);
    let mut s = PropSheet::new(&["General"], vec![page], (398, 424));
    s.drawn_with(&p);
    assert!(s.fit((300, 424)));
    assert!(!s.fit((300, 424)), "idempotent");
    assert_eq!(
        s.size,
        (398, 424),
        "the pages keep the size they were built for"
    );
    assert_eq!(s.client(), (u.len(300), u.len(424)));
    let l = s.layout(&text, px);
    assert_eq!(l.apply.right(), u.len(300 - MARGIN));
    assert_eq!(l.page.right(), u.len(300 - MARGIN));
    assert_eq!(l.bar, None, "the page shows whole down");
    let hbar = l.hbar.expect("a page cut narrow has a sideways bar");
    assert_eq!(hbar.bottom(), l.page.bottom() - PAGE_EDGE);
    assert_eq!(hbar.right(), l.page.right() - PAGE_EDGE);
    let mut c = Canvas::new(u.len(300) as u32, u.len(424) as u32);
    draw(&mut c, &text, &p, &s, false, px);
    let right = s.open().rect("right").unwrap();
    assert!(
        right.right() > hbar.right(),
        "Right reaches past the page's edge"
    );

    let arrow = s.hit(hbar.right() - 3, hbar.y + 2, &text, px);
    assert_eq!(arrow, Some(Hit::PageHBar(BarHit::Forward)));
    assert_eq!(s.click(arrow), Event::Redraw);
    s.release();
    assert_eq!(s.page_offset(), (-LINE_H, 0));

    s.open_mut().focus_id("left");
    s.key(form::Key::Tab { back: false }, None);
    assert_eq!(
        s.open().focus().map(|i| s.open().items[i].id),
        Some("right")
    );
    let right = s.open().rect("right").unwrap();
    assert!(
        right.right() <= hbar.right(),
        "Tab onto Right brings it into view: {right:?}"
    );
    assert_eq!(s.open().rect(OK).unwrap(), l.ok, "the buttons stay put");

    assert!(s.fit((398, 424)));
    assert_eq!(s.page_offset(), (0, 0), "a whole sheet does not scroll");
    assert_eq!(s.layout(&text, px).hbar, None);
}

/// A page cut short at 200% scrolls under a bar down its right edge. A control flush with
/// that edge would lie under the bar, so the page scrolls sideways by the bar's width as
/// well; one that keeps the page's margin leaves the page whole across.
#[test]
fn the_page_scrollbar_covers_no_control() {
    let theme = Theme::default().with_form_text(200);
    let (u, px) = (theme.text, theme.fonts.size_px);
    let text = TextRenderer::load(&FontConfig::default());
    let p = theme.paint();
    let edge = 398 - MARGIN - PAGE_EDGE;
    let at = |right: i32| {
        let page = Form::new(vec![
            Item::check("top", 20, 50, "Top"),
            Item::check("low", 20, 373, "Low"),
            Item::button(
                "flush",
                Rect::new(right - 75, 100, 75, 23),
                "Flush",
                true,
                false,
            ),
        ]);
        let mut s = PropSheet::new(&["General"], vec![page], (398, 424));
        s.drawn_with(&p);
        s.fit((398, 300));
        s
    };
    let mut s = at(edge);
    let l = s.layout(&text, px);
    let bar = l.bar.expect("a page cut short has a scrollbar");
    let hbar = l
        .hbar
        .expect("a control under the bar brings in the sideways bar");
    assert_eq!(hbar.right(), bar.x, "the bars meet at the corner");
    assert_eq!(bar.bottom(), hbar.y);
    assert_eq!(s.hrange(), u.design(s.open().controls().scroll));
    assert_eq!(
        s.wheel(Some(Hit::PageHBar(BarHit::Track)), 100),
        Event::Redraw
    );
    let flush = s.open().rect("flush").unwrap();
    assert!(
        flush.right() <= bar.x,
        "{flush:?} clears the bar at {bar:?}"
    );
    let mut c = Canvas::new(u.len(398) as u32, u.len(300) as u32);
    draw(&mut c, &text, &p, &s, false, px);

    // 16 px at 100%: the 32 px bar a 200% page wears.
    let s = at(edge - 16);
    assert!(s.layout(&text, px).bar.is_some());
    assert_eq!(s.layout(&text, px).hbar, None, "the margin holds the bar");

    let mut s = at(edge + 30);
    s.fit((398, 424));
    let l = s.layout(&text, px);
    assert_eq!(
        (l.bar, l.hbar),
        (None, None),
        "a whole sheet does not scroll to a rect built past its page"
    );
}

/// A reader's Click reaches every row of an open combo whose list hangs below the page, on a
/// page whole in its client and on one scrolled to its end, at the row's own middle, and a press
/// there picks the row: the list draws over the page and takes the press.
#[test]
fn a_readers_click_picks_each_row_of_a_combo_list_hanging_below_the_page() {
    use crate::access::{Act, Published, ROOT, Request, Route, Tree, accesskit::Role};
    use crate::controls::combo::Combo;
    let theme = Theme::default();
    let text = TextRenderer::load(&theme.fonts);
    let px = theme.fonts.size_px;
    const ROWS: &[&str] = &["A", "B", "C", "D", "E", "F", "G", "H"];
    for client in [(398, 424), (398, 300)] {
        for row in 0..ROWS.len() {
            let combo = Combo::new(ROWS, 0, ROWS.len());
            let page = Form::new(vec![Item::new(
                "combo",
                Rect::new(20, 300, 150, 21),
                Control::Combo(combo),
            )]);
            let mut s = PropSheet::new(&["P"], vec![page], (398, 424));
            s.fit(client);
            assert_eq!(s.scrolls(), client.1 < 424);
            s.wheel_page(100);
            if let Some(Control::Combo(c)) = s.open_mut().item_mut("combo").map(|i| &mut i.control)
            {
                c.open = Some(0);
            }
            let mut tree = Tree::window("T", client);
            s.access(&mut tree, ROOT, &text, px);
            let options: Vec<_> = (tree.nodes())
                .filter(|(_, n)| n.role() == Role::ListBoxOption)
                .map(|(id, n)| (id, n.bounds().expect("placed while open")))
                .collect();
            let (id, b) = options[row];
            let mut p = Published::default();
            p.update(tree);
            let name = format!("{client:?} row {row}");
            let pitch = options[0].1.height();
            assert_eq!(b.height(), pitch, "{name}: not cut to the page");
            let Route::Act(t) = p.route(Request {
                node: id,
                act: Act::Click,
            }) else {
                panic!("{name}: acted on");
            };
            assert_eq!(t.at, ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0), "{name}");
            let hit = s.hit(t.at.0 as i32, t.at.1 as i32, &text, px);
            s.click(hit);
            s.release();
            let Some(Control::Combo(c)) = s.open().item("combo").map(|i| &i.control) else {
                unreachable!()
            };
            assert_eq!((c.selected, c.open), (row, None), "{name}: picked");
        }
    }
}

/// A press on an open combo's list where it hangs over the page list picks the row: the list
/// lies over the sheet's own parts, as it draws.
#[test]
fn an_open_combo_list_over_the_page_list_takes_the_press() {
    use crate::controls::combo::Combo;
    let theme = Theme::default();
    let text = TextRenderer::load(&theme.fonts);
    let px = theme.fonts.size_px;
    const ROWS: &[&str] = &["A", "B", "C", "D", "E", "F", "G", "H"];
    let combo = Item::new(
        "combo",
        Rect::new(20, 50, 150, 21),
        Control::Combo(Combo::new(ROWS, 0, ROWS.len())),
    );
    let pages = vec![
        Form::new(vec![combo]),
        check_page("Effects"),
        check_page("Mouse"),
    ];
    let mut s = PropSheet::with_list(LIST_GROUPS, pages, (398, 424));
    s.fit((300, 424));
    s.wheel(Some(Hit::PageHBar(BarHit::Track)), 100);
    if let Some(Control::Combo(c)) = s.open_mut().item_mut("combo").map(|i| &mut i.control) {
        c.open = Some(0);
    }
    let list = s.layout(&text, px).list.unwrap().frame;
    let mut tree = crate::access::Tree::window("T", (300, 424));
    s.access(&mut tree, crate::access::ROOT, &text, px);
    let row = (tree.nodes())
        .filter(|(_, n)| n.role() == crate::access::accesskit::Role::ListBoxOption)
        .map(|(_, n)| n.bounds().unwrap())
        .nth(3)
        .unwrap();
    let (x, y) = (row.x0 as i32 + 1, ((row.y0 + row.y1) / 2.0) as i32);
    assert!(
        list.contains(x, y),
        "({x}, {y}) lies on the page list {list:?} too"
    );
    let hit = s.hit(x, y, &text, px);
    assert!(matches!(hit, Some(Hit::Control(_))), "{hit:?}");
    s.click(hit);
    s.release();
    let Some(Control::Combo(c)) = s.open().item("combo").map(|i| &i.control) else {
        unreachable!()
    };
    assert_eq!((c.selected, c.open), (3, None), "the row is picked");
}

/// An open combo's field scrolled out of the page's view leaves the press to the sheet's part
/// there: the page list, the search field or a page bar.
#[test]
fn an_open_combos_field_outside_the_view_leaves_the_sheets_parts_their_presses() {
    use crate::controls::combo::{self, Combo};
    let theme = Theme::default();
    let text = TextRenderer::load(&theme.fonts);
    let px = theme.fonts.size_px;
    const ROWS: &[&str] = &["A", "B", "C", "D"];
    let cases = [
        (true, 300, (300, 424), (0, 100)),
        (true, 12, (300, 424), (0, 100)),
        (false, 300, (300, 300), (100, 100)),
    ];
    for (listed, y, client, (down, across)) in cases {
        let combo = Item::new(
            "combo",
            Rect::new(20, y, 150, 21),
            Control::Combo(Combo::new(ROWS, 0, ROWS.len())),
        );
        let mut s = match listed {
            true => PropSheet::with_list(
                LIST_GROUPS,
                vec![
                    Form::new(vec![combo]),
                    check_page("Effects"),
                    check_page("Mouse"),
                ],
                (398, 424),
            ),
            false => PropSheet::new(
                &["P", "Q"],
                vec![Form::new(vec![combo]), check_page("Q")],
                (398, 424),
            ),
        };
        s.fit(client);
        s.wheel_page(down);
        s.wheel(Some(Hit::PageHBar(BarHit::Track)), across);
        let points = (0..client.1).flat_map(|y| (0..client.0).map(move |x| (x, y)));
        let before: Vec<_> = points
            .clone()
            .map(|(x, y)| s.hit(x, y, &text, px))
            .collect();
        if let Some(Control::Combo(c)) = s.open_mut().item_mut("combo").map(|i| &mut i.control) {
            c.open = Some(0);
        }
        let taken = (points.zip(before))
            .filter(|(_, b)| !matches!(b, Some(Hit::Control(_)) | None))
            .filter(|&((x, y), _)| {
                matches!(
                    s.hit(x, y, &text, px),
                    Some(Hit::Control(form::Hit {
                        part: form::Part::Combo(combo::Hit::Field | combo::Hit::Drop),
                        ..
                    }))
                )
            })
            .count();
        assert_eq!(
            taken, 0,
            "listed {listed}, combo at {y}, {client:?}: px taken"
        );
    }
}
