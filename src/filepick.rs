//! The compositor's host for `reprisekit::controls::filedlg`: the Browse
//! dialog for Run or Open With (see `Purpose`). The picker
//! owns the `FileDlg` model, the `reprisefs::folders::FolderTable` its
//! `FolderId`s resolve through, and the icons the host looked up; a places
//! tile resolves through `place_dirs`. `dialog.rs` frames it, routes keys
//! and clicks to it and draws it into the frame's client area.
//!
//! The picker never touches the file system itself: a folder read or a
//! typed name's lookup can block for good on a dead network mount. It
//! queues a [`Request`] instead; the host runs it off the loop and hands
//! the [`Reply`] back through [`FilePicker::reply`]. Thumbnails, of pictures
//! and of files an installed thumbnailer takes, come the same way:
//! [`FilePicker::take_pictures`], then [`FilePicker::thumbs`].

use crate::raster::Canvas;
use reprisefs::listing::Row;
use reprisekit::controls::filedlg::{self, Event, Fields, FileDlg, Folder, FolderId};
use std::path::{Path, PathBuf};

mod icons;
mod input;
mod listing;
mod run;
use icons::tile_places;
pub use icons::{Icons, icons};

pub const TOOLBAR: [&str; 4] = ["go-previous", "go-up", "folder-new", "view-list"];
/// Few themes carry `view-list`: without it Views gets the drawn icon at the dialog's scale.
const TOOLBAR_VIEWS: usize = 3;

/// What the picker is for: which filters it offers, which files it lists,
/// and where the pick goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Run's Browse...: programs (and every file behind the
    /// second filter); the pick lands quoted in Run's field.
    RunCommand,
    /// The Open With chooser's Browse...: the same listing, the pick
    /// joining the chooser's list.
    Program,
}

impl Purpose {
    fn fields(self) -> Fields {
        let filters = vec!["Programs".into(), "All Files".into()];
        Fields {
            title: "Browse".into(),
            accept_label: "Open".into(),
            filters,
            mode: filedlg::DlgMode::Open,
            multiple: false,
        }
    }

    /// The files the current filter lists: None is everything.
    fn exts(self, filter: usize) -> Option<&'static [&'static str]> {
        match (self, filter) {
            // "Programs" is what the shell would run: executable files, judged by
            // mode rather than by extension (`listing::Item::exec`).
            (Purpose::RunCommand | Purpose::Program, 0) => Some(&[]),
            (Purpose::RunCommand | Purpose::Program, _) => None,
        }
    }
}

/// The Browse dialog: the dialog, its folder table and icons.
#[derive(Clone, Debug, PartialEq)]
pub struct FilePicker {
    pub dlg: FileDlg,
    purpose: Purpose,
    /// Where each places tile points; None is a greyed tile.
    place_dirs: Vec<Option<PathBuf>>,
    /// The pinned folders the tiles after the built-in ones show, and whether My Recent
    /// Documents is live, to build the tiles again from.
    pins: Vec<reprisefs::pins::Pin>,
    recent_tile: bool,
    folders: reprisefs::folders::FolderTable,
    icons: Icons,
    /// The Files of type filter the listing was built for; a change
    /// relists the folder.
    filter: usize,
    /// Requests made since the host last took them.
    requests: Vec<Request>,
    /// Requests made and not yet answered. A request equal to one of them takes its number
    /// instead of another job, so relisting a folder that never answers strands no more threads.
    pending: Vec<Request>,
    /// The last request's number: every request takes the next.
    seq: u64,
    /// The folder listing still wanted and the typed name still being
    /// looked up, by request number. A reply for anything else is stale.
    listing: Option<u64>,
    opening: Option<u64>,
    /// The relisting of the folder shown that only gives its entries their icons at the
    /// dialog's new scale or icon theme ([`FilePicker::follow_scale`]).
    restyling: Option<u64>,
    /// My Recent Documents' files by the name each is listed as, while
    /// that listing is on screen.
    recent: std::collections::HashMap<String, PathBuf>,
    pictures: Pictures,
    /// The view and sort last stored, and the file they live in once
    /// [`FilePicker::remember_in`] named it.
    remembered: filedlg::Remembered,
    view_file: Option<PathBuf>,
    /// The stored view is still to come with the first listing, and no view was picked yet.
    view_awaited: bool,
    /// Watches the folder shown, so entries made, deleted or renamed there list without an F5
    /// ([`FilePicker::watch_due`]); `watched` is the folder it is on.
    watch: FolderWatch,
    watched: Option<PathBuf>,
    /// New Folder, renames and Deletes asked for since the host last took them ([`Op::run`]).
    ops: Vec<Op>,
}

