//! Pages to print: a text cut into pages of wrapped rows, each page's rows drawn black on white
//! with its header and footer, and a picture's place on a page.
//!
//! A header or footer is one row in the text's font, centred top to bottom in its margin and
//! kept inside it, from the left margin to the right one. Its codes, which take either case:
//! `&f` the file name, `&p` the page number, `&d` the date, `&t` the time, `&&` an ampersand;
//! `&l`, `&c` and `&r` put what follows at the row's left, centre or right. Text before any of
//! them is centred, and an unknown code prints as typed.

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

const PAPER: Color = Color::rgb(255, 255, 255);
const INK: Color = Color::rgb(0, 0, 0);

/// One page of [`text_pages`] on a white canvas `size` px, its rows from the top left.
pub fn draw_text_page(
    text: &TextRenderer,
    m: &Metrics,
    lines: &[String],
    rows: &[Row],
    size: (i32, i32),
) -> Canvas {
    let mut c = Canvas::new(size.0.max(1) as u32, size.1.max(1) as u32);
    c.fill_rect(c.bounds(), PAPER);
    draw_rows(&mut c, text, m, lines, rows, 0, size.0);
    c
}

fn draw_rows(
    c: &mut Canvas,
    text: &TextRenderer,
    m: &Metrics,
    lines: &[String],
    rows: &[Row],
    top: i32,
    width: i32,
) {
    for (i, row) in rows.iter().enumerate() {
        let Some(line) = lines.get(row.line) else {
            continue;
        };
        let shown = multiedit::expand(&line[row.start..row.end], m.tab_cols);
        let r = Rect::new(0, top + i as i32 * m.line_h, width, m.line_h);
        text.draw(c, r, &shown, m.bold, INK, Align::Left, m.font_px);
    }
}

/// What a header or footer's codes stand for on one page.
#[derive(Clone, Copy, Debug)]
pub struct Stamp<'a> {
    pub file: &'a str,
    /// Counted from 1 in the whole document, whatever range prints.
    pub page: u32,
    pub date: &'a str,
    pub time: &'a str,
}

/// A header or footer row: the text at its left, centre and right.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Band {
    pub left: String,
    pub centre: String,
    pub right: String,
}

impl Band {
    pub fn is_empty(&self) -> bool {
        self.left.is_empty() && self.centre.is_empty() && self.right.is_empty()
    }
}

/// `codes` (a header or footer as Page Setup holds it) on the page `s` describes.
pub fn band(codes: &str, s: &Stamp) -> Band {
    let mut b = Band::default();
    let mut at = Align::Center;
    let mut chars = codes.chars();
    while let Some(ch) = chars.next() {
        let part = match at {
            Align::Left => &mut b.left,
            Align::Center => &mut b.centre,
            Align::Right => &mut b.right,
        };
        if ch != '&' {
            part.push(ch);
            continue;
        }
        let Some(code) = chars.next() else {
            part.push('&');
            break;
        };
        match code.to_ascii_lowercase() {
            'f' => part.push_str(s.file),
            'p' => part.push_str(&s.page.to_string()),
            'd' => part.push_str(s.date),
            't' => part.push_str(s.time),
            '&' => part.push('&'),
            'l' => at = Align::Left,
            'c' => at = Align::Center,
            'r' => at = Align::Right,
            _ => {
                part.push('&');
                part.push(code);
            }
        }
    }
    b
}

/// A text page with its header and footer: a page `height` px tall whose margins above and below
/// the text are `top` and `bottom` px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: i32,
    pub height: i32,
    pub top: i32,
    pub bottom: i32,
}

impl Frame {
    /// The header's row and the margin it is kept in, rows `line_h` tall.
    pub fn header(&self, line_h: i32) -> (Rect, Rect) {
        let margin = Rect::new(0, 0, self.width, self.top.max(0));
        (centred_row(margin, line_h), margin)
    }

    /// The footer's row and the margin it is kept in.
    pub fn footer(&self, line_h: i32) -> (Rect, Rect) {
        let bottom = self.bottom.clamp(0, self.height);
        let margin = Rect::new(0, self.height - bottom, self.width, bottom);
        (centred_row(margin, line_h), margin)
    }
}

fn centred_row(margin: Rect, line_h: i32) -> Rect {
    Rect::new(
        margin.x,
        margin.y + (margin.height - line_h) / 2,
        margin.width,
        line_h,
    )
}

