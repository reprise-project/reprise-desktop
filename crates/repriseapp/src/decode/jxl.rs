//! A JPEG XL's first frame through libjxl, refused past a pixel budget, an allocation budget or
//! a deadline.

use std::cell::Cell;
use std::ffi::{c_int, c_void};
use std::mem::MaybeUninit;
use std::time::{Duration, Instant};

use jpegxl_sys::color::color_encoding::JxlColorEncoding;
use jpegxl_sys::common::memory_manager::JxlMemoryManager;
use jpegxl_sys::common::types::{JxlBool, JxlDataType, JxlEndianness, JxlPixelFormat};
use jpegxl_sys::decode as sys;
use jpegxl_sys::decode::JxlDecoderStatus as Status;
use jpegxl_sys::encoder::encode::JxlColorEncodingSetToSRGB;
use jpegxl_sys::metadata::codestream_header::JxlBasicInfo;
use jpegxl_sys::threads::parallel_runner::{
    JXL_PARALLEL_RET_RUNNER_ERROR, JXL_PARALLEL_RET_SUCCESS, JxlParallelRetCode,
    JxlParallelRunFunction, JxlParallelRunInit,
};
use reprisefs::allocbudget::{self, Budget};

/// Why a JPEG XL gave no picture.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// Its basic info states more RGBA bytes than the pixel budget.
    Size,
    /// libjxl asked for more than the allocation budget.
    Memory,
    /// The decode ran past its time limit.
    Time,
    /// Not a JPEG XL libjxl reads, or cut short.
    Broken,
}

/// Unpremultiplied 8-bit RGBA, rows top to bottom, with the file's orientation applied.
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub px: Vec<u8>,
}

