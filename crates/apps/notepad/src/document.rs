//! Notepad file I/O: reading, atomic writing, line-ending detection.

use reprisekit::tr;
use std::path::{Path, PathBuf};

/// Write so a crash or full disk never leaves the file truncated: temp file,
/// sync, rename, sync the directory. Keeps mode, xattrs and (where permitted)
/// owner. A symlink's target is replaced; hard-linked files and dangling
/// symlinks are written in place (`write_in_place`), since a rename would cut the link.
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
        return write_in_place(path, |f| write_lines(f, lines, sep));
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

/// Rewrites `path` in its own inode. A synced copy of the old bytes waits beside it until the new
/// ones are synced, and a failed write puts them back.
pub(crate) fn write_in_place(
    path: &Path,
    fill: impl FnOnce(&std::fs::File) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let Some(backup) = (match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => Some(backup_beside(path, m.permissions())?),
        _ => None,
    }) else {
        let f = std::fs::File::create(path)?;
        fill(&f)?;
        return f.sync_all();
    };
    let f = match std::fs::File::create(path) {
        Ok(f) => f,
        Err(e) => {
            let _ = std::fs::remove_file(&backup);
            return Err(e);
        }
    };
    if let Err(e) = fill(&f).and_then(|()| f.sync_all()) {
        match restore(&f, &backup) {
            Ok(()) => {
                let _ = std::fs::remove_file(&backup);
            }
            Err(r) => log::error!(
                "restore {} from {}: {r}; the old text stays there",
                path.display(),
                backup.display()
            ),
        }
        return Err(e);
    }
    let _ = std::fs::remove_file(&backup);
    Ok(())
}

fn backup_beside(path: &Path, mode: std::fs::Permissions) -> std::io::Result<PathBuf> {
    let (mut copy, at) = reprisefs::ops::create_temp_beside(path, Some(mode))?;
    let copied = std::fs::File::open(path)
        .and_then(|mut old| std::io::copy(&mut old, &mut copy))
        .and_then(|_| copy.sync_all());
    if let Err(e) = copied {
        let _ = std::fs::remove_file(&at);
        return Err(e);
    }
    Ok(at)
}

fn restore(mut f: &std::fs::File, backup: &Path) -> std::io::Result<()> {
    use std::io::Seek;
    f.set_len(0)?;
    f.rewind()?;
    std::io::copy(&mut std::fs::File::open(backup)?, &mut f)?;
    f.sync_all()
}

/// What a save checks its file against: any field that differs means another program wrote it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    mtime: i64,
    mtime_nsec: i64,
    len: u64,
    ino: u64,
}

impl Stamp {
    /// The file `path` names, through a symlink, whose own times stay put when its target is
    /// rewritten. `None` when there is no file.
    pub(crate) fn of(path: &Path) -> Option<Stamp> {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(path).ok()?;
        Some(Stamp {
            mtime: m.mtime(),
            mtime_nsec: m.mtime_nsec(),
            len: m.len(),
            ino: m.ino(),
        })
    }
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
    let path = path.display().to_string();
    match e.kind() {
        ErrorKind::NotFound => tr!("open-not-found", path = path),
        ErrorKind::FileTooLarge => tr!("open-too-large", path = path),
        ErrorKind::InvalidInput | ErrorKind::IsADirectory => tr!("open-not-a-file", path = path),
        ErrorKind::PermissionDenied => tr!("open-denied", path = path),
        _ => tr!("open-failed", path = path, error = e.to_string()),
    }
}

/// Where `emergency` leaves unsaved text: `$XDG_STATE_HOME/reprise/notepad`.
pub(crate) fn recovery_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("reprise").join("notepad"))
}

/// Write `lines` to `recovery-<pid>-<stem>.txt` in `dir`, made 0700 if missing.
pub(crate) fn write_recovery(
    dir: &Path,
    pid: u32,
    stem: &str,
    lines: &[impl AsRef<str>],
    sep: &str,
) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let path = dir.join(format!("{RECOVERY_PREFIX}{pid}-{stem}.txt"));
    write_document(&path, lines, sep)?;
    Ok(path)
}

const RECOVERY_PREFIX: &str = "recovery-";

/// The recovery files in `dir`, by name.
pub(crate) fn recovered_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(RECOVERY_PREFIX) && n.ends_with(".txt"))
        })
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn a_hard_linked_file_keeps_its_bytes_and_inode_when_the_write_fails_midway() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (a, b) = (tmp.path().join("a.txt"), tmp.path().join("b.txt"));
        std::fs::write(&a, "original text").expect("write");
        std::fs::hard_link(&a, &b).expect("link");

        let failed = write_in_place(&a, |mut f| {
            use std::io::Write;
            f.write_all(b"half")?;
            Err(std::io::Error::other("disk full"))
        });
        assert_eq!(
            failed.expect_err("the write failed").to_string(),
            "disk full"
        );
        for p in [&a, &b] {
            assert_eq!(std::fs::read_to_string(p).expect("read"), "original text");
        }
        let (ma, mb) = (
            std::fs::metadata(&a).unwrap(),
            std::fs::metadata(&b).unwrap(),
        );
        assert_eq!(ma.ino(), mb.ino());
        assert_eq!(ma.nlink(), 2);
        assert_eq!(
            std::fs::read_dir(tmp.path()).unwrap().count(),
            2,
            "the backup is gone"
        );

        write_document(&a, &["new", "text"], "\n").expect("save");
        assert_eq!(std::fs::read_to_string(&b).expect("read"), "new\ntext");
        assert_eq!(std::fs::metadata(&b).unwrap().ino(), ma.ino());
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 2);
    }
}
