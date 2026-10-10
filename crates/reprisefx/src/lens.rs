//! Molten's lens: near a shape's outer edge the glass reads the backdrop
//! from farther out along the edge's normal, so what lies just past the
//! edge bends into it, and the edge catches the light.
//!
//! At depth `d` inside the edge the lookup moves out by
//! `strength · profile(d)`, where `profile` is 1 at the edge and eases to
//! 0 at the band's depth with zero slope at both ends. The band widens
//! with the shape, up to [`BAND`], and never passes half its thinner
//! side, so the bend is gone before the shape's middle where the normal
//! turns. An edge on or past the output's border has nothing beyond it
//! to bend and gets neither bend nor light: the shader's lens box is the
//! shape pushed out past such edges.
//!
//! The glass shader's Molten branch is this module's arithmetic; the
//! tests here pin it on the CPU.
//!
//! # Tuning
//!
//! Every knob is a constant below; the shader takes them by name. What
//! each one does to the look, at the value it ships with:
//!
//! - Bend. [`BAND`] 32 px deep, reaching [`BEND`] 1.25 of it out: a
//!   window's whole caption bends; past the band the backdrop lies flat.
//! - Dispersion. Red bends [`DISPERSION`] 6% farther than green, blue 6%
//!   less: a faint red/blue fringe where the bend is strongest, nothing
//!   past the band. Above ~0.15 it reads as a broken lens.
//! - Vibrancy. [`VIBRANCE`] 1.4 pushes the backdrop's colours away from
//!   their grey before the tint: what lies behind looks richer than it
//!   does through Frosted. 1.0 turns it off.
//! - Light. It comes from up and to the left, [`LIGHT_ANGLE`] 35° off
//!   straight up. An edge facing it gets the full highlight, the edge
//!   facing away [`SPEC_BACK`] 0.7 of it (light bounced inside the
//!   glass), so the top-left and bottom-right corners shine brightest.
//!   [`SPEC_POWER`] 2 narrows the bright arcs; an edge square to the
//!   light keeps [`SPEC_FLOOR`] 0.2. A horizontal top edge gets ~0.74, a
//!   left edge ~0.47.
//! - Rim. [`RIM`] 0.8 at the brightest point, two layers: a crisp line
//!   falling off as `exp(-d / RIM_FADE)` ([`RIM_FADE`] 1 px) and a soft
//!   glow [`GLOW`] 10 px deep at [`GLOW_SHARE`] 0.3 of it. The light
//!   screens over the glass rather than adding, so it never clips flat
//!   white, and it leans [`SPEC_TINT`] 35% toward the backdrop's own hue.
//! - Thickness. A dark band [`SHADE`] 12% deep, centred [`SHADE_AT`] 4 px
//!   in and [`SHADE_WIDTH`] 2 px wide, just inside the rim: the edge reads
//!   as rounded glass rather than a painted line. 0 turns it off.
//!
//! Lengths are logical pixels, scaled to the output.

use reprisekit::glass::Material;
use smithay::utils::{Physical, Rectangle, Size};

/// The widest band, in logical pixels: a window's whole caption and the
/// taskbar's top half bend; a window's 8 px sides bend all across.
pub const BAND: f32 = 32.0;

pub const BEND: f32 = 1.25;

pub const DISPERSION: f32 = 0.06;

pub const VIBRANCE: f32 = 1.4;

pub const LIGHT_ANGLE: f32 = 35.0;

pub const SPEC_POWER: f32 = 2.0;

pub const SPEC_FLOOR: f32 = 0.2;

pub const SPEC_BACK: f32 = 0.7;

pub const SPEC_TINT: f32 = 0.35;

pub const RIM: f32 = 0.8;

pub const RIM_FADE: f32 = 1.0;

pub const GLOW: f32 = 10.0;

pub const GLOW_SHARE: f32 = 0.3;

pub const SHADE: f32 = 0.12;

pub const SHADE_AT: f32 = 4.0;

pub const SHADE_WIDTH: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    pub band: f32,
    pub strength: f32,
    pub rim: f32,
    pub scale: f32,
    /// The box whose edges bend, shape-local physical pixels: the shape,
    /// pushed past every edge on or beyond the output's border.
    pub area: [f32; 4],
}

impl Lens {
    /// Molten's lens over `shape` (output-local physical pixels) on an
    /// output of `output` pixels at `scale`. None for the other
    /// materials, which do not bend.
    pub fn new(
        material: Material,
        shape: Rectangle<i32, Physical>,
        output: Size<i32, Physical>,
        scale: f64,
    ) -> Option<Lens> {
        if material != Material::Molten {
            return None;
        }
        let band = band(shape.size, scale);
        if band <= 0.0 {
            return None;
        }
        let strength = BEND * band;
        // Far enough that the pushed edge's band never reaches the shape.
        let away = band + strength;
        let (w, h) = (shape.size.w as f32, shape.size.h as f32);
        let (mut x0, mut y0, mut x1, mut y1) = (0.0, 0.0, w, h);
        if shape.loc.x <= 0 {
            x0 -= away;
        }
        if shape.loc.y <= 0 {
            y0 -= away;
        }
        if shape.loc.x + shape.size.w >= output.w {
            x1 += away;
        }
        if shape.loc.y + shape.size.h >= output.h {
            y1 += away;
        }
        Some(Lens {
            band,
            strength,
            rim: RIM,
            scale: scale as f32,
            area: [x0, y0, x1 - x0, y1 - y0],
        })
    }

