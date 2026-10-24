use std::collections::HashMap;
use std::path::{Path, PathBuf};

use inotify::WatchMask;
use reprisefs::places;

use crate::{compositor::State, dirwatch::Want};

use super::scan::TRASH_MASK;

/// The drives' Recycle Bins as the last reads found them: the folders the watch adds for them
/// and whether all are empty. A drive can be a mount that never answers, so each drive is read on
/// a file job of its own.
#[derive(Default)]
pub(crate) struct DriveBins {
    pub(super) wants: Vec<Want>,
    pub(super) full: bool,
    /// Reads of the drive list, which looks at no drive.
    reads: crate::fsjobs::Reads,
    /// A read was asked for while one ran: read again when it lands.
    again: bool,
    /// The drives listed, in mount order.
    listed: Vec<PathBuf>,
    /// Each listed drive's bin folders and whether one holds something, as its last read found
    /// them.
    drives: HashMap<PathBuf, (Vec<Want>, bool)>,
    /// The drives whose bins are being read, by the drive list read that started each. A drive
    /// that never answers keeps its read, and no second one starts while it stays listed.
    drive_reads: HashMap<PathBuf, u64>,
    /// Each drive's Recycle Bin Properties tab as its last read found it, None for no bin.
    tabs: HashMap<PathBuf, Option<crate::binprops::Drive>>,
    /// The drives whose tab is being read, by the tab read that started each. A drive that never
    /// answers keeps its read, and no second one starts while it stays listed.
    tab_reads: HashMap<PathBuf, u64>,
    /// The last tab read's id.
    tab_read: u64,
}

impl DriveBins {
    /// A new read's id, or None while one runs, which then reads again when it lands.
    pub(super) fn start(&mut self, now: std::time::Instant) -> Option<u64> {
        let id = self.reads.start(now, ());
        self.again = id.is_none();
        id
    }

    /// Takes read `id`'s drive list `tops`: whether it changed the bins (a stale answer changes
    /// nothing), whether to read again, and the drives to read, now counted as read by `id`.
    pub(super) fn land(&mut self, id: u64, tops: Vec<PathBuf>) -> (bool, bool, Vec<PathBuf>) {
        let again = std::mem::take(&mut self.again);
        if !self.reads.land(id) {
            return (false, again, Vec::new());
        }
        self.drives.retain(|t, _| tops.contains(t));
        self.drive_reads.retain(|t, _| tops.contains(t));
        let read = (tops.iter())
            .filter(|t| !self.drive_reads.contains_key(*t))
            .cloned()
            .collect::<Vec<_>>();
        (self.drive_reads).extend(read.iter().map(|t| (t.clone(), id)));
        self.listed = tops;
        (self.sum(), again, read)
    }

    /// Takes drive `top`'s read, started by list read `id`: whether it changed the bins. A read
    /// of a drive that left the list meanwhile changes nothing.
    pub(super) fn land_drive(
        &mut self,
        top: PathBuf,
        id: u64,
        wants: Vec<Want>,
        full: bool,
    ) -> bool {
        if self.drive_reads.get(&top) != Some(&id) {
            return false;
        }
        self.drive_reads.remove(&top);
        self.drives.insert(top, (wants, full));
        self.sum()
    }

    /// Joins the drives' answers in mount order: whether that changed the bins.
    fn sum(&mut self) -> bool {
        let found = || self.listed.iter().filter_map(|t| self.drives.get(t));
        let wants: Vec<Want> = found().flat_map(|(w, _)| w.iter().cloned()).collect();
        let full = found().any(|(_, f)| *f);
        if self.wants == wants && self.full == full {
            return false;
        }
        self.wants = wants;
        self.full = full;
        true
    }

    /// The tabs of `tops` the reads have found so far, in `tops`' order.
    pub(crate) fn tabs(&self, tops: &[PathBuf]) -> Vec<crate::binprops::Drive> {
        (tops.iter())
            .filter_map(|t| self.tabs.get(t).cloned().flatten())
            .collect()
    }

    /// A new tab read's id and the drives of `tops` it reads: those with no read out, now counted
    /// as read by it.
    pub(crate) fn start_tab_reads(&mut self, tops: &[PathBuf]) -> (u64, Vec<PathBuf>) {
        self.tab_read += 1;
        let id = self.tab_read;
        let read = (tops.iter())
            .filter(|t| !self.tab_reads.contains_key(*t))
            .cloned()
            .collect::<Vec<_>>();
        (self.tab_reads).extend(read.iter().map(|t| (t.clone(), id)));
        (id, read)
    }

    /// Takes drive `top`'s tab from tab read `id`. A read of a drive that left the list meanwhile
    /// changes nothing.
    pub(crate) fn land_tab(&mut self, top: PathBuf, id: u64, tab: Option<crate::binprops::Drive>) {
        if self.tab_reads.get(&top) != Some(&id) {
            return;
        }
        self.tab_reads.remove(&top);
        self.tabs.insert(top, tab);
    }

    /// Drops the tabs and tab reads of drives no longer in `tops`, the drives listed now.
    pub(crate) fn drives_listed(&mut self, tops: &[PathBuf]) {
        self.tabs.retain(|t, _| tops.contains(t));
        self.tab_reads.retain(|t, _| tops.contains(t));
    }
}

