//! `*at()` calls relative to an open folder, for walks that must not follow a link.

use std::ffi::{CStr, CString, OsStr};
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// `renameat2(RENAME_NOREPLACE)`; without it a file is hard-linked then unlinked, and a folder
/// (or a file where links fail) is checked then renamed.
pub(crate) fn rename_free(
    from_dir: BorrowedFd,
    from: &CStr,
    to_dir: BorrowedFd,
    to: &CStr,
) -> io::Result<()> {
    let rename = |flags| {
        // SAFETY: both names are NUL-terminated and both folders are open.
        let rc = unsafe {
            libc::renameat2(
                from_dir.as_raw_fd(),
                from.as_ptr(),
                to_dir.as_raw_fd(),
                to.as_ptr(),
                flags,
            )
        };
        match rc {
            0 => Ok(()),
            _ => Err(io::Error::last_os_error()),
        }
    };
    match rename(libc::RENAME_NOREPLACE) {
        Err(e) if e.raw_os_error() == Some(libc::EINVAL) => {
            let file = stat_at(from_dir, from)?.st_mode & libc::S_IFMT != libc::S_IFDIR;
            // SAFETY: both names are NUL-terminated and both folders are open.
            let linked = file
                && unsafe {
                    libc::linkat(
                        from_dir.as_raw_fd(),
                        from.as_ptr(),
                        to_dir.as_raw_fd(),
                        to.as_ptr(),
                        0,
                    )
                } == 0;
            if linked {
                let moved = super::unlink_at(from_dir, from, 0);
                if moved.is_err() {
                    let _ = super::unlink_at(to_dir, to, 0);
                }
                return moved;
            }
            let e = io::Error::last_os_error();
            if file && e.raw_os_error() == Some(libc::EEXIST) {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            if stat_at(to_dir, to).is_ok() {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            rename(0)
        }
        r => r,
    }
}

pub(crate) fn cwd() -> BorrowedFd<'static> {
    // SAFETY: AT_FDCWD is valid for the whole process as an *at() base.
    unsafe { BorrowedFd::borrow_raw(libc::AT_FDCWD) }
}

pub(crate) fn c_path(p: &Path) -> io::Result<CString> {
    Ok(CString::new(p.as_os_str().as_bytes())?)
}

/// `name` as one path step: a NUL or a `/` in it is refused.
pub(crate) fn c_name(name: &OsStr) -> io::Result<CString> {
    if name.as_bytes().contains(&b'/') {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(CString::new(name.as_bytes())?)
}

pub(crate) fn open_at(dir: BorrowedFd, name: &CStr, flags: libc::c_int) -> io::Result<OwnedFd> {
    // SAFETY: `name` is NUL-terminated and `dir` is open (or AT_FDCWD).
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` was just opened and is owned by nothing else.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Open folder `name` in `dir`; a link as `name` itself fails with `ELOOP` unless `follow`.
pub(crate) fn open_dir(dir: BorrowedFd, name: &CStr, follow: bool) -> io::Result<OwnedFd> {
    let nofollow = if follow { 0 } else { libc::O_NOFOLLOW };
    open_at(dir, name, libc::O_RDONLY | libc::O_DIRECTORY | nofollow)
}

pub(crate) fn stat_fd(fd: BorrowedFd) -> io::Result<libc::stat> {
    // SAFETY: a zeroed `stat` is a valid out-buffer and `fd` is open.
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        match libc::fstat(fd.as_raw_fd(), &mut st) {
            0 => Ok(st),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

pub(crate) fn stat_at(dir: BorrowedFd, name: &CStr) -> io::Result<libc::stat> {
    // SAFETY: a zeroed `stat` is a valid out-buffer; `name` is NUL-terminated.
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        let flags = libc::AT_SYMLINK_NOFOLLOW;
        match libc::fstatat(dir.as_raw_fd(), name.as_ptr(), &mut st, flags) {
            0 => Ok(st),
            _ => Err(io::Error::last_os_error()),
        }
    }
}