/// The first frame of `bytes` (an animation's first, composed), refused when the image states
/// more than `pixel_bytes` of RGBA, libjxl holds more than `alloc_limit` at once, or the decode
/// takes longer than `time_limit`.
pub fn first_frame(
    bytes: &[u8],
    pixel_bytes: u64,
    alloc_limit: usize,
    time_limit: Duration,
) -> Result<Rgba, Refused> {
    let clock = Clock {
        deadline: Instant::now() + time_limit,
        late: Cell::new(false),
    };
    let budget = Budget::new(alloc_limit);
    let manager = JxlMemoryManager {
        opaque: (&raw const budget).cast_mut().cast(),
        alloc: Some(allocbudget::alloc),
        free: Some(allocbudget::free),
    };
    let mut px = Vec::new();
    // SAFETY: libjxl copies `manager`; `budget` outlives `dec`, declared after it.
    let dec = Decoder(unsafe { sys::JxlDecoderCreate(&manager) });
    let broken = || {
        Err(if budget.refused() {
            Refused::Memory
        } else if clock.late.get() {
            Refused::Time
        } else {
            Refused::Broken
        })
    };
    if dec.0.is_null() {
        return broken();
    }
    let format = JxlPixelFormat {
        num_channels: 4,
        data_type: JxlDataType::Uint8,
        endianness: JxlEndianness::Native,
        align: 0,
    };
    let events =
        Status::BasicInfo as c_int | Status::ColorEncoding as c_int | Status::FullImage as c_int;
    let mut size = None;
    let mut ask_srgb = None;
    // SAFETY: `dec` is a live decoder; `bytes` and `clock` outlive it, `bytes` unchanged while
    // it reads.
    unsafe {
        if sys::JxlDecoderSetParallelRunner(dec.0, run, (&raw const clock).cast_mut().cast())
            != Status::Success
            || sys::JxlDecoderSubscribeEvents(dec.0, events) != Status::Success
            || sys::JxlDecoderSetUnpremultiplyAlpha(dec.0, JxlBool::True) != Status::Success
            || sys::JxlDecoderSetInput(dec.0, bytes.as_ptr(), bytes.len()) != Status::Success
        {
            return broken();
        }
        sys::JxlDecoderCloseInput(dec.0);
    }
    loop {
        // SAFETY: `dec` is live; `px`, once handed over, outlives every call that writes it.
        match unsafe { sys::JxlDecoderProcessInput(dec.0) } {
            Status::BasicInfo => {
                let mut info = MaybeUninit::<JxlBasicInfo>::uninit();
                // SAFETY: libjxl fills `info` when it answers Success.
                let info = unsafe {
                    if sys::JxlDecoderGetBasicInfo(dec.0, info.as_mut_ptr()) != Status::Success {
                        return broken();
                    }
                    info.assume_init()
                };
                if u64::from(info.xsize) * u64::from(info.ysize) * 4 > pixel_bytes {
                    return Err(Refused::Size);
                }
                // Orientations 5 to 8 turn the picture a quarter.
                size = Some(if info.orientation as u32 > 4 {
                    (info.ysize, info.xsize)
                } else {
                    (info.xsize, info.ysize)
                });
                ask_srgb = (info.uses_original_profile == JxlBool::False)
                    .then_some(info.num_color_channels == 1);
            }
            // With no CMS, libjxl hands over an XYB picture whose profile it cannot state as an
            // enum in linear sRGB unless sRGB is asked for.
            Status::ColorEncoding => {
                if let Some(grey) = ask_srgb {
                    let mut srgb = MaybeUninit::<JxlColorEncoding>::uninit();
                    // SAFETY: the call fills `srgb`; `dec` is live and copies it.
                    let ok = unsafe {
                        JxlColorEncodingSetToSRGB(srgb.as_mut_ptr(), grey.into());
                        sys::JxlDecoderSetPreferredColorProfile(dec.0, srgb.as_ptr())
                    };
                    if ok != Status::Success {
                        return broken();
                    }
                }
            }
            Status::NeedImageOutBuffer => {
                let Some((w, h)) = size else {
                    return broken();
                };
                let mut need = 0usize;
                // SAFETY: `format` and `need` are live for the call.
                let ok = unsafe { sys::JxlDecoderImageOutBufferSize(dec.0, &format, &mut need) };
                if ok != Status::Success || need as u64 != u64::from(w) * u64::from(h) * 4 {
                    return broken();
                }
                px = vec![0; need];
                // SAFETY: `px` holds `need` bytes and is not touched again until FullImage.
                let ok = unsafe {
                    sys::JxlDecoderSetImageOutBuffer(dec.0, &format, px.as_mut_ptr().cast(), need)
                };
                if ok != Status::Success {
                    return broken();
                }
            }
            Status::FullImage => {
                let (width, height) = size.filter(|_| !px.is_empty()).ok_or(Refused::Broken)?;
                return Ok(Rgba {
                    width,
                    height,
                    px: std::mem::take(&mut px),
                });
            }
            _ => return broken(),
        }
    }
}

/// Destroys the decoder it holds.
struct Decoder(*mut sys::JxlDecoder);

impl Drop for Decoder {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: made by JxlDecoderCreate and destroyed once.
            unsafe { sys::JxlDecoderDestroy(self.0) }
        }
    }
}

/// When [`run`] stops starting libjxl's jobs; `late` is set once it has.
struct Clock {
    deadline: Instant,
    late: Cell<bool>,
}

