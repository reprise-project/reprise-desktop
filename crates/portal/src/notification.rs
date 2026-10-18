//! `org.freedesktop.impl.portal.Notification`: an app's notifications passed to the compositor's `org.freedesktop.Notifications` server, and clicks passed back. An `app.` action runs through the app's `org.freedesktop.Application`; any other goes back as `ActionInvoked`. A body click with no default action activates the app. The server's `ActivationToken` goes along as `activation-token` platform data.
//! Version 2: icons as `themed` names, `bytes` or a `file-descriptor`; `markup-body` goes as the server's markup body; `transient` keeps it out of the history. A button with purpose `im.reply-with-text` becomes the server's `inline-reply` action: the balloon's reply field, whose text (`NotificationReplied`) runs the button's action as the response. Sounds and categories are dropped.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

const SERVER: &str = "org.freedesktop.Notifications";
const SERVER_PATH: &str = "/org/freedesktop/Notifications";
const DEFAULT_KEY: &str = "default";
const REPLY_KEY: &str = "inline-reply";
/// The button purpose that asks for a reply field, and the category it
/// belongs to.
const REPLY_PURPOSE: &str = "im.reply-with-text";
const IM_CATEGORY: &str = "im.received";
const REPLY_LABEL: &str = "Reply";
const MAX_BUTTONS: usize = 15;
const MAX_ICON_SIDE: u32 = 1024;
const MAX_ICON_BYTES: u64 = 16 << 20;

#[derive(Debug, Default, PartialEq)]
pub struct Parsed {
    pub title: String,
    pub body: Body,
    pub icon: Icon,
    pub urgency: u8,
    pub transient: bool,
    pub default_action: Option<Action>,
    pub buttons: Vec<(String, Action)>,
    pub reply: Option<(String, Action)>,
}

#[derive(Debug, PartialEq)]
pub enum Body {
    Plain(String),
    Markup(String),
}

impl Default for Body {
    fn default() -> Body {
        Body::Plain(String::new())
    }
}

