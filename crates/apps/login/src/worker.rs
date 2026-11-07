//! The per-login worker, `reprise-login --worker <mode> <user>`: one process and one PAM
//! handle from `pam_authenticate` to `pam_close_session`, talking to the daemon over fd 3.

use std::cell::Cell;
use std::ffi::{CString, c_char, c_int};
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use repriselogin::frame::{read_msg, write_msg};
use repriselogin::greeter::{AuthKind, ErrorKind};
use repriselogin::{SESSION_SOCKET, SOCKET_ENV};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::pam::{Converse, Pam, PamError};
use crate::users::{self, PasswdEntry};
use crate::vt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Login,
    /// Checks a password and ends after `Authenticated`, starting nothing.
    Reauth,
    Greeter,
    Autologin,
}

impl Mode {
    pub fn service(self) -> &'static str {
        match self {
            Mode::Login | Mode::Reauth => "reprise-login",
            Mode::Greeter => "reprise-login-greeter",
            Mode::Autologin => "reprise-login-autologin",
        }
    }

    /// The `--worker` argument naming the mode.
    pub fn as_arg(self) -> &'static str {
        match self {
            Mode::Login => "login",
            Mode::Reauth => "reauth",
            Mode::Greeter => "greeter",
            Mode::Autologin => "autologin",
        }
    }

    pub fn parse(arg: &str) -> Option<Mode> {
        [Mode::Login, Mode::Reauth, Mode::Greeter, Mode::Autologin]
            .into_iter()
            .find(|m| m.as_arg() == arg)
    }

    /// Greeter and autologin stacks have no prompting module, so only these relay prompts.
    fn relays_prompts(self) -> bool {
        matches!(self, Mode::Login | Mode::Reauth)
    }
}

/// The daemon to the worker. `Answer` carries `None` for Info and Error.
#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
pub enum ToWorker {
    Answer(Option<Zeroizing<String>>),
    Start { vt: u16 },
    Cancel,
}

impl fmt::Debug for ToWorker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToWorker::Answer(Some(_)) => f.write_str("Answer(<redacted>)"),
            ToWorker::Answer(None) => f.write_str("Answer(None)"),
            ToWorker::Start { vt } => f.debug_struct("Start").field("vt", vt).finish(),
            ToWorker::Cancel => f.write_str("Cancel"),
        }
    }
}

/// The worker to the daemon. The worker's exit status is 0 only when its session ended
/// with 0, or when a `Reauth` succeeded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FromWorker {
    Prompt { kind: AuthKind, text: String },
    Authenticated,
    Started { session_id: String, uid: u32 },
    Failed { kind: ErrorKind, text: String },
}

/// The conversation over the worker socket: each message goes out as `Prompt` and waits for
/// an `Answer`. `Cancel`, a closed socket or a bad frame ends it, and later prompts get `None`.
pub struct SocketConverse {
    sock: UnixStream,
    ended: Rc<Cell<bool>>,
}

impl SocketConverse {
    pub fn new(sock: UnixStream) -> SocketConverse {
        SocketConverse {
            sock,
            ended: Rc::default(),
        }
    }

    /// Set once the conversation has ended; shared so the worker can read it after PAM returns.
    pub fn ended(&self) -> Rc<Cell<bool>> {
        self.ended.clone()
    }
}

impl Converse for SocketConverse {
    fn prompt(&mut self, kind: AuthKind, text: &str) -> Option<Zeroizing<String>> {
        if self.ended.get() {
            return None;
        }
        let prompt = FromWorker::Prompt {
            kind,
            text: text.to_owned(),
        };
        let answer = match write_msg(&mut self.sock, &prompt) {
            Ok(()) => read_msg::<ToWorker>(&mut self.sock),
            Err(e) => Err(e),
        };
        match answer {
            Ok(ToWorker::Answer(answer)) => match kind {
                AuthKind::Visible | AuthKind::Secret => answer,
                AuthKind::Info | AuthKind::Error => None,
            },
            _ => {
                self.ended.set(true);
                None
            }
        }
    }
}

/// The conversation of the greeter and autologin stacks: prompts are refused and text is logged.
struct Unattended;

