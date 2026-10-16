//! File operations, and the argument vectors and shell quoting around them.

use crate::replace::Seen;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// `s` in single quotes with embedded quotes escaped, safe for `sh -c`.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// How deep [`copy_dir_all`] recurses before giving up: a backstop for a tree
/// that grows under the walk.
const MAX_DEPTH: usize = 256;

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
pub(crate) fn copy_dir_until(src: &Path, dest: &Path, stop: &AtomicBool) -> io::Result<()> {
    let entries = read_then_create(src, dest)?;
    // `dest` is ours from here on, so a failure may remove it.
    copy_entries(entries, dest, 0, stop).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(dest);
    })
}

/// The entries of `src`, read before `dest` is created.
fn read_then_create(src: &Path, dest: &Path) -> io::Result<Vec<std::fs::DirEntry>> {
    let entries = std::fs::read_dir(src)?.collect::<io::Result<_>>()?;
    std::fs::create_dir(dest)?;
    Ok(entries)
}

/// Copy `entries` into `dest`, which sits `depth` folders below the top.
fn copy_entries(
    entries: Vec<std::fs::DirEntry>,
    dest: &Path,
    depth: usize,
    stop: &AtomicBool,
) -> io::Result<()> {
    for entry in entries {
        if stop.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "The copy was cancelled.",
            ));
        }
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
            copy_entries(read_then_create(&from, &to)?, &to, depth + 1, stop)?;
        } else if kind.is_file() {
            copy_new(&from, &to)?;
        }
    }
    Ok(())
}

/// `fs::copy`, except that an existing `to` is an `AlreadyExists` error
/// instead of being overwritten: the name was picked as free a moment
/// ago and nothing else may have taken it since. Keeps `from`'s mode
/// and its launcher trust marker. A copy that fails after `to` was
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
        dst.set_permissions(src.metadata()?.permissions())?;
        if crate::apps::is_trusted(from) {
            crate::apps::set_trusted_fd(&dst)?;
        }
        Ok(())
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
        back.advance(dst, len);
        meter.wrote(n as u64)?;
    }
    meter.wrote_file(dst, len, back.used());
    Ok(())
}

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

/// Copy the extended attributes of `from` (not followed through a symlink)
/// onto `to`: POSIX ACLs, `user.*` and `security.*` labels. Best effort:
/// one the file system or the process cannot read or set is skipped. File
/// capabilities are never copied, since they were granted to the old bytes.
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
        if let Some(value) = value {
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

/// `path` with `.` and `..` resolved: through the file system when it is
/// there (which also resolves symlinks), lexically when it is not.
fn resolved(path: &Path) -> PathBuf {
    if let Ok(real) = std::fs::canonicalize(path) {
        return real;
    }
    lexical(&std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()))
}

/// `path` with `.` dropped and each `..` taking off the part before it,
/// without asking the file system: `a/b/../c` is `a/c` even where `b` is
/// a symlink. A `..` with nothing before it is dropped.
pub fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Whether `dir` is `src` itself or somewhere under it. Copying a folder
/// into its own tree would copy the copy, and so on until the name runs
/// out of room, so [`put_in`] refuses it.
pub fn inside(src: &Path, dir: &Path) -> bool {
    let (src, dir) = (resolved(src), resolved(dir));
    dir.starts_with(&src)
}

/// Why [`put_in`] failed. `Display` is the sentence a host puts after
/// "Cannot move x: " in its error box ([`DropOp::failure`]).
#[derive(Debug)]
pub enum PutError {
    /// The destination folder is `src` or lies under it.
    Subfolder,
    /// A replacement that cannot go ahead: the folder being replaced
    /// holds the source, or a merge would put a file onto a folder (or
    /// a folder onto a file) somewhere in the tree.
    Occupied,
    /// A move into the folder that already holds `src`.
    SameFolder,
    /// A replacement of a file by itself: the same file under another
    /// link, or a link to the file it would replace.
    SameFile,
    Io(io::Error),
}

impl std::fmt::Display for PutError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            PutError::Subfolder => {
                f.write_str("The destination folder is a subfolder of the source folder.")
            }
            PutError::Occupied => {
                f.write_str("There is already a file with the same name in this location.")
            }
            PutError::SameFolder => {
                f.write_str("The source and destination file names are the same.")
            }
            PutError::SameFile => f.write_str("The source and destination are the same file."),
            PutError::Io(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for PutError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PutError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for PutError {
    fn from(e: io::Error) -> Self {
        PutError::Io(e)
    }
}

/// Whether a directory on device `dev`, inside a directory on
/// `parent_dev`, is the root of another file system: a mount point, or a
/// btrfs subvolume. A move off its device copies it, empties it, then
/// fails to remove the root itself, so none may be a move's source.
pub fn mount_root(dev: u64, parent_dev: u64) -> bool {
    dev != parent_dev
}

/// [`mount_root`] for `path` against its lexical parent. `/` is one; a
/// symlink never is (it lives in its parent's file system). A path that
/// cannot be read is not: the operation on it fails on its own.
pub fn is_mount_point(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if meta.is_symlink() {
        return false;
    }
    let parent = match path.parent() {
        None => return true,
        Some(p) if p.as_os_str().is_empty() => Path::new("."),
        Some(p) => p,
    };
    std::fs::metadata(parent).is_ok_and(|p| mount_root(meta.dev(), p.dev()))
}

/// `Err` when `path` [is a mount point](is_mount_point): the check every
/// move, trash and erase makes before it touches the source.
pub fn refuse_mount_point(path: &Path) -> io::Result<()> {
    if is_mount_point(path) {
        return Err(io::Error::new(
            io::ErrorKind::ResourceBusy,
            format!("{} is the root of a mounted drive", path.display()),
        ));
    }
    Ok(())
}

/// `Err` when a folder anywhere under `root` is a [`mount_root`]: the
/// check before a tree is copied then removed, erased or trashed, since
/// each would empty the drive mounted there. Links are not followed. A
/// file or a link as `root` holds nothing.
pub fn refuse_holding_mount(root: &Path) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let dev_of = |p: &Path| std::fs::symlink_metadata(p).map(|m| m.dev());
    if let Some(inner) = mount_under(root, &dev_of)? {
        return Err(io::Error::new(
            io::ErrorKind::ResourceBusy,
            format!(
                "{} contains a mounted drive at {}",
                root.display(),
                inner.display()
            ),
        ));
    }
    Ok(())
}

/// The first folder under `root` whose device, per `dev_of`, differs from
/// the folder holding it. A folder below `root` that cannot be listed or
/// looked at is skipped: the copy and the remove cannot reach into it
/// either, and fail there on their own.
fn mount_under(
    root: &Path,
    dev_of: &dyn Fn(&Path) -> io::Result<u64>,
) -> io::Result<Option<PathBuf>> {
    if !std::fs::symlink_metadata(root)?.is_dir() {
        return Ok(None);
    }
    let mut dirs = vec![(root.to_path_buf(), dev_of(root)?)];
    while let Some((dir, dev)) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            // `DirEntry::file_type` does not follow a link.
            if !entry.file_type().is_ok_and(|k| k.is_dir()) {
                continue;
            }
            let path = entry.path();
            let Ok(sub) = dev_of(&path) else {
                continue;
            };
            if mount_root(sub, dev) {
                return Ok(Some(path));
            }
            dirs.push((path, sub));
        }
    }
    Ok(None)
}

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

/// Whether this process may create entries in `dir`: write and search
/// permission for its effective ids, as the kernel judges them, so root
/// ownership, ACLs and a read-only mount all count. False for a path that
/// holds a NUL.
pub fn can_write(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `c` is a NUL-terminated path that outlives the call.
    unsafe {
        libc::faccessat(
            libc::AT_FDCWD,
            c.as_ptr(),
            libc::W_OK | libc::X_OK,
            libc::AT_EACCESS,
        ) == 0
    }
}

/// `dir/name`, numbered `name (2)`... ([`crate::names::first_free`]) if
/// taken.
pub fn unique_path(dir: &Path, name: impl AsRef<OsStr>) -> PathBuf {
    dir.join(crate::names::first_free(name.as_ref(), |c| {
        dir.join(c).exists()
    }))
}

/// Create a new file at the first free name of [`unique_path`]'s
/// numbering, for a writer whose name another user or a sandboxed app can
/// predict. `O_EXCL` means a file or symlink already at a name is never
/// opened or followed: that name counts as taken and the next is tried.
pub fn create_new_numbered(dir: &Path, name: &str) -> io::Result<(std::fs::File, PathBuf)> {
    let mut last = None;
    for candidate in crate::names::numbered(OsStr::new(name), crate::names::paren).take(1000) {
        let path = dir.join(candidate);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(f) => return Ok((f, path)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.expect("the loop ran"))
}

/// The argument vector that zips `paths`, items of one folder, into one
/// archive `zip` with bsdtar, which every libarchive install has. The
/// archive holds each item under its own name. Run with no shell, so each
/// path reaches bsdtar as its own bytes. bsdtar reads `-C dir` and
/// `@archive` among the names even past `--`, so a name starting with `-`
/// or `@` goes in as `./name`.
pub fn zip_argv(paths: &[PathBuf], zip: &Path) -> Vec<OsString> {
    let dir = paths
        .first()
        .and_then(|p| p.parent())
        .unwrap_or(Path::new("."));
    let mut argv: Vec<OsString> = ["bsdtar", "-a", "-cf"].map(OsString::from).into();
    argv.push(zip.into());
    argv.push("-C".into());
    argv.push(dir.into());
    argv.push("--".into());
    argv.extend(paths.iter().map(|p| {
        let name = p.file_name().unwrap_or_default();
        match name.as_encoded_bytes().first() {
            Some(b'-' | b'@') => Path::new(".").join(name).into_os_string(),
            _ => name.to_os_string(),
        }
    }));
    argv
}

/// Delete `path`: into its drive's trash ([`crate::trash::trash`]), or
/// for good through [`erase`], as `plan` says. A path already gone is not an error when
/// trashing. One on a drive without a Recycle Bin is refused with
/// `CrossesDevices`; the caller asks to erase it instead.
pub fn delete(path: &Path, plan: crate::bincap::Plan) -> io::Result<()> {
    match plan {
        crate::bincap::Plan::Erase => self::erase(path),
        crate::bincap::Plan::Trash(known) => {
            gone_is_done(crate::trash::trash(path, known).map(|(t, _)| t), path)
        }
    }
}

/// [`delete`]'s trashing, into the trash at `root`.
#[cfg(test)]
fn trash_in(root: &Path, path: &Path) -> io::Result<()> {
    gone_is_done(crate::trash::trash_in(root, path), path)
}

/// A trashing's result, with `path` already gone taken as done.
fn gone_is_done(r: io::Result<crate::trash::Trashed>, path: &Path) -> io::Result<()> {
    match r {
        Err(e)
            if e.kind() == io::ErrorKind::NotFound && std::fs::symlink_metadata(path).is_err() =>
        {
            Ok(())
        }
        r => r.map(drop),
    }
}

/// The right-drag menu's choice; a left drop takes the natural one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropOp {
    Copy,
    Move,
    Shortcut,
}

