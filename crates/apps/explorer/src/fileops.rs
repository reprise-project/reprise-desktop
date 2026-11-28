//! The file operations behind Explorer's menus. The work is `reprisefs`'s; every operation ends in
//! a re-listing so view, status bar and Details box agree with disk.

use crate::acting::{Action, Deadline};
use crate::dialogs::{ChooseProgram, ConfirmKind, Dialog, ReplaceAsk, SheetId, TRASH_ICON};
use crate::icons::IconSlot;
use crate::place::Place;
use crate::{Explorer, Focus};
use repriseapp::iconjobs::IconWaits;
use repriseapp::openwith::ChooserInput;
use repriseapp::{Ctx, FileClip, Reply};
use reprisefs::apps::{self, Launch};
use reprisefs::bincap::BinLack;
use reprisefs::deadline::Gate;
use reprisefs::jobs::{PutEvent, PutItem, PutJob};
use reprisefs::names;
use reprisefs::ops::DropOp;
use reprisefs::replace::{self, Answer, Asker, Clash, Step, Verdict};
use reprisefs::sendto::{self, SendTo};
use reprisefs::stillfile;
use reprisefs::undo::{Hold, Undoable};
use reprisekit::controls::menu::{ContextBitmaps, MenuStyle};
use reprisekit::controls::openwith::{Check, OpenWith, Program};
use reprisekit::controls::replacebox;
use reprisekit::raster::Canvas;
use reprisekit::theme::Theme;
use reprisekit::tr;
use std::path::{Path, PathBuf};

/// The icon size of the Open With and Send To menus' rows: the menu's icon column at the text size.
pub(crate) fn menu_icon_px(theme: &Theme) -> i32 {
    MenuStyle::context(theme, ContextBitmaps::Icons).icon_px()
}

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
    App(Box<apps::AppEntry>),
    /// A zip's Paste, or a Paste in a compressed folder of one: the clipboard's files go into the
    /// zip or that folder of it ([`Explorer::zip_into`]).
    Zip(PathBuf),
}

pub(crate) struct Put {
    pub(crate) job: PutJob,
    pub(crate) dir: PathBuf,
    pub(crate) ops: Vec<ItemOp>,
    batch: Vec<Undoable>,
    landed: Vec<PathBuf>,
    /// A folder landed, so the Folders tree refills.
    landed_folder: bool,
    select_landed: bool,
    /// The last failure; `Failed` and `Finished` can arrive in different polls.
    failed: Option<(String, String)>,
    pub(crate) progress: crate::copydlg::Progress,
}

fn undo_held() -> String {
    tr!("fileops-undo-held-another-undo-is-still-running")
}

pub(crate) type JobDone = Box<dyn FnOnce(&mut Explorer, &mut Ctx) -> Reply + Send>;

/// Work on the session's undo history for [`Explorer::edit_undos`].
pub(crate) type UndoEdit = Box<dyn FnOnce(&reprisefs::undo::SharedUndo) -> JobDone + Send>;

/// An item of a put: its source, op, display name and whether it is a folder ([`Checked`]).
pub(crate) type ItemOp = (PathBuf, DropOp, String, bool);

/// Wake the window once a job just started may have run long enough to show its dialog.
pub(crate) fn wake_to_show(ctx: &Ctx) {
    let wake = ctx.wake.clone();
    std::thread::spawn(move || {
        std::thread::sleep(reprisekit::controls::copyprogress::SHOW_AFTER);
        wake.wake();
    });
}

/// What File > New makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum New {
    Folder,
    TextDocument,
    /// An empty zip.
    CompressedFolder,
}

pub struct PendingPut {
    pub(crate) items: Vec<PutItem>,
    /// Whether each of `items` is a folder, as [`Checked`] read it.
    folders: Vec<bool>,
    pub(crate) dir: PathBuf,
    select_landed: bool,
    /// The listing of the zip `dir` names and the folder of it the items go into, when they go
    /// into a zip ([`Explorer::put_in_zip`]).
    pub(crate) zip: Option<crate::intozip::IntoZip>,
}

impl PendingPut {
    pub(crate) fn new(
        items: Vec<PutItem>,
        folders: Vec<bool>,
        dir: PathBuf,
        zip: Option<crate::intozip::IntoZip>,
    ) -> PendingPut {
        PendingPut {
            items,
            folders,
            dir,
            select_landed: true,
            zip,
        }
    }

    fn ops(&self) -> Vec<ItemOp> {
        self.items
            .iter()
            .zip(&self.folders)
            .map(|(it, &folder)| (it.src.clone(), it.op, it.display_name.clone(), folder))
            .collect()
    }

    pub(crate) fn apply(&mut self, verdicts: &[Verdict]) {
        let mut v = verdicts.iter();
        let mut folders = std::mem::take(&mut self.folders).into_iter();
        self.items.retain_mut(|it| {
            let folder = folders.next().unwrap_or(false);
            let keep = match v.next() {
                Some(Verdict::Skip) => false,
                Some(Verdict::Replace(r)) => {
                    it.replace = Some(r.clone());
                    true
                }
                _ => true,
            };
            if keep {
                self.folders.push(folder);
            }
            keep
        });
    }
}

/// A batch's items as (source, op, is a folder, is a program) and their clashes in the target
/// folder, read on a listing thread before the replace prompt.
pub struct Checked {
    puts: Vec<(PathBuf, DropOp, bool, bool)>,
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

pub(crate) fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The title of a delete's error boxes.
fn delete_title() -> String {
    tr!("fileops-delete-title-error-deleting-file-or-folder")
}

/// Each of `paths`' fate in the session's Recycle Bins, and whether a delete asks first. Reads
/// the paths and their mounts, so it runs off the loop.
fn read_fates(paths: &[PathBuf]) -> (Vec<reprisefs::bincap::Fate>, bool) {
    use reprisefs::bincap::Fate;
    let bins = reprisefs::trash::Bins::system();
    let fates = (paths.iter())
        .map(|p| bins.as_ref().map_or(Fate::NoBin, |b| b.fate(p)))
        .collect();
    (fates, bins.as_ref().is_none_or(|b| b.policy.confirm))
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
        if matches!(self.place, Place::Archive(..)) {
            // Its entries come out first; cutting one would change a read-only place.
            return match cut {
                true => Reply::none(),
                false => match self.archive_picks(&self.selection_paths()) {
                    Some(picks) => self.copy_out(picks, crate::extracting::Then::Clipboard, ctx),
                    None => Reply::none(),
                },
            };
        }
        let paths = self.selected_paths();
        if paths.is_empty() || self.place == Place::RecycleBin {
            return Reply::none();
        }
        let picked: Vec<usize> = self.picked().map(|(i, _)| i).collect();
        for (i, e) in self.sv.entries.iter_mut().enumerate() {
            e.cut = cut && picked.contains(&i);
        }
        self.clipboard_paths(cut, paths, ctx)
    }

    /// Copy as Path: the selection's paths as text, when every one is a local path.
    pub fn copy_as_path(&self) -> Reply {
        let Some(items) = self.selected_local() else {
            return Reply::none();
        };
        Reply {
            copy: Some(reprisefs::ops::path_lines(
                items.iter().map(|it| it.path.as_path()),
            )),
            ..Default::default()
        }
    }

