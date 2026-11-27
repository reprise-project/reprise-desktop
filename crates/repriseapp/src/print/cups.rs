//! What a program asks cupsd to print: the queues it may print on, the one the user prints on by
//! default, and a document sent as a job. Printing goes out as the account, never through
//! cups-pk-helper.
//!
//! A driverless (IPP Everywhere) printer avahi-daemon finds on the network is offered with the
//! queues while cupsd has no queue for it. Printing on one first asks cupsd to make a temporary
//! queue for it (CUPS-Create-Local-Printer, as libcups' destinations do), which cupsd sets up
//! from the printer's own attributes and drops once it has stood idle.

use super::http::{self, Server};
use super::ipp::{self, JOB, PRINTER, Request, Value};
use reprisebus::avahi::{self, IPP_TYPES, PRINTER_TYPES, Service};
use reprisekit::controls::printdlg::{Dest, DestState};
use reprisekit::tr;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const PRINT_JOB: u16 = 0x0002;
const GET_PRINTER_ATTRIBUTES: u16 = 0x000B;
const CUPS_GET_DEFAULT: u16 = 0x4001;
const CUPS_GET_PRINTERS: u16 = 0x4002;
const CUPS_CREATE_LOCAL_PRINTER: u16 = 0x4028;
const NOT_FOUND: u16 = 0x0406;
/// `printer-state` of a queue that is stopped.
const STOPPED: i32 = 5;
/// How long a temporary queue may take to read its printer and get ready.
const READY_WAIT: Duration = Duration::from_secs(30);
/// How long the queue listing may stall: it decides what the Print dialog shows.
pub const LIST_TIMEOUT: Duration = Duration::from_secs(5);
/// The longest job-name and requesting-user-name cupsd takes, in bytes.
const MAX_NAME: usize = 255;

const QUEUE_ATTRS: [&str; 8] = [
    "printer-name",
    "printer-info",
    "printer-location",
    "printer-state",
    "printer-state-message",
    "printer-is-accepting-jobs",
    "printer-type",
    "device-uri",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    NotRunning,
    Denied,
    /// cupsd refused with an IPP status and its own words.
    Server {
        status: u16,
        why: String,
    },
    /// cupsd refused with this IPP status and no words.
    Status(u16),
    /// An HTTP status other than 2xx.
    Http(u16),
    /// A reply that does not read as HTTP or IPP.
    BadReply,
    /// The connection broke or timed out.
    Broken,
    /// A temporary queue stayed stopped, with cupsd's message, which may be empty.
    NotReady(String),
    /// cupsd answered a temporary queue with this destination, which prints elsewhere.
    Taken(String),
}

impl Error {
    pub fn text(&self) -> String {
        match self {
            Error::NotRunning => tr!("print-not-running"),
            Error::Denied => tr!("print-denied"),
            Error::Server { why, .. } => tr!("print-refused", why = why.as_str()),
            Error::Status(code) => tr!("print-status", status = format!("0x{code:04x}")),
            Error::Http(code) => tr!("print-http-status", status = *code),
            Error::BadReply => tr!("print-bad-reply"),
            Error::Broken => tr!("print-broken"),
            Error::NotReady(m) if m.is_empty() => tr!("print-not-ready"),
            Error::NotReady(m) => tr!("print-not-ready-why", why = m.as_str()),
            Error::Taken(name) => tr!("print-name-taken", name = name.as_str()),
        }
    }
}

impl From<http::Error> for Error {
    fn from(e: http::Error) -> Error {
        log::warn!("print: {e:?}");
        match e {
            http::Error::Unreachable(_) => Error::NotRunning,
            http::Error::Denied => Error::Denied,
            http::Error::Io(_) => Error::Broken,
            http::Error::Status(code) => Error::Http(code),
            http::Error::BadReply(_) => Error::BadReply,
        }
    }
}

/// The queues and driverless printers, sorted by name, and the one printed on unless the user
/// picks another.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Queues {
    pub list: Vec<Dest>,
    pub default: Option<String>,
    /// The printers in `list` with no queue yet, by name.
    pub driverless: BTreeMap<String, Driverless>,
    /// The DNS-SD announcements the queues print through: name (lower-cased) and type.
    served: Vec<(String, String)>,
    /// Every destination's name, classes included, lower-cased.
    taken: Vec<String>,
}

