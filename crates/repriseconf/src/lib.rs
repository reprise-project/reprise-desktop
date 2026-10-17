//! Editing `config.toml` in place: `toml_edit` keeps comments, key order and unknown tables, so a
//! program editing a few keys leaves the rest of the file byte-identical.

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

static CONFIG_PATH: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Points [`config_path`] at `path` for the whole process, so a program's tests never read the
/// developer's own file.
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
    xdg_home("XDG_CONFIG_HOME", ".config")
        .join("reprise")
        .join("config.toml")
}

/// Whether `[privacy] recent_files` lets Reprise show recently used files (true unless `false`),
/// read now.
pub fn recent_files() -> bool {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    recent_files_in(&text)
}

fn recent_files_in(text: &str) -> bool {
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return true;
    };
    get(&doc, &key("privacy", "recent_files"))
        .and_then(Item::as_bool)
        .unwrap_or(true)
}

/// `[glass] color` as written (a colour's name or "#RRGGBB"), read now; `None` when unset.
pub fn glass_color() -> Option<String> {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    let doc = text.parse::<DocumentMut>().ok()?;
    get(&doc, &key("glass", "color"))
        .and_then(Item::as_str)
        .map(String::from)
}

pub fn shell_dlg() -> Option<String> {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    shell_dlg_in(&text)
}

fn shell_dlg_in(text: &str) -> Option<String> {
    let doc = text.parse::<DocumentMut>().ok()?;
    get(&doc, &key("fonts", "shell_dlg"))
        .and_then(Item::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(String::from)
}

/// `[fonts] family`, read now; `None` when unset, empty or not a list of strings.
pub fn font_family() -> Option<Vec<String>> {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    font_family_in(&text)
}

fn font_family_in(text: &str) -> Option<Vec<String>> {
    let doc = text.parse::<DocumentMut>().ok()?;
    let list = get(&doc, &key("fonts", "family"))?.as_array()?;
    let families: Option<Vec<String>> = list.iter().map(|v| v.as_str().map(String::from)).collect();
    families.filter(|f| !f.is_empty())
}

pub fn text_size() -> u32 {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    text_size_in(&text)
}

fn text_size_in(text: &str) -> u32 {
    text.parse::<DocumentMut>()
        .ok()
        .and_then(|doc| get(&doc, &key("accessibility", "text_size"))?.as_integer())
        .and_then(|v| u32::try_from(v).ok())
        .filter(|v| (100..=200).contains(v) && v % 25 == 0)
        .unwrap_or(100)
}

/// `[metrics] double_click_ms`, read now; `None` when unset or outside 0..=10000, which leaves the
/// interface's own time.
pub fn double_click_ms() -> Option<i32> {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    double_click_ms_in(&text)
}

fn double_click_ms_in(text: &str) -> Option<i32> {
    let doc = text.parse::<DocumentMut>().ok()?;
    get(&doc, &key("metrics", "double_click_ms"))?
        .as_integer()
        .and_then(|v| i32::try_from(v).ok())
        .filter(|v| (0..=10_000).contains(v))
}

/// `[taskbar] position`, read now: "bottom" when unset, not a string or unparsable. Tray applets
/// ask each time a popup opens.
pub fn taskbar_position() -> String {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    taskbar_position_in(&text)
}

fn taskbar_position_in(text: &str) -> String {
    text.parse::<DocumentMut>()
        .ok()
        .and_then(|doc| {
            get(&doc, &key("taskbar", "position"))
                .and_then(Item::as_str)
                .map(String::from)
        })
        .unwrap_or_else(|| "bottom".into())
}

/// `[drives] on_insert`, read now: what reprise-drives does with a plugged-in drive, "ask" when unset
/// or not a string.
pub fn drives_on_insert() -> String {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    drives_on_insert_in(&text)
}

fn drives_on_insert_in(text: &str) -> String {
    text.parse::<DocumentMut>()
        .ok()
        .and_then(|doc| {
            get(&doc, &key("drives", "on_insert"))
                .and_then(Item::as_str)
                .map(String::from)
        })
        .unwrap_or_else(|| "ask".into())
}

/// The top-level `terminal`, read now: the command a program in a terminal runs under.
pub fn terminal() -> String {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    terminal_in(&text)
}

fn terminal_in(text: &str) -> String {
    let doc = text.parse::<DocumentMut>().ok();
    doc.as_ref()
        .and_then(|d| get(d, &key("", "terminal"))?.as_str())
        .unwrap_or(reprisefs::apps::DEFAULT_TERMINAL)
        .to_string()
}

/// The top-level `script_terminal`, read now: "stay" when unset or invalid.
pub fn script_terminal() -> reprisefs::program::ScriptEnd {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    script_terminal_in(&text)
}

fn script_terminal_in(text: &str) -> reprisefs::program::ScriptEnd {
    text.parse::<DocumentMut>()
        .ok()
        .and_then(|d| {
            get(&d, &key("", "script_terminal"))?
                .as_str()
                .and_then(reprisefs::program::ScriptEnd::parse)
        })
        .unwrap_or_default()
}

/// `[thumbnail_cache]`: whether pictures' thumbnails are kept in `$XDG_CACHE_HOME/thumbnails`,
/// shared with other file managers, and how much room they may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThumbnailCache {
    pub enabled: bool,
    /// The cap in MiB; past it the least recently used thumbnails go, whichever program wrote them.
    pub max_mb: u32,
}

