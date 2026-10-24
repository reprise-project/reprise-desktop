use super::*;

/// A memo takes typing, Space, a pasted line break and Ctrl+Enter as text; Enter is still
/// the dialog's default and Tab still leaves it. It draws in a 2 px sunken frame with a
/// 16 px vertical scrollbar; a greyed memo takes no key.
#[test]
fn a_memo_edits_lines_and_keeps_its_scrollbar() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let mut f = Form::new(vec![
        Item::memo("m", Rect::new(0, 0, 263, 80), "", true),
        Item::button("ok", Rect::new(0, 90, 75, 23), "OK", true, true),
    ]);
    let memo = |f: &Form| match &f.items[0].control {
        Control::Memo(m) => m.edit.text(),
        _ => unreachable!(),
    };
    assert_eq!(f.key(Key::Char('a')), Event::Changed("m"));
    f.key(Key::Space);
    assert_eq!(f.paste("b\ncd".into()), Event::Changed("m"));
    assert_eq!(memo(&f), "a b\ncd");
    f.key(Key::Up);
    f.key(Key::Char('X'));
    assert_eq!(memo(&f), "a Xb\ncd", "the caret kept its column");
    assert_eq!(f.key(Key::Enter), Event::Default);
    assert_eq!(f.key(Key::NewLine), Event::Changed("m"));
    assert_eq!(memo(&f), "a X\nb\ncd", "Ctrl+Enter breaks the line");
    let edge = |end| Key::DocEdge { end, select: false };
    assert_eq!(f.key(edge(false)), Event::Redraw);
    f.key(Key::Char('<'));
    f.key(edge(true));
    f.key(Key::Char('>'));
    assert_eq!(memo(&f), "<a X\nb\ncd>", "Ctrl+Home and Ctrl+End");
    assert_eq!(f.key(Key::Tab { back: false }), Event::Redraw);
    assert_eq!(f.focus(), Some(1));
    assert_eq!(
        f.key(Key::NewLine),
        Event::Pressed("ok"),
        "off the memo it is Enter"
    );

    let mut c = Canvas::new(263, 80);
    c.fill_rect(c.bounds(), cs.button_face);
    let text = TextRenderer::load(&FontConfig::default());
    draw(&mut c, &text, &classic_paint(cs, 1), &f, false, 11);
    assert_eq!(c.get(0, 0), cs.button_shadow.argb(), "the sunken frame");
    assert_eq!(c.get(10, 40), cs.window.argb(), "the white box");
    assert_ne!(c.get(260 - 8, 40), cs.window.argb(), "the scrollbar");
    assert_eq!(c.get(263 - 2 - 16 - 1, 40), cs.window.argb());

    let mut greyed = Form::new(vec![Item::memo("m", Rect::new(0, 0, 263, 80), "", false)]);
    assert_eq!(greyed.focus(), None);
    assert_eq!(greyed.key(Key::Char('a')), Event::None);
}

