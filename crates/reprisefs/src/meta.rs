//! What a file says about itself in `user.` extended attributes, by the freedesktop shared-metadata
//! convention: backup/index flags (`user.xdg.robots.*`) and a description (`user.dublincore.*`,
//! `user.xdg.comment`, `user.xdg.tags`). A file system without user attributes (FAT, network
//! shares) answers `None` to every read.

use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

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

/// The `[Desktop Entry]` key holding a folder's picture, the one Thumbnails view draws on it: a
/// path, relative to the folder when the picture is inside it. Escaped as desktop entry strings
/// are (`\s`, `\n`, `\t`, `\r`, `\\`).
pub const PICTURE_KEY: &str = "X-Reprise-Picture";

const ICON_KEY: &str = "Icon";

const DIRECTORY_MAX: u64 = 64 * 1024;

/// Folder `dir`'s own icon: its `.directory` Icon=, else the one Nautilus set in gvfs metadata.
pub fn folder_icon(dir: &Path) -> Option<String> {
    let text = directory_text(dir);
    icon_of(dir, text.as_deref())
}

/// Folder `dir`'s picture: its `.directory` [`PICTURE_KEY`] (relative to `dir` unless absolute),
/// else gvfs metadata's [`crate::gvfsmeta::PICTURE_KEY`]. Nothing checks that the file is there.
pub fn folder_picture(dir: &Path) -> Option<PathBuf> {
    let text = directory_text(dir);
    picture_of(dir, text.as_deref())
}

/// [`folder_icon`] and [`folder_picture`] from one read of the `.directory`.
pub fn folder_look(dir: &Path) -> (Option<String>, Option<PathBuf>) {
    let text = directory_text(dir);
    (
        icon_of(dir, text.as_deref()),
        picture_of(dir, text.as_deref()),
    )
}

fn icon_of(dir: &Path, text: Option<&str>) -> Option<String> {
    (text.and_then(|t| entry_value(t, ICON_KEY))).or_else(|| crate::gvfsmeta::custom_icon(dir))
}

fn picture_of(dir: &Path, text: Option<&str>) -> Option<PathBuf> {
    match text.and_then(|t| entry_value(t, PICTURE_KEY)) {
        Some(v) => Some(dir.join(unescape(&v))),
        None => crate::gvfsmeta::custom_picture(dir),
    }
}

fn directory_text(dir: &Path) -> Option<String> {
    let bytes = crate::ops::read_regular(&dir.join(DIRECTORY_FILE), DIRECTORY_MAX).ok()?;
    String::from_utf8(bytes).ok()
}

/// The key `line` sets, `None` for a group header, comment or blank.
fn key_of(line: &str) -> Option<&str> {
    let (k, _) = line.split_once('=')?;
    let k = k.trim();
    (!k.starts_with('#') && !k.starts_with('[')).then_some(k)
}

/// `key`'s value in the `[Desktop Entry]` group of desktop entry `text`, unless empty.
fn entry_value(text: &str, key: &str) -> Option<String> {
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry
            && key_of(line) == Some(key)
            && let Some((_, v)) = line.split_once('=')
            && !v.trim().is_empty()
        {
            return Some(v.trim().to_string());
        }
    }
    None
}

fn escape(v: &str) -> String {
    let end = v.trim_end_matches(' ').len();
    let mut out = String::with_capacity(v.len());
    for (i, c) in v.char_indices() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            ' ' if i == 0 || i >= end => out.push_str("\\s"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

/// `text` with each of `keys` set to its value, or removed for None, in its `[Desktop Entry]`
/// group, which is added when missing. Every other line is kept.
pub fn with_entry_keys(text: &str, keys: &[(&str, Option<&str>)]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_entry = false;
    let mut had_entry = false;
    let mut placed = vec![false; keys.len()];
    let place_rest = |out: &mut Vec<String>, placed: &mut [bool]| {
        for ((k, v), done) in keys.iter().zip(placed.iter_mut()) {
            if let Some(v) = v.filter(|_| !*done) {
                out.push(format!("{k}={v}"));
                *done = true;
            }
        }
    };
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if in_entry {
                place_rest(&mut out, &mut placed);
            }
            in_entry = t == "[Desktop Entry]";
            had_entry |= in_entry;
        } else if in_entry
            && let Some(k) = key_of(t)
            && let Some(i) = keys.iter().position(|(key, _)| *key == k)
        {
            if let Some(v) = keys[i].1.filter(|_| !placed[i]) {
                out.push(format!("{k}={v}"));
                placed[i] = true;
            }
            continue;
        }
        out.push(line.to_string());
    }
    if !had_entry && keys.iter().any(|(_, v)| v.is_some()) {
        out.push("[Desktop Entry]".to_string());
    }
    place_rest(&mut out, &mut placed);
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// A change to a folder's look: each `Some` sets its part (`Some(value)`) or clears it (`None`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FolderLook<'a> {
    pub icon: Option<Option<&'a str>>,
    pub picture: Option<Option<&'a Path>>,
}

/// Give `dir` the icon `icon`, or remove it for None ([`set_folder_look`]).
pub fn set_folder_icon(dir: &Path, icon: Option<&str>) -> io::Result<()> {
    set_folder_look(
        dir,
        FolderLook {
            icon: Some(icon),
            ..FolderLook::default()
        },
    )
}

