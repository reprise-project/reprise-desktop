//! The file operations behind Explorer's menus. The work is `reprisefs`'s; every operation ends in
//! a re-listing so view, status bar and Details box agree with disk.

use crate::acting::{Action, Deadline};
use crate::dialogs::{ChooseProgram, ConfirmKind, Dialog, ReplaceAsk};
use crate::place::Place;
use crate::{Explorer, Focus};
use repriseapp::openwith::ChooserInput;
use repriseapp::{Ctx, FileClip, Reply};
use reprisefs::apps::{self, Launch};
use reprisefs::bincap::BinLack;
use reprisefs::deadline::Gate;
use reprisefs::jobs::{PutEvent, PutItem, PutJob};
use reprisefs::listing::Item;
use reprisefs::names;
use reprisefs::ops::DropOp;
use reprisefs::replace::{self, Answer, Asker, Clash, Step, Verdict};
use reprisefs::sendto::{self, SendTo};
use reprisefs::undo::Undoable;
use reprisekit::controls::openwith::{self, Check, OpenWith, Program};
use reprisekit::controls::replacebox;
use reprisekit::raster::Canvas;
use std::path::{Path, PathBuf};

/// An item picked in the Recycle Bin: its entry under `files/` and the record it was listed with;
/// Restore and Delete re-check the record ([`reprisefs::trash::check_listed`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinItem {
    pub path: PathBuf,
    pub record: Option<reprisefs::trash::Trashed>,
}

/// Where a paste goes.
#[derive(Clone, Debug)]
pub enum PasteTo {
    /// The place on view; dropped if the window has left it by the answer.
    Place { dir: PathBuf, shortcut: bool },
    /// A folder entry's Paste.
    Folder(PathBuf),
    /// A launcher's Paste: its program opens the clipboard's files.
    App(apps::AppEntry),
}

pub(crate) struct Put {
    pub(crate) job: PutJob,
    pub(crate) dir: PathBuf,
    pub(crate) ops: Vec<ItemOp>,
    batch: Vec<Undoable>,
    landed: Vec<PathBuf>,
    select_landed: bool,
    /// The last failure; `Failed` and `Finished` can arrive in different polls.
    failed: Option<(&'static str, String)>,
    pub(crate) progress: crate::copydlg::Progress,
}

pub(crate) type JobDone = Box<dyn FnOnce(&mut Explorer, &mut Ctx) -> Reply + Send>;

pub(crate) type ItemOp = (PathBuf, DropOp, String);

/// Wake the window once a job just started may have run long enough to show its dialog.
pub(crate) fn wake_to_show(ctx: &Ctx) {
    let wake = ctx.wake.clone();
    std::thread::spawn(move || {
        std::thread::sleep(reprisekit::controls::copyprogress::SHOW_AFTER);
        wake.wake();
    });
}

pub struct PendingPut {
    pub(crate) items: Vec<PutItem>,
    pub(crate) dir: PathBuf,
    select_landed: bool,
}

impl PendingPut {
    fn ops(&self) -> Vec<ItemOp> {
        self.items
            .iter()
            .map(|it| (it.src.clone(), it.op, it.display_name.clone()))
            .collect()
    }

    pub(crate) fn apply(&mut self, verdicts: &[Verdict]) {
        let mut v = verdicts.iter();
        self.items.retain_mut(|it| match v.next() {
            Some(Verdict::Skip) => false,
            Some(Verdict::Replace(r)) => {
                it.replace = Some(r.clone());
                true
            }
            _ => true,
        });
    }
}

/// A batch's items as (source, op, is a folder) and their clashes in the target folder, read on a
/// listing thread before the replace prompt.
pub struct Checked {
    puts: Vec<(PathBuf, DropOp, bool)>,
    clashes: Vec<Option<replace::Found>>,
}

/// What a paste does with one path, or `None` to ignore it (a cut file pasted back into its own
/// folder).
pub fn plan_paste(cut: bool, shortcut: bool, src: &Path, dir: &Path) -> Option<DropOp> {
    if shortcut {
        return Some(DropOp::Shortcut);
    }
    if !cut {
        return Some(DropOp::Copy);
    }
    (src.parent() != Some(dir)).then_some(DropOp::Move)
}

/// Display names already in `dir`, for "Copy of" and "Shortcut to" numbering; an unreadable folder
/// answers nothing since `unique_path` still refuses to clobber.
fn existing_names(dir: &Path) -> Vec<String> {
    match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(e) => {
            log::warn!("explorer: cannot list {}: {e}", dir.display());
            Vec::new()
        }
    }
}

pub(crate) fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// A Delete's work on its job: each path trashed into `bins` or erased, stopping at the first
/// failure. The loop pushes trashed items as one undo and drops undos for items the bin's cap
/// erased.
fn delete_planned(
    bins: Option<reprisefs::trash::Bins>,
    plan: Vec<(PathBuf, reprisefs::bincap::Plan)>,
    from: String,
) -> JobDone {
    Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| {
        let p = crate::removing::PendingRemoval::delete(plan, bins, from);
        ex.remove(p, ctx);
        Reply::none()
    })
}

impl Explorer {
    pub fn clipboard(&mut self, cut: bool, ctx: &mut Ctx) -> Reply {
        let paths = self.selected_paths();
        if paths.is_empty() || self.place == Place::RecycleBin {
            return Reply::none();
        }
        for (i, e) in self.sv.entries.iter_mut().enumerate() {
            e.cut = cut && self.sv.selection.set.contains(&i);
        }
        self.clipboard_paths(cut, paths, ctx)
    }

