//! The freedesktop trash: `$XDG_DATA_HOME/Trash` and per-drive `$topdir/.Trash/$uid` or
//! `.Trash-$uid` ([`Bins`]). Per-drive rules and caps are [`crate::bincap`]'s.

use reprisebus::mutex::lock;
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::bincap::{self, Fate, Policy, Rule};
use crate::bindir::{self, BinDir};
use crate::locale;
use crate::ops::{build_dir, refuse_holding_mount, refuse_mount_point};
use crate::places;
use crate::progress::Meter;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trashed {
    pub original: PathBuf,
    pub files: PathBuf,
    pub info: PathBuf,
    pub deleted: Option<i64>,
}

/// Move `path` into the trash at `root` (the directory holding `files/`
/// and `info/`; `places::trash_root()` for the user's). It lands as
/// `files/STEM` beside `info/STEM.trashinfo`, STEM being the first of
/// `candidates(NAME)` free in both. A folder holding a mount point is
/// refused: in the trash, emptying it would empty the mounted drive. A
/// path on another drive is refused with `CrossesDevices` (see
/// [`recyclable_in`]): nothing is ever copied into a trash. A trash kept
/// at a drive's top (`.Trash-$uid`, `.Trash/$uid`) records the path
/// relative to that top, so the record holds wherever the drive mounts.
pub fn trash_in(root: &Path, path: &Path) -> io::Result<Trashed> {
    refuse_holding_mount(path)?;
    if !recyclable_in(root, path) {
        return Err(io::Error::new(
            io::ErrorKind::CrossesDevices,
            "it is on a drive without a Recycle Bin",
        ));
    }
    let files_dir = root.join("files");
    let info_dir = root.join("info");
    create_dir(&files_dir)?;
    create_dir(&info_dir)?;

    let original = absolutize(path)?;
    let name = original
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?
        .to_string_lossy()
        .into_owned();
    let when = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let stored = topdir_of(root)
        .and_then(|top| original.strip_prefix(top).ok())
        .filter(|rel| !rel.as_os_str().is_empty())
        .unwrap_or(&original);
    let body = trashinfo(stored, when);

    // The spec's claim: `info/STEM.trashinfo` made with O_EXCL owns STEM,
    // and the move refuses a taken `files/STEM`, so a concurrent trasher
    // on the same stem loses neither its info nor its file.
    let mut stems = candidates(&name);
    let (files, info) = loop {
        let stem = stems.next().expect("candidates never ends");
        let info = info_dir.join(format!("{stem}.trashinfo"));
        let mut claim = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&info)
        {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let files = files_dir.join(&stem);
        let moved = claim
            .write_all(body.as_bytes())
            .and_then(|()| move_free(&original, &files, None));
        match moved {
            Ok(()) => break (files, info),
            Err(e) => {
                let _ = std::fs::remove_file(&info);
                if e.kind() != io::ErrorKind::AlreadyExists {
                    return Err(e);
                }
            }
        }
    };

    Ok(Trashed {
        original,
        files,
        info,
        deleted: Some(when),
    })
}

