//! The tray's D-Bus side, on its own thread.

use super::screensaver::{self, SCREENSAVER_NAME, SCREENSAVER_PATHS};
use super::settings::{self, Appearance, SETTINGS_NAME, SETTINGS_PATH, Settings};
use super::{TrayCommand, TrayEvent, TrayItem};
use crate::icontheme::{IconCache, IconTheme};
use crate::mutex::lock;
use futures_util::stream::{BoxStream, SelectAll, StreamExt};
use reprisebus::peer::Identities;
use std::collections::{HashMap, HashSet};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use zbus::message::Header;
use zbus::object_server::{InterfaceRef, SignalEmitter};
use zbus::proxy::CacheProperties;
use zbus::{Connection, Proxy};

mod decoders;
mod item;
mod menu;
mod notifications;
#[cfg(test)]
mod tests;
mod watcher;

use item::{call_item, fetch_item, provide_token};
use menu::{get_layout, send_menu_event};
use notifications::{Notifications, Programs, Senders};
use watcher::{Published, Watcher, split_service};

const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_IFACE: &str = "org.kde.StatusNotifierWatcher";
const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
/// Item signals that change what `fetch_item` reads.
const ITEM_SIGNALS: [&str; 7] = [
    "NewIcon",
    "NewAttentionIcon",
    "NewOverlayIcon",
    "NewTitle",
    "NewStatus",
    "NewToolTip",
    "NewMenu",
];

const DBUS_IFACE: &str = "org.freedesktop.DBus";
const NOTIFY_NAME: &str = "org.freedesktop.Notifications";
const NOTIFY_PATH: &str = "/org/freedesktop/Notifications";

/// Cap on every method call. Calls run beside the loop, so a hung tray app holds up only its own.
const METHOD_TIMEOUT: Duration = Duration::from_secs(5);

fn wake(eventfd: RawFd) {
    let one: u64 = 1;
    // SAFETY: the pointer covers `one`'s 8 bytes, live for the whole call;
    // write reads nothing else. The result is ignored: a full counter
    // (EAGAIN) still leaves the fd readable.
    unsafe {
        libc::write(eventfd, &one as *const u64 as *const libc::c_void, 8);
    }
}

pub(super) fn emit(events: &Sender<TrayEvent>, eventfd: RawFd, ev: TrayEvent) {
    if events.send(ev).is_ok() {
        wake(eventfd);
    }
}

struct Shell {
    events: Sender<TrayEvent>,
    eventfd: RawFd,
    pads: crate::tabletpad::Shared,
    programs: Arc<Mutex<Programs>>,
    responding: Arc<Mutex<Responding>>,
}

/// The bus names following `NotRespondingChanged`, and the windows it last named.
#[derive(Default)]
struct Responding {
    watchers: HashSet<String>,
    hung: Vec<reprisebus::tray::Hung>,
}

impl Responding {
    /// `sender` follows the list: whether it is the first.
    fn watch(&mut self, sender: &str) -> bool {
        self.watchers.insert(sender.to_string()) && self.watchers.len() == 1
    }

    /// `name` left the bus: whether it was the last to follow the list.
    fn left(&mut self, name: &str) -> bool {
        self.watchers.remove(name) && self.watchers.is_empty()
    }
}

#[zbus::interface(name = "org.reprise.Tray")]
impl Shell {
    async fn customize(&self) {
        emit(&self.events, self.eventfd, TrayEvent::Customize);
    }

    async fn critical_turn_off(&self) {
        emit(&self.events, self.eventfd, TrayEvent::CriticalTurnOff);
    }

    async fn restart(&self) {
        emit(&self.events, self.eventfd, TrayEvent::Restart);
    }

    /// Asks to empty the Recycle Bin, as its desktop icon's menu does.
    async fn empty_recycle_bin(&self) {
        emit(&self.events, self.eventfd, TrayEvent::EmptyRecycleBin);
    }

    async fn identify(&self, outputs: Vec<String>) {
        emit(&self.events, self.eventfd, TrayEvent::Identify(outputs));
    }

    async fn preview_screen_saver(&self, name: String) -> zbus::fdo::Result<()> {
        let kind = reprisekit::saver::Kind::parse(&name)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no screen saver {name:?}")))?;
        emit(
            &self.events,
            self.eventfd,
            TrayEvent::PreviewScreenSaver(kind),
        );
        Ok(())
    }

    async fn forget_window_frames(&self) {
        emit(&self.events, self.eventfd, TrayEvent::ForgetWindowFrames);
    }

    async fn reload_remembered(&self) {
        emit(&self.events, self.eventfd, TrayEvent::ReloadRemembered);
    }

    /// The icon of the window playing a sound stream; see [`crate::streamwindow`].
    async fn window_icon(&self, pids: Vec<u32>, title: String) -> reprisebus::tray::WindowIconWire {
        use crate::streamwindow::{lineage, read_proc};
        let own = i32::try_from(std::process::id()).unwrap_or(0);
        let session: Vec<i32> = lineage(own, &[], read_proc)
            .iter()
            .map(|s| s.host)
            .collect();
        let lineages = pids
            .iter()
            .filter_map(|&p| i32::try_from(p).ok())
            .map(|p| lineage(p, &session, read_proc))
            .collect();
        let (reply, answer) = async_channel::bounded(1);
        emit(
            &self.events,
            self.eventfd,
            TrayEvent::WindowIcon {
                lineages,
                title,
                reply,
            },
        );
        answer
            .recv()
            .await
            .unwrap_or_else(|_| reprisebus::tray::WindowIcon::wire(None))
    }