    pub fn clipboard_paths(&mut self, cut: bool, paths: Vec<PathBuf>, ctx: &mut Ctx) -> Reply {
        self.refresh_chrome(ctx);
        Reply {
            redraw: true,
            copy_files: Some(FileClip { cut, paths }),
            ..Default::default()
        }
    }

    pub fn begin_paste(&mut self, shortcut: bool) -> Reply {
        if self.place.read_only() {
            return Reply::none();
        }
        let Some(dir) = self.place.path() else {
            return Reply::none();
        };
        self.paste = Some(PasteTo::Place { dir, shortcut });
        Reply {
            paste_files: true,
            ..Default::default()
        }
    }

    /// Paste on the folder or launcher the item menu was opened on.
    pub fn begin_paste_on_item(&mut self) -> Reply {
        if self.menu_paste.is_none() {
            return Reply::none();
        }
        self.paste = self.menu_paste.clone();
        Reply {
            paste_files: true,
            ..Default::default()
        }
    }

    pub fn pasted(&mut self, clip: FileClip, ctx: &mut Ctx) -> Reply {
        let (dir, shortcut) = match self.paste.take() {
            None => return Reply::none(),
            Some(PasteTo::App(app)) => {
                let terminal = repriseconf::terminal();
                return Reply {
                    spawn: (clip.paths.iter())
                        .map(|p| reprisefs::apps::open_argv(&app, &terminal, p))
                        .collect(),
                    spawn_app: Some(app.id),
                    ..Default::default()
                };
            }
            Some(PasteTo::Folder(dir)) => (dir, false),
            Some(PasteTo::Place { dir, shortcut }) => {
                if self.place.read_only() || self.place.path() != Some(dir.clone()) {
                    log::info!("explorer: the paste's folder is gone, dropping it");
                    return Reply::none();
                }
                (dir, shortcut)
            }
        };
        if clip.paths.is_empty() {
            return Reply::none();
        }
        let cut = clip.cut;
        let plan = |src: &Path| plan_paste(cut, shortcut, src, &dir);
        self.put_all(&clip.paths, &dir, &plan, false, ctx)
    }

    /// Copy, move or link `paths` into `dir` on a `PutJob`; `plan` picks each op (`None` skips).
    /// One undo, re-list and error box per batch; a batch started while one runs waits in
    /// `put_queue`. Which items are folders and which names clash is read on a listing thread
    /// ([`Checked`]); a later answer (a dead mount) lands through [`Explorer::take_clashes`].
    pub(crate) fn put_all(
        &mut self,
        paths: &[PathBuf],
        dir: &Path,
        plan: &dyn Fn(&Path) -> Option<DropOp>,
        select_landed: bool,
        ctx: &mut Ctx,
    ) -> Reply {
        let puts: Vec<(PathBuf, DropOp)> = paths
            .iter()
            .filter_map(|src| Some((src.clone(), plan(src)?)))
            .collect();
        if puts.is_empty() {
            return Reply::none();
        }
        let title = puts[0].1.failure_title();
        let into = dir.to_path_buf();
        let read = move || Checked {
            clashes: replace::clashes(
                puts.iter().map(|(src, op)| (src.as_path(), *op, false)),
                &into,
            ),
            puts: puts
                .into_iter()
                .map(|(src, op)| {
                    let is_dir = src.is_dir();
                    (src, op, is_dir)
                })
                .collect(),
        };
        let to = (dir.to_path_buf(), select_landed);
        let job = if self.clashing.is_empty() {
            match self.read_job(ctx, read) {
                Ok(checked) => return self.checked_put(to, checked, ctx),
                Err(job) => job,
            }
        } else {
            // Behind a batch still being checked, so the prompts come in the order pasted.
            repriseapp::listjob::Job::spawn(Some(&ctx.wake), read)
        };
        self.clashing.push_back(Clashing {
            to,
            job,
            deadline: Deadline::start(self.action_deadline, ctx),
            title,
        });
        Reply::none()
    }

    /// Asks about the batches whose check ([`Explorer::put_all`]) answered late, in order; one
    /// past its deadline is dropped with an error box.
    pub(crate) fn take_clashes(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let mut reply = None;
        while let Some(front) = self.clashing.front() {
            if let Some(checked) = front.job.take() {
                let c = self.clashing.pop_front()?;
                self.checked_put(c.to, checked, ctx);
            } else if front.deadline.passed(std::time::Instant::now()) {
                let c = self.clashing.pop_front()?;
                log::warn!(
                    "explorer: a paste's check into {} gave up",
                    c.to.0.display()
                );
                self.message_box(
                    c.title,
                    reprisefs::deadline::not_responding(false, false),
                    ctx,
                );
            } else {
                break;
            }
            reply = Some(Reply::redraw_all());
        }
        reply
    }

    fn checked_put(
        &mut self,
        (dir, select_landed): (PathBuf, bool),
        c: Checked,
        ctx: &mut Ctx,
    ) -> Reply {
        let items = c
            .puts
            .into_iter()
            .map(|(src, op, is_dir)| {
                let icon = self.icon_name_as(&src, is_dir, ctx);
                let command = apps::open_command_as(&src, is_dir);
                PutItem {
                    display_name: file_label(&src),
                    src,
                    is_app: false,
                    op,
                    shortcut: Box::new(move |label| {
                        apps::launcher_text(label, Some(&icon), Launch::Exec(command.as_deref()?))
                    }),
                    replace: None,
                }
            })
            .collect();
        let pending = PendingPut {
            items,
            dir,
            select_landed,
        };
        self.ask_replace(pending, Asker::new(c.clashes), ctx)
    }

