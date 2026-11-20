//! Test pictures encoded by libjxl, shared by the unit tests and the tests that run the binary.
#![allow(dead_code, unused_imports)]

pub use jpegxl_sys::color::color_encoding::JxlTransferFunction as Tf;
use jpegxl_sys::color::color_encoding::{JxlColorEncoding as Encoding, JxlPrimaries as Prim};
use jpegxl_sys::common::types::{JxlBool, JxlDataType, JxlEndianness, JxlPixelFormat as Format};
use jpegxl_sys::encoder::encode as enc;
pub use jpegxl_sys::encoder::encode::JxlEncoderFrameSettingId as Setting;
use jpegxl_sys::encoder::encode::JxlEncoderStatus as EncStatus;
use jpegxl_sys::metadata::codestream_header::{
    JxlBasicInfo, JxlExtraChannelInfo, JxlExtraChannelType, JxlFrameHeader,
};
use std::mem::MaybeUninit;

/// A lossless JPEG XL of `frames` (`w`x`h` RGBA each), an animation of 1-tick frames when
/// there are more than one; `premultiplied` stores the samples as already multiplied by alpha.
pub fn encode(w: u32, h: u32, frames: &[&[u8]], premultiplied: bool) -> Vec<u8> {
    let timed: Vec<_> = frames.iter().map(|f| (*f, 1)).collect();
    encode_with(w, h, &timed, premultiplied, &[])
}

/// [`encode`] with each frame's duration in ticks (0 for a layer composed into the next) and
/// encoder `options`.
pub fn encode_with(
    w: u32,
    h: u32,
    frames: &[(&[u8], u32)],
    premultiplied: bool,
    options: &[(enc::JxlEncoderFrameSettingId, i64)],
) -> Vec<u8> {
    Picture {
        premultiplied,
        options,
        ..Picture::new(w, h, frames)
    }
    .encode()
}

/// How a test picture's colour is stored.
#[derive(Clone, Copy)]
pub enum Colour<'a> {
    Lossless,
    /// Lossy (XYB), tagged with this ICC profile.
    Icc(&'a [u8]),
    /// Near-lossless XYB, Display P3 stated as an enum.
    P3,
    /// Near-lossless XYB, BT.2100 primaries with this transfer function, for a display of this
    /// many nits.
    Hdr(Tf, f32),
}

/// A test picture: `frames` of `w`x`h` pixels with their durations, encoded by libjxl.
pub struct Picture<'a> {
    pub w: u32,
    pub h: u32,
    pub frames: &'a [(&'a [u8], u32)],
    /// Samples a pixel: 4 for RGBA, 2 for grey and alpha, 1 for grey.
    pub channels: u32,
    pub premultiplied: bool,
    pub options: &'a [(enc::JxlEncoderFrameSettingId, i64)],
    pub colour: Colour<'a>,
    /// Extra channels after alpha, each a copy of it.
    pub extra: u32,
    /// Each zero-duration frame is kept in reference slot `index % 3` (libjxl keeps slot 3).
    pub references: bool,
    /// The frames hold 16-bit samples, native-endian, rather than 8-bit ones.
    pub sixteen: bool,
}

