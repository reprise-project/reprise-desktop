//! The trust marker `user.reprise.trusted`: an HMAC of the file's filesystem id, inode and
//! contents under a per-user key (docs/security.md).
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub(crate) const TRUST_XATTR: &std::ffi::CStr = c"user.reprise.trusted";
const KEY_LEN: usize = 32;
/// HMAC-SHA256's output.
const MARKER_LEN: usize = 32;
type Key = [u8; KEY_LEN];

/// The largest file the trust check hashes, in bytes. A bigger one is never trusted, so opening
/// it always asks.
pub const TRUST_HASH_MAX: u64 = 128 << 20;

/// What the marker covers: the file's `f_fsid` and inode, and the SHA-256 of its contents.
/// st_dev is left out: btrfs subvolumes and NVMe or dm devices can get another number at each
/// boot, while f_fsid follows the filesystem's UUID.
#[derive(Debug, PartialEq, Eq)]
struct Ident {
    fsid: libc::c_ulong,
    ino: u64,
    contents: [u8; 32],
}

fn mac(key: &Key, id: &Ident) -> Hmac<Sha256> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    m.update(format!("{:x}:{}:", id.fsid, id.ino).as_bytes());
    m.update(&id.contents);
    m
}

/// The regular file at `path` (followed through a symlink), open to read; the open does not
/// block on a FIFO.
fn open(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
}

fn ident(path: &Path) -> io::Result<Ident> {
    ident_fd(&open(path)?)
}

