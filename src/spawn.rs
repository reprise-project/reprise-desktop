//! Launch child processes with the compositor's display sockets exported.

use std::os::unix::process::CommandExt;
use std::sync::Mutex;

/// Children started here and still to be reaped, by pid and log name.
///
/// Not a `SIGCHLD`/`SIG_IGN` disposition: it is inherited across exec, and Xwayland waits on
/// `xkbcomp`, which fails with `ECHILD` when children are auto-reaped. `reap_children` reaps
/// our own pids instead.
static CHILDREN: Mutex<Vec<(libc::pid_t, String)>> = Mutex::new(Vec::new());

const TERM_SIGNALS: [libc::c_int; 2] = [libc::SIGTERM, libc::SIGINT];

/// Block `SIGCHLD`, `SIGTERM` and `SIGINT` before any thread exists so only the loop's
/// signalfds see them; `start` clears the mask in children.
pub fn block_signals() {
    // SAFETY: `set` is a local sigset_t, all-zero is a valid value for it,
    // and every call gets a pointer to it or null.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for s in std::iter::once(libc::SIGCHLD).chain(TERM_SIGNALS) {
            libc::sigaddset(&mut set, s);
        }
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
}

/// Run `f` with the `block_signals` signals unblocked on this thread, for a child another crate
/// forks (Xwayland): it must not inherit them blocked. A `SIGCHLD` meanwhile is reaped on the
/// next one; a `SIGTERM` or `SIGINT` meanwhile ends the compositor without its signalfd.
pub fn with_signals_unblocked<T>(f: impl FnOnce() -> T) -> T {
    // SAFETY: as in `block_signals`.
    let mask = |how| unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for s in std::iter::once(libc::SIGCHLD).chain(TERM_SIGNALS) {
            libc::sigaddset(&mut set, s);
        }
        libc::pthread_sigmask(how, &set, std::ptr::null_mut());
    };
    mask(libc::SIG_UNBLOCK);
    let r = f();
    mask(libc::SIG_BLOCK);
    r
}

/// A signalfd readable on SIGCHLD (blocked by `block_signals`).
pub fn sigchld_fd() -> std::io::Result<std::os::fd::OwnedFd> {
    signal_fd(&[libc::SIGCHLD])
}

/// A signalfd readable on SIGTERM or SIGINT (`read_signal` names which).
pub fn term_fd() -> std::io::Result<std::os::fd::OwnedFd> {
    signal_fd(&TERM_SIGNALS)
}

pub fn read_signal(fd: &std::os::fd::OwnedFd) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let mut info = std::mem::MaybeUninit::<libc::signalfd_siginfo>::uninit();
    let size = std::mem::size_of::<libc::signalfd_siginfo>();
    // SAFETY: `info` is a writable buffer of `size` bytes and `fd` is a
    // live non-blocking signalfd; a full read initialises it.
    let n = unsafe { libc::read(fd.as_raw_fd(), info.as_mut_ptr().cast(), size) };
    // SAFETY: the kernel wrote a whole `signalfd_siginfo`.
    (n == size as isize).then(|| unsafe { info.assume_init() }.ssi_signo)
}

