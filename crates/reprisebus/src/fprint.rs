//! fprintd on the system bus: whether a user has a finger, and the caller's own fingers to read,
//! add and delete. The verification itself runs in PAM (`pam_fprintd`), never here.

use std::cell::Cell;
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::future::{AbortHandle, AbortRegistration, Abortable};
use zbus::proxy::MethodFlags;
use zbus::{Connection, OwnedObjectPath, Proxy};

pub const NAME: &str = "net.reactivated.Fprint";
const MANAGER_PATH: &str = "/net/reactivated/Fprint/Manager";
const MANAGER: &str = "net.reactivated.Fprint.Manager";
const DEVICE: &str = "net.reactivated.Fprint.Device";

const TIMEOUT: Duration = Duration::from_secs(5);
/// How long a claim, add or delete may wait, the password prompt fprintd may raise included.
const ASKING: Duration = Duration::from_secs(600);

/// [`enrolled`] on a system bus connection of its own, blocking, with each call cut off after 5 s
/// (false).
pub fn has_finger(user: &str) -> bool {
    zbus::block_on(async {
        let Ok(conn) = zbus::connection::Builder::system()
            .method_timeout(TIMEOUT)
            .build()
            .await
        else {
            return false;
        };
        enrolled(&conn, user).await
    })
}

/// Whether the default reader has a finger of `user`'s. False without fprintd or a reader, or when
/// the caller may not ask about `user`.
pub async fn enrolled(conn: &Connection, user: &str) -> bool {
    let device = match default_device(conn).await {
        Ok(d) => d,
        Err(e) => {
            log::info!("fprint: no fingerprint reader ({e})");
            return false;
        }
    };
    match device
        .call::<_, _, Vec<String>>("ListEnrolledFingers", &(user,))
        .await
    {
        Ok(fingers) => !fingers.is_empty(),
        Err(e) => {
            log::info!("fprint: no enrolled finger for {user} ({e})");
            false
        }
    }
}

async fn default_device(conn: &Connection) -> zbus::Result<Proxy<'static>> {
    let manager = Proxy::new(conn, NAME, MANAGER_PATH, MANAGER).await?;
    let path: OwnedObjectPath = manager.call("GetDefaultDevice", &()).await?;
    zbus::proxy::Builder::<Proxy>::new(conn)
        .destination(NAME)
        .path(path)
        .interface(DEVICE)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
}

fn error_name(e: &zbus::Error) -> Option<String> {
    crate::error::name(e).map(|n| n.to_string())
}

const NO_SERVICE: &str = "org.freedesktop.DBus.Error.ServiceUnknown";
const NO_DEVICE: &str = "net.reactivated.Fprint.Error.NoSuchDevice";
const NO_PRINTS: &str = "net.reactivated.Fprint.Error.NoEnrolledPrints";

/// The default reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reader {
    pub name: String,
    /// The reader takes a swiped finger, not one laid on it.
    pub swipe: bool,
    /// Scans to add a finger; 0 when the reader doesn't say.
    pub stages: u32,
    /// The caller's fingers on it, by fprintd's names.
    pub fingers: Vec<String>,
}

/// What a read of the default reader found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// fprintd isn't installed.
    NoService,
    NoReader,
    /// fprintd failed or gave no answer in time.
    Silent,
    Reader(Reader),
}

/// [`read_on`] on a system bus connection of its own, blocking, each call cut off after 5 s.
pub fn read() -> Found {
    zbus::block_on(async {
        let built = zbus::connection::Builder::system()
            .method_timeout(TIMEOUT)
            .build()
            .await;
        match built {
            Ok(conn) => read_on(&conn).await,
            Err(e) => {
                log::warn!("fprint: no system bus ({e})");
                Found::Silent
            }
        }
    })
}

