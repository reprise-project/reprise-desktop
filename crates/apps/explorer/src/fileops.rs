//! The file operations behind Explorer's menus. The work is `reprisefs`'s; every operation ends in
//! a re-listing so view, status bar and Details box agree with disk.

use crate::dialogs::{ChooseProgram, ConfirmKind, Dialog, ReplaceAsk};
use crate::place::Place;
use crate::{Explorer, Focus};
use repriseapp::openwith::ChooserInput;
use repriseapp::{Ctx, FileClip, Reply};
use reprisefs::apps::{self, Launch};
use reprisefs::jobs::{PutEvent, PutItem, PutJob};
use reprisefs::listing::Item;
use reprisefs::names;
use reprisefs::ops::DropOp;
use reprisefs::replace::{self, Asker, Clash, Step, Verdict};
use reprisefs::sendto::{self, SendTo};
use reprisefs::undo::Undoable;
use reprisekit::controls::openwith::{self, Check, OpenWith, Program};
use reprisekit::controls::replacebox;
use reprisekit::raster::Canvas;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// An item picked in the Recycle Bin: its entry under `files/` and the record it was listed with;
/// Restore and Delete re-check the record ([`reprisefs::trash::check_listed`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinItem {
    pub path: PathBuf,
    pub record: Option<reprisefs::trash::Trashed>,
}

pub(crate) struct Put {
    job: PutJob,
    dir: PathBuf,
    ops: Vec<ItemOp>,
    batch: Vec<Undoable>,
    landed: Vec<PathBuf>,
    select_landed: bool,
}

pub(crate) type JobDone = Box<dyn FnOnce(&mut Explorer, &mut Ctx) -> Reply + Send>;

type ItemOp = (PathBuf, DropOp, String);

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
            Some(Verdict::Replace(seen)) => {
                it.replace = Some(*seen);
                true
            }
            _ => true,
        });
    }
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

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// A Delete's work on its job: each path trashed into `bins` or erased, stopping at the first
/// failure. The loop pushes trashed items as one undo and drops undos for items the bin's cap
/// erased.
fn delete_planned(bins: Option<&reprisefs::trash::Bins>, plan: Vec<(PathBuf, bool)>) -> JobDone {
    let mut batch = Vec::new();
    let mut purged = Vec::new();
    let mut failed = None;
    for (path, erase) in &plan {
        let done = if *erase {
            reprisefs::ops::erase(path).map(|()| None)
        } else {
            bins.ok_or_else(|| std::io::Error::other("cannot find the Recycle Bin"))
                .and_then(|b| b.trash(path))
                .map(|(t, gone)| {
                    purged.extend(gone);
                    Some(Undoable::Trash(t))
                })
        };
        match done {
            Ok(Some(u)) => batch.push(u),
            Ok(None) => {}
            Err(e) => {
                failed = Some(format!("Cannot delete {}: {e}", file_label(path)));
                break;
            }
        }
    }
    Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| {
        ex.undos.push_all(batch);
        ex.undos.forget_gone(&purged);
        ex.sync_undo();
        let reply = ex.refresh(ctx);
        match failed {
            Some(text) => ex.message_box("Error Deleting File or Folder", text, ctx),
            None => reply,
        }
    })
}