impl<'a> Picture<'a> {
    pub fn new(w: u32, h: u32, frames: &'a [(&'a [u8], u32)]) -> Picture<'a> {
        Picture {
            w,
            h,
            frames,
            channels: 4,
            premultiplied: false,
            options: &[],
            colour: Colour::Lossless,
            extra: 0,
            references: false,
            sixteen: false,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let ok = |s: EncStatus| assert_eq!(s, EncStatus::Success);
        let format = Format {
            num_channels: self.channels,
            data_type: if self.sixteen {
                JxlDataType::Uint16
            } else {
                JxlDataType::Uint8
            },
            endianness: JxlEndianness::Native,
            align: 0,
        };
        let grey = Format {
            num_channels: 1,
            ..format
        };
        let lossless = matches!(self.colour, Colour::Lossless);
        let animated = self.frames.len() > 1;
        // SAFETY: every pointer handed over is live for the call; the encoder is destroyed
        // last.
        unsafe {
            let e = enc::JxlEncoderCreate(std::ptr::null());
            let mut info = MaybeUninit::<JxlBasicInfo>::uninit();
            enc::JxlEncoderInitBasicInfo(info.as_mut_ptr());
            let mut info = info.assume_init();
            (info.xsize, info.ysize) = (self.w, self.h);
            info.bits_per_sample = if self.sixteen { 16 } else { 8 };
            if let Colour::Hdr(_, nits) = self.colour {
                info.intensity_target = nits;
            }
            let alpha = self.channels.is_multiple_of(2);
            info.num_color_channels = if self.channels < 3 { 1 } else { 3 };
            info.num_extra_channels = u32::from(alpha) + self.extra;
            info.alpha_bits = if alpha { 8 } else { 0 };
            info.alpha_premultiplied = self.premultiplied.into();
            info.uses_original_profile = lossless.into();
            if animated {
                info.have_animation = JxlBool::True;
                info.animation.tps_numerator = 10;
                info.animation.tps_denominator = 1;
            }
            ok(enc::JxlEncoderSetBasicInfo(e, &info));
            for index in 1..=self.extra as usize {
                let mut ec = MaybeUninit::<JxlExtraChannelInfo>::uninit();
                enc::JxlEncoderInitExtraChannelInfo(
                    JxlExtraChannelType::SelectionMask,
                    ec.as_mut_ptr(),
                );
                let mut ec = ec.assume_init();
                ec.bits_per_sample = 8;
                ok(enc::JxlEncoderSetExtraChannelInfo(e, index, &ec));
            }
            let settings = enc::JxlEncoderFrameSettingsCreate(e, std::ptr::null());
            let mut srgb = MaybeUninit::<Encoding>::uninit();
            let is_grey = JxlBool::from(self.channels < 3);
            enc::JxlColorEncodingSetToSRGB(srgb.as_mut_ptr(), is_grey);
            let mut srgb = srgb.assume_init();
            match self.colour {
                Colour::Lossless => {
                    ok(enc::JxlEncoderSetColorEncoding(e, &srgb));
                    ok(enc::JxlEncoderSetFrameLossless(settings, JxlBool::True));
                }
                Colour::Icc(icc) => {
                    ok(enc::JxlEncoderSetICCProfile(e, icc.as_ptr(), icc.len()));
                }
                Colour::P3 => {
                    srgb.primaries = Prim::P3;
                    ok(enc::JxlEncoderSetColorEncoding(e, &srgb));
                    ok(enc::JxlEncoderSetFrameDistance(settings, 0.1));
                }
                Colour::Hdr(tf, _) => {
                    srgb.primaries = Prim::Rec2100;
                    srgb.transfer_function = tf;
                    ok(enc::JxlEncoderSetColorEncoding(e, &srgb));
                    ok(enc::JxlEncoderSetFrameDistance(settings, 0.1));
                }
            }
            for &(id, value) in self.options {
                ok(enc::JxlEncoderFrameSettingsSetOption(settings, id, value));
            }
            let mut layer = 0;
            for &(frame, duration) in self.frames {
                if animated {
                    let mut head = MaybeUninit::<JxlFrameHeader>::uninit();
                    enc::JxlEncoderInitFrameHeader(head.as_mut_ptr());
                    let mut head = head.assume_init();
                    head.duration = duration;
                    if self.references && duration == 0 {
                        head.layer_info.save_as_reference = layer % 3;
                        layer += 1;
                    }
                    ok(enc::JxlEncoderSetFrameHeader(settings, &head));
                }
                ok(enc::JxlEncoderAddImageFrame(
                    settings,
                    &format,
                    frame.as_ptr().cast(),
                    frame.len(),
                ));
                let alpha: Vec<u8> = frame.iter().skip(3).step_by(4).copied().collect();
                for index in 1..=self.extra {
                    ok(enc::JxlEncoderSetExtraChannelBuffer(
                        settings,
                        &grey,
                        alpha.as_ptr().cast(),
                        alpha.len(),
                        index,
                    ));
                }
            }
            enc::JxlEncoderCloseInput(e);
            let mut out = vec![0u8; 1 << 16];
            let mut done = 0;
            loop {
                let mut next = out.as_mut_ptr().add(done);
                let mut avail = out.len() - done;
                let s = enc::JxlEncoderProcessOutput(e, &mut next, &mut avail);
                done = out.len() - avail;
                if s != EncStatus::NeedMoreOutput {
                    ok(s);
                    break;
                }
                out.resize(out.len() * 2, 0);
            }
            enc::JxlEncoderDestroy(e);
            out.truncate(done);
            out
        }
    }
}

