//! Font lookup (fontdb) and hinted glyph rasterization (FreeType). With `antialias` off glyphs load
//! as `FT_LOAD_TARGET_MONO` (bytecode hinter, monochrome); on, hinted 8-bit coverage is blended and
//! `full_hinting` fits stems across as well as down.

use crate::fontfallback::{
    FaceFile, FallbackCache, FallbackDraw, fallback_draw, fallback_strike, fontconfig_face_for,
    no_fallback,
};
use crate::raster::{Canvas, Rect};
use crate::theme::{Color, FontConfig};
use freetype::face::LoadFlag;
use freetype::{Face, Library};
use rustc_hash::FxHashMap;
use std::cell::RefCell;

pub mod fontfile;
mod fonts;
mod mnemonic;
#[cfg(test)]
mod tests;
mod wrap;

pub use fonts::families;
pub(crate) use fonts::system_fonts;
pub(crate) use fonts::test_fonts;
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
    /// This glyph's ink pixels, laid from device pixel (`x`, `y`), that fall outside `keep`: above
    /// or below it, and beside it within its rows. Faint antialiased fringes don't count.
    fn ink_cut(&self, x: i32, y: i32, keep: Rect) -> (u32, u32) {
        let mut cut = (0, 0);
        for (row, line) in self
            .bitmap
            .chunks(self.width.max(1))
            .take(self.height)
            .enumerate()
        {
            let py = y + row as i32;
            for (col, &cov) in line.iter().enumerate() {
                let px = x + col as i32;
                if cov < 0x60 {
                    continue;
                } else if py < keep.y || py >= keep.bottom() {
                    cut.0 += 1;
                } else if px < keep.x || px >= keep.right() {
                    cut.1 += 1;
                }
            }
        }
        cut
    }

    /// The rows below the bitmap's top this glyph inks, as [`Glyph::ink_span`] does columns.
    fn ink_rows(&self) -> Option<(i32, i32)> {
        self.rows(0x60)
    }

    /// The rows below the bitmap's top a blit of this glyph visibly sets, as
    /// [`Glyph::drawn_span`] does columns.
    fn drawn_rows(&self) -> Option<(i32, i32)> {
        self.rows(if self.smooth { 0x20 } else { 0x60 })
    }

    fn rows(&self, min: u8) -> Option<(i32, i32)> {
        let inked = |y: &usize| {
            self.bitmap
                .chunks(self.width.max(1))
                .nth(*y)
                .is_some_and(|row| row.iter().any(|&cov| cov >= min))
        };
        let t = (0..self.height).find(inked)?;
        let b = (t..self.height).rev().find(inked)?;
        Some((t as i32, b as i32 + 1))
    }

    /// The columns right of the pen this glyph inks, faint antialiased fringes left out as in
    /// [`Glyph::ink_cut`]; `None` for a blank glyph.
    fn ink_span(&self) -> Option<(i32, i32)> {
        self.span(0x60)
    }

    /// The columns right of the pen a blit of this glyph visibly sets: a smooth glyph's fringes
    /// from 1/8 coverage too.
    fn drawn_span(&self) -> Option<(i32, i32)> {
        self.span(if self.smooth { 0x20 } else { 0x60 })
    }

    fn span(&self, min: u8) -> Option<(i32, i32)> {
        let rows = || self.bitmap.chunks(self.width.max(1)).take(self.height);
        let inked = |x: usize| rows().any(|row| row.get(x).is_some_and(|&cov| cov >= min));
        let l = (0..self.width).find(|&x| inked(x))?;
        let r = (l..self.width).rev().find(|&x| inked(x))?;
        Some((self.left + l as i32, self.left + r as i32 + 1))
    }

    /// Whether this glyph's bitmap, drawn at scale `s` from logical pen x `pen`, lies within
    /// `line`'s columns.
    fn boxed_in(&self, pen: i32, s: i32, line: Rect) -> bool {
        self.left >= s * (line.x - pen) && self.left + self.width as i32 <= s * (line.right() - pen)
    }

    /// How far to move this glyph, drawn at scale `s` from logical pen x `pen`, so its ink stays
    /// within `line`'s columns, when `one`, its ink at scale 1, does: as far as it can without
    /// leaving the other side.
    fn nudge_into(&self, one: Option<(i32, i32)>, pen: i32, s: i32, line: Rect) -> i32 {
        let (lo, hi) = (s * (line.x - pen), s * (line.right() - pen));
        let (Some((l, r)), Some((l1, r1))) = (self.ink_span(), one) else {
            return 0;
        };
        if pen + l1 < line.x || pen + r1 > line.right() {
            return 0;
        }
        if r > hi {
            -(r - hi).min((l - lo).max(0))
        } else if l < lo {
            (lo - l).min((hi - r).max(0))
        } else {
            0
        }
    }

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

