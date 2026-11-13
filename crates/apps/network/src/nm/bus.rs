//! The NetworkManager side: the connection, subscriptions, snapshot building and the commands the
//! windows send back.

use super::*;
use futures_util::stream::StreamExt;
use std::sync::mpsc::{Receiver, Sender, channel};
use zbus::Connection;

mod act;
mod call;
mod snapshot;
mod wifi;
use act::command;
use call::{Failure, cmd_device, failure_title};
use snapshot::{build, saved};

const NAME: &str = "org.freedesktop.NetworkManager";
const MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";
const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const SETTINGS_IFACE: &str = "org.freedesktop.NetworkManager.Settings";
const CONNECTION_IFACE: &str = "org.freedesktop.NetworkManager.Settings.Connection";
const ACTIVE_IFACE: &str = "org.freedesktop.NetworkManager.Connection.Active";
const DEVICE_IFACE: &str = "org.freedesktop.NetworkManager.Device";
const WIRED_IFACE: &str = "org.freedesktop.NetworkManager.Device.Wired";
const WIRELESS_IFACE: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const STATS_IFACE: &str = "org.freedesktop.NetworkManager.Device.Statistics";
const AP_IFACE: &str = "org.freedesktop.NetworkManager.AccessPoint";
const IP4_IFACE: &str = "org.freedesktop.NetworkManager.IP4Config";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const MODIFY_SYSTEM: &str = "org.freedesktop.NetworkManager.settings.modify.system";
const LOGIND: &str = "org.freedesktop.login1";

const STATS_MS: u32 = 2000;

/// How long [`NmHandle::quit`] waits for the thread to flush, so a wedged daemon cannot hold up
/// exit.
const QUIT_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// How long one call waits for NetworkManager; long enough to type an admin password at the
/// authentication dialog.
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

#[derive(Debug)]
pub struct NmHandle {
    cmds: async_channel::Sender<Cmd>,
    done: Receiver<()>,
}

impl NmHandle {
    pub fn send(&self, cmd: Cmd) {
        if let Err(e) = self.cmds.send_blocking(cmd) {
            log::warn!("the NetworkManager thread is gone: {e}");
        }
    }

    pub fn quit(&self) {
        if self.cmds.send_blocking(Cmd::Quit).is_err() {
            return;
        }
        if let Err(e) = self.done.recv_timeout(QUIT_WAIT) {
            log::warn!("the NetworkManager thread did not answer ({e}); exiting anyway");
        }
    }
}

pub fn watch(waker: repriseapp::Waker) -> (Receiver<NmMsg>, NmHandle) {
    let (tx, rx) = channel();
    let (done_tx, done_rx) = channel();
    let (cmd_tx, cmd_rx) = async_channel::unbounded::<Cmd>();
    if let Err(e) = std::thread::Builder::new()
        .name("reprise-network-nm".into())
        .spawn(move || zbus::block_on(run(tx, waker, cmd_rx, done_tx)))
    {
        log::warn!("could not start the NetworkManager thread: {e}");
        cmd_tx.close();
    }
    (
        rx,
        NmHandle {
            cmds: cmd_tx,
            done: done_rx,
        },
    )
}

#[derive(Clone, Debug, PartialEq)]
enum Wake {
    Bus,
    Settings,
    Stats {
        path: String,
        rx: Option<u64>,
        tx: Option<u64>,
    },
    Strength {
        path: String,
        strength: Option<u8>,
    },
    Ran {
        device: Option<String>,
        error: Option<String>,
        reply: Option<Box<NmMsg>>,
    },
    Quit,
}

