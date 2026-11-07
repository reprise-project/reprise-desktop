//! The per-login worker, `reprise-login --worker <mode>`: one process and one PAM handle from
//! `pam_authenticate` to `pam_close_session`, talking to the daemon over fd 3.

use std::cell::Cell;
use std::ffi::{CString, c_char, c_int};
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use repriselogin::frame::{FrameError, read_msg, write_msg};
use repriselogin::greeter::{AuthKind, ErrorKind};
use repriselogin::{GREETER_USER, SESSION_SOCKET, SOCKET_ENV};
use serde::de::DeserializeOwned;
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
    /// Checks a remote login's password, then opens its session in the background.
    Remote,
    /// Runs the account step of a Network Level Authentication login, then opens its session.
    RemoteNla,
    /// Checks a login password and starts nothing.
    Check,
}

/// Every mode, in `--worker` order.
const MODES: [Mode; 7] = [
    Mode::Login,
    Mode::Reauth,
    Mode::Greeter,
    Mode::Autologin,
    Mode::Remote,
    Mode::RemoteNla,
    Mode::Check,
];

impl Mode {
    pub fn service(self) -> &'static str {
        match self {
            Mode::Login | Mode::Reauth => "reprise-login",
            Mode::Greeter => "reprise-login-greeter",
            Mode::Autologin => "reprise-login-autologin",
            Mode::Remote => "reprise-login-remote",
            Mode::RemoteNla => "reprise-login-remote-nla",
            Mode::Check => "reprise-login-check",
        }
    }

    /// The `--worker` argument naming the mode.
    pub fn as_arg(self) -> &'static str {
        match self {
            Mode::Login => "login",
            Mode::Reauth => "reauth",
            Mode::Greeter => "greeter",
            Mode::Autologin => "autologin",
            Mode::Remote => "remote",
            Mode::RemoteNla => "remote-nla",
            Mode::Check => "check",
        }
    }

    pub fn parse(arg: &str) -> Option<Mode> {
        MODES.into_iter().find(|m| m.as_arg() == arg)
    }

    /// Whether a worker of this mode may run for `user`. The greeter's and autologin's stacks
    /// ask no password, so they run only for the greeter's account and `autologin_user`. The
    /// daemon decides who gets the remote and check modes.
    pub fn runs_for(self, user: &str, autologin_user: Option<&str>) -> bool {
        match self {
            Mode::Greeter => user == GREETER_USER,
            Mode::Autologin => autologin_user == Some(user),
            Mode::Login | Mode::Reauth | Mode::Remote | Mode::RemoteNla | Mode::Check => true,
        }
    }

    /// The greeter, autologin and NLA stacks have no prompting module, so only these relay
    /// prompts.
    fn relays_prompts(self) -> bool {
        matches!(
            self,
            Mode::Login | Mode::Reauth | Mode::Remote | Mode::Check
        )
    }

    /// Whether the worker waits for `Start` after `Authenticated` and opens a session.
    fn opens_session(self) -> bool {
        !matches!(self, Mode::Reauth | Mode::Check)
    }
}

/// The daemon to the worker. `Answer` carries `None` for Info and Error.
#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
pub enum ToWorker {
    /// The first frame. The user travels here, not in argv, which every local account can read.
    Begin {
        user: String,
        autologin_user: Option<String>,
        /// The greeter's `--greeter-socket`.
        greeter_socket: bool,
        /// The remote client's address, for `PAM_RHOST`.
        rhost: Option<String>,
    },
    Answer(Option<Zeroizing<String>>),
    Start {
        vt: u16,
        /// Opens the session without taking the console to its VT.
        background: bool,
    },
    Cancel,
}

impl fmt::Debug for ToWorker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToWorker::Begin { .. } => f.write_str("Begin { <redacted> }"),
            ToWorker::Answer(Some(_)) => f.write_str("Answer(<redacted>)"),
            ToWorker::Answer(None) => f.write_str("Answer(None)"),
            ToWorker::Start { vt, background } => (f.debug_struct("Start"))
                .field("vt", vt)
                .field("background", background)
                .finish(),
            ToWorker::Cancel => f.write_str("Cancel"),
        }
    }
}

/// The worker to the daemon. The worker's exit status is 0 only when its session ended
/// with 0, or when a `Reauth` or `Check` succeeded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FromWorker {
    Prompt {
        kind: AuthKind,
        text: String,
    },
    /// The account PAM resolved after authentication, which may differ from the entered name.
    Authenticated {
        user: String,
        uid: u32,
    },
    Started {
        session_id: String,
        uid: u32,
    },
    Failed {
        kind: ErrorKind,
        text: String,
    },
}

/// The conversation over the worker socket: each message goes out as `Prompt` and waits for an
/// `Answer`. `Cancel`, a stop, a closed socket or a bad frame ends it; later prompts get `None`.
pub struct SocketConverse {
    sock: UnixStream,
    ended: Rc<Cell<bool>>,
    stop: &'static AtomicBool,
}