/// The drives `bins` may hold a Recycle Bin on, read without looking at any.
pub(super) fn drive_tops(bins: &reprisefs::trash::Bins) -> Vec<PathBuf> {
    (bins.mounts.iter())
        .filter(|m| !bins.unlisted.contains(*m))
        .cloned()
        .collect()
}

/// Drive `top`'s [`drive_trash_wants`] for the folders there now, and whether its bin holds
/// something.
pub(super) fn read_drive_bin(bins: &reprisefs::trash::Bins, top: &Path) -> (Vec<Want>, bool) {
    let wants = drive_trash_wants(bins, top)
        .into_iter()
        .filter(|w| w.dir.is_dir())
        .collect();
    let full = (bins.drive_roots(top).iter()).any(|r| !places::trash_empty(&r.join("files")));
    (wants, full)
}

/// Drive `top`'s trashes, and the top itself for its trash being made; an eject drops the
/// watch, which is a change. A top the user cannot read (`/root` on its own subvolume) holds no
/// trash of theirs and is left out.
pub(super) fn drive_trash_wants(bins: &reprisefs::trash::Bins, top: &Path) -> Vec<Want> {
    let mut want = Vec::new();
    if readable(top) {
        let names = [".Trash".into(), format!(".Trash-{}", bins.uid).into()];
        want.push(Want {
            dir: top.to_path_buf(),
            mask: WatchMask::CREATE | WatchMask::MOVED_TO,
            only: names.to_vec(),
        });
    }
    for root in bins.drive_roots(top) {
        want.push(Want {
            dir: root.join("files"),
            mask: TRASH_MASK,
            only: Vec::new(),
        });
        want.push(Want {
            dir: root,
            mask: TRASH_MASK,
            only: Vec::new(),
        });
    }
    want
}

/// Whether the user may list `dir`, which inotify needs to watch it.
pub(super) fn readable(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `c` is a valid NUL-terminated path.
    unsafe { libc::access(c.as_ptr(), libc::R_OK) == 0 }
}

impl State {
    /// Scans the mounted drives on a file job for the Send To submenu, or once more after the
    /// scan under way.
    pub(super) fn scan_drives(&mut self) {
        let Some(mut scanner) = self.desktop.as_mut().and_then(|v| v.drives.start()) else {
            return;
        };
        self.spawn_fs_job(move || {
            let listed = scanner.listed().to_vec();
            Box::new(move |state: &mut State| {
                let Some(view) = state.desktop.as_mut() else {
                    return;
                };
                let before = view.drives.found().to_vec();
                let (again, stat) = view.drives.land(scanner, listed);
                if view.drives.found() != before {
                    view.bins.drives_listed(view.drives.found());
                    state.drives_changed();
                }
                for top in stat {
                    state.stat_drive(top, |_, _| {});
                }
                if again {
                    state.scan_drives();
                }
            })
        });
    }

    /// Stats each listed drive again, as a menu holding Send To opens: one that stopped answering
    /// greys its row.
    pub(crate) fn restat_drives(&mut self) {
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        for top in view.drives.restat() {
            self.stat_drive(top, |_, _| {});
        }
    }

    /// Stats drive `top` (its stat already counted as out) on a file job, then runs `then` with
    /// whether it is usable. A change to the drives shows in an open Recycle Bin Properties.
    pub(super) fn stat_drive(
        &mut self,
        top: PathBuf,
        then: impl FnOnce(&mut State, bool) + Send + 'static,
    ) {
        self.spawn_fs_job(move || {
            let usable = places::usable_drive(&top);
            Box::new(move |state: &mut State| {
                let Some(view) = state.desktop.as_mut() else {
                    return;
                };
                if view.drives.land_stat(top, usable) {
                    state.drives_changed();
                }
                then(state, usable);
            })
        });
    }

    /// Lists the drives on a file job, then reads each drive's Recycle Bins on a file job of its
    /// own; an answer that changes them rescans.
    pub(super) fn read_drive_bins(&mut self) {
        let now = std::time::Instant::now();
        let Some(id) = (self.desktop.as_mut()).and_then(|v| v.bins.start(now)) else {
            return;
        };
        self.spawn_fs_job(move || {
            let bins = reprisefs::trash::Bins::system();
            Box::new(move |state: &mut State| {
                let tops = bins.as_ref().map(drive_tops).unwrap_or_default();
                let Some(view) = state.desktop.as_mut() else {
                    return;
                };
                let (changed, again, read) = view.bins.land(id, tops);
                if let Some(bins) = bins {
                    for top in read {
                        state.read_drive_bin(bins.clone(), top, id);
                    }
                }
                if changed {
                    state.rescan_desktop();
                } else if again {
                    state.read_drive_bins();
                }
            })
        });
    }

    fn read_drive_bin(&mut self, bins: reprisefs::trash::Bins, top: PathBuf, id: u64) {
        self.spawn_fs_job(move || {
            let (wants, full) = read_drive_bin(&bins, &top);
            Box::new(move |state: &mut State| {
                let Some(view) = state.desktop.as_mut() else {
                    return;
                };
                if view.bins.land_drive(top, id, wants, full) {
                    state.rescan_desktop();
                }
            })
        });
    }
}
