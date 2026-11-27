//! Extract Here, Extract to and Extract All... on an archive's menu, Compress to on any items, and
//! Copy or Copy To Folder... in a compressed folder: an [`ArchiveJob`] with its progress dialog.
//! An archive that fails asks Skip, Retry or Cancel; a zip or a copy that fails shows an error box.
//! A copy comes out into a private scratch folder, then goes onto the clipboard or through the
//! put a paste makes, so it lands as any copied file does.

use crate::Explorer;
use crate::copydlg::Progress;
use crate::place::Place;
use repriseapp::{Ctx, Reply};
use reprisefs::compress::{self, Compress};
use reprisefs::extract::{self, Archive, ArchiveEvent, ArchiveJob};
use reprisefs::listing::Item;
use reprisefs::ops::DropOp;
use reprisekit::controls::copyprogress::Kind;
use reprisekit::tr;
use std::path::PathBuf;
use std::sync::Arc;

/// The extraction or compression running, with its dialog.
pub(crate) struct Extracting {
    pub job: ArchiveJob,
    pub progress: Progress,
    /// The folders archives came out into so far, or the zip.
    landed: Vec<PathBuf>,
    /// Extract All: the place shown when it started, which goes to the first folder that came
    /// out unless the window has moved on.
    show: Option<Place>,
    /// A compressed folder's copy: where its entries go once out, and the scratch folder they
    /// come out into.
    then: Option<Then>,
    pub scratch: Option<PathBuf>,
}

/// Where the entries a compressed folder's Copy, Copy To Folder... or drag copied out go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Then {
    Clipboard,
    Into(PathBuf),
    /// The drop that asked for them.
    Drag(repriseapp::Staged),
}

/// One run of the job: archives to extract, or items to zip.
pub(crate) enum Batch {
    Extract(Vec<Archive>),
    Compress(Compress),
    /// Entries of `archive` (paths inside it, from the listing `entries`) into the scratch
    /// folder `dir`.
    CopyOut {
        archive: PathBuf,
        entries: Arc<Vec<reprisefs::archive::Entry>>,
        picked: Vec<PathBuf>,
        dir: Option<PathBuf>,
        then: Then,
    },
}

/// What Extract to names after an archive, `name/`.
fn folder_label(it: &Item) -> String {
    let name = it.path.file_name().unwrap_or_default();
    format!("{}/", extract::folder_name(name).to_string_lossy())
}

impl Explorer {
    /// The selected items, when every one is an archive bsdtar extracts.
    fn selected_archives(&self) -> Option<Vec<&Item>> {
        let mut all = Vec::new();
        for (_, it) in self.picked() {
            let archive = !it.is_folder() && it.mime.as_deref().is_some_and(extract::is_archive);
            if !archive {
                return None;
            }
            all.push(it);
        }
        (!all.is_empty()).then_some(all)
    }

    /// The item menu's Extract to row says where: `name/` for one archive, else Folders.
    pub(crate) fn extract_label(&self) -> Option<String> {
        if self.place.read_only() {
            return None;
        }
        match self.selected_archives()?.as_slice() {
            [it] => Some(folder_label(it)),
            _ => Some(tr!("extracting-extract-label-folders")),
        }
    }

    /// Extract Here (`here`) or Extract to: the selected archives come out on a job, after the
    /// one running.
    pub(crate) fn extract(&mut self, here: bool, ctx: &mut Ctx) -> Reply {
        if self.place.read_only() {
            return Reply::none();
        }
        let Some(items) = self.selected_archives() else {
            return Reply::none();
        };
        let batch = items
            .iter()
            .map(|it| Archive {
                src: it.path.clone(),
                here,
                to: None,
            })
            .collect();
        self.queue_archive_job(Batch::Extract(batch), ctx);
        Reply::none()
    }

    /// Extract All...: the picker for where the selected archives, or the compressed folder
    /// shown, come out.
    pub(crate) fn extract_all(&mut self, ctx: &mut Ctx) -> Reply {
        let archives = match &self.place {
            Place::Archive(file, _) => vec![file.clone()],
            _ => (self.selected_archives().unwrap_or_default().iter())
                .map(|it| it.path.clone())
                .collect(),
        };
        self.browse_items(crate::browse::Pick::Extract, archives, ctx)
    }

