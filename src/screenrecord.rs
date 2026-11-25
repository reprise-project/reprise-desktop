//! Screen recordings: `reprise-record` writes an output, or an area of it, to a new MP4 under
//! Videos/Screencasts while the tray's Recording cell shows. The bind records the output under the
//! pointer and stops a running recording; the picker's Record mode records an area, clipped to the
//! output holding most of it. None starts while locked. A stop lets the recorder finish its file;
//! one idle for `STOP_GRACE`, past `finish_cap` or stopped again is killed and its file kept. A
//! recording stopped before anything was written leaves no file and no notice. An area recording
//! stops when its output's scale or transform changes.

use std::ffi::OsString;
use std::io::BufRead;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use smithay::output::Output;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::State;
use crate::notifyview::OwnAction;
use crate::raster::Rect;

const PROGRAM: &str = "reprise-record";
/// How long a stopped recorder may go without using the CPU, reading or writing its storage
/// before it is killed. One that works on or waits on its storage is waited for, up to
/// [`finish_cap`].
const STOP_GRACE: Duration = Duration::from_secs(5);
const SAVED: &str = "Screen recording saved";
const NOT_FINISHED: &str = "Screen recording not finished";
const MAY_NOT_PLAY: &str = "It was stopped while it closed the file and may not play.";
/// The recorder's exit status for a recording that stopped by itself and kept its file; its last
/// stderr line says why.
const ENDED_EARLY: i32 = 3;
/// The recorder's exit status for a recording saved whole whose sound stopped; its last stderr
/// line says why.
const SOUND_STOPPED: i32 = 4;
/// The recorder's exit status when it was stopped before it wrote anything.
const STOPPED_UNSTARTED: i32 = 5;
/// The recorder's exit status for a recording saved whole whose microphone stopped; its last
/// stderr line is the notice's line.
const MICROPHONE_STOPPED: i32 = 6;
/// The dynamic loader's exit status when a library does not load.
const LIBRARY_MISSING: i32 = 127;

/// What a recording records: an output, or an area of it in output-local logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub output: Output,
    pub area: Option<Rect>,
}

pub struct Recording {
    pub target: Target,
    /// The output's scale and transform as the recording started.
    shape: Shape,
    started: Instant,
    /// The recorder, once its file is made and it runs.
    pid: Option<u32>,
    /// The recorder's pidfd, which Stop signals: its `wait` job may reap it before the loop
    /// hears, and a reaped pid can name another process.
    pidfd: Option<OwnedFd>,
    /// A stop was asked for: the recorder finishes its file, or was not running yet.
    stopping: bool,
    /// The stopped recorder's [`work`] at the last look.
    work: Option<Work>,
    /// When a stopped recorder is killed however it works, see [`finish_cap`].
    finish_by: Option<Instant>,
    /// Set as the recorder is killed finishing its file, which is then kept unless empty.
    killed: Arc<AtomicBool>,
}

impl Recording {
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}

#[cfg(test)]
impl Recording {
    /// A recording whose recorder runs, with a pid no process has.
    pub fn running(target: Target) -> Recording {
        Recording {
            shape: shape(&target.output),
            target,
            started: Instant::now(),
            pid: Some(i32::MAX as u32),
            pidfd: None,
            stopping: false,
            work: None,
            finish_by: None,
            killed: Arc::default(),
        }
    }

    /// A recording whose recorder, with a pid no process has, finishes its file.
    pub fn stopped(target: Target) -> Recording {
        Recording {
            stopping: true,
            ..Recording::running(target)
        }
    }
}

type Shape = (f64, smithay::utils::Transform);

fn shape(output: &Output) -> Shape {
    (
        output.current_scale().fractional_scale(),
        output.current_transform(),
    )
}

/// A recording's file as made: its path, and its device and inode, so a name swapped in the
/// folder since is never taken for it.
#[derive(Debug)]
struct Made {
    path: PathBuf,
    id: (u64, u64),
}

impl Made {
    fn of(file: &std::fs::File, path: PathBuf) -> std::io::Result<Made> {
        let m = file.metadata()?;
        Ok(Made {
            path,
            id: (m.dev(), m.ino()),
        })
    }

    /// The name's metadata when it is still the file made; blocking.
    fn metadata(&self) -> Option<std::fs::Metadata> {
        (std::fs::symlink_metadata(&self.path).ok()).filter(|m| (m.dev(), m.ino()) == self.id)
    }

