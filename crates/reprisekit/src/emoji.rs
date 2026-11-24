//! Colour emoji: the picker's catalog (names, shortcodes and groups from the `emojis` crate) and
//! the first installed colour emoji font, which the picker and text draw from. Sequences (ZWJ
//! families, flags, keycaps) are shaped with harfrust to the one glyph the font draws; ones without
//! a single glyph are left out of the catalog. Glyphs come from FreeType with `FT_LOAD_COLOR`: a
//! bitmap font (CBDT, sbix) from its nearest strike, scaled; a scalable one (COLRv0) at the size
//! asked. The picker box-filters each to a square; a font without colour draws its coverage in the
//! given ink.

use crate::raster::Canvas;
use crate::theme::Color;
use fontdb::{Family, Query};
use freetype::face::LoadFlag;
use freetype::{Face, Library};
use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use unicode_properties::{EmojiStatus, UnicodeEmoji};

pub const FAMILIES: &[&str] = &[
    "Noto Color Emoji",
    "Twemoji",
    "Twitter Color Emoji",
    "JoyPixels",
    "OpenMoji Color",
    "Apple Color Emoji",
    "Segoe UI Emoji",
    "Noto Emoji",
    "Symbola",
];

/// Rendered squares are dropped past this many and redrawn on demand.
const CACHE_CAP: usize = 4096;

/// Rendered squares by (emoji, side in device px); None when the font cannot draw it.
type Squares = HashMap<(String, u32), Option<Rc<Canvas>>>;

/// Whether `ch` draws as an emoji on its own (`Emoji_Presentation`), not only before U+FE0F.
pub(crate) fn presents_as_emoji(ch: char) -> bool {
    matches!(
        ch.emoji_status(),
        EmojiStatus::EmojiPresentation
            | EmojiStatus::EmojiPresentationAndModifierBase
            | EmojiStatus::EmojiPresentationAndEmojiComponent
            | EmojiStatus::EmojiPresentationAndModifierAndEmojiComponent
    )
}

/// Whether `ch` may be part of a glyph the emoji face draws in colour.
pub(crate) fn may_draw_in_colour(ch: char) -> bool {
    !ch.is_ascii() && ch.is_emoji_char_or_emoji_component()
}

/// Whether `next` after `ch` carries on an emoji sequence: a presentation selector, a joiner, a
/// skin tone, a keycap, a tag or a flag's second letter.
pub(crate) fn continues(ch: char, next: char) -> bool {
    use unicode_properties::emoji::is_regional_indicator as flag;
    ch.is_emoji_char_or_emoji_component()
        && (matches!(
            next,
            '\u{FE0F}' | '\u{200D}' | '\u{20E3}' | '\u{1F3FB}'..='\u{1F3FF}' | '\u{E0020}'..='\u{E007F}'
        ) || flag(ch) && flag(next))
}

/// A glyph as text lays it, in device px: premultiplied pixels, and their offsets right of the
/// pen and up from the baseline.
pub(crate) struct TextGlyph {
    pub pixels: Canvas,
    pub left: i32,
    pub top: i32,
    pub advance: i32,
}

thread_local! {
    static SHARED: OnceCell<Option<Rc<EmojiFont>>> = const { OnceCell::new() };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub emoji: &'static str,
    pub name: &'static str,
    pub shortcodes: &'static [&'static str],
}

impl Entry {
    pub fn get(emoji: &str) -> Option<Entry> {
        emojis::get(emoji).map(Entry::of)
    }

    fn of(e: &'static emojis::Emoji) -> Entry {
        Entry {
            emoji: e.as_str(),
            name: e.name(),
            shortcodes: SHORTCODES.with(|m| m.get(e.as_str()).copied().unwrap_or(&[])),
        }
    }
}

