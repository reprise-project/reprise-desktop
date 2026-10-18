//! The Sounds and Audio Devices sheet model. Nothing here talks to PipeWire: the host feeds it [`Snapshot`]s
//! and applies the [`Change`]s it hands back. Every change waits for Apply.

use std::collections::BTreeMap;

use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::listbox::ListBox;
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::DialogFont;
use reprisekit::trackbar::{self, TrackBar};

use crate::appicons::AppIcon;
use crate::pipewire::{App, Device, Snapshot};

pub const CLIENT: (i32, i32) = (361, 440);

const TABS: [&str; 2] = ["Audio", "Applications"];
pub const AUDIO: usize = 0;
pub const APPS: usize = 1;

const STEPS: usize = 21;
const ICON_CELL: i32 = 41;
const ICON_PX: i32 = 32;
const LIST_ROWS: usize = 4;

struct Row {
    sink: bool,
    group: &'static str,
    icon: &'static str,
    list: &'static str,
    set_default: &'static str,
    toggle: &'static str,
    mixer: &'static str,
    mixer_label: &'static str,
    volume_label: &'static str,
    track: &'static str,
    percent: &'static str,
    mute: &'static str,
    empty: &'static str,
}

const SINK: Row = Row {
    sink: true,
    group: "sink_group",
    icon: "sink_icon",
    list: "sink_list",
    set_default: "sink_set_default",
    toggle: "sink_toggle",
    mixer: "sink_mixer",
    mixer_label: "V&olume...",
    volume_label: "sink_volume_label",
    track: "sink_track",
    percent: "sink_percent",
    mute: "sink_mute",
    empty: "No Playback Devices",
};

const SOURCE: Row = Row {
    sink: false,
    group: "source_group",
    icon: "source_icon",
    list: "source_list",
    set_default: "source_set_default",
    toggle: "source_toggle",
    mixer: "source_mixer",
    mixer_label: "Vol&ume...",
    volume_label: "source_volume_label",
    track: "source_track",
    percent: "source_percent",
    mute: "source_mute",
    empty: "No Recording Devices",
};

const SINK_Y: i32 = 47;
const SOURCE_Y: i32 = 225;
const GROUP_H: i32 = 166;

/// The Applications page's controls: one group box as tall as the Audio
/// page's two, the program list filling it down to the volume row.
const APP_GROUP: &str = "app_group";
const APP_ICON: &str = "app_icon";
const APP_LIST: &str = "app_list";
const APP_VOLUME_LABEL: &str = "app_volume_label";
const APP_TRACK: &str = "app_track";
const APP_PERCENT: &str = "app_percent";
const APP_MUTE: &str = "app_mute";
const APP_EMPTY: &str = "No programs are playing sound";
const APP_ROWS: usize = 16;
/// The volume row's top: the same 12 px above the group's bottom edge
/// as on the Audio page.
const APP_TRACK_Y: i32 = SOURCE_Y + 128;

/// The theme's speaker and microphone, at 32 px for the group boxes and
/// 16 px for the list rows. A missing one becomes the magenta placeholder
/// in a group box and nothing in a row.
#[derive(Default)]
pub struct Icons {
    pub speaker: Option<Canvas>,
    pub mic: Option<Canvas>,
    pub speaker_small: Option<Canvas>,
    pub mic_small: Option<Canvas>,
}

/// What the user has picked, before Apply. Device indices are into the
/// matching list of [`Sheet::applied`].
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    pub sink: Option<usize>,
    pub source: Option<usize>,
    pub sink_volume: f64,
    pub sink_mute: bool,
    pub source_volume: f64,
    pub source_mute: bool,
    /// Cards to switch on (`true`) or off, by id; only the ones that
    /// differ from `applied`.
    pub cards: BTreeMap<u32, bool>,
    /// Programs' volumes and mutes, by [`App::key`]: the ones touched
    /// since the last Apply. One that has stopped playing is dropped.
    pub app_volume: BTreeMap<String, f64>,
    pub app_mute: BTreeMap<String, bool>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    Profile(u32, u32),
    Default(u32),
    Volume(u32, f64),
    Mute(u32, bool),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    None,
    Redraw,
    Apply(Vec<Change>),
    Close,
    ApplyAndClose(Vec<Change>),
    /// A Volume... button: open Volume Control, on its Recording view
    /// for the recording row's.
    VolumeControl {
        record: bool,
    },
}

/// One row of a device list: a device of the snapshot, by index, or a
/// card that is off (and so has no devices), by id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    Device(usize),
    Card(u32),
}

/// A row as a snapshot can move it: a device by `node.name` (with its
/// card, where its row goes when the card is switched off), or an off
/// card by id. Indices shift when devices come and go; these do not.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Held {
    Device(String, Option<u32>),
    Card(u32),
}

pub struct Sheet {
    pub sheet: PropSheet,
    /// The last snapshot the sheet was built or refreshed from: what
    /// `pending` is compared against to decide whether Apply lights.
    pub applied: Snapshot,
    pub pending: Pending,
    shown: [Vec<Held>; 2],
    /// Per direction, the default a Disable moved the pick off, by
    /// `node.name`: an Enable of the same card puts it back.
    displaced: [Option<String>; 2],
    small: [Option<Canvas>; 2],
    app_icons: BTreeMap<String, AppIcon>,
    shown_apps: Vec<String>,
}

impl Sheet {
    pub fn new(snap: &Snapshot, icons: Icons) -> Sheet {
        let mut items = row_items(&SINK, SINK_Y, "Sound playback", icons.speaker);
        items.extend(row_items(&SOURCE, SOURCE_Y, "Sound recording", icons.mic));
        let pages = vec![
            Form::new(items).set_in(DialogFont::ShellDlg),
            Form::new(app_items()),
        ];
        let mut sheet = Sheet {
            sheet: PropSheet::new(&TABS, pages, CLIENT),
            applied: snap.clone(),
            pending: Pending::from_snapshot(snap),
            shown: [Vec::new(), Vec::new()],
            displaced: [None, None],
            small: [icons.speaker_small, icons.mic_small],
            app_icons: BTreeMap::new(),
            shown_apps: Vec::new(),
        };
        sheet.write();
        sheet
    }

    /// Take a snapshot from outside: the lists always follow, the values
    /// only when the page has nothing pending.
    pub fn refresh(&mut self, snap: &Snapshot) {
        if self.dirty() {
            // Keep the picks on the devices they named, since the lists
            // may have grown or shrunk under them.
            let sink = self.selected_name(true);
            let source = self.selected_name(false);
            self.applied = snap.clone();
            self.pending.sink = repoint(&snap.sinks, sink.as_deref());
            self.pending.source = repoint(&snap.sources, source.as_deref());
            let applied = &self.applied;
            self.pending
                .cards
                .retain(|id, on| applied.cards.iter().any(|c| c.id == *id && c.off == *on));
            let playing = |key: &String| applied.apps.iter().any(|a| a.key == *key);
            self.pending.app_volume.retain(|k, _| playing(k));
            self.pending.app_mute.retain(|k, _| playing(k));
        } else {
            self.applied = snap.clone();
            self.pending = Pending::from_snapshot(snap);
            self.displaced = [None, None];
        }
        self.write();
    }

