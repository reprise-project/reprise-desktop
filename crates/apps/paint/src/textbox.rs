//! The text tool's box: the text typed into it, its font, and the coverage it lays on the
//! picture. The box sits on the picture at `Doc::text_box`; its lines wrap at the box's width and
//! it grows down as rows are added. A box a click opened also widens with its widest line, up to
//! the picture's right edge.

use reprisekit::controls::multiedit::{self, Metrics, MultiEdit, MultiKey};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::{Align, TextRenderer};
use reprisekit::theme::{Color, FontConfig};
use std::cell::RefCell;

pub const SIZES: [u32; 12] = [8, 9, 10, 11, 12, 14, 16, 18, 20, 24, 28, 36];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Font {
    pub family: String,
    pub px: u32,
    pub bold: bool,
}

impl Default for Font {
    fn default() -> Font {
        Font {
            family: "DejaVu Sans".into(),
            px: 16,
            bold: false,
        }
    }
}

/// How far short of its width the kit's edit wraps a row: the caret's room.
const CARET_ROOM: i32 = 3;

/// A clicked box's width at first, in ems of its font.
const CLICKED_EMS: i32 = 4;

pub struct TextBox {
    pub edit: MultiEdit,
    pub font: Font,
    /// Set while the box was opened by a click: the widest it may grow, in picture px.
    grow: Option<i32>,
    renderer: RefCell<Option<(String, TextRenderer)>>,
}

impl Default for TextBox {
    fn default() -> TextBox {
        TextBox {
            edit: MultiEdit::new(""),
            font: Font::default(),
            grow: None,
            renderer: RefCell::new(None),
        }
    }
}

impl TextBox {
    fn with_renderer<T>(&self, f: impl FnOnce(&TextRenderer) -> T) -> T {
        let mut slot = self.renderer.borrow_mut();
        if slot
            .as_ref()
            .is_none_or(|(fam, _)| *fam != self.font.family)
        {
            let r = TextRenderer::load(&FontConfig {
                family: vec![self.font.family.clone()],
                size_px: self.font.px,
                antialias: true,
                ..FontConfig::default()
            });
            *slot = Some((self.font.family.clone(), r));
        }
        let (_, r) = slot.as_ref().expect("loaded above");
        f(r)
    }

    pub fn line_h(&self) -> i32 {
        let (px, bold) = (self.font.px, self.font.bold);
        self.with_renderer(|r| r.line(Rect::new(0, 0, 1, 1), bold, px).map(|(_, h)| h))
            .unwrap_or(self.font.px as i32 * 5 / 4)
            .max(1)
    }

    fn metrics(&self) -> Metrics {
        Metrics {
            font_px: self.font.px,
            bold: self.font.bold,
            line_h: self.line_h(),
            bars_always: false,
            flat: true,
            wrap: true,
            ..Metrics::default()
        }
    }

    /// The width rows wrap at in a box `w` px wide, as the kit's edit lays it out; laying it out
    /// is also what Up, Down, Home and End move by.
    fn wrap_at(&self, r: &TextRenderer, m: &Metrics, w: i32) -> i32 {
        let tall = Rect::new(0, 0, w.max(1), 1 << 20);
        multiedit::layout(r, m, &self.edit, tall, 1)
            .wrap
            .unwrap_or(w)
    }

    /// Each row in a box `w` px wide: its line and byte range.
    fn rows(&self, r: &TextRenderer, m: &Metrics, w: i32) -> Vec<(usize, usize, usize)> {
        let width = self.wrap_at(r, m, w);
        let mut out = Vec::new();
        for (i, line) in self.edit.lines().iter().enumerate() {
            let mut start = 0;
            for b in multiedit::breaks(r, m, line, width) {
                out.push((i, start, b));
                start = b;
            }
            out.push((i, start, line.len()));
        }
        out
    }

    pub fn clear(&mut self) {
        self.edit = MultiEdit::new("");
        self.grow = None;
    }

