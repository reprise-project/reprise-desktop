//! A JPEG XL's first frame through jxl-rs, refused past a pixel budget, a heap budget or a
//! deadline.
//!
//! jxl-rs takes no allocator, so its heap is held by [`crate::heapcap`] on the decoding thread:
//! past [`HEAP_PER_PIXEL_BYTE`] times the pixel budget a large request gets null, which jxl-rs
//! answers with an out-of-memory error. The image and each frame may also state no more samples
//! (pixels times channels) than the budget's RGBA bytes, and Level 5's limits stay on.

use std::time::{Duration, Instant};

use jxl::api::{
    AlphaMode, Endianness, Event, JxlColorEncoding, JxlColorProfile, JxlColorType, JxlDataFormat,
    JxlDecoder, JxlDecoderOptions, JxlOutputBuffer, JxlParallelRunner, JxlParallelRunnerFun,
    JxlPixelFormat, JxlPrimaries, JxlTransferFunction, JxlWhitePoint,
};
use jxl::error::Error;

use crate::heapcap;

/// What a decode may hold at once, per byte of the pixel budget: an XYB picture at the budget
/// takes about 2.3 times its RGBA, and a reference frame 4 more (f32 a channel).
pub const HEAP_PER_PIXEL_BYTE: u64 = 3;

/// Why a JPEG XL gave no picture.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// The image or one of its frames states more than the pixel budget.
    Size,
    /// The decode asked for more than its heap budget, the file passes Level 5's limits, or the
    /// heap budget cannot be held because [`heapcap::Capped`] is not the global allocator.
    Memory,
    /// The decode ran past its time limit.
    Time,
    /// Not a JPEG XL jxl-rs reads, or cut short.
    Broken,
}

/// Unpremultiplied 8-bit sRGB RGBA, rows top to bottom, with the file's orientation applied.
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub px: Vec<u8>,
}

/// The first frame of `bytes` (an animation's first, composed), refused when the image or a
/// frame states more samples than `pixel_bytes`, the decode holds more than
/// [`HEAP_PER_PIXEL_BYTE`] times that, or it takes longer than `time_limit`.
pub fn first_frame(bytes: &[u8], pixel_bytes: u64, time_limit: Duration) -> Result<Rgba, Refused> {
    let heap = pixel_bytes.saturating_mul(HEAP_PER_PIXEL_BYTE);
    let heap = usize::try_from(heap).unwrap_or(usize::MAX);
    match heapcap::within(heap, || decode(bytes, pixel_bytes, time_limit)) {
        Ok((Err(_), spent)) if spent.refused => Err(Refused::Memory),
        Ok((got, _)) => got,
        Err(heapcap::NotInstalled) => Err(Refused::Memory),
    }
}

fn decode(bytes: &[u8], pixel_bytes: u64, time_limit: Duration) -> Result<Rgba, Refused> {
    let mut options = JxlDecoderOptions::default();
    // jxl-rs refuses a count at or past the limit.
    options.sample_limit = Some(usize::try_from(pixel_bytes).map_or(usize::MAX, |p| p + 1));
    options.alpha_mode = AlphaMode::Unpremultiplied;
    let mut dec = JxlDecoder::new(options);
    let mut clock = Clock {
        deadline: Instant::now() + time_limit,
        late: false,
    };
    let mut input = bytes;
    let mut out: Option<Out> = None;
    loop {
        if clock.check().is_err() {
            return Err(Refused::Time);
        }
        let left = input.len();
        let mut bufs = out.as_mut().map(|o| {
            let (w, h) = o.size;
            let row = w * o.channels * o.sample_bytes();
            [JxlOutputBuffer::new(&mut o.px[..row * h], h, row)]
        });
        let event = dec.process(
            &mut input,
            bufs.as_mut().map(|b| &mut b[..]),
            Some(&mut clock),
        );
        match event.map_err(|e| refusal(e, clock.late))? {
            Event::BasicInfo => out = Some(Out::start(&mut dec, pixel_bytes)?),
            Event::FrameHeader => {}
            Event::FrameComplete { .. } => return out.ok_or(Refused::Broken).map(Out::finish),
            Event::NeedMoreInput { .. } if input.len() < left => {}
            Event::NeedMoreInput { .. } | Event::Complete => return Err(Refused::Broken),
        }
    }
}

