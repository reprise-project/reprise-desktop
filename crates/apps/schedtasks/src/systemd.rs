//! The systemd user manager on the session bus. [`watch`] starts two threads: one runs the
//! window's commands and reads the timers after each, the other follows the manager's signals and
//! asks for a read when a shown timer or its unit changes, a unit file is enabled or disabled, or
//! the manager reloads. The window never waits on the bus.

use crate::model::{self, Task};
use crate::units::{self, NewTask};
use futures_util::StreamExt;
use repriseapp::Poster;
use reprisebus::mutex::lock;
use reprisebus::{Props, Signal, prop};
use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zbus::OwnedObjectPath;
use zbus::zvariant::DynamicType;
use zbus::{Connection, MatchRule, Message};

pub const DEST: &str = "org.freedesktop.systemd1";
pub const PATH: &str = "/org/freedesktop/systemd1";
const MANAGER: &str = "org.freedesktop.systemd1.Manager";
const UNIT: &str = "org.freedesktop.systemd1.Unit";
const TIMER: &str = "org.freedesktop.systemd1.Timer";
const SERVICE: &str = "org.freedesktop.systemd1.Service";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// Every unit's object lies under this path.
const UNITS: &str = "/org/freedesktop/systemd1/unit";

/// How long a read waits after a change, so one read takes in the burst a start or stop sends.
const SETTLE: Duration = Duration::from_millis(150);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    Refresh,
    /// Start the unit a timer runs, `backup.service`.
    Run(String),
    /// Stop that unit.
    Stop(String),
    /// Read that unit's lines from the journal ([`crate::output::journal`]).
    Output(String),
    /// Write `task`'s unit files ([`units::write`]), reload, and enable and start its timer.
    /// `taken` are the timers shown, whose names the new one keeps clear of.
    Create {
        task: NewTask,
        taken: Vec<String>,
    },
    /// Disable and stop a timer New Task wrote and its service, remove both files and reload.
    Delete(String),
    /// Enable and start a timer (`on`), or disable and stop it. `file_state` is its
    /// `UnitFileState`: only an enabled or disabled file is changed, a static or transient timer
    /// is only started or stopped.
    Turn {
        timer: String,
        file_state: String,
        on: bool,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum Update {
    /// The timers by name, or why the manager could not list them.
    Tasks(Result<Vec<Task>, String>),
    /// The manager refused a command; its message.
    Refused(Cmd, String),
    /// A unit's journal lines, or why they could not be read.
    Output(String, Result<String, String>),
    /// New Task made this timer.
    Created(String),
}

/// The window's end of the threads.
pub struct Bus {
    pub cmds: Sender<Cmd>,
    pub updates: Receiver<Update>,
}

/// Starts the threads and the first read.
pub fn watch(waker: repriseapp::Waker) -> Bus {
    let (cmds, rx) = channel();
    let (post, updates) = Poster::channel(waker);
    let shown = Arc::new(Mutex::new(HashSet::new()));
    let tx = cmds.clone();
    let paths = Arc::clone(&shown);
    spawn("reprise-schedtasks-bus", move || serve(rx, post, paths));
    spawn("reprise-schedtasks-changes", move || {
        follow_changes(tx, shown)
    });
    let _ = cmds.send(Cmd::Refresh);
    Bus { cmds, updates }
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(f) {
        log::warn!("could not start the {name} thread: {e}");
    }
}

/// Runs commands as they come, the queued ones together, then reads the timers.
fn serve(rx: Receiver<Cmd>, post: Poster<Update>, shown: Arc<Mutex<HashSet<String>>>) {
    while let Ok(first) = rx.recv() {
        if first == Cmd::Refresh {
            std::thread::sleep(SETTLE);
        }
        let batch: Vec<Cmd> = std::iter::once(first).chain(rx.try_iter()).collect();
        let (outputs, batch): (Vec<Cmd>, Vec<Cmd>) =
            batch.into_iter().partition(|c| matches!(c, Cmd::Output(_)));
        for cmd in outputs {
            if let Cmd::Output(unit) = cmd {
                let post = post.clone();
                spawn("reprise-schedtasks-journal", move || {
                    let read = crate::output::journal(&unit);
                    post.post(Update::Output(unit, read));
                });
            }
        }
        if batch.is_empty() {
            continue;
        }
        let conn = match reprisebus::session() {
            Ok(c) => c,
            Err(e) => {
                log::info!("no session bus: {e}");
                post.post(Update::Tasks(Err(told(&e))));
                continue;
            }
        };
        let folders = reprisebus::wait(unit_dir(&conn));
        let dir = folders.as_ref().map(|f| f.own.as_path());
        for cmd in batch.iter().filter(|c| **c != Cmd::Refresh) {
            let done = match cmd {
                Cmd::Create { task, taken } => {
                    reprisebus::wait(create(&conn, folders.as_ref(), task, taken))
                        .map(|timer| post.post(Update::Created(timer)))
                }
                Cmd::Delete(timer) => reprisebus::wait(delete(&conn, dir, timer)),
                cmd => reprisebus::wait(act(&conn, cmd)),
            };
            if let Err(e) = done {
                log::warn!("{cmd:?}: {e}");
                post.post(Update::Refused(cmd.clone(), told(&e)));
            }
        }
        match reprisebus::wait(read(&conn, clocks(), dir)) {
            Ok((tasks, paths)) => {
                *lock(&shown) = paths;
                post.post(Update::Tasks(Ok(tasks)));
            }
            Err(e) => {
                log::info!("systemd user timers: {e}");
                post.post(Update::Tasks(Err(told(&e))));
            }
        }
    }
}

/// An error as the user reads it: the manager's message, or the error itself.
fn told(e: &zbus::Error) -> String {
    reprisebus::error::message(e).map_or_else(|| e.to_string(), String::from)
}

/// The wall clock and `CLOCK_MONOTONIC` in µs.
fn clocks() -> (u64, u64) {
    (clock(libc::CLOCK_REALTIME), clock(libc::CLOCK_MONOTONIC))
}

fn clock(id: libc::clockid_t) -> u64 {
    // SAFETY: `timespec` is plain data, and `clock_gettime` writes only `ts`.
    let ts = unsafe {
        let mut ts: libc::timespec = std::mem::zeroed();
        libc::clock_gettime(id, &mut ts);
        ts
    };
    ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1_000
}

async fn call<B>(conn: &Connection, method: &str, body: &B) -> zbus::Result<Message>
where
    B: serde::Serialize + DynamicType,
{
    conn.call_method(Some(DEST), PATH, Some(MANAGER), method, body)
        .await
}

async fn get_all(conn: &Connection, path: &OwnedObjectPath, iface: &str) -> zbus::Result<Props> {
    conn.call_method(Some(DEST), path, Some(PROPERTIES), "GetAll", &(iface,))
        .await?
        .body()
        .deserialize()
}

async fn load(conn: &Connection, name: &str) -> zbus::Result<OwnedObjectPath> {
    call(conn, "LoadUnit", &(name,)).await?.body().deserialize()
}

/// The folders of the manager's own search path, which can differ from this program's
/// `$XDG_CONFIG_HOME`.
async fn unit_dir(conn: &Connection) -> Option<units::Folders> {
    let reply = conn
        .call_method(
            Some(DEST),
            PATH,
            Some(PROPERTIES),
            "Get",
            &(MANAGER, "UnitPath"),
        )
        .await
        .inspect_err(|e| log::info!("systemd UnitPath: {e}"))
        .ok()?;
    let search: zbus::OwnedVariant = reply.body().deserialize().ok()?;
    units::Folders::of(&Vec::<String>::try_from(search).ok()?)
}

/// A `ListUnitsByPatterns` row: name, description, load, active and sub state, followed unit,
/// object path, job id, type and path.
type Listed = (
    String,
    String,
    String,
    String,
    String,
    String,
    OwnedObjectPath,
    u32,
    String,
    OwnedObjectPath,
);

/// The timers the manager has loaded and those with a unit file, templates left out.
async fn timer_names(conn: &Connection) -> zbus::Result<BTreeSet<String>> {
    let (states, patterns): (&[&str], &[&str]) = (&[], &["*.timer"]);
    let loaded: Vec<Listed> = call(conn, "ListUnitsByPatterns", &(states, patterns))
        .await?
        .body()
        .deserialize()?;
    let files: Vec<(String, String)> = call(conn, "ListUnitFilesByPatterns", &(states, patterns))
        .await?
        .body()
        .deserialize()?;
    let mut names: BTreeSet<String> = loaded.into_iter().map(|u| u.0).collect();
    names.extend(
        files
            .into_iter()
            .filter_map(|(path, _)| Some(Path::new(&path).file_name()?.to_str()?.to_string())),
    );
    names.retain(|n| !n.contains("@."));
    Ok(names)
}

/// Every loaded timer, sorted by name, and the object paths of the timers and their units.
/// `now` is [`clocks`], `dir` the own folder of [`unit_dir`]. A timer that does not read is
/// left out.
pub async fn read(
    conn: &Connection,
    now: (u64, u64),
    dir: Option<&Path>,
) -> zbus::Result<(Vec<Task>, HashSet<String>)> {
    let mut tasks = Vec::new();
    let mut paths = HashSet::new();
    for name in timer_names(conn).await? {
        match task(conn, &name, now, dir).await {
            Ok(Some((t, timer, service))) => {
                tasks.push(t);
                paths.insert(timer.to_string());
                paths.insert(service.to_string());
            }
            Ok(None) => {}
            Err(e) => log::info!("{name}: {e}"),
        }
    }
    tasks.sort_by_cached_key(|t| (t.name().to_lowercase(), t.timer.clone()));
    Ok((tasks, paths))
}

/// Timer `name` and the paths of it and its unit; None when its file is missing or masked.
async fn task(
    conn: &Connection,
    name: &str,
    now: (u64, u64),
    dir: Option<&Path>,
) -> zbus::Result<Option<(Task, OwnedObjectPath, OwnedObjectPath)>> {
    let text = |p: &Props, k: &str| prop::<String>(p, k).unwrap_or_default();
    let path = load(conn, name).await?;
    let unit = get_all(conn, &path, UNIT).await?;
    if text(&unit, "LoadState") != "loaded" {
        return Ok(None);
    }
    let timer = get_all(conn, &path, TIMER).await?;
    let service = text(&timer, "Unit");
    let service_path = load(conn, &service).await?;
    let started = get_all(conn, &service_path, UNIT).await?;
    let loaded = text(&started, "LoadState") == "loaded";
    let exec = match loaded {
        true => get_all(conn, &service_path, SERVICE)
            .await
            .unwrap_or_default(),
        false => Props::default(),
    };
    let time = |k: &str| prop::<u64>(&timer, k).unwrap_or(0);
    let calendar = prop::<Vec<(String, String, u64)>>(&timer, "TimersCalendar").unwrap_or_default();
    let monotonic = prop::<Vec<(String, u64, u64)>>(&timer, "TimersMonotonic").unwrap_or_default();
    let file = text(&unit, "FragmentPath");
    let t = Task {
        ours: dir.is_some_and(|d| units::ours(d, name, &file)),
        timer: name.to_string(),
        description: if loaded {
            text(&started, "Description")
        } else {
            String::new()
        },
        service_state: if loaded {
            text(&started, "ActiveState")
        } else {
            String::new()
        },
        service,
        timer_state: text(&unit, "ActiveState"),
        file_state: text(&unit, "UnitFileState"),
        calendar: calendar.into_iter().map(|(b, spec, _)| (b, spec)).collect(),
        monotonic: monotonic.into_iter().map(|(b, us, _)| (b, us)).collect(),
        next: model::next_elapse(
            time("NextElapseUSecRealtime"),
            time("NextElapseUSecMonotonic"),
            now,
        ),
        last: model::last_run([
            time("LastTriggerUSec"),
            prop(&exec, "ExecMainStartTimestamp").unwrap_or(0),
            prop(&started, "InactiveExitTimestamp").unwrap_or(0),
        ]),
        result: text(&exec, "Result"),
        exit_code: prop(&exec, "ExecMainCode").unwrap_or(0),
        exit_status: prop(&exec, "ExecMainStatus").unwrap_or(0),
        command: command(&exec),
        file,
    };
    Ok(Some((t, path, service_path)))
}

/// One `ExecStart` entry: path, argv, ignore failure, start and exit times, pid, code, status.
type Exec = (String, Vec<String>, bool, u64, u64, u64, u64, u32, i32, i32);

/// The service's `ExecStart` command lines, an argument holding a space in quotes.
fn command(exec: &Props) -> String {
    let lines = prop::<Vec<Exec>>(exec, "ExecStart").unwrap_or_default();
    let quote = |a: &String| match a.contains(char::is_whitespace) || a.is_empty() {
        true => format!("\"{a}\""),
        false => a.clone(),
    };
    lines
        .iter()
        .map(|e| e.1.iter().map(quote).collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Runs `cmd` as `systemctl --user start`, `stop`, `enable --now` or `disable --now` would.
pub async fn act(conn: &Connection, cmd: &Cmd) -> zbus::Result<()> {
    match cmd {
        Cmd::Refresh | Cmd::Output(_) | Cmd::Create { .. } | Cmd::Delete(_) => {}
        Cmd::Run(service) => {
            call(conn, "StartUnit", &(service, "replace")).await?;
        }
        Cmd::Stop(service) => {
            call(conn, "StopUnit", &(service, "replace")).await?;
        }
        Cmd::Turn {
            timer,
            file_state,
            on,
        } => {
            let units = [timer.as_str()];
            if *on && file_state == "disabled" {
                call(conn, "EnableUnitFiles", &(&units[..], false, false)).await?;
                call(conn, "Reload", &()).await?;
            } else if !on && file_state.starts_with("enabled") {
                let runtime = file_state == "enabled-runtime";
                call(conn, "DisableUnitFiles", &(&units[..], runtime)).await?;
                call(conn, "Reload", &()).await?;
            }
            let job = if *on { "StartUnit" } else { "StopUnit" };
            call(conn, job, &(timer, "replace")).await?;
        }
    }
    Ok(())
}

/// Writes `task` in the own folder of `folders` ([`unit_dir`]), enables it, reloads and starts
/// its timer, as `systemctl --user enable` and `daemon-reload` and `start` would; returns the
/// timer. A step that fails after the files are written takes them away again.
async fn create(
    conn: &Connection,
    folders: Option<&units::Folders>,
    task: &NewTask,
    taken: &[String],
) -> zbus::Result<String> {
    let folders = folders.ok_or_else(no_folder)?;
    let dir = &folders.own;
    let timer = units::write(folders, task, taken).map_err(|e| failed(&e.to_string()))?;
    let units = [timer.as_str()];
    let started = async {
        call(conn, "EnableUnitFiles", &(&units[..], false, false)).await?;
        call(conn, "Reload", &()).await?;
        call(conn, "StartUnit", &(&timer, "replace")).await
    };
    let Err(e) = started.await else {
        return Ok(timer);
    };
    let _ = call(conn, "DisableUnitFiles", &(&units[..], false)).await;
    let removed = units::remove(dir, &timer);
    let _ = call(conn, "Reload", &()).await;
    match removed {
        Ok(()) => Err(e),
        Err(left) => Err(failed(&format!(
            "{}; {timer} and its service stay in {}: {left}",
            told(&e),
            dir.display()
        ))),
    }
}

/// Disables and stops timer `timer` and stops its service, removes their files from `dir` and
/// reloads, when the pair is ours ([`units::ours`]) by the manager's current `FragmentPath`;
/// anything else is refused untouched. The service goes first, so a Delete that stops between
/// the two can be done again.
async fn delete(conn: &Connection, dir: Option<&Path>, timer: &str) -> zbus::Result<()> {
    let dir = dir.ok_or_else(no_folder)?;
    let not_ours = || failed("Scheduled Tasks did not make this task");
    let stem = timer
        .strip_suffix(".timer")
        .filter(|s| units::is_our_stem(s));
    let stem = stem.ok_or_else(not_ours)?;
    let service = format!("{stem}.service");
    let fragment = get_all(conn, &load(conn, timer).await?, UNIT).await?;
    let fragment = prop::<String>(&fragment, "FragmentPath").unwrap_or_default();
    if !units::ours(dir, timer, &fragment) {
        return Err(not_ours());
    }
    call(conn, "DisableUnitFiles", &(&[timer][..], false)).await?;
    call(conn, "StopUnit", &(timer, "replace")).await?;
    call(conn, "StopUnit", &(&service, "replace")).await?;
    units::remove(dir, timer).map_err(|e| failed(&e.to_string()))?;
    call(conn, "Reload", &()).await?;
    // A failed service stays loaded, with no file, until its failure is reset.
    let _ = call(conn, "ResetFailedUnit", &(&service,)).await;
    Ok(())
}

fn no_folder() -> zbus::Error {
    failed("the user manager lists no folder for your own unit files")
}

/// An error of this program's own, read as `why` by [`told`].
fn failed(why: &str) -> zbus::Error {
    zbus::Error::FDO(Box::new(zbus::fdo::Error::Failed(why.into())))
}

/// Asks `tx` for a read on every change the manager signals, connecting again after the bus or
/// the manager goes; ends once the window is gone.
fn follow_changes(tx: Sender<Cmd>, shown: Arc<Mutex<HashSet<String>>>) {
    let mut pause = Duration::ZERO;
    loop {
        let start = Instant::now();
        let gone = match reprisebus::session() {
            Ok(conn) => reprisebus::wait(changes(&conn, &tx, &shown)),
            Err(e) => {
                log::info!("no session bus: {e}");
                false
            }
        };
        if gone {
            return;
        }
        pause = reprisebus::reconnect_pause(pause, start.elapsed());
        std::thread::sleep(pause);
        if tx.send(Cmd::Refresh).is_err() {
            return;
        }
    }
}

fn rules() -> Vec<zbus::OwnedMatchRule> {
    let signal = || MatchRule::builder().msg_type(zbus::message::Type::Signal);
    let manager = |member| signal().path(PATH).interface(MANAGER).member(member);
    let built = [
        signal()
            .path_namespace(UNITS)
            .interface(PROPERTIES)
            .member("PropertiesChanged")
            .build(),
        manager("UnitFilesChanged").build(),
        manager("Reloading").build(),
    ];
    built
        .into_iter()
        .filter_map(|r| r.inspect_err(|e| log::warn!("no match rule: {e}")).ok())
        .map(Into::into)
        .collect()
}

/// Sends a [`Cmd::Refresh`] for each change [`relevant`] lets through until the manager leaves
/// or the stream ends; true once the receiver is gone.
async fn changes(conn: &Connection, tx: &Sender<Cmd>, shown: &Mutex<HashSet<String>>) -> bool {
    let mut signals = reprisebus::signals(conn, DEST, rules()).await;
    // The manager sends unit signals only while some client is subscribed.
    if let Err(e) = call(conn, "Subscribe", &()).await {
        log::info!("systemd Subscribe: {e}");
    }
    if tx.send(Cmd::Refresh).is_err() {
        return true;
    }
    while let Some(signal) = signals.next().await {
        let refresh = match &signal {
            Signal::OwnerChanged => return false,
            Signal::Message(m) => relevant(m, &lock(shown)),
        };
        if refresh && tx.send(Cmd::Refresh).is_err() {
            return true;
        }
    }
    false
}

/// Whether a signal changes what the window shows: any timer's properties, those of a unit a
/// shown timer runs, a unit file enabled or disabled, or the end of a reload.
fn relevant(m: &Message, shown: &HashSet<String>) -> bool {
    let hdr = m.header();
    let path = hdr.path().map(|p| p.as_str()).unwrap_or_default();
    match hdr.member().map(|m| m.as_str()) {
        Some("PropertiesChanged") => {
            let iface = m.body().deserialize::<(String, Props, Vec<String>)>();
            iface.is_ok_and(|(i, ..)| i == TIMER) || shown.contains(path)
        }
        Some("Reloading") => m.body().deserialize::<bool>().is_ok_and(|active| !active),
        Some("UnitFilesChanged") => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests;
