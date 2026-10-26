//! The faces UI text falls back to for a char its own face lacks (e.g.
//! CJK), found through fontconfig and kept per block of code points.

use std::collections::{HashMap, HashSet};
use std::ffi::CStr;
use std::path::PathBuf;

/// Code points per block: a face found for one char of a block is tried
/// first for the rest of it, so a line of Han asks fontconfig once.
const BLOCK_BITS: u32 = 7;

/// Past this many chars remembered as drawn by no face, the record is
/// dropped: clients pick the text (window titles), so it stays bounded.
const MISSING_CAP: usize = 4096;

/// The faces found so far, as `F`, each with the `K` it was opened from,
/// and which of them each block has used. A block's faces are tried in
/// the order they were found; fontconfig is asked only for a char none of
/// them has, and a char no face has is not asked about again.
pub(crate) struct FallbackCache<K, F> {
    faces: Vec<(K, F)>,
    blocks: HashMap<(u32, bool), Vec<usize>>,
    missing: HashSet<(char, bool)>,
}

impl<K: PartialEq, F> Default for FallbackCache<K, F> {
    fn default() -> Self {
        FallbackCache {
            faces: Vec::new(),
            blocks: HashMap::new(),
            missing: HashSet::new(),
        }
    }
}

impl<K: PartialEq, F> FallbackCache<K, F> {
    /// The face that draws `ch` (in the bold face when `bold`): one its
    /// block already used when `has` says it has `ch`, else what `find`
    /// names for it, opened by `open` unless an earlier find opened the
    /// same. `None` when no face has it.
    pub fn pick(
        &mut self,
        ch: char,
        bold: bool,
        has: impl Fn(&F, char) -> bool,
        find: impl FnOnce(char, bool) -> Option<K>,
        open: impl FnOnce(&K) -> Option<F>,
    ) -> Option<&F> {
        if self.missing.contains(&(ch, bold)) {
            return None;
        }
        let block = (ch as u32 >> BLOCK_BITS, bold);
        let known = self
            .blocks
            .get(&block)
            .and_then(|used| used.iter().copied().find(|&i| has(&self.faces[i].1, ch)));
        let found = known.or_else(|| {
            let key = find(ch, bold)?;
            let i = match self.faces.iter().position(|(k, _)| *k == key) {
                Some(i) => i,
                None => {
                    let face = open(&key)?;
                    self.faces.push((key, face));
                    self.faces.len() - 1
                }
            };
            let used = self.blocks.entry(block).or_default();
            if !used.contains(&i) {
                used.push(i);
            }
            Some(i)
        });
        match found {
            Some(i) => Some(&self.faces[i].1),
            None => {
                if self.missing.len() >= MISSING_CAP {
                    self.missing.clear();
                }
                self.missing.insert((ch, bold));
                None
            }
        }
    }
}

pub(crate) type FaceFile = (PathBuf, u32);

/// The file of the face fontconfig ranks first for sans-serif text that
/// has `ch` (bold when `bold`), as the user's fontconfig rules and locale
/// order them. Outline faces, or with `bitmap` bitmap faces (fixed
/// strikes: a bitmap CJK font for mono text); never a colour face (an
/// emoji font). `None` when no such face has it, or fontconfig cannot
/// start.
pub(crate) fn fontconfig_face_for(ch: char, bold: bool, bitmap: bool) -> Option<FaceFile> {
    use fontconfig_sys::constants::{
        FC_CHARSET, FC_FAMILY, FC_WEIGHT, FC_WEIGHT_BOLD, FC_WEIGHT_REGULAR,
    };
    use fontconfig_sys::*;
    use std::ptr::null_mut;
    // SAFETY: every object made here is destroyed before the return; the
    // strings and charsets read from the sorted set's patterns are only
    // read while the set lives.
    unsafe {
        if FcInit() == 0 {
            return None;
        }
        let pat = FcPatternCreate();
        if pat.is_null() {
            return None;
        }
        FcPatternAddString(pat, FC_FAMILY.as_ptr(), c"sans-serif".as_ptr().cast());
        let weight = if bold {
            FC_WEIGHT_BOLD
        } else {
            FC_WEIGHT_REGULAR
        };
        FcPatternAddInteger(pat, FC_WEIGHT.as_ptr(), weight);
        let want = FcCharSetCreate();
        if want.is_null() {
            FcPatternDestroy(pat);
            return None;
        }
        FcCharSetAddChar(want, ch as FcChar32);
        FcPatternAddCharSet(pat, FC_CHARSET.as_ptr(), want);
        FcConfigSubstitute(null_mut(), pat, FcMatchPattern);
        FcDefaultSubstitute(pat);
        let mut result = 0;
        let set = FcFontSort(null_mut(), pat, 1, null_mut(), &mut result);
        let mut out = None;
        if !set.is_null() {
            let (fonts, n) = ((*set).fonts, (*set).nfont);
            // An empty set may hold a null `fonts`.
            if n > 0 && !fonts.is_null() {
                out = first_with(std::slice::from_raw_parts(fonts, n as usize), ch, bitmap);
            }
            FcFontSetDestroy(set);
        }
        FcCharSetDestroy(want);
        FcPatternDestroy(pat);
        out
    }
}

