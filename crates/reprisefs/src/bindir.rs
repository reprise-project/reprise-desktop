//! A trash's folders reached by descriptor, no link followed: every listing, erase and purge in
//! a Recycle Bin goes through [`BinDir`], a drive item's Restore through [`restore_beneath`].

use crate::ops::at::{c_name, c_path, cwd, open_at, open_dir, rename_free, stat_at, stat_fd};
use crate::progress::Meter;
use std::ffi::{CStr, OsStr, OsString};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStringExt;
use std::path::{Component, Path, PathBuf};

/// The most of a `.trashinfo` read: a record is a path and a date.
const INFO_MAX: u64 = 64 * 1024;

/// One of a trash's `files/` and `info/`.
enum Sub {
    Missing,
    Dir(OwnedFd),
    /// A link, a file, or on a drive a folder of another user's: never entered.
    Stray,
}

/// A trash opened from its drive's top, or the home trash by its path. A drive trash opens only
/// as a real `.Trash-$uid` of this user's or `$uid` in a real sticky `.Trash`.
pub(crate) struct BinDir {
    root: OwnedFd,
    files: Sub,
    info: Sub,
}

impl BinDir {
    /// Fails with `NotFound` when there is no trash at `root`.
    pub(crate) fn open(root: &Path) -> io::Result<BinDir> {
        // SAFETY: getuid has no failure mode.
        let uid = unsafe { libc::getuid() };
        let drive = crate::trash::topdir_of(root);
        let root_fd = match drive {
            Some(top) => open_drive_root(top, root, uid)?,
            None => open_dir(cwd(), &c_path(root)?, true)?,
        };
        let sub = |name: &CStr| match open_dir(root_fd.as_fd(), name, false) {
            Ok(fd) if drive.is_none() || stat_fd(fd.as_fd())?.st_uid == uid => Ok(Sub::Dir(fd)),
            Ok(_) => Ok(Sub::Stray),
            Err(e) if e.raw_os_error() == Some(libc::ENOENT) => Ok(Sub::Missing),
            Err(e) if matches!(e.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR)) => {
                Ok(Sub::Stray)
            }
            Err(e) => Err(e),
        };
        Ok(BinDir {
            files: sub(c"files")?,
            info: sub(c"info")?,
            root: root_fd,
        })
    }

    pub(crate) fn files(&self) -> Vec<OsString> {
        names(&self.files)
    }

    pub(crate) fn infos(&self) -> Vec<OsString> {
        names(&self.info)
    }

    /// `files/NAME`'s `lstat`.
    pub(crate) fn stat(&self, name: &OsStr) -> Option<libc::stat> {
        let Sub::Dir(fd) = &self.files else {
            return None;
        };
        stat_at(fd.as_fd(), &c_name(name).ok()?).ok()
    }

    /// Erase `files/NAME` as [`crate::ops::erase_at`] does; `path` names it for the mount checks.
    pub(crate) fn erase(&self, name: &OsStr, path: &Path, meter: Option<&Meter>) -> io::Result<()> {
        let Sub::Dir(fd) = &self.files else {
            return Err(io::ErrorKind::NotFound.into());
        };
        crate::ops::erase_at(fd.as_fd(), &c_name(name)?, path, meter)
    }

    /// Remove `info/NAME`, a file or a link, never a folder.
    pub(crate) fn unlink_info(&self, name: &OsStr) -> io::Result<()> {
        let Sub::Dir(fd) = &self.info else {
            return Err(io::ErrorKind::NotFound.into());
        };
        crate::ops::unlink_at(fd.as_fd(), &c_name(name)?, 0)
    }

    /// `info/NAME`'s text, a regular file only.
    pub(crate) fn read_info(&self, name: &OsStr) -> io::Result<String> {
        let Sub::Dir(fd) = &self.info else {
            return Err(io::ErrorKind::NotFound.into());
        };
        let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY;
        let file = std::fs::File::from(open_at(fd.as_fd(), &c_name(name)?, flags)?);
        let bytes = crate::ops::read_opened(&file, INFO_MAX)?;
        String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Unlink a `files` or `info` that is a link or a file, and the `directorysizes` cache. A
    /// stray folder of another user's stays.
    pub(crate) fn clear_strays(&self) -> io::Result<()> {
        for (name, sub) in [(c"files", &self.files), (c"info", &self.info)] {
            if matches!(sub, Sub::Stray) {
                match crate::ops::unlink_at(self.root.as_fd(), name, 0) {
                    Err(e) if e.raw_os_error() == Some(libc::EISDIR) => {
                        log::warn!("trash: left a {name:?} folder that is not this user's");
                    }
                    r => r?,
                }
            }
        }
        match crate::ops::unlink_at(self.root.as_fd(), c"directorysizes", 0) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }

    fn files_fd(&self) -> io::Result<BorrowedFd<'_>> {
        match &self.files {
            Sub::Dir(fd) => Ok(fd.as_fd()),
            _ => Err(io::ErrorKind::NotFound.into()),
        }
    }
}

