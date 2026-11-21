//! The "Remote client" sink: one PipeWire null sink made the default, with the playback streams
//! sent to other sinks moved into it, while any viewer listens; its monitor captured once per
//! viewer at that viewer's rate.

use pipewire::properties::properties;
use pipewire::proxy::ProxyT;
use repriseaudio::{Snapshot, Stream};
use reprisevideo::monitor;
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The sink's `node.name`, which the default-sink metadata names.
const NAME: &str = "reprise-remote-client";
const DESCRIPTION: &str = "Remote client";
/// The capture streams' application name.
const APP: &str = "Remote Desktop";
/// How long one default switch may take before the sink goes on regardless.
const GIVE_BACK: Duration = Duration::from_secs(2);
/// How often a give-back after PipeWire took the sink away is tried again, within [`GIVE_BACK`].
const RETRY: Duration = Duration::from_millis(250);
/// How long a sink's thread may take to end: a switch back, plus the PipeWire teardown. A new
/// sink waits as long for the last one's turn, then takes it regardless.
const JOIN: Duration = Duration::from_secs(3);

/// Taken for a shared sink's whole life, so the sink made after the last viewer left sees the
/// default the last one gave back rather than the last one's sink.
static ONE: Turn = Turn::new();

/// The sink every viewer hears through, while one listens.
static SHARED: Mutex<Weak<Shared>> = Mutex::new(Weak::new());

/// The `T` in `slot` while it is held and `alive`, or a new one from `make` that `slot` then
/// holds weakly.
fn acquire<T>(
    slot: &Mutex<Weak<T>>,
    alive: impl FnOnce(&T) -> bool,
    make: impl FnOnce() -> io::Result<T>,
) -> io::Result<Arc<T>> {
    let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(live) = slot.upgrade().filter(|t| alive(t)) {
        return Ok(live);
    }
    let made = Arc::new(make()?);
    *slot = Arc::downgrade(&made);
    Ok(made)
}

/// Waits up to [`JOIN`] for the shared sink's thread to give the default back: a sink dropped on
/// the runtime ends on a thread of its own, which a process about to exit would cut short.
pub fn settle() {
    drop(ONE.take(JOIN));
}

/// One holder at a time, with a wait that gives up on a holder that never lets go.
struct Turn {
    /// The holder's number and the next one to hand out.
    state: Mutex<(Option<u64>, u64)>,
    freed: Condvar,
}

impl Turn {
    const fn new() -> Turn {
        Turn {
            state: Mutex::new((None, 0)),
            freed: Condvar::new(),
        }
    }

    fn state(&self) -> MutexGuard<'_, (Option<u64>, u64)> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Wait up to `patience` for the holder to let go, then take the turn from it.
    fn take(&self, patience: Duration) -> Held<'_> {
        let until = Instant::now() + patience;
        let mut state = self.state();
        while state.0.is_some() {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                log::warn!("sound: the last sink did not go within {patience:?}");
                break;
            }
            state = (self.freed.wait_timeout(state, left))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        let number = state.1;
        *state = (Some(number), number + 1);
        Held { turn: self, number }
    }
}

/// A turn on [`Turn`]; dropping it lets go unless it was taken over meanwhile.
struct Held<'a> {
    turn: &'a Turn,
    number: u64,
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        let mut state = self.turn.state();
        if state.0 == Some(self.number) {
            state.0 = None;
            self.turn.freed.notify_all();
        }
    }
}

/// One viewer's capture of the shared sink. Dropping the last one gives the default back and
/// removes the sink.
pub struct Sink {
    /// Wakes a capture still waiting for the sink to come up.
    cancel: mpsc::Sender<bool>,
    _capture: Worker,
    _shared: Arc<Shared>,
}

impl Sink {
    /// Make the sink the default and pass what plays into it to `samples`: S16LE stereo at
    /// `rate`, in the chunks PipeWire hands over.
    pub fn open(rate: u32, samples: impl FnMut(&[u8]) + Send + 'static) -> io::Result<Sink> {
        let shared = acquire(&SHARED, Shared::alive, Shared::open)?;
        let (cancel, ready) = mpsc::channel();
        shared.when_ready(cancel.clone());
        let capture = Worker::spawn("reprise-rdp-sound", move |stopped| {
            if ready.recv() != Ok(true) {
                return;
            }
            let mut samples = samples;
            let samples = Box::new(move |pcm: &[u8], _| samples(pcm));
            let tap = monitor::Tap::Monitor(Some(NAME));
            if let Err(e) = monitor::capture(tap, APP, DESCRIPTION, rate, samples, stopped) {
                log::warn!("sound: {e}");
            }
        })?;
        Ok(Sink {
            cancel,
            _capture: capture,
            _shared: shared,
        })
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        let _ = self.cancel.send(false);
    }
}

/// Keeps the shared sink the default and every playback stream in it with nothing captured from
/// it, so what plays in the session goes nowhere rather than to this computer's speakers.
pub struct Silence {
    _shared: Arc<Shared>,
}

impl Silence {
    pub fn hold() -> io::Result<Silence> {
        let shared = acquire(&SHARED, Shared::alive, Shared::open)?;
        Ok(Silence { _shared: shared })
    }
}

/// The null sink and the default switch to it, on a thread whose PipeWire connection owns the
/// sink. Dropping it gives the default back and removes the sink.
struct Shared {
    ready: Arc<Mutex<Ready>>,
    /// Locked never: the mutex makes it `Sync` for [`SHARED`], as its end channel is not.
    _thread: Mutex<Worker>,
}

