//! The glass colour. A colour and an intensity set how the glass lays over
//! the blurred backdrop B, per channel:
//!
//! ```text
//! out = blur·B + colour·C + afterglow·C·luma(B)/255
//! ```
//!
//! the three balances summing to 1 and moving with the intensity ([`balance`]); the material and the
//! colour move them ([`material_glass`]). A render pass blends with one alpha and cannot multiply by
//! what it blends over, so each surface draws the single alpha and premultiplied colour closest to that
//! over black and white:
//!
//! ```text
//! a   = 1 - blur - afterglow·mean(C)/255
//! P_c = colour·C_c + afterglow·(C_c - mean(C))/2      (0 ..= 255a)
//! ```
//!
//! each scaled and offset by the surface's [`Fit`]. The compositor's glass lifts the blurred backdrop
//! under the art by the rest of the afterglow ([`lift`]), so the art lands on the formula. The art is drawn in Sky at the default intensity;
//! [`Recolour`] moves every pixel by how far the formula moves the surface's plain glass, so Sky at the
//! default intensity is the art itself and sheens, lines and glyphs keep what they add over the glass.

use super::art;
use super::{Material, Settings, Swatch, bar};
use crate::backdrop::Lift;
use crate::theme::Color;

/// How a glass colour splits between the colour itself, the colour lit by the backdrop's brightness
/// (afterglow) and the backdrop, in percent; the three sum to 100.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Balance {
    pub colour: u8,
    pub afterglow: u8,
    pub blur: u8,
}

/// A surface's correction of the formula: `alpha` and `colour` are (scale, offset) of its alpha (0..1)
/// and premultiplied colour (levels); `opaque` is how far the opaque glass's colour follows the glass colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub alpha: (f64, f64),
    pub colour: (f64, f64),
    pub opaque: f64,
}

const fn mix(colour: u8, afterglow: u8, blur: u8) -> Balance {
    Balance {
        colour,
        afterglow,
        blur,
    }
}

const fn swatch(name: &'static str, color: Color, intensity: u8, balance: Balance) -> Swatch {
    Swatch {
        name,
        color,
        intensity,
        balance,
    }
}

/// The balances the swatches come at, from faint to strong: the colour share rises with the intensity.
const FAINT: Balance = mix(5, 34, 61);
const LIGHT: Balance = mix(5, 35, 60);
const SOFT: Balance = mix(5, 45, 50);
const CLEAR: Balance = mix(8, 43, 49);
const MILD: Balance = mix(12, 40, 48);
const FULL: Balance = mix(24, 32, 44);
const RICH: Balance = mix(29, 29, 42);
const DEEP: Balance = mix(56, 11, 33);

#[rustfmt::skip]
pub const SWATCHES: [Swatch; 16] = [
    swatch("sky", Color::rgb(112, 175, 252), 42, CLEAR),
    swatch("twilight", Color::rgb(0, 60, 168), 75, DEEP),
    swatch("sea", Color::rgb(49, 193, 201), 53, FULL),
    swatch("leaf", Color::rgb(11, 162, 0), 39, SOFT),
    swatch("lime", Color::rgb(141, 216, 51), 39, SOFT),
    swatch("sun", Color::rgb(250, 226, 9), 30, LIGHT),
    swatch("pumpkin", Color::rgb(250, 165, 0), 53, FULL),
    swatch("ruby", Color::rgb(201, 24, 15), 75, DEEP),
    swatch("fuchsia", Color::rgb(250, 0, 137), 39, SOFT),
    swatch("blush", Color::rgb(252, 197, 245), 45, MILD),
    swatch("violet", Color::rgb(112, 58, 157), 56, RICH),
    swatch("lavender", Color::rgb(141, 88, 145), 29, FAINT),
    swatch("taupe", Color::rgb(149, 133, 74), 39, SOFT),
    swatch("chocolate", Color::rgb(76, 29, 26), 75, DEEP),
    swatch("slate", Color::rgb(83, 83, 83), 53, FULL),
    swatch("frost", Color::rgb(252, 251, 252), 30, LIGHT),
];

