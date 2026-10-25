//! Font lookup (fontdb) and hinted glyph rasterization (FreeType). With `antialias` off glyphs load
//! as `FT_LOAD_TARGET_MONO` (bytecode hinter, monochrome); on, hinted 8-bit coverage is blended and
//! `full_hinting` fits stems across as well as down.

use crate::fontfallback::{
    FaceFile, FallbackCache, FallbackDraw, fallback_draw, fallback_strike, fontconfig_face_for,
};
use crate::raster::{Canvas, Rect};
use crate::theme::{Color, FontConfig};
use freetype::face::LoadFlag;
use freetype::{Face, Library};
use rustc_hash::FxHashMap;
use std::cell::RefCell;

mod fonts;
mod mnemonic;
#[cfg(test)]
mod tests;
mod wrap;

pub(crate) use fonts::system_fonts;
pub use fonts::test_tahoma;
use fonts::{full_hinting, load_faces, load_shell_dlg};
pub use mnemonic::{access_key, escape_mnemonic, split_mnemonic};
pub use wrap::{wrap, wrap_bold, wrap_chars};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

impl Align {
    pub fn pen(self, rect: Rect, width: i32) -> i32 {
        match self {
            Align::Left => rect.x,
            Align::Center => rect.x + (rect.width - width) / 2,
            Align::Right => rect.right() - width,
        }
    }
}

struct Glyph {
    bitmap: Vec<u8>,
    width: usize,
    height: usize,
    left: i32,
    top: i32,
    advance: i32,
    smooth: bool,
}

impl Glyph {
    const EMPTY: Glyph = Glyph {
        bitmap: Vec::new(),
        width: 0,
        height: 0,
        left: 0,
        top: 0,
        advance: 0,
        smooth: false,
    };

    fn scaled(self, k: usize) -> Glyph {
        let (width, height) = (self.width * k, self.height * k);
        let mut bitmap = Vec::with_capacity(width * height);
        for y in 0..height {
            let row = &self.bitmap[y / k * self.width..][..self.width];
            bitmap.extend((0..width).map(|x| row[x / k]));
        }
        let k = k as i32;
        Glyph {
            bitmap,
            width,
            height,
            left: self.left * k,
            top: self.top * k,
            advance: self.advance * k,
            smooth: self.smooth,
        }
    }

    /// This glyph slanted for a family without a slanted face: each row moved right by its
    /// height over the baseline over [`SHEAR_RISE`], rows under it left.
    fn sheared(self) -> Glyph {
        if self.width == 0 || self.bitmap.len() != self.width * self.height {
            return self;
        }
        let shift = |row: usize| (self.top - 1 - row as i32).div_euclid(SHEAR_RISE);
        let lo = shift(self.height - 1);
        let width = self.width + (shift(0) - lo) as usize;
        let mut bitmap = vec![0; width * self.height];
        for (row, ink) in self.bitmap.chunks(self.width).enumerate() {
            let x = row * width + (shift(row) - lo) as usize;
            bitmap[x..x + self.width].copy_from_slice(ink);
        }
        Glyph {
            bitmap,
            width,
            left: self.left + lo,
            ..self
        }
    }
}

pub(crate) fn fixed_sizes(f: &Face) -> &[freetype::ffi::FT_Bitmap_Size] {
    let raw = f.raw();
    if raw.available_sizes.is_null() {
        return &[];
    }
    // SAFETY: FreeType keeps `num_fixed_sizes` entries at `available_sizes`
    // for the face's lifetime, which the borrow of `f` covers.
    unsafe { std::slice::from_raw_parts(raw.available_sizes, raw.num_fixed_sizes.max(0) as usize) }
}

/// A bitmap face's strikes as whole-pixel ppems.
fn strike_ppems(f: &Face) -> impl Iterator<Item = u32> + '_ {
    fixed_sizes(f).iter().map(|s| ((s.y_ppem + 32) >> 6) as u32)
}

#[derive(Clone)]
struct LoadedFace {
    face: Face,
    family: String,
    /// Slanted: an italic or an oblique face.
    italic: bool,
}

/// A sheared glyph's rows move right a pixel for each this many over the baseline.
const SHEAR_RISE: i32 = 4;

