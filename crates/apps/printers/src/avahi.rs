//! The network printers avahi-daemon sees, as the Add Printer Wizard lists them. Browsing
//! DNS-SD needs no admin rights, where CUPS-Get-Devices does in cupsd's default policy.

use std::collections::{BTreeMap, HashSet};
use std::num::NonZeroUsize;

use zbus::message::Type;
use zbus::{Connection, MatchRule, Message, MessageStream, OwnedObjectPath, Proxy};

use crate::cups::Device;

const DEST: &str = "org.freedesktop.Avahi";
const SERVER: &str = "org.freedesktop.Avahi.Server";
const BROWSER: &str = "org.freedesktop.Avahi.ServiceBrowser";
const UNSPEC: i32 = -1;
/// `AVAHI_LOOKUP_RESULT_LOCAL`: the service is published by this computer.
const LOCAL: u32 = 8;

/// The service types CUPS's dnssd backend reports printers for, the IPP ones first: a
/// printer announced as several shows as the first.
pub const TYPES: [&str; 4] = [
    "_ipp._tcp",
    "_ipps._tcp",
    "_pdl-datastream._tcp",
    "_printer._tcp",
];

/// A service as ItemNew and ItemRemove name it: interface, protocol, name, type, domain.
type Key = (i32, i32, String, String, String);

/// The device a resolved service stands for, in the shape CUPS's dnssd backend reports it, so
/// the two lists merge and cupsd resolves the name again when it prints. `None` for a queue
/// this computer's own cupsd shares.
pub fn device(name: &str, kind: &str, domain: &str, txt: &[Vec<u8>], flags: u32) -> Option<Device> {
    let pairs: Vec<(String, String)> = txt
        .iter()
        .filter_map(|e| {
            let e = String::from_utf8_lossy(e);
            let (k, v) = e.split_once('=')?;
            Some((k.to_ascii_lowercase(), v.to_string()))
        })
        .collect();
    let get = |k: &str| {
        pairs
            .iter()
            .find(|(key, v)| key == k && !v.is_empty())
            .map(|(_, v)| v.as_str())
    };
    let shared = get("printer-type").is_some();
    if shared && flags & LOCAL != 0 {
        return None;
    }
    let host = format!("{}.{kind}.{domain}", escape_label(name));
    let mut uri = format!("dnssd://{}/", percent(&host));
    if shared {
        uri.push_str("cups");
    }
    if let Some(uuid) = get("uuid") {
        uri.push_str("?uuid=");
        uri.push_str(&percent(uuid));
    }
    let device_id: String = pairs
        .iter()
        .filter_map(|(k, v)| Some(format!("{}:{v};", k.strip_prefix("usb_")?.to_uppercase())))
        .collect();
    let make = get("usb_mfg")
        .or_else(|| get("usb_manu"))
        .or_else(|| get("usb_manufacturer"));
    let model = get("usb_mdl").or_else(|| get("usb_model"));
    let make_model = match (make, model) {
        (Some(m), Some(d)) if d.starts_with(m) => d.to_string(),
        (Some(m), Some(d)) => format!("{m} {d}"),
        _ => get("ty")
            .map(|t| t.split(',').next().unwrap_or(t).to_string())
            .or_else(|| {
                get("product").map(|p| p.trim_start_matches('(').trim_end_matches(')').into())
            })
            .unwrap_or_default(),
    };
    Some(Device {
        uri,
        info: name.to_string(),
        make_model,
        class: "network".into(),
        device_id,
    })
}

/// A DNS label as avahi joins it into a full name: a dot or backslash in the name is escaped.
fn escape_label(name: &str) -> String {
    name.replace('\\', "\\\\").replace('.', "\\.")
}