/// At scale 2 a memo's selection ends where its text does, at the
/// width the device font measures; a line scrolled sideways to its
/// caret shows its end, and a click on a character lands on it.
#[test]
fn a_memo_at_scale_two_lines_its_selection_and_clicks_up_with_its_text() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let t = tr();
    let mut f = Form::new(vec![Item::memo("m", Rect::new(0, 0, 120, 40), "abc", true)]);
    f.key(Key::Edit(EditKey::SelectAll));
    let shot = |f: &Form| {
        let mut c = Canvas::new(240, 80);
        draw(&mut c, &t, &classic_paint(cs, 2), f, false, 2 * PX);
        c
    };
    let c = shot(&f);
    // The text starts a device px inside the 2 px (4 device px) edge.
    let x0 = 4 + 2;
    let row = 4 + 2 + 6;
    let lit: Vec<i32> = (0..240)
        .filter(|&x| c.get(x, row) == cs.hilight.argb())
        .collect();
    let w = t.measure("abc", false, 2 * PX);
    assert_eq!(lit.first(), Some(&x0));
    assert_eq!(lit.last(), Some(&(x0 + w - 1)), "the run is {w} px wide");

    // A line three boxes long, the caret at its end: the view scrolls
    // it left, and a click at the box's left lands on a character
    // there, not near the start.
    let long = "m".repeat(80);
    let Control::Memo(m) = &mut f.items[0].control else {
        unreachable!()
    };
    m.edit = multiedit::MultiEdit::new(&long);
    m.edit.place(
        multiedit::Pos {
            line: 0,
            col: long.len(),
        },
        false,
    );
    shot(&f);
    let left = match &f.items[0].control {
        Control::Memo(m) => m.view.get().left,
        _ => unreachable!(),
    };
    let cell = t.measure("m", false, PX);
    let hit = f.hit(3 + cell / 2, 10, &t, PX).expect("the memo");
    let Part::Memo(at) = hit.part else {
        panic!("a text position, not {:?}", hit.part)
    };
    let want = (left / 2 / cell) as usize;
    assert!(
        at.col.abs_diff(want) <= 1,
        "the click lands at column {} for a view {left} device px in, near {want}",
        at.col
    );
}

/// A press on a memo's scrollbar scrolls it and leaves the caret
/// where it was; the next draw keeps the view there.
#[test]
fn a_press_on_a_memos_scrollbar_scrolls_without_moving_the_caret() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let t = tr();
    let text: Vec<String> = (0..12).map(|k| format!("line {k}")).collect();
    let r = Rect::new(0, 0, 200, 80);
    let mut f = Form::new(vec![Item::memo("m", r, &text.join("\n"), true)]);
    let shot = |f: &Form| {
        let mut c = Canvas::new(200, 80);
        draw(&mut c, &t, &classic_paint(cs, 1), f, false, PX);
    };
    shot(&f);
    let memo = |f: &Form| match &f.items[0].control {
        Control::Memo(m) => (m.edit.caret(), m.view.get().top),
        _ => unreachable!(),
    };
    let start = multiedit::Pos::default();
    let lay = multiedit::layout(
        &t,
        &Memo::metrics(PX, &ControlMetrics::CLASSIC, TextUnits::ONE),
        &{
            let Control::Memo(m) = &f.items[0].control else {
                unreachable!()
            };
            m.edit.clone()
        },
        r,
        1,
    );
    let vs = lay.vscroll.expect("the memo keeps its vertical bar");
    for (y, top) in [(vs.bottom() - 4, 1), (vs.bottom() - 20, 1 + lay.rows)] {
        let hit = f.hit(vs.x + 4, y, &t, PX);
        assert_eq!(f.click(hit), Event::Redraw);
        f.release();
        shot(&f);
        assert_eq!(memo(&f), (start, top), "a press at y {y}");
    }
}

