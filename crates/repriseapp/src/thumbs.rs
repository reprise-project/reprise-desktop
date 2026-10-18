//! Pictures' thumbnails for Thumbnails and Tiles, decoded off the event loop by one worker thread
//! through the freedesktop thumbnail cache. Explorer and the file dialog host share it; Explorer's
//! [`serve_stdio`] decodes for the compositor's Browse dialog in a process of its own. Navigation
//! bumps a generation counter and the worker drops requests older than the newest it has seen.
//! A file that does not answer within [`WITHIN`] keeps its icon, and the worker moves on.

use crate::Waker;
use crate::decode::Decoders;
use reprisefs::thumbnails::{Cleared, Rgba, Thumbnails, edge_for};
use reprisefs::thumbpipe;
use reprisekit::controls::shellview;
use reprisekit::raster::Canvas;
use std::collections::{HashSet, VecDeque};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::JoinHandle;
use std::time::Duration;

/// How long a picture's open or read may take before the picture keeps its icon: well past a
/// large photo's read on a slow network mount, short enough that a dead mount holds the rest of
/// the folder back only briefly.
const WITHIN: Duration = Duration::from_secs(3);

/// Calls given up on and still blocked, past which pictures keep their icons without a try until
/// one returns: each holds a thread.
const MAX_STUCK: usize = 4;

struct Request {
    path: PathBuf,
    generation: u64,
    /// The draw scale the thumbnail is for.
    scale: i32,
}

pub struct Done {
    pub path: PathBuf,
    pub thumb: Canvas,
    pub dims: (u32, u32),
    pub generation: u64,
    /// The draw scale it was asked for: `thumb` fills [`shellview::thumb_px`] of it.
    pub scale: i32,
}

pub struct Thumbs {
    tx: Option<Sender<Request>>,
    rx: Receiver<Done>,
    pending: HashSet<(PathBuf, i32)>,
    generation: u64,
    worker: Option<JoinHandle<()>>,
    done_tx: Sender<Done>,
}

impl Thumbs {
    pub fn new(decoders: Arc<Decoders>, cache: Option<Thumbnails>, wake: Waker) -> Thumbs {
        let (tx, req_rx) = channel::<Request>();
        let (done_tx, rx) = channel::<Done>();
        let test_tx = done_tx.clone();
        let worker = std::thread::Builder::new()
            .name("thumbs".to_string())
            .spawn(move || work(decoders, cache, req_rx, done_tx, wake));
        let worker = match worker {
            Ok(h) => Some(h),
            Err(e) => {
                log::warn!("no thumbnail worker: {e}");
                None
            }
        };
        Thumbs {
            tx: worker.is_some().then_some(tx),
            rx,
            pending: HashSet::new(),
            generation: 0,
            worker,
            done_tx: test_tx,
        }
    }

    /// Hands `done` in as the worker would: tests deliver thumbnails without decoding.
    pub fn deliver(&self, done: Done) {
        self.done_tx.send(done).expect("the receiver is ours");
    }

    /// Asks for `path`'s thumbnail for a window drawn at `scale`; a path already asked for at
    /// another scale is asked for again.
    pub fn request(&mut self, path: PathBuf, generation: u64, scale: i32) {
        self.generation = generation;
        if !self.pending.insert((path.clone(), scale)) {
            return;
        }
        let Some(tx) = &self.tx else {
            return;
        };
        let req = Request {
            path,
            generation,
            scale,
        };
        if tx.send(req).is_err() {
            log::warn!("the thumbnail worker went away");
            self.tx = None;
        }
    }

    /// Whether `path`'s thumbnail for `scale` is asked for and has not arrived.
    pub fn asked(&self, path: &std::path::Path, scale: i32) -> bool {
        self.pending.contains(&(path.to_path_buf(), scale))
    }

    pub fn cancel_all(&mut self) {
        self.pending.clear();
    }