    async fn tablet_pads(&self) -> Vec<reprisebus::tray::PadWire> {
        let pads = lock(&self.pads);
        pads.iter().map(reprisebus::tray::Pad::wire).collect()
    }

    /// The programs that sent notifications this session, as (key, name).
    async fn notification_programs(&self) -> Vec<(String, String)> {
        lock(&self.programs).0.clone()
    }

    /// The windows not answering pings now. Until the caller leaves the bus, every window is
    /// pinged now and then and `NotRespondingChanged` follows the list.
    async fn watch_responding(
        &self,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Vec<reprisebus::tray::Hung> {
        let mut r = lock(&self.responding);
        if let Some(sender) = hdr.sender()
            && r.watch(sender)
        {
            emit(&self.events, self.eventfd, TrayEvent::WatchResponding(true));
        }
        r.hung.clone()
    }

    #[zbus(signal)]
    async fn not_responding_changed(
        emitter: &SignalEmitter<'_>,
        windows: &[reprisebus::tray::Hung],
    ) -> zbus::Result<()>;
}

/// Property caching off: the lazy cache backs every `Get` with one `GetAll`, so one failure poisons
/// every later read, and SNI peers routinely omit properties.
async fn uncached_proxy(
    conn: &Connection,
    bus: &str,
    path: &str,
    interface: &'static str,
) -> zbus::Result<Proxy<'static>> {
    zbus::proxy::Builder::<Proxy<'static>>::new(conn)
        .destination(bus.to_string())
        .path(path.to_string())
        .interface(interface)
        .cache_properties(CacheProperties::No)
        .build()
        .await
}

type Icons = Arc<Mutex<IconCache>>;

/// The px item icons are drawn at, which [`TrayCommand::IconSizes`] sets.
type Sizes = Arc<Mutex<Vec<i32>>>;

fn sizes_now(sizes: &Sizes) -> Vec<i32> {
    lock(sizes).clone()
}

enum Wake {
    Command(TrayCommand),
    Registered {
        bus: String,
        path: String,
    },
    ForeignRegistered(String),
    ForeignUnregistered(String),
    ItemChanged(String),
    Owner {
        name: String,
        new_owner: String,
    },
    /// A read's item, and whether its named icons were decoded.
    Fetched {
        key: String,
        fetch: u64,
        item: Option<Box<TrayItem>>,
        decoded: bool,
    },
    Unmuted(String),
    /// The connection to the bus is gone.
    Lost,
}

/// How long a change signal waits for its read, so a burst of them costs one read.
const SETTLE: Duration = Duration::from_millis(50);

/// When a read whose named icons weren't decoded is tried once more.
const RETRY_AFTER: Duration = Duration::from_secs(2);

/// Change signals an item may send within [`CHANGE_WINDOW`]; past that its match rules are dropped
/// for [`MUTED_FOR`], so a flood stops at the bus daemon, which would disconnect a slow reader.
const MAX_CHANGES: u32 = 100;
const CHANGE_WINDOW: Duration = Duration::from_secs(1);
const MUTED_FOR: Duration = Duration::from_secs(5);

struct Changes {
    since: Instant,
    count: u32,
}

impl Changes {
    /// Counts one at `now`: whether the item is past [`MAX_CHANGES`] in its window.
    fn note(&mut self, now: Instant) -> bool {
        if now.duration_since(self.since) >= CHANGE_WINDOW {
            self.since = now;
            self.count = 0;
        }
        self.count += 1;
        self.count > MAX_CHANGES
    }
}

struct Fetch {
    /// Tells its result from that of a fetch dropped with a removed item.
    id: u64,
    /// Set as the read begins: a change before that is still read by it.
    reading: Arc<AtomicBool>,
    /// A change arrived during the read and needs a read of its own.
    again: bool,
    /// The one read again after a read whose named icons weren't decoded.
    retry: bool,
    /// Dropped with the fetch, which ends its read.
    _stop: async_channel::Sender<()>,
}

enum Ended {
    Shutdown,
    /// With the items registered with the watcher it hosted.
    Lost(Vec<Published>),
}

type WatcherItems = Arc<Mutex<Vec<Published>>>;

struct Loop {
    conn: Connection,
    events: Sender<TrayEvent>,
    eventfd: RawFd,
    items: HashMap<String, TrayItem>,
    fetching: HashMap<String, Fetch>,
    next_fetch: u64,
    /// Dropping an item's sender ends its signal streams, and their match rules with them.
    item_stops: HashMap<String, async_channel::Sender<()>>,
    changes: HashMap<String, Changes>,
    /// Items whose signals are dropped for flooding, until their `Unmuted`.
    muted: HashSet<String>,
    muted_for: Duration,
    published: Option<WatcherItems>,
    /// The watcher's items and registrations while another process holds its name.
    idle_watcher: Option<(WatcherItems, async_channel::Receiver<Wake>)>,
    screensaver: screensaver::Shared,
    streams: SelectAll<BoxStream<'static, Wake>>,
    appearance: Arc<Mutex<Appearance>>,
    icons: Icons,
    sizes: Sizes,
    senders: Arc<Mutex<Senders>>,
    responding: Arc<Mutex<Responding>>,
    identities: Arc<Identities>,
}

