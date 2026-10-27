//! Work off the event loop: one thread runs checks and installs serially and
//! checks on a schedule; others check when PackageKit, fwupd or a Flatpak
//! installation says something changed.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use futures_util::FutureExt;
use futures_util::stream::StreamExt;
use zbus::Connection;

use crate::model::{Found, Group, Install, Origin, Request, Restart, Update};
use crate::{flatpak, fwupd, packagekit};

const FIRST_CHECK: Duration = Duration::from_secs(90);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const SETTLE: Duration = Duration::from_secs(3);
/// A change heard this soon after a check started reading its source came before the read (the
/// signal and the reply race on separate connections), or came from the check itself.
const HEARD_LATE: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub enum Job {
    /// Look for updates. `refresh` downloads package lists and due firmware
    /// metadata first; `manual` (Check Now) fetches all firmware metadata.
    Check {
        refresh: bool,
        manual: bool,
    },
    Install(Vec<Update>),
    /// A source said it changed at this instant; checked again unless a check has read it since.
    Changed(Group, Instant),
}

#[derive(Debug)]
pub enum Msg {
    Checking,
    Checked(Found),
    Progress {
        fraction: Option<f64>,
        doing: String,
    },
    /// A device's request while its firmware installs, `None` once it carries on.
    Request(Option<Request>),
    Installed(Result<Restart, Failure>),
}

/// Why an install stopped.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// Every password prompt was dismissed: nothing was installed and nothing to tell.
    Dismissed,
    Text(String),
}

type Poster = repriseapp::Poster<Msg>;

#[derive(Clone)]
pub struct Jobs(Sender<Job>);

impl Jobs {
    pub fn send(&self, job: Job) {
        let _ = self.0.send(job);
    }
}

/// Start both threads; `check_now` runs the first check at once.
pub fn start(waker: repriseapp::Waker, check_now: bool) -> (Receiver<Msg>, Jobs) {
    let (jobs_tx, jobs_rx) = channel();
    let (poster, rx) = Poster::channel(waker);
    let first = if check_now {
        Duration::ZERO
    } else {
        FIRST_CHECK
    };
    if let Err(e) = std::thread::Builder::new()
        .name("reprise-updates-jobs".into())
        .spawn(move || run_jobs(jobs_rx, poster, first))
    {
        log::warn!("could not start the updates thread: {e}");
    }
    let jobs = Jobs(jobs_tx);
    let watcher = jobs.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("reprise-updates-watch".into())
        .spawn(move || zbus::block_on(watch(watcher)))
    {
        log::warn!("could not start the updates watcher: {e}");
    }
    let watcher = jobs.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("reprise-updates-flatpak".into())
        .spawn(move || watch_flatpak(watcher))
    {
        log::warn!("could not start the Flatpak watcher: {e}");
    }
    (rx, jobs)
}

/// Installs first, then one check, then the changes, which that check has mostly read already.
fn fold(first: Job, rest: impl Iterator<Item = Job>) -> Vec<Job> {
    let mut installs = Vec::new();
    let mut check: Option<(bool, bool)> = None;
    let mut changes = Vec::new();
    for job in std::iter::once(first).chain(rest) {
        match job {
            Job::Install(u) => installs.push(Job::Install(u)),
            Job::Check { refresh, manual } => {
                let (r, m) = check.unwrap_or((false, false));
                check = Some((r || refresh, m || manual));
            }
            Job::Changed(..) => changes.push(job),
        }
    }
    installs.extend(check.map(|(refresh, manual)| Job::Check { refresh, manual }));
    installs.extend(changes);
    installs
}

/// When each source was last read by a check.
type Read = HashMap<Group, Instant>;

fn seen(read: &Read, group: Group, at: Instant) -> bool {
    read.get(&group).is_some_and(|&r| r + HEARD_LATE > at)
}

