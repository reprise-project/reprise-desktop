//! The text tool's box: the text typed into it, its font, and the coverage it lays on the
//! picture. The box sits on the picture at `Doc::text_box` and grows down as lines are added.

use reprisekit::controls::multiedit::{Metrics, MultiEdit, MultiKey};
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

pub struct TextBox {
    pub edit: MultiEdit,
    pub font: Font,
    renderer: RefCell<Option<(String, TextRenderer)>>,
}

impl Default for TextBox {
    fn default() -> TextBox {
        TextBox {
            edit: MultiEdit::new(""),
            font: Font::default(),
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
            line_h: self.line_h(),
            ..Metrics::default()
        }
    }

    pub fn clear(&mut self) {
        self.edit = MultiEdit::new("");
    }

    pub fn key(&mut self, key: MultiKey) -> bool {
        let m = self.metrics();
        self.with_renderer(|_| ());
        let slot = self.renderer.borrow();
        let (_, r) = slot.as_ref().expect("loaded above");
        self.edit.key(key, r, &m)
    }

    pub fn paste(&mut self, text: &str) -> bool {
        self.edit.paste(text)
    }

    /// `r` grown down to hold every line.
    pub fn fit(&self, r: Rect) -> Rect {
        let h = self.edit.lines().len() as i32 * self.line_h();
        Rect::new(r.x, r.y, r.width, r.height.max(h))
    }

    /// One byte of coverage per pixel of a `w`x`h` box.
    pub fn cover(&self, w: i32, h: i32) -> Vec<u8> {
        let mut c = Canvas::new(w.max(1) as u32, h.max(1) as u32);
        let lh = self.line_h();
        let (px, bold) = (self.font.px, self.font.bold);
        self.with_renderer(|r| {
            for (i, line) in self.edit.lines().iter().enumerate() {
                let rect = Rect::new(0, i as i32 * lh, w, lh);
                r.draw(
                    &mut c,
                    rect,
                    line,
                    bold,
                    Color::rgb(255, 255, 255),
                    Align::Left,
                    px,
                );
            }
        });
        c.pixels.iter().map(|p| (p >> 24) as u8).collect()
    }

    /// The caret's rect in the box, one pixel wide and a line tall.
    pub fn caret(&self) -> Rect {
        let pos = self.edit.caret();
        let lh = self.line_h();
        let line = self.edit.lines().get(pos.line).map_or("", String::as_str);
        let (px, bold) = (self.font.px, self.font.bold);
        let x = self.with_renderer(|r| r.measure(&line[..pos.col.min(line.len())], bold, px));
        Rect::new(x, pos.line as i32 * lh, 1, lh)
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
