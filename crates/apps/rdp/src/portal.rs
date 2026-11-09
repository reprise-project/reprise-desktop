//! A connection's RemoteDesktop portal session (keyboard, pointer, the shared outputs cast onto
//! it), on a bus connection in its own thread that callers reach through a channel.

use futures_util::StreamExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{LazyLock, OnceLock};
use std::time::Duration;
use zbus::zvariant::{DeserializeDict, DynamicType, Type, Variant};
use zbus::{Connection, MessageStream, OwnedObjectPath, OwnedVariant};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const REMOTE: &str = "org.freedesktop.portal.RemoteDesktop";
const SCREENCAST: &str = "org.freedesktop.portal.ScreenCast";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SESSION: &str = "org.freedesktop.portal.Session";

const KEYBOARD: u32 = 1;
const POINTER: u32 = 2;
const MONITOR: u32 = 1;
const VIRTUAL: u32 = 4;
const CURSOR_METADATA: u32 = 4;
/// How long a request may go unanswered: no dialog is expected, so a silent portal is stuck.
const ANSWER: Duration = Duration::from_secs(30);

/// A started session: the shared screens, the EIS socket and the allowed input devices.
/// Dropping it closes the session.
pub struct Grant {
    /// One PipeWire connection per screen, in `screens` order.
    pw_fds: Vec<OwnedFd>,
    /// In the portal's screen order, the primary first.
    pub screens: Vec<Screen>,
    pub eis_fd: OwnedFd,
    pub session: OwnedObjectPath,
    pub keyboard: bool,
    pub pointer: bool,
}

/// A shared output: its PipeWire node and logical place in the layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub node: u32,
    pub position: (i32, i32),
    pub size: (u32, u32),
    pub mapping_id: Option<String>,
    /// Cast from a virtual output, as lock mode's screens are, not a monitor.
    pub virtual_output: bool,
}

impl Grant {
    /// Each screen's PipeWire remote, in `screens` order; a second call returns none. Every
    /// fd is a client connection of its own: two pw_cores on one socket break each other.
    pub fn take_pipewire_fds(&mut self) -> Vec<OwnedFd> {
        std::mem::take(&mut self.pw_fds)
    }
}

enum Command {
    Open(tokio::sync::oneshot::Sender<Result<Grant, String>>),
    Close(OwnedObjectPath),
}

/// Start a session; ends with it closed again when any step fails.
pub async fn open() -> Result<Grant, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    OPEN.send_modify(|n| *n += 1);
    if commands().send(Command::Open(tx)).is_err() {
        OPEN.send_modify(|n| *n -= 1);
        return Err(GONE.to_string());
    }
    rx.await.map_err(|_| GONE.to_string())?
}

/// `open` (production passes [`open`]), tried again every 250 ms until it succeeds or `deadline`
/// has passed, for a session whose portal is still starting.
pub async fn open_when_ready<T, F: Future<Output = Result<T, String>>>(
    mut open: impl FnMut() -> F,
    deadline: Duration,
) -> Result<T, String> {
    let until = tokio::time::Instant::now() + deadline;
    loop {
        match open().await {
            Ok(opened) => return Ok(opened),
            Err(e) if tokio::time::Instant::now() >= until => return Err(e),
            Err(e) => log::debug!("the portal is not ready: {e}"),
        }
        tokio::time::sleep(RETRY).await;
    }
}

const RETRY: Duration = Duration::from_millis(250);

/// Resolves once every session asked for is closed or failed to open.
pub async fn closed() {
    let _ = OPEN.subscribe().wait_for(|n| *n == 0).await;
}

/// Sessions from the request to open them until they are closed.
static OPEN: LazyLock<tokio::sync::watch::Sender<usize>> =
    LazyLock::new(|| tokio::sync::watch::Sender::new(0));

impl Drop for Grant {
    fn drop(&mut self) {
        if commands()
            .send(Command::Close(self.session.clone()))
            .is_err()
        {
            OPEN.send_modify(|n| *n -= 1);
        }
    }
}

const GONE: &str = "the portal thread is gone";

fn commands() -> &'static mpsc::Sender<Command> {
    static COMMANDS: OnceLock<mpsc::Sender<Command>> = OnceLock::new();
    COMMANDS.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        if let Err(e) = std::thread::Builder::new()
            .name("reprise-rdp-portal".into())
            .spawn(move || serve(rx))
        {
            log::error!("no portal thread: {e}");
        }
        tx
    })
}

