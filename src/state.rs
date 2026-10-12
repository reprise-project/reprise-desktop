//! What survives a restart without living in the config: one small TOML file under
//! `$XDG_STATE_HOME/reprise`. Every field is optional, and a save rewrites only the keys it owns,
//! leaving the rest of the file as it is.

use crate::taskbar::Bands;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Persisted {
    /// Absent means locked, the default.
    pub taskbar_locked: Option<bool>,
    /// The Quick Launch order an older session kept here, read once into the folder's own order
    /// file (`reprisefs::quicklaunch::ORDER`) and no longer written.
    pub quicklaunch: Vec<String>,
    /// Which taskbar band comes first and how wide it is, once dragged.
    pub bands: Bands,
    /// Toolbars > Quick Launch; absent means shown.
    pub quicklaunch_shown: Option<bool>,
    /// The desktop menu's Auto Arrange; off by default.
    pub auto_arrange: bool,
    /// Desktop icon file name (a system icon's default label) to its grid cell (column, row), for
    /// icons that were dragged somewhere; the rest fill in column-major.
    pub desktop_positions: BTreeMap<String, (i32, i32)>,
    /// Commands typed into Run, newest first.
    pub run_history: Vec<String>,
    /// A place icon's label after an F2 rename, by its `system_icons` name; absent means the
    /// default label.
    pub system_icon_names: BTreeMap<String, String>,
    /// Each app's last normal frame, by app id (`placement`).
    pub window_geometry: BTreeMap<String, crate::windowrules::Remembered>,
    /// Recycle Bin Properties as an older session kept it here: moved once into `[recycle_bin]` in
    /// `config.toml`, then dropped (`dialogview::setup`).
    pub bin: Option<crate::binprops::OldBinSettings>,
    /// The apps open when the last session ended, started again at login.
    pub session: Vec<crate::session::SavedApp>,
    /// The tab groups open when the last session ended, regrouped at login.
    pub session_tabs: Vec<crate::session::SavedTabs>,
    /// The programs pinned to the Glass Start menu, by desktop file id.
    pub start_pins: Vec<String>,
    /// Launched programs' desktop file ids, newest first.
    pub launches: Vec<String>,
    /// The tray Customize dialog's choices, by tray item key.
    pub tray_icons: BTreeMap<String, crate::tray::Remembered>,
    /// The colour filter `color-filter` last turned off, by its config name: the one it turns back
    /// on.
    pub color_filter_last: Option<String>,
}

/// `$XDG_STATE_HOME/reprise/state.toml`.
pub fn path() -> PathBuf {
    repriseconf::xdg_home("XDG_STATE_HOME", ".local/state")
        .join("reprise")
        .join("state.toml")
}

/// Read the file. A key that does not parse falls back to its default and the file is copied to
/// `state.toml.bad`.
pub fn load() -> Persisted {
    load_from(&path())
}

pub fn load_from(path: &Path) -> Persisted {
    let mut p = read_from(path);
    p.rename_system_icons();
    p
}

impl Persisted {
    fn rename_system_icons(&mut self) {
        let names = std::mem::take(&mut self.system_icon_names);
        for (name, label) in names {
            if let Some(name) = repriseconf::system_icons::current(&name) {
                self.system_icon_names.insert(name.to_string(), label);
            }
        }
        for (old, new) in [
            ("My Documents", Some("Documents")),
            ("My Computer", Some("Computer")),
            ("My Network Places", None),
        ] {
            if let Some(cell) = self.desktop_positions.remove(old)
                && let Some(new) = new
            {
                self.desktop_positions
                    .entry(new.to_string())
                    .or_insert(cell);
            }
        }
    }
}

fn read_from(path: &Path) -> Persisted {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Persisted::default();
    };
    let (p, dropped) = match parse_lenient(&text) {
        Ok((p, dropped)) if dropped.is_empty() => return p,
        Ok((p, dropped)) => (p, dropped),
        Err(e) => (Persisted::default(), vec![("the file".to_string(), e)]),
    };
    for (key, e) in &dropped {
        log::warn!("state {}: {key}: {e}; using its default", path.display());
    }
    // A copy rather than a rename: `load` runs more than once per login.
    let bad = path.with_extension("toml.bad");
    if let Err(e) = std::fs::copy(path, &bad) {
        log::warn!("state: could not keep {}: {e}", bad.display());
    }
    p
}

