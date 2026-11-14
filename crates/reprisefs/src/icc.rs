//! ICC display profiles: the header, the description and the `vcgt` calibration curves a profile
//! carries for the video card's gamma table. Only the tags asked for are read, each under a size
//! cap, so a large or broken file costs a few small reads and an error.

use std::fs::File;
use std::io;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// The largest profile file read at all.
pub const MAX_FILE: u64 = 64 << 20;
/// More tags than any profile carries; a larger count is a broken header.
const MAX_TAGS: u32 = 256;
const MAX_DESC: u32 = 64 << 10;
/// Three channels of 65535 two-byte entries and the table's header.
const MAX_VCGT: u32 = 18 + 3 * 65535 * 2;
const HEADER: usize = 128;
/// How many files a profile folder listing reads.
const MAX_LISTED: usize = 512;

/// A profile's calibration curves: device value in, device value out, both 0 to 1.
#[derive(Clone, Debug, PartialEq)]
pub enum Vcgt {
    /// `entries` points per channel, spread evenly from 0 to 1, in red, green, blue order.
    Table { entries: usize, data: Vec<f64> },
    /// Per channel `min + (max - min) * x^gamma`.
    Formula([Formula; 3]),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Formula {
    pub gamma: f64,
    pub min: f64,
    pub max: f64,
}

impl Vcgt {
    /// Channel `c`'s (0 red, 1 green, 2 blue) output for input `x`, linear between table points,
    /// clamped to 0..=1.
    pub fn value(&self, c: usize, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        let v = match self {
            Vcgt::Table { entries, data } => {
                let Some(row) = data
                    .get(c.min(2) * entries..)
                    .and_then(|d| d.get(..*entries))
                else {
                    return x;
                };
                if row.len() < 2 {
                    return x;
                }
                let entries = &row.len();
                let at = x * (entries - 1) as f64;
                let i = (at.floor() as usize).min(entries - 2);
                let f = at - i as f64;
                row[i] + (row[i + 1] - row[i]) * f
            }
            Vcgt::Formula(f) => {
                let f = f[c.min(2)];
                f.min + (f.max - f.min) * x.powf(f.gamma)
            }
        };
        v.clamp(0.0, 1.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// The profile's own name, empty when it states none.
    pub description: String,
    /// The device class is a display (`mntr`).
    pub display: bool,
    pub vcgt: Option<Vcgt>,
}

/// Where a profile's bytes come from: a file, or memory in tests.
pub trait Source {
    fn size(&self) -> u64;
    /// Exactly `buf.len()` bytes from `offset`.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
}

impl Source for [u8] {
    fn size(&self) -> u64 {
        self.len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let start = usize::try_from(offset).map_err(|_| short())?;
        let end = start.checked_add(buf.len()).ok_or_else(short)?;
        buf.copy_from_slice(self.get(start..end).ok_or_else(short)?);
        Ok(())
    }
}

struct FileAt<'a>(&'a File, u64);

impl Source for FileAt<'_> {
    fn size(&self) -> u64 {
        self.1
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.0.read_exact_at(buf, offset)
    }
}

fn short() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "cut short")
}

fn bad(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_string())
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn s15f16(b: &[u8], at: usize) -> f64 {
    f64::from(be32(b, at) as i32) / 65536.0
}

/// A tag table entry: signature, offset, size.
type Tag = ([u8; 4], u32, u32);

