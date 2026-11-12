use super::*;
use crate::look::classic_paint;
use crate::theme::{FontConfig, Theme};

fn font() -> TextRenderer {
    TextRenderer::load(&FontConfig::default())
}

fn wrapping() -> Metrics {
    Metrics {
        wrap: true,
        bars_always: false,
        flat: true,
        ..Default::default()
    }
}

fn p(line: usize, col: usize) -> Pos {
    Pos { line, col }
}

/// A box whose rows wrap at exactly the width of `fits`.
fn box_for(text: &TextRenderer, m: &Metrics, fits: &str, height: i32) -> Rect {
    Rect::new(0, 0, text.measure(fits, false, m.font_px) + 3, height)
}

fn press(t: &mut MultiEdit, text: &TextRenderer, m: &Metrics, key: MultiKey) {
    t.key(key, text, m);
}

const DOWN: MultiKey = MultiKey::Down { select: false };
const UP: MultiKey = MultiKey::Up { select: false };
const HOME: MultiKey = MultiKey::Edit(EditKey::Home { select: false });
const END: MultiKey = MultiKey::Edit(EditKey::End { select: false });
const RIGHT: MultiKey = MultiKey::Edit(EditKey::Right {
    select: false,
    word: false,
});

#[test]
fn a_row_breaks_before_the_word_that_crosses_the_edge() {
    let (text, m) = (font(), wrapping());
    let w = text.measure("xx xx", false, m.font_px);
    assert_eq!(breaks(&text, &m, "xx xx xx", w), [6]);
    assert_eq!(breaks(&text, &m, "xx xx xx", w - 1), [3, 6]);
    assert_eq!(breaks(&text, &m, "xx xx", w), Vec::<usize>::new());
    assert_eq!(breaks(&text, &m, "", w), Vec::<usize>::new());
}

#[test]
fn bold_text_breaks_where_its_bold_words_cross_the_edge() {
    let text = font();
    let m = Metrics {
        font_px: 20,
        bold: true,
        ..wrapping()
    };
    let regular = text.measure("xx xx", false, m.font_px);
    let bold = text.measure("xx xx", true, m.font_px);
    assert!(regular < bold, "bold is wider: {regular} < {bold}");
    assert_eq!(breaks(&text, &m, "xx xx xx", bold), [6]);
    assert_eq!(breaks(&text, &m, "xx xx xx", regular), [3, 6]);
}

#[test]
fn a_word_wider_than_a_row_breaks_inside_it() {
    let (text, m) = (font(), wrapping());
    let w = text.measure("xxxxx", false, m.font_px);
    assert_eq!(breaks(&text, &m, "xxxxxxxxxxxx", w), [5, 10]);
    assert_eq!(
        breaks(&text, &m, "xx xxxxxxxxxxxx", w),
        [3, 8, 13],
        "the long word starts its own row first"
    );
}

#[test]
fn spaces_ending_a_row_hang_past_the_edge() {
    let (text, m) = (font(), wrapping());
    let w = text.measure("xx", false, m.font_px);
    assert_eq!(breaks(&text, &m, "xx        xx", w), [10]);
    assert_eq!(breaks(&text, &m, "xx   ", w), Vec::<usize>::new());
}

#[test]
fn a_cluster_never_splits() {
    let (text, m) = (font(), wrapping());
    assert_eq!(breaks(&text, &m, "e\u{301}e\u{301}e\u{301}", 1), [3, 6]);
}

#[test]
fn a_narrower_box_rewraps_and_an_edit_rewraps_its_line() {
    let (text, m) = (font(), wrapping());
    let mut t = MultiEdit::new("xx xx xx xx\nyy");
    let wide = layout(&text, &m, &t, box_for(&text, &m, "xx xx", 200), 1);
    assert_eq!((wide.total, wide.hscroll), (3, None));
    let narrow = layout(&text, &m, &t, box_for(&text, &m, "xx", 200), 1);
    assert_eq!(narrow.total, 5);
    let wide = layout(&text, &m, &t, box_for(&text, &m, "xx xx", 200), 1);
    assert_eq!(wide.total, 3, "widening joins the rows again");
    t.place(p(1, 2), false);
    t.paste(" yy yy");
    let lay = layout(&text, &m, &t, box_for(&text, &m, "xx xx", 200), 1);
    assert_eq!(lay.total, 4);
    assert_eq!(
        t.row(&text, &m, lay.wrap, 3),
        Row {
            line: 1,
            start: 6,
            end: 8,
            last: true
        }
    );
}

#[test]
fn rows_that_outgrow_the_box_bring_the_bar_and_rewrap_beside_it() {
    let (text, m) = (font(), wrapping());
    let t = MultiEdit::new("xx xx xx xx xx xx xx");
    let r = box_for(&text, &m, "xx xx xx", m.line_h * 2);
    let lay = layout(&text, &m, &t, r, 1);
    let bar = lay.vscroll.expect("three rows outgrow two");
    assert_eq!(lay.inner.width, r.width - bar.width);
    assert_eq!(lay.wrap, Some(lay.inner.width - 3));
    assert!(lay.total > 2);
    assert_eq!(lay.hscroll, None);
}