    /// The selected items, when there are some and each stands at a path of its own: an item in
    /// the Recycle Bin or an unmounted volume (listed by its device) has none to copy.
    pub(crate) fn selected_local(&self) -> Option<Vec<&reprisefs::listing::Item>> {
        let items: Vec<_> = self.picked().map(|(_, it)| it).collect();
        let local = |it: &&reprisefs::listing::Item| {
            it.trashed.is_none()
                && it.path.is_absolute()
                && !(it.drive.is_some() && it.path.starts_with("/dev"))
        };
        (self.place != Place::RecycleBin && !items.is_empty() && items.iter().all(local))
            .then_some(items)
    }

    pub fn clipboard_paths(&mut self, cut: bool, paths: Vec<PathBuf>, ctx: &mut Ctx) -> Reply {
        // The copies a compressed folder's Copy put on the clipboard are let go with it.
        self.clip_scratch.0 = None;
        self.refresh_chrome(ctx);
        Reply {
            redraw: true,
            copy_files: Some(FileClip { cut, paths }),
            ..Default::default()
        }
    }

    pub fn begin_paste(&mut self, shortcut: bool) -> Reply {
        if let Some((zip, inner)) = self.zip_view().filter(|_| !shortcut) {
            self.paste = Some(PasteTo::Zip(crate::place::in_archive(zip, inner)));
            return Reply {
                paste_files: true,
                ..Default::default()
            };
        }
        if self.place.read_only() {
            return Reply::none();
        }
        let Some(dir) = self.place.path() else {
            return Reply::none();
        };
        self.paste = Some(PasteTo::Place { dir, shortcut });
        // A picture alone on the clipboard saves as a file of its own.
        let image =
            !shortcut && !repriseapp::clipboard_has_files() && repriseapp::clipboard_has_image();
        Reply {
            paste_files: !image,
            paste_image: image,
            ..Default::default()
        }
    }

