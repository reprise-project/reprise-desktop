//! The tray's D-Bus side, on its own thread.

use super::screensaver::{self, SCREENSAVER_NAME, SCREENSAVER_PATHS};
use super::settings::{self, Appearance, SETTINGS_NAME, SETTINGS_PATH, Settings};
use super::{
    DEFAULT_PROP, MenuNode, Notification, Orientation, TrayCommand, TrayEvent, TrayItem,
    decode_named, item_pixmaps, notify,
};
use crate::icontheme::{IconCache, IconTheme};
use futures_util::stream::{BoxStream, SelectAll, StreamExt};
use reprisekit::controls::tooltip::TIP_CHARS;
use reprisekit::icon::IconSource;
use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use zbus::message::{Body, Header};
use zbus::object_server::SignalEmitter;
use zbus::proxy::CacheProperties;
use zbus::zvariant::{DeserializeValue, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, Proxy};

const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_IFACE: &str = "org.kde.StatusNotifierWatcher";
const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
/// Item signals that change what `fetch_item` reads; no `NewAttentionIcon`, as no attention icon
/// is read.
const ITEM_SIGNALS: [&str; 5] = ["NewIcon", "NewTitle", "NewStatus", "NewToolTip", "NewMenu"];
const MENU_IFACE: &str = "com.canonical.dbusmenu";
const DBUS_IFACE: &str = "org.freedesktop.DBus";
const NOTIFY_NAME: &str = "org.freedesktop.Notifications";
const NOTIFY_PATH: &str = "/org/freedesktop/Notifications";

/// A peer's menu is cut to these as it is parsed, so label widths and the menu canvas stay
/// bounded.
const LABEL_CHARS: usize = 255;
const MENU_ROWS: usize = 256;

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

/// `service` as RegisterStatusNotifierItem passes it: an object path on the caller's bus name, a
/// bus name at the well-known path, or a bus name followed by the path.
fn resolve_registration(service: &str, sender: Option<&str>) -> Option<(String, String)> {
    if service.starts_with('/') {
        Some((sender?.to_string(), service.to_string()))
    } else if service.is_empty() {
        None
    } else {
        Some(split_service(service))
    }
}

fn split_service(service: &str) -> (String, String) {
    match service.find('/') {
        Some(i) => (service[..i].to_string(), service[i..].to_string()),
        None => (service.to_string(), "/StatusNotifierItem".to_string()),
    }
}

/// Each item costs property round-trips and a match rule per [`ITEM_SIGNALS`] entry, and the bus
/// daemon caps rules per connection.
const MAX_REGISTRATIONS: usize = 64;
const MAX_REGISTRATIONS_PER_SENDER: usize = 8;

struct Published {
    key: String,
    sender: String,
}

#[derive(Debug, PartialEq)]
enum Admission {
    Known,
    New,
    TotalFull,
    SenderFull,
}

fn admit(published: &[Published], key: &str, sender: &str) -> Admission {
    if published.iter().any(|p| p.key == key) {
        return Admission::Known;
    }
    if published.len() >= MAX_REGISTRATIONS {
        return Admission::TotalFull;
    }
    if published.iter().filter(|p| p.sender == sender).count() >= MAX_REGISTRATIONS_PER_SENDER {
        return Admission::SenderFull;
    }
    Admission::New
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

    async fn tablet_pads(&self) -> Vec<(String, u32, u32, u32)> {
        let pads = self.pads.lock().unwrap_or_else(|e| e.into_inner());
        (pads.iter())
            .map(|p| (p.name.clone(), p.buttons, p.rings, p.strips))
            .collect()
    }
}

struct Watcher {
    items: Arc<Mutex<Vec<Published>>>,
    registrations: async_channel::Sender<Wake>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let Some(sender) = hdr.sender().map(|s| s.as_str()) else {
            return;
        };
        let Some((bus, path)) = resolve_registration(service, Some(sender)) else {
            log::warn!("tray: ignoring item registration {service:?}");
            return;
        };
        let key = format!("{bus}{path}");
        {
            let mut items = self.items.lock().unwrap_or_else(|e| e.into_inner());
            match admit(&items, &key, sender) {
                Admission::Known => {}
                Admission::New => items.push(Published {
                    key: key.clone(),
                    sender: sender.to_string(),
                }),
                Admission::TotalFull => {
                    log::warn!("tray: refusing item {key}: {MAX_REGISTRATIONS} already registered");
                    return;
                }
                Admission::SenderFull => {
                    log::warn!(
                        "tray: refusing item {key}: {sender} already registered {MAX_REGISTRATIONS_PER_SENDER}"
                    );
                    return;
                }
            }
        }
        let _ = self
            .registrations
            .send(Wake::Registered { bus, path })
            .await;
        let _ = Self::status_notifier_item_registered(&emitter, &key).await;
    }

    async fn register_status_notifier_host(
        &self,
        _service: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let _ = Self::status_notifier_host_registered(&emitter).await;
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        let items = self.items.lock().unwrap_or_else(|e| e.into_inner());
        items.iter().map(|p| p.key.clone()).collect()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// Connection and app name per open notification. A replace is honoured from either (`notify-send
/// -r` is a new connection each time), a close only from the connection.
#[derive(Default)]
struct Senders(HashMap<u32, (String, String)>);

impl Senders {
    fn claim(
        &mut self,
        sender: &str,
        app: &str,
        replaces: u32,
        fresh: impl FnOnce() -> u32,
    ) -> u32 {
        let replaceable = self
            .0
            .get(&replaces)
            .is_some_and(|(s, a)| s == sender || (!app.is_empty() && a == app));
        let id = if replaceable { replaces } else { fresh() };
        self.0.insert(id, (sender.to_string(), app.to_string()));
        id
    }

    fn owns(&self, id: u32, sender: &str) -> bool {
        self.0.get(&id).is_some_and(|(s, _)| s == sender)
    }

    fn sent_by(&self, sender: &str) -> bool {
        self.0.values().any(|(s, _)| s == sender)
    }
}

fn lock_senders(senders: &Mutex<Senders>) -> std::sync::MutexGuard<'_, Senders> {
    senders.lock().unwrap_or_else(|e| e.into_inner())
}