impl ThumbnailCache {
    pub const DEFAULT: ThumbnailCache = ThumbnailCache {
        enabled: true,
        max_mb: 512,
    };
    /// A thumbnail is at most 64 KiB of pixels at scale 1, 256 KiB at scale 2 and 1 MiB
    /// at its largest.
    pub const MAX_MB: std::ops::RangeInclusive<u32> = 1..=4096;

    pub const fn max_bytes(self) -> u64 {
        (self.max_mb as u64) << 20
    }
}

/// `[thumbnail_cache]`, read now: a key unset or not a bool or integer keeps its default, and
/// `max_mb` is clamped to [`ThumbnailCache::MAX_MB`].
pub fn thumbnail_cache() -> ThumbnailCache {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    thumbnail_cache_in(&text)
}

fn thumbnail_cache_in(text: &str) -> ThumbnailCache {
    let d = ThumbnailCache::DEFAULT;
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return d;
    };
    let mb = ThumbnailCache::MAX_MB;
    ThumbnailCache {
        enabled: get(&doc, &key("thumbnail_cache", "enabled"))
            .and_then(Item::as_bool)
            .unwrap_or(d.enabled),
        max_mb: get(&doc, &key("thumbnail_cache", "max_mb"))
            .and_then(Item::as_integer)
            .map_or(d.max_mb, |v| {
                v.clamp(i64::from(*mb.start()), i64::from(*mb.end())) as u32
            }),
    }
}

pub fn xdg_home(var: &str, fallback: &str) -> PathBuf {
    xdg_home_from(std::env::var_os(var), std::env::var_os("HOME"), fallback)
}

fn xdg_home_from(value: Option<OsString>, home: Option<OsString>, fallback: &str) -> PathBuf {
    value
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home.map(|h| PathBuf::from(h).join(fallback)))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Write `text` to `path` with `reprisefs::ops::write_atomic`; the compositor's watch sees the
/// rename as `MOVED_TO`/`CLOSE_WRITE`.
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    reprisefs::ops::write_atomic(path, text.as_bytes())
}

