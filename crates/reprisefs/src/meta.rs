//! What a file says about itself in `user.` extended attributes, by the freedesktop shared-metadata
//! convention: backup/index flags (`user.xdg.robots.*`) and a description (`user.dublincore.*`,
//! `user.xdg.comment`, `user.xdg.tags`). A file system without user attributes (FAT, network
//! shares) answers `None` to every read.

use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub const BACKUP: &str = "user.xdg.robots.backup";
pub const INDEX: &str = "user.xdg.robots.index";

pub const TITLE: &str = "user.dublincore.title";
pub const SUBJECT: &str = "user.dublincore.subject";
pub const AUTHOR: &str = "user.dublincore.creator";
pub const CATEGORY: &str = "user.dublincore.type";
pub const KEYWORDS: &str = "user.xdg.tags";
pub const COMMENTS: &str = "user.xdg.comment";

pub const SUMMARY_KEYS: [&str; 6] = [TITLE, SUBJECT, AUTHOR, CATEGORY, KEYWORDS, COMMENTS];

fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path holds a NUL"))
}

fn c_name(name: &str) -> io::Result<CString> {
    CString::new(name).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "name holds a NUL"))
}

pub fn get(path: &Path, name: &str) -> io::Result<Option<String>> {
    let (p, n) = (c_path(path)?, c_name(name)?);
    let mut buf = vec![0u8; 256];
    loop {
        // SAFETY: `p` and `n` are NUL-terminated and `buf` is writable
        // for its length, all live across the call.
        let got =
            unsafe { libc::getxattr(p.as_ptr(), n.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
        if got >= 0 {
            buf.truncate(got as usize);
            return Ok(Some(String::from_utf8_lossy(&buf).into_owned()));
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::ENODATA) => return Ok(None),
            Some(libc::ERANGE) if buf.len() < 1 << 16 => buf.resize(buf.len() * 4, 0),
            _ => return Err(e),
        }
    }
}

/// Set attribute `name` on `path` to `value`, or remove it for `None`. Writing needs write
/// permission, so a read-only file of one's own gets its owner's write bit for the write and loses
/// it after.
pub fn set(path: &Path, name: &str, value: Option<&str>) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    match set_once(path, name, value) {
        Err(e) if e.raw_os_error() == Some(libc::EACCES) => {
            let mode = std::fs::metadata(path)?.permissions().mode();
            if mode & 0o200 != 0 {
                return Err(e);
            }
            std::fs::set_permissions(path, PermissionsExt::from_mode(mode | 0o200))?;
            let r = set_once(path, name, value);
            std::fs::set_permissions(path, PermissionsExt::from_mode(mode))?;
            r
        }
        r => r,
    }
}

fn set_once(path: &Path, name: &str, value: Option<&str>) -> io::Result<()> {
    let (p, n) = (c_path(path)?, c_name(name)?);
    // SAFETY: `p`, `n` and the value are live, NUL-terminated or read for
    // their length, across each call.
    let rc = unsafe {
        match value {
            Some(v) => libc::setxattr(p.as_ptr(), n.as_ptr(), v.as_ptr().cast(), v.len(), 0),
            None => libc::removexattr(p.as_ptr(), n.as_ptr()),
        }
    };
    if rc == 0 {
        return Ok(());
    }
    let e = io::Error::last_os_error();
    match (value, e.raw_os_error()) {
        (None, Some(libc::ENODATA)) => Ok(()),
        _ => Err(e),
    }
}

pub fn supported(path: &Path) -> bool {
    get(path, BACKUP).is_ok()
}

pub fn robots(path: &Path, name: &str) -> Option<bool> {
    get(path, name).ok().map(|v| v.as_deref() != Some("false"))
}

pub fn set_robots(path: &Path, name: &str, on: bool) -> io::Result<()> {
    set(path, name, (!on).then_some("false"))
}

pub fn summary(path: &Path) -> Option<[String; 6]> {
    let mut out: [String; 6] = Default::default();
    for (slot, key) in out.iter_mut().zip(SUMMARY_KEYS) {
        *slot = get(path, key).ok()?.unwrap_or_default();
    }
    Some(out)
}

pub fn set_summary(path: &Path, key: &str, value: &str) -> io::Result<()> {
    set(path, key, (!value.is_empty()).then_some(value))
}

pub const DIRECTORY_FILE: &str = ".directory";

const DIRECTORY_MAX: u64 = 64 * 1024;