/// The header and the tag table (signature, offset, size); Err when a tag runs past the profile's
/// stated size or the file's end.
fn tags(src: &(impl Source + ?Sized)) -> io::Result<(Vec<u8>, Vec<Tag>)> {
    let len = src.size();
    if len > MAX_FILE {
        return Err(bad("larger than an ICC profile"));
    }
    let mut head = vec![0; HEADER + 4];
    src.read_at(0, &mut head)?;
    if &head[36..40] != b"acsp" {
        return Err(bad("not an ICC profile"));
    }
    if !matches!(head[8], 2 | 4) {
        return Err(bad("an ICC version other than 2 or 4"));
    }
    let size = u64::from(be32(&head, 0)).min(len);
    let count = be32(&head, HEADER);
    if count > MAX_TAGS {
        return Err(bad("too many tags"));
    }
    let mut table = vec![0; count as usize * 12];
    src.read_at(HEADER as u64 + 4, &mut table)?;
    let tags: Vec<_> = table
        .as_chunks::<12>()
        .0
        .iter()
        .map(|t| ([t[0], t[1], t[2], t[3]], be32(t, 4), be32(t, 8)))
        .collect();
    if tags
        .iter()
        .any(|&(_, at, n)| u64::from(at) + u64::from(n) > size)
    {
        return Err(bad("a tag past the profile's end"));
    }
    Ok((head, tags))
}

/// Tag `sig`'s bytes, None when absent; Err when it is larger than `cap`.
fn tag(
    src: &(impl Source + ?Sized),
    tags: &[Tag],
    sig: &[u8; 4],
    cap: u32,
) -> io::Result<Option<Vec<u8>>> {
    let Some(&(_, at, n)) = tags.iter().find(|(s, ..)| s == sig) else {
        return Ok(None);
    };
    if n > cap {
        return Err(bad("a tag larger than it can be"));
    }
    let mut b = vec![0; n as usize];
    src.read_at(u64::from(at), &mut b)?;
    Ok(Some(b))
}

/// The description: a v2 `desc` ASCII text, or a v4 `mluc` with US English first, else its first
/// record.
fn description(b: &[u8]) -> String {
    let text = |s: &[u8]| {
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        String::from_utf8_lossy(&s[..end]).trim().to_string()
    };
    match b.get(..4) {
        Some(b"desc") if b.len() >= 12 => {
            let n = be32(b, 8) as usize;
            b.get(12..12usize.saturating_add(n))
                .map(text)
                .unwrap_or_default()
        }
        Some(b"mluc") if b.len() >= 16 => {
            let (count, size) = (be32(b, 8) as usize, be32(b, 12) as usize);
            if size < 12 {
                return String::new();
            }
            let records: Vec<(&[u8], usize, usize)> = (0..count.min(b.len() / size))
                .filter_map(|i| {
                    let r = b.get(16 + i * size..)?.get(..12)?;
                    Some((&r[..4], be32(r, 4) as usize, be32(r, 8) as usize))
                })
                .collect();
            let pick = (records.iter().find(|r| r.0 == b"enUS")).or(records.first());
            let Some(&(_, n, at)) = pick else {
                return String::new();
            };
            let Some(s) = b.get(at..at.saturating_add(n)) else {
                return String::new();
            };
            let units: Vec<u16> = s
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&c| u16::from_be_bytes(c))
                .collect();
            let s = String::from_utf16_lossy(&units);
            s.trim_end_matches('\0').trim().to_string()
        }
        _ => String::new(),
    }
}

fn vcgt(b: &[u8]) -> io::Result<Vcgt> {
    if b.len() < 12 || &b[..4] != b"vcgt" {
        return Err(bad("vcgt of another type"));
    }
    match be32(b, 8) {
        0 => {
            if b.len() < 18 {
                return Err(short());
            }
            let (channels, entries, width) = (
                usize::from(be16(b, 12)),
                usize::from(be16(b, 14)),
                usize::from(be16(b, 16)),
            );
            if !matches!(channels, 1 | 3) || entries < 2 || !matches!(width, 1 | 2) {
                return Err(bad("a vcgt table of an unknown shape"));
            }
            let body = (b.get(18..18 + channels * entries * width)).ok_or_else(short)?;
            let max = if width == 1 { 255.0 } else { 65535.0 };
            let read: Vec<f64> = body
                .chunks_exact(width)
                .map(|c| match c {
                    [v] => f64::from(*v) / max,
                    _ => f64::from(be16(c, 0)) / max,
                })
                .collect();
            let data = if channels == 1 { read.repeat(3) } else { read };
            Ok(Vcgt::Table { entries, data })
        }
        1 => {
            let body = b.get(12..48).ok_or_else(short)?;
            let f = |c: usize| Formula {
                gamma: s15f16(body, c * 12),
                min: s15f16(body, c * 12 + 4),
                max: s15f16(body, c * 12 + 8),
            };
            let all = [f(0), f(1), f(2)];
            let sane = |f: &Formula| {
                (0.01..=10.0).contains(&f.gamma)
                    && (0.0..=1.0).contains(&f.min)
                    && (0.0..=1.0).contains(&f.max)
            };
            if !all.iter().all(sane) {
                return Err(bad("a vcgt formula out of range"));
            }
            Ok(Vcgt::Formula(all))
        }
        _ => Err(bad("a vcgt of an unknown type")),
    }
}

