use std::io;
use std::path::{Path, PathBuf};

/// The bytes of the regular file at `path` (a link to one is followed), at most
/// `max`. The open does not block and takes no controlling tty, so a FIFO or
/// device fails with `InvalidInput` instead of hanging; a file over `max` fails
/// with `FileTooLarge`. Every small file Reprise reads out of a user's folders is
/// read through here or [`read_opened`].
pub fn read_regular(path: &Path, max: u64) -> io::Result<Vec<u8>> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)?;
    read_opened(&file, max)
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
    use std::io::Write;
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
    let mode = std::fs::metadata(&path).ok().map(|m| m.permissions());
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

/// Copy the extended attributes of `from` (not followed through a symlink)
/// onto `to`: POSIX ACLs, `user.*` and `security.*` labels. Best effort:
/// one the file system or the process cannot read or set is skipped. File
/// capabilities are never copied, since they were granted to the old bytes.
/// The launcher trust marker names the inode, so a valid one is made anew for `to`.
pub fn copy_xattrs(from: &Path, to: &std::fs::File) {
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
