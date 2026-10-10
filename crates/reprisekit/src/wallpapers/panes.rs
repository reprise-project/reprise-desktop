//! Panes: thick glass slabs on a polished floor, traced analytically in linear
//! light: refraction in, tint by path length, Fresnel split at each face, with
//! dispersion on the eased edges. Three passes: base (one ray per pixel, with a
//! surface signature), flag (pixels differing from a neighbour), and a
//! multi-jittered supersample of flagged pixels. The flag pass reads only the
//! base pass, so the picture is the same at any thread count.

use super::noise::Rng;
use super::noise::hash32;
use super::noise::light::{hash2, unit};
use super::tone::{C, add, lit, mix, mul, mulc, oklch, quantise_shoulder, smooth};
use super::{HUES, SEED, Variant, centred, hue_of, par_rows};

const FOCAL: f32 = 1.7;
const Y0: f32 = -0.08;
const EYE: f32 = 0.9;
const EASE: f32 = 0.05;
const REF: f32 = 0.5;
const IOR: f32 = 1.5;
const DISP: f32 = 0.02;
const HAZE: f32 = 1.4;
const RIM: f32 = 0.75;
const SOFT: (f32, f32) = (0.5, 1.0);
const FOLLOW: f32 = 0.08;
const GLOSS: f32 = 0.45;
const FOCUS: f32 = 0.6;
const SPEC: f32 = 1.5;
const EDGE: f32 = 0.5;
const FOG: f32 = 14.0;
const AMBIENT: f32 = 0.45;
const FADE: f32 = 0.8;
const SKY_H: f32 = 0.3;
const LOBE: (f32, f32, f32) = (0.55, 0.22, 0.0);
const SUN_K: f32 = 0.7;
const SPREAD: f32 = 1.5;
const JUMP: f32 = 0.035;
const CUT_SIDE: usize = 4;
const CUT_SIDE_4K: usize = 3;
const STEEP_SIDE: usize = 2;
const LIGHT_MAX: f32 = 4.0;

type V3 = [f32; 3];

