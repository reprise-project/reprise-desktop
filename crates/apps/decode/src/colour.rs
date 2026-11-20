//! Samples in another colour space turned to 8-bit sRGB, for the colour spaces jxl-rs writes an
//! XYB picture in (Display P3, BT.2100 PQ, ...) and the ones an AVIF states (CICP code points or
//! an RGB matrix/TRC ICC profile). Colour outside sRGB is clipped, and PQ is read with
//! `intensity_target` nits as sRGB's white.

use jxl::api::{JxlColorEncoding, JxlPrimaries, JxlTransferFunction, JxlWhitePoint};

use crate::Refused;

/// A transfer curve: a named one, or an ICC profile's.
#[derive(Clone)]
enum Curve {
    Named(JxlTransferFunction),
    Icc(IccCurve),
}

/// A sample's linear value from its encoded one, both 0 to 1.
#[derive(Clone)]
enum IccCurve {
    Gamma(f64),
    /// An ICC `para` curve: its function type (0 to 4) and parameters g, a, b, c, d, e, f.
    Parametric(u16, [f64; 7]),
    /// Linear values evenly spaced over the encoded range.
    Table(Vec<f32>),
}

impl IccCurve {
    fn eval(&self, v: f64) -> f64 {
        match self {
            IccCurve::Gamma(g) => v.powf(*g),
            IccCurve::Parametric(kind, [g, a, b, c, d, e, f]) => {
                let power = |v: f64| (a * v + b).max(0.0).powf(*g);
                match kind {
                    0 => v.powf(*g),
                    1 if v >= -b / a => power(v),
                    1 => 0.0,
                    2 if v >= -b / a => power(v) + c,
                    2 => *c,
                    3 if v >= *d => power(v),
                    3 => c * v,
                    _ if v >= *d => power(v) + e,
                    _ => c * v + f,
                }
            }
            IccCurve::Table(t) => {
                let at = v.clamp(0.0, 1.0) * (t.len() - 1) as f64;
                let i = (at as usize).min(t.len() - 2);
                let w = (at - i as f64) as f32;
                f64::from(t[i] * (1.0 - w) + t[i + 1] * w)
            }
        }
    }
}

/// Encoded samples, each up to `max`, turned to 8-bit sRGB through linear light.
pub struct ToSrgb {
    curve: Curve,
    /// The largest sample: 255 for 8-bit samples, more for deeper ones.
    pub max: u16,
    /// Each sample's linear value.
    linear: Vec<f32>,
    /// HLG's OOTF from scene to display light: the luminance weights of linear RGB, and the
    /// exponent of luminance each sample is multiplied by.
    ootf: Option<([f32; 3], f32)>,
    /// From the picture's linear RGB to linear sRGB; None for grey.
    matrix: Option<[[f32; 3]; 3]>,
    /// sRGB's 8-bit sample for each linear value in 1/65535 steps.
    encode: Vec<u8>,
}

impl ToSrgb {
    /// None for sRGB itself or a colour space this cannot turn.
    pub fn from(enc: &JxlColorEncoding, intensity_target: f32) -> Result<Option<ToSrgb>, Refused> {
        match enc {
            JxlColorEncoding::RgbColorSpace {
                white_point,
                primaries,
                transfer_function,
                ..
            } => ToSrgb::named(
                transfer_function,
                Some((primaries, white_point)),
                intensity_target,
            ),
            JxlColorEncoding::GrayscaleColorSpace {
                transfer_function, ..
            } => ToSrgb::named(transfer_function, None, intensity_target),
            JxlColorEncoding::XYB { .. } => Ok(None),
        }
    }

    /// Samples under `tf`, RGB with `rgb`'s primaries and white or grey without; None for sRGB.
    pub fn named(
        tf: &JxlTransferFunction,
        rgb: Option<(&JxlPrimaries, &JxlWhitePoint)>,
        intensity_target: f32,
    ) -> Result<Option<ToSrgb>, Refused> {
        let (matrix, luma) = match rgb {
            Some((primaries, white)) => {
                let same = *primaries == JxlPrimaries::SRGB && *white == JxlWhitePoint::D65;
                let matrix = (!same).then(|| to_srgb_matrix(primaries, white));
                let luma = rgb_to_xyz(primaries.to_xy_coords(), white.to_xy_coords())
                    .map(|m| m[1].map(|v| v as f32));
                (matrix.flatten(), luma)
            }
            None => (None, Some([1.0 / 3.0; 3])),
        };
        if *tf == JxlTransferFunction::SRGB && matrix.is_none() {
            return Ok(None);
        }
        let ootf = match tf {
            JxlTransferFunction::HLG => luma.map(|l| (l, hlg_system_gamma(intensity_target) - 1.0)),
            _ => None,
        };
        ToSrgb::with(Curve::Named(tf.clone()), matrix, ootf, intensity_target)
    }