/// The default reader and the caller's fingers on it.
pub async fn read_on(conn: &Connection) -> Found {
    let missing = |e: zbus::Error| match error_name(&e).as_deref() {
        Some(NO_SERVICE) => Found::NoService,
        Some(NO_DEVICE) => Found::NoReader,
        _ => {
            log::warn!("fprint: the reader can't be read ({e})");
            Found::Silent
        }
    };
    let device = match default_device(conn).await {
        Ok(d) => d,
        Err(e) => return missing(e),
    };
    let fingers = match device
        .call::<_, _, Vec<String>>("ListEnrolledFingers", &("",))
        .await
    {
        Ok(f) => f,
        Err(e) if error_name(&e).as_deref() == Some(NO_PRINTS) => Vec::new(),
        Err(e) => return missing(e),
    };
    let name = match device.get_property::<String>("name").await {
        Ok(n) => n,
        Err(e) => return missing(e),
    };
    let stages = device
        .get_property::<i32>("num-enroll-stages")
        .await
        .map_or(0, |n| u32::try_from(n).unwrap_or(0));
    let swipe = device
        .get_property::<String>("scan-type")
        .await
        .is_ok_and(|t| t == "swipe");
    Found::Reader(Reader {
        name,
        swipe,
        stages,
        fingers,
    })
}

/// Why fprintd turned a claim, add or delete down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// polkit refused, or its password prompt was cancelled.
    NotAuthorized,
    /// Another program holds the reader.
    InUse,
    Failed,
}

fn refusal(doing: &str, e: &zbus::Error) -> Refusal {
    match error_name(e).as_deref() {
        Some("net.reactivated.Fprint.Error.PermissionDenied") => Refusal::NotAuthorized,
        Some("net.reactivated.Fprint.Error.AlreadyInUse") => Refusal::InUse,
        _ => {
            log::warn!("fprint: {doing}: {e}");
            Refusal::Failed
        }
    }
}

async fn asking(work: impl AsyncFnOnce(Connection) -> Result<(), Refusal>) -> Result<(), Refusal> {
    let built = zbus::connection::Builder::system()
        .method_timeout(ASKING)
        .build()
        .await;
    match built {
        Ok(conn) => work(conn).await,
        Err(e) => Err(refusal("system bus", &e)),
    }
}

/// A call fprintd may answer with a password prompt first.
async fn interactive(device: &Proxy<'_>, method: &str, arg: Option<&str>) -> zbus::Result<()> {
    let flags = MethodFlags::ALLOW_INTERACTIVE_AUTH;
    match arg {
        Some(a) => {
            device
                .call_with_flags::<_, _, ()>(method, flags, &(a,))
                .await
        }
        None => device.call_with_flags::<_, _, ()>(method, flags, &()).await,
    }
    .map(drop)
}

/// Delete one of the caller's fingers by fprintd's name, or with `None` all of them, on its own
/// system bus connection, blocking.
pub fn delete(finger: Option<&str>) -> Result<(), Refusal> {
    zbus::block_on(asking(async |conn| delete_on(&conn, finger).await))
}

pub async fn delete_on(conn: &Connection, finger: Option<&str>) -> Result<(), Refusal> {
    let device = default_device(conn)
        .await
        .map_err(|e| refusal("no reader", &e))?;
    interactive(&device, "Claim", Some(""))
        .await
        .map_err(|e| refusal("claim", &e))?;
    let deleted = match finger {
        Some(f) => interactive(&device, "DeleteEnrolledFinger", Some(f)).await,
        None => interactive(&device, "DeleteEnrolledFingers2", None).await,
    }
    .map_err(|e| refusal("delete", &e));
    if let Err(e) = device.call::<_, _, ()>("Release", &()).await {
        log::debug!("fprint: release: {e}");
    }
    deleted
}

/// How far an add has come.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Adding {
    /// The reader is claimed and waits for the finger.
    Started,
    /// fprintd's result for a scan; `done` ends the add.
    Status { result: String, done: bool },
}

/// Stops an add from another thread: [`Stop::stop`], or dropping it.
pub struct Stop(AbortHandle);

/// The add's end of a [`Stop`].
pub struct StopSignal(AbortRegistration);

impl Stop {
    pub fn new() -> (Stop, StopSignal) {
        let (handle, registration) = AbortHandle::new_pair();
        (Stop(handle), StopSignal(registration))
    }

