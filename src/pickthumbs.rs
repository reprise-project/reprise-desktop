//! The Browse dialog's thumbnails, fetched on one worker thread: a hit in the shared thumbnail
//! cache is read there, a miss goes to the thumbnail helper (`reprisefs::thumbpipe`), so no
//! picture decoder runs in the compositor. Batches reach the loop through the fsjobs channel.

use crate::fsjobs::OnLoop;
use crate::raster::Canvas;
use reprisefs::thumbnails::Thumbnails;
use reprisefs::thumbpipe;
use smithay::reexports::calloop::channel::Sender as LoopSender;
use std::collections::{HashMap, VecDeque};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

/// A picture, its thumbnail at scale 1 and its own size.
pub type Done = (PathBuf, Canvas, (u32, u32));

/// How long fetched thumbnails wait for company before they go to the loop: each batch
/// redraws the dialog once.
const BATCH: Duration = Duration::from_millis(100);

/// Helper failures in a row after which the rest of the queue goes without it.
const GIVE_UP: u32 = 3;

enum Msg {
    Want {
        dialog: u64,
        generation: u64,
        paths: Vec<PathBuf>,
    },
    Forget(u64),
}

/// The loop's end of the worker, which the first [`PickThumbs::want`] starts.
#[derive(Default)]
pub struct PickThumbs {
    tx: Option<Sender<Msg>>,
}

impl PickThumbs {
    /// Fetches `paths`' thumbnails for listing `generation` of Browse dialog `dialog`, dropping
    /// what is still queued for its earlier listings. Batches run
    /// `State::picker_thumbs` on the loop through `to_loop`.
    pub fn want(
        &mut self,
        to_loop: Option<LoopSender<OnLoop>>,
        dialog: u64,
        generation: u64,
        paths: Vec<PathBuf>,
    ) {
        if self.tx.is_none() {
            let Some(to_loop) = to_loop else {
                return;
            };
            let (tx, rx) = channel();
            let spawned = std::thread::Builder::new()
                .name("pickthumbs".into())
                .spawn(move || work(rx, to_loop));
            match spawned {
                Ok(_) => self.tx = Some(tx),
                Err(e) => {
                    log::warn!("pickthumbs: no worker ({e}); Browse shows icons");
                    return;
                }
            }
        }
        let want = Msg::Want {
            dialog,
            generation,
            paths,
        };
        if self.tx.as_ref().is_some_and(|tx| tx.send(want).is_err()) {
            log::warn!("pickthumbs: the worker went away");
            self.tx = None;
        }
    }

    /// Browse dialog `dialog` closed: what is queued for it is dropped.
    pub fn forget(&mut self, dialog: u64) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Forget(dialog));
        }
    }
}

struct Job {
    dialog: u64,
    generation: u64,
    path: PathBuf,
}

/// The worker's queue: only each dialog's newest listing keeps its jobs.
#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    newest: HashMap<u64, u64>,
}

impl Queue {
    fn take(&mut self, msg: Msg) {
        match msg {
            Msg::Want {
                dialog,
                generation,
                paths,
            } => {
                self.newest.insert(dialog, generation);
                self.jobs.extend(paths.into_iter().map(|path| Job {
                    dialog,
                    generation,
                    path,
                }));
            }
            Msg::Forget(dialog) => {
                self.newest.remove(&dialog);
            }
        }
        let newest = &self.newest;
        self.jobs
            .retain(|j| newest.get(&j.dialog) == Some(&j.generation));
    }

    fn next(&mut self) -> Option<Job> {
        self.jobs.pop_front()
    }

    fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }
}

/// Thumbnails fetched for one listing, not yet on the loop.
#[derive(Default)]
struct Batch {
    dialog: u64,
    generation: u64,
    done: Vec<Done>,
    since: Option<Instant>,
}

impl Batch {
    fn push(&mut self, job: Job, thumb: Canvas, dims: (u32, u32)) {
        self.dialog = job.dialog;
        self.generation = job.generation;
        self.done.push((job.path, thumb, dims));
        self.since.get_or_insert_with(Instant::now);
    }

    fn holds(&self, job: &Job) -> bool {
        self.done.is_empty() || (self.dialog, self.generation) == (job.dialog, job.generation)
    }

    fn due(&self) -> bool {
        self.since.is_some_and(|t| t.elapsed() >= BATCH)
    }