/// A `w`x`h` codestream of `layers` zero-duration grey layers over the whole canvas, then one
/// shown frame: a few bytes a layer, each composed over every pixel.
pub fn layered(w: u32, h: u32, layers: usize) -> Vec<u8> {
    use enc::JxlEncoderFrameSettingId::{Effort, ModularGroupSize};
    let grey = [90, 90, 90, 255].repeat((w * h) as usize);
    let small = [(Effort, 2), (ModularGroupSize, 3)];
    let one = encode_with(w, h, &[(&grey, 0), (&grey, 1)], false, &small);
    let two = encode_with(w, h, &[(&grey, 0), (&grey, 0), (&grey, 1)], false, &small);
    assert!(one.starts_with(&[0xff, 0x0a]), "a bare codestream");
    let layer = two.len() - one.len();
    let at = (0..=one.len())
        .find(|&i| two[..i] == one[..i] && two[i + layer..] == one[i..])
        .unwrap();
    let mut out = one[..at].to_vec();
    for _ in 0..layers - 1 {
        out.extend(&two[at..at + layer]);
    }
    out.extend(&one[at..]);
    out
}

/// An ICC v2 RGB profile with sRGB's primaries and its curve as an 8-entry table, which a
/// JPEG XL can only carry as ICC bytes.
pub fn tabled_srgb_icc() -> Vec<u8> {
    let s15 = |v: f64| ((v * 65536.0).round() as i32).to_be_bytes();
    let xyz = |x, y, z| [&b"XYZ \0\0\0\0"[..], &s15(x), &s15(y), &s15(z)].concat();
    let curve = (0..8).flat_map(|i| {
        let v = f64::from(i) / 7.0;
        let linear = if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        };
        ((linear * 65535.0).round() as u16).to_be_bytes()
    });
    let trc = [&b"curv\0\0\0\0\0\0\0\x08"[..], &curve.collect::<Vec<u8>>()].concat();
    let tags = [
        (b"wtpt", xyz(0.9642, 1.0, 0.8249)),
        (b"rXYZ", xyz(0.4361, 0.2225, 0.0139)),
        (b"gXYZ", xyz(0.3851, 0.7169, 0.0971)),
        (b"bXYZ", xyz(0.1431, 0.0606, 0.7141)),
        (b"rTRC", trc.clone()),
        (b"gTRC", trc.clone()),
        (b"bTRC", trc),
    ];
    let mut table = (tags.len() as u32).to_be_bytes().to_vec();
    let mut data = Vec::new();
    let start = 128 + 4 + 12 * tags.len();
    for (sig, body) in &tags {
        data.resize(data.len().next_multiple_of(4), 0);
        table.extend(*sig);
        table.extend(((start + data.len()) as u32).to_be_bytes());
        table.extend((body.len() as u32).to_be_bytes());
        data.extend(body);
    }
    let mut icc = ((128 + table.len() + data.len()) as u32)
        .to_be_bytes()
        .to_vec();
    icc.extend(b"none\x02\x10\0\0mntrRGB XYZ ");
    icc.resize(36, 0);
    icc.extend(b"acsp");
    icc.resize(68, 0);
    icc.extend([s15(0.9642), s15(1.0), s15(0.8249)].concat());
    icc.resize(128, 0);
    icc.extend(table);
    icc.extend(data);
    icc
}

pub fn noise(w: u32, h: u32) -> Vec<u8> {
    (0..w * h * 4)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect()
}
