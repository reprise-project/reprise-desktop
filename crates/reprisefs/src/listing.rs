//! Reading a directory the way Explorer does: folders first (a symlink
//! to a folder is a shortcut and sorts among the files), then
//! case-insensitively by name, hidden entries (a leading dot, or a line
//! of the folder's `.hidden`) kept but flagged (the file dialog hides
//! them), and each entry's kind and date text formatted for
//! display. A host maps [`Item`] to the row type it draws and fills in
//! the icon.

use crate::apps::MimeDb;
use crate::{hidden, locale, places, props, recent};
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub path: PathBuf,
    /// Opens as a folder: a folder, or a symlink to one. Whether it
    /// lists as a folder is [`Item::is_folder`].
    pub is_dir: bool,
    pub is_link: bool,
    pub exec: bool,
    /// Runs when opened ([`crate::program::is_program`]), so it wears a program's icon.
    pub program: bool,
    pub hidden: bool,
    /// Bytes and the modification time; a symlink's are its own, not
    /// its target's.
    pub size: u64,
    pub modified: Option<i64>,
    pub mime: Option<String>,
    /// The Type column: [`SHORTCUT`] or [`WEB_SHORTCUT`] for a shortcut
    /// or a symlink, as its Properties sheet reads, else
    /// [`kind_text_in`]'s.
    pub kind: String,
    pub drive: Option<crate::places::DriveKind>,
    /// A file system is mounted here. It may never answer, so the listing gives its cached
    /// attributes and reads nothing inside it.
    pub mount_root: bool,
    /// An item listed in the Recycle Bin: where it was deleted from and
    /// when, as its `.trashinfo` records them. Boxed so every other
    /// listing's items stay small.
    pub trashed: Option<Box<crate::trash::Trashed>>,
    /// A launcher or a web link ([`crate::apps::shortcut_at`]): listed by
    /// its own name and icon. Boxed as `trashed` is.
    pub shortcut: Option<Box<crate::apps::Shortcut>>,
}

impl Item {
    pub fn label(&self) -> &str {
        self.shortcut.as_ref().map_or(&self.name, |s| s.name())
    }

    /// Lists as a folder: sorts among the folders, shows no size and
    /// stands in the Folders tree. A symlink to a folder is a shortcut,
    /// listed among the files with its own size, though it still opens
    /// as the folder ([`Item::is_dir`]).
    pub fn is_folder(&self) -> bool {
        self.is_dir && !self.is_link
    }

    /// A file whose MIME type is a picture: Thumbnails and Tiles decode it.
    pub fn is_image(&self) -> bool {
        !self.is_dir
            && self
                .mime
                .as_deref()
                .is_some_and(|m| m.starts_with("image/"))
    }
}

/// Everything in `dir`, folders ([`Item::is_folder`]) first then
/// case-insensitively by name.
/// `globs` is `apps::mime_globs()`; pass `MimeDb::empty()` to skip MIME.
/// A symlink whose target has not answered within [`LINK_WAIT`] (one into a dead mount) lists as a
/// broken link would, and the folder is due again through [`Relist`] once the target answers.
pub fn read_dir(dir: &Path, globs: &MimeDb) -> Vec<Item> {
    read_dir_with(dir, globs, target_of, LINK_WAIT)
}

/// How long a listing waits on its symlinks' targets after the last one answered.
pub const LINK_WAIT: Duration = Duration::from_secs(2);

/// Past this many stuck targets ([`stuck_key`]) a listing lists its links unfollowed, so blocked
/// threads stay bounded.
const MAX_STUCK_LINKS: usize = 8;

const NOT_FOLLOWING: usize = usize::MAX;

/// The links whose follow outlasted its wait and has not returned yet, by [`stuck_key`]. One
/// blocked thread stands behind each key; a link with its key here is not followed again until
/// that thread returns.
static STUCK: Mutex<Stuck> = Mutex::new(Stuck {
    keys: BTreeMap::new(),
    capped: BTreeSet::new(),
});

struct Stuck {
    /// Each key's folders that listed a link behind it unfollowed: they go to the [`Relist`]s
    /// when its follow returns.
    keys: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    /// The folders that listed their links unfollowed past [`MAX_STUCK_LINKS`]: they go along
    /// when any follow returns.
    capped: BTreeSet<PathBuf>,
}

fn stuck() -> MutexGuard<'static, Stuck> {
    STUCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn folder_of(link: &Path) -> PathBuf {
    link.parent().unwrap_or(Path::new("/")).to_path_buf()
}

/// A host's ear for links listed unfollowed: once a blocked follow returns, the folders that
/// listed a link behind it unfollowed are due a fresh listing, which [`Relist::take`] gives.
/// Dropping it stops the calls.
pub struct Relist(Arc<Listener>);

struct Listener {
    due: Mutex<BTreeSet<PathBuf>>,
    wake: Box<dyn Fn() + Send + Sync>,
}

static RELISTS: Mutex<Vec<std::sync::Weak<Listener>>> = Mutex::new(Vec::new());