/// Every key that fails on its own (top-level, or one inside a table) is left at its default and
/// returned with its error; Err only when `text` is not TOML.
fn parse_lenient(text: &str) -> Result<(Persisted, Vec<(String, String)>), String> {
    use toml::{Table, Value};
    let mut table: Table = toml::from_str(text).map_err(|e| e.to_string())?;
    let parse = |t: Table| -> Result<Persisted, String> {
        t.try_into()
            .map_err(|e: toml::de::Error| e.to_string().trim_end().to_string())
    };
    let alone = |key: &str, value: Value| parse(Table::from_iter([(key.to_string(), value)]));
    let mut dropped = Vec::new();
    for (key, value) in table.iter_mut() {
        let Err(e) = alone(key, value.clone()) else {
            continue;
        };
        match value {
            Value::Table(inner) => inner.retain(|k, v| {
                let one = Value::Table(Table::from_iter([(k.to_string(), v.clone())]));
                let kept = alone(key, one);
                if let Err(e) = &kept {
                    dropped.push((format!("{key}.{k}"), e.clone()));
                }
                kept.is_ok()
            }),
            _ => dropped.push((key.clone(), e)),
        }
    }
    table.retain(|key, _| !dropped.iter().any(|(k, _)| k == key));
    Ok((parse(table)?, dropped))
}

/// Queue a write of the desktop's keys (`auto_arrange`, `desktop_positions`) into the file, leaving
/// every other key as it is.
pub fn save_desktop(auto_arrange: bool, positions: &BTreeMap<String, (i32, i32)>) {
    let path = path();
    let keys = desktop_keys(auto_arrange, positions);
    run_in_order(move || patch(&path, keys));
}

/// Queue a write of `window_geometry` into the file, leaving every other key as it is.
pub fn save_window_geometry(geometry: &BTreeMap<String, crate::windowrules::Remembered>) {
    let path = path();
    let keys = window_geometry_keys(geometry);
    run_in_order(move || patch(&path, keys));
}

fn window_geometry_keys(
    geometry: &BTreeMap<String, crate::windowrules::Remembered>,
) -> toml::Table {
    match toml::Table::try_from(geometry) {
        Ok(t) => toml::Table::from_iter([("window_geometry".to_string(), t.into())]),
        Err(e) => {
            log::warn!("state: could not serialize the window geometry: {e}");
            toml::Table::new()
        }
    }
}

/// Queue a write of the apps and tab groups the session ended with (`session`, `session_tabs`),
/// leaving every other key as it is.
pub fn save_session(apps: &[crate::session::SavedApp], tabs: &[crate::session::SavedTabs]) {
    let path = path();
    let keys = session_keys(apps, tabs);
    run_in_order(move || patch(&path, keys));
}

/// Queue a write of `color_filter_last`, leaving every other key as it is.
pub fn save_color_filter_last(name: &str) {
    let path = path();
    let keys = toml::Table::from_iter([("color_filter_last".to_string(), name.into())]);
    run_in_order(move || patch(&path, keys));
}

fn session_keys(
    apps: &[crate::session::SavedApp],
    tabs: &[crate::session::SavedTabs],
) -> toml::Table {
    match (toml::Value::try_from(apps), toml::Value::try_from(tabs)) {
        (Ok(a), Ok(t)) => {
            toml::Table::from_iter([("session".to_string(), a), ("session_tabs".to_string(), t)])
        }
        (Err(e), _) | (_, Err(e)) => {
            log::warn!("state: could not serialize the session: {e}");
            toml::Table::new()
        }
    }
}

/// Queue a write of the dialogs' key (`run_history`) into the file, leaving every other key as it
/// is.
pub fn save_dialogs(run_history: &[String]) {
    let path = path();
    let history = toml::Value::Array(run_history.iter().map(|c| c.as_str().into()).collect());
    let keys = toml::Table::from_iter([("run_history".to_string(), history)]);
    run_in_order(move || patch(&path, keys));
}

/// Queue taking the top-level key `name` out of the file, leaving every other key as it is: one
/// this build no longer keeps here.
pub fn drop_key(name: &'static str) {
    let path = path();
    run_in_order(move || unpatch(&path, name));
}

/// Take `name` out of the file at `path`; a file without it, or one that does not parse, is left
/// alone.
fn unpatch(path: &Path, name: &str) {
    let Some(mut table) = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| toml::from_str::<toml::Table>(&t).ok())
    else {
        return;
    };
    if table.remove(name).is_none() {
        return;
    }
    let text = match toml::to_string(&table) {
        Ok(t) => t,
        Err(e) => return log::warn!("state: could not serialize: {e}"),
    };
    if let Err(e) = repriseconf::write_atomic(path, &text) {
        log::warn!("state: could not write {}: {e}", path.display());
    }
}