/// The terminal a `Terminal=true` program is wrapped in; a program cannot read the compositor's
/// config, so the environment decides.
fn terminal() -> String {
    std::env::var("TERMINAL").unwrap_or_else(|_| apps::DEFAULT_TERMINAL.to_string())
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
        self.paste_shortcut = shortcut;
        self.paste_dir = Some(dir);
        Reply {
            paste_files: true,
            ..Default::default()
        }
    }

    pub fn pasted(&mut self, clip: FileClip, ctx: &mut Ctx) -> Reply {
        let shortcut = std::mem::take(&mut self.paste_shortcut);
        let Some(dir) = self.paste_dir.take() else {
            return Reply::none();
        };
        if self.place.read_only() || self.place.path() != Some(dir.clone()) {
            log::info!("explorer: the paste's folder is gone, dropping it");
            return Reply::none();
        }
        if clip.paths.is_empty() {
            return Reply::none();
        }
        let cut = clip.cut;
        let plan = |src: &Path| plan_paste(cut, shortcut, src, &dir);
        self.put_all(&clip.paths, &dir, &plan, false, ctx)
    }

    /// Copy, move or link `paths` into `dir` on a `PutJob`; `plan` picks each op (`None` skips).
    /// One undo, re-list and error box per batch; a batch started while one runs waits in
    /// `put_queue`.
    pub(crate) fn put_all(
        &mut self,
        paths: &[PathBuf],
        dir: &Path,
        plan: &dyn Fn(&Path) -> Option<DropOp>,
        select_landed: bool,
        ctx: &mut Ctx,
    ) -> Reply {
        let mut items = Vec::new();
        for src in paths {
            let Some(op) = plan(src) else {
                continue;
            };
            let name = file_label(src);
            let icon = self.icon_name_for(src, ctx);
            let command = apps::open_command_for(src);
            items.push(PutItem {
                src: src.clone(),
                display_name: name.clone(),
                is_app: false,
                op,
                shortcut: Box::new(move |label| {
                    apps::launcher_text(label, Some(&icon), Launch::Exec(command.as_deref()?))
                }),
                replace: None,
            });
        }
        if items.is_empty() {
            return Reply::none();
        }
        let asker = Asker::new(replace::clashes(
            items.iter().map(|it| (it.src.as_path(), it.op, it.is_app)),
            dir,
        ));
        let pending = PendingPut {
            items,
            dir: dir.to_path_buf(),
            select_landed,
        };
        self.ask_replace(pending, asker, ctx)
    }

    pub(crate) fn ask_replace(
        &mut self,
        mut pending: PendingPut,
        asker: Asker,
        ctx: &mut Ctx,
    ) -> Reply {
        match asker.step() {
            Step::Cancelled => Reply::none(),
            Step::Ask { .. } if matches!(self.dialog, Some(Dialog::Replace(_))) => {
                self.replace_waiting.push_back((pending, asker));
                Reply::none()
            }
            Step::Ask { index, clash } => {
                if let Some(Dialog::Message { title, text }) = self.dialog.take() {
                    self.messages_waiting.push_back((title, text));
                }
                let many = pending.items.len() > 1;
                let src = pending.items[index].src.clone();
                let fields = self.replace_fields(&src, &pending.dir, clash, many, ctx);
                self.dialog = Some(Dialog::Replace(Box::new(ReplaceAsk {
                    fields,
                    asker,
                    pending,
                    focus: None,
                })));
                Reply::redraw_all()
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

    fn replace_fields(
        &self,
        src: &Path,
        dir: &Path,
        clash: Clash,
        many: bool,
        ctx: &mut Ctx,
    ) -> replacebox::Fields {
        let is_12h = reprisefs::locale::is_12h();
        let side = |path: &Path, ex: &Self, ctx: &mut Ctx| {
            let (size, modified) = replace::facts(path, is_12h);
            let icon = ex.icon_name_for(path, ctx);
            replacebox::Side {
                size,
                modified,
                icon: ctx.icons.get(&icon, 32),
            }
        };
        let subject = match clash {
            Clash::File => {
                let dest = dir.join(src.file_name().unwrap_or_default());
                replacebox::Subject::File {
                    existing: Box::new(side(&dest, self, ctx)),
                    incoming: Box::new(side(src, self, ctx)),
                }
            }
            Clash::Folder => replacebox::Subject::Folder,
        };
        replacebox::Fields {
            name: file_label(src),
            subject,
            icon: crate::chrome_icon(ctx, &["folder"], 32),
            many,
        }
    }

    fn start_put(&mut self, pending: PendingPut, ops: Vec<ItemOp>, ctx: &Ctx) {
        let wake = ctx.wake.clone();
        let job = PutJob::start(pending.items, pending.dir.clone(), move || wake.wake());
        self.put = Some(Put {
            job,
            dir: pending.dir,
            ops,
            batch: Vec::new(),
            landed: Vec::new(),
            select_landed: pending.select_landed,
        });
    }

    pub(crate) fn poll_put(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let put = self.put.as_mut()?;
        let mut failed = None;
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
                    failed = Some(op.failure(name, &error));
                }
                PutEvent::Finished => finished = true,
            }
        }
        if !finished {
            return Some(Reply::none());
        }
        let put = self.put.take()?;
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
        let mut reply = None;
        while let Ok(done) = self.jobs_rx.try_recv() {
            self.jobs_pending = self.jobs_pending.saturating_sub(1);
            reply = Some(done(self, ctx));
        }
        reply
    }

    pub(crate) fn busy(&self) -> bool {
        self.put.is_some() || self.jobs_pending > 0
    }

    /// File > Close or the close box. With work in flight the window hides and [`Explorer::wake`]
    /// quits once it drains.
    pub fn close(&mut self) -> Reply {
        self.flush_settings();
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
        while self.put.is_some() || self.jobs_pending > 0 || self.tip_walk_out() {
            std::thread::sleep(std::time::Duration::from_millis(2));
            if let Some(r) = self.poll_put(ctx) {
                reply = r;
            }
            if self.poll_tip_walk(ctx) {
                reply = Reply::redraw();
            }
            if let Some(r) = self.poll_jobs(ctx) {
                reply = r;
            }
        }
        reply
    }

    /// Delete (or Shift+Delete): a Confirm box per [`reprisefs::bincap::delete_mode`]. Items with
    /// no Recycle Bin, or already in one, are erased (the latter always asked about).
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
        let bins = reprisefs::trash::Bins::system();
        let fates: Vec<Fate> = paths
            .iter()
            .map(|p| bins.as_ref().map_or(Fate::NoBin, |b| b.fate(p)))
            .collect();
        let confirm = bins.as_ref().is_none_or(|b| b.policy.confirm);
        let mode = delete_mode(confirm, &fates, erase);
        if !in_bin && !mode.ask {
            return self.delete_now(&paths, mode.erase, ctx);
        }
        let no_bin = !in_bin && mode.no_bin;
        let erase = in_bin || mode.erase;
        let only = self.sv.selection.single().and_then(|i| self.items.get(i));
        let is_dir = only.is_some_and(|it| it.is_dir && !it.is_link);
        let name = only.map(|it| it.name.clone()).unwrap_or_default();
        let kind = ConfirmKind::of(paths.len(), is_dir);
        let icon = if erase {
            self.sv
                .selection
                .single()
                .and_then(|i| self.sv.entries.get(i))
                .and_then(|e| e.icon32.as_deref().cloned())
                .or_else(|| crate::chrome_icon(ctx, &["user-trash-full"], 32))
        } else {
            crate::chrome_icon(ctx, &["user-trash-full"], 32)
        };
        let bin = in_bin.then(|| self.selected_bin_items());
        let count = paths.len();
        let reply = self.confirm_box(kind, &name, paths, erase, icon, false, bin);
        if no_bin && let Some(Dialog::Confirm(c)) = self.dialog.as_mut() {
            c.text = format!("{} {}", reprisefs::trash::no_bin_lead(count), c.text);
        }
        reply
    }

    pub fn confirm_empty_bin(&mut self, ctx: &mut Ctx) -> Reply {
        if self.items.is_empty() {
            return Reply::none();
        }
        let only = (self.items.len() == 1).then(|| &self.items[0]);
        let kind = ConfirmKind::of(self.items.len(), only.is_some_and(|it| it.is_dir));
        let name = only.map(|it| it.name.clone()).unwrap_or_default();
        let icon = crate::chrome_icon(ctx, &["user-trash-full"], 32);
        self.confirm_box(kind, &name, Vec::new(), true, icon, true, None)
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
        self.spawn_job(ctx, move || {
            let emptied = bins.empty();
            Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| {
                if emptied.is_ok() {
                    ex.undos.retain_non_trash();
                    ex.sync_undo();
                }
                let reply = ex.refresh(ctx);
                match emptied {
                    Ok(n) => {
                        log::info!("explorer: emptied the Recycle Bin ({n} items)");
                        reply
                    }
                    Err(e) => ex.message_box(
                        "Error Deleting File or Folder",
                        format!("Cannot empty the Recycle Bin: {e}"),
                        ctx,
                    ),
                }
            })
        });
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
        if items.is_empty() {
            return Reply::none();
        }
        self.spawn_job(ctx, move || {
            let mut restored = Vec::new();
            let mut failed = None;
            for BinItem { path, record } in items {
                let back = reprisefs::trash::check_listed(&path, record.as_ref())
                    .and_then(|()| reprisefs::trash::trashed(&path))
                    .and_then(|t| reprisefs::trash::restore_free(&t));
                match back {
                    Ok(to) => {
                        log::info!("explorer: restored {}", to.display());
                        restored.push(path);
                    }
                    Err(e) => {
                        failed = Some(format!("Cannot restore {}: {e}", file_label(&path)));
                        break;
                    }
                }
            }
            Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| {
                ex.undos.forget_gone(&restored);
                ex.sync_undo();
                let reply = ex.refresh(ctx);
                match failed {
                    Some(text) => ex.message_box("Error Restoring File or Folder", text, ctx),
                    None => reply,
                }
            })
        });
        Reply::none()
    }

    /// Yes on the Confirm box: trash or remove every path on a job as one undo. Items too big for
    /// their Recycle Bin are asked about one at a time first ([`Explorer::too_big_step`]).
    pub(crate) fn delete_now(&mut self, paths: &[PathBuf], erase: bool, ctx: &mut Ctx) -> Reply {
        let paths = paths.to_vec();
        self.spawn_job(ctx, move || {
            let bins = reprisefs::trash::Bins::system();
            if !erase {
                let big: Vec<bool> = paths
                    .iter()
                    .map(|p| bins.as_ref().is_some_and(|b| b.too_big(p)))
                    .collect();
                if big.contains(&true) {
                    let asker = reprisefs::bincap::TooBig::new(paths, big);
                    return Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| {
                        ex.too_big_step(asker, ctx)
                    });
                }
            }
            let plan = paths.into_iter().map(|p| (p, erase)).collect();
            delete_planned(bins.as_ref(), plan)
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
                let is_dir = path.is_dir() && !path.is_symlink();
                let kind = ConfirmKind::of(1, is_dir);
                let text = reprisefs::bincap::too_big_text(&reprisefs::bincap::label(path));
                let icon = crate::chrome_icon(
                    ctx,
                    if is_dir {
                        &["folder"]
                    } else {
                        &["text-x-generic"]
                    },
                    32,
                );
                let reply = self.confirm_box(kind, "", Vec::new(), true, icon, false, None);
                if let Some(Dialog::Confirm(c)) = self.dialog.as_mut() {
                    c.text = text;
                    c.too_big = Some(asker);
                }
                reply
            }
            TooBigStep::Done(plan) => {
                self.spawn_job(ctx, move || {
                    let bins = reprisefs::trash::Bins::system();
                    delete_planned(bins.as_ref(), plan)
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
        self.spawn_job(ctx, move || {
            let mut erased = Vec::new();
            let mut failed = None;
            for BinItem { path, record } in items {
                match reprisefs::trash::erase_trashed(&path, record.as_ref()) {
                    Ok(()) => erased.push(path),
                    Err(e) => {
                        failed = Some(format!("Cannot delete {}: {e}", file_label(&path)));
                        break;
                    }
                }
            }
            Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| {
                ex.undos.forget_gone(&erased);
                ex.sync_undo();
                let reply = ex.refresh(ctx);
                match failed {
                    Some(text) => ex.message_box("Error Deleting File or Folder", text, ctx),
                    None => reply,
                }
            })
        });
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
        // The listing can be stale, so neither kind may land on an existing name: both fail with
        // `AlreadyExists`.
        let made = if folder {
            std::fs::create_dir(&path)
        } else {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map(drop)
        };
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
        self.send_to = sendto::session_rows(&reprisefs::places::mounted_drives());
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
                let existing = paths[0].parent().map(existing_names).unwrap_or_default();
                let spawn = sendto::zip_argv(&paths, &existing).into_iter().collect();
                return Reply {
                    spawn,
                    ..self.refresh(ctx)
                };
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
            SendTo::Folder(d) => (d, DropOp::Copy),
        };
        self.put_all(&paths, &dir, &|_| Some(op), false, ctx)
    }

    pub fn open_argv_for(&self, it: &Item) -> Vec<OsString> {
        match it.shortcut.as_deref() {
            Some(apps::Shortcut::App(app)) => apps::launch_argv(app, &terminal()),
            Some(apps::Shortcut::Link(link)) => vec!["xdg-open".into(), link.url.clone().into()],
            None => {
                reprisefs::props::open_argv_in(&it.path, &self.globs, &self.app_index, &terminal())
            }
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
            spawn: vec![reprisefs::apps::open_argv(app, &terminal(), &path)],
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
            spawn: vec![reprisefs::apps::open_argv(&app, &terminal(), &c.path)],
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
        let dirs = self.undos.undo_dirs();
        let Some(done) = self.undos.undo() else {
            return Reply::none();
        };
        self.sync_undo();
        for d in dirs {
            self.refill_tree_node(&Place::from_path(&d), ctx);
        }
        let reply = self.refresh(ctx);
        match done {
            Ok(()) => reply,
            Err(e) => self.message_box("Error", format!("Cannot undo: {e}"), ctx),
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

    fn icon_name_for(&self, path: &Path, ctx: &mut Ctx) -> String {
        if path.is_dir() {
            return "folder".to_string();
        }
        let mime = reprisefs::apps::mime_of_in(&self.globs, path);
        let has = |n: &str| ctx.icons.has(n);
        reprisefs::icons::pick_icon(mime.as_deref(), path, &has)
    }
}

#[cfg(test)]
#[path = "fileops_tests.rs"]
mod tests;
