//! The tray's D-Bus side, on its own thread.

use super::screensaver::{self, SCREENSAVER_NAME, SCREENSAVER_PATHS};
use super::settings::{self, Appearance, SETTINGS_NAME, SETTINGS_PATH, Settings};
use super::{TrayCommand, TrayEvent, TrayItem};
use crate::icontheme::{IconCache, IconTheme};
use crate::mutex::lock;
use futures_util::stream::{BoxStream, SelectAll, StreamExt};
use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use zbus::object_server::InterfaceRef;
use zbus::proxy::CacheProperties;
use zbus::{Connection, Proxy};

mod item;
mod menu;
mod notifications;
#[cfg(test)]
mod tests;
mod watcher;

use item::{call_item, decode_names, fetch_item, provide_token};
use menu::{get_layout, send_menu_event};
use notifications::{Notifications, Senders};
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
}

#[zbus::interface(name = "org.reprise.Tray")]
impl Shell {
    async fn customize(&self) {
        emit(&self.events, self.eventfd, TrayEvent::Customize);
    }

    async fn critical_turn_off(&self) {
        emit(&self.events, self.eventfd, TrayEvent::CriticalTurnOff);
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
    Fetched {
        key: String,
        fetch: u64,
        item: Option<Box<TrayItem>>,
    },
}

/// How long a change signal waits for its read, so a burst of them costs one read.
const SETTLE: Duration = Duration::from_millis(50);

struct Fetch {
    /// Tells its result from that of a fetch dropped with a removed item.
    id: u64,
    /// Set as the read begins: a change before that is still read by it.
    reading: Arc<AtomicBool>,
    /// A change arrived during the read and needs a read of its own.
    again: bool,
}

struct Loop {
    conn: Connection,
    events: Sender<TrayEvent>,
    eventfd: RawFd,
    items: HashMap<String, TrayItem>,
    fetching: HashMap<String, Fetch>,
    next_fetch: u64,
    /// Dropping an item's sender ends its signal streams, and their match rules with them.
    item_stops: HashMap<String, async_channel::Sender<()>>,
    published: Option<Arc<Mutex<Vec<Published>>>>,
    screensaver: screensaver::Shared,
    streams: SelectAll<BoxStream<'static, Wake>>,
    appearance: Arc<Mutex<Appearance>>,
    icons: Icons,
    sizes: Sizes,
    senders: Arc<Mutex<Senders>>,
}

impl Loop {
    fn emit(&self, ev: TrayEvent) {
        emit(&self.events, self.eventfd, ev);
    }

    /// The watch comes first, so a change the item makes while being read is read as well.
    async fn track(&mut self, bus: String, path: String) {
        self.watch_item(&bus, &path).await;
        self.fetch(bus, path, Duration::ZERO);
    }