    /// The clipboard's picture for a Paste into the place on view or a folder's item, saved there
    /// in `[screenshot] format` ([`stillfile::pasted_file`]), then selected and renamed as a new
    /// file is. The
    /// encode runs on a read job: a JPEG XL takes seconds, past the listing wait.
    pub fn pasted_image(&mut self, bytes: Vec<u8>, ctx: &mut Ctx) -> Reply {
        let dir = match self.paste.take() {
            Some(PasteTo::Place { dir, .. }) if self.place.path() == Some(dir.clone()) => dir,
            Some(PasteTo::Folder(dir)) => dir,
            _ => return Reply::none(),
        };
        if bytes.is_empty() {
            return Reply::none();
        }
        let how = repriseconf::current().screenshot.still();
        self.spawn_read_job(ctx, move || {
            let file = stillfile::pasted_file(&bytes, &how);
            Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| ex.write_pasted(dir, file, ctx))
        });
        Reply::none()
    }

    fn write_pasted(
        &mut self,
        dir: PathBuf,
        file: std::io::Result<(Vec<u8>, &'static str)>,
        ctx: &mut Ctx,
    ) -> Reply {
        let secs = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH))
            .map_or(0, |d| d.as_secs() as i64);
        let work = move |gate: &Gate| {
            let (file, ext) = file?;
            if !gate.change(&dir) {
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            let path = stillfile::write_pasted(&dir, &file, ext, secs)?;
            Ok((reprisefs::undo::file_id(&path), path))
        };
        self.act(
            ctx,
            Action::changes(tr!("fileops-write-pasted-paste")),
            work,
            move |ex, made, ctx| {
                ex.item_made(&tr!("fileops-write-pasted-the-pasted-picture"), made, ctx)
            },
        )
    }

    /// Paste on the folder or launcher the item menu was opened on.
    pub fn begin_paste_on_item(&mut self) -> Reply {
        if self.menu_paste.is_none() {
            return Reply::none();
        }
        self.paste = self.menu_paste.clone();
        let image = matches!(self.paste, Some(PasteTo::Folder(_)))
            && !repriseapp::clipboard_has_files()
            && repriseapp::clipboard_has_image();
        Reply {
            paste_files: !image,
            paste_image: image,
            ..Default::default()
        }
    }

    pub fn pasted(&mut self, clip: FileClip, ctx: &mut Ctx) -> Reply {
        let (dir, shortcut) = match self.paste.take() {
            None => return Reply::none(),
            Some(PasteTo::App(app)) => {
                let terminal = repriseconf::current().terminal.clone();
                let spawn = (clip.paths.iter())
                    .map(|p| reprisefs::apps::open_argv(&app, &terminal, p))
                    .collect();
                return match spawn {
                    Ok(spawn) => Reply {
                        spawn,
                        spawn_dir: app.path,
                        spawn_app: Some(app.id.app_id()),
                        ..Default::default()
                    },
                    Err(refused) => self.exec_refused(&refused, ctx),
                };
            }
            Some(PasteTo::Folder(dir)) => (dir, false),
            Some(PasteTo::Zip(dir)) => {
                return match self.zip_into(&dir) {
                    Some((zip, inner)) => self.put_in_zip(&clip.paths, (&zip, &inner), ctx),
                    None => Reply::none(),
                };
            }
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
        self.put_all(&clip.paths, &dir, &plan, true, ctx)
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
        if let Some((zip, inner)) = self.zip_into(dir) {
            return self.put_in_zip(paths, (&zip, &inner), ctx);
        }
        let puts: Vec<(PathBuf, DropOp)> = paths
            .iter()
            .filter_map(|src| Some((src.clone(), plan(src)?)))
            .collect();
        if puts.is_empty() {
            return Reply::none();
        }
        for (src, _) in &puts {
            if !self.put_holds.iter().any(|h| src.starts_with(h.path()))
                && let Some(held) = reprisefs::archive::Scratch::hold(src)
            {
                self.put_holds.push(held);
            }
        }
        let title = puts[0].1.failure_title();
        let into = dir.to_path_buf();
        let apps_dirs = apps::default_dirs();
        let read = move || {
            let puts: Vec<_> = puts
                .into_iter()
                .map(|(src, op)| {
                    // A moved link is renamed, never read through (its target may be a dead
                    // mount), so any link counts as a folder and the tree refills either way.
                    let is_dir = match op {
                        DropOp::Move => std::fs::symlink_metadata(&src)
                            .is_ok_and(|m| m.is_dir() || m.is_symlink()),
                        _ => src.is_dir(),
                    };
                    let program = (op == DropOp::Copy)
                        .then(|| apps::program_source(&apps_dirs, &src, &into))
                        .flatten();
                    let is_app = program.is_some();
                    (program.unwrap_or(src), op, is_dir, is_app)
                })
                .collect();
            Checked {
                clashes: replace::clashes(
                    puts.iter()
                        .map(|(src, op, _, is_app)| (src.as_path(), *op, *is_app)),
                    &into,
                ),
                puts,
            }
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
            title: title.to_string(),
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
                    &c.title,
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
        let folders = c
            .puts
            .iter()
            .map(|&(_, op, is_dir, _)| is_dir && op != DropOp::Shortcut)
            .collect();
        let items = c
            .puts
            .into_iter()
            .map(|(src, op, is_dir, is_app)| {
                let icon = self.icon_name_as(&src, is_dir, ctx);
                let command = apps::open_command_as(&src, is_dir);
                PutItem {
                    display_name: file_label(&src),
                    src,
                    is_app,
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
            folders,
            dir,
            select_landed,
            zip: None,
        };
        self.ask_replace(pending, Asker::new(c.clashes), ctx)
    }

    /// Asks about the next clash of `pending`. A closed window has no one to ask, so it answers
    /// No to each: nothing is replaced and the rest is put. Behind a box up the paste waits its
    /// turn, except behind a busy Connect to Server, whose place the prompt takes
    /// ([`Explorer::raise`]).
    pub(crate) fn ask_replace(
        &mut self,
        mut pending: PendingPut,
        mut asker: Asker,
        ctx: &mut Ctx,
    ) -> Reply {
        while self.closing && matches!(asker.step(), Step::Ask { .. }) {
            asker.answer(Answer::No);
        }
        let waits = !self.dialogs.is_empty() && !self.answered && !self.connecting_up();
        let step = asker.step();
        if matches!(step, Step::Cancelled | Step::Done(_)) {
            // The drop's put asks nothing more: a later box is not the drop's.
            self.dropped_at = None;
        }
        match step {
            Step::Cancelled => Reply::none(),
            Step::Ask { .. } if waits => self.raise(Dialog::Paste(Box::new((pending, asker)))),
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
                let held = pending.zip.clone();
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
                    let existing = |path: PathBuf| match &held {
                        Some(into) => {
                            let name = path.file_name().unwrap_or_default().to_owned();
                            let (size, modified) = crate::intozip::held_facts(into, &name, is_12h);
                            ReadSide {
                                path,
                                size,
                                modified,
                                is_dir: false,
                            }
                        }
                        None => ReadSide::of(path, is_12h),
                    };
                    (clash == Clash::File).then(|| [existing(dest), ReadSide::of(src, is_12h)])
                };
                match self.read_job(ctx, read) {
                    Ok(sides) => self.open_replace(ask, sides, ctx),
                    Err(job) => {
                        let deadline = Deadline::start(self.action_deadline, ctx);
                        self.raise(Dialog::Reading(Box::new((ask, job, deadline))));
                        Reply::none()
                    }
                }
            }
            Step::Done(verdicts) => {
                pending.apply(&verdicts);
                if pending.zip.is_some() {
                    if !pending.items.is_empty() {
                        self.start_zip_add(pending, ctx);
                    }
                    return Reply::none();
                }
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

    /// Raises the replace prompt for `ask` on its sides as read ([`ReadSide`]).
    fn open_replace(&mut self, ask: Asking, sides: Option<[ReadSide; 2]>, ctx: &mut Ctx) -> Reply {
        let side = |s: ReadSide, ex: &Self, ctx: &mut Ctx| {
            let pick = crate::icons::file_pick(&ex.globs, &s.path, s.is_dir);
            let icon = IconSlot::new(&mut ctx.icons, pick, 32);
            let side = replacebox::Side {
                size: s.size,
                modified: s.modified,
                icon: icon.icon(),
            };
            (Box::new(side), icon)
        };
        let (subject, sides) = match sides {
            Some([existing, incoming]) => {
                let (existing, a) = side(existing, self, ctx);
                let (incoming, b) = side(incoming, self, ctx);
                let subject = replacebox::Subject::File { existing, incoming };
                (subject, Some([a, b]))
            }
            None => (replacebox::Subject::Folder, None),
        };
        let folder = IconSlot::chrome(&mut ctx.icons, &["folder"], 32);
        let fields = replacebox::Fields {
            name: file_label(&ask.src),
            folder: ask.folder,
            subject,
            icon: folder.icon(),
            many: ask.many,
        };
        self.raise(Dialog::Replace(Box::new(ReplaceAsk {
            fields,
            icons: (folder, sides),
            asker: ask.asker,
            pending: ask.pending,
            focus: None,
        })))
    }

    /// Raises the replace prompt whose sides ([`Explorer::ask_replace`]) were read late. A window
    /// closed meanwhile asks again, which answers No, and so do the pastes waiting behind it.
    pub(crate) fn take_asking(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let reply = self.asking_landed(ctx);
        self.answer_ends();
        reply
    }

    fn asking_landed(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let Some(Dialog::Reading(r)) = self.dialogs.up() else {
            return None;
        };
        let (ask, job, deadline) = &**r;
        let Some(sides) = job.take() else {
            if !deadline.passed(std::time::Instant::now()) {
                return None;
            }
            let title = (ask.pending.items.first()).map_or_else(
                || tr!("fileops-asking-landed-paste"),
                |it| it.op.failure_title().to_string(),
            );
            log::warn!("explorer: a replace prompt's files gave up");
            self.take_up();
            if self.closing {
                return Some(Reply::none());
            }
            let text = reprisefs::deadline::not_responding(false, false);
            return Some(self.message_box(&title, text, ctx));
        };
        let Some(Dialog::Reading(r)) = self.take_up() else {
            return None;
        };
        let (ask, ..) = *r;
        if self.closing {
            return Some(self.ask_replace(ask.pending, ask.asker, ctx));
        }
        Some(self.open_replace(ask, sides, ctx))
    }

    fn start_put(&mut self, pending: PendingPut, ops: Vec<ItemOp>, ctx: &Ctx) {
        use reprisekit::controls::copyprogress::Kind;
        let wake = ctx.wake.clone();
        let job = PutJob::start_asking(
            pending.items,
            pending.dir.clone(),
            self.action_deadline,
            move || wake.wake(),
        );
        wake_to_show(ctx);
        let kind = if ops.iter().all(|(_, op, ..)| *op == DropOp::Move) {
            Kind::Move
        } else {
            Kind::Copy
        };
        let from = ops
            .first()
            .and_then(|(src, ..)| src.parent())
            .map(|p| self.title_of(&Place::from_path(p)))
            .unwrap_or_default();
        let to = self.title_of(&Place::from_path(&pending.dir));
        let progress = crate::copydlg::Progress::new(job.meter().clone(), kind, from, to);
        self.put = Some(Put {
            job,
            dir: pending.dir,
            ops,
            batch: Vec::new(),
            landed: Vec::new(),
            landed_folder: false,
            select_landed: pending.select_landed,
            failed: None,
            progress,
        });
    }

    pub(crate) fn poll_put(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        self.unsettled.retain(|j| !j.settled());
        let put = self.put.as_mut()?;
        let mut finished = false;
        let mut ask = None;
        for ev in put.job.poll() {
            match ev {
                PutEvent::Landed {
                    index,
                    landed,
                    id,
                    copied,
                    replaced,
                    unlisted,
                } => {
                    let (src, op, _, folder) = &put.ops[index];
                    let dest = put.dir.join(&landed);
                    put.landed.push(dest.clone());
                    put.landed_folder |= folder;
                    if replaced {
                        continue;
                    }
                    put.batch.push(match op {
                        DropOp::Move => Undoable::Move {
                            from: src.clone(),
                            to: dest,
                            listed: unlisted,
                            id,
                        },
                        _ => Undoable::Copied(dest, copied),
                    });
                }
                PutEvent::Failed { index, error } => {
                    let (_, op, name, _) = &put.ops[index];
                    let failure = op.failure(name, &error);
                    let failure = (failure.0.to_string(), failure.1);
                    match put.progress.meter.cancelled() {
                        true => put.failed = Some(failure),
                        false => ask = Some(failure),
                    }
                }
                PutEvent::Finished => finished = true,
            }
        }
        if !finished && let Some((title, text)) = ask {
            let meter = put.progress.meter.clone();
            return Some(self.ask_failed(
                reprisefs::progress::Asked::new(meter),
                &title,
                text,
                ctx,
            ));
        }
        // A job that ended does not wait for an answer: its end tells the failure.
        if let Some(failure) = ask {
            put.failed = Some(failure);
        }
        if !finished {
            return Some(Reply::none());
        }
        let mut put = self.put.take()?;
        self.job_ended(&put.progress.meter, ctx);
        if !put.job.settled() {
            self.unsettled.push(put.job);
        }
        let failed = put.failed.take();
        put.progress.ended(put.landed.len(), failed.as_ref());
        self.publish_task_progress();
        let batch = std::mem::take(&mut put.batch);
        self.record_undos(ctx, move |undos| undos.push_all(batch));
        if put.landed_folder {
            let mut dirs = vec![put.dir.clone()];
            for (src, op, ..) in &put.ops {
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
            self.select_once_listed(self.place.clone(), put.landed, ctx);
        }
        if let Some((title, text)) = failed {
            reply = self.message_box(&title, text, ctx);
        }
        if let Some(next) = self.put_queue.pop_front() {
            let ops = next.ops();
            self.start_put(next, ops, ctx);
        }
        Some(reply)
    }

    /// Run `edit` on the session's undo history on a thread off the loop, after every edit this
    /// window queued before it: each edit rewrites the whole history, which may run to megabytes.
    /// Its result runs on the window like a job's, which a close waits for. The Undo rows stay
    /// greyed till then ([`Explorer::sync_undo`]).
    pub(crate) fn edit_undos(
        &mut self,
        ctx: &Ctx,
        edit: impl FnOnce(&reprisefs::undo::SharedUndo) -> JobDone + Send + 'static,
    ) {
        let mut edit: UndoEdit = Box::new(move |undos| {
            // A panic still takes the edit off the window's count, and the edits behind it run.
            let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| edit(undos)));
            Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| {
                ex.undo_edits_pending -= 1;
                match done {
                    Ok(done) => done(ex, ctx),
                    Err(_) => {
                        ex.sync_undo();
                        Reply::redraw()
                    }
                }
            })
        });
        self.jobs_pending += 1;
        self.undo_edits_pending += 1;
        if let Some(tx) = &self.undo_edits {
            match tx.send(edit) {
                Ok(()) => return,
                Err(e) => edit = e.0,
            }
        }
        let (tx, rx) = std::sync::mpsc::channel::<UndoEdit>();
        let (undos, done, wake) = (self.undos.clone(), self.jobs_tx.clone(), ctx.wake.clone());
        let spawned = std::thread::Builder::new()
            .name("reprise-explorer-undo".into())
            .spawn(move || {
                for edit in rx {
                    if done.send(edit(&undos)).is_ok() {
                        wake.wake();
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("explorer: no thread for the undo history ({e}); editing it on the window");
            let _ = self.jobs_tx.send(edit(&self.undos));
            ctx.wake.wake();
            return;
        }
        let _ = tx.send(edit);
        self.undo_edits = Some(tx);
    }

    /// [`Explorer::edit_undos`] for an edit the window learns of through its Undo rows alone.
    pub(crate) fn record_undos(
        &mut self,
        ctx: &Ctx,
        edit: impl FnOnce(&reprisefs::undo::SharedUndo) + Send + 'static,
    ) {
        self.edit_undos(ctx, move |undos| {
            edit(undos);
            Box::new(|ex: &mut Explorer, _: &mut Ctx| {
                ex.sync_undo();
                Reply::redraw()
            })
        });
    }

    /// Run `work` on a thread; its result runs on the window at the next wake. Used for slow work
    /// that changes files or drives (deletes, Empty, mounts), which a close waits for.
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

    /// [`Explorer::spawn_job`] for work that only reads, on a listing thread
    /// ([`Explorer::spawn_read`]) a close does not wait for: a FIFO or a dead mount it hangs on
    /// never holds the closed window's process.
    pub(crate) fn spawn_read_job(
        &mut self,
        ctx: &Ctx,
        read: impl FnOnce() -> JobDone + Send + 'static,
    ) {
        let job = self.spawn_read(ctx, read);
        self.reads.push(job);
    }

    /// The reads [`Explorer::spawn_read_job`] started that have answered, run on the window.
    pub(crate) fn take_reads(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let mut done = Vec::new();
        self.reads.retain(|j| match j.take() {
            Some(d) => {
                done.push(d);
                false
            }
            None => true,
        });
        done.into_iter().map(|d| d(self, ctx)).reduce(Reply::and)
    }

    pub(crate) fn poll_jobs(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let mut reply: Option<Reply> = None;
        while let Ok(done) = self.jobs_rx.try_recv() {
            self.jobs_pending = self.jobs_pending.saturating_sub(1);
            let r = done(self, ctx);
            self.answer_ends();
            reply = Some(match reply {
                Some(was) => was.and(r),
                None => r,
            });
        }
        reply
    }

    /// Whether no put is running, queued, being checked or asking about a clash.
    pub(crate) fn puts_idle(&self) -> bool {
        self.put.is_none()
            && self.put_queue.is_empty()
            && self.clashing.is_empty()
            && self.unsettled.iter().all(|j| j.settled())
            && !(self.dialogs.iter()).any(|d| {
                matches!(
                    d,
                    Dialog::Paste(_) | Dialog::Reading(_) | Dialog::Replace(_)
                )
            })
    }

    pub(crate) fn busy(&self) -> bool {
        self.put.is_some()
            || self.unsettled.iter().any(|j| !j.settled())
            || !self.clashing.is_empty()
            || self.removing.is_some()
            || self.extracting.is_some()
            || self.jobs_pending > 0
            || !self.opening.is_empty()
            || !self.launches.is_empty()
            || !self.starting.is_empty()
            || self.dialogs.iter().any(|d| matches!(d, Dialog::Reading(_)))
            || !self.acting.is_empty()
            || !self.send.sends.is_empty()
            || !self.browse_left.is_empty()
            || self.favorites_busy()
            || self.pins.as_ref().is_some_and(|p| p.has_failed())
    }

    /// File > Close or the close box. With work in flight the window hides and [`Explorer::wake`]
    /// quits once it drains and every error it raised since is answered
    /// ([`Explorer::owes_a_box`]). The hidden window asks nothing: the box up (the taskbar's Close
    /// passes it) and those waiting are dismissed ([`Explorer::dismiss`]), which ends a job held
    /// on a failed item, and every replace prompt answers No, so the rest of its paste is put.
    /// The sheets and the preview go with the window.
    pub fn close(&mut self, ctx: &mut Ctx) -> Reply {
        self.flush_settings();
        // A password check is a job the close would wait for.
        self.archives.cancel();
        self.closing = self
            .dialogs
            .iter()
            .any(|d| !matches!(d, Dialog::Reading(_)));
        // A paste's work stays: behind a prompt whose files are being read, `take_asking` and
        // `ask_pastes` answer them in turn.
        for d in self.dialogs.withdraw(Dialog::shows) {
            self.dismiss(d, ctx);
        }
        self.ask_pastes(ctx);
        self.answer_ends();
        (self.props, self.props_due, self.bin_props, self.preview) = (None, None, None, None);
        self.folder_opts = None;
        if self.busy() {
            self.closing = true;
            return Reply::none();
        }
        Reply {
            quit: true,
            ..Default::default()
        }
    }

    /// Whether the closed window still owes an error box, which comes up with no folder window
    /// till it is answered. A question raised since the close is dismissed unasked.
    pub(crate) fn owes_a_box(&mut self, ctx: &mut Ctx) -> bool {
        let question = |d: &Dialog| d.shows() && !matches!(d, Dialog::Message { .. });
        for d in self.dialogs.withdraw(question) {
            self.dismiss(d, ctx);
        }
        self.dialog().is_some()
    }

    /// The title and text of the box a failed item raised ([`Explorer::ask_failed`]), answered
    /// `d` through its button, then the jobs run out.
    #[cfg(test)]
    pub(crate) fn answer_failed(
        &mut self,
        d: reprisefs::progress::Decision,
        ctx: &mut Ctx,
    ) -> Option<(String, String)> {
        use reprisefs::progress::Decision;
        let Some(Dialog::Confirm(c)) = self.dialogs.up() else {
            return None;
        };
        c.job.as_ref()?;
        let said = (c.title.to_string(), c.text.clone());
        let button = match d {
            Decision::Skip => 0,
            Decision::Retry => 1,
            Decision::Pending | Decision::Abort => 2,
        };
        let id = reprisekit::controls::dlgtemplate::BUTTON_IDS[button];
        self.dialog_outcome(reprisekit::controls::form::Event::Pressed(id), ctx);
        self.finish_puts(ctx);
        Some(said)
    }

    #[cfg(test)]
    pub(crate) fn finish_puts(&mut self, ctx: &mut Ctx) -> Reply {
        let mut reply = Reply::none();
        while (self.busy() && !self.holds_a_job()) || self.tip_walk_out() || !self.reads.is_empty()
        {
            std::thread::sleep(std::time::Duration::from_millis(2));
            if let Some(r) = self.take_reads(ctx) {
                reply = r;
            }
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
        use reprisefs::bincap::delete_mode;
        let in_bin = self.place == Place::RecycleBin;
        if self.place.read_only() && !in_bin {
            return Reply::none();
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return Reply::none();
        }
        let single = match self.picked().collect::<Vec<_>>()[..] {
            [(i, _)] => Some(i),
            _ => None,
        };
        let only = single.and_then(|i| self.items.get(i));
        let is_dir = only.is_some_and(|it| it.is_dir && !it.is_link);
        let name = only.map(|it| it.label().to_string()).unwrap_or_default();
        let kind = ConfirmKind::of(paths.len(), is_dir);
        let selected = single.and_then(|i| self.sv.entries.get(i));
        let own_icon = selected
            .and_then(|e| e.icon32.as_ref().or(e.icon.as_ref()).or(e.icon48.as_ref()))
            .and_then(|worn| self.list_icons.name_of(worn))
            .map(str::to_string);
        let bin = in_bin.then(|| self.selected_bin_items());
        let read = paths.clone();
        let work = move |_: &Gate| read_fates(&read);
        let action = Action::reads(delete_title());
        self.act(ctx, action, work, move |ex, (fates, confirm), ctx| {
            let mode = delete_mode(confirm, &fates, erase);
            if !in_bin && !mode.ask {
                return ex.delete_now(&paths, mode.erase, ctx);
            }
            let no_bin = if in_bin { BinLack::None } else { mode.no_bin };
            let erase = in_bin || mode.erase;
            let own = own_icon.as_deref().filter(|_| erase);
            let names: Vec<&str> = own.into_iter().chain(TRASH_ICON.iter().copied()).collect();
            let icon = IconSlot::chrome(&mut ctx.icons, &names, 32);
            ex.confirm_box(kind, &name, paths, erase, no_bin, icon, None, bin)
        })
    }

    pub fn confirm_empty_bin(&mut self, ctx: &mut Ctx) -> Reply {
        let only = (self.items.len() == 1).then(|| &self.items[0]);
        let one = only.map(|it| (it.label().to_string(), it.is_dir));
        self.ask_empty(self.items.len(), one, ctx)
    }

    /// The Empty question over `count` items, `one` naming the only one and whether it is a
    /// folder; nothing to ask when the bins hold none.
    fn ask_empty(&mut self, count: usize, one: Option<(String, bool)>, ctx: &mut Ctx) -> Reply {
        if count == 0 {
            return Reply::none();
        }
        let (name, is_dir) = one.filter(|_| count == 1).unwrap_or_default();
        let kind = ConfirmKind::of(count, is_dir);
        let icon = IconSlot::chrome(&mut ctx.icons, TRASH_ICON, 32);
        self.confirm_box(
            kind,
            &name,
            Vec::new(),
            true,
            BinLack::None,
            icon,
            Some(count),
            None,
        )
    }

    /// `--delete`: Delete, or Shift+Delete with `erase`, on `paths` with no folder window, asked
    /// as [`Explorer::confirm_delete`] asks of a selection outside the Recycle Bin.
    pub(crate) fn confirm_delete_paths(
        &mut self,
        paths: Vec<PathBuf>,
        erase: bool,
        ctx: &mut Ctx,
    ) -> Reply {
        let read = paths.clone();
        let work = move |_: &Gate| {
            // A link to a folder deletes as a file: only the link goes.
            let one = match read.as_slice() {
                [p] => {
                    let meta = std::fs::symlink_metadata(p).ok();
                    let shortcut = meta.and_then(|m| apps::shortcut_at(p, &m));
                    let label = shortcut.map(|s| s.name().to_string());
                    Some((label, p.is_dir() && !p.is_symlink()))
                }
                _ => None,
            };
            (read_fates(&read), one)
        };
        self.act(
            ctx,
            Action::reads(delete_title()),
            work,
            move |ex, read, ctx| {
                let ((fates, confirm), one) = read;
                let mode = reprisefs::bincap::delete_mode(confirm, &fates, erase);
                if !mode.ask {
                    return ex.delete_now(&paths, mode.erase, ctx);
                }
                let is_dir = one.as_ref().is_some_and(|(_, d)| *d);
                let name = match (paths.as_slice(), one) {
                    ([p], Some((label, _))) => label.unwrap_or_else(|| file_label(p)),
                    _ => String::new(),
                };
                let own = match paths.as_slice() {
                    [p] if mode.erase => Some(ex.icon_name_as(p, is_dir, ctx)),
                    _ => None,
                };
                let names: Vec<&str> = (own.as_deref().into_iter())
                    .chain(TRASH_ICON.iter().copied())
                    .collect();
                let icon = IconSlot::chrome(&mut ctx.icons, &names, 32);
                let kind = ConfirmKind::of(paths.len(), is_dir);
                ex.confirm_box(
                    kind,
                    &name,
                    paths,
                    mode.erase,
                    mode.no_bin,
                    icon,
                    None,
                    None,
                )
            },
        )
    }

    /// `--empty-bin`: Empty Recycle Bin with no folder window. What the bins hold is counted off
    /// the loop, so a drive's bin on a mount that stopped answering gives up instead of asking
    /// minutes later.
    pub(crate) fn confirm_empty_bins(&mut self, ctx: &mut Ctx) -> Reply {
        let work = |_: &Gate| {
            let files = reprisefs::trash::Bins::system()
                .map(|bins| bins.files_dirs())
                .unwrap_or_default();
            let mut count = 0;
            let mut first: Option<PathBuf> = None;
            for dir in files {
                let (n, name) = reprisefs::places::trash_contents(&dir);
                count += n;
                if let Some(name) = name.map(|n| dir.join(n))
                    && first
                        .as_ref()
                        .is_none_or(|f| name.file_name() < f.file_name())
                {
                    first = Some(name);
                }
            }
            let one = first.filter(|_| count == 1).map(|f| {
                let is_dir = f.is_dir() && !f.is_symlink();
                (file_label(&f), is_dir)
            });
            (count, one)
        };
        self.act(
            ctx,
            Action::reads(delete_title()),
            work,
            |ex, (count, one), ctx| ex.ask_empty(count, one, ctx),
        )
    }

    /// `--put`: `sources` copied, moved or linked into `dir` as a paste puts them, with no folder
    /// window; a source moved into the folder that holds it stays.
    pub(crate) fn put_paths(
        &mut self,
        op: DropOp,
        sources: &[PathBuf],
        dir: &Path,
        ctx: &mut Ctx,
    ) -> Reply {
        let (cut, shortcut) = (op == DropOp::Move, op == DropOp::Shortcut);
        let plan = |src: &Path| plan_paste(cut, shortcut, src, dir);
        self.put_all(sources, dir, &plan, false, ctx)
    }

    /// Yes on that prompt: empty every trash. `Trash` undos go with it (their `files/` copies are
    /// gone); other undos stay. A failure leaves the stack as it was.
    pub(crate) fn empty_bin_now(&mut self, ctx: &mut Ctx) -> Reply {
        let Some(bins) = reprisefs::trash::Bins::system() else {
            return self.message_box(
                &tr!("fileops-empty-bin-now-error-deleting-file-or-folder"),
                tr!("fileops-empty-bin-now-cannot-find-the-recycle-bin"),
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
        let from = self.title_of(&self.place);
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
                let mut c = self.delete_question(
                    kind,
                    "",
                    Vec::new(),
                    true,
                    BinLack::None,
                    icon,
                    None,
                    None,
                );
                c.text = text;
                c.too_big = Some(asker);
                self.raise(Dialog::Confirm(c))
            }
            TooBigStep::Done(plan) => {
                let from = self.title_of(&self.place);
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

    pub fn new_item(&mut self, kind: New, ctx: &mut Ctx) -> Reply {
        if self.zip_view().is_some() {
            return self.new_in_zip(kind, ctx);
        }
        let Some(dir) = self.place.path() else {
            return Reply::none();
        };
        if self.place.read_only() {
            return Reply::none();
        }
        let existing: Vec<String> = self.items.iter().map(|it| it.name.clone()).collect();
        let (name, title) = match kind {
            New::Folder => (names::new_folder_name(&existing), tr!("new-folder-name")),
            New::TextDocument => (
                names::new_text_document_name(&existing),
                tr!("fileops-new-item-new-text-document"),
            ),
            New::CompressedFolder => (
                names::new_compressed_folder_name(&existing),
                tr!("fileops-new-item-new-compressed-folder"),
            ),
        };
        let path = dir.join(&name);
        // The listing can be stale, so no kind may land on an existing name: each fails with
        // `AlreadyExists`.
        let work = move |gate: &Gate| {
            if !gate.change(&dir) {
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            let file = || {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
            };
            let made = match kind {
                New::Folder => std::fs::create_dir(&path),
                New::TextDocument => file().map(drop),
                New::CompressedFolder => file().and_then(|mut f| {
                    std::io::Write::write_all(&mut f, &reprisefs::zipwrite::EMPTY_ZIP)
                }),
            };
            made.map(|()| (reprisefs::undo::file_id(&path), path))
        };
        self.act(ctx, Action::changes(title), work, move |ex, made, ctx| {
            ex.item_made(&name, made, ctx)
        })
    }

    /// New > a template: a copy of `template` in the folder shown ([`reprisefs::templates::make`]),
    /// then selected and renamed as a new folder is.
    pub fn new_from_template(&mut self, template: PathBuf, ctx: &mut Ctx) -> Reply {
        let Some(dir) = self.place.path() else {
            return Reply::none();
        };
        if self.place.read_only() {
            return Reply::none();
        }
        let name = file_label(&template);
        let work = move |gate: &Gate| {
            if !gate.change(&dir) {
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            reprisefs::templates::make(&template, &dir).map(|p| (reprisefs::undo::file_id(&p), p))
        };
        self.act(
            ctx,
            Action::changes(tr!("fileops-new-from-template-new")),
            work,
            move |ex, made, ctx| ex.item_made(&name, made, ctx),
        )
    }

    /// A New's item landed at the path `made` holds, with its identity: one undo entry, then it
    /// is selected and scrolled into view with its rename begun. A failure names `name` in an
    /// error box.
    pub(crate) fn item_made(
        &mut self,
        name: &str,
        made: std::io::Result<(Option<reprisefs::undo::FileId>, PathBuf)>,
        ctx: &mut Ctx,
    ) -> Reply {
        let (id, path) = match made {
            Ok(made) => made,
            Err(e) => {
                return self.message_box(
                    &tr!("fileops-item-made-error-creating-file-or-folder"),
                    tr!(
                        "fileops-item-made-cannot-create-name-e",
                        name = name,
                        e = e.to_string()
                    ),
                    ctx,
                );
            }
        };
        let made = Undoable::New(path.clone(), id);
        self.record_undos(ctx, move |undos| undos.push(made));
        let reply = self.refresh(ctx);
        if self.index_of(&path).is_some() {
            self.select_shown(std::slice::from_ref(&path), ctx);
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

    /// Send To rows for the selection as (label, enabled, icon), cached so the menu's row
    /// index resolves to a destination; stand-ins noted in `waits` while an icon decodes.
    pub fn build_send_to(
        &mut self,
        waits: &mut IconWaits,
        ctx: &mut Ctx,
    ) -> Vec<(String, bool, Option<Canvas>)> {
        self.send_to = self.send_rows(ctx);
        let px = menu_icon_px(&ctx.theme);
        (self.send_to.iter())
            .map(|r| {
                let names = ctx.icons.rank(r.icons);
                let icon = waits.first(&mut ctx.icons, &names, px);
                (r.label.clone(), r.enabled, icon)
            })
            .collect()
    }

    /// A read-only place sends nothing; the zip row runs [`Explorer::compress`]. Copies and
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
            SendTo::Zip => return self.compress(ctx),
            SendTo::Mail => return self.mail(paths, ctx),
            SendTo::DesktopShortcut => match reprisefs::places::user_dir("DESKTOP", "Desktop") {
                Some(d) => (d, DropOp::Shortcut),
                None => return Reply::none(),
            },
            SendTo::Folder(d) => return self.send_to_folder(paths, d, ctx),
            SendTo::Phone(id) => {
                sendto::send_to_phone(id, &paths);
                return Reply::none();
            }
        };
        self.put_all(&paths, &dir, &|_| Some(op), false, ctx)
    }

    /// Mail Recipient: a new mail with `paths` attached, or the box saying xdg-email is missing.
    /// A folder sends nothing, as its rows are greyed.
    pub fn mail(&mut self, paths: Vec<PathBuf>, ctx: &mut Ctx) -> Reply {
        if paths.is_empty() || self.picked().any(|(_, it)| it.is_folder()) {
            return Reply::none();
        }
        match sendto::mail_argv(&paths) {
            Some(argv) => Reply {
                spawn: vec![argv],
                ..Reply::none()
            },
            None => self.message_box(sendto::NO_MAIL_TITLE, sendto::NO_MAIL_TEXT.into(), ctx),
        }
    }

    /// How a launcher or web link shortcut opens.
    pub fn open_for(
        &self,
        shortcut: &apps::Shortcut,
    ) -> Result<reprisefs::props::Opening, apps::ExecRefused> {
        use reprisefs::props::Opening;
        Ok(match shortcut {
            apps::Shortcut::App(app) => Opening::of(
                app,
                apps::launch_argv(app, &repriseconf::current().terminal)?,
            ),
            apps::Shortcut::Link(link) => Opening {
                argv: vec!["xdg-open".into(), link.url.clone().into()],
                app: None,
                dir: None,
            },
        })
    }

    /// The box for a desktop entry Reprise refuses to run; nothing runs.
    pub(crate) fn exec_refused(&mut self, refused: &apps::ExecRefused, ctx: &mut Ctx) -> Reply {
        self.message_box(&refused.name, refused.to_string(), ctx)
    }

    /// Open With rows for item `i`, stand-ins noted in `waits` while an icon decodes.
    pub fn build_open_with(
        &mut self,
        i: usize,
        waits: &mut IconWaits,
        ctx: &mut Ctx,
    ) -> Vec<(String, Option<Canvas>)> {
        self.open_with_path = self.items.get(i).map(|it| it.path.clone());
        self.open_with_apps = self
            .items
            .get(i)
            .and_then(|it| {
                let name = || it.path.file_name().map(|n| n.to_string_lossy());
                // Only files get the submenu, so the name decides without a stat.
                (it.mime.clone()).or_else(|| self.globs.mime_of_name(&name()?))
            })
            .map(|m| reprisefs::apps::open_with(&self.app_index, &m))
            .unwrap_or_default();
        let apps = self.open_with_apps.clone();
        let px = menu_icon_px(&ctx.theme);
        apps.iter()
            .filter_map(|&a| self.app_index.get(a).cloned())
            .map(|app| {
                let name = app.icon.as_deref();
                let icon = name.and_then(|n| waits.first(&mut ctx.icons, &[n], px));
                (app.name.clone(), icon)
            })
            .collect()
    }

    /// The icon names of the Open With or Send To row that runs `cmd`, the first the theme carries
    /// worn; none for any other row.
    pub(crate) fn row_icons(&self, cmd: &crate::commands::Command) -> Vec<String> {
        use crate::commands::Command;
        match cmd {
            Command::OpenWith(i) => {
                let app = self
                    .open_with_apps
                    .get(*i)
                    .and_then(|&a| self.app_index.get(a));
                app.and_then(|a| a.icon.clone()).into_iter().collect()
            }
            Command::SendTo(i) => (self.send_to.get(*i)).map_or(Vec::new(), |r| {
                r.icons.iter().map(|n| n.to_string()).collect()
            }),
            _ => Vec::new(),
        }
    }

    pub fn open_with(&mut self, i: usize, ctx: &mut Ctx) -> Reply {
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
        match reprisefs::apps::open_argv(app, &repriseconf::current().terminal, &path) {
            Ok(argv) => Reply {
                redraw: true,
                spawn: vec![argv],
                spawn_app: Some(app.id.app_id()),
                spawn_dir: app.path.clone(),
                ..Default::default()
            },
            Err(refused) => self.exec_refused(&refused, ctx),
        }
    }

    pub fn choose_program(&mut self, ctx: &mut Ctx) -> Reply {
        let Some(path) = self.open_with_path.clone() else {
            return Reply::none();
        };
        self.choose_program_for(&path, None, ctx)
    }

    /// The Open With chooser for `path`, once a listing thread has read its type. For `sheet`'s
    /// Change... a link's target is read in its place, and no chooser comes up once that sheet
    /// is gone.
    pub(crate) fn choose_program_for(
        &mut self,
        path: &Path,
        sheet: Option<SheetId>,
        ctx: &mut Ctx,
    ) -> Reply {
        let (path, globs) = (path.to_path_buf(), self.globs.clone());
        let work = move |_: &Gate| {
            let path = match sheet {
                Some(_) => std::fs::canonicalize(&path).unwrap_or(path),
                None => path,
            };
            let mime = reprisefs::apps::mime_of_in(&globs, &path);
            (path, mime)
        };
        let title = match sheet {
            Some(_) => tr!("fileops-choose-program-for-properties"),
            None => tr!("fileops-choose-program-for-open-with"),
        };
        let action = Action {
            sheet,
            ..Action::reads(title)
        };
        self.act(ctx, action, work, move |ex, (path, mime), ctx| {
            if sheet.is_some_and(|s| !ex.sheet_up(s)) {
                return Reply::none();
            }
            ex.open_chooser(path, mime, sheet, ctx)
        })
    }

    fn open_chooser(
        &mut self,
        path: PathBuf,
        mime: Option<String>,
        sheet: Option<SheetId>,
        ctx: &mut Ctx,
    ) -> Reply {
        let apps = &self.app_index;
        let default = mime
            .as_deref()
            .and_then(|m| reprisefs::props::default_app(m, apps));
        let (recommended, others) = reprisefs::apps::chooser_groups(apps, mime.as_deref(), default);
        let program = |i: usize| Program {
            id: apps[i].id.to_string(),
            name: apps[i].name.clone(),
            icon: apps[i].icon.clone(),
        };
        let mut ow = OpenWith::new(
            &file_label(&path),
            recommended.into_iter().map(program).collect(),
            others.into_iter().map(program).collect(),
        );
        if let Some(d) = default {
            ow.select_id(apps[d].id.as_str());
        }
        if sheet.is_some() {
            ow.always = Check {
                on: true,
                enabled: false,
            };
        }
        let mut cp = ChooseProgram {
            path,
            mime,
            ow,
            sheet,
            icon: None,
            row_icons: Vec::new(),
            waits: IconWaits::default(),
            input: ChooserInput::opened(self.keyed),
            browse: None,
        };
        cp.wear_icons(&mut ctx.icons, self.forms.controls().text);
        self.raise(Dialog::ChooseProgram(Box::new(cp)))
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
            if c.sheet.is_some() || !Path::new(&id).is_absolute() {
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
        if let Some(sheet) = c.sheet {
            self.props_set_opens_with(sheet, app.name.clone(), app.icon.clone(), ctx);
            return Reply::redraw_all();
        }
        match reprisefs::apps::open_argv(&app, &repriseconf::current().terminal, &c.path) {
            Ok(argv) => Reply {
                redraw_all: true,
                spawn: vec![argv],
                spawn_dir: app.path,
                spawn_app: Some(app.id.app_id()),
                ..Default::default()
            },
            Err(refused) => self.exec_refused(&refused, ctx),
        }
    }

    fn browsed_entry(
        &mut self,
        id: &str,
        c: &ChooseProgram,
        always: bool,
    ) -> Option<reprisefs::apps::AppEntry> {
        let mime = c.mime.as_deref().filter(|_| always || c.sheet.is_some())?;
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

    /// Edit > Undo, the background menu's Undo and Ctrl+Z: Undo of the entry the Undo rows name,
    /// or a box saying another undo still runs. While this window's own edits of the history are
    /// on their way, of the entry they leave on top once they land.
    pub fn undo_last(&mut self, ctx: &mut Ctx) -> Reply {
        if self.undo_held {
            return self.message_box(&tr!("fileops-undo-last-error"), undo_held(), ctx);
        }
        match self.undo_entry {
            Some(entry) => self.undo_with(entry, false, ctx),
            None if self.undo_edits_pending > 0 => {
                self.edit_undos(ctx, |undos| {
                    let (head, held) = (undos.head(), undos.held());
                    Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| {
                        ex.sync_undo();
                        match (head, held) {
                            (Some((entry, _)), None) => ex.undo_with(entry, false, ctx),
                            (Some(_), Some(Hold::Undo)) => {
                                ex.message_box(&tr!("fileops-undo-last-error"), undo_held(), ctx)
                            }
                            _ => Reply::redraw(),
                        }
                    })
                });
                Reply::none()
            }
            None => Reply::none(),
        }
    }

    /// Undo of `entry`, only while it is the top one of the session's history and no undo in any
    /// window runs. With `erase` what the Recycle Bin refuses is removed for good; without it such
    /// an item stops the undo and asks first. The entry leaves the history and its steps run on a
    /// removal job with the progress dialog ([`crate::removing::PendingRemoval::undo`]); the
    /// Recycle Bins are found on that thread first, as for a delete.
    pub(crate) fn undo_with(
        &mut self,
        entry: reprisefs::undo::EntryId,
        erase: bool,
        ctx: &mut Ctx,
    ) -> Reply {
        self.edit_undos(ctx, move |undos| {
            let taken = undos.take_entry(entry);
            Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| {
                ex.sync_undo();
                if let Some(undo) = taken {
                    ex.spawn_job(ctx, move || {
                        let bins = reprisefs::trash::Bins::system();
                        Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| {
                            let shares = &ex.shares.shares;
                            let pending =
                                crate::removing::PendingRemoval::undo(undo, erase, bins, shares);
                            ex.remove(pending, ctx);
                            Reply::redraw()
                        })
                    });
                }
                Reply::redraw()
            })
        });
        Reply::redraw()
    }

    /// The box after an undo that `e` stopped: the question whether to erase what a Recycle Bin
    /// refused, about the entry `left` of so many items that went back on the stack, else the
    /// error.
    pub(crate) fn undo_failed(
        &mut self,
        e: &std::io::Error,
        left: Option<(reprisefs::undo::EntryId, usize)>,
        ctx: &mut Ctx,
    ) -> Reply {
        let (Some(no_bin), Some((entry, items))) = (reprisefs::undo::NoBin::of(e), left) else {
            return self.message_box(
                &tr!("fileops-undo-failed-error"),
                tr!("fileops-undo-failed-cannot-undo-e", e = e.to_string()),
                ctx,
            );
        };
        let many = items > 1;
        let is_dir = no_bin.path.is_dir() && !no_bin.path.is_symlink();
        let text = no_bin.question(many);
        let kind = ConfirmKind::of(items, is_dir);
        let icon = question_icon(ctx, is_dir);
        let mut c =
            self.delete_question(kind, "", Vec::new(), true, BinLack::None, icon, None, None);
        c.text = text;
        c.undo_erase = Some(entry);
        self.raise(Dialog::Confirm(c))
    }

    /// The Undo rows from the history's head, greyed while an undo runs, a clear waits to land or
    /// an edit of this window's has yet to land. An open menu keeps the entry its row names.
    pub(crate) fn sync_undo(&mut self) {
        if self.bar.open.is_some() || self.popup.is_some() {
            return;
        }
        if self.undo_edits_pending > 0 {
            (self.undo, self.undo_entry, self.undo_held) = (None, None, false);
            return;
        }
        let head = self.undos.head();
        let held = head.and_then(|_| self.undos.held());
        self.undo_held = held == Some(Hold::Undo);
        let head = head.filter(|_| held.is_none());
        self.undo_entry = head.map(|(id, _)| id);
        self.undo = head.map(|(_, label)| label.trim_start_matches("Undo "));
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

    /// The paths a file operation on the selection acts on: [`Explorer::picked`]'s.
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.picked().map(|(_, it)| it.path.clone()).collect()
    }

    /// Every selected entry's path, a Desktop row's included, for selecting them again.
    pub(crate) fn selection_paths(&self) -> Vec<PathBuf> {
        (self.sv.selection.set.iter())
            .filter_map(|&i| self.items.get(i).map(|it| it.path.clone()))
            .collect()
    }

    /// The selected files and folders with their indices: the selection without the Desktop's
    /// rows for the shell places, which are not files to cut, delete or rename. A compressed
    /// folder's entries are not on disk, so none of them is picked.
    pub(crate) fn picked(&self) -> impl Iterator<Item = (usize, &reprisefs::listing::Item)> {
        let on_disk = !matches!(self.place, Place::Archive(..));
        (self.sv.selection.set.iter())
            .filter(move |_| on_disk)
            .filter_map(|&i| Some((i, self.items.get(i)?)))
            .filter(|(_, it)| Place::of_row(&it.path).is_none())
    }

    /// The shell place entry `i` stands for, when it is one of the Desktop's rows.
    pub(crate) fn shell_row(&self, i: usize) -> Option<Place> {
        Place::of_row(&self.items.get(i)?.path)
    }

    /// The icon of a file or folder already known to be one or the other; it reads nothing on disk.
    pub(crate) fn icon_name_as(&self, path: &Path, is_dir: bool, ctx: &mut Ctx) -> String {
        if is_dir {
            return "folder".to_string();
        }
        let mime = path
            .file_name()
            .and_then(|n| self.globs.mime_of_name(&n.to_string_lossy()));
        let has = |n: &str| ctx.icons.has_unread(n);
        reprisefs::icons::pick_icon(mime.as_deref(), path, &has)
    }
}

/// A batch whose check ([`Checked`]) is still out: where it goes and whether what lands is
/// selected, and the title of the box that says it gave up.
pub(crate) struct Clashing {
    to: (PathBuf, bool),
    job: repriseapp::listjob::Job<Checked>,
    deadline: Deadline,
    title: String,
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

impl Asking {
    /// The paste the prompt asks for and its asker.
    pub(crate) fn into_paste(self) -> (PendingPut, Asker) {
        (self.pending, self.asker)
    }
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
fn question_icon(ctx: &mut Ctx, is_dir: bool) -> IconSlot {
    let names: &[&str] = if is_dir {
        &["folder"]
    } else {
        &["text-x-generic"]
    };
    IconSlot::chrome(&mut ctx.icons, names, 32)
}