/// The wake a `PropertiesChanged` on `path` is worth: counters-only statistics and strength-only
/// access point updates are patched in place; a bare `LastScan` stamp is none; anything else
/// rebuilds.
fn wake_for_change(path: &str, iface: &str, changed: &Props) -> Option<Wake> {
    if iface == WIRELESS_IFACE && !changed.is_empty() && changed.keys().all(|k| k == "LastScan") {
        return None;
    }
    if iface == STATS_IFACE
        && !changed.is_empty()
        && changed.keys().all(|k| k == "RxBytes" || k == "TxBytes")
    {
        let counter = |k: &str| changed.get(k).and_then(|v| u64::try_from(v.clone()).ok());
        return Some(Wake::Stats {
            path: path.to_string(),
            rx: counter("RxBytes"),
            tx: counter("TxBytes"),
        });
    }
    if iface == AP_IFACE
        && !changed.is_empty()
        && changed.keys().all(|k| k == "Strength" || k == "LastSeen")
    {
        return Some(Wake::Strength {
            path: path.to_string(),
            strength: changed
                .get("Strength")
                .and_then(|v| u8::try_from(v.clone()).ok()),
        });
    }
    Some(Wake::Bus)
}

/// Patch an access point's new strength into the snapshot; false when no device lists it (snapshot
/// behind, wants a read).
fn patch_strength(snapshot: &mut Snapshot, path: &str, strength: Option<u8>) -> bool {
    let Some(wifi) = snapshot
        .devices
        .iter_mut()
        .filter_map(|d| d.wifi.as_mut())
        .find(|w| w.aps.iter().any(|a| a.path == path))
    else {
        return false;
    };
    let Some(strength) = strength else {
        return true;
    };
    for ap in wifi.aps.iter_mut().filter(|a| a.path == path) {
        ap.strength = strength;
        if ap.active {
            wifi.strength = strength;
        }
    }
    sort_aps(&mut wifi.aps);
    true
}

/// Patch a statistics tick into the snapshot (counters the signal omits keep their value); false
/// when no device has that path.
fn patch_stats(snapshot: &mut Snapshot, path: &str, rx: Option<u64>, tx: Option<u64>) -> bool {
    let Some(d) = snapshot.devices.iter_mut().find(|d| d.path == path) else {
        return false;
    };
    if let Some(rx) = rx {
        d.rx_bytes = rx;
    }
    if let Some(tx) = tx {
        d.tx_bytes = tx;
    }
    true
}

async fn run(
    tx: Sender<NmMsg>,
    waker: repriseapp::Waker,
    cmds: async_channel::Receiver<Cmd>,
    done: Sender<()>,
) {
    let send = |m: NmMsg| {
        if tx.send(m).is_ok() {
            waker.wake();
        }
    };
    let conn = zbus::connection::Builder::system()
        .method_timeout(CALL_TIMEOUT)
        .build()
        .await;
    let conn = match conn {
        Ok(c) => c,
        Err(e) => {
            log::info!("no system bus ({e}); no network to show");
            send(NmMsg::Snapshot(Snapshot::default()));
            while let Ok(cmd) = cmds.recv().await {
                if cmd == Cmd::Quit {
                    let _ = done.send(());
                    return;
                }
                log::warn!("no system bus; dropping {cmd:?}");
            }
            return;
        }
    };
    let mut snapshot = Snapshot::default();
    // Saved connections are read once and kept until a settings signal says otherwise.
    let mut saved_cache: Option<Vec<(String, HashMap<String, Props>)>> = None;
    let (wake_tx, wake_rx) = async_channel::unbounded::<Wake>();
    let _pump = pump(&conn, wake_tx.clone()).await;
    follow_switches(wake_tx.clone());
    let _commands = commands(&conn, cmds, wake_tx);
    let mut rebuild = true;
    loop {
        if rebuild {
            let saved = match &saved_cache {
                Some(s) => s,
                None => saved_cache.insert(saved(&conn).await),
            };
            snapshot = build(&conn, &snapshot, saved).await;
        }
        send(NmMsg::Snapshot(snapshot.clone()));
        let Ok(first) = wake_rx.recv().await else {
            log::warn!("nothing left to watch; the network store stops here");
            let _ = done.send(());
            return;
        };
        // A wireless scan fires a signal per access point; drain what is pending so ten cost one
        // rebuild.
        let mut wakes = vec![first];
        while let Ok(w) = wake_rx.try_recv() {
            wakes.push(w);
        }
        rebuild = false;
        for wake in wakes {
            match wake {
                Wake::Bus => rebuild = true,
                Wake::Settings => {
                    saved_cache = None;
                    rebuild = true;
                }
                Wake::Stats { path, rx, tx } => {
                    if !patch_stats(&mut snapshot, &path, rx, tx) {
                        rebuild = true;
                    }
                }
                Wake::Strength { path, strength } => {
                    if !patch_strength(&mut snapshot, &path, strength) {
                        rebuild = true;
                    }
                }
                Wake::Ran {
                    device,
                    error,
                    reply,
                } => {
                    rebuild = true;
                    if let Some(msg) = reply {
                        send(*msg);
                    }
                    if let Some(body) = error {
                        send(NmMsg::Failed {
                            title: failure_title(device.as_deref(), &snapshot),
                            body,
                        });
                    }
                }
                Wake::Quit => {
                    let _ = done.send(());
                    return;
                }
            }
        }
    }
}