impl Relist {
    /// `wake` runs on the thread whose follow returned, so it only wakes the host's loop.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Relist {
        let l = Arc::new(Listener {
            due: Mutex::default(),
            wake: Box::new(wake),
        });
        let mut all = RELISTS.lock().unwrap_or_else(|e| e.into_inner());
        all.retain(|w| w.strong_count() > 0);
        all.push(Arc::downgrade(&l));
        Relist(l)
    }

    /// The folders due a fresh listing since the last take.
    pub fn take(&self) -> BTreeSet<PathBuf> {
        std::mem::take(&mut self.0.due.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

fn relist(dirs: BTreeSet<PathBuf>) {
    if dirs.is_empty() {
        return;
    }
    let live: Vec<Arc<Listener>> = RELISTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter_map(std::sync::Weak::upgrade)
        .collect();
    for l in live {
        let mut due = l.due.lock().unwrap_or_else(|e| e.into_inner());
        due.extend(dirs.iter().cloned());
        drop(due);
        (l.wake)();
    }
}

/// [`read_dir`] with `follow` reading the symlinks' targets on a thread of their own.
fn read_dir_with(
    dir: &Path,
    globs: &MimeDb,
    follow: fn(&Path) -> Target,
    wait: Duration,
) -> Vec<Item> {
    entries_with(dir, globs, follow, wait)
        .into_iter()
        .map(|(i, _)| i)
        .collect()
}

/// [`read_dir_with`]'s items, each with whether its target answered: false for a symlink listed
/// unfollowed, whose type nobody knows yet.
fn entries_with(
    dir: &Path,
    globs: &MimeDb,
    follow: fn(&Path) -> Target,
    wait: Duration,
) -> Vec<(Item, bool)> {
    let entries: Vec<std::fs::DirEntry> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().collect())
        .unwrap_or_default();
    let listed = hidden::Listed::read(dir);
    // The entry's own type is free on Linux.
    let (links, plain): (Vec<_>, Vec<_>) = entries
        .iter()
        .map(|e| (e.path(), e.file_type().is_ok_and(|t| t.is_symlink())))
        .partition(|(_, is_link)| *is_link);
    let mut items: Vec<(Item, bool)> = plain
        .iter()
        .map(|(p, _)| (item_with(p, false, target_of(p), &listed, globs), true))
        .collect();
    let mut links: Vec<PathBuf> = links.into_iter().map(|(p, _)| p).collect();
    links.sort();
    let targets = follow_links(links.clone(), follow, wait);
    items.extend(links.iter().zip(targets).map(|(p, target)| {
        let followed = target.followed;
        (item_with(p, true, target, &listed, globs), followed)
    }));
    // Sort on the collected `Item`s: re-statting in the comparator can
    // change answers mid-sort and panic.
    // Names differing only in case break on the exact name, not on
    // read_dir's order, which is the filesystem's hash order.
    items.sort_by_cached_key(|(i, _)| (!i.is_folder(), i.name.to_lowercase(), i.name.clone()));
    items
}

/// `follow` of each of `links`, read in order off the caller's thread; one per link. A link whose
/// follow outlasts `wait` (counted from the previous answer, and only once the follow is under
/// way) gives `T::default()` and the rest go on on a fresh thread; a link whose target is already
/// stuck gives the default without a follow. A thread left blocked ends when its mount answers or
/// goes away; the folders that listed a link into it unfollowed meanwhile then go to every
/// [`Relist`].
pub fn follow_links<T: Default + Send + 'static>(
    links: Vec<PathBuf>,
    follow: fn(&Path) -> T,
    wait: Duration,
) -> Vec<T> {
    let mut out: Vec<T> = std::iter::repeat_with(T::default)
        .take(links.len())
        .collect();
    if links.is_empty() {
        return out;
    }
    let network = places::network_mounts(std::fs::read("/proc/self/mounts").unwrap_or_default());
    let keys: Arc<Vec<PathBuf>> = Arc::new(links.iter().map(|l| stuck_key(l, &network)).collect());
    let links = Arc::new(links);
    let mut from = 0;
    loop {
        {
            let mut s = stuck();
            if from >= links.len() {
                break;
            }
            if s.keys.len() >= MAX_STUCK_LINKS {
                s.capped.extend(links[from..].iter().map(|l| folder_of(l)));
                break;
            }
        }
        let (tx, rx) = std::sync::mpsc::channel::<(usize, Option<T>)>();
        // The link the thread is inside `follow` of, else `NOT_FOLLOWING`; changed under STUCK.
        let following = Arc::new(AtomicUsize::new(NOT_FOLLOWING));
        let (l, k, f) = (links.clone(), keys.clone(), following.clone());
        let spawned = std::thread::Builder::new()
            .name("listing-links".into())
            .spawn(move || {
                for i in from..l.len() {
                    let skip = {
                        let mut s = stuck();
                        match s.keys.get_mut(&k[i]) {
                            Some(folders) => {
                                folders.insert(folder_of(&l[i]));
                                true
                            }
                            None => {
                                f.store(i, Ordering::Relaxed);
                                false
                            }
                        }
                    };
                    let t = (!skip).then(|| follow(&l[i]));
                    // Held over the send, so a caller giving up on `i` sees either the answer or
                    // the follow still under way.
                    let mut s = stuck();
                    f.store(NOT_FOLLOWING, Ordering::Relaxed);
                    let due = match t.as_ref().and_then(|_| s.keys.remove(&k[i])) {
                        Some(mut folders) => {
                            folders.append(&mut s.capped);
                            folders
                        }
                        None => BTreeSet::new(),
                    };
                    let sent = tx.send((i, t));
                    drop(s);
                    relist(due);
                    if sent.is_err() {
                        break;
                    }
                }
            });
        if spawned.is_err() {
            break;
        }
        let mut next = from;
        while next < links.len() {
            match rx.recv_timeout(wait) {
                Ok((i, t)) => {
                    out[i] = t.unwrap_or_default();
                    next = i + 1;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    let mut s = stuck();
                    if let Ok((i, t)) = rx.try_recv() {
                        drop(s);
                        out[i] = t.unwrap_or_default();
                        next = i + 1;
                        continue;
                    }
                    // A thread that has not reached `next` yet is late, not blocked.
                    if following.load(Ordering::Relaxed) != next {
                        continue;
                    }
                    (s.keys.entry(keys[next].clone()).or_default()).insert(folder_of(&links[next]));
                    next += 1;
                    break;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    next = links.len();
                    break;
                }
            }
        }
        from = next;
    }
    out
}