fn dot3(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm3(a: V3) -> V3 {
    let l = dot3(a, a).sqrt().max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}

fn reflect(d: V3, n: V3) -> V3 {
    let k = 2.0 * dot3(d, n);
    [d[0] - k * n[0], d[1] - k * n[1], d[2] - k * n[2]]
}

fn refract(d: V3, n: V3, eta: f32) -> Option<V3> {
    let ci = -dot3(n, d);
    let k = 1.0 - eta * eta * (1.0 - ci * ci);
    if k < 0.0 {
        return None;
    }
    let m = eta * ci - k.sqrt();
    Some([
        eta * d[0] + m * n[0],
        eta * d[1] + m * n[1],
        eta * d[2] + m * n[2],
    ])
}

fn box_hit(o: V3, d: V3, e: V3) -> (f32, f32, usize) {
    let (mut tn, mut tf, mut ax) = (f32::MIN, f32::MAX, 0);
    for k in 0..3 {
        let inv = 1.0
            / if d[k].abs() < 1e-9 {
                1e-9f32.copysign(d[k])
            } else {
                d[k]
            };
        let (t1, t2) = ((-e[k] - o[k]) * inv, (e[k] - o[k]) * inv);
        let (a, b) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
        if a > tn {
            tn = a;
            ax = k;
        }
        tf = tf.min(b);
    }
    (tn, tf, ax)
}

struct Slab {
    c: V3,
    ax: [V3; 3],
    e: V3,
    ease: f32,
    absorb: C,
    r: f32,
}

impl Slab {
    fn misses(&self, o: V3, d: V3, margin: f32) -> bool {
        let oc = [self.c[0] - o[0], self.c[1] - o[1], self.c[2] - o[2]];
        let tca = dot3(oc, d);
        let r = self.r + margin;
        tca < -r || dot3(oc, oc) - tca * tca > r * r
    }

    fn local(&self, p: V3) -> V3 {
        let q = [p[0] - self.c[0], p[1] - self.c[1], p[2] - self.c[2]];
        [
            dot3(q, self.ax[0]),
            dot3(q, self.ax[1]),
            dot3(q, self.ax[2]),
        ]
    }

    fn local_dir(&self, d: V3) -> V3 {
        [
            dot3(d, self.ax[0]),
            dot3(d, self.ax[1]),
            dot3(d, self.ax[2]),
        ]
    }

    fn world(&self, p: V3) -> V3 {
        let a = &self.ax;
        [
            self.c[0] + p[0] * a[0][0] + p[1] * a[1][0] + p[2] * a[2][0],
            self.c[1] + p[0] * a[0][1] + p[1] * a[1][1] + p[2] * a[2][1],
            self.c[2] + p[0] * a[0][2] + p[1] * a[1][2] + p[2] * a[2][2],
        ]
    }

    fn world_dir(&self, d: V3) -> V3 {
        let a = &self.ax;
        [
            d[0] * a[0][0] + d[1] * a[1][0] + d[2] * a[2][0],
            d[0] * a[0][1] + d[1] * a[1][1] + d[2] * a[2][1],
            d[0] * a[0][2] + d[1] * a[1][2] + d[2] * a[2][2],
        ]
    }

    fn ease_depth(&self, p: V3, k: usize, ease: f32) -> f32 {
        let mut wmax = 0.0f32;
        for (i, (e, q)) in self.e.iter().zip(p).enumerate() {
            if i != k {
                let w = (1.0 - (e - q.abs()) / (self.ease * ease)).max(0.0);
                wmax = wmax.max(w * w);
            }
        }
        wmax
    }

    fn normal(&self, p: V3, k: usize, ease: f32) -> (V3, f32) {
        let mut n = [0.0; 3];
        n[k] = p[k].signum();
        let mut wmax = 0.0f32;
        for i in 0..3 {
            if i == k {
                continue;
            }
            let di = self.e[i] - p[i].abs();
            let w = (1.0 - di / (self.ease * ease)).max(0.0);
            let w = w * w;
            n[i] = p[i].signum() * w;
            wmax = wmax.max(w);
        }
        (norm3(n), wmax)
    }
}

struct Scene {
    slabs: Vec<Slab>,
    sun: V3,
    sky_hz: C,
    sky_top: C,
    floor_near: C,
    floor_far: C,
    glow: C,
    edge: C,
    haze: C,
    rim: C,
    lobe: (f32, f32, f32),
}

impl Scene {
    fn ray(&self, x: f32, y: f32) -> V3 {
        norm3([x / FOCAL, (y - Y0) / FOCAL, 1.0])
    }

    fn sky(&self, d: V3) -> C {
        let el = -d[1];
        let base = mix(self.sky_hz, self.sky_top, smooth(-0.02, SKY_H, el));
        let s = dot3(d, self.sun).max(0.0);
        let s2 = s * s;
        let s8 = s2 * s2 * s2 * s2;
        let lobe = self.lobe.0 * s8 * s8 * s8 + self.lobe.1 * s8 + self.lobe.2 * s2;
        let band = 0.18 * (-(el * el) / 0.004).exp();
        add(base, mul(self.glow, lobe + band))
    }

    fn nearest(&self, o: V3, d: V3, skip: usize) -> Option<(usize, f32, usize)> {
        let mut best: Option<(usize, f32, usize)> = None;
        for (i, s) in self.slabs.iter().enumerate() {
            if i == skip || s.misses(o, d, 0.0) {
                continue;
            }
            let (tn, tf, ax) = box_hit(s.local(o), s.local_dir(d), s.e);
            if tn < tf && tn > 1e-4 && best.is_none_or(|b| tn < b.1) {
                best = Some((i, tn, ax));
            }
        }
        best
    }

    fn trace(&self, o: V3, d: V3, skip: usize, depth: u32, pc: &mut Path) -> C {
        match self.nearest(o, d, skip) {
            Some((i, t, ax)) => self.glass(i, o, d, t, ax, depth, pc),
            None => self.backdrop(o, d, depth, pc),
        }
    }

    fn backdrop(&self, o: V3, d: V3, depth: u32, pc: &mut Path) -> C {
        if d[1] <= 1e-5 {
            return self.sky(d);
        }
        let t = (EYE - o[1]) / d[1];
        let q = [o[0] + t * d[0], EYE, o[2] + t * d[2]];
        let dist = q[2].max(0.0);
        let near = smooth(14.0, 1.0, dist);
        let base = mix(self.floor_far, self.floor_near, near);
        let (pass, caustic) = self.light_at(q);
        let mut col = mulc(base, pass.map(|p| AMBIENT + SUN_K * p));
        col = add(col, caustic);
        let g = 1.0 - d[1];
        let g2 = g * g;
        let fres = 0.04 + 0.96 * g2 * g2 * g;
        let rd = [d[0], -d[1], d[2]];
        let refl = match self.nearest(q, rd, usize::MAX) {
            Some((i, t, ax)) if depth == 0 => {
                let k = (-t / FADE).exp();
                let mut sub = pc.child(k * (fres * GLOSS).min(1.0));
                let c = mix(
                    self.sky(rd),
                    self.glass(i, q, rd, t, ax, depth + 1, &mut sub),
                    k,
                );
                pc.fold(sub);
                c
            }
            _ => self.sky(rd),
        };
        col = mix(col, refl, (fres * GLOSS).min(1.0));
        let fog = 1.0 - (-dist / FOG).exp();
        mix(col, self.sky([d[0], -0.0, d[2]]), fog * fog)
    }

    fn light_at(&self, q: V3) -> (C, C) {
        let mut k = [1.0f32; 3];
        let mut extra = [0.0f32; 3];
        for s in &self.slabs {
            let reach = 0.004
                + 0.035
                    * (dot3([s.c[0] - q[0], s.c[1] - q[1], s.c[2] - q[2]], self.sun) + s.r)
                        .max(0.0);
            if s.misses(q, self.sun, 2.0 * reach) {
                continue;
            }
            let (o, d) = (s.local(q), s.local_dir(self.sun));
            let (tn, tf, ax) = box_hit(o, d, s.e);
            if tf <= 0.0 {
                continue;
            }
            let m = tf - tn;
            let blur = 0.004 + 0.035 * tn.max(0.0);
            let cover = smooth(-blur, blur, m);
            if cover <= 0.0 {
                continue;
            }
            let path = m.max(0.0);
            let p = [o[0] + tn * d[0], o[1] + tn * d[1], o[2] + tn * d[2]];
            for ch in 0..3 {
                let w = s.ease_depth(
                    p,
                    ax,
                    (1.0 + (ch as f32 - 1.0) * 0.25) * (1.0 + SPREAD * tn.max(0.0)),
                );
                let pass = 0.82 * (s.absorb[ch] * path).exp() * (1.0 - 0.7 * w);
                k[ch] *= 1.0 + (pass - 1.0) * cover;
                let e = (w - 0.45) / 0.22;
                extra[ch] +=
                    FOCUS * self.glow[ch] * (-e * e).exp() * cover / (1.0 + 0.5 * tn.max(0.0));
            }
        }
        (k, extra)
    }

    #[allow(clippy::too_many_arguments)]
    fn glass(&self, i: usize, o: V3, d: V3, t: f32, ax: usize, depth: u32, pc: &mut Path) -> C {
        let s = &self.slabs[i];
        pc.mark(0x100 + i as u32 * 8 + ax as u32);
        let (ol, dl) = (s.local(o), s.local_dir(d));
        let p = [ol[0] + t * dl[0], ol[1] + t * dl[1], ol[2] + t * dl[2]];
        let pw = s.world(p);
        let (n, w_in) = s.normal(p, ax, 1.0);
        let ci = -dot3(n, dl);
        let g = 1.0 - ci.clamp(0.0, 1.0);
        let g2 = g * g;
        let fres = 0.04 + 0.96 * g2 * g2 * g;
        let rw = s.world_dir(reflect(dl, n));
        let mut sub = pc.child(fres);
        let refl = if depth < 2 {
            self.trace(pw, rw, i, depth + 1, &mut sub)
        } else {
            self.sky(rw)
        };
        pc.fold(sub);
        let sp = dot3(rw, self.sun).max(0.0);
        let sp4 = sp * sp * sp * sp;
        let spec = sp4 * sp4 * sp4 * sp4 * SPEC;
        let entry = |ior: f32, sub: &mut Path, bevel: &mut bool| -> C {
            *bevel = w_in > 0.01;
            match refract(dl, n, 1.0 / (IOR + (ior - IOR) * w_in)) {
                Some(r) => self.inside(i, p, r, ior, BOUNCES, depth, sub, bevel),
                None => self.haze,
            }
        };
        let ior = IOR * (1.0 - DISP * pc.lambda[1]);
        let soft = soften(w_in);
        let mut sub = pc.child((1.0 - fres) * (1.0 - soft));
        let mut bevel = false;
        let mut col = entry(ior, &mut sub, &mut bevel);
        if bevel && sub.w > 0.15 {
            for ch in [0, 2] {
                let mut sub_c = pc.child((1.0 - fres) * (1.0 - soft));
                col[ch] = entry(IOR * (1.0 - DISP * pc.lambda[ch]), &mut sub_c, &mut false)[ch];
                sub.split |= sub_c.sig != sub.sig || sub_c.split;
            }
        }
        pc.fold(sub);
        let col = mix(col, self.rim, soft);
        let mut c = mix(col, refl, fres);
        c = add(c, mul(self.edge, w_in * EDGE));
        add(c, mul(self.glow, spec))
    }

    /// Light carried back along a ray inside slab `i` from local point `q` in
    /// direction `r`; the light splits by Fresnel at each face and the leaving
    /// share falls smoothly to nothing near the critical angle. `ior` applies in
    /// full only inside the eased edges and blends to the mean on the flat faces,
    /// so colours split on the bevels. `bevel` is set once the path meets one.
    #[allow(clippy::too_many_arguments)]
    fn inside(
        &self,
        i: usize,
        q: V3,
        r: V3,
        ior: f32,
        bounces: usize,
        depth: u32,
        pc: &mut Path,
        bevel: &mut bool,
    ) -> C {
        let s = &self.slabs[i];
        let (mut tx, mut k) = (f32::MAX, 0);
        for a in 0..3 {
            let ta = (s.e[a].copysign(r[a]) - q[a]) / if r[a].abs() < 1e-9 { 1e-9 } else { r[a] };
            if ta < tx {
                tx = ta;
                k = a;
            }
        }
        let q = [q[0] + tx * r[0], q[1] + tx * r[1], q[2] + tx * r[2]];
        pc.mark(2 + k as u32 * 2 + (q[k] > 0.0) as u32);
        let (no, w_out) = s.normal(q, k, 1.0);
        *bevel |= w_out > 0.01;
        let fade = s.absorb.map(|a| (a * tx).exp());
        let onward = |pc: &mut Path, bevel: &mut bool| -> C {
            if bounces > 1 {
                let keep = KEEP[BOUNCES - bounces];
                let mut sub = pc.child(keep);
                let c = self.inside(
                    i,
                    q,
                    reflect(r, no),
                    ior,
                    bounces - 1,
                    depth,
                    &mut sub,
                    bevel,
                );
                pc.fold(sub);
                mix(self.haze, c, keep)
            } else {
                self.haze
            }
        };
        let col = match refract(r, [-no[0], -no[1], -no[2]], IOR + (ior - IOR) * w_out) {
            Some(e) => {
                let cos_t = dot3(e, no).clamp(0.0, 1.0);
                let g = 1.0 - cos_t;
                let g2 = g * g;
                let f = 0.04 + 0.96 * g2 * g2 * g;
                let (qw, ew) = (s.world(q), s.world_dir(e));
                let soft = soften(w_out);
                let mut sub = pc.child(1.0 - soft);
                let out = if depth < 2 {
                    self.trace(qw, ew, i, depth + 1, &mut sub)
                } else {
                    self.backdrop(qw, ew, 2, &mut sub)
                };
                pc.fold(sub);
                let out = mix(out, self.rim, soft);
                let follow = smooth(FOLLOW, 0.3, f);
                if follow > 0.0 {
                    let mut branch = pc.child(f * follow);
                    let back = onward(&mut branch, bevel);
                    pc.fold(branch);
                    add(mul(out, 1.0 - f * follow), mul(back, f * follow))
                } else {
                    out
                }
            }
            None => {
                pc.mark(0x7e);
                onward(pc, bevel)
            }
        };
        mulc(col, fade)
    }

    fn pixel(&self, x: f32, y: f32, lambda: C) -> (C, Path) {
        let mut pc = Path::new(lambda);
        let c = self.trace([0.0; 3], self.ray(x, y), usize::MAX, 0, &mut pc);
        (c, pc)
    }
}

fn soften(w: f32) -> f32 {
    smooth(SOFT.0, SOFT.1, w)
}

const BOUNCES: usize = 3;
const KEEP: [f32; BOUNCES - 1] = [0.85, 0.5];

struct Path {
    lambda: C,
    sig: u32,
    split: bool,
    w: f32,
}

const FAINT: f32 = 0.08;

impl Path {
    fn new(lambda: C) -> Self {
        Path {
            lambda,
            sig: 0x811c_9dc5,
            split: false,
            w: 1.0,
        }
    }

    fn child(&self, share: f32) -> Self {
        Path {
            w: self.w * share,
            ..Path::new(self.lambda)
        }
    }

    fn fold(&mut self, sub: Path) {
        if sub.w > FAINT {
            self.mark(sub.sig);
            self.split |= sub.split;
        }
    }

    fn mark(&mut self, v: u32) {
        self.sig = (self.sig ^ v).wrapping_mul(0x0100_0193);
    }
}

const RGB: C = [1.0, 0.0, -1.0];

struct SlabSpec {
    x: f32,
    z: f32,
    hw: f32,
    hh: f32,
    ht: f32,
    yaw: f32,
    lean: f32,
    roll: f32,
    hue: f32,
}

fn layout(seed: u32) -> [SlabSpec; 2] {
    let mut s = [
        SlabSpec {
            x: 1.0,
            z: 6.5,
            hw: 0.55,
            hh: 1.05,
            ht: 0.25,
            yaw: 0.5,
            lean: 0.0,
            roll: 0.0,
            hue: -0.4,
        },
        SlabSpec {
            x: 1.98,
            z: 6.6,
            hw: 0.5,
            hh: 0.8,
            ht: 0.07,
            yaw: -0.25,
            lean: 0.0,
            roll: -0.35,
            hue: 0.8,
        },
    ];
    let k = seed ^ SEED;
    if k != 0 {
        let mut rng = Rng(0x9a4e5 ^ k as u64);
        s[0].yaw += rng.r(-0.2, 0.2);
        s[0].x += rng.r(-0.15, 0.15);
        s[1].yaw += rng.r(-0.15, 0.15);
        s[1].roll += rng.r(-0.1, 0.1);
        s[1].x += rng.r(-0.08, 0.08);
    }
    s
}

pub(super) fn render(w: usize, h: usize, v: Variant, seed: u32) -> Vec<u32> {
    let (hue, dark) = hue_of(v, &HUES);
    let slabs = layout(seed)
        .iter()
        .map(|s| {
            let rot = |v: V3| -> V3 {
                let (c, sn) = (s.roll.cos(), s.roll.sin());
                let v = [c * v[0] - sn * v[1], sn * v[0] + c * v[1], v[2]];
                let (c, sn) = (s.lean.cos(), s.lean.sin());
                let v = [v[0], c * v[1] - sn * v[2], sn * v[1] + c * v[2]];
                let (c, sn) = (s.yaw.cos(), s.yaw.sin());
                [c * v[0] + sn * v[2], v[1], -sn * v[0] + c * v[2]]
            };
            let (a0, up, a2) = (
                rot([1.0, 0.0, 0.0]),
                rot([0.0, 1.0, 0.0]),
                rot([0.0, 0.0, 1.0]),
            );
            let low = s.hw * a0[1].abs() + s.hh * up[1].abs() + s.ht * a2[1].abs();
            let c = [s.x, EYE - low, s.z];
            let tl = if dark { 0.80 } else { 0.88 };
            let tint = oklch(tl, 0.13, lit(hue, 12.0 * s.hue));
            let absorb = tint.map(|t| t.max(1e-3).ln() / REF);
            Slab {
                c,
                ax: [a0, up, a2],
                e: [s.hw, s.hh, s.ht],
                ease: EASE,
                absorb,
                r: (s.hw * s.hw + s.hh * s.hh + s.ht * s.ht).sqrt(),
            }
        })
        .collect();
    let pick = |d: C, l: C| if dark { d } else { l };
    let edge = pick(
        oklch(0.72, 0.17, lit(hue, 10.0)),
        oklch(0.80, 0.16, lit(hue, 10.0)),
    );
    let lobe_k = if dark { 0.45 } else { 1.0 };
    let scene = Scene {
        slabs,
        sun: norm3([-0.45, -0.15, 0.9]),
        sky_hz: pick(
            oklch(0.30, 0.08, lit(hue, 10.0)),
            oklch(0.54, 0.13, lit(hue, 10.0)),
        ),
        sky_top: pick(
            oklch(0.12, 0.05, lit(hue, -15.0)),
            oklch(0.32, 0.14, lit(hue, -10.0)),
        ),
        floor_near: pick(
            oklch(0.14, 0.05, lit(hue, -15.0)),
            oklch(0.34, 0.12, lit(hue, -12.0)),
        ),
        floor_far: pick(oklch(0.22, 0.07, hue), oklch(0.48, 0.12, hue)),
        glow: pick(
            oklch(0.72, 0.14, lit(hue, 25.0)),
            oklch(0.92, 0.10, lit(hue, 22.0)),
        ),
        edge,
        haze: mul(edge, HAZE),
        rim: mul(edge, RIM),
        lobe: (LOBE.0 * lobe_k, LOBE.1 * lobe_k, LOBE.2),
    };

    let mut base = vec![Base::default(); w * h];
    par_rows(&mut base, w, |y, row| {
        for (x, o) in row.iter_mut().enumerate() {
            let (sx, sy) = centred(x, y, w, h);
            let (c, pc) = scene.pixel(sx, sy, RGB);
            *o = Base {
                c,
                percept: c.map(|v| v.clamp(0.0, LIGHT_MAX).sqrt()),
                sig: pc.sig,
                split: pc.split,
            };
        }
    });

    let cut_side = if h >= 2000 { CUT_SIDE_4K } else { CUT_SIDE };
    let mut side = vec![0u8; w * h];
    par_rows(&mut side, w, |y, row| {
        for (x, o) in row.iter_mut().enumerate() {
            let a = &base[y * w + x];
            let mut level = if a.split { 2 } else { 0 };
            for (dx, dy) in NEIGHBOURS {
                let (nx, ny) = (x as isize + dx, y as isize + dy);
                if level == 2 || nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                    continue;
                }
                let b = &base[ny as usize * w + nx as usize];
                if b.sig != a.sig || b.split {
                    level = 2;
                } else if (0..3).any(|c| (b.percept[c] - a.percept[c]).abs() > JUMP) {
                    level = 1;
                }
            }
            *o = [0, STEEP_SIDE, cut_side][level] as u8;
        }
    });

    let jitter = 0x9a7e ^ seed ^ SEED;
    let px_sz = 1.0 / h as f32;
    let mut px: Vec<C> = vec![[0.0; 3]; w * h];
    par_rows(&mut px, w, |y, row| {
        for (x, o) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let side = side[i] as usize;
            if side == 0 {
                *o = base[i].c;
                continue;
            }
            let n = side * side;
            let (sx, sy) = centred(x, y, w, h);
            let key = hash2(x as i32, y as i32, jitter);
            let mut acc = [0.0f32; 3];
            for k in 0..n {
                // Multi-jittered: one sample per cell and per row and column, so a near-axis edge sees n coverage levels.
                let r = hash32(key ^ k as u32);
                let (col, row) = (k % side, k / side);
                let fx = (row + (key as usize >> 8) + col * 3) % side;
                let fy = (col + (key as usize >> 16) + row * 3) % side;
                let ox = (col as f32 + (fx as f32 + unit(r)) / side as f32) / side as f32 - 0.5;
                let oy =
                    (row as f32 + (fy as f32 + unit(hash32(r))) / side as f32) / side as f32 - 0.5;
                // Wavelengths are stratified as a Latin square over the grid.
                let lk = ((col + row + (key as usize >> 4)) % side) * side
                    + (row + (key as usize >> 12)) % side;
                let u = (lk as f32 + unit(hash32(r ^ 0x51))) / n as f32 - 0.5;
                let (c, _) = scene.pixel(sx + ox * px_sz, sy + oy * px_sz, RGB.map(|l| l + u));
                acc = add(acc, c);
            }
            *o = mul(acc, 1.0 / n as f32);
        }
    });
    quantise_shoulder(&px, w)
}

const NEIGHBOURS: [(isize, isize); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

#[derive(Clone, Copy, Default)]
struct Base {
    c: C,
    percept: C,
    sig: u32,
    split: bool,
}
