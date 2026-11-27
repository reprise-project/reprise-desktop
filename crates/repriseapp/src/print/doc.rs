//! What a program prints, cut into pages and drawn away from its event loop: a text wrapped
//! between the margins, or a picture placed between them as Page Setup's scaling says. Pages are
//! drawn one at a time and deflated before the next is drawn, so a long document holds one raw
//! page at once, and a picture larger than it prints is shrunk as it is laid out, so the job does
//! not keep it whole.

use super::{DPI, pdf};
use reprisekit::controls::imageview;
use reprisekit::controls::multiedit::{Metrics, Row};
use reprisekit::controls::pagesetup::{self, PageSetup, points};
use reprisekit::controls::printdlg::Range;
use reprisekit::controls::printpage::{self, Frame, Stamp, band};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::FontConfig;
use std::sync::Arc;

/// The largest text size printed, in points.
const MAX_PT: u32 = 1000;

/// The face a text prints in: `family` empty is the fixed-pitch face plain text is set in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextFont {
    pub family: String,
    pub bold: bool,
    pub italic: bool,
    pub pt: u32,
}

impl TextFont {
    fn px(&self) -> u32 {
        self.pt.clamp(1, MAX_PT) * DPI as u32 / 72
    }

    /// The face at [`DPI`]; a renderer cannot cross threads, so each thread loads its own.
    fn load(&self) -> TextRenderer {
        match self.family.is_empty() {
            true => TextRenderer::monospace(self.px()),
            false => TextRenderer::load(&FontConfig {
                family: vec![self.family.clone()],
                size_px: self.px(),
                antialias: true,
                ..FontConfig::default()
            }),
        }
    }

    /// Rows as tall as `face`'s line at the size printed.
    fn metrics(&self, face: &TextRenderer) -> Metrics {
        Metrics {
            bold: self.bold,
            ..Metrics::for_font(face, self.px())
        }
    }
}

/// What a program hands over to print.
pub enum Doc {
    Text {
        font: TextFont,
        lines: Vec<String>,
    },
    /// A picture turned `turns` quarter turns clockwise, made at `dpi` dots per inch. A program
    /// hands over its own pixels at full size; laying it out shrinks it.
    Picture {
        picture: Arc<Canvas>,
        turns: u8,
        dpi: u32,
    },
}

/// A [`Doc`] cut into pages at a page setup.
pub struct Laid {
    doc: Doc,
    setup: PageSetup,
    /// What a text's header and footer call the file.
    title: String,
    /// A text's rows, page by page; a picture is one page.
    rows: Vec<Vec<Row>>,
    /// Where a picture goes on its page, hundredths of a millimetre.
    picture_at: Rect,
}

impl Laid {
    /// `doc`, titled `title`, cut into pages of `setup`.
    pub fn new(doc: Doc, setup: PageSetup, title: &str) -> Laid {
        let mut picture_at = Rect::default();
        let (doc, rows) = match doc {
            Doc::Text { font, lines } => {
                let face = font.load();
                let (w, h) = body_px(&setup);
                let face = face_of(&face, &font);
                let rows = printpage::text_pages(face, &font.metrics(face), &lines, w, h);
                (Doc::Text { font, lines }, rows)
            }
            Doc::Picture {
                picture,
                turns,
                dpi,
            } => {
                picture_at = picture_place(&setup, (picture.width, picture.height), turns, dpi);
                let picture = shrunk(picture, turns, picture_at);
                (
                    Doc::Picture {
                        picture,
                        turns,
                        dpi,
                    },
                    vec![Vec::new()],
                )
            }
        };
        Laid {
            doc,
            setup,
            title: title.into(),
            rows,
            picture_at,
        }
    }

    pub fn pages(&self) -> u32 {
        self.rows.len() as u32
    }

    pub fn setup(&self) -> &PageSetup {
        &self.setup
    }

    /// The pages `range` covers, drawn one after the other and each handed to `out` deflated.
    pub fn render(&self, range: Range, mut out: impl FnMut(pdf::Page)) {
        self.draw(range, |_, c, at| out(page(&self.setup, c, at)));
    }