/// The glyph cache is bounded because clients pick the drawn text (titles); past this many entries
/// it is dropped and refilled.
const GLYPH_CAP: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DialogFont {
    #[default]
    Ui,
    /// The second dialog font, `[fonts] shell_dlg`; the UI font stands in while it is unset.
    ShellDlg,
}

/// (char, bold, logical size, canvas scale). A `None` char is the shared notdef entry; the face is
/// settled by the logical size alone.
type GlyphKey = (Option<char>, bool, u32, u32);

type LineMetrics = (i32, i32, i32);

pub struct TextRenderer {
    regular: Option<LoadedFace>,
    bold: Option<LoadedFace>,
    antialias: bool,
    cache: RefCell<FxHashMap<GlyphKey, Glyph>>,
    /// (ascender, descender, line height) per (bold, size); asked of FreeType once because a size
    /// change invalidates the face's hinting state.
    metrics: RefCell<FxHashMap<(bool, u32), LineMetrics>>,
    shell_dlg: Option<Box<TextRenderer>>,
    /// The (regular, bold) faces line metrics come from when not this renderer's own: the second
    /// dialog font takes the UI font's so both sit on one baseline.
    line_faces: Option<(Face, Face)>,
    lead: i32,
    fallback: RefCell<FallbackCache<FaceFile, Face>>,
    fallback_mono: RefCell<FallbackCache<FaceFile, Face>>,
    fallback_lib: std::cell::OnceCell<Option<Library>>,
    /// The config [`TextRenderer::load`] read, which [`TextRenderer::italic`] loads slanted.
    config: Option<FontConfig>,
    /// Glyphs lean as [`Glyph::sheared`] draws them: the italic of a family without one.
    sheared: bool,
    italic: std::cell::OnceCell<Option<Box<TextRenderer>>>,
    /// Sizes are an output's device px on a scale-1 canvas at another scale
    /// ([`TextRenderer::for_output`]): bitmap fallback strikes are off.
    prescaled: bool,
    for_output: std::cell::OnceCell<Box<TextRenderer>>,
}

fn coverage(bm: &freetype::Bitmap) -> Vec<u8> {
    use freetype::bitmap::PixelMode;
    let (w, h, pitch) = (bm.width() as usize, bm.rows() as usize, bm.pitch());
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let bits = match bm.pixel_mode() {
        Ok(PixelMode::Mono) => 1,
        Ok(PixelMode::Gray2) => 2,
        Ok(PixelMode::Gray4) => 4,
        Ok(PixelMode::Bgra) => 32,
        _ => 8,
    };
    let buf = bm.buffer();
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        let row_start = if pitch >= 0 {
            y * pitch as usize
        } else {
            (h - 1 - y) * (-pitch) as usize
        };
        for x in 0..w {
            let at = |i: usize| buf.get(row_start + i).copied().unwrap_or(0);
            out[y * w + x] = if bits == 32 {
                at(4 * x + 3)
            } else {
                let bit = x * bits;
                let max = (1u16 << bits) - 1;
                let v = u16::from(at(bit / 8)) >> (8 - bits - bit % 8) & max;
                (v * 255 / max) as u8
            };
        }
    }
    out
}

impl TextRenderer {
    pub fn load(cfg: &FontConfig) -> TextRenderer {
        let library = match Library::init() {
            Ok(l) => Some(l),
            Err(e) => {
                log::warn!("FreeType init failed: {e}; text will not be drawn");
                None
            }
        };
        let style = if cfg.italic {
            fontdb::Style::Italic
        } else {
            fontdb::Style::Normal
        };
        let (regular, bold) = match &library {
            Some(lib) => {
                if cfg.full_hinting {
                    full_hinting(lib);
                }
                let db = system_fonts(&cfg.family);
                load_faces(lib, &db, &cfg.family, true, style)
            }
            None => (None, None),
        };
        if regular.is_none() {
            log::warn!("no usable font found; text will not be drawn");
        }
        let mut r = TextRenderer::with_faces(regular, bold, cfg.antialias);
        r.config = Some(cfg.clone());
        match cfg
            .shell_dlg
            .as_deref()
            .and_then(|spec| load_shell_dlg(spec, cfg))
        {
            Some(second) => r.with_shell_dlg(second),
            None => r,
        }
    }