    /// The picker's Extract: each archive comes out into a folder of its own in `dir`, after the
    /// job running, and the window then shows the first.
    pub(crate) fn extract_into(&mut self, archives: Vec<PathBuf>, dir: PathBuf, ctx: &Ctx) {
        let batch = (archives.into_iter())
            .map(|src| Archive {
                src,
                here: false,
                to: Some(dir.clone()),
            })
            .collect();
        self.queue_archive_job(Batch::Extract(batch), ctx);
    }

    /// The archive on view and the paths in it of the entries at `items`: what a Copy, Copy To
    /// Folder... or drag in a compressed folder copies out. None elsewhere, or for no entry.
    pub(crate) fn archive_picks(&self, items: &[PathBuf]) -> Option<(PathBuf, Vec<PathBuf>)> {
        let Place::Archive(archive, _) = &self.place else {
            return None;
        };
        let picked: Vec<PathBuf> = (items.iter())
            .filter_map(|p| Some(self.place.archive_entry(p)?.1))
            .collect();
        (!picked.is_empty()).then(|| (archive.clone(), picked))
    }

    /// The entries `picked` of `archive` ([`Explorer::archive_picks`]) come out on a job, after
    /// the one running, into a fresh scratch folder, and go where `then` says.
    pub(crate) fn copy_out(
        &mut self,
        (archive, picked): (PathBuf, Vec<PathBuf>),
        then: Then,
        ctx: &mut Ctx,
    ) -> Reply {
        let batch = Batch::CopyOut {
            entries: self.archives.kept(&archive).unwrap_or_default(),
            archive,
            picked,
            dir: reprisefs::archive::scratch_name(),
            then,
        };
        self.queue_archive_job(batch, ctx);
        Reply::none()
    }

    /// A box saying the drop of the entries of the archive `name` stopped waiting for them.
    pub(crate) fn drop_gave_up(&mut self, name: &str, ctx: &mut Ctx) -> Reply {
        let text = tr!("archive-drop-gave-up", name = name);
        self.message_box(&tr!("archive-copy-failed-title"), text, ctx)
    }

    /// Drags out of a compressed folder whose drop gave up while their copy waited its turn:
    /// each is dropped from the queue, with its box.
    pub(crate) fn take_abandoned_drags(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let mut names = Vec::new();
        self.extract_queue.retain(|b| match b {
            Batch::CopyOut {
                archive,
                then: Then::Drag(staged),
                ..
            } if staged.abandoned() => {
                names.push(
                    archive
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                );
                false
            }
            _ => true,
        });
        (names.into_iter())
            .map(|name| self.drop_gave_up(&name, ctx))
            .reduce(Reply::and)
    }

    /// Removes the scratch folders of finished Copy To Folder... copies once no put is running,
    /// queued, checked or asking: by then the put has read them.
    pub(crate) fn sweep_put_scratch(&mut self) {
        if self.put_scratch.is_empty() || !self.puts_idle() {
            return;
        }
        crate::fileops::remove_in_background(std::mem::take(&mut self.put_scratch));
    }

    /// The zip Compress to and Send To make of the selection: `name.zip`, numbered past a name
    /// the folder lists. None in a read-only place, or for items of several folders.
    pub(crate) fn compress_target(&self) -> Option<Compress> {
        if self.place.read_only() {
            return None;
        }
        let picked: Vec<&Item> = self.picked().map(|(_, it)| it).collect();
        let dir = picked.first()?.path.parent()?;
        let paths: Vec<PathBuf> = picked.iter().map(|it| it.path.clone()).collect();
        let one_folder = matches!(picked.as_slice(), [it] if it.is_folder());
        compress::target(&paths, one_folder, |c| {
            (self.items.iter()).any(|it| it.path == dir.join(c))
        })
    }

    /// The item menu's Compress to row names the zip.
    pub(crate) fn compress_label(&self) -> Option<String> {
        let c = self.compress_target()?;
        Some(c.zip.file_name()?.to_string_lossy().into_owned())
    }

    /// Compress to, or Send To > Compressed (zipped) Folder: the selection goes into a new zip
    /// on a job, after the one running.
    pub(crate) fn compress(&mut self, ctx: &mut Ctx) -> Reply {
        if let Some(c) = self.compress_target() {
            self.queue_archive_job(Batch::Compress(c), ctx);
        }
        Reply::none()
    }

