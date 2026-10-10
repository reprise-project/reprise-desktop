//! The PipeWire side of the sound programs: a store fed by `pw-dump -m`, and the `wpctl` and `pw-cli`
//! calls that write back. Nothing here draws; [`Store::snapshot`] projects the stored objects into the
//! shape the UI wants.

mod channels;
mod recent;

pub use channels::{Channels, Side, side};
pub use recent::{Recent, SETTLE};

use std::collections::BTreeMap;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use log::warn;
use serde_json::Value;

const NODE: &str = "PipeWire:Interface:Node";
const METADATA: &str = "PipeWire:Interface:Metadata";
const DEVICE: &str = "PipeWire:Interface:Device";

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: u32,
    /// `node.name`, which the default-device metadata matches on.
    pub name: String,
    pub label: String,
    /// 0..1 linear, as `wpctl` shows it: the cube root of the loudest channel's raw volume.
    pub volume: f64,
    pub mute: bool,
    /// The card it belongs to (`device.id`); `None` for a virtual sink or filter, which cannot be switched off.
    pub card: Option<u32>,
    /// Each channel's level, from which the balance is read.
    pub channels: Channels,
    /// The card route the node plays through, which takes its channel volumes; `None` if the node takes them itself.
    pub route: Option<Route>,
}

/// A sound card's active route for one of its nodes: the `Device` object's id `device`, route `index`
/// on profile device `card_device` (the node's `card.profile.device`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub device: u32,
    pub index: u32,
    pub card_device: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub id: u32,
    pub channels: Channels,
}

/// A sound card as Disable and Enable see it: whether its profile is `off`, which directions its profiles carry, and which profile Enable picks.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub id: u32,
    pub label: String,
    pub off: bool,
    pub sinks: bool,
    pub sources: bool,
    pub off_index: Option<u32>,
    /// What Enable sets: the highest-priority profile other than `off` not reported unavailable, else the highest-priority one.
    pub best: Option<u32>,
}

/// A program playing sound: its playback streams together, one volume per program.
#[derive(Clone, Debug, PartialEq)]
pub struct App {
    /// What ties its streams together, lowercased: the sandbox app id, else `binary/name`, else `node.name`.
    pub key: String,
    pub label: String,
    /// Names to look its icon up by, best first.
    pub icon_hints: Vec<String>,
    /// `application.process.binary`, which a `.desktop` file's `Exec` may name when nothing else matches.
    pub binary: Option<String>,
    pub streams: Vec<Stream>,
    pub volume: f64,
    pub mute: bool,
}

impl App {
    pub fn ids(&self) -> Vec<u32> {
        self.streams.iter().map(|s| s.id).collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub sinks: Vec<Device>,
    pub sources: Vec<Device>,
    pub default_sink: Option<String>,
    pub default_source: Option<String>,
    pub cards: Vec<Card>,
    pub apps: Vec<App>,
}

/// Every object `pw-dump` has reported, keyed by id. `pw-dump -m` prints the full dump, then one array
/// per change carrying only what moved, so objects are merged in rather than replaced.
#[derive(Debug, Default)]
pub struct Store {
    objects: BTreeMap<u32, Value>,
    /// Each node's channel shares the last time it was not silent, by id: a node turned down to nothing hands them on.
    heard: BTreeMap<u32, Vec<f64>>,
}

impl Store {
    pub fn new() -> Store {
        Store::default()
    }

    /// Apply one top-level array from `pw-dump -m`. Anything that is not an array of objects with an
    /// `id` is ignored, as is any object that is not a node, device or metadata.
    pub fn apply(&mut self, array: &Value) {
        let Some(items) = array.as_array() else {
            return;
        };
        for item in items {
            let Some(id) = item.get("id").and_then(Value::as_u64) else {
                continue;
            };
            let id = id as u32;
            if matches!(item.get("info"), Some(Value::Null)) {
                self.objects.remove(&id);
                self.heard.remove(&id);
                continue;
            }
            match self.objects.get_mut(&id) {
                Some(stored) if stored.get("type").and_then(Value::as_str) == Some(METADATA) => {
                    merge_metadata(stored, item)
                }
                Some(stored) => merge(stored, item),
                None => {
                    let kind = item.get("type").and_then(Value::as_str);
                    if matches!(kind, Some(NODE) | Some(METADATA) | Some(DEVICE)) {
                        self.objects.insert(id, item.clone());
                    }
                }
            }
            self.note_heard(id);
        }
    }

    fn note_heard(&mut self, id: u32) {
        let Some(object) = self.objects.get(&id) else {
            return;
        };
        if object.get("type").and_then(Value::as_str) != Some(NODE) {
            return;
        }
        let ch = device(id, object).channels;
        if ch.volume() > 0.0
            && let Some(shares) = ch.heard_shares()
        {
            self.heard.insert(id, shares);
        }
    }

    fn shares(&self, id: u32) -> Vec<f64> {
        self.heard.get(&id).cloned().unwrap_or_default()
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut snap = Snapshot::default();
        for (&id, object) in &self.objects {
            match object.get("type").and_then(Value::as_str) {
                Some(NODE) => {
                    let props = &object["info"]["props"];
                    let list = match props["media.class"].as_str() {
                        Some("Audio/Sink") => &mut snap.sinks,
                        Some("Audio/Source") => &mut snap.sources,
                        Some("Stream/Output/Audio") => {
                            add_stream(&mut snap.apps, id, object, self.shares(id));
                            continue;
                        }
                        _ => continue,
                    };
                    let mut d = device(id, object);
                    d.channels.shares = self.shares(id);
                    d.route = self.route(object);
                    list.push(d);
                }
                Some(DEVICE) if object["info"]["props"]["media.class"] == "Audio/Device" => {
                    snap.cards.push(card(id, object));
                }
                Some(METADATA) if object["props"]["metadata.name"] == "default" => {
                    snap.default_sink = default_name(object, "default.audio.sink");
                    snap.default_source = default_name(object, "default.audio.source");
                }
                _ => {}
            }
        }
        snap.apps.sort_by(|a, b| {
            (a.label.to_lowercase(), &a.key).cmp(&(b.label.to_lowercase(), &b.key))
        });
        snap
    }

