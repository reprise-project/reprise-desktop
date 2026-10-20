//! The thread that owns the conversation with cupsd: it reads printers and jobs (`schedule`
//! says when), runs what the windows ask one request at a time, and reads again after.

use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant, SystemTime};

use crate::avahi;
use crate::cups::{self, Change, Client, Device, Job, JobOp, NewPrinter, Options, Ppd, Snapshot};
use crate::lpwatch;
use crate::notifier::{self, Lease};

/// While a window is up or a job waits with no notifier.
const FAST: Duration = Duration::from_secs(2);
/// While a window is up or a job waits with the notifier. cupsd's D-Bus notifier sends from one
/// subscription only, so another program's with fewer events (no `job-progress`) can hold it.
/// Longer than cupsd's 60 s idle exit, so an open window lets it exit.
const SAFETY: Duration = Duration::from_secs(5 * 60);
/// Idle with no notifier: enough for the tray icon to come up for a job a program sent.
const SLOW: Duration = Duration::from_secs(20);
/// Idle with the notifier: how often the lease is looked at; cupsd is asked only when it is due.
const LEASE_CHECK: Duration = Duration::from_secs(10 * 60);
/// A burst of events reads once.
const SETTLE: Duration = Duration::from_millis(200);
/// How long the backends get to find devices.
const DISCOVERY_SECS: i32 = 6;

#[derive(Clone, Debug)]
pub enum Cmd {
    Refresh,
    /// cupsd raised a printer or job event.
    Changed,
    /// Whether a window is up, so the queue moves while it is watched.
    Watching(bool),
    Pause(String),
    Resume(String),
    CancelAll(String),
    SetDefault(String),
    Delete(String),
    Rename {
        old: String,
        new: String,
    },
    Modify {
        printer: String,
        change: Change,
    },
    TestPage(String),
    Job {
        id: i32,
        op: JobOp,
    },
    Options(String),
    /// Ask cupsd's backends; `prompt` lets cups-pk-helper ask for the admin password.
    Devices {
        prompt: bool,
    },
    /// Browse avahi-daemon's network printers while the wizard is up.
    Network(bool),
    Ppds,
    Add {
        printer: NewPrinter,
        default: bool,
        test_page: bool,
    },
}

#[derive(Debug)]
pub enum Msg {
    Snapshot(Snapshot),
    /// A job left the queue: how it ended.
    JobEnded(Job),
    /// A request failed: the title and body of its message box.
    Failed {
        title: String,
        body: String,
    },
    Options(String, cups::Result<Options>),
    Devices(cups::Result<Vec<Device>>),
    /// The network printers avahi-daemon sees; `None` without avahi-daemon.
    Network(Option<Vec<Device>>),
    Ppds(cups::Result<Vec<Ppd>>),
    Added(cups::Result<String>),
    TestPage(String),
}

#[derive(Clone)]
struct Poster {
    tx: Sender<Msg>,
    waker: repriseapp::Waker,
}

impl Poster {
    fn post(&self, m: Msg) {
        if self.tx.send(m).is_ok() {
            self.waker.wake();
        }
    }
}

#[derive(Clone)]
pub struct Handle(Sender<Cmd>);

impl Handle {
    pub fn send(&self, cmd: Cmd) {
        let _ = self.0.send(cmd);
    }
}

pub fn start(waker: repriseapp::Waker) -> (Receiver<Msg>, Handle) {
    let (tx, rx) = channel();
    let (cmd_tx, cmd_rx) = channel();
    let poster = Poster { tx, waker };
    std::thread::Builder::new()
        .name("cups".into())
        .spawn({
            let cmd_tx = cmd_tx.clone();
            move || run(cmd_rx, cmd_tx, poster)
        })
        .expect("spawn the cups thread");
    (rx, Handle(cmd_tx))
}

struct State {
    client: Client,
    poster: Poster,
    last: Option<Snapshot>,
    jobs: Jobs,
    watching: bool,
    /// Whether the system bus carries cupsd's signals to this thread.
    listening: bool,
    lease: Option<Lease>,
    /// The system bus connection the avahi-daemon browsers live on; closing it ends them.
    network: Option<zbus::blocking::Connection>,
}