thread_local! {
    /// `emojis` hands shortcodes out as an iterator; an `Entry` keeps a slice.
    static SHORTCODES: HashMap<&'static str, &'static [&'static str]> = emojis::iter()
        .map(|e| {
            let codes: Vec<&'static str> = e.shortcodes().collect();
            (e.as_str(), &*Vec::leak(codes))
        })
        .collect();
}

/// Every emoji at its default skin tone in Unicode order, less those `font`
/// has no single glyph for. Without a font, all of them.
pub fn catalog(font: Option<&EmojiFont>) -> Vec<Entry> {
    let shaper = font.and_then(|f| f.shaper());
    emojis::iter()
        .filter(|e| match (font, &shaper) {
            (Some(f), Some(s)) => f.glyph_with(s, e.as_str()).is_some(),
            _ => true,
        })
        .map(Entry::of)
        .collect()
}

pub struct EmojiFont {
    shaper: Option<harfrust::Font>,
    face: Face<Arc<[u8]>>,
    family: String,
    cache: RefCell<Squares>,
    /// The glyph each sequence shapes to, as [`EmojiFont::glyph`] finds it.
    sequences: RefCell<HashMap<Box<str>, Option<u32>>>,
    /// Under `REPRISE_TEST_FONTS`, set for a face outside the test fonts.
    stray: Option<Box<str>>,
}

impl EmojiFont {
    /// This thread's font, loaded on first use.
    pub fn shared() -> Option<Rc<EmojiFont>> {
        SHARED.with(|f| f.get_or_init(|| EmojiFont::load().map(Rc::new)).clone())
    }

    pub fn load() -> Option<EmojiFont> {
        let wanted: Vec<String> = FAMILIES.iter().map(|f| f.to_string()).collect();
        let db = crate::text::system_fonts(&wanted);
        let lib = Library::init().ok()?;
        FAMILIES.iter().find_map(|family| {
            let id = db.query(&Query {
                families: &[Family::Name(family)],
                ..Query::default()
            })?;
            // A query names a fallback when the family is missing; take only the family asked for.
            let face = db.face(id)?;
            if !face.families.iter().any(|(n, _)| n == family) {
                return None;
            }
            let path = match &face.source {
                fontdb::Source::File(p) | fontdb::Source::SharedFile(p, _) => p,
                fontdb::Source::Binary(_) => return None,
            };
            let data: Arc<[u8]> = std::fs::read(path).ok()?.into();
            let ft = match lib.new_memory_face2(data.clone(), face.index as isize) {
                Ok(f) => f,
                Err(e) => {
                    log::warn!("FreeType could not open {family}: {e}");
                    return None;
                }
            };
            let bytes: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(data);
            log::info!("emoji font: {family}");
            Some(EmojiFont {
                shaper: harfrust::Font::new(bytes, face.index),
                face: ft,
                family: family.to_string(),
                cache: RefCell::new(HashMap::new()),
                sequences: RefCell::new(HashMap::new()),
                stray: crate::text::stray_note(path, family, || "emoji".into()),
            })
        })
    }

    pub fn family(&self) -> &str {
        &self.family
    }

    fn shaper(&self) -> Option<harfrust::ShaperFont<'_, '_>> {
        self.shaper.as_ref().map(harfrust::ShaperFont::new)
    }

    pub fn glyph(&self, emoji: &str) -> Option<u32> {
        self.glyph_with(&self.shaper()?, emoji)
    }

    fn glyph_with(&self, shaper: &harfrust::ShaperFont<'_, '_>, emoji: &str) -> Option<u32> {
        let mut buf = harfrust::Buffer::new();
        buf.push_str(emoji);
        buf.guess_segment_properties();
        buf.set_flags(harfrust::BufferFlags::REMOVE_DEFAULT_IGNORABLES);
        harfrust::shape(shaper, &mut buf, harfrust::ShapeOptions::new()).ok()?;
        match buf.glyph_infos() {
            [g] if g.glyph_id != 0 => Some(g.glyph_id),
            _ => None,
        }
    }

    pub fn render(&self, emoji: &str, side: u32, ink: Color) -> Option<Rc<Canvas>> {
        let key = (emoji.to_string(), side);
        if let Some(hit) = self.cache.borrow().get(&key) {
            return hit.clone();
        }
        let drawn = self
            .glyph(emoji)
            .and_then(|g| self.raster(g, side, ink))
            .map(|c| Rc::new(fit_square(&c, side)));
        let mut cache = self.cache.borrow_mut();
        if cache.len() >= CACHE_CAP {
            cache.clear();
        }
        cache.insert(key, drawn.clone());
        drawn
    }

    /// A bitmap font's nearest strike at or above `side` (else its largest), a scalable font at `side`.
    fn raster(&self, gid: u32, side: u32, ink: Color) -> Option<Canvas> {
        self.size_for(side)?;
        self.face
            .load_glyph(gid, LoadFlag::COLOR | LoadFlag::RENDER)
            .ok()?;
        bitmap_canvas(&self.face.glyph().bitmap(), ink)
    }

    /// Set the face to `px` or, for a bitmap font, its nearest strike at or above it (else its
    /// largest); the ppem set.
    fn size_for(&self, px: u32) -> Option<u32> {
        let f = &self.face;
        if f.is_scalable() {
            f.set_pixel_sizes(0, px).ok()?;
            return Some(px);
        }
        let strikes = strike_sizes(f);
        let (pick, &ppem) = strikes
            .iter()
            .enumerate()
            .filter(|&(_, &ppem)| ppem >= px)
            .min_by_key(|&(_, &ppem)| ppem)
            .or_else(|| strikes.iter().enumerate().max_by_key(|&(_, &ppem)| ppem))?;
        f.select_size(pick as i32).ok()?;
        Some(ppem)
    }

    pub(crate) fn has_colour(&self) -> bool {
        self.face.has_color()
    }

    pub(crate) fn stray(&self) -> Option<&str> {
        self.stray.as_deref()
    }

    /// The glyph `ch` maps to on its own.
    pub(crate) fn char_glyph(&self, ch: char) -> Option<u32> {
        self.face.get_char_index(ch as usize)
    }

    /// [`EmojiFont::glyph`], remembered per sequence.
    pub(crate) fn sequence(&self, emoji: &str) -> Option<u32> {
        if let Some(&hit) = self.sequences.borrow().get(emoji) {
            return hit;
        }
        let gid = self.glyph(emoji);
        let mut seen = self.sequences.borrow_mut();
        if seen.len() >= CACHE_CAP {
            seen.clear();
        }
        seen.insert(emoji.into(), gid);
        gid
    }

    /// Glyph `gid` in colour at `ppem` px to the em, shrunk to at most `max_h` px tall; `None`
    /// when the font draws it without colour.
    pub(crate) fn text_glyph(&self, gid: u32, ppem: u32, max_h: u32) -> Option<TextGlyph> {
        use freetype::bitmap::PixelMode;
        let source = self.size_for(ppem)?;
        self.face
            .load_glyph(gid, LoadFlag::COLOR | LoadFlag::RENDER)
            .ok()?;
        let slot = self.face.glyph();
        let bm = slot.bitmap();
        if !matches!(bm.pixel_mode(), Ok(PixelMode::Bgra)) {
            return None;
        }
        let img = bitmap_canvas(&bm, Color::rgb(0, 0, 0))?;
        let k = (f64::from(ppem) / f64::from(source)).min(f64::from(max_h) / f64::from(img.height));
        let at = |v: f64| (v * k).round() as i32;
        let (w, h) = (
            at(f64::from(img.width)).max(1),
            at(f64::from(img.height)).max(1),
        );
        Some(TextGlyph {
            pixels: img.resample_box(w as u32, h as u32),
            left: at(f64::from(slot.bitmap_left())),
            top: at(f64::from(slot.bitmap_top())),
            advance: at(slot.advance().x as f64 / 64.0),
        })
    }
}

fn strike_sizes(f: &Face<Arc<[u8]>>) -> Vec<u32> {
    crate::text::fixed_sizes(f)
        .iter()
        .map(|s| (s.y_ppem >> 6).max(i64::from(s.height)) as u32)
        .collect()
}

/// BGRA as is (FreeType's is premultiplied like the canvas), coverage as `ink` at that alpha.
fn bitmap_canvas(bm: &freetype::Bitmap, ink: Color) -> Option<Canvas> {
    use freetype::bitmap::PixelMode;
    let (w, h, pitch) = (bm.width() as usize, bm.rows() as usize, bm.pitch());
    if w == 0 || h == 0 {
        return None;
    }
    let buf = bm.buffer();
    let row = |y: usize| {
        if pitch >= 0 {
            y * pitch as usize
        } else {
            (h - 1 - y) * (-pitch) as usize
        }
    };
    let mode = bm.pixel_mode().ok()?;
    let mut px = vec![0u32; w * h];
    for y in 0..h {
        let r = row(y);
        for x in 0..w {
            px[y * w + x] = match mode {
                PixelMode::Bgra => {
                    let p = &buf[r + 4 * x..r + 4 * x + 4];
                    u32::from_le_bytes([p[0], p[1], p[2], p[3]])
                }
                PixelMode::Gray => ink_at(ink, buf[r + x]),
                PixelMode::Mono => ink_at(
                    ink,
                    if buf[r + x / 8] & (0x80 >> (x % 8)) != 0 {
                        255
                    } else {
                        0
                    },
                ),
                _ => return None,
            };
        }
    }
    Some(Canvas::from_pixels(w as u32, h as u32, px))
}

fn ink_at(ink: Color, a: u8) -> u32 {
    let argb = ink.argb();
    let ch = |shift: u32| ((argb >> shift) & 0xFF) * u32::from(a) / 255;
    (u32::from(a) << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

fn fit_square(src: &Canvas, side: u32) -> Canvas {
    let (sw, sh) = (src.width.max(1), src.height.max(1));
    let (w, h) = if sw <= side && sh <= side {
        (sw, sh)
    } else if sw >= sh {
        (side, (sh * side / sw).max(1))
    } else {
        ((sw * side / sh).max(1), side)
    };
    let small = src.resample_box(w, h);
    let mut c = Canvas::new(side, side);
    c.blit_over(((side - w) / 2) as i32, ((side - h) / 2) as i32, &small);
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_starts_with_the_smileys_and_names_them() {
        let all = catalog(None);
        assert!(all.len() > 1500, "{}", all.len());
        assert_eq!(all[0].emoji, "😀");
        assert_eq!(all[0].name, "grinning face");
        assert!(all[0].shortcodes.contains(&"grinning"));
        assert_eq!(Entry::get("👍").map(|e| e.name), Some("thumbs up"));
        assert_eq!(Entry::get("hello"), None);
    }

    #[test]
    fn a_square_keeps_the_shape_and_centres() {
        let mut wide = Canvas::new(40, 20);
        wide.fill_rect(wide.bounds(), Color::rgb(255, 0, 0));
        let sq = fit_square(&wide, 20);
        assert_eq!((sq.width, sq.height), (20, 20));
        assert_eq!(sq.get(0, 0), 0, "letterboxed above");
        assert_eq!(sq.get(10, 10), Color::rgb(255, 0, 0).argb());
        let small = fit_square(&Canvas::from_pixels(2, 2, vec![0xFF00_0000; 4]), 8);
        assert_eq!(small.get(3, 3), 0xFF00_0000);
        assert_eq!(small.get(2, 2), 0);
    }

    #[test]
    fn emoji_presentation_and_sequence_continuations() {
        for ch in ['😀', '✅', '🇭', '🏽'] {
            assert!(presents_as_emoji(ch), "U+{:04X}", ch as u32);
        }
        for ch in ['a', '#', '\u{2764}', '\u{A9}', '中'] {
            assert!(!presents_as_emoji(ch), "U+{:04X}", ch as u32);
        }
        assert!(continues('\u{2764}', '\u{FE0F}'));
        assert!(continues('👍', '🏽'));
        assert!(continues('👨', '\u{200D}'));
        assert!(continues('🇭', '🇺'));
        assert!(continues('#', '\u{FE0F}'));
        assert!(!continues('a', '\u{FE0F}'), "not an emoji");
        assert!(!continues('😀', 'b'));
        assert!(
            !continues('😀', '🇺'),
            "a flag's letter follows only a letter"
        );
    }

    #[test]
    fn ink_is_premultiplied_by_coverage() {
        assert_eq!(ink_at(Color::rgb(255, 255, 255), 255), 0xFFFF_FFFF);
        assert_eq!(ink_at(Color::rgb(255, 0, 0), 0), 0);
        assert_eq!(ink_at(Color::rgb(255, 0, 0), 128), 0x8080_0000);
    }

    #[test]
    fn sequences_shape_to_one_glyph_and_draw_in_colour() {
        let Some(font) = EmojiFont::load() else {
            return;
        };
        for e in ["😀", "👍", "❤️", "👨‍👩‍👧", "🇭🇺", "#️⃣"] {
            assert!(font.glyph(e).is_some(), "{e} in {}", font.family());
        }
        assert_eq!(font.glyph("ab"), None);
        let sq = font.render("😀", 24, Color::rgb(0, 0, 0)).unwrap();
        assert_eq!((sq.width, sq.height), (24, 24));
        if font.face.has_color() {
            let coloured = sq.pixels.iter().any(|&p| {
                let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
                r != g || g != b
            });
            assert!(coloured, "a colour font draws in colour");
        }
        let again = font.render("😀", 24, Color::rgb(0, 0, 0)).unwrap();
        assert!(Rc::ptr_eq(&sq, &again));
        let covered = catalog(Some(&font));
        assert!(covered.len() > 1000, "{}", covered.len());
        assert!(covered.iter().all(|e| font.glyph(e.emoji).is_some()));
    }
}
