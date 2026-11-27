//! The Services page's rows: each service of the user manager and, while they show, the system
//! manager's, with its status and how it starts. They are read on a thread of their own
//! ([`spawn`]); a job ([`run`]) and a log read ([`read_log`]) each get a thread, since a system
//! job may wait minutes on a password prompt.

use std::collections::{HashMap, HashSet};
use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use reprisebus::services::{Failed, Loaded, Manager, Reread, UnitFile, Verb, is_service_name};
use reprisekit::tr;

/// The lines asked of the journal for a service's log.
pub const LOG_LINES: u32 = 200;
/// The most of journalctl's output a log shows.
const LOG_BYTES: u64 = 1 << 20;
/// How long journalctl may take before it is killed.
const LOG_DEADLINE: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    /// Active with nothing left running: a one-shot service that did its work.
    Finished,
    Starting,
    Stopping,
    Stopped,
    Failed,
}

impl Status {
    /// The status of a unit in systemd's `active` and `sub` states.
    pub fn of(active: &str, sub: &str) -> Status {
        match active {
            "active" | "reloading" | "refreshing" if sub == "exited" => Status::Finished,
            "active" | "reloading" | "refreshing" => Status::Running,
            "activating" => Status::Starting,
            "deactivating" => Status::Stopping,
            "failed" => Status::Failed,
            _ => Status::Stopped,
        }
    }