#[test]
fn up_down_home_and_end_go_by_rows() {
    let (text, m) = (font(), wrapping());
    let mut t = MultiEdit::new("xx xx xx xx\nxx");
    layout(&text, &m, &t, box_for(&text, &m, "xx xx", 200), 1);
    t.place(p(0, 1), false);
    press(&mut t, &text, &m, DOWN);
    assert_eq!(t.caret(), p(0, 7), "the next row of the same line");
    press(&mut t, &text, &m, DOWN);
    assert_eq!(t.caret(), p(1, 1));
    press(&mut t, &text, &m, UP);
    press(&mut t, &text, &m, UP);
    assert_eq!(t.caret(), p(0, 1), "the goal x comes back");
    press(&mut t, &text, &m, END);
    assert_eq!(
        t.caret(),
        p(0, 5),
        "a soft row ends before its hanging space"
    );
    t.place(p(0, 8), false);
    press(&mut t, &text, &m, HOME);
    assert_eq!(t.caret(), p(0, 6));
    press(&mut t, &text, &m, END);
    assert_eq!(
        t.caret(),
        p(0, 11),
        "the line's last row ends at the line's end"
    );
    press(&mut t, &text, &m, MultiKey::Down { select: true });
    assert_eq!(t.selected_text(), "\nxx");
}

#[test]
fn right_at_a_rows_end_moves_onto_the_next_row() {
    let (text, m) = (font(), wrapping());
    let mut t = MultiEdit::new("xx xx xx");
    let lay = layout(&text, &m, &t, box_for(&text, &m, "xx xx", 200), 1);
    t.place(p(0, 5), false);
    press(&mut t, &text, &m, RIGHT);
    assert_eq!(t.caret(), p(0, 6));
    assert_eq!(t.row_of(&text, &m, lay.wrap, t.caret()), 1);
    assert_eq!(x_of(&text, &m, &t, &lay, t.caret()), 0);
}

#[test]
fn page_down_steps_rows() {
    let (text, m) = (font(), wrapping());
    let mut t = MultiEdit::new("xx xx xx xx xx xx xx xx");
    layout(&text, &m, &t, box_for(&text, &m, "xx", 200), 1);
    press(
        &mut t,
        &text,
        &m,
        MultiKey::PageDown {
            select: false,
            rows: 3,
        },
    );
    assert_eq!(t.caret(), p(0, 9));
}

#[test]
fn a_click_lands_on_the_row_under_it() {
    let (text, m) = (font(), wrapping());
    let t = MultiEdit::new("xx xx xx xx\nab");
    let lay = layout(&text, &m, &t, box_for(&text, &m, "xx xx", 200), 1);
    let x2 = 1 + text.measure("xx", false, m.font_px);
    let row_y = |r: i32| 1 + r * m.line_h + m.line_h / 2;
    let at = |x, y| pos_at(&text, &m, &t, &lay, View::default(), x, y, 1);
    assert_eq!(at(x2, row_y(1)), p(0, 8));
    assert_eq!(
        at(lay.inner.right() - 1, row_y(0)),
        p(0, 5),
        "past a soft row's text"
    );
    assert_eq!(at(1, row_y(2)), p(1, 0));
    assert_eq!(at(1, row_y(9)), p(1, 0), "below the text is its last row");
}

#[test]
fn the_view_follows_the_caret_by_rows_and_never_scrolls_sideways() {
    let (text, m) = (font(), wrapping());
    let mut t = MultiEdit::new("xx xx xx xx xx xx xx xx");
    let mut r = box_for(&text, &m, "xx", m.line_h * 3);
    r.width += m.bar;
    let lay = layout(&text, &m, &t, r, 1);
    assert_eq!(
        (lay.rows, lay.wrap),
        (3, Some(text.measure("xx", false, m.font_px)))
    );
    let mut v = View::default();
    t.place(t.end(), false);
    v.follow(&text, &m, &t, &lay, 1);
    assert_eq!((v.top, v.left), (lay.total - 3, 0));
    assert!(!v.scroll_lines(1, &lay), "held at the last row");
    assert!(v.scroll_lines(-100, &lay));
    assert_eq!(v.top, 0);
}

