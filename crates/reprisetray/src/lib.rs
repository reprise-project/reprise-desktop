//! A notification-area item for an applet: a StatusNotifierItem with a dbusmenu, served on its own
//! D-Bus thread, plus balloons through `org.freedesktop.Notifications` and the single-instance bus name.
//!
//! The applet hands over a [`Config`] and a [`Shown`], talks to the thread through [`TrayHandle`] and
//! drains [`Msg`]s on every wake. Row meaning, icon art and click handling stay with the applet.

use futures_util::stream::StreamExt;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use zbus::{Connection, Proxy};

use reprisebus::instance::Handoff;

pub const ITEM_PATH: &str = "/StatusNotifierItem";
pub const MENU_PATH: &str = "/MenuBar";

const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
const MENU_IFACE: &str = "com.canonical.dbusmenu";
const HANDOFF_IFACE: &str = "org.reprise.TrayItem";
const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const NOTIFY_NAME: &str = "org.freedesktop.Notifications";
const NOTIFY_PATH: &str = "/org/freedesktop/Notifications";

#[derive(Clone, Debug)]
pub struct Config {
    pub bus_name: &'static str,
    pub id: &'static str,
    pub title: &'static str,
    pub notify_icon: &'static str,
    pub raise: Raise,
}

/// How a second instance raises the first: SNI `Activate`, or a click on one dbusmenu row when `Activate` opens something else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Raise {
    Activate,
    MenuRow(i32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Icon {
    Name(String),
    Pixmap { size: i32, argb: Vec<u8> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub id: i32,
    pub label: String,
    pub separator: bool,
    pub toggle: Option<bool>,
    /// The toggle is one of a set of radio rows (a bullet) rather than a checkmark.
    pub radio: bool,
    /// The menu's default row, drawn bold by the host: what a double click on the icon does. Sent as [`DEFAULT_PROP`].
    pub default: bool,
}

/// The dbusmenu property marking a menu's default row, a vendor property under the spec's `x-<vendor>-` prefix.
pub const DEFAULT_PROP: &str = "x-reprise-default";

impl Node {
    pub fn row(id: i32, label: impl Into<String>) -> Node {
        Node {
            id,
            label: label.into(),
            separator: false,
            toggle: None,
            radio: false,
            default: false,
        }
    }

    pub fn separator(id: i32) -> Node {
        Node {
            id,
            label: String::new(),
            separator: true,
            toggle: None,
            radio: false,
            default: false,
        }
    }

    pub fn as_default(self) -> Node {
        Node {
            default: true,
            ..self
        }
    }

    pub fn toggle(id: i32, label: impl Into<String>, on: bool) -> Node {
        Node {
            toggle: Some(on),
            ..Node::row(id, label)
        }
    }

    pub fn radio(id: i32, label: impl Into<String>, on: bool) -> Node {
        Node {
            radio: true,
            ..Node::toggle(id, label, on)
        }
    }
}

/// What the item shows: written by the applet through `set_state`, read by both served interfaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub icon: Icon,
    pub tip: String,
    pub active: bool,
    pub nodes: Vec<Node>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Msg {
    /// Left click, in global layout coordinates: reported over the icon's centre on the taskbar's edge
    /// facing the screen, so a popup opened there meets the bar.
    Activate(i32, i32),
    SecondaryActivate,
    Scroll(i32),
    Clicked(i32),
    /// A balloon sent with `notify_clickable` was clicked, by its tag; the window's token waits in `take_activation_token`.
    BalloonClicked(u32),
    /// Button `i` (in the order given) of a balloon sent with `notify_buttons` was pressed, by its tag; the token waits as for a click.
    BalloonButton(u32, usize),
}

pub type Wake = Arc<dyn Fn() + Send + Sync>;

fn owned(v: Value<'_>) -> OwnedValue {
    v.try_to_owned().expect("menu property is ownable")
}

fn props(node: &Node) -> Vec<(&'static str, Value<'static>)> {
    if node.separator {
        return vec![("type", Value::from("separator"))];
    }
    let mut out = vec![
        ("label", Value::from(node.label.clone())),
        ("enabled", Value::from(true)),
        ("visible", Value::from(true)),
    ];
    if let Some(on) = node.toggle {
        let kind = if node.radio { "radio" } else { "checkmark" };
        out.push(("toggle-type", Value::from(kind)));
        out.push(("toggle-state", Value::from(if on { 1i32 } else { 0i32 })));
    }
    if node.default {
        out.push((DEFAULT_PROP, Value::from(true)));
    }
    out
}

pub fn node_value(node: &Node) -> OwnedValue {
    let props: HashMap<&str, Value<'static>> = props(node).into_iter().collect();
    owned(Value::from((node.id, props, Vec::<Value>::new())))
}

fn node_props(node: &Node) -> HashMap<String, OwnedValue> {
    props(node)
        .into_iter()
        .map(|(k, v)| (k.to_string(), owned(v)))
        .collect()
}

type LayoutItem = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

#[derive(Debug)]
struct State {
    shown: Shown,
    revision: u32,
}

enum Cmd {
    Set(Shown),
    Notify {
        summary: String,
        body: String,
        tag: Option<u32>,
        buttons: Vec<String>,
    },
}

/// What a second instance or a balloon click handed over, until the applet takes it.
type Token = Arc<Mutex<Option<Handoff>>>;

pub struct TrayHandle {
    commands: async_channel::Sender<Cmd>,
    token: Token,
}

impl TrayHandle {
    pub fn set_state(&self, shown: Shown) {
        let _ = self.commands.send_blocking(Cmd::Set(shown));
    }

    /// A balloon over the tray, through `org.freedesktop.Notifications`. `body` is plain text: the spec
    /// lets a daemon read a markup subset, so what came off the air (an SSID) is escaped here.
    pub fn notify(&self, summary: &str, body: &str) {
        let _ = self.commands.send_blocking(Cmd::Notify {
            summary: summary.into(),
            body: escape_markup(body),
            tag: None,
            buttons: Vec::new(),
        });
    }

    pub fn notify_clickable(&self, summary: &str, body: &str, tag: u32) {
        let _ = self.commands.send_blocking(Cmd::Notify {
            summary: summary.into(),
            body: escape_markup(body),
            tag: Some(tag),
            buttons: Vec::new(),
        });
    }

    /// A balloon with a button per label and no click on its body; a press comes back as [`Msg::BalloonButton`].
    pub fn notify_buttons(&self, summary: &str, body: &str, tag: u32, buttons: &[&str]) {
        let _ = self.commands.send_blocking(Cmd::Notify {
            summary: summary.into(),
            body: escape_markup(body),
            tag: Some(tag),
            buttons: buttons.iter().map(|b| b.to_string()).collect(),
        });
    }

    /// What a second instance handed over before its raise, or a balloon click left, once; for the
    /// `Reply::activate` of the window the raise shows.
    pub fn take_handoff(&self) -> Option<Handoff> {
        self.token.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// The token [`TrayHandle::take_handoff`] would give, for a window another process opens.
    pub fn take_activation_token(&self) -> Option<String> {
        self.take_handoff().and_then(|h| h.token)
    }

    /// A handle with no tray behind it, holding `handoff` as if a second instance had sent it: for
    /// the applets' tests.
    #[doc(hidden)]
    pub fn holding(handoff: Handoff) -> TrayHandle {
        TrayHandle {
            commands: async_channel::unbounded().0,
            token: Arc::new(Mutex::new(Some(handoff))),
        }
    }
}

/// `s` as a notification body reads it literally: the markup subset's three characters escaped.
pub fn escape_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

#[derive(Clone)]
struct Shared {
    state: Arc<Mutex<State>>,
    token: Token,
    msgs: Sender<Msg>,
    wake: Wake,
    config: Config,
}

impl Shared {
    fn send(&self, msg: Msg) {
        if self.msgs.send(msg).is_ok() {
            (self.wake)();
        }
    }

    fn with<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        f(&self.state.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// A token on its own (the tray host's `ProvideXdgActivationToken`, a balloon click) came from a
/// pointer: it drops an earlier by-key mark.
fn hand_token(handoff: &Token, token: String) {
    *handoff.lock().unwrap_or_else(|e| e.into_inner()) = Some(Handoff {
        token: Some(token),
        by_key: false,
    });
}

/// A second instance's [`Handoff`], next to the item: the SNI interface carries only a token.
struct Handoffs(Token);

#[zbus::interface(name = "org.reprise.TrayItem")]
impl Handoffs {
    fn provide_platform_data(&self, platform_data: HashMap<String, OwnedValue>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(Handoff::from_platform_data(&platform_data));
    }
}

struct Item(Shared);

type ToolTip = (String, Vec<(i32, i32, Vec<u8>)>, String, String);

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    fn activate(&self, x: i32, y: i32) {
        self.0.send(Msg::Activate(x, y));
    }

    fn secondary_activate(&self, _x: i32, _y: i32) {
        self.0.send(Msg::SecondaryActivate);
    }

    fn provide_xdg_activation_token(&self, token: String) {
        hand_token(&self.0.token, token);
    }

    fn context_menu(&self, _x: i32, _y: i32) {}

    fn scroll(&self, delta: i32, orientation: &str) {
        if orientation.eq_ignore_ascii_case("vertical") {
            self.0.send(Msg::Scroll(delta));
        }
    }

    #[zbus(property)]
    fn category(&self) -> &str {
        "Hardware"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        self.0.config.id
    }

    #[zbus(property)]
    fn title(&self) -> &str {
        self.0.config.title
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        if self.0.with(|s| s.shown.active) {
            "Active"
        } else {
            "Passive"
        }
    }

    #[zbus(property)]
    fn icon_name(&self) -> String {
        self.0.with(|s| match &s.shown.icon {
            Icon::Name(n) => n.clone(),
            Icon::Pixmap { .. } => String::new(),
        })
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        self.0.with(|s| match &s.shown.icon {
            Icon::Name(_) => Vec::new(),
            Icon::Pixmap { size, argb } => vec![(*size, *size, argb.clone())],
        })
    }

    #[zbus(property)]
    fn tool_tip(&self) -> ToolTip {
        (
            String::new(),
            Vec::new(),
            self.0.with(|s| s.shown.tip.clone()),
            String::new(),
        )
    }

    #[zbus(property)]
    fn menu(&self) -> ObjectPath<'_> {
        ObjectPath::from_static_str_unchecked(MENU_PATH)
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }

    #[zbus(signal)]
    async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_title(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_status(emitter: &SignalEmitter<'_>, status: &str) -> zbus::Result<()>;
}

struct Menu(Shared);

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    /// The menu is one flat level under the root (id 0); any other parent has no children.
    fn get_layout(
        &self,
        parent_id: i32,
        _recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> (u32, LayoutItem) {
        self.0.with(|s| {
            if parent_id != 0 {
                return (s.revision, (parent_id, HashMap::new(), Vec::new()));
            }
            let mut root = HashMap::new();
            root.insert(
                "children-display".to_string(),
                owned(Value::from("submenu")),
            );
            let children = s.shown.nodes.iter().map(node_value).collect();
            (s.revision, (0, root, children))
        })
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        self.0.with(|s| {
            s.shown
                .nodes
                .iter()
                .filter(|n| ids.is_empty() || ids.contains(&n.id))
                .map(|n| (n.id, node_props(n)))
                .collect()
        })
    }

    /// The layout never changes behind the host's back: `set_state` bumps the revision and signals `LayoutUpdated`.
    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn event(&self, id: i32, event_id: &str, _data: Value<'_>, _timestamp: u32) {
        if event_id == "clicked" {
            self.0.send(Msg::Clicked(id));
        }
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }

    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(signal)]
    async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;
}

/// Tell the watcher about our item. Harmless when no watcher is up: the next `NameOwnerChanged` brings us back.
async fn register(conn: &Connection) {
    let Ok(proxy) = Proxy::new(conn, WATCHER_NAME, WATCHER_PATH, WATCHER_NAME).await else {
        return;
    };
    if let Err(e) = proxy
        .call_method("RegisterStatusNotifierItem", &(ITEM_PATH,))
        .await
    {
        log::debug!("tray: RegisterStatusNotifierItem failed: {e}");
    }
}

/// Ask the instance that owns the bus name to show itself, handing over our [`Handoff`] first so its
/// window takes focus, with its cues out after a start by key.
async fn activate_existing(conn: &Connection, config: &Config) {
    let handoff = Handoff::from_env();
    if handoff != Handoff::default()
        && let Ok(proxy) = Proxy::new(conn, config.bus_name, ITEM_PATH, HANDOFF_IFACE).await
        && let Err(e) = proxy
            .call_method("ProvidePlatformData", &(handoff.platform_data(),))
            .await
    {
        log::debug!("tray: ProvidePlatformData failed: {e}");
    }
    let result = match config.raise {
        Raise::Activate => match Proxy::new(conn, config.bus_name, ITEM_PATH, ITEM_IFACE).await {
            Ok(proxy) => proxy.call_method("Activate", &(0i32, 0i32)).await,
            Err(e) => Err(e),
        },
        Raise::MenuRow(id) => {
            match Proxy::new(conn, config.bus_name, MENU_PATH, MENU_IFACE).await {
                Ok(proxy) => {
                    let data = Value::from(0u32);
                    proxy
                        .call_method("Event", &(id, "clicked", data, 0u32))
                        .await
                }
                Err(e) => Err(e),
            }
        }
    };
    if let Err(e) = result {
        log::warn!("could not raise the running {}: {e}", config.id);
    }
}

async fn publish(conn: &Connection, state: &Arc<Mutex<State>>) {
    let (revision, active) = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        (s.revision, s.shown.active)
    };
    if let Ok(item) = conn.object_server().interface::<_, Item>(ITEM_PATH).await {
        let e = item.signal_emitter();
        let _ = Item::new_icon(e).await;
        let _ = Item::new_tool_tip(e).await;
        let _ = Item::new_status(e, if active { "Active" } else { "Passive" }).await;
    }
    if let Ok(menu) = conn.object_server().interface::<_, Menu>(MENU_PATH).await {
        let _ = Menu::layout_updated(menu.signal_emitter(), revision, 0).await;
    }
}

/// The `Notify` actions list: a key and label per button, else a `default` action when `clickable`.
fn actions(clickable: bool, buttons: &[String]) -> Vec<String> {
    if !buttons.is_empty() {
        buttons
            .iter()
            .enumerate()
            .flat_map(|(i, label)| [format!("{BUTTON_KEY}{i}"), label.clone()])
            .collect()
    } else if clickable {
        vec!["default".into(), String::new()]
    } else {
        Vec::new()
    }
}

const BUTTON_KEY: &str = "button";

/// A balloon: `Notify(app_name, replaces_id, icon, summary, body, actions, hints, timeout)`, with
/// [`actions`]. The server's id for it, None when the call failed.
async fn notify(
    conn: &Connection,
    config: &Config,
    summary: &str,
    body: &str,
    actions: Vec<String>,
) -> Option<u32> {
    let proxy = Proxy::new(conn, NOTIFY_NAME, NOTIFY_PATH, NOTIFY_NAME)
        .await
        .ok()?;
    let hints: HashMap<&str, Value<'_>> = HashMap::new();
    let reply = proxy
        .call_method(
            "Notify",
            &(
                config.id,
                0u32,
                config.notify_icon,
                summary,
                body,
                actions,
                hints,
                -1i32,
            ),
        )
        .await;
    match reply.and_then(|m| m.body().deserialize::<u32>()) {
        Ok(id) => Some(id),
        Err(e) => {
            log::warn!("notification failed: {e}");
            None
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum BalloonSignal {
    ActivationToken(u32, String),
    ActionInvoked(u32, String),
    Closed(u32),
}

fn balloon_signal(m: &zbus::Message) -> Option<BalloonSignal> {
    let hdr = m.header();
    if hdr.interface().map(|i| i.as_str()) != Some(NOTIFY_NAME) {
        return None;
    }
    let body = m.body();
    match hdr.member()?.as_str() {
        "ActivationToken" => {
            let (id, token): (u32, String) = body.deserialize().ok()?;
            Some(BalloonSignal::ActivationToken(id, token))
        }
        "ActionInvoked" => {
            let (id, key): (u32, String) = body.deserialize().ok()?;
            Some(BalloonSignal::ActionInvoked(id, key))
        }
        "NotificationClosed" => {
            let (id, _reason): (u32, u32) = body.deserialize().ok()?;
            Some(BalloonSignal::Closed(id))
        }
        _ => None,
    }
}

/// The clickable balloons and balloons with buttons still up, by server id, with the tag each comes back with.
#[derive(Debug, Default)]
struct Clickable(HashMap<u32, u32>);

impl Clickable {
    /// What a server signal means here: a click or button press on one of ours, by its tag; its token goes to `token`.
    fn on(&mut self, signal: BalloonSignal, token: &Token) -> Option<Msg> {
        match signal {
            BalloonSignal::ActivationToken(id, t) if self.0.contains_key(&id) => {
                hand_token(token, t);
                None
            }
            BalloonSignal::ActionInvoked(id, key) if key == "default" => {
                self.0.remove(&id).map(Msg::BalloonClicked)
            }
            BalloonSignal::ActionInvoked(id, key) => {
                let i = key.strip_prefix(BUTTON_KEY)?.parse().ok()?;
                self.0.remove(&id).map(|tag| Msg::BalloonButton(tag, i))
            }
            BalloonSignal::Closed(id) => {
                self.0.remove(&id);
                None
            }
            _ => None,
        }
    }
}

/// Follow the server's balloon signals, only from its current owner (see [`reprisebus::signals`]).
async fn balloon_signals(conn: &Connection) -> futures_util::stream::BoxStream<'static, Wakeup> {
    let mut rules = Vec::new();
    for member in ["ActivationToken", "ActionInvoked", "NotificationClosed"] {
        match zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(NOTIFY_NAME)
            .and_then(|b| b.path(NOTIFY_PATH))
            .and_then(|b| b.interface(NOTIFY_NAME))
            .and_then(|b| b.member(member))
        {
            Ok(b) => rules.push(b.build().into()),
            Err(e) => log::warn!("no match rule for {member}: {e}"),
        }
    }
    reprisebus::signals(conn, NOTIFY_NAME, rules)
        .await
        .filter_map(|s| {
            std::future::ready(match s {
                reprisebus::Signal::Message(m) => balloon_signal(&m).map(Wakeup::Balloon),
                reprisebus::Signal::OwnerChanged => None,
            })
        })
        .boxed()
}

enum Wakeup {
    Cmd(Cmd),
    WatcherUp,
    Balloon(BalloonSignal),
    /// Every `TrayHandle` is gone: the applet dropped it, or `start` gave up waiting.
    HandleDropped,
}

async fn run(conn: Connection, shared: Shared, commands: async_channel::Receiver<Cmd>) {
    let (config, state) = (shared.config.clone(), shared.state.clone());
    let mut clickable = Clickable::default();
    let mut streams: futures_util::stream::SelectAll<
        futures_util::stream::BoxStream<'static, Wakeup>,
    > = futures_util::stream::SelectAll::new();
    streams.push(
        commands
            .map(Wakeup::Cmd)
            .chain(futures_util::stream::once(std::future::ready(
                Wakeup::HandleDropped,
            )))
            .boxed(),
    );
    if let Ok(dbus) = zbus::fdo::DBusProxy::new(&conn).await
        && let Ok(s) = dbus
            .receive_name_owner_changed_with_args(&[(0, WATCHER_NAME)])
            .await
    {
        streams.push(
            s.filter_map(|m| async move {
                m.args()
                    .ok()?
                    .new_owner()
                    .is_some()
                    .then_some(Wakeup::WatcherUp)
            })
            .boxed(),
        );
    }
    // Subscribing takes bus round trips, which must not hold up the commands or the loop's end.
    let bus = conn.clone();
    streams.push(
        futures_util::stream::once(async move { balloon_signals(&bus).await })
            .flatten()
            .boxed(),
    );
    while let Some(wake) = streams.next().await {
        match wake {
            Wakeup::Cmd(Cmd::Set(shown)) => {
                {
                    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                    s.shown = shown;
                    s.revision = s.revision.wrapping_add(1);
                }
                publish(&conn, &state).await;
            }
            Wakeup::Cmd(Cmd::Notify {
                summary,
                body,
                tag,
                buttons,
            }) => {
                let actions = actions(tag.is_some(), &buttons);
                let id = notify(&conn, &config, &summary, &body, actions).await;
                if let (Some(id), Some(tag)) = (id, tag) {
                    clickable.0.insert(id, tag);
                }
            }
            Wakeup::Balloon(signal) => {
                if let Some(msg) = clickable.on(signal, &shared.token) {
                    shared.send(msg);
                }
            }
            Wakeup::WatcherUp => register(&conn).await,
            Wakeup::HandleDropped => break,
        }
    }
}

/// How long `start` waits for the bus thread. A bus that answers nothing would otherwise keep the window off the screen.
const READY_TIMEOUT: Duration = Duration::from_secs(10);

pub enum Started {
    /// The item is up: push state through the handle, drain the receiver on every wake.
    Running(TrayHandle, Receiver<Msg>),
    /// Another instance owns the bus name and was asked to raise itself; this one should exit.
    AlreadyRunning,
    /// No session bus or thread, or no answer within `READY_TIMEOUT`: the program runs with its window only.
    Unavailable,
}

/// Bring the tray item up on its own thread, showing `initial` until the first `set_state`.
pub fn start(config: Config, initial: Shown, wake: Wake) -> Started {
    let (ready_tx, ready_rx) = channel::<Started>();
    let spawned = std::thread::Builder::new()
        .name(format!("{}-dbus", config.id))
        .spawn(move || {
            zbus::block_on(async move {
                let conn = match Connection::session().await {
                    Ok(c) => c,
                    Err(e) => {
                        log::info!("no session bus, running without a tray icon: {e}");
                        let _ = ready_tx.send(Started::Unavailable);
                        return;
                    }
                };
                let state = Arc::new(Mutex::new(State {
                    shown: initial,
                    revision: 1,
                }));
                let (msg_tx, msg_rx) = channel::<Msg>();
                let token = Token::default();
                let shared = Shared {
                    state: state.clone(),
                    token: token.clone(),
                    msgs: msg_tx,
                    wake,
                    config: config.clone(),
                };
                let served = conn
                    .object_server()
                    .at(ITEM_PATH, Item(shared.clone()))
                    .await
                    .and(
                        conn.object_server()
                            .at(ITEM_PATH, Handoffs(token.clone()))
                            .await,
                    )
                    .and(
                        conn.object_server()
                            .at(MENU_PATH, Menu(shared.clone()))
                            .await,
                    );
                if let Err(e) = served {
                    log::warn!("could not serve the tray item: {e}");
                    let _ = ready_tx.send(Started::Unavailable);
                    return;
                }
                let owned = conn
                    .request_name_with_flags(config.bus_name, RequestNameFlags::DoNotQueue.into())
                    .await;
                if !matches!(owned, Ok(RequestNameReply::PrimaryOwner)) {
                    activate_existing(&conn, &config).await;
                    let _ = ready_tx.send(Started::AlreadyRunning);
                    return;
                }
                let (cmd_tx, cmd_rx) = async_channel::unbounded::<Cmd>();
                if ready_tx
                    .send(Started::Running(
                        TrayHandle {
                            commands: cmd_tx,
                            token,
                        },
                        msg_rx,
                    ))
                    .is_err()
                {
                    return;
                }
                register(&conn).await;
                run(conn, shared, cmd_rx).await;
            })
        });
    if spawned.is_err() {
        return Started::Unavailable;
    }
    ready_rx.recv_timeout(READY_TIMEOUT).unwrap_or_else(|e| {
        log::warn!("the session bus did not answer in time ({e}); running without a tray icon");
        Started::Unavailable
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A beacon can carry markup in its SSID; the daemon's body shows it as text.
    #[test]
    fn a_balloon_body_is_escaped_for_the_markup_subset() {
        assert_eq!(
            escape_markup("<b>x</b> & y"),
            "&lt;b&gt;x&lt;/b&gt; &amp; y"
        );
        let (tx, rx) = async_channel::unbounded::<Cmd>();
        TrayHandle {
            commands: tx,
            token: Token::default(),
        }
        .notify("Wireless", "Connected to: <b>cafe</b>");
        match rx.try_recv().expect("the command") {
            Cmd::Notify {
                summary, body, tag, ..
            } => {
                assert_eq!(summary, "Wireless");
                assert_eq!(body, "Connected to: &lt;b&gt;cafe&lt;/b&gt;");
                assert_eq!(tag, None);
            }
            _ => panic!("not a notify"),
        }
    }

    /// Once the handle is gone the thread drops its connection, so no item outlives its applet.
    #[test]
    fn the_bus_loop_ends_once_the_handle_is_dropped() {
        let (a, b) = std::os::unix::net::UnixStream::pair().expect("socket pair");
        let (done_tx, done_rx) = channel();
        std::thread::spawn(move || {
            zbus::block_on(async move {
                let (server, _client) = futures_util::future::join(
                    zbus::connection::Builder::async_io_unix_stream(a)
                        .server(zbus::Guid::generate())
                        .expect("server")
                        .p2p()
                        .build(),
                    zbus::connection::Builder::async_io_unix_stream(b)
                        .p2p()
                        .build(),
                )
                .await;
                let config = Config {
                    bus_name: "reprise.test",
                    id: "reprise-test",
                    title: "Test",
                    notify_icon: "",
                    raise: Raise::Activate,
                };
                let state = Arc::new(Mutex::new(State {
                    shown: Shown {
                        icon: Icon::Name(String::new()),
                        tip: String::new(),
                        active: false,
                        nodes: Vec::new(),
                    },
                    revision: 1,
                }));
                let (msgs, _msg_rx) = channel();
                let shared = Shared {
                    state,
                    token: Token::default(),
                    msgs,
                    wake: Arc::new(|| {}),
                    config,
                };
                let (cmd_tx, cmd_rx) = async_channel::unbounded::<Cmd>();
                drop(cmd_tx);
                run(server.expect("p2p connection"), shared, cmd_rx).await;
                let _ = done_tx.send(());
            })
        });
        assert!(
            done_rx.recv_timeout(Duration::from_secs(5)).is_ok(),
            "run returns once its command channel closes"
        );
    }

    fn sent(h: &Handoff) -> HashMap<String, OwnedValue> {
        h.platform_data()
            .into_iter()
            .map(|(k, v)| (k.to_string(), OwnedValue::try_from(v).unwrap()))
            .collect()
    }

    #[test]
    fn a_second_instances_key_mark_waits_for_the_raise_and_a_pointer_token_drops_it() {
        let token = Token::default();
        let handle = TrayHandle {
            commands: async_channel::unbounded().0,
            token: token.clone(),
        };
        let by_key = Handoff {
            token: Some("t-key".into()),
            by_key: true,
        };
        Handoffs(token.clone()).provide_platform_data(sent(&by_key));
        assert_eq!(handle.take_handoff(), Some(by_key.clone()));
        assert_eq!(handle.take_handoff(), None, "taken once");
        Handoffs(token.clone()).provide_platform_data(sent(&by_key));
        hand_token(&token, "t-click".into());
        assert_eq!(
            handle.take_handoff(),
            Some(Handoff {
                token: Some("t-click".into()),
                by_key: false
            })
        );
    }

    /// A click on one of our balloons comes back as its tag, token first; another's balloon, another action, or a closed balloon is nothing.
    #[test]
    fn a_click_on_a_clickable_balloon_comes_back_as_its_tag() {
        let token = Token::default();
        let mut c = Clickable::default();
        c.0.insert(7, 42);
        assert_eq!(
            c.on(BalloonSignal::ActivationToken(7, "tok".into()), &token),
            None
        );
        assert_eq!(
            c.on(BalloonSignal::ActivationToken(8, "other".into()), &token),
            None
        );
        assert_eq!(
            token
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|h| h.token.as_deref()),
            Some("tok")
        );
        assert_eq!(
            c.on(BalloonSignal::ActionInvoked(9, "default".into()), &token),
            None
        );
        assert_eq!(
            c.on(BalloonSignal::ActionInvoked(7, "open".into()), &token),
            None
        );
        assert_eq!(
            c.on(BalloonSignal::ActionInvoked(7, "default".into()), &token),
            Some(Msg::BalloonClicked(42))
        );
        assert_eq!(
            c.on(BalloonSignal::ActionInvoked(7, "default".into()), &token),
            None,
            "a balloon clicks once"
        );
        c.0.insert(3, 1);
        c.on(BalloonSignal::Closed(3), &token);
        assert!(c.0.is_empty(), "a closed balloon is forgotten");
    }

    #[test]
    fn a_balloon_button_comes_back_as_its_tag_and_index() {
        assert_eq!(
            actions(true, &["Mount and Open".into(), "Mount".into()]),
            ["button0", "Mount and Open", "button1", "Mount"],
            "buttons replace the body click"
        );
        assert_eq!(actions(true, &[]), ["default", ""]);
        assert!(actions(false, &[]).is_empty());
        let token = Token::default();
        let mut c = Clickable::default();
        c.0.insert(5, 9);
        assert_eq!(
            c.on(BalloonSignal::ActionInvoked(5, "other".into()), &token),
            None
        );
        assert_eq!(
            c.on(BalloonSignal::ActionInvoked(5, "button1".into()), &token),
            Some(Msg::BalloonButton(9, 1))
        );
        assert_eq!(
            c.on(BalloonSignal::ActionInvoked(5, "button0".into()), &token),
            None,
            "a balloon answers once"
        );
    }

    fn prop(row: &OwnedValue, key: &str) -> Option<OwnedValue> {
        let Value::Structure(s) = &**row else {
            panic!("a menu row is not a structure");
        };
        let Some(Value::Dict(props)) = s.fields().get(1) else {
            panic!("a menu row has no property dictionary");
        };
        props.iter().find_map(|(k, v)| match k {
            Value::Str(k) if k.as_str() == key => match v {
                Value::Value(inner) => Some(owned((**inner).clone())),
                other => Some(owned(other.clone())),
            },
            _ => None,
        })
    }

    /// The host's `parse_node` reads `(i, a{sv}, av)` children with these exact property names; pin the serialized shape.
    #[test]
    fn nodes_serialize_as_dbusmenu_children() {
        let mute = node_value(&Node::toggle(2, "Mute", true));
        let Value::Structure(s) = &*mute else {
            panic!("a menu row is not a structure");
        };
        assert_eq!(s.fields().first(), Some(&Value::I32(2)));
        assert_eq!(prop(&mute, "label"), Some(owned(Value::from("Mute"))));
        assert_eq!(
            prop(&mute, "toggle-type"),
            Some(owned(Value::from("checkmark")))
        );
        assert_eq!(prop(&mute, "toggle-state"), Some(owned(Value::from(1i32))));
        assert_eq!(prop(&mute, "enabled"), Some(owned(Value::from(true))));
        assert_eq!(
            prop(&node_value(&Node::toggle(2, "Mute", false)), "toggle-state"),
            Some(owned(Value::from(0i32)))
        );

        let saver = node_value(&Node::radio(16, "Power saver", true));
        assert_eq!(
            prop(&saver, "toggle-type"),
            Some(owned(Value::from("radio")))
        );
        assert_eq!(prop(&saver, "toggle-state"), Some(owned(Value::from(1i32))));

        let plain = node_value(&Node::row(1, "Exit"));
        assert_eq!(prop(&plain, "label"), Some(owned(Value::from("Exit"))));
        assert_eq!(prop(&plain, "toggle-type"), None);
        assert_eq!(
            prop(&plain, DEFAULT_PROP),
            None,
            "a plain row is no default"
        );
        assert_eq!(
            prop(
                &node_value(&Node::row(5, "Open").as_default()),
                DEFAULT_PROP
            ),
            Some(owned(Value::from(true))),
            "the default row says so"
        );

        let sep = node_value(&Node::separator(3));
        assert_eq!(
            prop(&sep, "type"),
            Some(owned(Value::from("separator"))),
            "the separator row must carry type=separator"
        );
        assert_eq!(prop(&sep, "label"), None);
    }

    #[test]
    fn group_properties_match_the_layout_row() {
        let p = node_props(&Node::toggle(2, "Mute", true));
        assert_eq!(p.get("label"), Some(&owned(Value::from("Mute"))));
        assert_eq!(p.get("toggle-state"), Some(&owned(Value::from(1i32))));
        assert_eq!(
            node_props(&Node::separator(3)).get("type"),
            Some(&owned(Value::from("separator")))
        );
    }
}
