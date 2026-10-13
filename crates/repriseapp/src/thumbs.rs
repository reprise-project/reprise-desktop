//! Pictures' thumbnails for Thumbnails and Tiles, decoded off the event loop by one worker thread
//! through the freedesktop thumbnail cache. Explorer and the file dialog host share it. Navigation
//! bumps a generation counter and the worker drops requests older than the newest it has seen.

use crate::Waker;
use crate::decode::Decoders;
use reprisefs::thumbnails::{Rgba, Thumbnails};
use reprisekit::controls::shellview::THUMB_FRAME;
use reprisekit::raster::Canvas;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::JoinHandle;

pub const FRAME_INNER: u32 = (THUMB_FRAME - 2) as u32;

struct Request {
    path: PathBuf,
    generation: u64,
}

pub struct Done {
    pub path: PathBuf,
    pub thumb: Canvas,
    pub dims: (u32, u32),
    pub generation: u64,
}

pub struct Thumbs {
    tx: Option<Sender<Request>>,
    rx: Receiver<Done>,
    pending: HashSet<PathBuf>,
    generation: u64,
    worker: Option<JoinHandle<()>>,
    done_tx: Sender<Done>,
}

impl Thumbs {
    pub fn new(
        decoders: Arc<Decoders>,
        size: u32,
        cache: Option<Thumbnails>,
        wake: Waker,
    ) -> Thumbs {
        let (tx, req_rx) = channel::<Request>();
        let (done_tx, rx) = channel::<Done>();
        let test_tx = done_tx.clone();
        let edge = reprisefs::thumbnails::edge_for(size);
        let worker = std::thread::Builder::new()
            .name("thumbs".to_string())
            .spawn(move || work(decoders, edge, cache, req_rx, done_tx, wake));
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

    pub fn request(&mut self, path: PathBuf, generation: u64) {
        self.generation = generation;
        if !self.pending.insert(path.clone()) {
            return;
        }
        let Some(tx) = &self.tx else {
            return;
        };
        if tx.send(Request { path, generation }).is_err() {
            log::warn!("the thumbnail worker went away");
            self.tx = None;
        }
    }

    pub fn cancel_all(&mut self) {
        self.pending.clear();
    }

    pub fn drain(&mut self) -> Vec<Done> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(done) => {
                    self.pending.remove(&done.path);
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
    edge: u32,
    mut cache: Option<Thumbnails>,
    rx: Receiver<Request>,
    tx: Sender<Done>,
    wake: Waker,
) {
    let mut queue: VecDeque<Request> = VecDeque::new();
    let mut newest = 0u64;
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
            // is not one) answers nothing, so its path stays in the
            // window's `pending` set for the rest of the listing and is
            // never asked for again. That is deliberate: a retry costs a
            // read and a probe per pass and would never succeed. The
            // next listing clears the set.
            let Some((thumb, dims)) = thumbnail(&decoders, edge, cache.as_mut(), &req.path) else {
                continue;
            };
            let done = Done {
                path: req.path,
                thumb,
                dims,
                generation: req.generation,
            };
            if tx.send(done).is_err() {
                return;
            }
            wake.wake();
        }
    }
}

/// The shared cache under `$XDG_CACHE_HOME/thumbnails`, capped at `max_bytes`.
pub fn open_cache(max_bytes: u64) -> Option<Thumbnails> {
    Some(Thumbnails::at(
        Thumbnails::default_dir()?,
        PROGRAM,
        max_bytes,
    ))
}

/// The cap of a program that has no setting for it.
pub const CACHE_BYTES: u64 = 512 << 20;

const PROGRAM: &str = concat!("reprise-", env!("CARGO_PKG_VERSION"));

