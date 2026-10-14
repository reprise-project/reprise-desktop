//! The shared thumbnail cache of the freedesktop Thumbnail Managing Standard, so thumbnails other file managers made serve Reprise too. This side names, checks, writes and trims entries; decoding the picture and the entry's pixels is the caller's.
//!
//! One PNG per picture per size under `$XDG_CACHE_HOME/thumbnails` (or `~/.cache/thumbnails`): `normal/`, `large/`, `x-large/` and `xx-large/` hold thumbnails fitted inside 128, 256, 512 and 1024 px squares, named by the MD5 of the picture's `file://` URI. An entry is trusted only when its `Thumb::URI` and `Thumb::MTime` text (and `Thumb::Size` when present) match the picture as it stands.
//!
//! A picture the caller could read but not decode gets a 1x1 PNG with the same keys under `fail/<program>-<version>/`, so the next visit skips it until the file changes. Only the program's own failures are read.
//!
//! Pictures inside the cache, and pictures that already fit the size, are not stored. Directories are 0700 and entries 0600; an entry lands by renaming a temp file beside it, so readers never see half of one.
//!
//! The size directories and the program's failures are held under a cap: past it, the least recently used entries go (a hit touches its file's mtime), whichever program wrote them. The size is re-measured before a trim, since other programs write there too.

use std::fs::Metadata;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const SIZES: [(&str, u32); 4] = [
    ("normal", 128),
    ("large", 256),
    ("x-large", 512),
    ("xx-large", 1024),
];

pub fn edge_for(px: u32) -> u32 {
    SIZES
        .iter()
        .map(|&(_, edge)| edge)
        .find(|&edge| edge >= px)
        .unwrap_or(SIZES[3].1)
}

/// Bytes past which a file is not a thumbnail fitted inside an `edge`
/// square: its raw pixels plus a generous allowance for the text. A
/// larger file is a miss without being read in.
fn max_entry(edge: u32) -> u64 {
    u64::from(edge) * u64::from(edge) * 4 + (64 << 10)
}

/// A trim goes down to this share of the cap, so a full cache is not
/// trimmed again on every single write.
const TRIM_TO: (u64, u64) = (9, 10);

pub struct Hit {
    /// The entry's PNG. Its text has been checked; its pixels have not,
    /// so decode them with a cap of the size asked for.
    pub png: Vec<u8>,
    /// The picture's own size, from `Thumb::Image::Width` and `Height`,
    /// which the standard leaves optional.
    pub dims: Option<(u32, u32)>,
}

pub struct Rgba<'a> {
    pub width: u32,
    pub height: u32,
    pub pixels: &'a [u8],
}

pub struct Thumbnails {
    root: PathBuf,
    /// `<program>-<version>`: the `fail/` subdirectory, the PNG
    /// `Software` key and the temp file prefix.
    program: String,
    max_bytes: u64,
    /// What the trimmed directories hold, in bytes: measured on the first
    /// write, then kept up to date by this process's own writes.
    used: Option<u64>,
    seq: u64,
}

impl Thumbnails {
    pub fn at(root: PathBuf, program: &str, max_bytes: u64) -> Thumbnails {
        Thumbnails {
            root,
            program: program.to_string(),
            max_bytes,
            used: None,
            seq: 0,
        }
    }

    pub fn default_dir() -> Option<PathBuf> {
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
        Some(cache.join("thumbnails"))
    }

    fn named(&self, dir: &Path, path: &Path) -> Option<PathBuf> {
        if !path.is_absolute() || path.starts_with(&self.root) {
            return None;
        }
        Some(dir.join(format!("{:x}.png", md5::compute(file_uri(path)))))
    }

    fn entry(&self, path: &Path, edge: u32) -> Option<PathBuf> {
        let (dir, _) = SIZES.iter().find(|&&(_, e)| e == edge)?;
        self.named(&self.root.join(dir), path)
    }

    fn fail_dir(&self) -> PathBuf {
        self.root.join("fail").join(&self.program)
    }

