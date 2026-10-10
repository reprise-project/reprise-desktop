//! Work off the event loop: one thread runs checks and installs serially and
//! checks on a schedule; another checks on PackageKit's `UpdatesChanged`.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use futures_util::FutureExt;
use futures_util::stream::StreamExt;
use zbus::Connection;

use crate::model::{Found, Group, Install, Origin, Restart, Update};
use crate::{flatpak, fwupd, packagekit};

const FIRST_CHECK: Duration = Duration::from_secs(90);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const SETTLE: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub enum Job {
    /// Look for updates. `refresh` downloads package lists and due firmware
    /// metadata first; `manual` (Check Now) fetches all firmware metadata.
    Check {
        refresh: bool,
        manual: bool,
    },
    Install(Vec<Update>),
}

#[derive(Debug)]
pub enum Msg {
    Checking,
    Checked(Found),
    Progress {
        fraction: Option<f64>,
        doing: String,
    },
    Installed(Result<Restart, String>),
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
pub struct Jobs(Sender<Job>);

impl Jobs {
    pub fn send(&self, job: Job) {
        let _ = self.0.send(job);
    }
}

/// Start both threads; `check_now` runs the first check at once.
pub fn start(waker: repriseapp::Waker, check_now: bool) -> (Receiver<Msg>, Jobs) {
    let (tx, rx) = channel();
    let (jobs_tx, jobs_rx) = channel();
    let poster = Poster { tx, waker };
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
    (rx, jobs)
}

fn fold(first: Job, rest: impl Iterator<Item = Job>) -> Vec<Job> {
    let mut installs = Vec::new();
    let mut check: Option<(bool, bool)> = None;
    for job in std::iter::once(first).chain(rest) {
        match job {
            Job::Install(u) => installs.push(Job::Install(u)),
            Job::Check { refresh, manual } => {
                let (r, m) = check.unwrap_or((false, false));
                check = Some((r || refresh, m || manual));
            }
        }
    }
    installs.extend(check.map(|(refresh, manual)| Job::Check { refresh, manual }));
    installs
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
                    let found = zbus::block_on(check(conn.as_ref(), refresh, manual));
                    poster.post(Msg::Checked(found));
                }
                Job::Install(updates) => {
                    let done = zbus::block_on(install(conn.as_ref(), updates, &poster));
                    poster.post(Msg::Installed(done));
                    poster.post(Msg::Checking);
                    let found = zbus::block_on(check(conn.as_ref(), false, false));
                    poster.post(Msg::Checked(found));
                }
            }
        }
    }
}

async fn check(conn: Option<&Connection>, refresh: bool, manual: bool) -> Found {
    let mut found = Found::new();
    if let Some(c) = conn {
        match packagekit::updates(c, refresh).await {
            Ok(u) => found.push((Group::Packages, Ok(u))),
            Err(packagekit::Error::Unavailable) => {
                log::info!("PackageKit is not installed; no package updates");
            }
            Err(packagekit::Error::Failed(e)) => found.push((Group::Packages, Err(e))),
        }
    }
    if flatpak::present() {
        found.push((Group::Flatpak, flatpak::updates()));
    }
    if let Some(c) = conn {
        if refresh && let Err(fwupd::Error::Failed(e)) = fwupd::refresh(c, manual).await {
            log::info!("refreshing the firmware metadata: {e}");
        }
        match fwupd::updates(c).await {
            Ok(u) => found.push((Group::Firmware, Ok(u))),
            Err(fwupd::Error::Unavailable) => {
                log::info!("fwupd is not installed; no firmware updates");
            }
            Err(fwupd::Error::Failed(e)) => found.push((Group::Firmware, Err(e))),
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

fn ref_name(refs: &[Update], r: &str) -> String {
    refs.iter()
        .find(|u| u.id == r)
        .map(|u| u.name.clone())
        .unwrap_or_else(|| r.split('/').nth(1).unwrap_or(r).to_string())
}

async fn install(
    conn: Option<&Connection>,
    updates: Vec<Update>,
    poster: &Poster,
) -> Result<Restart, String> {
    let stages = stages(updates);
    let n = stages.len().max(1) as f64;
    let mut doing = String::from("Preparing to install");
    poster.post(Msg::Progress {
        fraction: None,
        doing: doing.clone(),
    });
    let mut restart = Restart::None;
    let mut failure: Option<String> = None;
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
                let total = ids.len() as f64;
                let mut seen = 0.0;
                flatpak::install(at, &ids, &mut |r| {
                    let f = (seen / total).min(0.95);
                    report(Some(f), Some(format!("Updating {}", ref_name(&refs, r))));
                    seen += 1.0;
                })
                .map(|()| Restart::None)
            }
            (_, None) => Err("There is no system bus.".into()),
            (Stage::Packages(ids), Some(c)) => {
                match packagekit::install(c, ids, &mut report).await {
                    Ok(r) => Ok(r),
                    Err(packagekit::Error::Unavailable) => {
                        Err("PackageKit is not installed.".into())
                    }
                    Err(packagekit::Error::Failed(e)) => Err(e),
                }
            }
            (Stage::Firmware(u), Some(c)) => match fwupd::install(c, &u, &mut report).await {
                Ok(r) => Ok(r),
                Err(fwupd::Error::Unavailable) => Err("fwupd is not installed.".into()),
                Err(fwupd::Error::Failed(e)) => Err(e),
            },
        };
        match done {
            Ok(r) => restart = restart.max(r),
            Err(e) => {
                log::warn!("installing updates: {e}");
                failure.get_or_insert(e);
            }
        }
    }
    match failure {
        Some(e) => Err(e),
        None => Ok(restart),
    }
}

async fn watch(jobs: Jobs) {
    let conn = match Connection::system().await {
        Ok(c) => c,
        Err(_) => return,
    };
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(packagekit::NAME)
        .and_then(|b| b.path(packagekit::PATH))
        .and_then(|b| b.interface(packagekit::IFACE))
        .and_then(|b| b.member("UpdatesChanged"))
        .map(|b| b.build().into());
    let mut signals = match rule {
        Ok(r) => reprisebus::signals(&conn, packagekit::NAME, vec![r]).await,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kind;

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
