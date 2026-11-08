//! Requests and folder work as the host runs them off the loop.

use super::icons::icon_files;
use super::*;
use crate::filedlggeom::OWN_APP;

impl Op {
    /// Does the work. Blocks for as long as the file system does.
    pub fn run(self) -> OpDone {
        use reprisefs::dialogops;
        match self.work {
            Work::NewFolder(base) => match dialogops::new_folder(&self.dir, &base, &self.taken) {
                Ok(name) => OpDone::Named(name, true),
                Err(e) => {
                    log::warn!("browse: no new folder in {}: {e}", self.dir.display());
                    OpDone::Nothing
                }
            },
            Work::Rename { from, old, typed } => {
                match dialogops::rename(&from, &old, &typed, &self.taken) {
                    Ok(name) => OpDone::Named(name, false),
                    Err(why) => OpDone::Refused { old, typed, why },
                }
            }
        }
    }
}

impl Request {
    /// Asks for the same file system work, whatever its number.
    pub(super) fn same_work(&self, o: &Request) -> bool {
        self.id == o.id
            && self.scale == o.scale
            && self.list == o.list
            && self.path == o.path
            && self.exts == o.exts
            && self.pattern == o.pattern
            && self.folder_only == o.folder_only
    }

    /// Reads the folder or looks the name up. Blocks for as long as the
    /// file system does.
    pub fn run(self) -> Reply {
        let pattern = self.pattern.as_deref();
        let found = if self.id.is_some() {
            Found::Folder(read_folder(&self.path, self.exts, pattern))
        } else {
            match std::fs::metadata(&self.path) {
                Ok(m) if m.is_dir() => Found::Folder(read_folder(&self.path, self.exts, pattern)),
                Ok(m) if m.is_file() && !self.folder_only => Found::File,
                _ => Found::Missing,
            }
        };
        let art = match &found {
            Found::Folder(rows) => icon_files(rows, self.scale, self.list),
            _ => Vec::new(),
        };
        let view = (self.view_file.as_deref())
            .map(repriseconf::filedialog::load_from)
            .map(|t| filedlg::Remembered::parse(&repriseconf::filedialog::section(&t, OWN_APP)));
        Reply {
            view,
            seq: self.seq,
            scale: self.scale,
            list: self.list,
            id: self.id,
            path: self.path,
            pattern: self.folder_only.then_some(self.pattern).flatten(),
            found,
            art,
        }
    }
}

/// The shared-mime-info globs every listing's Type column reads, parsed
/// on the first listing and kept with their description memo for the
/// session.
static GLOBS: std::sync::LazyLock<reprisefs::apps::MimeDb> =
    std::sync::LazyLock::new(reprisefs::apps::mime_globs);

/// `dir`'s subfolders and its files: those a typed `pattern` keeps,
/// else those with an extension in `exts`, the executable ones for an
/// empty `exts`, all of them for `None`, as `listing::dialog_rows` orders
/// them.
pub(super) fn read_folder(dir: &Path, exts: Option<&[&str]>, pattern: Option<&str>) -> Vec<Listed> {
    let rows = reprisefs::listing::dialog_rows(dir, &GLOBS, |i| {
        i.is_dir
            || match (pattern, exts) {
                (Some(p), _) => filedlg::pattern_keeps(p, &i.name),
                (None, None) => true,
                (None, Some([])) => i.exec,
                (None, Some(exts)) => has_ext(&i.path, exts),
            }
    });
    rows.into_iter()
        .map(|row| {
            let own_icon = reprisefs::icons::own_icon(&row.item);
            Listed { row, own_icon }
        })
        .collect()
}

fn has_ext(p: &Path, exts: &[&str]) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
}
