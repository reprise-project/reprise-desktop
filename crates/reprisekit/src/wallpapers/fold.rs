//! Fold: one sheet of paper pleated into a zigzag tessellation, flat in the top-left corner and folding deeper to the right.

use std::f32::consts::TAU;

use super::noise::Rng;
use super::noise::form::perlin;
use super::tone::{C, Ramp, hex, mul, quantise, sat as clamp01, smooth as smoothstep};
use super::{Variant, each};

pub(super) fn render(w: usize, h: usize, v: Variant, seed: u32) -> Vec<u32> {
    let var = match v {
        Variant::Named(n) => n,
        _ => "teal",
    };
    quantise(&draw(w, h, var, seed), w, seed)
}

struct FoldPalette {
    ramp: [(f32, u32); 4],
    grain: f32,
    accent: u32,
}

fn palette(v: &str) -> FoldPalette {
    match v {
        "teal" => FoldPalette {
            ramp: [
                (0.0, 0x0a3a3e),
                (0.45, 0x1b6764),
                (0.75, 0x4f9790),
                (1.0, 0xbfe0d3),
            ],
            grain: 0.012,
            accent: 0xe3aa48,
        },
        "steel" => FoldPalette {
            ramp: [
                (0.0, 0x13304f),
                (0.45, 0x2b5a8c),
                (0.75, 0x5f8cbc),
                (1.0, 0xd3e1ee),
            ],
            grain: 0.012,
            accent: 0xe8826a,
        },
        "sand" => FoldPalette {
            ramp: [
                (0.0, 0x8a7357),
                (0.45, 0xc4b08f),
                (0.75, 0xe2d5bd),
                (1.0, 0xfbf7ef),
            ],
            grain: 0.01,
            accent: 0xc4512a,
        },
        _ => FoldPalette {
            ramp: [
                (0.0, 0x0b0c12),
                (0.45, 0x1b1d2a),
                (0.75, 0x343a58),
                (1.0, 0x8094d0),
            ],
            grain: 0.01,
            accent: 0xff9e64,
        },
    }
}

fn tri(u: f32) -> (f32, f32) {
    let f = u - u.floor();
    if f < 0.5 {
        (2.0 * f, 2.0)
    } else {
        (2.0 - 2.0 * f, -2.0)
    }
}

fn draw(w: usize, h: usize, v: &str, seed: u32) -> Vec<C> {
    let pal = palette(v);
    let ramp = Ramp::hex(&pal.ramp);
    let mut rng = Rng(seed as u64 ^ 0xf01d);
    let phase = (rng.f(), rng.f());
    let px = 1.0 / h as f32;
    // Pleat width, row height, how far each row's zigzag swings, and the
    // depth of the folds at the right edge.
    let (pw, ph, swing, depth) = (0.19f32, 0.31f32, 0.36f32, 0.5f32);
    // The pattern lies at a slant, and widens a little to the right.
    let (rc, rs) = ((-0.33f32).cos(), (-0.33f32).sin());
    let l = {
        let (x, y, z) = (-0.3f32, -0.75f32, 0.6f32);
        let n = (x * x + y * y + z * z).sqrt();
        (x / n, y / n, z / n)
    };
    let accent = hex(pal.accent);
    // Tone at a point, the facet it lies on (half-row, half-pleat) and
    // whether a crease passes within a pixel or two.
    let tone = |x: f32, y: f32| -> (f32, (i32, i32), bool) {
        // Fold depth: a wave of folding crossing the sheet on a diagonal,
        // flat in the top-left corner and easing off at the bottom right.
        let band = (x * 0.75 + y * 0.66 - 0.3) / 0.42;
        let a = depth * (-band * band).exp() * smoothstep(-0.9, 0.2, x);
        let (x, y) = (rc * x - rs * y, rs * x + rc * y);
        // Rows breathe: some pleats sit deeper than others.
        let a = a * (0.78 + 0.22 * (2.1 * y + 1.4 * x + phase.0 * TAU).sin());
        let x = x + 0.12 * x * x;
        let r = y / ph + phase.1;
        let (fy, dfy) = tri(r);
        let u = (x + swing * ph * fy) / pw + phase.0;
        let (fu, dfu) = tri(u);
        let gx = a * dfu * 0.5;
        let gy = a * dfu * 0.5 * swing * dfy;
        let nz = 1.0 / (gx * gx + gy * gy + 1.0).sqrt();
        let shade = (-gx * l.0 - gy * l.1 + l.2) * nz;
        let mut t = 0.56 + (shade - l.2) * 0.9;
        // Valleys (fu = 0) darken softly; ridges (fu = 1) catch a hairline.
        let dv = fu * pw * 0.5;
        t -= 0.4 * a * (-dv / 0.02).exp();
        let dr = (1.0 - fu) * pw * 0.5;
        t += 0.06 * (a / depth) * (1.0 - clamp01(dr / px - 0.3));
        let (r2, u2) = (2.0 * r, 2.0 * u);
        let edge = |v: f32, size: f32| (v - v.round()).abs() * size < 2.5 * px;
        let near = edge(u2, pw * 0.5 / (1.0 + swing)) || edge(r2, ph * 0.5);
        (t, (r2.floor() as i32, u2.floor() as i32), near)
    };
    let pick = tone(0.47, -0.2).1;
    let paint = |t: f32, id: (i32, i32)| {
        if id == pick {
            mul(accent, 0.86 + 0.45 * (t - 0.5))
        } else {
            ramp.at(t)
        }
    };
    each(w, h, |_, _, x, y| {
        let lift =
            0.07 * (-x * 0.25 - y * 0.7) + pal.grain * perlin(x * 420.0, y * 420.0, seed ^ 0x9a1);
        let (t, id, near) = tone(x, y);
        if !near {
            return paint(t + lift, id);
        }
        let o = 0.25 * px;
        let mut c = [0.0; 3];
        for (sx, sy) in [(-o, -o), (o, -o), (-o, o), (o, o)] {
            let (t, id, _) = tone(x + sx, y + sy);
            let k = paint(t + lift, id);
            c = [c[0] + k[0] * 0.25, c[1] + k[1] * 0.25, c[2] + k[2] * 0.25];
        }
        c
    })
}
