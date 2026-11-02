//! What the window shows of a scan: a folder's entries as rows, each with its share of the
//! folder, and the status line.

use reprisefs::props::{short_size, thousands as count};
use reprisefs::usage::{Kind, Node, NodeId, Sort, Tree};

/// The columns, by index.
pub const NAME: usize = 0;
pub const SIZE: usize = 1;
pub const SHARE: usize = 2;
pub const FILES: usize = 3;

/// One entry of the folder shown.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub id: NodeId,
    pub name: String,
    pub kind: Kind,
    pub size: u64,
    pub files: u64,
    /// Its part of the folder's size, 0 to 1.
    pub share: f64,
    /// It and everything under it is counted.
    pub done: bool,
}

impl Line {
    pub fn cell(&self, col: usize) -> String {
        match col {
            NAME => self.name.clone(),
            SIZE if self.kind == Kind::Mount => "Another drive".into(),
            SIZE => short_size(self.size),
            FILES if self.kind == Kind::Dir => count(self.files),
            _ => String::new(),
        }
    }

    pub fn opens(&self) -> bool {
        matches!(self.kind, Kind::Dir | Kind::Mount)
    }
}

/// `size` of `total` as a bar's share, 0 for an empty folder.
pub fn share(size: u64, total: u64) -> f64 {
    match total {
        0 => 0.0,
        t => size as f64 / t as f64,
    }
}

/// Folder `at`'s entries in `sort` order.
pub fn lines(t: &Tree, at: NodeId, sort: Sort, flipped: bool) -> Vec<Line> {
    let total = t.node(at).size;
    (t.sorted(at, sort, flipped).into_iter())
        .map(|id| {
            let n: &Node = t.node(id);
            Line {
                id,
                name: n.name.to_string_lossy().into_owned(),
                kind: n.kind,
                size: n.size,
                files: n.files,
                share: share(n.size, total),
                done: n.done(),
            }
        })
        .collect()
}

/// The column a header click sorts by, None for the bar's.
pub fn sort_of(col: usize) -> Option<Sort> {
    match col {
        NAME => Some(Sort::Name),
        SIZE => Some(Sort::Size),
        FILES => Some(Sort::Files),
        _ => None,
    }
}

pub fn col_of(sort: Sort) -> usize {
    match sort {
        Sort::Name => NAME,
        Sort::Size => SIZE,
        Sort::Files => FILES,
    }
}

/// Where the scan stands, said of folder `at`.
pub fn status(t: &Tree, at: NodeId, answered: bool, waited_out: bool) -> String {
    let n = t.node(at);
    let noun = if n.files == 1 { "file" } else { "files" };
    let sized = format!("{} in {} {noun}", short_size(n.size), count(n.files));
    match () {
        _ if !answered && waited_out => "The location is not responding.".into(),
        _ if n.unreadable => "The folder cannot be read.".into(),
        _ if n.done() => sized,
        _ => format!("Counting... {sized} so far"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_share_is_the_entrys_part_of_its_folder() {
        assert_eq!(share(25, 100), 0.25);
        assert_eq!(share(100, 100), 1.0);
        assert_eq!(share(0, 0), 0.0, "an empty folder draws empty bars");
    }

    #[test]
    fn rows_list_the_folder_largest_first_with_their_shares() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir(r.join("big")).unwrap();
        std::fs::write(r.join("big/a"), vec![1u8; 96 * 1024]).unwrap();
        std::fs::write(r.join("small"), vec![1u8; 4096]).unwrap();
        let s = reprisefs::usage::Scan::start(r, 2, || {});
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !s.done() {
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let t = s.tree();
        let rows = lines(&t, reprisefs::usage::ROOT, Sort::Size, false);
        let names: Vec<&str> = rows.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["big", "small"]);
        let sum: f64 = rows.iter().map(|l| l.share).sum();
        assert!((sum - 1.0).abs() < 1e-9, "the shares make up the folder");
        assert!(rows[0].share > rows[1].share);
        assert_eq!(rows[0].cell(FILES), "1");
        assert_eq!(rows[1].cell(FILES), "", "a file has no count");
        assert!(rows[0].opens() && !rows[1].opens());
        assert_eq!(
            status(&t, reprisefs::usage::ROOT, true, false),
            format!("{} in 2 files", short_size(t.node(0).size))
        );
    }

    fn counted(root: &std::path::Path) -> reprisefs::usage::Scan {
        let s = reprisefs::usage::Scan::start(root, 1, || {});
        while !s.done() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        s
    }

    #[test]
    fn the_status_says_when_the_folder_does_not_answer() {
        let d = tempfile::tempdir().unwrap();
        let s = counted(&d.path().join("gone"));
        let t = s.tree();
        assert_eq!(status(&t, 0, true, false), "The folder cannot be read.");
        assert_eq!(
            status(&t, 0, false, true),
            "The location is not responding."
        );
    }

    #[test]
    fn one_file_is_one_file() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("one"), b"1").unwrap();
        let s = counted(d.path());
        assert!(status(&s.tree(), 0, true, false).ends_with(" in 1 file"));
    }
}