pub(crate) fn fixed_sizes<B>(f: &Face<B>) -> &[freetype::ffi::FT_Bitmap_Size] {
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
    /// Set under `REPRISE_TEST_FONTS` for a face a family lookup found outside the test fonts (a
    /// face opened by path has none); [`crate::pins::stray`] reports it when the face draws.
    stray: Option<Box<str>>,
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

/// A selection's highlight at a canvas scale, from [`TextRenderer::highlight`].
pub struct Highlight<'a> {
    /// The highlight as given, in logical pixels.
    pub rect: Rect,
    /// Its device pixels, out to the selected glyphs' ink.
    pub dev: Rect,
    s: i32,
    pen: i32,
    baseline: Option<i32>,
    line: Rect,
    text: &'a str,
    bold: bool,
    size_px: u32,
}

impl Highlight<'_> {
    /// Fill the highlight's device pixels within `clip` in `color`.
    pub fn fill(&self, c: &mut Canvas, clip: Rect, color: Color) {
        let r = self.dev.intersect(&clip.scaled(self.s));
        c.in_device_px(|c| c.fill_rect(r, color));
    }

    /// The device pixels a caret `bar` inverts: one that starts past `rect`'s right edge but
    /// inside `dev` starts past `dev` instead, so it reads beside the highlight as it does at
    /// scale 1, without passing `clip`'s right edge.
    fn caret_px(&self, bar: Rect, clip: Rect) -> Rect {
        let mut b = bar.scaled(self.s);
        if b.x >= self.rect.right() * self.s && b.x < self.dev.right() {
            b.x = self.dev.right().min(clip.right() * self.s - b.width);
        }
        b
    }

    /// Invert a caret `bar` where [`Highlight::caret`] puts it.
    pub fn invert_caret(&self, c: &mut Canvas, bar: Rect, clip: Rect) {
        let b = self.caret_px(bar, clip);
        c.in_device_px(|c| c.invert_rect(b));
    }

    /// The logical pixels holding the caret `bar` as [`Highlight::invert_caret`] draws it within
    /// `clip`: the damage a blink reports.
    pub fn caret(&self, bar: Rect, clip: Rect) -> Rect {
        let (b, s) = (self.caret_px(bar, clip), self.s);
        let (x0, x1) = (b.x.div_euclid(s), (b.right() + s - 1).div_euclid(s));
        Rect::new(x0, bar.y, x1 - x0, bar.height)
    }
}

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
    /// Every glyph from this face, a char it lacks as its notdef: a font file's own sample.
    own_glyphs: bool,
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
    /// The fixed-pitch face plain text is set in at `px`: Lucida Console drawn hinted monochrome,
    /// else Liberation Mono or DejaVu Sans Mono antialiased, as a fallback mono blotches
    /// monochrome at 13 px.
    pub fn monospace(px: u32) -> TextRenderer {
        let cfg = |antialias| FontConfig {
            family: vec![
                "Lucida Console".into(),
                "Liberation Mono".into(),
                "DejaVu Sans Mono".into(),
            ],
            size_px: px,
            antialias,
            italic: false,
            full_hinting: false,
            shell_dlg: None,
        };
        let mono = TextRenderer::load(&cfg(false));
        if mono.has_face("Lucida Console") {
            mono
        } else {
            TextRenderer::load(&cfg(true))
        }
    }

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
            own_glyphs: false,
        }
    }

    /// Face `index` of the font file at `path`, antialiased, drawing only its own glyphs; `None`
    /// when FreeType cannot open it.
    pub fn from_file(path: &std::path::Path, index: u32) -> Option<TextRenderer> {
        let lib = Library::init().ok()?;
        let face = match lib.new_face(path.as_os_str(), index as isize) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("FreeType could not open {}: {e}", path.display());
                return None;
            }
        };
        let loaded = LoadedFace {
            family: face.family_name().unwrap_or_default(),
            italic: false,
            stray: None,
            face,
        };
        let mut t = TextRenderer::with_faces(Some(loaded.clone()), Some(loaded), true);
        t.own_glyphs = true;
        Some(t)
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

    /// The characters the regular face maps, in code point order; none without a face.
    pub fn chars(&self) -> Vec<char> {
        let Some(face) = self.face(false) else {
            return Vec::new();
        };
        face.chars()
            .filter_map(|(code, _)| char::from_u32(code as u32))
            .collect()
    }

    pub fn has_face(&self, family: &str) -> bool {
        self.regular.as_ref().is_some_and(|f| f.family == family)
    }

    #[cfg(test)]
    pub(crate) fn is_italic(&self) -> bool {
        self.regular.as_ref().is_some_and(|f| f.italic)
    }

    fn loaded(&self, bold: bool) -> Option<&LoadedFace> {
        let f = if bold { &self.bold } else { &self.regular };
        f.as_ref()
    }

    fn face(&self, bold: bool) -> Option<&Face> {
        self.loaded(bold).map(|l| &l.face)
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
            None if self.own_glyphs || no_fallback(ch) => None,
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
        if fallback.is_none()
            && let Some(stray) = self.loaded(bold).and_then(|l| l.stray.as_deref())
        {
            crate::pins::stray(stray);
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
                let family = face.family_name().unwrap_or_default();
                if let Some(stray) = fonts::stray_note(path, &family, || {
                    let ui = self.loaded(bold).map_or("", |l| &l.family);
                    format!("U+{:04X}, which {ui} lacks", ch as u32)
                }) {
                    crate::pins::stray(&stray);
                }
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

    /// `rect` grown to a line's ascent and descent at `size_px` where it is shorter, its baseline
    /// kept: a row shorter than the font's line (a 13 px row rounded at 125%, a radio's row at
    /// scale 2) draws its text without cutting the ink.
    pub fn line_box(&self, rect: Rect, bold: bool, size_px: u32) -> Rect {
        let Some((ascent, descent, _)) = self.line_metrics(bold, size_px) else {
            return rect;
        };
        let line = ascent - descent;
        match self.baseline(rect, bold, size_px) {
            Some(baseline) if rect.height < line => {
                Rect::new(rect.x, baseline - ascent, rect.width, line)
            }
            _ => rect,
        }
    }

    /// [`TextRenderer::line_box`] grown evenly above and below, so its baseline stays, until it
    /// holds every ink row `text` draws on a canvas at `scale`: a glyph that hangs past the
    /// font's descent (Tahoma's `|` at 14 and 22 px) keeps its ink.
    pub fn ink_box(&self, rect: Rect, text: &str, bold: bool, size_px: u32, scale: i32) -> Rect {
        let r = self.line_box(rect, bold, size_px);
        let Some(baseline) = self.baseline(r, bold, size_px) else {
            return r;
        };
        let s = scale.max(1);
        let (mut above, mut below) = (0, 0);
        for ch in text.chars() {
            let rows = self.with_glyph(ch, bold, size_px, s as u32, |g| {
                g.ink_rows()
                    .map(|(t, b)| (baseline * s - g.top + t, baseline * s - g.top + b))
            });
            if let Some(Some((top, bottom))) = rows {
                above = above.max(r.y * s - top);
                below = below.max(bottom - r.bottom() * s);
            }
        }
        let grow = (above.max(below).max(0) + s - 1) / s;
        Rect::new(r.x, r.y - grow, r.width, r.height + 2 * grow)
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

    /// `text` cut to end in "..." within `width`; only the dots that fit when even "..." doesn't.
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
        if dots > width {
            let dot = self.advance('.', bold, size_px).max(1);
            return ".".repeat((width / dot).clamp(0, 3) as usize).into();
        }
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

    /// `text` cut to fit `width` with "..." before its last `keep` bytes, which stay whole; once no
    /// character of the start fits, it is cut from the start instead ("...end").
    pub fn ellipsize_keeping<'t>(
        &self,
        text: &'t str,
        keep: usize,
        width: i32,
        bold: bool,
        size_px: u32,
    ) -> std::borrow::Cow<'t, str> {
        let at = text.len().saturating_sub(keep);
        let dots = self.measure("...", bold, size_px);
        if keep == 0
            || !text.is_char_boundary(at)
            || dots > width
            || self.measure(text, bold, size_px) <= width
        {
            return self.ellipsize(text, width, bold, size_px);
        }
        let (head, tail) = text.split_at(at);
        let room = width - dots - self.measure(tail, bold, size_px);
        let (mut used, mut end) = (0, 0);
        for (i, ch) in head.char_indices() {
            used += self.advance(ch, bold, size_px);
            if used > room {
                break;
            }
            end = i + ch.len_utf8();
        }
        let start = head[..end].trim_end_matches(['.', ' ']);
        if !start.is_empty() {
            return format!("{start}...{tail}").into();
        }
        let (mut used, mut from) = (dots, text.len());
        for (i, ch) in text.char_indices().rev() {
            used += self.advance(ch, bold, size_px);
            if used > width {
                break;
            }
            from = i;
        }
        format!("...{}", text[from..].trim_start()).into()
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

    /// [`TextRenderer::draw`] onto a translucent canvas, as [`TextRenderer::draw_run_over`].
    #[expect(clippy::too_many_arguments)]
    pub fn draw_over(
        &self,
        canvas: &mut Canvas,
        rect: Rect,
        text: &str,
        bold: bool,
        color: Color,
        align: Align,
        size_px: u32,
    ) {
        let Some(baseline) = self.baseline(rect, bold, size_px) else {
            return;
        };
        let width = match align {
            Align::Left => 0,
            Align::Center | Align::Right => self.measure(text, bold, size_px),
        };
        let pen = align.pen(rect, width);
        self.draw_run_over(canvas, pen, baseline, rect, text, bold, color, size_px);
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
        self.draw_line(
            canvas, pen, baseline, clip, rect, text, bold, color, size_px,
        );
    }

    /// Draw `text` from `rect`'s x on its baseline, clipped to `clip` and `line`, with glyphs kept
    /// inside `line` as [`TextRenderer::draw_run`] keeps them inside its clip. A row with room left
    /// of its pen passes that room in `line`, so at scale 2 its first glyph moves only at the
    /// room's edge. Draws that share `line` place every glyph alike.
    #[expect(clippy::too_many_arguments)]
    pub fn draw_in(
        &self,
        canvas: &mut Canvas,
        rect: Rect,
        line: Rect,
        clip: Rect,
        text: &str,
        bold: bool,
        color: Color,
        size_px: u32,
    ) {
        let Some(baseline) = self.baseline(rect, bold, size_px) else {
            return;
        };
        let clip = line.intersect(&clip);
        if !clip.is_empty() {
            self.draw_line(
                canvas, rect.x, baseline, clip, line, text, bold, color, size_px,
            );
        }
    }

    /// Draw `label` with its access key underlined (`&File` underlines F), the underline `s` px
    /// under the baseline and `s` px thick.
    #[expect(clippy::too_many_arguments)]
    pub fn draw_marked(
        &self,
        c: &mut Canvas,
        r: Rect,
        label: &str,
        bold: bool,
        color: Color,
        align: Align,
        px: u32,
        s: i32,
    ) {
        let (plain, mnemonic) = split_mnemonic(label);
        let Some(m) = mnemonic else {
            self.draw(c, r, &plain, bold, color, align, px);
            return;
        };
        let Some(baseline) = self.baseline(r, bold, px) else {
            return;
        };
        let (dx, w, total) = self.span_and_measure(&plain, m, bold, px);
        let start = align.pen(r, total);
        self.draw_run(c, start, baseline, r, &plain, bold, color, px);
        let cs = c.scale();
        let nudge = plain
            .chars()
            .nth(m)
            .filter(|_| cs > 1)
            .map_or(0, |ch| self.nudge(ch, bold, px, cs, start + dx, r));
        let ul = Rect::new(start + dx, baseline + s, w, s).scaled(cs);
        let ul = Rect::new(ul.x + nudge, ul.y, ul.width, ul.height);
        c.in_device_px(|c| c.fill_rect(ul.intersect(&r.scaled(cs)), color));
    }

    /// How far [`TextRenderer::draw_run`] moves `ch` drawn at scale `s` from pen x `pen` in `line`.
    fn nudge(&self, ch: char, bold: bool, size_px: u32, s: i32, pen: i32, line: Rect) -> i32 {
        let scaled = s as u32;
        if self.with_glyph(ch, bold, size_px, scaled, |g| g.boxed_in(pen, s, line)) != Some(false) {
            return 0;
        }
        let one = self
            .with_glyph(ch, bold, size_px, 1, |g| g.ink_span())
            .flatten();
        self.with_glyph(ch, bold, size_px, scaled, |g| {
            g.nudge_into(one, pen, s, line)
        })
        .unwrap_or(0)
    }

    /// The highlight `hl` of bytes `sel` of `text`, drawn from pen x `pen` in `line`, at canvas
    /// scale `s`. Its device pixels reach out to the selected glyphs' ink: up and down within
    /// `line`'s rows, as a descender can ink past the font's line, and at scale 2 sideways on a
    /// side where that ink fits `hl` at scale 1, as a glyph can ink a device column past its
    /// advance at `s`.
    #[expect(clippy::too_many_arguments)]
    pub fn highlight<'a>(
        &self,
        hl: Rect,
        text: &'a str,
        sel: std::ops::Range<usize>,
        pen: i32,
        line: Rect,
        bold: bool,
        size_px: u32,
        s: i32,
    ) -> Highlight<'a> {
        let mut h = Highlight {
            rect: hl,
            dev: hl.scaled(s),
            s,
            pen: pen + self.measure(&text[..sel.start], bold, size_px),
            baseline: self.baseline(line, bold, size_px),
            line,
            text: &text[sel],
            bold,
            size_px,
        };
        let join = |a: Option<(i32, i32)>, (l, r): (i32, i32)| {
            Some(a.map_or((l, r), |(al, ar)| (al.min(l), ar.max(r))))
        };
        let (mut one, mut ink, mut rows, mut at) = (None, None, None, h.pen);
        for ch in h.text.chars() {
            if s > 1
                && let Some((l, r)) = self
                    .with_glyph(ch, bold, size_px, 1, |g| g.ink_span())
                    .flatten()
            {
                one = join(one, (at + l, at + r));
            }
            let nudge = if s > 1 {
                self.nudge(ch, bold, size_px, s, at, line)
            } else {
                0
            };
            let dx = s * at + nudge;
            let drawn = self.with_glyph(ch, bold, size_px, s as u32, |g| {
                let top = |(t, b): (i32, i32)| (t - g.top, b - g.top);
                (g.drawn_span(), g.drawn_rows().map(top))
            });
            if let Some((span, drawn_rows)) = drawn {
                if let Some((l, r)) = span {
                    ink = join(ink, (dx + l, dx + r));
                }
                if let Some(r) = drawn_rows {
                    rows = join(rows, r);
                }
            }
            at += self.advance(ch, bold, size_px);
        }
        if let (Some(base), Some((t, b))) = (h.baseline, rows) {
            let y0 = h.dev.y.min((s * base + t).max(s * line.y));
            let y1 = h.dev.bottom().max((s * base + b).min(s * line.bottom()));
            h.dev = Rect::new(h.dev.x, y0, h.dev.width, y1 - y0);
        }
        let (Some((l1, r1)), Some((l, r))) = (one, ink) else {
            return h;
        };
        let (mut x0, mut x1) = (h.dev.x, h.dev.right());
        if l1 >= hl.x {
            x0 = x0.min(l.max(s * line.x));
        }
        if r1 <= hl.right() {
            x1 = x1.max(r.min(s * line.right()));
        }
        h.dev = Rect::new(x0, h.dev.y, x1 - x0, h.dev.height);
        h
    }

    /// Draw `h`'s selected glyphs in `color` where their ink passes `h.rect` into `h.dev`, within
    /// `clip`. Only the selected glyphs draw there, so a neighbour's ink in those device pixels
    /// keeps the text colour and a pixel both ink takes `color`.
    pub fn draw_overhang(&self, c: &mut Canvas, h: &Highlight, clip: Rect, color: Color) {
        let (s, r, d) = (h.s, h.rect, h.dev);
        let Some(baseline) = h.baseline.filter(|_| d != r.scaled(s)) else {
            return;
        };
        let (x0, y0) = (d.x.div_euclid(s), d.y.div_euclid(s));
        let x1 = (d.right() + s - 1).div_euclid(s);
        let y1 = (d.bottom() + s - 1).div_euclid(s);
        let ring = [
            Rect::new(x0, y0, r.x - x0, y1 - y0),
            Rect::new(r.right(), y0, x1 - r.right(), y1 - y0),
            Rect::new(r.x, y0, r.width, r.y - y0),
            Rect::new(r.x, r.bottom(), r.width, y1 - r.bottom()),
        ];
        for strip in ring.map(|strip| strip.intersect(&clip)) {
            if !strip.is_empty() {
                let (bold, px) = (h.bold, h.size_px);
                self.draw_line(c, h.pen, baseline, strip, h.line, h.text, bold, color, px);
            }
        }
    }

    /// Draw `text` from pen x `pen` on `baseline`, clipped to `clip`. At canvas scale `s` glyphs
    /// rasterize at `s * size_px` but the pen moves by the `size_px` advances, keeping the logical
    /// width carets are measured against. A glyph there can ink wider than `s` times itself at
    /// `size_px`, so one whose ink fits `clip` at scale 1 but would leave it moves back inside.
    #[expect(clippy::too_many_arguments)]
    pub fn draw_run(
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
        self.draw_line(
            canvas, pen, baseline, clip, clip, text, bold, color, size_px,
        );
    }

    /// [`TextRenderer::draw_run`] clipped to `clip`, with glyphs kept inside `line`, the rect the
    /// text was laid in, and ink outside it noted for [`crate::pins::clipped_ink`]. Draws that share
    /// `line` place every glyph alike whatever their clips.
    #[expect(clippy::too_many_arguments)]
    fn draw_line(
        &self,
        canvas: &mut Canvas,
        mut pen: i32,
        baseline: i32,
        clip: Rect,
        line: Rect,
        text: &str,
        bold: bool,
        color: Color,
        size_px: u32,
    ) {
        let s = canvas.scale();
        let watch = crate::pins::watching_clips().then(|| line.scaled(s));
        let mut cut = (0, 0);
        for ch in text.chars() {
            let logical = (s > 1).then(|| self.advance(ch, bold, size_px));
            let nudge = if s > 1 {
                self.nudge(ch, bold, size_px, s, pen, line)
            } else {
                0
            };
            let drawn = self.with_glyph(ch, bold, size_px, s as u32, |g| {
                let gx = pen * s + g.left + nudge;
                let gy = baseline * s - g.top;
                if let Some(keep) = watch {
                    let (rows, cols) = g.ink_cut(gx, gy, keep);
                    cut = (cut.0 + rows, cut.1 + cols);
                }
                canvas.blit_mask(gx, gy, g.width, g.height, &g.bitmap, color, g.smooth, clip);
                g.advance
            });
            pen += logical.or(drawn).unwrap_or(0);
        }
        if cut != (0, 0) {
            crate::pins::clipped(crate::pins::Cut {
                text: text.into(),
                rect: line,
                px: size_px,
                rows: cut.0,
                cols: cut.1,
            });
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
