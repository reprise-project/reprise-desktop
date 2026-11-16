//! `config.toml`: [`config`] reads and validates it with every default, [`current`] is a
//! program's read of it, and the rest edits it in place: `toml_edit` keeps comments, key order and
//! unknown tables, so a program editing a few keys leaves the rest of the file byte-identical.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use toml_edit::{Decor, DocumentMut, Item, TableLike, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub table: &'static str,
    pub name: &'static str,
}

pub const fn key(table: &'static str, name: &'static str) -> Key {
    Key { table, name }
}

pub struct Lens<S, T> {
    get: fn(&S) -> &T,
    get_mut: fn(&mut S) -> &mut T,
}

impl<S, T> Lens<S, T> {
    pub const fn new(get: fn(&S) -> &T, get_mut: fn(&mut S) -> &mut T) -> Self {
        Lens { get, get_mut }
    }

    pub fn get<'a>(&self, s: &'a S) -> &'a T {
        (self.get)(s)
    }

    pub fn get_mut<'a>(&self, s: &'a mut S) -> &'a mut T {
        (self.get_mut)(s)
    }
}

#[macro_export]
macro_rules! lens {
    ($field:ident) => {
        $crate::Lens::new(|s| &s.$field, |s| &mut s.$field)
    };
}

/// The config file: the compositor's (`reprise --config` included), else the user's own
/// ([`reprisefs::configfile::path`]).
pub fn config_path() -> PathBuf {
    reprisefs::configfile::path()
}

/// The compositor's state file, `$XDG_STATE_HOME/reprise/state.toml`.
pub fn state_path() -> PathBuf {
    reprisexdg::state_home()
        .unwrap_or_default()
        .join("reprise")
        .join("state.toml")
}

/// The config file read as the compositor reads it ([`config::lenient_over`]): a key that fails
/// keeps its value from this thread's last read, or its default on the first, and a file that
/// can't be read or isn't TOML keeps the last read whole; an absent file reads as all defaults.
/// Read again on this thread only once the file's size, time or inode changes.
pub fn current() -> std::rc::Rc<config::Config> {
    current_at(config_path())
}

/// [`current`]'s last read on a thread.
struct Last {
    path: PathBuf,
    stamp: Option<FileStamp>,
    cfg: std::rc::Rc<config::Config>,
    /// The table `cfg` was built from, the base of the next read.
    table: toml::Table,
}

fn current_at(path: PathBuf) -> std::rc::Rc<config::Config> {
    thread_local! {
        static LAST: std::cell::RefCell<Option<Last>> = const { std::cell::RefCell::new(None) };
    }
    let stamp = FileStamp::of(&path);
    LAST.with_borrow_mut(|last| {
        let last_here = last.as_ref().filter(|l| l.path == path);
        if let Some(l) = last_here.filter(|l| l.stamp == stamp) {
            return l.cfg.clone();
        }
        let base = last_here.map(|l| (l.cfg.clone(), l.table.clone()));
        let (cfg, table) = read_file(&path, base);
        let next = Last {
            path,
            stamp,
            cfg: cfg.clone(),
            table,
        };
        *last = Some(next);
        cfg
    })
}

/// `path` read over `last`, the config and table of the last read of it.
fn read_file(
    path: &Path,
    last: Option<(std::rc::Rc<config::Config>, toml::Table)>,
) -> (std::rc::Rc<config::Config>, toml::Table) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.to_string()),
    };
    let base = last.as_ref().map(|(_, t)| t.clone()).unwrap_or_default();
    let read = text.and_then(|text| config::lenient_over(&text, &base, config::Config::from_file));
    match (read, last) {
        (Ok((cfg, _, table)), _) => (std::rc::Rc::new(cfg), table),
        (Err(e), Some(last)) => {
            log::warn!("{}: {e}; keeping the settings read before", path.display());
            last
        }
        (Err(e), None) => {
            log::warn!("{}: {e}; every key reads at its default", path.display());
            let (cfg, _) = config::Config::parse_lenient("").expect("an empty file reads");
            (std::rc::Rc::new(cfg), toml::Table::new())
        }
    }
}

/// What [`current`] compares to learn the file changed.
#[derive(Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: (i64, i64),
}

impl FileStamp {
    fn of(path: &Path) -> Option<FileStamp> {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(path).ok()?;
        Some(FileStamp {
            dev: m.dev(),
            ino: m.ino(),
            len: m.len(),
            mtime: (m.mtime(), m.mtime_nsec()),
        })
    }
}

