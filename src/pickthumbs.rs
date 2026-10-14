//! The Browse dialog's thumbnails, fetched on one worker thread: a hit in the shared thumbnail
//! cache is read there, a miss goes to the thumbnail helper (`reprisefs::thumbpipe`), so no
//! picture decoder runs in the compositor. Batches reach the loop through the fsjobs channel.

use crate::fsjobs::OnLoop;
use crate::raster::Canvas;
use reprisefs::thumbnails::Thumbnails;
use reprisefs::thumbpipe;
use smithay::reexports::calloop::channel::Sender as LoopSender;
use std::collections::{HashMap, VecDeque};
use std::io::{self, BufReader, Read};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

/// A picture, its thumbnail at scale 1 and its own size.
pub type Done = (PathBuf, Canvas, (u32, u32));

/// How long fetched thumbnails wait for company before they go to the loop: each batch
/// redraws the dialog once.
const BATCH: Duration = Duration::from_millis(100);

/// Helper failures in a row (a crash, a garbled answer, no answer in [`ANSWER_WITHIN`]) after
/// which the rest of the queue goes without it.
const GIVE_UP: u32 = 3;

/// How long the helper may spend on one picture before it is killed and restarted for the rest
/// of the queue: well past a large photo's decode on a slow machine, short enough that a picture
/// on a dead network mount holds the others back only briefly.
const ANSWER_WITHIN: Duration = Duration::from_secs(5);

/// How long the worker waits on a picture's stat before the picture keeps its icon.
const STAT_WITHIN: Duration = Duration::from_secs(2);

