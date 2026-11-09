//! Export and Import: the files in [`ENTRIES`] in one TOML bundle, each file's text under its
//! name, so a bundle carries Reprise's settings and remembered places to another account or machine.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, Value};

/// The table naming a bundle and its format; a TOML file without it is not one.
const HEADER: &str = "reprise_settings";
const VERSION: i64 = 1;
const FILES: &str = "files";

/// Which XDG directory a file lives under, in its `reprise` folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Root {
    Config,
    State,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    /// Every file directly inside the folder; Import replaces the folder's files whole.
    Folder,
}

/// One file or folder a bundle carries; `what` is the line the Backup page and Import show for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub root: Root,
    pub name: &'static str,
    pub kind: Kind,
    pub what: &'static str,
}

/// What a bundle carries. Left out on purpose: `state.toml` (the session's open programs and the
/// Run history), `location.toml` (location answers given on this machine), `trust.key` (a per-user
/// secret) and `volume.toml` (this machine's sound devices).
pub const ENTRIES: &[Entry] = &[
    Entry {
        root: Root::Config,
        name: "config.toml",
        kind: Kind::File,
        what: "Settings and keyboard shortcuts (config.toml)",
    },
    Entry {
        root: Root::Config,
        name: "terminal.toml",
        kind: Kind::File,
        what: "Terminal options",
    },
    Entry {
        root: Root::Config,
        name: "quicklaunch",
        kind: Kind::Folder,
        what: "Quick Launch buttons",
    },
    Entry {
        root: Root::State,
        name: "windows.toml",
        kind: Kind::File,
        what: "Where each program's windows open",
    },
    // Keyed by monitor id: a layout from another machine applies only to the same monitors.
    Entry {
        root: Root::State,
        name: "displays.toml",
        kind: Kind::File,
        what: "Display arrangements",
    },
    Entry {
        root: Root::State,
        name: "filedialog.toml",
        kind: Kind::File,
        what: "File dialog folders, views and sizes",
    },
    Entry {
        root: Root::State,
        name: "explorer.toml",
        kind: Kind::File,
        what: "Explorer view settings",
    },
];

/// The `reprise` folders under `$XDG_CONFIG_HOME` and `$XDG_STATE_HOME`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roots {
    pub config: PathBuf,
    pub state: PathBuf,
}

impl Roots {
    pub fn current() -> Roots {
        Roots {
            config: repriseconf::xdg_home("XDG_CONFIG_HOME", ".config").join("reprise"),
            state: repriseconf::xdg_home("XDG_STATE_HOME", ".local/state").join("reprise"),
        }
    }

    fn dir(&self, root: Root) -> &Path {
        match root {
            Root::Config => &self.config,
            Root::State => &self.state,
        }
    }

    fn path(&self, e: &Entry) -> PathBuf {
        self.dir(e.root).join(e.name)
    }

    /// Where Import keeps the files it replaced, as a bundle Import takes back.
    pub fn before_import(&self) -> PathBuf {
        self.config.join("reprise-settings-before-import.toml")
    }
}

/// The name Export suggests: `reprise-settings-YYYY-MM-DD.toml`, today's local date.
pub fn suggested_name(today: (i32, u32, u32)) -> String {
    let (y, m, d) = today;
    format!("reprise-settings-{y:04}-{m:02}-{d:02}.toml")
}

/// Today's local date, for [`suggested_name`].
pub fn today() -> (i32, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let tm = reprisefs::locale::localtime(secs);
    (tm.tm_year + 1900, tm.tm_mon as u32 + 1, tm.tm_mday as u32)
}

/// A file's key in a bundle: `config/config.toml`, `config/quicklaunch/x.desktop`.
fn key(e: &Entry, child: Option<&str>) -> String {
    let root = match e.root {
        Root::Config => "config",
        Root::State => "state",
    };
    match child {
        Some(c) => format!("{root}/{}/{c}", e.name),
        None => format!("{root}/{}", e.name),
    }
}

