//! The MPRIS media players on the session bus, followed on a thread: each player's state, its
//! process and app, and what it says it can do. A thread follows every player's
//! `PropertiesChanged` and the players coming and going, and sends the whole set on each change.

use crate::bounded::{self, Dict, Value};
use crate::follow::{Feed, Run, follow};
use futures_util::stream::{self, StreamExt};
use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zbus::{Connection, MatchRule, Message, MessageStream};

const ROOT: &str = "org.mpris.MediaPlayer2";
pub const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// What [`Player::apply`] reads, asked one by one when `GetAll` is too large.
const WANTED: &[&str] = &[
    "PlaybackStatus",
    "Metadata",
    "CanControl",
    "CanGoPrevious",
    "CanGoNext",
    "CanPlay",
    "CanPause",
];
const ABOUT: &[&str] = &["Identity", "DesktopEntry"];
/// How long a player gets to answer; one that doesn't holds up the others no longer.
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    Playing,
    Paused,
    #[default]
    Stopped,
}

/// One player. Its text is as the player sent it: a host cuts what it shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Player {
    pub status: Status,
    pub title: String,
    pub artist: String,
    pub url: String,
    pub identity: String,
    /// The desktop entry the player names itself by (`DesktopEntry`).
    pub desktop_entry: String,
    /// The unique bus name owning the player's name when it was found: the one its calls go to,
    /// so a name that changes hands takes no call meant for its last owner.
    pub owner: String,
    /// The process owning the player's bus name.
    pub pid: Option<u32>,
    /// The app that process runs as: its Flatpak or snap id, or the desktop id of the Reprise app
    /// scope it runs in ([`crate::peer::app_identity`]).
    pub app: Option<String>,
    can_control: bool,
    can_previous: bool,
    can_next: bool,
    can_play: bool,
    can_pause: bool,
    /// When it last started playing, or was found: bigger is later.
    pub active: u64,
}

/// Who a player is beyond its properties: read once, when it comes on the bus.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct About {
    pub identity: String,
    pub desktop_entry: String,
    pub owner: String,
    pub pid: Option<u32>,
    pub app: Option<String>,
}

/// `xesam:artist` is a list of names; some players send one string.
fn artists_of(v: &Value) -> Option<String> {
    match v {
        Value::Strs(a) => Some(a.join(", ")),
        Value::Str(s) => Some(s.clone()),
        _ => None,
    }
}

impl Player {
    /// Take `props` in; true when it just started playing.
    fn apply(&mut self, props: &Dict) -> bool {
        let was = self.status;
        for (k, v) in props.iter() {
            let flag = || v.bool().unwrap_or(false);
            match k {
                "PlaybackStatus" => {
                    self.status = match v.str() {
                        Some("Playing") => Status::Playing,
                        Some("Paused") => Status::Paused,
                        _ => Status::Stopped,
                    }
                }
                "Metadata" => {
                    let meta = v.dict().cloned().unwrap_or_default();
                    let text = |k| meta.get(k).and_then(Value::str).unwrap_or_default();
                    self.title = text("xesam:title").to_string();
                    self.artist = meta
                        .get("xesam:artist")
                        .and_then(artists_of)
                        .unwrap_or_default();
                    self.url = text("xesam:url").to_string();
                }
                "CanControl" => self.can_control = flag(),
                "CanGoPrevious" => self.can_previous = flag(),
                "CanGoNext" => self.can_next = flag(),
                "CanPlay" => self.can_play = flag(),
                "CanPause" => self.can_pause = flag(),
                _ => {}
            }
        }
        was != Status::Playing && self.status == Status::Playing
    }

    /// A player that cannot be controlled takes no Previous, Next, Play or Pause.
    pub fn can_previous(&self) -> bool {
        self.can_control && self.can_previous
    }

    pub fn can_next(&self) -> bool {
        self.can_control && self.can_next
    }

    pub fn can_play(&self) -> bool {
        self.can_control && self.can_play
    }

    pub fn can_pause(&self) -> bool {
        self.can_control && self.can_pause
    }
}

/// Every player on the bus, by bus name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Players {
    players: BTreeMap<String, Player>,
    clock: u64,
}

impl Players {
    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    pub fn add(&mut self, name: &str, props: &Dict, about: About) {
        let mut p = Player {
            identity: about.identity,
            desktop_entry: about.desktop_entry,
            owner: about.owner,
            pid: about.pid,
            app: about.app,
            // MPRIS: a player that does not say is controllable.
            can_control: true,
            ..Player::default()
        };
        p.apply(props);
        p.active = self.tick();
        self.players.insert(name.to_string(), p);
    }

