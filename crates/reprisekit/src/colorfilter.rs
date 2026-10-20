//! Colour filters over the whole screen (`[accessibility] color_filter`):
//! grayscale, invert, and three colour-blindness corrections. Each is one
//! affine map `out = M · rgb + offset` on 0..1 channels, clamped. The
//! compositor applies it on the GPU; the settings preview applies it here.
//!
//! Corrections are daltonization in linear light: simulate the eye (`S`, via
//! LMS), take the lost colour (`rgb - S·rgb`) and add it into channels still
//! told apart (`D`), so the matrix is `I + k·D·(I - S)`, `k` the intensity (0
//! no change, 1 full correction). Neutral greys are unchanged. Grayscale and
//! invert work on the sRGB-encoded values.

use crate::wallpapers::tone::{to_linear, to_srgb};

/// One `[accessibility] color_filter` choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    None,
    Grayscale,
    Invert,
    /// Red-green, green cones missing.
    Deuteranopia,
    /// Red-green, red cones missing.
    Protanopia,
    /// Blue-yellow, blue cones missing.
    Tritanopia,
}

impl Filter {
    pub const ALL: [Filter; 6] = [
        Filter::None,
        Filter::Grayscale,
        Filter::Invert,
        Filter::Deuteranopia,
        Filter::Protanopia,
        Filter::Tritanopia,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Filter::None => "none",
            Filter::Grayscale => "grayscale",
            Filter::Invert => "invert",
            Filter::Deuteranopia => "deuteranopia",
            Filter::Protanopia => "protanopia",
            Filter::Tritanopia => "tritanopia",
        }
    }

    pub fn parse(name: &str) -> Option<Filter> {
        Filter::ALL.into_iter().find(|f| f.name() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            Filter::None => "None",
            Filter::Grayscale => "Grayscale",
            Filter::Invert => "Inverted",
            Filter::Deuteranopia => "Red-green (deutan)",
            Filter::Protanopia => "Red-green (protan)",
            Filter::Tritanopia => "Blue-yellow (tritan)",
        }
    }

    pub fn takes_intensity(self) -> bool {
        matches!(
            self,
            Filter::Deuteranopia | Filter::Protanopia | Filter::Tritanopia
        )
    }
}

/// An affine colour map, row by row: `out[i] = rows[i][0..3] · rgb + rows[i][3]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix {
    pub rows: [[f32; 4]; 3],
    /// The map takes and gives linear light: channels are sRGB-decoded before it and encoded after.
    pub linear: bool,
}

type M3 = [[f64; 3]; 3];

const IDENTITY: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// RGB to the LMS cone responses (Vienot, Brettel and Mollon).
const RGB_TO_LMS: M3 = [
    [17.8824, 43.5161, 4.11935],
    [3.45565, 27.1554, 3.86714],
    [0.0299566, 0.184309, 1.46709],
];

/// What an eye missing one kind of cone sees, in LMS: that response made from the other two.
const PROTAN_LMS: M3 = [[0.0, 2.02344, -2.52581], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
const DEUTAN_LMS: M3 = [[1.0, 0.0, 0.0], [0.494207, 0.0, 1.24827], [0.0, 0.0, 1.0]];
const TRITAN_LMS: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [-0.395913, 0.801109, 0.0]];

/// Where the lost colour goes: red-green into green and blue, blue-yellow into red and green.
const RED_GREEN_SHIFT: M3 = [[0.0, 0.0, 0.0], [0.7, 1.0, 0.0], [0.7, 0.0, 1.0]];
const BLUE_YELLOW_SHIFT: M3 = [[1.0, 0.0, 0.7], [0.0, 1.0, 0.7], [0.0, 0.0, 0.0]];

/// Rec. 709 luma weights.
const LUMA: [f64; 3] = [0.2126, 0.7152, 0.0722];

fn mul(a: &M3, b: &M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

fn add(a: &M3, b: &M3, k: f64) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| a[i][j] + k * b[i][j]))
}

fn inverse(m: &M3) -> M3 {
    let c = |r: usize, s: usize| {
        let (r0, r1) = ((r + 1) % 3, (r + 2) % 3);
        let (s0, s1) = ((s + 1) % 3, (s + 2) % 3);
        m[r0][s0] * m[r1][s1] - m[r0][s1] * m[r1][s0]
    };
    let det: f64 = (0..3).map(|j| m[0][j] * c(0, j)).sum();
    std::array::from_fn(|i| std::array::from_fn(|j| c(j, i) / det))
}