/// Whether the shared sink is up, or the captures waiting to hear.
enum Ready {
    Waiting(Vec<mpsc::Sender<bool>>),
    Done(bool),
}

impl Shared {
    fn open() -> io::Result<Shared> {
        let ready = Arc::new(Mutex::new(Ready::Waiting(Vec::new())));
        let thread = Worker::spawn("reprise-rdp-sink", {
            let ready = ready.clone();
            move |stopped| {
                let _one = ONE.take(JOIN);
                if let Err(e) = run(stopped, &ready) {
                    log::warn!("sound: {e}");
                }
                report(&ready, false);
            }
        })?;
        Ok(Shared {
            ready,
            _thread: Mutex::new(thread),
        })
    }

    /// Whether the thread is still up or coming up: one that ended on its own stays dead.
    fn alive(&self) -> bool {
        let ready = self.ready.lock().unwrap_or_else(PoisonError::into_inner);
        !matches!(*ready, Ready::Done(false))
    }

    /// Send `up` whether the sink is up, once it is or has failed.
    fn when_ready(&self, up: mpsc::Sender<bool>) {
        match &mut *self.ready.lock().unwrap_or_else(PoisonError::into_inner) {
            Ready::Waiting(waiting) => waiting.push(up),
            Ready::Done(done) => {
                let _ = up.send(*done);
            }
        }
    }
}

fn report(ready: &Mutex<Ready>, up: bool) {
    let mut ready = ready.lock().unwrap_or_else(PoisonError::into_inner);
    if let Ready::Waiting(waiting) = &*ready {
        for w in waiting {
            let _ = w.send(up);
        }
    }
    *ready = Ready::Done(up);
}

/// A thread with a PipeWire loop; dropping it stops the loop and waits for the thread.
struct Worker {
    stop: pipewire::channel::Sender<()>,
    /// The thread, and a channel that disconnects once it has dropped everything else, a turn
    /// on [`ONE`] included.
    thread: Option<(JoinHandle<()>, mpsc::Receiver<()>)>,
}

impl Worker {
    fn spawn(
        name: &str,
        body: impl FnOnce(pipewire::channel::Receiver<()>) + Send + 'static,
    ) -> io::Result<Worker> {
        let (stop, stopped) = pipewire::channel::channel();
        let (ending, ended) = mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let _ending = ending;
                body(stopped);
            })?;
        Ok(Worker {
            stop,
            thread: Some((thread, ended)),
        })
    }
}

/// On a runtime thread the wait for the thread moves to a thread of its own; at process
/// shutdown, outside the runtime, dropping waits so the default is back before the exit.
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        let Some((thread, ended)) = self.thread.take() else {
            return;
        };
        let wait = move || match ended.recv_timeout(JOIN) {
            Err(RecvTimeoutError::Disconnected) => {
                let _ = thread.join();
            }
            _ => log::warn!(
                "sound: {} did not end within {JOIN:?}",
                thread.thread().name().unwrap_or_default()
            ),
        };
        if tokio::runtime::Handle::try_current().is_err() {
            wait();
        } else if let Err(e) = std::thread::Builder::new()
            .name("reprise-rdp-sound-end".into())
            .spawn(wait)
        {
            log::warn!("sound: {e}");
        }
    }
}

impl std::fmt::Debug for Sink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sink")
    }
}

/// The default ours takes over, to give back as it goes: the one the user configured, or
/// while none is (or a crashed server's sink still is) the one in effect. Never our own sink.
fn taken_over(now: &Snapshot) -> Option<String> {
    (now.configured_sink.clone())
        .filter(|c| c != NAME)
        .or_else(|| now.default_sink.clone())
        .filter(|d| d != NAME)
}

/// Whether a sink of ours other than node `id` is up: the next connection's, which took over
/// while this one was slow to go and now owns the default and the record.
fn superseded(now: &Snapshot, id: u32) -> bool {
    now.sinks.iter().any(|d| d.name == NAME && d.id != id)
}

/// The sink to make the default again as ours goes: `before`, while the default is still ours
/// and `before` is still there. A default the user picked meanwhile stays.
fn give_back(now: &Snapshot, before: Option<&str>) -> Option<u32> {
    if now.default_sink.as_deref() != Some(NAME) {
        return None;
    }
    let before = before.filter(|b| *b != NAME)?;
    now.sinks.iter().find(|d| d.name == before).map(|d| d.id)
}

/// The sink to make the default again for a record a killed server left: `recorded`, while the
/// configured default still names our sink or nothing.
fn left_behind(now: &Snapshot, recorded: &str) -> Option<u32> {
    if now.configured_sink.as_deref().is_some_and(|c| c != NAME) {
        return None;
    }
    let recorded = Some(recorded).filter(|r| *r != NAME)?;
    now.sinks.iter().find(|d| d.name == recorded).map(|d| d.id)
}

/// What a sink changed and gives back as it goes, kept in [`record_path`] meanwhile so a server
/// killed meanwhile can be made good. A sink named here is a `node.name`; `None` is the default.
#[derive(Clone, Debug, Default, PartialEq)]
struct Record {
    /// The default taken over.
    before: Option<String>,
    /// Our sink's `object.serial`, which each moved stream's `target.object` names.
    sink: Option<u64>,
    /// Where each moved stream still playing played before, by its `object.serial`.
    streams: BTreeMap<u64, Option<String>>,
    /// Where each program's moved streams played before, by [`repriseaudio::App::key`], for its
    /// streams that started since.
    apps: BTreeMap<String, Option<String>>,
}

