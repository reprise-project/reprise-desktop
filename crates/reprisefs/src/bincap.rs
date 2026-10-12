//! The Recycle Bin's settings (`[recycle_bin]` in `config.toml`): per-drive trash or
//! remove-immediately, and the trash size cap.

use std::collections::{BTreeMap, HashMap};
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, TableLike, value};

pub const DEFAULT_PERCENT: u8 = 10;

pub const HOME: &str = "home";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    pub remove_immediately: bool,
    pub max_percent: u8,
}

impl Default for Rule {
    fn default() -> Rule {
        Rule {
            remove_immediately: false,
            max_percent: DEFAULT_PERCENT,
        }
    }
}

impl Rule {
    pub fn cap(self, size: u64) -> u64 {
        (u128::from(size) * u128::from(self.max_percent.min(100)) / 100) as u64
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub confirm: bool,
    /// "Configure drives independently": each drive follows its entry in
    /// `drives` (the global rule where it has none) rather than `global`.
    pub independent: bool,
    pub global: Rule,
    /// Each drive's own rule, by [`drive_key`]; [`HOME`] for the home
    /// trash's drive. Kept while `independent` is off, so turning it on
    /// again brings them back.
    pub drives: BTreeMap<String, Rule>,
}

impl Default for Policy {
    fn default() -> Policy {
        Policy {
            confirm: true,
            independent: false,
            global: Rule::default(),
            drives: BTreeMap::new(),
        }
    }
}

impl Policy {
    pub fn rule(&self, key: &str) -> Rule {
        match self.independent {
            true => self.drives.get(key).copied().unwrap_or(self.global),
            false => self.global,
        }
    }

    pub fn load() -> Policy {
        Policy::parse(&std::fs::read_to_string(config_path()).unwrap_or_default())
    }

    pub fn parse(text: &str) -> Policy {
        let mut p = Policy::default();
        let Ok(doc) = text.parse::<DocumentMut>() else {
            return p;
        };
        let Some(t) = doc.get("recycle_bin").and_then(Item::as_table_like) else {
            return p;
        };
        let flag = |t: &dyn TableLike, k: &str| t.get(k).and_then(Item::as_bool);
        p.confirm = flag(t, "confirm").unwrap_or(true);
        p.independent = flag(t, "independent").unwrap_or(false);
        let rule = |t: &dyn TableLike, base: Rule| Rule {
            remove_immediately: flag(t, "remove_immediately").unwrap_or(base.remove_immediately),
            max_percent: t
                .get("max_percent")
                .and_then(Item::as_integer)
                .map_or(base.max_percent, |v| v.clamp(0, 100) as u8),
        };
        p.global = rule(t, Rule::default());
        if let Some(drives) = t.get("drives").and_then(Item::as_table_like) {
            for (key, d) in drives.iter() {
                if let Some(d) = d.as_table_like() {
                    p.drives.insert(key.to_string(), rule(d, p.global));
                }
            }
        }
        p
    }

    pub fn write(&self, text: &str) -> Result<String, String> {
        let mut doc = text.parse::<DocumentMut>().map_err(|e| e.to_string())?;
        let t = doc
            .entry("recycle_bin")
            .or_insert_with(toml_edit::table)
            .as_table_mut()
            .ok_or("recycle_bin is not a table")?;
        let set_rule = |t: &mut Table, r: Rule| {
            t["remove_immediately"] = value(r.remove_immediately);
            t["max_percent"] = value(i64::from(r.max_percent));
        };
        t["confirm"] = value(self.confirm);
        t["independent"] = value(self.independent);
        set_rule(t, self.global);
        if self.drives.is_empty() {
            t.remove("drives");
        } else {
            let mut drives = Table::new();
            drives.set_implicit(true);
            for (key, r) in &self.drives {
                let mut d = Table::new();
                set_rule(&mut d, *r);
                drives.insert(key, Item::Table(d));
            }
            t.insert("drives", Item::Table(drives));
        }
        Ok(doc.to_string())
    }
}

static CONFIG_PATH: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

#[doc(hidden)]
pub fn set_config_path(path: Option<PathBuf>) {
    *CONFIG_PATH.lock().unwrap_or_else(|e| e.into_inner()) = path;
}

pub fn config_path() -> PathBuf {
    if let Some(path) = CONFIG_PATH
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
    {
        return path;
    }
    crate::autostart::config_home()
        .join("reprise")
        .join("config.toml")
}

pub fn drive_key(device: &str, top: &Path) -> String {
    uuid_in(Path::new("/dev/disk/by-uuid"), device)
        .unwrap_or_else(|| top.to_string_lossy().into_owned())
}

fn uuid_in(dir: &Path, device: &str) -> Option<String> {
    if !device.starts_with('/') {
        return None;
    }
    let dev = std::fs::canonicalize(device).ok()?;
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|e| std::fs::canonicalize(e.path()).is_ok_and(|p| p == dev))
        .map(|e| e.file_name().to_string_lossy().into_owned())
}