    fn flush(&mut self, to_loop: &LoopSender<OnLoop>) {
        let done = std::mem::take(&mut self.done);
        self.since = None;
        if done.is_empty() {
            return;
        }
        let (dialog, generation) = (self.dialog, self.generation);
        // The loop may have gone meanwhile; the thumbnails go with it.
        let _ = to_loop.send(Box::new(move |state| {
            state.picker_thumbs(dialog, generation, done)
        }));
    }
}

fn work(rx: Receiver<Msg>, to_loop: LoopSender<OnLoop>) {
    let mut cache = None;
    let edge = reprisefs::thumbnails::edge_for(reprisekit::controls::shellview::THUMB_INNER);
    let mut queue = Queue::default();
    let mut batch = Batch::default();
    let mut helper = Fetcher::default();
    let (mut hits, mut asked) = (0u32, 0u32);
    loop {
        if queue.is_empty() {
            batch.flush(&to_loop);
            helper.idle();
            if hits + asked > 0 {
                log::info!("pickthumbs: {hits} from the cache, {asked} asked of the helper");
                (hits, asked) = (0, 0);
            }
            match rx.recv() {
                Ok(msg) => queue.take(msg),
                Err(_) => return,
            }
            cache = cache_for(repriseconf::thumbnail_cache());
        }
        while let Ok(msg) = rx.try_recv() {
            queue.take(msg);
        }
        let Some(job) = queue.next() else {
            continue;
        };
        if !batch.holds(&job) {
            batch.flush(&to_loop);
        }
        let thumb = match cached(cache.as_ref(), &job.path, edge) {
            Some(hit) => {
                hits += 1;
                Some(hit)
            }
            None => {
                asked += 1;
                helper.fetch(&job.path, edge)
            }
        };
        if let Some((thumb, dims)) = thumb {
            batch.push(job, thumb, dims);
        }
        if batch.due() {
            batch.flush(&to_loop);
        }
    }
}

/// The shared cache to read as `setting` (config.toml's `[thumbnail_cache]`) has it: None when
/// it is off. Read only: the helper writes the entries.
fn cache_for(setting: repriseconf::ThumbnailCache) -> Option<Thumbnails> {
    setting
        .enabled
        .then(|| Thumbnails::default_dir().map(|dir| Thumbnails::at(dir, "reprise", 0)))
        .flatten()
}

/// `path`'s thumbnail fitted inside an `edge` square and the picture's size, from `cache`. An
/// entry for an older copy of the file, or without the picture's size (Tiles shows it), is a
/// miss.
fn cached(cache: Option<&Thumbnails>, path: &Path, edge: u32) -> Option<(Canvas, (u32, u32))> {
    let meta = std::fs::metadata(path).ok()?;
    let hit = cache?.get(path, &meta, edge)?;
    let dims = hit.dims?;
    let thumb = reprisekit::icontheme::png_to_canvas(&hit.png, edge).ok()?;
    Some((thumb, dims))
}

/// The helper process, started on the first miss and stopped once the queue is empty.
#[derive(Default)]
struct Fetcher {
    helper: Option<Helper>,
    failures: u32,
}

impl Fetcher {
    fn fetch(&mut self, path: &Path, edge: u32) -> Option<(Canvas, (u32, u32))> {
        if self.failures >= GIVE_UP {
            return None;
        }
        if self.helper.is_none() {
            match Helper::start() {
                Ok(h) => self.helper = Some(h),
                Err(e) => {
                    log::warn!("pickthumbs: cannot start {}: {e}", thumbpipe::PROGRAM);
                    self.failures = GIVE_UP;
                    return None;
                }
            }
        }
        let helper = self.helper.as_mut()?;
        match helper.ask(path, edge) {
            Ok(thumb) => {
                self.failures = 0;
                let t = thumb?;
                Some((Canvas::from_pixels(t.width, t.height, t.pixels), t.dims))
            }
            Err(e) => {
                log::warn!("pickthumbs: the helper failed on {}: {e}", path.display());
                if let Some(h) = self.helper.take() {
                    h.stop(true);
                }
                self.failures += 1;
                None
            }
        }
    }

    fn idle(&mut self) {
        if let Some(h) = self.helper.take() {
            h.stop(false);
        }
        self.failures = 0;
    }
}