    /// `--zip`: Send To > Compressed (zipped) Folder on `paths` with no folder window, the zip
    /// named past what their folder holds, read off the loop.
    pub(crate) fn zip_paths(&mut self, paths: Vec<PathBuf>, ctx: &mut Ctx) -> Reply {
        let work = move |_: &reprisefs::deadline::Gate| {
            let existing: Vec<std::ffi::OsString> =
                (std::fs::read_dir(paths.first()?.parent()?).ok()?.flatten())
                    .map(|e| e.file_name())
                    .collect();
            let c = compress::target(&paths, compress::one_folder(&paths), |c| {
                existing.iter().any(|e| e == c)
            });
            if c.is_none() {
                log::warn!("explorer: --zip makes no zip of {paths:?}");
            }
            c
        };
        let action = crate::acting::Action::reads(compress::failure_title());
        self.act(ctx, action, work, |ex, c, ctx| {
            if let Some(c) = c {
                ex.queue_archive_job(Batch::Compress(c), ctx);
            }
            Reply::none()
        })
    }

    pub(crate) fn queue_archive_job(&mut self, batch: Batch, ctx: &Ctx) {
        match self.extracting {
            Some(_) => self.extract_queue.push_back(batch),
            None => self.start_archive_job(batch, ctx),
        }
    }

