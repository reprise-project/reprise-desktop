//! The media player the volume popup's strip shows, followed over MPRIS on the session bus: the newest Playing player, else the one that played last and is Paused. A thread follows every player's `PropertiesChanged` and the players coming and going, and sends the pick on change.

use futures_util::stream::{self, StreamExt};
use reprisekit::controls::volumepopup::{NowPlaying, Transport};
use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zbus::OwnedVariant;
use zbus::{Connection, MatchRule, Message, MessageStream};

const ROOT: &str = "org.mpris.MediaPlayer2";
const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    Playing,
    Paused,
    #[default]
    Stopped,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Entry {
    status: Status,
    title: String,
    artist: String,
    /// The file name from `xesam:url`, for a track with no title.
    file: String,
    identity: String,
    can_control: bool,
    can_previous: bool,
    can_next: bool,
    can_play: bool,
    can_pause: bool,
    /// When it last started playing, or was found: bigger is later.
    active: u64,
}

fn bool_of(v: &OwnedVariant) -> Option<bool> {
    bool::try_from(v.clone()).ok()
}

fn string_of(v: &OwnedVariant) -> Option<String> {
    String::try_from(v.clone()).ok()
}

/// `xesam:artist` is a list of names; some players send one string.
fn artists_of(v: &OwnedVariant) -> Option<String> {
    Vec::<String>::try_from(v.clone())
        .map(|a| a.join(", "))
        .ok()
        .or_else(|| string_of(v))
}

fn file_name(url: &str) -> String {
    let last = url.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let bytes = last.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Entry {
    fn apply(&mut self, props: &HashMap<String, OwnedVariant>) -> bool {
        let was = self.status;
        for (k, v) in props {
            match k.as_str() {
                "PlaybackStatus" => {
                    self.status = match string_of(v).as_deref() {
                        Some("Playing") => Status::Playing,
                        Some("Paused") => Status::Paused,
                        _ => Status::Stopped,
                    }
                }
                "Metadata" => {
                    let meta =
                        HashMap::<String, OwnedVariant>::try_from(v.clone()).unwrap_or_default();
                    let get = |k: &str| meta.get(k);
                    self.title = get("xesam:title").and_then(string_of).unwrap_or_default();
                    self.artist = get("xesam:artist").and_then(artists_of).unwrap_or_default();
                    self.file = get("xesam:url")
                        .and_then(string_of)
                        .map(|u| file_name(&u))
                        .unwrap_or_default();
                }
                "CanControl" => self.can_control = bool_of(v).unwrap_or(false),
                "CanGoPrevious" => self.can_previous = bool_of(v).unwrap_or(false),
                "CanGoNext" => self.can_next = bool_of(v).unwrap_or(false),
                "CanPlay" => self.can_play = bool_of(v).unwrap_or(false),
                "CanPause" => self.can_pause = bool_of(v).unwrap_or(false),
                _ => {}
            }
        }
        was != Status::Playing && self.status == Status::Playing
    }

    /// A player that cannot be controlled greys every button.
    fn now_playing(&self) -> NowPlaying {
        let title = [&self.title, &self.file, &self.identity]
            .into_iter()
            .find(|s| !s.trim().is_empty())
            .cloned()
            .unwrap_or_default();
        let c = self.can_control;
        NowPlaying {
            title,
            artist: self.artist.clone(),
            playing: self.status == Status::Playing,
            can_previous: c && self.can_previous,
            can_next: c && self.can_next,
            can_play: c && self.can_play,
            can_pause: c && self.can_pause,
        }
    }
}

#[derive(Debug, Default)]
pub struct Players {
    players: BTreeMap<String, Entry>,
    clock: u64,
}

impl Players {
    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    pub fn add(&mut self, name: &str, props: &HashMap<String, OwnedVariant>, identity: String) {
        let mut e = Entry {
            identity,
            // MPRIS: a player that does not say is controllable.
            can_control: true,
            ..Entry::default()
        };
        e.apply(props);
        e.active = self.tick();
        self.players.insert(name.to_string(), e);
    }

    pub fn changed(&mut self, name: &str, props: &HashMap<String, OwnedVariant>) {
        let now = self.clock + 1;
        if let Some(e) = self.players.get_mut(name)
            && e.apply(props)
        {
            e.active = now;
            self.clock = now;
        }
    }

    pub fn remove(&mut self, name: &str) {
        self.players.remove(name);
    }

    /// The player the strip shows and its bus name: the newest Playing
    /// one, else the most recently active Paused one.
    pub fn pick(&self) -> Option<(String, NowPlaying)> {
        let newest = |status| {
            self.players
                .iter()
                .filter(|(_, e)| e.status == status)
                .max_by_key(|(_, e)| e.active)
        };
        newest(Status::Playing)
            .or_else(|| newest(Status::Paused))
            .map(|(n, e)| (n.clone(), e.now_playing()))
    }
}

pub fn method(t: Transport) -> &'static str {
    match t {
        Transport::Previous => "Previous",
        Transport::PlayPause => "PlayPause",
        Transport::Next => "Next",
    }
}

