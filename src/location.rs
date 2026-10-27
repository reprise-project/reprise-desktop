//! Location: Reprise is the session's geoclue agent. It asks the user
//! before a program gets the location, and shows a location pin in the
//! tray while one reads it.
//!
//! A thread on the system bus serves `org.freedesktop.GeoClue2.Agent` at
//! `/org/freedesktop/GeoClue2/Agent` and registers it with geoclue's
//! manager (`AddAgent`) whenever geoclue starts; geoclue exits when idle
//! and is started again by the next program, so the thread watches its
//! name. geoclue takes the agent only if `reprise` is in its agent
//! whitelist, which the package's `/etc/geoclue/conf.d/90-reprise.conf`
//! adds; without an agent geoclue refuses every program.
//!
//! `MaxAccuracyLevel` is the switch: 0 while `[privacy] location` is off,
//! which makes geoclue refuse every program and stop the running ones.
//! For a sandboxed program geoclue calls `AuthorizeApp`, which lands here
//! as an [`Ask`]: a remembered answer (`repriseconf::location`) is given
//! at once, else the location prompt (`reprisekit::controls::
//! locationprompt`) asks, and its answer goes back to geoclue and, with
//! its check box ticked, into the file. geoclue lets programs outside a
//! sandbox through without asking the agent.
//!
//! Only geoclue's current owner is heard: an `AuthorizeApp` or a property
//! change from any other peer is ignored. One prompt shows at a time; a
//! second program asking meanwhile is refused.
//!
//! The pin shows while geoclue's `InUse` is true. Its tooltip names the
//! programs from the `DesktopId` and `Active` changes geoclue's clients
//! announce, when the bus delivers them; else it names no one.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::stream::{BoxStream, SelectAll, StreamExt};
use reprisekit::controls::locationprompt::{self, Answer};
use smithay::reexports::calloop::{
    LoopHandle,
    channel::{Event, Sender, channel},
};
use zbus::OwnedVariant;

use crate::compositor::State;

const GEOCLUE: &str = "org.freedesktop.GeoClue2";
const MANAGER_PATH: &str = "/org/freedesktop/GeoClue2/Manager";
const MANAGER: &str = "org.freedesktop.GeoClue2.Manager";
const CLIENT_PREFIX: &str = "/org/freedesktop/GeoClue2/Client/";
const AGENT_PATH: &str = "/org/freedesktop/GeoClue2/Agent";
/// The id geoclue checks against its agent whitelist.
const AGENT_ID: &str = "reprise";
/// geoclue's `GCLUE_ACCURACY_LEVEL_EXACT`: the most a program may get.
const EXACT: u32 = 8;
/// How long the thread waits for the bus before it tries again, and how
/// often the pin's state is read again while it shows.
const RETRY: Duration = Duration::from_secs(5);
/// How long `AuthorizeApp` waits for the prompt. geoclue calls with the
/// default D-Bus timeout (25 s); answering just before it lets geoclue hear
/// the refusal, and the prompt closes with it.
const ANSWER_WITHIN: Duration = Duration::from_secs(24);

pub struct Ask {
    pub desktop_id: String,
    pub reply: async_channel::Sender<bool>,
}

enum FromAgent {
    Ask(Ask),
    /// An `AuthorizeApp` for this desktop id gave up waiting.
    Expired(String),
    /// geoclue's `InUse`, and the desktop ids of the clients known to be
    /// active.
    Usage {
        in_use: bool,
        apps: Vec<String>,
    },
}

#[derive(Default)]
pub struct Location {
    in_use: bool,
    apps: Vec<String>,
    pending: Vec<(String, Vec<async_channel::Sender<bool>>)>,
    enabled: Option<async_channel::Sender<bool>>,
}

