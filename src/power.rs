//! The laptop side: power source, lid, power and sleep buttons, whose logind handling a `block`
//! inhibitor holds off for the config's actions, and the logind session's lock signals and hints.

use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reprisekit::controls::osd::Level;
use smithay::{
    backend::input::{Switch, SwitchState},
    output::Output,
    reexports::calloop::{
        RegistrationToken,
        channel::{self, Sender},
        timer::{TimeoutAction, Timer},
    },
};
use zbus::{Connection, Proxy};
use zbus::{OwnedFd as ZOwnedFd, OwnedObjectPath};

use crate::compositor::State;
use crate::config::PowerAction;
use crate::lockview::Origin;
use crate::media::Value;
use crate::sessionlock::CoverWait;

const UPOWER_NAME: &str = "org.freedesktop.UPower";
const UPOWER_PATH: &str = "/org/freedesktop/UPower";
pub(crate) const LOGIN1_NAME: &str = "org.freedesktop.login1";
pub(crate) const LOGIN1_PATH: &str = "/org/freedesktop/login1";
pub(crate) const LOGIN1_MANAGER: &str = "org.freedesktop.login1.Manager";
const LOGIN1_SESSION: &str = "org.freedesktop.login1.Session";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// Well under logind's `InhibitDelayMaxSec` (5 s by default).
const COVER_WAIT: Duration = Duration::from_millis(2000);
/// How long after a refused sleep delay inhibitor it is asked for again.
const DELAY_RETRY: Duration = Duration::from_secs(30);
/// How long after logind refused a hint it is sent again.
const HINT_RETRY: Duration = Duration::from_secs(5);
/// How long a closed lid with no external output waits before its action
/// when closing didn't cause that state (the lid at startup, reported before
/// any output exists, or the last external output leaving). Gives late
/// outputs (a dock's, a display that drops out briefly) time to show up.
/// logind's `HoldoffTimeoutSec` default.
const LID_GRACE: Duration = Duration::from_secs(30);

/// Who the login manager's inhibitors name.
const LOGIN_MANAGER: &str = "Reprise login manager";

/// Under `$XDG_RUNTIME_DIR`: the darkened panels, one `connector level` per line.
const DARKENED_RECORD: &str = "reprise-darkened";

/// None only if a component does not parse as a bus name or path.
pub(crate) fn signal_rule(
    sender: &str,
    path: &str,
    interface: &str,
    member: &str,
) -> Option<zbus::OwnedMatchRule> {
    zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(sender)
        .path(path)
        .interface(interface)
        .member(member)
        .build()
        .map(Into::into)
        .ok()
}

pub(crate) enum Report {
    OnBattery(bool),
    /// Kept until shutdown so logind leaves the lid and buttons to us.
    Inhibitor(OwnedFd),
    /// Taken at startup and after every resume; dropped once the lock cover is on every screen.
    SleepDelay(OwnedFd),
    /// `PrepareForSleep(true)`: the machine sleeps once every delay inhibitor is released.
    Sleeping,
    /// `ListInhibitors` read again (`remote_elsewhere`).
    RemoteElsewhere(bool),
    /// The login screen's hold on logind's lid switch, for a remote client elsewhere.
    LidHold(OwnedFd),
    Resumed,
    Lock,
    Unlock,
    /// The session's `State` read `closing`: its leader is gone.
    Closing,
    /// `org.freedesktop.login1` changed hands: a restarted logind keeps no hints.
    LogindRestarted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    Locked(bool),
    Idle(bool),
}

/// What the loop asks of its logind session object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Call {
    Hint(Hint),
    /// Read `State`; a `closing` session is reported.
    State,
    /// Take logind's lid switch for the login screen, answered by `Report::LidHold`.
    HoldLid,
}

/// The hints logind heard, or hears once the loop starts, so each goes out only on change.
/// Both start false, as for a new session.
#[derive(Default)]
struct SessionHints {
    locked: bool,
    idle: bool,
}

impl SessionHints {
    fn changed(&mut self, h: Hint) -> bool {
        let (slot, v) = match h {
            Hint::Locked(v) => (&mut self.locked, v),
            Hint::Idle(v) => (&mut self.idle, v),
        };
        std::mem::replace(slot, v) != v
    }
}

#[derive(Default)]
pub struct PowerState {
    pub on_battery: bool,
    _inhibitor: Option<OwnedFd>,
    sleep_delay: Option<OwnedFd>,
    cover_wait: Option<CoverWait<Output>>,
    cover_timer: Option<RegistrationToken>,
    hints: SessionHints,
    calls: Option<async_channel::Sender<Call>>,
    pub lid_closed: bool,
    /// The lid at the login screen, which keeps it apart from `lid_closed`: it only darkens
    /// the panels under a remote client elsewhere.
    greeter_lid_closed: bool,
    /// Runs the lid action `LID_GRACE` after arming if the lid is still closed with no external output.
    lid_timer: Option<RegistrationToken>,
    /// The internal panels a shut lid darkened under remote control, with the brightness each had.
    darkened: Vec<(String, u8)>,
    /// Where `darkened` is kept, so a compositor started after a crash brings the panels back.
    record: Option<PathBuf>,
    /// The record a previous compositor left, each panel restored once it reads 0.
    left_dark: Vec<(String, u8)>,
    /// What `remote_control_changed` last saw of `State::remote_controlled`.
    remote: bool,
    /// A remote client works in another session on this machine (`remote_elsewhere`).
    pub remote_elsewhere: bool,
    /// The login screen's hold on logind's lid switch while `remote_elsewhere`.
    lid_hold: Option<OwnedFd>,
}

/// One of logind's `ListInhibitors`: what, who, why, mode, uid, pid.
pub type Inhibitor = (String, String, String, String, u32, u32);

/// Whether the login manager holds sleep off for a remote client: a `block` inhibitor on `sleep`
/// that root holds in its name.
pub fn remote_elsewhere(inhibitors: &[Inhibitor]) -> bool {
    inhibitors.iter().any(|(what, who, _, mode, uid, _)| {
        who == LOGIN_MANAGER
            && mode == "block"
            && *uid == 0
            && what.split(':').any(|w| w == "sleep")
    })
}

/// The login manager's sleep inhibitor came or went: idle re-arms, and a shut lid's panels go
/// dark or come back as under remote control.
pub(crate) fn remote_elsewhere_changed(state: &mut State, elsewhere: bool) {
    if std::mem::replace(&mut state.power.remote_elsewhere, elsewhere) == elsewhere {
        return;
    }
    log::info!(
        "power: a remote client in another session {}",
        if elsewhere { "connected" } else { "left" }
    );
    if state.host == crate::compositor::Host::Greeter {
        hold_lid(state, elsewhere);
    }
    crate::idle::arm(state);
    remote_lid(state);
}

/// The login screen takes logind's lid switch while a remote client works elsewhere and lets
/// it go when the client leaves.
fn hold_lid(state: &mut State, hold: bool) {
    if !hold {
        state.power.lid_hold = None;
    } else if let Some(tx) = &state.power.calls
        && state.power.lid_hold.is_none()
    {
        let _ = tx.try_send(Call::HoldLid);
    }
}

/// Whether a remote client holds the lid's action off: this session's or another's.
fn remote_held(state: &State) -> bool {
    state.remote_controlled() || state.power.remote_elsewhere
}