/// The entry and, inside a folder, the file name that `key` stands for; `None` for any key no
/// entry gives, so a bundle never names a path outside [`ENTRIES`].
fn entry_of(key_: &str) -> Option<(&'static Entry, Option<&str>)> {
    ENTRIES.iter().find_map(|e| {
        if key_ == key(e, None) && e.kind == Kind::File {
            return Some((e, None));
        }
        let child = key_.strip_prefix(&key(e, Some("")))?;
        let plain = !child.is_empty() && !child.contains('/') && child != "." && child != "..";
        (e.kind == Kind::Folder && plain).then_some((e, Some(child)))
    })
}

/// Why a file was not taken as a bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    NotToml(String),
    NotABundle,
    NewerVersion(i64),
    UnknownFile(String),
    BadConfig(String),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::NotToml(e) => write!(f, "The file is not a Reprise settings file: {e}"),
            Refused::NotABundle => write!(f, "The file is not a Reprise settings file."),
            Refused::NewerVersion(v) => write!(
                f,
                "The file was exported by a newer Reprise (format {v}); this one reads format {VERSION}."
            ),
            Refused::UnknownFile(k) => {
                write!(f, "The file holds {k}, which Reprise does not import.")
            }
            Refused::BadConfig(e) => write!(f, "The file's config.toml does not parse: {e}"),
        }
    }
}

/// The files of one export, by key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bundle {
    files: BTreeMap<String, String>,
}

impl Bundle {
    /// Read every entry under `roots`; a missing file or folder is left out, a file that is not
    /// UTF-8 text is skipped with a warning.
    pub fn gather(roots: &Roots) -> std::io::Result<Bundle> {
        let mut files = BTreeMap::new();
        let mut take = |path: &Path, k: String| -> std::io::Result<()> {
            match std::fs::read(path) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(text) => {
                        files.insert(k, text);
                    }
                    Err(_) => log::warn!("{} is not text; not exported", path.display()),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            Ok(())
        };
        for e in ENTRIES {
            let path = roots.path(e);
            match e.kind {
                Kind::File => take(&path, key(e, None))?,
                Kind::Folder => {
                    for name in folder_files(&path)? {
                        take(&path.join(&name), key(e, Some(&name)))?;
                    }
                }
            }
        }
        Ok(Bundle { files })
    }

    pub fn render(&self) -> String {
        let mut doc = DocumentMut::new();
        let mut header = Table::new();
        header.insert("version", Item::Value(Value::from(VERSION)));
        doc.insert(HEADER, Item::Table(header));
        let mut files = Table::new();
        for (k, text) in &self.files {
            files.insert(k, Item::Value(Value::from(text.as_str())));
        }
        doc.insert(FILES, Item::Table(files));
        doc.to_string()
    }

    pub fn parse(text: &str) -> Result<Bundle, Refused> {
        let doc = text
            .parse::<DocumentMut>()
            .map_err(|e| Refused::NotToml(e.message().to_string()))?;
        let version = doc
            .get(HEADER)
            .and_then(|h| h.get("version"))
            .and_then(Item::as_integer)
            .ok_or(Refused::NotABundle)?;
        if version > VERSION {
            return Err(Refused::NewerVersion(version));
        }
        let table = doc
            .get(FILES)
            .and_then(Item::as_table)
            .ok_or(Refused::NotABundle)?;
        let mut files = BTreeMap::new();
        for (k, item) in table.iter() {
            let text = item
                .as_str()
                .filter(|_| entry_of(k).is_some())
                .ok_or_else(|| Refused::UnknownFile(k.to_string()))?;
            files.insert(k.to_string(), text.to_string());
        }
        if let Some(config) = files.get(&key(&ENTRIES[0], None)) {
            config
                .parse::<DocumentMut>()
                .map_err(|e| Refused::BadConfig(e.message().to_string()))?;
        }
        Ok(Bundle { files })
    }

