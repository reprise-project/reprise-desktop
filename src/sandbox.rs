//! The sandbox filter: which globals a client may see and bind.
//!
//! wp-security-context-v1 is how a sandbox (Flatpak) marks the clients
//! it lets in: they connect on a listening socket the sandbox hands us,
//! and every client from it carries that context (`ClientSandbox`). A
//! Flatpak app that skipped the context still shows `/.flatpak-info` in
//! its process root, and a process that is gone or can't be inspected by
//! the time it is accepted counts as sandboxed too. Such a client is shown none of `PRIVILEGED_GLOBALS`,
//! except the data control globals for the apps `[clipboard]
//! trusted_apps` names; `reprise_input_v1` is shown to the installed
//! reprise-portal alone.
//!
//! Every privileged global is made with `filter(name)` as its
//! `can_view`, which Smithay also applies to every bind.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, RwLock};

use smithay::{
    reexports::wayland_server::{Client, Resource},
    wayland::security_context::{
        SecurityContext, SecurityContextHandler, SecurityContextListenerSource,
        SecurityContextState,
    },
};

use crate::compositor::{ClientState, State};

/// The globals a sandboxed client must not see, by interface name: they read
/// the screen, the clipboard, other windows or typed keys, inject input,
/// reshape or recolour the desktop, or keep a window over the others (and so over a
/// permission prompt). The portal reaches the rest on the sandbox's behalf.
pub const PRIVILEGED_GLOBALS: &[&str] = &[
    "reprise_input_v1",
    "zwlr_screencopy_manager_v1",
    "ext_image_copy_capture_manager_v1",
    "ext_output_image_capture_source_manager_v1",
    "ext_foreign_toplevel_image_capture_source_manager_v1",
    "ext_foreign_toplevel_list_v1",
    "zwlr_foreign_toplevel_manager_v1",
    "ext_workspace_manager_v1",
    "zwlr_layer_shell_v1",
    "zwlr_output_manager_v1",
    "zwlr_output_power_manager_v1",
    "zwlr_data_control_manager_v1",
    "ext_data_control_manager_v1",
    "zwlr_gamma_control_manager_v1",
    "zwp_input_method_manager_v2",
    "zwp_virtual_keyboard_manager_v1",
    "zwlr_virtual_pointer_manager_v1",
    "ext_session_lock_manager_v1",
    "wp_security_context_manager_v1",
    "reprise_on_top_v1",
];

pub fn is_privileged(name: &str) -> bool {
    PRIVILEGED_GLOBALS.contains(&name)
}

/// The clipboard managers' globals, which `[clipboard] trusted_apps` opens
/// to the sandboxed apps it names. A password manager in Flatpak copies
/// through them (arboard); its fallback, an X11 selection, is refused
/// while a Wayland window has the focus.
const DATA_CONTROL_GLOBALS: &[&str] = &[
    "zwlr_data_control_manager_v1",
    "ext_data_control_manager_v1",
];

/// The sandbox engine name Flatpak gives its security contexts. Another
/// engine's app id means what that engine says it means, so only Flatpak's
/// is matched against `trusted_apps`.
const FLATPAK_ENGINE: &str = "org.flatpak";

/// Whether a sandboxed client whose sandbox `engine` named it `app_id` may
/// see the privileged global `name`: only a data control global, and only
/// for a Flatpak app in `trusted`.
pub fn sandbox_may_bind(
    name: &str,
    engine: Option<&str>,
    app_id: Option<&str>,
    trusted: &[String],
) -> bool {
    DATA_CONTROL_GLOBALS.contains(&name)
        && engine == Some(FLATPAK_ENGINE)
        && app_id.is_some_and(|id| !id.is_empty() && trusted.iter().any(|t| t == id))
}

pub fn is_portal_exe(exe: &[u8]) -> bool {
    crate::inputproto::is_installed_exe(exe, "reprise-portal")
}

/// Who opened a sandbox socket, in its own words.
#[derive(Debug, Default, Clone)]
pub struct Sandbox {
    pub engine: Option<String>,
    pub app_id: Option<String>,
}

impl From<&SecurityContext> for Sandbox {
    fn from(c: &SecurityContext) -> Sandbox {
        Sandbox {
            engine: c.sandbox_engine.clone(),
            app_id: c.app_id.clone(),
        }
    }
}

