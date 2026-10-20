//! GNOME's per-file metadata (`gio set FILE metadata::KEY VALUE`), read from gvfsd-metadata's
//! database in `$XDG_DATA_HOME/gvfs-metadata`: one tree file per mount, plus the journal of writes
//! not yet folded into it. Files under `$HOME` live in the `home` tree, other files on the root
//! mount in `root`; trees of other mounts are not read. Numbers in both files are big-endian.

use std::collections::HashMap;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const TREE_MAGIC: &[u8] = b"\xda\x1ameta";
const JOURNAL_MAGIC: &[u8] = b"\xda\x1ajour";
/// Header: magic, major, minor, rotated, random_tag, root, attributes, time_t_base (u64).
const TREE_HEADER: usize = 32;
/// Header: magic, major, minor, random_tag, file_size, num_entries.
const JOURNAL_HEADER: usize = 20;
/// The key of a data entry whose value is a string list.
const KEY_IS_LIST: u32 = 1 << 31;
const FILE_MAX: u64 = 64 * 1024 * 1024;
/// How long a read of the database answers lookups, so a listing reads it once.
const FRESH: Duration = Duration::from_secs(1);

/// Folder `dir`'s icon as Nautilus sets it: `metadata::custom-icon` (a `file://` URI) as a path,
/// else `metadata::custom-icon-name` as a theme icon name.
pub fn custom_icon(dir: &Path) -> Option<String> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    custom_icon_in(dir, &data.join("gvfs-metadata"), home.as_deref())
}

/// [`custom_icon`] from the database in `db_dir`, `home` being the home tree's folder.
fn custom_icon_in(dir: &Path, db_dir: &Path, home: Option<&Path>) -> Option<String> {
    let (tree, path) = tree_of(dir, db_dir, home)?;
    let db = load(&tree)?;
    let db = Db::parse(&db.0, db.1.as_deref())?;
    if let Some(uri) = db.lookup(&path, "custom-icon") {
        return file_path(&uri);
    }
    db.lookup(&path, "custom-icon-name")
        .filter(|n| !n.is_empty())
}

/// The path a `file://` URI names, when it is UTF-8.
fn file_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let bytes = crate::trash::percent_decode(rest);
    String::from_utf8(bytes).ok().filter(|p| p.starts_with('/'))
}

/// The tree in `db_dir` holding `file`, and `file`'s path in it. The home tree holds only files on
/// home's own file system: one mounted under it keeps its own tree, as gvfsd-metadata files it.
fn tree_of(file: &Path, db_dir: &Path, home: Option<&Path>) -> Option<(PathBuf, Vec<u8>)> {
    let file = std::fs::canonicalize(file).ok()?;
    let home = home.and_then(|h| std::fs::canonicalize(h).ok());
    let dev = |p: &Path| std::fs::metadata(p).ok().map(|m| m.dev());
    let file_dev = dev(&file)?;
    if let Some(h) = home.as_deref()
        && let Ok(rel) = file.strip_prefix(h)
        && dev(h) == Some(file_dev)
    {
        let mut path = b"/".to_vec();
        path.extend_from_slice(rel.as_os_str().as_bytes());
        return Some((db_dir.join("home"), path));
    }
    (file_dev == dev(Path::new("/"))?)
        .then(|| (db_dir.join("root"), file.as_os_str().as_bytes().to_vec()))
}

type Loaded = Arc<(Vec<u8>, Option<Vec<u8>>)>;
type Cache = HashMap<PathBuf, (Instant, Option<Loaded>)>;

/// Tree file `tree` and its journal, read again once [`FRESH`] has passed.
fn load(tree: &Path) -> Option<Loaded> {
    static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some((at, db)) = cache.get(tree)
        && at.elapsed() < FRESH
    {
        return db.clone();
    }
    let db = read(tree).map(Arc::new);
    cache.insert(tree.to_path_buf(), (Instant::now(), db.clone()));
    db
}

fn read(tree: &Path) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
    let bytes = crate::ops::read_regular(tree, FILE_MAX).ok()?;
    let tag = u32_at(&bytes, 12)?;
    let mut name = tree.file_name()?.to_os_string();
    name.push(format!("-{tag:08x}.log"));
    let journal = crate::ops::read_regular(&tree.with_file_name(name), FILE_MAX).ok();
    Some((bytes, journal))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes(s.try_into().ok()?))
}

fn cstr_at(b: &[u8], at: usize) -> Option<&[u8]> {
    let s = b.get(at..)?;
    Some(&s[..s.iter().position(|&c| c == 0)?])
}

/// One journal write, newest last.
#[derive(Debug, PartialEq)]
enum Op<'a> {
    Set {
        path: &'a [u8],
        key: &'a [u8],
        value: Option<&'a [u8]>,
    },
    Unset {
        path: &'a [u8],
        key: &'a [u8],
    },
    Copy {
        path: &'a [u8],
        from: &'a [u8],
    },
    Remove {
        path: &'a [u8],
    },
}