/// The op the held modifiers force on a drop, the same everywhere a drag
/// lands: Ctrl copies, Shift moves, Alt or Ctrl+Shift makes a shortcut.
/// None when no modifier picks one, so the drop takes its natural op.
pub fn modifier_op(ctrl: bool, shift: bool, alt: bool) -> Option<DropOp> {
    match (ctrl, shift, alt) {
        (_, _, true) | (true, true, false) => Some(DropOp::Shortcut),
        (true, false, false) => Some(DropOp::Copy),
        (false, true, false) => Some(DropOp::Move),
        (false, false, false) => None,
    }
}

impl DropOp {
    /// The error box's title and text when this op on `name` fails with
    /// `error`.
    pub fn failure(self, name: &str, error: &dyn std::fmt::Display) -> (&'static str, String) {
        let (title, what) = match self {
            DropOp::Move => ("Error Moving File or Folder", format!("move {name}")),
            DropOp::Copy => ("Error Copying File or Folder", format!("copy {name}")),
            DropOp::Shortcut => (
                "Error Creating Shortcut",
                format!("create a shortcut to {name}"),
            ),
        };
        (title, format!("Cannot {what}: {error}"))
    }
}

/// Whether `src` and `dir` sit on the same file system, which is what
/// decides a plain left drop: a move inside a drive, a copy between
/// drives. Anything unreadable answers "the same", the within-a-drive
/// default.
pub fn same_device(src: &Path, dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let of = |p: &Path| std::fs::metadata(p).ok().map(|m| m.dev());
    match (src.parent().and_then(of), of(dir)) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    }
}

/// Whether `src` sits directly in `dir`, through links and `..` alike:
/// a put there is a copy beside itself, or a move that changes nothing.
pub fn same_folder(src: &Path, dir: &Path) -> bool {
    src.parent().is_some_and(|p| resolved(p) == resolved(dir))
}

/// Whether two looks at the file system name the same file.
fn same_inode(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    (a.dev(), a.ino()) == (b.dev(), b.ino())
}

/// `src`'s own metadata for a put: a move takes a link as a link, a copy
/// follows it to what it points at, as a plain copy does.
fn source_meta(src: &Path, moving: bool) -> io::Result<std::fs::Metadata> {
    if moving {
        std::fs::symlink_metadata(src)
    } else {
        std::fs::metadata(src)
    }
}

/// Whether `dest` exists and is the same kind as `src` (both folders, or
/// both not): only then can one replace the other.
fn same_kind(src: &Path, dest: &Path, moving: bool) -> bool {
    match (source_meta(src, moving), std::fs::symlink_metadata(dest)) {
        (Ok(a), Ok(b)) => a.is_dir() == b.is_dir(),
        _ => false,
    }
}

/// Move `src` into `dir` under `name`, or the first free numbering of it
/// when the name is taken. Each candidate is tried by the move itself, so
/// a name taken meanwhile (by another job, or by a dangling link) is
/// passed over rather than retried. Returns the name it took.
fn move_numbered(src: &Path, dir: &Path, name: &OsStr) -> Result<OsString, PutError> {
    let mut last = None;
    for candidate in crate::names::numbered(name, crate::names::paren).take(1000) {
        match crate::trash::move_to_free(src, &dir.join(&candidate)) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e.into()),
        }
    }
    Err(PutError::Io(last.expect("the loop ran")))
}

/// Copy `src` (a file, or a folder with everything in it) into `dir`
/// under `name`, or the first free numbering of it, each candidate tried
/// by the copy itself as [`move_numbered`] tries it. Returns the path.
fn copy_numbered(src: &Path, dir: &Path, name: &OsStr, stop: &AtomicBool) -> io::Result<PathBuf> {
    let is_dir = src.is_dir();
    let mut last = None;
    for candidate in crate::names::numbered(name, crate::names::paren).take(1000) {
        let dest = dir.join(candidate);
        let made = if is_dir {
            copy_dir_until(src, &dest, stop)
        } else {
            copy_new(src, &dest)
        };
        match made {
            Ok(()) => return Ok(dest),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.expect("the loop ran"))
}

/// `renameat2(a, b, flags)`.
fn rename2(a: &Path, b: &Path, flags: libc::c_uint) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let (Ok(a), Ok(b)) = (
        std::ffi::CString::new(a.as_os_str().as_bytes()),
        std::ffi::CString::new(b.as_os_str().as_bytes()),
    ) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path holds a NUL",
        ));
    };
    // SAFETY: both are valid NUL-terminated paths; AT_FDCWD resolves them
    // against the working directory, as `rename` would.
    let rc = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            a.as_ptr(),
            libc::AT_FDCWD,
            b.as_ptr(),
            flags,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Whether an error says the file system has no `renameat2` flags.
fn unsupported(e: &io::Error) -> bool {
    matches!(e.raw_os_error(), Some(libc::EINVAL | libc::ENOSYS))
}

/// The device and inode at `path`, its link taken as a link.
fn id_at(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path)
        .ok()
        .map(|m| (m.dev(), m.ino()))
}

/// How putting a file over another ended.
enum Swap {
    /// It took the other file's place.
    Replaced,
    /// What held the name was no longer the file it was meant to replace
    /// (another job put its own there): it landed under this numbered
    /// name beside it instead.
    Numbered(OsString),
    /// The same, found only after the swap, and the swap back failed: it
    /// holds the name, and the other job's file was moved aside to this
    /// name (or kept under the temporary one, which is logged). Nothing
    /// was replaced.
    Displaced(OsString),
}