fn refusal(e: Error, late: bool) -> Refused {
    match e {
        _ if late => Refused::Time,
        Error::ImageSizeTooLarge(..) | Error::ImageDimensionTooLarge(_) => Refused::Size,
        Error::OutOfMemory(_) | Error::ImageOutOfMemory(..) | Error::LevelLimitExceeded { .. } => {
            Refused::Memory
        }
        _ => Refused::Broken,
    }
}

/// The picture being decoded: `channels` (grey and alpha, or RGBA) a pixel at the start of `px`,
/// which holds the RGBA it becomes. Samples are 8-bit, or 16-bit (native-endian) for a picture
/// turned to sRGB here.
struct Out {
    size: (usize, usize),
    channels: usize,
    px: Vec<u8>,
    to_srgb: Option<ToSrgb>,
}

impl Out {
    /// Asks `dec` for its samples once its basic info is read.
    fn start(dec: &mut JxlDecoder, pixel_bytes: u64) -> Result<Out, Refused> {
        let info = dec.basic_info().ok_or(Refused::Broken)?;
        let (w, h) = info.size;
        if (w as u64).saturating_mul(h as u64).saturating_mul(4) > pixel_bytes {
            return Err(Refused::Size);
        }
        let xyb = !info.uses_original_profile;
        let intensity = info.tone_mapping.intensity_target;
        let extra = info.extra_channels.len();
        let grey = dec
            .current_pixel_format()
            .is_some_and(|f| f.color_type.is_grayscale());
        let format = |samples| JxlPixelFormat {
            color_type: if grey {
                JxlColorType::GrayscaleAlpha
            } else {
                JxlColorType::Rgba
            },
            color_data_format: Some(samples),
            extra_channel_format: vec![None; extra],
        };
        // The colour type picks the output profile, so it is set before the profile is read.
        (dec.set_pixel_format(format(JxlDataFormat::U8 { bit_depth: 8 })))
            .map_err(|_| Refused::Broken)?;
        let to_srgb = match dec.output_color_profile() {
            Some(JxlColorProfile::Simple(enc)) if xyb => ToSrgb::from(enc, intensity)?,
            _ => None,
        };
        // 8-bit PQ, HLG or linear samples band once turned to sRGB. 16-bit ones cost a byte more
        // per byte of RGBA, so a picture over half the pixel budget keeps 8 bits within the heap
        // budget.
        let room = (w as u64).saturating_mul(h as u64).saturating_mul(8) <= pixel_bytes;
        let to_srgb = match to_srgb {
            Some(t) if t.bands_at_8_bits() && room => Some(t.at_16_bits(intensity)?),
            other => other,
        };
        if to_srgb.as_ref().is_some_and(|t| t.max == u16::MAX) {
            let u16 = JxlDataFormat::U16 {
                endianness: Endianness::native(),
                bit_depth: 16,
            };
            dec.set_pixel_format(format(u16))
                .map_err(|_| Refused::Broken)?;
        }
        let out = Out {
            size: (w, h),
            channels: if grey { 2 } else { 4 },
            px: Vec::new(),
            to_srgb,
        };
        let len = w * h * out.channels * out.sample_bytes();
        Ok(Out {
            px: zeroed(len.max(w * h * 4))?,
            ..out
        })
    }

    fn sample_bytes(&self) -> usize {
        match &self.to_srgb {
            Some(t) if t.max == u16::MAX => 2,
            _ => 1,
        }
    }

    fn finish(self) -> Rgba {
        let bytes = self.sample_bytes();
        let Out {
            size: (w, h),
            channels: c,
            mut px,
            to_srgb,
        } = self;
        let stride = c * bytes;
        if stride != 4 || to_srgb.is_some() {
            let mut rgba = |i: usize| {
                let at = stride * i;
                let s = |k: usize| match bytes {
                    1 => u16::from(px[at + k]),
                    _ => u16::from_ne_bytes([px[at + 2 * k], px[at + 2 * k + 1]]),
                };
                let (rgb, a) = match c {
                    2 => ([s(0); 3], s(1)),
                    _ => ([s(0), s(1), s(2)], s(3)),
                };
                let out = match &to_srgb {
                    Some(t) => t.pixel(rgb, a),
                    None => [rgb[0] as u8, rgb[1] as u8, rgb[2] as u8, a as u8],
                };
                px[4 * i..4 * i + 4].copy_from_slice(&out);
            };
            // Each pixel's samples are read before its RGBA is written, over samples of pixels
            // already done.
            if stride >= 4 {
                (0..w * h).for_each(&mut rgba);
            } else {
                (0..w * h).rev().for_each(&mut rgba);
            }
            px.truncate(w * h * 4);
            px.shrink_to_fit();
        }
        Rgba {
            width: w as u32,
            height: h as u32,
            px,
        }
    }
}

