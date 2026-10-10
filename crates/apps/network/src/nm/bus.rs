//! The NetworkManager side: the connection, subscriptions, snapshot building and the commands the
//! windows send back.

use super::*;
use futures_util::future::join_all;
use futures_util::stream::StreamExt;
use std::sync::mpsc::{Receiver, Sender, channel};
use zbus::zvariant::{ObjectPath, Value};
use zbus::{Connection, Proxy};

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
    let conn = match zbus::connection::Builder::system() {
        Ok(b) => b.method_timeout(CALL_TIMEOUT).build().await,
        Err(e) => Err(e),
    };
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

/// Run the windows' commands on zbus's executor, each answered by a [`Wake::Ran`], so the loop
/// keeps reading while a call waits on the authentication dialog. The returned task must be kept
/// alive.
fn commands(
    conn: &Connection,
    cmds: async_channel::Receiver<Cmd>,
    wakes: async_channel::Sender<Wake>,
) -> zbus::Task<()> {
    let bus = conn.clone();
    conn.executor().spawn(
        async move {
            queue(cmds, wakes, |cmd| {
                let bus = bus.clone();
                async move { command(&bus, &cmd).await }
            })
            .await
        },
        "reprise-network-commands",
    )
}

/// [`commands`] without the bus. `Stats` commands take their own lane so a Status sheet opened
/// during a pending join does not show frozen counters; each lane keeps its order.
async fn queue<F, Fut>(
    cmds: async_channel::Receiver<Cmd>,
    wakes: async_channel::Sender<Wake>,
    run: F,
) where
    F: Fn(Cmd) -> Fut,
    Fut: std::future::Future<Output = Result<Option<NmMsg>, String>>,
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