impl Loop {
    fn emit(&self, ev: TrayEvent) {
        emit(&self.events, self.eventfd, ev);
    }

    /// The watch comes first, so a change the item makes while being read is read as well.
    async fn track(&mut self, bus: String, path: String) {
        self.watch_item(&bus, &path).await;
        self.fetch(bus, path, Duration::ZERO, false);
    }

    /// Subscribe to [`ITEM_SIGNALS`] on an item, unless already watching it; each fires
    /// `ItemChanged`.
    async fn watch_item(&mut self, bus: &str, path: &str) {
        let key = format!("{bus}{path}");
        if self.item_stops.contains_key(&key) || self.muted.contains(&key) {
            return;
        }
        let Ok(proxy) = uncached_proxy(&self.conn, bus, path, ITEM_IFACE).await else {
            return;
        };
        let (stop_tx, stop_rx) = async_channel::bounded::<()>(1);
        self.item_stops.insert(key.clone(), stop_tx);
        for sig in ITEM_SIGNALS {
            if let Ok(stream) = proxy.receive_signal(sig).await {
                let key = key.clone();
                let stop = stop_rx.clone();
                let changes = stream.map(move |_| Wake::ItemChanged(key.clone()));
                self.streams.push(
                    pumped(changes)
                        .take_until(Box::pin(async move {
                            let _ = stop.recv().await;
                        }))
                        .boxed(),
                );
            }
        }
    }

    async fn unpublish(&self, key: &str) {
        let Some(published) = &self.published else {
            return;
        };
        lock(published).retain(|p| p.key != key);
        if let Ok(iface) = self
            .conn
            .object_server()
            .interface::<_, Watcher>(WATCHER_PATH)
            .await
        {
            let _ = Watcher::status_notifier_item_unregistered(iface.signal_emitter(), key).await;
        }
    }

    async fn notifications(&self) -> zbus::Result<InterfaceRef<Notifications>> {
        self.conn
            .object_server()
            .interface::<_, Notifications>(NOTIFY_PATH)
            .await
    }

