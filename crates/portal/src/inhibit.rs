//! `org.freedesktop.impl.portal.Inhibit`: an app keeping the screen awake or the machine from
//! sleeping until it closes the call's `Request`.
//!
//! Idle shares one `org.freedesktop.ScreenSaver` cookie, taken for the first inhibition and given
//! back after the last; Suspend holds a logind `sleep` block descriptor; Logout goes into the list
//! `org.reprise.SessionEnd` gives the compositor. A call that cannot hold what it asked for fails
//! and holds nothing.

use reprisebus::desklock::RETRY;
use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use zbus::object_server::SignalEmitter;
use zbus::{ObjectPath, OwnedObjectPath, Variant};

pub const FLAG_LOGOUT: u32 = 1;
pub const FLAG_SWITCH: u32 = 2;
pub const FLAG_SUSPEND: u32 = 4;
pub const FLAG_IDLE: u32 = 8;

const SCREENSAVER: &str = "org.freedesktop.ScreenSaver";
const SCREENSAVER_PATH: &str = "/org/freedesktop/ScreenSaver";
const LOGIND: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const LOGIND_MANAGER: &str = "org.freedesktop.login1.Manager";
const LOGIND_SESSION: &str = "org.freedesktop.login1.Session";
const NO_SESSION: &str = "org.freedesktop.login1.NoSessionForPID";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const SESSION_RUNNING: u32 = 1;
const SESSION_QUERY_END: u32 = 2;
/// So the frontend has the monitor session in hand before its first `StateChanged` comes.
const FIRST_STATE_DELAY: std::time::Duration = std::time::Duration::from_millis(100);
const REASON_CHARS: usize = 63;

#[derive(Default)]
struct Held {
    owner: String,
    idle: bool,
    sleep: Option<OwnedFd>,
    logout: Option<(String, String)>,
}

/// A message to the idle task, which owns the cookie.
pub enum IdleMsg {
    /// One more idle inhibition, for `app` because of `reason`; the reply says whether idle is
    /// held.
    Take {
        app: String,
        reason: String,
        reply: async_channel::Sender<bool>,
    },
    Release,
}

/// The open inhibitions by request handle, the monitor sessions by path with their frontend, the
/// way to the idle task, what the desktop's lock reads from and whether the session is ending.
pub struct Inhibits {
    held: Mutex<HashMap<String, Held>>,
    monitors: Mutex<HashMap<String, String>>,
    idle: async_channel::Sender<IdleMsg>,
    lock: Mutex<LockSources>,
    ending: AtomicBool,
}

/// The compositor's lock once its owner is verified, else logind's `LockedHint` for the
/// compositor's session; locked while neither is known.
#[derive(Default)]
struct LockSources {
    desktop: Option<bool>,
    hint: Option<bool>,
}

impl LockSources {
    fn locked(&self) -> bool {
        self.desktop.or(self.hint).unwrap_or(true)
    }
}

pub type Shared = Arc<Inhibits>;

/// The shared state, and the receiving end `run_idle` takes.
pub fn shared() -> (Shared, async_channel::Receiver<IdleMsg>) {
    let (tx, rx) = async_channel::unbounded();
    let shared = Inhibits {
        held: Mutex::default(),
        monitors: Mutex::default(),
        idle: tx,
        lock: Mutex::default(),
        ending: AtomicBool::new(false),
    };
    (Arc::new(shared), rx)
}

impl Inhibits {
    async fn take_idle(&self, app: &str, reason: &str) -> bool {
        let (reply, answer) = async_channel::bounded(1);
        let msg = IdleMsg::Take {
            app: app.to_string(),
            reason: reason.to_string(),
            reply,
        };
        self.idle.send(msg).await.is_ok() && answer.recv().await.unwrap_or(false)
    }

    fn release(&self, held: Held) {
        if held.idle {
            let _ = self.idle.try_send(IdleMsg::Release);
        }
        drop(held.sleep);
    }

    fn logout_holders(&self) -> Vec<(String, String)> {
        let held = self.held.lock().unwrap();
        let mut v: Vec<(&String, &(String, String))> = held
            .iter()
            .filter_map(|(k, h)| Some((k, h.logout.as_ref()?)))
            .collect();
        v.sort_by(|a, b| a.0.cmp(b.0));
        v.into_iter().map(|(_, l)| l.clone()).collect()
    }

    fn monitor_state(&self) -> HashMap<&'static str, Variant<'static>> {
        let locked = self.lock.lock().unwrap().locked();
        state(locked, self.ending.load(Ordering::Relaxed))
    }

    /// Applies `change` to the lock's sources and tells every monitor session when the lock moves.
    async fn set_lock(&self, conn: &zbus::Connection, change: impl FnOnce(&mut LockSources)) {
        let moved = {
            let mut sources = self.lock.lock().unwrap();
            let before = sources.locked();
            change(&mut sources);
            let after = sources.locked();
            (after != before).then_some(after)
        };
        if let Some(locked) = moved {
            log::info!("Inhibit monitor: screensaver-active {locked}");
            emit_all(conn, self).await;
        }
    }
}