    fn fail_entry(&self, path: &Path) -> Option<PathBuf> {
        self.named(&self.fail_dir(), path)
    }

    /// `path`'s thumbnail fitted inside an `edge` square, if one is on
    /// disk for the file `meta` describes. A hit counts as a use for the
    /// cap.
    pub fn get(&self, path: &Path, meta: &Metadata, edge: u32) -> Option<Hit> {
        let (png, text) = read_valid(&self.entry(path, edge)?, path, meta, max_entry(edge))?;
        let key = |k: &str| key(&text, k).and_then(|v| v.parse().ok());
        let dims = key("Thumb::Image::Width").zip(key("Thumb::Image::Height"));
        Some(Hit { png, dims })
    }

    /// Whether this program failed to thumbnail `path` as `meta`
    /// describes it. A hit counts as a use for the cap.
    pub fn failed(&self, path: &Path, meta: &Metadata) -> bool {
        self.fail_entry(path)
            .and_then(|e| read_valid(&e, path, meta, max_entry(1)))
            .is_some()
    }

    /// Keep `thumb`, fitted inside an `edge` square, as the thumbnail of
    /// the `dims` picture at `path` that `meta` describes. A failure is
    /// logged, not fatal: the next visit decodes again.
    pub fn put(&mut self, path: &Path, meta: &Metadata, edge: u32, thumb: Rgba, dims: (u32, u32)) {
        if dims.0 <= edge && dims.1 <= edge {
            return;
        }
        let Some(entry) = self.entry(path, edge) else {
            return;
        };
        let text = [
            ("Thumb::Image::Width", dims.0.to_string()),
            ("Thumb::Image::Height", dims.1.to_string()),
        ];
        self.store(&entry, path, meta, thumb, &text, max_entry(edge));
    }

    pub fn put_failed(&mut self, path: &Path, meta: &Metadata) {
        let Some(entry) = self.fail_entry(path) else {
            return;
        };
        let clear = Rgba {
            width: 1,
            height: 1,
            pixels: &[0; 4],
        };
        self.store(&entry, path, meta, clear, &[], max_entry(1));
    }

    fn store(
        &mut self,
        entry: &Path,
        path: &Path,
        meta: &Metadata,
        thumb: Rgba,
        extra: &[(&str, String)],
        cap: u64,
    ) {
        let mut text = vec![
            ("Thumb::URI", file_uri(path)),
            ("Thumb::MTime", meta.mtime().to_string()),
            ("Thumb::Size", meta.len().to_string()),
            ("Software", self.program.clone()),
        ];
        text.extend_from_slice(extra);
        let bytes = match encode(&thumb, &text) {
            Ok(b) => b,
            Err(e) => {
                log::debug!("thumbnail cache: could not encode {}: {e}", path.display());
                return;
            }
        };
        let len = bytes.len() as u64;
        if len > cap || len > self.max_bytes {
            return;
        }
        let old = std::fs::metadata(entry).map(|m| m.len()).unwrap_or(0);
        if let Err(e) = self.write(entry, &bytes) {
            log::debug!("thumbnail cache: could not write {}: {e}", entry.display());
            return;
        }
        let used = match self.used {
            Some(u) => u.saturating_sub(old) + len,
            None => self.measure(),
        };
        self.used = Some(used);
        if used > self.max_bytes {
            self.trim();
        }
    }

    /// A temp file beside `entry`, renamed over it. No sync: a torn entry
    /// after a crash fails its PNG checks.
    fn write(&mut self, entry: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let dir = entry.parent().unwrap_or(&self.root);
        // The standard wants every directory from the root down at 0700;
        // one that already exists is left as it is.
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        self.seq += 1;
        let tmp = dir.join(format!(
            ".{}-{}-{}.tmp",
            self.program,
            std::process::id(),
            self.seq
        ));
        let written = std::fs::File::options()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .and_then(|mut f| f.write_all(bytes))
            .and_then(|()| std::fs::rename(&tmp, entry));
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        written
    }

