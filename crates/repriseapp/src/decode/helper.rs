//! Formats decoded in reprise-decode, a confined process of its own: a decoder's crash, abort or
//! runaway allocation costs that process and never the program. An abort or a kill reads as
//! [`Unread::TooLarge`], a crash as [`Unread::Unreadable`].

use std::io::Read;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::time::Duration;

use reprise_decode_protocol::{BadReply, Format, Limits, PROGRAM, Picture, Reply, Request};
use repriseconfine::helper::{self, signal_name};

use super::{Budget, Decoded, ImageDecoder, JXL_TIME_LIMIT, Unread, fit_buffer, is_jxl};

/// How long past its decode's deadline reprise-decode may take to start, confine itself and
/// reply before it is killed.
const SLACK: Duration = Duration::from_secs(5);

/// One format through reprise-decode, the picture fitted here.
pub struct HelperDecoder {
    format: Format,
    helper: Result<PathBuf, String>,
    deadline: Duration,
    /// How long past `deadline` reprise-decode is killed.
    slack: Duration,
}

impl HelperDecoder {
    /// JPEG XL's first frame, by reprise-decode beside this program or in /usr/bin, under
    /// [`JXL_TIME_LIMIT`].
    pub fn jxl() -> HelperDecoder {
        HelperDecoder::new(Format::Jxl, helper::find(PROGRAM), JXL_TIME_LIMIT)
    }

    /// [`HelperDecoder::jxl`] by the reprise-decode at `path`.
    pub fn jxl_at(path: PathBuf) -> HelperDecoder {
        HelperDecoder::new(Format::Jxl, Ok(path), JXL_TIME_LIMIT)
    }

    pub(crate) fn new(
        format: Format,
        helper: Result<PathBuf, String>,
        deadline: Duration,
    ) -> HelperDecoder {
        HelperDecoder {
            format,
            helper,
            deadline,
            slack: SLACK,
        }
    }

    fn name(&self) -> &'static str {
        match self.format {
            Format::Jxl => "JPEG XL",
        }
    }

    /// [`ImageDecoder::decode`] of open file `f`, streamed to reprise-decode rather than read in
    /// here; a file past `budget.file` is refused unread.
    pub fn decode_file(
        &self,
        mut f: std::fs::File,
        max: (u32, u32),
        budget: Budget,
    ) -> Result<Decoded, Unread> {
        let len = f.metadata().map_err(|_| Unread::Unreadable)?.len();
        if len > budget.file {
            return Err(Unread::TooLarge);
        }
        let mut head = Vec::new();
        (&mut f)
            .take(HEAD.min(len))
            .read_to_end(&mut head)
            .map_err(|_| Unread::Unreadable)?;
        if too_large(&head, budget) {
            return Err(Unread::TooLarge);
        }
        let p = self.picture(head.as_slice().chain(f), len, budget)?;
        fitted(p, max)
    }

    /// The `len` bytes of `file` decoded by reprise-decode under `budget`.
    fn picture(&self, file: impl Read, len: u64, budget: Budget) -> Result<Picture, Unread> {
        let name = self.name();
        let path = self.helper.as_ref().map_err(|why| {
            log::warn!("{name} not decoded: {why}");
            Unread::Unreadable
        })?;
        let limits = Limits {
            file: budget.file.max(len),
            pixels: budget.pixels,
            deadline: self.deadline,
        };
        let request = Request {
            format: self.format,
            len,
        };
        let mut c = helper::command(path);
        c.args(limits.to_args());
        let cap = limits.reply_cap();
        let head = request.to_bytes();
        let input = head.as_slice().chain(file.take(len));
        let ran = helper::run(c, input, cap, self.deadline + self.slack).map_err(|e| {
            log::warn!("{name} not decoded: cannot start {}: {e}", path.display());
            Unread::Unreadable
        })?;
        let ok = ran.status.as_ref().is_ok_and(|s| s.success());
        let said = ran.said.trim();
        let warn = |why: &str| {
            log::warn!("{name} not decoded: {why}; reprise-decode said: {said}");
            Unread::Unreadable
        };
        match Reply::parse(ran.reply, limits) {
            _ if ran.late => {
                log::debug!("{name}: reprise-decode ran past its deadline and was killed");
                Err(Unread::Unreadable)
            }
            _ if ran.past_cap => Err(warn(&format!(
                "reprise-decode replied past {} bytes",
                cap.max
            ))),
            Ok(Reply::Picture(p)) if ok => Ok(p),
            Ok(Reply::TooLarge) => Err(Unread::TooLarge),
            Ok(Reply::Unreadable(why)) => {
                log::debug!("{name}: {why}");
                Err(Unread::Unreadable)
            }
            Ok(Reply::Failed(why)) => Err(warn(&why)),
            Err(BadReply::Tag(tag)) => Err(warn(&format!("reprise-decode replied {tag}"))),
            Err(BadReply::Size) => Err(warn("reprise-decode replied a picture of the wrong size")),
            _ => match ran.status {
                Ok(s) => match s.signal() {
                    Some(libc::SIGSYS) => Err(warn(
                        "reprise-decode was killed by SIGSYS: a system call outside its seccomp \
                         filter",
                    )),
                    Some(signal @ (libc::SIGSEGV | libc::SIGBUS)) => Err(warn(&format!(
                        "the decoder crashed ({})",
                        signal_name(signal)
                    ))),
                    // The soft CPU limit, set at the deadline.
                    Some(libc::SIGXCPU) => {
                        log::debug!("{name}: reprise-decode ran past its CPU time and stopped");
                        Err(Unread::Unreadable)
                    }
                    // An abort is how a decoder meets an allocation it was refused, a SIGKILL
                    // the OOM killer.
                    Some(signal) => {
                        log::debug!(
                            "{name}: reprise-decode was killed by {}",
                            signal_name(signal)
                        );
                        Err(Unread::TooLarge)
                    }
                    None => Err(warn(&format!(
                        "reprise-decode stopped ({s}) without a reply"
                    ))),
                },
                Err(e) => Err(warn(&format!("reprise-decode was lost: {e}"))),
            },
        }
    }
}