fn serve(rx: mpsc::Receiver<Command>) {
    let mut bus: Option<Connection> = None;
    for command in rx {
        match command {
            Command::Open(reply) => {
                let opened = zbus::block_on(async {
                    let conn = match &bus {
                        Some(c) => c.clone(),
                        None => {
                            let c = Connection::session().await.map_err(|e| e.to_string())?;
                            bus = Some(c.clone());
                            c
                        }
                    };
                    open_on(&conn).await
                });
                if opened.is_err() {
                    OPEN.send_modify(|n| *n -= 1);
                }
                let _ = reply.send(opened);
            }
            Command::Close(session) => {
                if let Some(conn) = &bus {
                    zbus::block_on(close_on(conn, &session));
                }
                OPEN.send_modify(|n| *n -= 1);
            }
        }
    }
}

async fn open_on(conn: &Connection) -> Result<Grant, String> {
    let token = next_token();
    let created: CreateResults = request(conn, REMOTE, "CreateSession", |mut options| {
        options.insert("session_handle_token", Variant::from(token));
        (options,)
    })
    .await?;
    let session =
        OwnedObjectPath::try_from(created.session_handle.ok_or("CreateSession: no session")?)
            .map_err(|e| e.to_string())?;
    match start(conn, &session).await {
        Ok(grant) => Ok(grant),
        Err(e) => {
            close_on(conn, &session).await;
            Err(e)
        }
    }
}

async fn start(conn: &Connection, session: &OwnedObjectPath) -> Result<Grant, String> {
    let _: Ignored = request(conn, REMOTE, "SelectDevices", |mut options| {
        options.insert("types", Variant::from(KEYBOARD | POINTER));
        (session, options)
    })
    .await?;
    let _: Ignored = request(conn, SCREENCAST, "SelectSources", |mut options| {
        options.insert("types", Variant::from(MONITOR));
        options.insert("cursor_mode", Variant::from(CURSOR_METADATA));
        (session, options)
    })
    .await?;
    let started: StartResults =
        request(conn, REMOTE, "Start", |options| (session, "", options)).await?;
    let screens = screens(started.streams)?;
    let mut pw_fds = Vec::with_capacity(screens.len());
    for _ in &screens {
        pw_fds.push(fd(conn, SCREENCAST, "OpenPipeWireRemote", session).await?);
    }
    let eis_fd = fd(conn, REMOTE, "ConnectToEIS", session).await?;
    let devices = started.devices.unwrap_or(KEYBOARD | POINTER);
    Ok(Grant {
        pw_fds,
        screens,
        eis_fd,
        session: session.clone(),
        keyboard: devices & KEYBOARD != 0,
        pointer: devices & POINTER != 0,
    })
}

fn screens(streams: Option<Vec<(u32, StreamProps)>>) -> Result<Vec<Screen>, String> {
    let streams = streams.unwrap_or_default();
    if streams.is_empty() {
        return Err("Start: no stream was shared".into());
    }
    (streams.into_iter())
        .map(|(node, stream)| {
            let size = stream.size.ok_or("Start: a stream has no size")?;
            Ok(Screen {
                node,
                position: stream.position.unwrap_or_default(),
                size: (size.0.max(0) as u32, size.1.max(0) as u32),
                mapping_id: stream.mapping_id,
                virtual_output: stream.source_type == Some(VIRTUAL),
            })
        })
        .collect()
}

async fn fd(
    conn: &Connection,
    iface: &str,
    method: &str,
    session: &OwnedObjectPath,
) -> Result<OwnedFd, String> {
    let reply = conn
        .call_method(
            Some(PORTAL),
            PORTAL_PATH,
            Some(iface),
            method,
            &(session, HashMap::<&str, Variant>::new()),
        )
        .await
        .map_err(|e| format!("{method}: {e}"))?;
    let fd: zbus::OwnedFd = reply
        .body()
        .deserialize()
        .map_err(|e| format!("{method}: {e}"))?;
    Ok(fd.into())
}

async fn close_on(conn: &Connection, session: &OwnedObjectPath) {
    match (conn.call_method(Some(PORTAL), session, Some(SESSION), "Close", &())).await {
        Ok(_) => {}
        Err(e) if already_closed(&e) => log::debug!("the portal session was already closed"),
        Err(e) => log::info!("closing the portal session: {e}"),
    }
}