fn parse_from(src: &(impl Source + ?Sized), curves: bool) -> io::Result<Profile> {
    let (head, tags) = tags(src)?;
    let description = (tag(src, &tags, b"desc", MAX_DESC)?)
        .map(|b| description(&b))
        .unwrap_or_default();
    let vcgt = if curves {
        tag(src, &tags, b"vcgt", MAX_VCGT)?
            .map(|b| vcgt(&b))
            .transpose()?
    } else {
        None
    };
    Ok(Profile {
        description,
        display: &head[12..16] == b"mntr",
        vcgt,
    })
}

/// The profile in `bytes`.
pub fn parse(bytes: &[u8]) -> io::Result<Profile> {
    parse_from(bytes, true)
}

fn open(path: &Path) -> io::Result<(File, u64)> {
    // Non-blocking, so a FIFO in a profile folder fails at once instead of waiting for a writer.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(bad("not a file"));
    }
    Ok((file, meta.len()))
}

/// The profile at `path`, with its calibration curves.
pub fn read(path: &Path) -> io::Result<Profile> {
    let (file, len) = open(path)?;
    parse_from(&FileAt(&file, len), true)
}

/// The profile at `path` without its curves: enough to list it.
pub fn read_head(path: &Path) -> io::Result<Profile> {
    let (file, len) = open(path)?;
    parse_from(&FileAt(&file, len), false)
}

/// The folders display profiles are kept in: the user's (where colord imports them too), the
/// system's and colord's own store.
pub fn dirs() -> Vec<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    (data.map(|d| d.join("icc")).into_iter())
        .chain([
            PathBuf::from("/usr/share/color/icc"),
            PathBuf::from("/var/lib/colord/icc"),
        ])
        .collect()
}

/// A profile as a picker lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub path: PathBuf,
    /// The description, else the file name.
    pub name: String,
}

/// The display profiles (`.icc`, `.icm`) in `dirs` and two folder levels below, plus `extra`
/// files; each path once, sorted by name.
pub fn list(dirs: &[PathBuf], extra: &[PathBuf]) -> Vec<Listed> {
    let mut files = Vec::new();
    for d in dirs {
        walk(d, 0, &mut files);
    }
    files.extend(extra.iter().cloned());
    let mut out: Vec<Listed> = Vec::new();
    for path in files.into_iter().take(MAX_LISTED) {
        if out.iter().any(|l| l.path == path) {
            continue;
        }
        let Ok(p) = read_head(&path) else {
            continue;
        };
        if !p.display {
            continue;
        }
        let name = if p.description.is_empty() {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            p.description
        };
        out.push(Listed { path, name });
    }
    out.sort_by_key(|l| l.name.to_lowercase());
    out
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if out.len() >= MAX_LISTED {
            return;
        }
        if p.is_dir() {
            if depth < 2 {
                walk(&p, depth + 1, out);
            }
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("icc") || e.eq_ignore_ascii_case("icm"))
        {
            out.push(p);
        }
    }
}

#[cfg(test)]
mod tests;
