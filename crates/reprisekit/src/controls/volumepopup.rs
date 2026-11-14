//! The speaker's volume popup: a raised flyout ([`crate::flyout`]) over the tray with Output and Input device combos on top and, under a separator, master's column, an etched line and a column per program playing sound.
//!
//! Columns are 96 px wide: the 32 px icon, the name ("Volume" on master), the level in percent, a vertical trackbar (one position per 5 %, loudest on top, on to 150 % shaded past 100 with [`Mix::boost`]) and a 30x26 speaker button that mutes; a muted column greys its trackbar and level. With two outputs or more a program's column ends in a combo picking its output ("Default", then each output), whose list opens above it. Six programs fit side by side; past six a scrollbar under them scrolls them a column at a time while master stays. The device combos are 360 px wide. Lengths are at 100% text and grow with it; icons, glyphs, the thumb and the scrollbar keep their size.
//!
//! While a media player is active a strip under a second separator shows the title over the artist and Previous, Play or Pause and Next buttons (32x28, greyed when the player cannot act). Text is set in the second dialog font (`DialogFont::ShellDlg`).
//!
//! The host owns the devices and the writes: it hands the popup a [`Mix`] with [`VolumePopup::sync`] and acts on the [`Event`]s at once; there is no Apply.

use crate::classic::widgets::ScrollHit;
use crate::controls::combo::Combo;
use crate::look::widgets::MediaGlyph;
use crate::pointer::Pointer;
use crate::raster::Canvas;
use crate::trackbar::{self, TrackBar};
use std::cell::Cell;

mod access_tree;
mod draw;
mod input;
mod layout;
pub use draw::draw;
pub use layout::{ColumnLayout, Hit, Layout, StripLayout, hit, layout};

/// Trackbar positions: 0 to 20, one per 5 % of volume.
pub const STEPS: usize = 21;
/// With [`Mix::boost`], positions run on to 30, 150 %, shaded past 20.
pub const BOOST_STEPS: usize = 31;
pub const COL_W: i32 = 96;
/// Between two program columns.
const COL_GAP: i32 = 6;
/// Program columns shown side by side; past this a scrollbar runs under
/// them.
pub const MAX_SHOWN: usize = 6;
/// From the panel's outer edge in to its parts, all round.
const MARGIN: i32 = 12;
const COMBO_H: i32 = 21;
const COMBO_GAP: i32 = 6;
const LABEL_GAP: i32 = 8;
/// The Output and Input labels' column, at its narrowest.
const LABEL_W: i32 = 52;
/// The Output and Input combos' width.
const FIELD_W: i32 = 360;
const SEP_GAP: i32 = 10;
const SEP_H: i32 = 2;
/// Either side of the etched line between master and the programs.
const DIVIDER_GAP: i32 = 6;
/// A column, from its top: the icon, the name and level rows under it,
/// the trackbar, then the speaker button under the trackbar.
const ICON: i32 = 32;
const NAME_Y: i32 = ICON + 6;
const ROW_H: i32 = 13;
const LEVEL_Y: i32 = NAME_Y + ROW_H + 1;
const TRACK_Y: i32 = LEVEL_Y + ROW_H + 8;
const TRACK_H: i32 = 140;
/// The thumb, its width past the look's own.
const THUMB_WIDER: i32 = 4;
const THUMB_H: i32 = 15;
const MUTE_Y: i32 = TRACK_Y + TRACK_H + 8;
const MUTE_W: i32 = 30;
const MUTE_H: i32 = 26;
/// The speaker glyph's box.
const SPEAKER: i32 = 16;
const COL_H: i32 = MUTE_Y + MUTE_H;
/// A program's name and level leave 2 px clear on either side of the
/// centred text.
const NAME_PAD: i32 = 2;
const BAR_GAP: i32 = 8;
/// Rows an open device list shows before it scrolls.
const LIST_ROWS: usize = 8;
/// A program's output combo: under the speaker button, inset from the column's sides.
const DEVICE_GAP: i32 = 8;
const DEVICE_INSET: i32 = 4;
/// The now-playing strip: its height, and its transport buttons.
const STRIP_H: i32 = 34;
const MEDIA_W: i32 = 32;
const MEDIA_H: i32 = 28;
const MEDIA_GAP: i32 = 4;

