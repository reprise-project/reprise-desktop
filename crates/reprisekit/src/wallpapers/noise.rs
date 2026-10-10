//! Seeded hashing, a small RNG, gradient noise and coarse precomputed fields
//! for the drawn wallpapers. Same seed, same numbers, on every machine:
//! nothing here reads the clock or the thread it runs on.
//!
//! Three noise families are kept apart because each design's picture is
//! pinned to its own: [`light`] (caustic, reeded, panes), [`land`] (ridges, tarn,
//! terraces) and [`form`] (fold, contour, flow).

use super::par_rows;

#[inline]
pub fn hash32(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h
}

#[inline]
pub fn hash_land(x: i32, y: i32, s: u32) -> u32 {
    hash32(
        (x as u32).wrapping_mul(0x9E37_79B1)
            ^ (y as u32).wrapping_mul(0x85EB_CA77).rotate_left(13)
            ^ s.wrapping_mul(0xC2B2_AE3D),
    )
}

#[inline]
pub fn hash_form(x: i32, y: i32, s: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ s.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    h
}

pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn f(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / 16_777_216.0
    }

    pub fn r(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }
}

#[inline]
fn catmull_rom(t: f32) -> [f32; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    ]
}

/// Noise for caustic, reeded and panes: 16 gradient directions, about -1..1,
/// with a libm-free floor and sine for the per-pixel inner loops.
pub mod light {
    use super::hash32;
    use std::f32::consts::TAU;

    /// floor without a libm call (baseline x86-64 has no roundss).
    #[inline]
    pub fn ffloor(x: f32) -> f32 {
        let i = x as i32 as f32;
        if x < i { i - 1.0 } else { i }
    }

    /// sin(x) to about 1e-3: a parabola refined once, after reducing to
    /// one turn.
    #[inline]
    pub fn fast_sin(x: f32) -> f32 {
        let t = x * (1.0 / TAU);
        let t = t - ffloor(t + 0.5);
        let y = 8.0 * t - 16.0 * t * t.abs();
        0.225 * (y * y.abs() - y) + y
    }

    pub fn hash2(x: i32, y: i32, seed: u32) -> u32 {
        hash32(
            (x as u32).wrapping_mul(0x8da6_b343)
                ^ (y as u32).wrapping_mul(0xd816_3841)
                ^ seed.wrapping_mul(0xcb1a_b31f),
        )
    }

    pub fn unit(h: u32) -> f32 {
        (h >> 8) as f32 * (1.0 / 16_777_216.0)
    }

    /// 16 unit vectors, k * 22.5 degrees, to five places: the pictures
    /// are pinned to these values.
    #[allow(clippy::approx_constant)]
    const GRADS: [(f32, f32); 16] = [
        (1.0, 0.0),
        (0.923_88, 0.382_68),
        (0.707_11, 0.707_11),
        (0.382_68, 0.923_88),
        (0.0, 1.0),
        (-0.382_68, 0.923_88),
        (-0.707_11, 0.707_11),
        (-0.923_88, 0.382_68),
        (-1.0, 0.0),
        (-0.923_88, -0.382_68),
        (-0.707_11, -0.707_11),
        (-0.382_68, -0.923_88),
        (0.0, -1.0),
        (0.382_68, -0.923_88),
        (0.707_11, -0.707_11),
        (0.923_88, -0.382_68),
    ];

    pub fn noise(x: f32, y: f32, seed: u32) -> f32 {
        let xf = ffloor(x);
        let yf = ffloor(y);
        let (ix, iy) = (xf as i32, yf as i32);
        let (fx, fy) = (x - xf, y - yf);
        let g = |i: i32, j: i32| {
            let (gx, gy) = GRADS[(hash2(ix + i, iy + j, seed) & 15) as usize];
            gx * (fx - i as f32) + gy * (fy - j as f32)
        };
        let u = fx * fx * fx * (fx * (fx * 6.0 - 15.0) + 10.0);
        let v = fy * fy * fy * (fy * (fy * 6.0 - 15.0) + 10.0);
        let a = g(0, 0) + (g(1, 0) - g(0, 0)) * u;
        let b = g(0, 1) + (g(1, 1) - g(0, 1)) * u;
        (a + (b - a) * v) * 1.4
    }
}

/// Noise for the landscapes: Perlin over 8 directions, fbm and ridged
/// multifractal, and a coarse field in device pixels.
pub mod land {
    use super::{catmull_rom, hash_land, par_rows};
    use crate::wallpapers::tone::{lerp, sat};