fn signal_fd(signals: &[libc::c_int]) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    // SAFETY: `set` is a local sigset_t, all-zero is a valid value for it;
    // signalfd gets a pointer to it and returns a new fd or -1.
    let fd = unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for &s in signals {
            libc::sigaddset(&mut set, s);
        }
        libc::signalfd(-1, &set, libc::SFD_NONBLOCK | libc::SFD_CLOEXEC)
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` is a fresh descriptor nothing else owns.
    Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) })
}

/// Read every pending signal off a signalfd so it stops being readable.
pub fn drain_signalfd(fd: &std::os::fd::OwnedFd) {
    use std::os::fd::AsRawFd;
    let mut info = std::mem::MaybeUninit::<libc::signalfd_siginfo>::uninit();
    let size = std::mem::size_of::<libc::signalfd_siginfo>();
    // SAFETY: `info` is a writable buffer of `size` bytes and `fd` is a
    // live non-blocking signalfd; the loop ends on EAGAIN or a short read.
    while unsafe { libc::read(fd.as_raw_fd(), info.as_mut_ptr().cast(), size) } == size as isize {}
}

/// What every child and the activation environment get. Kept out of the process environment:
/// worker threads read `getenv` and `setenv` is not synchronised against them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionEnv {
    pub wayland_display: String,
    pub x11_display: Option<String>,
    /// `XDG_CURRENT_DESKTOP`: xdg-desktop-portal's `<desktop>-portals.conf` key and the
    /// autostart filter. A TTY start leaves it unset, so it is filled in at startup.
    pub desktop: String,
    pub colors: String,
    pub interface: reprisekit::interface::Interface,
    pub blue_variant: reprisekit::blue::Variant,
    pub glass_mode: reprisekit::glass::Mode,
    pub glass_material: reprisekit::glass::Material,
    pub icons: String,
    /// `XCURSOR_THEME` and `XCURSOR_SIZE`: the `[cursor]` theme and size, so clients draw the
    /// same pointer.
    pub cursor: (String, u32),
    /// `[toolkits] qt_platform_theme`, resolved per launch against the colours
    /// (`crate::config::resolve_qt`). Children only: a switch back to light leaves nothing to
    /// unset in the activation environment.
    pub qt_platform_theme: String,
    /// The Qt tool that took the generated palette (`crate::toolkits`), which "reprise"
    /// resolves to.
    pub qt_generated: Option<&'static str>,
    pub renderer: &'static str,
    pub login: Vec<(&'static str, String)>,
    /// Nested in another session (`WAYLAND_DISPLAY` or `DISPLAY` set at startup): that
    /// session's values of the published variables, put back at the end. Empty otherwise.
    pub parent: Vec<(&'static str, String)>,
    /// The logind session a display manager started Reprise as, which ends with it. None for a
    /// TTY start or nested run.
    pub logind_session: Option<String>,
}

/// What the login manager and pam_systemd set that services outside our process tree still need.
const FORWARDED: [&str; 3] = ["XDG_SESSION_DESKTOP", "XDG_SEAT", "XDG_SEAT_PATH"];

impl SessionEnv {
    /// Read at startup, before any worker thread exists.
    pub fn new(
        wayland_display: String,
        colors: &reprisekit::theme::ColorScheme,
        interface: reprisekit::interface::Interface,
        blue_variant: reprisekit::blue::Variant,
        (glass_mode, glass_material): (reprisekit::glass::Mode, reprisekit::glass::Material),
        icons: &str,
        cursor: (String, u32),
    ) -> SessionEnv {
        SessionEnv {
            wayland_display,
            x11_display: None,
            desktop: std::env::var("XDG_CURRENT_DESKTOP")
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "reprise".into()),
            colors: colors.to_env(),
            interface,
            blue_variant,
            glass_mode,
            glass_material,
            icons: icons.into(),
            cursor,
            qt_platform_theme: crate::config::TOOLKIT_AUTO.into(),
            qt_generated: None,
            renderer: "",
            login: FORWARDED
                .into_iter()
                .filter_map(|k| Some((k, std::env::var(k).ok().filter(|v| !v.is_empty())?)))
                .collect(),
            parent: parent_session(|k| std::env::var(k).ok().filter(|v| !v.is_empty())),
            logind_session: own_session(|k| std::env::var(k).ok().filter(|v| !v.is_empty())),
        }
    }

    /// Take the look that launched programs read. Returns whether any changed, so the session
    /// bus needs them.
    pub fn set_look(
        &mut self,
        colors: String,
        interface: reprisekit::interface::Interface,
        blue_variant: reprisekit::blue::Variant,
        (glass_mode, glass_material): (reprisekit::glass::Mode, reprisekit::glass::Material),
        icons: &str,
        cursor: (String, u32),
    ) -> bool {
        if colors == self.colors
            && interface == self.interface
            && blue_variant == self.blue_variant
            && glass_mode == self.glass_mode
            && glass_material == self.glass_material
            && icons == self.icons
            && cursor == self.cursor
        {
            return false;
        }
        self.colors = colors;
        self.interface = interface;
        self.blue_variant = blue_variant;
        self.glass_mode = glass_mode;
        self.glass_material = glass_material;
        self.icons = icons.into();
        self.cursor = cursor;
        true
    }

    fn pairs(&self) -> Vec<(&'static str, String)> {
        let mut v = vec![
            ("WAYLAND_DISPLAY", self.wayland_display.clone()),
            ("XDG_CURRENT_DESKTOP", self.desktop.clone()),
            // Always Wayland: a TTY start has pam_systemd set "tty".
            ("XDG_SESSION_TYPE", "wayland".into()),
            (reprisekit::theme::COLORS_ENV, self.colors.clone()),
            (reprisekit::interface::ENV, self.interface.as_str().into()),
            (reprisekit::blue::ENV, self.blue_variant.as_str().into()),
            (reprisekit::glass::MODE_ENV, self.glass_mode.as_str().into()),
            (
                reprisekit::glass::MATERIAL_ENV,
                self.glass_material.as_str().into(),
            ),
            (reprisekit::icontheme::ENV, self.icons.clone()),
            ("XCURSOR_THEME", self.cursor.0.clone()),
            ("XCURSOR_SIZE", self.cursor.1.to_string()),
            (reprisefs::sysinfo::RENDERER_ENV, self.renderer.into()),
            // GTK3 opens its file dialogs through the portal.
            ("GTK_USE_PORTAL", "1".into()),
        ];
        if let Some(d) = &self.x11_display {
            v.push(("DISPLAY", d.clone()));
        }
        v.extend(self.login.iter().cloned());
        v
    }

    fn dark(&self) -> bool {
        reprisekit::theme::ColorScheme::from_env(&self.colors).is_some_and(|c| c.is_dark())
    }

    fn qt_platform_theme(&self) -> Option<&str> {
        crate::config::resolve_qt(&self.qt_platform_theme, self.dark(), self.qt_generated)
    }
}

/// Publish the display sockets to the systemd user manager and D-Bus activation environment, so
/// bus-activated services find the compositor. [`begin_session`] publishes first; this runs when
/// `DISPLAY` is known and when colours change. Best effort.
pub fn export_session_env(env: &SessionEnv) {
    let argv = export_argv(env);
    let mut c = std::process::Command::new(&argv[0]);
    c.args(&argv[1..]);
    // Not waited on: the tool round-trips the bus, and colour changes call this from the event
    // loop.
    if let Err(e) = start(c, "dbus-update-activation-environment", None) {
        log::warn!("dbus-update-activation-environment: {e}; portals may not find the display");
    }
}

fn export_argv(env: &SessionEnv) -> Vec<String> {
    let mut argv = vec![
        "dbus-update-activation-environment".to_string(),
        "--systemd".into(),
    ];
    argv.extend(env.pairs().into_iter().map(|(k, v)| format!("{k}={v}")));
    argv
}

/// Reprise's session target (data/systemd). It binds `graphical-session.target`, so units wanted
/// by or part of that target run with the session.
const SESSION_TARGET: &str = "reprise-session.target";

/// The commands that open the session `env` describes, in order: publish the variables, stop the
/// target a crashed session left active (its units hold the old display), start the target. A
/// nested run only publishes: `graphical-session.target` is the parent's.
pub fn begin_commands(env: &SessionEnv) -> Vec<Vec<String>> {
    let mut out = vec![export_argv(env)];
    if env.parent.is_empty() {
        // `graphical-session.target` by name: it stops when unneeded only after the stop job,
        // so the start that follows would keep it and its units running.
        let stop = ["stop", SESSION_TARGET, "graphical-session.target"];
        let start = ["--no-block", "start", SESSION_TARGET];
        for args in [&stop[..], &start[..]] {
            let mut c = vec!["systemctl".to_string(), "--user".into()];
            c.extend(args.iter().map(|a| a.to_string()));
            out.push(c);
        }
    }
    out
}

/// Run `begin_commands` in order on a thread of its own: each waits for the last, so the
/// target's units start with the variables published.
pub fn begin_session(env: &SessionEnv) {
    let cmds = begin_commands(env);
    std::thread::spawn(move || {
        for argv in cmds {
            let mut c = std::process::Command::new(&argv[0]);
            c.args(&argv[1..]);
            detach(&mut c);
            match c.status() {
                Ok(st) if !st.success() => log::warn!("{argv:?} exited with {st}"),
                Ok(_) => {}
                Err(e) => log::warn!("{argv:?}: {e}"),
            }
        }
    });
}

/// The GTK portal backend's unit; it draws GTK3's print dialog (`Print=gtk` in
/// reprise-portals.conf).
const PORTAL_GTK_UNIT: &str = "xdg-desktop-portal-gtk.service";

/// Hand xdg-desktop-portal-gtk `theme` so its print dialog looks like the program. The backend
/// reads GSettings, which Reprise leaves alone; `GTK_THEME` outranks that, via a drop-in under
/// `$XDG_RUNTIME_DIR`. A running backend restarts to read it. No-op when the drop-in already
/// says `theme` or on a nested run.
pub fn theme_portal_gtk(env: &SessionEnv, theme: &str) {
    if !env.parent.is_empty() {
        return;
    }
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") else {
        return;
    };
    match write_portal_gtk_drop_in(std::path::Path::new(&runtime), theme) {
        Ok(false) => {}
        Ok(true) => {
            let mut c = std::process::Command::new("sh");
            c.arg("-c").arg(format!(
                "systemctl --user daemon-reload && systemctl --user --no-block try-restart {PORTAL_GTK_UNIT}"
            ));
            if let Err(e) = start(c, "restart xdg-desktop-portal-gtk", None) {
                log::warn!("{PORTAL_GTK_UNIT}: {e}; the print dialog keeps its theme");
            }
        }
        Err(e) => log::warn!("{PORTAL_GTK_UNIT} drop-in: {e}; the print dialog keeps its theme"),
    }
}

/// Write the `GTK_THEME` drop-in for the GTK portal backend under `runtime`. Returns whether
/// the file changed.
fn write_portal_gtk_drop_in(runtime: &std::path::Path, theme: &str) -> std::io::Result<bool> {
    let dir = runtime
        .join("systemd/user")
        .join(format!("{PORTAL_GTK_UNIT}.d"));
    let path = dir.join("reprise.conf");
    // systemd unquotes quoted assignments and expands % specifiers.
    let value = theme
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    let text = format!("[Service]\nEnvironment=\"GTK_THEME={value}\"\n");
    if std::fs::read_to_string(&path).is_ok_and(|t| t == text) {
        return Ok(false);
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(&path, text)?;
    Ok(true)
}

/// Every variable `export_session_env` can publish.
fn exported_names() -> Vec<&'static str> {
    let every = SessionEnv {
        x11_display: Some(String::new()),
        login: FORWARDED.map(|k| (k, String::new())).to_vec(),
        ..SessionEnv::default()
    };
    every.pairs().into_iter().map(|(k, _)| k).collect()
}

/// The published variables `get` has, when it describes a nested session (one with a display of
/// its own); empty otherwise.
fn parent_session(get: impl Fn(&str) -> Option<String>) -> Vec<(&'static str, String)> {
    if get("WAYLAND_DISPLAY").is_none() && get("DISPLAY").is_none() {
        return Vec::new();
    }
    exported_names()
        .into_iter()
        .filter_map(|k| Some((k, get(k)?)))
        .collect()
}

/// `XDG_SESSION_ID` when `get` describes a session a display manager started for Reprise (typed
/// "wayland", no other display). A TTY login is typed "tty" and ends with its shell.
fn own_session(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    let own =
        get("XDG_SESSION_TYPE").as_deref() == Some("wayland") && parent_session(&get).is_empty();
    own.then(|| get("XDG_SESSION_ID")).flatten()
}

/// Reprise's portal backend unit and the portal frontend that routes to it.
const PORTAL_UNITS: [&str; 2] = ["reprise-portal.service", "xdg-desktop-portal.service"];

/// The commands that end the session `env` published, in order: unset (or restore the parent's)
/// variables in the systemd user manager, stop the portal backend, and the session target and
/// frontend too unless nested (it reads `XDG_CURRENT_DESKTOP` once, so a survivor would route the next desktop's
/// dialogs here). dbus-broker defers to systemd, so that is all to undo. With `end_processes`
/// and not nested, the app scopes stop too, and a session of our own is terminated last: logind
/// leaves its processes running without `KillUserProcesses`, e.g. a windowless Deskflow server.
pub fn end_commands(env: &SessionEnv, end_processes: bool) -> Vec<Vec<String>> {
    let user = |verb: &str| vec!["systemctl".to_string(), "--user".into(), verb.into()];
    let mut out = Vec::new();
    if !env.parent.is_empty() {
        let mut c = user("set-environment");
        c.extend(env.parent.iter().map(|(k, v)| format!("{k}={v}")));
        out.push(c);
    }
    let unset: Vec<String> = exported_names()
        .into_iter()
        .filter(|k| !env.parent.iter().any(|(p, _)| p == k))
        .map(String::from)
        .collect();
    if !unset.is_empty() {
        let mut c = user("unset-environment");
        c.extend(unset);
        out.push(c);
    }
    let mut stop = user("stop");
    stop.insert(2, "--no-block".into());
    if env.parent.is_empty() {
        stop.push(SESSION_TARGET.into());
        stop.extend(PORTAL_UNITS.iter().map(|u| u.to_string()));
    } else {
        stop.push(PORTAL_UNITS[0].into());
    }
    if end_processes && env.parent.is_empty() {
        stop.push(reprisefs::scope::PATTERN.into());
    }
    out.push(stop);
    if end_processes && let Some(id) = &env.logind_session {
        out.push(vec![
            "loginctl".into(),
            "terminate-session".into(),
            id.clone(),
        ]);
    }
    out
}

/// Run `end_commands` as the compositor exits.
pub fn end_session_env(env: &SessionEnv, end_processes: bool) {
    run_waited(end_commands(env, end_processes));
}

/// The command that stops the app scopes a previous session left behind when it crashed, before
/// this one starts any. None when nested (the scopes are the parent's) or with `end_processes`
/// off (they were left on purpose).
pub fn stale_scopes_command(env: &SessionEnv, end_processes: bool) -> Option<Vec<String>> {
    (end_processes && env.parent.is_empty()).then(|| {
        [
            "systemctl",
            "--user",
            "--no-block",
            "stop",
            reprisefs::scope::PATTERN,
        ]
        .map(String::from)
        .to_vec()
    })
}

/// Run `stale_scopes_command`, waited on so its pattern matches only scopes that existed before.
pub fn stop_stale_scopes(env: &SessionEnv, end_processes: bool) {
    run_waited(Vec::from_iter(stale_scopes_command(env, end_processes)));
}

/// Run `cmds` in order, giving all of them `WAIT` together. Best effort: failures are logged,
/// never waited on past the deadline.
fn run_waited(cmds: Vec<Vec<String>>) {
    const WAIT: std::time::Duration = std::time::Duration::from_secs(2);
    let deadline = std::time::Instant::now() + WAIT;
    for argv in cmds {
        let mut c = std::process::Command::new(&argv[0]);
        c.args(&argv[1..]);
        detach(&mut c);
        let mut child = match c.spawn() {
            Ok(child) => child,
            Err(e) => {
                log::warn!("{argv:?}: {e}");
                return;
            }
        };
        loop {
            match child.try_wait() {
                Ok(Some(st)) if !st.success() => {
                    log::warn!("{argv:?} exited with {st}");
                    break;
                }
                Ok(Some(_)) => break,
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Ok(None) | Err(_) => {
                    log::warn!("{argv:?} still running at exit; not waiting for it");
                    return;
                }
            }
        }
    }
}

/// `cmd` as a `sh -c` process with the session's variables exported. `token` becomes
/// `XDG_ACTIVATION_TOKEN`; without one the variable is removed so an inherited token never
/// reaches a client. `DESKTOP_STARTUP_ID` is always removed.
pub fn command(cmd: &str, env: &SessionEnv, token: Option<&str>) -> std::process::Command {
    let mut c = std::process::Command::new("sh");
    c.arg("-c").arg(cmd);
    with_session_env(&mut c, env, token);
    c
}

/// `program` with `args` as a process of its own, no shell: a desktop entry's `Exec`
/// (`reprisefs::apps::exec_argv`).
fn command_argv<S: AsRef<std::ffi::OsStr>>(
    program: &S,
    args: &[S],
    env: &SessionEnv,
    token: Option<&str>,
) -> std::process::Command {
    let mut c = std::process::Command::new(program);
    c.args(args);
    with_session_env(&mut c, env, token);
    c
}

fn with_session_env(c: &mut std::process::Command, env: &SessionEnv, token: Option<&str>) {
    for (k, v) in env.pairs() {
        c.env(k, v);
    }
    if let Some(qt) = env.qt_platform_theme() {
        c.env("QT_QPA_PLATFORMTHEME", qt);
    }
    if env.x11_display.is_none() {
        c.env_remove("DISPLAY");
    }
    c.env_remove("DESKTOP_STARTUP_ID");
    match token {
        Some(t) => {
            c.env("XDG_ACTIVATION_TOKEN", t);
        }
        None => {
            c.env_remove("XDG_ACTIVATION_TOKEN");
        }
    }
}

/// Returns whether the shell started; what it runs may still fail.
pub fn spawn(cmd: &str, env: &SessionEnv, token: Option<&str>) -> bool {
    spawn_in(cmd, None, env, token)
}

/// `spawn` in an app scope, as [`spawn_argv_in`] does.
pub fn spawn_in(cmd: &str, app: Option<&str>, env: &SessionEnv, token: Option<&str>) -> bool {
    let name = log_name(
        app,
        &crate::session::program(cmd).unwrap_or_else(|| "sh".into()),
    );
    let argv = reprisefs::scope::in_scope(&["sh", "-c", cmd], Some(&name));
    match start(
        command_argv(&argv[0], &argv[1..], env, token),
        cmd,
        Some(&name),
    ) {
        Ok(()) => true,
        Err(e) => {
            log::error!("spawn {cmd:?}: {e}");
            false
        }
    }
}

/// `spawn` for an argument vector; an empty one is logged and skipped.
pub fn spawn_argv<S: AsRef<std::ffi::OsStr> + std::fmt::Debug>(
    argv: &[S],
    env: &SessionEnv,
    token: Option<&str>,
) {
    spawn_argv_in(argv, None, None, env, token);
}

/// `spawn_argv` in working directory `dir`, or reprise's own when `None`. The process runs in
/// `app-reprise-<name>-*.scope` (xdg-desktop-portal reads a host app's id there), `name` being
/// `app`, the desktop entry id the command came from, else the program's name. journald
/// rate-limits per unit of the writing process, so the scope keeps a chatty program's log from
/// suppressing the compositor's.
pub fn spawn_argv_in<S: AsRef<std::ffi::OsStr> + std::fmt::Debug>(
    argv: &[S],
    app: Option<&str>,
    dir: Option<&std::path::Path>,
    env: &SessionEnv,
    token: Option<&str>,
) {
    let program = argv.first().map(|p| p.as_ref().to_string_lossy());
    let name = program.map(|p| log_name(app, &p));
    if let Err(e) = try_spawn_argv_in(argv, name.as_deref(), dir, env, token) {
        log::error!("spawn {argv:?}: {e}");
    }
}

/// [`spawn_argv_in`] that hands back why the program did not start (the exec's own error, such
/// as a missing interpreter). Waits for the exec, so a program on a hung mount blocks the caller.
/// Without an `app` it runs unscoped: behind `systemd-run` the error would be systemd-run's.
pub fn try_spawn_argv_in<S: AsRef<std::ffi::OsStr> + std::fmt::Debug>(
    argv: &[S],
    app: Option<&str>,
    dir: Option<&std::path::Path>,
    env: &SessionEnv,
    token: Option<&str>,
) -> std::io::Result<()> {
    let Some(program) = argv.first() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty command line",
        ));
    };
    let what = program.as_ref().to_string_lossy();
    let name = log_name(app, &what);
    let argv = reprisefs::scope::in_scope(argv, app);
    let mut c = command_argv(&argv[0], &argv[1..], env, token);
    if let Some(dir) = dir {
        c.current_dir(dir);
    }
    start(c, &what, Some(&name))
}

/// The journal identifier of a spawned program: its desktop entry id, else its file name.
fn log_name(app: Option<&str>, program: &str) -> String {
    match app {
        Some(app) => app.strip_suffix(".desktop").unwrap_or(app).to_string(),
        None => program.rsplit('/').next().unwrap_or(program).to_string(),
    }
}

/// The app scope of shell command `cmd`: the id of the entry in `apps` named like the program it
/// runs or whose `Exec` runs that program, else the program's own name (systemd's
/// `basename(argv[0])` fallback). `gtk-launch <id>` takes the entry it names. None when it starts
/// through another launcher such as `sh` or `env`.
pub fn command_app(cmd: &str, apps: &[reprisefs::apps::AppEntry]) -> Option<String> {
    if let Some(id) = gtk_launch_id(cmd) {
        return reprisefs::apps::entry_for_app_id(apps, &id, |_| true).map(|e| e.id.clone());
    }
    let program = crate::session::program(cmd)?;
    let program = program.trim_matches('\'');
    if program.contains('=') {
        return None;
    }
    let entry = reprisefs::apps::entry_for_app_id(apps, program, |_| true).or_else(|| {
        apps.iter()
            .find(|e| crate::session::program(&e.exec_line).as_deref() == Some(program))
    });
    Some(entry.map_or_else(|| program.to_string(), |e| e.id.clone()))
}

/// The folder a Run line names when the whole line is one path: absolute or under `~`, bare or
/// single-quoted as Browse... writes it. None for a line the shell would read as more than a path.
pub fn command_folder(cmd: &str, home: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let cmd = cmd.trim();
    let path = if let Some(inner) = cmd.strip_prefix('\'').and_then(|c| c.strip_suffix('\'')) {
        let word = inner.replace("'\\''", "'");
        if inner.replace("'\\''", "").contains('\'') {
            return None;
        }
        PathBuf::from(word)
    } else if cmd.contains(|c: char| c.is_whitespace() || "'\"\\$`;&|<>(){}*?[]".contains(c)) {
        return None;
    } else if cmd == "~" {
        home?.to_path_buf()
    } else if let Some(rest) = cmd.strip_prefix("~/") {
        home?.join(rest)
    } else {
        PathBuf::from(cmd)
    };
    path.is_absolute().then_some(path)
}