/// Runs on the D-Bus thread until the bus goes away.
async fn run(reports: Sender<Report>, calls: async_channel::Receiver<Call>) {
    let conn = match Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            log::info!("power: no system bus ({e}); always plugged in");
            return;
        }
    };
    if let Ok(login1) = Proxy::new(&conn, LOGIN1_NAME, LOGIN1_PATH, LOGIN1_MANAGER).await {
        let what = "handle-lid-switch:handle-power-key:handle-suspend-key";
        match login1
            .call_method(
                "Inhibit",
                &(
                    what,
                    "reprise",
                    "the compositor handles the lid and the buttons",
                    "block",
                ),
            )
            .await
        {
            Ok(reply) => match reply.body().deserialize::<ZOwnedFd>() {
                Ok(fd) => {
                    let _ = reports.send(Report::Inhibitor(fd.into()));
                }
                Err(e) => log::warn!("power: logind's Inhibit answered oddly: {e}"),
            },
            Err(e) => {
                log::info!("power: no logind inhibitor ({e}); logind keeps the lid and buttons")
            }
        }
        match session_path(&login1).await {
            // Uncached: logind sends no change signal for `State`.
            Ok(path) => match session_proxy(&conn, path).await {
                Ok(session) => {
                    let watch =
                        watch_session(conn.clone(), session, calls, reports.clone(), HINT_RETRY);
                    std::thread::Builder::new()
                        .name("power-session".into())
                        .spawn(move || zbus::block_on(watch))
                        .ok();
                }
                Err(e) => log::info!("power: no session proxy ({e})"),
            },
            Err(e) => log::info!("power: no logind session ({e}); no lock-session, no hints"),
        }
        let inhibitors = watch_inhibitors(login1.clone(), reports.clone());
        std::thread::Builder::new()
            .name("power-inhibitors".into())
            .spawn(move || zbus::block_on(inhibitors))
            .ok();
        let sleep = watch_sleep(login1, reports.clone(), DELAY_RETRY);
        std::thread::Builder::new()
            .name("power-sleep".into())
            .spawn(move || zbus::block_on(sleep))
            .ok();
    }
    let Ok(upower) = Proxy::new(&conn, UPOWER_NAME, UPOWER_PATH, UPOWER_NAME).await else {
        return;
    };
    // Subscribe before the first read so a flip in between is not lost. Only
    // UPower's own broadcasts get through (see `reprisebus::signals`).
    let Some(rule) = signal_rule(UPOWER_NAME, UPOWER_PATH, PROPERTIES, "PropertiesChanged") else {
        return;
    };
    let mut changes = reprisebus::signals(&conn, UPOWER_NAME, vec![rule]).await;
    match upower.get_property::<bool>("OnBattery").await {
        Ok(v) => {
            let _ = reports.send(Report::OnBattery(v));
        }
        Err(e) => {
            log::info!("power: UPower unavailable ({e}); always plugged in");
            return;
        }
    }
    use futures_util::stream::StreamExt;
    while let Some(sig) = changes.next().await {
        let reprisebus::Signal::Message(msg) = sig else {
            continue;
        };
        let Ok((iface, changed, _invalidated)) = msg.body().deserialize::<(
            String,
            std::collections::HashMap<String, zbus::OwnedVariant>,
            Vec<String>,
        )>() else {
            continue;
        };
        if iface != UPOWER_NAME {
            continue;
        }
        if let Some(v) = changed
            .get("OnBattery")
            .and_then(|v| bool::try_from(v.clone()).ok())
        {
            let _ = reports.send(Report::OnBattery(v));
        }
    }
}

/// By `XDG_SESSION_ID`, else by our pid. Not `session/auto`, which is the
/// D-Bus caller's session as logind sees it and may be another one.
async fn session_path(login1: &Proxy<'_>) -> zbus::Result<OwnedObjectPath> {
    let reply = match std::env::var("XDG_SESSION_ID") {
        Ok(id) if !id.is_empty() => login1.call_method("GetSession", &(id,)).await?,
        _ => {
            login1
                .call_method("GetSessionByPID", &(std::process::id(),))
                .await?
        }
    };
    reply.body().deserialize()
}

async fn session_proxy(conn: &Connection, path: OwnedObjectPath) -> zbus::Result<Proxy<'static>> {
    zbus::proxy::Builder::new(conn)
        .destination(LOGIN1_NAME)
        .path(path)
        .interface(LOGIN1_SESSION)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
}

/// Only logind's own broadcasts get through (see `reprisebus::signals`), so a
/// spoofed `Unlock` cannot end the lock.
async fn watch_session(
    conn: Connection,
    session: Proxy<'static>,
    calls: async_channel::Receiver<Call>,
    reports: Sender<Report>,
    retry: Duration,
) {
    use futures_util::stream::StreamExt;
    let signals = async {
        let path = session.path().to_string();
        let mut rules = Vec::new();
        for member in ["Lock", "Unlock"] {
            let Some(rule) = signal_rule(LOGIN1_NAME, &path, LOGIN1_SESSION, member) else {
                log::info!("power: no Lock/Unlock signals; lock-session does nothing");
                return;
            };
            rules.push(rule);
        }
        let mut stream = reprisebus::signals(&conn, LOGIN1_NAME, rules).await;
        while let Some(sig) = stream.next().await {
            let m = match sig {
                reprisebus::Signal::Message(m) => m,
                reprisebus::Signal::OwnerChanged => {
                    let _ = reports.send(Report::LogindRestarted);
                    continue;
                }
            };
            let r = match m.header().member().map(|m| m.as_str()) {
                Some("Lock") => Report::Lock,
                Some("Unlock") => Report::Unlock,
                _ => continue,
            };
            let _ = reports.send(r);
        }
    };
    let setter = async {
        // Each hint logind refused, by kind, until it lands or a newer one of its kind comes.
        let mut unsent: [Option<Hint>; 2] = [None, None];
        let kind = |h: Hint| match h {
            Hint::Locked(_) => 0,
            Hint::Idle(_) => 1,
        };
        loop {
            let next = if unsent.iter().any(Option::is_some) {
                let wait = async {
                    async_io::Timer::after(retry).await;
                    None
                };
                futures_lite::future::or(async { Some(calls.recv().await) }, wait).await
            } else {
                Some(calls.recv().await)
            };
            let hints: Vec<Hint> = match next {
                None => unsent.iter_mut().filter_map(Option::take).collect(),
                Some(Err(_)) => return,
                Some(Ok(Call::Hint(h))) => {
                    unsent[kind(h)] = None;
                    vec![h]
                }
                Some(Ok(Call::State)) => {
                    match session.get_property::<String>("State").await {
                        Ok(s) if s == "closing" => {
                            let _ = reports.send(Report::Closing);
                        }
                        Ok(_) => {}
                        Err(e) => log::debug!("power: reading the session's State: {e}"),
                    }
                    continue;
                }
                Some(Ok(Call::HoldLid)) => continue,
            };
            for h in hints {
                let (method, v) = match h {
                    Hint::Locked(v) => ("SetLockedHint", v),
                    Hint::Idle(v) => ("SetIdleHint", v),
                };
                if let Err(e) = session.call_method(method, &(v,)).await {
                    if next.is_some() {
                        log::warn!("power: {method}({v}) failed: {e}; sent again every {retry:?}");
                    }
                    unsent[kind(h)] = Some(h);
                }
            }
        }
    };
    futures_lite::future::zip(signals, setter).await;
}

async fn sleep_delay(login1: &Proxy<'_>) -> Result<OwnedFd, String> {
    let reply = login1
        .call_method(
            "Inhibit",
            &(
                "sleep",
                "reprise",
                "locking the screen before sleep",
                "delay",
            ),
        )
        .await
        .map_err(|e| e.to_string())?;
    let fd = reply
        .body()
        .deserialize::<ZOwnedFd>()
        .map_err(|e| e.to_string())?;
    Ok(fd.into())
}

/// Whether the delay inhibitor was taken; a refusal logs at info when `loud`, else at debug.
async fn take_sleep_delay(login1: &Proxy<'_>, reports: &Sender<Report>, loud: bool) -> bool {
    match sleep_delay(login1).await {
        Ok(fd) => {
            let _ = reports.send(Report::SleepDelay(fd));
            true
        }
        Err(e) => {
            let level = if loud {
                log::Level::Info
            } else {
                log::Level::Debug
            };
            log::log!(
                level,
                "power: no sleep delay inhibitor ({e}); asked again in {retry:?}",
                retry = DELAY_RETRY
            );
            false
        }
    }
}

