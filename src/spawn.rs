//! Launch child processes with the compositor's display sockets exported.

use crate::mutex::lock;
use reprisebus::units::SESSION_TARGET;
use std::collections::BTreeSet;
use std::os::unix::process::CommandExt;
use std::sync::Mutex;

/// Children started here and still to be reaped, by pid and log name.
///
/// Not a `SIGCHLD`/`SIG_IGN` disposition: it is inherited across exec, and Xwayland waits on
/// `xkbcomp`, which fails with `ECHILD` when children are auto-reaped. `reap_children` reaps
/// our own pids instead.
static CHILDREN: Mutex<Vec<(libc::pid_t, String)>> = Mutex::new(Vec::new());

#[cfg(test)]
thread_local! {
    /// Set by [`record`]: this thread's starts by log name, run in no process.
    static RECORDED: std::cell::RefCell<Option<Vec<String>>> =
        const { std::cell::RefCell::new(None) };
}

/// From here on, this thread's starts are only recorded; [`recorded`] lists them.
#[cfg(test)]
pub fn record() {
    RECORDED.set(Some(Vec::new()));
}

#[cfg(test)]
pub fn recorded() -> Vec<String> {
    RECORDED.with_borrow(|r| r.clone().unwrap_or_default())
}

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
    pub x11: Option<X11Env>,
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
    /// `LANG` and the other locale variables ([`take_user_locale`] applied), so services the bus
    /// starts follow the session's language. Empty when nested or without `LANG`.
    pub locale: Vec<(&'static str, String)>,
    /// Nested in another session (`WAYLAND_DISPLAY` or `DISPLAY` set at startup): that
    /// session's values of the published variables, put back at the end. Empty otherwise.
    pub parent: Vec<(&'static str, String)>,
    /// The logind session a display manager started Reprise as, which ends with it. None for a
    /// TTY start or nested run.
    pub logind_session: Option<String>,
    /// Set on the login screen: [`command`], the `spawn*` functions and the `try_spawn*` ones
    /// refuse every launch.
    pub refuses_programs: bool,
    /// The logind session our launches name in their app scopes
    /// ([`reprisefs::scope::session_pattern`]).
    pub session_id: Option<String>,
    /// Started from a text login while another Reprise session of the user runs: that session
    /// keeps the user manager's variables and units, which this one neither publishes nor stops.
    pub defers_user_units: bool,
    /// The session proxy file's variables ([`reprisefs::proxy`]), set in every child.
    pub proxy: Vec<(String, String)>,
    /// The proxy variables an earlier read of the file set and the last one did not: taken out
    /// of every child, as Settings took them out of the user manager.
    pub proxy_gone: Vec<String>,
}

/// How programs reach the X11 display: `DISPLAY` and the `XAUTHORITY` file with its cookie.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct X11Env {
    pub display: String,
    pub authority: String,
}

/// What the login manager and pam_systemd set that services outside our process tree still need:
/// units such as localsearch's start only under `XDG_SESSION_CLASS=user`.
const FORWARDED: [&str; 4] = [
    "XDG_SESSION_DESKTOP",
    "XDG_SESSION_CLASS",
    "XDG_SEAT",
    "XDG_SEAT_PATH",
];

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
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let parent = parent_session(get);
        SessionEnv {
            locale: match parent.is_empty() {
                true => session_locale(get),
                false => Vec::new(),
            },
            parent,
            wayland_display,
            x11: None,
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
            logind_session: own_session(get),
            refuses_programs: false,
            session_id: reprisefs::scope::launch_session(),
            defers_user_units: false,
            proxy: reprisefs::proxy::read(&reprisefs::proxy::path()),
            proxy_gone: Vec::new(),
        }
    }

    /// Take the proxy file's variables as read now. Returns whether they changed.
    pub fn set_proxy(&mut self, vars: Vec<(String, String)>) -> bool {
        if vars == self.proxy {
            return false;
        }
        let set = |k: &str| vars.iter().any(|(n, _)| n == k);
        let mut gone: Vec<String> = (self.proxy.iter().map(|(k, _)| k))
            .chain(&self.proxy_gone)
            .filter(|k| !set(k))
            .cloned()
            .collect();
        gone.sort();
        gone.dedup();
        self.proxy = vars;
        self.proxy_gone = gone;
        true
    }

    /// Started from a text login, not nested: a login manager's session started later takes the
    /// user manager over.
    pub fn is_text_login(&self) -> bool {
        self.logind_session.is_none() && self.parent.is_empty()
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
        if let Some(x) = &self.x11 {
            v.push(("DISPLAY", x.display.clone()));
            v.push(("XAUTHORITY", x.authority.clone()));
        }
        v.extend(self.login.iter().cloned());
        v.extend(self.locale.iter().cloned());
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
    publish(env, login_manager_session_runs_reprise);
}

/// [`export_session_env`], unless this is a text-login session and `taken_over` says a login
/// manager's Reprise session has since taken the user manager over.
fn publish(env: &SessionEnv, taken_over: impl FnOnce() -> bool) {
    if env.defers_user_units {
        return;
    }
    if env.is_text_login() && taken_over() {
        log::info!("a login manager's Reprise session runs; its user manager variables stay");
        return;
    }
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
    // The user manager keeps the variables it read from /etc/locale.conf through any
    // `unset-environment`; an empty value is unset to setlocale, so it masks them.
    if !env.locale.is_empty() {
        let absent = reprisefs::region::VARS
            .iter()
            .filter(|k| !env.locale.iter().any(|(set, _)| set == *k));
        argv.extend(absent.map(|k| format!("{k}=")));
    }
    argv
}

/// The AT-SPI registry's units. dbus-broker activates the registry as a transient unit outside
/// `at-spi-dbus-bus.service`, and it outlives the accessibility bus that stops with
/// `graphical-session.target`, so whatever stops that target stops these too.
const A11Y_REGISTRY: &str = "dbus-*-org.a11y.atspi.Registry@*.service";

