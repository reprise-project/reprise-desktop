//! BlueZ's devices on the system bus: the paired ones Settings lists, and whether the one named
//! in `[lock] away_device` is connected, which the compositor follows.

use std::collections::HashMap;

use futures_util::stream::StreamExt;
use zbus::{Connection, Message, OwnedMatchRule, OwnedObjectPath, OwnedVariant};

use crate::Signal;

pub const NAME: &str = "org.bluez";
const ROOT: &str = "/";
const BLUEZ_PATH: &str = "/org/bluez";
const DEVICE: &str = "org.bluez.Device1";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    pub path: String,
    pub address: String,
    /// `Alias`, which BlueZ fills from the device's own name until the user renames it.
    pub name: String,
    pub paired: bool,
    pub connected: bool,
}

impl Device {
    /// Whether `setting` (an `[lock] away_device` value) names this device.
    pub fn is(&self, setting: &str) -> bool {
        !setting.is_empty() && (self.path == setting || self.address.eq_ignore_ascii_case(setting))
    }
}

type Interfaces = HashMap<String, HashMap<String, OwnedVariant>>;

fn device(path: &str, interfaces: &Interfaces) -> Option<Device> {
    let props = interfaces.get(DEVICE)?;
    let text = |k: &str| {
        props
            .get(k)
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_default()
    };
    let flag = |k: &str| {
        props
            .get(k)
            .and_then(|v| bool::try_from(v.clone()).ok())
            .unwrap_or(false)
    };
    Some(Device {
        path: path.to_string(),
        address: text("Address"),
        name: text("Alias"),
        paired: flag("Paired"),
        connected: flag("Connected"),
    })
}

/// Every device BlueZ knows, from its object manager.
pub async fn devices(conn: &Connection) -> zbus::Result<Vec<Device>> {
    let reply = conn
        .call_method(
            Some(NAME),
            ROOT,
            Some(OBJECT_MANAGER),
            "GetManagedObjects",
            &(),
        )
        .await?;
    let objects: HashMap<OwnedObjectPath, Interfaces> = reply.body().deserialize()?;
    Ok(objects
        .iter()
        .filter_map(|(path, ifaces)| device(path.as_str(), ifaces))
        .collect())
}

/// The paired devices by name, on a system bus connection of its own; empty without BlueZ.
pub fn paired() -> Vec<Device> {
    let found = zbus::block_on(async {
        let conn = zbus::connection::Builder::system()
            .method_timeout(crate::METHOD_TIMEOUT)
            .build()
            .await?;
        devices(&conn).await
    });
    let mut found: Vec<Device> = match found {
        Ok(d) => d.into_iter().filter(|d| d.paired).collect(),
        Err(e) => {
            log::info!("bluez: no devices ({e})");
            Vec::new()
        }
    };
    found.sort_by(|a, b| (&a.name, &a.address).cmp(&(&b.name, &b.address)));
    found
}

fn rules() -> Vec<OwnedMatchRule> {
    let signal = || {
        zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(NAME)
    };
    let props = signal()
        .interface(PROPERTIES)
        .member("PropertiesChanged")
        .path_namespace(BLUEZ_PATH)
        .build();
    let added = signal()
        .interface(OBJECT_MANAGER)
        .member("InterfacesAdded")
        .path(ROOT)
        .build();
    let removed = signal()
        .interface(OBJECT_MANAGER)
        .member("InterfacesRemoved")
        .path(ROOT)
        .build();
    [props, added, removed]
        .into_iter()
        .filter_map(|r| r.ok().map(Into::into))
        .collect()
}

/// `Connected` from a device's `PropertiesChanged`, when it changed.
fn connected_change(m: &Message) -> Option<bool> {
    let (iface, changed, _): (String, HashMap<String, OwnedVariant>, Vec<String>) =
        m.body().deserialize().ok()?;
    (iface == DEVICE)
        .then(|| {
            changed
                .get("Connected")
                .and_then(|v| bool::try_from(v.clone()).ok())
        })
        .flatten()
}

/// The device's path and whether it is connected; a device BlueZ does not have, or BlueZ itself
/// gone, is not.
async fn look_up(conn: &Connection, target: &str) -> (Option<String>, bool) {
    match devices(conn).await {
        Ok(all) => match all.into_iter().find(|d| d.is(target)) {
            Some(d) => (Some(d.path), d.connected),
            None => (None, false),
        },
        Err(e) => {
            log::info!("bluez: cannot list devices ({e}); {target} counts as gone");
            (None, false)
        }
    }
}