impl Converse for Unattended {
    fn prompt(&mut self, kind: AuthKind, text: &str) -> Option<Zeroizing<String>> {
        match kind {
            AuthKind::Info | AuthKind::Error => log::info!("pam: {text}"),
            AuthKind::Visible | AuthKind::Secret => log::warn!("pam: refused prompt {text:?}"),
        }
        None
    }
}

const SESSION_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/bin";
const LOCALE_CONF: &str = "/etc/locale.conf";
const JOURNAL_STDOUT: &str = "/run/systemd/journal/stdout";
const JOURNAL_IDENTIFIER: &str = "reprise-session";

/// The session's whole environment: the account's variables, then the PAM environment, whose
/// value wins on a clash. Nothing comes from the worker's own environment.
pub fn session_env(
    user: &PasswdEntry,
    mode: Mode,
    pam_env: Vec<(String, String)>,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = [
        ("HOME", user.home.to_string_lossy().into_owned()),
        ("USER", user.name.clone()),
        ("LOGNAME", user.name.clone()),
        ("SHELL", shell(user).to_owned()),
        ("PATH", SESSION_PATH.to_owned()),
        ("XDG_CURRENT_DESKTOP", "reprise".to_owned()),
        ("DESKTOP_SESSION", "reprise".to_owned()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    if mode != Mode::Greeter {
        env.push((SOCKET_ENV.to_owned(), SESSION_SOCKET.to_owned()));
    }
    for (key, value) in pam_env {
        match env.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => env.push((key, value)),
        }
    }
    env
}

/// The account's login shell, `/bin/sh` when passwd names none.
fn shell(user: &PasswdEntry) -> &str {
    if user.shell.is_empty() {
        "/bin/sh"
    } else {
        &user.shell
    }
}

/// The session's program. A bare name is looked up on the session's `PATH`.
pub fn session_argv(user: &PasswdEntry, mode: Mode) -> Vec<String> {
    let argv: &[&str] = match mode {
        Mode::Greeter => &["reprise", "--greeter"],
        Mode::Login | Mode::Reauth | Mode::Autologin => &[shell(user), "-l", "-c", "exec reprise"],
    };
    argv.iter().map(|s| s.to_string()).collect()
}

/// `LANG` and `LC_*` assignments from locale.conf text.
pub fn locale_env(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| line.trim().split_once('='))
        .filter(|(k, _)| *k == "LANG" || k.starts_with("LC_"))
        .map(|(k, v)| (k.to_owned(), v.trim_matches('"').to_owned()))
        .collect()
}