/// The bytes `path` takes: a file's or a link's length, a folder's the
/// sum over everything in it, links not followed and other drives
/// mounted inside left out. Counting stops once it passes `limit`, so a
/// caller asking only whether something fits never walks all of a huge
/// folder. Entries below `path` that cannot be read count as nothing.
pub fn item_size(path: &Path, limit: u64) -> io::Result<u64> {
    use std::os::unix::fs::MetadataExt;
    let top = std::fs::symlink_metadata(path)?;
    if !top.is_dir() {
        return Ok(top.len());
    }
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(m) = e.metadata() else {
                continue;
            };
            if m.dev() != top.dev() {
                continue;
            }
            if m.is_dir() {
                stack.push(e.path());
            } else {
                total = total.saturating_add(m.len());
                if total > limit {
                    return Ok(total);
                }
            }
        }
    }
    Ok(total)
}

pub fn drive_size(root: &Path) -> u64 {
    root.ancestors()
        .find(|a| a.exists())
        .and_then(crate::places::statvfs_totals)
        .map_or(0, |(total, _)| total)
}

type DirSizes = BTreeMap<Vec<u8>, (u64, i64)>;

fn read_dir_sizes(root: &Path) -> DirSizes {
    let text = std::fs::read(root.join("directorysizes")).unwrap_or_default();
    text.split(|&b| b == b'\n')
        .filter_map(|line| {
            let mut f = line.splitn(3, |&b| b == b' ');
            let size = std::str::from_utf8(f.next()?).ok()?.parse().ok()?;
            let mtime = std::str::from_utf8(f.next()?).ok()?.parse().ok()?;
            let name = crate::trash::percent_decode(std::str::from_utf8(f.next()?).ok()?);
            Some((name, (size, mtime)))
        })
        .collect()
}

fn write_dir_sizes(root: &Path, sizes: &DirSizes) {
    let mut text = String::new();
    for (name, (size, mtime)) in sizes {
        let name = crate::trash::percent_encode(Path::new(OsStr::from_bytes(name)));
        text.push_str(&format!("{size} {mtime} {name}\n"));
    }
    if let Err(e) = crate::ops::write_atomic(&root.join("directorysizes"), text.as_bytes()) {
        log::warn!(
            "trash: could not write {}/directorysizes: {e}",
            root.display()
        );
    }
}

fn info_mtime(root: &Path, name: &OsStr) -> i64 {
    use std::os::unix::fs::MetadataExt;
    let mut info = name.to_os_string();
    info.push(".trashinfo");
    std::fs::metadata(root.join("info").join(info)).map_or(0, |m| m.mtime())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Held {
    name: OsString,
    size: u64,
}

fn held(root: &Path) -> Vec<Held> {
    let cached = read_dir_sizes(root);
    let mut kept = DirSizes::new();
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(root.join("files")) else {
        return out;
    };
    for e in rd.flatten() {
        let Ok(m) = std::fs::symlink_metadata(e.path()) else {
            continue;
        };
        let name = e.file_name();
        let size = if m.is_dir() {
            let mtime = info_mtime(root, &name);
            let key = name.as_bytes().to_vec();
            let size = match cached.get(&key) {
                Some(&(size, t)) if t == mtime => size,
                _ => item_size(&e.path(), u64::MAX).unwrap_or(0),
            };
            kept.insert(key, (size, mtime));
            size
        } else {
            m.len()
        };
        out.push(Held { name, size });
    }
    if kept != cached {
        write_dir_sizes(root, &kept);
    }
    out
}

type Stamp = (u64, u64, i64, i64);

fn stamp(root: &Path) -> Option<Stamp> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(root.join("files")).ok()?;
    Some((m.dev(), m.ino(), m.mtime(), m.mtime_nsec()))
}

