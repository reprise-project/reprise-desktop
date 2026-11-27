//! Clean Up: what the Recycle Bin, the thumbnail cache, the user's cache folder, crash dumps and
//! unused Flatpak runtimes take, and removing them.
//!
//! A folder part is emptied only from inside itself. The cache folder must be an absolute path,
//! is opened without following a link and must be the user's own; the thumbnail cache is opened
//! the same way inside it. Either is refused without a home folder, or when it is the home folder
//! or above it, a drive's root other than `~/.cache` itself, or holds the config, data or state
//! folder or one of the user's folders (Documents, Pictures...), as written or where its links
//! lead. Under them no link is followed and no drive mounted inside is entered.
//!
//! The Recycle Bin empties through [`Bins::empty`]. Flatpak decides which runtimes are unused;
//! a removal asks it again and removes only the confirmed runtimes it still calls unused. Crash
//! dumps are only counted: the system owns their files.

use crate::ops::at::{c_path, cwd, open_dir, stat_fd};
use crate::ops::{entries_at, erase_all_at};
use crate::subprocess;
use crate::trash::Bins;
use std::collections::HashSet;
use std::ffi::{CStr, OsStr};
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    RecycleBin,
    Thumbnails,
    Caches,
    Coredumps,
    Flatpak,
}

impl Part {
    pub const ALL: [Part; 5] = [
        Part::RecycleBin,
        Part::Thumbnails,
        Part::Caches,
        Part::Coredumps,
        Part::Flatpak,
    ];

    /// Whether its box is checked when Clean Up opens.
    pub fn checked_first(self) -> bool {
        matches!(self, Part::RecycleBin | Part::Thumbnails)
    }
}

/// What a part holds now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// Bytes on disk.
    pub size: u64,
    /// Whether [`Places::clean`] can remove it: false for crash dumps, and for a folder that is
    /// refused (see the module header).
    pub removable: bool,
    /// The folder a folder part empties.
    pub folder: Option<PathBuf>,
    /// The runtimes Flatpak calls unused.
    pub items: Vec<Runtime>,
}

impl Found {
    fn nothing(removable: bool) -> Found {
        Found {
            size: 0,
            removable,
            folder: None,
            items: Vec::new(),
        }
    }
}

/// One runtime Flatpak calls unused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Runtime {
    pub id: String,
    pub arch: String,
    pub branch: String,
    /// In the system installation, not the user's.
    pub system: bool,
}

impl Runtime {
    /// Flatpak's full ref, `runtime/ID/ARCH/BRANCH`.
    pub fn full_ref(&self) -> String {
        format!("runtime/{}/{}/{}", self.id, self.arch, self.branch)
    }
}

/// The thumbnail cache's folder in the cache folder ([`crate::thumbnails::Thumbnails::default_dir`]).
const THUMBNAILS: &CStr = c"thumbnails";

/// Flatpak's installations, by the option that picks each.
const INSTALLATIONS: [(&str, bool); 2] = [("--user", false), ("--system", true)];

const LIST_DEADLINE: Duration = Duration::from_secs(60);
/// Long enough for a password asked for the system installation.
const REMOVE_DEADLINE: Duration = Duration::from_secs(15 * 60);

/// The user's folders a cache folder must not be or hold, by `user-dirs.dirs` key and fallback.
const USER_DIRS: [(&str, &str); 8] = [
    ("DESKTOP", "Desktop"),
    ("DOCUMENTS", "Documents"),
    ("DOWNLOAD", "Downloads"),
    ("MUSIC", "Music"),
    ("PICTURES", "Pictures"),
    ("PUBLICSHARE", "Public"),
    ("TEMPLATES", "Templates"),
    ("VIDEOS", "Videos"),
];