    /// Asks about the next clash of `pending`. A closed window has no one to ask, so it answers
    /// No to each: nothing is replaced and the rest is put.
    pub(crate) fn ask_replace(
        &mut self,
        mut pending: PendingPut,
        mut asker: Asker,
        ctx: &mut Ctx,
    ) -> Reply {
        while self.closing && matches!(asker.step(), Step::Ask { .. }) {
            asker.answer(Answer::No);
        }
        match asker.step() {
            Step::Cancelled => Reply::none(),
            Step::Ask { .. }
                if matches!(self.dialog, Some(Dialog::Replace(_))) || self.asking.is_some() =>
            {
                self.replace_waiting.push_back((pending, asker));
                Reply::none()
            }
            Step::Ask {
                index,
                clash,
                inside,
            } => {
                let many = pending.items.len() > 1 || inside.is_some();
                let item = &pending.items[index].src;
                let (src, dir) = match &inside {
                    Some(under) => {
                        let src = item.join(under);
                        let landing = pending.dir.join(item.file_name().unwrap_or_default());
                        let dir = landing.join(under.parent().unwrap_or(Path::new("")));
                        (src, dir)
                    }
                    None => (item.clone(), pending.dir.clone()),
                };
                let folder = inside
                    .as_deref()
                    .map(|under| replace::merge_folder(item, under));
                let ask = Asking {
                    pending,
                    asker,
                    src: src.clone(),
                    many,
                    folder,
                };
                let dest = dir.join(src.file_name().unwrap_or_default());
                let read = move || {
                    let is_12h = reprisefs::locale::is_12h();
                    (clash == Clash::File)
                        .then(|| [dest, src].map(|path| ReadSide::of(path, is_12h)))
                };
                match self.read_job(ctx, read) {
                    Ok(sides) => self.open_replace(ask, sides, ctx),
                    Err(job) => {
                        let deadline = Deadline::start(self.action_deadline, ctx);
                        self.asking = Some((ask, job, deadline));
                        Reply::none()
                    }
                }
            }
            Step::Done(verdicts) => {
                pending.apply(&verdicts);
                if !pending.items.is_empty() {
                    if self.put.is_some() {
                        self.put_queue.push_back(pending);
                    } else {
                        let ops = pending.ops();
                        self.start_put(pending, ops, ctx);
                    }
                }
                Reply::none()
            }
        }
    }

    /// Raises the replace prompt for `ask` on its sides as read ([`ReadSide`]); a message box up
    /// waits behind it.
    fn open_replace(&mut self, ask: Asking, sides: Option<[ReadSide; 2]>, ctx: &mut Ctx) -> Reply {
        if let Some(Dialog::Message { title, text }) = self.dialog.take() {
            self.messages_waiting.push_back((title, text));
        }
        let side = |s: ReadSide, ex: &Self, ctx: &mut Ctx| {
            let icon = ex.icon_name_as(&s.path, s.is_dir, ctx);
            replacebox::Side {
                size: s.size,
                modified: s.modified,
                icon: ctx.icons.get(&icon, 32),
            }
        };
        let subject = match sides {
            Some([existing, incoming]) => replacebox::Subject::File {
                existing: Box::new(side(existing, self, ctx)),
                incoming: Box::new(side(incoming, self, ctx)),
            },
            None => replacebox::Subject::Folder,
        };
        let fields = replacebox::Fields {
            name: file_label(&ask.src),
            folder: ask.folder,
            subject,
            icon: crate::chrome_icon(ctx, &["folder"], 32),
            many: ask.many,
        };
        self.dialog = Some(Dialog::Replace(Box::new(ReplaceAsk {
            fields,
            asker: ask.asker,
            pending: ask.pending,
            focus: None,
        })));
        Reply::redraw_all()
    }

    /// Raises the replace prompt whose sides ([`Explorer::ask_replace`]) were read late. A window
    /// closed meanwhile asks again, which answers No, and so do the pastes waiting behind it.
    pub(crate) fn take_asking(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let (ask, job, deadline) = self.asking.as_ref()?;
        let Some(sides) = job.take() else {
            if !deadline.passed(std::time::Instant::now()) {
                return None;
            }
            let title = (ask.pending.items.first()).map_or("Paste", |it| it.op.failure_title());
            log::warn!("explorer: a replace prompt's files gave up");
            self.asking = None;
            if self.closing {
                return Some(self.show_waiting(ctx));
            }
            let text = reprisefs::deadline::not_responding(false, false);
            return Some(self.message_box(title, text, ctx));
        };
        let (ask, ..) = self.asking.take()?;
        if self.closing {
            self.ask_replace(ask.pending, ask.asker, ctx);
            return Some(self.show_waiting(ctx));
        }
        Some(self.open_replace(ask, sides, ctx))
    }

