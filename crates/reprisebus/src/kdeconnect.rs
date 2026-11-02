//! Phones paired through KDE Connect: `kdeconnectd`'s reachable devices on the session bus,
//! followed through its signals and restarts, and the ring and share calls made on one.

use crate::Signal;
use futures_util::FutureExt;
use futures_util::stream::StreamExt;
use std::collections::HashMap;
use zbus::{Connection, OwnedMatchRule, OwnedVariant};

pub const NAME: &str = "org.kde.kdeconnect";
const DAEMON_PATH: &str = "/modules/kdeconnect";
const DAEMON: &str = "org.kde.kdeconnect.daemon";
const DEVICE: &str = "org.kde.kdeconnect.device";
const BATTERY: &str = "org.kde.kdeconnect.device.battery";
const FIND: &str = "org.kde.kdeconnect.device.findmyphone";
const SHARE: &str = "org.kde.kdeconnect.device.share";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

const RING_PLUGIN: &str = "kdeconnect_findmyphone";
const SHARE_PLUGIN: &str = "kdeconnect_share";

/// A paired device KDE Connect reaches now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    /// KDE Connect's device type: "smartphone", "tablet", "desktop", "laptop" or "tv".
    pub kind: String,
    /// None while the battery plugin is off or the device has not reported a charge.
    pub battery: Option<Battery>,
    /// The find-my-phone plugin is loaded: [`ring`] makes it ring.
    pub ring: bool,
    /// The share plugin is loaded: [`share`] sends it files.
    pub share: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    /// Per cent, 0 to 100.
    pub charge: u8,
    pub charging: bool,
}

fn device_path(id: &str) -> String {
    format!("{DAEMON_PATH}/devices/{id}")
}

async fn get_all(
    conn: &Connection,
    path: &str,
    iface: &str,
) -> Option<HashMap<String, OwnedVariant>> {
    let reply = conn
        .call_method(Some(NAME), path, Some(PROPERTIES), "GetAll", &(iface,))
        .await
        .ok()?;
    reply.body().deserialize().ok()
}

fn prop<T: TryFrom<OwnedVariant>>(props: &HashMap<String, OwnedVariant>, key: &str) -> Option<T> {
    props.get(key)?.clone().try_into().ok()
}

/// The battery as `charge` and `isCharging` give it; a negative charge is none reported yet.
fn battery(props: &HashMap<String, OwnedVariant>) -> Option<Battery> {
    let charge: i32 = prop(props, "charge")?;
    Some(Battery {
        charge: u8::try_from(charge).ok()?.min(100),
        charging: prop(props, "isCharging").unwrap_or(false),
    })
}

async fn device(conn: &Connection, id: String) -> Option<Device> {
    let path = device_path(&id);
    let props = get_all(conn, &path, DEVICE).await?;
    let reachable: bool = prop(&props, "isReachable").unwrap_or(false);
    let paired: bool = prop(&props, "isPaired").unwrap_or(false);
    if !reachable || !paired {
        return None;
    }
    let plugins: Vec<String> = conn
        .call_method(
            Some(NAME),
            path.as_str(),
            Some(DEVICE),
            "loadedPlugins",
            &(),
        )
        .await
        .and_then(|m| m.body().deserialize())
        .unwrap_or_default();
    let battery = match plugins.iter().any(|p| p == "kdeconnect_battery") {
        true => get_all(conn, &format!("{path}/battery"), BATTERY)
            .await
            .and_then(|p| battery(&p)),
        false => None,
    };
    Some(Device {
        name: prop(&props, "name").unwrap_or_else(|| id.clone()),
        kind: prop(&props, "type").unwrap_or_default(),
        battery,
        ring: plugins.iter().any(|p| p == RING_PLUGIN),
        share: plugins.iter().any(|p| p == SHARE_PLUGIN),
        id,
    })
}

/// The paired devices kdeconnectd reaches, in its order; None while nothing owns [`NAME`].
pub async fn read(conn: &Connection) -> Option<Vec<Device>> {
    crate::name_owner(conn, NAME).await?;
    // `devices` is overloaded; (onlyReachable, onlyPaired) picks the two-flag one.
    let ids: Vec<String> = match conn
        .call_method(
            Some(NAME),
            DAEMON_PATH,
            Some(DAEMON),
            "devices",
            &(true, true),
        )
        .await
        .and_then(|m| m.body().deserialize())
    {
        Ok(ids) => ids,
        Err(e) => {
            log::info!("kdeconnect: no device list ({e})");
            return Some(Vec::new());
        }
    };
    let mut out = Vec::new();
    for id in ids {
        out.extend(device(conn, id).await);
    }
    Some(out)
}

fn rules() -> Vec<OwnedMatchRule> {
    [DAEMON, DEVICE, BATTERY]
        .into_iter()
        .filter_map(|iface| {
            zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender(NAME)
                .interface(iface)
                .build()
                .ok()
                .map(Into::into)
        })
        .collect()
}