/// Work on the folder shown that the host runs off the loop with [`Op::run`].
#[derive(Clone, Debug, PartialEq)]
pub struct Op {
    dir: PathBuf,
    /// The folder's names as listed.
    taken: Vec<String>,
    work: Work,
}

#[derive(Clone, Debug, PartialEq)]
enum Work {
    NewFolder(String),
    Rename {
        from: PathBuf,
        old: String,
        typed: String,
    },
}

/// What an [`Op`] did, for [`FilePicker::op_done`].
#[derive(Debug)]
pub enum OpDone {
    /// A folder made, or an entry renamed: its name, and whether it opens in the rename box.
    Named(String, bool),
    /// A rename refused: the entry, the name typed and why, worded for the user.
    Refused {
        old: String,
        typed: String,
        why: String,
    },
    Nothing,
}

/// A picker's folder watch. Not dialog state: a clone watches nothing, it compares equal and
/// debug-prints as nothing.
#[derive(Default)]
struct FolderWatch(Option<reprisefs::folderwatch::Watch>);

impl Clone for FolderWatch {
    fn clone(&self) -> FolderWatch {
        FolderWatch(None)
    }
}

impl PartialEq for FolderWatch {
    fn eq(&self, _: &FolderWatch) -> bool {
        true
    }
}

impl std::fmt::Debug for FolderWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FolderWatch")
    }
}

/// The listed files with thumbnails ([`reprisefs::listing::Item::thumbnailable`]) and how far
/// their thumbnails have got.
#[derive(Clone, Debug, Default, PartialEq)]
struct Pictures {
    /// Picture `i`'s path; its entry's id is `i + 1`.
    paths: Vec<PathBuf>,
    /// Bumped by every listing, so thumbnails for an earlier one are dropped.
    generation: u64,
    /// This listing's pictures went to the host.
    asked: bool,
    /// The scale the dialog drew at when listed: the thumbnails' device pixels.
    scale: i32,
}

/// File system work the picker is waiting on, for the host to run off
/// the loop with [`Request::run`].
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    seq: u64,
    /// The folder to list, or None to look `path` up first: a typed name
    /// is a folder to enter, a file to pick, or nothing.
    id: Option<FolderId>,
    path: PathBuf,
    exts: Option<&'static [&'static str]>,
    /// The pattern typed into File name, which lists in place of `exts`.
    pattern: Option<String>,
    /// The lookup is a typed pattern's folder part: a folder is entered
    /// and the dialog takes `pattern`; anything else is no pick.
    folder_only: bool,
    /// The dialog's draw scale, at which icon files the listing names are decoded.
    scale: i32,
    /// List and Details' icon size ([`shellview::list_icon_px`]).
    list: i32,
    /// The view file ([`FilePicker::remember_in`]) the first listing reads along with its
    /// folder, so a dead `$XDG_STATE_HOME` never stalls the loop.
    view_file: Option<PathBuf>,
}

/// A listed row and, for a folder, its own icon (`.directory`'s `Icon=`), read on the job's
/// thread: a subfolder may be a mount point that never answers.
#[derive(Debug)]
struct Listed {
    row: Row,
    own_icon: Option<String>,
}

/// What a [`Request`] found, for [`FilePicker::reply`].
#[derive(Debug)]
pub struct Reply {
    seq: u64,
    /// The scale and List and Details' icon size its icons were made for.
    scale: i32,
    list: i32,
    id: Option<FolderId>,
    path: PathBuf,
    /// The typed pattern whose folder part was looked up, which the
    /// dialog takes once that is a folder.
    pattern: Option<String>,
    found: Found,
    /// Icon files the listing's rows wear (a folder's or a launcher's `Icon=/...`), decoded at
    /// [`icontheme::listing_px`] of `list` times `scale` on the job's thread: the file may be
    /// anywhere, a dead mount included.
    art: Vec<(String, [Option<Canvas>; 3])>,
    /// The stored view and sort, when the request carried the view file.
    view: Option<filedlg::Remembered>,
}

#[derive(Debug)]
enum Found {
    /// `listing::Row` rather than `Entry`, which holds `Rc` icons and so
    /// cannot come back from the thread that read the folder.
    Folder(Vec<Listed>),
    File,
    Missing,
}

