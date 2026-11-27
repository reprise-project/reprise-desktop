//! Pages to print: a text cut into pages of wrapped rows, each page's rows drawn black on white,
//! and a picture's place on a page.

use crate::controls::multiedit::{self, Metrics, Row};
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Color;

/// The rows of `lines` wrapped at `width` px in `m`'s font, `m.line_h` apart, cut into pages
/// `height` px tall: a page holds at least one row, and an empty text is one empty page.
pub fn text_pages(
    text: &TextRenderer,
    m: &Metrics,
    lines: &[String],
    width: i32,
    height: i32,
) -> Vec<Vec<Row>> {
    let per_page = (height / m.line_h.max(1)).max(1) as usize;
    let mut rows = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let breaks = multiedit::breaks(text, m, line, width.max(1));
        let starts = std::iter::once(0).chain(breaks.iter().copied());
        let ends = breaks.iter().copied().chain(std::iter::once(line.len()));
        let n = breaks.len();
        for (r, (start, end)) in starts.zip(ends).enumerate() {
            rows.push(Row {
                line: i,
                start,
                end,
                last: r == n,
            });
        }
    }
    let mut pages: Vec<Vec<Row>> = rows.chunks(per_page).map(<[Row]>::to_vec).collect();
    if pages.is_empty() {
        pages.push(Vec::new());
    }
    pages
}

/// One page of [`text_pages`] on a white canvas `size` px, its rows from the top left.
pub fn draw_text_page(
    text: &TextRenderer,
    m: &Metrics,
    lines: &[String],
    rows: &[Row],
    size: (i32, i32),
) -> Canvas {
    let mut c = Canvas::new(size.0.max(1) as u32, size.1.max(1) as u32);
    c.fill_rect(c.bounds(), Color::rgb(255, 255, 255));
    for (i, row) in rows.iter().enumerate() {
        let Some(line) = lines.get(row.line) else {
            continue;
        };
        let shown = multiedit::expand(&line[row.start..row.end], m.tab_cols);
        let r = Rect::new(0, i as i32 * m.line_h, size.0, m.line_h);
        text.draw(
            &mut c,
            r,
            &shown,
            m.bold,
            Color::rgb(0, 0, 0),
            Align::Left,
            m.font_px,
        );
    }
    c
}

/// Where a picture `w` by `h` goes in `room`: as large as fits with its shape kept, centred.
pub fn fit(w: i32, h: i32, room: Rect) -> Rect {
    let (w, h) = (w.max(1) as i64, h.max(1) as i64);
    let (rw, rh) = (room.width as i64, room.height as i64);
    let (fw, fh) = if rw * h <= rh * w {
        (rw, rw * h / w)
    } else {
        (rh * w / h, rh)
    };
    Rect::new(
        room.x + ((rw - fw) / 2) as i32,
        room.y + ((rh - fh) / 2) as i32,
        fw as i32,
        fh as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> Metrics {
        Metrics {
            font_px: 10,
            line_h: 10,
            tab_cols: 4,
            ..Metrics::default()
        }
    }

    fn stub() -> TextRenderer {
        TextRenderer::stub(10, |_| 10)
    }

    #[test]
    fn long_lines_wrap_at_the_width_and_rows_fill_pages_of_the_height() {
        let lines = vec!["aaaa bbbb cccc".to_string(), String::new(), "d".into()];
        let pages = text_pages(&stub(), &metrics(), &lines, 100, 25);
        let rows: Vec<Vec<(usize, usize, usize)>> = pages
            .iter()
            .map(|p| p.iter().map(|r| (r.line, r.start, r.end)).collect())
            .collect();
        assert_eq!(
            rows,
            vec![vec![(0, 0, 10), (0, 10, 14)], vec![(1, 0, 0), (2, 0, 1)]],
            "two rows of 10 px a 25 px page; the empty line is a row"
        );
    }

    #[test]
    fn an_empty_text_is_one_empty_page_and_a_short_page_holds_a_row() {
        assert_eq!(text_pages(&stub(), &metrics(), &[], 100, 25), vec![vec![]]);
        let lines = vec!["a".to_string(), "b".into()];
        assert_eq!(text_pages(&stub(), &metrics(), &lines, 100, 4).len(), 2);
    }

    #[test]
    fn a_picture_fills_the_room_one_way_and_is_centred_the_other() {
        let room = Rect::new(10, 20, 200, 100);
        assert_eq!(fit(400, 100, room), Rect::new(10, 45, 200, 50));
        assert_eq!(fit(50, 100, room), Rect::new(85, 20, 50, 100));
        assert_eq!(
            fit(1, 1, room),
            Rect::new(60, 20, 100, 100),
            "small ones grow"
        );
    }
}
