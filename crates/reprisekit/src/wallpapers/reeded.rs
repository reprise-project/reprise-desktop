//! Reeded: a light behind a pane of reeded glass. Left of the pane's edge
//! the light shows clean: a soft disc lit from its upper left, one thin
//! ring, a bloom and a cooler lobe low right. Right of it every vertical
//! rib is a small cylindrical lens showing a mirrored, squeezed slice of
//! what is behind, a little wider in red than in blue; grooves draw thin
//! dark lines, crowns catch a highlight, and the pane's edge carries a
//! bright bevel. Drawn in linear light; every colour comes from the
//! variant's hue.

use super::noise::hash32;
use super::noise::light::{ffloor, unit};
use super::tone::{C, add, lit, mix, mul, oklch, quantise_shoulder, smooth};
use super::{HUES, SEED, Variant, centred, hue_of, par_rows};

struct Reeded {
    bg_lo: C,
    bg_hi: C,
    core: C,
    glow: C,
    cool: C,
    orb: (f32, f32, f32),
    edge: f32,
    rib: f32,
    mag: f32,
    /// Red bends this much more than green, blue this much less.
    disp: f32,
    lean: f32,
    key: u32,
}

impl Reeded {
    fn behind(&self, x: f32, y: f32) -> C {
        let (ox, oy, r) = self.orb;
        let (dx, dy) = (x - ox, y - oy);
        let d2 = (dx * dx + dy * dy) / (r * r);
        let t = smooth(0.0, 2.2, d2.sqrt() * 0.5 + (y + 0.5) * 0.25);
        let mut c = mix(self.bg_hi, self.bg_lo, t);
        let dr = d2.sqrt();
        // The disc is lit like a sphere from the upper left: hot there,
        // falling to the glow colour at its far edge.
        let disc = smooth(1.0, 0.82, dr);
        let (lx, ly) = (dx / r + 0.35, dy / r + 0.45);
        let hot = (-(lx * lx + ly * ly) / 0.55).exp();
        c = add(
            c,
            mul(mix(self.glow, self.core, 0.35 + 0.65 * hot), 0.95 * disc),
        );
        let ring = 0.45 * (-((dr - 1.38) * (dr - 1.38)) / 0.0005).exp();
        c = add(c, mul(self.glow, 0.8 * ring));
        c = add(
            c,
            mul(
                self.glow,
                0.55 * (-d2 * 0.9).exp() + 0.25 * (-d2 * 0.18).exp(),
            ),
        );
        let (ex, ey) = (x - ox - 1.6 * r, y - oy - 0.9 * r);
        let e2 = (ex * ex * 0.6 + ey * ey * 1.4) / (r * r);
        add(c, mul(self.cool, 0.45 * (-e2 * 1.2).exp()))
    }

    fn shade(&self, x: f32, y: f32, aa: f32) -> C {
        let ex = x - self.lean * y;
        let clear = self.behind(x, y);
        if ex < self.edge - aa * 4.0 - 0.01 {
            return clear;
        }
        let f = (ex - self.edge) / self.rib;
        let fi = ffloor(f);
        // -1..1 across one rib.
        let u = (f - fi) * 2.0 - 1.0;
        // Cylinder profile: the crown is flat, the sides steep.
        let tilt = u / (1.0 - u * u * 0.85).sqrt();
        let mut c = [0.0f32; 3];
        for (ch, o) in c.iter_mut().enumerate() {
            let m = self.mag * (1.0 + (ch as f32 - 1.0) * self.disp);
            *o = self.behind(x - tilt * self.rib * m, y)[ch];
        }
        // Grooves darken, crowns lift.
        let groove = smooth(0.80, 1.0, u.abs());
        let rib_var = 0.95 + 0.08 * unit(hash32(fi as i32 as u32 ^ self.key));
        c = mul(c, rib_var * (0.92 - 0.38 * groove));
        let crown = (-((u + 0.35) * (u + 0.35)) / 0.03).exp();
        let lightness = self.glow_at(x, y);
        c = add(c, mul(self.glow, 0.10 * crown * (0.3 + lightness)));
        // The pane's own edge: a bright bevel line just right of the clear
        // part.
        let de = ex - self.edge;
        let bevel = (-(de * de) / (0.0025 * 0.0025)).exp();
        c = add(c, mul(self.core, 0.6 * bevel * (0.35 + lightness)));
        let cov = smooth(-aa, aa, de);
        mix(clear, c, cov)
    }

    fn glow_at(&self, x: f32, y: f32) -> f32 {
        let (ox, oy, r) = self.orb;
        let d2 = ((x - ox).powi(2) + (y - oy).powi(2)) / (r * r);
        (-d2 * 0.5).exp()
    }
}

pub(super) fn render(w: usize, h: usize, v: Variant, seed: u32) -> Vec<u32> {
    let (hue, dark) = hue_of(v, &HUES);
    let pick = |d: C, l: C| if dark { d } else { l };
    let scene = Reeded {
        bg_lo: pick(
            oklch(0.05, 0.025, lit(hue, -15.0)),
            oklch(0.17, 0.10, lit(hue, -15.0)),
        ),
        bg_hi: pick(oklch(0.17, 0.07, hue), oklch(0.40, 0.15, hue)),
        core: pick(
            oklch(0.92, 0.06, lit(hue, 40.0)),
            oklch(0.97, 0.05, lit(hue, 40.0)),
        ),
        glow: pick(
            oklch(0.62, 0.17, lit(hue, 18.0)),
            oklch(0.80, 0.15, lit(hue, 18.0)),
        ),
        cool: pick(
            oklch(0.45, 0.16, lit(hue, -35.0)),
            oklch(0.60, 0.17, lit(hue, -35.0)),
        ),
        orb: (0.16, -0.04, 0.24),
        edge: 0.16,
        rib: 0.045,
        mag: 2.6,
        disp: 0.06,
        lean: 0.0,
        key: 0xa11 ^ seed ^ SEED,
    };
    let aa = 0.8 / h as f32;
    let mut px: Vec<C> = vec![[0.0; 3]; w * h];
    par_rows(&mut px, w, |y, row| {
        for (x, out) in row.iter_mut().enumerate() {
            let (sx, sy) = centred(x, y, w, h);
            *out = scene.shade(sx, sy, aa);
        }
    });
    quantise_shoulder(&px, w)
}