/// What a client's process showed of a sandbox as it connected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Process {
    #[default]
    Host,
    /// Its root holds Flatpak's `/.flatpak-info`, which a Flatpak app has even without a
    /// security context.
    Flatpak,
    /// Gone by the time it was inspected, or not ours to inspect.
    Unknown,
}

/// What the filter knows of one client, fixed when it connects.
#[derive(Debug, Default)]
pub struct ClientSandbox {
    /// The sandbox engine and app id of the sandbox socket it came in
    /// on (wp-security-context-v1), when it came in on one.
    pub context: Option<Sandbox>,
    pub process: Process,
    pub pid: Option<i32>,
}

impl ClientSandbox {
    /// A client on `stream`, in the sandbox `context` when it came in on
    /// a sandbox's socket.
    pub fn of_stream(stream: &UnixStream, context: Option<Sandbox>) -> ClientSandbox {
        let pid = smithay::reexports::rustix::net::sockopt::socket_peercred(stream)
            .ok()
            .map(|c| c.pid.as_raw_nonzero().get())
            .filter(|&p| p > 0);
        let process = pid.map_or(Process::Unknown, |p| inspect(stream, p));
        ClientSandbox {
            context,
            process,
            pid,
        }
    }

    pub fn sandboxed(&self) -> bool {
        self.context.is_some() || self.process != Process::Host
    }

    /// The Flatpak app id of its sandbox: its context's when Flatpak
    /// opened that, else the one in its process's `/.flatpak-info`.
    pub fn flatpak_app(&self) -> Option<String> {
        let ctx = self
            .context
            .as_ref()
            .filter(|c| c.engine.as_deref() == Some(FLATPAK_ENGINE))
            .and_then(|c| c.app_id.clone())
            .filter(|id| !id.is_empty());
        ctx.or_else(|| {
            let pid = self.pid.filter(|_| self.process == Process::Flatpak)?;
            reprisefs::flatpak::app_of_pid(pid as u32)
        })
    }

    /// Whether the client runs an installed reprise-portal, read afresh
    /// from its pid. Same-user code that runs the portal binary itself (a
    /// preload, ptrace) still passes: the check keeps other clients from
    /// binding directly, it is no boundary against the user.
    fn is_portal(&self) -> bool {
        #[cfg(test)]
        if self.pid == Some(TEST_PORTAL_PID) {
            return true;
        }
        self.pid.is_some_and(|pid| {
            std::fs::read_link(format!("/proc/{pid}/exe")).is_ok_and(|exe| {
                is_portal_exe(std::os::unix::ffi::OsStrExt::as_bytes(exe.as_os_str()))
            })
        })
    }
}

/// What `stream`'s peer, process `pid` as it connected, shows of a sandbox. Every read goes
/// through `/proc/<pid>`, and counts only if the peer still runs after them: its pidfd says it
/// was not reaped (so the pid was not reused) and its `stat` that it is no zombie.
fn inspect(stream: &UnixStream, pid: i32) -> Process {
    let Some(pidfd) = peer_pidfd(stream, pid) else {
        return Process::Unknown;
    };
    let Ok(dir) = std::fs::File::open(format!("/proc/{pid}")) else {
        return Process::Unknown;
    };
    let Some(root) = open_at(&dir, c"root", libc::O_PATH | libc::O_DIRECTORY) else {
        return Process::Unknown;
    };
    let flatpak = exists_at(&root, c".flatpak-info");
    let running = read_at(&dir, c"stat").is_some_and(|s| runs(&s));
    if !running || !alive(&pidfd) {
        return Process::Unknown;
    }
    match flatpak {
        Some(true) => Process::Flatpak,
        Some(false) => Process::Host,
        None => Process::Unknown,
    }
}

fn open_at(dir: &impl AsRawFd, name: &std::ffi::CStr, flags: libc::c_int) -> Option<OwnedFd> {
    // SAFETY: `name` is NUL-terminated and `dir` an open directory.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags | libc::O_CLOEXEC) };
    // SAFETY: a fresh fd nothing else owns.
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
}

fn read_at(dir: &impl AsRawFd, name: &std::ffi::CStr) -> Option<String> {
    let fd = open_at(dir, name, libc::O_RDONLY)?;
    std::io::read_to_string(std::fs::File::from(fd)).ok()
}

/// Whether `name` exists under `dir`; None when that can't be told.
fn exists_at(dir: &impl AsRawFd, name: &std::ffi::CStr) -> Option<bool> {
    // SAFETY: an all-zero stat is valid for fstatat to fill.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `name` is NUL-terminated, `st` live, `dir` an open directory.
    let r = unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            name.as_ptr(),
            &mut st,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if r == 0 {
        return Some(true);
    }
    (std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT)).then_some(false)
}

