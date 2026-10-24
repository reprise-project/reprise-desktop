//! Printers and jobs as the windows read them, and the snapshot of both.

use super::*;

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
    pub(super) fn from_group(g: &ipp::Group) -> Printer {
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
    /// CUPS still holds the job's files, which Restart-Job needs.
    pub preserved: bool,
}

impl Job {
    pub(super) fn from_group(g: &ipp::Group) -> Job {
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
            preserved: g.bool("job-preserved").unwrap_or(false),
        }
    }

    /// cupsd restarts a stopped job, or a finished one whose files it kept.
    pub fn restartable(&self) -> bool {
        self.state == JobState::Stopped || (self.state.done() && self.preserved)
    }

    pub fn status_word(&self) -> &'static str {
        match self.state {
            JobState::Pending => "Waiting",
            JobState::Held => "Paused",
            JobState::Printing => "Printing",
            JobState::Stopped => "Stopped",
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
    /// Aborted jobs whose files CUPS still holds: they wait in the queue
    /// window for a restart or a cancel.
    pub aborted: Vec<Job>,
}

impl Snapshot {
    pub fn printer(&self, name: &str) -> Option<&Printer> {
        self.printers.iter().find(|p| p.name == name)
    }

    pub fn jobs_of<'a>(&'a self, printer: &'a str) -> impl Iterator<Item = &'a Job> {
        self.jobs.iter().filter(move |j| j.printer == printer)
    }

    /// A printer's documents: its unfinished jobs and the aborted ones
    /// CUPS keeps, as the folder counts them and the queue window lists them.
    pub fn documents_of<'a>(&'a self, printer: &'a str) -> impl Iterator<Item = &'a Job> {
        self.jobs
            .iter()
            .chain(&self.aborted)
            .filter(move |j| j.printer == printer)
    }

    /// The documents a printer's queue window lists, oldest first.
    pub fn queue_of(&self, printer: &str) -> Vec<Job> {
        let mut out: Vec<Job> = self.documents_of(printer).cloned().collect();
        out.sort_by_key(|j| j.id);
        out
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

const JOB_ATTRS: [&str; 10] = [
    "job-id",
    "job-printer-uri",
    "job-name",
    "job-originating-user-name",
    "job-state",
    "job-k-octets",
    "job-impressions",
    "job-media-sheets-completed",
    "time-at-creation",
    "job-preserved",
];

/// The printers, the default, the unfinished jobs and the restartable
/// aborted ones. A cupsd that does not answer is a snapshot that says so.
pub fn snapshot(c: &Client) -> Result<Snapshot> {
    let printers = printers(c)?;
    let jobs = jobs(c, None)?;
    let aborted = aborted(c, None)?;
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
        aborted,
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
    get_jobs(c, printer, "not-completed")
}

pub(super) fn aborted(c: &Client, printer: Option<&str>) -> Result<Vec<Job>> {
    let mut out = get_jobs(c, printer, "aborted")?;
    out.retain(|j| j.state == JobState::Aborted && j.preserved);
    Ok(out)
}

/// Get-Jobs for `which`, an RFC 8011 `which-jobs` keyword or one of CUPS's
/// extensions such as `aborted`.
fn get_jobs(c: &Client, printer: Option<&str>, which: &str) -> Result<Vec<Job>> {
    let uri = printer.map_or_else(|| "ipp://localhost/".to_string(), printer_uri);
    let req = Request::new(GET_JOBS)
        .op_attr("printer-uri", Value::uri(&uri))
        .op_attr("which-jobs", Value::keyword(which))
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

/// The finished jobs numbered after `after`, from cupsd's history: how the worker learns of a job
/// that came and went between two reads. `first-job-id` is a CUPS extension to Get-Jobs.
pub fn finished_after(c: &Client, after: i32) -> Result<Vec<Job>> {
    let req = Request::new(GET_JOBS)
        .op_attr("printer-uri", Value::uri("ipp://localhost/"))
        .op_attr("which-jobs", Value::keyword("completed"))
        .op_attr("first-job-id", Value::Int(after.saturating_add(1)))
        .requested(&JOB_ATTRS);
    match c.call("/", req) {
        Ok(r) => Ok(r
            .groups(JOB)
            .map(Job::from_group)
            .filter(|j| j.id > after)
            .collect()),
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