    /// Unlink the name when it is still the file made; blocking.
    fn remove(&self) {
        if self.metadata().is_some() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// How a recorder run ended: its file and exit, or why it never started.
enum Ended {
    Exited {
        made: Made,
        ok: bool,
        status: Option<i32>,
        last_line: String,
        /// Killed while it finished the file.
        killed: bool,
        /// Nothing was written into the file.
        empty: bool,
    },
    NotStarted(String),
}

impl State {
    /// The bind: record the output under the pointer, or stop the recording running.
    pub fn toggle_screen_recording(&mut self) {
        if self.recording.is_some() {
            return self.stop_recording();
        }
        let p = self.pointer_location();
        let output = self
            .output_at(p.x, p.y)
            .or_else(|| self.scene.outputs().first().cloned());
        if let Some(output) = output {
            self.start_recording(Target { output, area: None });
        }
    }

    /// The region picker's Record mode picked `area` (layout coordinates): record it, clipped to
    /// the output holding most of it. A whole output's area records that output.
    pub fn record_area(&mut self, area: Rect) {
        if self.recording.is_some() {
            return;
        }
        let outputs: Vec<(Output, Rect)> = (self.scene.outputs().iter())
            .map(|o| (o.clone(), self.layout_box(Some(o))))
            .collect();
        let boxes: Vec<Rect> = outputs.iter().map(|(_, r)| *r).collect();
        let Some((i, local)) = pick_output(area, &boxes) else {
            return;
        };
        let whole = local == Rect::new(0, 0, boxes[i].width, boxes[i].height);
        self.start_recording(Target {
            output: outputs[i].0.clone(),
            area: (!whole).then_some(local),
        });
    }

    fn start_recording(&mut self, target: Target) {
        if self.recording.is_some() || self.desktop_locked() {
            return;
        }
        self.recording = Some(Recording {
            shape: shape(&target.output),
            target,
            started: Instant::now(),
            pid: None,
            pidfd: None,
            stopping: false,
            work: None,
            finish_by: None,
            killed: Arc::default(),
        });
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let queued = self.spawn_fs_job(move || {
            let made = make_file(now);
            Box::new(move |state: &mut State| state.recording_file_made(made))
        });
        if !queued {
            self.recording = None;
        }
        self.recording_changed();
    }

    /// Stop the recording: the recorder finishes its file and exits, or is killed and forgotten
    /// once it does nothing for [`STOP_GRACE`]. A stop while it finishes kills it now.
    pub fn stop_recording(&mut self) {
        let Some(r) = self.recording.as_mut() else {
            return;
        };
        if r.stopping {
            if let Some(pid) = r.pid {
                log::info!("{PROGRAM} (pid {pid}) stopped while finishing; killing it");
                self.kill_recorder();
            }
            return;
        }
        r.stopping = true;
        if let Some(fd) = &r.pidfd
            && let Err(e) = signal(fd, libc::SIGINT)
        {
            log::debug!("stopping {PROGRAM}: {e}");
        }
        let Some(pid) = r.pid else {
            return;
        };
        r.work = work(pid);
        let written = r.work.map_or(0, |w| w.written);
        r.finish_by = Some(Instant::now() + finish_cap(written));
        let timer = Timer::from_duration(STOP_GRACE);
        let armed =
            self.handle
                .insert_source(timer, move |_, _, state| match state.stop_overdue(pid) {
                    true => TimeoutAction::ToDuration(STOP_GRACE),
                    false => TimeoutAction::Drop,
                });
        if let Err(e) = armed {
            log::warn!("stopping {PROGRAM}: no timer: {}", e.error);
        }
        self.recording_changed();
    }

    /// The recorder `pid` was asked to stop and [`STOP_GRACE`] passed: true while it is waited
    /// for, else it is killed when it still runs and a new recording may start.
    fn stop_overdue(&mut self, pid: u32) -> bool {
        let Some(r) = (self.recording.as_mut()).filter(|r| r.stopping && r.pid == Some(pid)) else {
            return false;
        };
        let now = work(pid);
        let late = r.finish_by.is_some_and(|t| Instant::now() >= t);
        if !late && still_finishing(r.work, now) {
            log::info!("{PROGRAM} (pid {pid}) is still finishing the recording");
            r.work = now;
            return true;
        }
        match late {
            true => log::warn!("{PROGRAM} (pid {pid}) took too long to finish; killing it"),
            false => log::warn!("{PROGRAM} (pid {pid}) did not stop; killing it"),
        }
        self.kill_recorder();
        false
    }

    /// Kill the stopped recorder and forget it, so a new recording may start; its file is kept
    /// unless empty.
    fn kill_recorder(&mut self) {
        let Some(r) = self.recording.take() else {
            return;
        };
        r.killed.store(true, Ordering::SeqCst);
        if let Some(fd) = &r.pidfd
            && let Err(e) = signal(fd, libc::SIGKILL)
        {
            log::debug!("killing {PROGRAM}: {e}");
        }
        self.recording_changed();
    }

    fn recording_file_made(&mut self, made: Result<(std::fs::File, Made), String>) {
        let (file, made) = match made {
            Ok(m) => m,
            Err(e) => return self.recording_ended(None, Ended::NotStarted(e)),
        };
        let Some(r) = self.recording.as_ref() else {
            return remove_later(self, made);
        };
        if r.stopping {
            self.recording = None;
            self.recording_changed();
            return remove_later(self, made);
        }
        let argv = recorder_argv(
            file.as_raw_fd(),
            &r.target.output.name(),
            r.target.area,
            Heard::of(&self.config.screenshot),
        );
        let spawned = crate::spawn::spawn_waited(&argv, &self.env, Some(file.as_fd()));
        drop(file);
        let child = match spawned {
            Ok(Some(c)) => c,
            Ok(None) => {
                remove_later(self, made);
                return self.recording_ended(None, Ended::NotStarted(String::new()));
            }
            Err(e) => {
                remove_later(self, made);
                let why = format!("{PROGRAM} did not start: {e}.");
                return self.recording_ended(None, Ended::NotStarted(why));
            }
        };
        log::info!(
            "screen recording into {} (pid {})",
            made.path.display(),
            child.id()
        );
        self.grant_recorder(child.id());
        let pidfd = pidfd_open(child.id())
            .inspect_err(|e| log::warn!("no pidfd for {PROGRAM}: {e}"))
            .ok();
        let killed = Arc::new(AtomicBool::new(false));
        if let Some(r) = self.recording.as_mut() {
            r.pid = Some(child.id());
            r.pidfd = pidfd;
            r.started = Instant::now();
            r.killed = killed.clone();
        }
        self.recording_changed();
        let pid = child.id();
        self.spawn_job(move || {
            let ended = wait(child, made, &killed);
            Box::new(move |state: &mut State| state.recording_ended(Some(pid), ended))
        });
    }

    /// The recorder `pid` (None: none ran) ended; a recording started since `stop_overdue`
    /// forgot it runs on.
    fn recording_ended(&mut self, pid: Option<u32>, ended: Ended) {
        if self.recording.as_ref().is_some_and(|r| r.pid == pid) {
            self.recording = None;
            self.recording_changed();
        }
        match ended {
            Ended::Exited { made, ok: true, .. } => saved_notice(self, SAVED, &made.path, None),
            Ended::Exited {
                made,
                status: Some(STOPPED_UNSTARTED),
                ..
            }
            | Ended::Exited {
                made,
                status: None,
                killed: true,
                empty: true,
                ..
            } => {
                log::info!("screen recording stopped before anything was written");
                remove_later(self, made);
            }
            Ended::Exited {
                made,
                status: None,
                killed: true,
                ..
            } => {
                log::warn!("screen recording killed while finishing; its file is kept");
                saved_notice(self, NOT_FINISHED, &made.path, Some(MAY_NOT_PLAY));
            }
            Ended::Exited {
                made,
                status: Some(ENDED_EARLY),
                last_line,
                ..
            } => {
                log::warn!("screen recording ended early: {last_line}");
                let note = format!("Stopped early: {last_line}");
                saved_notice(self, SAVED, &made.path, Some(&note));
            }
            Ended::Exited {
                made,
                status: Some(SOUND_STOPPED),
                last_line,
                ..
            } => {
                log::warn!("screen recording lost its sound: {last_line}");
                let note = format!("The sound stopped: {last_line}");
                saved_notice(self, SAVED, &made.path, Some(&note));
            }
            Ended::Exited {
                made,
                status: Some(MICROPHONE_STOPPED),
                last_line,
                ..
            } => {
                log::warn!("screen recording lost its microphone: {last_line}");
                saved_notice(self, SAVED, &made.path, Some(&last_line));
            }
            Ended::Exited {
                made,
                status,
                last_line,
                ..
            } => {
                remove_later(self, made);
                let why = failed_why(status, last_line);
                log::warn!("screen recording failed: {why}");
                crate::notifyview::notice(self, "Screen recording failed", &why);
            }
            Ended::NotStarted(why) if why.is_empty() => {}
            Ended::NotStarted(why) => {
                log::warn!("screen recording not started: {why}");
                crate::notifyview::notice(self, "Screen recording failed", &why);
            }
        }
    }

    /// The tray cell, the border and the access trees follow a start or an end.
    fn recording_changed(&mut self) {
        self.redraw_taskbars();
        crate::captureborder::refresh(self);
        crate::access::touch(self);
    }
}

/// An area recording stops once its output's scale or transform changes: the recorder cuts the
/// area out of each frame at the pixels it took at the start.
pub fn outputs_changed(state: &mut State) {
    let moved = (state.recording.as_ref()).is_some_and(|r| {
        !r.stopping && r.target.area.is_some() && shape(&r.target.output) != r.shape
    });
    if moved {
        log::info!("the recorded area's screen changed scale or turned; stopping the recording");
        state.stop_recording();
    }
}

/// The recorded area as the capture border outlines it, while an area recording runs.
pub fn border_target(
    state: &State,
) -> Option<crate::captureborder::Captured<Output, crate::window::WindowId>> {
    let r = (state.recording.as_ref()).filter(|r| r.pid.is_some() && !r.stopping)?;
    let a = r.target.area?;
    let area = smithay::utils::Rectangle::new((a.x, a.y).into(), (a.width, a.height).into());
    Some(crate::captureborder::Captured::Region(
        r.target.output.clone(),
        area,
    ))
}

/// Whether a stopped recorder is finishing its file.
pub fn finishing(state: &State) -> bool {
    (state.recording.as_ref()).is_some_and(|r| r.stopping && r.pid.is_some())
}

/// The tray cell's tooltip, also its name for a screen reader.
pub fn tip(r: &Recording) -> String {
    match r.stopping {
        true => "Finishing the recording...\nClick to stop it now; the file may not play.".into(),
        false => running_tip(r.elapsed()),
    }
}

fn running_tip(elapsed: Duration) -> String {
    let s = elapsed.as_secs();
    let time = match s / 3600 {
        0 => format!("{}:{:02}", s / 60, s % 60),
        h => format!("{h}:{:02}:{:02}", s / 60 % 60, s % 60),
    };
    format!("Recording the screen, {time}\nClick to stop recording.")
}

/// The output whose layout box (one of `outputs`) holds most of `area`, and `area` clipped to it
/// in that output's coordinates. None when `area` misses every output.
pub fn pick_output(area: Rect, outputs: &[Rect]) -> Option<(usize, Rect)> {
    let (i, clip) = (outputs.iter().enumerate())
        .map(|(i, b)| (i, area.intersect(b)))
        .filter(|(_, c)| !c.is_empty())
        .max_by_key(|(_, c)| i64::from(c.width) * i64::from(c.height))?;
    let b = outputs[i];
    Some((
        i,
        Rect::new(clip.x - b.x, clip.y - b.y, clip.width, clip.height),
    ))
}

/// What a recording hears besides the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heard {
    Nothing,
    Played,
    /// What plays with the default source mixed in.
    PlayedAndMicrophone,
}

impl Heard {
    /// `record_microphone` mixes into the sound `record_sound` records, so alone it records none.
    pub fn of(shot: &repriseconf::config::ScreenshotConfig) -> Heard {
        match (shot.record_sound, shot.record_microphone) {
            (false, _) => Heard::Nothing,
            (true, false) => Heard::Played,
            (true, true) => Heard::PlayedAndMicrophone,
        }
    }
}

/// The recorder's command line; it writes only into descriptor `fd`, which it inherits. Only
/// [`Heard::PlayedAndMicrophone`] lets it open the microphone.
pub fn recorder_argv(fd: RawFd, output: &str, area: Option<Rect>, heard: Heard) -> Vec<OsString> {
    let mut argv: Vec<OsString> = vec![
        PROGRAM.into(),
        "--fd".into(),
        fd.to_string().into(),
        "--output".into(),
        output.into(),
    ];
    if let Some(a) = area {
        argv.push("--area".into());
        argv.push(format!("{},{},{},{}", a.x, a.y, a.width, a.height).into());
    }
    match heard {
        Heard::Nothing => {}
        Heard::Played => argv.push("--sound".into()),
        Heard::PlayedAndMicrophone => argv.extend(["--sound".into(), "--microphone".into()]),
    }
    argv
}

/// `pidfd_open(pid)`, for our unreaped child `pid`.
fn pidfd_open(pid: u32) -> std::io::Result<OwnedFd> {
    let pid = libc::pid_t::try_from(pid).map_err(|_| std::io::ErrorKind::InvalidInput)?;
    // SAFETY: pidfd_open takes a pid and flags, no pointers.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the kernel returned a new descriptor that nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd as RawFd) })
}

/// What `/proc` shows of a recorder at a look.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Work {
    /// CPU time in ticks.
    cpu: u64,
    /// Bytes read from storage.
    read: u64,
    /// Bytes written towards storage.
    written: u64,
    /// In uninterruptible sleep (state D), as on a network folder whose server is gone.
    blocked: bool,
}