impl Location {
    /// Start the agent thread, reporting on `handle`'s loop; `enabled` is
    /// `[privacy] location` at startup.
    pub fn start(handle: &LoopHandle<'static, State>, enabled: bool) -> Location {
        let (tx, rx) = channel::<FromAgent>();
        let (en_tx, en_rx) = async_channel::unbounded::<bool>();
        let inserted = handle.insert_source(rx, |event, _, state| {
            let Event::Msg(msg) = event else {
                return;
            };
            match msg {
                FromAgent::Ask(ask) => asked(state, ask),
                FromAgent::Expired(id) => expired(state, &id),
                FromAgent::Usage { in_use, apps } => {
                    let l = &mut state.location;
                    if (in_use, &apps) != (l.in_use, &l.apps) {
                        log::info!("location in use: {in_use} ({apps:?})");
                        l.in_use = in_use;
                        l.apps = apps;
                        state.redraw_taskbars();
                    }
                }
            }
        });
        if let Err(e) = inserted {
            log::warn!("location: channel: {e}");
            return Location::default();
        }
        let spawned = std::thread::Builder::new()
            .name("geoclue-agent".into())
            .spawn(move || zbus::block_on(agent(tx, en_rx, enabled)));
        if let Err(e) = spawned {
            log::warn!("location: no agent thread: {e}");
        }
        Location {
            enabled: Some(en_tx),
            ..Location::default()
        }
    }

    pub fn shown(&self) -> bool {
        self.in_use
    }
}

/// `[privacy] location` changed: geoclue is told through the agent's
/// `MaxAccuracyLevel`.
pub fn config_changed(state: &mut State) {
    if let Some(tx) = &state.location.enabled {
        let _ = tx.try_send(state.config.location);
    }
}

/// The pin's tooltip: who reads the location, by the name in their
/// desktop entries.
pub fn tip(state: &State) -> String {
    let mut names: Vec<String> = state
        .location
        .apps
        .iter()
        .map(|id| app_name(state, id))
        .collect();
    names.sort();
    names.dedup();
    match names.as_slice() {
        [] => "A program is using your location".to_string(),
        [one] => format!("{one} is using your location"),
        [rest @ .., last] => format!("{} and {last} are using your location", rest.join(", ")),
    }
}

fn app_name(state: &State, desktop_id: &str) -> String {
    reprisefs::apps::entry_for_app_id(&state.app_index, desktop_id, |_| true)
        .map(|e| e.name.clone())
        .unwrap_or_else(|| desktop_id.to_string())
}

/// What answers `desktop_id` without asking: the location switched off
/// refuses, a remembered answer stands; None asks.
fn known_answer(enabled: bool, remembered: Option<bool>) -> Option<bool> {
    if !enabled {
        return Some(false);
    }
    remembered
}

/// Where a new ask about a program goes among the pending prompts.
#[derive(Debug, PartialEq)]
enum Slot {
    /// That program's prompt is open: wait on it.
    Join(usize),
    /// Nothing is asked yet: open a prompt.
    Open,
    /// Another program's prompt is open: refuse.
    Busy,
}

fn slot<T>(pending: &[(String, T)], desktop_id: &str) -> Slot {
    match pending.iter().position(|(id, _)| id == desktop_id) {
        Some(at) => Slot::Join(at),
        None if pending.is_empty() => Slot::Open,
        None => Slot::Busy,
    }
}

