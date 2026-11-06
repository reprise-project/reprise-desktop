//! `org.reprise.RemoteDesktop` on the session bus: the server's status for Settings, and
//! Disconnect, DisconnectClient and RegenerateCertificate. The bus connection lives on a thread
//! of its own.

use tokio::sync::{mpsc, oneshot, watch};
use zbus::connection::Builder;

pub const NAME: &str = "org.reprise.RemoteDesktop";
pub const PATH: &str = "/org/reprise/RemoteDesktop";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// The listening address, or `error: ` and why it cannot listen.
    pub listening: String,
    pub clients: Vec<Client>,
    /// The encoder the latest connection to change it uses, or the configured one with none.
    pub encoder: String,
    /// The `[remote_desktop] encoder` values that work here besides `auto`.
    pub encoders: Vec<String>,
    pub fingerprint: String,
    /// The monitors of lock mode's first viewer, primary first; empty in show mode.
    pub console_layout: Vec<crate::console::Monitor>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Client {
    pub address: String,
    pub user: String,
    /// Unix seconds.
    pub since: u64,
}

/// Which viewers a disconnect ends.
pub enum Request {
    All,
    /// The viewer at this address; the reply says whether there was one.
    Client(String, oneshot::Sender<bool>),
}

/// A request for a new certificate, answered once it is in use.
pub type Regenerate = oneshot::Sender<Result<(), String>>;

/// Serve `status` on the session bus until it is dropped, passing the bus's requests on.
pub fn spawn(
    status: watch::Receiver<Status>,
    disconnect: mpsc::UnboundedSender<Request>,
    regenerate: mpsc::UnboundedSender<Regenerate>,
) {
    let spawned = std::thread::Builder::new()
        .name("reprise-rdp-control".into())
        .spawn(move || {
            let served = zbus::block_on(async {
                let iface = RemoteDesktop {
                    status: status.clone(),
                    disconnect,
                    regenerate,
                };
                serve(Builder::session().name(NAME), iface, status).await
            });
            if let Err(e) = served {
                log::error!("{NAME} is not on the session bus: {e}");
            }
        });
    if let Err(e) = spawned {
        log::error!("no control thread: {e}");
    }
}

async fn serve(
    builder: Builder<'_>,
    iface: RemoteDesktop,
    mut status: watch::Receiver<Status>,
) -> zbus::Result<()> {
    let conn = builder.serve_at(PATH, iface).build().await?;
    let iface = conn
        .object_server()
        .interface::<_, RemoteDesktop>(PATH)
        .await?;
    let mut sent = status.borrow_and_update().clone();
    while status.changed().await.is_ok() {
        let now = status.borrow_and_update().clone();
        let emitter = iface.signal_emitter();
        let server = iface.get().await;
        if now.listening != sent.listening {
            signalled(server.listening_changed(emitter).await);
        }
        if now.clients != sent.clients {
            signalled(server.clients_changed(emitter).await);
        }
        if now.encoder != sent.encoder {
            signalled(server.encoder_changed(emitter).await);
        }
        if now.encoders != sent.encoders {
            signalled(server.encoders_changed(emitter).await);
        }
        if now.fingerprint != sent.fingerprint {
            signalled(server.fingerprint_changed(emitter).await);
        }
        if now.console_layout != sent.console_layout {
            signalled(server.console_layout_changed(emitter).await);
        }
        sent = now;
    }
    Ok(())
}

/// A failed signal leaves the property readable; the server keeps serving.
fn signalled(sent: zbus::Result<()>) {
    if let Err(e) = sent {
        log::warn!("{NAME}: a property change was not signalled: {e}");
    }
}

struct RemoteDesktop {
    status: watch::Receiver<Status>,
    disconnect: mpsc::UnboundedSender<Request>,
    regenerate: mpsc::UnboundedSender<Regenerate>,
}

#[zbus::interface(name = "org.reprise.RemoteDesktop")]
impl RemoteDesktop {
    #[zbus(property)]
    fn listening(&self) -> String {
        self.status.borrow().listening.clone()
    }