/// Process `pid`'s [`Work`]; None once it is gone. Without `/proc/PID/io` (another user's
/// process) the bytes read 0.
fn work(pid: u32) -> Option<Work> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // State, utime and stime, fields 3, 14 and 15, after the name in parentheses, which may hold
    // spaces.
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let tick = |i: usize| fields.get(i)?.parse::<u64>().ok();
    let io = std::fs::read_to_string(format!("/proc/{pid}/io")).unwrap_or_default();
    let bytes = |key: &str| {
        (io.lines())
            .find_map(|l| l.strip_prefix(key)?.trim().parse::<u64>().ok())
            .unwrap_or(0)
    };
    Some(Work {
        cpu: tick(11)? + tick(12)?,
        read: bytes("read_bytes:"),
        written: bytes("write_bytes:"),
        blocked: fields.first() == Some(&"D"),
    })
}

/// Whether a stopped recorder seen as `now` (None: gone), and as `before` a look earlier, is
/// finishing its file: it used the CPU or its storage since, or waits on its storage.
fn still_finishing(before: Option<Work>, now: Option<Work>) -> bool {
    let Some(now) = now else {
        return false;
    };
    let moved = |b: Work| (b.cpu, b.read, b.written) != (now.cpu, now.read, now.written);
    now.blocked || before.is_none_or(moved)
}

