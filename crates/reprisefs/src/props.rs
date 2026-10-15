//! What the file Properties sheet's General tab shows, read from the file system and formatted for display, plus the writes the sheet and the Open With chooser make.

use crate::apps::{self, AppEntry, MimeDb};
use crate::mimetree::MimeTree;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileInfo {
    /// The name the icon shows: the file name less a hidden extension, or a launcher's `Name=`.
    pub name: String,
    /// The extension the icon hides (".txt"), or ""; a rename puts it back.
    pub hidden_ext: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub type_name: String,
    pub opens_with: Option<String>,
    pub opens_with_icon: Option<String>,
    pub location: String,
    pub size: u64,
    pub on_disk: u64,
    /// A folder's "Contains" counts; `None` for a file and for a folder `glance` has not walked yet.
    pub contains: Option<(u64, u64)>,
    pub created: Option<i64>,
    pub modified: i64,
    pub accessed: i64,
    pub read_only: bool,
    /// A file's owner execute bit; false for a folder.
    pub executable: bool,
    /// Whether the Executable box can change it ([`can_set_executable`]).
    pub can_set_executable: bool,
    pub hidden: bool,
    pub listed: bool,
    /// A symlink or a launcher: the second tab is Compatibility, not Summary.
    pub is_shortcut: bool,
    /// Whether backups and search indexers take it ([`crate::meta::BACKUP`], [`crate::meta::INDEX`]); `None` on a file system without user attributes.
    pub robots: Option<[bool; 2]>,
    pub summary: Option<[String; 6]>,
    pub own_icon: Option<String>,
    /// A shortcut's own keys, shown on its Shortcut tab (launcher, symlink) or Web Document tab (web link); None for anything else.
    pub launcher: Option<crate::launcher::Launcher>,
    /// A shortcut's "Description": its name, or what a symlink points at; a symlink to a file some program opens shows that program instead.
    pub description: Option<String>,
}

/// Entries a folder walk visits before giving up on an exact count.
const WALK_LIMIT: u64 = 50_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FolderTotals {
    pub files: u64,
    pub folders: u64,
    pub size: u64,
    pub on_disk: u64,
}

impl FileInfo {
    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.name.clone())
    }

    pub fn add_totals(&mut self, t: FolderTotals) {
        self.size = t.size;
        self.on_disk = t.on_disk;
        self.contains = Some((t.files, t.folders));
    }
}

/// Facts about several files and folders at once, for the combined Properties sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetInfo {
    pub paths: Vec<PathBuf>,
    pub has_dir: bool,
    pub files: u64,
    pub folders: u64,
    pub size: u64,
    pub on_disk: u64,
    pub type_name: Option<String>,
    pub location: Option<String>,
    pub read_only: Option<bool>,
    pub hidden: Option<bool>,
    /// The robots flags every item agrees on (`None` inside where they differ), or `None` when an item's file system has no user attributes.
    pub robots: Option<[Option<bool>; 2]>,
    pub has_shortcut: bool,
    pub has_document: bool,
    /// A folder of the set has not been walked: the totals hold only what [`glance_set`] was handed until [`SetInfo::add_totals`].
    pub walking: bool,
}

pub fn glance_set(items: &[FileInfo]) -> SetInfo {
    fn shared<T: PartialEq>(mut it: impl Iterator<Item = T>) -> Option<T> {
        let first = it.next()?;
        it.all(|v| v == first).then_some(first)
    }
    let mut set = SetInfo {
        paths: items.iter().map(|i| i.path.clone()).collect(),
        has_dir: items.iter().any(|i| i.is_dir),
        files: 0,
        folders: 0,
        size: 0,
        on_disk: 0,
        type_name: shared(items.iter().map(|i| &i.type_name)).cloned(),
        location: shared(items.iter().map(|i| &i.location)).cloned(),
        read_only: shared(items.iter().map(|i| i.read_only)),
        hidden: shared(items.iter().map(|i| i.hidden)),
        robots: items
            .iter()
            .map(|i| i.robots)
            .collect::<Option<Vec<_>>>()
            .map(|all| [0, 1].map(|k| shared(all.iter().map(|r| r[k])))),
        has_shortcut: items.iter().any(|i| i.is_shortcut),
        has_document: items.iter().any(|i| !i.is_shortcut && !i.is_dir),
        walking: false,
    };
    for i in items {
        if i.is_dir {
            set.folders += 1;
            match i.contains {
                Some((files, folders)) => {
                    set.files += files;
                    set.folders += folders;
                }
                None => set.walking = true,
            }
        } else {
            set.files += 1;
        }
        set.size += i.size;
        set.on_disk += i.on_disk;
    }
    set
}

impl SetInfo {
    pub fn add_totals(&mut self, t: FolderTotals) {
        self.files += t.files;
        self.folders += t.folders;
        self.size += t.size;
        self.on_disk += t.on_disk;
        self.walking = false;
    }

    pub fn count_text(&self) -> String {
        format_contains(self.files, self.folders)
    }

    pub fn type_text(&self) -> String {
        match &self.type_name {
            Some(t) => format!("All of type {t}"),
            None => "Multiple Types".to_string(),
        }
    }

    pub fn location_text(&self) -> String {
        match &self.location {
            Some(dir) => format!("All in {dir}"),
            None => "Various Folders".to_string(),
        }
    }
}

pub fn inspect(path: &Path, name: &str, apps: &[AppEntry]) -> std::io::Result<FileInfo> {
    let mut info = glance(path, name, apps)?;
    if info.is_dir {
        info.add_totals(folder_totals(path));
    }
    Ok(info)
}

/// `inspect` without the folder walk: a folder's sizes read 0 and its `contains` stays `None` until [`FileInfo::add_totals`].
pub fn glance(path: &Path, name: &str, apps: &[AppEntry]) -> std::io::Result<FileInfo> {
    glance_in(path, name, apps, &apps::mime_globs())
}

pub fn glance_in(
    path: &Path,
    name: &str,
    apps: &[AppEntry],
    db: &MimeDb,
) -> std::io::Result<FileInfo> {
    use crate::launcher::{Kind, Launcher};
    use std::os::unix::fs::MetadataExt;
    let link = std::fs::symlink_metadata(path)?;
    let meta = std::fs::metadata(path).unwrap_or_else(|_| link.clone());
    let symlink = link.file_type().is_symlink();
    let is_dir = meta.is_dir() && !symlink;
    // A `.desktop` file is a shortcut where it lists as one: an untrusted
    // one is a plain file here as in a listing and on the desktop.
    let launcher = Launcher::at(path, db);
    let is_shortcut = symlink || launcher.is_some();
    let mime = if is_shortcut {
        None
    } else {
        apps::mime_of_in(db, path)
    };
    let type_name = match &launcher {
        Some(l) if l.kind == Kind::Web => crate::listing::WEB_SHORTCUT.to_string(),
        _ if is_shortcut => crate::listing::SHORTCUT.to_string(),
        _ => crate::listing::kind_text_in(db, path, is_dir, mime.as_deref()),
    };
    let target_mime = match &launcher {
        Some(l) if l.kind == Kind::Symlink && meta.is_file() => std::fs::canonicalize(path)
            .ok()
            .and_then(|to| apps::mime_of_in(db, &to)),
        _ => None,
    };
    // A program runs when opened, so it opens with nothing; its sheet names it instead.
    let program = !is_dir && crate::program::is_program(path, &meta);
    let opens_with_entry = if is_dir || program {
        None
    } else {
        mime.or(target_mime)
            .as_deref()
            .and_then(|m| default_app_entry(m, apps))
    };
    let opens_with = opens_with_entry.map(|a| a.name.clone());
    let opens_with_icon = opens_with_entry.and_then(|a| a.icon.clone());
    let description = match &launcher {
        Some(l) if opens_with.is_none() => Some(match l.kind {
            Kind::Symlink => {
                let to = std::fs::read_link(path).unwrap_or_default();
                to.file_name()
                    .unwrap_or(to.as_os_str())
                    .to_string_lossy()
                    .into_owned()
            }
            _ => name.to_string(),
        }),
        _ if program => Some(name.to_string()),
        _ => None,
    };
    let own = if symlink { &link } else { &meta };
    let (size, on_disk) = if is_dir {
        (0, 0)
    } else {
        (own.len(), own.blocks() * 512)
    };
    let secs = |t: std::io::Result<std::time::SystemTime>| {
        t.ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
    };
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let hidden_ext = file_name
        .strip_prefix(name)
        .filter(|ext| ext.starts_with('.'))
        .unwrap_or("")
        .to_string();
    Ok(FileInfo {
        name: name.to_string(),
        hidden_ext,
        path: path.to_path_buf(),
        is_dir,
        type_name,
        opens_with,
        opens_with_icon,
        location: path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size,
        on_disk,
        contains: None,
        created: secs(own.created()),
        modified: secs(own.modified()).unwrap_or(0),
        accessed: secs(own.accessed()).unwrap_or(0),
        read_only: meta.mode() & 0o200 == 0,
        executable: !is_dir && meta.mode() & 0o100 != 0,
        can_set_executable: !is_dir
            && !is_shortcut
            && can_set_executable(meta.uid(), Mount::of(path)),
        hidden: crate::hidden::is_hidden(path),
        listed: crate::hidden::is_listed(path),
        is_shortcut,
        robots: crate::meta::robots(path, crate::meta::BACKUP)
            .zip(crate::meta::robots(path, crate::meta::INDEX))
            .map(|(b, i)| [b, i]),
        summary: (!is_dir && !is_shortcut)
            .then(|| crate::meta::summary(path))
            .flatten(),
        own_icon: is_dir.then(|| crate::meta::folder_icon(path)).flatten(),
        launcher,
        description,
    })
}