/// A memo's thumb, grabbed, follows the pointer down its bar, on it or
/// off to the side, until the release; the caret stays put. At scale
/// 2 the horizontal thumb dragged to the track's end shows the long
/// line's tail, in device px.
#[test]
fn a_memos_thumb_drags_its_view() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let t = tr();
    let lines: Vec<String> = (0..12).map(|k| format!("line {k}")).collect();
    let r = Rect::new(0, 0, 200, 80);
    let mut f = Form::new(vec![Item::memo("m", r, &lines.join("\n"), true)]);
    let shot = |f: &Form, s: i32| {
        let mut c = Canvas::new(200 * s as u32, 80 * s as u32);
        draw(&mut c, &t, &classic_paint(cs, s), f, false, s as u32 * PX);
    };
    let memo = |f: &Form| match &f.items[0].control {
        Control::Memo(m) => (m.edit.caret(), m.view.get()),
        _ => unreachable!(),
    };
    let parts = |f: &Form, bar| {
        let Control::Memo(m) = &f.items[0].control else {
            unreachable!()
        };
        let mm = Memo::metrics(PX, &ControlMetrics::CLASSIC, TextUnits::ONE);
        let lay = multiedit::layout(&t, &mm, &m.edit, r, 1);
        let v = f.memo_view_1x(m);
        (
            lay.rows,
            multiedit::bar_parts(&t, &mm, &m.edit, &lay, v, bar).unwrap(),
        )
    };
    shot(&f, 1);
    let (rows, p) = parts(&f, multiedit::ScrollBar::Vertical);
    let thumb = p.thumb.expect("12 lines overflow the box");
    let (gx, gy) = (thumb.x + 4, thumb.y + 3);
    assert_eq!(
        f.click(f.hit(gx, gy, &t, PX)),
        Event::None,
        "the press moves nothing"
    );
    assert!(
        matches!(f.drag, Some(Drag::Memo { .. })),
        "the press grabs the thumb"
    );
    let dy = p.track.height - thumb.height;
    assert_eq!(f.motion(gx, gy + dy, None), Event::Redraw);
    assert_eq!(
        memo(&f).1.top,
        12 - rows,
        "the track's end shows the last page"
    );
    assert_eq!(f.motion(gx - 300, gy, None), Event::Redraw);
    assert_eq!(memo(&f).1.top, 0, "off the bar its y still steers");
    f.release();
    assert_eq!(f.motion(gx, gy + dy, None), Event::None, "let go, it stays");
    assert_eq!(
        memo(&f),
        (multiedit::Pos::default(), multiedit::View::default())
    );

    let Control::Memo(m) = &mut f.items[0].control else {
        unreachable!()
    };
    m.edit = multiedit::MultiEdit::new(&"m".repeat(80));
    shot(&f, 2);
    let (_, p) = parts(&f, multiedit::ScrollBar::Horizontal);
    let thumb = p.thumb.expect("the line overflows the box");
    f.click(f.hit(thumb.x + 2, thumb.y + 4, &t, PX));
    assert_eq!(
        f.motion(p.track.right() + 50, thumb.y + 4, None),
        Event::Redraw
    );
    let Control::Memo(m) = &f.items[0].control else {
        unreachable!()
    };
    let mm = Memo::metrics(PX, &ControlMetrics::CLASSIC, TextUnits::ONE);
    let lay = multiedit::layout(&t, &mm, &m.edit, r, 1);
    let total = multiedit::widest_px(&t, &mm, &m.edit) + 2;
    assert_eq!(memo(&f).1.left, 2 * (total - lay.inner.width));
}

/// A focused memo is the form's focused edit, so the host's clipboard
/// copies its selection and its caret blinks; a password field hands
/// nothing out.
#[test]
fn a_focused_memo_is_the_focused_edit() {
    let mut f = Form::new(vec![
        Item::memo("m", Rect::new(0, 0, 263, 80), "one\ntwo", true),
        Item::edit("pw", Rect::new(0, 90, 100, 20), "secret", true),
    ]);
    f.key(Key::Edit(EditKey::SelectAll));
    let copied = f.focused_edit().and_then(|e| e.copy_text());
    assert_eq!(copied.as_deref(), Some("one\ntwo"));
    if let Some(Control::Edit { edit, .. }) = f.item_mut("pw").map(|i| &mut i.control) {
        *edit = TextEdit::secret();
        edit.paste("abc", &[]);
        edit.key(EditKey::SelectAll);
    }
    f.focus_id("pw");
    assert!(f.focused_edit().is_some());
    assert_eq!(f.focused_edit().and_then(|e| e.copy_text()), None);
}

