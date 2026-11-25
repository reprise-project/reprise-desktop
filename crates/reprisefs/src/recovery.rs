//! Recovery files: unsaved work a program's `emergency` leaves in its folder under
//! `$XDG_STATE_HOME/reprise`, named `recovery-<pid>-<stem>.<ext>`, for its next start to offer.

use std::path::{Path, PathBuf};

const PREFIX: &str = "recovery-";

/// `$XDG_STATE_HOME/reprise/<program>`.
pub fn dir(program: &str) -> Option<PathBuf> {
    Some(reprisexdg::state_home()?.join("reprise").join(program))
}

/// Writes `bytes` to `recovery-<pid>-<stem>.<ext>` in `dir`, made 0700 if missing.
pub fn write(
    dir: &Path,
    pid: u32,
    stem: &str,
    ext: &str,
    bytes: &[u8],
) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let path = dir.join(format!("{PREFIX}{pid}-{stem}.{ext}"));
    crate::ops::write_document(&path, bytes)?;
    Ok(path)
}

/// The stem a recovery file was written under: `notes` for `recovery-42-notes.txt`.
pub fn stem(path: &Path) -> Option<&str> {
    let rest = path.file_name()?.to_str()?.strip_prefix(PREFIX)?;
    let (pid, rest) = rest.split_once('-')?;
    if pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(rest.rsplit_once('.').map_or(rest, |(stem, _)| stem))
}

/// The recovery files in `dir` ending in `.<ext>`, by name.
pub fn found(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let suffix = format!(".{ext}");
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(PREFIX) && n.ends_with(&suffix))
        })
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_written_is_found_by_its_extension_in_a_private_folder() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("reprise").join("paint");
        let kept = write(&dir, 42, "Untitled", "png", b"picture").unwrap();
        assert_eq!(kept, dir.join("recovery-42-Untitled.png"));
        std::fs::write(dir.join("recovery-43-notes.txt"), "text").unwrap();
        std::fs::write(dir.join("Untitled.png"), "not a recovery file").unwrap();
        assert_eq!(found(&dir, "png"), [kept]);
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn a_recovery_file_gives_back_its_stem() {
        let stem =
            |name: &str| stem(Path::new("/state/notepad").join(name).as_path()).map(String::from);
        assert_eq!(
            stem("recovery-42-my-notes.v2.txt").as_deref(),
            Some("my-notes.v2")
        );
        assert_eq!(
            stem("recovery-42-Untitled.png").as_deref(),
            Some("Untitled")
        );
        assert_eq!(stem("recovery-x-notes.txt"), None);
        assert_eq!(stem("notes.txt"), None);
    }
}
