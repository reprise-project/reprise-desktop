use super::item::{icon_ref, owner_pid};
use super::{Icons, emit, lock};
use crate::tray::{
    ACTION_ICON_SIZES, BODY_CHARS, NOTIFY_SIZES, Notification, REPLY_SEND, Reply, SUMMARY_CHARS,
    TrayEvent, clip, decode_named_at, notify,
};
use reprisekit::icon::IconSource;
use std::collections::HashMap;
use std::os::fd::RawFd;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use zbus::Connection;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;

/// Connection, app name and PID per open notification. A replace is honoured from the connection,
/// or under the same app name from its process or once it left the bus (`notify-send -r` is a new
/// process each time); a close only from the connection.
#[derive(Default)]
pub(super) struct Senders(pub(super) HashMap<u32, (String, String, Option<i32>)>);

impl Senders {
    /// The connection that sent `id` when `sender`, another process under the same app name,
    /// asks to replace it: the replace waits on whether that connection is still on the bus.
    pub(super) fn other_sender(
        &self,
        id: u32,
        sender: &str,
        pid: Option<i32>,
        app: &str,
    ) -> Option<String> {
        let (s, a, p) = self.0.get(&id)?;
        let other = s != sender && !app.is_empty() && a == app && (pid.is_none() || *p != pid);
        other.then(|| s.clone())
    }

    /// The id for a notification from `sender`: `replaces` when it may replace it (`left` says
    /// the [`Senders::other_sender`] has left the bus), else `fresh()`.
    pub(super) fn claim(
        &mut self,
        sender: &str,
        pid: Option<i32>,
        app: &str,
        replaces: u32,
        left: bool,
        fresh: impl FnOnce() -> u32,
    ) -> u32 {
        let id = match self.0.get(&replaces) {
            Some((s, _, _)) if s == sender => replaces,
            Some((_, a, p))
                if !app.is_empty() && a == app && ((pid.is_some() && *p == pid) || left) =>
            {
                return replaces;
            }
            _ => fresh(),
        };
        self.0
            .insert(id, (sender.to_string(), app.to_string(), pid));
        id
    }

    pub(super) fn owns(&self, id: u32, sender: &str) -> bool {
        self.0.get(&id).is_some_and(|(s, _, _)| s == sender)
    }

    pub(super) fn sent_by(&self, sender: &str) -> bool {
        self.0.values().any(|(s, _, _)| s == sender)
    }
}

/// Whether `name` has an owner on the bus; true when the bus cannot say.
async fn on_bus(conn: &Connection, name: &str) -> bool {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else {
        return true;
    };
    let Ok(name) = zbus::names::BusName::try_from(name) else {
        return false;
    };
    dbus.name_has_owner(name).await.unwrap_or(true)
}

pub(super) struct Notifications {
    pub(super) events: Sender<TrayEvent>,
    pub(super) eventfd: RawFd,
    pub(super) next_id: std::sync::atomic::AtomicU32,
    pub(super) senders: Arc<Mutex<Senders>>,
    pub(super) icons: Icons,
}