/// Calls `seen` with whether `target` (an address or object path) is connected: once at the
/// start, then on every change BlueZ broadcasts, the device or BlueZ going away being `false`.
/// Returns only when the connection closes; drop the future to stop.
pub async fn follow(conn: &Connection, target: &str, mut seen: impl FnMut(bool)) {
    // Subscribed before the first read, so a change in between is not lost.
    let mut signals = crate::signals(conn, NAME, rules()).await;
    let (mut path, connected) = look_up(conn, target).await;
    seen(connected);
    while let Some(sig) = signals.next().await {
        let m = match sig {
            Signal::OwnerChanged => None,
            Signal::Message(m) => Some(m),
        };
        let member = m
            .as_ref()
            .and_then(|m| m.header().member().map(|n| n.to_string()));
        if member.as_deref() == Some("PropertiesChanged") {
            let Some(m) = m else { continue };
            let here = m.header().path().map(|p| p.as_str()) == path.as_deref();
            if let Some(c) = connected_change(&m).filter(|_| here) {
                seen(c);
            }
            continue;
        }
        let (p, connected) = look_up(conn, target).await;
        path = p;
        seen(connected);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
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

    const PHONE: &str = "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_01";
    const HEADSET: &str = "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_02";

    struct FakeDevice {
        address: &'static str,
        alias: &'static str,
        paired: bool,
        connected: bool,
    }

    #[zbus::interface(name = "org.bluez.Device1")]
    impl FakeDevice {
        #[zbus(property)]
        fn address(&self) -> String {
            self.address.into()
        }
        #[zbus(property)]
        fn alias(&self) -> String {
            self.alias.into()
        }
        #[zbus(property)]
        fn paired(&self) -> bool {
            self.paired
        }
        #[zbus(property)]
        fn connected(&self) -> bool {
            self.connected
        }
    }

    async fn fake_bluez(address: &str) -> Connection {
        let c = connect(address).await;
        let server = c.object_server();
        server.at(ROOT, zbus::fdo::ObjectManager).await.unwrap();
        let phone = FakeDevice {
            address: "AA:BB:CC:DD:EE:01",
            alias: "Phone",
            paired: true,
            connected: true,
        };
        let headset = FakeDevice {
            address: "AA:BB:CC:DD:EE:02",
            alias: "Headset",
            paired: false,
            connected: false,
        };
        server.at(PHONE, phone).await.unwrap();
        server.at(HEADSET, headset).await.unwrap();
        c.request_name(NAME).await.unwrap();
        c
    }

    async fn set_connected(c: &Connection, on: bool) {
        let iface = c
            .object_server()
            .interface::<_, FakeDevice>(PHONE)
            .await
            .unwrap();
        iface.get_mut().await.connected = on;
        iface
            .get()
            .await
            .connected_changed(iface.signal_emitter())
            .await
            .unwrap();
    }

    #[test]
    fn the_devices_carry_their_address_name_and_states() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let mut got = zbus::block_on(async {
            let _bluez = fake_bluez(&bus.1).await;
            devices(&connect(&bus.1).await).await.unwrap()
        });
        got.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(
            got,
            [
                Device {
                    path: PHONE.into(),
                    address: "AA:BB:CC:DD:EE:01".into(),
                    name: "Phone".into(),
                    paired: true,
                    connected: true,
                },
                Device {
                    path: HEADSET.into(),
                    address: "AA:BB:CC:DD:EE:02".into(),
                    name: "Headset".into(),
                    paired: false,
                    connected: false,
                },
            ]
        );
        assert!(got[0].is("aa:bb:cc:dd:ee:01") && got[0].is(PHONE));
        assert!(!got[0].is("") && !got[1].is("AA:BB:CC:DD:EE:01"));
    }

    #[test]
    fn follow_reports_the_device_dropping_returning_and_vanishing() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let (tx, rx) = mpsc::channel();
        let (bluez_tx, bluez_rx) = mpsc::channel();
        let address = bus.1.clone();
        std::thread::spawn(move || {
            zbus::block_on(async {
                let bluez = fake_bluez(&address).await;
                bluez_tx.send(bluez).unwrap();
                let conn = connect(&address).await;
                follow(&conn, "aa:bb:cc:dd:ee:01", |c| tx.send(c).unwrap()).await;
            })
        });
        let bluez = bluez_rx.recv().unwrap();
        let next = || rx.recv_timeout(Duration::from_secs(5)).expect("a report");
        assert!(next(), "connected at the start");
        zbus::block_on(set_connected(&bluez, false));
        assert!(!next(), "dropped");
        zbus::block_on(set_connected(&bluez, true));
        assert!(next(), "back");
        zbus::block_on(async {
            bluez
                .object_server()
                .remove::<FakeDevice, _>(PHONE)
                .await
                .unwrap()
        });
        assert!(!next(), "unpaired, the device is gone");
        drop(bluez);
        while let Ok(c) = rx.recv_timeout(Duration::from_secs(2)) {
            assert!(!c, "BlueZ leaving the bus is gone too");
        }
    }
}
