//! The `screen-reader` action: starts Orca, or stops every copy this user runs. An OSD line says
//! which, and a sound marks it for a user who cannot see the line.
//!
//! The login screen, which starts no other program (`SessionEnv::refuses_programs`), starts
//! [`GREETER_READER`] alone: no arguments, an environment of its own variables
//! ([`greeter_reader_env`]) and one Wayland connection handed over as `WAYLAND_SOCKET`, since
//! it listens on no socket. It stops that one process on the next press and when it exits.

use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::Duration;

use reprisefs::screenreader::{self, PROGRAM};

use crate::compositor::{ClientState, Host, State};
use crate::media::Report;

/// The one program the login screen starts.
pub const GREETER_READER: &str = "/usr/bin/orca";

/// The variables of the login screen's own environment, which reprise-login's worker set from
/// passwd and PAM, that its reader gets; `LC_*` as well.
const GREETER_KEEPS: [&str; 11] = [
    "HOME",
    "USER",
    "LOGNAME",
    "PATH",
    "LANG",
    "LANGUAGE",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_SESSION_ID",
    "XDG_SEAT",
    "XDG_VTNR",
];

/// How long a switch waits for the reader it started or stopped to show in `/proc`.
const SETTLE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switched {
    On,
    Off,
    /// Orca is not installed, or would not start.
    Missing,
    /// Orca started but did not show running within [`SETTLE`].
    Failed,
}

impl Switched {
    pub fn title(self) -> &'static str {
        match self {
            Switched::On => "Screen reader on",
            Switched::Off => "Screen reader off",
            Switched::Missing => "No screen reader installed",
            Switched::Failed => "The screen reader did not start",
        }
    }

    /// XDG sound theme names.
    fn sound(self) -> &'static str {
        match self {
            Switched::On => "device-added",
            Switched::Off => "device-removed",
            Switched::Missing | Switched::Failed => "dialog-error",
        }
    }
}

/// What the action does: stop the running readers `running`, else start one when `installed`.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Stop(Vec<i32>),
    Start,
    Missing,
}

fn step(running: Vec<i32>, installed: bool) -> Step {
    match (running.is_empty(), installed) {
        (false, _) => Step::Stop(running),
        (true, true) => Step::Start,
        (true, false) => Step::Missing,
    }
}

impl State {
    /// Turn the screen reader on or off. The `/proc` walk and the spawn run off the loop, and the
    /// switch lasts until `/proc` shows it; a press meanwhile is dropped.
    pub fn toggle_screen_reader(&mut self) {
        if self.host == Host::Greeter {
            self.toggle_greeter_reader(Path::new(GREETER_READER));
            return;
        }
        if self.screen_reader_busy {
            return;
        }
        self.screen_reader_busy = true;
        let env = self.env.clone();
        let spawned = self.spawn_job(move || {
            let switched = match step(screenreader::running(), screenreader::installed()) {
                Step::Stop(pids) => {
                    screenreader::stop(&pids);
                    screenreader::wait_for(false, SETTLE);
                    Switched::Off
                }
                Step::Start => {
                    match crate::spawn::try_spawn_argv_in(
                        &[PROGRAM],
                        Some(PROGRAM),
                        None,
                        &env,
                        None,
                    ) {
                        Ok(Some(_)) => match screenreader::wait_for(true, SETTLE) {
                            true => Switched::On,
                            false => Switched::Failed,
                        },
                        Ok(None) => Switched::Missing,
                        Err(e) => {
                            log::warn!("screen reader: {e}");
                            Switched::Missing
                        }
                    }
                }
                Step::Missing => Switched::Missing,
            };
            log::info!("screen reader: {}", switched.title());
            Box::new(move |state: &mut State| state.screen_reader_switched(switched))
        });
        if !spawned {
            log::warn!("screen reader: no thread to switch it");
            self.screen_reader_busy = false;
        }
    }

    fn screen_reader_switched(&mut self, switched: Switched) {
        self.screen_reader_busy = false;
        let theme = self.config.sounds.theme.clone();
        repriseaudio::sounds::play(
            repriseaudio::sounds::Sound::Name(switched.sound().into()),
            theme,
        );
        self.show_osd(&Report::ScreenReader(switched));
    }

