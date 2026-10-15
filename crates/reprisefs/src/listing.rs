//! Reading a directory the way Explorer does: folders first (a symlink
//! to a folder is a shortcut and sorts among the files), then
//! case-insensitively by name, hidden entries (a leading dot, or a line
//! of the folder's `.hidden`) kept but flagged (the file dialog hides
//! them), and each entry's kind and date text formatted for
//! display. A host maps [`Item`] to the row type it draws and fills in
//! the icon.

use crate::apps::MimeDb;
use crate::{hidden, locale, places, props, recent};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub path: PathBuf,
    /// Opens as a folder: a folder, or a symlink to one. Whether it
    /// lists as a folder is [`Item::is_folder`].
    pub is_dir: bool,
    pub is_link: bool,
    pub exec: bool,
    /// Runs when opened ([`crate::program::is_program`]), so it wears a program's icon.
    pub program: bool,
    pub hidden: bool,
    /// Bytes and the modification time; a symlink's are its own, not
    /// its target's.
    pub size: u64,
    pub modified: Option<i64>,
    pub mime: Option<String>,
    /// The Type column: [`SHORTCUT`] or [`WEB_SHORTCUT`] for a shortcut
    /// or a symlink, as its Properties sheet reads, else
    /// [`kind_text_in`]'s.
    pub kind: String,
    pub drive: Option<crate::places::DriveKind>,
    /// An item listed in the Recycle Bin: where it was deleted from and
    /// when, as its `.trashinfo` records them. Boxed so every other
    /// listing's items stay small.
    pub trashed: Option<Box<crate::trash::Trashed>>,
    /// A launcher or a web link ([`crate::apps::shortcut_at`]): listed by
    /// its own name and icon. Boxed as `trashed` is.
    pub shortcut: Option<Box<crate::apps::Shortcut>>,
}

impl Item {
    pub fn label(&self) -> &str {
        self.shortcut.as_ref().map_or(&self.name, |s| s.name())
    }

    /// Lists as a folder: sorts among the folders, shows no size and
    /// stands in the Folders tree. A symlink to a folder is a shortcut,
    /// listed among the files with its own size, though it still opens
    /// as the folder ([`Item::is_dir`]).
    pub fn is_folder(&self) -> bool {
        self.is_dir && !self.is_link
    }

    /// A file whose MIME type is a picture: Thumbnails and Tiles decode it.
    pub fn is_image(&self) -> bool {
        !self.is_dir
            && self
                .mime
                .as_deref()
                .is_some_and(|m| m.starts_with("image/"))
    }
}

/// Everything in `dir`, folders ([`Item::is_folder`]) first then
/// case-insensitively by name.
/// `globs` is `apps::mime_globs()`; pass `MimeDb::empty()` to skip MIME.
pub fn read_dir(dir: &Path, globs: &MimeDb) -> Vec<Item> {
    let entries: Vec<std::fs::DirEntry> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().collect())
        .unwrap_or_default();
    let listed = hidden::Listed::read(dir);
    let mut items: Vec<Item> = entries
        .iter()
        .map(|e| {
            // The entry's own type is free on Linux.
            let is_link = e.file_type().is_ok_and(|t| t.is_symlink());
            item_with(&e.path(), is_link, &listed, globs)
        })
        .collect();
    // Sort on the collected `Item`s: re-statting in the comparator can
    // change answers mid-sort and panic.
    // Names differing only in case break on the exact name, not on
    // read_dir's order, which is the filesystem's hash order.
    items.sort_by_cached_key(|i| (!i.is_folder(), i.name.to_lowercase(), i.name.clone()));
    items
}

pub fn item_at(path: &Path, globs: &MimeDb) -> Item {
    let is_link = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
    let listed = path.parent().map(hidden::Listed::read).unwrap_or_default();
    item_with(path, is_link, &listed, globs)
}

