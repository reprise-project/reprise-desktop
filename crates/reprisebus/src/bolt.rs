//! boltd's Thunderbolt and USB4 devices on the system bus: the list and its changes, and
//! authorizing, enrolling and forgetting a device. boltd asks polkit for each change.

use std::collections::HashMap;

use futures_util::stream::StreamExt;
use zbus::proxy::{CacheProperties, MethodFlags};
use zbus::{Connection, OwnedMatchRule, OwnedObjectPath, OwnedVariant, Proxy, Variant};

pub const NAME: &str = "org.freedesktop.bolt";
const PATH: &str = "/org/freedesktop/bolt";
const MANAGER: &str = "org.freedesktop.bolt1.Manager";
const DEVICE: &str = "org.freedesktop.bolt1.Device";
const DOMAIN: &str = "org.freedesktop.bolt1.Domain";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// The policy an enrolled device gets: boltd authorizes it on its own whenever it connects.
const AUTO: &str = "auto";

/// A device's `Status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Disconnected,
    Connecting,
    /// Connected and not authorized: it does nothing until it is.
    Connected,
    Authorizing,
    AuthError,
    Authorized,
    Unknown,
}

impl Status {
    fn parse(s: &str) -> Status {
        match s {
            "disconnected" => Status::Disconnected,
            "connecting" => Status::Connecting,
            "connected" => Status::Connected,
            "authorizing" => Status::Authorizing,
            "auth-error" => Status::AuthError,
            // `authorized-secure`, `authorized-newkey` and `authorized-dponly` say how.
            s if s.starts_with("authorized") => Status::Authorized,
            _ => Status::Unknown,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub path: String,
    pub uid: String,
    pub name: String,
    pub vendor: String,
    pub status: Status,
    /// Enrolled: boltd keeps it in its database across connections.
    pub stored: bool,
    /// Enrolled with a policy under which boltd authorizes it itself when it connects.
    pub auto: bool,
}

impl Device {
    /// The vendor and the name, or the name alone when it already starts with the vendor.
    pub fn label(&self) -> String {
        let (vendor, name) = (self.vendor.trim(), self.name.trim());
        match (vendor.is_empty(), name.is_empty()) {
            (_, true) if !vendor.is_empty() => vendor.to_string(),
            (_, true) => "Thunderbolt device".into(),
            (true, false) => name.to_string(),
            (false, false) if name.starts_with(vendor) => name.to_string(),
            (false, false) => format!("{vendor} {name}"),
        }
    }

    /// Connected and waiting for someone to authorize it; boltd authorizes an `auto` one itself.
    pub fn waiting(&self) -> bool {
        self.status == Status::Connected && !self.auto
    }
}

type Props = HashMap<String, OwnedVariant>;

/// A device in `props`. Under the `iommu` policy boltd authorizes it itself only while the IOMMU
/// is on, which `iommu` says.
fn device(path: &str, props: &Props, iommu: bool) -> Option<Device> {
    let text = |k: &str| {
        props
            .get(k)
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_default()
    };
    // The computer's own controller is listed too and is never authorized.
    if text("Type") == "host" {
        return None;
    }
    let stored = props
        .get("Stored")
        .and_then(|v| bool::try_from(v.clone()).ok())
        .unwrap_or(false);
    Some(Device {
        path: path.to_string(),
        uid: text("Uid"),
        name: text("Name"),
        vendor: text("Vendor"),
        status: Status::parse(&text("Status")),
        stored,
        auto: stored
            && match text("Policy").as_str() {
                AUTO => true,
                "iommu" => iommu,
                _ => false,
            },
    })
}

async fn props(conn: &Connection, path: &str, iface: &str) -> zbus::Result<Props> {
    let p = zbus::proxy::Builder::<Proxy>::new(conn)
        .destination(NAME)
        .path(path)
        .interface(PROPERTIES)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    p.call("GetAll", &(iface,)).await
}

/// Whether every domain reports its IOMMU on; false for one that can't be read.
async fn iommu_on(conn: &Connection, domains: &[OwnedObjectPath]) -> bool {
    for d in domains {
        let on = (props(conn, d.as_str(), DOMAIN).await.ok())
            .and_then(|p| p.get("IOMMU").and_then(|v| bool::try_from(v.clone()).ok()));
        if on != Some(true) {
            return false;
        }
    }
    !domains.is_empty()
}

async fn manager(conn: &Connection) -> zbus::Result<Proxy<'static>> {
    zbus::proxy::Builder::<Proxy>::new(conn)
        .destination(NAME)
        .path(PATH)
        .interface(MANAGER)
        .cache_properties(CacheProperties::No)
        .build()
        .await
}

/// What boltd knows: how many Thunderbolt or USB4 controllers (domains) it found, and the devices
/// besides them, by label.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub domains: usize,
    pub devices: Vec<Device>,
}

