//! File operations, and the argument vectors and shell quoting around them.

pub(crate) mod at;
mod copy;
mod erase;
mod fileio;
mod mounts;
mod parts;
mod put;

pub use copy::copy_dir_all;
pub(crate) use copy::{build_dir, copy_dir_until, copy_new, in_tree, remove_tree};
pub use erase::erase;
pub(crate) use erase::{entries_at, erase_at, erase_counted, erase_stepped, unlink_at};
pub use fileio::{
    copy_xattrs, create_temp_beside, is_read_only, locked_edit, open_regular, read_regular,
    real_path, write_atomic, write_document, write_private,
};
pub(crate) use fileio::{on_a_drive, read_held, read_opened};
pub use mounts::{is_mount_point, mount_root, refuse_holding_mount, refuse_mount_point};
#[cfg(test)]
pub(crate) use put::IN_FLUSH;
pub(crate) use put::{
    Landing, go_on, land_part, merge_clashes, put_in_until, rename2, sync_landed, unsupported,
};
pub use put::{PutError, put_in, put_in_with, same_device, same_folder};

use reprisekit::tr;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};

/// `s` in single quotes with embedded quotes escaped, safe for `sh -c`.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Copy as Path's text: one path per line, each in double quotes when a shell would split or
/// expand it unquoted, with `"`, `\\`, `$` and `` ` `` escaped inside the quotes. A name that is not
/// UTF-8 is copied lossily.
pub fn path_lines<'a>(paths: impl IntoIterator<Item = &'a Path>) -> String {
    let lines: Vec<String> = paths
        .into_iter()
        .map(|p| {
            let s = p.to_string_lossy();
            if s.chars().all(shell_plain) {
                return s.into_owned();
            }
            let mut q = String::with_capacity(s.len() + 2);
            q.push('"');
            for c in s.chars() {
                if matches!(c, '"' | '\\' | '$' | '`') {
                    q.push('\\');
                }
                q.push(c);
            }
            q.push('"');
            q
        })
        .collect();
    lines.join("\n")
}

/// A character a shell reads as itself anywhere in a word: ASCII letters, digits and
/// `/ . _ - + , : @ % =`, and non-ASCII characters that are not whitespace.
fn shell_plain(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(c, '/' | '.' | '_' | '-' | '+' | ',' | ':' | '@' | '%' | '=')
        || (!c.is_ascii() && !c.is_whitespace())
}

/// How deep [`copy_dir_all`] recurses before giving up: a backstop for a tree
/// that grows under the walk.
const MAX_DEPTH: usize = 256;

/// `path` with `.` and `..` resolved: through the file system when it is
/// there (which also resolves symlinks), lexically when it is not.
fn resolved(path: &Path) -> PathBuf {
    #[cfg(test)]
    crate::testfs::dead::stall(path);
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
/// out of room, so [`put_in`] refuses it. A move with `moving` takes a link
/// as the link: only its folder is resolved, so its target is never looked at.
pub fn inside(src: &Path, dir: &Path, moving: bool) -> bool {
    let src = match (src.parent(), src.file_name()) {
        (Some(parent), Some(name)) if moving => {
            let parent = if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            };
            resolved(parent).join(name)
        }
        _ => resolved(src),
    };
    resolved(dir).starts_with(&src)
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

/// Rename `from` to the first free name of [`unique_path`]'s numbering of `to`, never over a file
/// that took a name meanwhile.
pub fn rename_numbered(from: &Path, to: &Path) -> io::Result<PathBuf> {
    let dir = to.parent().unwrap_or(Path::new("."));
    let name = to.file_name().unwrap_or_default();
    for candidate in crate::names::numbered(name, crate::names::paren).take(1000) {
        let to = dir.join(candidate);
        match rename2(from, &to, libc::RENAME_NOREPLACE) {
            Ok(()) => return Ok(to),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) if unsupported(&e) => match link_over(from, &to) {
                Ok(()) => return Ok(to),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                // Neither RENAME_NOREPLACE nor hard links: the check and the rename can race.
                Err(e) if no_links(&e) => {
                    if to.symlink_metadata().is_ok() {
                        continue;
                    }
                    std::fs::rename(from, &to)?;
                    return Ok(to);
                }
                Err(e) => return Err(e),
            },
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "every numbered name is taken",
    ))
}

/// `from` linked to `to`, which link(2) refuses with EEXIST when taken, then `from` unlinked.
fn link_over(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::hard_link(from, to)?;
    if let Err(e) = std::fs::remove_file(from) {
        log::warn!(
            "file ops: {} stays beside {}: {e}",
            from.display(),
            to.display()
        );
    }
    Ok(())
}

/// Whether link(2) failed because the file system has no hard links.
fn no_links(e: &io::Error) -> bool {
    matches!(
        e.raw_os_error(),
        Some(libc::EPERM | libc::EOPNOTSUPP | libc::ENOSYS)
    )
}

/// The folder of `paths`, items of one folder, and their names as bsdtar takes them after
/// `-C folder --`: bsdtar reads `-C dir` and `@archive` among the names even past `--`, so a
/// name starting with `-` or `@` goes in as `./name`.
pub(crate) fn members(paths: &[PathBuf]) -> (&Path, Vec<OsString>) {
    let dir = paths
        .first()
        .and_then(|p| p.parent())
        .unwrap_or(Path::new("."));
    let names = paths
        .iter()
        .map(|p| {
            let name = p.file_name().unwrap_or_default();
            match name.as_encoded_bytes().first() {
                Some(b'-' | b'@') => Path::new(".").join(name).into_os_string(),
                _ => name.to_os_string(),
            }
        })
        .collect();
    (dir, names)
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
    pub fn failure(self, name: &str, error: &dyn std::fmt::Display) -> (String, String) {
        let error = error.to_string();
        let text = match self {
            DropOp::Move => tr!("move-failed", name = name, why = error.as_str()),
            DropOp::Copy => tr!("copy-failed", name = name, why = error.as_str()),
            DropOp::Shortcut => tr!("shortcut-failed", name = name, why = error.as_str()),
        };
        (self.failure_title(), text)
    }

    /// The title of this op's error box.
    pub fn failure_title(self) -> String {
        match self {
            DropOp::Move => tr!("move-failed-title"),
            DropOp::Copy => tr!("copy-failed-title"),
            DropOp::Shortcut => tr!("shortcut-failed-title"),
        }
    }

    /// The word `reprise-explorer --put` takes for this op.
    pub fn arg(self) -> &'static str {
        match self {
            DropOp::Copy => "copy",
            DropOp::Move => "move",
            DropOp::Shortcut => "link",
        }
    }

    /// The op [`DropOp::arg`] names.
    pub fn from_arg(arg: &str) -> Option<DropOp> {
        [DropOp::Copy, DropOp::Move, DropOp::Shortcut]
            .into_iter()
            .find(|op| op.arg() == arg)
    }
}

#[cfg(test)]
mod tests;