    fn start_put(&mut self, pending: PendingPut, ops: Vec<ItemOp>, ctx: &Ctx) {
        use reprisekit::controls::copyprogress::Kind;
        let wake = ctx.wake.clone();
        let job = PutJob::start(pending.items, pending.dir.clone(), move || wake.wake());
        wake_to_show(ctx);
        let kind = if ops.iter().all(|(_, op, _)| *op == DropOp::Move) {
            Kind::Move
        } else {
            Kind::Copy
        };
        let from = ops
            .first()
            .and_then(|(src, _, _)| src.parent())
            .map(|p| Place::from_path(p).title())
            .unwrap_or_default();
        let to = Place::from_path(&pending.dir).title();
        let progress = crate::copydlg::Progress::new(job.meter().clone(), kind, from, to);
        self.put = Some(Put {
            job,
            dir: pending.dir,
            ops,
            batch: Vec::new(),
            landed: Vec::new(),
            select_landed: pending.select_landed,
            failed: None,
            progress,
        });
    }

    pub(crate) fn poll_put(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let put = self.put.as_mut()?;
        let mut finished = false;
        for ev in put.job.poll() {
            match ev {
                PutEvent::Landed {
                    index,
                    landed,
                    replaced,
                    unlisted,
                } => {
                    let (src, op, _) = &put.ops[index];
                    let dest = put.dir.join(&landed);
                    put.landed.push(dest.clone());
                    if replaced {
                        continue;
                    }
                    put.batch.push(match op {
                        DropOp::Move => Undoable::Move {
                            from: src.clone(),
                            to: dest,
                            listed: unlisted,
                        },
                        _ => Undoable::Copied(dest),
                    });
                }
                PutEvent::Failed { index, error } => {
                    let (_, op, name) = &put.ops[index];
                    put.failed = Some(op.failure(name, &error));
                }
                PutEvent::Finished => finished = true,
            }
        }
        if !finished {
            return Some(Reply::none());
        }
        let mut put = self.put.take()?;
        let failed = put.failed.take();
        put.progress.ended(put.landed.len(), failed.as_ref());
        self.publish_task_progress();
        self.undos.push_all(put.batch);
        self.sync_undo();
        if put.landed.iter().any(|p| p.is_dir()) {
            let mut dirs = vec![put.dir.clone()];
            for (src, op, _) in &put.ops {
                if *op == DropOp::Move
                    && let Some(parent) = src.parent()
                    && !dirs.iter().any(|d| d == parent)
                {
                    dirs.push(parent.to_path_buf());
                }
            }
            for d in dirs {
                self.refill_tree_node(&Place::from_path(&d), ctx);
            }
        }
        let mut reply = self.refresh(ctx);
        if put.select_landed
            && self.place.path().as_deref() == Some(&*put.dir)
            && !put.landed.is_empty()
        {
            self.select_paths(&put.landed);
            self.refresh_chrome(ctx);
        }
        if let Some((title, text)) = failed {
            reply = self.message_box(title, text, ctx);
        }
        if let Some(next) = self.put_queue.pop_front() {
            let ops = next.ops();
            self.start_put(next, ops, ctx);
        }
        Some(reply)
    }

    /// Run `work` on a thread; its result runs on the window at the next wake. Used for slow work
    /// (deletes, Empty, folder walks).
    pub(crate) fn spawn_job(&mut self, ctx: &Ctx, work: impl FnOnce() -> JobDone + Send + 'static) {
        let tx = self.jobs_tx.clone();
        let wake = ctx.wake.clone();
        self.jobs_pending += 1;
        let spawned = std::thread::Builder::new()
            .name("reprise-explorer-job".into())
            .spawn(move || {
                if tx.send(work()).is_ok() {
                    wake.wake();
                }
            });
        if let Err(e) = spawned {
            self.jobs_pending -= 1;
            log::warn!("explorer: cannot spawn a job thread ({e}); the work was dropped");
        }
    }

    pub(crate) fn poll_jobs(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let mut reply: Option<Reply> = None;
        while let Ok(done) = self.jobs_rx.try_recv() {
            self.jobs_pending = self.jobs_pending.saturating_sub(1);
            let r = done(self, ctx);
            reply = Some(match reply {
                Some(was) => was.and(r),
                None => r,
            });
        }
        reply
    }

    pub(crate) fn busy(&self) -> bool {
        self.put.is_some()
            || !self.clashing.is_empty()
            || self.removing.is_some()
            || self.jobs_pending > 0
            || !self.opening.is_empty()
            || self.asking.is_some()
            || !self.acting.is_empty()
            || !self.send.sends.is_empty()
    }

