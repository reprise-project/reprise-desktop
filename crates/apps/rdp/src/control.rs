//! `org.reprise.RemoteDesktop` on the session bus, on a thread of its own: the server's status
//! for Settings, and Disconnect, SessionEnding, DisconnectClient and RegenerateCertificate.

use tokio::sync::{mpsc, oneshot, watch};
use zbus::connection::Builder;
use zbus::message::Header;

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

/// Which viewers a disconnect ends, and what each is told.
pub enum Request {
    All,
    /// Every viewer, each told the session logged off.
    SessionEnding,
    /// The viewer at this address; the reply says whether there was one.
    Client(String, oneshot::Sender<bool>),
}

/// A request for a new certificate, answered once it is in use.
pub type Regenerate = oneshot::Sender<Result<(), String>>;

/// Which server serves the bus.
pub enum Server {
    /// The user's own server, which makes its certificates.
    Own(mpsc::UnboundedSender<Regenerate>),
    /// reprise-login's hand-over server: it holds [`NAME`] alone, and the login manager owns the
    /// certificate.
    HandOver,
}

/// The pid of the compositor the server serves; it may block while the display comes up.
pub type Compositor = fn() -> Result<u32, String>;

/// Serve `status` on the session bus until it is dropped, passing the bus's requests on. The
/// receiver hears once whether [`NAME`] was claimed; neither server gives it up, and the
/// hand-over server claims it only from nobody.
pub fn spawn(
    status: watch::Receiver<Status>,
    disconnect: mpsc::UnboundedSender<Request>,
    server: Server,
) -> std::sync::mpsc::Receiver<zbus::Result<()>> {
    let compositor = crate::wayland::compositor_pid;
    spawn_on(Builder::session(), status, disconnect, server, compositor)
}

fn spawn_on(
    builder: Builder<'static>,
    status: watch::Receiver<Status>,
    disconnect: mpsc::UnboundedSender<Request>,
    server: Server,
    compositor: Compositor,
) -> std::sync::mpsc::Receiver<zbus::Result<()>> {
    let (alone, regenerate) = match server {
        Server::Own(regenerate) => (false, Some(regenerate)),
        Server::HandOver => (true, None),
    };
    let (claimed, claim) = std::sync::mpsc::sync_channel(1);
    let spawned = std::thread::Builder::new()
        .name("reprise-rdp-control".into())
        .spawn(move || {
            let served = zbus::block_on(async {
                let iface = RemoteDesktop {
                    status: status.clone(),
                    disconnect,
                    regenerate,
                    compositor,
                };
                let builder = (builder.name(NAME))
                    .allow_name_replacements(false)
                    .replace_existing_names(!alone);
                serve(builder, iface, status, claimed).await
            });
            if let Err(e) = served {
                log::error!("{NAME} is not on the session bus: {e}");
            }
        });
    if let Err(e) = spawned {
        log::error!("no control thread: {e}");
    }
    claim
}

async fn serve(
    builder: Builder<'_>,
    iface: RemoteDesktop,
    mut status: watch::Receiver<Status>,
    claimed: std::sync::mpsc::SyncSender<zbus::Result<()>>,
) -> zbus::Result<()> {
    let built = builder.serve_at(PATH, iface).build().await;
    let _ = claimed.send(built.as_ref().map(|_| ()).map_err(Clone::clone));
    let conn = built?;
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
            signalled(server.clients_invalidate(emitter).await);
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
    /// None on the hand-over server.
    regenerate: Option<mpsc::UnboundedSender<Regenerate>>,
    compositor: Compositor,
}

#[zbus::interface(name = "org.reprise.RemoteDesktop")]
impl RemoteDesktop {
    #[zbus(property)]
    fn listening(&self) -> String {
        self.status.borrow().listening.clone()
    }

