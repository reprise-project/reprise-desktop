use super::at::{open_dir, stat_fd};
use super::copy::in_tree;
use super::{refuse_holding_mount, refuse_mount_point};
use crate::progress::Meter;
use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};

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
    erase_entry_at(dir, name, path, meter, None)
}

/// What [`erase_all_at`] could not remove: how many entries, and the first one's error.
#[derive(Debug, Default)]
pub(crate) struct Failures {
    pub count: usize,
    pub first: Option<io::Error>,
}

impl Failures {
    pub(crate) fn add(&mut self, e: io::Error) {
        self.count += 1;
        self.first.get_or_insert(e);
    }
}

/// [`erase_at`] that goes on past an entry it cannot remove at any depth, counting it in
/// `failed`: a folder it cannot write keeps its entries, and an entry that stays keeps the
/// folders above it. An entry already gone is no failure. The error is `name` itself failing, or
/// what stops the walk part way, as a folder replaced on the way.
pub(crate) fn erase_all_at(
    dir: BorrowedFd,
    name: &CStr,
    path: &Path,
    failed: &mut Failures,
) -> io::Result<()> {
    erase_entry_at(dir, name, path, None, Some(failed))
}

fn erase_entry_at(
    dir: BorrowedFd,
    name: &CStr,
    path: &Path,
    meter: Option<&Meter>,
    failed: Option<&mut Failures>,
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
    remove_tree_at(dir, name, meter, failed)
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
    remove_tree_at(parent.as_fd(), &name, meter, None)
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
    /// Something under it stayed, so it stays too.
    kept: bool,
}

/// Remove folder `name` in the folder open at `parent` with everything under it, children first,
/// without following a link or entering another file system. Iterative, so depth costs no stack.
/// An entry that fails is named by its path under `name` ([`in_tree`]) and stops the walk, or
/// with `failed` joins it ([`settle`]).
pub(super) fn remove_tree_at(
    parent: BorrowedFd,
    name: &CStr,
    meter: Option<&Meter>,
    mut failed: Option<&mut Failures>,
) -> io::Result<()> {
    let keep_going = failed.is_some();
    let dev = Mount::of(parent)?;
    let mut stack = vec![level(parent, name, dev)?];
    while !stack.is_empty() {
        let top = stack.len() - 1;
        match stack[top].left.pop() {
            Some((child, is_dir)) => {
                match meter {
                    Some(m) => m.hold()?,
                    None => crate::progress::step(),
                }
                let at = reopen(parent, &mut stack, dev).map_err(|e| stopped(e, keep_going))?;
                let stays = if is_dir {
                    match level(at, &child, dev) {
                        Ok(next) => {
                            stack.push(next);
                            if let Some(far) = stack.len().checked_sub(OPEN_LEVELS + 1) {
                                stack[far].fd = None;
                            }
                            false
                        }
                        Err(e) => settle(&mut failed, in_tree(&under(&stack, &child), e))?,
                    }
                } else {
                    match unlink_at(at, &child, 0) {
                        Ok(()) => {
                            if let Some(m) = meter {
                                m.removed();
                            }
                            false
                        }
                        Err(e) => settle(&mut failed, in_tree(&under(&stack, &child), e))?,
                    }
                };
                stack[top].kept |= stays;
            }
            None => {
                let done = stack.pop().expect("a level is left");
                drop(done.fd);
                let up = match stack.is_empty() {
                    true => parent,
                    false => reopen(parent, &mut stack, dev).map_err(|e| stopped(e, keep_going))?,
                };
                let stays = done.kept
                    || match unlink_at(up, &done.name, libc::AT_REMOVEDIR) {
                        Ok(()) => false,
                        Err(e) => settle(&mut failed, in_tree(&under(&stack, &done.name), e))?,
                    };
                if let Some(above) = stack.last_mut() {
                    above.kept |= stays;
                }
            }
        }
    }
    Ok(())
}