impl Record {
    /// `before` and `sink` on a line each, then a tab-separated line per stream and program; an
    /// absent value is an empty field.
    fn to_text(&self) -> String {
        let field = |v: &Option<String>| v.clone().unwrap_or_default();
        let mut text = format!(
            "{}\n{}\n",
            field(&self.before),
            self.sink.map(|s| s.to_string()).unwrap_or_default()
        );
        for (serial, sink) in &self.streams {
            text += &format!("stream\t{serial}\t{}\n", field(sink));
        }
        for (key, sink) in &self.apps {
            text += &format!("app\t{key}\t{}\n", field(sink));
        }
        text
    }

    fn parse(text: &str) -> Record {
        let mut lines = text.lines();
        let field = |v: &str| Some(v.trim()).filter(|v| !v.is_empty()).map(str::to_string);
        let mut record = Record {
            before: lines.next().and_then(field),
            sink: lines.next().and_then(|l| l.trim().parse().ok()),
            ..Record::default()
        };
        for line in lines {
            let mut parts = line.splitn(3, '\t');
            match (parts.next(), parts.next(), parts.next()) {
                (Some("stream"), Some(serial), Some(sink)) => {
                    if let Ok(serial) = serial.parse() {
                        record.streams.insert(serial, field(sink));
                    }
                }
                (Some("app"), Some(key), Some(sink)) => {
                    record.apps.insert(key.into(), field(sink));
                }
                _ => {}
            }
        }
        record
    }
}

/// Notes in `record` where each stream `now` shows sent to a sink other than `id` plays, and
/// forgets ended ones. Returns our sink's serial and the streams to move into it.
fn note(now: &Snapshot, id: u32, record: &mut Record) -> Option<(u64, Vec<u32>)> {
    let ours = now.sinks.iter().find(|d| d.id == id)?.serial?;
    record.sink = Some(ours);
    let playing: BTreeSet<u64> = (now.apps.iter().flat_map(|a| &a.streams))
        .filter_map(|s| s.serial)
        .collect();
    record.streams.retain(|serial, _| playing.contains(serial));
    // One following the default, ours, stays: a move could hide WirePlumber restoring its pick.
    let elsewhere = |s: &&Stream| s.target_serial.map_or(s.named_target, |t| t != ours);
    let mut moves = Vec::new();
    for app in &now.apps {
        for stream in app.streams.iter().filter(elsewhere) {
            let Some(serial) = stream.serial else {
                continue;
            };
            let from = stream.target.clone().filter(|t| t != NAME);
            record.streams.insert(serial, from.clone());
            record.apps.insert(app.key.clone(), from);
            moves.push(stream.id);
        }
    }
    Some((ours, moves))
}

/// The moves back as our sink goes: each stream still playing to it goes where `record` says it,
/// or else its program, played before while that sink is there, and otherwise follows the default.
fn put_back(now: &Snapshot, record: &Record) -> Vec<Write> {
    let Some(ours) = record.sink else {
        return Vec::new();
    };
    let mut writes = Vec::new();
    for app in &now.apps {
        for stream in app.streams.iter().filter(|s| s.target_serial == Some(ours)) {
            let by_stream = stream.serial.and_then(|s| record.streams.get(&s));
            let before = by_stream.or_else(|| record.apps.get(&app.key));
            let sink = (before.and_then(Option::as_deref))
                .and_then(|b| now.sinks.iter().find(|d| d.name == b)?.serial);
            writes.push(Write::Target(stream.id, sink));
        }
    }
    writes
}

/// `$XDG_RUNTIME_DIR/reprise/rdp/sound-before`: the [`Record`] of the live sink.
fn record_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|d| d.is_absolute())
        .map(|d| d.join("reprise/rdp/sound-before"))
}

/// Give back what a server killed while a client listened left on its sink.
pub fn recover() {
    if let Some(record) = record_path() {
        start_mending(recover_at(&record, repriseaudio::snapshot_now, put_wait));
    }
}

/// [`recover`] from the record at `path`; the programs it leaves [`owed`].
fn recover_at(path: &Path, now: fn() -> Snapshot, put: fn(&[Write]) -> bool) -> Owed {
    let Ok(text) = fs::read_to_string(path) else {
        return Owed::new();
    };
    let record = Record::parse(&text);
    let now = now();
    if now.sinks.is_empty() {
        return Owed::new();
    }
    let mut writes = put_back(&now, &record);
    let default = (record.before.as_deref()).and_then(|b| left_behind(&now, b));
    writes.extend(default.map(Write::Default));
    if !put(&writes) {
        return Owed::new();
    }
    if default.is_some() {
        log::info!(
            "sound: {} is the default sink again",
            record.before.as_deref().unwrap_or_default()
        );
    }
    let _ = fs::remove_file(path);
    owed(&now, &record)
}

/// One PipeWire write.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Write {
    /// Make node `id` the default sink.
    Default(u32),
    /// Move stream `id` to the sink with this serial, or back to the default.
    Target(u32, Option<u64>),
}

/// Queue `writes` in order and wait up to [`GIVE_BACK`] for all of them to take.
fn put_wait(writes: &[Write]) -> bool {
    let (done, given) = mpsc::channel();
    for write in writes {
        let done = done.clone();
        let then = move |ok| {
            let _ = done.send(ok);
        };
        let queued = match *write {
            Write::Default(id) => repriseaudio::set_default_then(id, then),
            Write::Target(id, sink) => repriseaudio::set_target_then(id, sink, then),
        };
        if !queued {
            return false;
        }
    }
    let until = Instant::now() + GIVE_BACK;
    writes.iter().all(|_| {
        let left = until.saturating_duration_since(Instant::now());
        given.recv_timeout(left) == Ok(true)
    })
}

