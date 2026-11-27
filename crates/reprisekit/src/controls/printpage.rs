//! Pages to print: a text cut into pages of wrapped rows, each page's rows drawn black on white
//! with its header and footer, and a picture's place on a page.
//!
//! A header or footer is one row in the text's font, centred top to bottom in its margin, from
//! the left margin to the right one; a margin thinner than the row prints none. Its left, centre
//! and right parts each keep to their own room ([`band_rooms`]), and control characters print as
//! spaces. Its codes, which take either case:
//! `&f` the file name, `&p` the page number, `&d` the date, `&t` the time, `&&` an ampersand;
//! `&l`, `&c` and `&r` put what follows at the row's left, centre or right. Text before any of
//! them is centred, and an unknown code prints as typed.

use crate::controls::multiedit::{self, Metrics, Row};
use crate::controls::pagesetup::{self, Scaling};
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
    let page = c.bounds();
    for (i, row) in rows.iter().enumerate() {
        let Some(line) = lines.get(row.line) else {
            continue;
        };
        let shown = multiedit::expand(&line[row.start..row.end], m.tab_cols);
        let r = Rect::new(0, top + i as i32 * m.line_h, width, m.line_h);
        text.draw_tall(c, r, page, &shown, m.bold, INK, Align::Left, m.font_px);
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
    for part in [&mut b.left, &mut b.centre, &mut b.right] {
        *part = part
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
    }
    b
}

/// The room each part of a band takes in a row `width` px wide, as (x, width), given the parts'
/// widths: the centre part centred, the left part up to `gap` before it and the right part from
/// `gap` after it. With no centre part the right one keeps its width and the left one ends
/// `gap` before it. A part is drawn only inside its room, so long parts are cut, never overprinted.
pub fn band_rooms(width: i32, gap: i32, [_, centre, right]: [i32; 3]) -> [(i32, i32); 3] {
    let right_room = |from: i32| (from, width - from);
    if centre > 0 {
        let w = centre.min(width);
        let x = (width - w) / 2;
        return [(0, x - gap), (x, w), right_room(x + w + gap)];
    }
    if right > 0 {
        let x = width - right.min(width);
        return [(0, x - gap), (x, 0), right_room(x)];
    }
    [(0, width), (0, 0), (width, 0)]
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
    /// The header's row, `line_h` tall; none when the top margin is thinner than a row.
    pub fn header(&self, line_h: i32) -> Option<Rect> {
        centred_row(Rect::new(0, 0, self.width, self.top.max(0)), line_h)
    }

    /// The footer's row; none when the bottom margin is thinner than a row.
    pub fn footer(&self, line_h: i32) -> Option<Rect> {
        let bottom = self.bottom.clamp(0, self.height);
        centred_row(
            Rect::new(0, self.height - bottom, self.width, bottom),
            line_h,
        )
    }
}

fn centred_row(margin: Rect, line_h: i32) -> Option<Rect> {
    (margin.height >= line_h).then(|| {
        Rect::new(
            margin.x,
            margin.y + (margin.height - line_h) / 2,
            margin.width,
            line_h,
        )
    })
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
    let rows = [
        (header, frame.header(m.line_h)),
        (footer, frame.footer(m.line_h)),
    ];
    for (band, row) in rows {
        let Some(row) = row else {
            continue;
        };
        let parts = [&band.left, &band.centre, &band.right];
        let widths = parts.map(|p| match p.is_empty() {
            true => 0,
            false => text.measure(p, m.bold, m.font_px),
        });
        let rooms = band_rooms(row.width, m.font_px as i32, widths);
        let aligns = [Align::Left, Align::Center, Align::Right];
        for ((part, (x, w)), align) in parts.into_iter().zip(rooms).zip(aligns) {
            if !part.is_empty() && w > 0 {
                let room = Rect::new(row.x + x, row.y, w, row.height);
                text.draw(&mut c, room, part, m.bold, INK, align, m.font_px);
            }
        }
    }
    c
}