fn ran(cmd: &Cmd, result: Result<Option<NmMsg>, String>) -> Wake {
    let (reply, error) = match result {
        Ok(reply) => (reply.map(Box::new), None),
        Err(e) => {
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
async fn pump(conn: &Connection, wakes: async_channel::Sender<Wake>) -> zbus::Task<()> {
    let mut streams = subscribe(conn).await;
    conn.executor().spawn(
        async move {
            while let Some(wake) = streams.next().await {
                if wakes.send(wake).await.is_err() {
                    return;
                }
            }
            log::warn!("the network store's signal streams all ended");
        },
        "reprise-network-signals",
    )
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
            .and_then(|b| b.path_namespace(MANAGER_PATH))
            .and_then(|b| b.member(*member))
        {
            Ok(b) => rules.push(b.build().into()),
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

/// One object's properties, None when gone or refused; a failed read leaves its part out of the
/// snapshot and is logged.
async fn get_all(conn: &Connection, path: &str, iface: &str) -> Option<Props> {
    let p = match Proxy::new(conn, NAME, path.to_string(), PROPERTIES).await {
        Ok(p) => p,
        Err(e) => {
            log::debug!("no properties proxy for {path}: {e}");
            return None;
        }
    };
    match p.call_method("GetAll", &(iface,)).await {
        Ok(reply) => match reply.body().deserialize() {
            Ok(props) => Some(props),
            Err(e) => {
                log::debug!("GetAll({iface}) on {path} did not deserialize: {e}");
                None
            }
        },
        Err(e) => {
            log::debug!("GetAll({iface}) on {path} failed: {e}");
            None
        }
    }
}

fn object(props: &Props, key: &str) -> Option<String> {
    let path = props
        .get(key)
        .and_then(|v| ObjectPath::try_from(v.clone()).ok())?
        .to_string();
    (path != "/").then_some(path)
}

async fn paths(conn: &Connection, path: &str, iface: &str, method: &str) -> Vec<String> {
    let p = match Proxy::new(conn, NAME, path.to_string(), iface).await {
        Ok(p) => p,
        Err(e) => {
            log::debug!("no {iface} proxy for {path}: {e}");
            return vec![];
        }
    };
    match p.call_method(method, &()).await {
        Ok(reply) => match reply
            .body()
            .deserialize::<Vec<zbus::zvariant::OwnedObjectPath>>()
        {
            Ok(v) => v.iter().map(|p| p.as_str().to_string()).collect(),
            Err(e) => {
                log::debug!("{method} on {path} did not deserialize: {e}");
                vec![]
            }
        },
        Err(e) => {
            log::debug!("{method} on {path} failed: {e}");
            vec![]
        }
    }
}

/// Every saved connection by object path; the `GetSettings` reads go out together so a rebuild
/// waits one round trip.
async fn saved(conn: &Connection) -> Vec<(String, HashMap<String, Props>)> {
    let list = paths(conn, SETTINGS_PATH, SETTINGS_IFACE, "ListConnections").await;
    join_all(list.into_iter().map(|path| async move {
        let p = match Proxy::new(conn, NAME, path.clone(), CONNECTION_IFACE).await {
            Ok(p) => p,
            Err(e) => {
                log::debug!("no connection proxy for {path}: {e}");
                return None;
            }
        };
        match p.call_method("GetSettings", &()).await {
            Ok(reply) => match reply.body().deserialize::<HashMap<String, Props>>() {
                Ok(s) => Some((path, s)),
                Err(e) => {
                    log::debug!("the settings of {path} did not deserialize: {e}");
                    None
                }
            },
            Err(e) => {
                log::debug!("GetSettings on {path} failed: {e}");
                None
            }
        }
    }))
    .await
    .into_iter()
    .flatten()
    .collect()
}

async fn get_all_of(conn: &Connection, paths: Vec<String>, iface: &str) -> Vec<(String, Props)> {
    join_all(paths.into_iter().map(|path| async move {
        let props = get_all(conn, &path, iface).await?;
        Some((path, props))
    }))
    .await
    .into_iter()
    .flatten()
    .collect()
}

async fn build(
    conn: &Connection,
    prev: &Snapshot,
    saved: &[(String, HashMap<String, Props>)],
) -> Snapshot {
    let Some(manager_props) = get_all(conn, MANAGER_PATH, NAME).await else {
        log::info!("NetworkManager is not answering");
        return Snapshot::default();
    };
    let active_paths = manager_props
        .get("ActiveConnections")
        .and_then(|v| Vec::<zbus::zvariant::OwnedObjectPath>::try_from(v.clone()).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.as_str().to_string())
        .collect();
    let (devices, actives) = futures_util::join!(
        paths(conn, MANAGER_PATH, NAME, "GetDevices"),
        get_all_of(conn, active_paths, ACTIVE_IFACE),
    );
    let (known_ssids, mut vpns) = known_and_vpns(saved);

    let mut active_vpns = HashSet::new();
    let primary_active = object(&manager_props, "PrimaryConnection");
    let mut primary = None;
    let mut profile_of = HashMap::new();
    for (path, props) in actives {
        if let Some(conn_path) = object(&props, "Connection") {
            profile_of.insert(path.clone(), conn_path.clone());
            if uint(&props, "State") == 2 {
                active_vpns.insert(conn_path);
            }
        }
        if primary_active.as_deref() == Some(path.as_str()) {
            primary = props
                .get("Devices")
                .and_then(|v| Vec::<zbus::zvariant::OwnedObjectPath>::try_from(v.clone()).ok())
                .and_then(|d| d.first().map(|p| p.as_str().to_string()));
        }
    }
    for vpn in &mut vpns {
        vpn.active = active_vpns.contains(&vpn.conn_path);
    }

    let known_ssids = &known_ssids;
    let hotspots = hotspot_profiles(saved);
    let (hotspots, profile_of) = (&hotspots, &profile_of);
    let mut out: Vec<Device> = join_all(devices.into_iter().map(|path| async move {
        let props = get_all(conn, &path, DEVICE_IFACE).await?;
        let kind = uint(&props, "DeviceType");
        if kind != 1 && kind != 2 {
            return None;
        }
        let wifi = kind == 2;
        let link_iface = if wifi { WIRELESS_IFACE } else { WIRED_IFACE };
        let ip4_path = object(&props, "Ip4Config");
        let read_if = |p: Option<String>, iface| async move {
            match p {
                Some(p) => get_all(conn, &p, iface).await,
                None => None,
            }
        };
        let (link, ip4, stats, ap_paths) = futures_util::join!(
            get_all(conn, &path, link_iface),
            read_if(ip4_path, IP4_IFACE),
            get_all(conn, &path, STATS_IFACE),
            async {
                if wifi {
                    paths(conn, &path, WIRELESS_IFACE, "GetAllAccessPoints").await
                } else {
                    vec![]
                }
            },
        );
        let aps = get_all_of(conn, ap_paths, AP_IFACE).await;
        let active_ap = wifi
            .then(|| link.as_ref().and_then(|p| object(p, "ActiveAccessPoint")))
            .flatten();
        let dhcp = object(&props, "Dhcp4Config").is_some();
        let hotspot = object(&props, "ActiveConnection")
            .and_then(|a| profile_of.get(&a))
            .and_then(|c| hotspots.get(c))
            .cloned();
        Some(fold_device(&DeviceProps {
            path: path.clone(),
            device: props,
            wired: (!wifi).then(|| link.clone().unwrap_or_default()),
            wireless: wifi.then(|| link.clone().unwrap_or_default()),
            ip4,
            stats,
            aps,
            known_ssids: known_ssids.clone(),
            active_ap,
            dhcp,
            hotspot,
        }))
    }))
    .await
    .into_iter()
    .flatten()
    .collect();
    for device in &mut out {
        let was = prev.devices.iter().find(|d| d.path == device.path);
        device.connected_since = match (device.state, was) {
            (State::Connected, Some(d)) if d.state == State::Connected => d.connected_since,
            (State::Connected, _) => Some(Instant::now()),
            _ => None,
        };
    }
    let names = shell_names(&out.iter().map(|d| d.kind).collect::<Vec<_>>());
    for (device, name) in out.iter_mut().zip(names) {
        device.shell_name = name;
    }
    Snapshot {
        devices: out,
        vpns,
        primary,
        nm_running: true,
        connectivity: Connectivity::from_code(uint(&manager_props, "Connectivity")),
        check_uri: text(&manager_props, "ConnectivityCheckUri"),
    }
}

fn cmd_device(cmd: &Cmd) -> Option<&str> {
    match cmd {
        Cmd::Connect { device, .. }
        | Cmd::Disconnect(device)
        | Cmd::Disable(device)
        | Cmd::Enable(device)
        | Cmd::Repair(device)
        | Cmd::Rescan(device)
        | Cmd::Hotspot { device, .. }
        | Cmd::Stats { device, .. } => Some(device),
        Cmd::VpnUp(_)
        | Cmd::VpnDown(_)
        | Cmd::LoadVpn(_)
        | Cmd::SaveWireGuard { .. }
        | Cmd::SavePluginVpn { .. }
        | Cmd::DeleteVpn(_)
        | Cmd::Import { .. }
        | Cmd::Quit => None,
    }
}

fn failure_title(device: Option<&str>, snapshot: &Snapshot) -> String {
    device
        .and_then(|p| snapshot.devices.iter().find(|d| d.path == p))
        .map(|d| d.shell_name.clone())
        .unwrap_or_else(|| "Network Connections".into())
}

/// A D-Bus error as the message box shows it, without NM's bus-name prefix.
fn clean(e: impl std::fmt::Display) -> String {
    e.to_string().replace("org.freedesktop.NetworkManager.", "")
}

fn opath(path: &str) -> Result<ObjectPath<'_>, String> {
    ObjectPath::try_from(path).map_err(clean)
}

async fn proxy<'a>(conn: &Connection, path: &'a str, iface: &'a str) -> Result<Proxy<'a>, String> {
    proxy_on(conn, NAME, path, iface).await
}

/// A proxy with single `Get`/`Set` calls: a command touches a property once, so caching would only
/// add a match rule and a `GetAll`.
async fn proxy_on<'a>(
    conn: &Connection,
    dest: &'a str,
    path: &'a str,
    iface: &'a str,
) -> Result<Proxy<'a>, String> {
    let path = opath(path)?;
    zbus::proxy::Builder::<Proxy>::new(conn)
        .destination(dest)
        .and_then(|b| b.path(path))
        .and_then(|b| b.interface(iface))
        .map_err(clean)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .map_err(clean)
}

/// A call that may raise the admin password prompt: carries allow-interactive-authorization,
/// without which polkit refuses outright.
async fn call_auth<B, R>(p: &Proxy<'_>, method: &str, body: &B) -> Result<Option<R>, String>
where
    B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
    R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
{
    p.call_with_flags(
        method,
        zbus::proxy::MethodFlags::AllowInteractiveAuth.into(),
        body,
    )
    .await
    .map_err(clean)
}

/// `ActivateConnection(connection, device, specific)`; "/" means "any".
async fn activate(
    conn: &Connection,
    connection: &str,
    device: &str,
    ap: &str,
) -> Result<(), String> {
    let manager = proxy(conn, MANAGER_PATH, NAME).await?;
    call_auth::<_, zbus::zvariant::OwnedObjectPath>(
        &manager,
        "ActivateConnection",
        &(opath(connection)?, opath(device)?, opath(ap)?),
    )
    .await
    .map(|_| ())
}

/// A fresh Wi-Fi profile for `ssid`, with `key_mgmt` security when a passphrase was typed. With an
/// `owner` it is that login's own profile (`settings.modify.own`), else system-wide.
fn wifi_settings<'a>(
    ssid: &Ssid,
    psk: Option<&'a str>,
    key_mgmt: &'static str,
    owner: Option<&str>,
) -> HashMap<String, HashMap<String, Value<'a>>> {
    let mut out: HashMap<String, HashMap<String, Value<'a>>> = HashMap::new();
    let mut connection = HashMap::from([
        ("id".to_string(), Value::from(ssid.to_string())),
        ("type".to_string(), Value::from("802-11-wireless")),
    ]);
    if let Some(owner) = owner {
        connection.insert(
            "permissions".to_string(),
            Value::from(vec![format!("user:{owner}")]),
        );
    }
    out.insert("connection".into(), connection);
    out.insert(
        "802-11-wireless".into(),
        HashMap::from([
            ("ssid".to_string(), Value::from(ssid.0.clone())),
            ("mode".to_string(), Value::from("infrastructure")),
        ]),
    );
    if let Some(psk) = psk {
        out.insert(
            "802-11-wireless-security".into(),
            HashMap::from([
                ("key-mgmt".to_string(), Value::from(key_mgmt)),
                ("psk".to_string(), Value::from(psk)),
            ]),
        );
    }
    out
}

/// The `key-mgmt` for a passphrase profile: "sae" when the AP offers SAE and no PSK (NM will not
/// put wpa-psk on a WPA3-only network), else "wpa-psk".
fn key_mgmt_for(wpa: u32, rsn: u32) -> &'static str {
    if rsn & 0x400 != 0 && (wpa | rsn) & 0x100 == 0 {
        "sae"
    } else {
        "wpa-psk"
    }
}