    pub fn changed(&mut self, name: &str, props: &Dict) {
        let now = self.clock + 1;
        if let Some(p) = self.players.get_mut(name)
            && p.apply(props)
        {
            p.active = now;
            self.clock = now;
        }
    }

    pub fn remove(&mut self, name: &str) {
        self.players.remove(name);
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Player)> {
        self.players.iter().map(|(n, p)| (n.as_str(), p))
    }

    /// The player the media keys and every single media control go to: the newest Playing one,
    /// else the most recently active Paused one, else the most recently active Stopped one.
    pub fn target(&self) -> Option<(&str, &Player)> {
        let newest = |status| {
            self.iter()
                .filter(|(_, p)| p.status == status)
                .max_by_key(|(_, p)| p.active)
        };
        newest(Status::Playing)
            .or_else(|| newest(Status::Paused))
            .or_else(|| newest(Status::Stopped))
    }

    /// The [`target`](Players::target) while it plays or is paused: the player a control showing
    /// a track shows.
    pub fn newest(&self) -> Option<(&str, &Player)> {
        self.target().filter(|(_, p)| p.status != Status::Stopped)
    }
}

/// The method of the player interface a transport button calls: `Previous`, `PlayPause`, `Next`.
pub type Method = &'static str;

pub struct Watch {
    /// The set each time it changes; the default (no players) while the bus is gone.
    pub players: Receiver<Players>,
    conn: Arc<Mutex<Option<Connection>>>,
}

impl Watch {
    /// Call `method` on the player `bus` from a thread of its own, so a player that never answers
    /// holds up nothing.
    pub fn send(&self, bus: &str, method: Method) {
        let Some(conn) = self.conn.lock().ok().and_then(|c| c.clone()) else {
            return;
        };
        let bus = bus.to_string();
        let spawned = std::thread::Builder::new()
            .name("mpris-call".into())
            .spawn(move || {
                let call = conn.call_method(Some(bus.as_str()), PATH, Some(PLAYER), method, &());
                if let Err(e) = zbus::block_on(call) {
                    log::info!("{bus}: {method} failed: {e}");
                }
            });
        if let Err(e) = spawned {
            log::warn!("no thread for the player call: {e}");
        }
    }
}

async fn read(conn: &Connection, name: &str) -> Option<(Dict, String, String)> {
    let player = bounded::proxy(conn, name, PATH, PLAYER).await.ok()?;
    let props = bounded::properties_of(&player, WANTED).await.ok()?;
    let about = match bounded::proxy(conn, name, PATH, ROOT).await {
        Ok(root) => bounded::properties_of(&root, ABOUT).await.ok(),
        Err(_) => None,
    }
    .unwrap_or_default();
    let text = |k| about.get(k).and_then(Value::str).unwrap_or_default();
    let entry = text("DesktopEntry").to_string();
    Some((props, text("Identity").to_string(), entry))
}

/// A player and who it is, or None when it does not answer.
async fn read_about(
    conn: &Connection,
    dbus: &zbus::fdo::DBusProxy<'_>,
    name: &str,
    owner: &str,
) -> Option<(Dict, About)> {
    let (props, identity, desktop_entry) = read(conn, name).await?;
    let pid = match zbus::names::BusName::try_from(owner) {
        Ok(o) => dbus.get_connection_unix_process_id(o).await.ok(),
        Err(_) => None,
    };
    let app = match crate::peer::app_identity(conn, owner).await {
        crate::peer::AppIdentity::Sandboxed { app_id } => Some(app_id),
        crate::peer::AppIdentity::Host { desktop_id } => desktop_id,
        crate::peer::AppIdentity::Unknown => None,
    };
    let about = About {
        identity,
        desktop_entry,
        owner: owner.to_string(),
        pid,
        app,
    };
    Some((props, about))
}

async fn owner(dbus: &zbus::fdo::DBusProxy<'_>, name: &str) -> Option<String> {
    let name = zbus::names::BusName::try_from(name).ok()?;
    dbus.get_name_owner(name).await.ok().map(|o| o.to_string())
}

enum Wake {
    Owner(String, String, String),
    /// A `PropertiesChanged` and its sender, decoded only once the sender is known to own a player.
    Props(String, Message),
}

fn wake_of(m: &Message) -> Option<Wake> {
    let h = m.header();
    if h.member().is_some_and(|n| n.as_str() == "NameOwnerChanged") {
        let (name, old, new): (String, String, String) = m.body().deserialize().ok()?;
        return Some(Wake::Owner(name, old, new));
    }
    let sender = h.sender()?.to_string();
    Some(Wake::Props(sender, m.clone()))
}