pub type Pick = Option<(String, NowPlaying)>;

pub struct Watch {
    pub picks: Receiver<Pick>,
    conn: Arc<Mutex<Option<Connection>>>,
}

impl Watch {
    pub fn send(&self, bus: &str, t: Transport) {
        let Some(conn) = self.conn.lock().ok().and_then(|c| c.clone()) else {
            return;
        };
        let bus = bus.to_string();
        let spawned = std::thread::Builder::new()
            .name("reprise-sound-mpris-call".into())
            .spawn(move || {
                let call = conn.call_method(Some(bus.as_str()), PATH, Some(PLAYER), method(t), &());
                if let Err(e) = zbus::block_on(call) {
                    log::info!("{bus}: {} failed: {e}", method(t));
                }
            });
        if let Err(e) = spawned {
            log::warn!("no thread for the player call: {e}");
        }
    }
}

/// The thread's end: sends a pick and wakes the app only when it differs from the last.
struct Feed {
    tx: Sender<Pick>,
    waker: repriseapp::Waker,
    last: Option<Pick>,
}

impl Feed {
    fn send(&mut self, p: Pick) -> bool {
        if self.last.as_ref() == Some(&p) {
            return true;
        }
        if self.tx.send(p.clone()).is_err() {
            return false;
        }
        self.last = Some(p);
        self.waker.wake();
        true
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Run {
    Ended { delivered: bool },
    Closed,
}

async fn read(conn: &Connection, name: &str) -> Option<(HashMap<String, OwnedVariant>, String)> {
    let get_all = |iface: &'static str| async move {
        conn.call_method(Some(name), PATH, Some(PROPERTIES), "GetAll", &(iface,))
            .await
            .ok()?
            .body()
            .deserialize::<HashMap<String, OwnedVariant>>()
            .ok()
    };
    let props = get_all(PLAYER).await?;
    let identity = get_all(ROOT)
        .await
        .and_then(|root| root.get("Identity").and_then(string_of))
        .unwrap_or_default();
    Some((props, identity))
}

async fn owner(dbus: &zbus::fdo::DBusProxy<'_>, name: &str) -> Option<String> {
    let name = zbus::names::BusName::try_from(name).ok()?;
    dbus.get_name_owner(name).await.ok().map(|o| o.to_string())
}

enum Wake {
    Owner(String, String, String),
    Props(
        String,
        Option<(String, HashMap<String, OwnedVariant>, Vec<String>)>,
    ),
}

fn wake_of(m: &Message) -> Option<Wake> {
    let h = m.header();
    if h.member().is_some_and(|n| n.as_str() == "NameOwnerChanged") {
        let (name, old, new): (String, String, String) = m.body().deserialize().ok()?;
        return Some(Wake::Owner(name, old, new));
    }
    let sender = h.sender()?.to_string();
    Some(Wake::Props(sender, m.body().deserialize().ok()))
}

async fn run(feed: &mut Feed, shared: &Mutex<Option<Connection>>) -> Run {
    let conn = match Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            log::info!("no session bus ({e}); no player to show");
            return if feed.send(None) {
                Run::Ended { delivered: false }
            } else {
                Run::Closed
            };
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
        if let (Some(o), Some((props, identity))) =
            (owner(&dbus, &name).await, read(&conn, &name).await)
        {
            owners.entry(o).or_default().push(name.clone());
            players.add(&name, &props, identity);
        }
    }
    if !feed.send(players.pick()) {
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
                    && let Some((props, identity)) = read(&conn, &name).await
                {
                    owners.entry(new).or_default().push(name.clone());
                    players.add(&name, &props, identity);
                }
            }
            Wake::Props(sender, body) => {
                let names = owners.get(&sender).cloned().unwrap_or_default();
                for name in names {
                    match &body {
                        // The signal carries what moved; an invalidated key, or a body that did not parse, is read again.
                        Some((_, changed, invalidated)) if invalidated.is_empty() => {
                            players.changed(&name, changed);
                        }
                        _ => {
                            if let Some((props, _)) = read(&conn, &name).await {
                                players.changed(&name, &props);
                            }
                        }
                    }
                }
            }
        }
        if !feed.send(players.pick()) {
            return Run::Closed;
        }
    }
    if let Ok(mut c) = shared.lock() {
        *c = None;
    }
    Run::Ended { delivered: true }
}

const RETRY_MIN: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(60);

