use super::at::{open_dir, stat_fd};
use super::{refuse_holding_mount, refuse_mount_point};
use crate::progress::Meter;
use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::path::Path;

/// Remove `path` for good: a folder with everything under it, a file or
/// a link on its own. A mount point, or a folder holding one, is refused
/// before anything goes.
pub fn erase(path: &Path) -> io::Result<()> {
    refuse_mount_point(path)?;
    if std::fs::symlink_metadata(path)?.is_dir() {
        refuse_holding_mount(path)?;
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// [`erase`] one file at a time into `meter`: each file or link removed counts as an item, a pause
/// holds before the next, and a cancel stops with `Interrupted`, keeping what is left. Folders are
/// opened without following links, so one swapped for a link on the way is never walked into.
pub(crate) fn erase_counted(path: &Path, meter: &Meter) -> io::Result<()> {
    meter.hold()?;
    erase_tree(path, Some(meter))
}

/// [`erase`], each entry a step of this thread's job ([`crate::progress::step`]) without counting
/// on its bar or taking its pause and cancel: the purge of a bin, the original of a move.
pub(crate) fn erase_stepped(path: &Path) -> io::Result<()> {
    match crate::progress::current() {
        Some(_) => erase_tree(path, None),
        None => erase(path),
    }
}

/// [`erase`] of `name` in the folder open as `dir`, counted on `meter` as [`erase_counted`] or
/// else stepped; `path` names it for the mount checks.
pub(crate) fn erase_at(
    dir: BorrowedFd,
    name: &CStr,
    path: &Path,
    meter: Option<&Meter>,
) -> io::Result<()> {
    refuse_mount_point(path)?;
    if !is_dir_at(dir, name)? {
        unlink_at(dir, name, 0)?;
        if let Some(m) = meter {
            m.removed();
        }
        return Ok(());
    }
    refuse_holding_mount(path)?;
    remove_tree_at(dir, name, meter)
}

/// Whether `name` in the folder open as `dir` is a folder, not a link to one.
fn is_dir_at(dir: BorrowedFd, name: &CStr) -> io::Result<bool> {
    // SAFETY: a zeroed `stat` is a valid out-buffer; `name` is NUL-terminated.
    let (r, mode) = unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        let r = libc::fstatat(
            dir.as_raw_fd(),
            name.as_ptr(),
            &mut st,
            libc::AT_SYMLINK_NOFOLLOW,
        );
        (r, st.st_mode)
    };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(mode & libc::S_IFMT == libc::S_IFDIR)
}

/// `unlinkat(dir, name, flags)`.
pub(crate) fn unlink_at(dir: BorrowedFd, name: &CStr, flags: libc::c_int) -> io::Result<()> {
    // SAFETY: `name` is NUL-terminated and outlives the call; `dir` is an open folder.
    if unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), flags) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Remove `path` one entry at a time, each file counted on `meter` (an item, a pause and cancel
/// between two) or, without one, only stepping this thread's job.
fn erase_tree(path: &Path, meter: Option<&Meter>) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    refuse_mount_point(path)?;
    if !std::fs::symlink_metadata(path)?.is_dir() {
        std::fs::remove_file(path)?;
        if let Some(m) = meter {
            m.removed();
        }
        return Ok(());
    }
    refuse_holding_mount(path)?;
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no folder to erase",
        ));
    };
    let parent = std::fs::File::open(if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    })?;
    let name = std::ffi::CString::new(name.as_bytes())?;
    remove_tree_at(parent.as_fd(), &name, meter)
}

/// The folders [`remove_tree_at`] keeps open on its way down; one above them is closed and
/// opened again by name when the walk climbs back to it.
const OPEN_LEVELS: usize = 64;

/// One folder on [`remove_tree_at`]'s way down.
struct Level {
    name: CString,
    fd: Option<OwnedFd>,
    ino: u64,
    left: Vec<(CString, bool)>,
}

/// Remove folder `name` in the folder open at `parent` with everything under it, children first,
/// without following a link or entering another file system. Iterative, so depth costs no stack.
fn remove_tree_at(parent: BorrowedFd, name: &CStr, meter: Option<&Meter>) -> io::Result<()> {
    let dev = stat_fd(parent)?.st_dev;
    let mut stack = vec![level(parent, name, dev)?];
    while !stack.is_empty() {
        let top = stack.len() - 1;
        match stack[top].left.pop() {
            Some((child, is_dir)) => {
                match meter {
                    Some(m) => m.hold()?,
                    None => crate::progress::step(),
                }
                let at = reopen(parent, &mut stack, dev)?;
                if is_dir {
                    let next = level(at, &child, dev)?;
                    stack.push(next);
                    if let Some(far) = stack.len().checked_sub(OPEN_LEVELS + 1) {
                        stack[far].fd = None;
                    }
                } else {
                    unlink_at(at, &child, 0)?;
                    if let Some(m) = meter {
                        m.removed();
                    }
                }
            }
            None => {
                let done = stack.pop().expect("a level is left");
                drop(done.fd);
                let up = match stack.is_empty() {
                    true => parent,
                    false => reopen(parent, &mut stack, dev)?,
                };
                unlink_at(up, &done.name, libc::AT_REMOVEDIR)?;
            }
        }
    }
    Ok(())
}

