//! The display's MIT-MAGIC-COOKIE-1. Xwayland reads it with `-auth`, which turns off its
//! same-user local access, and programs find it through `XAUTHORITY`.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub const NAME: &[u8] = b"MIT-MAGIC-COOKIE-1";
/// Xauth's family for an entry that matches any address.
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
            f.write_all(&entry(display, &data))?;
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

impl Drop for Cookie {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// One Xauthority entry: family, address, display number, name and data, each but the family
/// length-prefixed, all big-endian. A wildcard entry with the display's number is what Flatpak
/// copies into an app with X11 access.
fn entry(display: u32, data: &[u8]) -> Vec<u8> {
    let number = display.to_string();
    let mut out = FAMILY_WILD.to_be_bytes().to_vec();
    for field in [&[][..], number.as_bytes(), NAME, data] {
        out.extend((field.len() as u16).to_be_bytes());
        out.extend(field);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_cookie_file_is_one_wildcard_entry_for_its_display() {
        let dir = crate::harness::scratch().join("xauth-entry");
        let c = Cookie::write(&dir, 7).unwrap();
        let bytes = fs::read(c.path()).unwrap();
        let mut expect = vec![0xff, 0xff, 0, 0, 0, 1, b'7', 0, 18];
        expect.extend(NAME);
        expect.extend([0, 16]);
        expect.extend(c.data());
        assert_eq!(bytes, expect);
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