    fn with_faces(
        regular: Option<LoadedFace>,
        bold: Option<LoadedFace>,
        antialias: bool,
    ) -> TextRenderer {
        TextRenderer {
            regular,
            bold,
            antialias,
            cache: RefCell::default(),
            metrics: RefCell::default(),
            shell_dlg: None,
            line_faces: None,
            lead: 0,
            fallback: RefCell::new(FallbackCache::default()),
            fallback_mono: RefCell::new(FallbackCache::default()),
            fallback_lib: std::cell::OnceCell::new(),
            config: None,
            sheared: false,
            italic: std::cell::OnceCell::new(),
            prescaled: false,
            for_output: std::cell::OnceCell::new(),
        }
    }

    /// These faces for a host that draws text at an output's device size on a scale-1 canvas
    /// (sizes from `theme.scaled(scale)`): off scale 1 a char the UI face lacks draws from the
    /// outline face, as on a canvas at `scale`. Itself at scale 1.
    pub fn for_output(&self, scale: i32) -> &TextRenderer {
        if scale <= 1 || self.prescaled {
            return self;
        }
        self.for_output
            .get_or_init(|| Box::new(self.prescaled_copy()))
    }

    fn prescaled_copy(&self) -> TextRenderer {
        let mut t =
            TextRenderer::with_faces(self.regular.clone(), self.bold.clone(), self.antialias);
        t.shell_dlg = self
            .shell_dlg
            .as_ref()
            .map(|s| Box::new(s.prescaled_copy()));
        t.line_faces = self.line_faces.clone();
        t.lead = self.lead;
        t.config = self.config.clone();
        t.sheared = self.sheared;
        t.prescaled = true;
        t
    }

    fn set_prescaled(&mut self) {
        self.prescaled = true;
        if let Some(s) = self.shell_dlg.as_mut() {
            s.set_prescaled();
        }
    }

    /// These families' italic or oblique faces, loaded on first use; a family without one has
    /// its upright glyphs sheared. Itself when it is slanted already or was not loaded from a
    /// config.
    pub fn italic(&self) -> &TextRenderer {
        let Some(cfg) = self.config.as_ref().filter(|c| !c.italic) else {
            return self;
        };
        self.italic
            .get_or_init(|| {
                let cfg = FontConfig {
                    italic: true,
                    ..cfg.clone()
                };
                let mut t = TextRenderer::load(&cfg);
                t.sheared = !t.regular.as_ref().is_some_and(|f| f.italic);
                if self.prescaled {
                    t.set_prescaled();
                }
                t.regular.is_some().then(|| Box::new(t))
            })
            .as_deref()
            .unwrap_or(self)
    }

    pub fn with_shell_dlg(mut self, mut second: TextRenderer) -> TextRenderer {
        second.line_faces = self
            .regular
            .as_ref()
            .zip(self.bold.as_ref())
            .map(|(r, b)| (r.face.clone(), b.face.clone()));
        second.metrics.borrow_mut().clear();
        second.lead = 1;
        self.shell_dlg = Some(Box::new(second));
        self
    }

    /// Blank columns this font's glyphs start with before their ink, beyond the UI font's (0 for
    /// the UI font, 1 for the second dialog font).
    pub fn lead(&self) -> i32 {
        self.lead
    }

    pub fn dialog(&self, font: DialogFont) -> &TextRenderer {
        match (font, &self.shell_dlg) {
            (DialogFont::ShellDlg, Some(second)) => second,
            _ => self,
        }
    }

    pub fn has_shell_dlg(&self) -> bool {
        self.shell_dlg.is_some()
    }

    /// A faceless renderer whose printable ASCII glyphs are `advance(ch)` px wide and ink one pixel
    /// at the pen; layout tests use it as a second dialog font.
    #[doc(hidden)]
    pub fn stub(size_px: u32, advance: impl Fn(char) -> i32) -> TextRenderer {
        let r = TextRenderer::with_faces(None, None, false);
        {
            let mut cache = r.cache.borrow_mut();
            for ch in ' '..='~' {
                for bold in [false, true] {
                    let ink = ch != ' ';
                    let g = Glyph {
                        bitmap: if ink { vec![255] } else { Vec::new() },
                        width: ink as usize,
                        height: ink as usize,
                        left: 0,
                        top: 1,
                        advance: advance(ch),
                        smooth: false,
                    };
                    cache.insert((Some(ch), bold, size_px, 1), g);
                }
            }
        }
        r
    }