/// A printer on the network with no queue yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Driverless {
    /// The `dnssd://` URI cupsd makes a temporary queue from.
    pub uri: String,
    /// Its DNS-SD service name, the temporary queue's description.
    pub info: String,
}

impl Queues {
    /// `found` added to the list as printers without a queue, none of this computer's own
    /// shared queues. An IPP printer that also announces IPPS is one printer, offered over
    /// IPPS, when both are [`same_printer`]; a printer is left out when a queue prints through
    /// any of its announcements. A name a destination already has is not offered again. When
    /// several printers on the network would get one queue name, all of them are numbered, so
    /// none takes the plain name another one is known by.
    pub fn add_driverless(&mut self, found: &[Service]) {
        let rank = |s: &Service| IPP_TYPES.iter().position(|t| *t == s.kind);
        let mut found: Vec<&Service> = found
            .iter()
            .filter(|s| rank(s).is_some() && !s.own_queue())
            .collect();
        found.sort_by_key(|s| rank(s));
        let mut printers: Vec<Vec<&Service>> = Vec::new();
        for s in found {
            let twin = printers
                .iter_mut()
                .find(|p| p[0].name.eq_ignore_ascii_case(&s.name) && same_printer(p[0], s));
            match twin {
                Some(p) => p.push(s),
                None => printers.push(vec![s]),
            }
        }
        let served = |s: &&Service| {
            let id = (s.name.to_lowercase(), s.kind.clone());
            self.served.contains(&id)
        };
        let mut by_name: BTreeMap<String, Vec<&Service>> = BTreeMap::new();
        for p in printers.iter().filter(|p| !p.iter().any(served)) {
            let base = queue_name(&p[0].name);
            if base.is_empty() {
                log::debug!(
                    "print: no queue name for the network printer {:?}",
                    p[0].name
                );
            } else if !self.taken.contains(&base.to_lowercase()) {
                by_name.entry(base).or_default().push(p[0]);
            }
        }
        for (base, mut group) in by_name {
            group.sort_by_key(|s| (s.host.to_lowercase(), s.kind.clone()));
            let numbered = group.len() > 1;
            for s in group {
                let name = if numbered {
                    let free = |c: &String| !self.taken.contains(&c.to_lowercase());
                    let Some(name) = (1..100).map(|i| format!("{base}_{i}")).find(free) else {
                        continue;
                    };
                    name
                } else {
                    base.clone()
                };
                self.offer(name, s);
            }
        }
        self.list.sort_by_key(|d| d.name.to_lowercase());
    }

    fn offer(&mut self, name: String, s: &Service) {
        let info = match s.host.is_empty() {
            true => s.name.clone(),
            false => tr!(
                "print-network-printer",
                name = s.name.as_str(),
                host = s.host.trim_end_matches('.')
            ),
        };
        self.list.push(Dest {
            name: name.clone(),
            info,
            location: s.get("note").unwrap_or_default().into(),
            state: DestState::Ready,
        });
        self.taken.push(name.to_lowercase());
        let printer = Driverless {
            uri: s.uri(),
            info: s.name.clone(),
        };
        self.driverless.insert(name, printer);
    }

    /// Where the default queue is in `list`, else the first.
    pub fn default_at(&self) -> usize {
        self.default
            .as_deref()
            .and_then(|d| self.list.iter().position(|q| q.name == d))
            .unwrap_or(0)
    }
}

/// Whether two announcements of one service name are one printer: the same host, and the
/// same uuid when both carry one. A uuid alone is no proof, as anyone may announce it.
fn same_printer(a: &Service, b: &Service) -> bool {
    let uuids_agree = match (a.get("uuid"), b.get("uuid")) {
        (Some(x), Some(y)) => x.eq_ignore_ascii_case(y),
        _ => true,
    };
    !a.host.is_empty() && a.host.eq_ignore_ascii_case(&b.host) && uuids_agree
}

/// A document for [`print`]: a PDF, its title in the queue, and what the job asks of the printer.
pub struct Job {
    pub title: String,
    pub pdf: Vec<u8>,
    pub copies: u32,
    /// The paper as a PWG media name (`iso_a4_210x297mm`).
    pub media: String,
}

