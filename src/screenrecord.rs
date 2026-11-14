//! Screen recordings: `reprise-record` writes an output, or an area of it, to a new MP4 under
//! Videos/Screencasts while the tray's Recording cell shows (docs/features.md, Screenshots).

use std::ffi::OsString;
use std::io::BufRead;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use smithay::output::Output;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::State;
use crate::notifyview::OwnAction;
use crate::raster::Rect;

const PROGRAM: &str = "reprise-record";
/// How long a recorder has to finish its file after Stop before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(5);

/// What a recording records: an output, or an area of it in output-local logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub output: Output,
    pub area: Option<Rect>,
}

pub struct Recording {
    pub target: Target,
    started: Instant,
    /// The recorder, once its file is made and it runs.
    pid: Option<u32>,
    /// The recorder's pidfd, which Stop signals: its `wait` job may reap it before the loop
    /// hears, and a reaped pid can name another process.
    pidfd: Option<OwnedFd>,
    /// A stop asked for before the recorder ran.
    stopping: bool,
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
            target,
            started: Instant::now(),
            pid: Some(i32::MAX as u32),
            pidfd: None,
            stopping: false,
        }
    }
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

    /// Unlink the name when it is still the file made; blocking.
    fn remove(&self) {
        let same =
            std::fs::symlink_metadata(&self.path).is_ok_and(|m| (m.dev(), m.ino()) == self.id);
        if same {
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
            target,
            started: Instant::now(),
            pid: None,
            pidfd: None,
            stopping: false,
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
    /// after [`STOP_GRACE`].
    pub fn stop_recording(&mut self) {
        let Some(r) = self.recording.as_mut() else {
            return;
        };
        let first = !std::mem::replace(&mut r.stopping, true);
        if let Some(fd) = &r.pidfd
            && let Err(e) = signal(fd, libc::SIGINT)
        {
            log::debug!("stopping {PROGRAM}: {e}");
        }
        let Some(pid) = r.pid.filter(|_| first) else {
            return;
        };
        let timer = Timer::from_duration(STOP_GRACE);
        let armed = self.handle.insert_source(timer, move |_, _, state| {
            state.stop_overdue(pid);
            TimeoutAction::Drop
        });
        if let Err(e) = armed {
            log::warn!("stopping {PROGRAM}: no timer: {}", e.error);
        }
    }

    /// The recorder `pid` was asked to stop [`STOP_GRACE`] ago: kill it when it still runs, and
    /// let a new recording start.
    fn stop_overdue(&mut self, pid: u32) {
        let Some(r) = self.recording.take_if(|r| r.pid == Some(pid)) else {
            return;
        };
        log::warn!("{PROGRAM} (pid {pid}) did not stop; killing it");
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
            self.config.screenshot.record_sound,
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
        if let Some(r) = self.recording.as_mut() {
            r.pid = Some(child.id());
            r.pidfd = pidfd;
            r.started = Instant::now();
        }
        self.recording_changed();
        let pid = child.id();
        self.spawn_job(move || {
            let ended = wait(child, made);
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
            Ended::Exited { made, ok: true, .. } => saved_notice(self, &made.path),
            Ended::Exited {
                made,
                status,
                last_line,
                ..
            } => {
                remove_later(self, made);
                let why = match (last_line.is_empty(), status) {
                    (false, _) => last_line,
                    (true, Some(code)) => format!("The recorder stopped with status {code}."),
                    (true, None) => "The recorder was killed.".to_string(),
                };
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

/// The recorded area as the capture border outlines it, while an area recording runs.
pub fn border_target(
    state: &State,
) -> Option<crate::captureborder::Captured<Output, crate::window::WindowId>> {
    let r = state.recording.as_ref().filter(|r| r.pid.is_some())?;
    let a = r.target.area?;
    let area = smithay::utils::Rectangle::new((a.x, a.y).into(), (a.width, a.height).into());
    Some(crate::captureborder::Captured::Region(
        r.target.output.clone(),
        area,
    ))
}

/// The tray cell's tooltip, also its name for a screen reader.
pub fn tip(elapsed: Duration) -> String {
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

/// The recorder's command line; it writes only into descriptor `fd`, which it inherits.
pub fn recorder_argv(fd: RawFd, output: &str, area: Option<Rect>, sound: bool) -> Vec<OsString> {
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
    if sound {
        argv.push("--sound".into());
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

/// Signal `sig` to the process `pidfd` holds; `ESRCH` once it was reaped.
fn signal(pidfd: &OwnedFd, sig: libc::c_int) -> std::io::Result<()> {
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
fn wait(mut child: std::process::Child, made: Made) -> Ended {
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
            made,
            ok: s.success(),
            status: s.code(),
            last_line,
        },
        Err(e) => Ended::NotStarted(format!("Waiting for {PROGRAM} failed: {e}.")),
    }
}

/// Remove an unused or failed recording's file on a file job.
fn remove_later(state: &mut State, made: Made) {
    state.spawn_fs_job(move || {
        made.remove();
        Box::new(|_: &mut State| {})
    });
}

fn saved_notice(state: &mut State, path: &Path) {
    log::info!("screen recording saved to {}", path.display());
    let name = path.file_name().unwrap_or_default().to_string_lossy();
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
    crate::notifyview::notice_with_actions(state, "Screen recording saved", &name, actions);
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
        h.state.recording = Some(r);
        let ignores_int = || {
            let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
            let mask = status
                .lines()
                .find_map(|l| l.strip_prefix("SigIgn:"))
                .unwrap();
            u64::from_str_radix(mask.trim(), 16).unwrap() & (1 << (libc::SIGINT - 1)) != 0
        };
        let until = Instant::now() + Duration::from_secs(5);
        while !ignores_int() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        h.state.stop_recording();
        assert!(h.state.recording.is_some(), "it has time to finish");
        h.state.stop_overdue(pid);
        assert!(h.state.recording.is_none(), "forgotten");
        assert_eq!(deaf.wait().unwrap().signal(), Some(libc::SIGKILL));
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
            words(recorder_argv(7, "DP-1", None, false)),
            [PROGRAM, "--fd", "7", "--output", "DP-1"]
        );
        assert_eq!(
            words(recorder_argv(
                7,
                "DP-1",
                Some(Rect::new(10, 20, 300, 200)),
                true
            ))[5..],
            ["--area", "10,20,300,200", "--sound"]
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
    fn the_tip_reads_the_elapsed_time() {
        assert_eq!(
            tip(Duration::from_secs(42)),
            "Recording the screen, 0:42\nClick to stop recording."
        );
        assert!(tip(Duration::from_secs(3725)).starts_with("Recording the screen, 1:02:05\n"));
    }
}