/// Queue a write of the Glass Start menu's pins and launch history, leaving every other key as it
/// is.
pub fn save_start(l: &crate::launches::Launches) {
    let path = path();
    let keys = start_keys(l);
    run_in_order(move || patch(&path, keys));
}

fn start_keys(l: &crate::launches::Launches) -> toml::Table {
    let list = |v: &[String]| toml::Value::Array(v.iter().map(|s| s.as_str().into()).collect());
    toml::Table::from_iter([
        ("start_pins".to_string(), list(&l.pins)),
        ("launches".to_string(), list(&l.recent)),
    ])
}

/// Queue a write of the tray's choices (`tray_icons`), leaving every other key as it is.
pub fn save_tray_icons(choices: &BTreeMap<String, crate::tray::Remembered>) {
    let path = path();
    let keys = tray_icons_keys(choices);
    run_in_order(move || patch(&path, keys));
}

fn tray_icons_keys(choices: &BTreeMap<String, crate::tray::Remembered>) -> toml::Table {
    match toml::Table::try_from(choices) {
        Ok(t) => toml::Table::from_iter([("tray_icons".to_string(), t.into())]),
        Err(e) => {
            log::warn!("state: could not serialize the tray choices: {e}");
            toml::Table::new()
        }
    }
}

/// Queue a write of the place icons' renamed labels (`system_icon_names`), leaving every other key
/// as it is.
pub fn save_system_names(names: &BTreeMap<String, String>) {
    let path = path();
    let keys = system_names_keys(names);
    run_in_order(move || patch(&path, keys));
}

fn system_names_keys(names: &BTreeMap<String, String>) -> toml::Table {
    let names: toml::Table = names
        .iter()
        .map(|(k, v)| (k.clone(), toml::Value::String(v.clone())))
        .collect();
    toml::Table::from_iter([("system_icon_names".to_string(), names.into())])
}

fn desktop_keys(auto_arrange: bool, positions: &BTreeMap<String, (i32, i32)>) -> toml::Table {
    let cells: toml::Table = positions
        .iter()
        .map(|(name, &(col, row))| {
            let cell = toml::Value::Array(vec![col.into(), row.into()]);
            (name.clone(), cell)
        })
        .collect();
    toml::Table::from_iter([
        ("auto_arrange".to_string(), auto_arrange.into()),
        ("desktop_positions".to_string(), cells.into()),
    ])
}

/// Queue a write of the taskbar's keys (`taskbar_locked`, `bands`, `quicklaunch_shown`) into the
/// file, leaving every other key as it is.
pub fn save_taskbar(locked: bool, bands: Bands, quick_shown: bool) {
    let path = path();
    let keys = taskbar_keys(locked, bands, quick_shown);
    run_in_order(move || patch(&path, keys));
}

fn taskbar_keys(locked: bool, bands: Bands, quick_shown: bool) -> toml::Table {
    let bands = toml::Value::try_from(bands).unwrap_or_else(|e| {
        log::warn!("state: could not serialize the bands: {e}");
        toml::Table::new().into()
    });
    toml::Table::from_iter([
        ("taskbar_locked".to_string(), locked.into()),
        ("bands".to_string(), bands),
        ("quicklaunch_shown".to_string(), quick_shown.into()),
    ])
}

/// Read `path` (nothing when missing), set `keys` and write it back atomically. A file that is not
/// TOML is replaced, as the next load would read it as the defaults.
fn patch(path: &Path, keys: toml::Table) {
    let mut table: toml::Table = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default();
    table.extend(keys);
    let text = match toml::to_string(&table) {
        Ok(t) => t,
        Err(e) => return log::warn!("state: could not serialize: {e}"),
    };
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        log::warn!("state: could not create {}: {e}", dir.display());
    }
    if let Err(e) = repriseconf::write_atomic(path, &text) {
        log::warn!("state: could not write {}: {e}", path.display());
    }
}

enum Job {
    Run(Box<dyn FnOnce() + Send>),
    Flush(Sender<()>),
}

static WRITER: OnceLock<Option<Sender<Job>>> = OnceLock::new();