    /// The pages `range` covers, drawn one after the other at [`DPI`]: each page's number from
    /// 0, its drawing and where that goes on the page (hundredths of a millimetre) handed to
    /// `out`, then dropped. Printing and Print Preview both draw their pages here.
    pub fn draw(&self, range: Range, mut out: impl FnMut(u32, &Canvas, Rect)) {
        let b = self.setup.body();
        match &self.doc {
            Doc::Text { font, lines } => {
                let face = font.load();
                let face = face_of(&face, font);
                let m = font.metrics(face);
                let size = body_px(&self.setup);
                let s = &self.setup;
                let framed = !(s.header.is_empty() && s.footer.is_empty());
                let (date, time) = match framed {
                    true => now_stamp(),
                    false => Default::default(),
                };
                for i in range.of(self.pages()) {
                    let Some(rows) = self.rows.get(i as usize) else {
                        continue;
                    };
                    if !framed {
                        let raw = Raw::new(printpage::draw_text_page(face, &m, lines, rows, size));
                        out(i, &raw.0, b);
                        continue;
                    }
                    let stamp = Stamp {
                        file: &self.title,
                        page: i + 1,
                        date: &date,
                        time: &time,
                    };
                    let (header, footer) = (band(&s.header, &stamp), band(&s.footer, &stamp));
                    let raw = Raw::new(printpage::draw_framed_page(
                        face,
                        &m,
                        lines,
                        rows,
                        frame_px(s),
                        &header,
                        &footer,
                    ));
                    out(i, &raw.0, Rect::new(b.x, 0, b.width, s.size().1));
                }
            }
            Doc::Picture { picture, turns, .. } => {
                let size = imageview::turned((picture.width, picture.height), *turns);
                let at = self.picture_at;
                let drawn = fitted(size, at);
                let raw = Raw::new(imageview::render(picture, *turns, drawn));
                out(0, &raw.0, at);
            }
        }
    }
}

fn face_of<'a>(face: &'a TextRenderer, font: &TextFont) -> &'a TextRenderer {
    match font.italic {
        true => face.italic(),
        false => face,
    }
}

/// The part of `s`'s page inside the margins, device px at [`DPI`].
fn body_px(s: &PageSetup) -> (i32, i32) {
    let b = s.body();
    (pagesetup::px(b.width, DPI), pagesetup::px(b.height, DPI))
}

/// The strip of `s`'s page between the left and right margins, top to bottom, device px at
/// [`DPI`]: the text's rows start below the top margin, as [`body_px`] has them.
fn frame_px(s: &PageSetup) -> Frame {
    let (width, body_h) = body_px(s);
    let height = pagesetup::px(s.size().1, DPI);
    let top = pagesetup::px(s.body().y, DPI);
    Frame {
        width,
        height,
        top,
        bottom: height - top - body_h,
    }
}

/// The date and time a header's `&d` and `&t` print, in the user's LC_TIME locale.
fn now_stamp() -> (String, String) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let tm = reprisefs::locale::localtime(now);
    (
        reprisefs::locale::short_date(&tm),
        reprisefs::locale::short_time(&tm),
    )
}

/// A picture `size` px drawn into `at` (hundredths of a millimetre): its size at [`DPI`] there,
/// never more than its own.
fn fitted(size: (u32, u32), at: Rect) -> (u32, u32) {
    let w = pagesetup::px(at.width, DPI).max(1) as u32;
    let h = pagesetup::px(at.height, DPI).max(1) as u32;
    match w < size.0 {
        true => (w, h),
        false => size,
    }
}

/// Where a picture `size` px at `dpi`, turned `turns`, goes on a page of `s`.
fn picture_place(s: &PageSetup, size: (u32, u32), turns: u8, dpi: u32) -> Rect {
    let shown = imageview::turned(size, turns);
    printpage::place(shown, dpi, s.body(), s.scaling)
}

/// `picture` shrunk to the size it prints at in `at` when it is larger, unturned.
fn shrunk(picture: Arc<Canvas>, turns: u8, at: Rect) -> Arc<Canvas> {
    let size = (picture.width, picture.height);
    let shown = imageview::turned(size, turns);
    let drawn = fitted(shown, at);
    if drawn == shown {
        return picture;
    }
    let (w, h) = imageview::turned(drawn, turns);
    let stride = size.0 as usize;
    Arc::new(Canvas::resample_box_from(size.0, size.1, w, h, |x, y| {
        picture.pixels[y as usize * stride + x as usize]
    }))
}

fn page(s: &PageSetup, c: &Canvas, at: Rect) -> pdf::Page {
    let (w, h) = s.size();
    pdf::Page {
        size: (points(w), points(h)),
        image: pdf::Image::from_canvas(c),
        at: [at.x, at.y, at.width, at.height].map(points),
    }
}

/// A page drawn and not yet deflated. Tests count how many are alive at once.
struct Raw(Canvas);

impl Raw {
    fn new(c: Canvas) -> Raw {
        #[cfg(test)]
        tests::LIVE.with(|l| {
            let (live, peak) = l.get();
            l.set((live + 1, peak.max(live + 1)));
        });
        Raw(c)
    }
}

#[cfg(test)]
impl Drop for Raw {
    fn drop(&mut self) {
        tests::LIVE.with(|l| l.set((l.get().0 - 1, l.get().1)));
    }
}

#[cfg(test)]
mod tests;