/// Hands `on` the devices [`read`] finds, at once and after each change kdeconnectd signals
/// (its start and exit included), on a thread of its own over [`crate::session`].
pub fn follow(on: impl FnMut(Option<Vec<Device>>) + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("reprise-kdeconnect".into())
        .spawn(move || match crate::session() {
            Ok(conn) => follow_on(&conn, on),
            Err(e) => log::info!("kdeconnect: no session bus ({e})"),
        });
    if let Err(e) = spawned {
        log::warn!("kdeconnect: cannot start its thread: {e}");
    }
}

/// [`follow`] on `conn`, blocking the calling thread for as long as the bus is up. A burst of
/// signals costs one read, and `on` hears only a list that differs from the last.
pub fn follow_on(conn: &Connection, mut on: impl FnMut(Option<Vec<Device>>)) {
    crate::wait(async {
        let mut signals = crate::signals(conn, NAME, rules()).await;
        let mut last = None;
        loop {
            let now = read(conn).await;
            if last.as_ref() != Some(&now) {
                on(now.clone());
                last = Some(now);
            }
            match signals.next().await {
                None => return,
                Some(Signal::OwnerChanged | Signal::Message(_)) => {}
            }
            while let Some(Some(_)) = signals.next().now_or_never() {}
        }
    })
}

/// Rings device `id`, loud even when it is on silent.
pub async fn ring(conn: &Connection, id: &str) -> zbus::Result<()> {
    let path = format!("{}/findmyphone", device_path(id));
    conn.call_method(Some(NAME), path.as_str(), Some(FIND), "ring", &())
        .await
        .map(drop)
}