    /// The entries this bundle replaces, in [`ENTRIES`]' order.
    pub fn replaces(&self) -> Vec<&'static Entry> {
        ENTRIES
            .iter()
            .filter(|e| {
                self.files
                    .keys()
                    .any(|k| entry_of(k).is_some_and(|(o, _)| o == *e))
            })
            .collect()
    }

    /// Keep the files this bundle replaces in [`Roots::before_import`], then write its files: a
    /// folder entry loses the files the bundle does not hold. Returns where the old files went.
    pub fn import(&self, roots: &Roots) -> std::io::Result<PathBuf> {
        let replaced = self.replaces();
        let current = Bundle::gather(roots)?;
        let kept = Bundle {
            files: (current.files.into_iter())
                .filter(|(k, _)| entry_of(k).is_some_and(|(e, _)| replaced.contains(&e)))
                .collect(),
        };
        let backup = roots.before_import();
        std::fs::create_dir_all(&roots.config)?;
        repriseconf::write_atomic(&backup, &kept.render())?;
        for e in replaced {
            let path = roots.path(e);
            if e.kind == Kind::Folder {
                std::fs::create_dir_all(&path)?;
                for name in folder_files(&path)? {
                    if !self.files.contains_key(&key(e, Some(&name))) {
                        std::fs::remove_file(path.join(name))?;
                    }
                }
            }
        }
        for (k, text) in &self.files {
            let Some((e, child)) = entry_of(k) else {
                continue;
            };
            let path = match child {
                Some(c) => roots.path(e).join(c),
                None => roots.path(e),
            };
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            repriseconf::write_atomic(&path, text)?;
        }
        Ok(backup)
    }
}

/// The plain files directly in `dir`, by name; none when it is missing.
fn folder_files(dir: &Path) -> std::io::Result<Vec<String>> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut names = Vec::new();
    for entry in rd {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && let Some(name) = entry.file_name().to_str()
        {
            names.push(name.to_string());
        }
    }
    names.sort();
    Ok(names)
}

/// `reprise-settings --export FILE`: writes the bundle, an error line on failure.
pub fn export_to(path: &Path, roots: &Roots) -> Result<(), String> {
    let bundle = Bundle::gather(roots).map_err(|e| format!("Cannot read the settings: {e}."))?;
    repriseconf::write_atomic(path, &bundle.render())
        .map_err(|e| format!("Cannot write {}: {e}.", path.display()))
}

/// Read and check the bundle at `path`.
pub fn read(path: &Path) -> Result<Bundle, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("Cannot read {}: {e}.", path.display()))?;
    Bundle::parse(&text).map_err(|e| e.to_string())
}