    pub fn drain(&mut self) -> Vec<Done> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(done) => {
                    self.pending.remove(&(done.path.clone(), done.scale));
                    if done.generation == self.generation {
                        out.push(done);
                    }
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return out,
            }
        }
    }
}

impl Drop for Thumbs {
    fn drop(&mut self) {
        self.tx = None;
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

fn work(
    decoders: Arc<Decoders>,
    mut cache: Option<Thumbnails>,
    rx: Receiver<Request>,
    tx: Sender<Done>,
    wake: Waker,
) {
    let mut queue: VecDeque<Request> = VecDeque::new();
    let mut newest = 0u64;
    let fs = Deadlines::new(WITHIN);
    loop {
        let Ok(req) = rx.recv() else {
            return;
        };
        newest = newest.max(req.generation);
        queue.push_back(req);
        take_queued(&rx, &mut queue, &mut newest);
        while let Some(req) = queue.pop_front() {
            // A fresh listing may have landed while the last decode ran.
            take_queued(&rx, &mut queue, &mut newest);
            if req.generation < newest {
                continue;
            }
            // A file no decoder claims (a truncated JPEG, a `.png` that
            // is not one) or whose read is given up on answers nothing, so
            // its path stays in the window's `pending` set for the rest of
            // the listing and is never asked for again: a retry costs a read
            // per pass and would not succeed. The next listing clears the set.
            let edge = edge_for(shellview::thumb_px(req.scale));
            let Some((thumb, dims)) = thumbnail(&decoders, edge, cache.as_mut(), &req.path, &fs)
            else {
                continue;
            };
            let done = Done {
                path: req.path,
                thumb,
                dims,
                generation: req.generation,
                scale: req.scale,
            };
            if tx.send(done).is_err() {
                return;
            }
            wake.wake();
        }
    }
}

/// The thumbnail helper of [`reprisefs::thumbpipe`]: answers the paths on stdin through the
/// shared cache as `[thumbnail_cache]` sets it, decoding a miss into it, until stdin closes.
pub fn serve_stdio() -> std::io::Result<()> {
    let mut cache = configured_cache();
    serve(
        &Decoders::default_set(),
        cache.as_mut(),
        std::io::stdin().lock(),
        std::io::stdout().lock(),
    )
}

fn serve(
    decoders: &Decoders,
    mut cache: Option<&mut Thumbnails>,
    mut input: impl std::io::BufRead,
    mut output: impl std::io::Write,
) -> std::io::Result<()> {
    let fs = Deadlines::new(WITHIN);
    while let Some((path, edge)) = thumbpipe::read_request(&mut input)? {
        let edge = edge_for(edge);
        let thumb = thumbnail(decoders, edge, cache.as_deref_mut(), &path, &fs).map(|(c, dims)| {
            let (width, height) = c.device_size();
            thumbpipe::Thumb {
                width,
                height,
                pixels: c.pixels,
                dims,
            }
        });
        thumbpipe::write_reply(&mut output, thumb.as_ref())?;
    }
    Ok(())
}

/// The shared cache under `$XDG_CACHE_HOME/thumbnails`, capped at `max_bytes`.
pub fn open_cache(max_bytes: u64) -> Option<Thumbnails> {
    Some(Thumbnails::at(
        Thumbnails::default_dir()?,
        PROGRAM,
        max_bytes,
    ))
}

/// The cap when `[thumbnail_cache] max_mb` is unset.
pub const CACHE_BYTES: u64 = repriseconf::ThumbnailCache::DEFAULT.max_bytes();

/// The shared cache as config.toml's `[thumbnail_cache]` sets it, read now. None when it is off:
/// pictures still decode, and nothing is kept.
pub fn configured_cache() -> Option<Thumbnails> {
    cache_for(repriseconf::thumbnail_cache())
}

/// The shared cache as `setting` has it: None when it is off.
pub fn cache_for(setting: repriseconf::ThumbnailCache) -> Option<Thumbnails> {
    setting
        .enabled
        .then(|| open_cache(setting.max_bytes()))
        .flatten()
}

/// Remove what every Reprise version kept in the shared cache, other programs' thumbnails left
/// in place. Blocks on the file system.
pub fn clear_cache() -> Cleared {
    Thumbnails::default_dir().map_or(Cleared::default(), |root| {
        reprisefs::thumbnails::clear_written_by(&root, PROGRAM_PREFIX)
    })
}

const PROGRAM_PREFIX: &str = "reprise-";
const PROGRAM: &str = concat!("reprise-", env!("CARGO_PKG_VERSION"));

/// `path`'s thumbnail at `edge` and its size, from `cache` or decoded into it. The stat comes
/// before the read, so a picture changed in between is kept under its old stamp and simply misses
/// next time. Anything but a regular file (a named pipe, a device) is never opened, since the open
/// or the read may never return. A picture whose open or read `fs` gives up on is not marked
/// failed in the cache, so it is tried again once its mount answers.
fn thumbnail(
    decoders: &Decoders,
    edge: u32,
    mut cache: Option<&mut Thumbnails>,
    path: &std::path::Path,
    fs: &Deadlines,
) -> Option<(Canvas, (u32, u32))> {
    let p = path.to_path_buf();
    let (file, meta) = fs
        .run(move || {
            if !std::fs::metadata(&p)?.is_file() {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            let f = std::fs::File::open(&p)?;
            let meta = f.metadata()?;
            Ok((f, meta))
        })
        .map_err(|e| log::debug!("thumbnail {}: {e}", path.display()))
        .ok()?;
    if let Some(cache) = cache.as_deref_mut() {
        // An entry without the picture's size is a miss, since the Tiles
        // line needs it, and is written again with it.
        if let Some(hit) = cache.get(path, &meta, edge)
            && let Some(dims) = hit.dims
            && let Ok(thumb) = reprisekit::icontheme::png_to_canvas(&hit.png, edge)
        {
            return Some((thumb, dims));
        }
        if cache.failed(path, &meta) {
            return None;
        }
    }
    let img = match fs.run(move || crate::decode::read_capped_from(file)) {
        Err(e) if e.kind() == io::ErrorKind::TimedOut => {
            log::debug!("thumbnail {}: {e}", path.display());
            return None;
        }
        bytes => bytes.ok().and_then(|b| decoders.decode(&b, (edge, edge))),
    };
    if let Some(cache) = cache {
        match &img {
            Some(img) => {
                let (width, height) = img.canvas.device_size();
                let pixels = straight_rgba(&img.canvas);
                let thumb = Rgba {
                    width,
                    height,
                    pixels: &pixels,
                };
                cache.put(path, &meta, edge, thumb, (img.width, img.height));
            }
            None => cache.put_failed(path, &meta),
        }
    }
    let img = img?;
    Some((img.canvas, (img.width, img.height)))
}

/// File-system calls on threads of their own, each given up after `within`: a file on a dead
/// mount holds the worker that long at most.
struct Deadlines {
    within: Duration,
    /// Calls started and not yet returned.
    running: Arc<AtomicUsize>,
}

impl Deadlines {
    fn new(within: Duration) -> Deadlines {
        Deadlines {
            within,
            running: Arc::default(),
        }
    }

    /// What `call` returns, or `TimedOut` when it takes longer than `within` or [`MAX_STUCK`]
    /// earlier calls are still blocked.
    fn run<T: Send + 'static>(
        &self,
        call: impl FnOnce() -> io::Result<T> + Send + 'static,
    ) -> io::Result<T> {
        if self.running.load(Ordering::Acquire) >= MAX_STUCK {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{MAX_STUCK} file reads have not returned"),
            ));
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let running = self.running.clone();
        running.fetch_add(1, Ordering::AcqRel);
        let spawned = std::thread::Builder::new()
            .name("thumbs-read".into())
            .spawn(move || {
                let out = call();
                running.fetch_sub(1, Ordering::AcqRel);
                let _ = tx.send(out);
            });
        if let Err(e) = spawned {
            self.running.fetch_sub(1, Ordering::AcqRel);
            return Err(e);
        }
        rx.recv_timeout(self.within)
            .unwrap_or_else(|_| Err(io::ErrorKind::TimedOut.into()))
    }
}

fn straight_rgba(canvas: &Canvas) -> Vec<u8> {
    canvas
        .to_argb_be()
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[a, r, g, b]| [r, g, b, a])
        .collect()
}