/// Send `req` with `data` after it, each read or write allowed `timeout`; an IPP error status
/// is an [`Error`].
fn call(
    server: &Server,
    user: &str,
    resource: &str,
    req: Request,
    data: &[u8],
    timeout: Duration,
) -> Result<ipp::Response, Error> {
    let req = req.op_attr("requesting-user-name", Value::name(&ipp_name(user)));
    let head = req.encode_head();
    let reply = http::post_parts(server, resource, &[&head, data], user, timeout)?;
    let resp = ipp::parse(&reply.body).map_err(|e| {
        log::warn!("print: unreadable IPP reply: {}", e.0);
        Error::BadReply
    })?;
    if resp.ok() {
        return Ok(resp);
    }
    Err(match resp.message() {
        Some(m) => Error::Server {
            status: resp.status,
            why: m.into(),
        },
        None => Error::Status(resp.status),
    })
}

/// `s` as cupsd takes a name: control characters as spaces, cut to 255 bytes on a character
/// boundary.
pub fn ipp_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().map(|c| if c.is_control() { ' ' } else { c }) {
        if out.len() + c.len_utf8() > MAX_NAME {
            break;
        }
        out.push(c);
    }
    out
}

/// The printers cupsd has, classes left out, and the default: the user's own lpoptions
/// default when cupsd has that queue, else cupsd's.
pub fn queues(server: &Server, user: &str) -> Result<Queues, Error> {
    let list = Request::new(CUPS_GET_PRINTERS).requested(&QUEUE_ATTRS);
    let resp = match call(server, user, "/", list, &[], LIST_TIMEOUT) {
        Ok(r) => r,
        Err(
            Error::Status(NOT_FOUND)
            | Error::Server {
                status: NOT_FOUND, ..
            },
        ) => return Ok(Queues::default()),
        Err(e) => return Err(e),
    };
    let taken = resp
        .groups(PRINTER)
        .map(|g| g.string("printer-name").to_lowercase())
        .collect();
    let printers: Vec<&ipp::Group> = resp
        .groups(PRINTER)
        .filter(|g| g.int("printer-type").unwrap_or(0) & 1 == 0)
        .collect();
    let served = printers
        .iter()
        .filter_map(|g| avahi::service_of(g.str("device-uri")?, &PRINTER_TYPES))
        .collect();
    let mut list: Vec<Dest> = printers
        .into_iter()
        .map(dest)
        .filter(|d| !d.name.is_empty())
        .collect();
    list.sort_by_key(|d| d.name.to_lowercase());
    let server_default = call(
        server,
        user,
        "/",
        Request::new(CUPS_GET_DEFAULT).requested(&["printer-name"]),
        &[],
        LIST_TIMEOUT,
    )
    .ok()
    .and_then(|r| r.first(PRINTER).map(|g| g.string("printer-name")))
    .filter(|n| !n.is_empty());
    let default = own_default()
        .filter(|n| list.iter().any(|d| &d.name == n))
        .or(server_default);
    Ok(Queues {
        list,
        default,
        driverless: BTreeMap::new(),
        served,
        taken,
    })
}

