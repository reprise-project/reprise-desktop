//! `libreprise_encode_jxl.so`: JPEG XL encoding through libjxl, behind one C entry
//! ([`reprise_encode_jxl_v1`]) that reprise-encode looks up after loading it.

mod allocbudget;

use std::ffi::c_void;

use jpegxl_rs::encode::EncoderSpeed;
use jpegxl_rs::memory::MemoryManager;
use jpegxl_sys::common::memory_manager::{JpegxlAllocFunc, JpegxlFreeFunc};

use allocbudget::Budget;

/// `[screenshot] jxl_lossless`, `jxl_quality` and `jxl_effort`.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub lossless: bool,
    pub quality: u8,
    pub effort: u8,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Failed {
    /// libjxl asked for more than the budget.
    Memory,
    Codec(String),
}

/// The return values of [`reprise_encode_jxl_v1`], which are reprise-encode's reply bytes.
pub const FILE: u8 = 0;
pub const MEMORY: u8 = 1;
pub const CODEC: u8 = 2;

/// Encodes `w` by `h` 8-bit RGB pixels at `rgb` with at most `budget` bytes of libjxl memory on
/// `threads` workers. Returns [`FILE`] after passing the file to `sink`, [`MEMORY`] past the
/// budget, or [`CODEC`] after passing libjxl's error text to `sink`. `sink` is called at most
/// once, with `ctx`.
///
/// # Safety
/// `rgb` points to `w * h * 3` readable bytes; `sink` is safe to call with `ctx` and a live
/// slice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reprise_encode_jxl_v1(
    rgb: *const u8,
    w: u32,
    h: u32,
    lossless: bool,
    quality: u8,
    effort: u8,
    budget: u64,
    threads: u32,
    sink: unsafe extern "C" fn(ctx: *mut c_void, bytes: *const u8, len: usize),
    ctx: *mut c_void,
) -> u8 {
    let Some(len) = (w as usize * h as usize).checked_mul(3) else {
        return MEMORY;
    };
    // SAFETY: the caller's.
    let rgb = unsafe { std::slice::from_raw_parts(rgb, len) };
    let s = Settings {
        lossless,
        quality,
        effort,
    };
    let budget = usize::try_from(budget).unwrap_or(usize::MAX);
    let encoded = std::panic::catch_unwind(|| encode(rgb, w, h, s, budget, threads as usize))
        .unwrap_or_else(|_| Err(Failed::Codec("the JPEG XL encoder panicked".into())));
    let (code, bytes) = match &encoded {
        Ok(file) => (FILE, file.as_slice()),
        Err(Failed::Memory) => return MEMORY,
        Err(Failed::Codec(why)) => (CODEC, why.as_bytes()),
    };
    // SAFETY: the caller's; bytes is live for the call.
    unsafe { sink(ctx, bytes.as_ptr(), bytes.len()) };
    code
}

/// `[screenshot] jxl_effort` 1..=9 in order.
const EFFORT: [EncoderSpeed; 9] = [
    EncoderSpeed::Lightning,
    EncoderSpeed::Thunder,
    EncoderSpeed::Falcon,
    EncoderSpeed::Cheetah,
    EncoderSpeed::Hare,
    EncoderSpeed::Wombat,
    EncoderSpeed::Squirrel,
    EncoderSpeed::Kitten,
    EncoderSpeed::Tortoise,
];

/// libjxl's allocations counted against a [`Budget`].
#[repr(transparent)]
struct Capped(Budget);

impl MemoryManager for Capped {
    fn alloc(&self) -> JpegxlAllocFunc {
        allocbudget::alloc
    }

    fn free(&self) -> JpegxlFreeFunc {
        allocbudget::free
    }
}

/// `rgb`, `w` by `h`, as a JPEG XL, with libjxl holding at most `budget` bytes.
pub fn encode(
    rgb: &[u8],
    w: u32,
    h: u32,
    s: Settings,
    budget: usize,
    threads: usize,
) -> Result<Vec<u8>, Failed> {
    let capped = Capped(Budget::new(budget));
    encode_through(rgb, w, h, s, threads, &capped, || capped.0.refused()).map_err(|e| match capped
        .0
        .refused()
    {
        true => Failed::Memory,
        false => Failed::Codec(e.to_string()),
    })
}