/// The balance at intensity 0, 25, 50, 75 and 100: nearly all backdrop at 0, mostly colour at 100, afterglow strongest between.
pub const INTENSITY_ROWS: [(u8, Balance); 5] = [
    (0, mix(5, 5, 90)),
    (25, mix(5, 30, 65)),
    (50, mix(20, 35, 45)),
    (75, DEEP),
    (100, mix(85, 0, 15)),
];

pub const ACTIVE: Fit = Fit {
    alpha: (0.94, 0.056),
    colour: (0.97, 5.5),
    opaque: 0.40,
};

/// Every other window's frame and caption buttons: under half the focused alpha and colour, so it reads paler.
pub const INACTIVE: Fit = Fit {
    alpha: (0.40, 0.01),
    colour: (0.39, 12.0),
    opaque: 0.15,
};

pub const TASKBAR: Fit = Fit {
    alpha: (0.96, 0.043),
    colour: (0.98, 5.5),
    opaque: 0.40,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Active,
    Inactive,
    Taskbar,
}

impl Surface {
    /// This surface's fit under `material`. An unfocused Smoked window keeps [`SMOKE_INACTIVE`] of its focused strength, so it stays dark under its white title.
    fn fit(self, material: Material) -> Fit {
        match (self, material) {
            (Surface::Active, _) => ACTIVE,
            (Surface::Inactive, Material::Smoked) => {
                let k = SMOKE_INACTIVE;
                Fit {
                    alpha: (ACTIVE.alpha.0 * k, ACTIVE.alpha.1 * k),
                    colour: (ACTIVE.colour.0 * k, ACTIVE.colour.1 * k),
                    opaque: INACTIVE.opaque,
                }
            }
            (Surface::Inactive, _) => INACTIVE,
            (Surface::Taskbar, _) => TASKBAR,
        }
    }

    /// The art's plain glass pixel for this surface (Sky, default intensity): through [`Recolour::new`] it lands on [`plain`].
    pub fn plain_px(self) -> u32 {
        self.art().0
    }

    fn art(self) -> (u32, [u8; 3]) {
        match self {
            Surface::Active => (art::PLAIN[0], art::OPAQUE_UNDER[0]),
            Surface::Inactive => (art::PLAIN[1], art::OPAQUE_UNDER[1]),
            Surface::Taskbar => (bar::BAR_PLAIN, bar::BAR_OPAQUE_UNDER),
        }
    }
}

/// The balance at `intensity` as fractions (colour, afterglow, blur), linear between the nearest known
/// intensities: Sky's five rows and each named colour's own intensity.
pub fn balance(intensity: u8) -> [f64; 3] {
    let knots = INTENSITY_ROWS
        .iter()
        .copied()
        .chain(SWATCHES.iter().map(|w| (w.intensity, w.balance)));
    let (mut lo, mut hi) = (INTENSITY_ROWS[0], INTENSITY_ROWS[4]);
    for (i, b) in knots {
        if i <= intensity && i >= lo.0 {
            lo = (i, b);
        }
        if i >= intensity && i <= hi.0 {
            hi = (i, b);
        }
    }
    let t = if hi.0 == lo.0 {
        0.0
    } else {
        f64::from(intensity - lo.0) / f64::from(hi.0 - lo.0)
    };
    let mix = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * t) / 100.0;
    [
        mix(lo.1.colour, hi.1.colour),
        mix(lo.1.afterglow, hi.1.afterglow),
        mix(lo.1.blur, hi.1.blur),
    ]
}

pub const SMOKE: Color = Color::rgb(0x0F, 0x0F, 0x11);

/// How much of the glass colour tints Smoked's grey at intensity 100; at `i` it is `i/100` of this.
pub const SMOKE_TINT: f64 = 0.25;

pub const SMOKE_BLUR: f64 = 0.7;

pub const SMOKE_INACTIVE: f64 = 0.8;

/// Molten's colour and afterglow shares against Frosted's at the same intensity; the backdrop takes the rest.
pub const MOLTEN_GLASS: f64 = 0.7;

