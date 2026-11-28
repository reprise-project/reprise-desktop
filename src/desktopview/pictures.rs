//! Folder pictures on the desktop: a folder whose `.directory` names a picture
//! ([`reprisefs::meta::folder_picture`]) wears it in place of its icon, fitted inside the icon
//! square. The compositor never opens the picture: the thumbnail helper
//! ([`reprisefs::thumbpipe`]) reads and decodes it as Explorer's Thumbnails view does, within that
//! reader's deadline and stuck-read caps, on a job thread here that waits [`ANSWER_WITHIN`] for each
//! answer. A folder shows its icon until its picture lands. A picture saved over in place, which
//! the scan sees by its stamp and its watch ([`super::scan::picture_wants`]), is asked for again
//! and the old one shows until the new one lands.

use std::collections::HashMap;
use std::io::{self, BufReader, Read};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use reprisefs::{thumbnailers, thumbpipe};
use reprisekit::raster::{Canvas, fit_size};

use crate::{
    compositor::State,
    desktop::{FolderPicture, ICON_PX},
};

/// How long the helper may take over one picture before it is killed: its open and read (3 s
/// each), a slot wait, jail probe and jailed run ([`thumbnailers::DEADLINE`] each), a decode.
const ANSWER_WITHIN: Duration = Duration::from_secs(10 + 3 * thumbnailers::DEADLINE.as_secs());

/// How soon a picture that gave none (a dead mount, a file that is no picture) is asked for again
/// by a later read of the desktop.
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// What a picture looked like at an icon size, `None` once the helper gave none.
struct Worn {
    stamp: Option<(std::time::SystemTime, u64)>,
    px: i32,
    canvas: Option<Arc<Canvas>>,
    at: Instant,
    /// Unique per landing, so a cell redraws when its picture changes.
    serial: u64,
}

/// The desktop's folder pictures, by picture path, and the round fetching more.
#[derive(Default)]
pub(crate) struct Pictures {
    worn: HashMap<PathBuf, Worn>,
    /// The round running: its icon size and its stop flag.
    round: Option<(i32, Arc<AtomicBool>)>,
    serial: u64,
}

impl Pictures {
    /// The picture `path` at `px` device pixels and its landing's serial, once it has landed.
    pub(crate) fn shown(&self, path: &Path, px: i32) -> Option<(u64, Arc<Canvas>)> {
        let w = self.worn.get(path).filter(|w| w.px == px)?;
        Some((w.serial, w.canvas.clone()?))
    }

    /// The pictures `named` (the listed folders') that still want asking for at `px`: none yet
    /// at that size or changed since (a new stamp), then those that gave none over [`RETRY_AFTER`]
    /// ago, oldest first. Forgets the ones no folder names; a changed one shows meanwhile.
    fn wanted<'a>(
        &mut self,
        named: impl IntoIterator<Item = &'a FolderPicture>,
        px: i32,
        now: Instant,
    ) -> Vec<FolderPicture> {
        let named: HashMap<&PathBuf, &FolderPicture> =
            named.into_iter().map(|p| (&p.path, p)).collect();
        self.worn.retain(|p, _| named.contains_key(p));
        let mut want: Vec<FolderPicture> = (named.into_values())
            .filter(|p| {
                self.worn.get(&p.path).is_none_or(|w| {
                    w.px != px
                        || w.stamp != p.stamp
                        || (w.canvas.is_none() && now.duration_since(w.at) >= RETRY_AFTER)
                })
            })
            .cloned()
            .collect();
        let gave_none = |p: &FolderPicture| {
            (self.worn.get(&p.path))
                .filter(|w| w.px == px && w.stamp == p.stamp && w.canvas.is_none())
                .map(|w| w.at)
        };
        want.sort_by_cached_key(|p| (gave_none(p), p.path.clone()));
        want
    }

    fn land(&mut self, picture: FolderPicture, px: i32, canvas: Option<Canvas>, now: Instant) {
        self.serial += 1;
        let worn = Worn {
            stamp: picture.stamp,
            px,
            canvas: canvas.map(Arc::new),
            at: now,
            serial: self.serial,
        };
        self.worn.insert(picture.path, worn);
    }
}