fn run_jobs(rx: Receiver<Job>, poster: Poster, first: Duration) {
    let conn = match zbus::block_on(Connection::system()) {
        Ok(c) => Some(c),
        Err(e) => {
            log::info!("no system bus ({e}); only Flatpak updates to look for");
            None
        }
    };
    let mut next_check = Instant::now() + first;
    let mut read = Read::new();
    loop {
        let wait = next_check.saturating_duration_since(Instant::now());
        let job = match rx.recv_timeout(wait) {
            Ok(job) => job,
            Err(RecvTimeoutError::Timeout) => Job::Check {
                refresh: true,
                manual: false,
            },
            Err(RecvTimeoutError::Disconnected) => return,
        };
        for job in fold(job, rx.try_iter()) {
            match job {
                Job::Check { refresh, manual } => {
                    if refresh {
                        next_check = Instant::now() + CHECK_EVERY;
                    }
                    poster.post(Msg::Checking);
                    let found = zbus::block_on(check(conn.as_ref(), refresh, manual, &mut read));
                    poster.post(Msg::Checked(found));
                }
                Job::Changed(group, at) => {
                    if seen(&read, group, at) {
                        continue;
                    }
                    log::info!("{group:?} changed; checking");
                    poster.post(Msg::Checking);
                    let found = zbus::block_on(check(conn.as_ref(), false, false, &mut read));
                    poster.post(Msg::Checked(found));
                }
                Job::Install(updates) => {
                    let done = zbus::block_on(install(conn.as_ref(), updates, &poster));
                    poster.post(Msg::Installed(done));
                    poster.post(Msg::Checking);
                    let found = zbus::block_on(check(conn.as_ref(), false, false, &mut read));
                    poster.post(Msg::Checked(found));
                }
            }
        }
    }
}

async fn check(conn: Option<&Connection>, refresh: bool, manual: bool, read: &mut Read) -> Found {
    let mut found = Found::new();
    if let Some(c) = conn {
        match packagekit::updates(c, refresh).await {
            Ok(u) => found.push((Group::Packages, Ok(u))),
            Err(packagekit::Error::Unavailable) => {
                log::info!("PackageKit is not installed; no package updates");
            }
            Err(packagekit::Error::Failed(e)) => found.push((Group::Packages, Err(e))),
            Err(packagekit::Error::Dismissed) => {}
        }
    }
    if flatpak::present() {
        read.insert(Group::Flatpak, Instant::now());
        found.push((Group::Flatpak, flatpak::updates()));
    }
    if let Some(c) = conn {
        if refresh && let Err(fwupd::Error::Failed(e)) = fwupd::refresh(c, manual).await {
            log::info!("refreshing the firmware metadata: {e}");
        }
        read.insert(Group::Firmware, Instant::now());
        match fwupd::updates(c).await {
            Ok(u) => found.push((Group::Firmware, Ok(u))),
            Err(fwupd::Error::Unavailable) => {
                log::info!("fwupd is not installed; no firmware updates");
            }
            Err(fwupd::Error::Failed(e)) => found.push((Group::Firmware, Err(e))),
            Err(fwupd::Error::Dismissed) => {}
        }
    }
    found
}

#[derive(Debug, PartialEq)]
enum Stage {
    Packages(Vec<String>),
    Flatpak(Install, Vec<Update>),
    Firmware(Update),
}

fn stages(updates: Vec<Update>) -> Vec<Stage> {
    let mut packages = Vec::new();
    let (mut system, mut user) = (Vec::new(), Vec::new());
    let mut firmware = Vec::new();
    for u in updates {
        match u.origin {
            Origin::Package => packages.push(u.id),
            Origin::App(Install::System) | Origin::Runtime(Install::System) => system.push(u),
            Origin::App(Install::User) | Origin::Runtime(Install::User) => user.push(u),
            Origin::Firmware => firmware.push(Stage::Firmware(u)),
        }
    }
    let mut out = Vec::new();
    if !packages.is_empty() {
        out.push(Stage::Packages(packages));
    }
    for (at, refs) in [(Install::System, system), (Install::User, user)] {
        if !refs.is_empty() {
            out.push(Stage::Flatpak(at, refs));
        }
    }
    out.extend(firmware);
    out
}

/// The name of the update whose ref has flatpak id `id`, else the id.
fn ref_name(refs: &[Update], id: &str) -> String {
    refs.iter()
        .find(|u| u.id.split('/').nth(1) == Some(id))
        .map_or_else(|| id.to_string(), |u| u.name.clone())
}