    /// Samples under the RGB matrix/TRC ICC `profile`. None for a profile that is about sRGB,
    /// and for one this cannot read: another kind of profile, or a curve per channel.
    pub fn icc(profile: &[u8]) -> Result<Option<ToSrgb>, Refused> {
        let Some((colorants, curve)) = icc_rgb(profile) else {
            return Ok(None);
        };
        let matrix = from_d50(&colorants);
        let srgb_curve = (0..=16).all(|i| {
            let v = f64::from(i) / 16.0;
            let want = to_linear(&JxlTransferFunction::SRGB, v as f32, 1.0);
            (curve.eval(v) - f64::from(want)).abs() < 0.002
        });
        let identity = matrix.as_ref().is_none_or(|m| {
            (0..3).all(|i| (0..3).all(|j| (m[i][j] - f32::from(u8::from(i == j))).abs() < 0.01))
        });
        if srgb_curve && identity {
            return Ok(None);
        }
        ToSrgb::with(Curve::Icc(curve), matrix, None, 1.0)
    }

    fn with(
        curve: Curve,
        matrix: Option<[[f32; 3]; 3]>,
        ootf: Option<([f32; 3], f32)>,
        intensity_target: f32,
    ) -> Result<Option<ToSrgb>, Refused> {
        let linear = linear_table(&curve, u16::from(u8::MAX), intensity_target)?;
        let mut encode = Vec::new();
        encode
            .try_reserve_exact(1 << 16)
            .map_err(|_| Refused::Memory)?;
        encode.extend((0..=u16::MAX).map(|v| {
            let l = f64::from(v) / 65535.0;
            let s = if l <= 0.003_130_8 {
                l * 12.92
            } else {
                1.055 * l.powf(1.0 / 2.4) - 0.055
            };
            (s * 255.0).round() as u8
        }));
        Ok(Some(ToSrgb {
            curve,
            max: u16::from(u8::MAX),
            linear,
            ootf,
            matrix,
            encode,
        }))
    }

    /// Whether 8-bit samples band once turned: PQ, HLG and linear ones.
    pub fn bands_at_8_bits(&self) -> bool {
        use JxlTransferFunction::{HLG, Linear, PQ};
        matches!(&self.curve, Curve::Named(PQ | HLG | Linear))
    }

    /// The same turn for samples up to `max`.
    pub fn at_max(self, max: u16, intensity_target: f32) -> Result<ToSrgb, Refused> {
        Ok(ToSrgb {
            max,
            linear: linear_table(&self.curve, max, intensity_target)?,
            ..self
        })
    }

    /// The 8-bit sRGB RGBA of samples `rgb` at alpha `a`, each up to `max`.
    pub fn pixel(&self, rgb: [u16; 3], a: u16) -> [u8; 4] {
        let mut l = rgb.map(|v| self.linear[usize::from(v.min(self.max))]);
        if let Some((luma, exponent)) = self.ootf {
            let y = luma[0] * l[0] + luma[1] * l[1] + luma[2] * l[2];
            if y > 0.0 {
                let m = y.powf(exponent);
                l = l.map(|v| v * m);
            }
        }
        if let Some(m) = &self.matrix {
            l = m.map(|row| row[0] * l[0] + row[1] * l[1] + row[2] * l[2]);
        }
        let encode = |l: f32| self.encode[(l.clamp(0.0, 1.0) * 65535.0).round() as usize];
        let max = u32::from(self.max);
        let a = ((u32::from(a.min(self.max)) * 255 + max / 2) / max) as u8;
        [encode(l[0]), encode(l[1]), encode(l[2]), a]
    }
}

