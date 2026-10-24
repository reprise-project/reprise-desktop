//! The pages of the Reprise Settings sheet: pure geometry and the binding
//! between a control and its [`Settings`] field. Nothing here reads a file or
//! talks to a compositor. Which pages exist, and their order, is `registry`'s.
//!
//! Each page is built for an [`Area`]: at its smallest, a 398x528 client with
//! its page at (6, 28, 386, 464), so the content column runs x 20..378 and
//! starts 16 px below the page's top edge; a larger window widens the column
//! and lengthens the page. The sheet moves every page right of its list.

use reprisefs::props::short_size;
use reprisefs::thumbnails::Cleared;
use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::propsheet::PropSheet;
use reprisekit::controls::timefield::TimeField;
use reprisekit::controls::updown::{self, UpDown};
use reprisekit::controls::{combo, pagelist};
use reprisekit::raster::Rect;
use reprisekit::trackbar::{self, TrackBar};

use crate::model::{Choices, REPRISE_TOOLKIT_ROW, TOOLKIT_AUTO, TOOLKIT_REPRISE};

mod appearance;
mod bind;
mod desktop;
mod input;
mod lists;
mod system;

pub use appearance::{effects, fonts_and_icons, night_light};
pub use bind::{fill_layouts, follow, layout_selection, read_one, write, write_form, write_forms};
pub use desktop::{desktops, launcher, switcher, taskbar, windows};
pub use input::{accessibility, keyboard, mouse, pointers, preview_size};
pub use lists::{
    draw_list, draw_shortcuts, fill_list, fill_locations, fill_shortcuts, fit_list_columns,
    fit_location_columns, fit_shortcut_columns, list_key, new_list, new_location_list,
    new_shortcut_list, show_locations_loading,
};
pub use system::{advanced, power, startup};

/// The client every page is built for at [`Area::MIN`]; the sheet puts each page right of its list ([`PropSheet::on_page`]).
pub const CLIENT: (i32, i32) = (398, 528);

/// The Pointers page's preview and the combo row standing for an empty `[cursor] theme`.
pub const CURSOR_PREVIEW: &str = "cursor_preview";
pub const CURSOR_DEFAULT_ROW: &str = "(Default)";

const CURSOR_FIXED_ROWS: &Choices = &[("", CURSOR_DEFAULT_ROW)];

pub const GTK_AUTO_ROW: &str = "(Automatic)";

/// The GTK theme combo's first rows: "auto", then "reprise", the theme generated from the look in force.
const GTK_FIXED_ROWS: &Choices = &[
    (TOOLKIT_AUTO, GTK_AUTO_ROW),
    (TOOLKIT_REPRISE, REPRISE_TOOLKIT_ROW),
];

/// The night light trackbar's range and step, in kelvin: warmest on the left.
const KELVIN_MIN: i64 = 2500;
const KELVIN_MAX: i64 = 6500;
const KELVIN_STEP: i64 = 100;
const KELVIN_STEPS: usize = ((KELVIN_MAX - KELVIN_MIN) / KELVIN_STEP) as usize + 1;

/// The SlowKeys delay trackbar: 0.3 to 2.0 s in tenths, shortest on the left.
const DELAY_MIN: i64 = 3;
const DELAY_MAX: i64 = 20;
const DELAY_STEPS: usize = (DELAY_MAX - DELAY_MIN) as usize + 1;

/// The MouseKeys top speed trackbar: 100 to 2000 pixels a second in steps of 100, slowest on the left.
const MK_SPEED: Steps = Steps {
    first: 100,
    step: 100,
    count: 20,
};
/// The MouseKeys acceleration trackbar: 5 s down to none in half seconds (tenths in `Settings`), quickest on the right.
const MK_ACCEL: Steps = Steps {
    first: 50,
    step: -5,
    count: 11,
};

/// A trackbar whose position `p` stands for `first + p * step`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Steps {
    first: i64,
    step: i64,
    count: usize,
}

impl Steps {
    fn pos(self, v: i64) -> usize {
        let p = (v - self.first + self.step / 2) / self.step;
        p.clamp(0, self.count as i64 - 1) as usize
    }

    fn value(self, pos: usize) -> i64 {
        self.first + pos as i64 * self.step
    }
}

const SPEED_STEPS: usize = 21;

pub(crate) const X: i32 = 20;
pub(crate) const GX: i32 = X + 12;
const EDIT_H: i32 = 20;
/// The widest a combo grows to with the column; one built wider keeps its width.
pub(crate) const COMBO_MAX: i32 = 360;
pub(crate) const TRACK_MAX: i32 = 300;

/// What a page fills: its content column's width and how much taller than
/// [`CLIENT`]'s the page is. At [`Area::MIN`] each page sits as in a
/// [`CLIENT`] sized sheet; a larger area gives extra width to what holds
/// content (fields, lists, combos up to [`COMBO_MAX`], trackbars up to
/// [`TRACK_MAX`], group boxes), moves what keeps to the right edge, and gives
/// extra height to a page's list or preview. Check boxes, radios, buttons and
/// up-downs keep their size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub w: i32,
    pub dh: i32,
}