    pub fn dirty(&self) -> bool {
        !self.changes().is_empty()
    }

    pub fn event(&mut self, ev: propsheet::Event) -> Outcome {
        match ev {
            propsheet::Event::None => Outcome::None,
            propsheet::Event::Redraw => Outcome::Redraw,
            propsheet::Event::Page(_, form::Event::Changed(id)) => {
                self.read(id);
                Outcome::Redraw
            }
            propsheet::Event::Page(_, form::Event::Pressed(id)) if id == SINK.mixer => {
                Outcome::VolumeControl { record: false }
            }
            propsheet::Event::Page(_, form::Event::Pressed(id)) if id == SOURCE.mixer => {
                Outcome::VolumeControl { record: true }
            }
            propsheet::Event::Page(_, form::Event::Pressed(id)) => {
                self.press(id);
                Outcome::Redraw
            }
            propsheet::Event::Page(..) => Outcome::Redraw,
            propsheet::Event::Apply => match self.changes() {
                changes if changes.is_empty() => Outcome::Redraw,
                changes => {
                    self.accept();
                    Outcome::Apply(changes)
                }
            },
            propsheet::Event::Ok => match self.changes() {
                changes if changes.is_empty() => Outcome::Close,
                changes => {
                    self.accept();
                    Outcome::ApplyAndClose(changes)
                }
            },
            propsheet::Event::Cancel => {
                self.pending = Pending::from_snapshot(&self.applied);
                self.displaced = [None, None];
                self.write();
                Outcome::Close
            }
        }
    }

    fn list(&self, sink: bool) -> &[Device] {
        if sink {
            &self.applied.sinks
        } else {
            &self.applied.sources
        }
    }

    fn pick(&self, sink: bool) -> Option<usize> {
        if sink {
            self.pending.sink
        } else {
            self.pending.source
        }
    }

    fn set_pick(&mut self, sink: bool, i: Option<usize>) {
        if sink {
            self.pending.sink = i;
        } else {
            self.pending.source = i;
        }
    }

    /// Whether card `id` is on in the applied snapshot. A card the
    /// snapshot does not list counts as on.
    fn card_was_on(&self, id: u32) -> bool {
        self.applied
            .cards
            .iter()
            .find(|c| c.id == id)
            .is_none_or(|c| !c.off)
    }

    fn card_on(&self, id: u32) -> bool {
        self.pending
            .cards
            .get(&id)
            .copied()
            .unwrap_or_else(|| self.card_was_on(id))
    }

    fn device_on(&self, d: &Device) -> bool {
        d.card.is_none_or(|c| self.card_on(c))
    }

    /// Every row of a list: its devices, then each card that is off and
    /// carries that direction.
    fn entries(&self, sink: bool) -> Vec<Entry> {
        let devices = (0..self.list(sink).len()).map(Entry::Device);
        let cards = self
            .applied
            .cards
            .iter()
            .filter(|c| c.off && if sink { c.sinks } else { c.sources })
            .map(|c| Entry::Card(c.id));
        devices.chain(cards).collect()
    }

    fn entry_text(&self, e: Entry, sink: bool) -> (String, bool) {
        match e {
            Entry::Device(i) => {
                let d = &self.list(sink)[i];
                if !self.device_on(d) {
                    (format!("{} (Disabled)", d.label), true)
                } else if self.pick(sink) == Some(i) {
                    (format!("{} (Default)", d.label), false)
                } else {
                    (d.label.clone(), false)
                }
            }
            Entry::Card(id) => {
                let label = self
                    .applied
                    .cards
                    .iter()
                    .find(|c| c.id == id)
                    .map_or_else(String::new, |c| c.label.clone());
                if self.card_on(id) {
                    (label, false)
                } else {
                    (format!("{label} (Disabled)"), true)
                }
            }
        }
    }

    /// The entry the list of `ids` has selected, found again in the
    /// current snapshot: a device by name, or once its card is off, that
    /// card's row; a card's row, or once it is on, its first device.
    fn selected(&self, ids: &Row) -> Option<Entry> {
        let k = usize::from(!ids.sink);
        let i = match &self.sheet.pages[AUDIO].item(ids.list)?.control {
            Control::ListBox { list, .. } => list.selected?,
            _ => return None,
        };
        let entries = self.entries(ids.sink);
        let list = self.list(ids.sink);
        let card_row = |c: u32| entries.iter().copied().find(|e| *e == Entry::Card(c));
        match self.shown[k].get(i)? {
            Held::Device(name, card) => list
                .iter()
                .position(|d| d.name == *name)
                .map(Entry::Device)
                .or_else(|| card.and_then(card_row)),
            Held::Card(c) => card_row(*c).or_else(|| {
                list.iter()
                    .position(|d| d.card == Some(*c))
                    .map(Entry::Device)
            }),
        }
    }

    fn held(&self, e: Entry, sink: bool) -> Held {
        match e {
            Entry::Device(i) => {
                let d = &self.list(sink)[i];
                Held::Device(d.name.clone(), d.card)
            }
            Entry::Card(c) => Held::Card(c),
        }
    }

    fn entry_card(&self, e: Entry, sink: bool) -> Option<u32> {
        match e {
            Entry::Device(i) => self.list(sink)[i].card,
            Entry::Card(id) => Some(id),
        }
    }

    fn press(&mut self, id: &'static str) {
        for ids in [&SINK, &SOURCE] {
            let Some(e) = self.selected(ids) else {
                continue;
            };
            if id == ids.set_default {
                if let Entry::Device(i) = e
                    && self.device_on(&self.list(ids.sink)[i])
                {
                    self.set_pick(ids.sink, Some(i));
                    self.follow_selection(ids.sink);
                    self.displaced[usize::from(!ids.sink)] = None;
                }
            } else if id == ids.toggle
                && let Some(card) = self.entry_card(e, ids.sink)
            {
                let on = !self.card_on(card);
                if on == self.card_was_on(card) {
                    self.pending.cards.remove(&card);
                } else {
                    self.pending.cards.insert(card, on);
                }
                // A card carries both directions, so either default may
                // just have gone, or come back.
                for sink in [true, false] {
                    self.restore_displaced(sink);
                    self.leave_off_devices(sink);
                }
            }
        }
        self.write();
    }

    /// When the pending default of a direction is on a card that is now
    /// off, the first device still on takes over, and the one it replaced
    /// is remembered.
    fn leave_off_devices(&mut self, sink: bool) {
        let list = self.list(sink);
        let Some(gone) = self.pick(sink).and_then(|i| list.get(i)) else {
            return;
        };
        if self.device_on(gone) {
            return;
        }
        let gone = gone.name.clone();
        let k = usize::from(!sink);
        self.displaced[k].get_or_insert(gone);
        let list = self.list(sink);
        let next = list.iter().position(|d| self.device_on(d));
        self.set_pick(sink, next);
        self.follow_selection(sink);
    }

    /// A default a Disable moved the pick off is the pick again once its
    /// card is back on.
    fn restore_displaced(&mut self, sink: bool) {
        let k = usize::from(!sink);
        let Some(name) = &self.displaced[k] else {
            return;
        };
        let list = self.list(sink);
        let Some(i) = list.iter().position(|d| d.name == *name) else {
            return;
        };
        if !self.device_on(&list[i]) {
            return;
        }
        self.displaced[k] = None;
        self.set_pick(sink, Some(i));
        self.follow_selection(sink);
    }

