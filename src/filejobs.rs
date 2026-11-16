//! The desktop's file jobs run in Explorer, each in a `reprise-explorer` process of its own that
//! asks, shows the progress dialog and says why it stopped: a drop's or paste's put (`--put`), a
//! delete (`--delete`), Empty Recycle Bin (`--empty-bin`) and Send To's zip (`--zip`).
//!
//! The desktop keeps the processes it started. A session end leaves their windows open, waits for
//! them and lists them as one Files row; End Now ends them. A put onto a desktop cell names the
//! icons it will land, and each takes its cell when the desktop's scan first finds it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use reprisefs::ops::DropOp;

use crate::compositor::State;

/// The program that runs the jobs.
const EXPLORER: &str = "reprise-explorer";

/// How long a job's icons may still land once its process has ended: the desktop's scan that
/// finds the last of them can come after the exit.
const LAND_AFTER_END: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    Put(DropOp),
    Delete,
    Empty,
    Zip,
}

impl What {
    /// What the session-end prompt's Files row says the job is doing.
    fn reason(self) -> &'static str {
        match self {
            What::Put(DropOp::Move) => "Moving files",
            What::Put(DropOp::Copy) => "Copying files",
            What::Put(DropOp::Shortcut) => "Creating shortcuts",
            What::Delete => "Deleting files",
            What::Empty => "Emptying the Recycle Bin",
            What::Zip => "Compressing files",
        }
    }

    /// The title of the box that says the job did not start.
    fn failure_title(self) -> &'static str {
        match self {
            What::Put(op) => op.failure_title(),
            What::Delete | What::Empty => "Error Deleting File or Folder",
            What::Zip => reprisefs::compress::FAILURE_TITLE,
        }
    }
}

pub struct Job {
    pub pid: u32,
    pub what: What,
    /// The command line after the program's name.
    pub args: Vec<OsString>,
    /// The desktop icons the job lands by name, each with its 100% cell (`desktop::to_saved`).
    landings: Vec<(String, (i32, i32))>,
    /// When the desktop first saw the process gone.
    ended: Option<Instant>,
}

#[derive(Default)]
pub struct FileJobs {
    pub jobs: Vec<Job>,
}

impl FileJobs {
    /// The jobs whose process still runs.
    fn running(&self) -> impl Iterator<Item = &Job> {
        (self.jobs.iter()).filter(|j| crate::spawn::running(j.pid))
    }

    /// Whether `pid` is a job's process still running.
    pub fn holds(&self, pid: Option<i32>) -> bool {
        pid.is_some_and(|pid| self.running().any(|j| j.pid as i32 == pid))
    }

    /// The pids of the jobs still running.
    pub fn pids(&self) -> Vec<i32> {
        self.running().map(|j| j.pid as i32).collect()
    }
}

/// The name a put of `src` into the desktop folder lands under when nothing in `existing` (the
/// desktop's file names) holds it, as Explorer's put names it; None when that name is taken, since
/// the put then numbers past it.
pub(crate) fn landing_name(
    src: &Path,
    op: DropOp,
    same_dir: bool,
    existing: &[String],
) -> Option<String> {
    let file = src.file_name()?.to_string_lossy().into_owned();
    let name = match op {
        DropOp::Shortcut => {
            let base = file.strip_suffix(".desktop").unwrap_or(&file);
            format!("Shortcut to {base}.desktop")
        }
        DropOp::Copy if same_dir => format!("Copy of {file}"),
        DropOp::Copy | DropOp::Move => file,
    };
    (!existing.contains(&name)).then_some(name)
}

impl State {
    /// `sources` copied, moved or linked into `dir` by Explorer; `landings` are the desktop
    /// icons it will land and their cells.
    pub(crate) fn put_files(
        &mut self,
        dir: &Path,
        op: DropOp,
        sources: Vec<PathBuf>,
        landings: Vec<(String, (i32, i32))>,
    ) {
        if sources.is_empty() {
            return;
        }
        let mut args: Vec<OsString> = ["--put", op.arg()].map(OsString::from).into();
        args.push(dir.into());
        args.push("--".into());
        args.extend(sources.into_iter().map(OsString::from));
        self.file_job(What::Put(op), args, landings);
    }

    /// Delete on `paths` in Explorer, asked first as the Recycle Bin settings say; Shift+Delete
    /// with `erase`.
    pub(crate) fn delete_files(&mut self, paths: Vec<PathBuf>, erase: bool) {
        if paths.is_empty() {
            return;
        }
        let mut args = vec![OsString::from("--delete")];
        if erase {
            args.push("--erase".into());
        }
        args.extend(paths.into_iter().map(OsString::from));
        self.file_job(What::Delete, args, Vec::new());
    }