/// The entry id `gtk-launch [options] <id> [uris]` names, `.desktop` optional.
fn gtk_launch_id(cmd: &str) -> Option<String> {
    let argv = reprisefs::apps::exec_argv(cmd);
    let (program, args) = argv.split_first()?;
    if program.rsplit('/').next() != Some("gtk-launch") {
        return None;
    }
    let id = args.iter().find(|a| !a.starts_with('-'))?;
    Some(id.strip_suffix(".desktop").unwrap_or(id).to_string())
}

/// Start `c` detached, hand its pid to `reap_children` and record the launch for `mapfocus`;
/// `what` names it in the log. A `name` gives its output a journal stream of its own; without
/// one it writes to the compositor's.
fn start(mut c: std::process::Command, what: &str, name: Option<&str>) -> std::io::Result<()> {
    detach(&mut c);
    if let Some((fd, stream)) = name.and_then(crate::journal::stream_for) {
        c.env("JOURNAL_STREAM", stream);
        c.stdout(fd.try_clone()?);
        c.stderr(fd);
    }
    // Dropping the `Child` does not reap.
    let child = c.spawn()?;
    CHILDREN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((child.id() as libc::pid_t, what.to_string()));
    crate::mapfocus::grant(child.id() as i32);
    log::info!("spawned {what:?}");
    Ok(())
}

