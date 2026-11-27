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
    /// The printers in `list` with no queue yet, by name: the `dnssd://` URI cupsd makes a
    /// temporary queue from.
    pub driverless: BTreeMap<String, String>,
    /// The DNS-SD services the queues print through, lower-cased.
    served: Vec<String>,
}

impl Queues {
    /// `found` added to the list as printers without a queue: each IPP printer once, over IPPS
    /// when it announces both, and none that a queue already prints to or names, nor this
    /// computer's own shared queues.
    pub fn add_driverless(&mut self, found: &[Service]) {
        let rank = |s: &Service| IPP_TYPES.iter().position(|t| *t == s.kind);
        let mut found: Vec<&Service> = found
            .iter()
            .filter(|s| rank(s).is_some() && !s.own_queue())
            .collect();
        found.sort_by_key(|s| rank(s));
        for s in found {
            let name = queue_name(&s.name);
            let service = s.name.to_lowercase();
            let named = self.list.iter().any(|d| d.name.eq_ignore_ascii_case(&name));
            if name.is_empty() || named || self.served.contains(&service) {
                continue;
            }
            self.served.push(service);
            self.list.push(Dest {
                name: name.clone(),
                info: s.name.clone(),
                location: s.get("note").unwrap_or_default().into(),
                state: DestState::Ready,
            });
            self.driverless.insert(name, s.uri());
        }
        self.list.sort_by_key(|d| d.name.to_lowercase());
    }

    /// Where the default queue is in `list`, else the first.
    pub fn default_at(&self) -> usize {
        self.default
            .as_deref()
            .and_then(|d| self.list.iter().position(|q| q.name == d))
            .unwrap_or(0)
    }
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
    let printers: Vec<&ipp::Group> = resp
        .groups(PRINTER)
        .filter(|g| g.int("printer-type").unwrap_or(0) & 1 == 0)
        .collect();
    let served = printers
        .iter()
        .filter_map(|g| avahi::service_name(g.str("device-uri")?, &PRINTER_TYPES))
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
    })
}

/// A queue name for the DNS-SD service `name`, as libcups names its temporary queues: letters,
/// digits, `-` and `_` kept, each run of anything else one `_`, none at the end.
pub fn queue_name(name: &str) -> String {
    let mut out = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
            out.push(b as char);
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

/// Have cupsd make a temporary queue named about `name` for the driverless printer at
/// `device_uri`, described as `info`, and wait up to [`READY_WAIT`] while cupsd reads the
/// printer; the queue's name. cupsd hands back its existing queue for a printer it has one for.
pub fn create_local(
    server: &Server,
    user: &str,
    name: &str,
    device_uri: &str,
    info: &str,
) -> Result<String, Error> {
    let req = Request::new(CUPS_CREATE_LOCAL_PRINTER)
        .op_attr("printer-uri", Value::uri("ipp://localhost/"))
        .printer_attr("printer-name", Value::name(&ipp_name(name)))
        .printer_attr("device-uri", Value::uri(device_uri))
        .printer_attr("printer-info", Value::text(&ipp_name(info)));
    let r = call(server, user, "/", req, &[], LIST_TIMEOUT)?;
    let made = r.first(PRINTER).ok_or(Error::BadReply)?;
    let queue = made
        .str("printer-uri-supported")
        .and_then(queue_of)
        .ok_or(Error::BadReply)?;
    let until = Instant::now() + READY_WAIT;
    let mut state = (
        made.int("printer-state"),
        made.string("printer-state-message"),
    );
    while state.0 == Some(STOPPED) {
        if Instant::now() >= until {
            return Err(Error::NotReady(state.1));
        }
        std::thread::sleep(Duration::from_secs(1));
        let req = Request::new(GET_PRINTER_ATTRIBUTES)
            .op_attr("printer-uri", Value::uri(&printer_uri(&queue)))
            .requested(&["printer-state", "printer-state-message"]);
        let r = call(server, user, "/", req, &[], LIST_TIMEOUT)?;
        let g = r.first(PRINTER).ok_or(Error::BadReply)?;
        state = (g.int("printer-state"), g.string("printer-state-message"));
    }
    Ok(queue)
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

    fn found(name: &str, kind: &str, txt: &[(&str, &str)], flags: u32) -> Service {
        Service {
            name: name.into(),
            kind: kind.into(),
            domain: "local".into(),
            txt: txt
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            flags,
        }
    }

    fn queue(name: &str) -> Dest {
        Dest {
            name: name.into(),
            ..Dest::default()
        }
    }

    #[test]
    fn a_service_name_becomes_a_queue_name_as_libcups_makes_it() {
        assert_eq!(
            queue_name("HP OfficeJet 5200 [A1B2]"),
            "HP_OfficeJet_5200_A1B2"
        );
        assert_eq!(queue_name("Lab-2 (2nd floor)"), "Lab-2_2nd_floor");
        assert_eq!(queue_name("  Caf\u{e9}  "), "Caf");
        assert_eq!(queue_name("()"), "");
    }

    #[test]
    fn driverless_printers_join_the_queues_once_each_and_never_twice_with_a_queue() {
        let mut q = Queues {
            list: vec![queue("PDF"), queue("Hall")],
            served: vec!["attic laser".into()],
            ..Queues::default()
        };
        q.add_driverless(&[
            found("Office Jet", "_ipp._tcp", &[("note", "Room 4")], 0),
            found(
                "Office Jet",
                "_ipps._tcp",
                &[("uuid", "u1"), ("note", "Room 4")],
                0,
            ),
            found("Attic Laser", "_ipps._tcp", &[], 0),
            found("hall", "_ipp._tcp", &[], 0),
            found(
                "Desk",
                "_ipp._tcp",
                &[("printer-type", "0x809046")],
                avahi::LOCAL,
            ),
            found("Plotter", "_pdl-datastream._tcp", &[], 0),
        ]);
        let names: Vec<&str> = q.list.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Hall", "Office_Jet", "PDF"]);
        let jet = &q.list[1];
        assert_eq!(
            (jet.info.as_str(), jet.location.as_str()),
            ("Office Jet", "Room 4")
        );
        assert_eq!(
            q.driverless.get("Office_Jet").map(String::as_str),
            Some("dnssd://Office%20Jet._ipps._tcp.local/?uuid=u1"),
            "over IPPS when it announces both"
        );
        assert_eq!(q.driverless.len(), 1);
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