/// A thread handing `step` each state PipeWire reports, until `step` says it is done or
/// [`Watcher::end`] stops it.
struct Watcher<T> {
    /// `true` when PipeWire changed, `false` to stop.
    wake: mpsc::Sender<bool>,
    thread: JoinHandle<T>,
}

impl<T: Send + 'static> Watcher<T> {
    fn start(
        name: &str,
        state: T,
        mut step: impl FnMut(&Snapshot, &mut T) -> bool + Send + 'static,
    ) -> io::Result<Watcher<T>> {
        let (wake, woken) = mpsc::channel();
        let changed = wake.clone();
        let thread = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let watch = repriseaudio::watch(move || {
                    let _ = changed.send(true);
                });
                let mut state = state;
                while woken.recv() == Ok(true) {
                    let Some(now) = std::iter::from_fn(|| watch.try_recv().ok()).last() else {
                        continue;
                    };
                    if !step(&now, &mut state) {
                        break;
                    }
                }
                state
            })?;
        Ok(Watcher { wake, thread })
    }

    /// Stops it; the state, unless the thread panicked.
    fn end(self) -> Option<T> {
        let _ = self.wake.send(false);
        self.thread.join().ok()
    }
}

/// Moves each playback stream sent to another sink into ours, node `id`, while it lives,
/// keeping the [`Record`] of where each played before at `path`.
fn follow(id: u32, record: Record, path: Option<PathBuf>) -> io::Result<Watcher<Record>> {
    Watcher::start("reprise-rdp-streams", record, move |now, record| {
        let noted = record.clone();
        let Some((ours, moves)) = note(now, id, record) else {
            return true;
        };
        if let Some(p) = path.as_deref().filter(|_| *record != noted) {
            keep_record(p, record);
        }
        for stream in moves {
            repriseaudio::set_target(stream, Some(ours));
        }
        true
    })
}

/// Programs owed their output, by [`repriseaudio::App::key`], with the sink each played to.
type Owed = BTreeMap<String, String>;

/// The programs a sink going away leaves owed: moved into it with an output picked, and with no
/// stream there left to move back. WirePlumber remembers our sink for them, gone with it, so
/// their next streams would follow the default.
fn owed(now: &Snapshot, record: &Record) -> Owed {
    let in_ours = |key: &str| {
        (now.apps.iter().filter(|a| a.key == key))
            .flat_map(|a| &a.streams)
            .any(|s| s.target_serial.is_some() && s.target_serial == record.sink)
    };
    (record.apps.iter())
        .filter_map(|(key, sink)| Some((key.clone(), sink.clone()?)))
        .filter(|(key, _)| !in_ours(key))
        .collect()
}

/// The moves that send each owed program's new streams back to its sink: those that follow
/// the default, while that sink is there. A program seen playing is no longer owed; one already
/// sent somewhere was given an output meanwhile.
fn mend(now: &Snapshot, owed: &mut Owed) -> Vec<(u32, u64)> {
    let mut moves = Vec::new();
    for app in &now.apps {
        let Some(sink) = owed.remove(&app.key) else {
            continue;
        };
        let Some(serial) = now
            .sinks
            .iter()
            .find(|d| d.name == sink)
            .and_then(|d| d.serial)
        else {
            continue;
        };
        let following = |s: &&Stream| s.target_serial.is_none() && !s.named_target;
        moves.extend(app.streams.iter().filter(following).map(|s| (s.id, serial)));
    }
    moves
}

/// The [`mend`]ing after the last sink, until each owed program played again or a new sink takes
/// over what is left.
static MENDING: Mutex<Option<Watcher<Owed>>> = Mutex::new(None);

/// Mend `owed` on a thread of its own, along with whatever is owed already.
fn start_mending(owed: Owed) {
    let mut all = stop_mending();
    all.extend(owed);
    let owed = all;
    if owed.is_empty() {
        return;
    }
    let mending = Watcher::start("reprise-rdp-mend", owed, |now, owed| {
        for (stream, sink) in mend(now, owed) {
            repriseaudio::set_target(stream, Some(sink));
        }
        !owed.is_empty()
    });
    match mending {
        Ok(m) => *MENDING.lock().unwrap_or_else(PoisonError::into_inner) = Some(m),
        Err(e) => log::warn!("sound: programs that stopped while connected lose their output: {e}"),
    }
}

/// Stops the mending; what was still owed.
fn stop_mending() -> Owed {
    let mending = MENDING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    mending.and_then(Watcher::end).unwrap_or_default()
}

/// Gives the default and the moved streams back when dropped, by every way out of [`run`]. The
/// record stays while PipeWire cannot be asked or a write fails, for [`recover`].
struct GiveBack {
    /// Our sink's node id.
    id: u32,
    /// What there is to give back, while no [`follow`] keeps it.
    record: Record,
    path: Option<PathBuf>,
    follow: Option<Watcher<Record>>,
    now: fn() -> Snapshot,
    put: fn(&[Write]) -> bool,
    mend: fn(Owed),
}