/// What `s`'s material lays over the backdrop: the colour it tints with and its balance (colour,
/// afterglow, blur) as fractions.
///
/// Frosted is `s`'s colour at [`balance`]. Smoked lets [`SMOKE_BLUR`] of Frosted's backdrop through, no
/// afterglow; the rest is [`SMOKE`] tinted toward the colour by up to [`SMOKE_TINT`], more as intensity
/// rises. Molten keeps [`MOLTEN_GLASS`] of Frosted's colour and afterglow, so more backdrop shows.
pub fn material_glass(s: &Settings) -> ([f64; 3], [f64; 3]) {
    let c = [s.color.r, s.color.g, s.color.b].map(f64::from);
    let [cb, ab, bb] = balance(s.intensity);
    match s.material {
        Material::Frosted => (c, [cb, ab, bb]),
        Material::Smoked => {
            let k = SMOKE_TINT * f64::from(s.intensity) / 100.0;
            let grey = [SMOKE.r, SMOKE.g, SMOKE.b].map(f64::from);
            let blur = SMOKE_BLUR * bb;
            (
                std::array::from_fn(|i| grey[i] + k * (c[i] - grey[i])),
                [1.0 - blur, 0.0, blur],
            )
        }
        Material::Molten => {
            let (cm, am) = (MOLTEN_GLASS * cb, MOLTEN_GLASS * ab);
            (c, [cm, am, 1.0 - cm - am])
        }
    }
}

/// The formula's plain glass on `surface` for `s`: alpha and premultiplied colour in levels (0..255).
fn formula(surface: Surface, s: &Settings) -> (f64, [f64; 3]) {
    let f = surface.fit(s.material);
    let (c, [cb, ab, bb]) = material_glass(s);
    let m = (c[0] + c[1] + c[2]) / 3.0;
    let am = 1.0 - bb - ab * m / 255.0;
    let a = f.alpha.0 * am + f.alpha.1;
    let p = c.map(|v| {
        let pm = (cb * v + ab * (v - m) / 2.0).clamp(0.0, 255.0 * am);
        (f.colour.0 * pm + f.colour.1).clamp(0.0, 255.0 * a)
    });
    (a * 255.0, p)
}

fn sky() -> Settings {
    Settings {
        color: SWATCHES[0].color,
        ..Settings::default()
    }
}

/// How far `s` moves `surface`'s plain glass from the art's: alpha and premultiplied colour, in levels.
fn shift(s: &Settings, surface: Surface) -> (f64, [f64; 3]) {
    let d = sky();
    let (a, p) = formula(surface, s);
    let (a0, p0) = formula(surface, &d);
    (a - a0, std::array::from_fn(|i| p[i] - p0[i]))
}

pub(crate) fn split(px: u32) -> (f64, [f64; 3]) {
    let ch = |shift: u32| f64::from((px >> shift) & 0xFF);
    (ch(24), [ch(16), ch(8), ch(0)])
}

/// `surface`'s translucent plain glass for `s` (alpha, premultiplied colour, levels), moved so that Sky
/// at the default intensity is the art's own plain glass.
pub fn plain(s: &Settings, surface: Surface) -> (f64, [f64; 3]) {
    let (a0, p0) = split(surface.plain_px());
    let (da, dp) = shift(s, surface);
    let a = (a0 + da).clamp(0.0, 255.0);
    (a, std::array::from_fn(|i| (p0[i] + dp[i]).clamp(0.0, a)))
}

/// How `s`'s glass lifts the blurred backdrop under `surface`'s art. Over a backdrop B the art's plain
/// glass, `P + (1 - a)·B`, misses `afterglow·(C·luma(B) - mean(C)·B - (C - mean(C))/2)` of the
/// formula: nothing over mid grey, more colour over light backdrops and less over dark, a grey
/// glass's afterglow greying a coloured backdrop. The lift adds that, at the surface's colour scale, through what the plain glass lets pass.
pub fn lift(s: &Settings, surface: Surface) -> Lift {
    let (c, [_, afterglow, _]) = material_glass(s);
    let pass = 1.0 - plain(s, surface).0 / 255.0;
    if !s.transparency || afterglow == 0.0 || pass <= 0.0 {
        return Lift::NONE;
    }
    let c = c.map(|v| v / 255.0);
    Lift {
        colour: c.map(|v| v as f32),
        mean: ((c[0] + c[1] + c[2]) / 3.0) as f32,
        gain: (surface.fit(s.material).colour.0 * afterglow / pass) as f32,
    }
}