    /// Every write the pending state asks for: the profiles first, since
    /// a default can only land on a device that exists, then the playback
    /// row and the recording row.
    fn changes(&self) -> Vec<Change> {
        let mut out = Vec::new();
        for (&id, &on) in &self.pending.cards {
            let Some(c) = self.applied.cards.iter().find(|c| c.id == id) else {
                continue;
            };
            if on == !c.off {
                continue;
            }
            if let Some(index) = if on { c.best } else { c.off_index } {
                out.push(Change::Profile(id, index));
            }
        }
        let p = &self.pending;
        for (sink, sel, volume, mute) in [
            (true, p.sink, p.sink_volume, p.sink_mute),
            (false, p.source, p.source_volume, p.source_mute),
        ] {
            let sel = sel.filter(|&i| self.list(sink).get(i).is_some_and(|d| self.device_on(d)));
            let default = if sink {
                self.applied.default_sink.as_deref()
            } else {
                self.applied.default_source.as_deref()
            };
            row_changes(self.list(sink), default, sel, volume, mute, &mut out);
        }
        for app in &self.applied.apps {
            let (volume, mute) = self.app_values(app);
            for id in app.ids() {
                if pos_of(volume) != pos_of(app.volume) {
                    out.push(Change::Volume(id, quantized(volume)));
                }
                if mute != app.mute {
                    out.push(Change::Mute(id, mute));
                }
            }
        }
        out
    }

    fn app_values(&self, app: &App) -> (f64, bool) {
        let p = &self.pending;
        (
            p.app_volume.get(&app.key).copied().unwrap_or(app.volume),
            p.app_mute.get(&app.key).copied().unwrap_or(app.mute),
        )
    }

    pub fn set_app_icons(&mut self, icons: BTreeMap<String, AppIcon>) {
        self.app_icons = icons;
        self.write();
    }

    pub fn open(&mut self, tab: usize) {
        self.sheet.tabs.open = tab;
    }

    fn selected_app(&self) -> Option<&App> {
        let i = match &self.sheet.pages[APPS].item(APP_LIST)?.control {
            Control::ListBox { list, .. } => list.selected?,
            _ => return None,
        };
        let key = self.shown_apps.get(i)?;
        self.applied.apps.iter().find(|a| a.key == *key)
    }

    fn selected_name(&self, sink: bool) -> Option<String> {
        self.pick(sink)
            .and_then(|i| self.list(sink).get(i))
            .map(|d| d.name.clone())
    }

    /// Fold the pending state into `applied` so Apply greys at once. The
    /// `pw-dump` that follows the writes is what really settles it; this
    /// only spares the sheet a lit Apply in between.
    fn accept(&mut self) {
        let p = self.pending.clone();
        for (id, on) in &p.cards {
            if let Some(c) = self.applied.cards.iter_mut().find(|c| c.id == *id) {
                c.off = !on;
            }
        }
        self.pending.cards.clear();
        if let Some(name) = self.selected_name(true) {
            let i = p.sink.expect("selected_name implies a pick");
            let d = &mut self.applied.sinks[i];
            d.volume = quantized(p.sink_volume);
            d.mute = p.sink_mute;
            self.applied.default_sink = Some(name);
        }
        if let Some(name) = self.selected_name(false) {
            let i = p.source.expect("selected_name implies a pick");
            let d = &mut self.applied.sources[i];
            d.volume = quantized(p.source_volume);
            d.mute = p.source_mute;
            self.applied.default_source = Some(name);
        }
        for app in &mut self.applied.apps {
            if let Some(v) = p.app_volume.get(&app.key) {
                app.volume = quantized(*v);
            }
            if let Some(m) = p.app_mute.get(&app.key) {
                app.mute = *m;
            }
        }
        self.pending.app_volume.clear();
        self.pending.app_mute.clear();
        self.displaced = [None, None];
        self.write();
    }

    /// The host could not write what `accept` took: `snap` (what
    /// PipeWire last reported) is the baseline again and the picks stay
    /// pending with Apply live.
    pub fn unapplied(&mut self, snap: &Snapshot) {
        let sink = self.selected_name(true);
        let source = self.selected_name(false);
        // `accept` folded the card switches into `applied`; whatever
        // differs from the snapshot is pending again.
        for c in &self.applied.cards {
            if snap.cards.iter().any(|s| s.id == c.id && s.off != c.off) {
                self.pending.cards.insert(c.id, !c.off);
            }
        }
        for app in &self.applied.apps {
            let Some(was) = snap.apps.iter().find(|a| a.key == app.key) else {
                continue;
            };
            if pos_of(was.volume) != pos_of(app.volume) {
                self.pending.app_volume.insert(app.key.clone(), app.volume);
            }
            if was.mute != app.mute {
                self.pending.app_mute.insert(app.key.clone(), app.mute);
            }
        }
        self.applied = snap.clone();
        self.pending.sink = repoint(&snap.sinks, sink.as_deref());
        self.pending.source = repoint(&snap.sources, source.as_deref());
        self.write();
    }

    fn read(&mut self, id: &'static str) {
        let page = &self.sheet.pages[AUDIO];
        let pos = |id| page.track_pos(id);
        let checked = |id| page.checked(id);
        let apps = &self.sheet.pages[APPS];
        let key = self.selected_app().map(|a| a.key.clone());
        match id {
            _ if id == APP_TRACK => {
                if let (Some(p), Some(k)) = (apps.track_pos(id), key) {
                    self.pending.app_volume.insert(k, volume_of(p));
                }
            }
            _ if id == APP_MUTE => {
                if let (Some(c), Some(k)) = (apps.checked(id), key) {
                    self.pending.app_mute.insert(k, c);
                }
            }
            _ if id == SINK.track => {
                if let Some(p) = pos(id) {
                    self.pending.sink_volume = volume_of(p);
                }
            }
            _ if id == SOURCE.track => {
                if let Some(p) = pos(id) {
                    self.pending.source_volume = volume_of(p);
                }
            }
            _ if id == SINK.mute => {
                if let Some(c) = checked(id) {
                    self.pending.sink_mute = c;
                }
            }
            _ if id == SOURCE.mute => {
                if let Some(c) = checked(id) {
                    self.pending.source_mute = c;
                }
            }
            _ => {}
        }
        self.write();
    }

    /// After the pending default moves, the trackbar and the check show
    /// the newly picked device's own volume and mute.
    fn follow_selection(&mut self, sink: bool) {
        let d = self.pick(sink).and_then(|i| self.list(sink).get(i));
        let (v, m) = d.map(|d| (d.volume, d.mute)).unwrap_or((0.0, false));
        if sink {
            self.pending.sink_volume = v;
            self.pending.sink_mute = m;
        } else {
            self.pending.source_volume = v;
            self.pending.source_mute = m;
        }
    }

    fn write(&mut self) {
        for ids in [&SINK, &SOURCE] {
            self.write_row(ids);
        }
        self.write_apps();
        let dirty = self.dirty();
        self.sheet.set_apply_enabled(dirty);
    }