impl SocketConverse {
    fn stoppable(sock: UnixStream, stop: &'static AtomicBool) -> SocketConverse {
        SocketConverse {
            sock,
            ended: Rc::default(),
            stop,
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
            Ok(()) => read_until_stopped::<ToWorker>(&self.sock, self.stop),
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

/// The conversation of the stacks with no prompting module: prompts are refused, which sets
/// `refused`, and text is logged.
struct Unattended {
    refused: Rc<Cell<bool>>,
}

impl Converse for Unattended {
    fn prompt(&mut self, kind: AuthKind, text: &str) -> Option<Zeroizing<String>> {
        match kind {
            AuthKind::Info | AuthKind::Error => log::info!("pam: {text}"),
            AuthKind::Visible | AuthKind::Secret => {
                log::warn!("pam: refused prompt {text:?}");
                self.refused.set(true);
            }
        }
        None
    }
}

/// The worker's log stream, the daemon's stderr. Its stdout and stderr are `/dev/null`, which
/// is what a helper a PAM module starts inherits.
pub const LOG_FD: c_int = 4;

const SESSION_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/bin";
const LOCALE_CONF: &str = "/etc/locale.conf";
const JOURNAL_STDOUT: &str = "/run/systemd/journal/stdout";
const JOURNAL_IDENTIFIER: &str = "reprise-session";
/// The worker socket's read timeout: the longest a stop waits on a read it raced into.
const STOP_POLL: Duration = Duration::from_millis(200);
/// The one text the daemon gets for any `pam_authenticate` failure, so an unknown user and
/// a wrong password look the same.
pub const AUTH_FAILED: &str = "The user name or password is incorrect.";
/// The text of a login whose stack asked an unattended conversation for input.
const REFUSED_PROMPT: &str = "the PAM stack asked for input this login cannot give";

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
pub fn session_argv(user: &PasswdEntry, mode: Mode, greeter_socket: bool) -> Vec<String> {
    let argv: &[&str] = match mode {
        Mode::Greeter if greeter_socket => &["reprise", "--greeter", "--greeter-socket"],
        Mode::Greeter => &["reprise", "--greeter"],
        Mode::Login
        | Mode::Reauth
        | Mode::Autologin
        | Mode::Remote
        | Mode::RemoteNla
        | Mode::Check => &[shell(user), "-l", "-c", "exec reprise"],
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

/// Runs one login for the user the daemon's `Begin` names and returns the worker's exit status.
pub fn worker_main(mode: Mode, mut sock: UnixStream) -> i32 {
    // SAFETY: plain signal disposition change; an inherited SIG_IGN would reap the session itself.
    if unsafe { libc::signal(libc::SIGCHLD, libc::SIG_DFL) } == libc::SIG_ERR {
        log::error!("worker: SIGCHLD: {}", io::Error::last_os_error());
        return 1;
    }
    if let Err(e) = install_stop_handler() {
        log::error!("worker: signal handlers: {e}");
        return 1;
    }
    if let Err(e) = prepare_socket(&sock) {
        log::error!("worker: socket timeout: {e}");
        return 1;
    }
    match login(
        mode,
        &mut sock,
        &STOP,
        Pam::start,
        users::lookup,
        &mut Console,
    ) {
        Login::Done(status) => status,
        Login::Opened(session) => run_session(session, mode, &mut sock),
    }
}

/// How `login` ends: with the worker's exit status, or with a session open for `run_session`.
enum Login<P, H> {
    Done(i32),
    Opened(Opened<P, H>),
}

struct Opened<P, H> {
    pam: P,
    account: PasswdEntry,
    greeter_socket: bool,
    vt: u16,
    /// Held open for the session's life, so logind's autovt never starts a getty on it.
    tty: H,
    groups: Vec<libc::gid_t>,
}

/// Everything from `Begin` to `pam_open_session`, with libpam, passwd and the VTs passed in.
fn login<P: LoginPam, V: Vts>(
    mode: Mode,
    sock: &mut UnixStream,
    stop: &'static AtomicBool,
    start: impl FnOnce(&str, &str, Box<dyn Converse>) -> Result<P, PamError>,
    lookup: impl FnOnce(&str) -> io::Result<Option<PasswdEntry>>,
    vts: &mut V,
) -> Login<P, V::Held> {
    let stopped = || stop.load(Ordering::Relaxed);
    let (user, autologin_user, greeter_socket, rhost) = match read_until_stopped(sock, stop) {
        Ok(ToWorker::Begin {
            user,
            autologin_user,
            greeter_socket,
            rhost,
        }) => (user, autologin_user, greeter_socket, rhost),
        Ok(_) => {
            return Login::Done(fail(
                sock,
                ErrorKind::Failed,
                "the first frame is not Begin",
            ));
        }
        Err(_) => return Login::Done(1),
    };
    if !mode.runs_for(&user, autologin_user.as_deref()) {
        let text = format!("a {} worker does not run for {user}", mode.as_arg());
        return Login::Done(fail(sock, ErrorKind::Refused, &text));
    }
    let mut ended = None;
    let refused = Rc::new(Cell::new(false));
    let conv: Box<dyn Converse> = if mode.relays_prompts() {
        let Ok(clone) = sock.try_clone() else {
            log::error!("worker: cannot clone the socket");
            return Login::Done(1);
        };
        let c = SocketConverse::stoppable(clone, stop);
        ended = Some(c.ended());
        Box::new(c)
    } else {
        Box::new(Unattended {
            refused: refused.clone(),
        })
    };
    let cancelled = || stopped() || ended.as_ref().is_some_and(|e| e.get());

    let mut pam = match start(mode.service(), &user, conv) {
        Ok(pam) => pam,
        Err(e) => return Login::Done(fail(sock, ErrorKind::Failed, &e.to_string())),
    };
    let checked = check(&mut pam, rhost.as_deref());
    if cancelled() {
        return Login::Done(1);
    }
    // The entered name stays out of the log: it may be a password typed into the name field.
    if let Err((e, kind, text)) = checked {
        log::warn!("worker: login refused: {e}");
        return Login::Done(match refused.get() {
            true => fail(sock, ErrorKind::Refused, REFUSED_PROMPT),
            false => fail(sock, kind, &text),
        });
    }
    let account = match account_of(pam.user(), lookup) {
        Ok(account) => account,
        Err(text) => return Login::Done(fail(sock, ErrorKind::Failed, &text)),
    };
    let authenticated = FromWorker::Authenticated {
        user: account.name.clone(),
        uid: account.uid,
    };
    if write_msg(sock, &authenticated).is_err() {
        return Login::Done(1);
    }
    if !mode.opens_session() {
        return Login::Done(0);
    }
    let (vt, background) = match read_until_stopped(sock, stop) {
        Ok(ToWorker::Start { vt, background }) if (1..=vt::LAST_VT).contains(&vt) => {
            (vt, background)
        }
        Ok(ToWorker::Start { vt, .. }) => {
            let text = format!("VT {vt} is out of range");
            return Login::Done(fail(sock, ErrorKind::Failed, &text));
        }
        _ => return Login::Done(1),
    };
    match open(&mut pam, vts, mode, &account, vt, background, &stopped) {
        Ok((tty, groups)) => Login::Opened(Opened {
            pam,
            account,
            greeter_socket,
            vt,
            tty,
            groups,
        }),
        Err(_) if stopped() => Login::Done(1),
        Err(text) => Login::Done(fail(sock, ErrorKind::Failed, &text)),
    }
}

/// Bounds the worker's reads, so a stop is seen, and its writes, so a stuck daemon ends it.
fn prepare_socket(sock: &UnixStream) -> io::Result<()> {
    sock.set_read_timeout(Some(STOP_POLL))?;
    sock.set_write_timeout(Some(WRITE_WAIT))
}

/// `pam_authenticate` and the account step. A remote login's `PAM_RHOST` is set for both, so
/// the auth log names the address, and cleared after, so logind opens a local seat0 session.
fn check(
    pam: &mut impl LoginPam,
    rhost: Option<&str>,
) -> Result<(), (PamError, ErrorKind, String)> {
    let failed = |e: PamError| {
        let text = e.to_string();
        (e, ErrorKind::Failed, text)
    };
    if rhost.is_some() {
        pam.set_item_rhost(rhost).map_err(failed)?;
    }
    if let Some(refused) = refusal(pam.authenticate(), || pam.account()) {
        return Err(refused);
    }
    if rhost.is_some() {
        pam.set_item_rhost(None).map_err(failed)?;
    }
    Ok(())
}

/// Why the login stops before its session: a failed password gets the fixed text, and an account
/// refused after a correct one gets PAM's.
fn refusal(
    authenticated: Result<(), PamError>,
    account: impl FnOnce() -> Result<(), PamError>,
) -> Option<(PamError, ErrorKind, String)> {
    match authenticated {
        Err(e) => Some((e, ErrorKind::Auth, AUTH_FAILED.to_owned())),
        Ok(()) => account().err().map(|e| {
            let text = e.text.clone();
            (e, ErrorKind::Account, text)
        }),
    }
}

/// The account PAM let in, by the name PAM resolved; the error names only that name.
fn account_of(
    pam_user: Option<String>,
    lookup: impl FnOnce(&str) -> io::Result<Option<PasswdEntry>>,
) -> Result<PasswdEntry, String> {
    let Some(name) = pam_user else {
        return Err("PAM names no user".into());
    };
    match lookup(&name) {
        Ok(Some(account)) => Ok(account),
        Ok(None) => Err(format!("{name} has no passwd entry")),
        Err(e) => Err(format!("looking up {name}: {e}")),
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

/// Opens the session on `vt`: its tty and environment, the VT held and, unless `background`,
/// shown, then the credentials and `pam_open_session`.
fn open<V: Vts>(
    pam: &mut impl LoginPam,
    vts: &mut V,
    mode: Mode,
    account: &PasswdEntry,
    vt: u16,
    background: bool,
    stopped: &dyn Fn() -> bool,
) -> Result<(V::Held, Vec<libc::gid_t>), String> {
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
    let tty = vts.hold(vt).map_err(|e| format!("/dev/tty{vt}: {e}"))?;
    if !background {
        vts.activate_and_wait(vt, stopped)
            .map_err(|e| format!("VT {vt}: {e}"))?;
    }
    if stopped() {
        return Err("stopped".into());
    }
    let name = CString::new(account.name.as_str()).map_err(|_| "the user name holds a NUL byte")?;
    let groups = establish_creds(pam, &group_list(&name, account.gid)?)?;
    if let Err(e) = pam.open_session() {
        let _ = pam.delete_cred();
        return Err(e.to_string());
    }
    Ok((tty, groups))
}

/// Runs the opened session's program to the end and closes the session again. The status is 0
/// only when the program exited with 0.
fn run_session(session: Opened<Pam, File>, mode: Mode, sock: &mut UnixStream) -> i32 {
    let Opened {
        mut pam,
        account,
        greeter_socket,
        vt,
        tty,
        groups,
    } = session;
    let tty_path = format!("/dev/tty{vt}");
    let ran = run_program(
        &mut pam,
        mode,
        &account,
        greeter_socket,
        &tty_path,
        groups,
        sock,
    );
    let closed = pam.close_session();
    let deleted = Pam::delete_cred(&mut pam);
    drop(tty);
    for e in [closed.err(), deleted.err()].into_iter().flatten() {
        log::warn!("worker: closing {}'s session: {e}", account.name);
    }
    match ran {
        Ok(clean) => i32::from(!clean),
        Err(_) if stopped() => 1,
        Err(text) => fail(sock, ErrorKind::Failed, &text),
    }
}

/// Forks the session's program in the open session and waits for it.
fn run_program(
    pam: &mut Pam,
    mode: Mode,
    account: &PasswdEntry,
    greeter_socket: bool,
    tty_path: &str,
    groups: Vec<libc::gid_t>,
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
    let env = session_env(account, mode, pam_env);
    let argv = session_argv(account, mode, greeter_socket);
    let launch = Launch::new(account, mode, argv, tty_path, env, groups)?;
    let waited = block_session_signals();
    // Checked with the signals blocked: a later stop stays pending for `wait_for`.
    if stopped() {
        unblock(&waited);
        return Err("stopped".into());
    }
    // SAFETY: the child makes only async-signal-safe calls before `execve` or `_exit`, which
    // fork allows whatever other threads the worker has.
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
    unblock(&waited);
    Ok(status.is_some_and(|s| libc::WIFEXITED(s) && libc::WEXITSTATUS(s) == 0))
}

/// The calls that give the session its credentials, apart so their order is testable.
trait Creds {
    fn set_groups(&mut self, groups: &[libc::gid_t]) -> io::Result<()>;
    fn establish_cred(&mut self) -> Result<(), PamError>;
    fn groups(&self) -> io::Result<Vec<libc::gid_t>>;
    fn delete_cred(&mut self) -> Result<(), PamError>;
}

/// Puts the account's groups on the worker, runs `pam_setcred`, and returns the worker's
/// groups after it: the session's, with any a module such as pam_group added.
fn establish_creds(
    c: &mut impl Creds,
    account: &[libc::gid_t],
) -> Result<Vec<libc::gid_t>, String> {
    c.set_groups(account)
        .map_err(|e| format!("setgroups: {e}"))?;
    c.establish_cred().map_err(|e| e.to_string())?;
    c.groups().map_err(|e| {
        if let Err(d) = c.delete_cred() {
            log::warn!("worker: deleting the credentials: {d}");
        }
        format!("getgroups: {e}")
    })
}

impl Creds for Pam {
    fn set_groups(&mut self, groups: &[libc::gid_t]) -> io::Result<()> {
        // SAFETY: `groups` holds `groups.len()` entries.
        match unsafe { libc::setgroups(groups.len(), groups.as_ptr()) } {
            0 => Ok(()),
            _ => Err(io::Error::last_os_error()),
        }
    }

    fn establish_cred(&mut self) -> Result<(), PamError> {
        Pam::establish_cred(self)
    }

    fn delete_cred(&mut self) -> Result<(), PamError> {
        Pam::delete_cred(self)
    }

    fn groups(&self) -> io::Result<Vec<libc::gid_t>> {
        // SAFETY: a size query, then a fill of a buffer of that size.
        unsafe {
            let n = libc::getgroups(0, std::ptr::null_mut());
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            let mut groups = vec![0; n as usize];
            let n = libc::getgroups(n, groups.as_mut_ptr());
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            groups.truncate(n as usize);
            Ok(groups)
        }
    }
}

/// The PAM calls of a login, apart so their order is testable without libpam.
trait LoginPam: Creds {
    fn set_item_rhost(&mut self, rhost: Option<&str>) -> Result<(), PamError>;
    fn authenticate(&mut self) -> Result<(), PamError>;
    /// `pam_acct_mgmt`, changing an expired password through the conversation.
    fn account(&mut self) -> Result<(), PamError>;
    fn user(&self) -> Option<String>;
    fn set_tty(&mut self, tty: &str) -> Result<(), PamError>;
    fn putenv(&mut self, key: &str, value: &str) -> Result<(), PamError>;
    fn open_session(&mut self) -> Result<(), PamError>;
}

impl LoginPam for Pam {
    fn set_item_rhost(&mut self, rhost: Option<&str>) -> Result<(), PamError> {
        Pam::set_item_rhost(self, rhost)
    }

    fn authenticate(&mut self) -> Result<(), PamError> {
        Pam::authenticate(self)
    }

    fn account(&mut self) -> Result<(), PamError> {
        Pam::account(self)
    }

    fn user(&self) -> Option<String> {
        Pam::user(self)
    }

    fn set_tty(&mut self, tty: &str) -> Result<(), PamError> {
        Pam::set_tty(self, tty)
    }

    fn putenv(&mut self, key: &str, value: &str) -> Result<(), PamError> {
        Pam::putenv(self, key, value)
    }

    fn open_session(&mut self) -> Result<(), PamError> {
        Pam::open_session(self)
    }
}

/// The VT calls of a session start, apart so a background start is testable.
trait Vts {
    type Held;
    fn hold(&mut self, vt: u16) -> io::Result<Self::Held>;
    fn activate_and_wait(&mut self, vt: u16, stop: &dyn Fn() -> bool) -> io::Result<()>;
}

/// The kernel's VTs, through `vt`.
struct Console;

impl Vts for Console {
    type Held = File;

    fn hold(&mut self, vt: u16) -> io::Result<File> {
        vt::hold(vt)
    }

    fn activate_and_wait(&mut self, vt: u16, stop: &dyn Fn() -> bool) -> io::Result<()> {
        vt::activate_and_wait(vt, stop)
    }
}

/// The signals the worker unwinds on through `pam_end`, and passes on to a running session.
const STOP_SIGNALS: [c_int; 3] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP];
/// The longest a write to the daemon may block.
const WRITE_WAIT: Duration = Duration::from_secs(5);

/// Set by a stop signal.
static STOP: AtomicBool = AtomicBool::new(false);

fn stopped() -> bool {
    STOP.load(Ordering::Relaxed)
}

extern "C" fn on_stop(_: c_int) {
    STOP.store(true, Ordering::Relaxed);
}

/// Points the stop signals at `on_stop`. Without `SA_RESTART`, the blocking call a stop
/// lands in returns `EINTR`. A handler, unlike a blocked mask, does not reach PAM's helpers.
fn install_stop_handler() -> io::Result<()> {
    // SAFETY: `on_stop` only stores to an atomic, which is async-signal-safe.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_stop as extern "C" fn(c_int) as libc::sighandler_t;
        libc::sigemptyset(&mut action.sa_mask);
        for sig in STOP_SIGNALS {
            if libc::sigaction(sig, &action, std::ptr::null_mut()) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

/// One frame from `sock`, or an error once `stop` is set. A stop that lands just before a
/// read waits out the socket's read timeout.
fn read_until_stopped<T: DeserializeOwned>(
    sock: &UnixStream,
    stop: &AtomicBool,
) -> Result<T, FrameError> {
    read_msg(&mut Stoppable { sock, stop })
}

struct Stoppable<'a> {
    sock: &'a UnixStream,
    stop: &'a AtomicBool,
}

impl Read for Stoppable<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return Err(io::Error::other("the worker was stopped"));
            }
            match self.sock.read(buf) {
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) => {}
                r => return r,
            }
        }
    }
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

/// Waits for `child`, passing each stop signal on as SIGTERM. `None` when it can't be reaped.
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
        if STOP_SIGNALS.contains(&sig) {
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
    journal: libc::sockaddr_un,
    journal_header: Vec<u8>,
}

impl Launch {
    fn new(
        account: &PasswdEntry,
        mode: Mode,
        argv: Vec<String>,
        tty_path: &str,
        env: Vec<(String, String)>,
        groups: Vec<libc::gid_t>,
    ) -> Result<Launch, String> {
        let nul = |what: &str| format!("{what} holds a NUL byte");
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
        Ok(Launch {
            path,
            argv_ptrs: pointers(&argv),
            _argv: argv,
            envp_ptrs: pointers(&envp),
            _envp: envp,
            groups,
            uid: account.uid,
            gid: account.gid,
            home: CString::new(account.home.as_os_str().as_bytes()).map_err(|_| nul("home"))?,
            tty: CString::new(tty_path).map_err(|_| nul("tty"))?,
            no_new_privs: mode == Mode::Greeter,
            journal: journal_address(),
            journal_header: format!("{JOURNAL_IDENTIFIER}\n\n6\n1\n0\n0\n0\n").into_bytes(),
        })
    }

    /// Becomes the session's program. A failure before its output is set up goes to the worker's
    /// log, a later one to the session's; either exits 1, or 127 when `execve` fails.
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
            libc::dup2(LOG_FD, 2);

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
            if libc::syscall(libc::SYS_close_range, 3, u32::MAX, 0) < 0 {
                fail(b"reprise-login: cannot close the worker's files\n");
            }
            if libc::setgroups(self.groups.len(), self.groups.as_ptr()) < 0
                || libc::setgid(self.gid) < 0
                || libc::setuid(self.uid) < 0
            {
                fail(b"reprise-login: cannot drop to the user's ids\n");
            }
            if libc::getuid() != self.uid
                || libc::geteuid() != self.uid
                || libc::getgid() != self.gid
                || libc::getegid() != self.gid
            {
                fail(b"reprise-login: the ids did not change\n");
            }
            // Connected as the user, so journald files the output under the user's uid.
            let out = match self.connect_journal() {
                Some(fd) => fd,
                None => {
                    let msg =
                        b"reprise-login: no journal stream; the session's output is dropped\n";
                    libc::write(2, msg.as_ptr().cast(), msg.len());
                    libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY)
                }
            };
            if out < 0 || libc::dup2(out, 1) < 0 || libc::dup2(out, 2) < 0 {
                fail(b"reprise-login: cannot set up the session's output\n");
            }
            if out > 2 {
                libc::close(out);
            }
            // dup2 onto itself keeps FD_CLOEXEC, so clear it on all three.
            for fd in 0..3 {
                if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                    fail(b"reprise-login: cannot keep the session's stdio open\n");
                }
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

    /// A journal stdout stream with the header written, or `None` when journald takes none.
    ///
    /// # Safety
    /// Called only in the child; it makes only async-signal-safe calls.
    unsafe fn connect_journal(&self) -> Option<c_int> {
        // SAFETY: socket, connect, write, shutdown and close are async-signal-safe, and
        // `journal` and `journal_header` were made before the fork.
        unsafe {
            let fd = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0);
            if fd < 0 {
                return None;
            }
            let addr = (&self.journal as *const libc::sockaddr_un).cast::<libc::sockaddr>();
            let len = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
            let mut ok = libc::connect(fd, addr, len) == 0;
            let mut sent = 0;
            while ok && sent < self.journal_header.len() {
                let rest = &self.journal_header[sent..];
                let n = libc::write(fd, rest.as_ptr().cast(), rest.len());
                ok = n > 0;
                sent += n.max(0) as usize;
            }
            if ok && libc::shutdown(fd, libc::SHUT_RD) == 0 {
                return Some(fd);
            }
            libc::close(fd);
            None
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

/// journald's stdout stream socket, built before fork for `Launch::connect_journal`.
fn journal_address() -> libc::sockaddr_un {
    // SAFETY: `sockaddr_un` is a plain C struct; all zeroes is a valid value.
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (dst, src) in addr.sun_path.iter_mut().zip(JOURNAL_STDOUT.as_bytes()) {
        *dst = *src as c_char;
    }
    addr
}

#[cfg(test)]
mod tests {
    use super::*;
    use repriselogin::frame::FrameError;
    use std::sync::atomic::{AtomicBool, Ordering};
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
        let services: Vec<_> = MODES.iter().map(|m| m.service()).collect();
        assert_eq!(
            services,
            [
                "reprise-login",
                "reprise-login",
                "reprise-login-greeter",
                "reprise-login-autologin",
                "reprise-login-remote",
                "reprise-login-remote-nla",
                "reprise-login-check"
            ]
        );
        let args: Vec<_> = MODES.iter().map(|m| m.as_arg()).collect();
        assert_eq!(
            args[4..],
            ["remote", "remote-nla", "check"],
            "the remote and check modes' arguments"
        );
        for m in MODES {
            assert_eq!(Mode::parse(m.as_arg()), Some(m));
        }
        assert_eq!(Mode::parse("root"), None);
    }

    #[test]
    fn a_greeter_worker_runs_only_for_the_greeter_account() {
        assert!(Mode::Greeter.runs_for(GREETER_USER, None));
        assert!(!Mode::Greeter.runs_for("anna", None));
        assert!(!Mode::Greeter.runs_for("root", Some("root")));
        for m in [
            Mode::Login,
            Mode::Reauth,
            Mode::Remote,
            Mode::RemoteNla,
            Mode::Check,
        ] {
            assert!(m.runs_for("anna", None));
        }
    }

    /// The index of the first `auth` line of `stack` running `module`.
    fn auth_line(stack: &str, module: &str) -> Option<usize> {
        (stack.lines()).position(|l| l.starts_with("auth") && l.contains(module))
    }

    #[test]
    fn the_passwordless_stacks_check_the_account_before_pam_permit() {
        let greeter = include_str!("../../../../data/pam.d/reprise-login-greeter");
        let guard = auth_line(greeter, "pam_succeed_if.so user = reprise-greeter");
        assert!(guard.is_some(), "{greeter}");
        assert!(guard < auth_line(greeter, "pam_permit.so"));
        let autologin = include_str!("../../../../data/pam.d/reprise-login-autologin");
        let guard = auth_line(autologin, "pam_succeed_if.so uid >= 1000");
        assert!(guard.is_some(), "{autologin}");
        assert!(guard < auth_line(autologin, "pam_permit.so"));
    }

    #[test]
    fn the_login_stack_refuses_root_before_any_password_is_checked() {
        let login = include_str!("../../../../data/pam.d/reprise-login");
        let delay = auth_line(login, "pam_faildelay.so delay=2000000");
        let guard = auth_line(login, "pam_succeed_if.so quiet_success uid ne 0");
        let include = auth_line(login, "system-login");
        assert!(delay.is_some() && delay < guard, "{login}");
        assert!(guard.is_some() && guard < include, "{login}");
        let guard = login.lines().nth(guard.unwrap()).unwrap();
        assert!(
            guard.contains("[success=ok user_unknown=ignore default=die]"),
            "an unknown user still reaches the password prompt: {guard}"
        );
    }

    #[test]
    fn an_autologin_worker_runs_only_for_the_configured_user() {
        assert!(Mode::Autologin.runs_for("anna", Some("anna")));
        assert!(!Mode::Autologin.runs_for("root", Some("anna")));
        assert!(!Mode::Autologin.runs_for("anna", None));
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
        let mut conv = SocketConverse::stoppable(worker, &NEVER);
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
        let mut conv = SocketConverse::stoppable(worker, &NEVER);
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
        let mut conv = SocketConverse::stoppable(worker, &NEVER);
        assert_eq!(conv.prompt(AuthKind::Visible, "login:"), None);
        assert!(conv.ended().get());
    }

    #[test]
    fn worker_messages_round_trip() {
        use repriselogin::frame::{decode, encode};
        let to = [
            ToWorker::Begin {
                user: "anna".into(),
                autologin_user: Some("anna".into()),
                greeter_socket: true,
                rhost: Some("192.0.2.1".into()),
            },
            ToWorker::Answer(Some(Zeroizing::new("pw".into()))),
            ToWorker::Answer(None),
            ToWorker::Start {
                vt: 7,
                background: true,
            },
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
            FromWorker::Authenticated {
                user: "anna".into(),
                uid: 1000,
            },
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
    fn an_account_refusal_carries_pams_text_and_a_failed_password_the_fixed_one() {
        let err = |code, text: &str| PamError {
            code,
            text: text.into(),
        };
        let expired = || Err(err(13, "User account has expired"));
        let (_, kind, text) = refusal(Ok(()), expired).expect("refused");
        assert_eq!(
            (kind, text.as_str()),
            (ErrorKind::Account, "User account has expired")
        );
        let (_, kind, text) =
            refusal(Err(err(7, "Authentication failure")), || unreachable!()).expect("refused");
        assert_eq!((kind, text.as_str()), (ErrorKind::Auth, AUTH_FAILED));
        assert!(refusal(Ok(()), || Ok(())).is_none());
    }

    #[test]
    fn a_missing_account_is_named_by_pams_user_only() {
        let err = account_of(Some("anna".into()), |_| Ok(None)).unwrap_err();
        assert_eq!(err, "anna has no passwd entry");
        assert_eq!(
            account_of(None, |_| unreachable!()).unwrap_err(),
            "PAM names no user"
        );
    }

    #[test]
    fn authenticated_names_the_account_pam_resolved() {
        let m = FromWorker::Authenticated {
            user: "anna".into(),
            uid: 1000,
        };
        let frame = repriselogin::frame::encode(&m).unwrap();
        assert_eq!(
            std::str::from_utf8(&frame[4..]).unwrap(),
            r#"{"type":"authenticated","user":"anna","uid":1000}"#
        );
    }

    #[test]
    fn debug_of_an_answer_never_shows_it() {
        let m = ToWorker::Answer(Some(Zeroizing::new("hunter2".into())));
        assert_eq!(format!("{m:?}"), "Answer(<redacted>)");
        let begin = ToWorker::Begin {
            user: "hunter2".into(),
            autologin_user: None,
            greeter_socket: false,
            rhost: Some("192.0.2.1".into()),
        };
        assert_eq!(format!("{begin:?}"), "Begin { <redacted> }");
        assert_eq!(format!("{:?}", ToWorker::Answer(None)), "Answer(None)");
        assert_eq!(
            format!(
                "{:?}",
                ToWorker::Start {
                    vt: 7,
                    background: true
                }
            ),
            "Start { vt: 7, background: true }"
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
        assert_eq!(session_argv(&anna(), Mode::Login, false), argv);
        assert_eq!(session_argv(&anna(), Mode::Autologin, true), argv);
        let shell_less = PasswdEntry {
            shell: String::new(),
            ..anna()
        };
        assert_eq!(session_argv(&shell_less, Mode::Login, false)[0], "/bin/sh");
    }

    #[test]
    fn the_greeter_argv_is_reprise_greeter_with_a_socket_only_when_asked() {
        assert_eq!(
            session_argv(&anna(), Mode::Greeter, false),
            ["reprise", "--greeter"]
        );
        assert_eq!(
            session_argv(&anna(), Mode::Greeter, true),
            ["reprise", "--greeter", "--greeter-socket"]
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

    static NEVER: AtomicBool = AtomicBool::new(false);

    #[test]
    fn a_stop_while_prompting_ends_the_conversation() {
        static STOPPED: AtomicBool = AtomicBool::new(false);
        let (worker, mut daemon) = UnixStream::pair().unwrap();
        worker
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let mut conv = SocketConverse::stoppable(worker, &STOPPED);
        let daemon = std::thread::spawn(move || {
            let prompt = read_msg::<FromWorker>(&mut daemon).unwrap();
            STOPPED.store(true, Ordering::Relaxed);
            (prompt, daemon)
        });
        assert_eq!(conv.prompt(AuthKind::Secret, "Password:"), None);
        assert!(conv.ended().get());
        let (prompt, _daemon) = daemon.join().unwrap();
        assert!(matches!(prompt, FromWorker::Prompt { .. }), "{prompt:?}");
    }

    #[test]
    fn sigterm_ends_a_blocked_socket_read() {
        use std::os::unix::thread::JoinHandleExt;
        install_stop_handler().unwrap();
        let (worker, daemon) = UnixStream::pair().unwrap();
        worker
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let reader = std::thread::spawn(move || read_until_stopped::<ToWorker>(&worker, &STOP));
        std::thread::sleep(Duration::from_millis(50));
        // SAFETY: the thread is alive until joined below; SIGTERM has the stop handler.
        unsafe { libc::pthread_kill(reader.as_pthread_t(), libc::SIGTERM) };
        let r = reader.join().unwrap();
        assert!(matches!(r, Err(FrameError::Io(_))), "{r:?}");
        assert!(STOP.load(Ordering::Relaxed));
        drop(daemon);
    }

    /// The worker's groups as `setgroups` leaves them; `pam_setcred` adds `added`.
    #[derive(Default)]
    struct FakeCreds {
        groups: Vec<libc::gid_t>,
        added: libc::gid_t,
        groups_fail: bool,
        deleted: bool,
    }

    impl Creds for FakeCreds {
        fn set_groups(&mut self, groups: &[libc::gid_t]) -> std::io::Result<()> {
            self.groups = groups.to_vec();
            Ok(())
        }

        fn establish_cred(&mut self) -> Result<(), PamError> {
            self.groups.push(self.added);
            Ok(())
        }

        fn groups(&self) -> std::io::Result<Vec<libc::gid_t>> {
            match self.groups_fail {
                true => Err(std::io::ErrorKind::OutOfMemory.into()),
                false => Ok(self.groups.clone()),
            }
        }

        fn delete_cred(&mut self) -> Result<(), PamError> {
            self.deleted = true;
            Ok(())
        }
    }

    #[test]
    fn groups_added_by_setcred_reach_the_session() {
        let mut creds = FakeCreds {
            groups: vec![0],
            added: 990,
            ..FakeCreds::default()
        };
        assert_eq!(
            establish_creds(&mut creds, &[1000, 998]),
            Ok(vec![1000, 998, 990])
        );
        assert!(!creds.deleted);
    }

    #[test]
    fn a_getgroups_failure_after_setcred_deletes_the_credentials() {
        let mut creds = FakeCreds {
            groups_fail: true,
            ..FakeCreds::default()
        };
        assert!(establish_creds(&mut creds, &[1000]).is_err());
        assert!(creds.deleted, "pam_setcred's credentials are deleted again");
    }

    #[test]
    fn sighup_stops_the_worker_like_sigterm() {
        install_stop_handler().unwrap();
        for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            // SAFETY: a query of the disposition into a zeroed local.
            let handler = unsafe {
                let mut old: libc::sigaction = std::mem::zeroed();
                assert_eq!(libc::sigaction(sig, std::ptr::null(), &mut old), 0);
                old.sa_sigaction
            };
            assert_eq!(
                handler, on_stop as extern "C" fn(c_int) as libc::sighandler_t,
                "signal {sig}"
            );
        }
        assert!(STOP_SIGNALS.contains(&libc::SIGHUP));
    }

    #[test]
    fn the_worker_socket_times_out_reads_and_writes() {
        let (worker, _daemon) = UnixStream::pair().unwrap();
        prepare_socket(&worker).unwrap();
        assert_eq!(worker.read_timeout().unwrap(), Some(STOP_POLL));
        assert_eq!(worker.write_timeout().unwrap(), Some(WRITE_WAIT));
    }

    #[test]
    fn the_remote_stacks_refuse_root_and_skip_faillock() {
        let remote = include_str!("../../../../data/pam.d/reprise-login-remote");
        let root = auth_line(remote, "pam_succeed_if.so quiet_success uid ne 0");
        let shells = auth_line(remote, "pam_shells.so");
        let unix = auth_line(remote, "pam_unix.so");
        assert!(root.is_some() && root < shells && shells < unix, "{remote}");
        let unix = remote.lines().nth(unix.unwrap()).unwrap();
        assert!(unix.ends_with("pam_unix.so nodelay"), "{unix}");
        let faillock = (remote.lines()).any(|l| !l.starts_with('#') && l.contains("pam_faillock"));
        assert!(!faillock, "{remote}");

        let nla = include_str!("../../../../data/pam.d/reprise-login-remote-nla");
        let uid = auth_line(nla, "pam_succeed_if.so uid >= 1000");
        let shells = auth_line(nla, "pam_shells.so");
        assert!(uid.is_some() && uid < shells, "{nla}");
        assert!(shells < auth_line(nla, "pam_permit.so"), "{nla}");

        let check = include_str!("../../../../data/pam.d/reprise-login-check");
        let stack: Vec<Vec<&str>> = (check.lines())
            .filter(|l| !l.starts_with('#'))
            .map(|l| l.split_whitespace().collect())
            .collect();
        assert_eq!(
            stack,
            [
                ["auth", "include", "system-auth"],
                ["account", "include", "system-auth"]
            ]
        );
    }

    type Calls = Rc<std::cell::RefCell<Vec<String>>>;

    /// A PAM transaction that records its calls; `authenticate` asks `prompt` first if set.
    struct FakePam {
        calls: Calls,
        conv: Box<dyn Converse>,
        prompt: Option<(AuthKind, &'static str)>,
        env: Vec<(String, String)>,
        groups: Vec<libc::gid_t>,
    }

    impl FakePam {
        fn call(&self, call: impl Into<String>) -> Result<(), PamError> {
            self.calls.borrow_mut().push(call.into());
            Ok(())
        }
    }

    impl Creds for FakePam {
        fn set_groups(&mut self, groups: &[libc::gid_t]) -> std::io::Result<()> {
            self.groups = groups.to_vec();
            Ok(())
        }

        fn establish_cred(&mut self) -> Result<(), PamError> {
            self.call("setcred")
        }

        fn groups(&self) -> std::io::Result<Vec<libc::gid_t>> {
            Ok(self.groups.clone())
        }

        fn delete_cred(&mut self) -> Result<(), PamError> {
            self.call("delete_cred")
        }
    }

    impl LoginPam for FakePam {
        fn set_item_rhost(&mut self, rhost: Option<&str>) -> Result<(), PamError> {
            self.call(format!("set_item_rhost({rhost:?})"))
        }

        fn authenticate(&mut self) -> Result<(), PamError> {
            self.call("authenticate")?;
            if let Some((kind, text)) = self.prompt
                && self.conv.prompt(kind, text).is_none()
            {
                return Err(PamError {
                    code: crate::pam::ffi::PAM_CONV_ERR,
                    text: "Conversation error".into(),
                });
            }
            Ok(())
        }

        fn account(&mut self) -> Result<(), PamError> {
            self.call("acct_mgmt")
        }

        fn user(&self) -> Option<String> {
            Some("anna".into())
        }

        fn set_tty(&mut self, tty: &str) -> Result<(), PamError> {
            self.env.push(("PAM_TTY".into(), tty.into()));
            Ok(())
        }

        fn putenv(&mut self, key: &str, value: &str) -> Result<(), PamError> {
            self.env.push((key.into(), value.into()));
            Ok(())
        }

        fn open_session(&mut self) -> Result<(), PamError> {
            self.call("open_session")
        }
    }

    #[derive(Default)]
    struct FakeVts(Vec<String>);

    impl Vts for FakeVts {
        type Held = ();

        fn hold(&mut self, vt: u16) -> std::io::Result<()> {
            self.0.push(format!("hold({vt})"));
            Ok(())
        }

        fn activate_and_wait(&mut self, vt: u16, _: &dyn Fn() -> bool) -> std::io::Result<()> {
            self.0.push(format!("activate_and_wait({vt})"));
            Ok(())
        }
    }

    /// What a `login` against the fakes did.
    struct Ran {
        /// The exit status, or `None` when a session was opened.
        status: Option<i32>,
        pam: Vec<String>,
        vts: Vec<String>,
        env: Vec<(String, String)>,
        sent: Vec<FromWorker>,
    }

    /// Runs `login` in `mode` against the fakes, with `queued` waiting for the worker and the
    /// daemon's side closed after them.
    fn run_login(
        mode: Mode,
        queued: Vec<ToWorker>,
        prompt: Option<(AuthKind, &'static str)>,
    ) -> Ran {
        let (mut worker, mut daemon) = UnixStream::pair().unwrap();
        for m in &queued {
            write_msg(&mut daemon, m).unwrap();
        }
        daemon.shutdown(std::net::Shutdown::Write).unwrap();
        let calls = Calls::default();
        let start = |_: &str, _: &str, conv: Box<dyn Converse>| {
            Ok(FakePam {
                calls: calls.clone(),
                conv,
                prompt,
                env: Vec::new(),
                groups: Vec::new(),
            })
        };
        let mut vts = FakeVts::default();
        let lookup = |_: &str| Ok(Some(anna()));
        let (status, env) = match login(mode, &mut worker, &NEVER, start, lookup, &mut vts) {
            Login::Done(status) => (Some(status), Vec::new()),
            Login::Opened(opened) => (None, opened.pam.env),
        };
        drop(worker);
        let mut sent = Vec::new();
        while let Ok(m) = read_msg::<FromWorker>(&mut daemon) {
            sent.push(m);
        }
        let pam = calls.borrow().clone();
        Ran {
            status,
            pam,
            vts: vts.0,
            env,
            sent,
        }
    }

    fn begin(rhost: Option<&str>) -> ToWorker {
        ToWorker::Begin {
            user: "anna".into(),
            autologin_user: None,
            greeter_socket: false,
            rhost: rhost.map(str::to_owned),
        }
    }

    fn answer(pw: &str) -> ToWorker {
        ToWorker::Answer(Some(Zeroizing::new(pw.into())))
    }

    const PASSWORD: Option<(AuthKind, &str)> = Some((AuthKind::Secret, "Password:"));

    #[test]
    fn rhost_is_set_for_authentication_and_cleared_before_the_session_opens() {
        let start = ToWorker::Start {
            vt: 7,
            background: true,
        };
        let queued = vec![begin(Some("192.0.2.7")), answer("pw"), start];
        let ran = run_login(Mode::Remote, queued, PASSWORD);
        assert_eq!(ran.status, None, "the session opened: {:?}", ran.sent);
        assert_eq!(
            ran.pam,
            [
                r#"set_item_rhost(Some("192.0.2.7"))"#,
                "authenticate",
                "acct_mgmt",
                "set_item_rhost(None)",
                "setcred",
                "open_session"
            ]
        );
        assert_eq!(
            ran.sent,
            [
                FromWorker::Prompt {
                    kind: AuthKind::Secret,
                    text: "Password:".into()
                },
                FromWorker::Authenticated {
                    user: "anna".into(),
                    uid: 1000
                }
            ],
            "a remote login relays its prompt as a console one does"
        );
    }

    #[test]
    fn a_background_start_never_activates_the_vt() {
        let start = |background| ToWorker::Start { vt: 7, background };
        let queued = vec![begin(Some("192.0.2.7")), answer("pw"), start(true)];
        let ran = run_login(Mode::Remote, queued, PASSWORD);
        assert_eq!(ran.status, None);
        assert_eq!(ran.vts, ["hold(7)"]);
        let console = run_login(
            Mode::Login,
            vec![begin(None), answer("pw"), start(false)],
            PASSWORD,
        );
        assert_eq!(console.vts, ["hold(7)", "activate_and_wait(7)"]);
        assert_eq!(
            ran.env, console.env,
            "a background session's tty and XDG variables"
        );
        for want in [
            ("PAM_TTY", "tty7"),
            ("XDG_VTNR", "7"),
            ("XDG_SESSION_CLASS", "user"),
            ("XDG_SESSION_TYPE", "wayland"),
        ] {
            assert!(
                ran.env.contains(&(want.0.into(), want.1.into())),
                "{want:?}"
            );
        }
    }

    #[test]
    fn a_remote_nla_worker_relays_no_prompt() {
        let ran = run_login(Mode::RemoteNla, vec![begin(Some("192.0.2.7"))], PASSWORD);
        assert_eq!(ran.status, Some(1));
        assert_eq!(
            ran.sent,
            [FromWorker::Failed {
                kind: ErrorKind::Refused,
                text: REFUSED_PROMPT.into()
            }]
        );
        assert_eq!(
            ran.pam,
            [r#"set_item_rhost(Some("192.0.2.7"))"#, "authenticate"]
        );
    }

    #[test]
    fn a_check_worker_ends_after_authenticated_and_opens_nothing() {
        let start = ToWorker::Start {
            vt: 7,
            background: false,
        };
        let ran = run_login(
            Mode::Check,
            vec![begin(None), answer("pw"), start],
            PASSWORD,
        );
        assert_eq!(ran.status, Some(0));
        assert_eq!(ran.pam, ["authenticate", "acct_mgmt"]);
        assert!(ran.vts.is_empty(), "{:?}", ran.vts);
        assert_eq!(
            ran.sent,
            [
                FromWorker::Prompt {
                    kind: AuthKind::Secret,
                    text: "Password:".into()
                },
                FromWorker::Authenticated {
                    user: "anna".into(),
                    uid: 1000
                }
            ]
        );
    }
}
