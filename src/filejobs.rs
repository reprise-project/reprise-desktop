//! The desktop's file jobs run in Explorer, each in a `reprise-explorer` process of its own that
//! asks, shows the progress dialog and says why it stopped: a drop's or paste's put (`--put`), a
//! delete (`--delete`), Empty Recycle Bin (`--empty-bin`) and Send To's zip (`--zip`).
//!
//! The desktop keeps the processes it started. A put onto the desktop names the icons it will
//! land; when the desktop's scan first finds one, it takes its cell (a put onto a cell) and is
//! selected: a scan's first landing in place of what was, the rest beside it. A landing leaves the
//! selection alone once the user changed it since the put started. A move's cut files leave the
//! clipboard once its process has ended, those it moved only.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use reprisefs::ops::DropOp;

use crate::compositor::State;

/// The program that runs the jobs.
const EXPLORER: &str = "reprise-explorer";
const EXPLORER_APP: reprisefs::apps::AppId = reprisefs::apps::AppId::from_static(EXPLORER);

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
    /// The desktop icons the job lands by name, each with its 100% cell (`desktop::to_saved`)
    /// when it was put onto one.
    landings: Vec<(String, Option<(i32, i32)>)>,
    /// One of `landings` has landed, so the next joins the selection.
    selected: bool,
    /// The desktop's selection, by name, when the put started.
    picked: BTreeSet<String>,
    /// A move's sources, handed to [`State::cut_moved`] once the process has ended.
    moving: Vec<PathBuf>,
    /// When the desktop first saw the process gone.
    ended: Option<Instant>,
}

#[derive(Default)]
pub struct FileJobs {
    pub jobs: Vec<Job>,
    /// The selection, by name, the last landing left.
    landed: Option<BTreeSet<String>>,
}

impl FileJobs {
    /// The jobs whose process still runs.
    fn running(&self) -> impl Iterator<Item = &Job> {
        (self.jobs.iter()).filter(|j| crate::spawn::running(j.pid))
    }
}

/// The name Explorer's put of `src` lands under in the desktop folder, numbered past `existing`;
/// None for a copy or move from elsewhere onto a taken name, which asks to replace it.
pub(crate) fn landing_name(
    src: &Path,
    op: DropOp,
    same_dir: bool,
    existing: &[String],
) -> Option<String> {
    let file = src.file_name()?.to_string_lossy().into_owned();
    match op {
        DropOp::Shortcut => Some(format!(
            "{}.desktop",
            reprisefs::names::shortcut_name(&file, existing)
        )),
        DropOp::Copy if same_dir => Some(reprisefs::names::copy_name(&file, existing)),
        DropOp::Copy | DropOp::Move => (!existing.contains(&file)).then_some(file),
    }
}

impl State {
    /// `sources` copied, moved or linked into `dir` by Explorer; `landings` are the desktop
    /// icons it will land and their cells.
    pub(crate) fn put_files(
        &mut self,
        dir: &Path,
        op: DropOp,
        sources: Vec<PathBuf>,
        landings: Vec<(String, Option<(i32, i32)>)>,
    ) {
        if sources.is_empty() {
            return;
        }
        let mut args: Vec<OsString> = ["--put", op.arg()].map(OsString::from).into();
        args.push(dir.into());
        args.push("--".into());
        args.extend(sources.iter().map(OsString::from));
        let moving = match op {
            DropOp::Move => sources,
            _ => Vec::new(),
        };
        self.file_job(What::Put(op), args, landings, moving);
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
        self.file_job(What::Delete, args, Vec::new(), Vec::new());
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
        self.file_job(
            What::Empty,
            vec!["--empty-bin".into()],
            Vec::new(),
            Vec::new(),
        );
    }

    /// Send To > Compressed (zipped) Folder on `paths` in Explorer.
    pub(crate) fn zip_files(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let args = std::iter::once("--zip".into())
            .chain(paths.into_iter().map(OsString::from))
            .collect();
        self.file_job(What::Zip, args, Vec::new(), Vec::new());
    }

