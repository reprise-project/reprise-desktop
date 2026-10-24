//! Reading `config.toml` into [`Settings`] and writing it back.

use super::fields::{Field, INTERFACE, fields, list_to_text, words};
use super::shortcuts::{
    DENIED, GLOBALS, KEYBINDS, keybind_entries, merge_denied, merge_programs, program_tables,
    read_programs, read_shortcuts, shortcut_tables,
};
use super::{
    ACTIONS, ProgramShortcut, Settings, Shortcut, check_keys, check_program_keys, default_chord,
    glass_swatch,
};
use repriseconf::{Key, get, key as k, remove, set};
use toml_edit::{Array, DocumentMut, Item, Value};

fn defaults_in(doc: &DocumentMut) -> Settings {
    Settings::for_interface(get(doc, &INTERFACE).and_then(Item::as_str).unwrap_or(""))
}

/// Read every key the sheet knows out of `text`, leaving the rest of the
/// defaults of its interface in place. A file that does not parse gives
/// the defaults whole, as the compositor's own `Config::load` does.
pub fn load(text: &str) -> Settings {
    let Ok(doc) = text.parse::<DocumentMut>() else {
        log::warn!("config.toml does not parse; showing defaults");
        return Settings::default();
    };
    let mut s = defaults_in(&doc);
    for f in fields() {
        let Some(item) = get(&doc, f.key()) else {
            continue;
        };
        match &f {
            Field::Text(_, at) => {
                if let Some(v) = item.as_str() {
                    *at.get_mut(&mut s) = v.to_string();
                }
            }
            Field::Int(_, at) => {
                if let Some(v) = item.as_integer() {
                    *at.get_mut(&mut s) = v;
                }
            }
            Field::Flag(_, at) => {
                if let Some(v) = item.as_bool() {
                    *at.get_mut(&mut s) = v;
                }
            }
            Field::List(_, at) => {
                if let Some(v) = list_to_text(item) {
                    *at.get_mut(&mut s) = v;
                }
            }
            Field::Words(_, at) => {
                if let Some(arr) = item.as_array() {
                    let w: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
                    *at.get_mut(&mut s) = w.join(" ");
                }
            }
            Field::Tenths(_, at, (lo, hi)) => {
                // `speed = 1` is a valid float to the compositor too.
                let v = item
                    .as_float()
                    .or_else(|| item.as_integer().map(|i| i as f64));
                if let Some(v) = v {
                    *at.get_mut(&mut s) = ((v * 10.0).round() as i64).clamp(*lo, *hi);
                }
            }
            // An older file's icon names read as the current ones.
            Field::Names(_, at) => {
                if let Some(arr) = item.as_array() {
                    *at.get_mut(&mut s) = repriseconf::system_icons::current_list(
                        arr.iter().filter_map(|v| v.as_str()),
                    );
                }
            }
        }
    }
    if let Some(list) = get(&doc, &KEYBINDS).and_then(read_shortcuts) {
        use repriseconf::keybind::{NONE, layered, same};
        // A default's "none" stays a row, shown and re-enabled from the list:
        // `layered` drops entries it reads as "none", so it sees them as "".
        let mut binds = layered(
            list,
            |b| (&b.keys, if b.action == NONE { "" } else { &b.action }),
            |k, a| Shortcut::new(k, a, ""),
        );
        let bound: Vec<String> = (binds.iter())
            .filter(|b| b.action != NONE)
            .map(|b| b.keys.clone())
            .collect();
        binds.retain(|b| {
            b.action != NONE || default_chord(&b.keys) && !bound.iter().any(|k| same(k, &b.keys))
        });
        s.shortcuts = binds;
    }
    if let Some(list) = get(&doc, &GLOBALS).and_then(Item::as_array_of_tables) {
        s.programs = read_programs(list);
    }
    if let Some(arr) = get(&doc, &DENIED).and_then(Item::as_array) {
        s.denied = arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::to_string)
            .collect();
    }
    if let Some(menu) = crate::startmenu::read(&doc) {
        s.startmenu = menu;
    }
    s.tablet_pads = crate::tabletpage::read(&doc);
    if glass_swatch(&s.glass_color).is_none() {
        s.glass_custom = s.glass_color.clone();
    }
    s
}