/// Run `work` (a small file write that fsyncs) on the writer thread, after the work queued before
/// it, so it does not stall the event loop.
pub fn run_in_order(work: impl FnOnce() + Send + 'static) {
    queue(Job::Run(Box::new(work)));
}

/// Block until every write queued so far is on disk: before the compositor exits.
pub fn flush() {
    let (tx, rx) = channel();
    queue(Job::Flush(tx));
    let _ = rx.recv();
}

fn queue(job: Job) {
    let writer = WRITER.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        let spawned = std::thread::Builder::new()
            .name("statewrite".into())
            .spawn(move || rx.into_iter().for_each(run));
        match spawned {
            Ok(_) => Some(tx),
            Err(e) => {
                log::warn!("state: cannot spawn a thread ({e}); saves run on the loop");
                None
            }
        }
    });
    match writer {
        Some(tx) => {
            // The send fails only once the writer has died; then the caller runs it.
            if let Err(e) = tx.send(job) {
                run(e.0);
            }
        }
        None => run(job),
    }
}

fn run(job: Job) {
    match job {
        Job::Run(work) => work(),
        Job::Flush(done) => {
            let _ = done.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reprise-state-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn load_drops_only_the_keys_that_fail_and_keeps_the_file_as_bad() {
        let dir = scratch("bad");
        let path = dir.join("state.toml");
        let text = "auto_arrange = \"yes\"\n\
                    [desktop_positions]\n\"Reports\" = [2, 1]\n\"Notes\" = \"oops\"\n";
        std::fs::write(&path, text).unwrap();
        let p = load_from(&path);
        assert_eq!(
            p.desktop_positions,
            BTreeMap::from([("Reports".to_string(), (2, 1))])
        );
        assert!(!p.auto_arrange);
        assert_eq!(
            std::fs::read_to_string(dir.join("state.toml.bad")).unwrap(),
            text
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_older_files_place_icons_load_under_their_current_names() {
        let mut p = Persisted {
            system_icon_names: BTreeMap::from([
                ("my-documents".to_string(), "Docs".to_string()),
                ("my-network-places".to_string(), "LAN".to_string()),
            ]),
            desktop_positions: BTreeMap::from([
                ("My Computer".to_string(), (0, 2)),
                ("My Network Places".to_string(), (0, 3)),
                ("My Documents".to_string(), (1, 0)),
                ("Documents".to_string(), (4, 4)),
            ]),
            ..Persisted::default()
        };
        p.rename_system_icons();
        assert_eq!(
            p.system_icon_names,
            BTreeMap::from([("documents".to_string(), "Docs".to_string())])
        );
        assert_eq!(
            p.desktop_positions,
            BTreeMap::from([
                ("Computer".to_string(), (0, 2)),
                ("Documents".to_string(), (4, 4)),
            ]),
            "a cell already kept under the new label stays"
        );
    }

    #[test]
    fn a_window_geometry_save_round_trips_and_keeps_the_desktop() {
        let dir = scratch("geometry");
        let path = dir.join("state.toml");
        std::fs::write(&path, "auto_arrange = true\n").unwrap();
        let geometry = BTreeMap::from([(
            "org.example.App".to_string(),
            crate::windowrules::Remembered {
                output: "DP-1".into(),
                x: -4,
                y: 30,
                width: 640,
                height: 480,
                maximized: true,
            },
        )]);
        let p = path.clone();
        let g = geometry.clone();
        run_in_order(move || patch(&p, window_geometry_keys(&g)));
        flush();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("[window_geometry.\"org.example.App\"]"),
            "{text}"
        );
        let back = load_from(&path);
        assert_eq!(back.window_geometry, geometry);
        assert!(back.auto_arrange);
    }

    #[test]
    fn the_tray_choices_round_trip_and_keep_the_desktop() {
        use reprisekit::controls::traycustomize::Behaviour;
        let dir = scratch("tray");
        let path = dir.join("state.toml");
        std::fs::write(&path, "auto_arrange = true\n").unwrap();
        let choices = BTreeMap::from([
            (
                "nm-applet".to_string(),
                crate::tray::Remembered {
                    behaviour: Behaviour::Hide,
                    name: "Network".into(),
                    icon: Some("network-wired".into()),
                },
            ),
            (
                "mail".to_string(),
                crate::tray::Remembered {
                    behaviour: Behaviour::Mute,
                    name: "Mail".into(),
                    icon: None,
                },
            ),
        ]);
        let p = path.clone();
        let c = choices.clone();
        run_in_order(move || patch(&p, tray_icons_keys(&c)));
        flush();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[tray_icons.nm-applet]"), "{text}");
        assert!(text.contains("behaviour = \"mute\""), "{text}");
        let back = load_from(&path);
        assert_eq!(back.tray_icons, choices);
        assert!(back.auto_arrange);
    }

    #[test]
    fn a_session_save_round_trips_and_keeps_the_desktop() {
        let dir = scratch("session");
        let path = dir.join("state.toml");
        std::fs::write(&path, "auto_arrange = true\n").unwrap();
        let apps = vec![crate::session::SavedApp {
            app_id: "foot".into(),
            entry: "foot.desktop".into(),
            desktop: 1,
        }];
        let tabs = vec![crate::session::SavedTabs {
            apps: vec!["foot".into(), "foot".into()],
            shown: 1,
            desktop: 1,
        }];
        let (p, a, t) = (path.clone(), apps.clone(), tabs.clone());
        run_in_order(move || patch(&p, session_keys(&a, &t)));
        flush();
        let back = load_from(&path);
        assert_eq!(back.session, apps);
        assert_eq!(back.session_tabs, tabs);
        assert!(back.auto_arrange);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The desktop's saves rewrite their keys and keep every other key of the file, those this
    /// build does not read included.
    #[test]
    fn a_desktop_save_keeps_the_other_keys() {
        let dir = scratch("patch");
        let path = dir.join("state.toml");
        std::fs::write(
            &path,
            "taskbar_locked = false\nrun_history = [\"calc\"]\n\
             [desktop_positions]\n\"Old\" = [9, 9]\n",
        )
        .unwrap();
        let cells = BTreeMap::from([("Reports".to_string(), (2, 1))]);
        let p = path.clone();
        let c = cells.clone();
        run_in_order(move || patch(&p, desktop_keys(true, &c)));
        let names = BTreeMap::from([("documents".to_string(), "Papers".to_string())]);
        let (p, n) = (path.clone(), names.clone());
        run_in_order(move || patch(&p, system_names_keys(&n)));
        flush();
        let back = load_from(&path);
        assert!(back.auto_arrange);
        assert_eq!(back.desktop_positions, cells);
        assert_eq!(back.system_icon_names, names, "a renamed place is kept");
        let table: toml::Table = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(table["taskbar_locked"], toml::Value::Boolean(false));
        assert_eq!(
            table["run_history"],
            toml::Value::Array(vec!["calc".into()])
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_old_bin_key_loads_and_drops_alone() {
        let dir = scratch("bin");
        let path = dir.join("state.toml");
        std::fs::write(
            &path,
            "run_history = [\"calc\"]\n[bin]\nno_bin = true\nno_confirm = false\n",
        )
        .unwrap();
        let back = load_from(&path);
        assert_eq!(
            back.bin,
            Some(crate::binprops::OldBinSettings {
                no_bin: true,
                no_confirm: false
            })
        );
        unpatch(&path, "bin");
        let back = load_from(&path);
        assert_eq!(back.bin, None);
        assert_eq!(back.run_history, ["calc"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_start_save_loads_back_and_keeps_the_other_keys() {
        let dir = scratch("start");
        let path = dir.join("state.toml");
        std::fs::write(&path, "run_history = [\"calc\"]\n").unwrap();
        let l = crate::launches::Launches {
            pins: vec!["a.desktop".into()],
            recent: vec!["b.desktop".into(), "a.desktop".into()],
        };
        let p = path.clone();
        run_in_order(move || patch(&p, start_keys(&l)));
        flush();
        let back = load_from(&path);
        assert_eq!(back.start_pins, ["a.desktop"]);
        assert_eq!(back.launches, ["b.desktop", "a.desktop"]);
        assert_eq!(back.run_history, ["calc"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_taskbar_save_loads_back_and_keeps_the_desktop_keys() {
        let dir = scratch("taskbar");
        let path = dir.join("state.toml");
        std::fs::write(&path, "auto_arrange = true\n").unwrap();
        let bands = Bands {
            tasks_first: true,
            first_width: Some(425),
        };
        let p = path.clone();
        run_in_order(move || patch(&p, taskbar_keys(false, bands, false)));
        flush();
        let back = load_from(&path);
        assert_eq!(back.taskbar_locked, Some(false));
        assert_eq!(back.bands, bands);
        assert_eq!(back.quicklaunch_shown, Some(false));
        assert!(back.auto_arrange);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