/// libjxl's runner: each job inline on the calling thread, none started past the deadline. libjxl
/// decodes every group of every frame through it, so a file of many layers stops there too.
unsafe extern "C-unwind" fn run(
    opaque: *mut c_void,
    jxl: *mut c_void,
    init: JxlParallelRunInit,
    func: JxlParallelRunFunction,
    start: u32,
    end: u32,
) -> JxlParallelRetCode {
    // SAFETY: `opaque` is the `Clock` `first_frame` keeps alive past the decoder.
    let clock = unsafe { &*opaque.cast::<Clock>() };
    // SAFETY: libjxl hands `init` and `func` the `jxl` it passed, on the one thread declared.
    let ret = unsafe { init(jxl, 1) };
    if ret != JXL_PARALLEL_RET_SUCCESS {
        return ret;
    }
    let deadline = clock.deadline;
    for value in start..end {
        // Nothing may unwind into libjxl; a clock that fails counts as late.
        if std::panic::catch_unwind(|| Instant::now() >= deadline).unwrap_or(true) {
            clock.late.set(true);
            return JXL_PARALLEL_RET_RUNNER_ERROR;
        }
        // SAFETY: as for `init`; thread id 0 is below the one thread.
        unsafe { func(jxl, value, 0) };
    }
    JXL_PARALLEL_RET_SUCCESS
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use jpegxl_sys::encoder::encode as enc;
    use jpegxl_sys::encoder::encode::JxlEncoderStatus as EncStatus;
    use jpegxl_sys::metadata::codestream_header::JxlFrameHeader;

    /// A lossless JPEG XL of `frames` (`w`x`h` RGBA each), an animation of 1-tick frames when
    /// there are more than one; `premultiplied` stores the samples as already multiplied by alpha.
    pub(in crate::decode) fn encode(
        w: u32,
        h: u32,
        frames: &[&[u8]],
        premultiplied: bool,
    ) -> Vec<u8> {
        let timed: Vec<_> = frames.iter().map(|f| (*f, 1)).collect();
        encode_with(w, h, &timed, premultiplied, &[])
    }

    /// [`encode`] with each frame's duration in ticks (0 for a layer composed into the next) and
    /// encoder `options`.
    pub(in crate::decode) fn encode_with(
        w: u32,
        h: u32,
        frames: &[(&[u8], u32)],
        premultiplied: bool,
        options: &[(enc::JxlEncoderFrameSettingId, i64)],
    ) -> Vec<u8> {
        encode_tagged(w, h, frames, premultiplied, options, None)
    }

    /// [`encode_with`], lossy (XYB) and tagged with `icc` when given.
    fn encode_tagged(
        w: u32,
        h: u32,
        frames: &[(&[u8], u32)],
        premultiplied: bool,
        options: &[(enc::JxlEncoderFrameSettingId, i64)],
        icc: Option<&[u8]>,
    ) -> Vec<u8> {
        let ok = |s: EncStatus| assert_eq!(s, EncStatus::Success);
        let format = JxlPixelFormat {
            num_channels: 4,
            data_type: JxlDataType::Uint8,
            endianness: JxlEndianness::Native,
            align: 0,
        };
        // SAFETY: every pointer handed over is live for the call; the encoder is destroyed last.
        unsafe {
            let e = enc::JxlEncoderCreate(std::ptr::null());
            let mut info = MaybeUninit::<JxlBasicInfo>::uninit();
            enc::JxlEncoderInitBasicInfo(info.as_mut_ptr());
            let mut info = info.assume_init();
            (info.xsize, info.ysize) = (w, h);
            info.bits_per_sample = 8;
            info.num_extra_channels = 1;
            info.alpha_bits = 8;
            info.alpha_premultiplied = if premultiplied {
                JxlBool::True
            } else {
                JxlBool::False
            };
            info.uses_original_profile = icc.is_none().into();
            if frames.len() > 1 {
                info.have_animation = JxlBool::True;
                info.animation.tps_numerator = 10;
                info.animation.tps_denominator = 1;
            }
            ok(enc::JxlEncoderSetBasicInfo(e, &info));
            let settings = enc::JxlEncoderFrameSettingsCreate(e, std::ptr::null());
            if let Some(icc) = icc {
                ok(enc::JxlEncoderSetICCProfile(e, icc.as_ptr(), icc.len()));
            } else {
                let mut srgb = MaybeUninit::<JxlColorEncoding>::uninit();
                enc::JxlColorEncodingSetToSRGB(srgb.as_mut_ptr(), JxlBool::False);
                ok(enc::JxlEncoderSetColorEncoding(e, srgb.as_ptr()));
                ok(enc::JxlEncoderSetFrameLossless(settings, JxlBool::True));
            }
            for &(id, value) in options {
                ok(enc::JxlEncoderFrameSettingsSetOption(settings, id, value));
            }
            for &(frame, duration) in frames {
                if frames.len() > 1 {
                    let mut head = MaybeUninit::<JxlFrameHeader>::uninit();
                    enc::JxlEncoderInitFrameHeader(head.as_mut_ptr());
                    let mut head = head.assume_init();
                    head.duration = duration;
                    ok(enc::JxlEncoderSetFrameHeader(settings, &head));
                }
                ok(enc::JxlEncoderAddImageFrame(
                    settings,
                    &format,
                    frame.as_ptr().cast(),
                    frame.len(),
                ));
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

    /// The JPEG XL test pictures' allocation budget, far past what any of them needs.
    const PLENTY: usize = 64 << 20;
    /// Their time limit, far past what any of them takes.
    const NO_HURRY: Duration = Duration::from_secs(60);

    /// A `w`x`h` codestream of `layers` zero-duration grey layers over the whole canvas, then one
    /// shown frame: a few bytes a layer, each composed over every pixel.
    fn layered(w: u32, h: u32, layers: usize) -> Vec<u8> {
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
    fn tabled_srgb_icc() -> Vec<u8> {
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

    #[test]
    fn a_lossy_picture_tagged_with_an_icc_profile_decodes_to_srgb() {
        let grey = [128, 128, 128, 255].repeat(64 * 64);
        let icc = tabled_srgb_icc();
        let bytes = encode_tagged(64, 64, &[(&grey, 1)], false, &[], Some(&icc));
        let px = first_frame(&bytes, 1 << 20, PLENTY, NO_HURRY).unwrap().px;
        assert!(
            px[..3].iter().all(|&v| v.abs_diff(128) <= 4),
            "grey 128 stays near 128, not its linear value (about 56): {:?}",
            &px[..3]
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
        assert_eq!(reprisefs::imagesize::dimensions(&file), Some((64, 64)));
        let whole = first_frame(&file, 64 * 64 * 4, PLENTY, NO_HURRY).unwrap();
        assert_eq!((whole.width, whole.height), (64, 64));
        assert_eq!(
            first_frame(&file, 64 * 64 * 4 - 1, PLENTY, NO_HURRY).err(),
            Some(Refused::Size),
            "refused once libjxl states the size, before any pixel buffer"
        );
    }

    #[test]
    fn a_decode_needing_more_than_the_allocation_budget_is_refused() {
        let noise: Vec<u8> = (0..256u32 * 256 * 4)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        let bytes = encode(256, 256, &[&noise], false);
        let pixels = 256 * 256 * 4;
        assert_eq!(
            first_frame(&bytes, pixels, PLENTY, NO_HURRY).unwrap().px,
            noise
        );
        assert_eq!(
            first_frame(&bytes, pixels, 4 * pixels as usize, NO_HURRY).err(),
            Some(Refused::Memory),
            "the header passes, the decode needs more"
        );
    }

    #[test]
    fn an_animation_decodes_its_first_frame_without_the_rest() {
        let red = [200, 10, 10, 255].repeat(64 * 64);
        let noise: Vec<u8> = (0..64u32 * 64 * 4)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8 | 1)
            .collect();
        let bytes = encode(64, 64, &[&red, &noise], false);
        assert_eq!(
            first_frame(&bytes, 1 << 20, PLENTY, NO_HURRY).unwrap().px,
            red
        );
        let cut = &bytes[..bytes.len() / 2];
        assert_eq!(
            first_frame(cut, 1 << 20, PLENTY, NO_HURRY).unwrap().px,
            red,
            "the second frame is never read"
        );
    }

    #[test]
    fn a_file_of_many_full_canvas_layers_is_refused_at_its_deadline() {
        let (px, alloc) = (1024 * 1024 * 4, crate::decode::JXL_ALLOC_LIMIT);
        let few = layered(1024, 1024, 3);
        assert_eq!(
            first_frame(&few, px, alloc, NO_HURRY).unwrap().px[..4],
            [90, 90, 90, 255]
        );
        let many = layered(1024, 1024, 20_000);
        let limit = Duration::from_millis(300);
        let start = Instant::now();
        assert_eq!(
            first_frame(&many, px, alloc, limit).err(),
            Some(Refused::Time)
        );
        let took = start.elapsed();
        assert!(
            took < limit + Duration::from_secs(1),
            "stops within a layer of the deadline, took {took:?}"
        );
    }
}
