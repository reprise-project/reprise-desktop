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

/// What a file's mount allows of its execute bit, from `statfs` (and mountinfo for fuse).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mount {
    pub read_only: bool,
    pub noexec: bool,
    /// FAT, exFAT or NTFS, or a `fuseblk` mount whose driver was not asked to keep modes: modes
    /// are fixed by the mount options.
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
        const FUSE: libc::__fsword_t = 0x6573_5546;
        let fixed_modes =
            FIXED_MODES.contains(&fs.f_type) || fs.f_type == FUSE && fuseblk_fixed_modes(path);
        Mount {
            read_only: vfs.f_flag & libc::ST_RDONLY != 0,
            noexec: vfs.f_flag & libc::ST_NOEXEC != 0,
            fixed_modes,
        }
    }
}

/// Whether `path` is on a `fuseblk` mount (ntfs-3g, exfat-fuse) whose driver keeps no modes. Only
/// ntfs-3g's `permissions` or `acl` keeps them; mountinfo does not show either, so the driver's
/// command line is read from /proc.
fn fuseblk_fixed_modes(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(dev) = std::fs::metadata(path).map(|m| m.dev()) else {
        return false;
    };
    let Some((source, target)) = std::fs::read_to_string("/proc/self/mountinfo")
        .ok()
        .and_then(|info| fuseblk_mount(&info, libc::major(dev), libc::minor(dev)))
    else {
        return false;
    };
    let Ok(procs) = std::fs::read_dir("/proc") else {
        return true;
    };
    !procs.flatten().any(|e| {
        std::fs::read(e.path().join("cmdline")).is_ok_and(|cmd| {
            let args: Vec<&str> = cmd
                .split(|&b| b == 0)
                .filter_map(|a| std::str::from_utf8(a).ok())
                .collect();
            keeps_modes(&args, &source, &target)
        })
    })
}

/// The source and mount point `mountinfo` (`/proc/self/mountinfo`) lists for device
/// `major:minor`, when it is a `fuseblk` mount.
fn fuseblk_mount(mountinfo: &str, major: u32, minor: u32) -> Option<(String, String)> {
    let dev = format!("{major}:{minor}");
    mountinfo.lines().find_map(|line| {
        let (head, tail) = line.split_once(" - ")?;
        let head: Vec<&str> = head.split(' ').collect();
        let mut tail = tail.split(' ');
        let (fstype, source) = (tail.next()?, tail.next()?);
        if head.get(2) != Some(&dev.as_str()) || fstype != "fuseblk" {
            return None;
        }
        Some((
            unescape_mountinfo(source)?,
            unescape_mountinfo(head.get(4)?)?,
        ))
    })
}

/// mountinfo writes a space, tab, newline and backslash as a three-digit octal escape.
fn unescape_mountinfo(s: &str) -> Option<String> {
    let mut out = Vec::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && let Some(code) = s
                .get(i + 1..i + 4)
                .and_then(|o| u8::from_str_radix(o, 8).ok())
        {
            out.push(code);
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Whether `args`, a process's command line, is an ntfs-3g mounting `source` on `target` with
/// `permissions` or `acl` among its `-o` options.
fn keeps_modes(args: &[&str], source: &str, target: &str) -> bool {
    if !(args.contains(&source) && args.contains(&target)) {
        return false;
    }
    let mut opts = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if *a == "-o" {
            opts.extend(it.next());
        } else if let Some(o) = a.strip_prefix("-o") {
            opts.push(o);
        }
    }
    opts.iter()
        .flat_map(|o| o.split(','))
        .any(|o| o == "permissions" || o == "acl")
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
mod tests;