/// The first executable `name` on `path`, or `name` itself when it holds a `/`.
pub fn find_program(name: &str, path: &str) -> Option<PathBuf> {
    if name.contains('/') {
        return Some(PathBuf::from(name));
    }
    path.split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| Path::new(dir).join(name))
        .find(|p| {
            p.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

/// Runs one login for `user` and returns the worker's exit status.
pub fn worker_main(mode: Mode, user: &str, mut sock: UnixStream) -> i32 {
    // SAFETY: plain signal disposition change; an inherited SIG_IGN would reap the session itself.
    unsafe { libc::signal(libc::SIGCHLD, libc::SIG_DFL) };
    let mut ended = None;
    let conv: Box<dyn Converse> = if mode.relays_prompts() {
        let Ok(clone) = sock.try_clone() else {
            log::error!("worker: cannot clone the socket");
            return 1;
        };
        let c = SocketConverse::new(clone);
        ended = Some(c.ended());
        Box::new(c)
    } else {
        Box::new(Unattended)
    };
    let cancelled = || ended.as_ref().is_some_and(|e| e.get());

    let mut pam = match Pam::start(mode.service(), user, conv) {
        Ok(pam) => pam,
        Err(e) => return fail(&mut sock, ErrorKind::Failed, &e.to_string()),
    };
    let checked = pam.authenticate().and_then(|()| pam.account());
    if cancelled() {
        return 1;
    }
    if let Err(e) = checked {
        log::warn!("worker: {user} refused: {e}");
        return fail(&mut sock, ErrorKind::Auth, &e.text);
    }
    let account = match pam.user().map(|name| users::lookup(&name)) {
        Some(Ok(Some(account))) => account,
        _ => {
            return fail(
                &mut sock,
                ErrorKind::Failed,
                &format!("{user} has no passwd entry"),
            );
        }
    };
    if write_msg(&mut sock, &FromWorker::Authenticated).is_err() {
        return 1;
    }
    if mode == Mode::Reauth {
        return 0;
    }
    let vt = match read_msg::<ToWorker>(&mut sock) {
        Ok(ToWorker::Start { vt }) if (1..=vt::LAST_VT).contains(&vt) => vt,
        Ok(ToWorker::Start { vt }) => {
            return fail(
                &mut sock,
                ErrorKind::Failed,
                &format!("VT {vt} is out of range"),
            );
        }
        _ => return 1,
    };
    match run_session(&mut pam, mode, &account, vt, &mut sock) {
        Ok(clean) => i32::from(!clean),
        Err(text) => fail(&mut sock, ErrorKind::Failed, &text),
    }
}

/// Tells the daemon why the login failed and gives the exit status for it.
fn fail(sock: &mut UnixStream, kind: ErrorKind, text: &str) -> i32 {
    log::warn!("worker: {text}");
    let _ = write_msg(
        sock,
        &FromWorker::Failed {
            kind,
            text: text.to_owned(),
        },
    );
    1
}

/// Opens the session on `vt`, runs its program to the end and closes it again. `Ok` says
/// whether the program exited with 0.
fn run_session(
    pam: &mut Pam,
    mode: Mode,
    account: &PasswdEntry,
    vt: u16,
    sock: &mut UnixStream,
) -> Result<bool, String> {
    let pam_err = |e: PamError| e.to_string();
    pam.set_tty(&format!("tty{vt}")).map_err(pam_err)?;
    let class = if mode == Mode::Greeter {
        "greeter"
    } else {
        "user"
    };
    let vtnr = vt.to_string();
    for (key, value) in [
        ("XDG_SEAT", "seat0"),
        ("XDG_VTNR", vtnr.as_str()),
        ("XDG_SESSION_CLASS", class),
        ("XDG_SESSION_TYPE", "wayland"),
        ("XDG_SESSION_DESKTOP", "reprise"),
    ] {
        pam.putenv(key, value).map_err(pam_err)?;
    }
    if mode == Mode::Greeter {
        let conf = std::fs::read_to_string(LOCALE_CONF).unwrap_or_default();
        for (key, value) in locale_env(&conf) {
            pam.putenv(&key, &value).map_err(pam_err)?;
        }
    }
    let tty_path = format!("/dev/tty{vt}");
    // Held open for the session's life, so logind's autovt never starts a getty on it.
    let tty = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY | libc::O_CLOEXEC)
        .open(&tty_path)
        .map_err(|e| format!("{tty_path}: {e}"))?;
    vt::activate_and_wait(vt).map_err(|e| format!("VT {vt}: {e}"))?;
    pam.establish_cred().map_err(pam_err)?;
    if let Err(e) = pam.open_session() {
        let _ = pam.delete_cred();
        return Err(e.to_string());
    }
    let clean = run_program(pam, mode, account, &tty_path, sock);
    let closed = pam.close_session();
    let deleted = pam.delete_cred();
    drop(tty);
    for e in [closed.err(), deleted.err()].into_iter().flatten() {
        log::warn!("worker: closing {}'s session: {e}", account.name);
    }
    clean
}

/// Forks the session's program in the open session and waits for it.
fn run_program(
    pam: &mut Pam,
    mode: Mode,
    account: &PasswdEntry,
    tty_path: &str,
    sock: &mut UnixStream,
) -> Result<bool, String> {
    let pam_env = pam.env();
    let Some(session_id) = pam_env
        .iter()
        .find(|(k, _)| k == "XDG_SESSION_ID")
        .map(|(_, v)| v.clone())
    else {
        return Err("pam_systemd registered no session (no XDG_SESSION_ID)".into());
    };
    let launch = Launch::new(account, mode, tty_path, session_env(account, mode, pam_env))?;
    let waited = block_session_signals();
    // SAFETY: the worker is single-threaded; the child makes only async-signal-safe calls
    // before `execve` or `_exit`.
    let child = unsafe { libc::fork() };
    if child < 0 {
        unblock(&waited);
        return Err(format!("fork: {}", std::io::Error::last_os_error()));
    }
    if child == 0 {
        // SAFETY: this is the child right after fork.
        unsafe { launch.exec() };
    }
    drop(launch);
    let started = FromWorker::Started {
        session_id,
        uid: account.uid,
    };
    if write_msg(sock, &started).is_err() {
        log::warn!("worker: the daemon is gone; the session runs on");
    }
    let status = wait_for(child, &waited);
    Ok(status.is_some_and(|s| libc::WIFEXITED(s) && libc::WEXITSTATUS(s) == 0))
}

/// Blocks the signals the parent waits on before `fork`, so none is lost before `wait_for`.
fn block_session_signals() -> libc::sigset_t {
    // SAFETY: sigset operations on a local set, and a mask change of this thread.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGCHLD] {
            libc::sigaddset(&mut set, sig);
        }
        libc::sigprocmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        set
    }
}

