//! A file's SHA-256 and MD5, read once for both, and telling a pasted hash apart from them.

use sha2::Digest;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const CHUNK: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sums {
    /// Lowercase hex.
    pub sha256: String,
    pub md5: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algo {
    Sha256,
    Md5,
}

impl Algo {
    pub fn label(self) -> &'static str {
        match self {
            Algo::Sha256 => "SHA-256",
            Algo::Md5 => "MD5",
        }
    }

    /// The algorithm whose hex digest is `len` characters long.
    fn of_len(len: usize) -> Option<Algo> {
        match len {
            64 => Some(Algo::Sha256),
            32 => Some(Algo::Md5),
            _ => None,
        }
    }
}

impl Sums {
    pub fn of(&self, algo: Algo) -> &str {
        match algo {
            Algo::Sha256 => &self.sha256,
            Algo::Md5 => &self.md5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Blank,
    Matches(Algo),
    NoMatch,
    /// Neither a SHA-256 nor an MD5 in hex.
    NotAHash,
}

impl Verdict {
    pub fn text(self) -> String {
        match self {
            Verdict::Blank => String::new(),
            Verdict::Matches(a) => format!("Matches {}", a.label()),
            Verdict::NoMatch => "No match".into(),
            Verdict::NotAHash => "Not a SHA-256 or MD5 hash".into(),
        }
    }
}

/// `typed` against `sums`: trimmed, any case, the algorithm told by its length.
pub fn compare(typed: &str, sums: &Sums) -> Verdict {
    let typed = typed.trim();
    if typed.is_empty() {
        return Verdict::Blank;
    }
    let algo = match Algo::of_len(typed.len()) {
        Some(a) if typed.bytes().all(|b| b.is_ascii_hexdigit()) => a,
        _ => return Verdict::NotAHash,
    };
    match typed.eq_ignore_ascii_case(sums.of(algo)) {
        true => Verdict::Matches(algo),
        false => Verdict::NoMatch,
    }
}

/// Shared between a hashing thread and the window waiting on it.
#[derive(Debug, Default)]
pub struct Meter {
    /// Bytes read so far.
    pub done: AtomicU64,
    pub cancel: AtomicBool,
}

/// The sums of `path`, `None` once `meter` is cancelled. `tick` runs after every chunk read.
pub fn sums_of(
    path: &Path,
    meter: &Meter,
    mut tick: impl FnMut(),
) -> std::io::Result<Option<Sums>> {
    let mut file = std::fs::File::open(path)?;
    let mut sha = sha2::Sha256::new();
    let mut md5 = md5::Context::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        if meter.cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        sha.update(&buf[..n]);
        md5.consume(&buf[..n]);
        meter.done.fetch_add(n as u64, Ordering::Relaxed);
        tick();
    }
    Ok(Some(Sums {
        sha256: hex(&sha.finalize()),
        md5: hex(&md5.finalize().0),
    }))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const ABC_MD5: &str = "900150983cd24fb0d6963f7d28e17f72";

    fn abc() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abc.txt");
        std::fs::write(&path, b"abc").unwrap();
        (dir, path)
    }

    #[test]
    fn sums_of_a_file_are_its_sha256_and_md5() {
        let (_dir, path) = abc();
        let meter = Meter::default();
        let sums = sums_of(&path, &meter, || {}).unwrap().unwrap();
        assert_eq!(sums.sha256, ABC_SHA256);
        assert_eq!(sums.md5, ABC_MD5);
        assert_eq!(meter.done.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn an_empty_file_has_the_empty_sums() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty");
        std::fs::write(&path, b"").unwrap();
        let sums = sums_of(&path, &Meter::default(), || {}).unwrap().unwrap();
        assert_eq!(
            sums.sha256,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(sums.md5, "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn a_file_over_several_chunks_hashes_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big");
        let data: Vec<u8> = (0..CHUNK * 2 + 17).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        let mut ticks = 0;
        let sums = sums_of(&path, &Meter::default(), || ticks += 1)
            .unwrap()
            .unwrap();
        assert_eq!(sums.sha256, hex(&sha2::Sha256::digest(&data)));
        assert_eq!(sums.md5, hex(&md5::compute(&data).0));
        assert_eq!(ticks, 3, "one tick per chunk read");
    }

    #[test]
    fn a_cancelled_meter_stops_the_read() {
        let (_dir, path) = abc();
        let meter = Meter::default();
        meter.cancel.store(true, Ordering::Relaxed);
        assert_eq!(sums_of(&path, &meter, || {}).unwrap(), None);
        assert_eq!(meter.done.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_missing_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(sums_of(&dir.path().join("gone"), &Meter::default(), || {}).is_err());
    }

    #[test]
    fn compare_tells_the_algorithm_by_length_in_any_case_trimmed() {
        let sums = Sums {
            sha256: ABC_SHA256.into(),
            md5: ABC_MD5.into(),
        };
        let upper = format!("  {}\n", ABC_SHA256.to_uppercase());
        assert_eq!(compare(&upper, &sums), Verdict::Matches(Algo::Sha256));
        assert_eq!(compare(ABC_MD5, &sums), Verdict::Matches(Algo::Md5));
        assert_eq!(compare(&"0".repeat(64), &sums), Verdict::NoMatch);
        assert_eq!(compare(&"0".repeat(32), &sums), Verdict::NoMatch);
        assert_eq!(compare("abc", &sums), Verdict::NotAHash);
        assert_eq!(compare(&"g".repeat(32), &sums), Verdict::NotAHash);
        assert_eq!(compare("   ", &sums), Verdict::Blank);
        assert_eq!(Verdict::Matches(Algo::Sha256).text(), "Matches SHA-256");
        assert_eq!(Verdict::NoMatch.text(), "No match");
    }
}