    #[inline]
    pub fn rnd(x: i32, y: i32, s: u32) -> f32 {
        (hash_land(x, y, s) >> 8) as f32 * (1.0 / 16_777_216.0)
    }

    #[inline]
    fn grad(h: u32, x: f32, y: f32) -> f32 {
        const D: f32 = std::f32::consts::FRAC_1_SQRT_2;
        match h & 7 {
            0 => x,
            1 => -x,
            2 => y,
            3 => -y,
            4 => (x + y) * D,
            5 => (x - y) * D,
            6 => (y - x) * D,
            _ => -(x + y) * D,
        }
    }

    #[inline]
    pub fn noise(x: f32, y: f32, s: u32) -> f32 {
        let (xi, yi) = (x.floor(), y.floor());
        let (xf, yf) = (x - xi, y - yi);
        let (ix, iy) = (xi as i32, yi as i32);
        let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
        let (u, v) = (fade(xf), fade(yf));
        let a = grad(hash_land(ix, iy, s), xf, yf);
        let b = grad(hash_land(ix + 1, iy, s), xf - 1.0, yf);
        let c = grad(hash_land(ix, iy + 1, s), xf, yf - 1.0);
        let d = grad(hash_land(ix + 1, iy + 1, s), xf - 1.0, yf - 1.0);
        1.414 * lerp(lerp(a, b, u), lerp(c, d, u), v)
    }

    /// Fractal sum of `oct` octaves, each rotated so no axis shows; about
    /// -1..1.
    pub fn fbm(x: f32, y: f32, oct: u32, s: u32) -> f32 {
        let (mut x, mut y, mut a, mut sum) = (x, y, 0.5, 0.0);
        for i in 0..oct {
            sum += a * noise(x, y, s.wrapping_add(i * 1013));
            let nx = 1.6 * x - 1.2 * y + 17.1;
            y = 1.2 * x + 1.6 * y + 3.7;
            x = nx;
            a *= 0.5;
        }
        sum
    }

    /// Ridged multifractal, 0..1 with sharp crests.
    pub fn ridged(x: f32, y: f32, oct: u32, s: u32) -> f32 {
        let (mut x, mut y, mut a, mut sum, mut w, mut norm) = (x, y, 0.5, 0.0, 1.0f32, 0.0);
        for i in 0..oct {
            let n = 1.0 - noise(x, y, s.wrapping_add(i * 7919)).abs();
            let n = n * n * w;
            w = sat(n * 1.6);
            sum += a * n;
            norm += a;
            let nx = 1.6 * x - 1.2 * y + 5.3;
            y = 1.2 * x + 1.6 * y + 9.1;
            x = nx;
            a *= 0.5;
        }
        sum / norm
    }

    /// A smooth scalar field evaluated on a coarse grid of `step` device
    /// pixels and read back with Catmull-Rom: soft things (clouds, terrain
    /// heights) cost a fraction of a per-pixel evaluation.
    pub struct Field {
        gw: usize,
        gh: usize,
        step: f32,
        d: Vec<f32>,
    }

    impl Field {
        pub fn new(w: usize, h: usize, step: f32, f: impl Fn(f32, f32) -> f32 + Sync) -> Field {
            let gw = (w as f32 / step).ceil() as usize + 4;
            let gh = (h as f32 / step).ceil() as usize + 4;
            let mut d = vec![0.0; gw * gh];
            par_rows(&mut d, gw, |j, row| {
                for (i, o) in row.iter_mut().enumerate() {
                    *o = f((i as f32 - 1.0) * step, (j as f32 - 1.0) * step);
                }
            });
            Field { gw, gh, step, d }
        }

        #[inline]
        pub fn at(&self, x: f32, y: f32) -> f32 {
            let fx = x / self.step + 1.0;
            let fy = y / self.step + 1.0;
            let ix = (fx.floor() as isize).clamp(1, self.gw as isize - 3) as usize;
            let iy = (fy.floor() as isize).clamp(1, self.gh as isize - 3) as usize;
            let wx = catmull_rom(fx - ix as f32);
            let wy = catmull_rom(fy - iy as f32);
            let mut s = 0.0;
            for (j, wyj) in wy.iter().enumerate() {
                let row = &self.d[(iy - 1 + j) * self.gw + ix - 1..];
                s += wyj * (row[0] * wx[0] + row[1] * wx[1] + row[2] * wx[2] + row[3] * wx[3]);
            }
            s
        }
    }
}