fn unblock(set: &libc::sigset_t) {
    // SAFETY: a mask change of this thread with a set built by `block_session_signals`.
    unsafe { libc::sigprocmask(libc::SIG_UNBLOCK, set, std::ptr::null_mut()) };
}

/// Waits for `child`, passing SIGTERM and SIGINT on as SIGTERM. `None` when it can't be reaped.
fn wait_for(child: libc::pid_t, set: &libc::sigset_t) -> Option<c_int> {
    loop {
        let mut status = 0;
        // SAFETY: `status` is a live local.
        let r = unsafe { libc::waitpid(child, &mut status, libc::WNOHANG) };
        if r == child {
            return Some(status);
        }
        if r < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            log::error!("worker: waitpid: {}", std::io::Error::last_os_error());
            return None;
        }
        // SAFETY: `set` holds only blocked signals; no siginfo is asked for.
        let sig = unsafe { libc::sigwaitinfo(set, std::ptr::null_mut()) };
        if sig == libc::SIGTERM || sig == libc::SIGINT {
            // SAFETY: `child` is our unreaped child, so the pid is still its.
            unsafe { libc::kill(child, libc::SIGTERM) };
        }
    }
}

/// Everything the session's child needs, made before `fork`, so the child only makes
/// async-signal-safe calls.
struct Launch {
    path: CString,
    _argv: Vec<CString>,
    argv_ptrs: Vec<*const c_char>,
    _envp: Vec<CString>,
    envp_ptrs: Vec<*const c_char>,
    groups: Vec<libc::gid_t>,
    uid: libc::uid_t,
    gid: libc::gid_t,
    home: CString,
    tty: CString,
    no_new_privs: bool,
    stream: OwnedFd,
}