    /// Rewrite one group box: the list (keeping its selection on the same
    /// entry), the buttons for that entry, the trackbar, the percentage
    /// and Mute.
    fn write_row(&mut self, ids: &Row) {
        let k = usize::from(!ids.sink);
        let old = self.selected(ids);
        let entries = self.entries(ids.sink);
        let rows: Vec<(String, bool)> = entries
            .iter()
            .map(|&e| self.entry_text(e, ids.sink))
            .collect();
        let pick = self.pick(ids.sink);
        let sel = old
            .and_then(|o| entries.iter().position(|e| *e == o))
            .or_else(|| pick.and_then(|i| entries.iter().position(|e| *e == Entry::Device(i))))
            .or_else(|| (!entries.is_empty()).then_some(0));
        let on = pick
            .and_then(|i| self.list(ids.sink).get(i))
            .is_some_and(|d| self.device_on(d));
        let (volume, mute) = if ids.sink {
            (self.pending.sink_volume, self.pending.sink_mute)
        } else {
            (self.pending.source_volume, self.pending.source_mute)
        };
        let chosen = sel.and_then(|i| entries.get(i).copied());
        let can_default = match chosen {
            Some(Entry::Device(i)) => self.device_on(&self.list(ids.sink)[i]) && pick != Some(i),
            _ => false,
        };
        let card = chosen.and_then(|e| self.entry_card(e, ids.sink));
        let (toggle_label, can_toggle) = match card {
            Some(id) => {
                let c = self.applied.cards.iter().find(|c| c.id == id);
                if self.card_on(id) {
                    ("&Disable", c.is_some_and(|c| c.off_index.is_some()))
                } else {
                    ("&Enable", c.is_some_and(|c| c.best.is_some()))
                }
            }
            None => ("&Disable", false),
        };
        let icon = self.small[k].clone();
        let page = &mut self.sheet.pages[AUDIO];
        if let Some(item) = page.item_mut(ids.list) {
            let (items, greyed): (Vec<String>, Vec<bool>) = if rows.is_empty() {
                (vec![ids.empty.to_string()], vec![true])
            } else {
                rows.into_iter().unzip()
            };
            let top = match &item.control {
                Control::ListBox { list, .. } if list.items.len() == items.len() => list.top,
                _ => 0,
            };
            let icons = if entries.is_empty() {
                vec![None]
            } else {
                vec![icon; entries.len()]
            };
            let mut list = ListBox::new(items, None);
            list.greyed = greyed;
            list.top = top;
            if let Some(i) = sel {
                list.select(i, LIST_ROWS);
            }
            item.control = Control::ListBox { list, icons };
        }
        if let Some(item) = page.item_mut(ids.set_default) {
            item.control = button("Set &Default", can_default);
        }
        if let Some(item) = page.item_mut(ids.toggle) {
            item.control = button(toggle_label, can_toggle);
        }
        // Through set_track, so a thumb the device moves to or from under
        // the pointer turns hot or cold.
        page.set_enabled(ids.track, on);
        page.set_track(ids.track, pos_of(volume));
        page.set_text(ids.percent, &format!("{}%", pos_of(volume) * 5));
        page.set_enabled(ids.percent, on);
        page.set_enabled(ids.volume_label, on);
        if let Some(item) = page.item_mut(ids.mute) {
            item.control = Control::Check {
                label: "&Mute".into(),
                checked: mute && on,
                enabled: on,
                mixed: None,
            };
        }
        self.shown[k] = entries.iter().map(|&e| self.held(e, ids.sink)).collect();
    }
}

impl Sheet {
    /// Rewrite the Applications page: the program list (keeping its
    /// selection on the same program), the chosen one's icon, and its
    /// trackbar, percentage and Mute.
    fn write_apps(&mut self) {
        let old = self.selected_app().map(|a| a.key.clone());
        let apps = &self.applied.apps;
        let sel = old
            .and_then(|k| apps.iter().position(|a| a.key == k))
            .or_else(|| (!apps.is_empty()).then_some(0));
        let rows: Vec<(String, String, Option<Canvas>)> = apps
            .iter()
            .map(|a| {
                let (volume, mute) = self.app_values(a);
                let detail = if mute {
                    "Muted".to_string()
                } else {
                    format!("{}%", pos_of(volume) * 5)
                };
                let icon = self.app_icons.get(&a.key).and_then(|i| i.small.clone());
                (a.label.clone(), detail, icon)
            })
            .collect();
        let chosen = sel.map(|i| &apps[i]);
        let (volume, mute) = chosen.map_or((0.0, false), |a| self.app_values(a));
        let large = chosen
            .and_then(|a| self.app_icons.get(&a.key))
            .and_then(|i| i.large.clone());
        let on = chosen.is_some();
        let keys = apps.iter().map(|a| a.key.clone()).collect();

        let page = &mut self.sheet.pages[APPS];
        if let Some(item) = page.item_mut(APP_LIST) {
            let top = match &item.control {
                Control::ListBox { list, .. } if list.items.len() == rows.len() => list.top,
                _ => 0,
            };
            let (mut list, icons) = if rows.is_empty() {
                let mut l = ListBox::new(vec![APP_EMPTY.to_string()], None);
                l.greyed = vec![true];
                (l, vec![None])
            } else {
                let mut items = Vec::new();
                let mut detail = Vec::new();
                let mut icons = Vec::new();
                for (label, d, icon) in rows {
                    items.push(label);
                    detail.push(d);
                    icons.push(icon);
                }
                let mut l = ListBox::new(items, None);
                l.detail = detail;
                (l, icons)
            };
            list.top = top;
            if let Some(i) = sel {
                list.select(i, APP_ROWS);
            }
            item.control = Control::ListBox { list, icons };
        }
        if let Some(item) = page.item_mut(APP_ICON) {
            item.control = Control::Picture {
                canvas: large,
                frame: 0,
            };
        }
        page.set_enabled(APP_TRACK, on);
        page.set_track(APP_TRACK, pos_of(volume));
        page.set_text(
            APP_PERCENT,
            &if on {
                format!("{}%", pos_of(volume) * 5)
            } else {
                String::new()
            },
        );
        page.set_enabled(APP_PERCENT, on);
        page.set_enabled(APP_VOLUME_LABEL, on);
        if let Some(item) = page.item_mut(APP_MUTE) {
            item.control = Control::Check {
                label: "&Mute".into(),
                checked: mute && on,
                enabled: on,
                mixed: None,
            };
        }
        self.shown_apps = keys;
    }
}

fn app_items() -> Vec<Item> {
    let inset = (ICON_CELL - ICON_PX) / 2;
    let y = SINK_Y;
    let list_h = APP_ROWS as i32 * 17 + 4;
    vec![
        Item::new(
            APP_GROUP,
            Rect::new(21, y, 320, SOURCE_Y + GROUP_H - y),
            Control::Group("Program volume".into()),
        ),
        Item::new(
            APP_ICON,
            Rect::new(28 + inset, y + 18 + inset, ICON_PX, ICON_PX),
            Control::Picture {
                canvas: None,
                frame: 0,
            },
        ),
        Item::new(
            APP_LIST,
            Rect::new(70, y + 18, 261, list_h),
            Control::ListBox {
                list: ListBox::new(Vec::new(), None),
                icons: Vec::new(),
            },
        ),
        Item::label_at(APP_VOLUME_LABEL, 71, APP_TRACK_Y + 4, 44, "&Volume:"),
        Item::new(
            APP_TRACK,
            Rect::new(120, APP_TRACK_Y, 130, trackbar::HEIGHT),
            Control::TrackBar(TrackBar::new(0, STEPS)),
        ),
        Item::label_at(APP_PERCENT, 256, APP_TRACK_Y + 4, 30, ""),
        Item::new(
            APP_MUTE,
            Rect::new(290, APP_TRACK_Y + 8, 13, 13),
            Control::Check {
                label: "&Mute".into(),
                checked: false,
                enabled: true,
                mixed: None,
            },
        ),
    ]
}