/// The [`Ident`] of the open `file`, whose contents are read from the start without moving its
/// offset. Reads the whole file, which must be regular and at most [`TRUST_HASH_MAX`].
fn ident_fd(file: &std::fs::File) -> io::Result<Ident> {
    use std::os::unix::fs::FileExt;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    if meta.len() > TRUST_HASH_MAX {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            "too large to hash for trust",
        ));
    }
    // SAFETY: an all-zero statvfs is a valid value of a plain C struct.
    let mut buf: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: the fd is open and `buf` is writable across the call.
    if unsafe { libc::fstatvfs(file.as_raw_fd(), &mut buf) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut hash = Sha256::new();
    let mut chunk = vec![0u8; 64 * 1024];
    let mut at = 0;
    loop {
        match file.read_at(&mut chunk, at) {
            Ok(0) => break,
            Ok(n) => {
                hash.update(&chunk[..n]);
                at += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(Ident {
        fsid: buf.f_fsid,
        ino: meta.ino(),
        contents: hash.finalize().into(),
    })
}

fn value(key: &Key, id: &Ident) -> Vec<u8> {
    mac(key, id).finalize().into_bytes().to_vec()
}

/// Whether `value` is the marker for the file at `path`.
pub(crate) fn verifies(value: &[u8], path: &Path) -> bool {
    key().is_ok_and(|k| ident(path).is_ok_and(|id| verifies_with(&k, value, &id)))
}

fn verifies_with(key: &Key, value: &[u8], id: &Ident) -> bool {
    mac(key, id).verify_slice(value).is_ok()
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

#[cfg(test)]
fn cpath(path: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path holds a NUL"))
}

/// Whether the file at `path` (followed through a symlink) is one the user allowed to run, as it
/// was then. A marked file is read whole, so a host calls this off its event loop.
pub fn is_trusted(path: &Path) -> bool {
    key().is_ok_and(|k| is_trusted_with(&k, path))
}

fn is_trusted_with(key: &Key, path: &Path) -> bool {
    let Ok(file) = open(path) else {
        return false;
    };
    let Some(marker) = marker_of(&file) else {
        return false;
    };
    ident_fd(&file).is_ok_and(|id| verifies_with(key, &marker, &id))
}

/// The open `file`'s marker, None when it has none of the HMAC's length.
fn marker_of(file: &std::fs::File) -> Option<Vec<u8>> {
    let mut buf = [0u8; 64];
    // SAFETY: the fd is open, the attribute name is NUL-terminated and `buf` is writable for
    // its length, all live across the call.
    let n = unsafe {
        libc::fgetxattr(
            file.as_raw_fd(),
            TRUST_XATTR.as_ptr(),
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    };
    let n = usize::try_from(n).ok().filter(|&n| n == MARKER_LEN)?;
    Some(buf[..n].to_vec())
}

/// Mark the file at `path` trusted as it is now; any later change to its contents drops it.
pub fn set_trusted(path: &Path) -> io::Result<()> {
    let file = open(path)?;
    mark(&file, &file)
}

/// Allow the `.desktop` file at `path` to launch: the execute bit and the trust marker.
pub fn allow_launching(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)?.permissions().mode();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode | 0o111))?;
    set_trusted(path)
}

/// [`set_trusted`] for an open `file`, which may be open for writing only; its contents are read
/// through a second, read-only open of the same file.
pub(crate) fn set_trusted_fd(file: &std::fs::File) -> io::Result<()> {
    let again = open(Path::new(&format!("/proc/self/fd/{}", file.as_raw_fd())))?;
    mark(&again, file)
}

/// Mark `target` with the marker of `read`, an open of the same file to read.
fn mark(read: &std::fs::File, target: &std::fs::File) -> io::Result<()> {
    let v = value(&key()?, &ident_fd(read)?);
    // SAFETY: the fd is open for the call, the name is NUL-terminated and
    // the value is readable for its length.
    let rc = unsafe {
        libc::fsetxattr(
            target.as_raw_fd(),
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
    fn editing_a_trusted_file_in_place_drops_its_trust() {
        use std::os::unix::fs::MetadataExt;
        let t = tempfile::tempdir().unwrap();
        let Some(path) = marked(t.path()) else { return };
        let ino = std::fs::metadata(&path).unwrap().ino();
        std::fs::write(&path, "[Desktop Entry]\nExec=false\n").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().ino(),
            ino,
            "the same inode"
        );
        assert!(!is_trusted(&path));
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
    fn the_marker_covers_the_filesystem_id_the_inode_and_the_contents() {
        let k = key().unwrap();
        let id = |fsid, ino, contents| Ident {
            fsid,
            ino,
            contents,
        };
        let v = value(&k, &id(0x1234_5678_9abc_def0, 42, [1; 32]));
        assert!(verifies_with(
            &k,
            &v,
            &id(0x1234_5678_9abc_def0, 42, [1; 32])
        ));
        assert!(!verifies_with(
            &k,
            &v,
            &id(0x1234_5678_9abc_def1, 42, [1; 32])
        ));
        assert!(!verifies_with(
            &k,
            &v,
            &id(0x1234_5678_9abc_def0, 43, [1; 32])
        ));
        assert!(!verifies_with(
            &k,
            &v,
            &id(0x1234_5678_9abc_def0, 42, [2; 32])
        ));
    }

    #[test]
    fn a_marked_file_verifies_through_its_path_and_its_fd() {
        let t = tempfile::tempdir().unwrap();
        let Some(path) = marked(t.path()) else { return };
        let file = std::fs::File::open(&path).unwrap();
        assert_eq!(ident(&path).unwrap(), ident_fd(&file).unwrap());
        assert!(verifies(&raw(&path), &path));
    }

    #[test]
    fn another_key_does_not_verify_the_marker() {
        let t = tempfile::tempdir().unwrap();
        let Some(path) = marked(t.path()) else { return };
        assert!(is_trusted_with(&key().unwrap(), &path));
        assert!(!is_trusted_with(&[8; KEY_LEN], &path));
    }

    #[test]
    fn a_file_past_the_hash_cap_is_never_trusted_even_with_a_right_marker() {
        let t = tempfile::tempdir().unwrap();
        let Some(path) = marked(t.path()) else { return };
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(TRUST_HASH_MAX + 1).unwrap();
        let mut hash = Sha256::new();
        hash.update(b"[Desktop Entry]\nExec=true\n");
        let zeros = vec![0u8; 1 << 20];
        let mut left = TRUST_HASH_MAX + 1 - 26;
        while left > 0 {
            let n = left.min(zeros.len() as u64) as usize;
            hash.update(&zeros[..n]);
            left -= n as u64;
        }
        // SAFETY: an all-zero statvfs is a valid value of a plain C struct.
        let mut buf: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: the fd is open and `buf` is writable across the call.
        assert_eq!(unsafe { libc::fstatvfs(file.as_raw_fd(), &mut buf) }, 0);
        let id = Ident {
            fsid: buf.f_fsid,
            ino: std::fs::metadata(&path).unwrap().ino(),
            contents: hash.finalize().into(),
        };
        put_raw(&path, &value(&key().unwrap(), &id));
        assert!(!is_trusted(&path));
        assert!(set_trusted(&path).is_err(), "nor can it be marked");
    }

    #[test]
    fn an_unmarked_large_file_is_untrusted_without_being_read() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("big.AppImage");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(16 << 30)
            .unwrap();
        let at = std::time::Instant::now();
        assert!(!is_trusted(&path));
        assert!(
            at.elapsed() < std::time::Duration::from_millis(500),
            "answered in {:?}",
            at.elapsed()
        );
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