/// The longest a recorder that has written `written` bytes is waited for after a stop: 2 min,
/// and 1 min more for each GiB.
fn finish_cap(written: u64) -> Duration {
    Duration::from_secs(120 + 60 * (written >> 30))
}

/// Signal `sig` to the process `pidfd` holds; `ESRCH` once it was reaped.
pub(crate) fn signal(pidfd: &OwnedFd, sig: libc::c_int) -> std::io::Result<()> {
    // SAFETY: a null siginfo is allowed and sends the signal as kill(2) would.
    let r = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            sig,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    match r {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    }
}

/// Forward the recorder's stderr to the log until it exits, keeping its last line: its reason
/// when it fails. Blocking.
fn wait(mut child: std::process::Child, made: Made, killed: &AtomicBool) -> Ended {
    let mut last_line = String::new();
    if let Some(err) = child.stderr.take() {
        for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
            let line = line.trim();
            if !line.is_empty() {
                log::info!("{PROGRAM}: {line}");
                last_line = line.to_string();
            }
        }
    }
    match child.wait() {
        Ok(s) => Ended::Exited {
            empty: made.metadata().is_some_and(|m| m.len() == 0),
            made,
            ok: s.success(),
            status: s.code(),
            last_line,
            killed: killed.load(Ordering::SeqCst),
        },
        Err(e) => Ended::NotStarted(format!("Waiting for {PROGRAM} failed: {e}.")),
    }
}