/// Write into `text` the fields the sheet moved, `want` against `base`, touching as little as possible: keys the sheet did not move are left alone even if the file changed under it, keys moved back to the interface's default are removed (and tables left empty), tables are created only when a key needs one.
///
/// `Err` when `text` does not parse; the caller must not write then.
pub fn save(text: &str, want: &Settings, base: &Settings) -> Result<String, String> {
    let mut doc = text.parse::<DocumentMut>().map_err(|e| e.to_string())?;
    // What the file says today, with the defaults standing in for the
    // keys it leaves out.
    let have = load(text);
    // The defaults of the interface the file names once this save lands:
    // the sheet's when it moved it, else the one on disk.
    let moved = want.interface != base.interface && want.interface != have.interface;
    let def = Settings::for_interface(if moved {
        &want.interface
    } else {
        &have.interface
    });
    // The tables a key was removed from.
    let mut shrunk: Vec<&'static str> = Vec::new();
    let mut drop_key = |doc: &mut DocumentMut, key: &Key| {
        if get(doc, key).is_some() {
            remove(doc, key);
            shrunk.push(key.table);
        }
    };
    for f in fields() {
        if f.same(want, base) || f.same(want, &have) {
            continue;
        }
        if f.same(want, &def) {
            drop_key(&mut doc, f.key());
        } else {
            set(&mut doc, f.key(), f.value(want)).map_err(|e| e.to_string())?;
        }
    }
    for table in shrunk {
        remove_if_empty(&mut doc, table);
    }
    // The list is written whole or not at all: a table's comments do not
    // survive a rewrite anyway.
    if want.shortcuts != base.shortcuts && want.shortcuts != have.shortcuts {
        let entries = keybind_entries(&want.shortcuts);
        if entries.is_empty() {
            remove(&mut doc, &KEYBINDS);
        } else {
            doc[KEYBINDS.name] = Item::ArrayOfTables(shortcut_tables(&entries));
        }
    }
    // Merged per entry: the portal writes these too.
    if want.programs != base.programs {
        let list = merge_programs(&want.programs, &base.programs, &have.programs);
        if list.is_empty() {
            remove(&mut doc, &GLOBALS);
        } else {
            doc[GLOBALS.name] = Item::ArrayOfTables(program_tables(&list));
        }
    }
    if want.denied != base.denied {
        let list = merge_denied(&want.denied, &base.denied, &have.denied);
        if list.is_empty() {
            remove(&mut doc, &DENIED);
            remove_if_empty(&mut doc, DENIED.table);
        } else {
            let arr: Array = list.iter().map(String::as_str).collect();
            set(&mut doc, &DENIED, Value::Array(arr)).map_err(|e| e.to_string())?;
        }
    }
    crate::startmenu::write(&mut doc, &want.startmenu, &base.startmenu, &have.startmenu);
    crate::tabletpage::write(&mut doc, &want.tablet_pads, &base.tablet_pads);
    Ok(doc.to_string())
}

/// `config.toml` changed under the sheet: every value `want` still holds as `base` (what the sheet loaded or last saved) takes `fresh`'s, the file as it reads now; a value the user moved stays pending.
pub fn rebase(want: &mut Settings, base: &Settings, fresh: &Settings) {
    fn follow<T: PartialEq + Clone>(want: &mut T, base: &T, fresh: &T) {
        if want == base {
            *want = fresh.clone();
        }
    }
    for f in fields() {
        if f.same(want, base) {
            f.copy(fresh, want);
        }
    }
    follow(
        &mut want.glass_custom,
        &base.glass_custom,
        &fresh.glass_custom,
    );
    follow(&mut want.shortcuts, &base.shortcuts, &fresh.shortcuts);
    follow(&mut want.programs, &base.programs, &fresh.programs);
    follow(&mut want.denied, &base.denied, &fresh.denied);
    follow(&mut want.startmenu, &base.startmenu, &fresh.startmenu);
    follow(&mut want.tablet_pads, &base.tablet_pads, &fresh.tablet_pads);
}

