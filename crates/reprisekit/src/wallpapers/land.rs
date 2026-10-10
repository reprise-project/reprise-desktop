//! What ridges and tarn share: the float picture they paint, the sky
//! (gradient, sun or moon with glow and rays, streaked cloud, stars and a
//! milky band) and silhouettes (conifers, supersampled ridge lines).

use super::noise::land::{Field, fbm, ridged, rnd};
use super::par_rows;
use super::tone::{C, add, hex, mix, mul, sat, screen, smooth};

/// An image under construction: linear-ish float RGB, row-major.
pub(super) struct Img {
    pub w: usize,
    pub h: usize,
    pub px: Vec<C>,
}
impl Img {
    pub fn new(w: usize, h: usize) -> Img {
        Img {
            w,
            h,
            px: vec![[0.0; 3]; w * h],
        }
    }
    /// Design coordinates of a device pixel's centre: `u` across from the
    /// middle, `v` down from the top, both in picture heights.
    #[inline]
    pub fn uv(&self, x: usize, y: usize) -> (f32, f32) {
        let hf = self.h as f32;
        (
            (x as f32 + 0.5 - self.w as f32 * 0.5) / hf,
            (y as f32 + 0.5) / hf,
        )
    }
    /// Bilinear read with clamped edges.
    #[inline]
    pub fn sample(&self, x: f32, y: f32) -> C {
        let x = x.clamp(0.0, self.w as f32 - 1.001);
        let y = y.clamp(0.0, self.h as f32 - 1.001);
        let (ix, iy) = (x as usize, y as usize);
        let (tx, ty) = (x - ix as f32, y - iy as f32);
        let i = iy * self.w + ix;
        let a = mix(self.px[i], self.px[i + 1], tx);
        let b = mix(self.px[i + self.w], self.px[i + self.w + 1], tx);
        mix(a, b, ty)
    }
}

#[derive(Clone)]
pub(super) struct Sky {
    pub(super) zen: C,
    pub(super) mid: C,
    pub(super) hor: C,
    /// Where the horizon colour is reached, in picture heights.
    pub(super) hor_v: f32,
    pub(super) sun: [f32; 2],
    pub(super) sun_r: f32,
    pub(super) sun_core: C,
    pub(super) glow: C,
    pub(super) glow_k: f32,
    pub(super) halo: C,
    pub(super) halo_k: f32,
    /// Light scattered in front of the land near the sun.
    pub(super) bloom: f32,
    /// Strength of the shafts fanning up from the sun.
    pub(super) rays: f32,
    pub(super) cloud: f32,
    pub(super) cloud_lit: C,
    pub(super) cloud_dark: C,
    /// Height band the cloud streaks live in.
    pub(super) cloud_v: [f32; 2],
    pub(super) stars: f32,
    pub(super) milky: f32,
    pub(super) seed: u32,
}

/// Coarse fields the sky reads: cloud density, the milky band, the rays.
pub(super) struct SkyFields {
    pub(super) cloud: Field,
    pub(super) milky: Option<Field>,
    pub(super) rays: Option<Field>,
}

impl Sky {
    pub(super) fn fields(&self, img: &Img) -> SkyFields {
        let (w, h) = (img.w, img.h);
        let hf = h as f32;
        let step = (hf / 270.0).max(2.0);
        let s = self.seed;
        let cv = self.cloud_v;
        let cloud = Field::new(w, (hf * (cv[1] + 0.05)) as usize + 4, step, |x, y| {
            let (u, v) = ((x - w as f32 * 0.5) / hf, y / hf);
            let band =
                smooth(cv[0] - 0.05, cv[0] + 0.05, v) * (1.0 - smooth(cv[1] - 0.08, cv[1], v));
            if band <= 0.0 {
                return 0.0;
            }
            let wx = fbm(u * 0.9, v * 4.0, 3, s + 31) * 0.6;
            let n = fbm(u * 1.4 + wx, v * 11.0 + wx * 2.0, 5, s + 32);
            let streak = fbm(u * 0.5, v * 2.0, 2, s + 33) * 0.5 + 0.5;
            band * smooth(0.02, 0.45, n + streak * 0.25 - 0.1)
        });
        let milky = (self.milky > 0.0).then(|| {
            Field::new(w, h, step, |x, y| {
                let (u, v) = ((x - w as f32 * 0.5) / hf, y / hf);
                // A band tilted across the sky, dusty, split by a dark lane.
                let d = (u * 0.42 + v * 0.91 - 0.05 - 0.1 * (u * 1.3).sin()) / 0.12;
                let core = (-d * d).exp();
                let dust = fbm(u * 3.5 + 2.0, v * 3.5, 5, s + 41) * 0.5 + 0.5;
                let lane = ridged(u * 2.6 + 3.0, v * 2.6, 4, s + 42);
                let lane_d = (-(d + 0.1) * (d + 0.1) * 5.0).exp();
                (core * (0.25 + 1.1 * dust * dust) * (1.0 - 0.8 * lane * lane_d)).max(0.0)
            })
        });
        let sun = self.sun;
        let rays = (self.rays > 0.0).then(|| {
            Field::new(w, h, step, |x, y| {
                let (u, v) = ((x - w as f32 * 0.5) / hf, y / hf);
                let a = (v - sun[1]).atan2(u - sun[0]);
                let r = fbm(a * 6.0, 0.5, 3, s + 51) * 0.5 + 0.5;
                smooth(0.45, 0.75, r)
            })
        });
        SkyFields { cloud, milky, rays }
    }