/// Lists boltd's domains and devices; a call starts boltd if it isn't running.
pub async fn found(conn: &Connection) -> zbus::Result<Found> {
    let m = manager(conn).await?;
    let domains: Vec<OwnedObjectPath> = m.call("ListDomains", &()).await?;
    let paths: Vec<OwnedObjectPath> = m.call("ListDevices", &()).await?;
    let iommu = iommu_on(conn, &domains).await;
    let mut devices = Vec::new();
    for path in paths {
        match props(conn, path.as_str(), DEVICE).await {
            Ok(p) => devices.extend(device(path.as_str(), &p, iommu)),
            Err(e) => log::info!("bolt: {}: {e}", path.as_str()),
        }
    }
    devices.sort_by_key(|d| (d.label().to_lowercase(), d.uid.clone()));
    Ok(Found {
        domains: domains.len(),
        devices,
    })
}

/// [`found`] on the system bus, blocking for at most [`crate::METHOD_TIMEOUT`] a call; None
/// when boltd can't be reached.
pub fn read() -> Option<Found> {
    let conn = crate::system().ok()?;
    crate::wait(found(&conn))
        .inspect_err(|e| log::info!("bolt: {e}"))
        .ok()
}

fn rules() -> Vec<OwnedMatchRule> {
    let signal = || {
        zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(NAME)
    };
    let manager = |member| {
        signal()
            .interface(MANAGER)
            .member(member)
            .path(PATH)
            .build()
    };
    let props = signal()
        .interface(PROPERTIES)
        .member("PropertiesChanged")
        .path_namespace(PATH)
        .build();
    [manager("DeviceAdded"), manager("DeviceRemoved"), props]
        .into_iter()
        .filter_map(|r| r.ok().map(Into::into))
        .collect()
}

async fn now(conn: &Connection) -> Option<Found> {
    crate::name_owner(conn, NAME).await?;
    match found(conn).await {
        Ok(f) => Some(f),
        Err(e) => {
            log::info!("bolt: cannot list devices ({e})");
            None
        }
    }
}

/// Calls `seen` with what boltd knows: once at the start, then after each device it adds, removes
/// or changes. None while boltd is not on the bus; this never starts it. Returns only when the
/// connection closes; drop the future to stop.
pub async fn follow(conn: &Connection, mut seen: impl FnMut(Option<Found>)) {
    // Subscribed before the first read, so a change in between is not lost.
    let mut signals = crate::signals(conn, NAME, rules()).await;
    seen(now(conn).await);
    while signals.next().await.is_some() {
        seen(now(conn).await);
    }
}

/// What a button asks boltd to do with a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Authorize it for this connection only.
    Authorize,
    /// Enroll it so boltd authorizes it whenever it connects, and authorize it now.
    Always,
    /// Take it out of boltd's database: it waits for authorization the next time it connects.
    Forget,
}

/// Carries out `change` on `d`; polkit may ask for a password through the session's agent.
pub async fn apply(conn: &Connection, d: &Device, change: Change) -> zbus::Result<()> {
    let interactive = MethodFlags::ALLOW_INTERACTIVE_AUTH;
    match change {
        Change::Authorize => authorize(conn, d).await,
        Change::Always if !d.stored => {
            let m = manager(conn).await?;
            m.call_with_flags::<_, _, OwnedObjectPath>(
                "EnrollDevice",
                interactive,
                &(&d.uid, AUTO, ""),
            )
            .await
            .map(|_| ())
        }
        Change::Always => {
            let p = zbus::proxy::Builder::<Proxy>::new(conn)
                .destination(NAME)
                .path(d.path.as_str())
                .interface(PROPERTIES)
                .cache_properties(CacheProperties::No)
                .build()
                .await?;
            p.call_with_flags::<_, _, ()>(
                "Set",
                interactive,
                &(DEVICE, "Policy", Variant::from(AUTO)),
            )
            .await?;
            if d.status == Status::Connected {
                authorize(conn, d).await?;
            }
            Ok(())
        }
        Change::Forget => {
            let m = manager(conn).await?;
            m.call_with_flags::<_, _, ()>("ForgetDevice", interactive, &(&d.uid,))
                .await
                .map(|_| ())
        }
    }
}