#[derive(Debug, PartialEq)]
enum Step {
    Nothing,
    Take,
    Release(u32),
}

/// The pure part of the idle task: how many inhibitions share the cookie and the cookie itself.
/// Messages are handled one at a time, each to its end, so a release never overtakes its take.
#[derive(Default)]
struct IdleCount {
    holders: usize,
    cookie: Option<u32>,
}

impl IdleCount {
    fn up(&mut self) -> Step {
        self.holders += 1;
        if self.cookie.is_none() {
            Step::Take
        } else {
            Step::Nothing
        }
    }

    fn taken(&mut self, cookie: Option<u32>) -> bool {
        match cookie {
            Some(c) => self.cookie = Some(c),
            None => self.holders -= 1,
        }
        cookie.is_some()
    }

    fn down(&mut self) -> Step {
        self.holders = self.holders.saturating_sub(1);
        match self.cookie {
            Some(c) if self.holders == 0 => {
                self.cookie = None;
                Step::Release(c)
            }
            _ => Step::Nothing,
        }
    }
}

/// Own the idle cookie: take it for the first holder, give it back after the last.
pub async fn run_idle(conn: zbus::Connection, rx: async_channel::Receiver<IdleMsg>) {
    let mut count = IdleCount::default();
    while let Ok(msg) = rx.recv().await {
        match msg {
            IdleMsg::Take { app, reason, reply } => {
                let held = match count.up() {
                    Step::Take => {
                        let cookie = screensaver_inhibit(&conn, &app, &reason)
                            .await
                            .map_err(|e| log::warn!("Inhibit idle: {e}"))
                            .ok();
                        count.taken(cookie)
                    }
                    _ => true,
                };
                let _ = reply.send(held).await;
            }
            IdleMsg::Release => {
                if let Step::Release(cookie) = count.down() {
                    screensaver_uninhibit(&conn, cookie).await;
                }
            }
        }
    }
}