/// Whether `/proc/<pid>/stat` describes a process that has not exited: its state, after the
/// command's closing parenthesis, is neither zombie nor dead.
fn runs(stat: &str) -> bool {
    stat.rsplit_once(')')
        .and_then(|(_, rest)| rest.trim_start().chars().next())
        .is_some_and(|state| !matches!(state, 'Z' | 'X' | 'x'))
}

/// `SO_PEERPIDFD` (Linux 6.5): a pidfd for the process that connected.
const SO_PEERPIDFD: libc::c_int = 77;

/// A pidfd for `stream`'s peer: `SO_PEERPIDFD`, or on a kernel without it `pidfd_open` of the
/// `SO_PEERCRED` pid, which a pid reused before this call fools. None when the peer is gone.
fn peer_pidfd(stream: &UnixStream, pid: i32) -> Option<OwnedFd> {
    let mut fd: libc::c_int = -1;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: `fd` and `len` are live and sized for the option's int.
    let r = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            SO_PEERPIDFD,
            (&raw mut fd).cast(),
            &mut len,
        )
    };
    if r == 0 && fd >= 0 {
        // SAFETY: the kernel handed us a fresh pidfd nothing else owns.
        return Some(unsafe { OwnedFd::from_raw_fd(fd) });
    }
    if std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOPROTOOPT) {
        return None;
    }
    // SAFETY: pidfd_open takes a pid and flags and returns a new fd or -1.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    // SAFETY: a fresh fd nothing else owns.
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd as libc::c_int) })
}

/// Whether the process `pidfd` names has not been reaped.
fn alive(pidfd: &OwnedFd) -> bool {
    // SAFETY: signal 0 through a pidfd only checks that the process exists.
    let r = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            0,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    r == 0
}

/// The pid a harness client takes to pass as the installed portal.
#[cfg(test)]
pub const TEST_PORTAL_PID: i32 = i32::MAX;

/// Whether a client may see and bind the global `name`: a privileged
/// global is hidden from a sandboxed client, except data control for the
/// sandboxed apps in `trusted`, and `reprise_input_v1` from every client
/// but the portal (`is_portal`, asked only then). Every other global
/// stays visible.
pub fn visible(
    name: &str,
    sandbox: &ClientSandbox,
    trusted: &[String],
    is_portal: impl FnOnce() -> bool,
) -> bool {
    if !is_privileged(name) {
        return true;
    }
    if sandbox.sandboxed() {
        // The engine and app id are the sandbox's word (Flatpak's), not
        // the app's: a client that already has a security context gets
        // no manager to open another under someone else's id.
        let ctx = sandbox.context.as_ref();
        let app_id = ctx.and_then(|c| c.app_id.as_deref());
        let engine = ctx.and_then(|c| c.engine.as_deref());
        let allowed = sandbox_may_bind(name, engine, app_id, trusted);
        if !allowed && DATA_CONTROL_GLOBALS.contains(&name) {
            log::debug!(
                "hiding {name} from sandboxed {}; [clipboard] trusted_apps opens it",
                app_id.unwrap_or("client")
            );
        }
        return allowed;
    }
    name != "reprise_input_v1" || is_portal()
}

/// `[clipboard] trusted_apps`, which the filters read on every bind.
static TRUSTED: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// Take `[clipboard] trusted_apps` from a (re)loaded config.
pub fn set_trusted(apps: &[String]) {
    if let Ok(mut t) = TRUSTED.write() {
        apps.clone_into(&mut t);
    }
}

pub fn client_may_see(client: &Client, name: &str) -> bool {
    let Some(data) = client.get_data::<ClientState>() else {
        return true;
    };
    let trusted = TRUSTED.read().map(|t| t.clone()).unwrap_or_default();
    visible(name, &data.sandbox, &trusted, || data.sandbox.is_portal())
}

