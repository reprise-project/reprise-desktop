//! A still picture's file: PNG and JPEG encode in process, JPEG XL in reprise-encode, so a
//! crash, the OOM killer or a broken libjxl takes that process and the caller goes on. Screenshots
//! and pasted pictures save through it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use image::ImageEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
pub use reprise_encode::Jxl;
use reprise_encode::{BadReply, Format, Limits, Reply, Request};
use repriseconfine::helper::{self, signal_name};

/// How hard a PNG is squeezed; every level is lossless.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PngLevel {
    Fast,
    Default,
    Best,
}

/// A file's format and its settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Still {
    Png(PngLevel),
    /// The quality, 1..=100.
    Jpg(u8),
    Jxl(Jxl),
}

impl Still {
    pub fn extension(&self) -> &'static str {
        match self {
            Still::Png(_) => "png",
            Still::Jpg(_) => "jpg",
            Still::Jxl(_) => "jxl",
        }
    }
}

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

/// Why an encode gave no file.
#[derive(Debug, PartialEq, Eq)]
pub enum Failed {
    /// The encode needed more than this many bytes ([`jxl_budget`]).
    Memory(usize),
    /// reprise-encode ran past this deadline and was killed.
    Late(Duration),
    /// reprise-encode could not load its libraries: the binary was updated under the session.
    Outdated,
    /// reprise-encode's codec library did not load: the paths it tried and the loader's errors.
    Unavailable(String),
    /// reprise-encode died of this signal.
    Killed(i32),
    Codec(String),
}

/// The reason a failed save's warning gives.
impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Failed::Memory(budget) => write!(f, "the encode needed more than {budget} bytes"),
            Failed::Late(limit) => write!(
                f,
                "reprise-encode ran past its {} s deadline and was killed",
                limit.as_secs()
            ),
            Failed::Outdated => f.write_str(
                "reprise-encode exited with 127: the dynamic loader could not find a library",
            ),
            Failed::Unavailable(why) => write!(f, "the JPEG XL codec did not load: {why}"),
            Failed::Killed(signal) => {
                write!(f, "reprise-encode was killed by {}", signal_name(*signal))?;
                f.write_str(match *signal {
                    libc::SIGSYS => {
                        ": a system call outside its seccomp filter, which the kernel log's \
                         audit line (type=1326) names"
                    }
                    libc::SIGABRT => ": it aborted, usually out of memory under its limits",
                    libc::SIGXCPU => ": its CPU time limit",
                    libc::SIGKILL => ": the OOM killer or its hard CPU time limit",
                    _ => "",
                })
            }
            Failed::Codec(why) => f.write_str(why),
        }
    }
}

/// Opaque `rgb` (`w` x `h`, 3 bytes a pixel) encoded `how`. Blocking: a JPEG XL at high effort
/// takes seconds.
pub fn encode(rgb: &[u8], w: u32, h: u32, how: &Still) -> Result<Vec<u8>, Failed> {
    let mut out = Vec::new();
    let ty = image::ExtendedColorType::Rgb8;
    match *how {
        Still::Png(level) => {
            let level = match level {
                PngLevel::Fast => CompressionType::Fast,
                PngLevel::Default => CompressionType::Default,
                PngLevel::Best => CompressionType::Best,
            };
            PngEncoder::new_with_quality(&mut out, level, FilterType::Adaptive)
                .write_image(rgb, w, h, ty)
        }
        Still::Jpg(quality) => {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
                .write_image(rgb, w, h, ty)
        }
        Still::Jxl(jxl) => return jxl_encode(rgb, w, h, jxl, jxl_budget()),
    }
    .map_err(|e| Failed::Codec(e.to_string()))?;
    Ok(out)
}

/// A JPEG XL encodes in reprise-encode, so a crash, the OOM killer or a broken libjxl takes that
/// process and the session goes on.
fn jxl_encode(rgb: &[u8], w: u32, h: u32, jxl: Jxl, budget: usize) -> Result<Vec<u8>, Failed> {
    let request = Request {
        w,
        h,
        format: Format::Jxl(jxl),
    };
    let limits = Limits {
        budget: budget as u64,
        deadline: JXL_DEADLINE,
    };
    let encoded = helper::find("reprise-encode")
        .map_err(Failed::Codec)
        .and_then(|path| {
            let mut c = helper::command(&path);
            c.args(limits.to_args());
            in_child(c, request, rgb, limits)
        });
    if let Err(e) = &encoded {
        log::warn!("still: no JPEG XL: {e}");
    }
    encoded
}

