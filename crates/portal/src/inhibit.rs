//! `org.freedesktop.impl.portal.Inhibit`: an app keeping the screen awake or the machine from
//! sleeping until it closes the call's `Request`.
//!
//! Idle shares one `org.freedesktop.ScreenSaver` cookie, taken for the first inhibition and given
//! back after the last; Suspend holds a logind `sleep` block descriptor; Logout goes into the list
//! `org.reprise.SessionEnd` gives the compositor. A call that cannot hold what it asked for fails
//! and holds nothing.

use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};

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

/// The inhibitions not yet closed, by request handle, the monitor sessions by path with the
/// frontend that made each, the way to the idle task, whether the screen is locked and whether the
/// compositor is ending the session.
pub struct Inhibits {
    held: Mutex<HashMap<String, Held>>,
    monitors: Mutex<HashMap<String, String>>,
    idle: async_channel::Sender<IdleMsg>,
    locked: AtomicBool,
    ending: AtomicBool,
}

pub type Shared = Arc<Inhibits>;

/// The shared state, and the receiving end `run_idle` takes.
pub fn shared() -> (Shared, async_channel::Receiver<IdleMsg>) {
    let (tx, rx) = async_channel::unbounded();
    let shared = Inhibits {
        held: Mutex::default(),
        monitors: Mutex::default(),
        idle: tx,
        locked: AtomicBool::new(false),
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

    fn monitor_state(&self) -> HashMap<&'static str, Value<'static>> {
        state(
            self.locked.load(Ordering::Relaxed),
            self.ending.load(Ordering::Relaxed),
        )
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
        options: HashMap<String, Value<'_>>,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<()> {
        self.frontend.check(&hdr)?;
        crate::check_handle(&handle, crate::REQUEST_PREFIX)?;
        let reason = match options.get("reason") {
            Some(Value::Str(s)) => clip(s.as_str()),
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
        self.frontend.check(&hdr)?;
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
        conn.executor()
            .spawn(
                async move {
                    async_io::Timer::after(FIRST_STATE_DELAY).await;
                    // Unless the frontend closed it in the meantime.
                    if shared.monitors.lock().unwrap().contains_key(path.as_str()) {
                        emit_state(&conn2, &path, shared.monitor_state()).await;
                    }
                },
                "first monitor state",
            )
            .detach();
        Ok(crate::RESPONSE_OK)
    }

    // Nothing to answer: the session end waits on the inhibitors themselves.
    fn query_end_response(&self, _session_handle: ObjectPath<'_>) {}

    #[zbus(signal)]
    async fn state_changed(
        emitter: &SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        state: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<()>;
}

fn state(locked: bool, ending: bool) -> HashMap<&'static str, Value<'static>> {
    let session = if ending {
        SESSION_QUERY_END
    } else {
        SESSION_RUNNING
    };
    HashMap::from([
        ("screensaver-active", Value::Bool(locked)),
        ("session-state", Value::U32(session)),
    ])
}

async fn emit_state(
    conn: &zbus::Connection,
    session: &ObjectPath<'_>,
    state: HashMap<&'static str, Value<'static>>,
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

/// The `LockedHint` a `PropertiesChanged` from a logind session carries: Some(value) when it
/// changed, Some(None) when it was invalidated and must be read again, None when the signal is
/// about other properties.
fn locked_hint_change(
    iface: &str,
    changed: &HashMap<String, zbus::zvariant::OwnedValue>,
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

/// This session's `LockedHint`; None when logind cannot say.
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
    let v: zbus::zvariant::OwnedValue = reply.body().deserialize().ok()?;
    bool::try_from(v).ok()
}

/// Follow this session's lock through logind and tell every monitor session when it changes. The
/// portal runs as a user service outside any session, so logind's `auto` is the user's display
/// session.
pub async fn watch_lock(conn: zbus::Connection, shared: Shared) {
    use futures_lite::StreamExt;
    let system = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Inhibit monitor: no system bus ({e}); the lock is not followed");
            return;
        }
    };
    let session = system
        .call_method(
            Some(LOGIND),
            LOGIND_PATH,
            Some(LOGIND_MANAGER),
            "GetSession",
            &("auto",),
        )
        .await
        .and_then(|m| m.body().deserialize::<OwnedObjectPath>());
    let session = match session {
        Ok(s) => s,
        Err(e) => {
            log::warn!("Inhibit monitor: no logind session ({e}); the lock is not followed");
            return;
        }
    };
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(PROPERTIES)
        .and_then(|b| b.member("PropertiesChanged"))
        .and_then(|b| b.path(session.clone()))
        .and_then(|b| b.add_arg(LOGIND_SESSION))
        .map(|b| b.build().into());
    let rules = match rule {
        Ok(r) => vec![r],
        Err(e) => {
            log::warn!("Inhibit monitor: {e}");
            return;
        }
    };
    let mut signals = reprisebus::signals(&system, LOGIND, rules).await;
    let set = |locked: bool| {
        let (conn, shared) = (conn.clone(), shared.clone());
        async move {
            if shared.locked.swap(locked, Ordering::Relaxed) == locked {
                return;
            }
            log::info!("Inhibit monitor: screensaver-active {locked}");
            emit_all(&conn, &shared).await;
        }
    };
    if let Some(locked) = read_locked(&system, &session).await {
        set(locked).await;
    }
    while let Some(sig) = signals.next().await {
        let reread = match sig {
            reprisebus::Signal::OwnerChanged => true,
            reprisebus::Signal::Message(m) => {
                type Changed = (
                    String,
                    HashMap<String, zbus::zvariant::OwnedValue>,
                    Vec<String>,
                );
                let Ok((iface, changed, invalidated)) = m.body().deserialize::<Changed>() else {
                    continue;
                };
                match locked_hint_change(&iface, &changed, &invalidated) {
                    Some(Some(locked)) => {
                        set(locked).await;
                        false
                    }
                    Some(None) => true,
                    None => false,
                }
            }
        };
        if reread && let Some(locked) = read_locked(&system, &session).await {
            set(locked).await;
        }
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
    let fd: zbus::zvariant::OwnedFd = reply.body().deserialize()?;
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
        assert_eq!(s["screensaver-active"], Value::Bool(true));
        assert_eq!(s["session-state"], Value::U32(1));
        assert_eq!(
            state(false, false)["screensaver-active"],
            Value::Bool(false)
        );
        assert_eq!(
            state(false, true)["session-state"],
            Value::U32(2),
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
        let owned = |b: bool| Value::Bool(b).try_to_owned().unwrap();
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
                .unwrap()
                .name(SCREENSAVER)
                .unwrap()
                .serve_at(
                    SCREENSAVER_PATH,
                    FakeSaver {
                        calls: calls.clone(),
                    },
                )
                .unwrap()
                .build()
                .await
                .unwrap();
            let (shared, rx) = super::shared();
            let client = zbus::Connection::session().await.unwrap();
            let frontend = crate::Frontend::default();
            frontend.set(client.unique_name().map(|n| n.to_string()));
            let portal = zbus::connection::Builder::session()
                .unwrap()
                .serve_at(
                    crate::PATH,
                    Inhibit {
                        shared: shared.clone(),
                        frontend,
                    },
                )
                .unwrap()
                .build()
                .await
                .unwrap();
            portal
                .executor()
                .spawn(run_idle(portal.clone(), rx), "idle")
                .detach();
            let dest = portal.unique_name().unwrap().to_string();
            let handles = [
                "/org/freedesktop/portal/desktop/request/1_1/a",
                "/org/freedesktop/portal/desktop/request/1_1/b",
            ];
            for handle in handles {
                let mut options: HashMap<&str, Value<'_>> = HashMap::new();
                options.insert("reason", Value::from("Playing"));
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

    /// Needs a bus: `dbus-run-session cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn a_monitor_gets_its_first_state_on_a_session_bus() {
        use futures_lite::StreamExt;
        zbus::block_on(async {
            let (shared, _rx) = super::shared();
            let client = zbus::Connection::session().await.unwrap();
            let frontend = crate::Frontend::default();
            frontend.set(client.unique_name().map(|n| n.to_string()));
            let portal = zbus::connection::Builder::session()
                .unwrap()
                .serve_at(
                    crate::PATH,
                    Inhibit {
                        shared: shared.clone(),
                        frontend,
                    },
                )
                .unwrap()
                .build()
                .await
                .unwrap();
            let mut states = zbus::MessageStream::for_match_rule(
                zbus::MatchRule::builder()
                    .msg_type(zbus::message::Type::Signal)
                    .interface("org.freedesktop.impl.portal.Inhibit")
                    .unwrap()
                    .member("StateChanged")
                    .unwrap()
                    .build(),
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
            let stranger = zbus::connection::Builder::session()
                .unwrap()
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
            let (path, state): (OwnedObjectPath, HashMap<String, zbus::zvariant::OwnedValue>) =
                m.body().deserialize().unwrap();
            assert_eq!(path.as_str(), session);
            assert_eq!(
                bool::try_from(state["screensaver-active"].clone()),
                Ok(false)
            );
            assert_eq!(u32::try_from(state["session-state"].clone()), Ok(1));
        });
    }
}