#[test]
fn soft_rows_draw_as_hard_lines_broken_at_the_same_bytes() {
    let (text, m) = (font(), wrapping());
    let theme = Theme::default();
    let paint = classic_paint(&theme.colors, 1);
    let soft = MultiEdit::new("one two three");
    let r = box_for(&text, &m, "one two", 80);
    let lay = layout(&text, &m, &soft, r, 1);
    let hard = MultiEdit::new("one two \nthree");
    let unwrapped = Metrics { wrap: false, ..m };
    let hard_lay = layout(&text, &unwrapped, &hard, r, 1);
    let mut a = Canvas::new(r.width as u32, r.height as u32);
    let mut b = Canvas::new(r.width as u32, r.height as u32);
    let v = View::default();
    draw(
        &mut a,
        &text,
        &paint,
        &m,
        &soft,
        v,
        &lay,
        Caret::Unfocused,
        None,
    );
    draw(
        &mut b,
        &text,
        &paint,
        &unwrapped,
        &hard,
        v,
        &hard_lay,
        Caret::Unfocused,
        None,
    );
    assert_eq!(lay.total, 2);
    for y in 0..1 + 2 * m.line_h {
        for x in 0..r.width {
            assert_eq!(a.get(x, y), b.get(x, y), "at {x},{y}");
        }
    }
}

#[test]
fn a_selection_across_a_soft_break_lights_both_rows() {
    let (text, m) = (font(), wrapping());
    let theme = Theme::default();
    let paint = classic_paint(&theme.colors, 1);
    let mut t = MultiEdit::new("xx xx xx");
    let r = box_for(&text, &m, "xx xx", 40);
    let lay = layout(&text, &m, &t, r, 1);
    t.place(p(0, 4), false);
    t.place(p(0, 7), true);
    let mut c = Canvas::new(r.width as u32, r.height as u32);
    let v = View::default();
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
    let lit = theme.colors.hilight.argb();
    let x = |s: &str| 1 + text.measure(s, false, m.font_px);
    let mid = |row: i32| 1 + row * m.line_h + m.line_h / 2;
    let lit_in = |row: i32| -> Vec<i32> {
        (0..r.width)
            .filter(|&px| c.get(px, mid(row)) == lit)
            .collect()
    };
    let row0 = lit_in(0);
    assert_eq!(row0.first(), Some(&x("xx x")), "from the anchor");
    assert_eq!(
        row0.last(),
        Some(&(x("xx xx ").min(r.width - 1) - 1)),
        "through the hanging space"
    );
    let row1 = lit_in(1);
    assert_eq!(row1.first(), Some(&1));
    assert_eq!(row1.last(), Some(&(x("x") - 1)), "no line break is lit");
}

#[test]
fn the_input_method_caret_sits_on_the_carets_row() {
    let (text, m) = (font(), wrapping());
    let theme = Theme::default();
    let mut t = MultiEdit::new("xx xx xx");
    let r = box_for(&text, &m, "xx xx", 40);
    let lay = layout(&text, &m, &t, r, 1);
    t.place(p(0, 7), false);
    let mut c = Canvas::new(r.width as u32, r.height as u32);
    draw(
        &mut c,
        &text,
        &classic_paint(&theme.colors, 1),
        &m,
        &t,
        View::default(),
        &lay,
        Caret::new(true, true),
        None,
    );
    let spot = c.take_spot().expect("focused");
    assert_eq!(
        (spot.caret.x, spot.caret.y),
        (1 + text.measure("x", false, m.font_px), 1 + m.line_h)
    );
    assert_eq!(spot.surrounding.unwrap().text, "xx xx xx", "the whole line");
}

#[test]
fn access_runs_are_rows_and_the_value_is_the_text() {
    use crate::access::{ROOT, Tree};
    let (text, m) = (font(), wrapping());
    let mut t = MultiEdit::new("xx xx xx\nab");
    let lay = layout(&text, &m, &t, box_for(&text, &m, "xx xx", 100), 1);
    t.place(p(0, 7), false);
    let mut tree = Tree::window("T", (100, 100));
    let v = View::default();
    let id = access(&t, v, &lay, &m, &text, &mut tree, ROOT, "edit");
    let node = tree.get(id).unwrap();
    assert_eq!(node.value(), Some("xx xx xx\nab"));
    let runs = node.children().to_vec();
    let values: Vec<_> = runs.iter().map(|r| tree.get(*r).unwrap().value()).collect();
    assert_eq!(values, [Some("xx xx "), Some("xx\n"), Some("ab")]);
    assert_eq!(tree.get(runs[0]).unwrap().next_on_line(), Some(runs[1]));
    assert_eq!(tree.get(runs[1]).unwrap().next_on_line(), None);
    let second = tree.get(runs[1]).unwrap().bounds().unwrap();
    assert_eq!(second.y0, f64::from(1 + m.line_h));
    let sel = node.text_selection().unwrap();
    assert_eq!((sel.focus.node, sel.focus.character_index), (runs[1], 1));
}

#[test]
fn the_context_menu_opens_under_the_carets_row() {
    let (text, m) = (font(), wrapping());
    let mut t = MultiEdit::new("xx xx xx");
    let lay = layout(&text, &m, &t, box_for(&text, &m, "xx xx", 100), 1);
    t.place(p(0, 8), false);
    assert_eq!(
        caret_point(&text, &m, &t, &lay, View::default()),
        (1 + text.measure("xx", false, m.font_px), 1 + 2 * m.line_h)
    );
}