/// Read `path` (absent is empty), hand its text to `edit`, write the result atomically, under an
/// exclusive `flock` on a `.lock` file beside it (the target's inode is replaced by the rename).
/// `edit`'s error comes back as `Ok(Err)` with nothing written; unchanged text is not written.
pub fn edit<E>(
    path: &Path,
    edit: impl FnOnce(&str) -> Result<String, E>,
) -> std::io::Result<Result<String, E>> {
    use std::os::fd::AsRawFd;
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    std::fs::create_dir_all(&dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "out".into());
    let lock = std::fs::File::create(dir.join(format!(".{name}.lock")))?;
    // SAFETY: a valid fd from the open file above; LOCK_EX blocks until
    // the other editor is done and the lock goes with the fd on drop.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let out = match edit(&text) {
        Ok(out) => out,
        Err(e) => return Ok(Err(e)),
    };
    if out != text {
        write_atomic(path, &out)?;
    }
    Ok(Ok(out))
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
    let Some(next) = next else {
        if name.is_empty() || parent.is_empty() {
            let old = doc.trailing().as_str().unwrap_or_default().to_string();
            doc.set_trailing(format!("{prefix}{old}"));
        } else {
            carry_past_table(doc, parent, prefix);
        }
        return;
    };
    if let Some(pt) = table_of_mut(doc, parent) {
        prepend_entry_prefix(pt, &next, prefix);
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

pub mod glob;
pub mod keybind;
pub mod location;
pub mod pages;

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

    #[test]
    fn recent_files_is_on_unless_privacy_turns_it_off() {
        assert!(recent_files_in(""));
        assert!(recent_files_in("[privacy]\n"));
        assert!(!recent_files_in("[privacy]\nrecent_files = false\n"));
        assert!(recent_files_in("[privacy]\nrecent_files = true\n"));
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
    fn the_second_dialog_font_is_read_when_set() {
        assert_eq!(shell_dlg_in(""), None);
        assert_eq!(shell_dlg_in("[fonts]\nshell_dlg = \" \"\n"), None);
        assert_eq!(shell_dlg_in("[fonts]\nshell_dlg = 3\n"), None);
        assert_eq!(shell_dlg_in("[fonts\n"), None);
        assert_eq!(
            shell_dlg_in("[fonts]\nshell_dlg = \"Some Sans\"\n").as_deref(),
            Some("Some Sans")
        );
    }

    #[test]
    fn the_font_family_is_read_when_a_list_of_names() {
        assert_eq!(font_family_in(""), None);
        assert_eq!(font_family_in("[fonts]\nfamily = []\n"), None);
        assert_eq!(font_family_in("[fonts]\nfamily = \"DejaVu Sans\"\n"), None);
        assert_eq!(font_family_in("[fonts]\nfamily = [\"A\", 3]\n"), None);
        assert_eq!(
            font_family_in("[fonts]\nfamily = [\"DejaVu Sans\", \"Noto Sans\"]\n"),
            Some(vec!["DejaVu Sans".into(), "Noto Sans".into()])
        );
    }

    #[test]
    fn the_text_size_is_read_or_100() {
        assert_eq!(text_size_in(""), 100);
        assert_eq!(text_size_in("[accessibility]\ntext_size = 150\n"), 150);
        assert_eq!(text_size_in("[accessibility]\ntext_size = 140\n"), 100);
        assert_eq!(text_size_in("[accessibility]\ntext_size = \"150\"\n"), 100);
    }

    #[test]
    fn the_double_click_time_is_read_or_left_to_the_interface() {
        assert_eq!(double_click_ms_in(""), None);
        assert_eq!(
            double_click_ms_in("[metrics]\ndouble_click_ms = 900\n"),
            Some(900)
        );
        assert_eq!(
            double_click_ms_in("[metrics]\ndouble_click_ms = -1\n"),
            None
        );
        assert_eq!(
            double_click_ms_in("[metrics]\ndouble_click_ms = \"900\"\n"),
            None
        );
    }

    #[test]
    fn the_drive_insert_mode_is_read_or_ask() {
        assert_eq!(drives_on_insert_in(""), "ask");
        assert_eq!(
            drives_on_insert_in("[drives]\non_insert = \"open\"\n"),
            "open"
        );
        assert_eq!(drives_on_insert_in("[drives]\non_insert = true\n"), "ask");
    }

    #[test]
    fn a_script_runs_in_the_terminal_key_and_stays_unless_set_to_close() {
        use reprisefs::program::ScriptEnd;
        assert_eq!(terminal_in(""), "alacritty");
        assert_eq!(script_terminal_in(""), ScriptEnd::Stay);
        let text = "terminal = \"foot -o x=1\"\nscript_terminal = \"close\"\n[drives]\nterminal = \"st\"\n";
        assert_eq!(terminal_in(text), "foot -o x=1");
        assert_eq!(script_terminal_in(text), ScriptEnd::Close);
        assert_eq!(
            terminal_in("[drives]\nterminal = \"st\"\n"),
            "alacritty",
            "top-level keys only"
        );
        assert_eq!(
            script_terminal_in("script_terminal = \"later\"\n"),
            ScriptEnd::Stay
        );
    }

    #[test]
    fn the_thumbnail_cache_is_on_at_512_mb_unless_set() {
        assert_eq!(thumbnail_cache_in(""), ThumbnailCache::DEFAULT);
        assert_eq!(
            thumbnail_cache_in("[thumbnail_cache]\nenabled = false\n"),
            ThumbnailCache {
                enabled: false,
                max_mb: 512
            }
        );
        let cap =
            |v: &str| thumbnail_cache_in(&format!("[thumbnail_cache]\nmax_mb = {v}\n")).max_mb;
        assert_eq!(cap("64"), 64);
        assert_eq!(cap("0"), 1);
        assert_eq!(cap("-5"), 1);
        assert_eq!(cap("100000"), 4096);
        assert_eq!(cap("\"big\""), 512);
        assert_eq!(ThumbnailCache::DEFAULT.max_bytes(), 512 << 20);
    }

    #[test]
    fn the_taskbar_position_is_read_or_bottom() {
        assert_eq!(taskbar_position_in(""), "bottom");
        assert_eq!(
            taskbar_position_in("[taskbar]\nposition = \"top\"\n"),
            "top"
        );
        assert_eq!(taskbar_position_in("[taskbar]\nposition = 1\n"), "bottom");
        assert_eq!(
            taskbar_position_in("[taskbar]\nposition = \"left\"\nside_width = 90\n"),
            "left"
        );
        assert_eq!(
            taskbar_position_in("[taskbar\nposition = \"top\"\n"),
            "bottom",
            "a file that does not parse"
        );
    }

    #[test]
    fn xdg_home_ignores_an_empty_or_relative_value() {
        let home = || Some(OsString::from("/home/u"));
        let pick = |v: &str| xdg_home_from(Some(v.into()), home(), ".local/state");
        assert_eq!(pick("/x/state"), PathBuf::from("/x/state"));
        assert_eq!(pick(""), PathBuf::from("/home/u/.local/state"));
        assert_eq!(pick("rel/state"), PathBuf::from("/home/u/.local/state"));
        assert_eq!(
            xdg_home_from(None, home(), ".config"),
            PathBuf::from("/home/u/.config")
        );
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
    fn remove_carries_a_comment_to_the_next_key() {
        let mut d = doc("# about b\na = 1\nb = 2\n");
        remove(&mut d, &key("", "a"));
        assert_eq!(d.to_string(), "# about b\nb = 2\n");
    }
}