    /// The PNGs of the size directories and this program's failures:
    /// (mtime, bytes, path). Another program's temp files are left out,
    /// so a trim never pulls one from under its rename.
    fn scan(&self) -> Vec<(SystemTime, u64, PathBuf)> {
        SIZES
            .iter()
            .map(|(dir, _)| self.root.join(dir))
            .chain([self.fail_dir()])
            .filter_map(|dir| std::fs::read_dir(dir).ok())
            .flatten()
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                m.is_file().then(|| {
                    (
                        m.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                        m.len(),
                        e.path(),
                    )
                })
            })
            .collect()
    }

    fn measure(&self) -> u64 {
        self.scan().iter().map(|(_, len, _)| len).sum()
    }

    /// Remove the least recently used entries until the cache is back
    /// under [`TRIM_TO`] of the cap. Measured afresh, since another
    /// program may have written or trimmed since.
    fn trim(&mut self) {
        let mut files = self.scan();
        files.sort();
        let mut used: u64 = files.iter().map(|(_, len, _)| len).sum();
        let target = self.max_bytes / TRIM_TO.1 * TRIM_TO.0;
        for (_, len, path) in files {
            if used <= target {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                used = used.saturating_sub(len);
            }
        }
        self.used = Some(used);
    }
}

/// Remove the size directories' entries whose `Software` key starts with `program`, and the
/// failures under `fail/<program>*`, leaving every other program's; returns how many files went.
/// Only each file's first [`HEAD`] bytes are read, since [`Thumbnails`] writes its text ahead of
/// the pixels.
pub fn clear_written_by(root: &Path, program: &str) -> usize {
    let ours = |entry: &Path| {
        let mut head = Vec::new();
        std::fs::File::open(entry)
            .and_then(|f| f.take(HEAD).read_to_end(&mut head))
            .is_ok()
            && text_chunks(&head)
                .is_some_and(|t| key(&t, "Software").is_some_and(|v| v.starts_with(program)))
    };
    let pngs = |dir: PathBuf| {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "png") && p.is_file())
    };
    let mut gone = SIZES
        .iter()
        .flat_map(|(dir, _)| pngs(root.join(dir)))
        .filter(|p| ours(p))
        .filter(|p| std::fs::remove_file(p).is_ok())
        .count();
    let fails = std::fs::read_dir(root.join("fail")).into_iter().flatten();
    for dir in fails.filter_map(Result::ok) {
        if dir.file_name().as_bytes().starts_with(program.as_bytes()) {
            gone += pngs(dir.path())
                .filter(|p| std::fs::remove_file(p).is_ok())
                .count();
            let _ = std::fs::remove_dir(dir.path());
        }
    }
    gone
}

/// How much of an entry [`clear_written_by`] reads for its `Software` key.
const HEAD: u64 = 16 << 10;

/// `path` as the standard names it: `file://` and the absolute path,
/// every byte outside `A-Z a-z 0-9 ! $ & ' ( ) * + , - . / : = @ _ ~`
/// percent-encoded in upper-case hex. That is the set GLib leaves
/// unescaped in a file URI, so the MD5 matches the one GLib-based file
/// managers compute.
pub fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"!$&'()*+,-./:=@_~".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// The PNG at `entry` and its text, if the file is no larger than `cap`
/// and its text vouches for `path` as `meta` describes it. A hit's mtime
/// is touched, since the trim judges use by it; a read-only cache still
/// answers, it just forgets in write order.
fn read_valid(entry: &Path, path: &Path, meta: &Metadata, cap: u64) -> Option<(Vec<u8>, Text)> {
    let mut f = std::fs::File::open(entry).ok()?;
    if f.metadata().ok()?.len() > cap {
        return None;
    }
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).ok()?;
    let text = text_chunks(&bytes)?;
    if !vouches(&text, &file_uri(path), meta) {
        return None;
    }
    if let Ok(f) = std::fs::File::options().write(true).open(entry) {
        let _ = f.set_modified(SystemTime::now());
    }
    Some((bytes, text))
}

