//! The `org.bluez.obex.Agent1` obexd asks before it takes a pushed file, registered while
//! `[bluetooth] receive_files` is on. docs/security.md covers what it refuses and where files go.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::stream::StreamExt;
use zbus::message::Header;
use zbus::{Connection, ObjectPath, OwnedObjectPath, OwnedVariant};

use super::bus::Poster;
use super::{BzMsg, Objects, Offer, Props, device_at, text};

pub const NAME: &str = "org.bluez.obex";
pub const PATH: &str = "/reprise/bluetooth/obex";
const MANAGER_PATH: &str = "/org/bluez/obex";
const MANAGER_IFACE: &str = "org.bluez.obex.AgentManager1";
const TRANSFER_IFACE: &str = "org.bluez.obex.Transfer1";
const SESSION_IFACE: &str = "org.bluez.obex.Session1";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const DBUS: &str = "org.freedesktop.DBus";

/// obexd stops waiting after 60 s and then takes the file into its own folder, so an unanswered
/// offer is refused before that.
const ANSWER_WITHIN: Duration = Duration::from_secs(50);

/// Every refusal is `Rejected`: obexd reads any other error, `Canceled` included, as no answer
/// and saves the file in its own folder.
#[derive(zbus::DBusError, Debug)]
#[zbus(prefix = "org.bluez.obex.Error")]
enum Error {
    Rejected(String),
}

impl From<zbus::Error> for Error {
    fn from(e: zbus::Error) -> Error {
        Error::Rejected(e.to_string())
    }
}

/// An accepted transfer, until obexd reports it complete or failed.
#[derive(Clone, Debug)]
struct Accepted {
    /// Where obexd writes it.
    incoming: PathBuf,
    /// The name the device gave it.
    name: String,
    sender: String,
}

#[derive(Default)]
struct Shared {
    accepted: Mutex<HashMap<String, Accepted>>,
    /// The transfer whose offer waits on the user; obexd's `Cancel` names none.
    waiting: Mutex<Option<String>>,
    receiving: AtomicBool,
    /// A `RegisterAgent` is out; the obexd it starts is not a restarted one.
    registering: AtomicBool,
    /// The obexd the agent is registered with, by unique name.
    registered: Mutex<Option<String>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

struct Agent {
    poster: Poster,
    /// The bus BlueZ is on, to look the sender up.
    bluez: Connection,
    shared: Arc<Shared>,
}

async fn props(conn: &Connection, path: &str, iface: &str) -> zbus::Result<Props> {
    conn.call_method(Some(NAME), path, Some(PROPERTIES), "GetAll", &(iface,))
        .await?
        .body()
        .deserialize::<Props>()
}

async fn check_caller(conn: &Connection, hdr: &Header<'_>) -> Result<(), Error> {
    let owner = reprisebus::name_owner(conn, NAME).await;
    let sender = hdr.sender().map(|s| s.as_str());
    if reprisebus::sent_by(sender, owner.as_deref()) {
        return Ok(());
    }
    log::warn!("refusing an OBEX agent call from {sender:?}, which is not {NAME}");
    Err(Error::Rejected(format!("only {NAME} may call the agent")))
}

/// How the prompt and balloons name a device: its alias, with the address when they differ.
fn sender_label(alias: &str, address: &str) -> String {
    if alias.is_empty() || alias == address {
        address.to_string()
    } else {
        format!("{alias} ({address})")
    }
}

impl Agent {
    /// The paired device at the far end of obexd's `session`, by its label, and obexd's root
    /// folder; None when BlueZ doesn't list the device as paired.
    async fn paired_sender(
        &self,
        conn: &Connection,
        session: &str,
    ) -> zbus::Result<Option<(String, PathBuf)>> {
        let s = props(conn, session, SESSION_IFACE).await?;
        let (Some(source), Some(dest), Some(root)) = (
            text(&s, "Source"),
            text(&s, "Destination"),
            text(&s, "Root"),
        ) else {
            return Ok(None);
        };
        let objects = self
            .bluez
            .call_method(
                Some(super::bus::NAME),
                "/",
                Some(OBJECT_MANAGER),
                "GetManagedObjects",
                &(),
            )
            .await?
            .body()
            .deserialize::<Objects>()?;
        Ok(device_at(&objects, &source, &dest)
            .filter(|d| d.paired)
            .map(|d| (sender_label(&d.alias, &d.address), PathBuf::from(root))))
    }