/// Move drive trash item `files` back to `dir/name` under `top` through folders made when gone and
/// never links, numbered beside a taken name; `InvalidData` when `dir` is not under `top`.
pub(crate) fn restore_beneath(
    bin: &BinDir,
    files: &Path,
    top: &Path,
    dir: &Path,
    name: &OsStr,
) -> io::Result<PathBuf> {
    let rel = beneath(top, dir).ok_or_else(outside)?;
    let from = c_name(files.file_name().ok_or_else(outside)?)?;
    let mut at = open_dir(cwd(), &c_path(top)?, true)?;
    for part in rel.components() {
        let part = c_name(part.as_os_str())?;
        // SAFETY: `part` is NUL-terminated; mkdirat takes the umask like any new folder.
        if unsafe { libc::mkdirat(at.as_raw_fd(), part.as_ptr(), 0o777) } != 0 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() != Some(libc::EEXIST) {
                return Err(e);
            }
        }
        at = open_dir(at.as_fd(), &part, false).map_err(|e| match e.raw_os_error() {
            Some(libc::ELOOP | libc::ENOTDIR) => io::Error::new(
                io::ErrorKind::InvalidData,
                "its original folder is a link or a file now",
            ),
            _ => e,
        })?;
    }
    let src = bin.files_fd()?;
    for candidate in crate::names::numbered(name, crate::names::paren) {
        let to = c_name(&candidate)?;
        match rename_free(src, &from, at.as_fd(), &to) {
            Ok(()) => return Ok(top.join(rel).join(candidate)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
                return Err(io::Error::new(
                    io::ErrorKind::CrossesDevices,
                    "its original folder is on another drive",
                ));
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!("numbered never ends")
}

/// `path` relative to `top` when it lies under it, named by plain steps only (no `.`, `..` or
/// root).
pub(crate) fn beneath(top: &Path, path: &Path) -> Option<PathBuf> {
    let rel = path.strip_prefix(top).ok()?;
    rel.components()
        .all(|c| matches!(c, Component::Normal(_)))
        .then(|| rel.to_path_buf())
}

pub(crate) fn outside() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "the Recycle Bin's record puts it outside its drive",
    )
}

fn open_drive_root(top: &Path, root: &Path, uid: u32) -> io::Result<OwnedFd> {
    let refused = |why: &str| io::Error::new(io::ErrorKind::PermissionDenied, why.to_string());
    let top_fd = open_dir(cwd(), &c_path(top)?, true)?;
    let name = c_name(root.file_name().unwrap_or_default())?;
    let own = if root.parent() == Some(top) {
        open_dir(top_fd.as_fd(), &name, false)
    } else {
        let shared = open_dir(top_fd.as_fd(), c".Trash", false).map_err(not_a_bin)?;
        if stat_fd(shared.as_fd())?.st_mode & libc::S_ISVTX == 0 {
            return Err(refused("the drive's shared Recycle Bin is not sticky"));
        }
        open_dir(shared.as_fd(), &name, false)
    }
    .map_err(not_a_bin)?;
    if stat_fd(own.as_fd())?.st_uid != uid {
        return Err(refused("the drive's Recycle Bin is not yours"));
    }
    Ok(own)
}

fn not_a_bin(e: io::Error) -> io::Error {
    match e.raw_os_error() {
        Some(libc::ELOOP | libc::ENOTDIR) => io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the drive's Recycle Bin is a link or a file",
        ),
        _ => e,
    }
}

fn names(sub: &Sub) -> Vec<OsString> {
    let Sub::Dir(fd) = sub else {
        return Vec::new();
    };
    crate::ops::entries_at(fd.as_fd())
        .unwrap_or_default()
        .into_iter()
        .map(|(n, _)| OsString::from_vec(n.into_bytes()))
        .collect()
}

#[cfg(test)]
mod tests;
