use super::*;
use crate::look::classic_paint;
use crate::theme::{FontConfig, Theme};

fn e(text: &str) -> MultiEdit {
    MultiEdit::new(text)
}
fn k(edit: &mut MultiEdit, key: MultiKey) -> bool {
    let text = TextRenderer::load(&FontConfig::default());
    edit.key(key, &text, &Metrics::default())
}

#[test]
fn changes_are_counted_and_the_widest_line_follows_them() {
    let text = TextRenderer::load(&FontConfig::default());
    let m = Metrics::default();
    let mut t = e("ab\nc");
    assert!(
        !k(
            &mut t,
            MultiKey::Edit(EditKey::Left {
                select: false,
                word: false
            })
        ),
        "at the start already"
    );
    let w0 = widest_px(&text, &m, &t);
    assert_eq!(
        w0,
        widest_px(&text, &m, &t),
        "the same document measures the same"
    );
    assert!(k(
        &mut t,
        MultiKey::Edit(EditKey::Insert("wider line".into()))
    ));
    let w1 = widest_px(&text, &m, &t);
    assert!(w1 > w0);
    assert!(k(&mut t, MultiKey::Edit(EditKey::Undo)));
    assert_eq!(widest_px(&text, &m, &t), w0);
    assert_eq!(
        t.replace_all(
            "c",
            "cccccccccccccc",
            &FindOpts {
                match_case: true,
                down: true
            }
        ),
        1
    );
    assert!(widest_px(&text, &m, &t) > w0);
}
fn p(line: usize, col: usize) -> Pos {
    Pos { line, col }
}

#[test]
fn dropped_text_lands_at_the_drop_point_selected() {
    let mut t = e("one two\nthree");
    t.place(p(1, 0), false);
    t.place(p(1, 5), true);
    assert!(t.drop_text(p(0, 4), "big\r\nred "));
    assert_eq!(t.lines, ["one big", "red two", "three"]);
    assert_eq!(t.selected_text(), "big\nred ", "the drop is selected");
    assert_eq!(t.caret(), p(1, 4), "the caret sits at its end");
}

#[test]
fn one_undo_takes_a_drop_back_and_restores_the_selection() {
    let mut t = e("one two\nthree");
    t.place(p(1, 0), false);
    t.place(p(1, 5), true);
    t.drop_text(p(0, 4), "big ");
    assert!(k(&mut t, MultiKey::Edit(EditKey::Undo)));
    assert_eq!(t.lines, ["one two", "three"]);
    assert_eq!(t.selected_text(), "three");
}

#[test]
fn a_drop_of_nothing_printable_changes_nothing() {
    let mut t = e("ab");
    assert!(!t.drop_text(p(0, 1), "\r\u{7}"));
    assert_eq!(t.lines, ["ab"]);
}

#[test]
fn delete_surrounding_crosses_lines_and_keeps_the_selection() {
    let mut t = e("ab\ncd\nef");
    t.place(p(1, 1), false);
    t.place(p(2, 1), true);
    assert!(k(
        &mut t,
        MultiKey::Edit(EditKey::DeleteSurrounding {
            before: 2,
            after: 1
        })
    ));
    assert_eq!(t.lines, ["abd", "e"]);
    assert_eq!(t.selected_text(), "d\ne");
    assert_eq!(t.caret(), p(1, 1), "the caret stays at the selection's end");
    assert!(!k(
        &mut t,
        MultiKey::Edit(EditKey::DeleteSurrounding {
            before: 0,
            after: 0
        })
    ));
}

#[test]
fn a_preedit_draws_in_the_caret_line_and_the_spot_is_that_line() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let m = Metrics {
        bars_always: false,
        ..Default::default()
    };
    let mut t = e("top\nab");
    t.place(p(1, 1), false);
    let lay = layout(&text, &m, &t, Rect::new(0, 0, 200, 60), 1);
    let mut c = Canvas::new(200, 60);
    c.set_preedit(crate::textedit::Preedit::new("xyz".into(), -1, -1));
    draw(
        &mut c,
        &text,
        &classic_paint(&theme.colors, 1),
        &m,
        &t,
        View::default(),
        &lay,
        Caret::Hidden,
        None,
    );
    let x0 = lay.inner.x + 1 + text.measure("a", false, m.font_px);
    let x1 = lay.inner.x + 1 + text.measure("axyz", false, m.font_px);
    let row = Rect::new(lay.inner.x + 1, lay.inner.y + 1 + m.line_h, 10, m.line_h);
    let ul = text.baseline(row, false, m.font_px).unwrap() + 1;
    let ink = theme.colors.button_text.argb();
    assert!((x0..x1).all(|x| c.get(x, ul) == ink));
    let spot = c.take_spot().expect("focused");
    assert!(spot.multiline);
    assert_eq!(spot.caret.x, x1, "a hidden preedit cursor sits at its end");
    let sur = spot.surrounding.unwrap();
    assert_eq!((sur.text.as_str(), sur.cursor), ("ab", 1));
}

#[test]
fn a_preedit_range_draws_as_a_caret_at_its_end_over_no_selection() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let m = Metrics {
        bars_always: false,
        ..Default::default()
    };
    let mut t = e("top\nab");
    t.place(p(0, 1), false);
    t.place(p(1, 1), true);
    let lay = layout(&text, &m, &t, Rect::new(0, 0, 200, 60), 1);
    let draw_with = |pre| {
        let mut c = Canvas::new(200, 60);
        c.set_preedit(pre);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &m,
            &t,
            View::default(),
            &lay,
            Caret::Shown,
            None,
        );
        c
    };
    let mut range = draw_with(crate::textedit::Preedit::new("xyz".into(), 1, 3));
    let mut at_end = draw_with(crate::textedit::Preedit::new("xyz".into(), 3, 3));
    let hl = theme.colors.hilight.argb();
    assert!(range.pixels.iter().all(|&p| p != hl), "nothing on hilight");
    assert_eq!(range.pixels, at_end.pixels);
    let x = lay.inner.x + 1 + text.measure("axyz", false, m.font_px);
    let spot = range.take_spot().expect("focused");
    assert_eq!(spot.caret.x, x, "the caret ends the range");
    assert_eq!(Some(spot), at_end.take_spot());
}

#[test]
fn the_drop_point_draws_a_bar_at_its_position() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let m = Metrics::default();
    let t = e("top\nab");
    let lay = layout(&text, &m, &t, Rect::new(0, 0, 200, 60), 1);
    let paint = classic_paint(&theme.colors, 1);
    let draw_with = |view, drop| {
        let mut c = Canvas::new(200, 60);
        draw(
            &mut c,
            &text,
            &paint,
            &m,
            &t,
            view,
            &lay,
            Caret::Hidden,
            drop,
        );
        c
    };
    let before = draw_with(View::default(), None);
    let c = draw_with(View::default(), Some(p(1, 1)));
    let x = lay.inner.x + 1 + text.measure("a", false, m.font_px);
    let y = lay.inner.y + 1 + m.line_h;
    let paper = theme.colors.window.argb();
    let (bar, _) = crate::paint::inverted_bar(&before, &c, paper);
    assert_eq!(
        (bar.y, bar.width, bar.height),
        (y, 1, m.line_h),
        "1 px wide, one line tall"
    );
    assert!([x - 1, x].contains(&bar.x), "at the point: {bar:?}");

    let scrolled = View {
        top: 1,
        ..View::default()
    };
    let before = draw_with(scrolled, None);
    let c = draw_with(scrolled, Some(p(0, 1)));
    assert!(c == before, "line 0 is above the box");
}