    /// Address, user and Unix seconds since the login, for each connected client. A change is
    /// signalled without the value, which a sandboxed peer can't read.
    #[zbus(property(emits_changed_signal = "invalidates"))]
    async fn clients(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: Option<Header<'_>>,
    ) -> zbus::fdo::Result<Vec<(String, String, u64)>> {
        let header = header.ok_or_else(|| zbus::fdo::Error::AccessDenied("no sender".into()))?;
        unsandboxed(conn, &header).await?;
        Ok((self.status.borrow().clients.iter())
            .map(|c| (c.address.clone(), c.user.clone(), c.since))
            .collect())
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
    async fn disconnect(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<()> {
        unsandboxed(conn, &header).await?;
        let _ = self.disconnect.send(Request::All);
        Ok(())
    }

    /// Ends every client, telling each the session logged off. Only the compositor the server
    /// serves may call it: the user's other Reprise sessions share the bus.
    async fn session_ending(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<()> {
        unsandboxed(conn, &header).await?;
        from_compositor(conn, &header, self.compositor).await?;
        let _ = self.disconnect.send(Request::SessionEnding);
        Ok(())
    }

    /// [`Self::session_ending`] for the computer turning off or restarting, which ends every
    /// session: any Reprise session of the user may call it.
    async fn power_ending(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<()> {
        unsandboxed(conn, &header).await?;
        let _ = self.disconnect.send(Request::SessionEnding);
        Ok(())
    }

    /// Ends the session of the client at `address`, as listed in `Clients`.
    async fn disconnect_client(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
        address: String,
    ) -> zbus::fdo::Result<()> {
        unsandboxed(conn, &header).await?;
        let (tx, rx) = oneshot::channel();
        let none = || zbus::fdo::Error::Failed(format!("no client at {address}"));
        let _ = self.disconnect.send(Request::Client(address.clone(), tx));
        rx.await.unwrap_or(false).then_some(()).ok_or_else(none)
    }

    /// Replies once the new certificate is in use; clients connected now keep the old one. The
    /// hand-over server refuses it.
    async fn regenerate_certificate(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<()> {
        unsandboxed(conn, &header).await?;
        let (tx, rx) = oneshot::channel();
        let gone = || zbus::fdo::Error::Failed("the server is stopping".into());
        let Some(regenerate) = &self.regenerate else {
            let owned = "the login manager owns the certificate";
            return Err(zbus::fdo::Error::Failed(owned.into()));
        };
        regenerate.send(tx).map_err(|_| gone())?;
        rx.await
            .map_err(|_| gone())?
            .map_err(zbus::fdo::Error::Failed)
    }
}

/// Refuses a peer that is not the process `compositor` names.
async fn from_compositor(
    conn: &zbus::Connection,
    header: &Header<'_>,
    compositor: Compositor,
) -> zbus::fdo::Result<()> {
    let sender =
        (header.sender()).ok_or_else(|| zbus::fdo::Error::AccessDenied("no sender".into()))?;
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || tx.send(compositor()));
    let pid = (rx.await)
        .unwrap_or_else(|_| Err("no answer".into()))
        .map_err(|e| zbus::fdo::Error::Failed(format!("the compositor: {e}")))?;
    match reprisebus::peer::is_process(conn, sender, pid).await {
        Ok(true) => Ok(()),
        Ok(false) => {
            log::info!(
                "{NAME}: {sender} is not the compositor of this server's session; its clients stay"
            );
            Err(zbus::fdo::Error::AccessDenied(
                "not the compositor of this server's session".into(),
            ))
        }
        Err(e) => Err(zbus::fdo::Error::Failed(format!(
            "the process behind {sender}: {e}"
        ))),
    }
}

/// Refuses a peer in a Flatpak sandbox: one whose app id is this name's may reach the bus name,
/// and the process the bus reports is its bus proxy's, which runs in the sandbox too.
async fn unsandboxed(conn: &zbus::Connection, header: &Header<'_>) -> zbus::fdo::Result<()> {
    let refused = || zbus::fdo::Error::AccessDenied("not from a sandbox".into());
    let sender = header.sender().ok_or_else(refused)?;
    match reprisebus::peer::sandboxed(conn, sender).await {
        false => Ok(()),
        true => {
            log::warn!("{NAME}: refused a call from sandboxed peer {sender}");
            Err(refused())
        }
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
            regenerate: Some(regenerate),
            compositor: this_process,
        };
        let server = std::thread::spawn(move || {
            zbus::block_on(async move {
                let builder = Builder::address(address.as_str()).name(NAME);
                let (claimed, _claim) = std::sync::mpsc::sync_channel(1);
                serve(builder, iface, rx, claimed).await
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
            assert!(
                args.changed_properties().is_empty(),
                "no addresses broadcast"
            );
            assert_eq!(
                args.invalidated_properties()[..],
                ["Clients"],
                "only what changed is signalled"
            );
            let clients = <Vec<(String, String, u64)>>::try_from(
                props.get(iface.clone(), "Clients").await.unwrap(),
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

    #[test]
    fn session_ending_asks_to_end_every_client() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let (status, rx) = watch::channel(Status::default());
        let (disconnect, mut disconnected) = mpsc::unbounded_channel();
        let builder = Builder::address(bus.1.as_str());
        let claim = spawn_on(builder, rx, disconnect, Server::HandOver, this_process);
        claim.recv().expect("the claim is told").unwrap();
        zbus::block_on(async {
            let conn = Builder::address(bus.1.as_str()).build().await.unwrap();
            conn.call_method(Some(NAME), PATH, Some(NAME), "SessionEnding", &())
                .await
                .unwrap();
        });
        assert!(matches!(
            disconnected.blocking_recv(),
            Some(Request::SessionEnding)
        ));
        drop(status);
    }

    #[test]
    fn a_session_ending_from_another_session_leaves_every_client_connected() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let (status, rx) = watch::channel(Status::default());
        let (disconnect, mut disconnected) = mpsc::unbounded_channel();
        let builder = Builder::address(bus.1.as_str());
        let elsewhere: Compositor = || Ok(1);
        let claim = spawn_on(builder, rx, disconnect, Server::HandOver, elsewhere);
        claim.recv().expect("the claim is told").unwrap();
        let refused = zbus::block_on(async {
            let conn = Builder::address(bus.1.as_str()).build().await.unwrap();
            conn.call_method(Some(NAME), PATH, Some(NAME), "SessionEnding", &())
                .await
        });
        assert!(
            matches!(&refused, Err(zbus::Error::FDO(e))
                if matches!(&**e, zbus::fdo::Error::AccessDenied(_))),
            "{refused:?}"
        );
        assert!(disconnected.try_recv().is_err(), "no client is ended");
        zbus::block_on(async {
            let conn = Builder::address(bus.1.as_str()).build().await.unwrap();
            conn.call_method(Some(NAME), PATH, Some(NAME), "PowerEnding", &())
                .await
                .unwrap();
        });
        assert!(
            matches!(disconnected.blocking_recv(), Some(Request::SessionEnding)),
            "a Turn Off from that session ends every client"
        );
        drop(status);
    }

    /// The test process stands in for the compositor.
    fn this_process() -> Result<u32, String> {
        Ok(std::process::id())
    }

    fn spawned(bus: &Bus, server: Server) -> (watch::Sender<Status>, zbus::Result<()>) {
        let (status, rx) = watch::channel(Status::default());
        let (disconnect, _) = mpsc::unbounded_channel();
        let builder = Builder::address(bus.1.as_str());
        let claim = spawn_on(builder, rx, disconnect, server, this_process);
        (status, claim.recv().expect("the claim is told"))
    }

    #[test]
    fn a_hand_over_server_claims_the_name_alone_or_not_at_all() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let others =
            |bus: &Bus| zbus::block_on(Builder::address(bus.1.as_str()).name(NAME).build());
        let other = others(&bus).unwrap();
        let (_status, claimed) = spawned(&bus, Server::HandOver);
        assert!(
            matches!(claimed, Err(zbus::Error::NameTaken)),
            "another server's name is not taken from it: {claimed:?}"
        );
        zbus::block_on(other.release_name(NAME)).unwrap();

        let (status, claimed) = spawned(&bus, Server::HandOver);
        assert!(claimed.is_ok(), "{claimed:?}");
        assert!(
            matches!(others(&bus), Err(zbus::Error::NameTaken)),
            "nor is the hand-over server's given up to another"
        );
        drop(status);
    }

    #[test]
    fn the_own_server_takes_the_name_and_never_gives_it_up() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let (regenerate, _) = mpsc::unbounded_channel();
        let (status, claimed) = spawned(&bus, Server::Own(regenerate));
        claimed.unwrap();
        let replaced = zbus::block_on(
            Builder::address(bus.1.as_str())
                .name(NAME)
                .replace_existing_names(true)
                .build(),
        );
        assert!(
            matches!(replaced, Err(zbus::Error::NameTaken)),
            "{replaced:?}"
        );
        drop(status);
    }

    #[test]
    fn a_hand_over_server_leaves_the_certificate_to_the_login_manager() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let (status, claimed) = spawned(&bus, Server::HandOver);
        claimed.unwrap();
        let refused = zbus::block_on(async {
            let conn = Builder::address(bus.1.as_str()).build().await.unwrap();
            let method = "RegenerateCertificate";
            conn.call_method(Some(NAME), PATH, Some(NAME), method, &())
                .await
        });
        assert!(
            matches!(&refused, Err(zbus::Error::FDO(e))
                if matches!(&**e, zbus::fdo::Error::Failed(m)
                    if m == "the login manager owns the certificate")),
            "{refused:?}"
        );
        drop(status);
    }
}
