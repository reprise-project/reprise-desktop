//! Colour emoji for the emoji picker: the catalog (names, shortcodes and
//! groups from the `emojis` crate) and the first installed colour emoji font.
//! Sequences (ZWJ families, flags, keycaps) are shaped with rustybuzz to the
//! one glyph the font draws; ones without a single glyph are left out of the
//! catalog. Glyphs come from FreeType with `FT_LOAD_COLOR`: a bitmap font at
//! its nearest strike, a scalable one at the size asked, each box-filtered
//! to a square. A font without colour draws its coverage in the given ink.

use crate::raster::Canvas;
use crate::theme::Color;
use fontdb::{Family, Query};
use freetype::face::LoadFlag;
use freetype::{Face, Library};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

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
    data: Rc<Vec<u8>>,
    index: u32,
    face: Face,
    family: String,
    cache: RefCell<Squares>,
}

impl EmojiFont {
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
            let data = match &face.source {
                fontdb::Source::File(p) | fontdb::Source::SharedFile(p, _) => {
                    std::fs::read(p).ok()?
                }
                fontdb::Source::Binary(_) => return None,
            };
            let data = Rc::new(data);
            let ft = match lib.new_memory_face(data.clone(), face.index as isize) {
                Ok(f) => f,
                Err(e) => {
                    log::warn!("FreeType could not open {family}: {e}");
                    return None;
                }
            };
            log::info!("emoji font: {family}");
            Some(EmojiFont {
                data,
                index: face.index,
                face: ft,
                family: family.to_string(),
                cache: RefCell::new(HashMap::new()),
            })
        })
    }

    pub fn family(&self) -> &str {
        &self.family
    }

    fn shaper(&self) -> Option<rustybuzz::Face<'_>> {
        rustybuzz::Face::from_slice(&self.data, self.index)
    }

    pub fn glyph(&self, emoji: &str) -> Option<u32> {
        self.glyph_with(&self.shaper()?, emoji)
    }

    fn glyph_with(&self, shaper: &rustybuzz::Face<'_>, emoji: &str) -> Option<u32> {
        let mut buf = rustybuzz::UnicodeBuffer::new();
        buf.push_str(emoji);
        buf.set_flags(rustybuzz::BufferFlags::REMOVE_DEFAULT_IGNORABLES);
        let out = rustybuzz::shape(shaper, &[], buf);
        match out.glyph_infos() {
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
        let f = &self.face;
        if f.is_scalable() {
            f.set_pixel_sizes(0, side).ok()?;
        } else {
            let strikes = strike_sizes(f);
            let pick = strikes
                .iter()
                .enumerate()
                .filter(|&(_, &ppem)| ppem >= side)
                .min_by_key(|&(_, &ppem)| ppem)
                .or_else(|| strikes.iter().enumerate().max_by_key(|&(_, &ppem)| ppem))?
                .0;
            f.select_size(pick as i32).ok()?;
        }
        f.load_glyph(gid, LoadFlag::COLOR | LoadFlag::RENDER).ok()?;
        bitmap_canvas(&f.glyph().bitmap(), ink)
    }
}

fn strike_sizes(f: &Face) -> Vec<u32> {
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