/// Where the parts are on this system; a part with no place is not offered.
#[derive(Clone, Debug)]
pub struct Places {
    /// `$XDG_CACHE_HOME`.
    pub cache: Option<PathBuf>,
    pub bins: Option<Bins>,
    pub flatpak: Option<PathBuf>,
    /// Flatpak's user installation, asked about only while it exists: `flatpak --user` creates
    /// one where it is missing.
    pub flatpak_user: Option<PathBuf>,
    pub coredumpctl: Option<PathBuf>,
    pub uid: u32,
    /// `$HOME`: a cache folder there or above it is refused.
    pub home: Option<PathBuf>,
    /// Folders a cache folder must not be or hold.
    pub kept: Vec<PathBuf>,
}

impl Places {
    pub fn system() -> Places {
        let program = crate::autostart::find_program;
        let kept = [
            reprisexdg::config_home(),
            reprisexdg::data_home(),
            reprisexdg::state_home(),
        ];
        let user_dirs = USER_DIRS.map(|(key, fallback)| crate::places::user_dir(key, fallback));
        Places {
            cache: reprisexdg::cache_home(),
            bins: Bins::system(),
            flatpak: program("flatpak"),
            flatpak_user: std::env::var_os("FLATPAK_USER_DIR")
                .map(PathBuf::from)
                .or_else(|| reprisexdg::data_home().map(|d| d.join("flatpak"))),
            coredumpctl: program("coredumpctl"),
            // SAFETY: getuid cannot fail.
            uid: unsafe { libc::getuid() },
            home: std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|h| h.is_absolute()),
            kept: kept.into_iter().chain(user_dirs).flatten().collect(),
        }
    }

    /// The parts this system has, in [`Part::ALL`] order.
    pub fn parts(&self) -> Vec<Part> {
        let has = |p: Part| match p {
            Part::RecycleBin => self.bins.is_some(),
            Part::Thumbnails | Part::Caches => self.cache.is_some(),
            Part::Coredumps => self.coredumpctl.is_some(),
            Part::Flatpak => self.flatpak.is_some(),
        };
        Part::ALL.into_iter().filter(|p| has(*p)).collect()
    }

    /// The installations to list, by the option that picks each: the user's only while it exists.
    fn installations(&self) -> impl Iterator<Item = (&'static str, bool)> {
        let user = self.flatpak_user.as_deref().is_some_and(Path::exists);
        INSTALLATIONS
            .into_iter()
            .filter(move |&(_, system)| system || user)
    }

    /// What `part` holds; a part this system lacks holds nothing. A long count gives up, with
    /// what it has, once `stop` is set.
    pub fn measure(&self, part: Part, stop: &AtomicBool) -> Found {
        match part {
            Part::RecycleBin => match &self.bins {
                Some(bins) => Found {
                    size: bins.roots().iter().map(|r| crate::bincap::used(r)).sum(),
                    ..Found::nothing(true)
                },
                None => Found::nothing(false),
            },
            Part::Thumbnails => self.measure_folder(true, stop),
            Part::Caches => self.measure_folder(false, stop),
            Part::Coredumps => Found {
                size: (self.coredumpctl.as_deref()).map_or(0, |p| coredumps(p, self.uid)),
                ..Found::nothing(false)
            },
            Part::Flatpak => match &self.flatpak {
                Some(p) => unused_runtimes(p, self.installations(), stop),
                None => Found::nothing(false),
            },
        }
    }

    /// Removes what `part` holds; for Flatpak only `confirmed`'s runtimes. One item that fails
    /// does not stop the rest; the error is the first failure.
    pub fn clean(&self, part: Part, confirmed: &Found) -> io::Result<()> {
        match part {
            Part::RecycleBin => match &self.bins {
                Some(bins) => bins.empty().map(|_| ()),
                None => Ok(()),
            },
            Part::Thumbnails => {
                let (fd, path) = self.thumbnails()?;
                empty_folder(fd.as_fd(), &path, None)
            }
            Part::Caches => {
                let (fd, path) = self.cache()?;
                empty_folder(fd.as_fd(), &path, Some(THUMBNAILS))
            }
            Part::Coredumps => Err(io::ErrorKind::PermissionDenied.into()),
            Part::Flatpak => match &self.flatpak {
                Some(p) => remove_confirmed(p, &confirmed.items),
                None => Ok(()),
            },
        }
    }

    fn cache(&self) -> io::Result<(OwnedFd, PathBuf)> {
        let path = self.cache.clone().ok_or(io::ErrorKind::NotFound)?;
        if !path.is_absolute() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let fd = own_folder(self.uid, cwd(), &c_path(&path)?)?;
        self.refuse_kept(fd.as_fd(), &path)?;
        Ok((fd, path))
    }

    fn thumbnails(&self) -> io::Result<(OwnedFd, PathBuf)> {
        let (cache, path) = self.cache()?;
        let fd = own_folder(self.uid, cache.as_fd(), THUMBNAILS)?;
        let path = path.join(OsStr::from_bytes(THUMBNAILS.to_bytes()));
        self.refuse_kept(fd.as_fd(), &path)?;
        Ok((fd, path))
    }

    /// Refuses the folder open as `fd` (at `path`) when there is no `$HOME`, or it is `$HOME` or
    /// above it, a drive's root other than `$HOME/.cache` itself, or one of [`Places::kept`] or
    /// above one (as written or with its links resolved), comparing device and inode.
    fn refuse_kept(&self, fd: BorrowedFd, path: &Path) -> io::Result<()> {
        let refused = Err(io::ErrorKind::PermissionDenied.into());
        let Some(home) = &self.home else {
            return refused;
        };
        let st = stat_fd(fd)?;
        let key = (st.st_dev, st.st_ino);
        let key_of = |p: &Path| std::fs::metadata(p).ok().map(|m| (m.dev(), m.ino()));
        let is_it = |p: &Path| key_of(p) == Some(key);
        let above = |p: &PathBuf| {
            p.ancestors().any(is_it) || p.canonicalize().is_ok_and(|c| c.ancestors().any(is_it))
        };
        let up = stat_fd(open_dir(fd, c"..", false)?.as_fd())?;
        let mounted = crate::ops::refuse_mount_point(path).is_err();
        let default_cache = std::fs::symlink_metadata(home.join(".cache"))
            .ok()
            .map(|m| (m.dev(), m.ino()));
        let up = (up.st_dev, up.st_ino);
        if foreign_root(key, up, mounted, default_cache)
            || above(home)
            || self.kept.iter().any(above)
        {
            return refused;
        }
        Ok(())
    }

    fn measure_folder(&self, thumbnails: bool, stop: &AtomicBool) -> Found {
        let opened = match thumbnails {
            true => self.thumbnails(),
            false => self.cache(),
        };
        match opened {
            Ok((_, path)) => {
                let skip = (!thumbnails).then(|| OsStr::from_bytes(THUMBNAILS.to_bytes()));
                Found {
                    size: contents_size(&path, skip, stop),
                    folder: Some(path),
                    ..Found::nothing(true)
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Found::nothing(true),
            Err(e) => {
                let named = self.cache.as_ref().map(|c| match thumbnails {
                    true => c.join(OsStr::from_bytes(THUMBNAILS.to_bytes())),
                    false => c.clone(),
                });
                log::warn!("clean up: {named:?} is left alone: {e}");
                Found::nothing(false)
            }
        }
    }
}

/// Whether a folder with device and inode `at`, whose `..` is `up`, is a drive's root Clean Up
/// refuses: any but `default_cache` (`$HOME/.cache`), which may be a subvolume or tmpfs of its own.
fn foreign_root(
    at: (u64, u64),
    up: (u64, u64),
    mounted: bool,
    default_cache: Option<(u64, u64)>,
) -> bool {
    let root = mounted || up.0 != at.0 || up.1 == at.1;
    root && default_cache != Some(at)
}

/// Folder `name` in `at`, opened without following a link; one that is not `uid`'s is refused.
fn own_folder(uid: u32, at: BorrowedFd, name: &CStr) -> io::Result<OwnedFd> {
    let fd = open_dir(at, name, false)?;
    if stat_fd(fd.as_fd())?.st_uid != uid {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(fd)
}

/// The bytes on disk under folder `root`, `root`'s own entry and its entry `skip` left out, a
/// file with several links counted once. Links are not followed and drives mounted inside are
/// not counted. Stops with what it has once `stop` is set.
fn contents_size(root: &Path, skip: Option<&OsStr>, stop: &AtomicBool) -> u64 {
    let Ok(top) = std::fs::symlink_metadata(root) else {
        return 0;
    };
    let mut total = 0u64;
    let mut linked = HashSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            if dir == root && Some(e.file_name().as_os_str()) == skip {
                continue;
            }
            let Ok(m) = e.metadata() else {
                continue;
            };
            if m.dev() != top.dev() || (m.nlink() > 1 && !m.is_dir() && !linked.insert(m.ino())) {
                continue;
            }
            total = total.saturating_add(m.blocks() * 512);
            if m.is_dir() {
                stack.push(e.path());
            }
        }
    }
    total
}