async fn ap_key_mgmt(conn: &Connection, ap: &str) -> &'static str {
    get_all(conn, ap, AP_IFACE).await.map_or("wpa-psk", |p| {
        key_mgmt_for(uint(&p, "WpaFlags"), uint(&p, "RsnFlags"))
    })
}

fn rekey_mgmt<'a>(current: &'a str, offered: &'a str) -> &'a str {
    match current {
        "wpa-psk" | "sae" => current,
        _ => offered,
    }
}

/// Who a new profile belongs to: nobody (system-wide) when NM allows `settings.modify.system`, else
/// `login`; a system-wide profile would cost everyone else an admin password on every join.
fn profile_owner(perms: &HashMap<String, String>, login: Option<String>) -> Option<String> {
    if perms.get(MODIFY_SYSTEM).is_some_and(|r| r == "yes") {
        None
    } else {
        login
    }
}

async fn new_profile_owner(conn: &Connection) -> Option<String> {
    let perms = async {
        let manager = proxy(conn, MANAGER_PATH, NAME).await?;
        let reply = manager
            .call_method("GetPermissions", &())
            .await
            .map_err(clean)?;
        reply
            .body()
            .deserialize::<HashMap<String, String>>()
            .map_err(clean)
    }
    .await
    .unwrap_or_else(|e| {
        log::debug!("GetPermissions: {e}");
        HashMap::new()
    });
    let login = match proxy_on(
        conn,
        LOGIND,
        "/org/freedesktop/login1/user/self",
        "org.freedesktop.login1.User",
    )
    .await
    {
        Ok(user) => user.get_property::<String>("Name").await.ok(),
        Err(_) => None,
    };
    if login.is_none() {
        log::warn!("logind has no name for this user; a new profile is system-wide");
    }
    profile_owner(&perms, login)
}