/// What a stuck follow of `link` is remembered by: the network mount its target names (read
/// without following, `..` taken lexically), so one dead mount stops every link into it, else the
/// link itself.
fn stuck_key(link: &Path, network: &std::collections::HashSet<PathBuf>) -> PathBuf {
    let Ok(target) = std::fs::read_link(link) else {
        return link.to_path_buf();
    };
    let mut lexical = PathBuf::new();
    for c in link
        .parent()
        .unwrap_or(Path::new("/"))
        .join(target)
        .components()
    {
        match c {
            std::path::Component::ParentDir => {
                lexical.pop();
            }
            std::path::Component::CurDir => {}
            c => lexical.push(c),
        }
    }
    network
        .iter()
        .filter(|m| lexical.starts_with(m))
        .max_by_key(|m| m.components().count())
        .cloned()
        .unwrap_or_else(|| link.to_path_buf())
}

pub fn item_at(path: &Path, globs: &MimeDb) -> Item {
    let is_link = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
    let listed = path.parent().map(hidden::Listed::read).unwrap_or_default();
    item_with(path, is_link, target_of(path), &listed, globs)
}

/// What an entry's listing reads through a symlink: everything that may wait on the target's
/// file system. The default is a link whose target never answered.
#[derive(Default)]
struct Target {
    mount: Option<MountStat>,
    meta: Option<std::fs::Metadata>,
    program: bool,
    shortcut: Option<crate::apps::Shortcut>,
    followed: bool,
}

/// One stat follows a link for its folder-ness and what opens it. A mount root is described from
/// [`mount_root_stat`] alone.
fn target_of(p: &Path) -> Target {
    let mount = mount_root_stat(p);
    let meta = match mount {
        Some(_) => None,
        None => std::fs::metadata(p).ok(),
    };
    let program = meta.as_ref().is_some_and(|m| {
        m.is_file() && m.permissions().mode() & 0o111 != 0 && crate::program::is_program(p, m)
    });
    let shortcut = meta.as_ref().and_then(|m| crate::apps::shortcut_at(p, m));
    Target {
        mount,
        meta,
        program,
        shortcut,
        followed: true,
    }
}

/// The size and date the columns show are a symlink's own, a shortcut's as its Properties sheet
/// reads them. `listed` is the folder's `.hidden`.
fn item_with(
    p: &Path,
    is_link: bool,
    target: Target,
    listed: &hidden::Listed,
    globs: &MimeDb,
) -> Item {
    let Target {
        mount,
        meta,
        program,
        shortcut,
        followed: _,
    } = target;
    let is_dir = match &mount {
        Some(m) => m.is_dir,
        None => meta.as_ref().is_some_and(|m| m.is_dir()),
    };
    let exec = meta
        .as_ref()
        .is_some_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let hidden = p.file_name().is_some_and(|n| listed.hides(n));
    let own = match is_link {
        true => std::fs::symlink_metadata(p).ok(),
        false => meta.clone(),
    };
    let (size, modified) = match (&mount, &own) {
        (Some(m), None) => (m.size, Some(m.modified)),
        _ => (
            own.as_ref().map(|m| m.len()).unwrap_or(0),
            own.as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64),
        ),
    };
    let mime = if globs.is_empty() || is_dir {
        None
    } else {
        p.file_name()
            .and_then(|n| globs.mime_of_name(&n.to_string_lossy()))
    };
    let shortcut = shortcut.map(Box::new);
    let kind = match shortcut.as_deref() {
        Some(crate::apps::Shortcut::Link(_)) => WEB_SHORTCUT.to_string(),
        Some(crate::apps::Shortcut::App(_)) => SHORTCUT.to_string(),
        None if is_link => SHORTCUT.to_string(),
        None => kind_text_in(globs, p, is_dir, mime.as_deref()),
    };
    Item {
        name,
        path: p.to_path_buf(),
        is_dir,
        is_link,
        exec,
        program,
        hidden,
        size,
        modified,
        mime,
        kind,
        drive: None,
        mount_root: mount.is_some(),
        trashed: None,
        shortcut,
    }
}

/// A mount root's attributes as last cached, which a mount that stopped answering still gives.
struct MountStat {
    is_dir: bool,
    size: u64,
    modified: i64,
}

/// Whether a file system is mounted on `p` (following a link), asked without waiting on it.
pub fn is_mount_root(p: &Path) -> bool {
    mount_root_stat(p).is_some()
}

/// [`MountStat`] when a file system is mounted on `p` (following a link), else None. Asks only the
/// mount table and cached attributes (`AT_STATX_DONT_SYNC`), so it never waits on the mount.
fn mount_root_stat(p: &Path) -> Option<MountStat> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
    let mut stx = std::mem::MaybeUninit::<libc::statx>::zeroed();
    // SAFETY: `c` is a NUL-terminated path and `stx` a writable statx buffer.
    let r = unsafe {
        libc::statx(
            libc::AT_FDCWD,
            c.as_ptr(),
            libc::AT_STATX_DONT_SYNC,
            libc::STATX_TYPE | libc::STATX_SIZE | libc::STATX_MTIME,
            stx.as_mut_ptr(),
        )
    };
    if r != 0 {
        return None;
    }
    // SAFETY: statx filled `stx` on success.
    let stx = unsafe { stx.assume_init() };
    (stx.stx_attributes & libc::STATX_ATTR_MOUNT_ROOT as u64 != 0).then(|| MountStat {
        is_dir: u32::from(stx.stx_mode) & libc::S_IFMT == libc::S_IFDIR,
        size: stx.stx_size,
        modified: stx.stx_mtime.tv_sec,
    })
}