struct Notifications {
    events: Sender<TrayEvent>,
    eventfd: RawFd,
    next_id: std::sync::atomic::AtomicU32,
    senders: Arc<Mutex<Senders>>,
    icons: Icons,
}

const CAPABILITIES: [&str; 6] = [
    "actions",
    "body",
    "body-markup",
    "icon-static",
    "persistence",
    "inline-reply",
];

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    #[expect(clippy::too_many_arguments)]
    async fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, notify::Hint<'_>>,
        expire_timeout: i32,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
    ) -> u32 {
        let sender = hdr.sender().map_or("", |s| s.as_str()).to_string();
        let id = lock_senders(&self.senders).claim(
            &sender,
            &app_name,
            replaces_id,
            // Ids start at 1; 0 is "none" in the protocol.
            || self.next_id.fetch_add(1, Ordering::Relaxed).max(1),
        );
        let pid = owner_pid(conn, &sender).await;
        let hints = notify::hints(hints);
        let entry = hints.desktop_entry.as_deref().and_then(desktop_entry);
        let icon = notification_icon(&self.icons, &hints, app_icon, entry.as_ref());
        let app = entry.map_or(app_name, |e| e.name);
        let actions = notify::actions(&actions);
        let reply = actions
            .iter()
            .any(|(k, _)| k == notify::INLINE_REPLY)
            .then(|| super::Reply {
                placeholder: super::clip(
                    hints.reply_placeholder.unwrap_or_default(),
                    notify::ACTION_LABEL_CHARS,
                ),
                send: super::clip(
                    hints.reply_send.unwrap_or_else(|| super::REPLY_SEND.into()),
                    notify::ACTION_LABEL_CHARS,
                ),
            });
        emit(
            &self.events,
            self.eventfd,
            TrayEvent::Notify(Notification {
                id,
                app: super::clip(app, super::SUMMARY_CHARS),
                summary: super::clip(summary, super::SUMMARY_CHARS),
                body: super::clip(notify::plain_text(&body), super::BODY_CHARS),
                icon,
                expire_timeout,
                actions,
                urgency: hints.urgency,
                transient: hints.transient,
                resident: hints.resident,
                sender,
                pid,
                reply,
            }),
        );
        id
    }

    async fn close_notification(&self, id: u32, #[zbus(header)] hdr: Header<'_>) {
        let sender = hdr.sender().map_or("", |s| s.as_str());
        if lock_senders(&self.senders).owns(id, sender) {
            emit(&self.events, self.eventfd, TrayEvent::CloseRequest(id));
        }
    }

    fn get_capabilities(&self) -> Vec<String> {
        CAPABILITIES.iter().map(|c| c.to_string()).collect()
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "reprise".to_string(),
            "reprise".to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
            "1.2".to_string(),
        )
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn notification_replied(
        emitter: &SignalEmitter<'_>,
        id: u32,
        text: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn activation_token(
        emitter: &SignalEmitter<'_>,
        id: u32,
        activation_token: &str,
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
        .destination(bus.to_string())?
        .path(path.to_string())?
        .interface(interface)?
        .cache_properties(CacheProperties::No)
        .build()
        .await
}

async fn prop<T>(proxy: &Proxy<'_>, name: &str) -> Option<T>
where
    T: TryFrom<OwnedValue>,
    T::Error: Into<zbus::Error>,
{
    proxy.get_property::<T>(name).await.ok()
}

type Pixmaps<'a> = Vec<(i32, i32, &'a [u8])>;

type ToolTip<'a> = (&'a str, Pixmaps<'a>, &'a str, &'a str);

/// One property's `Properties.Get` reply body, for decoding in place with [`DeserializeValue`].
/// [`prop`]'s `OwnedValue` holds each `ay` byte as its own `Value`: gigabytes for a 128 MiB pixmap.
async fn prop_body(proxy: &Proxy<'_>, name: &str) -> Option<Body> {
    let reply = proxy
        .connection()
        .call_method(
            Some(proxy.destination()),
            proxy.path(),
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(proxy.interface(), name),
        )
        .await
        .ok()?;
    Some(reply.body())
}

type Icons = Arc<Mutex<IconCache>>;

fn lock(icons: &Icons) -> std::sync::MutexGuard<'_, IconCache> {
    icons.lock().unwrap_or_else(|e| e.into_inner())
}

fn item_named(icons: &Icons, item: &TrayItem) -> Arc<[IconSource]> {
    match item.icon_name.as_deref() {
        Some(name) if item.pixmaps.is_empty() => {
            let extra: Vec<PathBuf> = item.icon_theme_path.iter().cloned().collect();
            decode_named(&mut lock(icons), name, &extra).into()
        }
        _ => Arc::from([]),
    }
}

