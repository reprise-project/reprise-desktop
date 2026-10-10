//! Caustic: sunlight focused through a slow swell onto a deep floor.

use std::f32::consts::TAU;

use super::noise::Rng;
use super::noise::light::{fast_sin, noise};
use super::tone::{C, add, lit, mix, mulc, oklch, quantise_shoulder, smooth};
use super::{HUES, SEED, Variant, centred, hue_of, par_rows};

const WAVES: usize = 14;
const STRENGTH: f32 = 3.0;
const TILT: f32 = 0.9;
const EPS: f32 = 0.035;
const GAIN: f32 = 0.16;
const SCALE: f32 = 2.0;
const POOL_R: f32 = 0.12;
const CUT: f32 = 0.8;
const DISPERSION: f32 = 0.03;
const BLOOM: f32 = 0.6;
const FOG: f32 = 0.7;

pub(super) fn render(w: usize, h: usize, v: Variant, seed: u32) -> Vec<u32> {
    let (hue, dark) = hue_of(v, &HUES);
    // The default seed draws the picture as designed; others reshuffle it.
    let k = seed ^ SEED;
    let mut rng = Rng(0xC0FFEE ^ k as u64);
    let mut kx = [0.0f32; 16];
    let mut ky = [0.0f32; 16];
    let mut ph = [0.0f32; 16];
    let mut cxx = [0.0f32; 16];
    let mut cxy = [0.0f32; 16];
    let mut cyy = [0.0f32; 16];
    for i in 0..WAVES {
        let ang = rng.r(0.0, TAU);
        let wl = 0.09 + 0.32 * (i as f32 / WAVES as f32).powf(1.3) + rng.r(0.0, 0.03);
        let (dx, dy, wk) = (ang.cos(), ang.sin(), TAU / wl);
        let c = rng.r(0.6, 1.0) / (WAVES as f32).sqrt();
        kx[i] = wk * dx;
        ky[i] = wk * dy;
        ph[i] = rng.r(0.0, TAU);
        cxx[i] = c * dx * dx;
        cxy[i] = c * dx * dy;
        cyy[i] = c * dy * dy;
    }

    let (floor_lo, floor_hi, light, deep) = if dark {
        (
            oklch(0.12, 0.05, lit(hue, -10.0)),
            oklch(0.24, 0.09, hue),
            oklch(0.80, 0.13, lit(hue, 30.0)),
            oklch(0.07, 0.03, lit(hue, -15.0)),
        )
    } else {
        (
            oklch(0.26, 0.11, lit(hue, -12.0)),
            oklch(0.46, 0.15, lit(hue, 5.0)),
            oklch(0.96, 0.07, lit(hue, 35.0)),
            oklch(0.17, 0.08, lit(hue, -18.0)),
        )
    };
    let haze = if dark {
        oklch(0.16, 0.06, hue)
    } else {
        oklch(0.34, 0.12, hue)
    };
    let glow = if dark {
        oklch(0.62, 0.16, lit(hue, 12.0))
    } else {
        oklch(0.75, 0.15, lit(hue, 15.0))
    };
    let mut px: Vec<C> = vec![[0.0; 3]; w * h];
    par_rows(&mut px, w, |y, row| {
        let vy = (y as f32 + 0.5) / h as f32;
        let z = 1.0 / (1.0 + TILT * (vy - 0.5));
        let focus = (vy - 0.45) * 1.8;
        let eps = EPS + 0.25 * focus * focus;
        for (x, out) in row.iter_mut().enumerate() {
            let (sx, sy) = centred(x, y, w, h);
            let mut qx = sx * z * SCALE;
            let mut qy = (vy - 0.5) * z * SCALE;
            qx += 0.22 * noise(qx * 0.7, qy * 0.7, 3 ^ k);
            qy += 0.22 * noise(qx * 0.7 + 5.2, qy * 0.7, 4 ^ k);
            let mut s = [0.0f32; 16];
            for i in 0..16 {
                s[i] = fast_sin(kx[i] * qx + ky[i] * qy + ph[i]);
            }
            let (mut axx, mut axy, mut ayy) = ([0.0f32; 4], [0.0f32; 4], [0.0f32; 4]);
            for c in 0..4 {
                for l in 0..4 {
                    let i = c * 4 + l;
                    axx[l] -= s[i] * cxx[i];
                    axy[l] -= s[i] * cxy[i];
                    ayy[l] -= s[i] * cyy[i];
                }
            }
            let hxx = (axx[0] + axx[1]) + (axx[2] + axx[3]);
            let hxy = (axy[0] + axy[1]) + (axy[2] + axy[3]);
            let hyy = (ayy[0] + ayy[1]) + (ayy[2] + ayy[3]);
            let tr = hxx + hyy;
            let dt = hxx * hyy - hxy * hxy;
            let mut lum = [0.0f32; 3];
            for (c, l) in lum.iter_mut().enumerate() {
                let s = STRENGTH * (1.0 + (c as f32 - 1.0) * DISPERSION);
                let d = 1.0 + s * tr + s * s * dt;
                *l = 1.0 / (d * d + eps * eps).sqrt();
            }
            let dg = 1.0 + STRENGTH * tr + STRENGTH * STRENGTH * dt;
            let bloom = 1.0 / (dg * dg + 0.16).sqrt();
            // The pool of light: an ellipse leaning from the upper right
            // into the middle, and the floor beneath.
            let (px_, py_) = (sx - 0.30, sy + 0.06);
            let (u, w2) = (px_ * 0.83 + py_ * 0.55, -px_ * 0.55 + py_ * 0.83);
            let re = ((u * u * 0.55 + w2 * w2 * 1.6) / POOL_R).sqrt();
            let pool = smooth(1.15, 0.25, re);
            let halo = (-(px_ * px_ + py_ * py_) / 0.8).exp();
            let floor = mix(deep, mix(floor_lo, floor_hi, pool), halo);
            let gain = GAIN * (0.006 + pool * pool + 0.04 * halo);
            let caus = lum.map(|l| (l - CUT).max(0.0) * gain);
            let mut col = add(floor, mulc(caus, light));
            let b = (bloom - 0.9).max(0.0) * (0.08 * halo + 0.9 * pool) * BLOOM;
            col = add(col, glow.map(|g| g * b));
            col = add(col, glow.map(|g| g * (0.08 * pool)));
            *out = mix(col, haze, FOG * smooth(0.55, 0.0, vy));
        }
    });
    quantise_shoulder(&px, w)
}