#[test]
fn an_edit_remeasures_only_the_lines_it_touched() {
    let text = TextRenderer::load(&FontConfig::default());
    let m = Metrics::default();
    let unmeasured = |t: &MultiEdit| t.widths.borrow().px.iter().filter(|w| w.is_none()).count();
    let mut t = e("one\ntwo\nthree\nfour");
    widest_px(&text, &m, &t);
    assert_eq!(unmeasured(&t), 0);
    t.place(p(1, 3), false);
    k(&mut t, MultiKey::Edit(EditKey::Insert("x".into())));
    assert_eq!(unmeasured(&t), 1, "typing touches its own line");
    k(&mut t, MultiKey::Enter);
    assert_eq!(unmeasured(&t), 2, "Enter splits it in two");
    assert_eq!(
        widest_px(&text, &m, &t),
        widest_px(&text, &m, &e(&t.text()))
    );
    t.place(p(0, 1), false);
    t.place(p(3, 2), true);
    k(&mut t, MultiKey::Edit(EditKey::Delete { word: false }));
    assert_eq!(t.lines, ["oree", "four"]);
    assert_eq!(unmeasured(&t), 1, "a deletion across lines leaves one");
    assert_eq!(
        widest_px(&text, &m, &t),
        widest_px(&text, &m, &e(&t.text()))
    );
    t.paste("a much longer pasted line\nb");
    assert_eq!(
        widest_px(&text, &m, &t),
        widest_px(&text, &m, &e(&t.text()))
    );
}

#[test]
fn splits_joins_and_edits_lines() {
    let mut t = e("ab\ncd");
    assert_eq!(t.lines, ["ab", "cd"]);
    assert_eq!(t.text(), "ab\ncd");
    k(
        &mut t,
        MultiKey::Edit(EditKey::Right {
            select: false,
            word: false,
        }),
    );
    k(&mut t, MultiKey::Enter);
    assert_eq!(t.lines, ["a", "b", "cd"]);
    assert_eq!(t.caret, p(1, 0));
    k(&mut t, MultiKey::Edit(EditKey::Backspace { word: false }));
    assert_eq!(t.lines, ["ab", "cd"]);
    k(&mut t, MultiKey::Edit(EditKey::End { select: false }));
    k(&mut t, MultiKey::Edit(EditKey::Delete { word: false }));
    assert_eq!(t.lines, ["abcd"]);
    t.paste("x\ny");
    assert_eq!(t.lines, ["abx", "ycd"]);
    assert_eq!(t.caret, p(1, 1));
}

#[test]
fn selection_spans_lines_and_typing_replaces_it() {
    let mut t = e("one\ntwo\nthree");
    t.place(p(0, 1), false);
    t.place(p(2, 2), true);
    assert_eq!(t.selected_text(), "ne\ntwo\nth");
    k(&mut t, MultiKey::Edit(EditKey::Insert("X".into())));
    assert_eq!(t.lines, ["oXree"]);
    k(&mut t, MultiKey::Edit(EditKey::Undo));
    assert_eq!(t.lines, ["one", "two", "three"]);
    assert_eq!(t.selected_text(), "ne\ntwo\nth");
    t.select_all();
    assert_eq!(t.selected_text(), "one\ntwo\nthree");
    k(&mut t, MultiKey::Edit(EditKey::Cut));
    assert_eq!(t.lines, [""]);
}

#[test]
fn vertical_moves_keep_the_goal_column() {
    let text = TextRenderer::load(&FontConfig::default());
    let m = Metrics::default();
    let mut t = e("longer line\nab\nlonger line");
    t.place(p(0, 8), false);
    let lay = layout(&text, &m, &t, Rect::new(0, 0, 200, 60), 1);
    let goal = x_of(&text, &m, &t, &lay, p(0, 8));
    t.key(MultiKey::Down { select: false }, &text, &m);
    assert_eq!(t.caret, p(1, 2), "a short line clamps");
    assert_eq!(t.goal_x, Some(goal));
    t.key(MultiKey::Down { select: false }, &text, &m);
    assert_eq!(t.caret, p(2, 8), "the goal column comes back");
    k(&mut t, MultiKey::DocHome { select: true });
    assert_eq!(t.selection(), (p(0, 0), p(2, 8)));
    assert_eq!(t.goal_x, None);
    k(&mut t, MultiKey::DocEnd { select: false });
    assert_eq!(t.caret, p(2, 11));
}

#[test]
fn words_wrap_across_lines_and_double_click_selects_them() {
    let mut t = e("ab cd\nef");
    k(&mut t, MultiKey::DocEnd { select: false });
    k(
        &mut t,
        MultiKey::Edit(EditKey::Left {
            select: false,
            word: true,
        }),
    );
    assert_eq!(t.caret, p(1, 0));
    k(
        &mut t,
        MultiKey::Edit(EditKey::Left {
            select: false,
            word: true,
        }),
    );
    assert_eq!(
        t.caret,
        p(0, 5),
        "at a line start, up to the previous line's end"
    );
    k(
        &mut t,
        MultiKey::Edit(EditKey::Left {
            select: false,
            word: true,
        }),
    );
    assert_eq!(t.caret, p(0, 3));
    t.select_word_at(p(0, 1));
    assert_eq!(t.selected_text(), "ab ");
    k(&mut t, MultiKey::Edit(EditKey::Backspace { word: true }));
    assert_eq!(t.lines, ["cd", "ef"]);
}

#[test]
fn tabs_expand_for_measuring_and_map_back() {
    assert_eq!(expand("a\tb", 8), "a       b");
    assert_eq!(expanded_col("a\tb", 2, 8), 8);
    assert_eq!(
        expanded_col("é\tb", 3, 8),
        expand("é\t", 8).len(),
        "a 2-byte char is one column before the tab"
    );
    assert_eq!(raw_col("a\tb", 8, 8), 2);
    assert_eq!(raw_col("a\tb", 4, 8), 1, "inside the tab lands on it");
    assert_eq!(raw_col("ab", 9, 8), 2);
}

#[test]
fn zero_metrics_do_not_divide_by_zero() {
    let text = TextRenderer::load(&FontConfig::default());
    let m = Metrics {
        line_h: 0,
        tab_cols: 0,
        ..Metrics::default()
    };
    let t = e("a\tb\nc");
    assert_eq!(expand("a\tb", 0), "a b");
    assert_eq!(raw_col("a\tb", 2, 0), 2);
    let lay = layout(&text, &m, &t, Rect::new(0, 0, 200, 60), 1);
    assert_eq!(
        pos_at(&text, &m, &t, &lay, View::default(), 3, 4, 1).line,
        1
    );
}

#[test]
fn a_17_px_bar_takes_17_px_off_the_text_box_each_way() {
    let text = TextRenderer::load(&FontConfig::default());
    let m = Metrics {
        bar: 17,
        ..Metrics::default()
    };
    let l = layout(&text, &m, &e("a"), Rect::new(0, 0, 200, 60), 1);
    assert_eq!(l.inner, Rect::new(2, 2, 196 - 17, 56 - 17));
    assert_eq!(l.vscroll, Some(Rect::new(2 + 196 - 17, 2, 17, 56 - 17)));
    assert_eq!(l.hscroll, Some(Rect::new(2, 2 + 56 - 17, 196 - 17, 17)));
    assert_eq!(
        l.corner(),
        Some(Rect::new(2 + 196 - 17, 2 + 56 - 17, 17, 17))
    );
    let l = layout(&text, &m, &e("a"), Rect::new(0, 0, 400, 120), 2);
    assert_eq!(
        l.corner(),
        Some(Rect::new(4 + 392 - 34, 4 + 112 - 34, 34, 34))
    );
}

#[test]
fn layout_adds_scrollbars_when_the_text_outgrows_the_box() {
    let text = TextRenderer::load(&FontConfig::default());
    let m = Metrics {
        bars_always: false,
        ..Metrics::default()
    };
    let r = Rect::new(0, 0, 200, 60);
    let short = e("a\nb");
    let l = layout(&text, &m, &short, r, 1);
    assert_eq!(l.inner, r.inset(2));
    assert_eq!(l.rows, 4);
    assert!(l.vscroll.is_none() && l.hscroll.is_none());
    let l = layout(&text, &Metrics::default(), &short, r, 1);
    assert_eq!(l.vscroll, Some(Rect::new(2 + 196 - 16, 2, 16, 56 - 16)));
    assert_eq!(l.hscroll, Some(Rect::new(2, 2 + 56 - 16, 196 - 16, 16)));
    assert_eq!(
        l.corner(),
        Some(Rect::new(2 + 196 - 16, 2 + 56 - 16, 16, 16))
    );
    assert_eq!(l.rows, 3);
    let tall = e("a\nb\nc\nd\ne\nf");
    let l = layout(&text, &m, &tall, r, 1);
    assert_eq!(l.vscroll, Some(Rect::new(2 + 196 - 16, 2, 16, 56)));
    assert_eq!(l.inner.width, 196 - 16);
    let wide = e(&"x".repeat(80));
    let l = layout(&text, &m, &wide, r, 1);
    assert!(l.hscroll.is_some());
    assert_eq!(l.rows, (56 - 16) / 13);
}