    /// The sky's colour without clouds or stars: gradient and glow.
    #[inline]
    pub(super) fn base(&self, u: f32, v: f32) -> C {
        let t = sat(v / self.hor_v);
        let mut c = if t < 0.55 {
            mix(self.zen, self.mid, smooth(0.0, 1.0, t / 0.55))
        } else {
            mix(self.mid, self.hor, smooth(0.0, 1.0, (t - 0.55) / 0.45))
        };
        let du = u - self.sun[0];
        let dv = v - self.sun[1];
        // Glow spreads along the horizon more than up.
        let d = (du * du * 0.55 + dv * dv * 1.4).sqrt();
        let g = (-d * self.glow_k).exp();
        let hl = (-d * self.halo_k).exp();
        c = screen(c, mul(self.halo, hl));
        c = screen(c, mul(self.glow, g));
        c
    }

    /// Glow laid over land and water near the sun: the air between.
    #[inline]
    pub(super) fn bloom_at(&self, c: C, u: f32, v: f32) -> C {
        if self.bloom <= 0.0 {
            return c;
        }
        let du = u - self.sun[0];
        let dv = v - self.sun[1];
        let d = (du * du * 0.5 + dv * dv * 2.0).sqrt();
        screen(c, mul(self.glow, (-d * 6.0).exp() * self.bloom))
    }

    #[inline]
    pub(super) fn at(&self, f: &SkyFields, x: f32, y: f32, u: f32, v: f32, px: f32) -> C {
        let mut c = self.base(u, v);
        let du = u - self.sun[0];
        let dv = v - self.sun[1];
        let ds = (du * du + dv * dv).sqrt();
        if let Some(r) = &f.rays {
            let k = r.at(x, y) * (-ds * 2.2).exp() * smooth(0.0, 0.08, -dv) * self.rays;
            c = screen(c, mul(self.glow, k * 0.3));
        }
        if let Some(m) = &f.milky {
            let mv = m.at(x, y);
            let tint = mix(hex(0x9AAAD8), hex(0xE8D8C8), sat(mv - 0.4));
            c = screen(c, mul(tint, mv * self.milky * 0.3));
            c = add(
                c,
                mul(
                    self.stars_at(u, v, px, 0.0045, 0.45 * mv, 7),
                    self.milky * 0.8,
                ),
            );
        }
        if self.stars > 0.0 {
            c = add(c, mul(self.stars_at(u, v, px, 0.018, 0.3, 3), self.stars));
        }
        let cov = sat((self.sun_r - ds) / px + 0.5);
        if cov > 0.0 {
            c = mix(c, self.sun_core, cov);
        }
        if self.cloud > 0.0 && v < self.cloud_v[1] + 0.05 {
            let cd = f.cloud.at(x, y) * self.cloud;
            if cd > 0.001 {
                let near = (-ds * 3.0).exp();
                let lit = mix(self.cloud_dark, self.cloud_lit, sat(near * 1.4 + 0.1));
                c = mix(c, lit, sat(cd));
            }
        }
        c
    }