/// The commands that open the session `env` describes, in order: publish the variables, stop the
/// target a crashed session left active (its units hold the old display) and the registries of
/// the accessibility bus it takes down, start the target. A nested run only publishes:
/// `graphical-session.target` is the parent's.
pub fn begin_commands(env: &SessionEnv) -> Vec<Vec<String>> {
    if env.defers_user_units {
        return Vec::new();
    }
    let mut out = vec![export_argv(env)];
    if env.parent.is_empty() {
        // `graphical-session.target` by name: it stops when unneeded only after the stop job,
        // so the start that follows would keep it and its units running.
        let stop = [
            "stop",
            SESSION_TARGET,
            "graphical-session.target",
            A11Y_REGISTRY,
        ];
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
/// target's units start with the variables published. The thread runs `then` and ends once the
/// start is queued.
pub fn begin_session(
    env: &SessionEnv,
    then: impl FnOnce() + Send + 'static,
) -> std::thread::JoinHandle<()> {
    let cmds = begin_commands(env);
    std::thread::spawn(move || {
        for argv in cmds {
            run_to_end(&argv);
        }
        then();
    })
}

/// The command that unsets the variables another session published that `env` doesn't, e.g. its
/// `DISPLAY` and `XAUTHORITY` when this one has no X11.
pub(crate) fn unset_unpublished(env: &SessionEnv) -> Option<Vec<String>> {
    let published = env.pairs();
    let stale: Vec<String> = exported_names()
        .into_iter()
        .filter(|k| !published.iter().any(|(p, _)| p == k))
        .map(String::from)
        .collect();
    (!stale.is_empty()).then(|| {
        let mut c = vec![
            "systemctl".into(),
            "--user".into(),
            "unset-environment".into(),
        ];
        c.extend(stale);
        c
    })
}

/// Run `argv` and wait for it; a failure is logged.
pub(crate) fn run_to_end(argv: &[String]) {
    let mut c = std::process::Command::new(&argv[0]);
    c.args(&argv[1..]);
    detach(&mut c);
    match c.status() {
        Ok(st) if !st.success() => log::warn!("{argv:?} exited with {st}"),
        Ok(_) => {}
        Err(e) => log::warn!("{argv:?}: {e}"),
    }
}

/// The GTK portal backend's unit; it draws GTK3's print dialog (`Print=gtk` in
/// reprise-portals.conf).
const PORTAL_GTK_UNIT: &str = "xdg-desktop-portal-gtk.service";

/// Hand xdg-desktop-portal-gtk `theme` so its print dialog looks like the program. The backend
/// reads GSettings, which Reprise leaves alone; `GTK_THEME` outranks that, via a drop-in under
/// `$XDG_RUNTIME_DIR`. A running backend restarts to read it. No-op when the drop-in already
/// says `theme`, on a nested run or beside another Reprise session.
pub fn theme_portal_gtk(env: &SessionEnv, theme: &str) {
    if portal_gtk_drop_in(env, theme) {
        let mut c = std::process::Command::new("sh");
        c.arg("-c").arg(format!(
            "systemctl --user daemon-reload && systemctl --user --no-block try-restart {PORTAL_GTK_UNIT}"
        ));
        if let Err(e) = start(c, "restart xdg-desktop-portal-gtk", None) {
            log::warn!("{PORTAL_GTK_UNIT}: {e}; the print dialog keeps its theme");
        }
    }
}

/// Write [`theme_portal_gtk`]'s drop-in, without the restart. Returns whether it changed, so the
/// user manager needs a `daemon-reload`.
pub(crate) fn portal_gtk_drop_in(env: &SessionEnv, theme: &str) -> bool {
    if !env.parent.is_empty() || env.defers_user_units {
        return false;
    }
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") else {
        return false;
    };
    write_portal_gtk_drop_in(std::path::Path::new(&runtime), theme)
        .inspect_err(|e| {
            log::warn!("{PORTAL_GTK_UNIT} drop-in: {e}; the print dialog keeps its theme")
        })
        .unwrap_or(false)
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
        x11: Some(X11Env::default()),
        login: FORWARDED.map(|k| (k, String::new())).to_vec(),
        ..SessionEnv::default()
    };
    every.pairs().into_iter().map(|(k, _)| k).collect()
}