#[test]
fn scrollbar_presses_map_to_parts_and_a_thumb_drag_scrolls() {
    let text = TextRenderer::load(&FontConfig::default());
    let m = Metrics::default();
    let mut src = "x".repeat(200);
    for _ in 0..39 {
        src.push_str("\na");
    }
    let t = e(&src);
    let r = Rect::new(0, 0, 200, 150);
    let l = layout(&text, &m, &t, r, 1);
    let vb = l.vscroll.unwrap();
    let hb = l.hscroll.unwrap();
    let view = View::default();
    let hit = |x, y| bar_hit(&text, &m, &t, &l, view, x, y).and_then(|(b, h)| Some((b, h.part()?)));
    assert_eq!(
        hit(vb.x + 8, vb.y + 8),
        Some((ScrollBar::Vertical, ScrollHit::Back))
    );
    assert_eq!(
        hit(vb.x + 8, vb.bottom() - 8),
        Some((ScrollBar::Vertical, ScrollHit::Forward))
    );
    let vp = bar_parts(&text, &m, &t, &l, view, ScrollBar::Vertical).unwrap();
    let thumb = vp.thumb.unwrap();
    assert_eq!(thumb.y, vp.track.y, "at the top the thumb leads the track");
    assert_eq!(
        hit(vb.x + 8, thumb.bottom() + 1),
        Some((ScrollBar::Vertical, ScrollHit::PageForward))
    );
    assert_eq!(
        hit(hb.x + 8, hb.y + 8),
        Some((ScrollBar::Horizontal, ScrollHit::Back))
    );
    assert_eq!(
        hit(hb.right() - 8, hb.y + 8),
        Some((ScrollBar::Horizontal, ScrollHit::Forward))
    );
    assert_eq!(hit(100, 30), None);
    let (bar, h) = bar_hit(&text, &m, &t, &l, view, vb.x + 8, thumb.y + 2).unwrap();
    assert_eq!((bar, h), (ScrollBar::Vertical, BarHit::Thumb(2)));
    let mut held = ThumbDrag::default();
    held.press(bar.axis(), h);
    let at = |y| drag_bar(&text, &m, &t, &l, view, &held, (vb.x + 8, y)).unwrap();
    assert_eq!(at(vp.track.bottom()).top, 40 - l.rows);
    assert_eq!(at(thumb.y + 2).top, 0);
    let mid = vp.track.y + (vp.track.height - thumb.height) / 2 + 2;
    assert_eq!(at(mid).top, (40 - l.rows) / 2);
    let hp = bar_parts(&text, &m, &t, &l, view, ScrollBar::Horizontal).unwrap();
    let total = widest_px(&text, &m, &t) + 2;
    held.press(Axis::Horizontal, BarHit::Thumb(0));
    let end = drag_bar(&text, &m, &t, &l, view, &held, (hp.track.right(), 0)).unwrap();
    assert_eq!(end.left, total - l.inner.width);
}

#[test]
fn view_follows_the_caret_and_clicks_land() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let m = Metrics::default();
    let mut t = e("a\nb\nc\nd\ne\nf\ng");
    let r = Rect::new(0, 0, 200, 60);
    let lay = layout(&text, &m, &t, r, 1);
    let mut v = View::default();
    k(&mut t, MultiKey::DocEnd { select: false });
    v.follow(&text, &m, &t, &lay, 1);
    assert_eq!(v.top, 7 - lay.rows);
    let q = pos_at(&text, &m, &t, &lay, v, 4, lay.inner.y + 1 + 13 + 3, 1);
    assert_eq!(q, p(v.top + 1, 0));
    // Draws without panicking, caret on the last row.
    let mut c = Canvas::new(200, 60);
    draw(
        &mut c,
        &text,
        &classic_paint(&theme.colors, 1),
        &m,
        &t,
        v,
        &lay,
        Caret::Shown,
        None,
    );
    assert_eq!(c.get(0, 0), theme.colors.button_shadow.argb());
    assert_eq!(c.get(3, 3), theme.colors.window.argb());
}

#[test]
fn a_drag_resting_in_an_edge_row_or_band_scrolls_one_step() {
    let text = TextRenderer::load(&FontConfig::default());
    let m = Metrics::default();
    let long = "x".repeat(80);
    let t = e(&format!("{long}\n{}", "l\n".repeat(49)));
    let lay = layout(&text, &m, &t, Rect::new(0, 0, 200, 80), 1);
    let i = lay.inner;
    let (mid_x, mid_y) = (i.x + i.width / 2, i.y + 1 + 13 + 6);
    let last_row = i.y + 1 + (lay.rows as i32 - 1) * 13;
    let step = |v: View, x, y| drag_scroll(&text, &m, &t, &lay, v, x, y, 1);
    let at = |top, left| View {
        top,
        left,
        ..View::default()
    };
    assert_eq!(step(at(5, 0), mid_x, mid_y), None);
    let bar = lay.vscroll.unwrap();
    assert_eq!(step(at(5, 0), bar.x + 2, last_row + 2), None);
    assert_eq!(step(at(5, 0), mid_x, i.y), Some(at(4, 0)));
    assert_eq!(step(at(5, 0), mid_x, i.y + 13), Some(at(4, 0)));
    assert_eq!(step(at(5, 0), mid_x, last_row), Some(at(6, 0)));
    assert_eq!(step(at(5, 0), mid_x, i.bottom() - 1), Some(at(6, 0)));
    assert_eq!(step(at(0, 0), mid_x, i.y), None);
    assert_eq!(
        step(at(t.lines().len() - lay.rows, 0), mid_x, last_row),
        None
    );
    assert_eq!(step(at(5, 0), i.right() - 1, mid_y), Some(at(5, 13)));
    assert_eq!(step(at(5, 13), i.x, mid_y), Some(at(5, 0)));
    assert_eq!(step(at(5, 5), i.x + 12, mid_y), Some(at(5, 0)));
    assert_eq!(step(at(5, 0), i.x, mid_y), None);
    let max = (widest_px(&text, &m, &t) + 2) - i.width;
    assert_eq!(step(at(5, max - 3), i.right() - 1, mid_y), Some(at(5, max)));
    assert_eq!(step(at(5, max), i.right() - 1, mid_y), None);
    assert_eq!(step(at(5, 0), i.right() - 1, i.y), Some(at(4, 13)));
}

#[test]
fn a_selection_drag_past_an_edge_scrolls_on_ticks_and_extends_to_the_edge_row() {
    let text = TextRenderer::load(&FontConfig::default());
    let m = Metrics::default();
    let mut t = e(&"line\n".repeat(50));
    let lay = layout(&text, &m, &t, Rect::new(0, 0, 200, 80), 1);
    let i = lay.inner;
    let rows = lay.rows;
    let mut v = View::default();
    let drag = |t: &mut MultiEdit, v: &mut View, x, y, tick| {
        select_drag(&text, &m, t, &lay, v, x, y, 1, tick)
    };
    t.place(Pos { line: 1, col: 2 }, false);
    let below = i.bottom() + 40;
    let r = drag(&mut t, &mut v, i.x + 20, below, false);
    assert!(r.changed && r.scrolling);
    assert_eq!(v.top, 0, "motion past the edge does not scroll");
    assert_eq!(
        t.caret().line,
        rows - 1,
        "the caret goes to the last full row"
    );
    for n in 1..=3 {
        let r = drag(&mut t, &mut v, i.x + 20, below, true);
        assert!(r.changed && r.scrolling);
        assert_eq!(v.top, n);
        assert_eq!(t.caret().line, n + rows - 1);
    }
    assert_eq!(t.selection(), (Pos { line: 1, col: 2 }, t.caret()));
    let r = drag(&mut t, &mut v, i.x + 20, i.y - 30, true);
    assert_eq!((v.top, t.caret().line), (2, 2), "above the edit scrolls up");
    assert!(r.scrolling);
    let r = drag(&mut t, &mut v, i.x + 20, i.y + 1 + 13 + 6, true);
    assert!(!r.scrolling, "the middle rests still");
    assert_eq!(v.top, 2);

    let mut t = e(&format!("{}\nb", "x".repeat(80)));
    let mut v = View::default();
    let r = drag(&mut t, &mut v, i.right() + 50, i.y + 2, true);
    assert!(
        r.scrolling && v.left > 0,
        "past the right side scrolls right"
    );
    let left = v.left;
    let r = drag(&mut t, &mut v, i.x - 50, i.y + 2, true);
    assert!(
        r.changed && v.left < left,
        "past the left side scrolls back"
    );
}

