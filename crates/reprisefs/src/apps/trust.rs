//! The launcher trust marker: `user.reprise.trusted` holding HMAC-SHA256 of the file's
//! `dev:ino` under a per-user key, so an archive or another user's copy cannot carry it.
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub(crate) const TRUST_XATTR: &std::ffi::CStr = c"user.reprise.trusted";
const KEY_LEN: usize = 32;
type Key = [u8; KEY_LEN];

fn mac(key: &Key, meta: &std::fs::Metadata) -> Hmac<Sha256> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    m.update(format!("{}:{}", meta.dev(), meta.ino()).as_bytes());
    m
}

fn value(key: &Key, meta: &std::fs::Metadata) -> Vec<u8> {
    mac(key, meta).finalize().into_bytes().to_vec()
}

/// Whether `value` is the marker for the file `meta` describes.
pub(crate) fn verifies(value: &[u8], meta: &std::fs::Metadata) -> bool {
    key().is_ok_and(|k| verifies_with(&k, value, meta))
}

fn verifies_with(key: &Key, value: &[u8], meta: &std::fs::Metadata) -> bool {
    mac(key, meta).verify_slice(value).is_ok()
}

/// `$XDG_STATE_HOME/reprise/trust.key`, else `~/.local/state/reprise/trust.key`.
#[cfg(not(test))]
fn key_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("reprise/trust.key"))
}

#[cfg(not(test))]
fn key() -> io::Result<Key> {
    static KEY: std::sync::OnceLock<Key> = std::sync::OnceLock::new();
    if let Some(k) = KEY.get() {
        return Ok(*k);
    }
    let path = key_path().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home"))?;
    let k = load_or_create_key(&path)?;
    Ok(*KEY.get_or_init(|| k))
}

#[cfg(test)]
fn key() -> io::Result<Key> {
    Ok([7; KEY_LEN])
}

/// The key at `path`, made with fresh random bytes (mode 0600) when there is none. A key
/// another process lands first wins: the new one is written beside it and hard-linked in.
fn load_or_create_key(path: &Path) -> io::Result<Key> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    match read_key(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        r => return r,
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut k = [0u8; KEY_LEN];
    // SAFETY: `k` is writable for its length.
    let n = unsafe { libc::getrandom(k.as_mut_ptr().cast(), k.len(), 0) };
    if n != KEY_LEN as isize {
        return Err(io::Error::last_os_error());
    }
    let tmp = dir.join(format!(".trust.key.{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let made = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(&k)?;
        f.sync_all()?;
        std::fs::hard_link(&tmp, path)
    })();
    let _ = std::fs::remove_file(&tmp);
    match made {
        Ok(()) => Ok(k),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => read_key(path),
        Err(e) => Err(e),
    }
}

fn read_key(path: &Path) -> io::Result<Key> {
    let bytes = std::fs::read(path)?;
    bytes
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "trust.key is not 32 bytes"))
}

fn cpath(path: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path holds a NUL"))
}

/// Whether the `.desktop` file at `path` (followed through a symlink) is one the user
/// allowed to run.
pub fn is_trusted(path: &Path) -> bool {
    key().is_ok_and(|k| is_trusted_with(&k, path))
}

fn is_trusted_with(key: &Key, path: &Path) -> bool {
    let (Ok(c), Ok(meta)) = (cpath(path), std::fs::metadata(path)) else {
        return false;
    };
    let mut buf = [0u8; 64];
    // SAFETY: `c` and the attribute name are NUL-terminated and `buf` is
    // writable for its length, all live across the call.
    let n = unsafe {
        libc::getxattr(
            c.as_ptr(),
            TRUST_XATTR.as_ptr(),
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    };
    usize::try_from(n).is_ok_and(|n| verifies_with(key, &buf[..n], &meta))
}

pub fn set_trusted(path: &Path) -> io::Result<()> {
    let c = cpath(path)?;
    let v = value(&key()?, &std::fs::metadata(path)?);
    // SAFETY: `c` and the name are NUL-terminated and the value is
    // readable for its length, all live across the call.
    let rc = unsafe {
        libc::setxattr(
            c.as_ptr(),
            TRUST_XATTR.as_ptr(),
            v.as_ptr().cast(),
            v.len(),
            0,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn allow_launching(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)?.permissions().mode();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode | 0o111))?;
    set_trusted(path)
}

pub(crate) fn set_trusted_fd(file: &std::fs::File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let v = value(&key()?, &file.metadata()?);
    // SAFETY: the fd is open for the call, the name is NUL-terminated and
    // the value is readable for its length.
    let rc = unsafe {
        libc::fsetxattr(
            file.as_raw_fd(),
            TRUST_XATTR.as_ptr(),
            v.as_ptr().cast(),
            v.len(),
            0,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked(dir: &Path) -> Option<PathBuf> {
        let path = dir.join("app.desktop");
        std::fs::write(&path, "[Desktop Entry]\nExec=true\n").unwrap();
        set_trusted(&path)
            .inspect_err(|e| eprintln!("SKIPPED: {} takes no user xattrs ({e})", dir.display()))
            .ok()?;
        Some(path)
    }

    fn raw(path: &Path) -> Vec<u8> {
        let c = cpath(path).unwrap();
        let mut buf = [0u8; 64];
        // SAFETY: both strings are NUL-terminated and `buf` is writable for its length.
        let n = unsafe {
            libc::getxattr(
                c.as_ptr(),
                TRUST_XATTR.as_ptr(),
                buf.as_mut_ptr().cast(),
                64,
            )
        };
        buf[..usize::try_from(n).unwrap()].to_vec()
    }

    fn put_raw(path: &Path, v: &[u8]) {
        let c = cpath(path).unwrap();
        // SAFETY: both strings are NUL-terminated and `v` is readable for its length.
        let rc = unsafe {
            libc::setxattr(
                c.as_ptr(),
                TRUST_XATTR.as_ptr(),
                v.as_ptr().cast(),
                v.len(),
                0,
            )
        };
        assert_eq!(rc, 0);
    }

    #[test]
    fn the_literal_true_marker_is_not_trusted() {
        let t = tempfile::tempdir().unwrap();
        let Some(path) = marked(t.path()) else { return };
        put_raw(&path, b"true");
        assert!(!is_trusted(&path));
    }

    #[test]
    fn a_marked_file_renamed_on_its_drive_stays_trusted() {
        let t = tempfile::tempdir().unwrap();
        let Some(path) = marked(t.path()) else { return };
        let moved = t.path().join("moved.desktop");
        std::fs::rename(&path, &moved).unwrap();
        assert!(is_trusted(&moved));
    }

    #[test]
    fn a_marker_copied_raw_onto_another_file_is_not_trusted() {
        let t = tempfile::tempdir().unwrap();
        let Some(path) = marked(t.path()) else { return };
        let other = t.path().join("other.desktop");
        std::fs::write(&other, "x").unwrap();
        put_raw(&other, &raw(&path));
        assert!(!is_trusted(&other));
    }

    #[test]
    fn another_key_does_not_verify_the_marker() {
        let t = tempfile::tempdir().unwrap();
        let Some(path) = marked(t.path()) else { return };
        assert!(is_trusted_with(&key().unwrap(), &path));
        assert!(!is_trusted_with(&[8; KEY_LEN], &path));
    }

    #[test]
    fn the_key_is_made_once_private_and_then_read_back() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("reprise/trust.key");
        let first = load_or_create_key(&path).unwrap();
        assert_eq!(load_or_create_key(&path).unwrap(), first);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(
            std::fs::read_dir(t.path().join("reprise")).unwrap().count(),
            1
        );
    }
}
