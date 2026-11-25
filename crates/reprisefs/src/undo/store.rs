//! The session's one undo history: every Explorer window, the desktop and Explorer's file jobs
//! push to it and undo from it, so Undo anywhere reverses the most recent file operation done
//! anywhere.
//!
//! It is the file `$XDG_RUNTIME_DIR/reprise-undo`, holding the newest [`super::CAP`] entries that
//! fit in [`MAX_SIZE`] and rewritten whole through a rename under an exclusive lock on
//! `reprise-undo.lock`; readers take no lock and reparse only when the file changed. The Undo
//! row's label and the id of the entry it names sit at its head ([`SharedUndo::head`]), so a
//! host's loop shows the row without reading the rest, and the row takes that entry alone. An
//! undo holds `reprise-undo.run` until what it leaves is back ([`Running`]), so another window's
//! Undo does nothing meanwhile ([`SharedUndo::held`]) instead of reversing the entry below a
//! half-reversed one. Without a runtime directory the history is the process's own.
//!
//! An edit acts only on the history as it reread it under the lock, and counts only once written
//! back. One the lock keeps out past [`PATIENCE`], or that cannot read or write the file, changes
//! nothing, and then:
//! - a take returns nothing: that Undo does nothing, as does one that cannot open `.run`;
//! - a push, `forget_gone` or `retain_non_trash` is dropped: every step checks its item before it
//!   acts, so Undo reaching past a lost entry still never reverses the wrong item;
//! - a put-back or a clear is retried on a thread of its own for up to [`RETRY_FOR`], holding
//!   `.run` so that no Undo reaches past it meanwhile: a put-back keeps its undo's hold, a clear
//!   takes `.run` once a running undo ends, however long it runs, and marks it with
//!   `.clearing` ([`Hold::Clear`]). A process about to quit waits for it ([`SharedUndo::settle`]).

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{EntryId, FileId, Landed, Undo, UndoStack, Undoable};
use crate::trash::Trashed;

/// How long an edit waits for another process's, which only rewrites a small file.
const PATIENCE: Duration = Duration::from_millis(200);

/// How long a put-back or a clear the lock kept out keeps trying.
const RETRY_FOR: Duration = Duration::from_secs(30);

const MAGIC: &[u8] = b"reprise-undo 5";

/// Every [`Undoable`] label, as [`SharedUndo::label`] reads one back from the history's head.
const LABELS: [&str; 5] = [
    "Undo Move",
    "Undo Rename",
    "Undo Delete",
    "Undo Copy",
    "Undo New",
];

/// Past [`MAGIC`], the longest of [`LABELS`] and the longest [`EntryId`].
const HEAD: u64 = 128;

/// A batch holds single steps; one level more is room to spare, and anything deeper is not a
/// history Reprise wrote.
const MAX_NESTING: usize = 2;

/// Far past what [`super::CAP`] entries of real batches take. A bigger file is unreadable, so
/// [`encode`] drops the oldest entries to stay within it.
const MAX_SIZE: u64 = 64 << 20;

/// The history and the file it lives in, read again whenever another process rewrote it. A clone
/// is another handle on the same history whose edits wait on nothing the original holds, so a
/// host hands one to a thread to edit off its loop.
pub struct SharedUndo {
    file: Option<PathBuf>,
    seen: Arc<Mutex<Seen>>,
    /// The threads retrying a put-back or a clear, shared by every clone.
    retries: Arc<Mutex<Vec<JoinHandle<()>>>>,
    /// [`RETRY_FOR`], shorter in tests.
    retry_for: Duration,
}

impl Default for SharedUndo {
    fn default() -> Self {
        SharedUndo {
            file: None,
            seen: Arc::default(),
            retries: Arc::default(),
            retry_for: RETRY_FOR,
        }
    }
}

impl Clone for SharedUndo {
    fn clone(&self) -> Self {
        let (retries, retry_for) = (self.retries.clone(), self.retry_for);
        match &self.file {
            Some(file) => SharedUndo {
                retries,
                retry_for,
                ..SharedUndo::at(file.clone())
            },
            None => SharedUndo {
                file: None,
                seen: self.seen.clone(),
                retries,
                retry_for,
            },
        }
    }
}

#[derive(Default)]
struct Seen {
    stamp: Option<Stamp>,
    stack: UndoStack,
}

/// The file's device, inode, change time and size: every write renames a new inode in.
type Stamp = (u64, u64, i64, i64, u64);

/// Held while an undo taken off a [`SharedUndo`] runs; [`SharedUndo::put_back`] lets the next
/// Undo in once what it left is back.
pub struct Running {
    /// A retried clear's mark beside `.run` ([`Hold::Clear`]), let go before `.run`.
    _clearing: Option<File>,
    _lock: Option<File>,
}

/// What holds every Undo off ([`SharedUndo::held`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hold {
    /// An undo runs, or its put-back is retried.
    Undo,
    /// A clear the lock kept out waits to land.
    Clear,
}