pub fn filter(name: &'static str) -> impl Fn(&Client) -> bool + Clone + Send + Sync + 'static {
    move |client| client_may_see(client, name)
}

/// wp-security-context-v1, offered to clients outside a sandbox only: a
/// sandboxed client cannot open a sandbox of its own under another id.
pub fn security_context_state(
    dh: &smithay::reexports::wayland_server::DisplayHandle,
) -> SecurityContextState {
    SecurityContextState::new::<State, _>(dh, filter("wp_security_context_manager_v1"))
}

/// A sandbox opened a listening socket: every client on it carries the
/// sandbox's context.
impl SecurityContextHandler for State {
    fn context_created(&mut self, source: SecurityContextListenerSource, context: SecurityContext) {
        log::info!(
            "sandbox {} for {} opened a socket",
            context.sandbox_engine.as_deref().unwrap_or("(unnamed)"),
            context.app_id.as_deref().unwrap_or("(no app id)")
        );
        let sandbox = Sandbox::from(&context);
        let inserted = self.handle.insert_source(source, move |stream, _, state| {
            let data = Arc::new(ClientState::of_stream(&stream, Some(sandbox.clone())));
            if let Err(e) = state.dh.insert_client(stream, data) {
                log::warn!("sandboxed client refused: {e}");
            }
        });
        if let Err(e) = inserted {
            log::warn!("sandbox socket: {}", e.error);
        }
    }
}

/// The Flatpak app id of the sandbox `win`'s client runs in; an X11
/// window's is its X client's, by the host pid the X server reports
/// (XRes), since `_NET_WM_PID` is the pid inside the sandbox. Err when the
/// XRes query fails.
pub fn window_app(win: &crate::window::Window) -> Result<Option<String>, ()> {
    match win.x11() {
        Some(x) => {
            let pid = x.get_client_pid().ok().filter(|&p| p > 0).ok_or(())?;
            Ok(reprisefs::flatpak::app_of_pid(pid))
        }
        None => Ok(win
            .wl_client()
            .and_then(|c| c.get_data::<ClientState>()?.sandbox.flatpak_app())),
    }
}

pub fn resource_sandboxed(resource: &impl Resource) -> bool {
    resource.client().is_some_and(|c| client_sandboxed(&c))
}

pub fn client_sandboxed(client: &Client) -> bool {
    client
        .get_data::<ClientState>()
        .is_some_and(|d| d.sandbox.sandboxed())
}

/// Where the document portal exports the files it grants sandboxed programs.
pub fn document_root() -> std::path::PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            // SAFETY: getuid cannot fail.
            format!("/run/user/{}", unsafe { libc::getuid() }).into()
        });
    runtime.join("doc")
}