/// `thumb`, a thumbnail of a picture, fitted inside the icon square at `scale` and never past the
/// picture's own size, centred on a transparent `ICON_PX` square in device pixels.
pub(crate) fn icon_of(thumb: &thumbpipe::Thumb, scale: i32) -> Canvas {
    let s = scale.max(1) as u32;
    let (dw, dh) = thumb.dims.unwrap_or((thumb.width, thumb.height));
    let (lw, lh) = fit_size(dw, dh, (ICON_PX as u32, ICON_PX as u32));
    let (w, h) = (lw.max(1) * s, lh.max(1) * s);
    let (sw, sh) = (thumb.width, thumb.height);
    let fitted =
        Canvas::resample_box_from(sw, sh, w, h, |x, y| thumb.pixels[(y * sw + x) as usize]);
    let side = ICON_PX as u32 * s;
    let mut c = Canvas::new(side, side);
    c.blit_over(((side - w) / 2) as i32, ((side - h) / 2) as i32, &fitted);
    c
}

impl State {
    /// Starts a round of the thumbnail helper for the listed folders' pictures that have not
    /// landed at the desktop's icon size, unless one runs: a new size stops it, and the next
    /// round starts when it ends.
    pub(crate) fn ask_desktop_pictures(&mut self) {
        let px = ICON_PX * self.draw_scale_at(self.first_output().as_ref());
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        if let Some((at, stop)) = &view.pictures.round {
            if *at != px {
                stop.store(true, Ordering::Release);
            }
            return;
        }
        let named = view.entries.iter().filter_map(|e| e.picture.as_ref());
        let want = view.pictures.wanted(named, px, Instant::now());
        // Without the loop's channel no picture could land.
        let Some(to_loop) = self.fsjobs.sender().filter(|_| !want.is_empty()) else {
            return;
        };
        let stop = Arc::new(AtomicBool::new(false));
        view.pictures.round = Some((px, stop.clone()));
        let scale = px / ICON_PX;
        // The job opens no file itself, so it never waits behind file jobs stuck on a mount.
        let started = self.spawn_job(move || {
            let paths: Vec<PathBuf> = want.iter().map(|p| p.path.clone()).collect();
            let mut want = want.into_iter();
            fetch(
                helper_command,
                &paths,
                px as u32,
                &stop,
                ANSWER_WITHIN,
                |path, thumb| {
                    let Some(picture) = want.find(|p| p.path == path) else {
                        return;
                    };
                    let canvas = thumb.map(|t| icon_of(&t, scale));
                    let _ = to_loop.send(Box::new(move |state: &mut State| {
                        state.desktop_picture_landed(picture, px, canvas)
                    }));
                },
            );
            Box::new(|state: &mut State| {
                if let Some(view) = state.desktop.as_mut() {
                    view.pictures.round = None;
                }
                state.ask_desktop_pictures();
            })
        });
        if !started && let Some(view) = self.desktop.as_mut() {
            view.pictures.round = None;
        }
    }

    fn desktop_picture_landed(&mut self, picture: FolderPicture, px: i32, canvas: Option<Canvas>) {
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        let shows = canvas.is_some();
        view.pictures.land(picture, px, canvas, Instant::now());
        if shows {
            self.relayout_desktop();
        }
    }
}

/// Helper failures in a row (a crash, a garbled answer, no answer within the deadline) after which
/// the rest of a round goes without it.
const GIVE_UP: u32 = 3;