pub const VOLUME: &str = "Volume";
pub const MUTE: &str = "Mute";
pub const OUTPUT: &str = "Output:";
pub const INPUT: &str = "Input:";
pub const NOW_PLAYING: &str = "Now Playing";
/// A program's output combo's first row: the program follows the default output.
pub const DEFAULT: &str = "Default";

/// The track a media player has on, and what it can do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NowPlaying {
    pub title: String,
    /// Empty when the player names none: the title stands alone.
    pub artist: String,
    /// Playing rather than paused: the middle button pauses.
    pub playing: bool,
    pub can_previous: bool,
    pub can_next: bool,
    pub can_play: bool,
    pub can_pause: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Previous,
    PlayPause,
    Next,
}

impl Transport {
    pub const ALL: [Transport; 3] = [Transport::Previous, Transport::PlayPause, Transport::Next];
}

impl NowPlaying {
    /// Whether button `t` acts: Play / Pause by what the middle button
    /// would do now.
    pub fn can(&self, t: Transport) -> bool {
        match t {
            Transport::Previous => self.can_previous,
            Transport::Next => self.can_next,
            Transport::PlayPause if self.playing => self.can_pause,
            Transport::PlayPause => self.can_play,
        }
    }

    pub fn glyph(&self, t: Transport) -> (MediaGlyph, &'static str) {
        match t {
            Transport::Previous => (MediaGlyph::Previous, "Previous"),
            Transport::Next => (MediaGlyph::Next, "Next"),
            Transport::PlayPause if self.playing => (MediaGlyph::Pause, "Pause"),
            Transport::PlayPause => (MediaGlyph::Play, "Play"),
        }
    }
}

/// A device's volume and mute as trackbar positions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Level {
    pub pos: usize,
    pub mute: bool,
    /// False with no device: the column greys.
    pub live: bool,
    /// Master's 32 px icon.
    pub icon: Option<Canvas>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppLevel {
    /// What ties the program's streams together (`pipewire::App::key`).
    pub key: String,
    pub label: String,
    /// The program's 32 px icon.
    pub icon: Option<Canvas>,
    pub pos: usize,
    pub mute: bool,
    /// The output the program plays to, an index into [`Mix::outputs`]; `None` follows the default.
    pub output: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mix {
    /// The default sink; `live: false` greys the master column.
    pub master: Level,
    pub outputs: Vec<String>,
    /// The default output's index into `outputs`.
    pub output: Option<usize>,
    pub inputs: Vec<String>,
    pub input: Option<usize>,
    pub apps: Vec<AppLevel>,
    /// The media player's track; `None` with no player, and no strip.
    pub playing: Option<NowPlaying>,
    /// Volumes may go past 100 %: every trackbar runs to [`BOOST_STEPS`].
    pub boost: bool,
}

/// Whose volume an [`Event`] sets: the default sink, or a program by its
/// key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Channel {
    Master,
    App(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Output,
    Input,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    Close,
    Volume(Channel, usize),
    Mute(Channel, bool),
    /// A new default device: an index into `Mix::outputs` / `Mix::inputs`.
    Default(Dir, usize),
    /// Program `.0` (its key) moves to output `.1`, an index into `Mix::outputs`; `None` follows the default.
    AppOutput(String, Option<usize>),
    Media(Transport),
}

/// One column: master (`key: None`) or a program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub key: Option<String>,
    pub label: String,
    pub icon: Option<Canvas>,
    pub volume: TrackBar,
    pub mute: bool,
    /// A program's output combo, with two outputs or more to pick from.
    pub output: Option<Combo>,
}

/// A combo on the popup: a default device's, or the output of the program in column `.0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Device(Dir),
    App(usize),
}