/// Write the Read-only attribute: setting clears every write bit, clearing gives the owner's back. Group and other keep what they had.
pub fn set_read_only(path: &Path, on: bool) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)?.permissions().mode();
    let new = read_only_mode(mode, on);
    if new == mode {
        return Ok(());
    }
    std::fs::set_permissions(path, PermissionsExt::from_mode(new))
}

fn read_only_mode(mode: u32, on: bool) -> u32 {
    if on { mode & !0o222 } else { mode | 0o200 }
}

/// Write the Executable attribute: setting gives the owner execute, and group and other execute
/// where they can read; clearing takes every execute bit. A file system that keeps no execute
/// bit can report success and change nothing, so the mode is read back.
pub fn set_executable(path: &Path, on: bool) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)?.permissions().mode();
    let new = executable_mode(mode, on);
    if new == mode {
        return Ok(());
    }
    std::fs::set_permissions(path, PermissionsExt::from_mode(new))?;
    let now = std::fs::metadata(path)?.permissions().mode();
    if now & 0o111 != new & 0o111 {
        return Err(std::io::Error::other(
            "This file system cannot mark files as executable.",
        ));
    }
    Ok(())
}

fn executable_mode(mode: u32, on: bool) -> u32 {
    if on {
        mode | 0o100 | ((mode & 0o044) >> 2)
    } else {
        mode & !0o111
    }
}

/// What a file's mount allows of its execute bit, from `statfs`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mount {
    pub read_only: bool,
    pub noexec: bool,
    /// FAT, exFAT or NTFS: modes are fixed by the mount options.
    pub fixed_modes: bool,
}

impl Mount {
    /// `path`'s mount, or the default (everything allowed) where it cannot be read; a write then
    /// reports what went wrong.
    pub fn of(path: &Path) -> Mount {
        use std::os::unix::ffi::OsStrExt;
        let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return Mount::default();
        };
        let mut vfs = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        let mut fs = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: `c` is NUL-terminated and each out-param points at room for its struct.
        let read = unsafe {
            libc::statvfs(c.as_ptr(), vfs.as_mut_ptr()) == 0
                && libc::statfs(c.as_ptr(), fs.as_mut_ptr()) == 0
        };
        if !read {
            return Mount::default();
        }
        // SAFETY: both calls returned 0, so they filled their structs.
        let (vfs, fs) = unsafe { (vfs.assume_init(), fs.assume_init()) };
        const FIXED_MODES: [libc::__fsword_t; 4] = [
            0x4d44,      // FAT
            0x2011_bab0, // exFAT
            0x7366_746e, // NTFS (ntfs3)
            0x5346_544e, // NTFS (ntfs)
        ];
        Mount {
            read_only: vfs.f_flag & libc::ST_RDONLY != 0,
            noexec: vfs.f_flag & libc::ST_NOEXEC != 0,
            fixed_modes: FIXED_MODES.contains(&fs.f_type),
        }
    }
}

/// Whether the Executable box takes a click on a file owned by `owner` (a uid) on `mount`: the user
/// owns it (or is root), and the mount is writable, runs programs and keeps modes.
pub fn can_set_executable(owner: u32, mount: Mount) -> bool {
    // SAFETY: geteuid has no preconditions.
    let me = unsafe { libc::geteuid() };
    (owner == me || me == 0) && !mount.read_only && !mount.noexec && !mount.fixed_modes
}

/// `set_read_only` down a whole tree. The walk goes by directory fd and never follows a link, so an entry swapped for a symlink mid-walk is skipped; `path` itself is followed.
pub fn set_read_only_tree(path: &Path, on: bool) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let top = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY)
        .open(path)
    {
        Ok(top) => top,
        Err(e) if e.raw_os_error() == Some(libc::ENOTDIR) => return set_read_only(path, on),
        Err(e) => return Err(e),
    };
    set_read_only_fd(&top, on)?;
    let mut pending = Vec::new();
    push_tree_entries(std::rc::Rc::new(top), on, &mut pending);
    while let Some((parent, name)) = pending.pop() {
        let dir = match open_dir_at(&parent, &name) {
            Ok(dir) => dir,
            Err(e) => {
                log::warn!("properties: could not open {name:?}: {e}");
                continue;
            }
        };
        if let Err(e) = set_read_only_fd(&dir, on) {
            log::warn!("properties: could not set read-only: {e}");
        }
        push_tree_entries(std::rc::Rc::new(dir), on, &mut pending);
    }
    Ok(())
}

type PendingDir = (std::rc::Rc<std::fs::File>, std::ffi::CString);

fn push_tree_entries(dir: std::rc::Rc<std::fs::File>, on: bool, pending: &mut Vec<PendingDir>) {
    let names = match dir_entry_names(&dir) {
        Ok(names) => names,
        Err(e) => {
            log::warn!("properties: could not list a folder: {e}");
            return;
        }
    };
    for name in names {
        let Ok(mode) = mode_at(&dir, &name) else {
            continue;
        };
        match mode & libc::S_IFMT {
            libc::S_IFDIR => pending.push((dir.clone(), name)),
            libc::S_IFLNK => {}
            _ => {
                if let Err(e) = chmod_at(&dir, &name, mode, on) {
                    log::warn!("properties: could not set read-only: {e}");
                }
            }
        }
    }
}

fn set_read_only_fd(file: &std::fs::File, on: bool) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = file.metadata()?.permissions().mode();
    let new = read_only_mode(mode, on);
    if new == mode {
        return Ok(());
    }
    file.set_permissions(PermissionsExt::from_mode(new))
}

fn open_dir_at(dir: &std::fs::File, name: &std::ffi::CStr) -> std::io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: `dir` is an open fd and `name` a NUL-terminated string that
    // outlives the call.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `openat` returned a fresh fd that nothing else owns.
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