/// Removes every entry of the folder open as `dir` (at `path`) but `keep`, through
/// [`erase_all_at`]: links go as links, mounted drives are refused, and an entry that fails at
/// any depth keeps only itself and the folders above it. Each failure is logged; the error is
/// the first.
fn empty_folder(dir: BorrowedFd, path: &Path, keep: Option<&CStr>) -> io::Result<()> {
    let mut failed = Vec::new();
    for (name, _) in entries_at(dir)? {
        if Some(name.as_c_str()) == keep {
            continue;
        }
        let at = path.join(OsStr::from_bytes(name.to_bytes()));
        let start = failed.len();
        if let Err(e) = erase_all_at(dir, &name, &at, &mut failed)
            && e.kind() != io::ErrorKind::NotFound
        {
            failed.push(e);
        }
        for e in &failed[start..] {
            log::warn!("clean up: {} stays: {e}", at.display());
        }
    }
    failed.into_iter().next().map_or(Ok(()), Err)
}

/// `program ARGS` with nothing on its standard input, so a question it asks is answered no; in
/// the C locale when its output is read.
fn run(program: &Path, args: &[&str], within: Duration, read: bool) -> io::Result<subprocess::Ran> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if read {
        cmd.env("LC_ALL", "C");
    }
    subprocess::run(&mut cmd, within).map_err(|e| io::Error::other(e.to_string()))
}

