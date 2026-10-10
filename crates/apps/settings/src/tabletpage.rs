//! The Tablet page: the tablet pads the compositor has, and for the one
//! picked a combo per button and per ring and strip direction, offering
//! (None), a few common actions and key chords, and whatever else the
//! file binds it to.
//!
//! The bindings are `[[tablet_pads]]` entries in `config.toml`. A pad
//! edits the first entry whose `name` glob matches it, as the compositor
//! picks it, or a new entry naming it. Only the entry's own keys are
//! edited, in place: its `[[tablet_pads.modes]]` and comments stay. The
//! pads come over D-Bus (`reprisebus::tray::tablet_pads`) each time the
//! page opens.

use std::collections::BTreeMap;

use reprisebus::tray::Pad;
use reprisekit::controls::form::{Form, Item};
use reprisekit::raster::Rect;
use toml_edit::{ArrayOfTables, DocumentMut, InlineTable, Item as TomlItem, Table};

use crate::pages::{Area, GX, X};

/// One `[[tablet_pads]]` entry's own keys; an empty string is a key the
/// file leaves out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PadEntry {
    pub name: String,
    pub buttons: BTreeMap<u32, String>,
    pub ring_up: String,
    pub ring_down: String,
    pub strip_up: String,
    pub strip_down: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Button(u32),
    RingUp,
    RingDown,
    StripUp,
    StripDown,
}

impl Slot {
    fn label(self) -> String {
        match self {
            Slot::Button(n) => format!("Button {n}:"),
            Slot::RingUp => "Ring up:".into(),
            Slot::RingDown => "Ring down:".into(),
            Slot::StripUp => "Strip up:".into(),
            Slot::StripDown => "Strip down:".into(),
        }
    }
}

impl PadEntry {
    pub fn get(&self, slot: Slot) -> &str {
        match slot {
            Slot::Button(n) => self.buttons.get(&n).map_or("", String::as_str),
            Slot::RingUp => &self.ring_up,
            Slot::RingDown => &self.ring_down,
            Slot::StripUp => &self.strip_up,
            Slot::StripDown => &self.strip_down,
        }
    }

    fn set(&mut self, slot: Slot, value: &str) {
        let v = value.to_string();
        match slot {
            Slot::Button(n) if v.is_empty() => {
                self.buttons.remove(&n);
            }
            Slot::Button(n) => {
                self.buttons.insert(n, v);
            }
            Slot::RingUp => self.ring_up = v,
            Slot::RingDown => self.ring_down = v,
            Slot::StripUp => self.strip_up = v,
            Slot::StripDown => self.strip_down = v,
        }
    }
}

/// The rows a pad has: each button, then up and down for its rings and
/// for its strips. At most [`ROWS`].
pub fn slots(pad: &Pad) -> Vec<Slot> {
    let mut out: Vec<Slot> = (1..=pad.buttons).map(Slot::Button).collect();
    if pad.rings > 0 {
        out.extend([Slot::RingUp, Slot::RingDown]);
    }
    if pad.strips > 0 {
        out.extend([Slot::StripUp, Slot::StripDown]);
    }
    out.truncate(ROWS);
    out
}

pub fn entry_for(list: &[PadEntry], name: &str) -> Option<usize> {
    list.iter().position(|e| {
        let glob = if e.name.is_empty() { "*" } else { &e.name };
        repriseconf::glob::glob(glob, name)
    })
}

/// Bind `slot` of pad `name` to `value` ("" for none), in the entry that
/// maps the pad, or a new one naming it.
pub fn bind(list: &mut Vec<PadEntry>, name: &str, slot: Slot, value: &str) {
    let i = match entry_for(list, name) {
        Some(i) => i,
        None if value.is_empty() => return,
        None => {
            list.push(PadEntry {
                name: name.to_string(),
                ..PadEntry::default()
            });
            list.len() - 1
        }
    };
    list[i].set(slot, value);
}

const KEY: &str = "tablet_pads";
const STRINGS: [&str; 4] = ["ring_up", "ring_down", "strip_up", "strip_down"];

fn strings(e: &PadEntry) -> [&String; 4] {
    [&e.ring_up, &e.ring_down, &e.strip_up, &e.strip_down]
}

