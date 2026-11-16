//! The BlueZ side: system bus connection, signals, snapshot and commands.
//! `Pair` and `Connect` run on their own tasks since they can wait on the user,
//! so a `CancelPair` behind them is not stuck in the queue.

use super::*;
use futures_util::future;
use futures_util::stream::StreamExt;
use std::collections::HashMap;
use std::pin::pin;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zbus::Connection;
use zbus::{ObjectPath, OwnedVariant, Variant};

pub(super) const NAME: &str = "org.bluez";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

const QUIT_WAIT: Duration = Duration::from_secs(2);
const CALL_TIMEOUT: Duration = Duration::from_secs(300);
/// The least time between two reads of every object, so a burst of changes costs one.
const READ_GAP: Duration = Duration::from_millis(200);

/// The interfaces whose `PropertiesChanged` may change the snapshot.
const FOLDED: [&str; 3] = [ADAPTER_IFACE, DEVICE_IFACE, BATTERY_IFACE];

#[derive(Debug)]
pub struct BzHandle {
    cmds: async_channel::Sender<Cmd>,
    done: Receiver<()>,
}

impl BzHandle {
    pub fn send(&self, cmd: Cmd) {
        if let Err(e) = self.cmds.send_blocking(cmd) {
            log::warn!("the BlueZ thread is gone: {e}");
        }
    }

    pub fn quit(&self) {
        if self.cmds.send_blocking(Cmd::Quit).is_err() {
            return;
        }
        if let Err(e) = self.done.recv_timeout(QUIT_WAIT) {
            log::warn!("the BlueZ thread did not answer ({e}); exiting anyway");
        }
    }
}

#[cfg(test)]
impl BzHandle {
    pub fn detached() -> (BzHandle, async_channel::Receiver<Cmd>) {
        let (cmds, rx) = async_channel::unbounded();
        let (_, done) = channel();
        (BzHandle { cmds, done }, rx)
    }
}

pub(super) type Poster = repriseapp::Poster<BzMsg>;

