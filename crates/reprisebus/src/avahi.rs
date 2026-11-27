//! Printers announced over DNS-SD, as avahi-daemon browses and resolves them: the services, their
//! TXT keys, and the `dnssd://` URI cupsd prints to one through.

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use futures_lite::FutureExt;
use futures_util::StreamExt;
use zbus::message::Type;
use zbus::{Connection, MatchRule, Message, MessageStream, OwnedObjectPath, Proxy};

const DEST: &str = "org.freedesktop.Avahi";
const SERVER: &str = "org.freedesktop.Avahi.Server";
const BROWSER: &str = "org.freedesktop.Avahi.ServiceBrowser";
const UNSPEC: i32 = -1;
/// `AVAHI_LOOKUP_RESULT_LOCAL`: the service is published by this computer.
pub const LOCAL: u32 = 8;

/// The service types of printers that take IPP, the secure one first.
pub const IPP_TYPES: [&str; 2] = ["_ipps._tcp", "_ipp._tcp"];

/// The service types CUPS's dnssd backend reports printers for, the IPP ones first: a printer
/// announced as several shows as the first.
pub const PRINTER_TYPES: [&str; 4] = [
    "_ipp._tcp",
    "_ipps._tcp",
    "_pdl-datastream._tcp",
    "_printer._tcp",
];

/// A service as ItemNew and ItemRemove name it: interface, protocol, name, type, domain.
pub type Key = (i32, i32, String, String, String);

/// A resolved service. TXT keys are lower-cased; a value may be empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Service {
    pub name: String,
    pub kind: String,
    pub domain: String,
    pub txt: Vec<(String, String)>,
    /// avahi's lookup result flags ([`LOCAL`]).
    pub flags: u32,
}

impl Service {
    /// The value of TXT key `k` (lower case), when it has one.
    pub fn get(&self, k: &str) -> Option<&str> {
        self.txt
            .iter()
            .find(|(key, v)| key == k && !v.is_empty())
            .map(|(_, v)| v.as_str())
    }

    /// A queue some cupsd shares: its TXT names the queue's CUPS printer type.
    pub fn shared(&self) -> bool {
        self.get("printer-type").is_some()
    }

    /// One of this computer's own cupsd queues, announced back to it.
    pub fn own_queue(&self) -> bool {
        self.shared() && self.flags & LOCAL != 0
    }

    /// The URI CUPS's dnssd backend writes for the service, so cupsd resolves the name again when
    /// it prints: `dnssd://<name>.<type>.<domain>/`, `cups` after the slash for a shared queue,
    /// and the TXT `uuid`.
    pub fn uri(&self) -> String {
        let host = format!("{}.{}.{}", escape_label(&self.name), self.kind, self.domain);
        let mut uri = format!("dnssd://{}/", percent(&host));
        if self.shared() {
            uri.push_str("cups");
        }
        if let Some(uuid) = self.get("uuid") {
            uri.push_str("?uuid=");
            uri.push_str(&percent(uuid));
        }
        uri
    }
}