impl Body {
    pub fn for_server(&self) -> String {
        match self {
            Body::Plain(t) => escape(t),
            Body::Markup(m) => m.clone(),
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub enum Icon {
    #[default]
    None,
    Themed(Vec<String>),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub name: String,
    pub target: Option<OwnedValue>,
}

#[derive(Debug, PartialEq)]
pub enum Invoke {
    Activate,
    ActivateAction(Action),
    Signal(Action),
}

#[derive(Debug)]
struct Shown {
    app_id: String,
    id: String,
    default_action: Option<Action>,
    buttons: Vec<Action>,
    reply: Option<Action>,
}

impl Shown {
    /// What the server's action `key` runs: `default` for the body,
    /// `button<n>` for the n-th button.
    fn route(&self, key: &str) -> Option<Invoke> {
        let action = if key == DEFAULT_KEY {
            match &self.default_action {
                Some(a) => a.clone(),
                None => return Some(Invoke::Activate),
            }
        } else {
            let n: usize = key.strip_prefix("button")?.parse().ok()?;
            self.buttons.get(n)?.clone()
        };
        Some(match action.name.strip_prefix("app.") {
            Some(name) => Invoke::ActivateAction(Action {
                name: name.to_string(),
                target: action.target,
            }),
            None => Invoke::Signal(action),
        })
    }
}

#[derive(Default)]
pub struct Book {
    by_key: HashMap<(String, String), u32>,
    shown: HashMap<u32, Shown>,
}

impl Book {
    fn insert(&mut self, server_id: u32, shown: Shown) {
        let key = (shown.app_id.clone(), shown.id.clone());
        if let Some(old) = self.by_key.insert(key, server_id)
            && old != server_id
        {
            self.shown.remove(&old);
        }
        self.shown.insert(server_id, shown);
    }

    fn take_key(&mut self, app_id: &str, id: &str) -> Option<u32> {
        let server_id = self.by_key.remove(&(app_id.to_string(), id.to_string()))?;
        self.shown.remove(&server_id);
        Some(server_id)
    }

    fn closed(&mut self, server_id: u32) {
        if let Some(s) = self.shown.remove(&server_id) {
            self.by_key.remove(&(s.app_id, s.id));
        }
    }
}

pub type Shared = Arc<Mutex<Book>>;

pub struct Notification {
    pub book: Shared,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Notification")]
impl Notification {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    #[zbus(property)]
    fn supported_options(&self) -> HashMap<String, OwnedValue> {
        let list = |v: &str| Value::from(vec![v]).try_to_owned().ok();
        [("category", IM_CATEGORY), ("button-purpose", REPLY_PURPOSE)]
            .into_iter()
            .filter_map(|(k, v)| Some((k.to_string(), list(v)?)))
            .collect()
    }

    async fn add_notification(
        &self,
        app_id: String,
        id: String,
        notification: HashMap<String, OwnedValue>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        let p = parse(&notification);
        let replaces = self
            .book
            .lock()
            .unwrap()
            .by_key
            .get(&(app_id.clone(), id.clone()))
            .copied()
            .unwrap_or(0);
        let server_id = notify(conn, &app_id, replaces, &p).await.map_err(|e| {
            log::warn!("Notification {app_id:?}/{id:?}: {e}");
            zbus::fdo::Error::Failed(format!("{SERVER}: {e}"))
        })?;
        self.book.lock().unwrap().insert(
            server_id,
            Shown {
                app_id,
                id,
                default_action: p.default_action,
                buttons: p.buttons.into_iter().map(|(_, a)| a).collect(),
                reply: p.reply.map(|(_, a)| a),
            },
        );
        Ok(())
    }

    async fn remove_notification(
        &self,
        app_id: String,
        id: String,
        #[zbus(connection)] conn: &zbus::Connection,
    ) {
        let Some(server_id) = self.book.lock().unwrap().take_key(&app_id, &id) else {
            return;
        };
        let _ = conn
            .call_method(
                Some(SERVER),
                SERVER_PATH,
                Some(SERVER),
                "CloseNotification",
                &(server_id,),
            )
            .await;
    }

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        app_id: &str,
        id: &str,
        action: &str,
        parameter: Vec<OwnedValue>,
    ) -> zbus::Result<()>;
}

/// The portal's vardict read into `Parsed`; wrong types and unknown keys
/// are left out, as the frontend already refused malformed ones.
pub fn parse(n: &HashMap<String, OwnedValue>) -> Parsed {
    let string = |k: &str| n.get(k).and_then(|v| String::try_from(v.clone()).ok());
    let default_action = string("default-action").map(|name| Action {
        name,
        target: n.get("default-action-target").cloned(),
    });
    let mut reply = None;
    let buttons = n
        .get("buttons")
        .and_then(|v| Vec::<HashMap<String, OwnedValue>>::try_from(v.clone()).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|b| {
            let s = |k: &str| b.get(k).and_then(|v| String::try_from(v.clone()).ok());
            let name = s("action").filter(|a| !a.is_empty())?;
            let target = b.get("target").cloned();
            let label = s("label").filter(|l| !l.is_empty());
            if s("purpose").as_deref() == Some(REPLY_PURPOSE) {
                if reply.is_none() {
                    let label = label.unwrap_or_else(|| REPLY_LABEL.to_string());
                    reply = Some((label, Action { name, target }));
                }
                return None;
            }
            Some((label?, Action { name, target }))
        })
        .take(MAX_BUTTONS)
        .collect();
    // A markup body stands in for the plain one when both come.
    let body = match string("markup-body") {
        Some(m) => Body::Markup(m),
        None => Body::Plain(string("body").unwrap_or_default()),
    };
    let transient = n
        .get("display-hint")
        .and_then(|v| Vec::<String>::try_from(v.clone()).ok())
        .is_some_and(|hints| hints.iter().any(|h| h == "transient"));
    Parsed {
        title: string("title").unwrap_or_default(),
        body,
        transient,
        icon: n.get("icon").map(parse_icon).unwrap_or_default(),
        urgency: match string("priority").as_deref() {
            Some("low") => 0,
            Some("urgent") => 2,
            _ => 1,
        },
        default_action,
        buttons,
        reply,
    }
}

/// A serialized icon: `("themed", as)`, `("bytes", ay)` or
/// `("file-descriptor", h)`, whose file is read here into `Bytes`.
fn parse_icon(v: &OwnedValue) -> Icon {
    let Value::Structure(st) = &**v else {
        return Icon::None;
    };
    let [Value::Str(kind), inner] = st.fields() else {
        return Icon::None;
    };
    // The second field is a variant.
    let inner = match inner {
        Value::Value(v) => &**v,
        v => v,
    };
    if let ("file-descriptor", Value::Fd(fd)) = (kind.as_str(), inner) {
        return read_icon(fd)
            .map_err(|e| log::debug!("notification icon: {e}"))
            .map(Icon::Bytes)
            .unwrap_or_default();
    }
    let Ok(inner) = inner.try_to_owned() else {
        return Icon::None;
    };
    match kind.as_str() {
        "themed" => Vec::<String>::try_from(inner)
            .map(Icon::Themed)
            .unwrap_or_default(),
        "bytes" => Vec::<u8>::try_from(inner)
            .map(Icon::Bytes)
            .unwrap_or_default(),
        _ => Icon::None,
    }
}

/// The whole file behind an icon's descriptor, read from its start
/// whatever its offset; an error for one over `MAX_ICON_BYTES`.
fn read_icon(fd: &zbus::zvariant::Fd<'_>) -> std::io::Result<Vec<u8>> {
    use std::os::fd::AsFd;
    use std::os::unix::fs::FileExt;
    let file = std::fs::File::from(fd.as_fd().try_clone_to_owned()?);
    let mut out = Vec::new();
    let mut chunk = vec![0u8; 64 << 10];
    loop {
        let n = file.read_at(&mut chunk, out.len() as u64)?;
        if n == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&chunk[..n]);
        if out.len() as u64 > MAX_ICON_BYTES {
            return Err(std::io::Error::other("icon file too large"));
        }
    }
}

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

pub fn action_list(p: &Parsed) -> Vec<String> {
    let mut out = vec![DEFAULT_KEY.to_string(), String::new()];
    for (i, (label, _)) in p.buttons.iter().enumerate() {
        out.push(format!("button{i}"));
        out.push(label.clone());
    }
    if let Some((label, _)) = &p.reply {
        out.push(REPLY_KEY.to_string());
        out.push(label.clone());
    }
    out
}

/// The `image-data` hint: width, height, row stride, has alpha, bits per
/// sample, channels, pixels.
type ImageData = (i32, i32, i32, bool, i32, i32, Vec<u8>);

/// An encoded picture as the server's `image-data` hint,
/// `(iiibiiay)` of straight RGBA; None when it does not decode or is
/// larger than `MAX_ICON_SIDE` on a side.
fn image_data(bytes: &[u8]) -> Option<ImageData> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_ICON_SIDE);
    limits.max_image_height = Some(MAX_ICON_SIDE);
    reader.limits(limits);
    let img = reader
        .decode()
        .map_err(|e| log::debug!("notification icon: {e}"))
        .ok()?
        .into_rgba8();
    let (w, h) = (img.width() as i32, img.height() as i32);
    Some((w, h, w * 4, true, 8, 4, img.into_raw()))
}