/// Asks a helper `command` starts for each of `paths` fitted inside `edge`, in order, handing each
/// answer to `each` (`None` for none, or a helper that failed or took over `within`, which is
/// killed). Past [`GIVE_UP`] failures in a row, or once `stop` is set, the rest are not asked;
/// a helper that cannot start gives none for all of them.
fn fetch(
    command: impl Fn() -> Command,
    paths: &[PathBuf],
    edge: u32,
    stop: &AtomicBool,
    within: Duration,
    mut each: impl FnMut(PathBuf, Option<thumbpipe::Thumb>),
) {
    let edge = reprisefs::thumbnails::edge_for(edge);
    let (mut helper, mut failures) = (None, 0);
    for (i, path) in paths.iter().enumerate() {
        if failures >= GIVE_UP || stop.load(Ordering::Acquire) {
            break;
        }
        if helper.is_none() {
            match Helper::start(command()) {
                Ok(h) => helper = Some(h),
                Err(e) => {
                    log::warn!("desktop pictures: cannot start {}: {e}", thumbpipe::PROGRAM);
                    for p in &paths[i..] {
                        each(p.clone(), None);
                    }
                    return;
                }
            }
        }
        let Some(h) = helper.as_mut() else {
            continue;
        };
        match h.ask(path, edge, within) {
            Ok(thumb) => {
                failures = 0;
                each(path.clone(), thumb);
            }
            // A path the pipe cannot carry (a NUL in it) was never sent.
            Err(e) if e.kind() == io::ErrorKind::InvalidInput => each(path.clone(), None),
            Err(e) => {
                log::warn!(
                    "desktop pictures: the helper failed on {}: {e}",
                    path.display()
                );
                if let Some(h) = helper.take() {
                    h.stop(true);
                }
                failures += 1;
                each(path.clone(), None);
            }
        }
    }
    if let Some(h) = helper {
        h.stop(false);
    }
}

fn helper_command() -> Command {
    use std::os::unix::process::CommandExt;
    let mut c = Command::new(thumbpipe::PROGRAM);
    c.arg(thumbpipe::FLAG);
    crate::spawn::detach(&mut c);
    // SAFETY: the closure runs between fork and exec and only calls setpriority, a system call
    // that does not allocate.
    unsafe {
        c.pre_exec(|| {
            // Decoding gives way to the compositor; failing to lower it is harmless.
            libc::setpriority(libc::PRIO_PROCESS, 0, 10);
            Ok(())
        });
    }
    c
}

struct Helper {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<Deadline>,
}

impl Helper {
    fn start(mut c: Command) -> io::Result<Helper> {
        c.stdin(Stdio::piped()).stdout(Stdio::piped());
        let mut child = c.spawn()?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            reap(child);
            return Err(io::ErrorKind::BrokenPipe.into());
        };
        let stdout = Deadline {
            out: stdout,
            until: Instant::now(),
        };
        Ok(Helper {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    /// `path`'s thumbnail, failing with `TimedOut` when the answer takes longer than `within`.
    fn ask(
        &mut self,
        path: &Path,
        edge: u32,
        within: Duration,
    ) -> io::Result<Option<thumbpipe::Thumb>> {
        thumbpipe::write_request(&mut self.stdin, path, edge)?;
        self.stdout.get_mut().until = Instant::now() + within;
        thumbpipe::read_reply(&mut self.stdout, edge)
    }

    /// Closes the helper's stdin, which ends it between pictures, `kill` also SIGKILLs it, and a
    /// thread of its own waits for it: one blocked on a dead mount may outlive SIGKILL a while.
    fn stop(self, kill: bool) {
        let Helper {
            mut child, stdin, ..
        } = self;
        drop(stdin);
        if kill {
            let _ = child.kill();
        }
        reap(child);
    }
}

fn reap(mut child: Child) {
    let spawned = std::thread::Builder::new()
        .name("desktop-pictures-reap".into())
        .spawn(move || child.wait());
    if let Err(e) = spawned {
        log::warn!("desktop pictures: no thread to reap the helper: {e}");
    }
}

/// The helper's stdout: a read waiting past `until` fails with `TimedOut`.
struct Deadline {
    out: ChildStdout,
    until: Instant,
}

impl Read for Deadline {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.until.saturating_duration_since(Instant::now());
        let mut fd = libc::pollfd {
            fd: self.out.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = i32::try_from(left.as_millis()).unwrap_or(i32::MAX);
        // SAFETY: `fd` is one valid pollfd that outlives the call.
        match unsafe { libc::poll(&mut fd, 1, ms) } {
            0 => Err(io::ErrorKind::TimedOut.into()),
            // EINTR comes back as `Interrupted`, which `read_exact` retries.
            n if n < 0 => Err(io::Error::last_os_error()),
            _ => self.out.read(buf),
        }
    }
}

#[cfg(test)]
mod tests;