    #[cfg(test)]
    pub(crate) fn fontless() -> TextRenderer {
        TextRenderer::with_faces(None, None, false)
    }

    pub fn has_face(&self, family: &str) -> bool {
        self.regular.as_ref().is_some_and(|f| f.family == family)
    }

    #[cfg(test)]
    pub(crate) fn is_italic(&self) -> bool {
        self.regular.as_ref().is_some_and(|f| f.italic)
    }

    fn face(&self, bold: bool) -> Option<&Face> {
        let f = if bold { &self.bold } else { &self.regular };
        f.as_ref().map(|l| &l.face)
    }

    /// The face a glyph at logical `size_px` comes from, with the strike (ppem) to scale up for a
    /// bitmap font. A bitmap font is used only at whole multiples of a strike, nearest neighbour;
    /// any other size is set in the UI font. The choice goes by logical size so `measure` and
    /// `draw` agree at every scale.
    fn source(&self, bold: bool, size_px: u32) -> Option<(&Face, Option<u32>)> {
        let face = self.face(bold)?;
        if face.is_scalable() {
            return Some((face, None));
        }
        let strike = strike_ppems(face)
            .filter(|&p| p > 0 && size_px.is_multiple_of(p))
            .max();
        Some(match (strike, &self.line_faces) {
            (Some(p), _) => (face, Some(p)),
            (None, Some((regular, b))) => (if bold { b } else { regular }, None),
            (None, None) => (face, None),
        })
    }

    fn rasterize(
        &self,
        face: &Face,
        index: u32,
        size_px: u32,
        strike: Option<u32>,
        smooth: bool,
    ) -> Option<Glyph> {
        let (load_px, k) = match strike {
            Some(p) => (p, size_px / p),
            None => (size_px, 1),
        };
        face.set_pixel_sizes(0, load_px).ok()?;
        let target = if smooth {
            LoadFlag::TARGET_NORMAL
        } else {
            LoadFlag::TARGET_MONO
        };
        face.load_glyph(index, LoadFlag::RENDER | target).ok()?;
        let slot = face.glyph();
        let bm = slot.bitmap();
        let glyph = Glyph {
            bitmap: coverage(&bm),
            width: bm.width() as usize,
            height: bm.rows() as usize,
            left: slot.bitmap_left(),
            top: slot.bitmap_top(),
            advance: (slot.advance().x >> 6) as i32,
            smooth,
        };
        Some(if k > 1 {
            glyph.scaled(k as usize)
        } else {
            glyph
        })
    }

    /// Run `f` with the glyph for `ch` at `size_px` on a canvas at `scale`, caching it. A char
    /// FreeType rejects caches as [`Glyph::EMPTY`]. `None` only when there is no face.
    fn with_glyph<R>(
        &self,
        ch: char,
        bold: bool,
        size_px: u32,
        scale: u32,
        f: impl FnOnce(&Glyph) -> R,
    ) -> Option<R> {
        let mut key = (Some(ch), bold, size_px, scale);
        if let Some(g) = self.cache.borrow().get(&key) {
            return Some(f(g));
        }
        let (face, strike) = self.source(bold, size_px)?;
        let index = face.get_char_index(ch as usize);
        // A char the UI face lacks comes from a fallback face: a bitmap face in mono when
        // `antialias` is off, a strike fits and the text is drawn at scale 1, else an outline
        // face smoothed (an outline CJK face in mono ink at 11 px loses its strokes).
        let fallback = match index {
            Some(_) => None,
            None => {
                let strike = self.fallback_face(ch, bold, true).and_then(|fb| {
                    let p = fallback_strike(strike_ppems(&fb), size_px)?;
                    Some((fb, p))
                });
                let outline = |px: u32| {
                    let fb = self.fallback_face(ch, bold, false)?;
                    let index = fb.get_char_index(ch as usize)?;
                    self.rasterize(&fb, index, px, None, true)
                };
                match fallback_draw(strike.as_ref().map(|s| s.1), size_px, scale) {
                    FallbackDraw::Strike(p) => strike
                        .and_then(|(fb, _)| {
                            let index = fb.get_char_index(ch as usize)?;
                            self.rasterize(&fb, index, p, Some(p), false)
                        })
                        .or_else(|| outline(size_px)),
                    FallbackDraw::Outline(px) => outline(px),
                }
            }
        };
        if index.is_none() && fallback.is_none() {
            key.0 = None;
            if let Some(g) = self.cache.borrow().get(&key) {
                return Some(f(g));
            }
        }
        let glyph = match fallback {
            Some(g) => g,
            None => self
                .rasterize(
                    face,
                    index.unwrap_or(0),
                    size_px * scale,
                    strike,
                    self.antialias,
                )
                .unwrap_or(Glyph::EMPTY),
        };
        let glyph = if self.sheared { glyph.sheared() } else { glyph };
        let r = f(&glyph);
        let mut cache = self.cache.borrow_mut();
        if cache.len() >= GLYPH_CAP {
            cache.clear();
        }
        cache.insert(key, glyph);
        Some(r)
    }