/// `n` zero bytes, or [`Refused::Memory`] when the heap budget refuses them.
fn zeroed(n: usize) -> Result<Vec<u8>, Refused> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| Refused::Memory)?;
    v.resize(n, 0);
    Ok(v)
}

/// jxl-rs writes an XYB picture in its own colour space when it can (Display P3, BT.2100 PQ,
/// ...), and has no setter for the output profile, so those samples are turned to sRGB here.
struct ToSrgb {
    tf: JxlTransferFunction,
    /// The largest sample: 255, or 65535 for 16-bit samples.
    max: u16,
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
    fn from(enc: &JxlColorEncoding, intensity_target: f32) -> Result<Option<ToSrgb>, Refused> {
        let (tf, matrix, luma) = match enc {
            JxlColorEncoding::RgbColorSpace {
                white_point,
                primaries,
                transfer_function,
                ..
            } => {
                let same = *primaries == JxlPrimaries::SRGB && *white_point == JxlWhitePoint::D65;
                let matrix = (!same).then(|| to_srgb_matrix(primaries, white_point));
                let luma = rgb_to_xyz(primaries.to_xy_coords(), white_point.to_xy_coords())
                    .map(|m| m[1].map(|v| v as f32));
                (transfer_function, matrix.flatten(), luma)
            }
            JxlColorEncoding::GrayscaleColorSpace {
                transfer_function, ..
            } => (transfer_function, None, Some([1.0 / 3.0; 3])),
            JxlColorEncoding::XYB { .. } => return Ok(None),
        };
        if *tf == JxlTransferFunction::SRGB && matrix.is_none() {
            return Ok(None);
        }
        let ootf = match tf {
            JxlTransferFunction::HLG => luma.map(|l| (l, hlg_system_gamma(intensity_target) - 1.0)),
            _ => None,
        };
        let linear = linear_table(tf, u16::from(u8::MAX), intensity_target)?;
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
            tf: tf.clone(),
            max: u16::from(u8::MAX),
            linear,
            ootf,
            matrix,
            encode,
        }))
    }

    fn bands_at_8_bits(&self) -> bool {
        use JxlTransferFunction::{HLG, Linear, PQ};
        matches!(self.tf, PQ | HLG | Linear)
    }

    fn at_16_bits(self, intensity_target: f32) -> Result<ToSrgb, Refused> {
        Ok(ToSrgb {
            max: u16::MAX,
            linear: linear_table(&self.tf, u16::MAX, intensity_target)?,
            ..self
        })
    }

    /// The 8-bit sRGB RGBA of samples `rgb` at alpha `a`, each up to `max`.
    fn pixel(&self, rgb: [u16; 3], a: u16) -> [u8; 4] {
        let mut l = rgb.map(|v| self.linear[usize::from(v)]);
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
        let a = ((u32::from(a) * 255 + max / 2) / max) as u8;
        [encode(l[0]), encode(l[1]), encode(l[2]), a]
    }
}