/// Write `bundle`'s files under `roots` and ask the compositor to reread the remembered places;
/// the compositor rereads `config.toml` on its own. Returns where the replaced files went.
pub fn import(bundle: &Bundle, roots: &Roots) -> Result<PathBuf, String> {
    let backup = bundle
        .import(roots)
        .map_err(|e| format!("The settings could not be imported: {e}."))?;
    if let Err(e) = reprisebus::tray::reload_remembered() {
        log::warn!("ReloadRemembered: {e}");
    }
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(dir: &Path) -> Roots {
        Roots {
            config: dir.join("config/reprise"),
            state: dir.join("state/reprise"),
        }
    }

    fn put(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read_to(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn the_bundle_carries_settings_and_remembered_places_only() {
        let names: Vec<(Root, &str)> = ENTRIES.iter().map(|e| (e.root, e.name)).collect();
        assert_eq!(
            names,
            [
                (Root::Config, "config.toml"),
                (Root::Config, "terminal.toml"),
                (Root::Config, "quicklaunch"),
                (Root::State, "windows.toml"),
                (Root::State, "displays.toml"),
                (Root::State, "filedialog.toml"),
                (Root::State, "explorer.toml"),
            ]
        );
    }

    #[test]
    fn a_bundle_round_trips_every_file_it_gathered() {
        let a = tempfile::tempdir().unwrap();
        let from = roots(a.path());
        put(
            &from.config.join("config.toml"),
            "# mine\n[taskbar]\nauto_hide = true\n",
        );
        put(
            &from.config.join("quicklaunch/a.desktop"),
            "[Desktop Entry]\nName=A\n",
        );
        put(&from.config.join("quicklaunch/.order"), "a.desktop\n");
        put(&from.state.join("windows.toml"), "[\"set\".app]\nx = 1\n");
        put(
            &from.state.join("state.toml"),
            "run_history = [\"secret\"]\n",
        );
        put(&from.config.join("other.toml"), "x = 1\n");
        let bundle = Bundle::gather(&from).unwrap();
        let text = bundle.render();
        assert!(!text.contains("secret"), "state.toml stays home: {text}");
        assert!(!text.contains("other.toml"));
        let back = Bundle::parse(&text).unwrap();
        assert_eq!(back, bundle);
        let b = tempfile::tempdir().unwrap();
        let to = roots(b.path());
        back.import(&to).unwrap();
        assert_eq!(
            read_to(&to.config.join("config.toml")),
            "# mine\n[taskbar]\nauto_hide = true\n"
        );
        assert_eq!(
            read_to(&to.config.join("quicklaunch/.order")),
            "a.desktop\n"
        );
        assert_eq!(
            read_to(&to.state.join("windows.toml")),
            "[\"set\".app]\nx = 1\n"
        );
        assert!(!to.state.join("state.toml").exists());
        let names: Vec<&str> = back.replaces().iter().map(|e| e.name).collect();
        assert_eq!(names, ["config.toml", "quicklaunch", "windows.toml"]);
    }

    #[test]
    fn import_keeps_the_files_it_replaces_as_a_bundle() {
        let a = tempfile::tempdir().unwrap();
        let r = roots(a.path());
        put(
            &r.config.join("config.toml"),
            "[taskbar]\nauto_hide = false\n",
        );
        put(&r.config.join("quicklaunch/old.desktop"), "old\n");
        put(&r.state.join("explorer.toml"), "view = \"list\"\n");
        let mut incoming = Bundle::default();
        incoming.files.insert(
            "config/config.toml".into(),
            "[taskbar]\nauto_hide = true\n".into(),
        );
        incoming
            .files
            .insert("config/quicklaunch/new.desktop".into(), "new\n".into());
        let backup = incoming.import(&r).unwrap();
        assert_eq!(backup, r.config.join("reprise-settings-before-import.toml"));
        assert_eq!(
            read_to(&r.config.join("config.toml")),
            "[taskbar]\nauto_hide = true\n"
        );
        assert!(
            !r.config.join("quicklaunch/old.desktop").exists(),
            "the folder is replaced"
        );
        assert_eq!(read_to(&r.config.join("quicklaunch/new.desktop")), "new\n");
        assert_eq!(read_to(&r.state.join("explorer.toml")), "view = \"list\"\n");
        let kept = Bundle::parse(&read_to(&backup)).unwrap();
        assert_eq!(
            kept.files.keys().collect::<Vec<_>>(),
            ["config/config.toml", "config/quicklaunch/old.desktop"],
            "only what was replaced, not explorer.toml"
        );
        kept.import(&r).unwrap();
        assert_eq!(
            read_to(&r.config.join("config.toml")),
            "[taskbar]\nauto_hide = false\n"
        );
        assert!(r.config.join("quicklaunch/old.desktop").exists());
        assert!(!r.config.join("quicklaunch/new.desktop").exists());
    }

    #[test]
    fn a_corrupt_or_foreign_file_is_refused() {
        assert!(matches!(
            Bundle::parse("not = [toml"),
            Err(Refused::NotToml(_))
        ));
        assert_eq!(
            Bundle::parse("[taskbar]\nauto_hide = true\n"),
            Err(Refused::NotABundle)
        );
        assert_eq!(
            Bundle::parse("[reprise_settings]\nversion = 2\n[files]\n"),
            Err(Refused::NewerVersion(2))
        );
        for k in [
            "config/../../.bashrc",
            "config/quicklaunch/../config.toml",
            "config/quicklaunch/a/b.desktop",
            "config/windows.toml",
            "state/state.toml",
            "config/config.toml/x",
        ] {
            let text = format!("[reprise_settings]\nversion = 1\n[files]\n{k:?} = \"x\"\n");
            assert_eq!(
                Bundle::parse(&text),
                Err(Refused::UnknownFile(k.into())),
                "{k}"
            );
        }
        let bad = "[reprise_settings]\nversion = 1\n[files]\n\"config/config.toml\" = \"[x\"\n";
        assert!(matches!(Bundle::parse(bad), Err(Refused::BadConfig(_))));
    }

    #[test]
    fn export_suggests_a_dated_name() {
        assert_eq!(
            suggested_name((2026, 3, 7)),
            "reprise-settings-2026-03-07.toml"
        );
    }
}