/// Why a recorder that ended with `status` (None: killed) and `last_line` on stderr failed. The
/// loader's own line for a library that did not load stays in the log.
fn failed_why(status: Option<i32>, last_line: String) -> String {
    match (status, last_line.is_empty()) {
        (Some(LIBRARY_MISSING), _) => "A library the recorder needs did not load.".to_string(),
        (_, false) => last_line,
        (Some(code), true) => format!("The recorder stopped with status {code}."),
        (None, true) => "The recorder was killed.".to_string(),
    }
}

/// Remove an unused or failed recording's file on a file job.
fn remove_later(state: &mut State, made: Made) {
    state.spawn_fs_job(move || {
        made.remove();
        Box::new(|_: &mut State| {})
    });
}

/// The notice for a recording kept at `path`, under `summary`, with a `note` below its name; the
/// file goes in the recent files.
fn saved_notice(state: &mut State, summary: &str, path: &Path, note: Option<&str>) {
    log::info!("screen recording saved to {}", path.display());
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let body = match note {
        Some(note) => format!("{name}\n{note}"),
        None => name.into_owned(),
    };
    let open = OwnAction::Open(path.to_path_buf());
    let folder = OwnAction::Run(reprisefs::apps::open_location_argv(path));
    let actions = vec![
        (
            crate::tray::notify::DEFAULT_ACTION.to_string(),
            String::new(),
            open.clone(),
        ),
        ("open".to_string(), "Open".to_string(), open),
        ("folder".to_string(), "Show in Folder".to_string(), folder),
    ];
    crate::notifyview::notice_with_actions(state, summary, &body, actions);
    state.record_recent(vec![path.to_path_buf()], None);
}

/// Make a new empty file named for `secs` in the Screencasts folder, made if missing, for the
/// recorder to write through the open file. Blocking.
fn make_file(secs: i64) -> Result<(std::fs::File, Made), String> {
    let videos = reprisefs::places::user_dir("VIDEOS", "Videos").filter(|p| p.is_dir());
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let dir = folder(videos, home).ok_or("There is no home folder to save it in.")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot make {}: {e}", dir.display()))?;
    let name = file_name(&reprisefs::locale::localtime(secs));
    let cannot = |e: std::io::Error| format!("Cannot write in {}: {e}", dir.display());
    let (file, path) = reprisefs::ops::create_new_numbered(&dir, &name).map_err(cannot)?;
    let made = Made::of(&file, path).map_err(cannot)?;
    Ok((file, made))
}

/// `Screencasts` in the Videos folder when there is one, else in home.
fn folder(videos: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    videos.or(home).map(|d| d.join("Screencasts"))
}