/// Recolours art drawn in Sky at the default intensity to a glass colour and intensity on one surface,
/// and makes it opaque with transparency off. Every art module draws its glass through [`Recolour::px`].
///
/// A pixel of the art is whatever it adds (sheen, line, glyph) over the surface's plain glass, so the
/// glass's move shows through as far as the pixel lets the glass through: a pixel of alpha `a` over
/// plain glass of alpha `a0` moves by `(255 - a) / (255 - a0)` of it. Plain glass moves fully, an
/// opaque glyph not at all, a clear pixel (outside a rounded corner) stays clear.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Recolour {
    a0: f64,
    da: f64,
    dp: [f64; 3],
    under: Option<[u8; 3]>,
}

impl Recolour {
    pub const NONE: Recolour = Recolour {
        a0: 0.0,
        da: 0.0,
        dp: [0.0; 3],
        under: None,
    };

    /// The recolour of `surface`'s art for `s`.
    ///
    /// With transparency off the plain glass lands on the art's opaque colour moved by
    /// `opaque·((255 - Sky) - r·(255 - C))` per channel, `r` the glass's colour share (1 - blur) over
    /// Sky's at the default intensity, so opaque glass pales toward light grey as the intensity falls.
    /// The rest of the art lies over the colour that puts the plain glass there.
    pub fn new(s: &Settings, surface: Surface) -> Recolour {
        let (art_px, art_under) = surface.art();
        let (a0, p0) = split(art_px);
        let (da, dp) = shift(s, surface);
        let under = (!s.transparency).then(|| {
            let d = sky();
            let (a1, p1) = plain(s, surface);
            let (c, [_, _, blur]) = material_glass(s);
            let share = (1.0 - blur) / (1.0 - balance(d.intensity)[2]);
            let c0 = [d.color.r, d.color.g, d.color.b].map(f64::from);
            let keep = 1.0 - a0 / 255.0;
            std::array::from_fn(|i| {
                let dk =
                    surface.fit(s.material).opaque * ((255.0 - c0[i]) - share * (255.0 - c[i]));
                let v = (p0[i] - p1[i] + keep * f64::from(art_under[i]) + dk)
                    / (1.0 - a1 / 255.0).max(1e-3);
                v.clamp(0.0, 255.0).round() as u8
            })
        });
        Recolour { a0, da, dp, under }
    }

    /// `px` of the art, recoloured, and made opaque over the glass colour when the glass is.
    pub fn px(&self, px: u32) -> u32 {
        let (a, p) = split(px);
        if a == 0.0 {
            return 0;
        }
        let px = if self.da == 0.0 && self.dp == [0.0; 3] {
            px
        } else {
            let w = (255.0 - a) / (255.0 - self.a0);
            let a = (a + w * self.da).clamp(0.0, 255.0).round();
            let ch = |i: usize| (p[i] + w * self.dp[i]).clamp(0.0, a).round() as u32;
            (a as u32) << 24 | ch(0) << 16 | ch(1) << 8 | ch(2)
        };
        self.finish(px)
    }

    /// `px`, glass already in this recolour's colour (built from [`plain`]), made opaque over the glass colour when the glass is.
    pub fn finish(&self, px: u32) -> u32 {
        match self.under {
            Some(u) => opaque(px, u),
            None => px,
        }
    }
}