pub struct Inhibit {
    pub shared: Shared,
    pub frontend: crate::Frontend,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Inhibit")]
impl Inhibit {
    #[allow(clippy::too_many_arguments)]
    async fn inhibit(
        &self,
        handle: ObjectPath<'_>,
        app_id: String,
        _window: String,
        flags: u32,
        options: HashMap<String, Variant<'_>>,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<()> {
        self.frontend.check(&hdr).await?;
        crate::check_handle(&handle, crate::REQUEST_PREFIX)?;
        let reason = match options.get("reason") {
            Some(Variant::Str(s)) => clip(s.as_str()),
            _ => String::new(),
        };
        let who = if app_id.is_empty() {
            "unknown"
        } else {
            &app_id
        };
        log::info!(
            "Inhibit {} for {who:?}: flags {flags} {reason:?}",
            handle.as_str()
        );
        if flags & FLAG_SWITCH != 0 {
            log::info!("Inhibit: User Switch is not held");
        }
        let mut held = Held {
            owner: crate::sender(&hdr),
            logout: (flags & FLAG_LOGOUT != 0).then(|| (app_id.clone(), reason.clone())),
            ..Held::default()
        };
        if flags & FLAG_SUSPEND != 0 {
            let fd = sleep_inhibit(who, &reason).await.map_err(|e| {
                log::warn!("Inhibit suspend: {e}");
                zbus::fdo::Error::Failed(format!("{LOGIND}: {e}"))
            })?;
            held.sleep = Some(fd);
        }
        if flags & FLAG_IDLE != 0 {
            if !self.shared.take_idle(who, &reason).await {
                return Err(zbus::fdo::Error::Failed(format!(
                    "{SCREENSAVER} refused the inhibition"
                )));
            }
            held.idle = true;
        }
        let key = handle.to_string();
        let old = self.shared.held.lock().unwrap().insert(key.clone(), held);
        if let Some(old) = old {
            self.shared.release(old);
        }
        let request = InhibitRequest {
            path: key,
            shared: self.shared.clone(),
        };
        if let Err(e) = server.at(handle.clone(), request).await {
            let held = self.shared.held.lock().unwrap().remove(handle.as_str());
            if let Some(h) = held {
                self.shared.release(h);
            }
            return Err(e.into());
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_monitor(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        _window: String,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<u32> {
        self.frontend.check(&hdr).await?;
        crate::check_session(&session_handle)?;
        log::info!("Inhibit monitor {} for {app_id:?}", session_handle.as_str());
        self.shared
            .monitors
            .lock()
            .unwrap()
            .insert(session_handle.to_string(), crate::sender(&hdr));
        let monitor = Monitor {
            path: session_handle.clone().into(),
            shared: self.shared.clone(),
        };
        server.at(session_handle.clone(), monitor).await?;
        let (conn2, shared) = (conn.clone(), self.shared.clone());
        let path: OwnedObjectPath = session_handle.into();
        zruntime::spawn(async move {
            async_io::Timer::after(FIRST_STATE_DELAY).await;
            // Unless the frontend closed it in the meantime.
            if shared.monitors.lock().unwrap().contains_key(path.as_str()) {
                emit_state(&conn2, &path, shared.monitor_state()).await;
            }
        })
        .detach();
        Ok(crate::RESPONSE_OK)
    }

    // Nothing to answer: the session end waits on the inhibitors themselves.
    fn query_end_response(&self, _session_handle: ObjectPath<'_>) {}

    #[zbus(signal)]
    async fn state_changed(
        emitter: &SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        state: HashMap<&str, Variant<'_>>,
    ) -> zbus::Result<()>;
}

fn state(locked: bool, ending: bool) -> HashMap<&'static str, Variant<'static>> {
    let session = if ending {
        SESSION_QUERY_END
    } else {
        SESSION_RUNNING
    };
    HashMap::from([
        ("screensaver-active", Variant::Bool(locked)),
        ("session-state", Variant::U32(session)),
    ])
}

async fn emit_state(
    conn: &zbus::Connection,
    session: &ObjectPath<'_>,
    state: HashMap<&'static str, Variant<'static>>,
) {
    let Ok(emitter) = SignalEmitter::new(conn, crate::PATH) else {
        return;
    };
    if let Err(e) = Inhibit::state_changed(&emitter, session.clone(), state).await {
        log::warn!("Inhibit monitor {}: {e}", session.as_str());
    }
}

async fn emit_all(conn: &zbus::Connection, shared: &Inhibits) {
    let paths: Vec<String> = shared.monitors.lock().unwrap().keys().cloned().collect();
    for p in paths {
        if let Ok(p) = ObjectPath::try_from(p.as_str()) {
            emit_state(conn, &p, shared.monitor_state()).await;
        }
    }
}

/// What the compositor asks while it ends the session: Reprise's own interface on the portal's
/// object.
pub struct SessionEnd {
    pub shared: Shared,
}

#[zbus::interface(name = "org.reprise.SessionEnd")]
impl SessionEnd {
    /// The logout inhibitors held now, as (app id, reason). The monitors move to query-end when
    /// `ending` starts and back to running when it is called off.
    async fn query(
        &self,
        ending: bool,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Vec<(String, String)> {
        if self.shared.ending.swap(ending, Ordering::Relaxed) != ending {
            log::info!("Inhibit monitor: session ending {ending}");
            emit_all(conn, &self.shared).await;
        }
        self.shared.logout_holders()
    }
}

/// Follows the desktop's lock for every monitor session: the compositor `pid`'s, with its logind
/// session's `LockedHint` standing in while that can't be read.
pub async fn watch_lock(conn: zbus::Connection, shared: Shared, pid: u32) {
    let hint = async {
        let system = until_answered("no system bus", zbus::Connection::system).await;
        follow_hint(&conn, &system, &shared, pid).await
    };
    futures_lite::future::zip(follow_desktop(&conn, &shared, pid), hint).await;
}

/// `ask` again every [`RETRY`] until it succeeds; the first failure is logged as `what`.
async fn until_answered<T, F: Future<Output = zbus::Result<T>>>(
    what: &str,
    mut ask: impl FnMut() -> F,
) -> T {
    let mut logged = false;
    loop {
        match ask().await {
            Ok(answer) => return answer,
            Err(e) if !logged => {
                log::warn!("Inhibit monitor: {what} ({e}); asking again until it answers");
                logged = true;
            }
            Err(_) => {}
        }
        async_io::Timer::after(RETRY).await;
    }
}

async fn follow_desktop(conn: &zbus::Connection, shared: &Shared, pid: u32) {
    use reprisebus::desklock::Lock;
    let report = |lock: Lock| async move {
        let desktop = match lock {
            Lock::Locked => Some(true),
            Lock::Unlocked => Some(false),
            Lock::Unknown(why) => {
                log::warn!("Inhibit monitor: the desktop's lock is unknown: {why}");
                None
            }
        };
        shared.set_lock(conn, |s| s.desktop = desktop).await;
    };
    let ended = reprisebus::desklock::follow(conn, || Ok(pid), &report).await;
    report(Lock::Unknown(ended)).await;
}

/// The `LockedHint` a `PropertiesChanged` from a logind session carries: Some(value) when it
/// changed, Some(None) when it must be read again, None when the signal is about other properties.
fn locked_hint_change(
    iface: &str,
    changed: &HashMap<String, zbus::OwnedVariant>,
    invalidated: &[String],
) -> Option<Option<bool>> {
    if iface != LOGIND_SESSION {
        return None;
    }
    if let Some(v) = changed.get("LockedHint") {
        return Some(bool::try_from(v.clone()).ok());
    }
    invalidated
        .iter()
        .any(|p| p == "LockedHint")
        .then_some(None)
}

/// The session's `LockedHint`; None when logind cannot say.
async fn read_locked(system: &zbus::Connection, session: &ObjectPath<'_>) -> Option<bool> {
    let reply = system
        .call_method(
            Some(LOGIND),
            session,
            Some(PROPERTIES),
            "Get",
            &(LOGIND_SESSION, "LockedHint"),
        )
        .await
        .ok()?;
    let v: zbus::OwnedVariant = reply.body().deserialize().ok()?;
    bool::try_from(v).ok()
}

/// Follows `LockedHint` on `system` for the logind session of the compositor `pid`.
async fn follow_hint(
    conn: &zbus::Connection,
    system: &zbus::Connection,
    shared: &Shared,
    pid: u32,
) {
    use futures_lite::StreamExt;
    let session = until_answered("no logind session for the compositor", || async {
        let reply = system
            .call_method(
                Some(LOGIND),
                LOGIND_PATH,
                Some(LOGIND_MANAGER),
                "GetSessionByPID",
                &(pid,),
            )
            .await;
        match reply {
            Ok(reply) => reply.body().deserialize::<OwnedObjectPath>().map(Some),
            Err(e) if reprisebus::error::name(&e).is_some_and(|n| n == NO_SESSION) => Ok(None),
            Err(e) => Err(e),
        }
    })
    .await;
    let Some(session) = session else {
        log::warn!(
            "Inhibit monitor: the compositor is in no logind session; no LockedHint stands in"
        );
        return;
    };
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(PROPERTIES)
        .member("PropertiesChanged")
        .path(session.clone())
        .add_arg(LOGIND_SESSION)
        .build()
        .map(Into::into);
    let rules = match rule {
        Ok(r) => vec![r],
        Err(e) => {
            log::warn!("Inhibit monitor: {e}");
            return;
        }
    };
    let mut signals = reprisebus::signals(system, LOGIND, rules).await;
    let set = |hint: Option<bool>| shared.set_lock(conn, move |s| s.hint = hint);
    set(read_locked(system, &session).await).await;
    while let Some(sig) = signals.next().await {
        let hint = match sig {
            reprisebus::Signal::OwnerChanged => None,
            reprisebus::Signal::Message(m) => {
                type Changed = (String, HashMap<String, zbus::OwnedVariant>, Vec<String>);
                let Ok((iface, changed, invalidated)) = m.body().deserialize::<Changed>() else {
                    continue;
                };
                match locked_hint_change(&iface, &changed, &invalidated) {
                    Some(hint) => hint,
                    None => continue,
                }
            }
        };
        let hint = match hint {
            Some(hint) => Some(hint),
            None => read_locked(system, &session).await,
        };
        set(hint).await;
    }
}

fn clip(reason: &str) -> String {
    reason.chars().take(REASON_CHARS).collect()
}

async fn screensaver_inhibit(conn: &zbus::Connection, who: &str, why: &str) -> zbus::Result<u32> {
    let why = if why.is_empty() { "Inhibited" } else { why };
    conn.call_method(
        Some(SCREENSAVER),
        SCREENSAVER_PATH,
        Some(SCREENSAVER),
        "Inhibit",
        &(who, why),
    )
    .await?
    .body()
    .deserialize::<u32>()
}

async fn screensaver_uninhibit(conn: &zbus::Connection, cookie: u32) {
    let _ = conn
        .call_method(
            Some(SCREENSAVER),
            SCREENSAVER_PATH,
            Some(SCREENSAVER),
            "UnInhibit",
            &(cookie,),
        )
        .await;
}

async fn sleep_inhibit(who: &str, why: &str) -> zbus::Result<OwnedFd> {
    let why = if why.is_empty() { "Inhibited" } else { why };
    let system = zbus::Connection::system().await?;
    let reply = system
        .call_method(
            Some(LOGIND),
            LOGIND_PATH,
            Some(LOGIND_MANAGER),
            "Inhibit",
            &("sleep", who, why, "block"),
        )
        .await?;
    let fd: zbus::OwnedFd = reply.body().deserialize()?;
    Ok(fd.into())
}

/// Release every inhibition and monitor session `owner` made; for a frontend that left the bus.
pub async fn release_owned_by(conn: &zbus::Connection, shared: &Shared, owner: &str) {
    let gone: Vec<(String, Held)> = shared
        .held
        .lock()
        .unwrap()
        .extract_if(|_, h| h.owner == owner)
        .collect();
    for (key, h) in gone {
        log::info!("Inhibit {key}: {owner} left the bus");
        shared.release(h);
        let _ = conn
            .object_server()
            .remove::<InhibitRequest, _>(key.as_str())
            .await;
    }
    let monitors: Vec<String> = shared
        .monitors
        .lock()
        .unwrap()
        .extract_if(|_, o| o == owner)
        .map(|(path, _)| path)
        .collect();
    for path in monitors {
        log::info!("Inhibit monitor {path}: {owner} left the bus");
        let _ = conn
            .object_server()
            .remove::<Monitor, _>(path.as_str())
            .await;
    }
}

struct InhibitRequest {
    path: String,
    shared: Shared,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl InhibitRequest {
    async fn close(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<()> {
        let owner = (self.shared.held.lock().unwrap().get(&self.path)).map(|h| h.owner.clone());
        crate::check_owner(&hdr, owner.as_deref())?;
        log::info!("Inhibit {} closed", self.path);
        let held = self.shared.held.lock().unwrap().remove(&self.path);
        if let Some(h) = held {
            self.shared.release(h);
        }
        let _ = server.remove::<InhibitRequest, _>(self.path.as_str()).await;
        Ok(())
    }
}

struct Monitor {
    path: OwnedObjectPath,
    shared: Shared,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl Monitor {
    async fn close(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let owner = (self.shared.monitors.lock().unwrap())
            .get(self.path.as_str())
            .cloned();
        crate::check_owner(&hdr, owner.as_deref())?;
        log::info!("Inhibit monitor {} closed", self.path.as_str());
        self.shared
            .monitors
            .lock()
            .unwrap()
            .remove(self.path.as_str());
        let _ = Monitor::closed(&emitter).await;
        let _ = server.remove::<Monitor, _>(self.path.as_str()).await;
        Ok(())
    }

    #[zbus(signal)]
    async fn closed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockmode::tests::{Bus, bus, connect};

    #[test]
    fn reasons_are_clipped_by_chars() {
        assert_eq!(clip("short"), "short");
        let long = "é".repeat(100);
        assert_eq!(clip(&long).chars().count(), REASON_CHARS);
    }

    #[test]
    fn the_cookie_is_taken_for_the_first_holder_and_given_back_after_the_last() {
        let mut c = IdleCount::default();
        assert_eq!(c.up(), Step::Take);
        assert!(c.taken(Some(7)));
        for _ in 0..9 {
            assert_eq!(c.up(), Step::Nothing, "later holders share the cookie");
        }
        for _ in 0..9 {
            assert_eq!(c.down(), Step::Nothing, "the cookie stays while one holds");
        }
        assert_eq!(c.down(), Step::Release(7));
        assert_eq!(
            c.down(),
            Step::Nothing,
            "a stray release gives back nothing"
        );
        assert_eq!(c.up(), Step::Take, "the next holder takes a new cookie");
    }

    #[test]
    fn a_monitor_state_is_running_with_the_lock_as_the_screensaver() {
        let s = state(true, false);
        assert_eq!(s["screensaver-active"], Variant::Bool(true));
        assert_eq!(s["session-state"], Variant::U32(1));
        assert_eq!(
            state(false, false)["screensaver-active"],
            Variant::Bool(false)
        );
        assert_eq!(
            state(false, true)["session-state"],
            Variant::U32(2),
            "an ending session is query-end"
        );
    }

    #[test]
    fn logout_holders_list_only_logout_inhibitions() {
        let (shared, _rx) = shared();
        let held = |logout: Option<(&str, &str)>| Held {
            logout: logout.map(|(a, r)| (a.to_string(), r.to_string())),
            ..Held::default()
        };
        {
            let mut h = shared.held.lock().unwrap();
            h.insert(
                "/r/2".into(),
                held(Some(("org.gnome.TextEditor", "Unsaved"))),
            );
            h.insert("/r/1".into(), held(Some(("", "Burning a disc"))));
            h.insert("/r/3".into(), held(None));
        }
        assert_eq!(
            shared.logout_holders(),
            [
                (String::new(), "Burning a disc".to_string()),
                ("org.gnome.TextEditor".to_string(), "Unsaved".to_string()),
            ]
        );
    }

    #[test]
    fn only_a_locked_hint_change_moves_the_lock() {
        let owned = |b: bool| Variant::Bool(b).try_to_owned().unwrap();
        let changed = HashMap::from([("LockedHint".to_string(), owned(true))]);
        assert_eq!(
            locked_hint_change(LOGIND_SESSION, &changed, &[]),
            Some(Some(true))
        );
        let idle = HashMap::from([("IdleHint".to_string(), owned(true))]);
        assert_eq!(locked_hint_change(LOGIND_SESSION, &idle, &[]), None);
        assert_eq!(
            locked_hint_change(LOGIND_SESSION, &idle, &["LockedHint".to_string()]),
            Some(None),
            "an invalidated hint is read again"
        );
        assert_eq!(
            locked_hint_change("org.freedesktop.login1.User", &changed, &[]),
            None,
            "another interface's LockedHint is not the session's"
        );
    }

    #[test]
    fn a_refused_take_leaves_no_holder() {
        let mut c = IdleCount::default();
        assert_eq!(c.up(), Step::Take);
        assert!(!c.taken(None));
        assert_eq!(c.down(), Step::Nothing);
        assert_eq!(c.up(), Step::Take, "the next holder asks again");
        assert!(c.taken(Some(3)));
        assert_eq!(c.down(), Step::Release(3));
    }

    struct FakeSaver {
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[zbus::interface(name = "org.freedesktop.ScreenSaver")]
    impl FakeSaver {
        fn inhibit(&self, app: String, reason: String) -> u32 {
            self.calls
                .lock()
                .unwrap()
                .push(format!("inhibit {app} {reason}"));
            7
        }

        fn un_inhibit(&self, cookie: u32) {
            self.calls
                .lock()
                .unwrap()
                .push(format!("uninhibit {cookie}"));
        }
    }

    /// Needs a bus: `dbus-run-session cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn idle_round_trip_on_a_session_bus() {
        zbus::block_on(async {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let _saver = zbus::connection::Builder::session()
                .name(SCREENSAVER)
                .serve_at(
                    SCREENSAVER_PATH,
                    FakeSaver {
                        calls: calls.clone(),
                    },
                )
                .build()
                .await
                .unwrap();
            let (shared, rx) = super::shared();
            let client = zbus::Connection::session().await.unwrap();
            let frontend = crate::Frontend::default();
            frontend.set(client.unique_name().map(|n| n.to_string()));
            let portal = zbus::connection::Builder::session()
                .serve_at(
                    crate::PATH,
                    Inhibit {
                        shared: shared.clone(),
                        frontend: frontend.clone(),
                    },
                )
                .build()
                .await
                .unwrap();
            let _ = frontend.conn.set(portal.clone());
            zruntime::spawn(run_idle(portal.clone(), rx)).detach();
            let dest = portal.unique_name().unwrap().to_string();
            let handles = [
                "/org/freedesktop/portal/desktop/request/1_1/a",
                "/org/freedesktop/portal/desktop/request/1_1/b",
            ];
            for handle in handles {
                let mut options: HashMap<&str, Variant<'_>> = HashMap::new();
                options.insert("reason", Variant::from("Playing"));
                client
                    .call_method(
                        Some(dest.as_str()),
                        crate::PATH,
                        Some("org.freedesktop.impl.portal.Inhibit"),
                        "Inhibit",
                        &(
                            ObjectPath::try_from(handle).unwrap(),
                            "org.example.Player",
                            "",
                            FLAG_IDLE,
                            options,
                        ),
                    )
                    .await
                    .unwrap();
            }
            let close = |handle: &'static str| {
                let (client, dest) = (client.clone(), dest.clone());
                async move {
                    client
                        .call_method(
                            Some(dest.as_str()),
                            handle,
                            Some("org.freedesktop.impl.portal.Request"),
                            "Close",
                            &(),
                        )
                        .await
                        .unwrap();
                }
            };
            close(handles[0]).await;
            assert!(shared.take_idle("probe", "").await);
            shared.release(Held {
                idle: true,
                ..Held::default()
            });
            assert_eq!(
                *calls.lock().unwrap(),
                ["inhibit org.example.Player Playing"],
                "one cookie for both, kept while the second holds"
            );
            close(handles[1]).await;
            assert!(shared.take_idle("probe", "").await);
            assert!(shared.held.lock().unwrap().is_empty());
            assert_eq!(
                *calls.lock().unwrap(),
                [
                    "inhibit org.example.Player Playing",
                    "uninhibit 7",
                    "inhibit probe Inhibited"
                ]
            );
        });
    }

    struct Desktop(Arc<AtomicBool>);

    #[zbus::interface(name = "org.freedesktop.ScreenSaver")]
    impl Desktop {
        fn get_active(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }

        #[zbus(signal)]
        async fn active_changed(emitter: &SignalEmitter<'_>, active: bool) -> zbus::Result<()>;
    }

    /// The compositor's `org.freedesktop.ScreenSaver` on `bus`, served from this process.
    struct Compositor(zbus::Connection, Arc<AtomicBool>);

    impl Compositor {
        async fn on(bus: &Bus, active: bool) -> Compositor {
            let active = Arc::new(AtomicBool::new(active));
            let conn = zbus::connection::Builder::address(bus.1.as_str())
                .serve_at(SCREENSAVER_PATH, Desktop(active.clone()))
                .name(SCREENSAVER)
                .build()
                .await
                .unwrap();
            Compositor(conn, active)
        }

        async fn lock(&self, active: bool) {
            self.1.store(active, Ordering::SeqCst);
            let emitter = SignalEmitter::new(&self.0, SCREENSAVER_PATH).unwrap();
            Desktop::active_changed(&emitter, active).await.unwrap();
        }
    }

    const SESSION_PATH: &str = "/org/freedesktop/login1/session/_31";

    struct Manager;

    #[zbus::interface(name = "org.freedesktop.login1.Manager")]
    impl Manager {
        #[zbus(name = "GetSessionByPID")]
        fn get_session_by_pid(&self, _pid: u32) -> OwnedObjectPath {
            OwnedObjectPath::try_from(SESSION_PATH).unwrap()
        }
    }

    struct Session(bool);

    #[zbus::interface(name = "org.freedesktop.login1.Session")]
    impl Session {
        #[zbus(property)]
        fn locked_hint(&self) -> bool {
            self.0
        }
    }

    /// logind on `bus`, whose one session's `LockedHint` is `locked`.
    struct Logind(zbus::Connection);

    impl Logind {
        async fn on(bus: &Bus, locked: bool) -> Logind {
            let conn = zbus::connection::Builder::address(bus.1.as_str())
                .serve_at(LOGIND_PATH, Manager)
                .serve_at(SESSION_PATH, Session(locked))
                .name(LOGIND)
                .build()
                .await
                .unwrap();
            Logind(conn)
        }

        async fn hint(&self, locked: bool) {
            let session = self.0.object_server();
            let session = session.interface::<_, Session>(SESSION_PATH).await.unwrap();
            session.get_mut().await.0 = locked;
            let emitter = session.signal_emitter();
            session
                .get()
                .await
                .locked_hint_changed(emitter)
                .await
                .unwrap();
        }
    }

    const MONITOR: &str = "/org/freedesktop/portal/desktop/session/1_1/m";

    /// A portal on `bus`, which also stands in for the system bus, following the lock of the
    /// compositor `pid` with one monitor session; the frontend's `StateChanged` stream.
    async fn portal(bus: &Bus, pid: u32) -> (zbus::Connection, zbus::MessageStream) {
        let (shared, _rx) = super::shared();
        let client = connect(bus).await;
        let frontend = crate::Frontend::default();
        frontend.set(client.unique_name().map(|n| n.to_string()));
        let portal = zbus::connection::Builder::address(bus.1.as_str())
            .serve_at(
                crate::PATH,
                Inhibit {
                    shared: shared.clone(),
                    frontend: frontend.clone(),
                },
            )
            .serve_at(
                crate::PATH,
                SessionEnd {
                    shared: shared.clone(),
                },
            )
            .build()
            .await
            .unwrap();
        let _ = frontend.conn.set(portal.clone());
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.impl.portal.Inhibit")
            .member("StateChanged")
            .build()
            .unwrap();
        let states = zbus::MessageStream::for_match_rule(rule, &client, None)
            .await
            .unwrap();
        let request = "/org/freedesktop/portal/desktop/request/1_1/r";
        client
            .call_method(
                portal.unique_name().map(|n| n.as_str()),
                crate::PATH,
                Some("org.freedesktop.impl.portal.Inhibit"),
                "CreateMonitor",
                &(
                    ObjectPath::try_from(request).unwrap(),
                    ObjectPath::try_from(MONITOR).unwrap(),
                    "org.example.App",
                    "",
                ),
            )
            .await
            .unwrap();
        let (conn, system) = (portal.clone(), connect(bus).await);
        zruntime::spawn(async move {
            let desktop = follow_desktop(&conn, &shared, pid);
            futures_lite::future::zip(desktop, follow_hint(&conn, &system, &shared, pid)).await;
        })
        .detach();
        (portal, states)
    }

    /// Whether the frontend hears (screensaver-active, session-state) `want` within 5 s.
    async fn hears(states: &mut zbus::MessageStream, want: (bool, u32)) -> bool {
        use futures_lite::StreamExt;
        let until = async_io::Timer::after(std::time::Duration::from_secs(5));
        let heard = async {
            while let Some(Ok(m)) = states.next().await {
                type Body = (OwnedObjectPath, HashMap<String, zbus::OwnedVariant>);
                let (path, state): Body = m.body().deserialize().unwrap();
                assert_eq!(path.as_str(), MONITOR);
                let active = bool::try_from(state["screensaver-active"].clone()).unwrap();
                let session = u32::try_from(state["session-state"].clone()).unwrap();
                if (active, session) == want {
                    return true;
                }
            }
            false
        };
        futures_lite::future::or(heard, async {
            until.await;
            false
        })
        .await
    }

    /// Every (screensaver-active, session-state) the frontend hears up to the first query-end,
    /// within 5 s.
    async fn heard_until_query_end(states: &mut zbus::MessageStream) -> Vec<(bool, u32)> {
        use futures_lite::StreamExt;
        let mut heard = Vec::new();
        let until = async_io::Timer::after(std::time::Duration::from_secs(5));
        let listen = async {
            while let Some(Ok(m)) = states.next().await {
                type Body = (OwnedObjectPath, HashMap<String, zbus::OwnedVariant>);
                let (_, state): Body = m.body().deserialize().unwrap();
                let active = bool::try_from(state["screensaver-active"].clone()).unwrap();
                let session = u32::try_from(state["session-state"].clone()).unwrap();
                heard.push((active, session));
                if session == SESSION_QUERY_END {
                    return;
                }
            }
        };
        futures_lite::future::or(listen, async {
            until.await;
        })
        .await;
        heard
    }

    /// The compositor's `SessionEnd.Query(ending)`.
    async fn end_session(portal: &zbus::Connection, bus: &Bus, ending: bool) {
        connect(bus)
            .await
            .call_method(
                portal.unique_name().map(|n| n.as_str()),
                crate::PATH,
                Some("org.reprise.SessionEnd"),
                "Query",
                &(ending,),
            )
            .await
            .unwrap();
    }

    #[test]
    fn the_remote_sides_lock_reads_active() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async {
            let compositor = Compositor::on(&bus, false).await;
            let (_portal, mut states) = portal(&bus, std::process::id()).await;
            assert!(hears(&mut states, (false, SESSION_RUNNING)).await);
            compositor.lock(true).await;
            assert!(hears(&mut states, (true, SESSION_RUNNING)).await);
            compositor.lock(false).await;
            assert!(hears(&mut states, (false, SESSION_RUNNING)).await);
        });
    }

    #[test]
    fn an_unverified_owner_reads_logind_locked_hint() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async {
            let logind = Logind::on(&bus, false).await;
            let _stranger = Compositor::on(&bus, true).await;
            let (_portal, mut states) = portal(&bus, 1).await;
            assert!(hears(&mut states, (false, SESSION_RUNNING)).await);
            logind.hint(true).await;
            assert!(hears(&mut states, (true, SESSION_RUNNING)).await);
            logind.hint(false).await;
            assert!(hears(&mut states, (false, SESSION_RUNNING)).await);
        });
    }

    #[test]
    fn a_logind_that_answers_late_still_stands_in() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async {
            let _stranger = Compositor::on(&bus, false).await;
            let (_portal, mut states) = portal(&bus, 1).await;
            async_io::Timer::after(std::time::Duration::from_millis(300)).await;
            let _logind = Logind::on(&bus, false).await;
            assert!(hears(&mut states, (false, SESSION_RUNNING)).await);
        });
    }

    #[test]
    fn a_verified_compositor_wins_over_locked_hint() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async {
            let logind = Logind::on(&bus, true).await;
            let _compositor = Compositor::on(&bus, false).await;
            let (portal, mut states) = portal(&bus, std::process::id()).await;
            assert!(hears(&mut states, (false, SESSION_RUNNING)).await);
            logind.hint(false).await;
            logind.hint(true).await;
            // Time for a hint that wrongly won to reach the monitors before the query-end fence.
            async_io::Timer::after(std::time::Duration::from_millis(300)).await;
            end_session(&portal, &bus, true).await;
            let heard = heard_until_query_end(&mut states).await;
            assert_eq!(heard.last(), Some(&(false, SESSION_QUERY_END)), "{heard:?}");
            assert!(heard.iter().all(|(active, _)| !active), "{heard:?}");
        });
    }