    pub fn bend(&self, depth: f32) -> f32 {
        self.strength * profile(depth, self.band)
    }

    /// How far past the shape the lookup can read (red bends farthest):
    /// the element's geometry grows by this on top of the blur's reach.
    pub fn reach(&self) -> i32 {
        (self.strength * (1.0 + DISPERSION)).ceil() as i32
    }

    /// The highlight at `depth` physical pixels inside an edge whose
    /// outward normal is `n`: the crisp rim plus the soft glow, lit by
    /// [`spec`].
    pub fn light(&self, n: [f32; 2], depth: f32) -> f32 {
        let rim = (-depth / (RIM_FADE * self.scale)).exp();
        let glow = 1.0 - smoothstep(depth / (GLOW * self.scale));
        self.rim * spec(n) * (rim + GLOW_SHARE * glow)
    }

    pub fn shade(&self, depth: f32) -> f32 {
        let s = (depth - SHADE_AT * self.scale) / (SHADE_WIDTH * self.scale);
        SHADE * (-s * s).exp()
    }
}

pub fn light_dir() -> [f32; 2] {
    let a = LIGHT_ANGLE.to_radians();
    [-a.sin(), -a.cos()]
}

/// How much of the highlight an edge with outward normal `n` catches:
/// 1 facing the light, [`SPEC_BACK`] facing straight away, [`SPEC_FLOOR`]
/// (of either) square to it.
pub fn spec(n: [f32; 2]) -> f32 {
    let l = light_dir();
    let facing = n[0] * l[0] + n[1] * l[1];
    let s = SPEC_FLOOR + (1.0 - SPEC_FLOOR) * facing.abs().max(1e-4).powf(SPEC_POWER);
    if facing < 0.0 { s * SPEC_BACK } else { s }
}

pub fn vibrant(c: [f32; 3]) -> [f32; 3] {
    let l = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    c.map(|v| (l + (v - l) * VIBRANCE).clamp(0.0, 1.0))
}