/// One stat follows a link for its folder-ness and what opens it; the
/// size and date the columns show are a symlink's own, a shortcut's as
/// its Properties sheet reads them. `listed` is the folder's `.hidden`.
fn item_with(p: &Path, is_link: bool, listed: &hidden::Listed, globs: &MimeDb) -> Item {
    let meta = std::fs::metadata(p).ok();
    let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
    let exec = meta
        .as_ref()
        .is_some_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    let program = exec
        && meta
            .as_ref()
            .is_some_and(|m| crate::program::is_program(p, m));
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let hidden = p.file_name().is_some_and(|n| listed.hides(n));
    let own = match is_link {
        true => std::fs::symlink_metadata(p).ok(),
        false => meta.clone(),
    };
    let size = own.as_ref().map(|m| m.len()).unwrap_or(0);
    let modified = own
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    let mime = if globs.is_empty() || is_dir {
        None
    } else {
        p.file_name()
            .and_then(|n| globs.mime_of_name(&n.to_string_lossy()))
    };
    let shortcut = meta
        .as_ref()
        .and_then(|m| crate::apps::shortcut_at(p, m))
        .map(Box::new);
    let kind = match shortcut.as_deref() {
        Some(crate::apps::Shortcut::Link(_)) => WEB_SHORTCUT.to_string(),
        Some(crate::apps::Shortcut::App(_)) => SHORTCUT.to_string(),
        None if is_link => SHORTCUT.to_string(),
        None => kind_text_in(globs, p, is_dir, mime.as_deref()),
    };
    Item {
        name,
        path: p.to_path_buf(),
        is_dir,
        is_link,
        exec,
        program,
        hidden,
        size,
        modified,
        mime,
        kind,
        drive: None,
        trashed: None,
        shortcut,
    }
}

/// A file dialog's listed item and its Date Modified text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub item: Item,
    pub modified: String,
}

/// `dir` as a file dialog lists it: [`read_dir`]'s order, hidden entries
/// dropped, and only the entries `keep` accepts. The My Recent Documents
/// place ([`places::RECENT`]) lists the recently used files instead.
pub fn dialog_rows(dir: &Path, globs: &MimeDb, keep: impl FnMut(&Item) -> bool) -> Vec<Row> {
    if places::is_recent(dir) {
        let files = recent::RecentFiles::new().current(RECENT_LIMIT);
        return recent_rows(&files, globs, keep);
    }
    rows(read_dir(dir, globs), keep)
}

const RECENT_LIMIT: usize = 50;

/// `files` (newest first) as My Recent Documents lists them: each row's
/// `path` is the file's own, and a name already listed (the same name in
/// another folder) is left out, so a name picks one file.
fn recent_rows(files: &[PathBuf], globs: &MimeDb, keep: impl FnMut(&Item) -> bool) -> Vec<Row> {
    let mut seen = std::collections::HashSet::new();
    let items = files
        .iter()
        .map(|p| item_at(p, globs))
        .filter(|i| seen.insert(i.name.clone()))
        .collect();
    rows(items, keep)
}

fn rows(items: Vec<Item>, mut keep: impl FnMut(&Item) -> bool) -> Vec<Row> {
    let is_12h = locale::is_12h();
    items
        .into_iter()
        .filter(|i| !i.hidden && keep(i))
        .map(|item| Row {
            modified: item
                .modified
                .map(|s| date_text(s, is_12h))
                .unwrap_or_default(),
            item,
        })
        .collect()
}

/// The type a listing and the Properties sheet give a program launcher
/// or a symlink, whatever it points at.
pub const SHORTCUT: &str = "Shortcut";
pub const WEB_SHORTCUT: &str = "Internet Shortcut";

/// "File Folder", the MIME comment (`props::comment_of`) when
/// shared-mime-info has one, else "TXT File" / "File".
pub fn kind_text(path: &Path, is_dir: bool, mime: Option<&str>) -> String {
    kind_text_in(&MimeDb::empty(), path, is_dir, mime)
}

pub fn kind_text_in(db: &MimeDb, path: &Path, is_dir: bool, mime: Option<&str>) -> String {
    if is_dir {
        return "File Folder".to_string();
    }
    mime.and_then(|m| db.description(m))
        .unwrap_or_else(|| props::generic_type_name(path))
}