    #[test]
    fn a_lock_while_the_session_ends_keeps_query_end() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async {
            let compositor = Compositor::on(&bus, false).await;
            let (portal, mut states) = portal(&bus, std::process::id()).await;
            assert!(hears(&mut states, (false, SESSION_RUNNING)).await);
            end_session(&portal, &bus, true).await;
            assert!(hears(&mut states, (false, SESSION_QUERY_END)).await);
            compositor.lock(true).await;
            assert!(hears(&mut states, (true, SESSION_QUERY_END)).await);
            end_session(&portal, &bus, false).await;
            assert!(hears(&mut states, (true, SESSION_RUNNING)).await);
        });
    }

    #[test]
    fn a_monitor_first_hears_locked_before_any_reading() {
        use futures_lite::StreamExt;
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async {
            let (shared, _rx) = super::shared();
            let client = connect(&bus).await;
            client.request_name(crate::FRONTEND).await.unwrap();
            let frontend = crate::Frontend::default();
            frontend.set(client.unique_name().map(|n| n.to_string()));
            let portal = zbus::connection::Builder::address(bus.1.as_str())
                .serve_at(
                    crate::PATH,
                    Inhibit {
                        shared: shared.clone(),
                        frontend: frontend.clone(),
                    },
                )
                .build()
                .await
                .unwrap();
            let _ = frontend.conn.set(portal.clone());
            let mut states = zbus::MessageStream::for_match_rule(
                zbus::MatchRule::builder()
                    .msg_type(zbus::message::Type::Signal)
                    .interface("org.freedesktop.impl.portal.Inhibit")
                    .member("StateChanged")
                    .build()
                    .unwrap(),
                &client,
                None,
            )
            .await
            .unwrap();
            let session = "/org/freedesktop/portal/desktop/session/1_1/m";
            let monitor = |caller: zbus::Connection| {
                let dest = portal.unique_name().unwrap().to_string();
                async move {
                    caller
                        .call_method(
                            Some(dest.as_str()),
                            crate::PATH,
                            Some("org.freedesktop.impl.portal.Inhibit"),
                            "CreateMonitor",
                            &(
                                ObjectPath::try_from(
                                    "/org/freedesktop/portal/desktop/request/1_1/r",
                                )
                                .unwrap(),
                                ObjectPath::try_from(session).unwrap(),
                                "org.example.App",
                                "",
                            ),
                        )
                        .await
                }
            };
            let stranger = zbus::connection::Builder::address(bus.1.as_str())
                .build()
                .await
                .unwrap();
            let refused = monitor(stranger).await.unwrap_err();
            assert!(
                matches!(
                    zbus::fdo::Error::from(refused),
                    zbus::fdo::Error::AccessDenied(_)
                ),
                "a peer other than the frontend is refused"
            );
            assert!(shared.monitors.lock().unwrap().is_empty());
            let reply = client
                .call_method(
                    Some(portal.unique_name().unwrap().as_str()),
                    crate::PATH,
                    Some("org.freedesktop.impl.portal.Inhibit"),
                    "CreateMonitor",
                    &(
                        ObjectPath::try_from("/org/freedesktop/portal/desktop/request/1_1/r")
                            .unwrap(),
                        ObjectPath::try_from(session).unwrap(),
                        "org.example.App",
                        "",
                    ),
                )
                .await
                .unwrap();
            assert_eq!(reply.body().deserialize::<u32>().unwrap(), 0);
            let m = states.next().await.unwrap().unwrap();
            let (path, state): (OwnedObjectPath, HashMap<String, zbus::OwnedVariant>) =
                m.body().deserialize().unwrap();
            assert_eq!(path.as_str(), session);
            assert_eq!(
                bool::try_from(state["screensaver-active"].clone()),
                Ok(true),
                "a lock not yet read reads as locked"
            );
            assert_eq!(u32::try_from(state["session-state"].clone()), Ok(1));
        });
    }
}