    fn start_archive_job(&mut self, batch: Batch, ctx: &Ctx) {
        let name = |p: Option<&std::path::Path>| {
            p.and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let wake = ctx.wake.clone();
        let wake = move || wake.wake();
        let show = matches!(&batch, Batch::Extract(b) if b.iter().any(|a| a.to.is_some()))
            .then(|| self.place.clone());
        let (then, scratch) = match &batch {
            Batch::CopyOut { then, dir, .. } => (Some(then.clone()), dir.clone()),
            _ => (None, None),
        };
        let (job, kind, from, to) = match batch {
            Batch::Extract(batch) => {
                let first = batch.first();
                let src = first.map(|a| a.src.as_path());
                let dir = first.and_then(|a| a.to.as_deref().or(a.src.parent()));
                let (from, to) = (name(src), name(dir));
                (ArchiveJob::extract(batch, wake), Kind::Extract, from, to)
            }
            Batch::Compress(c) => {
                let from = name(c.zip.parent());
                let to = name(Some(&c.zip));
                (ArchiveJob::compress(c, wake), Kind::Compress, from, to)
            }
            Batch::CopyOut {
                archive,
                entries,
                picked,
                dir,
                then,
            } => {
                let from = name(Some(&archive));
                let (to, dest) = match then {
                    Then::Clipboard => (tr!("extracting-to-clipboard"), None),
                    Then::Drag(_) => (tr!("extracting-to-drop"), None),
                    Then::Into(dest) => (name(Some(&dest)), Some(dest)),
                };
                let job = reprisefs::archive::copy_job(archive, entries, picked, (dir, dest), wake);
                (job, Kind::Extract, from, to)
            }
        };
        let progress = Progress::new(job.meter().clone(), kind, from, to);
        crate::fileops::wake_to_show(ctx);
        self.extracting = Some(Extracting {
            job,
            progress,
            landed: Vec::new(),
            show,
            then,
            scratch,
        });
    }

    /// Takes the job's events: a failed archive asks Skip, Retry or Cancel while its job waits,
    /// and a failed zip or an ended job's failure shows an error box; the end lists the folder
    /// again with the new folders or zip selected and starts the next batch.
    pub(crate) fn poll_extract(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let ex = self.extracting.as_mut()?;
        if let Some(Then::Drag(staged)) = &ex.then
            && staged.abandoned()
        {
            ex.job.meter().cancel();
        }
        let zipping = ex.progress.kind == Kind::Compress;
        let copying = ex.then.is_some();
        let mut ask = None;
        let mut finished = false;
        for ev in ex.job.poll() {
            match ev {
                ArchiveEvent::Landed { into, .. } => ex.landed.push(into),
                ArchiveEvent::Failed { text, .. } => ask = Some(text),
                ArchiveEvent::Finished => finished = true,
            }
        }
        // A job that ended does not wait for an answer: its end tells the failure.
        if let Some(text) = ask.take_if(|_| !zipping && !copying && !finished) {
            let meter = ex.job.meter().clone();
            return Some(self.ask_failed(
                reprisefs::progress::Asked::new(meter),
                &extract::failure_title(),
                text,
                ctx,
            ));
        }
        let failed = ask.map(|text| match (zipping, copying) {
            (true, _) => (compress::failure_title(), text),
            (_, true) => {
                let name = ex.progress.from.clone();
                let text = tr!("archive-copy-out-failed", name = name, why = text);
                (tr!("archive-copy-failed-title"), text)
            }
            _ => (extract::failure_title(), text),
        });
        let boxed = (failed.clone()).map(|(title, text)| self.message_box(&title, text, ctx));
        if !finished {
            return boxed;
        }
        let ex = self.extracting.take()?;
        self.job_ended(ex.job.meter(), ctx);
        if let Some(then) = ex.then {
            ex.progress
                .ended(ex.job.meter().snapshot().items as usize, failed.as_ref());
            self.publish_task_progress();
            if let Some(next) = self.extract_queue.pop_front() {
                self.start_archive_job(next, ctx);
            }
            let out = failed.is_none() && !ex.job.meter().cancelled() && !ex.landed.is_empty();
            if !out {
                crate::fileops::remove_in_background(ex.scratch.iter().cloned().collect());
            }
            let landed = ex.landed;
            let reply = match then {
                Then::Drag(staged) => {
                    staged.done(out);
                    match staged.abandoned() {
                        true => self.drop_gave_up(&ex.progress.from, ctx),
                        false => Reply::none(),
                    }
                }
                _ if !out => Reply::none(),
                Then::Clipboard => {
                    let reply = self.clipboard_paths(false, landed, ctx);
                    self.clip_scratch = ex.scratch;
                    reply
                }
                Then::Into(dir) => {
                    let reply = self.put_all(&landed, &dir, &|_| Some(DropOp::Copy), true, ctx);
                    self.put_scratch.extend(ex.scratch);
                    reply
                }
            };
            return Some(match boxed {
                Some(b) => reply.and(b),
                None => reply,
            });
        }
        let shown_after = (ex.show.as_ref() == Some(&self.place))
            .then(|| ex.landed.first().cloned())
            .flatten();
        let done = match zipping {
            true => ex.job.meter().snapshot().items as usize,
            false => ex.landed.len(),
        };
        ex.progress.ended(done, failed.as_ref());
        self.publish_task_progress();
        let shown = self.place.path();
        let landed: Vec<PathBuf> = ex
            .landed
            .into_iter()
            .filter(|p| shown.as_deref() != Some(p))
            .collect();
        if !zipping && let Some(dir) = landed.first().and_then(|f| f.parent()) {
            self.refill_tree_node(&Place::from_path(dir), ctx);
        }
        let reply = self.refresh(ctx);
        let here: Vec<PathBuf> = (landed.iter())
            .filter(|f| f.parent() == shown.as_deref())
            .cloned()
            .collect();
        if !here.is_empty() {
            self.select_paths(&here);
            self.refresh_chrome(ctx);
        }
        if let Some(next) = self.extract_queue.pop_front() {
            self.start_archive_job(next, ctx);
        }
        let reply = match shown_after {
            Some(dir) => reply.and(self.go(Place::from_path(&dir), ctx)),
            None => reply,
        };
        Some(match boxed {
            Some(b) => reply.and(b),
            None => reply,
        })
    }

    /// Cancel on the dialog: the running job stops and the batches queued behind it never
    /// start.
    pub(crate) fn cancel_extracts(&mut self) {
        self.extract_queue.clear();
        if let Some(ex) = &self.extracting {
            ex.job.meter().cancel();
        }
    }

    /// Waits for the extraction and the batches queued behind it, answering a failed archive's
    /// box with `d`.
    #[cfg(test)]
    pub(crate) fn finish_extracts(&mut self, d: reprisefs::progress::Decision, ctx: &mut Ctx) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while self.extracting.is_some() {
            assert!(std::time::Instant::now() < deadline, "the extraction ends");
            if self.holds_a_job() {
                self.answer_failed(d, ctx);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            let _ = self.poll_extract(ctx);
        }
    }
}