impl FilePicker {
    /// Run's Browse..., listing `start` (the home folder). Its Open puts
    /// the file in Run's field.
    pub fn run_command(start: &Path, icons: Icons, recent: bool) -> FilePicker {
        FilePicker::new(Purpose::RunCommand, start, icons, recent)
    }

    /// The Open With chooser's Browse..., listing `start` (`/usr/bin`).
    /// Its Open adds the program to the chooser.
    pub fn program(start: &Path, icons: Icons, recent: bool) -> FilePicker {
        FilePicker::new(Purpose::Program, start, icons, recent)
    }

    /// `recent` is `[privacy] recent_files`: without it the My Recent
    /// Documents tile is greyed.
    fn new(purpose: Purpose, start: &Path, mut icons: Icons, recent: bool) -> FilePicker {
        icons.scale = icons.scale.max(1);
        let tiles = reprisefs::places::dialog_places(recent);
        let places = tile_places(&tiles, &icons);
        let mut dlg = FileDlg::new(purpose.fields(), places);
        let tb = |i: usize| icons.toolbar.get(i).cloned().flatten();
        dlg.set_toolbar_icons(tb(0), tb(1), tb(2), tb(3));
        let mut picker = FilePicker {
            dlg,
            purpose,
            place_dirs: tiles.into_iter().map(|p| p.dir).collect(),
            pins: Vec::new(),
            recent_tile: recent,
            folders: reprisefs::folders::FolderTable::default(),
            icons,
            filter: 0,
            requests: Vec::new(),
            pending: Vec::new(),
            seq: 0,
            listing: None,
            opening: None,
            restyling: None,
            recent: Default::default(),
            pictures: Pictures::default(),
            remembered: Default::default(),
            view_file: None,
            view_awaited: false,
            watch: FolderWatch::default(),
            watched: None,
            ops: Vec::new(),
        };
        let tiles = picker.place_dirs.iter();
        let folders = tiles
            .map(|d| d.as_ref().map(|d| picker.folders.id(d)))
            .collect();
        picker.dlg.set_place_folders(folders);
        let id = picker.folders.id(start);
        picker.list(id);
        picker
    }

    pub fn current(&self) -> Option<&Path> {
        self.dlg.folder().and_then(|id| self.folders.path(id))
    }

    fn tiles(&self) -> Vec<reprisefs::places::DialogPlace> {
        reprisefs::places::dialog_places_with(self.recent_tile, &self.pins)
    }

    /// Shows `pins` after the built-in tiles; true when they changed.
    pub fn set_pins(&mut self, pins: &[reprisefs::pins::Pin]) -> bool {
        if self.pins == pins {
            return false;
        }
        self.pins = pins.to_vec();
        let tiles = self.tiles();
        let folders = (tiles.iter())
            .map(|p| p.dir.as_ref().map(|d| self.folders.id(d)))
            .collect();
        self.dlg
            .set_places(tile_places(&tiles, &self.icons), folders);
        self.place_dirs = tiles.into_iter().map(|p| p.dir).collect();
        self.mark_pinned();
        true
    }

    /// Tells the dialog which of the shown folder's entries are pinned.
    fn mark_pinned(&mut self) {
        let Some(dir) = self.current().map(Path::to_path_buf) else {
            return;
        };
        let names = (self.dlg.entries().iter())
            .filter(|e| e.is_dir && reprisefs::pins::is_pinned(&self.pins, &dir.join(&e.name)))
            .map(|e| e.name.clone())
            .collect();
        self.dlg.set_pinned_entries(names);
    }

    /// Pin to Places, Unpin from Places and Move Up/Down, written off the loop; the bookmarks
    /// watch brings the change back to every dialog.
    fn pin(&self, ev: Event) {
        let tile = |i: usize| self.tiles().into_iter().nth(i).and_then(|p| p.pin);
        let file = reprisefs::pins::bookmarks_file();
        let write: Box<dyn FnOnce() + Send> = match ev {
            Event::PinEntry { name, pin } => {
                let Some(dir) = self.current().map(|d| d.join(name)) else {
                    return;
                };
                Box::new(move || reprisefs::pins::set_pinned(&file, &dir, pin))
            }
            Event::UnpinPlace(i) => {
                let Some(dir) = tile(i) else { return };
                Box::new(move || reprisefs::pins::set_pinned(&file, &dir, false))
            }
            Event::ShiftPlace { tile: i, up } => {
                let Some(dir) = tile(i) else { return };
                Box::new(move || reprisefs::pins::shift(&file, &dir, up))
            }
            _ => return,
        };
        crate::state::run_in_order(write);
    }
}

#[cfg(test)]
mod tests;