    /// The face that draws `ch`, which the UI face lacks: the block's previous face if it has it,
    /// else the one fontconfig names (bitmap or outline per `bitmap`). `None` when no face has it.
    fn fallback_face(&self, ch: char, bold: bool, bitmap: bool) -> Option<Face> {
        if bitmap && (self.antialias || self.prescaled) {
            return None;
        }
        self.face(bold)?;
        let lib = self
            .fallback_lib
            .get_or_init(|| Library::init().ok())
            .as_ref()?;
        let open = |(path, index): &FaceFile| match lib.new_face(path, *index as isize) {
            Ok(face) => {
                log::info!("fallback font for U+{:04X}: {}", ch as u32, path.display());
                Some(face)
            }
            Err(e) => {
                log::warn!("FreeType could not open {}: {e}", path.display());
                None
            }
        };
        let has = |f: &Face, c: char| f.get_char_index(c as usize).is_some();
        let cache = if bitmap {
            &self.fallback_mono
        } else {
            &self.fallback
        };
        let find = |c: char, b: bool| fontconfig_face_for(c, b, bitmap);
        cache.borrow_mut().pick(ch, bold, has, find, open).cloned()
    }

    fn line_metrics(&self, bold: bool, size_px: u32) -> Option<LineMetrics> {
        if let Some(&m) = self.metrics.borrow().get(&(bold, size_px)) {
            return Some(m);
        }
        let face = match &self.line_faces {
            Some((regular, b)) => Some(if bold { b } else { regular }),
            None => self.face(bold),
        }?;
        face.set_pixel_sizes(0, size_px).ok()?;
        let m = face.size_metrics()?;
        let m = (
            (m.ascender >> 6) as i32,
            (m.descender >> 6) as i32,
            (m.height >> 6) as i32,
        );
        self.metrics.borrow_mut().insert((bold, size_px), m);
        Some(m)
    }

    pub fn measure(&self, text: &str, bold: bool, size_px: u32) -> i32 {
        text.chars().map(|ch| self.advance(ch, bold, size_px)).sum()
    }

    /// How far right of the pen `text`'s ink reaches: [`TextRenderer::measure`] less the last
    /// glyph's blank columns after its ink (a space's whole advance).
    pub fn ink_width(&self, text: &str, bold: bool, size_px: u32) -> i32 {
        let Some(last) = text.chars().next_back() else {
            return 0;
        };
        let pen = self.measure(text, bold, size_px) - self.advance(last, bold, size_px);
        let ink = self
            .with_glyph(last, bold, size_px, 1, |g| {
                if g.width == 0 {
                    0
                } else {
                    g.left + g.width as i32
                }
            })
            .unwrap_or(0);
        pen + ink.min(self.advance(last, bold, size_px))
    }

    pub fn advance(&self, ch: char, bold: bool, size_px: u32) -> i32 {
        self.with_glyph(ch, bold, size_px, 1, |g| g.advance)
            .unwrap_or(0)
    }

    pub fn baseline(&self, rect: Rect, bold: bool, size_px: u32) -> Option<i32> {
        let (ascent, descent, _) = self.line_metrics(bold, size_px)?;
        Some(rect.y + (rect.height - (ascent - descent)) / 2 + ascent)
    }

    pub fn line(&self, rect: Rect, bold: bool, size_px: u32) -> Option<(i32, i32)> {
        let (ascent, _, height) = self.line_metrics(bold, size_px)?;
        Some((self.baseline(rect, bold, size_px)? - ascent, height))
    }