async fn notify(
    conn: &zbus::Connection,
    app_id: &str,
    replaces: u32,
    p: &Parsed,
) -> zbus::Result<u32> {
    let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
    hints.insert("urgency", Value::U8(p.urgency));
    if p.transient {
        hints.insert("transient", Value::Bool(true));
    }
    if !app_id.is_empty() {
        hints.insert("desktop-entry", Value::from(app_id));
    }
    let mut app_icon = String::new();
    match &p.icon {
        Icon::Themed(names) => app_icon = names.first().cloned().unwrap_or_default(),
        Icon::Bytes(bytes) => {
            if let Some(data) = image_data(bytes) {
                hints.insert("image-data", Value::from(data));
            }
        }
        Icon::None => {}
    }
    let reply = conn
        .call_method(
            Some(SERVER),
            SERVER_PATH,
            Some(SERVER),
            "Notify",
            &(
                app_id,
                replaces,
                app_icon.as_str(),
                p.title.as_str(),
                p.body.for_server(),
                action_list(p),
                hints,
                -1i32,
            ),
        )
        .await?;
    reply.body().deserialize::<u32>()
}

/// The object path `org.freedesktop.Application` is served at for
/// `app_id`: dots become slashes and dashes underscores.
pub fn app_path(app_id: &str) -> String {
    format!("/{}", app_id.replace('.', "/").replace('-', "_"))
}

