//! The Tablet page: the tablet pads the compositor has, and for the one
//! picked a combo per button, pad key and ring, strip and dial direction,
//! offering (None), a few common actions and key chords, and whatever else
//! the file binds it to; and an up-down for the ring and strip steps.
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
use toml_edit::{ArrayOfTables, DocumentMut, InlineTable, Item as TomlItem, Table, Value};

use crate::pages::{Area, GX, X};

/// One `[[tablet_pads]]` entry's own keys; an empty string or `None` is a
/// key the file leaves out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PadEntry {
    pub name: String,
    pub buttons: BTreeMap<u32, String>,
    /// By pad key name ("controlpanel").
    pub keys: BTreeMap<String, String>,
    pub ring_up: String,
    pub ring_down: String,
    pub strip_up: String,
    pub strip_down: String,
    pub dial_up: String,
    pub dial_down: String,
    /// Degrees.
    pub ring_step: Option<f64>,
    /// Of the strip's length, above 0 to 1.
    pub strip_step: Option<f64>,
}

/// `read` keeps only finite steps, so equality is reflexive.
impl Eq for PadEntry {}

/// The pad keys a `keys` table names, and their row labels.
pub const PAD_KEYS: [(&str, &str); 3] = [
    ("buttonconfig", "Config key:"),
    ("controlpanel", "Panel key:"),
    ("onscreen_keyboard", "Keyboard key:"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Button(u32),
    /// A name from [`PAD_KEYS`].
    Key(&'static str),
    RingUp,
    RingDown,
    StripUp,
    StripDown,
    DialUp,
    DialDown,
}

impl Slot {
    fn label(self) -> String {
        match self {
            Slot::Button(n) => format!("Button {n}:"),
            Slot::Key(k) => PAD_KEYS.iter().find(|p| p.0 == k).map_or(k, |p| p.1).into(),
            Slot::RingUp => "Ring up:".into(),
            Slot::RingDown => "Ring down:".into(),
            Slot::StripUp => "Strip up:".into(),
            Slot::StripDown => "Strip down:".into(),
            Slot::DialUp => "Dial up:".into(),
            Slot::DialDown => "Dial down:".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Ring,
    Strip,
}

impl Step {
    /// The up-down's id, its range and its unit; it shows the step in
    /// whole units.
    fn spin(self) -> (&'static str, &'static str, (i32, i32), &'static str) {
        match self {
            Step::Ring => (RING_STEP, RING_UNITS, (1, 360), "°"),
            Step::Strip => (STRIP_STEP, STRIP_UNITS, (1, 100), "%"),
        }
    }

    /// The file's value as the up-down's whole units: degrees, or percent
    /// of the strip.
    fn units(self, v: f64) -> i32 {
        let (_, _, (min, max), _) = self.spin();
        let v = match self {
            Step::Ring => v,
            Step::Strip => v * 100.0,
        };
        (v.round() as i32).clamp(min, max)
    }

    fn value(self, units: i32) -> f64 {
        match self {
            Step::Ring => f64::from(units),
            Step::Strip => f64::from(units) / 100.0,
        }
    }

    /// The compositor's step when the entry leaves it out.
    const fn default(self) -> f64 {
        match self {
            Step::Ring => 15.0,
            Step::Strip => 0.1,
        }
    }
}

/// A row of the page: a binding combo, or a step's up-down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Bind(Slot),
    Step(Step),
}

impl PadEntry {
    pub fn get(&self, slot: Slot) -> &str {
        match slot {
            Slot::Button(n) => self.buttons.get(&n).map_or("", String::as_str),
            Slot::Key(k) => self.keys.get(k).map_or("", String::as_str),
            Slot::RingUp => &self.ring_up,
            Slot::RingDown => &self.ring_down,
            Slot::StripUp => &self.strip_up,
            Slot::StripDown => &self.strip_down,
            Slot::DialUp => &self.dial_up,
            Slot::DialDown => &self.dial_down,
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
            Slot::Key(k) if v.is_empty() => {
                self.keys.remove(k);
            }
            Slot::Key(k) => {
                self.keys.insert(k.to_string(), v);
            }
            Slot::RingUp => self.ring_up = v,
            Slot::RingDown => self.ring_down = v,
            Slot::StripUp => self.strip_up = v,
            Slot::StripDown => self.strip_down = v,
            Slot::DialUp => self.dial_up = v,
            Slot::DialDown => self.dial_down = v,
        }
    }

    pub fn step(&self, step: Step) -> Option<f64> {
        match step {
            Step::Ring => self.ring_step,
            Step::Strip => self.strip_step,
        }
    }
}

/// The rows a pad has: each button and pad key, then up and down for its
/// rings and a ring step, the same for its strips, and up and down for its
/// dials. At most [`ROWS`].
pub fn rows_of(pad: &Pad) -> Vec<Row> {
    let mut out: Vec<Row> = (1..=pad.buttons)
        .map(|n| Row::Bind(Slot::Button(n)))
        .collect();
    out.extend(
        (PAD_KEYS.iter())
            .filter(|(k, _)| pad.keys.iter().any(|h| h == k))
            .map(|&(k, _)| Row::Bind(Slot::Key(k))),
    );
    if pad.rings > 0 {
        out.extend([
            Row::Bind(Slot::RingUp),
            Row::Bind(Slot::RingDown),
            Row::Step(Step::Ring),
        ]);
    }
    if pad.strips > 0 {
        out.extend([
            Row::Bind(Slot::StripUp),
            Row::Bind(Slot::StripDown),
            Row::Step(Step::Strip),
        ]);
    }
    if pad.dials > 0 {
        out.extend([Row::Bind(Slot::DialUp), Row::Bind(Slot::DialDown)]);
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

/// The entry that maps pad `name`, or a new one naming it.
fn entry_mut<'a>(list: &'a mut Vec<PadEntry>, name: &str) -> &'a mut PadEntry {
    let i = entry_for(list, name).unwrap_or_else(|| {
        list.push(PadEntry {
            name: name.to_string(),
            ..PadEntry::default()
        });
        list.len() - 1
    });
    &mut list[i]
}

/// Bind `slot` of pad `name` to `value` ("" for none), in the entry that
/// maps the pad, or a new one naming it.
pub fn bind(list: &mut Vec<PadEntry>, name: &str, slot: Slot, value: &str) {
    if value.is_empty() && entry_for(list, name).is_none() {
        return;
    }
    entry_mut(list, name).set(slot, value);
}

/// Set pad `name`'s `step` to `units` of its up-down.
pub fn set_step(list: &mut Vec<PadEntry>, name: &str, step: Step, units: i32) {
    let e = entry_mut(list, name);
    let v = Some(step.value(units));
    match step {
        Step::Ring => e.ring_step = v,
        Step::Strip => e.strip_step = v,
    }
}

const KEY: &str = "tablet_pads";
const STRINGS: [&str; 6] = [
    "ring_up",
    "ring_down",
    "strip_up",
    "strip_down",
    "dial_up",
    "dial_down",
];
const STEPS: [&str; 2] = ["ring_step", "strip_step"];

fn strings(e: &PadEntry) -> [&String; 6] {
    [
        &e.ring_up,
        &e.ring_down,
        &e.strip_up,
        &e.strip_down,
        &e.dial_up,
        &e.dial_down,
    ]
}

fn steps(e: &PadEntry) -> [Option<f64>; 2] {
    [e.ring_step, e.strip_step]
}

/// Set `key` to `v`, keeping the comment after the old value.
fn set(t: &mut Table, key: &str, v: impl Into<Value>) {
    let mut v = v.into();
    if let Some(old) = t.get(key).and_then(TomlItem::as_value) {
        *v.decor_mut() = old.decor().clone();
    }
    t[key] = TomlItem::Value(v);
}

/// `map` as an inline table, or the key removed when it is empty.
fn write_map<K: ToString>(t: &mut Table, key: &str, map: &BTreeMap<K, String>) {
    if map.is_empty() {
        t.remove(key);
        return;
    }
    let mut b = InlineTable::new();
    for (k, v) in map {
        b.insert(k.to_string(), v.as_str().into());
    }
    set(t, key, b);
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
    let map = |t: &Table, k: &str| -> Vec<(String, String)> {
        (t.get(k).and_then(TomlItem::as_table_like))
            .map(|b| {
                b.iter()
                    .filter_map(|(k, v)| Some((k.to_string(), v.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default()
    };
    let number = |t: &Table, k: &str| {
        let v = t.get(k)?.as_value()?;
        let n = v.as_float().or_else(|| v.as_integer().map(|n| n as f64))?;
        n.is_finite().then_some(n)
    };
    list.iter()
        .map(|t| PadEntry {
            name: text(t, "name"),
            buttons: (map(t, "buttons").into_iter())
                .filter_map(|(k, v)| Some((k.parse().ok()?, v)))
                .collect(),
            keys: map(t, "keys").into_iter().collect(),
            ring_up: text(t, "ring_up"),
            ring_down: text(t, "ring_down"),
            strip_up: text(t, "strip_up"),
            strip_down: text(t, "strip_down"),
            dial_up: text(t, "dial_up"),
            dial_down: text(t, "dial_down"),
            ring_step: number(t, "ring_step"),
            strip_step: number(t, "strip_step"),
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
            set(t, "name", &e.name);
        }
        if e.buttons != old.buttons {
            write_map(t, "buttons", &e.buttons);
        }
        if e.keys != old.keys {
            write_map(t, "keys", &e.keys);
        }
        for ((k, v), was) in STRINGS.iter().zip(strings(e)).zip(strings(&old)) {
            if v == was {
                continue;
            }
            if v.is_empty() {
                t.remove(k);
            } else {
                set(t, k, v.as_str());
            }
        }
        for ((k, v), was) in STEPS.iter().zip(steps(e)).zip(steps(&old)) {
            if v == was {
                continue;
            }
            match v {
                None => {
                    t.remove(k);
                }
                // A whole step reads back as the integer the file
                // would have.
                Some(v) if v.fract() == 0.0 => set(t, k, v as i64),
                Some(v) => set(t, k, v),
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
pub const RING_STEP: &str = "tablet_ring_step";
const RING_UNITS: &str = "tablet_ring_units";
pub const STRIP_STEP: &str = "tablet_strip_step";
const STRIP_UNITS: &str = "tablet_strip_units";

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
    let list = rows_of(pad);
    let two = list.len() > COLUMN_ROWS;
    let col_w = if two { (gw - 12) / 2 } else { gw };
    let combo_w = |w: i32| w.min(crate::pages::COMBO_MAX);
    let label_w = if two { 64 } else { 96 };
    for (i, row) in list.into_iter().enumerate() {
        let (col, line) = (i / COLUMN_ROWS, i % COLUMN_ROWS);
        let x = GX + col as i32 * (col_w + 12);
        let y = ROW0 + line as i32 * ROW_STEP;
        let slot = match row {
            Row::Bind(slot) => slot,
            Row::Step(step) => {
                let (id, units_id, range, units) = step.spin();
                let label = match step {
                    Step::Ring => "Ring step:",
                    Step::Strip => "Strip step:",
                };
                let v = entry.and_then(|e| e.step(step)).unwrap_or(step.default());
                let mut spin = crate::pages::spin(LABEL_IDS[i], id, x, y, label_w, label, range);
                if let reprisekit::controls::form::Control::UpDown(u) = &mut spin[1].control {
                    u.set(step.units(v));
                }
                items.extend(spin);
                let ux = label_w + 4 + reprisekit::controls::updown::WIDTH + 6;
                items.push(Item::label_at(
                    units_id,
                    x + ux,
                    y,
                    (col_w - ux).min(48),
                    units,
                ));
                continue;
            }
        };
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
        "Per-mode bindings go in config.toml.",
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
            ..Pad::default()
        }
    }

    #[test]
    fn a_pad_has_a_row_per_button_key_and_direction_and_a_step_per_ring_and_strip() {
        let p = Pad {
            dials: 2,
            keys: vec!["onscreen_keyboard".into(), "controlpanel".into()],
            ..pad("p", 2, 1, 2)
        };
        use Row::{Bind, Step as S};
        assert_eq!(
            rows_of(&p),
            [
                Bind(Slot::Button(1)),
                Bind(Slot::Button(2)),
                Bind(Slot::Key("controlpanel")),
                Bind(Slot::Key("onscreen_keyboard")),
                Bind(Slot::RingUp),
                Bind(Slot::RingDown),
                S(Step::Ring),
                Bind(Slot::StripUp),
                Bind(Slot::StripDown),
                S(Step::Strip),
                Bind(Slot::DialUp),
                Bind(Slot::DialDown),
            ]
        );
        assert_eq!(
            rows_of(&pad("p", 2, 0, 0)),
            [Bind(Slot::Button(1)), Bind(Slot::Button(2))],
            "no ring, strip, dial or key, no row for them"
        );
        assert_eq!(
            rows_of(&pad("p", 40, 1, 0)).len(),
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
        set_step(&mut list, "Gaomon Pad", Step::Strip, 25);
        assert_eq!(
            (list[2].name.as_str(), list[2].strip_step),
            ("Gaomon Pad", Some(0.25))
        );
        assert_eq!(
            entry_for(&[PadEntry::default()], "any"),
            Some(0),
            "no name matches every pad"
        );
    }

    #[test]
    fn a_step_shows_in_whole_units_of_its_up_down() {
        assert_eq!(Step::Ring.units(7.5), 8);
        assert_eq!(Step::Ring.units(400.0), 360);
        assert_eq!(Step::Strip.units(0.1), 10);
        assert_eq!(Step::Strip.units(0.001), 1);
        assert_eq!(Step::Strip.value(25), 0.25);
        assert_eq!(Step::Ring.value(30), 30.0);
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
    fn keys_dials_and_steps_round_trip_and_only_moved_ones_are_written() {
        let text = "[[tablet_pads]]\nname = \"Reprise*\"\nkeys = { controlpanel = \"run\" } # k\ndial_up = \"plus\" # d\nring_step = 7.5 # r\nstrip_step = 0.125 # s\n";
        let mut doc: DocumentMut = text.parse().unwrap();
        let base = read(&doc);
        assert_eq!(
            base,
            [PadEntry {
                name: "Reprise*".into(),
                keys: [("controlpanel".to_string(), "run".to_string())].into(),
                dial_up: "plus".into(),
                ring_step: Some(7.5),
                strip_step: Some(0.125),
                ..PadEntry::default()
            }]
        );
        write(&mut doc, &base.clone(), &base);
        assert_eq!(doc.to_string(), text, "nothing moved, nothing written");

        let mut want = base.clone();
        let name = "Reprise Test Pad";
        bind(&mut want, name, Slot::DialDown, "minus");
        set_step(&mut want, name, Step::Ring, 30);
        write(&mut doc, &want, &base);
        let out = doc.to_string();
        assert_eq!(
            out,
            "[[tablet_pads]]\nname = \"Reprise*\"\nkeys = { controlpanel = \"run\" } # k\ndial_up = \"plus\" # d\nring_step = 30 # r\nstrip_step = 0.125 # s\ndial_down = \"minus\"\n"
        );
        assert_eq!(read(&out.parse().unwrap()), want);

        let base = want.clone();
        bind(&mut want, name, Slot::Key("onscreen_keyboard"), "mode");
        bind(&mut want, name, Slot::DialUp, "");
        set_step(&mut want, name, Step::Strip, 20);
        write(&mut doc, &want, &base);
        let out = doc.to_string();
        assert!(
            out.contains("keys = { controlpanel = \"run\", onscreen_keyboard = \"mode\" }"),
            "{out}"
        );
        assert!(!out.contains("dial_up"), "{out}");
        assert!(out.contains("strip_step = 0.2 # s"), "{out}");
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

    #[test]
    fn a_step_row_is_an_up_down_with_its_unit_after_it() {
        use reprisekit::controls::form::Control;
        use reprisekit::controls::updown;
        let p = Pad {
            dials: 1,
            keys: vec!["controlpanel".into()],
            ..pad("Pad", 2, 1, 1)
        };
        let entries = [PadEntry {
            strip_step: Some(0.25),
            ..PadEntry::default()
        }];
        let f = page(Area::MIN, &[p], 0, &entries);
        let r = |id: &str| f.item(id).unwrap().rect;
        let y = |row: i32| ROW0 + row * ROW_STEP;
        assert_eq!(r(ROW_IDS[2]).y, y(2) - 4, "the pad key under the buttons");
        assert_eq!(
            r(RING_STEP),
            Rect::new(GX + 100, y(5) - 4, updown::WIDTH, updown::HEIGHT)
        );
        assert_eq!(r(RING_UNITS), Rect::new(GX + 156, y(5), 48, 13));
        assert_eq!(r(STRIP_STEP).y, y(8) - 4);
        assert_eq!(r(ROW_IDS[10]).y, y(10) - 4, "dial down last");
        assert!(f.item(ROW_IDS[5]).is_none(), "a step row has no combo");
        let value = |id: &str| match &f.item(id).unwrap().control {
            Control::UpDown(u) => u.value(),
            _ => panic!("{id} is no up-down"),
        };
        assert_eq!(value(RING_STEP), 15, "the default when the file has none");
        assert_eq!(value(STRIP_STEP), 25);
        let no_ring = page(Area::MIN, &[pad("Pad", 2, 0, 1)], 0, &[]);
        assert!(no_ring.item(RING_STEP).is_none() && no_ring.item(STRIP_STEP).is_some());
    }
}