    pub fn span(&self, text: &str, idx: usize, bold: bool, size_px: u32) -> (i32, i32) {
        let (x, w, _) = self.span_and_measure(text, idx, bold, size_px);
        (x, w)
    }

    pub fn span_and_measure(
        &self,
        text: &str,
        idx: usize,
        bold: bool,
        size_px: u32,
    ) -> (i32, i32, i32) {
        let mut hit = None;
        let mut total = 0;
        for (i, ch) in text.chars().enumerate() {
            let adv = self.advance(ch, bold, size_px);
            if i == idx {
                hit = Some((total, adv));
            }
            total += adv;
        }
        let (x, w) = hit.unwrap_or((total, 0));
        (x, w, total)
    }

    pub fn ellipsize<'t>(
        &self,
        text: &'t str,
        width: i32,
        bold: bool,
        size_px: u32,
    ) -> std::borrow::Cow<'t, str> {
        if self.measure(text, bold, size_px) <= width {
            return std::borrow::Cow::Borrowed(text);
        }
        let dots = self.measure("...", bold, size_px);
        let mut kept = String::new();
        let mut used = 0;
        for ch in text.chars() {
            let adv = self.advance(ch, bold, size_px);
            if used + adv + dots > width {
                break;
            }
            used += adv;
            kept.push(ch);
        }
        let trimmed = kept.trim_end();
        format!("{trimmed}...").into()
    }

    #[expect(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        canvas: &mut Canvas,
        rect: Rect,
        text: &str,
        bold: bool,
        color: Color,
        align: Align,
        size_px: u32,
    ) {
        self.draw_clipped(canvas, rect, rect, text, bold, color, align, size_px);
    }

    #[expect(clippy::too_many_arguments)]
    pub fn draw_clipped(
        &self,
        canvas: &mut Canvas,
        rect: Rect,
        clip: Rect,
        text: &str,
        bold: bool,
        color: Color,
        align: Align,
        size_px: u32,
    ) {
        let Some(baseline) = self.baseline(rect, bold, size_px) else {
            return;
        };
        let clip = rect.intersect(&clip);
        if clip.is_empty() {
            return;
        }
        let width = match align {
            Align::Left => 0,
            Align::Center | Align::Right => self.measure(text, bold, size_px),
        };
        let pen = align.pen(rect, width);
        self.draw_run(canvas, pen, baseline, clip, text, bold, color, size_px);
    }

    /// Draw `text` from pen x `pen` on `baseline`, clipped to `clip`. At canvas scale `s` glyphs
    /// rasterize at `s * size_px` but the pen moves by the `size_px` advances, keeping the logical
    /// width carets are measured against.
    #[expect(clippy::too_many_arguments)]
    pub fn draw_run(
        &self,
        canvas: &mut Canvas,
        mut pen: i32,
        baseline: i32,
        clip: Rect,
        text: &str,
        bold: bool,
        color: Color,
        size_px: u32,
    ) {
        let s = canvas.scale();
        for ch in text.chars() {
            let logical = (s > 1).then(|| self.advance(ch, bold, size_px));
            let drawn = self.with_glyph(ch, bold, size_px, s as u32, |g| {
                let gx = pen * s + g.left;
                let gy = baseline * s - g.top;
                canvas.blit_mask(gx, gy, g.width, g.height, &g.bitmap, color, g.smooth, clip);
                g.advance
            });
            pen += logical.or(drawn).unwrap_or(0);
        }
    }

    #[expect(clippy::too_many_arguments)]
    pub fn draw_run_over(
        &self,
        canvas: &mut Canvas,
        pen: i32,
        baseline: i32,
        clip: Rect,
        text: &str,
        bold: bool,
        color: Color,
        size_px: u32,
    ) {
        if clip.width <= 0 || clip.height <= 0 {
            return;
        }
        let local = Rect::new(0, 0, clip.width, clip.height);
        let ink = Canvas::new(clip.width as u32, clip.height as u32);
        let coverage = ink.ink_coverage(|c, white| {
            self.draw_run(
                c,
                pen - clip.x,
                baseline - clip.y,
                local,
                text,
                bold,
                white,
                size_px,
            );
        });
        let all = canvas.bounds();
        canvas.blit_mask_over(
            clip.x,
            clip.y,
            clip.width as usize,
            clip.height as usize,
            &coverage,
            color,
            all,
        );
    }
}
