//! What a program prints, cut into pages and drawn away from its event loop: a text wrapped
//! between the margins, or a picture fitted to them. Pages are drawn one at a time and deflated
//! before the next is drawn, so a long document holds one raw page at once.

use super::{DPI, pdf};
use reprisekit::controls::imageview;
use reprisekit::controls::multiedit::{Metrics, Row};
use reprisekit::controls::pagesetup::{self, PageSetup, points};
use reprisekit::controls::printdlg::Range;
use reprisekit::controls::printpage;
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

    fn metrics(&self) -> Metrics {
        let px = self.px();
        Metrics {
            font_px: px,
            bold: self.bold,
            line_h: px as i32,
            ..Metrics::default()
        }
    }
}

/// What a program hands over to print.
pub enum Doc {
    Text {
        font: TextFont,
        lines: Vec<String>,
    },
    /// A picture turned `turns` quarter turns clockwise.
    Picture {
        picture: Arc<Canvas>,
        turns: u8,
    },
}

/// A [`Doc`] cut into pages at a page setup.
pub struct Laid {
    doc: Doc,
    setup: PageSetup,
    /// A text's rows, page by page; a picture is one page.
    rows: Vec<Vec<Row>>,
}

impl Laid {
    pub fn new(doc: Doc, setup: PageSetup) -> Laid {
        let rows = match &doc {
            Doc::Text { font, lines } => {
                let face = font.load();
                let (w, h) = body_px(&setup);
                printpage::text_pages(face_of(&face, font), &font.metrics(), lines, w, h)
            }
            Doc::Picture { .. } => vec![Vec::new()],
        };
        Laid { doc, setup, rows }
    }

    pub fn pages(&self) -> u32 {
        self.rows.len() as u32
    }

    pub fn setup(&self) -> &PageSetup {
        &self.setup
    }

    /// The pages `range` covers, drawn one after the other and each handed to `out` deflated.
    pub fn render(&self, range: Range, mut out: impl FnMut(pdf::Page)) {
        let b = self.setup.body();
        match &self.doc {
            Doc::Text { font, lines } => {
                let face = font.load();
                let face = face_of(&face, font);
                let m = font.metrics();
                let size = body_px(&self.setup);
                for rows in range
                    .of(self.pages())
                    .filter_map(|i| self.rows.get(i as usize))
                {
                    let raw = Raw::new(printpage::draw_text_page(face, &m, lines, rows, size));
                    out(page(&self.setup, &raw.0, b));
                }
            }
            Doc::Picture { picture, turns } => {
                let size = imageview::turned((picture.width, picture.height), *turns);
                let at = printpage::fit(size.0 as i32, size.1 as i32, b);
                let drawn = fitted(size, at);
                let raw = Raw::new(imageview::render(picture, *turns, drawn));
                out(page(&self.setup, &raw.0, at));
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

/// The size a picture `size` px is drawn at on a page of `s`: what [`Laid::render`] makes of
/// it, so a program can shrink a large picture before handing it over.
pub fn picture_size(s: &PageSetup, size: (u32, u32)) -> (u32, u32) {
    fitted(size, printpage::fit(size.0 as i32, size.1 as i32, s.body()))
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