    /// Subscribe to [`ITEM_SIGNALS`] on an item, unless already watching it; each fires
    /// `ItemChanged`.
    async fn watch_item(&mut self, bus: &str, path: &str) {
        let key = format!("{bus}{path}");
        if self.item_stops.contains_key(&key) {
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

    /// Run `call` beside the loop, so a peer that never answers holds up only its own call.
    fn spawn(&mut self, call: impl Future<Output = ()> + Send + 'static) {
        let done = futures_util::stream::once(call).filter_map(|()| std::future::ready(None));
        self.streams.push(pumped(done));
    }

    fn fetch(&mut self, bus: String, path: String, settle: Duration) {
        let key = format!("{bus}{path}");
        if let Some(f) = self.fetching.get_mut(&key) {
            f.again |= f.reading.load(Ordering::Relaxed);
            return;
        }
        let fetch = self.next_fetch;
        self.next_fetch += 1;
        let reading = Arc::new(AtomicBool::new(false));
        self.fetching.insert(
            key.clone(),
            Fetch {
                id: fetch,
                reading: reading.clone(),
                again: false,
            },
        );
        let (conn, icons, sizes) = (self.conn.clone(), self.icons.clone(), self.sizes.clone());
        let read = async move {
            if !settle.is_zero() {
                async_io::Timer::after(settle).await;
            }
            reading.store(true, Ordering::Relaxed);
            let item = fetch_item(&conn, &icons, &sizes, &bus, &path)
                .await
                .map(Box::new);
            Wake::Fetched { key, fetch, item }
        };
        self.streams.push(pumped(futures_util::stream::once(read)));
    }

    async fn fetched(&mut self, key: String, fetch: u64, item: Option<Box<TrayItem>>) {
        if self.fetching.get(&key).is_none_or(|f| f.id != fetch) {
            return;
        }
        let again = self.fetching.remove(&key).is_some_and(|f| f.again);
        match item {
            Some(item) if self.items.get(&key) == Some(item.as_ref()) => {}
            Some(item) => {
                let item = *item;
                let existed = self.items.insert(key.clone(), item.clone()).is_some();
                self.emit(if existed {
                    TrayEvent::Updated(item)
                } else {
                    TrayEvent::Added(item)
                });
            }
            None if self.items.contains_key(&key) => {}
            None => {
                log::warn!("tray: could not read item {key}");
                // Registered but unreadable: never advertise it as one of ours,
                // and stop watching it unless a change asked for another read.
                if !again {
                    self.item_stops.remove(&key);
                }
                self.unpublish(&key).await;
            }
        }
        if again {
            let (bus, path) = split_service(&key);
            self.fetch(bus, path, SETTLE);
        }
    }

    fn refresh_item(&mut self, key: &str) {
        if self.items.contains_key(key) || self.fetching.contains_key(key) {
            let (bus, path) = split_service(key);
            self.fetch(bus, path, SETTLE);
        }
    }

    async fn remove_item(&mut self, key: &str) {
        let was_fetching = self.fetching.remove(key).is_some();
        self.item_stops.remove(key);
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
                if let Ok(iface) = self.notifications().await {
                    lock(&self.senders).0.remove(&id);
                    let _ = Notifications::notification_closed(iface.signal_emitter(), id, reason)
                        .await;
                }
            }
            TrayCommand::Replied { id, text } => {
                if let Ok(iface) = self.notifications().await {
                    let emitter = iface.signal_emitter();
                    let _ = Notifications::notification_replied(emitter, id, &text).await;
                }
            }
            TrayCommand::ActionInvoked { id, key, token } => {
                if let Ok(iface) = self.notifications().await {
                    let emitter = iface.signal_emitter();
                    if let Some(token) = token {
                        let _ = Notifications::activation_token(emitter, id, &token).await;
                    }
                    let _ = Notifications::action_invoked(emitter, id, &key).await;
                }
            }
            TrayCommand::Appearance(a) => settings::update(&self.conn, &self.appearance, a).await,
            TrayCommand::IconTheme(name) => self.retheme(&name),
            TrayCommand::IconSizes(sizes) => self.resize_icons(sizes),
            TrayCommand::Shutdown => return false,
        }
        true
    }

    /// Reads every item again, for `IconPixmap` entries and named icons at `sizes`.
    fn resize_icons(&mut self, sizes: Vec<i32>) {
        *lock(&self.sizes) = sizes;
        let keys: Vec<String> = self.items.keys().cloned().collect();
        for key in keys {
            self.refresh_item(&key);
        }
    }

    fn retheme(&mut self, name: &str) {
        *lock(&self.icons) = load_icons(name);
        let sizes = sizes_now(&self.sizes);
        for item in self.items.values_mut() {
            if decode_names(&self.icons, item, &sizes) {
                emit(&self.events, self.eventfd, TrayEvent::Updated(item.clone()));
            }
        }
    }

