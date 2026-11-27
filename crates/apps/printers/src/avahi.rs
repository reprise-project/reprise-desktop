//! The network printers avahi-daemon sees, as the Add Printer Wizard lists them. Browsing
//! DNS-SD needs no admin rights, where CUPS-Get-Devices does in cupsd's default policy.

use std::collections::{BTreeMap, HashSet};

use reprisebus::avahi::{self as bus, Key, Service};
use zbus::{Connection, MessageStream};

use crate::cups::Device;

/// The service types CUPS's dnssd backend reports printers for.
pub const TYPES: [&str; 4] = bus::PRINTER_TYPES;

/// The device a resolved service stands for, in the shape CUPS's dnssd backend reports it, so
/// the two lists merge and cupsd resolves the name again when it prints. `None` for a queue
/// this computer's own cupsd shares.
pub fn device(s: &Service) -> Option<Device> {
    if s.own_queue() {
        return None;
    }
    let device_id: String = s
        .txt
        .iter()
        .filter_map(|(k, v)| Some(format!("{}:{v};", k.strip_prefix("usb_")?.to_uppercase())))
        .collect();
    let make = s
        .get("usb_mfg")
        .or_else(|| s.get("usb_manu"))
        .or_else(|| s.get("usb_manufacturer"));
    let model = s.get("usb_mdl").or_else(|| s.get("usb_model"));
    let make_model = match (make, model) {
        (Some(m), Some(d)) if d.starts_with(m) => d.to_string(),
        (Some(m), Some(d)) => format!("{m} {d}"),
        _ => s
            .get("ty")
            .map(|t| t.split(',').next().unwrap_or(t).to_string())
            .or_else(|| {
                s.get("product")
                    .map(|p| p.trim_start_matches('(').trim_end_matches(')').into())
            })
            .unwrap_or_default(),
    };
    Some(Device {
        uri: s.uri(),
        info: s.name.clone(),
        make_model,
        class: "network".into(),
        device_id,
    })
}

/// A service as avahi-daemon resolves it, for tests.
#[cfg(test)]
pub fn announced(name: &str, kind: &str, txt: &[&str], flags: u32) -> Service {
    let txt: Vec<Vec<u8>> = txt.iter().map(|e| e.as_bytes().to_vec()).collect();
    Service {
        name: name.into(),
        kind: kind.into(),
        domain: "local".into(),
        txt: bus::txt_pairs(&txt),
        flags,
    }
}

/// The DNS-SD service a URI names in its host ([`bus::service_name`]).
pub fn service_name(uri: &str) -> Option<String> {
    bus::service_name(uri, &TYPES)
}

/// The services browsed so far. One announced on several interfaces or over IPv4 and IPv6 is
/// one device until the last of them goes.
#[derive(Debug, Default)]
pub struct Services {
    found: BTreeMap<Key, Device>,
}

impl Services {
    /// True when the list changed.
    pub fn add(&mut self, key: Key, d: Device) -> bool {
        let before = self.devices();
        self.found.insert(key, d);
        self.devices() != before
    }

    /// True when the list changed.
    pub fn remove(&mut self, key: &Key) -> bool {
        let before = self.devices();
        self.found.remove(key);
        self.devices() != before
    }

    /// The device already resolved for this name, type and domain on another interface.
    fn known(&self, key: &Key) -> Option<Device> {
        self.found
            .iter()
            .find(|(k, _)| (&k.2, &k.3, &k.4) == (&key.2, &key.3, &key.4))
            .map(|(_, d)| d.clone())
    }

    pub fn devices(&self) -> Vec<Device> {
        let rank = |k: &Key| TYPES.iter().position(|t| *t == k.3).unwrap_or(TYPES.len());
        let mut keyed: Vec<(&Key, &Device)> = self.found.iter().collect();
        keyed.sort_by_key(|(k, d)| (d.info.to_lowercase(), rank(k)));
        let mut seen = HashSet::new();
        keyed
            .into_iter()
            .filter(|(_, d)| seen.insert(d.uri.clone()))
            .map(|(_, d)| d.clone())
            .collect()
    }
}

/// Ask avahi-daemon for a browser per printer type. An error means no avahi-daemon (or no
/// system bus): the wizard asks cupsd instead.
pub fn start(conn: &Connection) -> zbus::Result<MessageStream> {
    bus::start(conn, &TYPES).map(|(signals, _)| signals)
}