fn simulation(lms: &M3) -> M3 {
    mul(&inverse(&RGB_TO_LMS), &mul(lms, &RGB_TO_LMS))
}

fn linear_light(m: M3) -> Matrix {
    Matrix {
        rows: std::array::from_fn(|i| [m[i][0] as f32, m[i][1] as f32, m[i][2] as f32, 0.0]),
        linear: true,
    }
}

/// The map `filter` draws with at `intensity` (0 to 1, corrections only); None for no filter.
pub fn matrix(filter: Filter, intensity: f64) -> Option<Matrix> {
    let k = intensity.clamp(0.0, 1.0);
    let correction = |lms: &M3, shift: &M3| {
        let lost = add(&IDENTITY, &simulation(lms), -1.0);
        linear_light(add(&IDENTITY, &mul(shift, &lost), k))
    };
    Some(match filter {
        Filter::None => return None,
        Filter::Grayscale => {
            let row = [LUMA[0] as f32, LUMA[1] as f32, LUMA[2] as f32, 0.0];
            Matrix {
                rows: [row; 3],
                linear: false,
            }
        }
        Filter::Invert => Matrix {
            rows: [
                [-1.0, 0.0, 0.0, 1.0],
                [0.0, -1.0, 0.0, 1.0],
                [0.0, 0.0, -1.0, 1.0],
            ],
            linear: false,
        },
        Filter::Deuteranopia => correction(&DEUTAN_LMS, &RED_GREEN_SHIFT),
        Filter::Protanopia => correction(&PROTAN_LMS, &RED_GREEN_SHIFT),
        Filter::Tritanopia => correction(&TRITAN_LMS, &BLUE_YELLOW_SHIFT),
    })
}