    /// At the login screen: stop the reader it started, else start `program` when installed.
    /// Without one nothing happens. The switch shows its OSD line and plays no sound, which
    /// would start a player.
    fn toggle_greeter_reader(&mut self, program: &Path) {
        if self.stop_greeter_reader() {
            log::info!("screen reader: {}", Switched::Off.title());
            self.show_osd(&Report::ScreenReader(Switched::Off));
            return;
        }
        let installed =
            (program.metadata()).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
        if !installed {
            log::info!("screen reader: no {}", program.display());
            return;
        }
        match self.start_greeter_reader(program) {
            Ok(child) => {
                log::info!("screen reader: {} ({})", Switched::On.title(), child.id());
                self.greeter.reader = Some(child);
                self.show_osd(&Report::ScreenReader(Switched::On));
            }
            Err(e) => log::warn!("screen reader: {}: {e}", program.display()),
        }
    }

    /// `program` as a child of the login screen with its own Wayland connection, a sandboxed
    /// client, which ends with the login screen should it die first.
    fn start_greeter_reader(&mut self, program: &Path) -> io::Result<Child> {
        let (ours, theirs) = UnixStream::pair()?;
        let data = ClientState::of_stream(
            &ours,
            Some(crate::sandbox::Sandbox {
                engine: Some("reprise-login".into()),
                app_id: Some(PROGRAM.into()),
            }),
        );
        let client = (self.dh.insert_client(ours, Arc::new(data))).map_err(io::Error::other)?;
        let fd = theirs.as_raw_fd();
        let own = std::env::vars_os()
            .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)));
        let mut c = Command::new(program);
        c.env_clear().envs(greeter_reader_env(own, fd));
        crate::spawn::detach(&mut c);
        let parent = std::process::id();
        // SAFETY: runs between fork and exec; fcntl, prctl and getppid are async-signal-safe and
        // nothing allocates.
        unsafe {
            c.pre_exec(move || {
                if libc::fcntl(fd, libc::F_SETFD, 0) != 0
                    || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0
                {
                    return Err(io::Error::last_os_error());
                }
                // The login screen died before the death signal was set.
                if libc::getppid() as u32 != parent {
                    return Err(io::Error::from_raw_os_error(libc::ESRCH));
                }
                Ok(())
            });
        }
        let spawned = c.spawn();
        drop(theirs);
        if spawned.is_err() {
            self.drop_client(client.id());
        }
        spawned
    }

    /// Reap the login screen's reader once it exited on its own.
    pub fn reap_greeter_reader(&mut self) {
        let Some(child) = self.greeter.reader.as_mut() else {
            return;
        };
        match child.try_wait() {
            Ok(None) => {}
            Ok(Some(status)) => {
                log::warn!("screen reader {}: {status}", child.id());
                self.greeter.reader = None;
            }
            Err(e) => {
                log::warn!("screen reader {}: {e}", child.id());
                self.greeter.reader = None;
            }
        }
    }

    /// Ask the reader the login screen started to quit; false when none runs.
    pub fn stop_greeter_reader(&mut self) -> bool {
        let Some(mut child) = self.greeter.reader.take() else {
            return false;
        };
        if !matches!(child.try_wait(), Ok(None)) {
            return false;
        }
        let pid = child.id() as libc::pid_t;
        // SAFETY: kill only sends a signal; the child is not reaped yet, so `pid` is still it.
        if unsafe { libc::kill(pid, libc::SIGTERM) } != 0 {
            log::warn!("screen reader {pid}: {}", io::Error::last_os_error());
        }
        crate::spawn::adopt(pid, PROGRAM);
        true
    }
}