fn platform_data(token: Option<&str>) -> HashMap<&'static str, Value<'_>> {
    token
        .map(|t| ("activation-token", Value::from(t)))
        .into_iter()
        .collect()
}

/// What a click on `server_id`'s `key` runs, with the app id and the
/// app's notification id; None for a notification that is not ours.
/// Read as the click comes in, since the server's `NotificationClosed`
/// for it follows at once.
fn clicked(book: &Book, server_id: u32, key: &str) -> Option<(String, String, Invoke)> {
    let s = book.shown.get(&server_id)?;
    Some((s.app_id.clone(), s.id.clone(), s.route(key)?))
}

fn replied(book: &Book, server_id: u32) -> Option<(String, String, Action)> {
    let s = book.shown.get(&server_id)?;
    Some((s.app_id.clone(), s.id.clone(), s.reply.clone()?))
}

/// Run reply action `a` of `app_id`'s notification `id` with `text` as
/// its response: an `app.` action's parameter is its target then the
/// text; any other (or one the app does not answer on the bus) goes back
/// as `ActionInvoked` with the text after the platform data.
async fn reply(conn: &zbus::Connection, app_id: &str, id: &str, a: Action, text: &str) {
    if let Some(name) = a.name.strip_prefix("app.") {
        let mut param: Vec<Value<'_>> = a.target.iter().map(|t| Value::from(t.clone())).collect();
        param.push(Value::from(text));
        let body = (name, param, platform_data(None));
        match app_call(conn, app_id, "ActivateAction", &body).await {
            Ok(()) => return,
            Err(e) => log::info!("Notification {app_id:?}/{id:?}: {e}; telling the app instead"),
        }
    }
    signal(conn, app_id, id, &a, None, Some(text)).await;
}

/// Run `what` for the click on `app_id`'s notification `id`. An `app.`
/// action the app does not answer on the bus (not running, not D-Bus
/// activatable, no app id) goes back to it as `ActionInvoked` instead.
async fn invoke(
    conn: &zbus::Connection,
    app_id: &str,
    id: &str,
    what: Invoke,
    token: Option<&str>,
) {
    let platform = platform_data(token);
    match what {
        Invoke::Activate => {
            if let Err(e) = app_call(conn, app_id, "Activate", &(platform,)).await {
                log::info!("Notification {app_id:?}/{id:?}: Activate: {e}");
            }
        }
        Invoke::ActivateAction(a) => {
            let param: Vec<Value<'_>> = a.target.iter().map(|t| Value::from(t.clone())).collect();
            let body = (a.name.as_str(), param, platform);
            if let Err(e) = app_call(conn, app_id, "ActivateAction", &body).await {
                log::info!("Notification {app_id:?}/{id:?}: {e}; telling the app instead");
                let a = Action {
                    name: format!("app.{}", a.name),
                    target: a.target,
                };
                signal(conn, app_id, id, &a, token, None).await;
            }
        }
        Invoke::Signal(a) => signal(conn, app_id, id, &a, token, None).await,
    }
}

async fn app_call<B>(
    conn: &zbus::Connection,
    app_id: &str,
    method: &str,
    body: &B,
) -> zbus::Result<()>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    if app_id.is_empty() || zbus::names::BusName::try_from(app_id).is_err() {
        return Err(zbus::Error::Failure(format!(
            "{app_id:?} is not a bus name"
        )));
    }
    let iface = Some("org.freedesktop.Application");
    let path = app_path(app_id);
    conn.call_method(Some(app_id), path.as_str(), iface, method, body)
        .await
        .map(drop)
}