impl Matrix {
    /// `rgb` (0 to 1) through the map, clamped; what the shader computes.
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let v = if self.linear { rgb.map(to_linear) } else { rgb };
        let out =
            (self.rows).map(|r| (r[0] * v[0] + r[1] * v[1] + r[2] * v[2] + r[3]).clamp(0.0, 1.0));
        if self.linear { out.map(to_srgb) } else { out }
    }

    pub fn apply_argb(&self, px: u32) -> u32 {
        let ch = |s: u32| ((px >> s) & 0xff) as f32 / 255.0;
        let [r, g, b] = self.apply([ch(16), ch(8), ch(0)]);
        let q = |v: f32| (v * 255.0).round() as u32;
        (px & 0xff00_0000) | q(r) << 16 | q(g) << 8 | q(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORRECTIONS: [(Filter, &M3); 3] = [
        (Filter::Deuteranopia, &DEUTAN_LMS),
        (Filter::Protanopia, &PROTAN_LMS),
        (Filter::Tritanopia, &TRITAN_LMS),
    ];

    fn close(a: [f32; 3], b: [f32; 3], eps: f32) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() <= eps)
    }

    fn see(m: &M3, rgb: [f32; 3]) -> [f32; 3] {
        let v = rgb.map(f64::from);
        std::array::from_fn(|i| (0..3).map(|k| m[i][k] * v[k]).sum::<f64>() as f32)
    }

    fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
        (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt()
    }

    #[test]
    fn names_round_trip_and_none_draws_nothing() {
        for f in Filter::ALL {
            assert_eq!(Filter::parse(f.name()), Some(f));
        }
        assert_eq!(Filter::parse("sepia"), None);
        assert_eq!(matrix(Filter::None, 1.0), None);
        assert_eq!(Filter::default(), Filter::None);
    }

    #[test]
    fn grayscale_gives_equal_channels_by_luma() {
        let m = matrix(Filter::Grayscale, 1.0).unwrap();
        assert_eq!(m.apply([1.0; 3]), [1.0; 3], "white stays white");
        let red = m.apply([1.0, 0.0, 0.0]);
        assert_eq!(red, [0.2126; 3]);
        assert_eq!(
            m.apply_argb(0xff00_ff00),
            0xffb6_b6b6,
            "green is light grey"
        );
    }

    #[test]
    fn invert_swaps_black_and_white() {
        let m = matrix(Filter::Invert, 0.0).unwrap();
        assert_eq!(m.apply_argb(0xff00_0000), 0xffff_ffff);
        assert_eq!(m.apply_argb(0xffff_ffff), 0xff00_0000);
        assert_eq!(m.apply_argb(0x80ff_8000), 0x8000_7fff, "alpha kept");
    }

    #[test]
    fn corrections_leave_greys_alone() {
        for (f, _) in CORRECTIONS {
            let m = matrix(f, 1.0).unwrap();
            for v in [0.0, 0.25, 0.5, 1.0] {
                let got = m.apply([v; 3]);
                assert!(
                    close(got, [v; 3], 0.01),
                    "{f:?} turns grey {v} into {got:?}"
                );
            }
        }
    }

    #[test]
    fn intensity_zero_is_no_change() {
        for (f, _) in CORRECTIONS {
            let m = matrix(f, 0.0).unwrap();
            let c = [0.8, 0.3, 0.1];
            assert!(close(m.apply(c), c, 1e-5), "{f:?}");
        }
    }

    /// What the eye sees of sRGB-encoded `rgb`, in linear light.
    fn see_encoded(m: &M3, rgb: [f32; 3]) -> [f32; 3] {
        see(m, rgb.map(to_linear))
    }

    /// Colours the eye confuses (red/green, blue/green-grey) come apart once corrected.
    #[test]
    fn corrections_separate_colours_the_eye_confuses() {
        let pairs = [
            ([0.8, 0.2, 0.2], [0.4, 0.5, 0.2]),
            ([0.8, 0.2, 0.2], [0.4, 0.5, 0.2]),
            ([0.3, 0.3, 0.9], [0.3, 0.5, 0.5]),
        ];
        for ((f, lms), (a, b)) in CORRECTIONS.into_iter().zip(pairs) {
            let eye = simulation(lms);
            let before = distance(see_encoded(&eye, a), see_encoded(&eye, b));
            let m = matrix(f, 1.0).unwrap();
            let after = distance(see_encoded(&eye, m.apply(a)), see_encoded(&eye, m.apply(b)));
            assert!(
                after > before * 1.2,
                "{f:?}: {before} apart before, {after} after"
            );
        }
    }

    #[test]
    fn corrections_map_linear_light_and_grayscale_and_invert_the_encoded_values() {
        for f in Filter::ALL.into_iter().skip(1) {
            let m = matrix(f, 1.0).unwrap();
            assert_eq!(m.linear, f.takes_intensity(), "{f:?}");
        }
        let m = matrix(Filter::Protanopia, 1.0).unwrap();
        let c = [0.8, 0.3, 0.1];
        let lin = c.map(to_linear);
        let want = m
            .rows
            .map(|r| to_srgb(r[0] * lin[0] + r[1] * lin[1] + r[2] * lin[2]));
        assert!(close(m.apply(c), want, 1e-6), "decoded, mapped, encoded");
    }

    #[test]
    fn corrections_give_these_pixels() {
        let colours = [0xffff_0000, 0xff00_ff00, 0xff00_00ff, 0x80c0_4020];
        let want = [
            (
                Filter::Deuteranopia,
                [0xffff_7cbe, 0xff00_e700, 0xff00_00ff, 0x80c0_6b8c],
            ),
            (
                Filter::Protanopia,
                [0xffff_bdce, 0xff00_ba00, 0xff00_00ff, 0x80c0_9397],
            ),
            (
                Filter::Tritanopia,
                [0xffff_ff00, 0xff00_0000, 0xff00_00ff, 0x80ff_ea20],
            ),
        ];
        let got = want.map(|(f, _)| {
            let m = matrix(f, 1.0).unwrap();
            (f, colours.map(|c| m.apply_argb(c)))
        });
        assert_eq!(got, want, "{got:08x?}");
    }

    #[test]
    fn half_intensity_is_halfway() {
        let full = matrix(Filter::Protanopia, 1.0).unwrap();
        let half = matrix(Filter::Protanopia, 0.5).unwrap();
        for i in 0..3 {
            for j in 0..3 {
                let id = if i == j { 1.0 } else { 0.0 };
                let want = id + 0.5 * (full.rows[i][j] - id);
                assert!((half.rows[i][j] - want).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn the_inverse_inverts() {
        let p = mul(&RGB_TO_LMS, &inverse(&RGB_TO_LMS));
        for (i, row) in p.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                let id = if i == j { 1.0 } else { 0.0 };
                assert!((v - id).abs() < 1e-9);
            }
        }
    }
}