/// The locale variables `get` has, when `LANG` is among them.
fn session_locale(get: impl Fn(&str) -> Option<String>) -> Vec<(&'static str, String)> {
    let set: Vec<(&'static str, String)> = reprisefs::region::VARS
        .iter()
        .filter_map(|&k| Some((k, get(k)?)))
        .collect();
    match set.iter().any(|(k, _)| *k == "LANG") {
        true => set,
        false => Vec::new(),
    }
}

/// Put the user's locale.conf ([`reprisefs::region::user_conf`]) into this process's
/// environment, which every program it starts inherits. Runs first thing in `main`, before any
/// other thread exists. Returns the file's assignments, for the log.
pub fn take_user_locale() -> Vec<(&'static str, Option<String>)> {
    let Ok(text) = std::fs::read_to_string(reprisefs::region::user_conf()) else {
        return Vec::new();
    };
    let env = reprisefs::region::session_env(&text);
    for (k, v) in &env {
        // SAFETY: called before any other thread exists, so nothing reads the environment
        // while it changes.
        unsafe {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
    env
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
/// variables in the systemd user manager, stop the portal backend, and the session target,
/// [`A11Y_REGISTRY`] and frontend too unless nested (it reads `XDG_CURRENT_DESKTOP` once, so a
/// survivor would route the next desktop's dialogs here). dbus-broker defers to systemd, so that
/// is all to undo. With `end_processes` and not nested, the app scopes stop too, and a session of
/// our own is terminated last: logind leaves its processes running without `KillUserProcesses`,
/// e.g. a windowless Deskflow server. While `another_runs` (another Reprise session of the user)
/// the variables and units stay its own and only this session's app scopes stop.
pub fn end_commands(env: &SessionEnv, end_processes: bool, another_runs: bool) -> Vec<Vec<String>> {
    let user = |verb: &str| vec!["systemctl".to_string(), "--user".into(), verb.into()];
    let nested = !env.parent.is_empty();
    let mut out = Vec::new();
    if nested {
        let mut c = user("set-environment");
        c.extend(env.parent.iter().map(|(k, v)| format!("{k}={v}")));
        out.push(c);
    }
    let unset: Vec<String> = exported_names()
        .into_iter()
        .filter(|k| !env.parent.iter().any(|(p, _)| p == k))
        .map(String::from)
        .collect();
    if !unset.is_empty() && (nested || !another_runs) {
        let mut c = user("unset-environment");
        c.extend(unset);
        out.push(c);
    }
    let mut stop = user("stop");
    stop.insert(2, "--no-block".into());
    let stop_verb = stop.len();
    if nested {
        stop.push(PORTAL_UNITS[0].into());
    } else if !another_runs {
        stop.push(SESSION_TARGET.into());
        stop.push(A11Y_REGISTRY.into());
        stop.extend(PORTAL_UNITS.iter().map(|u| u.to_string()));
    }
    if end_processes && !nested {
        if !another_runs {
            stop.push(reprisefs::scope::PATTERN.into());
        } else if let Some(id) = &env.session_id {
            stop.push(reprisefs::scope::session_pattern(id));
        }
    }
    if stop.len() > stop_verb {
        out.push(stop);
    }
    if end_processes && let Some(id) = &env.logind_session {
        out.push(vec![
            "loginctl".into(),
            "terminate-session".into(),
            id.clone(),
        ]);
    }
    out
}

/// Whether a process named `comm`, as this compositor is, runs in one of the `online` logind
/// sessions of the user other than `own`; `procs` are each process's `comm` and `cgroup` files.
fn runs_elsewhere(
    comm: &str,
    own: Option<&str>,
    online: &[&str],
    procs: impl IntoIterator<Item = (String, String)>,
) -> bool {
    procs.into_iter().any(|(c, cgroup)| {
        c.trim_end() == comm
            && reprisebus::logind::session_of_cgroup(&cgroup)
                .is_some_and(|s| Some(s) != own && online.contains(&s))
    })
}

/// Whether another of the user's logind sessions runs Reprise: its user manager, portals and
/// app scopes are then shared with that session.
pub fn another_session_runs_reprise() -> bool {
    reprise_runs_in_another(|_| true)
}

/// Whether Reprise runs in another of the user's sessions that a login manager opened.
pub(crate) fn login_manager_session_runs_reprise() -> bool {
    reprise_runs_in_another(|id| {
        let record = reprisebus::logind::session_record(id);
        reprisebus::logind::type_of_session(&record) == Some("wayland")
    })
}

/// Whether Reprise runs in another of the user's sessions that a login manager or an older
/// logind session opened; a deferred text login keeps deferring to those.
pub(crate) fn login_manager_or_older_session_runs_reprise() -> bool {
    let cgroup = reprisebus::logind::own_cgroup();
    let own = reprisebus::logind::session_of_cgroup(&cgroup).map(|id| {
        let opened = reprisebus::logind::opened_of_session(&reprisebus::logind::session_record(id));
        (opened, id)
    });
    reprise_runs_in_another(|id| {
        let record = reprisebus::logind::session_record(id);
        reprisebus::logind::type_of_session(&record) == Some("wayland")
            || opened_before((reprisebus::logind::opened_of_session(&record), id), own)
    })
}

/// Whether session `other` opened before `own`, each as logind's opening time and its id. An
/// unknown time, or an unknown own session, counts as opened before.
fn opened_before(other: (Option<u64>, &str), own: Option<(Option<u64>, &str)>) -> bool {
    match (other, own) {
        ((Some(at), id), Some((Some(own_at), own_id))) => (at, id) < (own_at, own_id),
        _ => true,
    }
}

/// Whether Reprise runs in another of the user's online sessions that `counts`.
fn reprise_runs_in_another(counts: impl Fn(&str) -> bool) -> bool {
    let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
    let own_comm = read("/proc/self/comm");
    let own = reprisebus::logind::own_cgroup();
    let record = reprisebus::logind::own_user_record();
    let me = std::process::id();
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return false;
    };
    let procs = dir.filter_map(|e| {
        let pid: u32 = e.ok()?.file_name().to_str()?.parse().ok()?;
        (pid != me).then_some(())?;
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
        // A compositor that exited but isn't reaped yet runs no session.
        (comm != own_comm || !exited(pid)).then_some(())?;
        let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
        Some((comm, cgroup))
    });
    let mut online = reprisebus::logind::online_sessions(&record);
    online.retain(|id| counts(id));
    if online.is_empty() {
        return false;
    }
    runs_elsewhere(
        own_comm.trim_end(),
        reprisebus::logind::session_of_cgroup(&own),
        &online,
        procs,
    )
}

/// Whether process `pid` exited and waits to be reaped: its `/proc/<pid>/stat` state is Z or X.
fn exited(pid: u32) -> bool {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    let state = stat
        .rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().next());
    matches!(state, Some("Z" | "X"))
}

/// Run `end_commands` as the compositor exits.
pub fn end_session_env(env: &SessionEnv, end_processes: bool) {
    run_waited(end_commands(
        env,
        end_processes,
        another_session_runs_reprise(),
    ));
}

/// The command that stops the app scopes a previous session left behind when it crashed, before
/// this one starts any. None when nested (the scopes are the parent's), with `end_processes` off
/// (they were left on purpose), while `another_running` (the user manager's scopes are that
/// session's), or `after_crash`: the programs that outlived the crash keep running.
pub fn stale_scopes_command(
    env: &SessionEnv,
    end_processes: bool,
    another_running: bool,
    after_crash: bool,
) -> Option<Vec<String>> {
    (end_processes && env.parent.is_empty() && !another_running && !after_crash).then(|| {
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
pub fn stop_stale_scopes(
    env: &SessionEnv,
    end_processes: bool,
    another_running: bool,
    after_crash: bool,
) {
    run_waited(Vec::from_iter(stale_scopes_command(
        env,
        end_processes,
        another_running,
        after_crash,
    )));
}

/// Whether another compositor holds `org.reprise.Tray` on the session bus. Asked before this
/// one's tray thread requests the name; false without a bus or an answer.
pub fn another_session_on_bus() -> bool {
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
    let owned = async || -> zbus::Result<bool> {
        let conn = zbus::connection::Builder::session()
            .method_timeout(TIMEOUT)
            .build()
            .await?;
        let name = zbus::names::BusName::try_from(reprisebus::tray::NAME)?;
        Ok(zbus::fdo::DBusProxy::new(&conn)
            .await?
            .name_has_owner(name)
            .await?)
    };
    zbus::block_on(owned()).unwrap_or_else(|e| {
        log::debug!("{}: {e}", reprisebus::tray::NAME);
        false
    })
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

/// A start the user asked for: its xdg-activation token, and whether the last input was a key.
#[derive(Clone, Debug)]
pub struct Launch {
    pub token: String,
    pub by_key: bool,
}

/// `cmd` as a `sh -c` process with the session's variables exported. `launch`'s token becomes
/// `XDG_ACTIVATION_TOKEN` and a start by key sets [`reprisebus::instance::KEYBOARD_LAUNCH_ENV`];
/// without one both are removed so an inherited value never reaches a client.
/// `DESKTOP_STARTUP_ID` is always removed.
/// None when `env` refuses programs.
pub fn command(
    cmd: &str,
    env: &SessionEnv,
    launch: Option<&Launch>,
) -> Option<std::process::Command> {
    let name = crate::session::program(cmd).unwrap_or_else(|| "sh".into());
    command_argv(&log_name(None, &name), &"sh", &["-c", cmd], env, launch)
}

/// Program names whose refusal is already in the journal.
static REFUSED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// `program` with `args` as a process of its own, no shell: a desktop entry's `Exec`
/// (`reprisefs::apps::exec_argv`). [`command`] and every `spawn*` build their command here, so
/// this is where [`SessionEnv::refuses_programs`] stops one: None, logged once per `name`.
fn command_argv<S: AsRef<std::ffi::OsStr>>(
    name: &str,
    program: &S,
    args: &[S],
    env: &SessionEnv,
    launch: Option<&Launch>,
) -> Option<std::process::Command> {
    if env.refuses_programs {
        if lock(&REFUSED).insert(name.to_string()) {
            log::info!("the login screen starts no programs: refused {name}");
        }
        return None;
    }
    let mut c = std::process::Command::new(program);
    c.args(args);
    with_session_env(&mut c, env, launch);
    Some(c)
}

fn with_session_env(c: &mut std::process::Command, env: &SessionEnv, launch: Option<&Launch>) {
    for (k, v) in env.pairs() {
        c.env(k, v);
    }
    for k in &env.proxy_gone {
        c.env_remove(k);
    }
    for (k, v) in &env.proxy {
        c.env(k, v);
    }
    if let Some(qt) = env.qt_platform_theme() {
        c.env("QT_QPA_PLATFORMTHEME", qt);
    }
    if env.x11.is_none() {
        c.env_remove("DISPLAY");
        c.env_remove("XAUTHORITY");
    }
    reprisebus::instance::Handoff {
        token: launch.map(|l| l.token.clone()),
        by_key: launch.is_some_and(|l| l.by_key),
    }
    .apply(c);
}

/// Returns whether the shell started; what it runs may still fail.
pub fn spawn(cmd: &str, env: &SessionEnv, launch: Option<&Launch>) -> bool {
    spawn_in(cmd, None, env, launch)
}

/// `spawn` in an app scope, as [`spawn_argv_in`] does.
pub fn spawn_in(cmd: &str, app: Option<&str>, env: &SessionEnv, launch: Option<&Launch>) -> bool {
    let name = log_name(
        app,
        &crate::session::program(cmd).unwrap_or_else(|| "sh".into()),
    );
    let argv = reprisefs::scope::in_scope(&["sh", "-c", cmd], Some(&name));
    let Some(c) = command_argv(&name, &argv[0], &argv[1..], env, launch) else {
        return false;
    };
    match start(c, &name, Some(&name)) {
        Ok(_) => true,
        Err(e) => {
            log::error!("spawn {name:?}: {e}");
            false
        }
    }
}

/// `spawn` for an argument vector; an empty one is logged and skipped.
pub fn spawn_argv<S: AsRef<std::ffi::OsStr> + std::fmt::Debug>(
    argv: &[S],
    env: &SessionEnv,
    launch: Option<&Launch>,
) {
    spawn_argv_in(argv, None, None, env, launch);
}

/// `spawn_argv` in `dir` (else reprise's own) inside `app-reprise-<name>-*.scope`, which gives the
/// portal its app id: `name` is `app`, the desktop entry id, else the program's name.
pub fn spawn_argv_in<S: AsRef<std::ffi::OsStr> + std::fmt::Debug>(
    argv: &[S],
    app: Option<&str>,
    dir: Option<&std::path::Path>,
    env: &SessionEnv,
    launch: Option<&Launch>,
) {
    let program = argv.first().map(|p| p.as_ref().to_string_lossy());
    let name = program.map(|p| log_name(app, &p));
    if let Err(e) = try_spawn_argv_in(argv, name.as_deref(), dir, env, launch) {
        log::error!("spawn {argv:?}: {e}");
    }
}

/// [`spawn_argv_in`] that hands back the pid, None for a refused launch, or why the program did
/// not start (the exec's own error, such as a missing interpreter). Waits for the exec, so a
/// program on a hung mount blocks the caller. Without an `app` it starts unscoped.
pub fn try_spawn_argv_in<S: AsRef<std::ffi::OsStr> + std::fmt::Debug>(
    argv: &[S],
    app: Option<&str>,
    dir: Option<&std::path::Path>,
    env: &SessionEnv,
    launch: Option<&Launch>,
) -> std::io::Result<Option<u32>> {
    let what = first(argv)?.as_ref().to_string_lossy();
    let name = log_name(app, &what);
    let argv = reprisefs::scope::in_scope(argv, app);
    let Some(mut c) = command_argv(&name, &argv[0], &argv[1..], env, launch) else {
        return Ok(None);
    };
    if let Some(dir) = dir {
        c.current_dir(dir);
    }
    start(c, &what, Some(&name)).map(Some)
}

/// [`try_spawn_argv_in`] in `app`'s scope without `systemd-run` in front, whose error would hide
/// the exec's: the child waits before its exec until it is in the scope
/// ([`reprisefs::scope::hold`]), so this also waits on the session bus. For off the loop.
pub fn try_spawn_held<S: AsRef<std::ffi::OsStr> + std::fmt::Debug>(
    argv: &[S],
    app: &str,
    dir: Option<&std::path::Path>,
    env: &SessionEnv,
    launch: Option<&Launch>,
) -> std::io::Result<Option<u32>> {
    let what = first(argv)?.as_ref().to_string_lossy();
    let name = log_name(Some(app), &what);
    let Some(mut c) = command_argv(&name, &argv[0], &argv[1..], env, launch) else {
        return Ok(None);
    };
    if let Some(dir) = dir {
        c.current_dir(dir);
    }
    reprisefs::scope::hold(&mut c, app)?;
    start_forked(c, &what, Some(&name)).map(Some)
}

/// `argv` unscoped with the session's environment and its stderr piped back, for a helper the
/// caller waits on itself (`Child::wait`) rather than [`reap_children`]. None when this session
/// starts no programs.
pub fn spawn_waited<S: AsRef<std::ffi::OsStr>>(
    argv: &[S],
    env: &SessionEnv,
) -> std::io::Result<Option<std::process::Child>> {
    let program = first(argv)?;
    let name = log_name(None, &program.as_ref().to_string_lossy());
    let Some(mut c) = command_argv(&name, program, &argv[1..], env, None) else {
        return Ok(None);
    };
    detach(&mut c);
    c.stdout(std::process::Stdio::null());
    c.stderr(std::process::Stdio::piped());
    c.spawn().map(Some)
}

fn first<S>(argv: &[S]) -> std::io::Result<&S> {
    argv.first()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "empty command line"))
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

/// The file `xdg-open <file>` opens; None for any other command line.
pub fn xdg_open_file(argv: &[std::ffi::OsString]) -> Option<&std::path::Path> {
    match argv {
        [open, file] if open == "xdg-open" => {
            Some(std::path::Path::new(file)).filter(|p| p.is_absolute())
        }
        _ => None,
    }
}

/// The regular file `xdg-open <file>` opens; None for any other command line or path. Stats
/// the path: call it off the loop.
pub fn xdg_open_document(argv: &[std::ffi::OsString]) -> Option<std::path::PathBuf> {
    xdg_open_file(argv)
        .filter(|p| p.is_file())
        .map(std::path::Path::to_path_buf)
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
/// one it writes to the compositor's. Spawned without a fork ([`spawn_detached`]).
fn start(mut c: std::process::Command, what: &str, name: Option<&str>) -> std::io::Result<u32> {
    use std::os::fd::AsFd;
    let journal = name.and_then(crate::journal::stream_for);
    if let Some((_, stream)) = &journal {
        c.env("JOURNAL_STREAM", stream);
    }
    let out = journal.as_ref().map(|(fd, _)| fd.as_fd());
    started(what, || spawn_detached(&c, out))
}

/// [`start`] for a `c` with a `pre_exec` of its own, which needs a fork.
fn start_forked(
    mut c: std::process::Command,
    what: &str,
    name: Option<&str>,
) -> std::io::Result<u32> {
    detach(&mut c);
    if let Some((fd, stream)) = name.and_then(crate::journal::stream_for) {
        c.env("JOURNAL_STREAM", stream);
        c.stdout(fd.try_clone()?);
        c.stderr(fd);
    }
    started(what, || c.spawn().map(|child| child.id()))
}

fn started(what: &str, spawn: impl FnOnce() -> std::io::Result<u32>) -> std::io::Result<u32> {
    #[cfg(test)]
    if RECORDED
        .with_borrow_mut(|r| r.as_mut().map(|r| r.push(what.to_string())))
        .is_some()
    {
        return Ok(0);
    }
    // Held across the spawn: a child started off the loop that exits at once is listed before
    // the loop's `reap_children` can look for it. Dropping a `Child` does not reap.
    let mut children = lock(&CHILDREN);
    let pid = spawn()?;
    children.push((pid as libc::pid_t, what.to_string()));
    drop(children);
    crate::mapfocus::grant(pid as i32);
    log::info!("spawned {what:?}");
    Ok(pid)
}

/// Spawns `c`'s program, arguments, environment changes and working directory set up as
/// [`detach`] does, with stdout and stderr on `out` or else the compositor's, and returns the
/// pid or the exec's own error. posix_spawn instead of `Command::spawn`, which forks once a
/// `pre_exec` is set: forking the compositor's address space holds the loop for milliseconds.
fn spawn_detached(
    c: &std::process::Command,
    out: Option<std::os::fd::BorrowedFd>,
) -> std::io::Result<u32> {
    use std::ffi::{CString, OsStr, OsString};
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    let cstr = |s: &OsStr| {
        CString::new(s.as_bytes())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "nul in argument"))
    };
    let check = |r: libc::c_int| match r {
        0 => Ok(()),
        e => Err(std::io::Error::from_raw_os_error(e)),
    };
    let program = cstr(c.get_program())?;
    let args = std::iter::once(c.get_program())
        .chain(c.get_args())
        .map(cstr)
        .collect::<std::io::Result<Vec<_>>>()?;
    let mut env: std::collections::BTreeMap<OsString, OsString> = std::env::vars_os().collect();
    for (k, v) in c.get_envs() {
        match v {
            Some(v) => env.insert(k.to_owned(), v.to_owned()),
            None => env.remove(k),
        };
    }
    let env = env
        .into_iter()
        .map(|(mut kv, v)| {
            kv.push("=");
            kv.push(v);
            cstr(&kv)
        })
        .collect::<std::io::Result<Vec<_>>>()?;
    let dir = c
        .get_current_dir()
        .map(|d| cstr(d.as_os_str()))
        .transpose()?;
    let ptrs = |v: &[CString]| -> Vec<*mut libc::c_char> {
        (v.iter().map(|s| s.as_ptr().cast_mut()))
            .chain(std::iter::once(std::ptr::null_mut()))
            .collect()
    };
    let (argv, envp) = (ptrs(&args), ptrs(&env));
    // SAFETY: the actions and attributes are initialised before use and destroyed once; every
    // pointer handed over points into a CString or array above that outlives the call.
    unsafe {
        let mut actions: libc::posix_spawn_file_actions_t = std::mem::zeroed();
        let mut attr: libc::posix_spawnattr_t = std::mem::zeroed();
        check(libc::posix_spawn_file_actions_init(&mut actions))?;
        check(libc::posix_spawnattr_init(&mut attr))?;
        let mut pid: libc::pid_t = 0;
        let r = (|| {
            check(libc::posix_spawn_file_actions_addopen(
                &mut actions,
                0,
                c"/dev/null".as_ptr(),
                libc::O_RDONLY,
                0,
            ))?;
            if let Some(out) = out {
                for to in [1, 2] {
                    let fd = out.as_raw_fd();
                    check(libc::posix_spawn_file_actions_adddup2(&mut actions, fd, to))?;
                }
            }
            if let Some(dir) = &dir {
                check(libc::posix_spawn_file_actions_addchdir_np(
                    &mut actions,
                    dir.as_ptr(),
                ))?;
            }
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            check(libc::posix_spawnattr_setsigmask(&mut attr, &set))?;
            libc::sigaddset(&mut set, libc::SIGINT);
            libc::sigaddset(&mut set, libc::SIGPIPE);
            check(libc::posix_spawnattr_setsigdefault(&mut attr, &set))?;
            let flags = libc::POSIX_SPAWN_SETSIGMASK | libc::POSIX_SPAWN_SETSIGDEF;
            check(libc::posix_spawnattr_setflags(
                &mut attr,
                libc::POSIX_SPAWN_SETSID | flags as libc::c_short,
            ))?;
            let spawn = match program.as_bytes().contains(&b'/') {
                true => libc::posix_spawn,
                false => libc::posix_spawnp,
            };
            check(spawn(
                &mut pid,
                program.as_ptr(),
                &actions,
                &attr,
                argv.as_ptr(),
                envp.as_ptr(),
            ))
        })();
        libc::posix_spawn_file_actions_destroy(&mut actions);
        libc::posix_spawnattr_destroy(&mut attr);
        r.map(|()| pid as u32)
    }
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

/// Leave `pid`, a child spawned outside this module and not yet waited for, to `reap_children`.
pub fn adopt(pid: libc::pid_t, what: &str) {
    lock(&CHILDREN).push((pid, what.to_string()));
}

/// Leave the children the crashed image of this process started, still ours after the restart's
/// exec, to `reap_children`, and reap the ones already gone: their `SIGCHLD` went before the exec.
pub fn adopt_orphans() {
    let path = format!("/proc/self/task/{}/children", std::process::id());
    let children = std::fs::read_to_string(&path).unwrap_or_default();
    for pid in children.split_whitespace().filter_map(|p| p.parse().ok()) {
        adopt(pid, "started before the crash");
    }
    reap_children();
}

/// Reap the children we spawned, when the SIGCHLD signalfd fires. Only our own pids: Xwayland
/// is reaped through a pidfd, so `waitpid(-1, ..)` would steal its status.
pub fn reap_children() {
    let mut children = lock(&CHILDREN);
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
            x11: x11.map(|d| X11Env {
                display: d.into(),
                authority: format!("/run/user/1000/reprise/xauth-{}", &d[1..]),
            }),
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
            locale: Vec::new(),
            parent: Vec::new(),
            logind_session: None,
            refuses_programs: false,
            session_id: None,
            defers_user_units: false,
            proxy: Vec::new(),
            proxy_gone: Vec::new(),
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
    fn a_document_shortcut_names_its_file_and_a_program_launcher_names_none() {
        let argv = |exec: &str| {
            let text = format!("[Desktop Entry]\nType=Application\nName=N\nExec={exec}\n");
            match crate::quicklaunch::parse(&text, "n.desktop", "foot")
                .unwrap()
                .action
            {
                crate::config::Action::Exec { argv, .. } => argv.unwrap(),
                a => panic!("{a:?}"),
            }
        };
        let doc = argv("xdg-open \"/home/u/My notes.txt\"");
        assert_eq!(
            xdg_open_file(&doc),
            Some(std::path::Path::new("/home/u/My notes.txt"))
        );
        assert_eq!(xdg_open_file(&argv("xdg-open https://example.org/")), None);
        assert_eq!(xdg_open_file(&argv("xdg-open a.txt b.txt")), None);
        assert_eq!(xdg_open_file(&argv("gedit /home/u/a.txt")), None);
    }

    #[test]
    fn a_document_shortcut_opens_by_type_only_on_a_regular_file() {
        let dir = std::env::temp_dir().join(format!("reprise-xdg-doc-{}", std::process::id()));
        let folder = dir.join("Backup.zip");
        let file = dir.join("notes.txt");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(&file, "n").unwrap();
        let open = |p: &std::path::Path| vec!["xdg-open".into(), p.as_os_str().to_owned()];
        assert_eq!(xdg_open_document(&open(&file)), Some(file.clone()));
        assert_eq!(
            xdg_open_document(&open(&folder)),
            None,
            "a folder named like an archive"
        );
        assert_eq!(xdg_open_document(&open(&dir.join("gone.txt"))), None);
        std::fs::remove_dir_all(&dir).unwrap();
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
    /// session target, stopping one a crashed session left and the AT-SPI
    /// registries first; a nested run leaves the parent's target alone.
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
                "graphical-session.target",
                "dbus-*-org.a11y.atspi.Registry@*.service",
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

    /// The session's locale reaches the user manager whole: what it sets is published, the
    /// locale variables it lacks go out empty, so the manager's own `LC_TIME` does not linger.
    #[test]
    fn a_session_with_a_locale_publishes_it_and_empties_the_rest() {
        let de = SessionEnv {
            locale: vec![
                ("LANG", "de_DE.UTF-8".into()),
                ("LC_TIME", "C.UTF-8".into()),
            ],
            ..env(None)
        };
        let export = &begin_commands(&de)[0];
        for set in [
            "LANG=de_DE.UTF-8",
            "LC_TIME=C.UTF-8",
            "LC_NUMERIC=",
            "LANGUAGE=",
        ] {
            assert!(export.contains(&set.to_string()), "{set}");
        }
        assert!(!export.contains(&"LANG=".to_string()));
        assert!(
            !export_argv(&env(None)).iter().any(|a| a == "LC_NUMERIC="),
            "a session without LANG leaves the manager's locale"
        );
        let mut c = std::process::Command::new("true");
        with_session_env(&mut c, &de, None);
        assert_eq!(
            env_of(&c, "LC_NUMERIC"),
            None,
            "nothing set empty for a child"
        );
        assert!(
            !end_commands(&de, true, false)[0].contains(&"LANG".to_string()),
            "the user manager keeps a locale after the session"
        );

        let get = |vars: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                vars.iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert_eq!(
            session_locale(get(&[("LANG", "de_DE.UTF-8"), ("PATH", "/bin")])),
            vec![("LANG", "de_DE.UTF-8".to_string())]
        );
        assert!(
            session_locale(get(&[("LC_TIME", "C.UTF-8")])).is_empty(),
            "no LANG publishes nothing"
        );
    }

    /// A session of its own unsets every variable it can publish, set or
    /// not, then stops the session target, the AT-SPI registries, the portal
    /// backend, the frontend and its app scopes.
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
            "XAUTHORITY",
            "XDG_SESSION_DESKTOP",
            "XDG_SESSION_CLASS",
            "XDG_SEAT",
            "XDG_SEAT_PATH",
        ];
        let mut unset = argv(&["systemctl", "--user", "unset-environment"]);
        unset.extend(every.iter().map(|s| s.to_string()));
        assert_eq!(
            end_commands(&env(None), true, false),
            vec![
                unset,
                argv(&[
                    "systemctl",
                    "--user",
                    "--no-block",
                    "stop",
                    "reprise-session.target",
                    "dbus-*-org.a11y.atspi.Registry@*.service",
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
            end_commands(&e, true, false).last(),
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
            !end_commands(&env(None), true, false)
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
        let cmds = end_commands(&e, false, false);
        assert!(!cmds.iter().any(|c| c[0] == "loginctl"));
        assert_eq!(
            cmds.last(),
            Some(&argv(&[
                "systemctl",
                "--user",
                "--no-block",
                "stop",
                "reprise-session.target",
                "dbus-*-org.a11y.atspi.Registry@*.service",
                "reprise-portal.service",
                "xdg-desktop-portal.service",
            ]))
        );
        assert_eq!(stale_scopes_command(&e, false, false, false), None);
    }

    /// Scopes a crashed session left are stopped before the next one starts
    /// any; a nested run leaves its parent's alone.
    #[test]
    fn a_session_starts_by_stopping_app_scopes_left_by_the_last() {
        assert_eq!(
            stale_scopes_command(&env(None), true, false, false),
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
        assert_eq!(stale_scopes_command(&nested, true, false, false), None);
        assert!(
            !end_commands(&nested, true, false)
                .concat()
                .contains(&"app-reprise-*.scope".to_string())
        );
    }

    /// A Reprise started from a text login while another Reprise session of the user runs
    /// leaves the user manager's variables and units to that session.
    #[test]
    fn a_text_login_beside_another_reprise_session_publishes_and_restarts_nothing() {
        let e = SessionEnv {
            defers_user_units: true,
            ..env(None)
        };
        assert_eq!(begin_commands(&e), Vec::<Vec<String>>::new());
    }

    #[test]
    fn a_text_login_stops_publishing_once_a_login_managers_reprise_session_runs() {
        let publishes = |e: &SessionEnv, taken_over: bool| {
            record();
            publish(e, || taken_over);
            recorded() == ["dbus-update-activation-environment"]
        };
        let tty = env(None);
        assert!(publishes(&tty, false), "alone, a text login publishes");
        assert!(
            !publishes(&tty, true),
            "taken over, it leaves the variables"
        );
        let dm = SessionEnv {
            logind_session: Some("7".into()),
            ..env(None)
        };
        assert!(
            publishes(&dm, true),
            "a login manager's session always publishes"
        );
        let nested = SessionEnv {
            parent: vec![("WAYLAND_DISPLAY", "wayland-0".into())],
            ..env(None)
        };
        assert!(
            publishes(&nested, true),
            "a nested run publishes to its parent"
        );
    }

    /// The user's other Reprise session keeps the variables, the session target, the portals
    /// and its own app scopes.
    #[test]
    fn a_session_ending_beside_another_reprise_session_stops_only_its_own_scopes() {
        let dm = SessionEnv {
            logind_session: Some("7".into()),
            session_id: Some("7".into()),
            ..env(None)
        };
        assert_eq!(
            end_commands(&dm, true, true),
            vec![
                argv(&[
                    "systemctl",
                    "--user",
                    "--no-block",
                    "stop",
                    "app-reprise-*-s7x*.scope"
                ]),
                argv(&["loginctl", "terminate-session", "7"]),
            ]
        );
        let tty = SessionEnv {
            session_id: Some("9".into()),
            defers_user_units: true,
            ..env(None)
        };
        assert_eq!(
            end_commands(&tty, true, true),
            vec![argv(&[
                "systemctl",
                "--user",
                "--no-block",
                "stop",
                "app-reprise-*-s9x*.scope"
            ])]
        );
        assert_eq!(end_commands(&tty, false, true), Vec::<Vec<String>>::new());
    }

    #[test]
    fn another_reprise_session_is_the_compositor_in_another_online_session_of_the_user() {
        let cg = |id: &str| format!("0::/user.slice/user-1000.slice/session-{id}.scope\n");
        let runs = |comm: &str, cgroup: String| {
            runs_elsewhere(
                "reprise",
                Some("8"),
                &["3", "8"],
                [(format!("{comm}\n"), cgroup)],
            )
        };
        assert!(runs("reprise", cg("3")));
        assert!(!runs("reprise", cg("8")), "its own session");
        assert!(!runs("reprise", cg("5")), "a session that is closing");
        assert!(!runs("reprise-portal", cg("3")), "another program");
        let app =
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-reprise-x-1.scope";
        assert!(!runs("reprise", app.into()), "outside every session");
        assert!(runs_elsewhere(
            "reprise",
            None,
            &["3"],
            [("reprise".into(), cg("3"))]
        ));
    }

    #[test]
    fn of_two_sessions_that_deferred_only_the_one_logind_opened_later_goes_on_deferring() {
        let t1 = (Some(100), "4");
        let t2 = (Some(200), "6");
        assert!(opened_before(t1, Some(t2)), "the later one gives way");
        assert!(!opened_before(t2, Some(t1)), "the older one takes back");
        assert!(
            opened_before((None, "4"), Some(t2)),
            "an unknown time gives way"
        );
        assert!(opened_before(t2, None), "so does an unknown own session");
    }

    #[test]
    fn taking_back_unsets_what_another_session_published_and_this_one_does_not() {
        let tty = SessionEnv {
            login: vec![("XDG_SEAT", "seat0".into())],
            ..SessionEnv::default()
        };
        assert_eq!(
            unset_unpublished(&tty),
            Some(argv(&[
                "systemctl",
                "--user",
                "unset-environment",
                "DISPLAY",
                "XAUTHORITY",
                "XDG_SESSION_DESKTOP",
                "XDG_SESSION_CLASS",
                "XDG_SEAT_PATH"
            ]))
        );
        let every = SessionEnv {
            x11: Some(X11Env::default()),
            login: FORWARDED.map(|k| (k, String::new())).to_vec(),
            ..SessionEnv::default()
        };
        assert_eq!(unset_unpublished(&every), None);
    }

    #[test]
    fn a_compositor_that_exited_unreaped_runs_no_session() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        for _ in 0..500 {
            if exited(pid) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(exited(pid), "an unreaped child");
        child.wait().unwrap();
        assert!(!exited(std::process::id()));
    }

    #[test]
    fn a_second_session_leaves_the_running_ones_scopes() {
        assert_eq!(stale_scopes_command(&env(None), true, true, false), None);
    }

    #[test]
    fn a_restart_after_a_crash_leaves_the_scopes_running() {
        assert_eq!(stale_scopes_command(&env(None), true, false, true), None);
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
        let cmds = end_commands(&e, true, false);
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
        let launch = Launch {
            token: "tok-1".into(),
            by_key: false,
        };
        let c = command("true", &env(None), Some(&launch)).unwrap();
        assert_eq!(
            env_of(&c, "XDG_ACTIVATION_TOKEN"),
            Some(Some("tok-1".into()))
        );
        assert_eq!(
            env_of(&c, "WAYLAND_DISPLAY"),
            Some(Some("wayland-9".into()))
        );
        // No X11 display: DISPLAY and its cookie are removed rather than inherited.
        assert_eq!(env_of(&c, "DISPLAY"), Some(None));
        assert_eq!(env_of(&c, "XAUTHORITY"), Some(None));
    }

    #[test]
    fn a_child_gets_the_proxy_file_and_loses_what_it_stopped_setting() {
        let vars = |v: &[(&str, &str)]| -> Vec<(String, String)> {
            v.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        let mut e = env(None);
        let http = ("http_proxy", "http://a:1");
        assert!(e.set_proxy(vars(&[http, ("no_proxy", "localhost")])));
        assert!(!e.set_proxy(vars(&[http, ("no_proxy", "localhost")])));
        let c = command("true", &e, None).unwrap();
        assert_eq!(env_of(&c, "http_proxy"), Some(Some("http://a:1".into())));
        assert_eq!(env_of(&c, "no_proxy"), Some(Some("localhost".into())));
        assert_eq!(
            env_of(&c, "HTTPS_PROXY"),
            None,
            "one never set is inherited"
        );

        assert!(e.set_proxy(vars(&[http])));
        assert!(e.set_proxy(Vec::new()));
        let c = command("true", &e, None).unwrap();
        assert_eq!(e.proxy_gone, ["http_proxy", "no_proxy"]);
        assert_eq!(env_of(&c, "http_proxy"), Some(None));
        assert_eq!(env_of(&c, "no_proxy"), Some(None));

        assert!(e.set_proxy(vars(&[("no_proxy", "x")])));
        assert_eq!(e.proxy_gone, ["http_proxy"]);
        let c = command("true", &e, None).unwrap();
        assert_eq!(env_of(&c, "no_proxy"), Some(Some("x".into())));
    }

    #[test]
    fn command_marks_only_a_start_by_key() {
        let key = |by_key| {
            let launch = Launch {
                token: "t".into(),
                by_key,
            };
            env_of(
                &command("true", &env(None), Some(&launch)).unwrap(),
                reprisebus::instance::KEYBOARD_LAUNCH_ENV,
            )
        };
        assert_eq!(key(true), Some(Some("1".into())));
        assert_eq!(
            key(false),
            Some(None),
            "a click start removes an inherited mark"
        );
        assert_eq!(
            env_of(
                &command("true", &env(None), None).unwrap(),
                reprisebus::instance::KEYBOARD_LAUNCH_ENV
            ),
            Some(None)
        );
    }

    #[test]
    fn command_removes_stale_token_without_one() {
        // A token inherited from the environment that launched reprise, of
        // either kind, must not leak into children: the compositor is the
        // only source of valid tokens for its own clients.
        let c = command("true", &env(Some(":1")), None).unwrap();
        assert_eq!(env_of(&c, "XDG_ACTIVATION_TOKEN"), Some(None));
        assert_eq!(env_of(&c, "DESKTOP_STARTUP_ID"), Some(None));
        assert_eq!(env_of(&c, "DISPLAY"), Some(Some(":1".into())));
        assert_eq!(
            env_of(&c, "XAUTHORITY"),
            Some(Some("/run/user/1000/reprise/xauth-1".into()))
        );
    }

    /// The desktop name, the colours, the interface, Blue's variant and
    /// the icon theme reach a child through the command, not the process
    /// environment.
    #[test]
    fn command_carries_the_desktop_and_colours() {
        let c = command("true", &env(None), None).unwrap();
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
        let light = command("true", &env(None), None).unwrap();
        assert_eq!(env_of(&light, "QT_QPA_PLATFORMTHEME"), None);
        let dark_scheme = ColorScheme {
            window: Color::rgb(32, 32, 32),
            ..ColorScheme::default()
        };
        let e = SessionEnv {
            colors: dark_scheme.to_env(),
            ..env(None)
        };
        let dark = command("true", &e, None).unwrap();
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
            env_of(
                &command("true", &qt6ct, None).unwrap(),
                "QT_QPA_PLATFORMTHEME"
            ),
            Some(Some("qt6ct".into()))
        );
        let reprise = SessionEnv {
            qt_platform_theme: "reprise".into(),
            qt_generated: Some("qt5ct"),
            ..env(None)
        };
        assert_eq!(
            env_of(
                &command("true", &reprise, None).unwrap(),
                "QT_QPA_PLATFORMTHEME"
            ),
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
            "sh",
            &"sh",
            &[
                "-c",
                "cut -d' ' -f6 /proc/self/stat; readlink /proc/self/fd/0",
            ],
            &env(None),
            None,
        )
        .unwrap();
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

    /// Runs `c` through [`spawn_detached`] and returns what it printed.
    fn detached_output(c: &std::process::Command) -> String {
        use std::io::Read;
        use std::os::fd::{AsFd, FromRawFd, OwnedFd};
        let mut fds = [0; 2];
        // SAFETY: `fds` is the two-fd out-param pipe2 fills.
        assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
        // SAFETY: pipe2 succeeded, so both fds are open and owned by nothing else.
        let (r, w) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        let pid = spawn_detached(c, Some(w.as_fd())).unwrap();
        drop(w);
        let mut out = String::new();
        std::fs::File::from(r).read_to_string(&mut out).unwrap();
        // SAFETY: `pid` is this test's own child; a null status is allowed.
        unsafe { libc::waitpid(pid as libc::pid_t, std::ptr::null_mut(), 0) };
        out
    }

    #[test]
    fn a_detached_spawn_runs_in_its_own_session_with_its_env_and_dir() {
        let mut c = std::process::Command::new("sh");
        c.args([
            "-c",
            "cut -d' ' -f6 /proc/self/stat; readlink /proc/self/fd/0; echo $LAUNCHED ${HOME-unset}; pwd",
        ]);
        c.env("LAUNCHED", "yes").env_remove("HOME").current_dir("/");
        let out = detached_output(&c);
        let mut lines = out.lines();
        let sid: libc::pid_t = lines.next().unwrap().parse().unwrap();
        // SAFETY: getsid(0) only reads the calling process's session id.
        let ours = unsafe { libc::getsid(0) };
        assert_ne!(sid, ours, "the child's session is not the test's");
        assert_eq!(lines.next(), Some("/dev/null"), "stdin is /dev/null");
        assert_eq!(
            lines.next(),
            Some("yes unset"),
            "LAUNCHED set, HOME removed"
        );
        assert_eq!(lines.next(), Some("/"), "runs in /");
    }

    #[test]
    fn a_detached_spawn_starts_with_no_blocked_or_ignored_interrupt() {
        // SAFETY: the mask is this thread's own and restored; SIGINT's disposition is restored
        // before the test returns, and no test relies on it.
        let (old_mask, old_int) = unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            let mut old: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGTERM);
            libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut old);
            (old, libc::signal(libc::SIGINT, libc::SIG_IGN))
        };
        let mut c = std::process::Command::new("sh");
        c.args(["-c", "grep -E '^Sig(Blk|Ign)' /proc/self/status"]);
        let out = detached_output(&c);
        // SAFETY: restores what the block above saved.
        unsafe {
            libc::signal(libc::SIGINT, old_int);
            libc::pthread_sigmask(libc::SIG_SETMASK, &old_mask, std::ptr::null_mut());
        }
        let mask = |key: &str| {
            let line = out.lines().find(|l| l.starts_with(key)).unwrap();
            u64::from_str_radix(line[key.len()..].trim(), 16).unwrap()
        };
        assert_eq!(mask("SigBlk:"), 0, "no signal is blocked");
        assert_eq!(
            mask("SigIgn:") & (1 << (libc::SIGINT - 1)),
            0,
            "SIGINT is not ignored"
        );
    }

    #[test]
    fn a_launch_refused_by_the_env_reaches_no_start() {
        let refusing = SessionEnv {
            refuses_programs: true,
            ..env(None)
        };
        record();
        assert!(!spawn("true", &refusing, None));
        let argv = ["true"];
        assert_eq!(
            try_spawn_argv_in(&argv, None, None, &refusing, None).unwrap(),
            None
        );
        assert_eq!(
            try_spawn_held(&argv, "true", None, &refusing, None).unwrap(),
            None
        );
        assert!(command("true", &refusing, None).is_none());
        assert_eq!(recorded(), Vec::<String>::new());
        assert!(spawn("true", &env(None), None));
        assert_eq!(recorded(), ["true"], "a session's launch starts");
    }

    #[test]
    fn a_detached_spawn_of_a_missing_program_returns_the_exec_error() {
        let c = std::process::Command::new("/srv/no-such-program");
        let e = spawn_detached(&c, None).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
    }

    /// A child of a compositor that ignores SIGINT still gets SIGINT's
    /// default action.
    #[test]
    fn detached_child_does_not_inherit_an_ignored_interrupt() {
        let mut c = command_argv(
            "sh",
            &"sh",
            &["-c", "grep SigIgn /proc/self/status"],
            &env(None),
            None,
        )
        .unwrap();
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