impl Launch {
    fn new(
        account: &PasswdEntry,
        mode: Mode,
        tty_path: &str,
        env: Vec<(String, String)>,
    ) -> Result<Launch, String> {
        let nul = |what: &str| format!("{what} holds a NUL byte");
        let argv = session_argv(account, mode);
        let path_var = env
            .iter()
            .rev()
            .find(|(k, _)| k == "PATH")
            .map_or(SESSION_PATH, |(_, v)| v.as_str());
        let path = find_program(&argv[0], path_var)
            .ok_or_else(|| format!("{} is not on {path_var}", argv[0]))?;
        let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| nul("program"))?;
        let argv: Vec<CString> = argv
            .into_iter()
            .map(CString::new)
            .collect::<Result<_, _>>()
            .map_err(|_| nul("argv"))?;
        let envp: Vec<CString> = env
            .into_iter()
            .filter_map(|(k, v)| CString::new(format!("{k}={v}")).ok())
            .collect();
        let name = CString::new(account.name.as_str()).map_err(|_| nul("user name"))?;
        Ok(Launch {
            path,
            argv_ptrs: pointers(&argv),
            _argv: argv,
            envp_ptrs: pointers(&envp),
            _envp: envp,
            groups: group_list(&name, account.gid)?,
            uid: account.uid,
            gid: account.gid,
            home: CString::new(account.home.as_os_str().as_bytes()).map_err(|_| nul("home"))?,
            tty: CString::new(tty_path).map_err(|_| nul("tty"))?,
            no_new_privs: mode == Mode::Greeter,
            stream: journal_stream(),
        })
    }

    /// Becomes the session's program. Any failure is written to the stream and exits 1, or
    /// 127 when `execve` itself fails.
    ///
    /// # Safety
    /// Called only in the child right after `fork`.
    unsafe fn exec(&self) -> ! {
        let fail = |msg: &[u8]| -> ! {
            // SAFETY: write and _exit are async-signal-safe; `msg` is a static string.
            unsafe {
                libc::write(2, msg.as_ptr().cast(), msg.len());
                libc::_exit(1)
            }
        };
        // SAFETY: every call is async-signal-safe and every pointer is to data `Launch` made
        // before the fork, which the child's copy of the address space still holds.
        unsafe {
            let mut dfl: libc::sigaction = std::mem::zeroed();
            dfl.sa_sigaction = libc::SIG_DFL;
            for sig in 1..65 {
                libc::sigaction(sig, &dfl, std::ptr::null_mut());
            }
            let mut empty: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut empty);
            libc::sigprocmask(libc::SIG_SETMASK, &empty, std::ptr::null_mut());

            let stream = self.stream.as_raw_fd();
            if libc::dup2(stream, 1) < 0 || libc::dup2(stream, 2) < 0 {
                libc::_exit(1);
            }
            if libc::setsid() < 0 {
                fail(b"reprise-login: setsid failed\n");
            }
            let tty = libc::open(self.tty.as_ptr(), libc::O_RDWR);
            if tty < 0 {
                fail(b"reprise-login: cannot open the session's tty\n");
            }
            if libc::ioctl(tty, libc::TIOCSCTTY, 0) < 0 {
                fail(b"reprise-login: cannot take the tty as controlling terminal\n");
            }
            if libc::dup2(tty, 0) < 0 {
                fail(b"reprise-login: cannot make the tty stdin\n");
            }
            libc::syscall(libc::SYS_close_range, 3, u32::MAX, 0);
            if libc::setgroups(self.groups.len(), self.groups.as_ptr()) < 0
                || libc::setgid(self.gid) < 0
                || libc::setuid(self.uid) < 0
            {
                fail(b"reprise-login: cannot drop to the user's ids\n");
            }
            if libc::getuid() != self.uid || libc::geteuid() != self.uid {
                fail(b"reprise-login: the uid did not change\n");
            }
            if libc::chdir(self.home.as_ptr()) < 0 {
                libc::chdir(c"/".as_ptr());
            }
            if self.no_new_privs && libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) < 0 {
                fail(b"reprise-login: cannot set no_new_privs\n");
            }
            libc::execve(
                self.path.as_ptr(),
                self.argv_ptrs.as_ptr(),
                self.envp_ptrs.as_ptr(),
            );
            let msg = b"reprise-login: cannot run the session's program\n";
            libc::write(2, msg.as_ptr().cast(), msg.len());
            libc::_exit(127)
        }
    }
}