struct Db<'a> {
    tree: &'a [u8],
    journal: Vec<Op<'a>>,
}

impl<'a> Db<'a> {
    /// None when `tree` is no version 1 tree file. A journal of another tree, or its entries past
    /// the first torn one, are left out.
    fn parse(tree: &'a [u8], journal: Option<&'a [u8]>) -> Option<Db<'a>> {
        (tree.len() >= TREE_HEADER && tree.starts_with(TREE_MAGIC) && tree[6] == 1).then_some(())?;
        let tag = u32_at(tree, 12)?;
        let journal = journal
            .filter(|j| j.starts_with(JOURNAL_MAGIC) && j.get(6) == Some(&1))
            .filter(|j| u32_at(j, 8) == Some(tag))
            .map(ops)
            .unwrap_or_default();
        Some(Db { tree, journal })
    }

    /// `path`'s string value of `key`: the journal's newest word on it, else the tree's. A
    /// journal copy onto `path` or a folder above it sends the lookup to the copy's source.
    fn lookup(&self, path: &[u8], key: &str) -> Option<String> {
        let key = key.as_bytes();
        let mut path = path.to_vec();
        for op in self.journal.iter().rev() {
            match *op {
                Op::Set {
                    path: p,
                    key: k,
                    value,
                } if p == path && k == key => {
                    return value.and_then(|v| String::from_utf8(v.to_vec()).ok());
                }
                Op::Unset { path: p, key: k } if p == path && k == key => return None,
                Op::Remove { path: p } if under(&path, p).is_some() => return None,
                Op::Copy { path: p, from } => {
                    if let Some(rest) = under(&path, p) {
                        let mut moved = from.to_vec();
                        moved.extend_from_slice(rest);
                        path = moved;
                    }
                }
                _ => {}
            }
        }
        self.tree_lookup(&path, key)
    }

    fn tree_lookup(&self, path: &[u8], key: &[u8]) -> Option<String> {
        let t = self.tree;
        let mut ent = u32_at(t, 16)? as usize;
        for name in path.split(|&c| c == b'/').filter(|n| !n.is_empty()) {
            let dir = u32_at(t, ent + 4)? as usize;
            if dir == 0 {
                return None;
            }
            let n = u32_at(t, dir)? as usize;
            ent = (0..n.min(t.len() / 16))
                .map(|i| dir + 4 + i * 16)
                .find(|&e| u32_at(t, e).and_then(|s| cstr_at(t, s as usize)) == Some(name))?;
        }
        let data = u32_at(t, ent + 8)? as usize;
        if data == 0 {
            return None;
        }
        let attrs = u32_at(t, 20)? as usize;
        let index = (0..u32_at(t, attrs)?.min(t.len() as u32 / 4)).find(|&i| {
            u32_at(t, attrs + 4 + i as usize * 4).and_then(|s| cstr_at(t, s as usize)) == Some(key)
        })?;
        let n = u32_at(t, data)? as usize;
        let value = (0..n.min(t.len() / 8))
            .map(|i| data + 4 + i * 8)
            .find(|&e| u32_at(t, e).is_some_and(|k| k & !KEY_IS_LIST == index))?;
        if u32_at(t, value)? & KEY_IS_LIST != 0 {
            return None;
        }
        let s = cstr_at(t, u32_at(t, value + 4)? as usize)?;
        String::from_utf8(s.to_vec()).ok()
    }
}

/// What follows `dir` in `path` when `path` is `dir` or inside it.
fn under<'p>(path: &'p [u8], dir: &[u8]) -> Option<&'p [u8]> {
    let rest = path.strip_prefix(dir.strip_suffix(b"/").unwrap_or(dir))?;
    (rest.is_empty() || rest.starts_with(b"/")).then_some(rest)
}