    /// Notification `id`'s signals, addressed to its sender: a typed reply or token is for it alone.
    async fn to_sender(&self, id: u32) -> Option<SignalEmitter<'static>> {
        let sender = lock(&self.senders).0.get(&id)?.0.clone();
        let name = zbus::names::BusName::try_from(sender).ok()?;
        let iface = self.notifications().await.ok()?;
        Some(iface.signal_emitter().to_owned().set_destination(name))
    }

    /// Run `call` beside the loop, so a peer that never answers holds up only its own call.
    fn spawn(&mut self, call: impl Future<Output = ()> + Send + 'static) {
        let done = futures_util::stream::once(call).filter_map(|()| std::future::ready(None));
        self.streams.push(pumped(done));
    }

    /// Read the item after `settle`, unless a read of it is under way; `retry` marks the one
    /// read again after a read whose named icons weren't decoded.
    fn fetch(&mut self, bus: String, path: String, settle: Duration, retry: bool) {
        let key = format!("{bus}{path}");
        if let Some(f) = self.fetching.get_mut(&key) {
            f.again |= f.reading.load(Ordering::Relaxed);
            return;
        }
        let fetch = self.next_fetch;
        self.next_fetch += 1;
        let reading = Arc::new(AtomicBool::new(false));
        let (stop, stopped) = async_channel::bounded::<()>(1);
        self.fetching.insert(
            key.clone(),
            Fetch {
                id: fetch,
                reading: reading.clone(),
                again: false,
                retry,
                _stop: stop,
            },
        );
        let (conn, icons, sizes) = (self.conn.clone(), self.icons.clone(), self.sizes.clone());
        let identities = self.identities.clone();
        let read = async move {
            if !settle.is_zero() {
                async_io::Timer::after(settle).await;
            }
            reading.store(true, Ordering::Relaxed);
            let read = fetch_item(&conn, &icons, &sizes, &identities, &bus, &path).await;
            let decoded = read.as_ref().is_none_or(|r| r.1);
            let item = read.map(|r| Box::new(r.0));
            Wake::Fetched {
                key,
                fetch,
                item,
                decoded,
            }
        };
        let read = futures_util::stream::once(read).take_until(async move {
            let _ = stopped.recv().await;
        });
        self.streams.push(pumped(read));
    }

    async fn fetched(
        &mut self,
        key: String,
        fetch: u64,
        item: Option<Box<TrayItem>>,
        decoded: bool,
    ) {
        if self.fetching.get(&key).is_none_or(|f| f.id != fetch) {
            return;
        }
        let Some(done) = self.fetching.remove(&key) else {
            return;
        };
        let retry = !decoded && !done.retry;
        match item {
            Some(mut item) => {
                if !decoded && let Some(old) = self.items.get(&key) {
                    keep_named(&mut item, old);
                }
                self.show(key.clone(), *item);
            }
            None if self.items.contains_key(&key) => {}
            None => {
                log::warn!("tray: could not read item {key}");
                // Registered but unreadable: never advertise it as one of ours,
                // and stop watching it unless a change asked for another read.
                if !done.again {
                    self.item_stops.remove(&key);
                }
                self.unpublish(&key).await;
            }
        }
        let (bus, path) = split_service(&key);
        if done.again {
            self.fetch(bus, path, SETTLE, false);
        } else if retry {
            self.fetch(bus, path, RETRY_AFTER, true);
        }
    }

    /// Store a read item and tell the compositor, unless it is unchanged.
    fn show(&mut self, key: String, item: TrayItem) {
        if self.items.get(&key) == Some(&item) {
            return;
        }
        let existed = self.items.insert(key, item.clone()).is_some();
        self.emit(if existed {
            TrayEvent::Updated(item)
        } else {
            TrayEvent::Added(item)
        });
    }

    /// A change signal from the item `key`: read it again, or stop listening to it for a while
    /// when it sends too many.
    fn item_changed(&mut self, key: String, now: Instant) {
        let changes = self.changes.entry(key.clone()).or_insert(Changes {
            since: now,
            count: 0,
        });
        if !changes.note(now) {
            self.refresh_item(&key);
            return;
        }
        if self.muted.insert(key.clone()) {
            log::warn!(
                "tray: {key} sent over {MAX_CHANGES} changes within {CHANGE_WINDOW:?}; ignoring it for {:?}",
                self.muted_for
            );
            self.item_stops.remove(&key);
            let after = self.muted_for;
            let unmute = async move {
                async_io::Timer::after(after).await;
                Wake::Unmuted(key)
            };
            self.streams
                .push(futures_util::stream::once(unmute).boxed());
        }
    }

    /// Listen to a muted item again, and read it for what it changed meanwhile.
    async fn unmuted(&mut self, key: String) {
        if !self.muted.remove(&key) {
            return;
        }
        self.changes.remove(&key);
        if self.items.contains_key(&key) || self.fetching.contains_key(&key) {
            let (bus, path) = split_service(&key);
            self.watch_item(&bus, &path).await;
            self.fetch(bus, path, Duration::ZERO, false);
        }
    }

    fn refresh_item(&mut self, key: &str) {
        if self.items.contains_key(key) || self.fetching.contains_key(key) {
            let (bus, path) = split_service(key);
            self.fetch(bus, path, SETTLE, false);
        }
    }

    async fn remove_item(&mut self, key: &str) {
        let was_fetching = self.fetching.remove(key).is_some();
        self.item_stops.remove(key);
        self.changes.remove(key);
        self.muted.remove(key);
        if self.items.remove(key).is_none() {
            if was_fetching {
                self.unpublish(key).await;
            }
            return;
        }
        self.item_stops.remove(key);
        self.unpublish(key).await;
        self.emit(TrayEvent::Removed(key.to_string()));
    }

    async fn remove_items_on(&mut self, bus: &str) {
        let shown = self
            .items
            .values()
            .filter(|i| i.service == bus)
            .map(|i| i.id.clone());
        let read = self
            .fetching
            .keys()
            .filter(|k| split_service(k).0 == bus)
            .cloned();
        let gone: Vec<String> = shown.chain(read).collect();
        for key in gone {
            self.remove_item(&key).await;
        }
    }

    async fn handle_command(&mut self, cmd: TrayCommand) -> bool {
        let peer = |id: &str| self.items.get(id).cloned();
        match cmd {
            TrayCommand::Click {
                id,
                x,
                y,
                kind,
                token,
            } => {
                if let Some(p) = peer(&id) {
                    let conn = self.conn.clone();
                    self.spawn(async move {
                        if let Some(token) = token {
                            provide_token(&conn, &p, &token).await;
                        }
                        call_item(&conn, &p, kind.method(), &(x, y)).await
                    });
                }
            }
            TrayCommand::Scroll {
                id,
                delta,
                orientation,
            } => {
                if let Some(p) = peer(&id) {
                    let conn = self.conn.clone();
                    self.spawn(async move {
                        call_item(&conn, &p, "Scroll", &(delta, orientation.as_str())).await
                    });
                }
            }
            TrayCommand::GetLayout { id, parent } => {
                // The main loop is waiting on this; an unreachable item or a
                // menu we cannot read answers with an empty list rather than
                // with silence.
                let p = peer(&id);
                let (conn, events, eventfd) =
                    (self.conn.clone(), self.events.clone(), self.eventfd);
                self.spawn(async move {
                    let items = match p {
                        Some(p) => get_layout(&conn, &p, parent).await.unwrap_or_default(),
                        None => Vec::new(),
                    };
                    emit(
                        &events,
                        eventfd,
                        TrayEvent::MenuLayout { id, parent, items },
                    );
                });
            }
            TrayCommand::MenuEvent {
                id,
                item: node,
                token,
            } => {
                if let Some(p) = peer(&id) {
                    let conn = self.conn.clone();
                    self.spawn(async move {
                        provide_token(&conn, &p, &token).await;
                        send_menu_event(&conn, &p, node).await
                    });
                }
            }
            TrayCommand::NotificationClosed { id, reason } => {
                if let Some(to) = self.to_sender(id).await {
                    let _ = Notifications::notification_closed(&to, id, reason).await;
                }
                lock(&self.senders).0.remove(&id);
            }
            TrayCommand::NotResponding(hung) => {
                lock(&self.responding).hung = hung.clone();
                let shell = (self.conn.object_server())
                    .interface::<_, Shell>(reprisebus::tray::PATH)
                    .await;
                if let Ok(iface) = shell {
                    let _ = Shell::not_responding_changed(iface.signal_emitter(), &hung).await;
                }
            }
            TrayCommand::Locked(locked) => {
                self.screensaver.set_active(locked);
                for path in SCREENSAVER_PATHS {
                    let iface = (self.conn.object_server())
                        .interface::<_, screensaver::ScreenSaver>(path)
                        .await;
                    if let Ok(iface) = iface {
                        let emitter = iface.signal_emitter();
                        let _ = screensaver::ScreenSaver::active_changed(emitter, locked).await;
                    }
                }
            }
            TrayCommand::Replied { id, text } => {
                if let Some(to) = self.to_sender(id).await {
                    let _ = Notifications::notification_replied(&to, id, &text).await;
                }
            }
            TrayCommand::ActionInvoked { id, key, token } => {
                if let Some(to) = self.to_sender(id).await {
                    if let Some(token) = token {
                        let _ = Notifications::activation_token(&to, id, &token).await;
                    }
                    let _ = Notifications::action_invoked(&to, id, &key).await;
                }
            }
            TrayCommand::Appearance(a) => settings::update(&self.conn, &self.appearance, a).await,
            TrayCommand::IconTheme(name) => self.retheme(&name),
            TrayCommand::IconSizes(sizes) => self.resize_icons(sizes),
            TrayCommand::RetakeNames => self.retake_names().await,
            TrayCommand::Shutdown => return false,
        }
        true
    }

    /// Ask again for every name not ours, without replacing a holder, so a session started
    /// meanwhile keeps its own; a later one still replaces ours.
    async fn retake_names(&mut self) {
        use zbus::fdo::{RequestNameFlags, RequestNameReply};
        let flags = retake_flags();
        let queued = flags.difference(RequestNameFlags::DO_NOT_QUEUE);
        for (name, flags) in [
            (NOTIFY_NAME, flags),
            (SETTINGS_NAME, flags),
            (reprisebus::tray::NAME, flags),
            (SCREENSAVER_NAME, queued),
        ] {
            match self.conn.request_name_with_flags(name, flags).await {
                Ok(RequestNameReply::PrimaryOwner) => log::info!("tray: hosting {name} again"),
                Ok(RequestNameReply::AlreadyOwner) => {}
                Ok(reply) => log::debug!("tray: {name}: {reply:?}; its holder keeps it"),
                Err(e) => log::info!("tray: {name}: {e}"),
            }
        }
        if own(&self.conn, WATCHER_NAME, flags).await.is_ok()
            && let Some((items, registrations)) = self.idle_watcher.take()
        {
            log::info!("tray: hosting {WATCHER_NAME}");
            self.published = Some(items);
            self.streams.push(registrations.boxed());
        }
    }

    /// Reads every item again, for `IconPixmap` entries and named icons at `sizes`.
    fn resize_icons(&mut self, sizes: Vec<i32>) {
        *lock(&self.sizes) = sizes;
        let keys: Vec<String> = self.items.keys().cloned().collect();
        for key in keys {
            self.refresh_item(&key);
        }
    }

    /// Reads every item again, for named icons from the new theme.
    fn retheme(&mut self, name: &str) {
        *lock(&self.icons) = load_icons(name);
        let keys: Vec<String> = self.items.keys().cloned().collect();
        for key in keys {
            self.refresh_item(&key);
        }
    }

    /// `name` left the bus: drop what it registered, inhibited or watched.
    async fn left(&mut self, name: &str) {
        self.remove_items_on(name).await;
        self.identities.forget(name);
        if lock(&self.senders).sent_by(name) {
            self.emit(TrayEvent::NotifySenderGone(name.to_string()));
        }
        if lock(&self.responding).left(name) {
            self.emit(TrayEvent::WatchResponding(false));
        }
        let dropped = self.screensaver.update(|h| h.forget(name));
        if dropped > 0 {
            log::info!("screensaver: {name} left the bus; {dropped} cookie(s) dropped");
        }
    }

    /// On a new connection, drop what peers that left while the tray was off the bus left
    /// behind: the old connection never heard them go. On another bus, every peer is gone.
    async fn forget_gone(&mut self, same_bus: bool) {
        self.identities.clear();
        let mut names: HashSet<String> = self.screensaver.holders().into_iter().collect();
        names.extend(lock(&self.senders).0.values().map(|s| s.0.clone()));
        names.extend(lock(&self.responding).watchers.iter().cloned());
        let dbus = zbus::fdo::DBusProxy::new(&self.conn).await.ok();
        for name in names {
            let gone = match (&dbus, zbus::names::BusName::try_from(name.as_str())) {
                _ if !same_bus => true,
                (Some(dbus), Ok(bus)) => dbus.name_has_owner(bus).await.ok() == Some(false),
                _ => false,
            };
            if gone {
                self.left(&name).await;
            }
        }
    }

    /// Hosting the watcher again, take back the items registered with the old one; a client
    /// that registers once at its start never registers them again.
    fn readmit(&mut self, registered: Vec<Published>) {
        let Some(published) = &self.published else {
            return;
        };
        for p in &registered {
            let (bus, path) = split_service(&p.key);
            push_known_item(&mut self.streams, bus, path);
        }
        lock(published).extend(registered);
    }

    async fn run(mut self) -> Ended {
        while let Some(wake) = self.streams.next().await {
            match wake {
                Wake::Command(cmd) => {
                    if !self.handle_command(cmd).await {
                        break;
                    }
                }
                Wake::Lost => {
                    for id in self.items.keys() {
                        self.emit(TrayEvent::Removed(id.clone()));
                    }
                    let registered = self.published.map(|p| std::mem::take(&mut *lock(&p)));
                    return Ended::Lost(registered.unwrap_or_default());
                }
                Wake::Unmuted(key) => self.unmuted(key).await,
                Wake::Registered { bus, path } => self.track(bus, path).await,
                Wake::ForeignRegistered(s) => {
                    let (bus, path) = split_service(&s);
                    self.track(bus, path).await;
                }
                Wake::ForeignUnregistered(s) => {
                    let (bus, path) = split_service(&s);
                    self.remove_item(&format!("{bus}{path}")).await;
                }
                Wake::ItemChanged(key) => self.item_changed(key, Instant::now()),
                Wake::Fetched {
                    key,
                    fetch,
                    item,
                    decoded,
                } => self.fetched(key, fetch, item, decoded).await,
                Wake::Owner { name, new_owner } => {
                    if new_owner.is_empty() {
                        self.left(&name).await;
                    }
                }
            }
        }
        Ended::Shutdown
    }
}