async fn authorize(conn: &Connection, d: &Device) -> zbus::Result<()> {
    let p = zbus::proxy::Builder::<Proxy>::new(conn)
        .destination(NAME)
        .path(d.path.as_str())
        .interface(DEVICE)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    p.call_with_flags::<_, _, ()>("Authorize", MethodFlags::ALLOW_INTERACTIVE_AUTH, &("",))
        .await
        .map(|_| ())
}

/// [`apply`] on a system bus connection of its own, blocking until polkit and boltd answer. The
/// error is what to tell the user; None when they dismissed the password prompt.
pub fn carry_out(d: &Device, change: Change) -> Result<(), Option<String>> {
    let e = match zbus::block_on(async {
        let conn = Connection::system().await?;
        apply(&conn, d, change).await
    }) {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    log::warn!("bolt: {change:?} {}: {e}", d.uid);
    Err(crate::error::to_tell(e, crate::polkit::dismissed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

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

    const DOCK: &str = "/org/freedesktop/bolt/devices/dock";
    const DRIVE: &str = "/org/freedesktop/bolt/devices/drive";
    const HOST: &str = "/org/freedesktop/bolt/devices/host";

    type Calls = Arc<Mutex<Vec<String>>>;

    struct FakeDevice {
        uid: &'static str,
        name: &'static str,
        vendor: &'static str,
        kind: &'static str,
        status: String,
        stored: bool,
        policy: String,
        calls: Calls,
    }

    #[zbus::interface(name = "org.freedesktop.bolt1.Device")]
    impl FakeDevice {
        #[zbus(property)]
        fn uid(&self) -> String {
            self.uid.into()
        }
        #[zbus(property)]
        fn name(&self) -> String {
            self.name.into()
        }
        #[zbus(property)]
        fn vendor(&self) -> String {
            self.vendor.into()
        }
        #[zbus(property, name = "Type")]
        fn kind(&self) -> String {
            self.kind.into()
        }
        #[zbus(property)]
        fn status(&self) -> String {
            self.status.clone()
        }
        #[zbus(property)]
        fn stored(&self) -> bool {
            self.stored
        }
        #[zbus(property)]
        fn policy(&self) -> String {
            self.policy.clone()
        }
        #[zbus(property)]
        fn set_policy(&mut self, p: String) {
            self.calls
                .lock()
                .unwrap()
                .push(format!("Policy {} {p}", self.uid));
            self.policy = p;
        }
        fn authorize(&mut self, flags: String) {
            self.calls
                .lock()
                .unwrap()
                .push(format!("Authorize {} {flags:?}", self.uid));
        }
    }

    struct FakeManager {
        devices: Vec<OwnedObjectPath>,
        calls: Calls,
    }

    #[zbus::interface(name = "org.freedesktop.bolt1.Manager")]
    impl FakeManager {
        fn list_domains(&self) -> Vec<OwnedObjectPath> {
            vec![OwnedObjectPath::try_from("/org/freedesktop/bolt/domains/d0").unwrap()]
        }
        fn list_devices(&self) -> Vec<OwnedObjectPath> {
            self.devices.clone()
        }
        fn enroll_device(&self, uid: String, policy: String, flags: String) -> OwnedObjectPath {
            self.calls
                .lock()
                .unwrap()
                .push(format!("EnrollDevice {uid} {policy} {flags:?}"));
            OwnedObjectPath::try_from(DOCK).unwrap()
        }
        fn forget_device(&self, uid: String) {
            self.calls
                .lock()
                .unwrap()
                .push(format!("ForgetDevice {uid}"));
        }
        #[zbus(signal)]
        async fn device_added(
            e: &zbus::object_server::SignalEmitter<'_>,
            d: OwnedObjectPath,
        ) -> zbus::Result<()>;
    }

    fn fake(uid: &'static str, kind: &'static str, status: &str, calls: &Calls) -> FakeDevice {
        FakeDevice {
            uid,
            name: "Dock",
            vendor: "Acme",
            kind,
            status: status.into(),
            stored: false,
            policy: "default".into(),
            calls: calls.clone(),
        }
    }

    async fn fake_bolt(address: &str, calls: &Calls) -> Connection {
        let c = connect(address).await;
        let server = c.object_server();
        let paths = [HOST, DOCK].map(|p| OwnedObjectPath::try_from(p).unwrap());
        server
            .at(
                PATH,
                FakeManager {
                    devices: paths.to_vec(),
                    calls: calls.clone(),
                },
            )
            .await
            .unwrap();
        server
            .at(HOST, fake("host-uid", "host", "authorized", calls))
            .await
            .unwrap();
        server
            .at(DOCK, fake("dock-uid", "peripheral", "connected", calls))
            .await
            .unwrap();
        c.request_name(NAME).await.unwrap();
        c
    }

    async fn set_status(c: &Connection, path: &str, status: &str) {
        let iface = c
            .object_server()
            .interface::<_, FakeDevice>(path)
            .await
            .unwrap();
        iface.get_mut().await.status = status.into();
        iface
            .get()
            .await
            .status_changed(iface.signal_emitter())
            .await
            .unwrap();
    }

    fn dock(status: Status) -> Device {
        Device {
            path: DOCK.into(),
            uid: "dock-uid".into(),
            name: "Dock".into(),
            vendor: "Acme".into(),
            status,
            stored: false,
            auto: false,
        }
    }

    #[test]
    fn the_status_reads_every_authorized_flavour_as_authorized() {
        assert_eq!(Status::parse("connected"), Status::Connected);
        assert_eq!(Status::parse("authorized"), Status::Authorized);
        assert_eq!(Status::parse("authorized-newkey"), Status::Authorized);
        assert_eq!(Status::parse("auth-error"), Status::AuthError);
        assert_eq!(Status::parse("bogus"), Status::Unknown);
    }

    #[test]
    fn the_label_joins_vendor_and_name_once() {
        let named = |vendor: &str, name: &str| Device {
            vendor: vendor.into(),
            name: name.into(),
            ..dock(Status::Connected)
        };
        assert_eq!(named("Acme", "Dock").label(), "Acme Dock");
        assert_eq!(named("Acme", "Acme Dock").label(), "Acme Dock");
        assert_eq!(named("", "Dock").label(), "Dock");
        assert_eq!(named("Acme", "").label(), "Acme");
        assert_eq!(named("", " ").label(), "Thunderbolt device");
    }

    #[test]
    fn only_a_connected_device_boltd_leaves_alone_waits() {
        assert!(dock(Status::Connected).waiting());
        assert!(!dock(Status::Authorized).waiting());
        let auto = Device {
            stored: true,
            auto: true,
            ..dock(Status::Connected)
        };
        assert!(!auto.waiting(), "boltd authorizes an auto device itself");
    }

    #[test]
    fn a_stored_device_is_auto_under_the_auto_policy_or_the_iommu_one_with_the_iommu_on() {
        let props = |stored: bool, policy: &str| -> Props {
            [
                ("Stored", Variant::from(stored)),
                ("Policy", Variant::from(policy)),
                ("Status", Variant::from("connected")),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.try_to_owned().unwrap()))
            .collect()
        };
        let auto =
            |stored, policy, iommu| device(DOCK, &props(stored, policy), iommu).unwrap().auto;
        assert!(auto(true, "auto", false));
        assert!(auto(true, "iommu", true));
        assert!(
            !auto(true, "iommu", false),
            "boltd leaves it waiting with the IOMMU off"
        );
        assert!(!auto(true, "manual", true));
        assert!(!auto(false, "auto", true), "not enrolled");
    }

    #[test]
    fn the_list_leaves_out_the_host_and_counts_the_domains() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let calls = Calls::default();
        let got = zbus::block_on(async {
            let _bolt = fake_bolt(&bus.1, &calls).await;
            found(&connect(&bus.1).await).await.unwrap()
        });
        assert_eq!(
            got,
            Found {
                domains: 1,
                devices: vec![dock(Status::Connected)],
            }
        );
    }

    struct FakeDomain(bool);

    #[zbus::interface(name = "org.freedesktop.bolt1.Domain")]
    impl FakeDomain {
        #[zbus(property, name = "IOMMU")]
        fn iommu(&self) -> bool {
            self.0
        }
    }

    #[test]
    fn an_iommu_device_is_auto_only_while_the_domains_report_the_iommu_on() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let calls = Calls::default();
        let autos = zbus::block_on(async {
            let bolt = fake_bolt(&bus.1, &calls).await;
            let server = bolt.object_server();
            let dock = server.interface::<_, FakeDevice>(DOCK).await.unwrap();
            dock.get_mut().await.stored = true;
            dock.get_mut().await.policy = "iommu".into();
            let me = connect(&bus.1).await;
            let auto = async || found(&me).await.unwrap().devices[0].auto;
            let unread = auto().await;
            let domain = "/org/freedesktop/bolt/domains/d0";
            server.at(domain, FakeDomain(false)).await.unwrap();
            let off = auto().await;
            server.remove::<FakeDomain, _>(domain).await.unwrap();
            server.at(domain, FakeDomain(true)).await.unwrap();
            (unread, off, auto().await)
        });
        assert_eq!(autos, (false, false, true));
    }

    #[test]
    fn follow_reports_changes_and_none_without_bolt_and_never_starts_it() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let (tx, rx) = mpsc::channel();
        let address = bus.1.clone();
        std::thread::spawn(move || {
            zbus::block_on(async {
                let conn = connect(&address).await;
                follow(&conn, |f| tx.send(f).unwrap()).await;
            })
        });
        let next = || rx.recv_timeout(Duration::from_secs(5)).expect("a report");
        assert_eq!(next(), None, "no boltd yet");

        let calls = Calls::default();
        let bolt = zbus::block_on(fake_bolt(&bus.1, &calls));
        let mut f = next();
        while f.as_ref().is_none_or(|f| f.devices.is_empty()) {
            f = next();
        }
        assert_eq!(f.unwrap().devices, [dock(Status::Connected)]);

        zbus::block_on(set_status(&bolt, DOCK, "authorized"));
        let mut f = next();
        while f.as_ref().unwrap().devices[0].status != Status::Authorized {
            f = next();
        }

        zbus::block_on(async {
            let server = bolt.object_server();
            server
                .at(DRIVE, fake("drive-uid", "peripheral", "connected", &calls))
                .await
                .unwrap();
            let m = server.interface::<_, FakeManager>(PATH).await.unwrap();
            m.get_mut()
                .await
                .devices
                .push(OwnedObjectPath::try_from(DRIVE).unwrap());
            FakeManager::device_added(
                m.signal_emitter(),
                OwnedObjectPath::try_from(DRIVE).unwrap(),
            )
            .await
            .unwrap();
        });
        let mut f = next();
        while f.as_ref().unwrap().devices.len() < 2 {
            f = next();
        }

        drop(bolt);
        let mut f = next();
        while f.is_some() {
            f = next();
        }
    }

    #[test]
    fn each_change_makes_its_calls() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let calls = Calls::default();
        zbus::block_on(async {
            let _bolt = fake_bolt(&bus.1, &calls).await;
            let conn = connect(&bus.1).await;
            let d = dock(Status::Connected);
            apply(&conn, &d, Change::Authorize).await.unwrap();
            apply(&conn, &d, Change::Always).await.unwrap();
            let stored = Device {
                stored: true,
                ..d.clone()
            };
            apply(&conn, &stored, Change::Always).await.unwrap();
            let away = Device {
                status: Status::Disconnected,
                ..stored.clone()
            };
            apply(&conn, &away, Change::Always).await.unwrap();
            apply(&conn, &stored, Change::Forget).await.unwrap();
        });
        assert_eq!(
            *calls.lock().unwrap(),
            [
                "Authorize dock-uid \"\"",
                "EnrollDevice dock-uid auto \"\"",
                "Policy dock-uid auto",
                "Authorize dock-uid \"\"",
                "Policy dock-uid auto",
                "ForgetDevice dock-uid",
            ]
        );
    }
}
