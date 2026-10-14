//! Reader for a theme directory's `icon-theme.cache`, the index `gtk-update-icon-cache` writes
//! (format 1.0, big-endian offsets into the file).

use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

pub const FILE: &str = "icon-theme.cache";

/// Suffix flags of one directory's image of a name.
pub const XPM: u16 = 1;
pub const SVG: u16 = 2;
pub const PNG: u16 = 4;

pub struct ThemeCache {
    bytes: Vec<u8>,
    hash: usize,
    buckets: u32,
    /// Directory index by path relative to the theme directory.
    dirs: HashMap<String, u16>,
    pub mtime: SystemTime,
}

impl ThemeCache {
    /// `theme_dir`'s cache, or None when it is missing, malformed, or older than the directory.
    pub fn open(theme_dir: &Path) -> Option<ThemeCache> {
        let bytes = std::fs::read(theme_dir.join(FILE)).ok()?;
        let mtime = std::fs::metadata(theme_dir.join(FILE))
            .ok()?
            .modified()
            .ok()?;
        if std::fs::metadata(theme_dir).ok()?.modified().ok()? > mtime {
            return None;
        }
        ThemeCache::parse(bytes, mtime)
    }

    pub fn parse(bytes: Vec<u8>, mtime: SystemTime) -> Option<ThemeCache> {
        let mut cache = ThemeCache {
            bytes,
            hash: 0,
            buckets: 0,
            dirs: HashMap::new(),
            mtime,
        };
        if (cache.u16_at(0)?, cache.u16_at(2)?) != (1, 0) {
            return None;
        }
        cache.hash = cache.u32_at(4)? as usize;
        cache.buckets = cache.u32_at(cache.hash)?;
        let list = cache.u32_at(8)? as usize;
        let n = cache.u32_at(list)?;
        if n > u16::MAX as u32 || cache.buckets == 0 {
            return None;
        }
        let mut dirs = HashMap::with_capacity(n as usize);
        for i in 0..n {
            let name = cache.str_at(cache.u32_at(list + 4 + 4 * i as usize)? as usize)?;
            dirs.insert(String::from_utf8(name.to_vec()).ok()?, i as u16);
        }
        cache.dirs = dirs;
        Some(cache)
    }

    /// The index of directory `rel`, or None when the cache saw no icons in it.
    pub fn dir(&self, rel: &str) -> Option<u16> {
        self.dirs.get(rel).copied()
    }

    /// Suffix flags of `name`'s images in directory `dir`; 0 when it has none or the cache is
    /// malformed past its header.
    pub fn flags(&self, name: &str, dir: u16) -> u16 {
        self.find(name.as_bytes(), dir).unwrap_or(0)
    }

    fn find(&self, name: &[u8], dir: u16) -> Option<u16> {
        let bucket = hash(name) % self.buckets;
        let mut icon = self.u32_at(self.hash + 4 + 4 * bucket as usize)?;
        // A chain longer than the file has icons is a loop.
        for _ in 0..self.bytes.len() / 12 {
            if icon == u32::MAX {
                return None;
            }
            let at = icon as usize;
            if self.str_at(self.u32_at(at + 4)? as usize)? == name {
                let list = self.u32_at(at + 8)? as usize;
                for i in 0..self.u32_at(list)? as usize {
                    let image = list + 4 + 8 * i;
                    if self.u16_at(image)? == dir {
                        return self.u16_at(image + 2);
                    }
                }
                return None;
            }
            icon = self.u32_at(at)?;
        }
        None
    }

    fn u16_at(&self, at: usize) -> Option<u16> {
        Some(u16::from_be_bytes(
            self.bytes.get(at..at.checked_add(2)?)?.try_into().ok()?,
        ))
    }

    fn u32_at(&self, at: usize) -> Option<u32> {
        Some(u32::from_be_bytes(
            self.bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
        ))
    }

    fn str_at(&self, at: usize) -> Option<&[u8]> {
        let tail = self.bytes.get(at..)?;
        Some(&tail[..tail.iter().position(|&b| b == 0)?])
    }
}

