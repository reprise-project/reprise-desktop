use std::sync::Arc;

use crate::text::TextRenderer;
use crate::theme::{Color, FontConfig};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub color: Color,
    pub dx: i32,
    pub dy: i32,
    pub soft: Option<SoftShadow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoftShadow {
    pub size: i32,
    /// Scales the coverage summed under each pixel (numerator, denominator),
    /// capped at full.
    pub gain: (u32, u32),
}

impl Shadow {
    pub fn alpha(&self, coverage: &[u8], w: usize, h: usize, d: i32) -> Vec<u8> {
        let (ox, oy) = (self.dx * d, self.dy * d);
        let (n, gain) = match self.soft {
            Some(soft) => (soft.size * d, soft.gain),
            None => (1, (1, 1)),
        };
        let mut sums = vec![0u64; (w + 1) * (h + 1)];
        for y in 0..h {
            let mut row = 0u64;
            for x in 0..w {
                row += u64::from(coverage[y * w + x]);
                sums[(y + 1) * (w + 1) + x + 1] = sums[y * (w + 1) + x + 1] + row;
            }
        }
        let area = |x0: i32, y0: i32, x1: i32, y1: i32| -> u64 {
            let cx = |x: i32| x.clamp(0, w as i32) as usize;
            let cy = |y: i32| y.clamp(0, h as i32) as usize;
            let (x0, x1, y0, y1) = (cx(x0), cx(x1), cy(y0), cy(y1));
            if x0 >= x1 || y0 >= y1 {
                return 0;
            }
            let at = |x: usize, y: usize| sums[y * (w + 1) + x];
            at(x1, y1) + at(x0, y0) - at(x0, y1) - at(x1, y0)
        };
        // A pixel p lands its block on [p + o - n / 2, p + o - n / 2 + n).
        let lo = |x: i32, o: i32| x - o + n / 2 - n + 1;
        let (num, den) = (u64::from(gain.0), u64::from(gain.1) * (n * n) as u64);
        let mut out = vec![0u8; w * h];
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let (x0, y0) = (lo(x, ox), lo(y, oy));
                let sum = area(x0, y0, x0 + n, y0 + n);
                out[y as usize * w + x as usize] = (sum * num / den).min(255) as u8;
            }
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextSpec {
    pub family: Vec<String>,
    pub size_px: u32,
    pub family_sizes: Vec<(String, u32)>,
    pub bold: bool,
    pub italic: bool,
    pub antialias: bool,
    pub full_hinting: bool,
    pub color: Color,
    pub inactive: Color,
    pub shadow: Option<Shadow>,
    /// Exponent on each pixel's coverage (0 to 1) before inking; under 1 sets
    /// the text heavier.
    pub ink_gamma: f32,
}

impl TextSpec {
    pub fn font(&self) -> FontConfig {
        FontConfig {
            family: self.family.clone(),
            size_px: self.size_px,
            antialias: self.antialias,
            italic: self.italic,
            full_hinting: self.full_hinting,
            shell_dlg: None,
        }
    }

    /// The size to draw this spec at in `text`, a renderer loaded from
    /// [`TextSpec::font`]: the `family_sizes` entry for the face it
    /// loaded, else `size_px`.
    pub fn size_in(&self, text: &TextRenderer) -> u32 {
        self.family_sizes
            .iter()
            .find(|(family, _)| text.has_face(family))
            .map_or(self.size_px, |&(_, px)| px)
    }

    /// This spec at `pct` percent of its size, `family_sizes` too
    /// ([`crate::theme::text_px`]).
    pub fn with_text_size(&self, pct: u32) -> TextSpec {
        let px = |n| crate::theme::text_px(n, pct);
        TextSpec {
            size_px: px(self.size_px),
            family_sizes: self
                .family_sizes
                .iter()
                .map(|(f, n)| (f.clone(), px(*n)))
                .collect(),
            ..self.clone()
        }
    }

    pub fn weigh(&self, coverage: &mut [u8]) {
        if self.ink_gamma == 1.0 {
            return;
        }
        let table: Vec<u8> = (0..=255u8)
            .map(|c| (255.0 * (f32::from(c) / 255.0).powf(self.ink_gamma)).round() as u8)
            .collect();
        for c in coverage {
            *c = table[*c as usize];
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CaptionTitle {
    pub icon: [(i32, i32); 2],
    pub text_x: i32,
    pub baseline: Option<[i32; 2]>,
    pub glow: Option<TitleGlow>,
}

impl CaptionTitle {
    pub const CENTRED: CaptionTitle = CaptionTitle {
        icon: [(2, 4), (2, 4)],
        text_x: 22,
        baseline: None,
        glow: None,
    };
}

#[derive(Clone, Copy, Debug)]
pub struct TitleGlow {
    /// Device pixels, given device pixels per design pixel.
    pub reach: fn(i32) -> i32,
    pub color: Color,
    /// At scale 1: rows below the band's top it starts, rows above the client's
    /// top it ends; restored then maximized.
    pub rows: [(i32, i32); 2],
    pub alpha: fn(&GlowMask) -> Arc<[u8]>,
}

pub struct GlowMask<'a> {
    pub coverage: &'a [u8],
    pub width: usize,
    pub height: usize,
    pub rows: (usize, usize),
    pub active: bool,
    pub scale: i32,
}