static TOTALS: std::sync::Mutex<Option<HashMap<PathBuf, (Stamp, u64)>>> =
    std::sync::Mutex::new(None);

pub fn used(root: &Path) -> u64 {
    let Some(now) = stamp(root) else {
        return 0;
    };
    let mut totals = TOTALS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&(was, total)) = totals.get_or_insert_default().get(root)
        && was == now
    {
        return total;
    }
    drop(totals);
    let total = held(root).iter().map(|h| h.size).sum();
    remember(root, total);
    total
}

fn remember(root: &Path, total: u64) {
    let Some(now) = stamp(root) else {
        return;
    };
    TOTALS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_default()
        .insert(root.to_path_buf(), (now, total));
}

pub(crate) fn took(root: &Path, t: &crate::trash::Trashed, size: u64, before: u64) {
    if std::fs::symlink_metadata(&t.files).is_ok_and(|m| m.is_dir())
        && let Some(name) = t.files.file_name()
    {
        let mut sizes = read_dir_sizes(root);
        sizes.insert(name.as_bytes().to_vec(), (size, info_mtime(root, name)));
        write_dir_sizes(root, &sizes);
    }
    remember(root, before.saturating_add(size));
}

/// Erase the oldest items of the trash at `root` (by their `.trashinfo`
/// DeletionDate, an item with none counting as oldest, then by name)
/// until `incoming` more bytes fit under `cap`, given it holds `used`.
/// Each goes with its `.trashinfo`. Returns what the trash holds after,
/// and the `files/` path of each item erased. An item that cannot be
/// erased stops the purge with its error.
pub(crate) fn make_room(
    root: &Path,
    used: u64,
    incoming: u64,
    cap: u64,
) -> io::Result<(u64, Vec<PathBuf>)> {
    if used.saturating_add(incoming) <= cap {
        return Ok((used, Vec::new()));
    }
    let mut items: Vec<(i64, Held)> = held(root)
        .into_iter()
        .map(|h| {
            let mut info = h.name.clone();
            info.push(".trashinfo");
            let when = std::fs::read_to_string(root.join("info").join(info))
                .ok()
                .and_then(|body| crate::trash::deletion_date(&body))
                .unwrap_or(i64::MIN);
            (when, h)
        })
        .collect();
    items.sort_by(|(a, x), (b, y)| a.cmp(b).then_with(|| x.name.cmp(&y.name)));
    let mut used: u64 = items.iter().map(|(_, h)| h.size).sum();
    let mut purged = Vec::new();
    for (_, h) in items {
        if used.saturating_add(incoming) <= cap {
            break;
        }
        let files = root.join("files").join(&h.name);
        crate::ops::erase(&files)?;
        let mut info = h.name.clone();
        info.push(".trashinfo");
        if let Err(e) = std::fs::remove_file(root.join("info").join(info))
            && e.kind() != io::ErrorKind::NotFound
        {
            log::warn!("trash: left the record of {} behind: {e}", files.display());
        }
        log::info!("trash: made room by erasing {}", files.display());
        used -= h.size;
        purged.push(files);
    }
    Ok((used, purged))
}

