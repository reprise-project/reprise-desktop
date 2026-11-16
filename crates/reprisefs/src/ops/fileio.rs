use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The regular file at `path` (links followed), at most `max` bytes, else `FileTooLarge`; a FIFO
/// or device is never opened (`InvalidInput`). Small files from a user's folders read here.
pub fn read_regular(path: &Path, max: u64) -> io::Result<Vec<u8>> {
    read_opened(&open_regular(path)?, max)
}

/// The regular file at `path` (links followed) opened to read; a FIFO or device is never opened
/// (`InvalidInput`), so the open cannot wait for a writer.
pub fn open_regular(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let held = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH)
        .open(path)?;
    reopen_held(held.into())
}

/// [`read_regular`] for a file held `O_PATH`.
pub(crate) fn read_held(held: OwnedFd, max: u64) -> io::Result<Vec<u8>> {
    read_opened(&reopen_held(held)?, max)
}

/// A file held `O_PATH`, reopened to read through `/proc/self/fd` only once it is known to be a
/// regular file.
fn reopen_held(held: OwnedFd) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let held = std::fs::File::from(held);
    if !held.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(format!("/proc/self/fd/{}", held.as_raw_fd()))
}

/// [`read_regular`] for a file opened elsewhere (by a directory fd, or
/// with `O_NOFOLLOW`), which the opener gave `O_NONBLOCK | O_NOCTTY`.
pub(crate) fn read_opened(file: &std::fs::File, max: u64) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let too_large = || io::Error::new(io::ErrorKind::FileTooLarge, format!("over {max} bytes"));
    if meta.len() > max {
        return Err(too_large());
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    // One byte past `max` tells a file that grew since the stat.
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(too_large());
    }
    Ok(bytes)
}

/// Create the temp file for a write-then-rename save of `path`, beside it
/// as `.{name}.{random}.tmp`. `O_EXCL` means a file or symlink someone
/// planted at that name is never opened or followed; a clash retries under
/// a fresh name. `mode` is the replaced file's, set past the umask; `None`
/// creates it 0666 under the umask like any new file.
pub fn create_temp_beside(
    path: &Path,
    mode: Option<std::fs::Permissions>,
) -> io::Result<(std::fs::File, PathBuf)> {
    use std::hash::{BuildHasher, Hasher};
    use std::os::unix::fs::OpenOptionsExt;
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let mut tries = 0;
    loop {
        // RandomState keys are OS-random per thread and step on every call.
        let tag = std::hash::RandomState::new().build_hasher().finish();
        let tmp = dir.join(format!(".{name}.{tag:016x}.tmp"));
        let opened = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(if mode.is_some() { 0o600 } else { 0o666 })
            .open(&tmp);
        match opened {
            Ok(f) => {
                if let Some(m) = mode
                    && let Err(e) = f.set_permissions(m)
                {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(e);
                }
                return Ok((f, tmp));
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && tries < 16 => tries += 1,
            Err(e) => return Err(e),
        }
    }
}

/// Write `bytes` to `path` through a temp file beside it and a rename
/// over the target, so neither a reader nor a crash sees half a file.
/// A symlinked target (a dotfiles checkout) is followed and the real file
/// replaced, so the link survives; the new file keeps the old one's mode,
/// and is synced before the rename so a crash cannot leave it empty. The
/// directory is synced after it, so the rename itself survives a crash.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    replace(path, bytes, false)
}

/// Read `path` (absent is empty, and with `reset` so is one not UTF-8), hand its text to `edit` and
/// [`write_atomic`] a changed result, under an exclusive `flock` on a `.lock` file beside it (the
/// rename replaces the target's inode). `edit`'s error comes back as `Ok(Err)`, nothing written.
pub fn locked_edit<E>(
    path: &Path,
    reset: bool,
    edit: impl FnOnce(&str) -> Result<String, E>,
) -> io::Result<Result<String, E>> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    std::fs::create_dir_all(&dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "out".into());
    let lock = std::fs::File::create(dir.join(format!(".{name}.lock")))?;
    // SAFETY: a valid fd from the open file above; LOCK_EX blocks until
    // the other editor is done and the lock goes with the fd on drop.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut stale = false;
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) if reset && e.kind() == io::ErrorKind::InvalidData => {
            stale = true;
            String::new()
        }
        Err(e) => return Err(e),
    };
    let out = match edit(&text) {
        Ok(out) => out,
        Err(e) => return Ok(Err(e)),
    };
    if out != text || stale {
        write_atomic(path, out.as_bytes())?;
    }
    Ok(Ok(out))
}

/// [`write_atomic`] for a file only its owner may read: the new file is 0600 whatever the old
/// one's mode was.
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    replace(path, bytes, true)
}