/// `[drives] on_insert`'s default.
pub const DRIVES_ON_INSERT: &str = "ask";

/// `[thumbnail_cache]`: whether pictures' thumbnails are kept in `$XDG_CACHE_HOME/thumbnails`,
/// shared with other file managers, and how much room they may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThumbnailCache {
    pub enabled: bool,
    /// The cap in MiB; past it the least recently used thumbnails go, whichever program wrote them.
    pub max_mb: u32,
    /// `external_thumbnailers`: files Reprise cannot decode (PDFs, videos, fonts) get thumbnails
    /// from the installed freedesktop thumbnailers.
    pub external_thumbnailers: bool,
}

impl ThumbnailCache {
    pub const DEFAULT: ThumbnailCache = ThumbnailCache {
        enabled: true,
        max_mb: 512,
        external_thumbnailers: true,
    };
    /// A thumbnail is at most 64 KiB of pixels at scale 1, 256 KiB at scale 2 and 1 MiB
    /// at its largest.
    pub const MAX_MB: std::ops::RangeInclusive<u32> = 1..=4096;

    pub const fn max_bytes(self) -> u64 {
        (self.max_mb as u64) << 20
    }
}

/// `[storage] low_space_percent`: reprise-drives warns when the disk of `$HOME` or `/` has less
/// free than this share of its size; 0 turns the warning off.
pub const LOW_SPACE_PERCENT: u8 = 5;
pub const LOW_SPACE_MAX: u8 = 50;

/// `[bluetooth] battery_warning`: reprise-bluetooth warns once when a connected device's battery
/// drops to this percent, again after it charges [`BT_BATTERY_REARM`] points past it; 0 turns the
/// warning off.
pub const BT_BATTERY_WARNING: u8 = 15;
pub const BT_BATTERY_MAX: u8 = 50;
pub const BT_BATTERY_REARM: u8 = 5;

/// Write `[bluetooth] battery_warning = percent` into the config file, the default as no key (and
/// no table once it is empty).
pub fn save_bt_battery_warning(percent: u8) -> Result<(), String> {
    save_bt_key(
        "battery_warning",
        (percent != BT_BATTERY_WARNING).then(|| Value::from(i64::from(percent))),
    )
}

/// Write `[bluetooth] receive_files = on` into the config file, off as no key (and no table once
/// it is empty).
pub fn save_bt_receive_files(on: bool) -> Result<(), String> {
    save_bt_key("receive_files", on.then(|| Value::from(true)))
}

fn save_bt_key(name: &'static str, value: Option<Value>) -> Result<(), String> {
    match edit(&config_path(), |text| with_bt_key(text, name, value)) {
        Ok(r) => r.map(|_| ()),
        Err(e) => Err(e.to_string()),
    }
}

/// `text` with `[bluetooth] name` set to `value`, or removed for None along with a table left empty.
fn with_bt_key(text: &str, name: &'static str, value: Option<Value>) -> Result<String, String> {
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let k = key("bluetooth", name);
    match value {
        Some(v) => set(&mut doc, &k, v).map_err(|e| e.to_string())?,
        None => {
            remove(&mut doc, &k);
            remove_table_if_empty(&mut doc, k.table);
        }
    }
    Ok(doc.to_string())
}

/// Write `[time_zone] automatic = on` into the config file, off as no key (and no table once it
/// is empty).
pub fn save_auto_time_zone(on: bool) -> Result<(), String> {
    match edit(&config_path(), |text| with_auto_time_zone(text, on)) {
        Ok(r) => r.map(|_| ()),
        Err(e) => Err(e.to_string()),
    }
}

fn with_auto_time_zone(text: &str, on: bool) -> Result<String, String> {
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let k = key("time_zone", "automatic");
    if on {
        set(&mut doc, &k, Value::from(true)).map_err(|e| e.to_string())?;
    } else {
        remove(&mut doc, &k);
        remove_table_if_empty(&mut doc, k.table);
    }
    Ok(doc.to_string())
}

pub type ConfigWatch = reprisefs::folderwatch::Watch;

/// A watch on the config file: `wake` runs on its thread after each write or rename onto it, and
/// [`ConfigWatch::take_changed`] says whether one came. None when inotify fails.
pub fn watch(wake: impl Fn() + Send + Sync + 'static) -> Option<ConfigWatch> {
    watch_at(&config_path(), wake)
}