    /// The user's answer to `offer`, false when the offer is dropped or [`ANSWER_WITHIN`] passes.
    async fn ask(&self, mut offer: Offer) -> bool {
        let (done, wait) = async_channel::bounded(1);
        offer.done = Some(done);
        let transfer = offer.transfer.clone();
        *lock(&self.shared.waiting) = Some(transfer.clone());
        self.poster.post(BzMsg::Offer(offer));
        let answer = match zruntime::SharedRuntime::current() {
            Ok(rt) => matches!(rt.timeout(ANSWER_WITHIN, wait.recv()).await, Ok(Ok(true))),
            Err(e) => {
                log::warn!("no timer to bound the offer with ({e}); refusing it");
                false
            }
        };
        lock(&self.shared.waiting).take();
        self.poster.post(BzMsg::OfferGone(transfer));
        answer
    }
}

#[zbus::interface(name = "org.bluez.obex.Agent1")]
impl Agent {
    async fn release(
        &self,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<(), Error> {
        check_caller(conn, &hdr).await?;
        lock(&self.shared.registered).take();
        log::info!("obexd released the file transfer agent");
        Ok(())
    }

    /// The name obexd writes the file under in its root folder.
    async fn authorize_push(
        &self,
        transfer: OwnedObjectPath,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<String, Error> {
        check_caller(conn, &hdr).await?;
        if !self.shared.receiving.load(Ordering::Relaxed) {
            return Err(Error::Rejected("receiving files is turned off".into()));
        }
        let t = props(conn, transfer.as_str(), TRANSFER_IFACE).await?;
        let name = text(&t, "Name").unwrap_or_default();
        let size = t.get("Size").and_then(|v| u64::try_from(v.clone()).ok());
        let session = t
            .get("Session")
            .and_then(|v| OwnedObjectPath::try_from(v.clone()).ok())
            .ok_or_else(|| Error::Rejected("the transfer names no session".into()))?;
        let Some((sender, root)) = self.paired_sender(conn, session.as_str()).await? else {
            log::warn!("refusing {name:?} from a device that is not paired");
            return Err(Error::Rejected("only paired devices may send files".into()));
        };
        let offer = Offer {
            transfer: transfer.to_string(),
            sender: sender.clone(),
            name: name.clone(),
            size,
            done: None,
        };
        if !self.ask(offer).await {
            return Err(Error::Rejected("declined".into()));
        }
        let reply = crate::receive::incoming_name(transfer.as_str());
        log::info!("receiving {name:?} from {sender}");
        let incoming = root.join(&reply);
        lock(&self.shared.accepted).insert(
            transfer.to_string(),
            Accepted {
                incoming,
                name,
                sender,
            },
        );
        Ok(reply)
    }

    async fn cancel(
        &self,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<(), Error> {
        check_caller(conn, &hdr).await?;
        if let Some(t) = lock(&self.shared.waiting).clone() {
            self.poster.post(BzMsg::OfferGone(t));
        }
        Ok(())
    }
}

/// The agent on the session bus, and whether obexd is there to call it.
#[derive(Clone)]
pub struct Obex {
    conn: Connection,
    shared: Arc<Shared>,
}

/// Whether `name` has an owner or a service file that starts one.
async fn present(conn: &Connection, name: &str) -> bool {
    if reprisebus::name_owner(conn, name).await.is_some() {
        return true;
    }
    conn.call_method(
        Some(DBUS),
        "/org/freedesktop/DBus",
        Some(DBUS),
        "ListActivatableNames",
        &(),
    )
    .await
    .and_then(|m| m.body().deserialize::<Vec<String>>())
    .is_ok_and(|names| names.iter().any(|n| n == name))
}

/// Serves the agent on the session bus, with BlueZ read off `bluez`, and posts whether obexd is
/// installed. None without a session bus.
pub(super) async fn start(bluez: &Connection, poster: Poster) -> Option<Obex> {
    match zbus::connection::Builder::session().build().await {
        Ok(c) => serve(c, bluez.clone(), poster).await,
        Err(e) => {
            log::info!("no session bus ({e}); files cannot be received");
            poster.post(BzMsg::Obex(false));
            None
        }
    }
}

async fn serve(conn: Connection, bluez: Connection, poster: Poster) -> Option<Obex> {
    let shared = Arc::new(Shared::default());
    let agent = Agent {
        poster: poster.clone(),
        bluez,
        shared: shared.clone(),
    };
    if let Err(e) = conn.object_server().at(PATH, agent).await {
        log::warn!("cannot serve the file transfer agent: {e}");
        poster.post(BzMsg::Obex(false));
        return None;
    }
    let obex = Obex { conn, shared };
    poster.post(BzMsg::Obex(present(&obex.conn, NAME).await));
    obex.follow(poster).await;
    Some(obex)
}

impl Obex {
    /// Registers the agent with obexd, starting it, or unregisters it.
    pub async fn receive(&self, on: bool) {
        self.shared.receiving.store(on, Ordering::Relaxed);
        if on {
            self.register().await;
        } else if lock(&self.shared.registered).take().is_some() {
            let path = ObjectPath::from_static_str_unchecked(PATH);
            let r = self
                .conn
                .call_method(
                    Some(NAME),
                    MANAGER_PATH,
                    Some(MANAGER_IFACE),
                    "UnregisterAgent",
                    &(&path,),
                )
                .await;
            if let Err(e) = r {
                log::info!("obexd did not drop the file transfer agent ({e})");
            }
        }
    }

    async fn register(&self) {
        let path = ObjectPath::from_static_str_unchecked(PATH);
        self.shared.registering.store(true, Ordering::Relaxed);
        let r = self
            .conn
            .call_method(
                Some(NAME),
                MANAGER_PATH,
                Some(MANAGER_IFACE),
                "RegisterAgent",
                &(&path,),
            )
            .await;
        match r {
            Ok(_) => {
                *lock(&self.shared.registered) = reprisebus::name_owner(&self.conn, NAME).await;
                log::info!("registered as the file transfer agent");
            }
            Err(e) => log::info!("obexd did not take the file transfer agent ({e})"),
        }
        self.shared.registering.store(false, Ordering::Relaxed);
    }

    /// Follows the accepted transfers to their end, and a restarted obexd, which forgets them
    /// and the agent. A transfer that never started is removed with no status of its own.
    async fn follow(&self, poster: Poster) {
        let status = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(NAME)
            .interface(PROPERTIES)
            .member("PropertiesChanged")
            .add_arg(TRANSFER_IFACE)
            .build();
        let removed = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(NAME)
            .interface(OBJECT_MANAGER)
            .member("InterfacesRemoved")
            .build();
        let mut rules = Vec::new();
        for rule in [status, removed] {
            match rule {
                Ok(r) => rules.push(r.into()),
                Err(e) => log::warn!("no match rule for transfers: {e}"),
            }
        }
        let mut signals = reprisebus::signals(&self.conn, NAME, rules).await;
        let me = self.clone();
        zruntime::spawn(async move {
            while let Some(s) = signals.next().await {
                match s {
                    reprisebus::Signal::Message(m) => me.changed(&m, &poster),
                    reprisebus::Signal::OwnerChanged => me.restarted(&poster).await,
                }
            }
        })
        .detach();
    }

    fn changed(&self, m: &zbus::Message, poster: &Poster) {
        let hdr = m.header();
        if hdr.member().is_some_and(|n| n == "InterfacesRemoved") {
            let Ok((path, ifaces)) = m.body().deserialize::<(OwnedObjectPath, Vec<String>)>()
            else {
                return;
            };
            let gone = ifaces.iter().any(|i| i == TRANSFER_IFACE);
            let a = gone.then(|| lock(&self.shared.accepted).remove(path.as_str()));
            if let Some(a) = a.flatten() {
                finish(a, false, poster);
            }
            return;
        }
        let Some(path) = hdr.path().map(|p| p.to_string()) else {
            return;
        };
        let Ok((_, changed, _)) = m
            .body()
            .deserialize::<(String, HashMap<String, OwnedVariant>, Vec<String>)>()
        else {
            return;
        };
        let status = text(&changed, "Status");
        let Some(done) = status
            .as_deref()
            .filter(|s| matches!(*s, "complete" | "error"))
        else {
            return;
        };
        let Some(a) = lock(&self.shared.accepted).remove(&path) else {
            return;
        };
        finish(a, done == "complete", poster);
    }

    async fn restarted(&self, poster: &Poster) {
        let lost: Vec<Accepted> = lock(&self.shared.accepted)
            .drain()
            .map(|(_, a)| a)
            .collect();
        for a in lost {
            finish(a, false, poster);
        }
        // An obexd that went away is started again, so the computer keeps offering Object Push.
        let owner = reprisebus::name_owner(&self.conn, NAME).await;
        let fresh = (owner.is_none() || *lock(&self.shared.registered) != owner)
            && !self.shared.registering.load(Ordering::Relaxed);
        if fresh && self.shared.receiving.load(Ordering::Relaxed) {
            self.register().await;
        }
    }
}

/// Moves a complete transfer into Downloads and posts it; a failed one's partial file goes.
fn finish(a: Accepted, complete: bool, poster: &Poster) {
    if complete {
        let landed = match crate::receive::downloads() {
            Some(dir) => crate::receive::land(&a.incoming, &dir, &a.name),
            None => Err(std::io::Error::other("no home folder")),
        };
        match landed {
            Ok(path) => {
                log::info!("received {}", path.display());
                poster.post(BzMsg::Received {
                    path,
                    sender: a.sender,
                });
                return;
            }
            Err(e) => log::warn!("cannot move {} into Downloads: {e}", a.incoming.display()),
        }
    } else {
        log::warn!(
            "{:?} from {} stopped before it was complete",
            a.name,
            a.sender
        );
    }
    let _ = std::fs::remove_file(&a.incoming);
    poster.post(BzMsg::NotReceived {
        name: a.name,
        sender: a.sender,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;

    struct Bus(Child, String);

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn bus() -> Option<Bus> {
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

    async fn connect(address: &str) -> Connection {
        zbus::connection::Builder::address(address)
            .build()
            .await
            .expect("connect")
    }

    const ADAPTER: &str = "00:AA:01:00:00:00";
    const PAIRED: &str = "AA:BB:CC:DD:EE:01";
    const STRANGER: &str = "AA:BB:CC:DD:EE:02";
    const SESSION: &str = "/org/bluez/obex/server/session0";
    /// An offer is posted before the agent replies, so one missing past the reply never came.
    const SHORT: Duration = Duration::from_millis(300);
    const LONG: Duration = Duration::from_secs(5);
    const TRANSFER: &str = "/org/bluez/obex/server/session0/transfer0";

    struct FakeAdapter;

    #[zbus::interface(name = "org.bluez.Adapter1")]
    impl FakeAdapter {
        #[zbus(property)]
        fn address(&self) -> String {
            ADAPTER.into()
        }
    }

    struct FakeDevice {
        address: &'static str,
        paired: bool,
    }

    #[zbus::interface(name = "org.bluez.Device1")]
    impl FakeDevice {
        #[zbus(property)]
        fn address(&self) -> String {
            self.address.into()
        }
        #[zbus(property)]
        fn alias(&self) -> String {
            "Pixel".into()
        }
        #[zbus(property)]
        fn paired(&self) -> bool {
            self.paired
        }
    }

    struct FakeSession {
        destination: &'static str,
        root: String,
    }

    #[zbus::interface(name = "org.bluez.obex.Session1")]
    impl FakeSession {
        #[zbus(property)]
        fn source(&self) -> String {
            ADAPTER.into()
        }
        #[zbus(property)]
        fn destination(&self) -> String {
            self.destination.into()
        }
        #[zbus(property)]
        fn root(&self) -> String {
            self.root.clone()
        }
    }

    /// The fake obexd's root folder for a transfer of `name`, one per test.
    fn root_for(name: &str) -> PathBuf {
        let home = crate::receive::downloads().unwrap();
        home.parent()
            .unwrap()
            .join(".cache")
            .join(format!("obexd-{}", crate::receive::safe_name(name)))
    }

    struct FakeTransfer {
        name: String,
        status: String,
    }

    #[zbus::interface(name = "org.bluez.obex.Transfer1")]
    impl FakeTransfer {
        #[zbus(property)]
        fn name(&self) -> String {
            self.name.clone()
        }
        #[zbus(property)]
        fn size(&self) -> u64 {
            1234
        }
        #[zbus(property)]
        fn session(&self) -> OwnedObjectPath {
            OwnedObjectPath::try_from(SESSION).unwrap()
        }
        #[zbus(property)]
        fn status(&self) -> String {
            self.status.clone()
        }
    }

    struct FakeManager(mpsc::Sender<String>);

    #[zbus::interface(name = "org.bluez.obex.AgentManager1")]
    impl FakeManager {
        fn register_agent(&self, agent: OwnedObjectPath) {
            let _ = self.0.send(agent.to_string());
        }
    }

    /// BlueZ with one adapter, a paired device and an unpaired one, and an obexd whose one
    /// transfer of `name` comes from `from`.
    async fn fakes(
        address: &str,
        name: &str,
        from: &'static str,
    ) -> (Connection, Connection, mpsc::Receiver<String>) {
        let bluez = connect(address).await;
        let server = bluez.object_server();
        server.at("/", zbus::fdo::ObjectManager).await.unwrap();
        server.at("/org/bluez/hci0", FakeAdapter).await.unwrap();
        for (address, paired) in [(PAIRED, true), (STRANGER, false)] {
            let path = format!("/org/bluez/hci0/dev_{}", address.replace(':', "_"));
            server
                .at(path, FakeDevice { address, paired })
                .await
                .unwrap();
        }
        bluez.request_name(super::super::bus::NAME).await.unwrap();
        let obexd = connect(address).await;
        let (tx, registered) = mpsc::channel();
        let server = obexd.object_server();
        server.at("/", zbus::fdo::ObjectManager).await.unwrap();
        server.at(MANAGER_PATH, FakeManager(tx)).await.unwrap();
        let root = root_for(name);
        std::fs::create_dir_all(&root).unwrap();
        let session = FakeSession {
            destination: from,
            root: root.to_string_lossy().into_owned(),
        };
        server.at(SESSION, session).await.unwrap();
        let transfer = FakeTransfer {
            name: name.into(),
            status: "queued".into(),
        };
        server.at(TRANSFER, transfer).await.unwrap();
        obexd.request_name(NAME).await.unwrap();
        (bluez, obexd, registered)
    }

    /// Answers every offer `accept`, and hands on what the agent posts.
    fn user(rx: mpsc::Receiver<BzMsg>, accept: bool) -> mpsc::Receiver<BzMsg> {
        let (tx, out) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(mut m) = rx.recv() {
                if let BzMsg::Offer(o) = &mut m {
                    o.answer(accept);
                }
                if tx.send(m).is_err() {
                    return;
                }
            }
        });
        out
    }

    fn next(
        rx: &mpsc::Receiver<BzMsg>,
        wait: Duration,
        what: impl Fn(&BzMsg) -> bool,
    ) -> Option<BzMsg> {
        let until = std::time::Instant::now() + wait;
        while let Some(left) = until.checked_duration_since(std::time::Instant::now()) {
            match rx.recv_timeout(left) {
                Ok(m) if what(&m) => return Some(m),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
        None
    }

    async fn push(obexd: &Connection, agent: &Connection) -> zbus::Result<String> {
        let path = ObjectPath::try_from(TRANSFER).unwrap();
        obexd
            .call_method(
                agent.unique_name().map(|n| n.as_str()),
                PATH,
                Some("org.bluez.obex.Agent1"),
                "AuthorizePush",
                &(&path,),
            )
            .await?
            .body()
            .deserialize::<String>()
    }

    fn rejected(r: zbus::Result<String>) -> bool {
        reprisebus::error::name(&r.unwrap_err()).as_deref() == Some("org.bluez.obex.Error.Rejected")
    }

    /// Runs `body` against the fakes and an agent receiving files; the user answers `accept`.
    /// The bus comes back too: the agent still reads signals after `body` returns.
    fn with_agent<T>(
        name: &str,
        from: &'static str,
        accept: bool,
        body: impl AsyncFnOnce(&str, &Connection, &Connection, &Obex) -> T,
    ) -> Option<(T, mpsc::Receiver<BzMsg>, mpsc::Receiver<String>, Bus)> {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return None;
        };
        let (poster, rx) = Poster::channel(repriseapp::Waker::noop());
        let posted = user(rx, accept);
        let (out, posted, registered) = zbus::block_on(async {
            let (_bluez, obexd, registered) = fakes(&bus.1, name, from).await;
            let conn = connect(&bus.1).await;
            let obex = serve(conn.clone(), conn.clone(), poster)
                .await
                .expect("served");
            obex.receive(true).await;
            let out = body(&bus.1, &obexd, &conn, &obex).await;
            (out, posted, registered)
        });
        Some((out, posted, registered, bus))
    }

    /// obexd writing `reply` into its root folder for a transfer of `name`, then ending it
    /// `complete` (a Status change) or not (the transfer object goes without one).
    async fn obexd_ends(obexd: &Connection, name: &str, reply: &str, complete: bool) {
        std::fs::write(root_for(name).join(reply), "picture").unwrap();
        let server = obexd.object_server();
        if !complete {
            server.remove::<FakeTransfer, _>(TRANSFER).await.unwrap();
            return;
        }
        let iface = server.interface::<_, FakeTransfer>(TRANSFER).await.unwrap();
        iface.get_mut().await.status = "complete".into();
        iface
            .get()
            .await
            .status_changed(iface.signal_emitter())
            .await
            .unwrap();
    }

    #[test]
    fn an_accepted_file_from_a_paired_device_moves_to_downloads_once_complete() {
        let name = ".holiday photo.jpg";
        let got = with_agent(name, PAIRED, true, async |_, obexd, conn, _| {
            let reply = push(obexd, conn).await.expect("accepted");
            obexd_ends(obexd, name, &reply, true).await;
            reply
        });
        let Some((reply, posted, registered, _bus)) = got else {
            return;
        };
        assert_eq!(registered.try_recv().as_deref(), Ok(PATH));
        assert_eq!(
            reply, ".reprise-session0-transfer0",
            "a bare name: obexd's root folder"
        );
        assert!(matches!(
            next(&posted, LONG, |m| matches!(m, BzMsg::Obex(_))),
            Some(BzMsg::Obex(true))
        ));
        let offer = next(&posted, LONG, |m| matches!(m, BzMsg::Offer(_)));
        assert!(
            matches!(&offer, Some(BzMsg::Offer(o)) if o.sender == "Pixel (AA:BB:CC:DD:EE:01)" && o.size == Some(1234)),
            "{offer:?}"
        );
        let want = crate::receive::downloads()
            .unwrap()
            .join("holiday photo.jpg");
        let done = next(&posted, LONG, |m| matches!(m, BzMsg::Received { .. }));
        assert!(
            matches!(&done, Some(BzMsg::Received { path, .. }) if *path == want),
            "{done:?}"
        );
        assert_eq!(std::fs::read_to_string(&want).unwrap(), "picture");
        assert!(!root_for(name).join(&reply).exists(), "moved, not copied");
    }

    #[test]
    fn a_transfer_gone_without_completing_is_not_received_and_its_file_goes() {
        let name = "cut short.jpg";
        let got = with_agent(name, PAIRED, true, async |_, obexd, conn, _| {
            let reply = push(obexd, conn).await.expect("accepted");
            obexd_ends(obexd, name, &reply, false).await;
            reply
        });
        let Some((reply, posted, _, _bus)) = got else {
            return;
        };
        let done = next(&posted, LONG, |m| matches!(m, BzMsg::NotReceived { .. }));
        assert!(
            matches!(&done, Some(BzMsg::NotReceived { name: n, .. }) if n == name),
            "{done:?}"
        );
        assert!(!root_for(name).join(&reply).exists());
        assert!(!crate::receive::downloads().unwrap().join(name).exists());
    }

    #[test]
    fn an_unpaired_device_is_refused_without_asking() {
        let got = with_agent("stranger.txt", STRANGER, true, async |_, obexd, conn, _| {
            push(obexd, conn).await
        });
        let Some((reply, posted, _, _bus)) = got else {
            return;
        };
        assert!(rejected(reply));
        assert!(next(&posted, SHORT, |m| matches!(m, BzMsg::Offer(_))).is_none());
    }

    #[test]
    fn a_declined_file_is_refused() {
        let got = with_agent("declined.txt", PAIRED, false, async |_, obexd, conn, _| {
            push(obexd, conn).await
        });
        let Some((reply, _, _, _bus)) = got else {
            return;
        };
        assert!(rejected(reply));
    }

    #[test]
    fn only_obexd_may_ask_and_only_while_receiving() {
        let got = with_agent(
            "other.txt",
            PAIRED,
            true,
            async |address, obexd, conn, obex| {
                let other = connect(address).await;
                let stranger = push(&other, conn).await;
                obex.receive(false).await;
                let off = push(obexd, conn).await;
                (stranger, off)
            },
        );
        let Some(((stranger, off), posted, _, _bus)) = got else {
            return;
        };
        assert!(rejected(stranger), "not the owner of {NAME}");
        assert!(rejected(off), "receiving turned off");
        assert!(next(&posted, SHORT, |m| matches!(m, BzMsg::Offer(_))).is_none());
    }
}