impl Drop for GiveBack {
    fn drop(&mut self) {
        let record = match self.follow.take() {
            Some(follow) => match follow.end() {
                Some(record) => record,
                None => return,
            },
            None => std::mem::take(&mut self.record),
        };
        let until = Instant::now() + GIVE_BACK;
        let default = loop {
            let now = (self.now)();
            if now.sinks.is_empty() || superseded(&now, self.id) {
                return;
            }
            let mut writes = put_back(&now, &record);
            let before = record.before.as_deref();
            // Gone with a PipeWire restart, our sink may still be the configured default.
            let gone = !now.sinks.iter().any(|d| d.name == NAME);
            let default = match gone {
                false => give_back(&now, before),
                true => before.and_then(|b| left_behind(&now, b)),
            };
            writes.extend(default.map(Write::Default));
            if (self.put)(&writes) {
                (self.mend)(owed(&now, &record));
                break default;
            }
            // WirePlumber takes writes again a moment after PipeWire is back.
            if !gone || Instant::now() >= until {
                return;
            }
            std::thread::sleep(RETRY);
        };
        if default.is_some() {
            log::info!(
                "sound: {} is the default sink again",
                record.before.as_deref().unwrap_or_default()
            );
        }
        if let Some(p) = &self.path {
            let _ = fs::remove_file(p);
        }
    }
}

fn keep_record(path: &Path, record: &Record) {
    let written = path
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(path, record.to_text()));
    if let Err(e) = written {
        log::warn!("sound: {}: {e}", path.display());
    }
}

/// The shared sink's thread: make the sink the default, move the streams sent elsewhere into it,
/// report it `ready`, and give both back once stopped.
fn run(stopped: pipewire::channel::Receiver<()>, ready: &Mutex<Ready>) -> Result<(), String> {
    pipewire::init();
    let path = record_path();
    let mut owed = stop_mending();
    if let Some(p) = &path {
        owed.extend(recover_at(p, repriseaudio::snapshot_now, put_wait));
    }
    let before = taken_over(&repriseaudio::snapshot_now());
    monitor::on_pipewire(stopped, |main_loop, core, stopping| {
        let node: pipewire::node::Node = core
            .create_object(
                "adapter",
                &properties! {
                    "factory.name" => "support.null-audio-sink",
                    "node.name" => NAME,
                    "node.description" => DESCRIPTION,
                    "media.class" => "Audio/Sink",
                    "audio.position" => "FL,FR",
                    "monitor.channel-volumes" => "true",
                },
            )
            .map_err(|e| format!("no sink: {e}"))?;
        let id = Rc::new(Cell::new(None));
        let _bound = node
            .upcast_ref()
            .add_listener_local()
            .bound({
                let id = id.clone();
                move |global| id.set(Some(global))
            })
            .register();
        let synced = roundtrip(main_loop, core);
        // A stop handled in the same dispatch as `done` has already spent its quit.
        if stopping.get() {
            return Ok(());
        }
        synced?;
        let id = id.get().ok_or("PipeWire did not make the sink")?;
        // An owed program's next stream comes back into ours, WirePlumber's pick for it.
        let record = Record {
            before,
            apps: owed
                .into_iter()
                .map(|(key, sink)| (key, Some(sink)))
                .collect(),
            ..Record::default()
        };
        if let Some(p) = &path {
            keep_record(p, &record);
        }
        let mut guard = GiveBack {
            id,
            record: record.clone(),
            path: path.clone(),
            follow: None,
            now: repriseaudio::snapshot_now,
            put: put_wait,
            mend: start_mending,
        };
        if !put_wait(&[Write::Default(id)]) {
            log::warn!("sound: \"{DESCRIPTION}\" did not become the default sink");
        }
        match follow(id, record, path) {
            Ok(follow) => guard.follow = Some(follow),
            Err(e) => log::warn!("sound: streams that picked an output stay on it: {e}"),
        }
        report(ready, true);
        main_loop.run();
        Ok(())
    })
}