/// A file dialog's listed item and its Date Modified text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub item: Item,
    pub modified: String,
}

/// `dir` as a file dialog lists it: [`read_dir`]'s order, hidden entries
/// dropped, and only the entries `keep` accepts, plus the links listed
/// unfollowed, which may yet open as folders. The My Recent Documents
/// place ([`places::RECENT`]) lists the recently used files instead.
pub fn dialog_rows(dir: &Path, globs: &MimeDb, keep: impl FnMut(&Item) -> bool) -> Vec<Row> {
    dialog_rows_with(dir, globs, keep, target_of, LINK_WAIT)
}

fn dialog_rows_with(
    dir: &Path,
    globs: &MimeDb,
    keep: impl FnMut(&Item) -> bool,
    follow: fn(&Path) -> Target,
    wait: Duration,
) -> Vec<Row> {
    if places::is_recent(dir) {
        let files = recent::RecentFiles::new().current(RECENT_LIMIT);
        return recent_rows(&files, globs, keep);
    }
    rows(entries_with(dir, globs, follow, wait), keep)
}

const RECENT_LIMIT: usize = 50;

/// `files` (newest first) as My Recent Documents lists them: each row's
/// `path` is the file's own, and a name already listed (the same name in
/// another folder) is left out, so a name picks one file.
fn recent_rows(files: &[PathBuf], globs: &MimeDb, keep: impl FnMut(&Item) -> bool) -> Vec<Row> {
    let mut seen = std::collections::HashSet::new();
    let items = files
        .iter()
        .map(|p| item_at(p, globs))
        .filter(|i| seen.insert(i.name.clone()))
        .map(|i| (i, true));
    rows(items, keep)
}

/// `entries` a dialog lists: those not hidden that `keep` accepts or whose target never answered.
fn rows(
    entries: impl IntoIterator<Item = (Item, bool)>,
    mut keep: impl FnMut(&Item) -> bool,
) -> Vec<Row> {
    let is_12h = locale::is_12h();
    entries
        .into_iter()
        .filter(|(i, followed)| !i.hidden && (!followed || keep(i)))
        .map(|(item, _)| Row {
            modified: item
                .modified
                .map(|s| date_text(s, is_12h))
                .unwrap_or_default(),
            item,
        })
        .collect()
}

/// The type a listing and the Properties sheet give a program launcher
/// or a symlink, whatever it points at.
pub const SHORTCUT: &str = "Shortcut";
pub const WEB_SHORTCUT: &str = "Internet Shortcut";

/// "File Folder", the MIME comment (`props::comment_of`) when
/// shared-mime-info has one, else "TXT File" / "File".
pub fn kind_text(path: &Path, is_dir: bool, mime: Option<&str>) -> String {
    kind_text_in(&MimeDb::empty(), path, is_dir, mime)
}

pub fn kind_text_in(db: &MimeDb, path: &Path, is_dir: bool, mime: Option<&str>) -> String {
    if is_dir {
        return "File Folder".to_string();
    }
    mime.and_then(|m| db.description(m))
        .unwrap_or_else(|| props::generic_type_name(path))
}