/// An entry [`remove_tree_at`] could not remove: without `failed` the walk stops with `e`; with
/// it `e` is counted in `failed` and the entry stays (true), unless it is already gone.
fn settle(failed: &mut Option<&mut Failures>, e: io::Error) -> io::Result<bool> {
    let Some(failures) = failed else {
        return Err(e);
    };
    if e.kind() == io::ErrorKind::NotFound {
        return Ok(false);
    }
    failures.add(e);
    Ok(true)
}

/// The error of a walk that stops part way. One that goes on past failures never stops as
/// NotFound, which its caller reads as the entry already gone.
fn stopped(e: io::Error, keep_going: bool) -> io::Error {
    match keep_going && e.kind() == io::ErrorKind::NotFound {
        true => io::Error::other(e),
        false => e,
    }
}

/// The path of `name`, in the last of `levels`, under the first; empty with no levels, `name`
/// being the first itself.
fn under(levels: &[Level], name: &CStr) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    let Some((_, below)) = levels.split_first() else {
        return PathBuf::new();
    };
    let part = |n: &CStr| Path::new(std::ffi::OsStr::from_bytes(n.to_bytes())).to_path_buf();
    below
        .iter()
        .map(|l| part(&l.name))
        .chain([part(name)])
        .collect()
}

/// The mount a folder lies on: its device, and the mount's id where statx gives one, which also
/// tells apart a bind mount of the same file system.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Mount {
    dev: libc::dev_t,
    id: Option<u64>,
}

impl Mount {
    fn of(fd: BorrowedFd) -> io::Result<Mount> {
        Ok(Mount::with(fd, &stat_fd(fd)?))
    }

    /// The mount of `fd`, whose `stat` is `st`.
    fn with(fd: BorrowedFd, st: &libc::stat) -> Mount {
        let mut stx = std::mem::MaybeUninit::<libc::statx>::zeroed();
        // SAFETY: the empty path is NUL-terminated, `fd` is open and `stx` is a writable statx
        // buffer.
        let r = unsafe {
            libc::statx(
                fd.as_raw_fd(),
                c"".as_ptr(),
                libc::AT_EMPTY_PATH,
                libc::STATX_MNT_ID,
                stx.as_mut_ptr(),
            )
        };
        // SAFETY: statx filled `stx` on success, and the zeroed buffer is a valid value otherwise.
        let stx = unsafe { stx.assume_init() };
        let id = (r == 0 && stx.stx_mask & libc::STATX_MNT_ID != 0).then_some(stx.stx_mnt_id);
        Mount { dev: st.st_dev, id }
    }
}

/// Open folder `name` in `at` and list it; one on another mount than `dev` is refused.
fn level(at: BorrowedFd, name: &CStr, dev: Mount) -> io::Result<Level> {
    let fd = open_dir(at, name, false)?;
    let st = stat_fd(fd.as_fd())?;
    if Mount::with(fd.as_fd(), &st) != dev {
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
        kept: false,
    })
}

/// The open folder of `stack`'s last level, opening again by name the levels [`remove_tree_at`]
/// closed; one that is no longer the folder it listed fails.
fn reopen<'a>(
    parent: BorrowedFd<'a>,
    stack: &'a mut [Level],
    dev: Mount,
) -> io::Result<BorrowedFd<'a>> {
    let top = stack.len() - 1;
    let first = stack
        .iter()
        .rposition(|l| l.fd.is_some())
        .map_or(0, |i| i + 1);
    for i in first..=top {
        let named = |e| in_tree(&under(&stack[..i], &stack[i].name), e);
        let at = match i {
            0 => parent,
            _ => stack[i - 1].fd.as_ref().expect("opened above").as_fd(),
        };
        let fd = open_dir(at, &stack[i].name, false).map_err(named)?;
        let st = stat_fd(fd.as_fd()).map_err(named)?;
        if Mount::with(fd.as_fd(), &st) != dev || st.st_ino != stack[i].ino {
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
