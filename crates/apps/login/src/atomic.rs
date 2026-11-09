//! Replaces a file as root writes it: whole or not at all, in modes no umask changes.

use std::fs::{DirBuilder, File, OpenOptions, Permissions};
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;

/// Makes `dir` in `mode` when it is missing.
pub fn make_dir(dir: &Path, mode: u32) -> io::Result<()> {
    if !dir.exists() {
        DirBuilder::new().recursive(true).mode(mode).create(dir)?;
        // The modes here and in `replace` are masked by the umask, which a caller may pass on.
        std::fs::set_permissions(dir, Permissions::from_mode(mode))?;
    }
    Ok(())
}

/// Writes `bytes` to `path.tmp` in `file_mode`, syncs it and renames it over `path`, then syncs
/// the directory, made in `dir_mode` when missing. A failure before the rename removes the tmp.
pub fn replace(path: &Path, bytes: &[u8], dir_mode: u32, file_mode: u32) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    make_dir(dir, dir_mode)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    // unlink does not follow a planted symlink, and O_EXCL | O_NOFOLLOW refuses one planted since.
    match std::fs::remove_file(&tmp) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW)
        .mode(file_mode)
        .open(&tmp)?;
    let replaced = (|| {
        file.set_permissions(Permissions::from_mode(file_mode))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = replaced {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // Once renamed the file is in place, so the caller must not treat it as unwritten.
    if let Err(e) = File::open(dir).and_then(|d| d.sync_all()) {
        log::warn!(
            "syncing {} after replacing {}: {e}",
            dir.display(),
            path.display()
        );
    }
    Ok(())
}
