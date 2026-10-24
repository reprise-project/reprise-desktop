//! What reprise-printers asks cupsd, as plain functions over a [`Client`], and the model types the windows read.
//! An admin operation goes over the peer credentials first; if cupsd says no, it goes to cups-pk-helper
//! (pkhelper.rs), else [`Error::NeedsHelper`].

use std::sync::Arc;

use crate::avahi;
use crate::http::{self, Server};
use crate::ipp::{self, JOB, OPERATION, PRINTER, Request, Response, Value};
use crate::pkhelper::{self, Call, Helper};
use crate::ppd;

const PRINT_JOB: u16 = 0x0002;
const CANCEL_JOB: u16 = 0x0008;
const GET_JOB_ATTRIBUTES: u16 = 0x0009;
const GET_JOBS: u16 = 0x000A;
const GET_PRINTER_ATTRIBUTES: u16 = 0x000B;
const HOLD_JOB: u16 = 0x000C;
const RELEASE_JOB: u16 = 0x000D;
const RESTART_JOB: u16 = 0x000E;
const PAUSE_PRINTER: u16 = 0x0010;
const RESUME_PRINTER: u16 = 0x0011;
const CANCEL_JOBS: u16 = 0x0038;
const CANCEL_MY_JOBS: u16 = 0x0039;
const CUPS_GET_DEFAULT: u16 = 0x4001;
const CUPS_GET_PRINTERS: u16 = 0x4002;
const CUPS_ADD_MODIFY_PRINTER: u16 = 0x4003;
const CUPS_DELETE_PRINTER: u16 = 0x4004;
const CUPS_SET_DEFAULT: u16 = 0x400A;
const CUPS_GET_DEVICES: u16 = 0x400B;
const CUPS_GET_PPDS: u16 = 0x400C;
const CUPS_MOVE_JOB: u16 = 0x400D;

const NOT_AUTHENTICATED: u16 = 0x0402;
const NOT_AUTHORIZED: u16 = 0x0403;
const FORBIDDEN: u16 = 0x0401;

mod devices;
mod lpoptions;
mod model;
mod ops;
mod options;
mod setup;

pub use devices::*;
pub use lpoptions::*;
pub use model::*;
pub use ops::*;
pub use options::*;
pub use setup::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    NotRunning,
    Denied,
    /// cupsd said no to an admin operation and cups-pk-helper, which
    /// could ask for a password instead, is not installed.
    NeedsHelper,
    /// cups-pk-helper was asked and polkit refused the password.
    NotAuthorized,
    /// cups-pk-helper was asked and the password prompt was dismissed:
    /// nothing to tell.
    Dismissed,
    Refused(String),
}

impl Error {
    pub fn text(&self) -> String {
        match self {
            Error::NotRunning => "The printing service (CUPS) is not running.".into(),
            Error::Denied => "You are not allowed to change printers. CUPS lets the members of \
                its admin group do this: SystemGroup in /etc/cups/cups-files.conf, wheel on \
                most systems."
                .into(),
            Error::NeedsHelper => format!(
                "{} Install cups-pk-helper to make changes with an administrator's password.",
                Error::Denied.text()
            ),
            Error::NotAuthorized => {
                "Nothing was changed: this needs an administrator's password.".into()
            }
            Error::Refused(why) => why.clone(),
            Error::Dismissed => String::new(),
        }
    }
}

impl From<http::Error> for Error {
    fn from(e: http::Error) -> Error {
        match e {
            http::Error::Unreachable(_) => Error::NotRunning,
            http::Error::Denied => Error::Denied,
            http::Error::Failed(why) => Error::Refused(why),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// cupsd's address, the login name requests go out under, and the way
/// to cups-pk-helper.
#[derive(Clone, Debug)]
pub struct Client {
    pub server: Server,
    pub user: String,
    pub helper: Arc<dyn Helper>,
}

impl Client {
    pub fn new() -> Client {
        Client {
            server: Server::find(),
            user: login_name(),
            helper: Arc::new(pkhelper::Bus::default()),
        }
    }

    /// An admin operation: `peer` is how it went over the peer
    /// credentials. When cupsd said no, `calls` go to cups-pk-helper if
    /// it is installed.
    fn admin(&self, peer: Result<()>, calls: impl FnOnce() -> Vec<Call>) -> Result<()> {
        match peer {
            Err(Error::Denied) if self.helper.installed() => {
                pkhelper::run(self.helper.as_ref(), &calls())
            }
            Err(Error::Denied) => Err(Error::NeedsHelper),
            r => r,
        }
    }

    /// Send `req` to `resource` and read the answer; an IPP error status
    /// is an [`Error`].
    pub(crate) fn call(&self, resource: &str, req: Request) -> Result<Response> {
        let req = req.op_attr("requesting-user-name", Value::name(&self.user));
        let reply = http::post(&self.server, resource, &req.encode(), &self.user)?;
        let resp = ipp::parse(&reply.body).map_err(|e| {
            Error::Refused(format!("CUPS sent a reply Reprise cannot read ({}).", e.0))
        })?;
        if resp.ok() {
            return Ok(resp);
        }
        Err(match resp.status {
            FORBIDDEN | NOT_AUTHENTICATED | NOT_AUTHORIZED => Error::Denied,
            code => Error::Refused(
                resp.message()
                    .map(sentence)
                    .unwrap_or_else(|| format!("CUPS refused the request (status 0x{code:04x}).")),
            ),
        })
    }
}

impl Default for Client {
    fn default() -> Client {
        Client::new()
    }
}

fn login_name() -> String {
    if let Ok(u) = std::env::var("USER")
        && !u.is_empty()
    {
        return u;
    }
    // SAFETY: getuid cannot fail and touches no memory of ours.
    let uid = unsafe { libc::getuid() };
    std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|p| {
            p.lines().find_map(|l| {
                let f: Vec<&str> = l.split(':').collect();
                (f.get(2)?.parse::<u32>().ok()? == uid).then(|| f[0].to_string())
            })
        })
        .unwrap_or_default()
}

/// cupsd's messages come lowercase and unpunctuated at times; a box
/// shows a sentence.
pub fn sentence(s: &str) -> String {
    let s = s.trim();
    let mut out: String = s
        .chars()
        .take(1)
        .flat_map(char::to_uppercase)
        .chain(s.chars().skip(1))
        .collect();
    if !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

pub fn printer_uri(name: &str) -> String {
    format!("ipp://localhost/printers/{}", escape(name))
}

fn job_uri(id: i32) -> String {
    format!("ipp://localhost/jobs/{id}")
}

fn escape(s: &str) -> String {
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

#[cfg(test)]
mod tests;