/// `true` hands the loop the moment to lock (it drops the fd once the cover
/// is on screen, letting the sleep go on); `false` is the resume, when the
/// inhibitor is taken again. A refused inhibitor is asked for again every
/// `retry` until one is taken or a sleep starts without it.
async fn watch_sleep(login1: Proxy<'static>, reports: Sender<Report>, retry: Duration) {
    use futures_util::stream::StreamExt;
    // Only logind's own broadcast is acted on (see `reprisebus::signals`), so a
    // spoofed `PrepareForSleep` cannot lock the screen or move the inhibitor.
    let Some(rule) = signal_rule(LOGIN1_NAME, LOGIN1_PATH, LOGIN1_MANAGER, "PrepareForSleep")
    else {
        log::info!("power: no PrepareForSleep signal; no lock on sleep");
        return;
    };
    let mut signals = reprisebus::signals(login1.connection(), LOGIN1_NAME, vec![rule]).await;
    let mut ask_again = !take_sleep_delay(&login1, &reports, true).await;
    loop {
        let next = if ask_again {
            let wait = async {
                async_io::Timer::after(retry).await;
                None
            };
            futures_lite::future::or(async { Some(signals.next().await) }, wait).await
        } else {
            Some(signals.next().await)
        };
        let Some(sig) = next else {
            ask_again = !take_sleep_delay(&login1, &reports, false).await;
            continue;
        };
        let Some(sig) = sig else {
            return;
        };
        let reprisebus::Signal::Message(msg) = sig else {
            continue;
        };
        let Ok(starting) = msg.body().deserialize::<bool>() else {
            continue;
        };
        if starting {
            // One taken now would hold this sleep back past the cover.
            ask_again = false;
            let _ = reports.send(Report::Sleeping);
        } else {
            let _ = reports.send(Report::Resumed);
            ask_again = !take_sleep_delay(&login1, &reports, true).await;
        }
    }
}

/// logind's `BlockInhibited` changes as block inhibitors come and go; each change, and each new
/// logind, reads `ListInhibitors` again. Only logind's own broadcasts get through.
async fn watch_inhibitors(login1: Proxy<'static>, reports: Sender<Report>) {
    use futures_util::stream::StreamExt;
    let Some(rule) = signal_rule(LOGIN1_NAME, LOGIN1_PATH, PROPERTIES, "PropertiesChanged") else {
        return;
    };
    let mut changes = reprisebus::signals(login1.connection(), LOGIN1_NAME, vec![rule]).await;
    read_inhibitors(&login1, &reports).await;
    while let Some(sig) = changes.next().await {
        if let reprisebus::Signal::Message(msg) = sig {
            let Ok((iface, changed, invalidated)) = msg.body().deserialize::<(
                String,
                std::collections::HashMap<String, zbus::OwnedVariant>,
                Vec<String>,
            )>() else {
                continue;
            };
            let block = "BlockInhibited";
            if iface != LOGIN1_MANAGER
                || !(changed.contains_key(block) || invalidated.iter().any(|p| p == block))
            {
                continue;
            }
        }
        read_inhibitors(&login1, &reports).await;
    }
}

async fn read_inhibitors(login1: &Proxy<'_>, reports: &Sender<Report>) {
    let list = match login1.call_method("ListInhibitors", &()).await {
        Ok(reply) => reply.body().deserialize::<Vec<Inhibitor>>(),
        Err(e) => Err(e),
    };
    match list {
        Ok(list) => {
            let _ = reports.send(Report::RemoteElsewhere(remote_elsewhere(&list)));
        }
        Err(e) => log::info!("power: no ListInhibitors ({e}); no remote client seen elsewhere"),
    }
}

pub(crate) fn report(state: &mut State, r: Report) {
    match r {
        Report::Inhibitor(fd) => state.power._inhibitor = Some(fd),
        Report::SleepDelay(fd) => {
            stop_cover_wait(state);
            state.power.sleep_delay = Some(fd);
        }
        Report::Sleeping => sleeping(state),
        Report::RemoteElsewhere(v) => remote_elsewhere_changed(state, v),
        Report::LidHold(fd) => {
            if state.power.remote_elsewhere {
                state.power.lid_hold = Some(fd);
            }
        }
        Report::Resumed => {
            crate::idle::resumed(state);
            crate::nightlight::update(state);
            crate::displays::reread(state);
        }
        Report::Lock => {
            log::info!("power: logind asks for the lock");
            crate::lockview::lock_all(state);
        }
        Report::Unlock if crate::loginlink::refuses_unlock(state) => {
            log::warn!("power: logind ends the lock off the seat with no reprise-login link; kept");
        }
        Report::Unlock => {
            log::info!("power: logind ends the lock");
            crate::lockview::unlock(state);
        }
        Report::Closing => crate::loginlink::session_closing(state),
        Report::LogindRestarted => {
            log::info!("power: logind changed hands; the hints go out again");
            send_hints(state);
        }
        Report::OnBattery(v) => {
            if state.power.on_battery != v {
                log::info!("power: {}", if v { "on battery" } else { "plugged in" });
                state.power.on_battery = v;
                crate::idle::arm(state);
            }
        }
    }
}

/// Whether going to sleep locks, and whether it then waits for the cover: only a held delay
/// inhibitor can hold the sleep back.
fn sleep_plan(lock_on_sleep: bool, has_delay: bool) -> (bool, bool) {
    (lock_on_sleep, lock_on_sleep && has_delay)
}

/// `PrepareForSleep(true)`: lock, then hold logind's delay until every output
/// that is on has presented the cover, or `COVER_WAIT` has passed.
fn sleeping(state: &mut State) {
    if state.power.cover_wait.is_some() {
        return;
    }
    let has_delay = state.power.sleep_delay.is_some();
    let (lock, wait) = sleep_plan(state.config.power.lock_on_sleep, has_delay);
    if lock {
        log::info!("power: sleeping, locking first");
        crate::lockview::lock_all(state);
    }
    if !wait {
        if has_delay {
            release_sleep(state, "sleeping without a lock");
        } else if lock {
            log::info!("power: no delay inhibitor; locking without waiting for the cover");
        }
        return;
    }
    let outputs: Vec<Output> = state.scene.outputs().to_vec();
    let mut armed = Vec::new();
    for o in outputs {
        if crate::backend::output_power(state, &o) == Some(true)
            && !crate::backend::stalled(state, &o)
        {
            crate::backend::schedule(state, &o);
            armed.push(o);
        }
    }
    let wait = CoverWait::new(armed);
    if wait.done() {
        release_sleep(state, "no output to show the cover");
        return;
    }
    state.power.cover_wait = Some(wait);
    let timer = state
        .handle
        .insert_source(Timer::from_duration(COVER_WAIT), |_, _, state| {
            state.power.cover_timer = None;
            if state.power.cover_wait.is_some() {
                release_sleep(state, "the cover was not presented in time");
            }
            TimeoutAction::Drop
        });
    match timer {
        Ok(t) => state.power.cover_timer = Some(t),
        Err(e) => log::warn!("power: cover timer: {}", e.error),
    }
}

fn stop_cover_wait(state: &mut State) {
    state.power.cover_wait = None;
    if let Some(t) = state.power.cover_timer.take() {
        state.handle.remove(t);
    }
}

pub fn hint(state: &mut State, h: Hint) {
    let p = &mut state.power;
    if p.hints.changed(h)
        && let Some(tx) = &p.calls
    {
        let _ = tx.try_send(Call::Hint(h));
    }
}

/// Asks logind for the session's `State`; a `closing` one comes back as `Report::Closing`.
pub fn check_closing(state: &mut State) {
    if let Some(tx) = &state.power.calls {
        let _ = tx.try_send(Call::State);
    }
}

/// The calls the loop makes from now on, for a test to read.
#[cfg(test)]
pub fn capture_calls(state: &mut State) -> async_channel::Receiver<Call> {
    let (tx, rx) = async_channel::unbounded();
    attach(state, tx);
    rx
}