/// The encode with libjxl allocating through `memory`, which says whether it `refused` a request.
fn encode_through(
    rgb: &[u8],
    w: u32,
    h: u32,
    s: Settings,
    threads: usize,
    memory: &dyn MemoryManager,
    refused: impl Fn() -> bool,
) -> Result<Vec<u8>, jpegxl_rs::EncodeError> {
    let runner = jpegxl_rs::ThreadsRunner::new(Some(memory), Some(threads))
        .ok_or(jpegxl_rs::EncodeError::CannotCreateEncoder)?;
    let speed = EFFORT[usize::from(s.effort.clamp(1, 9)) - 1];
    let builder = jpegxl_rs::encoder_builder()
        .memory_manager(memory)
        .parallel_runner(&runner)
        .speed(speed);
    // libjxl refuses a lossless frame unless the original colour profile is kept.
    let mut enc = if s.lossless {
        builder.lossless(true).uses_original_profile(true).build()?
    } else {
        // Quality 100 is distance 0, which libjxl's lossy (XYB) path does not take.
        builder
            .jpeg_quality(f32::from(s.quality.clamp(1, 99)))
            .build()?
    };
    // jpegxl-rs keeps the frame settings libjxl could not allocate as null and passes them on.
    if refused() {
        return Err(jpegxl_rs::EncodeError::CannotCreateEncoder);
    }
    let coded: jpegxl_rs::encode::EncoderResult<u8> = enc.encode(rgb, w, h)?;
    Ok(coded.data)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn test_rgb(w: u32, h: u32) -> Vec<u8> {
        (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                [(x * 4) as u8, (y * 5) as u8, (((x ^ y) * 3) & 0xff) as u8]
            })
            .collect()
    }

    const LOSSLESS: Settings = Settings {
        lossless: true,
        quality: 90,
        effort: 7,
    };

    fn lossy(quality: u8) -> Settings {
        Settings {
            lossless: false,
            quality,
            effort: 1,
        }
    }

    #[test]
    fn a_lossless_jxl_keeps_every_pixel() {
        let rgb = test_rgb(64, 48);
        let bytes = encode(&rgb, 64, 48, LOSSLESS, 256 << 20, 2).unwrap();
        let dec = jpegxl_rs::decoder_builder()
            .pixel_format(jpegxl_rs::decode::PixelFormat {
                num_channels: 3,
                ..Default::default()
            })
            .build()
            .unwrap();
        let (meta, px) = dec.decode_with::<u8>(&bytes).unwrap();
        assert_eq!((meta.width, meta.height), (64, 48));
        assert_eq!(px, rgb);
    }

    #[test]
    fn a_lossy_jxl_at_quality_100_saves_at_99() {
        let rgb = test_rgb(64, 48);
        let at = |q| encode(&rgb, 64, 48, lossy(q), 256 << 20, 2).unwrap();
        assert_eq!(at(100), at(99));
        assert_ne!(at(99), at(90));
    }

    #[test]
    fn a_jxl_encode_past_its_memory_budget_fails_as_memory() {
        let rgb = test_rgb(256, 256);
        assert!(encode(&rgb, 256, 256, lossy(90), 256 << 20, 2).is_ok());
        assert_eq!(
            encode(&rgb, 256, 256, lossy(90), 1 << 20, 2),
            Err(Failed::Memory)
        );
    }

    #[test]
    fn the_c_entry_passes_the_file_or_the_error_to_its_sink() {
        unsafe extern "C" fn collect(ctx: *mut c_void, bytes: *const u8, len: usize) {
            // SAFETY: ctx is the Vec below and the entry passes `len` live bytes.
            unsafe {
                (*ctx.cast::<Vec<u8>>()).extend_from_slice(std::slice::from_raw_parts(bytes, len))
            };
        }
        let rgb = test_rgb(64, 48);
        let call = |budget| {
            let mut out: Vec<u8> = Vec::new();
            // SAFETY: rgb holds 64 * 48 * 3 bytes and `out` outlives the call.
            let code = unsafe {
                reprise_encode_jxl_v1(
                    rgb.as_ptr(),
                    64,
                    48,
                    false,
                    90,
                    1,
                    budget,
                    2,
                    collect,
                    (&raw mut out).cast(),
                )
            };
            (code, out)
        };
        let (code, file) = call(256 << 20);
        assert_eq!(code, FILE);
        assert!(file.starts_with(&[0xff, 0x0a]));
        assert_eq!(call(4096), (MEMORY, Vec::new()));
    }

    /// Refuses allocation number `nth` (from 0) and hands every other to malloc.
    struct FailAt {
        nth: usize,
        seen: AtomicUsize,
    }

    impl FailAt {
        fn new(nth: usize) -> FailAt {
            FailAt {
                nth,
                seen: AtomicUsize::new(0),
            }
        }

        fn refused(&self) -> bool {
            self.seen.load(Ordering::Relaxed) > self.nth
        }
    }

    unsafe extern "C-unwind" fn fail_at_alloc(opaque: *mut c_void, size: usize) -> *mut c_void {
        // SAFETY: libjxl passes back the `FailAt` its manager points to.
        let f = unsafe { &*opaque.cast::<FailAt>() };
        if f.seen.fetch_add(1, Ordering::Relaxed) == f.nth {
            return std::ptr::null_mut();
        }
        // SAFETY: plain malloc; libjxl frees it through `fail_at_free`.
        unsafe { libc::malloc(size) }
    }

    unsafe extern "C-unwind" fn fail_at_free(_: *mut c_void, address: *mut c_void) {
        // SAFETY: `address` is null or came from `fail_at_alloc`'s malloc.
        unsafe { libc::free(address) }
    }

    impl MemoryManager for FailAt {
        fn alloc(&self) -> JpegxlAllocFunc {
            fail_at_alloc
        }

        fn free(&self) -> JpegxlFreeFunc {
            fail_at_free
        }
    }

    /// The encodes the refusal sweeps run: lossy and lossless at each of `efforts`, on a picture
    /// two groups wide.
    fn sweep_shots(efforts: &[u8]) -> impl Iterator<Item = Settings> {
        efforts.iter().flat_map(|&effort| {
            [false, true].map(|lossless| Settings {
                lossless,
                quality: 90,
                effort,
            })
        })
    }

    const SWEEP: (u32, u32) = (272, 40);
    /// The sweeps' encoder threads, fixed: the memory an encode needs grows with them.
    const SWEEP_THREADS: usize = 2;

    #[test]
    fn a_jxl_encode_at_effort_3_refused_any_one_allocation_fails_without_aborting() {
        refuse_each_allocation(&[3]);
    }

    #[test]
    #[ignore = "about 40 s on two cores: lossy effort 9 makes 755 allocations, each a fresh encode"]
    fn a_jxl_encode_refused_any_one_allocation_fails_without_aborting() {
        refuse_each_allocation(&[3, 9]);
    }

    fn refuse_each_allocation(efforts: &[u8]) {
        let (w, h) = SWEEP;
        let rgb = test_rgb(w, h);
        for s in sweep_shots(efforts) {
            let all = FailAt::new(usize::MAX);
            encode_through(&rgb, w, h, s, SWEEP_THREADS, &all, || all.refused()).unwrap();
            let count = all.seen.load(Ordering::Relaxed);
            for nth in 0..count {
                let one = FailAt::new(nth);
                // libjxl goes on without some allocations; what it then gives must still decode.
                if let Ok(file) =
                    encode_through(&rgb, w, h, s, SWEEP_THREADS, &one, || one.refused())
                {
                    let dec = jpegxl_rs::decoder_builder().build().unwrap();
                    let (meta, _) = dec.decode_with::<u8>(&file).unwrap_or_else(|e| {
                        panic!("allocation {nth} of {count} refused: {e}, {s:?}")
                    });
                    assert_eq!((meta.width, meta.height), (w, h));
                }
            }
        }
    }

    #[test]
    fn a_jxl_encode_short_of_its_budget_anywhere_fails_as_memory() {
        let (w, h) = SWEEP;
        let rgb = test_rgb(w, h);
        for s in sweep_shots(&[3, 9]) {
            let mut refused = 0;
            // The sweep stops at the first budget that suffices: larger ones repeat whole encodes.
            for budget in (0..64 << 20).step_by(16 << 10) {
                match encode(&rgb, w, h, s, budget, SWEEP_THREADS) {
                    Ok(_) => break,
                    Err(Failed::Memory) => refused += 1,
                    Err(e) => panic!("{e:?} at a budget of {budget}: {s:?}"),
                }
            }
            assert!(refused > 20, "{refused} budgets short: {s:?}");
            for budget in [64 << 20, 1 << 30] {
                assert!(
                    encode(&rgb, w, h, s, budget, SWEEP_THREADS).is_ok(),
                    "{budget}: {s:?}"
                );
            }
        }
    }
}
