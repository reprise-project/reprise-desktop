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

    let s = at(edge - 12);
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
