//! The laptop side of the compositor: the power source (UPower's
//! `OnBattery`, which picks between `[idle]` and `[idle.battery]`), the lid
//! switch, the power and sleep buttons, and the actions they run.
//!
//! logind would act on the lid and buttons itself, so at startup the module
//! takes its `block` inhibitor for them and keeps the fd for the process's
//! life; the config's actions decide. A `delay` inhibitor on sleep lets
//! `PrepareForSleep` lock the screen and release it once every output that
//! is on has presented the cover, so a suspend resumes to the lock.
//!
//! The compositor's logind session object carries the lock both ways: its
//! `Lock` and `Unlock` signals (`loginctl lock-session`, `unlock-session`)
//! lock and unlock, and `LockedHint` and `IdleHint` follow the lock and idle timer.
//!
//! Under remote control (`State::remote_controlled`) a shut lid darkens the
//! internal panels instead of running `[power] lid_close`.

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
use crate::media::Value;
use crate::sessionlock::CoverWait;

const UPOWER_NAME: &str = "org.freedesktop.UPower";
const UPOWER_PATH: &str = "/org/freedesktop/UPower";
const LOGIN1_NAME: &str = "org.freedesktop.login1";
const LOGIN1_PATH: &str = "/org/freedesktop/login1";
const LOGIN1_MANAGER: &str = "org.freedesktop.login1.Manager";
const LOGIN1_SESSION: &str = "org.freedesktop.login1.Session";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// Well under logind's `InhibitDelayMaxSec` (5 s by default).
const COVER_WAIT: Duration = Duration::from_millis(2000);
/// How long a closed lid with no external output waits before its action
/// when closing didn't cause that state (the lid at startup, reported before
/// any output exists, or the last external output leaving). Gives late
/// outputs (a dock's, a display that drops out briefly) time to show up.
/// logind's `HoldoffTimeoutSec` default.
const LID_GRACE: Duration = Duration::from_secs(30);

/// Under `$XDG_RUNTIME_DIR`: the darkened panels, one `connector level` per line.
const DARKENED_RECORD: &str = "reprise-darkened";

/// None only if a component does not parse as a bus name or path.
fn signal_rule(
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

enum Report {
    OnBattery(bool),
    /// Kept until shutdown so logind leaves the lid and buttons to us.
    Inhibitor(OwnedFd),
    /// Taken at startup and after every resume; dropped once the lock cover is on every screen.
    SleepDelay(OwnedFd),
    /// `PrepareForSleep(true)`: the machine sleeps once every delay inhibitor is released.
    Sleeping,
    Resumed,
    Lock,
    Unlock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    Locked(bool),
    Idle(bool),
}

/// The hints logind last heard, so each goes out only on change. Both start false, as for a new session.
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
    hint_tx: Option<async_channel::Sender<Hint>>,
    pub lid_closed: bool,
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
}

/// Runs on the D-Bus thread until the bus goes away.
async fn run(reports: Sender<Report>, hints: async_channel::Receiver<Hint>) {
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
            Ok(path) => match Proxy::new(&conn, LOGIN1_NAME, path, LOGIN1_SESSION).await {
                Ok(session) => {
                    let watch = watch_session(conn.clone(), session, hints, reports.clone());
                    std::thread::Builder::new()
                        .name("power-session".into())
                        .spawn(move || zbus::block_on(watch))
                        .ok();
                }
                Err(e) => log::info!("power: no session proxy ({e})"),
            },
            Err(e) => log::info!("power: no logind session ({e}); no lock-session, no hints"),
        }
        let sleep = watch_sleep(login1, reports.clone());
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