impl Area {
    /// The smallest area, a [`CLIENT`] sized sheet's: the column runs x 20..378.
    pub const MIN: Area = Area { w: 358, dh: 0 };

    /// The area of a list sheet of client `size` (the list included), no smaller than [`Area::MIN`].
    pub fn of_size(size: (i32, i32)) -> Area {
        let w = size.0 - pagelist::ADVANCE - (CLIENT.0 - Area::MIN.w);
        Area {
            w: w.max(Area::MIN.w),
            dh: (size.1 - CLIENT.1).max(0),
        }
    }

    pub fn of(sheet: &PropSheet) -> Area {
        Area::of_size(sheet.size)
    }

    pub fn sheet_size(self) -> (i32, i32) {
        (
            self.w + CLIENT.0 - Area::MIN.w + pagelist::ADVANCE,
            CLIENT.1 + self.dh,
        )
    }

    pub fn dw(self) -> i32 {
        self.w - Area::MIN.w
    }

    pub fn gw(self) -> i32 {
        self.w - 24
    }

    pub fn right(self) -> i32 {
        X + self.w
    }

    pub fn gright(self) -> i32 {
        GX + self.gw()
    }

    /// A combo built `w` wide at [`Area::MIN`], grown with the column up to [`COMBO_MAX`].
    pub fn combo(self, w: i32) -> i32 {
        (w + self.dw()).min(COMBO_MAX.max(w))
    }

    /// A trackbar built `w` wide at [`Area::MIN`], grown with the column up to [`TRACK_MAX`].
    pub fn track(self, w: i32) -> i32 {
        (w + self.dw()).min(TRACK_MAX.max(w))
    }
}

pub const EDIT_FILE: &str = "edit_file";
/// The Taskbar page's Customize... beside the tray overflow box; the compositor opens the tray's Customize dialog.
pub const TRAY_CUSTOMIZE: &str = "taskbar_tray_customize";
pub const ADD: &str = "add";
pub const REMOVE: &str = "remove";

/// The Startup page's autostart list: 11 rows under its header at
/// [`Area::MIN`], more with extra page height; the rest scroll, so Command
/// takes what the other columns and the scrollbar leave ([`fit_list_columns`]).
pub fn list(a: Area) -> Rect {
    Rect::new(X, 44, a.w, 208 + a.dh)
}
pub const COLUMNS: [(&str, i32); 3] = [("Startup Item", 164), ("Command", 114), ("Location", 60)];

/// The Advanced page's list of remembered location answers, under its check,
/// with Forget beside it. Three rows show at [`Area::MIN`]; the rest scroll,
/// so Program takes what Location and the scrollbar leave ([`fit_location_columns`]).
pub fn locations(a: Area) -> Rect {
    Rect::new(X, 220, a.w - BUTTON_W - 8, 68 + a.dh)
}
pub const LOCATION_COLUMNS: [(&str, i32); 2] = [("Program", 185), ("Location", 70)];
pub const PRIVACY_LOCATION: &str = "privacy_location";
/// The Advanced page's debug overlay check and the one indented under it, greyed while the overlay is off.
pub const DEBUG_OVERLAY: &str = "debug_overlay";
pub const DEBUG_IN_CAPTURES: &str = "debug_overlay_in_captures";
pub const FORGET: &str = "location_forget";
pub const WATER: &str = "wallpaper_water";
pub const WATER_STYLE: &str = "water_style";
/// The Advanced page's thumbnail cache check, its size row (greyed while the cache is off),
/// Clear, which removes the thumbnails Reprise kept, and the line saying what Clear removed.
pub const THUMBS: &str = "thumbnail_cache";
pub const THUMBS_MB: &str = "thumbnail_max_mb";
const THUMBS_SIZE: [&str; 2] = ["thumbnail_size_label", THUMBS_MB];
pub const THUMBS_CLEAR: &str = "thumbnail_clear";
pub const THUMBS_CLEARED: &str = "thumbnail_cleared";

/// Where the Advanced page's Clear is: running, or ended with what it removed or without a
/// result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clearing {
    Running,
    Done(Cleared),
    Failed,
}

/// Clear is greyed while a clear runs, and the line beside it says so; once it ends, the line
/// says what it removed.
pub fn show_clearing(page: &mut Form, state: Clearing) {
    page.set_enabled(THUMBS_CLEAR, state != Clearing::Running);
    page.set_text(THUMBS_CLEARED, &cleared_text(state));
}

fn cleared_text(state: Clearing) -> String {
    match state {
        Clearing::Running => "Clearing...".into(),
        Clearing::Failed => "Couldn't clear the thumbnails.".into(),
        Clearing::Done(Cleared { files: 0, .. }) => "Nothing to remove.".into(),
        Clearing::Done(Cleared { files: 1, bytes }) => {
            format!("Removed 1 file, {}.", short_size(bytes))
        }
        Clearing::Done(Cleared { files, bytes }) => {
            format!("Removed {files} files, {}.", short_size(bytes))
        }
    }
}