#[test]
fn a_scrolled_view_shifts_every_line_left() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let m = Metrics::default();
    let t = e("ab\nabcdefghijklmnopqrstuvwxyz abcdefghijklmnopqrstuvwxyz");
    let r = Rect::new(0, 0, 120, 60);
    let lay = layout(&text, &m, &t, r, 1);
    let mut flat = Canvas::new(400, 60);
    let wide = layout(&text, &m, &t, Rect::new(0, 0, 400, 60), 1);
    draw(
        &mut flat,
        &text,
        &classic_paint(&theme.colors, 1),
        &m,
        &t,
        View::default(),
        &wide,
        Caret::Unfocused,
        None,
    );
    let v = View {
        left: 40,
        ..View::default()
    };
    let mut c = Canvas::new(120, 60);
    draw(
        &mut c,
        &text,
        &classic_paint(&theme.colors, 1),
        &m,
        &t,
        v,
        &lay,
        Caret::Unfocused,
        None,
    );
    let x0 = lay.inner.x + 1;
    for y in lay.inner.y + 1..lay.inner.y + 2 * m.line_h {
        for x in x0..lay.inner.right() - 1 {
            assert_eq!(c.get(x, y), flat.get(x + 40, y), "at {x},{y}");
        }
    }
}

#[test]
fn find_returns_none_for_an_empty_query() {
    let lines = vec!["anything".to_string()];
    let o = FindOpts {
        match_case: false,
        down: true,
    };
    assert_eq!(find(&lines, "", Pos::default(), &o), None);
}

#[test]
fn find_walks_down_from_the_caret() {
    let lines = vec!["hello world".to_string(), "hello again".to_string()];
    let o = FindOpts {
        match_case: true,
        down: true,
    };
    assert_eq!(
        find(&lines, "hello", Pos::default(), &o),
        Some((Pos { line: 0, col: 0 }, Pos { line: 0, col: 5 }))
    );
    assert_eq!(
        find(&lines, "hello", Pos { line: 0, col: 1 }, &o),
        Some((Pos { line: 1, col: 0 }, Pos { line: 1, col: 5 }))
    );
}

#[test]
fn find_up_finds_the_match_before_the_caret() {
    let lines = vec!["hello hello".to_string()];
    let o = FindOpts {
        match_case: true,
        down: false,
    };
    assert_eq!(
        find(&lines, "hello", Pos { line: 0, col: 11 }, &o),
        Some((Pos { line: 0, col: 6 }, Pos { line: 0, col: 11 }))
    );
    assert_eq!(
        find(&lines, "hello", Pos { line: 0, col: 6 }, &o),
        Some((Pos { line: 0, col: 0 }, Pos { line: 0, col: 5 }))
    );
}

#[test]
fn find_does_not_wrap() {
    let lines = vec!["alpha".to_string(), "beta".to_string()];
    let down = FindOpts {
        match_case: true,
        down: true,
    };
    assert_eq!(find(&lines, "alpha", Pos { line: 1, col: 4 }, &down), None);
    let up = FindOpts {
        match_case: true,
        down: false,
    };
    assert_eq!(find(&lines, "beta", Pos { line: 0, col: 0 }, &up), None);
}

#[test]
fn find_ignores_case_only_when_asked() {
    let lines = vec!["Hello".to_string()];
    let sensitive = FindOpts {
        match_case: true,
        down: true,
    };
    let insensitive = FindOpts {
        match_case: false,
        down: true,
    };
    assert_eq!(find(&lines, "hello", Pos::default(), &sensitive), None);
    assert_eq!(
        find(&lines, "hello", Pos::default(), &insensitive),
        Some((Pos { line: 0, col: 0 }, Pos { line: 0, col: 5 }))
    );
}

#[test]
fn find_returns_byte_offsets_on_a_multibyte_line() {
    let lines = vec!["héllo wörld".to_string()];
    let o = FindOpts {
        match_case: false,
        down: true,
    };
    let (s, e) = find(&lines, "WÖRLD", Pos::default(), &o).expect("the match is there");
    assert_eq!(&lines[0][s.col..e.col], "wörld");
}

/// Replace All rewrites the whole document and must not run over its own output: replacing "a"
/// with "aa" terminates and reports 3.
#[test]
fn replace_all_does_not_rerun_over_its_own_output() {
    let mut lines = vec!["a b a".to_string(), "a".to_string()];
    let o = FindOpts {
        match_case: true,
        down: true,
    };
    assert_eq!(replace_all(&mut lines, "a", "aa", &o), 3);
    assert_eq!(lines, vec!["aa b aa".to_string(), "aa".to_string()]);
}

#[test]
fn replace_all_leaves_a_valid_caret_and_one_undo_step() {
    let mut t = e("cat cat cat");
    k(&mut t, MultiKey::Edit(EditKey::Insert("x".into())));
    t.place(p(0, 5), false);
    t.place(p(0, 8), true);
    assert_eq!(t.selected_text(), "cat");
    let o = FindOpts {
        match_case: true,
        down: true,
    };
    assert_eq!(t.replace_all("cat", "c", &o), 3);
    assert_eq!(t.text(), "xc c c");
    assert_eq!(t.caret, p(0, 6));
    assert!(!t.has_selection());
    assert_eq!(t.selected_text(), "");
    k(&mut t, MultiKey::Edit(EditKey::Undo));
    assert_eq!(
        t.text(),
        "xcat cat cat",
        "Ctrl+Z undoes the Replace All alone"
    );
    assert_eq!(t.replace_all("zzz", "y", &o), 0);
    k(&mut t, MultiKey::Edit(EditKey::Undo));
    assert_eq!(
        t.text(),
        "cat cat cat",
        "a Replace All that matched nothing adds no step"
    );
    k(&mut t, MultiKey::Edit(EditKey::Redo));
    k(&mut t, MultiKey::Edit(EditKey::Redo));
    assert_eq!(t.text(), "xc c c", "Ctrl+Y makes the Replace All again");
}

fn typed(t: &mut MultiEdit, s: &str) {
    for c in s.chars() {
        k(t, MultiKey::Edit(EditKey::Insert(c.into())));
    }
}

#[test]
fn ctrl_z_walks_back_a_step_at_a_time_and_ctrl_y_forward() {
    let mut t = e("");
    typed(&mut t, "abc");
    k(&mut t, MultiKey::Enter);
    typed(&mut t, "def");
    assert_eq!(t.undo.held().0, 3, "a typing run is one op");
    let undo = |t: &mut MultiEdit| k(t, MultiKey::Edit(EditKey::Undo));
    let redo = |t: &mut MultiEdit| k(t, MultiKey::Edit(EditKey::Redo));
    assert!(undo(&mut t));
    assert_eq!((t.text().as_str(), t.caret), ("abc\n", p(1, 0)));
    assert!(undo(&mut t));
    assert_eq!((t.text().as_str(), t.caret), ("abc", p(0, 3)));
    assert!(undo(&mut t));
    assert_eq!((t.text().as_str(), t.caret), ("", p(0, 0)));
    assert!(!undo(&mut t), "nothing older");
    assert!(redo(&mut t));
    assert_eq!(t.text(), "abc");
    assert!(redo(&mut t));
    assert_eq!((t.text().as_str(), t.caret), ("abc\n", p(1, 0)));
    assert!(redo(&mut t));
    assert_eq!((t.text().as_str(), t.caret), ("abc\ndef", p(1, 3)));
    assert!(!redo(&mut t));
}