/// A program asked: answer from the switch or the remembered answers, or
/// open the prompt (a second ask for a program already prompted waits on
/// the same prompt).
fn asked(state: &mut State, ask: Ask) {
    let Ask { desktop_id, reply } = ask;
    let remembered = repriseconf::location::load().get(&desktop_id).copied();
    if let Some(allow) = known_answer(state.config.location, remembered) {
        log::info!("location for {desktop_id:?}: {allow} without asking");
        let _ = reply.try_send(allow);
        return;
    }
    // A prompt closed some other way left its callers; they are refused.
    let open: Vec<String> = prompts(state).into_iter().map(|(_, id)| id).collect();
    state.location.pending.retain(|(id, _)| open.contains(id));
    match slot(&state.location.pending, &desktop_id) {
        Slot::Join(at) => {
            state.location.pending[at].1.push(reply);
            return;
        }
        Slot::Busy => {
            log::info!("location for {desktop_id:?}: refused, another prompt is open");
            let _ = reply.try_send(false);
            return;
        }
        Slot::Open => {}
    }
    log::info!("location: asking about {desktop_id:?}");
    let program = app_name(state, &desktop_id);
    let icon = reprisefs::apps::entry_for_app_id(&state.app_index, &desktop_id, |_| true)
        .and_then(|e| e.icon.clone())
        .and_then(|name| state.icons.get(&name, 32))
        .or_else(|| state.icons.get("dialog-question", 32));
    let fields = locationprompt::Fields { program, icon };
    let t = &state.config.theme;
    let (px, u) = (t.fonts.size_px, t.text);
    let d = crate::dialog::Dialog::location_prompt(&fields, desktop_id.clone(), &state.text, px, u);
    state.location.pending.push((desktop_id, vec![reply]));
    state.open_dialog(d);
}

