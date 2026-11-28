//! The preview pane's reader: a thread of its own taking one file at a time, so a read that
//! hangs never holds the listing threads the window navigates with.
//!
//! Every route starts at one choke point: a single call that opens the file (`open_regular`,
//! which stats it and refuses anything but a regular file) and reads up to the route's byte cap,
//! all inside [`repriseapp::thumbs::named_read`]'s deadline. Everything after works from those
//! bytes, or from a copy of them in a scratch file the reader owns and removes: a font's sampler
//! and a picture helper get the copy, never the path on the mount.
//!
//! When the open itself misses the deadline, the selection's folder is marked slow for
//! [`MARK_FOR`], and later files in it answer at once with no preview and no read. A read that
//! was only slow after the open, or a call refused untried because earlier ones have not
//! returned, marks nothing.

use crate::preview::{self, Loaded};
use repriseapp::Waker;
use reprisekit::controls::filepreview::{self, Route, SNIFF_BYTES, TEXT_BYTES};
use reprisekit::raster::Canvas;
use std::collections::HashMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

/// A file as the pane tells it apart: its path, size and time, so a changed file is read again.
pub(crate) type Key = (PathBuf, u64, Option<i64>);

/// A font file larger than this has no preview.
const FONT_BYTES: u64 = 32 << 20;
/// How long a folder whose open missed the deadline answers without a read.
pub(crate) const MARK_FOR: Duration = Duration::from_secs(30);
/// The longest extension a scratch copy keeps.
const EXT_MAX: usize = 8;
const SCRATCH_PREFIX: &str = "reprise-preview-";

/// What the reader is asked: a file, how to read it, and the device px and scale a picture fits.
pub(crate) struct Request {
    pub key: Key,
    pub route: Route,
    pub max: (u32, u32),
    pub scale: i32,
}

/// A file's metadata and its bytes up to a cap.
pub(crate) type Fetched = (std::fs::Metadata, Vec<u8>);
/// The file at a path opened and read up to a cap, setting the flag once the open is done.
pub(crate) type Fetch = Arc<dyn Fn(&Path, u64, &AtomicBool) -> io::Result<Fetched> + Send + Sync>;
/// A call on a path run under a deadline.
pub(crate) type Call = Box<dyn FnOnce() -> io::Result<Fetched> + Send>;
pub(crate) type Guard = Arc<dyn Fn(&Path, Call) -> io::Result<Fetched> + Send + Sync>;
/// A font file's sample, or a picture a helper decodes, from a local copy, fitted inside device px
/// at a scale.
pub(crate) type FromCopy = Arc<dyn Fn(&Path, (u32, u32), i32) -> Option<Canvas> + Send + Sync>;

/// How the reader reaches files: real ones through [`Parts::installed`], a test's stand-ins
/// otherwise.
#[derive(Clone)]
pub(crate) struct Parts {
    pub fetch: Fetch,
    pub guard: Guard,
    pub sample: FromCopy,
    pub helper_picture: FromCopy,
    /// Whether a file with no preview of its own falls back to the thumbnail cache.
    pub cache: bool,
}

impl Parts {
    pub(crate) fn installed() -> Parts {
        Parts {
            fetch: Arc::new(|path, cap, opened| {
                let f = reprisefs::ops::open_regular(path)?;
                let meta = f.metadata()?;
                opened.store(true, Ordering::SeqCst);
                let mut bytes = Vec::new();
                f.take(cap).read_to_end(&mut bytes)?;
                Ok((meta, bytes))
            }),
            guard: Arc::new(repriseapp::thumbs::named_read),
            sample: Arc::new(|copy, max, scale| {
                preview::Jailed::installed().sample(copy, max, scale)
            }),
            helper_picture: Arc::new(|copy, max, scale| {
                let j = preview::Jailed::installed();
                repriseapp::thumbs::picture_in(j.runner.as_ref(), &j.exe, copy, max)
                    .map(|c| c.at_scale(scale))
            }),
            cache: true,
        }
    }
}

/// Folders whose open missed the deadline, and when.
#[derive(Default)]
pub(crate) struct Marks(HashMap<PathBuf, Instant>);

impl Marks {
    pub(crate) fn mark(&mut self, dir: PathBuf, at: Instant) {
        self.0.insert(dir, at);
    }

    /// Whether `dir` is marked at `now`; a mark past [`MARK_FOR`] is dropped.
    pub(crate) fn holds(&mut self, dir: &Path, now: Instant) -> bool {
        self.0
            .retain(|_, at| now.saturating_duration_since(*at) < MARK_FOR);
        self.0.contains_key(dir)
    }
}

/// What [`Reader::take`] finds.
pub(crate) enum Taken {
    Answer(Key, Loaded),
    Waiting,
    /// The reader's thread is gone: a new reader takes its place.
    Gone,
}

pub(crate) struct Reader {
    ask: Sender<Request>,
    answers: Receiver<(Key, Loaded)>,
}