impl SharedUndo {
    /// The session's history in `$XDG_RUNTIME_DIR`, or the process's own without one.
    pub fn session() -> Self {
        match std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            Some(dir) => Self::at(dir.join("reprise-undo")),
            None => Self::default(),
        }
    }

    pub fn at(file: PathBuf) -> Self {
        SharedUndo {
            file: Some(file),
            ..SharedUndo::default()
        }
    }

    fn seen(&self) -> MutexGuard<'_, Seen> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn warn(&self, what: &str, e: &io::Error) {
        let file = self.file.as_deref().unwrap_or(Path::new("-"));
        log::warn!("reprisefs: undo history {}: {what}: {e}", file.display());
    }

    fn read<R>(&self, f: impl FnOnce(&UndoStack) -> R) -> R {
        let mut seen = self.seen();
        if let Some(file) = &self.file
            && let Err(e) = seen.refresh(file)
        {
            self.warn("read", &e);
        }
        f(&seen.stack)
    }

    /// `f` on the history as the file holds it under the lock, written back before the lock goes;
    /// an error, with nothing changed, when the lock stays held past [`PATIENCE`] or the file
    /// cannot be read or written.
    fn edit<R>(&self, f: impl FnOnce(&mut UndoStack) -> R) -> io::Result<R> {
        let mut seen = self.seen();
        let Some(file) = &self.file else {
            return Ok(f(&mut seen.stack));
        };
        let _lock = hold(&beside(file, ".lock"), Some(PATIENCE))?;
        seen.refresh(file)?;
        let mut stack = seen.stack.clone();
        let r = f(&mut stack);
        let stamp = save(file, &mut stack)?;
        *seen = Seen {
            stamp: Some(stamp),
            stack,
        };
        Ok(r)
    }

    /// `edit`, dropped with a warning when it fails.
    fn edit_or_drop(&self, what: &str, f: impl FnOnce(&mut UndoStack)) {
        if let Err(e) = self.edit(f) {
            self.warn(what, &e);
        }
    }

    pub fn push(&self, u: Undoable) {
        self.edit_or_drop("dropped a push", |s| s.push(u))
    }

    /// [`UndoStack::push_all`].
    pub fn push_all(&self, batch: Vec<Undoable>) {
        if !batch.is_empty() {
            self.edit_or_drop("dropped a push", |s| s.push_all(batch))
        }
    }

    /// [`UndoStack::clear`].
    pub fn clear(&self) {
        if let Err(e) = self.edit(UndoStack::clear) {
            self.warn("retrying a clear", &e);
            let run = self.file.as_deref().and_then(|f| hold_for_clear(f).ok());
            self.retry(UndoStack::clear, run);
        }
    }

    /// [`UndoStack::retain_non_trash`].
    pub fn retain_non_trash(&self) {
        self.edit_or_drop("kept trash entries", UndoStack::retain_non_trash)
    }

    /// [`UndoStack::forget_gone`].
    pub fn forget_gone(&self, files: &[PathBuf]) {
        if !files.is_empty() {
            self.edit_or_drop("kept gone trash entries", |s| s.forget_gone(files))
        }
    }

    /// The entry Undo would reverse next and its label, read from the history's head alone, which
    /// a host's loop can afford: the whole history may run to [`MAX_SIZE`]. An Undo row offering
    /// the label takes that entry ([`SharedUndo::take_entry`]).
    pub fn head(&self) -> Option<(EntryId, &'static str)> {
        let Some(file) = &self.file else {
            let seen = self.seen();
            let (id, u) = seen.stack.entries.last()?;
            return Some((*id, u.label()?));
        };
        let read = || -> io::Result<Vec<u8>> {
            let mut head = Vec::new();
            File::open(file)?.take(HEAD).read_to_end(&mut head)?;
            Ok(head)
        };
        match read() {
            Ok(head) => head_of(&head),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => {
                self.warn("read", &e);
                None
            }
        }
    }

    /// [`SharedUndo::head`]'s label.
    pub fn label(&self) -> Option<&'static str> {
        self.head().map(|(_, label)| label)
    }

    /// What, in any process, makes an Undo now do nothing; None when nothing does.
    pub fn held(&self) -> Option<Hold> {
        let file = self.file.as_ref()?;
        let busy = |suffix| {
            let held = hold(&beside(file, suffix), None);
            matches!(held, Err(e) if e.kind() == io::ErrorKind::WouldBlock)
        };
        busy(".run").then(|| match busy(".clearing") {
            true => Hold::Clear,
            false => Hold::Undo,
        })
    }

    /// `f` on the entry Undo would reverse next, as the file holds it now.
    pub fn top<R>(&self, f: impl FnOnce(EntryId, &Undoable) -> R) -> Option<R> {
        self.read(|s| s.entries.last().map(|(id, u)| f(*id, u)))
    }

    /// The most recent entry, off the history ([`UndoStack::take`]), with the hold that keeps
    /// every other Undo off until it is put back. None while another undo runs.
    pub fn take(&self) -> Option<(Undo, Running)> {
        self.take_with(UndoStack::take)
    }

    /// [`SharedUndo::take`] when the top entry is still `id` ([`UndoStack::take_entry`]).
    pub fn take_entry(&self, id: EntryId) -> Option<(Undo, Running)> {
        self.take_with(|s| s.take_entry(id))
    }

    fn take_with(
        &self,
        take: impl FnOnce(&mut UndoStack) -> Option<Undo>,
    ) -> Option<(Undo, Running)> {
        let lock = match &self.file {
            None => None,
            // Long enough to outlast another process's `held` probe.
            Some(file) => match hold(&beside(file, ".run"), Some(Duration::from_millis(20))) {
                Ok(f) => Some(f),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return None,
                Err(e) => {
                    self.warn("Undo did nothing", &e);
                    return None;
                }
            },
        };
        match self.edit(take) {
            Ok(undo) => Some((
                undo?,
                Running {
                    _clearing: None,
                    _lock: lock,
                },
            )),
            Err(e) => {
                self.warn("Undo did nothing", &e);
                None
            }
        }
    }

    /// [`UndoStack::put_back`], then `running` lets the next Undo in. Returns None while a
    /// put-back the lock kept out is retried.
    pub fn put_back(
        &self,
        undo: Undo,
        running: Running,
        done: usize,
        error: Option<&io::Error>,
    ) -> Option<EntryId> {
        let left = undo.leftover(done, error)?;
        match self.edit(|s| s.restore(left.clone())) {
            Ok(id) => id,
            Err(e) => {
                self.warn("retrying a put-back", &e);
                self.retry(
                    move |s| {
                        s.restore(left.clone());
                    },
                    Some(running),
                );
                None
            }
        }
    }

    /// Whether a put-back or a clear the lock kept out is still being retried: a process quitting
    /// meanwhile loses it.
    pub fn retrying(&self) -> bool {
        (self
            .retries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter())
        .any(|h| !h.is_finished())
    }

    /// Waits for every retried put-back and clear to land or give up.
    pub fn settle(&self) {
        let retries =
            std::mem::take(&mut *self.retries.lock().unwrap_or_else(PoisonError::into_inner));
        for h in retries {
            let _ = h.join();
        }
    }

    /// `op` tried again on a thread of its own until an edit takes it or [`RETRY_FOR`] runs out,
    /// holding `.run` throughout: `running` for a put-back, or for a clear `.run` taken first
    /// ([`hold_for_clear`]) once the undo holding it ends, [`RETRY_FOR`] counting from then.
    fn retry(&self, op: impl Fn(&mut UndoStack) + Send + 'static, running: Option<Running>) {
        let Some(file) = self.file.clone() else {
            return;
        };
        let retry_for = self.retry_for;
        let spawned = std::thread::Builder::new()
            .name("undo history".into())
            .spawn(move || {
                let store = SharedUndo::at(file.clone());
                let mut since = Instant::now();
                let mut running = running;
                let given_up = |since: Instant, e: &io::Error| {
                    let late = since.elapsed() >= retry_for;
                    if late {
                        store.warn("gave up", e);
                    }
                    late
                };
                loop {
                    std::thread::sleep(PATIENCE);
                    if running.is_none() {
                        match hold_for_clear(&file) {
                            Ok(r) => (running, since) = (Some(r), Instant::now()),
                            // An undo holds `.run` till it ends, however long it runs.
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                            Err(e) if given_up(since, &e) => break,
                            Err(_) => continue,
                        }
                    }
                    match store.edit(&op) {
                        Ok(()) => break,
                        Err(e) if given_up(since, &e) => break,
                        Err(_) => {}
                    }
                }
                drop(running);
            });
        match spawned {
            Ok(h) => (self.retries.lock().unwrap_or_else(PoisonError::into_inner)).push(h),
            Err(e) => self.warn("no thread to retry on", &e),
        }
    }
}