/// `path` made absolute against the current directory, without
/// resolving symlinks (unlike `canonicalize`): freedesktop's
/// `.trashinfo` `Path=` must be absolute, and a relative `original`
/// would resolve against whatever the CWD is later, at `restore` time.
fn absolutize(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

/// Whether `path` sits on the same file system as the trash at `root`, so
/// trashing it is a rename. Anything else has no Recycle Bin and is
/// deleted for good: moving it would copy it whole into the home trash.
/// `root` need not exist yet; its nearest existing ancestor stands in,
/// with links followed. A `path` that cannot be looked at counts as
/// recyclable, so trashing it reports the real error.
pub fn recyclable_in(root: &Path, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(item) = std::fs::symlink_metadata(path) else {
        return true;
    };
    root.ancestors()
        .find_map(|a| std::fs::metadata(a).ok())
        .is_some_and(|bin| bin.dev() == item.dev())
}

pub fn recyclable(path: &Path) -> bool {
    Bins::system().is_some_and(|b| b.recyclable(path))
}

/// [`Bins::trash_measured`] into the session's trashes.
pub fn trash(path: &Path, known: Option<bincap::Measured>) -> io::Result<(Trashed, Vec<PathBuf>)> {
    Bins::system()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no trash directory"))?
        .trash_measured(path, known)
}

/// Every trash a session uses: the home trash, and the one each other
/// mounted drive keeps at its top. A path goes to the home trash when it
/// is on the home trash's drive, else to its own drive's
/// `$topdir/.Trash/$uid` (when `.Trash` is a real folder with the sticky
/// bit) or `$topdir/.Trash-$uid`. A drive where neither can be made (read
/// only, a FAT drive mounted for another user) has no Recycle Bin.
#[derive(Clone, Debug)]
pub struct Bins {
    pub home: PathBuf,
    pub mounts: Vec<PathBuf>,
    pub unlisted: HashSet<PathBuf>,
    pub devices: HashMap<PathBuf, String>,
    pub policy: Policy,
    pub uid: u32,
}

static MOUNT_TABLE: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Make [`Bins::system`] read the mount table at `table` (`None`: the
/// real one again). For tests in other crates that point the home trash
/// at a temp dir: `/dev/null` keeps the real drives' trashes out.
#[doc(hidden)]
pub fn set_mount_table(table: Option<PathBuf>) {
    *lock(&MOUNT_TABLE) = table;
}

static HOME_TRASH: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Make [`Bins::system`] keep its home trash at `root` (`None`: under
/// `$XDG_DATA_HOME` again), for tests that cannot change the environment.
#[doc(hidden)]
pub fn set_home_trash(root: Option<PathBuf>) {
    *lock(&HOME_TRASH) = root;
}

impl Bins {
    pub fn system() -> Option<Bins> {
        let file = lock(&MOUNT_TABLE)
            .clone()
            .unwrap_or_else(|| PathBuf::from("/proc/self/mounts"));
        let table = std::fs::read(file).unwrap_or_default();
        let home = lock(&HOME_TRASH).clone();
        let mut bins = Bins::from_table(home.or_else(places::trash_root)?, &table);
        bins.policy = Policy::load();
        Some(bins)
    }

    pub fn from_table(home: PathBuf, table: &[u8]) -> Bins {
        let devices = places::mount_devices(table).into_iter().collect();
        let table = places::mount_table(table);
        let mut bins = Bins::new(home, table.iter().map(|(m, _)| m.clone()).collect());
        bins.unlisted = table
            .into_iter()
            .filter(|(_, fs)| places::unlisted_fs(fs))
            .map(|(m, _)| m)
            .collect();
        bins.devices = devices;
        bins
    }

    pub fn new(home: PathBuf, mounts: Vec<PathBuf>) -> Bins {
        Bins {
            home,
            mounts,
            unlisted: HashSet::new(),
            devices: HashMap::new(),
            policy: Policy::default(),
            // SAFETY: getuid has no failure mode.
            uid: unsafe { libc::getuid() },
        }
    }

    fn topdir(&self, path: &Path) -> Option<&Path> {
        self.mounts
            .iter()
            .filter(|m| path.starts_with(m))
            .max_by_key(|m| m.as_os_str().len())
            .map(PathBuf::as_path)
    }

    /// The trash deleting `path` uses, made first when `make`: the home
    /// trash for a path on its drive, else the drive's own. `None` when
    /// that drive has none and none can be made. Without `make` nothing on
    /// disk changes: a trash that would be made counts when its folder is
    /// writable.
    pub fn bin_for(&self, path: &Path, make: bool) -> Option<PathBuf> {
        // By the folder holding it: a mount point itself is an entry on
        // the drive below.
        let real = real(path).ok()?;
        let top = self.topdir(real.parent().unwrap_or(&real));
        if top == self.home_top() {
            return Some(self.home.clone());
        }
        self.drive_bin(top?, make)
    }

    pub fn home_top(&self) -> Option<&Path> {
        let home = self.home.ancestors().find_map(|a| a.canonicalize().ok())?;
        self.topdir(&home)
    }

    pub fn drive_key(&self, top: &Path) -> String {
        if Some(top) == self.home_top() {
            return bincap::HOME.into();
        }
        let device = self.devices.get(top).map_or("", String::as_str);
        bincap::drive_key(device, top)
    }

    fn rule_for(&self, root: &Path) -> Rule {
        if root == self.home {
            return self.policy.rule(bincap::HOME);
        }
        match topdir_of(root) {
            Some(top) => self.policy.rule(&self.drive_key(top)),
            None => self.policy.global,
        }
    }

    fn cap(&self, root: &Path) -> u64 {
        self.rule_for(root).cap(bincap::drive_size(root))
    }

    pub fn fate(&self, path: &Path) -> Fate {
        if !self.recyclable(path) {
            return Fate::NoBin;
        }
        match self.bin_for(path, false) {
            Some(root) if self.rule_for(&root).remove_immediately => Fate::RemoveImmediately,
            _ => Fate::Recycle,
        }
    }

    /// `path`'s size against the cap of the trash it would go to; `None`
    /// when it would not be recycled or cannot be looked at.
    pub fn measure(&self, path: &Path) -> Option<bincap::Measured> {
        if self.fate(path) != Fate::Recycle {
            return None;
        }
        let root = self.bin_for(path, false)?;
        bincap::Measured::of(path, self.cap(&root)).ok()
    }

    fn drive_bin(&self, top: &Path, make: bool) -> Option<PathBuf> {
        let shared = top.join(".Trash");
        if sticky_dir(&shared) {
            let own = shared.join(self.uid.to_string());
            if own_dir(&own, self.uid) && sound(&own, self.uid) && (make || writable(&own)) {
                return Some(own);
            }
            if std::fs::symlink_metadata(&own).is_err()
                && if make {
                    create_bin(&own).is_ok()
                } else {
                    writable(&shared)
                }
            {
                return Some(own);
            }
        }
        let own = top.join(format!(".Trash-{}", self.uid));
        if own_dir(&own, self.uid) {
            return (sound(&own, self.uid) && (make || writable(&own))).then_some(own);
        }
        if std::fs::symlink_metadata(&own).is_ok() {
            return None;
        }
        if make {
            create_bin(&own).ok().map(|()| own)
        } else {
            writable(top).then_some(own)
        }
    }

    /// Whether deleting `path` can move it into a trash: its drive has one
    /// or can make one, and the move is a rename. A `path` that cannot be
    /// looked at counts as recyclable, so trashing it reports the real
    /// error.
    pub fn recyclable(&self, path: &Path) -> bool {
        if std::fs::symlink_metadata(path).is_err() {
            return true;
        }
        self.bin_for(path, false)
            .is_some_and(|root| recyclable_in(&root, path))
    }

    /// Move `path` into the trash [`Bins::bin_for`] picks, made if need
    /// be, within the size its drive's rule caps that trash at: the
    /// trash's oldest items are erased first until `path` fits
    /// ([`bincap::make_room`]). Returns the item trashed and the `files/`
    /// path of each item erased to make room, for the host to drop the
    /// undos that would put those back ([`crate::undo::UndoStack::forget_gone`]).
    /// Refused with nothing moved or erased: on a drive with no Recycle
    /// Bin with `CrossesDevices`, on one set to remove files immediately
    /// with `Unsupported`, and an item bigger than the whole cap with
    /// `FileTooLarge`.
    pub fn trash(&self, path: &Path) -> io::Result<(Trashed, Vec<PathBuf>)> {
        self.trash_measured(path, None)
    }

    /// [`Bins::trash`], reusing `known`'s size when it still holds
    /// ([`bincap::Measured`]); otherwise the item is measured again.
    pub fn trash_measured(
        &self,
        path: &Path,
        known: Option<bincap::Measured>,
    ) -> io::Result<(Trashed, Vec<PathBuf>)> {
        let root = self.bin_for(path, true).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::CrossesDevices,
                "it is on a drive without a Recycle Bin",
            )
        })?;
        if self.rule_for(&root).remove_immediately {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "its drive removes files immediately when deleted",
            ));
        }
        // By its real folder off the home trash's drive, so the record is
        // relative to the drive's top.
        let path = match root == self.home {
            true => path.to_path_buf(),
            false => real(path)?,
        };
        // `trash_in` refuses an item on another drive before any walk.
        if !recyclable_in(&root, &path) {
            return trash_in(&root, &path).map(|t| (t, Vec::new()));
        }
        let cap = self.cap(&root);
        let size = match known.and_then(|m| m.still(&path)) {
            Some(size) => size,
            None => bincap::item_size(&path, cap)?,
        };
        if size > cap {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "it is too big for the Recycle Bin",
            ));
        }
        let (used, purged) = bincap::make_room(&root, bincap::used(&root), size, cap)?;
        let t = trash_in(&root, &path)?;
        bincap::took(&root, &t, size, used);
        Ok((t, purged))
    }

    pub fn roots(&self) -> Vec<PathBuf> {
        let mut seen: HashSet<(u64, u64)> = self.home_key().into_iter().collect();
        let mut out = vec![self.home.clone()];
        for top in self.mounts.iter().filter(|m| !self.unlisted.contains(*m)) {
            let found = self.top_roots(top).into_iter();
            out.extend(found.filter(|(_, key)| seen.insert(*key)).map(|(r, _)| r));
        }
        out
    }

    /// The trashes of drive `top` that [`Bins::roots`] lists, the home trash left out. Looks
    /// inside `top` only.
    pub fn drive_roots(&self, top: &Path) -> Vec<PathBuf> {
        let home = self.home_key();
        (self.top_roots(top).into_iter())
            .filter(|(_, key)| Some(*key) != home)
            .map(|(r, _)| r)
            .collect()
    }

    fn home_key(&self) -> Option<(u64, u64)> {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(&self.home)
            .ok()
            .map(|m| (m.dev(), m.ino()))
    }

    /// The user's trash folders at `top`, each with its device and inode.
    fn top_roots(&self, top: &Path) -> Vec<(PathBuf, (u64, u64))> {
        use std::os::unix::fs::MetadataExt;
        let shared = top.join(".Trash");
        let found = [
            sticky_dir(&shared).then(|| shared.join(self.uid.to_string())),
            Some(top.join(format!(".Trash-{}", self.uid))),
        ];
        let mut out: Vec<(PathBuf, (u64, u64))> = Vec::new();
        for root in found.into_iter().flatten() {
            let Ok(m) = std::fs::symlink_metadata(&root) else {
                continue;
            };
            let key = (m.dev(), m.ino());
            if m.is_dir() && m.uid() == self.uid && !out.iter().any(|(_, k)| *k == key) {
                out.push((root, key));
            }
        }
        out
    }

    pub fn files_dirs(&self) -> Vec<PathBuf> {
        self.roots()
            .into_iter()
            .filter_map(|r| {
                let files = r.join("files");
                let meta = match r == self.home {
                    true => files.metadata(),
                    false => files.symlink_metadata(),
                };
                meta.is_ok_and(|m| m.is_dir()).then_some(files)
            })
            .collect()
    }

    /// Empty every trash ([`empty_in`] each of [`Bins::roots`]), returning
    /// how many items went. One that fails does not stop the rest; the
    /// error is the first failure.
    pub fn empty(&self) -> io::Result<usize> {
        let mut count = 0;
        let mut first = None;
        for root in self.roots() {
            match empty_in(&root) {
                Ok(n) => count += n,
                Err(e) => {
                    first.get_or_insert(e);
                }
            }
        }
        first.map_or(Ok(count), Err)
    }
}

/// `path` made absolute with its folder's links resolved and its own name
/// kept: where it really is, without following a link that `path` itself
/// is.
fn real(path: &Path) -> io::Result<PathBuf> {
    let path = absolutize(path)?;
    match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) => Ok(dir
            .canonicalize()
            .unwrap_or_else(|_| dir.to_path_buf())
            .join(name)),
        _ => Ok(path),
    }
}

fn sticky_dir(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() && m.permissions().mode() & 0o1000 != 0)
}

fn own_dir(p: &Path, uid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() && m.uid() == uid)
}

/// Whether the drive trash at `root` has its `files` and `info` as real folders of `uid`'s, or
/// not yet at all: one that leads elsewhere takes nothing.
fn sound(root: &Path, uid: u32) -> bool {
    ["files", "info"].into_iter().all(|sub| {
        own_dir(&root.join(sub), uid) || {
            std::fs::symlink_metadata(root.join(sub))
                .is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
        }
    })
}

fn writable(p: &Path) -> bool {
    let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `c` is a valid NUL-terminated path.
    unsafe { libc::access(c.as_ptr(), libc::W_OK | libc::X_OK) == 0 }
}

fn create_bin(root: &Path) -> io::Result<()> {
    std::fs::DirBuilder::new().mode(0o700).create(root)
}

/// The drive top a trash at `root` records paths against: the folder
/// holding `.Trash-$uid`, or the one holding `.Trash/$uid`. `None` for
/// any other trash: the home trash records absolute paths.
pub(crate) fn topdir_of(root: &Path) -> Option<&Path> {
    let parent = root.parent()?;
    if root.file_name()?.as_bytes().starts_with(b".Trash-") {
        Some(parent)
    } else if parent.file_name().is_some_and(|n| n == ".Trash") {
        parent.parent()
    } else {
        None
    }
}

/// Move `t` back to its original path. Fails with `AlreadyExists` if
/// something is there already; on success also removes the info file.
pub fn restore(t: &Trashed) -> io::Result<()> {
    move_to_free(&t.files, &t.original)?;
    let _ = std::fs::remove_file(&t.info);
    bincap::gone(&t.files);
    Ok(())
}

/// The trashed item at `files`, an entry of a trash's `files/`, with the
/// original path and deletion date its `info/NAME.trashinfo` records.
/// Fails with `NotFound` when there is no info file, and `InvalidData`
/// when it names no absolute path, or for a drive's trash a path off that
/// drive.
pub fn trashed(files: &Path) -> io::Result<Trashed> {
    let info = info_path(files)?;
    let root = bin_root(files)?;
    let body = BinDir::open(root)
        .and_then(|bin| bin.read_info(info.file_name().unwrap_or_default()))
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => io::Error::new(
                io::ErrorKind::NotFound,
                "the Recycle Bin has no record of where it came from",
            ),
            _ => e,
        })?;
    // A relative record is against the drive's top, or for the home
    // trash the folder holding it.
    let top = topdir_of(root);
    let original = original_path(&body, top.or(root.parent())).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "the Recycle Bin's record of where it came from is unreadable",
        )
    })?;
    if let Some(top) = top
        && bindir::beneath(top, &original).is_none()
    {
        return Err(bindir::outside());
    }
    Ok(Trashed {
        original,
        files: files.to_path_buf(),
        info,
        deleted: deletion_date(&body),
    })
}