async fn connection_for(conn: &Connection, ssid: &Ssid) -> Option<String> {
    if ssid.is_empty() {
        return None;
    }
    saved(conn)
        .await
        .into_iter()
        .find(|(_, s)| {
            s.get("802-11-wireless")
                .is_some_and(|w| ssid_of(w, "ssid") == *ssid)
        })
        .map(|(path, _)| path)
}

/// Write a new passphrase into a saved connection. `GetSettings` omits secrets and `Update`
/// replaces the whole profile, so the map sent back is everything else plus the passphrase;
/// `psk-flags` goes to 0 so NM stores the key. A PSK profile holds nothing else secret.
async fn rekey(
    conn: &Connection,
    conn_path: &str,
    psk: &str,
    offered: &'static str,
) -> Result<(), String> {
    let p = proxy(conn, conn_path, CONNECTION_IFACE).await?;
    let reply = p.call_method("GetSettings", &()).await.map_err(clean)?;
    let current: HashMap<String, Props> = reply.body().deserialize().map_err(clean)?;
    let key_mgmt = current
        .get("802-11-wireless-security")
        .map(|s| text(s, "key-mgmt"))
        .unwrap_or_default();
    let key_mgmt = rekey_mgmt(&key_mgmt, offered).to_string();
    let mut settings: HashMap<String, HashMap<String, Value>> = current
        .into_iter()
        .map(|(group, entries)| {
            let entries = entries
                .into_iter()
                .map(|(k, v)| (k, Value::from(v)))
                .collect();
            (group, entries)
        })
        .collect();
    let security = settings
        .entry("802-11-wireless-security".into())
        .or_default();
    security.insert("key-mgmt".into(), Value::from(key_mgmt));
    security.insert("psk".into(), Value::from(psk));
    security.insert("psk-flags".into(), Value::from(0u32));
    p.call_method("Update", &(settings,))
        .await
        .map(|_| ())
        .map_err(clean)
}

/// `Device.Disconnect`; Disable and Repair only log its error, since a device already down refuses
/// it.
async fn disconnect(conn: &Connection, device: &str) -> Result<(), String> {
    proxy(conn, device, DEVICE_IFACE)
        .await?
        .call_method("Disconnect", &())
        .await
        .map(|_| ())
        .map_err(clean)
}

async fn command(conn: &Connection, cmd: &Cmd) -> Result<Option<NmMsg>, String> {
    match cmd {
        Cmd::LoadVpn(conn_path) => load_vpn(conn, conn_path).await.map(|e| Some(NmMsg::Vpn(e))),
        cmd => act(conn, cmd).await.map(|()| None),
    }
}

async fn settings_of(conn: &Connection, conn_path: &str) -> Result<HashMap<String, Props>, String> {
    let p = proxy(conn, conn_path, CONNECTION_IFACE).await?;
    let reply = p.call_method("GetSettings", &()).await.map_err(clean)?;
    reply.body().deserialize().map_err(clean)
}

/// A saved VPN for its Properties sheet; WireGuard keys come from `GetSecrets`, which may prompt;
/// refused, the key fields are empty.
async fn load_vpn(conn: &Connection, conn_path: &str) -> Result<VpnEdit, String> {
    let settings = settings_of(conn, conn_path).await?;
    let kind = settings
        .get("connection")
        .map(|c| text(c, "type"))
        .unwrap_or_default();
    if kind != "wireguard" {
        return Ok(plugin_vpn(conn_path, &settings));
    }
    let secrets = async {
        let p = proxy(conn, conn_path, CONNECTION_IFACE).await?;
        call_auth::<_, HashMap<String, Props>>(&p, "GetSecrets", &("wireguard",))
            .await?
            .ok_or_else(|| "no secrets in the reply".to_string())
    }
    .await;
    let secrets = secrets
        .inspect_err(|e| log::warn!("the keys of {conn_path} are not readable: {e}"))
        .ok();
    Ok(VpnEdit::WireGuard {
        conn_path: conn_path.to_string(),
        cfg: Box::new(crate::wg::from_settings(&settings, secrets.as_ref())),
    })
}