/// `path`'s thumbnail at `edge` and its size, from `cache` or decoded into it. A file the stat
/// cannot read skips the cache both ways, so nobody else's thumbnail of
/// it shows. The stat comes before the read, so a picture changed in
/// between is kept under its old stamp and simply misses next time.
fn thumbnail(
    decoders: &Decoders,
    edge: u32,
    cache: Option<&mut Thumbnails>,
    path: &std::path::Path,
) -> Option<(Canvas, (u32, u32))> {
    let stat = std::fs::File::open(path).and_then(|f| f.metadata());
    let cached = cache.zip(stat.ok());
    if let Some((cache, meta)) = &cached {
        // An entry without the picture's size is a miss, since the Tiles
        // line needs it, and is written again with it.
        if let Some(hit) = cache.get(path, meta, edge)
            && let Some(dims) = hit.dims
            && let Ok(thumb) = reprisekit::icontheme::png_to_canvas(&hit.png, edge)
        {
            return Some((thumb, dims));
        }
        if cache.failed(path, meta) {
            return None;
        }
    }
    let img = decoders.decode_file(path, (edge, edge));
    if let Some((cache, meta)) = cached {
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

pub fn for_display(thumb: Canvas, dims: (u32, u32), scale: i32) -> Canvas {
    let s = scale.max(1) as u32;
    let (lw, lh) = crate::decode::fit(dims.0, dims.1, (FRAME_INNER, FRAME_INNER));
    let (w, h) = (lw.max(1) * s, lh.max(1) * s);
    let (sw, sh) = thumb.device_size();
    let device = if (sw, sh) == (w, h) {
        thumb
    } else {
        Canvas::resample_box_from(sw, sh, w, h, |x, y| thumb.pixels[(y * sw + x) as usize])
    };
    device.at_scale(scale)
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

    fn done(path: &str, generation: u64) -> Done {
        Done {
            path: PathBuf::from(path),
            thumb: Canvas::new(1, 1),
            dims: (800, 600),
            generation,
        }
    }

    #[test]
    fn drain_drops_a_thumbnail_from_a_folder_we_have_left() {
        let (tx, mut thumbs) = stub();
        thumbs.request(PathBuf::from("/pics/a.png"), 1);
        thumbs.request(PathBuf::from("/pics/b.png"), 1);
        thumbs.cancel_all();
        thumbs.request(PathBuf::from("/other/c.png"), 2);
        tx.send(done("/pics/b.png", 1)).expect("send");
        tx.send(done("/other/c.png", 2)).expect("send");
        let out = thumbs.drain();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].path, PathBuf::from("/other/c.png"));
    }

    #[test]
    fn the_same_path_is_only_asked_for_once() {
        let (tx, mut thumbs) = stub();
        thumbs.request(PathBuf::from("/pics/a.png"), 1);
        assert!(thumbs.pending.contains(&PathBuf::from("/pics/a.png")));
        thumbs.request(PathBuf::from("/pics/a.png"), 1);
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
        let first = thumbnail(&decoders, 128, Some(&mut cache), &src).expect("decoded");
        let again = thumbnail(&decoders, 128, Some(&mut cache), &src).expect("cached");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(first.0, again.0);
        assert_eq!(again.1, (640, 320));
        thumbnail(&decoders, 128, None, &src).expect("decoded");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn a_picture_that_failed_is_not_decoded_again_until_it_changes() {
        let (_dir, src, decoders, count, mut cache) = counted(b"FAKE BAD picture");
        assert!(thumbnail(&decoders, 128, Some(&mut cache), &src).is_none());
        assert!(thumbnail(&decoders, 128, Some(&mut cache), &src).is_none());
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
        std::fs::write(&src, b"FAKE picture, saved again").unwrap();
        assert!(thumbnail(&decoders, 128, Some(&mut cache), &src).is_some());
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn a_thumbnail_fits_the_frame_at_the_windows_scale() {
        let big = Canvas::from_pixels(256, 192, vec![0xff00_00ff; 256 * 192]);
        let one = for_display(big.clone(), (800, 600), 1);
        assert_eq!((one.width, one.height, one.scale()), (94, 71, 1));
        assert_eq!(one.get(10, 10), 0xff00_00ff);
        let two = for_display(big, (800, 600), 2);
        assert_eq!((two.width, two.height, two.scale()), (94, 71, 2));
        assert_eq!(two.device_size(), (188, 142));
        let small = Canvas::from_pixels(94, 71, vec![0xff00_00ff; 94 * 71]);
        assert_eq!(for_display(small, (800, 600), 2).device_size(), (188, 142));
        let tiny = Canvas::from_pixels(16, 8, vec![1; 128]);
        let t = for_display(tiny.clone(), (16, 8), 1);
        assert_eq!(t, tiny);
        assert_eq!(for_display(tiny, (16, 8), 3).device_size(), (48, 24));
    }
}
