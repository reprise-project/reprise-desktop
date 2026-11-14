//! A screenshot's saved file in `[screenshot] format`. The clipboard always takes the fast PNG.

use image::ImageEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use jpegxl_rs::encode::EncoderSpeed;
use jpegxl_rs::memory::MemoryManager;
use jpegxl_sys::common::memory_manager::{JpegxlAllocFunc, JpegxlFreeFunc};
use repriseconf::config::{PngCompression, ScreenshotConfig, ShotFormat};
use reprisefs::allocbudget::Budget;

use crate::raster::Canvas;

/// File encodes under way at once: the one running and one waiting for it. A screenshot taken past
/// them is saved as its clipboard PNG, so queued stills cannot pile up in the compositor.
pub const MAX_ENCODES: usize = 2;

/// What libjxl may hold for one encode: half the available memory (512 MiB when /proc/meminfo
/// does not say), at most the room under the cgroup's memory limits and 4 GiB. A lossy 1920x1080
/// shot at effort 9 needs about 550 MiB.
pub fn jxl_budget() -> usize {
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let own = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    let room = own
        .lines()
        .find_map(|l| l.strip_prefix("0::"))
        .and_then(|p| cgroup_room(std::path::Path::new("/sys/fs/cgroup"), p));
    budget_of(mem_available(&meminfo), room)
}

fn budget_of(available: Option<usize>, cgroup_room: Option<usize>) -> usize {
    let half = available.map_or(512 << 20, |a| a / 2);
    half.min(cgroup_room.unwrap_or(usize::MAX)).min(4 << 30)
}

/// `MemAvailable` in bytes.
fn mem_available(meminfo: &str) -> Option<usize> {
    let line = meminfo
        .lines()
        .find_map(|l| l.strip_prefix("MemAvailable:"))?;
    let kb: usize = line.trim().strip_suffix("kB")?.trim().parse().ok()?;
    kb.checked_mul(1024)
}

/// The least `memory.max - memory.current` of cgroup `path` under `root` and its ancestors, None
/// when none of them has a limit.
fn cgroup_room(root: &std::path::Path, path: &str) -> Option<usize> {
    let read = |dir: &std::path::Path, file| -> Option<usize> {
        std::fs::read_to_string(dir.join(file))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    let mut dir = root.join(path.trim_start_matches('/'));
    let mut least: Option<usize> = None;
    while dir.starts_with(root) {
        if let (Some(max), Some(current)) = (read(&dir, "memory.max"), read(&dir, "memory.current"))
        {
            let room = max.saturating_sub(current);
            least = Some(least.map_or(room, |l| l.min(room)));
        }
        if !dir.pop() {
            break;
        }
    }
    least
}

/// Where a screenshot's file comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// The clipboard's PNG byte for byte: `[screenshot] format` is PNG at fast compression.
    Clipboard,
    /// An encode of its own in `[screenshot] format`.
    Encode,
    /// The clipboard's PNG, saved as PNG: [`MAX_ENCODES`] are under way.
    Busy,
}

/// The file of a screenshot taken while `encodes` file encodes are under way.
pub fn plan(s: &ScreenshotConfig, encodes: usize) -> Plan {
    if s.format == ShotFormat::Png && s.png_compression == PngCompression::Fast {
        Plan::Clipboard
    } else if encodes >= MAX_ENCODES {
        Plan::Busy
    } else {
        Plan::Encode
    }
}

/// Why an encode gave no file.
#[derive(Debug, PartialEq, Eq)]
pub enum Failed {
    /// libjxl asked for more than this many bytes ([`jxl_budget`]).
    Memory(usize),
    /// The encoder child ran past this deadline and was killed.
    Late(std::time::Duration),
    /// The encoder child could not load its libraries: the binary was updated under the session.
    Outdated,
    Codec(String),
}