fn watch_at(path: &Path, wake: impl Fn() + Send + Sync + 'static) -> Option<ConfigWatch> {
    let mut w = reprisefs::folderwatch::Watch::new(wake)
        .map_err(|e| log::warn!("the config file is not watched: {e}"))
        .ok()?;
    let entries: Vec<_> = watch_targets(path)
        .into_iter()
        .flat_map(|(dir, names)| names.into_iter().map(move |n| (dir.clone(), Some(n))))
        .collect();
    w.watch_all(&entries);
    Some(w)
}

/// The directories a watch on the config file at `path` covers, each with the entry names that
/// count: `path`'s own, and, when `path` is a symlink, the file it resolves to, since
/// [`write_atomic`] and editors writing through the link replace that file in its own directory.
pub fn watch_targets(path: &Path) -> Vec<(PathBuf, Vec<OsString>)> {
    let Some(name) = path.file_name() else {
        return Vec::new();
    };
    // A relative `--config path.toml` has "" for a parent; that is the working directory, which
    // is what inotify wants spelled out.
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let mut targets = vec![(dir.clone(), vec![name.to_os_string()])];
    if let Ok(real) = std::fs::canonicalize(path)
        && let (Some(real_dir), Some(real_name)) = (real.parent(), real.file_name())
    {
        // One inotify watch per directory: a link to a file beside it adds a name, not a second
        // watch.
        if std::fs::canonicalize(&dir).is_ok_and(|d| d == real_dir) {
            if real_name != name {
                targets[0].1.push(real_name.to_os_string());
            }
        } else {
            targets.push((real_dir.to_path_buf(), vec![real_name.to_os_string()]));
        }
    }
    targets
}

/// A refresh in Hz with the decimals it needs: `60`, `59.94`.
pub fn hz_string(refresh_mhz: i32) -> String {
    let hz = format!("{:.3}", refresh_mhz as f64 / 1000.0);
    hz.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Write `text` to `path` with `reprisefs::ops::write_atomic`; the compositor's watch sees the
/// rename as `MOVED_TO`/`CLOSE_WRITE`.
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    reprisefs::ops::write_atomic(path, text.as_bytes())
}

/// `reprisefs::ops::locked_edit` of `path`: read, edited and written atomically under a lock
/// beside it.
pub fn edit<E>(
    path: &Path,
    edit: impl FnOnce(&str) -> Result<String, E>,
) -> std::io::Result<Result<String, E>> {
    reprisefs::ops::locked_edit(path, false, edit)
}

/// [`edit`] for a file whose text can be rebuilt: one that is not UTF-8 reads as empty and is
/// overwritten.
pub fn edit_or_reset<E>(
    path: &Path,
    edit: impl FnOnce(&str) -> Result<String, E>,
) -> std::io::Result<Result<String, E>> {
    reprisefs::ops::locked_edit(path, true, edit)
}

pub fn get<'a>(doc: &'a DocumentMut, key: &Key) -> Option<&'a Item> {
    table_of(doc, key.table)?.get(key.name)
}

fn prefix_of(d: &Decor) -> String {
    d.prefix()
        .and_then(|s| s.as_str())
        .unwrap_or_default()
        .to_string()
}

fn segments(path: &str) -> impl Iterator<Item = &str> {
    path.split('.').filter(|s| !s.is_empty())
}

fn table_of<'a>(doc: &'a DocumentMut, path: &str) -> Option<&'a dyn TableLike> {
    let mut t: &dyn TableLike = doc.as_table();
    for seg in segments(path) {
        t = t.get(seg)?.as_table_like()?;
    }
    Some(t)
}

fn table_of_mut<'a>(doc: &'a mut DocumentMut, path: &str) -> Option<&'a mut dyn TableLike> {
    let mut t: &mut dyn TableLike = doc.as_table_mut();
    for seg in segments(path) {
        t = t.get_mut(seg)?.as_table_like_mut()?;
    }
    Some(t)
}

fn split_last(path: &str) -> Option<(&str, &str)> {
    let last = segments(path).last()?;
    let parent = &path[..path.len() - last.len()];
    Some((parent.trim_end_matches('.'), last))
}

/// Remove `key`, handing the comments above it to whatever comes next (they are as often about the
/// next key). The file stays byte-identical but for the removed line.
pub fn remove(doc: &mut DocumentMut, key: &Key) {
    let Some(table) = table_of(doc, key.table) else {
        return;
    };
    if table.get(key.name).is_none() {
        return;
    }
    let prefix = entry_prefix(table, key.name);
    let next = table
        .iter()
        .map(|(n, _)| n.to_string())
        .skip_while(|n| n != key.name)
        .nth(1);
    if let Some(t) = table_of_mut(doc, key.table) {
        t.remove(key.name);
    }
    if !prefix.contains('#') {
        return;
    }
    if let Some(next) = next
        && let Some(t) = table_of_mut(doc, key.table)
    {
        prepend_entry_prefix(t, &next, &prefix);
        return;
    }
    carry_past_table(doc, key.table, &prefix);
}

