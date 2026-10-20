//! Font lookup (fontdb) and hinted glyph rasterization (FreeType). With `antialias` off glyphs load
//! as `FT_LOAD_TARGET_MONO` (bytecode hinter, monochrome); on, hinted 8-bit coverage is blended and
//! `full_hinting` fits stems across as well as down.

use crate::fontfallback::{
    FaceFile, FallbackCache, FallbackDraw, fallback_draw, fallback_strike, fontconfig_face_for,
};
use crate::raster::{Canvas, Rect};
use crate::theme::{Color, FontConfig};
use fontdb::{Database, Family, Query, Weight};
use freetype::face::LoadFlag;
use freetype::{Face, Library};
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

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

fn find_face(
    db: &Database,
    families: &[String],
    generic: bool,
    weight: Weight,
    style: fontdb::Style,
) -> Option<(PathBuf, u32, String, fontdb::Style)> {
    let mut fams: Vec<Family> = families.iter().map(|f| Family::Name(f.as_str())).collect();
    if generic {
        fams.push(Family::SansSerif);
    }
    let id = db.query(&Query {
        families: &fams,
        weight,
        style,
        ..Query::default()
    })?;
    let (source, index) = db.face_source(id)?;
    let path = match source {
        fontdb::Source::File(path) => path,
        fontdb::Source::SharedFile(path, _) => path,
        fontdb::Source::Binary(_) => return None,
    };
    let info = db.face(id)?;
    let family = info
        .families
        .first()
        .map(|(name, _)| name.clone())
        .unwrap_or_default();
    Some((path, index, family, info.style))
}

