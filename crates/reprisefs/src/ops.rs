//! File operations, and the argument vectors and shell quoting around them.

mod copy;
mod erase;
mod fileio;
mod mounts;
mod put;

pub use copy::copy_dir_all;
pub(crate) use copy::{copy_dir_until, copy_new};
pub use erase::erase;
pub(crate) use erase::{erase_counted, erase_stepped};
pub(crate) use fileio::read_opened;
pub use fileio::{copy_xattrs, create_temp_beside, read_regular, write_atomic};
pub use mounts::{is_mount_point, mount_root, refuse_holding_mount, refuse_mount_point};
pub(crate) use put::{Landing, merge_clashes, put_in_until, sync_landed};
pub use put::{PutError, put_in, put_in_with, same_device, same_folder};

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};

/// `s` in single quotes with embedded quotes escaped, safe for `sh -c`.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// How deep [`copy_dir_all`] recurses before giving up: a backstop for a tree
/// that grows under the walk.
const MAX_DEPTH: usize = 256;

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
        let what = match self {
            DropOp::Move => format!("move {name}"),
            DropOp::Copy => format!("copy {name}"),
            DropOp::Shortcut => format!("create a shortcut to {name}"),
        };
        (self.failure_title(), format!("Cannot {what}: {error}"))
    }

    /// The title of this op's error box.
    pub fn failure_title(self) -> &'static str {
        match self {
            DropOp::Move => "Error Moving File or Folder",
            DropOp::Copy => "Error Copying File or Folder",
            DropOp::Shortcut => "Error Creating Shortcut",
        }
    }
}

#[cfg(test)]
mod tests;