/// Close answered by no session object: the frontend closes a session itself when a request
/// on it fails, as Start does with no screen to share.
fn already_closed(e: &zbus::Error) -> bool {
    matches!(
        reprisebus::error::name(e).as_deref(),
        Some(
            "org.freedesktop.DBus.Error.UnknownMethod" | "org.freedesktop.DBus.Error.UnknownObject"
        )
    )
}

type Options = HashMap<&'static str, Variant<'static>>;
type Ignored = HashMap<String, OwnedVariant>;

#[derive(DeserializeDict, Type)]
#[zvariant(signature = "a{sv}")]
struct CreateResults {
    session_handle: Option<String>,
}

#[derive(DeserializeDict, Type)]
#[zvariant(signature = "a{sv}")]
struct StartResults {
    devices: Option<u32>,
    streams: Option<Vec<(u32, StreamProps)>>,
}

#[derive(DeserializeDict, Type)]
#[zvariant(signature = "a{sv}")]
struct StreamProps {
    position: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
    mapping_id: Option<String>,
    source_type: Option<u32>,
}

fn next_token() -> String {
    static SERIAL: AtomicU32 = AtomicU32::new(0);
    format!("reprise_rdp_{}", SERIAL.fetch_add(1, Ordering::Relaxed))
}

/// The request object the portal answers on, derived from our unique name and the handle
/// token, so the Response can be subscribed before the call.
fn request_path(unique: &str, token: &str) -> String {
    let sender = unique.trim_start_matches(':').replace('.', "_");
    format!("{PORTAL_PATH}/request/{sender}/{token}")
}

