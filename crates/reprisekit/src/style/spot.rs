use std::sync::{Arc, Mutex, PoisonError};

use crate::raster::Canvas;
use crate::theme::Color;

use super::{PartImage, State};

#[derive(Clone, Copy, Debug)]
pub struct TaskGlow {
    pub colour: fn(&Canvas) -> Option<Color>,
    pub label_edges: Option<GlowingPair>,
}

pub type GlowingPair = fn(&PartImage, State, usize, Color) -> PartImage;

#[derive(Clone, Debug)]
pub struct TaskSpot {
    pub fit: SpotFit,
    pub button: SpotFrames,
    pub labelled: Option<SpotFrames>,
    pub edges: Option<SpotFrames>,
}

#[derive(Clone, Debug)]
pub struct SpotFrames {
    pub base: PartImage,
    /// Laid out like `base`; each pixel's gain in 32nds in its low byte.
    pub gain: PartImage,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotFit {
    pub dx: f32,
    pub halo: SpotLayer,
    pub core: SpotLayer,
}

impl SpotFit {
    /// The light over a `w` x `h` frame, in design px.
    pub fn in_colour(&self, glow: Option<Color>, w: f32, h: f32) -> SpotLight {
        SpotLight {
            layers: [self.halo, self.core].map(|l| LitLayer::new(&l, glow, w, h)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SpotLight {
    layers: [LitLayer; 2],
}

impl SpotLight {
    /// `under` lit through gain `g`, `x` design px right of the spot's centre and `y` below the frame's top.
    #[inline]
    pub fn light(&self, under: u32, g: f32, x: f32, y: f32) -> u32 {
        if g == 0.0 {
            return under;
        }
        let (g, x, y) = (f64::from(g), f64::from(x), f64::from(y));
        let [halo, core] = &self.layers;
        let (uh, uc) = (halo.unlit(g, x, y), core.unlit(g, x, y));
        let keep = uh * uc;
        let add = [0, 1, 2].map(|i| (1.0 - uh) * uc * halo.colour[i] + (1.0 - uc) * core.colour[i]);
        let ch = |sh: u32, add: f64| {
            let v = f64::from((under >> sh) & 0xFF) * keep + add;
            ((v + 0.5).min(255.0) as u32) << sh
        };
        ch(24, 255.0 * (1.0 - keep)) | ch(16, add[0]) | ch(8, add[1]) | ch(0, add[2])
    }
}

#[derive(Clone, Debug)]
struct LitLayer {
    falloff: Arc<Falloff>,
    /// Reciprocals of the radii and the centre's height, in design px.
    rx: f64,
    ry: f64,
    foot: f64,
    colour: [f64; 3],
}

impl LitLayer {
    fn new(l: &SpotLayer, glow: Option<Color>, w: f32, h: f32) -> LitLayer {
        LitLayer {
            falloff: falloff(l),
            rx: 1.0 / f64::from(l.rx * w),
            ry: 1.0 / f64::from(l.ry * h),
            foot: f64::from(l.foot * h),
            colour: l.colour(glow).map(f64::from),
        }
    }

    /// The share of the pixel under left through gain `g`, `(1 - alpha)^g`.
    #[inline(always)]
    fn unlit(&self, g: f64, x: f64, y: f64) -> f64 {
        let (dx, dy) = (x * self.rx, (y - self.foot) * self.ry);
        exp_neg(g * self.falloff.ln_unlit((dx * dx + dy * dy).sqrt()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotLayer {
    /// Below 1: the light never covers a pixel.
    pub peak: f32,
    pub rx: f32,
    pub ry: f32,
    /// Above 0.5, so the falloff is flat at the centre.
    pub power: f32,
    pub foot: f32,
    pub white: [u8; 3],
    pub tint: f32,
}

impl SpotLayer {
    pub fn colour(&self, glow: Option<Color>) -> [f32; 3] {
        let white = self.white.map(f32::from);
        match glow.map(glow_chroma).filter(|c| *c != [0.0; 3]) {
            Some(c) => [0, 1, 2].map(|i| (white[i] + self.tint * c[i]).clamp(0.0, 255.0)),
            None => [(0.299 * white[0] + 0.587 * white[1] + 0.114 * white[2]).round(); 3],
        }
    }
}

pub fn glow_chroma(c: Color) -> [f32; 3] {
    let v = [c.r, c.g, c.b].map(f32::from);
    let hi = v[0].max(v[1]).max(v[2]);
    let lo = v[0].min(v[1]).min(v[2]);
    if hi == lo {
        return [0.0; 3];
    }
    let full = v.map(|x| (x - lo) * 255.0 / (hi - lo));
    let mean = full.iter().sum::<f32>() / 3.0;
    full.map(|x| x - mean)
}

const NODES: usize = 512;
const REACH: f64 = 4.0;

/// `ln(1 - alpha)` of a layer against its reach, as cubic Hermite nodes on
/// `0..REACH` (value, slope times the node step); a lit pixel's alpha `a`
/// through gain `g` leaves `(1 - a)^g = exp(g ln(1 - a))` of the pixel under it.
#[derive(Debug)]
struct Falloff {
    peak: f64,
    q: f64,
    nodes: Box<[(f64, f64)]>,
}

impl Falloff {
    fn new(peak: f32, power: f32) -> Falloff {
        assert!(peak < 1.0 && power > 0.5, "spot layer {peak} {power}");
        let (peak, q) = (f64::from(peak), 2.0 * f64::from(power));
        let step = REACH / NODES as f64;
        let nodes = (0..=NODES)
            .map(|i| {
                let r = i as f64 * step;
                let s = r.powf(q);
                let slope = peak * q * r.powf(q - 1.0) / ((1.0 + s) * (1.0 + s - peak));
                ((1.0 - peak / (1.0 + s)).ln(), slope * step)
            })
            .collect();
        Falloff { peak, q, nodes }
    }

    /// At `r`, the pixel's distance from the layer's centre in its radii.
    #[inline(always)]
    fn ln_unlit(&self, r: f64) -> f64 {
        let u = r * (NODES as f64 / REACH);
        let i = u as usize;
        if i >= NODES {
            return (1.0 - self.peak / (1.0 + r.powf(self.q))).ln();
        }
        let t = u - i as f64;
        let ((f0, d0), (f1, d1)) = (self.nodes[i], self.nodes[i + 1]);
        let (t2, t3) = (t * t, t * t * t);
        f0 * (2.0 * t3 - 3.0 * t2 + 1.0)
            + d0 * (t3 - 2.0 * t2 + t)
            + f1 * (3.0 * t2 - 2.0 * t3)
            + d1 * (t3 - t2)
    }
}

/// `e^t` for `t <= 0` to about 1e-9 relative: `2^k e^r`, `k` the integer nearest
/// `t log2 e` and `|r| <= ln 2 / 2`, with `e^r` by its Taylor series to `r^8` in Estrin's scheme.
#[inline(always)]
fn exp_neg(t: f64) -> f64 {
    use std::f64::consts::{LN_2, LOG2_E};
    // Adding 1.5 * 2^52 rounds to an integer, held in the low mantissa bits.
    const SHIFT: f64 = 6_755_399_441_055_744.0;
    let t = t.max(-700.0);
    let z = t * LOG2_E + SHIFT;
    let r = t - (z - SHIFT) * LN_2;
    let (r2, r4) = (r * r, r * r * r * r);
    let lo = (1.0 + r) + r2 * (1.0 / 2.0 + r * (1.0 / 6.0));
    let hi = (1.0 / 24.0 + r * (1.0 / 120.0)) + r2 * (1.0 / 720.0 + r * (1.0 / 5040.0));
    let k = z.to_bits() as i64 - SHIFT.to_bits() as i64;
    (lo + r4 * (hi + r4 * (1.0 / 40320.0))) * f64::from_bits(((k + 1023) << 52) as u64)
}

fn falloff(l: &SpotLayer) -> Arc<Falloff> {
    static KEPT: Mutex<Vec<(f32, f32, Arc<Falloff>)>> = Mutex::new(Vec::new());
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((.., f)) = kept.iter().find(|k| (k.0, k.1) == (l.peak, l.power)) {
        return f.clone();
    }
    let f = Arc::new(Falloff::new(l.peak, l.power));
    kept.push((l.peak, l.power, f.clone()));
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass::taskbutton::SPOT;

    fn powf_light(fit: &SpotFit, glow: Option<Color>, under: u32, g: f32, p: [f32; 4]) -> u32 {
        let [x, y, w, h] = p;
        if g == 0.0 {
            return under;
        }
        let (mut keep, mut add) = (1.0f32, [0.0f32; 3]);
        for layer in [fit.halo, fit.core] {
            let d = (x / (layer.rx * w)).powi(2) + ((y - layer.foot * h) / (layer.ry * h)).powi(2);
            let alpha = (layer.peak / (1.0 + d.powf(layer.power))).min(1.0);
            let a = 1.0 - (1.0 - alpha).powf(g);
            keep *= 1.0 - a;
            for (v, c) in add.iter_mut().zip(layer.colour(glow)) {
                *v = *v * (1.0 - a) + a * c;
            }
        }
        let ch = |sh: u32, add: f32| {
            let v = ((under >> sh) & 0xFF) as f32 * keep + add;
            (v.round().clamp(0.0, 255.0) as u32) << sh
        };
        ch(24, 255.0 * (1.0 - keep)) | ch(16, add[0]) | ch(8, add[1]) | ch(0, add[2])
    }

    #[test]
    fn the_tabled_falloff_lights_each_channel_within_one_of_powf() {
        let glows = [
            None,
            Some(Color::rgb(40, 120, 220)),
            Some(Color::rgb(230, 60, 20)),
        ];
        let unders = [0x0000_0000, 0xFF20_3040, 0xC0A0_6010];
        let (mut worst, mut differ, mut total) = (0, 0u64, 0u64);
        for glow in glows {
            for (w, h) in [(44, 30), (60, 40), (160, 40)] {
                let (wf, hf) = (w as f32, h as f32);
                let light = SPOT.in_colour(glow, wf, hf);
                for n in [1, 2] {
                    let mid = |v: i32| (v as f32 + 0.5) / n as f32;
                    for at in [0, w / 4, w / 2, w - 1] {
                        let cx = at as f32 + 0.5 + SPOT.dx;
                        for gain in [1u8, 13, 32, 64, 255] {
                            let g = f32::from(gain) / 32.0;
                            for py in (0..h * n).map(mid) {
                                for px in (0..w * n).map(mid) {
                                    for under in unders {
                                        let new = light.light(under, g, px - cx, py);
                                        let old = powf_light(
                                            &SPOT,
                                            glow,
                                            under,
                                            g,
                                            [px - cx, py, wf, hf],
                                        );
                                        let d = (0..4)
                                            .map(|i| {
                                                ((new >> (8 * i)) as u8)
                                                    .abs_diff((old >> (8 * i)) as u8)
                                            })
                                            .max()
                                            .unwrap();
                                        worst = worst.max(d);
                                        differ += u64::from(d > 0);
                                        total += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        eprintln!("worst {worst}, {differ} of {total} pixels differ");
        assert!(worst <= 1, "a channel moved by {worst}");
    }

    #[test]
    fn exp_neg_is_exp_to_1e_9() {
        for i in 0..=700_000 {
            let t = -f64::from(i) / 1000.0;
            let (got, want) = (exp_neg(t), t.exp());
            assert!(
                (got - want).abs() <= 1e-9 * want,
                "e^{t}: {got} against {want}"
            );
        }
    }
}
