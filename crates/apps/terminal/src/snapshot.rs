//! A `Term`'s visible cells as a `termgrid::Snapshot`, each row marked dirty when it changed.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::Flags as TermFlags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color as TermColor, CursorShape as TermCursor, NamedColor};
use reprisekit::controls::termgrid::{Cell, CursorShape, Flags, Rgb, Snapshot};
use reprisekit::theme::Color;
use unicode_width::UnicodeWidthChar;

use crate::find::Marks;
use crate::schemes::Scheme;

/// Bring `out` up to `term`'s viewport and cursor in `scheme`'s colours, marking each changed row
/// dirty (every row on a new size or default colours); [`move_cursor`] dirties the cursor's rows.
#[cfg(test)]
pub fn fill<T: EventListener>(term: &Term<T>, scheme: &Scheme, out: &mut Snapshot) {
    fill_marked(term, scheme, &Marks::default(), out);
}

/// [`fill`] with Find's `marks` flagged on the cells they cover.
pub fn fill_marked<T: EventListener>(
    term: &Term<T>,
    scheme: &Scheme,
    marks: &Marks,
    out: &mut Snapshot,
) {
    let content = term.renderable_content();
    let colors = content.colors;
    let fg = colors[NamedColor::Foreground].map_or(scheme.fg, rgb);
    let bg = colors[NamedColor::Background].map_or(scheme.bg, rgb);
    let (cols, rows) = (term.columns(), term.screen_lines());
    if (out.cols, out.rows) != (cols, rows) {
        *out = Snapshot::new(cols, rows, fg, bg);
    }
    if out.bg != bg || out.accents != scheme.accents {
        out.bg = bg;
        out.accents = scheme.accents;
        out.dirty.fill(true);
    }
    let offset = content.display_offset as i32;
    let selection = content.selection;
    for indexed in content.display_iter {
        let (Ok(row), col) = (
            usize::try_from(indexed.point.line.0 + offset),
            indexed.point.column.0,
        ) else {
            continue;
        };
        if row >= rows || col >= cols {
            continue;
        }
        let mut cell = convert(indexed.cell, colors, scheme, fg, bg);
        if selection.is_some_and(|s| s.contains(indexed.point)) {
            cell.flags |= Flags::SELECTED;
        }
        if marks.is_match(indexed.point) {
            cell.flags |= Flags::MATCH;
            if marks.is_current(indexed.point) {
                cell.flags |= Flags::MATCH_CURRENT;
            }
        }
        let slot = &mut out.cells[row * cols + col];
        if *slot != cell {
            *slot = cell;
            out.dirty[row] = true;
        }
    }
    let cursor = content.cursor;
    let row = usize::try_from(cursor.point.line.0 + offset)
        .ok()
        .filter(|&r| r < rows);
    out.cursor = match row {
        Some(row) => (row, cursor.point.column.0, shape(cursor.shape)),
        None => (0, 0, CursorShape::Hidden),
    };
}

/// Show `to` as `out`'s cursor where `from` was drawn, dirtying the rows of both when they differ.
pub fn move_cursor(
    out: &mut Snapshot,
    from: (usize, usize, CursorShape),
    to: (usize, usize, CursorShape),
) {
    if from != to {
        for r in [from.0, to.0] {
            if let Some(d) = out.dirty.get_mut(r) {
                *d = true;
            }
        }
    }
    out.cursor = to;
}

/// Lay an input method's `text` over the cells from the cursor, underlined, and dirty its row.
/// Text past the row's end is cut.
pub fn overlay_preedit(out: &mut Snapshot, text: &str) {
    let (row, mut col, _) = out.cursor;
    if row >= out.rows {
        return;
    }
    let (fg, bg) = (out.accents.cursor, out.bg);
    for ch in text.chars() {
        let wide = ch.width() == Some(2);
        if col + usize::from(wide) >= out.cols {
            break;
        }
        let mut cell = Cell::blank(fg, bg);
        cell.ch = ch;
        cell.flags = Flags::UNDERLINE;
        if wide {
            cell.flags |= Flags::WIDE;
            let mut spacer = Cell::blank(fg, bg);
            spacer.flags = Flags::WIDE_SPACER | Flags::UNDERLINE;
            out.cells[row * out.cols + col + 1] = spacer;
        }
        out.cells[row * out.cols + col] = cell;
        col += 1 + usize::from(wide);
    }
    out.dirty[row] = true;
}

fn shape(s: TermCursor) -> CursorShape {
    match s {
        TermCursor::Block => CursorShape::Block,
        TermCursor::Underline => CursorShape::Underline,
        TermCursor::Beam => CursorShape::Bar,
        TermCursor::HollowBlock => CursorShape::HollowBlock,
        TermCursor::Hidden => CursorShape::Hidden,
    }
}