/// The sheet's interface moved from `from` to `s.interface`: the scheme goes back to the new interface's own, and every field `text` leaves unset that held `from`'s default takes the new one's. Moved back to `loaded`'s interface, the scheme and glass colour are `loaded`'s again.
pub fn switch_interface(s: &mut Settings, from: &str, text: &str, loaded: &Settings) {
    let doc = text
        .parse::<DocumentMut>()
        .unwrap_or_else(|_| DocumentMut::new());
    let (old, new) = (
        Settings::for_interface(from),
        Settings::for_interface(&s.interface),
    );
    for f in fields() {
        if get(&doc, f.key()).is_none() && f.same(s, &old) {
            f.copy(&new, s);
        }
    }
    if s.interface == loaded.interface {
        s.color_scheme = loaded.color_scheme.clone();
        s.glass_color = loaded.glass_color.clone();
    } else {
        s.color_scheme.clear();
    }
}

/// Remove the table at `path` (dotted for a nested one) when nothing is left in it.
fn remove_if_empty(doc: &mut DocumentMut, path: &'static str) {
    let (parent, name) = path.rsplit_once('.').unwrap_or(("", path));
    let key = k(parent, name);
    let empty = get(doc, &key)
        .and_then(Item::as_table_like)
        .is_some_and(|t| t.is_empty());
    if !name.is_empty() && empty {
        remove(doc, &key);
    }
}

/// The compositor's own parse of each spec in `text`, at most `ZONE_MAX` of them.
fn check_snap_layouts(text: &str) -> Result<(), String> {
    use reprisekit::snaplayouts::{ZONE_MAX, ZoneLayout};
    let specs: Vec<&str> = words(text).collect();
    if specs.len() > ZONE_MAX {
        return Err(format!(
            "Snap layouts: at most {ZONE_MAX} layouts, got {}.",
            specs.len()
        ));
    }
    for spec in specs {
        ZoneLayout::parse(spec).map_err(|e| format!("Snap layouts: {e}."))?;
    }
    Ok(())
}

/// What Apply refuses before writing: a bind's key that will not bind, or a program key changed from `base` that fails `check_program_keys`.
pub fn validate(s: &Settings, base: &Settings) -> Result<(), String> {
    if s.nl_from == s.nl_to {
        return Err("The night light's From and To times must differ.".into());
    }
    for sc in &s.shortcuts {
        check_keys(&sc.keys)?;
        if sc.action == repriseconf::keybind::NONE && default_chord(&sc.keys) {
            continue;
        }
        if !ACTIONS.contains(&sc.action.as_str()) {
            return Err("A shortcut's action is not one the compositor knows.".into());
        }
        if sc.action == "spawn" && sc.command.trim().is_empty() {
            return Err("A spawn shortcut needs a command.".into());
        }
    }
    check_snap_layouts(&s.snap_layouts)?;
    let as_loaded = |p: &ProgramShortcut| {
        (base.programs.iter()).any(|b| b.app == p.app && b.id == p.id && b.keys == p.keys)
    };
    for p in s.programs.iter().filter(|p| !as_loaded(p)) {
        if let Err(why) = check_program_keys(&p.keys, &p.keys) {
            let name = if p.description.is_empty() {
                &p.id
            } else {
                &p.description
            };
            return Err(format!("{name}: {why}"));
        }
    }
    Ok(())
}