/// `item`'s named icons, where it would have decoded the same names as `old`, from `old`.
fn keep_named(item: &mut TrayItem, old: &TrayItem) {
    if item.icon_theme_path != old.icon_theme_path {
        return;
    }
    if item::named(&item.icon_name, &item.pixmaps).is_some() && item.icon_name == old.icon_name {
        item.named = old.named.clone();
    }
    let (new, was) = (&mut item.attention, &old.attention);
    if item::named(&new.name, &new.pixmaps).is_some() && new.name == was.name {
        new.named = was.named.clone();
    }
    let (new, was) = (&mut item.overlay, &old.overlay);
    if item::named(&new.name, &new.pixmaps).is_some() && new.name == was.name {
        new.named = was.named.clone();
    }
}

async fn signal_stream(
    proxy: &Proxy<'static>,
    member: &'static str,
    f: impl Fn(zbus::Message) -> Option<Wake> + Send + 'static,
) -> Option<BoxStream<'static, Wake>> {
    let stream = proxy.receive_signal(member).await.ok()?;
    let wakes = stream.filter_map(move |m| std::future::ready(f(m)));
    Some(pumped(wakes))
}

/// Drain `stream` on a task of its own, so it moves while the loop is inside an await: a
/// full zbus match queue never stops the socket, and a call never sits on the connection's write
/// lock until the loop polls again. Dropping the returned stream cancels the drain.
fn pumped<T: Send + 'static>(
    stream: impl futures_util::Stream<Item = T> + Send + 'static,
) -> BoxStream<'static, T> {
    let (tx, rx) = async_channel::unbounded();
    let drain = zruntime::spawn(async move {
        let mut stream = std::pin::pin!(stream);
        while let Some(item) = stream.next().await {
            if tx.send(item).await.is_err() {
                break;
            }
        }
    });
    futures_util::stream::unfold((rx, drain), |(rx, drain)| async move {
        let item = rx.recv().await.ok()?;
        Some((item, (rx, drain)))
    })
    .boxed()
}