    pub fn stop(&self) {
        self.0.abort();
    }
}

impl Drop for Stop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Add the caller's `finger` (fprintd's name) on a system bus connection of its own, blocking,
/// telling `tell` each step until fprintd is done or `stop` is stopped.
pub fn add(finger: &str, stop: StopSignal, tell: impl FnMut(Adding)) -> Result<(), Refusal> {
    zbus::block_on(asking(async |conn| add_on(&conn, finger, stop, tell).await))
}

pub async fn add_on(
    conn: &Connection,
    finger: &str,
    stop: StopSignal,
    mut tell: impl FnMut(Adding),
) -> Result<(), Refusal> {
    let device = default_device(conn)
        .await
        .map_err(|e| refusal("no reader", &e))?;
    let (claimed, started) = (Cell::new(false), Cell::new(false));
    let run = async {
        interactive(&device, "Claim", Some(""))
            .await
            .map_err(|e| refusal("claim", &e))?;
        claimed.set(true);
        let mut status = device
            .receive_signal("EnrollStatus")
            .await
            .map_err(|e| refusal("follow", &e))?;
        interactive(&device, "EnrollStart", Some(finger))
            .await
            .map_err(|e| refusal("start", &e))?;
        started.set(true);
        tell(Adding::Started);
        while let Some(msg) = status.next().await {
            let Ok((result, done)) = msg.body().deserialize::<(String, bool)>() else {
                continue;
            };
            tell(Adding::Status { result, done });
            if done {
                return Ok(());
            }
        }
        log::warn!("fprint: the reader stopped answering");
        Err(Refusal::Failed)
    };
    let added = match Abortable::new(run, stop.0).await {
        Ok(added) => added,
        Err(_) => {
            if started.get()
                && let Err(e) = device.call::<_, _, ()>("EnrollStop", &()).await
            {
                log::debug!("fprint: stop: {e}");
            }
            Ok(())
        }
    };
    // A claim cut short by the stop ends with this connection.
    if claimed.get()
        && let Err(e) = device.call::<_, _, ()>("Release", &()).await
    {
        log::debug!("fprint: release: {e}");
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};

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

    const DEVICE_PATH: &str = "/net/reactivated/Fprint/Device/0";

    struct Manager;

    #[zbus::interface(name = "net.reactivated.Fprint.Manager")]
    impl Manager {
        fn get_default_device(&self) -> OwnedObjectPath {
            OwnedObjectPath::try_from(DEVICE_PATH).unwrap()
        }
    }

    struct FakeDevice(Vec<String>);

    #[zbus::interface(name = "net.reactivated.Fprint.Device")]
    impl FakeDevice {
        fn list_enrolled_fingers(&self, user: &str) -> Vec<String> {
            if user == "someone" {
                self.0.clone()
            } else {
                Vec::new()
            }
        }
    }

    fn ask(fingers: Option<&[&str]>, user: &str) -> Option<bool> {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return None;
        };
        Some(zbus::block_on(async move {
            let _fprintd = match fingers {
                Some(fingers) => {
                    let c = connect(&bus.1).await;
                    c.object_server().at(MANAGER_PATH, Manager).await.unwrap();
                    let device = FakeDevice(fingers.iter().map(|f| f.to_string()).collect());
                    c.object_server().at(DEVICE_PATH, device).await.unwrap();
                    c.request_name(NAME).await.unwrap();
                    Some(c)
                }
                None => None,
            };
            let conn = connect(&bus.1).await;
            enrolled(&conn, user).await
        }))
    }

    #[test]
    fn a_user_with_a_finger_has_one_enrolled() {
        let Some(yes) = ask(Some(&["right-index-finger"]), "someone") else {
            return;
        };
        assert!(yes);
        assert_eq!(ask(Some(&["right-index-finger"]), "other"), Some(false));
        assert_eq!(ask(Some(&[]), "someone"), Some(false));
    }

    #[test]
    fn no_fprintd_means_no_finger() {
        assert!(!ask(None, "someone").unwrap_or(false));
    }

    use std::sync::{Arc, Mutex};
    use zbus::object_server::SignalEmitter;

    #[derive(Debug, zbus::DBusError)]
    #[zbus(prefix = "net.reactivated.Fprint.Error")]
    enum FprintError {
        #[zbus(error)]
        ZBus(zbus::Error),
        PermissionDenied(String),
        NoEnrolledPrints(String),
    }

    /// A reader that adds a finger in two scans, or with `hang` waits for one forever, and logs
    /// every call made to it.
    #[derive(Clone, Default)]
    struct Reader {
        fingers: Arc<Mutex<Vec<String>>>,
        calls: Arc<Mutex<Vec<String>>>,
        deny: bool,
        hang: bool,
    }

    impl Reader {
        fn log(&self, call: String) {
            self.calls.lock().unwrap().push(call);
        }
    }

    #[zbus::interface(name = "net.reactivated.Fprint.Device")]
    impl Reader {
        fn claim(&self, user: &str) -> Result<(), FprintError> {
            self.log(format!("Claim {user:?}"));
            if self.deny {
                return Err(FprintError::PermissionDenied("Not Authorized".into()));
            }
            Ok(())
        }

        fn release(&self) {
            self.log("Release".into());
        }

        fn list_enrolled_fingers(&self, user: &str) -> Result<Vec<String>, FprintError> {
            assert_eq!(user, "", "only the caller's own");
            let f = self.fingers.lock().unwrap().clone();
            if f.is_empty() {
                return Err(FprintError::NoEnrolledPrints("none".into()));
            }
            Ok(f)
        }

        async fn enroll_start(
            &self,
            finger: &str,
            #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        ) -> Result<(), FprintError> {
            self.log(format!("EnrollStart {finger}"));
            if self.hang {
                return Ok(());
            }
            for (result, done) in [
                ("enroll-stage-passed", false),
                ("enroll-finger-not-centered", false),
                ("enroll-stage-passed", false),
                ("enroll-completed", true),
            ] {
                Self::enroll_status(&emitter, result, done).await?;
            }
            self.fingers.lock().unwrap().push(finger.into());
            Ok(())
        }

        fn enroll_stop(&self) {
            self.log("EnrollStop".into());
        }

        fn delete_enrolled_finger(&self, finger: &str) {
            self.log(format!("DeleteEnrolledFinger {finger}"));
            self.fingers.lock().unwrap().retain(|f| f != finger);
        }

        fn delete_enrolled_fingers2(&self) {
            self.log("DeleteEnrolledFingers2".into());
            self.fingers.lock().unwrap().clear();
        }

        #[zbus(signal)]
        async fn enroll_status(
            emitter: &SignalEmitter<'_>,
            result: &str,
            done: bool,
        ) -> zbus::Result<()>;

        #[zbus(property, name = "name")]
        fn name(&self) -> String {
            "Virtual reader".into()
        }

        #[zbus(property, name = "num-enroll-stages")]
        fn stages(&self) -> i32 {
            2
        }

        #[zbus(property, name = "scan-type")]
        fn scan_type(&self) -> String {
            "swipe".into()
        }
    }

    /// `test` against `reader` served as fprintd's default device on a bus of its own.
    fn with_reader<T>(reader: &Reader, test: impl AsyncFnOnce(&Connection) -> T) -> Option<T> {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return None;
        };
        Some(zbus::block_on(async move {
            let c = connect(&bus.1).await;
            c.object_server().at(MANAGER_PATH, Manager).await.unwrap();
            c.object_server()
                .at(DEVICE_PATH, reader.clone())
                .await
                .unwrap();
            c.request_name(NAME).await.unwrap();
            let conn = connect(&bus.1).await;
            let out = test(&conn).await;
            drop(c);
            out
        }))
    }

    fn calls(r: &Reader) -> Vec<String> {
        r.calls.lock().unwrap().clone()
    }

    #[test]
    fn a_read_finds_the_reader_and_your_fingers() {
        let r = Reader::default();
        let Some(found) = with_reader(&r, async |c| read_on(c).await) else {
            return;
        };
        assert_eq!(
            found,
            Found::Reader(super::Reader {
                name: "Virtual reader".into(),
                swipe: true,
                stages: 2,
                fingers: Vec::new(),
            }),
            "no prints is no fingers"
        );
        r.fingers.lock().unwrap().push("left-thumb".into());
        let found = with_reader(&r, async |c| read_on(c).await);
        assert!(
            matches!(found, Some(Found::Reader(super::Reader { fingers, .. })) if fingers == ["left-thumb"])
        );
    }

    #[test]
    fn without_fprintd_a_read_finds_no_service() {
        let Some(bus) = bus() else {
            return;
        };
        let found = zbus::block_on(async { read_on(&connect(&bus.1).await).await });
        assert_eq!(found, Found::NoService);
    }

    #[test]
    fn an_add_claims_for_you_tells_each_scan_and_lets_the_reader_go() {
        let r = Reader::default();
        let (_stop, signal) = Stop::new();
        let mut told = Vec::new();
        let Some(out) = with_reader(&r, async |c| {
            add_on(c, "right-index-finger", signal, |a| told.push(a)).await
        }) else {
            return;
        };
        assert_eq!(out, Ok(()));
        let status = |result: &str, done| Adding::Status {
            result: result.into(),
            done,
        };
        assert_eq!(
            told,
            [
                Adding::Started,
                status("enroll-stage-passed", false),
                status("enroll-finger-not-centered", false),
                status("enroll-stage-passed", false),
                status("enroll-completed", true),
            ]
        );
        assert_eq!(
            calls(&r),
            ["Claim \"\"", "EnrollStart right-index-finger", "Release"]
        );
    }

    #[test]
    fn a_stopped_add_stops_the_reader_and_lets_it_go() {
        let r = Reader {
            hang: true,
            ..Reader::default()
        };
        let (stop, signal) = Stop::new();
        let mut stop = Some(stop);
        let Some(out) = with_reader(&r, async |c| {
            add_on(c, "left-thumb", signal, |a| {
                if a == Adding::Started {
                    stop.take();
                }
            })
            .await
        }) else {
            return;
        };
        assert_eq!(out, Ok(()));
        assert_eq!(
            calls(&r),
            [
                "Claim \"\"",
                "EnrollStart left-thumb",
                "EnrollStop",
                "Release"
            ]
        );
    }

    #[test]
    fn a_refused_claim_reads_as_not_authorized() {
        let r = Reader {
            deny: true,
            ..Reader::default()
        };
        let (_stop, signal) = Stop::new();
        let Some(out) = with_reader(&r, async |c| add_on(c, "left-thumb", signal, |_| {}).await)
        else {
            return;
        };
        assert_eq!(out, Err(Refusal::NotAuthorized));
        let out = with_reader(&r, async |c| delete_on(c, None).await);
        assert_eq!(out, Some(Err(Refusal::NotAuthorized)));
        assert_eq!(
            calls(&r),
            ["Claim \"\"", "Claim \"\""],
            "nothing past the claim"
        );
    }

    #[test]
    fn a_delete_claims_deletes_and_lets_the_reader_go() {
        let r = Reader::default();
        r.fingers
            .lock()
            .unwrap()
            .extend(["left-thumb".into(), "right-index-finger".into()]);
        let Some(out) = with_reader(&r, async |c| delete_on(c, Some("left-thumb")).await) else {
            return;
        };
        assert_eq!(out, Ok(()));
        assert_eq!(*r.fingers.lock().unwrap(), ["right-index-finger"]);
        assert_eq!(
            with_reader(&r, async |c| delete_on(c, None).await),
            Some(Ok(()))
        );
        assert!(r.fingers.lock().unwrap().is_empty());
        assert_eq!(
            calls(&r),
            [
                "Claim \"\"",
                "DeleteEnrolledFinger left-thumb",
                "Release",
                "Claim \"\"",
                "DeleteEnrolledFingers2",
                "Release"
            ]
        );
    }
}
