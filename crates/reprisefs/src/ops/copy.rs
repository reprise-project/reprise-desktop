use super::MAX_DEPTH;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Copy a folder and everything under it to `dest`, which must not exist yet
/// (`AlreadyExists`, left untouched). A symlink is copied as a symlink, as
/// `cp -a` does: following one would lose the shortcut and let a link pointing
/// back up the tree recurse. Entries are listed before `dest` exists, so a `dest`
/// inside `src` is never walked. A copy that fails part way removes what it made
/// of `dest`. Entries that are neither files, folders nor links (a FIFO, a
/// socket) are skipped: `fs::copy` would block opening them.
pub fn copy_dir_all(src: &Path, dest: &Path) -> io::Result<()> {
    copy_dir_until(src, dest, &AtomicBool::new(false))
}

/// [`copy_dir_all`], checking `stop` before each entry. Once it is set
/// the copy fails with `Interrupted` and removes what it made of `dest`.
pub(super) fn copy_dir_until(src: &Path, dest: &Path, stop: &AtomicBool) -> io::Result<()> {
    let entries = read_then_create(src, dest)?;
    // `dest` is ours from here on, so a failure may remove it. Folders keep the mode they were
    // made with until the whole tree has landed, so a read-only one never blocks the cleanup.
    let mut folders = Vec::new();
    copy_entries(entries, dest, 0, stop, &mut folders)
        .and_then(|()| {
            folders.push((src.to_path_buf(), dest.to_path_buf()));
            (folders.iter()).try_for_each(|(from, to)| copy_dir_meta(from, to))
        })
        .inspect_err(|_| {
            let _ = remove_tree(dest);
        })
}

/// `remove_dir_all`, making each folder writable before emptying it.
fn remove_tree(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_dir() {
        return std::fs::remove_file(path);
    }
    let mode = meta.permissions().mode();
    if mode & 0o300 != 0o300 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode | 0o700))?;
    }
    for entry in std::fs::read_dir(path)? {
        remove_tree(&entry?.path())?;
    }
    std::fs::remove_dir(path)
}

/// Give the folder `to` the mode, xattrs and times of `from`, once its
/// entries have landed: each one landing moves the folder's mtime.
fn copy_dir_meta(from: &Path, to: &Path) -> io::Result<()> {
    let meta = std::fs::metadata(from)?;
    super::fileio::copy_meta(from, &meta, &std::fs::File::open(to)?, true)
}

/// The entries of `src`, read before `dest` is created.
fn read_then_create(src: &Path, dest: &Path) -> io::Result<Vec<std::fs::DirEntry>> {
    let entries = std::fs::read_dir(src)?.collect::<io::Result<_>>()?;
    std::fs::create_dir(dest)?;
    Ok(entries)
}

/// Copy `entries` into `dest`, which sits `depth` folders below the top. Each folder made is
/// pushed on `folders` after its own entries, for its metadata to follow once all have landed.
fn copy_entries(
    entries: Vec<std::fs::DirEntry>,
    dest: &Path,
    depth: usize,
    stop: &AtomicBool,
    folders: &mut Vec<(std::path::PathBuf, std::path::PathBuf)>,
) -> io::Result<()> {
    for entry in entries {
        if stop.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "The copy was cancelled.",
            ));
        }
        crate::progress::step();
        let to = dest.join(entry.file_name());
        let from = entry.path();
        // `symlink_metadata`, not `is_dir`: the latter follows the link.
        let kind = std::fs::symlink_metadata(&from)?.file_type();
        if kind.is_symlink() {
            std::os::unix::fs::symlink(std::fs::read_link(&from)?, &to)?;
        } else if kind.is_dir() {
            if depth + 1 >= MAX_DEPTH {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("The folder is more than {MAX_DEPTH} levels deep."),
                ));
            }
            copy_entries(read_then_create(&from, &to)?, &to, depth + 1, stop, folders)?;
            folders.push((from, to));
        } else if kind.is_file() {
            copy_new(&from, &to)?;
        }
    }
    Ok(())
}

/// `fs::copy`, except that an existing `to` is an `AlreadyExists` error
/// instead of being overwritten: the name was picked as free a moment
/// ago and nothing else may have taken it since. Keeps `from`'s mode,
/// xattrs (the launcher trust marker re-made for the copy) and times. A copy that fails after `to` was
/// created removes it, so no truncated file is left behind.
///
/// `from` must be a regular file (a link to one is followed): anything
/// else is an `InvalidInput` error before `to` is made. The open is
/// non-blocking, so a FIFO is refused instead of waiting for a writer.
pub(crate) fn copy_new(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut src = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(from)?;
    if !src.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Only files and folders can be copied.",
        ));
    }
    let mut dst = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)?;
    // `create_new` made `to`, so it is ours to remove on failure.
    (|| {
        match crate::progress::current() {
            Some(meter) => {
                meter.set_current(from.file_name().unwrap_or_default());
                copy_metered(&mut src, &dst, &meter)?
            }
            None => {
                io::copy(&mut src, &mut dst)?;
            }
        }
        super::fileio::copy_meta(from, &src.metadata()?, &dst, false)
    })()
    .inspect_err(|_| {
        let _ = std::fs::remove_file(to);
    })
}

/// [`copy_new`] under a put's meter: 1 MiB chunks through `copy_file_range` (else read and
/// write), each counted, paused and cancellable, a large file flushed as it goes.
fn copy_metered(
    src: &mut std::fs::File,
    dst: &std::fs::File,
    meter: &crate::progress::Meter,
) -> io::Result<()> {
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    const CHUNK: usize = 1 << 20;
    let mut buf = Vec::new();
    let mut back = crate::progress::Writeback::new();
    let mut len = 0u64;
    let mut out = dst;
    let mut ranged = true;
    loop {
        let n = if ranged {
            // SAFETY: both fds are open; null offsets use and advance each file's own position.
            let n = unsafe {
                libc::copy_file_range(
                    src.as_raw_fd(),
                    std::ptr::null_mut(),
                    dst.as_raw_fd(),
                    std::ptr::null_mut(),
                    CHUNK,
                    0,
                )
            };
            if n >= 0 {
                n as usize
            } else {
                let e = io::Error::last_os_error();
                match e.raw_os_error() {
                    Some(libc::EINTR) => continue,
                    Some(libc::EXDEV | libc::ENOSYS | libc::EOPNOTSUPP | libc::EINVAL)
                        if len == 0 =>
                    {
                        ranged = false;
                        continue;
                    }
                    _ => return Err(e),
                }
            }
        } else {
            if buf.is_empty() {
                let size = src.metadata().map_or(0, |m| m.len());
                buf = vec![0; size.clamp(1 << 16, CHUNK as u64) as usize];
            }
            match src.read(&mut buf) {
                Ok(n) => {
                    out.write_all(&buf[..n])?;
                    n
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        };
        if n == 0 {
            break;
        }
        len += n as u64;
        back.advance(dst, len, meter);
        meter.wrote(n as u64)?;
    }
    meter.wrote_file(dst, len, back.used());
    Ok(())
}
