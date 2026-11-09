//! The display's MIT-MAGIC-COOKIE-1. Xwayland reads it with `-auth`, which turns off its
//! same-user local access, and programs find it through `XAUTHORITY`.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub const NAME: &[u8] = b"MIT-MAGIC-COOKIE-1";
/// Xauth's families: a local connection from the host named by the address, and any address.
const FAMILY_LOCAL: u16 = 256;
const FAMILY_WILD: u16 = 0xffff;

/// An authority file holding one random cookie for one display; dropping it removes the file.
pub struct Cookie {
    path: PathBuf,
    data: [u8; 16],
}

impl Cookie {
    /// Write a fresh cookie for `display` to `dir/xauth-<display>`, the file 0600 and `dir`
    /// 0700.
    pub fn write(dir: &Path, display: u32) -> io::Result<Cookie> {
        let mut data = [0u8; 16];
        // SAFETY: `data` is writable for its length.
        let n = unsafe { libc::getrandom(data.as_mut_ptr().cast(), data.len(), 0) };
        if n != data.len() as isize {
            return Err(io::Error::last_os_error());
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        let path = dir.join(format!("xauth-{display}"));
        let tmp = dir.join(format!(".xauth-{display}.{}", std::process::id()));
        let _ = fs::remove_file(&tmp);
        let written = (|| {
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp)?;
            f.write_all(&entry(FAMILY_LOCAL, &hostname(), display, &data))?;
            f.write_all(&entry(FAMILY_WILD, b"", display, &data))?;
            f.sync_all()?;
            fs::rename(&tmp, &path)
        })();
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        Ok(Cookie { path, data })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn data(&self) -> [u8; 16] {
        self.data
    }
}

fn hostname() -> Vec<u8> {
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is writable for its length; the name is NUL-terminated when it fits.
    if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } != 0 {
        return Vec::new();
    }
    buf.iter().take_while(|&&b| b != 0).copied().collect()
}

impl Drop for Cookie {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// One Xauthority entry: family, then address, display number, name and data length-prefixed,
/// all big-endian. Some clients (python-xlib) match only a local entry for their host.
fn entry(family: u16, address: &[u8], display: u32, data: &[u8]) -> Vec<u8> {
    let number = display.to_string();
    let mut out = family.to_be_bytes().to_vec();
    for field in [address, number.as_bytes(), NAME, data] {
        out.extend((field.len() as u16).to_be_bytes());
        out.extend(field);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// An entry's family, then its address, display number, name and data.
    type Entry = (u16, [Vec<u8>; 4]);

    /// The entries of an Xauthority file.
    fn entries(mut bytes: &[u8]) -> Vec<Entry> {
        let field = |b: &mut &[u8]| {
            let n = u16::from_be_bytes([b[0], b[1]]) as usize;
            let f = b[2..2 + n].to_vec();
            *b = &b[2 + n..];
            f
        };
        let mut out = Vec::new();
        while !bytes.is_empty() {
            let family = u16::from_be_bytes([bytes[0], bytes[1]]);
            bytes = &bytes[2..];
            out.push((family, std::array::from_fn(|_| field(&mut bytes))));
        }
        out
    }

    #[test]
    fn a_cookie_file_has_an_entry_for_this_host_and_a_wildcard_one() {
        let dir = crate::harness::scratch().join("xauth-entry");
        let c = Cookie::write(&dir, 7).unwrap();
        let host = fs::read_to_string("/proc/sys/kernel/hostname").unwrap();
        let host = host.trim_end().as_bytes().to_vec();
        let cookie = |family: u16, address: Vec<u8>| -> Entry {
            (
                family,
                [address, b"7".to_vec(), NAME.to_vec(), c.data().to_vec()],
            )
        };
        assert_eq!(
            entries(&fs::read(c.path()).unwrap()),
            [cookie(256, host), cookie(0xffff, Vec::new())],
            "FamilyLocal for clients that match only their host, then FamilyWild"
        );
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(c.path()), 0o600);
        assert_eq!(mode(&dir), 0o700);
    }

    #[test]
    fn each_cookie_is_fresh_and_its_file_goes_with_it() {
        let dir = crate::harness::scratch().join("xauth-fresh");
        let a = Cookie::write(&dir, 1).unwrap();
        let b = Cookie::write(&dir, 2).unwrap();
        assert_ne!(a.data(), b.data());
        let path = a.path().to_path_buf();
        drop(a);
        assert!(!path.exists());
        assert!(b.path().exists());
    }
}