async fn signal(
    conn: &zbus::Connection,
    app_id: &str,
    id: &str,
    a: &Action,
    token: Option<&str>,
    response: Option<&str>,
) {
    let mut parameter: Vec<OwnedValue> = a.target.iter().cloned().collect();
    if let Ok(v) = Value::from(platform_data(token)).try_to_owned() {
        parameter.push(v);
    }
    if let Some(Ok(v)) = response.map(|r| Value::from(r).try_to_owned()) {
        parameter.push(v);
    }
    let Ok(iface) = conn
        .object_server()
        .interface::<_, Notification>(crate::PATH)
        .await
    else {
        return;
    };
    let _ =
        Notification::action_invoked(iface.signal_emitter(), app_id, id, &a.name, parameter).await;
}

/// Follow the server's `ActivationToken`, `ActionInvoked`,
/// `NotificationReplied` and `NotificationClosed` for the notifications
/// in `book`. Runs for as long
/// as the connection does. One rule for the whole interface keeps the
/// token ahead of the click it belongs to; each click runs in its own
/// task, so an app slow to answer never stalls the stream.
pub async fn watch(conn: zbus::Connection, book: Shared) {
    use futures_lite::StreamExt;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(SERVER)
        .and_then(|b| b.path(SERVER_PATH))
        .map(|b| b.build().into());
    let rules = match rule {
        Ok(r) => vec![r],
        Err(e) => {
            log::warn!("Notification: {e}");
            return;
        }
    };
    let mut signals = reprisebus::signals(&conn, SERVER, rules).await;
    // The token the server sent last, for the `ActionInvoked` that follows.
    let mut token: Option<(u32, String)> = None;
    while let Some(sig) = signals.next().await {
        let m = match sig {
            reprisebus::Signal::Message(m) => m,
            // A new server numbers its notifications afresh.
            reprisebus::Signal::OwnerChanged => {
                *book.lock().unwrap() = Book::default();
                continue;
            }
        };
        let member = m.header().member().map(|m| m.to_string());
        match member.as_deref() {
            Some("ActivationToken") => {
                token = m.body().deserialize::<(u32, String)>().ok();
            }
            Some("ActionInvoked") => {
                let Ok((server_id, key)) = m.body().deserialize::<(u32, String)>() else {
                    continue;
                };
                let token = token
                    .take()
                    .filter(|(id, _)| *id == server_id)
                    .map(|(_, t)| t);
                let Some((app_id, id, what)) = clicked(&book.lock().unwrap(), server_id, &key)
                else {
                    continue;
                };
                let runs = match &what {
                    Invoke::Activate => "Activate",
                    Invoke::ActivateAction(a) | Invoke::Signal(a) => a.name.as_str(),
                };
                log::info!("Notification {app_id:?}/{id:?}: {key} runs {runs:?}");
                let conn2 = conn.clone();
                conn.executor()
                    .spawn(
                        async move { invoke(&conn2, &app_id, &id, what, token.as_deref()).await },
                        "notification click",
                    )
                    .detach();
            }
            Some("NotificationReplied") => {
                let Ok((server_id, text)) = m.body().deserialize::<(u32, String)>() else {
                    continue;
                };
                let Some((app_id, id, a)) = replied(&book.lock().unwrap(), server_id) else {
                    continue;
                };
                log::info!("Notification {app_id:?}/{id:?}: a reply runs {:?}", a.name);
                let conn2 = conn.clone();
                conn.executor()
                    .spawn(
                        async move { reply(&conn2, &app_id, &id, a, &text).await },
                        "notification reply",
                    )
                    .detach();
            }
            Some("NotificationClosed") => {
                if let Ok((server_id, _)) = m.body().deserialize::<(u32, u32)>() {
                    book.lock().unwrap().closed(server_id);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned<'a>(v: impl Into<Value<'a>>) -> OwnedValue {
        v.into().try_to_owned().unwrap()
    }

    fn button(label: &str, action: &str) -> HashMap<String, OwnedValue> {
        let mut b = HashMap::new();
        b.insert("label".to_string(), owned(label));
        b.insert("action".to_string(), owned(action));
        b
    }

    #[test]
    fn a_portal_notification_parses() {
        let mut n = HashMap::new();
        n.insert("title".to_string(), owned("Hi"));
        n.insert("body".to_string(), owned("1 < 2"));
        n.insert("priority".to_string(), owned("urgent"));
        n.insert("default-action".to_string(), owned("app.open"));
        n.insert("default-action-target".to_string(), owned(7u32));
        n.insert(
            "icon".to_string(),
            owned(("themed", Value::from(vec!["mail-unread", "mail"]))),
        );
        n.insert(
            "buttons".to_string(),
            owned(vec![button("Reply", "reply"), button("", "nolabel")]),
        );
        let p = parse(&n);
        assert_eq!(p.title, "Hi");
        assert_eq!(p.urgency, 2);
        assert_eq!(
            p.icon,
            Icon::Themed(vec!["mail-unread".into(), "mail".into()])
        );
        assert_eq!(p.default_action.as_ref().unwrap().name, "app.open");
        assert_eq!(p.buttons.len(), 1, "a button without a label is dropped");
        assert_eq!(
            action_list(&p),
            ["default", "", "button0", "Reply"].map(String::from)
        );
    }

    #[test]
    fn a_reply_purpose_button_becomes_the_reply_field() {
        let mut reply = button("", "app.reply");
        reply.insert("purpose".to_string(), owned(REPLY_PURPOSE));
        reply.insert("target".to_string(), owned("chat-7"));
        let mut second = button("Answer", "app.answer");
        second.insert("purpose".to_string(), owned(REPLY_PURPOSE));
        let mut n = HashMap::new();
        n.insert(
            "buttons".to_string(),
            owned(vec![button("Mute", "mute"), reply, second]),
        );
        let p = parse(&n);
        assert_eq!(p.buttons.len(), 1);
        let (label, a) = p.reply.clone().unwrap();
        assert_eq!((label.as_str(), a.name.as_str()), ("Reply", "app.reply"));
        assert_eq!(
            action_list(&p),
            ["default", "", "button0", "Mute", "inline-reply", "Reply"].map(String::from)
        );
        let mut book = Book::default();
        book.insert(
            3,
            Shown {
                app_id: "org.example.Chat".into(),
                id: "m1".into(),
                default_action: None,
                buttons: vec![],
                reply: Some(a.clone()),
            },
        );
        assert_eq!(
            replied(&book, 3),
            Some(("org.example.Chat".into(), "m1".into(), a))
        );
        assert_eq!(replied(&book, 4), None);
    }

    #[test]
    fn the_reply_field_is_advertised() {
        let opts = Notification {
            book: Shared::default(),
        }
        .supported_options();
        let list = |k: &str| Vec::<String>::try_from(opts[k].clone()).unwrap();
        assert_eq!(list("button-purpose"), [REPLY_PURPOSE]);
        assert_eq!(list("category"), [IM_CATEGORY]);
    }

    #[test]
    fn bytes_icons_and_unknown_priorities() {
        let mut n = HashMap::new();
        n.insert(
            "icon".to_string(),
            owned(("bytes", Value::from(vec![1u8, 2, 3]))),
        );
        n.insert("priority".to_string(), owned("low"));
        let p = parse(&n);
        assert_eq!(p.icon, Icon::Bytes(vec![1, 2, 3]));
        assert_eq!(p.urgency, 0);
        n.insert("priority".to_string(), owned("high"));
        assert_eq!(parse(&n).urgency, 1);
    }

    #[test]
    fn a_markup_body_passes_as_it_is_and_a_plain_one_is_escaped() {
        let mut n = HashMap::new();
        n.insert("body".to_string(), owned("1 < 2"));
        let p = parse(&n);
        assert_eq!(p.body.for_server(), "1 &lt; 2");
        assert!(!p.transient);
        n.insert("markup-body".to_string(), owned("<b>Hi</b>"));
        n.insert(
            "display-hint".to_string(),
            owned(vec!["show-as-new", "transient"]),
        );
        let p = parse(&n);
        assert_eq!(p.body, Body::Markup("<b>Hi</b>".into()));
        assert_eq!(p.body.for_server(), "<b>Hi</b>");
        assert!(p.transient, "the transient display hint is kept");
    }

    #[test]
    fn a_file_descriptor_icon_is_read_from_its_start() {
        use std::io::{Seek, Write};
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"picture").unwrap();
        file.seek(std::io::SeekFrom::Start(3)).unwrap();
        let fd = zbus::zvariant::Fd::from(std::os::fd::OwnedFd::from(file));
        let mut n = HashMap::new();
        n.insert(
            "icon".to_string(),
            owned(("file-descriptor", Value::Fd(fd))),
        );
        assert_eq!(parse(&n).icon, Icon::Bytes(b"picture".to_vec()));
    }

    #[test]
    fn clicks_route_by_prefix() {
        let s = Shown {
            app_id: "org.example.App".into(),
            id: "n".into(),
            reply: None,
            default_action: None,
            buttons: vec![
                Action {
                    name: "app.reply".into(),
                    target: None,
                },
                Action {
                    name: "dismiss".into(),
                    target: None,
                },
            ],
        };
        assert_eq!(s.route("default"), Some(Invoke::Activate));
        assert_eq!(
            s.route("button0"),
            Some(Invoke::ActivateAction(Action {
                name: "reply".into(),
                target: None
            }))
        );
        assert_eq!(
            s.route("button1"),
            Some(Invoke::Signal(Action {
                name: "dismiss".into(),
                target: None
            }))
        );
        assert_eq!(s.route("button2"), None);
        assert_eq!(s.route("other"), None);
    }

    #[test]
    fn the_book_follows_replacements_and_closes() {
        let shown = |id: &str| Shown {
            app_id: "a".into(),
            id: id.into(),
            default_action: None,
            buttons: vec![],
            reply: None,
        };
        let mut b = Book::default();
        b.insert(5, shown("x"));
        b.insert(6, shown("x"));
        assert!(!b.shown.contains_key(&5), "a new server id drops the old");
        b.insert(7, shown("y"));
        b.closed(7);
        assert_eq!(b.take_key("a", "y"), None);
        assert_eq!(b.take_key("a", "x"), Some(6));
        assert!(b.shown.is_empty());
    }

    #[test]
    fn bodies_are_escaped_and_paths_derived() {
        assert_eq!(escape("a & <b>"), "a &amp; &lt;b&gt;");
        assert_eq!(app_path("org.gnome.Foo-Bar"), "/org/gnome/Foo_Bar");
    }

    #[test]
    fn a_png_icon_becomes_image_data() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(2, 3, image::Rgba([1, 2, 3, 4]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let (w, h, stride, alpha, bits, ch, data) = image_data(&png).unwrap();
        assert_eq!((w, h, stride, alpha, bits, ch), (2, 3, 8, true, 8, 4));
        assert_eq!(&data[..4], &[1, 2, 3, 4]);
        assert!(image_data(b"not a picture").is_none());
    }

    type Got = (u32, String, Vec<String>, Vec<String>);

    struct FakeServer {
        got: Arc<Mutex<Vec<Got>>>,
    }

    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl FakeServer {
        #[allow(clippy::too_many_arguments)]
        fn notify(
            &self,
            _app_name: String,
            replaces_id: u32,
            _app_icon: String,
            summary: String,
            _body: String,
            actions: Vec<String>,
            hints: HashMap<String, OwnedValue>,
            _expire_timeout: i32,
        ) -> u32 {
            let mut keys: Vec<String> = hints.into_keys().collect();
            keys.sort();
            let mut got = self.got.lock().unwrap();
            got.push((replaces_id, summary, actions, keys));
            if replaces_id != 0 {
                replaces_id
            } else {
                40 + got.len() as u32
            }
        }

        fn close_notification(&self, _id: u32) {}
    }

    /// Round trip on a real session bus; needs one: `dbus-run-session cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn round_trip_on_a_session_bus() {
        use futures_lite::StreamExt;
        zbus::block_on(async {
            let got = Arc::new(Mutex::new(Vec::new()));
            let server = zbus::connection::Builder::session()
                .unwrap()
                .name(SERVER)
                .unwrap()
                .serve_at(SERVER_PATH, FakeServer { got: got.clone() })
                .unwrap()
                .build()
                .await
                .unwrap();
            let book = Shared::default();
            let portal = zbus::connection::Builder::session()
                .unwrap()
                .serve_at(crate::PATH, Notification { book: book.clone() })
                .unwrap()
                .build()
                .await
                .unwrap();
            portal
                .executor()
                .spawn(watch(portal.clone(), book.clone()), "watch")
                .detach();
            let client = zbus::Connection::session().await.unwrap();
            let dest = portal.unique_name().unwrap().to_string();
            let mut invoked = zbus::MessageStream::for_match_rule(
                zbus::MatchRule::builder()
                    .msg_type(zbus::message::Type::Signal)
                    .interface("org.freedesktop.impl.portal.Notification")
                    .unwrap()
                    .member("ActionInvoked")
                    .unwrap()
                    .build(),
                &client,
                None,
            )
            .await
            .unwrap();
            let mut png = Vec::new();
            image::RgbaImage::from_pixel(4, 4, image::Rgba([9, 9, 9, 255]))
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .unwrap();
            let mut n: HashMap<&str, Value<'_>> = HashMap::new();
            n.insert("title", Value::from("Build done"));
            n.insert("icon", Value::from(("bytes", Value::from(png))));
            let mut b: HashMap<&str, Value<'_>> = HashMap::new();
            b.insert("label", Value::from("Show"));
            b.insert("action", Value::from("show"));
            b.insert("target", Value::from("log"));
            n.insert("buttons", Value::from(vec![b]));
            for _ in 0..2 {
                client
                    .call_method(
                        Some(dest.as_str()),
                        crate::PATH,
                        Some("org.freedesktop.impl.portal.Notification"),
                        "AddNotification",
                        &("org.example.App", "n1", &n),
                    )
                    .await
                    .unwrap();
            }
            {
                let got = got.lock().unwrap();
                assert_eq!(got.len(), 2);
                assert_eq!(got[0].0, 0);
                assert_eq!(got[1].0, 41, "the second call replaces the first");
                assert_eq!(
                    got[0].2,
                    ["default", "", "button0", "Show"].map(String::from)
                );
                assert_eq!(
                    got[0].3,
                    ["desktop-entry", "image-data", "urgency"].map(String::from)
                );
            }
            let emitter = SignalEmitter::new(&server, SERVER_PATH).unwrap();
            emitter
                .emit(SERVER, "ActivationToken", &(41u32, "tok"))
                .await
                .unwrap();
            emitter
                .emit(SERVER, "ActionInvoked", &(41u32, "button0"))
                .await
                .unwrap();
            emitter
                .emit(SERVER, "NotificationClosed", &(41u32, 2u32))
                .await
                .unwrap();
            let m = invoked.next().await.unwrap().unwrap();
            let (app, id, action, param): (String, String, String, Vec<OwnedValue>) =
                m.body().deserialize().unwrap();
            assert_eq!(
                (app.as_str(), id.as_str(), action.as_str()),
                ("org.example.App", "n1", "show")
            );
            assert_eq!(String::try_from(param[0].clone()).unwrap(), "log");
            assert_eq!(param.len(), 2, "the target, then the platform data");
            let platform = HashMap::<String, OwnedValue>::try_from(param[1].clone()).unwrap();
            assert_eq!(
                String::try_from(platform["activation-token"].clone()).unwrap(),
                "tok"
            );
        });
    }
}