/// The highlight `light` screened over the glass colour `c`, leaning
/// [`SPEC_TINT`] toward the backdrop `b`'s hue.
pub fn screen(c: [f32; 3], b: [f32; 3], light: f32) -> [f32; 3] {
    let top = b[0].max(b[1]).max(b[2]).max(1e-3);
    let light = light.clamp(0.0, 1.0);
    std::array::from_fn(|i| {
        let hl = light * (1.0 + (b[i] / top - 1.0) * SPEC_TINT);
        1.0 - (1.0 - c[i]) * (1.0 - hl)
    })
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// The band for a shape of `size` physical pixels at `scale`: [`BAND`],
/// or half the shape's thinner side when that is less.
pub fn band(size: Size<i32, Physical>, scale: f64) -> f32 {
    let half = size.w.min(size.h) as f32 / 2.0;
    (BAND * scale as f32).min(half).max(0.0)
}

/// The bend's profile: 1 at the edge, 0 from `band` in, easing with zero
/// slope at both ends: `(1 - smoothstep(0, band, depth))²`.
pub fn profile(depth: f32, band: f32) -> f32 {
    let t = 1.0 - smoothstep(depth / band);
    t * t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    const OUT: (i32, i32) = (1920, 1080);

    #[test]
    fn the_profile_eases_from_the_edge_to_the_band() {
        assert_eq!(profile(0.0, 24.0), 1.0);
        assert_eq!(profile(24.0, 24.0), 0.0);
        assert_eq!(profile(40.0, 24.0), 0.0, "nothing past the band");
        assert_eq!(profile(12.0, 24.0), 0.25, "a quarter at half the band");
        let row: Vec<f32> = (0..=24).map(|d| profile(d as f32, 24.0)).collect();
        assert!(
            row.windows(2).all(|w| w[1] < w[0]),
            "falls row by row: {row:?}"
        );
        let slope = |d: f32| (profile(d + 0.01, 24.0) - profile(d, 24.0)) / 0.01;
        assert!(slope(0.0).abs() < 1e-3, "flat at the edge");
        assert!(slope(23.99).abs() < 1e-3, "flat at the band");
    }

    #[test]
    fn the_band_widens_with_the_shape_up_to_its_cap() {
        let s = |w, h| Size::from((w, h));
        assert_eq!(band(s(800, 600), 1.0), BAND, "a window: the full band");
        assert_eq!(band(s(1920, 40), 1.0), 20.0, "a 40 px taskbar: half");
        assert_eq!(band(s(1920, 30), 1.0), 15.0);
        assert_eq!(band(s(800, 600), 2.0), 2.0 * BAND, "scaled");
        assert_eq!(band(s(0, 40), 1.0), 0.0);
    }

    #[test]
    fn only_molten_bends() {
        let r = rect(100, 100, 400, 300);
        let out = OUT.into();
        assert_eq!(Lens::new(Material::Frosted, r, out, 1.0), None);
        assert_eq!(Lens::new(Material::Smoked, r, out, 1.0), None);
        let l = Lens::new(Material::Molten, r, out, 1.0).unwrap();
        assert_eq!((l.band, l.strength, l.rim), (BAND, BEND * BAND, RIM));
        assert_eq!(l.scale, 1.0);
        assert_eq!(l.area, [0.0, 0.0, 400.0, 300.0], "every edge bends");
        assert_eq!(l.bend(0.0), l.strength);
        assert_eq!(l.bend(l.band), 0.0);
        assert_eq!(l.reach(), 43, "red's lookup, 6% past the 40 px bend");
    }

    #[test]
    fn edges_on_the_output_border_do_not_bend() {
        let out = OUT.into();
        let bar = Lens::new(Material::Molten, rect(0, 1040, 1920, 40), out, 1.0).unwrap();
        let away = bar.band + bar.strength;
        assert_eq!(
            bar.area,
            [-away, 0.0, 1920.0 + 2.0 * away, 40.0 + away],
            "a bottom taskbar bends along its top edge alone"
        );
        let past = Lens::new(Material::Molten, rect(-50, 900, 400, 300), out, 1.0).unwrap();
        let away = past.band + past.strength;
        assert_eq!(
            past.area,
            [-away, 0.0, 400.0 + away, 300.0 + away],
            "a window hanging off the left and bottom bends at its top and right"
        );
    }

    const UP: [f32; 2] = [0.0, -1.0];
    const LEFT: [f32; 2] = [-1.0, 0.0];
    const DOWN: [f32; 2] = [0.0, 1.0];
    const RIGHT: [f32; 2] = [1.0, 0.0];

    #[test]
    fn the_light_shines_brightest_up_left_and_bounces_to_down_right() {
        let l = light_dir();
        let away = [-l[0], -l[1]];
        assert!((spec(l) - 1.0).abs() < 1e-6, "facing the light: full");
        assert!((spec(away) - SPEC_BACK).abs() < 1e-6, "facing away");
        assert!((spec(UP) - 0.737).abs() < 1e-3, "a top edge: {}", spec(UP));
        assert!(
            (spec(LEFT) - 0.463).abs() < 1e-3,
            "a left edge: {}",
            spec(LEFT)
        );
        assert!(spec(DOWN) < spec(UP) && spec(RIGHT) < spec(LEFT));
        let square = [l[1], -l[0]];
        assert!((spec(square) - SPEC_FLOOR).abs() < 1e-3, "square to it");
    }

    #[test]
    fn the_rim_is_a_crisp_line_over_a_soft_glow() {
        let l = Lens::new(Material::Molten, rect(100, 100, 400, 300), OUT.into(), 1.0).unwrap();
        let at = |d: f32| l.light(UP, d) / (l.rim * spec(UP));
        assert!((at(0.0) - 1.3).abs() < 1e-6, "rim and glow at the edge");
        assert!(
            at(1.0) < 0.67,
            "the line has mostly gone 1 px in: {}",
            at(1.0)
        );
        assert!(at(5.0) > 0.1, "the glow still shows 5 px in: {}", at(5.0));
        assert!((at(GLOW) - (-GLOW).exp()).abs() < 1e-6, "gone at its depth");
        let l2 = Lens::new(Material::Molten, rect(100, 100, 800, 600), OUT.into(), 2.0).unwrap();
        assert_eq!(l2.light(UP, 10.0), l.light(UP, 5.0), "lengths scale");
    }

    #[test]
    fn the_thickness_band_lies_just_inside_the_rim() {
        let l = Lens::new(Material::Molten, rect(100, 100, 400, 300), OUT.into(), 1.0).unwrap();
        assert_eq!(l.shade(SHADE_AT), SHADE);
        assert!(l.shade(0.0) < 0.02 && l.shade(10.0) < 0.002);
    }

    #[test]
    fn vibrancy_pushes_colours_from_their_grey() {
        assert_eq!(vibrant([0.5; 3]), [0.5; 3], "greys stay");
        let [r, g, b] = vibrant([0.6, 0.4, 0.4]);
        assert!(r > 0.6 && g < 0.4 && b < 0.4, "{r} {g} {b}");
        assert_eq!(vibrant([1.0, 0.0, 0.0])[0], 1.0, "clamped");
    }

    #[test]
    fn the_highlight_screens_and_takes_the_backdrops_hue() {
        let dark = screen([0.4; 3], [0.4; 3], 0.0);
        assert!(
            dark.iter().all(|c| (c - 0.4).abs() < 1e-6),
            "no light: {dark:?}"
        );
        let [g] = [screen([0.4; 3], [0.4; 3], 0.5)[0]];
        assert!((g - 0.7).abs() < 1e-6, "screen, not add: {g}");
        assert_eq!(screen([0.9; 3], [0.9; 3], 1.0), [1.0; 3]);
        let [r, _, b] = screen([0.2; 3], [0.8, 0.2, 0.2], 0.6);
        assert!(r > b, "over red the highlight is reddish: {r} against {b}");
    }
}