/// The calls `calls` holds, past every hint.
#[cfg(test)]
pub fn calls_but_hints(calls: &async_channel::Receiver<Call>) -> Vec<Call> {
    std::iter::from_fn(|| calls.try_recv().ok())
        .filter(|c| !matches!(c, Call::Hint(_)))
        .collect()
}

/// Hands the loop its calls; both hints go out first, so logind drops what a crashed
/// compositor left and hears a lock set before the loop ran.
fn attach(state: &mut State, tx: async_channel::Sender<Call>) {
    state.power.calls = Some(tx);
    send_hints(state);
}

/// Both hints as they stand, whatever logind last heard.
fn send_hints(state: &State) {
    let h = &state.power.hints;
    if let Some(tx) = &state.power.calls {
        for hint in [Hint::Locked(h.locked), Hint::Idle(h.idle)] {
            let _ = tx.try_send(Call::Hint(hint));
        }
    }
}

/// A sleep still waiting for the cover goes on, since there is none left to wait for.
pub fn unlocked(state: &mut State) {
    hint(state, Hint::Locked(false));
    if state.power.cover_wait.is_some() {
        release_sleep(state, "unlocked before the cover showed");
    }
}

fn release_sleep(state: &mut State, why: &str) {
    log::info!("power: {why}; sleep goes on");
    stop_cover_wait(state);
    state.power.sleep_delay = None;
}

/// A queued frame (`drawn`) counts toward a pending cover wait once
/// presented; a frame with nothing new (`empty`) shows the cover already.
pub fn rendered(state: &mut State, output: &Output, drawn: bool, empty: bool) {
    let Some(wait) = state.power.cover_wait.as_mut() else {
        return;
    };
    if drawn {
        wait.queued(output);
    } else if empty {
        wait.forget(output);
    }
    if wait.done() {
        release_sleep(state, "the cover is on every output");
    }
}

pub fn presented(state: &mut State, output: &Output) {
    let Some(wait) = state.power.cover_wait.as_mut() else {
        return;
    };
    wait.presented(output);
    if wait.done() {
        release_sleep(state, "the cover is on every output");
    }
}

/// `output`'s commit stalled (`backend::stalled`): a sleep stops waiting for its cover.
pub fn stalled(state: &mut State, output: &Output) {
    let Some(wait) = state.power.cover_wait.as_mut() else {
        return;
    };
    wait.forget(output);
    if wait.done() {
        release_sleep(state, "the cover is on every output that is not stalled");
    }
}

/// A gone output stops holding up a sleep, and the lid's state is weighed again.
pub fn outputs_changed(state: &mut State) {
    if let Some(wait) = state.power.cover_wait.as_mut() {
        let live = state.scene.outputs();
        wait.retain(|o| live.contains(o));
        if wait.done() {
            release_sleep(state, "the last waiting output went away");
        }
    }
    follow_lid(state);
}

pub fn setup(state: &mut State) {
    let record = std::env::var_os("XDG_RUNTIME_DIR").map(|d| Path::new(&d).join(DARKENED_RECORD));
    state.power.left_dark = record.as_deref().map(read_record).unwrap_or_default();
    state.power.record = record;
    let (tx, rx) = channel::channel::<Report>();
    if let Err(e) = state.handle.insert_source(rx, |event, _, state| {
        if let channel::Event::Msg(r) = event {
            report(state, r);
        }
    }) {
        log::warn!("power: no report channel ({}); always plugged in", e.error);
        return;
    }
    let (calls_tx, calls_rx) = async_channel::unbounded::<Call>();
    let thread = std::thread::Builder::new()
        .name("power-dbus".into())
        .spawn(move || zbus::block_on(run(tx, calls_rx)));
    if thread.is_ok() {
        attach(state, calls_tx);
    }
}

/// The login screen's part: the login manager's sleep inhibitor is watched, and logind's lid
/// switch held while it says a remote client works elsewhere.
pub fn setup_greeter(state: &mut State) {
    let (tx, rx) = channel::channel::<Report>();
    if let Err(e) = state.handle.insert_source(rx, |event, _, state| {
        if let channel::Event::Msg(r) = event {
            report(state, r);
        }
    }) {
        log::warn!(
            "power: no report channel ({}); logind keeps the lid",
            e.error
        );
        return;
    }
    let (calls_tx, calls_rx) = async_channel::unbounded::<Call>();
    let thread = std::thread::Builder::new()
        .name("greeter-power".into())
        .spawn(move || zbus::block_on(run_greeter(tx, calls_rx)));
    if thread.is_ok() {
        attach(state, calls_tx);
    }
}

async fn run_greeter(reports: Sender<Report>, calls: async_channel::Receiver<Call>) {
    let login1 = match Connection::system().await {
        Ok(conn) => Proxy::new(&conn, LOGIN1_NAME, LOGIN1_PATH, LOGIN1_MANAGER).await,
        Err(e) => Err(e),
    };
    let login1 = match login1 {
        Ok(p) => p,
        Err(e) => return log::info!("power: no logind ({e}); logind keeps the lid"),
    };
    let inhibitors = watch_inhibitors(login1.clone(), reports.clone());
    std::thread::Builder::new()
        .name("power-inhibitors".into())
        .spawn(move || zbus::block_on(inhibitors))
        .ok();
    while let Ok(c) = calls.recv().await {
        if c != Call::HoldLid {
            continue;
        }
        let why = "a remote client is connected; a shut lid darkens the panel";
        let taken = login1
            .call_method("Inhibit", &("handle-lid-switch", "reprise", why, "block"))
            .await
            .and_then(|reply| reply.body().deserialize::<ZOwnedFd>());
        match taken {
            Ok(fd) => {
                let _ = reports.send(Report::LidHold(fd.into()));
            }
            Err(e) => log::warn!("power: no lid inhibitor for the login screen: {e}"),
        }
    }
}

pub fn run_action(state: &mut State, a: PowerAction, origin: Origin) {
    log::info!("power: {}", a.name());
    match a {
        PowerAction::Nothing => {}
        PowerAction::ScreenOff => crate::idle::screen_off_by(state, origin),
        PowerAction::Lock => crate::lockview::lock_for(state, origin),
        PowerAction::Standby => {
            let cmd = state.config.power.standby.clone();
            crate::sessionend::sleep(state, crate::sessionend::Act::StandBy, cmd);
        }
        PowerAction::Hibernate => {
            let cmd = state.config.power.hibernate.clone();
            crate::sessionend::sleep(state, crate::sessionend::Act::Hibernate, cmd);
        }
        PowerAction::TurnOff => {
            let cmd = state.config.power.poweroff.clone();
            crate::sessionend::begin(
                state,
                reprisekit::controls::endsession::Goal::TurnOff,
                Some(cmd),
                origin,
            );
        }
        PowerAction::Ask => {
            // Dialogs live under the lock cover, where no key reaches them: a locked
            // session has nothing to ask.
            if state.locked_for(origin) {
                log::info!("power: ask ignored while locked");
                return;
            }
            let offered = crate::console::disconnect_offered(state);
            let d = crate::dialog::Dialog::turn_off(&state.config.power, offered);
            state.open_dialog(d);
        }
    }
}

pub fn is_internal_output(name: &str) -> bool {
    ["eDP-", "LVDS-", "DSI-"]
        .iter()
        .any(|p| name.starts_with(p))
}

/// Always with the lid open; with it closed only when nothing else would show the desktop.
pub fn internal_wanted(lid_closed: bool, external_in_layout: bool) -> bool {
    !lid_closed || !external_in_layout
}

/// Whether a monitor besides the built-in panels shows the desktop; the standby screen is none.
fn external_in_layout(state: &State) -> bool {
    state.scene.outputs().iter().any(|o| {
        !is_internal_output(&o.name())
            && !crate::backend::is_virtual(o)
            && state.layout_box(Some(o)).width > 0
    })
}