async fn install(
    conn: Option<&Connection>,
    updates: Vec<Update>,
    poster: &Poster,
) -> Result<Restart, Failure> {
    let stages = stages(updates);
    let n = stages.len().max(1) as f64;
    let mut doing = String::from("Preparing to install");
    poster.post(Msg::Progress {
        fraction: None,
        doing: doing.clone(),
    });
    let mut done_stages = Vec::new();
    for (i, stage) in stages.into_iter().enumerate() {
        let mut report = |fraction: Option<f64>, line: Option<String>| {
            if let Some(l) = line {
                doing = l;
            }
            poster.post(Msg::Progress {
                fraction: fraction.map(|f| (i as f64 + f) / n),
                doing: doing.clone(),
            });
        };
        let done = match (stage, conn) {
            (Stage::Flatpak(at, refs), _) => {
                let ids: Vec<String> = refs.iter().map(|u| u.id.clone()).collect();
                flatpak::install(at, &ids, &mut |id, f| {
                    let line =
                        (!id.is_empty()).then(|| format!("Updating {}", ref_name(&refs, id)));
                    report(Some(f), line);
                })
                .map(|()| Restart::None)
                .map_err(Failure::Text)
            }
            (_, None) => Err(Failure::Text("There is no system bus.".into())),
            (Stage::Packages(ids), Some(c)) => {
                match packagekit::install(c, ids, &mut report).await {
                    Ok(r) => Ok(r),
                    Err(packagekit::Error::Unavailable) => {
                        Err(Failure::Text("PackageKit is not installed.".into()))
                    }
                    Err(packagekit::Error::Dismissed) => Err(Failure::Dismissed),
                    Err(packagekit::Error::Failed(e)) => Err(Failure::Text(e)),
                }
            }
            (Stage::Firmware(u), Some(c)) => {
                match fwupd::install(c, &u, &mut report, &mut |r| poster.post(Msg::Request(r)))
                    .await
                {
                    Ok(r) => Ok(r),
                    Err(fwupd::Error::Unavailable) => {
                        Err(Failure::Text("fwupd is not installed.".into()))
                    }
                    Err(fwupd::Error::Dismissed) => Err(Failure::Dismissed),
                    Err(fwupd::Error::Failed(e)) => Err(Failure::Text(e)),
                }
            }
        };
        if let Err(Failure::Text(e)) = &done {
            log::warn!("installing updates: {e}");
        }
        done_stages.push(done);
    }
    outcome(done_stages)
}

/// The stages' results as one: the first failure with something to tell, else the
/// restart the installed stages ask for, else (every prompt dismissed) a dismissal.
fn outcome(stages: Vec<Result<Restart, Failure>>) -> Result<Restart, Failure> {
    let mut restart = None;
    for done in stages {
        match done {
            Ok(r) => restart = Some(restart.unwrap_or(Restart::None).max(r)),
            Err(Failure::Dismissed) => {}
            Err(e) => return Err(e),
        }
    }
    restart.ok_or(Failure::Dismissed)
}

async fn watch(jobs: Jobs) {
    let conn = match Connection::system().await {
        Ok(c) => c,
        Err(_) => return,
    };
    futures_util::future::join(watch_packagekit(&conn, &jobs), watch_fwupd(&conn, &jobs)).await;
}

async fn watch_packagekit(conn: &Connection, jobs: &Jobs) {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(packagekit::NAME)
        .path(packagekit::PATH)
        .interface(packagekit::IFACE)
        .member("UpdatesChanged")
        .build()
        .map(Into::into);
    let mut signals = match rule {
        Ok(r) => reprisebus::signals(conn, packagekit::NAME, vec![r]).await,
        Err(e) => {
            log::warn!("no match rule for UpdatesChanged: {e}");
            return;
        }
    };
    loop {
        match signals.next().await {
            None => return,
            Some(reprisebus::Signal::OwnerChanged) => continue,
            Some(reprisebus::Signal::Message(_)) => {}
        }
        async_io::Timer::after(SETTLE).await;
        while let Some(Some(_)) = signals.next().now_or_never() {}
        jobs.send(Job::Check {
            refresh: false,
            manual: false,
        });
    }
}

async fn watch_fwupd(conn: &Connection, jobs: &Jobs) {
    let mut signals = match fwupd::change_rules() {
        Ok(rules) => reprisebus::signals(conn, fwupd::NAME, rules).await,
        Err(e) => {
            log::warn!("no match rules for fwupd changes: {e}");
            return;
        }
    };
    loop {
        match signals.next().await {
            None => return,
            Some(reprisebus::Signal::OwnerChanged) => continue,
            Some(reprisebus::Signal::Message(_)) => {}
        }
        let mut last = Instant::now();
        loop {
            let timer = async_io::Timer::at(last + SETTLE);
            futures_util::pin_mut!(timer);
            match futures_util::future::select(signals.next(), timer).await {
                futures_util::future::Either::Left((None, _)) => return,
                futures_util::future::Either::Left((Some(_), _)) => last = Instant::now(),
                futures_util::future::Either::Right(_) => break,
            }
        }
        jobs.send(Job::Changed(Group::Firmware, last));
    }
}