/// `[[tablet_pads]]` as entries; anything but an array of tables reads as
/// none.
pub fn read(doc: &DocumentMut) -> Vec<PadEntry> {
    let Some(list) = doc.get(KEY).and_then(TomlItem::as_array_of_tables) else {
        return Vec::new();
    };
    let text = |t: &Table, k: &str| {
        t.get(k)
            .and_then(TomlItem::as_str)
            .unwrap_or("")
            .to_string()
    };
    list.iter()
        .map(|t| {
            let buttons = (t.get("buttons").and_then(TomlItem::as_table_like))
                .map(|b| {
                    b.iter()
                        .filter_map(|(k, v)| Some((k.parse().ok()?, v.as_str()?.to_string())))
                        .collect()
                })
                .unwrap_or_default();
            PadEntry {
                name: text(t, "name"),
                buttons,
                ring_up: text(t, "ring_up"),
                ring_down: text(t, "ring_down"),
                strip_up: text(t, "strip_up"),
                strip_down: text(t, "strip_down"),
            }
        })
        .collect()
}

/// Write the entries that differ from `base` into `doc`'s
/// `[[tablet_pads]]`, each in place by its index; new ones go last.
pub fn write(doc: &mut DocumentMut, want: &[PadEntry], base: &[PadEntry]) {
    if want == base {
        return;
    }
    if doc.get(KEY).is_none() {
        doc[KEY] = TomlItem::ArrayOfTables(ArrayOfTables::new());
    }
    let Some(list) = doc[KEY].as_array_of_tables_mut() else {
        log::warn!(
            "tablet_pads is not a list of tables; the Tablet page's changes are not written"
        );
        return;
    };
    for (i, e) in want.iter().enumerate() {
        if base.get(i) == Some(e) {
            continue;
        }
        if list.get(i).is_none() {
            list.push(Table::new());
        }
        let t = list.get_mut(i).expect("pushed above");
        let old = base.get(i).cloned().unwrap_or_default();
        // Only a key that changed is touched, so the rest keep their
        // comments.
        if e.name != old.name && !e.name.is_empty() {
            t["name"] = toml_edit::value(&e.name);
        }
        if e.buttons != old.buttons {
            if e.buttons.is_empty() {
                t.remove("buttons");
            } else {
                let mut b = InlineTable::new();
                for (n, v) in &e.buttons {
                    b.insert(n.to_string(), v.as_str().into());
                }
                t["buttons"] = toml_edit::value(b);
            }
        }
        for ((k, v), was) in STRINGS.iter().zip(strings(e)).zip(strings(&old)) {
            if v == was {
                continue;
            }
            if v.is_empty() {
                t.remove(k);
            } else {
                t[*k] = toml_edit::value(v.as_str());
            }
        }
    }
}

/// What the combos offer besides a value the file has: (value, row).
/// Mirrors action names in the compositor's `Action::name` and chords
/// as `[[keybinds]]` spell them.
pub const CHOICES: &[(&str, &str)] = &[
    ("", "(None)"),
    ("ctrl+z", "Undo (Ctrl+Z)"),
    ("ctrl+shift+z", "Redo (Ctrl+Shift+Z)"),
    ("ctrl+plus", "Zoom in (Ctrl++)"),
    ("ctrl+minus", "Zoom out (Ctrl+-)"),
    ("zoom-in", "Magnify"),
    ("zoom-out", "Magnify less"),
    ("showdesktop", "Show desktop"),
    ("startmenu", "Start menu"),
    ("overview", "Overview"),
    ("launcher", "Launcher"),
    ("run", "Run..."),
    ("screenshot-region", "Screenshot"),
    ("volume-up", "Volume up"),
    ("volume-down", "Volume down"),
    ("mode", "Next mode"),
];

/// The combo rows for a slot bound to `value`: the choices, then
/// `value` itself when they do not offer it; and the row to select.
pub fn rows(value: &str) -> (Vec<String>, usize) {
    let mut rows: Vec<String> = CHOICES.iter().map(|c| c.1.to_string()).collect();
    let at = match CHOICES.iter().position(|c| c.0 == value) {
        Some(i) => i,
        None => {
            rows.push(value.to_string());
            rows.len() - 1
        }
    };
    (rows, at)
}

pub fn value_at(row: usize, value: &str) -> String {
    match CHOICES.get(row) {
        Some(c) => c.0.to_string(),
        None => value.to_string(),
    }
}

pub const PAD: &str = "tablet_pad";
const PAD_LABEL: &str = "tablet_pad_label";
const NONE: &str = "tablet_none";
const GROUP: &str = "tablet_group";
const NOTE: &str = "tablet_note";

pub const ROWS: usize = 2 * COLUMN_ROWS;
const COLUMN_ROWS: usize = 13;
const ROW0: i32 = 106;
const ROW_STEP: i32 = 23;
const DROP_ROWS: usize = 12;

