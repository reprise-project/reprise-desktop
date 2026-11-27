//! A file Explorer copied out of a compressed folder ([`crate::archive::copy_out`] into a
//! [`crate::archive::scratch`] folder) is read-only and goes with its folder a day later, so a
//! program saves it elsewhere and keeps nothing pointing at it.

use std::ffi::{CStr, CString};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// The xattr on a copy's scratch folder naming the compressed folder it came from; the copy
/// itself is read-only, and a read-only file takes no user xattrs.
const ORIGIN: &CStr = c"user.reprise.archive";

/// Notes on `copy`'s scratch folder that it came out of `archive`, for [`origin`].
pub fn set_origin(copy: &Path, archive: &Path) -> io::Result<()> {
    let dir = cstring(copy.parent().ok_or_else(|| io::Error::other("no folder"))?)?;
    let value = archive.as_os_str().as_bytes();
    // SAFETY: `dir` and `ORIGIN` are NUL-terminated, and `value` is valid for its length.
    let set = unsafe {
        libc::setxattr(
            dir.as_ptr(),
            ORIGIN.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    if set == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// The compressed folder `path` was copied out of, when it is an archive copy whose folder
/// says ([`set_origin`]).
pub fn origin(path: &Path) -> Option<PathBuf> {
    if !crate::archive::is_copy(path) {
        return None;
    }
    let dir = cstring(path.parent()?).ok()?;
    let mut buf = vec![0u8; libc::PATH_MAX as usize];
    // SAFETY: `dir` and `ORIGIN` are NUL-terminated, and `buf` is writable for its length.
    let got = unsafe {
        libc::getxattr(
            dir.as_ptr(),
            ORIGIN.as_ptr(),
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    };
    buf.truncate(usize::try_from(got).ok()?);
    let archive = PathBuf::from(std::ffi::OsString::from_vec(buf));
    archive.is_absolute().then_some(archive)
}

/// The folder a file dialog starts in for the document at `path`: its own, or for an archive
/// copy the folder holding the archive (None when the copy does not say).
pub fn dialog_folder(path: &Path) -> Option<PathBuf> {
    if crate::archive::is_copy(path) {
        return origin(path)?.parent().map(Path::to_path_buf);
    }
    path.parent().map(Path::to_path_buf)
}

/// What `[wallpaper] image` points at for `picture`: the picture itself, or for an archive copy
/// a copy of it under `$XDG_DATA_HOME/reprise/backgrounds`, which outlives the scratch folder.
pub fn lasting_background(picture: &Path) -> io::Result<PathBuf> {
    if !crate::archive::is_copy(picture) {
        return Ok(picture.to_path_buf());
    }
    let dir = reprisexdg::data_home()
        .ok_or_else(|| io::Error::other("no data directory"))?
        .join("reprise/backgrounds");
    std::fs::create_dir_all(&dir)?;
    let name = picture
        .file_name()
        .ok_or_else(|| io::Error::other("no name"))?;
    let kept = dir.join(name);
    crate::ops::write_document(&kept, &std::fs::read(picture)?)?;
    Ok(kept)
}

fn cstring(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)
}