pub fn watch(waker: repriseapp::Waker) -> (Receiver<BzMsg>, BzHandle) {
    let (done_tx, done_rx) = channel();
    let (cmd_tx, cmd_rx) = async_channel::unbounded::<Cmd>();
    let (poster, rx) = Poster::channel(waker);
    if let Err(e) = std::thread::Builder::new()
        .name("reprise-bluetooth-bz".into())
        .spawn(move || zbus::block_on(run(poster, cmd_rx, done_tx)))
    {
        log::warn!("could not start the BlueZ thread: {e}");
        cmd_tx.close();
    }
    (
        rx,
        BzHandle {
            cmds: cmd_tx,
            done: done_rx,
        },
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Wake {
    /// An object came or went.
    Objects,
    Changed(Change),
    /// A radio switch changed, so airplane mode is read again.
    Switch,
    Restarted,
    Quit,
}

impl Wake {
    fn alters(&self, last: &Snapshot) -> bool {
        match self {
            Wake::Changed(c) => c.alters(last),
            _ => true,
        }
    }
}

/// `REPRISE_BLUEZ_BUS=session` reads BlueZ off the session bus, for a test's stand-in service.
fn bus() -> zbus::connection::Builder<'static> {
    if std::env::var_os("REPRISE_BLUEZ_BUS").is_some_and(|v| v == "session") {
        zbus::connection::Builder::session()
    } else {
        zbus::connection::Builder::system()
    }
}

async fn run(poster: Poster, cmds: async_channel::Receiver<Cmd>, done: Sender<()>) {
    let conn = bus().method_timeout(CALL_TIMEOUT).build().await;
    let conn = match conn {
        Ok(c) => c,
        Err(e) => {
            log::info!("no system bus ({e}); no Bluetooth to show");
            poster.post(BzMsg::Snapshot(Snapshot::default()));
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
    if let Err(e) = agent::serve(&conn, poster.clone()).await {
        log::warn!("cannot serve the pairing agent: {e}");
    }
    if owns_bluez(&conn).await {
        register_agent(&conn).await;
    }
    let obex = super::obex::start(&conn, poster.clone()).await;
    let (wake_tx, wake_rx) = async_channel::unbounded::<Wake>();
    let _pump = pump(&conn, wake_tx.clone()).await;
    follow_switches(wake_tx.clone());
    let _commands = commands(&conn, cmds, wake_tx, poster.clone(), obex);
    follow(&conn, &wake_rx, &poster).await;
    let _ = done.send(());
}

/// Posts the snapshot, then reads and posts it again after each wake that may change it, at most
/// once per [`READ_GAP`]; one equal to the last posted is not posted. Airplane mode is read only
/// at the start and after a radio switch. Returns on [`Wake::Quit`].
async fn follow(conn: &Connection, wakes: &async_channel::Receiver<Wake>, poster: &Poster) {
    let mut airplane = reprisebus::airplane::read(conn).await;
    let mut last = snapshot(conn, airplane).await;
    poster.post(BzMsg::Snapshot(last.clone()));
    let mut read_at = Instant::now();
    loop {
        let mut woke = Vec::new();
        while woke.is_empty() {
            let Ok(w) = wakes.recv().await else {
                return;
            };
            if w.alters(&last) {
                woke.push(w);
            }
        }
        if let Some(left) = READ_GAP.checked_sub(read_at.elapsed()) {
            match zruntime::SharedRuntime::current() {
                Ok(rt) => rt.sleep(left).await,
                Err(e) => log::debug!("no runtime to wait on: {e}"),
            }
        }
        woke.extend(std::iter::from_fn(|| wakes.try_recv().ok()));
        if woke.contains(&Wake::Quit) {
            return;
        }
        if woke.contains(&Wake::Restarted) && owns_bluez(conn).await {
            register_agent(conn).await;
        }
        if woke.contains(&Wake::Switch) {
            airplane = reprisebus::airplane::read(conn).await;
        }
        let next = snapshot(conn, airplane).await;
        read_at = Instant::now();
        if next != last {
            poster.post(BzMsg::Snapshot(next.clone()));
            last = next;
        }
    }
}

async fn snapshot(conn: &Connection, airplane: Option<reprisebus::airplane::Mode>) -> Snapshot {
    let mut s = if owns_bluez(conn).await {
        read(conn).await
    } else {
        Snapshot::default()
    };
    s.airplane = airplane;
    s
}

/// Read again on every rfkill change, which airplane mode and a radio switched elsewhere make.
fn follow_switches(wakes: async_channel::Sender<Wake>) {
    let spawned = std::thread::Builder::new()
        .name("reprise-bluetooth-rfkill".into())
        .spawn(move || {
            if let Err(e) = reprisebus::rfkill::follow(|| wakes.send_blocking(Wake::Switch).is_ok())
            {
                log::info!("radio switches are not followed: {e}");
            }
        });
    if let Err(e) = spawned {
        log::warn!("could not start the rfkill thread: {e}");
    }
}

/// Whether bluetoothd owns `org.bluez`. `read` and `register_agent` check it:
/// plain method calls would auto-start bluetoothd after an admin stopped it.
async fn owns_bluez(conn: &Connection) -> bool {
    reprisebus::name_owner(conn, NAME).await.is_some()
}

async fn register_agent(conn: &Connection) {
    match agent::register(conn).await {
        Ok(()) => log::info!("registered as the Bluetooth pairing agent"),
        Err(e) => log::info!("BlueZ did not take the pairing agent ({e})"),
    }
}

async fn read(conn: &Connection) -> Snapshot {
    match reprisebus::bluez::objects(conn).await {
        Ok(objects) => fold(&objects),
        Err(e) => {
            log::debug!("no BlueZ objects ({e})");
            Snapshot::default()
        }
    }
}

fn quiet(cmd: &Cmd) -> bool {
    matches!(cmd, Cmd::StopDiscovery(_) | Cmd::CancelPair(_))
}

pub(crate) fn error_of(e: &zbus::Error) -> String {
    match reprisebus::error::name(e) {
        Some(name) => plain_error(&name, reprisebus::error::message(e).unwrap_or("")),
        None => e.to_string(),
    }
}

/// The pairings in flight by device, each under a serial of its own. Dropping a pairing's sender
/// tells its task the user cancelled it.
#[derive(Clone, Default)]
struct Pairings(Arc<Mutex<InFlight>>);

#[derive(Default)]
struct InFlight {
    serial: u64,
    by_device: HashMap<String, (u64, async_channel::Sender<()>)>,
}

impl Pairings {
    fn lock(&self) -> std::sync::MutexGuard<'_, InFlight> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn start(&self, device: &str) -> (u64, async_channel::Receiver<()>) {
        let (tx, rx) = async_channel::bounded(1);
        let mut m = self.lock();
        m.serial += 1;
        let serial = m.serial;
        m.by_device.insert(device.to_string(), (serial, tx));
        (serial, rx)
    }

    fn cancel(&self, device: &str) {
        self.lock().by_device.remove(device);
    }

    fn end(&self, device: &str, serial: u64) {
        let mut m = self.lock();
        if m.by_device.get(device).is_some_and(|(s, _)| *s == serial) {
            m.by_device.remove(device);
        }
    }
}

fn commands(
    conn: &Connection,
    cmds: async_channel::Receiver<Cmd>,
    wakes: async_channel::Sender<Wake>,
    poster: Poster,
    obex: Option<super::obex::Obex>,
) -> zruntime::Task<(), zruntime::Shared> {
    let bus = conn.clone();
    let pairings = Pairings::default();
    zruntime::spawn(async move {
        while let Ok(cmd) = cmds.recv().await {
            if let Cmd::CancelPair(device) = &cmd {
                pairings.cancel(device);
            }
            match cmd {
                Cmd::Quit => {
                    let _ = wakes.send(Wake::Quit).await;
                    return;
                }
                Cmd::Pair(device) => {
                    let (b, p, all) = (bus.clone(), poster.clone(), pairings.clone());
                    let (serial, cancelled) = all.start(&device);
                    zruntime::spawn(async move {
                        pair(&b, &device, &p, &cancelled).await;
                        all.end(&device, serial);
                    })
                    .detach();
                }
                Cmd::Receive(on) => {
                    if let Some(o) = &obex {
                        o.receive(on).await;
                    }
                }
                Cmd::Airplane(on) => {
                    if let Err(body) = reprisebus::airplane::set(&bus, on).await {
                        log::warn!("{cmd:?} failed: {body}");
                        poster.post(BzMsg::Failed {
                            title: failure_title(&cmd).into(),
                            body,
                        });
                    }
                }
                Cmd::Connect(device) => {
                    let (b, p) = (bus.clone(), poster.clone());
                    zruntime::spawn(async move {
                        let error = call0(&b, &device, DEVICE_IFACE, "Connect")
                            .await
                            .err()
                            .map(|e| error_of(&e));
                        p.post(BzMsg::Connected { device, error });
                    })
                    .detach();
                }
                cmd => {
                    if let Err(e) = command(&bus, &cmd).await {
                        log::warn!("{cmd:?} failed: {e}");
                        if !quiet(&cmd) {
                            poster.post(BzMsg::Failed {
                                title: failure_title(&cmd).into(),
                                body: error_of(&e),
                            });
                        }
                    }
                }
            }
        }
    })
}

/// Pairs, trusts, then connects `device`. Once `cancelled` closes, a device that paired anyway
/// is removed, so a cancelled wizard leaves it neither paired nor trusted.
async fn pair(
    conn: &Connection,
    device: &str,
    poster: &Poster,
    cancelled: &async_channel::Receiver<()>,
) {
    if let Err(e) = call0(conn, device, DEVICE_IFACE, "Pair").await {
        poster.post(BzMsg::PairFailed {
            device: device.into(),
            reason: error_of(&e),
        });
        return;
    }
    let connected = async {
        if let Err(e) = set(conn, device, DEVICE_IFACE, "Trusted", Variant::from(true)).await {
            log::warn!("could not trust {device}: {e}");
        }
        call0(conn, device, DEVICE_IFACE, "Connect").await
    };
    let cancel = cancelled.recv();
    let connected = match cancelled.is_closed() {
        true => None,
        false => match future::select(pin!(connected), pin!(cancel)).await {
            future::Either::Left((r, _)) if !cancelled.is_closed() => Some(r),
            _ => None,
        },
    };
    let Some(connected) = connected else {
        log::info!("pairing {device} was cancelled after it paired; removing it");
        let adapter = device.rsplit_once('/').map_or("", |(a, _)| a);
        if let Err(e) = remove(conn, adapter, device).await {
            log::warn!("could not remove {device}: {e}");
        }
        return;
    };
    poster.post(BzMsg::Paired {
        device: device.into(),
        connect_error: connected.err().map(|e| error_of(&e)),
    });
}

async fn remove(conn: &Connection, adapter: &str, device: &str) -> zbus::Result<()> {
    let dev = ObjectPath::try_from(device)?;
    conn.call_method(
        Some(NAME),
        adapter,
        Some(ADAPTER_IFACE),
        "RemoveDevice",
        &(dev,),
    )
    .await
    .map(|_| ())
}

async fn command(conn: &Connection, cmd: &Cmd) -> zbus::Result<()> {
    match cmd {
        Cmd::SetPowered { adapter, on } => {
            if *on
                && let Err(e) =
                    reprisebus::rfkill::set_blocked(reprisebus::rfkill::Kind::Bluetooth, false)
            {
                log::info!("could not unblock Bluetooth through rfkill: {e}");
            }
            // Every adapter follows, so `fold` never shows another adapter that is still on.
            let mut r = set(conn, adapter, ADAPTER_IFACE, "Powered", Variant::from(*on)).await;
            for other in adapters(conn).await.iter().filter(|a| *a != adapter) {
                match set(conn, other, ADAPTER_IFACE, "Powered", Variant::from(*on)).await {
                    Ok(()) => r = Ok(()),
                    Err(e) => log::info!("could not switch {other}: {e}"),
                }
            }
            r
        }
        Cmd::SetDiscoverable { adapter, on } => {
            set(
                conn,
                adapter,
                ADAPTER_IFACE,
                "Discoverable",
                Variant::from(*on),
            )
            .await
        }
        Cmd::StartDiscovery(a) => call0(conn, a, ADAPTER_IFACE, "StartDiscovery").await,
        Cmd::StopDiscovery(a) => call0(conn, a, ADAPTER_IFACE, "StopDiscovery").await,
        Cmd::CancelPair(d) => call0(conn, d, DEVICE_IFACE, "CancelPairing").await,
        Cmd::Disconnect(d) => call0(conn, d, DEVICE_IFACE, "Disconnect").await,
        Cmd::SetTrusted { device, on } => {
            set(conn, device, DEVICE_IFACE, "Trusted", Variant::from(*on)).await
        }
        Cmd::SetAlias { device, alias } => {
            set(
                conn,
                device,
                DEVICE_IFACE,
                "Alias",
                Variant::from(alias.as_str()),
            )
            .await
        }
        Cmd::Remove { adapter, device } => remove(conn, adapter, device).await,
        Cmd::Pair(_) | Cmd::Connect(_) | Cmd::Receive(_) | Cmd::Airplane(_) | Cmd::Quit => Ok(()),
    }
}

/// The paths of every adapter BlueZ lists.
async fn adapters(conn: &Connection) -> Vec<String> {
    let objects = reprisebus::bluez::objects(conn).await;
    match objects {
        Ok(o) => (o.into_iter())
            .filter(|(_, ifaces)| ifaces.contains_key(ADAPTER_IFACE))
            .map(|(p, _)| p.to_string())
            .collect(),
        Err(e) => {
            log::info!("cannot list the adapters ({e})");
            Vec::new()
        }
    }
}

async fn call0(conn: &Connection, path: &str, iface: &str, method: &str) -> zbus::Result<()> {
    conn.call_method(Some(NAME), path, Some(iface), method, &())
        .await
        .map(|_| ())
}

async fn set(
    conn: &Connection,
    path: &str,
    iface: &str,
    prop: &str,
    value: Variant<'_>,
) -> zbus::Result<()> {
    conn.call_method(
        Some(NAME),
        path,
        Some(PROPERTIES),
        "Set",
        &(iface, prop, value),
    )
    .await
    .map(|_| ())
}

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
        log::warn!("the BlueZ signal streams all ended");
    })
}