/// Follow the browsers until the connection closes. `changed` gets the whole list once every
/// browser has sent what avahi-daemon knew, then on each change.
pub fn run(conn: &Connection, signals: MessageStream, mut changed: impl FnMut(Vec<Device>)) {
    let Ok(server) = bus::server(conn) else {
        return;
    };
    let resolve = |key: &Key| reprisebus::wait(bus::resolve(&server, key)).and_then(|s| device(&s));
    let mut services = Services::default();
    let mut done: HashSet<String> = HashSet::new();
    for msg in reprisebus::blocking_iter(signals) {
        let Ok(msg) = msg else {
            break;
        };
        let ready = done.len() >= TYPES.len();
        let moved = match bus::member(&msg).as_deref() {
            Some("ItemNew") => match bus::item(&msg) {
                Some((key, _)) => match services.known(&key).or_else(|| resolve(&key)) {
                    Some(d) => services.add(key, d),
                    None => false,
                },
                None => false,
            },
            Some("ItemRemove") => bus::item(&msg).is_some_and(|(key, _)| services.remove(&key)),
            Some("AllForNow" | "Failure") => {
                if let Some(path) = msg.header().path() {
                    done.insert(path.to_string());
                }
                !ready && done.len() >= TYPES.len()
            }
            _ => false,
        };
        if moved && done.len() >= TYPES.len() {
            changed(services.devices());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(iface: i32, name: &str, kind: &str) -> Key {
        (iface, 0, name.into(), kind.into(), "local".into())
    }

    #[test]
    fn a_resolved_ipp_service_is_a_dnssd_device_like_cups_reports() {
        let d = device(&announced(
            "Office Jet",
            "_ipp._tcp",
            &[
                "txtvers=1",
                "rp=ipp/print",
                "ty=HP OfficeJet 5200 series,1.0",
                "usb_MFG=HP",
                "usb_MDL=OfficeJet 5200",
                "UUID=0b2c-11",
                "pdl=application/pdf",
            ],
            0,
        ))
        .expect("a network printer");
        assert_eq!(d.uri, "dnssd://Office%20Jet._ipp._tcp.local/?uuid=0b2c-11");
        assert_eq!(d.info, "Office Jet");
        assert_eq!(d.make_model, "HP OfficeJet 5200");
        assert_eq!(d.device_id, "MFG:HP;MDL:OfficeJet 5200;");
        assert_eq!(d.class, "network");
        assert!(d.driverless());
        assert_eq!(d.maker(), "HP");
        assert_eq!(d.label(), "Office Jet (network, no driver needed)");
    }

    #[test]
    fn a_service_without_usb_keys_takes_its_model_from_ty_or_product() {
        let d = device(&announced(
            "Lab",
            "_pdl-datastream._tcp",
            &["ty=Brother HL-L2350DW series"],
            0,
        ))
        .expect("a network printer");
        assert_eq!(d.uri, "dnssd://Lab._pdl-datastream._tcp.local/");
        assert_eq!(d.make_model, "Brother HL-L2350DW series");
        assert!(!d.driverless(), "a raw socket printer needs a driver");
        assert_eq!(d.maker(), "Brother");

        let d = device(&announced(
            "Lab",
            "_printer._tcp",
            &["product=(Laser 9)"],
            0,
        ))
        .expect("a network printer");
        assert_eq!(d.make_model, "Laser 9");
        assert_eq!(d.device_id, "");
    }

    #[test]
    fn a_shared_cups_queue_has_the_cups_path_and_this_computers_own_is_left_out() {
        let shared = &["printer-type=0x809046", "UUID=u1"];
        let d = device(&announced("Hall @ desk", "_ipp._tcp", shared, 0)).expect("a shared queue");
        assert_eq!(
            d.uri,
            "dnssd://Hall%20%40%20desk._ipp._tcp.local/cups?uuid=u1"
        );
        assert_eq!(
            device(&announced("Hall", "_ipp._tcp", shared, bus::LOCAL)),
            None
        );
        assert!(
            device(&announced("Hall", "_ipp._tcp", &["UUID=u1"], bus::LOCAL)).is_some(),
            "a printer announced from this computer that is not a cupsd queue is listed"
        );
    }

    #[test]
    fn the_service_a_uri_names_is_read_from_its_host() {
        assert_eq!(
            service_name("ipps://Reprise%20Test%20Printer._ipps._tcp.local/").as_deref(),
            Some("reprise test printer")
        );
        assert_eq!(
            service_name("dnssd://Lab%20v2%5C.1._ipp._tcp.local/cups?uuid=1").as_deref(),
            Some("lab v2.1")
        );
        assert_eq!(service_name("ipp://10.0.0.7/ipp/print"), None);
        assert_eq!(service_name("usb://HP/LaserJet?serial=1"), None);
    }

    #[test]
    fn a_service_on_two_interfaces_is_one_device_until_both_go() {
        let mut s = Services::default();
        let lab = |kind: &str| device(&announced("Lab", kind, &[], 0)).expect("a printer");
        assert!(s.add(key(2, "Lab", "_ipp._tcp"), lab("_ipp._tcp")));
        assert!(!s.add(key(3, "Lab", "_ipp._tcp"), lab("_ipp._tcp")));
        assert_eq!(s.known(&key(4, "Lab", "_ipp._tcp")), Some(lab("_ipp._tcp")));
        assert!(s.add(key(2, "Lab", "_ipps._tcp"), lab("_ipps._tcp")));
        assert!(s.add(
            key(2, "Attic", "_ipp._tcp"),
            device(&announced("Attic", "_ipp._tcp", &[], 0)).unwrap()
        ));
        let uris: Vec<String> = s.devices().into_iter().map(|d| d.uri).collect();
        assert_eq!(
            uris,
            [
                "dnssd://Attic._ipp._tcp.local/",
                "dnssd://Lab._ipp._tcp.local/",
                "dnssd://Lab._ipps._tcp.local/"
            ]
        );
        assert!(!s.remove(&key(2, "Lab", "_ipp._tcp")));
        assert!(s.remove(&key(3, "Lab", "_ipp._tcp")));
        assert_eq!(s.devices().len(), 2);
    }
}
