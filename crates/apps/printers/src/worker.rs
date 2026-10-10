//! The work off the event loop: one thread that owns the conversation with cupsd. It reads
//! printers and jobs on a schedule (every couple of seconds while a window is up or a job
//! is waiting, rarely otherwise), runs what the windows ask one request at a time, and reads
//! again after each change. Device discovery and the driver list take seconds, so each runs
//! on its own thread.

use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use crate::cups::{self, Change, Client, Device, Job, JobOp, NewPrinter, Options, Ppd, Snapshot};

const FAST: Duration = Duration::from_secs(2);
/// Otherwise: enough for the tray icon to come up for a job a program
/// sent.
const SLOW: Duration = Duration::from_secs(20);
/// How long the backends get to find devices.
const DISCOVERY_SECS: i32 = 6;

#[derive(Clone, Debug)]
pub enum Cmd {
    Refresh,
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
    Devices,
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
        .spawn(move || run(cmd_rx, poster))
        .expect("spawn the cups thread");
    (rx, Handle(cmd_tx))
}

struct State {
    client: Client,
    poster: Poster,
    last: Option<Snapshot>,
    /// The jobs the last read had, to tell which ones ended since.
    active: BTreeMap<i32, Job>,
    watching: bool,
}

fn run(rx: Receiver<Cmd>, poster: Poster) {
    let mut st = State {
        client: Client::new(),
        poster,
        last: None,
        active: BTreeMap::new(),
        watching: false,
    };
    st.refresh();
    let mut next = Instant::now() + st.interval();
    loop {
        let wait = next.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(cmd) => {
                if st.handle(cmd) {
                    st.refresh();
                    next = Instant::now() + st.interval();
                } else {
                    next = next.min(Instant::now() + st.interval());
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                st.refresh();
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
            op: JobOp::Cancel, ..
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
        Cmd::Refresh | Cmd::Watching(_) | Cmd::Options(_) | Cmd::Devices | Cmd::Ppds => "Printers",
    }
}

impl State {
    fn interval(&self) -> Duration {
        let busy = self.last.as_ref().is_some_and(|s| !s.jobs.is_empty());
        if self.watching || busy { FAST } else { SLOW }
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
        let first = self.last.is_none();
        let now: BTreeMap<i32, Job> = snap.jobs.iter().map(|j| (j.id, j.clone())).collect();
        if !first {
            for (id, was) in &self.active {
                if now.contains_key(id) {
                    continue;
                }
                let ended = cups::job(&self.client, *id).unwrap_or_else(|_| Job {
                    state: cups::JobState::Completed,
                    ..was.clone()
                });
                // A job read back without its printer keeps the one it
                // had.
                let ended = Job {
                    printer: if ended.printer.is_empty() {
                        was.printer.clone()
                    } else {
                        ended.printer
                    },
                    ..ended
                };
                if ended.state.done() {
                    self.poster.post(Msg::JobEnded(ended));
                }
            }
        }
        self.active = now;
        if self.last.as_ref() != Some(&snap) {
            self.last = Some(snap.clone());
            self.poster.post(Msg::Snapshot(snap));
        }
    }

    /// Run one command. True when it may have changed what a read shows.
    fn handle(&mut self, cmd: Cmd) -> bool {
        let c = &self.client;
        let result = match &cmd {
            Cmd::Refresh => return true,
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
            Cmd::Devices => {
                let (c, poster) = (c.clone(), self.poster.clone());
                std::thread::spawn(move || {
                    poster.post(Msg::Devices(cups::devices(&c, DISCOVERY_SECS)));
                });
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
                            Err(e) => self.poster.post(Msg::Failed {
                                title: title(&Cmd::TestPage(String::new())).into(),
                                body: e.text(),
                            }),
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
            self.poster.post(Msg::Failed {
                title: title(&cmd).into(),
                body: e.text(),
            });
        }
        true
    }
}