/// Held through each file encode, so a second Print Screen waits for the first rather than
/// both taking the cores at once.
static ENCODING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `still`'s opaque RGB in `s.format`, one encode at a time, on a thread at nice 10. Blocking: a
/// JPEG XL at high effort takes seconds.
pub fn encode(still: &Canvas, s: &ScreenshotConfig) -> Result<Vec<u8>, Failed> {
    let _one = ENCODING.lock().unwrap_or_else(|e| e.into_inner());
    std::thread::scope(|scope| {
        let low = std::thread::Builder::new()
            .name("shotfile".into())
            .spawn_scoped(scope, || {
                lower_priority();
                encode_at(still, s, jxl_budget())
            });
        match low {
            Ok(t) => t
                .join()
                .unwrap_or_else(|_| Err(Failed::Codec("the encoder panicked".into()))),
            Err(_) => encode_at(still, s, jxl_budget()),
        }
    })
}

/// Nice 10 for the calling thread; the JPEG XL encoder child it starts inherits it.
fn lower_priority() {
    // SAFETY: plain system calls about the calling thread.
    let set = unsafe { libc::setpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t, 10) };
    if set != 0 {
        log::debug!(
            "screenshot: encoding at normal priority: {}",
            std::io::Error::last_os_error()
        );
    }
}

fn encode_at(still: &Canvas, s: &ScreenshotConfig, jxl_budget: usize) -> Result<Vec<u8>, Failed> {
    let (w, h) = still.device_size();
    let rgb: Vec<u8> = (still.pixels.iter())
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
        .collect();
    let mut out = Vec::new();
    let ty = image::ExtendedColorType::Rgb8;
    match s.format {
        ShotFormat::Png => {
            let level = match s.png_compression {
                PngCompression::Fast => CompressionType::Fast,
                PngCompression::Default => CompressionType::Default,
                PngCompression::Best => CompressionType::Best,
            };
            PngEncoder::new_with_quality(&mut out, level, FilterType::Adaptive)
                .write_image(&rgb, w, h, ty)
        }
        ShotFormat::Jpg => image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut out,
            s.jpg_quality.clamp(1, 100),
        )
        .write_image(&rgb, w, h, ty),
        ShotFormat::Jxl => return jxl_encode(&rgb, w, h, s, jxl_budget),
    }
    .map_err(|e| Failed::Codec(e.to_string()))?;
    Ok(out)
}

/// The compositor binary's hidden flag that makes it the JPEG XL encoder ([`child_main`]).
pub const CHILD_FLAG: &str = "--encode-jxl";

/// A JPEG XL encodes in a child of the compositor binary, so a crash or the OOM killer in libjxl
/// takes the child and the session goes on.
#[cfg(not(test))]
fn jxl_encode(
    rgb: &[u8],
    w: u32,
    h: u32,
    s: &ScreenshotConfig,
    budget: usize,
) -> Result<Vec<u8>, Failed> {
    use std::os::unix::process::CommandExt;
    let mut c = std::process::Command::new("/proc/self/exe");
    c.arg0("reprise").arg(CHILD_FLAG);
    in_child(c, Request::of(w, h, s, budget), rgb, JXL_DEADLINE)
}

/// How long the encoder child may take: over three times the slowest encode the budget lets
/// through on two cores, a lossless effort 9 7680x2160 shot at 160 s.
#[cfg(not(test))]
const JXL_DEADLINE: std::time::Duration = std::time::Duration::from_secs(600);

/// Tests run as the test binary, which has no [`CHILD_FLAG`].
#[cfg(test)]
fn jxl_encode(
    rgb: &[u8],
    w: u32,
    h: u32,
    s: &ScreenshotConfig,
    budget: usize,
) -> Result<Vec<u8>, Failed> {
    jxl(rgb, w, h, s, budget, encode_threads())
}

/// What the encoder child reads from stdin ahead of the RGB pixels, little-endian.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Request {
    w: u32,
    h: u32,
    lossless: bool,
    quality: u8,
    effort: u8,
    budget: u64,
}

const REQUEST_LEN: usize = 19;