    /// Empty Recycle Bin in Explorer, which asks first. While one runs, its window comes to the
    /// front instead.
    pub fn empty_bin(&mut self) {
        let running = self.filejobs.running().find(|j| j.what == What::Empty);
        if let Some(pid) = running.map(|j| j.pid as i32) {
            let shown = (self.windows.iter())
                .filter(|w| w.mapped)
                .map(|w| w.id)
                .find(|&w| self.client_pid(w) == Some(pid));
            if let Some(w) = shown {
                self.bring_up(w);
            }
            return;
        }
        self.file_job(What::Empty, vec!["--empty-bin".into()], Vec::new());
    }

    /// Send To > Compressed (zipped) Folder on `paths` in Explorer.
    pub(crate) fn zip_files(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let args = std::iter::once("--zip".into())
            .chain(paths.into_iter().map(OsString::from))
            .collect();
        self.file_job(What::Zip, args, Vec::new());
    }

    fn file_job(&mut self, what: What, args: Vec<OsString>, landings: Vec<(String, (i32, i32))>) {
        let argv: Vec<&std::ffi::OsStr> = std::iter::once(EXPLORER.as_ref())
            .chain(args.iter().map(|a| a.as_os_str()))
            .collect();
        let launch = self.new_launch();
        match crate::spawn::try_spawn_argv_in(&argv, Some(EXPLORER), None, &self.env, Some(&launch))
        {
            Ok(Some(pid)) => {
                let job = Job {
                    pid,
                    what,
                    args,
                    landings,
                    ended: None,
                };
                log::info!("desktop: file job in pid {pid}: {:?}", job.args);
                self.filejobs.jobs.push(job);
            }
            Ok(None) => {}
            Err(e) => {
                log::error!("desktop: {argv:?}: {e}");
                let text = format!("Cannot start Explorer: {e}");
                self.open_dialog(crate::dialog::Dialog::error(what.failure_title(), text));
            }
        }
    }

    /// What a desktop file job under way does, for the session-end prompt; None when none runs.
    pub(crate) fn files_under_way(&self) -> Option<&'static str> {
        self.filejobs.running().next().map(|j| j.what.reason())
    }

    /// A desktop scan found `fresh`, names it did not list before: the jobs' icons among them
    /// take their cells. Jobs whose process ended a while ago are let go.
    pub(crate) fn land_job_icons(&mut self, fresh: &[String]) {
        let mut landed = Vec::new();
        let now = Instant::now();
        for j in &mut self.filejobs.jobs {
            j.landings.retain(|(name, cell)| {
                let found = fresh.contains(name);
                if found {
                    landed.push((name.clone(), *cell));
                }
                !found
            });
            if j.ended.is_none() && !crate::spawn::running(j.pid) {
                j.ended = Some(now);
            }
        }
        (self.filejobs.jobs).retain(|j| {
            j.ended
                .is_none_or(|t| now.duration_since(t) < LAND_AFTER_END)
        });
        for (name, cell) in landed {
            self.land_icon(&name, cell);
        }
    }
}

#[cfg(test)]
impl FileJobs {
    /// A job of `what` in process `pid`, as if the desktop had started it.
    pub fn started(&mut self, pid: u32, what: What) {
        self.jobs.push(Job {
            pid,
            what,
            args: Vec::new(),
            landings: Vec::new(),
            ended: None,
        });
    }
}

/// A process standing in for a job's: a `sleep` the spawner reaps, killed on drop.
#[cfg(test)]
pub struct FakeJob(std::process::Child);

#[cfg(test)]
impl FakeJob {
    pub fn start() -> FakeJob {
        let child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("sleep");
        crate::spawn::adopt(child.id() as libc::pid_t, "sleep");
        FakeJob(child)
    }

    pub fn pid(&self) -> u32 {
        self.0.id()
    }
}

#[cfg(test)]
impl Drop for FakeJob {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        crate::spawn::reap_children();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_put_names_the_icon_it_lands_unless_the_desktop_holds_that_name() {
        let src = Path::new("/home/u/Documents/a.txt");
        let taken = ["Copy of a.txt".to_string()];
        assert_eq!(
            landing_name(src, DropOp::Move, false, &[]).as_deref(),
            Some("a.txt")
        );
        assert_eq!(
            landing_name(src, DropOp::Copy, true, &[]).as_deref(),
            Some("Copy of a.txt"),
            "a copy beside its source"
        );
        assert_eq!(landing_name(src, DropOp::Copy, true, &taken), None);
        let link = Path::new("/home/u/Desktop/notes.desktop");
        assert_eq!(
            landing_name(link, DropOp::Shortcut, true, &[]).as_deref(),
            Some("Shortcut to notes.desktop")
        );
    }
}