#[test]
fn undo_takes_a_paste_back_to_the_exact_text_and_caret() {
    let mut t = e("one two\nthree");
    t.place(p(0, 4), false);
    t.place(p(1, 2), true);
    assert!(t.paste("big\r\nred\nfat "));
    assert_eq!(t.text(), "one big\nred\nfat ree");
    assert!(k(&mut t, MultiKey::Edit(EditKey::Undo)));
    assert_eq!(t.text(), "one two\nthree");
    assert_eq!((t.anchor, t.caret), (p(0, 4), p(1, 2)));
    assert!(k(&mut t, MultiKey::Edit(EditKey::Redo)));
    assert_eq!(
        (t.text().as_str(), t.caret),
        ("one big\nred\nfat ree", p(2, 4))
    );
}

#[test]
fn deleting_across_lines_is_one_step_and_comes_back_whole() {
    let mut t = e("ab\ncd");
    t.place(p(1, 1), false);
    for _ in 0..3 {
        k(&mut t, MultiKey::Edit(EditKey::Backspace { word: false }));
    }
    assert_eq!(t.text(), "ad");
    assert_eq!(t.undo.held(), (1, 3), "one op holding b, the break and c");
    k(&mut t, MultiKey::Edit(EditKey::Undo));
    assert_eq!((t.text().as_str(), t.caret), ("ab\ncd", p(1, 1)));
}

#[test]
fn the_undo_log_holds_what_changed_never_the_document() {
    let doc: String = (0..5000).map(|i| format!("line {i}\n")).collect();
    let mut t = e(&doc);
    for _ in 0..50 {
        k(&mut t, MultiKey::Enter);
        typed(&mut t, "xy");
    }
    assert_eq!(
        t.undo.held(),
        (100, 150),
        "an Enter and two chars per round"
    );
    for _ in 0..100 {
        k(&mut t, MultiKey::Edit(EditKey::Undo));
    }
    assert_eq!(t.text(), doc);
}

#[test]
fn char_col_counts_characters_not_bytes() {
    const COL_AFTER_AB_TAB_C: usize = 5;
    let line = "ab\tc";
    assert_eq!(char_col(line, line.len()), COL_AFTER_AB_TAB_C);
    let wide = "héllo";
    assert_eq!(char_col(wide, wide.len()), 6);
    assert_eq!(char_col(wide, 0), 1);
}

#[test]
fn lines_are_runs_and_the_caret_lands_on_its_line() {
    use crate::access::{ROOT, Tree, accesskit::Role};
    let text = TextRenderer::load(&crate::theme::FontConfig::default());
    let m = Metrics::default();
    let mut edit = MultiEdit::new("hello\nworld");
    edit.place(Pos { line: 1, col: 2 }, false);
    let lay = layout(&text, &m, &edit, Rect::new(0, 20, 300, 200), 1);
    let mut tree = Tree::window("T", (300, 220));
    let id = access(
        &edit,
        View::default(),
        &lay,
        &m,
        &text,
        &mut tree,
        ROOT,
        "edit",
    );
    let node = tree.get(id).unwrap();
    assert_eq!(node.role(), Role::MultilineTextInput);
    let runs = node.children().to_vec();
    let values: Vec<_> = runs.iter().map(|r| tree.get(*r).unwrap().value()).collect();
    assert_eq!(values, [Some("hello\n"), Some("world")]);
    let second = tree.get(runs[1]).unwrap().bounds().unwrap();
    assert_eq!(second.y0, f64::from(lay.inner.y + 1 + m.line_h));
    let sel = node.text_selection().unwrap();
    assert_eq!((sel.focus.node, sel.focus.character_index), (runs[1], 2));
}

#[test]
fn a_long_document_publishes_runs_for_the_rows_in_view_and_the_caret_line() {
    use crate::access::{ROOT, Tree};
    let text = TextRenderer::load(&crate::theme::FontConfig::default());
    let m = Metrics::default();
    let mut edit = MultiEdit::new(&"line\n".repeat(10_000));
    edit.place(
        Pos {
            line: 9_000,
            col: 2,
        },
        false,
    );
    let r = Rect::new(0, 0, 300, 30 * m.line_h + 4);
    let lay = layout(&text, &m, &edit, r, 1);
    let rows = lay.rows;
    assert!((28..=30).contains(&rows));
    let view = View {
        top: 100,
        ..View::default()
    };
    let mut tree = Tree::window("T", (300, r.height));
    let id = access(&edit, view, &lay, &m, &text, &mut tree, ROOT, "edit");
    let node = tree.get(id).unwrap();
    let runs = node.children().to_vec();
    assert_eq!(
        runs.len(),
        rows + 1,
        "the rows in view and the caret's line"
    );
    assert_eq!(tree.get(runs[0]).unwrap().value(), Some("line\n"));
    let sel = node.text_selection().unwrap();
    assert_eq!((sel.focus.node, sel.focus.character_index), (runs[rows], 2));
    let line_h = f64::from(m.line_h);
    assert_eq!(node.scroll_y(), Some(100.0 * line_h));
    assert_eq!(node.scroll_y_max(), Some((10_001 - rows) as f64 * line_h));
    assert_eq!(node.value().map(str::len), Some(edit.text().len()));

    let big = MultiEdit::new(&format!("{}\n", "x".repeat(99)).repeat(20_000));
    let lay = layout(&text, &m, &big, r, 1);
    let mut tree = Tree::window("T", (300, r.height));
    let id = access(
        &big,
        View::default(),
        &lay,
        &m,
        &text,
        &mut tree,
        ROOT,
        "edit",
    );
    assert_eq!(
        tree.get(id).unwrap().value().map(str::len),
        Some(lay.rows * 100 - 1),
        "a document past 1 MB reads only the rows in view"
    );
}

#[test]
fn a_blue_text_boxs_lit_bar_part_draws_hot() {
    use crate::classic::widgets::{Axis, ScrollHit};
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let text = TextRenderer::load(&FontConfig::default());
    let mut m = Metrics::default();
    m.follow_look(&theme.look.controls_at_100());
    let t = e(&"line\n".repeat(60));
    let r = Rect::new(0, 0, 200, 120);
    let lay = layout(&text, &m, &t, r, 1);
    let sb = lay.vscroll.expect("60 lines overflow the box");
    let mut view = View::default();
    let up = bar_hit(&text, &m, &t, &lay, view, sb.x + 8, sb.y + 2)
        .and_then(|(b, h)| Some((b.axis(), h.part()?)));
    assert_eq!(up, Some((Axis::Vertical, ScrollHit::Back)));
    view.bars.motion(up);
    let mut c = Canvas::new(200, 120);
    draw(
        &mut c,
        &text,
        &theme.paint(),
        &m,
        &t,
        view,
        &lay,
        Caret::Unfocused,
        None,
    );
    assert_eq!(
        c.get(sb.x + 8, sb.y + 2),
        crate::look::styled::fixture::blue_pixel(
            crate::style::Part::ScrollArrowUp,
            crate::style::State::Hot,
            8,
            2
        ),
        "the hot up arrow"
    );
}

#[test]
fn right_and_delete_step_over_a_zwj_family_whole() {
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let mut t = e(&format!("{family}\nx"));
    t.place(p(0, 0), false);
    k(
        &mut t,
        MultiKey::Edit(EditKey::Right {
            select: false,
            word: false,
        }),
    );
    assert_eq!(t.caret, p(0, family.len()));
    t.place(p(0, 0), false);
    k(&mut t, MultiKey::Edit(EditKey::Delete { word: false }));
    assert_eq!(t.lines, ["", "x"]);
}