fn convert(
    cell: &alacritty_terminal::term::cell::Cell,
    colors: &Colors,
    scheme: &Scheme,
    fg: Rgb,
    bg: Rgb,
) -> Cell {
    let f = cell.flags;
    let mut flags = Flags::default();
    for (theirs, ours) in [
        (TermFlags::BOLD, Flags::BOLD),
        (TermFlags::ITALIC, Flags::ITALIC),
        (TermFlags::STRIKEOUT, Flags::STRIKE),
        (TermFlags::INVERSE, Flags::INVERSE),
        (TermFlags::DIM, Flags::DIM),
        (TermFlags::WIDE_CHAR, Flags::WIDE),
        (TermFlags::WIDE_CHAR_SPACER, Flags::WIDE_SPACER),
    ] {
        if f.contains(theirs) {
            flags |= ours;
        }
    }
    if f.intersects(TermFlags::ALL_UNDERLINES) {
        flags |= Flags::UNDERLINE;
    }
    let blank = f.intersects(TermFlags::HIDDEN | TermFlags::LEADING_WIDE_CHAR_SPACER);
    // A tab stop keeps its '\t' in the cell it starts at, for copying.
    Cell {
        ch: if blank || cell.c == '\t' { ' ' } else { cell.c },
        extra: cell
            .zerowidth()
            .filter(|_| !blank)
            .map(|z| z.iter().collect::<String>().into_boxed_str()),
        fg: resolve(cell.fg, colors, scheme, fg, bg),
        bg: resolve(cell.bg, colors, scheme, fg, bg),
        flags,
    }
}

fn rgb(c: alacritty_terminal::vte::ansi::Rgb) -> Rgb {
    Color::rgb(c.r, c.g, c.b)
}

/// `c` in `scheme`, after any colour the program set with an OSC.
fn resolve(c: TermColor, colors: &Colors, scheme: &Scheme, fg: Rgb, bg: Rgb) -> Rgb {
    let index = match c {
        TermColor::Spec(v) => return rgb(v),
        TermColor::Indexed(i) => usize::from(i),
        TermColor::Named(n) => n as usize,
    };
    if let Some(set) = colors[index] {
        return rgb(set);
    }
    match c {
        TermColor::Indexed(i) => indexed(i, scheme),
        TermColor::Named(n) => named(n, scheme, fg, bg),
        TermColor::Spec(_) => unreachable!(),
    }
}

fn named(n: NamedColor, scheme: &Scheme, fg: Rgb, bg: Rgb) -> Rgb {
    let dim = |c: Rgb| Color::rgb(c.r / 3 * 2, c.g / 3 * 2, c.b / 3 * 2);
    match n as usize {
        i @ 0..16 => scheme.ansi[i],
        _ => match n {
            NamedColor::Background => bg,
            NamedColor::Cursor => scheme.accents.cursor,
            NamedColor::DimForeground => dim(fg),
            NamedColor::DimBlack
            | NamedColor::DimRed
            | NamedColor::DimGreen
            | NamedColor::DimYellow
            | NamedColor::DimBlue
            | NamedColor::DimMagenta
            | NamedColor::DimCyan
            | NamedColor::DimWhite => dim(scheme.ansi[n as usize - NamedColor::DimBlack as usize]),
            _ => fg,
        },
    }
}