pub(super) const CAPABILITIES: [&str; 7] = [
    "action-icons",
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
        let pid = owner_pid(conn, &sender).await;
        let other = lock(&self.senders).other_sender(replaces_id, &sender, pid, &app_name);
        let left = match other {
            Some(other) => !on_bus(conn, &other).await,
            None => false,
        };
        let id = lock(&self.senders).claim(
            &sender,
            pid,
            &app_name,
            replaces_id,
            left,
            // Ids start at 1; 0 is "none" in the protocol.
            || self.next_id.fetch_add(1, Ordering::Relaxed).max(1),
        );
        let hints = notify::hints(hints);
        let sound = hints.balloon_sound();
        let entry = hints.desktop_entry.as_deref().and_then(desktop_entry);
        let (app_icon, image) = notification_icons(&self.icons, &hints, app_icon, entry.as_ref());
        let app = entry.map_or(app_name, |e| e.name);
        let actions = notify::actions(&actions);
        let action_icons = if hints.action_icons {
            action_icons(&self.icons, &actions)
        } else {
            HashMap::new()
        };
        let reply = actions
            .iter()
            .any(|(k, _)| k == notify::INLINE_REPLY)
            .then(|| Reply {
                placeholder: clip(
                    hints.reply_placeholder.unwrap_or_default(),
                    notify::ACTION_LABEL_CHARS,
                ),
                send: clip(
                    hints.reply_send.unwrap_or_else(|| REPLY_SEND.into()),
                    notify::ACTION_LABEL_CHARS,
                ),
            });
        emit(
            &self.events,
            self.eventfd,
            TrayEvent::Notify(Notification {
                id,
                app: clip(app, SUMMARY_CHARS),
                summary: clip(summary, SUMMARY_CHARS),
                body: clip(notify::plain_text(&body), BODY_CHARS),
                app_icon,
                image,
                expire_timeout,
                actions,
                action_icons,
                urgency: hints.urgency,
                transient: hints.transient,
                resident: hints.resident,
                sender,
                pid,
                reply,
                sound,
            }),
        );
        id
    }

    async fn close_notification(&self, id: u32, #[zbus(header)] hdr: Header<'_>) {
        let sender = hdr.sender().map_or("", |s| s.as_str());
        if lock(&self.senders).owns(id, sender) {
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
    pub(super) async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub(super) async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub(super) async fn notification_replied(
        emitter: &SignalEmitter<'_>,
        id: u32,
        text: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub(super) async fn activation_token(
        emitter: &SignalEmitter<'_>,
        id: u32,
        activation_token: &str,
    ) -> zbus::Result<()>;
}

/// Each of `actions`' [`notify::icon_keys`] that resolves as a theme icon, decoded.
pub(super) fn action_icons(
    icons: &Icons,
    actions: &[(String, String)],
) -> HashMap<String, Vec<IconSource>> {
    notify::icon_keys(actions)
        .filter_map(|k| {
            let sources = decode_named_at(&mut lock(icons), k, &[], &ACTION_ICON_SIZES);
            (!sources.is_empty()).then(|| (k.to_string(), sources))
        })
        .collect()
}

fn desktop_entry(hint: &str) -> Option<reprisefs::apps::AppEntry> {
    let id = notify::desktop_id(hint)?;
    let path = reprisefs::apps::find_file(&id)?;
    let text = reprisefs::apps::read_desktop_file(&path).ok()?;
    reprisefs::apps::parse_desktop(&text, &id)
}

/// `(app_icon, image)`: the app icon name, else the desktop entry's icon;
/// the `image-data` hint, else `image-path`. Each is the first that resolves.
pub(super) fn notification_icons(
    icons: &Icons,
    hints: &notify::Hints,
    app_icon: String,
    entry: Option<&reprisefs::apps::AppEntry>,
) -> (Vec<IconSource>, Vec<IconSource>) {
    let named = |name: Option<String>| {
        icon_ref(name)
            .map(|name| decode_named_at(&mut lock(icons), &name, &[], &NOTIFY_SIZES))
            .unwrap_or_default()
    };
    let image = hints
        .image
        .as_ref()
        .map(notify::image_sources)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let path = hints
                .image_path
                .as_deref()
                .map(|p| match p.strip_prefix("file://") {
                    Some(rest) => String::from_utf8_lossy(&reprisefs::trash::percent_decode(rest))
                        .into_owned(),
                    None => p.to_string(),
                });
            let file = path.clone().filter(|p| p.starts_with('/'));
            let sources = named(path);
            match file {
                Some(file) if sources.is_empty() => {
                    notify::picture_file_sources(std::path::Path::new(&file))
                }
                _ => sources,
            }
        });
    let app = [Some(app_icon), entry.and_then(|e| e.icon.clone())]
        .into_iter()
        .map(named)
        .find(|sources| !sources.is_empty())
        .unwrap_or_default();
    (app, image)
}
