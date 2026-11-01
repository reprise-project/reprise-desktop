//! Extract Here and Extract to on an archive's menu, and Compress to on any items: an
//! [`ArchiveJob`] with its progress dialog. An archive that fails asks Skip, Retry or Cancel; a zip
//! that fails shows an error box.

use crate::Explorer;
use crate::copydlg::Progress;
use crate::place::Place;
use repriseapp::{Ctx, Reply};
use reprisefs::compress::{self, Compress};
use reprisefs::extract::{self, Archive, ArchiveEvent, ArchiveJob};
use reprisefs::listing::Item;
use reprisekit::controls::copyprogress::Kind;
use std::path::PathBuf;

/// The extraction or compression running, with its dialog.
pub(crate) struct Extracting {
    pub job: ArchiveJob,
    pub progress: Progress,
    /// The folders archives came out into so far, or the zip.
    landed: Vec<PathBuf>,
}

/// One run of the job: archives to extract, or items to zip.
pub(crate) enum Batch {
    Extract(Vec<Archive>),
    Compress(Compress),
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
        for &i in &self.sv.selection.set {
            let it = self.items.get(i)?;
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
            _ => Some("Folders".into()),
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
            })
            .collect();
        self.queue_archive_job(Batch::Extract(batch), ctx);
        Reply::none()
    }

    /// The zip Compress to and Send To make of the selection: `name.zip`, numbered past a name
    /// the folder lists. None in a read-only place, or for items of several folders.
    pub(crate) fn compress_target(&self) -> Option<Compress> {
        if self.place.read_only() {
            return None;
        }
        let picked: Vec<&Item> = (self.sv.selection.set.iter())
            .filter_map(|&i| self.items.get(i))
            .collect();
        let dir = picked.first()?.path.parent()?;
        if picked.iter().any(|it| it.path.parent() != Some(dir)) {
            return None;
        }
        let paths: Vec<PathBuf> = picked.iter().map(|it| it.path.clone()).collect();
        let one_folder = matches!(picked.as_slice(), [it] if it.is_folder());
        let wanted = compress::zip_name(&paths, one_folder);
        let name = reprisefs::names::first_free(&wanted, |c| {
            (self.items.iter()).any(|it| it.path == dir.join(c))
        });
        Some(Compress {
            zip: dir.join(name),
            paths,
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

    fn queue_archive_job(&mut self, batch: Batch, ctx: &Ctx) {
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
        let (job, kind, from, to) = match batch {
            Batch::Extract(batch) => {
                let src = batch.first().map(|a| a.src.as_path());
                let (from, to) = (name(src), name(src.and_then(|s| s.parent())));
                (ArchiveJob::extract(batch, wake), Kind::Extract, from, to)
            }
            Batch::Compress(c) => {
                let from = name(c.zip.parent());
                let to = name(Some(&c.zip));
                (ArchiveJob::compress(c, wake), Kind::Compress, from, to)
            }
        };
        let progress = Progress::new(job.meter().clone(), kind, from, to);
        crate::fileops::wake_to_show(ctx);
        self.extracting = Some(Extracting {
            job,
            progress,
            landed: Vec::new(),
        });
    }

    /// Takes the job's events: a failed archive asks Skip, Retry or Cancel, a failed zip shows an
    /// error box; the end lists the folder again with the new folders or zip selected and starts
    /// the next batch.
    pub(crate) fn poll_extract(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let ex = self.extracting.as_mut()?;
        let zipping = ex.progress.kind == Kind::Compress;
        let mut ask = None;
        let mut finished = false;
        for ev in ex.job.poll() {
            match ev {
                ArchiveEvent::Landed { into, .. } => ex.landed.push(into),
                ArchiveEvent::Failed { text, .. } => ask = Some(text),
                ArchiveEvent::Finished => finished = true,
            }
        }
        if let Some(text) = ask.take_if(|_| !zipping) {
            let meter = ex.job.meter().clone();
            return Some(self.ask_failed(meter, extract::FAILURE_TITLE, text, ctx));
        }
        let failed = ask.map(|text| (compress::FAILURE_TITLE, text));
        let boxed = (failed.clone()).map(|(title, text)| self.message_box(title, text, ctx));
        if !finished {
            return boxed;
        }
        let ex = self.extracting.take()?;
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
            let m = ex.job.meter();
            m.set_paused(false);
            m.cancel();
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
