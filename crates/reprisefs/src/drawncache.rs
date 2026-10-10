//! Entries are named by everything that changes their pixels; a name is never rewritten with other pixels.
//! An entry is `RPW1`, the width and height as little-endian u32, then
//! the opaque pixels as RGB bytes: no compression to undo on the way in.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const MAGIC: &[u8; 4] = b"RPW1";
const HEADER: usize = 12;

pub const MAX_BYTES: u64 = 256 << 20;

pub fn default_dir() -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(cache.join("reprise").join("wallpapers"))
}

/// Entry `name` in `dir` when it holds a `w` x `h` picture, as opaque
/// ARGB pixels; its use time is bumped so trimming keeps it.
pub fn load(dir: &Path, name: &str, w: u32, h: u32) -> Option<Vec<u32>> {
    let path = dir.join(name);
    let bytes = std::fs::read(&path).ok()?;
    let n = w as usize * h as usize;
    if bytes.len() != HEADER + n * 3
        || &bytes[..4] != MAGIC
        || bytes[4..8] != w.to_le_bytes()
        || bytes[8..12] != h.to_le_bytes()
    {
        return None;
    }
    if let Ok(f) = std::fs::File::options().write(true).open(&path) {
        let _ = f.set_modified(SystemTime::now());
    }
    Some(
        bytes[HEADER..]
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| 0xFF00_0000 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32)
            .collect(),
    )
}

pub fn store(dir: &Path, name: &str, w: u32, h: u32, pixels: &[u32]) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut bytes = Vec::with_capacity(HEADER + pixels.len() * 3);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&w.to_le_bytes());
    bytes.extend_from_slice(&h.to_le_bytes());
    for p in pixels {
        bytes.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
    }
    crate::ops::write_atomic(&dir.join(name), &bytes)?;
    trim(dir, MAX_BYTES, name);
    Ok(())
}

pub fn get_or_make(
    dir: Option<&Path>,
    name: &str,
    w: u32,
    h: u32,
    make: impl FnOnce() -> Vec<u32>,
) -> Vec<u32> {
    if let Some(px) = dir.and_then(|d| load(d, name, w, h)) {
        return px;
    }
    let px = make();
    if let Some(d) = dir
        && let Err(e) = store(d, name, w, h, &px)
    {
        log::warn!("wallpaper cache {}: {e}", d.display());
    }
    px
}

fn trim(dir: &Path, max: u64, keep: &str) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<(SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok().filter(|m| m.is_file())?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = entries.iter().map(|e| e.1).sum();
    entries.sort();
    for (_, len, path) in entries {
        if total <= max {
            break;
        }
        if path.file_name().is_some_and(|n| n == keep) {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_picture_loads_back_at_its_size_only() {
        let dir = tempfile::tempdir().unwrap();
        let px = vec![0xFF12_3456, 0xFFAB_CDEF, 0xFF00_0000, 0xFFFF_FFFF];
        store(dir.path(), "a", 2, 2, &px).unwrap();
        assert_eq!(load(dir.path(), "a", 2, 2), Some(px));
        assert_eq!(load(dir.path(), "a", 4, 1), None);
        assert_eq!(load(dir.path(), "b", 2, 2), None);
    }

    #[test]
    fn get_or_make_draws_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut drawn = 0;
        for _ in 0..2 {
            let px = get_or_make(Some(dir.path()), "x", 1, 1, || {
                drawn += 1;
                vec![0xFF01_0203]
            });
            assert_eq!(px, vec![0xFF01_0203]);
        }
        assert_eq!(drawn, 1);
        get_or_make(None, "x", 1, 1, || {
            drawn += 1;
            vec![0xFF01_0203]
        });
        assert_eq!(drawn, 2);
    }

    #[test]
    fn trimming_drops_the_oldest_and_keeps_the_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let old = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        for (i, name) in ["old", "mid", "new"].iter().enumerate() {
            std::fs::write(d.join(name), [0u8; 100]).unwrap();
            let f = std::fs::File::options()
                .write(true)
                .open(d.join(name))
                .unwrap();
            f.set_modified(old + std::time::Duration::from_secs(i as u64))
                .unwrap();
        }
        trim(d, 200, "old");
        assert!(d.join("old").exists());
        assert!(!d.join("mid").exists());
        assert!(d.join("new").exists());
    }
}