/// Call `method` with the body `body` builds around the options carrying a fresh handle token,
/// and wait for its Response: the results on success, the method and code otherwise.
async fn request<B, R>(
    conn: &Connection,
    iface: &str,
    method: &str,
    body: impl FnOnce(Options) -> B,
) -> Result<R, String>
where
    B: Serialize + DynamicType,
    R: DeserializeOwned + Type,
{
    let failed = |e: zbus::Error| format!("{method}: {e}");
    let token = next_token();
    let unique = conn
        .unique_name()
        .map(|n| n.to_string())
        .unwrap_or_default();
    let path = request_path(&unique, &token);
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(REQUEST)
        .member("Response")
        .path(path.as_str())
        .build()
        .map_err(failed)?;
    let mut responses = MessageStream::for_match_rule(rule, conn, None)
        .await
        .map_err(failed)?;
    let options = Options::from([("handle_token", Variant::from(token))]);
    conn.call_method(
        Some(PORTAL),
        PORTAL_PATH,
        Some(iface),
        method,
        &body(options),
    )
    .await
    .map_err(failed)?;
    let runtime = zruntime::SharedRuntime::current().map_err(|e| format!("{method}: {e}"))?;
    let answer = runtime
        .timeout(ANSWER, responses.next())
        .await
        .map_err(|_| format!("{method}: no answer in {} s", ANSWER.as_secs()))?
        .ok_or_else(|| format!("{method}: the bus closed"))?
        .map_err(failed)?;
    let (code, results): (u32, R) = answer.body().deserialize().map_err(failed)?;
    match code {
        0 => Ok(results),
        1 => Err(format!("{method}: refused")),
        _ => Err(format!("{method}: failed")),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A grant of no portal session, counted open like one until it is dropped.
    pub(crate) fn grant() -> Grant {
        OPEN.send_modify(|n| *n += 1);
        let null = || OwnedFd::from(std::fs::File::open("/dev/null").unwrap());
        Grant {
            pw_fds: vec![null()],
            screens: vec![Screen {
                node: 0,
                position: (0, 0),
                size: (0, 0),
                mapping_id: None,
                virtual_output: false,
            }],
            eis_fd: null(),
            session: OwnedObjectPath::try_from("/org/reprise/test/session").unwrap(),
            keyboard: true,
            pointer: true,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_portal_is_retried_until_the_session_is_ready() {
        let start = tokio::time::Instant::now();
        let mut tries = 0;
        let opened = open_when_ready(
            || {
                tries += 1;
                let ready = tries > 2;
                async move {
                    if ready {
                        Ok(())
                    } else {
                        Err("not yet".to_string())
                    }
                }
            },
            crate::wayland::SESSION_READY_WAIT,
        )
        .await;
        assert_eq!(opened, Ok(()));
        assert_eq!(start.elapsed(), Duration::from_millis(500));

        let start = tokio::time::Instant::now();
        let failed = open_when_ready(
            || async { Err::<(), _>("no portal".to_string()) },
            crate::wayland::SESSION_READY_WAIT,
        )
        .await;
        assert_eq!(failed, Err("no portal".to_string()));
        assert_eq!(start.elapsed(), Duration::from_secs(30));
    }

    #[test]
    fn a_close_of_a_session_the_frontend_already_closed_is_not_an_error() {
        let fdo = |e| zbus::Error::FDO(Box::new(e));
        let gone =
            "Object does not exist at path \"/org/freedesktop/portal/desktop/session/1_1/x\"";
        assert!(already_closed(&fdo(zbus::fdo::Error::UnknownMethod(
            gone.into()
        ))));
        assert!(already_closed(&fdo(zbus::fdo::Error::UnknownObject(
            gone.into()
        ))));
        assert!(!already_closed(&fdo(zbus::fdo::Error::AccessDenied(
            "no".into()
        ))));
        assert!(!already_closed(&zbus::Error::InterfaceNotFound));
    }

    #[test]
    fn start_results_keep_the_portal_order() {
        use zbus::wire::{LE, serialized::Context, to_bytes};
        let stream = |x: i32, width: i32, name: &str| {
            HashMap::from([
                ("position", Variant::from((x, 0))),
                ("size", Variant::from((width, 1080))),
                ("source_type", Variant::from(MONITOR)),
                ("mapping_id", Variant::from(name.to_string())),
            ])
        };
        let streams = vec![
            (52u32, stream(1920, 1280, "DP-2")),
            (47u32, stream(0, 1920, "HDMI-A-1")),
        ];
        let results = HashMap::from([
            ("devices", Variant::from(KEYBOARD)),
            ("streams", Variant::from(streams)),
        ]);
        let bytes = to_bytes(Context::new(LE, 0), &results).unwrap();
        let (started, _): (StartResults, _) = bytes.deserialize().unwrap();
        let screen = |node, x, width, name: &str| Screen {
            node,
            position: (x, 0),
            size: (width, 1080),
            mapping_id: Some(name.into()),
            virtual_output: false,
        };
        assert_eq!(
            screens(started.streams).unwrap(),
            vec![
                screen(52, 1920, 1280, "DP-2"),
                screen(47, 0, 1920, "HDMI-A-1")
            ],
            "the screens in the order the portal sent them"
        );
    }

    #[test]
    fn start_results_mark_the_streams_cast_from_virtual_outputs() {
        use zbus::wire::{LE, serialized::Context, to_bytes};
        let stream = |source_type: u32| {
            HashMap::from([
                ("size", Variant::from((1920, 1080))),
                ("source_type", Variant::from(source_type)),
            ])
        };
        let streams = vec![(52u32, stream(VIRTUAL)), (47u32, stream(MONITOR))];
        let results = HashMap::from([("streams", Variant::from(streams))]);
        let bytes = to_bytes(Context::new(LE, 0), &results).unwrap();
        let (started, _): (StartResults, _) = bytes.deserialize().unwrap();
        let virtual_outputs: Vec<bool> = (screens(started.streams).unwrap().iter())
            .map(|s| s.virtual_output)
            .collect();
        assert_eq!(virtual_outputs, [true, false]);
    }

    /// The frontend's RemoteDesktop and ScreenCast calls, answering each request at once with
    /// two shared screens.
    struct Fake {
        remotes: std::sync::Arc<std::sync::Mutex<Vec<std::os::unix::net::UnixStream>>>,
    }

    type Asked = HashMap<String, OwnedVariant>;

    async fn answer(
        conn: &Connection,
        hdr: &zbus::message::Header<'_>,
        options: &Asked,
        results: HashMap<&str, Variant<'_>>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let token = String::try_from(options["handle_token"].try_clone().unwrap()).unwrap();
        let path = request_path(hdr.sender().unwrap().as_str(), &token);
        let body = (0u32, results);
        (conn.emit_signal(None::<&str>, path.as_str(), REQUEST, "Response", &body)).await?;
        Ok(OwnedObjectPath::try_from(path).unwrap())
    }

    fn socket() -> (zbus::OwnedFd, std::os::unix::net::UnixStream) {
        let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        (OwnedFd::from(ours).into(), theirs)
    }

    #[zbus::interface(name = "org.freedesktop.portal.RemoteDesktop")]
    impl Fake {
        async fn create_session(
            &self,
            options: Asked,
            #[zbus(connection)] conn: &Connection,
            #[zbus(header)] hdr: zbus::message::Header<'_>,
        ) -> zbus::fdo::Result<OwnedObjectPath> {
            let session =
                Variant::from("/org/freedesktop/portal/desktop/session/1_1/s".to_string());
            answer(
                conn,
                &hdr,
                &options,
                HashMap::from([("session_handle", session)]),
            )
            .await
        }

        async fn select_devices(
            &self,
            _session: OwnedObjectPath,
            options: Asked,
            #[zbus(connection)] conn: &Connection,
            #[zbus(header)] hdr: zbus::message::Header<'_>,
        ) -> zbus::fdo::Result<OwnedObjectPath> {
            answer(conn, &hdr, &options, HashMap::new()).await
        }

        async fn start(
            &self,
            _session: OwnedObjectPath,
            _parent: String,
            options: Asked,
            #[zbus(connection)] conn: &Connection,
            #[zbus(header)] hdr: zbus::message::Header<'_>,
        ) -> zbus::fdo::Result<OwnedObjectPath> {
            let stream = |x: i32| {
                HashMap::from([
                    ("position", Variant::from((x, 0))),
                    ("size", Variant::from((1920, 1080))),
                ])
            };
            let results = HashMap::from([
                ("devices", Variant::from(KEYBOARD | POINTER)),
                (
                    "streams",
                    Variant::from(vec![(63u32, stream(0)), (74u32, stream(1920))]),
                ),
            ]);
            answer(conn, &hdr, &options, results).await
        }

        #[zbus(name = "ConnectToEIS")]
        async fn connect_to_eis(
            &self,
            _session: OwnedObjectPath,
            _options: Asked,
        ) -> zbus::OwnedFd {
            socket().0
        }
    }

    struct FakeCast(Fake);

    #[zbus::interface(name = "org.freedesktop.portal.ScreenCast")]
    impl FakeCast {
        async fn select_sources(
            &self,
            _session: OwnedObjectPath,
            options: Asked,
            #[zbus(connection)] conn: &Connection,
            #[zbus(header)] hdr: zbus::message::Header<'_>,
        ) -> zbus::fdo::Result<OwnedObjectPath> {
            answer(conn, &hdr, &options, HashMap::new()).await
        }

        #[zbus(name = "OpenPipeWireRemote")]
        async fn open_pipewire_remote(
            &self,
            _session: OwnedObjectPath,
            _options: Asked,
        ) -> zbus::OwnedFd {
            let (ours, theirs) = socket();
            self.0.remotes.lock().unwrap().push(theirs);
            ours
        }
    }

    #[test]
    fn each_screen_gets_a_pipewire_remote_of_its_own() {
        use std::os::unix::fs::MetadataExt;
        let Some(bus) = crate::control::tests::bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let remotes = std::sync::Arc::default();
        let fake = || Fake {
            remotes: std::sync::Arc::clone(&remotes),
        };
        let mut grant = zbus::block_on(async {
            let _portal = zbus::connection::Builder::address(bus.1.as_str())
                .name(PORTAL)
                .serve_at(PORTAL_PATH, fake())
                .serve_at(PORTAL_PATH, FakeCast(fake()))
                .build()
                .await
                .unwrap();
            let conn = zbus::connection::Builder::address(bus.1.as_str())
                .build()
                .await
                .unwrap();
            OPEN.send_modify(|n| *n += 1);
            open_on(&conn).await.unwrap()
        });
        let fds = grant.take_pipewire_fds();
        assert_eq!(
            remotes.lock().unwrap().len(),
            2,
            "one OpenPipeWireRemote call per screen"
        );
        let socket = |fd: &OwnedFd| {
            std::fs::File::from(fd.try_clone().unwrap())
                .metadata()
                .unwrap()
                .ino()
        };
        assert_eq!(fds.len(), 2, "a PipeWire fd for each of the two screens");
        assert_ne!(
            socket(&fds[0]),
            socket(&fds[1]),
            "the screens' fds are different sockets"
        );
        assert!(
            grant.take_pipewire_fds().is_empty(),
            "the fds are handed out once"
        );
    }

    #[test]
    fn the_request_path_follows_the_unique_name_and_token() {
        assert_eq!(
            request_path(":1.42", "reprise_rdp_3"),
            "/org/freedesktop/portal/desktop/request/1_42/reprise_rdp_3"
        );
    }
}