    async fn run(mut self) {
        while let Some(wake) = self.streams.next().await {
            match wake {
                Wake::Command(cmd) => {
                    if !self.handle_command(cmd).await {
                        break;
                    }
                }
                Wake::Registered { bus, path } => self.track(bus, path).await,
                Wake::ForeignRegistered(s) => {
                    let (bus, path) = split_service(&s);
                    self.track(bus, path).await;
                }
                Wake::ForeignUnregistered(s) => {
                    let (bus, path) = split_service(&s);
                    self.remove_item(&format!("{bus}{path}")).await;
                }
                Wake::ItemChanged(key) => self.refresh_item(&key),
                Wake::Fetched { key, fetch, item } => self.fetched(key, fetch, item).await,
                Wake::Owner { name, new_owner } => {
                    if new_owner.is_empty() {
                        self.remove_items_on(&name).await;
                        if lock(&self.senders).sent_by(&name) {
                            self.emit(TrayEvent::NotifySenderGone(name.clone()));
                        }
                        let dropped = self.screensaver.update(|h| h.forget(&name));
                        if dropped > 0 {
                            log::info!(
                                "screensaver: {name} left the bus; {dropped} cookie(s) dropped"
                            );
                        }
                    }
                }
            }
        }
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

async fn setup(
    events: Sender<TrayEvent>,
    eventfd: RawFd,
    commands: async_channel::Receiver<TrayCommand>,
    appearance: Appearance,
    icon_theme: String,
    pads: crate::tabletpad::Shared,
    sizes: Vec<i32>,
) -> Result<Loop, String> {
    let icons: Icons = Arc::new(Mutex::new(load_icons(&icon_theme)));
    let senders: Arc<Mutex<Senders>> = Arc::default();
    let conn = zbus::connection::Builder::session()
        .method_timeout(METHOD_TIMEOUT)
        .build()
        .await
        .map_err(|e| format!("session bus: {e}"))?;
    let mut streams: SelectAll<BoxStream<'static, Wake>> = SelectAll::new();
    streams.push(commands.map(Wake::Command).boxed());

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
                next_id: std::sync::atomic::AtomicU32::new(1),
                senders: senders.clone(),
                icons: icons.clone(),
            },
        )
        .await
        .map_err(|e| format!("object server (notifications): {e}"))?;
    match conn.request_name(NOTIFY_NAME).await {
        Ok(_) => log::info!("notifications: hosting {NOTIFY_NAME}"),
        Err(_) => log::info!("notifications: {NOTIFY_NAME} already owned; balloons off"),
    }
    let screensaver = screensaver::Shared::new(events.clone(), eventfd);
    for path in SCREENSAVER_PATHS {
        conn.object_server()
            .at(path, screensaver::ScreenSaver(screensaver.clone()))
            .await
            .map_err(|e| format!("object server (screensaver): {e}"))?;
    }
    match conn.request_name(SCREENSAVER_NAME).await {
        Ok(_) => log::info!("screensaver: hosting {SCREENSAVER_NAME}"),
        Err(e) => log::info!("screensaver: {SCREENSAVER_NAME}: {e}; inhibits go to its owner"),
    }
    let current = Arc::new(Mutex::new(appearance));
    conn.object_server()
        .at(
            SETTINGS_PATH,
            Settings {
                current: current.clone(),
            },
        )
        .await
        .map_err(|e| format!("object server (settings): {e}"))?;
    match conn.request_name(SETTINGS_NAME).await {
        Ok(_) => log::info!("settings portal: hosting {SETTINGS_NAME}"),
        Err(e) => {
            log::warn!("settings portal: {SETTINGS_NAME}: {e}; dark mode stays unknown to portals")
        }
    }
    conn.object_server()
        .at(
            reprisebus::tray::PATH,
            Shell {
                events: events.clone(),
                eventfd,
                pads,
            },
        )
        .await
        .map_err(|e| format!("object server (tray): {e}"))?;
    if let Err(e) = conn.request_name(reprisebus::tray::NAME).await {
        log::warn!(
            "tray: {}: {e}; Customize... reaches nobody",
            reprisebus::tray::NAME
        );
    }
    let own_watcher = conn.request_name(WATCHER_NAME).await.is_ok();
    conn.request_name(host_name.as_str())
        .await
        .map_err(|e| format!("host name: {e}"))?;

    let published = if own_watcher {
        log::info!("tray: hosting {WATCHER_NAME}");
        streams.push(reg_rx.boxed());
        Some(published)
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
        let existing: Vec<String> = foreign
            .get_property("RegisteredStatusNotifierItems")
            .await
            .unwrap_or_default();
        for s in existing {
            let (bus, path) = split_service(&s);
            push_known_item(&mut streams, bus, path);
        }
        None
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
        published,
        screensaver,
        streams,
        appearance: current,
        icons,
        sizes: Arc::new(Mutex::new(sizes)),
        senders,
    })
}

fn push_known_item(streams: &mut SelectAll<BoxStream<'static, Wake>>, bus: String, path: String) {
    let once = futures_util::stream::once(std::future::ready(Wake::Registered { bus, path }));
    streams.push(once.boxed());
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
) -> Option<(async_channel::Sender<TrayCommand>, JoinHandle<()>)> {
    let (tx, rx) = async_channel::unbounded::<TrayCommand>();
    let handle = std::thread::Builder::new()
        .name("tray-dbus".into())
        .spawn(move || {
            // The thread owns its copy of the eventfd, so the loop side can close its own without
            // waiting for a stalled peer.
            let fd = eventfd.as_raw_fd();
            zbus::block_on(async move {
                match setup(
                    events.clone(),
                    fd,
                    rx,
                    appearance,
                    icon_theme,
                    pads,
                    icon_sizes,
                )
                .await
                {
                    Ok(l) => l.run().await,
                    Err(e) => emit(&events, fd, TrayEvent::Unavailable(e)),
                }
            });
            drop(eventfd);
        })
        .ok()?;
    Some((tx, handle))
}