fn opaque(px: u32, under: [u8; 3]) -> u32 {
    let a = px >> 24;
    if a == 0 {
        return 0;
    }
    let ch = |shift: u32, d: u8| {
        let p = (px >> shift) & 0xFF;
        (p + ((255 - a) * d as u32 + 127) / 255).min(255)
    };
    0xFF00_0000 | ch(16, under[0]) << 16 | ch(8, under[1]) << 8 | ch(0, under[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    const SURFACES: [Surface; 3] = [Surface::Active, Surface::Inactive, Surface::Taskbar];

    fn settings(rgb: [u8; 3], intensity: u8, transparency: bool) -> Settings {
        Settings {
            color: Color::rgb(rgb[0], rgb[1], rgb[2]),
            intensity,
            transparency,
            ..Settings::default()
        }
    }

    #[test]
    fn every_swatch_recolours_the_plain_glass_to_its_formula() {
        // Each swatch's plain glass on each surface, translucent and opaque, as an FNV-1a hash.
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for w in SWATCHES {
            let rgb = [w.color.r, w.color.g, w.color.b];
            for surface in SURFACES {
                let (plain_px, _) = surface.art();
                for transparency in [true, false] {
                    let r = Recolour::new(&settings(rgb, w.intensity, transparency), surface);
                    let px = r.px(plain_px);
                    let what = format!("{} {surface:?} translucent {transparency}", w.name);
                    if transparency {
                        let (a, p) = split(px);
                        assert!(p.iter().all(|&c| c <= a), "{what}: premultiplied");
                        // The glass leans toward its colour: a swatch with one clearly strongest channel gives the glass the same.
                        let c = rgb.map(i32::from);
                        let top = (0..3).max_by_key(|&i| c[i]).unwrap();
                        if (0..3).all(|i| i == top || c[top] - c[i] >= 40) {
                            assert!(
                                (0..3).all(|i| i == top || p[top] > p[i]),
                                "{what}: {px:#010x}"
                            );
                        }
                    } else {
                        assert_eq!(px >> 24, 0xFF, "{what}: opaque");
                    }
                    for b in px.to_le_bytes() {
                        hash = (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
                    }
                }
            }
        }
        assert_eq!(hash, 0xa421_888c_a1c0_9df1, "every swatch's plain glass");
    }

    #[test]
    fn sky_at_the_default_intensity_keeps_the_frames_pixels() {
        let d = Settings::default();
        for surface in SURFACES {
            let r = Recolour::new(&d, surface);
            let frame = art::PLAIN
                .iter()
                .chain(art::LINES.as_flattened())
                .chain(&art::SHEEN);
            for &px in frame {
                assert_eq!(r.px(px), px, "{surface:?}, the frame's {px:#010x} as it is");
            }
            let opaque = Recolour::new(
                &Settings {
                    transparency: false,
                    ..d
                },
                surface,
            );
            assert_eq!(
                opaque.under,
                Some(surface.art().1),
                "{surface:?}: the art's opaque colour"
            );
        }
    }

    #[test]
    fn every_swatch_comes_at_its_own_balance() {
        for w in SWATCHES {
            let b = balance(w.intensity);
            let want = [w.balance.colour, w.balance.afterglow, w.balance.blur];
            for i in 0..3 {
                assert!(
                    (b[i] * 100.0 - f64::from(want[i])).abs() < 1e-9,
                    "{} at {}: {want:?} percent",
                    w.name,
                    w.intensity
                );
            }
            assert_eq!(
                want.iter().map(|&v| u32::from(v)).sum::<u32>(),
                100,
                "{}'s balance sums to 100",
                w.name
            );
        }
        assert_eq!(SWATCHES[0].name, "sky", "Sky is first");
        assert_eq!(
            SWATCHES[0].intensity,
            Settings::default().intensity,
            "Sky comes at the default intensity"
        );
    }

    #[test]
    fn intensity_0_and_100_bracket_the_sky_rows() {
        assert_eq!(balance(0), [0.05, 0.05, 0.90], "intensity 0");
        assert_eq!(balance(100), [0.85, 0.0, 0.15], "intensity 100");
        for surface in SURFACES {
            let alpha = |i| plain(&settings([112, 175, 252], i, true), surface).0;
            let steps = [0, 25, 42, 50, 75, 100].map(alpha);
            assert!(
                steps.windows(2).all(|w| w[0] < w[1]),
                "{surface:?}: alpha rises through the rows, {steps:?}"
            );
            for i in 0..100 {
                assert!(alpha(i) <= alpha(i + 1), "{surface:?}: {i} to {}", i + 1);
            }
        }
    }

    #[test]
    fn a_pixel_keeps_what_it_adds_over_the_glass() {
        let ruby = Recolour::new(&settings([206, 15, 15], 75, true), Surface::Active);
        let (_, [pr, pg, _]) = split(ruby.px(art::PLAIN[0]));
        assert!(pr > pg + 90.0, "plain glass turns red: {pr} over {pg}");
        assert_eq!(ruby.px(0), 0, "a clear corner stays clear");
        assert_eq!(ruby.px(0xFFE3_E2E3), 0xFFE3_E2E3, "an opaque glyph stays");
        let (a, p) = split(ruby.px(0xD700_0307));
        let w = (255.0 - 215.0) / (255.0 - 62.0);
        assert_eq!(a, (215.0 + w * ruby.da).round(), "the outline's alpha");
        assert_eq!(p[0], (w * ruby.dp[0]).round(), "the outline's red");
        let sheen = art::SHEEN[0];
        let (sa, sp) = split(sheen);
        let (ra, rp) = split(ruby.px(sheen));
        let w = (255.0 - sa) / (255.0 - 62.0);
        assert!((0.6..0.7).contains(&w), "the sheen passes {w}");
        assert_eq!(ra, (sa + w * ruby.da).round(), "the sheen's alpha");
        assert_eq!(
            rp[1],
            (sp[1] + w * ruby.dp[1]).max(0.0).round(),
            "its green"
        );
    }

    /// `s`'s plain glass on `surface`: its alpha, and what it shows over a black and a white backdrop, in levels.
    fn over_black_and_white(s: &Settings, surface: Surface) -> (u8, [u8; 3], [u8; 3]) {
        let (a, p) = plain(s, surface);
        let over = |g: f64| p.map(|v| (v + (255.0 - a) * g / 255.0).round() as u8);
        (a.round() as u8, over(0.0), over(255.0))
    }

    fn sky_in(material: Material, intensity: u8) -> Settings {
        Settings {
            material,
            intensity,
            ..Settings::default()
        }
    }

    #[test]
    fn smoked_glass_is_a_tinted_smoke_grey_that_lets_less_through() {
        for (intensity, want) in [
            (42, (170, [16, 25, 29], [101, 110, 114])),
            (75, (197, [25, 38, 48], [83, 96, 106])),
        ] {
            let s = sky_in(Material::Smoked, intensity);
            assert_eq!(
                over_black_and_white(&s, Surface::Active),
                want,
                "Sky at {intensity}, focused: alpha, over black, over white"
            );
            let (_, _, frosted) =
                over_black_and_white(&sky_in(Material::Frosted, intensity), Surface::Active);
            assert!(
                want.2.iter().zip(frosted).all(|(&s, f)| s + 60 < f),
                "over white, {:?} is darker than Frosted's {frosted:?}",
                want.2
            );
        }
        assert_eq!(
            over_black_and_white(&sky_in(Material::Smoked, 42), Surface::Inactive),
            (148, [19, 29, 36], [126, 136, 143]),
            "an unfocused window stays dark"
        );
        for i in 0..=100 {
            let (_, [_, ag, bb]) = material_glass(&sky_in(Material::Smoked, i));
            assert_eq!(ag, 0.0, "no afterglow at {i}");
            assert!(bb < balance(i)[2], "less blur than Frosted at {i}");
        }
    }

    #[test]
    fn molten_glass_is_lighter_than_frosted() {
        for (intensity, want) in [
            (42, (47, [0, 13, 27], [208, 221, 234])),
            (75, (112, [40, 71, 101], [183, 214, 244])),
        ] {
            let s = sky_in(Material::Molten, intensity);
            assert_eq!(
                over_black_and_white(&s, Surface::Active),
                want,
                "Sky at {intensity}, focused: alpha, over black, over white"
            );
        }
        for i in 0..=100 {
            let [cf, af, bf] = balance(i);
            let (_, [cm, am, bm]) = material_glass(&sky_in(Material::Molten, i));
            assert!(
                cm < cf,
                "less colour than Frosted at {i}: {cm} against {cf}"
            );
            assert!(am <= af, "no more afterglow at {i}");
            assert!(bm > bf, "more blur than Frosted at {i}");
            assert!((cm + am + bm - 1.0).abs() < 1e-9, "the shares sum to 1");
        }
    }

    /// `s`'s plain glass on `surface` over the backdrop `b` (0..1), in levels: as the art blends over the
    /// lifted backdrop, and as it blends over `b` itself.
    fn lifted_and_linear(s: &Settings, surface: Surface, b: [f32; 3]) -> ([f64; 3], [f64; 3]) {
        let (a, p) = plain(s, surface);
        let over = |b: [f32; 3]| std::array::from_fn(|i| p[i] + (255.0 - a) * f64::from(b[i]));
        (over(lift(s, surface).over(b)), over(b))
    }

    #[test]
    fn the_lifted_plain_glass_gains_the_afterglow_the_one_alpha_fit_misses() {
        let backdrops = [
            [0.5, 0.5, 0.5],
            [0.2, 0.4, 0.7],
            [0.7, 0.5, 0.3],
            [0.25; 3],
            [0.8; 3],
        ];
        for w in [SWATCHES[0], SWATCHES[2], SWATCHES[7], SWATCHES[11]] {
            for surface in SURFACES {
                let s = settings([w.color.r, w.color.g, w.color.b], w.intensity, true);
                let (c, [cb, ab, bb]) = material_glass(&s);
                let m = (c[0] + c[1] + c[2]) / 3.0;
                let k = surface.fit(s.material).colour.0;
                for b in backdrops {
                    let b8 = b.map(|v| f64::from(v) * 255.0);
                    let l = (0.299 * b8[0] + 0.587 * b8[1] + 0.114 * b8[2]) / 255.0;
                    let (got, linear) = lifted_and_linear(&s, surface, b);
                    for i in 0..3 {
                        let formula = bb * b8[i] + cb * c[i] + ab * c[i] * l;
                        let fit =
                            (cb * c[i] + ab * (c[i] - m) / 2.0) + (bb + ab * m / 255.0) * b8[i];
                        let want = linear[i] + k * (formula - fit);
                        assert!(
                            (got[i] - want).abs() < 0.01,
                            "{} {surface:?} over {b:?}, channel {i}: {want}, got {}",
                            w.name,
                            got[i]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn light_backdrops_show_more_of_the_colour_and_dark_ones_less() {
        let sky = Settings::default();
        let shift = |g: f32| {
            let (got, linear) = lifted_and_linear(&sky, Surface::Active, [g; 3]);
            [got[0] - linear[0], got[2] - linear[2]]
        };
        assert_eq!(shift(0.5), [0.0, 0.0], "mid grey: the fit itself");
        let [r, b] = shift(0.85);
        assert!(r < -8.0 && b > 8.0, "over light grey, bluer: {r} {b}");
        let [r, b] = shift(0.15);
        assert!(r > 8.0 && b < -8.0, "over dark grey, less blue: {r} {b}");
    }

    #[test]
    fn grey_glass_greys_only_a_coloured_backdrop_and_smoked_or_opaque_glass_lifts_nothing() {
        let slate = lift(&settings([83, 83, 83], 53, true), Surface::Active);
        for g in [0.1, 0.5, 0.9] {
            let got = slate.over([g; 3]);
            assert!(
                got.iter().all(|v| (v - g).abs() < 1e-6),
                "Slate over grey {g}: {got:?}"
            );
        }
        let b = [0.1, 0.6, 0.9];
        let got = slate.over(b);
        assert!(
            got[0] > b[0] && got[2] < b[2],
            "Slate over {b:?} greys it: {got:?}"
        );
        assert_eq!(
            lift(&sky_in(Material::Smoked, 75), Surface::Active),
            Lift::NONE
        );
        assert_eq!(
            lift(&settings([112, 175, 252], 42, false), Surface::Active),
            Lift::NONE
        );
        assert_eq!(
            lift(&sky_in(Material::Frosted, 100), Surface::Active),
            Lift::NONE,
            "no afterglow at 100"
        );
        let molten = lift(&sky_in(Material::Molten, 42), Surface::Active).gain;
        let frosted = lift(&sky_in(Material::Frosted, 42), Surface::Active).gain;
        assert!(
            molten > 0.0 && molten < frosted,
            "Molten lifts less: {molten} {frosted}"
        );
    }

    #[test]
    fn frosted_glass_is_its_colour_at_its_balance() {
        for i in [0, 42, 75, 100] {
            let s = sky_in(Material::Frosted, i);
            let c = [s.color.r, s.color.g, s.color.b].map(f64::from);
            assert_eq!(material_glass(&s), (c, balance(i)), "Sky at {i}");
        }
    }
}