pub const ROW_IDS: [&str; ROWS] = [
    "tablet_row_0",
    "tablet_row_1",
    "tablet_row_2",
    "tablet_row_3",
    "tablet_row_4",
    "tablet_row_5",
    "tablet_row_6",
    "tablet_row_7",
    "tablet_row_8",
    "tablet_row_9",
    "tablet_row_10",
    "tablet_row_11",
    "tablet_row_12",
    "tablet_row_13",
    "tablet_row_14",
    "tablet_row_15",
    "tablet_row_16",
    "tablet_row_17",
    "tablet_row_18",
    "tablet_row_19",
    "tablet_row_20",
    "tablet_row_21",
    "tablet_row_22",
    "tablet_row_23",
    "tablet_row_24",
    "tablet_row_25",
];
const LABEL_IDS: [&str; ROWS] = [
    "tablet_label_0",
    "tablet_label_1",
    "tablet_label_2",
    "tablet_label_3",
    "tablet_label_4",
    "tablet_label_5",
    "tablet_label_6",
    "tablet_label_7",
    "tablet_label_8",
    "tablet_label_9",
    "tablet_label_10",
    "tablet_label_11",
    "tablet_label_12",
    "tablet_label_13",
    "tablet_label_14",
    "tablet_label_15",
    "tablet_label_16",
    "tablet_label_17",
    "tablet_label_18",
    "tablet_label_19",
    "tablet_label_20",
    "tablet_label_21",
    "tablet_label_22",
    "tablet_label_23",
    "tablet_label_24",
    "tablet_label_25",
];