fn info_value<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let mut in_group = false;
    for line in body.lines() {
        let line = line.trim_end();
        if line.starts_with('[') {
            in_group = line == "[Trash Info]";
        } else if in_group && let Some(v) = line.strip_prefix(key).and_then(|r| r.strip_prefix('='))
        {
            return Some(v);
        }
    }
    None
}

/// The `Path=` key of a `.trashinfo` body's `[Trash Info]` group,
/// decoded. A relative one is resolved against `base`, the folder the
/// trash sits in (a drive's top for a drive trash). `None` when it is
/// missing, or relative with no `base` or with a `..` in it.
pub fn original_path(body: &str, base: Option<&Path>) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let v = info_value(body, "Path")?;
    let path = PathBuf::from(std::ffi::OsString::from_vec(percent_decode(v)));
    if path.is_absolute() {
        return Some(path);
    }
    let climbs = path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir));
    if climbs || path.as_os_str().is_empty() {
        return None;
    }
    base.map(|b| b.join(path))
}

/// The `DeletionDate=` key of a `.trashinfo` body's `[Trash Info]` group,
/// `YYYY-MM-DDThh:mm:ss` in local time, as epoch seconds. `None` when it
/// is missing or not in that form.
pub fn deletion_date(body: &str) -> Option<i64> {
    let v = info_value(body, "DeletionDate")?.as_bytes();
    let shaped = v.len() == 19
        && v.iter().enumerate().all(|(i, &b)| match i {
            4 | 7 => b == b'-',
            10 => b == b'T',
            13 | 16 => b == b':',
            _ => b.is_ascii_digit(),
        });
    if !shaped {
        return None;
    }
    let num = |from: usize, to: usize| {
        v[from..to]
            .iter()
            .fold(0i32, |n, &d| n * 10 + i32::from(d - b'0'))
    };
    let (mo, d, h, mi, s) = (num(5, 7), num(8, 10), num(11, 13), num(14, 16), num(17, 19));
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let when = locale::mktime(num(0, 4), mo, d, h, mi, s);
    (when != -1).then_some(when)
}

/// Check that the item at `files` (an entry of a trash's `files/`) is
/// still the one `expected` records, as it was listed: the same original
/// path and deletion date, or, for `None`, still no readable record. An
/// action picked on a listing so never lands on an item that has taken
/// the name since. Fails with `Other` on a mismatch.
pub fn check_listed(files: &Path, expected: Option<&Trashed>) -> io::Result<()> {
    if trashed(files).ok().as_ref() == expected {
        Ok(())
    } else {
        Err(io::Error::other(
            "it has changed since the Recycle Bin was listed",
        ))
    }
}

fn info_path(files: &Path) -> io::Result<PathBuf> {
    let (name, root) = (item_name(files)?, bin_root(files)?);
    Ok(root.join("info").join(info_name(name)))
}

fn item_name(files: &Path) -> io::Result<&std::ffi::OsStr> {
    files.file_name().ok_or_else(not_in_bin)
}

/// The trash holding `files`, an entry of its `files/`.
fn bin_root(files: &Path) -> io::Result<&Path> {
    files.parent().and_then(Path::parent).ok_or_else(not_in_bin)
}

fn not_in_bin() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "not an item in the Recycle Bin",
    )
}

fn info_name(name: &std::ffi::OsStr) -> std::ffi::OsString {
    let mut info = name.to_os_string();
    info.push(".trashinfo");
    info
}

/// Delete trashed item `files` for good through [`BinDir`], once [`check_listed`] finds it is
/// still `expected`. The bin lists `files/`, so a record left behind is only logged.
pub fn erase_trashed(files: &Path, expected: Option<&Trashed>) -> io::Result<()> {
    erase_trashed_with(files, expected, None)
}

/// [`erase_trashed`], each file counted on `meter` when given.
pub(crate) fn erase_trashed_with(
    files: &Path,
    expected: Option<&Trashed>,
    meter: Option<&Meter>,
) -> io::Result<()> {
    let (name, root) = (item_name(files)?, bin_root(files)?);
    check_listed(files, expected)?;
    let bin = BinDir::open(root)?;
    if let Some(m) = meter {
        m.hold()?;
    }
    bin.erase(name, files, meter)?;
    if let Err(e) = bin.unlink_info(&info_name(name))
        && e.kind() != io::ErrorKind::NotFound
    {
        log::warn!("trash: left the record of {} behind: {e}", files.display());
    }
    bincap::gone(files);
    Ok(())
}