    /// File > Close or the close box. With work in flight the window hides and [`Explorer::wake`]
    /// quits once it drains. A Replace prompt still up (the taskbar's Close passes it) answers No,
    /// as do the pastes waiting behind it, and the rest is put.
    pub fn close(&mut self, ctx: &mut Ctx) -> Reply {
        self.flush_settings();
        if let Some(Dialog::Replace(r)) = self.dialog.take() {
            self.closing = true;
            self.ask_replace(r.pending, r.asker, ctx);
            self.show_waiting(ctx);
        }
        if self.busy() {
            self.closing = true;
            return Reply::none();
        }
        Reply {
            quit: true,
            ..Default::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn finish_puts(&mut self, ctx: &mut Ctx) -> Reply {
        let mut reply = Reply::none();
        while self.busy() || self.tip_walk_out() {
            std::thread::sleep(std::time::Duration::from_millis(2));
            if let Some(r) = self.poll_put(ctx) {
                reply = r;
            }
            if let Some(r) = self.poll_removal(ctx) {
                reply = r;
            }
            if self.poll_tip_walk(ctx) {
                reply = Reply::redraw();
            }
            if let Some(r) = self.poll_jobs(ctx) {
                reply = r;
            }
            if let Some(r) = self.take_acting(ctx) {
                reply = r;
            }
            if let Some(r) = self.take_send_reads(ctx) {
                reply = r;
            }
        }
        reply
    }

    /// Delete (or Shift+Delete): a Confirm box per [`reprisefs::bincap::delete_mode`]. Items with
    /// no Recycle Bin, or already in one, are erased (the latter always asked about). Each item's
    /// fate is read on a listing thread: it stats the item and its mount.
    pub fn confirm_delete(&mut self, erase: bool, ctx: &mut Ctx) -> Reply {
        use reprisefs::bincap::{Fate, delete_mode};
        let in_bin = self.place == Place::RecycleBin;
        if self.place.read_only() && !in_bin {
            return Reply::none();
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return Reply::none();
        }
        let only = self.sv.selection.single().and_then(|i| self.items.get(i));
        let is_dir = only.is_some_and(|it| it.is_dir && !it.is_link);
        let name = only.map(|it| it.name.clone()).unwrap_or_default();
        let kind = ConfirmKind::of(paths.len(), is_dir);
        let own_icon = self
            .sv
            .selection
            .single()
            .and_then(|i| self.sv.entries.get(i))
            .and_then(|e| e.icon32.as_deref().cloned());
        let bin = in_bin.then(|| self.selected_bin_items());
        let read = paths.clone();
        let work = move |_: &Gate| {
            let bins = reprisefs::trash::Bins::system();
            let fates: Vec<Fate> = (read.iter())
                .map(|p| bins.as_ref().map_or(Fate::NoBin, |b| b.fate(p)))
                .collect();
            (fates, bins.as_ref().is_none_or(|b| b.policy.confirm))
        };
        let action = Action::reads("Error Deleting File or Folder");
        self.act(ctx, action, work, move |ex, (fates, confirm), ctx| {
            let mode = delete_mode(confirm, &fates, erase);
            if !in_bin && !mode.ask {
                return ex.delete_now(&paths, mode.erase, ctx);
            }
            let no_bin = if in_bin { BinLack::None } else { mode.no_bin };
            let erase = in_bin || mode.erase;
            let icon = match erase {
                true => own_icon.or_else(|| crate::chrome_icon(ctx, &["user-trash-full"], 32)),
                false => crate::chrome_icon(ctx, &["user-trash-full"], 32),
            };
            ex.confirm_box(kind, &name, paths, erase, no_bin, icon, false, bin)
        })
    }

    pub fn confirm_empty_bin(&mut self, ctx: &mut Ctx) -> Reply {
        if self.items.is_empty() {
            return Reply::none();
        }
        let only = (self.items.len() == 1).then(|| &self.items[0]);
        let kind = ConfirmKind::of(self.items.len(), only.is_some_and(|it| it.is_dir));
        let name = only.map(|it| it.name.clone()).unwrap_or_default();
        let icon = crate::chrome_icon(ctx, &["user-trash-full"], 32);
        self.confirm_box(
            kind,
            &name,
            Vec::new(),
            true,
            BinLack::None,
            icon,
            true,
            None,
        )
    }

    /// Yes on that prompt: empty every trash. `Trash` undos go with it (their `files/` copies are
    /// gone); other undos stay. A failure leaves the stack as it was.
    pub(crate) fn empty_bin_now(&mut self, ctx: &mut Ctx) -> Reply {
        let Some(bins) = reprisefs::trash::Bins::system() else {
            return self.message_box(
                "Error Deleting File or Folder",
                "Cannot find the Recycle Bin.".to_string(),
                ctx,
            );
        };
        self.remove(crate::removing::PendingRemoval::empty(bins), ctx);
        Reply::none()
    }

    /// The Recycle Bin's Restore: the selection (or every item) back where `.trashinfo` says
    /// ([`reprisefs::trash::restore_free`]). Runs on a job; the first failure stops the rest.
    pub fn restore(&mut self, ctx: &mut Ctx) -> Reply {
        if self.place != Place::RecycleBin {
            return Reply::none();
        }
        let mut items = self.selected_bin_items();
        if items.is_empty() {
            items = (0..self.items.len())
                .filter_map(|i| self.bin_item(i))
                .collect();
        }
        self.restore_items(items, ctx)
    }

    pub(crate) fn restore_items(&mut self, items: Vec<BinItem>, ctx: &mut Ctx) -> Reply {
        self.remove(crate::removing::PendingRemoval::from_bin(items, true), ctx);
        Reply::none()
    }

    /// Yes on the Confirm box: trash or remove every path on a job as one undo. Items too big for
    /// their Recycle Bin are asked about one at a time first ([`Explorer::too_big_step`]).
    pub(crate) fn delete_now(&mut self, paths: &[PathBuf], erase: bool, ctx: &mut Ctx) -> Reply {
        let paths = paths.to_vec();
        let from = self.place.title();
        use reprisefs::bincap::{Plan, TooBig, TooBigStep};
        // The walk that measures a folder for its bin's cap takes as long as the folder is big, so
        // it runs on a job with no deadline.
        self.spawn_job(ctx, move || {
            let bins = reprisefs::trash::Bins::system();
            if erase {
                let plan = paths.into_iter().map(|p| (p, Plan::Erase)).collect();
                return delete_planned(bins, plan, from);
            }
            let measured = paths
                .iter()
                .map(|p| bins.as_ref().and_then(|b| b.measure(p)))
                .collect();
            let asker = TooBig::new(paths, measured);
            match asker.step() {
                TooBigStep::Done(plan) => delete_planned(bins, plan, from),
                _ => Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| ex.too_big_step(asker, ctx)),
            }
        });
        Reply::none()
    }

    pub(crate) fn too_big_step(
        &mut self,
        asker: reprisefs::bincap::TooBig,
        ctx: &mut Ctx,
    ) -> Reply {
        use reprisefs::bincap::TooBigStep;
        match asker.step() {
            TooBigStep::Ask(path) => {
                let listed = self.items.iter().find(|it| it.path == *path);
                let is_dir = listed.is_some_and(|it| it.is_dir && !it.is_link);
                let kind = ConfirmKind::of(1, is_dir);
                let text = reprisefs::bincap::too_big_text(&reprisefs::bincap::label(path));
                let icon = question_icon(ctx, is_dir);
                let reply =
                    self.confirm_box(kind, "", Vec::new(), true, BinLack::None, icon, false, None);
                if let Some(Dialog::Confirm(c)) = self.dialog.as_mut() {
                    c.text = text;
                    c.too_big = Some(asker);
                }
                reply
            }
            TooBigStep::Done(plan) => {
                let from = self.place.title();
                self.spawn_job(ctx, move || {
                    let bins = reprisefs::trash::Bins::system();
                    delete_planned(bins, plan, from)
                });
                Reply::none()
            }
            TooBigStep::Cancelled => Reply::none(),
        }
    }

    /// Yes on the Confirm box raised in the Recycle Bin: delete `items` for good on a job, each
    /// re-checked against its listing. The first failure stops the rest; what went has its Delete
    /// undo dropped.
    pub(crate) fn erase_from_bin(&mut self, items: Vec<BinItem>, ctx: &mut Ctx) -> Reply {
        self.remove(crate::removing::PendingRemoval::from_bin(items, false), ctx);
        Reply::none()
    }

    pub fn new_item(&mut self, folder: bool, ctx: &mut Ctx) -> Reply {
        let Some(dir) = self.place.path() else {
            return Reply::none();
        };
        if self.place.read_only() {
            return Reply::none();
        }
        let existing: Vec<String> = self.items.iter().map(|it| it.name.clone()).collect();
        let name = if folder {
            names::new_folder_name(&existing)
        } else {
            names::new_text_document_name(&existing)
        };
        let path = dir.join(&name);
        let title = if folder {
            "New Folder"
        } else {
            "New Text Document"
        };
        let made = path.clone();
        // The listing can be stale, so neither kind may land on an existing name: both fail with
        // `AlreadyExists`.
        let work = move |gate: &Gate| {
            if !gate.change(&dir) {
                return Ok(());
            }
            if folder {
                std::fs::create_dir(&made)
            } else {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&made)
                    .map(drop)
            }
        };
        self.act(ctx, Action::changes(title), work, move |ex, made, ctx| {
            ex.item_made(path, &name, made, ctx)
        })
    }