#[cfg(test)]
thread_local! {
    /// Tests: make the swap back fail, as a full disk or a vanished
    /// folder would.
    static FAIL_SWAP_BACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Swap `new` and `dest` back after a swap found the wrong file.
fn swap_back(new: &Path, dest: &Path) -> io::Result<()> {
    #[cfg(test)]
    if FAIL_SWAP_BACK.with(|f| f.get()) {
        return Err(io::Error::other("the swap back was made to fail"));
    }
    rename2(new, dest, libc::RENAME_EXCHANGE)
}

/// Put `new` (a file or link in `dest`'s folder, or a moved source on
/// its drive) over `dest`, but only while `dest` is still the file
/// `expect` names (device and inode). The two are swapped in one
/// `RENAME_EXCHANGE`; if what came back is not `expect`, they are
/// swapped back and `new` lands numbered instead, so a file another job
/// put there meanwhile is never lost. The replaced file is removed.
///
/// Once the first swap has happened, nothing here removes either file:
/// a failed swap back leaves `new` holding the name and moves the other
/// job's file aside ([`Swap::Displaced`]), and a replaced file that will
/// not go is left under the temporary name with a warning.
///
/// A file system without `RENAME_EXCHANGE` checks `dest` just before a
/// plain rename: a file landing in the moment between the two is the one
/// window left.
fn swap_in(new: &Path, dest: &Path, expect: (u64, u64)) -> Result<Swap, PutError> {
    let dir = dest.parent().unwrap_or(Path::new("."));
    let name = dest.file_name().unwrap_or_default();
    match rename2(new, dest, libc::RENAME_EXCHANGE) {
        Ok(()) if id_at(new) == Some(expect) => {
            if let Err(e) = std::fs::remove_file(new) {
                log::warn!(
                    "reprisefs: replaced {}, but the old file stays at {}: {e}",
                    dest.display(),
                    new.display()
                );
            }
            Ok(Swap::Replaced)
        }
        Ok(()) => match swap_back(new, dest) {
            Ok(()) => Ok(Swap::Numbered(move_numbered(new, dir, name)?)),
            Err(e) => {
                // `new` now holds another job's file, and ours holds the
                // name: keep both.
                log::warn!(
                    "reprisefs: {} changed before it was replaced and cannot be swapped back \
                     ({e}); moving it aside",
                    dest.display()
                );
                match move_numbered(new, dir, name) {
                    Ok(aside) => Ok(Swap::Displaced(aside)),
                    Err(e) => {
                        log::warn!(
                            "reprisefs: the file that was at {} is kept at {}: {e}",
                            dest.display(),
                            new.display()
                        );
                        Ok(Swap::Displaced(
                            new.file_name().unwrap_or_default().to_owned(),
                        ))
                    }
                }
            }
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound && std::fs::symlink_metadata(new).is_ok() => {
            Ok(Swap::Numbered(move_numbered(new, dir, name)?))
        }
        Err(e) if unsupported(&e) => {
            if id_at(dest) == Some(expect) {
                std::fs::rename(new, dest)?;
                Ok(Swap::Replaced)
            } else {
                Ok(Swap::Numbered(move_numbered(new, dir, name)?))
            }
        }
        Err(e) => Err(e.into()),
    }
}

/// Flush `path` to the disk, and everything under it when it is a folder
/// (links are not followed): the step before a move to another drive
/// removes the original.
pub(crate) fn sync_tree(path: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() {
        for entry in std::fs::read_dir(path)? {
            sync_tree(&entry?.path())?;
        }
    }
    if meta.is_dir() || meta.is_file() {
        match std::fs::File::open(path) {
            Ok(f) => f.sync_all()?,
            // A file this process can write but not read back (mode 0200)
            // is flushed with its folder instead.
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Flush `path` and the folder entry that names it.
pub(crate) fn sync_landed(path: &Path) -> io::Result<()> {
    sync_tree(path)?;
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::File::open(dir)?.sync_all()
}

/// Refuse a move to another drive whose original cannot be removed once
/// it is copied: the folder holding it must be writable.
fn refuse_stuck_original(src: &Path) -> Result<(), PutError> {
    let parent = src.parent().unwrap_or(Path::new("."));
    if can_write(parent) {
        return Ok(());
    }
    Err(PutError::Io(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "{} cannot be removed from where it is, so it cannot be moved.",
            src.file_name().unwrap_or_default().to_string_lossy()
        ),
    )))
}

/// Make the temporary file beside the one being replaced with `make`,
/// under a short hidden name of its own (the file's name with more
/// around it could run past the file system's limit), numbered past one
/// already there, whether a file or a dangling link. Returns its path.
fn make_part(dir: &Path, make: impl Fn(&Path) -> io::Result<()>) -> io::Result<PathBuf> {
    let mut last = None;
    for candidate in
        crate::names::numbered(OsStr::new(".reprise.part"), crate::names::paren).take(1000)
    {
        let part = dir.join(candidate);
        match make(&part) {
            Ok(()) => return Ok(part),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.expect("the loop ran"))
}

/// The cancelled copy's error.
fn cancelled() -> PutError {
    PutError::Io(io::Error::new(
        io::ErrorKind::Interrupted,
        "The copy was cancelled.",
    ))
}

/// Whether putting `src` (whose own metadata is `from`) over `dest`
/// would remove the very file it is: `dest` is `src` under another link,
/// or `src` is a link to `dest`.
fn is_itself(src: &Path, from: &std::fs::Metadata, dest: &std::fs::Metadata) -> bool {
    same_inode(from, dest) || std::fs::metadata(src).is_ok_and(|m| same_inode(&m, dest))
}

/// Put `src` where `dest` already is, replacing it: the step behind a
/// Yes on the replace prompt. A file (or a moved link) takes the place of
/// the file or link at `dest` in one rename, so `dest` is never missing:
/// a copy is written beside it under a short hidden name first. A folder
/// merges into the folder at `dest`, entry by entry; a moved folder is
/// removed once it is empty.
///
/// Refused before anything changes:
/// - `dest` is `src` itself, another link to the same file, or what
///   `src` links to, anywhere in the tree ([`PutError::SameFile`]).
/// - `dest` holds `src`, or a merge would put a file onto a folder or a
///   folder onto a file (or a link) anywhere in the tree
///   ([`PutError::Occupied`]).
/// - a folder under `src` that cannot be read, a merge deeper than
///   [`MAX_DEPTH`], or a move to another drive of a tree holding a FIFO
///   or a socket, which the move could not carry.
/// - a move to another drive whose original could not be removed after
///   the copy: its folder, or a folder under a merged one, is read-only.
/// - a mount point as a move's source, or a moved folder holding one.
///
/// A file lands only over `expect` (device and inode), the file the
/// prompt asked about; anything else there by then stays, and the file
/// lands numbered beside it ([`swap_in`]).
fn replace_into(
    src: &Path,
    dest: &Path,
    moving: bool,
    expect: (u64, u64),
    stop: &AtomicBool,
) -> Result<Swap, PutError> {
    use std::os::unix::fs::MetadataExt;
    if stop.load(Ordering::Relaxed) {
        return Err(cancelled());
    }
    let from = source_meta(src, moving)?;
    let to = std::fs::symlink_metadata(dest)?;
    if is_itself(src, &from, &to) {
        return Err(PutError::SameFile);
    }
    if resolved(src).starts_with(resolved(dest)) {
        return Err(PutError::Occupied);
    }
    if moving {
        refuse_mount_point(src)?;
    }
    match (from.is_dir(), to.is_dir()) {
        (true, true) => {
            if moving {
                refuse_holding_mount(src)?;
            }
            let crossing = moving && from.dev() != to.dev();
            if crossing {
                refuse_stuck_original(src)?;
            }
            preflight_merge(src, Some(dest), moving, crossing, 0)?;
            merge_into(src, dest, moving, 0, stop)?;
            Ok(Swap::Replaced)
        }
        (false, false) => {
            if moving && from.dev() != to.dev() {
                refuse_stuck_original(src)?;
            }
            replace_file(src, dest, &from, moving, expect)
        }
        _ => Err(PutError::Occupied),
    }
}

/// Whether a merge copies or moves this kind of entry. A copy skips what
/// is neither a file, a folder nor a link (a FIFO, a socket), as
/// [`copy_dir_all`] does; a move renames it like anything else.
fn merges(kind: std::fs::FileType, moving: bool) -> bool {
    moving || kind.is_dir() || kind.is_file() || kind.is_symlink()
}

/// Walk the merge of `src` into `dest` (None past where `dest` has
/// nothing) without changing anything, refusing it whole when:
/// - an entry meets one of the other kind (a file or a link onto a
///   folder, a folder onto a file or a link): [`PutError::Occupied`].
/// - an entry would replace itself: [`PutError::SameFile`].
/// - a folder cannot be read, or the tree runs past [`MAX_DEPTH`].
/// - `crossing` (a move to another drive, which copies) meets a FIFO or
///   a socket, which the copy cannot carry, or a read-only folder, whose
///   entries could not be removed after the copy.
fn preflight_merge(
    src: &Path,
    dest: Option<&Path>,
    moving: bool,
    crossing: bool,
    depth: usize,
) -> Result<(), PutError> {
    if depth >= MAX_DEPTH {
        return Err(too_deep().into());
    }
    if crossing && !can_write(src) {
        return Err(PutError::Io(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{} is read-only, so what is in it cannot be moved.",
                src.file_name().unwrap_or_default().to_string_lossy()
            ),
        )));
    }
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let meta = std::fs::symlink_metadata(&from)?;
        let kind = meta.file_type();
        let plain = kind.is_dir() || kind.is_file() || kind.is_symlink();
        if crossing && !plain {
            return Err(PutError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{} cannot be moved to another drive.",
                    from.file_name().unwrap_or_default().to_string_lossy()
                ),
            )));
        }
        if !merges(kind, moving) {
            continue;
        }
        let there = dest.map(|d| d.join(entry.file_name()));
        let there_meta = there
            .as_deref()
            .and_then(|t| std::fs::symlink_metadata(t).ok());
        match (&there_meta, kind.is_dir()) {
            (Some(t), true) if t.is_dir() => {
                preflight_merge(&from, there.as_deref(), moving, crossing, depth + 1)?
            }
            (Some(t), false) if !t.is_dir() => {
                if is_itself(&from, &meta, t) {
                    return Err(PutError::SameFile);
                }
            }
            (Some(_), _) => return Err(PutError::Occupied),
            (None, true) => preflight_merge(&from, None, moving, crossing, depth + 1)?,
            (None, false) => {}
        }
    }
    Ok(())
}

fn too_deep() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("The folder is more than {MAX_DEPTH} levels deep."),
    )
}

/// Merge the entries of folder `src` into folder `dest`, which sits
/// `depth` folders below the top. Links are merged as links.
fn merge_into(
    src: &Path,
    dest: &Path,
    moving: bool,
    depth: usize,
    stop: &AtomicBool,
) -> Result<(), PutError> {
    if depth >= MAX_DEPTH {
        return Err(too_deep().into());
    }
    let entries = std::fs::read_dir(src)?.collect::<io::Result<Vec<_>>>()?;
    for entry in entries {
        if stop.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let (from, to) = (entry.path(), dest.join(entry.file_name()));
        let meta = std::fs::symlink_metadata(&from)?;
        let kind = meta.file_type();
        if !merges(kind, moving) {
            continue;
        }
        match std::fs::symlink_metadata(&to) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if moving {
                    crate::trash::move_to_free(&from, &to)?;
                } else if kind.is_dir() {
                    copy_dir_until(&from, &to, stop)?;
                } else if kind.is_symlink() {
                    std::os::unix::fs::symlink(std::fs::read_link(&from)?, &to)?;
                } else {
                    copy_new(&from, &to)?;
                }
            }
            Err(e) => return Err(e.into()),
            Ok(there) if kind.is_dir() && there.is_dir() => {
                merge_into(&from, &to, moving, depth + 1, stop)?
            }
            Ok(there) if !kind.is_dir() && !there.is_dir() => {
                use std::os::unix::fs::MetadataExt;
                let expect = (there.dev(), there.ino());
                if let Swap::Numbered(name) | Swap::Displaced(name) =
                    replace_file(&from, &to, &meta, moving, expect)?
                {
                    log::info!(
                        "reprisefs: {} changed during the merge; {} is beside it",
                        to.display(),
                        name.to_string_lossy()
                    );
                }
            }
            // The preflight found none; one that appeared since stops here.
            Ok(_) => return Err(PutError::Occupied),
        }
    }
    // Everything is in `dest` by now; a folder that will not go (a file
    // appeared in it meanwhile) is left behind rather than failing a
    // merge that happened.
    if moving && let Err(e) = std::fs::remove_dir(src) {
        log::warn!(
            "reprisefs: merged {}, but cannot remove it: {e}",
            src.display()
        );
    }
    Ok(())
}