/// A column's volume trackbar at `pos`.
fn volume_bar(pos: usize, boost: bool) -> TrackBar {
    let count = if boost { BOOST_STEPS } else { STEPS };
    TrackBar::new(pos, count)
        .paged(4, 2)
        .untick()
        .boosted(boost.then_some(STEPS - 1))
}

impl Column {
    fn master(level: &Level, boost: bool) -> Column {
        let mut volume = volume_bar(level.pos, boost);
        volume.enabled = level.live;
        Column {
            key: None,
            label: VOLUME.into(),
            icon: level.icon.clone(),
            volume,
            mute: level.mute,
            output: None,
        }
    }

    fn app(a: &AppLevel, mix: &Mix) -> Column {
        Column {
            key: Some(a.key.clone()),
            label: a.label.clone(),
            icon: a.icon.clone(),
            volume: volume_bar(a.pos, mix.boost),
            mute: a.mute,
            output: (mix.outputs.len() > 1)
                .then(|| Combo::names(&app_outputs(mix), Some(app_output(a)), LIST_ROWS)),
        }
    }
}

/// A program's output combo's rows: [`DEFAULT`], then each output.
fn app_outputs(mix: &Mix) -> Vec<String> {
    std::iter::once(DEFAULT.to_string())
        .chain(mix.outputs.iter().cloned())
        .collect()
}

/// A program's row in its output combo.
fn app_output(a: &AppLevel) -> usize {
    a.output.map_or(0, |i| i + 1)
}

fn columns(mix: &Mix) -> Vec<Column> {
    let mut columns = vec![Column::master(&mix.master, mix.boost)];
    columns.extend(mix.apps.iter().map(|a| Column::app(a, mix)));
    columns
}

/// What the keyboard is on. Column parts carry the column's index into
/// [`VolumePopup::columns`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Output,
    Input,
    Thumb(usize),
    Mute(usize),
    /// A program's output combo.
    Device(usize),
    Media(Transport),
}

/// The parts the pointer can light or hold. Column parts carry the
/// column's index into [`VolumePopup::columns`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Thumb(usize),
    Mute(usize),
    Combo(Pick),
    Bar(ScrollHit),
    Media(Transport),
}

pub struct VolumePopup {
    pub columns: Vec<Column>,
    pub output: Combo,
    pub input: Combo,
    /// The first program shown, an index into `columns[1..]`.
    pub first: usize,
    pub focus: Focus,
    /// Whether a key has been pressed since the popup opened: only then is
    /// the focus ring drawn.
    pub cues: bool,
    pub ptr: Pointer<Part>,
    pub playing: Option<NowPlaying>,
    /// Where the pointer last pressed or moved on the popup; `None` once
    /// it left. A key or the device moving a thumb finds the part under
    /// it again.
    point: Option<(i32, i32)>,
    grab: i32,
    /// Whether the last draw's look lights a part under the pointer.
    styled: Cell<bool>,
}

fn combo_focus(p: Pick) -> Focus {
    match p {
        Pick::Device(Dir::Output) => Focus::Output,
        Pick::Device(Dir::Input) => Focus::Input,
        Pick::App(i) => Focus::Device(i),
    }
}

impl VolumePopup {
    pub fn new(mix: &Mix) -> VolumePopup {
        VolumePopup {
            columns: columns(mix),
            output: Combo::names(&mix.outputs, mix.output, LIST_ROWS),
            input: Combo::names(&mix.inputs, mix.input, LIST_ROWS),
            first: 0,
            focus: Focus::Thumb(0),
            cues: false,
            ptr: Pointer::new(),
            playing: mix.playing.clone(),
            point: None,
            grab: 0,
            styled: Cell::new(false),
        }
    }

    pub fn channel(&self, col: usize) -> Channel {
        match &self.columns[col].key {
            None => Channel::Master,
            Some(k) => Channel::App(k.clone()),
        }
    }