/// The file of the first of `fonts` (fontconfig's sorted patterns) that has `ch` and is an
/// outline face, or a bitmap face with `bitmap`, and not a colour face.
///
/// # Safety
/// Every pattern in `fonts` is live for the call.
unsafe fn first_with(
    fonts: &[*mut fontconfig_sys::FcPattern],
    ch: char,
    bitmap: bool,
) -> Option<FaceFile> {
    use fontconfig_sys::constants::{FC_CHARSET, FC_COLOR, FC_FILE, FC_INDEX, FC_SCALABLE};
    use fontconfig_sys::*;
    use std::ptr::null_mut;
    for &font in fonts {
        // SAFETY: `font` is live (the caller's contract); the charset and file string it hands
        // out are owned by it and read before the return.
        unsafe {
            let mut cs = null_mut();
            let has = FcPatternGetCharSet(font, FC_CHARSET.as_ptr(), 0, &mut cs) == 0
                && FcCharSetHasChar(cs, ch as FcChar32) != 0;
            let flag = |name: &CStr| {
                let mut b = 0;
                FcPatternGetBool(font, name.as_ptr(), 0, &mut b) == 0 && b != 0
            };
            if !has || flag(FC_SCALABLE) == bitmap || flag(FC_COLOR) {
                continue;
            }
            let mut file = null_mut();
            if FcPatternGetString(font, FC_FILE.as_ptr(), 0, &mut file) != 0 {
                continue;
            }
            let mut index = 0;
            FcPatternGetInteger(font, FC_INDEX.as_ptr(), 0, &mut index);
            let path = CStr::from_ptr(file.cast()).to_string_lossy().into_owned();
            return Some((PathBuf::from(path), index.max(0) as u32));
        }
    }
    None
}

/// The strike (ppem) of a bitmap fallback face that stands in for UI text
/// at `size_px`: the one nearest it within 2 px, the larger on a tie
/// (bitmap CJK faces start at 12 px, beside 11 px Latin). `None` when
/// none is that close; the char is then drawn from an outline face.
pub(crate) fn fallback_strike(strikes: impl IntoIterator<Item = u32>, size_px: u32) -> Option<u32> {
    strikes
        .into_iter()
        .filter(|&p| p > 0 && p.abs_diff(size_px) <= 2)
        .min_by_key(|&p| (p.abs_diff(size_px), std::cmp::Reverse(p)))
}

/// How a char the UI face lacks draws at `size_px` on a canvas at `scale`, given the bitmap
/// fallback face's [`fallback_strike`] for `size_px`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FallbackDraw {
    /// The bitmap strike at this ppem, in mono.
    Strike(u32),
    /// An outline face at this device ppem, smoothed.
    Outline(u32),
}

