//! Printers announced over DNS-SD, as avahi-daemon browses and resolves them: the services, their
//! TXT keys, and the `dnssd://` URI cupsd prints to one through.

use std::collections::{BTreeMap, HashSet};
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use futures_lite::FutureExt;
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
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
    /// The host it runs on, as avahi resolved it.
    pub host: String,
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

/// The unique name avahi-daemon has on `conn`'s bus. Its browsers' signals come from it alone:
/// any peer may send a signal to ours.
pub fn owner(conn: &Connection) -> Option<String> {
    crate::wait(crate::name_owner(conn, DEST))
}

/// Whether `msg` came from avahi-daemon, `owner` as [`owner`] gives it.
pub fn from_avahi(msg: &Message, owner: Option<&str>) -> bool {
    crate::sent_by(msg.header().sender().map(|s| s.as_str()), owner)
}

/// Ask avahi-daemon for a browser per type in `types`: the browsers' signals, and their paths.
/// An error means no avahi-daemon (or no system bus).
pub fn start(
    conn: &Connection,
    types: &[&str],
) -> zbus::Result<(MessageStream, Vec<OwnedObjectPath>)> {
    crate::wait(start_on(conn, types))
}

async fn start_on(
    conn: &Connection,
    types: &[&str],
) -> zbus::Result<(MessageStream, Vec<OwnedObjectPath>)> {
    // Subscribed before the browsers exist: their first signals can beat the method reply.
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(BROWSER)
        .build()?;
    let signals = MessageStream::for_match_rule(rule, conn, NonZeroUsize::new(1024)).await?;
    let server = Proxy::new(conn, DEST, "/", SERVER).await?;
    let mut paths = Vec::new();
    for kind in types {
        paths.push(
            server
                .call("ServiceBrowserNew", &(UNSPEC, UNSPEC, *kind, "", 0u32))
                .await?,
        );
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
        Ok((_, _, _, _, _, host, _, _, _, txt, flags)) => Some(Service {
            name: name.clone(),
            kind: kind.clone(),
            domain: domain.clone(),
            host,
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
/// The most services one [`browse`] resolves.
const MOST: usize = 64;

/// The services of `types` avahi-daemon finds and resolves within `budget`, each resolved once
/// however many interfaces and protocols announce it, at most [`MOST`]. Browsing stops at each
/// browser's first answer or a quarter of the budget before its end; resolving at its end, with
/// what resolved by then. Empty without avahi-daemon.
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
    let found = crate::wait(browse_on(&conn, types, deadline));
    let _ = crate::wait(conn.close());
    found
}

/// `fut`'s output, or None once `deadline` has passed.
async fn by<T>(deadline: Instant, fut: impl Future<Output = T>) -> Option<T> {
    async { Some(fut.await) }
        .or(async {
            async_io::Timer::at(deadline).await;
            None
        })
        .await
}

async fn browse_on(conn: &Connection, types: &[&str], deadline: Instant) -> Vec<Service> {
    let (mut signals, paths) = match by(deadline, start_on(conn, types)).await {
        Some(Ok(s)) => s,
        Some(Err(e)) => {
            log::info!("no network printers from avahi-daemon: {e}");
            return Vec::new();
        }
        None => {
            log::info!("avahi-daemon did not start browsing in time");
            return Vec::new();
        }
    };
    let owner = by(deadline, crate::name_owner(conn, DEST)).await.flatten();
    let ours = |m: &Message| {
        from_avahi(m, owner.as_deref())
            && m.header()
                .path()
                .is_some_and(|p| paths.iter().any(|q| q.as_str() == p.as_str()))
    };
    let now = Instant::now();
    let browsing = now + deadline.saturating_duration_since(now) * 3 / 4;
    let mut seen = Seen::default();
    let mut done: HashSet<String> = HashSet::new();
    while done.len() < paths.len() {
        let Some(Some(Ok(msg))) = by(browsing, signals.next()).await else {
            break;
        };
        if !ours(&msg) {
            continue;
        }
        match member(&msg).as_deref() {
            Some("ItemNew") => {
                if let Some((key, _)) = item(&msg) {
                    seen.add(key);
                }
            }
            Some("ItemRemove") => {
                if let Some((key, _)) = item(&msg) {
                    seen.remove(&key);
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
    let Some(Ok(server)) = by(deadline, Proxy::new(conn, DEST, "/", SERVER)).await else {
        return Vec::new();
    };
    let mut pending: FuturesUnordered<_> = seen.keys().map(|k| resolve(&server, k)).collect();
    let mut found = Vec::new();
    while let Some(Some(r)) = by(deadline, pending.next()).await {
        found.extend(r);
    }
    if !pending.is_empty() {
        log::debug!("{} network printers did not resolve in time", pending.len());
    }
    found
}

/// The services announced so far, by name, type and domain, each with the interfaces and
/// protocols that announce it; at most [`MOST`].
#[derive(Debug, Default)]
struct Seen(BTreeMap<(String, String, String), Vec<Key>>);

impl Seen {
    fn add(&mut self, key: Key) {
        let id = (key.2.clone(), key.3.clone(), key.4.clone());
        if !self.0.contains_key(&id) && self.0.len() >= MOST {
            return;
        }
        let keys = self.0.entry(id).or_default();
        if !keys.contains(&key) {
            keys.push(key);
        }
    }

    /// Gone from one interface or protocol; the service goes with the last.
    fn remove(&mut self, key: &Key) {
        let id = (key.2.clone(), key.3.clone(), key.4.clone());
        if let Some(keys) = self.0.get_mut(&id) {
            keys.retain(|k| k != key);
            if keys.is_empty() {
                self.0.remove(&id);
            }
        }
    }

    /// A key to resolve each service by.
    fn keys(&self) -> impl Iterator<Item = &Key> {
        self.0.values().filter_map(|k| k.first())
    }
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
            host: "printer.local".into(),
            txt: txt_pairs(&txt),
            flags,
        }
    }

    fn key(iface: i32, proto: i32, name: &str) -> Key {
        (
            iface,
            proto,
            name.into(),
            "_ipp._tcp".into(),
            "local".into(),
        )
    }

    /// avahi-daemon's Server: browsers that announce "Fast" and "Slow" after 200 ms, a spoofer
    /// that tells the client its browsers are done before that, and a resolve of "Slow" that
    /// never answers.
    struct FakeAvahi {
        spoofer: Connection,
        made: std::sync::atomic::AtomicUsize,
    }

    #[zbus::interface(name = "org.freedesktop.Avahi.Server")]
    impl FakeAvahi {
        #[allow(clippy::too_many_arguments)]
        fn service_browser_new(
            &self,
            _iface: i32,
            _proto: i32,
            kind: String,
            _domain: String,
            _flags: u32,
            #[zbus(header)] hdr: zbus::message::Header<'_>,
            #[zbus(connection)] conn: &Connection,
        ) -> OwnedObjectPath {
            let n = self.made.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = OwnedObjectPath::try_from(format!("/browser/{n}")).unwrap();
            let dest = hdr.sender().unwrap().to_string();
            let (conn, spoofer, at) = (conn.clone(), self.spoofer.clone(), path.clone());
            std::thread::spawn(move || {
                zbus::block_on(async move {
                    let send = async |c: &Connection, member: &str| {
                        c.emit_signal(Some(dest.as_str()), &at, BROWSER, member, &())
                            .await
                            .unwrap()
                    };
                    send(&spoofer, "AllForNow").await;
                    async_io::Timer::after(Duration::from_millis(200)).await;
                    if kind == "_ipp._tcp" {
                        for name in ["Fast", "Slow"] {
                            let body = (1i32, 0i32, name, kind.as_str(), "local", 0u32);
                            conn.emit_signal(Some(dest.as_str()), &at, BROWSER, "ItemNew", &body)
                                .await
                                .unwrap();
                        }
                    }
                    send(&conn, "AllForNow").await;
                })
            });
            path
        }

        #[allow(clippy::too_many_arguments)]
        async fn resolve_service(
            &self,
            iface: i32,
            proto: i32,
            name: String,
            kind: String,
            domain: String,
            _aproto: i32,
            _flags: u32,
        ) -> Resolved {
            if name == "Slow" {
                std::future::pending::<()>().await;
            }
            let txt = vec![b"rp=ipp/print".to_vec()];
            let addr = "10.0.0.9".to_string();
            (
                iface,
                proto,
                name,
                kind,
                domain,
                "fast.local".into(),
                0,
                addr,
                631,
                txt,
                0,
            )
        }
    }

    #[test]
    fn browsing_keeps_to_its_deadline_and_to_avahi_daemons_own_signals() {
        let Some(bus) = crate::testbus::bus() else {
            return;
        };
        let (found, took) = zbus::block_on(async {
            let connect = async || crate::testbus::connect(&bus.1).await;
            let avahi = connect().await;
            let fake = FakeAvahi {
                spoofer: connect().await,
                made: Default::default(),
            };
            crate::testbus::serve(&avahi, "/", fake).await;
            avahi.request_name(DEST).await.unwrap();
            let me = connect().await;
            let start = Instant::now();
            let found = browse_on(&me, &IPP_TYPES, start + Duration::from_millis(1500)).await;
            (found, start.elapsed())
        });
        let names: Vec<&str> = found.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            ["Fast"],
            "the spoofed AllForNow is ignored and the resolve that hangs is left out"
        );
        assert_eq!(found[0].host, "fast.local");
        assert!(
            took < Duration::from_millis(1900),
            "a resolve that never answers ends with the budget: {took:?}"
        );
    }

    #[test]
    fn a_service_stays_while_any_interface_or_protocol_announces_it() {
        let mut seen = Seen::default();
        seen.add(key(2, 0, "Lab"));
        seen.add(key(2, 1, "Lab"));
        seen.add(key(2, 1, "Lab"));
        seen.add(key(3, 0, "Attic"));
        assert_eq!(seen.keys().count(), 2, "one key per service");
        seen.remove(&key(2, 0, "Lab"));
        assert_eq!(seen.keys().count(), 2, "Lab is still announced over IPv6");
        seen.remove(&key(2, 1, "Lab"));
        let left: Vec<&str> = seen.keys().map(|k| k.2.as_str()).collect();
        assert_eq!(left, ["Attic"]);
    }

    #[test]
    fn at_most_64_services_are_kept() {
        let mut seen = Seen::default();
        for i in 0..100 {
            seen.add(key(2, 0, &format!("P{i}")));
        }
        assert_eq!(seen.keys().count(), MOST);
        seen.add(key(3, 0, "P0"));
        assert_eq!(
            seen.0
                .get(&("P0".into(), "_ipp._tcp".into(), "local".into()))
                .map(Vec::len),
            Some(2),
            "a kept one still gains interfaces"
        );
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
