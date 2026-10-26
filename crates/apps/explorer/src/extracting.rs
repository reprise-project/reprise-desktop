//! Extract Here and Extract to on an archive's menu: an [`ExtractJob`] with its progress dialog
//! and the Skip, Retry and Cancel box on an archive that fails.

use crate::Explorer;
use crate::copydlg::Progress;
use crate::place::Place;
use repriseapp::{Ctx, Reply};
use reprisefs::extract::{self, Archive, ExtractEvent, ExtractJob};
use reprisefs::listing::Item;
use reprisekit::controls::copyprogress::Kind;
use std::path::PathBuf;

/// The extraction running, with its dialog.
pub(crate) struct Extracting {
    pub job: ExtractJob,
    pub progress: Progress,
    /// The folders archives came out into so far.
    landed: Vec<PathBuf>,
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
        match self.extracting {
            Some(_) => self.extract_queue.push_back(batch),
            None => self.start_extract(batch, ctx),
        }
        Reply::none()
    }

    fn start_extract(&mut self, batch: Vec<Archive>, ctx: &Ctx) {
        let from = batch
            .first()
            .and_then(|a| a.src.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let to = batch
            .first()
            .and_then(|a| a.src.parent())
            .and_then(|d| d.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let wake = ctx.wake.clone();
        let job = ExtractJob::start(batch, move || wake.wake());
        let progress = Progress::new(job.meter().clone(), Kind::Extract, from, to);
        self.extracting = Some(Extracting {
            job,
            progress,
            landed: Vec::new(),
        });
    }

    /// Takes the extraction's events: a failed archive asks Skip, Retry or Cancel; the end lists
    /// the folder again with the new folders selected and starts the next batch.
    pub(crate) fn poll_extract(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let ex = self.extracting.as_mut()?;
        let mut ask = None;
        let mut finished = false;
        for ev in ex.job.poll() {
            match ev {
                ExtractEvent::Landed { into, .. } => ex.landed.push(into),
                ExtractEvent::Failed { text, .. } => ask = Some(text),
                ExtractEvent::Finished => finished = true,
            }
        }
        if let Some(text) = ask {
            let meter = ex.job.meter().clone();
            return Some(self.ask_failed(meter, extract::FAILURE_TITLE, text, ctx));
        }
        if !finished {
            return None;
        }
        let ex = self.extracting.take()?;
        ex.progress.ended(ex.landed.len(), None);
        self.publish_task_progress();
        let shown = self.place.path();
        let folders: Vec<PathBuf> = ex
            .landed
            .into_iter()
            .filter(|p| shown.as_deref() != Some(p))
            .collect();
        if let Some(dir) = folders.first().and_then(|f| f.parent()) {
            self.refill_tree_node(&Place::from_path(dir), ctx);
        }
        let reply = self.refresh(ctx);
        let here: Vec<PathBuf> = (folders.iter())
            .filter(|f| f.parent() == shown.as_deref())
            .cloned()
            .collect();
        if !here.is_empty() {
            self.select_paths(&here);
            self.refresh_chrome(ctx);
        }
        if let Some(next) = self.extract_queue.pop_front() {
            self.start_extract(next, ctx);
        }
        Some(reply)
    }

    /// Cancel on the dialog: the running extraction stops and the batches queued behind it
    /// never start.
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