fn button(label: &str, enabled: bool) -> Control {
    Control::Button {
        label: label.into(),
        enabled,
        default: false,
    }
}

fn pos_of(volume: f64) -> usize {
    ((volume * (STEPS - 1) as f64)
        .round()
        .clamp(0.0, (STEPS - 1) as f64)) as usize
}

fn volume_of(pos: usize) -> f64 {
    pos as f64 / (STEPS - 1) as f64
}

/// A volume rounded to the trackbar's step, which is all the sheet can
/// express and so all it ever compares.
fn quantized(volume: f64) -> f64 {
    volume_of(pos_of(volume))
}

/// The index of the default device in `list`: the one the metadata names,
/// else the first. An empty list has none.
fn default_index(list: &[Device], default: Option<&str>) -> Option<usize> {
    default
        .and_then(|n| list.iter().position(|d| d.name == n))
        .or_else(|| (!list.is_empty()).then_some(0))
}

fn repoint(list: &[Device], name: Option<&str>) -> Option<usize> {
    name.and_then(|n| list.iter().position(|d| d.name == n))
        .or_else(|| (!list.is_empty()).then_some(0))
}

/// One row's writes. Picking the device that is already default is not a
/// change, so a sheet whose metadata names nothing still opens clean.
fn row_changes(
    list: &[Device],
    default: Option<&str>,
    sel: Option<usize>,
    volume: f64,
    mute: bool,
    out: &mut Vec<Change>,
) {
    let Some(d) = sel.and_then(|i| list.get(i)) else {
        return;
    };
    if sel != default_index(list, default) {
        out.push(Change::Default(d.id));
    }
    if pos_of(volume) != pos_of(d.volume) {
        out.push(Change::Volume(d.id, quantized(volume)));
    }
    if mute != d.mute {
        out.push(Change::Mute(d.id, mute));
    }
}

fn row_items(ids: &Row, y: i32, title: &str, icon: Option<Canvas>) -> Vec<Item> {
    let inset = (ICON_CELL - ICON_PX) / 2;
    vec![
        Item::new(
            ids.group,
            Rect::new(21, y, 320, GROUP_H),
            Control::Group(title.into()),
        ),
        Item::new(
            ids.icon,
            Rect::new(28 + inset, y + 18 + inset, ICON_PX, ICON_PX),
            Control::Picture {
                canvas: Some(icon.unwrap_or_else(|| placeholder(ICON_PX))),
                frame: 0,
            },
        ),
        Item::new(
            ids.list,
            Rect::new(70, y + 18, 261, 72),
            Control::ListBox {
                list: ListBox::new(Vec::new(), None),
                icons: Vec::new(),
            },
        ),
        // The row fills the list's width: two 74 px buttons and the
        // 105 px Volume..., 4 px apart.
        Item::new(
            ids.set_default,
            Rect::new(70, y + 96, 74, 23),
            button("Set &Default", false),
        ),
        Item::new(
            ids.toggle,
            Rect::new(148, y + 96, 74, 23),
            button("&Disable", false),
        ),
        Item::new(
            ids.mixer,
            Rect::new(226, y + 96, 105, 23),
            button(ids.mixer_label, true),
        ),
        // A label's text is drawn from the rect's left edge, centred in its
        // height.
        Item::label_at(ids.volume_label, 71, y + 132, 44, "&Volume:"),
        Item::new(
            ids.track,
            Rect::new(120, y + 128, 130, trackbar::HEIGHT),
            Control::TrackBar(TrackBar::new(0, STEPS)),
        ),
        Item::label_at(ids.percent, 256, y + 132, 30, ""),
        Item::new(
            ids.mute,
            Rect::new(290, y + 136, 13, 13),
            Control::Check {
                label: "&Mute".into(),
                checked: false,
                enabled: true,
                mixed: None,
            },
        ),
    ]
}

impl Pending {
    fn from_snapshot(snap: &Snapshot) -> Pending {
        let sink = default_index(&snap.sinks, snap.default_sink.as_deref());
        let source = default_index(&snap.sources, snap.default_source.as_deref());
        let at = |list: &[Device], i: Option<usize>| {
            i.and_then(|i| list.get(i))
                .map(|d| (d.volume, d.mute))
                .unwrap_or((0.0, false))
        };
        let (sink_volume, sink_mute) = at(&snap.sinks, sink);
        let (source_volume, source_mute) = at(&snap.sources, source);
        Pending {
            sink,
            source,
            sink_volume,
            sink_mute,
            source_volume,
            source_mute,
            cards: BTreeMap::new(),
            app_volume: BTreeMap::new(),
            app_mute: BTreeMap::new(),
        }
    }
}

