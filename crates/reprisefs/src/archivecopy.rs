//! A file Explorer copied out of a compressed folder ([`crate::archive::copy_out`] into a
//! [`crate::archive::scratch`] folder) is read-only and goes with its folder a day later, so a
//! program saves it elsewhere and keeps nothing pointing at it.

use std::io;
use std::path::{Path, PathBuf};

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