fn load_icons(theme: &str) -> IconCache {
    IconCache::new(IconTheme::load(theme, &IconTheme::default_roots()))
}

/// How the tray asks for its well-known names. A session that defers to another Reprise session
/// of the user takes only names nobody holds and lets none go, so it never takes that session's
/// notifications, tray or settings over.
fn name_flags(defers: bool) -> zbus::fdo::RequestNameFlags {
    use zbus::fdo::RequestNameFlags;
    if defers {
        RequestNameFlags::DO_NOT_QUEUE
    } else {
        RequestNameFlags::default()
    }
}

/// How a session asks for its names again once the session it gave way to ended: only names
/// nobody holds, and replaceable by a later session.
fn retake_flags() -> zbus::fdo::RequestNameFlags {
    use zbus::fdo::RequestNameFlags;
    RequestNameFlags::ALLOW_REPLACEMENT | RequestNameFlags::DO_NOT_QUEUE
}

/// `org.freedesktop.ScreenSaver` as [`name_flags`] asks for it, except that a session that doesn't
/// defer queues for it, so it gets the name whenever its holder leaves.
fn screensaver_flags(defers: bool) -> zbus::fdo::RequestNameFlags {
    let flags = name_flags(defers);
    if defers {
        flags
    } else {
        flags.difference(zbus::fdo::RequestNameFlags::DO_NOT_QUEUE)
    }
}

/// Serves `org.freedesktop.ScreenSaver` on both its paths and asks for the name.
async fn host_screensaver(
    conn: &Connection,
    shared: &screensaver::Shared,
    defers: bool,
) -> Result<(), String> {
    for path in SCREENSAVER_PATHS {
        conn.object_server()
            .at(path, screensaver::ScreenSaver(shared.clone()))
            .await
            .map_err(|e| format!("object server (screensaver): {e}"))?;
    }
    match own(conn, SCREENSAVER_NAME, screensaver_flags(defers)).await {
        Ok(_) => log::info!("screensaver: hosting {SCREENSAVER_NAME}"),
        Err(e) => log::info!("screensaver: {SCREENSAVER_NAME}: {e}; inhibits go to its owner"),
    }
    Ok(())
}