impl ImageDecoder for HelperDecoder {
    fn probe(&self, bytes: &[u8]) -> bool {
        match self.format {
            Format::Jxl => is_jxl(bytes),
        }
    }

    /// A header past `budget` is refused here, before reprise-decode starts.
    fn decode(&self, bytes: &[u8], max: (u32, u32), budget: Budget) -> Result<Decoded, Unread> {
        if too_large(bytes, budget) {
            return Err(Unread::TooLarge);
        }
        let p = self.picture(bytes, bytes.len() as u64, budget)?;
        fitted(p, max)
    }
}

fn fitted(p: Picture, max: (u32, u32)) -> Result<Decoded, Unread> {
    let img = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(p.width, p.height, p.rgba())
        .ok_or(Unread::Unreadable)?;
    let (fw, fh) = reprisekit::raster::fit_size(p.width, p.height, max);
    Ok(Decoded {
        canvas: fit_buffer(&img, fw, fh),
        width: p.width,
        height: p.height,
    })
}

/// The bytes of a file [`HelperDecoder::decode_file`] reads first to find its size header in.
const HEAD: u64 = 64 << 10;

/// Whether the picture `head` starts states more pixels than `budget`.
fn too_large(head: &[u8], budget: Budget) -> bool {
    reprisefs::imagesize::dimensions(head)
        .is_some_and(|(w, h)| u64::from(w) * u64::from(h) * 4 > budget.pixels)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    /// A reprise-decode stand-in running `body` after it has read its request.
    pub(crate) fn fake(dir: &Path, body: &str) -> HelperDecoder {
        HelperDecoder::new(Format::Jxl, Ok(fake_at(dir, body)), Duration::from_secs(30))
    }

    pub(crate) fn fake_at(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("reprise-decode");
        std::fs::write(&path, format!("#!/bin/sh\ncat >/dev/null\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    const FILE: &[u8] = &[0xff, 0x0a, 0, 0];

    fn decode(d: &HelperDecoder) -> Result<Decoded, Unread> {
        d.decode(FILE, (96, 96), Budget::THUMBNAIL)
    }

    #[test]
    fn a_picture_reply_is_fitted_here() {
        let dir = tempfile::tempdir().unwrap();
        // 2x1: opaque red, then transparent blue.
        let reply = r"\000\002\000\000\000\001\000\000\000\377\000\000\377\000\000\377\000";
        let d = fake(dir.path(), &format!("printf '{reply}'"));
        let full = decode(&d).unwrap();
        assert_eq!((full.width, full.height), (2, 1));
        assert_eq!(full.canvas.get(0, 0), 0xffff_0000);
        let fitted = d.decode(FILE, (1, 1), Budget::THUMBNAIL).unwrap();
        assert_eq!((fitted.width, fitted.canvas.width), (2, 1));
        assert_eq!(fitted.canvas.get(0, 0), 0x8080_0000);
    }

    #[test]
    fn the_helper_gets_its_limits_and_the_request() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise-decode");
        let seen = dir.path().join("seen");
        let script = format!(
            "#!/bin/sh\necho \"$@\" > '{0}'\nhead -c 10 | od -An -tu1 >> '{0}'\nprintf '\\001'\n",
            seen.display()
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let d = HelperDecoder::new(Format::Jxl, Ok(path), JXL_TIME_LIMIT);
        assert_eq!(decode(&d).err(), Some(Unread::TooLarge));
        let seen = std::fs::read_to_string(&seen).unwrap();
        let mut lines = seen.lines();
        let b = Budget::THUMBNAIL;
        assert_eq!(
            lines.next(),
            Some(format!("--file {} --pixels {} --deadline 60", b.file, b.pixels).as_str())
        );
        let head: Vec<&str> = lines.flat_map(str::split_whitespace).collect();
        assert_eq!(head, ["1", "1", "4", "0", "0", "0", "0", "0", "0", "0"]);
    }

    #[test]
    fn a_helper_that_aborts_or_is_killed_reads_as_too_large() {
        let dir = tempfile::tempdir().unwrap();
        for signal in ["ABRT", "KILL"] {
            let d = fake(dir.path(), &format!("kill -{signal} $$"));
            assert_eq!(decode(&d).err(), Some(Unread::TooLarge), "{signal}");
        }
    }

    #[test]
    fn a_broken_or_missing_helper_reads_as_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        for body in [
            "exit 0",
            "exit 127",
            "printf '\\002cut short'",
            "printf '\\003another version'",
            "printf '\\011'",
            "printf '\\000\\001\\000\\000\\000\\001\\000\\000\\000\\377'",
            "kill -SYS $$",
            "kill -SEGV $$",
            "kill -BUS $$",
            "kill -XCPU $$",
        ] {
            let d = fake(dir.path(), body);
            assert_eq!(decode(&d).err(), Some(Unread::Unreadable), "{body}");
        }
        let missing = dir.path().join("none");
        let d = HelperDecoder::new(Format::Jxl, Ok(missing), JXL_TIME_LIMIT);
        assert_eq!(decode(&d).err(), Some(Unread::Unreadable));
        let unfound =
            HelperDecoder::new(Format::Jxl, Err("no reprise-decode".into()), JXL_TIME_LIMIT);
        assert_eq!(decode(&unfound).err(), Some(Unread::Unreadable));
    }

    #[test]
    fn a_helper_past_its_deadline_is_killed_and_reads_as_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise-decode");
        std::fs::write(&path, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let d = HelperDecoder {
            slack: Duration::from_millis(200),
            ..HelperDecoder::new(Format::Jxl, Ok(path), Duration::ZERO)
        };
        let start = std::time::Instant::now();
        assert_eq!(decode(&d).err(), Some(Unread::Unreadable));
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_file_is_streamed_to_the_helper_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise-decode");
        let seen = dir.path().join("seen");
        let script = format!(
            "#!/bin/sh\nhead -c 10 >/dev/null\ncat > '{}'\nprintf '\\001'\n",
            seen.display()
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let file: Vec<u8> = [0xff, 0x0a]
            .into_iter()
            .chain((0..200_000u32).map(|i| i as u8))
            .collect();
        let src = dir.path().join("a.jxl");
        std::fs::write(&src, &file).unwrap();
        let d = HelperDecoder::jxl_at(path);
        let open = || std::fs::File::open(&src).unwrap();
        assert_eq!(
            d.decode_file(open(), (96, 96), Budget::THUMBNAIL).err(),
            Some(Unread::TooLarge)
        );
        assert!(
            std::fs::read(&seen).unwrap() == file,
            "the file as it is on disk"
        );
        let small = Budget {
            file: file.len() as u64 - 1,
            ..Budget::THUMBNAIL
        };
        std::fs::remove_file(&seen).unwrap();
        assert_eq!(
            d.decode_file(open(), (96, 96), small).err(),
            Some(Unread::TooLarge)
        );
        assert!(!seen.exists(), "a file past the budget is not sent");
    }

    #[test]
    fn a_header_past_the_budget_is_refused_without_the_helper() {
        // A codestream whose SizeHeader claims 20000x20000 (the 18-bit height form, ratio 1:1).
        let big = [0xff, 0x0a, 0xfc, 0x70, 0x22, 0x01];
        assert_eq!(
            reprisefs::imagesize::dimensions(&big),
            Some((20_000, 20_000))
        );
        let d = HelperDecoder::new(Format::Jxl, Err("never run".into()), JXL_TIME_LIMIT);
        assert!(d.probe(&big));
        assert_eq!(
            d.decode(&big, (96, 96), Budget::THUMBNAIL).err(),
            Some(Unread::TooLarge)
        );
    }
}
