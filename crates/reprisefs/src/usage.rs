//! What takes a folder's space: [`Scan`] counts it on a pool of threads into a [`Tree`] of on-disk
//! sizes. A mount point is never looked at, so a dead mount under the folder holds nothing up.

use reprisebus::mutex::lock;
use std::collections::{HashSet, VecDeque};
use std::ffi::OsString;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub type NodeId = usize;

pub const ROOT: NodeId = 0;

/// How often a scan's progress wakes its host at most.
const NOTIFY_EVERY: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File,
    Link,
    /// Another file system mounted here: not entered, counted as nothing.
    Mount,
    /// A device, socket or pipe.
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub name: OsString,
    pub parent: Option<NodeId>,
    pub kind: Kind,
    /// Bytes on disk of the entry and, for a folder, of everything found under it so far.
    pub size: u64,
    /// Files and links found under a folder, or 1 for one.
    pub files: u64,
    pub children: Vec<NodeId>,
    /// Listings still to come under a folder: its own and each unfinished subfolder's.
    open: u32,
    /// A folder whose listing failed (no permission).
    pub unreadable: bool,
}

impl Node {
    fn new(name: OsString, parent: Option<NodeId>, kind: Kind, size: u64) -> Node {
        Node {
            name,
            parent,
            kind,
            size,
            files: u64::from(matches!(kind, Kind::File | Kind::Link | Kind::Other)),
            children: Vec::new(),
            open: u32::from(kind == Kind::Dir),
            unreadable: false,
        }
    }

    /// The folder and everything under it is counted.
    pub fn done(&self) -> bool {
        self.open == 0
    }
}

/// How [`Tree::sorted`] orders a folder's entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Name,
    Size,
    Files,
}

#[derive(Clone, Debug)]
pub struct Tree {
    pub root: PathBuf,
    nodes: Vec<Node>,
}

impl Tree {
    fn new(root: PathBuf) -> Tree {
        let name = root.file_name().unwrap_or(root.as_os_str()).to_os_string();
        Tree {
            root,
            nodes: vec![Node::new(name, None, Kind::Dir, 0)],
        }
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    pub fn path(&self, id: NodeId) -> PathBuf {
        let mut names = Vec::new();
        let mut at = id;
        while let Some(up) = self.nodes[at].parent {
            names.push(self.nodes[at].name.as_os_str());
            at = up;
        }
        let mut p = self.root.clone();
        p.extend(names.into_iter().rev());
        p
    }

    /// The node at `path`, a folder at or under the root.
    pub fn find(&self, path: &Path) -> Option<NodeId> {
        let rest = path.strip_prefix(&self.root).ok()?;
        rest.iter().try_fold(ROOT, |at, name| {
            (self.nodes[at].children.iter().copied()).find(|&c| self.nodes[c].name == name)
        })
    }

    /// `id`'s entries by `by`, largest (or A) first and ties by name; the other way round
    /// when `flipped`.
    pub fn sorted(&self, id: NodeId, by: Sort, flipped: bool) -> Vec<NodeId> {
        let mut out = self.nodes[id].children.clone();
        let name = |c: NodeId| self.nodes[c].name.to_string_lossy().to_lowercase();
        out.sort_by(|&a, &b| {
            let (na, nb) = (&self.nodes[a], &self.nodes[b]);
            let key = match by {
                Sort::Name => std::cmp::Ordering::Equal,
                Sort::Size => nb.size.cmp(&na.size),
                Sort::Files => nb.files.cmp(&na.files),
            };
            key.then_with(|| name(a).cmp(&name(b)))
        });
        if flipped {
            out.reverse();
        }
        out
    }

    /// Takes `id` (gone from disk) out of its folder, its size and files with it.
    pub fn remove(&mut self, id: NodeId) {
        let Some(parent) = self.nodes[id].parent else {
            return;
        };
        let (size, files) = (self.nodes[id].size, self.nodes[id].files);
        self.nodes[parent].children.retain(|&c| c != id);
        let mut at = Some(parent);
        while let Some(a) = at {
            let n = &mut self.nodes[a];
            n.size = n.size.saturating_sub(size);
            n.files = n.files.saturating_sub(files);
            at = n.parent;
        }
    }

    fn add(&mut self, parent: NodeId, node: Node) -> NodeId {
        let (size, files) = (node.size, node.files);
        let id = self.nodes.len();
        self.nodes.push(node);
        self.nodes[parent].children.push(id);
        let mut at = Some(parent);
        while let Some(a) = at {
            let n = &mut self.nodes[a];
            n.size += size;
            n.files += files;
            at = n.parent;
        }
        id
    }

    /// `id`'s own listing is in, `subdirs` of its entries still to list.
    fn listed(&mut self, id: NodeId, subdirs: u32) {
        self.nodes[id].open += subdirs;
        let mut at = Some(id);
        while let Some(a) = at {
            let n = &mut self.nodes[a];
            n.open -= 1;
            if n.open > 0 {
                return;
            }
            at = n.parent;
        }
    }
}

struct Work {
    tree: Mutex<Tree>,
    queue: Mutex<Queue>,
    more: Condvar,
    cancel: AtomicBool,
    seen: Mutex<HashSet<(u64, u64)>>,
    mounts: HashSet<PathBuf>,
    dev: Mutex<Option<u64>>,
    notified: Mutex<Instant>,
    notify: Box<dyn Fn() + Send + Sync>,
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<(NodeId, PathBuf)>,
    busy: usize,
}

/// A running scan, cancelled when dropped.
pub struct Scan {
    work: Arc<Work>,
}

impl Scan {
    /// Starts counting `root` on `threads` threads (the system's mount points left out),
    /// calling `notify` from them as sizes come in and once when the scan ends.
    pub fn start(root: &Path, threads: usize, notify: impl Fn() + Send + Sync + 'static) -> Scan {
        let table = std::fs::read("/proc/self/mounts").unwrap_or_default();
        let mounts = crate::places::mount_table(&table)
            .into_iter()
            .map(|(m, _)| m);
        Scan::start_with(root, threads, mounts.collect(), Box::new(notify))
    }

