//! The "Remote client" sink: one PipeWire null sink made the default while any viewer listens,
//! its monitor captured once per viewer at that viewer's rate.

use pipewire::properties::properties;
use pipewire::proxy::ProxyT;
use pipewire::spa;
use pipewire::spa::param::audio::{AudioFormat, AudioInfoRaw};
use pipewire::spa::pod::Pod;
use pipewire::spa::pod::serialize::PodSerializer;
use pipewire::spa::sys as spa_sys;
use pipewire::stream::{StreamFlags, StreamRc, StreamState};
use repriseaudio::Snapshot;
use std::cell::Cell;
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
/// How long one default switch may take before the sink goes on regardless.
const GIVE_BACK: Duration = Duration::from_secs(2);
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
            if let Err(e) = capture(rate, Box::new(samples), stopped) {
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

/// Keeps the shared sink the default with nothing captured from it, so what plays in the session
/// goes nowhere rather than to this computer's speakers.
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

/// `$XDG_RUNTIME_DIR/reprise/rdp/sound-before`: the default a sink took over, kept until it is
/// given back so a server killed meanwhile can be made good.
fn record_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|d| d.is_absolute())
        .map(|d| d.join("reprise/rdp/sound-before"))
}

/// Give back a default that a server killed while a client listened left on its sink.
pub fn recover() {
    if let Some(record) = record_path() {
        recover_at(&record, repriseaudio::snapshot_now, set_default_wait);
    }
}

fn recover_at(record: &Path, now: fn() -> Snapshot, set: fn(u32) -> bool) {
    let Ok(recorded) = fs::read_to_string(record) else {
        return;
    };
    let now = now();
    if now.sinks.is_empty() {
        return;
    }
    if let Some(id) = left_behind(&now, recorded.trim()) {
        if !set(id) {
            return;
        }
        log::info!("sound: {} is the default sink again", recorded.trim());
    }
    let _ = fs::remove_file(record);
}

/// Switch the default to `id`, waiting up to [`GIVE_BACK`] for it to take.
fn set_default_wait(id: u32) -> bool {
    let (done, given) = mpsc::channel();
    repriseaudio::set_default_then(id, move |ok| {
        let _ = done.send(ok);
    }) && given.recv_timeout(GIVE_BACK) == Ok(true)
}

/// Gives the default back when dropped, so every way out of [`run`] after the switch does.
/// The record stays while PipeWire cannot be asked or the switch fails, for [`recover`].
struct GiveBack {
    /// Our sink's node id.
    id: u32,
    before: Option<String>,
    record: Option<PathBuf>,
    now: fn() -> Snapshot,
    set: fn(u32) -> bool,
}

impl Drop for GiveBack {
    fn drop(&mut self) {
        let now = (self.now)();
        if now.sinks.is_empty() || superseded(&now, self.id) {
            return;
        }
        if let Some(back) = give_back(&now, self.before.as_deref()) {
            if !(self.set)(back) {
                return;
            }
            log::info!(
                "sound: {} is the default sink again",
                self.before.as_deref().unwrap_or_default()
            );
        }
        if let Some(r) = &self.record {
            let _ = fs::remove_file(r);
        }
    }
}

/// Write `before` to `record` ahead of the switch.
fn keep_record(record: &Path, before: &str) {
    let written = record
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(record, before));
    if let Err(e) = written {
        log::warn!("sound: {}: {e}", record.display());
    }
}

type Samples = Box<dyn FnMut(&[u8]) + Send>;

/// Run `body` on a new PipeWire connection whose loop quits on a stop from `stopped` or a core
/// error. `body` gets the loop, the core and whether a stop came.
fn on_pipewire(
    stopped: pipewire::channel::Receiver<()>,
    body: impl FnOnce(
        &pipewire::main_loop::MainLoopRc,
        &pipewire::core::CoreRc,
        &Cell<bool>,
    ) -> Result<(), String>,
) -> Result<(), String> {
    let main_loop = pipewire::main_loop::MainLoopRc::new(None).map_err(|e| e.to_string())?;
    let context = pipewire::context::ContextRc::new(&main_loop, None).map_err(|e| e.to_string())?;
    let core = context.connect_rc(None).map_err(|e| e.to_string())?;
    // Attached first, so a stop also ends a wait on a PipeWire that never answers.
    let stopping = Rc::new(Cell::new(false));
    let weak = main_loop.downgrade();
    let _stop = stopped.attach(main_loop.loop_(), {
        let stopping = stopping.clone();
        move |()| {
            stopping.set(true);
            if let Some(l) = weak.upgrade() {
                l.quit();
            }
        }
    });
    let weak = main_loop.downgrade();
    let _errors = core
        .add_listener_local()
        .error(move |id, _, res, message| {
            log::warn!("sound: PipeWire object {id}: {message} ({res})");
            if id == pipewire::core::PW_ID_CORE
                && let Some(l) = weak.upgrade()
            {
                l.quit();
            }
        })
        .register();
    body(&main_loop, &core, &stopping)
}