impl Seen {
    /// Reparse `file` when it changed since it was last read. A missing or unreadable file is a
    /// history that starts over; one that cannot be opened or read is an error, the copy here
    /// left as it was.
    fn refresh(&mut self, file: &Path) -> io::Result<()> {
        let f = match File::open(file) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if self.stamp.take().is_some() {
                    self.stack = UndoStack::default();
                }
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        let now = stamp(&f.metadata()?);
        if self.stamp == Some(now) {
            return Ok(());
        }
        let mut data = Vec::new();
        f.take(MAX_SIZE + 1).read_to_end(&mut data)?;
        self.stack = decode(&data).unwrap_or_else(|| {
            log::warn!("reprisefs: undo history {} is unreadable", file.display());
            UndoStack::default()
        });
        self.stamp = Some(now);
        Ok(())
    }
}

/// The top entry's id and label from the start of a history file.
fn head_of(head: &[u8]) -> Option<(EntryId, &'static str)> {
    let mut fields = head.split(|&b| b == 0);
    if fields.next()? != MAGIC {
        return None;
    }
    let label = fields.next()?;
    let label = LABELS.into_iter().find(|l| l.as_bytes() == label)?;
    let id = std::str::from_utf8(fields.next()?).ok()?.parse().ok()?;
    Some((id, label))
}