    fn item_made(
        &mut self,
        path: PathBuf,
        name: &str,
        made: std::io::Result<()>,
        ctx: &mut Ctx,
    ) -> Reply {
        if let Err(e) = made {
            return self.message_box(
                "Error Creating File or Folder",
                format!("Cannot create {name}: {e}"),
                ctx,
            );
        }
        self.undos.push(Undoable::New(path.clone()));
        self.sync_undo();
        let reply = self.refresh(ctx);
        if let Some(i) = self.index_of(&path) {
            self.sv.selection.set_only(Some(i));
            self.focus = Focus::View;
            self.begin_rename(ctx);
        }
        reply
    }

    pub fn create_shortcut(&mut self, ctx: &mut Ctx) -> Reply {
        let Some(dir) = self.place.path() else {
            return Reply::none();
        };
        let paths = self.selected_paths();
        if paths.is_empty() || self.place.read_only() {
            return Reply::none();
        }
        self.put_all(&paths, &dir, &|_| Some(DropOp::Shortcut), false, ctx)
    }

    /// Send To rows for the selection as (label, enabled, 16 px icon), cached so the menu's row
    /// index resolves to a destination.
    pub fn build_send_to(&mut self, ctx: &mut Ctx) -> Vec<(String, bool, Option<Canvas>)> {
        self.send_to = self.send_rows(ctx);
        self.send_to
            .iter()
            .map(|r| (r.label.clone(), r.enabled, ctx.icons.get(r.icon, 16)))
            .collect()
    }

    /// A read-only place sends nothing; the zip row writes beside the zipped file. Copies and
    /// desktop shortcuts go through [`Explorer::put_all`].
    pub fn send_to(&mut self, i: usize, ctx: &mut Ctx) -> Reply {
        if self.place.read_only() {
            return Reply::none();
        }
        let Some(dest) = self.send_to.get(i).map(|r| r.dest.clone()) else {
            return Reply::none();
        };
        let paths = self.selected_paths();
        if paths.is_empty() {
            return Reply::none();
        }
        let (dir, op) = match dest {
            // zip is bsdtar's work: one process for the whole selection, named after the focused
            // item.
            SendTo::Zip => {
                let mut paths = paths;
                let focus = self.sv.selection.focus.and_then(|i| self.items.get(i));
                if let Some(at) = focus.and_then(|f| paths.iter().position(|p| *p == f.path)) {
                    paths[..=at].rotate_right(1);
                }
                let dir = paths[0].parent().map(Path::to_path_buf);
                let work = move |_: &Gate| dir.as_deref().map(existing_names).unwrap_or_default();
                return self.act(
                    ctx,
                    Action::reads("Send To"),
                    work,
                    move |ex, existing, ctx| {
                        let spawn = sendto::zip_argv(&paths, &existing).into_iter().collect();
                        Reply {
                            spawn,
                            ..ex.refresh(ctx)
                        }
                    },
                );
            }
            SendTo::Mail => {
                return Reply {
                    spawn: vec![sendto::mail_argv(&paths)],
                    ..Reply::none()
                };
            }
            SendTo::DesktopShortcut => match reprisefs::places::user_dir("DESKTOP", "Desktop") {
                Some(d) => (d, DropOp::Shortcut),
                None => return Reply::none(),
            },
            SendTo::Folder(d) => return self.send_to_folder(paths, d, ctx),
        };
        self.put_all(&paths, &dir, &|_| Some(op), false, ctx)
    }