    pub fn text(self) -> String {
        match self {
            Status::Running => tr!("svc-running"),
            Status::Finished => tr!("svc-finished"),
            Status::Starting => tr!("svc-starting"),
            Status::Stopping => tr!("svc-stopping"),
            Status::Stopped => tr!("svc-stopped"),
            Status::Failed => tr!("svc-failed"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Startup {
    /// Enabled: a user service starts at login, a system one at boot.
    Automatic,
    /// Disabled: it has an install section and starts only when asked.
    Manual,
    /// Masked: it cannot start.
    Disabled,
    /// No install section, or a generated or transient unit: it starts when another unit or a
    /// socket asks for it.
    OnDemand,
}

impl Startup {
    /// How a unit file in systemd's unit file `state` starts.
    pub fn of(state: &str) -> Startup {
        match state {
            "enabled" => Startup::Automatic,
            "disabled" => Startup::Manual,
            "masked" | "masked-runtime" => Startup::Disabled,
            _ => Startup::OnDemand,
        }
    }

    pub fn text(self) -> String {
        match self {
            Startup::Automatic => tr!("startup-automatic"),
            Startup::Manual => tr!("startup-manual"),
            Startup::Disabled => tr!("startup-disabled"),
            Startup::OnDemand => tr!("startup-on-demand"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Service {
    pub name: String,
    pub description: String,
    pub manager: Manager,
    pub status: Status,
    pub startup: Startup,
}

impl Service {
    pub fn key(&self) -> (Manager, &str) {
        (self.manager, &self.name)
    }

    /// Whether its state lets `verb` run: Start when stopped and not masked, Stop while it runs or
    /// starts, Restart while it runs, Start at Login on a user service whose file installs.
    pub fn takes(&self, verb: Verb) -> bool {
        use Status::*;
        let installs = self.manager == Manager::User;
        match verb {
            Verb::Start => {
                matches!(self.status, Stopped | Failed) && self.startup != Startup::Disabled
            }
            Verb::Stop => matches!(self.status, Running | Finished | Starting),
            Verb::Restart => matches!(self.status, Running | Finished),
            Verb::Enable => installs && self.startup == Startup::Manual,
            Verb::Disable => installs && self.startup == Startup::Automatic,
        }
    }

    pub fn shown_name(&self) -> String {
        shown_name(&self.name)
    }

    pub fn group(&self) -> String {
        match self.manager {
            Manager::User => tr!("group-user"),
            Manager::System => tr!("group-system"),
        }
    }
}

/// Service `name` as the page shows it: unescaped and without `.service`, an instance as
/// `getty@tty1`.
pub fn shown_name(name: &str) -> String {
    reprisebus::services::unescape(name.strip_suffix(".service").unwrap_or(name))
}

/// Whether `s` shows under the Services page's filter: `text`, case aside, in its shown name or
/// its description, and failed when `failed_only`.
pub fn shows(s: &Service, text: &str, failed_only: bool) -> bool {
    if failed_only && s.status != Status::Failed {
        return false;
    }
    let text = text.trim().to_lowercase();
    text.is_empty()
        || s.shown_name().to_lowercase().contains(&text)
        || s.description.to_lowercase().contains(&text)
}

/// Manager `m`'s rows by name: its loaded services and, as stopped, each file not loaded, which
/// `describe` reads. Templates, aliases and units whose file is missing are left out.
pub fn rows(
    m: Manager,
    loaded: &[Loaded],
    files: &[UnitFile],
    mut describe: impl FnMut(&str) -> String,
) -> Vec<Service> {
    let file_name = |f: &UnitFile| Path::new(&f.path).file_name()?.to_str().map(String::from);
    let states: HashMap<String, &str> = (files.iter())
        .filter_map(|f| Some((file_name(f)?, f.state.as_str())))
        .collect();
    let mut seen = HashSet::new();
    let mut out: Vec<Service> = (loaded.iter())
        .filter(|u| matches!(u.load.as_str(), "loaded" | "masked") && is_service_name(&u.name))
        .filter(|u| seen.insert(u.name.clone()))
        .map(|u| Service {
            name: u.name.clone(),
            description: u.description.clone(),
            manager: m,
            status: Status::of(&u.active, &u.sub),
            startup: Startup::of(states.get(&u.name).copied().unwrap_or_default()),
        })
        .collect();
    for f in files.iter().filter(|f| f.state != "alias") {
        let Some(name) = file_name(f) else { continue };
        if name.ends_with("@.service") || !is_service_name(&name) || !seen.insert(name.clone()) {
            continue;
        }
        out.push(Service {
            description: describe(&f.path),
            name,
            manager: m,
            status: Status::Stopped,
            startup: Startup::of(&f.state),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The `Description=` of a unit file's `[Unit]` section, the last one set.
pub fn description(unit_file: &str) -> Option<String> {
    let mut section = "";
    let mut found = None;
    for line in unit_file.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line;
        } else if section == "[Unit]"
            && let Some(v) = (line.strip_prefix("Description"))
                .and_then(|rest| rest.trim_start().strip_prefix('='))
        {
            found = Some(v.trim().to_string());
        }
    }
    found.filter(|d| !d.is_empty())
}

/// The description in the unit file at `path`, or nothing for a file that does not read: a
/// masked unit's link to `/dev/null`, or anything but a plain file. Opened without blocking, so a
/// FIFO in its place never holds the reader.
fn describe_file(path: &str) -> String {
    use std::os::unix::fs::OpenOptionsExt as _;
    let read = || {
        let file = (std::fs::OpenOptions::new().read(true))
            .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
            .open(path)
            .ok()?;
        if !file.metadata().ok()?.is_file() {
            return None;
        }
        let mut text = String::new();
        file.take(64 * 1024).read_to_string(&mut text).ok()?;
        description(&text)
    };
    read().unwrap_or_default()
}

/// A job on one service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub manager: Manager,
    pub name: String,
    pub verb: Verb,
}

pub enum Msg {
    /// The rows, read with the system manager's when `system`, and whether polkit may let this
    /// user manage system services.
    Listed {
        rows: Vec<Service>,
        system: bool,
        manage_system: bool,
    },
    Done(Job, Result<(), Failed>),
    Log {
        manager: Manager,
        name: String,
        read: Result<String, LogError>,
    },
    /// The user manager signalled a change to its services ([`follow`]).
    Reread(Reread),
}

/// Where the threads send what they got: the event loop's channel, which they wake.
#[derive(Clone)]
pub struct Post {
    pub tx: Sender<Msg>,
    pub wake: repriseapp::Waker,
}

impl Post {
    /// Sends `m` and wakes the loop; false once the receiver is gone.
    pub fn send(&self, m: Msg) -> bool {
        let sent = self.tx.send(m).is_ok();
        if sent {
            self.wake.wake();
        }
        sent
    }
}

/// Asks [`spawn`]'s thread for the rows: the system manager's too when `system`, its unit files
/// read again when `files` (they are read once otherwise, as that walks the unit folders).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Read {
    pub system: bool,
    pub files: bool,
}

/// Starts the thread that reads the rows on each [`Read`], posting [`Msg::Listed`]. Reads asked
/// while one runs are taken together.
pub fn spawn(post: Post) -> Sender<Read> {
    let (tx, rx) = channel::<Read>();
    std::thread::spawn(move || reader(&rx, &post));
    tx
}

fn reader(rx: &Receiver<Read>, post: &Post) {
    let mut files: HashMap<Manager, Vec<UnitFile>> = HashMap::new();
    let mut described: HashMap<String, String> = HashMap::new();
    let mut manage_system = true;
    while let Ok(first) = rx.recv() {
        let ask = rx.try_iter().fold(first, |a, b| Read {
            system: b.system,
            files: a.files || b.files,
        });
        let managers = [Manager::User]
            .into_iter()
            .chain(ask.system.then_some(Manager::System));
        if ask.files {
            described.clear();
        }
        let mut rows = Vec::new();
        for m in managers {
            let mut describe = |p: &str| {
                (described.entry(p.to_string()))
                    .or_insert_with(|| describe_file(p))
                    .clone()
            };
            match read(m, ask.files, &mut files, &mut describe) {
                Ok(r) => rows.extend(r),
                Err(e) => log::info!("{m:?} services: {e}"),
            }
        }
        if ask.system && ask.files {
            manage_system = reprisebus::services::may_manage_system();
        }
        post.send(Msg::Listed {
            rows,
            system: ask.system,
            manage_system,
        });
    }
}

fn read(
    m: Manager,
    fresh: bool,
    files: &mut HashMap<Manager, Vec<UnitFile>>,
    describe: &mut impl FnMut(&str) -> String,
) -> Result<Vec<Service>, String> {
    use reprisebus::services as bus;
    let conn = bus::connection(m).map_err(|e| e.to_string())?;
    let loaded = reprisebus::wait(bus::loaded(&conn)).map_err(|e| e.to_string())?;
    if fresh || !files.contains_key(&m) {
        let read = reprisebus::wait(bus::files(&conn)).map_err(|e| e.to_string())?;
        files.insert(m, read);
    }
    Ok(rows(m, &loaded, &files[&m], describe))
}

/// Follows the user manager's signals on a thread of its own, posting a [`Msg::Reread`] for each
/// change to its services, until the window is gone.
pub fn follow(post: Post) {
    std::thread::spawn(move || {
        reprisebus::services::follow_rereads(|r| post.send(Msg::Reread(r)));
    });
}

/// Runs `job` off the event loop, posting [`Msg::Done`].
pub fn run(job: Job, post: Post) {
    std::thread::spawn(move || {
        let r = reprisebus::services::carry_out(job.manager, &job.name, job.verb);
        post.send(Msg::Done(job, r));
    });
}

/// Reads service `name`'s log off the event loop, posting [`Msg::Log`].
pub fn read_log(manager: Manager, name: String, post: Post) {
    std::thread::spawn(move || {
        let read = journal(manager, &name);
        post.send(Msg::Log {
            manager,
            name,
            read,
        });
    });
}

/// `journalctl`'s arguments for the last [`LOG_LINES`] entries of service `name` as
/// [`reprisefs::journal::lines`] reads them: the one unit, as one `--unit=` or `--user-unit=`
/// argument.
pub fn journal_args(m: Manager, name: &str) -> Vec<String> {
    let unit = match m {
        Manager::User => format!("--user-unit={name}"),
        Manager::System => format!("--unit={name}"),
    };
    let args = [unit, format!("--lines={LOG_LINES}")].into_iter();
    (args.chain(["--no-pager", "--quiet"].map(String::from)))
        .chain(reprisefs::journal::JSON_ARGS.map(String::from))
        .collect()
}

/// Why a service's log did not read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogError {
    NotAService,
    /// journalctl did not start.
    NotStarted,
    /// journalctl ended with this code and wrote this on stderr.
    Exited(i32, String),
    /// A signal ended journalctl; it wrote this on stderr.
    Killed(String),
    /// journalctl ran past [`LOG_DEADLINE`] and was killed.
    TookTooLong,
    /// Its end could not be waited for; it was killed.
    Unwatched,
}

/// The entries of service `name` the journals this user may read hold, as JSON lines.
fn journal(m: Manager, name: &str) -> Result<String, LogError> {
    if !is_service_name(name) {
        return Err(LogError::NotAService);
    }
    let mut cmd = Command::new("journalctl");
    cmd.args(journal_args(m, name));
    let out = capped(cmd, LOG_BYTES, LOG_DEADLINE)?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Runs `cmd` for the last `cap` bytes of its stdout, from the start of a line; past `deadline`
/// it is killed.
fn capped(mut cmd: Command, cap: u64, deadline: Duration) -> Result<Vec<u8>, LogError> {
    let mut child = (cmd.stdin(Stdio::null()))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| LogError::NotStarted)?;
    let out = child.stdout.take();
    let out = std::thread::spawn(move || out.map_or_else(Vec::new, |p| tail(p, cap as usize)));
    let err = child.stderr.take();
    let err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = err {
            let _ = p.take(16 * 1024).read_to_end(&mut buf);
        }
        buf
    });
    let end = Instant::now() + deadline;
    let status = loop {
        let late = match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < end => {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            Ok(None) => LogError::TookTooLong,
            Err(_) => LogError::Unwatched,
        };
        let _ = child.kill();
        let _ = child.wait();
        break Err(late);
    };
    let (out, err) = (
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default(),
    );
    let status = status?;
    let err = String::from_utf8_lossy(&err).trim().to_string();
    match status.code() {
        _ if status.success() => Ok(out),
        Some(code) => Err(LogError::Exited(code, err)),
        None => Err(LogError::Killed(err)),
    }
}

/// All of `pipe` but the last `cap` bytes, which are kept from the first line that starts in
/// them.
fn tail(mut pipe: impl std::io::Read, cap: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    let mut cut = false;
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
        // One byte before the window stays, to tell whether a line starts right at it.
        if buf.len() > 2 * cap + 1 {
            buf.drain(..buf.len() - cap - 1);
            cut = true;
        }
    }
    if buf.len() > cap {
        buf.drain(..buf.len() - cap - 1);
        cut = true;
    }
    if cut {
        let line = buf
            .iter()
            .position(|&b| b == b'\n')
            .map_or(buf.len(), |i| i + 1);
        buf.drain(..line);
    }
    buf
}

#[cfg(test)]
mod tests;