fn replace(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let path = std::fs::canonicalize(path).or_else(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Ok(path.to_path_buf())
        } else {
            Err(e)
        }
    })?;
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(dir)?;
    let mode = match private {
        true => Some(std::fs::Permissions::from_mode(0o600)),
        false => std::fs::metadata(&path).ok().map(|m| m.permissions()),
    };
    let (mut f, tmp) = create_temp_beside(&path, mode)?;
    let written = (|| {
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return written;
    }
    std::fs::File::open(dir)?.sync_all()
}

/// Write `bytes` over a file a user edits so a crash or full disk never leaves it truncated:
/// temp file, sync, rename, sync the directory. Keeps mode, xattrs and (where permitted) owner
/// and group. A symlink's target is replaced; hard-linked files and dangling symlinks are written
/// in place ([`write_in_place`]), since a rename would cut the link.
pub fn write_document(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;
    let resolved;
    let mut path = path;
    let mut meta = std::fs::symlink_metadata(path).ok();
    if meta.as_ref().is_some_and(|m| m.file_type().is_symlink())
        && let Ok(real) = std::fs::canonicalize(path)
    {
        resolved = real;
        path = &resolved;
        meta = std::fs::symlink_metadata(path).ok();
    }
    if meta
        .as_ref()
        .is_some_and(|m| m.file_type().is_symlink() || m.nlink() > 1)
    {
        return write_in_place(path, |mut f| f.write_all(bytes));
    }
    if meta.as_ref().is_some_and(|m| m.is_file()) {
        std::fs::OpenOptions::new().write(true).open(path)?;
    }
    let (f, tmp) = create_temp_beside(path, meta.as_ref().map(|m| m.permissions()))?;
    let written = (|| {
        if let Some(m) = &meta {
            // Another user's file keeps its owner only under root; its group
            // stays when the process is a member of it.
            if std::os::unix::fs::fchown(&f, Some(m.uid()), Some(m.gid())).is_err() {
                let _ = std::os::unix::fs::fchown(&f, None, Some(m.gid()));
            }
        }
        if meta.is_some() {
            copy_xattrs(path, &f);
        }
        (&f).write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return written;
    }
    // The directory sync makes the rename durable; a file system that refuses it does not fail the save.
    if let Some(dir) = tmp.parent()
        && let Ok(d) = std::fs::File::open(dir)
    {
        let _ = d.sync_all();
    }
    Ok(())
}

/// Rewrites `path` in its own inode. A synced copy of the old bytes waits beside it until the new
/// ones are synced, and a failed write puts them back.
pub(crate) fn write_in_place(
    path: &Path,
    fill: impl FnOnce(&std::fs::File) -> io::Result<()>,
) -> io::Result<()> {
    let Some(backup) = (match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => Some(backup_beside(path, m.permissions())?),
        _ => None,
    }) else {
        let f = std::fs::File::create(path)?;
        fill(&f)?;
        return f.sync_all();
    };
    let f = match std::fs::File::create(path) {
        Ok(f) => f,
        Err(e) => {
            let _ = std::fs::remove_file(&backup);
            return Err(e);
        }
    };
    if let Err(e) = fill(&f).and_then(|()| f.sync_all()) {
        match restore(&f, &backup) {
            Ok(()) => {
                let _ = std::fs::remove_file(&backup);
            }
            Err(r) => log::error!(
                "restore {} from {}: {r}; the old text stays there",
                path.display(),
                backup.display()
            ),
        }
        return Err(e);
    }
    let _ = std::fs::remove_file(&backup);
    Ok(())
}

fn backup_beside(path: &Path, mode: std::fs::Permissions) -> io::Result<PathBuf> {
    let (mut copy, at) = create_temp_beside(path, Some(mode))?;
    let copied = std::fs::File::open(path)
        .and_then(|mut old| std::io::copy(&mut old, &mut copy))
        .and_then(|_| copy.sync_all());
    if let Err(e) = copied {
        let _ = std::fs::remove_file(&at);
        return Err(e);
    }
    Ok(at)
}

fn restore(mut f: &std::fs::File, backup: &Path) -> io::Result<()> {
    use std::io::Seek;
    f.set_len(0)?;
    f.rewind()?;
    std::io::copy(&mut std::fs::File::open(backup)?, &mut f)?;
    f.sync_all()
}

/// Give the fresh copy `to` the xattrs, mode and times of `from`. Xattrs go first: a `user.*`
/// one needs the write bit the mode may drop. Only a file's mode is not best effort (vfat refuses some).
pub(crate) fn copy_meta(
    from: &Path,
    from_meta: &std::fs::Metadata,
    to: &std::fs::File,
    dir: bool,
) -> io::Result<()> {
    copy_xattrs(from, to);
    let mode = to.set_permissions(from_meta.permissions());
    if !dir {
        mode?;
    }
    let mut times = std::fs::FileTimes::new();
    if let Ok(t) = from_meta.accessed() {
        times = times.set_accessed(t);
    }
    if let Ok(t) = from_meta.modified() {
        times = times.set_modified(t);
    }
    let _ = to.set_times(times);
    Ok(())
}