/// The page for `pads` in `a`, `picked` the one shown, its rows from
/// `entries`. With no pad, a line saying so. The combos grow with the
/// column up to their most.
pub fn page(a: Area, pads: &[Pad], picked: usize, entries: &[PadEntry]) -> Form {
    let gw = a.gw();
    let mut items = vec![Item::group(GROUP, Rect::new(X, 44, a.w, 380), "Tablet Pad")];
    let Some(pad) = pads.get(picked) else {
        items.push(Item::label_at(
            NONE,
            GX,
            66,
            gw,
            "No tablet pad is connected.",
        ));
        return Form::new(items);
    };
    let names: Vec<&str> = pads.iter().map(|p| p.name.as_str()).collect();
    items.push(Item::label_at(PAD_LABEL, GX, 66, 40, "&Pad:"));
    let mut c = Item::combo(
        PAD,
        GX + 44,
        62,
        a.combo(Area::MIN.gw() - 44),
        &names,
        names.len().min(8),
    );
    if let reprisekit::controls::form::Control::Combo(combo) = &mut c.control {
        combo.selected = picked;
    }
    items.push(c);
    let entry = entry_for(entries, &pad.name).map(|i| &entries[i]);
    let slots = slots(pad);
    let two = slots.len() > COLUMN_ROWS;
    let col_w = if two { (gw - 12) / 2 } else { gw };
    let combo_w = |w: i32| w.min(crate::pages::COMBO_MAX);
    let label_w = if two { 64 } else { 96 };
    for (i, slot) in slots.into_iter().enumerate() {
        let (col, row) = (i / COLUMN_ROWS, i % COLUMN_ROWS);
        let x = GX + col as i32 * (col_w + 12);
        let y = ROW0 + row as i32 * ROW_STEP;
        items.push(Item::label_at(LABEL_IDS[i], x, y, label_w, &slot.label()));
        let value = entry.map_or("", |e| e.get(slot));
        let (rows, at) = rows(value);
        let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
        let mut c = Item::combo(
            ROW_IDS[i],
            x + label_w + 4,
            y - 4,
            combo_w(col_w - label_w - 4),
            &refs,
            DROP_ROWS,
        );
        if let reprisekit::controls::form::Control::Combo(combo) = &mut c.control {
            combo.selected = at;
        }
        items.push(c);
    }
    items.push(Item::label_at(
        NOTE,
        GX,
        86,
        gw,
        "Other keys and per-mode bindings go in config.toml.",
    ));
    Form::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(name: &str, buttons: u32, rings: u32, strips: u32) -> Pad {
        Pad {
            name: name.into(),
            buttons,
            rings,
            strips,
        }
    }

    #[test]
    fn a_pad_has_a_row_per_button_then_its_ring_and_strip_directions() {
        assert_eq!(
            slots(&pad("p", 2, 1, 2)),
            [
                Slot::Button(1),
                Slot::Button(2),
                Slot::RingUp,
                Slot::RingDown,
                Slot::StripUp,
                Slot::StripDown
            ]
        );
        assert_eq!(
            slots(&pad("p", 40, 1, 0)).len(),
            ROWS,
            "rows past two columns are not shown"
        );
    }

    #[test]
    fn a_pad_edits_the_first_entry_its_name_matches_or_a_new_one() {
        let mut list = vec![PadEntry {
            name: "Wacom*".into(),
            ..PadEntry::default()
        }];
        bind(&mut list, "Wacom Intuos Pad", Slot::Button(1), "zoom-in");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].get(Slot::Button(1)), "zoom-in");
        bind(&mut list, "HUION Pad", Slot::Button(1), "");
        assert_eq!(list.len(), 1, "none on an unmapped pad adds nothing");
        bind(&mut list, "HUION Pad", Slot::RingUp, "ctrl+plus");
        assert_eq!(list[1].name, "HUION Pad");
        assert_eq!(list[1].ring_up, "ctrl+plus");
        bind(&mut list, "Wacom Intuos Pad", Slot::Button(1), "");
        assert!(list[0].buttons.is_empty());
        assert_eq!(
            entry_for(&[PadEntry::default()], "any"),
            Some(0),
            "no name matches every pad"
        );
    }

    #[test]
    fn a_value_the_choices_lack_is_its_own_row() {
        let (r, at) = rows("ctrl+z");
        assert_eq!((r.len(), r[at].as_str()), (CHOICES.len(), "Undo (Ctrl+Z)"));
        let (r, at) = rows("ctrl+s");
        assert_eq!((r.len(), r[at].as_str()), (CHOICES.len() + 1, "ctrl+s"));
        assert_eq!(value_at(at, "ctrl+s"), "ctrl+s");
        assert_eq!(value_at(0, "ctrl+s"), "");
        assert_eq!(value_at(5, ""), "zoom-in");
    }

    #[test]
    fn entries_write_in_place_and_keep_their_modes() {
        let text = "# pads\n[[tablet_pads]]\nname = \"Wacom*\" # mine\nbuttons = { 1 = \"run\" }\nring_up = \"zoom-in\"\n\n[[tablet_pads.modes]]\nring_up = \"volume-up\"\n";
        let mut doc: DocumentMut = text.parse().unwrap();
        let base = read(&doc);
        assert_eq!(base.len(), 1);
        assert_eq!(base[0].buttons.get(&1).map(String::as_str), Some("run"));
        let mut want = base.clone();
        bind(&mut want, "Wacom Intuos Pad", Slot::Button(2), "ctrl+z");
        bind(&mut want, "Wacom Intuos Pad", Slot::RingUp, "");
        bind(&mut want, "HUION Pad", Slot::StripDown, "mode");
        write(&mut doc, &want, &base);
        let out = doc.to_string();
        assert!(out.contains("name = \"Wacom*\" # mine"), "{out}");
        assert!(
            out.contains("buttons = { 1 = \"run\", 2 = \"ctrl+z\" }"),
            "{out}"
        );
        assert!(!out.contains("ring_up = \"zoom-in\""), "{out}");
        assert!(
            out.contains("[[tablet_pads.modes]]\nring_up = \"volume-up\""),
            "{out}"
        );
        assert!(
            out.ends_with("[[tablet_pads]]\nname = \"HUION Pad\"\nstrip_down = \"mode\"\n"),
            "{out}"
        );
        assert_eq!(read(&out.parse().unwrap()), want);
    }

    #[test]
    fn nothing_changed_writes_nothing() {
        let mut doc: DocumentMut = "a = 1\n".parse().unwrap();
        write(&mut doc, &[], &[]);
        assert_eq!(doc.to_string(), "a = 1\n");
    }

    #[test]
    fn the_page_lays_one_column_out_then_two() {
        let one = page(Area::MIN, &[pad("Pad", 4, 1, 0)], 0, &[]);
        let r = |f: &Form, id: &str| f.item(id).unwrap().rect;
        assert_eq!(
            r(&one, ROW_IDS[0]),
            Rect::new(
                GX + 100,
                ROW0 - 4,
                Area::MIN.gw() - 100,
                reprisekit::controls::combo::COMBO_H
            )
        );
        assert!(one.item(ROW_IDS[6]).is_none());
        let two = page(Area::MIN, &[pad("Pad", 18, 0, 0)], 0, &[]);
        assert_eq!(
            r(&two, ROW_IDS[13]).x,
            GX + (Area::MIN.gw() - 12) / 2 + 12 + 68
        );
        assert_eq!(r(&two, ROW_IDS[13]).y, ROW0 - 4);
        let none = page(Area::MIN, &[], 0, &[]);
        assert!(none.item(NONE).is_some() && none.item(PAD).is_none());
    }
}