    pub fn open_for(&self, it: &Item) -> reprisefs::props::Opening {
        use reprisefs::props::Opening;
        match it.shortcut.as_deref() {
            Some(apps::Shortcut::App(app)) => Opening {
                argv: apps::launch_argv(app, &repriseconf::terminal()),
                app: Some(app.id.clone()),
            },
            Some(apps::Shortcut::Link(link)) => Opening {
                argv: vec!["xdg-open".into(), link.url.clone().into()],
                app: None,
            },
            None => reprisefs::props::open_in(
                &it.path,
                &self.globs,
                &self.app_index,
                &repriseconf::terminal(),
            ),
        }
    }

    pub fn build_open_with(&mut self, i: usize, ctx: &mut Ctx) -> Vec<(String, Option<Canvas>)> {
        self.open_with_path = self.items.get(i).map(|it| it.path.clone());
        self.open_with_apps = self
            .items
            .get(i)
            .and_then(|it| {
                it.mime
                    .clone()
                    .or_else(|| reprisefs::apps::mime_of_in(&self.globs, &it.path))
            })
            .map(|m| reprisefs::apps::open_with(&self.app_index, &m))
            .unwrap_or_default();
        let apps = self.open_with_apps.clone();
        apps.iter()
            .filter_map(|&a| self.app_index.get(a).cloned())
            .map(|app| {
                let icon = app.icon.as_deref().and_then(|n| ctx.icons.get(n, 16));
                (app.name.clone(), icon)
            })
            .collect()
    }

    pub fn open_with(&mut self, i: usize) -> Reply {
        let Some(path) = self.open_with_path.clone() else {
            return Reply::none();
        };
        let Some(app) = self
            .open_with_apps
            .get(i)
            .and_then(|&a| self.app_index.get(a))
        else {
            return Reply::none();
        };
        Reply {
            redraw: true,
            spawn: vec![reprisefs::apps::open_argv(
                app,
                &repriseconf::terminal(),
                &path,
            )],
            spawn_app: Some(app.id.clone()),
            ..Default::default()
        }
    }

    pub fn choose_program(&mut self, ctx: &mut Ctx) -> Reply {
        let Some(path) = self.open_with_path.clone() else {
            return Reply::none();
        };
        self.choose_program_for(&path, false, ctx)
    }

    pub(crate) fn choose_program_for(
        &mut self,
        path: &Path,
        set_default: bool,
        ctx: &mut Ctx,
    ) -> Reply {
        let path = path.to_path_buf();
        let mime = reprisefs::apps::mime_of_in(&self.globs, &path);
        let apps = &self.app_index;
        let default = mime
            .as_deref()
            .and_then(|m| reprisefs::props::default_app(m, apps));
        let (recommended, others) = reprisefs::apps::chooser_groups(apps, mime.as_deref(), default);
        let program = |i: usize| Program {
            id: apps[i].id.clone(),
            name: apps[i].name.clone(),
            icon: apps[i].icon.clone(),
        };
        let mut ow = OpenWith::new(
            &file_label(&path),
            recommended.into_iter().map(program).collect(),
            others.into_iter().map(program).collect(),
        );
        if let Some(d) = default {
            ow.select_id(&apps[d].id);
        }
        if set_default {
            ow.always = Check {
                on: true,
                enabled: false,
            };
        }
        let row_icons = crate::openwith::chooser_row_icons(&ow, &mut ctx.icons);
        let icon = ctx.icons.get(openwith::ICON, 32);
        self.dialog = Some(Dialog::ChooseProgram(Box::new(ChooseProgram {
            path,
            mime,
            ow,
            set_default,
            icon,
            row_icons,
            input: ChooserInput::opened(self.keyed),
            browse: None,
        })));
        Reply::redraw_all()
    }

    pub(crate) fn choose_program_ok(
        &mut self,
        c: ChooseProgram,
        id: String,
        always: bool,
        ctx: &mut Ctx,
    ) -> Reply {
        let found = self.app_index.iter().find(|a| a.id == id).cloned();
        let Some(app) = found.or_else(|| self.browsed_entry(&id, &c, always)) else {
            if c.set_default || !Path::new(&id).is_absolute() {
                return Reply::redraw_all();
            }
            return Reply {
                redraw_all: true,
                spawn: vec![vec![id.into(), c.path.into_os_string()]],
                ..Default::default()
            };
        };
        if always
            && let Some(mime) = &c.mime
            && let Err(e) = reprisefs::props::set_default_app(mime, &app.id)
        {
            log::warn!("explorer: cannot make {} the default: {e}", app.id);
        }
        if c.set_default {
            let icon = app.icon.as_deref().and_then(|n| ctx.icons.get(n, 16));
            self.props_set_opens_with(app.name.clone(), icon);
            return Reply::redraw_all();
        }
        Reply {
            redraw_all: true,
            spawn: vec![reprisefs::apps::open_argv(
                &app,
                &repriseconf::terminal(),
                &c.path,
            )],
            spawn_app: Some(app.id),
            ..Default::default()
        }
    }