fn file_name(tm: &libc::tm) -> String {
    format!(
        "Screencast {:04}-{:02}-{:02} {:02}-{:02}-{:02}.mp4",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_recording_goes_in_the_recent_files_unless_they_are_off() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let dir = crate::harness::scratch().join("recording-recent");
        std::fs::create_dir_all(&dir).unwrap();
        let xbel = dir.join("recently-used.xbel");
        h.state.recent = crate::recentdocs::RecentDocs::reading(xbel.clone());
        let listed = |name: &str| std::fs::read_to_string(&xbel).is_ok_and(|t| t.contains(name));

        for name in ["Off.mp4", "Screencast On.mp4"] {
            std::fs::write(dir.join(name), "").unwrap();
        }

        h.state.config.recent_files = false;
        saved_notice(&mut h.state, SAVED, &dir.join("Off.mp4"), None);
        h.state.config.recent_files = true;
        saved_notice(&mut h.state, SAVED, &dir.join("Screencast On.mp4"), None);
        h.until("the recording is in the recent files", |_| {
            listed("Screencast%20On.mp4")
        });
        assert!(
            !listed("Off.mp4"),
            "nothing recorded while recent files are off"
        );
    }

    #[test]
    fn a_recorder_whose_library_did_not_load_says_so_in_place_of_the_loader_line() {
        let loader = "reprise-record: error while loading shared libraries: libjxl.so.0.11: \
                      cannot open shared object file: No such file or directory";
        assert_eq!(
            failed_why(Some(127), loader.into()),
            "A library the recorder needs did not load."
        );
        assert_eq!(failed_why(Some(1), "no output".into()), "no output");
        assert_eq!(
            failed_why(Some(1), String::new()),
            "The recorder stopped with status 1."
        );
        assert_eq!(failed_why(None, String::new()), "The recorder was killed.");
    }

    #[test]
    fn a_stop_interrupts_the_recorder_and_after_it_was_reaped_signals_nothing() {
        use std::os::unix::process::ExitStatusExt;
        let mut live = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        signal(&pidfd_open(live.id()).unwrap(), libc::SIGINT).unwrap();
        assert_eq!(live.wait().unwrap().signal(), Some(libc::SIGINT));
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let fd = pidfd_open(child.id()).unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        let e = signal(&fd, libc::SIGINT).unwrap_err();
        assert_eq!(e.raw_os_error(), Some(libc::ESRCH));
    }

    /// Waits until process `pid` ignores SIGINT.
    fn ignoring_int(pid: u32) {
        let ignores = || {
            let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
            let mask = status
                .lines()
                .find_map(|l| l.strip_prefix("SigIgn:"))
                .unwrap();
            u64::from_str_radix(mask.trim(), 16).unwrap() & (1 << (libc::SIGINT - 1)) != 0
        };
        let until = Instant::now() + Duration::from_secs(5);
        while !ignores() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_recorder_deaf_to_stop_is_killed_and_forgotten_after_the_grace() {
        use std::os::unix::process::ExitStatusExt;
        let mut h = crate::harness::Harness::new();
        let output = h.output("TEST-1", 1024, 768);
        let mut deaf = std::process::Command::new("sh")
            .args(["-c", "trap '' INT; sleep 30"])
            .spawn()
            .unwrap();
        let pid = deaf.id();
        let mut r = Recording::running(Target { output, area: None });
        r.pid = Some(pid);
        r.pidfd = Some(pidfd_open(pid).unwrap());
        let killed = r.killed.clone();
        h.state.recording = Some(r);
        ignoring_int(pid);
        // Past starting `sleep`, so the shell uses no CPU from here on.
        std::thread::sleep(Duration::from_millis(100));
        h.state.stop_recording();
        assert!(h.state.recording.is_some(), "it has time to finish");
        h.state.stop_overdue(pid);
        assert!(h.state.recording.is_none(), "forgotten");
        assert!(killed.load(Ordering::SeqCst), "its file is kept");
        assert_eq!(deaf.wait().unwrap().signal(), Some(libc::SIGKILL));
    }

    /// A recorder spinning on the CPU and deaf to SIGINT, as recording `h`'s first output.
    fn busy_recorder(h: &mut crate::harness::Harness) -> std::process::Child {
        let output = h.output("TEST-1", 1024, 768);
        let busy = std::process::Command::new("sh")
            .args(["-c", "trap '' INT; while :; do :; done"])
            .spawn()
            .unwrap();
        let pid = busy.id();
        ignoring_int(pid);
        let mut r = Recording::running(Target { output, area: None });
        r.pid = Some(pid);
        r.pidfd = Some(pidfd_open(pid).unwrap());
        h.state.recording = Some(r);
        busy
    }

    #[test]
    fn a_stopped_recorder_still_working_is_waited_for_until_its_cap() {
        use std::os::unix::process::ExitStatusExt;
        let mut h = crate::harness::Harness::new();
        let mut busy = busy_recorder(&mut h);
        let pid = busy.id();
        h.state.stop_recording();
        let r = h.state.recording.as_ref().unwrap();
        let cap = r.finish_by.unwrap() - Instant::now();
        assert!(cap > Duration::from_secs(119), "{cap:?}");
        let before = work(pid);
        let until = Instant::now() + Duration::from_secs(5);
        while work(pid) == before && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(h.state.stop_overdue(pid), "it used the CPU since the stop");
        assert!(h.state.recording.is_some());
        h.state.recording.as_mut().unwrap().finish_by = Some(Instant::now());
        assert!(!h.state.stop_overdue(pid), "past its cap");
        assert!(h.state.recording.is_none());
        assert_eq!(busy.wait().unwrap().signal(), Some(libc::SIGKILL));
    }

    #[test]
    fn a_second_stop_kills_a_finishing_recorder_and_keeps_its_file() {
        use std::os::unix::process::ExitStatusExt;
        let mut h = crate::harness::Harness::new();
        let mut busy = busy_recorder(&mut h);
        let killed = h.state.recording.as_ref().unwrap().killed.clone();
        h.state.stop_recording();
        assert!(finishing(&h.state));
        h.state.stop_recording();
        assert!(h.state.recording.is_none(), "forgotten");
        assert!(killed.load(Ordering::SeqCst));
        assert_eq!(busy.wait().unwrap().signal(), Some(libc::SIGKILL));
    }

    #[test]
    fn a_recorder_blocked_on_its_storage_is_waited_for_and_an_idle_one_is_not() {
        let idle = Work {
            cpu: 7,
            read: 0,
            written: 1 << 20,
            blocked: false,
        };
        let blocked = Work {
            blocked: true,
            ..idle
        };
        let writing = Work {
            written: 2 << 20,
            ..idle
        };
        assert!(!still_finishing(Some(idle), Some(idle)));
        assert!(still_finishing(Some(idle), Some(blocked)), "state D");
        assert!(still_finishing(Some(idle), Some(writing)));
        assert!(!still_finishing(Some(idle), None), "gone");
        assert_eq!(finish_cap(0), Duration::from_secs(120));
        assert_eq!(finish_cap(3 << 30), Duration::from_secs(300), "1 min a GiB");
    }

    #[test]
    fn a_recorder_killed_while_finishing_keeps_its_file_and_says_so() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let dir = std::env::temp_dir().join(format!("reprise-record-cut-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Screencast.mp4");
        std::fs::write(&path, "ftyp").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let ended = Ended::Exited {
            made: Made::of(&file, path.clone()).unwrap(),
            ok: false,
            status: None,
            last_line: String::new(),
            killed: true,
            empty: false,
        };
        h.state.recording_ended(None, ended);
        assert!(path.exists(), "kept");
        let n = &h.state.notify.log.get(crate::notifyview::OWN_ID).unwrap().n;
        assert_eq!(n.summary, "Screen recording not finished");
        assert_eq!(
            n.body,
            "Screencast.mp4\nIt was stopped while it closed the file and may not play."
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_recording_stopped_before_anything_was_written_leaves_no_file_and_no_notice() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let dir = std::env::temp_dir().join(format!("reprise-record-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (unstarted, killed) = (dir.join("a.mp4"), dir.join("b.mp4"));
        for (path, status, was_killed) in [
            (&unstarted, Some(STOPPED_UNSTARTED), false),
            (&killed, None, true),
        ] {
            let file = std::fs::File::create(path).unwrap();
            let ended = Ended::Exited {
                made: Made::of(&file, path.clone()).unwrap(),
                ok: false,
                status,
                last_line: String::new(),
                killed: was_killed,
                empty: true,
            };
            h.state.recording_ended(None, ended);
        }
        h.until("both files removed", |_| {
            !unstarted.exists() && !killed.exists()
        });
        assert!(h.state.notify.log.get(crate::notifyview::OWN_ID).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_border_leaves_an_area_whose_recording_is_finishing() {
        let mut h = crate::harness::Harness::new();
        let output = h.output("TEST-1", 1024, 768);
        h.state.recording = Some(Recording::running(Target {
            output,
            area: Some(Rect::new(10, 10, 200, 100)),
        }));
        assert!(border_target(&h.state).is_some());
        h.state.recording.as_mut().unwrap().stopping = true;
        assert!(border_target(&h.state).is_none());
    }

    #[test]
    fn a_recording_that_ended_early_keeps_its_file_and_says_why() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let dir = std::env::temp_dir().join(format!("reprise-record-early-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Screencast.mp4");
        let file = std::fs::File::create(&path).unwrap();
        let made = Made::of(&file, path.clone()).unwrap();
        let ended = Ended::Exited {
            made,
            ok: false,
            status: Some(ENDED_EARLY),
            last_line: "The screen changed size or format.".into(),
            killed: false,
            empty: false,
        };
        h.state.recording_ended(None, ended);
        assert!(path.exists(), "kept");
        let n = &h.state.notify.log.get(crate::notifyview::OWN_ID).unwrap().n;
        assert_eq!(n.summary, "Screen recording saved");
        assert_eq!(
            n.body,
            "Screencast.mp4\nStopped early: The screen changed size or format."
        );
        assert!(n.actions.iter().any(|(key, _)| key == "folder"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_recording_whose_sound_or_microphone_stopped_is_saved_and_says_why() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let dir = std::env::temp_dir().join(format!("reprise-record-sound-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Screencast.mp4");
        for (status, why, body) in [
            (
                SOUND_STOPPED,
                "The sink went away.",
                "Screencast.mp4\nThe sound stopped: The sink went away.",
            ),
            (
                MICROPHONE_STOPPED,
                "The microphone was disconnected; the rest of the recording has only what played.",
                "Screencast.mp4\nThe microphone was disconnected; the rest of the recording has \
                 only what played.",
            ),
            (
                MICROPHONE_STOPPED,
                "No microphone was found.",
                "Screencast.mp4\nNo microphone was found.",
            ),
        ] {
            let file = std::fs::File::create(&path).unwrap();
            let made = Made::of(&file, path.clone()).unwrap();
            let ended = Ended::Exited {
                made,
                ok: false,
                status: Some(status),
                last_line: why.into(),
                killed: false,
                empty: false,
            };
            h.state.recording_ended(None, ended);
            assert!(path.exists(), "kept");
            let n = &h.state.notify.log.get(crate::notifyview::OWN_ID).unwrap().n;
            assert_eq!(n.summary, "Screen recording saved");
            assert_eq!(n.body, body);
            assert!(n.actions.iter().any(|(key, _)| key == "folder"));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_area_recording_stops_when_its_screen_changes_scale() {
        use smithay::output::Scale;
        let mut h = crate::harness::Harness::new();
        let output = h.output("TEST-1", 1024, 768);
        let whole = Target {
            output: output.clone(),
            area: None,
        };
        h.state.recording = Some(Recording::running(whole.clone()));
        output.change_current_state(None, None, Some(Scale::Fractional(2.0)), None);
        h.state.update_layout();
        assert!(
            !h.state.recording.as_ref().unwrap().stopping,
            "a whole screen goes on"
        );
        h.state.recording = Some(Recording::running(Target {
            area: Some(Rect::new(10, 10, 200, 100)),
            ..whole
        }));
        output.change_current_state(None, None, Some(Scale::Fractional(1.5)), None);
        h.state.update_layout();
        assert!(h.state.recording.as_ref().unwrap().stopping);
        output.change_current_state(None, None, Some(Scale::Fractional(1.0)), None);
        h.state.update_layout();
        assert!(h.state.recording.is_some(), "a finishing one is left alone");
    }

    #[test]
    fn a_recording_is_named_for_its_start_in_screencasts_under_videos() {
        // SAFETY: all-zero is a valid libc::tm.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        (tm.tm_year, tm.tm_mon, tm.tm_mday) = (126, 10, 3);
        (tm.tm_hour, tm.tm_min, tm.tm_sec) = (9, 5, 7);
        assert_eq!(file_name(&tm), "Screencast 2026-11-03 09-05-07.mp4");
        let (videos, home) = (PathBuf::from("/h/Videos"), PathBuf::from("/h"));
        assert_eq!(
            folder(Some(videos), Some(home.clone())),
            Some(PathBuf::from("/h/Videos/Screencasts"))
        );
        assert_eq!(
            folder(None, Some(home)),
            Some(PathBuf::from("/h/Screencasts"))
        );
    }

    #[test]
    fn an_area_records_on_the_output_holding_most_of_it() {
        let outputs = [Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 1280, 1024)];
        assert_eq!(
            pick_output(Rect::new(1800, 100, 400, 300), &outputs),
            Some((1, Rect::new(0, 100, 280, 300))),
            "280 px on the right one beat 120 on the left"
        );
        assert_eq!(
            pick_output(Rect::new(100, 100, 50, 50), &outputs),
            Some((0, Rect::new(100, 100, 50, 50)))
        );
        assert_eq!(pick_output(Rect::new(0, 2000, 10, 10), &outputs), None);
    }

    #[test]
    fn the_recorder_hears_the_descriptor_the_output_the_area_and_the_sound() {
        let words = |a: Vec<OsString>| -> Vec<String> {
            a.iter().map(|s| s.to_string_lossy().into_owned()).collect()
        };
        assert_eq!(
            words(recorder_argv(7, "DP-1", None, Heard::Nothing)),
            [PROGRAM, "--fd", "7", "--output", "DP-1"]
        );
        assert_eq!(
            words(recorder_argv(
                7,
                "DP-1",
                Some(Rect::new(10, 20, 300, 200)),
                Heard::Played
            ))[5..],
            ["--area", "10,20,300,200", "--sound"]
        );
        assert_eq!(
            words(recorder_argv(7, "DP-1", None, Heard::PlayedAndMicrophone))[5..],
            ["--sound", "--microphone"]
        );
    }

    #[test]
    fn the_microphone_is_heard_only_with_both_keys_on() {
        let shot = |record_sound, record_microphone| repriseconf::config::ScreenshotConfig {
            record_sound,
            record_microphone,
            ..Default::default()
        };
        assert_eq!(Heard::of(&shot(false, false)), Heard::Nothing);
        assert_eq!(Heard::of(&shot(false, true)), Heard::Nothing);
        assert_eq!(Heard::of(&shot(true, false)), Heard::Played);
        assert_eq!(Heard::of(&shot(true, true)), Heard::PlayedAndMicrophone);
        assert_eq!(
            Heard::of(&Default::default()),
            Heard::Nothing,
            "off by default"
        );
    }

    #[test]
    fn a_failed_recording_removes_its_own_file_but_not_a_name_swapped_in() {
        let dir = std::env::temp_dir().join(format!("reprise-record-made-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (name, victim) = (dir.join("a.mp4"), dir.join("victim"));
        std::fs::write(&victim, "keep").unwrap();
        let (file, made) = reprisefs::ops::create_new_numbered(&dir, "a.mp4").unwrap();
        let made = Made::of(&file, made).unwrap();
        std::fs::remove_file(&name).unwrap();
        std::os::unix::fs::symlink(&victim, &name).unwrap();
        made.remove();
        assert!(
            name.symlink_metadata().unwrap().is_symlink(),
            "the planted link stays"
        );
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        std::fs::remove_file(&name).unwrap();
        let (file, made) = reprisefs::ops::create_new_numbered(&dir, "a.mp4").unwrap();
        Made::of(&file, made).unwrap().remove();
        assert!(!name.exists(), "its own file goes");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_tip_reads_the_elapsed_time_and_finishing_once_stopped() {
        assert_eq!(
            running_tip(Duration::from_secs(42)),
            "Recording the screen, 0:42\nClick to stop recording."
        );
        let long = running_tip(Duration::from_secs(3725));
        assert!(long.starts_with("Recording the screen, 1:02:05\n"));
        let mut h = crate::harness::Harness::new();
        let output = h.output("TEST-1", 1024, 768);
        let mut r = Recording::running(Target { output, area: None });
        r.stopping = true;
        assert_eq!(
            tip(&r),
            "Finishing the recording...\nClick to stop it now; the file may not play."
        );
    }
}