/// A journal's entries, each `size, crc32, mtime (u64), type (u8), path\0, ...\0, pad to 4, size`.
fn ops(j: &[u8]) -> Vec<Op<'_>> {
    let count = u32_at(j, 16).unwrap_or(0);
    let mut out = Vec::new();
    let mut at = JOURNAL_HEADER;
    for _ in 0..count {
        let Some(size) = u32_at(j, at).map(|s| s as usize).filter(|&s| s >= 24) else {
            break;
        };
        let Some(entry) = j
            .get(at..at + size)
            .filter(|e| u32_at(e, size - 4) == Some(size as u32))
        else {
            break;
        };
        let body = &entry[17..size - 4];
        let mut strings = body.split(|&c| c == 0);
        let path = strings.next().unwrap_or_default();
        let op = match entry[16] {
            0 => strings.next().map(|key| Op::Set {
                path,
                key,
                value: strings.next(),
            }),
            1 => strings.next().map(|key| Op::Set {
                path,
                key,
                value: None,
            }),
            2 => strings.next().map(|key| Op::Unset { path, key }),
            3 => strings.next().map(|from| Op::Copy { path, from }),
            4 => Some(Op::Remove { path }),
            _ => None,
        };
        out.extend(op);
        at += size;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.split_whitespace().collect();
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// A home tree gvfsd-metadata 1.62 wrote after `gio set` on `~/gvfs-probe/a`
    /// (custom-icon), `~/gvfs-probe` (emblems, a list) and `~/Desktop/calc.desktop` (trusted).
    fn tree() -> Vec<u8> {
        hex(
            "da1a6d657461010000000000256f33610000004c00000020000000006acf
             f4de00000003000000300000003c00000044637573746f6d2d69636f6e00
             656d626c656d730074727573746564000000005c00000060000000dc0000
             00002f000000000000020000008f00000098000000e00000000000000084
             000000bc000000e40006b64e677666732d70726f6265004465736b746f70
             000000000001000000ac000000d4000001000000000163616c632e646573
             6b746f700000000000000001000000d0000000d8000001140006b64e6100
             0000000000000000000000000000000000000000000180000001000000f0
             00000002000000fe000000fc6200610000000001000000020000010c7472
             75650000000000000001000000000000012066696c653a2f2f2f7573722f
             73686172652f7069786d6170732f78253230792e706e6700",
        )
    }

    /// A journal for `tree()`: its first two entries as gvfsd-metadata wrote them for `gio set`
    /// of custom-icon and custom-icon-name on `~/gvfs-probe/a`.
    fn journal(more: &[(u8, &[&str])]) -> Vec<u8> {
        let mut j = hex("da1a6a6f75720100 256f3361 00008000 00000000
            00000054 48a37ef0 000000006ad6ab2c 00 2f677666732d70726f62652f6100
            637573746f6d2d69636f6e00
            66696c653a2f2f2f7573722f73686172652f7069786d6170732f78253230792e706e6700 00 00000054
            00000044 7203b301 000000006ad6ab2c 00 2f677666732d70726f62652f6100
            637573746f6d2d69636f6e2d6e616d6500 666f6c6465722d6d7573696300 000000 00000044");
        for (op, strings) in more {
            let start = j.len();
            j.extend_from_slice(&[0; 16]);
            j.push(*op);
            for s in *strings {
                j.extend_from_slice(s.as_bytes());
                j.push(0);
            }
            while !(j.len() - start).is_multiple_of(4) {
                j.push(0);
            }
            let size = (j.len() - start + 4) as u32;
            j[start..start + 4].copy_from_slice(&size.to_be_bytes());
            j.extend_from_slice(&size.to_be_bytes());
        }
        let n = (2 + more.len()) as u32;
        j[16..20].copy_from_slice(&n.to_be_bytes());
        j
    }

    #[test]
    fn the_tree_answers_a_folders_string_keys() {
        let t = tree();
        let db = Db::parse(&t, None).expect("a tree");
        assert_eq!(
            db.lookup(b"/gvfs-probe/a", "custom-icon").as_deref(),
            Some("file:///usr/share/pixmaps/x%20y.png")
        );
        assert_eq!(
            db.lookup(b"/Desktop/calc.desktop", "trusted").as_deref(),
            Some("true")
        );
        assert_eq!(db.lookup(b"/gvfs-probe/a", "custom-icon-name"), None);
        assert_eq!(
            db.lookup(b"/gvfs-probe", "emblems"),
            None,
            "a list is no string"
        );
        assert_eq!(db.lookup(b"/gvfs-probe", "custom-icon"), None);
        assert_eq!(db.lookup(b"/nowhere/a", "custom-icon"), None);
    }

    #[test]
    fn the_journal_reads_as_gvfsd_metadata_wrote_it() {
        let j = journal(&[]);
        assert_eq!(
            ops(&j),
            [
                Op::Set {
                    path: b"/gvfs-probe/a",
                    key: b"custom-icon",
                    value: Some(b"file:///usr/share/pixmaps/x%20y.png")
                },
                Op::Set {
                    path: b"/gvfs-probe/a",
                    key: b"custom-icon-name",
                    value: Some(b"folder-music")
                },
            ]
        );
    }

    #[test]
    fn the_newest_journal_entry_wins_over_the_tree() {
        let t = tree();
        let j = journal(&[(0, &["/gvfs-probe/a", "custom-icon", "file:///srv/new.png"])]);
        let db = Db::parse(&t, Some(&j)).expect("a tree");
        assert_eq!(
            db.lookup(b"/gvfs-probe/a", "custom-icon").as_deref(),
            Some("file:///srv/new.png")
        );
        assert_eq!(
            db.lookup(b"/gvfs-probe/a", "custom-icon-name").as_deref(),
            Some("folder-music")
        );

        let j = journal(&[(2, &["/gvfs-probe/a", "custom-icon"])]);
        let db = Db::parse(&t, Some(&j)).expect("a tree");
        assert_eq!(db.lookup(b"/gvfs-probe/a", "custom-icon"), None, "unset");
        assert_eq!(
            db.lookup(b"/gvfs-probe/a", "custom-icon-name").as_deref(),
            Some("folder-music")
        );
    }

    #[test]
    fn a_removed_or_moved_folder_takes_its_keys_along() {
        let t = tree();
        let j = journal(&[(4, &["/gvfs-probe"])]);
        let db = Db::parse(&t, Some(&j)).expect("a tree");
        assert_eq!(
            db.lookup(b"/gvfs-probe/a", "custom-icon"),
            None,
            "removed with its parent"
        );

        let j = journal(&[(3, &["/moved", "/gvfs-probe"]), (4, &["/gvfs-probe"])]);
        let db = Db::parse(&t, Some(&j)).expect("a tree");
        assert_eq!(
            db.lookup(b"/moved/a", "custom-icon").as_deref(),
            Some("file:///usr/share/pixmaps/x%20y.png")
        );
        assert_eq!(db.lookup(b"/gvfs-probe/a", "custom-icon"), None);
        assert_eq!(db.lookup(b"/moved-not/a", "custom-icon"), None);
    }

    #[test]
    fn another_trees_journal_or_a_torn_entry_is_left_out() {
        let t = tree();
        let mut j = journal(&[]);
        j[8..12].copy_from_slice(&[0, 0, 0, 1]);
        let db = Db::parse(&t, Some(&j)).expect("a tree");
        assert_eq!(
            db.lookup(b"/gvfs-probe/a", "custom-icon-name"),
            None,
            "another tag"
        );

        let mut j = journal(&[(0, &["/gvfs-probe/a", "custom-icon-name", "folder-red"])]);
        let end = j.len();
        j[end - 1] ^= 1;
        let db = Db::parse(&t, Some(&j)).expect("a tree");
        assert_eq!(
            db.lookup(b"/gvfs-probe/a", "custom-icon-name").as_deref(),
            Some("folder-music")
        );

        assert!(Db::parse(b"not a tree", None).is_none());
        assert!(
            Db::parse(&t[..40], None).is_some_and(|db| db.lookup(b"/gvfs-probe/a", "x").is_none())
        );
    }

    #[test]
    fn a_folder_under_home_reads_the_home_tree_and_its_journal() {
        let home = tempfile::tempdir().expect("a temp dir");
        let db = tempfile::tempdir().expect("a temp dir");
        std::fs::create_dir_all(home.path().join("gvfs-probe/a")).expect("mkdir");
        std::fs::write(db.path().join("home"), tree()).expect("tree");
        let dir = home.path().join("gvfs-probe/a");
        assert_eq!(
            custom_icon_in(&dir, db.path(), Some(home.path())).as_deref(),
            Some("/usr/share/pixmaps/x y.png")
        );

        let other = tempfile::tempdir().expect("a temp dir");
        std::fs::create_dir_all(other.path().join("gvfs-probe/a")).expect("mkdir");
        let j = journal(&[(2, &["/gvfs-probe/a", "custom-icon"])]);
        std::fs::write(other.path().join("home"), tree()).expect("tree");
        std::fs::write(other.path().join("home-256f3361.log"), j).expect("journal");
        assert_eq!(
            custom_icon_in(&dir, other.path(), Some(home.path())).as_deref(),
            Some("folder-music"),
            "the journal unset the file and set the name"
        );
        assert_eq!(
            custom_icon_in(&dir, other.path(), None),
            None,
            "not on the root tree"
        );
    }

    #[test]
    fn a_file_system_mounted_under_home_is_not_in_the_home_tree() {
        let db = Path::new("/srv/gvfs-metadata");
        let (tree, path) = tree_of(Path::new("/proc"), db, Some(Path::new("/proc")))
            .expect("home itself is in the home tree");
        assert_eq!((tree, path), (db.join("home"), b"/".to_vec()));
        assert_eq!(
            tree_of(Path::new("/proc/self"), db, Some(Path::new("/"))),
            None,
            "/proc is its own file system under a home of /"
        );
    }

    #[test]
    fn a_custom_icon_uri_names_a_local_path() {
        assert_eq!(
            file_path("file:///usr/share/x%20y.png").as_deref(),
            Some("/usr/share/x y.png")
        );
        assert_eq!(
            file_path("file://localhost/srv/a.png").as_deref(),
            Some("/srv/a.png")
        );
        assert_eq!(file_path("https://example.org/a.png"), None);
        assert_eq!(file_path("file:///%ff.png"), None, "not UTF-8");
    }
}