/// The wheel over a memo scrolls its view a line a row and leaves the
/// caret; the next draw keeps the view there. A greyed memo stays.
#[test]
fn the_wheel_scrolls_a_memos_view_and_not_its_caret() {
    let theme = Theme::default();
    let t = tr();
    let text: Vec<String> = (0..12).map(|k| format!("line {k}")).collect();
    let r = Rect::new(0, 0, 200, 80);
    let mut f = Form::new(vec![Item::memo("m", r, &text.join("\n"), true)]);
    let shot = |f: &Form| {
        let mut c = Canvas::new(200, 80);
        draw(&mut c, &t, &classic_paint(&theme.colors, 1), f, false, PX);
    };
    let memo = |f: &Form| match &f.items[0].control {
        Control::Memo(m) => (m.edit.caret(), m.view.get().top, m.rows.get()),
        _ => unreachable!(),
    };
    shot(&f);
    let rows = memo(&f).2;
    assert!(
        rows > 1 && rows < 12,
        "the box shows part of the text: {rows}"
    );
    let on = at(&f, 20, 20);
    assert!(matches!(on.map(|h| h.part), Some(Part::Memo(_))));
    let start = multiedit::Pos::default();
    assert_eq!(f.wheel(on, WHEEL_ROWS), Event::Redraw);
    assert_eq!(memo(&f), (start, 3, rows), "three lines a notch");
    shot(&f);
    assert_eq!(
        memo(&f).1,
        3,
        "the draw leaves the view where the wheel put it"
    );
    f.wheel(on, 20 * WHEEL_ROWS);
    assert_eq!(memo(&f).1, 12 - rows, "the last line stops at the bottom");
    assert_eq!(f.wheel(on, WHEEL_ROWS), Event::None, "at the bottom");
    f.wheel(on, -20 * WHEEL_ROWS);
    assert_eq!(memo(&f).1, 0);
    f.set_enabled("m", false);
    assert_eq!(f.wheel(on, WHEEL_ROWS), Event::None, "a greyed memo");
    assert_eq!(memo(&f).1, 0);
}

/// Page Down in a focused memo takes the caret down the rows the box
/// shows, and the next draw scrolls the view to
/// it; Page Up takes it back.
#[test]
fn page_keys_move_a_memos_caret_a_page() {
    let theme = Theme::default();
    let t = tr();
    let text: Vec<String> = (0..20).map(|k| format!("line {k}")).collect();
    let mut f = Form::new(vec![Item::memo("m", Rect::new(0, 0, 200, 80), "", true)]);
    let shot = |f: &Form| {
        let mut c = Canvas::new(200, 80);
        draw(&mut c, &t, &classic_paint(&theme.colors, 1), f, false, PX);
    };
    let memo = |f: &Form| match &f.items[0].control {
        Control::Memo(m) => (m.edit.caret(), m.view.get().top, m.rows.get()),
        _ => unreachable!(),
    };
    assert_eq!(f.paste(text.join("\n")), Event::Changed("m"));
    shot(&f);
    let rows = memo(&f).2;
    assert!(
        rows > 1 && rows < 20,
        "the box shows part of the text: {rows}"
    );
    f.key(Key::DocEdge {
        end: false,
        select: false,
    });
    shot(&f);
    assert_eq!(memo(&f).1, 0, "Ctrl+Home shows the first line");
    assert_eq!(f.key(Key::PageDown), Event::Redraw);
    let (caret, ..) = memo(&f);
    assert_eq!((caret.line, caret.col), (rows, 0), "a page down");
    shot(&f);
    assert_eq!(memo(&f).1, 1, "the view follows the caret to the last row");
    for _ in 0..20 / rows {
        f.key(Key::PageDown);
    }
    assert_eq!(memo(&f).0.line, 19, "held to the last line");
    assert_eq!(f.key(Key::PageDown), Event::None, "at the last line");
    f.key(Key::DocEdge {
        end: false,
        select: false,
    });
    f.key(Key::PageDown);
    assert_eq!(f.key(Key::PageUp), Event::Redraw);
    assert_eq!(memo(&f).0.line, 0, "Page Up takes it back");
    shot(&f);
    assert_eq!(memo(&f).1, 0);
}
