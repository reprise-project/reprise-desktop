//! The battery charge limit UPower offers where the kernel driver has `charge_control_*_threshold`:
//! whether it is on, where charging stops, and turning it on or off. UPower holds the thresholds and
//! asks polkit; Reprise only calls `EnableChargeThreshold`.

use std::collections::HashMap;
use std::time::Duration;
use zbus::proxy::{CacheProperties, MethodFlags};
use zbus::{Connection, OwnedObjectPath, OwnedVariant, Proxy};

pub const NAME: &str = "org.freedesktop.UPower";
const PATH: &str = "/org/freedesktop/UPower";
pub const DEVICE: &str = "org.freedesktop.UPower.Device";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

const TIMEOUT: Duration = Duration::from_secs(5);

/// UPower's `Type` for a battery.
const BATTERY: u32 = 2;

/// The `ChargeThresholdSettingsSupported` bit saying the end threshold applies.
const END_SETTING: u32 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChargeLimit {
    /// The battery's object path, which `EnableChargeThreshold` is called on.
    pub device: String,
    pub enabled: bool,
    /// Percent where charging stops; `None` when the firmware picks.
    pub end: Option<u8>,
}

impl ChargeLimit {
    /// The limit a device's properties describe: a system battery reporting `ChargeThresholdSupported`.
    pub fn from_props(device: &str, props: &HashMap<String, OwnedVariant>) -> Option<ChargeLimit> {
        let flag = |k: &str| {
            props
                .get(k)
                .and_then(|v| bool::try_from(v.clone()).ok())
                .unwrap_or(false)
        };
        let num = |k: &str| props.get(k).and_then(|v| u32::try_from(v.clone()).ok());
        if num("Type") != Some(BATTERY) || !flag("PowerSupply") || !flag("ChargeThresholdSupported")
        {
            return None;
        }
        let end_applies =
            num("ChargeThresholdSettingsSupported").is_none_or(|s| s & END_SETTING != 0);
        let end = num("ChargeEndThreshold")
            .filter(|e| end_applies && (1..=100).contains(e))
            .map(|e| e as u8);
        Some(ChargeLimit {
            device: device.to_string(),
            enabled: flag("ChargeThresholdEnabled"),
            end,
        })
    }
}

/// Whether a `PropertiesChanged` naming `keys` may move the limit.
pub fn touches_limit<'a>(mut keys: impl Iterator<Item = &'a str>) -> bool {
    keys.any(|k| k.starts_with("ChargeThreshold") || k.ends_with("Threshold"))
}

async fn device_props(conn: &Connection, path: &str) -> Option<HashMap<String, OwnedVariant>> {
    let props = zbus::proxy::Builder::<Proxy>::new(conn)
        .destination(NAME)
        .path(path)
        .interface(PROPERTIES)
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .ok()?;
    props.call("GetAll", &(DEVICE,)).await.ok()
}

/// The first battery UPower lists that supports a charge limit; `None` without UPower, a battery, or
/// support.
pub async fn find(conn: &Connection) -> Option<ChargeLimit> {
    let upower = Proxy::new(conn, NAME, PATH, NAME).await.ok()?;
    let devices: Vec<OwnedObjectPath> = match upower.call("EnumerateDevices", &()).await {
        Ok(d) => d,
        Err(e) => {
            log::info!("charge limit: UPower unavailable ({e})");
            return None;
        }
    };
    for path in devices {
        if let Some(props) = device_props(conn, path.as_str()).await
            && let Some(limit) = ChargeLimit::from_props(path.as_str(), &props)
        {
            return Some(limit);
        }
    }
    None
}

/// Turn the limit on `device` on or off; polkit may ask for a password through the session's agent.
pub async fn enable(conn: &Connection, device: &str, on: bool) -> zbus::Result<()> {
    let battery = Proxy::new(conn, NAME, device.to_string(), DEVICE).await?;
    battery
        .call_with_flags::<_, _, ()>(
            "EnableChargeThreshold",
            MethodFlags::ALLOW_INTERACTIVE_AUTH,
            &(on,),
        )
        .await
        .map(|_| ())
}

/// [`find`] on a system bus connection of its own, blocking, each call cut off after 5 s.
pub fn read() -> Option<ChargeLimit> {
    zbus::block_on(async {
        let conn = zbus::connection::Builder::system()
            .method_timeout(TIMEOUT)
            .build()
            .await
            .ok()?;
        find(&conn).await
    })
}