    /// The route a card's node plays through: the active `Route` on the node's `card.profile.device`.
    fn route(&self, node: &Value) -> Option<Route> {
        let props = &node["info"]["props"];
        let device = number(&props["device.id"])?;
        let card_device = number(&props["card.profile.device"])?;
        let routes = self.objects.get(&device)?["info"]["params"]["Route"].as_array()?;
        let index = routes
            .iter()
            .find(|r| number(&r["device"]) == Some(card_device))
            .and_then(|r| number(&r["index"]))?;
        Some(Route {
            device,
            index,
            card_device,
        })
    }
}

fn number(v: &Value) -> Option<u32> {
    v.as_u64()
        .or_else(|| v.as_str()?.parse().ok())
        .map(|n| n as u32)
}

impl Snapshot {
    pub fn node(&self, id: u32) -> Option<(&Channels, Option<Route>)> {
        let device = self.sinks.iter().chain(&self.sources).find(|d| d.id == id);
        if let Some(d) = device {
            return Some((&d.channels, d.route));
        }
        self.apps
            .iter()
            .flat_map(|a| &a.streams)
            .find(|s| s.id == id)
            .map(|s| (&s.channels, None))
    }
}

/// Merge `src` over `dst`, recursing into objects so a change carrying only `info.params` leaves
/// `info.props` alone. Arrays replace wholesale (`params.Props` always arrives complete); metadata
/// lists do not, see [`merge_metadata`].
fn merge(dst: &mut Value, src: &Value) {
    match (dst, src) {
        (Value::Object(dst), Value::Object(src)) => {
            for (key, value) in src {
                match dst.get_mut(key) {
                    Some(stored) => merge(stored, value),
                    None => {
                        dst.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (dst, src) => *dst = src.clone(),
    }
}

/// Merge a metadata update into the stored object. Only changed entries are reported, so each replaces
/// the stored entry with the same subject and key; a null value removes it.
fn merge_metadata(dst: &mut Value, src: &Value) {
    let mut entries = dst
        .get_mut("metadata")
        .and_then(Value::as_array_mut)
        .map(std::mem::take)
        .unwrap_or_default();
    for e in src
        .get("metadata")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        entries.retain(|d| (&d["subject"], &d["key"]) != (&e["subject"], &e["key"]));
        if !e["value"].is_null() {
            entries.push(e.clone());
        }
    }
    let mut rest = src.clone();
    if let Some(o) = rest.as_object_mut() {
        o.remove("metadata");
    }
    merge(dst, &rest);
    if let Some(o) = dst.as_object_mut() {
        o.insert("metadata".into(), Value::Array(entries));
    }
}

fn device(id: u32, object: &Value) -> Device {
    let props = &object["info"]["props"];
    let name = props["node.name"].as_str().unwrap_or_default();
    let label = ["node.description", "node.nick", "node.name"]
        .iter()
        .find_map(|key| props[*key].as_str().filter(|s| !s.is_empty()))
        .unwrap_or(name);

    let audio = &object["info"]["params"]["Props"][0];
    // PipeWire's channel volumes are cubic; wpctl shows and takes the root.
    let levels: Vec<f64> = audio["channelVolumes"]
        .as_array()
        .map(|channels| {
            channels
                .iter()
                .filter_map(Value::as_f64)
                .map(f64::cbrt)
                .collect()
        })
        .unwrap_or_default();
    let map: Vec<String> = audio["channelMap"]
        .as_array()
        .map(|m| {
            m.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    // A map that does not name every channel names none of them.
    let map = if map.len() == levels.len() {
        map
    } else {
        Vec::new()
    };
    let channels = Channels {
        map,
        levels,
        shares: Vec::new(),
    };

    Device {
        id,
        name: name.to_string(),
        label: label.to_string(),
        // A stream that has not negotiated its format reports no channels; its overall volume stands in.
        volume: if channels.levels.is_empty() {
            audio["volume"].as_f64().map_or(0.0, f64::cbrt)
        } else {
            channels.volume()
        },
        mute: audio["mute"].as_bool().unwrap_or(false),
        card: number(&props["device.id"]),
        channels,
        route: None,
    }
}

/// Fold playback stream `id` into its app. A peak meter's monitor stream is not a program and is left out.
fn add_stream(apps: &mut Vec<App>, id: u32, object: &Value, shares: Vec<f64>) {
    let props = &object["info"]["props"];
    let text = |key: &str| {
        props[key]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    if props["stream.monitor"] == true || text("stream.monitor").as_deref() == Some("true") {
        return;
    }
    let binary = text("application.process.binary");
    let name = text("application.name");
    let ids = [
        text("pipewire.access.portal.app_id"),
        text("application.id"),
    ];
    // Without an app id the executable alone would fold together programs sharing an interpreter
    // (python, electron, wine), so its name goes with it.
    let key = match (ids.iter().flatten().next(), &binary, &name) {
        (Some(id), ..) => id.clone(),
        (None, None, None) => match text("node.name") {
            Some(n) => n,
            None => return,
        },
        (None, b, n) => format!(
            "{}/{}",
            b.as_deref().unwrap_or_default(),
            n.as_deref().unwrap_or_default()
        ),
    }
    .to_lowercase();
    let mut v = device(id, object);
    v.channels.shares = shares;
    match apps.iter_mut().find(|a| a.key == key) {
        Some(a) => {
            a.streams.push(Stream {
                id,
                channels: v.channels,
            });
            a.volume = a.volume.max(v.volume);
            a.mute &= v.mute;
        }
        None => {
            let mut icon_hints: Vec<String> = Vec::new();
            // PipeWire spells it with a dash; pipewire-pulse passes the PulseAudio underscore through.
            let hints = [text("application.icon-name"), text("application.icon_name")]
                .into_iter()
                .chain(ids)
                .chain([binary.clone(), name.clone()]);
            // The lookup ignores case, so "Firefox" after "firefox" would only be tried twice.
            for h in hints.flatten() {
                if !icon_hints.iter().any(|o| o.eq_ignore_ascii_case(&h)) {
                    icon_hints.push(h);
                }
            }
            apps.push(App {
                label: name.or_else(|| binary.clone()).unwrap_or(v.label),
                key,
                icon_hints,
                binary,
                volume: v.volume,
                streams: vec![Stream {
                    id,
                    channels: v.channels,
                }],
                mute: v.mute,
            });
        }
    }
}

fn card(id: u32, object: &Value) -> Card {
    let props = &object["info"]["props"];
    let label = ["device.description", "device.nick", "device.name"]
        .iter()
        .find_map(|key| props[*key].as_str().filter(|s| !s.is_empty()))
        .unwrap_or_default()
        .to_string();
    let params = &object["info"]["params"];
    let profiles: &[Value] = params["EnumProfile"].as_array().map_or(&[], Vec::as_slice);
    let index = |p: &Value| p["index"].as_u64().map(|i| i as u32);
    let is_off = |p: &Value| p["name"] == "off";
    let carries = |class: &str| {
        profiles.iter().any(|p| {
            p["classes"]
                .as_array()
                .is_some_and(|cs| cs.iter().any(|c| c[0] == class))
        })
    };
    let ranked = |usable: bool| {
        profiles
            .iter()
            .filter(|p| !is_off(p) && (!usable || p["available"] != "no"))
            .max_by_key(|p| p["priority"].as_i64().unwrap_or(0))
            .and_then(index)
    };
    Card {
        id,
        label,
        off: params["Profile"][0]["name"] == "off",
        sinks: carries("Audio/Sink"),
        sources: carries("Audio/Source"),
        off_index: profiles.iter().find(|p| is_off(p)).and_then(index),
        best: ranked(true).or_else(|| ranked(false)),
    }
}

fn default_name(object: &Value, key: &str) -> Option<String> {
    object
        .get("metadata")?
        .as_array()?
        .iter()
        .find(|entry| entry["key"] == key)?["value"]["name"]
        .as_str()
        .map(str::to_string)
}

/// The running `pw-dump -m` and its thread, shared so [`Watch::stop`] can end the child at once
/// instead of leaving it to die of SIGPIPE at its next write.
#[derive(Default)]
struct Pump {
    child: Mutex<Option<Child>>,
    stopped: AtomicBool,
}

pub struct Watch {
    rx: Receiver<Snapshot>,
    pump: Arc<Pump>,
}

impl Watch {
    pub fn try_recv(&self) -> Result<Snapshot, TryRecvError> {
        self.rx.try_recv()
    }

    /// Kill the running `pw-dump` and keep the thread from starting another; the thread reaps it and ends.
    pub fn stop(&self) {
        self.pump.stopped.store(true, Ordering::SeqCst);
        if let Some(c) = self.pump.child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop();
    }
}

/// What one `pw-dump` run reports, for a window that sizes itself before [`watch`] first delivers.
/// Empty when PipeWire or `pw-dump` is not there.
pub fn snapshot_now() -> Snapshot {
    let Ok(out) = Command::new("pw-dump").stderr(Stdio::null()).output() else {
        return Snapshot::default();
    };
    let mut store = Store::new();
    for array in serde_json::Deserializer::from_slice(&out.stdout)
        .into_iter::<Value>()
        .map_while(Result::ok)
    {
        store.apply(&array);
    }
    store.snapshot()
}

/// Run `pw-dump -m` on a thread, applying every array it prints and sending a fresh [`Snapshot`] down
/// the channel, then waking the event loop.
///
/// If `pw-dump` exits (PipeWire restarting) the thread restarts it after a pause doubling from 2 s to a
/// minute while runs die without output, reset once one delivers. A missing `pw-dump` ends the thread
/// after one empty snapshot (PipeWire is optional). The thread also ends when the receiver is dropped or
/// [`Watch::stop`] runs (which dropping the [`Watch`] does); a `pw-dump` orphaned by thread death gets SIGKILL.
pub fn watch(waker: impl Fn() + Send + 'static) -> Watch {
    watch_program("pw-dump", waker)
}

fn watch_program(program: &'static str, waker: impl Fn() + Send + 'static) -> Watch {
    let (tx, rx) = mpsc::channel();
    let pump_state = Arc::new(Pump::default());
    let shared = pump_state.clone();
    std::thread::spawn(move || {
        let mut pause = RETRY_MIN;
        while !shared.stopped.load(Ordering::SeqCst) {
            match pump(program, &tx, &waker, &shared) {
                Run::Gone => break,
                Run::Missing => {
                    warn!("{program} is not installed; no audio devices");
                    let _ = tx.send(Snapshot::default());
                    waker();
                    break;
                }
                Run::Ended { delivered } => {
                    pause = next_pause(pause, delivered);
                    std::thread::sleep(pause);
                }
            }
        }
    });
    Watch {
        rx,
        pump: pump_state,
    }
}

const RETRY_MIN: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(60);

/// The pause before the next `pw-dump`: the minimum after a run that delivered, else doubled up to a minute.
fn next_pause(pause: Duration, delivered: bool) -> Duration {
    if delivered {
        RETRY_MIN
    } else {
        (pause * 2).min(RETRY_MAX)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Run {
    Gone,
    Missing,
    /// It exited or could not be started; `delivered` says whether it printed an array first.
    Ended {
        delivered: bool,
    },
}

/// One `program -m` run. The child sits in `shared` while it runs so `stop` can kill it.
fn pump(program: &str, tx: &Sender<Snapshot>, waker: &dyn Fn(), shared: &Pump) -> Run {
    let mut command = Command::new(program);
    // SAFETY: the closure runs in the forked child before exec and only
    // calls prctl, which is async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = match command
        .arg("-m")
        .stdout(Stdio::piped())
        // Its complaints when PipeWire is down would land in the session log on every retry.
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Run::Missing,
        Err(err) => {
            warn!("could not run {program}: {err}");
            return Run::Ended { delivered: false };
        }
    };
    let Some(stdout) = child.stdout.take() else {
        return Run::Ended { delivered: false };
    };
    *shared.child.lock().unwrap() = Some(child);
    if shared.stopped.load(Ordering::SeqCst)
        && let Some(c) = shared.child.lock().unwrap().as_mut()
    {
        let _ = c.kill();
    }

    let mut store = Store::new();
    let mut last: Option<Snapshot> = None;
    let mut run = Run::Ended { delivered: false };
    for array in
        serde_json::Deserializer::from_reader(std::io::BufReader::new(stdout)).into_iter::<Value>()
    {
        match array {
            Ok(array) => {
                store.apply(&array);
                run = Run::Ended { delivered: true };
                // Most changes touch objects the snapshot never shows; wake the window only when what it shows moved.
                let snap = store.snapshot();
                if last.as_ref() == Some(&snap) {
                    continue;
                }
                if tx.send(snap.clone()).is_err() {
                    run = Run::Gone;
                    break;
                }
                last = Some(snap);
                waker();
            }
            Err(err) => {
                warn!("{program} output did not parse: {err}");
                break;
            }
        }
    }

    if let Some(mut child) = shared.child.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    run
}

const WPCTL: &str = "wpctl";
const PW_CLI: &str = "pw-cli";

/// Each setter queues its write for the [`Writer`] thread and returns whether its program is installed.
pub fn set_default(id: u32) -> bool {
    set_default_then(id, |_| {})
}

/// [`set_default`], then `done` on the writer thread with whether the write took: `false` when `wpctl`
/// exits with an error or cannot start. `done` is not called when the write is not queued, nor for one a
/// newer write to the same node replaces.
pub fn set_default_then(id: u32, done: impl FnOnce(bool) + Send + 'static) -> bool {
    writer().send_then(
        Key::Default(id),
        WPCTL,
        vec!["set-default".into(), id.to_string()],
        Box::new(done),
    )
}

/// Set node `id` to volume `v` (linear 0..1, as [`Device::volume`]), each channel keeping its share of
/// the loudest as `snap` has them, so a balance survives.
pub fn set_volume(snap: &Snapshot, id: u32, v: f64) -> bool {
    match snap.node(id) {
        Some((ch, route)) if !ch.levels.is_empty() => write_levels(id, route, &ch.with_volume(v)),
        _ => set_flat(id, v),
    }
}

/// Set each channel of node `id` to its level in `levels` (linear, parallel to [`Channels::levels`]).
pub fn set_levels(snap: &Snapshot, id: u32, levels: &[f64]) -> bool {
    match snap.node(id) {
        Some((ch, route)) if ch.levels.len() == levels.len() && !levels.is_empty() => {
            write_levels(id, route, levels)
        }
        _ => set_flat(id, levels.iter().copied().fold(0.0, f64::max)),
    }
}

/// Every channel of `id` at `v` through `wpctl`, for a node whose channels are not known.
fn set_flat(id: u32, v: f64) -> bool {
    writer().send(
        Key::Volume(id),
        WPCTL,
        vec!["set-volume".into(), id.to_string(), format!("{v:.2}")],
    )
}

/// The channel volumes through `pw-cli`: on the card's route when the node has one, else on the node.
/// Without `pw-cli` every channel gets the loudest level through `wpctl`.
fn write_levels(id: u32, route: Option<Route>, levels: &[f64]) -> bool {
    if !installed(PW_CLI) {
        return set_flat(id, levels.iter().copied().fold(0.0, f64::max));
    }
    writer().send(Key::Volume(id), PW_CLI, levels_args(id, route, levels))
}

/// `pw-cli`'s arguments for [`write_levels`]; PipeWire's channel volumes are cubes of the linear levels.
fn levels_args(id: u32, route: Option<Route>, levels: &[f64]) -> Vec<String> {
    let cubes: Vec<String> = levels
        .iter()
        .map(|l| format!("{:.6}", l.max(0.0).powi(3)))
        .collect();
    let props = format!("{{ \"channelVolumes\": [ {} ] }}", cubes.join(", "));
    match route {
        Some(r) => vec![
            "set-param".into(),
            r.device.to_string(),
            "Route".into(),
            format!(
                "{{ \"index\": {}, \"device\": {}, \"props\": {props}, \"save\": true }}",
                r.index, r.card_device
            ),
        ],
        None => vec!["set-param".into(), id.to_string(), "Props".into(), props],
    }
}

pub fn set_profile(card: u32, index: u32) -> bool {
    writer().send(
        Key::Profile(card),
        WPCTL,
        vec!["set-profile".into(), card.to_string(), index.to_string()],
    )
}

pub fn set_mute(id: u32, on: bool) -> bool {
    set_mute_then(id, on, |_| {})
}

pub fn set_mute_then(id: u32, on: bool, done: impl FnOnce(bool) + Send + 'static) -> bool {
    let flag = if on { "1" } else { "0" };
    writer().send_then(
        Key::Mute(id),
        WPCTL,
        vec!["set-mute".into(), id.to_string(), flag.into()],
        Box::new(done),
    )
}

/// What a write sets; a write replaces a pending one with the same key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Default(u32),
    Volume(u32),
    Mute(u32),
    Profile(u32),
}

type Done = Box<dyn FnOnce(bool) + Send>;

struct Job {
    key: Key,
    program: &'static str,
    args: Vec<String>,
    done: Option<Done>,
}

#[derive(Default)]
struct Queue {
    pending: Mutex<Vec<Job>>,
    ready: Condvar,
}

/// Runs each queued write's program one at a time, waiting for each to exit: two in flight would race
/// and the older value could land last.
struct Writer {
    queue: Arc<Queue>,
}

impl Writer {
    fn new() -> Writer {
        let queue = Arc::new(Queue::default());
        let shared = queue.clone();
        std::thread::spawn(move || {
            loop {
                let Job {
                    program,
                    args,
                    done,
                    ..
                } = {
                    let mut pending = shared.pending.lock().unwrap();
                    while pending.is_empty() {
                        pending = shared.ready.wait(pending).unwrap();
                    }
                    pending.remove(0)
                };
                let took = match Command::new(program)
                    .args(&args)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                {
                    Ok(status) if !status.success() => {
                        warn!("{program} {} exited with {status}", args.join(" "));
                        false
                    }
                    Ok(_) => true,
                    Err(err) => {
                        warn!("could not run {program} {}: {err}", args.join(" "));
                        false
                    }
                };
                if let Some(done) = done {
                    done(took);
                }
            }
        });
        Writer { queue }
    }

    /// Queue a run of `program` behind the pending ones, dropping a pending write with the same key.
    /// Returns whether `program` is installed.
    fn send(&self, key: Key, program: &'static str, args: Vec<String>) -> bool {
        self.queue(Job {
            key,
            program,
            args,
            done: None,
        })
    }

    fn send_then(&self, key: Key, program: &'static str, args: Vec<String>, done: Done) -> bool {
        self.queue(Job {
            key,
            program,
            args,
            done: Some(done),
        })
    }

    fn queue(&self, job: Job) -> bool {
        if !installed(job.program) {
            warn!(
                "{} is not installed; {} not written",
                job.program,
                job.args.join(" ")
            );
            return false;
        }
        let mut pending = self.queue.pending.lock().unwrap();
        pending.retain(|j| j.key != job.key);
        pending.push(job);
        self.queue.ready.notify_one();
        true
    }
}

fn writer() -> &'static Writer {
    static WRITER: OnceLock<Writer> = OnceLock::new();
    WRITER.get_or_init(Writer::new)
}

fn installed(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let runnable = |p: &std::path::Path| {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        return runnable(std::path::Path::new(program));
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| runnable(&dir.join(program))))
}

pub fn icon_for(volume: f64, mute: bool) -> &'static str {
    if mute {
        "audio-volume-muted"
    } else if volume < 0.34 {
        "audio-volume-low"
    } else if volume < 0.67 {
        "audio-volume-medium"
    } else {
        "audio-volume-high"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = include_str!("../tests/fixtures/pw-dump.json");

    fn json(s: &str) -> Value {
        serde_json::from_str(s).expect("test json parses")
    }

    /// An executable script at `/tmp/repriseaudio-NAME-PID` running `body`. A child shell writes it: a
    /// write handle opened here would leak into a child a parallel test forks, failing with "Text file busy".
    fn script(name: &str, body: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("repriseaudio-{name}-{}", std::process::id()));
        let ok = Command::new("sh")
            .arg("-c")
            .arg(r#"printf '#!/bin/sh\n%s\n' "$1" > "$2" && chmod 755 "$2""#)
            .arg("sh")
            .arg(body)
            .arg(&path)
            .status()
            .expect("sh runs")
            .success();
        assert!(ok, "the script is written");
        path
    }

    /// A missing binary is reported as such so `watch` can stop; a program that exits without output is an ordinary end.
    #[test]
    fn pump_tells_a_missing_binary_from_an_exit() {
        let (tx, _rx) = mpsc::channel();
        let waker = || {};
        let shared = Pump::default();
        assert_eq!(
            pump("reprise-no-such-pw-dump", &tx, &waker, &shared),
            Run::Missing
        );
        assert_eq!(
            pump("true", &tx, &waker, &shared),
            Run::Ended { delivered: false }
        );
    }

    /// `stop` ends a child that would otherwise run on: the pump reaps it and starts no other.
    #[test]
    fn stop_kills_the_running_child() {
        let script = script("dump", "exec sleep 30");
        let path: &'static str = Box::leak(script.to_string_lossy().into_owned().into_boxed_str());
        let w = watch_program(path, || {});
        let started = std::time::Instant::now();
        while w.pump.child.lock().unwrap().is_none() {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the child never started"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        w.stop();
        let stopped = std::time::Instant::now();
        while w.pump.child.lock().unwrap().is_some() {
            assert!(
                stopped.elapsed() < Duration::from_secs(5),
                "the child was not reaped"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(50));
        assert!(w.pump.child.lock().unwrap().is_none(), "no second run");
        let _ = std::fs::remove_file(&script);
    }

    /// Writes run one at a time in order, and a queued write replaces an older pending write of its key.
    #[test]
    fn a_write_tells_its_caller_whether_it_took() {
        let w = Writer::new();
        let (tx, rx) = mpsc::channel();
        for (id, program) in [(1, "false"), (2, "true")] {
            let tx = tx.clone();
            assert!(w.send_then(
                Key::Default(id),
                program,
                Vec::new(),
                Box::new(move |took| tx.send((id, took)).unwrap()),
            ));
        }
        let heard: Vec<_> = (0..2)
            .map(|_| {
                rx.recv_timeout(Duration::from_secs(5))
                    .expect("a write ran")
            })
            .collect();
        assert_eq!(heard, [(1, false), (2, true)], "a refused write says so");
    }

    #[test]
    fn writes_run_in_order_and_the_latest_value_wins() {
        let log = std::env::temp_dir().join(format!("repriseaudio-wlog-{}", std::process::id()));
        let _ = std::fs::remove_file(&log);
        let script = script(
            "wpctl",
            &format!(r#"echo "$*" >> {}; sleep 0.3"#, log.display()),
        );
        let path: &'static str = Box::leak(script.to_string_lossy().into_owned().into_boxed_str());
        let w = Writer::new();
        let lines = || -> Vec<String> {
            std::fs::read_to_string(&log)
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect()
        };
        let wait_for = |n: usize| {
            let started = std::time::Instant::now();
            while lines().len() < n {
                assert!(
                    started.elapsed() < Duration::from_secs(5),
                    "{n} writes ran, got {:?}",
                    lines()
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        let volume = |v: &str| vec!["set-volume".to_string(), "51".into(), v.into()];

        assert!(w.send(Key::Volume(51), path, volume("0.55")));
        wait_for(1);
        assert!(w.send(Key::Volume(51), path, volume("0.60")));
        assert!(w.send(
            Key::Mute(51),
            path,
            vec!["set-mute".into(), "51".into(), "1".into()]
        ));
        assert!(w.send(Key::Volume(51), path, volume("0.65")));
        wait_for(3);
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(
            lines(),
            ["set-volume 51 0.55", "set-mute 51 1", "set-volume 51 0.65"]
        );
        let _ = std::fs::remove_file(&script);
        let _ = std::fs::remove_file(&log);
    }

    #[test]
    fn a_missing_writer_program_is_reported() {
        let w = Writer::new();
        assert!(!w.send(
            Key::Default(1),
            "reprise-no-such-wpctl",
            vec!["set-default".into(), "1".into()]
        ));
    }

    #[test]
    fn dropping_the_watch_kills_the_child() {
        let script = script("dropped", "exec sleep 30");
        let path: &'static str = Box::leak(script.to_string_lossy().into_owned().into_boxed_str());
        let w = watch_program(path, || {});
        let started = std::time::Instant::now();
        let pid = loop {
            if let Some(c) = w.pump.child.lock().unwrap().as_ref() {
                break c.id();
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the child never started"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        drop(w);
        let proc = std::path::PathBuf::from(format!("/proc/{pid}"));
        let dropped = std::time::Instant::now();
        while proc.exists() {
            assert!(
                dropped.elapsed() < Duration::from_secs(5),
                "the child was not killed and reaped"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = std::fs::remove_file(&script);
    }

    #[test]
    fn retry_pause_doubles_to_a_minute_and_resets_on_output() {
        let mut p = RETRY_MIN;
        let mut seen = vec![];
        for _ in 0..7 {
            p = next_pause(p, false);
            seen.push(p.as_secs());
        }
        assert_eq!(seen, [4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(next_pause(p, true), RETRY_MIN);
    }

    fn loaded() -> Store {
        let mut store = Store::new();
        store.apply(&json(DUMP));
        store
    }

    #[test]
    fn full_dump_yields_the_dummy_sink_and_its_default() {
        let snap = loaded().snapshot();

        assert_eq!(snap.sinks.len(), 1, "one sink in the fixture");
        let sink = &snap.sinks[0];
        assert_eq!(sink.id, 51);
        assert_eq!(sink.name, "auto_null");
        assert_eq!(sink.label, "Dummy Output");
        assert_eq!(sink.volume, 1.0);
        assert!(!sink.mute);

        assert!(snap.sources.is_empty(), "the fixture has no capture node");
        assert_eq!(snap.default_sink.as_deref(), Some("auto_null"));
        assert_eq!(snap.default_source, None);
    }

    /// Only nodes, devices and metadata are kept; the rest of the dump is never stored.
    #[test]
    fn only_nodes_devices_and_metadata_are_stored() {
        let store = loaded();
        assert_eq!(
            store.objects.len(),
            5 + 4,
            "the fixture's nodes and metadata"
        );
        let mut store = loaded();
        store.apply(&json(
            r#"[{"id": 900, "type": "PipeWire:Interface:Client", "info": {}}]"#,
        ));
        assert_eq!(store.objects.len(), 9, "a client arriving is not kept");
    }

    #[test]
    fn pump_sends_a_snapshot_only_when_it_changed() {
        let script = script("same", r"printf '[]\n[]\n'");
        let (tx, rx) = mpsc::channel();
        let run = pump(script.to_str().unwrap(), &tx, &|| {}, &Pump::default());
        assert_eq!(run, Run::Ended { delivered: true });
        assert_eq!(rx.try_iter().count(), 1);
        let _ = std::fs::remove_file(&script);
    }

    #[test]
    fn a_null_info_removes_the_object() {
        let mut store = loaded();
        store.apply(&json(r#"[{"id": 51, "info": null}]"#));

        let snap = store.snapshot();
        assert!(snap.sinks.is_empty(), "id 51 is gone");
        assert_eq!(snap.default_sink.as_deref(), Some("auto_null"));
    }

    #[test]
    fn a_params_only_update_keeps_the_props() {
        let mut store = loaded();
        store.apply(&json(
            r#"[{
                "id": 51,
                "type": "PipeWire:Interface:Node",
                "info": {
                    "change-mask": ["params"],
                    "params": {
                        "Props": [{ "channelVolumes": [0.262144, 0.262144], "mute": true }]
                    }
                }
            }]"#,
        ));

        let snap = store.snapshot();
        let sink = &snap.sinks[0];
        assert_eq!(sink.name, "auto_null");
        assert_eq!(sink.label, "Dummy Output");
        assert!((sink.volume - 0.64).abs() < 1e-9, "volume {}", sink.volume);
        assert!(sink.mute);
    }

    #[test]
    fn a_props_only_update_keeps_the_volumes() {
        let mut store = loaded();
        store.apply(&json(
            r#"[{
                "id": 51,
                "type": "PipeWire:Interface:Node",
                "info": {
                    "change-mask": ["props"],
                    "props": { "node.description": "Renamed Output" }
                }
            }]"#,
        ));

        let snap = store.snapshot();
        let sink = &snap.sinks[0];
        assert_eq!(sink.label, "Renamed Output");
        assert_eq!(sink.name, "auto_null", "media.class and node.name survive");
        assert_eq!(sink.volume, 1.0, "the params the update omitted survive");
    }

    #[test]
    fn metadata_updates_move_the_defaults() {
        let mut store = loaded();
        store.apply(&json(
            r#"[{
                "id": 39,
                "type": "PipeWire:Interface:Metadata",
                "props": { "metadata.name": "default" },
                "metadata": [
                    { "subject": 0, "key": "default.audio.sink", "value": { "name": "other_sink" } },
                    { "subject": 0, "key": "default.audio.source", "value": { "name": "some_mic" } }
                ]
            }]"#,
        ));

        let snap = store.snapshot();
        assert_eq!(snap.default_sink.as_deref(), Some("other_sink"));
        assert_eq!(snap.default_source.as_deref(), Some("some_mic"));
    }

    /// `pw-dump -m` reports a metadata change with only the changed keys: the rest stands, a null value removes one.
    #[test]
    fn a_metadata_update_keeps_the_keys_it_does_not_name() {
        let mut store = loaded();
        store.apply(&json(
            r#"[{
                "id": 39,
                "type": "PipeWire:Interface:Metadata",
                "metadata": [
                    { "subject": 0, "key": "default.audio.source", "value": { "name": "some_mic" } }
                ]
            }]"#,
        ));
        let snap = store.snapshot();
        assert_eq!(snap.default_sink.as_deref(), Some("auto_null"));
        assert_eq!(snap.default_source.as_deref(), Some("some_mic"));

        store.apply(&json(
            r#"[{
                "id": 39,
                "type": "PipeWire:Interface:Metadata",
                "metadata": [ { "subject": 0, "key": "default.audio.source", "value": null } ]
            }]"#,
        ));
        let snap = store.snapshot();
        assert_eq!(snap.default_source, None, "a null value removes the key");
        assert_eq!(snap.default_sink.as_deref(), Some("auto_null"));
    }

    #[test]
    fn volume_is_the_cube_root_of_the_loudest_channel() {
        let mut store = Store::new();
        store.apply(&json(
            r#"[{
                "id": 7,
                "type": "PipeWire:Interface:Node",
                "info": {
                    "props": { "media.class": "Audio/Source", "node.name": "mic" },
                    "params": {
                        "Props": [{ "channelVolumes": [0.125, 0.262144], "mute": false }]
                    }
                }
            }]"#,
        ));

        let src = &store.snapshot().sources[0];
        assert_eq!(src.label, "mic", "node.name is the last resort label");
        assert!((src.volume - 0.64).abs() < 1e-9, "volume {}", src.volume);
    }

    #[test]
    fn a_stream_with_no_channels_yet_reports_its_overall_volume() {
        let mut store = Store::new();
        store.apply(&json(&format!(
            "[{}]",
            r#"{ "id": 90, "type": "PipeWire:Interface:Node", "info": {
                "props": { "media.class": "Stream/Output/Audio", "application.name": "Radio" },
                "params": { "Props": [{ "volume": 0.512, "channelVolumes": [], "channelMap": [] }] }
            } }"#
        )));
        let app = &store.snapshot().apps[0];
        assert!((app.volume - 0.8).abs() < 1e-9, "volume {}", app.volume);
        assert!(!app.streams[0].channels.stereo());
    }

    #[test]
    fn a_node_reports_its_channels_by_position() {
        let mut store = Store::new();
        store.apply(&json(
            r#"[{
                "id": 7,
                "type": "PipeWire:Interface:Node",
                "info": {
                    "props": { "media.class": "Audio/Sink", "node.name": "out" },
                    "params": {
                        "Props": [{ "channelVolumes": [0.512, 0.064],
                                    "channelMap": ["FL", "FR"], "mute": false }]
                    }
                }
            }, {
                "id": 8,
                "type": "PipeWire:Interface:Node",
                "info": {
                    "props": { "media.class": "Audio/Sink", "node.name": "odd" },
                    "params": {
                        "Props": [{ "channelVolumes": [0.512, 0.064],
                                    "channelMap": ["MONO"], "mute": false }]
                    }
                }
            }]"#,
        ));
        let snap = store.snapshot();
        let out = &snap.sinks[0].channels;
        assert_eq!(out.map, ["FL", "FR"]);
        assert!((out.levels[0] - 0.8).abs() < 1e-9 && (out.levels[1] - 0.4).abs() < 1e-9);
        assert!(
            (out.balance() + 0.5).abs() < 1e-9,
            "the right is half the left"
        );
        assert!(
            snap.sinks[1].channels.map.is_empty(),
            "a map short of the channels is dropped"
        );
        assert_eq!(snap.sinks[0].route, None, "a virtual sink has no route");
    }

    #[test]
    fn a_node_turned_down_to_nothing_keeps_its_balance() {
        let node = |left: f64, right: f64| {
            json(&format!(
                r#"[{{
                    "id": 7,
                    "type": "PipeWire:Interface:Node",
                    "info": {{
                        "props": {{ "media.class": "Audio/Sink", "node.name": "out" }},
                        "params": {{
                            "Props": [{{ "channelVolumes": [{left}, {right}],
                                        "channelMap": ["FL", "FR"], "mute": false }}]
                        }}
                    }}
                }}]"#
            ))
        };
        let mut store = Store::new();
        store.apply(&node(0.512, 0.064));
        store.apply(&node(0.0, 0.0));
        let snap = store.snapshot();
        let ch = &snap.sinks[0].channels;
        assert_eq!(ch.volume(), 0.0);
        assert!((ch.balance() + 0.5).abs() < 1e-9, "the balance it had");
        let up = ch.with_volume(0.6);
        assert!(
            (up[0] - 0.6).abs() < 1e-9 && (up[1] - 0.3).abs() < 1e-9,
            "turned up, the right is half the left again: {up:?}"
        );
    }