/// The first byte of the child's reply: the file follows.
const REPLY_FILE: u8 = 0;
/// libjxl asked past the budget.
const REPLY_MEMORY: u8 = 1;
/// The codec's error message follows.
const REPLY_CODEC: u8 = 2;

impl Request {
    fn of(w: u32, h: u32, s: &ScreenshotConfig, budget: usize) -> Request {
        Request {
            w,
            h,
            lossless: s.jxl_lossless,
            quality: s.jxl_quality,
            effort: s.jxl_effort,
            budget: budget as u64,
        }
    }

    fn to_bytes(self) -> [u8; REQUEST_LEN] {
        let mut b = [0; REQUEST_LEN];
        b[0..4].copy_from_slice(&self.w.to_le_bytes());
        b[4..8].copy_from_slice(&self.h.to_le_bytes());
        b[8] = self.lossless.into();
        b[9] = self.quality;
        b[10] = self.effort;
        b[11..].copy_from_slice(&self.budget.to_le_bytes());
        b
    }

    fn from_bytes(b: &[u8; REQUEST_LEN]) -> Request {
        let word = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let mut budget = [0; 8];
        budget.copy_from_slice(&b[11..]);
        Request {
            w: word(0),
            h: word(4),
            lossless: b[8] != 0,
            quality: b[9],
            effort: b[10],
            budget: u64::from_le_bytes(budget),
        }
    }

    fn config(&self) -> ScreenshotConfig {
        ScreenshotConfig {
            format: ShotFormat::Jxl,
            jxl_lossless: self.lossless,
            jxl_quality: self.quality,
            jxl_effort: self.effort,
            ..Default::default()
        }
    }
}