/// The login screen reader's whole environment: [`GREETER_KEEPS`] and `LC_*` of `own`, and a
/// Wayland session reached through `socket`.
fn greeter_reader_env(
    own: impl Iterator<Item = (String, String)>,
    socket: RawFd,
) -> Vec<(String, String)> {
    let kept = own.filter(|(k, _)| GREETER_KEEPS.contains(&k.as_str()) || k.starts_with("LC_"));
    let given = [
        ("WAYLAND_SOCKET", socket.to_string()),
        ("XDG_SESSION_TYPE", "wayland".into()),
        ("XDG_CURRENT_DESKTOP", "reprise".into()),
        ("GDK_BACKEND", "wayland".into()),
        // Without WAYLAND_DISPLAY libatspi would watch keys through X11, which isn't there.
        ("ATSPI_USE_LEGACY_DEVICE", "1".into()),
    ];
    kept.chain(given.into_iter().map(|(k, v)| (k.to_owned(), v)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_while_a_switch_runs_is_dropped() {
        let mut h = crate::harness::Harness::new();
        crate::fsjobs::setup(&mut h.state);
        h.state.fsjobs.refuse_threads = true;
        h.state.toggle_screen_reader();
        assert!(
            !h.state.screen_reader_busy,
            "a refused thread frees the key"
        );
        h.state.screen_reader_busy = true;
        h.state.toggle_screen_reader();
        assert!(h.state.screen_reader_busy, "no second switch was tried");
    }

    #[test]
    fn the_login_screen_starts_its_reader_with_one_wayland_connection_and_stops_it() {
        let dir = crate::harness::scratch().join("greeter-reader");
        std::fs::create_dir_all(&dir).unwrap();
        let reader = dir.join("reader");
        std::fs::write(
            &reader,
            "#!/bin/sh\nreadlink /proc/$$/fd/$WAYLAND_SOCKET > \"$0.fd\"\nenv > \"$0.env\"\nexec sleep 30\n",
        )
        .unwrap();
        std::fs::set_permissions(&reader, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut h = crate::harness::Harness::with_host(Default::default(), Host::Greeter);
        h.output("TEST-1", 1024, 768);

        h.state.toggle_greeter_reader(&dir.join("missing"));
        assert!(h.state.greeter.reader.is_none(), "nothing to start");
        assert!(!h.state.osd.is_up(), "and nothing to say");

        h.state.toggle_greeter_reader(&reader);
        let pid = h.state.greeter.reader.as_ref().expect("started").id() as libc::pid_t;
        assert!(h.state.osd.is_up(), "the switch shows its line");
        let env_file = dir.join("reader.env");
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while !std::fs::read_to_string(&env_file).is_ok_and(|e| e.ends_with('\n')) {
            assert!(std::time::Instant::now() < end, "the reader ran");
            std::thread::sleep(Duration::from_millis(20));
        }
        let fd = std::fs::read_to_string(dir.join("reader.fd")).unwrap();
        assert!(fd.starts_with("socket:"), "{fd}");
        let env = std::fs::read_to_string(&env_file).unwrap();
        assert!(env.contains("GDK_BACKEND=wayland\n"), "{env}");
        assert!(!env.contains("WAYLAND_DISPLAY="), "{env}");

        h.state.toggle_greeter_reader(&reader);
        assert!(h.state.greeter.reader.is_none(), "stopped");
        let end = std::time::Instant::now() + Duration::from_secs(5);
        // SAFETY: kill with signal 0 only checks; the adopted pid is reaped below.
        while unsafe { libc::kill(pid, 0) } == 0 {
            assert!(std::time::Instant::now() < end, "the reader quit");
            crate::spawn::reap_children();
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn the_login_screens_reader_keeps_only_its_own_variables_and_a_wayland_socket() {
        let own = [
            ("HOME", "/var/lib/reprise-greeter"),
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/960/bus"),
            ("LC_TIME", "de_DE.UTF-8"),
            ("WAYLAND_DISPLAY", "wayland-1"),
            ("DISPLAY", ":0"),
            ("LD_PRELOAD", "/tmp/x.so"),
        ];
        let env = greeter_reader_env(
            own.into_iter().map(|(k, v)| (k.to_owned(), v.to_owned())),
            7,
        );
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            [
                "HOME",
                "DBUS_SESSION_BUS_ADDRESS",
                "LC_TIME",
                "WAYLAND_SOCKET",
                "XDG_SESSION_TYPE",
                "XDG_CURRENT_DESKTOP",
                "GDK_BACKEND",
                "ATSPI_USE_LEGACY_DEVICE"
            ]
        );
        assert!(env.contains(&("WAYLAND_SOCKET".into(), "7".into())));
    }

    #[test]
    fn a_running_reader_is_stopped_else_one_starts_where_installed() {
        assert_eq!(step(vec![12, 40], true), Step::Stop(vec![12, 40]));
        assert_eq!(step(vec![12], false), Step::Stop(vec![12]));
        assert_eq!(step(Vec::new(), true), Step::Start);
        assert_eq!(step(Vec::new(), false), Step::Missing);
    }
}