/// The name keys the icon cache, so the `PATH_MAX` cap bounds it.
fn icon_ref(name: Option<String>) -> Option<String> {
    name.filter(|s| !s.is_empty() && s.len() <= libc::PATH_MAX as usize)
}

fn desktop_entry(hint: &str) -> Option<reprisefs::apps::AppEntry> {
    let id = notify::desktop_id(hint)?;
    let path = reprisefs::apps::find_file(&id)?;
    let text = reprisefs::apps::read_desktop_file(&path).ok()?;
    reprisefs::apps::parse_desktop(&text, &id)
}

fn notification_icon(
    icons: &Icons,
    hints: &notify::Hints,
    app_icon: String,
    entry: Option<&reprisefs::apps::AppEntry>,
) -> Vec<IconSource> {
    if let Some(img) = &hints.image {
        let sources = notify::image_sources(img);
        if !sources.is_empty() {
            return sources;
        }
    }
    let path = hints
        .image_path
        .as_deref()
        .map(|p| match p.strip_prefix("file://") {
            Some(rest) => {
                String::from_utf8_lossy(&reprisefs::trash::percent_decode(rest)).into_owned()
            }
            None => p.to_string(),
        });
    let entry_icon = entry.and_then(|e| e.icon.clone());
    [path, Some(app_icon), entry_icon]
        .into_iter()
        .filter_map(icon_ref)
        .map(|name| decode_named(&mut lock(icons), &name, &[]))
        .find(|sources| !sources.is_empty())
        .unwrap_or_default()
}

fn item_texts(title: Option<String>, tip: Option<ToolTip>) -> (String, String) {
    let title = super::clip(title.unwrap_or_default(), TIP_CHARS);
    let tooltip = tip
        .map(|t| t.2)
        .filter(|s| !s.trim().is_empty())
        .map_or_else(|| title.clone(), |s| s.chars().take(TIP_CHARS).collect());
    (title, tooltip)
}

async fn fetch_item(conn: &Connection, icons: &Icons, bus: &str, path: &str) -> Option<TrayItem> {
    let proxy = uncached_proxy(conn, bus, path, ITEM_IFACE).await.ok()?;
    // Concurrent, so a silent peer costs one METHOD_TIMEOUT, not one per property.
    let (sni_id, title, icon_name, theme_path, pixmaps, menu, status, tip, pid) = futures_util::join!(
        prop::<String>(&proxy, "Id"),
        prop::<String>(&proxy, "Title"),
        prop::<String>(&proxy, "IconName"),
        prop::<String>(&proxy, "IconThemePath"),
        prop_body(&proxy, "IconPixmap"),
        prop::<OwnedObjectPath>(&proxy, "Menu"),
        prop::<String>(&proxy, "Status"),
        prop_body(&proxy, "ToolTip"),
        owner_pid(conn, bus),
    );
    let pixmaps = pixmaps
        .as_ref()
        .and_then(|b| b.deserialize::<DeserializeValue<Pixmaps>>().ok())
        .map(|v| v.0);
    let tip = tip
        .as_ref()
        .and_then(|b| b.deserialize::<DeserializeValue<ToolTip>>().ok())
        .map(|v| v.0);
    // Not one property answered: the peer is gone, or never was an SNI.
    // `IconThemePath` is left out, plenty of well-behaved items omit it.
    if title.is_none() && icon_name.is_none() && pixmaps.is_none() && menu.is_none() {
        log::warn!("tray: {bus}{path} answered none of Title/IconName/IconPixmap/Menu");
        return None;
    }
    let (title, tooltip) = item_texts(title, tip);
    let key = item_key(
        sni_id.as_deref(),
        pid.and_then(program_name).as_deref(),
        bus,
    );
    let mut item = TrayItem {
        id: format!("{bus}{path}"),
        key,
        service: bus.to_string(),
        path: path.to_string(),
        title,
        tooltip,
        icon_name: icon_ref(icon_name),
        icon_theme_path: icon_ref(theme_path).map(PathBuf::from),
        pixmaps: item_pixmaps(&pixmaps.unwrap_or_default()).into(),
        named: Arc::from([]),
        menu_path: menu.map(|p| p.as_str().to_string()).filter(|p| p != "/"),
        passive: status.is_some_and(|s| s.eq_ignore_ascii_case("Passive")),
        pid,
    };
    item.named = item_named(icons, &item);
    Some(item)
}

fn item_key(sni_id: Option<&str>, program: Option<&str>, bus: &str) -> String {
    let key = [sni_id, program]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or(bus);
    key.chars().take(TIP_CHARS).collect()
}

fn program_name(pid: i32) -> Option<String> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(comm.trim_end().to_string()).filter(|s| !s.is_empty())
}

async fn owner_pid(conn: &Connection, bus: &str) -> Option<i32> {
    conn.unique_name()?;
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let name = zbus::names::BusName::try_from(bus).ok()?;
    let pid = dbus.get_connection_unix_process_id(name).await.ok()?;
    i32::try_from(pid).ok()
}