fn mode_at(dir: &std::fs::File, name: &std::ffi::CStr) -> std::io::Result<u32> {
    use std::os::fd::AsRawFd;
    let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `dir` is an open fd, `name` NUL-terminated, and `st` points
    // at room for one `stat`.
    let rc = unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            name.as_ptr(),
            st.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fstatat` returned 0, so it filled `st`.
    Ok(unsafe { st.assume_init() }.st_mode)
}

/// Write the Read-only attribute of `name` in `dir`; `AT_SYMLINK_NOFOLLOW` makes an entry swapped for a symlink since `mode_at` fail rather than chmod the target.
fn chmod_at(
    dir: &std::fs::File,
    name: &std::ffi::CStr,
    mode: u32,
    on: bool,
) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let new = read_only_mode(mode, on);
    if new == mode {
        return Ok(());
    }
    // SAFETY: `dir` is an open fd and `name` a NUL-terminated string that
    // outlives the call.
    let rc = unsafe {
        libc::fchmodat(
            dir.as_raw_fd(),
            name.as_ptr(),
            new & 0o7777,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn dir_entry_names(dir: &std::fs::File) -> std::io::Result<Vec<std::ffi::CString>> {
    use std::os::fd::IntoRawFd;
    // `fdopendir` takes the fd over, so it gets a duplicate of `dir`'s.
    let fd = dir.try_clone()?.into_raw_fd();
    // SAFETY: `fd` is an open directory fd owned here; on success the
    // stream owns it and `closedir` below closes both.
    let stream = unsafe { libc::fdopendir(fd) };
    if stream.is_null() {
        let e = std::io::Error::last_os_error();
        // SAFETY: `fdopendir` failed, so `fd` is still ours to close once.
        unsafe { libc::close(fd) };
        return Err(e);
    }
    let mut names = Vec::new();
    loop {
        // SAFETY: `stream` is a live stream from `fdopendir`, used by this
        // thread only; a non-null entry's `d_name` is NUL-terminated and
        // valid until the next `readdir`, and is copied out before that.
        let name = unsafe {
            let entry = libc::readdir(stream);
            if entry.is_null() {
                break;
            }
            std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()).to_owned()
        };
        if name.as_bytes() != b"." && name.as_bytes() != b".." {
            names.push(name);
        }
    }
    // SAFETY: `stream` came from `fdopendir` and is closed exactly once.
    unsafe { libc::closedir(stream) };
    Ok(names)
}

/// Hide down a whole tree through `.hidden` lists, never a rename. Dotfiles are left alone and dotted folders not entered, so `.git` keeps what it had; `path` itself keeps its own rename. The walk goes by directory fd and never enters a link.
pub fn set_hidden_tree(path: &Path, hide: bool) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let top = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY)
        .open(path)
    {
        Ok(top) => top,
        Err(e) if e.raw_os_error() == Some(libc::ENOTDIR) => return Ok(()),
        Err(e) => return Err(e),
    };
    let mut pending = Vec::new();
    let mut first = list_entries(std::rc::Rc::new(top), hide, &mut pending).err();
    while let Some((parent, name)) = pending.pop() {
        match open_dir_at(&parent, &name) {
            Ok(dir) => {
                if let Err(e) = list_entries(std::rc::Rc::new(dir), hide, &mut pending) {
                    first.get_or_insert(e);
                }
            }
            Err(e) => log::warn!("properties: could not open {name:?}: {e}"),
        }
    }
    first.map_or(Ok(()), Err)
}

fn list_entries(
    dir: std::rc::Rc<std::fs::File>,
    hide: bool,
    pending: &mut Vec<PendingDir>,
) -> std::io::Result<()> {
    let names = match dir_entry_names(&dir) {
        Ok(names) => names,
        Err(e) => {
            log::warn!("properties: could not list a folder: {e}");
            return Ok(());
        }
    };
    let mut shown: Vec<&std::ffi::CStr> = names
        .iter()
        .filter(|n| !n.to_bytes().starts_with(b"."))
        .map(|n| n.as_c_str())
        .collect();
    shown.sort();
    let bytes: Vec<&[u8]> = shown.iter().map(|n| n.to_bytes()).collect();
    let (add, remove) = if hide {
        (&bytes[..], &[][..])
    } else {
        (&[][..], &bytes[..])
    };
    let written = crate::hidden::edit_at(&dir, add, remove)
        .inspect_err(|e| log::warn!("properties: could not write a folder's .hidden: {e}"));
    for name in shown {
        if mode_at(&dir, name).is_ok_and(|m| m & libc::S_IFMT == libc::S_IFDIR) {
            pending.push((dir.clone(), name.to_owned()));
        }
    }
    written
}

pub fn attr_verbs(read_only: Option<bool>, hidden: Option<bool>) -> String {
    AttrMoves {
        read_only,
        ..Default::default()
    }
    .verbs(hidden)
}

/// The attribute changes besides Hidden (which renames and lists, see [`write_hidden`]); `None` leaves an attribute as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttrMoves {
    pub read_only: Option<bool>,
    /// A single file's Executable box; a folder's and a set's never move.
    pub executable: Option<bool>,
    pub backup: Option<bool>,
    pub index: Option<bool>,
}

impl AttrMoves {
    pub fn is_empty(&self) -> bool {
        *self == AttrMoves::default()
    }

    pub fn verbs(&self, hidden: Option<bool>) -> String {
        let one = |on: bool, what: &str| format!("{}set {what}", if on { "" } else { "un" });
        [
            self.read_only.map(attr_verb),
            hidden.map(|h| if h { "hide" } else { "unhide" }.to_string()),
            self.backup.map(|on| one(on, "archive")),
            self.index.map(|on| one(on, "index")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ")
    }
}

/// Write `moves` to `path`, and with `tree` to everything under it (symlinks left alone). Every write is tried; the first failure is returned.
pub fn write_attrs(path: &Path, moves: &AttrMoves, tree: bool) -> std::io::Result<()> {
    let mut first = None;
    let mut note = |r: std::io::Result<()>| {
        if let Err(e) = r {
            log::warn!("properties: could not write an attribute: {e}");
            first.get_or_insert(e);
        }
    };
    if let Some(on) = moves.read_only {
        note(if tree {
            set_read_only_tree(path, on)
        } else {
            set_read_only(path, on)
        });
    }
    if let Some(on) = moves.executable {
        note(set_executable(path, on));
    }
    let robots = [
        (crate::meta::BACKUP, moves.backup),
        (crate::meta::INDEX, moves.index),
    ];
    if robots.iter().any(|(_, m)| m.is_some()) {
        let mut todo = vec![path.to_path_buf()];
        while let Some(p) = todo.pop() {
            for (name, on) in robots {
                if let Some(on) = on {
                    note(crate::meta::set_robots(&p, name, on));
                }
            }
            if tree && let Ok(dir) = std::fs::read_dir(&p) {
                for e in dir.flatten() {
                    if e.file_type().is_ok_and(|t| !t.is_symlink()) {
                        todo.push(e.path());
                    }
                }
            }
        }
    }
    first.map_or(Ok(()), Err)
}

pub fn attr_verb(read_only: bool) -> String {
    if read_only {
        "set read-only".to_string()
    } else {
        "unset read-only".to_string()
    }
}

/// What [`write_set`] did: where each item is now, the renames Hidden made (old path first), and the first failure.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetWrite {
    pub now: Vec<PathBuf>,
    pub renames: Vec<(PathBuf, PathBuf)>,
    pub failed: Vec<(PathBuf, String)>,
}

/// A set's Properties OK or Apply: `moves` and Hidden's lists on every item. A failed write is logged and noted in [`SetWrite::failed`] and the rest go ahead. Blocks on the file system, so run it off the event loop.
pub fn write_set(
    paths: Vec<PathBuf>,
    moves: AttrMoves,
    hidden: Option<bool>,
    tree: bool,
) -> SetWrite {
    let mut out = SetWrite::default();
    for path in paths {
        if let Err(e) = write_item(&path, &moves, hidden, tree) {
            out.failed.push((path.clone(), e.to_string()));
        }
        let name = path.file_name().unwrap_or_default().to_os_string();
        let wanted = hidden.map(|h| {
            let listed = crate::hidden::is_listed(&path);
            let dotted = name.as_encoded_bytes().starts_with(b".");
            hidden_name(&name, crate::hidden::wants_dot(h, dotted, listed))
        });
        match wanted.filter(|w| *w != name && !name.is_empty()) {
            Some(w) => {
                let to = path.with_file_name(w);
                match crate::hidden::rename(&path, &to) {
                    Ok(()) => {
                        out.renames.push((path, to.clone()));
                        out.now.push(to);
                    }
                    Err(e) => {
                        log::warn!("properties: could not rename {}: {e}", path.display());
                        out.failed.push((path.clone(), e.to_string()));
                        out.now.push(path);
                    }
                }
            }
            None => out.now.push(path),
        }
    }
    out
}

/// Hidden's writes besides the item's own rename: a shown item leaves its folder's `.hidden`, and with `tree` the names under a folder go on or off its lists ([`set_hidden_tree`]).
pub fn write_hidden(path: &Path, hide: bool, tree: bool) -> std::io::Result<()> {
    let mut first = None;
    if !hide && let Err(e) = crate::hidden::unlist(path) {
        log::warn!("properties: could not show {}: {e}", path.display());
        first = Some(e);
    }
    if tree && let Err(e) = set_hidden_tree(path, hide) {
        log::warn!("properties: could not hide under {}: {e}", path.display());
        first.get_or_insert(e);
    }
    first.map_or(Ok(()), Err)
}

pub fn write_item(
    path: &Path,
    moves: &AttrMoves,
    hidden: Option<bool>,
    tree: bool,
) -> std::io::Result<()> {
    let lock = moves.read_only == Some(true);
    let before = AttrMoves {
        read_only: moves.read_only.filter(|_| !lock),
        ..*moves
    };
    let mut first = write_attrs(path, &before, tree).err();
    if let Some(h) = hidden
        && let Err(e) = write_hidden(path, h, tree)
    {
        first.get_or_insert(e);
    }
    if lock {
        let last = AttrMoves {
            read_only: Some(true),
            ..Default::default()
        };
        if let Err(e) = write_attrs(path, &last, tree) {
            first.get_or_insert(e);
        }
    }
    first.map_or(Ok(()), Err)
}

pub fn hidden_name(name: &std::ffi::OsStr, dot: bool) -> std::ffi::OsString {
    use std::os::unix::ffi::OsStrExt;
    let bytes = name.as_bytes();
    let bare = bytes.strip_prefix(b".").unwrap_or(bytes);
    let mut out = std::ffi::OsString::with_capacity(bare.len() + 1);
    if dot {
        out.push(".");
    }
    out.push(std::ffi::OsStr::from_bytes(bare));
    out
}

/// Files, folders, bytes and bytes on disk under `dir`, stopping past `WALK_LIMIT` entries. Slow on a big tree with a cold cache; run it off the event loop.
pub fn folder_totals(dir: &Path) -> FolderTotals {
    folder_totals_unless(dir, &AtomicBool::new(false)).unwrap_or_default()
}

/// [`folder_totals`], given up with `None` as soon as `stop` is set.
pub fn folder_totals_unless(dir: &Path, stop: &AtomicBool) -> Option<FolderTotals> {
    use std::os::unix::fs::MetadataExt;
    let (mut files, mut folders, mut size, mut on_disk) = (0, 0, 0, 0);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            if files + folders >= WALK_LIMIT {
                return Some(FolderTotals {
                    files,
                    folders,
                    size,
                    on_disk,
                });
            }
            let Ok(m) = e.metadata() else {
                continue;
            };
            if m.is_dir() {
                folders += 1;
                stack.push(e.path());
            } else {
                files += 1;
                size += m.len();
                on_disk += m.blocks() * 512;
            }
        }
    }
    Some(FolderTotals {
        files,
        folders,
        size,
        on_disk,
    })
}

/// The MIME type's description from shared-mime-info (the untranslated `<comment>`).
pub(crate) fn mime_description(mime: &str) -> Option<String> {
    let roots = ["/usr/share/mime", "/usr/local/share/mime"];
    roots.iter().find_map(|r| {
        let text = std::fs::read_to_string(Path::new(r).join(format!("{mime}.xml"))).ok()?;
        comment_of(&text)
    })
}

pub fn comment_of(xml: &str) -> Option<String> {
    xml.lines().find_map(|l| {
        let l = l.trim();
        let rest = l.strip_prefix("<comment>")?;
        let (c, _) = rest.split_once("</comment>")?;
        Some(c.to_string())
    })
}

pub fn generic_type_name(path: &Path) -> String {
    match path.extension().map(|e| e.to_string_lossy().to_uppercase()) {
        Some(ext) if !ext.is_empty() => format!("{ext} File"),
        _ => "File".to_string(),
    }
}

/// The program registered as the default for `mime` in the mime-apps lists, in the spec's order (user config, applications dir; `<desktop>-mimeapps.list` before `mimeapps.list`). A type without its own program takes its nearest parent's default ([`default_in_lists`]).
pub fn default_app_entry<'a>(mime: &str, apps: &'a [AppEntry]) -> Option<&'a AppEntry> {
    default_app(mime, apps).map(|i| &apps[i])
}

/// How a `path` the caller knows is a file opens in `default`, else `xdg-open`, with no stat.
pub fn open_file(path: &Path, default: Option<&AppEntry>, terminal: &str) -> Opening {
    match default {
        Some(app) => Opening {
            argv: apps::open_argv(app, terminal, path),
            app: Some(app.id.clone()),
        },
        None => Opening {
            argv: vec!["xdg-open".into(), path.into()],
            app: None,
        },
    }
}

/// A command that opens a path, and the desktop entry id it runs as (its app scope): the default
/// program's, None for Explorer or `xdg-open`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opening {
    pub argv: Vec<std::ffi::OsString>,
    pub app: Option<String>,
}

/// How `path` opens, with its default program looked up from `globs` and `apps`. Everything that
/// opens a file on its own comes here.
pub fn open_in(path: &Path, globs: &MimeDb, apps: &[AppEntry], terminal: &str) -> Opening {
    let mime = apps::mime_of_in(globs, path);
    let default = mime.as_deref().and_then(|m| default_app_entry(m, apps));
    match default.filter(|_| !path.is_dir()) {
        Some(app) => Opening {
            argv: apps::open_argv(app, terminal, path),
            app: Some(app.id.clone()),
        },
        None => Opening {
            argv: apps::open_argv_for(path),
            app: None,
        },
    }
}

pub fn default_app(mime: &str, apps: &[AppEntry]) -> Option<usize> {
    let lists: Vec<String> = mimeapps_paths()
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    default_in_lists(&lists, &MimeTree::load(), mime, apps)
}

fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

/// Every mime-apps list a default is looked up in, in the spec's order, whether it exists or not.
pub fn mimeapps_paths() -> Vec<PathBuf> {
    let names: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|d| !d.is_empty())
        .map(|d| format!("{}-mimeapps.list", d.to_lowercase()))
        .chain(std::iter::once("mimeapps.list".to_string()))
        .collect();
    let dirs: Vec<PathBuf> = config_home()
        .into_iter()
        .chain(apps::default_dirs())
        .collect();
    dirs.iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .collect()
}

/// The list the user's own choices go in; no other list is ever written.
pub fn user_mimeapps() -> std::io::Result<PathBuf> {
    config_home()
        .map(|c| c.join("mimeapps.list"))
        .ok_or_else(|| std::io::Error::other("no home directory"))
}

/// [`default_app`] over `lists` (in precedence order) for a file of type `mime`. Walks the type's [`MimeTree::lineage`]: per type, first the `[Default Applications]` ids, then `[Added Associations]`, then the first program whose `MimeType=` names it; only a type with none goes on to its parent.
pub fn default_in_lists(
    lists: &[String],
    tree: &MimeTree,
    mime: &str,
    apps: &[AppEntry],
) -> Option<usize> {
    let installed = |id: &str| apps.iter().position(|a| a.id == id);
    let mut seen: Vec<String> = Vec::new();
    for step in tree.lineage(mime) {
        let canonical = tree.canonical(&step);
        if seen.contains(&canonical) {
            continue;
        }
        seen.push(canonical);
        let names = tree.names(&step);
        let listed = |group: &str| {
            lists.iter().find_map(|text| {
                names
                    .iter()
                    .flat_map(|m| ids_in_list(text, group, m))
                    .find_map(|id| installed(&id))
            })
        };
        let declared = || {
            apps.iter()
                .enumerate()
                .filter(|(_, a)| {
                    a.shown
                        && a.mime_types
                            .iter()
                            .any(|m| names.contains(&m.to_ascii_lowercase()))
                })
                .min_by_key(|(_, a)| a.name.to_lowercase())
                .map(|(i, _)| i)
        };
        let found = listed("Default Applications")
            .or_else(|| listed("Added Associations"))
            .or_else(declared);
        if found.is_some() {
            return found;
        }
    }
    None
}

/// File verbs a file of `mime` gets besides Open: Print for text, images and PDFs when `lp` is installed, Edit for text.
pub fn verbs(mime: Option<&str>, has_lp: bool) -> (bool, bool) {
    let Some(mime) = mime else {
        return (false, false);
    };
    let text = mime.starts_with("text/");
    let printable = text || mime.starts_with("image/") || mime == "application/pdf";
    (printable && has_lp, text)
}

pub fn has_lp() -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("lp").is_file()))
}

/// Make `id` the default program for `mime` in the user's `mimeapps.list`, keeping the rest.
pub fn set_default_app(mime: &str, id: &str) -> std::io::Result<()> {
    edit_defaults(&[(mime.to_string(), Some(id.to_string()))])
}

/// Make `edits` in the user's `mimeapps.list` ([`with_edits`]). The file is read just before the write; an unreadable one is left alone.
pub fn edit_defaults(edits: &[(String, Option<String>)]) -> std::io::Result<()> {
    let path = user_mimeapps()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::ops::write_atomic(&path, with_edits(&text, edits).as_bytes())
}

/// `text` with `edits` made in turn: `(mime, Some(id))` sets the default ([`with_default`]), `(mime, None)` drops the user's default ([`without_default`]).
pub fn with_edits(text: &str, edits: &[(String, Option<String>)]) -> String {
    edits
        .iter()
        .fold(text.to_string(), |t, (mime, id)| match id {
            Some(id) => with_default(&t, mime, id),
            None => without_default(&t, mime),
        })
}

pub fn without_default(text: &str, mime: &str) -> String {
    let mut in_defaults = false;
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| {
            let t = l.trim();
            if t.starts_with('[') {
                in_defaults = t == "[Default Applications]";
                return true;
            }
            !(in_defaults && t.split_once('=').is_some_and(|(k, _)| k.trim() == mime))
        })
        .collect();
    if kept.is_empty() {
        return String::new();
    }
    let mut s = kept.join("\n");
    s.push('\n');
    s
}

/// `text` with `mime=id` set under `[Default Applications]`: replaced where it exists, else added, the section created when missing.
pub fn with_default(text: &str, mime: &str, id: &str) -> String {
    let line = format!("{mime}={id}");
    let mut out: Vec<String> = Vec::new();
    let mut in_defaults = false;
    let mut placed = false;
    let mut seen_section = false;
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            if in_defaults && !placed {
                let at = out.len() - out.iter().rev().take_while(|l| l.trim().is_empty()).count();
                out.insert(at, line.clone());
                placed = true;
            }
            in_defaults = t == "[Default Applications]";
            seen_section |= in_defaults;
        } else if in_defaults && t.split_once('=').is_some_and(|(k, _)| k.trim() == mime) {
            if !placed {
                out.push(line.clone());
                placed = true;
            }
            continue;
        }
        out.push(l.to_string());
    }
    if !placed {
        if seen_section {
            out.push(line);
        } else {
            let mut head = vec!["[Default Applications]".to_string(), line, String::new()];
            head.extend(out);
            out = head;
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

pub fn default_in_list(text: &str, mime: &str) -> Option<String> {
    ids_in_list(text, "Default Applications", mime)
        .into_iter()
        .next()
}

fn ids_in_list(text: &str, group: &str, mime: &str) -> Vec<String> {
    let mut in_group = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[') {
            in_group = name.strip_suffix(']') == Some(group);
            continue;
        }
        if !in_group {
            continue;
        }
        if let Some((k, v)) = line.split_once('=')
            && k.trim().eq_ignore_ascii_case(mime)
        {
            return v
                .split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
        }
    }
    Vec::new()
}

/// [`format_size`] without the bracketed byte count: "19.8 GB".
pub fn short_size(bytes: u64) -> String {
    let full = format_size(bytes);
    match full.split_once(" (") {
        Some((short, _)) => short.to_string(),
        None => full,
    }
}

/// The size text: "0 bytes" under a kilobyte, else three significant digits of KB / MB / GB, cut not rounded, then the byte count: "1.23 KB (1,260 bytes)".
pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let units = ["KB", "MB", "GB", "TB"];
    let b = u128::from(bytes);
    let mut div: u128 = 1024;
    let mut unit = 0;
    while b / div >= 1000 && unit + 1 < units.len() {
        div *= 1024;
        unit += 1;
    }
    let h = b * 100 / div;
    let short = if h < 1_000 {
        format!("{}.{:02}", h / 100, h % 100)
    } else if h < 10_000 {
        format!("{}.{}", h / 100, h % 100 / 10)
    } else {
        format!("{}", h / 100)
    };
    format!("{short} {} ({} bytes)", units[unit], thousands(bytes))
}

pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn format_contains(files: u64, folders: u64) -> String {
    format!(
        "{} {}, {} {}",
        thousands(files),
        if files == 1 { "File" } else { "Files" },
        thousands(folders),
        if folders == 1 { "Folder" } else { "Folders" }
    )
}

/// The Properties sheet's date: "Today, September 10, 2026, 9:32:37 PM" (24-hour when `is_12h` is false).
pub fn format_time(secs: i64, is_12h: bool) -> String {
    long_date(secs, is_12h, true)
}

/// The same date without seconds, as the task pane's Details section writes it.
pub fn format_pane_time(secs: i64, is_12h: bool) -> String {
    long_date(secs, is_12h, false)
}

/// A local date's day number from 1970-01-01, so subtracting two gives the days between across month and year boundaries.
fn civil_day(t: &libc::tm) -> i64 {
    let (y, m, d) = (
        t.tm_year as i64 + 1900,
        t.tm_mon as i64 + 1,
        t.tm_mday as i64,
    );
    // March-based years, so a leap day falls at the end.
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The long date: "Today" or "Yesterday" where they apply, else the weekday, then month, day, year and clock in the LC_TIME locale ([`crate::locale::strftime`]). `seconds` adds `:%S`.
fn long_date(secs: i64, is_12h: bool, seconds: bool) -> String {
    let then = crate::locale::localtime(secs);
    let today = crate::locale::localtime(now_secs());
    // Days apart on the civil calendar rather than by seconds, so a DST shift cannot move a date across midnight.
    let named_day = match civil_day(&today) - civil_day(&then) {
        0 => "Today",
        1 => "Yesterday",
        _ => "%A",
    };
    let clock = match (is_12h, seconds) {
        (true, true) => "%l:%M:%S %p",
        (true, false) => "%l:%M %p",
        (false, true) => "%H:%M:%S",
        (false, false) => "%H:%M",
    };
    let fmt = std::ffi::CString::new(format!("{named_day}, %B %d, %Y, {clock}"))
        .expect("the format holds no NUL");
    crate::locale::strftime(&then, &fmt).replace(",  ", ", ")
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stopped_walk_gives_up() {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::write(dir.path().join("a"), "1234").expect("write");
        let go = AtomicBool::new(false);
        assert_eq!(
            folder_totals_unless(dir.path(), &go).map(|t| (t.files, t.size)),
            Some((1, 4))
        );
        let stop = AtomicBool::new(true);
        assert_eq!(folder_totals_unless(dir.path(), &stop), None);
    }

    #[test]
    fn short_sizes_drop_the_byte_count() {
        assert_eq!(short_size(21_365_735_424), "19.8 GB");
        assert_eq!(short_size(1260), "1.23 KB");
        assert_eq!(short_size(512), "512 bytes");
    }

    #[test]
    fn sizes_and_counts_read_as_byte_counts() {
        assert_eq!(format_size(0), "0 bytes");
        assert_eq!(format_size(1023), "1023 bytes");
        assert_eq!(format_size(1260), "1.23 KB (1,260 bytes)");
        assert_eq!(format_size(12_800), "12.5 KB (12,800 bytes)");
        assert_eq!(format_size(512_000), "500 KB (512,000 bytes)");
        assert_eq!(format_size(1_572_864), "1.50 MB (1,572,864 bytes)");
        assert_eq!(format_size(3 << 30), "3.00 GB (3,221,225,472 bytes)");
        assert_eq!(format_size(6_500), "6.34 KB (6,500 bytes)");
        assert_eq!(format_size(150_000), "146 KB (150,000 bytes)");
        assert_eq!(format_size(1_234_567), "1.17 MB (1,234,567 bytes)");
        assert_eq!(format_size(12_345_678), "11.7 MB (12,345,678 bytes)");
        assert_eq!(format_size(13_738_771), "13.1 MB (13,738,771 bytes)");
        assert_eq!(format_contains(1, 0), "1 File, 0 Folders");
        assert_eq!(format_contains(1200, 1), "1,200 Files, 1 Folder");
    }

    #[test]
    fn hidden_renames_a_name_that_is_not_utf8_by_its_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let t = tempfile::tempdir().unwrap();
        let plain = t.path().join(std::ffi::OsStr::from_bytes(b"caf\xe9.txt"));
        let dotted = t.path().join(std::ffi::OsStr::from_bytes(b".caf\xe9.txt"));
        std::fs::write(&plain, "x").unwrap();
        let w = write_set(vec![plain.clone()], AttrMoves::default(), Some(true), false);
        assert_eq!(w.now, std::slice::from_ref(&dotted));
        assert!(dotted.exists());
        let w = write_set(
            vec![dotted.clone()],
            AttrMoves::default(),
            Some(false),
            false,
        );
        assert_eq!(w.now, std::slice::from_ref(&plain));
        assert!(plain.exists());
    }

    #[test]
    fn hiding_adds_one_dot_and_showing_takes_it() {
        assert_eq!(hidden_name("notes.txt".as_ref(), true), ".notes.txt");
        assert_eq!(hidden_name(".notes.txt".as_ref(), true), ".notes.txt");
        assert_eq!(hidden_name(".notes.txt".as_ref(), false), "notes.txt");
        assert_eq!(hidden_name("notes.txt".as_ref(), false), "notes.txt");
    }

    #[test]
    fn thousands_groups_by_threes_from_the_right() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(361_672), "361,672");
        assert_eq!(thousands(2_097_152), "2,097,152");
        assert_eq!(thousands(1_234_567_890), "1,234,567,890");
    }

    #[test]
    fn type_names_and_defaults_come_from_mime_data() {
        assert_eq!(
            comment_of("<mime-type>\n  <comment>Plain text document</comment>\n  <comment xml:lang=\"uk\">x</comment>\n").as_deref(),
            Some("Plain text document")
        );
        assert_eq!(generic_type_name(Path::new("/x/a.tar.gz")), "GZ File");
        assert_eq!(generic_type_name(Path::new("/x/Makefile")), "File");
        let list = "[Added Associations]\ntext/plain=nvim.desktop;\n[Default Applications]\ntext/plain=micro.desktop;vim.desktop\nimage/png=\n";
        assert_eq!(
            default_in_list(list, "text/plain").as_deref(),
            Some("micro.desktop")
        );
        assert_eq!(default_in_list(list, "image/png"), None);
        assert_eq!(default_in_list(list, "text/html"), None);
        assert_eq!(verbs(Some("text/plain"), true), (true, true));
        assert_eq!(verbs(Some("text/x-rust"), false), (false, true));
        assert_eq!(verbs(Some("image/png"), true), (true, false));
        assert_eq!(verbs(Some("application/pdf"), true), (true, false));
        assert_eq!(verbs(Some("application/zip"), true), (false, false));
        assert_eq!(verbs(None, true), (false, false));
        let list = "[Added Associations]\ntext/plain=nvim.desktop;\n[Default Applications]\ntext/plain=micro.desktop;vim.desktop\nimage/png=eog.desktop\n";
        let out = with_default(list, "text/plain", "vim.desktop");
        assert_eq!(
            default_in_list(&out, "text/plain").as_deref(),
            Some("vim.desktop")
        );
        assert_eq!(
            default_in_list(&out, "image/png").as_deref(),
            Some("eog.desktop")
        );
        assert_eq!(
            out.matches("text/plain=").count(),
            2,
            "the association stays"
        );
        let out = with_default(list, "text/html", "firefox.desktop");
        assert_eq!(
            default_in_list(&out, "text/html").as_deref(),
            Some("firefox.desktop")
        );
        let out = with_default("", "text/html", "firefox.desktop");
        assert!(out.starts_with("[Default Applications]\ntext/html=firefox.desktop\n"));
    }

    #[test]
    fn edits_keep_the_rest_of_the_list() {
        let list = "# mine\n\
                    [Added Associations]\n\
                    text/plain=nvim.desktop;\n\
                    \n\
                    [Default Applications]\n\
                    image/png=eog.desktop\n\
                    text/plain=micro.desktop\n\
                    \n\
                    [X-Unknown Section]\n\
                    some=thing\n\
                    [Removed Associations]\n\
                    text/plain=gedit.desktop;\n";
        let edits = |e: &[(&str, Option<&str>)]| -> Vec<(String, Option<String>)> {
            e.iter()
                .map(|(m, id)| (m.to_string(), id.map(String::from)))
                .collect()
        };
        let set = with_edits(
            list,
            &edits(&[
                ("text/plain", Some("zed.desktop")),
                ("x-scheme-handler/https", Some("firefox.desktop")),
            ]),
        );
        assert_eq!(
            set,
            list.replace("text/plain=micro.desktop", "text/plain=zed.desktop")
                .replace(
                    "\n\n[X-Unknown",
                    "\nx-scheme-handler/https=firefox.desktop\n\n[X-Unknown"
                ),
            "replaced in place, the new type added at the section's end"
        );
        let dropped = with_edits(
            &set,
            &edits(&[
                ("x-scheme-handler/https", None),
                ("text/plain", Some("micro.desktop")),
            ]),
        );
        assert_eq!(dropped, list, "the round trip gives the file back");
        let restored = without_default(list, "text/plain");
        assert_eq!(default_in_list(&restored, "text/plain"), None);
        assert_eq!(
            restored
                .lines()
                .filter(|l| l.starts_with("text/plain="))
                .count(),
            2,
            "the added and removed associations stay"
        );
        assert_eq!(without_default("", "text/plain"), "");
    }

    /// A caller that knows the path is a file gets its program without a stat, which a file on a
    /// mount that does not answer would hold: even a folder by that name opens as a file.
    #[test]
    fn open_file_asks_the_file_system_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let viewer = AppEntry {
            exec_line: "viewer %F".into(),
            ..app("viewer.desktop", &["image/svg+xml"])
        };
        let open = |d| open_file(dir.path(), d, "foot");
        assert_eq!(
            open(Some(&viewer)),
            Opening {
                argv: vec!["viewer".into(), dir.path().as_os_str().to_owned()],
                app: Some("viewer.desktop".into()),
            },
            "in the program's app scope"
        );
        assert_eq!(open(None).argv[0], "xdg-open");
        assert_eq!(open(None).app, None);
    }

    #[test]
    fn open_in_looks_up_the_types_default_program_and_runs_as_it() {
        let dir = tempfile::tempdir().unwrap();
        let song = dir.path().join("song.rtest");
        let other = dir.path().join("song.rtother");
        let db = apps::MimeDb::parse(["50:application/x-reprise-test:*.rtest\n"]);
        let player = AppEntry {
            exec_line: "player %f".into(),
            ..app("player.desktop", &["application/x-reprise-test"])
        };
        let open = |p: &Path| open_in(p, &db, std::slice::from_ref(&player), "foot");
        assert_eq!(
            open(&song),
            Opening {
                argv: vec!["player".into(), song.clone().into_os_string()],
                app: Some("player.desktop".into()),
            }
        );
        assert_eq!(
            open(&other),
            Opening {
                argv: vec!["xdg-open".into(), other.into_os_string()],
                app: None,
            }
        );
        let folder = open(dir.path());
        assert_eq!(folder.argv[0], "reprise-explorer");
        assert_eq!(folder.app, None);
    }

    fn app(id: &str, mimes: &[&str]) -> AppEntry {
        AppEntry {
            name: id.into(),
            exec_line: String::new(),
            terminal: false,
            id: id.into(),
            icon: None,
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: mimes.iter().map(|m| m.to_string()).collect(),
            shown: true,
        }
    }

    fn pick<'a>(
        lists: &[&str],
        tree: &MimeTree,
        mime: &str,
        apps: &'a [AppEntry],
    ) -> Option<&'a str> {
        let lists: Vec<String> = lists.iter().map(|s| s.to_string()).collect();
        default_in_lists(&lists, tree, mime, apps).map(|i| apps[i].id.as_str())
    }

    const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";

    #[test]
    fn a_type_s_own_programs_come_before_a_parent_s_default() {
        let tree = MimeTree::parse(
            ["application/x-pdf application/pdf\n"],
            [format!(
                "image/svg+xml application/xml\n\
                 application/xml text/plain\n\
                 {DOCX} application/zip\n\
                 application/epub+zip application/zip\n\
                 application/x-foo application/zip\n"
            )
            .as_str()],
        );
        let apps = vec![
            app("notepad.desktop", &["text/plain"]),
            app("viewer.desktop", &["image/svg+xml"]),
            app("ark.desktop", &["application/zip"]),
            app("writer.desktop", &[DOCX]),
            app("reader.desktop", &["application/epub+zip"]),
            app("inkscape.desktop", &[]),
            app("evince.desktop", &["application/x-pdf"]),
        ];
        let system =
            "[Default Applications]\napplication/xml=notepad.desktop\ntext/plain=notepad.desktop\n";
        let user = "[Default Applications]\napplication/zip=ark.desktop\n";
        let lists = [user, system];
        let at = |mime| pick(&lists, &tree, mime, &apps);
        assert_eq!(at("image/svg+xml"), Some("viewer.desktop"));
        assert_eq!(at("application/xml"), Some("notepad.desktop"));
        assert_eq!(at(DOCX), Some("writer.desktop"));
        assert_eq!(at("application/epub+zip"), Some("reader.desktop"));
        assert_eq!(
            at("application/x-foo"),
            Some("ark.desktop"),
            "a type with no program of its own takes its parent's default"
        );
        assert_eq!(
            at("application/pdf"),
            Some("evince.desktop"),
            "a program declaring an alias handles the type"
        );
        let added = "[Added Associations]\nimage/svg+xml=gone.desktop;inkscape.desktop;\n";
        assert_eq!(
            pick(&[added, user, system], &tree, "image/svg+xml", &apps),
            Some("inkscape.desktop"),
            "an added association comes before a declared type"
        );
        let by_alias = "[Default Applications]\napplication/x-pdf=notepad.desktop\n";
        assert_eq!(
            pick(&[by_alias], &tree, "application/pdf", &apps),
            Some("notepad.desktop"),
            "a default set under an alias beats the type's programs"
        );
    }

    #[test]
    fn a_type_with_no_program_of_its_own_takes_its_parent_s_default() {
        let tree = MimeTree::parse([], ["text/x-python application/x-executable\n"]);
        let apps = vec![
            app("idle.desktop", &["text/x-python"]),
            app("mousepad.desktop", &["text/plain"]),
            app("zed.desktop", &[]),
        ];
        let user = "[Default Applications]\ntext/plain=zed.desktop\n";
        let gone = "[Default Applications]\ntext/x-python=gone.desktop\n";
        let system = "[Default Applications]\ntext/x-python=mousepad.desktop\n";
        assert_eq!(
            pick(&[user], &tree, "text/x-python", &apps[1..]),
            Some("zed.desktop"),
            "with no program of its own, the text/plain default"
        );
        assert_eq!(
            pick(&[user], &tree, "text/x-python", &apps),
            Some("idle.desktop"),
            "a program declaring the type beats the text/plain default"
        );
        assert_eq!(
            pick(&[gone, user, system], &tree, "text/x-python", &apps),
            Some("mousepad.desktop"),
            "the type's own default wins, skipping an id that is not installed"
        );
        assert_eq!(
            pick(&[], &tree, "text/x-python", &apps[1..]),
            Some("mousepad.desktop"),
            "without a default, a text/plain program"
        );
        assert_eq!(pick(&[user], &tree, "image/png", &apps), None);
    }

    #[test]
    fn inspect_reads_a_file_and_a_folder() {
        let base = std::env::temp_dir().join(format!("reprise-props-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let sub = base.join("Reports").join("2026");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(base.join("Reports").join("a.txt"), "hello").unwrap();
        std::fs::write(sub.join("b.txt"), "hello world").unwrap();
        let notes = base.join(".notes.txt");
        std::fs::write(&notes, "12345").unwrap();
        let f = inspect(&notes, ".notes.txt", &[]).unwrap();
        assert!(!f.is_dir && f.hidden && !f.listed && !f.read_only);
        std::fs::write(base.join(".hidden"), "Reports\n").unwrap();
        let listed = glance(&base.join("Reports"), "Reports", &[]).unwrap();
        assert!(listed.hidden && listed.listed);
        std::fs::remove_file(base.join(".hidden")).unwrap();
        assert_eq!(f.size, 5);
        assert!(f.on_disk >= 5 || f.on_disk == 0);
        assert_eq!(f.location, base.to_string_lossy());
        assert!(f.contains.is_none() && f.modified > 0);
        let d = inspect(&base.join("Reports"), "Reports", &[]).unwrap();
        assert!(d.is_dir);
        assert_eq!(d.type_name, "File Folder");
        assert_eq!(d.contains, Some((2, 1)));
        assert_eq!(d.size, 16);
        assert!(d.opens_with.is_none());
        let link = base.join("Shortcut to Reports");
        std::os::unix::fs::symlink(base.join("Reports"), &link).unwrap();
        let l = inspect(&link, "Shortcut to Reports", &[]).unwrap();
        assert_eq!(l.type_name, "Shortcut");
        std::fs::remove_dir_all(&base).unwrap();
        assert!(!l.is_dir && l.contains.is_none());
        assert_eq!(l.size, base.join("Reports").as_os_str().len() as u64);
        assert_eq!(l.description.as_deref(), Some("Reports"));
        let sc = l.launcher.expect("a symlink's keys");
        assert_eq!(sc.kind, crate::launcher::Kind::Symlink);
        assert_eq!(sc.target_type, "File Folder");
    }

    #[test]
    fn a_web_link_is_an_internet_shortcut() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ex.desktop");
        crate::apps::write_launcher(
            &path,
            &crate::apps::link_text("Ex", "https://example.org/").unwrap(),
        )
        .unwrap();
        let f = glance(&path, "Ex", &[]).unwrap();
        assert_eq!(f.type_name, "Internet Shortcut");
        assert_eq!(f.description.as_deref(), Some("Ex"));
        assert!(f.is_shortcut && f.opens_with.is_none());
        let l = f.launcher.expect("a web link's keys");
        assert_eq!(l.kind, crate::launcher::Kind::Web);
        assert_eq!(l.fields()[0], "https://example.org/");
    }

    #[test]
    fn an_untrusted_launcher_is_the_plain_file_it_lists_as() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let app = "[Desktop Entry]\nType=Application\nName=Terminal\nExec=foot\n";
        let plain = dir.path().join("plain.desktop");
        std::fs::write(&plain, app).unwrap();
        let trusted = dir.path().join("app.desktop");
        crate::apps::write_launcher(&trusted, app).unwrap();
        let db = MimeDb::empty();
        let listed = |p: &Path| crate::listing::item_at(p, &db).kind;
        for mode in [0o644, 0o755] {
            std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(mode)).unwrap();
            let f = glance_in(&plain, "plain.desktop", &[], &db).unwrap();
            assert!(!f.is_shortcut, "mode {mode:o}");
            assert_eq!((f.launcher, f.description), (None, None));
            assert_eq!(f.type_name, listed(&plain));
            assert_ne!(f.type_name, crate::listing::SHORTCUT);
        }
        let f = glance_in(&trusted, "Terminal", &[], &db).unwrap();
        assert!(f.is_shortcut && f.launcher.is_some());
        assert_eq!(f.type_name, listed(&trusted));
    }

    #[test]
    fn a_set_adds_up_its_items_and_names_what_they_share() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        std::fs::write(base.join("a.txt"), "12345678").unwrap();
        std::fs::write(base.join("b.txt"), "1234").unwrap();
        std::fs::create_dir_all(base.join("Reports/old")).unwrap();
        std::fs::write(base.join("Reports/x.txt"), "123").unwrap();
        let a = inspect(&base.join("a.txt"), "a", &[]).unwrap();
        let b = inspect(&base.join("b.txt"), "b", &[]).unwrap();
        let files = glance_set(&[a.clone(), b.clone()]);
        assert_eq!((files.files, files.folders, files.size), (2, 0, 12));
        assert!(!files.has_dir && !files.walking);
        assert_eq!(files.count_text(), "2 Files, 0 Folders");
        assert_eq!(files.type_text(), format!("All of type {}", a.type_name));
        let loc = base.to_string_lossy();
        assert_eq!(files.location_text(), format!("All in {loc}"));
        assert_eq!((files.read_only, files.hidden), (Some(false), Some(false)));

        let reports = glance(&base.join("Reports"), "Reports", &[]).unwrap();
        let mut mixed = glance_set(&[a.clone(), reports.clone()]);
        assert!(mixed.has_dir && mixed.walking);
        assert_eq!((mixed.files, mixed.folders), (1, 1));
        mixed.add_totals(folder_totals(&base.join("Reports")));
        assert!(!mixed.walking);
        assert_eq!(mixed.count_text(), "2 Files, 2 Folders");
        assert_eq!(mixed.size, 11);
        assert_eq!(mixed.type_text(), "Multiple Types");
        let walked = inspect(&base.join("Reports"), "Reports", &[]).unwrap();
        assert_eq!(
            glance_set(&[a.clone(), walked]),
            mixed,
            "inspect walks up front"
        );

        let x = inspect(&base.join("Reports/x.txt"), "x", &[]).unwrap();
        assert_eq!(
            glance_set(&[a.clone(), x]).location_text(),
            "Various Folders"
        );

        set_read_only(&base.join("b.txt"), true).unwrap();
        let b = glance(&base.join("b.txt"), "b", &[]).unwrap();
        assert_eq!(glance_set(&[a.clone(), b.clone()]).read_only, None);

        if a.robots.is_some() {
            assert_eq!(
                glance_set(&[a.clone(), b.clone()]).robots,
                Some([Some(true), Some(true)])
            );
            crate::meta::set_robots(&base.join("b.txt"), crate::meta::BACKUP, false).unwrap();
            let b = glance(&base.join("b.txt"), "b", &[]).unwrap();
            assert_eq!(glance_set(&[a, b]).robots, Some([None, Some(true)]));
        }
    }

    #[test]
    fn attribute_changes_read_as_one_line() {
        let moves = AttrMoves {
            read_only: Some(true),
            backup: None,
            index: Some(false),
            ..Default::default()
        };
        assert_eq!(moves.verbs(Some(true)), "set read-only, hide, unset index");
        assert!(AttrMoves::default().is_empty());
    }

    #[test]
    fn glance_in_types_the_file_from_the_given_tables() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("song.rtest");
        std::fs::write(&file, "x").unwrap();
        let db = apps::MimeDb::parse(["50:application/x-reprise-test:*.rtest\n"]);
        let player = AppEntry {
            name: "Player".into(),
            exec_line: "player %f".into(),
            terminal: false,
            id: "player.desktop".into(),
            icon: None,
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: vec!["application/x-reprise-test".into()],
            shown: true,
        };
        let f = glance_in(&file, "song", &[player], &db).unwrap();
        assert_eq!(f.opens_with.as_deref(), Some("Player"));
        assert_eq!(f.type_name, "RTEST File");
    }

    #[test]
    fn a_symlink_to_a_document_shows_what_opens_its_target() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("song.rtest"), "x").unwrap();
        let link = dir.path().join("Song");
        std::os::unix::fs::symlink("song.rtest", &link).unwrap();
        let db = apps::MimeDb::parse(["50:application/x-reprise-test:*.rtest\n"]);
        let player = AppEntry {
            name: "Player".into(),
            exec_line: "player %f".into(),
            terminal: false,
            id: "player.desktop".into(),
            icon: Some("audio-player".into()),
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: vec!["application/x-reprise-test".into()],
            shown: true,
        };
        let f = glance_in(&link, "Song", &[player], &db).unwrap();
        assert_eq!(f.type_name, "Shortcut");
        assert_eq!(f.opens_with.as_deref(), Some("Player"));
        assert_eq!(f.opens_with_icon.as_deref(), Some("audio-player"));
        assert_eq!(f.description, None);
        let l = f.launcher.expect("a symlink's keys");
        assert_eq!(l.kind, crate::launcher::Kind::Symlink);
        assert_eq!(l.target_type, "RTEST File");
        let bare = glance_in(&link, "Song", &[], &db).unwrap();
        assert_eq!(bare.opens_with, None);
        assert_eq!(bare.description.as_deref(), Some("song.rtest"));
    }

    /// Open runs an executable script, so its sheet names the script instead of the editor its type
    /// opens in; the same script without the execute bit opens there.
    #[test]
    fn an_executable_script_opens_with_nothing_and_is_described_by_its_name() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("hi.sh");
        std::fs::write(&f, "#!/bin/sh\n").unwrap();
        let db = apps::MimeDb::parse(["50:application/x-shellscript:*.sh\n"]);
        let editor = AppEntry {
            name: "Editor".into(),
            exec_line: "editor %f".into(),
            terminal: false,
            id: "editor.desktop".into(),
            icon: Some("text-editor".into()),
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: vec!["application/x-shellscript".into()],
            shown: true,
        };
        let apps = [editor];
        std::fs::set_permissions(&f, PermissionsExt::from_mode(0o644)).unwrap();
        let plain = glance_in(&f, "hi", &apps, &db).unwrap();
        assert_eq!(plain.opens_with.as_deref(), Some("Editor"));
        assert_eq!(plain.description, None);
        std::fs::set_permissions(&f, PermissionsExt::from_mode(0o755)).unwrap();
        let program = glance_in(&f, "hi", &apps, &db).unwrap();
        assert_eq!(program.opens_with, None);
        assert_eq!(program.opens_with_icon, None);
        assert_eq!(program.description.as_deref(), Some("hi"));
    }

    #[test]
    fn read_only_clears_and_restores_the_write_bits() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("reprise-attrs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let f = base.join("a.txt");
        std::fs::write(&f, "x").unwrap();
        std::fs::set_permissions(&f, PermissionsExt::from_mode(0o664)).unwrap();
        set_read_only(&f, true).unwrap();
        let mode = || std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(), 0o444);
        assert!(inspect(&f, "a", &[]).unwrap().read_only);
        set_read_only(&f, false).unwrap();
        assert_eq!(mode(), 0o644, "only the owner's write bit comes back");
        assert!(!inspect(&f, "a", &[]).unwrap().read_only);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn executable_sets_execute_where_a_class_can_read_and_clears_all() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("reprise-exec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let f = base.join("hi.sh");
        std::fs::write(&f, "#!/bin/sh\n").unwrap();
        let mode = || std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
        for (from, want) in [
            (0o644, 0o755),
            (0o640, 0o750),
            (0o600, 0o700),
            (0o604, 0o705),
        ] {
            std::fs::set_permissions(&f, PermissionsExt::from_mode(from)).unwrap();
            let info = inspect(&f, "hi", &[]).unwrap();
            assert!(!info.executable && info.can_set_executable, "{from:o}");
            set_executable(&f, true).unwrap();
            assert_eq!(mode(), want, "{from:o} made executable");
            assert!(inspect(&f, "hi", &[]).unwrap().executable);
            set_executable(&f, false).unwrap();
            assert_eq!(mode(), from, "{want:o} made not executable");
        }
        let moves = AttrMoves {
            executable: Some(true),
            ..Default::default()
        };
        write_item(&f, &moves, None, false).unwrap();
        assert_eq!(mode(), 0o705, "write_item writes the Executable move");
        let folder = inspect(&base, "base", &[]).unwrap();
        assert!(!folder.executable && !folder.can_set_executable, "a folder");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn the_executable_box_greys_off_the_users_files_and_on_mounts_that_cannot_take_it() {
        // SAFETY: geteuid has no preconditions.
        let me = unsafe { libc::geteuid() };
        let mine = Mount::default();
        assert!(can_set_executable(me, mine));
        if me != 0 {
            assert!(!can_set_executable(me + 1, mine), "another user's file");
        }
        for mount in [
            Mount {
                read_only: true,
                ..mine
            },
            Mount {
                noexec: true,
                ..mine
            },
            Mount {
                fixed_modes: true,
                ..mine
            },
        ] {
            assert!(!can_set_executable(me, mount), "{mount:?}");
        }
        assert!(
            !Mount::of(&std::env::temp_dir()).fixed_modes,
            "the temp folder keeps modes"
        );
    }

    #[test]
    fn attr_verbs_list_read_only_then_hidden() {
        assert_eq!(attr_verbs(Some(true), None), "set read-only");
        assert_eq!(attr_verbs(None, Some(true)), "hide");
        assert_eq!(attr_verbs(None, Some(false)), "unhide");
        assert_eq!(attr_verbs(Some(false), Some(true)), "unset read-only, hide");
    }

    #[test]
    fn the_hidden_tree_lists_names_and_never_renames() {
        let base = tempfile::tempdir().unwrap();
        let tree = base.path().join("tree");
        let outside = base.path().join("outside");
        let deep = tree.join("sub").join("deep");
        let git = tree.join(".git");
        for d in [&deep, &git, &outside] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(tree.join("a.txt"), "x").unwrap();
        std::fs::write(tree.join(".config"), "x").unwrap();
        std::fs::write(deep.join("b.txt"), "x").unwrap();
        std::fs::write(git.join("HEAD"), "x").unwrap();
        std::fs::write(outside.join("d.txt"), "x").unwrap();
        std::fs::write(tree.join("sub").join(".hidden"), "keep me\n").unwrap();
        std::os::unix::fs::symlink(&outside, tree.join("link")).unwrap();
        let list = |d: &Path| std::fs::read_to_string(d.join(".hidden")).ok();

        set_hidden_tree(&tree, true).unwrap();
        assert_eq!(list(&tree).as_deref(), Some("a.txt\nlink\nsub\n"));
        assert_eq!(list(&tree.join("sub")).as_deref(), Some("keep me\ndeep\n"));
        assert_eq!(list(&deep).as_deref(), Some("b.txt\n"));
        assert_eq!(list(&git), None, "a dotted folder is not entered");
        assert_eq!(list(&outside), None, "a linked folder is not entered");
        assert!(crate::hidden::is_hidden(&deep.join("b.txt")));
        assert!(
            !crate::hidden::is_hidden(&tree),
            "the folder is left to its rename"
        );
        set_hidden_tree(&tree, true).unwrap();
        assert_eq!(
            list(&deep).as_deref(),
            Some("b.txt\n"),
            "hiding twice lists once"
        );

        set_hidden_tree(&tree, false).unwrap();
        assert_eq!(list(&tree), None);
        assert_eq!(list(&deep), None);
        assert_eq!(list(&tree.join("sub")).as_deref(), Some("keep me\n"));
        for p in [tree.join(".config"), git.join("HEAD"), deep.join("b.txt")] {
            assert!(p.exists(), "{} keeps its name", p.display());
        }
        assert_eq!(std::fs::read_dir(&git).unwrap().count(), 1);

        let file = base.path().join("f.txt");
        std::fs::write(&file, "x").unwrap();
        set_hidden_tree(&file, true).unwrap();
        assert_eq!(list(base.path()), None, "a file has nothing under it");
    }

    #[test]
    fn read_only_and_hidden_land_together_down_a_tree() {
        let base = tempfile::tempdir().unwrap();
        let tree = base.path().join("tree");
        std::fs::create_dir_all(tree.join("sub")).unwrap();
        std::fs::write(tree.join("sub").join("a.txt"), "x").unwrap();
        let list = |d: &Path| std::fs::read_to_string(d.join(".hidden")).ok();
        let lock = AttrMoves {
            read_only: Some(true),
            ..Default::default()
        };
        let written = write_item(&tree, &lock, Some(true), true);
        let listed = (list(&tree), list(&tree.join("sub")));
        let locked = std::fs::metadata(&tree).unwrap().permissions().readonly();
        let again = write_item(&tree, &AttrMoves::default(), Some(false), true);
        set_read_only_tree(&tree, false).unwrap();
        written.unwrap();
        assert_eq!(listed.0.as_deref(), Some("sub\n"));
        assert_eq!(listed.1.as_deref(), Some("a.txt\n"));
        assert!(locked, "the folder is read-only after");
        assert!(again.is_err(), "a read-only folder's list is not written");
    }

    #[test]
    fn the_read_only_tree_leaves_symlinked_files_and_folders_alone() {
        use std::os::unix::fs::PermissionsExt;
        let base = tempfile::tempdir().unwrap();
        let tree = base.path().join("tree");
        let outside = base.path().join("outside");
        std::fs::create_dir_all(tree.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(tree.join("sub").join("a.txt"), "x").unwrap();
        std::fs::write(outside.join("b.txt"), "x").unwrap();
        std::os::unix::fs::symlink(outside.join("b.txt"), tree.join("b-link")).unwrap();
        std::os::unix::fs::symlink(&outside, tree.join("sub").join("outside-link")).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        for p in [&tree, &tree.join("sub"), &outside] {
            std::fs::set_permissions(p, PermissionsExt::from_mode(0o755)).unwrap();
        }
        for p in [tree.join("sub").join("a.txt"), outside.join("b.txt")] {
            std::fs::set_permissions(&p, PermissionsExt::from_mode(0o644)).unwrap();
        }
        set_read_only_tree(&tree, true).unwrap();
        assert_eq!(mode(&tree), 0o555);
        assert_eq!(mode(&tree.join("sub")), 0o555);
        assert_eq!(mode(&tree.join("sub").join("a.txt")), 0o444);
        assert_eq!(mode(&outside), 0o755, "a linked folder keeps its mode");
        assert_eq!(
            mode(&outside.join("b.txt")),
            0o644,
            "a linked file keeps its mode"
        );
        set_read_only_tree(&tree, false).unwrap();
        assert_eq!(mode(&tree.join("sub")), 0o755);
        assert_eq!(mode(&tree.join("sub").join("a.txt")), 0o644);
    }

    #[test]
    fn times_name_today_and_carry_seconds() {
        let now = now_secs();
        let today = format_time(now, crate::locale::is_12h());
        assert!(today.starts_with("Today, "), "{today}");
        assert_eq!(today.matches(':').count(), 2, "{today}");
        let old = format_time(now - 10 * 86_400, crate::locale::is_12h());
        assert!(!old.starts_with("Today"), "{old}");
    }

    #[test]
    fn the_day_distance_counts_across_months_and_years() {
        let day = |y: i32, m: i32, d: i32| {
            civil_day(&libc::tm {
                tm_sec: 0,
                tm_min: 0,
                tm_hour: 12,
                tm_mday: d,
                tm_mon: m - 1,
                tm_year: y - 1900,
                tm_wday: 0,
                tm_yday: 0,
                tm_isdst: 0,
                tm_gmtoff: 0,
                tm_zone: std::ptr::null(),
            })
        };
        assert_eq!(day(2027, 1, 1) - day(2026, 12, 31), 1, "new year");
        assert_eq!(day(2026, 10, 1) - day(2026, 9, 30), 1, "new month");
        assert_eq!(day(2024, 3, 1) - day(2024, 2, 29), 1, "leap day");
        assert_eq!(day(2026, 1, 1) - day(2025, 1, 1), 365);
        assert_eq!(day(1970, 1, 1), 0, "the epoch");
    }

    #[test]
    fn the_task_panes_date_names_yesterday_and_drops_the_seconds() {
        let now = now_secs();
        let is_12h = crate::locale::is_12h();
        let today = format_pane_time(now, is_12h);
        assert!(today.starts_with("Today, "), "{today}");
        assert_eq!(today.matches(':').count(), 1, "{today}");
        let old = format_pane_time(now - 10 * 86_400, is_12h);
        assert!(
            !old.starts_with("Today") && !old.starts_with("Yesterday"),
            "{old}"
        );
        // A day back is the day before, except within an hour of
        // midnight, where a DST shift can leave it on the same date.
        let hour = crate::locale::localtime(now).tm_hour;
        if (1..23).contains(&hour) {
            let yesterday = format_pane_time(now - 86_400, is_12h);
            assert!(yesterday.starts_with("Yesterday, "), "{yesterday}");
        }
    }
}
