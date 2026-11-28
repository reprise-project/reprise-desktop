//! Files dropped or pasted onto a zip go into it. Each name the zip's top already holds asks the
//! replace prompt, a folder over a folder as a merge; then a job rewrites the zip with them
//! ([`reprisefs::zipwrite`]), its progress and Cancel in the compress dialog.

use crate::Explorer;
use crate::acting::Action;
use crate::extracting::Batch;
use crate::fileops::{PendingPut, file_label};
use repriseapp::{Ctx, Reply};
use reprisefs::archive::{self, Entry, Kind};
use reprisefs::listing::Item;
use reprisefs::ops::{DropOp, PutError};
use reprisefs::replace::{Asker, Clash, Found, Seen};
use reprisefs::zipwrite::Adding;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What the check of a drop onto a zip read off the loop.
pub(crate) struct ZipChecked {
    entries: Arc<Vec<Entry>>,
    clashes: Vec<Option<Found>>,
    folders: Vec<bool>,
    /// The first item whose name the zip holds as the other kind, a file for a folder or a
    /// folder for a file, which refuses the drop.
    refused: Option<PathBuf>,
}

impl Explorer {
    /// Whether the listed `it` is a zip that files dropped or pasted onto go into: a zip of the
    /// user's that takes writes and opens as a compressed folder, in a folder that takes writes.
    /// A drop onto any other zip is refused.
    pub(crate) fn takes_into_zip(&self, it: &Item) -> bool {
        !it.is_dir
            && !it.is_link
            && it.takes_writes
            && it.mime.as_deref() == Some("application/zip")
            && !self.place.read_only()
            && archive::opens_as_folder("application/zip", &self.app_index)
    }

    /// Whether `path` is a listed zip [`Explorer::takes_into_zip`].
    pub(crate) fn is_zip_target(&self, path: &Path) -> bool {
        (self.items.iter()).any(|it| it.path == path && self.takes_into_zip(it))
    }

    /// Puts copies of `paths` into the zip `zip`: its listing and the names it holds are read off
    /// the loop, then the replace prompt asks about each, and the rest go in on a job. Of items
    /// sharing a name only the first goes in; a file onto a folder the zip holds, or a folder
    /// onto a file, refuses the drop with the box a copy gives.
    pub(crate) fn put_in_zip(&mut self, paths: &[PathBuf], zip: &Path, ctx: &mut Ctx) -> Reply {
        let mut names = std::collections::HashSet::new();
        let paths: Vec<PathBuf> = (paths.iter())
            .filter(|p| !zip.starts_with(p))
            .filter(|p| p.file_name().is_some_and(|n| names.insert(n.to_owned())))
            .cloned()
            .collect();
        if paths.is_empty() {
            return Reply::none();
        }
        let archives = self.archives.clone();
        let at = zip.to_path_buf();
        let read = {
            let paths = paths.clone();
            move |_: &reprisefs::deadline::Gate| zip_clashes(&archives.entries(&at)?, &paths)
        };
        let zip = zip.to_path_buf();
        let title = reprisefs::compress::failure_title();
        self.act(
            ctx,
            Action::reads(title.clone()),
            read,
            move |ex, read, ctx| {
                let checked = match read {
                    Ok(c) => c,
                    Err(why) => {
                        let text = reprisefs::zipwrite::failure(&zip, &why);
                        return ex.message_box(&title, text, ctx);
                    }
                };
                if let Some(p) = &checked.refused {
                    let (title, text) = DropOp::Copy.failure(&file_label(p), &PutError::Occupied);
                    return ex.message_box(&title, text, ctx);
                }
                let pending = PendingPut::into_zip(&paths, &zip, checked.folders, checked.entries);
                ex.ask_replace(pending, Asker::new(checked.clashes), ctx)
            },
        )
    }

    /// Starts the job adding what is left of `pending` after its prompts into its zip.
    pub(crate) fn start_zip_add(&mut self, pending: PendingPut, ctx: &Ctx) {
        let mut by_dir: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
        for it in &pending.items {
            let (Some(dir), Some(name)) = (it.src.parent(), it.src.file_name()) else {
                continue;
            };
            by_dir.entry(dir.into()).or_default().push(name.into());
        }
        let adds = (by_dir.into_iter())
            .map(|(dir, names)| Adding { dir, names })
            .collect();
        self.queue_archive_job(
            Batch::AddToZip {
                zip: pending.dir,
                adds,
            },
            ctx,
        );
    }
}

/// The clash each of `paths`, of distinct names, meets at the top of the zip listed as
/// `entries`, and whether each is a folder: a folder over one of the zip's folders merges, a file
/// over one of its files is replaced, and one over the other kind refuses the drop. Links are not
/// followed.
pub(crate) fn zip_clashes(
    entries: &Arc<Vec<Entry>>,
    paths: &[PathBuf],
) -> Result<ZipChecked, String> {
    let top = archive::children(entries, Path::new(""));
    let folders: Vec<bool> = (paths.iter())
        .map(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()))
        .collect();
    let mut refused = None;
    let clashes = (paths.iter().zip(&folders))
        .map(|(p, &folder)| {
            let name = p.file_name()?;
            let held = top.iter().find(|c| c.name == name)?;
            let clash = match (folder, held.kind == Kind::Folder) {
                (true, true) => Clash::Folder,
                (false, false) => Clash::File,
                _ => {
                    refused.get_or_insert_with(|| p.clone());
                    return None;
                }
            };
            Some(Found {
                clash,
                seen: Seen::default(),
                inside: Vec::new(),
            })
        })
        .collect();
    Ok(ZipChecked {
        entries: entries.clone(),
        clashes,
        folders,
        refused,
    })
}

impl PendingPut {
    /// Copies of `paths` going into the zip `zip`, listed as `entries`.
    fn into_zip(
        paths: &[PathBuf],
        zip: &Path,
        folders: Vec<bool>,
        entries: Arc<Vec<Entry>>,
    ) -> PendingPut {
        let items = (paths.iter())
            .map(|src| reprisefs::jobs::PutItem {
                display_name: file_label(src),
                src: src.clone(),
                is_app: false,
                op: DropOp::Copy,
                shortcut: Box::new(|_| None),
                replace: None,
            })
            .collect();
        PendingPut::new(items, folders, zip.to_path_buf(), Some(entries))
    }
}

/// The size and date the replace prompt shows for the name `name` at the top of the zip listed
/// as `entries`, as [`reprisefs::replace::facts`] words a file's.
pub(crate) fn held_facts(
    entries: &[Entry],
    name: &std::ffi::OsStr,
    is_12h: bool,
) -> (String, String) {
    let top = archive::children(entries, Path::new(""));
    match top.iter().find(|c| c.name == name) {
        Some(c) => (
            reprisefs::replace::size_text(c.size),
            (c.modified)
                .map(|t| reprisefs::replace::modified_text(t, is_12h))
                .unwrap_or_default(),
        ),
        None => (String::new(), String::new()),
    }
}