fn run(rx: Receiver<Cmd>, tx: Sender<Cmd>, poster: Poster) {
    let lp = tx.clone();
    let listening = notifier::listen(move || {
        let _ = tx.send(Cmd::Changed);
    });
    lpwatch::watch(cups::lpoptions_folders(), move || {
        let _ = lp.send(Cmd::Changed);
    });
    let mut st = State {
        client: Client::new(),
        poster,
        last: None,
        jobs: Jobs::default(),
        watching: false,
        listening,
        lease: None,
        network: None,
    };
    st.keep_subscribed();
    st.refresh();
    let mut next = Instant::now() + st.interval();
    // An event came in and the next tick reads.
    let mut stale = false;
    loop {
        let wait = next.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(Cmd::Changed) => {
                stale = true;
                next = next.min(Instant::now() + SETTLE);
            }
            Ok(cmd) => {
                if st.handle(cmd) {
                    st.refresh();
                    stale = false;
                    next = Instant::now() + st.interval();
                } else {
                    next = next.min(Instant::now() + st.interval());
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                // A new subscription may have missed events.
                let new = st.keep_subscribed();
                if new || stale || st.polling() {
                    st.refresh();
                }
                stale = false;
                next = Instant::now() + st.interval();
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn title(cmd: &Cmd) -> &'static str {
    match cmd {
        Cmd::Pause(_) => "Could not pause the printer",
        Cmd::Resume(_) => "Could not resume printing",
        Cmd::CancelAll(_) => "Could not cancel the documents",
        Cmd::SetDefault(_) => "Could not set the default printer",
        Cmd::Delete(_) => "Could not delete the printer",
        Cmd::Rename { .. } => "Could not rename the printer",
        Cmd::Modify { .. } => "Could not change the printer",
        Cmd::TestPage(_) => "Could not print a test page",
        Cmd::Job {
            op: JobOp::Cancel | JobOp::Remove,
            ..
        } => "Could not cancel the document",
        Cmd::Job {
            op: JobOp::Pause, ..
        } => "Could not pause the document",
        Cmd::Job {
            op: JobOp::Resume, ..
        } => "Could not resume the document",
        Cmd::Job {
            op: JobOp::Restart, ..
        } => "Could not restart the document",
        Cmd::Add { .. } => "Could not add the printer",
        Cmd::Refresh
        | Cmd::Changed
        | Cmd::Watching(_)
        | Cmd::Options(_)
        | Cmd::Devices { .. }
        | Cmd::Network(_)
        | Cmd::Ppds => "Printers",
    }
}

/// How long until the next tick, and whether that tick reads cupsd. Idle with the notifier
/// the tick only looks at the lease.
fn schedule(watching: bool, busy: bool, notified: bool) -> (Duration, bool) {
    match (watching || busy, notified) {
        (true, true) => (SAFETY, true),
        (true, false) => (FAST, true),
        (false, true) => (LEASE_CHECK, false),
        (false, false) => (SLOW, true),
    }
}

impl State {
    fn schedule(&self) -> (Duration, bool) {
        let busy = self.last.as_ref().is_some_and(|s| !s.jobs.is_empty());
        schedule(self.watching, busy, self.listening && self.lease.is_some())
    }

    fn interval(&self) -> Duration {
        self.schedule().0
    }

    fn polling(&self) -> bool {
        self.schedule().1
    }

    /// Renew the subscription when its lease is half gone, or make one while there is none
    /// and the bus is there to hear it. True when a new one was made.
    fn keep_subscribed(&mut self) -> bool {
        if !self.listening {
            return false;
        }
        let now = SystemTime::now();
        match &self.lease {
            Some(l) if !l.due(now) => false,
            Some(l) => match notifier::renew(&self.client, l.id, now) {
                Ok(l) => {
                    self.lease = Some(l);
                    false
                }
                Err(e) => {
                    log::info!("renewing printer events {}: {e:?}", l.id);
                    self.lease = None;
                    self.subscribe(now)
                }
            },
            None => self.subscribe(now),
        }
    }

    fn subscribe(&mut self, now: SystemTime) -> bool {
        match notifier::subscribe(&self.client, now) {
            Ok(l) => {
                log::debug!("printer events: subscription {}", l.id);
                self.lease = Some(l);
                true
            }
            Err(e) => {
                log::info!("no printer events from cupsd, reading it on a timer: {e:?}");
                false
            }
        }
    }

    fn browse(&mut self, on: bool) {
        if let Some(conn) = self.network.take() {
            let _ = conn.close();
        }
        if !on {
            return;
        }
        let conn = match zbus::blocking::Connection::system() {
            Ok(c) => c,
            Err(e) => {
                log::info!("no system bus for network printers: {e}");
                self.poster.post(Msg::Network(None));
                return;
            }
        };
        self.network = Some(conn.clone());
        let poster = self.poster.clone();
        std::thread::spawn(move || match avahi::start(&conn) {
            Ok(signals) => avahi::run(&conn, signals, |d| poster.post(Msg::Network(Some(d)))),
            // A closed connection is a wizard that went away, not a missing avahi-daemon.
            Err(_) if conn.is_closed() => {}
            Err(e) => {
                log::info!("no network printers from avahi-daemon, asking cupsd: {e}");
                poster.post(Msg::Network(None));
            }
        });
    }

    /// Read everything; post the snapshot when it changed and the jobs
    /// that ended since the last read.
    fn refresh(&mut self) {
        let snap = match cups::snapshot(&self.client) {
            Ok(s) => s,
            Err(e) => {
                log::debug!("reading printers: {e:?}");
                Snapshot::default()
            }
        };
        let finished = if snap.running {
            cups::finished_after(&self.client, self.jobs.newest.unwrap_or(0)).unwrap_or_else(|e| {
                log::debug!("reading finished jobs: {e:?}");
                Vec::new()
            })
        } else {
            Vec::new()
        };
        let client = &self.client;
        for j in self.jobs.read(self.last.as_ref(), &snap, finished, |id| {
            cups::job(client, id).ok()
        }) {
            self.poster.post(Msg::JobEnded(j));
        }
        if self.last.as_ref() != Some(&snap) {
            self.last = Some(snap.clone());
            self.poster.post(Msg::Snapshot(snap));
        }
    }

    /// Run one command. True when it may have changed what a read shows.
    fn handle(&mut self, cmd: Cmd) -> bool {
        let c = &self.client;
        let result = match &cmd {
            Cmd::Refresh | Cmd::Changed => return true,
            Cmd::Watching(on) => {
                let was = self.watching;
                self.watching = *on;
                return *on && !was;
            }
            Cmd::Pause(p) => cups::pause(c, p),
            Cmd::Resume(p) => cups::resume(c, p),
            Cmd::CancelAll(p) => cups::cancel_all(c, p),
            Cmd::SetDefault(p) => cups::set_default(c, p),
            Cmd::Delete(p) => cups::delete(c, p),
            Cmd::Rename { old, new } => {
                let snap = self.last.clone().unwrap_or_default();
                cups::rename(c, old, new, &snap)
            }
            Cmd::Modify { printer, change } => cups::modify(c, printer, change.clone()),
            Cmd::TestPage(p) => cups::test_page(c, p).map(|_| {
                self.poster.post(Msg::TestPage(p.clone()));
            }),
            Cmd::Job { id, op } => cups::job_op(c, *id, *op),
            Cmd::Options(p) => {
                let r = cups::options(c, p);
                self.poster.post(Msg::Options(p.clone(), r));
                return false;
            }
            Cmd::Devices { prompt } => {
                let (c, poster, prompt) = (c.clone(), self.poster.clone(), *prompt);
                std::thread::spawn(move || {
                    poster.post(Msg::Devices(cups::devices(&c, DISCOVERY_SECS, prompt)));
                });
                return false;
            }
            Cmd::Network(on) => {
                self.browse(*on);
                return false;
            }
            Cmd::Ppds => {
                let (c, poster) = (c.clone(), self.poster.clone());
                std::thread::spawn(move || poster.post(Msg::Ppds(cups::ppds(&c))));
                return false;
            }
            Cmd::Add {
                printer,
                default,
                test_page,
            } => {
                let r = cups::add(c, printer);
                if r.is_ok() {
                    if *default && let Err(e) = cups::set_default(c, &printer.name) {
                        log::warn!("setting {} as the default: {e:?}", printer.name);
                    }
                    if *test_page {
                        match cups::test_page(c, &printer.name) {
                            Ok(_) => self.poster.post(Msg::TestPage(printer.name.clone())),
                            Err(e) => {
                                if let Some(m) = failed(&Cmd::TestPage(String::new()), &e) {
                                    self.poster.post(m);
                                }
                            }
                        }
                    }
                }
                self.poster
                    .post(Msg::Added(r.map(|()| printer.name.clone())));
                return true;
            }
        };
        if let Err(e) = result {
            log::warn!("{cmd:?}: {e:?}");
            if let Some(m) = failed(&cmd, &e) {
                self.poster.post(m);
            }
        }
        true
    }
}

/// The message box for a failed request; a dismissed password prompt has none.
fn failed(cmd: &Cmd, e: &cups::Error) -> Option<Msg> {
    (*e != cups::Error::Dismissed).then(|| Msg::Failed {
        title: title(cmd).into(),
        body: e.text(),
    })
}

/// The jobs the worker has seen, to tell which ones ended between two reads.
#[derive(Default)]
struct Jobs {
    /// The queue as the last read had it; `None` before the first.
    active: Option<BTreeMap<i32, Job>>,
    /// The highest job id a read of a running cupsd has seen, history included.
    newest: Option<i32>,
}

impl Jobs {
    /// One read: `snap`'s queue (`last` the read before) and `finished`, cupsd's finished jobs numbered after
    /// `newest`. Returns the jobs that ended since the last read: those that left the queue,
    /// ended as `read_back` gives them, and those that came and went in between. Nothing
    /// before a first read has set the marks.
    fn read(
        &mut self,
        last: Option<&Snapshot>,
        snap: &Snapshot,
        finished: Vec<Job>,
        read_back: impl Fn(i32) -> Option<Job>,
    ) -> Vec<Job> {
        let now: BTreeMap<i32, Job> = snap.jobs.iter().map(|j| (j.id, j.clone())).collect();
        let mut out = Vec::new();
        if let Some(active) = &self.active {
            for (id, was) in active {
                if !now.contains_key(id) {
                    out.push(ended(was, read_back(*id), last, snap));
                }
            }
            if let Some(newest) = self.newest {
                let unseen = |j: &&Job| {
                    j.id > newest && !active.contains_key(&j.id) && !now.contains_key(&j.id)
                };
                out.extend(finished.iter().filter(unseen).cloned());
            }
            out.retain(|j| j.state.done());
            out.sort_by_key(|j| j.id);
        }
        if snap.running {
            let ids = now.keys().chain(finished.iter().map(|j| &j.id));
            self.newest = Some(ids.fold(self.newest.unwrap_or(0), |a, &b| a.max(b)));
        }
        self.active = Some(now);
        out
    }
}

/// How job `was`, gone from the queue, ended: as CUPS reads it back (`read`), keeping its printer
/// when the read has none. A job CUPS no longer has printed and was dropped from the history,
/// unless its printer was in `last` and is gone from `snap`: then it went with the printer,
/// cancelled. A class is in neither, so its jobs count as printed.
fn ended(was: &Job, read: Option<Job>, last: Option<&Snapshot>, snap: &Snapshot) -> Job {
    let Some(read) = read else {
        let deleted = last.is_some_and(|l| l.printer(&was.printer).is_some())
            && snap.printer(&was.printer).is_none();
        let state = if deleted {
            cups::JobState::Canceled
        } else {
            cups::JobState::Completed
        };
        return Job {
            state,
            ..was.clone()
        };
    };
    Job {
        printer: if read.printer.is_empty() {
            was.printer.clone()
        } else {
            read.printer
        },
        ..read
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_purged_with_its_printer_ended_cancelled_and_one_purged_alone_printed() {
        use cups::JobState;
        let was = Job {
            id: 7,
            printer: "Office".into(),
            name: "report.pdf".into(),
            state: JobState::Held,
            ..Job::default()
        };
        let with_office = Snapshot {
            running: true,
            printers: vec![cups::Printer {
                name: "Office".into(),
                ..Default::default()
            }],
            ..Snapshot::default()
        };
        let deleted = Snapshot {
            running: true,
            ..Snapshot::default()
        };
        let last = Some(&with_office);
        assert_eq!(ended(&was, None, last, &deleted).state, JobState::Canceled);
        assert_eq!(
            ended(&was, None, last, &with_office).state,
            JobState::Completed
        );

        let read = Job {
            printer: String::new(),
            state: JobState::Canceled,
            ..was.clone()
        };
        let back = ended(&was, Some(read), last, &with_office);
        assert_eq!(
            (back.state, back.printer.as_str()),
            (JobState::Canceled, "Office")
        );
    }

    #[test]
    fn a_job_sent_to_a_class_and_purged_from_the_history_printed() {
        let was = Job {
            id: 8,
            printer: "Floor2".into(),
            state: cups::JobState::Printing,
            ..Job::default()
        };
        let printers = Snapshot {
            running: true,
            printers: vec![cups::Printer {
                name: "Office".into(),
                ..Default::default()
            }],
            ..Snapshot::default()
        };
        assert_eq!(
            ended(&was, None, Some(&printers), &printers).state,
            cups::JobState::Completed
        );
    }

    #[test]
    fn a_job_that_came_and_went_between_two_reads_ends_once_and_history_never_does() {
        use cups::JobState;
        let job = |id: i32, state: JobState| Job {
            id,
            printer: "Office".into(),
            name: format!("doc{id}.pdf"),
            state,
            ..Job::default()
        };
        let queue = |jobs: Vec<Job>| Snapshot {
            running: true,
            printers: vec![cups::Printer {
                name: "Office".into(),
                ..Default::default()
            }],
            jobs,
            ..Snapshot::default()
        };
        let none = |_: i32| None;
        let ids = |v: Vec<Job>| v.iter().map(|j| (j.id, j.state)).collect::<Vec<_>>();
        let mut jobs = Jobs::default();

        let old = vec![job(3, JobState::Completed), job(4, JobState::Canceled)];
        assert_eq!(
            ids(jobs.read(None, &queue(vec![job(5, JobState::Printing)]), old, none)),
            [],
            "the first read only takes the marks"
        );
        assert_eq!(jobs.newest, Some(5));

        let fast = vec![job(5, JobState::Completed), job(6, JobState::Completed)];
        assert_eq!(
            ids(jobs.read(None, &queue(vec![]), fast, |id| Some(job(
                id,
                JobState::Completed
            )))),
            [(5, JobState::Completed), (6, JobState::Completed)],
            "the job that left the queue and the one never seen in it both ended"
        );
        assert_eq!(
            ids(jobs.read(
                None,
                &queue(vec![]),
                vec![job(6, JobState::Completed)],
                none
            )),
            [],
            "a job ends once"
        );
        assert_eq!(
            ids(jobs.read(
                None,
                &queue(vec![job(8, JobState::Pending)]),
                vec![job(7, JobState::Aborted)],
                none
            )),
            [(7, JobState::Aborted)],
            "a failed one ends too, beside a job still queued"
        );

        jobs.read(None, &Snapshot::default(), vec![], none);
        assert_eq!(jobs.newest, Some(8), "a cupsd that is down moves no mark");
    }

    #[test]
    fn with_the_notifier_cupsd_is_read_on_events_and_a_slow_safety_tick() {
        assert_eq!(schedule(false, false, true), (LEASE_CHECK, false));
        assert_eq!(schedule(true, false, true), (SAFETY, true));
        assert_eq!(schedule(false, true, true), (SAFETY, true));
        assert_eq!(schedule(false, false, false), (SLOW, true));
        assert_eq!(schedule(true, false, false), (FAST, true));
        assert_eq!(schedule(false, true, false), (FAST, true));
    }

    #[test]
    fn with_the_notifier_no_tick_reads_cupsd_before_it_may_idle_exit() {
        // cupsd's default IdleExitTimeout under socket activation.
        const CUPSD_IDLE_EXIT: Duration = Duration::from_secs(60);
        for (watching, busy) in [(true, false), (false, true), (true, true)] {
            let (wait, reads) = schedule(watching, busy, true);
            assert!(
                !reads || wait > CUPSD_IDLE_EXIT,
                "a read every {wait:?} keeps cupsd from exiting after {CUPSD_IDLE_EXIT:?} idle"
            );
        }
    }

    #[test]
    fn a_dismissed_prompt_has_no_message_box_and_a_refusal_has_one() {
        let delete = Cmd::Delete("Office".into());
        assert!(failed(&delete, &cups::Error::Dismissed).is_none());
        let Some(Msg::Failed { title, body }) = failed(&delete, &cups::Error::NotAuthorized) else {
            panic!("a refused password is told");
        };
        assert_eq!(title, "Could not delete the printer");
        assert_eq!(body, cups::Error::NotAuthorized.text());
    }
}
