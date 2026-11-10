//! Files a sandboxed program names, opened only where it could open them itself: inside its
//! Flatpak sandbox's root, or beneath a snap's own folders.

use crate::ops::at::{c_path, cwd, open_at};
use std::collections::HashSet;
use std::ffi::OsString;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};

/// Where a sandboxed program's named files open from. Paths are absolute, as the program
/// names them.
#[derive(Debug)]
pub enum Confined {
    /// A Flatpak sandbox's root (`/proc/<pid>/root`), held open: absolute links and `..`
    /// resolve inside it, and its `/proc` magic links are refused.
    Root(OwnedFd),
    /// Folders held open, each with the path the program names it by; a path opens only beneath
    /// the one it lies under, no link or `..` leading out.
    Beneath(Vec<(PathBuf, OwnedFd)>),
}

impl Confined {
    /// Snap instance `snap`'s own folders: `/snap/<snap>`, `~/snap/<snap>` and
    /// `$XDG_RUNTIME_DIR/snap.<snap>`, those that exist.
    pub fn snap(snap: &str) -> Confined {
        let home = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("snap"));
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
        Confined::snap_under(
            snap,
            Path::new("/snap"),
            home.as_deref(),
            runtime.as_deref(),
        )
    }

    /// [`Confined::snap`] under `mounted`, `home` and `runtime`. A parallel instance
    /// `<name>_<key>` sees its folders under `<name>` too, so a path by either name opens there.
    pub(crate) fn snap_under(
        snap: &str,
        mounted: &Path,
        home: Option<&Path>,
        runtime: Option<&Path>,
    ) -> Confined {
        let named = !snap.is_empty()
            && (snap.bytes()).all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !named {
            return Confined::Beneath(Vec::new());
        }
        let name = snap.split('_').next().unwrap_or(snap);
        let flags = libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW;
        let mut dirs = Vec::new();
        let bases = [
            Some((mounted, String::new())),
            home.map(|h| (h, String::new())),
            runtime.map(|r| (r, "snap.".to_string())),
        ];
        for (base, prefix) in bases.into_iter().flatten() {
            let own = base.join(format!("{prefix}{snap}"));
            let Some(fd) = (c_path(&own).ok()).and_then(|c| open_at(cwd(), &c, flags).ok()) else {
                continue;
            };
            if name != snap
                && let Ok(copy) = fd.try_clone()
            {
                dirs.push((base.join(format!("{prefix}{name}")), copy));
            }
            dirs.push((own, fd));
        }
        Confined::Beneath(dirs)
    }

    fn open(&self, path: &Path, flags: libc::c_int) -> io::Result<OwnedFd> {
        if !path.is_absolute() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let refused = || io::Error::from(io::ErrorKind::PermissionDenied);
        match self {
            Confined::Root(root) => {
                let resolve = libc::RESOLVE_IN_ROOT | libc::RESOLVE_NO_MAGICLINKS;
                openat2(root.as_fd(), path, flags, resolve)
            }
            Confined::Beneath(dirs) => {
                let (fd, rel) = (dirs.iter())
                    .find_map(|(dir, fd)| Some((fd, path.strip_prefix(dir).ok()?)))
                    .ok_or_else(refused)?;
                let rel = match rel.as_os_str().is_empty() {
                    true => Path::new("."),
                    false => rel,
                };
                let resolve = libc::RESOLVE_BENEATH | libc::RESOLVE_NO_MAGICLINKS;
                openat2(fd.as_fd(), rel, flags, resolve)
            }
        }
    }

    /// The bytes of regular file `path`, at most `max`, as [`crate::ops::read_regular`] reads.
    pub fn read(&self, path: &Path, max: u64) -> io::Result<Vec<u8>> {
        crate::ops::read_held(self.open(path, libc::O_PATH)?, max)
    }

    /// At most `max` of the entry names in folder `dir`; empty when it doesn't open.
    pub fn list(&self, dir: &Path, max: usize) -> HashSet<OsString> {
        let Ok(fd) = self.open(dir, libc::O_RDONLY | libc::O_DIRECTORY) else {
            return HashSet::new();
        };
        let Ok(entries) = std::fs::read_dir(format!("/proc/self/fd/{}", fd.as_raw_fd())) else {
            return HashSet::new();
        };
        (entries.filter_map(Result::ok))
            .map(|e| e.file_name())
            .take(max)
            .collect()
    }
}

fn openat2(dir: BorrowedFd, path: &Path, flags: libc::c_int, resolve: u64) -> io::Result<OwnedFd> {
    let path = c_path(path)?;
    // SAFETY: an all-zero `open_how` is a valid value of a plain C struct.
    let mut how: libc::open_how = unsafe { std::mem::zeroed() };
    how.flags = (flags | libc::O_CLOEXEC) as u64;
    how.resolve = resolve;
    // SAFETY: `path` is NUL-terminated, `how` is an `open_how` of the size passed, and `dir`
    // is an open folder.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            dir.as_raw_fd(),
            path.as_ptr(),
            &how,
            std::mem::size_of::<libc::open_how>(),
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` was just opened and is owned by nothing else.
    Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
}

#[cfg(test)]
mod tests;
