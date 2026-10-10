//! The BlueZ side: system bus connection, signals, snapshot and commands.
//! `Pair` and `Connect` run on their own tasks since they can wait on the user,
//! so a `CancelPair` behind them is not stuck in the queue.

use super::*;
use futures_util::stream::StreamExt;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;
use zbus::Connection;
use zbus::zvariant::{ObjectPath, Value};

pub(super) const NAME: &str = "org.bluez";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

const QUIT_WAIT: Duration = Duration::from_secs(2);
const CALL_TIMEOUT: Duration = Duration::from_secs(300);

const SIGNALS: &[&str] = &["InterfacesAdded", "InterfacesRemoved", "PropertiesChanged"];

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

#[derive(Clone)]
pub(super) struct Poster {
    tx: Sender<BzMsg>,
    waker: repriseapp::Waker,
}

impl Poster {
    pub(super) fn post(&self, m: BzMsg) {
        if self.tx.send(m).is_ok() {
            self.waker.wake();
        }
    }
}

pub fn watch(waker: repriseapp::Waker) -> (Receiver<BzMsg>, BzHandle) {
    let (tx, rx) = channel();
    let (done_tx, done_rx) = channel();
    let (cmd_tx, cmd_rx) = async_channel::unbounded::<Cmd>();
    let poster = Poster { tx, waker };
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wake {
    Bus,
    Restarted,
    Quit,
}

async fn run(poster: Poster, cmds: async_channel::Receiver<Cmd>, done: Sender<()>) {
    let conn = match zbus::connection::Builder::system() {
        Ok(b) => b.method_timeout(CALL_TIMEOUT).build().await,
        Err(e) => Err(e),
    };
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
    let (wake_tx, wake_rx) = async_channel::unbounded::<Wake>();
    let _pump = pump(&conn, wake_tx.clone()).await;
    let _commands = commands(&conn, cmds, wake_tx, poster.clone());
    loop {
        let snapshot = if owns_bluez(&conn).await {
            read(&conn).await
        } else {
            Snapshot::default()
        };
        poster.post(BzMsg::Snapshot(snapshot));
        let Ok(first) = wake_rx.recv().await else {
            let _ = done.send(());
            return;
        };
        let mut wakes = vec![first];
        while let Ok(w) = wake_rx.try_recv() {
            wakes.push(w);
        }
        if wakes.contains(&Wake::Quit) {
            let _ = done.send(());
            return;
        }
        if wakes.contains(&Wake::Restarted) && owns_bluez(&conn).await {
            register_agent(&conn).await;
        }
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
    let reply = conn
        .call_method(
            Some(NAME),
            "/",
            Some(OBJECT_MANAGER),
            "GetManagedObjects",
            &(),
        )
        .await;
    match reply.and_then(|m| m.body().deserialize::<Objects>()) {
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
    match e {
        zbus::Error::MethodError(name, detail, _) => {
            plain_error(name.as_str(), detail.as_deref().unwrap_or(""))
        }
        other => other.to_string(),
    }
}

fn commands(
    conn: &Connection,
    cmds: async_channel::Receiver<Cmd>,
    wakes: async_channel::Sender<Wake>,
    poster: Poster,
) -> zbus::Task<()> {
    let bus = conn.clone();
    conn.executor().spawn(
        async move {
            while let Ok(cmd) = cmds.recv().await {
                match cmd {
                    Cmd::Quit => {
                        let _ = wakes.send(Wake::Quit).await;
                        return;
                    }
                    Cmd::Pair(device) => {
                        let (b, p) = (bus.clone(), poster.clone());
                        bus.executor()
                            .spawn(
                                async move { pair(&b, &device, &p).await },
                                "reprise-bluetooth-pair",
                            )
                            .detach();
                    }
                    Cmd::Connect(device) => {
                        let (b, p) = (bus.clone(), poster.clone());
                        bus.executor()
                            .spawn(
                                async move {
                                    let error = call0(&b, &device, DEVICE_IFACE, "Connect")
                                        .await
                                        .err()
                                        .map(|e| error_of(&e));
                                    p.post(BzMsg::Connected { device, error });
                                },
                                "reprise-bluetooth-connect",
                            )
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
        },
        "reprise-bluetooth-commands",
    )
}

async fn pair(conn: &Connection, device: &str, poster: &Poster) {
    if let Err(e) = call0(conn, device, DEVICE_IFACE, "Pair").await {
        poster.post(BzMsg::PairFailed {
            device: device.into(),
            reason: error_of(&e),
        });
        return;
    }
    if let Err(e) = set(conn, device, DEVICE_IFACE, "Trusted", Value::from(true)).await {
        log::warn!("could not trust {device}: {e}");
    }
    let connect_error = call0(conn, device, DEVICE_IFACE, "Connect")
        .await
        .err()
        .map(|e| error_of(&e));
    poster.post(BzMsg::Paired {
        device: device.into(),
        connect_error,
    });
}

async fn command(conn: &Connection, cmd: &Cmd) -> zbus::Result<()> {
    match cmd {
        Cmd::SetPowered { adapter, on } => {
            if *on && let Err(e) = rfkill::unblock() {
                log::info!("could not unblock Bluetooth through rfkill: {e}");
            }
            set(conn, adapter, ADAPTER_IFACE, "Powered", Value::from(*on)).await
        }
        Cmd::SetDiscoverable { adapter, on } => {
            set(
                conn,
                adapter,
                ADAPTER_IFACE,
                "Discoverable",
                Value::from(*on),
            )
            .await
        }
        Cmd::StartDiscovery(a) => call0(conn, a, ADAPTER_IFACE, "StartDiscovery").await,
        Cmd::StopDiscovery(a) => call0(conn, a, ADAPTER_IFACE, "StopDiscovery").await,
        Cmd::CancelPair(d) => call0(conn, d, DEVICE_IFACE, "CancelPairing").await,
        Cmd::Disconnect(d) => call0(conn, d, DEVICE_IFACE, "Disconnect").await,
        Cmd::SetTrusted { device, on } => {
            set(conn, device, DEVICE_IFACE, "Trusted", Value::from(*on)).await
        }
        Cmd::SetAlias { device, alias } => {
            set(
                conn,
                device,
                DEVICE_IFACE,
                "Alias",
                Value::from(alias.as_str()),
            )
            .await
        }
        Cmd::Remove { adapter, device } => {
            let dev = ObjectPath::try_from(device.as_str())?;
            conn.call_method(
                Some(NAME),
                adapter.as_str(),
                Some(ADAPTER_IFACE),
                "RemoveDevice",
                &(dev,),
            )
            .await
            .map(|_| ())
        }
        Cmd::Pair(_) | Cmd::Connect(_) | Cmd::Quit => Ok(()),
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
    value: Value<'_>,
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

async fn pump(conn: &Connection, wakes: async_channel::Sender<Wake>) -> zbus::Task<()> {
    let mut streams = subscribe(conn).await;
    conn.executor().spawn(
        async move {
            while let Some(wake) = streams.next().await {
                if wakes.send(wake).await.is_err() {
                    return;
                }
            }
            log::warn!("the BlueZ signal streams all ended");
        },
        "reprise-bluetooth-signals",
    )
}

async fn subscribe(conn: &Connection) -> futures_util::stream::BoxStream<'static, Wake> {
    let mut rules = Vec::new();
    for member in SIGNALS {
        match zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(NAME)
            .and_then(|b| b.member(*member))
        {
            Ok(b) => rules.push(b.build().into()),
            Err(e) => log::warn!("no match rule for {member}: {e}"),
        }
    }
    reprisebus::signals(conn, NAME, rules)
        .await
        .map(|s| match s {
            reprisebus::Signal::OwnerChanged => Wake::Restarted,
            reprisebus::Signal::Message(_) => Wake::Bus,
        })
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_method_error_reads_through_plain_error() {
        let e = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.bluez.Error.NotReady").unwrap(),
            Some("Resource Not Ready".into()),
            zbus::Message::method_call("/", "X")
                .unwrap()
                .build(&())
                .unwrap(),
        );
        assert_eq!(
            error_of(&e),
            "Bluetooth is not ready. Make sure it is turned on."
        );
    }

    #[test]
    fn quiet_commands_do_not_raise_a_box() {
        assert!(quiet(&Cmd::StopDiscovery("/a".into())));
        assert!(quiet(&Cmd::CancelPair("/d".into())));
        assert!(!quiet(&Cmd::Disconnect("/d".into())));
    }
}
