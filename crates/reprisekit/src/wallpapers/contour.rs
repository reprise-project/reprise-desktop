//! Contour: contour lines of a warped terrain broken by a fault.

use super::noise::form::{Field, fbm};
use super::tone::{C, Ramp, mix, mul, quantise, sat as clamp01};
use super::{Variant, par_rows};

pub(super) fn render(w: usize, h: usize, v: Variant, seed: u32) -> Vec<u32> {
    let var = match v {
        Variant::Named(n) => n,
        _ => "teal",
    };
    quantise(&draw(w, h, var, seed), w, seed)
}

struct ContourPalette {
    field: [(f32, u32); 3],
    lines: [(f32, u32); 3],
    lines2: [(f32, u32); 2],
    line_a: f32,
    shadow: f32,
    lift: f32,
}

fn palette(v: &str) -> ContourPalette {
    match v {
        "teal" => ContourPalette {
            field: [(0.0, 0x0b3d42), (0.6, 0x12585a), (1.0, 0x1d6e6a)],
            lines: [(0.0, 0x5fa79a), (0.6, 0xa9d8c8), (1.0, 0xd8eee4)],
            lines2: [(0.0, 0xd9c48f), (1.0, 0xf3e3b8)],
            line_a: 0.45,
            shadow: 0.35,
            lift: 0.03,
        },
        "night" => ContourPalette {
            field: [(0.0, 0x0e0f16), (0.55, 0x171925), (1.0, 0x272b42)],
            lines: [(0.0, 0x3d59a1), (0.55, 0x7aa2f7), (1.0, 0x9ab8f9)],
            lines2: [(0.0, 0xbb9af7), (1.0, 0xff9e64)],
            line_a: 0.5,
            shadow: 0.55,
            lift: 0.025,
        },
        "paper" => ContourPalette {
            field: [(0.0, 0xe6dfd2), (0.6, 0xeee8dd), (1.0, 0xf6f2ea)],
            lines: [(0.0, 0x4d5b6a), (0.6, 0x55606b), (1.0, 0x6a6f74)],
            lines2: [(0.0, 0xc0603a), (1.0, 0xa8442a)],
            line_a: 0.55,
            shadow: 0.18,
            lift: 0.015,
        },
        _ => ContourPalette {
            field: [(0.0, 0x17122a), (0.6, 0x221a3a), (1.0, 0x2f2250)],
            lines: [(0.0, 0x5a4b96), (0.55, 0x8a6fc0), (1.0, 0xb08ad8)],
            lines2: [(0.0, 0xe07a8a), (1.0, 0xf6b878)],
            line_a: 0.5,
            shadow: 0.5,
            lift: 0.025,
        },
    }
}

fn draw(w: usize, h: usize, v: &str, seed: u32) -> Vec<C> {
    let pal = palette(v);
    let px = 1.0 / h as f32;
    let aspect = w as f32 / h as f32;
    let s = seed;
    let m = 0.15;
    let field = Field::build(
        (-aspect * 0.5 - m, -0.5 - m),
        (aspect * 0.5 + m, 0.5 + m),
        1.0 / 300.0,
        |x, y| {
            let wx = fbm(x * 0.9 + 3.1, y * 0.9 - 1.7, 3, s ^ 11);
            let wy = fbm(x * 0.9 - 5.3, y * 0.9 + 2.9, 3, s ^ 23);
            let (qx, qy) = (x + 0.4 * wx, y + 0.4 * wy);
            let t = fbm(qx * 1.25, qy * 1.25, 4, s ^ 37);
            let bump =
                |cx: f32, cy: f32, r: f32| (-((x - cx).powi(2) + (y - cy).powi(2)) / (r * r)).exp();
            0.5 + 0.95 * t + 0.3 * bump(0.45, -0.1, 0.35) - 0.15 * bump(-0.5, 0.3, 0.45)
        },
    );
    let field_r = Ramp::hex(&pal.field);
    let line_r = Ramp::hex(&pal.lines);
    let line2_r = Ramp::hex(&pal.lines2);
    // The fault: a steep line right of centre; the block beyond is slid
    // along it and its levels shifted by half a step.
    let f0 = (0.16, 0.0);
    let fa: f32 = 1.31;
    let fdir = (fa.cos(), fa.sin());
    let fnorm = (fdir.1, -fdir.0);
    let slide = 0.075;
    let inv = 1.0 / h as f32;
    let levels = 30.0;
    let uw = (h as f32 / 1080.0).max(1.0);
    let row = |j: usize| -> (Vec<f32>, Vec<f32>) {
        let y = (j as f32 + 0.5 - h as f32 * 0.5) * inv;
        let mut a = Vec::with_capacity(w + 1);
        let mut b = Vec::with_capacity(w + 1);
        for i in 0..=w {
            let x = (i as f32 + 0.5 - w as f32 * 0.5) * inv;
            a.push(field.at(x, y));
            b.push(field.at(x - slide * fdir.0, y - slide * fdir.1) + 0.5 / levels);
        }
        (a, b)
    };
    let shade = |hs: &[f32], hn: &[f32], i: usize, x: f32, y: f32, lift: f32, lr: &Ramp| -> C {
        let hv = hs[i];
        let gx = hs[i + 1] - hv;
        let gy = hn[i] - hv;
        let g = (gx * gx + gy * gy).sqrt().max(1e-7);
        let l = hv * levels;
        let fr = l - l.floor();
        let dl = fr.min(1.0 - fr) / (g * levels);
        let idx = l.round() as i32;
        let major = idx.rem_euclid(5) == 0;
        let tpos = clamp01(0.55 + 0.35 * (x / aspect) - 0.3 * y + lift);
        let base = field_r.at(clamp01(tpos + 0.22 * (hv - 0.5)));
        let lw = if major { 1.4 } else { 0.7 } * uw;
        let la = if major {
            (pal.line_a * 1.7).min(1.0)
        } else {
            pal.line_a
        };
        let cov = clamp01(lw * 0.5 + 0.5 - dl) * la;
        mix(base, lr.at(clamp01((hv - 0.1) * 1.1)), cov)
    };
    let mut img = vec![[0.0; 3]; w * h];
    par_rows(&mut img, w, |j, out_row| {
        let (cur, next) = (row(j), row(j + 1));
        let y = (j as f32 + 0.5 - h as f32 * 0.5) * inv;
        for (i, dst) in out_row.iter_mut().enumerate() {
            let x = (i as f32 + 0.5 - w as f32 * 0.5) * inv;
            let sd = (x - f0.0) * fnorm.0 + (y - f0.1) * fnorm.1;
            let k = clamp01(0.5 + sd / px);
            let mut out = [0.0; 3];
            if k < 1.0 {
                let mut c = shade(&cur.0, &next.0, i, x, y, 0.0, &line_r);
                let sh = pal.shadow * (-(-sd).max(0.0) / 0.018).exp();
                c = mul(c, 1.0 - sh);
                out = c;
            }
            if k > 0.0 {
                let mut c = shade(&cur.1, &next.1, i, x, y, pal.lift, &line2_r);
                let edge = 1.0 - clamp01((sd / px - 0.5) / 1.5);
                c = mix(c, [1.0; 3], edge * 0.12);
                out = mix(out, c, k);
            }
            *dst = out;
        }
    });
    img
}
