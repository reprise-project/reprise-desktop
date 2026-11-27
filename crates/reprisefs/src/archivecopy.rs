//! A file Explorer copied out of a compressed folder ([`crate::archive::copy_out`] into a
//! [`crate::archive::scratch`] folder) is read-only and goes with its folder a day later, so a
//! program saves it elsewhere and keeps nothing pointing at it.

use crate::archive::is_copy;
use reprisekit::tr;
use std::ffi::{CStr, CString, OsString};
use std::io::{self, Read};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// The xattr on a copy's scratch folder naming the compressed folder it came from; the copy
/// itself is read-only, and a read-only file takes no user xattrs.
const ORIGIN: &CStr = c"user.reprise.archive";

/// Notes on `copy`'s scratch folder that it came out of `archive`, for [`origin`]. An `archive`
/// that is itself a copy (a compressed folder inside another) notes the outer archive it came
/// from, or nothing when that is not known.
pub fn set_origin(copy: &Path, archive: &Path) -> io::Result<()> {
    let archive = if is_copy(archive) {
        match origin(archive) {
            Some(outer) => outer,
            None => return Ok(()),
        }
    } else {
        archive.to_path_buf()
    };
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
    if !is_copy(path) {
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
    let archive = PathBuf::from(OsString::from_vec(buf));
    archive.is_absolute().then_some(archive)
}

/// The folder a file dialog starts in for the document at `path`: its own, or for an archive
/// copy the folder holding the archive. None when the copy does not say, and for any folder in
/// the scratch folders, which go a day later.
pub fn dialog_folder(path: &Path) -> Option<PathBuf> {
    let folder = if is_copy(path) {
        origin(path)?.parent()?.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    (!is_copy(&folder)).then_some(folder)
}

/// What `[wallpaper] image` points at for `picture`: the picture itself, or for an archive copy
/// a new file under `$XDG_DATA_HOME/reprise/backgrounds`, which outlives the scratch folder. The
/// name is the picture's, numbered (`dunes (2).png`) past other pictures; one holding the same
/// bytes is used again. The file is written beside its name and linked to it, so a crash leaves
/// no part-written picture under a real name.
pub fn lasting_background(picture: &Path) -> io::Result<PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;
    if !is_copy(picture) {
        return Ok(picture.to_path_buf());
    }
    let dir = reprisexdg::data_home()
        .ok_or_else(|| io::Error::other(tr!("background-no-data-dir")))?
        .join("reprise/backgrounds");
    let name = picture
        .file_name()
        .ok_or_else(|| io::Error::other(tr!("background-no-name")))?;
    // The config holds the path as UTF-8.
    let name = OsString::from(name.to_string_lossy().into_owned());
    let bytes = std::fs::read(picture)?;
    std::fs::create_dir_all(&dir)?;
    let same = |p: &Path| {
        let len = bytes.len() as u64;
        std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file() && m.nlink() == 1 && m.len() == len)
            && holds(p, &bytes)
    };
    let pick = || {
        dir.join(crate::names::first_free(&name, |n| {
            let p = dir.join(n);
            std::fs::symlink_metadata(&p).is_ok() && !same(&p)
        }))
    };
    let first = pick();
    if same(&first) {
        return Ok(first);
    }
    let (mut file, tmp) = crate::ops::create_temp_beside(&dir.join(&name), None)?;
    if let Err(e) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // A name taken between the pick and the link is picked again.
    let mut tries = 0;
    let kept = loop {
        let kept = pick();
        if same(&kept) {
            break Ok(kept);
        }
        match std::fs::hard_link(&tmp, &kept) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && tries < 8 => tries += 1,
            linked => break linked.map(|()| kept),
        }
    };
    let _ = std::fs::remove_file(&tmp);
    kept
}

/// Whether the file at `path` holds exactly `bytes`, read a block at a time.
fn holds(path: &Path, bytes: &[u8]) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut block = vec![0u8; 64 * 1024];
    let mut at = 0;
    loop {
        match file.read(&mut block) {
            Ok(0) => return at == bytes.len(),
            Ok(n) if bytes.get(at..at + n) == Some(&block[..n]) => at += n,
            _ => return false,
        }
    }
}

fn cstring(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)
}