/// What a `PropertiesChanged` says moved; None when a key was invalidated or the body is over
/// [`bounded::MAX_BODY`] or of another type, so the player is read again.
fn changes_of(m: &Message) -> Option<Dict> {
    let body = bounded::within(m.body(), bounded::MAX_BODY).ok()?;
    let (_, changed, invalidated) = body.deserialize::<(&str, Dict, Vec<&str>)>().ok()?;
    invalidated.is_empty().then_some(changed)
}

fn timed(b: zbus::connection::Builder<'_>) -> zbus::connection::Builder<'_> {
    b.method_timeout(REPLY_TIMEOUT)
}

async fn run(feed: &mut Feed<Players>, shared: &Mutex<Option<Connection>>) -> Run {
    let conn = timed(zbus::connection::Builder::session()).build();
    let conn = match conn.await {
        Ok(c) => c,
        Err(e) => {
            return feed.unavailable_because(format_args!("no session bus ({e}); no players"));
        }
    };
    let rules = [
        MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")
            .interface("org.freedesktop.DBus")
            .member("NameOwnerChanged")
            .arg0ns(ROOT)
            .build(),
        MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .path(PATH)
            .interface(PROPERTIES)
            .member("PropertiesChanged")
            .arg(0, PLAYER)
            .build(),
    ];
    let mut streams = Vec::new();
    for rule in rules {
        let s = match rule {
            Ok(r) => MessageStream::for_match_rule(r, &conn, None).await,
            Err(e) => Err(e),
        };
        match s {
            Ok(s) => streams.push(s),
            Err(e) => {
                log::warn!("cannot follow the players: {e}");
                return Run::Ended { delivered: false };
            }
        }
    }
    let mut wakes = stream::select_all(streams)
        .filter_map(|m| async move { wake_of(&m.ok()?) })
        .boxed();
    let Ok(dbus) = zbus::fdo::DBusProxy::new(&conn).await else {
        return Run::Ended { delivered: false };
    };
    if let Ok(mut c) = shared.lock() {
        *c = Some(conn.clone());
    }
    let mut owners: HashMap<String, Vec<String>> = HashMap::new();
    let mut players = Players::default();
    let names = dbus.list_names().await.unwrap_or_default();
    for name in names.iter().map(|n| n.to_string()) {
        if !name.starts_with(PREFIX) {
            continue;
        }
        let Some(o) = owner(&dbus, &name).await else {
            continue;
        };
        if let Some((props, about)) = read_about(&conn, &dbus, &name, &o).await {
            owners.entry(o).or_default().push(name.clone());
            players.add(&name, &props, about);
        }
    }
    if !feed.send(players.clone()) {
        return Run::Closed;
    }
    while let Some(wake) = wakes.next().await {
        match wake {
            Wake::Owner(name, _, _) if !name.starts_with(PREFIX) => continue,
            Wake::Owner(name, old, new) => {
                if !old.is_empty() {
                    players.remove(&name);
                    if let Some(v) = owners.get_mut(&old) {
                        v.retain(|n| *n != name);
                    }
                }
                if !new.is_empty()
                    && let Some((props, about)) = read_about(&conn, &dbus, &name, &new).await
                {
                    owners.entry(new).or_default().push(name.clone());
                    players.add(&name, &props, about);
                }
            }
            Wake::Props(sender, msg) => {
                let names = owners.get(&sender).cloned().unwrap_or_default();
                let changes = if names.is_empty() {
                    None
                } else {
                    changes_of(&msg)
                };
                for name in names {
                    match &changes {
                        Some(changed) => players.changed(&name, changed),
                        None => {
                            if let Some((props, _, _)) = read(&conn, &name).await {
                                players.changed(&name, &props);
                            }
                        }
                    }
                }
            }
        }
        if !feed.send(players.clone()) {
            return Run::Closed;
        }
    }
    if let Ok(mut c) = shared.lock() {
        *c = None;
    }
    Run::Ended { delivered: true }
}

/// Follow the players on a thread called `thread`; every new set arrives on `players` and calls
/// `waker`. While the bus is gone an empty set arrives and the thread retries after a backoff.
pub fn watch(thread: &str, waker: impl Fn() + Send + 'static) -> Watch {
    let (tx, rx) = channel();
    let conn = Arc::new(Mutex::new(None));
    let shared = conn.clone();
    follow(thread, tx, waker, move |feed| {
        zbus::block_on(run(feed, &shared))
    });
    Watch { players: rx, conn }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::Variant;

    fn props(entries: Vec<(&str, Variant<'static>)>) -> Dict {
        let map: HashMap<&str, Variant> = entries.into_iter().collect();
        let msg = Message::signal(PATH, PROPERTIES, "PropertiesChanged")
            .build(&map)
            .unwrap();
        bounded::decode(&msg).unwrap()
    }

    fn status(s: &str) -> Dict {
        props(vec![("PlaybackStatus", Variant::from(s.to_string()))])
    }

    fn newest(p: &Players) -> Option<String> {
        p.newest().map(|(n, _)| n.to_string())
    }

    #[test]
    fn the_newest_playing_player_wins() {
        let mut p = Players::default();
        p.add(
            "org.mpris.MediaPlayer2.a",
            &status("Playing"),
            About::default(),
        );
        p.add(
            "org.mpris.MediaPlayer2.b",
            &status("Paused"),
            About::default(),
        );
        assert_eq!(newest(&p).as_deref(), Some("org.mpris.MediaPlayer2.a"));
        p.changed("org.mpris.MediaPlayer2.b", &status("Playing"));
        assert_eq!(
            newest(&p).as_deref(),
            Some("org.mpris.MediaPlayer2.b"),
            "b started last"
        );
        // A metadata change on a playing player does not make it newer.
        p.changed("org.mpris.MediaPlayer2.a", &status("Playing"));
        assert_eq!(newest(&p).as_deref(), Some("org.mpris.MediaPlayer2.b"));
    }

    #[test]
    fn with_none_playing_the_last_active_paused_one_wins() {
        let mut p = Players::default();
        p.add(
            "org.mpris.MediaPlayer2.a",
            &status("Paused"),
            About::default(),
        );
        p.add(
            "org.mpris.MediaPlayer2.b",
            &status("Playing"),
            About::default(),
        );
        p.add(
            "org.mpris.MediaPlayer2.c",
            &status("Stopped"),
            About::default(),
        );
        p.changed("org.mpris.MediaPlayer2.a", &status("Playing"));
        p.changed("org.mpris.MediaPlayer2.a", &status("Paused"));
        p.changed("org.mpris.MediaPlayer2.b", &status("Paused"));
        assert_eq!(
            newest(&p).as_deref(),
            Some("org.mpris.MediaPlayer2.a"),
            "a played after b was found"
        );
        p.remove("org.mpris.MediaPlayer2.a");
        assert_eq!(newest(&p).as_deref(), Some("org.mpris.MediaPlayer2.b"));
        p.remove("org.mpris.MediaPlayer2.b");
        assert_eq!(newest(&p), None, "a stopped player is not shown");
        assert_eq!(
            p.target().map(|(n, _)| n),
            Some("org.mpris.MediaPlayer2.c"),
            "the keys still reach it"
        );
    }

    #[test]
    fn a_player_that_cannot_be_controlled_can_do_nothing() {
        let mut p = Players::default();
        let can = vec![
            ("CanGoNext", Variant::from(true)),
            ("CanGoPrevious", Variant::from(false)),
            ("CanPlay", Variant::from(true)),
            ("CanPause", Variant::from(true)),
        ];
        p.add("org.mpris.MediaPlayer2.a", &props(can), About::default());
        let (_, a) = p.iter().next().unwrap();
        assert!(a.can_next() && a.can_play() && a.can_pause());
        assert!(!a.can_previous());
        p.changed(
            "org.mpris.MediaPlayer2.a",
            &props(vec![("CanControl", Variant::from(false))]),
        );
        let (_, a) = p.iter().next().unwrap();
        assert!(!a.can_next() && !a.can_play() && !a.can_pause());
    }

    #[test]
    fn a_player_that_never_answers_is_passed_over() {
        let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
        let (done, read_done) = channel();
        std::thread::spawn(move || {
            zbus::block_on(async {
                let (server, client) = futures_util::join!(
                    zbus::connection::Builder::unix_stream(a)
                        .server(zbus::Guid::generate())
                        .p2p()
                        .build(),
                    timed(zbus::connection::Builder::unix_stream(b).p2p()).build(),
                );
                let (_server, client) = (server.unwrap(), client.unwrap());
                let _ = done.send(read(&client, "org.mpris.MediaPlayer2.hung").await);
            })
        });
        let read = read_done.recv_timeout(Duration::from_secs(5));
        assert_eq!(read.expect("read gave up"), None);
    }
}