async fn own(
    conn: &Connection,
    name: &str,
    flags: zbus::fdo::RequestNameFlags,
) -> Result<(), String> {
    use zbus::fdo::RequestNameReply;
    match conn.request_name_with_flags(name, flags).await {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => Ok(()),
        Ok(reply) => Err(format!("{reply:?}")),
        Err(e) => Err(e.to_string()),
    }
}

/// What outlives a connection to the bus: the compositor's side, and what clients set up
/// through the services.
struct Kept {
    events: Sender<TrayEvent>,
    eventfd: RawFd,
    commands: async_channel::Receiver<TrayCommand>,
    appearance: Arc<Mutex<Appearance>>,
    icons: Icons,
    sizes: Sizes,
    pads: crate::tabletpad::Shared,
    senders: Arc<Mutex<Senders>>,
    programs: Arc<Mutex<Programs>>,
    responding: Arc<Mutex<Responding>>,
    identities: Arc<Identities>,
    screensaver: screensaver::Shared,
    /// So a notification's id is never reused by a later connection.
    next_id: Arc<AtomicU32>,
    defers: bool,
}

/// Connect to the session bus, or the bus at `address`, and serve the tray's services on it.
async fn setup(kept: &Kept, address: Option<&str>) -> Result<Loop, String> {
    let (events, eventfd) = (kept.events.clone(), kept.eventfd);
    let flags = name_flags(kept.defers);
    let (icons, senders, identities) = (
        kept.icons.clone(),
        kept.senders.clone(),
        kept.identities.clone(),
    );
    let builder = match address {
        Some(address) => zbus::connection::Builder::address(address),
        None => zbus::connection::Builder::session(),
    };
    let conn = builder
        .method_timeout(METHOD_TIMEOUT)
        .build()
        .await
        .map_err(|e| format!("session bus: {e}"))?;
    let mut streams: SelectAll<BoxStream<'static, Wake>> = SelectAll::new();
    streams.push(kept.commands.clone().map(Wake::Command).boxed());
    let closing = conn.clone();
    let lost = async move {
        closing.closed().await;
        Wake::Lost
    };
    streams.push(futures_util::stream::once(lost).boxed());

    let dbus = Proxy::new(&conn, DBUS_IFACE, "/org/freedesktop/DBus", DBUS_IFACE)
        .await
        .map_err(|e| format!("DBus proxy: {e}"))?;
    if let Some(s) = signal_stream(&dbus, "NameOwnerChanged", |m| {
        let (name, _old, new_owner): (String, String, String) = m.body().deserialize().ok()?;
        Some(Wake::Owner { name, new_owner })
    })
    .await
    {
        streams.push(s);
    }

    let host_name = format!("org.kde.StatusNotifierHost-{}", std::process::id());
    let (reg_tx, reg_rx) = async_channel::unbounded::<Wake>();
    let published = Arc::new(Mutex::new(Vec::new()));
    let watcher = Watcher {
        items: published.clone(),
        registrations: reg_tx,
    };
    conn.object_server()
        .at(WATCHER_PATH, watcher)
        .await
        .map_err(|e| format!("object server: {e}"))?;
    // Also host the freedesktop notifications service; if another daemon owns
    // the name we never get Notify calls.
    conn.object_server()
        .at(
            NOTIFY_PATH,
            Notifications {
                events: events.clone(),
                eventfd,
                next_id: kept.next_id.clone(),
                senders: senders.clone(),
                programs: kept.programs.clone(),
                icons: icons.clone(),
                identities: identities.clone(),
            },
        )
        .await
        .map_err(|e| format!("object server (notifications): {e}"))?;
    match own(&conn, NOTIFY_NAME, flags).await {
        Ok(_) => log::info!("notifications: hosting {NOTIFY_NAME}"),
        Err(_) => log::info!("notifications: {NOTIFY_NAME} already owned; balloons off"),
    }
    let screensaver = kept.screensaver.clone();
    host_screensaver(&conn, &screensaver, kept.defers).await?;
    let current = kept.appearance.clone();
    conn.object_server()
        .at(
            SETTINGS_PATH,
            Settings {
                current: current.clone(),
            },
        )
        .await
        .map_err(|e| format!("object server (settings): {e}"))?;
    match own(&conn, SETTINGS_NAME, flags).await {
        Ok(_) => log::info!("settings portal: hosting {SETTINGS_NAME}"),
        Err(e) => {
            log::warn!("settings portal: {SETTINGS_NAME}: {e}; dark mode stays unknown to portals")
        }
    }
    let responding = kept.responding.clone();
    conn.object_server()
        .at(
            reprisebus::tray::PATH,
            Shell {
                events: events.clone(),
                eventfd,
                pads: kept.pads.clone(),
                programs: kept.programs.clone(),
                responding: responding.clone(),
            },
        )
        .await
        .map_err(|e| format!("object server (tray): {e}"))?;
    if let Err(e) = own(&conn, reprisebus::tray::NAME, flags).await {
        log::warn!(
            "tray: {}: {e}; Customize... reaches nobody",
            reprisebus::tray::NAME
        );
    }
    let own_watcher = own(&conn, WATCHER_NAME, flags).await.is_ok();
    conn.request_name(host_name.as_str())
        .await
        .map_err(|e| format!("host name: {e}"))?;

    let (published, idle_watcher) = if own_watcher {
        log::info!("tray: hosting {WATCHER_NAME}");
        streams.push(reg_rx.boxed());
        (Some(published), None)
    } else {
        log::info!("tray: joining the existing {WATCHER_NAME}");
        let foreign = Proxy::new(&conn, WATCHER_NAME, WATCHER_PATH, WATCHER_IFACE)
            .await
            .map_err(|e| format!("foreign watcher: {e}"))?;
        if let Some(s) = signal_stream(&foreign, "StatusNotifierItemRegistered", |m| {
            let s: String = m.body().deserialize().ok()?;
            Some(Wake::ForeignRegistered(s))
        })
        .await
        {
            streams.push(s);
        }
        if let Some(s) = signal_stream(&foreign, "StatusNotifierItemUnregistered", |m| {
            let s: String = m.body().deserialize().ok()?;
            Some(Wake::ForeignUnregistered(s))
        })
        .await
        {
            streams.push(s);
        }
        let existing: Vec<String> =
            reprisebus::bounded::property(&foreign, "RegisteredStatusNotifierItems")
                .await
                .unwrap_or_default();
        for s in existing {
            let (bus, path) = split_service(&s);
            push_known_item(&mut streams, bus, path);
        }
        (None, Some((published, reg_rx)))
    };
    let watcher_proxy = Proxy::new(&conn, WATCHER_NAME, WATCHER_PATH, WATCHER_IFACE)
        .await
        .map_err(|e| format!("watcher proxy: {e}"))?;
    let _ = watcher_proxy
        .call_method("RegisterStatusNotifierHost", &(host_name.as_str(),))
        .await;

    Ok(Loop {
        conn,
        events,
        eventfd,
        items: HashMap::new(),
        fetching: HashMap::new(),
        next_fetch: 0,
        item_stops: HashMap::new(),
        changes: HashMap::new(),
        muted: HashSet::new(),
        muted_for: MUTED_FOR,
        published,
        idle_watcher,
        screensaver,
        streams,
        appearance: current,
        icons,
        sizes: kept.sizes.clone(),
        senders,
        responding,
        identities,
    })
}