/// Only logind's own broadcasts get through (see `reprisebus::signals`), so a
/// spoofed `Unlock` cannot end the lock.
async fn watch_session(
    conn: Connection,
    session: Proxy<'static>,
    hints: async_channel::Receiver<Hint>,
    reports: Sender<Report>,
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
            let reprisebus::Signal::Message(m) = sig else {
                continue;
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
        while let Ok(h) = hints.recv().await {
            let (method, v) = match h {
                Hint::Locked(v) => ("SetLockedHint", v),
                Hint::Idle(v) => ("SetIdleHint", v),
            };
            if let Err(e) = session.call_method(method, &(v,)).await {
                log::warn!("power: {method}({v}) failed: {e}");
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

async fn take_sleep_delay(login1: &Proxy<'_>, reports: &Sender<Report>) {
    match sleep_delay(login1).await {
        Ok(fd) => {
            let _ = reports.send(Report::SleepDelay(fd));
        }
        Err(e) => log::info!("power: no sleep delay inhibitor ({e}); no lock on sleep"),
    }
}

/// `true` hands the loop the moment to lock (it drops the fd once the cover
/// is on screen, letting the sleep go on); `false` is the resume, when the
/// inhibitor is taken again.
async fn watch_sleep(login1: Proxy<'static>, reports: Sender<Report>) {
    use futures_util::stream::StreamExt;
    // Only logind's own broadcast is acted on (see `reprisebus::signals`), so a
    // spoofed `PrepareForSleep` cannot lock the screen or move the inhibitor.
    let Some(rule) = signal_rule(LOGIN1_NAME, LOGIN1_PATH, LOGIN1_MANAGER, "PrepareForSleep")
    else {
        log::info!("power: no PrepareForSleep signal; no lock on sleep");
        return;
    };
    let mut signals = reprisebus::signals(login1.connection(), LOGIN1_NAME, vec![rule]).await;
    take_sleep_delay(&login1, &reports).await;
    while let Some(sig) = signals.next().await {
        let reprisebus::Signal::Message(msg) = sig else {
            continue;
        };
        let Ok(starting) = msg.body().deserialize::<bool>() else {
            continue;
        };
        if starting {
            let _ = reports.send(Report::Sleeping);
        } else {
            let _ = reports.send(Report::Resumed);
            take_sleep_delay(&login1, &reports).await;
        }
    }
}

fn report(state: &mut State, r: Report) {
    match r {
        Report::Inhibitor(fd) => state.power._inhibitor = Some(fd),
        Report::SleepDelay(fd) => {
            stop_cover_wait(state);
            state.power.sleep_delay = Some(fd);
        }
        Report::Sleeping => sleeping(state),
        Report::Resumed => {
            crate::idle::resumed(state);
            crate::nightlight::update(state);
            crate::displays::reread(state);
        }
        Report::Lock => {
            log::info!("power: logind asks for the lock");
            crate::lockview::lock(state);
        }
        Report::Unlock => {
            log::info!("power: logind ends the lock");
            crate::lockview::unlock(state);
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
        crate::lockview::lock(state);
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
        && let Some(tx) = &p.hint_tx
    {
        let _ = tx.try_send(h);
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
    let (hint_tx, hint_rx) = async_channel::unbounded::<Hint>();
    let thread = std::thread::Builder::new()
        .name("power-dbus".into())
        .spawn(move || zbus::block_on(run(tx, hint_rx)));
    if thread.is_ok() {
        state.power.hint_tx = Some(hint_tx);
    }
}

pub fn run_action(state: &mut State, a: PowerAction) {
    log::info!("power: {}", a.name());
    match a {
        PowerAction::Nothing => {}
        PowerAction::ScreenOff => crate::idle::screen_off(state),
        PowerAction::Lock => crate::lockview::lock(state),
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
            );
        }
        PowerAction::Ask => {
            // Dialogs live under the lock cover, where no key reaches them: a locked
            // session has nothing to ask.
            if state.is_locked() {
                log::info!("power: ask ignored while locked");
                return;
            }
            let d = crate::dialog::Dialog::turn_off(&state.config.power);
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

fn external_in_layout(state: &State) -> bool {
    state
        .scene
        .outputs()
        .iter()
        .any(|o| !is_internal_output(&o.name()) && state.layout_box(Some(o)).width > 0)
}

/// Clamshell mode: take the built-in panels out of the layout while the lid
/// is closed and an external output shows the desktop, and put them back when
/// the lid opens or the last external output goes. A panel returning under a
/// closed lid arms the lid check. Runs on every lid toggle and layout change.
pub fn follow_lid(state: &mut State) {
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

/// The lid is closed with nothing external: `[power] lid_close` runs, or under remote control
/// the internal panels go dark instead.
fn lid_shut(state: &mut State) {
    if state.remote_controlled() {
        log::info!("power: under remote control, the panels go dark instead");
        darken_panels(state);
        return;
    }
    let action = state.config.power.lid_close;
    run_action(state, action);
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

/// The lid's closing runs `[power] lid_close` (or, docked, leaves the
/// external output to carry on); its opening wakes the outputs and brings
/// back panels a remote-control session darkened. Other
/// switches (tablet mode) are ignored. An already closed lid reports when
/// its device is added, at startup before any output exists.
pub fn switch_toggled(state: &mut State, switch: Option<Switch>, st: SwitchState) {
    if switch != Some(Switch::Lid) {
        return;
    }
    match st {
        SwitchState::On => {
            state.power.lid_closed = true;
            if external_in_layout(state) {
                log::info!("power: lid closed, docked");
                follow_lid(state);
                return;
            }
            if state.scene.outputs().is_empty() {
                log::info!("power: lid closed before any output");
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
    let shut = state.power.lid_closed && !external_in_layout(state);
    if remote {
        if shut {
            darken_panels(state);
        }
    } else {
        restore_panels(state);
        if shut {
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

    mod remote {
        use std::sync::mpsc::Receiver;

        use super::super::{lid_grace_over, read_record, switch_toggled};
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
            assert!(!h.state.is_locked(), "lid_close does not run");
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
            assert!(!h.state.is_locked(), "the lid action waits out the grace");
            assert!(h.state.power.lid_timer.is_some(), "the lid check is armed");
            lid_grace_over(&mut h.state);
            assert!(h.state.is_locked(), "lid_close ran");
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