/// Folder `dir`'s own icon: its `.directory` Icon=, else the one Nautilus set in gvfs metadata.
pub fn folder_icon(dir: &Path) -> Option<String> {
    directory_icon(dir).or_else(|| crate::gvfsmeta::custom_icon(dir))
}

fn directory_icon(dir: &Path) -> Option<String> {
    let bytes = crate::ops::read_regular(&dir.join(DIRECTORY_FILE), DIRECTORY_MAX).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry
            && let Some(v) = line.strip_prefix("Icon=")
            && !v.trim().is_empty()
        {
            return Some(v.trim().to_string());
        }
    }
    None
}

pub fn with_folder_icon(text: &str, icon: Option<&str>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_entry = false;
    let mut had_entry = false;
    let mut placed = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if in_entry
                && !placed
                && let Some(i) = icon
            {
                out.push(format!("Icon={i}"));
                placed = true;
            }
            in_entry = t == "[Desktop Entry]";
            had_entry |= in_entry;
        } else if in_entry && t.starts_with("Icon=") {
            if let Some(i) = icon.filter(|_| !placed) {
                out.push(format!("Icon={i}"));
                placed = true;
            }
            continue;
        }
        out.push(line.to_string());
    }
    if let Some(i) = icon.filter(|_| !placed) {
        if !had_entry {
            out.push("[Desktop Entry]".to_string());
        }
        out.push(format!("Icon={i}"));
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Give `dir` the icon `icon`, or remove it for None, in its [`DIRECTORY_FILE`]. A file left with
/// only an empty `[Desktop Entry]` goes. A link, non-regular file or one over 64 KiB is refused.
/// Written via a temp file with the old mode and renamed over, so a crash leaves the old or new
/// file. None also clears an icon Nautilus set in gvfs metadata.
pub fn set_folder_icon(dir: &Path, icon: Option<&str>) -> io::Result<()> {
    set_directory_icon(dir, icon)?;
    if icon.is_none()
        && let Err(e) = crate::gvfsmeta::unset_custom_icon(dir)
    {
        log::warn!("{}: could not clear its gvfs icon: {e}", dir.display());
    }
    Ok(())
}

fn set_directory_icon(dir: &Path, icon: Option<&str>) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = dir.join(DIRECTORY_FILE);
    let opened = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(&path);
    let (old, mode) = match opened {
        Ok(f) => {
            let bytes = crate::ops::read_opened(&f, DIRECTORY_MAX)?;
            let text = String::from_utf8(bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            (text, Some(f.metadata()?.permissions()))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => (String::new(), None),
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
            return Err(io::Error::other("the folder's .directory is a link"));
        }
        Err(e) => return Err(e),
    };
    let new = with_folder_icon(&old, icon);
    if new.trim() == "[Desktop Entry]" || new.trim().is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let (mut f, tmp) = crate::ops::create_temp_beside(&path, mode)?;
    let written = (|| {
        f.write_all(new.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, &path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

pub fn write_summary(path: &Path, moves: &[(usize, String)]) -> io::Result<()> {
    let mut first = None;
    for (k, value) in moves {
        let Some(key) = SUMMARY_KEYS.get(*k) else {
            continue;
        };
        if let Err(e) = set_summary(path, key, value) {
            log::warn!(
                "properties: could not write {key} on {}: {e}",
                path.display()
            );
            first.get_or_insert(e);
        }
    }
    first.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> Option<(tempfile::TempDir, std::path::PathBuf)> {
        let dir = tempfile::tempdir().ok()?;
        let f = dir.path().join("a.txt");
        std::fs::write(&f, "a").ok()?;
        supported(&f).then_some((dir, f))
    }

    #[test]
    fn robots_are_on_until_set_to_false() {
        let Some((_d, f)) = file() else { return };
        assert_eq!(robots(&f, INDEX), Some(true));
        set_robots(&f, INDEX, false).unwrap();
        assert_eq!(get(&f, INDEX).unwrap().as_deref(), Some("false"));
        assert_eq!(robots(&f, INDEX), Some(false));
        set_robots(&f, INDEX, true).unwrap();
        assert_eq!(get(&f, INDEX).unwrap(), None, "on removes the attribute");
        set_robots(&f, INDEX, true).unwrap();
    }

    #[test]
    fn a_folders_icon_round_trips_through_its_directory_file() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert_eq!(folder_icon(d), None);
        set_folder_icon(d, Some("folder-music")).unwrap();
        assert_eq!(folder_icon(d).as_deref(), Some("folder-music"));
        std::fs::write(
            d.join(DIRECTORY_FILE),
            "[Desktop Entry]\nIcon=folder-music\nName=Tunes\n[Other]\nIcon=x\n",
        )
        .unwrap();
        set_folder_icon(d, Some("/p/pic.png")).unwrap();
        assert_eq!(
            std::fs::read_to_string(d.join(DIRECTORY_FILE)).unwrap(),
            "[Desktop Entry]\nIcon=/p/pic.png\nName=Tunes\n[Other]\nIcon=x\n"
        );
        set_folder_icon(d, None).unwrap();
        assert_eq!(folder_icon(d), None);
        assert!(d.join(DIRECTORY_FILE).exists(), "Name= keeps the file");
        std::fs::write(d.join(DIRECTORY_FILE), "[Desktop Entry]\nIcon=a\n").unwrap();
        set_folder_icon(d, None).unwrap();
        assert!(!d.join(DIRECTORY_FILE).exists(), "nothing left, no file");
    }

    #[test]
    fn a_directory_file_that_is_no_small_regular_file_names_no_icon() {
        let t = tempfile::tempdir().unwrap();
        let [fifo, zero, big] = ["fifo", "zero", "big"].map(|n| t.path().join(n));
        for d in [&fifo, &zero, &big] {
            std::fs::create_dir(d).unwrap();
        }
        crate::testfs::mkfifo(&fifo.join(DIRECTORY_FILE));
        std::os::unix::fs::symlink("/dev/zero", zero.join(DIRECTORY_FILE)).unwrap();
        let mut text = "[Desktop Entry]\nIcon=folder-music\n".to_string();
        text.push_str(&"#\n".repeat(DIRECTORY_MAX as usize));
        std::fs::write(big.join(DIRECTORY_FILE), text).unwrap();
        for d in [fifo, zero, big] {
            assert_eq!(crate::testfs::within(move || folder_icon(&d)), None);
        }
    }

    #[test]
    fn a_linked_directory_file_is_not_written_through() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let t = tempfile::tempdir().unwrap();
        let (linked, own) = (t.path().join("linked"), t.path().join("own"));
        std::fs::create_dir(&linked).unwrap();
        std::fs::create_dir(&own).unwrap();
        let victim = t.path().join("victim");
        std::fs::write(&victim, "keep\r\n").unwrap();
        std::os::unix::fs::symlink("../victim", linked.join(DIRECTORY_FILE)).unwrap();
        assert!(set_folder_icon(&linked, Some("folder-music")).is_err());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep\r\n");

        let file = own.join(DIRECTORY_FILE);
        std::fs::write(&file, "[Desktop Entry]\nName=Tunes\n").unwrap();
        std::fs::set_permissions(&file, PermissionsExt::from_mode(0o600)).unwrap();
        let before = std::fs::metadata(&file).unwrap().ino();
        set_folder_icon(&own, Some("folder-music")).unwrap();
        let after = std::fs::metadata(&file).unwrap();
        assert_ne!(after.ino(), before, "a new file is renamed into place");
        assert_eq!(after.permissions().mode() & 0o777, 0o600);
        assert_eq!(folder_icon(&own).as_deref(), Some("folder-music"));
        let names: Vec<_> = std::fs::read_dir(&own).unwrap().flatten().collect();
        assert_eq!(names.len(), 1, "no temporary file is left");
    }

    #[test]
    fn a_read_only_file_still_takes_a_flag() {
        use std::os::unix::fs::PermissionsExt;
        let Some((_d, f)) = file() else { return };
        std::fs::set_permissions(&f, PermissionsExt::from_mode(0o444)).unwrap();
        set_robots(&f, BACKUP, false).unwrap();
        assert_eq!(robots(&f, BACKUP), Some(false));
        let mode = std::fs::metadata(&f).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o444);
    }

    #[test]
    fn summary_reads_back_what_was_written_and_an_empty_field_removes_it() {
        let Some((_d, f)) = file() else { return };
        assert_eq!(summary(&f), Some(Default::default()));
        set_summary(&f, TITLE, "Report").unwrap();
        set_summary(&f, KEYWORDS, "q3,draft").unwrap();
        let s = summary(&f).unwrap();
        assert_eq!((s[0].as_str(), s[4].as_str()), ("Report", "q3,draft"));
        set_summary(&f, TITLE, "").unwrap();
        assert_eq!(get(&f, TITLE).unwrap(), None);
    }
}
