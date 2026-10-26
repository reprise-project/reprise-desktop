use super::Settings;
use repriseconf::{Key, key as k};
use toml_edit::{ArrayOfTables, Item, Table};

/// One `[[keybinds]]` entry as the sheet edits it: the key string the
/// compositor parses ("Super+Shift+b"), the action name and, for `spawn`,
/// the command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub keys: String,
    pub action: String,
    pub command: String,
}

impl Shortcut {
    pub fn new(keys: &str, action: &str, command: &str) -> Shortcut {
        Shortcut {
            keys: keys.into(),
            action: action.into(),
            command: command.into(),
        }
    }
}

/// One `[[global_shortcuts]]` entry: a key a program bound through the
/// portal; `keys` is empty when unassigned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramShortcut {
    pub app: String,
    pub id: String,
    pub description: String,
    pub keys: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Bind(usize),
    Program(usize),
    Denied(usize),
}

/// The compositor's action names, `spawn` first since it is the one with
/// a command. Mirrors `Action::name` in `src/config/keybinds.rs`.
pub const ACTIONS: [&str; 60] = [
    "spawn",
    "terminal",
    "close",
    "cycle",
    "flip",
    "exit",
    "maximize",
    "minimize",
    "lock",
    "startmenu",
    "run",
    "logoff",
    "turnoff",
    "windowmenu",
    "showdesktop",
    "toggleframe",
    "alwaysontop",
    "launcher",
    "overview",
    "focus-taskbar",
    "focus-tray",
    "clipboard-history",
    "emoji-picker",
    "next-layout",
    "center",
    "tab-next",
    "tab-prev",
    "do-not-disturb",
    "night-light",
    "snap-left",
    "snap-right",
    "snap-up",
    "snap-down",
    "snap-layouts",
    "zoom-in",
    "zoom-out",
    "zoom-reset",
    "color-filter",
    "desktop-prev",
    "desktop-next",
    "desktop-carry-prev",
    "desktop-carry-next",
    "all-desktops",
    "screenshot",
    "screenshot-window",
    "screenshot-region",
    "screenshot-content",
    "volume-up",
    "volume-down",
    "mute",
    "mic-mute",
    "brightness-up",
    "brightness-down",
    "kbd-brightness-up",
    "kbd-brightness-down",
    "play-pause",
    "next",
    "prev",
    "stop",
    "wifi",
];

/// The compositor's default binds (`repriseconf::keybind::DEFAULTS`), which
/// a file's `[[keybinds]]` layer over.
pub(super) fn default_shortcuts() -> Vec<Shortcut> {
    (repriseconf::keybind::DEFAULTS.iter())
        .map(|(k, a)| Shortcut::new(k, a, ""))
        .collect()
}

/// Whether `keys` is one of the default binds' chords, the only chords a
/// [`repriseconf::keybind::NONE`] row unbinds.
pub fn default_chord(keys: &str) -> bool {
    use repriseconf::keybind::{DEFAULTS, same};
    DEFAULTS.iter().any(|(k, _)| same(keys, k))
}

pub fn check_keys(keys: &str) -> Result<(), &'static str> {
    repriseconf::keybind::check(keys)
}

/// Why a program could not have `keys`, as `Err`: keys that would not
/// bind, or keys the compositor never gives a program (modifiers alone,
/// or kept by the desktop). Empty keys are fine: unassigned. `shown`
/// names the keys in the reason.
pub fn check_program_keys(keys: &str, shown: &str) -> Result<(), String> {
    if keys.is_empty() {
        return Ok(());
    }
    check_keys(keys)?;
    if repriseconf::keybind::modifier_only(keys) {
        return Err("A program shortcut ends in a key that is not a modifier.".into());
    }
    if repriseconf::keybind::reserved(keys) {
        return Err(format!("{shown} is kept by the desktop."));
    }
    Ok(())
}

pub fn program_text(p: &ProgramShortcut, name: &str) -> String {
    let what = if p.description.is_empty() {
        &p.id
    } else {
        &p.description
    };
    format!("{name}: {what}")
}

pub(super) const KEYBINDS: Key = k("", "keybinds");

/// `[[keybinds]]` (or an inline `keybinds = [{...}]`) as shortcuts; None
/// when the item is neither. An entry without `keys` or `action` is
/// skipped (the compositor would reject the file).
pub(super) fn read_shortcuts(item: &Item) -> Option<Vec<Shortcut>> {
    let tables: Vec<&dyn toml_edit::TableLike> = if let Some(a) = item.as_array_of_tables() {
        a.iter().map(|t| t as &dyn toml_edit::TableLike).collect()
    } else {
        item.as_array()?
            .iter()
            .filter_map(|v| v.as_inline_table().map(|t| t as &dyn toml_edit::TableLike))
            .collect()
    };
    let text = |t: &dyn toml_edit::TableLike, name: &str| {
        t.get(name).and_then(Item::as_str).map(str::to_string)
    };
    Some(
        tables
            .iter()
            .filter_map(|t| {
                Some(Shortcut {
                    keys: text(*t, "keys")?,
                    action: text(*t, "action")?,
                    command: text(*t, "command").unwrap_or_default(),
                })
            })
            .collect(),
    )
}