/// Stats given up on and still blocked, past which pictures keep their icons without a stat
/// until one returns: each holds a thread.
const MAX_STUCK_STATS: usize = 2;

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
    let mut helper = Fetcher::new(helper_command, ANSWER_WITHIN);
    let stat = Stat::new(|p| std::fs::metadata(p), STAT_WITHIN);
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
        let meta = match stat.of(&job.path) {
            Ok(m) => m,
            Err(e) => {
                if e.kind() == io::ErrorKind::TimedOut {
                    log::warn!("pickthumbs: {}: {e}", job.path.display());
                }
                continue;
            }
        };
        let thumb = match cached(cache.as_ref(), &job.path, &meta, edge) {
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
fn cached(
    cache: Option<&Thumbnails>,
    path: &Path,
    meta: &std::fs::Metadata,
    edge: u32,
) -> Option<(Canvas, (u32, u32))> {
    let hit = cache?.get(path, meta, edge)?;
    let dims = hit.dims?;
    let thumb = reprisekit::icontheme::png_to_canvas(&hit.png, edge).ok()?;
    Some((thumb, dims))
}

/// A picture's stat on a thread of its own, given up after `within`: a picture on a dead mount
/// holds the worker that long at most.
struct Stat {
    stat: fn(&Path) -> io::Result<std::fs::Metadata>,
    within: Duration,
    /// Stats started and not yet returned.
    running: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Stat {
    fn new(stat: fn(&Path) -> io::Result<std::fs::Metadata>, within: Duration) -> Stat {
        Stat {
            stat,
            within,
            running: Default::default(),
        }
    }

    /// `path`'s metadata, or `TimedOut` when it takes longer than `within` or
    /// [`MAX_STUCK_STATS`] earlier stats are still blocked.
    fn of(&self, path: &Path) -> io::Result<std::fs::Metadata> {
        use std::sync::atomic::Ordering;
        if self.running.load(Ordering::Acquire) >= MAX_STUCK_STATS {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{MAX_STUCK_STATS} stats have not returned"),
            ));
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let (stat, running, path) = (self.stat, self.running.clone(), path.to_path_buf());
        running.fetch_add(1, Ordering::AcqRel);
        let spawned = std::thread::Builder::new()
            .name("pickthumbs-stat".into())
            .spawn(move || {
                let meta = stat(&path);
                running.fetch_sub(1, Ordering::AcqRel);
                let _ = tx.send(meta);
            });
        if let Err(e) = spawned {
            self.running.fetch_sub(1, Ordering::AcqRel);
            return Err(e);
        }
        rx.recv_timeout(self.within)
            .unwrap_or_else(|_| Err(io::ErrorKind::TimedOut.into()))
    }
}

/// The helper process, started on the first miss and stopped once the queue is empty.
struct Fetcher {
    helper: Option<Helper>,
    failures: u32,
    command: fn() -> Command,
    within: Duration,
}

impl Fetcher {
    fn new(command: fn() -> Command, within: Duration) -> Fetcher {
        Fetcher {
            helper: None,
            failures: 0,
            command,
            within,
        }
    }

    fn fetch(&mut self, path: &Path, edge: u32) -> Option<(Canvas, (u32, u32))> {
        if self.failures >= GIVE_UP {
            return None;
        }
        if self.helper.is_none() {
            match Helper::start((self.command)()) {
                Ok(h) => self.helper = Some(h),
                Err(e) => {
                    log::warn!("pickthumbs: cannot start {}: {e}", thumbpipe::PROGRAM);
                    self.failures = GIVE_UP;
                    return None;
                }
            }
        }
        let helper = self.helper.as_mut()?;
        match helper.ask(path, edge, self.within) {
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

fn helper_command() -> Command {
    use std::os::unix::process::CommandExt;
    let mut c = Command::new(thumbpipe::PROGRAM);
    c.arg(thumbpipe::FLAG);
    crate::spawn::detach(&mut c);
    // SAFETY: the closure runs between fork and exec and only calls setpriority, a system call
    // that does not allocate.
    unsafe {
        c.pre_exec(|| {
            // Decoding gives way to the compositor; failing to lower it is harmless.
            libc::setpriority(libc::PRIO_PROCESS, 0, 10);
            Ok(())
        });
    }
    c
}

struct Helper {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<Deadline>,
}

impl Helper {
    fn start(mut c: Command) -> io::Result<Helper> {
        c.stdin(Stdio::piped()).stdout(Stdio::piped());
        let mut child = c.spawn()?;
        log::info!(
            "pickthumbs: started the thumbnail helper, pid {}",
            child.id()
        );
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::ErrorKind::BrokenPipe.into());
        };
        let stdout = Deadline {
            out: stdout,
            until: Instant::now(),
        };
        Ok(Helper {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    /// `path`'s thumbnail, failing with `TimedOut` when the answer takes longer than `within`.
    fn ask(
        &mut self,
        path: &Path,
        edge: u32,
        within: Duration,
    ) -> io::Result<Option<thumbpipe::Thumb>> {
        thumbpipe::write_request(&mut self.stdin, path)?;
        self.stdout.get_mut().until = Instant::now() + within;
        thumbpipe::read_reply(&mut self.stdout, edge)
    }

    /// Closes the helper's stdin, which ends it between pictures; `kill` ends one that broke or
    /// hung mid-answer.
    fn stop(self, kill: bool) {
        let Helper {
            mut child, stdin, ..
        } = self;
        drop(stdin);
        if !kill {
            let _ = child.wait();
            return;
        }
        let _ = child.kill();
        // A helper blocked on a dead mount may outlive SIGKILL for a while; the queue goes on.
        let _ = std::thread::Builder::new()
            .name("pickthumbs-reap".into())
            .spawn(move || child.wait());
    }
}

/// The helper's stdout: a read waiting past `until` fails with `TimedOut`.
struct Deadline {
    out: ChildStdout,
    until: Instant,
}

impl Read for Deadline {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.until.saturating_duration_since(Instant::now());
        let mut fd = libc::pollfd {
            fd: self.out.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = i32::try_from(left.as_millis()).unwrap_or(i32::MAX);
        // SAFETY: `fd` is one valid pollfd that outlives the call.
        match unsafe { libc::poll(&mut fd, 1, ms) } {
            0 => Err(io::ErrorKind::TimedOut.into()),
            // EINTR comes back as `Interrupted`, which `read_exact` retries.
            n if n < 0 => Err(io::Error::last_os_error()),
            _ => self.out.read(buf),
        }
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

    /// A helper answering each path with a 1x1 thumbnail of a 2x3 picture, which hangs on a
    /// path holding "hang".
    fn fake_helper() -> Command {
        let mut c = Command::new("bash");
        c.args([
            "-c",
            r"while IFS= read -r -d '' p; do
                case $p in *hang*) exec sleep 60;; esac
                printf '\1\0\0\0\1\0\0\0\2\0\0\0\3\0\0\0\377\0\0\377'
            done",
        ]);
        c
    }

    const WITHIN: Duration = Duration::from_millis(200);

    fn fetched(f: &mut Fetcher, path: &str) -> bool {
        f.fetch(Path::new(path), 128)
            .inspect(|(thumb, dims)| {
                assert_eq!(*dims, (2, 3));
                assert_eq!(thumb.pixels, [0xff00_00ff]);
            })
            .is_some()
    }

    fn helper_pid(f: &Fetcher) -> Option<u32> {
        f.helper.as_ref().map(|h| h.child.id())
    }

    #[test]
    fn a_picture_the_helper_hangs_on_is_skipped_and_the_rest_arrive() {
        let mut f = Fetcher::new(fake_helper, WITHIN);
        assert!(fetched(&mut f, "/pics/a.jpg"));
        let hung = helper_pid(&f).expect("a helper");
        let asked = Instant::now();
        assert!(!fetched(&mut f, "/pics/hang.jpg"));
        assert!(
            asked.elapsed() < WITHIN * 5,
            "gave up after {:?}",
            asked.elapsed()
        );
        assert!(f.helper.is_none(), "the hung helper is dropped");
        let proc = PathBuf::from(format!("/proc/{hung}"));
        let killed = Instant::now();
        while proc.exists() && killed.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!proc.exists(), "the hung helper is killed and reaped");
        assert!(fetched(&mut f, "/pics/b.jpg"));
        assert!(fetched(&mut f, "/pics/c.jpg"));
        assert_ne!(
            helper_pid(&f),
            Some(hung),
            "a fresh helper answers the rest"
        );
    }

    #[test]
    fn three_hangs_in_a_row_give_up_the_rest_of_the_queue() {
        let mut f = Fetcher::new(fake_helper, WITHIN);
        assert!(!fetched(&mut f, "/pics/hang-1.jpg"));
        assert!(!fetched(&mut f, "/pics/hang-2.jpg"));
        assert!(fetched(&mut f, "/pics/a.jpg"), "an answer resets the count");
        for n in 3..=5 {
            assert!(!fetched(&mut f, &format!("/pics/hang-{n}.jpg")));
        }
        assert!(!fetched(&mut f, "/pics/b.jpg"));
        assert!(f.helper.is_none(), "no helper is started after giving up");
        f.idle();
        assert!(fetched(&mut f, "/pics/c.jpg"), "the next queue tries again");
    }

    /// A stat that blocks on a named pipe until a writer opens it, as a stat on a dead mount
    /// blocks.
    fn open_then_stat(p: &Path) -> io::Result<std::fs::Metadata> {
        std::fs::File::open(p)?;
        std::fs::metadata(p)
    }

    /// A stat that does not return is given up after its deadline; past [`MAX_STUCK_STATS`] of
    /// them every picture is given up at once, until the blocked stats return.
    #[test]
    fn a_stat_that_hangs_is_given_up_and_bounded() {
        use std::os::unix::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("reprise-pickstat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (hang, pic) = (dir.join("hang.jpg"), dir.join("dunes.jpg"));
        std::fs::write(&pic, b"a picture").unwrap();
        let c = std::ffi::CString::new(hang.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: `c` is a valid NUL-terminated path for the call's duration.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let stat = Stat::new(open_then_stat, WITHIN);
        assert!(stat.of(&pic).is_ok());
        for _ in 0..MAX_STUCK_STATS {
            let asked = Instant::now();
            let e = stat.of(&hang).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::TimedOut);
            assert!(
                asked.elapsed() < WITHIN * 5,
                "gave up after {:?}",
                asked.elapsed()
            );
        }
        let asked = Instant::now();
        let e = stat.of(&pic).unwrap_err();
        assert_eq!(
            e.kind(),
            io::ErrorKind::TimedOut,
            "refused while stats hang"
        );
        assert!(asked.elapsed() < WITHIN, "refused at once");

        let end = Instant::now() + Duration::from_secs(5);
        while stat.running.load(std::sync::atomic::Ordering::Acquire) > 0 {
            assert!(Instant::now() < end, "the blocked stats return");
            let _ = std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&hang);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(stat.of(&pic).is_ok(), "stats run again");
        let _ = std::fs::remove_dir_all(&dir);
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
        let meta = |p: &Path| std::fs::metadata(p).unwrap();
        let (thumb, dims) = cached(Some(&cache), &pic, &meta(&pic), 128).expect("a hit");
        assert_eq!(dims, (640, 320));
        assert_eq!((thumb.width, thumb.height), (2, 1));
        assert_eq!(thumb.pixels, [0xffff_0000, 0xff00_00ff]);
        assert!(
            cached(None, &pic, &meta(&pic), 128).is_none(),
            "no cache, no hit"
        );
        let other = dir.join("meadow.jpg");
        std::fs::write(&other, b"another picture").unwrap();
        assert!(
            cached(Some(&cache), &other, &meta(&other), 128).is_none(),
            "no entry"
        );
        let earlier = std::time::SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&pic)
            .unwrap()
            .set_modified(earlier)
            .unwrap();
        assert!(
            cached(Some(&cache), &pic, &meta(&pic), 128).is_none(),
            "a stale mtime"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