    /// A volume write goes to the node's Props with each channel's share of the loudest kept, as cubes.
    #[test]
    fn a_volume_write_keeps_the_balance() {
        let ch = Channels {
            map: vec!["FL".into(), "FR".into()],
            levels: vec![0.8, 0.4],
            shares: Vec::new(),
        };
        assert_eq!(
            levels_args(51, None, &ch.with_volume(0.5)),
            [
                "set-param",
                "51",
                "Props",
                r#"{ "channelVolumes": [ 0.125000, 0.015625 ] }"#
            ]
        );
    }

    #[test]
    fn a_cards_sink_writes_its_route() {
        let mut store = Store::new();
        store.apply(&json(
            r#"[{
                "id": 54,
                "type": "PipeWire:Interface:Device",
                "info": {
                    "props": { "device.name": "alsa_card.pci", "media.class": "Audio/Device" },
                    "params": {
                        "Route": [
                            { "index": 3, "direction": "Input", "device": 2 },
                            { "index": 5, "direction": "Output", "device": 1 }
                        ]
                    }
                }
            }, {
                "id": 65,
                "type": "PipeWire:Interface:Node",
                "info": {
                    "props": { "media.class": "Audio/Sink", "node.name": "speaker",
                               "device.id": 54, "card.profile.device": 1 },
                    "params": { "Props": [{ "channelVolumes": [1.0, 1.0],
                                            "channelMap": ["FL", "FR"] }] }
                }
            }]"#,
        ));
        let snap = store.snapshot();
        let route = snap.sinks[0].route;
        assert_eq!(
            route,
            Some(Route {
                device: 54,
                index: 5,
                card_device: 1
            })
        );
        assert_eq!(
            levels_args(65, route, &[0.5, 1.0]),
            [
                "set-param",
                "54",
                "Route",
                r#"{ "index": 5, "device": 1, "props": { "channelVolumes": [ 0.125000, 1.000000 ] }, "save": true }"#
            ]
        );
        let (ch, r) = snap.node(65).expect("the sink is found by id");
        assert_eq!((ch.levels.len(), r), (2, route));
    }

    const CARD: &str = r#"[{
        "id": 54,
        "type": "PipeWire:Interface:Device",
        "info": {
            "props": { "device.name": "alsa_card.pci-0000_07_00.6",
                       "device.description": "Built-in Audio",
                       "media.class": "Audio/Device" },
            "params": {
                "EnumProfile": [
                    { "index": 0, "name": "off", "priority": 0, "available": "yes", "classes": [0] },
                    { "index": 1, "name": "HiFi (Headphones, Mic1)", "priority": 8500, "available": "no",
                      "classes": [2, ["Audio/Source", 1, "card.profile.devices", [1]],
                                     ["Audio/Sink", 1, "card.profile.devices", [3]]] },
                    { "index": 2, "name": "HiFi (Mic1, Speaker)", "priority": 8400, "available": "yes",
                      "classes": [2, ["Audio/Source", 1, "card.profile.devices", [1]],
                                     ["Audio/Sink", 1, "card.profile.devices", [0]]] },
                    { "index": 3, "name": "pro-audio", "priority": 1, "available": "unknown",
                      "classes": [1, ["Audio/Sink", 1, "card.profile.devices", [4]]] }
                ],
                "Profile": [{ "index": 2, "name": "HiFi (Mic1, Speaker)" }]
            }
        }
    }, {
        "id": 65,
        "type": "PipeWire:Interface:Node",
        "info": {
            "props": { "media.class": "Audio/Sink", "node.name": "speaker",
                       "node.description": "Speaker", "device.id": 54 },
            "params": { "Props": [{ "channelVolumes": [1.0], "mute": false }] }
        }
    }]"#;

    #[test]
    fn a_card_reports_its_state_and_its_best_profile() {
        let mut store = Store::new();
        store.apply(&json(CARD));
        let snap = store.snapshot();
        assert_eq!(snap.cards.len(), 1);
        let c = &snap.cards[0];
        assert_eq!(c.id, 54);
        assert_eq!(c.label, "Built-in Audio");
        assert!(!c.off);
        assert!(c.sinks && c.sources);
        assert_eq!(c.off_index, Some(0));
        assert_eq!(c.best, Some(2));
        assert_eq!(snap.sinks[0].card, Some(54), "device.id names the card");
    }

    #[test]
    fn a_card_on_its_off_profile_is_off() {
        let mut store = Store::new();
        store.apply(&json(CARD));
        store.apply(&json(
            r#"[{ "id": 54, "info": { "params": { "Profile": [{ "index": 0, "name": "off" }] } } }]"#,
        ));
        assert!(store.snapshot().cards[0].off);
    }

    #[test]
    fn with_nothing_available_the_highest_priority_profile_is_best() {
        let mut store = Store::new();
        let none_available = CARD
            .replace(
                r#""priority": 8400, "available": "yes""#,
                r#""priority": 8400, "available": "no""#,
            )
            .replace(r#""available": "unknown""#, r#""available": "no""#);
        store.apply(&json(&none_available));
        assert_eq!(store.snapshot().cards[0].best, Some(1));
    }

    #[test]
    fn a_node_without_a_device_has_no_card() {
        assert_eq!(loaded().snapshot().sinks[0].card, None);
    }

    fn stream(id: u32, props: &str, volumes: &str, mute: bool) -> String {
        format!(
            r#"{{ "id": {id}, "type": "PipeWire:Interface:Node", "info": {{
                "props": {{ "media.class": "Stream/Output/Audio", {props} }},
                "params": {{ "Props": [{{ "channelVolumes": {volumes}, "mute": {mute} }}] }}
            }} }}"#
        )
    }

    #[test]
    fn playback_streams_group_into_apps() {
        let mut store = loaded();
        let firefox = r#""application.name": "Firefox", "application.process.binary": "firefox",
                         "application.icon_name": "firefox", "node.name": "Firefox""#;
        store.apply(&json(&format!(
            "[{}, {}, {}, {}]",
            stream(90, firefox, "[0.125, 0.125]", true),
            stream(91, firefox, "[0.729]", false),
            stream(
                92,
                r#""application.name": "mpv", "application.process.binary": "mpv",
                   "pipewire.access.portal.app_id": "io.mpv.Mpv""#,
                "[1.0]",
                true
            ),
            stream(
                93,
                r#""application.name": "Peak", "stream.monitor": true"#,
                "[1.0]",
                false
            ),
        )));
        let apps = store.snapshot().apps;
        assert_eq!(apps.len(), 2, "the peak meter is left out: {apps:?}");
        let ff = &apps[0];
        assert_eq!(ff.key, "firefox/firefox");
        assert_eq!(ff.label, "Firefox");
        assert_eq!(ff.ids(), [90, 91]);
        assert!((ff.volume - 0.9).abs() < 1e-9, "the loudest: {}", ff.volume);
        assert!(!ff.mute, "one stream still plays");
        assert_eq!(ff.icon_hints, ["firefox"], "hints are not repeated");
        let mpv = &apps[1];
        assert_eq!(mpv.key, "io.mpv.mpv", "the sandbox's id comes first");
        assert_eq!(mpv.icon_hints, ["io.mpv.Mpv", "mpv"]);
        assert_eq!(mpv.binary.as_deref(), Some("mpv"));
        assert!(mpv.mute);
    }

    #[test]
    fn programs_sharing_an_executable_stay_apart() {
        let mut store = Store::new();
        let electron = |name: &str| {
            format!(r#""application.name": "{name}", "application.process.binary": "electron34""#)
        };
        store.apply(&json(&format!(
            "[{}, {}, {}]",
            stream(90, &electron("Code - OSS"), "[1.0]", false),
            stream(91, &electron("Signal"), "[1.0]", false),
            stream(92, &electron("Signal"), "[1.0]", false)
        )));
        let apps = store.snapshot().apps;
        let rows: Vec<(&str, Vec<u32>)> =
            apps.iter().map(|a| (a.label.as_str(), a.ids())).collect();
        assert_eq!(rows, [("Code - OSS", vec![90]), ("Signal", vec![91, 92])]);
    }

    #[test]
    fn a_stream_going_away_leaves_its_app() {
        let mut store = loaded();
        let props = r#""application.name": "Game""#;
        store.apply(&json(&format!(
            "[{}, {}]",
            stream(90, props, "[1.0]", false),
            stream(91, props, "[1.0]", false)
        )));
        store.apply(&json(r#"[{"id": 90, "info": null}]"#));
        assert_eq!(store.snapshot().apps[0].ids(), [91]);
        store.apply(&json(r#"[{"id": 91, "info": null}]"#));
        assert!(store.snapshot().apps.is_empty());
    }

    #[test]
    fn apps_sort_by_label() {
        let mut store = Store::new();
        store.apply(&json(&format!(
            "[{}, {}]",
            stream(90, r#""application.name": "zoom""#, "[1.0]", false),
            stream(91, r#""application.name": "Audacious""#, "[1.0]", false)
        )));
        let labels: Vec<String> = store.snapshot().apps.into_iter().map(|a| a.label).collect();
        assert_eq!(labels, ["Audacious", "zoom"]);
    }

    #[test]
    fn icons_follow_the_volume_thresholds() {
        assert_eq!(icon_for(0.9, true), "audio-volume-muted");
        assert_eq!(icon_for(0.0, true), "audio-volume-muted");
        assert_eq!(icon_for(0.0, false), "audio-volume-low");
        assert_eq!(icon_for(0.33, false), "audio-volume-low");
        assert_eq!(icon_for(0.34, false), "audio-volume-medium");
        assert_eq!(icon_for(0.66, false), "audio-volume-medium");
        assert_eq!(icon_for(0.67, false), "audio-volume-high");
        assert_eq!(icon_for(1.0, false), "audio-volume-high");
    }
}
