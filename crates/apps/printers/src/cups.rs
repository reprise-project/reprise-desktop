//! What reprise-printers asks cupsd, as plain functions over a [`Client`], and the model types the windows read.
//! An admin operation goes over the peer credentials first; if cupsd says no, it goes to cups-pk-helper
//! (pkhelper.rs), else [`Error::NeedsHelper`].

use std::sync::Arc;

use crate::http::{self, Server};
use crate::ipp::{self, JOB, OPERATION, PRINTER, Request, Response, Value};
use crate::pkhelper::{self, Call, Helper};

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

const TEST_PAGES: [(&str, &str); 2] = [
    (
        "/usr/share/cups/data/default-testpage.pdf",
        "application/pdf",
    ),
    (
        "/usr/share/cups/data/testprint",
        "application/vnd.cups-banner",
    ),
];

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrinterState {
    #[default]
    Idle,
    Printing,
    Stopped,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Printer {
    pub name: String,
    pub info: String,
    pub location: String,
    pub make_model: String,
    pub device_uri: String,
    pub state: PrinterState,
    pub message: String,
    pub reasons: Vec<String>,
    pub accepting: bool,
    pub shared: bool,
    pub queued: u32,
}

impl Printer {
    fn from_group(g: &ipp::Group) -> Printer {
        Printer {
            name: g.string("printer-name"),
            info: g.string("printer-info"),
            location: g.string("printer-location"),
            make_model: g.string("printer-make-and-model"),
            device_uri: g.string("device-uri"),
            state: match g.int("printer-state") {
                Some(4) => PrinterState::Printing,
                Some(5) => PrinterState::Stopped,
                _ => PrinterState::Idle,
            },
            message: g.string("printer-state-message"),
            reasons: g
                .strs("printer-state-reasons")
                .into_iter()
                .filter(|r| r != "none")
                .collect(),
            accepting: g.bool("printer-is-accepting-jobs").unwrap_or(true),
            shared: g.bool("printer-is-shared").unwrap_or(false),
            queued: g.int("queued-job-count").unwrap_or(0).max(0) as u32,
        }
    }

    /// Paused by hand, as opposed to stopped on an error: cupsd says
    /// "paused" for both, so an error reason is what tells them apart.
    pub fn paused(&self) -> bool {
        self.state == PrinterState::Stopped && self.problem().is_none()
    }

    pub fn problem(&self) -> Option<&str> {
        self.reasons
            .iter()
            .find(|r| r.ends_with("-error") || !r.contains('-') && r.as_str() != "paused")
            .or_else(|| {
                self.reasons.iter().find(|r| {
                    !r.ends_with("-report") && !r.ends_with("-warning") && r.as_str() != "paused"
                })
            })
            .map(|r| r.trim_end_matches("-error").trim_end_matches("-warning"))
    }

    pub fn status_word(&self) -> String {
        if !self.accepting && self.state != PrinterState::Printing {
            return "Not accepting documents".into();
        }
        match self.state {
            PrinterState::Idle => "Ready".into(),
            PrinterState::Printing => "Printing".into(),
            PrinterState::Stopped if self.paused() => "Paused".into(),
            PrinterState::Stopped => match self.problem() {
                Some(r) => format!("Error: {}", reason_words(r)),
                None => "Error".into(),
            },
        }
    }
}

pub fn reason_words(r: &str) -> String {
    let words = r.replace('-', " ");
    sentence(&words).trim_end_matches('.').to_string()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JobState {
    #[default]
    Pending,
    Held,
    Printing,
    Stopped,
    Canceled,
    Aborted,
    Completed,
}

impl JobState {
    fn from_enum(i: i32) -> JobState {
        match i {
            4 => JobState::Held,
            5 => JobState::Printing,
            6 => JobState::Stopped,
            7 => JobState::Canceled,
            8 => JobState::Aborted,
            9 => JobState::Completed,
            _ => JobState::Pending,
        }
    }

    pub fn done(self) -> bool {
        matches!(
            self,
            JobState::Canceled | JobState::Aborted | JobState::Completed
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Job {
    pub id: i32,
    pub printer: String,
    pub name: String,
    pub owner: String,
    pub state: JobState,
    pub k_octets: i32,
    pub pages: i32,
    pub created: i64,
}

impl Job {
    fn from_group(g: &ipp::Group) -> Job {
        let printer_uri = g.string("job-printer-uri");
        Job {
            id: g.int("job-id").unwrap_or(0),
            printer: printer_uri
                .rsplit('/')
                .next()
                .map(unescape)
                .unwrap_or_default(),
            name: g.string("job-name"),
            owner: g.string("job-originating-user-name"),
            state: JobState::from_enum(g.int("job-state").unwrap_or(3)),
            k_octets: g.int("job-k-octets").unwrap_or(0),
            pages: g
                .int("job-impressions")
                .or_else(|| g.int("job-media-sheets-completed"))
                .unwrap_or(0),
            created: g.int("time-at-creation").unwrap_or(0) as i64,
        }
    }

    pub fn status_word(&self) -> &'static str {
        match self.state {
            JobState::Pending => "Waiting",
            JobState::Held => "Paused",
            JobState::Printing => "Printing",
            JobState::Stopped => "Error",
            JobState::Canceled => "Deleting",
            JobState::Aborted => "Error",
            JobState::Completed => "Printed",
        }
    }
}

fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub running: bool,
    pub printers: Vec<Printer>,
    pub default: Option<String>,
    pub jobs: Vec<Job>,
}

impl Snapshot {
    pub fn printer(&self, name: &str) -> Option<&Printer> {
        self.printers.iter().find(|p| p.name == name)
    }

    pub fn jobs_of<'a>(&'a self, printer: &'a str) -> impl Iterator<Item = &'a Job> {
        self.jobs.iter().filter(move |j| j.printer == printer)
    }
}

const PRINTER_ATTRS: [&str; 12] = [
    "printer-name",
    "printer-info",
    "printer-location",
    "printer-make-and-model",
    "device-uri",
    "printer-state",
    "printer-state-message",
    "printer-state-reasons",
    "printer-is-accepting-jobs",
    "printer-is-shared",
    "queued-job-count",
    "printer-type",
];

const JOB_ATTRS: [&str; 9] = [
    "job-id",
    "job-printer-uri",
    "job-name",
    "job-originating-user-name",
    "job-state",
    "job-k-octets",
    "job-impressions",
    "job-media-sheets-completed",
    "time-at-creation",
];

/// The printers, the default and the unfinished jobs. A cupsd that does
/// not answer is a snapshot that says so.
pub fn snapshot(c: &Client) -> Result<Snapshot> {
    let printers = printers(c)?;
    let jobs = jobs(c, None)?;
    let server_default = c
        .call(
            "/",
            Request::new(CUPS_GET_DEFAULT).requested(&["printer-name"]),
        )
        .ok()
        .and_then(|r| r.first(PRINTER).map(|g| g.string("printer-name")))
        .filter(|n| !n.is_empty());
    let default = own_default()
        .filter(|n| printers.iter().any(|p| &p.name == n))
        .or(server_default);
    Ok(Snapshot {
        running: true,
        printers,
        default,
        jobs,
    })
}

fn printers(c: &Client) -> Result<Vec<Printer>> {
    let resp = match c.call(
        "/",
        Request::new(CUPS_GET_PRINTERS).requested(&PRINTER_ATTRS),
    ) {
        Ok(r) => r,
        Err(Error::Refused(_)) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out: Vec<Printer> = resp
        .groups(PRINTER)
        .filter(|g| g.int("printer-type").unwrap_or(0) & 1 == 0)
        .map(Printer::from_group)
        .filter(|p| !p.name.is_empty())
        .collect();
    out.sort_by_key(|p| p.name.to_lowercase());
    Ok(out)
}

pub fn jobs(c: &Client, printer: Option<&str>) -> Result<Vec<Job>> {
    let uri = printer.map_or_else(|| "ipp://localhost/".to_string(), printer_uri);
    let req = Request::new(GET_JOBS)
        .op_attr("printer-uri", Value::uri(&uri))
        .op_attr("which-jobs", Value::keyword("not-completed"))
        .requested(&JOB_ATTRS);
    match c.call("/", req) {
        Ok(r) => {
            let mut jobs: Vec<Job> = r.groups(JOB).map(Job::from_group).collect();
            jobs.sort_by_key(|j| j.id);
            Ok(jobs)
        }
        Err(Error::Refused(_)) => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// One job, finished or not: how the worker learns how a job that left
/// the queue ended.
pub fn job(c: &Client, id: i32) -> Result<Job> {
    let req = Request::new(GET_JOB_ATTRIBUTES)
        .op_attr("job-uri", Value::uri(&job_uri(id)))
        .requested(&JOB_ATTRS);
    let r = c.call("/", req)?;
    r.first(JOB)
        .map(Job::from_group)
        .ok_or_else(|| Error::Refused("CUPS has no such document.".into()))
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
const SYSTEM_LPOPTIONS: &str = "/etc/cups";

fn lpoptions_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".cups/lpoptions"))
}

/// The folders whose lpoptions name a default: the user's, then the system's.
pub fn lpoptions_folders() -> Vec<std::path::PathBuf> {
    let user = lpoptions_path().and_then(|p| p.parent().map(|d| d.to_path_buf()));
    user.into_iter()
        .chain([std::path::PathBuf::from(SYSTEM_LPOPTIONS)])
        .collect()
}

fn own_default() -> Option<String> {
    let system = std::fs::read_to_string(std::path::Path::new(SYSTEM_LPOPTIONS).join("lpoptions"))
        .unwrap_or_default();
    own_default_in(&lpoptions_text(), &system)
}

/// The default an lpoptions file names, the user's before the system's.
fn own_default_in(user: &str, system: &str) -> Option<String> {
    user_default(user).or_else(|| user_default(system))
}

fn lpoptions_text() -> String {
    lpoptions_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default()
}

/// `text` with its `Default` line naming `name`. Other lines, other
/// destinations' options included, stay as they are.
pub fn with_user_default(text: &str, name: &str) -> String {
    let mut out = String::new();
    for l in text.lines() {
        match l.trim().strip_prefix("Default ") {
            // The options after the destination are its own; they move
            // to a Dest line so they stay with it.
            Some(rest) => {
                let mut parts = rest.splitn(2, char::is_whitespace);
                let dest = parts.next().unwrap_or_default();
                if let Some(opts) = parts.next().map(str::trim).filter(|o| !o.is_empty()) {
                    out.push_str(&format!("Dest {dest} {opts}\n"));
                }
            }
            None => {
                out.push_str(l);
                out.push('\n');
            }
        }
    }
    format!("Default {name}\n{out}")
}

fn printer_op(c: &Client, op: u16, name: &str) -> Result<()> {
    let req = Request::new(op).op_attr("printer-uri", Value::uri(&printer_uri(name)));
    c.call("/admin/", req).map(drop)
}

pub fn pause(c: &Client, name: &str) -> Result<()> {
    c.admin(printer_op(c, PAUSE_PRINTER, name), || {
        vec![Call::PrinterSetEnabled(name.into(), false)]
    })
}

pub fn resume(c: &Client, name: &str) -> Result<()> {
    c.admin(printer_op(c, RESUME_PRINTER, name), || {
        vec![Call::PrinterSetEnabled(name.into(), true)]
    })
}

/// Cancel every job on `name`. cups-pk-helper has no per-printer
/// cancel, so it is asked job by job. An account that may not cancel
/// other people's jobs and has no cups-pk-helper cancels its own.
pub fn cancel_all(c: &Client, name: &str) -> Result<()> {
    let all = c.admin(printer_op(c, CANCEL_JOBS, name), || {
        cancel_calls(&jobs(c, Some(name)).unwrap_or_default())
    });
    match all {
        Err(Error::NeedsHelper) => {
            let req =
                Request::new(CANCEL_MY_JOBS).op_attr("printer-uri", Value::uri(&printer_uri(name)));
            c.call("/", req).map(drop)
        }
        r => r,
    }
}

fn cancel_calls(jobs: &[Job]) -> Vec<Call> {
    jobs.iter()
        .map(|j| Call::JobCancelPurge(j.id, false))
        .collect()
}

/// Make `name` the default: the server's, and the user's own when an
/// lpoptions names one (it would hide the server's otherwise). An
/// account that may not set the server's and has no cups-pk-helper sets
/// its own.
pub fn set_default(c: &Client, name: &str) -> Result<()> {
    let text = lpoptions_text();
    let server = c.admin(printer_op(c, CUPS_SET_DEFAULT, name), || {
        vec![Call::PrinterSetDefault(name.into())]
    });
    let has_own = own_default().is_some();
    if has_own || server == Err(Error::NeedsHelper) {
        write_lpoptions(&with_user_default(&text, name))?;
        return Ok(());
    }
    server
}

fn write_lpoptions(text: &str) -> Result<()> {
    let Some(path) = lpoptions_path() else {
        return Err(Error::Refused(
            "There is no home folder to keep the setting in.".into(),
        ));
    };
    let write = || -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("reprise-tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)
    };
    write().map_err(|e| Error::Refused(format!("Could not save the default printer: {e}.")))
}

pub fn delete(c: &Client, name: &str) -> Result<()> {
    c.admin(printer_op(c, CUPS_DELETE_PRINTER, name), || {
        vec![Call::PrinterDelete(name.into())]
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobOp {
    Pause,
    Resume,
    Restart,
    Cancel,
}

pub fn job_op(c: &Client, id: i32, op: JobOp) -> Result<()> {
    let code = match op {
        JobOp::Pause => HOLD_JOB,
        JobOp::Resume => RELEASE_JOB,
        JobOp::Restart => RESTART_JOB,
        JobOp::Cancel => CANCEL_JOB,
    };
    let req = Request::new(code).op_attr("job-uri", Value::uri(&job_uri(id)));
    c.admin(c.call("/jobs/", req).map(drop), || vec![job_call(id, op)])
}

/// A job operation as cups-pk-helper takes it: another user's job needs
/// a password there, the account's own does not.
fn job_call(id: i32, op: JobOp) -> Call {
    match op {
        JobOp::Pause => Call::JobSetHoldUntil(id, "indefinite".into()),
        JobOp::Resume => Call::JobSetHoldUntil(id, "no-hold".into()),
        JobOp::Restart => Call::JobRestart(id),
        JobOp::Cancel => Call::JobCancelPurge(id, false),
    }
}

/// Print a test page on `name`: the PDF cups-filters ships when it is
/// there, which every queue takes, else cupsd's banner page, which needs
/// a banner filter. Printing is not an admin operation: this goes out as
/// the account and never through cups-pk-helper, which cannot print.
pub fn test_page(c: &Client, name: &str) -> Result<i32> {
    let (path, format) = TEST_PAGES
        .iter()
        .find(|(p, _)| std::path::Path::new(p).exists())
        .copied()
        .unwrap_or(TEST_PAGES[TEST_PAGES.len() - 1]);
    let data = std::fs::read(path)
        .map_err(|e| Error::Refused(format!("The test page could not be read ({path}: {e}).")))?;
    let req = Request::new(PRINT_JOB)
        .op_attr("printer-uri", Value::uri(&printer_uri(name)))
        .op_attr("job-name", Value::name("Test Page"))
        .op_attr("document-format", Value::mime(format))
        .with_data(data);
    let r = c.call(&format!("/printers/{}", escape(name)), req)?;
    Ok(r.first(JOB).and_then(|g| g.int("job-id")).unwrap_or(0))
}

/// What Properties changes on a printer, field by field: `None` leaves
/// it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Change {
    pub info: Option<String>,
    pub location: Option<String>,
    pub device_uri: Option<String>,
    pub shared: Option<bool>,
    pub defaults: Vec<(String, Value)>,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        self.info.is_none()
            && self.location.is_none()
            && self.device_uri.is_none()
            && self.shared.is_none()
            && self.defaults.is_empty()
    }

    fn into_request(self, name: &str) -> Request {
        let mut req = Request::new(CUPS_ADD_MODIFY_PRINTER)
            .op_attr("printer-uri", Value::uri(&printer_uri(name)));
        if let Some(s) = self.info {
            req = req.printer_attr("printer-info", Value::text(&s));
        }
        if let Some(s) = self.location {
            req = req.printer_attr("printer-location", Value::text(&s));
        }
        if let Some(s) = self.device_uri {
            req = req.printer_attr("device-uri", Value::uri(&s));
        }
        if let Some(b) = self.shared {
            req = req.printer_attr("printer-is-shared", Value::Bool(b));
        }
        for (k, v) in self.defaults {
            req = req.printer_attr(&k, v);
        }
        req
    }

    fn calls(&self, name: &str) -> Vec<Call> {
        let n = || name.to_string();
        let mut out = Vec::new();
        if let Some(s) = &self.info {
            out.push(Call::PrinterSetInfo(n(), s.clone()));
        }
        if let Some(s) = &self.location {
            out.push(Call::PrinterSetLocation(n(), s.clone()));
        }
        if let Some(s) = &self.device_uri {
            out.push(Call::PrinterSetDevice(n(), s.clone()));
        }
        if let Some(b) = self.shared {
            out.push(Call::PrinterSetShared(n(), b));
        }
        for (k, v) in &self.defaults {
            let text = match v {
                Value::Int(i) | Value::Enum(i) => i.to_string(),
                Value::Str(_, s) => s.clone(),
                Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            let option = k.strip_suffix("-default").unwrap_or(k).to_string();
            out.push(Call::PrinterAddOptionDefault(n(), option, vec![text]));
        }
        out
    }
}

pub fn modify(c: &Client, name: &str, change: Change) -> Result<()> {
    if change.is_empty() {
        return Ok(());
    }
    let calls = change.calls(name);
    c.admin(
        c.call("/admin/", change.into_request(name)).map(drop),
        || calls,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Driver {
    Everywhere,
    Ppd(String),
    File(std::path::PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewPrinter {
    pub name: String,
    pub device_uri: String,
    pub driver: Driver,
    pub info: String,
    pub location: String,
}

pub fn add(c: &Client, p: &NewPrinter) -> Result<()> {
    let mut req = Request::new(CUPS_ADD_MODIFY_PRINTER)
        .op_attr("printer-uri", Value::uri(&printer_uri(&p.name)))
        .printer_attr("device-uri", Value::uri(&p.device_uri))
        .printer_attr("printer-info", Value::text(&p.info))
        .printer_attr("printer-location", Value::text(&p.location))
        .printer_attr("printer-is-accepting-jobs", Value::Bool(true))
        .printer_attr("printer-state", Value::Enum(3));
    match &p.driver {
        Driver::Everywhere => req = req.op_attr("ppd-name", Value::name("everywhere")),
        Driver::Ppd(name) => req = req.op_attr("ppd-name", Value::name(name)),
        Driver::File(path) => {
            let data = std::fs::read(path).map_err(|e| {
                Error::Refused(format!("{} could not be read: {e}.", path.display()))
            })?;
            req = req.with_data(data);
        }
    }
    c.admin(c.call("/admin/", req).map(drop), || add_calls(p))
}

/// Adding as cups-pk-helper does it: its add leaves the printer's state
/// to cupsd, so enabling and accepting follow.
fn add_calls(p: &NewPrinter) -> Vec<Call> {
    let (name, uri, info, location) = (
        p.name.clone(),
        p.device_uri.clone(),
        p.info.clone(),
        p.location.clone(),
    );
    let add = match &p.driver {
        Driver::Everywhere => Call::PrinterAdd {
            name,
            uri,
            ppd: "everywhere".into(),
            info,
            location,
        },
        Driver::Ppd(ppd) => Call::PrinterAdd {
            name,
            uri,
            ppd: ppd.clone(),
            info,
            location,
        },
        Driver::File(path) => Call::PrinterAddWithPpdFile {
            name,
            uri,
            file: path.display().to_string(),
            info,
            location,
        },
    };
    vec![
        add,
        Call::PrinterSetEnabled(p.name.clone(), true),
        Call::PrinterSetAcceptJobs(p.name.clone(), true),
    ]
}

/// Rename a printer: CUPS has no rename, so a new queue takes the old
/// one's address, driver, description and sharing, its jobs move over,
/// it becomes the default if the old one was, and the old one goes.
/// cups-pk-helper's PrinterRename takes the same steps as root. The
/// user's own default follows the name either way.
pub fn rename(c: &Client, old: &str, new: &str, s: &Snapshot) -> Result<()> {
    c.admin(rename_by_peer(c, old, new, s), || {
        vec![Call::PrinterRename(old.into(), new.into())]
    })?;
    let text = lpoptions_text();
    if user_default(&text).as_deref() == Some(old) {
        write_lpoptions(&with_user_default(&text, new))?;
    }
    Ok(())
}

fn rename_by_peer(c: &Client, old: &str, new: &str, s: &Snapshot) -> Result<()> {
    let Some(p) = s.printer(old) else {
        return Err(Error::Refused(format!("{old} is no longer there.")));
    };
    if s.printer(new).is_some() {
        return Err(Error::Refused(format!(
            "There is already a printer called {new}."
        )));
    }
    let ppd = http::get(
        &c.server,
        &format!("/printers/{}.ppd", escape(old)),
        &c.user,
    );
    let mut req = Request::new(CUPS_ADD_MODIFY_PRINTER)
        .op_attr("printer-uri", Value::uri(&printer_uri(new)))
        .printer_attr("device-uri", Value::uri(&p.device_uri))
        .printer_attr("printer-info", Value::text(&p.info))
        .printer_attr("printer-location", Value::text(&p.location))
        .printer_attr("printer-is-shared", Value::Bool(p.shared))
        .printer_attr("printer-is-accepting-jobs", Value::Bool(true))
        .printer_attr("printer-state", Value::Enum(3));
    match ppd {
        Ok(r) if !r.body.is_empty() => req = req.with_data(r.body),
        Ok(_) | Err(http::Error::Failed(_)) => {}
        Err(e) => return Err(e.into()),
    }
    c.call("/admin/", req)?;
    for j in s.jobs_of(old) {
        let req = Request::new(CUPS_MOVE_JOB)
            .op_attr("job-uri", Value::uri(&job_uri(j.id)))
            .add(JOB, "job-printer-uri", vec![Value::uri(&printer_uri(new))]);
        if let Err(e) = c.call("/jobs/", req) {
            log::warn!("moving job {} to {new}: {e:?}", j.id);
        }
    }
    if s.default.as_deref() == Some(old) {
        set_default(c, new)?;
    }
    if p.paused() {
        pause(c, new)?;
    }
    delete(c, old)
}

/// A device cupsd's backends found: a USB printer, a network printer
/// announced over DNS-SD.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    pub uri: String,
    pub info: String,
    pub make_model: String,
    pub class: String,
    pub device_id: String,
}

impl Device {
    /// The maker the device ID names, else the first word of the make
    /// and model: what the driver search starts on.
    pub fn maker(&self) -> String {
        let from_id = self.device_id.split(';').find_map(|f| {
            let (k, v) = f.split_once(':')?;
            matches!(k.trim(), "MFG" | "MANUFACTURER").then(|| v.trim().to_string())
        });
        from_id
            .filter(|m| !m.is_empty())
            .or_else(|| {
                self.make_model
                    .split_whitespace()
                    .next()
                    .map(str::to_string)
            })
            .unwrap_or_default()
    }

    /// Whether the printer speaks IPP itself, so cupsd can set it up with
    /// no driver.
    pub fn driverless(&self) -> bool {
        driverless_uri(&self.uri)
    }

    pub fn label(&self) -> String {
        let name = if self.info.is_empty() {
            &self.make_model
        } else {
            &self.info
        };
        // The driverless backend names its finds "... (driverless)"
        // already.
        let name = name.trim_end_matches(" (driverless)");
        let how = if self.driverless() {
            "network, no driver needed"
        } else if self.class == "network" {
            "network"
        } else {
            "connected to this computer"
        };
        format!("{name} ({how})")
    }
}

/// An address cupsd can reach the printer at over IPP: ipp, ipps, or a
/// DNS-SD name of an IPP service.
pub fn driverless_uri(uri: &str) -> bool {
    uri.starts_with("ipp://")
        || uri.starts_with("ipps://")
        || uri.starts_with("dnssd://") && (uri.contains("._ipp.") || uri.contains("._ipps."))
}

const EXCLUDED_SCHEMES: [&str; 3] = ["cups-brf", "beh", "fax"];

/// The devices cupsd's backends find within `secs` seconds. The ones a
/// backend lists as a protocol without a printer behind it ("socket",
/// "ipp", "lpd") are left out; the wizard asks for an address instead.
pub fn devices(c: &Client, secs: i32) -> Result<Vec<Device>> {
    let req = Request::new(CUPS_GET_DEVICES)
        .op_attr("timeout", Value::Int(secs))
        .add(
            OPERATION,
            "exclude-schemes",
            EXCLUDED_SCHEMES.iter().map(|s| Value::name(s)).collect(),
        );
    let found: Vec<Device> = match c.call("/admin/", req) {
        Ok(r) => r
            .groups(PRINTER)
            .map(|g| Device {
                uri: g.string("device-uri"),
                info: g.string("device-info"),
                make_model: g.string("device-make-and-model"),
                class: g.string("device-class"),
                device_id: g.string("device-id"),
            })
            .collect(),
        Err(Error::Denied) if c.helper.installed() => {
            let found = c.helper.devices(secs, &EXCLUDED_SCHEMES)?;
            pkhelper::devices_from(&found)
                .iter()
                .map(|d| {
                    let get = |k: &str| d.get(k).cloned().unwrap_or_default();
                    Device {
                        uri: get("device-uri"),
                        info: get("device-info"),
                        make_model: get("device-make-and-model"),
                        class: get("device-class"),
                        device_id: get("device-id"),
                    }
                })
                .collect()
        }
        Err(Error::Denied) => return Err(Error::NeedsHelper),
        Err(e) => return Err(e),
    };
    let mut out: Vec<Device> = found
        .into_iter()
        .filter(|d| d.uri.contains(':') && d.uri.contains('/'))
        .collect();
    // A printer both announced over DNS-SD and found another way shows
    // once, as its driverless address when it has one.
    out.sort_by_key(|d| (!d.driverless(), d.info.to_lowercase()));
    out.dedup_by(|b, a| !a.info.is_empty() && a.info == b.info);
    Ok(out)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ppd {
    pub name: String,
    pub make: String,
    pub make_model: String,
}

pub fn ppds(c: &Client) -> Result<Vec<Ppd>> {
    let req =
        Request::new(CUPS_GET_PPDS).requested(&["ppd-name", "ppd-make", "ppd-make-and-model"]);
    let r = match c.call("/", req) {
        Ok(r) => r,
        Err(Error::Refused(_)) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out: Vec<Ppd> = r
        .groups(PRINTER)
        .map(|g| Ppd {
            name: g.string("ppd-name"),
            make: g.string("ppd-make"),
            make_model: g.string("ppd-make-and-model"),
        })
        .filter(|p| !p.name.is_empty())
        .collect();
    out.sort_by_key(|p| p.make_model.to_lowercase());
    out.dedup_by(|b, a| a.make_model == b.make_model);
    Ok(out)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Setting {
    pub choices: Vec<String>,
    pub default: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub media: Setting,
    pub sides: Setting,
    pub color: Setting,
    pub quality: Setting,
}

impl Options {
    fn from_group(g: &ipp::Group) -> Options {
        let setting = |name: &str| {
            let text = |v: &Value| match v {
                Value::Int(i) | Value::Enum(i) => Some(i.to_string()),
                Value::Str(_, s) => Some(s.clone()),
                _ => None,
            };
            Setting {
                choices: g
                    .get(&format!("{name}-supported"))
                    .unwrap_or_default()
                    .iter()
                    .filter_map(text)
                    .collect(),
                default: g
                    .get(&format!("{name}-default"))
                    .and_then(|v| v.first())
                    .and_then(text),
            }
        };
        Options {
            media: setting("media"),
            sides: setting("sides"),
            color: setting("print-color-mode"),
            quality: setting("print-quality"),
        }
    }
}

pub fn options(c: &Client, name: &str) -> Result<Options> {
    let req = Request::new(GET_PRINTER_ATTRIBUTES)
        .op_attr("printer-uri", Value::uri(&printer_uri(name)))
        .requested(&[
            "media-supported",
            "media-default",
            "sides-supported",
            "sides-default",
            "print-color-mode-supported",
            "print-color-mode-default",
            "print-quality-supported",
            "print-quality-default",
        ]);
    let r = c.call("/", req)?;
    Ok(r.first(PRINTER)
        .map(Options::from_group)
        .unwrap_or_default())
}

/// A PWG media keyword as a size name: "iso_a4_210x297mm" reads
/// "A4 (210 x 297 mm)", "na_letter_8.5x11in" "Letter (8.5 x 11 in)".
pub fn media_label(k: &str) -> String {
    let parts: Vec<&str> = k.split('_').collect();
    if parts.len() < 3 {
        return k.to_string();
    }
    let size = parts[parts.len() - 1];
    let name = parts[1..parts.len() - 1].join(" ");
    let name: Vec<String> = name
        .split(['-', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let code = w.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && w.chars().skip(1).all(|c| c.is_ascii_digit());
            if code || w.len() <= 2 && w.chars().all(|c| c.is_ascii_alphabetic()) {
                w.to_uppercase()
            } else {
                let mut c = w.chars();
                c.next()
                    .map(|f| f.to_uppercase().chain(c).collect())
                    .unwrap_or_default()
            }
        })
        .collect();
    let (dims, unit) = if let Some(d) = size.strip_suffix("mm") {
        (d, "mm")
    } else if let Some(d) = size.strip_suffix("in") {
        (d, "in")
    } else {
        (size, "")
    };
    match dims.split_once('x') {
        Some((w, h)) if !unit.is_empty() => format!("{} ({w} x {h} {unit})", name.join(" ")),
        _ => name.join(" "),
    }
}

pub fn sides_label(k: &str) -> String {
    match k {
        "one-sided" => "Off".into(),
        "two-sided-long-edge" => "Long edge (standard)".into(),
        "two-sided-short-edge" => "Short edge (flip up)".into(),
        k => reason_words(k),
    }
}

pub fn color_label(k: &str) -> String {
    match k {
        "monochrome" => "Black and white".into(),
        "color" => "Color".into(),
        "auto" => "Automatic".into(),
        "auto-monochrome" => "Automatic black and white".into(),
        "process-monochrome" => "Black and white (process)".into(),
        k => reason_words(k),
    }
}

pub fn quality_label(k: &str) -> String {
    match k {
        "3" => "Draft".into(),
        "4" => "Normal".into(),
        "5" => "High".into(),
        k => k.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(attrs: Vec<(&str, Vec<Value>)>) -> ipp::Group {
        ipp::Group {
            tag: PRINTER,
            attrs: attrs
                .into_iter()
                .map(|(n, v)| ipp::Attr {
                    name: n.into(),
                    values: v,
                })
                .collect(),
        }
    }

    #[test]
    fn a_printer_reads_its_state_and_reasons() {
        let g = group(vec![
            ("printer-name", vec![Value::name("Laser")]),
            ("printer-state", vec![Value::Enum(5)]),
            ("printer-state-reasons", vec![Value::keyword("paused")]),
            ("printer-is-shared", vec![Value::Bool(true)]),
            ("queued-job-count", vec![Value::Int(2)]),
        ]);
        let p = Printer::from_group(&g);
        assert_eq!(p.name, "Laser");
        assert!(p.paused());
        assert_eq!(p.status_word(), "Paused");
        assert!(p.shared);
        assert_eq!(p.queued, 2);

        let g = group(vec![
            ("printer-state", vec![Value::Enum(5)]),
            (
                "printer-state-reasons",
                vec![
                    Value::keyword("paused"),
                    Value::keyword("media-empty-error"),
                ],
            ),
        ]);
        let p = Printer::from_group(&g);
        assert!(!p.paused(), "an error is not a pause");
        assert_eq!(p.status_word(), "Error: Media empty");

        let idle = Printer::from_group(&group(vec![(
            "printer-state-reasons",
            vec![Value::keyword("none")],
        )]));
        assert!(idle.reasons.is_empty());
        assert_eq!(idle.status_word(), "Ready");
    }

    #[test]
    fn a_job_names_its_printer_from_its_uri() {
        let g = group(vec![
            ("job-id", vec![Value::Int(7)]),
            (
                "job-printer-uri",
                vec![Value::uri("ipp://localhost/printers/Front%20Desk")],
            ),
            ("job-state", vec![Value::Enum(4)]),
            ("job-name", vec![Value::name("report.pdf")]),
        ]);
        let j = Job::from_group(&g);
        assert_eq!(j.id, 7);
        assert_eq!(j.printer, "Front Desk");
        assert_eq!(j.state, JobState::Held);
        assert_eq!(j.status_word(), "Paused");
        assert_eq!(
            printer_uri("Front Desk"),
            "ipp://localhost/printers/Front%20Desk"
        );
    }

    #[test]
    fn the_users_default_reads_and_writes_in_lpoptions() {
        assert_eq!(user_default(""), None);
        let text = "Dest Other sides=one-sided\nDefault Laser/draft media=a4\n";
        assert_eq!(user_default(text), Some("Laser".into()));
        let new = with_user_default(text, "Inkjet");
        assert_eq!(
            new,
            "Default Inkjet\nDest Other sides=one-sided\nDest Laser/draft media=a4\n"
        );
        assert_eq!(user_default(&new), Some("Inkjet".into()));
        assert_eq!(own_default_in(&new, text), Some("Inkjet".into()));
        assert_eq!(
            own_default_in("Dest Other\n", text),
            Some("Laser".into()),
            "the system's lpoptions names it when the user's does not"
        );
        assert_eq!(own_default_in("", ""), None);
    }

    #[test]
    fn devices_are_driverless_over_ipp() {
        assert!(driverless_uri("ipp://10.0.0.5/ipp/print"));
        assert!(driverless_uri(
            "dnssd://Office%20Jet._ipp._tcp.local/?uuid=1"
        ));
        assert!(!driverless_uri(
            "dnssd://Office%20Jet._pdl-datastream._tcp.local/"
        ));
        assert!(!driverless_uri("usb://HP/LaserJet?serial=1"));
        let d = Device {
            uri: "usb://HP/LaserJet".into(),
            info: "HP LaserJet".into(),
            class: "direct".into(),
            ..Default::default()
        };
        assert_eq!(d.label(), "HP LaserJet (connected to this computer)");
        assert_eq!(d.maker(), "", "no device ID and no make and model");
        let found = Device {
            uri: "ipps://Lab._ipps._tcp.local/".into(),
            info: "Lab (driverless)".into(),
            make_model: "Example Printer".into(),
            device_id: "MFG:Example;MDL:Printer;CMD:PDF;".into(),
            ..Default::default()
        };
        assert_eq!(found.label(), "Lab (network, no driver needed)");
        assert_eq!(found.maker(), "Example");
        let pdf = Device {
            make_model: "Virtual PDF Printer".into(),
            ..Default::default()
        };
        assert_eq!(pdf.maker(), "Virtual");
    }

    #[test]
    fn choices_read_as_words() {
        assert_eq!(media_label("iso_a4_210x297mm"), "A4 (210 x 297 mm)");
        assert_eq!(media_label("na_letter_8.5x11in"), "Letter (8.5 x 11 in)");
        assert_eq!(media_label("na_index-4x6_4x6in"), "Index 4x6 (4 x 6 in)");
        assert_eq!(media_label("custom"), "custom");
        assert_eq!(media_label("iso_dl_110x220mm"), "DL (110 x 220 mm)");
        assert_eq!(sides_label("two-sided-long-edge"), "Long edge (standard)");
        assert_eq!(color_label("monochrome"), "Black and white");
        assert_eq!(quality_label("5"), "High");
    }

    #[test]
    fn options_read_supported_and_default() {
        let g = group(vec![
            (
                "media-supported",
                vec![
                    Value::keyword("iso_a4_210x297mm"),
                    Value::keyword("na_letter_8.5x11in"),
                ],
            ),
            ("media-default", vec![Value::keyword("iso_a4_210x297mm")]),
            (
                "print-quality-supported",
                vec![Value::Enum(3), Value::Enum(4)],
            ),
            ("print-quality-default", vec![Value::Enum(4)]),
        ]);
        let o = Options::from_group(&g);
        assert_eq!(o.media.choices.len(), 2);
        assert_eq!(o.media.default.as_deref(), Some("iso_a4_210x297mm"));
        assert_eq!(o.quality.choices, ["3", "4"]);
        assert_eq!(o.quality.default.as_deref(), Some("4"));
        assert!(o.sides.choices.is_empty());
    }

    #[test]
    fn messages_read_as_sentences() {
        assert_eq!(sentence("printer not found"), "Printer not found.");
        assert_eq!(sentence("Done!"), "Done!");
    }

    fn client(helper: &Arc<pkhelper::fake::Fake>) -> Client {
        Client {
            server: Server::Tcp("localhost:631".into()),
            user: "ann".into(),
            helper: helper.clone(),
        }
    }

    #[test]
    fn admin_operations_go_to_the_helper_only_when_cupsd_says_no() {
        let delete = || vec![Call::PrinterDelete("Laser".into())];

        let h = Arc::new(pkhelper::fake::Fake::installed());
        assert_eq!(client(&h).admin(Ok(()), delete), Ok(()));
        assert!(h.calls().is_empty());

        assert_eq!(client(&h).admin(Err(Error::Denied), delete), Ok(()));
        assert_eq!(h.calls(), delete());

        let h = Arc::new(pkhelper::fake::Fake::installed());
        let refused = Err(Error::Refused("Printer not found.".into()));
        assert_eq!(client(&h).admin(refused.clone(), delete), refused);
        assert!(h.calls().is_empty());

        let h = Arc::new(pkhelper::fake::Fake::default());
        assert_eq!(
            client(&h).admin(Err(Error::Denied), delete),
            Err(Error::NeedsHelper)
        );
        assert!(h.calls().is_empty());
        assert!(Error::NeedsHelper.text().contains("Install cups-pk-helper"));

        let h = Arc::new(pkhelper::fake::Fake {
            answer: Some(Error::NotAuthorized),
            ..pkhelper::fake::Fake::installed()
        });
        assert_eq!(
            client(&h).admin(Err(Error::Denied), delete),
            Err(Error::NotAuthorized)
        );
        assert!(
            Error::NotAuthorized
                .text()
                .contains("administrator's password")
        );
    }

    #[test]
    fn a_change_becomes_one_helper_call_per_field() {
        let change = Change {
            info: Some("Front desk".into()),
            location: Some("Hall".into()),
            device_uri: Some("socket://10.0.0.9".into()),
            shared: Some(true),
            defaults: vec![
                ("media-default".into(), Value::keyword("iso_a4_210x297mm")),
                ("print-quality-default".into(), Value::Enum(5)),
            ],
        };
        let n = || "Laser".to_string();
        assert_eq!(
            change.calls("Laser"),
            [
                Call::PrinterSetInfo(n(), "Front desk".into()),
                Call::PrinterSetLocation(n(), "Hall".into()),
                Call::PrinterSetDevice(n(), "socket://10.0.0.9".into()),
                Call::PrinterSetShared(n(), true),
                Call::PrinterAddOptionDefault(n(), "media".into(), vec!["iso_a4_210x297mm".into()]),
                Call::PrinterAddOptionDefault(n(), "print-quality".into(), vec!["5".into()]),
            ]
        );
        assert!(Change::default().calls("Laser").is_empty());
    }

    #[test]
    fn adding_through_the_helper_names_the_driver_then_enables() {
        let mut p = NewPrinter {
            name: "Lab".into(),
            device_uri: "ipp://10.0.0.5/ipp/print".into(),
            driver: Driver::Everywhere,
            info: "Lab printer".into(),
            location: "Room 2".into(),
        };
        let tail = [
            Call::PrinterSetEnabled("Lab".into(), true),
            Call::PrinterSetAcceptJobs("Lab".into(), true),
        ];
        let add = |ppd: &str| Call::PrinterAdd {
            name: "Lab".into(),
            uri: "ipp://10.0.0.5/ipp/print".into(),
            ppd: ppd.into(),
            info: "Lab printer".into(),
            location: "Room 2".into(),
        };
        assert_eq!(add_calls(&p)[0], add("everywhere"));
        assert_eq!(add_calls(&p)[1..], tail);
        p.driver = Driver::Ppd("drv:///sample.drv/generic.ppd".into());
        assert_eq!(add_calls(&p)[0], add("drv:///sample.drv/generic.ppd"));
        p.driver = Driver::File("/home/ann/laser.ppd".into());
        assert_eq!(
            add_calls(&p)[0],
            Call::PrinterAddWithPpdFile {
                name: "Lab".into(),
                uri: "ipp://10.0.0.5/ipp/print".into(),
                file: "/home/ann/laser.ppd".into(),
                info: "Lab printer".into(),
                location: "Room 2".into(),
            }
        );
        assert_eq!(add_calls(&p)[1..], tail);
    }

    #[test]
    fn job_operations_map_to_the_helpers_job_calls() {
        assert_eq!(
            job_call(7, JobOp::Pause),
            Call::JobSetHoldUntil(7, "indefinite".into())
        );
        assert_eq!(
            job_call(7, JobOp::Resume),
            Call::JobSetHoldUntil(7, "no-hold".into())
        );
        assert_eq!(job_call(7, JobOp::Restart), Call::JobRestart(7));
        assert_eq!(job_call(7, JobOp::Cancel), Call::JobCancelPurge(7, false));
        let jobs = [3, 9].map(|id| Job {
            id,
            ..Job::default()
        });
        assert_eq!(
            cancel_calls(&jobs),
            [
                Call::JobCancelPurge(3, false),
                Call::JobCancelPurge(9, false)
            ]
        );
    }
}