fn parse_node(v: &OwnedValue) -> Option<MenuNode> {
    let Value::Structure(s) = &**v else {
        return None;
    };
    let fields = s.fields();
    let Value::I32(id) = fields.first()? else {
        return None;
    };
    let Value::Dict(props) = fields.get(1)? else {
        return None;
    };
    let mut node = MenuNode {
        id: *id,
        label: String::new(),
        separator: false,
        enabled: true,
        visible: true,
        checked: None,
        radio: false,
        has_children: false,
        default: false,
    };
    let mut toggle_type: Option<String> = None;
    let mut toggle_state: Option<i32> = None;
    for (k, v) in props.iter() {
        let Value::Str(k) = k else { continue };
        let v = match v {
            Value::Value(inner) => &**inner,
            other => other,
        };
        match (k.as_str(), v) {
            ("label", Value::Str(s)) => node.label = s.chars().take(LABEL_CHARS).collect(),
            ("enabled", Value::Bool(b)) => node.enabled = *b,
            ("visible", Value::Bool(b)) => node.visible = *b,
            ("type", Value::Str(s)) => node.separator = s.as_str() == "separator",
            ("toggle-type", Value::Str(s)) => toggle_type = Some(s.as_str().to_string()),
            ("toggle-state", Value::I32(i)) => toggle_state = Some(*i),
            ("children-display", Value::Str(s)) => node.has_children = s.as_str() == "submenu",
            (DEFAULT_PROP, Value::Bool(b)) => node.default = *b,
            _ => {}
        }
    }
    if matches!(toggle_type.as_deref(), Some("checkmark") | Some("radio")) {
        node.checked = Some(toggle_state == Some(1));
        node.radio = toggle_type.as_deref() == Some("radio");
    }
    Some(node)
}

/// The first `MENU_ROWS` well-formed children of a `GetLayout` reply, in order; malformed children
/// are skipped.
pub fn parse_layout(children: &[OwnedValue]) -> Vec<MenuNode> {
    children
        .iter()
        .filter_map(parse_node)
        .take(MENU_ROWS)
        .collect()
}

type LayoutReply = (u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>));

async fn get_layout(conn: &Connection, item: &TrayItem, parent: i32) -> Option<Vec<MenuNode>> {
    let menu_path = item.menu_path.as_deref()?;
    let proxy = uncached_proxy(conn, &item.service, menu_path, MENU_IFACE)
        .await
        .ok()?;
    // Lets the app refresh the menu; its answer (whether it changed) is moot
    // since we fetch the layout right after either way.
    let _ = proxy.call_method("AboutToShow", &(parent,)).await;
    let props = [
        "label",
        "enabled",
        "visible",
        "type",
        "toggle-type",
        "toggle-state",
        "children-display",
        DEFAULT_PROP,
    ];
    let reply = proxy
        .call_method("GetLayout", &(parent, 1i32, props.as_slice()))
        .await
        .ok()?;
    let (_, (_, _, children)): LayoutReply = reply.body().deserialize().ok()?;
    Some(parse_layout(&children))
}

async fn send_menu_event(conn: &Connection, item: &TrayItem, node: i32) {
    let Some(menu_path) = item.menu_path.as_deref() else {
        return;
    };
    let Ok(proxy) = uncached_proxy(conn, &item.service, menu_path, MENU_IFACE).await else {
        return;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0);
    let _ = proxy
        .call_method("Event", &(node, "clicked", Value::I32(0), now))
        .await;
}

async fn call_item(conn: &Connection, item: &TrayItem, method: &str, x: i32, y: i32) {
    if let Ok(proxy) = uncached_proxy(conn, &item.service, &item.path, ITEM_IFACE).await
        && let Err(e) = proxy.call_method(method, &(x, y)).await
    {
        log::debug!("tray: {method} on {} failed: {e}", item.id);
    }
}