    /// Take the devices and programs from the host. Programs are matched by key: one that stays keeps its focus, light and drag; one that left sends its focus to master and ends its drag. A held thumb keeps its own position since the device lags the drag's writes. Changed combos close their lists; `Redraw` when anything shown moved.
    pub fn sync(&mut self, lay: &Layout, mix: &Mix) -> Event {
        let mut columns = columns(mix);
        let moved: Vec<Option<usize>> = self
            .columns
            .iter()
            .map(|old| match &old.key {
                None => Some(0),
                Some(k) => columns.iter().position(|c| c.key.as_ref() == Some(k)),
            })
            .collect();
        let remap = |i: usize| moved.get(i).copied().flatten();
        if let Some(Part::Thumb(i)) = self.ptr.held()
            && let Some(j) = remap(i)
        {
            columns[j].volume.set(self.columns[i].volume.pos);
        }
        // A program's combo keeps its open list while its rows stay.
        let rows = app_outputs(mix);
        let mut rebuilt = vec![false; columns.len()];
        for (i, old) in self.columns.iter_mut().enumerate() {
            if let (Some(j), Some(mut c)) = (remap(i), old.output.take())
                && let Some(a) = j.checked_sub(1).and_then(|a| mix.apps.get(a))
                && columns[j].output.is_some()
            {
                rebuilt[j] = c.sync_names(&rows, Some(app_output(a)), LIST_ROWS);
                columns[j].output = Some(c);
            }
        }
        let has_combo = |j: usize| columns.get(j).is_some_and(|c| c.output.is_some());
        let can = |t: Transport| mix.playing.as_ref().is_some_and(|np| np.can(t));
        self.focus = match self.focus {
            Focus::Thumb(i) => remap(i).map_or(Focus::Thumb(0), Focus::Thumb),
            Focus::Mute(i) => remap(i).map_or(Focus::Thumb(0), Focus::Mute),
            Focus::Device(i) => match remap(i) {
                Some(j) if has_combo(j) => Focus::Device(j),
                Some(j) => Focus::Thumb(j),
                None => Focus::Thumb(0),
            },
            Focus::Media(t) if !can(t) => Focus::Thumb(0),
            f => f,
        };
        let output = self.output.sync_names(&mix.outputs, mix.output, LIST_ROWS);
        let input = self.input.sync_names(&mix.inputs, mix.input, LIST_ROWS);
        let bar = columns.len() - 1 > MAX_SHOWN;
        self.ptr = std::mem::take(&mut self.ptr).carry(|p| match p {
            Part::Thumb(i) => remap(i).map(Part::Thumb),
            Part::Mute(i) => remap(i).map(Part::Mute),
            Part::Combo(Pick::Device(Dir::Output)) if output => None,
            Part::Combo(Pick::Device(Dir::Input)) if input => None,
            Part::Combo(Pick::App(i)) => remap(i)
                .filter(|&j| has_combo(j) && !rebuilt[j])
                .map(|j| Part::Combo(Pick::App(j))),
            Part::Bar(_) if !bar => None,
            Part::Media(t) if !can(t) => None,
            p => Some(p),
        });
        self.columns = columns;
        self.playing = mix.playing.clone();
        self.first = self
            .first
            .min(self.columns.len().saturating_sub(1 + MAX_SHOWN));
        self.rehit(lay)
    }

    pub fn dragging(&self) -> bool {
        matches!(self.ptr.held(), Some(Part::Thumb(_)))
    }

    /// `lay` with every shown thumb where its volume puts it now.
    fn current(&self, lay: &Layout) -> Layout {
        let columns = lay
            .columns
            .iter()
            .zip(&self.columns)
            .map(|(cl, c)| {
                cl.as_ref().map(|cl| ColumnLayout {
                    track: trackbar::at_position(&cl.track, c.volume.pos),
                    ..cl.clone()
                })
            })
            .collect();
        Layout {
            columns,
            ..lay.clone()
        }
    }
}

#[cfg(test)]
mod tests;