    fn browsed_entry(
        &mut self,
        id: &str,
        c: &ChooseProgram,
        always: bool,
    ) -> Option<reprisefs::apps::AppEntry> {
        let mime = c.mime.as_deref().filter(|_| always || c.set_default)?;
        if !Path::new(id).is_absolute() {
            return None;
        }
        let entry = reprisefs::apps::program_entry(Path::new(id), Some(mime))
            .map_err(|e| log::warn!("explorer: no desktop entry for {id}: {e}"))
            .ok()?;
        match self.app_index.iter_mut().find(|a| a.id == entry.id) {
            Some(a) => *a = entry.clone(),
            None => self.app_index.push(entry.clone()),
        }
        Some(entry)
    }

    pub fn undo_last(&mut self, ctx: &mut Ctx) -> Reply {
        self.undo_with(false, ctx)
    }

    /// Undo, with `erase` removing for good what the Recycle Bin refuses. Without it such an item
    /// stops the undo and asks first.
    pub(crate) fn undo_with(&mut self, erase: bool, ctx: &mut Ctx) -> Reply {
        let dirs = self.undos.undo_dirs();
        let done = match erase {
            true => self.undos.undo_erasing(),
            false => self.undos.undo(),
        };
        let Some(done) = done else {
            return Reply::none();
        };
        self.sync_undo();
        for d in dirs {
            self.refill_tree_node(&Place::from_path(&d), ctx);
        }
        let reply = self.refresh(ctx);
        let Err(e) = done else {
            return reply;
        };
        match reprisefs::undo::NoBin::of(&e) {
            Some(no_bin) => {
                let many = self.undos.many();
                let is_dir = no_bin.path.is_dir() && !no_bin.path.is_symlink();
                let text = no_bin.question(many);
                let kind = ConfirmKind::of(if many { 2 } else { 1 }, is_dir);
                let icon = question_icon(ctx, is_dir);
                let reply =
                    self.confirm_box(kind, "", Vec::new(), true, BinLack::None, icon, false, None);
                if let Some(Dialog::Confirm(c)) = self.dialog.as_mut() {
                    c.text = text;
                    c.undo_erase = true;
                }
                reply
            }
            None => self.message_box("Error", format!("Cannot undo: {e}"), ctx),
        }
    }

    pub(crate) fn sync_undo(&mut self) {
        self.undo = self
            .undos
            .label()
            .map(|label| label.trim_start_matches("Undo "));
    }

    pub(crate) fn bin_item(&self, i: usize) -> Option<BinItem> {
        self.items.get(i).map(|it| BinItem {
            path: it.path.clone(),
            record: it.trashed.as_deref().cloned(),
        })
    }

    pub(crate) fn selected_bin_items(&self) -> Vec<BinItem> {
        self.sv
            .selection
            .set
            .iter()
            .filter_map(|&i| self.bin_item(i))
            .collect()
    }

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.sv
            .selection
            .set
            .iter()
            .filter_map(|&i| self.items.get(i).map(|it| it.path.clone()))
            .collect()
    }

    /// The icon of a file or folder already known to be one or the other; it reads nothing on disk.
    fn icon_name_as(&self, path: &Path, is_dir: bool, ctx: &mut Ctx) -> String {
        if is_dir {
            return "folder".to_string();
        }
        let mime = path
            .file_name()
            .and_then(|n| self.globs.mime_of_name(&n.to_string_lossy()));
        let has = |n: &str| ctx.icons.has(n);
        reprisefs::icons::pick_icon(mime.as_deref(), path, &has)
    }
}

/// A batch whose check ([`Checked`]) is still out: where it goes and whether what lands is
/// selected, and the title of the box that says it gave up.
pub(crate) struct Clashing {
    to: (PathBuf, bool),
    job: repriseapp::listjob::Job<Checked>,
    deadline: Deadline,
    title: &'static str,
}

/// A replace prompt whose sides are still being read, and when the read gives up.
pub(crate) type ReadingSides = (
    Asking,
    repriseapp::listjob::Job<Option<[ReadSide; 2]>>,
    Deadline,
);

/// A replace prompt waiting on its sides: the batch, its asker, the incoming file and what the
/// prompt says of it.
pub(crate) struct Asking {
    pending: PendingPut,
    asker: Asker,
    src: PathBuf,
    many: bool,
    folder: Option<String>,
}

/// One file of the replace prompt, read off the loop: its size and date lines and whether it is a folder.
pub(crate) struct ReadSide {
    path: PathBuf,
    size: String,
    modified: String,
    is_dir: bool,
}

impl ReadSide {
    fn of(path: PathBuf, is_12h: bool) -> ReadSide {
        let (size, modified) = replace::facts(&path, is_12h);
        ReadSide {
            is_dir: path.is_dir(),
            path,
            size,
            modified,
        }
    }
}

#[cfg(test)]
#[path = "fileops_tests.rs"]
mod tests;

/// The 32 px picture a delete question about one file or folder shows.
fn question_icon(ctx: &mut Ctx, is_dir: bool) -> Option<reprisekit::raster::Canvas> {
    let names: &[&str] = if is_dir {
        &["folder"]
    } else {
        &["text-x-generic"]
    };
    crate::chrome_icon(ctx, names, 32)
}