    fn start_with(
        root: &Path,
        threads: usize,
        mounts: HashSet<PathBuf>,
        notify: Box<dyn Fn() + Send + Sync>,
    ) -> Scan {
        let work = Arc::new(Work {
            tree: Mutex::new(Tree::new(root.to_path_buf())),
            queue: Mutex::new(Queue {
                jobs: VecDeque::from([(ROOT, root.to_path_buf())]),
                busy: 0,
            }),
            more: Condvar::new(),
            cancel: AtomicBool::new(false),
            seen: Mutex::new(HashSet::new()),
            mounts,
            dev: Mutex::new(None),
            notified: Mutex::new(Instant::now()),
            notify,
        });
        for _ in 0..threads.max(1) {
            let w = work.clone();
            let spawned = std::thread::Builder::new()
                .name("usage-scan".into())
                .spawn(move || run(&w));
            if let Err(e) = spawned {
                log::warn!("usage: no scan thread: {e}");
            }
        }
        Scan { work }
    }

    pub fn tree(&self) -> MutexGuard<'_, Tree> {
        lock(&self.work.tree)
    }

    /// The whole folder is counted.
    pub fn done(&self) -> bool {
        self.tree().node(ROOT).done()
    }

    /// Whether the folder itself has been looked at: false while it does not answer.
    pub fn answered(&self) -> bool {
        lock(&self.work.dev).is_some() || self.done()
    }
}

impl Drop for Scan {
    fn drop(&mut self) {
        self.work.cancel.store(true, Ordering::Release);
        let _q = lock(&self.work.queue);
        self.work.more.notify_all();
    }
}