/// The open location prompts: their index among the dialogs and the
/// desktop id each asks about.
fn prompts(state: &State) -> Vec<(usize, String)> {
    state
        .dialogs
        .views
        .iter()
        .enumerate()
        .filter_map(|(i, v)| match &v.dialog.style {
            crate::dialog::Style::Form(f) => match &f.kind {
                crate::dialog::Kind::Location(id) => Some((i, id.clone())),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// The prompt for `desktop_id` was answered: close it, answer every call
/// waiting on it, and keep the answer if asked to.
pub fn answer(state: &mut State, desktop_id: &str, a: Answer) {
    if let Some((i, _)) = prompts(state).into_iter().find(|(_, id)| id == desktop_id) {
        state.close_dialog_at(i);
    }
    log::info!("location for {desktop_id:?}: {a:?}");
    if let Some(at) = state
        .location
        .pending
        .iter()
        .position(|(id, _)| id == desktop_id)
    {
        let (_, replies) = state.location.pending.remove(at);
        for r in replies {
            let _ = r.try_send(a.allow);
        }
    }
    if a.remember
        && let Err(e) = repriseconf::location::remember(desktop_id, a.allow)
    {
        log::warn!("location: could not remember {desktop_id:?}: {e}");
    }
}

/// Drop the callers that stopped waiting; true when none is left.
fn abandoned(replies: &mut Vec<async_channel::Sender<bool>>) -> bool {
    replies.retain(|r| !r.is_closed());
    replies.is_empty()
}

/// An `AuthorizeApp` about `desktop_id` gave up: once no caller waits on
/// its prompt any more, the prompt closes unanswered.
fn expired(state: &mut State, desktop_id: &str) {
    let Some(at) = state
        .location
        .pending
        .iter_mut()
        .position(|(id, replies)| id == desktop_id && abandoned(replies))
    else {
        return;
    };
    state.location.pending.remove(at);
    log::info!("location for {desktop_id:?}: no answer in time, refused");
    if let Some((i, _)) = prompts(state).into_iter().find(|(_, id)| id == desktop_id) {
        state.close_dialog_at(i);
    }
}

/// The prompt's answer, a refusal when the prompt went away unanswered, or
/// `None` when nothing came within `limit`.
async fn answer_within(answer: &async_channel::Receiver<bool>, limit: Duration) -> Option<bool> {
    let got = async { Some(answer.recv().await.unwrap_or(false)) };
    let late = async {
        async_io::Timer::after(limit).await;
        None
    };
    futures_lite::future::or(got, late).await
}

struct Agent {
    tx: Sender<FromAgent>,
    max: Arc<AtomicU32>,
    /// geoclue's unique name, None while it is not running.
    geoclue: Arc<Mutex<Option<String>>>,
}

#[zbus::interface(name = "org.freedesktop.GeoClue2.Agent")]
impl Agent {
    /// geoclue asks whether `desktop_id` may have the location; the
    /// accuracy it asked for is what it gets.
    async fn authorize_app(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        desktop_id: String,
        req_accuracy_level: u32,
    ) -> (bool, u32) {
        let sender = hdr.sender().map(|s| s.as_str());
        if !reprisebus::sent_by(sender, reprisebus::mutex::lock(&self.geoclue).as_deref()) {
            log::warn!("location: AuthorizeApp({desktop_id:?}) from {sender:?}, not geoclue");
            return (false, 0);
        }
        let (reply, answer) = async_channel::bounded(1);
        let ask = Ask {
            desktop_id: desktop_id.clone(),
            reply,
        };
        if self.tx.send(FromAgent::Ask(ask)).is_err() {
            return (false, 0);
        }
        let allow = match answer_within(&answer, ANSWER_WITHIN).await {
            Some(allow) => allow,
            None => {
                drop(answer);
                let _ = self.tx.send(FromAgent::Expired(desktop_id));
                false
            }
        };
        (allow, if allow { req_accuracy_level } else { 0 })
    }

    #[zbus(property)]
    fn max_accuracy_level(&self) -> u32 {
        self.max.load(Ordering::Relaxed)
    }
}

enum Wake {
    Geoclue(reprisebus::Signal),
    Enabled(bool),
    Tick,
}

/// The thread: connect, serve, register, then follow geoclue. A bus that
/// is not there yet is tried again every `RETRY`.
async fn agent(tx: Sender<FromAgent>, enabled: async_channel::Receiver<bool>, on: bool) {
    let max = Arc::new(AtomicU32::new(if on { EXACT } else { 0 }));
    loop {
        match follow(&tx, &enabled, &max).await {
            Ok(()) => return,
            Err(e) => {
                log::info!("location: geoclue agent stopped ({e}); trying again");
                async_io::Timer::after(RETRY).await;
            }
        }
    }
}

async fn follow(
    tx: &Sender<FromAgent>,
    enabled: &async_channel::Receiver<bool>,
    max: &Arc<AtomicU32>,
) -> zbus::Result<()> {
    let geoclue = Arc::new(Mutex::new(None));
    let conn = zbus::connection::Builder::system()
        .serve_at(
            AGENT_PATH,
            Agent {
                tx: tx.clone(),
                max: max.clone(),
                geoclue: geoclue.clone(),
            },
        )
        .build()
        .await?;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.DBus.Properties")
        .member("PropertiesChanged")
        .path_namespace("/org/freedesktop/GeoClue2")
        .build()?;
    let signals = reprisebus::signals(&conn, GEOCLUE, vec![rule.into()]).await;
    let mut wakes: SelectAll<BoxStream<'static, Wake>> = SelectAll::new();
    wakes.push(signals.map(Wake::Geoclue).boxed());
    wakes.push(enabled.clone().map(Wake::Enabled).boxed());
    wakes.push(async_io::Timer::interval(RETRY).map(|_| Wake::Tick).boxed());

    let mut clients = Clients::default();
    let mut present = follow_owner(&conn, &geoclue).await;
    if present {
        register(&conn).await;
        clients.in_use = in_use(&conn).await;
        clients.report(tx);
    }
    while let Some(w) = wakes.next().await {
        match w {
            Wake::Geoclue(reprisebus::Signal::OwnerChanged) => {
                let up = follow_owner(&conn, &geoclue).await;
                present = up;
                if up {
                    // The first client's changes may have come in already;
                    // they are kept.
                    register(&conn).await;
                    clients.in_use = in_use(&conn).await;
                } else {
                    clients = Clients::default();
                }
                clients.report(tx);
            }
            Wake::Enabled(on) => {
                max.store(if on { EXACT } else { 0 }, Ordering::Relaxed);
                let iface = conn
                    .object_server()
                    .interface::<_, Agent>(AGENT_PATH)
                    .await?;
                iface
                    .get()
                    .await
                    .max_accuracy_level_changed(iface.signal_emitter())
                    .await?;
                log::info!("location: turned {}", if on { "on" } else { "off" });
            }
            Wake::Geoclue(reprisebus::Signal::Message(msg)) => {
                if clients.changed(&msg) {
                    clients.report(tx);
                }
            }
            Wake::Tick if present && clients.in_use => {
                let now = in_use(&conn).await;
                if now != clients.in_use {
                    clients.in_use = now;
                    clients.active.clear();
                    clients.report(tx);
                }
            }
            Wake::Tick => {}
        }
    }
    Ok(())
}

/// Read geoclue's unique name into `owner`; true when it is running.
async fn follow_owner(conn: &zbus::Connection, owner: &Mutex<Option<String>>) -> bool {
    let now = reprisebus::name_owner(conn, GEOCLUE).await;
    let up = now.is_some();
    *reprisebus::mutex::lock(owner) = now;
    up
}

/// Register the agent with geoclue's manager. Refused when `reprise` is
/// not in geoclue's agent whitelist.
async fn register(conn: &zbus::Connection) {
    let r = conn
        .call_method(
            Some(GEOCLUE),
            MANAGER_PATH,
            Some(MANAGER),
            "AddAgent",
            &(AGENT_ID,),
        )
        .await;
    match r {
        Ok(_) => log::info!("location: registered as the geoclue agent"),
        Err(e) => log::warn!(
            "location: geoclue refused the agent ({e}); is {AGENT_ID:?} in the [agent] \
             whitelist (/etc/geoclue/conf.d/90-reprise.conf)?"
        ),
    }
}

/// geoclue's `InUse`, false when it cannot be read.
async fn in_use(conn: &zbus::Connection) -> bool {
    let reply = conn
        .call_method(
            Some(GEOCLUE),
            MANAGER_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(MANAGER, "InUse"),
        )
        .await;
    reply
        .ok()
        .and_then(|m| m.body().deserialize::<OwnedVariant>().ok())
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false)
}

/// What the property changes have told about geoclue: `InUse`, and each
/// client's desktop id and whether it is active, by object path.
#[derive(Default, Debug, PartialEq)]
struct Clients {
    in_use: bool,
    ids: HashMap<String, String>,
    active: HashMap<String, bool>,
}

impl Clients {
    /// Take in one `PropertiesChanged` from geoclue; true when the pin's
    /// state moved.
    fn changed(&mut self, msg: &zbus::Message) -> bool {
        let Some(path) = msg.header().path().map(|p| p.to_string()) else {
            return false;
        };
        let Ok((_, props, _)) = msg
            .body()
            .deserialize::<(String, HashMap<String, OwnedVariant>, Vec<String>)>()
        else {
            return false;
        };
        let props: Vec<(String, Prop)> = props
            .into_iter()
            .filter_map(|(k, v)| Some((k.clone(), Prop::from(&k, v)?)))
            .collect();
        self.apply(&path, &props)
    }

    fn apply(&mut self, path: &str, props: &[(String, Prop)]) -> bool {
        let before = (self.in_use, self.apps());
        for (_, p) in props {
            match p {
                Prop::InUse(b) if path == MANAGER_PATH => {
                    self.in_use = *b;
                    // geoclue announces no client going away; with none in
                    // use, none is active.
                    if !b {
                        self.active.clear();
                    }
                }
                Prop::DesktopId(id) if path.starts_with(CLIENT_PREFIX) => {
                    self.ids.insert(path.to_string(), id.clone());
                }
                Prop::Active(b) if path.starts_with(CLIENT_PREFIX) => {
                    self.active.insert(path.to_string(), *b);
                }
                _ => {}
            }
        }
        before != (self.in_use, self.apps())
    }

    fn apps(&self) -> Vec<String> {
        let mut apps: Vec<String> = self
            .active
            .iter()
            .filter(|(_, on)| **on)
            .filter_map(|(path, _)| self.ids.get(path).cloned())
            .collect();
        apps.sort();
        apps.dedup();
        apps
    }

    fn report(&self, tx: &Sender<FromAgent>) {
        let _ = tx.send(FromAgent::Usage {
            in_use: self.in_use,
            apps: if self.in_use { self.apps() } else { Vec::new() },
        });
    }
}

#[derive(Debug, PartialEq)]
enum Prop {
    InUse(bool),
    DesktopId(String),
    Active(bool),
}

impl Prop {
    fn from(name: &str, v: OwnedVariant) -> Option<Prop> {
        Some(match name {
            "InUse" => Prop::InUse(bool::try_from(v).ok()?),
            "Active" => Prop::Active(bool::try_from(v).ok()?),
            "DesktopId" => Prop::DesktopId(String::try_from(v).ok()?),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unanswered_prompt_gives_up_and_a_closed_one_refuses() {
        let short = Duration::from_millis(20);
        let (tx, rx) = async_channel::bounded(1);
        assert_eq!(zbus::block_on(answer_within(&rx, short)), None);
        tx.try_send(true).unwrap();
        assert_eq!(zbus::block_on(answer_within(&rx, short)), Some(true));
        drop(tx);
        assert_eq!(zbus::block_on(answer_within(&rx, short)), Some(false));
    }

    #[test]
    fn a_prompt_is_abandoned_only_once_every_caller_stopped_waiting() {
        let (first, first_rx) = async_channel::bounded::<bool>(1);
        let (second, second_rx) = async_channel::bounded::<bool>(1);
        let mut replies = vec![first, second];
        drop(first_rx);
        assert!(!abandoned(&mut replies), "the second caller still waits");
        assert_eq!(replies.len(), 1);
        drop(second_rx);
        assert!(abandoned(&mut replies));
    }

    #[test]
    fn the_switch_refuses_and_a_remembered_answer_stands() {
        assert_eq!(known_answer(false, Some(true)), Some(false));
        assert_eq!(known_answer(true, Some(true)), Some(true));
        assert_eq!(known_answer(true, Some(false)), Some(false));
        assert_eq!(known_answer(true, None), None, "nothing known asks");
    }

    #[test]
    fn one_prompt_shows_at_a_time() {
        let none: Vec<(String, ())> = Vec::new();
        assert_eq!(slot(&none, "org.gnome.Maps"), Slot::Open);
        let one = vec![("org.gnome.Maps".to_string(), ())];
        assert_eq!(slot(&one, "org.gnome.Maps"), Slot::Join(0));
        assert_eq!(slot(&one, "org.mozilla.firefox"), Slot::Busy);
    }

    #[test]
    fn active_clients_are_named_by_their_desktop_ids() {
        let mut c = Clients::default();
        let one = format!("{CLIENT_PREFIX}1");
        let two = format!("{CLIENT_PREFIX}2");
        let s = |k: &str, p: Prop| (k.to_string(), p);
        assert!(!c.apply(
            &one,
            &[s("DesktopId", Prop::DesktopId("org.gnome.Maps".into()))]
        ));
        assert!(!c.apply(
            &two,
            &[s(
                "DesktopId",
                Prop::DesktopId("org.example.Weather".into())
            )]
        ));
        assert!(c.apply(&one, &[s("Active", Prop::Active(true))]));
        assert!(c.apply(MANAGER_PATH, &[s("InUse", Prop::InUse(true))]));
        assert_eq!(c.apps(), ["org.gnome.Maps"]);
        assert!(c.apply(&two, &[s("Active", Prop::Active(true))]));
        assert_eq!(c.apps(), ["org.example.Weather", "org.gnome.Maps"]);
        assert!(c.apply(&one, &[s("Active", Prop::Active(false))]));
        assert_eq!(c.apps(), ["org.example.Weather"]);
        assert!(
            !c.apply(&one, &[s("InUse", Prop::InUse(false))]),
            "InUse counts only from the manager"
        );
        assert!(c.in_use);
        assert!(c.apply(MANAGER_PATH, &[s("InUse", Prop::InUse(false))]));
        assert!(
            c.apps().is_empty(),
            "nothing is active once nothing is in use"
        );
    }
}