/// How long reprise-encode may take: over three times the slowest encode the budget lets through
/// on two cores, a lossless effort 9 7680x2160 shot at 160 s.
const JXL_DEADLINE: Duration = Duration::from_secs(600);

/// The most of reprise-encode's stdout read: a file twice the size of the pixels is no
/// picture.
fn reply_cap(rgb_len: usize) -> usize {
    rgb_len.saturating_mul(2).saturating_add(64 << 10)
}

/// Run `c` as reprise-encode: `request` and `rgb` to its stdin, the reply from its stdout. One
/// still running past the deadline, or replying past [`reply_cap`], is killed. A failure logs
/// the end of its stderr.
fn in_child(
    c: std::process::Command,
    request: Request,
    rgb: &[u8],
    limits: Limits,
) -> Result<Vec<u8>, Failed> {
    use std::io::Read;
    use std::os::unix::process::ExitStatusExt;
    let program = c.get_program().to_string_lossy().into_owned();
    let cap = reply_cap(rgb.len());
    let head = request.to_bytes();
    let input = head.as_slice().chain(rgb);
    let ran = helper::run(c, input, helper::ReplyCap::growing(cap), limits.deadline)
        .map_err(|e| Failed::Codec(format!("cannot start {program}: {e}")))?;
    let ok = ran.status.as_ref().is_ok_and(|s| s.success());
    let failed = match Reply::parse(ran.reply) {
        _ if ran.late => Failed::Late(limits.deadline),
        _ if ran.past_cap => Failed::Codec(format!("reprise-encode replied past {cap} bytes")),
        Ok(Reply::File(file)) if ok => return Ok(file),
        Ok(Reply::Memory) => Failed::Memory(limits.budget as usize),
        Ok(Reply::Codec(why)) => Failed::Codec(why),
        Ok(Reply::Unavailable(why)) => Failed::Unavailable(why),
        Err(BadReply::Tag(tag)) => Failed::Codec(format!("reprise-encode replied {tag}")),
        // The dynamic loader's exit status for a library it cannot find.
        _ if ran.status.as_ref().is_ok_and(|s| s.code() == Some(127)) => Failed::Outdated,
        _ => match ran.status {
            Ok(s) => match s.signal() {
                Some(signal) => Failed::Killed(signal),
                None => Failed::Codec(format!("reprise-encode stopped ({s}) without a reply")),
            },
            Err(e) => Failed::Codec(format!("reprise-encode was lost: {e}")),
        },
    };
    if !ran.said.trim().is_empty() {
        log::warn!("still: reprise-encode said: {}", ran.said.trim());
    }
    Err(failed)
}

/// The picture types a paste saves, best first: those [`decode_rgb`] reads.
pub const PASTE_MIMES: [&str; 8] = [
    "image/png",
    "image/bmp",
    "image/x-bmp",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/tiff",
    "image/x-portable-pixmap",
];

/// `bytes` as opaque RGB (3 bytes a pixel) with its size, transparent pixels on white; None for
/// a picture the `image` crate does not read or one past its default memory limits.
pub fn decode_rgb(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let pic = reader.decode().ok()?.into_rgba8();
    let (w, h) = pic.dimensions();
    let rgb = (pic.pixels())
        .flat_map(|p| {
            let [r, g, b, a] = p.0;
            let on_white =
                |c: u8| ((c as u32 * a as u32 + 255 * (255 - a as u32) + 127) / 255) as u8;
            [on_white(r), on_white(g), on_white(b)]
        })
        .collect();
    Some((rgb, w, h))
}