/// The pool's threads: each takes a folder, lists it and queues its subfolders, until no
/// folder is queued or being listed.
fn run(w: &Work) {
    loop {
        let (id, path) = {
            let mut q = lock(&w.queue);
            loop {
                if w.cancel.load(Ordering::Acquire) {
                    return;
                }
                if let Some(job) = q.jobs.pop_front() {
                    q.busy += 1;
                    break job;
                }
                if q.busy == 0 {
                    w.more.notify_all();
                    return;
                }
                q = w.more.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        let subdirs = list(w, id, &path);
        let mut q = lock(&w.queue);
        q.jobs.extend(subdirs);
        q.busy -= 1;
        let ended = q.jobs.is_empty() && q.busy == 0;
        w.more.notify_all();
        drop(q);
        let mut last = lock(&w.notified);
        if ended || last.elapsed() >= NOTIFY_EVERY {
            *last = Instant::now();
            drop(last);
            (w.notify)();
        }
    }
}

/// Lists folder `id` at `path` into the tree, returning its subfolders to list.
fn list(w: &Work, id: NodeId, path: &Path) -> Vec<(NodeId, PathBuf)> {
    if id == ROOT {
        match std::fs::metadata(path) {
            Ok(m) => *lock(&w.dev) = Some(m.dev()),
            Err(_) => {
                let mut t = lock(&w.tree);
                t.nodes[ROOT].unreadable = true;
                t.listed(ROOT, 0);
                return Vec::new();
            }
        }
    }
    let dev = lock(&w.dev).unwrap_or(0);
    let mut found = Vec::new();
    let mut unreadable = false;
    match std::fs::read_dir(path) {
        Ok(rd) => {
            for e in rd.flatten() {
                if w.cancel.load(Ordering::Acquire) {
                    break;
                }
                found.push(entry(w, &e.path(), e.file_name(), dev));
            }
        }
        Err(_) => unreadable = true,
    }
    let mut t = lock(&w.tree);
    t.nodes[id].unreadable = unreadable;
    let mut subdirs = Vec::new();
    for (name, kind, size, sub) in found {
        let child = t.add(id, Node::new(name, Some(id), kind, size));
        if let Some(sub) = sub {
            subdirs.push((child, sub));
        }
    }
    t.listed(id, subdirs.len() as u32);
    subdirs
}

type Found = (OsString, Kind, u64, Option<PathBuf>);

fn entry(w: &Work, path: &Path, name: OsString, dev: u64) -> Found {
    if w.mounts.contains(path) {
        return (name, Kind::Mount, 0, None);
    }
    let Ok(m) = std::fs::symlink_metadata(path) else {
        return (name, Kind::Other, 0, None);
    };
    let ft = m.file_type();
    let size = m.blocks() * 512;
    if ft.is_dir() {
        if m.dev() != dev {
            return (name, Kind::Mount, 0, None);
        }
        return (name, Kind::Dir, size, Some(path.to_path_buf()));
    }
    let size = match m.nlink() > 1 && !lock(&w.seen).insert((m.dev(), m.ino())) {
        true => 0,
        false => size,
    };
    let kind = match () {
        _ if ft.is_symlink() => Kind::Link,
        _ if ft.is_file() => Kind::File,
        _ => Kind::Other,
    };
    (name, kind, size, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::mpsc;

    fn scan(root: &Path, mounts: &[PathBuf]) -> Tree {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let s = Scan::start_with(
            root,
            3,
            mounts.iter().cloned().collect(),
            Box::new(move || {
                let _ = lock(&tx).send(());
            }),
        );
        let until = Instant::now() + Duration::from_secs(10);
        while !s.done() {
            assert!(Instant::now() < until, "the scan ends");
            let _ = rx.recv_timeout(Duration::from_millis(50));
        }
        s.tree().clone()
    }

    fn blocks(p: &Path) -> u64 {
        fs::symlink_metadata(p).unwrap().blocks() * 512
    }

    fn named(t: &Tree, path: &str) -> NodeId {
        t.find(&t.root.join(path))
            .unwrap_or_else(|| panic!("{path} found"))
    }

    #[test]
    fn a_folder_counts_what_is_on_disk_under_it_and_a_hard_link_once() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        fs::create_dir_all(r.join("a/deep")).unwrap();
        fs::create_dir(r.join("b")).unwrap();
        fs::write(r.join("a/deep/big"), vec![7u8; 64 * 1024]).unwrap();
        fs::write(r.join("b/small"), b"x").unwrap();
        fs::hard_link(r.join("a/deep/big"), r.join("b/link")).unwrap();
        let t = scan(r, &[]);
        let big = blocks(&r.join("a/deep/big"));
        assert!(big >= 64 * 1024);
        let dirs = [".", "a", "a/deep", "b"].map(|p| blocks(&r.join(p)));
        let small = blocks(&r.join("b/small"));
        assert_eq!(
            t.node(ROOT).size,
            dirs[1] + dirs[2] + dirs[3] + big + small,
            "the link's blocks once; the root's own entry is not its content"
        );
        assert_eq!(t.node(ROOT).files, 3);
        let (a, b) = (named(&t, "a"), named(&t, "b"));
        assert_eq!(t.node(a).size + t.node(b).size, t.node(ROOT).size);
        assert!(t.node(ROOT).done() && t.node(a).done());
        assert_eq!(t.path(named(&t, "a/deep")), r.join("a/deep"));
    }

    #[test]
    fn a_link_loop_is_counted_as_a_link_and_not_followed() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        fs::create_dir(r.join("a")).unwrap();
        std::os::unix::fs::symlink("..", r.join("a/up")).unwrap();
        std::os::unix::fs::symlink(r, r.join("a/abs")).unwrap();
        let t = scan(r, &[]);
        let up = named(&t, "a/up");
        assert_eq!(t.node(up).kind, Kind::Link);
        assert!(t.node(up).children.is_empty());
        assert_eq!(t.node(ROOT).files, 2);
    }

    #[test]
    fn a_mount_point_under_the_folder_is_not_entered() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        fs::create_dir_all(r.join("media/stick")).unwrap();
        fs::write(r.join("media/stick/f"), vec![1u8; 9000]).unwrap();
        let t = scan(r, &[r.join("media/stick")]);
        let stick = named(&t, "media/stick");
        assert_eq!(t.node(stick).kind, Kind::Mount);
        assert_eq!(t.node(stick).size, 0);
        assert!(t.node(stick).children.is_empty());
        assert_eq!(t.node(ROOT).files, 0);
    }

    #[test]
    fn another_device_under_the_folder_is_not_entered() {
        // /dev holds /dev/pts and /dev/shm on file systems of their own where they are mounted.
        let (dev, pts) = (Path::new("/dev"), Path::new("/dev/pts"));
        let other = |p: &Path| fs::metadata(p).map(|m| m.dev());
        if other(pts).ok().is_none() || other(pts).ok() == other(dev).ok() {
            return;
        }
        let t = scan(dev, &[]);
        let n = t.find(pts).unwrap();
        assert_eq!(t.node(n).kind, Kind::Mount);
        assert!(t.node(n).children.is_empty());
    }

    #[test]
    fn entries_sort_by_size_files_or_name_with_ties_by_name() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        fs::write(r.join("b"), vec![0u8; 40_000]).unwrap();
        fs::write(r.join("a"), vec![0u8; 40_000]).unwrap();
        fs::write(r.join("C"), b"c").unwrap();
        fs::create_dir(r.join("d")).unwrap();
        fs::write(r.join("d/1"), b"1").unwrap();
        fs::write(r.join("d/2"), b"2").unwrap();
        let t = scan(r, &[]);
        let names = |ids: Vec<NodeId>| -> Vec<String> {
            ids.iter()
                .map(|&i| t.node(i).name.to_string_lossy().into_owned())
                .collect()
        };
        assert_eq!(names(t.sorted(ROOT, Sort::Size, false))[..2], ["a", "b"]);
        assert_eq!(
            names(t.sorted(ROOT, Sort::Name, false)),
            ["a", "b", "C", "d"]
        );
        assert_eq!(
            names(t.sorted(ROOT, Sort::Name, true)),
            ["d", "C", "b", "a"]
        );
        assert_eq!(names(t.sorted(ROOT, Sort::Files, false))[0], "d");
        let size_up = names(t.sorted(ROOT, Sort::Size, true));
        assert_eq!(&size_up[2..], ["b", "a"], "smallest first");
    }

    #[test]
    fn a_removed_entry_takes_its_size_off_every_folder_above() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        fs::create_dir_all(r.join("a/b")).unwrap();
        fs::write(r.join("a/b/f"), vec![0u8; 20_000]).unwrap();
        fs::write(r.join("g"), b"g").unwrap();
        let mut t = scan(r, &[]);
        let (root, a) = (t.node(ROOT).size, t.node(named(&t, "a")).size);
        let b = named(&t, "a/b");
        let gone = t.node(b).size;
        t.remove(b);
        assert_eq!(t.node(ROOT).size, root - gone);
        assert_eq!(t.node(named(&t, "a")).size, a - gone);
        assert_eq!(t.node(ROOT).files, 1);
        assert_eq!(t.find(&r.join("a/b")), None);
    }

    #[test]
    fn a_folder_that_cannot_be_read_ends_the_scan_marked() {
        let d = tempfile::tempdir().unwrap();
        let t = scan(&d.path().join("missing"), &[]);
        assert!(t.node(ROOT).unreadable);
        assert!(t.node(ROOT).done());
    }

    #[test]
    fn a_dropped_scan_stops_its_threads() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..50 {
            fs::create_dir_all(d.path().join(format!("{i}/x/y"))).unwrap();
        }
        let s = Scan::start_with(d.path(), 2, HashSet::new(), Box::new(|| {}));
        let work = Arc::downgrade(&s.work);
        drop(s);
        let until = Instant::now() + Duration::from_secs(5);
        while work.strong_count() > 0 {
            assert!(Instant::now() < until, "the threads let go of the scan");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
