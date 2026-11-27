//! What a program asks cupsd to print: the queues it may print on, the one the user prints on by
//! default, and a document sent as a job. Printing goes out as the account, never through
//! cups-pk-helper.

use super::http::{self, Server};
use super::ipp::{self, JOB, PRINTER, Request, Value};
use reprisekit::controls::printdlg::{Dest, DestState};
use reprisekit::tr;
use std::time::Duration;

const PRINT_JOB: u16 = 0x0002;
const CUPS_GET_DEFAULT: u16 = 0x4001;
const CUPS_GET_PRINTERS: u16 = 0x4002;
const NOT_FOUND: u16 = 0x0406;
/// How long the queue listing may stall: it decides what the Print dialog shows.
pub const LIST_TIMEOUT: Duration = Duration::from_secs(5);
/// The longest job-name and requesting-user-name cupsd takes, in bytes.
const MAX_NAME: usize = 255;

const QUEUE_ATTRS: [&str; 7] = [
    "printer-name",
    "printer-info",
    "printer-location",
    "printer-state",
    "printer-state-message",
    "printer-is-accepting-jobs",
    "printer-type",
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

/// The queues, sorted by name, and the one printed on unless the user picks another.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Queues {
    pub list: Vec<Dest>,
    pub default: Option<String>,
}

impl Queues {
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
    let mut list: Vec<Dest> = resp
        .groups(PRINTER)
        .filter(|g| g.int("printer-type").unwrap_or(0) & 1 == 0)
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
    Ok(Queues { list, default })
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
}