/// NULL-terminated pointers into `strings`, for `execve`.
fn pointers(strings: &[CString]) -> Vec<*const c_char> {
    strings
        .iter()
        .map(|s| s.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect()
}

/// `getgrouplist` for the account: its primary group and every group listing it.
fn group_list(name: &CString, gid: libc::gid_t) -> Result<Vec<libc::gid_t>, String> {
    let mut len: c_int = 64;
    loop {
        let mut groups = vec![0; len as usize];
        let mut n = len;
        // SAFETY: `groups` holds `n` entries and `name` is NUL-terminated.
        let r = unsafe { libc::getgrouplist(name.as_ptr(), gid, groups.as_mut_ptr(), &mut n) };
        if r >= 0 {
            groups.truncate(n as usize);
            return Ok(groups);
        }
        if len >= 65536 {
            return Err("getgrouplist: too many groups".into());
        }
        len = n.max(len * 2);
    }
}

/// A journal stream for the session's stdout and stderr, or /dev/null when journald has none.
fn journal_stream() -> OwnedFd {
    let header = format!("{JOURNAL_IDENTIFIER}\n\n6\n1\n0\n0\n0\n");
    let stream = UnixStream::connect(JOURNAL_STDOUT).and_then(|mut s| {
        s.write_all(header.as_bytes())?;
        s.shutdown(std::net::Shutdown::Read)?;
        Ok(s)
    });
    match stream {
        Ok(s) => s.into(),
        Err(e) => {
            log::warn!("worker: {JOURNAL_STDOUT}: {e}; the session's output is dropped");
            File::options()
                .write(true)
                .open("/dev/null")
                .expect("/dev/null opens")
                .into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use repriselogin::frame::FrameError;
    use std::time::Duration;

    fn anna() -> PasswdEntry {
        PasswdEntry {
            name: "anna".into(),
            uid: 1000,
            gid: 1000,
            gecos: "Anna Berg".into(),
            home: PathBuf::from("/home/anna"),
            shell: "/bin/bash".into(),
        }
    }

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn modes_name_their_pam_service_and_argument() {
        let all = [Mode::Login, Mode::Reauth, Mode::Greeter, Mode::Autologin];
        let services: Vec<_> = all.iter().map(|m| m.service()).collect();
        assert_eq!(
            services,
            [
                "reprise-login",
                "reprise-login",
                "reprise-login-greeter",
                "reprise-login-autologin"
            ]
        );
        for m in all {
            assert_eq!(Mode::parse(m.as_arg()), Some(m));
        }
        assert_eq!(Mode::parse("root"), None);
    }

    #[test]
    fn a_secret_prompt_round_trips_over_the_socket() {
        let (worker, mut daemon) = UnixStream::pair().unwrap();
        let daemon = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for answer in [Some("hunter2"), None] {
                seen.push(read_msg::<FromWorker>(&mut daemon).unwrap());
                let answer = answer.map(|a| Zeroizing::new(a.to_owned()));
                write_msg(&mut daemon, &ToWorker::Answer(answer)).unwrap();
            }
            seen
        });
        let mut conv = SocketConverse::new(worker);
        let answer = conv.prompt(AuthKind::Secret, "Password:");
        assert_eq!(answer.as_deref().map(String::as_str), Some("hunter2"));
        assert_eq!(conv.prompt(AuthKind::Info, "Welcome"), None);
        assert!(!conv.ended().get());
        assert_eq!(
            daemon.join().unwrap(),
            [
                FromWorker::Prompt {
                    kind: AuthKind::Secret,
                    text: "Password:".into()
                },
                FromWorker::Prompt {
                    kind: AuthKind::Info,
                    text: "Welcome".into()
                },
            ]
        );
    }

    #[test]
    fn cancel_while_prompting_ends_the_conversation() {
        let (worker, mut daemon) = UnixStream::pair().unwrap();
        worker
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut conv = SocketConverse::new(worker);
        let ended = conv.ended();
        let daemon = std::thread::spawn(move || {
            let first = read_msg::<FromWorker>(&mut daemon).unwrap();
            write_msg(&mut daemon, &ToWorker::Cancel).unwrap();
            (first, read_msg::<FromWorker>(&mut daemon))
        });
        assert_eq!(conv.prompt(AuthKind::Secret, "Password:"), None);
        assert!(ended.get());
        assert_eq!(conv.prompt(AuthKind::Secret, "Password:"), None);
        drop(conv);
        let (first, after) = daemon.join().unwrap();
        assert!(matches!(first, FromWorker::Prompt { .. }), "{first:?}");
        assert_eq!(after.unwrap_err(), FrameError::Closed);
    }

    #[test]
    fn a_closed_socket_ends_the_conversation() {
        let (worker, daemon) = UnixStream::pair().unwrap();
        drop(daemon);
        let mut conv = SocketConverse::new(worker);
        assert_eq!(conv.prompt(AuthKind::Visible, "login:"), None);
        assert!(conv.ended().get());
    }

    #[test]
    fn worker_messages_round_trip() {
        use repriselogin::frame::{decode, encode};
        let to = [
            ToWorker::Answer(Some(Zeroizing::new("pw".into()))),
            ToWorker::Answer(None),
            ToWorker::Start { vt: 7 },
            ToWorker::Cancel,
        ];
        for m in to {
            let frame = encode(&m).unwrap();
            assert_eq!(decode::<ToWorker>(&frame[4..]).unwrap(), m);
        }
        let from = [
            FromWorker::Prompt {
                kind: AuthKind::Visible,
                text: "login:".into(),
            },
            FromWorker::Authenticated,
            FromWorker::Started {
                session_id: "3".into(),
                uid: 1000,
            },
            FromWorker::Failed {
                kind: ErrorKind::Auth,
                text: "Authentication failure".into(),
            },
        ];
        for m in from {
            let frame = encode(&m).unwrap();
            assert_eq!(decode::<FromWorker>(&frame[4..]).unwrap(), m);
        }
    }

    #[test]
    fn debug_of_an_answer_never_shows_it() {
        let m = ToWorker::Answer(Some(Zeroizing::new("hunter2".into())));
        assert_eq!(format!("{m:?}"), "Answer(<redacted>)");
        assert_eq!(format!("{:?}", ToWorker::Answer(None)), "Answer(None)");
        assert_eq!(
            format!("{:?}", ToWorker::Start { vt: 7 }),
            "Start { vt: 7 }"
        );
    }

    #[test]
    fn session_env_is_exactly_the_listed_variables() {
        let pam = pairs(&[
            ("XDG_SESSION_ID", "3"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ]);
        assert_eq!(
            session_env(&anna(), Mode::Login, pam),
            pairs(&[
                ("HOME", "/home/anna"),
                ("USER", "anna"),
                ("LOGNAME", "anna"),
                ("SHELL", "/bin/bash"),
                ("PATH", "/usr/local/sbin:/usr/local/bin:/usr/bin"),
                ("XDG_CURRENT_DESKTOP", "reprise"),
                ("DESKTOP_SESSION", "reprise"),
                ("REPRISE_LOGIN_SOCKET", "/run/reprise-login/session.sock"),
                ("XDG_SESSION_ID", "3"),
                ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ])
        );
    }

    #[test]
    fn the_pam_environment_wins_a_clash() {
        let env = session_env(&anna(), Mode::Autologin, pairs(&[("PATH", "/opt/bin")]));
        let paths: Vec<_> = env.iter().filter(|(k, _)| k == "PATH").collect();
        assert_eq!(paths, [&("PATH".to_string(), "/opt/bin".to_string())]);
    }

    #[test]
    fn the_greeter_gets_no_login_socket_variable() {
        let env = session_env(&anna(), Mode::Greeter, vec![]);
        assert!(
            env.iter().all(|(k, _)| k != "REPRISE_LOGIN_SOCKET"),
            "{env:?}"
        );
        assert_eq!(env.len(), 7);
    }

    #[test]
    fn session_argv_runs_the_login_shell() {
        let argv = ["/bin/bash", "-l", "-c", "exec reprise"];
        assert_eq!(session_argv(&anna(), Mode::Login), argv);
        assert_eq!(session_argv(&anna(), Mode::Autologin), argv);
        let shell_less = PasswdEntry {
            shell: String::new(),
            ..anna()
        };
        assert_eq!(session_argv(&shell_less, Mode::Login)[0], "/bin/sh");
    }

    #[test]
    fn the_greeter_argv_is_reprise_greeter() {
        assert_eq!(
            session_argv(&anna(), Mode::Greeter),
            ["reprise", "--greeter"]
        );
    }

    #[test]
    fn the_first_executable_on_the_path_wins() {
        let dir = tempfile::tempdir().unwrap();
        let (local, usr) = (dir.path().join("local"), dir.path().join("usr"));
        for (d, mode) in [(&local, 0o644), (&usr, 0o755)] {
            std::fs::create_dir(d).unwrap();
            std::fs::write(d.join("reprise"), "").unwrap();
            std::fs::set_permissions(d.join("reprise"), std::fs::Permissions::from_mode(mode))
                .unwrap();
        }
        let path = format!("{}:{}", local.display(), usr.display());
        assert_eq!(find_program("reprise", &path), Some(usr.join("reprise")));
        std::fs::set_permissions(
            local.join("reprise"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        assert_eq!(find_program("reprise", &path), Some(local.join("reprise")));
        assert_eq!(find_program("nothing", &path), None);
        assert_eq!(
            find_program("/bin/bash", &path),
            Some(PathBuf::from("/bin/bash"))
        );
    }

    #[test]
    fn locale_conf_gives_lang_and_lc_variables() {
        let conf = "# set by localectl\nLANG=de_DE.UTF-8\nLC_TIME=\"en_GB.UTF-8\"\nKEYMAP=de\n\n";
        assert_eq!(
            locale_env(conf),
            pairs(&[("LANG", "de_DE.UTF-8"), ("LC_TIME", "en_GB.UTF-8")])
        );
    }
}
