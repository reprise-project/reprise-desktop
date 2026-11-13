//! The long-lived Wayland connection behind RemoteDesktop, InputCapture, GlobalShortcuts, Background and PickColor:
//! binds `reprise_input_v1` and `zxdg_output_manager_v1`, keeps the zones and the sandboxed apps with a
//! window current, and runs every request on
//! its own thread. Requests come in over a channel with a socket as the wake-up; capture and shortcuts
//! events go out on an async channel each capture or shortcuts object owns.

use crate::inputproto::{
    reprise_color_pick_v1::{self, RepriseColorPickV1},
    reprise_eis_client_v1::RepriseEisClientV1,
    reprise_input_capture_v1::{self, RepriseInputCaptureV1},
    reprise_input_v1::{self, Kind, RepriseInputV1},
    reprise_keys_v1::{KeyState, RepriseKeysV1},
    reprise_lock_screen_v1::{self, Reason, RepriseLockScreenV1},
    reprise_sandboxed_v1::{self, RepriseSandboxedV1},
    reprise_shortcuts_v1::{self, RepriseShortcutsV1},
};
use std::collections::{BTreeSet, HashMap};
use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use wayland_client::backend::WaylandError;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::xdg_output::zv1::client::{
    zxdg_output_manager_v1::ZxdgOutputManagerV1,
    zxdg_output_v1::{self, ZxdgOutputV1},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Zone {
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Barrier {
    pub id: u32,
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CaptureEvent {
    Activated {
        activation_id: u32,
        barrier_id: u32,
        x: f64,
        y: f64,
    },
    Deactivated {
        activation_id: u32,
    },
    ZonesChanged,
}

/// The `reprise_shortcuts_v1` objects the link holds at once. The compositor ends a client's whole
/// `reprise_input_v1` with `too_many` past its `globalshortcuts::MAX_PER_CLIENT`, taking RemoteDesktop
/// and InputCapture down too, so the two must stay equal (or this lower).
pub const MAX_SHORTCUTS: usize = 16;

/// The `reprise_input_capture_v1` objects the link holds at once; like `MAX_SHORTCUTS`, it must stay equal
/// to the compositor's `inputcapture::MAX_CAPTURES_PER_CLIENT` (or lower).
pub const MAX_CAPTURES: usize = 8;

/// A remote client's monitor as `set_layout` takes it: x, y, width, height and scale in percent.
pub type Monitor = (i32, i32, i32, i32, u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockEvent {
    /// The names of the screens made for lock mode, primary first.
    Screens(Vec<String>),
    Ended(WEnum<Reason>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShortcutEvent {
    Activated { id: String, time: u32 },
    Deactivated { id: String, time: u32 },
    Changed,
}

/// Whose RemoteDesktop session an input object serves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    /// Reprise's remote desktop server, for the session started without a dialog
    /// (`crate::trusted`).
    Server,
    /// An app, by the app id the frontend gave, "" when it gave none.
    App(String),
}

enum Cmd {
    ConnectSender(OwnedFd, u32, u32, Owner),
    CreateCapture(u32, async_channel::Sender<CaptureEvent>),
    ConnectReceiver(u32, OwnedFd, u32, u32),
    SetBarriers(u32, Vec<Barrier>),
    Enable(u32),
    Disable(u32),
    Release(u32, u32, Option<(f64, f64)>),
    DestroyEis(u32),
    DestroyCapture(u32),
    CreateShortcuts(u32, String, async_channel::Sender<ShortcutEvent>),
    DestroyShortcuts(u32),
    CreateKeys(u32, Owner),
    Key(u32, u32, bool),
    Keysym(u32, u32, bool),
    DestroyKeys(u32),
    PickColor(u32, async_channel::Sender<Option<u32>>),
    DestroyPick(u32),
    LockScreen(
        u32,
        Vec<u8>,
        async_channel::Sender<LockEvent>,
        Arc<AtomicBool>,
    ),
    SetLayout(u32, Vec<u8>),
    DestroyLockScreen(u32),
}

struct Shared {
    zones: Vec<Zone>,
    zone_set: u32,
    /// The compositor's last `reprise_sandboxed_v1` list; None before a v5 compositor sent one.
    sandboxed: Option<BTreeSet<String>>,
    /// The recorders the compositor started for the user (`recorder_granted`).
    grants: crate::trusted::Grants,
}

#[derive(Clone)]
pub struct Link {
    tx: Sender<Cmd>,
    wake: Arc<UnixStream>,
    shared: Arc<Mutex<Shared>>,
    next_id: Arc<AtomicU32>,
    /// Set by the thread when the connection is lost.
    dead: Arc<AtomicBool>,
    live_captures: Arc<AtomicUsize>,
    shortcuts: bool,
    live_shortcuts: Arc<AtomicUsize>,
    keys: bool,
    picks: bool,
    lock_screens: bool,
    sandboxed_changes: async_channel::Receiver<()>,
}

/// One EIS connection. Dropping it destroys the `reprise_eis_client_v1`, disconnecting the libei client.
pub struct EisHandle {
    link: Link,
    id: u32,
}

impl Drop for EisHandle {
    fn drop(&mut self) {
        self.link.send(Cmd::DestroyEis(self.id));
    }
}

/// One capture object. Dropping it destroys the `reprise_input_capture_v1` and every barrier it holds.
pub struct CaptureHandle {
    link: Link,
    id: u32,
    events: async_channel::Receiver<CaptureEvent>,
}

impl CaptureHandle {
    /// A receiver on `fd` offered only the `devices` (keyboard and pointer bits) the session holds.
    pub fn connect_receiver(&self, fd: OwnedFd, devices: u32) -> EisHandle {
        let id = self.link.alloc();
        self.link
            .send(Cmd::ConnectReceiver(self.id, fd, id, devices));
        EisHandle {
            link: self.link.clone(),
            id,
        }
    }

    pub fn set_barriers(&self, b: &[Barrier]) {
        self.link.send(Cmd::SetBarriers(self.id, b.to_vec()));
    }

    pub fn enable(&self) {
        self.link.send(Cmd::Enable(self.id));
    }

    pub fn disable(&self) {
        self.link.send(Cmd::Disable(self.id));
    }

    /// End the capture. None ends whatever is active: the frontend may call Release with no activation_id (0 on the wire).
    pub fn release(&self, activation_id: Option<u32>, pos: Option<(f64, f64)>) {
        self.link
            .send(Cmd::Release(self.id, activation_id.unwrap_or(0), pos));
    }

    pub fn events(&self) -> async_channel::Receiver<CaptureEvent> {
        self.events.clone()
    }
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        self.link.send(Cmd::DestroyCapture(self.id));
        self.link.live_captures.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One shortcuts object. Dropping it destroys the `reprise_shortcuts_v1`; the compositor lets go of the
/// app's chords once no object for that app is left.
pub struct ShortcutsHandle {
    link: Link,
    id: u32,
    events: async_channel::Receiver<ShortcutEvent>,
}

impl ShortcutsHandle {
    pub fn events(&self) -> async_channel::Receiver<ShortcutEvent> {
        self.events.clone()
    }
}

impl Drop for ShortcutsHandle {
    fn drop(&mut self) {
        // The destroy is queued ahead of any later create, so the place is free by the time it is reused.
        self.link.send(Cmd::DestroyShortcuts(self.id));
        self.link.live_shortcuts.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One session's keys. Dropping it destroys the `reprise_keys_v1`, releasing every key it holds.
pub struct KeysHandle {
    link: Link,
    id: u32,
}

impl KeysHandle {
    pub fn key(&self, code: u32, pressed: bool) {
        self.link.send(Cmd::Key(self.id, code, pressed));
    }

    pub fn keysym(&self, sym: u32, pressed: bool) {
        self.link.send(Cmd::Keysym(self.id, sym, pressed));
    }
}

impl Drop for KeysHandle {
    fn drop(&mut self) {
        self.link.send(Cmd::DestroyKeys(self.id));
    }
}

/// One colour pick. Dropping it destroys the `reprise_color_pick_v1`, closing the picker if it is
/// still open.
pub struct PickHandle {
    link: Link,
    id: u32,
    answer: async_channel::Receiver<Option<u32>>,
}

impl PickHandle {
    /// The pick as 0xRRGGBB, None for a cancel or a lost link.
    pub async fn answer(&self) -> Option<u32> {
        self.answer.recv().await.ok().flatten()
    }
}

impl Drop for PickHandle {
    fn drop(&mut self) {
        self.link.send(Cmd::DestroyPick(self.id));
    }
}

/// One session's hold on lock mode. Dropping it destroys the `reprise_lock_screen_v1`; lock mode
/// ends once no session holds one.
pub struct LockScreenHandle {
    link: Link,
    id: u32,
    events: async_channel::Receiver<LockEvent>,
    ended: Arc<AtomicBool>,
}

impl LockScreenHandle {
    /// Whether the compositor ended lock mode for it.
    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }

    pub fn set_layout(&self, layout: &[Monitor]) {
        self.link
            .send(Cmd::SetLayout(self.id, layout_bytes(layout)));
    }

    /// Closed once the object is gone: after `ended`, or with the connection.
    pub fn events(&self) -> async_channel::Receiver<LockEvent> {
        self.events.clone()
    }
}

impl Drop for LockScreenHandle {
    fn drop(&mut self) {
        self.link.send(Cmd::DestroyLockScreen(self.id));
    }
}

/// `set_layout`'s array: each monitor's five words in native byte order.
fn layout_bytes(layout: &[Monitor]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(layout.len() * 20);
    for &(x, y, w, h, scale) in layout {
        for word in [x, y, w, h] {
            bytes.extend(word.to_ne_bytes());
        }
        bytes.extend(scale.to_ne_bytes());
    }
    bytes
}

impl Link {
    pub fn start() -> Result<Link, String> {
        let conn = Connection::connect_to_env().map_err(|e| format!("wayland: {e}"))?;
        Link::start_on(conn, true)
    }

    /// With `exit_when_lost`, a lost connection ends the process so its unit restarts it with a fresh link.
    fn start_on(conn: Connection, exit_when_lost: bool) -> Result<Link, String> {
        let (globals, mut queue) =
            registry_queue_init::<State>(&conn).map_err(|e| format!("registry: {e}"))?;
        let qh = queue.handle();
        let input: RepriseInputV1 = globals
            .bind(&qh, 2..=11, ())
            .map_err(|e| format!("reprise_input_v1: {e}"))?;
        let shortcuts = input.version() >= 3;
        let keys = input.version() >= 4;
        let picks = input.version() >= 6;
        let lock_screens = input.version() >= 9;
        let xdg: ZxdgOutputManagerV1 = globals
            .bind(&qh, 3..=3, ())
            .map_err(|e| format!("xdg_output: {e}"))?;
        let shared = Arc::new(Mutex::new(Shared {
            zones: Vec::new(),
            zone_set: 1,
            sandboxed: None,
            grants: Default::default(),
        }));
        let sandboxed = (input.version() >= 5).then(|| input.watch_sandboxed(&qh, ()));
        if sandboxed.is_none() {
            log::warn!(
                "reprise_input_v1 v{} has no list of sandboxed windows: Background app states \
                 fail until the session restarts on a newer compositor",
                input.version()
            );
        }
        let (changed_tx, changed_rx) = async_channel::bounded(1);
        let mut state = State {
            input,
            xdg,
            outputs: Vec::new(),
            eis: HashMap::new(),
            captures: HashMap::new(),
            shortcuts: HashMap::new(),
            keys: HashMap::new(),
            picks: HashMap::new(),
            locks: HashMap::new(),
            _sandboxed: sandboxed,
            sandboxed_pending: BTreeSet::new(),
            sandboxed_changed: changed_tx,
            shared: shared.clone(),
            registry: globals.registry().clone(),
        };
        for g in globals
            .contents()
            .clone_list()
            .iter()
            .filter(|g| g.interface == wl_output::WlOutput::interface().name)
        {
            state.bind_output(&qh, g.name, g.version.min(4));
        }
        queue
            .roundtrip(&mut state)
            .map_err(|e| format!("roundtrip: {e}"))?;
        state.publish_zones();
        let (tx, rx) = channel::<Cmd>();
        let (r, w) = wake_pair().map_err(|e| format!("wake socket: {e}"))?;
        let link = Link {
            tx,
            wake: Arc::new(w),
            shared,
            next_id: Arc::new(AtomicU32::new(1)),
            dead: Arc::new(AtomicBool::new(false)),
            live_captures: Arc::new(AtomicUsize::new(0)),
            shortcuts,
            live_shortcuts: Arc::new(AtomicUsize::new(0)),
            keys,
            picks,
            lock_screens,
            sandboxed_changes: changed_rx,
        };
        let dead = link.dead.clone();
        std::thread::Builder::new()
            .name("reprise-portal-input".into())
            .spawn(move || {
                run(conn, queue, state, rx, r);
                dead.store(true, Ordering::Relaxed);
                if exit_when_lost {
                    log::error!("input link: connection lost; exiting for a restart");
                    std::process::exit(1);
                }
            })
            .map_err(|e| format!("thread: {e}"))?;
        Ok(link)
    }

    /// The Flatpak app ids of the sandboxed clients with a window, or None when the compositor
    /// lists none (before v5) or the connection is lost.
    pub fn sandboxed(&self) -> Option<BTreeSet<String>> {
        if self.dead.load(Ordering::Relaxed) {
            return None;
        }
        self.shared.lock().unwrap().sandboxed.clone()
    }

    /// Use up the compositor's grant for recorder `pid`, if a fresh one is there.
    pub fn take_recorder_grant(&self, pid: u32) -> bool {
        let now = std::time::Instant::now();
        self.shared.lock().unwrap().grants.take(pid, now)
    }

    /// Wakes after each list `sandboxed` takes; several lists may share one wake.
    pub fn sandboxed_changes(&self) -> async_channel::Receiver<()> {
        self.sandboxed_changes.clone()
    }

    pub fn zones(&self) -> (Vec<Zone>, u32) {
        let s = self.shared.lock().unwrap();
        (s.zones.clone(), s.zone_set)
    }

    /// A sender gets the devices in the `reprise_input_v1` mask `devices` and no others; the
    /// server's is counted as a user at the computer, an app's shows the sharing indicator.
    pub fn connect_sender(&self, fd: OwnedFd, devices: u32, owner: Owner) -> EisHandle {
        let id = self.alloc();
        self.send(Cmd::ConnectSender(fd, id, devices, owner));
        EisHandle {
            link: self.clone(),
            id,
        }
    }

    /// A capture object, or None while `MAX_CAPTURES` are live.
    pub fn create_capture(&self) -> Option<CaptureHandle> {
        self.live_captures
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < MAX_CAPTURES).then_some(n + 1)
            })
            .ok()?;
        let id = self.alloc();
        let (tx, rx) = async_channel::unbounded();
        self.send(Cmd::CreateCapture(id, tx));
        Some(CaptureHandle {
            link: self.clone(),
            id,
            events: rx,
        })
    }

    pub fn has_shortcuts(&self) -> bool {
        self.shortcuts
    }

    /// A shortcuts object for `app_id`, or None while `MAX_SHORTCUTS` are live. Without `has_shortcuts`
    /// nothing is sent and the handle's events end at once.
    pub fn create_shortcuts(&self, app_id: &str) -> Option<ShortcutsHandle> {
        self.live_shortcuts
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < MAX_SHORTCUTS).then_some(n + 1)
            })
            .ok()?;
        let id = self.alloc();
        let (tx, rx) = async_channel::unbounded();
        self.send(Cmd::CreateShortcuts(id, app_id.to_string(), tx));
        Some(ShortcutsHandle {
            link: self.clone(),
            id,
            events: rx,
        })
    }

    pub fn create_keys(&self, owner: Owner) -> Option<KeysHandle> {
        if !self.keys {
            return None;
        }
        let id = self.alloc();
        self.send(Cmd::CreateKeys(id, owner));
        Some(KeysHandle {
            link: self.clone(),
            id,
        })
    }

    /// Open the compositor's colour picker; None before v6.
    pub fn pick_color(&self) -> Option<PickHandle> {
        if !self.picks {
            return None;
        }
        let id = self.alloc();
        let (tx, rx) = async_channel::bounded(1);
        self.send(Cmd::PickColor(id, tx));
        Some(PickHandle {
            link: self.clone(),
            id,
            answer: rx,
        })
    }

    /// Ask for lock mode with `layout`; None before v9.
    pub fn lock_screen(&self, layout: &[Monitor]) -> Option<LockScreenHandle> {
        if !self.lock_screens {
            return None;
        }
        let id = self.alloc();
        let (tx, rx) = async_channel::unbounded();
        let ended = Arc::new(AtomicBool::new(false));
        self.send(Cmd::LockScreen(id, layout_bytes(layout), tx, ended.clone()));
        Some(LockScreenHandle {
            link: self.clone(),
            id,
            events: rx,
            ended,
        })
    }

    fn alloc(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
        let _ = (&*self.wake).write(&[1]);
    }
}

/// `link` while its connection lives: RemoteDesktop and InputCapture answer a lost link as a missing one.
pub fn live(link: &Option<Link>) -> Option<&Link> {
    link.as_ref().filter(|l| !l.dead.load(Ordering::Relaxed))
}

/// The two ends of an EIS connection: ours goes to the compositor through the link, the other back to the client from ConnectToEIS.
pub fn socketpair() -> zbus::fdo::Result<(OwnedFd, OwnedFd)> {
    let (ours, theirs) =
        UnixStream::pair().map_err(|e| zbus::fdo::Error::Failed(format!("socketpair: {e}")))?;
    Ok((ours.into(), theirs.into()))
}

/// The link thread's wake-up: both ends non-blocking, so a flood of commands never blocks the sender.
fn wake_pair() -> std::io::Result<(UnixStream, UnixStream)> {
    let (r, w) = UnixStream::pair()?;
    r.set_nonblocking(true)?;
    w.set_nonblocking(true)?;
    Ok((r, w))
}

struct State {
    input: RepriseInputV1,
    xdg: ZxdgOutputManagerV1,
    outputs: Vec<(wl_output::WlOutput, ZxdgOutputV1, Zone, bool)>,
    eis: HashMap<u32, RepriseEisClientV1>,
    captures: HashMap<u32, (RepriseInputCaptureV1, async_channel::Sender<CaptureEvent>)>,
    shortcuts: HashMap<u32, (RepriseShortcutsV1, async_channel::Sender<ShortcutEvent>)>,
    keys: HashMap<u32, RepriseKeysV1>,
    picks: HashMap<u32, (RepriseColorPickV1, async_channel::Sender<Option<u32>>)>,
    /// Each object with the screen names heard since its last `done`.
    locks: HashMap<u32, LockScreen>,
    /// Held so the compositor goes on sending the list.
    _sandboxed: Option<RepriseSandboxedV1>,
    sandboxed_pending: BTreeSet<String>,
    sandboxed_changed: async_channel::Sender<()>,
    shared: Arc<Mutex<Shared>>,
    registry: wl_registry::WlRegistry,
}

impl State {
    fn bind_output(&mut self, qh: &QueueHandle<State>, name: u32, version: u32) {
        let out: wl_output::WlOutput = self.registry.bind(name, version, qh, name);
        let xo = self.xdg.get_xdg_output(&out, qh, name);
        self.outputs.push((
            out,
            xo,
            Zone {
                width: 0,
                height: 0,
                x: 0,
                y: 0,
            },
            false,
        ));
    }

    fn publish_zones(&mut self) {
        let zones: Vec<Zone> = self
            .outputs
            .iter()
            .filter(|o| o.3 && o.2.width > 0)
            .map(|o| o.2.clone())
            .collect();
        let mut s = self.shared.lock().unwrap();
        if s.zones != zones {
            s.zones = zones;
            // Any change will do. 0 is skipped on wrap, so a client that never called GetZones and sends 0 cannot match.
            s.zone_set = s.zone_set.wrapping_add(1).max(1);
        }
    }

    fn handle(&mut self, qh: &QueueHandle<State>, cmd: Cmd) {
        match cmd {
            Cmd::ConnectSender(fd, id, devices, owner) => {
                let v = self.input.version();
                let c = match owner {
                    Owner::App(app) if v >= 10 => {
                        self.input
                            .connect_app_sender(fd.as_fd(), devices, app, qh, ())
                    }
                    Owner::Server if v >= 7 => (self.input).connect_eis(
                        fd.as_fd(),
                        Kind::RemoteSession,
                        None,
                        devices,
                        qh,
                        (),
                    ),
                    _ => (self.input).connect_eis(fd.as_fd(), Kind::Sender, None, devices, qh, ()),
                };
                self.eis.insert(id, c);
            }
            Cmd::CreateCapture(id, tx) => {
                let c = self.input.create_capture(qh, id);
                self.captures.insert(id, (c, tx));
            }
            Cmd::ConnectReceiver(cap, fd, id, devices) => {
                if let Some((c, _)) = self.captures.get(&cap) {
                    let e = self.input.connect_eis(
                        fd.as_fd(),
                        Kind::Receiver,
                        Some(c),
                        devices,
                        qh,
                        (),
                    );
                    self.eis.insert(id, e);
                }
            }
            Cmd::SetBarriers(cap, barriers) => {
                if let Some((c, _)) = self.captures.get(&cap) {
                    c.clear_barriers();
                    for b in barriers {
                        c.add_barrier(b.id, b.x1, b.y1, b.x2, b.y2);
                    }
                }
            }
            Cmd::Enable(cap) => {
                if let Some((c, _)) = self.captures.get(&cap) {
                    c.enable();
                }
            }
            Cmd::Disable(cap) => {
                if let Some((c, _)) = self.captures.get(&cap) {
                    c.disable();
                }
            }
            Cmd::Release(cap, id, pos) => {
                if let Some((c, _)) = self.captures.get(&cap) {
                    let (has, x, y) = match pos {
                        Some((x, y)) => (1, x, y),
                        None => (0, 0.0, 0.0),
                    };
                    c.release(id, has, x, y);
                }
            }
            Cmd::DestroyEis(id) => {
                if let Some(e) = self.eis.remove(&id) {
                    e.destroy();
                }
            }
            Cmd::DestroyCapture(id) => {
                if let Some((c, _)) = self.captures.remove(&id) {
                    c.destroy();
                }
            }
            Cmd::CreateShortcuts(id, app, tx) => {
                // A v2 compositor would answer with a protocol error and take the link down; dropping `tx` ends the handle's events instead.
                if self.input.version() < 3 {
                    return;
                }
                let s = self.input.create_shortcuts(app, qh, id);
                self.shortcuts.insert(id, (s, tx));
            }
            Cmd::DestroyShortcuts(id) => {
                if let Some((s, _)) = self.shortcuts.remove(&id) {
                    s.destroy();
                }
            }
            Cmd::CreateKeys(id, owner) => {
                let k = match owner {
                    Owner::App(app) if self.input.version() >= 10 => {
                        self.input.create_app_keys(app, qh, ())
                    }
                    _ => self.input.create_keys(qh, ()),
                };
                self.keys.insert(id, k);
            }
            Cmd::Key(id, code, pressed) => {
                if let Some(k) = self.keys.get(&id) {
                    k.key(code, key_state(pressed));
                }
            }
            Cmd::Keysym(id, sym, pressed) => {
                if let Some(k) = self.keys.get(&id) {
                    k.keysym(sym, key_state(pressed));
                }
            }
            Cmd::DestroyKeys(id) => {
                if let Some(k) = self.keys.remove(&id) {
                    k.destroy();
                }
            }
            Cmd::PickColor(id, tx) => {
                let p = self.input.pick_color(qh, id);
                self.picks.insert(id, (p, tx));
            }
            Cmd::DestroyPick(id) => {
                if let Some((p, _)) = self.picks.remove(&id) {
                    p.destroy();
                }
            }
            Cmd::LockScreen(id, layout, events, ended) => {
                let object = self.input.lock_screen(qh, id);
                object.set_layout(layout);
                let lock = LockScreen {
                    object,
                    screens: Vec::new(),
                    events,
                    ended,
                };
                self.locks.insert(id, lock);
            }
            Cmd::SetLayout(id, layout) => {
                if let Some(l) = self.locks.get(&id) {
                    l.object.set_layout(layout);
                }
            }
            Cmd::DestroyLockScreen(id) => {
                if let Some(l) = self.locks.remove(&id) {
                    l.object.destroy();
                }
            }
        }
    }
}

struct LockScreen {
    object: RepriseLockScreenV1,
    screens: Vec<String>,
    events: async_channel::Sender<LockEvent>,
    ended: Arc<AtomicBool>,
}

fn key_state(pressed: bool) -> KeyState {
    if pressed {
        KeyState::Pressed
    } else {
        KeyState::Released
    }
}

fn run(
    conn: Connection,
    mut queue: wayland_client::EventQueue<State>,
    mut state: State,
    rx: std::sync::mpsc::Receiver<Cmd>,
    wake: UnixStream,
) {
    let qh = queue.handle();
    loop {
        loop {
            match rx.try_recv() {
                Ok(cmd) => state.handle(&qh, cmd),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                // Every Link clone is gone, so nothing can ask again. Let the connection close with the thread.
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    log::info!("input link: last handle dropped, closing the connection");
                    return;
                }
            }
        }
        if let Err(e) = queue.dispatch_pending(&mut state) {
            log::warn!("input link: dispatch: {e}");
            break;
        }
        // A full socket keeps the rest of the requests for a later round; the poll below also wakes when it drains.
        let unflushed = match queue.flush() {
            Ok(()) => false,
            Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => true,
            Err(e) => {
                log::warn!("input link: flush: {e}");
                break;
            }
        };
        // None means events are already queued: dispatch them rather than block in poll.
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let mut fds = [
            libc::pollfd {
                fd: guard.connection_fd().as_raw_fd(),
                events: if unflushed {
                    libc::POLLIN | libc::POLLOUT
                } else {
                    libc::POLLIN
                },
                revents: 0,
            },
            libc::pollfd {
                fd: wake.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: `fds` holds the two pollfds the count names, and both fds stay open for the call: the
        // guard holds the connection's, `wake` is owned here.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
        if n < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            break;
        }
        if fds[1].revents & libc::POLLIN != 0 {
            let mut buf = [0u8; 64];
            while (&wake).read(&mut buf).is_ok_and(|n| n > 0) {}
        }
        // A hangup or error with nothing left to read means the fd is dead: poll would return on it immediately.
        let dead = |p: &libc::pollfd| {
            p.revents & (libc::POLLHUP | libc::POLLERR) != 0 && p.revents & libc::POLLIN == 0
        };
        if dead(&fds[0]) || dead(&fds[1]) {
            log::warn!(
                "input link: poll revents wayland {:#x} wake {:#x}",
                fds[0].revents,
                fds[1].revents
            );
            break;
        }
        if fds[0].revents & libc::POLLIN != 0 {
            // EAGAIN is not a lost connection: poll and the guard race when a command wakes us in the
            // same turn as an event (a Session close sends destroy requests while the compositor's last
            // deactivated is still in flight), and the socket is empty by the time we read. Go round again.
            match guard.read() {
                Ok(_) => {}
                Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(e) => {
                    log::warn!("input link: read: {e}");
                    break;
                }
            }
        } else {
            drop(guard);
        }
    }
    log::warn!("input link: connection lost");
    drop(conn);
}

impl Dispatch<RepriseInputV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &RepriseInputV1,
        event: reprise_input_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let reprise_input_v1::Event::RecorderGranted { pid } = event;
        log::info!("the compositor started recorder {pid} for the user");
        let now = std::time::Instant::now();
        state.shared.lock().unwrap().grants.grant(pid, now);
    }
}
delegate_noop!(State: ignore RepriseEisClientV1);
delegate_noop!(State: ignore RepriseKeysV1);
delegate_noop!(State: ignore ZxdgOutputManagerV1);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        state: &mut Self,
        _: &wl_registry::WlRegistry,
        ev: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match ev {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == wl_output::WlOutput::interface().name => {
                state.bind_output(qh, name, version.min(4));
            }
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(i) = state
                    .outputs
                    .iter()
                    .position(|o| o.0.data::<u32>() == Some(&name))
                {
                    let (o, xo, _, _) = state.outputs.remove(i);
                    xo.destroy();
                    if o.version() >= 3 {
                        o.release();
                    }
                    state.publish_zones();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for State {
    fn event(
        state: &mut Self,
        out: &wl_output::WlOutput,
        ev: wl_output::Event,
        _: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Done = ev {
            if let Some(o) = state.outputs.iter_mut().find(|o| &o.0 == out) {
                o.3 = true;
            }
            state.publish_zones();
        }
    }
}

impl Dispatch<ZxdgOutputV1, u32> for State {
    fn event(
        state: &mut Self,
        xo: &ZxdgOutputV1,
        ev: zxdg_output_v1::Event,
        _: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(o) = state.outputs.iter_mut().find(|o| &o.1 == xo) else {
            return;
        };
        match ev {
            zxdg_output_v1::Event::LogicalPosition { x, y } => {
                o.2.x = x;
                o.2.y = y;
            }
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                o.2.width = width as u32;
                o.2.height = height as u32;
            }
            _ => {}
        }
    }
}

impl Dispatch<RepriseInputCaptureV1, u32> for State {
    fn event(
        state: &mut Self,
        _: &RepriseInputCaptureV1,
        ev: reprise_input_capture_v1::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some((_, tx)) = state.captures.get(id) else {
            return;
        };
        let e = match ev {
            reprise_input_capture_v1::Event::Activated {
                activation_id,
                barrier_id,
                x,
                y,
            } => CaptureEvent::Activated {
                activation_id,
                barrier_id,
                x,
                y,
            },
            reprise_input_capture_v1::Event::Deactivated { activation_id } => {
                CaptureEvent::Deactivated { activation_id }
            }
            reprise_input_capture_v1::Event::ZonesChanged => CaptureEvent::ZonesChanged,
        };
        let _ = tx.try_send(e);
    }
}

impl Dispatch<RepriseColorPickV1, u32> for State {
    fn event(
        state: &mut Self,
        _: &RepriseColorPickV1,
        ev: reprise_color_pick_v1::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some((_, tx)) = state.picks.get(id) else {
            return;
        };
        let _ = tx.try_send(match ev {
            reprise_color_pick_v1::Event::Picked { rgb } => Some(rgb),
            reprise_color_pick_v1::Event::Cancelled => None,
        });
    }
}

impl Dispatch<RepriseLockScreenV1, u32> for State {
    fn event(
        state: &mut Self,
        _: &RepriseLockScreenV1,
        ev: reprise_lock_screen_v1::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(lock) = state.locks.get_mut(id) else {
            return;
        };
        match ev {
            reprise_lock_screen_v1::Event::Screen { name } => lock.screens.push(name),
            reprise_lock_screen_v1::Event::Done => {
                let screens = std::mem::take(&mut lock.screens);
                let _ = lock.events.try_send(LockEvent::Screens(screens));
            }
            reprise_lock_screen_v1::Event::Ended { reason } => {
                lock.ended.store(true, Ordering::Release);
                let _ = lock.events.try_send(LockEvent::Ended(reason));
                if let Some(l) = state.locks.remove(id) {
                    l.object.destroy();
                }
            }
        }
    }
}

impl Dispatch<RepriseSandboxedV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &RepriseSandboxedV1,
        ev: reprise_sandboxed_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match ev {
            reprise_sandboxed_v1::Event::App { app_id } => {
                state.sandboxed_pending.insert(app_id);
            }
            reprise_sandboxed_v1::Event::Done => {
                let now = std::mem::take(&mut state.sandboxed_pending);
                state.shared.lock().unwrap().sandboxed = Some(now);
                let _ = state.sandboxed_changed.try_send(());
            }
        }
    }
}