    /// Address, user and Unix seconds since the login, for each connected client.
    #[zbus(property)]
    fn clients(&self) -> Vec<(String, String, u64)> {
        (self.status.borrow().clients.iter())
            .map(|c| (c.address.clone(), c.user.clone(), c.since))
            .collect()
    }

    #[zbus(property)]
    fn encoder(&self) -> String {
        self.status.borrow().encoder.clone()
    }

    #[zbus(property)]
    fn encoders(&self) -> Vec<String> {
        self.status.borrow().encoders.clone()
    }

    #[zbus(property)]
    fn fingerprint(&self) -> String {
        self.status.borrow().fingerprint.clone()
    }

    /// x, y, width, height and scale percent of each monitor of lock mode's first viewer,
    /// primary first, for the portal to make its screens from; empty in show mode.
    #[zbus(property)]
    fn console_layout(&self) -> Vec<(i32, i32, i32, i32, u32)> {
        self.status.borrow().console_layout.clone()
    }

    /// Ends every connected client's session.
    fn disconnect(&self) {
        let _ = self.disconnect.send(Request::All);
    }

    /// Ends the session of the client at `address`, as listed in `Clients`.
    async fn disconnect_client(&self, address: String) -> zbus::fdo::Result<()> {
        let (tx, rx) = oneshot::channel();
        let none = || zbus::fdo::Error::Failed(format!("no client at {address}"));
        let _ = self.disconnect.send(Request::Client(address.clone(), tx));
        rx.await.unwrap_or(false).then_some(()).ok_or_else(none)
    }