/// Clamshell mode: take the built-in panels out of the layout while the lid
/// is closed and an external output shows the desktop, and put them back when
/// the lid opens or the last external output goes. A panel returning under a
/// closed lid arms the lid check. Runs on every lid toggle and layout change of a session.
pub fn follow_lid(state: &mut State) {
    if state.host == crate::compositor::Host::Greeter {
        return;
    }
    let want = internal_wanted(state.power.lid_closed, external_in_layout(state));
    let panels: Vec<(smithay::output::Output, bool)> = crate::backend::heads(state)
        .into_iter()
        .filter(|h| is_internal_output(&h.output.name()))
        .map(|h| (h.output, h.enabled))
        .collect();
    let mut undocked = false;
    for (panel, shown) in panels {
        if shown != want {
            state.set_in_layout(&panel, want);
            undocked |= want && state.power.lid_closed;
        }
    }
    if undocked {
        log::info!("power: undocked with the lid closed");
        arm_lid_check(state);
    }
}

/// Arming again restarts the wait.
fn arm_lid_check(state: &mut State) {
    if let Some(t) = state.power.lid_timer.take() {
        state.handle.remove(t);
    }
    let timer = state
        .handle
        .insert_source(Timer::from_duration(LID_GRACE), |_, _, state| {
            state.power.lid_timer = None;
            lid_grace_over(state);
            TimeoutAction::Drop
        });
    match timer {
        Ok(t) => state.power.lid_timer = Some(t),
        Err(e) => log::warn!("power: lid timer: {}", e.error),
    }
}

fn lid_grace_over(state: &mut State) {
    if state.power.lid_closed && !external_in_layout(state) {
        log::info!("power: lid still closed, nothing external");
        lid_shut(state);
    }
}

/// The lid is closed with nothing external: `[power] lid_close` runs, or with a remote client
/// (`remote_held`) `held_lid`. Off the seat, nothing.
fn lid_shut(state: &mut State) {
    if !crate::backend::session_active(state) {
        log::info!("power: lid shut off the seat; nothing to do");
        return;
    }
    if remote_held(state) {
        log::info!("power: with a remote client, the panels go dark instead");
        held_lid(state);
        return;
    }
    let action = state.config.power.lid_close;
    run_action(state, action, Origin::Console);
}

/// A shut lid a remote client holds the action of: the panels go dark, and unless this
/// session's own client shows its desktop, a `lid_close` that would hide the desktop locks it.
fn held_lid(state: &mut State) {
    let hides = !matches!(
        state.config.power.lid_close,
        PowerAction::Nothing | PowerAction::ScreenOff
    );
    if hides && !state.remote_controlled() {
        crate::lockview::lock(state);
    }
    darken_panels(state);
}

fn darken_panels(state: &mut State) {
    let panels: Vec<(String, u8)> = (state.displays.list.iter())
        .filter(|d| d.internal && !state.power.darkened.iter().any(|(c, _)| *c == d.connector))
        .filter_map(|d| match d.level(Value::Brightness) {
            Level::At(level) => Some((d.connector.clone(), level)),
            Level::Unread | Level::Fixed => None,
        })
        .collect();
    for (connector, level) in panels {
        crate::displays::darken(state, &connector, Value::Brightness);
        state.power.darkened.push((connector, level));
    }
    save_record(state);
}

fn restore_panels(state: &mut State) {
    for (connector, level) in std::mem::take(&mut state.power.darkened) {
        crate::displays::set(state, &connector, Value::Brightness, level);
    }
    save_record(state);
}

/// A display reading came in: a panel the record left dark comes back to its level if it reads
/// 0, and leaves the record if it reads anything else.
pub fn displays_read(state: &mut State) {
    if state.power.left_dark.is_empty() {
        return;
    }
    let mut back = Vec::new();
    state.power.left_dark.retain(|(connector, level)| {
        let read = (state.displays.list.iter())
            .find(|d| d.connector == *connector)
            .map(|d| d.level(Value::Brightness));
        match read {
            Some(Level::At(0)) => back.push((connector.clone(), *level)),
            Some(Level::At(_) | Level::Fixed) => {}
            None | Some(Level::Unread) => return true,
        }
        false
    });
    for (connector, level) in back {
        log::info!("power: {connector} was left dark; back to {level}%");
        crate::displays::set(state, &connector, Value::Brightness, level);
    }
    save_record(state);
}

fn save_record(state: &State) {
    let Some(path) = &state.power.record else {
        return;
    };
    let panels: String = (state.power.darkened.iter())
        .chain(&state.power.left_dark)
        .map(|(connector, level)| format!("{connector} {level}\n"))
        .collect();
    let saved = if panels.is_empty() {
        std::fs::remove_file(path).or_else(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Ok(()),
            _ => Err(e),
        })
    } else {
        std::fs::write(path, panels)
    };
    if let Err(e) = saved {
        log::warn!("power: {}: {e}", path.display());
    }
}

fn read_record(path: &Path) -> Vec<(String, u8)> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    (text.lines())
        .filter_map(|l| {
            let (connector, level) = l.split_once(' ')?;
            Some((connector.to_string(), level.parse().ok()?))
        })
        .collect()
}

/// The lid's closing runs `[power] lid_close` (docked, the external output carries on); its
/// opening wakes the outputs and panels a remote-control session darkened. Tablet mode is ignored.
pub fn switch_toggled(state: &mut State, switch: Option<Switch>, st: SwitchState) {
    if switch != Some(Switch::Lid) {
        return;
    }
    if state.host == crate::compositor::Host::Greeter {
        return greeter_lid(state, st == SwitchState::On);
    }
    match st {
        SwitchState::On => {
            state.power.lid_closed = true;
            if external_in_layout(state) {
                log::info!("power: lid closed, docked");
                follow_lid(state);
                return;
            }
            // A lid closed at startup reports when its device is added, before any output.
            if !(state.scene.outputs().iter()).any(|o| !crate::backend::is_virtual(o)) {
                log::info!("power: lid closed before any monitor");
                arm_lid_check(state);
                return;
            }
            log::info!("power: lid closed");
            lid_shut(state);
        }
        SwitchState::Off => {
            log::info!("power: lid opened");
            state.power.lid_closed = false;
            restore_panels(state);
            if let Some(t) = state.power.lid_timer.take() {
                state.handle.remove(t);
            }
            follow_lid(state);
            crate::idle::notify_activity(state);
        }
    }
}

/// `State::remote_controlled` may have changed: idle re-arms and a start counts as activity;
/// under a shut lid the panels go dark as it starts and come back, with a lid check, as it ends.
pub fn remote_control_changed(state: &mut State) {
    let remote = state.remote_controlled();
    if std::mem::replace(&mut state.power.remote, remote) == remote {
        return;
    }
    log::info!(
        "power: remote control {}",
        if remote { "started" } else { "ended" }
    );
    if remote {
        crate::idle::notify_activity(state);
    }
    // notify_activity re-arms only when a step changed; the idle config changed regardless.
    crate::idle::arm(state);
    remote_lid(state);
}

/// The login screen runs no lid action: a shut lid darkens the panels while a remote client
/// works elsewhere, and logind acts on it otherwise.
fn greeter_lid(state: &mut State, closed: bool) {
    state.power.greeter_lid_closed = closed;
    match closed {
        true => remote_lid(state),
        false => restore_panels(state),
    }
}