fn placeholder(px: i32) -> Canvas {
    reprisekit::classic::widgets::placeholder(px, px)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipewire::Card;
    use reprisekit::text::TextRenderer;
    use reprisekit::theme::{FontConfig, Theme};

    fn dev(id: u32, name: &str, label: &str, volume: f64, mute: bool, card: Option<u32>) -> Device {
        Device {
            id,
            name: name.into(),
            label: label.into(),
            volume,
            mute,
            card,
            channels: Default::default(),
            route: None,
        }
    }

    fn card(id: u32, label: &str, off: bool) -> Card {
        Card {
            id,
            label: label.into(),
            off,
            sinks: true,
            sources: true,
            off_index: Some(0),
            best: Some(2),
        }
    }

    /// A null sink with no card, the built-in card's speaker (default)
    /// and mic, and a USB card that is switched off.
    fn fixture() -> Snapshot {
        Snapshot {
            sinks: vec![
                dev(51, "auto_null", "Dummy Output", 1.0, false, None),
                dev(52, "speaker", "Speaker", 0.5, false, Some(40)),
            ],
            sources: vec![dev(60, "mic", "Built-in Microphone", 0.8, true, Some(40))],
            default_sink: Some("speaker".into()),
            default_source: Some("mic".into()),
            cards: vec![
                card(40, "Built-in Audio", false),
                card(41, "USB Headset", true),
            ],
            apps: Vec::new(),
        }
    }

    fn sheet(snap: &Snapshot) -> Sheet {
        Sheet::new(snap, Icons::default())
    }

    /// Both pages fit their text under every interface and text size.
    #[test]
    fn the_pages_fit_their_text() {
        let over = reprisekit::interfaces::form_overflows(|_, _, _| {
            let s = sheet(&fixture()).sheet;
            s.tabs.labels.into_iter().zip(s.pages).collect()
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    fn list(s: &Sheet, id: &str) -> ListBox {
        match &s.sheet.open().item(id).expect("list exists").control {
            Control::ListBox { list, .. } => list.clone(),
            other => panic!("{id} is {other:?}"),
        }
    }

    fn button_state(s: &Sheet, id: &str) -> (String, bool) {
        match &s.sheet.open().item(id).expect("button exists").control {
            Control::Button { label, enabled, .. } => (label.clone(), *enabled),
            other => panic!("{id} is {other:?}"),
        }
    }

    fn label_text(s: &Sheet, id: &str) -> String {
        match &s.sheet.open().item(id).expect("label exists").control {
            Control::Label { text, .. } => text.clone(),
            other => panic!("{id} is {other:?}"),
        }
    }

    fn track(s: &Sheet, id: &str) -> TrackBar {
        match &s.sheet.open().item(id).expect("trackbar exists").control {
            Control::TrackBar(t) => *t,
            other => panic!("{id} is {other:?}"),
        }
    }

    fn check(s: &Sheet, id: &str) -> (bool, bool) {
        match &s.sheet.open().item(id).expect("check exists").control {
            Control::Check {
                checked, enabled, ..
            } => (*checked, *enabled),
            other => panic!("{id} is {other:?}"),
        }
    }

    fn select(s: &mut Sheet, id: &'static str, row: usize) {
        s.sheet.open_mut().focus_id(id);
        let ev = s.sheet.key(form::Key::Home, None);
        s.event(ev);
        for _ in 0..row {
            let ev = s.sheet.key(form::Key::Down, None);
            s.event(ev);
        }
    }

    fn press_button(s: &mut Sheet, id: &'static str) -> Outcome {
        s.event(propsheet::Event::Page(0, form::Event::Pressed(id)))
    }

    #[test]
    fn each_rows_volume_button_opens_volume_control_on_its_side() {
        let mut s = sheet(&fixture());
        assert_eq!(
            rect_of(&s, SINK.mixer),
            Rect::new(226, SINK_Y + 96, 105, 23)
        );
        assert_eq!(
            rect_of(&s, SOURCE.mixer),
            Rect::new(226, SOURCE_Y + 96, 105, 23)
        );
        assert_eq!(
            rect_of(&s, SINK.mixer).right(),
            rect_of(&s, SINK.list).right(),
            "flush with the list"
        );
        assert_eq!(
            press_button(&mut s, SINK.mixer),
            Outcome::VolumeControl { record: false }
        );
        assert_eq!(
            press_button(&mut s, SOURCE.mixer),
            Outcome::VolumeControl { record: true }
        );
        assert!(!s.dirty(), "opening it changes nothing");
    }

    #[test]
    fn the_lists_show_devices_then_disabled_cards() {
        let s = sheet(&fixture());
        let l = list(&s, SINK.list);
        assert_eq!(
            l.items,
            vec![
                "Dummy Output",
                "Speaker (Default)",
                "USB Headset (Disabled)"
            ]
        );
        assert_eq!(l.greyed, vec![false, false, true]);
        assert_eq!(l.selected, Some(1), "the default is selected");
        assert_eq!(
            list(&s, SOURCE.list).items,
            vec!["Built-in Microphone (Default)", "USB Headset (Disabled)"]
        );
        assert_eq!(
            button_state(&s, SINK.set_default),
            ("Set &Default".into(), false),
            "already the default"
        );
        assert_eq!(button_state(&s, SINK.toggle), ("&Disable".into(), true));
        assert!(!s.dirty(), "a sheet opens with nothing to apply");
    }

    #[test]
    fn set_default_pends_until_apply() {
        let mut s = sheet(&fixture());
        select(&mut s, SINK.list, 0);
        assert!(button_state(&s, SINK.set_default).1);
        press_button(&mut s, SINK.set_default);
        assert_eq!(list(&s, SINK.list).items[0], "Dummy Output (Default)");
        assert_eq!(track(&s, SINK.track).pos, 20, "Dummy Output's own volume");
        assert_eq!(
            s.event(propsheet::Event::Apply),
            Outcome::Apply(vec![Change::Default(51)])
        );
        assert!(!s.dirty(), "Apply greys again until PipeWire disagrees");
    }

    #[test]
    fn a_device_without_a_card_cannot_be_disabled() {
        let mut s = sheet(&fixture());
        select(&mut s, SINK.list, 0);
        assert_eq!(button_state(&s, SINK.toggle), ("&Disable".into(), false));
    }

    #[test]
    fn disabling_the_default_card_moves_the_default() {
        let mut s = sheet(&fixture());
        select(&mut s, SINK.list, 1);
        press_button(&mut s, SINK.toggle);
        let l = list(&s, SINK.list);
        assert_eq!(
            l.items[0], "Dummy Output (Default)",
            "the next enabled device"
        );
        assert_eq!(l.items[1], "Speaker (Disabled)");
        assert_eq!(button_state(&s, SINK.toggle).0, "&Enable");
        assert_eq!(
            list(&s, SOURCE.list).items[0],
            "Built-in Microphone (Disabled)"
        );
        assert!(!track(&s, SOURCE.track).enabled);
        assert_eq!(
            s.event(propsheet::Event::Apply),
            Outcome::Apply(vec![Change::Profile(40, 0), Change::Default(51)])
        );
    }

    #[test]
    fn enabling_an_off_card_picks_its_best_profile() {
        let mut s = sheet(&fixture());
        select(&mut s, SINK.list, 2);
        assert_eq!(button_state(&s, SINK.toggle), ("&Enable".into(), true));
        assert!(
            !button_state(&s, SINK.set_default).1,
            "no device to make default yet"
        );
        press_button(&mut s, SINK.toggle);
        assert_eq!(list(&s, SINK.list).items[2], "USB Headset");
        assert_eq!(
            s.event(propsheet::Event::Apply),
            Outcome::Apply(vec![Change::Profile(41, 2)])
        );
    }

    #[test]
    fn a_toggle_pressed_twice_leaves_nothing_pending() {
        let mut s = sheet(&fixture());
        select(&mut s, SINK.list, 2);
        press_button(&mut s, SINK.toggle);
        press_button(&mut s, SINK.toggle);
        assert!(!s.dirty());
    }

    #[test]
    fn disabling_and_enabling_the_default_card_puts_the_defaults_back() {
        let mut s = sheet(&fixture());
        select(&mut s, SINK.list, 1);
        press_button(&mut s, SINK.toggle);
        press_button(&mut s, SINK.toggle);
        assert!(!s.dirty(), "nothing is left to apply");
        assert_eq!(list(&s, SINK.list).items[1], "Speaker (Default)");
        assert_eq!(
            list(&s, SOURCE.list).items[0],
            "Built-in Microphone (Default)"
        );
        assert!(track(&s, SOURCE.track).enabled);
    }

    #[test]
    fn after_a_disable_lands_the_selection_follows_the_card() {
        let mut snap = fixture();
        snap.sinks = vec![
            dev(52, "speaker", "Speaker", 0.5, false, Some(40)),
            dev(53, "loop", "Loopback", 0.5, false, Some(45)),
        ];
        snap.cards.push(card(45, "Loopback Card", false));
        let mut s = sheet(&snap);
        select(&mut s, SINK.list, 0);
        press_button(&mut s, SINK.toggle);
        s.event(propsheet::Event::Apply);

        let mut next = snap.clone();
        next.sinks.remove(0);
        next.sources.clear();
        next.cards[0].off = true;
        next.default_sink = Some("loop".into());
        s.refresh(&next);
        let l = list(&s, SINK.list);
        assert_eq!(
            l.items,
            vec![
                "Loopback (Default)",
                "Built-in Audio (Disabled)",
                "USB Headset (Disabled)"
            ]
        );
        assert_eq!(
            l.selected,
            Some(1),
            "still on the card that was switched off"
        );
        assert_eq!(button_state(&s, SINK.toggle).0, "&Enable");
    }

    #[test]
    fn the_percentage_follows_the_trackbar() {
        let mut s = sheet(&fixture());
        assert_eq!(label_text(&s, SINK.percent), "50%");
        s.sheet.open_mut().focus_id(SINK.track);
        let ev = s.sheet.key(form::Key::Right, None);
        s.event(ev);
        assert_eq!(label_text(&s, SINK.percent), "55%");
        assert_eq!(
            s.event(propsheet::Event::Apply),
            Outcome::Apply(vec![Change::Volume(52, 0.55)])
        );
    }

    #[test]
    fn a_failed_apply_leaves_the_picks_pending() {
        let mut s = sheet(&fixture());
        select(&mut s, SINK.list, 2);
        press_button(&mut s, SINK.toggle);
        select(&mut s, SINK.list, 0);
        press_button(&mut s, SINK.set_default);
        let want = Outcome::Apply(vec![Change::Profile(41, 2), Change::Default(51)]);
        assert_eq!(s.event(propsheet::Event::Apply), want);
        assert!(!s.dirty());
        s.unapplied(&fixture());
        assert!(s.dirty(), "the picks are pending again");
        assert_eq!(s.pending.sink, Some(0), "and still name Dummy Output");
        assert!(matches!(
            s.sheet.open().item(propsheet::APPLY).map(|i| &i.control),
            Some(Control::Button { enabled: true, .. })
        ));
        assert_eq!(
            s.event(propsheet::Event::Apply),
            want,
            "a second Apply writes the same changes"
        );
    }

    #[test]
    fn ok_applies_and_closes_and_cancel_discards() {
        let mut s = sheet(&fixture());
        s.sheet.open_mut().focus_id(SINK.mute);
        let ev = s.sheet.key(form::Key::Space, None);
        s.event(ev);
        assert_eq!(
            s.event(propsheet::Event::Ok),
            Outcome::ApplyAndClose(vec![Change::Mute(52, true)])
        );

        let mut s = sheet(&fixture());
        s.sheet.open_mut().focus_id(SINK.mute);
        let ev = s.sheet.key(form::Key::Space, None);
        s.event(ev);
        select(&mut s, SINK.list, 2);
        press_button(&mut s, SINK.toggle);
        assert_eq!(s.event(propsheet::Event::Cancel), Outcome::Close);
        assert!(!s.dirty(), "Cancel puts the sheet back, cards included");
        assert_eq!(check(&s, SINK.mute), (false, true));

        let mut s = sheet(&fixture());
        assert_eq!(s.event(propsheet::Event::Ok), Outcome::Close);
    }

    #[test]
    fn an_outside_refresh_updates_a_clean_page_and_spares_a_dirty_one() {
        let mut s = sheet(&fixture());
        let mut next = fixture();
        next.sinks[1].volume = 0.25;
        next.sinks[1].mute = true;
        s.refresh(&next);

        assert_eq!(track(&s, SINK.track).pos, 5);
        assert_eq!(label_text(&s, SINK.percent), "25%");
        assert_eq!(check(&s, SINK.mute), (true, true));
        assert!(!s.dirty(), "the page followed PipeWire, so nothing pends");

        let mut s = sheet(&fixture());
        s.sheet.open_mut().focus_id(SINK.track);
        let ev = s.sheet.key(form::Key::Right, None);
        s.event(ev);
        s.refresh(&next);
        assert_eq!(track(&s, SINK.track).pos, 11, "the pending volume stands");
        assert!(s.dirty());
    }

    #[test]
    fn a_refresh_keeps_the_pending_default_on_the_same_device() {
        let mut s = sheet(&fixture());
        select(&mut s, SINK.list, 0);
        press_button(&mut s, SINK.set_default);
        assert_eq!(s.pending.sink, Some(0), "auto_null picked");

        let mut next = fixture();
        next.sinks
            .insert(0, dev(70, "usb", "USB Speaker", 1.0, false, None));
        s.refresh(&next);

        assert_eq!(s.pending.sink, Some(1), "still auto_null");
        assert_eq!(list(&s, SINK.list).items[1], "Dummy Output (Default)");
        assert_eq!(
            s.event(propsheet::Event::Apply),
            Outcome::Apply(vec![Change::Default(51)])
        );
    }

    #[test]
    fn an_empty_list_greys_its_row() {
        let s = sheet(&Snapshot::default());

        let l = list(&s, SINK.list);
        assert_eq!(l.items, vec!["No Playback Devices"]);
        assert_eq!(l.greyed, vec![true]);
        assert_eq!(l.selected, None);
        assert_eq!(list(&s, SOURCE.list).items, vec!["No Recording Devices"]);
        assert!(!button_state(&s, SINK.set_default).1);
        assert!(!button_state(&s, SINK.toggle).1);
        assert!(!track(&s, SINK.track).enabled);
        assert_eq!(check(&s, SINK.mute), (false, false));
        assert!(!s.dirty(), "nothing to apply with no devices");
    }

    #[test]
    fn the_page_geometry() {
        let s = sheet(&fixture());
        let page = s.sheet.open();
        let rect = |id: &str| page.item(id).expect("item exists").rect;

        assert_eq!(rect(SINK.group), Rect::new(21, 47, 320, 166));
        assert_eq!(rect(SOURCE.group), Rect::new(21, 225, 320, 166));
        assert_eq!(rect(SINK.icon), Rect::new(32, 69, 32, 32));
        assert_eq!(rect(SINK.list), Rect::new(70, 65, 261, 72));
        assert_eq!(rect(SINK.set_default), Rect::new(70, 143, 74, 23));
        assert_eq!(rect(SINK.toggle), Rect::new(148, 143, 74, 23));
        assert_eq!(rect(SINK.volume_label), Rect::new(71, 179, 44, 13));
        assert_eq!(rect(SINK.track), Rect::new(120, 175, 130, trackbar::HEIGHT));
        assert_eq!(rect(SINK.percent), Rect::new(256, 179, 30, 13));
        assert_eq!(rect(SINK.mute), Rect::new(290, 183, 13, 13));
        assert_eq!(rect(SOURCE.list), Rect::new(70, 243, 261, 72));
        assert_eq!(s.sheet.tabs.labels, vec!["Audio", "Applications"]);
    }

    fn app(key: &str, label: &str, streams: &[u32], volume: f64, mute: bool) -> App {
        App {
            key: key.into(),
            label: label.into(),
            icon_hints: vec![key.into()],
            binary: Some(key.into()),
            streams: streams
                .iter()
                .map(|&id| crate::pipewire::Stream {
                    id,
                    channels: Default::default(),
                })
                .collect(),
            volume,
            mute,
        }
    }

    fn playing() -> Snapshot {
        let mut snap = fixture();
        snap.apps = vec![
            app("firefox", "Firefox", &[90, 91], 0.8, false),
            app("mpv", "mpv", &[92], 0.5, true),
        ];
        snap
    }

    fn select_app(s: &mut Sheet, row: usize) {
        s.sheet.tabs.open = APPS;
        select(s, APP_LIST, row);
    }

    #[test]
    fn the_programs_are_listed_with_their_volume() {
        let mut s = sheet(&playing());
        s.open(APPS);
        let l = list(&s, APP_LIST);
        assert_eq!(l.items, vec!["Firefox", "mpv"]);
        assert_eq!(l.detail, vec!["80%", "Muted"]);
        assert_eq!(l.selected, Some(0));
        assert_eq!(track(&s, APP_TRACK).pos, 16);
        assert_eq!(check(&s, APP_MUTE), (false, true));
        assert!(!s.dirty());
    }

    #[test]
    fn a_program_volume_writes_every_stream_on_apply() {
        let mut s = sheet(&playing());
        select_app(&mut s, 0);
        s.sheet.open_mut().focus_id(APP_TRACK);
        let ev = s.sheet.key(form::Key::Left, None);
        s.event(ev);
        assert_eq!(label_text(&s, APP_PERCENT), "75%");
        assert_eq!(list(&s, APP_LIST).detail[0], "75%");
        assert_eq!(
            s.event(propsheet::Event::Apply),
            Outcome::Apply(vec![Change::Volume(90, 0.75), Change::Volume(91, 0.75)])
        );
        assert!(!s.dirty());
    }

    #[test]
    fn the_controls_follow_the_selected_program() {
        let mut s = sheet(&playing());
        select_app(&mut s, 1);
        assert_eq!(track(&s, APP_TRACK).pos, 10);
        assert_eq!(check(&s, APP_MUTE), (true, true));
        s.sheet.open_mut().focus_id(APP_MUTE);
        let ev = s.sheet.key(form::Key::Space, None);
        s.event(ev);
        assert_eq!(list(&s, APP_LIST).detail[1], "50%", "unmuted");
        assert_eq!(
            s.event(propsheet::Event::Ok),
            Outcome::ApplyAndClose(vec![Change::Mute(92, false)])
        );
    }

    #[test]
    fn a_program_that_stops_playing_takes_its_changes_with_it() {
        let mut s = sheet(&playing());
        select_app(&mut s, 1);
        s.sheet.open_mut().focus_id(APP_MUTE);
        let ev = s.sheet.key(form::Key::Space, None);
        s.event(ev);
        assert!(s.dirty());
        let mut next = playing();
        next.apps.remove(1);
        s.refresh(&next);
        assert!(!s.dirty(), "nothing is left to write to");
        assert_eq!(list(&s, APP_LIST).items, vec!["Firefox"]);
    }

    #[test]
    fn the_selection_stays_on_its_program() {
        let mut s = sheet(&playing());
        select_app(&mut s, 1);
        let mut next = playing();
        next.apps
            .insert(0, app("audacious", "Audacious", &[95], 1.0, false));
        s.refresh(&next);
        assert_eq!(list(&s, APP_LIST).selected, Some(2), "still mpv");
    }

    #[test]
    fn a_failed_apply_leaves_a_program_change_pending() {
        let mut s = sheet(&playing());
        select_app(&mut s, 0);
        s.sheet.open_mut().focus_id(APP_MUTE);
        let ev = s.sheet.key(form::Key::Space, None);
        s.event(ev);
        let want = Outcome::Apply(vec![Change::Mute(90, true), Change::Mute(91, true)]);
        assert_eq!(s.event(propsheet::Event::Apply), want);
        s.unapplied(&playing());
        assert!(s.dirty());
        assert_eq!(s.event(propsheet::Event::Apply), want);
    }

    #[test]
    fn with_nothing_playing_the_page_greys() {
        let mut s = sheet(&fixture());
        s.open(APPS);
        let l = list(&s, APP_LIST);
        assert_eq!(l.items, vec![APP_EMPTY]);
        assert_eq!(l.greyed, vec![true]);
        assert!(!track(&s, APP_TRACK).enabled);
        assert_eq!(check(&s, APP_MUTE), (false, false));
        assert_eq!(label_text(&s, APP_PERCENT), "");
    }

    #[test]
    fn the_program_icons_land_in_the_list_and_the_picture() {
        let mut s = sheet(&playing());
        let icon = |px| Some(Canvas::new(px, px));
        let mut icons = BTreeMap::new();
        icons.insert(
            "mpv".to_string(),
            AppIcon {
                small: icon(16),
                large: icon(32),
            },
        );
        s.set_app_icons(icons);
        select_app(&mut s, 1);
        let page = &s.sheet.pages[APPS];
        match &page.item(APP_LIST).unwrap().control {
            Control::ListBox { icons, .. } => {
                assert!(icons[0].is_none() && icons[1].is_some())
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            &page.item(APP_ICON).unwrap().control,
            Control::Picture { canvas: Some(c), .. } if c.width == 32
        ));
    }

    #[test]
    fn the_applications_page_geometry() {
        let s = sheet(&playing());
        let page = &s.sheet.pages[APPS];
        let rect = |id: &str| page.item(id).expect("item exists").rect;
        assert_eq!(rect(APP_GROUP), Rect::new(21, 47, 320, 344));
        assert_eq!(rect(APP_ICON), rect_of(&s, SINK.icon));
        assert_eq!(rect(APP_LIST), Rect::new(70, 65, 261, 276));
        assert_eq!(rect(APP_TRACK), Rect::new(120, 353, 130, trackbar::HEIGHT));
        assert_eq!(rect(APP_MUTE), Rect::new(290, 361, 13, 13));
    }

    fn rect_of(s: &Sheet, id: &str) -> Rect {
        s.sheet.pages[AUDIO].item(id).expect("item exists").rect
    }

    #[test]
    #[ignore]
    fn render_the_applications_tab() {
        use reprisekit::icontheme::{IconCache, IconTheme};
        let theme = Theme {
            access_keys: true,
            ..Theme::default()
        };
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let mut snap = playing();
        snap.apps
            .push(app("mystery-player", "Mystery Player", &[93], 0.3, false));
        let mut s = sheet(&snap);
        let cache = IconCache::new(IconTheme::load(
            &std::env::var(reprisekit::icontheme::ENV).unwrap_or_else(|_| "hicolor".into()),
            &IconTheme::default_roots(),
        ));
        let icons = crate::appicons::Resolver::default().icons(&snap.apps, &cache);
        s.set_app_icons(icons);
        s.open(APPS);
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        propsheet::draw(&mut c, &text, &theme.paint(), &s.sheet, true, px);
        reprisekit::raster::write_preview(&c, "reprise-sound-apps.png");
    }

    #[test]
    #[ignore]
    fn render_the_sheet() {
        let theme = Theme {
            access_keys: true,
            ..Theme::default()
        };
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let s = sheet(&fixture());
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        propsheet::draw(&mut c, &text, &theme.paint(), &s.sheet, true, px);
        reprisekit::raster::write_preview(&c, "reprise-sound.png");
    }
}