fn push_known_item(streams: &mut SelectAll<BoxStream<'static, Wake>>, bus: String, path: String) {
    let once = futures_util::stream::once(std::future::ready(Wake::Registered { bus, path }));
    streams.push(once.boxed());
}

/// When the tray connects again after losing its connection: at first soon, then backing
/// off while connections keep dying within [`STEADY`] of being made.
const RECONNECT_FIRST: Duration = Duration::from_secs(1);
const RECONNECT_MOST: Duration = Duration::from_secs(30);
const STEADY: Duration = Duration::from_secs(60);

/// Serve the tray on the session bus, or the bus at `address`, connecting again whenever the
/// connection is lost, until a shutdown or a first connection that can't be set up.
async fn serve(kept: &Kept, address: Option<&str>) {
    let mut pause = Duration::ZERO;
    let mut bus = None;
    let mut registered = Vec::new();
    loop {
        let began = Instant::now();
        match setup(kept, address).await {
            Ok(mut lp) => {
                let guid = lp.conn.server_guid().clone();
                if let Some(was) = bus.replace(guid.clone()) {
                    lp.forget_gone(was == guid).await;
                    lp.readmit(std::mem::take(&mut registered));
                }
                match lp.run().await {
                    Ended::Shutdown => return,
                    Ended::Lost(items) => registered = items,
                }
                log::warn!("tray: lost the session bus connection");
            }
            Err(e) if bus.is_none() => {
                emit(&kept.events, kept.eventfd, TrayEvent::Unavailable(e));
                return;
            }
            Err(e) => log::warn!("tray: could not serve on the session bus again: {e}"),
        }
        pause = match pause.is_zero() || began.elapsed() >= STEADY {
            true => RECONNECT_FIRST,
            false => (pause * 2).min(RECONNECT_MOST),
        };
        log::warn!("tray: connecting again in {pause:?}");
        async_io::Timer::after(pause).await;
    }
}

/// Start the D-Bus thread. `None` means it could not be spawned; a later bus failure arrives as
/// `TrayEvent::Unavailable`.
pub fn start(
    events: Sender<TrayEvent>,
    eventfd: OwnedFd,
    appearance: Appearance,
    icon_theme: String,
    pads: crate::tabletpad::Shared,
    icon_sizes: Vec<i32>,
    defers: bool,
) -> Option<(async_channel::Sender<TrayCommand>, JoinHandle<()>)> {
    let (tx, rx) = async_channel::unbounded::<TrayCommand>();
    let handle = std::thread::Builder::new()
        .name("tray-dbus".into())
        .spawn(move || {
            // The thread owns its copy of the eventfd, so the loop side can close its own without
            // waiting for a stalled peer.
            let fd = eventfd.as_raw_fd();
            let kept = Kept {
                events: events.clone(),
                eventfd: fd,
                commands: rx,
                appearance: Arc::new(Mutex::new(appearance)),
                icons: Arc::new(Mutex::new(load_icons(&icon_theme))),
                sizes: Arc::new(Mutex::new(icon_sizes)),
                pads,
                senders: Arc::default(),
                programs: Arc::default(),
                responding: Arc::default(),
                identities: Arc::default(),
                screensaver: screensaver::Shared::new(events, fd),
                next_id: Arc::new(AtomicU32::new(1)),
                defers,
            };
            zbus::block_on(serve(&kept, None));
            drop(eventfd);
        })
        .ok()?;
    Some((tx, handle))
}
