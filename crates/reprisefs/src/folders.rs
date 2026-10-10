//! The handle table a file dialog host resolves folder ids through.
//! `reprisekit::controls::filedlg` only carries an opaque `u64` for a
//! folder, so this table is the whole file-system boundary: the host
//! mints an id per path it lists and resolves the ids the dialog hands
//! back.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FolderTable {
    paths: Vec<PathBuf>,
    index: HashMap<PathBuf, u64>,
}

impl FolderTable {
    pub fn id(&mut self, p: &Path) -> u64 {
        if let Some(&i) = self.index.get(p) {
            return i;
        }
        let i = self.paths.len() as u64;
        self.paths.push(p.to_path_buf());
        self.index.insert(p.to_path_buf(), i);
        i
    }

    pub fn path(&self, id: u64) -> Option<&Path> {
        self.paths.get(id as usize).map(PathBuf::as_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_per_path() {
        let mut t = FolderTable::default();
        let x = t.id(Path::new("/x"));
        assert_eq!(x, 0);
        assert_eq!(t.id(Path::new("/x")), x);
        assert_eq!(t.id(Path::new("/y")), 1);
        assert_eq!(t.path(x), Some(Path::new("/x")));
        assert_eq!(t.path(9), None);
    }
}