/// "M/D/YYYY H:MM AM/PM" (or 24-hour), the Date Modified text.
pub fn date_text(secs: i64, is_12h: bool) -> String {
    let tm = locale::localtime(secs);
    let date = format!("{}/{}/{}", tm.tm_mon + 1, tm.tm_mday, tm.tm_year + 1900);
    if is_12h {
        let (h12, ampm) = match tm.tm_hour {
            0 => (12, "AM"),
            1..=11 => (tm.tm_hour, "AM"),
            12 => (12, "PM"),
            h => (h - 12, "PM"),
        };
        format!("{date} {h12}:{:02} {ampm}", tm.tm_min)
    } else {
        format!("{date} {}:{:02}", tm.tm_hour, tm.tm_min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folders_first_then_case_insensitive_with_hidden_flagged() {
        let t = tempfile::tempdir().unwrap();
        for n in ["b.txt", "A.txt", ".dot", "c.txt"] {
            std::fs::write(t.path().join(n), "").unwrap();
        }
        std::fs::write(t.path().join(".hidden"), "c.txt\n").unwrap();
        std::fs::create_dir(t.path().join("zdir")).unwrap();
        let items = read_dir(t.path(), &MimeDb::empty());
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            names,
            ["zdir", ".dot", ".hidden", "A.txt", "b.txt", "c.txt"]
        );
        let hidden: Vec<bool> = items.iter().map(|i| i.hidden).collect();
        assert_eq!(hidden, [false, true, true, false, false, true]);
        assert!(item_at(&t.path().join("c.txt"), &MimeDb::empty()).hidden);
        assert_eq!(items[0].kind, "File Folder");
        assert_eq!(items[4].kind, "TXT File");
    }
    #[test]
    fn a_trusted_launcher_is_listed_by_its_name() {
        let t = tempfile::tempdir().unwrap();
        let app = "[Desktop Entry]\nType=Application\nName=Terminal\nExec=foot\nIcon=utilities-terminal\n";
        let link = "[Desktop Entry]\nType=Link\nName=Example\nURL=https://example.org/\n";
        crate::apps::write_launcher(&t.path().join("org.foot.desktop"), app).unwrap();
        crate::apps::write_launcher(&t.path().join("web.desktop"), link).unwrap();
        std::fs::write(t.path().join("plain.desktop"), app).unwrap();
        let items = read_dir(t.path(), &MimeDb::empty());
        let got: Vec<(&str, &str, Option<&str>)> = items
            .iter()
            .map(|i| {
                let icon = i.shortcut.as_ref().map(|s| s.icon());
                (i.name.as_str(), i.label(), icon)
            })
            .collect();
        assert_eq!(
            got,
            [
                ("org.foot.desktop", "Terminal", Some("utilities-terminal")),
                ("plain.desktop", "plain.desktop", None),
                ("web.desktop", "Example", Some("text-html")),
            ]
        );
    }
    #[test]
    fn a_shortcut_is_typed_as_its_sheet_types_it() {
        let t = tempfile::tempdir().unwrap();
        let app = "[Desktop Entry]\nType=Application\nName=Terminal\nExec=foot\n";
        let link = "[Desktop Entry]\nType=Link\nName=Example\nURL=https://example.org/\n";
        crate::apps::write_launcher(&t.path().join("app.desktop"), app).unwrap();
        crate::apps::write_launcher(&t.path().join("web.desktop"), link).unwrap();
        std::fs::write(t.path().join("plain.desktop"), app).unwrap();
        std::fs::create_dir(t.path().join("dir")).unwrap();
        std::fs::write(t.path().join("a.txt"), "").unwrap();
        std::os::unix::fs::symlink("dir", t.path().join("to-dir")).unwrap();
        std::os::unix::fs::symlink("a.txt", t.path().join("to-file")).unwrap();
        let kinds: Vec<(String, String)> = read_dir(t.path(), &MimeDb::empty())
            .into_iter()
            .map(|i| (i.name, i.kind))
            .collect();
        let want = [
            ("dir", "File Folder"),
            ("a.txt", "TXT File"),
            ("app.desktop", "Shortcut"),
            ("plain.desktop", "DESKTOP File"),
            ("to-dir", "Shortcut"),
            ("to-file", "Shortcut"),
            ("web.desktop", "Internet Shortcut"),
        ];
        let want: Vec<(String, String)> = want
            .iter()
            .map(|(n, k)| (n.to_string(), k.to_string()))
            .collect();
        assert_eq!(kinds, want);
    }
    #[test]
    fn a_symlink_lists_its_own_size_and_date() {
        let t = tempfile::tempdir().unwrap();
        let target = t.path().join("big.txt");
        std::fs::write(&target, vec![b'x'; 5000]).unwrap();
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        std::fs::File::options()
            .write(true)
            .open(&target)
            .unwrap()
            .set_modified(old)
            .unwrap();
        std::os::unix::fs::symlink("big.txt", t.path().join("link")).unwrap();
        let link = item_at(&t.path().join("link"), &MimeDb::empty());
        let own = std::fs::symlink_metadata(t.path().join("link")).unwrap();
        let sheet = props::glance(&t.path().join("link"), "link", &[]).unwrap();
        assert_eq!(link.size, "big.txt".len() as u64);
        assert_eq!(link.size, sheet.size);
        assert_ne!(link.modified, Some(1_000_000_000));
        assert_eq!(
            link.modified,
            own.modified()
                .ok()
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
        );
        assert_eq!(link.modified, Some(sheet.modified));
        assert_eq!(item_at(&target, &MimeDb::empty()).size, 5000);
    }
    #[test]
    fn a_symlink_to_a_folder_sorts_among_the_files() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir(t.path().join("mfolder")).unwrap();
        std::fs::write(t.path().join("a.txt"), "x").unwrap();
        std::fs::write(t.path().join("c.txt"), "x").unwrap();
        std::os::unix::fs::symlink("mfolder", t.path().join("b link")).unwrap();
        let items = read_dir(t.path(), &MimeDb::empty());
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["mfolder", "a.txt", "b link", "c.txt"]);
        let link = &items[2];
        assert!(
            link.is_dir && !link.is_folder(),
            "opens as a folder, lists as a file"
        );
        assert_eq!(link.size, "mfolder".len() as u64);
        assert!(items[0].is_folder());
    }
    #[test]
    fn a_program_is_an_executable_script_or_elf_file() {
        let t = tempfile::tempdir().unwrap();
        let put = |n: &str, bytes: &[u8], m: u32| {
            std::fs::write(t.path().join(n), bytes).unwrap();
            std::fs::set_permissions(t.path().join(n), std::fs::Permissions::from_mode(m)).unwrap();
        };
        put("elf", b"\x7fELF\x02", 0o755);
        put("photo.png", b"\x89PNG", 0o755);
        put("plain.sh", b"#!/bin/sh\n", 0o644);
        put("run.sh", b"#!/bin/sh\n", 0o755);
        let program: Vec<(String, bool)> = read_dir(t.path(), &MimeDb::empty())
            .into_iter()
            .map(|i| (i.name, i.program))
            .collect();
        assert_eq!(
            program,
            [
                ("elf".into(), true),
                ("photo.png".into(), false),
                ("plain.sh".into(), false),
                ("run.sh".into(), true),
            ]
        );
    }

    #[test]
    fn exec_is_a_regular_file_with_an_execute_bit() {
        let t = tempfile::tempdir().unwrap();
        let mode = |n: &str, m: u32| {
            std::fs::write(t.path().join(n), "").unwrap();
            std::fs::set_permissions(t.path().join(n), std::fs::Permissions::from_mode(m)).unwrap();
        };
        mode("plain", 0o644);
        mode("tool", 0o755);
        mode("owner", 0o700);
        std::fs::create_dir(t.path().join("dir")).unwrap();
        let exec: Vec<(String, bool)> = read_dir(t.path(), &MimeDb::empty())
            .into_iter()
            .map(|i| (i.name, i.exec))
            .collect();
        assert_eq!(
            exec,
            [
                ("dir".into(), false),
                ("owner".into(), true),
                ("plain".into(), false),
                ("tool".into(), true),
            ]
        );
    }
    #[test]
    fn dialog_rows_drop_hidden_entries_and_what_keep_refuses() {
        let t = tempfile::tempdir().unwrap();
        for n in ["b.txt", "a.png", ".dot.txt", "listed.txt"] {
            std::fs::write(t.path().join(n), "").unwrap();
        }
        std::fs::write(t.path().join(".hidden"), "listed.txt\n").unwrap();
        std::fs::create_dir(t.path().join("dir")).unwrap();
        let rows = dialog_rows(t.path(), &MimeDb::empty(), |i| {
            i.is_dir || i.name.ends_with(".txt")
        });
        let names: Vec<&str> = rows.iter().map(|r| r.item.name.as_str()).collect();
        assert_eq!(names, ["dir", "b.txt"]);
        assert_eq!(rows[1].item.path, t.path().join("b.txt"));
        assert!(!rows[1].modified.is_empty(), "a file's date is formatted");
    }
    #[test]
    fn recent_rows_keep_each_files_path_and_the_newest_of_a_name() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir(t.path().join("old")).unwrap();
        let (new, old, pic) = (
            t.path().join("notes.txt"),
            t.path().join("old/notes.txt"),
            t.path().join("pic.png"),
        );
        for p in [&new, &old, &pic] {
            std::fs::write(p, "").unwrap();
        }
        let rows = recent_rows(&[new.clone(), pic, old], &MimeDb::empty(), |i| {
            i.name.ends_with(".txt")
        });
        let got: Vec<(&str, &Path)> = rows
            .iter()
            .map(|r| (r.item.name.as_str(), r.item.path.as_path()))
            .collect();
        assert_eq!(got, [("notes.txt", new.as_path())]);
    }
    #[test]
    fn date_text_is_explorers() {
        // 2026-09-13 20:25 local: build the epoch from the components.
        let secs = crate::locale::mktime(2026, 9, 13, 20, 25, 0);
        assert_eq!(date_text(secs, true), "9/13/2026 8:25 PM");
        assert_eq!(date_text(secs, false), "9/13/2026 20:25");
    }
}