/// The shared sink's thread: make the sink the default, report it `ready`, and give the default
/// back once stopped.
fn run(stopped: pipewire::channel::Receiver<()>, ready: &Mutex<Ready>) -> Result<(), String> {
    pipewire::init();
    let record = record_path();
    if let Some(r) = &record {
        recover_at(r, repriseaudio::snapshot_now, set_default_wait);
    }
    let before = taken_over(&repriseaudio::snapshot_now());
    on_pipewire(stopped, |main_loop, core, stopping| {
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
        if let (Some(r), Some(b)) = (&record, &before) {
            keep_record(r, b);
        }
        let _give_back = GiveBack {
            id,
            before,
            record,
            now: repriseaudio::snapshot_now,
            set: set_default_wait,
        };
        if !set_default_wait(id) {
            log::warn!("sound: \"{DESCRIPTION}\" did not become the default sink");
        }
        report(ready, true);
        main_loop.run();
        Ok(())
    })
}

/// One viewer's thread: capture the shared sink's monitor at `rate` into `samples`.
fn capture(
    rate: u32,
    samples: Samples,
    stopped: pipewire::channel::Receiver<()>,
) -> Result<(), String> {
    pipewire::init();
    on_pipewire(stopped, |main_loop, core, _| {
        let mut props = properties! {
            *pipewire::keys::MEDIA_TYPE => "Audio",
            *pipewire::keys::MEDIA_CATEGORY => "Capture",
            *pipewire::keys::APP_NAME => "Remote Desktop",
            "stream.capture.sink" => "true",
            "target.object" => NAME,
            "node.dont-reconnect" => "true",
        };
        props.insert("node.latency", format!("{}/{rate}", rate / 50));
        let stream =
            StreamRc::new(core.clone(), "Remote client", props).map_err(|e| e.to_string())?;
        let weak = main_loop.downgrade();
        let _listener = stream
            .add_local_listener_with_user_data(samples)
            .state_changed(move |_, _, _, new| {
                if let StreamState::Error(e) = new {
                    log::warn!("sound: capture: {e}");
                    if let Some(l) = weak.upgrade() {
                        l.quit();
                    }
                }
            })
            .process(|stream, samples| {
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };
                let Some(data) = buffer.datas_mut().first_mut() else {
                    return;
                };
                let chunk = data.chunk();
                let (offset, size) = (chunk.offset() as usize, chunk.size() as usize);
                if let Some(bytes) = data.data().and_then(|d| d.get(offset..offset + size)) {
                    samples(bytes);
                }
            })
            .register()
            .map_err(|e| e.to_string())?;
        let param = format_param(rate)?;
        let pod = Pod::from_bytes(&param).ok_or("bad format pod")?;
        stream
            .connect(
                spa::utils::Direction::Input,
                None,
                StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
                &mut [pod],
            )
            .map_err(|e| e.to_string())?;
        log::info!("sound: capturing \"{DESCRIPTION}\"");
        main_loop.run();
        let _ = stream.disconnect();
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

fn format_param(rate: u32) -> Result<Vec<u8>, String> {
    let mut info = AudioInfoRaw::new();
    info.set_format(AudioFormat::S16LE);
    info.set_rate(rate);
    info.set_channels(2);
    let mut position = [0; spa_sys::SPA_AUDIO_MAX_CHANNELS as usize];
    position[0] = spa_sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = spa_sys::SPA_AUDIO_CHANNEL_FR;
    info.set_position(position);
    let object = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: info.into(),
    };
    PodSerializer::serialize(
        io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(object),
    )
    .map(|(cursor, _)| cursor.into_inner())
    .map_err(|e| format!("format pod: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use repriseaudio::{Channels, Device};

    fn sink(id: u32, name: &str) -> Device {
        Device {
            id,
            name: name.into(),
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
        static SET: Cell<Option<u32>> = const { Cell::new(None) };
    }

    fn ours() -> Snapshot {
        now(NAME)
    }

    fn set(id: u32) -> bool {
        SET.set(Some(id));
        true
    }

    fn capture_failing_after_the_switch(guard: GiveBack) -> Result<(), String> {
        let _guard = guard;
        Err("no capture stream".into())
    }

    #[test]
    fn the_default_comes_back_when_capture_fails_after_the_switch() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("sound-before");
        keep_record(&record, "speakers");
        let guard = GiveBack {
            id: 77,
            before: Some("speakers".into()),
            record: Some(record.clone()),
            now: ours,
            set,
        };
        assert!(capture_failing_after_the_switch(guard).is_err());
        assert_eq!(SET.get(), Some(40));
        assert!(!record.exists(), "the record goes once the default is back");
    }

    #[test]
    fn the_record_stays_while_pipewire_is_away() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("sound-before");
        keep_record(&record, "speakers");
        drop(GiveBack {
            id: 77,
            before: Some("speakers".into()),
            record: Some(record.clone()),
            now: Snapshot::default,
            set,
        });
        assert_eq!(SET.get(), None);
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
        keep_record(&record, "speakers");
        drop(GiveBack {
            id: 77,
            before: Some("speakers".into()),
            record: Some(record.clone()),
            now: ours_and_the_next_connections,
            set,
        });
        assert_eq!(SET.get(), None, "the default stays on the live sink");
        assert!(record.exists(), "the record is the live sink's");
    }

    #[test]
    fn recovery_puts_the_recorded_default_back_and_forgets_it() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("sound-before");
        keep_record(&record, "speakers");
        recover_at(&record, Snapshot::default, set);
        assert!(record.exists(), "kept while PipeWire is away");
        recover_at(&record, || configured(Some(NAME)), set);
        assert_eq!(SET.get(), Some(40));
        assert!(!record.exists());
    }
}