fn stamp(m: &std::fs::Metadata) -> Stamp {
    (m.dev(), m.ino(), m.ctime(), m.ctime_nsec(), m.size())
}

fn beside(file: &Path, suffix: &str) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

/// `path` (made 0600 when missing) under an exclusive `flock`, waiting up to `patience` for
/// another holder; `WouldBlock` once it runs out.
fn hold(path: &Path, patience: Option<Duration>) -> io::Result<File> {
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    let deadline = Instant::now() + patience.unwrap_or_default();
    loop {
        match f.try_lock() {
            Ok(()) => return Ok(f),
            Err(std::fs::TryLockError::Error(e)) => return Err(e),
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1))
            }
            Err(std::fs::TryLockError::WouldBlock) => return Err(io::ErrorKind::WouldBlock.into()),
        }
    }
}

/// `.run` for a retried clear, with `.clearing` beside it to tell [`SharedUndo::held`] so.
fn hold_for_clear(file: &Path) -> io::Result<Running> {
    let run = hold(&beside(file, ".run"), None)?;
    // Long enough to outlast a `held` probe, the only other holder while `.run` is ours.
    let clearing = hold(&beside(file, ".clearing"), Some(Duration::from_millis(20)));
    Ok(Running {
        _clearing: clearing.ok(),
        _lock: Some(run),
    })
}

fn save(file: &Path, stack: &mut UndoStack) -> io::Result<Stamp> {
    let new = beside(file, ".new");
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&new)?;
    f.write_all(&encode(stack, MAX_SIZE))?;
    std::fs::rename(&new, file)?;
    Ok(stamp(&f.metadata()?))
}

/// The history as NUL-terminated fields: [`MAGIC`], the top entry's label and id or `-` twice, its
/// nonce, the clear count, the push count, the entry count, then each entry's number and item. A
/// trashed item's record is not stored: it is always the one beside its `files/` entry.
///
/// Only the newest entries that fit in `max` bytes are written, and `stack` keeps just those; an
/// entry too big to fit alone is left out with a warning.
fn encode(stack: &mut UndoStack, max: u64) -> Vec<u8> {
    // The fields ahead of the entries take at most two `HEAD`s.
    let cap = max.saturating_sub(2 * HEAD);
    let mut room = cap;
    let mut keep = vec![false; stack.entries.len()];
    let mut kept = Vec::new();
    for (i, entry) in stack.entries.iter().enumerate().rev() {
        let bytes = entry_bytes(entry);
        let size = bytes.len() as u64;
        if size > cap {
            log::warn!("reprisefs: undo history left out an entry of {size} bytes");
            continue;
        }
        if size > room {
            break;
        }
        room -= size;
        keep[i] = true;
        kept.push(bytes);
    }
    let mut keep = keep.into_iter();
    stack.entries.retain(|_| keep.next() == Some(true));
    let mut out = Out(Vec::new());
    out.field(MAGIC);
    match stack.entries.last() {
        Some((id, u)) => {
            out.field(u.label().unwrap_or("-").as_bytes());
            out.num(id);
        }
        None => (0..2).for_each(|_| out.field(b"-")),
    }
    out.num(stack.nonce);
    out.num(stack.epoch);
    out.num(stack.pushed);
    out.num(stack.entries.len());
    kept.into_iter().rev().for_each(|bytes| out.0.extend(bytes));
    out.0
}

fn entry_bytes((id, u): &(EntryId, Undoable)) -> Vec<u8> {
    let mut out = Out(Vec::new());
    out.num(id.n);
    out.item(u);
    out.0
}

fn decode(data: &[u8]) -> Option<UndoStack> {
    if data.len() as u64 > MAX_SIZE {
        return None;
    }
    let mut fields = data.strip_suffix(b"\0")?.split(|&b| b == 0);
    let mut i = In(&mut fields);
    if i.field()? != MAGIC {
        return None;
    }
    i.field()?;
    i.field()?;
    let (nonce, epoch, pushed) = (i.num()?, i.num()?, i.num()?);
    let entries = (0..i.num::<usize>()?)
        .map(|_| {
            let history = nonce;
            Some((
                EntryId {
                    history,
                    n: i.num()?,
                },
                i.item(0)?,
            ))
        })
        .collect::<Option<_>>()?;
    fields.next().is_none().then_some(UndoStack {
        entries,
        nonce,
        epoch,
        pushed,
    })
}

struct Out(Vec<u8>);

impl Out {
    fn field(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
        self.0.push(0);
    }

    fn num(&mut self, n: impl std::fmt::Display) {
        self.field(n.to_string().as_bytes())
    }

    fn path(&mut self, p: &Path) {
        self.field(p.as_os_str().as_bytes())
    }

    fn opt<T: std::fmt::Display>(&mut self, n: Option<T>) {
        match n {
            Some(n) => self.num(n),
            None => self.field(b"-"),
        }
    }