/// A remote client came or went (`remote_held`): a shut lid's panels go dark as the first comes
/// and come back, with a lid check in a session, as the last goes. Off the seat, nothing.
fn remote_lid(state: &mut State) {
    let closed = match state.host {
        crate::compositor::Host::Greeter => state.power.greeter_lid_closed,
        _ => state.power.lid_closed,
    };
    let shut = closed && !external_in_layout(state) && crate::backend::session_active(state);
    if remote_held(state) {
        if shut {
            held_lid(state);
        }
    } else {
        restore_panels(state);
        if shut && state.host != crate::compositor::Host::Greeter {
            arm_lid_check(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn going_to_sleep_locks_without_a_delay_inhibitor_and_waits_only_with_one() {
        assert_eq!(sleep_plan(true, true), (true, true));
        assert_eq!(
            sleep_plan(true, false),
            (true, false),
            "no inhibitor: lock, nothing to hold"
        );
        assert_eq!(sleep_plan(false, true), (false, false));
        assert_eq!(sleep_plan(false, false), (false, false));
    }

    #[test]
    fn a_logind_lock_locks_both_sides() {
        let (mut h, _portal, _lock) = crate::console::tests::in_lock_mode();
        report(&mut h.state, Report::Lock);
        assert!(h.state.console_locked());
        assert!(h.state.desktop_locked(), "the remote side locks too");
    }

    #[test]
    fn a_logind_lock_away_from_the_seat_locks_both_sides() {
        let (mut h, _portal, _lock) = crate::console::tests::in_lock_mode();
        crate::lockview::seat_away(&mut h.state, true);
        report(&mut h.state, Report::Lock);
        assert!(h.state.console_locked());
        assert!(h.state.desktop_locked(), "the remote side locks too");
    }

    #[test]
    fn a_logind_unlock_unlocks_both_sides_and_ends_lock_mode() {
        let (mut h, _portal, _lock) = crate::console::tests::in_lock_mode();
        crate::lockview::lock_remote(&mut h.state);
        assert!(h.state.desktop_locked());
        report(&mut h.state, Report::Unlock);
        assert!(!h.state.console_locked(), "the console unlocks");
        assert!(!h.state.desktop_locked(), "the remote side unlocks");
        assert!(!crate::console::remote_side(&h.state), "lock mode ends");
        assert!(!crate::crash::locked());
    }

    #[test]
    fn a_logind_unlock_off_the_seat_without_reprise_login_still_unlocks() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.lock.helpers.finger = |_| None;
        crate::lockview::lock(&mut h.state);
        crate::lockview::seat_away(&mut h.state, true);
        assert!(h.state.login_link.is_none());
        report(&mut h.state, Report::Unlock);
        assert!(!h.state.console_locked());
    }

    #[test]
    fn built_in_panels_are_told_apart_by_connector() {
        assert!(is_internal_output("eDP-1"));
        assert!(is_internal_output("LVDS-1"));
        assert!(is_internal_output("DSI-1"));
        assert!(!is_internal_output("DP-2"));
        assert!(!is_internal_output("HDMI-A-1"));
        assert!(!is_internal_output("Virtual-1"));
    }

    #[test]
    fn a_hint_goes_out_only_when_it_changes() {
        let mut h = SessionHints::default();
        assert!(!h.changed(Hint::Locked(false)), "a new session is unlocked");
        assert!(!h.changed(Hint::Idle(false)), "a new session is active");
        assert!(h.changed(Hint::Idle(true)));
        assert!(!h.changed(Hint::Idle(true)), "still idle");
        assert!(h.changed(Hint::Locked(true)));
        assert!(!h.changed(Hint::Locked(true)), "still locked");
        assert!(h.changed(Hint::Idle(false)));
        assert!(!h.changed(Hint::Idle(false)), "activity while active");
        assert!(h.changed(Hint::Locked(false)));
    }

    #[test]
    fn a_closed_lid_hides_the_panel_only_when_docked() {
        assert!(internal_wanted(false, false));
        assert!(internal_wanted(false, true));
        assert!(internal_wanted(true, false), "the panel is all there is");
        assert!(!internal_wanted(true, true));
    }

    #[test]
    fn remote_elsewhere_reads_the_login_managers_sleep_inhibitor() {
        let held = |what: &str, who: &str, mode: &str, uid| {
            let why = "A Remote Desktop client is connected";
            remote_elsewhere(&[(what.into(), who.into(), why.into(), mode.into(), uid, 1)])
        };
        let who = "Reprise login manager";
        assert!(held("sleep:idle", who, "block", 0));
        assert!(!held("idle", who, "block", 0));
        assert!(!held("sleep", "reprise", "block", 0));
        assert!(!held("sleep:idle", who, "delay", 0));
        assert!(
            !held("sleep:idle", who, "block", 1000),
            "only root speaks for the login manager"
        );
        assert!(!remote_elsewhere(&[]));
    }

    mod logind {
        use crate::testbus::bus;
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::{Arc, Mutex};
        use std::time::{Duration, Instant};

        use super::super::{
            Call, Hint, LOGIN1_MANAGER, LOGIN1_NAME, LOGIN1_PATH, Report, session_proxy,
            watch_session, watch_sleep,
        };

        /// Refuses `Inhibit` while `refusals` lasts, then hands out /dev/null.
        struct FakeManager {
            refusals: Arc<AtomicU32>,
            asked: Arc<AtomicU32>,
        }

        #[zbus::interface(name = "org.freedesktop.login1.Manager")]
        impl FakeManager {
            fn inhibit(
                &self,
                _what: &str,
                _who: &str,
                _why: &str,
                _mode: &str,
            ) -> zbus::fdo::Result<zbus::OwnedFd> {
                self.asked.fetch_add(1, Ordering::SeqCst);
                if self.refusals.load(Ordering::SeqCst) > 0 {
                    self.refusals.fetch_sub(1, Ordering::SeqCst);
                    return Err(zbus::fdo::Error::Failed("not now".into()));
                }
                let null = std::fs::File::open("/dev/null").expect("/dev/null");
                Ok(std::os::fd::OwnedFd::from(null).into())
            }
        }

        #[test]
        fn a_refused_sleep_delay_is_asked_for_again() {
            let Some(bus) = bus() else {
                eprintln!("no dbus-daemon; skipped");
                return;
            };
            let asked = Arc::new(AtomicU32::new(0));
            let fake = FakeManager {
                refusals: Arc::new(AtomicU32::new(2)),
                asked: asked.clone(),
            };
            let (tx, rx) = smithay::reexports::calloop::channel::channel::<Report>();
            let address = bus.1.clone();
            std::thread::spawn(move || {
                zbus::block_on(async move {
                    let logind = zbus::connection::Builder::address(address.as_str())
                        .name(LOGIN1_NAME)
                        .serve_at(LOGIN1_PATH, fake)
                        .build()
                        .await
                        .expect("the fake logind");
                    let ours = zbus::connection::Builder::address(address.as_str())
                        .build()
                        .await
                        .expect("connect");
                    let login1 = zbus::Proxy::new(&ours, LOGIN1_NAME, LOGIN1_PATH, LOGIN1_MANAGER)
                        .await
                        .expect("proxy");
                    watch_sleep(login1, tx, Duration::from_millis(20)).await;
                    drop(logind);
                });
            });
            let end = Instant::now() + Duration::from_secs(5);
            loop {
                if let Ok(Report::SleepDelay(_)) = rx.try_recv() {
                    break;
                }
                assert!(
                    Instant::now() < end,
                    "no delay inhibitor after {} asks",
                    asked.load(Ordering::SeqCst)
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                asked.load(Ordering::SeqCst),
                3,
                "two refusals, then the delay"
            );
        }

        /// Refuses `SetLockedHint` while `refusals` lasts and keeps each value it took.
        struct FakeSession {
            refusals: Arc<AtomicU32>,
            asked: Arc<AtomicU32>,
            heard: Arc<Mutex<Vec<bool>>>,
        }

        #[zbus::interface(name = "org.freedesktop.login1.Session")]
        impl FakeSession {
            fn set_locked_hint(&self, locked: bool) -> zbus::fdo::Result<()> {
                self.asked.fetch_add(1, Ordering::SeqCst);
                if self.refusals.load(Ordering::SeqCst) > 0 {
                    self.refusals.fetch_sub(1, Ordering::SeqCst);
                    return Err(zbus::fdo::Error::Failed("not now".into()));
                }
                self.heard.lock().expect("heard").push(locked);
                Ok(())
            }
        }

        #[test]
        fn a_refused_locked_hint_is_sent_again() {
            let Some(bus) = bus() else {
                eprintln!("no dbus-daemon; skipped");
                return;
            };
            const SESSION: &str = "/org/freedesktop/login1/session/_31";
            let asked = Arc::new(AtomicU32::new(0));
            let heard = Arc::new(Mutex::new(Vec::new()));
            let fake = FakeSession {
                refusals: Arc::new(AtomicU32::new(2)),
                asked: asked.clone(),
                heard: heard.clone(),
            };
            let (calls_tx, calls) = async_channel::unbounded();
            let (tx, _rx) = smithay::reexports::calloop::channel::channel::<Report>();
            let address = bus.1.clone();
            std::thread::spawn(move || {
                zbus::block_on(async move {
                    let logind = zbus::connection::Builder::address(address.as_str())
                        .name(LOGIN1_NAME)
                        .serve_at(SESSION, fake)
                        .build()
                        .await
                        .expect("the fake logind");
                    let ours = zbus::connection::Builder::address(address.as_str())
                        .build()
                        .await
                        .expect("connect");
                    let path = zbus::OwnedObjectPath::try_from(SESSION).expect("path");
                    let session = session_proxy(&ours, path).await.expect("proxy");
                    watch_session(ours, session, calls, tx, Duration::from_millis(20)).await;
                    drop(logind);
                });
            });
            calls_tx
                .try_send(Call::Hint(Hint::Locked(true)))
                .expect("the loop's calls");
            let end = Instant::now() + Duration::from_secs(5);
            while heard.lock().expect("heard").is_empty() {
                assert!(
                    Instant::now() < end,
                    "no LockedHint taken after {} tries",
                    asked.load(Ordering::SeqCst)
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                asked.load(Ordering::SeqCst),
                3,
                "two refusals, then the hint"
            );
            assert_eq!(*heard.lock().expect("heard"), [true]);
        }
    }

    fn hints_heard(calls: &async_channel::Receiver<Call>) -> Vec<Hint> {
        std::iter::from_fn(|| calls.try_recv().ok())
            .filter_map(|c| match c {
                Call::Hint(h) => Some(h),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_restarted_logind_hears_both_hints_again() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let calls = capture_calls(&mut h.state);
        assert_eq!(
            hints_heard(&calls),
            [Hint::Locked(false), Hint::Idle(false)],
            "the loop's start sets both"
        );
        crate::lockview::lock(&mut h.state);
        assert_eq!(hints_heard(&calls), [Hint::Locked(true)]);
        report(&mut h.state, Report::LogindRestarted);
        assert_eq!(
            hints_heard(&calls),
            [Hint::Locked(true), Hint::Idle(false)],
            "the new logind hears the lock"
        );
    }

    mod remote {
        use std::os::fd::OwnedFd;
        use std::sync::mpsc::Receiver;

        use super::super::{
            Call, Report, calls_but_hints, capture_calls, lid_grace_over, read_record,
            remote_elsewhere_changed, report, switch_toggled,
        };
        use smithay::backend::input::{Switch, SwitchState};

        use crate::config::{Config, PowerAction};
        use crate::harness::Harness;
        use crate::media::{DisplayJob, DisplayReport, Job, Outcome, Reading, Value};

        /// One laptop panel at 60% brightness, a lid that locks, and the media worker's jobs.
        fn laptop() -> (Harness, Receiver<Job>) {
            laptop_with(60, |_| {})
        }

        /// [`laptop`] with the panel reading `level`, `setup` run before the output comes.
        fn laptop_with(level: u8, setup: impl FnOnce(&mut Harness)) -> (Harness, Receiver<Job>) {
            let mut config = Config::default();
            config.power.lid_close = PowerAction::Lock;
            config.power.standby = String::new();
            let mut h = Harness::with_config(config);
            setup(&mut h);
            let jobs = h.state.key_input.media.capture();
            h.output("eDP-1", 800, 600);
            let asked: Vec<Job> = jobs.try_iter().collect();
            for job in asked {
                if let Job::Display(DisplayJob::Read {
                    display,
                    value,
                    seq,
                }) = job
                {
                    let outcome = Outcome::Read(Some(Reading { level, floor: 1 }));
                    let r = DisplayReport {
                        display,
                        value,
                        seq,
                        outcome,
                    };
                    crate::displays::report(&mut h.state, r);
                }
            }
            (h, jobs)
        }

        /// The levels set on eDP-1's brightness since the last call.
        fn panel_sets(jobs: &Receiver<Job>) -> Vec<u8> {
            jobs.try_iter()
                .filter_map(|j| match j {
                    Job::Display(DisplayJob::Set {
                        display,
                        value: Value::Brightness,
                        level,
                        ..
                    }) if display == "eDP-1" => Some(level),
                    _ => None,
                })
                .collect()
        }

        #[test]
        fn the_lid_dims_instead_of_sleeping_under_remote_control() {
            let (mut h, jobs) = laptop();
            let _peer = h.ei_remote_session();
            assert!(h.state.remote_controlled());
            switch_toggled(&mut h.state, Some(Switch::Lid), SwitchState::On);
            assert!(!h.state.console_locked(), "lid_close does not run");
            assert_eq!(panel_sets(&jobs), [0], "the panel goes dark");
            switch_toggled(&mut h.state, Some(Switch::Lid), SwitchState::Off);
            assert_eq!(panel_sets(&jobs), [60], "opening the lid brings back 60%");
        }

        #[test]
        fn ending_remote_control_with_the_lid_shut_runs_the_lid_action() {
            let (mut h, jobs) = laptop();
            let peer = h.ei_remote_session();
            switch_toggled(&mut h.state, Some(Switch::Lid), SwitchState::On);
            assert_eq!(panel_sets(&jobs), [0]);
            h.ei_close(peer);
            assert!(!h.state.remote_controlled());
            assert_eq!(panel_sets(&jobs), [60], "the panel comes back at 60%");
            assert!(
                !h.state.console_locked(),
                "the lid action waits out the grace"
            );
            assert!(h.state.power.lid_timer.is_some(), "the lid check is armed");
            lid_grace_over(&mut h.state);
            assert!(h.state.console_locked(), "lid_close ran");
        }

        #[test]
        fn unplugging_the_monitor_under_a_shut_lid_with_remote_desktop_on_brings_the_panel_back() {
            let mut config = Config::default();
            config.power.lid_close = PowerAction::Lock;
            let mut h = Harness::with_config(config);
            h.head("eDP-1", 800, 600);
            let dp = h.head("DP-1", 1920, 1080);
            let names = |h: &Harness| -> Vec<String> {
                h.state.scene.outputs().iter().map(|o| o.name()).collect()
            };
            switch_toggled(&mut h.state, Some(Switch::Lid), SwitchState::On);
            h.dispatch();
            assert_eq!(
                names(&h),
                ["DP-1"],
                "docked, the shut panel leaves the layout"
            );
            crate::standby::rdp_changed(&mut h.state, true);
            h.dispatch();
            h.state.test_heads.retain(|o| *o != dp);
            h.state.scene.remove_output(&dp);
            h.state.output_removed(&dp);
            h.dispatch();
            assert_eq!(
                names(&h),
                ["eDP-1"],
                "the panel comes back and no standby stays"
            );
            assert!(h.state.power.lid_timer.is_some(), "the lid check is armed");
            lid_grace_over(&mut h.state);
            assert!(h.state.console_locked(), "lid_close ran");
        }

        #[test]
        fn a_shut_lid_with_a_remote_client_elsewhere_locks_and_darkens_the_panel() {
            let (mut h, jobs) = laptop();
            crate::power::remote_elsewhere_changed(&mut h.state, true);
            switch_toggled(&mut h.state, Some(Switch::Lid), SwitchState::On);
            assert!(h.state.console_locked(), "the session locks");
            assert_eq!(panel_sets(&jobs), [0], "the panel goes dark");
            crate::power::remote_elsewhere_changed(&mut h.state, false);
            assert_eq!(panel_sets(&jobs), [60], "the panel comes back at 60%");
            assert!(h.state.power.lid_timer.is_some(), "the lid check is armed");
            lid_grace_over(&mut h.state);
            assert!(h.state.console_locked(), "lid_close ran");
        }

        #[test]
        fn a_remote_client_elsewhere_locks_the_session_for_every_lid_action_that_hides_it() {
            for (action, locks) in [
                (PowerAction::Lock, true),
                (PowerAction::Standby, true),
                (PowerAction::Hibernate, true),
                (PowerAction::TurnOff, true),
                (PowerAction::Ask, true),
                (PowerAction::Nothing, false),
                (PowerAction::ScreenOff, false),
            ] {
                let (mut h, jobs) = laptop_with(60, |h| h.state.config.power.lid_close = action);
                remote_elsewhere_changed(&mut h.state, true);
                switch_toggled(&mut h.state, Some(Switch::Lid), SwitchState::On);
                assert_eq!(h.state.console_locked(), locks, "{action:?}");
                assert_eq!(panel_sets(&jobs), [0], "{action:?}: the panel goes dark");
                assert!(
                    h.state.ending.is_none(),
                    "{action:?}: the computer stays on for the client"
                );
            }
        }

        #[test]
        fn a_shut_lid_locks_when_this_sessions_client_leaves_while_one_works_elsewhere() {
            let (mut h, jobs) = laptop();
            remote_elsewhere_changed(&mut h.state, true);
            let peer = h.ei_remote_session();
            switch_toggled(&mut h.state, Some(Switch::Lid), SwitchState::On);
            assert!(
                !h.state.console_locked(),
                "its own client shows the desktop"
            );
            assert_eq!(panel_sets(&jobs), [0]);
            h.ei_close(peer);
            assert!(h.state.console_locked(), "the session locks");
            assert_eq!(panel_sets(&jobs), Vec::<u8>::new(), "the panel stays dark");
        }

        fn null_fd() -> OwnedFd {
            std::fs::File::open("/dev/null").expect("/dev/null").into()
        }

        #[test]
        fn the_login_screen_holds_the_lid_only_while_a_remote_client_works_elsewhere() {
            let (mut h, _jobs) = laptop();
            h.state.host = crate::compositor::Host::Greeter;
            let calls = capture_calls(&mut h.state);
            remote_elsewhere_changed(&mut h.state, true);
            assert_eq!(calls_but_hints(&calls), [Call::HoldLid]);
            report(&mut h.state, Report::LidHold(null_fd()));
            assert!(h.state.power.lid_hold.is_some(), "the lid switch is held");
            remote_elsewhere_changed(&mut h.state, false);
            assert!(
                h.state.power.lid_hold.is_none(),
                "logind's own lid action applies again"
            );
            report(&mut h.state, Report::LidHold(null_fd()));
            assert!(
                h.state.power.lid_hold.is_none(),
                "a hold taken after the client left is dropped"
            );
            assert!(calls_but_hints(&calls).is_empty());
        }

        #[test]
        fn a_session_takes_no_lid_hold_of_its_own_for_a_remote_client_elsewhere() {
            let (mut h, _jobs) = laptop();
            let calls = capture_calls(&mut h.state);
            remote_elsewhere_changed(&mut h.state, true);
            assert!(
                calls_but_hints(&calls).is_empty(),
                "its startup inhibitor holds the lid"
            );
        }

        #[test]
        fn a_shut_lid_at_the_login_screen_darkens_the_panel_only_with_a_remote_client_elsewhere() {
            let (mut h, jobs) = laptop();
            h.state.host = crate::compositor::Host::Greeter;
            switch_toggled(&mut h.state, Some(Switch::Lid), SwitchState::On);
            assert_eq!(
                panel_sets(&jobs),
                Vec::<u8>::new(),
                "logind acts on the lid"
            );
            assert!(h.state.power.lid_timer.is_none());
            switch_toggled(&mut h.state, Some(Switch::Lid), SwitchState::Off);

            remote_elsewhere_changed(&mut h.state, true);
            switch_toggled(&mut h.state, Some(Switch::Lid), SwitchState::On);
            assert_eq!(panel_sets(&jobs), [0], "the panel goes dark");
            switch_toggled(&mut h.state, Some(Switch::Lid), SwitchState::Off);
            assert_eq!(panel_sets(&jobs), [60], "opening the lid brings back 60%");

            switch_toggled(&mut h.state, Some(Switch::Lid), SwitchState::On);
            assert_eq!(panel_sets(&jobs), [0]);
            remote_elsewhere_changed(&mut h.state, false);
            assert_eq!(
                panel_sets(&jobs),
                [60],
                "the panel comes back for logind's action"
            );
            assert!(
                h.state.power.lid_timer.is_none(),
                "the login screen runs no lid action"
            );
            assert!(!h.state.console_locked());
        }

        #[test]
        fn a_shut_lid_at_the_login_screen_leaves_the_layout_and_arms_no_lid_check() {
            let mut h = Harness::with_config(Config::default());
            h.state.host = crate::compositor::Host::Greeter;
            h.head("eDP-1", 800, 600);
            let dp = h.head("DP-1", 1920, 1080);
            let names = |h: &Harness| -> Vec<String> {
                h.state.scene.outputs().iter().map(|o| o.name()).collect()
            };
            switch_toggled(&mut h.state, Some(Switch::Lid), SwitchState::On);
            crate::power::outputs_changed(&mut h.state);
            h.dispatch();
            assert_eq!(names(&h), ["eDP-1", "DP-1"], "docked, the panel stays");
            h.state.test_heads.retain(|o| *o != dp);
            h.state.scene.remove_output(&dp);
            h.state.output_removed(&dp);
            h.dispatch();
            assert_eq!(names(&h), ["eDP-1"]);
            assert!(
                h.state.power.lid_timer.is_none(),
                "no lid check after undocking"
            );
        }

        #[test]
        fn an_inactive_session_runs_no_lid_action() {
            let (mut h, jobs) = laptop();
            crate::backend::pause_session(&mut h.state);
            switch_toggled(&mut h.state, Some(Switch::Lid), SwitchState::On);
            lid_grace_over(&mut h.state);
            assert!(!h.state.console_locked(), "lid_close does not run");
            let _peer = h.ei_remote_session();
            crate::power::remote_elsewhere_changed(&mut h.state, true);
            assert_eq!(
                panel_sets(&jobs),
                Vec::<u8>::new(),
                "the panel is left alone"
            );
        }

        fn record_in_scratch(name: &str) -> std::path::PathBuf {
            let path = crate::harness::scratch().join(name);
            let _ = std::fs::remove_file(&path);
            path
        }

        #[test]
        fn darkened_panels_are_recorded_until_they_come_back() {
            let record = record_in_scratch("darkened-recorded");
            let (mut h, _jobs) = laptop();
            h.state.power.record = Some(record.clone());
            let _peer = h.ei_remote_session();
            switch_toggled(&mut h.state, Some(Switch::Lid), SwitchState::On);
            assert_eq!(read_record(&record), [("eDP-1".to_string(), 60)]);
            switch_toggled(&mut h.state, Some(Switch::Lid), SwitchState::Off);
            assert!(!record.exists(), "the record goes with the panels back");
        }

        #[test]
        fn a_panel_left_dark_by_an_ended_compositor_comes_back_to_the_recorded_level() {
            let record = record_in_scratch("darkened-left");
            std::fs::write(&record, "eDP-1 60\n").expect("write the record");
            let (_h, jobs) = laptop_with(0, |h| {
                h.state.power.left_dark = read_record(&record);
                h.state.power.record = Some(record.clone());
            });
            assert_eq!(
                panel_sets(&jobs),
                [60],
                "eDP-1 reads 0 and goes back to 60%"
            );
            assert!(!record.exists(), "the record goes once restored");
        }

        #[test]
        fn a_recorded_panel_that_reads_lit_is_left_alone() {
            let record = record_in_scratch("darkened-lit");
            std::fs::write(&record, "eDP-1 60\n").expect("write the record");
            let (_h, jobs) = laptop_with(30, |h| {
                h.state.power.left_dark = read_record(&record);
                h.state.power.record = Some(record.clone());
            });
            assert_eq!(panel_sets(&jobs), Vec::<u8>::new(), "30% stays");
            assert!(!record.exists(), "the record is dropped");
        }
    }
}