/// Move `t` back to its original path, making gone folders and numbering a taken name, and
/// return where it landed; a drive's item only onto its drive ([`bindir::restore_beneath`]).
pub fn restore_free(t: &Trashed) -> io::Result<PathBuf> {
    let (Some(dir), Some(name)) = (t.original.parent(), t.original.file_name()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "it has no original location",
        ));
    };
    let root = bin_root(&t.files)?;
    if let Some(top) = topdir_of(root) {
        let bin = BinDir::open(root)?;
        let to = bindir::restore_beneath(&bin, &t.files, top, dir, name)?;
        let _ = bin.unlink_info(&info_name(item_name(&t.files)?));
        bincap::gone(&t.files);
        return Ok(to);
    }
    std::fs::create_dir_all(dir)?;
    for candidate in crate::names::numbered(name, crate::names::paren) {
        let to = dir.join(candidate);
        match move_to_free(&t.files, &to) {
            Ok(()) => {
                let _ = std::fs::remove_file(&t.info);
                bincap::gone(&t.files);
                return Ok(to);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!("numbered never ends")
}

/// Delete everything in the trash at `root` through [`BinDir`], returning how many items went.
/// A failed item keeps its record and the rest carry on; the error carries the first failure.
pub fn empty_in(root: &Path) -> io::Result<usize> {
    empty_with(root, None)
}

/// [`empty_in`], each file counted on `meter` when given; a cancel stops at once.
pub(crate) fn empty_with(root: &Path, meter: Option<&Meter>) -> io::Result<usize> {
    let bin = match BinDir::open(root) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        bin => bin?,
    };
    let mut count = 0;
    let mut failed = 0;
    let mut first: Option<io::Error> = None;
    let mut fail = |e: io::Error| {
        failed += 1;
        first.get_or_insert(e);
    };
    let gone = |r: io::Result<()>| match r {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        r => r,
    };
    for name in bin.files() {
        let path = root.join("files").join(&name);
        let erased = (|| {
            if let Some(m) = meter {
                m.set_current(&name);
                m.hold()?;
            }
            bin.erase(&name, &path, meter)?;
            gone(bin.unlink_info(&info_name(&name)))
        })();
        match erased {
            Ok(()) => count += 1,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => return Err(e),
            Err(e) => fail(e),
        }
    }
    let left: HashSet<std::ffi::OsString> = bin.files().into_iter().collect();
    for name in bin.infos() {
        let stem = name.as_bytes().strip_suffix(b".trashinfo");
        if stem.is_some_and(|s| left.contains(std::ffi::OsStr::from_bytes(s))) {
            continue;
        }
        if let Err(e) = gone(bin.unlink_info(&name)) {
            fail(e);
        }
    }
    if let Err(e) = bin.clear_strays() {
        log::warn!(
            "trash: left a link or the size cache in {}: {e}",
            root.display()
        );
    }
    match first {
        None => Ok(count),
        Some(e) => Err(io::Error::new(
            e.kind(),
            format!(
                "{failed} item{} could not be deleted: {e}",
                if failed == 1 { "" } else { "s" }
            ),
        )),
    }
}

pub fn trashinfo(original: &Path, when: i64) -> String {
    let tm = locale::localtime(when);
    format!(
        "[Trash Info]\nPath={}\nDeletionDate={:04}-{:02}-{:02}T{:02}:{:02}:{:02}\n",
        percent_encode(original),
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
    )
}

pub fn percent_encode(path: &Path) -> String {
    let mut out = String::new();
    for &byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Undo URL percent-encoding into the raw bytes, which need not be
/// UTF-8; bad sequences are kept literally.
pub fn percent_decode(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// The freedesktop-style names to try for `name`, in order: `x.txt`,
/// then `x.2.txt`, `x.3.txt`... (no extension: `x`, `x.2`, `x.3`...).
fn candidates(name: &str) -> impl Iterator<Item = String> {
    crate::names::numbered(std::ffi::OsStr::new(name), |n| format!(".{n}"))
        .map(|c| c.to_string_lossy().into_owned())
}

fn create_dir(dir: &Path) -> io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

fn move_path(from: &Path, to: &Path, stop: &AtomicBool) -> io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => copy_then_remove(from, to, stop),
        Err(e) => Err(e),
    }
}

/// The cross-device half of `move_path`. `symlink_metadata`, not
/// `is_dir`: a link is moved as a link, not by copying whatever tree it
/// points at and unlinking only the link. A mount point is refused before
/// the copy, as [`move_to_free`] refuses it before the rename, and so is a
/// folder holding one: the remove would empty that drive.
/// The copy is built and flushed under a hidden part name, then renamed to `to`
/// ([`crate::ops::land_part`]): `stop` set by then, or a failed flush, removes only
/// the part and keeps `from`.
fn copy_then_remove(from: &Path, to: &Path, stop: &AtomicBool) -> io::Result<()> {
    refuse_mount_point(from)?;
    let kind = std::fs::symlink_metadata(from)?.file_type();
    if kind.is_dir() {
        refuse_holding_mount(from)?;
    }
    let link = kind
        .is_symlink()
        .then(|| std::fs::read_link(from))
        .transpose()?;
    let make = |part: &Path| match &link {
        Some(target) => std::os::unix::fs::symlink(target, part),
        None if kind.is_dir() => build_dir(from, part, stop),
        None => crate::ops::copy_new(from, part),
    };
    let ready = |part: &Path| crate::ops::sync_landed(part).and_then(|()| crate::ops::go_on(stop));
    crate::ops::land_part(to, make, ready)?;
    // The new name is on the disk before the original goes.
    let dir = to.parent().unwrap_or(Path::new("."));
    if let Err(e) = std::fs::File::open(dir).and_then(|d| d.sync_all()) {
        log::warn!(
            "reprisefs: moved {} to {}, but the original stays: {e}",
            from.display(),
            to.display()
        );
        return Ok(());
    }
    if kind.is_dir() {
        crate::ops::erase_stepped(from)
    } else {
        std::fs::remove_file(from)
    }
}

/// `renameat2(RENAME_NOREPLACE)` makes the refusal atomic; a file system
/// without it (`EINVAL`) gets test-then-rename. `EXDEV` comes before the
/// kernel looks at `to`, so a taken `to` is checked before the copy, and
/// the copy's final rename fails on a name taken in between.
///
/// A `to` that is `from` under another case on a case-insensitive file
/// system (FAT, exFAT) is a case-only rename, done through a free name. A
/// `from` that [is a mount point](crate::ops::is_mount_point) is refused
/// with `ResourceBusy` before anything moves.
pub fn move_to_free(from: &Path, to: &Path) -> io::Result<()> {
    move_to_free_until(from, to, &AtomicBool::new(false))
}

/// [`move_to_free`], whose copy to another file system is stopped by `stop` as
/// [`copy_then_remove`] stops it.
pub(crate) fn move_to_free_until(from: &Path, to: &Path, stop: &AtomicBool) -> io::Result<()> {
    move_free(from, to, Some(stop))
}

/// [`move_to_free`] that refuses to cross file systems (`CrossesDevices`).
pub(crate) fn rename_free(from: &Path, to: &Path) -> io::Result<()> {
    move_free(from, to, None)
}

/// [`move_to_free`], copying across file systems only with `copy`, stopped by
/// its flag; without it a move onto another file system fails with
/// `CrossesDevices`.
fn move_free(from: &Path, to: &Path, copy: Option<&AtomicBool>) -> io::Result<()> {
    refuse_mount_point(from)?;
    if same_file_other_case(from, to) {
        return rename_case(from, to);
    }
    let taken = || {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", to.display()),
        )
    };
    let (Ok(from_c), Ok(to_c)) = (
        std::ffi::CString::new(from.as_os_str().as_bytes()),
        std::ffi::CString::new(to.as_os_str().as_bytes()),
    ) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path holds a NUL",
        ));
    };
    // SAFETY: both are valid NUL-terminated paths; AT_FDCWD resolves them
    // against the working directory, as `rename` would.
    let rc = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from_c.as_ptr(),
            libc::AT_FDCWD,
            to_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if rc == 0 {
        return Ok(());
    }
    let e = io::Error::last_os_error();
    match e.raw_os_error() {
        Some(libc::EEXIST) => Err(taken()),
        Some(libc::EXDEV) if std::fs::symlink_metadata(to).is_ok() => Err(taken()),
        Some(libc::EXDEV) => match copy {
            Some(stop) => copy_then_remove(from, to, stop),
            None => Err(io::Error::new(
                io::ErrorKind::CrossesDevices,
                "it is on another drive",
            )),
        },
        Some(libc::EINVAL) | Some(libc::ENOSYS) => {
            // No RENAME_NOREPLACE here. A file or a link is linked in
            // under the new name first, which refuses a taken name in the
            // same step, then unlinked from the old one; a folder, which
            // cannot be linked, is checked and renamed.
            if !std::fs::symlink_metadata(from)?.is_dir() {
                match std::fs::hard_link(from, to) {
                    Ok(()) => {
                        return std::fs::remove_file(from).inspect_err(|_| {
                            let _ = std::fs::remove_file(to);
                        });
                    }
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(taken()),
                    Err(_) => {}
                }
            }
            if std::fs::symlink_metadata(to).is_ok() {
                return Err(taken());
            }
            match copy {
                Some(stop) => move_path(from, to, stop),
                None => std::fs::rename(from, to),
            }
        }
        _ => Err(e),
    }
}

/// Whether `to` is `from`'s own directory entry under a name that differs
/// only in case: same folder, same inode. Two hard links with such names
/// match too; `rename_case` then finds `to` taken and refuses.
fn same_file_other_case(from: &Path, to: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let (Some(a), Some(b)) = (
        from.file_name().and_then(|n| n.to_str()),
        to.file_name().and_then(|n| n.to_str()),
    ) else {
        return false;
    };
    if a == b || a.to_lowercase() != b.to_lowercase() || from.parent() != to.parent() {
        return false;
    }
    match (
        std::fs::symlink_metadata(from),
        std::fs::symlink_metadata(to),
    ) {
        (Ok(f), Ok(t)) => (f.dev(), f.ino()) == (t.dev(), t.ino()),
        _ => false,
    }
}