/// Where a picture `size` px at `dpi` goes in `room` (hundredths of a millimetre), centred: at
/// the size its resolution gives it unless `scaling` fits it to the room or it is larger than
/// the room, then as large as fits.
pub fn place(size: (u32, u32), dpi: u32, room: Rect, scaling: Scaling) -> Rect {
    let own = |n: u32| pagesetup::from_px(n.min(i32::MAX as u32) as i32, dpi as i32).max(1);
    let (w, h) = (own(size.0), own(size.1));
    if scaling == Scaling::Fit || w > room.width || h > room.height {
        let px = |n: u32| n.min(i32::MAX as u32) as i32;
        return fit(px(size.0), px(size.1), room);
    }
    Rect::new(
        room.x + (room.width - w) / 2,
        room.y + (room.height - h) / 2,
        w,
        h,
    )
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
    fn header_and_footer_rows_sit_centred_in_their_margins_and_skip_thin_ones() {
        let frame = Frame {
            width: 200,
            height: 1000,
            top: 100,
            bottom: 60,
        };
        assert_eq!(frame.header(20), Some(Rect::new(0, 40, 200, 20)));
        assert_eq!(frame.footer(20), Some(Rect::new(0, 960, 200, 20)));
        let tight = Frame { top: 19, ..frame };
        assert_eq!(
            tight.header(20),
            None,
            "a margin thinner than a row prints none"
        );
    }

    #[test]
    fn band_parts_keep_to_rooms_that_never_overlap() {
        assert_eq!(
            band_rooms(300, 10, [500, 40, 60]),
            [(0, 120), (130, 40), (180, 120)],
            "a long left part is cut before the centred one"
        );
        assert_eq!(
            band_rooms(300, 10, [500, 0, 60]),
            [(0, 230), (240, 0), (240, 60)],
            "with no centre part the right one keeps its width"
        );
        assert_eq!(
            band_rooms(300, 10, [500, 0, 0]),
            [(0, 300), (0, 0), (300, 0)]
        );
        let [l, c, r] = band_rooms(300, 10, [0, 400, 50]);
        assert_eq!(
            c,
            (0, 300),
            "a centre part wider than the row takes all of it"
        );
        assert!(l.1 <= 0 && r.1 <= 0);
        for widths in [[90, 120, 90], [10, 280, 10], [0, 0, 400]] {
            let rooms = band_rooms(300, 10, widths);
            let used: Vec<(i32, i32)> = rooms.into_iter().filter(|r| r.1 > 0).collect();
            for (i, a) in used.iter().enumerate() {
                for b in &used[i + 1..] {
                    assert!(
                        a.0 + a.1 <= b.0 || b.0 + b.1 <= a.0,
                        "{widths:?}: {a:?} {b:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn control_characters_in_a_header_print_as_spaces() {
        let s = Stamp {
            file: "a\tb\nc.txt",
            ..stamp(1)
        };
        assert_eq!(band("&f\u{7}", &s), centred("a b c.txt "));
    }

    #[test]
    fn a_framed_page_draws_its_header_and_footer_in_the_margins_only() {
        crate::pins::fonts();
        let text = TextRenderer::load(&crate::theme::FontConfig {
            size_px: 10,
            ..crate::theme::FontConfig::default()
        });
        let m = Metrics {
            tab_cols: 4,
            ..Metrics::for_font(&text, 10)
        };
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
        assert!(
            !inked(&c, 40 + m.line_h, 260),
            "neither runs into the text's room"
        );
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

    #[test]
    fn a_picture_that_fits_prints_at_its_own_resolution_unless_fitted() {
        let a4_body = Rect::new(2000, 2500, 17000, 24700);
        let icon = place((48, 48), 96, a4_body, Scaling::Actual);
        assert_eq!(
            (icon.width, icon.height),
            (1270, 1270),
            "48 px at 96 dpi is 12.7 mm"
        );
        assert_eq!(
            (icon.x, icon.y),
            (2000 + (17000 - 1270) / 2, 2500 + (24700 - 1270) / 2),
            "centred between the margins"
        );
        let scan = place((1800, 600), 300, a4_body, Scaling::Actual);
        assert_eq!(
            (scan.width, scan.height),
            (15240, 5080),
            "1800 px at 300 dpi is 152.4 mm"
        );
        let photo = place((4000, 3000), 72, a4_body, Scaling::Actual);
        assert_eq!(
            photo,
            fit(4000, 3000, a4_body),
            "a picture larger than the body is fitted"
        );
        let wide = place((25_000_000, 1), 24, a4_body, Scaling::Actual);
        assert_eq!(
            wide,
            fit(25_000_000, 1, a4_body),
            "a size past i32 is fitted, not wrapped"
        );
        let fitted = place((48, 48), 96, a4_body, Scaling::Fit);
        assert_eq!(
            (fitted.width, fitted.height),
            (17000, 17000),
            "Fit to page fills the width"
        );
    }

    #[test]
    fn accents_over_capitals_and_descenders_print_whole() {
        crate::pins::fonts();
        let face = TextRenderer::monospace(50);
        let text = "\u{150}\u{c9}\u{c1}\u{dc}\u{d6} gjpq";
        let lines = vec![String::new(), text.to_string(), String::new()];
        let inked_rows = |m: &Metrics| {
            let pages = text_pages(&face, m, &lines, 1000, 3 * m.line_h);
            let c = draw_text_page(&face, m, &lines, &pages[0], (1000, 3 * m.line_h));
            let rows: Vec<i32> = (0..c.height as i32)
                .filter(|&y| (0..c.width as i32).any(|x| c.get(x, y) != PAPER.argb()))
                .collect();
            let ink = (0..c.height as i32)
                .flat_map(|y| (0..c.width as i32).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) != PAPER.argb())
                .count();
            (rows, ink)
        };
        let short = Metrics {
            font_px: 50,
            line_h: 50,
            ..Metrics::default()
        };
        let (rows, short_ink) = inked_rows(&short);
        assert!(
            rows.first().is_some_and(|&y| y < short.line_h),
            "the accents ink above a row shorter than the face's line: {:?}",
            rows.first()
        );
        let m = Metrics::for_font(&face, 50);
        assert!(
            m.line_h > 50,
            "a row is as tall as the face's line, {}",
            m.line_h
        );
        let (rows, ink) = inked_rows(&m);
        assert_eq!(
            ink, short_ink,
            "every accent and descender pixel prints either way"
        );
        assert!(
            rows.iter().all(|y| (m.line_h..2 * m.line_h).contains(y)),
            "{rows:?}"
        );
    }
}