pub fn too_big_text(name: &str) -> String {
    format!("'{name}' is too big for the Recycle Bin. Do you want to delete it permanently?")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TooBigStep<'a> {
    Ask(&'a Path),
    /// Every question is answered: each item kept in the batch, with
    /// whether it is erased (`true`) or trashed. An item answered No is
    /// left out, and stays where it is.
    Done(Vec<(PathBuf, bool)>),
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TooBig {
    paths: Vec<PathBuf>,
    big: Vec<bool>,
    answers: Vec<Option<bool>>,
    cancelled: bool,
}

impl TooBig {
    pub fn new(paths: Vec<PathBuf>, big: Vec<bool>) -> TooBig {
        let answers = vec![None; paths.len()];
        TooBig {
            paths,
            big,
            answers,
            cancelled: false,
        }
    }

    pub fn many(&self) -> bool {
        self.paths.len() > 1
    }

    fn next(&self) -> Option<usize> {
        (0..self.paths.len()).find(|&i| self.big[i] && self.answers[i].is_none())
    }

    pub fn step(&self) -> TooBigStep<'_> {
        if self.cancelled {
            return TooBigStep::Cancelled;
        }
        if let Some(i) = self.next() {
            return TooBigStep::Ask(&self.paths[i]);
        }
        TooBigStep::Done(
            self.paths
                .iter()
                .enumerate()
                .filter_map(|(i, p)| match (self.big[i], self.answers[i]) {
                    (false, _) => Some((p.clone(), false)),
                    (true, Some(true)) => Some((p.clone(), true)),
                    (true, _) => None,
                })
                .collect(),
        )
    }

    pub fn answer(&mut self, a: crate::replace::Answer) {
        use crate::replace::Answer;
        let Some(i) = self.next().filter(|_| !self.cancelled) else {
            return;
        };
        match a {
            Answer::Cancel => self.cancelled = true,
            Answer::No => self.answers[i] = Some(false),
            Answer::Yes => self.answers[i] = Some(true),
            Answer::YesToAll => {
                for j in i..self.paths.len() {
                    if self.big[j] {
                        self.answers[j] = Some(true);
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fate {
    Recycle,
    NoBin,
    RemoveImmediately,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeleteMode {
    pub erase: bool,
    pub ask: bool,
    pub no_bin: bool,
}

/// How Delete (or Shift+Delete, `shift`) treats items whose fates are
/// `fates`, with the confirmation `confirm` on or off. Shift+Delete
/// erases, asking unless the prompt is off. Delete erases when any item
/// cannot be recycled; it asks unless the prompt is off, and always
/// when a drive with no Recycle Bin is the reason for the erase.
pub fn delete_mode(confirm: bool, fates: &[Fate], shift: bool) -> DeleteMode {
    if shift {
        return DeleteMode {
            erase: true,
            ask: confirm,
            no_bin: false,
        };
    }
    let no_bin = fates.contains(&Fate::NoBin);
    DeleteMode {
        erase: fates.iter().any(|f| *f != Fate::Recycle),
        ask: confirm || no_bin,
        no_bin,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmKind {
    File,
    Folder,
    Multiple,
}

impl ConfirmKind {
    pub fn of(count: usize, is_dir: bool) -> ConfirmKind {
        match (count, is_dir) {
            (1, true) => ConfirmKind::Folder,
            (1, false) => ConfirmKind::File,
            _ => ConfirmKind::Multiple,
        }
    }
}

/// The caption and question of a delete prompt over `count` items of
/// `kind`, `name` being the lone item's. `erase` removes rather than
/// recycles and says so; `no_bin` leads with the drive having no Recycle
/// Bin, the reason for the erase.
pub fn delete_prompt(
    kind: ConfirmKind,
    name: &str,
    count: usize,
    erase: bool,
    no_bin: bool,
) -> (&'static str, String) {
    let (title, question) = match (kind, erase) {
        (ConfirmKind::File, false) => (
            "Confirm File Delete",
            format!("Are you sure you want to send '{name}' to the Recycle Bin?"),
        ),
        (ConfirmKind::File, true) => (
            "Confirm File Delete",
            format!("Are you sure you want to delete '{name}'?"),
        ),
        (ConfirmKind::Folder, false) => (
            "Confirm Folder Delete",
            format!(
                "Are you sure you want to remove the folder '{name}' and move all its \
                 contents to the Recycle Bin?"
            ),
        ),
        (ConfirmKind::Folder, true) => (
            "Confirm Folder Delete",
            format!("Are you sure you want to remove the folder '{name}' and all its contents?"),
        ),
        (ConfirmKind::Multiple, false) => (
            "Confirm Multiple File Delete",
            format!("Are you sure you want to send these {count} items to the Recycle Bin?"),
        ),
        (ConfirmKind::Multiple, true) => (
            "Confirm Multiple File Delete",
            format!("Are you sure you want to delete these {count} items?"),
        ),
    };
    if !no_bin {
        return (title, question);
    }
    let lead = match count {
        1 => "This drive has no Recycle Bin.",
        _ => "Some of these items are on a drive with no Recycle Bin.",
    };
    (title, format!("{lead} {question}"))
}

pub fn label(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replace::Answer;
    use std::fs;

    #[test]
    fn a_rule_caps_its_share_of_the_drive() {
        let r = |max_percent| Rule {
            remove_immediately: false,
            max_percent,
        };
        assert_eq!(r(10).cap(1_000_000), 100_000);
        assert_eq!(r(1).cap(64 << 20), 671_088);
        assert_eq!(r(0).cap(1 << 40), 0);
        assert_eq!(r(100).cap(u64::MAX), u64::MAX, "no overflow");
        assert_eq!(r(250).cap(1000), 1000, "past 100% is the whole drive");
    }

    #[test]
    fn the_config_round_trips_and_keeps_the_rest_of_the_file() {
        let text = "# mine\n[appearance]\ninterface = \"glass\"\n";
        let mut p = Policy {
            confirm: false,
            independent: true,
            ..Policy::default()
        };
        p.global.max_percent = 7;
        p.drives.insert(HOME.into(), Rule::default());
        p.drives.insert(
            "5A1F-33C2".into(),
            Rule {
                remove_immediately: true,
                max_percent: 1,
            },
        );
        let out = p.write(text).unwrap();
        assert!(out.starts_with(text), "{out}");
        assert!(out.contains("[recycle_bin.drives.home]"), "{out}");
        assert!(out.contains("[recycle_bin.drives.5A1F-33C2]"), "{out}");
        assert_eq!(Policy::parse(&out), p);
        assert_eq!(p.write(&out).unwrap(), out);
        p.drives.clear();
        let out = p.write(&out).unwrap();
        assert!(!out.contains("drives"), "{out}");
        assert_eq!(Policy::parse(&out), p);
    }

    #[test]
    fn missing_and_wrong_keys_keep_their_defaults() {
        assert_eq!(Policy::parse(""), Policy::default());
        assert_eq!(Policy::parse("not toml ["), Policy::default());
        let p = Policy::parse(
            "[recycle_bin]\nconfirm = 3\nmax_percent = 400\n\
             [recycle_bin.drives.home]\nremove_immediately = true\n",
        );
        assert!(p.confirm);
        assert_eq!(p.global.max_percent, 100);
        assert_eq!(
            p.drives[HOME],
            Rule {
                remove_immediately: true,
                max_percent: 100,
            },
            "a drive's missing key is the global one"
        );
    }

    #[test]
    fn a_set_config_path_is_the_one_load_reads() {
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join("config.toml");
        fs::write(&file, "[recycle_bin]\nconfirm = false\n").unwrap();
        set_config_path(Some(file.clone()));
        let (named, set) = (config_path(), Policy::load());
        set_config_path(Some(t.path().join("missing.toml")));
        let missing = Policy::load();
        set_config_path(None);
        assert_eq!(named, file);
        assert!(!set.confirm, "the set file's [recycle_bin]");
        assert_eq!(missing, Policy::default(), "a missing file: the defaults");
        assert_ne!(config_path(), file, "None: the real path again");
    }

    #[test]
    fn drives_follow_their_own_rule_only_when_independent() {
        let mut p = Policy::default();
        let own = Rule {
            remove_immediately: true,
            max_percent: 3,
        };
        p.drives.insert("stick".into(), own);
        assert_eq!(p.rule("stick"), Rule::default());
        p.independent = true;
        assert_eq!(p.rule("stick"), own);
        assert_eq!(p.rule("other"), p.global, "no entry: the global rule");
    }

    #[test]
    fn a_drive_is_keyed_by_its_uuid_link_else_its_mount_point() {
        let t = tempfile::tempdir().unwrap();
        let dev = t.path().join("sdb1");
        fs::write(&dev, "").unwrap();
        let links = t.path().join("by-uuid");
        fs::create_dir(&links).unwrap();
        std::os::unix::fs::symlink("../sdb1", links.join("5A1F-33C2")).unwrap();
        assert_eq!(
            uuid_in(&links, dev.to_str().unwrap()).as_deref(),
            Some("5A1F-33C2")
        );
        assert_eq!(uuid_in(&links, "tmpfs"), None);
        assert_eq!(
            drive_key("tmpfs", Path::new("/run/media/u/STICK")),
            "/run/media/u/STICK"
        );
    }

    #[test]
    fn item_size_sums_a_folder_and_stops_past_the_limit() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("d");
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("a"), [0u8; 100]).unwrap();
        fs::write(d.join("sub/b"), [0u8; 50]).unwrap();
        std::os::unix::fs::symlink("/usr", d.join("link")).unwrap();
        let link = fs::symlink_metadata(d.join("link")).unwrap().len();
        assert_eq!(item_size(&d, u64::MAX).unwrap(), 150 + link);
        assert!(item_size(&d, 10).unwrap() > 10);
        assert_eq!(item_size(&d.join("a"), 0).unwrap(), 100);
        assert!(item_size(&d.join("gone"), 0).is_err());
    }

    fn put(root: &Path, name: &str, size: usize, date: &str, dir: bool) {
        fs::create_dir_all(root.join("files")).unwrap();
        fs::create_dir_all(root.join("info")).unwrap();
        let files = root.join("files").join(name);
        if dir {
            fs::create_dir(&files).unwrap();
            fs::write(files.join("inner"), vec![0u8; size]).unwrap();
        } else {
            fs::write(&files, vec![0u8; size]).unwrap();
        }
        fs::write(
            root.join(format!("info/{name}.trashinfo")),
            format!("[Trash Info]\nPath=/x/{name}\nDeletionDate={date}\n"),
        )
        .unwrap();
    }

    #[test]
    fn used_counts_files_and_keeps_folder_sizes_in_directorysizes() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        put(&root, "a.txt", 10, "2026-01-01T00:00:00", false);
        put(&root, "My Dir", 20, "2026-01-02T00:00:00", true);
        assert_eq!(used(&root), 30);
        let sizes = fs::read_to_string(root.join("directorysizes")).unwrap();
        let line = sizes.lines().next().unwrap();
        assert!(line.starts_with("20 "), "{sizes}");
        assert!(line.ends_with(" My%20Dir"), "{sizes}");
        assert_eq!(sizes.lines().count(), 1, "files have no line");

        let mtime = info_mtime(&root, OsStr::new("My Dir"));
        fs::write(
            root.join("directorysizes"),
            format!("999 {mtime} My%20Dir\n7 1 Gone\n"),
        )
        .unwrap();
        // Forget this trash's total only: tests running alongside keep
        // theirs in the same map.
        TOTALS.lock().unwrap().get_or_insert_default().remove(&root);
        assert_eq!(used(&root), 1009);
        assert_eq!(
            fs::read_to_string(root.join("directorysizes")).unwrap(),
            format!("999 {mtime} My%20Dir\n"),
            "a folder no longer in the trash loses its line"
        );
    }

    #[test]
    fn used_keeps_its_total_until_files_changes() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        put(&root, "a", 10, "2026-01-01T00:00:00", false);
        assert_eq!(used(&root), 10);
        // Growing a file in place leaves files/ as it was: the kept total.
        fs::write(root.join("files/a"), [0u8; 40]).unwrap();
        assert_eq!(used(&root), 10);
        put(&root, "b", 5, "2026-01-01T00:00:00", false);
        assert_eq!(used(&root), 45);
        assert_eq!(used(&t.path().join("none")), 0);
    }

    #[test]
    fn make_room_erases_the_oldest_items_until_the_new_one_fits() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        put(&root, "newest", 30, "2026-03-01T00:00:00", false);
        put(&root, "oldest", 30, "2026-01-01T00:00:00", true);
        put(&root, "middle", 30, "2026-02-01T00:00:00", false);
        put(&root, "undated", 1, "garbage", false);

        // 91 held, cap 100: 40 more needs 31 gone, the undated one and
        // the oldest.
        let (left, purged) = make_room(&root, 91, 40, 100).unwrap();
        assert_eq!(left, 60);
        assert_eq!(
            purged,
            [root.join("files/undated"), root.join("files/oldest")]
        );
        for name in ["undated", "oldest"] {
            assert!(!root.join("files").join(name).exists());
            assert!(!root.join(format!("info/{name}.trashinfo")).exists());
        }
        assert!(root.join("files/middle").exists() && root.join("files/newest").exists());

        assert_eq!(make_room(&root, 60, 40, 100).unwrap(), (60, Vec::new()));
    }

    #[test]
    fn too_big_asks_in_order_and_yes_to_all_answers_the_rest() {
        let p = |s: &str| PathBuf::from(s);
        let paths = vec![p("a"), p("big1"), p("b"), p("big2"), p("big3")];
        let big = vec![false, true, false, true, true];
        let mut q = TooBig::new(paths.clone(), big.clone());
        assert!(q.many());
        assert_eq!(q.step(), TooBigStep::Ask(Path::new("big1")));
        q.answer(Answer::No);
        assert_eq!(q.step(), TooBigStep::Ask(Path::new("big2")));
        q.answer(Answer::YesToAll);
        assert_eq!(
            q.step(),
            TooBigStep::Done(vec![
                (p("a"), false),
                (p("b"), false),
                (p("big2"), true),
                (p("big3"), true),
            ])
        );

        let mut q = TooBig::new(paths.clone(), big.clone());
        q.answer(Answer::Yes);
        q.answer(Answer::Cancel);
        assert_eq!(q.step(), TooBigStep::Cancelled);
        q.answer(Answer::Yes);
        assert_eq!(q.step(), TooBigStep::Cancelled);

        let mut one = TooBig::new(vec![p("big")], vec![true]);
        assert!(!one.many());
        one.answer(Answer::Yes);
        assert_eq!(one.step(), TooBigStep::Done(vec![(p("big"), true)]));
    }

    #[test]
    fn the_delete_prompts_read_as_expected() {
        use ConfirmKind::*;
        let p = |kind, name, count, erase, no_bin| delete_prompt(kind, name, count, erase, no_bin);
        let same = |(title, text): (&str, String), want: (&str, &str)| {
            assert_eq!((title, text.as_str()), want);
        };
        same(
            p(File, "notes.txt", 1, false, false),
            (
                "Confirm File Delete",
                "Are you sure you want to send 'notes.txt' to the Recycle Bin?",
            ),
        );
        same(
            p(File, "notes.txt", 1, true, false),
            (
                "Confirm File Delete",
                "Are you sure you want to delete 'notes.txt'?",
            ),
        );
        same(
            p(Folder, "Reports", 1, false, false),
            (
                "Confirm Folder Delete",
                "Are you sure you want to remove the folder 'Reports' and move all its contents \
                 to the Recycle Bin?",
            ),
        );
        same(
            p(Folder, "Reports", 1, true, false),
            (
                "Confirm Folder Delete",
                "Are you sure you want to remove the folder 'Reports' and all its contents?",
            ),
        );
        same(
            p(Multiple, "", 4, false, false),
            (
                "Confirm Multiple File Delete",
                "Are you sure you want to send these 4 items to the Recycle Bin?",
            ),
        );
        same(
            p(Multiple, "", 4, true, false),
            (
                "Confirm Multiple File Delete",
                "Are you sure you want to delete these 4 items?",
            ),
        );
        same(
            p(File, "d.txt", 1, true, true),
            (
                "Confirm File Delete",
                "This drive has no Recycle Bin. Are you sure you want to delete 'd.txt'?",
            ),
        );
        same(
            p(Folder, "Reports", 1, true, true),
            (
                "Confirm Folder Delete",
                "This drive has no Recycle Bin. Are you sure you want to remove the folder \
                 'Reports' and all its contents?",
            ),
        );
        same(
            p(Multiple, "", 3, true, true),
            (
                "Confirm Multiple File Delete",
                "Some of these items are on a drive with no Recycle Bin. Are you sure you want \
                 to delete these 3 items?",
            ),
        );
    }

    #[test]
    fn one_entry_is_a_file_or_a_folder_and_more_are_multiple() {
        assert_eq!(ConfirmKind::of(1, false), ConfirmKind::File);
        assert_eq!(ConfirmKind::of(1, true), ConfirmKind::Folder);
        assert_eq!(ConfirmKind::of(3, true), ConfirmKind::Multiple);
        assert_eq!(ConfirmKind::of(0, false), ConfirmKind::Multiple);
    }

    #[test]
    fn delete_mode_erases_what_cannot_be_recycled_and_asks_as_set() {
        use Fate::*;
        let m = |erase, ask, no_bin| DeleteMode { erase, ask, no_bin };
        assert_eq!(delete_mode(true, &[Recycle], false), m(false, true, false));
        assert_eq!(
            delete_mode(false, &[Recycle], false),
            m(false, false, false)
        );
        assert_eq!(
            delete_mode(false, &[Recycle, RemoveImmediately], false),
            m(true, false, false),
            "a drive set to remove immediately erases without the lead"
        );
        assert_eq!(
            delete_mode(false, &[Recycle, NoBin], false),
            m(true, true, true),
            "an erase the settings did not choose asks even with the prompt off"
        );
        assert_eq!(delete_mode(true, &[Recycle], true), m(true, true, false));
        assert_eq!(delete_mode(false, &[NoBin], true), m(true, false, false));
    }
}