/// One page of [`text_pages`] on a white canvas the frame's size: its rows below the top margin,
/// `header` in that margin and `footer` in the bottom one.
pub fn draw_framed_page(
    text: &TextRenderer,
    m: &Metrics,
    lines: &[String],
    rows: &[Row],
    frame: Frame,
    header: &Band,
    footer: &Band,
) -> Canvas {
    let mut c = Canvas::new(frame.width.max(1) as u32, frame.height.max(1) as u32);
    c.fill_rect(c.bounds(), PAPER);
    draw_rows(&mut c, text, m, lines, rows, frame.top, frame.width);
    for (band, (row, margin)) in [
        (header, frame.header(m.line_h)),
        (footer, frame.footer(m.line_h)),
    ] {
        for (part, align) in [
            (&band.left, Align::Left),
            (&band.centre, Align::Center),
            (&band.right, Align::Right),
        ] {
            if !part.is_empty() {
                text.draw_clipped(&mut c, row, margin, part, m.bold, INK, align, m.font_px);
            }
        }
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

    fn stamp(page: u32) -> Stamp<'static> {
        Stamp {
            file: "notes.txt",
            page,
            date: "11/27/2026",
            time: "9:05 AM",
        }
    }

    fn centred(s: &str) -> Band {
        Band {
            centre: s.into(),
            ..Band::default()
        }
    }

    #[test]
    fn header_codes_put_in_the_file_page_date_and_time_in_either_case() {
        assert_eq!(band("&f", &stamp(3)), centred("notes.txt"));
        assert_eq!(band("Page &P", &stamp(12)), centred("Page 12"));
        assert_eq!(band("&d &T", &stamp(1)), centred("11/27/2026 9:05 AM"));
        assert_eq!(band("", &stamp(1)), Band::default());
        assert!(band("", &stamp(1)).is_empty());
    }

    #[test]
    fn align_codes_send_what_follows_left_centre_or_right() {
        assert_eq!(
            band("&lLeft &f&cMid&rPage &p", &stamp(2)),
            Band {
                left: "Left notes.txt".into(),
                centre: "Mid".into(),
                right: "Page 2".into(),
            }
        );
        assert_eq!(
            band("a&rb&lc", &stamp(1)),
            Band {
                left: "c".into(),
                centre: "a".into(),
                right: "b".into(),
            }
        );
    }

    #[test]
    fn an_ampersand_pair_an_unknown_code_and_a_last_ampersand_print_as_typed() {
        assert_eq!(band("R&&D &x 50&", &stamp(1)), centred("R&D &x 50&"));
    }

    #[test]
    fn header_and_footer_rows_sit_centred_in_their_margins() {
        let frame = Frame {
            width: 200,
            height: 1000,
            top: 100,
            bottom: 60,
        };
        assert_eq!(
            frame.header(20),
            (Rect::new(0, 40, 200, 20), Rect::new(0, 0, 200, 100))
        );
        assert_eq!(
            frame.footer(20),
            (Rect::new(0, 960, 200, 20), Rect::new(0, 940, 200, 60))
        );
        let tight = Frame { top: 10, ..frame };
        let (row, margin) = tight.header(20);
        assert_eq!(
            row.y, -5,
            "a row taller than its margin overhangs both ways"
        );
        assert_eq!(
            margin,
            Rect::new(0, 0, 200, 10),
            "and is drawn only inside it"
        );
    }

    #[test]
    fn a_framed_page_draws_its_header_and_footer_in_the_margins_only() {
        crate::pins::fonts();
        let text = TextRenderer::load(&crate::theme::FontConfig {
            size_px: 10,
            ..crate::theme::FontConfig::default()
        });
        let m = metrics();
        let frame = Frame {
            width: 200,
            height: 300,
            top: 40,
            bottom: 40,
        };
        let lines = vec!["body".to_string()];
        let pages = text_pages(&text, &m, &lines, 200, 220);
        let inked = |c: &Canvas, from: i32, to: i32| {
            (from..to).any(|y| (0..200).any(|x| c.get(x, y) != PAPER.argb()))
        };
        let blank = draw_framed_page(
            &text,
            &m,
            &lines,
            &pages[0],
            frame,
            &Band::default(),
            &Band::default(),
        );
        assert!(!inked(&blank, 0, 40) && !inked(&blank, 260, 300));
        assert!(
            inked(&blank, 40, 50),
            "the text starts below the top margin"
        );
        let c = draw_framed_page(
            &text,
            &m,
            &lines,
            &pages[0],
            frame,
            &centred("Head"),
            &centred("Foot"),
        );
        assert!(
            inked(&c, 15, 25),
            "the header is in the middle of the top margin"
        );
        assert!(
            inked(&c, 275, 285),
            "the footer in the middle of the bottom one"
        );
        assert!(!inked(&c, 50, 260), "neither runs into the text's room");
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