fn linear_table(curve: &Curve, max: u16, intensity_target: f32) -> Result<Vec<f32>, Refused> {
    let mut linear = Vec::new();
    (linear.try_reserve_exact(usize::from(max) + 1)).map_err(|_| Refused::Memory)?;
    let top = f32::from(max);
    linear.extend((0..=max).map(|v| {
        let v = f32::from(v) / top;
        match curve {
            Curve::Named(tf) => to_linear(tf, v, intensity_target),
            Curve::Icc(c) => c.eval(f64::from(v)) as f32,
        }
    }));
    Ok(linear)
}

/// HLG's system gamma on a display of `intensity` nits, 1.2 at 1000 (BT.2100's extended form).
fn hlg_system_gamma(intensity: f32) -> f32 {
    (1.2 * 1.111f64.powf((f64::from(intensity) / 1000.0).log2())) as f32
}

/// The linear value of sample `v` (0 to 1) under `tf`: for PQ 1 is `intensity_target` nits, for
/// HLG scene light with 1 its peak, before the OOTF.
fn to_linear(tf: &JxlTransferFunction, v: f32, intensity_target: f32) -> f32 {
    let v = f64::from(v);
    let l = match tf {
        JxlTransferFunction::Linear => v,
        JxlTransferFunction::SRGB if v <= 0.04045 => v / 12.92,
        JxlTransferFunction::SRGB => ((v + 0.055) / 1.055).powf(2.4),
        JxlTransferFunction::BT709 if v < 0.081 => v / 4.5,
        JxlTransferFunction::BT709 => ((v + 0.099) / 1.099).powf(1.0 / 0.45),
        JxlTransferFunction::DCI => v.powf(2.6),
        // The encoding exponent, under 1.
        JxlTransferFunction::Gamma(g) => v.powf(1.0 / f64::from(*g)),
        JxlTransferFunction::PQ => {
            let (m1, m2) = (2610.0 / 16384.0, 2523.0 / 4096.0 * 128.0);
            let (c1, c2, c3) = (
                3424.0 / 4096.0,
                2413.0 / 4096.0 * 32.0,
                2392.0 / 4096.0 * 32.0,
            );
            let p = v.powf(1.0 / m2);
            let nits = ((p - c1).max(0.0) / (c2 - c3 * p)).powf(1.0 / m1) * 10000.0;
            nits / f64::from(intensity_target)
        }
        JxlTransferFunction::HLG if v <= 0.5 => v * v / 3.0,
        JxlTransferFunction::HLG => {
            let (a, b, c) = (0.178_832_77, 0.284_668_92, 0.559_910_73);
            (((v - c) / a).exp() + b) / 12.0
        }
    };
    l as f32
}

/// ICC's PCS white, D50.
const D50: (f32, f32) = (0.3457, 0.3585);

/// The matrix from linear RGB under `primaries` and `white` to linear sRGB, the white point
/// adapted to D65 by Bradford; None for degenerate primaries.
fn to_srgb_matrix(primaries: &JxlPrimaries, white: &JxlWhitePoint) -> Option<[[f32; 3]; 3]> {
    let src = rgb_to_xyz(primaries.to_xy_coords(), white.to_xy_coords())?;
    to_srgb(&src, white.to_xy_coords())
}