/// Run `c` as the encoder child: `request` and `rgb` to its stdin, the file from its stdout. A
/// child still running after `deadline` is killed.
fn in_child(
    mut c: std::process::Command,
    request: Request,
    rgb: &[u8],
    deadline: std::time::Duration,
) -> Result<Vec<u8>, Failed> {
    use std::io::{Read, Write};
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    use std::sync::mpsc;
    c.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // SAFETY: the closure runs between fork and exec; sigemptyset, pthread_sigmask, signal,
    // prctl and close_range are async-signal-safe and nothing allocates.
    unsafe {
        c.pre_exec(|| {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::pthread_sigmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            // Close-on-exec rather than closed: std reports a failed exec on a descriptor above 2.
            libc::close_range(
                3,
                libc::c_uint::MAX,
                libc::CLOSE_RANGE_CLOEXEC as libc::c_int,
            );
            Ok(())
        });
    }
    let mut child = c
        .spawn()
        .map_err(|e| Failed::Codec(format!("cannot start the encoder: {e}")))?;
    let pid = child.id() as libc::pid_t;
    let (stdin, stdout) = (child.stdin.take(), child.stdout.take());
    let (done, finished) = mpsc::channel::<()>();
    let (tag, file, late) = std::thread::scope(|scope| {
        let watch = std::thread::Builder::new()
            .name("shotfile-watch".into())
            .spawn_scoped(scope, move || {
                let late = finished.recv_timeout(deadline) == Err(mpsc::RecvTimeoutError::Timeout);
                if late {
                    // SAFETY: a plain signal; the child is reaped only after this thread is
                    // joined, so `pid` is still the child's.
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                }
                late
            });
        if watch.is_err() {
            // SAFETY: as above, the child is not reaped yet.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        // The child reads all of its input before it writes, so neither pipe can fill up the other.
        if let Some(mut stdin) = stdin {
            let _ = stdin
                .write_all(&request.to_bytes())
                .and_then(|()| stdin.write_all(rgb));
        }
        let mut tag = [0; 1];
        let mut file = Vec::new();
        let tag = stdout.and_then(|mut out| {
            out.read_exact(&mut tag).ok()?;
            let _ = out.read_to_end(&mut file);
            Some(tag[0])
        });
        drop(done);
        let late = watch.is_ok_and(|w| w.join().unwrap_or(false));
        (tag, file, late)
    });
    let status = child.wait();
    let ok = status.as_ref().is_ok_and(|s| s.success());
    match tag {
        _ if late => Err(Failed::Late(deadline)),
        Some(REPLY_FILE) if ok => Ok(file),
        Some(REPLY_MEMORY) => Err(Failed::Memory(request.budget as usize)),
        Some(REPLY_CODEC) => Err(Failed::Codec(String::from_utf8_lossy(&file).into_owned())),
        // The dynamic loader's exit status for a library it cannot find.
        _ if status.as_ref().is_ok_and(|s| s.code() == Some(127)) => Err(Failed::Outdated),
        _ => Err(Failed::Codec(match status {
            Ok(s) => format!("the encoder stopped ({s})"),
            Err(e) => format!("the encoder was lost: {e}"),
        })),
    }
}

/// The encoder child's whole run ([`CHILD_FLAG`]): a request from stdin, the reply to stdout.
pub fn child_main() -> i32 {
    become_encoder();
    let served = serve(&mut std::io::stdin().lock(), &mut std::io::stdout().lock());
    i32::from(served.is_err())
}

/// The OOM killer's first pick, so it takes the encoder rather than the compositor, named
/// reprise-jxl in top and the kernel log; libjxl's threads inherit both.
fn become_encoder() {
    // Raising one's own score needs no privilege. The journal is not set up in the child.
    let _ = std::fs::write("/proc/self/oom_score_adj", "1000");
    // SAFETY: PR_SET_NAME reads a NUL-terminated string of at most 16 bytes.
    unsafe { libc::prctl(libc::PR_SET_NAME, c"reprise-jxl".as_ptr()) };
}

fn serve(input: &mut impl std::io::Read, output: &mut impl std::io::Write) -> std::io::Result<()> {
    let mut head = [0; REQUEST_LEN];
    input.read_exact(&mut head)?;
    let r = Request::from_bytes(&head);
    let len = (r.w as usize)
        .checked_mul(r.h as usize)
        .and_then(|n| n.checked_mul(3))
        .ok_or(std::io::ErrorKind::InvalidData)?;
    let mut rgb = vec![0; len];
    input.read_exact(&mut rgb)?;
    let budget = usize::try_from(r.budget).unwrap_or(usize::MAX);
    match jxl(&rgb, r.w, r.h, &r.config(), budget, encode_threads()) {
        Ok(file) => {
            output.write_all(&[REPLY_FILE])?;
            output.write_all(&file)?;
        }
        Err(Failed::Memory(_)) => output.write_all(&[REPLY_MEMORY])?,
        Err(Failed::Codec(why)) => {
            output.write_all(&[REPLY_CODEC])?;
            output.write_all(why.as_bytes())?;
        }
        Err(Failed::Late(_) | Failed::Outdated) => unreachable!("only the parent sees these"),
    }
    output.flush()
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
        reprisefs::allocbudget::alloc
    }

    fn free(&self) -> JpegxlFreeFunc {
        reprisefs::allocbudget::free
    }
}

/// libjxl's worker threads for an encode: half the cores. Its working memory grows with each.
fn encode_threads() -> usize {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    (cores / 2).max(1)
}

fn jxl(
    rgb: &[u8],
    w: u32,
    h: u32,
    s: &ScreenshotConfig,
    budget: usize,
    threads: usize,
) -> Result<Vec<u8>, Failed> {
    let capped = Capped(Budget::new(budget));
    jxl_through(rgb, w, h, s, threads, &capped, || capped.0.refused()).map_err(|e| {
        match capped.0.refused() {
            true => Failed::Memory(budget),
            false => Failed::Codec(e.to_string()),
        }
    })
}