impl Dispatch<RepriseShortcutsV1, u32> for State {
    fn event(
        state: &mut Self,
        _: &RepriseShortcutsV1,
        ev: reprise_shortcuts_v1::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some((_, tx)) = state.shortcuts.get(id) else {
            return;
        };
        let e = match ev {
            reprise_shortcuts_v1::Event::Activated { id, time_msec } => ShortcutEvent::Activated {
                id,
                time: time_msec,
            },
            reprise_shortcuts_v1::Event::Deactivated { id, time_msec } => {
                ShortcutEvent::Deactivated {
                    id,
                    time: time_msec,
                }
            }
            reprise_shortcuts_v1::Event::Changed => ShortcutEvent::Changed,
        };
        let _ = tx.try_send(e);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::inputproto::reprise_input_v1::Device;
    use std::time::{Duration, Instant};

    pub(crate) fn send(sock: &mut UnixStream, object: u32, opcode: u32, body: &[u8]) {
        let size = (8 + body.len()) as u32;
        let mut msg = object.to_ne_bytes().to_vec();
        msg.extend((size << 16 | opcode).to_ne_bytes());
        msg.extend(body);
        sock.write_all(&msg).unwrap();
    }

    pub(crate) fn put_string(body: &mut Vec<u8>, s: &str) {
        body.extend((s.len() as u32 + 1).to_ne_bytes());
        body.extend(s.as_bytes());
        body.push(0);
        body.resize(body.len().next_multiple_of(4), 0);
    }

    fn get_string(body: &[u8]) -> String {
        let len = u32::from_ne_bytes(body[..4].try_into().unwrap()) as usize;
        String::from_utf8(body[4..4 + len - 1].to_vec()).unwrap()
    }

    fn global(sock: &mut UnixStream, registry: u32, name: u32, interface: &str, version: u32) {
        let mut body = name.to_ne_bytes().to_vec();
        put_string(&mut body, interface);
        body.extend(version.to_ne_bytes());
        send(sock, registry, 0, &body);
    }

    pub(crate) fn request(sock: &mut UnixStream) -> std::io::Result<(u32, u32, Vec<u8>)> {
        let mut head = [0u8; 8];
        sock.read_exact(&mut head)?;
        let object = u32::from_ne_bytes(head[..4].try_into().unwrap());
        let word = u32::from_ne_bytes(head[4..].try_into().unwrap());
        let mut body = vec![0u8; (word >> 16) as usize - 8];
        sock.read_exact(&mut body)?;
        Ok((object, word & 0xffff, body))
    }

    /// Just enough compositor for `Link::start_on`: the two globals it binds, `reprise_input_v1` at
    /// `version`, and an answer to each of `syncs` syncs, ignoring other requests. Returns the object id
    /// the client bound `reprise_input_v1` to.
    pub(crate) fn fake_compositor(sock: &mut UnixStream, syncs: usize, version: u32) -> u32 {
        let mut answered = 0;
        let mut registry = 0;
        let mut input = 0;
        while answered < syncs {
            let (object, opcode, body) = request(sock).unwrap();
            let new_id = || u32::from_ne_bytes(body[..4].try_into().unwrap());
            match (object, opcode) {
                (1, 0) => {
                    send(sock, new_id(), 0, &0u32.to_ne_bytes());
                    answered += 1;
                }
                (1, 1) => {
                    registry = new_id();
                    global(sock, registry, 1, "reprise_input_v1", version);
                    global(sock, registry, 2, "zxdg_output_manager_v1", 3);
                }
                (o, 0) if o == registry && new_id() == 1 => {
                    input = u32::from_ne_bytes(body[body.len() - 4..].try_into().unwrap());
                }
                _ => {}
            }
        }
        input
    }

    /// A link that is no process's lifeline, on the client end of a fake compositor's socket.
    pub(crate) fn link_on(ours: UnixStream) -> Link {
        Link::start_on(Connection::from_socket(ours).unwrap(), false).unwrap()
    }

    /// A link on a fake compositor serving `reprise_input_v1` at `version`. The fake hangs up when the sender sends or drops; the handle joins it.
    fn start_with_fake(version: u32) -> (Link, Sender<()>, std::thread::JoinHandle<()>) {
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        let (hang_up, hang_up_rx) = channel::<()>();
        let compositor = std::thread::spawn(move || {
            fake_compositor(&mut theirs, 2, version);
            let _ = hang_up_rx.recv();
        });
        let link = Link::start_on(Connection::from_socket(ours).unwrap(), false).unwrap();
        (link, hang_up, compositor)
    }

    #[test]
    fn a_v2_compositor_offers_no_shortcuts() {
        let (link, _hang_up, _fake) = start_with_fake(2);
        assert!(!link.has_shortcuts(), "a v2 global has no create_shortcuts");
    }

    #[test]
    fn a_v3_compositor_offers_shortcuts() {
        let (link, _hang_up, _fake) = start_with_fake(3);
        assert!(link.has_shortcuts(), "a v3 global has create_shortcuts");
    }

    #[test]
    fn only_a_v4_compositor_gets_keys_objects() {
        let (link, _hang_up, _fake) = start_with_fake(3);
        assert!(
            link.create_keys(Owner::Server).is_none(),
            "a v3 global has no create_keys"
        );
        let (link, _hang_up, _fake) = start_with_fake(4);
        assert!(
            link.create_keys(Owner::Server).is_some(),
            "a v4 global has create_keys"
        );
    }

    /// Past `MAX_SHORTCUTS` live objects the link makes no more, so the compositor never ends it with `too_many`; a dropped one frees a place.
    #[test]
    fn the_link_keeps_no_more_shortcuts_objects_than_the_compositor_takes() {
        let (link, _hang_up, _fake) = start_with_fake(3);
        let mut held: Vec<ShortcutsHandle> = (0..MAX_SHORTCUTS)
            .map(|i| {
                link.create_shortcuts("org.example.App")
                    .unwrap_or_else(|| panic!("object {i} is made"))
            })
            .collect();
        assert!(
            link.create_shortcuts("org.example.App").is_none(),
            "one past {MAX_SHORTCUTS} is refused"
        );
        held.pop();
        assert!(
            link.create_shortcuts("org.example.App").is_some(),
            "a dropped object frees its place"
        );
    }

    /// Past `MAX_CAPTURES` live objects the link makes no more, so the compositor never ends it with `too_many`; a dropped one frees a place.
    #[test]
    fn the_link_keeps_no_more_capture_objects_than_the_compositor_takes() {
        let (link, _hang_up, _fake) = start_with_fake(2);
        let mut held: Vec<CaptureHandle> = (0..MAX_CAPTURES)
            .map(|i| {
                link.create_capture()
                    .unwrap_or_else(|| panic!("capture {i} is made"))
            })
            .collect();
        assert!(
            link.create_capture().is_none(),
            "one past {MAX_CAPTURES} is refused"
        );
        held.pop();
        assert!(
            link.create_capture().is_some(),
            "a dropped capture frees its place"
        );
    }

    #[test]
    fn a_shortcuts_object_reports_its_events_and_dies_with_its_handle() {
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        let compositor = std::thread::spawn(move || {
            let input = fake_compositor(&mut theirs, 2, 3);
            theirs
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let (shortcuts, app_id) = loop {
                let (object, opcode, body) = request(&mut theirs).unwrap();
                if (object, opcode) == (input, 3) {
                    let id = u32::from_ne_bytes(body[..4].try_into().unwrap());
                    break (id, get_string(&body[4..]));
                }
            };
            for (opcode, id, time) in [(0, "toggle", 10u32), (1, "toggle", 20)] {
                let mut body = Vec::new();
                put_string(&mut body, id);
                body.extend(time.to_ne_bytes());
                send(&mut theirs, shortcuts, opcode, &body);
            }
            send(&mut theirs, shortcuts, 2, &[]);
            let destroyed = std::iter::from_fn(|| request(&mut theirs).ok())
                .any(|(object, opcode, _)| (object, opcode) == (shortcuts, 0));
            (app_id, destroyed)
        });
        let link = Link::start_on(Connection::from_socket(ours).unwrap(), false).unwrap();
        let handle = link.create_shortcuts("org.example.Player").unwrap();
        let events = handle.events();
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while got.len() < 3 && Instant::now() < deadline {
            match events.try_recv() {
                Ok(e) => got.push(e),
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert_eq!(
            got,
            [
                ShortcutEvent::Activated {
                    id: "toggle".into(),
                    time: 10
                },
                ShortcutEvent::Deactivated {
                    id: "toggle".into(),
                    time: 20
                },
                ShortcutEvent::Changed,
            ],
            "the press, the release and the config change arrive in order"
        );
        drop(handle);
        let (app_id, destroyed) = compositor.join().unwrap();
        assert_eq!(
            app_id, "org.example.Player",
            "create_shortcuts carries the app id"
        );
        assert!(destroyed, "dropping the handle destroys the object");
    }

    #[test]
    fn a_pick_hears_the_colour_and_dies_with_its_handle() {
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        let compositor = std::thread::spawn(move || {
            let input = fake_compositor(&mut theirs, 2, 6);
            theirs
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let pick = loop {
                let (object, opcode, body) = request(&mut theirs).unwrap();
                if (object, opcode) == (input, 6) {
                    break u32::from_ne_bytes(body[..4].try_into().unwrap());
                }
            };
            send(&mut theirs, pick, 0, &0x0080_00ffu32.to_ne_bytes());
            std::iter::from_fn(|| request(&mut theirs).ok())
                .any(|(object, opcode, _)| (object, opcode) == (pick, 0))
        });
        let link = Link::start_on(Connection::from_socket(ours).unwrap(), false).unwrap();
        let handle = link.pick_color().unwrap();
        assert_eq!(zbus::block_on(handle.answer()), Some(0x0080_00ff));
        drop(handle);
        assert!(
            compositor.join().unwrap(),
            "dropping the handle destroys the object"
        );
        let (link, _hang_up, _fake) = start_with_fake(5);
        assert!(link.pick_color().is_none(), "a v5 global has no pick_color");
    }

    /// The requests `reprise_input_v1` gets until `stop` arrives: opcode and the string at
    /// `at` in the body, if any.
    fn named_requests(
        version: u32,
        stop: u32,
        at: impl Fn(u32) -> Option<usize> + Send + 'static,
        drive: impl FnOnce(&Link),
    ) -> Vec<(u32, Option<String>)> {
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        let compositor = std::thread::spawn(move || {
            let input = fake_compositor(&mut theirs, 2, version);
            theirs
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut seen = Vec::new();
            loop {
                let (object, opcode, body) = request(&mut theirs).unwrap();
                if object != input {
                    continue;
                }
                let name = at(opcode).map(|i| {
                    let len = u32::from_ne_bytes(body[i..i + 4].try_into().unwrap()) as usize;
                    String::from_utf8(body[i + 4..i + 4 + len - 1].to_vec()).unwrap()
                });
                seen.push((opcode, name));
                if opcode == stop {
                    break seen;
                }
            }
        });
        let link = Link::start_on(Connection::from_socket(ours).unwrap(), false).unwrap();
        drive(&link);
        compositor.join().unwrap()
    }

    #[test]
    fn a_v10_compositor_hears_which_app_an_input_object_serves() {
        const CONNECT_EIS: u32 = 0;
        const CREATE_KEYS: u32 = 4;
        const CONNECT_APP_SENDER: u32 = 9;
        const CREATE_APP_KEYS: u32 = 10;
        let at = |op| match op {
            CONNECT_APP_SENDER => Some(8),
            CREATE_APP_KEYS => Some(4),
            _ => None,
        };
        let app = || Owner::App("org.example.Remote".into());
        let seen = named_requests(10, CREATE_APP_KEYS, at, |link| {
            let (fd, _peer) = UnixStream::pair().unwrap();
            let _server_keys = link.create_keys(Owner::Server);
            let _eis = link.connect_sender(fd.into(), Device::Keyboard as u32, app());
            let _keys = link.create_keys(app());
            std::thread::sleep(Duration::from_millis(50));
        });
        let named = Some("org.example.Remote".to_string());
        assert_eq!(
            seen,
            [
                (CREATE_KEYS, None),
                (CONNECT_APP_SENDER, named.clone()),
                (CREATE_APP_KEYS, named)
            ]
        );
        let seen = named_requests(9, CREATE_KEYS, at, |link| {
            let (fd, _peer) = UnixStream::pair().unwrap();
            let _eis = link.connect_sender(fd.into(), Device::Keyboard as u32, app());
            let _keys = link.create_keys(app());
            std::thread::sleep(Duration::from_millis(50));
        });
        assert_eq!(
            seen,
            [(CONNECT_EIS, None), (CREATE_KEYS, None)],
            "a v9 compositor gets the requests it knows"
        );
    }

    #[test]
    fn only_the_remote_desktop_servers_sender_is_a_remote_session() {
        for (version, remote_session, kind) in [(7, true, 3), (7, false, 1), (6, true, 1)] {
            let owner = match remote_session {
                true => Owner::Server,
                false => Owner::App("org.example.App".into()),
            };
            let (ours, mut theirs) = UnixStream::pair().unwrap();
            let compositor = std::thread::spawn(move || {
                let input = fake_compositor(&mut theirs, 2, version);
                theirs
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                loop {
                    let (object, opcode, body) = request(&mut theirs).unwrap();
                    if (object, opcode) == (input, 0) {
                        break u32::from_ne_bytes(body[4..8].try_into().unwrap());
                    }
                }
            });
            let link = Link::start_on(Connection::from_socket(ours).unwrap(), false).unwrap();
            let (fd, _peer) = UnixStream::pair().unwrap();
            let _eis = link.connect_sender(fd.into(), Device::Keyboard as u32, owner);
            assert_eq!(
                compositor.join().unwrap(),
                kind,
                "v{version}, remote session {remote_session}"
            );
        }
    }

    #[test]
    fn a_receiver_is_offered_only_the_devices_its_session_holds() {
        for devices in [
            Device::Pointer as u32,
            Device::Keyboard as u32 | Device::Pointer as u32,
        ] {
            let (ours, mut theirs) = UnixStream::pair().unwrap();
            let compositor = std::thread::spawn(move || {
                let input = fake_compositor(&mut theirs, 2, 10);
                theirs
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                loop {
                    let (object, opcode, body) = request(&mut theirs).unwrap();
                    if (object, opcode) == (input, 0) {
                        let word =
                            |i: usize| u32::from_ne_bytes(body[i..i + 4].try_into().unwrap());
                        break (word(4), word(12));
                    }
                }
            });
            let link = Link::start_on(Connection::from_socket(ours).unwrap(), false).unwrap();
            let capture = link.create_capture().unwrap();
            let (fd, _peer) = UnixStream::pair().unwrap();
            let _eis = capture.connect_receiver(fd.into(), devices);
            assert_eq!(compositor.join().unwrap(), (Kind::Receiver as u32, devices));
        }
    }

    #[test]
    fn a_v2_compositor_gets_no_shortcuts_request_and_the_link_lives() {
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        let compositor = std::thread::spawn(move || {
            let input = fake_compositor(&mut theirs, 2, 2);
            theirs
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            // The capture made after the shortcuts object marks the end of what the link sent for it.
            let mut sent_shortcuts = false;
            loop {
                let (object, opcode, _) = request(&mut theirs).unwrap();
                match (object == input, opcode) {
                    (true, 1) => break,
                    (true, 3) => sent_shortcuts = true,
                    _ => {}
                }
            }
            (sent_shortcuts, theirs)
        });
        let link = Link::start_on(Connection::from_socket(ours).unwrap(), false).unwrap();
        let handle = link.create_shortcuts("org.example.Player").unwrap();
        let capture = link.create_capture().unwrap();
        let (sent_shortcuts, _theirs) = compositor.join().unwrap();
        assert!(!sent_shortcuts, "a v2 compositor gets no create_shortcuts");
        assert!(
            handle.events().is_closed(),
            "the handle's events end on a v2 compositor"
        );
        let link = Some(link);
        assert!(live(&link).is_some(), "the link is still live");
        drop((handle, capture));
    }

    #[test]
    fn a_lost_connection_takes_the_link_out_of_service() {
        let (link, hang_up, compositor) = start_with_fake(2);
        let link = Some(link);
        assert!(live(&link).is_some(), "the link is live while connected");
        hang_up.send(()).unwrap();
        compositor.join().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while live(&link).is_some() {
            assert!(
                Instant::now() < deadline,
                "the link is out of service within 5 s of the hangup"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn requests_held_by_a_full_socket_go_out_when_it_drains() {
        const REQUESTS: usize = 100;
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        // The kernel raises this to its minimum send buffer, which a handful of one-request writes fill.
        let size: libc::c_int = 1;
        // SAFETY: `ours` is an open socket and `size` is a c_int that outlives the call.
        let set = unsafe {
            libc::setsockopt(
                ours.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                (&size as *const libc::c_int).cast(),
                size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        assert_eq!(set, 0, "SO_SNDBUF is set");
        let (drain, drain_rx) = channel::<()>();
        let compositor = std::thread::spawn(move || {
            fake_compositor(&mut theirs, 2, 4);
            drain_rx.recv().unwrap();
            theirs
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut received = 0;
            while received < REQUESTS && request(&mut theirs).is_ok() {
                received += 1;
            }
            received
        });
        let link = Link::start_on(Connection::from_socket(ours).unwrap(), false).unwrap();
        let keys: Vec<KeysHandle> = (0..REQUESTS)
            .map(|_| {
                let k = link.create_keys(Owner::Server).unwrap();
                std::thread::sleep(Duration::from_millis(1));
                k
            })
            .collect();
        drain.send(()).unwrap();
        assert_eq!(
            compositor.join().unwrap(),
            REQUESTS,
            "every create_keys reaches the compositor once it reads"
        );
        drop(keys);
    }
}
