//! A screenshot's saved file in `[screenshot] format`. The clipboard always takes the fast PNG.

use std::time::Duration;

use image::ImageEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use reprise_encode::{BadReply, Format, Jxl, Limits, Reply, Request};
use repriseconf::config::{PngCompression, ScreenshotConfig, ShotFormat};
use repriseconfine::helper::{self, signal_name};

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

/// The reason a failed screenshot's warning gives.
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

/// Nice 10 for the calling thread; the reprise-encode it starts inherits it.
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

/// A JPEG XL encodes in reprise-encode, so a crash, the OOM killer or a broken libjxl takes that
/// process and the session goes on.
fn jxl_encode(
    rgb: &[u8],
    w: u32,
    h: u32,
    s: &ScreenshotConfig,
    budget: usize,
) -> Result<Vec<u8>, Failed> {
    let request = Request {
        w,
        h,
        format: Format::Jxl(Jxl {
            lossless: s.jxl_lossless,
            quality: s.jxl_quality,
            effort: s.jxl_effort,
        }),
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
        log::warn!("screenshot: no JPEG XL: {e}");
    }
    encoded
}

/// How long reprise-encode may take: over three times the slowest encode the budget lets through
/// on two cores, a lossless effort 9 7680x2160 shot at 160 s.
const JXL_DEADLINE: Duration = Duration::from_secs(600);

/// The most of reprise-encode's stdout read: a file twice the size of the pixels is no
/// screenshot.
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
        log::warn!("screenshot: reprise-encode said: {}", ran.said.trim());
    }
    Err(failed)
}

#[cfg(test)]
mod tests {
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
    fn png_and_jpg_write_their_own_files() {
        let still = test_canvas(64, 48);
        for (format, magic) in [
            (ShotFormat::Png, &b"\x89PNG"[..]),
            (ShotFormat::Jpg, &[0xff, 0xd8][..]),
        ] {
            let s = ScreenshotConfig {
                format,
                ..Default::default()
            };
            assert!(encode(&still, &s).unwrap().starts_with(magic), "{format:?}");
        }
    }

    #[test]
    fn every_png_level_keeps_every_pixel() {
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
}