/// TXT records as `key=value` pairs, keys lower-cased; an entry without `=` is left out.
pub fn txt_pairs(txt: &[Vec<u8>]) -> Vec<(String, String)> {
    txt.iter()
        .filter_map(|e| {
            let e = String::from_utf8_lossy(e);
            let (k, v) = e.split_once('=')?;
            Some((k.to_ascii_lowercase(), v.to_string()))
        })
        .collect()
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

/// The DNS-SD service of one of `types` a URI names in its host, as cupsd's dnssd and driverless
/// backends write it ("ipps://Lab._ipps._tcp.local/"), lower-cased: the same printer under
/// another type or scheme.
pub fn service_name(uri: &str, types: &[&str]) -> Option<String> {
    let rest = uri.split_once("://")?.1;
    let host = unpercent(rest.split(['/', '?']).next()?);
    let at = types
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

pub fn server(conn: &Connection) -> zbus::Result<Proxy<'static>> {
    crate::wait(Proxy::new(conn, DEST, "/", SERVER))
}

/// Ask avahi-daemon for a browser per type in `types`: the browsers' signals, and their paths.
/// An error means no avahi-daemon (or no system bus).
pub fn start(
    conn: &Connection,
    types: &[&str],
) -> zbus::Result<(MessageStream, Vec<OwnedObjectPath>)> {
    // Subscribed before the browsers exist: their first signals can beat the method reply.
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(BROWSER)
        .build()?;
    let signals = crate::wait(MessageStream::for_match_rule(
        rule,
        conn,
        NonZeroUsize::new(1024),
    ))?;
    let server = server(conn)?;
    let mut paths = Vec::new();
    for kind in types {
        paths.push(crate::wait(
            server.call("ServiceBrowserNew", &(UNSPEC, UNSPEC, *kind, "", 0u32)),
        )?);
    }
    Ok((signals, paths))
}

pub fn member(msg: &Message) -> Option<String> {
    msg.header().member().map(|m| m.to_string())
}

/// The service an ItemNew or ItemRemove names, and its flags.
pub fn item(msg: &Message) -> Option<(Key, u32)> {
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

/// The service `key` names, resolved by avahi-daemon; `None` when it does not resolve.
pub async fn resolve(server: &Proxy<'_>, key: &Key) -> Option<Service> {
    let (iface, proto, name, kind, domain) = key;
    let r: zbus::Result<Resolved> = server
        .call(
            "ResolveService",
            &(*iface, *proto, name, kind, domain, UNSPEC, 0u32),
        )
        .await;
    match r {
        Ok((.., txt, flags)) => Some(Service {
            name: name.clone(),
            kind: kind.clone(),
            domain: domain.clone(),
            txt: txt_pairs(&txt),
            flags,
        }),
        Err(e) => {
            log::debug!("resolving {name} ({kind}): {e}");
            None
        }
    }
}

/// How long one resolve may take within [`browse`].
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);

/// The services of `types` avahi-daemon finds within `budget` (each browser's first answer),
/// each resolved once however many interfaces and protocols announce it. Empty without
/// avahi-daemon.
pub fn browse(types: &[&str], budget: Duration) -> Vec<Service> {
    let deadline = Instant::now() + budget;
    let builder = zbus::connection::Builder::system().method_timeout(RESOLVE_TIMEOUT);
    let conn = match crate::connect(builder) {
        Ok(c) => c,
        Err(e) => {
            log::info!("no system bus for network printers: {e}");
            return Vec::new();
        }
    };
    let found = browse_on(&conn, types, deadline);
    let _ = crate::wait(conn.close());
    found
}

fn browse_on(conn: &Connection, types: &[&str], deadline: Instant) -> Vec<Service> {
    let (mut signals, paths) = match start(conn, types) {
        Ok(s) => s,
        Err(e) => {
            log::info!("no network printers from avahi-daemon: {e}");
            return Vec::new();
        }
    };
    let ours = |m: &Message| {
        m.header()
            .path()
            .is_some_and(|p| paths.iter().any(|q| q.as_str() == p.as_str()))
    };
    let mut keys: Vec<Key> = Vec::new();
    let mut done: HashSet<String> = HashSet::new();
    while done.len() < paths.len() {
        let next = async { Some(signals.next().await) }.or(async {
            async_io::Timer::at(deadline).await;
            None
        });
        let Some(Some(Ok(msg))) = crate::wait(next) else {
            break;
        };
        if !ours(&msg) {
            continue;
        }
        match member(&msg).as_deref() {
            Some("ItemNew") => {
                if let Some((key, _)) = item(&msg)
                    && !keys
                        .iter()
                        .any(|k| (&k.2, &k.3, &k.4) == (&key.2, &key.3, &key.4))
                {
                    keys.push(key);
                }
            }
            Some("ItemRemove") => {
                if let Some((key, _)) = item(&msg) {
                    keys.retain(|k| k != &key);
                }
            }
            Some("AllForNow" | "Failure") => {
                if let Some(p) = msg.header().path() {
                    done.insert(p.to_string());
                }
            }
            _ => {}
        }
    }
    let Ok(server) = server(conn) else {
        return Vec::new();
    };
    let resolved = crate::wait(futures_util::future::join_all(
        keys.iter().map(|k| resolve(&server, k)),
    ));
    resolved.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(name: &str, kind: &str, txt: &[&str], flags: u32) -> Service {
        let txt: Vec<Vec<u8>> = txt.iter().map(|e| e.as_bytes().to_vec()).collect();
        Service {
            name: name.into(),
            kind: kind.into(),
            domain: "local".into(),
            txt: txt_pairs(&txt),
            flags,
        }
    }

    #[test]
    fn a_service_uri_is_the_dnssd_one_cups_writes() {
        let s = service(
            "Office Jet",
            "_ipp._tcp",
            &["rp=ipp/print", "UUID=0b2c-11"],
            0,
        );
        assert_eq!(
            s.uri(),
            "dnssd://Office%20Jet._ipp._tcp.local/?uuid=0b2c-11"
        );
        assert_eq!(s.get("uuid"), Some("0b2c-11"), "keys read lower-cased");
        let shared = service("Hall @ desk", "_ipp._tcp", &["printer-type=0x809046"], 0);
        assert_eq!(
            shared.uri(),
            "dnssd://Hall%20%40%20desk._ipp._tcp.local/cups"
        );
        assert_eq!(
            service("Lab v2.1", "_ipps._tcp", &[], 0).uri(),
            "dnssd://Lab%20v2%5C.1._ipps._tcp.local/",
            "a dot in the name is escaped as avahi joins it"
        );
    }

    #[test]
    fn only_a_shared_queue_announced_from_this_computer_is_its_own() {
        let shared = ["printer-type=0x809046"];
        assert!(service("Hall", "_ipp._tcp", &shared, LOCAL).own_queue());
        assert!(!service("Hall", "_ipp._tcp", &shared, 0).own_queue());
        assert!(!service("Hall", "_ipp._tcp", &["UUID=u1"], LOCAL).own_queue());
    }

    #[test]
    fn the_service_a_uri_names_is_read_from_its_host() {
        assert_eq!(
            service_name(
                "ipps://Reprise%20Test%20Printer._ipps._tcp.local/",
                &IPP_TYPES
            )
            .as_deref(),
            Some("reprise test printer")
        );
        assert_eq!(
            service_name(
                "dnssd://Lab%20v2%5C.1._ipp._tcp.local/cups?uuid=1",
                &IPP_TYPES
            )
            .as_deref(),
            Some("lab v2.1")
        );
        assert_eq!(service_name("ipp://10.0.0.7/ipp/print", &IPP_TYPES), None);
        assert_eq!(service_name("usb://HP/LaserJet?serial=1", &IPP_TYPES), None);
    }
}