struct Helper {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Helper {
    fn start() -> std::io::Result<Helper> {
        use std::os::unix::process::CommandExt;
        let mut c = std::process::Command::new(thumbpipe::PROGRAM);
        c.arg(thumbpipe::FLAG);
        crate::spawn::detach(&mut c);
        c.stdin(Stdio::piped()).stdout(Stdio::piped());
        // SAFETY: the closure runs between fork and exec and only calls setpriority, a system
        // call that does not allocate.
        unsafe {
            c.pre_exec(|| {
                // Decoding gives way to the compositor; failing to lower it is harmless.
                libc::setpriority(libc::PRIO_PROCESS, 0, 10);
                Ok(())
            });
        }
        let mut child = c.spawn()?;
        log::info!(
            "pickthumbs: started the thumbnail helper, pid {}",
            child.id()
        );
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::ErrorKind::BrokenPipe.into());
        };
        Ok(Helper {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    fn ask(&mut self, path: &Path, edge: u32) -> std::io::Result<Option<thumbpipe::Thumb>> {
        thumbpipe::write_request(&mut self.stdin, path)?;
        thumbpipe::read_reply(&mut self.stdout, edge)
    }

    /// Closes the helper's stdin, which ends it between pictures; `kill` ends one that broke
    /// mid-answer.
    fn stop(self, kill: bool) {
        let Helper {
            mut child, stdin, ..
        } = self;
        drop(stdin);
        if kill {
            let _ = child.kill();
        }
        let _ = child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn want(dialog: u64, generation: u64, paths: &[&str]) -> Msg {
        Msg::Want {
            dialog,
            generation,
            paths: paths.iter().map(PathBuf::from).collect(),
        }
    }

    fn drain(q: &mut Queue) -> Vec<(u64, u64, String)> {
        std::iter::from_fn(|| q.next())
            .map(|j| (j.dialog, j.generation, j.path.display().to_string()))
            .collect()
    }

    #[test]
    fn a_new_listing_drops_what_was_queued_for_the_old_one() {
        let mut q = Queue::default();
        q.take(want(1, 1, &["/a/1.png", "/a/2.png"]));
        q.take(want(2, 5, &["/b/1.png"]));
        q.take(want(1, 2, &["/c/1.png"]));
        assert_eq!(
            drain(&mut q),
            [(2, 5, "/b/1.png".into()), (1, 2, "/c/1.png".into())]
        );
    }

    #[test]
    fn a_closed_dialog_drops_its_queue() {
        let mut q = Queue::default();
        q.take(want(1, 1, &["/a/1.png"]));
        q.take(want(2, 1, &["/b/1.png"]));
        q.take(Msg::Forget(1));
        assert_eq!(drain(&mut q), [(2, 1, "/b/1.png".into())]);
        assert!(q.is_empty());
    }

    /// A cache under `dir` holding `pic`'s 2x1 thumbnail at 128 as `pic` stands now.
    fn cache_with(dir: &Path, pic: &Path, dims: (u32, u32)) -> Thumbnails {
        let mut cache = Thumbnails::at(dir.join("thumbnails"), "test", 1 << 20);
        let meta = std::fs::metadata(pic).unwrap();
        let rgba = [255, 0, 0, 255, 0, 0, 255, 255];
        let thumb = reprisefs::thumbnails::Rgba {
            width: 2,
            height: 1,
            pixels: &rgba,
        };
        cache.put(pic, &meta, 128, thumb, dims);
        cache
    }

    #[test]
    fn a_cache_turned_off_is_not_read() {
        let off = repriseconf::ThumbnailCache {
            enabled: false,
            ..repriseconf::ThumbnailCache::DEFAULT
        };
        assert!(cache_for(off).is_none());
    }

    #[test]
    fn a_cache_entry_is_a_hit_until_the_picture_changes() {
        let dir = std::env::temp_dir().join(format!("reprise-pickthumbs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pic = dir.join("dunes.jpg");
        std::fs::write(&pic, b"a picture").unwrap();
        let cache = cache_with(&dir, &pic, (640, 320));
        let (thumb, dims) = cached(Some(&cache), &pic, 128).expect("a hit");
        assert_eq!(dims, (640, 320));
        assert_eq!((thumb.width, thumb.height), (2, 1));
        assert_eq!(thumb.pixels, [0xffff_0000, 0xff00_00ff]);
        assert!(cached(None, &pic, 128).is_none(), "no cache, no hit");
        let other = dir.join("meadow.jpg");
        std::fs::write(&other, b"another picture").unwrap();
        assert!(cached(Some(&cache), &other, 128).is_none(), "no entry");
        let earlier = std::time::SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&pic)
            .unwrap()
            .set_modified(earlier)
            .unwrap();
        assert!(cached(Some(&cache), &pic, 128).is_none(), "a stale mtime");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