/// The bytes the crash dumps of `uid` that are still on disk take, from `coredumpctl`.
fn coredumps(coredumpctl: &Path, uid: u32) -> u64 {
    let args = ["list", "--json=short", "--no-pager", "--quiet"];
    match run(coredumpctl, &args, LIST_DEADLINE, true) {
        Ok(ran) => dumps_size(&ran.stdout, uid),
        Err(e) => {
            log::warn!("clean up: coredumpctl: {e}");
            0
        }
    }
}

#[derive(serde::Deserialize)]
struct Dump {
    uid: Option<u32>,
    corefile: Option<String>,
    size: Option<u64>,
}

/// The sizes `coredumpctl list --json=short` gives the dumps of `uid` it still has, summed.
fn dumps_size(json: &str, uid: u32) -> u64 {
    let dumps: Vec<Dump> = serde_json::from_str(json.trim()).unwrap_or_default();
    (dumps.iter())
        .filter(|d| d.uid == Some(uid) && d.corefile.as_deref() == Some("present"))
        .filter_map(|d| d.size)
        .sum()
}

/// The rows of `flatpak uninstall`'s table of changes: ` 1.     ID [ARCH] BRANCH r`, the arch
/// column shown only when a row's differs from `arch`, the machine's. A row whose operation is
/// not a removal is not one.
fn unused_rows(table: &str, arch: &str, system: bool) -> Vec<Runtime> {
    table
        .lines()
        .filter_map(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            let (num, rest) = words.split_first()?;
            let n = num.strip_suffix('.')?;
            if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let (id, arch, branch) = match rest {
                [id, branch, "r"] => (id, arch, branch),
                [id, arch, branch, "r"] => (id, *arch, branch),
                _ => return None,
            };
            Some(Runtime {
                id: id.to_string(),
                arch: arch.to_string(),
                branch: branch.to_string(),
                system,
            })
        })
        .collect()
}

