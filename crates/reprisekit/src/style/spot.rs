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
    pub fn in_colour(&self, glow: Option<Color>) -> SpotLight {
        SpotLight {
            layers: [self.halo, self.core].map(|l| (l, l.colour(glow))),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SpotLight {
    layers: [(SpotLayer, [f32; 3]); 2],
}

impl SpotLight {
    pub fn light(&self, under: u32, g: f32, x: f32, y: f32, w: f32, h: f32) -> u32 {
        if g == 0.0 {
            return under;
        }
        let (mut keep, mut add) = (1.0f32, [0.0f32; 3]);
        for (layer, colour) in &self.layers {
            let a = 1.0 - (1.0 - layer.alpha(x, y, w, h)).powf(g);
            keep *= 1.0 - a;
            for (v, c) in add.iter_mut().zip(colour) {
                *v = *v * (1.0 - a) + a * c;
            }
        }
        let ch = |sh: u32, add: f32| {
            let v = ((under >> sh) & 0xFF) as f32 * keep + add;
            (v.round().clamp(0.0, 255.0) as u32) << sh
        };
        ch(24, 255.0 * (1.0 - keep)) | ch(16, add[0]) | ch(8, add[1]) | ch(0, add[2])
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotLayer {
    pub peak: f32,
    pub rx: f32,
    pub ry: f32,
    pub power: f32,
    pub foot: f32,
    pub white: [u8; 3],
    pub tint: f32,
}

impl SpotLayer {
    pub fn alpha(&self, x: f32, y: f32, w: f32, h: f32) -> f32 {
        let d = (x / (self.rx * w)).powi(2) + ((y - self.foot * h) / (self.ry * h)).powi(2);
        (self.peak / (1.0 + d.powf(self.power))).min(1.0)
    }

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