/// Writes `look` into `dir`'s [`DIRECTORY_FILE`] in one write: the icon as Icon=, the picture as
/// [`PICTURE_KEY`] (relative when inside `dir`; a path that is not UTF-8 is refused). A file left
/// with only an empty `[Desktop Entry]` goes. A link, non-regular file or one over 64 KiB is
/// refused. Written via a temp file with the old mode and renamed over, so a crash or a refused
/// write (a folder without write access) leaves the old file. A part cleared also clears what
/// gvfs metadata holds for it, once the file is written.
pub fn set_folder_look(dir: &Path, look: FolderLook) -> io::Result<()> {
    let picture = match look.picture {
        Some(Some(p)) => Some(Some(picture_value(dir, p)?)),
        Some(None) => Some(None),
        None => None,
    };
    let mut keys: Vec<(&str, Option<&str>)> = Vec::new();
    keys.extend(look.icon.map(|i| (ICON_KEY, i)));
    keys.extend(picture.as_ref().map(|p| (PICTURE_KEY, p.as_deref())));
    if keys.is_empty() {
        return Ok(());
    }
    set_directory_keys(dir, &keys)?;
    let mut unset: Vec<&str> = Vec::new();
    if look.icon == Some(None) {
        unset.extend(crate::gvfsmeta::ICON_KEYS);
    }
    if look.picture == Some(None) {
        unset.push(crate::gvfsmeta::PICTURE_KEY);
    }
    if !unset.is_empty()
        && let Err(e) = crate::gvfsmeta::unset(dir, &unset)
    {
        log::warn!("{}: could not clear its gvfs metadata: {e}", dir.display());
    }
    Ok(())
}

fn picture_value(dir: &Path, pic: &Path) -> io::Result<String> {
    let rel = (pic.strip_prefix(dir).ok())
        .filter(|r| !r.as_os_str().is_empty())
        .unwrap_or(pic);
    let s = rel
        .to_str()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidFilename))?;
    Ok(escape(s))
}

fn set_directory_keys(dir: &Path, keys: &[(&str, Option<&str>)]) -> io::Result<()> {
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
    let new = with_entry_keys(&old, keys);
    if new == old {
        return Ok(());
    }
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
    fn a_folders_picture_inside_it_is_kept_relative_and_one_outside_whole() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("Trip");
        std::fs::create_dir(&d).unwrap();
        let read = || std::fs::read_to_string(d.join(DIRECTORY_FILE)).unwrap();
        let set = |p: Option<&Path>| {
            set_folder_look(
                &d,
                FolderLook {
                    picture: Some(p),
                    ..FolderLook::default()
                },
            )
        };
        assert_eq!(folder_picture(&d), None);
        set(Some(&d.join(" beach.jpg"))).unwrap();
        assert_eq!(read(), "[Desktop Entry]\nX-Reprise-Picture=\\sbeach.jpg\n");
        assert_eq!(folder_picture(&d), Some(d.join(" beach.jpg")));
        set(Some(&d.join("best/b.jpg"))).unwrap();
        assert_eq!(read(), "[Desktop Entry]\nX-Reprise-Picture=best/b.jpg\n");
        let outside = t.path().join("Trip 2/a\\b\n.png");
        set(Some(&outside)).unwrap();
        assert_eq!(folder_picture(&d), Some(outside), "escaped and read back");
        assert!(!read().contains("a\\b\n"), "no raw newline or backslash");
        set(None).unwrap();
        assert_eq!(folder_picture(&d), None);
        assert!(!d.join(DIRECTORY_FILE).exists(), "nothing left, no file");
    }

    #[test]
    fn an_icon_and_a_picture_change_in_one_write_beside_the_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(
            d.join(DIRECTORY_FILE),
            "[Desktop Entry]\nName=Tunes\nX-Reprise-Picture = old.png\n[Other]\nIcon=x\n",
        )
        .unwrap();
        let both = FolderLook {
            icon: Some(Some("folder-music")),
            picture: Some(Some(Path::new("/srv/cover.png"))),
        };
        set_folder_look(d, both).unwrap();
        assert_eq!(
            std::fs::read_to_string(d.join(DIRECTORY_FILE)).unwrap(),
            "[Desktop Entry]\nName=Tunes\nX-Reprise-Picture=/srv/cover.png\nIcon=folder-music\n[Other]\nIcon=x\n"
        );
        assert_eq!(folder_icon(d).as_deref(), Some("folder-music"));
        set_folder_icon(d, None).unwrap();
        assert_eq!(folder_icon(d), None);
        assert_eq!(
            folder_picture(d).as_deref(),
            Some(Path::new("/srv/cover.png")),
            "clearing the icon keeps the picture"
        );
    }

    #[test]
    fn a_folder_without_write_access_keeps_its_directory_file() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("locked");
        std::fs::create_dir(&d).unwrap();
        let old = "[Desktop Entry]\nX-Reprise-Picture=a.png\n";
        std::fs::write(d.join(DIRECTORY_FILE), old).unwrap();
        std::fs::set_permissions(&d, PermissionsExt::from_mode(0o555)).unwrap();
        // Root writes through the mode bits.
        if std::fs::write(d.join("probe"), "").is_ok() {
            return;
        }
        let look = FolderLook {
            icon: Some(Some("folder-music")),
            picture: Some(Some(&d.join("b.png"))),
        };
        let refused = set_folder_look(&d, look);
        let cleared = set_folder_look(
            &d,
            FolderLook {
                picture: Some(None),
                ..FolderLook::default()
            },
        );
        std::fs::set_permissions(&d, PermissionsExt::from_mode(0o755)).unwrap();
        for e in [refused, cleared] {
            assert_eq!(
                e.map_err(|e| e.kind()),
                Err(io::ErrorKind::PermissionDenied)
            );
        }
        assert_eq!(
            std::fs::read_to_string(d.join(DIRECTORY_FILE)).unwrap(),
            old
        );
        let names: Vec<_> = std::fs::read_dir(&d).unwrap().flatten().collect();
        assert_eq!(names.len(), 1, "no temporary file is left");
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