/// Follow the players on a thread; every new pick arrives on `picks` and wakes the app. While the bus is gone `None` arrives and the thread retries after a backoff.
pub fn watch(waker: repriseapp::Waker) -> Watch {
    let (tx, rx) = channel();
    let conn = Arc::new(Mutex::new(None));
    let shared = conn.clone();
    let spawned = std::thread::Builder::new()
        .name("reprise-sound-mpris".into())
        .spawn(move || {
            let mut feed = Feed {
                tx,
                waker,
                last: None,
            };
            let mut pause = RETRY_MIN;
            while let Run::Ended { delivered } = zbus::block_on(run(&mut feed, &shared)) {
                pause = if delivered {
                    RETRY_MIN
                } else {
                    (pause * 2).min(RETRY_MAX)
                };
                std::thread::sleep(pause);
            }
        });
    if let Err(e) = spawned {
        log::warn!("could not start the MPRIS thread: {e}");
    }
    Watch { picks: rx, conn }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::Variant;

    fn props(entries: Vec<(&str, Variant<'static>)>) -> HashMap<String, OwnedVariant> {
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.try_to_owned().unwrap()))
            .collect()
    }

    fn status(s: &str) -> HashMap<String, OwnedVariant> {
        props(vec![("PlaybackStatus", Variant::from(s.to_string()))])
    }

    fn picked(p: &Players) -> Option<String> {
        p.pick().map(|(n, _)| n)
    }

    #[test]
    fn the_newest_playing_player_wins() {
        let mut p = Players::default();
        p.add("org.mpris.MediaPlayer2.a", &status("Playing"), "A".into());
        p.add("org.mpris.MediaPlayer2.b", &status("Paused"), "B".into());
        assert_eq!(picked(&p).as_deref(), Some("org.mpris.MediaPlayer2.a"));
        p.changed("org.mpris.MediaPlayer2.b", &status("Playing"));
        assert_eq!(
            picked(&p).as_deref(),
            Some("org.mpris.MediaPlayer2.b"),
            "b started last"
        );
        // A metadata change on a playing player does not make it newer.
        p.changed("org.mpris.MediaPlayer2.a", &status("Playing"));
        assert_eq!(picked(&p).as_deref(), Some("org.mpris.MediaPlayer2.b"));
    }

    #[test]
    fn with_none_playing_the_last_active_paused_one_wins() {
        let mut p = Players::default();
        p.add("org.mpris.MediaPlayer2.a", &status("Paused"), "A".into());
        p.add("org.mpris.MediaPlayer2.b", &status("Playing"), "B".into());
        p.add("org.mpris.MediaPlayer2.c", &status("Stopped"), "C".into());
        p.changed("org.mpris.MediaPlayer2.a", &status("Playing"));
        p.changed("org.mpris.MediaPlayer2.a", &status("Paused"));
        p.changed("org.mpris.MediaPlayer2.b", &status("Paused"));
        assert_eq!(
            picked(&p).as_deref(),
            Some("org.mpris.MediaPlayer2.a"),
            "a played after b was found"
        );
        p.remove("org.mpris.MediaPlayer2.a");
        assert_eq!(picked(&p).as_deref(), Some("org.mpris.MediaPlayer2.b"));
        p.remove("org.mpris.MediaPlayer2.b");
        assert_eq!(picked(&p), None, "a stopped player is not shown");
    }

    #[test]
    fn metadata_and_capabilities_reach_the_strip() {
        let mut p = Players::default();
        let meta: HashMap<String, Variant<'static>> = HashMap::from([
            ("xesam:title".to_string(), Variant::from("Song")),
            (
                "xesam:artist".to_string(),
                Variant::from(vec!["One".to_string(), "Two".to_string()]),
            ),
        ]);
        p.add(
            "org.mpris.MediaPlayer2.a",
            &props(vec![
                ("PlaybackStatus", Variant::from("Playing")),
                ("Metadata", Variant::from(meta)),
                ("CanGoNext", Variant::from(true)),
                ("CanGoPrevious", Variant::from(false)),
                ("CanPlay", Variant::from(true)),
                ("CanPause", Variant::from(true)),
            ]),
            "A".into(),
        );
        let (_, np) = p.pick().unwrap();
        assert_eq!(
            np,
            NowPlaying {
                title: "Song".into(),
                artist: "One, Two".into(),
                playing: true,
                can_previous: false,
                can_next: true,
                can_play: true,
                can_pause: true,
            }
        );
        p.changed(
            "org.mpris.MediaPlayer2.a",
            &props(vec![("CanControl", Variant::from(false))]),
        );
        let (_, np) = p.pick().unwrap();
        assert!(!np.can_next && !np.can_pause, "uncontrollable: all greyed");
    }

    #[test]
    fn a_track_with_no_title_shows_its_file_then_the_player() {
        let mut p = Players::default();
        let meta: HashMap<String, Variant<'static>> = HashMap::from([(
            "xesam:url".to_string(),
            Variant::from("file:///home/u/Music/My%20Song.ogg"),
        )]);
        p.add(
            "org.mpris.MediaPlayer2.a",
            &props(vec![
                ("PlaybackStatus", Variant::from("Paused")),
                ("Metadata", Variant::from(meta)),
            ]),
            "mpv".into(),
        );
        assert_eq!(p.pick().unwrap().1.title, "My Song.ogg");
        p.changed(
            "org.mpris.MediaPlayer2.a",
            &props(vec![(
                "Metadata",
                Variant::from(HashMap::<String, Variant<'static>>::new()),
            )]),
        );
        assert_eq!(p.pick().unwrap().1.title, "mpv");
    }
}