/// "M/D/YYYY H:MM AM/PM" (or 24-hour), the Date Modified text.
pub fn date_text(secs: i64, is_12h: bool) -> String {
    let tm = locale::localtime(secs);
    let date = format!("{}/{}/{}", tm.tm_mon + 1, tm.tm_mday, tm.tm_year + 1900);
    if is_12h {
        let (h12, ampm) = match tm.tm_hour {
            0 => (12, "AM"),
            1..=11 => (tm.tm_hour, "AM"),
            12 => (12, "PM"),
            h => (h - 12, "PM"),
        };
        format!("{date} {h12}:{:02} {ampm}", tm.tm_min)
    } else {
        format!("{date} {}:{:02}", tm.tm_hour, tm.tm_min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folders_first_then_case_insensitive_with_hidden_flagged() {
        let t = tempfile::tempdir().unwrap();
        for n in ["b.txt", "A.txt", ".dot", "c.txt"] {
            std::fs::write(t.path().join(n), "").unwrap();
        }
        std::fs::write(t.path().join(".hidden"), "c.txt\n").unwrap();
        std::fs::create_dir(t.path().join("zdir")).unwrap();
        let items = read_dir(t.path(), &MimeDb::empty());
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            names,
            ["zdir", ".dot", ".hidden", "A.txt", "b.txt", "c.txt"]
        );
        let hidden: Vec<bool> = items.iter().map(|i| i.hidden).collect();
        assert_eq!(hidden, [false, true, true, false, false, true]);
        assert!(item_at(&t.path().join("c.txt"), &MimeDb::empty()).hidden);
        assert_eq!(items[0].kind, "File Folder");
        assert_eq!(items[4].kind, "TXT File");
    }
    #[test]
    fn a_trusted_launcher_is_listed_by_its_name() {
        let t = tempfile::tempdir().unwrap();
        let app = "[Desktop Entry]\nType=Application\nName=Terminal\nExec=foot\nIcon=utilities-terminal\n";
        let link = "[Desktop Entry]\nType=Link\nName=Example\nURL=https://example.org/\n";
        crate::apps::write_launcher(&t.path().join("org.foot.desktop"), app).unwrap();
        crate::apps::write_launcher(&t.path().join("web.desktop"), link).unwrap();
        std::fs::write(t.path().join("plain.desktop"), app).unwrap();
        let items = read_dir(t.path(), &MimeDb::empty());
        let got: Vec<(&str, &str, Option<&str>)> = items
            .iter()
            .map(|i| {
                let icon = i.shortcut.as_ref().map(|s| s.icon());
                (i.name.as_str(), i.label(), icon)
            })
            .collect();
        assert_eq!(
            got,
            [
                ("org.foot.desktop", "Terminal", Some("utilities-terminal")),
                ("plain.desktop", "plain.desktop", None),
                ("web.desktop", "Example", Some("text-html")),
            ]
        );
    }
    #[test]
    fn a_shortcut_is_typed_as_its_sheet_types_it() {
        let t = tempfile::tempdir().unwrap();
        let app = "[Desktop Entry]\nType=Application\nName=Terminal\nExec=foot\n";
        let link = "[Desktop Entry]\nType=Link\nName=Example\nURL=https://example.org/\n";
        crate::apps::write_launcher(&t.path().join("app.desktop"), app).unwrap();
        crate::apps::write_launcher(&t.path().join("web.desktop"), link).unwrap();
        std::fs::write(t.path().join("plain.desktop"), app).unwrap();
        std::fs::create_dir(t.path().join("dir")).unwrap();
        std::fs::write(t.path().join("a.txt"), "").unwrap();
        std::os::unix::fs::symlink("dir", t.path().join("to-dir")).unwrap();
        std::os::unix::fs::symlink("a.txt", t.path().join("to-file")).unwrap();
        let kinds: Vec<(String, String)> = read_dir(t.path(), &MimeDb::empty())
            .into_iter()
            .map(|i| (i.name, i.kind))
            .collect();
        let want = [
            ("dir", "File Folder"),
            ("a.txt", "TXT File"),
            ("app.desktop", "Shortcut"),
            ("plain.desktop", "DESKTOP File"),
            ("to-dir", "Shortcut"),
            ("to-file", "Shortcut"),
            ("web.desktop", "Internet Shortcut"),
        ];
        let want: Vec<(String, String)> = want
            .iter()
            .map(|(n, k)| (n.to_string(), k.to_string()))
            .collect();
        assert_eq!(kinds, want);
    }
    #[test]
    fn a_symlink_lists_its_own_size_and_date() {
        let t = tempfile::tempdir().unwrap();
        let target = t.path().join("big.txt");
        std::fs::write(&target, vec![b'x'; 5000]).unwrap();
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        std::fs::File::options()
            .write(true)
            .open(&target)
            .unwrap()
            .set_modified(old)
            .unwrap();
        std::os::unix::fs::symlink("big.txt", t.path().join("link")).unwrap();
        let link = item_at(&t.path().join("link"), &MimeDb::empty());
        let own = std::fs::symlink_metadata(t.path().join("link")).unwrap();
        let sheet = props::glance(&t.path().join("link"), "link", &[]).unwrap();
        assert_eq!(link.size, "big.txt".len() as u64);
        assert_eq!(link.size, sheet.size);
        assert_ne!(link.modified, Some(1_000_000_000));
        assert_eq!(
            link.modified,
            own.modified()
                .ok()
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
        );
        assert_eq!(link.modified, Some(sheet.modified));
        assert_eq!(item_at(&target, &MimeDb::empty()).size, 5000);
    }
    #[test]
    fn a_symlink_to_a_folder_sorts_among_the_files() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir(t.path().join("mfolder")).unwrap();
        std::fs::write(t.path().join("a.txt"), "x").unwrap();
        std::fs::write(t.path().join("c.txt"), "x").unwrap();
        std::os::unix::fs::symlink("mfolder", t.path().join("b link")).unwrap();
        let items = read_dir(t.path(), &MimeDb::empty());
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["mfolder", "a.txt", "b link", "c.txt"]);
        let link = &items[2];
        assert!(
            link.is_dir && !link.is_folder(),
            "opens as a folder, lists as a file"
        );
        assert_eq!(link.size, "mfolder".len() as u64);
        assert!(items[0].is_folder());
    }
    #[test]
    fn a_symlink_whose_target_never_answers_lists_as_a_broken_link() {
        use std::os::unix::ffi::OsStrExt;
        // Opening a FIFO blocks until a writer comes, as a lookup on a stopped mount does.
        fn follow(p: &Path) -> Target {
            if std::fs::metadata(p).is_ok_and(|m| m.file_type().is_fifo()) {
                let _ = std::fs::File::open(p);
            }
            target_of(p)
        }
        use std::os::unix::fs::FileTypeExt;
        let t = tempfile::tempdir().unwrap();
        let fifo = t.path().join("pipe");
        let c = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: `c` is a NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        std::fs::create_dir(t.path().join("folder")).unwrap();
        std::fs::write(t.path().join("doc.txt"), "x").unwrap();
        std::os::unix::fs::symlink("folder", t.path().join("a live")).unwrap();
        std::os::unix::fs::symlink("pipe", t.path().join("b dead.txt")).unwrap();
        let started = std::time::Instant::now();
        let items = read_dir_with(
            t.path(),
            &MimeDb::empty(),
            follow,
            Duration::from_millis(200),
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        let by = |n: &str| items.iter().find(|i| i.name == n).unwrap();
        let live = by("a live");
        assert!(
            live.is_link && live.is_dir,
            "a link that answered opens as a folder"
        );
        let dead = by("b dead.txt");
        assert!(dead.is_link && !dead.is_dir && dead.shortcut.is_none());
        assert_eq!(dead.kind, SHORTCUT);
        assert_eq!(dead.size, "pipe".len() as u64, "the link's own size");
        assert!(by("folder").is_folder());
        assert_eq!(by("doc.txt").size, 1);
        // Lets the blocked follow return.
        drop(std::fs::File::options().write(true).open(&fifo));
    }
    /// A folder in a temp dir holding `pipe`, a FIFO whose open blocks until a writer comes, as a
    /// lookup on a stopped mount does, and `folder`.
    fn dead_and_live() -> tempfile::TempDir {
        use std::os::unix::ffi::OsStrExt;
        let t = tempfile::tempdir().unwrap();
        let c = std::ffi::CString::new(t.path().join("pipe").as_os_str().as_bytes()).unwrap();
        // SAFETY: `c` is a NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        std::fs::create_dir(t.path().join("folder")).unwrap();
        t
    }

    fn unblock(fifo: &Path) {
        use std::os::unix::fs::OpenOptionsExt;
        drop(
            std::fs::File::options()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(fifo),
        );
    }

    #[test]
    fn a_dead_link_sorted_before_a_live_one_leaves_the_live_one_followed() {
        use std::os::unix::fs::FileTypeExt;
        fn follow(p: &Path) -> Target {
            if std::fs::metadata(p).is_ok_and(|m| m.file_type().is_fifo()) {
                let _ = std::fs::File::open(p);
            }
            target_of(p)
        }
        let t = dead_and_live();
        std::os::unix::fs::symlink("pipe", t.path().join("a dead")).unwrap();
        std::os::unix::fs::symlink("folder", t.path().join("b live")).unwrap();
        std::os::unix::fs::symlink("folder", t.path().join("c live")).unwrap();
        let items = read_dir_with(
            t.path(),
            &MimeDb::empty(),
            follow,
            Duration::from_millis(200),
        );
        let by = |n: &str| items.iter().find(|i| i.name == n).unwrap();
        assert!(!by("a dead").is_dir);
        assert!(by("b live").is_dir, "b live opens as a folder");
        assert!(by("c live").is_dir, "c live opens as a folder");
        unblock(&t.path().join("pipe"));
    }

    #[test]
    fn relisting_a_dead_link_twenty_times_keeps_live_links_and_one_blocked_follow() {
        use std::os::unix::fs::FileTypeExt;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static BLOCKED: AtomicUsize = AtomicUsize::new(0);
        fn follow(p: &Path) -> Target {
            if std::fs::metadata(p).is_ok_and(|m| m.file_type().is_fifo()) {
                BLOCKED.fetch_add(1, Ordering::SeqCst);
                let _ = std::fs::File::open(p);
                BLOCKED.fetch_sub(1, Ordering::SeqCst);
            }
            target_of(p)
        }
        let t = dead_and_live();
        std::os::unix::fs::symlink("pipe", t.path().join("a dead")).unwrap();
        std::os::unix::fs::symlink("folder", t.path().join("b live")).unwrap();
        for n in 0..20 {
            let items = read_dir_with(
                t.path(),
                &MimeDb::empty(),
                follow,
                Duration::from_millis(100),
            );
            let live = items.iter().find(|i| i.name == "b live").unwrap();
            assert!(live.is_dir, "b live opens as a folder in listing {n}");
        }
        assert_eq!(
            BLOCKED.load(Ordering::SeqCst),
            1,
            "one follow stays blocked"
        );
        unblock(&t.path().join("pipe"));
    }

    #[test]
    fn a_folder_that_listed_a_dead_link_is_due_again_once_its_target_answers() {
        use std::os::unix::fs::FileTypeExt;
        fn follow(p: &Path) -> Target {
            if std::fs::metadata(p).is_ok_and(|m| m.file_type().is_fifo()) {
                let _ = std::fs::File::open(p);
            }
            target_of(p)
        }
        let t = dead_and_live();
        std::os::unix::fs::symlink("pipe", t.path().join("a dead")).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let relist = Relist::new(move || {
            let _ = tx.send(());
        });
        let wait = Duration::from_millis(100);
        for _ in 0..2 {
            let items = read_dir_with(t.path(), &MimeDb::empty(), follow, wait);
            let dead = items.iter().find(|i| i.name == "a dead").unwrap();
            assert!(!dead.is_dir, "the dead link lists unfollowed");
        }
        assert!(
            !relist.take().contains(t.path()),
            "nothing is due while the target hangs"
        );
        unblock(&t.path().join("pipe"));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut due = BTreeSet::new();
        while !due.contains(t.path()) && std::time::Instant::now() < deadline {
            let _ = rx.recv_timeout(Duration::from_millis(100));
            due.extend(relist.take());
        }
        assert!(
            due.contains(t.path()),
            "the folder is due once the target answers"
        );
        drop(relist);
    }

    #[test]
    fn a_dialog_keeps_a_link_listed_unfollowed_that_its_filter_refuses() {
        use std::os::unix::fs::FileTypeExt;
        fn follow(p: &Path) -> Target {
            if std::fs::metadata(p).is_ok_and(|m| m.file_type().is_fifo()) {
                let _ = std::fs::File::open(p);
            }
            target_of(p)
        }
        let t = dead_and_live();
        std::os::unix::fs::symlink("pipe", t.path().join("dead")).unwrap();
        std::os::unix::fs::symlink("gone", t.path().join("broken")).unwrap();
        std::fs::write(t.path().join("a.png"), "").unwrap();
        std::fs::write(t.path().join("b.txt"), "").unwrap();
        let rows = dialog_rows_with(
            t.path(),
            &MimeDb::empty(),
            |i| i.is_dir || i.name.ends_with(".txt"),
            follow,
            Duration::from_millis(100),
        );
        let names: Vec<&str> = rows.iter().map(|r| r.item.name.as_str()).collect();
        assert_eq!(names, ["folder", "b.txt", "dead"]);
        unblock(&t.path().join("pipe"));
    }

    /// Follows that wait for the test: every follow is counted, and one of a link named `held*`
    /// blocks until [`open`] lets that entry (1 for the first follow of the link, 2 for the
    /// second) through, and a [`Relist`] wake run on its thread afterwards blocks until
    /// [`open_wake`] does. Any other link answers at once.
    mod gate {
        use std::cell::RefCell;
        use std::collections::{HashMap, HashSet};
        use std::path::{Path, PathBuf};
        use std::sync::Mutex;
        use std::time::{Duration, Instant};

        type Entry = (PathBuf, usize);
        static ENTERED: Mutex<Option<HashMap<PathBuf, usize>>> = Mutex::new(None);
        static OPEN: Mutex<Option<HashSet<Entry>>> = Mutex::new(None);
        static WAKE_OPEN: Mutex<Option<HashSet<Entry>>> = Mutex::new(None);
        thread_local!(static LAST: RefCell<Option<Entry>> = const { RefCell::new(None) });

        fn has(set: &Mutex<Option<HashSet<Entry>>>, e: &Entry) -> bool {
            set.lock().unwrap().as_ref().is_some_and(|s| s.contains(e))
        }

        pub fn until(what: &str, mut done: impl FnMut() -> bool) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !done() {
                assert!(Instant::now() < deadline, "{what} within 10 s");
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        pub fn follow(p: &Path) -> bool {
            let n = {
                let mut e = ENTERED.lock().unwrap();
                let n = e
                    .get_or_insert_default()
                    .entry(p.to_path_buf())
                    .or_default();
                *n += 1;
                *n
            };
            if !(p.file_name()).is_some_and(|n| n.to_string_lossy().starts_with("held")) {
                return true;
            }
            let e = (p.to_path_buf(), n);
            LAST.with(|l| *l.borrow_mut() = Some(e.clone()));
            until("the follow is let through", || has(&OPEN, &e));
            true
        }

        pub fn wake() {
            if let Some(e) = LAST.with(|l| l.borrow().clone()) {
                until("the wake is let through", || has(&WAKE_OPEN, &e));
            }
        }

        pub fn entered(p: &Path, n: usize) {
            until("the follow starts", || {
                let e = ENTERED.lock().unwrap();
                e.as_ref().and_then(|e| e.get(p)).copied() == Some(n)
            });
        }

        pub fn open(p: &Path, n: usize) {
            (OPEN.lock().unwrap().get_or_insert_default()).insert((p.to_path_buf(), n));
        }

        pub fn open_wake(p: &Path, n: usize) {
            (WAKE_OPEN.lock().unwrap().get_or_insert_default()).insert((p.to_path_buf(), n));
        }
    }

    /// Lists `links` on a thread of its own, as a second listing would while a first one runs.
    fn list_aside(links: Vec<PathBuf>, wait: Duration) -> std::thread::JoinHandle<Vec<bool>> {
        std::thread::spawn(move || follow_links(links, gate::follow, wait))
    }

    #[test]
    fn a_wait_running_out_after_the_last_answer_ends_the_listing() {
        let t = tempfile::tempdir().unwrap();
        let held = t.path().join("held");
        let _relist = Relist::new(gate::wake);
        let first = list_aside(vec![held.clone()], Duration::from_secs(5));
        gate::entered(&held, 1);
        // A second listing marks the link stuck while the first one's follow is under way, so
        // the first one's answer hands its folder to the wake, which then holds the thread past
        // the wait.
        let second = follow_links(vec![held.clone()], gate::follow, Duration::from_millis(20));
        assert_eq!(second, [false], "the second listing gives up on the link");
        gate::open(&held, 1);
        let first = first.join();
        gate::open_wake(&held, 1);
        gate::open(&held, 2);
        assert_eq!(
            first.ok(),
            Some(vec![true]),
            "the first listing ends with its answer"
        );
    }

    #[test]
    fn a_link_thread_late_to_reach_a_link_leaves_it_unmarked() {
        let t = tempfile::tempdir().unwrap();
        let (held, late) = (t.path().join("held"), t.path().join("late"));
        let _relist = Relist::new(gate::wake);
        let wait = Duration::from_secs(1);
        let first = list_aside(vec![held.clone(), late.clone()], wait);
        gate::entered(&held, 1);
        let second = follow_links(vec![held.clone()], gate::follow, Duration::from_millis(1));
        assert_eq!(second, [false], "the second listing gives up on the link");
        // The first answer's wake holds the thread before `late` for longer than the wait.
        gate::open(&held, 1);
        std::thread::sleep(wait + wait / 2);
        gate::open_wake(&held, 1);
        // A wait that runs out while `late` is being followed may still list it unfollowed;
        // what counts is that the thread follows it and nothing stays marked.
        assert!(first.join().unwrap()[0], "held is followed");
        gate::entered(&late, 1);
        gate::until("late is not marked stuck", || {
            !stuck().keys.contains_key(&late)
        });
        gate::open(&held, 2);
    }

    #[test]
    fn a_stuck_follow_that_returns_makes_only_its_own_folders_due() {
        let (x, y) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (in_x, in_y) = (x.path().join("held"), y.path().join("held"));
        let relist = Relist::new(|| {});
        let wait = Duration::from_millis(20);
        assert_eq!(
            follow_links(vec![in_x.clone()], gate::follow, wait),
            [false]
        );
        assert_eq!(
            follow_links(vec![in_y.clone()], gate::follow, wait),
            [false]
        );
        gate::open(&in_x, 1);
        gate::open_wake(&in_x, 1);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut due = BTreeSet::new();
        while !due.contains(x.path()) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            due.extend(relist.take());
        }
        assert!(due.contains(x.path()), "x is due once its link answers");
        assert!(!due.contains(y.path()), "y's link still hangs");
        gate::open(&in_y, 1);
        gate::open_wake(&in_y, 1);
    }

    #[test]
    fn a_stuck_follow_is_remembered_by_the_network_mount_its_target_names() {
        let t = tempfile::tempdir().unwrap();
        let network = std::collections::HashSet::from([PathBuf::from("/mnt/nas")]);
        let link = t.path().join("a");
        std::os::unix::fs::symlink("/mnt/nas/share/../x", &link).unwrap();
        assert_eq!(stuck_key(&link, &network), PathBuf::from("/mnt/nas"));
        let local = t.path().join("b");
        std::os::unix::fs::symlink("/mnt/nas2/x", &local).unwrap();
        assert_eq!(stuck_key(&local, &network), local, "not under /mnt/nas");
    }

    #[test]
    fn a_program_is_an_executable_script_or_elf_file() {
        let t = tempfile::tempdir().unwrap();
        let put = |n: &str, bytes: &[u8], m: u32| {
            std::fs::write(t.path().join(n), bytes).unwrap();
            std::fs::set_permissions(t.path().join(n), std::fs::Permissions::from_mode(m)).unwrap();
        };
        put("elf", b"\x7fELF\x02", 0o755);
        put("photo.png", b"\x89PNG", 0o755);
        put("plain.sh", b"#!/bin/sh\n", 0o644);
        put("run.sh", b"#!/bin/sh\n", 0o755);
        let program: Vec<(String, bool)> = read_dir(t.path(), &MimeDb::empty())
            .into_iter()
            .map(|i| (i.name, i.program))
            .collect();
        assert_eq!(
            program,
            [
                ("elf".into(), true),
                ("photo.png".into(), false),
                ("plain.sh".into(), false),
                ("run.sh".into(), true),
            ]
        );
    }

    #[test]
    fn exec_is_a_regular_file_with_an_execute_bit() {
        let t = tempfile::tempdir().unwrap();
        let mode = |n: &str, m: u32| {
            std::fs::write(t.path().join(n), "").unwrap();
            std::fs::set_permissions(t.path().join(n), std::fs::Permissions::from_mode(m)).unwrap();
        };
        mode("plain", 0o644);
        mode("tool", 0o755);
        mode("owner", 0o700);
        std::fs::create_dir(t.path().join("dir")).unwrap();
        let exec: Vec<(String, bool)> = read_dir(t.path(), &MimeDb::empty())
            .into_iter()
            .map(|i| (i.name, i.exec))
            .collect();
        assert_eq!(
            exec,
            [
                ("dir".into(), false),
                ("owner".into(), true),
                ("plain".into(), false),
                ("tool".into(), true),
            ]
        );
    }
    #[test]
    fn dialog_rows_drop_hidden_entries_and_what_keep_refuses() {
        let t = tempfile::tempdir().unwrap();
        for n in ["b.txt", "a.png", ".dot.txt", "listed.txt"] {
            std::fs::write(t.path().join(n), "").unwrap();
        }
        std::fs::write(t.path().join(".hidden"), "listed.txt\n").unwrap();
        std::fs::create_dir(t.path().join("dir")).unwrap();
        let rows = dialog_rows(t.path(), &MimeDb::empty(), |i| {
            i.is_dir || i.name.ends_with(".txt")
        });
        let names: Vec<&str> = rows.iter().map(|r| r.item.name.as_str()).collect();
        assert_eq!(names, ["dir", "b.txt"]);
        assert_eq!(rows[1].item.path, t.path().join("b.txt"));
        assert!(!rows[1].modified.is_empty(), "a file's date is formatted");
    }
    #[test]
    fn recent_rows_keep_each_files_path_and_the_newest_of_a_name() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir(t.path().join("old")).unwrap();
        let (new, old, pic) = (
            t.path().join("notes.txt"),
            t.path().join("old/notes.txt"),
            t.path().join("pic.png"),
        );
        for p in [&new, &old, &pic] {
            std::fs::write(p, "").unwrap();
        }
        let rows = recent_rows(&[new.clone(), pic, old], &MimeDb::empty(), |i| {
            i.name.ends_with(".txt")
        });
        let got: Vec<(&str, &Path)> = rows
            .iter()
            .map(|r| (r.item.name.as_str(), r.item.path.as_path()))
            .collect();
        assert_eq!(got, [("notes.txt", new.as_path())]);
    }
    #[test]
    fn a_folder_with_a_file_system_mounted_on_it_is_a_mount_root() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("file"), "").unwrap();
        let db = MimeDb::empty();
        let proc = item_at(Path::new("/proc"), &db);
        assert!(proc.mount_root && proc.is_dir, "/proc is a mounted folder");
        assert!(is_mount_root(Path::new("/proc")));
        assert!(!item_at(t.path(), &db).mount_root, "a plain folder");
        assert!(!item_at(&t.path().join("file"), &db).mount_root, "a file");
    }

    #[test]
    fn date_text_is_explorers() {
        // 2026-09-13 20:25 local: build the epoch from the components.
        let secs = crate::locale::mktime(2026, 9, 13, 20, 25, 0);
        assert_eq!(date_text(secs, true), "9/13/2026 8:25 PM");
        assert_eq!(date_text(secs, false), "9/13/2026 20:25");
    }
}