fn hotspot_settings(
    ssid: &str,
    psk: &str,
    wpa3: bool,
    band: Band,
    owner: Option<&str>,
) -> crate::wg::Settings {
    let mut connection: HashMap<String, Value<'static>> = HashMap::from([
        ("id".to_string(), Value::from("Hotspot")),
        ("type".to_string(), Value::from("802-11-wireless")),
        ("autoconnect".to_string(), Value::from(false)),
    ]);
    if let Some(owner) = owner {
        connection.insert(
            "permissions".to_string(),
            Value::from(vec![format!("user:{owner}")]),
        );
    }
    let mut wireless: HashMap<String, Value<'static>> = HashMap::from([
        ("ssid".to_string(), Value::from(ssid.as_bytes().to_vec())),
        ("mode".to_string(), Value::from("ap")),
    ]);
    match band {
        Band::Auto => {}
        Band::Bg => {
            wireless.insert("band".into(), Value::from("bg"));
        }
        Band::A => {
            wireless.insert("band".into(), Value::from("a"));
        }
    }
    let security: HashMap<String, Value<'static>> = HashMap::from([
        (
            "key-mgmt".to_string(),
            Value::from(if wpa3 { "sae" } else { "wpa-psk" }),
        ),
        ("proto".to_string(), Value::from(vec!["rsn".to_string()])),
        (
            "pairwise".to_string(),
            Value::from(vec!["ccmp".to_string()]),
        ),
        ("group".to_string(), Value::from(vec!["ccmp".to_string()])),
        ("psk".to_string(), Value::from(psk.to_string())),
        ("psk-flags".to_string(), Value::from(0u32)),
    ]);
    HashMap::from([
        ("connection".to_string(), connection),
        ("802-11-wireless".to_string(), wireless),
        ("802-11-wireless-security".to_string(), security),
        (
            "ipv4".to_string(),
            HashMap::from([("method".to_string(), Value::from("shared"))]),
        ),
        (
            "ipv6".to_string(),
            HashMap::from([("method".to_string(), Value::from("ignore"))]),
        ),
    ])
}

fn import_error(stderr: &str) -> String {
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("The file could not be imported.");
    line.strip_prefix("Error: ").unwrap_or(line).to_string()
}

/// `nmcli connection import type KIND file PATH`: importers live in libnm and each VPN plugin's
/// editor library, not on the bus. Run on its own thread so the bus executor never waits.
async fn nmcli_import(path: &std::path::Path, kind: &str) -> Result<(), String> {
    let (tx, rx) = async_channel::bounded(1);
    let (path, kind) = (path.to_path_buf(), kind.to_string());
    std::thread::Builder::new()
        .name("reprise-network-import".into())
        .spawn(move || {
            let out = std::process::Command::new("nmcli")
                .args(["connection", "import", "type", &kind, "file"])
                .arg(&path)
                .output();
            let _ = tx.send_blocking(match out {
                Ok(o) if o.status.success() => Ok(()),
                Ok(o) => Err(import_error(&String::from_utf8_lossy(&o.stderr))),
                Err(e) => Err(format!("nmcli could not be started: {e}")),
            });
        })
        .map_err(|e| e.to_string())?;
    rx.recv().await.map_err(|e| e.to_string())?
}

async fn add_connection(conn: &Connection, settings: crate::wg::Settings) -> Result<(), String> {
    let p = proxy(conn, SETTINGS_PATH, SETTINGS_IFACE).await?;
    call_auth::<_, zbus::zvariant::OwnedObjectPath>(&p, "AddConnection", &(settings,))
        .await
        .map(|_| ())
}

async fn update(
    conn: &Connection,
    conn_path: &str,
    settings: crate::wg::Settings,
) -> Result<(), String> {
    let p = proxy(conn, conn_path, CONNECTION_IFACE).await?;
    call_auth::<_, ()>(&p, "Update", &(settings,))
        .await
        .map(|_| ())
}