/// Objects coming and going, and `PropertiesChanged` on the [`FOLDED`] interfaces only: a GATT
/// characteristic's notifications or a media player's track wake nothing.
async fn subscribe(conn: &Connection) -> futures_util::stream::BoxStream<'static, Wake> {
    let signal = || {
        zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(NAME)
    };
    let objects = ["InterfacesAdded", "InterfacesRemoved"]
        .map(|member| signal().interface(OBJECT_MANAGER).member(member).build());
    let changes = FOLDED.map(|iface| {
        (signal().interface(PROPERTIES).member("PropertiesChanged"))
            .add_arg(iface)
            .build()
    });
    let mut rules = Vec::new();
    for rule in objects.into_iter().chain(changes) {
        match rule {
            Ok(r) => rules.push(r.into()),
            Err(e) => log::warn!("no match rule for BlueZ: {e}"),
        }
    }
    reprisebus::signals(conn, NAME, rules)
        .await
        .filter_map(|s| async move {
            match s {
                reprisebus::Signal::OwnerChanged => Some(Wake::Restarted),
                reprisebus::Signal::Message(m) => wake_of(&m),
            }
        })
        .boxed()
}

/// The wake a BlueZ signal makes; None for a `PropertiesChanged` that doesn't parse.
fn wake_of(m: &zbus::Message) -> Option<Wake> {
    let hdr = m.header();
    if hdr.member().map(|n| n.as_str()) != Some("PropertiesChanged") {
        return Some(Wake::Objects);
    }
    let path = hdr.path()?.to_string();
    let (iface, changed, invalidated): (String, HashMap<String, OwnedVariant>, Vec<String>) =
        m.body().deserialize().ok()?;
    Some(Wake::Changed(Change {
        path,
        iface,
        changed: changed.into_keys().collect(),
        invalidated,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bz::obex::tests::{bus, connect};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const DEVICE: &str = "/org/bluez/hci0/dev_AA";

    /// BlueZ's object manager, counting its `GetManagedObjects` calls.
    struct FakeManager {
        connected: Arc<Mutex<bool>>,
        reads: Arc<AtomicUsize>,
    }

    #[zbus::interface(name = "org.freedesktop.DBus.ObjectManager")]
    impl FakeManager {
        fn get_managed_objects(&self) -> Objects {
            self.reads.fetch_add(1, Ordering::Relaxed);
            let connected = *self.connected.lock().unwrap();
            let props = |pairs: Vec<(&str, Variant)>| -> Props {
                (pairs.into_iter())
                    .map(|(k, v)| (k.to_string(), OwnedVariant::try_from(v).unwrap()))
                    .collect()
            };
            let adapter = props(vec![("Powered", Variant::from(true))]);
            let device = props(vec![
                ("Address", Variant::from("AA:BB:CC:DD:EE:01")),
                ("Connected", Variant::from(connected)),
                ("RSSI", Variant::from(-60_i16)),
            ]);
            let path = |p: &str| zbus::OwnedObjectPath::try_from(p).unwrap();
            Objects::from([
                (
                    path("/org/bluez/hci0"),
                    HashMap::from([(ADAPTER_IFACE.to_string(), adapter)]),
                ),
                (
                    path(DEVICE),
                    HashMap::from([(DEVICE_IFACE.to_string(), device)]),
                ),
            ])
        }
    }

    async fn changed(bluez: &Connection, path: &str, iface: &str, key: &str, value: Variant<'_>) {
        let body = (iface, HashMap::from([(key, value)]), Vec::<&str>::new());
        (bluez.emit_signal(None::<&str>, path, PROPERTIES, "PropertiesChanged", &body))
            .await
            .unwrap();
    }

    async fn pause(ms: u64) {
        let rt = zruntime::SharedRuntime::current().unwrap();
        rt.sleep(Duration::from_millis(ms)).await;
    }

    #[test]
    fn changes_the_snapshot_does_not_show_cost_no_read_and_a_burst_costs_one() {
        let Some(bus) = bus() else {
            return;
        };
        let connected = Arc::new(Mutex::new(false));
        let reads = Arc::new(AtomicUsize::new(0));
        let (poster, posted) = Poster::channel(repriseapp::Waker::noop());
        let (reads_quiet, reads_end) = zbus::block_on(async {
            let bluez = connect(&bus.1).await;
            let manager = FakeManager {
                connected: connected.clone(),
                reads: reads.clone(),
            };
            bluez.object_server().at("/", manager).await.unwrap();
            bluez.request_name(NAME).await.unwrap();
            let conn = connect(&bus.1).await;
            let (tx, rx) = async_channel::unbounded();
            let _pump = pump(&conn, tx.clone()).await;
            let drive = async {
                pause(300).await;
                for i in 0..50_i16 {
                    changed(&bluez, DEVICE, DEVICE_IFACE, "RSSI", Variant::from(-40 - i)).await;
                    changed(
                        &bluez,
                        DEVICE,
                        DEVICE_IFACE,
                        "ManufacturerData",
                        Variant::from(i),
                    )
                    .await;
                    let gatt = format!("{DEVICE}/service0001/char0002");
                    let value = Variant::from(vec![i as u8]);
                    changed(
                        &bluez,
                        &gatt,
                        "org.bluez.GattCharacteristic1",
                        "Value",
                        value,
                    )
                    .await;
                }
                pause(300).await;
                let quiet = reads.load(Ordering::Relaxed);
                *connected.lock().unwrap() = true;
                for _ in 0..5 {
                    changed(
                        &bluez,
                        DEVICE,
                        DEVICE_IFACE,
                        "Connected",
                        Variant::from(true),
                    )
                    .await;
                }
                pause(500).await;
                tx.send(Wake::Quit).await.unwrap();
                quiet
            };
            let ((), quiet) = futures_util::future::join(follow(&conn, &rx, &poster), drive).await;
            (quiet, reads.load(Ordering::Relaxed))
        });
        assert_eq!(
            reads_quiet, 1,
            "only the first read before the device connects"
        );
        assert!(
            reads_end <= 3,
            "the burst of five reads at most twice, {READ_GAP:?} apart: {reads_end}"
        );
        let shown: Vec<bool> = std::iter::from_fn(|| posted.try_recv().ok())
            .map(|m| match m {
                BzMsg::Snapshot(s) => s.devices[0].connected,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(shown, [false, true], "one snapshot per change");
    }

    /// A device whose `Connect` never answers when `hang`, reporting it on `calls`.
    struct FakeDevice {
        path: &'static str,
        hang: bool,
        trusted: bool,
        calls: std::sync::mpsc::Sender<String>,
    }

    #[zbus::interface(name = "org.bluez.Device1")]
    impl FakeDevice {
        fn pair(&self) {}
        fn cancel_pairing(&self) -> zbus::fdo::Result<()> {
            Err(zbus::fdo::Error::Failed("no pairing in progress".into()))
        }
        async fn connect(&self) {
            let _ = self.calls.send(format!("Connect {}", self.path));
            if self.hang {
                std::future::pending::<()>().await;
            }
        }
        #[zbus(property)]
        fn trusted(&self) -> bool {
            self.trusted
        }
        #[zbus(property)]
        fn set_trusted(&mut self, on: bool) {
            self.trusted = on;
        }
    }

    struct FakeAdapter {
        calls: std::sync::mpsc::Sender<String>,
        powered: bool,
    }

    #[zbus::interface(name = "org.bluez.Adapter1")]
    impl FakeAdapter {
        fn remove_device(&self, device: zbus::OwnedObjectPath) {
            let _ = self.calls.send(format!("RemoveDevice {}", device.as_str()));
        }
        #[zbus(property)]
        fn powered(&self) -> bool {
            self.powered
        }
        #[zbus(property)]
        fn set_powered(&mut self, on: bool) {
            self.powered = on;
        }
    }

    #[test]
    fn a_wizard_cancelled_after_pairing_removes_the_device() {
        let Some(bus) = bus() else {
            return;
        };
        let (calls_tx, calls) = std::sync::mpsc::channel();
        let (poster, posted) = Poster::channel(repriseapp::Waker::noop());
        const OTHER: &str = "/org/bluez/hci0/dev_BB";
        zbus::block_on(async {
            let bluez = connect(&bus.1).await;
            let server = bluez.object_server();
            let adapter = FakeAdapter {
                calls: calls_tx.clone(),
                powered: true,
            };
            server.at("/org/bluez/hci0", adapter).await.unwrap();
            for (path, hang) in [(DEVICE, true), (OTHER, false)] {
                let calls = calls_tx.clone();
                let device = FakeDevice {
                    path,
                    hang,
                    trusted: false,
                    calls,
                };
                server.at(path, device).await.unwrap();
            }
            bluez.request_name(NAME).await.unwrap();
            let conn = connect(&bus.1).await;
            let (cmds_tx, cmds) = async_channel::unbounded();
            let (wakes, _wakes_rx) = async_channel::unbounded();
            let _commands = commands(&conn, cmds, wakes, poster, None);
            cmds_tx.send(Cmd::Pair(DEVICE.into())).await.unwrap();
            let rt = zruntime::SharedRuntime::current().unwrap();
            let connecting = rt.timeout(Duration::from_secs(5), async {
                while calls.try_recv() != Ok(format!("Connect {DEVICE}")) {
                    pause(20).await;
                }
            });
            connecting.await.expect("Connect called");
            cmds_tx.send(Cmd::CancelPair(DEVICE.into())).await.unwrap();
            cmds_tx.send(Cmd::Pair(OTHER.into())).await.unwrap();
            pause(500).await;
            let other = server.interface::<_, FakeDevice>(OTHER).await.unwrap();
            assert!(other.get().await.trusted, "the paired device is trusted");
        });
        let mut seen: Vec<String> = calls.try_iter().collect();
        seen.sort();
        assert_eq!(
            seen,
            [format!("Connect {OTHER}"), format!("RemoveDevice {DEVICE}")],
            "the cancelled device goes and the other stays"
        );
        let paired: Vec<String> = std::iter::from_fn(|| posted.try_recv().ok())
            .filter_map(|m| match m {
                BzMsg::Paired { device, .. } => Some(device),
                _ => None,
            })
            .collect();
        assert_eq!(
            paired,
            [OTHER],
            "only the pairing left alone reports Paired"
        );
    }

    #[test]
    fn the_radio_switch_turns_every_adapter_off_and_on() {
        let Some(bus) = bus() else {
            return;
        };
        const ADAPTERS: [&str; 2] = ["/org/bluez/hci0", "/org/bluez/hci1"];
        zbus::block_on(async {
            let bluez = connect(&bus.1).await;
            let server = bluez.object_server();
            server.at("/", zbus::fdo::ObjectManager).await.unwrap();
            for path in ADAPTERS {
                let (calls, _) = std::sync::mpsc::channel();
                let adapter = FakeAdapter {
                    calls,
                    powered: true,
                };
                server.at(path, adapter).await.unwrap();
            }
            bluez.request_name(NAME).await.unwrap();
            let conn = connect(&bus.1).await;
            for on in [false, true] {
                let adapter = ADAPTERS[1].to_string();
                command(&conn, &Cmd::SetPowered { adapter, on })
                    .await
                    .unwrap();
                for path in ADAPTERS {
                    let iface = server.interface::<_, FakeAdapter>(path).await.unwrap();
                    assert_eq!(iface.get().await.powered, on, "{path}");
                }
            }
        });
    }

    #[test]
    fn a_method_error_reads_through_plain_error() {
        let e = reprisebus::error::reply("org.bluez.Error.NotReady", "Resource Not Ready");
        assert_eq!(
            error_of(&e),
            "Bluetooth is not ready. Make sure it is turned on."
        );
        let denied = reprisebus::error::reply(
            "org.freedesktop.DBus.Error.AccessDenied",
            "Rejected send message",
        );
        assert_eq!(error_of(&denied), "Rejected send message");
    }

    #[test]
    fn quiet_commands_do_not_raise_a_box() {
        assert!(quiet(&Cmd::StopDiscovery("/a".into())));
        assert!(quiet(&Cmd::CancelPair("/d".into())));
        assert!(!quiet(&Cmd::Disconnect("/d".into())));
    }
}