fn percent(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The DNS-SD service a URI names in its host, as cupsd's dnssd and driverless backends write
/// it ("ipps://Lab._ipps._tcp.local/"): the same printer under another type or scheme.
pub fn service_name(uri: &str) -> Option<String> {
    let rest = uri.split_once("://")?.1;
    let host = unpercent(rest.split(['/', '?']).next()?);
    let at = TYPES
        .iter()
        .filter_map(|t| host.find(&format!(".{t}.")))
        .min()?;
    Some(
        host[..at]
            .replace("\\.", ".")
            .replace("\\\\", "\\")
            .to_lowercase(),
    )
}

fn unpercent(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = (b[i] == b'%')
            .then(|| s.get(i + 1..i + 3))
            .flatten()
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(v) => {
                out.push(v);
                i += 3;
            }
            None => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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

fn server(conn: &Connection) -> zbus::Result<Proxy<'static>> {
    reprisebus::wait(Proxy::new(conn, DEST, "/", SERVER))
}

/// Ask avahi-daemon for a browser per printer type. An error means no avahi-daemon (or no
/// system bus): the wizard asks cupsd instead.
pub fn start(conn: &Connection) -> zbus::Result<MessageStream> {
    // Subscribed before the browsers exist: their first signals can beat the method reply.
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(BROWSER)
        .build()?;
    let signals = reprisebus::wait(MessageStream::for_match_rule(
        rule,
        conn,
        NonZeroUsize::new(1024),
    ))?;
    let server = server(conn)?;
    for kind in TYPES {
        let _: OwnedObjectPath =
            reprisebus::wait(server.call("ServiceBrowserNew", &(UNSPEC, UNSPEC, kind, "", 0u32)))?;
    }
    Ok(signals)
}

/// Follow the browsers until the connection closes. `changed` gets the whole list once every
/// browser has sent what avahi-daemon knew, then on each change.
pub fn run(conn: &Connection, signals: MessageStream, mut changed: impl FnMut(Vec<Device>)) {
    let Ok(server) = server(conn) else {
        return;
    };
    let mut services = Services::default();
    let mut done: HashSet<String> = HashSet::new();
    for msg in reprisebus::blocking_iter(signals) {
        let Ok(msg) = msg else {
            break;
        };
        let ready = done.len() >= TYPES.len();
        let moved = match member(&msg).as_deref() {
            Some("ItemNew") => match item(&msg) {
                Some((key, _)) => match services.known(&key).or_else(|| resolve(&server, &key)) {
                    Some(d) => services.add(key, d),
                    None => false,
                },
                None => false,
            },
            Some("ItemRemove") => item(&msg).is_some_and(|(key, _)| services.remove(&key)),
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

fn member(msg: &Message) -> Option<String> {
    msg.header().member().map(|m| m.to_string())
}

fn item(msg: &Message) -> Option<(Key, u32)> {
    let (iface, proto, name, kind, domain, flags): (i32, i32, String, String, String, u32) =
        msg.body().deserialize().ok()?;
    Some(((iface, proto, name, kind, domain), flags))
}

type Resolved = (
    i32,
    i32,
    String,
    String,
    String,
    String,
    i32,
    String,
    u16,
    Vec<Vec<u8>>,
    u32,
);

fn resolve(server: &Proxy, key: &Key) -> Option<Device> {
    let (iface, proto, name, kind, domain) = key;
    let r: zbus::Result<Resolved> = reprisebus::wait(server.call(
        "ResolveService",
        &(*iface, *proto, name, kind, domain, UNSPEC, 0u32),
    ));
    match r {
        Ok((.., txt, flags)) => device(name, kind, domain, &txt, flags),
        Err(e) => {
            log::debug!("resolving {name} ({kind}): {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn txt(entries: &[&str]) -> Vec<Vec<u8>> {
        entries.iter().map(|e| e.as_bytes().to_vec()).collect()
    }

    fn key(iface: i32, name: &str, kind: &str) -> Key {
        (iface, 0, name.into(), kind.into(), "local".into())
    }

    #[test]
    fn a_resolved_ipp_service_is_a_dnssd_device_like_cups_reports() {
        let d = device(
            "Office Jet",
            "_ipp._tcp",
            "local",
            &txt(&[
                "txtvers=1",
                "rp=ipp/print",
                "ty=HP OfficeJet 5200 series,1.0",
                "usb_MFG=HP",
                "usb_MDL=OfficeJet 5200",
                "UUID=0b2c-11",
                "pdl=application/pdf",
            ]),
            0,
        )
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
        let d = device(
            "Lab",
            "_pdl-datastream._tcp",
            "local",
            &txt(&["ty=Brother HL-L2350DW series"]),
            0,
        )
        .expect("a network printer");
        assert_eq!(d.uri, "dnssd://Lab._pdl-datastream._tcp.local/");
        assert_eq!(d.make_model, "Brother HL-L2350DW series");
        assert!(!d.driverless(), "a raw socket printer needs a driver");
        assert_eq!(d.maker(), "Brother");

        let d = device(
            "Lab",
            "_printer._tcp",
            "local",
            &txt(&["product=(Laser 9)"]),
            0,
        )
        .expect("a network printer");
        assert_eq!(d.make_model, "Laser 9");
        assert_eq!(d.device_id, "");
    }

    #[test]
    fn a_shared_cups_queue_has_the_cups_path_and_this_computers_own_is_left_out() {
        let shared = txt(&["printer-type=0x809046", "UUID=u1"]);
        let d = device("Hall @ desk", "_ipp._tcp", "local", &shared, 0).expect("a shared queue");
        assert_eq!(
            d.uri,
            "dnssd://Hall%20%40%20desk._ipp._tcp.local/cups?uuid=u1"
        );
        assert_eq!(device("Hall", "_ipp._tcp", "local", &shared, LOCAL), None);
        assert!(
            device("Hall", "_ipp._tcp", "local", &txt(&["UUID=u1"]), LOCAL).is_some(),
            "a printer announced from this computer that is not a cupsd queue is listed"
        );
    }

    #[test]
    fn a_dot_in_the_name_is_escaped_as_avahi_joins_it() {
        let d = device("Lab v2.1", "_ipps._tcp", "local", &[], 0).expect("a printer");
        assert_eq!(d.uri, "dnssd://Lab%20v2%5C.1._ipps._tcp.local/");
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
        let lab = |kind: &str| device("Lab", kind, "local", &[], 0).expect("a printer");
        assert!(s.add(key(2, "Lab", "_ipp._tcp"), lab("_ipp._tcp")));
        assert!(!s.add(key(3, "Lab", "_ipp._tcp"), lab("_ipp._tcp")));
        assert_eq!(s.known(&key(4, "Lab", "_ipp._tcp")), Some(lab("_ipp._tcp")));
        assert!(s.add(key(2, "Lab", "_ipps._tcp"), lab("_ipps._tcp")));
        assert!(s.add(
            key(2, "Attic", "_ipp._tcp"),
            device("Attic", "_ipp._tcp", "local", &[], 0).unwrap()
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