    fn id(&mut self, id: Option<FileId>) {
        self.opt(id.map(|(dev, ino)| format!("{dev}:{ino}")))
    }

    fn landed(&mut self, l: Option<Landed>) {
        self.opt(l.map(|l| {
            let ((dev, ino), (s, ns)) = (l.id, l.newest);
            format!("{dev}:{ino}:{s}:{ns}")
        }))
    }

    fn item(&mut self, u: &Undoable) {
        match u {
            Undoable::Move {
                from,
                to,
                listed,
                id,
            } => {
                self.field(b"move");
                self.path(from);
                self.path(to);
                self.num(u8::from(*listed));
                self.id(*id);
            }
            Undoable::Rename { from, to, id } => {
                self.field(b"rename");
                self.path(from);
                self.path(to);
                self.id(*id);
            }
            Undoable::Trash(t) => {
                self.field(b"trash");
                self.path(&t.original);
                self.field(t.name.as_bytes());
                self.path(&t.files);
                self.opt(t.deleted);
            }
            Undoable::Copied(p, landed) => {
                self.field(b"copied");
                self.path(p);
                self.landed(*landed);
            }
            Undoable::New(p, id) => {
                self.field(b"new");
                self.path(p);
                self.id(*id);
            }
            Undoable::Batch(items) => {
                self.field(b"batch");
                self.num(items.len());
                items.iter().for_each(|i| self.item(i));
            }
        }
    }
}

struct In<'a, 'b>(&'a mut dyn Iterator<Item = &'b [u8]>);