async fn act(conn: &Connection, cmd: &Cmd) -> Result<(), String> {
    match cmd {
        Cmd::Connect {
            device,
            ap,
            ssid,
            psk,
        } => {
            let known = connection_for(conn, ssid).await;
            match connect_plan(known.as_deref(), psk.as_ref()) {
                ConnectPlan::Activate(connection) => activate(conn, &connection, device, ap).await,
                ConnectPlan::Rekey { conn_path, psk } => {
                    let offered = ap_key_mgmt(conn, ap).await;
                    rekey(conn, &conn_path, psk, offered).await?;
                    activate(conn, &conn_path, device, ap).await
                }
                ConnectPlan::Add { psk } => {
                    let owner = new_profile_owner(conn).await;
                    let key_mgmt = ap_key_mgmt(conn, ap).await;
                    let manager = proxy(conn, MANAGER_PATH, NAME).await?;
                    manager
                        .call_method(
                            "AddAndActivateConnection",
                            &(
                                wifi_settings(ssid, psk.map(|p| &**p), key_mgmt, owner.as_deref()),
                                opath(device)?,
                                opath(ap)?,
                            ),
                        )
                        .await
                        .map(|_| ())
                        .map_err(clean)
                }
            }
        }
        Cmd::Disconnect(device) => disconnect(conn, device).await,
        Cmd::Disable(device) => {
            if let Err(e) = disconnect(conn, device).await {
                log::debug!("disconnecting {device} before disabling it: {e}");
            }
            proxy(conn, device, DEVICE_IFACE)
                .await?
                .set_property("Managed", false)
                .await
                .map_err(clean)
        }
        Cmd::Enable(device) => proxy(conn, device, DEVICE_IFACE)
            .await?
            .set_property("Managed", true)
            .await
            .map_err(clean),
        Cmd::Repair(device) => {
            if let Err(e) = disconnect(conn, device).await {
                log::debug!("disconnecting {device} before repairing it: {e}");
            }
            activate(conn, "/", device, "/").await
        }
        Cmd::VpnUp(connection) => activate(conn, connection, "/", "/").await,
        Cmd::VpnDown(connection) => {
            let manager = proxy(conn, MANAGER_PATH, NAME).await?;
            let actives = manager
                .get_property::<Vec<zbus::zvariant::OwnedObjectPath>>("ActiveConnections")
                .await
                .map_err(clean)?;
            for active in actives {
                let active = active.as_str().to_string();
                let Some(props) = get_all(conn, &active, ACTIVE_IFACE).await else {
                    continue;
                };
                if object(&props, "Connection").as_deref() != Some(connection.as_str()) {
                    continue;
                }
                return manager
                    .call_method("DeactivateConnection", &(opath(&active)?,))
                    .await
                    .map(|_| ())
                    .map_err(clean);
            }
            Err("That connection is not active.".into())
        }
        Cmd::SaveWireGuard { conn_path, cfg } => {
            let owner = new_profile_owner(conn).await;
            let fresh = crate::wg::settings(cfg, owner.as_deref());
            match conn_path {
                Some(path) => {
                    let current = settings_of(conn, path).await?;
                    update(conn, path, crate::wg::merge(current, fresh)).await
                }
                None => add_connection(conn, fresh).await,
            }
        }
        Cmd::SavePluginVpn {
            conn_path,
            name,
            data,
        } => {
            let current = settings_of(conn, conn_path).await?;
            update(conn, conn_path, plugin_update(current, name, data)).await
        }
        Cmd::DeleteVpn(conn_path) => {
            let p = proxy(conn, conn_path, CONNECTION_IFACE).await?;
            call_auth::<_, ()>(&p, "Delete", &()).await.map(|_| ())
        }
        Cmd::Import { path, kinds } => {
            let mut last = Err("No VPN plugin for NetworkManager is installed.".to_string());
            for kind in kinds {
                last = nmcli_import(path, kind).await;
                if last.is_ok() {
                    break;
                }
            }
            last
        }
        Cmd::Hotspot {
            device,
            ssid,
            psk,
            wpa3,
            band,
        } => {
            let owner = new_profile_owner(conn).await;
            let fresh = hotspot_settings(ssid, psk, *wpa3, *band, owner.as_deref());
            let existing = hotspot_profiles(&saved(conn).await).into_keys().min();
            match existing {
                Some(path) => {
                    let current = settings_of(conn, &path).await?;
                    update(conn, &path, merge_groups(current, fresh)).await?;
                    activate(conn, &path, device, "/").await
                }
                None => {
                    let manager = proxy(conn, MANAGER_PATH, NAME).await?;
                    call_auth::<
                        _,
                        (
                            zbus::zvariant::OwnedObjectPath,
                            zbus::zvariant::OwnedObjectPath,
                        ),
                    >(
                        &manager,
                        "AddAndActivateConnection",
                        &(fresh, opath(device)?, opath("/")?),
                    )
                    .await
                    .map(|_| ())
                }
            }
        }
        Cmd::Rescan(device) => {
            let empty: HashMap<&str, Value<'_>> = HashMap::new();
            proxy(conn, device, WIRELESS_IFACE)
                .await?
                .call_method("RequestScan", &(empty,))
                .await
                .map(|_| ())
                .map_err(clean)
        }
        Cmd::Stats { device, on } => proxy(conn, device, STATS_IFACE)
            .await?
            .set_property("RefreshRateMs", if *on { STATS_MS } else { 0 })
            .await
            .map_err(clean),
        Cmd::LoadVpn(_) => unreachable!("`command` answers LoadVpn itself"),
        Cmd::Quit => unreachable!("`commands` turns Quit into Wake::Quit"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(entries: Vec<(&str, Value<'static>)>) -> Props {
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.try_to_owned().unwrap()))
            .collect()
    }

    const DEV: &str = "/org/freedesktop/NetworkManager/Devices/2";

    fn join() -> Cmd {
        Cmd::Connect {
            device: "/org/freedesktop/NetworkManager/Devices/10".into(),
            ap: "/".into(),
            ssid: "Net".into(),
            psk: None,
        }
    }

    #[test]
    fn a_status_sheet_gets_its_counters_while_a_join_waits_on_a_password() {
        use futures_util::FutureExt;
        use futures_util::future::{Either, select};
        let (cmd_tx, cmd_rx) = async_channel::unbounded();
        let (wake_tx, wake_rx) = async_channel::unbounded();
        cmd_tx.send_blocking(join()).unwrap();
        cmd_tx
            .send_blocking(Cmd::Stats {
                device: DEV.into(),
                on: true,
            })
            .unwrap();
        let q = queue(cmd_rx, wake_tx, |cmd| async move {
            if matches!(cmd, Cmd::Connect { .. }) {
                std::future::pending::<()>().await;
            }
            Ok(None)
        });
        match select(Box::pin(q), Box::pin(wake_rx.recv())).now_or_never() {
            Some(Either::Right((Ok(wake), _))) => assert_eq!(
                wake,
                Wake::Ran {
                    device: Some(DEV.into()),
                    error: None,
                    reply: None,
                },
                "the Stats command answers while the join is pending"
            ),
            _ => panic!("the Stats command waited behind the pending join"),
        }
    }

    #[test]
    fn quit_answers_after_a_queued_stats_command_has_run() {
        let (cmd_tx, cmd_rx) = async_channel::unbounded();
        let (wake_tx, wake_rx) = async_channel::unbounded();
        cmd_tx
            .send_blocking(Cmd::Stats {
                device: DEV.into(),
                on: false,
            })
            .unwrap();
        cmd_tx.send_blocking(Cmd::Quit).unwrap();
        drop(cmd_tx);
        zbus::block_on(queue(cmd_rx, wake_tx, |_| async { Ok(None) }));
        let wakes: Vec<Wake> = std::iter::from_fn(|| wake_rx.try_recv().ok()).collect();
        assert_eq!(
            wakes,
            vec![
                Wake::Ran {
                    device: Some(DEV.into()),
                    error: None,
                    reply: None,
                },
                Wake::Quit
            ],
            "Stats off runs, then Quit answers"
        );
    }

    #[test]
    fn only_a_bare_statistics_tick_skips_the_rebuild() {
        let dev = "/org/freedesktop/NetworkManager/Devices/2";
        let tick = props(vec![("RxBytes", Value::from(10u64))]);
        assert_eq!(
            wake_for_change(dev, STATS_IFACE, &tick),
            Some(Wake::Stats {
                path: dev.into(),
                rx: Some(10),
                tx: None
            })
        );
        let rate = props(vec![
            ("RxBytes", Value::from(10u64)),
            ("RefreshRateMs", Value::from(2000u32)),
        ]);
        assert_eq!(wake_for_change(dev, STATS_IFACE, &rate), Some(Wake::Bus));
        assert_eq!(wake_for_change(dev, DEVICE_IFACE, &tick), Some(Wake::Bus));
        assert_eq!(
            wake_for_change(dev, STATS_IFACE, &props(vec![])),
            Some(Wake::Bus)
        );
    }

    #[test]
    fn only_a_bare_signal_update_skips_the_rebuild() {
        let ap = "/org/freedesktop/NetworkManager/AccessPoint/7";
        let scan = props(vec![
            ("Strength", Value::from(55u8)),
            ("LastSeen", Value::from(1234i32)),
        ]);
        assert_eq!(
            wake_for_change(ap, AP_IFACE, &scan),
            Some(Wake::Strength {
                path: ap.into(),
                strength: Some(55)
            })
        );
        let seen = props(vec![("LastSeen", Value::from(1234i32))]);
        assert_eq!(
            wake_for_change(ap, AP_IFACE, &seen),
            Some(Wake::Strength {
                path: ap.into(),
                strength: None
            })
        );
        let flags = props(vec![
            ("Strength", Value::from(55u8)),
            ("RsnFlags", Value::from(0x100u32)),
        ]);
        assert_eq!(wake_for_change(ap, AP_IFACE, &flags), Some(Wake::Bus));
    }

    #[test]
    fn a_bare_scan_stamp_wakes_nothing() {
        let dev = "/org/freedesktop/NetworkManager/Devices/3";
        let stamp = props(vec![("LastScan", Value::from(5000i64))]);
        assert_eq!(wake_for_change(dev, WIRELESS_IFACE, &stamp), None);
        let rate = props(vec![
            ("LastScan", Value::from(5000i64)),
            ("Bitrate", Value::from(54000u32)),
        ]);
        assert_eq!(wake_for_change(dev, WIRELESS_IFACE, &rate), Some(Wake::Bus));
    }

    #[test]
    fn a_strength_update_patches_the_ap_and_keeps_the_list_in_order() {
        let ap = |path: &str, ssid: &str, strength: u8, active: bool| AccessPoint {
            path: path.into(),
            ssid: ssid.into(),
            strength,
            active,
            ..AccessPoint::default()
        };
        let mut snap = Snapshot {
            devices: vec![Device {
                path: "/d/1".into(),
                wifi: Some(Wifi {
                    active_ssid: Some("Home".into()),
                    strength: 70,
                    aps: vec![
                        ap("/ap/1", "Home", 70, true),
                        ap("/ap/2", "Cafe", 40, false),
                    ],
                }),
                ..Device::default()
            }],
            ..Snapshot::default()
        };
        assert!(patch_strength(&mut snap, "/ap/1", Some(20)));
        let wifi = snap.devices[0].wifi.as_ref().unwrap();
        assert_eq!(wifi.strength, 20, "the live network's strength follows");
        assert_eq!(
            wifi.aps
                .iter()
                .map(|a| (a.path.as_str(), a.strength))
                .collect::<Vec<_>>(),
            [("/ap/2", 40), ("/ap/1", 20)],
            "strongest first"
        );
        assert!(patch_strength(&mut snap, "/ap/2", None));
        assert!(
            !patch_strength(&mut snap, "/ap/9", Some(1)),
            "an unknown access point wants a read"
        );
    }

    #[test]
    fn a_statistics_tick_patches_its_device_and_keeps_the_other_counter() {
        let mut snap = Snapshot {
            devices: vec![Device {
                path: "/d/1".into(),
                rx_bytes: 5,
                tx_bytes: 7,
                ..Device::default()
            }],
            ..Snapshot::default()
        };
        assert!(patch_stats(&mut snap, "/d/1", None, Some(9)));
        assert_eq!((snap.devices[0].rx_bytes, snap.devices[0].tx_bytes), (5, 9));
        assert!(
            !patch_stats(&mut snap, "/d/2", Some(1), None),
            "an unknown device wants a read"
        );
    }

    #[test]
    fn a_new_profile_is_the_users_own_unless_they_may_modify_system_ones() {
        let login = || Some("alice".to_string());
        let with = |r: &str| HashMap::from([(MODIFY_SYSTEM.to_string(), r.to_string())]);
        assert_eq!(profile_owner(&with("yes"), login()), None);
        assert_eq!(
            profile_owner(&with("auth"), login()).as_deref(),
            Some("alice")
        );
        assert_eq!(
            profile_owner(&with("no"), login()).as_deref(),
            Some("alice")
        );
        assert_eq!(
            profile_owner(&HashMap::new(), login()).as_deref(),
            Some("alice"),
            "no answer from NetworkManager keeps the profile private"
        );
        assert_eq!(profile_owner(&with("auth"), None), None);
    }

    #[test]
    fn an_owned_wifi_profile_carries_a_user_permission() {
        let owned = wifi_settings(&"Home".into(), Some("hunter22"), "wpa-psk", Some("alice"));
        assert_eq!(
            owned["connection"].get("permissions"),
            Some(&Value::from(vec!["user:alice".to_string()])),
            "permissions is [\"user:alice\"]"
        );
        let shared = wifi_settings(&"Home".into(), None, "wpa-psk", None);
        assert!(
            !shared["connection"].contains_key("permissions"),
            "a system-wide profile has no permissions key"
        );
    }

    #[test]
    fn a_wpa3_only_network_gets_an_sae_profile() {
        assert_eq!(key_mgmt_for(0, 0x400), "sae", "SAE alone is sae");
        assert_eq!(key_mgmt_for(0, 0x100 | 0x400), "wpa-psk", "a mixed network");
        assert_eq!(key_mgmt_for(0x100, 0x400), "wpa-psk", "WPA1 PSK beside SAE");
        assert_eq!(key_mgmt_for(0, 0x100), "wpa-psk");
        let sae = wifi_settings(&"Home".into(), Some("hunter22"), "sae", None);
        assert_eq!(
            sae["802-11-wireless-security"].get("key-mgmt"),
            Some(&Value::from("sae")),
            "key-mgmt is sae"
        );
    }

    #[test]
    fn a_new_profile_carries_the_ssids_own_bytes() {
        let latin1 = Ssid(b"Caf\xe9".to_vec());
        let s = wifi_settings(&latin1, None, "wpa-psk", None);
        assert_eq!(
            s["802-11-wireless"].get("ssid"),
            Some(&Value::from(b"Caf\xe9".to_vec())),
            "ssid is the bytes 43 61 66 e9"
        );
    }

    #[test]
    fn a_hotspot_profile_is_an_access_point_sharing_its_address() {
        let s = hotspot_settings("Desk", "hunter222", false, Band::A, Some("alice"));
        assert_eq!(s["802-11-wireless"]["mode"], Value::from("ap"));
        assert_eq!(s["802-11-wireless"]["band"], Value::from("a"));
        assert_eq!(s["802-11-wireless"]["ssid"], Value::from(b"Desk".to_vec()));
        assert_eq!(
            s["802-11-wireless-security"]["key-mgmt"],
            Value::from("wpa-psk")
        );
        assert_eq!(s["ipv4"]["method"], Value::from("shared"));
        assert_eq!(
            s["connection"]["permissions"],
            Value::from(vec!["user:alice".to_string()])
        );
        let wpa3 = hotspot_settings("Desk", "hunter222", true, Band::Auto, None);
        assert_eq!(
            wpa3["802-11-wireless-security"]["key-mgmt"],
            Value::from("sae")
        );
        assert!(
            !wpa3["802-11-wireless"].contains_key("band"),
            "Automatic leaves the band to the driver"
        );
    }

    #[test]
    fn a_rewritten_hotspot_keeps_its_uuid_and_drops_the_old_band() {
        let current = HashMap::from([
            (
                "connection".to_string(),
                props(vec![
                    ("id", Value::from("Hotspot")),
                    ("uuid", Value::from("u-1")),
                ]),
            ),
            (
                "802-11-wireless".to_string(),
                props(vec![("band", Value::from("a"))]),
            ),
        ]);
        let m = merge_groups(
            current,
            hotspot_settings("Desk", "hunter222", false, Band::Auto, None),
        );
        assert_eq!(m["connection"]["uuid"], Value::from("u-1"));
        assert!(!m["802-11-wireless"].contains_key("band"));
    }

    #[test]
    fn an_import_error_is_nmclis_own_sentence() {
        assert_eq!(
            import_error("Error: failed to find VPN plugin for openvpn.\n"),
            "failed to find VPN plugin for openvpn."
        );
        assert_eq!(import_error(""), "The file could not be imported.");
    }

    #[test]
    fn a_rekey_keeps_the_profiles_key_mgmt() {
        assert_eq!(rekey_mgmt("sae", "wpa-psk"), "sae");
        assert_eq!(rekey_mgmt("wpa-psk", "sae"), "wpa-psk");
        assert_eq!(rekey_mgmt("", "sae"), "sae", "no security group yet");
        assert_eq!(rekey_mgmt("none", "wpa-psk"), "wpa-psk", "a WEP profile");
    }

    #[test]
    fn quit_returns_at_once_with_no_thread() {
        let (cmds, rx) = async_channel::unbounded::<Cmd>();
        drop(rx);
        cmds.close();
        let (_done_tx, done) = channel();
        let handle = NmHandle { cmds, done };
        let start = std::time::Instant::now();
        handle.quit();
        assert!(start.elapsed() < QUIT_WAIT, "quit waited for the timeout");
    }
}