    fn file_job(
        &mut self,
        what: What,
        args: Vec<OsString>,
        landings: Vec<(String, Option<(i32, i32)>)>,
        moving: Vec<PathBuf>,
    ) {
        // A put and a delete push their undo from the job's process.
        if what == What::Zip {
            self.forget_desktop_undos();
        }
        let argv: Vec<&std::ffi::OsStr> = std::iter::once(EXPLORER.as_ref())
            .chain(args.iter().map(|a| a.as_os_str()))
            .collect();
        let launch = self.new_launch();
        match crate::spawn::try_spawn_argv_in(
            &argv,
            Some(&EXPLORER_APP),
            None,
            &self.env,
            Some(&launch),
        ) {
            Ok(Some(pid)) => {
                let picked = (self.desktop.as_ref())
                    .filter(|_| !landings.is_empty())
                    .map(selected_names)
                    .unwrap_or_default();
                let job = Job {
                    pid,
                    what,
                    args,
                    landings,
                    selected: false,
                    picked,
                    moving,
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

    /// Moves whose process has ended hand their cut files to [`State::cut_moved`]: a file gone
    /// from where it was moved, and one still there (a cancelled or failed move) stays cut.
    pub(crate) fn file_jobs_ended(&mut self) {
        let moving: Vec<PathBuf> = (self.filejobs.jobs.iter_mut())
            .filter(|j| !crate::spawn::running(j.pid))
            .flat_map(|j| std::mem::take(&mut j.moving))
            .collect();
        if moving.is_empty() {
            return;
        }
        self.spawn_fs_job(move || {
            let moved: Vec<PathBuf> = (moving.into_iter())
                .filter(|p| std::fs::symlink_metadata(p).is_err())
                .collect();
            Box::new(move |state: &mut State| state.cut_moved(&moved))
        });
    }

    /// A desktop scan found `fresh`, names it did not list before: the jobs' icons among them
    /// take their cells and are selected while the selection is the one their put started with,
    /// or the one their last landing left. Jobs whose process ended a while ago are let go.
    pub(crate) fn land_job_icons(&mut self, fresh: &[String]) {
        self.file_jobs_ended();
        let shown = self
            .desktop
            .as_ref()
            .map(selected_names)
            .unwrap_or_default();
        let ours = self.filejobs.landed.as_ref() == Some(&shown);
        let mut landed = Vec::new();
        let now = Instant::now();
        for j in &mut self.filejobs.jobs {
            let first = !j.selected;
            let select = if first { j.picked == shown } else { ours };
            j.landings.retain(|(name, cell)| {
                let found = fresh.contains(name);
                if found {
                    landed.push((name.clone(), *cell, select.then_some(first)));
                    j.selected = true;
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
        let mut replaced = false;
        for (name, cell, select) in landed {
            if let Some(cell) = cell {
                self.land_icon(&name, cell);
            }
            let Some(view) = self.desktop.as_mut() else {
                continue;
            };
            let (Some(first), Some(i)) = (select, view.entries.iter().position(|e| e.name == name))
            else {
                continue;
            };
            if first && !replaced {
                view.selection.set_only(Some(i));
                replaced = true;
            } else {
                view.selection.set.insert(i);
            }
            self.filejobs.landed = Some(selected_names(view));
        }
    }
}

/// The names of the desktop's selected icons.
fn selected_names(view: &crate::desktopview::DesktopView) -> BTreeSet<String> {
    (view.selection.set.iter())
        .filter_map(|&i| view.entries.get(i))
        .map(|e| e.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_put_predicts_the_name_explorer_lands_the_icon_under() {
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
        assert_eq!(
            landing_name(src, DropOp::Copy, true, &taken).as_deref(),
            Some("Copy of a (2).txt"),
            "a copy beside its source numbered past a taken name"
        );
        assert_eq!(
            landing_name(src, DropOp::Copy, false, &["a.txt".into()]),
            None,
            "a copy onto a taken name asks to replace it"
        );
        let link = Path::new("/home/u/Desktop/notes.desktop");
        assert_eq!(
            landing_name(link, DropOp::Shortcut, true, &[]).as_deref(),
            Some("Shortcut to notes.desktop")
        );
    }
}