/// Rename `from` to `to`, a name differing only in case, in two moves
/// through a free name beside it, since a case-insensitive file system
/// takes a direct rename as one onto itself. A failed second move puts
/// `from` back.
fn rename_case(from: &Path, to: &Path) -> io::Result<()> {
    let dir = from.parent().unwrap_or(Path::new("."));
    let via = (0..)
        .map(|n| dir.join(format!(".reprise-rename-{n}")))
        .find(|p| std::fs::symlink_metadata(p).is_err())
        .expect("an unbounded range yields a free name");
    move_to_free(from, &via)?;
    move_to_free(&via, to).inspect_err(|_| {
        let _ = move_to_free(&via, from);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn move_to_free_refuses_a_dangling_symlink_in_the_way() {
        let t = tempfile::tempdir().unwrap();
        let (from, to) = (t.path().join("a.txt"), t.path().join("b.txt"));
        fs::write(&from, "a").unwrap();
        std::os::unix::fs::symlink(t.path().join("gone"), &to).unwrap();
        let e = move_to_free(&from, &to).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert!(fs::symlink_metadata(&to).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&from).unwrap(), "a");
    }

    #[test]
    fn rename_case_moves_the_file_to_the_new_case() {
        let t = tempfile::tempdir().unwrap();
        let (from, to) = (t.path().join("notes.txt"), t.path().join("Notes.txt"));
        fs::write(&from, "n").unwrap();
        rename_case(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(fs::read_to_string(&to).unwrap(), "n");
        let left: Vec<_> = fs::read_dir(t.path()).unwrap().collect();
        assert_eq!(left.len(), 1, "only Notes.txt is left");
    }

    #[test]
    fn a_hard_link_under_another_case_is_a_taken_name() {
        let t = tempfile::tempdir().unwrap();
        let (from, to) = (t.path().join("a.txt"), t.path().join("A.txt"));
        fs::write(&from, "a").unwrap();
        fs::hard_link(&from, &to).unwrap();
        assert!(same_file_other_case(&from, &to));
        let e = move_to_free(&from, &to).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert!(from.exists() && to.exists(), "both links stay");
        let left: Vec<_> = fs::read_dir(t.path()).unwrap().collect();
        assert_eq!(left.len(), 2, "no rename leftover");
    }

    fn root() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        (t, root)
    }

    fn drives() -> (tempfile::TempDir, Bins, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home/.local/share/Trash");
        let drive = t.path().join("media/STICK");
        fs::create_dir_all(&drive).unwrap();
        let bins = Bins::new(home, vec![t.path().to_path_buf(), drive.clone()]);
        (t, bins, drive)
    }

    fn uid() -> u32 {
        // SAFETY: getuid has no failure mode.
        unsafe { libc::getuid() }
    }

    #[test]
    fn a_file_on_another_drive_goes_to_that_drives_own_trash() {
        let (_t, bins, drive) = drives();
        fs::create_dir(drive.join("docs")).unwrap();
        let src = drive.join("docs/x.txt");
        fs::write(&src, "x").unwrap();
        assert!(bins.recyclable(&src));

        let (put, _) = bins.trash(&src).unwrap();

        let root = drive.join(format!(".Trash-{}", uid()));
        assert_eq!(put.files, root.join("files/x.txt"));
        let mode = fs::metadata(&root).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "the drive trash is private");
        let info = fs::read_to_string(&put.info).unwrap();
        assert_eq!(info.lines().nth(1), Some("Path=docs/x.txt"), "relative");
        assert!(!bins.home.exists(), "nothing lands in the home trash");

        let found = trashed(&put.files).unwrap();
        assert_eq!(found.original, src);
        restore_free(&found).unwrap();
        assert_eq!(fs::read_to_string(&src).unwrap(), "x");
    }

    #[test]
    fn a_file_on_the_home_drive_goes_to_the_home_trash() {
        let (t, bins, _drive) = drives();
        let src = t.path().join("home/x.txt");
        fs::create_dir_all(src.parent().unwrap()).unwrap();
        fs::write(&src, "x").unwrap();

        let (put, _) = bins.trash(&src).unwrap();

        assert_eq!(put.files, bins.home.join("files/x.txt"));
        let info = fs::read_to_string(&put.info).unwrap();
        assert_eq!(
            info.lines().nth(1).unwrap(),
            format!("Path={}", percent_encode(&src))
        );
    }

    #[test]
    fn a_sticky_shared_trash_takes_a_folder_per_user() {
        let (_t, bins, drive) = drives();
        let shared = drive.join(".Trash");
        fs::create_dir(&shared).unwrap();
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o1777)).unwrap();
        let src = drive.join("x.txt");
        fs::write(&src, "x").unwrap();

        let (put, _) = bins.trash(&src).unwrap();

        assert_eq!(put.files, shared.join(format!("{}/files/x.txt", uid())));
        assert_eq!(
            fs::read_to_string(&put.info).unwrap().lines().nth(1),
            Some("Path=x.txt")
        );
        assert_eq!(trashed(&put.files).unwrap().original, src);
    }

    #[test]
    fn a_shared_trash_without_the_sticky_bit_or_as_a_link_is_passed_over() {
        let (t, bins, drive) = drives();
        fs::create_dir(drive.join(".Trash")).unwrap();
        fs::set_permissions(drive.join(".Trash"), fs::Permissions::from_mode(0o777)).unwrap();
        let src = drive.join("x.txt");
        fs::write(&src, "x").unwrap();
        let own = drive.join(format!(".Trash-{}", uid()));
        assert_eq!(bins.trash(&src).unwrap().0.files, own.join("files/x.txt"));

        let (_u, bins2, drive2) = drives();
        let elsewhere = t.path().join("sticky");
        fs::create_dir(&elsewhere).unwrap();
        fs::set_permissions(&elsewhere, fs::Permissions::from_mode(0o1777)).unwrap();
        std::os::unix::fs::symlink(&elsewhere, drive2.join(".Trash")).unwrap();
        fs::write(drive2.join("y.txt"), "y").unwrap();
        let (put, _) = bins2.trash(&drive2.join("y.txt")).unwrap();
        assert!(
            put.files
                .starts_with(drive2.join(format!(".Trash-{}", uid())))
        );
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
    }

    /// A drive trash name that is a link is somebody else's doing: the
    /// drive has no Recycle Bin rather than one that leads elsewhere.
    #[test]
    fn a_linked_drive_trash_is_not_used() {
        let (t, bins, drive) = drives();
        fs::create_dir(t.path().join("away")).unwrap();
        let own = drive.join(format!(".Trash-{}", uid()));
        std::os::unix::fs::symlink(t.path().join("away"), &own).unwrap();
        let src = drive.join("x.txt");
        fs::write(&src, "x").unwrap();

        assert!(!bins.recyclable(&src));
        let e = bins.trash(&src).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::CrossesDevices);
        assert!(src.exists());
        assert_eq!(fs::read_dir(t.path().join("away")).unwrap().count(), 0);
    }

    #[test]
    fn a_drive_that_cannot_hold_a_trash_has_no_recycle_bin() {
        let (_t, bins, drive) = drives();
        let src = drive.join("x.txt");
        fs::write(&src, "x").unwrap();
        fs::set_permissions(&drive, fs::Permissions::from_mode(0o555)).unwrap();

        let recyclable = bins.recyclable(&src);
        let e = bins.trash(&src).map(drop);

        fs::set_permissions(&drive, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!recyclable);
        assert_eq!(e.unwrap_err().kind(), io::ErrorKind::CrossesDevices);
        assert!(src.exists());
        assert_eq!(fs::read_dir(&drive).unwrap().count(), 1, "nothing made");
    }

    #[test]
    fn asking_whether_a_drive_has_a_recycle_bin_makes_nothing() {
        let (_t, bins, drive) = drives();
        let src = drive.join("x.txt");
        fs::write(&src, "x").unwrap();
        assert!(bins.recyclable(&src));
        assert_eq!(fs::read_dir(&drive).unwrap().count(), 1);
        // The mount point itself is an entry on the drive below it.
        assert_eq!(bins.bin_for(&drive, false), Some(bins.home.clone()));
    }

    #[test]
    fn roots_list_the_home_trash_then_each_drives_and_empty_clears_them_all() {
        let (t, mut bins, drive) = drives();
        let a = t.path().join("home/a.txt");
        fs::create_dir_all(a.parent().unwrap()).unwrap();
        fs::write(&a, "a").unwrap();
        let b = drive.join("b.txt");
        fs::write(&b, "b").unwrap();
        bins.trash(&a).unwrap();
        bins.trash(&b).unwrap();
        let own = drive.join(format!(".Trash-{}", uid()));

        assert_eq!(bins.roots(), [bins.home.clone(), own.clone()]);
        assert_eq!(bins.drive_roots(&drive), std::slice::from_ref(&own));
        assert_eq!(
            bins.files_dirs(),
            [bins.home.join("files"), own.join("files")]
        );
        bins.mounts.push(drive.clone());
        assert_eq!(bins.roots().len(), 2);
        let ejected = Bins::new(bins.home.clone(), vec![t.path().to_path_buf()]);
        assert_eq!(ejected.roots(), [bins.home.clone()]);
        bins.unlisted.insert(drive.clone());
        assert_eq!(bins.roots(), [bins.home.clone()]);
        bins.unlisted.clear();

        assert_eq!(bins.empty().unwrap(), 2);
        assert_eq!(fs::read_dir(own.join("files")).unwrap().count(), 0);
        assert_eq!(fs::read_dir(bins.home.join("files")).unwrap().count(), 0);
    }

    #[test]
    fn from_table_reads_the_mounts_and_leaves_kernel_and_network_ones_unlisted() {
        let table = b"proc /proc proc rw 0 0\n/dev/sdb1 /run/media/u/My\\040Stick vfat rw 0 0\n\
            srv:/x /mnt/nfs nfs4 rw 0 0\n/dev/sda2 / ext4 rw 0 0\n";
        let bins = Bins::from_table(PathBuf::from("/h/Trash"), table);
        assert_eq!(
            bins.mounts,
            [
                PathBuf::from("/proc"),
                PathBuf::from("/run/media/u/My Stick"),
                PathBuf::from("/mnt/nfs"),
                PathBuf::from("/"),
            ]
        );
        let mut unlisted: Vec<_> = bins.unlisted.iter().cloned().collect();
        unlisted.sort();
        assert_eq!(
            unlisted,
            [PathBuf::from("/mnt/nfs"), PathBuf::from("/proc")]
        );
    }

    #[test]
    fn trash_in_moves_the_file_and_writes_info() {
        let (t, root) = root();
        let src = t.path().join("x.txt");
        fs::write(&src, "hi").unwrap();

        let trashed = trash_in(&root, &src).unwrap();

        assert!(!src.exists());
        assert_eq!(fs::read_to_string(&trashed.files).unwrap(), "hi");
        assert_eq!(trashed.files, root.join("files/x.txt"));
        assert_eq!(trashed.info, root.join("info/x.txt.trashinfo"));

        let info = fs::read_to_string(&trashed.info).unwrap();
        assert!(info.starts_with("[Trash Info]\n"));
        let path_line = info.lines().nth(1).unwrap();
        assert_eq!(path_line, format!("Path={}", percent_encode(&src)));
        let date_line = info.lines().nth(2).unwrap();
        let date = date_line.strip_prefix("DeletionDate=").unwrap();
        let re_ok = date.len() == 19
            && date.as_bytes()[4] == b'-'
            && date.as_bytes()[7] == b'-'
            && date.as_bytes()[10] == b'T'
            && date.as_bytes()[13] == b':'
            && date.as_bytes()[16] == b':'
            && date
                .bytes()
                .enumerate()
                .all(|(i, b)| matches!(i, 4 | 7 | 10 | 13 | 16) || b.is_ascii_digit());
        assert!(
            re_ok,
            "DeletionDate {date:?} doesn't match YYYY-MM-DDThh:mm:ss"
        );
    }

    #[test]
    fn trash_in_moves_a_plain_folder_with_its_contents() {
        let (t, root) = root();
        let src = t.path().join("Folder");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("sub/a.txt"), "a").unwrap();

        let trashed = trash_in(&root, &src).unwrap();

        assert!(!src.exists());
        assert_eq!(
            fs::read_to_string(trashed.files.join("sub/a.txt")).unwrap(),
            "a"
        );
    }

    #[test]
    fn trash_in_creates_the_trash_and_its_parents_private() {
        let (t, _) = root();
        let root = t.path().join("share/Trash");
        let src = t.path().join("x.txt");
        fs::write(&src, "1").unwrap();

        trash_in(&root, &src).unwrap();

        for dir in [
            "share",
            "share/Trash",
            "share/Trash/files",
            "share/Trash/info",
        ] {
            let mode = fs::metadata(t.path().join(dir))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700, "{dir} is 0700");
        }
    }

    #[test]
    fn trash_in_refuses_a_path_on_another_drive_and_leaves_it() {
        let (_t, root) = root();
        // procfs is never the temp dir's file system.
        let other = Path::new("/proc/version");
        assert!(!recyclable_in(&root, other));

        let e = trash_in(&root, other).expect_err("another drive is refused");

        assert_eq!(e.kind(), io::ErrorKind::CrossesDevices);
        assert!(other.exists());
        assert!(!root.join("files").exists(), "nothing lands in the trash");
    }

    #[test]
    fn recyclable_in_takes_a_path_on_the_trash_drive() {
        let (t, root) = root();
        let src = t.path().join("x.txt");
        fs::write(&src, "1").unwrap();
        assert!(recyclable_in(&root, &src), "the root need not exist yet");
    }

    #[test]
    fn move_to_free_refuses_a_mount_point_as_its_source() {
        let t = tempfile::tempdir().unwrap();
        let to = t.path().join("root");

        let e = move_to_free(Path::new("/"), &to).unwrap_err();

        assert_eq!(e.kind(), io::ErrorKind::ResourceBusy);
        assert!(e.to_string().contains("mounted drive"), "{e}");
        assert!(!to.exists());
    }

    #[test]
    fn trash_in_numbers_a_same_named_file_freedesktop_style() {
        let (t, root) = root();
        let a = t.path().join("a").join("x.txt");
        let b = t.path().join("b").join("x.txt");
        fs::create_dir_all(a.parent().unwrap()).unwrap();
        fs::create_dir_all(b.parent().unwrap()).unwrap();
        fs::write(&a, "1").unwrap();
        fs::write(&b, "2").unwrap();

        let first = trash_in(&root, &a).unwrap();
        let second = trash_in(&root, &b).unwrap();

        assert_eq!(first.files, root.join("files/x.txt"));
        assert_eq!(second.files, root.join("files/x.2.txt"));
        assert_eq!(second.info, root.join("info/x.2.txt.trashinfo"));
        assert!(second.info.exists() && first.info.exists());
    }

    #[test]
    fn trash_in_numbers_an_extensionless_name() {
        let (t, root) = root();
        let a = t.path().join("a").join("x");
        let b = t.path().join("b").join("x");
        fs::create_dir_all(a.parent().unwrap()).unwrap();
        fs::create_dir_all(b.parent().unwrap()).unwrap();
        fs::write(&a, "1").unwrap();
        fs::write(&b, "2").unwrap();

        trash_in(&root, &a).unwrap();
        let second = trash_in(&root, &b).unwrap();

        assert_eq!(second.files, root.join("files/x.2"));
    }

    #[test]
    fn trash_in_skips_a_name_claimed_only_by_an_orphaned_info_file() {
        let (t, root) = root();
        let src = t.path().join("x.txt");
        fs::write(&src, "1").unwrap();
        fs::create_dir_all(root.join("info")).unwrap();
        fs::write(root.join("info/x.txt.trashinfo"), "orphan, no files/x.txt").unwrap();

        let trashed = trash_in(&root, &src).unwrap();

        assert_eq!(trashed.files, root.join("files/x.2.txt"));
        assert_eq!(trashed.info, root.join("info/x.2.txt.trashinfo"));
    }

    /// `files/x.txt` is a dangling link with no `.trashinfo`: a check that
    /// follows links sees the name as free, the claim does not.
    #[test]
    fn trash_in_skips_a_name_held_only_in_files_and_keeps_it() {
        let (t, root) = root();
        let src = t.path().join("x.txt");
        fs::write(&src, "mine").unwrap();
        fs::create_dir_all(root.join("files")).unwrap();
        let held = root.join("files/x.txt");
        std::os::unix::fs::symlink("/nonexistent/target", &held).unwrap();

        let trashed = trash_in(&root, &src).unwrap();

        assert_eq!(trashed.files, root.join("files/x.2.txt"));
        assert_eq!(trashed.info, root.join("info/x.2.txt.trashinfo"));
        assert_eq!(fs::read_to_string(&trashed.files).unwrap(), "mine");
        assert_eq!(
            fs::read_link(&held).unwrap(),
            Path::new("/nonexistent/target")
        );
        assert!(
            !root.join("info/x.txt.trashinfo").exists(),
            "the refused claim leaves no info file"
        );
    }

    #[test]
    fn restore_puts_it_back_and_removes_the_info_file() {
        let (t, root) = root();
        let src = t.path().join("x.txt");
        fs::write(&src, "hi").unwrap();
        let trashed = trash_in(&root, &src).unwrap();

        restore(&trashed).unwrap();

        assert_eq!(fs::read_to_string(&src).unwrap(), "hi");
        assert!(!trashed.files.exists());
        assert!(!trashed.info.exists());
    }

    #[test]
    fn restore_fails_when_the_original_path_is_taken() {
        let (t, root) = root();
        let src = t.path().join("x.txt");
        fs::write(&src, "hi").unwrap();
        let trashed = trash_in(&root, &src).unwrap();
        fs::write(&src, "new file landed here").unwrap();

        let err = restore(&trashed).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(trashed.files.exists(), "the trashed copy must stay put");
    }

    #[test]
    fn trashed_reads_the_original_path_from_the_info_file() {
        let (t, root) = root();
        let src = t.path().join("a b").join("ü.txt");
        fs::create_dir_all(src.parent().unwrap()).unwrap();
        fs::write(&src, "1").unwrap();
        let put = trash_in(&root, &src).unwrap();

        let found = trashed(&put.files).unwrap();

        assert_eq!(found.original, src);
        assert_eq!(found.files, put.files);
        assert_eq!(found.info, put.info);
    }

    #[test]
    fn trashed_without_an_info_file_is_not_found() {
        let (_t, root) = root();
        fs::create_dir_all(root.join("files")).unwrap();
        fs::write(root.join("files/stray.txt"), "1").unwrap();

        let e = trashed(&root.join("files/stray.txt")).err().unwrap();

        assert_eq!(e.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn original_path_reads_the_path_in_the_trash_info_group() {
        let body = "[Trash Info]\nDeletionDate=2026-09-15T12:34:56\nPath=/a%20b/%C3%BC.txt\n";
        assert_eq!(original_path(body, None), Some(PathBuf::from("/a b/ü.txt")));
        assert_eq!(original_path("[Other]\nPath=/x.txt\n", None), None);
        assert_eq!(original_path("[Trash Info]\nDeletionDate=x\n", None), None);
        let rel = "[Trash Info]\nPath=docs/x%20y.txt\n";
        assert_eq!(original_path(rel, None), None, "relative needs a base");
        assert_eq!(
            original_path(rel, Some(Path::new("/run/media/u/STICK"))),
            Some(PathBuf::from("/run/media/u/STICK/docs/x y.txt"))
        );
        let climb = "[Trash Info]\nPath=../etc/passwd\n";
        assert_eq!(original_path(climb, Some(Path::new("/m"))), None);
    }

    #[test]
    fn deletion_date_reads_local_time_in_the_trash_info_group() {
        let body = "[Trash Info]\nPath=/x.txt\nDeletionDate=2026-09-14T15:43:39\n";
        assert_eq!(
            deletion_date(body),
            Some(locale::mktime(2026, 9, 14, 15, 43, 39))
        );
        for bad in [
            "[Trash Info]\nDeletionDate=2026-09-14 15:43:39\n",
            "[Trash Info]\nDeletionDate=2026-13-14T15:43:39\n",
            "[Trash Info]\nDeletionDate=2026-09-14T15:43\n",
            "[Other]\nDeletionDate=2026-09-14T15:43:39\n",
            "[Trash Info]\nPath=/x.txt\n",
        ] {
            assert_eq!(deletion_date(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn trashed_carries_the_deletion_date_trash_in_wrote() {
        let (t, root) = root();
        let src = t.path().join("x.txt");
        fs::write(&src, "hi").unwrap();
        let put = trash_in(&root, &src).unwrap();

        let found = trashed(&put.files).unwrap();

        assert_eq!(found.deleted, put.deleted);
        assert!(found.deleted.is_some());
    }

    #[test]
    fn erase_trashed_removes_the_data_and_its_info_file_only() {
        let (t, root) = root();
        let (a, b) = (t.path().join("a"), t.path().join("b.txt"));
        fs::create_dir(&a).unwrap();
        fs::write(a.join("in.txt"), "1").unwrap();
        fs::write(&b, "2").unwrap();
        let (put_a, put_b) = (trash_in(&root, &a).unwrap(), trash_in(&root, &b).unwrap());

        erase_trashed(&put_a.files, Some(&put_a)).unwrap();

        assert!(fs::symlink_metadata(&put_a.files).is_err());
        assert!(!put_a.info.exists());
        assert!(put_b.files.exists() && put_b.info.exists(), "b stays");
        fs::remove_file(&put_b.info).unwrap();
        erase_trashed(&put_b.files, None).unwrap();
        assert!(!put_b.files.exists());
    }

    /// An item that took the name of the one listed is not the one the
    /// user picked: erase and the check refuse it and leave it be.
    #[test]
    fn erase_trashed_refuses_an_item_that_is_not_the_one_listed() {
        let (t, root) = root();
        let src = t.path().join("x.txt");
        fs::write(&src, "old").unwrap();
        let listed = trash_in(&root, &src).unwrap();
        fs::write(
            &listed.info,
            trashinfo(&t.path().join("elsewhere/x.txt"), 1_700_000_000),
        )
        .unwrap();

        let e = erase_trashed(&listed.files, Some(&listed)).unwrap_err();

        assert_eq!(e.kind(), io::ErrorKind::Other);
        assert!(listed.files.exists() && listed.info.exists());
        assert!(
            check_listed(&listed.files, None).is_err(),
            "it has a record"
        );
    }

    #[test]
    fn erase_trashed_counts_an_item_gone_when_only_its_info_file_stays() {
        let (t, root) = root();
        let src = t.path().join("x.txt");
        fs::write(&src, "x").unwrap();
        let listed = trash_in(&root, &src).unwrap();
        let info_dir = root.join("info");
        fs::set_permissions(&info_dir, fs::Permissions::from_mode(0o555)).unwrap();

        let erased = erase_trashed(&listed.files, Some(&listed));

        fs::set_permissions(&info_dir, fs::Permissions::from_mode(0o755)).unwrap();
        erased.unwrap();
        assert!(!listed.files.exists());
        assert!(listed.info.exists(), "the info file is left behind");
    }

    #[test]
    fn restore_free_puts_it_back_and_removes_the_info_file() {
        let (t, root) = root();
        let src = t.path().join("x.txt");
        fs::write(&src, "hi").unwrap();
        let put = trash_in(&root, &src).unwrap();

        let landed = restore_free(&trashed(&put.files).unwrap()).unwrap();

        assert_eq!(landed, src);
        assert_eq!(fs::read_to_string(&src).unwrap(), "hi");
        assert!(!put.files.exists() && !put.info.exists());
    }

    #[test]
    fn restore_free_makes_a_parent_folder_that_has_gone() {
        let (t, root) = root();
        let src = t.path().join("gone/deeper/x.txt");
        fs::create_dir_all(src.parent().unwrap()).unwrap();
        fs::write(&src, "hi").unwrap();
        let put = trash_in(&root, &src).unwrap();
        fs::remove_dir_all(t.path().join("gone")).unwrap();

        let landed = restore_free(&put).unwrap();

        assert_eq!(landed, src);
        assert_eq!(fs::read_to_string(&src).unwrap(), "hi");
    }

    #[test]
    fn restore_free_comes_back_numbered_beside_a_taken_name() {
        let (t, root) = root();
        let src = t.path().join("x.txt");
        fs::write(&src, "old").unwrap();
        let put = trash_in(&root, &src).unwrap();
        fs::write(&src, "new").unwrap();
        fs::write(t.path().join("x (2).txt"), "also taken").unwrap();

        let landed = restore_free(&put).unwrap();

        assert_eq!(landed, t.path().join("x (3).txt"));
        assert_eq!(fs::read_to_string(&landed).unwrap(), "old");
        assert_eq!(fs::read_to_string(&src).unwrap(), "new");
        assert!(!put.info.exists());
    }

    #[test]
    fn empty_in_clears_both_directories() {
        let (t, root) = root();
        let a = t.path().join("a.txt");
        let b = t.path().join("b.txt");
        fs::write(&a, "1").unwrap();
        fs::write(&b, "2").unwrap();
        trash_in(&root, &a).unwrap();
        trash_in(&root, &b).unwrap();

        let count = empty_in(&root).unwrap();

        assert_eq!(count, 2);
        assert_eq!(fs::read_dir(root.join("files")).unwrap().count(), 0);
        assert_eq!(fs::read_dir(root.join("info")).unwrap().count(), 0);
    }

    #[test]
    fn empty_in_keeps_going_past_a_failure_and_pairs_each_info_file() {
        let (t, root) = root();
        let locked = t.path().join("locked");
        fs::create_dir_all(locked.join("sub")).unwrap();
        fs::write(locked.join("sub/f.txt"), "1").unwrap();
        for name in ["a.txt", "z.txt"] {
            fs::write(t.path().join(name), "1").unwrap();
            trash_in(&root, &t.path().join(name)).unwrap();
        }
        let locked = trash_in(&root, &locked).unwrap().files.join("sub");
        fs::write(root.join("info/ghost.trashinfo"), "[Trash Info]\n").unwrap();
        fs::write(root.join("directorysizes"), "4096 0 locked\n").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();

        let e = empty_in(&root).unwrap_err();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();

        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
        assert!(
            e.to_string().starts_with("1 item could not be deleted"),
            "{e}"
        );
        let left = |dir: &str| -> Vec<String> {
            let mut v: Vec<_> = fs::read_dir(root.join(dir))
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        };
        assert_eq!(left("files"), ["locked"]);
        assert_eq!(left("info"), ["locked.trashinfo"]);
        assert!(!root.join("directorysizes").exists());
    }

    #[test]
    fn trashinfo_formats_path_and_date() {
        let when = locale::mktime(2026, 9, 15, 12, 34, 56);
        let text = trashinfo(Path::new("/a b/ü.txt"), when);
        assert_eq!(
            text,
            "[Trash Info]\nPath=/a%20b/%C3%BC.txt\nDeletionDate=2026-09-15T12:34:56\n"
        );
    }

    #[test]
    fn percent_encode_escapes_outside_unreserved_and_slash() {
        assert_eq!(percent_encode(Path::new("/a b/ü.txt")), "/a%20b/%C3%BC.txt");
        assert_eq!(
            percent_encode(Path::new("/A-Za-z0-9._~/x")),
            "/A-Za-z0-9._~/x"
        );
    }

    #[test]
    fn percent_decode_keeps_bad_escapes_literally() {
        assert_eq!(percent_decode("x%2Fy%zz%4"), b"x/y%zz%4");
    }

    #[test]
    fn absolutize_prefixes_a_relative_path_with_the_current_dir() {
        let cwd = std::env::current_dir().unwrap();

        let relative = absolutize(Path::new("relative/x.txt")).unwrap();
        assert!(relative.starts_with(&cwd));
        assert_eq!(relative, cwd.join("relative/x.txt"));

        let already_absolute = Path::new("/already/absolute/x.txt");
        assert_eq!(absolutize(already_absolute).unwrap(), already_absolute);
    }

    // EXDEV needs two file systems, so these tests call the cross-device
    // fallback directly.

    #[test]
    fn copy_then_remove_keeps_an_existing_destination_folder() {
        let t = tempfile::tempdir().unwrap();
        let from = t.path().join("from");
        std::fs::create_dir(&from).unwrap();
        std::fs::write(from.join("mine.txt"), "mine").unwrap();
        let to = t.path().join("to");
        std::fs::create_dir(&to).unwrap();
        std::fs::write(to.join("theirs.txt"), "theirs").unwrap();

        let e = copy_then_remove(&from, &to, &AtomicBool::new(false)).unwrap_err();

        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(to.join("theirs.txt")).unwrap(),
            "theirs"
        );
        assert_eq!(
            std::fs::read_to_string(from.join("mine.txt")).unwrap(),
            "mine"
        );
    }

    #[test]
    fn move_path_across_devices_keeps_the_mtime() {
        use std::os::unix::fs::MetadataExt;
        let shm = tempfile::tempdir_in("/dev/shm").unwrap();
        let t = tempfile::tempdir().unwrap();
        let from = shm.path().join("photo.jpg");
        std::fs::write(&from, "jpeg").unwrap();
        let then = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::open(&from)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(then))
            .unwrap();
        let to = t.path().join("photo.jpg");
        let dev = |p: &Path| std::fs::metadata(p).unwrap().dev();

        if dev(shm.path()) == dev(t.path()) {
            eprintln!("/dev/shm and the temp dir share a device: calling copy_then_remove");
            copy_then_remove(&from, &to, &AtomicBool::new(false)).unwrap();
        } else {
            move_path(&from, &to, &AtomicBool::new(false)).unwrap();
        }

        assert!(!from.exists(), "the original is gone");
        assert_eq!(std::fs::metadata(&to).unwrap().modified().unwrap(), then);
    }

    #[test]
    fn copy_then_remove_moves_a_symlink_as_a_link() {
        let t = tempfile::tempdir().unwrap();
        let target = t.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("keep.txt"), "x").unwrap();
        let link = t.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let to = t.path().join("moved");
        copy_then_remove(&link, &to, &AtomicBool::new(false)).unwrap();
        assert!(!link.exists() && std::fs::symlink_metadata(&link).is_err());
        assert!(
            std::fs::symlink_metadata(&to)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_link(&to).unwrap(), target);
        assert!(
            target.join("keep.txt").exists(),
            "the target tree is untouched"
        );
        let f = t.path().join("f.txt");
        std::fs::write(&f, "hi").unwrap();
        copy_then_remove(&f, &t.path().join("g.txt"), &AtomicBool::new(false)).unwrap();
        assert!(!f.exists());
        assert_eq!(
            std::fs::read_to_string(t.path().join("g.txt")).unwrap(),
            "hi"
        );
        copy_then_remove(&target, &t.path().join("dir"), &AtomicBool::new(false)).unwrap();
        assert!(!target.exists());
        assert!(t.path().join("dir/keep.txt").exists());
    }

    #[test]
    fn copy_then_remove_stopped_during_the_flush_keeps_the_original() {
        let t = tempfile::tempdir().unwrap();
        let (file, folder, link) = (t.path().join("a"), t.path().join("b"), t.path().join("c"));
        std::fs::write(&file, "a").unwrap();
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("inner.txt"), "b").unwrap();
        std::os::unix::fs::symlink(&file, &link).unwrap();
        for from in [&file, &folder, &link] {
            let stop = std::sync::Arc::new(AtomicBool::new(false));
            let flag = stop.clone();
            crate::ops::IN_FLUSH.set(Some(Box::new(move || {
                flag.store(true, std::sync::atomic::Ordering::Relaxed)
            })));
            let to = t.path().join("moved");
            let e = copy_then_remove(from, &to, &stop).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::Interrupted);
            assert!(
                std::fs::symlink_metadata(from).is_ok(),
                "{} stays",
                from.display()
            );
            assert!(
                std::fs::symlink_metadata(&to).is_err(),
                "no copy of {} is left",
                from.display()
            );
        }
        assert_eq!(
            std::fs::read_to_string(folder.join("inner.txt")).unwrap(),
            "b"
        );
    }

    #[test]
    fn a_move_stopped_during_the_flush_keeps_a_file_another_job_put_in_the_destination() {
        let t = tempfile::tempdir().unwrap();
        let from = t.path().join("Photos");
        std::fs::create_dir(&from).unwrap();
        std::fs::write(from.join("a.jpg"), "a").unwrap();
        let notes = t.path().join("notes.txt");
        std::fs::write(&notes, "theirs").unwrap();
        let to = t.path().join("usb/Photos");
        std::fs::create_dir(to.parent().unwrap()).unwrap();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let (flag, at, notes_from) = (stop.clone(), to.clone(), notes.clone());
        crate::ops::IN_FLUSH.set(Some(Box::new(move || {
            let _ = std::fs::create_dir(&at);
            std::fs::rename(&notes_from, at.join("notes.txt")).unwrap();
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        })));
        let e = copy_then_remove(&from, &to, &stop).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Interrupted);
        assert_eq!(
            std::fs::read_to_string(to.join("notes.txt")).unwrap(),
            "theirs"
        );
        assert!(
            !to.join("a.jpg").exists(),
            "the stopped copy is not in Photos"
        );
        assert_eq!(std::fs::read_to_string(from.join("a.jpg")).unwrap(), "a");
        let left: Vec<_> = std::fs::read_dir(to.parent().unwrap()).unwrap().collect();
        assert_eq!(left.len(), 1, "only Photos is in usb, no part");
    }

    fn capped() -> (tempfile::TempDir, Bins, PathBuf, u64) {
        let (t, mut bins, drive) = drives();
        bins.policy.global.max_percent = 1;
        let cap = bins.policy.global.cap(bincap::drive_size(&bins.home));
        (t, bins, drive, cap)
    }

    fn sparse(path: &Path, len: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::File::create(path).unwrap().set_len(len).unwrap();
    }

    #[test]
    fn an_item_bigger_than_the_whole_cap_is_refused_and_left() {
        let (t, bins, _drive, cap) = capped();
        let big = t.path().join("home/big.iso");
        sparse(&big, cap + 1);
        let fits = t.path().join("home/fits.iso");
        sparse(&fits, cap);

        assert!(bins.measure(&big).unwrap().too_big());
        assert!(!bins.measure(&fits).unwrap().too_big());
        let e = bins.trash(&big).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::FileTooLarge);
        assert!(big.exists());
        assert!(!bins.home.join("files").exists(), "nothing made or moved");
        bins.trash(&fits).unwrap();
    }

    #[test]
    fn trashing_past_the_cap_erases_the_oldest_items_first() {
        let (t, bins, _drive, cap) = capped();
        let item = |name: &str| {
            let p = t.path().join("home").join(name);
            sparse(&p, cap * 2 / 5);
            bins.trash(&p).unwrap()
        };
        let (a, b) = (item("a").0, item("b").0);
        for (put, date) in [(&a, "2026-02-01T00:00:00"), (&b, "2026-01-01T00:00:00")] {
            let body = format!("[Trash Info]\nPath=/x\nDeletionDate={date}\n");
            fs::write(&put.info, body).unwrap();
        }
        let (c, purged) = item("c");

        assert_eq!(
            purged,
            std::slice::from_ref(&b.files),
            "the purge names what it erased"
        );
        assert!(!b.files.exists() && !b.info.exists(), "the oldest went");
        assert!(a.files.exists() && c.files.exists());
        assert_eq!(bincap::used(&bins.home), 2 * (cap * 2 / 5));
    }

    #[test]
    fn trashing_reuses_the_measure_while_the_folder_is_unchanged() {
        let (t, bins, _drive) = drives();
        let dir = t.path().join("home/Photos");
        fs::create_dir_all(dir.join("2026")).unwrap();
        fs::write(dir.join("2026/a.jpg"), [0u8; 300]).unwrap();
        let measured = bins.measure(&dir).unwrap();
        assert_eq!(measured.size, 300);
        // Deeper than the folder's own entries: the stamp does not see it.
        fs::write(dir.join("2026/b.jpg"), [0u8; 200]).unwrap();
        bins.trash_measured(&dir, Some(measured)).unwrap();
        assert_eq!(bincap::used(&bins.home), 300, "the measured size is kept");
    }

    #[test]
    fn a_folder_that_gains_an_entry_after_its_measure_is_measured_again() {
        let (t, bins, _drive, cap) = capped();
        let dir = t.path().join("home/Downloads");
        sparse(&dir.join("part.bin"), cap / 2);
        let measured = bins.measure(&dir).unwrap();
        assert!(!measured.too_big());
        std::thread::sleep(std::time::Duration::from_millis(50));
        sparse(&dir.join("more.bin"), cap);

        let e = bins.trash_measured(&dir, Some(measured)).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::FileTooLarge);
        assert!(dir.exists());
    }

    #[test]
    fn a_trashed_folder_gets_its_line_in_directorysizes() {
        let (t, bins, _drive) = drives();
        let dir = t.path().join("home/Photos");
        fs::create_dir_all(dir.join("2026")).unwrap();
        fs::write(dir.join("2026/a.jpg"), [0u8; 300]).unwrap();
        fs::write(dir.join("b.jpg"), [0u8; 200]).unwrap();
        bins.trash(&dir).unwrap();
        let sizes = fs::read_to_string(bins.home.join("directorysizes")).unwrap();
        let fields: Vec<&str> = sizes.trim_end().split(' ').collect();
        assert_eq!((fields[0], fields[2]), ("500", "Photos"), "{sizes}");
        assert_eq!(bincap::used(&bins.home), 500);
    }

    #[test]
    fn a_folder_restored_or_erased_drops_its_line_in_directorysizes() {
        let (t, bins, _drive) = drives();
        let sizes = || fs::read_to_string(bins.home.join("directorysizes")).unwrap();
        let (a, b) = (t.path().join("home/A"), t.path().join("home/B"));
        for d in [&a, &b] {
            fs::create_dir_all(d).unwrap();
            fs::write(d.join("f"), [0u8; 100]).unwrap();
        }
        let (ta, _) = bins.trash(&a).unwrap();
        let (tb, _) = bins.trash(&b).unwrap();
        assert_eq!(sizes().lines().count(), 2, "{}", sizes());
        restore_free(&ta).unwrap();
        assert!(
            sizes().ends_with(" B\n"),
            "only B's line is left: {}",
            sizes()
        );
        erase_trashed(&tb.files, Some(&tb)).unwrap();
        assert_eq!(sizes(), "", "no line is left");
    }

    #[test]
    fn a_drive_set_to_remove_immediately_keeps_nothing_in_its_bin() {
        let (t, mut bins, drive) = drives();
        bins.policy.independent = true;
        let key = bins.drive_key(&drive);
        assert_eq!(key, drive.to_string_lossy(), "no device: the mount point");
        assert_eq!(bins.drive_key(t.path()), bincap::HOME);
        bins.policy.drives.insert(
            key,
            Rule {
                remove_immediately: true,
                max_percent: 10,
            },
        );
        let on_drive = drive.join("x.txt");
        fs::write(&on_drive, "x").unwrap();
        let at_home = t.path().join("home/y.txt");
        sparse(&at_home, 1);

        assert_eq!(bins.fate(&on_drive), Fate::RemoveImmediately);
        assert_eq!(bins.fate(&at_home), Fate::Recycle);
        assert_eq!(bins.measure(&on_drive), None);
        let e = bins.trash(&on_drive).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Unsupported);
        assert!(on_drive.exists());
        bins.trash(&at_home).unwrap();

        bins.policy.independent = false;
        bins.policy.global.remove_immediately = true;
        assert_eq!(bins.fate(&at_home), Fate::RemoveImmediately);
    }

    #[test]
    fn from_table_keeps_each_mounts_device() {
        let table = b"/dev/reprise-none /run/media/u/STICK vfat rw 0 0\n";
        let bins = Bins::from_table(PathBuf::from("/h/Trash"), table);
        assert_eq!(
            bins.devices[Path::new("/run/media/u/STICK")],
            "/dev/reprise-none"
        );
        assert_eq!(
            bins.drive_key(Path::new("/run/media/u/STICK")),
            "/run/media/u/STICK"
        );
    }
}
