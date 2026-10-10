//! What the glass lays over its blurred backdrop B, per pixel:
//!
//! ```text
//! glass = blur·B + colour·C + afterglow·C·luma(B)/255
//! out   = B + strength·(glass - B)
//! ```
//!
//! C and the three shares come from `reprisekit::glass::tint::material_glass`. A shader can multiply by
//! the backdrop, so this is the formula itself, not the one-alpha fit a blended art pixel settles for.
//! The strength is the surface's: focused frames and the taskbar lie at full strength; an unfocused
//! Smoked window keeps `SMOKE_INACTIVE` of it; an unfocused Frosted or Molten one lies as far over the
//! backdrop as its plain glass does against the focused frame's (alpha over alpha).

use reprisekit::glass::tint::{self, Surface};
use reprisekit::glass::{Material, Settings};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tint {
    pub colour: [f32; 3],
    pub balance: [f32; 3],
    pub strength: f32,
    pub material: Material,
}

impl Tint {
    pub fn new(s: &Settings, surface: Surface) -> Tint {
        let (c, b) = tint::material_glass(s);
        let strength = match (surface, s.material) {
            (Surface::Active | Surface::Taskbar, _) => 1.0,
            (Surface::Inactive, Material::Smoked) => tint::SMOKE_INACTIVE,
            (Surface::Inactive, Material::Frosted | Material::Molten) => {
                let active = tint::plain(s, Surface::Active).0;
                if active > 0.0 {
                    (tint::plain(s, Surface::Inactive).0 / active).min(1.0)
                } else {
                    0.0
                }
            }
        };
        Tint {
            colour: c.map(|v| (v / 255.0) as f32),
            balance: b.map(|v| v as f32),
            strength: strength as f32,
            material: s.material,
        }
    }

    /// What the glass shows over a backdrop pixel `b` (levels 0..255): the shader's formula on the CPU, unrounded.
    pub fn over(&self, b: [f64; 3]) -> [f64; 3] {
        let l = (0.299 * b[0] + 0.587 * b[1] + 0.114 * b[2]) / 255.0;
        let [cb, ab, bb] = self.balance.map(f64::from);
        let k = f64::from(self.strength);
        std::array::from_fn(|i| {
            let c = f64::from(self.colour[i]) * 255.0;
            let glass = (bb * b[i] + cb * c + ab * c * l).clamp(0.0, 255.0);
            b[i] + k * (glass - b[i])
        })
    }

    pub(crate) fn material_index(&self) -> f32 {
        match self.material {
            Material::Frosted => 0.0,
            Material::Smoked => 1.0,
            Material::Molten => 2.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::glass::SWATCHES;

    fn sky(material: Material, intensity: u8) -> Settings {
        Settings {
            color: SWATCHES[0].color,
            material,
            intensity,
            ..Settings::default()
        }
    }

    #[test]
    fn frosted_sky_over_white_is_the_formula() {
        let t = Tint::new(&sky(Material::Frosted, 42), Surface::Active);
        let want = [112.0, 175.0, 252.0].map(|c| 0.49 * 255.0 + 0.08 * c + 0.43 * c);
        let got = t.over([255.0; 3]);
        for i in 0..3 {
            assert!((got[i] - want[i]).abs() < 1e-3, "{got:?} is {want:?}");
        }
    }

    #[test]
    fn an_unfocused_smoked_window_keeps_most_of_its_glass() {
        let s = sky(Material::Smoked, 42);
        assert_eq!(Tint::new(&s, Surface::Active).strength, 1.0);
        assert_eq!(
            Tint::new(&s, Surface::Inactive).strength,
            tint::SMOKE_INACTIVE as f32
        );
        let f = Tint::new(&sky(Material::Frosted, 42), Surface::Inactive).strength;
        assert!(
            (0.3..0.7).contains(&f),
            "unfocused Frosted glass lies lighter: {f}"
        );
    }
}