/// Make `c` a child of its own: new session with no controlling terminal, stdin on `/dev/null`,
/// an empty signal mask (the `block_signals` mask survives exec) and SIGINT at its default (an
/// ignored one survives exec, as when started with `&`).
pub(crate) fn detach(c: &mut std::process::Command) {
    c.stdin(std::process::Stdio::null());
    // SAFETY: the closure runs between fork and exec; sigemptyset,
    // pthread_sigmask, signal and setsid are async-signal-safe and nothing
    // allocates.
    unsafe {
        c.pre_exec(|| {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::pthread_sigmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            // Fails only for a process group leader.
            libc::setsid();
            Ok(())
        });
    }
}

/// Reap the children we spawned, when the SIGCHLD signalfd fires. Only our own pids: Xwayland
/// is reaped through a pidfd, so `waitpid(-1, ..)` would steal its status.
pub fn reap_children() {
    let mut children = CHILDREN.lock().unwrap_or_else(|e| e.into_inner());
    children.retain(|(pid, what)| {
        let mut status = 0;
        // SAFETY: `status` is a valid out-param; `pid` is a child we spawned.
        match unsafe { libc::waitpid(*pid, &mut status, libc::WNOHANG) } {
            0 => true,
            r if r > 0 => {
                if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) != 0 {
                    log::warn!(
                        "child {pid} ({what:?}) exited with {}",
                        libc::WEXITSTATUS(status)
                    );
                } else if libc::WIFSIGNALED(status) {
                    log::warn!(
                        "child {pid} ({what:?}) killed by signal {}",
                        libc::WTERMSIG(status)
                    );
                } else {
                    log::info!("reaped child {pid}");
                }
                false
            }
            // ECHILD or similar: nothing left to wait for.
            _ => false,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn env_of(c: &std::process::Command, key: &str) -> Option<Option<String>> {
        c.get_envs()
            .find(|(k, _)| *k == OsStr::new(key))
            .map(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
    }

    fn env(x11: Option<&str>) -> SessionEnv {
        SessionEnv {
            wayland_display: "wayland-9".into(),
            x11_display: x11.map(str::to_string),
            desktop: "reprise".into(),
            colors: "k=v".into(),
            interface: reprisekit::interface::Interface::Classic,
            blue_variant: reprisekit::blue::Variant::Blue,
            glass_mode: Default::default(),
            glass_material: Default::default(),
            icons: "retro-icons".into(),
            cursor: ("default".into(), 24),
            qt_platform_theme: "auto".into(),
            qt_generated: None,
            renderer: "vulkan",
            login: vec![("XDG_SEAT", "seat0".into())],
            parent: Vec::new(),
            logind_session: None,
        }
    }

    fn argv(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_spawned_program_logs_under_its_entry_id_else_its_file_name() {
        assert_eq!(
            log_name(Some("reprise-sound.desktop"), "/usr/bin/reprise-sound"),
            "reprise-sound"
        );
        assert_eq!(
            log_name(Some("org.gnome.Calculator"), "gnome-calculator"),
            "org.gnome.Calculator"
        );
        assert_eq!(log_name(None, "/usr/bin/foot"), "foot");
    }

    #[test]
    fn a_typed_or_bound_command_scopes_as_its_entry_else_its_program() {
        let entry = |id: &str, exec: &str| {
            let text = format!("[Desktop Entry]\nType=Application\nName=N\nExec={exec}\n");
            reprisefs::apps::parse_desktop(&text, id).unwrap()
        };
        let apps = [
            entry("org.deskflow.deskflow.desktop", "deskflow"),
            entry("foot-server.desktop", "foot --server"),
            entry("foot.desktop", "foot"),
        ];
        let app = |cmd: &str| command_app(cmd, &apps);
        assert_eq!(
            app("deskflow").as_deref(),
            Some("org.deskflow.deskflow.desktop")
        );
        assert_eq!(app("/usr/bin/foot -e top").as_deref(), Some("foot.desktop"));
        assert_eq!(app("'/usr/bin/foot'").as_deref(), Some("foot.desktop"));
        assert_eq!(
            app("firefox https://example.org").as_deref(),
            Some("firefox")
        );
        assert_eq!(app("sh -c 'foot; foot'"), None);
        assert_eq!(
            app("gtk-launch org.deskflow.deskflow").as_deref(),
            Some("org.deskflow.deskflow.desktop")
        );
        assert_eq!(
            app("/usr/bin/gtk-launch -v foot.desktop /tmp/a").as_deref(),
            Some("foot.desktop")
        );
        assert_eq!(app("gtk-launch missing"), None);
        assert_eq!(app("GDK_BACKEND=x11 foot"), None);
        assert_eq!(app(""), None);
    }

    /// A session of its own publishes its variables before it starts the
    /// session target, stopping one a crashed session left first; a nested
    /// run leaves the parent's target alone.
    #[test]
    fn a_login_session_begins_by_publishing_then_restarting_the_session_target() {
        let cmds = begin_commands(&env(None));
        assert_eq!(cmds.len(), 3);
        assert_eq!(
            &cmds[0][..2],
            &argv(&["dbus-update-activation-environment", "--systemd"])[..]
        );
        assert!(cmds[0].contains(&"WAYLAND_DISPLAY=wayland-9".to_string()));
        assert_eq!(
            cmds[1],
            argv(&[
                "systemctl",
                "--user",
                "stop",
                "reprise-session.target",
                "graphical-session.target"
            ])
        );
        assert_eq!(
            cmds[2],
            argv(&[
                "systemctl",
                "--user",
                "--no-block",
                "start",
                "reprise-session.target"
            ])
        );
        let nested = SessionEnv {
            parent: vec![("WAYLAND_DISPLAY", "wayland-0".into())],
            ..env(None)
        };
        assert_eq!(begin_commands(&nested), vec![export_argv(&nested)]);
    }

    /// A session of its own unsets every variable it can publish, set or
    /// not, then stops the session target, the portal backend, the frontend
    /// and its app scopes.
    #[test]
    fn a_login_session_ends_by_unsetting_and_stopping_the_target_portals_and_app_scopes() {
        let every = [
            "WAYLAND_DISPLAY",
            "XDG_CURRENT_DESKTOP",
            "XDG_SESSION_TYPE",
            reprisekit::theme::COLORS_ENV,
            reprisekit::interface::ENV,
            reprisekit::blue::ENV,
            reprisekit::glass::MODE_ENV,
            reprisekit::glass::MATERIAL_ENV,
            reprisekit::icontheme::ENV,
            "XCURSOR_THEME",
            "XCURSOR_SIZE",
            reprisefs::sysinfo::RENDERER_ENV,
            "GTK_USE_PORTAL",
            "DISPLAY",
            "XDG_SESSION_DESKTOP",
            "XDG_SEAT",
            "XDG_SEAT_PATH",
        ];
        let mut unset = argv(&["systemctl", "--user", "unset-environment"]);
        unset.extend(every.iter().map(|s| s.to_string()));
        assert_eq!(
            end_commands(&env(None), true),
            vec![
                unset,
                argv(&[
                    "systemctl",
                    "--user",
                    "--no-block",
                    "stop",
                    "reprise-session.target",
                    "reprise-portal.service",
                    "xdg-desktop-portal.service",
                    "app-reprise-*.scope",
                ]),
            ]
        );
        // Whatever a session published is among them.
        for (k, _) in env(Some(":1")).pairs() {
            assert!(every.contains(&k), "{k} is unset at the end");
        }
    }

    /// A display manager's session ends with its own processes after the
    /// portals stop; a TTY login or a nested run leaves its session alone.
    #[test]
    fn only_a_display_manager_session_is_terminated_at_the_end() {
        let dm = |k: &str| match k {
            "XDG_SESSION_TYPE" => Some("wayland".to_string()),
            "XDG_SESSION_ID" => Some("3".into()),
            _ => None,
        };
        assert_eq!(own_session(dm), Some("3".into()));
        let e = SessionEnv {
            logind_session: own_session(dm),
            ..env(None)
        };
        assert_eq!(
            end_commands(&e, true).last(),
            Some(&argv(&["loginctl", "terminate-session", "3"]))
        );
        let tty = |k: &str| match k {
            "XDG_SESSION_TYPE" => Some("tty".to_string()),
            _ => dm(k),
        };
        assert_eq!(own_session(tty), None);
        let nested = |k: &str| match k {
            "WAYLAND_DISPLAY" => Some("wayland-0".to_string()),
            _ => dm(k),
        };
        assert_eq!(own_session(nested), None);
        assert!(
            !end_commands(&env(None), true)
                .iter()
                .any(|c| c[0] == "loginctl"),
            "no session, nothing terminated"
        );
    }

    /// `end_processes = false` leaves the login session and the app scopes
    /// running; the portals still stop.
    #[test]
    fn without_end_processes_the_app_scopes_and_session_outlive_the_end() {
        let e = SessionEnv {
            logind_session: Some("3".into()),
            ..env(None)
        };
        let cmds = end_commands(&e, false);
        assert!(!cmds.iter().any(|c| c[0] == "loginctl"));
        assert_eq!(
            cmds.last(),
            Some(&argv(&[
                "systemctl",
                "--user",
                "--no-block",
                "stop",
                "reprise-session.target",
                "reprise-portal.service",
                "xdg-desktop-portal.service",
            ]))
        );
        assert_eq!(stale_scopes_command(&e, false), None);
    }

    /// Scopes a crashed session left are stopped before the next one starts
    /// any; a nested run leaves its parent's alone.
    #[test]
    fn a_session_starts_by_stopping_app_scopes_left_by_the_last() {
        assert_eq!(
            stale_scopes_command(&env(None), true),
            Some(argv(&[
                "systemctl",
                "--user",
                "--no-block",
                "stop",
                "app-reprise-*.scope",
            ]))
        );
        let nested = SessionEnv {
            parent: vec![("WAYLAND_DISPLAY", "wayland-0".into())],
            ..env(None)
        };
        assert_eq!(stale_scopes_command(&nested, true), None);
        assert!(
            !end_commands(&nested, true)
                .concat()
                .contains(&"app-reprise-*.scope".to_string())
        );
    }

    /// A nested run puts the parent session's values back, unsets what the
    /// parent did not have, and leaves the parent's frontend running.
    #[test]
    fn a_nested_session_ends_by_restoring_its_parent() {
        let parent = parent_session(|k| match k {
            "WAYLAND_DISPLAY" => Some("wayland-0".into()),
            "XDG_CURRENT_DESKTOP" => Some("KDE".into()),
            _ => None,
        });
        assert_eq!(
            parent,
            vec![
                ("WAYLAND_DISPLAY", "wayland-0".to_string()),
                ("XDG_CURRENT_DESKTOP", "KDE".to_string()),
            ]
        );
        let e = SessionEnv {
            parent,
            ..env(None)
        };
        let cmds = end_commands(&e, true);
        assert_eq!(
            cmds[0],
            argv(&[
                "systemctl",
                "--user",
                "set-environment",
                "WAYLAND_DISPLAY=wayland-0",
                "XDG_CURRENT_DESKTOP=KDE",
            ])
        );
        assert_eq!(
            &cmds[1][..3],
            &argv(&["systemctl", "--user", "unset-environment"])[..]
        );
        assert!(!cmds[1].contains(&"WAYLAND_DISPLAY".to_string()));
        assert!(cmds[1].contains(&"XDG_SESSION_TYPE".to_string()));
        assert_eq!(
            cmds[2],
            argv(&[
                "systemctl",
                "--user",
                "--no-block",
                "stop",
                "reprise-portal.service"
            ])
        );
        // A login session has no parent to restore.
        assert!(
            parent_session(|k| (k == "XDG_CURRENT_DESKTOP").then(|| "reprise".into())).is_empty()
        );
    }

    #[test]
    fn only_a_new_look_is_exported() {
        let mut e = env(None);
        let classic = reprisekit::interface::Interface::Classic;
        let navy = reprisekit::blue::Variant::Blue;
        let light = (reprisekit::glass::Mode::Light, Default::default());
        let arrow = || ("default".to_string(), 24);
        assert!(
            !e.set_look("k=v".into(), classic, navy, light, "retro-icons", arrow()),
            "unchanged"
        );
        assert!(
            e.set_look("k=w".into(), classic, navy, light, "retro-icons", arrow()),
            "new colours"
        );
        assert_eq!(e.colors, "k=w");
        assert!(
            !e.set_look("k=w".into(), classic, navy, light, "retro-icons", arrow()),
            "and then unchanged"
        );
        assert!(
            e.set_look("k=w".into(), classic, navy, light, "hicolor", arrow()),
            "a new icon theme"
        );
        assert_eq!(e.icons, "hicolor");
        assert!(
            e.set_look(
                "k=w".into(),
                classic,
                navy,
                light,
                "hicolor",
                ("Adwaita".into(), 32)
            ),
            "a new cursor"
        );
        assert!(e.pairs().contains(&("XCURSOR_THEME", "Adwaita".into())));
        assert!(e.pairs().contains(&("XCURSOR_SIZE", "32".into())));
        let blue = reprisekit::interface::Interface::Blue;
        assert!(
            e.set_look(
                "k=w".into(),
                blue,
                navy,
                light,
                "hicolor",
                ("Adwaita".into(), 32)
            ),
            "a new interface"
        );
        let silver = reprisekit::blue::Variant::Silver;
        assert!(
            e.set_look(
                "k=w".into(),
                blue,
                silver,
                light,
                "hicolor",
                ("Adwaita".into(), 32)
            ),
            "a new Blue variant"
        );
        assert!(
            e.pairs()
                .contains(&(reprisekit::blue::ENV, "silver".into()))
        );
    }

    #[test]
    fn the_glass_mode_and_material_are_exported_and_a_change_republishes() {
        use reprisekit::glass::{MATERIAL_ENV, MODE_ENV, Material, Mode};
        let mut e = env(None);
        assert!(e.pairs().contains(&(MODE_ENV, "light".into())));
        assert!(e.pairs().contains(&(MATERIAL_ENV, "frosted".into())));
        let (colors, cursor) = (e.colors.clone(), e.cursor.clone());
        let icons = e.icons.clone();
        for glass in [
            (Mode::Dark, Material::Frosted),
            (Mode::Dark, Material::Smoked),
        ] {
            assert!(
                e.set_look(
                    colors.clone(),
                    e.interface,
                    e.blue_variant,
                    glass,
                    &icons,
                    cursor.clone()
                ),
                "{glass:?} republishes"
            );
        }
        assert!(e.pairs().contains(&(MODE_ENV, "dark".into())));
        assert!(e.pairs().contains(&(MATERIAL_ENV, "smoked".into())));
    }

    #[test]
    fn command_exports_token_when_given() {
        let c = command("true", &env(None), Some("tok-1"));
        assert_eq!(
            env_of(&c, "XDG_ACTIVATION_TOKEN"),
            Some(Some("tok-1".into()))
        );
        assert_eq!(
            env_of(&c, "WAYLAND_DISPLAY"),
            Some(Some("wayland-9".into()))
        );
        // No X11 display: DISPLAY is removed rather than inherited.
        assert_eq!(env_of(&c, "DISPLAY"), Some(None));
    }

    #[test]
    fn command_removes_stale_token_without_one() {
        // A token inherited from the environment that launched reprise, of
        // either kind, must not leak into children: the compositor is the
        // only source of valid tokens for its own clients.
        let c = command("true", &env(Some(":1")), None);
        assert_eq!(env_of(&c, "XDG_ACTIVATION_TOKEN"), Some(None));
        assert_eq!(env_of(&c, "DESKTOP_STARTUP_ID"), Some(None));
        assert_eq!(env_of(&c, "DISPLAY"), Some(Some(":1".into())));
    }

    /// The desktop name, the colours, the interface, Blue's variant and
    /// the icon theme reach a child through the command, not the process
    /// environment.
    #[test]
    fn command_carries_the_desktop_and_colours() {
        let c = command("true", &env(None), None);
        assert_eq!(
            env_of(&c, "XDG_CURRENT_DESKTOP"),
            Some(Some("reprise".into()))
        );
        assert_eq!(
            env_of(&c, reprisekit::theme::COLORS_ENV),
            Some(Some("k=v".into()))
        );
        assert_eq!(
            env_of(&c, reprisekit::interface::ENV),
            Some(Some("classic".into()))
        );
        assert_eq!(env_of(&c, reprisekit::blue::ENV), Some(Some("blue".into())));
        assert_eq!(
            env_of(&c, reprisekit::icontheme::ENV),
            Some(Some("retro-icons".into())),
            "the child reads the configured icon theme"
        );
    }

    /// "auto" points Qt at the portal under a dark scheme and leaves it
    /// alone under a light one; a named platform theme always stands. Never
    /// in the activation environment.
    #[test]
    fn qt_takes_the_portal_under_a_dark_look_or_the_configured_theme() {
        use reprisekit::theme::{Color, ColorScheme};
        let light = command("true", &env(None), None);
        assert_eq!(env_of(&light, "QT_QPA_PLATFORMTHEME"), None);
        let dark_scheme = ColorScheme {
            window: Color::rgb(32, 32, 32),
            ..ColorScheme::default()
        };
        let e = SessionEnv {
            colors: dark_scheme.to_env(),
            ..env(None)
        };
        let dark = command("true", &e, None);
        assert_eq!(
            env_of(&dark, "QT_QPA_PLATFORMTHEME"),
            Some(Some("xdgdesktopportal".into()))
        );
        assert!(!e.pairs().iter().any(|(k, _)| *k == "QT_QPA_PLATFORMTHEME"));
        let qt6ct = SessionEnv {
            qt_platform_theme: "qt6ct".into(),
            ..env(None)
        };
        assert_eq!(
            env_of(&command("true", &qt6ct, None), "QT_QPA_PLATFORMTHEME"),
            Some(Some("qt6ct".into()))
        );
        let reprise = SessionEnv {
            qt_platform_theme: "reprise".into(),
            qt_generated: Some("qt5ct"),
            ..env(None)
        };
        assert_eq!(
            env_of(&command("true", &reprise, None), "QT_QPA_PLATFORMTHEME"),
            Some(Some("qt5ct".into())),
            "\"reprise\" takes the tool holding the generated palette"
        );
    }

    /// The GTK portal backend's drop-in sets `GTK_THEME`, quoted for
    /// systemd, and reports a change only when its text moves.
    #[test]
    fn the_gtk_portal_backend_takes_the_theme_through_a_drop_in() {
        let runtime =
            std::env::temp_dir().join(format!("reprise-portal-gtk-{}", std::process::id()));
        let file = runtime.join("systemd/user/xdg-desktop-portal-gtk.service.d/reprise.conf");
        let read = || std::fs::read_to_string(&file).unwrap();
        assert!(write_portal_gtk_drop_in(&runtime, "Adwaita-dark").unwrap());
        assert_eq!(
            read(),
            "[Service]\nEnvironment=\"GTK_THEME=Adwaita-dark\"\n"
        );
        assert!(
            !write_portal_gtk_drop_in(&runtime, "Adwaita-dark").unwrap(),
            "the same theme leaves the backend running"
        );
        assert!(write_portal_gtk_drop_in(&runtime, "Adwaita").unwrap());
        assert_eq!(read(), "[Service]\nEnvironment=\"GTK_THEME=Adwaita\"\n");
        assert!(write_portal_gtk_drop_in(&runtime, r#"My "50%" \theme"#).unwrap());
        assert_eq!(
            read(),
            "[Service]\nEnvironment=\"GTK_THEME=My \\\"50%%\\\" \\\\theme\"\n",
            "quotes and backslashes escaped, % doubled"
        );
        std::fs::remove_dir_all(&runtime).unwrap();
    }

    /// A spawned child leads a session of its own and reads stdin from
    /// `/dev/null`.
    #[test]
    fn detached_child_has_its_own_session_and_no_stdin() {
        let mut c = command_argv(
            &"sh",
            &[
                "-c",
                "cut -d' ' -f6 /proc/self/stat; readlink /proc/self/fd/0",
            ],
            &env(None),
            None,
        );
        c.stdout(std::process::Stdio::piped());
        detach(&mut c);
        let out = c.spawn().unwrap().wait_with_output().unwrap();
        let out = String::from_utf8(out.stdout).unwrap();
        let mut lines = out.lines();
        let sid: libc::pid_t = lines.next().unwrap().parse().unwrap();
        // SAFETY: getsid(0) only reads the calling process's session id.
        let ours = unsafe { libc::getsid(0) };
        assert_ne!(sid, ours, "the child's session is not the test's");
        assert_eq!(lines.next(), Some("/dev/null"), "stdin is /dev/null");
    }

    /// A child of a compositor that ignores SIGINT still gets SIGINT's
    /// default action.
    #[test]
    fn detached_child_does_not_inherit_an_ignored_interrupt() {
        let mut c = command_argv(
            &"sh",
            &["-c", "grep SigIgn /proc/self/status"],
            &env(None),
            None,
        );
        c.stdout(std::process::Stdio::piped());
        // SAFETY: runs between fork and exec, before `detach`'s own closure;
        // signal is async-signal-safe. It ignores SIGINT in the child only,
        // as a compositor started with `&` would.
        unsafe {
            c.pre_exec(|| {
                libc::signal(libc::SIGINT, libc::SIG_IGN);
                Ok(())
            });
        }
        detach(&mut c);
        let out = c.spawn().unwrap().wait_with_output().unwrap();
        let out = String::from_utf8(out.stdout).unwrap();
        let mask =
            u64::from_str_radix(out.trim().trim_start_matches("SigIgn:").trim(), 16).unwrap();
        assert_eq!(mask & (1 << (libc::SIGINT - 1)), 0, "SIGINT is not ignored");
    }

    /// The activation environment carries the session type and the
    /// login's seat, alongside the display.
    #[test]
    fn pairs_carry_session_type_and_seat() {
        let p = env(None).pairs();
        assert!(p.contains(&("XDG_SESSION_TYPE", "wayland".into())));
        assert!(p.contains(&("XDG_SEAT", "seat0".into())));
    }

    #[test]
    fn a_run_line_naming_one_path_is_a_folder_to_check() {
        let home = std::path::Path::new("/home/u");
        let folder = |cmd| command_folder(cmd, Some(home));
        let at = |p: &str| Some(std::path::PathBuf::from(p));
        assert_eq!(folder(" /home/u/Documents "), at("/home/u/Documents"));
        assert_eq!(folder("~"), at("/home/u"));
        assert_eq!(folder("~/Documents"), at("/home/u/Documents"));
        assert_eq!(folder("'/srv/my files'"), at("/srv/my files"));
        assert_eq!(folder("'/srv/it'\\''s'"), at("/srv/it's"));
        assert_eq!(folder("'~/Documents'"), None, "a quoted ~ is not home");
        assert_eq!(command_folder("~/Documents", None), None);
        for line in [
            "foot",
            "Documents",
            "/usr/bin/foot -e top",
            "/srv/a;b",
            "'/srv/a' '/srv/b'",
            "$HOME",
        ] {
            assert_eq!(folder(line), None, "{line}");
        }
    }
}