impl<'b> In<'_, 'b> {
    fn field(&mut self) -> Option<&'b [u8]> {
        self.0.next()
    }

    fn text(&mut self) -> Option<&'b str> {
        std::str::from_utf8(self.field()?).ok()
    }

    fn num<T: std::str::FromStr>(&mut self) -> Option<T> {
        self.text()?.parse().ok()
    }

    fn path(&mut self) -> Option<PathBuf> {
        let p = PathBuf::from(OsString::from_vec(self.field()?.to_vec()));
        p.is_absolute().then_some(p)
    }

    /// `Some(None)` for `-`.
    fn opt<T: std::str::FromStr>(&mut self) -> Option<Option<T>> {
        match self.text()? {
            "-" => Some(None),
            n => n.parse().ok().map(Some),
        }
    }

    fn id(&mut self) -> Option<Option<FileId>> {
        match self.opt::<String>()? {
            None => Some(None),
            Some(s) => {
                let (dev, ino) = s.split_once(':')?;
                Some(Some((dev.parse().ok()?, ino.parse().ok()?)))
            }
        }
    }

    fn landed(&mut self) -> Option<Option<Landed>> {
        match self.opt::<String>()? {
            None => Some(None),
            Some(s) => {
                let [dev, ino, sec, ns] = s.split(':').collect::<Vec<_>>()[..] else {
                    return None;
                };
                Some(Some(Landed {
                    id: (dev.parse().ok()?, ino.parse().ok()?),
                    newest: (sec.parse().ok()?, ns.parse().ok()?),
                }))
            }
        }
    }

    /// One entry, `depth` batches down.
    fn item(&mut self, depth: usize) -> Option<Undoable> {
        Some(match self.field()? {
            b"move" => Undoable::Move {
                from: self.path()?,
                to: self.path()?,
                listed: self.num::<u8>()? == 1,
                id: self.id()?,
            },
            b"rename" => Undoable::Rename {
                from: self.path()?,
                to: self.path()?,
                id: self.id()?,
            },
            b"trash" => {
                let (original, name, files) = (self.path()?, self.text()?, self.path()?);
                if files.parent()?.file_name()? != "files" {
                    return None;
                }
                Undoable::Trash(Trashed {
                    original,
                    name: name.to_string(),
                    info: crate::trash::info_path(&files).ok()?,
                    files,
                    deleted: self.opt()?,
                })
            }
            b"copied" => Undoable::Copied(self.path()?, self.landed()?),
            b"new" => Undoable::New(self.path()?, self.id()?),
            b"batch" if depth < MAX_NESTING => Undoable::Batch(
                (0..self.num::<usize>()?)
                    .map(|_| self.item(depth + 1))
                    .collect::<Option<_>>()?,
            ),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::CAP;
    use super::*;
    use std::ffi::OsStr;

    fn two_windows() -> (tempfile::TempDir, SharedUndo, SharedUndo) {
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join("reprise-undo");
        (t, SharedUndo::at(file.clone()), SharedUndo::at(file))
    }

    fn path_of(undo: &Undo) -> PathBuf {
        match undo.steps() {
            [Undoable::Copied(p, _) | Undoable::New(p, _) | Undoable::Rename { to: p, .. }] => {
                p.clone()
            }
            s => panic!("one step: {s:?}"),
        }
    }

    #[test]
    fn two_windows_undo_one_history_newest_first() {
        let (_t, a, b) = two_windows();
        a.push(Undoable::Copied("/a/copy".into(), None));
        b.push(Undoable::Rename {
            from: "/b/old".into(),
            to: "/b/new".into(),
            id: None,
        });
        a.push(Undoable::New("/a/New Folder".into(), None));
        assert_eq!(b.label(), Some("Undo New"), "b sees a's latest");

        let order: Vec<PathBuf> =
            std::iter::from_fn(|| a.take().map(|(u, _)| path_of(&u))).collect();
        assert_eq!(
            order,
            ["/a/New Folder", "/b/new", "/a/copy"].map(PathBuf::from)
        );
        assert_eq!(b.label(), None);
    }

    #[test]
    fn a_second_undo_does_nothing_while_one_runs() {
        let (_t, a, b) = two_windows();
        a.push(Undoable::Copied("/x".into(), None));
        b.push(Undoable::Copied("/y".into(), None));
        let (undo, running) = a.take().expect("a takes the newest");
        assert!(b.take().is_none(), "b waits for a's undo");
        assert!(a.take().is_none(), "so does a second Undo in a");
        assert_eq!(b.label(), Some("Undo Copy"), "the entry below stays");
        a.put_back(undo, running, 1, None);
        let (undo, _) = b.take().expect("b's turn");
        assert_eq!(path_of(&undo), Path::new("/x"));
    }

    #[test]
    fn what_an_undo_leaves_goes_below_what_another_window_pushed_since() {
        let (_t, a, b) = two_windows();
        a.push(Undoable::Copied("/x".into(), None));
        let (undo, running) = a.take().unwrap();
        b.push(Undoable::New("/newer".into(), None));
        let left = a
            .put_back(undo, running, 0, None)
            .expect("a cancel keeps it");
        assert_eq!(path_of(&b.take().unwrap().0), Path::new("/newer"));
        let (undo, _) = b.take_entry(left).expect("ids hold across windows");
        assert_eq!(path_of(&undo), Path::new("/x"));
    }

    #[test]
    fn a_clear_in_one_window_drops_an_undo_another_has_under_way() {
        let (_t, a, b) = two_windows();
        a.push(Undoable::Copied("/x".into(), None));
        let (undo, running) = a.take().unwrap();
        b.clear();
        assert_eq!(a.put_back(undo, running, 0, None), None);
        assert_eq!(b.label(), None);
    }

    #[test]
    fn every_kind_of_entry_reads_back_as_written() {
        let (_t, a, b) = two_windows();
        let odd = PathBuf::from(OsStr::from_bytes(b"/d/caf\xe9\nnew"));
        let entries = vec![
            Undoable::Move {
                from: "/a/x".into(),
                to: odd.clone(),
                listed: true,
                id: Some((8, u64::MAX)),
            },
            Undoable::Rename {
                from: "/a/y".into(),
                to: "/a/z".into(),
                id: None,
            },
            Undoable::Trash(Trashed {
                original: "/a/t.txt".into(),
                name: "t.txt".into(),
                files: "/T/files/t.txt".into(),
                info: "/T/info/t.txt.trashinfo".into(),
                deleted: Some(-5),
            }),
            Undoable::Batch(vec![
                Undoable::Copied(
                    "/a/c".into(),
                    Some(Landed {
                        id: (1, 2),
                        newest: (-3, 4),
                    }),
                ),
                Undoable::New("/a/n".into(), None),
            ]),
        ];
        for u in entries.clone() {
            a.push(u);
        }
        a.clear();
        for u in entries {
            a.push(u);
        }
        b.top(|_, _| ());
        let (read, written) = (b.seen(), a.seen());
        let (read, written) = (&read.stack, &written.stack);
        assert_eq!(
            format!("{:?}", read.entries),
            format!("{:?}", written.entries)
        );
        assert_eq!((read.nonce, read.epoch, read.pushed), (written.nonce, 1, 8));
    }

    #[test]
    fn a_trash_entry_outside_a_bins_files_folder_is_unreadable() {
        let (t, a, _) = two_windows();
        let entry = |files: &str| {
            let fields = [
                "reprise-undo 5",
                "Undo Delete",
                "0:1",
                "0",
                "1",
                "0",
                "1",
                "1",
                "trash",
                "/a/x",
                "x",
                files,
                "-",
            ];
            fields.map(|f| format!("{f}\0")).concat()
        };
        let file = t.path().join("reprise-undo");
        std::fs::write(&file, entry("/T/files/x")).unwrap();
        assert!(readable(&a));
        std::fs::write(&file, entry("/home/u/.profile")).unwrap();
        assert!(!readable(&a));
    }

    #[test]
    fn a_history_nested_or_grown_past_what_reprise_writes_is_unreadable() {
        let (t, a, _) = two_windows();
        let nested = |depth: usize| {
            let mut fields = ["reprise-undo 5", "Undo New", "0:1", "0", "1", "0", "1", "1"]
                .map(String::from)
                .to_vec();
            fields.extend((0..depth).flat_map(|_| ["batch".into(), "1".into()]));
            fields.extend(["new", "/a/x", "-"].map(String::from));
            fields.iter().map(|f| format!("{f}\0")).collect::<String>()
        };
        let file = t.path().join("reprise-undo");
        std::fs::write(&file, nested(MAX_NESTING)).unwrap();
        assert!(readable(&a));
        std::fs::write(&file, nested(100_000)).unwrap();
        assert!(!readable(&a), "refused without recursing that deep");

        let mut big = nested(0).into_bytes();
        big.resize(MAX_SIZE as usize + 1, b'\0');
        std::fs::write(&file, big).unwrap();
        assert!(!readable(&a));
        assert_eq!(
            a.label(),
            Some("Undo New"),
            "the label reads the head alone"
        );
    }

    fn readable(u: &SharedUndo) -> bool {
        u.top(|_, _| ()).is_some()
    }

    #[test]
    fn the_head_names_the_top_entry_of_every_kind() {
        let (t, a, b) = two_windows();
        let trashed = Trashed {
            original: "/a/t.txt".into(),
            name: "t.txt".into(),
            files: "/T/files/t.txt".into(),
            info: "/T/info/t.txt.trashinfo".into(),
            deleted: None,
        };
        let kinds = [
            Undoable::Move {
                from: "/a/x".into(),
                to: "/b/x".into(),
                listed: false,
                id: None,
            },
            Undoable::Rename {
                from: "/a/y".into(),
                to: "/a/z".into(),
                id: None,
            },
            Undoable::Trash(trashed),
            Undoable::Copied("/a/c".into(), None),
            Undoable::New("/a/n".into(), None),
        ];
        assert_eq!(b.label(), None);
        for u in kinds {
            let label = u.label();
            a.push(u);
            assert_eq!(b.label(), label);
        }
        a.take().unwrap();
        assert_eq!(b.label(), Some("Undo Copy"), "a take rewrites the head");
        std::fs::write(t.path().join("reprise-undo"), b"reprise-undo 4\0Undo New\0").unwrap();
        assert_eq!(b.label(), None, "an older history's head is not read");
    }

    #[test]
    fn an_undo_from_before_the_history_started_over_reaches_nothing_in_it() {
        let (t, a, b) = two_windows();
        a.push(Undoable::Copied("/old".into(), None));
        let (undo, running) = a.take().unwrap();
        let asked = a
            .put_back(undo, running, 0, None)
            .expect("a cancel keeps it");
        for n in 0..=CAP {
            a.push(Undoable::Copied(format!("/c{n}").into(), None));
        }
        let (under_way, running) = a.take().unwrap();

        std::fs::write(t.path().join("reprise-undo"), b"garbage").unwrap();
        b.push(Undoable::Copied("/new".into(), None));
        assert_eq!(a.put_back(under_way, running, 0, None), None);
        assert!(
            b.take_entry(asked).is_none(),
            "the old id names no new entry"
        );
        assert_eq!(b.label(), Some("Undo Copy"));
    }

    fn locked(t: &tempfile::TempDir) -> File {
        hold(&t.path().join("reprise-undo.lock"), None).unwrap()
    }

    fn until(what: &str, done: impl Fn() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(10), "{what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_take_the_lock_keeps_out_does_nothing_and_leaves_the_entry() {
        let (t, a, b) = two_windows();
        a.push(Undoable::Copied("/x".into(), None));
        let lock = locked(&t);
        assert!(a.take().is_none());
        b.push(Undoable::Copied("/dropped".into(), None));
        drop(lock);
        assert_eq!(path_of(&a.take().unwrap().0), Path::new("/x"));
        assert!(b.take().is_none(), "nothing left: the push was dropped");
    }

    #[test]
    fn a_clear_the_lock_keeps_out_lands_once_it_is_free_and_no_undo_reaches_past_it_meanwhile() {
        let (t, a, b) = two_windows();
        a.push(Undoable::Copied("/x".into(), None));
        let lock = locked(&t);
        b.clear();
        drop(lock);
        assert!(a.take().is_none(), "the retry holds every Undo off");
        until("the clear lands", || a.label().is_none());
        assert!(a.take().is_none());
    }

    #[test]
    fn a_put_back_the_lock_keeps_out_lands_before_the_next_undo_starts() {
        let (t, a, b) = two_windows();
        a.push(Undoable::Copied("/x".into(), None));
        let (undo, running) = a.take().unwrap();
        let lock = locked(&t);
        assert_eq!(a.put_back(undo, running, 0, None), None);
        assert!(a.retrying());
        drop(lock);
        assert!(b.take().is_none(), "the retry holds the undo's hold");
        a.clone().settle();
        assert!(!a.retrying());
        assert_eq!(path_of(&b.take().unwrap().0), Path::new("/x"));
    }

    #[test]
    fn an_undo_that_cannot_open_its_hold_does_nothing() {
        let (t, a, _) = two_windows();
        a.push(Undoable::Copied("/x".into(), None));
        std::fs::create_dir(t.path().join("reprise-undo.run")).unwrap();
        assert!(a.take().is_none());
        assert_eq!(a.label(), Some("Undo Copy"));
    }

    #[test]
    fn an_unreadable_history_starts_over() {
        let (t, a, _) = two_windows();
        std::fs::write(t.path().join("reprise-undo"), b"reprise-undo 5\0-\0-\0x\0").unwrap();
        assert_eq!(a.label(), None);
        a.push(Undoable::Copied("/x".into(), None));
        assert_eq!(a.label(), Some("Undo Copy"));
    }

    #[test]
    fn the_head_names_the_top_entrys_id_beside_its_label() {
        let (_t, a, b) = two_windows();
        assert_eq!(b.head(), None);
        a.push(Undoable::Copied("/x".into(), None));
        a.push(Undoable::New("/y".into(), None));
        let top = a.top(|id, _| id);
        assert_eq!(b.head(), top.map(|id| (id, "Undo New")));
        let (undo, _) = b
            .take_entry(top.unwrap())
            .expect("the head's id takes its entry");
        assert_eq!(path_of(&undo), Path::new("/y"));
        assert_eq!(b.head(), a.top(|id, _| (id, "Undo Copy")));
    }

    fn copied(p: &str) -> Undoable {
        Undoable::Copied(p.into(), None)
    }

    fn paths(s: &UndoStack) -> Vec<PathBuf> {
        (s.entries.iter())
            .map(|(_, u)| match u {
                Undoable::Copied(p, _) => p.clone(),
                u => panic!("a copy: {u:?}"),
            })
            .collect()
    }

    #[test]
    fn a_history_past_its_size_keeps_its_newest_entries_and_leaves_out_one_too_big_alone() {
        let mut stack = UndoStack::default();
        stack.push(copied("/old"));
        stack.push(copied("/mid"));
        stack.push(Undoable::Batch(
            (0..100).map(|n| copied(&format!("/big/{n}"))).collect(),
        ));
        stack.push(copied("/new"));
        let one = entry_bytes(&stack.entries[0]).len() as u64;
        let max = 2 * HEAD + 2 * one + one / 2;

        let data = encode(&mut stack, max);
        assert!(data.len() as u64 <= max, "{} bytes", data.len());
        let read = decode(&data).expect("readable");
        assert_eq!(paths(&read), ["/mid", "/new"].map(PathBuf::from));
        assert_eq!(
            paths(&stack),
            paths(&read),
            "the history keeps what it wrote"
        );
        assert_eq!(head_of(&data), Some((stack.entries[1].0, "Undo Copy")));
    }

    #[test]
    fn held_while_an_undo_runs_in_any_window() {
        let (_t, a, b) = two_windows();
        a.push(copied("/x"));
        assert_eq!(b.held(), None);
        let (undo, running) = a.take().unwrap();
        assert_eq!((a.held(), b.held()), (Some(Hold::Undo), Some(Hold::Undo)));
        a.put_back(undo, running, 1, None);
        assert_eq!(b.held(), None);
    }

    #[test]
    fn a_take_waits_out_a_brief_hold_on_the_run_file() {
        let (t, a, _) = two_windows();
        a.push(copied("/x"));
        let run = hold(&t.path().join("reprise-undo.run"), None).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(5));
            drop(run);
        });
        assert!(
            a.take().is_some(),
            "a held() probe does not turn an Undo away"
        );
        release.join().unwrap();
    }

    #[test]
    fn a_clear_the_lock_keeps_out_waits_for_a_running_undo_then_holds_undo_off_till_it_lands() {
        let (t, a, b) = two_windows();
        a.push(copied("/x"));
        a.push(copied("/y"));
        let (_undo, running) = a.take().unwrap();
        let lock = locked(&t);
        b.clear();
        drop(running);
        until("the clear's retry takes the run file", || {
            a.held() == Some(Hold::Clear)
        });
        drop(lock);
        assert!(a.take().is_none(), "no Undo reaches past the clear");
        until("the clear lands", || a.label().is_none());
        b.clone().settle();
        assert_eq!(a.held(), None);
    }

    #[test]
    fn a_clear_the_lock_keeps_out_holds_undo_off_as_a_clear() {
        let (t, a, b) = two_windows();
        a.push(copied("/x"));
        let lock = locked(&t);
        b.clear();
        assert_eq!(a.held(), Some(Hold::Clear));
        drop(lock);
        b.clone().settle();
        assert_eq!((a.held(), a.label()), (None, None));
    }

    #[test]
    fn a_clear_the_lock_keeps_out_waits_for_an_undo_running_past_its_retry_time_and_lands() {
        let (t, a, mut b) = two_windows();
        b.retry_for = Duration::from_millis(300);
        a.push(copied("/x"));
        a.push(copied("/y"));
        let (_undo, running) = a.take().unwrap();
        let lock = locked(&t);
        b.clear();
        drop(lock);
        std::thread::sleep(3 * b.retry_for);
        assert!(b.retrying(), "the clear waits for the undo");
        drop(running);
        b.clone().settle();
        assert_eq!(a.label(), None, "the clear landed");
    }
}