#[test]
fn a_blink_only_frame_damages_just_the_caret_bar() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let m = Metrics::default();
    let mut t = e("one\ntwo");
    t.place(p(1, 2), false);
    let lay = layout(&text, &m, &t, Rect::new(0, 0, 200, 60), 1);
    let draw_with = |caret, blink_only| {
        let mut c = Canvas::new(200, 60);
        c.set_blink_only(blink_only);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &m,
            &t,
            View::default(),
            &lay,
            caret,
            None,
        );
        c
    };
    let mut full = draw_with(Caret::Shown, false);
    assert_eq!(
        full.take_damage(),
        None,
        "a normal frame damages everything"
    );
    let mut on = draw_with(Caret::Shown, true);
    let off = draw_with(Caret::Hidden, true);
    let spot = on.take_spot().expect("focused").caret;
    let damage = on.take_damage().expect("a blink damages");
    let [bar] = damage[..] else {
        panic!("one damaged rect, got {damage:?}")
    };
    assert_eq!((bar.width, bar.height), (spot.width, spot.height));
    assert!(
        (spot.x - 1..=spot.x).contains(&bar.x) && bar.y == spot.y,
        "the bar {bar:?} is the spot's {spot:?} or a pixel left of it"
    );
    assert_eq!(on.pixels, full.pixels, "the same pixels either way");
    let changed: Vec<usize> = (0..on.pixels.len())
        .filter(|&i| on.pixels[i] != off.pixels[i])
        .collect();
    assert!(!changed.is_empty());
    assert!(
        changed
            .iter()
            .all(|&i| bar.contains((i % 200) as i32, (i / 200) as i32)),
        "the blink changes nothing outside the bar"
    );
}

/// The A's foot inks the pen's column: the caret and the drop point stand in the free column left
/// of it.
#[test]
fn the_caret_and_the_drop_point_stand_off_a_stem_on_their_pen() {
    crate::pins::fonts();
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let m = Metrics::default();
    let mut t = e("AVA");
    t.place(p(0, 0), false);
    let lay = layout(&text, &m, &t, Rect::new(0, 0, 200, 60), 1);
    let paint = classic_paint(&theme.colors, 1);
    let draw_with = |caret, drop| {
        let mut c = Canvas::new(200, 60);
        draw(
            &mut c,
            &text,
            &paint,
            &m,
            &t,
            View::default(),
            &lay,
            caret,
            drop,
        );
        c
    };
    let off = draw_with(Caret::Hidden, None);
    let pen = Rect::new(lay.inner.x + 1, lay.inner.y + 1, 1, m.line_h);
    let paper = theme.colors.window.argb();
    let on = draw_with(Caret::Shown, None);
    assert_eq!(
        crate::paint::inverted_bar(&off, &on, paper),
        (
            Rect {
                x: pen.x - 1,
                ..pen
            },
            false
        ),
        "caret"
    );
    let drop = draw_with(Caret::Hidden, Some(p(0, 0)));
    let bar = crate::paint::inverted_bar(&off, &drop, paper);
    assert_eq!(
        bar,
        (
            Rect {
                x: pen.x - 1,
                ..pen
            },
            false
        ),
        "drop point"
    );
}

#[test]
fn a_drop_point_at_the_caret_hides_the_caret() {
    crate::pins::fonts();
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let m = Metrics::default();
    let mut t = e("AVA");
    t.place(p(0, 1), false);
    let lay = layout(&text, &m, &t, Rect::new(0, 0, 200, 60), 1);
    let paint = classic_paint(&theme.colors, 1);
    let draw_with = |caret, drop| {
        let mut c = Canvas::new(200, 60);
        draw(
            &mut c,
            &text,
            &paint,
            &m,
            &t,
            View::default(),
            &lay,
            caret,
            drop,
        );
        c
    };
    let off = draw_with(Caret::Hidden, None);
    let paper = theme.colors.window.argb();
    let drop = draw_with(Caret::Hidden, Some(p(0, 1)));
    assert_eq!(crate::paint::inverted_bar(&off, &drop, paper).0.width, 1);
    assert!(
        draw_with(Caret::Shown, Some(p(0, 1))) == drop,
        "a shown caret adds nothing to the drop bar"
    );
}

#[test]
fn a_sideways_scroll_steps_character_cells_to_the_text_end() {
    let text = TextRenderer::load(&FontConfig::default());
    let m = Metrics::default();
    let t = e(&"M".repeat(60));
    let l = layout(&text, &m, &t, Rect::new(0, 0, 200, 150), 1);
    let cell = text.measure("M", m.bold, m.font_px);
    let mut v = View::default();
    assert!(v.scroll_across(2, &text, &m, &t, &l));
    assert_eq!(v.left, 2 * cell, "two columns are two cells");
    assert!(v.scroll_across(-5, &text, &m, &t, &l));
    assert_eq!(v.left, 0, "held at the start");
    v.scroll_across(1000, &text, &m, &t, &l);
    assert_eq!(
        v.left,
        widest_px(&text, &m, &t) + 2 - l.inner.width,
        "held at the end"
    );
    let wrapped = Metrics { wrap: true, ..m };
    let wl = layout(&text, &wrapped, &t, Rect::new(0, 0, 200, 150), 1);
    let mut w = View::default();
    assert!(
        !w.scroll_across(2, &text, &wrapped, &t, &wl),
        "a wrapping box stays put"
    );
}

/// At scale 2 the selected part of a line draws over the line in the same places, so no ink of
/// the line's colour shows inside the highlight, whatever char ends the selection.
#[test]
fn a_selection_at_scale_2_shows_no_line_ink_inside_its_highlight() {
    crate::pins::fonts();
    let cs = &Theme::default().colors;
    let mut ghosts = Vec::new();
    for (family, antialias) in ["Tahoma", "DejaVu Sans"]
        .into_iter()
        .flat_map(|f| [(f, false), (f, true)])
    {
        let text = TextRenderer::load(&FontConfig {
            family: vec![family.into()],
            antialias,
            ..FontConfig::default()
        });
        for px in [11, 13] {
            let m = Metrics {
                font_px: px,
                ..Metrics::for_font(&text, px)
            };
            for ch in ('!'..='~').filter(char::is_ascii_graphic) {
                let mut t = e(&format!("o{ch}o"));
                t.place(p(0, 0), false);
                t.place(p(0, 2), true);
                let lay = layout(&text, &m, &t, Rect::new(0, 0, 200, 60), 1);
                let mut c = Canvas::with_scale(200, 60, 2);
                let paint = classic_paint(cs, 1);
                let (v, caret) = (View::default(), Caret::Hidden);
                draw(&mut c, &text, &paint, &m, &t, v, &lay, caret, None);
                let x0 = lay.inner.x + 1;
                let xb = x0 + text.measure(&format!("o{ch}"), false, px);
                let hl = Rect::new(x0, lay.inner.y + 1, xb - x0, m.line_h).scaled(2);
                let dark = (hl.y..hl.bottom())
                    .flat_map(|y| (hl.x..hl.right()).map(move |x| (x, y)))
                    .filter(|&(x, y)| {
                        let p = c.pixels[(y * 400 + x) as usize];
                        (p >> 16) & 0xff < 0x60 && p & 0xff < 0x60
                    })
                    .count();
                if dark > 0 {
                    ghosts.push(format!(
                        "{family} aa {antialias} {px} px: \"o{ch}\"|o {dark} px"
                    ));
                }
            }
        }
    }
    assert!(ghosts.is_empty(), "{}", ghosts.join("\n"));
}

/// A one-line box at (0, 0) 200x60 on a scale 2 canvas, as its device pixels (400 a row): each
/// printable ASCII char with `f` drawing the box holding a given text with a given selection,
/// in Tahoma and DejaVu Sans, mono and antialiased, 11 and 13 px. `f` also gets the left of the
/// highlight of a selection from char 1.
fn each_row_end(
    mut f: impl FnMut(char, &dyn Fn(&str, Option<(usize, usize)>) -> Vec<u32>, usize, &str),
) {
    let cs = &Theme::default().colors;
    for (family, antialias) in ["Tahoma", "DejaVu Sans"]
        .into_iter()
        .flat_map(|f| [(f, false), (f, true)])
    {
        let text = TextRenderer::load(&FontConfig {
            family: vec![family.into()],
            antialias,
            ..FontConfig::default()
        });
        for px in [11, 13] {
            let m = Metrics {
                font_px: px,
                ..Metrics::for_font(&text, px)
            };
            let lay = |t: &MultiEdit| layout(&text, &m, t, Rect::new(0, 0, 200, 60), 1);
            let run = |s: &str, sel: Option<(usize, usize)>| {
                let mut t = e(s);
                if let Some((a, b)) = sel {
                    t.place(p(0, a), false);
                    t.place(p(0, b), true);
                }
                let mut c = Canvas::with_scale(200, 60, 2);
                let (v, caret) = (View::default(), Caret::Hidden);
                draw(
                    &mut c,
                    &text,
                    &classic_paint(cs, 1),
                    &m,
                    &t,
                    v,
                    &lay(&t),
                    caret,
                    None,
                );
                c.pixels
            };
            let xa = 2 * (lay(&e("o")).inner.x + 1 + text.measure("o", false, px));
            for ch in ('!'..='~').filter(char::is_ascii_graphic) {
                let what = format!("{family} aa {antialias} {px} px");
                f(ch, &run, xa as usize, &what);
            }
        }
    }
}