/// Copy `from`'s extended attributes (no link followed) onto `to`, best effort: never file
/// capabilities, the trust marker only made anew for the new inode, and nothing off a drive.
pub fn copy_xattrs(from: &Path, to: &std::fs::File) {
    copy_xattrs_as(from, to, from_drive(from, to));
}

/// Whether `from` is on a drive and `to` is not on its device.
fn from_drive(from: &Path, to: &std::fs::File) -> bool {
    use std::os::unix::fs::MetadataExt;
    let (Ok(src), Ok(dst)) = (std::fs::symlink_metadata(from), to.metadata()) else {
        return false;
    };
    src.dev() != dst.dev()
        && mounts_for(&MOUNTS, src.dev(), Instant::now(), || {
            std::fs::read("/proc/self/mountinfo").ok()
        })
        .is_some_and(|info| off_a_drive(&info, src.dev(), dst.dev()))
}

/// How long one read of the mount table answers [`from_drive`] for the devices it lists.
pub(super) const MOUNTS_FRESH: Duration = Duration::from_secs(2);

pub(super) type HeldMounts = Mutex<Option<(Instant, Arc<[u8]>)>>;

static MOUNTS: HeldMounts = Mutex::new(None);

/// The mount table `held` keeps, read again with `read` once it is [`MOUNTS_FRESH`] old or
/// when it lists no mount of device `dev`.
pub(super) fn mounts_for(
    held: &HeldMounts,
    dev: u64,
    now: Instant,
    read: impl FnOnce() -> Option<Vec<u8>>,
) -> Option<Arc<[u8]>> {
    let mut held = held
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((at, info)) = held.as_ref()
        && now.saturating_duration_since(*at) < MOUNTS_FRESH
        && crate::places::mount_points(info, dev).next().is_some()
    {
        return Some(info.clone());
    }
    let info: Arc<[u8]> = read()?.into();
    *held = Some((now, info.clone()));
    Some(info)
}

/// Whether a copy from device `src` onto `dst` leaves a drive ([`crate::places::drive_device`]).
pub(super) fn off_a_drive(mountinfo: &[u8], src: u64, dst: u64) -> bool {
    src != dst && crate::places::drive_device(mountinfo, src)
}

/// [`copy_xattrs`], carrying nothing when `from_drive`.
pub(super) fn copy_xattrs_as(from: &Path, to: &std::fs::File, from_drive: bool) {
    if from_drive {
        return;
    }
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    let Ok(src) = std::ffi::CString::new(from.as_os_str().as_bytes()) else {
        return;
    };
    // SAFETY: `src` is NUL-terminated and `buf` is writable for `len`.
    let Some(names) = read_sized(|buf, len| unsafe { libc::llistxattr(src.as_ptr(), buf, len) })
    else {
        return;
    };
    for name in names.split(|&b| b == 0).filter(|n| !n.is_empty()) {
        if name == b"security.capability" {
            continue;
        }
        let Ok(name) = std::ffi::CString::new(name) else {
            continue;
        };
        // SAFETY: `src` and `name` are NUL-terminated and `buf` is writable
        // for `len`.
        let value = read_sized(|buf, len| unsafe {
            libc::lgetxattr(src.as_ptr(), name.as_ptr(), buf.cast(), len)
        });
        let Some(value) = value else { continue };
        if name.as_c_str() == crate::apps::TRUST_XATTR {
            if crate::apps::verifies(&value, from) {
                let _ = crate::apps::set_trusted_fd(to);
            }
            continue;
        }
        // SAFETY: the fd is open, `name` is NUL-terminated and `value`
        // is readable for its length.
        unsafe {
            libc::fsetxattr(
                to.as_raw_fd(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        };
    }
}

/// Run a `*xattr` read that fills `buf` up to `len` and returns the byte
/// count: once with no buffer for the size, then into a buffer that size,
/// again if the value grew in between (`ERANGE`). `None` on any other error.
fn read_sized(mut read: impl FnMut(*mut libc::c_char, usize) -> isize) -> Option<Vec<u8>> {
    for _ in 0..4 {
        let size = usize::try_from(read(std::ptr::null_mut(), 0)).ok()?;
        let mut buf = vec![0u8; size];
        match usize::try_from(read(buf.as_mut_ptr().cast(), size)) {
            Ok(n) => {
                buf.truncate(n);
                return Some(buf);
            }
            Err(_) if io::Error::last_os_error().raw_os_error() == Some(libc::ERANGE) => {}
            Err(_) => return None,
        }
    }
    None
}
