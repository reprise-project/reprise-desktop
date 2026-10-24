use super::{
    DropOp, MAX_DEPTH, can_write, copy_dir_until, copy_new, inside, refuse_holding_mount,
    refuse_mount_point, resolved, unique_path,
};
use crate::replace::{Replacing, Seen};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

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
    /// The job made no headway for [`crate::deadline::ACTION_DEADLINE`]: a call on a drive that
    /// stopped answering.
    NotResponding,
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
            PutError::NotResponding => f.write_str("The location is not responding."),
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
pub(super) enum Swap {
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
    pub(super) static FAIL_SWAP_BACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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
fn sync_tree(path: &Path) -> io::Result<()> {
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
/// Under a job it runs outside the job's watch: a slow disk takes as long as it takes.
pub(crate) fn sync_landed(path: &Path) -> io::Result<()> {
    crate::progress::flushing(|| {
        sync_tree(path)?;
        let dir = path.parent().unwrap_or(Path::new("."));
        std::fs::File::open(dir)?.sync_all()
    })
}

/// Refuse a move to another drive whose original cannot be removed once
/// it is copied: the folder holding it must be writable.
pub(super) fn refuse_stuck_original(src: &Path) -> Result<(), PutError> {
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
/// lands numbered beside it ([`swap_in`]). A merge puts a file over one
/// inside only as `files` lists it ([`Replacing::files`]).
fn replace_into(
    src: &Path,
    dest: &Path,
    moving: bool,
    expect: (u64, u64),
    files: &BTreeMap<PathBuf, Seen>,
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
            merge_into(src, dest, moving, (files, Path::new("")), stop)?;
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
/// [`copy_dir_all`](super::copy_dir_all) does; a move renames it like anything else.
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
pub(super) fn preflight_merge(
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

/// The files a merge of folder `src` into folder `dest` would put over a file, by path under
/// `src`, each as it is now: what the prompt asks about after the folder's Yes. A tree it cannot
/// read is cut short there, as the merge refuses it anyway ([`preflight_merge`]).
pub(crate) fn merge_clashes(src: &Path, dest: &Path, moving: bool) -> Vec<(PathBuf, Seen)> {
    fn walk(src: &Path, dest: &Path, rel: &Path, moving: bool, out: &mut Vec<(PathBuf, Seen)>) {
        if rel.components().count() >= MAX_DEPTH {
            return;
        }
        let Ok(rd) = std::fs::read_dir(src) else {
            return;
        };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            let kind = meta.file_type();
            if !merges(kind, moving) {
                continue;
            }
            let (to, rel) = (dest.join(entry.file_name()), rel.join(entry.file_name()));
            let Ok(there) = std::fs::symlink_metadata(&to) else {
                continue;
            };
            match (kind.is_dir(), there.is_dir()) {
                (true, true) => walk(&entry.path(), &to, &rel, moving, out),
                (false, false) => out.extend(Seen::of(&to).map(|seen| (rel, seen))),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(src, dest, Path::new(""), moving, &mut out);
    out
}

/// Merge the entries of folder `src` into folder `dest`, `at` (`files`, the path under the top)
/// telling which files inside are put over the file of the same name. Links are merged as links.
/// A file not in `files`, or changed since the prompt, stays, and the one coming in stays where it
/// was; a moved folder holding one is left behind.
fn merge_into(
    src: &Path,
    dest: &Path,
    moving: bool,
    at: (&BTreeMap<PathBuf, Seen>, &Path),
    stop: &AtomicBool,
) -> Result<(), PutError> {
    let (files, rel) = at;
    if rel.components().count() >= MAX_DEPTH {
        return Err(too_deep().into());
    }
    let mut kept = false;
    let entries = std::fs::read_dir(src)?.collect::<io::Result<Vec<_>>>()?;
    for entry in entries {
        if stop.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let (from, to) = (entry.path(), dest.join(entry.file_name()));
        let under = rel.join(entry.file_name());
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
                merge_into(&from, &to, moving, (files, &under), stop)?;
                kept |= moving && from.exists();
            }
            Ok(there) if !kind.is_dir() && !there.is_dir() => {
                let Some(seen) = files.get(&under).filter(|seen| seen.still(&to)) else {
                    log::info!("reprisefs: merge keeps {}", to.display());
                    kept = true;
                    continue;
                };
                if let Swap::Numbered(name) | Swap::Displaced(name) =
                    replace_file(&from, &to, &meta, moving, seen.id())?
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
    // Everything not kept is in `dest` by now; a folder that will not go (a
    // file appeared in it meanwhile) is left behind rather than failing a
    // merge that happened.
    if moving
        && !kept
        && let Err(e) = std::fs::remove_dir(src)
    {
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
pub(super) fn replace_file(
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
    replace: Option<&Replacing>,
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
    replace: Option<&Replacing>,
    stop: &AtomicBool,
) -> Result<Landing, PutError> {
    // Only what the prompt saw, and only by its own kind.
    let over = |dest: &Path, moving: bool| {
        replace.filter(|r| r.seen.still(dest) && same_kind(src, dest, moving))
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
            let landed = if let Some(r) = over(&dest, true) {
                let swap = replace_into(src, &dest, true, r.seen.id(), &r.files, stop)?;
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
                && let Some(r) = over(&dir.join(&name), false)
            {
                let dest = dir.join(&name);
                let swap = replace_into(src, &dest, false, r.seen.id(), &r.files, stop)?;
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