fn linear_table(
    tf: &JxlTransferFunction,
    max: u16,
    intensity_target: f32,
) -> Result<Vec<f32>, Refused> {
    let mut linear = Vec::new();
    (linear.try_reserve_exact(usize::from(max) + 1)).map_err(|_| Refused::Memory)?;
    let top = f32::from(max);
    linear.extend((0..=max).map(|v| to_linear(tf, f32::from(v) / top, intensity_target)));
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

/// The matrix from linear RGB under `primaries` and `white` to linear sRGB, the white point
/// adapted to D65 by Bradford; None for degenerate primaries.
fn to_srgb_matrix(primaries: &JxlPrimaries, white: &JxlWhitePoint) -> Option<[[f32; 3]; 3]> {
    let src = rgb_to_xyz(primaries.to_xy_coords(), white.to_xy_coords())?;
    let d65 = JxlWhitePoint::D65.to_xy_coords();
    let srgb = rgb_to_xyz(JxlPrimaries::SRGB.to_xy_coords(), d65)?;
    const BRADFORD: [[f64; 3]; 3] = [
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ];
    let cone = |xy| mul_vec(&BRADFORD, xyz(xy));
    let (from, to) = (cone(white.to_xy_coords()), cone(d65));
    let scale =
        std::array::from_fn(|i| std::array::from_fn(|j| f64::from(i == j) * to[i] / from[i]));
    let adapt = mul(&invert(&BRADFORD)?, &mul(&scale, &BRADFORD));
    let m = mul(&invert(&srgb)?, &mul(&adapt, &src));
    Some(m.map(|row| row.map(|v| v as f32)))
}

/// The XYZ of white `xy` at Y 1.
fn xyz((x, y): (f32, f32)) -> [f64; 3] {
    let (x, y) = (f64::from(x), f64::from(y));
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// Linear RGB to XYZ for primaries `rgb` and white `w`, white at Y 1.
fn rgb_to_xyz(rgb: [(f32, f32); 3], w: (f32, f32)) -> Option<[[f64; 3]; 3]> {
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

/// jxl-rs's runner: each job inline on the calling thread, none started past `deadline`. jxl-rs
/// decodes every group of every frame through it, so a file of many layers stops there too.
struct Clock {
    deadline: Instant,
    late: bool,
}

impl Clock {
    fn check(&mut self) -> jxl::error::Result<()> {
        if Instant::now() < self.deadline {
            return Ok(());
        }
        self.late = true;
        Err(Error::IOError(std::io::ErrorKind::TimedOut.into()))
    }
}

impl JxlParallelRunner for Clock {
    fn run(&mut self, num: usize, fun: &JxlParallelRunnerFun<'_>) -> jxl::error::Result<()> {
        self.run_ordered(num, None, fun)
    }

    fn num_threads(&self) -> usize {
        1
    }

    fn run_ordered(
        &mut self,
        num: usize,
        _max_threads: Option<usize>,
        fun: &JxlParallelRunnerFun<'_>,
    ) -> jxl::error::Result<()> {
        for i in 0..num {
            self.check()?;
            fun(i)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pictures::*;

    /// The JPEG XL test pictures' time limit, far past what any of them takes.
    const NO_HURRY: Duration = Duration::from_secs(60);

    #[test]
    fn a_lossy_picture_tagged_with_an_icc_profile_decodes_to_srgb() {
        let grey = [128, 128, 128, 255].repeat(64 * 64);
        let icc = tabled_srgb_icc();
        let bytes = Picture {
            colour: Colour::Icc(&icc),
            ..Picture::new(64, 64, &[(&grey, 1)])
        }
        .encode();
        let px = first_frame(&bytes, 1 << 20, NO_HURRY).unwrap().px;
        assert!(
            px[..3].iter().all(|&v| v.abs_diff(128) <= 4),
            "grey 128 stays near 128, not its linear value (about 56): {:?}",
            &px[..3]
        );
    }

    #[test]
    fn a_lossy_display_p3_picture_decodes_to_srgb() {
        let red_in_p3 = [234, 51, 36, 255].repeat(64 * 64);
        let bytes = Picture {
            colour: Colour::P3,
            ..Picture::new(64, 64, &[(&red_in_p3, 1)])
        }
        .encode();
        let px = first_frame(&bytes, 1 << 20, NO_HURRY).unwrap().px;
        assert!(
            px[0] >= 247 && px[1] <= 8 && px[2] <= 8,
            "sRGB red, not the P3 samples (234, 51, 36): {:?}",
            &px[..3]
        );
    }

    /// `w`x`h` RGBA of 16-bit samples, native-endian, grey `level(x)` in column x.
    fn grey16(w: u32, h: u32, level: impl Fn(u32) -> u16) -> Vec<u8> {
        let row: Vec<u8> = (0..w)
            .flat_map(|x| [level(x), level(x), level(x), u16::MAX])
            .flat_map(u16::to_ne_bytes)
            .collect();
        row.repeat(h as usize)
    }

    #[test]
    fn an_hlg_picture_decodes_to_display_light_in_srgb() {
        // HLG 0.75 is scene light 0.265; the OOTF at 1000 nits (gamma 1.2) makes it 0.203 of the
        // peak, sRGB 124, where scene light read as display light gives 141.
        let px = grey16(64, 64, |_| 49152);
        let bytes = Picture {
            colour: Colour::Hdr(Tf::HLG, 1000.0),
            sixteen: true,
            ..Picture::new(64, 64, &[(&px, 1)])
        }
        .encode();
        let got = first_frame(&bytes, 1 << 20, NO_HURRY).unwrap().px;
        assert!(
            got[..3].iter().all(|&v| v.abs_diff(124) <= 3),
            "HLG 0.75 at 1000 nits is sRGB 124: {:?}",
            &got[..4]
        );
    }

    #[test]
    fn a_pq_picture_at_10000_nits_keeps_a_step_for_each_srgb_code() {
        // A ramp over the PQ signal: read as 8-bit samples its sRGB has gaps of up to four codes
        // in the highlights.
        let w = 1024;
        let px = grey16(w, 8, |x| (x * 64) as u16);
        let bytes = Picture {
            colour: Colour::Hdr(Tf::PQ, 10000.0),
            sixteen: true,
            ..Picture::new(w, 8, &[(&px, 1)])
        }
        .encode();
        let got = first_frame(&bytes, 1 << 24, NO_HURRY).unwrap().px;
        let codes: std::collections::BTreeSet<u8> =
            got[..w as usize * 4].iter().step_by(4).copied().collect();
        let top = codes.last().copied().unwrap_or(0);
        assert!(
            top >= 200 && codes.len() == usize::from(top) + 1,
            "every sRGB code up to {top}: {codes:?}"
        );
    }

    #[test]
    fn a_split_container_states_its_real_size_to_the_pixel_budget() {
        let code = encode(64, 64, &[&[7; 64 * 64 * 4]], false);
        let mut file = b"\0\0\0\x0cJXL \r\n\x87\n\0\0\0\x14ftypjxl \0\0\0\0jxl ".to_vec();
        for (index, part) in [(0u32, &code[..2]), (0x8000_0001, &code[2..])] {
            file.extend((part.len() as u32 + 12).to_be_bytes());
            file.extend(b"jxlp");
            file.extend(index.to_be_bytes());
            file.extend(part);
        }
        let whole = first_frame(&file, 64 * 64 * 4, NO_HURRY).unwrap();
        assert_eq!((whole.width, whole.height), (64, 64));
        assert_eq!(
            first_frame(&file, 64 * 64 * 4 - 1, NO_HURRY).err(),
            Some(Refused::Size),
            "refused from the size header, before any pixel buffer"
        );
    }

    /// `w`x`w` noise pictures made to hold more than their pixels while decoding, with each one's
    /// result at a pixel budget of `w`x`w` RGBA: None for decoded, else the refusal.
    fn adversarial(w: u32) -> Vec<(&'static str, Vec<u8>, Option<Refused>)> {
        let big = noise(w, w);
        let ec = noise(w * 3 / 4, w * 3 / 4);
        let refs = [(&big[..], 0); 4];
        let shown = [(&big[..], 1)];
        let refs = |n: usize| [&refs[..n], &shown].concat();
        let refs = |n: usize| {
            Picture {
                references: true,
                ..Picture::new(w, w, &refs(n))
            }
            .encode()
        };
        vec![
            ("lossless", encode(w, w, &[&big], false), None),
            (
                "lossy",
                Picture {
                    colour: Colour::P3,
                    ..Picture::new(w, w, &shown)
                }
                .encode(),
                None,
            ),
            (
                "four extra channels",
                Picture {
                    extra: 3,
                    ..Picture::new(w * 3 / 4, w * 3 / 4, &[(&ec, 1)])
                }
                .encode(),
                None,
            ),
            (
                "two-frame animation",
                encode(w, w, &[&big, &big], false),
                None,
            ),
            ("fifty layers", layered(w, w, 50), Some(Refused::Memory)),
            ("one reference frame", refs(1), Some(Refused::Memory)),
            ("three reference frames", refs(3), Some(Refused::Memory)),
        ]
    }

    /// The most `f` held at once on this thread.
    fn peak_of<T>(f: impl FnOnce() -> T) -> (T, usize) {
        let (out, spent) = heapcap::within(usize::MAX, f).unwrap();
        (out, spent.peak)
    }

    #[test]
    fn a_decode_holds_under_four_times_the_pixel_budget() {
        let budget = 1024 * 1024 * 4;
        for (name, bytes, refused) in adversarial(1024) {
            let (got, peak) = peak_of(|| first_frame(&bytes, budget, NO_HURRY));
            eprintln!("{name}: peak {:.2}x", peak as f64 / budget as f64);
            assert_eq!(got.err(), refused, "{name}");
            assert!(peak < 4 * budget as usize, "{name} held {peak}");
        }
    }

    #[test]
    fn layers_and_reference_frames_decode_given_the_heap_they_need() {
        let budget = 512 * 512 * 4;
        for (name, bytes, refused) in adversarial(512) {
            if refused.is_some() {
                assert!(first_frame(&bytes, 8 * budget, NO_HURRY).is_ok(), "{name}");
            }
        }
    }

    /// The refusal sweep's pictures: noise made to allocate in many places.
    fn sweep_case(n: usize) -> Option<(&'static str, Vec<u8>)> {
        let px = || noise(256, 256);
        let wide = || noise(16384, 4);
        Some(match n {
            0 => ("lossless", encode(256, 256, &[&px()], false)),
            1 => (
                "lossy",
                Picture {
                    colour: Colour::P3,
                    ..Picture::new(256, 256, &[(&px(), 1)])
                }
                .encode(),
            ),
            2 => {
                let px = px();
                let refs = [(&px[..], 0), (&px[..], 0), (&px[..], 1)];
                let bytes = Picture {
                    references: true,
                    ..Picture::new(256, 256, &refs)
                }
                .encode();
                ("reference frames", bytes)
            }
            3 => ("wide lossless", encode(16384, 4, &[&wide()], false)),
            4 => (
                "wide lossy",
                Picture {
                    colour: Colour::P3,
                    ..Picture::new(16384, 4, &[(&wide(), 1)])
                }
                .encode(),
            ),
            _ => return None,
        })
    }

    const SWEEP_BUDGET: u64 = 16 << 20;
    const SWEEP_TEST: &str =
        "jxl::tests::a_decode_refused_any_one_large_allocation_fails_as_memory_or_aborts";
    /// Set in the sweep's child processes to the case and the large allocation to refuse.
    const SWEEP_REFUSE: &str = "REPRISE_DECODE_SWEEP";

    fn hash(px: &[u8]) -> u64 {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut h = DefaultHasher::new();
        px.hash(&mut h);
        h.finish()
    }

    /// Each case decoded once for every large allocation it makes, that one refused, in a child
    /// process of its own: each gives the picture, [`Refused::Memory`], or an abort (where jxl-rs
    /// allocates infallibly), which reprise-decode's caller reads as too large.
    #[test]
    fn a_decode_refused_any_one_large_allocation_fails_as_memory_or_aborts() {
        use std::os::unix::process::ExitStatusExt;
        if let Some(asked) = std::env::var_os(SWEEP_REFUSE) {
            let asked = asked.into_string().unwrap();
            let (case, nth) = asked.split_once(' ').unwrap();
            let (_, bytes) = sweep_case(case.parse().unwrap()).unwrap();
            let refuse = |f| heapcap::within_refusing(usize::MAX, nth.parse().unwrap(), f);
            match refuse(|| first_frame(&bytes, SWEEP_BUDGET, NO_HURRY))
                .unwrap()
                .0
            {
                Ok(p) => println!("SWEPT picture {}", hash(&p.px)),
                Err(e) => println!("SWEPT {e:?}"),
            }
            return;
        }
        let (mut decoded, mut refused, mut aborted) = (0, 0, 0);
        for case in 0.. {
            let Some((name, bytes)) = sweep_case(case) else {
                break;
            };
            let (whole, all) =
                heapcap::within(usize::MAX, || first_frame(&bytes, SWEEP_BUDGET, NO_HURRY))
                    .unwrap();
            let whole = whole.unwrap_or_else(|e| panic!("{name}: {e:?}")).px;
            let picture = format!("SWEPT picture {}", hash(&whole));
            assert!(all.large > 0, "{name}");
            for nth in 0..all.large {
                let out = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", SWEEP_TEST, "--nocapture", "--test-threads=1"])
                    .env(SWEEP_REFUSE, format!("{case} {nth}"))
                    .stderr(std::process::Stdio::null())
                    .output()
                    .unwrap();
                let said = String::from_utf8_lossy(&out.stdout);
                let said = said.lines().find_map(|l| Some(&l[l.find("SWEPT")?..]));
                match (out.status.signal(), said) {
                    (Some(libc::SIGABRT), _) => aborted += 1,
                    (None, Some("SWEPT Memory")) => refused += 1,
                    (None, Some(p)) if p == picture => decoded += 1,
                    other => panic!("{name}: allocation {nth} refused: {other:?}"),
                }
            }
        }
        eprintln!("refusals: {decoded} decoded, {refused} Memory, {aborted} aborted");
    }

    #[test]
    fn a_grey_picture_decodes_with_and_without_alpha() {
        let grey: Vec<u8> = (0..64 * 64).map(|i| (i % 251) as u8).collect();
        let bytes = Picture {
            channels: 1,
            ..Picture::new(64, 64, &[(&grey, 1)])
        }
        .encode();
        let px = first_frame(&bytes, 1 << 20, NO_HURRY).unwrap().px;
        let want: Vec<u8> = grey.iter().flat_map(|&g| [g, g, g, 255]).collect();
        assert!(px == want, "grey g becomes (g, g, g, 255)");
        let with_alpha: Vec<u8> = grey.iter().flat_map(|&g| [g, 255 - g]).collect();
        let bytes = Picture {
            channels: 2,
            ..Picture::new(64, 64, &[(&with_alpha, 1)])
        }
        .encode();
        let px = first_frame(&bytes, 1 << 20, NO_HURRY).unwrap().px;
        let want: Vec<u8> = grey.iter().flat_map(|&g| [g, g, g, 255 - g]).collect();
        assert!(px == want, "grey g at alpha a becomes (g, g, g, a)");
    }

    #[test]
    fn an_animation_decodes_its_first_frame_without_the_rest() {
        let red = [200, 10, 10, 255].repeat(64 * 64);
        let noise: Vec<u8> = (0..64u32 * 64 * 4)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8 | 1)
            .collect();
        let bytes = encode(64, 64, &[&red, &noise], false);
        assert_eq!(first_frame(&bytes, 1 << 20, NO_HURRY).unwrap().px, red);
        let cut = &bytes[..bytes.len() / 2];
        assert_eq!(
            first_frame(cut, 1 << 20, NO_HURRY).unwrap().px,
            red,
            "the second frame is never read"
        );
    }

    #[test]
    fn a_file_of_many_full_canvas_layers_is_refused_at_its_deadline() {
        let px = 8 * 1024 * 1024 * 4;
        let few = layered(1024, 1024, 3);
        assert_eq!(
            first_frame(&few, px, NO_HURRY).unwrap().px[..4],
            [90, 90, 90, 255]
        );
        let many = layered(1024, 1024, 20_000);
        let limit = Duration::from_millis(300);
        let start = Instant::now();
        assert_eq!(first_frame(&many, px, limit).err(), Some(Refused::Time));
        let took = start.elapsed();
        assert!(
            took < limit + Duration::from_secs(1),
            "stops within a layer of the deadline, took {took:?}"
        );
    }

    #[test]
    fn a_lossless_picture_decodes_to_its_samples() {
        let px: Vec<u8> = (0..40 * 30)
            .flat_map(|i| [(i % 40) as u8 * 6, (i / 40) as u8 * 8, 90, 255])
            .collect();
        let p = first_frame(&encode(40, 30, &[&px], false), 1 << 20, NO_HURRY).unwrap();
        assert_eq!((p.width, p.height), (40, 30));
        assert!(p.px == px);
    }

    /// Premultiplied samples come out of jxl-rs divided by alpha.
    #[test]
    fn a_premultiplied_picture_decodes_unpremultiplied() {
        let bytes = encode(1, 1, &[&[100, 50, 24, 128]], true);
        let px = first_frame(&bytes, 1 << 20, NO_HURRY).unwrap().px;
        assert_eq!(px[3], 128);
        for (got, want) in px[..3].iter().zip([100u32, 50, 24]) {
            let multiplied = (u32::from(*got) * 128 + 127) / 255;
            assert!(multiplied.abs_diff(want) <= 1, "{px:?}");
        }
    }

    #[test]
    fn a_large_picture_decodes_within_the_time_limit() {
        let (w, h) = (4000, 3000);
        let px: Vec<u8> = (0..w * h * 4u32)
            .map(|i| (i / 4 % w / 16 + (i.wrapping_mul(2_654_435_761) >> 29)) as u8)
            .collect();
        let effort = [(Setting::Effort, 2)];
        let bytes = encode_with(w, h, &[(&px, 1)], false, &effort);
        let p = first_frame(&bytes, 96 << 20, NO_HURRY).unwrap();
        assert_eq!((p.width, p.height), (w, h));
    }
}