/// The writer's bucket hash, over the name's bytes as signed chars.
pub fn hash(name: &[u8]) -> u32 {
    let mut bytes = name.iter().map(|&b| b as i8 as i32 as u32);
    let Some(first) = bytes.next() else {
        return 0;
    };
    bytes.fold(first, |h, b| (h << 5).wrapping_sub(h).wrapping_add(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `gtk-update-icon-cache` output for a theme of 16x16/apps (a.png, b.svg, a.svgz,
    /// org.example.Long-name.png), 16x16@2/apps (a.png), scalable/apps (a.svg, c.xpm) and an
    /// empty 22x22/apps.
    const WRITTEN: &[u8] = &[
        0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00, 0xd0, 0x00, 0x00, 0x00,
        0x0b, 0x00, 0x00, 0x00, 0x3c, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0x00, 0x00, 0x00, 0x58, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x88, 0x00, 0x00, 0x00, 0xb4,
        0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x48, 0x00, 0x00, 0x00, 0x4c, 0x63, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff,
        0xff, 0xff, 0x00, 0x00, 0x00, 0x64, 0x00, 0x00, 0x00, 0x7c, 0x6f, 0x72, 0x67, 0x2e, 0x65,
        0x78, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x2e, 0x4c, 0x6f, 0x6e, 0x67, 0x2d, 0x6e, 0x61, 0x6d,
        0x65, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00,
        0x00, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x94, 0x00, 0x00, 0x00, 0x98, 0x61, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x02, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00,
        0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00, 0xc4, 0x62, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x03, 0x00, 0x00, 0x00, 0xe0, 0x00, 0x00, 0x00, 0xec, 0x00, 0x00, 0x00, 0xfc, 0x31,
        0x36, 0x78, 0x31, 0x36, 0x2f, 0x61, 0x70, 0x70, 0x73, 0x00, 0x00, 0x31, 0x36, 0x78, 0x31,
        0x36, 0x40, 0x32, 0x2f, 0x61, 0x70, 0x70, 0x73, 0x00, 0x00, 0x00, 0x00, 0x73, 0x63, 0x61,
        0x6c, 0x61, 0x62, 0x6c, 0x65, 0x2f, 0x61, 0x70, 0x70, 0x73, 0x00, 0x00, 0x00,
    ];

    fn written() -> ThemeCache {
        ThemeCache::parse(WRITTEN.to_vec(), SystemTime::UNIX_EPOCH).unwrap()
    }

    #[test]
    fn reads_the_directories_holding_icons() {
        let c = written();
        assert_eq!(c.dir("16x16/apps"), Some(0));
        assert_eq!(c.dir("16x16@2/apps"), Some(1));
        assert_eq!(c.dir("scalable/apps"), Some(2));
        assert_eq!(c.dir("22x22/apps"), None);
        assert_eq!(c.dir("16x16"), None);
    }

    #[test]
    fn reads_the_suffixes_per_name_and_directory() {
        let c = written();
        assert_eq!(c.flags("a", 0), PNG, "a.svgz is not an icon suffix");
        assert_eq!(c.flags("b", 0), SVG);
        assert_eq!(c.flags("org.example.Long-name", 0), PNG);
        assert_eq!(c.flags("a", 1), PNG);
        assert_eq!(c.flags("a", 2), SVG);
        assert_eq!(c.flags("c", 2), XPM);
        assert_eq!(c.flags("c", 0), 0);
        assert_eq!(c.flags("missing", 0), 0);
        assert_eq!(c.flags("", 0), 0);
    }

    #[test]
    fn a_damaged_cache_reads_as_absent_or_empty() {
        let mut other_version = WRITTEN.to_vec();
        other_version[1] = 2;
        assert!(ThemeCache::parse(other_version, SystemTime::UNIX_EPOCH).is_none());
        assert!(ThemeCache::parse(WRITTEN[..10].to_vec(), SystemTime::UNIX_EPOCH).is_none());
        for cut in 12..WRITTEN.len() {
            if let Some(c) = ThemeCache::parse(WRITTEN[..cut].to_vec(), SystemTime::UNIX_EPOCH) {
                for name in ["a", "b", "c", "missing"] {
                    let _ = c.flags(name, 0);
                }
            }
        }
        // Every bucket's chain pointing at itself.
        let mut looped = WRITTEN.to_vec();
        let c = written();
        for b in 0..c.buckets as usize {
            let at = c.hash + 4 + 4 * b;
            let icon = c.u32_at(at).unwrap();
            if icon != u32::MAX {
                looped[icon as usize..icon as usize + 4].copy_from_slice(&icon.to_be_bytes());
            }
        }
        let c = ThemeCache::parse(looped, SystemTime::UNIX_EPOCH).unwrap();
        assert_eq!(c.flags("missing", 0), 0);
    }
}