/// What `flatpak uninstall --unused` would remove from installation `option`. It is asked with
/// nothing on its input, so it lists the changes and declines them.
fn unused_in(flatpak: &Path, option: &str, system: bool) -> io::Result<Vec<Runtime>> {
    let arch = run(flatpak, &["--default-arch"], LIST_DEADLINE, true)?;
    let arch = arch.stdout.trim();
    if !arch.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') || arch.is_empty() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let args = ["uninstall", "--unused", option];
    let table = run(flatpak, &args, LIST_DEADLINE, true)?.stdout;
    Ok(unused_rows(&table, arch, system))
}

/// Each installation's unused runtimes, with the sizes `flatpak info` gives.
fn unused_runtimes(
    flatpak: &Path,
    installations: impl Iterator<Item = (&'static str, bool)>,
    stop: &AtomicBool,
) -> Found {
    let mut found = Found::nothing(true);
    for (option, system) in installations {
        let rows = match unused_in(flatpak, option, system) {
            Ok(rows) => rows,
            Err(e) => {
                log::warn!("clean up: flatpak uninstall --unused {option}: {e}");
                continue;
            }
        };
        for r in rows {
            if stop.load(Ordering::Relaxed) {
                return found;
            }
            let arch = format!("--arch={}", r.arch);
            let args = ["info", "--show-size", option, &arch, &r.id, &r.branch];
            let size = run(flatpak, &args, LIST_DEADLINE, true)
                .ok()
                .filter(|ran| ran.status.success())
                .and_then(|ran| ran.stdout.trim().parse::<u64>().ok());
            found.size += size.unwrap_or(0);
            found.items.push(r);
        }
    }
    found
}

/// Removes, per installation, the runtimes of `confirmed` Flatpak still calls unused, by full
/// ref. An installation left with none is not asked to remove anything.
fn remove_confirmed(flatpak: &Path, confirmed: &[Runtime]) -> io::Result<()> {
    let mut first = None;
    for (option, system) in INSTALLATIONS {
        if !confirmed.iter().any(|r| r.system == system) {
            continue;
        }
        let still = match unused_in(flatpak, option, system) {
            Ok(rows) => rows,
            Err(e) => {
                first.get_or_insert(e);
                continue;
            }
        };
        let refs: Vec<String> = (still.iter())
            .filter(|r| confirmed.contains(r))
            .map(Runtime::full_ref)
            .collect();
        if refs.is_empty() {
            continue;
        }
        // `-y` rather than `--noninteractive`, which would also forbid the password prompt the
        // system installation needs.
        let mut args = vec!["uninstall", "-y", "--no-related", option];
        args.extend(refs.iter().map(String::as_str));
        let failed = match run(flatpak, &args, REMOVE_DEADLINE, false) {
            Ok(ran) if ran.status.success() => continue,
            Ok(ran) => {
                let last = ran
                    .stderr
                    .lines()
                    .rev()
                    .map(str::trim)
                    .find(|l| !l.is_empty());
                io::Error::other(last.map_or_else(|| ran.status.to_string(), str::to_string))
            }
            Err(e) => e,
        };
        log::warn!("clean up: flatpak uninstall {option}: {failed}");
        first.get_or_insert(failed);
    }
    first.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests;