fn load_face(
    lib: &Library,
    db: &Database,
    families: &[String],
    generic: bool,
    weight: Weight,
    style: fontdb::Style,
) -> Option<LoadedFace> {
    let (path, index, family, matched_style) = find_face(db, families, generic, weight, style)?;
    match lib.new_face(&path, index as isize) {
        Ok(face) => {
            log::info!(
                "font {families:?} weight {weight:?} style {style:?}: {family} ({})",
                path.display()
            );
            Some(LoadedFace {
                face,
                family,
                italic: matched_style != fontdb::Style::Normal,
            })
        }
        Err(e) => {
            log::warn!("FreeType could not open {}: {e}", path.display());
            None
        }
    }
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

/// Switch `lib`'s TrueType hinter to full mode (stems fit across as well as down in antialiased
/// text); per-renderer library.
fn full_hinting(lib: &Library) {
    let version: u32 = 35;
    // SAFETY: `lib.raw()` is a live library for the call; the module and
    // property names are NUL-terminated literals, and "interpreter-version"
    // takes a pointer to an FT_UInt that FreeType only reads.
    let err = unsafe {
        freetype::ffi::FT_Property_Set(
            lib.raw(),
            c"truetype".as_ptr(),
            c"interpreter-version".as_ptr(),
            (&raw const version).cast(),
        )
    };
    if err != 0 {
        log::warn!("FreeType refused full hinting (error {err}); titles stay lightly hinted");
    }
}

/// The system font database, shared by every renderer and cached on disk ([`crate::fontcache`]);
/// read again when it lacks the preferred family.
pub(crate) fn system_fonts(families: &[String]) -> Arc<Database> {
    static FONTS: Mutex<Option<Arc<Database>>> = Mutex::new(None);
    let mut fonts = FONTS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(db) = fonts.as_ref().filter(|db| !needs_scan(db, families)) {
        return db.clone();
    }
    let mut db = crate::fontcache::system_database();
    if let Some(dir) = test_fonts_dir() {
        load_test_fonts(&mut db, &dir);
    }
    fonts.insert(Arc::new(db)).clone()
}

/// The directory `REPRISE_TEST_FONTS` names (filled by `scripts/test-fonts.sh`), whose faces
/// replace the system's faces of the same family so pixel tests render alike on every machine.
fn test_fonts_dir() -> Option<PathBuf> {
    std::env::var_os("REPRISE_TEST_FONTS")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn load_test_fonts(db: &mut Database, dir: &std::path::Path) {
    let mut test = Database::new();
    test.load_fonts_dir(dir);
    let families: Vec<String> = test
        .faces()
        .flat_map(|f| f.families.iter().map(|(name, _)| name.clone()))
        .collect();
    let shadowed: Vec<fontdb::ID> = db
        .faces()
        .filter(|f| f.families.iter().any(|(name, _)| families.contains(name)))
        .map(|f| f.id)
        .collect();
    for id in shadowed {
        db.remove_face(id);
    }
    db.load_fonts_dir(dir);
}

fn needs_scan(db: &Database, families: &[String]) -> bool {
    families.first().is_some_and(|want| {
        !db.faces()
            .any(|f| f.families.iter().any(|(name, _)| name == want))
    })
}

/// Whether the default UI face is the Tahoma `REPRISE_TEST_FONTS` supplies, which the pixel tests
/// are pinned to; they skip without it. Cached.
pub fn test_tahoma() -> bool {
    static FOUND: OnceLock<bool> = OnceLock::new();
    *FOUND.get_or_init(|| {
        let Some(dir) = test_fonts_dir() else {
            return false;
        };
        let families = FontConfig::default().family;
        let db = system_fonts(&families);
        let dir = dir.canonicalize().unwrap_or(dir);
        families.first().is_some_and(|f| f == "Tahoma")
            && find_face(&db, &families, false, Weight::NORMAL, fontdb::Style::Normal)
                .is_some_and(|(path, _, family, _)| family == "Tahoma" && path.starts_with(&dir))
    })
}

/// The second dialog font `spec` names, a family (exact match) or an absolute font file path;
/// `None` with a warning if not found.
fn load_shell_dlg(spec: &str, cfg: &FontConfig) -> Option<TextRenderer> {
    let lib = Library::init().ok()?;
    if cfg.full_hinting {
        full_hinting(&lib);
    }
    let (regular, bold) = if spec.starts_with('/') {
        match lib.new_face(spec, 0) {
            Ok(face) => {
                log::info!("second dialog font: {spec}");
                select_any_charmap(&face);
                let loaded = |face: Face| LoadedFace {
                    family: face.family_name().unwrap_or_default(),
                    italic: false,
                    face,
                };
                (Some(loaded(face.clone())), Some(loaded(face)))
            }
            Err(e) => {
                log::warn!("second dialog font {spec}: {e}; dialogs keep the UI font");
                return None;
            }
        }
    } else {
        let families = [spec.to_string()];
        let db = system_fonts(&families);
        let normal = fontdb::Style::Normal;
        let regular = load_face(&lib, &db, &families, false, Weight::NORMAL, normal);
        let bold = load_face(&lib, &db, &families, false, Weight::BOLD, normal)
            .or_else(|| load_face(&lib, &db, &families, false, Weight::NORMAL, normal));
        (regular, bold)
    };
    if regular.is_none() {
        log::warn!("second dialog font {spec:?} not found; dialogs keep the UI font");
        return None;
    }
    Some(TextRenderer::with_faces(regular, bold, cfg.antialias))
}

/// Select `face`'s first charmap when FreeType picked none, as for a `.fon` bitmap font indexed by
/// code page; ASCII agrees, anything else may fall to notdef.
fn select_any_charmap(face: &Face) {
    if face.raw().charmap.is_null()
        && face.num_charmaps() > 0
        && let Err(e) = face.set_charmap(&face.get_charmap(0))
    {
        log::warn!("second dialog font: no character map selected ({e})");
    }
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
                let regular = load_face(lib, &db, &cfg.family, true, Weight::NORMAL, style);
                let bold = load_face(lib, &db, &cfg.family, true, Weight::BOLD, style)
                    .or_else(|| load_face(lib, &db, &cfg.family, true, Weight::NORMAL, style));
                (regular, bold)
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
        let strike = fixed_sizes(face)
            .iter()
            .map(|s| ((s.y_ppem + 32) >> 6) as u32)
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
                    let p = fallback_strike(
                        fixed_sizes(&fb)
                            .iter()
                            .map(|s| ((s.y_ppem + 32) >> 6) as u32),
                        size_px,
                    )?;
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
            let adv = self
                .with_glyph(ch, bold, size_px, 1, |g| g.advance)
                .unwrap_or(0);
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
        let mut ink = Canvas::new(clip.width as u32, clip.height as u32);
        let local = Rect::new(0, 0, clip.width, clip.height);
        let white = Color::rgb(0xFF, 0xFF, 0xFF);
        self.draw_run(
            &mut ink,
            pen - clip.x,
            baseline - clip.y,
            local,
            text,
            bold,
            white,
            size_px,
        );
        let coverage: Vec<u8> = ink.pixels.iter().map(|&p| (p >> 16) as u8).collect();
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
pub fn wrap(text: &TextRenderer, body: &str, max_w: i32, size: u32) -> Vec<String> {
    wrap_bold(text, body, max_w, size, false)
}

pub fn wrap_bold(
    text: &TextRenderer,
    body: &str,
    max_w: i32,
    size: u32,
    bold: bool,
) -> Vec<String> {
    let space = text.advance(' ', bold, size);
    wrap_by(body, max_w, space, |w| text.measure(w, bold, size))
}

pub fn wrap_chars(body: &str, cols: usize) -> Vec<String> {
    wrap_by(body, cols as i32, 1, |w| w.chars().count() as i32)
}

fn wrap_by(body: &str, max_w: i32, space: i32, measure: impl Fn(&str) -> i32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in body.split('\n') {
        let mut line = String::new();
        let mut line_w = 0;
        for word in para.split_whitespace() {
            let word_w = measure(word);
            if !line.is_empty() && line_w + space + word_w <= max_w {
                line.push(' ');
                line.push_str(word);
                line_w += space + word_w;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            let mut rest = word;
            let mut rest_w = word_w;
            while rest_w > max_w {
                let cut = word_break(rest, max_w, &measure);
                if cut == rest.len() {
                    break;
                }
                lines.push(rest[..cut].to_string());
                rest = &rest[cut..];
                rest_w = measure(rest);
            }
            line.push_str(rest);
            line_w = rest_w;
        }
        lines.push(line);
    }
    if lines.iter().all(|l| l.is_empty()) {
        lines.clear();
    }
    lines
}

/// The byte length of the head of `word` that goes on a line `max_w` wide:
/// through the last `/`, `\`, `-`, `_` or `.` past its first char that fits,
/// else the widest prefix, never less than one char.
fn word_break(word: &str, max_w: i32, measure: impl Fn(&str) -> i32) -> usize {
    let mut fits = 0;
    let mut after_sep = 0;
    for (i, c) in word.char_indices() {
        let end = i + c.len_utf8();
        if fits > 0 && measure(&word[..end]) > max_w {
            break;
        }
        fits = end;
        if i > 0 && matches!(c, '/' | '\\' | '-' | '_' | '.') {
            after_sep = end;
        }
    }
    if after_sep > 0 { after_sep } else { fits }
}

pub fn split_mnemonic(label: &str) -> (String, Option<usize>) {
    let mut out = String::with_capacity(label.len());
    let mut mnemonic = None;
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            match chars.peek() {
                Some('&') => {
                    chars.next();
                    out.push('&');
                }
                Some(_) if mnemonic.is_none() => mnemonic = Some(out.chars().count()),
                _ => {}
            }
        } else {
            out.push(c);
        }
    }
    (out, mnemonic)
}

/// `s` as a label [`split_mnemonic`] reads back unchanged: every `&` doubled,
/// so a file or program name never turns a letter into an access key.
pub fn escape_mnemonic(s: &str) -> String {
    s.replace('&', "&&")
}

pub fn access_key(label: &str) -> Option<char> {
    let (plain, m) = split_mnemonic(label);
    m.and_then(|m| plain.chars().nth(m))
        .and_then(|c| c.to_lowercase().next())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_mnemonic_marks_one_char_and_unescapes_ampersand() {
        assert_eq!(split_mnemonic("&Programs"), ("Programs".into(), Some(0)));
        assert_eq!(split_mnemonic("Sear&ch"), ("Search".into(), Some(4)));
        assert_eq!(
            split_mnemonic("Files && Folders"),
            ("Files & Folders".into(), None)
        );
        assert_eq!(split_mnemonic("&A && &B"), ("A & B".into(), Some(0)));
        assert_eq!(split_mnemonic("Plain"), ("Plain".into(), None));
        assert_eq!(split_mnemonic("Trailing&"), ("Trailing".into(), None));
        assert_eq!(access_key("&Always use"), Some('a'));
        assert_eq!(access_key("Sear&ch"), Some('c'));
        assert_eq!(access_key("Plain"), None);
    }

    #[test]
    fn an_escaped_name_reads_back_whole_with_no_access_key() {
        for name in ["R&D notes.txt", "&", "a&&b", "Plain", ""] {
            let label = escape_mnemonic(name);
            assert_eq!(split_mnemonic(&label), (name.to_string(), None), "{name}");
            assert_eq!(access_key(&label), None, "{name}");
        }
    }

    #[test]
    fn ellipsize_keeps_fitting_text_and_trims_the_rest() {
        let r = TextRenderer::load(&FontConfig::default());
        let full = r.measure("Printers", false, 11);
        assert_eq!(r.ellipsize("Printers", full, false, 11), "Printers");
        let short = r.ellipsize("Printers", full - 20, false, 11);
        assert!(short.ends_with("..."), "{short}");
        assert!(short.len() < "Printers".len());
        assert!(r.measure(&short, false, 11) <= full - 20);
        for width in 0..r.measure("Recycle Bin", false, 11) {
            let short = r.ellipsize("Recycle Bin", width, false, 11);
            assert!(
                !short.ends_with(" ..."),
                "no space before the dots: {short}"
            );
        }
        assert_eq!(r.ellipsize("abc", 0, false, 11), "...");
    }
    use crate::raster::{Canvas, Rect};
    use crate::theme::{Color, FontConfig};

    fn renderer() -> TextRenderer {
        TextRenderer::load(&FontConfig::default())
    }

    #[test]
    fn line_metrics_are_memoised_per_size() {
        let r = renderer();
        let rect = Rect::new(0, 0, 100, 20);
        let a = r.baseline(rect, false, 11);
        let b = r.baseline(rect, true, 22);
        assert_eq!(r.metrics.borrow().len(), 2);
        assert_eq!(r.baseline(rect, false, 11), a);
        assert_eq!(r.baseline(rect, true, 22), b);
        assert_eq!(r.metrics.borrow().len(), 2);
    }

    #[test]
    fn without_a_second_dialog_font_every_dialog_takes_the_ui_font() {
        let with = |spec: Option<&str>| {
            TextRenderer::load(&FontConfig {
                shell_dlg: spec.map(String::from),
                ..FontConfig::default()
            })
        };
        for spec in [None, Some("No Such Font Family"), Some("/no/such/dlg.ttf")] {
            let r = with(spec);
            assert!(!r.has_shell_dlg(), "{spec:?}");
            assert!(std::ptr::eq(r.dialog(DialogFont::ShellDlg), &r), "{spec:?}");
        }
    }

    #[test]
    fn the_second_dialog_font_loads_by_family_or_file_on_the_ui_baseline() {
        if !test_tahoma() {
            return;
        }
        let db = system_fonts(&[]);
        let Some((family, path)) = db.faces().find_map(|f| {
            let family = &f.families.first()?.0;
            match &f.source {
                fontdb::Source::File(p) if family != "Tahoma" && f.index == 0 => {
                    Some((family.clone(), p.to_string_lossy().into_owned()))
                }
                _ => None,
            }
        }) else {
            return;
        };
        for spec in [family.clone(), path] {
            let r = TextRenderer::load(&FontConfig {
                shell_dlg: Some(spec.clone()),
                ..FontConfig::default()
            });
            assert!(r.has_shell_dlg(), "{spec}");
            assert!(r.has_face("Tahoma"), "the UI font stays: {spec}");
            let second = r.dialog(DialogFont::ShellDlg);
            assert!(second.has_face(&family), "{spec}");
            assert!(std::ptr::eq(r.dialog(DialogFont::Ui), &r));
            for h in 11..24 {
                let rect = Rect::new(0, 0, 50, h);
                for bold in [false, true] {
                    assert_eq!(
                        second.baseline(rect, bold, 11),
                        r.baseline(rect, bold, 11),
                        "{spec} in a {h} px rect"
                    );
                }
            }
        }
    }

    #[test]
    fn a_bitmap_font_file_loads_as_the_second_dialog_font() {
        let path = "/usr/share/wine/fonts/sserife.fon";
        if !std::path::Path::new(path).exists() {
            return;
        }
        let r = TextRenderer::load(&FontConfig {
            shell_dlg: Some(path.into()),
            ..FontConfig::default()
        });
        let second = r.dialog(DialogFont::ShellDlg);
        assert!(r.has_shell_dlg());
        assert_eq!(['E', 'N'].map(|c| second.advance(c, false, 11)), [7, 8]);
    }

    #[test]
    fn a_bitmap_font_doubles_its_strike_at_scale_2_and_else_takes_the_ui_font() {
        let path = "/usr/share/wine/fonts/sserife.fon";
        if !std::path::Path::new(path).exists() {
            return;
        }
        let r = TextRenderer::load(&FontConfig {
            shell_dlg: Some(path.into()),
            ..FontConfig::default()
        });
        let second = r.dialog(DialogFont::ShellDlg);
        let text = "Reprise Settings";
        let rect = Rect::new(0, 0, 120, 20);
        let draw = |font: &TextRenderer, size, scale| {
            let mut c = Canvas::with_scale(120, 20, scale);
            let ink = Color::rgb(255, 255, 255);
            font.draw(&mut c, rect, text, false, ink, Align::Left, size);
            c
        };
        let (one, two) = (draw(second, 11, 1), draw(second, 11, 2));
        let width = second.measure(text, false, 11);
        let right = (0..120)
            .filter(|&x| (0..20).any(|y| one.get(x, y) != 0))
            .max()
            .expect("the strike inks at scale 1")
            + 1;
        assert!(
            (width - 2..=width).contains(&right),
            "ink ends at {right}, measure gives {width}"
        );
        for y in 0..40 {
            for x in 0..240 {
                assert_eq!(
                    two.pixels[y * 240 + x],
                    one.pixels[y / 2 * 120 + x / 2],
                    "device pixel ({x}, {y}) at scale 2"
                );
            }
        }
        assert_eq!(second.measure(text, false, 12), r.measure(text, false, 12));
        for scale in [1, 2] {
            let ui = draw(&r, 12, scale);
            assert!(ui.pixels.iter().any(|&p| p != 0), "UI ink at scale {scale}");
            assert_eq!(draw(second, 12, scale).pixels, ui.pixels, "scale {scale}");
        }
    }

    #[test]
    fn a_stub_second_font_inks_its_pens() {
        let ui = renderer();
        let rect = Rect::new(0, 0, 60, 13);
        let Some(baseline) = ui.baseline(rect, false, 11) else {
            return;
        };
        let r = ui.with_shell_dlg(TextRenderer::stub(11, |c| if c == 'E' { 7 } else { 6 }));
        let second = r.dialog(DialogFont::ShellDlg);
        assert_eq!(second.measure("En d", false, 11), 25);
        let mut c = Canvas::new(60, 13);
        let ink = Color::rgb(255, 255, 255);
        second.draw(&mut c, rect, "En d", false, ink, Align::Left, 11);
        let dots: Vec<(i32, i32)> = (0..60)
            .flat_map(|x| (0..13).map(move |y| (x, y)))
            .filter(|&(x, y)| c.get(x, y) != 0)
            .collect();
        let row = baseline - 1;
        assert_eq!(dots, vec![(0, row), (7, row), (19, row)]);
    }

    #[test]
    fn only_the_second_dialog_font_leads() {
        let ui = renderer();
        assert_eq!(ui.lead(), 0);
        assert_eq!(ui.dialog(DialogFont::ShellDlg).lead(), 0);
        let r = ui.with_shell_dlg(TextRenderer::stub(11, |_| 6));
        assert_eq!(r.lead(), 0);
        assert_eq!(r.dialog(DialogFont::ShellDlg).lead(), 1);
    }

    #[test]
    fn measures_wider_for_longer_text() {
        let r = renderer();
        assert!(r.measure("Start", true, 11) > r.measure("S", true, 11));
        assert_eq!(r.measure("", true, 11), 0);
        assert!(r.measure("Start", false, 22) > r.measure("Start", false, 11));
    }

    #[test]
    fn has_face_reports_loaded_family() {
        let r = renderer();
        assert!(r.has_face("Tahoma") || r.has_face("DejaVu Sans"));
        assert!(!r.has_face("No Such Font Family"));
    }

    #[test]
    fn draws_pixels_inside_rect_only() {
        let r = renderer();
        let mut c = Canvas::new(60, 20);
        let rect = Rect::new(2, 2, 40, 16);
        r.draw(
            &mut c,
            rect,
            "Start",
            true,
            Color::rgb(255, 255, 255),
            Align::Left,
            11,
        );
        let inside = (0..60)
            .flat_map(|x| (0..20).map(move |y| (x, y)))
            .filter(|&(x, y)| c.get(x, y) != 0)
            .count();
        assert!(inside > 10, "expected glyph pixels");
        for x in 0..60 {
            for y in 0..20 {
                if c.get(x, y) != 0 {
                    assert!(rect.contains(x, y), "pixel outside clip at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn mono_mode_yields_solid_pixels() {
        let r = TextRenderer::load(&FontConfig {
            antialias: false,
            ..FontConfig::default()
        });
        let mut c = Canvas::new(60, 20);
        r.draw(
            &mut c,
            Rect::new(0, 0, 60, 20),
            "Wg",
            true,
            Color::rgb(255, 255, 255),
            Align::Left,
            11,
        );
        for p in &c.pixels {
            assert!(
                *p == 0 || *p == 0xFFFFFFFF,
                "unexpected blended pixel {p:#x}"
            );
        }
        // `blit_mask` thresholds when antialias is off, so the coverage FreeType returned must
        // itself be bilevel.
        let cache = r.cache.borrow();
        assert!(!cache.is_empty(), "expected rasterized glyphs");
        for g in cache.values() {
            assert!(
                g.bitmap.iter().all(|&b| b == 0 || b == 255),
                "mono coverage has a blended byte"
            );
        }
    }

    #[test]
    fn reprise_test_fonts_supplies_the_ui_tahoma() {
        if test_fonts_dir().is_some() {
            assert!(test_tahoma(), "REPRISE_TEST_FONTS holds no Tahoma");
        }
    }

    #[test]
    fn grayscale_mode_blends() {
        // Wine's Tahoma has bilevel bitmap strikes from 8 to 16 px but none at 14.
        let r = TextRenderer::load(&FontConfig {
            antialias: true,
            ..FontConfig::default()
        });
        let mut c = Canvas::new(60, 20);
        c.fill_rect(Rect::new(0, 0, 60, 20), Color::rgb(0, 0, 0));
        r.draw(
            &mut c,
            Rect::new(0, 0, 60, 20),
            "Wg",
            false,
            Color::rgb(255, 255, 255),
            Align::Left,
            14,
        );
        let partial = c.pixels.iter().any(|&p| p != 0xFF000000 && p != 0xFFFFFFFF);
        assert!(partial, "expected at least one antialiased pixel");
        assert!(
            r.cache
                .borrow()
                .values()
                .any(|g| g.bitmap.iter().any(|&b| b > 0 && b < 255)),
            "grayscale coverage is bilevel"
        );
    }

    #[test]
    fn italic_is_the_families_slanted_face_when_installed() {
        let cfg = FontConfig {
            family: vec!["DejaVu Sans".into()],
            ..FontConfig::default()
        };
        let t = TextRenderer::load(&cfg);
        if !t.has_face("DejaVu Sans") {
            return;
        }
        let slanted = t.italic();
        assert!(slanted.is_italic(), "DejaVu Sans Oblique");
        assert!(std::ptr::eq(slanted.italic(), slanted));
        let none = TextRenderer::fontless();
        assert!(std::ptr::eq(none.italic(), &none), "no config to load");
    }

    /// Tahoma has no italic face: its upright faces lean right, a stem's top ink right of
    /// its foot.
    #[test]
    fn italic_shears_a_family_without_a_slanted_face() {
        if !test_tahoma() {
            return;
        }
        let t = TextRenderer::load(&FontConfig::default());
        let slanted = t.italic();
        assert!(!std::ptr::eq(slanted, &t));
        assert!(!slanted.is_italic(), "no slanted face");
        let stem = |face: &TextRenderer| {
            let mut c = Canvas::new(20, 20);
            face.draw(
                &mut c,
                Rect::new(4, 2, 16, 16),
                "l",
                false,
                Color::rgb(0, 0, 0),
                Align::Left,
                11,
            );
            let ink: Vec<(i32, i32)> = (0..20)
                .flat_map(|y| (0..20).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) >> 24 != 0)
                .collect();
            let top = ink.iter().map(|p| p.1).min().unwrap();
            let foot = ink.iter().map(|p| p.1).max().unwrap();
            let left_at = |y: i32| ink.iter().filter(|p| p.1 == y).map(|p| p.0).min().unwrap();
            left_at(top) - left_at(foot)
        };
        assert_eq!(stem(&t), 0, "upright");
        assert!(stem(slanted) > 0, "leans right");
    }

    #[test]
    fn an_italic_config_loads_an_italic_face_when_one_exists() {
        let db = system_fonts(&[]);
        let mut slanted: Vec<&str> = db
            .faces()
            .filter(|f| f.style == fontdb::Style::Italic)
            .filter_map(|f| f.families.first().map(|(name, _)| name.as_str()))
            .collect();
        slanted.sort_unstable();
        let Some(&family) = slanted.first() else {
            return;
        };
        let cfg = FontConfig {
            family: vec![family.into()],
            italic: true,
            ..FontConfig::default()
        };
        let t = TextRenderer::load(&cfg);
        assert!(t.has_face(family), "{family} loads");
        assert!(t.is_italic(), "{family} loads its italic face");
    }

    #[test]
    fn center_alignment_moves_text_right() {
        let r = renderer();
        let mut left = Canvas::new(100, 20);
        let mut center = Canvas::new(100, 20);
        r.draw(
            &mut left,
            Rect::new(0, 0, 100, 20),
            "ab",
            false,
            Color::rgb(255, 255, 255),
            Align::Left,
            11,
        );
        r.draw(
            &mut center,
            Rect::new(0, 0, 100, 20),
            "ab",
            false,
            Color::rgb(255, 255, 255),
            Align::Center,
            11,
        );
        let first = |c: &Canvas| {
            (0..100)
                .find(|&x| (0..20).any(|y| c.get(x, y) != 0))
                .unwrap()
        };
        assert!(first(&center) > first(&left) + 20);
    }

    #[test]
    fn glyph_cache_is_reused() {
        let r = renderer();
        assert_eq!(r.cache.borrow().len(), 0);
        r.measure("aa", false, 11);
        assert_eq!(r.cache.borrow().len(), 1);
        r.measure("ab", true, 11);
        assert_eq!(r.cache.borrow().len(), 3);
        r.measure("a", false, 12);
        assert_eq!(r.cache.borrow().len(), 4);
    }

    #[test]
    fn tahoma_widths_match_hdmx() {
        let r = TextRenderer::load(&FontConfig {
            antialias: false,
            ..FontConfig::default()
        });
        if !test_tahoma() {
            return;
        }
        assert_eq!(r.measure("Start", false, 11), 24);
        assert_eq!(r.measure("Start", true, 11), 29);
    }

    fn missing_chars(r: &TextRenderer, n: usize) -> Vec<char> {
        let face = r.face(false).expect("a face");
        let v: Vec<char> = (0x10_0000u32..0x10_FFFD)
            .filter_map(char::from_u32)
            .filter(|&ch| face.get_char_index(ch as usize).is_none())
            .filter(|&ch| {
                r.fallback_face(ch, false, false).is_none()
                    && r.fallback_face(ch, true, false).is_none()
            })
            .take(n)
            .collect();
        assert_eq!(v.len(), n, "the faces cover the private use plane");
        v
    }

    #[test]
    fn a_char_the_ui_face_lacks_draws_from_a_fallback_face() {
        let r = TextRenderer::load(&FontConfig {
            antialias: true,
            ..FontConfig::default()
        });
        let face = r.face(false).expect("a face");
        if face.get_char_index('中' as usize).is_some()
            || crate::fontfallback::fontconfig_face_for('中', false, false).is_none()
        {
            return;
        }
        let draw = |text: &str| {
            let mut c = Canvas::new(40, 20);
            let white = Color::rgb(255, 255, 255);
            r.draw(
                &mut c,
                Rect::new(0, 0, 40, 20),
                text,
                false,
                white,
                Align::Left,
                11,
            );
            c.pixels
        };
        let han = draw("中");
        assert!(han.iter().any(|&p| p != 0), "the glyph inks");
        let graded = han.iter().any(|&p| p != 0 && (p >> 16) & 0xFF != 0xFF);
        assert!(graded, "a fallback glyph is smoothed");
        assert!(r.advance('中', false, 11) >= 8, "a full-width advance");
        let notdef = r.advance(missing_chars(&r, 1)[0], false, 11);
        assert_ne!(han, draw("\u{10FFFD}"), "not the notdef box ({notdef} px)");
    }

    #[test]
    fn mono_text_draws_a_fallback_char_from_a_bitmap_face_at_scale_1_only() {
        let r = TextRenderer::load(&FontConfig {
            antialias: false,
            ..FontConfig::default()
        });
        let face = r.face(false).expect("a face");
        if face.get_char_index('中' as usize).is_some()
            || crate::fontfallback::fontconfig_face_for('中', false, false).is_none()
        {
            return;
        }
        let bitmap = r.fallback_face('中', false, true).is_some_and(|f| {
            let strikes = fixed_sizes(&f)
                .iter()
                .map(|s| ((s.y_ppem + 32) >> 6) as u32);
            fallback_strike(strikes, 11).is_some()
        });
        let draw = |scale: i32| {
            let mut c = Canvas::with_scale(40, 20, scale);
            let white = Color::rgb(255, 255, 255);
            r.draw(
                &mut c,
                Rect::new(0, 0, 40, 20),
                "中",
                false,
                white,
                Align::Left,
                11,
            );
            c
        };
        let is_mono = |c: &Canvas| c.pixels.iter().all(|&p| p == 0 || p == 0xFFFF_FFFF);
        let (one, two) = (draw(1), draw(2));
        assert!(one.pixels.iter().any(|&p| p != 0), "the glyph inks");
        assert_eq!(
            is_mono(&one),
            bitmap,
            "mono from a bitmap face, else smoothed"
        );
        assert!(
            two.pixels.iter().any(|&p| p != 0),
            "the glyph inks at scale 2"
        );
        assert!(!is_mono(&two), "smoothed outline at scale 2");
    }

    /// A BDF face with one strike at `px` whose only glyph, 中, is a solid `px` x `px` block.
    fn block_strike_face(px: u32) -> (PathBuf, Face) {
        let row = format!("{:06X}", ((1u32 << px) - 1) << (24 - px));
        let rows = vec![row; px as usize].join("\n");
        let bdf = format!(
            "STARTFONT 2.1\nFONT -reprise-block-medium-r-normal--{px}-{d}-75-75-c-{d}-iso10646-1\n\
             SIZE {px} 75 75\nFONTBOUNDINGBOX {px} {px} 0 -2\nSTARTPROPERTIES 5\nPIXEL_SIZE {px}\n\
             FONT_ASCENT {a}\nFONT_DESCENT 2\nCHARSET_REGISTRY \"ISO10646\"\nCHARSET_ENCODING \"1\"\n\
             ENDPROPERTIES\nCHARS 1\nSTARTCHAR U+4E2D\nENCODING 20013\nSWIDTH 1000 0\n\
             DWIDTH {px} 0\nBBX {px} {px} 0 -2\nBITMAP\n{rows}\nENDCHAR\nENDFONT\n",
            d = px * 10,
            a = px - 2,
        );
        let path =
            std::env::temp_dir().join(format!("reprise-block-{px}-{}.bdf", std::process::id()));
        std::fs::write(&path, bdf).expect("write the BDF");
        let face = Library::init()
            .expect("FreeType")
            .new_face(&path, 0)
            .expect("FreeType opens the BDF");
        (path, face)
    }

    #[test]
    fn a_bitmap_strike_at_the_device_size_draws_only_for_an_output_at_scale_1() {
        let r = TextRenderer::load(&FontConfig {
            antialias: false,
            ..FontConfig::default()
        });
        if r.face(false)
            .is_none_or(|f| f.get_char_index('中' as usize).is_some())
        {
            return;
        }
        let (path, face) = block_strike_face(22);
        let at_2 = r.for_output(2);
        for t in [&r, at_2] {
            t.fallback_mono.borrow_mut().pick(
                '中',
                false,
                |f: &Face, c| f.get_char_index(c as usize).is_some(),
                |_, _| Some((path.clone(), 0)),
                |_| Some(face.clone()),
            );
        }
        let draw = |t: &TextRenderer| {
            let mut c = Canvas::new(40, 30);
            let white = Color::rgb(255, 255, 255);
            t.draw(
                &mut c,
                Rect::new(0, 0, 40, 30),
                "中",
                false,
                white,
                Align::Left,
                22,
            );
            c
        };
        let lit = |c: &Canvas| c.pixels.iter().filter(|&&p| p == 0xFFFF_FFFF).count();
        let (one, two) = (draw(&r), draw(at_2));
        std::fs::remove_file(&path).ok();
        assert_eq!(
            lit(&one),
            22 * 22,
            "the 22 px strike's block at output scale 1"
        );
        assert!(
            two.pixels.iter().any(|&p| p != 0),
            "the glyph inks at output scale 2"
        );
        assert_ne!(lit(&two), 22 * 22, "no strike block at output scale 2");
        if crate::fontfallback::fontconfig_face_for('中', false, false).is_some() {
            assert!(
                two.pixels.iter().any(|&p| p != 0 && p != 0xFFFF_FFFF),
                "smoothed outline at output scale 2"
            );
        }
        assert!(std::ptr::eq(r.for_output(1), &r), "itself at scale 1");
    }

    #[test]
    fn missing_chars_share_one_notdef_entry() {
        let r = renderer();
        let missing: String = missing_chars(&r, 200).into_iter().collect();
        let first = missing.chars().next().unwrap();
        let notdef = r.advance(first, false, 11);
        assert_eq!(r.measure(&missing, false, 11), notdef * 200);
        assert_eq!(r.cache.borrow().len(), 1, "one notdef entry for all 200");
        r.measure(&missing, true, 11);
        assert_eq!(r.cache.borrow().len(), 2, "one notdef entry per weight");
    }

    #[test]
    fn glyph_cache_never_exceeds_its_cap() {
        let r = renderer();
        let face = r.face(false).expect("a face");
        let present: Vec<char> = (0x20u32..0x3_0000)
            .filter_map(char::from_u32)
            .filter(|&ch| face.get_char_index(ch as usize).is_some())
            .take(600)
            .collect();
        let mut peak = 0;
        'fill: for size in 8..40 {
            for bold in [false, true] {
                for &ch in &present {
                    r.advance(ch, bold, size);
                    let len = r.cache.borrow().len();
                    assert!(len <= GLYPH_CAP, "cache holds {len}, cap {GLYPH_CAP}");
                    peak = peak.max(len);
                    if peak == GLYPH_CAP && len < 10 {
                        break 'fill;
                    }
                }
            }
        }
        assert_eq!(peak, GLYPH_CAP, "the fill reaches the cap");
        assert!(
            r.cache.borrow().len() < 10,
            "the cache was cleared at the cap"
        );
    }

    #[test]
    fn a_cleared_cache_draws_and_measures_the_same() {
        let r = renderer();
        let odd = missing_chars(&r, 1)[0];
        let text = format!("Start Wg{odd}é");
        let snapshot = |r: &TextRenderer| {
            let mut c = Canvas::new(120, 20);
            r.draw(
                &mut c,
                Rect::new(0, 0, 120, 20),
                &text,
                true,
                Color::rgb(255, 255, 255),
                Align::Left,
                11,
            );
            let advances: Vec<i32> = text.chars().map(|ch| r.advance(ch, true, 11)).collect();
            (c.pixels, advances)
        };
        let before = snapshot(&r);
        let mut size = 12;
        let mut ch = 0x21u32;
        while r.cache.borrow().len() > 1 {
            if let Some(c) = char::from_u32(ch) {
                r.advance(c, false, size);
            }
            ch += 1;
            if ch == 0x2000 {
                ch = 0x21;
                size += 1;
            }
        }
        assert_eq!(snapshot(&r), before);
    }

    #[test]
    fn font_database_is_rescanned_only_for_a_missing_preferred_family() {
        let db = system_fonts(&[]);
        let present = db
            .faces()
            .find_map(|f| f.families.first())
            .map(|(name, _)| name.clone())
            .expect("a system font");
        let missing = "No Such Font Family".to_string();
        assert!(!needs_scan(&db, &[]));
        assert!(!needs_scan(&db, std::slice::from_ref(&present)));
        assert!(!needs_scan(&db, &[present.clone(), missing.clone()]));
        assert!(needs_scan(&db, &[missing, present]));
    }

    #[test]
    fn span_is_the_prefix_width_and_the_char_advance() {
        let r = renderer();
        let text = "Séarch";
        for (i, ch) in text.chars().enumerate() {
            let prefix: String = text.chars().take(i).collect();
            assert_eq!(
                r.span(text, i, true, 11),
                (r.measure(&prefix, true, 11), r.advance(ch, true, 11))
            );
        }
        assert_eq!(r.span(text, 6, false, 11), (r.measure(text, false, 11), 0));
        assert_eq!(
            r.span_and_measure(text, 2, true, 11),
            (
                r.measure("Sé", true, 11),
                r.advance('a', true, 11),
                r.measure(text, true, 11)
            )
        );
    }

    #[test]
    fn wrap_chars_fills_lines_to_the_column_count() {
        assert_eq!(
            wrap_chars("aaa bbb cc dddd", 7),
            vec!["aaa bbb", "cc dddd"],
            "a line takes a word that ends exactly on the column"
        );
        assert_eq!(wrap_chars("one\ntwo three", 20), vec!["one", "two three"]);
        assert_eq!(
            wrap_chars("toolongword x", 4),
            vec!["tool", "ongw", "ord", "x"]
        );
        assert!(wrap_chars("  ", 10).is_empty(), "blank text has no lines");
    }

    #[test]
    fn wrap_chars_breaks_a_long_word_after_its_last_separator_that_fits() {
        assert_eq!(
            wrap_chars("in /srv/projects/Downloads/archive", 18),
            vec!["in", "/srv/projects/", "Downloads/archive"]
        );
        assert_eq!(
            wrap_chars("a_very-long.name", 7),
            vec!["a_very-", "long.", "name"]
        );
        assert_eq!(wrap_chars("/abcdef", 3), vec!["/ab", "cde", "f"]);
        assert_eq!(
            wrap_chars("ab", 0),
            vec!["a", "b"],
            "a line holds one char at least"
        );
    }

    #[test]
    fn wrap_breaks_a_word_wider_than_the_line_at_the_widest_prefix() {
        let r = renderer();
        let word = "ProjectArchiveWithoutAnySpacesInItsName";
        let max_w = r.measure(word, false, 11) / 3;
        let lines = wrap(&r, word, max_w, 11);
        assert!(lines.len() >= 3, "{lines:?}");
        assert_eq!(lines.concat(), word);
        for pair in lines.windows(2) {
            assert!(
                r.measure(&pair[0], false, 11) <= max_w,
                "{pair:?} fits {max_w}"
            );
            let more = format!("{}{}", pair[0], pair[1].chars().next().unwrap());
            assert!(
                r.measure(&more, false, 11) > max_w,
                "{more:?} is over {max_w}"
            );
        }
    }

    #[test]
    fn wrap_keeps_words_that_fit_whole() {
        let r = renderer();
        let body = "Original folder /srv/projects/Documents";
        let max_w = r.measure("/srv/projects/Documents", false, 11);
        assert_eq!(
            wrap(&r, body, max_w, 11),
            vec!["Original folder", "/srv/projects/Documents"]
        );
    }

    #[test]
    fn wrap_breaks_on_width_and_newlines() {
        let r = renderer();
        assert_eq!(
            wrap(&r, "one two\nthree", 10_000, 11),
            vec!["one two", "three"]
        );
        assert!(wrap(&r, "aaaa bbbb cccc dddd eeee ffff", 40, 11).len() > 1);
    }

    #[test]
    fn wrap_fills_each_line_to_the_measured_width() {
        let r = renderer();
        let body = "The quick brown fox jumps over the lazy dog and keeps running past the fence";
        for bold in [false, true] {
            for max_w in [60, 90, 150] {
                let lines = wrap_bold(&r, body, max_w, 11, bold);
                assert_eq!(lines.join(" "), body);
                for pair in lines.windows(2) {
                    assert!(
                        r.measure(&pair[0], bold, 11) <= max_w,
                        "{pair:?} fits {max_w}"
                    );
                    let next = pair[1].split(' ').next().unwrap();
                    let joined = format!("{} {next}", pair[0]);
                    assert!(
                        r.measure(&joined, bold, 11) > max_w,
                        "{joined:?} is over {max_w}"
                    );
                }
            }
        }
    }

    fn ink_box(c: &Canvas) -> (i32, i32, i32, i32) {
        let w = c.device_size().0 as i32;
        let mut b = (i32::MAX, i32::MAX, 0, 0);
        for (i, &p) in c.pixels.iter().enumerate() {
            if p != 0 {
                let (x, y) = (i as i32 % w, i as i32 / w);
                b = (b.0.min(x), b.1.min(y), b.2.max(x + 1), b.3.max(y + 1));
            }
        }
        b
    }

    #[test]
    fn text_on_a_scale_2_canvas_is_rasterized_at_twice_the_size_on_the_same_layout() {
        let r = renderer();
        let draw = |s: i32| {
            let mut c = Canvas::with_scale(60, 20, s);
            let rect = Rect::new(4, 0, 56, 20);
            r.draw(
                &mut c,
                rect,
                "HHH",
                false,
                Color::rgb(0, 0, 0),
                Align::Left,
                11,
            );
            ink_box(&c)
        };
        let (a, b) = (draw(1), draw(2));
        assert!(a.2 > a.0, "expected ink at scale 1");
        let near = |got: i32, want: i32| (got - want).abs() <= 2;
        assert!(near(b.0, 2 * a.0), "left edge {b:?} is twice {a:?}");
        assert!(
            near(b.1, 2 * a.1) && near(b.3, 2 * a.3),
            "rows {b:?} are twice {a:?}"
        );
        assert!(near(b.2, 2 * a.2), "right edge {b:?} is twice {a:?}");
    }

    #[test]
    fn coverage_unpacks_every_bitmap_depth() {
        use freetype::ffi::{
            FT_Bitmap, FT_PIXEL_MODE_GRAY, FT_PIXEL_MODE_GRAY2, FT_PIXEL_MODE_GRAY4,
            FT_PIXEL_MODE_MONO,
        };
        let expand = |mode, pitch: i32, mut buf: Vec<u8>| {
            let raw = FT_Bitmap {
                rows: 2,
                width: 5,
                pitch,
                buffer: buf.as_mut_ptr(),
                num_grays: 0,
                pixel_mode: mode as _,
                palette_mode: 0,
                palette: std::ptr::null_mut(),
            };
            // SAFETY: `raw` and `buf` outlive the bitmap, whose buffer holds `pitch * rows` bytes.
            coverage(&unsafe { freetype::Bitmap::from_raw(&raw) })
        };
        let want = vec![255, 0, 255, 0, 255, 0, 0, 0, 0, 255];
        assert_eq!(expand(FT_PIXEL_MODE_MONO, 1, vec![0xA8, 0x08]), want);
        assert_eq!(
            expand(FT_PIXEL_MODE_GRAY2, 2, vec![0xCC, 0xC0, 0x00, 0x40]),
            [255, 0, 255, 0, 255, 0, 0, 0, 0, 85]
        );
        assert_eq!(
            expand(
                FT_PIXEL_MODE_GRAY4,
                3,
                vec![0xF0, 0xF0, 0xF0, 0x00, 0x00, 0x80]
            ),
            [255, 0, 255, 0, 255, 0, 0, 0, 0, 136]
        );
        assert_eq!(expand(FT_PIXEL_MODE_GRAY, 5, want.clone()), want);
    }
}
