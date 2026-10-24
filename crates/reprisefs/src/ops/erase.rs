use super::{refuse_holding_mount, refuse_mount_point};
use std::io;
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
pub(crate) fn erase_counted(path: &Path, meter: &crate::progress::Meter) -> io::Result<()> {
    meter.hold()?;
    erase_tree(path, meter, true)
}

/// [`erase`], each entry a step of this thread's job ([`crate::progress::step`]) without counting
/// on its bar or taking its pause and cancel: the purge of a bin, the original of a move.
pub(crate) fn erase_stepped(path: &Path) -> io::Result<()> {
    match crate::progress::current() {
        Some(meter) => erase_tree(path, &meter, false),
        None => erase(path),
    }
}

/// Remove `path` one entry at a time, `counted` on `meter` (each file an item, a pause and cancel
/// between two) or only stepping it.
fn erase_tree(path: &Path, meter: &crate::progress::Meter, counted: bool) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    refuse_mount_point(path)?;
    if !std::fs::symlink_metadata(path)?.is_dir() {
        std::fs::remove_file(path)?;
        if counted {
            meter.removed();
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
    remove_tree_at(
        std::os::fd::AsRawFd::as_raw_fd(&parent),
        &name,
        meter,
        counted,
    )
}

/// Remove folder `name` in the folder open at `parent` with everything under it, children first.
fn remove_tree_at(
    parent: std::os::fd::RawFd,
    name: &std::ffi::CStr,
    meter: &crate::progress::Meter,
    counted: bool,
) -> io::Result<()> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: `name` is NUL-terminated and outlives the call; `parent` is an open folder.
    let fd = unsafe { libc::openat(parent, name.as_ptr(), flags) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` was just opened and is owned by nothing else.
    let dir = unsafe { OwnedFd::from_raw_fd(fd) };
    for (child, is_dir) in entries_at(&dir)? {
        if counted {
            meter.hold()?;
        } else {
            crate::progress::step();
        }
        if is_dir {
            remove_tree_at(dir.as_raw_fd(), &child, meter, counted)?;
        } else {
            // SAFETY: as for `openat` above.
            if unsafe { libc::unlinkat(dir.as_raw_fd(), child.as_ptr(), 0) } != 0 {
                return Err(io::Error::last_os_error());
            }
            if counted {
                meter.removed();
            }
        }
    }
    // SAFETY: as for `openat` above.
    if unsafe { libc::unlinkat(parent, name.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The names in the folder open as `dir`, each with whether it is a folder (not a link to one).
fn entries_at(dir: &std::os::fd::OwnedFd) -> io::Result<Vec<(std::ffi::CString, bool)>> {
    use std::os::fd::AsRawFd;
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