    /// Point stars on a jittered grid of `cell`; `dens` is the chance a
    /// cell has one.
    #[inline]
    pub(super) fn stars_at(&self, u: f32, v: f32, px: f32, cell: f32, dens: f32, key: u32) -> C {
        let (cx, cy) = ((u / cell).floor() as i32, (v / cell).floor() as i32);
        let s = self.seed ^ key.wrapping_mul(0x51ED);
        if rnd(cx, cy, s) > dens {
            return [0.0; 3];
        }
        let sx = (cx as f32 + 0.2 + 0.6 * rnd(cx, cy, s + 1)) * cell;
        let sy = (cy as f32 + 0.2 + 0.6 * rnd(cx, cy, s + 2)) * cell;
        let b = rnd(cx, cy, s + 3).powf(4.0);
        let r = (0.0004 + 0.0009 * b).max(px * 0.55);
        let d = ((u - sx) * (u - sx) + (v - sy) * (v - sy)).sqrt();
        let core = sat((r - d) / px + 0.5);
        let glow = (-d / (r * 3.0)).exp() * b * 0.3;
        let k = (core * (0.18 + 0.82 * b) + glow).min(1.0);
        let tint = rnd(cx, cy, s + 4);
        mul(mix(hex(0xFFE4C0), hex(0xC8DCFF), tint), k)
    }
}

/// Conifer spires along a line: the height (picture heights) of the
/// tallest tree over `u`. `dens` thins them into clearings.
pub(super) fn trees(u: f32, spacing: f32, height: f32, s: u32, dens: impl Fn(f32) -> f32) -> f32 {
    let k = (u / spacing).floor() as i32;
    let reach = (height * 0.3 / spacing).ceil() as i32 + 1;
    let mut best = 0.0f32;
    for j in k - reach..=k + reach {
        let r1 = rnd(j, 1, s);
        let r3 = rnd(j, 3, s);
        let cx = (j as f32 + r1) * spacing;
        let dn = dens(cx);
        if dn < r3 {
            continue;
        }
        let r2 = rnd(j, 2, s);
        // Trees in thick stands grow taller.
        let th = height * (0.4 + 0.45 * r2 + 0.15 * dn);
        best = best.max(spruce(u - cx, th));
    }
    best
}

/// Height of a spruce of height `th` at `dx` from its trunk: five stacked
/// tiers, each a triangle wider than the one above.
#[inline]
pub(super) fn spruce(dx: f32, th: f32) -> f32 {
    let dx = dx.abs();
    let hw = th * 0.24;
    if dx >= hw {
        return 0.0;
    }
    const N: f32 = 5.0;
    let mut best = 0.0f32;
    for j in 0..5 {
        let jf = j as f32;
        let top = th * (1.0 - jf / N * 0.92);
        let bot = th * (1.0 - (jf + 1.6) / N).max(0.0);
        let wj = hw * (0.3 + 0.7 * (jf + 1.0) / N);
        if dx < wj {
            best = best.max(top - (top - bot) * dx / wj);
        }
    }
    best
}

/// Coverage of a pixel whose centre is `e` picture heights below a
/// silhouette edge of slope `s` (dv/du), anti-aliased over one pixel.
#[inline]
pub(super) fn edge_cov(e: f32, s: f32, px: f32) -> f32 {
    sat(e / (px * (1.0 + s * s).sqrt()) + 0.5)
}

/// Supersampled silhouette edges: `SS` tops per device column.
pub(super) const SS: usize = 3;

pub(super) fn tops_for(w: usize, h: usize, f: impl Fn(f32) -> f32 + Sync) -> Vec<f32> {
    let hf = h as f32;
    let mut tops = vec![0.0; w * SS];
    par_rows(&mut tops, 1, |i, t| {
        let x = (i as f32 + 0.5) / SS as f32;
        t[0] = f((x - w as f32 * 0.5) / hf);
    });
    tops
}

/// Coverage of pixel (x, v) by a silhouette whose sub-column tops are in
/// `tops` (picture heights), with slope-corrected anti-aliasing.
#[inline]
pub(super) fn sil_cov(tops: &[f32], x: usize, v: f32, px: f32) -> f32 {
    let n = tops.len();
    let mut c = 0.0;
    for k in 0..SS {
        let i = x * SS + k;
        let a = tops[i.saturating_sub(1)];
        let b = tops[(i + 1).min(n - 1)];
        let slope = (b - a) / (2.0 * px / SS as f32);
        c += edge_cov(v - tops[i], slope.clamp(-8.0, 8.0), px);
    }
    c / SS as f32
}

/// The highest (smallest) top over a device column.
pub(super) fn col_min(tops: &[f32], x: usize) -> f32 {
    tops[x * SS..x * SS + SS]
        .iter()
        .copied()
        .fold(f32::MAX, f32::min)
}