type Text = Vec<(String, String)>;

fn key<'a>(text: &'a [(String, String)], k: &str) -> Option<&'a str> {
    text.iter().find(|(key, _)| key == k).map(|(_, v)| v.trim())
}

/// Whether `text` names the picture at `uri` as `meta` describes it:
/// `Thumb::URI` and `Thumb::MTime` must match, and `Thumb::Size` too
/// when present.
fn vouches(text: &[(String, String)], uri: &str, meta: &Metadata) -> bool {
    let check = || -> Option<bool> {
        if key(text, "Thumb::URI")? != uri {
            return Some(false);
        }
        // Whole seconds; a fraction some writers add is ignored.
        let mtime = key(text, "Thumb::MTime")?.split('.').next()?;
        if mtime.parse::<i64>().ok()? != meta.mtime() {
            return Some(false);
        }
        Some(match key(text, "Thumb::Size") {
            Some(size) => size.parse::<u64>().ok()? == meta.len(),
            None => true,
        })
    };
    check() == Some(true)
}

fn text_chunks(bytes: &[u8]) -> Option<Text> {
    /// More than any URI a thumbnail names, so a compressed chunk cannot
    /// inflate without bound.
    const LIMIT: usize = 64 << 10;
    let reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .ok()?;
    let info = reader.info();
    let mut out: Text = info
        .uncompressed_latin1_text
        .iter()
        .map(|t| (t.keyword.clone(), t.text.clone()))
        .collect();
    for t in &info.compressed_latin1_text {
        let mut t = t.clone();
        if t.decompress_text_with_limit(LIMIT).is_ok()
            && let Ok(text) = t.get_text()
        {
            out.push((t.keyword, text));
        }
    }
    for t in &info.utf8_text {
        let mut t = t.clone();
        if t.decompress_text_with_limit(LIMIT).is_ok()
            && let Ok(text) = t.get_text()
        {
            out.push((t.keyword, text));
        }
    }
    Some(out)
}