fn entry_prefix(t: &dyn TableLike, name: &str) -> String {
    match t.get(name) {
        Some(Item::Table(tb)) => prefix_of(tb.decor()),
        Some(Item::ArrayOfTables(a)) => {
            a.get(0).map(|tb| prefix_of(tb.decor())).unwrap_or_default()
        }
        _ => t
            .key(name)
            .map(|k| prefix_of(k.leaf_decor()))
            .unwrap_or_default(),
    }
}

fn prepend_entry_prefix(t: &mut dyn TableLike, name: &str, prefix: &str) {
    let join = |old: String| {
        let old = if prefix.ends_with("\n\n") {
            old.trim_start_matches('\n').to_string()
        } else {
            old
        };
        format!("{prefix}{old}")
    };
    match t.get_mut(name) {
        Some(Item::Table(tb)) => {
            let new = join(prefix_of(tb.decor()));
            tb.decor_mut().set_prefix(new);
        }
        Some(Item::ArrayOfTables(a)) => {
            if let Some(tb) = a.get_mut(0) {
                let new = join(prefix_of(tb.decor()));
                tb.decor_mut().set_prefix(new);
            }
        }
        _ => {
            if let Some(mut k) = t.key_mut(name) {
                let new = join(prefix_of(k.leaf_decor()));
                k.leaf_decor_mut().set_prefix(new);
            }
        }
    }
}

fn carry_past_table(doc: &mut DocumentMut, table: &str, prefix: &str) {
    match after_table(doc, table) {
        Some((parent, next)) => {
            if let Some(pt) = table_of_mut(doc, &parent) {
                prepend_entry_prefix(pt, &next, prefix);
            }
        }
        None => {
            let old = doc.trailing().as_str().unwrap_or_default().to_string();
            doc.set_trailing(format!("{prefix}{old}"));
        }
    }
}

/// The entry whose prefix holds the text below `table`'s last entry, as `(parent, name)`; None
/// for the document's trailing text.
fn after_table(doc: &DocumentMut, table: &str) -> Option<(String, String)> {
    let (parent, name) = split_last(table).unwrap_or(("", ""));
    let names: Vec<String> = table_of(doc, parent)
        .map(|t| t.iter().map(|(n, _)| n.to_string()).collect())
        .unwrap_or_default();
    let next = if name.is_empty() {
        names
            .iter()
            .find(|n| {
                doc.get(n.as_str())
                    .map(|i| i.is_table_like() || i.is_array_of_tables())
                    .unwrap_or(false)
            })
            .cloned()
    } else {
        names
            .iter()
            .position(|n| n == name)
            .and_then(|i| names.get(i + 1))
            .cloned()
    };
    match next {
        Some(next) => Some((parent.to_string(), next)),
        None if name.is_empty() || parent.is_empty() => None,
        None => after_table(doc, parent),
    }
}

/// Remove the table at `path` (dotted for a nested one) when it holds no entry and no comment
/// sits on its header line or under it. A comment above it moves on, as [`remove`] moves it.
pub fn remove_table_if_empty(doc: &mut DocumentMut, path: &'static str) {
    let Some((parent, name)) = split_last(path) else {
        return;
    };
    let Some(table) = table_of(doc, parent).and_then(|t| t.get(name)) else {
        return;
    };
    if !table.as_table_like().is_some_and(|t| t.is_empty()) {
        return;
    }
    let on_header = (table.as_table())
        .and_then(|t| t.decor().suffix())
        .and_then(|r| r.as_str())
        .is_some_and(|s| s.contains('#'));
    let below = match after_table(doc, path) {
        Some((p, next)) => table_of(doc, &p).map(|t| entry_prefix(t, &next)),
        None => doc.trailing().as_str().map(str::to_string),
    };
    if !on_header && !below.is_some_and(|text| text.contains('#')) {
        remove(doc, &key(parent, name));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetError {
    NotATable(String),
    IsATable(String),
}

impl std::fmt::Display for SetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetError::NotATable(path) => write!(f, "{path} is not a table"),
            SetError::IsATable(path) => write!(f, "{path} is a table"),
        }
    }
}

impl std::error::Error for SetError {}

