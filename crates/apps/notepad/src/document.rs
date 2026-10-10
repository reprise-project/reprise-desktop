//! Notepad file I/O: reading, atomic writing, line-ending detection.

use std::path::Path;

/// Write so a crash or full disk never leaves the file truncated: temp file,
/// sync, rename, sync the directory. Keeps mode, xattrs and (where permitted)
/// owner. A symlink's target is replaced; hard-linked files and dangling
/// symlinks are written in place, since a rename would cut the link.
pub(crate) fn write_document(
    path: &Path,
    lines: &[impl AsRef<str>],
    sep: &str,
) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let resolved;
    let mut path = path;
    let mut meta = std::fs::symlink_metadata(path).ok();
    if meta.as_ref().is_some_and(|m| m.file_type().is_symlink())
        && let Ok(real) = std::fs::canonicalize(path)
    {
        resolved = real;
        path = &resolved;
        meta = std::fs::symlink_metadata(path).ok();
    }
    if meta
        .as_ref()
        .is_some_and(|m| m.file_type().is_symlink() || m.nlink() > 1)
    {
        return write_lines(std::fs::File::create(path)?, lines, sep);
    }
    if meta.as_ref().is_some_and(|m| m.is_file()) {
        std::fs::OpenOptions::new().write(true).open(path)?;
    }
    let (f, tmp) =
        reprisefs::ops::create_temp_beside(path, meta.as_ref().map(|m| m.permissions()))?;
    let written = (|| {
        if let Some(m) = &meta {
            // Another user's file keeps its owner only under root; its group
            // stays when the process is a member of it.
            if std::os::unix::fs::fchown(&f, Some(m.uid()), Some(m.gid())).is_err() {
                let _ = std::os::unix::fs::fchown(&f, None, Some(m.gid()));
            }
        }
        if meta.is_some() {
            reprisefs::ops::copy_xattrs(path, &f);
        }
        write_lines(&f, lines, sep)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return written;
    }
    // The directory sync makes the rename durable; a file system that refuses it does not fail the save.
    if let Some(dir) = tmp.parent()
        && let Ok(d) = std::fs::File::open(dir)
    {
        let _ = d.sync_all();
    }
    Ok(())
}

fn write_lines(
    out: impl std::io::Write,
    lines: &[impl AsRef<str>],
    sep: &str,
) -> std::io::Result<()> {
    use std::io::Write;
    let mut w = std::io::BufWriter::new(out);
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            w.write_all(sep.as_bytes())?;
        }
        w.write_all(line.as_ref().as_bytes())?;
    }
    w.flush()
}

/// Whether a file's breaks are CRLF. Any `\r\n` decides it, so a mixed file stays CRLF.
pub(crate) fn has_crlf(text: &str) -> bool {
    text.contains("\r\n")
}

/// Largest file Notepad opens: the whole text lives in one edit buffer.
pub(crate) const MAX_OPEN_BYTES: u64 = 32 << 20;

/// Only a regular UTF-8 file up to `MAX_OPEN_BYTES`; a FIFO or device fails without blocking.
pub(crate) fn read_text(path: &Path) -> std::io::Result<String> {
    let bytes = reprisefs::ops::read_regular(path, MAX_OPEN_BYTES)?;
    String::from_utf8(bytes).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Command-line open: a missing file is a new one at that path; other failures must not adopt the path, or a save would overwrite the original.
pub(crate) fn load(path: &Path) -> std::io::Result<String> {
    match read_text(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        r => r,
    }
}

pub(crate) fn open_failed_text(path: &Path, e: &std::io::Error) -> String {
    use std::io::ErrorKind;
    let path = path.display();
    match e.kind() {
        ErrorKind::NotFound => {
            format!("{path}\nFile not found.\nPlease verify the correct file name was given.")
        }
        ErrorKind::FileTooLarge => {
            format!(
                "The file {path} is too large for Notepad.\nUse another editor to edit the file."
            )
        }
        ErrorKind::InvalidInput | ErrorKind::IsADirectory => {
            format!("Cannot open {path}.\nIt is not a file.")
        }
        ErrorKind::PermissionDenied => format!("Cannot open {path}.\nAccess is denied."),
        _ => format!("Cannot open {path}.\n{e}"),
    }
}