/// Open folder `name` in `at` and list it; another device than `dev` is refused.
fn level(at: BorrowedFd, name: &CStr, dev: libc::dev_t) -> io::Result<Level> {
    let fd = open_dir(at, name, false)?;
    let st = stat_fd(fd.as_fd())?;
    if st.st_dev != dev {
        return Err(io::Error::new(
            io::ErrorKind::ResourceBusy,
            "a folder in it is a mounted drive",
        ));
    }
    Ok(Level {
        name: name.to_owned(),
        left: entries_at(fd.as_fd())?,
        ino: st.st_ino,
        fd: Some(fd),
    })
}

/// The open folder of `stack`'s last level, opening again by name the levels [`remove_tree_at`]
/// closed; one that is no longer the folder it listed fails.
fn reopen<'a>(
    parent: BorrowedFd<'a>,
    stack: &'a mut [Level],
    dev: libc::dev_t,
) -> io::Result<BorrowedFd<'a>> {
    let top = stack.len() - 1;
    let first = stack
        .iter()
        .rposition(|l| l.fd.is_some())
        .map_or(0, |i| i + 1);
    for i in first..=top {
        let fd = {
            let at = match i {
                0 => parent,
                _ => stack[i - 1].fd.as_ref().expect("opened above").as_fd(),
            };
            open_dir(at, &stack[i].name, false)?
        };
        let st = stat_fd(fd.as_fd())?;
        if st.st_dev != dev || st.st_ino != stack[i].ino {
            return Err(io::Error::other(
                "a folder in it was replaced while it was removed",
            ));
        }
        stack[i].fd = Some(fd);
        if let Some(far) = i.checked_sub(1).filter(|&j| top - j >= OPEN_LEVELS) {
            stack[far].fd = None;
        }
    }
    Ok(stack[top].fd.as_ref().expect("opened above").as_fd())
}

/// The names in the folder open as `dir`, each with whether it is a folder (not a link to one).
pub(crate) fn entries_at(dir: BorrowedFd) -> io::Result<Vec<(std::ffi::CString, bool)>> {
    // SAFETY: `dup` of an open descriptor; `fdopendir` takes the copy and `closedir` closes it.
    let copy = unsafe { libc::dup(dir.as_raw_fd()) };
    if copy < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `copy` is an open folder descriptor that only this stream owns from here.
    let stream = unsafe { libc::fdopendir(copy) };
    if stream.is_null() {
        let e = io::Error::last_os_error();
        // SAFETY: `fdopendir` failed, so `copy` is still ours to close.
        unsafe { libc::close(copy) };
        return Err(e);
    }
    // The copy shares `dir`'s offset, which an earlier listing left at the end.
    // SAFETY: `stream` is open.
    unsafe { libc::rewinddir(stream) };
    let mut out = Vec::new();
    loop {
        // SAFETY: `stream` is open until the `closedir` below.
        let ent = unsafe { libc::readdir(stream) };
        if ent.is_null() {
            break;
        }
        // SAFETY: `readdir` returned a valid entry whose `d_name` is NUL-terminated.
        let (name, kind) = unsafe {
            (
                std::ffi::CStr::from_ptr((*ent).d_name.as_ptr()),
                (*ent).d_type,
            )
        };
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            continue;
        }
        let is_dir = match kind {
            libc::DT_DIR => true,
            libc::DT_UNKNOWN => {
                // SAFETY: a zeroed `stat` is a valid out-buffer; `name` is NUL-terminated.
                let (r, mode) = unsafe {
                    let mut st: libc::stat = std::mem::zeroed();
                    let r = libc::fstatat(
                        dir.as_raw_fd(),
                        name.as_ptr(),
                        &mut st,
                        libc::AT_SYMLINK_NOFOLLOW,
                    );
                    (r, st.st_mode)
                };
                r == 0 && mode & libc::S_IFMT == libc::S_IFDIR
            }
            _ => false,
        };
        out.push((name.to_owned(), is_dir));
    }
    // SAFETY: `stream` came from `fdopendir` and is closed once.
    unsafe { libc::closedir(stream) };
    Ok(out)
}