/// Put file (or link) `src`, whose metadata is `from`, over the file or
/// link at `dest`, as long as that is still `expect` ([`swap_in`]). A
/// link is written as a link only when `from` is the link's own
/// metadata. A file that is `dest` itself (another link to it, or a link
/// pointing at it) is refused, since the replacement would remove it.
/// A move to another drive flushes the copy to the disk before it
/// removes the original.
fn replace_file(
    src: &Path,
    dest: &Path,
    from: &std::fs::Metadata,
    moving: bool,
    expect: (u64, u64),
) -> Result<Swap, PutError> {
    if is_itself(src, from, &std::fs::symlink_metadata(dest)?) {
        return Err(PutError::SameFile);
    }
    if moving {
        match swap_in(src, dest, expect) {
            Err(PutError::Io(e)) if e.raw_os_error() == Some(libc::EXDEV) => {}
            done => return done,
        }
    }
    let dir = dest.parent().unwrap_or(Path::new("."));
    let part = if from.file_type().is_symlink() {
        let target = std::fs::read_link(src)?;
        make_part(dir, |p| std::os::unix::fs::symlink(&target, p))?
    } else {
        make_part(dir, |p| copy_new(src, p))?
    };
    if moving && let Err(e) = sync_landed(&part) {
        let _ = std::fs::remove_file(&part);
        return Err(e.into());
    }
    // `swap_in` fails only before its swap, while `part` is still ours.
    let swap = swap_in(&part, dest, expect).inspect_err(|_| {
        let _ = std::fs::remove_file(&part);
    })?;
    if moving {
        // The new name is on the disk before the original goes; a source
        // that will not go is left behind with a warning, not reported as
        // a move that did nothing.
        let flushed = std::fs::File::open(dir).and_then(|d| d.sync_all());
        let removed = flushed.and_then(|()| std::fs::remove_file(src));
        if let Err(e) = removed {
            log::warn!(
                "reprisefs: moved {} over {}, but the original stays: {e}",
                src.display(),
                dest.display()
            );
        }
    }
    Ok(swap)
}

/// Copy, move or make a shortcut to `src` inside `dir`, named past `existing`
/// and past what `dir` holds: a copy or move onto a taken name lands numbered, "x
/// (2)". [`put_in_with`] with `replace` puts it over the taken name instead, once
/// the user said so ([`crate::replace`]). `is_app` marks a program's .desktop
/// file, which is always copied and made a launcher
/// ([`crate::apps::allow_launching`]). `shortcut` builds the launcher text for a
/// `Shortcut` drop from the display name; None refuses the drop. The launcher is
/// written 0755 and trusted ([`crate::apps::write_launcher`]). Returns the file
/// name of what landed, byte for byte (a name that is not UTF-8 is kept).
///
/// Refused before anything is written:
/// - a copy or a move into `src`'s own tree ([`PutError::Subfolder`]):
///   the destination would sit inside what is being copied. A shortcut
///   is not refused: a launcher pointing at its own folder is harmless.
/// - a move into the folder that already holds `src`
///   ([`PutError::SameFolder`]): it would change nothing.
/// - a copy of anything but a file or a folder (a FIFO, a socket, a
///   device), as an `InvalidInput` [`PutError::Io`].
pub fn put_in(
    src: &Path,
    display_name: &str,
    is_app: bool,
    dir: &Path,
    op: DropOp,
    existing: &[String],
    shortcut: &dyn Fn(&str) -> Option<String>,
) -> Result<OsString, PutError> {
    put_in_with(src, display_name, is_app, dir, op, existing, shortcut, None)
}

/// [`put_in`], and with `replace` a copy or move onto a taken name goes over it:
/// a file takes the other file's place, a folder merges into the other, its files
/// replacing same-named ones ([`replace_into`]). `replace` is what the prompt saw
/// at that name ([`Seen`]): anything else there by now is not replaced, nor is a
/// file by a folder of the same name or the reverse; the put is numbered past it.
#[allow(clippy::too_many_arguments)]
pub fn put_in_with(
    src: &Path,
    display_name: &str,
    is_app: bool,
    dir: &Path,
    op: DropOp,
    existing: &[String],
    shortcut: &dyn Fn(&str) -> Option<String>,
    replace: Option<Seen>,
) -> Result<OsString, PutError> {
    let never = AtomicBool::new(false);
    put_in_until(
        src,
        display_name,
        is_app,
        dir,
        op,
        existing,
        shortcut,
        replace,
        &never,
    )
    .map(|l| l.name)
}

/// Where [`put_in_until`] put an item.
#[derive(Debug)]
pub(crate) struct Landing {
    /// The file name it landed under, byte for byte.
    pub name: OsString,
    /// It took the place of what held that name, rather than landing
    /// beside it.
    pub replaced: bool,
    /// A move whose old folder's `.hidden` listed it: the line went
    /// with the move ([`crate::hidden::carry`]).
    pub unlisted: bool,
}

impl Landing {
    fn new((name, replaced): (OsString, bool)) -> Landing {
        Landing {
            name,
            replaced,
            unlisted: false,
        }
    }
}