/// Bitmap strikes draw only at scale 1. Off scale 1 the outline takes the strike's size times
/// the scale, so it fills the logical advance the strike set.
pub(crate) fn fallback_draw(strike: Option<u32>, size_px: u32, scale: u32) -> FallbackDraw {
    match strike {
        Some(p) if scale == 1 => FallbackDraw::Strike(p),
        Some(p) => FallbackDraw::Outline(p * scale),
        None => FallbackDraw::Outline(size_px * scale),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    type Fake = &'static str;

    fn has(f: &Fake, ch: char) -> bool {
        f.contains(ch)
    }

    #[test]
    fn a_block_asks_once_and_its_face_draws_the_rest() {
        let mut c = FallbackCache::<u32, Fake>::default();
        let asked = Cell::new(0);
        let find = |_: char, _: bool| {
            asked.set(asked.get() + 1);
            Some(1)
        };
        assert_eq!(
            c.pick('中', false, has, find, |_| Some("中丁")),
            Some(&"中丁")
        );
        assert_eq!(c.pick('丁', false, has, find, |_| None), Some(&"中丁"));
        assert_eq!(asked.get(), 1, "the second char came from the block's face");
    }

    #[test]
    fn a_char_the_block_s_face_lacks_asks_again_and_shares_an_opened_face() {
        let mut c = FallbackCache::<u32, Fake>::default();
        let opened = Cell::new(0);
        let open = |_: &u32| {
            opened.set(opened.get() + 1);
            Some("中")
        };
        c.pick('中', false, has, |_, _| Some(1), open);
        // `丁` shares `中`'s block but not its face: a second face.
        let got = c.pick('丁', false, has, |_, _| Some(2), |_| Some("丁"));
        assert_eq!(got, Some(&"丁"));
        // A char neither has that fontconfig names the first face for
        // reuses it.
        let got = c.pick('丂', false, |_, _| false, |_, _| Some(1), open);
        assert_eq!(got, Some(&"中"));
        assert_eq!(opened.get(), 1, "face 1 opened once");
    }

    #[test]
    fn a_char_no_face_has_is_not_asked_about_again() {
        let mut c = FallbackCache::<u32, Fake>::default();
        let asked = Cell::new(0);
        let none = |_: char, _: bool| {
            asked.set(asked.get() + 1);
            None
        };
        assert_eq!(c.pick('\u{E000}', false, has, none, |_| None), None);
        assert_eq!(c.pick('\u{E000}', false, has, none, |_| None), None);
        assert_eq!(asked.get(), 1);
    }

    #[test]
    fn bold_and_regular_pick_their_own_faces() {
        let mut c = FallbackCache::<u32, Fake>::default();
        c.pick('中', false, has, |_, _| Some(1), |_| Some("中"));
        let bold = c.pick(
            '中',
            true,
            has,
            |_, b| Some(if b { 2 } else { 1 }),
            |_| Some("中 bold"),
        );
        assert_eq!(bold, Some(&"中 bold"));
    }

    #[test]
    fn a_bitmap_fallback_takes_the_strike_nearest_the_size() {
        let wqy = [12, 13, 15, 16];
        assert_eq!(
            fallback_strike(wqy, 11),
            Some(12),
            "12 px beside 11 px text"
        );
        assert_eq!(fallback_strike(wqy, 13), Some(13));
        assert_eq!(fallback_strike(wqy, 14), Some(15), "the larger on a tie");
        assert_eq!(fallback_strike(wqy, 9), None, "none within 2 px");
        assert_eq!(fallback_strike([], 11), None);
    }

    #[test]
    fn a_bitmap_strike_draws_at_scale_1_and_an_outline_of_its_size_off_it() {
        assert_eq!(fallback_draw(Some(12), 11, 1), FallbackDraw::Strike(12));
        assert_eq!(fallback_draw(Some(12), 11, 2), FallbackDraw::Outline(24));
        assert_eq!(fallback_draw(Some(12), 11, 3), FallbackDraw::Outline(36));
        assert_eq!(fallback_draw(None, 11, 1), FallbackDraw::Outline(11));
        assert_eq!(fallback_draw(None, 11, 2), FallbackDraw::Outline(22));
    }

    #[test]
    fn an_empty_sorted_set_names_no_face() {
        // SAFETY: no patterns to read.
        assert_eq!(unsafe { first_with(&[], '中', false) }, None);
    }

    #[test]
    fn fontconfig_names_a_cjk_face_for_han_when_one_is_installed() {
        // Machines without a CJK font have nothing to name.
        let Some((path, _)) = fontconfig_face_for('中', false, false) else {
            return;
        };
        assert!(path.exists(), "{path:?}");
        assert_eq!(
            fontconfig_face_for('\u{10FFFD}', false, false),
            None,
            "no face has it"
        );
    }
}