/// The 256-colour palette: the scheme's 16, a 6x6x6 cube, then 24 greys.
pub fn indexed(i: u8, scheme: &Scheme) -> Rgb {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match i {
        0..16 => scheme.ansi[usize::from(i)],
        16..232 => {
            let v = i - 16;
            Color::rgb(
                LEVELS[usize::from(v / 36)],
                LEVELS[usize::from(v / 6 % 6)],
                LEVELS[usize::from(v % 6)],
            )
        }
        _ => {
            let g = 8 + 10 * (i - 232);
            Color::rgb(g, g, g)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::vte::ansi::Processor;
    use reprisekit::interface::Interface;

    const BLACK: Rgb = Color::rgb(0, 0, 0);

    fn term(cols: usize, rows: usize) -> Term<VoidListener> {
        Term::new(Config::default(), &TermSize::new(cols, rows), VoidListener)
    }

    fn feed(t: &mut Term<VoidListener>, bytes: &[u8]) {
        Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new().advance(t, bytes);
    }

    fn scheme() -> Scheme {
        crate::schemes::scheme("reprise", Interface::Classic)
    }

    fn filled(t: &Term<VoidListener>) -> Snapshot {
        let mut snap = Snapshot::new(0, 0, BLACK, BLACK);
        fill(t, &scheme(), &mut snap);
        snap
    }

    #[test]
    fn text_lands_in_its_cells() {
        let mut t = term(10, 3);
        feed(&mut t, b"hi\r\n  there");
        let snap = filled(&t);
        assert_eq!((snap.cols, snap.rows), (10, 3));
        assert_eq!(snap.row_text(0), "hi");
        assert_eq!(snap.row_text(1), "  there");
        assert_eq!(snap.cursor, (1, 7, CursorShape::Block));
    }

    #[test]
    fn only_changed_rows_are_dirty() {
        let mut t = term(10, 4);
        feed(&mut t, b"one\r\ntwo\r\nthree");
        let mut snap = filled(&t);
        assert!(
            snap.dirty.iter().all(|&d| d),
            "a first fill dirties every row"
        );
        snap.dirty.fill(false);
        fill(&t, &scheme(), &mut snap);
        assert_eq!(snap.dirty, vec![false; 4], "nothing changed");
        feed(&mut t, b"\x1b[2;1HTWO");
        fill(&t, &scheme(), &mut snap);
        assert_eq!(snap.dirty, vec![false, true, false, false]);
        assert_eq!(snap.row_text(1), "TWO");
    }

    #[test]
    fn scrolling_dirties_every_moved_row() {
        let mut t = term(5, 3);
        feed(&mut t, b"a\r\nb\r\nc");
        let mut snap = filled(&t);
        snap.dirty.fill(false);
        feed(&mut t, b"\r\nd");
        fill(&t, &scheme(), &mut snap);
        assert_eq!(snap.dirty, vec![true; 3]);
        assert_eq!(
            (0..3).map(|r| snap.row_text(r)).collect::<Vec<_>>(),
            ["b", "c", "d"]
        );
    }

    #[test]
    fn colours_resolve_through_the_scheme() {
        let mut t = term(4, 1);
        feed(
            &mut t,
            b"\x1b[31ma\x1b[38;2;1;2;3mb\x1b[48;5;196mc\x1b[0;7md",
        );
        let snap = filled(&t);
        let s = scheme();
        let row = snap.row(0);
        assert_eq!(row[0].fg, s.ansi[1]);
        assert_eq!(row[1].fg, Color::rgb(1, 2, 3));
        assert_eq!(row[2].bg, Color::rgb(255, 0, 0));
        assert_eq!((row[3].fg, row[3].bg), (s.fg, s.bg));
        assert!(row[3].flags.contains(Flags::INVERSE));
        assert_eq!(snap.bg, s.bg);
    }

    #[test]
    fn a_tab_stop_shows_blank() {
        let mut t = term(12, 1);
        feed(&mut t, b"a\tb");
        let snap = filled(&t);
        assert_eq!(snap.row_text(0), "a       b");
    }

    #[test]
    fn a_resize_dirties_everything() {
        let mut t = term(5, 2);
        let mut snap = filled(&t);
        snap.dirty.fill(false);
        t.resize(TermSize::new(6, 3));
        fill(&t, &scheme(), &mut snap);
        assert_eq!((snap.cols, snap.rows), (6, 3));
        assert_eq!(snap.dirty, vec![true; 3]);
    }

    #[test]
    fn a_moved_cursor_dirties_both_rows() {
        let mut snap = Snapshot::new(4, 4, BLACK, BLACK);
        snap.dirty.fill(false);
        move_cursor(
            &mut snap,
            (1, 0, CursorShape::Block),
            (1, 0, CursorShape::Block),
        );
        assert_eq!(snap.dirty, vec![false; 4]);
        move_cursor(
            &mut snap,
            (1, 0, CursorShape::Block),
            (3, 2, CursorShape::Block),
        );
        assert_eq!(snap.dirty, vec![false, true, false, true]);
        assert_eq!(snap.cursor, (3, 2, CursorShape::Block));
    }

    #[test]
    fn preedit_lies_over_the_cursor_cells() {
        let mut snap = Snapshot::new(6, 2, BLACK, BLACK);
        snap.dirty.fill(false);
        snap.cursor = (1, 1, CursorShape::Block);
        overlay_preedit(&mut snap, "a日b");
        let row = snap.row(1);
        assert_eq!(snap.row_text(1), " a日b");
        assert!(row[1].flags.contains(Flags::UNDERLINE));
        assert!(row[2].flags.contains(Flags::WIDE));
        assert!(row[3].flags.contains(Flags::WIDE_SPACER));
        assert_eq!(snap.dirty, vec![false, true]);
    }
}