/// Sends `urls` (file URIs) to device `id`.
pub async fn share(conn: &Connection, id: &str, urls: &[String]) -> zbus::Result<()> {
    let path = format!("{}/share", device_path(id));
    conn.call_method(
        Some(NAME),
        path.as_str(),
        Some(SHARE),
        "shareUrls",
        &(urls,),
    )
    .await
    .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;
    use zbus::object_server::SignalEmitter;

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

    fn connect(address: &str) -> Connection {
        crate::connect(zbus::connection::Builder::address(address)).expect("connect")
    }

    struct Daemon {
        ids: Vec<String>,
    }

    #[zbus::interface(name = "org.kde.kdeconnect.daemon")]
    impl Daemon {
        #[zbus(name = "devices")]
        fn devices(&self, only_reachable: bool, only_paired: bool) -> Vec<String> {
            assert!(
                only_reachable && only_paired,
                "asks for reachable paired ones"
            );
            self.ids.clone()
        }

        #[zbus(signal, name = "deviceListChanged")]
        async fn device_list_changed(e: &SignalEmitter<'_>) -> zbus::Result<()>;
    }

    struct FakeDevice {
        name: String,
        reachable: bool,
        plugins: Vec<&'static str>,
    }

    #[zbus::interface(name = "org.kde.kdeconnect.device")]
    impl FakeDevice {
        #[zbus(property, name = "name")]
        fn name(&self) -> String {
            self.name.clone()
        }

        #[zbus(property, name = "type")]
        fn kind(&self) -> String {
            "smartphone".into()
        }

        #[zbus(property, name = "isReachable")]
        fn reachable(&self) -> bool {
            self.reachable
        }

        #[zbus(property, name = "isPaired")]
        fn paired(&self) -> bool {
            true
        }

        #[zbus(name = "loadedPlugins")]
        fn loaded_plugins(&self) -> Vec<String> {
            self.plugins.iter().map(|p| p.to_string()).collect()
        }
    }

    struct FakeBattery(i32, bool);

    #[zbus::interface(name = "org.kde.kdeconnect.device.battery")]
    impl FakeBattery {
        #[zbus(property, name = "charge")]
        fn charge(&self) -> i32 {
            self.0
        }

        #[zbus(property, name = "isCharging")]
        fn charging(&self) -> bool {
            self.1
        }
    }

    type Calls = Arc<Mutex<Vec<String>>>;

    struct FakeFind(Calls);

    #[zbus::interface(name = "org.kde.kdeconnect.device.findmyphone")]
    impl FakeFind {
        #[zbus(name = "ring")]
        fn ring(&self) {
            self.0.lock().unwrap().push("ring".into());
        }
    }

    struct FakeShare(Calls);

    #[zbus::interface(name = "org.kde.kdeconnect.device.share")]
    impl FakeShare {
        #[zbus(name = "shareUrls")]
        fn share_urls(&self, urls: Vec<String>) {
            self.0.lock().unwrap().extend(urls);
        }
    }

    const ALL: [&str; 3] = [
        "kdeconnect_battery",
        "kdeconnect_findmyphone",
        "kdeconnect_share",
    ];

    /// A device the fake lists: (id, name, reachable, loaded plugins, battery).
    type Fake<'a> = (
        &'a str,
        &'a str,
        bool,
        &'a [&'static str],
        Option<(i32, bool)>,
    );

    /// A kdeconnectd on `address` listing `devices`.
    fn daemon(address: &str, devices: &[Fake<'_>], calls: &Calls) -> Connection {
        let c = connect(address);
        crate::wait(async {
            let ids = devices.iter().map(|d| d.0.to_string()).collect();
            c.object_server()
                .at(DAEMON_PATH, Daemon { ids })
                .await
                .unwrap();
            for (id, name, reachable, plugins, battery) in devices {
                let path = device_path(id);
                let device = FakeDevice {
                    name: name.to_string(),
                    reachable: *reachable,
                    plugins: plugins.to_vec(),
                };
                c.object_server().at(path.as_str(), device).await.unwrap();
                if let Some((charge, charging)) = battery {
                    let at = format!("{path}/battery");
                    let b = FakeBattery(*charge, *charging);
                    c.object_server().at(at, b).await.unwrap();
                }
                let find = FakeFind(calls.clone());
                c.object_server()
                    .at(format!("{path}/findmyphone"), find)
                    .await
                    .unwrap();
                let share = FakeShare(calls.clone());
                c.object_server()
                    .at(format!("{path}/share"), share)
                    .await
                    .unwrap();
            }
            c.request_name(NAME).await.unwrap();
        });
        c
    }

    #[test]
    fn read_lists_reachable_paired_devices_with_battery_and_plugins() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let calls = Calls::default();
        let me = connect(&bus.1);
        assert_eq!(crate::wait(read(&me)), None, "no kdeconnectd");
        let _d = daemon(
            &bus.1,
            &[
                ("p1", "Pixel", true, &ALL, Some((80, true))),
                ("t1", "Tab", true, &["kdeconnect_share"], None),
                ("gone", "Old", false, &ALL, None),
                ("unknown", "Fresh", true, &ALL, Some((-1, false))),
            ],
            &calls,
        );
        let devices = crate::wait(read(&me)).expect("kdeconnectd is up");
        let pixel = Device {
            id: "p1".into(),
            name: "Pixel".into(),
            kind: "smartphone".into(),
            battery: Some(Battery {
                charge: 80,
                charging: true,
            }),
            ring: true,
            share: true,
        };
        assert_eq!(devices[0], pixel);
        assert_eq!(
            (
                devices[1].name.as_str(),
                devices[1].battery,
                devices[1].ring,
                devices[1].share
            ),
            ("Tab", None, false, true),
            "no battery plugin, no find-my-phone"
        );
        assert_eq!(devices[2].name, "Fresh");
        assert_eq!(devices[2].battery, None, "a charge of -1 is none reported");
        assert_eq!(devices.len(), 3, "an unreachable device is left out");
    }

    #[test]
    fn ring_and_share_reach_the_devices_plugins() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let calls = Calls::default();
        let _d = daemon(&bus.1, &[("p1", "Pixel", true, &ALL, None)], &calls);
        let me = connect(&bus.1);
        crate::wait(ring(&me, "p1")).unwrap();
        let urls = [
            "file:///home/u/a.jpg".to_string(),
            "file:///home/u/b%20c.txt".to_string(),
        ];
        crate::wait(share(&me, "p1", &urls)).unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            ["ring", "file:///home/u/a.jpg", "file:///home/u/b%20c.txt"]
        );
    }

    #[test]
    fn follow_hears_the_daemon_come_change_and_go() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let (tx, rx) = mpsc::channel();
        let me = connect(&bus.1);
        std::thread::spawn(move || follow_on(&me, move |d| drop(tx.send(d))));
        let next = || rx.recv_timeout(Duration::from_secs(5)).expect("a list");
        assert_eq!(next(), None, "no kdeconnectd yet");

        let calls = Calls::default();
        let d = daemon(&bus.1, &[("p1", "Pixel", true, &ALL, None)], &calls);
        assert_eq!(next().map(|v| v.len()), Some(1), "kdeconnectd came up");

        crate::wait(async {
            let server = d.object_server();
            let device = server
                .interface::<_, FakeDevice>(device_path("p1").as_str())
                .await
                .unwrap();
            device.get_mut().await.name = "Renamed".into();
            let daemon = server.interface::<_, Daemon>(DAEMON_PATH).await.unwrap();
            Daemon::device_list_changed(daemon.signal_emitter())
                .await
                .unwrap();
        });
        let renamed = next().expect("still up");
        assert_eq!(renamed[0].name, "Renamed", "a signal reads the list again");

        crate::wait(d.release_name(NAME)).unwrap();
        assert_eq!(next(), None, "kdeconnectd left the bus");
    }
}
