//! A JPEG XL's first frame through libjxl, refused past a pixel budget or an allocation budget.

use std::alloc::Layout;
use std::ffi::{c_int, c_void};
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use jpegxl_sys::common::memory_manager::JxlMemoryManager;
use jpegxl_sys::common::types::{JxlBool, JxlDataType, JxlEndianness, JxlPixelFormat};
use jpegxl_sys::decode as sys;
use jpegxl_sys::decode::JxlDecoderStatus as Status;
use jpegxl_sys::metadata::codestream_header::JxlBasicInfo;

/// Why a JPEG XL gave no picture.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// Its basic info states more RGBA bytes than the pixel budget.
    Size,
    /// libjxl asked for more than the allocation budget.
    Memory,
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
/// more than `pixel_bytes` of RGBA or libjxl holds more than `alloc_limit` at once.
pub fn first_frame(bytes: &[u8], pixel_bytes: u64, alloc_limit: usize) -> Result<Rgba, Refused> {
    let budget = Budget {
        used: AtomicUsize::new(0),
        limit: alloc_limit,
        refused: AtomicBool::new(false),
    };
    let manager = JxlMemoryManager {
        opaque: (&raw const budget).cast_mut().cast(),
        alloc: Some(alloc),
        free: Some(free),
    };
    let mut px = Vec::new();
    // SAFETY: libjxl copies `manager`; `budget` outlives `dec`, declared after it.
    let dec = Decoder(unsafe { sys::JxlDecoderCreate(&manager) });
    let broken = || {
        Err(if budget.refused.load(Ordering::Relaxed) {
            Refused::Memory
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
    let events = Status::BasicInfo as c_int | Status::FullImage as c_int;
    let mut size = None;
    // SAFETY: `dec` is a live decoder; `bytes` outlives it and stays unchanged while it reads.
    unsafe {
        if sys::JxlDecoderSubscribeEvents(dec.0, events) != Status::Success
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

/// What libjxl holds through [`alloc`]; a request past `limit` gets null and sets `refused`.
struct Budget {
    used: AtomicUsize,
    limit: usize,
    refused: AtomicBool,
}

/// Each block starts with its total size, in a header that keeps malloc's 16-byte alignment.
const HEADER: usize = 16;

/// libjxl's allocator. It never panics: libjxl calls it across FFI and treats null as failure.
unsafe extern "C-unwind" fn alloc(opaque: *mut c_void, size: usize) -> *mut c_void {
    // SAFETY: `opaque` is the `Budget` `first_frame` keeps alive past the decoder.
    let budget = unsafe { &*opaque.cast::<Budget>() };
    let Some(total) = size.checked_add(HEADER) else {
        return std::ptr::null_mut();
    };
    let Ok(layout) = Layout::from_size_align(total, HEADER) else {
        return std::ptr::null_mut();
    };
    let room = budget
        .used
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |u| {
            u.checked_add(total).filter(|&n| n <= budget.limit)
        });
    if room.is_err() {
        budget.refused.store(true, Ordering::Relaxed);
        return std::ptr::null_mut();
    }
    // SAFETY: `layout` is non-zero sized.
    let block = unsafe { std::alloc::alloc(layout) };
    if block.is_null() {
        budget.used.fetch_sub(total, Ordering::Relaxed);
        return std::ptr::null_mut();
    }
    // SAFETY: `block` holds `total >= HEADER` bytes, aligned for usize.
    unsafe {
        block.cast::<usize>().write(total);
        block.add(HEADER).cast()
    }
}

unsafe extern "C-unwind" fn free(opaque: *mut c_void, address: *mut c_void) {
    if address.is_null() {
        return;
    }
    // SAFETY: `address` came from `alloc` with this `opaque`, so its header precedes it.
    unsafe {
        let budget = &*opaque.cast::<Budget>();
        let block = address.cast::<u8>().sub(HEADER);
        let total = block.cast::<usize>().read();
        std::alloc::dealloc(block, Layout::from_size_align_unchecked(total, HEADER));
        budget.used.fetch_sub(total, Ordering::Relaxed);
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use jpegxl_sys::color::color_encoding::JxlColorEncoding;
    use jpegxl_sys::encoder::encode as enc;
    use jpegxl_sys::encoder::encode::JxlEncoderStatus as EncStatus;
    use jpegxl_sys::metadata::codestream_header::JxlFrameHeader;

    /// A lossless JPEG XL of `frames` (`w`x`h` RGBA each), an animation when there are more
    /// than one; `premultiplied` stores the samples as already multiplied by alpha.
    pub(in crate::decode) fn encode(
        w: u32,
        h: u32,
        frames: &[&[u8]],
        premultiplied: bool,
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
            info.uses_original_profile = JxlBool::True;
            if frames.len() > 1 {
                info.have_animation = JxlBool::True;
                info.animation.tps_numerator = 10;
                info.animation.tps_denominator = 1;
            }
            ok(enc::JxlEncoderSetBasicInfo(e, &info));
            let mut srgb = MaybeUninit::<JxlColorEncoding>::uninit();
            enc::JxlColorEncodingSetToSRGB(srgb.as_mut_ptr(), JxlBool::False);
            ok(enc::JxlEncoderSetColorEncoding(e, srgb.as_ptr()));
            let settings = enc::JxlEncoderFrameSettingsCreate(e, std::ptr::null());
            ok(enc::JxlEncoderSetFrameLossless(settings, JxlBool::True));
            for frame in frames {
                if frames.len() > 1 {
                    let mut head = MaybeUninit::<JxlFrameHeader>::uninit();
                    enc::JxlEncoderInitFrameHeader(head.as_mut_ptr());
                    let mut head = head.assume_init();
                    head.duration = 1;
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
        let whole = first_frame(&file, 64 * 64 * 4, PLENTY).unwrap();
        assert_eq!((whole.width, whole.height), (64, 64));
        assert_eq!(
            first_frame(&file, 64 * 64 * 4 - 1, PLENTY).err(),
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
        assert_eq!(first_frame(&bytes, pixels, PLENTY).unwrap().px, noise);
        assert_eq!(
            first_frame(&bytes, pixels, 4 * pixels as usize).err(),
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
        assert_eq!(first_frame(&bytes, 1 << 20, PLENTY).unwrap().px, red);
        let cut = &bytes[..bytes.len() / 2];
        assert_eq!(
            first_frame(cut, 1 << 20, PLENTY).unwrap().px,
            red,
            "the second frame is never read"
        );
    }
}