#[test]
fn a_rows_last_glyph_draws_at_scale_2_where_it_draws_with_a_char_after_it() {
    crate::pins::fonts();
    let mut moved = Vec::new();
    each_row_end(|ch, run, _, what| {
        if run(&format!("o{ch}"), None) != run(&format!("o{ch} "), None) {
            moved.push(format!("{what}: \"o{ch}\""));
        }
    });
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

/// Left of a row's selected last char the box shows what it shows when a char follows it.
#[test]
fn a_selected_last_glyph_at_scale_2_keeps_its_ink_inside_its_highlight() {
    crate::pins::fonts();
    let mut dark = Vec::new();
    each_row_end(|ch, run, xa, what| {
        let left = |s: &str| -> Vec<u32> {
            let px = run(s, Some((1, 2)));
            px.chunks(400).flat_map(|row| row[..xa].to_vec()).collect()
        };
        if left(&format!("o{ch}")) != left(&format!("o{ch} ")) {
            dark.push(format!("{what}: \"o|{ch}|\""));
        }
    });
    assert!(dark.is_empty(), "{}", dark.join("\n"));
}

/// Each printable ASCII char, in Tahoma and DejaVu Sans, mono and antialiased, 11 and 13 px, with
/// `f` drawing a flat box at `r` holding a given text scrolled `left` px, as device pixels of a
/// 240x60 scale 2 canvas (480 a row), and the width of a space.
fn each_row_start(mut f: impl FnMut(char, &dyn Fn(&str, Rect, i32) -> Vec<u32>, i32, &str)) {
    let cs = &Theme::default().colors;
    for (family, antialias) in ["Tahoma", "DejaVu Sans"]
        .into_iter()
        .flat_map(|f| [(f, false), (f, true)])
    {
        let text = TextRenderer::load(&FontConfig {
            family: vec![family.into()],
            antialias,
            ..FontConfig::default()
        });
        for px in [11, 13] {
            let m = Metrics {
                font_px: px,
                flat: true,
                bars_always: false,
                ..Metrics::for_font(&text, px)
            };
            let run = |s: &str, r: Rect, left: i32| {
                let t = e(s);
                let mut c = Canvas::with_scale(240, 60, 2);
                c.fill_rect(c.bounds(), cs.window);
                let v = View {
                    left,
                    ..View::default()
                };
                let lay = layout(&text, &m, &t, r, 1);
                let paint = classic_paint(cs, 1);
                draw(
                    &mut c,
                    &text,
                    &paint,
                    &m,
                    &t,
                    v,
                    &lay,
                    Caret::Unfocused,
                    None,
                );
                c.pixels
            };
            let space = text.measure(" ", false, px);
            for ch in ('!'..='~').filter(char::is_ascii_graphic) {
                f(ch, &run, space, &format!("{family} aa {antialias} {px} px"));
            }
        }
    }
}

#[test]
fn a_rows_first_glyph_draws_at_scale_2_where_it_draws_with_a_char_before_it() {
    crate::pins::fonts();
    let mut moved = Vec::new();
    each_row_start(|ch, run, space, what| {
        let at = |x: i32| Rect::new(x, 0, 200 - x, 60);
        if run(&ch.to_string(), at(20), 0) != run(&format!(" {ch}"), at(20 - space), 0) {
            moved.push(format!("{what}: {ch:?}"));
        }
    });
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

/// Scrolled across, a box shows inside its text clip what an unscrolled box that far left shows:
/// no glyph moves as it reaches the clip's left edge.
#[test]
fn a_box_scrolled_at_scale_2_draws_its_glyphs_where_their_pens_put_them() {
    crate::pins::fonts();
    let mut moved = Vec::new();
    each_row_start(|ch, run, _, what| {
        let row = format!("{ch}o{ch}o{ch}o{ch}");
        for left in 1..=8 {
            let scrolled = run(&row, Rect::new(20, 0, 180, 60), left);
            let shifted = run(&row, Rect::new(20 - left, 0, 180 + left, 60), 0);
            let clip =
                (2 * 21..2 * 199).flat_map(|x| (2..2 * 59).map(move |y| (y * 480 + x) as usize));
            if clip.clone().any(|i| scrolled[i] != shifted[i]) {
                moved.push(format!("{what}: {row:?} {left} px"));
            }
        }
    });
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

#[test]
fn a_box_scrolled_at_scale_2_keeps_the_inset_left_of_its_text_clip_clear() {
    crate::pins::fonts();
    let mut inked = Vec::new();
    each_row_start(|ch, run, _, what| {
        if !matches!(ch, 'J' | 'j') {
            return;
        }
        let r = Rect::new(20, 0, 180, 60);
        let blank = run("", r, 0);
        let row = format!("{ch}o{ch}o{ch}o{ch}");
        for left in 1..=8 {
            let scrolled = run(&row, r, left);
            let inset =
                (2 * 20..2 * 21).flat_map(|x| (0..120).map(move |y| (y * 480 + x) as usize));
            if inset.clone().any(|i| scrolled[i] != blank[i]) {
                inked.push(format!("{what}: {row:?} {left} px"));
            }
        }
    });
    assert!(inked.is_empty(), "{}", inked.join("\n"));
}

#[test]
fn a_selected_glyph_at_scale_2_lights_whole_and_leaves_its_neighbour() {
    crate::pins::fonts();
    let cs = &Theme::default().colors;
    let (w, h) = (64, 30);
    let mut faults = Vec::new();
    for family in ["Tahoma", "DejaVu Sans"] {
        let text = TextRenderer::load(&FontConfig {
            family: vec![family.into()],
            antialias: false,
            ..FontConfig::default()
        });
        for px in [11, 13] {
            let m = Metrics {
                bars_always: false,
                ..Metrics::for_font(&text, px)
            };
            let lay = |t: &MultiEdit| layout(&text, &m, t, Rect::new(0, 0, w, h), 1);
            let inner = lay(&e("")).inner;
            let (x0, y) = (inner.x + 1, inner.y + 1);
            let row = Rect::new(x0 - 1, y, inner.right() - x0 + 1, m.line_h);
            let paint_with = |s: i32, line: &str, sel: std::ops::Range<usize>, caret| {
                let mut t = e(line);
                t.place(p(0, sel.start), false);
                t.place(p(0, sel.end), true);
                let mut c = Canvas::with_scale(w as u32, h as u32, s);
                let paint = classic_paint(cs, 1);
                let v = View::default();
                draw(&mut c, &text, &paint, &m, &t, v, &lay(&t), caret, None);
                c.pixels
            };
            let draw = |s, t: &str, sel| paint_with(s, t, sel, Caret::Hidden);
            let caret = |t: &str, sel, shown| paint_with(2, t, sel, Caret::new(true, shown));
            let geom = |_: &str| (x0, row, y..y + m.line_h);
            let pairs = crate::paint::ascii_pairs;
            let (lit, hilight) = (cs.hilight_text, cs.hilight);
            let found = crate::paint::selection_faults(&text, px, (w, h), pairs(), geom, draw, lit);
            let carets =
                crate::paint::caret_faults(&text, px, (w, h), pairs(), geom, caret, hilight);
            let found = found.into_iter().chain(carets);
            faults.extend(found.map(|f| format!("{family} {px} px: {f}")));
        }
    }
    assert!(
        faults.is_empty(),
        "{} faults\n{}",
        faults.len(),
        faults.join("\n")
    );
}

#[test]
fn a_caret_past_a_widened_highlight_at_scale_2_blinks_in_its_damage_and_clip() {
    crate::pins::fonts();
    let cs = &Theme::default().colors;
    let text = TextRenderer::load(&FontConfig {
        family: vec!["Tahoma".into()],
        antialias: false,
        ..FontConfig::default()
    });
    let m = Metrics {
        wrap: true,
        ..Metrics::for_font(&text, 11)
    };
    let mut t = e("m");
    t.place(p(0, 0), false);
    t.place(p(0, 1), true);
    for w in [200, 31] {
        let lay = layout(&text, &m, &t, Rect::new(0, 0, w, 60), 1);
        let draw_with = |caret| {
            let mut c = Canvas::with_scale(w as u32, 60, 2);
            c.set_blink_only(true);
            let paint = classic_paint(cs, 1);
            draw(
                &mut c,
                &text,
                &paint,
                &m,
                &t,
                View::default(),
                &lay,
                caret,
                None,
            );
            c
        };
        let (mut on, off) = (draw_with(Caret::Shown), draw_with(Caret::Hidden));
        let damage = on.take_damage().expect("a blink reports its damage");
        let clip = text_clip(lay.inner, 1).scaled(2);
        let at = |i: usize| (i as i32 % (2 * w), i as i32 / (2 * w));
        let changed: Vec<_> = (0..on.pixels.len())
            .filter(|&i| on.pixels[i] != off.pixels[i])
            .map(at)
            .collect();
        assert!(!changed.is_empty(), "{w} px: the caret blinks");
        let outside = |r: Rect| changed.iter().filter(|&&(x, y)| !r.contains(x, y)).count();
        let undamaged = outside(
            damage
                .iter()
                .fold(Rect::default(), |a, r| a.join(r))
                .scaled(2),
        );
        assert_eq!(
            undamaged, 0,
            "{w} px: every blinking px lies in the damage {damage:?}"
        );
        assert_eq!(
            outside(clip),
            0,
            "{w} px: every blinking px lies in the text clip {clip:?}"
        );
    }
}

/// Rows starting and ending with digits inked from their first column and with descenders, for
/// selections that start and end at row edges.
const EDGE_ROWS: &str = "8605\ngjpqy\n8605 gjpqy\ngjpqy 8605";
const EDGE_SELECTIONS: [((usize, usize), (usize, usize)); 6] = [
    ((0, 0), (3, 10)),
    ((0, 0), (1, 5)),
    ((1, 0), (3, 10)),
    ((1, 0), (2, 10)),
    ((2, 0), (2, 10)),
    ((3, 0), (3, 10)),
];

/// A selection over rows keeps every selected glyph's edge on the highlight, left, right, above
/// and below, with "8605" and "gjpqy" at the rows' starts and ends, Tahoma and DejaVu Sans at 11
/// and 13 px, scale 1 and 2.
#[test]
fn a_selection_over_rows_keeps_its_glyphs_edges_on_the_highlight() {
    crate::pins::fonts();
    let cs = &Theme::default().colors;
    let (w, h) = (150, 90);
    let lines: Vec<&str> = EDGE_ROWS.split('\n').collect();
    let mut faults = Vec::new();
    for family in ["Tahoma", "DejaVu Sans"] {
        let text = TextRenderer::load(&FontConfig {
            family: vec![family.into()],
            antialias: false,
            ..FontConfig::default()
        });
        for px in [11, 13] {
            let m = Metrics {
                bars_always: false,
                ..Metrics::for_font(&text, px)
            };
            let lay = layout(&text, &m, &e(EDGE_ROWS), Rect::new(0, 0, w, h), 1);
            let inner = lay.inner;
            let x0 = inner.x + 1;
            let room = Rect::new(inner.x, inner.y + 1, inner.width - 1, inner.height - 2);
            for s in [1, 2] {
                let paint = |sel: Option<(Pos, Pos)>| {
                    let mut t = e(EDGE_ROWS);
                    if let Some((a, b)) = sel {
                        t.place(a, false);
                        t.place(b, true);
                    }
                    let mut c = Canvas::with_scale(w as u32, h as u32, s);
                    let paint = classic_paint(cs, 1);
                    let v = View::default();
                    draw(&mut c, &text, &paint, &m, &t, v, &lay, Caret::Hidden, None);
                    c.pixels
                };
                let plain = paint(None);
                for ((al, ac), (bl, bc)) in EDGE_SELECTIONS {
                    let selected = paint(Some((p(al, ac), p(bl, bc))));
                    let rows: Vec<crate::paint::SelectedRow> = (lines.iter().enumerate())
                        .map(|(li, line)| {
                            let a = if li == al { ac } else { 0 };
                            let b = if li == bl {
                                bc.min(line.len())
                            } else {
                                line.len()
                            };
                            let sel = if (al..=bl).contains(&li) { a..b } else { 0..0 };
                            let y = inner.y + 1 + li as i32 * m.line_h;
                            let width = text.measure(line, false, px).max(inner.right() - x0);
                            (*line, sel, x0, Rect::new(x0 - 1, y, width + 1, m.line_h))
                        })
                        .collect();
                    let lost = crate::paint::lost_selected_ink(
                        &text,
                        px,
                        (w, h),
                        &rows,
                        room,
                        s,
                        (&plain, &selected),
                        cs.hilight_text,
                    );
                    if !lost.is_empty() {
                        faults.push(format!(
                            "{family} {px} px @{s} {al}:{ac}..{bl}:{bc}: {} px, first {:?}",
                            lost.len(),
                            lost[0]
                        ));
                    }
                }
            }
        }
    }
    assert!(
        faults.is_empty(),
        "{} faults\n{}",
        faults.len(),
        faults.join("\n")
    );
}

/// Rows selected from their start share the inset column left of the text as their left edge,
/// whether or not their first glyph inks its pen's column.
#[test]
fn rows_selected_from_their_start_share_one_left_edge() {
    crate::pins::fonts();
    let text = TextRenderer::load(&FontConfig::default());
    let cs = &Theme::default().colors;
    let m = Metrics::default();
    let mut t = e("8605\nTo\n gjpqy");
    t.place(p(0, 0), false);
    t.place(p(2, 6), true);
    let lay = layout(&text, &m, &t, Rect::new(0, 0, 120, 60), 1);
    let mut c = Canvas::new(120, 60);
    let v = View::default();
    draw(
        &mut c,
        &text,
        &classic_paint(cs, 1),
        &m,
        &t,
        v,
        &lay,
        Caret::Hidden,
        None,
    );
    let x0 = lay.inner.x + 1;
    for row in 0..3 {
        let y = lay.inner.y + 1 + row * m.line_h + m.line_h / 2;
        assert_eq!(
            c.get(x0 - 1, y),
            cs.hilight.argb(),
            "row {row} starts in the inset"
        );
        assert_ne!(
            c.get(x0 - 2, y),
            cs.hilight.argb(),
            "row {row}: the frame stays"
        );
    }
}

/// Without the focus the box hides its selection and keeps its caret where it was; the focus
/// coming back shows the selection again.
#[test]
fn an_unfocused_box_hides_its_selection() {
    crate::pins::fonts();
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let m = Metrics::default();
    let mut t = e("one\ntwo");
    t.place(p(0, 1), false);
    t.place(p(1, 2), true);
    let lay = layout(&text, &m, &t, Rect::new(0, 0, 120, 60), 1);
    let paint = classic_paint(&theme.colors, 1);
    let draw_as = |t: &MultiEdit, caret| {
        let mut c = Canvas::new(120, 60);
        draw(
            &mut c,
            &text,
            &paint,
            &m,
            t,
            View::default(),
            &lay,
            caret,
            None,
        );
        c
    };
    let hl = theme.colors.hilight.argb();
    let lit = |c: &Canvas| (0..60).any(|y| (0..120).any(|x| c.get(x, y) == hl));
    assert!(
        lit(&draw_as(&t, Caret::Hidden)),
        "focused, the selection shows"
    );
    let away = draw_as(&t, Caret::Unfocused);
    assert!(!lit(&away), "unfocused, it is hidden");
    let mut collapsed = t.clone();
    collapsed.place(t.caret, false);
    assert!(
        away == draw_as(&collapsed, Caret::Unfocused),
        "drawn as the caret alone"
    );
}