/// Set `key` to `value`, creating tables on the way; an existing key keeps its decor. `Err` when a
/// scalar sits where a table is wanted or the reverse; the user wrote that, so it stays.
pub fn set(doc: &mut DocumentMut, key: &Key, value: Value) -> Result<(), SetError> {
    let segs: Vec<&str> = segments(key.table).collect();
    let mut t: &mut dyn TableLike = doc.as_table_mut();
    for (i, seg) in segs.iter().enumerate() {
        match t.get(seg) {
            None => {
                let mut table = toml_edit::Table::new();
                table.set_implicit(i + 1 < segs.len());
                t.insert(seg, Item::Table(table));
            }
            Some(item) if !item.is_table_like() => {
                return Err(SetError::NotATable(segs[..=i].join(".")));
            }
            Some(_) => {}
        }
        t = t
            .get_mut(seg)
            .and_then(Item::as_table_like_mut)
            .expect("just checked or inserted a table");
    }
    match t.get_mut(key.name) {
        Some(Item::Value(old)) => {
            let decor = old.decor().clone();
            *old = value;
            *old.decor_mut() = decor;
        }
        Some(_) => return Err(SetError::IsATable(format!("{}.{}", key.table, key.name))),
        None => {
            t.insert(key.name, Item::Value(value));
        }
    }
    Ok(())
}

pub mod config;
pub mod displays;
pub mod filedialog;
pub mod glob;
pub mod keybind;
pub mod location;
pub mod pages;
pub mod projection;
pub mod recording;
pub mod remote;
pub mod search;
pub mod statefile;
pub mod welcome;

pub mod system_icons {
    pub fn current(name: &str) -> Option<&str> {
        match name {
            "my-documents" => Some("documents"),
            "my-computer" => Some("computer"),
            "my-network-places" => None,
            name => Some(name),
        }
    }