fn watch_flatpak(jobs: Jobs) {
    if !flatpak::present() {
        return;
    }
    let mut watch = match flatpak::Watch::new(flatpak::installations()) {
        Ok(w) => w,
        Err(e) => {
            log::warn!("could not watch the Flatpak installations: {e}");
            return;
        }
    };
    loop {
        match watch.next(SETTLE) {
            Ok(at) => jobs.send(Job::Changed(Group::Flatpak, at)),
            Err(e) => {
                log::warn!("the Flatpak watch stopped: {e}");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kind;

    #[test]
    fn a_dismissed_stage_is_silent_unless_every_stage_was_dismissed() {
        let text = || Err(Failure::Text("No network.".into()));
        assert_eq!(
            outcome(vec![Err(Failure::Dismissed), Ok(Restart::System)]),
            Ok(Restart::System)
        );
        assert_eq!(outcome(vec![Err(Failure::Dismissed), text()]), text());
        assert_eq!(outcome(vec![Ok(Restart::None), text()]), text());
        assert_eq!(
            outcome(vec![Err(Failure::Dismissed), Err(Failure::Dismissed)]),
            Err(Failure::Dismissed)
        );
    }

    #[test]
    fn waiting_checks_fold_into_one_after_the_installs() {
        let check = |refresh, manual| Job::Check { refresh, manual };
        let rest = vec![
            check(true, false),
            Job::Install(Vec::new()),
            check(false, true),
        ];
        let jobs = fold(check(false, false), rest.into_iter());
        assert_eq!(jobs.len(), 2);
        assert!(matches!(jobs[0], Job::Install(_)));
        assert!(matches!(
            jobs[1],
            Job::Check {
                refresh: true,
                manual: true
            }
        ));
    }

    #[test]
    fn a_change_is_checked_unless_a_check_read_its_source_after_it() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut read = Read::new();
        assert!(!seen(&read, Group::Flatpak, at(10)), "never read");
        read.insert(Group::Flatpak, at(12));
        read.insert(Group::Firmware, at(5));
        assert!(seen(&read, Group::Flatpak, at(10)));
        assert!(
            !seen(&read, Group::Firmware, at(10)),
            "read before the change"
        );
        assert!(
            seen(&read, Group::Flatpak, at(12) + HEARD_LATE / 2),
            "the check's own change, heard just after it read"
        );
        assert!(!seen(&read, Group::Flatpak, at(14)));

        let jobs = fold(
            Job::Changed(Group::Flatpak, at(1)),
            vec![
                Job::Install(Vec::new()),
                Job::Changed(Group::Firmware, at(2)),
            ]
            .into_iter(),
        );
        assert!(matches!(jobs[0], Job::Install(_)));
        assert!(matches!(jobs[1], Job::Changed(Group::Flatpak, _)));
        assert!(matches!(jobs[2], Job::Changed(Group::Firmware, _)));
    }

    fn update(id: &str, origin: Origin) -> Update {
        Update {
            id: id.into(),
            name: id.into(),
            current: String::new(),
            version: "2".into(),
            summary: String::new(),
            kind: Kind::Normal,
            origin,
        }
    }

    #[test]
    fn an_install_runs_one_stage_per_source_and_installation() {
        let s = stages(vec![
            update("bash", Origin::Package),
            update("app/a", Origin::App(Install::User)),
            update("cam", Origin::Firmware),
            update("runtime/p", Origin::Runtime(Install::System)),
            update("zlib", Origin::Package),
            update("runtime/p", Origin::Runtime(Install::User)),
            update("dock", Origin::Firmware),
        ]);
        let kinds: Vec<String> = s
            .iter()
            .map(|s| match s {
                Stage::Packages(ids) => format!("packages {}", ids.join(" ")),
                Stage::Flatpak(at, refs) => format!(
                    "{} {}",
                    at.label(),
                    refs.iter()
                        .map(|u| u.id.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
                Stage::Firmware(u) => format!("firmware {}", u.id),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "packages bash zlib",
                "system runtime/p",
                "user app/a runtime/p",
                "firmware cam",
                "firmware dock"
            ]
        );
    }
}
