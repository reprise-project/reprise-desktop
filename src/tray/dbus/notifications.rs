use super::item::{icon_ref, owner_pid};
use super::{Icons, emit, lock};
use crate::tray::{
    ACTION_ICON_SIZES, BODY_CHARS, NOTIFY_SIZES, Notification, REPLY_SEND, Reply, SUMMARY_CHARS,
    TrayEvent, clip, decode_named_at, notify,
};
use repriseaudio::sounds::Sound;
use reprisebus::peer::{AppIdentity, Identities};
use reprisefs::names::shown_line;
use reprisekit::icon::IconSource;
use std::collections::HashMap;
use std::os::fd::RawFd;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zbus::Connection;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;

/// Connection, app name and PID per open notification. A replace from the connection, the same
/// app's process, or once it left the bus (`notify-send -r`) takes it over; a close only from it.
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
    /// the [`Senders::other_sender`] has left the bus, and `sender` takes its place), else the
    /// next of `fresh()` that is neither 0, the compositor's own nor open.
    pub(super) fn claim(
        &mut self,
        sender: &str,
        pid: Option<i32>,
        app: &str,
        replaces: u32,
        left: bool,
        mut fresh: impl FnMut() -> u32,
    ) -> u32 {
        let same_app = |a: &String| !app.is_empty() && a == app;
        let id = match self.0.get(&replaces) {
            Some((s, _, _)) if s == sender => replaces,
            Some((_, a, p)) if same_app(a) && pid.is_some() && *p == pid && !left => replaces,
            Some((_, a, _)) if same_app(a) && left => replaces,
            _ => loop {
                let id = fresh();
                if id != 0 && id != crate::notifyview::OWN_ID && !self.0.contains_key(&id) {
                    break id;
                }
            },
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

/// The programs that sent notifications this session as (key, name), in the order they first
/// came; Settings lists them beside `[notifications] no_balloons` and `no_sounds`.
#[derive(Default)]
pub(super) struct Programs(pub(super) Vec<(String, String)>);

impl Programs {
    const MAX: usize = 200;

    pub(super) fn saw(&mut self, key: &str, name: &str) {
        if key.is_empty() {
            return;
        }
        let full = self.0.len() >= Self::MAX;
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some(p) => p.1 = name.to_string(),
            None if !full => self.0.push((key.to_string(), name.to_string())),
            None => {}
        }
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
    pub(super) next_id: Arc<std::sync::atomic::AtomicU32>,
    pub(super) senders: Arc<Mutex<Senders>>,
    pub(super) programs: Arc<Mutex<Programs>>,
    pub(super) icons: Icons,
    pub(super) identities: Arc<Identities>,
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
        actions: notify::ActionList,
        hints: notify::HintMap<'_>,
        expire_timeout: i32,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
    ) -> u32 {
        let sender = hdr.sender().map_or("", |s| s.as_str()).to_string();
        let mut hints = notify::hints(hints.0);
        let identity = self.identities.of(conn, &sender).await;
        let (app_name, app_icon) = as_sent_by(&identity, &mut hints, app_name, app_icon);
        let pid = owner_pid(conn, &sender).await;
        let other = lock(&self.senders).other_sender(replaces_id, &sender, pid, &app_name);
        let left = match other {
            Some(other) => !on_bus(conn, &other).await,
            None => false,
        };
        let id = lock(&self.senders).claim(&sender, pid, &app_name, replaces_id, left, || {
            self.next_id.fetch_add(1, Ordering::Relaxed)
        });
        let sound = hints.balloon_sound();
        let entry = hints.desktop_entry.as_deref().and_then(desktop_entry);
        let (app_icon, image) =
            notification_icons(&self.icons, &hints, app_icon, entry.as_ref()).await;
        let key = clip(
            notify::program_key(hints.desktop_entry.as_deref(), &app_name),
            notify::KEY_CHARS,
        );
        let app = shown_line(&entry.map_or(app_name, |e| e.name), SUMMARY_CHARS);
        lock(&self.programs).saw(&key, &app);
        let actions = notify::actions(&actions.0);
        let action_icons = if hints.action_icons {
            action_icons(&self.icons, &actions)
        } else {
            HashMap::new()
        };
        let reply = actions
            .iter()
            .any(|(k, _)| k == notify::INLINE_REPLY)
            .then(|| Reply {
                placeholder: shown_line(
                    &hints.reply_placeholder.unwrap_or_default(),
                    notify::ACTION_LABEL_CHARS,
                ),
                send: shown_line(
                    &hints.reply_send.unwrap_or_else(|| REPLY_SEND.into()),
                    notify::ACTION_LABEL_CHARS,
                ),
            });
        emit(
            &self.events,
            self.eventfd,
            TrayEvent::Notify(Notification {
                id,
                app,
                key,
                summary: shown_line(&summary, SUMMARY_CHARS),
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

/// The app name, app icon and `desktop-entry` a notification from `identity` goes under: a
/// sandboxed app's own app id, nothing for an unknown sender, a host's own (docs/security.md).
/// Only a host names a picture or sound file, which would be read with the user's rights.
pub(super) fn as_sent_by(
    identity: &AppIdentity,
    hints: &mut notify::Hints<'_>,
    app_name: String,
    app_icon: String,
) -> (String, String) {
    if !matches!(identity, AppIdentity::Host { .. }) {
        hints.image_path.take_if(|p| p.contains('/'));
        hints.sound.take_if(|s| matches!(s, Sound::File(_)));
    }
    match identity {
        AppIdentity::Host { .. } => (app_name, app_icon),
        AppIdentity::Sandboxed { app_id } => {
            hints.desktop_entry = Some(app_id.clone());
            (app_id.clone(), String::new())
        }
        AppIdentity::Unknown => {
            hints.desktop_entry = None;
            (String::new(), String::new())
        }
    }
}

pub(super) fn desktop_entry(hint: &str) -> Option<reprisefs::apps::AppEntry> {
    let (path, id) = (notify::desktop_ids(hint).into_iter())
        .find_map(|id| Some((reprisefs::apps::find_file(&id)?, id)))?;
    let text = reprisefs::apps::read_desktop_file(&path).ok()?;
    reprisefs::apps::parse_desktop(&text, &id)
}

/// How long a picture file gets to be read and decoded; past it the notification shows without it.
const PICTURE_WITHIN: Duration = Duration::from_secs(2);

/// [`notify::picture_file_sources`] on a thread of its own, so the bus thread waits at most
/// [`PICTURE_WITHIN`].
async fn picture_file(path: PathBuf) -> Vec<IconSource> {
    let (tx, rx) = async_channel::bounded(1);
    let spawned = std::thread::Builder::new()
        .name("notify-picture".into())
        .spawn(move || {
            let _ = tx.send_blocking(notify::picture_file_sources(&path));
        });
    if spawned.is_err() {
        return Vec::new();
    }
    let decoded = async { rx.recv().await.unwrap_or_default() };
    let late = async {
        async_io::Timer::after(PICTURE_WITHIN).await;
        Vec::new()
    };
    futures_lite::future::or(decoded, late).await
}

/// `(app_icon, image)`: the app icon name, else the desktop entry's icon;
/// the `image-data` hint, else `image-path`. Each is the first that resolves.
pub(super) async fn notification_icons(
    icons: &Icons,
    hints: &notify::Hints<'_>,
    app_icon: String,
    entry: Option<&reprisefs::apps::AppEntry>,
) -> (Vec<IconSource>, Vec<IconSource>) {
    let named = |name: Option<String>| {
        icon_ref(name)
            .map(|name| decode_named_at(&mut lock(icons), &name, &[], &NOTIFY_SIZES))
            .unwrap_or_default()
    };
    let mut image = (hints.image.as_ref())
        .map(notify::image_sources)
        .unwrap_or_default();
    if image.is_empty() {
        let path = hints
            .image_path
            .as_deref()
            .map(|p| match p.strip_prefix("file://") {
                Some(rest) => {
                    String::from_utf8_lossy(&reprisefs::trash::percent_decode(rest)).into_owned()
                }
                None => p.to_string(),
            });
        let file = path.clone().filter(|p| p.starts_with('/'));
        image = named(path);
        if image.is_empty()
            && let Some(file) = file
        {
            image = picture_file(PathBuf::from(file)).await;
        }
    }
    let app = [Some(app_icon), entry.and_then(|e| e.icon.clone())]
        .into_iter()
        .map(named)
        .find(|sources| !sources.is_empty())
        .unwrap_or_default();
    (app, image)
}