fn roundtrip(
    main_loop: &pipewire::main_loop::MainLoopRc,
    core: &pipewire::core::CoreRc,
) -> Result<(), String> {
    let pending = core.sync(0).map_err(|e| e.to_string())?;
    let done = Rc::new(Cell::new(false));
    let _listener = core
        .add_listener_local()
        .done({
            let done = done.clone();
            let weak = main_loop.downgrade();
            move |id, seq| {
                if id == pipewire::core::PW_ID_CORE && seq == pending {
                    done.set(true);
                    if let Some(l) = weak.upgrade() {
                        l.quit();
                    }
                }
            }
        })
        .register();
    main_loop.run();
    if done.get() {
        Ok(())
    } else {
        Err("PipeWire went away".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use repriseaudio::{App, Channels, Device};
    use std::cell::RefCell;

    fn sink(id: u32, name: &str) -> Device {
        Device {
            id,
            name: name.into(),
            serial: Some(id.into()),
            label: name.into(),
            volume: 1.0,
            mute: false,
            card: None,
            channels: Channels::default(),
            route: None,
        }
    }

    fn now(default: &str) -> Snapshot {
        Snapshot {
            sinks: vec![sink(40, "speakers"), sink(41, "hdmi"), sink(77, NAME)],
            default_sink: Some(default.into()),
            ..Snapshot::default()
        }
    }

    #[test]
    fn the_default_sink_comes_back_on_disconnect() {
        assert_eq!(give_back(&now(NAME), Some("speakers")), Some(40));
    }

    #[test]
    fn a_default_changed_by_the_user_is_left_alone() {
        assert_eq!(give_back(&now("hdmi"), Some("speakers")), None);
    }

    #[test]
    fn the_configured_default_is_the_one_given_back() {
        let mut snap = configured(Some("speakers"));
        assert_eq!(taken_over(&snap).as_deref(), Some("speakers"));
        snap.configured_sink = None;
        assert_eq!(
            taken_over(&snap).as_deref(),
            Some("hdmi"),
            "none configured"
        );
        snap.configured_sink = Some(NAME.into());
        assert_eq!(
            taken_over(&snap).as_deref(),
            Some("hdmi"),
            "a crashed server's"
        );
        snap.default_sink = Some(NAME.into());
        assert_eq!(taken_over(&snap), None, "our own sink is never recorded");
    }

    #[test]
    fn a_default_that_went_away_is_not_given_back() {
        assert_eq!(give_back(&now(NAME), Some("usb-headset")), None);
        assert_eq!(give_back(&now(NAME), None), None);
    }

    fn configured(name: Option<&str>) -> Snapshot {
        Snapshot {
            sinks: vec![sink(40, "speakers"), sink(41, "hdmi")],
            default_sink: Some("hdmi".into()),
            configured_sink: name.map(Into::into),
            ..Snapshot::default()
        }
    }

    #[test]
    fn a_killed_servers_record_restores_the_default_still_naming_its_sink() {
        assert_eq!(left_behind(&configured(Some(NAME)), "speakers"), Some(40));
        assert_eq!(left_behind(&configured(None), "speakers"), Some(40));
        assert_eq!(
            left_behind(&configured(Some("hdmi")), "speakers"),
            None,
            "a default picked since stays"
        );
        assert_eq!(left_behind(&configured(Some(NAME)), "usb-headset"), None);
        assert_eq!(left_behind(&configured(Some(NAME)), NAME), None);
    }

    #[test]
    fn a_sink_that_never_lets_go_is_passed_over() {
        let turn = Turn::new();
        let stuck = turn.take(JOIN);
        let started = Instant::now();
        let next = turn.take(Duration::from_millis(50));
        assert!(started.elapsed() < JOIN);
        drop(stuck);
        let waited = Instant::now();
        let after = turn.take(Duration::from_millis(200));
        assert!(
            waited.elapsed() >= Duration::from_millis(200),
            "the stuck sink letting go does not free the turn it lost"
        );
        drop((next, after));
        let _again = turn.take(JOIN);
    }

    #[test]
    fn the_next_sink_starts_once_the_last_lets_go() {
        static TURN: Turn = Turn::new();
        let last = TURN.take(JOIN);
        let next = std::thread::spawn(|| {
            let started = Instant::now();
            drop(TURN.take(JOIN));
            started.elapsed()
        });
        std::thread::sleep(Duration::from_millis(50));
        drop(last);
        assert!(next.join().unwrap() < JOIN);
    }

    #[test]
    fn the_server_exits_after_a_sink_dropped_on_its_runtime_gives_the_default_back() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let given_back = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let code = crate::run_to_end(&rt, tokio::task::LocalSet::new(), {
            let given_back = given_back.clone();
            async move {
                let (holding, held) = mpsc::channel();
                let sink = Worker::spawn("test-sink", move |_stopped| {
                    let _one = ONE.take(JOIN);
                    let _ = holding.send(());
                    std::thread::sleep(Duration::from_millis(200));
                    given_back.store(true, std::sync::atomic::Ordering::SeqCst);
                })
                .unwrap();
                held.recv().unwrap();
                drop(sink);
                std::process::ExitCode::SUCCESS
            }
        });
        assert_eq!(code, std::process::ExitCode::SUCCESS);
        assert!(given_back.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn dropping_a_sink_on_the_runtime_does_not_wait_for_its_thread() {
        let (stop, _stopped) = pipewire::channel::channel();
        let (ending, ended) = mpsc::channel::<()>();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            drop(ending);
        });
        let started = Instant::now();
        drop(Worker {
            stop,
            thread: Some((thread, ended)),
        });
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn acquire_returns_the_live_one() {
        let slot = Mutex::new(Weak::new());
        let made = Cell::new(0);
        let make = || {
            made.set(made.get() + 1);
            Ok(())
        };
        let first = acquire(&slot, |_| true, make).unwrap();
        let second = acquire(&slot, |_| true, make).unwrap();
        assert_eq!(made.get(), 1);
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn acquire_makes_a_new_one_after_the_last_drop() {
        let slot = Mutex::new(Weak::new());
        let made = Cell::new(0);
        let make = || {
            made.set(made.get() + 1);
            Ok(())
        };
        drop(acquire(&slot, |_| true, make).unwrap());
        drop(acquire(&slot, |_| true, make).unwrap());
        assert_eq!(made.get(), 2);
    }

    #[test]
    fn acquire_replaces_a_dead_one() {
        let slot = Mutex::new(Weak::new());
        let made = Cell::new(0);
        let make = || {
            made.set(made.get() + 1);
            Ok(made.get())
        };
        let dead = acquire(&slot, |_| true, make).unwrap();
        let fresh = acquire(&slot, |n| *n != 1, make).unwrap();
        assert_eq!(made.get(), 2);
        assert!(!Arc::ptr_eq(&dead, &fresh));
        let again = acquire(&slot, |n| *n != 1, make).unwrap();
        assert!(
            Arc::ptr_eq(&fresh, &again),
            "the replacement is the one held"
        );
    }

    struct Counted<'a>(&'a Cell<u32>);

    impl Drop for Counted<'_> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn the_shared_sink_stays_until_its_last_user_goes() {
        let dropped = Cell::new(0);
        let slot = Mutex::new(Weak::new());
        let first = acquire(&slot, |_| true, || Ok(Counted(&dropped))).unwrap();
        let second = acquire(&slot, |_| true, || Ok(Counted(&dropped))).unwrap();
        drop(first);
        assert_eq!(dropped.get(), 0, "one user is left");
        drop(second);
        assert_eq!(dropped.get(), 1, "gone with the last user");
    }

    thread_local! {
        static PUT: RefCell<Vec<Write>> = const { RefCell::new(Vec::new()) };
    }

    fn ours() -> Snapshot {
        now(NAME)
    }

    fn put(writes: &[Write]) -> bool {
        PUT.with_borrow_mut(|p| p.extend_from_slice(writes));
        true
    }

    fn before(name: &str) -> Record {
        Record {
            before: Some(name.into()),
            ..Record::default()
        }
    }

    fn give_back_guard(record: &Path, now: fn() -> Snapshot) -> GiveBack {
        GiveBack {
            id: 77,
            record: Record::parse(&fs::read_to_string(record).unwrap()),
            path: Some(record.into()),
            follow: None,
            now,
            put,
            mend: |_| {},
        }
    }

    fn capture_failing_after_the_switch(guard: GiveBack) -> Result<(), String> {
        let _guard = guard;
        Err("no capture stream".into())
    }

    #[test]
    fn the_default_comes_back_when_capture_fails_after_the_switch() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("sound-before");
        keep_record(&record, &before("speakers"));
        let guard = give_back_guard(&record, ours);
        assert!(capture_failing_after_the_switch(guard).is_err());
        assert_eq!(PUT.take(), [Write::Default(40)]);
        assert!(!record.exists(), "the record goes once the default is back");
    }

    #[test]
    fn the_record_stays_while_pipewire_is_away() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("sound-before");
        keep_record(&record, &before("speakers"));
        drop(give_back_guard(&record, Snapshot::default));
        assert_eq!(PUT.take(), []);
        assert!(record.exists(), "kept for recovery");
    }

    fn ours_and_the_next_connections() -> Snapshot {
        let mut snap = now(NAME);
        snap.sinks.push(sink(78, NAME));
        snap
    }

    #[test]
    fn a_sink_the_next_connection_replaced_leaves_the_default_and_record_alone() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("sound-before");
        keep_record(&record, &before("speakers"));
        drop(give_back_guard(&record, ours_and_the_next_connections));
        assert_eq!(PUT.take(), [], "the default stays on the live sink");
        assert!(record.exists(), "the record is the live sink's");
    }

    #[test]
    fn recovery_puts_the_recorded_default_back_and_forgets_it() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("sound-before");
        keep_record(&record, &before("speakers"));
        recover_at(&record, Snapshot::default, put);
        assert!(record.exists(), "kept while PipeWire is away");
        recover_at(&record, || configured(Some(NAME)), put);
        assert_eq!(PUT.take(), [Write::Default(40)]);
        assert!(!record.exists());
    }

    fn stream(id: u32, target: Option<&Device>) -> Stream {
        Stream {
            id,
            serial: Some(u64::from(id) * 10),
            channels: Channels::default(),
            target: target.map(|d| d.name.clone()),
            target_serial: target.and_then(|d| d.serial),
            named_target: false,
        }
    }

    fn app(key: &str, streams: Vec<Stream>) -> App {
        App {
            key: key.into(),
            label: key.into(),
            icon_hints: Vec::new(),
            binary: None,
            pids: Vec::new(),
            streams,
            volume: 1.0,
            mute: false,
        }
    }

    /// `apps` playing while our sink 77 is the default.
    fn playing(apps: Vec<App>) -> Snapshot {
        Snapshot { apps, ..ours() }
    }

    #[test]
    fn every_stream_sent_elsewhere_is_moved_in_and_where_it_played_noted() {
        let (speakers, remote) = (sink(40, "speakers"), sink(77, NAME));
        let named = Stream {
            named_target: true,
            ..stream(91, None)
        };
        let now = playing(vec![
            app("player", vec![stream(90, Some(&speakers))]),
            app(
                "browser",
                vec![named, stream(92, Some(&remote)), stream(93, None)],
            ),
        ]);
        let mut record = before("hdmi");
        let (serial, moves) = note(&now, 77, &mut record).expect("our sink is up");
        assert_eq!(serial, 77);
        assert_eq!(
            moves,
            [90, 91],
            "the one already in ours and the one following the default stay"
        );
        assert_eq!(record.sink, Some(77));
        let streams = [(900, Some("speakers".into())), (910, None)];
        assert_eq!(record.streams, BTreeMap::from(streams));
        let apps = [
            ("player".into(), Some("speakers".into())),
            ("browser".into(), None),
        ];
        assert_eq!(record.apps, BTreeMap::from(apps));
        assert_eq!(note(&Snapshot::default(), 77, &mut record), None);
    }

    #[test]
    fn a_stream_that_ended_is_forgotten_but_its_program_is_not() {
        let speakers = sink(40, "speakers");
        let mut record = Record::default();
        let then = playing(vec![app("player", vec![stream(90, Some(&speakers))])]);
        note(&then, 77, &mut record).unwrap();
        note(&playing(Vec::new()), 77, &mut record).unwrap();
        assert!(record.streams.is_empty());
        assert_eq!(record.apps["player"].as_deref(), Some("speakers"));
    }

    #[test]
    fn moved_streams_go_back_where_they_or_their_program_played() {
        let remote = sink(77, NAME);
        let hdmi = sink(41, "hdmi");
        let record = Record {
            sink: Some(77),
            streams: BTreeMap::from([(900, Some("speakers".into())), (910, None)]),
            apps: BTreeMap::from([("player".into(), Some("hdmi".into()))]),
            ..Record::default()
        };
        let now = playing(vec![
            app(
                "player",
                vec![
                    stream(90, Some(&remote)),
                    stream(91, Some(&remote)),
                    stream(93, Some(&remote)),
                    stream(94, Some(&hdmi)),
                ],
            ),
            app("game", vec![stream(95, Some(&remote))]),
        ]);
        assert_eq!(
            put_back(&now, &record),
            [
                Write::Target(90, Some(40)),
                Write::Target(91, None),
                Write::Target(93, Some(41)),
                Write::Target(95, None),
            ],
            "94 was moved again meanwhile; 93 started since and goes where its program played"
        );
        let mut unplugged = now.clone();
        unplugged.sinks.retain(|d| d.name != "speakers");
        assert_eq!(put_back(&unplugged, &record)[0], Write::Target(90, None));
    }

    #[test]
    fn a_killed_servers_record_moves_its_streams_back_once_its_sink_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sound-before");
        let record = Record {
            before: Some("speakers".into()),
            sink: Some(77),
            streams: BTreeMap::from([(900, Some("hdmi".into()))]),
            ..Record::default()
        };
        keep_record(&path, &record);
        assert_eq!(Record::parse(&fs::read_to_string(&path).unwrap()), record);
        recover_at(
            &path,
            || Snapshot {
                apps: vec![app("player", vec![stream(90, Some(&sink(77, NAME)))])],
                ..configured(Some(NAME))
            },
            put,
        );
        assert_eq!(
            PUT.take(),
            [Write::Target(90, Some(41)), Write::Default(40)]
        );
        assert!(!path.exists());
    }

    #[test]
    fn a_program_whose_moved_streams_all_ended_is_owed_its_output() {
        let remote = sink(77, NAME);
        let record = Record {
            sink: Some(77),
            apps: BTreeMap::from([
                ("player".into(), Some("speakers".into())),
                ("browser".into(), None),
                ("game".into(), Some("hdmi".into())),
            ]),
            ..Record::default()
        };
        let now = playing(vec![app("game", vec![stream(95, Some(&remote))])]);
        assert_eq!(
            owed(&now, &record),
            Owed::from([("player".into(), "speakers".into())]),
            "the game goes back with its stream; the browser followed the default"
        );
    }

    #[test]
    fn an_owed_programs_next_stream_goes_back_to_its_output() {
        let hdmi = sink(41, "hdmi");
        let mut owed = Owed::from([
            ("player".into(), "speakers".into()),
            ("game".into(), "hdmi".into()),
            ("radio".into(), "headphones".into()),
            ("idle".into(), "speakers".into()),
        ]);
        let now = Snapshot {
            apps: vec![
                app("player", vec![stream(90, None), stream(91, Some(&hdmi))]),
                app("game", vec![stream(92, Some(&hdmi))]),
                app("radio", vec![stream(93, None)]),
            ],
            ..now("speakers")
        };
        assert_eq!(
            mend(&now, &mut owed),
            [(90, 40)],
            "91 and the game were sent somewhere meanwhile; the headphones are gone"
        );
        assert_eq!(owed, Owed::from([("idle".into(), "speakers".into())]));
    }

    thread_local! {
        static MENDED: RefCell<Vec<Owed>> = const { RefCell::new(Vec::new()) };
    }

    #[test]
    fn giving_back_mends_what_it_could_not_move_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sound-before");
        let record = Record {
            apps: BTreeMap::from([("player".into(), Some("hdmi".into()))]),
            ..before("speakers")
        };
        keep_record(&path, &record);
        let mut guard = give_back_guard(&path, ours);
        guard.mend = |owed| MENDED.with_borrow_mut(|m| m.push(owed));
        drop(guard);
        assert_eq!(PUT.take(), [Write::Default(40)]);
        assert_eq!(
            MENDED.take(),
            [Owed::from([("player".into(), "hdmi".into())])]
        );
    }

    #[test]
    fn a_record_of_a_default_alone_reads_as_one() {
        assert_eq!(Record::parse("speakers"), before("speakers"));
        assert_eq!(Record::parse("speakers\n"), before("speakers"));
        let blank = Record::parse("\n\napp\tfoo/bar\t\n");
        assert_eq!(blank.before, None);
        assert_eq!(blank.apps["foo/bar"], None);
    }

    thread_local! {
        static REFUSED: Cell<u32> = const { Cell::new(0) };
    }

    /// [`put`] after failing as many times as [`REFUSED`] says, as WirePlumber does while it
    /// comes back.
    fn put_after_refusals(writes: &[Write]) -> bool {
        match REFUSED.get() {
            0 => put(writes),
            n => {
                REFUSED.set(n - 1);
                false
            }
        }
    }

    #[test]
    fn a_sink_pipewire_took_away_gives_back_the_configured_default_still_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("sound-before");
        keep_record(&record, &before("speakers"));
        REFUSED.set(2);
        let mut guard = give_back_guard(&record, || configured(Some(NAME)));
        guard.put = put_after_refusals;
        drop(guard);
        assert_eq!(PUT.take(), [Write::Default(40)]);
        assert!(!record.exists(), "the record goes once the default is back");
    }
}