/// The matrix from linear RGB with ICC `colorants` (XYZ, adapted to D50) to linear sRGB.
fn from_d50(colorants: &[[f64; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    to_srgb(colorants, D50)
}

/// The matrix from linear RGB whose XYZ is `to_xyz` under white `white` to linear sRGB.
fn to_srgb(to_xyz: &[[f64; 3]; 3], white: (f32, f32)) -> Option<[[f32; 3]; 3]> {
    let d65 = JxlWhitePoint::D65.to_xy_coords();
    let srgb = rgb_to_xyz(JxlPrimaries::SRGB.to_xy_coords(), d65)?;
    let m = mul(&invert(&srgb)?, &mul(&adapt(white, d65)?, to_xyz));
    Some(m.map(|row| row.map(|v| v as f32)))
}

/// Bradford's adaptation of XYZ under white `from` to white `to`.
fn adapt(from: (f32, f32), to: (f32, f32)) -> Option<[[f64; 3]; 3]> {
    const BRADFORD: [[f64; 3]; 3] = [
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ];
    let cone = |xy| mul_vec(&BRADFORD, xyz(xy));
    let (from, to) = (cone(from), cone(to));
    let scale =
        std::array::from_fn(|i| std::array::from_fn(|j| f64::from(i == j) * to[i] / from[i]));
    Some(mul(&invert(&BRADFORD)?, &mul(&scale, &BRADFORD)))
}

/// The XYZ of white `xy` at Y 1.
fn xyz((x, y): (f32, f32)) -> [f64; 3] {
    let (x, y) = (f64::from(x), f64::from(y));
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// Linear RGB to XYZ for primaries `rgb` and white `w`, white at Y 1.
pub fn rgb_to_xyz(rgb: [(f32, f32); 3], w: (f32, f32)) -> Option<[[f64; 3]; 3]> {
    let cols = rgb.map(xyz);
    let p = std::array::from_fn(|i| cols.map(|c| c[i]));
    let s = mul_vec(&invert(&p)?, xyz(w));
    Some(std::array::from_fn(|i| {
        std::array::from_fn(|j| p[i][j] * s[j])
    }))
}

fn mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

fn mul_vec(a: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    a.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
}

fn invert(m: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let c = |i: usize, j: usize| {
        let (r, s) = ((i + 1) % 3, (i + 2) % 3);
        let (t, u) = ((j + 1) % 3, (j + 2) % 3);
        m[r][t] * m[s][u] - m[r][u] * m[s][t]
    };
    let det: f64 = (0..3).map(|j| m[0][j] * c(0, j)).sum();
    (det.abs() > 1e-12).then(|| std::array::from_fn(|i| std::array::from_fn(|j| c(j, i) / det)))
}

/// The colorants (columns: red, green and blue XYZ under D50) and the tone curve of an RGB ICC
/// profile with an XYZ connection space and the same curve on each channel.
fn icc_rgb(p: &[u8]) -> Option<([[f64; 3]; 3], IccCurve)> {
    if p.get(16..20)? != b"RGB " || p.get(20..24)? != b"XYZ " {
        return None;
    }
    let count = be32(p, 128)? as usize;
    let tag = |sig: &[u8; 4]| {
        (0..count.min(p.len() / 12)).find_map(|i| {
            let at = 132 + 12 * i;
            if p.get(at..at + 4)? != sig {
                return None;
            }
            let (offset, size) = (be32(p, at + 4)? as usize, be32(p, at + 8)? as usize);
            p.get(offset..offset.checked_add(size)?)
        })
    };
    let s15 = |b: &[u8], at: usize| -> Option<f64> {
        let v = i32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?);
        Some(f64::from(v) / 65536.0)
    };
    let colorant = |sig| -> Option<[f64; 3]> {
        let b = tag(sig)?;
        (b.get(..4)? == b"XYZ ").then_some(())?;
        Some([s15(b, 8)?, s15(b, 12)?, s15(b, 16)?])
    };
    let cols = [colorant(b"rXYZ")?, colorant(b"gXYZ")?, colorant(b"bXYZ")?];
    let (r, g, b) = (tag(b"rTRC")?, tag(b"gTRC")?, tag(b"bTRC")?);
    if r != g || g != b {
        return None;
    }
    let curve = match r.get(..4)? {
        b"curv" => {
            let n = be32(r, 8)? as usize;
            let entry = |i: usize| {
                Some(u16::from_be_bytes(
                    r.get(12 + 2 * i..14 + 2 * i)?.try_into().ok()?,
                ))
            };
            match n {
                0 => IccCurve::Gamma(1.0),
                1 => IccCurve::Gamma(f64::from(entry(0)?) / 256.0),
                _ => {
                    r.get(12..12 + 2 * n)?;
                    let table = (0..n).map(|i| entry(i).map_or(0.0, |e| f32::from(e) / 65535.0));
                    IccCurve::Table(table.collect())
                }
            }
        }
        b"para" => {
            let kind = u16::from_be_bytes(r.get(8..10)?.try_into().ok()?);
            let n = [1, 3, 4, 5, 7].get(usize::from(kind))?;
            let mut params = [0.0; 7];
            for (i, v) in params.iter_mut().take(*n).enumerate() {
                *v = s15(r, 12 + 4 * i)?;
            }
            if kind != 0 && params[1] == 0.0 {
                return None;
            }
            IccCurve::Parametric(kind, params)
        }
        _ => return None,
    };
    let m = std::array::from_fn(|i| cols.map(|c| c[i]));
    Some((m, curve))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An RGB matrix/TRC profile with `colorants` (columns) and `curve`, a `curv` or `para` tag
    /// shared by the three channels.
    pub fn profile(colorants: [[f64; 3]; 3], curve: &[u8]) -> Vec<u8> {
        let mut tags: Vec<(&[u8; 4], Vec<u8>)> = Vec::new();
        for (sig, i) in [(b"rXYZ", 0), (b"gXYZ", 1), (b"bXYZ", 2)] {
            let mut t = b"XYZ \0\0\0\0".to_vec();
            for row in colorants {
                t.extend(((row[i] * 65536.0).round() as i32).to_be_bytes());
            }
            tags.push((sig, t));
        }
        for sig in [b"rTRC", b"gTRC", b"bTRC"] {
            tags.push((sig, curve.to_vec()));
        }
        let mut p = vec![0; 128];
        p[16..20].copy_from_slice(b"RGB ");
        p[20..24].copy_from_slice(b"XYZ ");
        p.extend((tags.len() as u32).to_be_bytes());
        let mut at = 132 + 12 * tags.len();
        let mut data: Vec<u8> = Vec::new();
        for (sig, t) in &tags {
            p.extend(*sig);
            p.extend((at as u32).to_be_bytes());
            p.extend((t.len() as u32).to_be_bytes());
            at += t.len();
            data.extend(t);
        }
        p.extend(data);
        p
    }

    /// sRGB's `para` curve: type 3, g 2.4, a 1/1.055, b 0.055/1.055, c 1/12.92, d 0.04045.
    pub fn srgb_para() -> Vec<u8> {
        let mut c = b"para\0\0\0\0\0\x03\0\0".to_vec();
        for v in [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045] {
            c.extend(((v * 65536.0f64).round() as i32).to_be_bytes());
        }
        c
    }

    /// ICC colorants of `primaries` and `white`: their XYZ adapted to D50.
    pub fn colorants_of(primaries: &JxlPrimaries, white: &JxlWhitePoint) -> [[f64; 3]; 3] {
        let w = white.to_xy_coords();
        let m = rgb_to_xyz(primaries.to_xy_coords(), w).unwrap();
        mul(&adapt(w, D50).unwrap(), &m)
    }

    #[test]
    fn an_srgb_profile_needs_no_turning_and_a_p3_one_does() {
        let srgb = colorants_of(&JxlPrimaries::SRGB, &JxlWhitePoint::D65);
        assert!(ToSrgb::icc(&profile(srgb, &srgb_para())).unwrap().is_none());
        let gamma22 = b"curv\0\0\0\0\0\0\0\x01\x02\x33";
        let t = ToSrgb::icc(&profile(srgb, gamma22))
            .unwrap()
            .expect("gamma 2.2 is turned");
        assert!(
            t.matrix
                .as_ref()
                .is_none_or(|m| (m[0][0] - 1.0).abs() < 0.01)
        );
        let p3 = colorants_of(&JxlPrimaries::P3, &JxlWhitePoint::D65);
        let t = ToSrgb::icc(&profile(p3, &srgb_para()))
            .unwrap()
            .expect("P3 is turned");
        let red = t.pixel([234, 51, 36], 255);
        assert!(red[0] >= 247 && red[1] <= 8 && red[2] <= 8, "{red:?}");
    }

    #[test]
    fn a_profile_this_cannot_read_reads_as_srgb() {
        let srgb = colorants_of(&JxlPrimaries::SRGB, &JxlWhitePoint::D65);
        let mut p = profile(srgb, b"curv\0\0\0\0\0\0\0\x01\x02\x33");
        let r_trc = p.windows(4).position(|w| w == b"curv").unwrap();
        p[r_trc + 13] = 0x66;
        assert!(ToSrgb::icc(&p).unwrap().is_none(), "a curve per channel");
        let mut cmyk = profile(srgb, &srgb_para());
        cmyk[16..20].copy_from_slice(b"CMYK");
        assert!(ToSrgb::icc(&cmyk).unwrap().is_none());
        assert!(ToSrgb::icc(&[0; 40]).unwrap().is_none());
    }
}