/// The Keyboard page's Shortcuts list, inside its group box under the layout
/// fields. Four rows show at [`Area::MIN`], more with extra page height; the
/// rest scroll, so Action takes what the scrollbar leaves
/// ([`fit_shortcut_columns`]; 194 px beside a 16 px bar).
pub fn shortcuts(a: Area) -> Rect {
    Rect::new(GX, 260, a.gw(), 92 + a.dh)
}
pub const SHORTCUT_COLUMNS: [(&str, i32); 2] = [("Keys", 120), ("Action", 194)];
pub const SC_ADD: &str = "sc_add";
pub const SC_EDIT: &str = "sc_edit";
pub const SC_REMOVE: &str = "sc_remove";

pub const KB_LAYOUTS: &str = "kb_layouts";
pub const KB_ADD: &str = "kb_add";
pub const KB_REMOVE: &str = "kb_remove";
pub const KB_UP: &str = "kb_up";

fn edit(id: &'static str, x: i32, y: i32, w: i32) -> Item {
    Item::edit(id, Rect::new(x, y, w, EDIT_H), "", true)
}

/// A label above its edit field: the label's baseline row, then the field 16 px under it.
fn field(
    label_id: &'static str,
    id: &'static str,
    x: i32,
    y: i32,
    w: i32,
    text: &str,
) -> [Item; 2] {
    [
        Item::label_at(label_id, x, y, w, text),
        edit(id, x, y + 16, w),
    ]
}

pub(crate) fn updown(id: &'static str, x: i32, y: i32, min: i32, max: i32) -> Item {
    Item::new(
        id,
        Rect::new(x, y, updown::WIDTH, updown::HEIGHT),
        Control::UpDown(UpDown::new(min, min, max)),
    )
}

/// A label and the up-down it names, the field 4 px above the label's row so they line up on their text.
pub(crate) fn spin(
    label_id: &'static str,
    id: &'static str,
    x: i32,
    y: i32,
    w: i32,
    text: &str,
    range: (i32, i32),
) -> [Item; 2] {
    [
        Item::label_at(label_id, x, y, w, text),
        updown(id, x + w + 4, y - 4, range.0, range.1),
    ]
}

fn combo(id: &'static str, x: i32, y: i32, w: i32, items: &[String], rows: usize) -> Item {
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    Item::combo(id, x, y, w, &refs, rows)
}

pub(crate) fn choice(id: &'static str, x: i32, y: i32, w: i32, choices: &Choices) -> Item {
    let labels: Vec<&str> = choices.iter().map(|c| c.1).collect();
    Item::combo(id, x, y, w, &labels, choices.len())
}

pub(crate) fn group(a: Area, id: &'static str, y: i32, h: i32, title: &str) -> Item {
    Item::group(id, Rect::new(X, y, a.w, h), title)
}

pub(crate) fn button(id: &'static str, x: i32, y: i32, text: &str) -> Item {
    Item::button(id, Rect::new(x, y, BUTTON_W, BUTTON_H), text, true, false)
}

pub(crate) fn radio(id: &'static str, group: &'static str, x: i32, y: i32, text: &str) -> Item {
    Item::radio(id, x, y, text, group)
}

/// A label and the hours-and-minutes field it names, the field 4 px above the label's row.
fn time(label_id: &'static str, id: &'static str, x: i32, y: i32, w: i32, text: &str) -> [Item; 2] {
    [
        Item::label_at(label_id, x, y, w, text),
        Item::new(
            id,
            Rect::new(x + w + 4, y - 4, 70, EDIT_H),
            Control::Time(TimeField::hm(0, 0, false)),
        ),
    ]
}

/// A trackbar between two end labels across a group's inside, its value
/// right-aligned above the right end: the labels are `ends` wide, the trackbar
/// grows with the column up to [`TRACK_MAX`], and the right label and value
/// follow its end. `y` is the end labels' row; the value sits `value_dy`
/// above it. Returns the value, left label, trackbar and right label.
#[allow(clippy::too_many_arguments)]
fn ends_track(
    a: Area,
    ids: [&'static str; 4],
    y: i32,
    value_dy: i32,
    ends: (&str, &str),
    track_x: i32,
    bar: TrackBar,
) -> [Item; 4] {
    let [left_id, track_id, right_id, value_id] = ids;
    // At `Area::MIN` the bar stops 50 px short of the inside's right, where the 44 px right label starts 6 px later.
    let w = a.track(Area::MIN.gright() - 50 - track_x);
    let track = Rect::new(track_x, y - 6, w, trackbar::HEIGHT);
    let right = track.right() + 6;
    [
        Item::value_at(value_id, right + 44 - 80, y - value_dy, 80, ""),
        Item::label_at(left_id, GX, y, 44, ends.0),
        Item::new(track_id, track, Control::TrackBar(bar)),
        Item::label_at(right_id, right, y, 44, ends.1),
    ]
}

#[cfg(test)]
mod tests;