/// The encode with libjxl allocating through `memory`, which says whether it `refused` a request.
fn jxl_through(
    rgb: &[u8],
    w: u32,
    h: u32,
    s: &ScreenshotConfig,
    threads: usize,
    memory: &dyn MemoryManager,
    refused: impl Fn() -> bool,
) -> Result<Vec<u8>, jpegxl_rs::EncodeError> {
    let runner = jpegxl_rs::ThreadsRunner::new(Some(memory), Some(threads))
        .ok_or(jpegxl_rs::EncodeError::CannotCreateEncoder)?;
    let speed = EFFORT[usize::from(s.jxl_effort.clamp(1, 9)) - 1];
    let builder = jpegxl_rs::encoder_builder()
        .memory_manager(memory)
        .parallel_runner(&runner)
        .speed(speed);
    // libjxl refuses a lossless frame unless the original colour profile is kept.
    let mut enc = if s.jxl_lossless {
        builder.lossless(true).uses_original_profile(true).build()?
    } else {
        // Quality 100 is distance 0, which libjxl's lossy (XYB) path does not take.
        builder
            .jpeg_quality(f32::from(s.jxl_quality.clamp(1, 99)))
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
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn test_canvas(w: u32, h: u32) -> Canvas {
        let px = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                0xff00_0000 | ((x * 4) << 16) | ((y * 5) << 8) | (((x ^ y) * 3) & 0xff)
            })
            .collect();
        Canvas::from_pixels(w, h, px)
    }

    fn rgb_of(c: &Canvas) -> Vec<u8> {
        (c.pixels.iter())
            .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
            .collect()
    }

    #[test]
    fn each_format_writes_its_own_file() {
        let still = test_canvas(64, 48);
        for (format, magic) in [
            (ShotFormat::Png, &b"\x89PNG"[..]),
            (ShotFormat::Jpg, &[0xff, 0xd8][..]),
            (ShotFormat::Jxl, &[0xff, 0x0a][..]),
        ] {
            let s = ScreenshotConfig {
                format,
                ..Default::default()
            };
            assert!(encode(&still, &s).unwrap().starts_with(magic), "{format:?}");
        }
        let lossy = ScreenshotConfig {
            format: ShotFormat::Jxl,
            jxl_lossless: false,
            jxl_effort: 1,
            ..Default::default()
        };
        assert!(encode(&still, &lossy).unwrap().starts_with(&[0xff, 0x0a]));
    }

    #[test]
    fn a_lossless_jxl_and_every_png_level_keep_every_pixel() {
        let still = test_canvas(64, 48);
        let want = rgb_of(&still);
        for png_compression in PngCompression::ALL {
            let s = ScreenshotConfig {
                png_compression,
                ..Default::default()
            };
            let back = image::load_from_memory(&encode(&still, &s).unwrap()).unwrap();
            assert_eq!(back.to_rgb8().into_raw(), want, "{png_compression:?}");
        }
        let s = ScreenshotConfig {
            format: ShotFormat::Jxl,
            ..Default::default()
        };
        let bytes = encode(&still, &s).unwrap();
        let dec = jpegxl_rs::decoder_builder()
            .pixel_format(jpegxl_rs::decode::PixelFormat {
                num_channels: 3,
                ..Default::default()
            })
            .build()
            .unwrap();
        let (meta, px) = dec.decode_with::<u8>(&bytes).unwrap();
        assert_eq!((meta.width, meta.height), (64, 48));
        assert_eq!(px, want);
    }

    #[test]
    fn a_lossy_jxl_at_quality_100_saves_at_99() {
        let still = test_canvas(64, 48);
        let at = |jxl_quality| {
            let s = ScreenshotConfig {
                format: ShotFormat::Jxl,
                jxl_lossless: false,
                jxl_quality,
                jxl_effort: 1,
                ..Default::default()
            };
            encode(&still, &s).unwrap()
        };
        assert_eq!(at(100), at(99));
        assert_ne!(at(99), at(90));
    }

    #[test]
    fn a_third_screenshot_under_way_keeps_its_clipboard_png() {
        let jxl = ScreenshotConfig {
            format: ShotFormat::Jxl,
            ..Default::default()
        };
        assert_eq!(plan(&jxl, 0), Plan::Encode);
        assert_eq!(
            plan(&jxl, 1),
            Plan::Encode,
            "the second waits for the first"
        );
        assert_eq!(plan(&jxl, 2), Plan::Busy);
        let fast = ScreenshotConfig {
            png_compression: PngCompression::Fast,
            ..Default::default()
        };
        assert_eq!(plan(&fast, 2), Plan::Clipboard);
        assert_eq!(
            plan(&ScreenshotConfig::default(), 0),
            Plan::Encode,
            "files default to Default compression"
        );
    }

    #[test]
    fn a_jxl_encode_past_its_memory_budget_fails_as_memory() {
        let still = test_canvas(256, 256);
        let s = ScreenshotConfig {
            format: ShotFormat::Jxl,
            jxl_lossless: false,
            ..Default::default()
        };
        assert!(encode_at(&still, &s, 256 << 20).is_ok());
        assert_eq!(encode_at(&still, &s, 1 << 20), Err(Failed::Memory(1 << 20)));
    }

    #[test]
    fn the_jxl_budget_reads_the_available_memory() {
        let meminfo = "MemTotal:        4000000 kB\nMemAvailable:    2097152 kB\n";
        assert_eq!(mem_available(meminfo), Some(2 << 30));
        assert_eq!(mem_available("MemTotal: 4000000 kB\n"), None);
    }

    #[test]
    fn the_encoder_child_answers_a_request_with_the_file_or_why_not() {
        let (w, h) = (64, 48);
        let rgb = rgb_of(&test_canvas(w, h));
        let s = ScreenshotConfig {
            format: ShotFormat::Jxl,
            ..Default::default()
        };
        let ask = |budget| {
            let r = Request::of(w, h, &s, budget);
            assert_eq!(Request::from_bytes(&r.to_bytes()), r);
            let mut input = r.to_bytes().to_vec();
            input.extend_from_slice(&rgb);
            let mut reply = Vec::new();
            serve(&mut input.as_slice(), &mut reply).unwrap();
            reply
        };
        let file = ask(256 << 20);
        assert_eq!(file[0], REPLY_FILE);
        assert!(file[1..].starts_with(&[0xff, 0x0a]), "a JPEG XL follows");
        assert_eq!(ask(4096), [REPLY_MEMORY]);
    }

    #[test]
    fn an_encoder_child_that_dies_gives_an_error() {
        let child = |script: &str| {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", script]);
            let r = Request::of(2, 1, &ScreenshotConfig::default(), 1 << 20);
            in_child(c, r, &[0; 6], std::time::Duration::from_secs(30))
        };
        let Err(Failed::Codec(why)) = child("cat >/dev/null; kill -SEGV $$") else {
            panic!("a crash is an error");
        };
        assert!(why.contains("SIGSEGV"), "{why}");
        assert_eq!(
            child("cat >/dev/null; printf '\\001'"),
            Err(Failed::Memory(1 << 20))
        );
        assert!(matches!(child("exit 0"), Err(Failed::Codec(_))), "no reply");
        assert_eq!(
            child("exit 127"),
            Err(Failed::Outdated),
            "the loader's exit status"
        );
    }

    #[test]
    fn an_encoder_child_past_its_deadline_is_killed() {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", "cat >/dev/null; kill -STOP $$"]);
        let r = Request::of(2, 1, &ScreenshotConfig::default(), 1 << 20);
        let deadline = std::time::Duration::from_millis(500);
        let start = std::time::Instant::now();
        assert_eq!(
            in_child(c, r, &[0; 6], deadline),
            Err(Failed::Late(deadline))
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
    }

    #[test]
    fn the_encoder_child_is_the_oom_killers_first_pick_and_named_reprise_jxl() {
        // The score and the name stay with the process: run in one of its own.
        const ALONE: &str = "REPRISE_SHOTFILE_TEST_ALONE";
        if std::env::var_os(ALONE).is_none() {
            let name = "shotfile::tests::the_encoder_child_is_the_oom_killers_first_pick_and_named_reprise_jxl";
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", name, "--quiet"])
                .env(ALONE, "1")
                .status()
                .unwrap();
            assert!(status.success(), "the test passes in a process of its own");
            return;
        }
        become_encoder();
        let read = |f| std::fs::read_to_string(f).unwrap();
        assert_eq!(read("/proc/self/oom_score_adj"), "1000\n");
        assert_eq!(read("/proc/thread-self/comm"), "reprise-jxl\n");
    }

    #[test]
    fn the_jxl_budget_is_half_the_available_memory_under_the_cgroup_room_and_4_gib() {
        assert_eq!(budget_of(Some(2 << 30), None), 1 << 30);
        assert_eq!(budget_of(None, None), 512 << 20);
        assert_eq!(budget_of(Some(2 << 30), Some(300 << 20)), 300 << 20);
        assert_eq!(budget_of(Some(64 << 30), None), 4 << 30);
    }

    #[test]
    fn the_cgroup_room_is_the_least_left_under_any_limit_above() {
        let root = crate::harness::scratch().join("cgroup-room");
        let leaf = root.join("user.slice/app.scope");
        std::fs::create_dir_all(&leaf).unwrap();
        let limit = |dir: &std::path::Path, max: &str, current: &str| {
            std::fs::write(dir.join("memory.max"), max).unwrap();
            std::fs::write(dir.join("memory.current"), current).unwrap();
        };
        assert_eq!(cgroup_room(&root, "/user.slice/app.scope"), None);
        limit(&leaf, "max\n", "100\n");
        limit(&root.join("user.slice"), "1000\n", "700\n");
        assert_eq!(cgroup_room(&root, "/user.slice/app.scope"), Some(300));
        limit(&leaf, "350\n", "100\n");
        assert_eq!(cgroup_room(&root, "/user.slice/app.scope"), Some(250));
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
    fn sweep_shots(efforts: &[u8]) -> impl Iterator<Item = ScreenshotConfig> {
        efforts.iter().flat_map(|&jxl_effort| {
            [false, true].map(|jxl_lossless| ScreenshotConfig {
                format: ShotFormat::Jxl,
                jxl_lossless,
                jxl_effort,
                ..Default::default()
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
        let rgb = rgb_of(&test_canvas(w, h));
        for s in sweep_shots(efforts) {
            let all = FailAt::new(usize::MAX);
            jxl_through(&rgb, w, h, &s, SWEEP_THREADS, &all, || all.refused()).unwrap();
            let count = all.seen.load(Ordering::Relaxed);
            for nth in 0..count {
                let one = FailAt::new(nth);
                // libjxl goes on without some allocations; what it then gives must still decode.
                if let Ok(file) = jxl_through(&rgb, w, h, &s, SWEEP_THREADS, &one, || one.refused())
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
        let rgb = rgb_of(&test_canvas(w, h));
        for s in sweep_shots(&[3, 9]) {
            let mut refused = 0;
            // The sweep stops at the first budget that suffices: larger ones repeat whole encodes.
            for budget in (0..64 << 20).step_by(16 << 10) {
                match jxl(&rgb, w, h, &s, budget, SWEEP_THREADS) {
                    Ok(_) => break,
                    Err(Failed::Memory(b)) => {
                        assert_eq!(b, budget);
                        refused += 1;
                    }
                    Err(e) => panic!("{e:?} at a budget of {budget}: {s:?}"),
                }
            }
            assert!(refused > 20, "{refused} budgets short: {s:?}");
            for budget in [64 << 20, 1 << 30] {
                assert!(
                    jxl(&rgb, w, h, &s, budget, SWEEP_THREADS).is_ok(),
                    "{budget}: {s:?}"
                );
            }
        }
    }
}