/// Rebuild on every rfkill change: Bluetooth's switch counts towards airplane mode and
/// NetworkManager does not report it.
fn follow_switches(wakes: async_channel::Sender<Wake>) {
    let spawned = std::thread::Builder::new()
        .name("reprise-network-rfkill".into())
        .spawn(move || {
            if let Err(e) = reprisebus::rfkill::follow(|| wakes.send_blocking(Wake::Bus).is_ok()) {
                log::info!("radio switches are not followed: {e}");
            }
        });
    if let Err(e) = spawned {
        log::warn!("could not start the rfkill thread: {e}");
    }
}

/// Run the windows' commands on a task beside the connection, each answered by a [`Wake::Ran`], so the loop
/// keeps reading while a call waits on the authentication dialog. The returned task must be kept
/// alive.
fn commands(
    conn: &Connection,
    cmds: async_channel::Receiver<Cmd>,
    wakes: async_channel::Sender<Wake>,
) -> zruntime::Task<(), zruntime::Shared> {
    let bus = conn.clone();
    zruntime::spawn(async move {
        queue(cmds, wakes, |cmd| {
            let bus = bus.clone();
            async move { command(&bus, &cmd).await }
        })
        .await
    })
}

/// [`commands`] without the bus. `Stats` commands take their own lane so a Status sheet opened
/// during a pending join does not show frozen counters; each lane keeps its order.
async fn queue<F, Fut>(
    cmds: async_channel::Receiver<Cmd>,
    wakes: async_channel::Sender<Wake>,
    run: F,
) where
    F: Fn(Cmd) -> Fut,
    Fut: std::future::Future<Output = Result<Option<NmMsg>, Failure>>,
{
    let (main_tx, main_rx) = async_channel::unbounded::<Cmd>();
    let (stats_tx, stats_rx) = async_channel::unbounded::<Cmd>();
    let (stats_done_tx, stats_done_rx) = async_channel::bounded::<()>(1);
    let route = async move {
        while let Ok(cmd) = cmds.recv().await {
            let quit = cmd == Cmd::Quit;
            let lane = match cmd {
                Cmd::Stats { .. } => &stats_tx,
                _ => &main_tx,
            };
            if lane.send(cmd).await.is_err() || quit {
                break;
            }
        }
    };
    let stats = async {
        while let Ok(cmd) = stats_rx.recv().await {
            if wakes.send(ran(&cmd, run(cmd.clone()).await)).await.is_err() {
                break;
            }
        }
        drop(stats_done_tx);
    };
    let main = async {
        while let Ok(cmd) = main_rx.recv().await {
            let wake = if cmd == Cmd::Quit {
                let _ = stats_done_rx.recv().await;
                Wake::Quit
            } else {
                ran(&cmd, run(cmd.clone()).await)
            };
            if wakes.send(wake).await.is_err() {
                return;
            }
        }
    };
    futures_util::future::join3(route, stats, main).await;
}