/// [`enable`] on a system bus connection of its own, blocking until polkit and UPower answer. The
/// error is what to tell the user; `None` when they dismissed the password prompt.
pub fn set(device: &str, on: bool) -> Result<(), Option<String>> {
    let e = match zbus::block_on(async {
        let conn = Connection::system().await?;
        enable(&conn, device, on).await
    }) {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    log::warn!("charge limit {}: {e}", if on { "on" } else { "off" });
    // UPower answers GeneralError for a refusal and a failed write alike.
    match e {
        zbus::Error::MethodError(..) if crate::polkit::dismissed() => Err(None),
        zbus::Error::MethodError(_, Some(msg), _) => Err(Some(msg)),
        e => Err(Some(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex};
    use zbus::Variant;

    fn props(entries: Vec<(&str, Variant<'static>)>) -> HashMap<String, OwnedVariant> {
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.try_to_owned().unwrap()))
            .collect()
    }

    fn battery(extra: Vec<(&'static str, Variant<'static>)>) -> HashMap<String, OwnedVariant> {
        let mut all = vec![
            ("Type", Variant::from(2u32)),
            ("PowerSupply", Variant::from(true)),
            ("ChargeThresholdSupported", Variant::from(true)),
            ("ChargeThresholdEnabled", Variant::from(true)),
            ("ChargeStartThreshold", Variant::from(75u32)),
            ("ChargeEndThreshold", Variant::from(80u32)),
            ("ChargeThresholdSettingsSupported", Variant::from(3u32)),
        ];
        for (k, v) in extra {
            all.retain(|(key, _)| *key != k);
            all.push((k, v));
        }
        props(all)
    }

    const BAT0: &str = "/org/freedesktop/UPower/devices/battery_BAT0";

    #[test]
    fn a_supported_battery_reports_its_end_threshold() {
        assert_eq!(
            ChargeLimit::from_props(BAT0, &battery(vec![])),
            Some(ChargeLimit {
                device: BAT0.into(),
                enabled: true,
                end: Some(80),
            })
        );
        let off = battery(vec![("ChargeThresholdEnabled", Variant::from(false))]);
        assert_eq!(
            ChargeLimit::from_props(BAT0, &off).map(|l| l.enabled),
            Some(false)
        );
    }

    #[test]
    fn an_unset_or_firmware_picked_end_is_none() {
        let end = |extra| ChargeLimit::from_props(BAT0, &battery(extra)).and_then(|l| l.end);
        assert_eq!(
            end(vec![("ChargeEndThreshold", Variant::from(u32::MAX))]),
            None
        );
        assert_eq!(end(vec![("ChargeEndThreshold", Variant::from(0u32))]), None);
        assert_eq!(
            end(vec![(
                "ChargeThresholdSettingsSupported",
                Variant::from(4u32)
            )]),
            None,
            "only firmware-controlled charging"
        );
        let mut no_bitmask = battery(vec![]);
        no_bitmask.remove("ChargeThresholdSettingsSupported");
        assert_eq!(
            ChargeLimit::from_props(BAT0, &no_bitmask).and_then(|l| l.end),
            Some(80),
            "UPower before the bitmask"
        );
    }

    #[test]
    fn only_a_system_battery_with_support_has_a_limit() {
        let none = |extra| ChargeLimit::from_props(BAT0, &battery(extra));
        assert_eq!(
            none(vec![("ChargeThresholdSupported", Variant::from(false))]),
            None
        );
        assert_eq!(
            none(vec![("PowerSupply", Variant::from(false))]),
            None,
            "a peripheral"
        );
        assert_eq!(
            none(vec![("Type", Variant::from(1u32))]),
            None,
            "line power"
        );
        assert_eq!(
            ChargeLimit::from_props(BAT0, &HashMap::new()),
            None,
            "UPower before 1.90"
        );
    }

    #[test]
    fn threshold_keys_touch_the_limit() {
        assert!(touches_limit(["ChargeThresholdEnabled"].into_iter()));
        assert!(touches_limit(
            ["Percentage", "ChargeEndThreshold"].into_iter()
        ));
        assert!(!touches_limit(
            ["Percentage", "State", "Energy"].into_iter()
        ));
    }

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

    const LINE: &str = "/org/freedesktop/UPower/devices/line_power_AC";

    struct FakeUPower;

    #[zbus::interface(name = "org.freedesktop.UPower")]
    impl FakeUPower {
        fn enumerate_devices(&self) -> Vec<OwnedObjectPath> {
            [LINE, BAT0]
                .into_iter()
                .map(|p| OwnedObjectPath::try_from(p).unwrap())
                .collect()
        }
    }

    struct FakeDevice {
        battery: bool,
        enabled: Arc<Mutex<bool>>,
    }

    #[zbus::interface(name = "org.freedesktop.UPower.Device")]
    impl FakeDevice {
        fn enable_charge_threshold(&self, on: bool) {
            *self.enabled.lock().unwrap() = on;
        }
        #[zbus(property, name = "Type")]
        fn kind(&self) -> u32 {
            if self.battery { 2 } else { 1 }
        }
        #[zbus(property)]
        fn power_supply(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn charge_threshold_supported(&self) -> bool {
            self.battery
        }
        #[zbus(property)]
        fn charge_threshold_enabled(&self) -> bool {
            *self.enabled.lock().unwrap()
        }
        #[zbus(property)]
        fn charge_end_threshold(&self) -> u32 {
            80
        }
    }

    #[test]
    fn the_battery_s_limit_is_found_and_turned_on() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async move {
            let enabled = Arc::new(Mutex::new(false));
            let upower = connect(&bus.1).await;
            let server = upower.object_server();
            server.at(PATH, FakeUPower).await.unwrap();
            let line = FakeDevice {
                battery: false,
                enabled: Arc::default(),
            };
            server.at(LINE, line).await.unwrap();
            let bat = FakeDevice {
                battery: true,
                enabled: enabled.clone(),
            };
            server.at(BAT0, bat).await.unwrap();
            upower.request_name(NAME).await.unwrap();

            let conn = connect(&bus.1).await;
            let found = find(&conn).await;
            assert_eq!(
                found,
                Some(ChargeLimit {
                    device: BAT0.into(),
                    enabled: false,
                    end: Some(80),
                })
            );
            enable(&conn, BAT0, true).await.unwrap();
            assert!(*enabled.lock().unwrap());
            assert_eq!(find(&conn).await.map(|l| l.enabled), Some(true));
        });
    }

    #[test]
    fn no_upower_means_no_limit() {
        let Some(bus) = bus() else {
            return;
        };
        let conn = zbus::block_on(connect(&bus.1));
        assert_eq!(zbus::block_on(find(&conn)), None);
    }
}