    pub fn current_list<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for name in names.into_iter().filter_map(current) {
            if !out.iter().any(|n| n == name) {
                out.push(name.to_string());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each key programs read, set wrong or out of range beside one that reads.
    const BAD_VALUES: &str = r##"
terminal = "foot -o x=1"
script_terminal = "later"
script_open = "terminal"
confirm_scripts = "no"
[privacy]
recent_files = false
location = 3
[glass]
color = "#102030"
[fonts]
shell_dlg = " "
family = ["A", 3]
antialias = "yes"
[accessibility]
text_size = 140
[metrics]
double_click_ms = -1
[taskbar]
position = 1
[drives]
on_insert = "open"
[power]
locker = "  "
[storage]
low_space_percent = 90
[bluetooth]
battery_warning = -3
receive_files = "yes"
[time_zone]
automatic = true
[sounds]
allow_boost = 1
[updates]
restart_notice = false
[thumbnail_cache]
max_mb = 100000
external_thumbnailers = false
"##;

    #[test]
    fn a_program_reads_each_key_as_the_compositor_does() {
        use reprisefs::program::{ScriptEnd, ScriptOpen};
        let c = read(BAD_VALUES);
        let d = read("");
        assert_eq!(c.terminal, "foot -o x=1");
        assert_eq!(c.script_terminal, ScriptEnd::Stay);
        assert_eq!(c.script_open, ScriptOpen::Terminal);
        assert!(c.confirm_scripts);
        assert!(!c.recent_files);
        assert!(c.location);
        assert_eq!(
            c.glass.color,
            reprisekit::theme::Color::rgb(0x10, 0x20, 0x30)
        );
        assert_eq!(c.theme.fonts.shell_dlg, None);
        assert_eq!(c.theme.fonts.family, d.theme.fonts.family);
        assert_eq!(c.theme.fonts.antialias, d.theme.fonts.antialias);
        assert_eq!(c.accessibility.text_size, 100);
        assert_eq!(c.theme.metrics.double_click_ms, 500);
        assert_eq!(c.taskbar.position, reprisekit::taskbar::Edge::Bottom);
        assert_eq!(c.drives_on_insert, "open");
        assert!(c.power.locker.trim().is_empty());
        assert_eq!(c.low_space_percent, LOW_SPACE_MAX);
        assert_eq!(c.bluetooth.battery_warning, 0);
        assert!(!c.bluetooth.receive_files);
        assert!(c.auto_time_zone);
        assert!(!c.sounds.allow_boost);
        assert!(!c.restart_notice);
        let cache = ThumbnailCache {
            max_mb: *ThumbnailCache::MAX_MB.end(),
            external_thumbnailers: false,
            ..ThumbnailCache::DEFAULT
        };
        assert_eq!(c.thumbnail_cache, cache);
        assert_eq!(read("[fonts").terminal, d.terminal, "not TOML: defaults");
    }

    #[test]
    fn the_defaults_programs_read_stay_as_documented() {
        let d = read("");
        assert_eq!(d.drives_on_insert, DRIVES_ON_INSERT);
        assert_eq!(d.low_space_percent, LOW_SPACE_PERCENT);
        assert_eq!(d.bluetooth.battery_warning, BT_BATTERY_WARNING);
        assert_eq!(d.thumbnail_cache, ThumbnailCache::DEFAULT);
        assert_eq!(ThumbnailCache::DEFAULT.max_bytes(), 512 << 20);
    }

    #[test]
    fn the_file_is_read_again_only_once_it_changes() {
        let dir = std::env::temp_dir().join(format!("repriseconf-current-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let absent = current_at(path.clone());
        assert!(std::rc::Rc::ptr_eq(&absent, &current_at(path.clone())));
        write_atomic(&path, "terminal = \"foot\"\n").unwrap();
        let first = current_at(path.clone());
        assert_eq!(first.terminal, "foot");
        assert!(std::rc::Rc::ptr_eq(&first, &current_at(path.clone())));
        write_atomic(&path, "terminal = \"st\"\n").unwrap();
        assert_eq!(current_at(path.clone()).terminal, "st");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_reread_keeps_what_it_cannot_read_from_the_last_read() {
        let dir = scratch("reread");
        let path = dir.join("config.toml");
        let read = || current_at(path.clone()).remote_desktop;
        write_atomic(&path, "[remote_desktop]\nport = 4000\nmax_viewers = 2\n").unwrap();
        assert_eq!((read().port, read().max_viewers), (4000, 2));
        write_atomic(&path, "[remote_desktop]\nport = 70000\nmax_viewers = 3\n").unwrap();
        assert_eq!(
            (read().port, read().max_viewers),
            (4000, 3),
            "the bad port keeps 4000"
        );
        write_atomic(&path, "[remote_desktop\nport = 4001\n").unwrap();
        assert_eq!((read().port, read().max_viewers), (4000, 3), "not TOML");
        std::fs::write(&path, b"[remote_desktop]\nport = 4002\n\xff\n").unwrap();
        assert_eq!(read().port, 4000, "not UTF-8");
        std::fs::remove_file(&path).unwrap();
        assert_eq!(read(), remote::RemoteDesktop::default(), "an absent file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_first_read_gives_a_bad_key_its_default() {
        let dir = scratch("first-read");
        let path = dir.join("config.toml");
        write_atomic(&path, "[remote_desktop]\nport = 70000\nmax_viewers = 3\n").unwrap();
        let rd = current_at(path).remote_desktop;
        assert_eq!((rd.port, rd.max_viewers), (3389, 3));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// [`current`]'s first read of a file holding `text`.
    fn read(text: &str) -> std::rc::Rc<config::Config> {
        static FILES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = FILES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = scratch(&format!("read-{n}"));
        let path = dir.join("config.toml");
        std::fs::write(&path, text).unwrap();
        let cfg = current_at(path);
        let _ = std::fs::remove_dir_all(&dir);
        cfg
    }

    /// A fresh directory for test `tag`.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("repriseconf-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn older_system_icon_names_read_as_the_current_ones() {
        assert_eq!(
            system_icons::current_list([
                "my-documents",
                "my-network-places",
                "computer",
                "my-computer",
                "recycle-bin",
                "custom-x",
            ]),
            ["documents", "computer", "recycle-bin", "custom-x"]
        );
        assert_eq!(system_icons::current("home"), Some("home"));
    }

    #[test]
    fn saving_the_bt_battery_warning_drops_the_default() {
        let warn = |text: &str, pct: u8| {
            with_bt_key(
                text,
                "battery_warning",
                (pct != BT_BATTERY_WARNING).then(|| Value::from(i64::from(pct))),
            )
        };
        let set = warn("# mine\n[taskbar]\nheight = 30\n", 25).unwrap();
        assert_eq!(read(&set).bluetooth.battery_warning, 25);
        assert!(
            set.contains("# mine\n[taskbar]\nheight = 30\n"),
            "the rest of the file stays"
        );
        let back = warn(&set, 15).unwrap();
        assert!(!back.contains("bluetooth"), "{back}");
        assert!(warn("[bluetooth\n", 25).is_err());
    }

    #[test]
    fn saving_bt_receive_files_writes_only_on() {
        let on = with_bt_key("", "receive_files", Some(Value::from(true))).unwrap();
        assert!(read(&on).bluetooth.receive_files);
        let back = with_bt_key(&on, "receive_files", None).unwrap();
        assert!(!read(&back).bluetooth.receive_files);
        assert!(!back.contains("bluetooth"), "off is no key: {back}");
    }

    #[test]
    fn saving_the_automatic_time_zone_writes_only_on() {
        let set = with_auto_time_zone("# mine\n[taskbar]\nheight = 30\n", true).unwrap();
        assert!(read(&set).auto_time_zone);
        assert!(
            set.contains("# mine\n[taskbar]\nheight = 30\n"),
            "the rest of the file stays"
        );
        let back = with_auto_time_zone(&set, false).unwrap();
        assert!(!back.contains("time_zone"), "{back}");
    }

    #[test]
    fn concurrent_edits_serialise_on_the_lock() {
        let dir = std::env::temp_dir().join(format!("repriseconf-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let slow = {
            let path = path.clone();
            std::thread::spawn(move || {
                edit::<()>(&path, |text| {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    Ok(format!("{text}a = 1\n"))
                })
                .unwrap()
                .unwrap()
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(100));
        let second = edit::<()>(&path, |text| Ok(format!("{text}b = 2\n")))
            .unwrap()
            .unwrap();
        slow.join().unwrap();
        assert_eq!(second, "a = 1\nb = 2\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = 1\nb = 2\n");
        let r = edit(&path, |_| Err::<String, _>("no")).unwrap();
        assert_eq!(r, Err("no"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = 1\nb = 2\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_edit_that_changes_nothing_writes_nothing() {
        use std::os::unix::fs::MetadataExt;
        let dir = std::env::temp_dir().join(format!("repriseconf-noop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "a = 1\n").unwrap();
        let before = std::fs::metadata(&path).unwrap().ino();
        let r = edit::<()>(&path, |text| Ok(text.to_string())).unwrap();
        assert_eq!(r, Ok("a = 1\n".to_string()));
        assert_eq!(
            std::fs::metadata(&path).unwrap().ino(),
            before,
            "the file was not replaced"
        );
        let absent = dir.join("absent.toml");
        assert_eq!(
            edit::<()>(&absent, |_| Ok(String::new())).unwrap(),
            Ok(String::new())
        );
        assert!(!absent.exists(), "no empty file is created");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_atomic_follows_a_symlink_and_keeps_the_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("repriseconf-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("dotfiles")).unwrap();
        let real = dir.join("dotfiles/config.toml");
        std::fs::write(&real, "old").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.join("config.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        write_atomic(&link, "new").unwrap();

        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
        assert_eq!(
            std::fs::metadata(&real).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_save_through_a_symlinked_config_wakes_the_watch() {
        let dir = std::env::temp_dir().join(format!("repriseconf-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("conf")).unwrap();
        std::fs::create_dir_all(dir.join("dotfiles")).unwrap();
        std::fs::write(dir.join("dotfiles/config.toml"), "").unwrap();
        let link = dir.join("conf/config.toml");
        std::os::unix::fs::symlink("../dotfiles/config.toml", &link).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut w = watch_at(&link, move || {
            let _ = tx.send(());
        })
        .unwrap();

        write_atomic(&link, "terminal = \"foot\"\n").unwrap();

        let woke = rx.recv_timeout(std::time::Duration::from_secs(10));
        assert!(woke.is_ok(), "the save through the link wakes the watch");
        assert!(w.take_changed());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_atomic_leaves_a_planted_temp_symlink_alone() {
        let dir = std::env::temp_dir().join(format!("repriseconf-plant-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let victim = dir.join("victim");
        std::fs::write(&path, "old").unwrap();
        std::fs::write(&victim, "keep").unwrap();
        let planted = dir.join(format!(".config.toml.{}.tmp", std::process::id()));
        std::os::unix::fs::symlink(&victim, &planted).unwrap();

        write_atomic(&path, "new").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn doc(s: &str) -> DocumentMut {
        s.parse().unwrap()
    }

    #[test]
    fn set_refuses_to_replace_a_scalar_with_a_table() {
        let mut d = doc("idle = 5
");
        let err = set(
            &mut d,
            &key("idle.battery", "suspend_after"),
            Value::from(900),
        );
        assert_eq!(err, Err(SetError::NotATable("idle".into())));
        assert_eq!(
            d.to_string(),
            "idle = 5
",
            "the file is untouched"
        );
    }

    #[test]
    fn set_refuses_to_replace_a_table_with_a_value() {
        let mut d = doc("[idle.battery]
x = 1
");
        let err = set(&mut d, &key("idle", "battery"), Value::from(900));
        assert_eq!(err, Err(SetError::IsATable("idle.battery".into())));
        assert_eq!(
            d.to_string(),
            "[idle.battery]
x = 1
"
        );
    }

    #[test]
    fn set_creates_a_nested_table_with_its_own_header() {
        let mut d = doc("");
        set(
            &mut d,
            &key("idle.battery", "suspend_after"),
            Value::from(900),
        )
        .unwrap();
        assert_eq!(d.to_string(), "[idle.battery]\nsuspend_after = 900\n");
    }

    #[test]
    fn set_leaves_an_existing_parent_alone() {
        let mut d = doc("[idle]\nlock_after = 1\n");
        set(
            &mut d,
            &key("idle.battery", "suspend_after"),
            Value::from(900),
        )
        .unwrap();
        assert_eq!(
            d.to_string(),
            "[idle]\nlock_after = 1\n\n[idle.battery]\nsuspend_after = 900\n"
        );
        assert_eq!(
            get(&d, &key("idle.battery", "suspend_after")).and_then(Item::as_integer),
            Some(900)
        );
    }

    /// Comments parked above `[[keybinds]]` are about `[[outputs]]`, so removing the array carries
    /// them to the next table.
    #[test]
    fn remove_of_an_array_of_tables_carries_its_comments_on() {
        let text = "\
terminal = \"foot\"

# [[outputs]]                # per-output settings
# name = \"DP-1\"

[[keybinds]]
keys = \"Super+t\"
action = \"terminal\"

[[outputs]]
name = \"Virtual-1\"
";
        let mut d = doc(text);
        remove(&mut d, &key("", "keybinds"));
        assert_eq!(
            d.to_string(),
            "\
terminal = \"foot\"

# [[outputs]]                # per-output settings
# name = \"DP-1\"

[[outputs]]
name = \"Virtual-1\"
"
        );
    }

    #[test]
    fn remove_of_a_table_carries_its_comments_on() {
        let mut d = doc("# about power\n[idle]\nlock_after = 1\n\n[power]\nstandby = \"x\"\n");
        remove(&mut d, &key("", "idle"));
        assert_eq!(d.to_string(), "# about power\n\n[power]\nstandby = \"x\"\n");
    }

    #[test]
    fn remove_from_a_nested_table_keeps_the_rest() {
        let mut d =
            doc("[idle]\nlock_after = 1\n\n[idle.battery]\n# a note\nsuspend_after = 900\n");
        remove(&mut d, &key("idle.battery", "suspend_after"));
        assert!(get(&d, &key("idle.battery", "suspend_after")).is_none());
        assert_eq!(
            get(&d, &key("idle", "lock_after")).and_then(Item::as_integer),
            Some(1)
        );
        assert!(d.to_string().contains("# a note"), "{}", d);
    }

    #[test]
    fn an_empty_table_goes_only_when_no_comment_is_on_or_under_its_header() {
        let emptied = |text: &str, path: &'static str| {
            let mut d = doc(text);
            remove_table_if_empty(&mut d, path);
            d.to_string()
        };
        assert_eq!(emptied("[a]\n\n[b]\nx = 1\n", "a"), "\n[b]\nx = 1\n");
        assert_eq!(emptied("[a]\nx = 1\n[b]\n", "b"), "[a]\nx = 1\n");
        assert_eq!(emptied("[a]\ny = 1\n[a.b]\n", "a.b"), "[a]\ny = 1\n");
        for kept in [
            "[a] # about a\n\n[b]\nx = 1\n",
            "[a]\n# x = 2\n\n[b]\nx = 1\n",
            "[b]\nx = 1\n[a]\n# x = 2\n",
        ] {
            assert_eq!(emptied(kept, "a"), kept);
        }
        assert_eq!(
            emptied("# about a\n[a]\n\n[b]\nx = 1\n", "a"),
            "# about a\n\n[b]\nx = 1\n",
            "a comment above the header moves on"
        );
        let full = "[a]\nx = 1\n";
        assert_eq!(emptied(full, "a"), full, "a table with a key stays");
    }

    #[test]
    fn remove_carries_a_comment_to_the_next_key() {
        let mut d = doc("# about b\na = 1\nb = 2\n");
        remove(&mut d, &key("", "a"));
        assert_eq!(d.to_string(), "# about b\nb = 2\n");
    }
}