impl Reader {
    /// Starts the reader's thread, which wakes the window with each answer and ends with the
    /// reader. A route that panics answers with no preview.
    pub(crate) fn spawn(parts: Parts, wake: Waker) -> Reader {
        let (ask, asked) = channel::<Request>();
        let (tell, answers) = channel();
        let started = std::thread::Builder::new()
            .name("preview-pane".into())
            .spawn(move || {
                sweep_scratch();
                let mut marks = Marks::default();
                for req in asked {
                    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        read(&parts, &mut marks, &req)
                    }));
                    let loaded = read.unwrap_or(Loaded::Nothing);
                    if tell.send((req.key, loaded)).is_err() {
                        break;
                    }
                    wake.wake();
                }
            });
        if let Err(e) = started {
            log::warn!("preview pane reader: {e}");
        }
        Reader { ask, answers }
    }

    /// Asks for `req`; false when the thread is gone.
    pub(crate) fn ask(&self, req: Request) -> bool {
        self.ask.send(req).is_ok()
    }

    pub(crate) fn take(&self) -> Taken {
        match self.answers.try_recv() {
            Ok((key, loaded)) => Taken::Answer(key, loaded),
            Err(TryRecvError::Empty) => Taken::Waiting,
            Err(TryRecvError::Disconnected) => Taken::Gone,
        }
    }
}

/// `req`'s file through the choke point; its folder marked in `marks` when the open missed the
/// deadline.
pub(crate) fn read(parts: &Parts, marks: &mut Marks, req: &Request) -> Loaded {
    let path = &req.key.0;
    let dir = path.parent().unwrap_or(path).to_path_buf();
    if marks.holds(&dir, Instant::now()) {
        return Loaded::Nothing;
    }
    let cap = match req.route {
        Route::Text | Route::Other => TEXT_BYTES as u64,
        Route::Picture => repriseapp::decode::Budget::THUMBNAIL.file,
        Route::Font => FONT_BYTES,
    };
    let opened = Arc::new(AtomicBool::new(false));
    let (fetch, p, flag) = (parts.fetch.clone(), path.clone(), opened.clone());
    let (meta, bytes) = match (parts.guard)(path, Box::new(move || fetch(&p, cap, &flag))) {
        Ok(read) => read,
        Err(e) => {
            let stalled_open = e.kind() == io::ErrorKind::TimedOut
                && !repriseapp::thumbs::budget_full(&e)
                && !opened.load(Ordering::SeqCst);
            if stalled_open {
                marks.mark(dir, Instant::now());
            }
            return Loaded::Nothing;
        }
    };
    let whole = meta.len() <= cap;
    let found = match req.route {
        Route::Text | Route::Other => text(&bytes, req.route).map(Loaded::Text),
        Route::Picture if whole => picture(parts, path, &bytes, req).map(Loaded::Picture),
        Route::Font if whole => (Scratch::of(path, &bytes).ok())
            .and_then(|copy| (parts.sample)(&copy.0, req.max, req.scale))
            .map(Loaded::Picture),
        Route::Picture | Route::Font => None,
    };
    found
        .or_else(|| {
            let cache = repriseapp::thumbs::cache_for(repriseconf::current().thumbnail_cache);
            let cache = cache.filter(|_| parts.cache)?;
            preview::cached(path, &meta, &cache, req.max, req.scale).map(Loaded::Picture)
        })
        .unwrap_or(Loaded::Nothing)
}

fn text(bytes: &[u8], route: Route) -> Option<String> {
    let head = &bytes[..bytes.len().min(SNIFF_BYTES)];
    let shows = filepreview::looks_text(head) && !(route == Route::Other && head.is_empty());
    shows.then(|| filepreview::text_of(bytes))
}

/// A picture decoded from its bytes, or by the helper from a copy for a format it decodes.
fn picture(parts: &Parts, path: &Path, bytes: &[u8], req: &Request) -> Option<Canvas> {
    if repriseapp::decode::helper_format(bytes).is_none() {
        let budget = repriseapp::decode::Budget::THUMBNAIL;
        let d = repriseapp::decode::Decoders::built_in().decode(bytes, req.max, budget);
        return d.ok().map(|d| d.canvas.at_scale(req.scale));
    }
    let copy = Scratch::of(path, bytes).ok()?;
    (parts.helper_picture)(&copy.0, req.max, req.scale)
}

fn scratch_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from)
}

/// Removes the scratch copies of Explorers no longer running, once a process.
fn sweep_scratch() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| sweep_scratch_in(&scratch_dir()));
}

fn sweep_scratch_in(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(rest) = name.to_str().and_then(|n| n.strip_prefix(SCRATCH_PREFIX)) else {
            continue;
        };
        let pid = rest.split('-').next().and_then(|p| p.parse::<u32>().ok());
        if pid.is_some_and(|pid| !Path::new(&format!("/proc/{pid}")).exists()) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// A copy of a file's bytes in the runtime directory, removed when dropped.
pub(crate) struct Scratch(pub PathBuf);

impl Scratch {
    fn of(original: &Path, bytes: &[u8]) -> io::Result<Scratch> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        use std::sync::atomic::AtomicU64;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let ext = (original.extension().and_then(|e| e.to_str()))
            .filter(|e| e.len() <= EXT_MAX && e.bytes().all(|b| b.is_ascii_alphanumeric()));
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let mut name = format!("{SCRATCH_PREFIX}{}-{n}", std::process::id());
        if let Some(ext) = ext {
            name = format!("{name}.{ext}");
        }
        let at = scratch_dir().join(name);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&at)?;
        let copy = Scratch(at);
        f.write_all(bytes)?;
        Ok(copy)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
#[path = "panereader_tests.rs"]
mod tests;