/// `thumb` as an 8-bit RGBA PNG, as the standard asks, carrying `text`
/// as `tEXt` chunks ahead of the pixels.
fn encode(thumb: &Rgba, text: &[(&str, String)]) -> Result<Vec<u8>, png::EncodingError> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, thumb.width, thumb.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    for (key, value) in text {
        enc.add_text_chunk(key.to_string(), value.clone())?;
    }
    let mut w = enc.write_header()?;
    w.write_image_data(thumb.pixels)?;
    w.finish()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const EDGE: u32 = 128;
    /// A picture's own size, larger than [`EDGE`] so its thumbnail is kept.
    const DIMS: (u32, u32) = (800, 600);
    const PROGRAM: &str = "reprise-test-1.0";

    fn pixels(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
        px.repeat((w * h) as usize)
    }

    fn rgba(w: u32, h: u32, pixels: &[u8]) -> Rgba<'_> {
        Rgba {
            width: w,
            height: h,
            pixels,
        }
    }

    fn setup(max_bytes: u64) -> (tempfile::TempDir, PathBuf, Thumbnails) {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.png");
        std::fs::write(&src, b"not really a png").unwrap();
        let cache = Thumbnails::at(dir.path().join("thumbnails"), PROGRAM, max_bytes);
        (dir, src, cache)
    }

    fn keys(path: &Path) -> std::collections::BTreeMap<String, String> {
        text_chunks(&std::fs::read(path).unwrap())
            .unwrap()
            .into_iter()
            .collect()
    }

    fn put(cache: &mut Thumbnails, src: &Path, meta: &Metadata) {
        let px = pixels(10, 10, [7, 7, 7, 255]);
        cache.put(src, meta, EDGE, rgba(10, 10, &px), DIMS);
    }

    #[test]
    fn an_entry_is_named_by_the_md5_of_the_file_uri() {
        let cache = Thumbnails::at(PathBuf::from("/home/jens/.cache/thumbnails"), PROGRAM, 1);
        let me = Path::new("/home/jens/photos/me.png");
        assert_eq!(file_uri(me), "file:///home/jens/photos/me.png");
        assert_eq!(
            cache.entry(me, EDGE).unwrap(),
            Path::new("/home/jens/.cache/thumbnails/normal/c6ee772d9e49320e97ec29a7eb5b1697.png")
        );
        assert_eq!(
            cache.entry(me, 256).unwrap().parent().unwrap(),
            Path::new("/home/jens/.cache/thumbnails/large")
        );
        assert_eq!(
            cache.fail_entry(me).unwrap(),
            Path::new(
                "/home/jens/.cache/thumbnails/fail/reprise-test-1.0/c6ee772d9e49320e97ec29a7eb5b1697.png"
            )
        );
        assert!(cache.entry(me, 100).is_none(), "not a standard size");
        assert!(cache.entry(Path::new("photos/me.png"), EDGE).is_none());
        assert_eq!(
            file_uri(Path::new("/a b/ü#%?;[x].png")),
            "file:///a%20b/%C3%BC%23%25%3F%3B%5Bx%5D.png"
        );
        assert_eq!(
            file_uri(Path::new("/!$&'()*+,-.:=@_~")),
            "file:///!$&'()*+,-.:=@_~"
        );
    }

    #[test]
    fn clearing_removes_only_the_programs_own_entries_and_failures() {
        let (dir, src, mut cache) = setup(1 << 20);
        let meta = std::fs::metadata(&src).unwrap();
        put(&mut cache, &src, &meta);
        cache.put_failed(&src, &meta);
        let other = dir.path().join("b.png");
        std::fs::write(&other, b"another picture").unwrap();
        let other_meta = std::fs::metadata(&other).unwrap();
        let mut theirs = Thumbnails::at(cache.root.clone(), "gnome-thumbnail-factory", 1 << 20);
        put(&mut theirs, &other, &other_meta);
        theirs.put_failed(&other, &other_meta);

        assert_eq!(clear_written_by(&cache.root, "reprise-"), 2);
        assert!(cache.get(&src, &meta, EDGE).is_none());
        assert!(!cache.failed(&src, &meta));
        assert!(!cache.fail_dir().exists(), "the empty failure folder goes");
        assert!(theirs.get(&other, &other_meta, EDGE).is_some());
        assert!(theirs.failed(&other, &other_meta));
        assert_eq!(clear_written_by(&cache.root, "reprise-"), 0);
    }

    #[test]
    fn a_decode_size_rounds_up_to_a_standard_size() {
        assert_eq!(edge_for(94), 128);
        assert_eq!(edge_for(128), 128);
        assert_eq!(edge_for(188), 256);
        assert_eq!(edge_for(512), 512);
        assert_eq!(edge_for(4000), 1024);
    }

    /// Written as a private PNG carrying the standard's keys, read back
    /// byte for byte.
    #[test]
    fn a_thumbnail_comes_back_as_it_went_in() {
        let (_dir, src, mut cache) = setup(1 << 20);
        let meta = std::fs::metadata(&src).unwrap();
        assert!(cache.get(&src, &meta, EDGE).is_none());
        let px = pixels(128, 96, [0x11, 0x22, 0x33, 0xff]);
        cache.put(&src, &meta, EDGE, rgba(128, 96, &px), DIMS);
        let hit = cache.get(&src, &meta, EDGE).expect("a hit");
        assert_eq!(hit.dims, Some(DIMS));

        let entry = cache.entry(&src, EDGE).unwrap();
        assert_eq!(hit.png, std::fs::read(&entry).unwrap());
        let mut reader = png::Decoder::new(std::io::Cursor::new(&hit.png))
            .read_info()
            .unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut buf).unwrap();
        assert_eq!(buf, px);

        let k = keys(&entry);
        assert_eq!(k["Thumb::URI"], file_uri(&src));
        assert_eq!(k["Thumb::MTime"], meta.mtime().to_string());
        assert_eq!(k["Thumb::Size"], "16");
        assert_eq!(k["Thumb::Image::Width"], "800");
        assert_eq!(k["Thumb::Image::Height"], "600");
        assert_eq!(k["Software"], PROGRAM);
        let mode = |p: &Path| std::fs::metadata(p).unwrap().mode() & 0o777;
        assert_eq!(mode(&cache.root), 0o700);
        assert_eq!(mode(entry.parent().unwrap()), 0o700);
        assert_eq!(mode(&entry), 0o600);
        let left: Vec<_> = std::fs::read_dir(entry.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(left, [entry], "no temp file stays behind");
    }

    #[test]
    fn a_picture_with_a_new_mtime_or_size_misses() {
        let (_dir, src, mut cache) = setup(1 << 20);
        let meta = std::fs::metadata(&src).unwrap();
        put(&mut cache, &src, &meta);
        let f = std::fs::File::options().write(true).open(&src).unwrap();
        f.set_modified(meta.modified().unwrap() + Duration::from_secs(1))
            .unwrap();
        let touched = std::fs::metadata(&src).unwrap();
        assert!(cache.get(&src, &touched, EDGE).is_none());
        std::fs::write(&src, b"a different, longer picture").unwrap();
        f.set_modified(meta.modified().unwrap()).unwrap();
        let resized = std::fs::metadata(&src).unwrap();
        assert_eq!(resized.mtime(), meta.mtime());
        assert!(cache.get(&src, &resized, EDGE).is_none());
        // Nor does a thumbnail of another size answer.
        assert!(cache.get(&src, &meta, 256).is_none());
    }

    /// Another program's thumbnail serves when its keys vouch for the
    /// file; `Thumb::Size` and the image size are optional.
    #[test]
    fn a_thumbnail_another_program_wrote_is_read() {
        let (dir, src, cache) = setup(1 << 20);
        let meta = std::fs::metadata(&src).unwrap();
        let entry = cache.entry(&src, EDGE).unwrap();
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        let px = pixels(4, 3, [0, 0xff, 0, 0xff]);
        let write = |text: &[(&str, String)]| {
            std::fs::write(&entry, encode(&rgba(4, 3, &px), text).unwrap()).unwrap();
        };
        let dims = [
            ("Thumb::Image::Width", "800".to_string()),
            ("Thumb::Image::Height", "600".to_string()),
        ];
        let stamp = |uri: String, mtime: String| [("Thumb::URI", uri), ("Thumb::MTime", mtime)];
        write(
            &[
                stamp(file_uri(&src), meta.mtime().to_string()),
                dims.clone(),
            ]
            .concat(),
        );
        assert_eq!(
            cache.get(&src, &meta, EDGE).expect("a hit").dims,
            Some(DIMS)
        );
        // GLib writes a fraction after the seconds.
        write(&stamp(file_uri(&src), format!("{}.25", meta.mtime())));
        assert_eq!(cache.get(&src, &meta, EDGE).expect("a hit").dims, None);
        // One for another URI hashing to the same name is not taken.
        let other = dir.path().join("b.png");
        write(
            &[
                stamp(file_uri(&other), meta.mtime().to_string()),
                dims.clone(),
            ]
            .concat(),
        );
        assert!(cache.get(&src, &meta, EDGE).is_none());
        // Nor one without a URI or an mtime.
        write(&[("Thumb::URI", file_uri(&src))]);
        assert!(cache.get(&src, &meta, EDGE).is_none());
        write(&[("Thumb::MTime", meta.mtime().to_string())]);
        assert!(cache.get(&src, &meta, EDGE).is_none());
        // Nor a file that is not a PNG.
        std::fs::write(&entry, b"not a png").unwrap();
        assert!(cache.get(&src, &meta, EDGE).is_none());
    }

    /// A failure answers until the picture changes, and only for the
    /// program that filed it.
    #[test]
    fn a_failure_is_remembered_until_the_picture_changes() {
        let (_dir, src, mut cache) = setup(1 << 20);
        let meta = std::fs::metadata(&src).unwrap();
        assert!(!cache.failed(&src, &meta));
        cache.put_failed(&src, &meta);
        assert!(cache.failed(&src, &meta));
        let entry = cache.fail_entry(&src).unwrap();
        let k = keys(&entry);
        assert_eq!(k["Thumb::URI"], file_uri(&src));
        assert_eq!(k["Thumb::MTime"], meta.mtime().to_string());
        assert_eq!(std::fs::metadata(&entry).unwrap().mode() & 0o777, 0o600);
        // No thumbnail stands in for it.
        assert!(cache.get(&src, &meta, EDGE).is_none());

        let other = Thumbnails::at(cache.root.clone(), "someone-else-2.0", 1 << 20);
        assert!(!other.failed(&src, &meta));

        std::fs::write(&src, b"now a longer, fixed picture").unwrap();
        let fixed = std::fs::metadata(&src).unwrap();
        assert!(!cache.failed(&src, &fixed));
    }

    /// Pictures inside the cache and pictures no larger than their
    /// thumbnail are never written.
    #[test]
    fn the_cache_and_small_pictures_are_left_out() {
        let (_dir, src, mut cache) = setup(1 << 20);
        let meta = std::fs::metadata(&src).unwrap();
        let px = pixels(2, 2, [1, 1, 1, 1]);
        cache.put(&src, &meta, EDGE, rgba(2, 2, &px), (128, 64));
        assert!(!cache.root.exists());
        let inner = cache.root.join("normal").join("x.png");
        std::fs::create_dir_all(inner.parent().unwrap()).unwrap();
        std::fs::write(&inner, b"x").unwrap();
        let inner_meta = std::fs::metadata(&inner).unwrap();
        cache.put(&inner, &inner_meta, EDGE, rgba(2, 2, &px), DIMS);
        cache.put_failed(&inner, &inner_meta);
        let files: Vec<PathBuf> = cache.scan().into_iter().map(|(_, _, p)| p).collect();
        assert_eq!(files, [inner], "the picture itself, and no thumbnail of it");
    }

    #[test]
    fn past_the_cap_the_least_recently_used_go() {
        let (dir, src, mut cache) = setup(1 << 20);
        // Every entry is the same PNG bar its name keys, so one entry's
        // size stands for all: room for three and a half.
        let meta = std::fs::metadata(&src).unwrap();
        put(&mut cache, &src, &meta);
        let one = cache.measure();
        std::fs::remove_file(cache.entry(&src, EDGE).unwrap()).unwrap();
        cache.used = None;
        cache.max_bytes = one * 3 + one / 2;
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mut srcs = Vec::new();
        for (i, name) in ["p0.png", "p1.png", "p2.png"].iter().enumerate() {
            let p = dir.path().join(name);
            std::fs::write(&p, b"x").unwrap();
            let m = std::fs::metadata(&p).unwrap();
            put(&mut cache, &p, &m);
            // Written oldest first, a second apart.
            let f = std::fs::File::options()
                .write(true)
                .open(cache.entry(&p, EDGE).unwrap())
                .unwrap();
            f.set_modified(old + Duration::from_secs(i as u64)).unwrap();
            srcs.push((p, m));
        }
        // p0 is looked at again, so p1 is now the least recently used.
        assert!(cache.get(&srcs[0].0, &srcs[0].1, EDGE).is_some());
        let p = dir.path().join("p3.png");
        std::fs::write(&p, b"x").unwrap();
        let m = std::fs::metadata(&p).unwrap();
        put(&mut cache, &p, &m);
        assert!(cache.get(&srcs[0].0, &srcs[0].1, EDGE).is_some());
        assert!(cache.get(&srcs[1].0, &srcs[1].1, EDGE).is_none());
        assert!(cache.get(&srcs[2].0, &srcs[2].1, EDGE).is_some());
        assert!(cache.get(&p, &m, EDGE).is_some());
        assert!(cache.measure() <= cache.max_bytes);
    }
}