/// Whether `path` lies under [`document_root`]: a sandboxed program can name a host file only
/// there.
pub fn portal_exported(path: &std::path::Path) -> bool {
    path.starts_with(document_root())
        && path
            .components()
            .all(|c| c != std::path::Component::ParentDir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_sandbox(engine: &str, app_id: &str) -> ClientSandbox {
        ClientSandbox {
            context: Some(Sandbox {
                engine: Some(engine.into()),
                app_id: Some(app_id.into()),
            }),
            process: Process::Host,
            pid: None,
        }
    }

    /// A stream accepted from a child process that connected and exited, reaped before the
    /// accept with `reap`, else left a zombie (unless another test's reaper takes it).
    fn from_exited_child(reap: bool) -> UnixStream {
        use std::os::unix::ffi::OsStrExt;
        let path = crate::harness::scratch().join(format!("exited-peer-{reap}"));
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        // SAFETY: an all-zero sockaddr_un is valid; the path fits, being a short scratch path.
        let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (d, s) in addr.sun_path.iter_mut().zip(path.as_os_str().as_bytes()) {
            *d = *s as libc::c_char;
        }
        // SAFETY: the child makes only async-signal-safe calls before `_exit`.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork");
        if pid == 0 {
            // SAFETY: as above; `addr` was built before the fork.
            unsafe {
                let fd = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0);
                let len = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
                libc::connect(fd, (&raw const addr).cast(), len);
                libc::_exit(0);
            }
        }
        let flags = libc::WEXITED | if reap { 0 } else { libc::WNOWAIT };
        // SAFETY: an all-zero siginfo_t is valid for waitid to fill.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is live; ECHILD only means another reaper took the child first.
        unsafe { libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, flags) };
        listener.accept().unwrap().0
    }

    #[test]
    fn a_peer_gone_before_its_accept_is_sandboxed() {
        let s = ClientSandbox::of_stream(&from_exited_child(true), None);
        assert!(s.pid.is_some(), "the kernel still names the peer's pid");
        assert!(s.sandboxed(), "a reaped peer can't be inspected");
        assert!(!visible("zwp_virtual_keyboard_manager_v1", &s, &[], || {
            false
        }));
    }

    #[test]
    fn a_peer_that_exited_unreaped_is_sandboxed() {
        let s = ClientSandbox::of_stream(&from_exited_child(false), None);
        assert!(s.sandboxed(), "a zombie's root can't be inspected");
        if let Some(pid) = s.pid {
            // SAFETY: reaps the test's own child; ECHILD if a reaper took it already.
            unsafe { libc::waitpid(pid, std::ptr::null_mut(), 0) };
        }
    }

    #[test]
    fn a_process_state_is_read_after_the_last_parenthesis() {
        assert!(runs("42 (foot) S 1 42"));
        assert!(
            !runs("42 (a) Z (b) Z 1"),
            "a command may hold a parenthesis"
        );
        assert!(runs("42 (a) Z (b) R 1"));
        assert!(!runs("42 (foot) X 1"));
        assert!(!runs(""));
    }

    #[test]
    fn a_live_peer_outside_a_sandbox_is_not_sandboxed() {
        let (ours, _theirs) = UnixStream::pair().unwrap();
        let s = ClientSandbox::of_stream(&ours, None);
        assert_eq!(s.process, Process::Host);
        assert!(!s.sandboxed());
    }

    #[test]
    fn unprivileged_globals_are_shown_to_everyone() {
        let s = in_sandbox("org.flatpak", "org.example.App");
        assert!(visible("wl_compositor", &s, &[], || false));
        assert!(visible(
            "xdg_wm_base",
            &ClientSandbox::default(),
            &[],
            || false
        ));
    }

    #[test]
    fn a_sandboxed_client_sees_no_privileged_global() {
        let s = in_sandbox("org.flatpak", "org.example.App");
        for name in PRIVILEGED_GLOBALS {
            assert!(!visible(name, &s, &[], || true), "{name} is hidden");
        }
    }

    #[test]
    fn a_flatpak_without_a_context_is_sandboxed_too() {
        let s = ClientSandbox {
            process: Process::Flatpak,
            ..ClientSandbox::default()
        };
        assert!(!visible(
            "ext_image_copy_capture_manager_v1",
            &s,
            &[],
            || false
        ));
    }

    #[test]
    fn trusted_apps_open_data_control_and_nothing_else() {
        let trusted = vec!["org.keepassxc.KeePassXC".to_string()];
        let s = in_sandbox("org.flatpak", "org.keepassxc.KeePassXC");
        assert!(visible("ext_data_control_manager_v1", &s, &trusted, || {
            false
        }));
        assert!(visible(
            "zwlr_data_control_manager_v1",
            &s,
            &trusted,
            || false
        ));
        assert!(!visible("zwlr_screencopy_manager_v1", &s, &trusted, || {
            false
        }));
        let other = in_sandbox("org.flatpak", "org.example.App");
        assert!(!visible(
            "ext_data_control_manager_v1",
            &other,
            &trusted,
            || false
        ));
    }

    #[test]
    fn only_flatpaks_app_id_is_trusted() {
        let trusted = vec!["org.keepassxc.KeePassXC".to_string()];
        let s = in_sandbox("org.example.engine", "org.keepassxc.KeePassXC");
        assert!(!visible(
            "ext_data_control_manager_v1",
            &s,
            &trusted,
            || false
        ));
        assert!(!sandbox_may_bind(
            "ext_data_control_manager_v1",
            Some("org.flatpak"),
            Some(""),
            &[String::new()]
        ));
    }

    #[test]
    fn the_input_global_is_the_portals_alone() {
        let host = ClientSandbox::default();
        assert!(visible("reprise_input_v1", &host, &[], || true));
        assert!(!visible("reprise_input_v1", &host, &[], || false));
        assert!(visible("zwlr_screencopy_manager_v1", &host, &[], || false));
    }

    #[test]
    fn the_portal_is_an_installed_binary() {
        assert!(is_portal_exe(b"/usr/bin/reprise-portal"));
        assert!(is_portal_exe(b"/usr/local/bin/reprise-portal (deleted)"));
        assert!(!is_portal_exe(b"/home/u/reprise-portal"));
    }

    #[test]
    fn a_document_portal_path_climbing_out_of_it_is_not_exported() {
        let root = document_root();
        assert!(portal_exported(&root.join("abc/x.png")));
        assert!(!portal_exported(&root.join("abc/../../../etc/passwd")));
        assert!(!portal_exported(std::path::Path::new("/home/u/x.png")));
    }
}