/// [`put_in_with`], with a folder copy stopped by `stop` as
/// [`copy_dir_until`] stops it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn put_in_until(
    src: &Path,
    display_name: &str,
    is_app: bool,
    dir: &Path,
    op: DropOp,
    existing: &[String],
    shortcut: &dyn Fn(&str) -> Option<String>,
    replace: Option<Seen>,
    stop: &AtomicBool,
) -> Result<Landing, PutError> {
    // Only what the prompt saw, and only by its own kind.
    let over = |dest: &Path, moving: bool| {
        replace.filter(|seen| seen.still(dest) && same_kind(src, dest, moving))
    };
    let landing = |swap: Swap, name: OsString| match swap {
        Swap::Replaced => (name, true),
        Swap::Numbered(other) => {
            log::info!(
                "reprisefs: {} changed after the prompt; {} landed as {} instead",
                dir.join(&name).display(),
                src.display(),
                other.to_string_lossy()
            );
            (other, false)
        }
        Swap::Displaced(aside) => {
            log::info!(
                "reprisefs: {} changed after the prompt; it was moved aside to {}",
                dir.join(&name).display(),
                aside.to_string_lossy()
            );
            (name, false)
        }
    };
    let file_name = src
        .file_name()
        .map(OsStr::to_owned)
        .unwrap_or_else(|| display_name.into());
    let same_dir = same_folder(src, dir);
    if op != DropOp::Shortcut && inside(src, dir) {
        return Err(PutError::Subfolder);
    }
    match op {
        DropOp::Move if !is_app && same_dir => Err(PutError::SameFolder),
        DropOp::Move if !is_app => {
            let dest = dir.join(&file_name);
            let landed = if let Some(seen) = over(&dest, true) {
                let swap = replace_into(src, &dest, true, seen.id(), stop)?;
                landing(swap, file_name)
            } else {
                // Never `fs::rename` straight over the destination: that
                // would delete whatever shares the name without a word. A
                // taken name is numbered past instead.
                (move_numbered(src, dir, &file_name)?, false)
            };
            let mut landed = Landing::new(landed);
            // The old folder's list lets go of the name; the new one
            // gains nothing.
            match crate::hidden::carry(src, &dir.join(&landed.name)) {
                Ok(unlisted) => landed.unlisted = unlisted,
                Err(e) => log::warn!(
                    "reprisefs: moved {}, but not its .hidden line: {e}",
                    src.display()
                ),
            }
            Ok(landed)
        }
        DropOp::Move | DropOp::Copy => {
            let name = if is_app || !same_dir {
                file_name
            } else {
                crate::names::copy_name(&file_name.to_string_lossy(), existing).into()
            };
            // Told to replace, a copy takes the place of what holds its
            // name. Otherwise, and for a program's launcher or a copy
            // into its own folder, a taken name is numbered past.
            if !is_app
                && !same_dir
                && let Some(seen) = over(&dir.join(&name), false)
            {
                let swap = replace_into(src, &dir.join(&name), false, seen.id(), stop)?;
                return Ok(Landing::new(landing(swap, name)));
            }
            let dest = copy_numbered(src, dir, &name, stop)?;
            if is_app && let Err(e) = crate::apps::allow_launching(&dest) {
                let _ = std::fs::remove_file(&dest);
                return Err(e.into());
            }
            Ok(Landing::new((
                dest.file_name().map(OsStr::to_owned).unwrap_or(name),
                false,
            )))
        }
        DropOp::Shortcut => {
            let name = crate::names::shortcut_name(display_name, existing);
            let text = shortcut(&name).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a name with a control character cannot go in a shortcut",
                )
            })?;
            let dest = unique_path(dir, format!("{name}.desktop"));
            crate::apps::write_launcher(&dest, &text)?;
            Ok(Landing::new((
                dest.file_name().map(OsStr::to_owned).unwrap_or(name.into()),
                false,
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testfs::mkfifo;
    use std::fs;

    #[test]
    fn read_regular_refuses_a_fifo_a_folder_and_a_file_over_the_cap() {
        let t = tempfile::tempdir().unwrap();
        let fifo = t.path().join("fifo");
        mkfifo(&fifo);
        let kind = |p: &Path, max| read_regular(p, max).unwrap_err().kind();
        assert_eq!(kind(&fifo, 10), io::ErrorKind::InvalidInput);
        assert_eq!(kind(t.path(), 10), io::ErrorKind::InvalidInput);
        let file = t.path().join("f");
        fs::write(&file, "0123456789").unwrap();
        assert_eq!(kind(&file, 9), io::ErrorKind::FileTooLarge);
        assert_eq!(read_regular(&file, 10).unwrap(), b"0123456789");
    }

    #[test]
    fn create_new_numbered_skips_a_planted_symlink_and_a_taken_name() {
        let t = tempfile::tempdir().unwrap();
        let victim = t.path().join("victim");
        fs::write(&victim, "keep").unwrap();
        std::os::unix::fs::symlink(&victim, t.path().join("Shot.png")).unwrap();
        fs::write(t.path().join("Shot (2).png"), "older").unwrap();
        let (mut f, path) = create_new_numbered(t.path(), "Shot.png").unwrap();
        io::Write::write_all(&mut f, b"new").unwrap();
        assert_eq!(path, t.path().join("Shot (3).png"));
        assert_eq!(fs::read_to_string(&victim).unwrap(), "keep");
        assert_eq!(
            fs::read_to_string(t.path().join("Shot (2).png")).unwrap(),
            "older"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    }

    /// A dot followed by a space does not start an extension, on disk as
    /// in a listing: "Mr. Smith" numbers as a whole.
    #[test]
    fn unique_path_numbers_a_name_whose_last_dot_is_not_an_extension() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("Mr. Smith"), "").unwrap();
        fs::write(t.path().join("a.txt"), "").unwrap();
        assert_eq!(
            unique_path(t.path(), "Mr. Smith"),
            t.path().join("Mr. Smith (2)")
        );
        assert_eq!(unique_path(t.path(), "a.txt"), t.path().join("a (2).txt"));
        assert_eq!(unique_path(t.path(), "b.txt"), t.path().join("b.txt"));
    }

    #[test]
    fn shell_quote_wraps_in_single_quotes_and_escapes_them() {
        assert_eq!(shell_quote("plain"), "'plain'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn zip_argv_runs_bsdtar() {
        assert_eq!(
            zip_argv(
                &["/home/u/Desktop/Reports".into()],
                Path::new("/home/u/Desktop/Reports.zip")
            ),
            [
                "bsdtar",
                "-a",
                "-cf",
                "/home/u/Desktop/Reports.zip",
                "-C",
                "/home/u/Desktop",
                "--",
                "Reports"
            ]
        );
    }

    #[test]
    fn zip_argv_keeps_a_name_that_is_not_utf8() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let path = Path::new(OsStr::from_bytes(b"/d/\xff dir/a\xff.txt"));
        let zip = Path::new(OsStr::from_bytes(b"/d/\xff dir/a\xff.zip"));
        let argv = zip_argv(&[path.to_path_buf()], zip);
        assert_eq!(argv[3], zip.as_os_str());
        assert_eq!(argv[5], OsStr::from_bytes(b"/d/\xff dir"));
        assert_eq!(argv[7], OsStr::from_bytes(b"a\xff.txt"));
    }

    /// A moved item leaves its old folder's `.hidden`, and the new
    /// folder's list gains nothing; a copy leaves the list alone.
    #[test]
    fn a_move_takes_a_listed_name_off_the_old_list_only() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("x.txt"), "1").unwrap();
        fs::write(a.join("y.txt"), "2").unwrap();
        fs::write(a.join(".hidden"), "x.txt\ny.txt\n").unwrap();
        let sc = |_: &str| Some(String::new());
        put_in(&a.join("y.txt"), "y.txt", false, &b, DropOp::Copy, &[], &sc).unwrap();
        put_in(&a.join("x.txt"), "x.txt", false, &b, DropOp::Move, &[], &sc).unwrap();
        assert_eq!(fs::read_to_string(a.join(".hidden")).unwrap(), "y.txt\n");
        assert!(!b.join(".hidden").exists());
    }

    #[test]
    fn put_in_moves_copies_and_names_a_copy_in_the_same_folder() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a");
        let b = t.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("x.txt"), "1").unwrap();
        let sc = |_: &str| Some(String::new());
        assert_eq!(
            put_in(&a.join("x.txt"), "x.txt", false, &b, DropOp::Move, &[], &sc).unwrap(),
            "x.txt"
        );
        assert!(b.join("x.txt").exists() && !a.join("x.txt").exists());
        let name = put_in(
            &b.join("x.txt"),
            "x.txt",
            false,
            &b,
            DropOp::Copy,
            &["x.txt".into()],
            &sc,
        )
        .unwrap();
        assert_eq!(name, "Copy of x.txt");
        assert!(b.join("Copy of x.txt").exists());
    }

    /// A name that is not UTF-8 keeps its bytes through a move and a
    /// copy, and `put_in` returns them, so an undo finds the file.
    #[test]
    fn put_in_keeps_a_non_utf8_name() {
        use std::os::unix::ffi::OsStrExt;
        let t = tempfile::tempdir().unwrap();
        let (a, b, c) = (t.path().join("a"), t.path().join("b"), t.path().join("c"));
        for d in [&a, &b, &c] {
            fs::create_dir(d).unwrap();
        }
        let name = std::ffi::OsStr::from_bytes(b"caf\xe9.txt");
        fs::write(a.join(name), "hi").unwrap();
        let sc = |_: &str| None;

        let moved = put_in(
            &a.join(name),
            "caf\u{fffd}.txt",
            false,
            &b,
            DropOp::Move,
            &[],
            &sc,
        );
        assert_eq!(moved.unwrap(), name);
        assert!(b.join(name).is_file());

        let copied = put_in(
            &b.join(name),
            "caf\u{fffd}.txt",
            false,
            &c,
            DropOp::Copy,
            &[],
            &sc,
        );
        assert_eq!(copied.unwrap(), name);
        assert!(c.join(name).is_file());

        // Taken in the destination: replaced when told to, bytes kept.
        fs::write(b.join(name), "again").unwrap();
        let again = put_in_with(
            &b.join(name),
            "caf\u{fffd}.txt",
            false,
            &c,
            DropOp::Copy,
            &[],
            &sc,
            Seen::of(&c.join(name)),
        );
        assert_eq!(again.unwrap(), name);
        assert_eq!(fs::read_to_string(c.join(name)).unwrap(), "again");
    }

    #[test]
    fn put_in_shortcut_writes_the_callers_text() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("doc.txt"), "").unwrap();
        let name = put_in(
            &t.path().join("doc.txt"),
            "doc.txt",
            false,
            t.path(),
            DropOp::Shortcut,
            &[],
            &|n| Some(format!("[Desktop Entry]\nName={n}\n")),
        )
        .unwrap();
        assert_eq!(name, "Shortcut to doc.txt.desktop");
        assert_eq!(
            fs::read_to_string(t.path().join("Shortcut to doc.txt.desktop")).unwrap(),
            "[Desktop Entry]\nName=Shortcut to doc.txt\n"
        );
    }

    /// Pasting a folder into itself, into a folder under it, or through
    /// a path with `..` in it: all refused, and nothing is written.
    #[test]
    fn put_in_refuses_a_destination_inside_the_source() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("Reports");
        let deep = src.join("2005").join("Q1");
        fs::create_dir_all(&deep).unwrap();
        fs::write(src.join("x.txt"), "1").unwrap();
        let sc = |_: &str| Some(String::new());
        let before = fs::read_dir(&src).unwrap().count();

        for dir in [
            src.clone(),
            deep.clone(),
            src.join("2005").join("..").join("2005"),
        ] {
            for op in [DropOp::Copy, DropOp::Move] {
                let err = put_in(&src, "Reports", false, &dir, op, &[], &sc).unwrap_err();
                assert!(matches!(err, PutError::Subfolder), "{dir:?} {op:?}");
                assert_eq!(
                    err.to_string(),
                    "The destination folder is a subfolder of the source folder."
                );
            }
        }
        assert_eq!(fs::read_dir(&src).unwrap().count(), before);
        assert!(fs::read_dir(&deep).unwrap().next().is_none());

        // A folder copied beside itself is not its own subfolder.
        let away = t.path().join("elsewhere");
        fs::create_dir(&away).unwrap();
        assert_eq!(
            put_in(&src, "Reports", false, &away, DropOp::Copy, &[], &sc).unwrap(),
            "Reports"
        );
        assert!(away.join("Reports").join("x.txt").exists());
        // And a shortcut inside the folder itself is still allowed.
        assert!(
            put_in(&src, "Reports", false, &src, DropOp::Shortcut, &[], &|n| {
                Some(format!("[Desktop Entry]\nName={n}\n"))
            })
            .is_ok()
        );
    }

    /// A move into the folder that already holds the file is refused and
    /// makes no copy.
    #[test]
    fn put_in_refuses_a_move_into_the_same_folder() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("x.txt"), "1").unwrap();
        let sc = |_: &str| None;
        let e = put_in(
            &t.path().join("x.txt"),
            "x.txt",
            false,
            t.path(),
            DropOp::Move,
            &["x.txt".into()],
            &sc,
        )
        .unwrap_err();
        assert!(matches!(e, PutError::SameFolder));
        assert_eq!(fs::read_dir(t.path()).unwrap().count(), 1);
    }

    /// A move or a copy onto a name the destination folder already holds
    /// lands numbered unless told to replace, and the file already there
    /// is left alone.
    #[test]
    fn put_in_numbers_past_an_existing_name() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a");
        let b = t.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("x.txt"), "source").unwrap();
        fs::write(b.join("x.txt"), "destination").unwrap();
        let sc = |_: &str| Some(String::new());

        let put = |op| put_in(&a.join("x.txt"), "x.txt", false, &b, op, &[], &sc).unwrap();
        assert_eq!(put(DropOp::Copy), "x (2).txt");
        assert_eq!(put(DropOp::Move), "x (3).txt");
        assert_eq!(fs::read_to_string(b.join("x.txt")).unwrap(), "destination");
        assert_eq!(fs::read_to_string(b.join("x (3).txt")).unwrap(), "source");
        assert!(!a.join("x.txt").exists(), "the move took its source");
    }

    /// Told to replace, a copy overwrites the file of the same name and a
    /// move takes its place, leaving no temporary file behind.
    #[test]
    fn put_in_with_replace_overwrites_a_file_by_copy_and_by_move() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("x.txt"), "source").unwrap();
        fs::write(b.join("x.txt"), "destination").unwrap();
        let sc = |_: &str| None;
        let put = |op| {
            put_in_with(
                &a.join("x.txt"),
                "x.txt",
                false,
                &b,
                op,
                &[],
                &sc,
                Seen::of(&b.join("x.txt")),
            )
        };

        assert_eq!(put(DropOp::Copy).unwrap(), "x.txt");
        assert_eq!(fs::read_to_string(b.join("x.txt")).unwrap(), "source");
        assert!(a.join("x.txt").exists(), "a copy keeps its source");

        fs::write(b.join("x.txt"), "destination").unwrap();
        assert_eq!(put(DropOp::Move).unwrap(), "x.txt");
        assert_eq!(fs::read_to_string(b.join("x.txt")).unwrap(), "source");
        assert!(!a.join("x.txt").exists(), "a move takes its source");
        assert_eq!(fs::read_dir(&b).unwrap().count(), 1, "no .part file left");
    }

    /// Told to replace, a folder merges into the folder of the same name:
    /// its files replace theirs, the others on both sides stay, and a
    /// moved folder is gone afterwards.
    #[test]
    fn put_in_with_replace_merges_a_folder() {
        let t = tempfile::tempdir().unwrap();
        let (src, dest) = (t.path().join("src/Reports"), t.path().join("dest"));
        fs::create_dir_all(src.join("q1")).unwrap();
        fs::create_dir_all(dest.join("Reports/q1")).unwrap();
        fs::write(src.join("same.txt"), "new").unwrap();
        fs::write(src.join("q1/deep.txt"), "new deep").unwrap();
        fs::write(src.join("mine.txt"), "mine").unwrap();
        fs::write(dest.join("Reports/same.txt"), "old").unwrap();
        fs::write(dest.join("Reports/q1/deep.txt"), "old deep").unwrap();
        fs::write(dest.join("Reports/theirs.txt"), "theirs").unwrap();
        let sc = |_: &str| None;
        let put = |op| {
            put_in_with(
                &src,
                "Reports",
                false,
                &dest,
                op,
                &[],
                &sc,
                Seen::of(&dest.join("Reports")),
            )
        };

        assert_eq!(put(DropOp::Copy).unwrap(), "Reports");
        let read = |p: &str| fs::read_to_string(dest.join("Reports").join(p)).unwrap();
        assert_eq!(read("same.txt"), "new");
        assert_eq!(read("q1/deep.txt"), "new deep");
        assert_eq!(read("mine.txt"), "mine");
        assert_eq!(read("theirs.txt"), "theirs");
        assert!(src.join("same.txt").exists(), "a copy keeps its source");

        fs::write(dest.join("Reports/same.txt"), "old").unwrap();
        assert_eq!(put(DropOp::Move).unwrap(), "Reports");
        assert_eq!(read("same.txt"), "new");
        assert_eq!(read("theirs.txt"), "theirs");
        assert!(!src.exists(), "a moved folder is removed once merged");
    }

    /// A file onto a folder, or a folder onto a file, never replaces it,
    /// even when told to: it lands numbered and the other stays.
    #[test]
    fn put_in_with_replace_numbers_a_file_onto_a_folder() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        fs::create_dir_all(a.join("Reports")).unwrap();
        fs::create_dir_all(b.join("x.txt")).unwrap();
        fs::write(a.join("x.txt"), "file").unwrap();
        fs::write(b.join("Reports"), "file").unwrap();
        let sc = |_: &str| None;
        let put = |name: &str, op| {
            put_in_with(
                &a.join(name),
                name,
                false,
                &b,
                op,
                &[],
                &sc,
                Seen::of(&b.join(name)),
            )
        };
        assert_eq!(put("x.txt", DropOp::Copy).unwrap(), "x (2).txt");
        assert_eq!(put("Reports", DropOp::Move).unwrap(), "Reports (2)");
        assert!(b.join("x.txt").is_dir() && b.join("Reports").is_file());
        assert!(b.join("Reports (2)").is_dir() && !a.join("Reports").exists());
    }

    /// A merge that would put a file onto a folder, or a link onto a
    /// folder, anywhere in the tree is refused before anything changes.
    #[test]
    fn a_merge_with_a_kind_mismatch_inside_changes_nothing() {
        let t = tempfile::tempdir().unwrap();
        let (src, dest) = (t.path().join("src/Reports"), t.path().join("dest"));
        fs::create_dir_all(src.join("q1")).unwrap();
        fs::create_dir_all(dest.join("Reports/q1/deep")).unwrap();
        fs::write(src.join("same.txt"), "new").unwrap();
        fs::write(dest.join("Reports/same.txt"), "old").unwrap();
        fs::write(src.join("q1/deep"), "a file onto a folder").unwrap();
        let sc = |_: &str| None;
        for op in [DropOp::Copy, DropOp::Move] {
            let err = put_in_with(
                &src,
                "Reports",
                false,
                &dest,
                op,
                &[],
                &sc,
                Seen::of(&dest.join("Reports")),
            )
            .unwrap_err();
            assert!(matches!(err, PutError::Occupied), "{op:?}");
            assert_eq!(
                fs::read_to_string(dest.join("Reports/same.txt")).unwrap(),
                "old"
            );
            assert!(src.join("same.txt").exists());
        }
        fs::remove_file(src.join("q1/deep")).unwrap();
        std::os::unix::fs::symlink(t.path(), src.join("q1/deep")).unwrap();
        let err = put_in_with(
            &src,
            "Reports",
            false,
            &dest,
            DropOp::Copy,
            &[],
            &sc,
            Seen::of(&dest.join("Reports")),
        )
        .unwrap_err();
        assert!(matches!(err, PutError::Occupied), "a link onto a folder");
    }

    /// A merge deeper than the copy's depth limit is refused before it
    /// changes anything.
    #[test]
    fn a_merge_past_the_depth_limit_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let deep = "d/".repeat(MAX_DEPTH + 1);
        let (src, dest) = (t.path().join("src/R"), t.path().join("dest"));
        fs::create_dir_all(src.join(&deep)).unwrap();
        fs::create_dir_all(dest.join("R").join(&deep)).unwrap();
        fs::write(src.join("top.txt"), "new").unwrap();
        let sc = |_: &str| None;
        let err = put_in_with(
            &src,
            "R",
            false,
            &dest,
            DropOp::Copy,
            &[],
            &sc,
            Seen::of(&dest.join("R")),
        )
        .unwrap_err();
        assert!(matches!(&err, PutError::Io(e) if e.kind() == io::ErrorKind::InvalidInput));
        assert!(!dest.join("R/top.txt").exists());
    }

    /// A link inside a merged folder that points at the file it would
    /// replace is refused before anything changes: replacing would leave
    /// a link to itself and lose the file.
    #[test]
    fn a_merge_never_replaces_a_file_with_a_link_to_it() {
        let t = tempfile::tempdir().unwrap();
        let (src, dest) = (t.path().join("src/R"), t.path().join("dest"));
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(dest.join("R")).unwrap();
        fs::write(dest.join("R/x.txt"), "keep").unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        std::os::unix::fs::symlink(dest.join("R/x.txt"), src.join("x.txt")).unwrap();
        let sc = |_: &str| None;
        for op in [DropOp::Copy, DropOp::Move] {
            let seen = Seen::of(&dest.join("R"));
            let err = put_in_with(&src, "R", false, &dest, op, &[], &sc, seen).unwrap_err();
            assert!(matches!(err, PutError::SameFile), "{op:?}");
            assert_eq!(fs::read_to_string(dest.join("R/x.txt")).unwrap(), "keep");
            assert!(!dest.join("R/a.txt").exists(), "nothing was merged");
        }
    }

    /// A Yes replaces only what the prompt saw: a file put at the name
    /// since (by another batch) stays, and the put lands numbered.
    #[test]
    fn a_stale_answer_numbers_instead_of_replacing() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("x.txt"), "mine").unwrap();
        fs::write(b.join("x.txt"), "asked about").unwrap();
        let seen = Seen::of(&b.join("x.txt"));
        fs::remove_file(b.join("x.txt")).unwrap();
        fs::write(b.join("x.txt"), "another batch's").unwrap();
        let sc = |_: &str| None;
        for (op, want) in [(DropOp::Copy, "x (2).txt"), (DropOp::Move, "x (3).txt")] {
            let Landing { name, replaced, .. } = put_in_until(
                &a.join("x.txt"),
                "x.txt",
                false,
                &b,
                op,
                &[],
                &sc,
                seen,
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!((name.to_str().unwrap(), replaced), (want, false));
        }
        assert_eq!(
            fs::read_to_string(b.join("x.txt")).unwrap(),
            "another batch's"
        );
    }

    /// A folder under the source that cannot be read refuses the merge
    /// before anything changes, rather than stopping it halfway.
    #[test]
    fn a_merge_with_an_unreadable_folder_changes_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let (src, dest) = (t.path().join("src/R"), t.path().join("dest"));
        fs::create_dir_all(src.join("locked")).unwrap();
        fs::create_dir_all(dest.join("R")).unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        fs::write(src.join("locked/b.txt"), "b").unwrap();
        fs::set_permissions(src.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read_dir(src.join("locked")).is_ok() {
            // Running as root: nothing is unreadable.
            return;
        }
        let sc = |_: &str| None;
        let seen = Seen::of(&dest.join("R"));
        let err = put_in_with(&src, "R", false, &dest, DropOp::Copy, &[], &sc, seen);
        fs::set_permissions(src.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(err, Err(PutError::Io(_))));
        assert!(!dest.join("R/a.txt").exists(), "nothing was merged");
    }

    /// A dangling link where the temporary file or a numbered name would
    /// go is passed over, not retried until the put fails.
    #[test]
    fn dangling_links_at_the_next_names_are_passed_over() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("x.txt"), "new").unwrap();
        fs::write(b.join("x.txt"), "old").unwrap();
        std::os::unix::fs::symlink(t.path().join("nowhere"), b.join(".reprise.part")).unwrap();
        std::os::unix::fs::symlink(t.path().join("nowhere"), b.join("x (2).txt")).unwrap();
        let sc = |_: &str| None;
        let seen = Seen::of(&b.join("x.txt"));
        put_in_with(
            &a.join("x.txt"),
            "x.txt",
            false,
            &b,
            DropOp::Copy,
            &[],
            &sc,
            seen,
        )
        .unwrap();
        assert_eq!(fs::read_to_string(b.join("x.txt")).unwrap(), "new");
        let name = put_in(&a.join("x.txt"), "x.txt", false, &b, DropOp::Move, &[], &sc).unwrap();
        assert_eq!(name, "x (3).txt");
    }

    /// A merge moving to another drive copies, and a copy cannot carry a
    /// FIFO: the preflight refuses it, even deep in a folder the other
    /// side does not have, before anything moves. On the same drive the
    /// move renames it like anything else.
    #[test]
    fn a_move_to_another_drive_refuses_a_fifo_up_front() {
        let t = tempfile::tempdir().unwrap();
        let (src, dest) = (t.path().join("src"), t.path().join("dest"));
        fs::create_dir_all(src.join("only/here")).unwrap();
        fs::create_dir_all(&dest).unwrap();
        mkfifo(&src.join("only/here/pipe"));
        let err = preflight_merge(&src, Some(&dest), true, true, 0).unwrap_err();
        assert!(matches!(&err, PutError::Io(e) if e.kind() == io::ErrorKind::InvalidInput));
        assert!(preflight_merge(&src, Some(&dest), true, false, 0).is_ok());
    }

    /// A file that took the name after the last check (another job's) is
    /// swapped back out and kept: the put lands numbered beside it, for a
    /// copy and for a move, and no temporary file is left.
    #[test]
    fn a_changed_target_is_swapped_back_and_numbered() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("x.txt"), "new").unwrap();
        fs::write(b.join("x.txt"), "another job's").unwrap();
        let asked_about = (0, 0);
        let from = fs::metadata(a.join("x.txt")).unwrap();
        for (moving, want) in [(false, "x (2).txt"), (true, "x (3).txt")] {
            match replace_file(
                &a.join("x.txt"),
                &b.join("x.txt"),
                &from,
                moving,
                asked_about,
            ) {
                Ok(Swap::Numbered(name)) => assert_eq!(name, want),
                Ok(_) => panic!("expected to land numbered beside another job's file"),
                Err(e) => panic!("{e}"),
            }
            assert_eq!(
                fs::read_to_string(b.join("x.txt")).unwrap(),
                "another job's"
            );
            assert_eq!(fs::read_to_string(b.join(want)).unwrap(), "new");
        }
        assert!(!a.join("x.txt").exists(), "the move took its source");
        let names: Vec<_> = fs::read_dir(&b)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 3, "no temporary file left: {names:?}");
    }

    /// A swap that finds another job's file and cannot swap it back
    /// keeps both: the new file holds the name, the other is moved aside,
    /// and no temporary file is left.
    #[test]
    fn a_failed_swap_back_keeps_both_files() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("x.txt"), "new").unwrap();
        fs::write(b.join("x.txt"), "another job's").unwrap();
        let from = fs::metadata(a.join("x.txt")).unwrap();
        FAIL_SWAP_BACK.with(|f| f.set(true));
        let swap = replace_file(&a.join("x.txt"), &b.join("x.txt"), &from, false, (0, 0));
        FAIL_SWAP_BACK.with(|f| f.set(false));
        match swap {
            Ok(Swap::Displaced(aside)) => {
                assert_eq!(aside, "x (2).txt");
                assert_eq!(fs::read_to_string(b.join(&aside)).unwrap(), "another job's");
            }
            Ok(_) => panic!("expected the other file moved aside"),
            Err(e) => panic!("{e}"),
        }
        assert_eq!(fs::read_to_string(b.join("x.txt")).unwrap(), "new");
        assert_eq!(
            fs::read_dir(&b).unwrap().count(),
            2,
            "no temporary file left"
        );
    }

    /// The file asked about is swapped out and removed, the new one in its
    /// place.
    #[test]
    fn the_target_asked_about_is_swapped_out() {
        use std::os::unix::fs::MetadataExt;
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("x.txt"), "new").unwrap();
        fs::write(b.join("x.txt"), "old").unwrap();
        let m = fs::symlink_metadata(b.join("x.txt")).unwrap();
        let from = fs::metadata(a.join("x.txt")).unwrap();
        let swap = replace_file(
            &a.join("x.txt"),
            &b.join("x.txt"),
            &from,
            false,
            (m.dev(), m.ino()),
        );
        assert!(matches!(swap, Ok(Swap::Replaced)));
        assert_eq!(fs::read_to_string(b.join("x.txt")).unwrap(), "new");
        assert_eq!(fs::read_dir(&b).unwrap().count(), 1);
    }

    /// A move to another drive whose original sits in a read-only folder
    /// is refused before the copy: the original could not be removed.
    #[test]
    fn a_stuck_original_refuses_the_move_to_another_drive() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let (src, dest) = (t.path().join("src"), t.path().join("dest"));
        fs::create_dir_all(src.join("R/locked")).unwrap();
        fs::create_dir_all(dest.join("R")).unwrap();
        fs::write(src.join("R/locked/a.txt"), "a").unwrap();
        fs::set_permissions(src.join("R/locked"), fs::Permissions::from_mode(0o555)).unwrap();
        let writable = can_write(&src.join("R/locked"));
        let merge = preflight_merge(&src.join("R"), Some(&dest.join("R")), true, true, 0);
        let file = refuse_stuck_original(&src.join("R/locked/a.txt"));
        fs::set_permissions(src.join("R/locked"), fs::Permissions::from_mode(0o755)).unwrap();
        if writable {
            // Running as root: nothing is read-only.
            return;
        }
        assert!(
            matches!(&merge, Err(PutError::Io(e)) if e.kind() == io::ErrorKind::PermissionDenied)
        );
        assert!(
            matches!(&file, Err(PutError::Io(e)) if e.kind() == io::ErrorKind::PermissionDenied)
        );
        assert!(refuse_stuck_original(&src.join("R")).is_ok());
    }

    /// A numbered copy passes over a name a dangling link holds.
    #[test]
    fn a_numbered_copy_passes_over_a_dangling_link() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("x.txt"), "new").unwrap();
        fs::write(b.join("x.txt"), "old").unwrap();
        std::os::unix::fs::symlink(t.path().join("nowhere"), b.join("x (2).txt")).unwrap();
        let sc = |_: &str| None;
        let name = put_in(&a.join("x.txt"), "x.txt", false, &b, DropOp::Copy, &[], &sc).unwrap();
        assert_eq!(name, "x (3).txt");
    }

    /// A copied merge skips a FIFO as a plain folder copy does.
    #[test]
    fn a_copied_merge_skips_a_fifo() {
        let t = tempfile::tempdir().unwrap();
        let (src, dest) = (t.path().join("src/Reports"), t.path().join("dest"));
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(dest.join("Reports")).unwrap();
        mkfifo(&src.join("pipe"));
        fs::write(src.join("a.txt"), "a").unwrap();
        let sc = |_: &str| None;
        put_in_with(
            &src,
            "Reports",
            false,
            &dest,
            DropOp::Copy,
            &[],
            &sc,
            Seen::of(&dest.join("Reports")),
        )
        .unwrap();
        assert!(dest.join("Reports/a.txt").exists());
        assert!(fs::symlink_metadata(dest.join("Reports/pipe")).is_err());
    }

    /// A link as the source of a copy is followed when it replaces, as it
    /// is when it lands on a free name: the file it points at is copied.
    #[test]
    fn a_copied_link_replaces_with_what_it_points_at() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(t.path().join("target.txt"), "target").unwrap();
        std::os::unix::fs::symlink(t.path().join("target.txt"), a.join("x.txt")).unwrap();
        fs::write(b.join("x.txt"), "old").unwrap();
        let sc = |_: &str| None;
        put_in_with(
            &a.join("x.txt"),
            "x.txt",
            false,
            &b,
            DropOp::Copy,
            &[],
            &sc,
            Seen::of(&b.join("x.txt")),
        )
        .unwrap();
        let meta = fs::symlink_metadata(b.join("x.txt")).unwrap();
        assert!(meta.is_file(), "a file, not a link");
        assert_eq!(fs::read_to_string(b.join("x.txt")).unwrap(), "target");
    }

    /// Replacing a file with another link to the same file is refused,
    /// and the source stays where it is.
    #[test]
    fn replacing_a_hard_link_to_itself_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("x.txt"), "one").unwrap();
        fs::hard_link(a.join("x.txt"), b.join("x.txt")).unwrap();
        let sc = |_: &str| None;
        let err = put_in_with(
            &a.join("x.txt"),
            "x.txt",
            false,
            &b,
            DropOp::Move,
            &[],
            &sc,
            Seen::of(&b.join("x.txt")),
        )
        .unwrap_err();
        assert!(matches!(err, PutError::SameFile));
        assert_eq!(
            err.to_string(),
            "The source and destination are the same file."
        );
        assert!(a.join("x.txt").exists() && b.join("x.txt").exists());
    }

    /// The temporary file beside the one being replaced has a short name
    /// of its own, so a name near the file system's limit still works.
    #[test]
    fn a_long_name_is_replaced_without_running_past_the_limit() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let name = format!("{}.txt", "n".repeat(250));
        fs::write(a.join(&name), "new").unwrap();
        fs::write(b.join(&name), "old").unwrap();
        let sc = |_: &str| None;
        put_in_with(
            &a.join(&name),
            &name,
            false,
            &b,
            DropOp::Copy,
            &[],
            &sc,
            Seen::of(&b.join(&name)),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(b.join(&name)).unwrap(), "new");
        assert_eq!(fs::read_dir(&b).unwrap().count(), 1);
    }

    /// A copy into the folder it is in, reached through a link or `..`,
    /// is still a copy beside itself: "Copy of x".
    #[test]
    fn a_copy_into_its_own_folder_through_a_link_is_a_copy_of() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a");
        fs::create_dir_all(&a).unwrap();
        fs::write(a.join("x.txt"), "x").unwrap();
        std::os::unix::fs::symlink(&a, t.path().join("via")).unwrap();
        let sc = |_: &str| None;
        let name = put_in_with(
            &a.join("x.txt"),
            "x.txt",
            false,
            &t.path().join("via"),
            DropOp::Copy,
            &["x.txt".into()],
            &sc,
            Seen::of(&a.join("x.txt")),
        )
        .unwrap();
        assert_eq!(name, "Copy of x.txt");
        assert_eq!(fs::read_to_string(a.join("x.txt")).unwrap(), "x");
    }

    /// A copy never lands on a file that appeared after the name was
    /// picked: the open itself refuses.
    #[test]
    fn copy_new_refuses_an_existing_destination() {
        let t = tempfile::tempdir().unwrap();
        let from = t.path().join("a");
        let to = t.path().join("b");
        fs::write(&from, "new").unwrap();
        fs::write(&to, "old").unwrap();
        let e = copy_new(&from, &to).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&to).unwrap(), "old");
        fs::remove_file(&to).unwrap();
        copy_new(&from, &to).unwrap();
        assert_eq!(fs::read_to_string(&to).unwrap(), "new");
    }

    /// A read that fails part way leaves no partial destination.
    /// `/proc/self/mem` is a regular file whose read at offset 0 is `EIO`.
    #[test]
    fn copy_new_removes_the_destination_when_the_read_fails() {
        let t = tempfile::tempdir().unwrap();
        let to = t.path().join("b");
        assert!(copy_new(Path::new("/proc/self/mem"), &to).is_err());
        assert!(!to.exists(), "partial copy left behind");
    }

    /// A FIFO handed over directly is refused at once instead of blocking
    /// the open, and nothing lands in the destination folder.
    #[test]
    fn put_in_refuses_a_fifo_as_the_source() {
        let t = tempfile::tempdir().unwrap();
        let pipe = t.path().join("pipe");
        mkfifo(&pipe);
        let dest = t.path().join("dest");
        fs::create_dir(&dest).unwrap();
        let sc = |_: &str| None;

        let e = put_in(&pipe, "pipe", false, &dest, DropOp::Copy, &[], &sc).unwrap_err();

        assert!(matches!(e, PutError::Io(e) if e.kind() == io::ErrorKind::InvalidInput));
        assert!(fs::read_dir(&dest).unwrap().next().is_none());
    }

    /// A symlink is copied as a symlink, so a link pointing back up the
    /// tree is one entry rather than a walk that never ends.
    #[test]
    fn copy_dir_all_copies_a_link_as_a_link_instead_of_following_it() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("Reports");
        fs::create_dir_all(src.join("2005")).unwrap();
        fs::write(src.join("x.txt"), "1").unwrap();
        std::os::unix::fs::symlink("..", src.join("loop")).unwrap();
        let dest = t.path().join("Copy");

        copy_dir_all(&src, &dest).unwrap();

        let link = dest.join("loop");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_link(&link).unwrap(), Path::new(".."));
        assert!(dest.join("2005").is_dir());
        assert_eq!(fs::read_to_string(dest.join("x.txt")).unwrap(), "1");
    }

    /// A copy that fails leaves no half-folder behind, and a FIFO in the
    /// tree is skipped rather than opened.
    #[test]
    fn a_failed_copy_removes_what_it_made() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("Reports");
        fs::create_dir_all(src.join("open")).unwrap();
        fs::write(src.join("open/a.txt"), "1").unwrap();
        mkfifo(&src.join("open/pipe"));
        fs::create_dir(src.join("shut")).unwrap();
        fs::write(src.join("shut/b.txt"), "2").unwrap();
        fs::set_permissions(src.join("shut"), fs::Permissions::from_mode(0o000)).unwrap();
        let dest = t.path().join("Copy");

        let r = copy_dir_all(&src, &dest);
        fs::set_permissions(src.join("shut"), fs::Permissions::from_mode(0o755)).unwrap();
        // SAFETY: geteuid takes no arguments and cannot fail.
        if unsafe { libc::geteuid() } == 0 {
            // root reads anything; only the FIFO part is checkable here.
            r.unwrap();
            assert!(!dest.join("open/pipe").exists());
            return;
        }
        assert!(r.is_err());
        assert!(!dest.exists(), "partial copy left behind");
    }

    #[test]
    fn can_write_follows_the_folders_write_and_search_bits() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        assert!(can_write(t.path()), "a fresh temp dir is writable");
        fs::set_permissions(t.path(), fs::Permissions::from_mode(0o555)).unwrap();
        let shut = can_write(t.path());
        fs::set_permissions(t.path(), fs::Permissions::from_mode(0o755)).unwrap();
        // SAFETY: geteuid takes no arguments and cannot fail.
        if unsafe { libc::geteuid() } == 0 {
            // root writes anywhere.
            return;
        }
        assert!(!shut, "a 0555 folder is not writable");
    }

    #[test]
    fn a_stopped_copy_fails_interrupted_and_removes_what_it_made() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("Reports");
        fs::create_dir_all(src.join("2005")).unwrap();
        fs::write(src.join("2005/a.txt"), "1").unwrap();
        let dest = t.path().join("Copy");
        let e = copy_dir_until(&src, &dest, &AtomicBool::new(true)).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Interrupted);
        assert!(!dest.exists(), "no partial copy left behind");
        assert!(src.join("2005/a.txt").exists(), "the source is untouched");
    }

    /// A destination that already exists is refused and left as it was.
    #[test]
    fn copy_dir_all_leaves_an_existing_destination_alone() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("Reports");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("new.txt"), "new").unwrap();
        let dest = t.path().join("Taken");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("old.txt"), "old").unwrap();

        let e = copy_dir_all(&src, &dest).unwrap_err();

        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(dest.join("old.txt")).unwrap(), "old");
        assert!(!dest.join("new.txt").exists());
    }

    /// A name that looks like an option or an archive to include is an
    /// operand: bsdtar reads `-C` and `@` even past the double dash.
    #[test]
    fn zip_argv_takes_a_name_like_an_option_as_a_name() {
        let z = zip_argv(
            &[
                "/d/-x".into(),
                "/d/-Cy".into(),
                "/d/@z".into(),
                "/d/a-b".into(),
            ],
            Path::new("/d/-x.zip"),
        );
        assert_eq!(z[6..], ["--", "./-x", "./-Cy", "./@z", "a-b"]);
    }

    #[test]
    fn zip_argv_puts_every_item_into_one_archive() {
        let z = zip_argv(&["/d/a.txt".into(), "/d/b".into()], Path::new("/d/a.zip"));
        assert_eq!(
            z,
            [
                "bsdtar", "-a", "-cf", "/d/a.zip", "-C", "/d", "--", "a.txt", "b"
            ]
        );
    }

    #[test]
    fn mount_root_is_a_device_change_from_the_parent() {
        assert!(
            mount_root(0x803, 0x801),
            "a directory on another device is a mount root"
        );
        assert!(
            !mount_root(0x801, 0x801),
            "a directory on its parent's device is not"
        );
    }

    #[test]
    fn is_mount_point_takes_the_root_and_not_a_plain_folder_or_a_link() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("Plain");
        fs::create_dir(&dir).unwrap();
        let link = t.path().join("Root link");
        std::os::unix::fs::symlink("/", &link).unwrap();

        assert!(is_mount_point(Path::new("/")), "/ is a mount point");
        assert!(!is_mount_point(&dir), "a plain folder is not");
        assert!(!is_mount_point(&link), "a link to / is not");
        assert!(
            !is_mount_point(&t.path().join("gone")),
            "a missing path is not"
        );
        assert!(refuse_mount_point(&dir).is_ok());
        assert_eq!(
            refuse_mount_point(Path::new("/")).unwrap_err().kind(),
            io::ErrorKind::ResourceBusy
        );
    }

    /// `Mounted` stands in for a mount: the lookup puts it and what is
    /// under it on device 2, the rest of the tree on 1. The link to it is
    /// not followed, so a tree holding only the link passes.
    #[test]
    fn mount_under_finds_a_folder_on_another_device_and_skips_links() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Stuff");
        let mounted = root.join("Docs").join("Mounted");
        fs::create_dir_all(mounted.join("deep")).unwrap();
        fs::write(root.join("a.txt"), "a").unwrap();
        let linked = t.path().join("Linked");
        fs::create_dir(&linked).unwrap();
        std::os::unix::fs::symlink(&mounted, linked.join("to mount")).unwrap();
        let dev_of = |p: &Path| Ok(if p.starts_with(&mounted) { 2 } else { 1 });

        assert_eq!(mount_under(&root, &dev_of).unwrap(), Some(mounted.clone()));
        assert_eq!(mount_under(&linked, &dev_of).unwrap(), None);
        assert_eq!(mount_under(&root.join("a.txt"), &dev_of).unwrap(), None);
        let one = |_: &Path| Ok(1);
        assert_eq!(mount_under(&root, &one).unwrap(), None);
        assert!(
            refuse_holding_mount(&root).is_ok(),
            "a real tree holds no mount"
        );
    }

    #[test]
    fn erase_removes_a_plain_folder_and_only_the_link_of_a_link() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("Folder");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("sub/a.txt"), "a").unwrap();
        let link = t.path().join("Link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();

        erase(&link).unwrap();
        assert!(!link.exists() && dir.join("sub/a.txt").exists());
        erase(&dir).unwrap();
        assert!(!dir.exists());
    }

    #[test]
    fn trash_in_keeps_a_record_and_takes_a_gone_path_as_done() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        let file = t.path().join("notes.txt");
        fs::write(&file, "n").unwrap();

        trash_in(&root, &file).unwrap();

        assert!(!file.exists(), "the file left its folder");
        assert_eq!(
            fs::read_to_string(root.join("files/notes.txt")).unwrap(),
            "n"
        );
        let info = fs::read_to_string(root.join("info/notes.txt.trashinfo")).unwrap();
        assert_eq!(
            crate::trash::original_path(&info, None).as_deref(),
            Some(file.as_path())
        );
        trash_in(&root, &file).expect("a gone path is already deleted");
    }

    /// A name that is not UTF-8 is erased by its own bytes; the name it
    /// would read as with U+FFFD in place of the byte stays.
    #[test]
    fn delete_erases_a_name_that_is_not_utf8() {
        use std::os::unix::ffi::OsStrExt;
        let t = tempfile::tempdir().unwrap();
        let raw = t.path().join(std::ffi::OsStr::from_bytes(b"a\xffb.txt"));
        let lossy = t.path().join("a\u{fffd}b.txt");
        fs::write(&raw, "raw").unwrap();
        fs::write(&lossy, "lossy").unwrap();

        delete(&raw, crate::bincap::Plan::Erase).unwrap();

        assert!(!raw.exists(), "the named file is gone");
        assert_eq!(fs::read_to_string(&lossy).unwrap(), "lossy");
    }

    #[test]
    fn modifier_op_takes_ctrl_shift_and_alt() {
        assert_eq!(modifier_op(false, false, false), None);
        assert_eq!(modifier_op(true, false, false), Some(DropOp::Copy));
        assert_eq!(modifier_op(false, true, false), Some(DropOp::Move));
        assert_eq!(modifier_op(false, false, true), Some(DropOp::Shortcut));
        assert_eq!(modifier_op(true, true, false), Some(DropOp::Shortcut));
        assert_eq!(modifier_op(true, false, true), Some(DropOp::Shortcut));
    }
}