/// A queue name for the DNS-SD service `name`, as libcups names its temporary queues: ASCII
/// letters and digits kept, each run of anything else one `_`, and a `_` at the end dropped
/// unless it is the whole name.
pub fn queue_name(name: &str) -> String {
    let mut out = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() {
            out.push(b as char);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    if out.len() > 1 && out.ends_with('_') {
        out.pop();
    }
    out
}

/// Have cupsd make a temporary queue named about `name` for `printer` and wait up to
/// [`READY_WAIT`] while cupsd reads it; the queue's name. cupsd hands back an existing
/// destination for a name or printer it already has, which is printed on only when it prints
/// to `printer`.
pub fn create_local(
    server: &Server,
    user: &str,
    name: &str,
    printer: &Driverless,
) -> Result<String, Error> {
    let req = Request::new(CUPS_CREATE_LOCAL_PRINTER)
        .op_attr("printer-uri", Value::uri("ipp://localhost/"))
        .printer_attr("printer-name", Value::name(&ipp_name(name)))
        .printer_attr("device-uri", Value::uri(&printer.uri))
        .printer_attr("printer-info", Value::text(&ipp_name(&printer.info)));
    let r = call(server, user, "/", req, &[], LIST_TIMEOUT)?;
    let queue = r
        .first(PRINTER)
        .and_then(|g| g.str("printer-uri-supported"))
        .and_then(queue_of)
        .ok_or(Error::BadReply)?;
    let until = Instant::now() + READY_WAIT;
    loop {
        let req = Request::new(GET_PRINTER_ATTRIBUTES)
            .op_attr("printer-uri", Value::uri(&printer_uri(&queue)))
            .requested(&["printer-state", "printer-state-message", "device-uri"]);
        let r = call(server, user, "/", req, &[], LIST_TIMEOUT)?;
        let g = r.first(PRINTER).ok_or(Error::BadReply)?;
        if made_ready(g, &queue, &printer.uri)? {
            return Ok(queue);
        }
        if Instant::now() >= until {
            return Err(Error::NotReady(g.string("printer-state-message")));
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Whether the queue `queue` cupsd made, whose attributes are `g`, is ready: refused when it
/// prints to another printer than `device_uri`, false while it is stopped. One that does not
/// say where it prints is taken at its word.
fn made_ready(g: &ipp::Group, queue: &str, device_uri: &str) -> Result<bool, Error> {
    match g.str("device-uri") {
        None => log::info!("print: cupsd does not say where {queue} prints"),
        Some(uri) if !same_device(uri, device_uri) => {
            log::warn!("print: {queue} prints to {uri}, not {device_uri}");
            return Err(Error::Taken(queue.into()));
        }
        Some(_) => {}
    }
    Ok(g.int("printer-state") != Some(STOPPED))
}

/// Whether two device URIs reach one DNS-SD announcement: the same service name and type
/// whatever the case or percent-encoding, and the same uuid when both carry one.
fn same_device(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let uuid = |u: &str| {
        let query = u.split_once('?')?.1;
        query
            .split('&')
            .find_map(|kv| kv.strip_prefix("uuid="))
            .map(str::to_lowercase)
    };
    let uuids_agree = match (uuid(a), uuid(b)) {
        (Some(x), Some(y)) => x == y,
        _ => true,
    };
    let service = |u| avahi::service_of(u, &IPP_TYPES);
    service(a).is_some() && service(a) == service(b) && uuids_agree
}

/// The queue a cupsd printer URI (`ipp://localhost/printers/<name>`) names, unescaped.
fn queue_of(uri: &str) -> Option<String> {
    let (_, escaped) = uri.rsplit_once("/printers/")?;
    let mut out = Vec::new();
    let mut bytes = escaped.bytes();
    while let Some(b) = bytes.next() {
        if b != b'%' {
            out.push(b);
            continue;
        }
        let hex = [bytes.next()?, bytes.next()?];
        out.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
    }
    let name = String::from_utf8(out).ok()?;
    (!name.is_empty() && !name.contains('/')).then_some(name)
}

fn dest(g: &ipp::Group) -> Dest {
    let message = g.string("printer-state-message");
    Dest {
        name: g.string("printer-name"),
        info: g.string("printer-info"),
        location: g.string("printer-location"),
        state: match g.int("printer-state") {
            _ if !g.bool("printer-is-accepting-jobs").unwrap_or(true) => DestState::Refusing,
            Some(4) => DestState::Printing,
            Some(5) => DestState::Stopped(message),
            _ => DestState::Ready,
        },
    }
}

/// Send `job` to queue `queue`; the job id cupsd gave it.
pub fn print(server: &Server, user: &str, queue: &str, job: Job) -> Result<i32, Error> {
    let req = Request::new(PRINT_JOB)
        .op_attr("printer-uri", Value::uri(&printer_uri(queue)))
        .op_attr("job-name", Value::name(&ipp_name(&job.title)))
        .op_attr("document-format", Value::mime("application/pdf"))
        .add(
            JOB,
            "copies",
            vec![Value::Int(job.copies.clamp(1, i32::MAX as u32) as i32)],
        )
        .add(JOB, "media", vec![Value::keyword(&job.media)]);
    let resource = format!("/printers/{}", escape(queue));
    let r = call(server, user, &resource, req, &job.pdf, http::TIMEOUT)?;
    Ok(r.first(JOB).and_then(|g| g.int("job-id")).unwrap_or(0))
}

pub fn printer_uri(name: &str) -> String {
    format!("ipp://localhost/printers/{}", escape(name))
}

/// `s` percent-encoded for a URI path segment.
pub fn escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The `Default` line of an lpoptions file: the user's own default,
/// which programs take over the server's.
pub fn user_default(lpoptions: &str) -> Option<String> {
    lpoptions.lines().find_map(|l| {
        let rest = l.trim().strip_prefix("Default ")?;
        let dest = rest.split_whitespace().next()?;
        Some(dest.split('/').next()?.to_string())
    })
}

/// The folder of the system's lpoptions, which the user's own overrides.
pub const SYSTEM_LPOPTIONS: &str = "/etc/cups";

pub fn lpoptions_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".cups/lpoptions"))
}

pub fn own_default() -> Option<String> {
    own_default_in(&lpoptions_text(), &system_lpoptions_text())
}

pub fn system_lpoptions_text() -> String {
    std::fs::read_to_string(std::path::Path::new(SYSTEM_LPOPTIONS).join("lpoptions"))
        .unwrap_or_default()
}

/// The default an lpoptions file names, the user's before the system's.
pub fn own_default_in(user: &str, system: &str) -> Option<String> {
    user_default(user).or_else(|| user_default(system))
}

pub fn lpoptions_text() -> String {
    lpoptions_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_loses_its_control_characters_and_stops_at_255_bytes_on_a_character() {
        assert_eq!(ipp_name("a\tb\nc.txt"), "a b c.txt");
        let long = "\u{e9}".repeat(200);
        let cut = ipp_name(&long);
        assert_eq!(cut.len(), 254, "127 two-byte characters");
        assert!(cut.chars().all(|c| c == '\u{e9}'));
    }

    fn found(name: &str, kind: &str, host: &str, txt: &[(&str, &str)]) -> Service {
        Service {
            name: name.into(),
            kind: kind.into(),
            domain: "local".into(),
            host: host.into(),
            txt: txt
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            flags: 0,
        }
    }

    fn queue(name: &str) -> Dest {
        Dest {
            name: name.into(),
            ..Dest::default()
        }
    }

    fn names(q: &Queues) -> Vec<&str> {
        q.list.iter().map(|d| d.name.as_str()).collect()
    }

    #[test]
    fn a_service_name_becomes_a_queue_name_as_libcups_makes_it() {
        assert_eq!(
            queue_name("HP OfficeJet 5200 [A1B2]"),
            "HP_OfficeJet_5200_A1B2"
        );
        assert_eq!(queue_name("H\u{e9}-llo.v2"), "H_llo_v2");
        assert_eq!(queue_name("a__b x"), "a_b_x");
        assert_eq!(queue_name(" Lead"), "_Lead");
        assert_eq!(queue_name("Caf\u{e9}"), "Caf");
        assert_eq!(queue_name("\u{30d7}\u{30ea}\u{30f3}\u{30bf}\u{30fc}"), "_");
        assert_eq!(queue_name(""), "");
    }

    #[test]
    fn driverless_printers_join_the_queues_once_each_and_never_twice_with_a_queue() {
        let mut q = Queues {
            list: vec![queue("PDF"), queue("Hall")],
            served: vec![("attic laser".into(), "_ipps._tcp".into())],
            taken: vec!["pdf".into(), "hall".into(), "lab".into()],
            ..Queues::default()
        };
        let jet = "jet.local";
        q.add_driverless(&[
            found(
                "Office Jet",
                "_ipp._tcp",
                jet,
                &[("note", "Room 4"), ("uuid", "U1")],
            ),
            found(
                "Office Jet",
                "_ipps._tcp",
                jet,
                &[("uuid", "u1"), ("note", "Room 4")],
            ),
            found("Attic Laser", "_ipps._tcp", "attic.local", &[]),
            found("hall", "_ipp._tcp", "hall.local", &[]),
            found("Lab", "_ipp._tcp", "lab.local", &[]),
            Service {
                flags: avahi::LOCAL,
                ..found(
                    "Desk",
                    "_ipp._tcp",
                    "me.local",
                    &[("printer-type", "0x809046")],
                )
            },
            found("Plotter", "_pdl-datastream._tcp", "plot.local", &[]),
        ]);
        assert_eq!(
            names(&q),
            ["Hall", "Office_Jet", "PDF"],
            "Lab is the name of a class"
        );
        let jet = &q.list[1];
        assert_eq!(
            (jet.info.as_str(), jet.location.as_str()),
            ("Office Jet on jet.local", "Room 4")
        );
        let made = &q.driverless["Office_Jet"];
        assert_eq!(
            made.uri, "dnssd://Office%20Jet._ipps._tcp.local/?uuid=u1",
            "over IPPS"
        );
        assert_eq!(made.info, "Office Jet");
        assert_eq!(q.driverless.len(), 1);
    }

    #[test]
    fn an_ipps_announcement_from_another_host_or_uuid_never_stands_in_for_an_ipp_printer() {
        let mut q = Queues::default();
        q.add_driverless(&[
            found("Office Jet", "_ipp._tcp", "jet.local", &[("uuid", "u1")]),
            found("Office Jet", "_ipps._tcp", "other.local", &[("uuid", "u1")]),
            found("Lab", "_ipp._tcp", "lab.local", &[("uuid", "u2")]),
            found("Lab", "_ipps._tcp", "lab.local", &[("uuid", "u3")]),
        ]);
        assert_eq!(
            names(&q),
            ["Lab_1", "Lab_2", "Office_Jet_1", "Office_Jet_2"],
            "printers sharing a name are all numbered, none takes the plain one"
        );
        let uri = |n: &str| q.driverless[n].uri.as_str();
        assert!(
            uri("Office_Jet_1").contains("._ipp._tcp"),
            "the printer announced over IPP alone stays listed, numbered by its host"
        );
        assert!(uri("Office_Jet_2").contains("._ipps._tcp"));
        assert!(
            uri("Lab_2").contains("._ipps._tcp"),
            "a uuid of its own is another printer"
        );
    }

    #[test]
    fn a_queue_hides_only_the_printer_whose_announcement_it_prints_through() {
        let printers = [
            found("Office Jet", "_ipp._tcp", "jet.local", &[]),
            found("Office Jet", "_ipps._tcp", "other.local", &[]),
            found("Lab", "_ipp._tcp", "lab.local", &[]),
            found("Lab", "_ipps._tcp", "lab.local", &[]),
        ];
        let mut q = Queues {
            served: vec![
                ("office jet".into(), "_ipps._tcp".into()),
                ("lab".into(), "_ipp._tcp".into()),
            ],
            ..Queues::default()
        };
        q.add_driverless(&printers);
        assert_eq!(
            names(&q),
            ["Office_Jet"],
            "a queue on the other host's announcement leaves the IPP printer listed, and one on \
             Lab's IPP announcement hides its IPPS twin too"
        );
        assert!(q.driverless["Office_Jet"].uri.contains("._ipp._tcp"));
    }

    #[test]
    fn a_made_queue_is_printed_on_only_when_it_prints_to_the_printer_asked_for() {
        let group = |uri: &str, state: i32| ipp::Group {
            tag: PRINTER,
            attrs: vec![
                ipp::Attr {
                    name: "device-uri".into(),
                    values: vec![Value::uri(uri)],
                },
                ipp::Attr {
                    name: "printer-state".into(),
                    values: vec![Value::Enum(state)],
                },
            ],
        };
        let jet = "dnssd://Office%20Jet._ipps._tcp.local/?uuid=u1";
        assert_eq!(made_ready(&group(jet, 3), "Jet", jet), Ok(true));
        assert_eq!(made_ready(&group(jet, STOPPED), "Jet", jet), Ok(false));
        let stored = "dnssd://office%20jet._IPPS._tcp.local./?uuid=U1";
        assert_eq!(
            made_ready(&group(stored, 3), "Jet", jet),
            Ok(true),
            "case, encoding and a trailing dot name the same printer"
        );
        let silent = ipp::Group {
            tag: PRINTER,
            attrs: Vec::new(),
        };
        assert_eq!(
            made_ready(&silent, "Jet", jet),
            Ok(true),
            "a queue that does not say where it prints is taken at its word"
        );
        for elsewhere in [
            "usb://HP/Laser",
            "dnssd://Office%20Jet._ipp._tcp.local/?uuid=u1",
            "dnssd://Office%20Jet._ipps._tcp.local/?uuid=u2",
        ] {
            assert_eq!(
                made_ready(&group(elsewhere, 3), "Jet", jet),
                Err(Error::Taken("Jet".into())),
                "{elsewhere}"
            );
        }
    }

    #[test]
    fn a_made_queue_is_named_by_its_printer_uri() {
        assert_eq!(
            queue_of("ipp://localhost/printers/Office_Jet").as_deref(),
            Some("Office_Jet")
        );
        assert_eq!(
            queue_of("ipp://localhost/printers/a%20b").as_deref(),
            Some("a b")
        );
        for bad in [
            "ipp://localhost/",
            "ipp://localhost/printers/",
            "ipp://h/printers/a%2Fb",
        ] {
            assert_eq!(queue_of(bad), None, "{bad}");
        }
    }
}