fn ran(cmd: &Cmd, result: Result<Option<NmMsg>, Failure>) -> Wake {
    let (reply, error) = match result {
        Ok(reply) => (reply.map(Box::new), None),
        Err(Failure::Dismissed) => {
            log::info!("{cmd:?}: the password prompt was dismissed");
            (None, None)
        }
        Err(Failure::Text(e)) if matches!(cmd, Cmd::Stats { .. }) => {
            log::info!("{cmd:?} failed: {e}");
            (None, None)
        }
        Err(Failure::Text(e)) => {
            log::warn!("{cmd:?} failed: {e}");
            (None, Some(e))
        }
    };
    Wake::Ran {
        device: cmd_device(cmd).map(str::to_string),
        error,
        reply,
    }
}

/// Drain the signal streams into the loop's unbounded channel: a full 64-slot `MessageStream` would
/// block zbus's socket reader and the replies [`build`] awaits. The returned task must be kept
/// alive.
async fn pump(
    conn: &Connection,
    wakes: async_channel::Sender<Wake>,
) -> zruntime::Task<(), zruntime::Shared> {
    let mut streams = subscribe(conn).await;
    zruntime::spawn(async move {
        while let Some(wake) = streams.next().await {
            if wakes.send(wake).await.is_err() {
                return;
            }
        }
        log::warn!("the network store's signal streams all ended");
    })
}

/// Signal names worth a wake anywhere under NetworkManager's tree; matched by member alone since
/// `StateChanged` and `PropertiesChanged` ride on many objects.
const SIGNALS: &[(&str, Wake)] = &[
    ("PropertiesChanged", Wake::Bus),
    ("StateChanged", Wake::Bus),
    ("DeviceAdded", Wake::Bus),
    ("DeviceRemoved", Wake::Bus),
    ("AccessPointAdded", Wake::Bus),
    ("AccessPointRemoved", Wake::Bus),
    ("NewConnection", Wake::Settings),
    ("ConnectionRemoved", Wake::Settings),
    ("Updated", Wake::Settings),
];

/// Every signal that should rebuild the snapshot, in one stream, built before the first snapshot;
/// rules match a path namespace, so later objects are already watched.
async fn subscribe(conn: &Connection) -> futures_util::stream::BoxStream<'static, Wake> {
    let mut rules = Vec::new();
    for (member, _) in SIGNALS {
        match zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(NAME)
            .path_namespace(MANAGER_PATH)
            .member(*member)
            .build()
        {
            Ok(r) => rules.push(r.into()),
            Err(e) => log::warn!("no match rule for {member}: {e}"),
        }
    }
    reprisebus::signals(conn, NAME, rules)
        .await
        .filter_map(|s| {
            std::future::ready(match s {
                reprisebus::Signal::OwnerChanged => Some(Wake::Settings),
                reprisebus::Signal::Message(m) => signal_wake(&m),
            })
        })
        .boxed()
}

fn signal_wake(m: &zbus::Message) -> Option<Wake> {
    let header = m.header();
    let member = header.member().map(|m| m.as_str());
    if member == Some("PropertiesChanged") {
        return property_wake(m);
    }
    Some(
        SIGNALS
            .iter()
            .find(|(name, _)| Some(*name) == member)
            .map_or(Wake::Bus, |(_, wake)| wake.clone()),
    )
}

fn property_wake(m: &zbus::Message) -> Option<Wake> {
    let header = m.header();
    let Some(path) = header.path() else {
        return Some(Wake::Bus);
    };
    match m.body().deserialize::<(String, Props, Vec<String>)>() {
        Ok((iface, changed, _invalidated)) => wake_for_change(path.as_str(), &iface, &changed),
        Err(_) => Some(Wake::Bus),
    }
}

#[cfg(test)]
mod tests;