/// The `[[keybinds]]` entries that layer over the defaults into `binds`:
/// each default chord `binds` holds otherwise (or "none" when it holds
/// nothing there), then the binds on no default chord.
pub(super) fn keybind_entries(binds: &[Shortcut]) -> Vec<Shortcut> {
    use repriseconf::keybind::{DEFAULTS, NONE, same};
    let mut out = Vec::new();
    for &(keys, action) in DEFAULTS {
        let mine: Vec<&Shortcut> = binds.iter().filter(|b| same(&b.keys, keys)).collect();
        match mine[..] {
            [] => out.push(Shortcut::new(keys, NONE, "")),
            [b] if *b == Shortcut::new(keys, action, "") => {}
            _ => out.extend(mine.into_iter().cloned()),
        }
    }
    let on_a_default = |b: &Shortcut| DEFAULTS.iter().any(|(k, _)| same(&b.keys, k));
    out.extend(binds.iter().filter(|b| !on_a_default(b)).cloned());
    out
}

pub(super) fn shortcut_tables(list: &[Shortcut]) -> ArrayOfTables {
    let mut out = ArrayOfTables::new();
    for sc in list {
        let mut t = Table::new();
        t["keys"] = toml_edit::value(&sc.keys);
        t["action"] = toml_edit::value(&sc.action);
        if sc.action == "spawn" {
            t["command"] = toml_edit::value(&sc.command);
        }
        out.push(t);
    }
    out
}

pub(super) const GLOBALS: Key = k("", "global_shortcuts");

pub(super) const DENIED: Key = k("portal", "shortcuts_denied");

pub(super) fn read_programs(list: &ArrayOfTables) -> Vec<ProgramShortcut> {
    let text = |t: &Table, name: &str| t.get(name).and_then(Item::as_str).map(str::to_string);
    list.iter()
        .filter_map(|t| {
            Some(ProgramShortcut {
                app: text(t, "app")?,
                id: text(t, "id")?,
                description: text(t, "description").unwrap_or_default(),
                keys: text(t, "keys").unwrap_or_default(),
            })
        })
        .collect()
}

pub(super) fn program_tables(list: &[ProgramShortcut]) -> ArrayOfTables {
    let mut out = ArrayOfTables::new();
    for p in list {
        let mut t = Table::new();
        t["app"] = toml_edit::value(&p.app);
        t["id"] = toml_edit::value(&p.id);
        t["description"] = toml_edit::value(&p.description);
        t["keys"] = toml_edit::value(&p.keys);
        out.push(t);
    }
    out
}

/// `want`'s edits since `base`, applied to what the file holds now
/// (`have`): the portal may have added programs while the sheet was open.
/// The sheet only changes keys and removes entries.
pub(super) fn merge_programs(
    want: &[ProgramShortcut],
    base: &[ProgramShortcut],
    have: &[ProgramShortcut],
) -> Vec<ProgramShortcut> {
    let same = |a: &ProgramShortcut, b: &ProgramShortcut| a.app == b.app && a.id == b.id;
    have.iter()
        .filter(|h| !base.iter().any(|b| same(b, h)) || want.iter().any(|w| same(w, h)))
        .map(|h| {
            match (
                want.iter().find(|w| same(w, h)),
                base.iter().find(|b| same(b, h)),
            ) {
                (Some(w), Some(b)) if w != b => w.clone(),
                _ => h.clone(),
            }
        })
        .collect()
}

pub(super) fn merge_denied(want: &[String], base: &[String], have: &[String]) -> Vec<String> {
    have.iter()
        .filter(|h| !base.contains(h) || want.contains(h))
        .cloned()
        .collect()
}

/// What the Shortcuts list's row `i` is: the desktop binds, then each
/// program shortcut, then each denied program.
pub fn row(s: &Settings, i: usize) -> Option<Row> {
    let (b, p) = (s.shortcuts.len(), s.programs.len());
    if i < b {
        Some(Row::Bind(i))
    } else if i < b + p {
        Some(Row::Program(i - b))
    } else if i < b + p + s.denied.len() {
        Some(Row::Denied(i - b - p))
    } else {
        None
    }
}

/// Delete on a row: a bind goes, or on a default's chord becomes "none";
/// a program forgets every shortcut of that program, or its refusal, so it
/// asks again.
pub fn forget(s: &mut Settings, r: Row) {
    match r {
        Row::Bind(i) if default_chord(&s.shortcuts[i].keys) => {
            let sc = &mut s.shortcuts[i];
            sc.action = repriseconf::keybind::NONE.into();
            sc.command.clear();
        }
        Row::Bind(i) => {
            s.shortcuts.remove(i);
        }
        Row::Program(i) => {
            let app = s.programs[i].app.clone();
            s.programs.retain(|p| p.app != app);
        }
        Row::Denied(i) => {
            s.denied.remove(i);
        }
    }
}