    /// The box a drag over `r` opens. A click, a 1x1 `r`, opens one a line tall and a few
    /// characters wide that widens as text is typed, up to `room` px.
    pub fn opened(&mut self, r: Rect, room: i32) -> Rect {
        let clicked = r.width == 1 && r.height == 1;
        self.grow = clicked.then_some(room);
        if !clicked {
            return r;
        }
        let w = (self.font.px as i32 * CLICKED_EMS).min(room);
        Rect::new(r.x, r.y, w, self.line_h())
    }

    /// A key in a box `w` px wide.
    pub fn key(&mut self, key: MultiKey, w: i32) -> multiedit::Took {
        let m = self.metrics();
        self.with_renderer(|_| ());
        let slot = self.renderer.borrow();
        let (_, r) = slot.as_ref().expect("loaded above");
        self.wrap_at(r, &m, w);
        self.edit.key(key, r, &m)
    }

    pub fn paste(&mut self, text: &str) -> bool {
        self.edit.paste(text)
    }

    /// `r` grown down to hold every row, and for a clicked box right to hold its widest line.
    pub fn fit(&self, r: Rect) -> Rect {
        let m = self.metrics();
        let (w, rows) = self.with_renderer(|t| {
            let w = match self.grow {
                Some(room) => (multiedit::widest_px(t, &m, &self.edit) + CARET_ROOM)
                    .clamp(r.width, room.max(r.width)),
                None => r.width,
            };
            (w, self.rows(t, &m, w).len())
        });
        let h = rows as i32 * self.line_h();
        Rect::new(r.x, r.y, w, r.height.max(h))
    }

    /// One byte of coverage per pixel of a `w`x`h` box.
    pub fn cover(&self, w: i32, h: i32) -> Vec<u8> {
        let mut c = Canvas::new(w.max(1) as u32, h.max(1) as u32);
        let lh = self.line_h();
        let (px, bold) = (self.font.px, self.font.bold);
        let (lines, m) = (self.edit.lines(), self.metrics());
        self.with_renderer(|r| {
            for (i, (line, start, end)) in self.rows(r, &m, w).into_iter().enumerate() {
                let rect = Rect::new(0, i as i32 * lh, w, lh);
                r.draw(
                    &mut c,
                    rect,
                    &lines[line][start..end],
                    bold,
                    Color::rgb(255, 255, 255),
                    Align::Left,
                    px,
                );
            }
        });
        c.pixels.iter().map(|p| (p >> 24) as u8).collect()
    }

    /// The caret's rect in a box `w` px wide, one pixel wide and a line tall. At a soft break
    /// it starts the next row.
    pub fn caret(&self, w: i32) -> Rect {
        let pos = self.edit.caret();
        let lh = self.line_h();
        let (px, bold) = (self.font.px, self.font.bold);
        let (lines, m) = (self.edit.lines(), self.metrics());
        self.with_renderer(|r| {
            let rows = self.rows(r, &m, w);
            let row = rows
                .iter()
                .rposition(|&(l, start, _)| l == pos.line && start <= pos.col)
                .unwrap_or(0);
            let (line, start, _) = rows[row];
            let col = pos.col.clamp(start, lines[line].len());
            let x = r.measure(&lines[line][start..col], bold, px);
            Rect::new(x, row as i32 * lh, 1, lh)
        })
    }

    pub fn is_empty(&self) -> bool {
        self.edit.lines().iter().all(String::is_empty)
    }
}

pub fn key_of(k: &repriseapp::KeyInput) -> Option<MultiKey> {
    use reprisekit::textedit::keysym;
    let shift = k.mods.shift;
    match k.sym {
        keysym::UP | keysym::KP_UP => Some(MultiKey::Up { select: shift }),
        keysym::DOWN | keysym::KP_DOWN => Some(MultiKey::Down { select: shift }),
        keysym::RETURN | keysym::KP_ENTER => Some(MultiKey::Enter),
        _ => k.edit.clone().map(MultiKey::Edit),
    }
}