async fn call_scroll(conn: &Connection, item: &TrayItem, delta: i32, orientation: Orientation) {
    if let Ok(proxy) = uncached_proxy(conn, &item.service, &item.path, ITEM_IFACE).await
        && let Err(e) = proxy
            .call_method("Scroll", &(delta, orientation.as_str()))
            .await
    {
        log::debug!("tray: Scroll on {} failed: {e}", item.id);
    }
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
                self.streams.push(
                    stream
                        .map(move |_| Wake::ItemChanged(key.clone()))
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
        published
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|p| p.key != key);
        if let Ok(iface) = self
            .conn
            .object_server()
            .interface::<_, Watcher>(WATCHER_PATH)
            .await
        {
            let _ = Watcher::status_notifier_item_unregistered(iface.signal_emitter(), key).await;
        }
    }

    /// Run `call` beside the loop, so a peer that never answers holds up only its own call.
    fn spawn(&mut self, call: impl Future<Output = ()> + Send + 'static) {
        let done = futures_util::stream::once(call).filter_map(|()| std::future::ready(None));
        self.streams.push(done.boxed());
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
        let (conn, icons) = (self.conn.clone(), self.icons.clone());
        let read = async move {
            if !settle.is_zero() {
                async_io::Timer::after(settle).await;
            }
            reading.store(true, Ordering::Relaxed);
            let item = fetch_item(&conn, &icons, &bus, &path).await.map(Box::new);
            Wake::Fetched { key, fetch, item }
        };
        self.streams.push(futures_util::stream::once(read).boxed());
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
            TrayCommand::Click { id, x, y, kind } => {
                if let Some(p) = peer(&id) {
                    let conn = self.conn.clone();
                    self.spawn(async move { call_item(&conn, &p, kind.method(), x, y).await });
                }
            }
            TrayCommand::Scroll {
                id,
                delta,
                orientation,
            } => {
                if let Some(p) = peer(&id) {
                    let conn = self.conn.clone();
                    self.spawn(async move { call_scroll(&conn, &p, delta, orientation).await });
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
            TrayCommand::MenuEvent { id, item: node } => {
                if let Some(p) = peer(&id) {
                    let conn = self.conn.clone();
                    self.spawn(async move { send_menu_event(&conn, &p, node).await });
                }
            }
            TrayCommand::NotificationClosed { id, reason } => {
                if let Ok(iface) = self
                    .conn
                    .object_server()
                    .interface::<_, Notifications>(NOTIFY_PATH)
                    .await
                {
                    lock_senders(&self.senders).0.remove(&id);
                    let _ = Notifications::notification_closed(iface.signal_emitter(), id, reason)
                        .await;
                }
            }
            TrayCommand::Replied { id, text } => {
                if let Ok(iface) = self
                    .conn
                    .object_server()
                    .interface::<_, Notifications>(NOTIFY_PATH)
                    .await
                {
                    let emitter = iface.signal_emitter();
                    let _ = Notifications::notification_replied(emitter, id, &text).await;
                }
            }
            TrayCommand::ActionInvoked { id, key, token } => {
                if let Ok(iface) = self
                    .conn
                    .object_server()
                    .interface::<_, Notifications>(NOTIFY_PATH)
                    .await
                {
                    let emitter = iface.signal_emitter();
                    if let Some(token) = token {
                        let _ = Notifications::activation_token(emitter, id, &token).await;
                    }
                    let _ = Notifications::action_invoked(emitter, id, &key).await;
                }
            }
            TrayCommand::Appearance(a) => settings::update(&self.conn, &self.appearance, a).await,
            TrayCommand::IconTheme(name) => self.retheme(&name),
            TrayCommand::Shutdown => return false,
        }
        true
    }

    fn retheme(&mut self, name: &str) {
        *lock(&self.icons) = load_icons(name);
        for item in self.items.values_mut() {
            let named = item_named(&self.icons, item);
            if named != item.named {
                item.named = named;
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
                        if lock_senders(&self.senders).sent_by(&name) {
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
    Some(stream.filter_map(move |m| std::future::ready(f(m))).boxed())
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
) -> Result<Loop, String> {
    let icons: Icons = Arc::new(Mutex::new(load_icons(&icon_theme)));
    let senders: Arc<Mutex<Senders>> = Arc::default();
    let conn = zbus::connection::Builder::session()
        .map_err(|e| format!("session bus: {e}"))?
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
) -> Option<(async_channel::Sender<TrayCommand>, JoinHandle<()>)> {
    let (tx, rx) = async_channel::unbounded::<TrayCommand>();
    let handle = std::thread::Builder::new()
        .name("tray-dbus".into())
        .spawn(move || {
            // The thread owns its copy of the eventfd, so the loop side can close its own without
            // waiting for a stalled peer.
            let fd = eventfd.as_raw_fd();
            zbus::block_on(async move {
                match setup(events.clone(), fd, rx, appearance, icon_theme, pads).await {
                    Ok(l) => l.run().await,
                    Err(e) => emit(&events, fd, TrayEvent::Unavailable(e)),
                }
            });
            drop(eventfd);
        })
        .ok()?;
    Some((tx, handle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::os::fd::FromRawFd;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc::Receiver;
    use zbus::zvariant::{OwnedValue, Value};

    fn child(id: i32, props: Vec<(&str, Value<'static>)>) -> OwnedValue {
        let map: HashMap<&str, Value> = props.into_iter().collect();
        Value::from((id, map, Vec::<Value>::new()))
            .try_to_owned()
            .unwrap()
    }

    #[test]
    fn parse_layout_extracts_nodes() {
        let children = vec![
            child(
                7,
                vec![
                    ("label", Value::from("_Connect")),
                    ("enabled", Value::from(false)),
                    ("toggle-type", Value::from("checkmark")),
                    ("toggle-state", Value::from(1i32)),
                ],
            ),
            child(8, vec![("type", Value::from("separator"))]),
            child(
                9,
                vec![
                    ("label", Value::from("More")),
                    ("visible", Value::from(false)),
                    ("children-display", Value::from("submenu")),
                ],
            ),
        ];
        let nodes = parse_layout(&children);
        assert_eq!(nodes.len(), 3);
        assert_eq!(
            nodes[0],
            MenuNode {
                id: 7,
                label: "_Connect".into(),
                separator: false,
                enabled: false,
                visible: true,
                checked: Some(true),
                radio: false,
                has_children: false,
                default: false,
            }
        );
        assert!(nodes[1].separator);
        assert_eq!(nodes[1].id, 8);
        assert!(!nodes[2].visible);
        assert!(nodes[2].has_children);
        assert_eq!(nodes[2].checked, None);
        let radio = parse_layout(&[child(
            10,
            vec![
                ("label", Value::from("Balanced")),
                ("toggle-type", Value::from("radio")),
                ("toggle-state", Value::from(1i32)),
            ],
        )]);
        assert_eq!(
            (radio[0].checked, radio[0].radio),
            (Some(true), true),
            "a radio row is on and bulleted"
        );
    }

    #[test]
    fn parse_layout_reads_variant_wrapped_properties() {
        let children = vec![child(
            3,
            vec![
                ("label", Value::from("_Connect")),
                ("enabled", Value::from(false)),
                ("toggle-type", Value::from("checkmark")),
                ("toggle-state", Value::from(0i32)),
                ("children-display", Value::from("submenu")),
                (DEFAULT_PROP, Value::from(true)),
            ],
        )];
        let Value::Structure(s) = &*children[0] else {
            panic!("fixture is not a structure");
        };
        let Some(Value::Dict(props)) = s.fields().get(1) else {
            panic!("fixture has no property dictionary");
        };
        assert!(
            props.iter().all(|(_, v)| matches!(v, Value::Value(_))),
            "fixture is not in the a{{sv}} wire shape"
        );
        assert_eq!(
            parse_layout(&children),
            vec![MenuNode {
                id: 3,
                label: "_Connect".into(),
                separator: false,
                enabled: false,
                visible: true,
                checked: Some(false),
                radio: false,
                has_children: true,
                default: true,
            }]
        );
    }

    #[test]
    fn parse_layout_clips_labels_and_caps_rows() {
        let long = "x".repeat(LABEL_CHARS + 1000);
        let children: Vec<OwnedValue> = (0..MENU_ROWS as i32 + 10)
            .map(|id| child(id, vec![("label", Value::from(long.clone()))]))
            .collect();
        let nodes = parse_layout(&children);
        assert_eq!(nodes.len(), MENU_ROWS);
        assert!(nodes.iter().all(|n| n.label.chars().count() == LABEL_CHARS));
    }

    #[test]
    fn icon_ref_drops_empty_names_and_names_past_path_max() {
        let max = libc::PATH_MAX as usize;
        assert_eq!(icon_ref(None), None);
        assert_eq!(icon_ref(Some(String::new())), None);
        assert_eq!(icon_ref(Some("x".repeat(max + 1))), None);
        let longest = "x".repeat(max);
        assert_eq!(icon_ref(Some(longest.clone())), Some(longest));
    }

    #[test]
    fn item_texts_clip_the_title_and_the_tooltip() {
        let long = "x".repeat(TIP_CHARS + 1000);
        let tip: ToolTip = ("", vec![], &long, "");
        let (title, tooltip) = item_texts(Some(long.clone()), Some(tip));
        assert_eq!(title.chars().count(), TIP_CHARS);
        assert_eq!(tooltip.chars().count(), TIP_CHARS);
        let blank: ToolTip = ("", vec![], " ", "");
        let (_, tooltip) = item_texts(Some(long), Some(blank));
        assert_eq!(tooltip.chars().count(), TIP_CHARS);
    }

    #[test]
    fn icon_pixmap_variant_decodes_borrowing_the_pixels() {
        use zbus::zvariant::{LE, serialized::Context, to_bytes};
        let list = vec![(2i32, 1i32, vec![7u8; 8])];
        let data = to_bytes(Context::new_dbus(LE, 0), &Value::from(list)).unwrap();
        let (v, _): (DeserializeValue<Pixmaps>, _) = data.deserialize().unwrap();
        assert_eq!(v.0, [(2, 1, &[7u8; 8][..])]);
    }

    #[test]
    fn an_item_name_is_decoded_here_at_the_drawn_sizes() {
        let dir = std::env::temp_dir().join(format!("reprise-tray-named-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("peer.svg"),
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#00f"/></svg>"##,
        )
        .unwrap();
        let icons: Icons = Arc::new(Mutex::new(IconCache::new(IconTheme::load("hicolor", &[]))));
        let mut item = TrayItem {
            id: ":1.5/StatusNotifierItem".into(),
            key: "peer".into(),
            service: ":1.5".into(),
            path: "/StatusNotifierItem".into(),
            title: String::new(),
            tooltip: String::new(),
            icon_name: Some("peer".into()),
            icon_theme_path: Some(dir.clone()),
            pixmaps: Arc::from([]),
            named: Arc::from([]),
            menu_path: None,
            passive: false,
            pid: None,
        };
        let named = item_named(&icons, &item);
        let sizes: Vec<(u32, u32)> = named.iter().map(|s| (s.width, s.height)).collect();
        assert_eq!(sizes, [(16, 16), (32, 32)]);
        let drawn = crate::tray::closest(&named, 32).unwrap();
        assert_eq!(drawn.get(16, 16), 0xFF00_00FF);

        item.pixmaps = item_pixmaps(&[(1, 1, &[255u8, 255, 0, 0][..])]).into();
        assert!(item_named(&icons, &item).is_empty());
        item.pixmaps = Arc::from([]);
        item.icon_name = Some("absent".into());
        assert!(item_named(&icons, &item).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parse_layout_skips_malformed_children() {
        let junk = Value::from(42i32).try_to_owned().unwrap();
        assert!(parse_layout(&[junk]).is_empty());
    }

    fn test_loop(conn: Connection, events: Sender<TrayEvent>, eventfd: RawFd) -> Loop {
        Loop {
            conn,
            events: events.clone(),
            eventfd,
            items: HashMap::new(),
            fetching: HashMap::new(),
            next_fetch: 0,
            item_stops: HashMap::new(),
            published: None,
            screensaver: screensaver::Shared::new(events, eventfd),
            streams: SelectAll::new(),
            appearance: Arc::new(Mutex::new(
                Appearance::of(&crate::config::Config::default()),
            )),
            icons: Arc::new(Mutex::new(IconCache::new(IconTheme::load("hicolor", &[])))),
            senders: Arc::default(),
        }
    }

    async fn peers(timeout: Duration) -> (Connection, Connection) {
        let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        let (item_side, our_side) = futures_util::join!(
            zbus::connection::Builder::async_io_unix_stream(theirs)
                .server(zbus::Guid::generate())
                .unwrap()
                .p2p()
                .build(),
            zbus::connection::Builder::async_io_unix_stream(ours)
                .p2p()
                .method_timeout(timeout)
                .build(),
        );
        (our_side.unwrap(), item_side.unwrap())
    }

    /// Serve items on `conn` from `calls`, its message stream: a `Title` read on `/answering` or
    /// `/changing` gets "B" and counts in `titles`, any other property read there an error, and
    /// calls on any other path no reply at all. `/changing` sends `NewTitle` right after its first
    /// `Title` read. The stream must exist before the calls arrive, or it never sees them.
    async fn serve_items(conn: Connection, mut calls: zbus::MessageStream, titles: &AtomicUsize) {
        while let Some(Ok(msg)) = calls.next().await {
            let hdr = msg.header();
            let Some(path) = hdr.path().map(|p| p.as_str().to_string()) else {
                continue;
            };
            if path != "/answering" && path != "/changing" {
                continue;
            }
            let Ok((_, name)) = msg.body().deserialize::<(String, String)>() else {
                continue;
            };
            if name == "Title" {
                let first = titles.fetch_add(1, Ordering::Relaxed) == 0;
                let _ = conn.reply(&hdr, &Value::from("B")).await;
                if first && path == "/changing" {
                    let changed =
                        zbus::message::Message::signal("/changing", ITEM_IFACE, "NewTitle")
                            .unwrap()
                            .sender(":1.1")
                            .unwrap()
                            .build(&())
                            .unwrap();
                    let _ = conn.send(&changed).await;
                }
            } else {
                let err = "org.freedesktop.DBus.Error.UnknownProperty";
                let _ = conn.reply_error(&hdr, err, &name).await;
            }
        }
    }

    struct Harness {
        wakes: async_channel::Sender<Wake>,
        events: Receiver<TrayEvent>,
        titles: AtomicUsize,
        item: Connection,
    }

    fn drive<T>(script: impl AsyncFnOnce(&Harness) -> T) -> T {
        let (tx, events) = std::sync::mpsc::channel();
        // SAFETY: eventfd takes no pointers; the result is checked below.
        let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        assert!(fd >= 0, "eventfd failed");
        // SAFETY: `fd` was just opened and nothing else owns it.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        zbus::block_on(async {
            let (ours, item) = peers(Duration::from_secs(10)).await;
            let calls = zbus::MessageStream::from(&item);
            let mut lp = test_loop(ours, tx, fd.as_raw_fd());
            let (wakes, wake_rx) = async_channel::unbounded();
            lp.streams.push(wake_rx.boxed());
            let h = Harness {
                wakes,
                events,
                titles: AtomicUsize::new(0),
                item,
            };
            let script = async {
                let out = script(&h).await;
                let _ = h.wakes.send(Wake::Command(TrayCommand::Shutdown)).await;
                out
            };
            let work = async { futures_util::join!(script, lp.run()).0 };
            let serve = async {
                serve_items(h.item.clone(), calls, &h.titles).await;
                std::future::pending().await
            };
            futures_lite::future::or(work, serve).await
        })
    }

    async fn register(wakes: &async_channel::Sender<Wake>, path: &str) {
        let bus = ":1.1".to_string();
        let path = path.to_string();
        wakes.send(Wake::Registered { bus, path }).await.unwrap();
    }

    async fn next_event(events: &Receiver<TrayEvent>, within: Duration) -> Option<TrayEvent> {
        let tick = Duration::from_millis(10);
        for _ in 0..within.as_millis() / tick.as_millis() {
            if let Ok(ev) = events.try_recv() {
                return Some(ev);
            }
            async_io::Timer::after(tick).await;
        }
        None
    }

    /// An item that never answers holds up only its own read: one registered after it is added
    /// within 2 s, while calls may take 10 s.
    #[test]
    fn a_silent_item_does_not_hold_up_the_next() {
        let added = drive(async |h| {
            register(&h.wakes, "/silent").await;
            register(&h.wakes, "/answering").await;
            match next_event(&h.events, Duration::from_secs(2)).await {
                Some(TrayEvent::Added(item)) => Some(item.id),
                _ => None,
            }
        });
        assert_eq!(added.as_deref(), Some(":1.1/answering"));
    }

    #[test]
    fn a_burst_of_changes_reads_the_item_once() {
        let (reads, updates) = drive(async |h| {
            register(&h.wakes, "/answering").await;
            let added = next_event(&h.events, Duration::from_secs(2)).await;
            assert!(
                matches!(added, Some(TrayEvent::Added(_))),
                "the item is added"
            );
            for _ in 0..10 {
                let key = ":1.1/answering".to_string();
                h.wakes.send(Wake::ItemChanged(key)).await.unwrap();
            }
            async_io::Timer::after(Duration::from_millis(300)).await;
            let updates = std::iter::from_fn(|| h.events.try_recv().ok()).count();
            (h.titles.load(Ordering::Relaxed), updates)
        });
        assert_eq!(reads, 2, "one read adds the item and one follows the burst");
        assert_eq!(updates, 0, "an unchanged item sends no update");
    }

    #[test]
    fn a_change_during_the_first_read_reads_the_item_again() {
        let reads = drive(async |h| {
            register(&h.wakes, "/changing").await;
            let added = next_event(&h.events, Duration::from_secs(2)).await;
            assert!(
                matches!(added, Some(TrayEvent::Added(_))),
                "the item is added"
            );
            async_io::Timer::after(Duration::from_millis(300)).await;
            h.titles.load(Ordering::Relaxed)
        });
        assert_eq!(
            reads, 2,
            "the NewTitle sent during the first read gets a second read"
        );
    }

    #[test]
    fn every_change_signal_reads_the_item_again() {
        let reads = drive(async |h| {
            register(&h.wakes, "/answering").await;
            let added = next_event(&h.events, Duration::from_secs(2)).await;
            assert!(
                matches!(added, Some(TrayEvent::Added(_))),
                "the item is added"
            );
            let mut reads = Vec::new();
            for signal in ["NewIcon", "NewTitle", "NewStatus", "NewToolTip", "NewMenu"] {
                let msg = zbus::message::Message::signal("/answering", ITEM_IFACE, signal)
                    .unwrap()
                    .sender(":1.1")
                    .unwrap()
                    .build(&())
                    .unwrap();
                h.item.send(&msg).await.unwrap();
                async_io::Timer::after(Duration::from_millis(200)).await;
                reads.push((signal, h.titles.load(Ordering::Relaxed)));
            }
            reads
        });
        let signals = ["NewIcon", "NewTitle", "NewStatus", "NewToolTip", "NewMenu"];
        let want: Vec<(&str, usize)> = signals.into_iter().zip(2..).collect();
        assert_eq!(reads, want, "one more Title read after each signal");
    }

    #[test]
    fn only_its_sender_replaces_or_closes_a_balloon() {
        let mut senders = Senders::default();
        let mut next = 1;
        let mut fresh = || {
            next += 1;
            next - 1
        };
        let a = senders.claim(":1.7", "mail", 0, &mut fresh);
        assert_eq!(a, 1, "a new balloon gets a fresh id");
        assert_eq!(
            senders.claim(":1.7", "mail", a, &mut fresh),
            a,
            "its sender replaces it"
        );
        assert_eq!(
            senders.claim(":1.8", "chat", a, &mut fresh),
            2,
            "another peer gets a fresh id"
        );
        assert_eq!(
            senders.claim(":1.8", "", a, &mut fresh),
            3,
            "a peer with no app name gets a fresh id"
        );
        assert!(senders.owns(a, ":1.7"));
        assert!(!senders.owns(a, ":1.8"), "another peer may not close it");
        senders.0.remove(&a);
        assert_eq!(
            senders.claim(":1.7", "mail", a, &mut fresh),
            4,
            "a closed id is not reused"
        );
    }

    #[test]
    fn a_new_connection_under_the_same_app_name_replaces_its_balloon() {
        let mut senders = Senders::default();
        let mut next = 1;
        let mut fresh = || {
            next += 1;
            next - 1
        };
        let a = senders.claim(":1.7", "notify-send", 0, &mut fresh);
        assert_eq!(senders.claim(":1.9", "notify-send", a, &mut fresh), a);
        assert!(
            senders.owns(a, ":1.9"),
            "the replacing connection may close it"
        );
        assert!(!senders.owns(a, ":1.10"));
    }

    #[test]
    fn a_registration_takes_a_path_a_bus_or_both() {
        let own = |bus: &str, path: &str| Some((bus.to_string(), path.to_string()));
        let reg = |service| resolve_registration(service, Some(":1.5"));
        assert_eq!(reg("/org/x"), own(":1.5", "/org/x"));
        assert_eq!(reg("org.x.App"), own("org.x.App", "/StatusNotifierItem"));
        let both = ":1.42/org/ayatana/NotificationItem/x";
        assert_eq!(reg(both), own(":1.42", "/org/ayatana/NotificationItem/x"));
        assert_eq!(reg(""), None);
    }

    fn published(sender: &str, n: usize) -> Vec<Published> {
        (0..n)
            .map(|i| Published {
                key: format!("{sender}/item{i}"),
                sender: sender.to_string(),
            })
            .collect()
    }

    #[test]
    fn admit_caps_one_sender_and_lets_another_register() {
        let mut items = published(":1.7", MAX_REGISTRATIONS_PER_SENDER);
        assert_eq!(admit(&items, ":1.7/extra", ":1.7"), Admission::SenderFull);
        assert_eq!(admit(&items, ":1.7/item0", ":1.7"), Admission::Known);
        assert_eq!(admit(&items, ":1.8/item0", ":1.8"), Admission::New);
        items.retain(|p| p.key != ":1.7/item3");
        assert_eq!(admit(&items, ":1.7/extra", ":1.7"), Admission::New);
    }

    #[test]
    fn admit_caps_the_total_across_senders() {
        let items: Vec<Published> = (0..MAX_REGISTRATIONS / MAX_REGISTRATIONS_PER_SENDER)
            .flat_map(|i| published(&format!(":1.{i}"), MAX_REGISTRATIONS_PER_SENDER))
            .collect();
        assert_eq!(items.len(), MAX_REGISTRATIONS);
        assert_eq!(admit(&items, ":1.99/item0", ":1.99"), Admission::TotalFull);
    }
}