fn take_queued(rx: &Receiver<Request>, queue: &mut VecDeque<Request>, newest: &mut u64) {
    while let Ok(req) = rx.try_recv() {
        *newest = (*newest).max(req.generation);
        queue.push_back(req);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stub() -> (Sender<Done>, Thumbs) {
        let (done_tx, rx) = channel::<Done>();
        let keep = done_tx.clone();
        (
            done_tx,
            Thumbs {
                tx: None,
                rx,
                pending: HashSet::new(),
                generation: 0,
                worker: None,
                done_tx: keep,
            },
        )
    }

    fn fs() -> Deadlines {
        Deadlines::new(WITHIN)
    }

    fn done(path: &str, generation: u64) -> Done {
        Done {
            path: PathBuf::from(path),
            thumb: Canvas::new(1, 1),
            dims: (800, 600),
            generation,
            scale: 1,
        }
    }

    #[test]
    fn a_cache_turned_off_is_not_opened() {
        let off = repriseconf::ThumbnailCache {
            enabled: false,
            ..repriseconf::ThumbnailCache::DEFAULT
        };
        assert!(cache_for(off).is_none());
        assert_eq!(
            cache_for(repriseconf::ThumbnailCache::DEFAULT).is_some(),
            Thumbnails::default_dir().is_some()
        );
    }

    #[test]
    fn drain_drops_a_thumbnail_from_a_folder_we_have_left() {
        let (tx, mut thumbs) = stub();
        thumbs.request(PathBuf::from("/pics/a.png"), 1, 1);
        thumbs.request(PathBuf::from("/pics/b.png"), 1, 1);
        thumbs.cancel_all();
        thumbs.request(PathBuf::from("/other/c.png"), 2, 1);
        tx.send(done("/pics/b.png", 1)).expect("send");
        tx.send(done("/other/c.png", 2)).expect("send");
        let out = thumbs.drain();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].path, PathBuf::from("/other/c.png"));
    }

    #[test]
    fn the_same_path_is_only_asked_for_once() {
        let (tx, mut thumbs) = stub();
        thumbs.request(PathBuf::from("/pics/a.png"), 1, 1);
        assert!(thumbs.pending.contains(&(PathBuf::from("/pics/a.png"), 1)));
        thumbs.request(PathBuf::from("/pics/a.png"), 1, 1);
        assert_eq!(thumbs.pending.len(), 1);
        tx.send(done("/pics/a.png", 1)).expect("send");
        assert_eq!(thumbs.drain().len(), 1);
        assert!(thumbs.pending.is_empty());
    }

    struct Counting(Arc<std::sync::atomic::AtomicUsize>);

    impl crate::decode::ImageDecoder for Counting {
        fn probe(&self, bytes: &[u8]) -> bool {
            bytes.starts_with(b"FAKE")
        }
        fn decode(&self, bytes: &[u8], _: (u32, u32)) -> Option<crate::decode::Decoded> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (!bytes.starts_with(b"FAKE BAD")).then(|| crate::decode::Decoded {
                canvas: Canvas::from_pixels(2, 1, vec![0xff00_00ff, 0xff00_ff00]),
                width: 640,
                height: 320,
            })
        }
    }

    fn counted(
        bytes: &[u8],
    ) -> (
        tempfile::TempDir,
        PathBuf,
        Decoders,
        Arc<std::sync::atomic::AtomicUsize>,
        Thumbnails,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.fake");
        std::fs::write(&src, bytes).unwrap();
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut decoders = Decoders::default_set();
        decoders.register(Box::new(Counting(count.clone())));
        let cache = Thumbnails::at(dir.path().join("cache"), PROGRAM, 1 << 20);
        (dir, src, decoders, count, cache)
    }

    #[test]
    fn a_cached_thumbnail_is_read_back_rather_than_decoded() {
        let (_dir, src, decoders, count, mut cache) = counted(b"FAKE picture");
        let first = thumbnail(&decoders, 128, Some(&mut cache), &src, &fs()).expect("decoded");
        let again = thumbnail(&decoders, 128, Some(&mut cache), &src, &fs()).expect("cached");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(first.0, again.0);
        assert_eq!(again.1, (640, 320));
        thumbnail(&decoders, 128, None, &src, &fs()).expect("decoded");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn the_helper_answers_each_path_in_order() {
        let (dir, src, decoders, count, mut cache) = counted(b"FAKE picture");
        let bad = dir.path().join("b.fake");
        std::fs::write(&bad, b"FAKE BAD picture").unwrap();
        let mut asked = Vec::new();
        for p in [&src, &bad, &src] {
            thumbpipe::write_request(&mut asked, p, 128).unwrap();
        }
        let mut answers = Vec::new();
        serve(&decoders, Some(&mut cache), &asked[..], &mut answers).unwrap();
        let mut r = std::io::Cursor::new(answers);
        let first = thumbpipe::read_reply(&mut r, 128)
            .unwrap()
            .expect("a thumbnail");
        assert_eq!((first.width, first.height, first.dims), (2, 1, (640, 320)));
        assert_eq!(first.pixels, [0xff00_00ff, 0xff00_ff00]);
        assert_eq!(thumbpipe::read_reply(&mut r, 128).unwrap(), None);
        assert_eq!(thumbpipe::read_reply(&mut r, 128).unwrap(), Some(first));
        assert_eq!(
            count.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "the second ask is a cache hit"
        );
    }

    #[test]
    fn a_named_pipe_is_not_opened() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("pipe.png");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(thumbnail(&Decoders::default_set(), 128, None, &fifo, &fs()).is_none());
        });
        let answer = rx.recv_timeout(std::time::Duration::from_secs(5));
        assert_eq!(
            answer,
            Ok(true),
            "no thumbnail, without waiting for a writer"
        );
    }

    #[test]
    fn a_frame_past_128_device_px_reads_and_writes_the_large_entries() {
        let edge = |scale| edge_for(shellview::thumb_px(scale));
        assert_eq!([edge(1), edge(2), edge(3)], [128, 256, 512]);
        let (dir, src, decoders, count, mut cache) = counted(b"FAKE picture");
        let held = |size: &str| {
            std::fs::read_dir(dir.path().join("cache").join(size)).map_or(0, |d| d.count())
        };
        let mut asked = Vec::new();
        thumbpipe::write_request(&mut asked, &src, shellview::thumb_px(2)).unwrap();
        let mut answers = Vec::new();
        serve(&decoders, Some(&mut cache), &asked[..], &mut answers).unwrap();
        assert!(
            thumbpipe::read_reply(&mut &answers[..], 256)
                .unwrap()
                .is_some()
        );
        assert_eq!((held("normal"), held("large")), (0, 1));
        thumbnail(&decoders, edge(2), Some(&mut cache), &src, &fs()).expect("cached");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
        thumbnail(&decoders, edge(1), Some(&mut cache), &src, &fs()).expect("decoded");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!((held("normal"), held("large")), (1, 1));
    }

    /// A 1024x512 picture whose red climbs by one every four columns, fitted inside `max`.
    struct Ramp;

    impl crate::decode::ImageDecoder for Ramp {
        fn probe(&self, bytes: &[u8]) -> bool {
            bytes.starts_with(b"RAMP")
        }
        fn decode(&self, _: &[u8], max: (u32, u32)) -> Option<crate::decode::Decoded> {
            let (w, h) = reprisekit::raster::fit_size(1024, 512, max);
            let row = (0..w).map(|x| 0xff00_4080 | (x * 256 / w) << 16);
            let pixels = std::iter::repeat_n(row, h as usize).flatten().collect();
            Some(crate::decode::Decoded {
                canvas: Canvas::from_pixels(w, h, pixels),
                width: 1024,
                height: 512,
            })
        }
    }

    /// The ramp picture's Thumbnails cell at scale 2, drawn from a thumbnail fetched at `edge`,
    /// and the frame's device pixels.
    fn cell_at_scale_2(edge: u32) -> (Canvas, reprisekit::raster::Rect) {
        use reprisekit::controls::shellview::{Entry, Mode, ShellView};
        let dir = tempfile::tempdir().unwrap();
        let pic = dir.path().join("ramp.fake");
        std::fs::write(&pic, b"RAMP").unwrap();
        let mut decoders = Decoders::default_set();
        decoders.register(Box::new(Ramp));
        let (thumb, dims) = thumbnail(&decoders, edge, None, &pic, &fs()).expect("decoded");
        let sv = ShellView {
            mode: Mode::Thumbnails,
            entries: vec![Entry {
                thumb: Some(shellview::frame_thumb(thumb, dims, 2)),
                ..Default::default()
            }],
            ..Default::default()
        };
        let theme = reprisekit::theme::Theme::default();
        let text = reprisekit::text::TextRenderer::load(&theme.fonts);
        let r = reprisekit::raster::Rect::new(0, 0, 140, 150);
        let cm = reprisekit::metrics::ControlMetrics::CLASSIC;
        let lay = shellview::layout(&sv, r, &cm);
        let mut c = Canvas::with_scale(r.width as u32, r.height as u32, 2);
        let px = theme.fonts.size_px;
        shellview::draw(&mut c, &text, &theme.paint(), &sv, &lay, None, px);
        let ramp: Vec<(i32, i32)> = (0..c.pixels.len() as i32)
            .filter(|&i| c.pixels[i as usize] & 0xff00_ffff == 0xff00_4080)
            .map(|i| (i % 280, i / 280))
            .collect();
        let (x0, y0) = ramp
            .iter()
            .fold((i32::MAX, i32::MAX), |m, &(x, y)| (m.0.min(x), m.1.min(y)));
        let (x1, y1) = ramp
            .iter()
            .fold((0, 0), |m, &(x, y)| (m.0.max(x), m.1.max(y)));
        assert_eq!(
            (x1 - x0 + 1, y1 - y0 + 1),
            (188, 94),
            "fitted inside the frame"
        );
        let frame = reprisekit::raster::Rect::new(x0 - 2, y0 - 48, 192, 192);
        let at = |x: i32, y: i32| c.pixels[(y * 280 + x) as usize];
        let (border, inside) = (theme.colors.button_face.argb(), theme.colors.window.argb());
        for (x, y) in [(frame.x, frame.y), (frame.right() - 1, frame.bottom() - 1)] {
            assert_eq!(at(x, y), border, "the frame's 2 px border at {x},{y}");
        }
        assert_eq!(at(frame.x + 2, frame.y + 2), inside);
        assert_eq!(at(frame.x + 1, frame.y + 1), border);
        (c, frame)
    }

    #[test]
    fn a_thumbnail_cell_at_scale_2_draws_every_device_column_of_the_picture() {
        let shades = |edge| {
            let (c, frame) = cell_at_scale_2(edge);
            let y = frame.y + frame.height / 2;
            let row = &c.pixels[(y * 280 + frame.x + 2) as usize..][..188];
            let shades = 1 + row.windows(2).filter(|p| p[0] != p[1]).count();
            let hash = (frame.y..frame.bottom())
                .flat_map(|y| (frame.x..frame.right()).map(move |x| (x, y)))
                .fold(0xcbf2_9ce4_8422_2325u64, |h, (x, y)| {
                    (h ^ u64::from(c.pixels[(y * 280 + x) as usize])).wrapping_mul(0x0100_0000_01b3)
                });
            (shades, hash)
        };
        let (large, hash) = shades(edge_for(shellview::thumb_px(2)));
        assert_eq!(large, 188, "one shade per device column");
        assert_eq!(hash, 0x399e_f26a_352b_475d, "the scale-2 frame");
        let (normal, _) = shades(128);
        assert_eq!(normal, 128, "a 128 px thumbnail stretched over 188 columns");
    }

    #[test]
    fn a_picture_that_failed_is_not_decoded_again_until_it_changes() {
        let (_dir, src, decoders, count, mut cache) = counted(b"FAKE BAD picture");
        assert!(thumbnail(&decoders, 128, Some(&mut cache), &src, &fs()).is_none());
        assert!(thumbnail(&decoders, 128, Some(&mut cache), &src, &fs()).is_none());
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
        std::fs::write(&src, b"FAKE picture, saved again").unwrap();
        assert!(thumbnail(&decoders, 128, Some(&mut cache), &src, &fs()).is_some());
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    /// A read that blocks on a named pipe until a writer opens it, as a read on a dead mount
    /// blocks, is given up after its deadline; past [`MAX_STUCK`] of them every read is refused
    /// at once, until the blocked reads return.
    #[test]
    fn a_read_that_hangs_is_given_up_and_bounded() {
        use std::time::Instant;
        const SHORT: Duration = Duration::from_millis(50);
        let dir = tempfile::tempdir().unwrap();
        let (hang, pic) = (dir.path().join("hang.png"), dir.path().join("dunes.png"));
        std::fs::write(&pic, b"a picture").unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&hang)
                .status()
                .unwrap()
                .success()
        );
        let fs = Deadlines::new(SHORT);
        let read = |p: &std::path::Path| {
            let p = p.to_path_buf();
            fs.run(move || std::fs::read(p))
        };
        assert_eq!(read(&pic).unwrap(), b"a picture");
        for _ in 0..MAX_STUCK {
            let asked = Instant::now();
            assert_eq!(read(&hang).unwrap_err().kind(), io::ErrorKind::TimedOut);
            assert!(
                asked.elapsed() < SHORT * 20,
                "gave up after {:?}",
                asked.elapsed()
            );
        }
        let asked = Instant::now();
        assert_eq!(
            read(&pic).unwrap_err().kind(),
            io::ErrorKind::TimedOut,
            "refused while reads hang"
        );
        assert!(asked.elapsed() < SHORT, "refused at once");

        let end = Instant::now() + Duration::from_secs(5);
        while fs.running.load(Ordering::Acquire) > 0 {
            assert!(Instant::now() < end, "the blocked reads return");
            // A read-write open of a pipe never blocks, and wakes the readers waiting on it.
            let _ = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&hang);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(read(&pic).unwrap(), b"a picture", "reads run again");
    }

    #[test]
    fn a_picture_given_up_on_keeps_its_icon_and_is_tried_again() {
        let (_dir, src, decoders, count, mut cache) = counted(b"FAKE picture");
        let fs = fs();
        fs.running.store(MAX_STUCK, Ordering::Release);
        assert!(thumbnail(&decoders, 128, Some(&mut cache), &src, &fs).is_none());
        assert_eq!(count.load(Ordering::SeqCst), 0, "nothing decoded");
        fs.running.store(0, Ordering::Release);
        assert!(
            thumbnail(&decoders, 128, Some(&mut cache), &src, &fs).is_some(),
            "not marked failed in the cache"
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
