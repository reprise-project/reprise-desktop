//! Files dropped or pasted onto a zip, or into a compressed folder of one, go into it, as do New's
//! folder, text document and compressed folder made there. Each name the zip's folder already
//! holds asks the replace prompt, a folder over a folder as a merge; then a job rewrites the zip
//! with them ([`reprisefs::zipwrite`]), its progress and Cancel in the compress dialog.

use crate::Explorer;
use crate::acting::Action;
use crate::extracting::Batch;
use crate::fileops::{New, PendingPut, file_label};
use crate::place::Place;
use repriseapp::{Ctx, Reply};
use reprisefs::archive::{self, Entry, Kind};
use reprisefs::listing::Item;
use reprisefs::ops::{DropOp, PutError};
use reprisefs::replace::{Asker, Clash, Found, Seen};
use reprisefs::zipwrite::{Adding, Made};
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

    /// The zip and its folder shown, when the place is a compressed folder of a zip that takes
    /// files ([`Explorer::zip_writes`]).
    pub(crate) fn zip_view(&self) -> Option<(&Path, &Path)> {
        match &self.place {
            Place::Archive(zip, inner) if self.zip_writes => Some((zip, inner)),
            _ => None,
        }
    }

    /// The zip and the folder of it that files put into `dir` go into: a listed zip
    /// [`Explorer::takes_into_zip`] at its top, or the compressed folder shown or a folder of it
    /// ([`Explorer::zip_view`]). None for a folder on disk.
    pub(crate) fn zip_into(&self, dir: &Path) -> Option<(PathBuf, PathBuf)> {
        if matches!(self.place, Place::Archive(..)) {
            let (zip, _) = self.zip_view()?;
            let inner = dir.strip_prefix(zip).ok()?;
            return Some((zip.to_path_buf(), inner.to_path_buf()));
        }
        (self.items.iter())
            .any(|it| it.path == dir && self.takes_into_zip(it))
            .then(|| (dir.to_path_buf(), PathBuf::new()))
    }

    /// Puts copies of `paths` into the folder `inner` of the zip `zip`: its listing and the names
    /// that folder holds are read off the loop, then the replace prompt asks about each, and the
    /// rest go in on a job. Of items sharing a name only the first goes in, and the zip's own
    /// entries and a folder holding the zip are left out; a file onto a folder the zip holds, or
    /// a folder onto a file, refuses the drop with the box a copy gives.
    pub(crate) fn put_in_zip(
        &mut self,
        paths: &[PathBuf],
        (zip, inner): (&Path, &Path),
        ctx: &mut Ctx,
    ) -> Reply {
        let mut names = std::collections::HashSet::new();
        let paths: Vec<PathBuf> = (paths.iter())
            .filter(|p| !zip.starts_with(p) && !p.starts_with(zip))
            .filter(|p| p.file_name().is_some_and(|n| names.insert(n.to_owned())))
            .cloned()
            .collect();
        if paths.is_empty() {
            return Reply::none();
        }
        let archives = self.archives.clone();
        let (at, under) = (zip.to_path_buf(), inner.to_path_buf());
        let read = {
            let paths = paths.clone();
            move |_: &reprisefs::deadline::Gate| {
                zip_clashes(&archives.entries(&at)?, &under, &paths)
            }
        };
        let (zip, inner) = (zip.to_path_buf(), inner.to_path_buf());
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
                let into = IntoZip {
                    entries: checked.entries,
                    inner,
                };
                let pending = PendingPut::into_zip(&paths, &zip, checked.folders, into);
                ex.ask_replace(pending, Asker::new(checked.clashes), ctx)
            },
        )
    }

    /// New in the compressed folder shown ([`Explorer::zip_view`]): an empty folder, text
    /// document or zip goes into that folder of the zip on a job, named past what it holds when
    /// the job runs ([`reprisefs::zipwrite::ArchiveJob::make_in_zip`]).
    pub(crate) fn new_in_zip(&mut self, kind: New, ctx: &mut Ctx) -> Reply {
        let Some((zip, inner)) = self.zip_view() else {
            return Reply::none();
        };
        let made = match kind {
            New::Folder => Made::Folder,
            New::TextDocument => Made::TextDocument,
            New::CompressedFolder => Made::Zip,
        };
        let batch = Batch::MakeInZip {
            zip: zip.to_path_buf(),
            under: inner.to_path_buf(),
            made,
        };
        self.queue_archive_job(batch, ctx);
        Reply::none()
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
        let under = (pending.zip.as_ref()).map_or_else(PathBuf::new, |z| z.inner.clone());
        let adds = (by_dir.into_iter())
            .map(|(dir, names)| Adding {
                dir,
                names,
                under: under.clone(),
                fresh: false,
            })
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

/// The zip a put goes into ([`PendingPut::zip`]): its listing and the folder of it the items go
/// into.
#[derive(Clone)]
pub(crate) struct IntoZip {
    pub entries: Arc<Vec<Entry>>,
    pub inner: PathBuf,
}

/// The clash each of `paths`, of distinct names, meets in the folder `inner` of the zip listed as
/// `entries`, and whether each is a folder: a folder over one of the zip's folders merges, a file
/// over one of its files is replaced, and one over the other kind refuses the drop. Links are not
/// followed.
pub(crate) fn zip_clashes(
    entries: &Arc<Vec<Entry>>,
    inner: &Path,
    paths: &[PathBuf],
) -> Result<ZipChecked, String> {
    let top = archive::children(entries, inner);
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
    /// Copies of `paths` going into the zip `zip` as `into` says.
    fn into_zip(paths: &[PathBuf], zip: &Path, folders: Vec<bool>, into: IntoZip) -> PendingPut {
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
        PendingPut::new(items, folders, zip.to_path_buf(), Some(into))
    }
}

/// The size and date the replace prompt shows for the name `name` in the folder of the zip `into`
/// names, as [`reprisefs::replace::facts`] words a file's.
pub(crate) fn held_facts(into: &IntoZip, name: &std::ffi::OsStr, is_12h: bool) -> (String, String) {
    let top = archive::children(&into.entries, &into.inner);
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