/// Noise for fold, contour and flow: Perlin over 16 directions (about
/// -0.7..0.7), normalised fbm, and a coarse field in picture units.
pub mod form {
    use super::{hash_form, par_rows};

    /// cos and sin of k * 22.5 degrees, as written: the pictures are
    /// pinned to these values.
    #[allow(clippy::approx_constant)]
    const GRADS: [(f32, f32); 16] = {
        let mut g = [(0.0, 0.0); 16];
        let c = [
            1.0,
            0.923_879_5,
            0.707_106_8,
            0.382_683_4,
            0.0,
            -0.382_683_4,
            -0.707_106_8,
            -0.923_879_5,
            -1.0,
            -0.923_879_5,
            -0.707_106_8,
            -0.382_683_4,
            0.0,
            0.382_683_4,
            0.707_106_8,
            0.923_879_5,
        ];
        let mut i = 0;
        while i < 16 {
            g[i] = (c[i], c[(i + 12) % 16]);
            i += 1;
        }
        g
    };

    pub fn perlin(x: f32, y: f32, s: u32) -> f32 {
        let (fx0, fy0) = (x.floor(), y.floor());
        let (ix, iy) = (fx0 as i32, fy0 as i32);
        let (fx, fy) = (x - fx0, y - fy0);
        let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
        let (u, v) = (fade(fx), fade(fy));
        let g = |dx: i32, dy: i32| {
            let (gx, gy) = GRADS[(hash_form(ix + dx, iy + dy, s) & 15) as usize];
            gx * (fx - dx as f32) + gy * (fy - dy as f32)
        };
        let a = g(0, 0) + (g(1, 0) - g(0, 0)) * u;
        let b = g(0, 1) + (g(1, 1) - g(0, 1)) * u;
        a + (b - a) * v
    }

    /// Normalised fractal sum of `oct` octaves, rotated between octaves so
    /// the lattice never lines up.
    pub fn fbm(x: f32, y: f32, oct: u32, s: u32) -> f32 {
        let (mut x, mut y) = (x, y);
        let (mut sum, mut amp, mut norm) = (0.0, 1.0, 0.0);
        for i in 0..oct {
            sum += amp * perlin(x, y, s.wrapping_add(i * 101));
            norm += amp;
            amp *= 0.5;
            let (nx, ny) = (0.8 * x - 0.6 * y, 0.6 * x + 0.8 * y);
            x = nx * 2.03 + 17.1;
            y = ny * 2.03 - 9.7;
        }
        sum / norm
    }

    /// A scalar field evaluated on a grid over centre-relative picture
    /// units, read back with Catmull-Rom so it stays smooth at any output
    /// size.
    pub struct Field {
        nx: usize,
        ny: usize,
        x0: f32,
        y0: f32,
        step: f32,
        v: Vec<f32>,
    }

    impl Field {
        pub fn build(
            (x0, y0): (f32, f32),
            (x1, y1): (f32, f32),
            step: f32,
            f: impl Fn(f32, f32) -> f32 + Sync,
        ) -> Field {
            let nx = ((x1 - x0) / step).ceil() as usize + 4;
            let ny = ((y1 - y0) / step).ceil() as usize + 4;
            let (x0, y0) = (x0 - step, y0 - step);
            let mut v = vec![0.0; nx * ny];
            par_rows(&mut v, nx, |j, row| {
                for (i, o) in row.iter_mut().enumerate() {
                    *o = f(x0 + i as f32 * step, y0 + j as f32 * step);
                }
            });
            Field {
                nx,
                ny,
                x0,
                y0,
                step,
                v,
            }
        }

        pub fn at(&self, x: f32, y: f32) -> f32 {
            let gx = ((x - self.x0) / self.step).clamp(1.0, self.nx as f32 - 3.001);
            let gy = ((y - self.y0) / self.step).clamp(1.0, self.ny as f32 - 3.001);
            let (ix, iy) = (gx as usize, gy as usize);
            let (tx, ty) = (gx - ix as f32, gy - iy as f32);
            let cr = |p0: f32, p1: f32, p2: f32, p3: f32, t: f32| {
                p1 + 0.5
                    * t
                    * (p2 - p0
                        + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3
                            + t * (3.0 * (p1 - p2) + p3 - p0)))
            };
            let row = |j: usize| {
                let b = j * self.nx + ix - 1;
                cr(self.v[b], self.v[b + 1], self.v[b + 2], self.v[b + 3], tx)
            };
            cr(row(iy - 1), row(iy), row(iy + 1), row(iy + 2), ty)
        }
    }
}