/// A pasted picture's file name at local time `tm`.
pub fn pasted_name(tm: &libc::tm, ext: &str) -> String {
    format!(
        "Pasted image {:04}-{:02}-{:02} {:02}-{:02}-{:02}.{ext}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// A pasted picture's file and its extension: `bytes` decoded and encoded `how`, or as a PNG at
/// default compression when `how` cannot be written (no JPEG XL codec). Blocking.
pub fn pasted_file(bytes: &[u8], how: &Still) -> std::io::Result<(Vec<u8>, &'static str)> {
    let (rgb, w, h) = decode_rgb(bytes)
        .ok_or_else(|| std::io::Error::other("the picture on the clipboard cannot be read"))?;
    encode_or_png(how, |how| encode(&rgb, w, h, how))
        .map_err(|e| std::io::Error::other(e.to_string()))
}

fn encode_or_png(
    how: &Still,
    encode: impl Fn(&Still) -> Result<Vec<u8>, Failed>,
) -> Result<(Vec<u8>, &'static str), Failed> {
    match encode(how) {
        Ok(file) => Ok((file, how.extension())),
        Err(e) if !matches!(how, Still::Png(_)) => {
            log::warn!("still: {e}; saving a PNG instead");
            let png = Still::Png(PngLevel::Default);
            encode(&png).map(|file| (file, png.extension()))
        }
        Err(e) => Err(e),
    }
}

/// Writes `file` into `dir` as a new [`pasted_name`] for `secs` (seconds since the epoch),
/// numbered past a name taken.
pub fn write_pasted(dir: &Path, file: &[u8], ext: &str, secs: i64) -> std::io::Result<PathBuf> {
    use std::io::Write;
    let name = pasted_name(&crate::locale::localtime(secs), ext);
    write_into(&dir.join(name), |f| f.write_all(file)).map_err(std::io::Error::other)
}

/// A new file at the first free numbering of `path`, written by `write` to a hidden temp file
/// beside it, synced and renamed into place, so no half-written file ever has the name.
pub fn write_into(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> Result<PathBuf, String> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let (mut file, tmp) = crate::ops::create_temp_beside(path, None)
        .map_err(|e| format!("Cannot write in {}: {e}", dir.display()))?;
    let landed = write(&mut file)
        .and_then(|()| file.sync_all())
        .and_then(|()| crate::ops::rename_numbered(&tmp, path));
    landed.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Cannot write {}: {e}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pasted_picture_is_named_for_the_time() {
        // SAFETY: plain data, all zeros is a valid tm.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        (tm.tm_year, tm.tm_mon, tm.tm_mday) = (126, 10, 26);
        (tm.tm_hour, tm.tm_min, tm.tm_sec) = (1, 10, 5);
        assert_eq!(
            pasted_name(&tm, "jpg"),
            "Pasted image 2026-11-26 01-10-05.jpg"
        );
    }

    #[test]
    fn a_paste_takes_the_screenshot_format_and_png_where_it_cannot_be_written() {
        let jxl = Still::Jxl(Jxl {
            lossless: true,
            quality: 90,
            effort: 7,
        });
        let no_jxl = |how: &Still| match how {
            Still::Jxl(_) => Err(Failed::Unavailable("no libjxl".into())),
            how => Ok(how.extension().as_bytes().to_vec()),
        };
        assert_eq!(encode_or_png(&Still::Jpg(80), no_jxl).unwrap().1, "jpg");
        assert_eq!(
            encode_or_png(&jxl, no_jxl).unwrap(),
            (b"png".to_vec(), "png"),
            "the codec missing, a PNG"
        );
        let broken = |_: &Still| Err(Failed::Codec("disk".into()));
        assert!(encode_or_png(&Still::Png(PngLevel::Best), broken).is_err());
    }

    #[test]
    fn a_pasted_picture_decodes_on_white_and_saves_as_a_numbered_file() {
        let mut png = Vec::new();
        let rgba = [255u8, 0, 0, 255, 0, 0, 255, 0];
        PngEncoder::new(&mut png)
            .write_image(&rgba, 2, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        assert_eq!(
            decode_rgb(&png),
            Some((vec![255, 0, 0, 255, 255, 255], 2, 1)),
            "the clear pixel is white"
        );
        assert_eq!(decode_rgb(b"not a picture"), None);
        let (file, ext) = pasted_file(&png, &Still::Jpg(90)).unwrap();
        assert!(file.starts_with(&[0xff, 0xd8]) && ext == "jpg");
        let tmp = tempfile::tempdir().unwrap();
        let a = write_pasted(tmp.path(), &file, ext, 0).unwrap();
        let b = write_pasted(tmp.path(), &file, ext, 0).unwrap();
        assert!(
            a.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("Pasted image ")
        );
        assert_ne!(a, b, "the second takes a free number");
        assert_eq!(std::fs::read(&b).unwrap(), file);
    }

    #[test]
    fn png_and_jpg_write_their_own_files() {
        let rgb: Vec<u8> = (0..64 * 48 * 3).map(|i| (i % 251) as u8).collect();
        for (how, magic) in [
            (Still::Png(PngLevel::Default), &b"\x89PNG"[..]),
            (Still::Jpg(90), &[0xff, 0xd8][..]),
        ] {
            assert!(
                encode(&rgb, 64, 48, &how).unwrap().starts_with(magic),
                "{how:?}"
            );
        }
    }

    #[test]
    fn the_jxl_budget_reads_the_available_memory() {
        let meminfo = "MemTotal:        4000000 kB\nMemAvailable:    2097152 kB\n";
        assert_eq!(mem_available(meminfo), Some(2 << 30));
        assert_eq!(mem_available("MemTotal: 4000000 kB\n"), None);
    }

    const REQUEST: Request = Request {
        w: 2,
        h: 1,
        format: Format::Jxl(Jxl {
            lossless: true,
            quality: 90,
            effort: 7,
        }),
    };

    fn limits(deadline: Duration) -> Limits {
        Limits {
            budget: 1 << 20,
            deadline,
        }
    }

    fn child(script: &str) -> Result<Vec<u8>, Failed> {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", script]);
        in_child(c, REQUEST, &[0; 6], limits(Duration::from_secs(30)))
    }

    #[test]
    fn an_encoder_child_that_dies_gives_an_error() {
        assert_eq!(
            child("cat >/dev/null; kill -SYS $$"),
            Err(Failed::Killed(libc::SIGSYS))
        );
        assert!(
            Failed::Killed(libc::SIGSYS).to_string().contains("seccomp"),
            "the warning says what a SIGSYS means"
        );
        assert_eq!(
            child("cat >/dev/null; kill -ABRT $$"),
            Err(Failed::Killed(libc::SIGABRT))
        );
        assert!(
            Failed::Killed(libc::SIGABRT)
                .to_string()
                .contains("aborted, usually out of memory"),
            "the warning says what a SIGABRT means"
        );
        assert_eq!(
            child("cat >/dev/null; printf '\\001'"),
            Err(Failed::Memory(1 << 20))
        );
        assert!(matches!(child("exit 0"), Err(Failed::Codec(_))), "no reply");
        assert_eq!(
            child("cat >/dev/null; printf '\\011'"),
            Err(Failed::Codec("reprise-encode replied 9".into()))
        );
        assert_eq!(
            child("exit 127"),
            Err(Failed::Outdated),
            "the loader's exit status"
        );
        let Err(Failed::Codec(why)) = in_child(
            std::process::Command::new("/nonexistent/reprise-encode"),
            REQUEST,
            &[0; 6],
            limits(Duration::from_secs(30)),
        ) else {
            panic!("no helper is an error")
        };
        assert!(why.contains("/nonexistent/reprise-encode"), "{why}");
    }

    #[test]
    fn the_codecs_load_error_reaches_the_failure() {
        let why = "/usr/lib/reprise/libreprise_encode_jxl.so: libjxl.so.0.13: cannot open shared object file: No such file or directory";
        assert_eq!(
            child(&format!("cat >/dev/null; printf '\\003{why}'")),
            Err(Failed::Unavailable(why.into()))
        );
        assert!(Failed::Unavailable(why.into()).to_string().ends_with(why));
    }

    #[test]
    fn an_encoder_child_past_its_deadline_is_killed() {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", "cat >/dev/null; kill -STOP $$"]);
        let deadline = Duration::from_millis(500);
        let start = std::time::Instant::now();
        assert_eq!(
            in_child(c, REQUEST, &[0; 6], limits(deadline)),
            Err(Failed::Late(deadline))
        );
        assert!(start.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn an_encoder_child_that_closes_its_stdout_is_still_killed_at_the_deadline() {
        let deadline = Duration::from_millis(500);
        for script in [
            "cat >/dev/null; exec >&-; kill -STOP $$",
            "cat >/dev/null; printf '\\000file'; exec >&-; kill -STOP $$",
        ] {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", script]);
            let start = std::time::Instant::now();
            assert_eq!(
                in_child(c, REQUEST, &[0; 6], limits(deadline)),
                Err(Failed::Late(deadline)),
                "{script}"
            );
            assert!(start.elapsed() < Duration::from_secs(10));
        }
    }

    #[test]
    fn a_reply_past_twice_the_pixels_is_cut_off() {
        let cap = reply_cap(6);
        assert_eq!(cap, 12 + (64 << 10));
        let past = Err(Failed::Codec(format!(
            "reprise-encode replied past {cap} bytes"
        )));
        let reply_of = |file: usize| {
            child(&format!(
                "cat >/dev/null; printf '\\000'; exec head -c {file} /dev/zero"
            ))
        };
        assert_eq!(reply_of(100_000), past);
        assert_eq!(
            reply_of(cap),
            past,
            "the tag and the file one byte past the cap"
        );
        let Ok(file) = reply_of(cap - 1) else {
            panic!("a file filling the cap")
        };
        assert_eq!(file.len(), cap - 1);
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
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("cgroup-room");
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
}