    /// Replies once the new certificate is in use; clients connected now keep the old one.
    async fn regenerate_certificate(&self) -> zbus::fdo::Result<()> {
        let (tx, rx) = oneshot::channel();
        let gone = || zbus::fdo::Error::Failed("the server is stopping".into());
        self.regenerate.send(tx).map_err(|_| gone())?;
        rx.await
            .map_err(|_| gone())?
            .map_err(zbus::fdo::Error::Failed)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use futures_util::StreamExt;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use zbus::OwnedVariant;
    use zbus::fdo::PropertiesProxy;
    use zbus::names::InterfaceName;

    pub(crate) struct Bus(Child, pub(crate) String);

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// A private session bus.
    pub(crate) fn bus() -> Option<Bus> {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut line = String::new();
        BufReader::new(child.stdout.take()?)
            .read_line(&mut line)
            .ok()?;
        Some(Bus(child, line.trim().to_string()))
    }

    fn client() -> Client {
        Client {
            address: "192.0.2.7:50123".into(),
            user: "user".into(),
            since: 1_762_000_000,
        }
    }

    #[test]
    fn the_status_is_served_and_each_change_is_signalled() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let (status, rx) = watch::channel(Status {
            listening: "0.0.0.0:3389".into(),
            encoder: "RemoteFX (no H.264 encoder)".into(),
            encoders: vec!["software".into(), "none".into()],
            fingerprint: "AB:CD".into(),
            ..Status::default()
        });
        let (disconnect, mut disconnected) = mpsc::unbounded_channel();
        let (regenerate, mut regenerations) = mpsc::unbounded_channel::<Regenerate>();
        let address = bus.1.clone();
        let iface = RemoteDesktop {
            status: rx.clone(),
            disconnect,
            regenerate,
        };
        let server = std::thread::spawn(move || {
            zbus::block_on(async move {
                let builder = Builder::address(address.as_str()).name(NAME);
                serve(builder, iface, rx).await
            })
        });
        zbus::block_on(async move {
            let conn = Builder::address(bus.1.as_str()).build().await.unwrap();
            let props = PropertiesProxy::builder(&conn)
                .destination(NAME)
                .path(PATH)
                .build()
                .await
                .unwrap();
            let iface = InterfaceName::try_from(NAME).unwrap();
            let all = loop {
                match props.get_all(iface.clone()).await {
                    Ok(all) => break all,
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
                }
            };
            let text = |all: &HashMap<String, OwnedVariant>, k: &str| {
                String::try_from(all[k].try_clone().unwrap()).unwrap()
            };
            assert_eq!(text(&all, "Listening"), "0.0.0.0:3389");
            assert_eq!(text(&all, "Encoder"), "RemoteFX (no H.264 encoder)");
            assert_eq!(text(&all, "Fingerprint"), "AB:CD");
            assert_eq!(all["Clients"].value_signature(), "a(sst)");
            assert_eq!(all["ConsoleLayout"].value_signature(), "a(iiiiu)");
            assert_eq!(
                <Vec<String>>::try_from(all["Encoders"].try_clone().unwrap()).unwrap(),
                ["software", "none"]
            );

            let mut changes = props.receive_properties_changed().await.unwrap();
            let runtime = zruntime::SharedRuntime::current().unwrap();
            let mut next_change = async || {
                let next = runtime.timeout(std::time::Duration::from_secs(5), changes.next());
                next.await.expect("a change is signalled").unwrap()
            };
            status.send_modify(|s| s.clients = vec![client()]);
            let change = next_change().await;
            let args = change.args().unwrap();
            let changed: Vec<_> = args.changed_properties().keys().copied().collect();
            assert_eq!(changed, ["Clients"], "only what changed is signalled");
            let clients = <Vec<(String, String, u64)>>::try_from(
                args.changed_properties()["Clients"].try_to_owned().unwrap(),
            )
            .unwrap();
            assert_eq!(
                clients,
                [(
                    "192.0.2.7:50123".to_string(),
                    "user".to_string(),
                    1_762_000_000
                )]
            );

            status.send_modify(|s| s.listening = "error: Address already in use".into());
            let change = next_change().await;
            let args = change.args().unwrap();
            assert_eq!(
                args.changed_properties()
                    .keys()
                    .copied()
                    .collect::<Vec<_>>(),
                ["Listening"]
            );

            let layout = vec![(0, 0, 2560, 1440, 150), (2560, 0, 1920, 1080, 100)];
            status.send_modify(|s| s.console_layout = layout.clone());
            let change = next_change().await;
            let args = change.args().unwrap();
            let changed = args.changed_properties();
            assert_eq!(
                changed.keys().copied().collect::<Vec<_>>(),
                ["ConsoleLayout"]
            );
            let published = <Vec<(i32, i32, i32, i32, u32)>>::try_from(
                changed["ConsoleLayout"].try_to_owned().unwrap(),
            )
            .unwrap();
            assert_eq!(published, layout);

            conn.call_method(Some(NAME), PATH, Some(NAME), "Disconnect", &())
                .await
                .unwrap();
            assert!(matches!(disconnected.recv().await, Some(Request::All)));

            let answer = std::thread::spawn(move || {
                for found in [true, false] {
                    let Some(Request::Client(address, reply)) = disconnected.blocking_recv() else {
                        panic!("a DisconnectClient request");
                    };
                    assert_eq!(
                        address,
                        if found {
                            "192.0.2.7:50123"
                        } else {
                            "192.0.2.8:1"
                        }
                    );
                    reply.send(found).unwrap();
                }
            });
            let method = "DisconnectClient";
            (conn.call_method(Some(NAME), PATH, Some(NAME), method, &"192.0.2.7:50123"))
                .await
                .unwrap();
            let missed =
                (conn.call_method(Some(NAME), PATH, Some(NAME), method, &"192.0.2.8:1")).await;
            answer.join().unwrap();
            assert!(
                matches!(&missed, Err(zbus::Error::FDO(e))
                    if matches!(&**e, zbus::fdo::Error::Failed(m) if m == "no client at 192.0.2.8:1")),
                "{missed:?}"
            );

            let answer = std::thread::spawn(move || {
                let reply = regenerations.blocking_recv().unwrap();
                reply.send(Err("no room".into())).unwrap();
            });
            let refused = conn
                .call_method(Some(NAME), PATH, Some(NAME), "RegenerateCertificate", &())
                .await;
            answer.join().unwrap();
            assert!(
                matches!(&refused, Err(zbus::Error::FDO(e))
                    if matches!(&**e, zbus::fdo::Error::Failed(m) if m == "no room")),
                "{refused:?}"
            );
            drop(status);
        });
        server.join().unwrap().unwrap();
    }
}
