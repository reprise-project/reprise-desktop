//! The pages of the Reprise Settings sheet: pure geometry and the binding
//! between a control and its [`Settings`] field. Nothing here reads a file or
//! talks to a compositor. Which pages exist, and their order, is `registry`'s.
//!
//! Each page is built for an [`Area`]: at its smallest, a 398x528 client with
//! its page at (6, 28, 386, 464), so the content column runs x 20..378 and
//! starts 16 px below the page's top edge; a larger window widens the column
//! and lengthens the page. The sheet moves every page right of its list.

use reprisefs::autostart::Entry;
use reprisefs::props::short_size;
use reprisefs::thumbnails::Cleared;
use reprisefs::xkb::{Layout, MAX_LAYOUTS};
use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, LINE_H};
use reprisekit::controls::combo;
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::listbox::ListBox;
use reprisekit::controls::listview::{self, Column, ListView, Row};
use reprisekit::controls::pagelist;
use reprisekit::controls::propsheet::PropSheet;
use reprisekit::controls::timefield::TimeField;
use reprisekit::controls::updown::{self, UpDown};
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::textedit::TextEdit;
use reprisekit::theme::{Color, FontConfig, Theme};
use reprisekit::trackbar::{self, TrackBar};

use crate::model::{
    CURSOR_SIZES, Choices, DESKTOPS_DEFAULT, DESKTOPS_MAX, EFFECTS_MS_MAX, FONT_SMOOTHINGS,
    HOT_CORNERS, INTERFACES, PAGER_MARKS, POSITIONS, PREVIEW_FPS_MAX, QT_PLATFORM_THEMES,
    REPRISE_TOOLKIT_ROW, SNAP_DISTANCE_MAX, STARTMENU_STYLES, SWITCH_ANIMATIONS, SWITCH_MS_MAX,
    SWITCHER_SIZES, SWITCHER_STYLES, Settings, TASKBAR_BUTTONS, TASKBAR_POSITIONS, TEXT_SIZES,
    TOOLKIT_AUTO, TOOLKIT_REPRISE, WATER_STYLES,
};
use crate::registry::Page;
use repriseconf::{Lens, ThumbnailCache, lens};

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

/// The task manager, the switches with no page of their own, the location
/// answers programs were given (a [`ListView`] the host draws over the page at
/// [`locations`]), the thumbnail cache and the config file itself. The terminal is picked on
/// Default Programs. The list takes the page's extra height.
pub fn advanced(a: Area) -> Form {
    let dh = a.dh;
    let mut items = Vec::new();
    items.extend(field(
        "task_manager_label",
        "task_manager",
        X,
        44,
        a.w,
        "Task &manager (empty runs top in the terminal):",
    ));
    let checks = [
        (
            "adaptive_sync",
            "&Variable refresh rate (adaptive sync) where supported",
        ),
        (
            "clipboard_history",
            "Keep a &clipboard history (Super+V opens it)",
        ),
        (
            "screenshot_save",
            "Save each screenshot as a file in &Pictures/Screenshots",
        ),
        (
            "recent_files",
            "Show recently used &files in menus and file dialogs",
        ),
        (
            "capture_border",
            "Show a red &border around what is being shared",
        ),
    ];
    for (i, (id, text)) in checks.into_iter().enumerate() {
        items.push(Item::check(id, X, 88 + 22 * i as i32, text));
    }
    items.push(Item::check(
        PRIVACY_LOCATION,
        X,
        198,
        "Let programs ask for your &location",
    ));
    items.push(button(
        FORGET,
        a.right() - BUTTON_W,
        locations(a).y,
        "F&orget",
    ));
    items.push(Item::check(
        DEBUG_OVERLAY,
        X,
        300 + dh,
        "Show a &debug overlay (frame rate, render times)",
    ));
    items.push(
        Item::check(
            DEBUG_IN_CAPTURES,
            X,
            322 + dh,
            "Show it in screenshots and screen &sharing",
        )
        .under_check(),
    );
    items.push(Item::check(WATER, X, 344 + dh, "Desktop &water"));
    items.push(choice(WATER_STYLE, X + 112, 340 + dh, 100, WATER_STYLES));
    items.push(Item::label_at(
        "water_note",
        X + 12,
        364 + dh,
        a.w - 12,
        "With Ink, Glow, Sparks or Neon, dragging on the desktop",
    ));
    items.push(Item::label_at(
        "water_note_2",
        X + 12,
        377 + dh,
        a.w - 12,
        "stirs the fluid instead of drawing a selection box.",
    ));
    items.push(Item::check(
        THUMBS,
        X,
        392 + dh,
        "Keep &thumbnails of pictures",
    ));
    let mb = ThumbnailCache::MAX_MB;
    items.extend(
        spin(
            THUMBS_SIZE[0],
            THUMBS_MB,
            X,
            418 + dh,
            70,
            "Si&ze in MB:",
            (*mb.start() as i32, *mb.end() as i32),
        )
        .map(Item::under_check),
    );
    let cleared_x = X + 184;
    items.push(Item::value_at(
        THUMBS_CLEARED,
        cleared_x,
        392 + dh,
        a.right() - cleared_x,
        "",
    ));
    items.push(button(
        THUMBS_CLEAR,
        a.right() - BUTTON_W,
        412 + dh,
        "Clea&r",
    ));
    items.push(Item::new(
        "note",
        Rect::new(X, 444 + dh, a.w - BUTTON_W - 8, 2 * LINE_H),
        Control::Lines(vec![
            "Keys no page sets are edited in".into(),
            "config.toml itself.".into(),
        ]),
    ));
    items.push(button(
        EDIT_FILE,
        a.right() - BUTTON_W,
        444 + dh,
        "&Edit File...",
    ));
    Form::new(items)
}

/// `qt_tools` is whether qt6ct or qt5ct is installed, which the Qt combo's Reprise row needs; without either it is greyed.
pub fn fonts_and_icons(
    a: Area,
    icon_themes: &[String],
    gtk_themes: &[String],
    qt_tools: bool,
) -> Form {
    let gw = a.gw();
    let mut items = vec![group(a, "fonts_group", 44, 84, "Fonts")];
    let half = (gw - 8) / 2;
    items.extend(field(
        "family_label",
        "font_family",
        GX,
        62,
        half,
        "&Family (comma separated):",
    ));
    items.extend(field(
        "shell_dlg_label",
        "font_shell_dlg",
        a.gright() - half,
        62,
        half,
        "Second dialo&g font:",
    ));
    items.extend(spin(
        "size_label",
        "font_size_px",
        GX,
        108,
        44,
        "Si&ze:",
        (
            FontConfig::SIZE_PX_MIN as i32,
            FontConfig::SIZE_PX_MAX as i32,
        ),
    ));
    items.push(Item::check("font_antialias", GX + 160, 108, "Antia&lias"));

    items.push(group(a, "icons_group", 138, 56, "Icons"));
    items.push(Item::label_at("theme_label", GX, 162, 56, "&Theme:"));
    items.push(combo(
        "icon_theme",
        GX + 60,
        158,
        a.combo(Area::MIN.gw() - 60),
        icon_themes,
        icon_themes.len().clamp(1, 8),
    ));

    items.push(group(a, "windows_group", 204, 92, "Windows"));
    items.extend(spin(
        "frame_label",
        "frame_width",
        GX,
        226,
        110,
        "Frame &width:",
        (0, 32),
    ));
    items.extend(spin(
        "caption_label",
        "caption_height",
        GX,
        252,
        110,
        "Caption &height:",
        (12, 128),
    ));
    items.extend(spin(
        "taskbar_label",
        "taskbar_height",
        GX,
        278,
        110,
        "Tas&kbar height:",
        (8, 256),
    ));

    // GTK and Qt look: `(Automatic)` and Automatic follow the look's darkness,
    // the Reprise rows its colours. Their font smoothing is apart from the Fonts
    // group's Antialias, which is Reprise's own text.
    let mut gtk: Vec<String> = GTK_FIXED_ROWS
        .iter()
        .map(|(_, row)| row.to_string())
        .collect();
    gtk.extend(gtk_themes.iter().cloned());
    let cw = a.combo(Area::MIN.gw() - 88);
    items.push(group(a, "toolkits_group", 306, 114, "Programs"));
    items.push(Item::label_at("gtk_label", GX, 330, 84, "GTK th&eme:"));
    items.push(combo(
        "gtk_theme",
        GX + 88,
        326,
        cw,
        &gtk,
        gtk.len().clamp(1, 8),
    ));
    items.push(Item::label_at("qt_label", GX, 356, 84, "&Qt style:"));
    let mut qt = choice("qt_platform_theme", GX + 88, 352, cw, QT_PLATFORM_THEMES);
    if let Control::Combo(c) = &mut qt.control
        && let Some(row) = c.items.iter_mut().find(|i| i.text == REPRISE_TOOLKIT_ROW)
    {
        row.greyed = !qt_tools;
    }
    items.push(qt);
    items.push(Item::label_at(
        "smoothing_label",
        GX,
        382,
        84,
        "Fo&nt smoothing:",
    ));
    items.push(choice("font_smoothing", GX + 88, 378, cw, FONT_SMOOTHINGS));
    items.push(Item::check(
        "gtk4_colors",
        GX,
        404,
        "GTK&4 and libadwaita in the look's colours",
    ));
    items.push(Item::new(
        "toolkits_note",
        Rect::new(X, 422, a.w, LINE_H),
        Control::Lines(vec![
            "Qt and GTK4 programs take a new style when they next start.".into(),
        ]),
    ));

    Form::new(items)
}

/// The Layouts list: four 17 px rows inside its 2 px frame, between the label column and the button column.
const LAYOUTS_Y: i32 = 62;
fn layouts(a: Area) -> Rect {
    Rect::new(
        GX + 84,
        LAYOUTS_Y,
        a.gw() - 84 - BUTTON_W - 6,
        4 * reprisekit::controls::listbox::ROW_H + 4,
    )
}

pub fn keyboard(a: Area) -> Form {
    let gw = a.gw();
    let mut items = vec![group(a, "keyboard_group", 44, 190, "Keyboard")];
    let row = |label_id, id, y, text| {
        [
            Item::label_at(label_id, GX, y, 80, text),
            edit(id, GX + 84, y - 4, gw - 84),
        ]
    };
    // The layouts in switching order, an empty list being the system default; each button 4 px under the last, beside the list.
    items.push(Item::label_at(
        "layouts_label",
        GX,
        LAYOUTS_Y + 4,
        80,
        "&Layouts:",
    ));
    items.push(Item::new(
        KB_LAYOUTS,
        layouts(a),
        Control::ListBox {
            list: ListBox::default(),
            icons: Vec::new(),
        },
    ));
    let bx = a.gright() - BUTTON_W;
    let step = BUTTON_H + 4;
    items.push(button(KB_ADD, bx, LAYOUTS_Y, "Add..."));
    items.push(button(KB_REMOVE, bx, LAYOUTS_Y + step, "Remo&ve"));
    items.push(button(KB_UP, bx, LAYOUTS_Y + 2 * step, "Move &Up"));
    items.extend(row("model_label", "kb_model", 150, "&Model:"));
    items.extend(row("options_label", "kb_options", 176, "&Options:"));
    items.extend(spin(
        "rate_label",
        "repeat_rate",
        GX,
        202,
        80,
        "&Repeat rate:",
        (1, 255),
    ));
    items.extend(spin(
        "delay_label",
        "repeat_delay",
        GX + 170,
        202,
        44,
        "Dela&y:",
        (0, 9999),
    ));
    // The Shortcuts list is a `ListView` the host draws over the page; the page carries its buttons only.
    items.push(group(a, "shortcuts_group", 242, 138 + a.dh, "Shortcuts"));
    let y = shortcuts(a).bottom() + 4;
    let r = shortcuts(a).right();
    items.push(button(
        SC_ADD,
        r - 3 * BUTTON_W - 2 * BUTTON_GAP,
        y,
        "&Add...",
    ));
    items.push(button(
        SC_EDIT,
        r - 2 * BUTTON_W - BUTTON_GAP,
        y,
        "&Edit...",
    ));
    items.push(button(SC_REMOVE, r - BUTTON_W, y, "&Delete"));
    Form::new(items)
}

/// The Motion group of the Pointer Options: the speed label, Slow and Fast
/// around a trackbar, and "Enhance pointer precision" under it. `y` is the
/// label's row; the items span 62 px. `ids` are the label, Slow, the trackbar,
/// Fast and the check; the page has two of these, so only the mouse's carries the access keys.
fn motion(a: Area, y: i32, ids: [&'static str; 5], keys: bool) -> [Item; 5] {
    let [label_id, slow_id, track_id, fast_id, check_id] = ids;
    let (speed, enhance) = if keys {
        ("Select a &pointer speed:", "&Enhance pointer precision")
    } else {
        ("Select a pointer speed:", "Enhance pointer precision")
    };
    let track = Rect::new(GX + 28, y + 15, a.track(160), trackbar::HEIGHT);
    [
        Item::label_at(label_id, GX, y, a.gw(), speed),
        Item::label_at(slow_id, GX, y + 21, 26, "Slow"),
        Item::new(
            track_id,
            track,
            Control::TrackBar(TrackBar::new(SPEED_STEPS / 2, SPEED_STEPS)),
        ),
        Item::label_at(fast_id, track.right() + 4, y + 21, 26, "Fast"),
        Item::check(check_id, GX, y + 49, enhance),
    ]
}

pub fn mouse(a: Area) -> Form {
    let mut items = vec![group(a, "mouse_group", 44, 152, "Mouse")];
    items.extend(spin(
        "double_click_label",
        "double_click_ms",
        GX,
        68,
        130,
        "Double-click &speed (ms):",
        (0, 9999),
    ));
    items.extend(motion(
        a,
        94,
        [
            "mouse_speed_label",
            "mouse_slow",
            "mouse_speed",
            "mouse_fast",
            "mouse_accel",
        ],
        true,
    ));
    items.push(Item::check(
        "mouse_natural",
        GX,
        165,
        "Re&verse scrolling direction",
    ));

    items.push(group(a, "touchpad_group", 206, 218, "Touchpad"));
    items.extend(motion(
        a,
        230,
        [
            "touchpad_speed_label",
            "touchpad_slow",
            "touchpad_speed",
            "touchpad_fast",
            "touchpad_accel",
        ],
        false,
    ));
    items.push(Item::check("tap_to_click", GX, 305, "&Tap to click"));
    items.push(Item::check(
        "two_finger_scroll",
        GX,
        327,
        "Scroll with two &fingers (edge scrolling when off)",
    ));
    items.push(Item::check(
        "two_finger_tap_right_click",
        GX,
        349,
        "Two-finger tap is a &right click (middle when off)",
    ));
    items.push(Item::check(
        "disable_while_typing",
        GX,
        371,
        "&Disable while typing",
    ));
    items.push(Item::check(
        "touchpad_natural",
        GX,
        393,
        "Reverse scr&olling direction",
    ));
    Form::new(items)
}

/// The pointer theme and size, and a preview of the theme's pointers the host
/// draws into `CURSOR_PREVIEW` (inside is [`preview_size`]), which takes the page's extra width and height.
pub fn pointers(a: Area, cursor_themes: &[String]) -> Form {
    let mut themes = vec![CURSOR_DEFAULT_ROW.to_string()];
    themes.extend(cursor_themes.iter().cloned());
    let sizes: Vec<&str> = CURSOR_SIZES.iter().map(|c| c.1).collect();
    Form::new(vec![
        group(a, "cursor_group", 44, 84, "Pointer Theme"),
        Item::label_at("cursor_theme_label", GX, 66, 56, "&Scheme:"),
        combo(
            "cursor_theme",
            GX + 60,
            62,
            a.combo(Area::MIN.gw() - 60),
            &themes,
            themes.len().clamp(1, 8),
        ),
        Item::label_at("cursor_size_label", GX, 96, 56, "Si&ze:"),
        Item::combo("cursor_size", GX + 60, 92, 110, &sizes, sizes.len()),
        group(a, "cursor_preview_group", 138, 242 + a.dh, "Preview"),
        Item::new(
            CURSOR_PREVIEW,
            preview(a),
            Control::Picture {
                canvas: None,
                frame: 2,
            },
        ),
    ])
}

fn preview(a: Area) -> Rect {
    Rect::new(GX, 160, a.gw(), 208 + a.dh)
}

pub fn preview_size(a: Area) -> (u32, u32) {
    let r = preview(a);
    (r.width as u32 - 4, r.height as u32 - 4)
}

pub fn power(a: Area) -> Form {
    let gw = a.gw();
    // One row per command: the label, then its field 4 px above the label's row, as on the Keyboard page.
    let command = |label_id, id, y, text| {
        [
            Item::label_at(label_id, GX, y, 70, text),
            edit(id, GX + 74, y - 4, gw - 74),
        ]
    };
    let mut items = vec![group(a, "turnoff_group", 44, 126, "Turn Off Computer")];
    items.extend(command("standby_label", "standby", 66, "Stand &by:"));
    items.extend(command("poweroff_label", "poweroff", 92, "&Turn off:"));
    items.extend(command("reboot_label", "reboot", 118, "&Restart:"));
    items.extend(command("hibernate_label", "hibernate", 144, "&Hibernate:"));

    items.push(group(a, "idle_group", 180, 196, "Idle"));
    // The up-down for a 114 px label sits at GX + 118 and is 50 wide, so the unit follows 6 px later.
    let seconds = |id: &'static str, y| Item::label_at(id, GX + 174, y, 60, "seconds");
    items.extend(spin(
        "lock_label",
        "lock_after",
        GX,
        204,
        114,
        "&Lock after:",
        (0, 99999),
    ));
    items.push(seconds("lock_units", 204));
    items.extend(spin(
        "screen_label",
        "screen_off_after",
        GX,
        230,
        114,
        "&Screen off after:",
        (0, 99999),
    ));
    items.push(seconds("screen_units", 230));
    items.extend(spin(
        "blank_label",
        "blank_after",
        GX,
        256,
        114,
        "Screen sa&ver after:",
        (0, 99999),
    ));
    items.push(seconds("blank_units", 256));
    items.extend(spin(
        "fade_label",
        "lock_fade",
        GX,
        282,
        114,
        "&Fade before lock:",
        (0, 99999),
    ));
    items.push(seconds("fade_units", 282));
    items.push(Item::label_at(
        "idle_note",
        GX,
        308,
        gw,
        "0 disables the step.",
    ));
    items.push(Item::check(
        "lock_on_sleep",
        GX,
        330,
        "Lock the computer when it goes to slee&p",
    ));
    items.push(Item::check(
        "lock_wallpaper",
        GX,
        352,
        "Show the &wallpaper on the lock screen",
    ));
    Form::new(items)
}

/// How windows snap, tab and come back: the Snapping group holds
/// `[window_snapping]`'s drag snapping and how near an edge it starts (greyed
/// with drag snapping off), the snap layouts on the maximize button, the snap
/// bar a drag shows (greyed too) and the layouts they offer; Tabs and Placement
/// holds `[windows]`, shaking included.
pub fn windows(a: Area) -> Form {
    let gw = a.gw();
    let mut items = vec![
        group(a, "snap_group", 44, 172, "Snapping"),
        Item::check(
            "snap_enabled",
            GX,
            66,
            "Snap a &dragged window to screen edges and layouts",
        ),
        Item::check(
            "snap_picker",
            GX,
            90,
            "Show snap &layouts when resting on a maximize button",
        ),
        Item::check(
            "snap_bar",
            GX,
            114,
            "Show snap layouts in a &bar when dragging a title bar up",
        ),
    ];
    items.extend(spin(
        "snap_distance_label",
        "snap_distance",
        GX,
        142,
        150,
        "Snap distance from &edges:",
        (1, SNAP_DISTANCE_MAX as i32),
    ));
    items.extend([
        Item::label_at("snap_distance_units", GX + 210, 142, 30, "px"),
        Item::label_at("snap_layouts_label", GX, 170, 84, "&Custom layouts:"),
        edit("snap_layouts", GX + 88, 166, gw - 88),
        Item::label_at(
            "snap_layouts_note",
            GX,
            192,
            gw,
            "Empty for the built-in six, e.g. 1|1 2|1 1:2|1:2",
        ),
        group(a, "tabs_group", 228, 96, "Tabs and Placement"),
        Item::check(
            "window_tabs",
            GX,
            250,
            "Drop a window on a title bar to group them as &tabs",
        ),
        Item::check(
            "remember_windows",
            GX,
            274,
            "Re&member where each program's window was closed",
        ),
        Item::check(
            "window_shake",
            GX,
            298,
            "&Shake a window by its title bar to minimize the others",
        ),
    ]);
    Form::new(items)
}

/// The Windows page controls that grey with drag snapping off.
const DRAG_SNAP_CONTROLS: [&str; 4] = [
    "snap_bar",
    "snap_distance_label",
    "snap_distance",
    "snap_distance_units",
];

/// The virtual desktop controls that grey with one desktop; the duration's also grey without an
/// animation, and the preview rate's while nothing shows a live picture.
const DESKTOP_CONTROLS: [&str; 6] = [
    "desktops_label",
    "desktops",
    "animation_label",
    "desktop_animation",
    "pager_mark_label",
    "pager_mark",
];
const DURATION_CONTROLS: [&str; 3] = ["duration_label", "desktop_animation_ms", "duration_units"];
const FPS_CONTROLS: [&str; 3] = ["fps_label", "previews_fps", "fps_units"];

pub fn desktops(a: Area) -> Form {
    let mut items = vec![group(a, "desktops_group", 44, 168, "Virtual Desktops")];
    items.push(Item::check("use_desktops", GX, 64, "&Use virtual desktops"));
    // Indented under the check box they depend on: `ix` is the box's x, and each item starts at its label's column.
    let ix = GX;
    let mut indented = Vec::new();
    indented.extend(spin(
        "desktops_label",
        "desktops",
        ix,
        88,
        206,
        "&Number of desktops:",
        (2, DESKTOPS_MAX as i32),
    ));
    indented.push(Item::label_at(
        "animation_label",
        ix,
        114,
        110,
        "Switch &animation:",
    ));
    indented.push(choice(
        "desktop_animation",
        ix + 114,
        110,
        130,
        SWITCH_ANIMATIONS,
    ));
    indented.extend(spin(
        "duration_label",
        "desktop_animation_ms",
        ix,
        140,
        110,
        "&Duration:",
        (0, SWITCH_MS_MAX as i32),
    ));
    indented.push(Item::label_at("duration_units", ix + 170, 140, 60, "ms"));
    indented.push(Item::label_at(
        "pager_mark_label",
        ix,
        166,
        190,
        "Current desktop on the &Glass taskbar:",
    ));
    indented.push(choice("pager_mark", ix, 184, 134, PAGER_MARKS));
    items.extend(indented.into_iter().map(Item::under_check));

    items.push(group(a, "previews_group", 222, 76, "Window Pictures"));
    items.push(Item::check(
        "preview_taskbar",
        GX,
        246,
        "Show live &window pictures over taskbar buttons",
    ));
    items.extend(spin(
        "fps_label",
        "previews_fps",
        GX,
        272,
        110,
        "Re&fresh rate:",
        (1, PREVIEW_FPS_MAX as i32),
    ));
    items.push(Item::label_at(
        "fps_units",
        GX + 170,
        272,
        100,
        "times a second",
    ));
    Form::new(items)
}

/// The edge the bar sits on and how windows become task buttons, then one
/// check box per optional part of the taskbar, auto-hide with its three
/// triggers indented under it, and the Start button's logo, menu banner and menu style.
pub fn taskbar(a: Area) -> Form {
    let mut items = vec![
        Item::label_at("location_label", X, 48, 100, "Taskbar l&ocation:"),
        choice("taskbar_position", X + 104, 44, 120, TASKBAR_POSITIONS),
        Item::label_at("buttons_label", X, 76, 100, "Taskbar &buttons:"),
        choice("taskbar_buttons", X + 104, 72, 230, TASKBAR_BUTTONS),
    ];
    let checks = [
        (
            "taskbar_pinned",
            "Show the Quick Launch programs as &pinned task buttons",
        ),
        (
            "taskbar_small_icons",
            "Use &small icons on a shorter taskbar",
        ),
        (
            "taskbar_show_desktop",
            "Show &desktop button at the taskbar's end",
        ),
        (
            "taskbar_peek",
            "Pee&k at the desktop or a window from the taskbar",
        ),
        ("taskbar_tray_overflow", "Hide &idle notification icons"),
    ];
    for (i, (id, text)) in checks.into_iter().enumerate() {
        items.push(Item::check(id, X, 108 + 22 * i as i32, text));
    }
    items.push(button(
        TRAY_CUSTOMIZE,
        a.right() - BUTTON_W,
        191,
        "C&ustomize...",
    ));
    items.push(Item::new(
        "taskbar_note",
        Rect::new(X, 216, a.w, 2 * LINE_H),
        Control::Lines(vec![
            "Left unchanged, these follow the interface: Glass".into(),
            "combines buttons and turns on all but small icons.".into(),
        ]),
    ));
    items.push(Item::check(
        "taskbar_auto_hide",
        X,
        250,
        "Auto-&hide the taskbar",
    ));
    let reveals = [
        "Show when the pointer &touches the edge",
        "Show for the Start menu and window s&witching",
        "Show when a window needs &attention",
    ];
    for (i, (id, text)) in REVEAL_CONTROLS.into_iter().zip(reveals).enumerate() {
        items.push(Item::check(id, X, 272 + 22 * i as i32, text).under_check());
    }
    items.push(group(a, "start_group", 338, 94, "Start Button"));
    items.push(Item::check(
        "start_icon",
        GX,
        360,
        "Show the icon theme's &logo on the Start button",
    ));
    items.push(Item::label_at(
        "banner_label",
        GX,
        384,
        110,
        "Start menu ba&nner:",
    ));
    items.push(edit("startmenu_banner", GX + 114, 380, a.gw() - 114));
    items.push(Item::label_at(
        "startmenu_style_label",
        GX,
        412,
        110,
        "Start menu st&yle:",
    ));
    items.push(choice(
        "startmenu_style",
        GX + 114,
        408,
        a.combo(Area::MIN.gw() - 114),
        STARTMENU_STYLES,
    ));
    Form::new(items)
}

/// The auto-hide triggers' check boxes, indented under "Auto-hide the taskbar" and greyed while it is off.
const REVEAL_CONTROLS: [&str; 3] = [
    "taskbar_reveal_on_edge",
    "taskbar_reveal_on_keys",
    "taskbar_reveal_on_attention",
];

/// The Launcher page: the Search group's check for files and folders, with what it adds under it.
pub fn launcher(a: Area) -> Form {
    Form::new(vec![
        group(a, "launcher_search_group", 44, 86, "Search"),
        Item::new(
            "launcher_search_note",
            Rect::new(GX, 66, a.gw(), 2 * LINE_H),
            Control::Lines(vec![
                "The launcher finds installed programs as you type".into(),
                "their names.".into(),
            ]),
        ),
        Item::check(
            "launcher_files",
            GX,
            104,
            "Also find &files and folders in my home folder",
        ),
    ])
}

/// The Window Switcher page: what Alt+Tab shows and how large its pictures
/// are, the desktop entry and peek with it, the 3D switcher on Super+Tab, and
/// the window overview's hot corner under it.
pub fn switcher(a: Area) -> Form {
    Form::new(vec![
        group(a, "switcher_group", 44, 168, "Alt+Tab"),
        Item::label_at("switcher_style_label", GX, 68, 110, "Alt+Tab &shows:"),
        choice("switcher_style", GX + 114, 64, 130, SWITCHER_STYLES),
        Item::label_at("switcher_size_label", GX, 94, 110, "Picture si&ze:"),
        choice("switcher_size", GX + 114, 90, 130, SWITCHER_SIZES),
        Item::check(
            "switcher_preview",
            GX,
            118,
            "Show a live &preview of the selected window",
        ),
        Item::check("switcher_desktop", GX, 140, "Show the &desktop in Alt+Tab"),
        Item::check("switcher_peek", GX, 162, "Pee&k at the selected window"),
        Item::check(
            "switcher_flip",
            GX,
            184,
            "Show windows in &3D with Super+Tab",
        ),
        group(a, "overview_group", 224, 76, "Window Overview"),
        Item::new(
            "overview_note",
            Rect::new(GX, 246, a.gw(), LINE_H),
            Control::Lines(vec!["Every window of this desktop side by side.".into()]),
        ),
        Item::label_at("hot_corner_label", GX, 272, 110, "Hot &corner:"),
        choice("hot_corner", GX + 114, 268, 130, HOT_CORNERS),
    ])
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

/// The night light's controls that grey with its check box, and those that also need the fixed schedule.
const NL_CONTROLS: [&str; 7] = [
    "nl_temp_label",
    "nl_temp_value",
    "nl_warm",
    "nl_temperature",
    "nl_cool",
    "nl_sun",
    "nl_fixed",
];
const NL_TIMES: [&str; 4] = ["nl_from_label", "nl_from", "nl_to_label", "nl_to"];

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

pub fn night_light(a: Area) -> Form {
    let mut items = vec![Item::check(
        "nl_enabled",
        X,
        44,
        "&Warm the screen's colors at night",
    )];
    items.push(group(a, "nl_color_group", 70, 76, "Color Temperature"));
    items.push(Item::label_at(
        "nl_temp_label",
        GX,
        90,
        200,
        "Screen &color at night:",
    ));
    items.extend(ends_track(
        a,
        ["nl_warm", "nl_temperature", "nl_cool", "nl_temp_value"],
        117,
        27,
        ("Warmer", "Cooler"),
        GX + 46,
        TrackBar::new(0, KELVIN_STEPS),
    ));
    items.push(group(a, "nl_schedule_group", 156, 104, "Schedule"));
    items.push(radio(
        "nl_sun",
        "nl_schedule",
        GX,
        178,
        "From &sunset to sunrise",
    ));
    items.push(radio("nl_fixed", "nl_schedule", GX, 200, "At &fixed times"));
    // The times row starts at the choices' label column: the check box's, which is the radio's too in looks that draw them alike.
    let times = time("nl_from_label", "nl_from", GX, 230, 36, "F&rom:")
        .into_iter()
        .chain(time("nl_to_label", "nl_to", GX + 132, 230, 24, "&To:"));
    items.extend(times.map(Item::under_check));
    items.push(Item::new(
        "nl_note",
        Rect::new(X, 272, a.w, 4 * LINE_H),
        Control::Lines(vec![
            "Sunset and sunrise are worked out for the time zone's city, or for".into(),
            "the latitude and longitude in config.toml. Each change of color".into(),
            "fades over 30 minutes. The tray icon pauses it until morning.".into(),
        ]),
    ));
    Form::new(items)
}

/// The StickyKeys controls that grey with its check box, the SlowKeys ones and the MouseKeys ones.
const STICKY_CONTROLS: [&str; 2] = ["sticky_lock", "sticky_two_keys_off"];
const MOUSE_KEYS_CONTROLS: [&str; 5] = [
    "mouse_keys_num_lock",
    "mk_speed_label",
    "mouse_keys_speed",
    "mk_accel_label",
    "mouse_keys_acceleration",
];
const SLOW_CONTROLS: [&str; 5] = [
    "slow_delay_label",
    "slow_delay_value",
    "slow_short",
    "slow_keys_delay",
    "slow_long",
];

/// The Accessibility page: the text size on top, then StickyKeys, SlowKeys
/// and MouseKeys, each in its group, and the shortcuts that turn them on and off.
pub fn accessibility(a: Area) -> Form {
    let gw = a.gw();
    let note = |id, y, text: &str| Item::label_at(id, GX, y, gw, text);
    let sizes: Vec<&str> = TEXT_SIZES.iter().map(|c| c.1).collect();
    let mut items = vec![
        Item::label_at("text_size_label", X, 48, 70, "&Text size:"),
        Item::combo("text_size", X + 74, 44, 80, &sizes, sizes.len()),
    ];
    items.push(group(a, "sticky_group", 70, 108, "StickyKeys"));
    items.push(note(
        "sticky_note",
        88,
        "Shift, Ctrl, Alt and the Super key work one key at a time.",
    ));
    items.push(Item::check("sticky_keys", GX, 109, "&Use StickyKeys"));
    items.push(Item::check(
        "sticky_lock",
        GX,
        131,
        "&Lock a modifier key when it is pressed twice",
    ));
    items.push(Item::check(
        "sticky_two_keys_off",
        GX,
        153,
        "Turn StickyKeys off when two keys are pressed at &once",
    ));
    items.push(group(a, "slow_group", 184, 117, "SlowKeys"));
    items.push(note(
        "slow_note",
        202,
        "A key counts only once it has been held down for the delay.",
    ));
    items.push(Item::check("slow_keys", GX, 223, "U&se SlowKeys"));
    items.push(Item::label_at(
        "slow_delay_label",
        GX,
        247,
        200,
        "&Keys must be held down for:",
    ));
    items.extend(ends_track(
        a,
        [
            "slow_short",
            "slow_keys_delay",
            "slow_long",
            "slow_delay_value",
        ],
        274,
        27,
        ("Shorter", "Longer"),
        GX + 46,
        TrackBar::new(0, DELAY_STEPS),
    ));
    // The keypad moves the pointer: the switch and Num Lock's share a row, and so
    // do the speed and acceleration, a label left of each trackbar.
    let half = gw / 2;
    items.push(group(a, "mouse_keys_group", 307, 72, "MouseKeys"));
    items.push(Item::check("mouse_keys", GX, 325, "Use &MouseKeys"));
    items.push(Item::check(
        "mouse_keys_num_lock",
        GX + half,
        325,
        "Only while &Num Lock is on",
    ));
    items.push(Item::label_at("mk_speed_label", GX, 353, 40, "S&peed:"));
    items.push(Item::new(
        "mouse_keys_speed",
        Rect::new(GX + 40, 347, (half - 48).min(TRACK_MAX), trackbar::HEIGHT),
        Control::TrackBar(TrackBar::new(0, MK_SPEED.count)),
    ));
    items.push(Item::label_at(
        "mk_accel_label",
        GX + half,
        353,
        70,
        "&Acceleration:",
    ));
    items.push(Item::new(
        "mouse_keys_acceleration",
        Rect::new(
            GX + half + 70,
            347,
            (gw - half - 70).min(TRACK_MAX),
            trackbar::HEIGHT,
        ),
        Control::TrackBar(TrackBar::new(0, MK_ACCEL.count)),
    ));
    items.push(Item::check(
        "key_shortcuts",
        X,
        387,
        "Use the keyboard s&hortcuts",
    ));
    items.push(
        Item::new(
            "shortcuts_note",
            Rect::new(X, 405, a.w - 18, 3 * LINE_H),
            Control::Lines(vec![
                "Shift five times turns StickyKeys on or off, right Shift held".into(),
                "eight seconds SlowKeys, left Alt+left Shift+Num Lock MouseKeys.".into(),
            ]),
        )
        .under_check(),
    );
    Form::new(items)
}

/// The Startup page: the autostart list (a [`ListView`] the host draws over
/// the page and routes the pointer to, see `main.rs`), its two buttons, and
/// the Session group: what the next login gets back of this one, and what Log
/// Off ends. The list takes the page's extra height.
pub fn startup(a: Area) -> Form {
    let list = list(a);
    let (y, dh) = (list.bottom() + 8, a.dh);
    Form::new(vec![
        button(ADD, list.right() - 2 * BUTTON_W - BUTTON_GAP, y, "&Add..."),
        button(REMOVE, list.right() - BUTTON_W, y, "&Remove"),
        group(a, "session_group", 292 + dh, 66, "Session"),
        Item::check(
            "session_restore",
            GX,
            314 + dh,
            "Re&open the programs that were open at the last log off",
        ),
        Item::check(
            "session_end_processes",
            GX,
            336 + dh,
            "&End the programs still running when I log off",
        ),
    ])
}

/// The Effects page: window animations and the fade of hovered buttons, each a check with its two durations
/// indented under it, and the screen's fade; then the menus' shadow and timings, then the task buttons' slide.
pub fn effects(a: Area) -> Form {
    let range = (0, EFFECTS_MS_MAX as i32);
    let mut items = vec![group(a, "effects_group", 44, 158, "Windows")];
    items.push(Item::check(
        "window_animation",
        GX,
        66,
        "&Animate windows opening and closing",
    ));
    let mut anim = Vec::new();
    anim.extend(spin(
        "open_label",
        "window_open_ms",
        GX,
        92,
        70,
        "&Open ms:",
        range,
    ));
    anim.extend(spin(
        "close_label",
        "window_close_ms",
        GX + 140,
        92,
        70,
        "&Close ms:",
        range,
    ));
    items.extend(anim.into_iter().map(Item::under_check));
    items.push(Item::check(
        "hover_fade",
        GX,
        122,
        "Fade button &highlights",
    ));
    let mut hover = Vec::new();
    hover.extend(spin(
        "in_label",
        "hover_in_ms",
        GX,
        148,
        70,
        "&In ms:",
        range,
    ));
    hover.extend(spin(
        "out_label",
        "hover_out_ms",
        GX + 140,
        148,
        70,
        "O&ut ms:",
        range,
    ));
    items.extend(hover.into_iter().map(Item::under_check));
    items.push(Item::check(
        "session_fade",
        GX,
        178,
        "&Fade the screen at log on and log off",
    ));
    items.push(group(a, "menus_group", 212, 74, "Menus"));
    items.extend(spin(
        "shadow_label",
        "menu_shadow",
        GX,
        234,
        60,
        "&Shadow:",
        (0, 32),
    ));
    items.extend(spin(
        "hover_label",
        "menu_hover_ms",
        GX + 140,
        234,
        70,
        "Ho&ver ms:",
        (0, 9999),
    ));
    items.extend(spin(
        "fade_label",
        "menu_fade_ms",
        GX,
        260,
        60,
        "Fa&de ms:",
        (0, 9999),
    ));
    items.push(group(a, "taskbar_group", 296, 48, "Taskbar"));

    items.push(Item::check(
        "taskbar_slide",
        GX,
        318,
        "Slide &taskbar buttons as they are dragged",
    ));
    Form::new(items)
}

const ANIM_CONTROLS: [&str; 4] = [
    "open_label",
    "window_open_ms",
    "close_label",
    "window_close_ms",
];
const HOVER_CONTROLS: [&str; 4] = ["in_label", "hover_in_ms", "out_label", "hover_out_ms"];

enum Bind {
    Text(&'static str, Lens<Settings, String>),
    Num(&'static str, Lens<Settings, i64>),
    Flag(&'static str, Lens<Settings, bool>),
    Choice(&'static str, Lens<Settings, String>),
    Named(&'static str, Lens<Settings, String>, &'static Choices),
    /// A [`Bind::Choice`] whose first rows are the labels of `Choices` standing for their values (`CURSOR_DEFAULT_ROW` for "").
    ChoiceOrDefault(&'static str, Lens<Settings, String>, &'static Choices),
    NamedNum(
        &'static str,
        Lens<Settings, i64>,
        &'static [(i64, &'static str)],
    ),
    /// The "Use virtual desktops" check box: ticked is more than one desktop.
    /// Unticking writes 1; ticking writes the count the up-down beside it shows.
    UseDesktops(&'static str, Lens<Settings, i64>),
    /// The desktop water check box: ticked is any style, unticked "off"; ticking takes the style the combo shows.
    Water(&'static str, Lens<Settings, String>),
    WaterStyle(&'static str, Lens<Settings, String>),
    /// The desktop count's up-down, 2 up. One desktop shows the default count, so ticking the check box starts from it.
    Desktops(&'static str, Lens<Settings, i64>),
    Track(&'static str, Lens<Settings, i64>),
    Kelvin(&'static str, Lens<Settings, i64>),
    Radio(&'static str, Lens<Settings, String>, &'static str),
    /// A check box standing for the field holding its first value when ticked, its second when not.
    Switch(
        &'static str,
        Lens<Settings, String>,
        &'static str,
        &'static str,
    ),
    Time(&'static str, Lens<Settings, String>),
    Delay(&'static str, Lens<Settings, i64>),
    Steps(&'static str, Lens<Settings, i64>, Steps),
    /// A scheme combo: the interface's own row (`own`) stands for the empty string, any other row for its name.
    Scheme(&'static str, Lens<Settings, String>, &'static str),
    /// The glass colour combo: a swatch row for its name, the Custom row for `Settings::glass_custom`.
    GlassColor(&'static str, Lens<Settings, String>),
    /// A trackbar whose position is the value less the first (glass intensity from 0, blur from 1).
    Level(&'static str, Lens<Settings, i64>, i64),
    /// A colour button against "#RRGGBB". Empty shows the desktop colour the look
    /// gives and stays empty while the button still shows it, so reading an untouched button back writes nothing.
    Hex(&'static str, Lens<Settings, String>),
    Icon(&'static str, Lens<Settings, Vec<String>>),
}

fn binds() -> Vec<Bind> {
    vec![
        Bind::Text("task_manager", lens!(task_manager)),
        Bind::Text("startmenu_banner", lens!(startmenu_banner)),
        Bind::Flag("desktop_icons", lens!(desktop_icons)),
        Bind::Named(
            crate::backgroundpage::EXTENSIONS,
            lens!(desktop_extensions),
            crate::model::DESKTOP_EXTENSIONS,
        ),
        Bind::Flag("start_icon", lens!(start_icon)),
        Bind::Flag("adaptive_sync", lens!(adaptive_sync)),
        Bind::Flag("switcher_preview", lens!(switcher_preview)),
        Bind::Flag("switcher_desktop", lens!(switcher_desktop)),
        Bind::Flag("switcher_peek", lens!(switcher_peek)),
        Bind::Flag("switcher_flip", lens!(switcher_flip)),
        Bind::Flag("preview_taskbar", lens!(preview_taskbar)),
        Bind::Num("previews_fps", lens!(previews_fps)),
        Bind::Named("switcher_style", lens!(switcher_style), SWITCHER_STYLES),
        Bind::Named("switcher_size", lens!(switcher_size), SWITCHER_SIZES),
        Bind::Named(
            crate::defaultspage::DRIVE_INSERT,
            lens!(drive_insert),
            crate::model::DRIVE_INSERTS,
        ),
        Bind::Named(
            crate::defaultspage::SCRIPT_END,
            lens!(script_terminal),
            crate::model::SCRIPT_ENDS,
        ),
        Bind::Named("hot_corner", lens!(hot_corner), HOT_CORNERS),
        Bind::Flag("screenshot_save", lens!(screenshot_save)),
        Bind::Flag("clipboard_history", lens!(clipboard_history)),
        Bind::Named("interface", lens!(interface), INTERFACES),
        Bind::Scheme(
            crate::appearancepage::CLASSIC_SCHEME,
            lens!(color_scheme),
            crate::appearancepage::classic_default(),
        ),
        Bind::Scheme(
            crate::appearancepage::BLUE_SCHEME,
            lens!(color_scheme),
            reprisekit::blue::Variant::default().name(),
        ),
        Bind::GlassColor(crate::appearancepage::GLASS_COLOR, lens!(glass_color)),
        Bind::Named("glass_mode", lens!(glass_mode), crate::model::GLASS_MODES),
        Bind::Named(
            "glass_material",
            lens!(glass_material),
            crate::model::GLASS_MATERIALS,
        ),
        Bind::Named(
            "glass_frame",
            lens!(glass_frame),
            crate::model::GLASS_FRAMES,
        ),
        Bind::Named(
            "glass_notification_style",
            lens!(glass_notification_style),
            crate::model::GLASS_NOTIFY_STYLES,
        ),
        Bind::Level("glass_intensity", lens!(glass_intensity), 0),
        Bind::Flag("glass_transparency", lens!(glass_transparency)),
        Bind::Level("glass_blur", lens!(glass_blur), 1),
        Bind::Flag("glass_streaks", lens!(glass_streaks)),
        Bind::Flag("glass_fast_drag", lens!(glass_fast_drag)),
        Bind::Flag("glass_blur_frames", lens!(glass_blur_frames)),
        Bind::Level("glass_orb_glow", lens!(glass_orb_glow), 0),
        Bind::Named("wallpaper_position", lens!(wallpaper_position), POSITIONS),
        Bind::NamedNum(
            crate::backgroundpage::INTERVAL,
            lens!(slideshow_interval),
            crate::model::SLIDE_INTERVALS,
        ),
        Bind::Flag("slideshow_shuffle", lens!(slideshow_shuffle)),
        Bind::Hex("background", lens!(background)),
        Bind::Icon("documents", lens!(system_icons)),
        Bind::Icon("computer", lens!(system_icons)),
        Bind::Icon("home", lens!(system_icons)),
        Bind::UseDesktops("use_desktops", lens!(desktops)),
        Bind::Desktops("desktops", lens!(desktops)),
        Bind::Named(
            "desktop_animation",
            lens!(desktop_animation),
            SWITCH_ANIMATIONS,
        ),
        Bind::Num("desktop_animation_ms", lens!(desktop_animation_ms)),
        Bind::Named("pager_mark", lens!(pager_mark), PAGER_MARKS),
        Bind::Flag("remember_windows", lens!(remember_windows)),
        Bind::Flag("window_tabs", lens!(window_tabs)),
        Bind::Flag("window_shake", lens!(window_shake)),
        Bind::Flag("session_restore", lens!(session_restore)),
        Bind::Flag("session_end_processes", lens!(session_end_processes)),
        Bind::Named(
            "taskbar_position",
            lens!(taskbar_position),
            TASKBAR_POSITIONS,
        ),
        Bind::Named("taskbar_buttons", lens!(taskbar_buttons), TASKBAR_BUTTONS),
        Bind::Named("startmenu_style", lens!(startmenu_style), STARTMENU_STYLES),
        Bind::Flag("taskbar_pinned", lens!(taskbar_pinned)),
        Bind::Flag("taskbar_small_icons", lens!(taskbar_small_icons)),
        Bind::Flag("taskbar_show_desktop", lens!(taskbar_show_desktop)),
        Bind::Flag("taskbar_peek", lens!(taskbar_peek)),
        Bind::Flag("taskbar_tray_overflow", lens!(taskbar_tray_overflow)),
        Bind::Flag("taskbar_auto_hide", lens!(taskbar_auto_hide)),
        Bind::Flag("taskbar_reveal_on_edge", lens!(taskbar_reveal_on_edge)),
        Bind::Flag("taskbar_reveal_on_keys", lens!(taskbar_reveal_on_keys)),
        Bind::Flag(
            "taskbar_reveal_on_attention",
            lens!(taskbar_reveal_on_attention),
        ),
        Bind::Flag("recent_files", lens!(recent_files)),
        Bind::Flag(PRIVACY_LOCATION, lens!(location)),
        Bind::Flag("capture_border", lens!(capture_border)),
        Bind::Water(WATER, lens!(wallpaper_water)),
        Bind::WaterStyle(WATER_STYLE, lens!(wallpaper_water)),
        Bind::Flag(DEBUG_OVERLAY, lens!(debug_overlay)),
        Bind::Flag(DEBUG_IN_CAPTURES, lens!(debug_overlay_in_captures)),
        Bind::Flag("launcher_files", lens!(launcher_files)),
        Bind::Flag(THUMBS, lens!(thumbnail_cache)),
        Bind::Num(THUMBS_MB, lens!(thumbnail_max_mb)),
        Bind::Text("font_family", lens!(font_family)),
        Bind::Num("font_size_px", lens!(font_size_px)),
        Bind::Flag("font_antialias", lens!(font_antialias)),
        Bind::Text("font_shell_dlg", lens!(font_shell_dlg)),
        Bind::Choice("icon_theme", lens!(icon_theme)),
        Bind::ChoiceOrDefault("cursor_theme", lens!(cursor_theme), CURSOR_FIXED_ROWS),
        Bind::ChoiceOrDefault("gtk_theme", lens!(gtk_theme), GTK_FIXED_ROWS),
        Bind::Named(
            "qt_platform_theme",
            lens!(qt_platform_theme),
            QT_PLATFORM_THEMES,
        ),
        Bind::Named("font_smoothing", lens!(font_smoothing), FONT_SMOOTHINGS),
        Bind::Flag("gtk4_colors", lens!(gtk4_colors)),
        Bind::NamedNum("cursor_size", lens!(cursor_size), CURSOR_SIZES),
        Bind::Num("frame_width", lens!(frame_width)),
        Bind::Num("caption_height", lens!(caption_height)),
        Bind::Num("taskbar_height", lens!(taskbar_height)),
        Bind::Num("menu_shadow", lens!(menu_shadow)),
        Bind::Num("menu_hover_ms", lens!(menu_hover_ms)),
        Bind::Num("menu_fade_ms", lens!(menu_fade_ms)),
        Bind::Flag("window_animation", lens!(window_animation)),
        Bind::Num("window_open_ms", lens!(window_open_ms)),
        Bind::Num("window_close_ms", lens!(window_close_ms)),
        Bind::Flag("hover_fade", lens!(hover_fade)),
        Bind::Num("hover_in_ms", lens!(hover_in_ms)),
        Bind::Num("hover_out_ms", lens!(hover_out_ms)),
        Bind::Flag("taskbar_slide", lens!(taskbar_slide)),
        Bind::Flag("session_fade", lens!(session_fade)),
        Bind::Num("double_click_ms", lens!(double_click_ms)),
        Bind::Text("kb_model", lens!(kb_model)),
        Bind::Text("kb_options", lens!(kb_options)),
        Bind::Num("repeat_rate", lens!(repeat_rate)),
        Bind::Num("repeat_delay", lens!(repeat_delay)),
        Bind::Text("standby", lens!(standby)),
        Bind::Text("poweroff", lens!(poweroff)),
        Bind::Text("reboot", lens!(reboot)),
        Bind::Text("hibernate", lens!(hibernate)),
        Bind::Flag("lock_on_sleep", lens!(lock_on_sleep)),
        Bind::Flag("lock_wallpaper", lens!(lock_wallpaper)),
        Bind::Num("lock_after", lens!(lock_after)),
        Bind::Num("screen_off_after", lens!(screen_off_after)),
        Bind::Num("blank_after", lens!(blank_after)),
        Bind::Num("lock_fade", lens!(lock_fade)),
        Bind::Track("mouse_speed", lens!(mouse_speed)),
        Bind::Flag("mouse_accel", lens!(mouse_accel)),
        Bind::Flag("mouse_natural", lens!(mouse_natural)),
        Bind::Track("touchpad_speed", lens!(touchpad_speed)),
        Bind::Flag("touchpad_accel", lens!(touchpad_accel)),
        Bind::Flag("touchpad_natural", lens!(touchpad_natural)),
        Bind::Flag("tap_to_click", lens!(tap_to_click)),
        Bind::Flag("two_finger_scroll", lens!(two_finger_scroll)),
        Bind::Flag(
            "two_finger_tap_right_click",
            lens!(two_finger_tap_right_click),
        ),
        Bind::Flag("disable_while_typing", lens!(disable_while_typing)),
        Bind::Flag("snap_enabled", lens!(snap_enabled)),
        Bind::Flag("snap_picker", lens!(snap_picker)),
        Bind::Flag("snap_bar", lens!(snap_bar)),
        Bind::Text("snap_layouts", lens!(snap_layouts)),
        Bind::Num("snap_distance", lens!(snap_distance)),
        Bind::Flag("nl_enabled", lens!(nl_enabled)),
        Bind::Kelvin("nl_temperature", lens!(nl_temperature)),
        Bind::Radio("nl_sun", lens!(nl_schedule), "sun"),
        Bind::Radio("nl_fixed", lens!(nl_schedule), "fixed"),
        Bind::Time("nl_from", lens!(nl_from)),
        Bind::Time("nl_to", lens!(nl_to)),
        Bind::Flag("sticky_keys", lens!(sticky_keys)),
        Bind::Flag("sticky_lock", lens!(sticky_lock)),
        Bind::Flag("sticky_two_keys_off", lens!(sticky_two_keys_off)),
        Bind::Flag("slow_keys", lens!(slow_keys)),
        Bind::Delay("slow_keys_delay", lens!(slow_keys_delay)),
        Bind::Flag("key_shortcuts", lens!(key_shortcuts)),
        Bind::Named(
            "color_filter",
            lens!(color_filter),
            crate::visionpage::FILTERS,
        ),
        Bind::Level("color_filter_intensity", lens!(color_filter_intensity), 0),
        Bind::Switch(
            "magnifier_follow",
            lens!(magnifier_follow),
            "pointer+focus",
            "pointer",
        ),
        Bind::Flag("mouse_keys", lens!(mouse_keys)),
        Bind::Flag("mouse_keys_num_lock", lens!(mouse_keys_num_lock)),
        Bind::Steps("mouse_keys_speed", lens!(mouse_keys_speed), MK_SPEED),
        Bind::Steps(
            "mouse_keys_acceleration",
            lens!(mouse_keys_acceleration),
            MK_ACCEL,
        ),
        Bind::NamedNum("text_size", lens!(text_size), TEXT_SIZES),
    ]
}

fn hhmm(s: &str) -> (u32, u32) {
    s.split_once(':')
        .and_then(|(h, m)| Some((h.trim().parse().ok()?, m.trim().parse().ok()?)))
        .filter(|&(h, m)| h < 24 && m < 60)
        .unwrap_or((0, 0))
}

fn kelvin_pos(k: i64) -> usize {
    ((k.clamp(KELVIN_MIN, KELVIN_MAX) - KELVIN_MIN + KELVIN_STEP / 2) / KELVIN_STEP) as usize
}

impl Bind {
    fn id(&self) -> &'static str {
        match self {
            Bind::Text(id, _)
            | Bind::Num(id, _)
            | Bind::Flag(id, _)
            | Bind::Choice(id, _)
            | Bind::Named(id, _, _)
            | Bind::ChoiceOrDefault(id, ..)
            | Bind::NamedNum(id, _, _)
            | Bind::UseDesktops(id, _)
            | Bind::Water(id, _)
            | Bind::WaterStyle(id, _)
            | Bind::Desktops(id, _)
            | Bind::Track(id, _)
            | Bind::Kelvin(id, _)
            | Bind::Radio(id, _, _)
            | Bind::Switch(id, _, _, _)
            | Bind::Time(id, _)
            | Bind::Delay(id, _)
            | Bind::Steps(id, _, _)
            | Bind::Scheme(id, _, _)
            | Bind::GlassColor(id, _)
            | Bind::Level(id, _, _)
            | Bind::Hex(id, _)
            | Bind::Icon(id, _) => id,
        }
    }
}

/// Push `s` into every bound control and the Layouts list. Rebuilds the edits,
/// so call it on load or cancel, never on a keystroke: `TextEdit::new` puts the caret at the end.
pub fn write(sheet: &mut PropSheet, s: &Settings, reg: &[Layout]) {
    fill_layouts(sheet, s, reg, None);
    crate::menupage::fill(&mut sheet.pages[Page::StartMenu.index()], &s.startmenu);
    write_forms(sheet, s);
    follow(sheet, s);
}

pub fn write_forms(sheet: &mut PropSheet, s: &Settings) {
    for page in &mut sheet.pages {
        write_form(page, s);
    }
}

/// Push `s` into the bound controls `form` has; rebuilds its edits, so never on a keystroke.
pub fn write_form(form: &mut Form, s: &Settings) {
    for b in binds() {
        let Some(item) = form.item_mut(b.id()) else {
            continue;
        };
        match (&b, &mut item.control) {
            (Bind::Text(_, at), Control::Edit { edit, first, .. }) => {
                *edit = TextEdit::new(at.get(s).clone(), false);
                first.set(0);
            }
            (Bind::Num(_, at), Control::UpDown(u)) => {
                u.set(*at.get(s) as i32);
            }
            (Bind::Flag(_, at), Control::Check { checked, .. }) => *checked = *at.get(s),
            (Bind::Choice(_, at), Control::Combo(c)) => {
                let want = at.get(s).clone();
                // A value the list does not offer (a theme that went away) joins it rather than being dropped.
                if !c.items.iter().any(|i| i.text == want) {
                    c.items.push(combo::ListItem::new(want.clone()));
                }
                c.selected = c.items.iter().position(|i| i.text == want).unwrap_or(0);
            }
            (Bind::ChoiceOrDefault(_, at, fixed), Control::Combo(c)) => {
                let v = at.get(s).as_str();
                let want = fixed
                    .iter()
                    .find(|(f, _)| *f == v)
                    .map_or(v, |(_, row)| *row);
                if !c.items.iter().any(|i| i.text == want) {
                    c.items.push(combo::ListItem::new(want.to_string()));
                }
                c.selected = c.items.iter().position(|i| i.text == want).unwrap_or(0);
            }
            (Bind::NamedNum(_, at, choices), Control::Combo(c)) => {
                let want = *at.get(s);
                c.items.truncate(choices.len());
                c.selected = match choices.iter().position(|(v, _)| *v == want) {
                    Some(i) => i,
                    None => {
                        c.items.push(combo::ListItem::new(want.to_string()));
                        choices.len()
                    }
                };
            }
            (Bind::Named(_, at, choices), Control::Combo(c)) => {
                let want = at.get(s);
                // A value the config does not know shows as itself after the known ones.
                c.items.truncate(choices.len());
                c.selected = match choices.iter().position(|(v, _)| v == want) {
                    Some(i) => i,
                    None => {
                        c.items.push(combo::ListItem::new(want.clone()));
                        choices.len()
                    }
                };
            }
            (Bind::UseDesktops(_, at), Control::Check { checked, .. }) => *checked = *at.get(s) > 1,
            (Bind::Water(_, at), Control::Check { checked, .. }) => *checked = at.get(s) != "off",
            (Bind::WaterStyle(_, at), Control::Combo(c)) => {
                if let Some(i) = WATER_STYLES.iter().position(|(v, _)| v == at.get(s)) {
                    c.selected = i;
                }
            }
            (Bind::Desktops(_, at), Control::UpDown(u)) => {
                let n = match *at.get(s) {
                    n if n > 1 => n,
                    _ => DESKTOPS_DEFAULT,
                };
                u.set(n as i32);
            }
            (Bind::Track(_, at), Control::TrackBar(t)) => {
                t.set((*at.get(s) + 10).clamp(0, SPEED_STEPS as i64 - 1) as usize);
            }
            (Bind::Kelvin(_, at), Control::TrackBar(t)) => {
                t.set(kelvin_pos(*at.get(s)));
            }
            (Bind::Radio(_, at, value), Control::Radio { checked, .. }) => {
                *checked = at.get(s) == value;
            }
            (Bind::Switch(_, at, on, _), Control::Check { checked, .. }) => {
                *checked = at.get(s) == on;
            }
            (Bind::Time(_, at), Control::Time(tf)) => {
                let (h, m) = hhmm(at.get(s));
                tf.set(h, m, 0);
            }
            (Bind::Delay(_, at), Control::TrackBar(t)) => {
                t.set((at.get(s).clamp(&DELAY_MIN, &DELAY_MAX) - DELAY_MIN) as usize);
            }
            (Bind::Steps(_, at, steps), Control::TrackBar(t)) => {
                t.set(steps.pos(*at.get(s)));
            }
            (Bind::Scheme(_, at, own), Control::Combo(c)) => {
                let want = match at.get(s).as_str() {
                    "" => *own,
                    v => v,
                };
                // A name the list lacks (another interface's) shows as itself.
                if !c.items.iter().any(|i| i.text == want) {
                    c.items.push(combo::ListItem::new(want.to_string()));
                }
                c.selected = c.items.iter().position(|i| i.text == want).unwrap_or(0);
            }
            (Bind::GlassColor(_, at), Control::Combo(c)) => {
                c.selected = crate::appearancepage::glass_row(at.get(s), c.items.len());
            }
            (Bind::Level(_, at, first), Control::TrackBar(t)) => {
                t.set((*at.get(s) - first).clamp(0, 100) as usize);
            }
            (Bind::Hex(_, at), Control::Color(b)) => {
                b.color = Color::parse(at.get(s))
                    .unwrap_or_else(|| crate::appearancepage::theme(s).colors.background);
            }
            (Bind::Icon(name, at), Control::Check { checked, .. }) => {
                *checked = at.get(s).iter().any(|n| n == name);
            }
            _ => {}
        }
    }
}

/// Grey what depends on another control: the tap button map without
/// tap-to-click; the Layouts buttons by the list (Add up to four, Remove on a
/// row, Move Up below the first); the night light's controls by its check box
/// and times by the fixed schedule; the StickyKeys, SlowKeys and delay
/// options; the Vision filter's intensity unless a correction is picked; on
/// Window Switcher, the preview check box while Alt+Tab shows thumbnails and
/// the 3D switcher outside Glass; on Desktops, the refresh rate while nothing
/// shows a live picture, and the count and animation without "Use virtual
/// desktops". The Appearance page is rebuilt for the pending interface and its
/// sample redrawn; the Background page greys the position without a wallpaper
/// and the file name extensions without the desktop's files, and shows the look's desktop
/// colour on its Color button; the Taskbar page
/// greys the auto-hide triggers while auto-hide is off, and the Advanced page
/// the debug overlay's captures check while the overlay is off and the thumbnail cache's size
/// while the cache is off. Runs from
/// `write` and after every read-back.
pub fn follow(sheet: &mut PropSheet, s: &Settings) {
    crate::appearancepage::follow(sheet, s);
    crate::backgroundpage::follow(&mut sheet.pages[Page::Background.index()], s);
    crate::saverpage::follow(&mut sheet.pages[Page::ScreenSaver.index()], s);
    if let Some(Control::UpDown(u)) = sheet.pages[Page::Power.index()]
        .item_mut("blank_after")
        .map(|i| &mut i.control)
    {
        u.set(s.blank_after.clamp(0, i32::MAX as i64) as i32);
    }
    let thumbnails = s.switcher_style == "thumbnails";
    let live = s.preview_taskbar || thumbnails || s.switcher_preview;
    let many = s.desktops > 1;
    let sp = &mut sheet.pages[Page::Switcher.index()];
    sp.set_enabled("switcher_preview", !thumbnails);
    sp.set_enabled("switcher_size_label", thumbnails);
    sp.set_enabled("switcher_size", thumbnails);
    sp.set_enabled("switcher_flip", s.interface == "glass");
    let water = &mut sheet.pages[Page::Advanced.index()];
    for id in [WATER_STYLE, "water_note", "water_note_2"] {
        water.set_enabled(id, s.wallpaper_water != "off");
    }
    let wp = &mut sheet.pages[Page::Windows.index()];
    for id in DRAG_SNAP_CONTROLS {
        wp.set_enabled(id, s.snap_enabled);
    }
    let dp = &mut sheet.pages[Page::Desktops.index()];
    for id in FPS_CONTROLS {
        dp.set_enabled(id, live);
    }
    for id in DESKTOP_CONTROLS {
        dp.set_enabled(id, many);
    }
    for id in ["pager_mark_label", "pager_mark"] {
        dp.set_enabled(id, many && s.interface == "glass");
    }
    for id in DURATION_CONTROLS {
        dp.set_enabled(id, many && s.desktop_animation != "none");
    }
    for page in &mut sheet.pages {
        if let Some(Control::Check { enabled, .. }) = page
            .item_mut("two_finger_tap_right_click")
            .map(|i| &mut i.control)
        {
            *enabled = s.tap_to_click;
        }
    }
    let nl = &mut sheet.pages[Page::NightLight.index()];
    for id in NL_CONTROLS {
        nl.set_enabled(id, s.nl_enabled);
    }
    for id in NL_TIMES {
        nl.set_enabled(id, s.nl_enabled && s.nl_schedule == "fixed");
    }
    nl.set_text("nl_temp_value", &format!("{} K", s.nl_temperature));
    let ax = &mut sheet.pages[Page::Accessibility.index()];
    for id in STICKY_CONTROLS {
        ax.set_enabled(id, s.sticky_keys);
    }
    for id in SLOW_CONTROLS {
        ax.set_enabled(id, s.slow_keys);
    }
    for id in MOUSE_KEYS_CONTROLS {
        ax.set_enabled(id, s.mouse_keys);
    }
    let tenths = s.slow_keys_delay;
    ax.set_text(
        "slow_delay_value",
        &format!("{}.{} seconds", tenths / 10, tenths % 10),
    );
    crate::visionpage::follow(&mut sheet.pages[Page::Vision.index()], s);
    let fx = &mut sheet.pages[Page::Effects.index()];
    for id in ANIM_CONTROLS {
        fx.set_enabled(id, s.window_animation);
    }
    for id in HOVER_CONTROLS {
        fx.set_enabled(id, s.hover_fade);
    }
    let tb = &mut sheet.pages[Page::Taskbar.index()];
    for id in REVEAL_CONTROLS {
        tb.set_enabled(id, s.taskbar_auto_hide);
    }
    tb.set_enabled(TRAY_CUSTOMIZE, s.taskbar_tray_overflow);
    let adv = &mut sheet.pages[Page::Advanced.index()];
    adv.set_enabled(DEBUG_IN_CAPTURES, s.debug_overlay);
    for id in THUMBS_SIZE {
        adv.set_enabled(id, s.thumbnail_cache);
    }
    let count = s.layouts().len();
    let picked = layout_selection(sheet);
    let page = &mut sheet.pages[Page::Keyboard.index()];
    let lit = [
        (KB_ADD, count < MAX_LAYOUTS),
        (KB_REMOVE, picked.is_some()),
        (KB_UP, picked.is_some_and(|i| i > 0)),
    ];
    for (id, on) in lit {
        page.set_enabled(id, on);
    }
    let focused = page.focus().and_then(|f| page.items.get(f)).map(|i| i.id);
    if lit.iter().any(|&(id, on)| !on && focused == Some(id)) {
        page.focus_id(KB_LAYOUTS);
    }
}

pub fn fill_layouts(sheet: &mut PropSheet, s: &Settings, reg: &[Layout], select: Option<usize>) {
    let rows: Vec<String> = s
        .layouts()
        .iter()
        .map(|(l, v)| reprisefs::xkb::describe(reg, l, v).description)
        .collect();
    let keep = select.or(layout_selection(sheet));
    if let Some(Control::ListBox { list, .. }) = sheet.pages[Page::Keyboard.index()]
        .item_mut(KB_LAYOUTS)
        .map(|i| &mut i.control)
    {
        let n = rows.len();
        *list = ListBox::new(rows, keep.filter(|&i| i < n));
    }
    follow(sheet, s);
}

pub fn layout_selection(sheet: &PropSheet) -> Option<usize> {
    match sheet.pages[Page::Keyboard.index()]
        .item(KB_LAYOUTS)
        .map(|i| &i.control)
    {
        Some(Control::ListBox { list, .. }) => list.selected,
        _ => None,
    }
}

/// Pull the control `id`'s value back into `s`, if it is a bound one.
///
/// Per-control on purpose: an up-down clamps whatever the file held into its
/// range when built, so reading every control back would fold those clamps
/// into the pending state and the next Apply would rewrite a key the user
/// never touched. The app reads back only what it saw a `form::Event::Changed` for.
pub fn read_one(sheet: &PropSheet, id: &str, s: &mut Settings) {
    for b in binds() {
        if b.id() != id {
            continue;
        }
        for page in &sheet.pages {
            let Some(item) = page.item(b.id()) else {
                continue;
            };
            match (&b, &item.control) {
                (Bind::Text(_, at), Control::Edit { edit, .. }) => {
                    *at.get_mut(s) = edit.text.clone()
                }
                (Bind::Num(_, at), Control::UpDown(u)) => *at.get_mut(s) = u.value() as i64,
                (Bind::Flag(_, at), Control::Check { checked, .. }) => *at.get_mut(s) = *checked,
                (Bind::Choice(_, at), Control::Combo(c)) => {
                    if let Some(v) = c.items.get(c.selected) {
                        *at.get_mut(s) = v.text.clone();
                    }
                }
                (Bind::ChoiceOrDefault(_, at, fixed), Control::Combo(c)) => {
                    if let Some(v) = c.items.get(c.selected) {
                        let t = v.text.as_str();
                        *at.get_mut(s) = fixed
                            .iter()
                            .find(|(_, row)| *row == t)
                            .map_or(t, |(value, _)| *value)
                            .to_string();
                    }
                }
                (Bind::NamedNum(_, at, choices), Control::Combo(c)) => {
                    let value = match choices.get(c.selected) {
                        Some((v, _)) => Some(*v),
                        None => c.items.get(c.selected).and_then(|i| i.text.parse().ok()),
                    };
                    if let Some(v) = value {
                        *at.get_mut(s) = v;
                    }
                }
                (Bind::Named(_, at, choices), Control::Combo(c)) => {
                    let value = match choices.get(c.selected) {
                        Some((v, _)) => Some(v.to_string()),
                        None => c.items.get(c.selected).map(|i| i.text.clone()),
                    };
                    if let Some(v) = value {
                        *at.get_mut(s) = v;
                    }
                }
                (Bind::UseDesktops(_, at), Control::Check { checked, .. }) => {
                    *at.get_mut(s) = match page.item("desktops").map(|i| &i.control) {
                        Some(Control::UpDown(u)) if *checked => u.value() as i64,
                        _ if *checked => DESKTOPS_DEFAULT,
                        _ => 1,
                    }
                }
                (Bind::Water(_, at), Control::Check { checked, .. }) => {
                    let style = page
                        .item(WATER_STYLE)
                        .and_then(|i| match &i.control {
                            Control::Combo(c) => WATER_STYLES.get(c.selected),
                            _ => None,
                        })
                        .map_or("calm", |(v, _)| *v);
                    if !*checked {
                        *at.get_mut(s) = "off".into();
                    } else if at.get(s) == "off" {
                        *at.get_mut(s) = style.into();
                    }
                }
                (Bind::WaterStyle(_, at), Control::Combo(c)) => {
                    if at.get(s) != "off"
                        && let Some((v, _)) = WATER_STYLES.get(c.selected)
                    {
                        *at.get_mut(s) = v.to_string();
                    }
                }
                (Bind::Desktops(_, at), Control::UpDown(u)) => *at.get_mut(s) = u.value() as i64,
                (Bind::Track(_, at), Control::TrackBar(t)) => *at.get_mut(s) = t.pos as i64 - 10,
                (Bind::Kelvin(_, at), Control::TrackBar(t)) => {
                    *at.get_mut(s) = KELVIN_MIN + t.pos as i64 * KELVIN_STEP
                }
                (Bind::Radio(_, at, value), Control::Radio { checked: true, .. }) => {
                    *at.get_mut(s) = value.to_string()
                }
                (Bind::Switch(_, at, on, off), Control::Check { checked, .. }) => {
                    *at.get_mut(s) = if *checked { on } else { off }.to_string()
                }
                (Bind::Time(_, at), Control::Time(tf)) => {
                    *at.get_mut(s) = format!("{:02}:{:02}", tf.h, tf.m)
                }
                (Bind::Delay(_, at), Control::TrackBar(t)) => {
                    *at.get_mut(s) = DELAY_MIN + t.pos as i64
                }
                (Bind::Steps(_, at, steps), Control::TrackBar(t)) => {
                    *at.get_mut(s) = steps.value(t.pos)
                }
                (Bind::Scheme(_, at, own), Control::Combo(c)) => {
                    if let Some(v) = c.items.get(c.selected) {
                        *at.get_mut(s) = if v.text == *own {
                            String::new()
                        } else {
                            v.text.clone()
                        };
                    }
                }
                (Bind::GlassColor(_, at), Control::Combo(c)) => {
                    let v = match reprisekit::glass::swatch_names().nth(c.selected) {
                        Some(n) => n.to_string(),
                        None => s.glass_custom.clone(),
                    };
                    if !v.is_empty() {
                        *at.get_mut(s) = v;
                    }
                }
                (Bind::Level(_, at, first), Control::TrackBar(t)) => {
                    *at.get_mut(s) = t.pos as i64 + first
                }
                (Bind::Hex(_, at), Control::Color(b)) => {
                    let own = at.get(s).is_empty()
                        && b.color == crate::appearancepage::theme(s).colors.background;
                    if !own {
                        let c = b.color;
                        *at.get_mut(s) = format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b);
                    }
                }
                (Bind::Icon(name, at), Control::Check { checked, .. }) => {
                    let icons = crate::model::system_icons_with(at.get(s), name, *checked);
                    *at.get_mut(s) = icons;
                }
                _ => {}
            }
        }
    }
}

pub fn new_list() -> ListView {
    ListView {
        columns: COLUMNS
            .iter()
            .map(|(title, w)| Column::new(title, *w))
            .collect(),
        checks: Some(Vec::new()),
        ..ListView::default()
    }
}

pub fn fit_list_columns(
    lv: &mut ListView,
    cm: &reprisekit::controls::metrics::ControlMetrics,
    rect: Rect,
) {
    lv.fill_beside_bar(1, rect, cm);
}

pub fn fill_list(lv: &mut ListView, entries: &[Entry], checks: &[bool]) {
    lv.rows = entries
        .iter()
        .map(|e| Row {
            icon: None,
            cells: vec![
                e.name.clone(),
                e.exec.clone(),
                crate::autostart::location(e).into(),
            ],
        })
        .collect();
    lv.checks = Some(checks.to_vec());
    if lv.selection.focus.is_some_and(|i| i >= lv.rows.len()) {
        lv.selection.set_only(None);
    }
    lv.top = lv.top.min(lv.rows.len().saturating_sub(1));
}

pub fn new_shortcut_list() -> ListView {
    ListView {
        columns: SHORTCUT_COLUMNS
            .iter()
            .map(|(title, w)| Column::new(title, *w))
            .collect(),
        ..ListView::default()
    }
}

/// Give Action what is left beside the look's scrollbar, so the list that scrolls grows no horizontal bar.
pub fn fit_shortcut_columns(
    lv: &mut ListView,
    cm: &reprisekit::controls::metrics::ControlMetrics,
    rect: Rect,
) {
    lv.fill_beside_bar(1, rect, cm);
}

/// Refill `lv` from `s` in [`crate::model::row`]'s order: the desktop's binds
/// (a spawn shows its command after the action), each program shortcut as
/// "Program: description", then each program refused one. `name` gives a
/// program's name for its app id.
pub fn fill_shortcuts(lv: &mut ListView, s: &Settings, name: &dyn Fn(&str) -> String) {
    let row = |keys: String, what: String| Row {
        icon: None,
        cells: vec![keys, what],
    };
    let binds = s.shortcuts.iter().map(|sc| {
        let what = if sc.action == "spawn" {
            format!("spawn {}", sc.command)
        } else {
            sc.action.clone()
        };
        row(sc.keys.clone(), what)
    });
    let programs = s.programs.iter().map(|p| {
        let keys = if p.keys.is_empty() { "None" } else { &p.keys };
        row(keys.into(), crate::model::program_text(p, &name(&p.app)))
    });
    let denied =
        (s.denied.iter()).map(|app| row(String::new(), format!("{}: not allowed", name(app))));
    lv.rows = binds.chain(programs).chain(denied).collect();
    if lv.selection.focus.is_some_and(|i| i >= lv.rows.len()) {
        lv.selection.set_only(None);
    }
    lv.top = lv.top.min(lv.rows.len().saturating_sub(1));
}

pub fn new_location_list() -> ListView {
    ListView {
        columns: LOCATION_COLUMNS
            .iter()
            .map(|(title, w)| Column::new(title, *w))
            .collect(),
        ..ListView::default()
    }
}

pub fn fit_location_columns(
    lv: &mut ListView,
    cm: &reprisekit::controls::metrics::ControlMetrics,
    rect: Rect,
) {
    lv.fill_beside_bar(0, rect, cm);
}

pub fn fill_locations(lv: &mut ListView, answers: &[(String, bool)]) {
    lv.rows = answers
        .iter()
        .map(|(name, allow)| Row {
            icon: None,
            cells: vec![
                name.clone(),
                if *allow { "Allowed" } else { "Denied" }.into(),
            ],
        })
        .collect();
    if lv.selection.focus.is_some_and(|i| i >= lv.rows.len()) {
        lv.selection.set_only(None);
    }
    lv.top = lv.top.min(lv.rows.len().saturating_sub(1));
}

/// Draw the Shortcuts list over the Keyboard page; `rect` is [`shortcuts`] where the page sits now.
pub fn draw_shortcuts(
    c: &mut Canvas,
    text: &TextRenderer,
    theme: &Theme,
    lv: &ListView,
    rect: Rect,
    px: u32,
    s: i32,
) {
    listview::draw(
        c,
        text,
        &Paint { s, ..theme.paint() },
        lv,
        &listview::layout(lv, rect, &theme.controls()),
        None,
        px,
    );
}

/// Draw the Startup list (or the Advanced page's location answers) over the
/// page the sheet already drew; `rect` is [`list`] (or [`locations`]) where the page sits now.
pub fn draw_list(
    c: &mut Canvas,
    text: &TextRenderer,
    theme: &Theme,
    lv: &ListView,
    rect: Rect,
    px: u32,
    s: i32,
) {
    listview::draw(
        c,
        text,
        &Paint { s, ..theme.paint() },
        lv,
        &listview::layout(lv, rect, &theme.controls()),
        None,
        px,
    );
}

/// The key the Startup list takes from a press, or `None` when it is not the
/// list's own. An editing key is degraded first: the space bar reaches a host
/// as an insert, and the list is not a form control.
pub fn list_key(key: &form::Key) -> Option<form::Key> {
    let flat = match key {
        form::Key::Edit(e) => form::flat_key(e)?,
        other => other.clone(),
    };
    matches!(
        flat,
        form::Key::Up
            | form::Key::Down
            | form::Key::Home
            | form::Key::End
            | form::Key::PageUp
            | form::Key::PageDown
            | form::Key::Space
    )
    .then_some(flat)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appearancepage;
    use crate::backgroundpage;
    use crate::model::Shortcut;
    use crate::registry;
    use reprisekit::controls::propsheet;
    use reprisekit::raster::Canvas;
    use reprisekit::theme::{FontConfig, Theme};

    fn read_all(sheet: &PropSheet, s: &mut Settings) {
        for b in binds() {
            read_one(sheet, b.id(), s);
        }
    }

    fn with(s: &Settings) -> PropSheet {
        registry::sheet(
            s,
            &registry::Sources {
                icon_themes: &themes(),
                gtk_themes: &gtk_themes(),
                qt_tools: true,
                cursor_themes: &cursor_themes(),
                layouts: &reg(),
                heads: &[],
            },
        )
    }

    /// Every control's access key is its page's alone, the sheet's own buttons
    /// aside ("&Apply" shares Alt+A with "&Add..." by design), under every interface.
    #[test]
    fn no_two_controls_on_a_page_share_an_access_key() {
        for i in ["classic", "blue", "glass"] {
            let s = under(i);
            for p in Page::all() {
                let mut seen: Vec<(char, &str)> = Vec::new();
                for item in &s.pages[p.index()].items {
                    if matches!(
                        item.id,
                        propsheet::OK | propsheet::CANCEL | propsheet::APPLY
                    ) {
                        continue;
                    }
                    let label = match &item.control {
                        Control::Label { text, .. } => text,
                        Control::Check { label, .. }
                        | Control::Radio { label, .. }
                        | Control::Button { label, .. } => label,
                        _ => continue,
                    };
                    let Some(k) = reprisekit::text::access_key(label) else {
                        continue;
                    };
                    if let Some((_, other)) = seen.iter().find(|(c, _)| *c == k) {
                        panic!("{i}, {}: {other} and {} share Alt+{k}", p.label(), item.id);
                    }
                    seen.push((k, item.id));
                }
            }
        }
    }

    /// A bound control sits on one page at most under each interface, and on one
    /// under some interface: none lost in a move or rebuild, none built twice.
    #[test]
    fn every_bind_names_a_control_on_exactly_one_page() {
        let sheets = ["classic", "blue", "glass"].map(under);
        for b in binds() {
            let mut shown = false;
            for s in &sheets {
                let on: Vec<&str> = Page::all()
                    .filter(|p| s.pages[p.index()].item(b.id()).is_some())
                    .map(Page::label)
                    .collect();
                assert!(on.len() <= 1, "{} is on {on:?}", b.id());
                shown |= on.len() == 1;
            }
            assert!(shown, "{} is on no page under any interface", b.id());
        }
    }

    /// Every page's text fits its rects under each interface, in Tahoma and in DejaVu Sans, the
    /// default family's second face, at each text size.
    #[test]
    fn every_page_fits_its_text_under_every_interface() {
        let over = reprisekit::interfaces::form_overflows(|i, _, _| {
            let sheet = under(i.as_str());
            Page::all()
                .map(|p| (p.label().to_string(), sheet.pages[p.index()].clone()))
                .collect()
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    /// The dialogs the pages open fit their text too, with long layout, variant and program names.
    #[test]
    fn every_dialog_fits_its_text_under_every_interface() {
        let reg = reprisefs::xkb::parse(
            "<layoutList>\
             <layout><configItem><name>us</name><description>English (US)</description>\
             </configItem><variantList><variant><configItem><name>intl</name>\
             <description>English (US, intl., with dead keys)</description></configItem>\
             </variant></variantList></layout>\
             <layout><configItem><name>hu</name><description>Hungarian</description>\
             </configItem><variantList><variant><configItem><name>101_qwerty_comma_dead</name>\
             <description>Hungarian (QWERTY, 101-key, comma, dead keys)</description>\
             </configItem></variant></variantList></layout>\
             </layoutList>",
        );
        let app = |name: &str| {
            let entry = format!("[Desktop Entry]\nType=Application\nName={name}\nExec=true\n");
            reprisefs::apps::parse_desktop(&entry, "a.desktop").unwrap()
        };
        let long = Shortcut::new(
            "Super+Shift+Alt+Page_Down",
            "spawn",
            "flatpak run org.example.App",
        );
        let over = reprisekit::interfaces::form_overflows(|_, _, _| {
            let mut forms = vec![
                (
                    "Add Item".into(),
                    crate::additem::AddItemDialog::new(
                        vec![app("LibreOffice Impress"), app("Text Editor")],
                        "foot",
                    )
                    .form,
                ),
                (
                    "Add Layout".into(),
                    crate::layoutdlg::LayoutDialog::new(&reg).form,
                ),
                (
                    "Keep display".into(),
                    crate::monitordlg::MonitorDialog::new(15, None).form,
                ),
                (
                    "Rename".into(),
                    crate::renamedlg::RenameDialog::new("Documents").form,
                ),
                (
                    "Shortcut".into(),
                    crate::shortcutdlg::ShortcutDialog::new(Some(&long), vec![]).form,
                ),
                (
                    "Program shortcut".into(),
                    crate::shortcutdlg::ShortcutDialog::for_program("Ctrl+Alt+T", vec![]).form,
                ),
            ];
            for kind in reprisekit::saver::Kind::ALL {
                let d = crate::saverdlg::SaverDialog::new(kind, &Settings::default());
                forms.push((format!("{kind:?} settings"), d.form));
            }
            forms
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    fn under(interface: &str) -> PropSheet {
        with(&Settings {
            interface: interface.into(),
            ..Settings::default()
        })
    }

    fn pick(s: &mut PropSheet, got: &mut Settings, page: Page, id: &str, row: usize) {
        if let Some(Control::Combo(c)) = s.pages[page.index()].item_mut(id).map(|i| &mut i.control)
        {
            c.selected = row;
        }
        read_one(s, id, got);
        follow(s, got);
    }

    fn tick(s: &mut PropSheet, got: &mut Settings, page: Page, id: &str, on: bool) {
        if let Some(Control::Check { checked, .. }) =
            s.pages[page.index()].item_mut(id).map(|i| &mut i.control)
        {
            *checked = on;
        }
        read_one(s, id, got);
        follow(s, got);
    }

    fn rows(s: &PropSheet, page: Page, id: &str) -> (Vec<String>, usize) {
        match &s.pages[page.index()]
            .item(id)
            .unwrap_or_else(|| panic!("{id} is on {}", page.label()))
            .control
        {
            Control::Combo(c) => (c.items.iter().map(|i| i.text.clone()).collect(), c.selected),
            _ => panic!("{id} is a combo"),
        }
    }

    #[test]
    fn the_appearance_page_geometry() {
        let s = built();
        let p = Page::Appearance;
        assert_eq!(
            rect(&s, p, appearancepage::SAMPLE),
            Rect::new(24, 44, 350, 188)
        );
        assert_eq!(rect(&s, p, "interface_label"), Rect::new(20, 242, 196, 13));
        assert_eq!(rect(&s, p, "interface"), Rect::new(20, 258, 196, 21));
        assert_eq!(rect(&s, p, "scheme_label"), Rect::new(20, 288, 196, 13));
        assert_eq!(
            rect(&s, p, appearancepage::CLASSIC_SCHEME),
            Rect::new(20, 304, 196, 21)
        );
        inside_page(&s, p);
        let g = under("glass");
        assert_eq!(
            rect(&g, p, appearancepage::GLASS_COLOR),
            Rect::new(20, 304, 196, 21)
        );
        assert_eq!(rect(&g, p, "material_label"), Rect::new(20, 334, 90, 13));
        assert_eq!(rect(&g, p, "glass_material"), Rect::new(20, 350, 90, 21));
        assert_eq!(rect(&g, p, "frame_label"), Rect::new(126, 334, 90, 13));
        assert_eq!(rect(&g, p, "glass_frame"), Rect::new(126, 350, 90, 21));
        assert_eq!(
            rect(&g, p, "glass_transparency"),
            Rect::new(20, 384, 13, 13)
        );
        assert_eq!(rect(&g, p, "glass_fast_drag"), Rect::new(20, 406, 13, 13));
        assert_eq!(rect(&g, p, "glass_blur_frames"), Rect::new(20, 428, 13, 13));
        assert_eq!(rect(&g, p, "glass_streaks"), Rect::new(232, 384, 13, 13));
        assert_eq!(rect(&g, p, "intensity_label"), Rect::new(232, 288, 146, 13));
        assert_eq!(rect(&g, p, "glass_intensity"), Rect::new(232, 304, 146, 26));
        assert_eq!(rect(&g, p, "blur_label"), Rect::new(232, 334, 146, 13));
        assert_eq!(rect(&g, p, "glass_blur"), Rect::new(232, 350, 146, 26));
        assert_eq!(rect(&g, p, "orb_glow_label"), Rect::new(232, 406, 146, 13));
        assert_eq!(rect(&g, p, "glass_orb_glow"), Rect::new(232, 422, 146, 26));
        assert_eq!(rect(&g, p, "notify_label"), Rect::new(20, 452, 90, 13));
        assert_eq!(
            rect(&g, p, "glass_notification_style"),
            Rect::new(20, 468, 90, 21)
        );
        inside_page(&g, p);
    }

    #[test]
    fn glass_shows_a_mode_combo_that_writes_glass_mode() {
        let g = under("glass");
        let p = Page::Appearance;
        assert_eq!(
            rect(&g, p, "glass_mode").x,
            232,
            "the Mode combo stands right of the interface combo"
        );
        assert!(
            under("classic").pages[p.index()]
                .item("glass_mode")
                .is_none(),
            "only Glass has it"
        );
    }

    #[test]
    fn glass_material_and_frame_combos_write_their_keys() {
        let p = Page::Appearance;
        let mut got = Settings {
            interface: "glass".into(),
            ..Settings::default()
        };
        let mut s = with(&got);
        assert_eq!(
            rows(&s, p, "glass_material"),
            (vec!["Frosted".into(), "Smoked".into(), "Molten".into()], 0)
        );
        assert_eq!(
            rows(&s, p, "glass_frame"),
            (vec!["Rounded".into(), "Square".into()], 0)
        );
        pick(&mut s, &mut got, p, "glass_material", 1);
        pick(&mut s, &mut got, p, "glass_frame", 1);
        assert_eq!(
            (got.glass_material.as_str(), got.glass_frame.as_str()),
            ("smoked", "square")
        );
        let g = appearancepage::glass(&got);
        assert_eq!(
            (g.material, g.frame),
            (
                reprisekit::glass::Material::Smoked,
                reprisekit::glass::Frame::Square
            ),
            "the sample draws with them"
        );
    }

    #[test]
    fn glass_notification_combo_writes_its_key() {
        let p = Page::Appearance;
        let mut got = Settings {
            interface: "glass".into(),
            glass_notification_style: "unlisted".into(),
            ..Settings::default()
        };
        let mut s = with(&got);
        assert_eq!(
            rows(&s, p, "glass_notification_style").0[..3],
            ["Lozenge", "Halo", "Droplet"]
        );
        pick(&mut s, &mut got, p, "glass_notification_style", 0);
        assert_eq!(got.glass_notification_style, "lozenge");
        pick(&mut s, &mut got, p, "glass_notification_style", 1);
        assert_eq!(got.glass_notification_style, "halo");
        pick(&mut s, &mut got, p, "glass_notification_style", 2);
        assert_eq!(got.glass_notification_style, "droplet");
        assert!(
            under("classic").pages[p.index()]
                .item("glass_notification_style")
                .is_none()
        );
    }

    #[test]
    fn the_glass_rows_show_under_glass_only() {
        let glass_rows = [
            "glass_mode",
            "glass_material",
            "glass_frame",
            "glass_transparency",
            "glass_fast_drag",
            "glass_blur_frames",
            "glass_intensity",
            "glass_blur",
            "glass_streaks",
            "glass_orb_glow",
            "glass_notification_style",
        ];
        let p = Page::Appearance;
        for i in ["classic", "blue"] {
            let s = under(i);
            for id in glass_rows {
                assert!(s.pages[p.index()].item(id).is_none(), "{id} under {i}");
            }
        }
        let mut got = Settings {
            interface: "glass".into(),
            ..Settings::default()
        };
        let mut s = with(&got);
        assert!(
            glass_rows
                .iter()
                .all(|id| s.pages[p.index()].item(id).is_some())
        );
        // The interface moves: the page is rebuilt for Blue, the focus staying on the combo.
        s.pages[p.index()].focus_id("interface");
        pick(&mut s, &mut got, p, "interface", 1);
        assert_eq!(got.interface, "blue");
        assert!(
            s.pages[p.index()]
                .item(appearancepage::GLASS_COLOR)
                .is_none()
        );
        assert_eq!(
            rows(&s, p, appearancepage::BLUE_SCHEME).0,
            [
                "Blue",
                "Olive Green",
                "Silver",
                "Slate",
                "Sapphire",
                "Obsidian",
                "Ember",
                "Cobalt",
                "Carbon",
                "Steel"
            ]
        );
        assert!(
            glass_rows
                .iter()
                .all(|id| s.pages[p.index()].item(id).is_none())
        );
        let focused = |s: &PropSheet| {
            let form = &s.pages[p.index()];
            form.focus().map(|f| form.items[f].id)
        };
        assert_eq!(
            focused(&s),
            Some("interface"),
            "the rebuild keeps the focus"
        );
        // A revert moves the interface under the scheme combo: the focus goes to the new interface's.
        s.pages[p.index()].focus_id(appearancepage::BLUE_SCHEME);
        got.interface = "glass".into();
        follow(&mut s, &got);
        assert_eq!(focused(&s), Some(appearancepage::GLASS_COLOR));
    }

    #[test]
    fn the_scheme_list_is_the_interfaces_own() {
        let p = Page::Appearance;
        let (classic, at) = rows(&built(), p, appearancepage::CLASSIC_SCHEME);
        assert_eq!(classic, reprisekit::schemes::names().collect::<Vec<_>>());
        assert_eq!(
            classic[at],
            appearancepage::classic_default(),
            "no key: the default row"
        );
        let mut got = Settings::default();
        let mut s = with(&got);
        let brick = classic
            .iter()
            .position(|n| n == "Brick")
            .expect("Brick is a Classic scheme");
        pick(&mut s, &mut got, p, appearancepage::CLASSIC_SCHEME, brick);
        assert_eq!(got.color_scheme, "Brick");
        let standard = classic
            .iter()
            .position(|n| n == appearancepage::classic_default())
            .unwrap();
        pick(
            &mut s,
            &mut got,
            p,
            appearancepage::CLASSIC_SCHEME,
            standard,
        );
        assert_eq!(got.color_scheme, "", "the default row writes no key");
        let (glass, at) = rows(&under("glass"), p, appearancepage::GLASS_COLOR);
        assert_eq!(
            glass.len(),
            reprisekit::glass::swatch_names().count(),
            "no Custom row"
        );
        assert_eq!((glass[0].as_str(), at), ("Sky", 0));
    }

    #[test]
    fn a_custom_glass_colour_keeps_its_row() {
        let p = Page::Appearance;
        let mut got = crate::model::load(
            "[appearance]\ninterface = \"glass\"\n[glass]\ncolor = \"#102030\"\n",
        );
        let mut s = with(&got);
        let (list, at) = rows(&s, p, appearancepage::GLASS_COLOR);
        assert_eq!(
            (list.last().map(String::as_str), at),
            (Some(appearancepage::CUSTOM_ROW), list.len() - 1)
        );
        let untouched = got.clone();
        read_one(&s, "interface", &mut got);
        assert_eq!(got, untouched, "an untouched sheet writes nothing");
        pick(&mut s, &mut got, p, appearancepage::GLASS_COLOR, 3);
        assert_eq!(got.glass_color, "leaf");
        assert_eq!(
            rows(&s, p, appearancepage::GLASS_COLOR).0.len(),
            list.len(),
            "Custom stays"
        );
        pick(
            &mut s,
            &mut got,
            p,
            appearancepage::GLASS_COLOR,
            list.len() - 1,
        );
        assert_eq!(got.glass_color, "#102030");
    }

    #[test]
    fn the_backdrop_rows_grey_without_transparency() {
        let p = Page::Appearance;
        let mut got = Settings {
            interface: "glass".into(),
            ..Settings::default()
        };
        let mut s = with(&got);
        let rows = [
            "glass_fast_drag",
            "glass_blur_frames",
            "glass_streaks",
            "blur_label",
            "glass_blur",
        ];
        for id in rows {
            assert!(enabled_on(&s, p, id), "{id}");
        }
        tick(&mut s, &mut got, p, "glass_transparency", false);
        assert!(!got.glass_transparency);
        for id in rows {
            assert!(
                !enabled_on(&s, p, id),
                "opaque glass has no backdrop for {id}"
            );
        }
    }

    #[test]
    fn the_reflections_grey_on_smoked_and_molten_glass() {
        let p = Page::Appearance;
        let mut got = Settings {
            interface: "glass".into(),
            ..Settings::default()
        };
        let mut s = with(&got);
        for (row, on) in [(1, false), (2, false), (0, true)] {
            pick(&mut s, &mut got, p, "glass_material", row);
            assert_eq!(
                enabled_on(&s, p, "glass_streaks"),
                on,
                "{} glass",
                got.glass_material
            );
            assert!(enabled_on(&s, p, "glass_blur"), "{}", got.glass_material);
        }
    }

    #[test]
    fn the_blur_trackbar_starts_at_blur_1() {
        let p = Page::Appearance;
        let mut got = Settings {
            interface: "glass".into(),
            ..Settings::default()
        };
        let mut s = with(&got);
        let Some(Control::TrackBar(t)) = s.pages[p.index()].item("glass_blur").map(|i| &i.control)
        else {
            panic!("a blur trackbar");
        };
        assert_eq!((t.pos, t.count), (1, 5), "the default blur 2 of 1..5");
        if let Some(Control::TrackBar(t)) = s.pages[p.index()]
            .item_mut("glass_blur")
            .map(|i| &mut i.control)
        {
            t.set(0);
        }
        read_one(&s, "glass_blur", &mut got);
        assert_eq!(got.glass_blur, 1, "the leftmost position is blur 1");
    }

    #[test]
    fn the_start_button_glow_trackbar_writes_orb_glow() {
        let p = Page::Appearance;
        let mut got = Settings {
            interface: "glass".into(),
            glass_orb_glow: 70,
            ..Settings::default()
        };
        let mut s = with(&got);
        let Some(Control::TrackBar(t)) = s.pages[p.index()]
            .item("glass_orb_glow")
            .map(|i| &i.control)
        else {
            panic!("a Start button glow trackbar");
        };
        assert_eq!((t.pos, t.count), (70, 101), "70 of 0..100");
        for v in [0, 50, 100] {
            if let Some(Control::TrackBar(t)) = s.pages[p.index()]
                .item_mut("glass_orb_glow")
                .map(|i| &mut i.control)
            {
                t.set(v);
            }
            read_one(&s, "glass_orb_glow", &mut got);
            assert_eq!(got.glass_orb_glow, v as i64);
            let base = "[appearance]\ninterface = \"glass\"\n";
            let text = crate::model::save(base, &got, &crate::model::load(base)).unwrap();
            assert_eq!(crate::model::load(&text).glass_orb_glow, v as i64, "{text}");
        }
    }

    /// The sample follows the glass colour, intensity and transparency: its look
    /// is the pending `[glass]`'s style, and a redraw shows the new colour.
    #[test]
    fn the_sample_follows_the_glass_settings() {
        use reprisekit::glass;
        use reprisekit::look::Look;
        use std::sync::Arc;
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let p = Page::Appearance;
        let draw = |s: &PropSheet| {
            let mut c = Canvas::new(s.size.0 as u32, s.size.1 as u32);
            form::draw(
                &mut c,
                &text,
                &Theme::default().paint(),
                &s.pages[p.index()],
                false,
                px,
            );
            c
        };
        let drawn_in_pending = |s: &PropSheet, got: &Settings| match &s.pages[p.index()]
            .item(appearancepage::SAMPLE)
            .unwrap()
            .control
        {
            Control::Sample {
                theme:
                    Theme {
                        look: Look::Styled(a),
                        ..
                    },
                ..
            } => Arc::ptr_eq(a, &glass::style(&appearancepage::glass(got))),
            _ => panic!("a Glass sample"),
        };
        let mut got = Settings {
            interface: "glass".into(),
            ..Settings::default()
        };
        let mut s = with(&got);
        let before = draw(&s);
        pick(&mut s, &mut got, p, appearancepage::GLASS_COLOR, 3);
        assert_eq!(got.glass_color, "leaf");
        assert!(drawn_in_pending(&s, &got), "the colour");
        assert_ne!(
            draw(&s).pixels,
            before.pixels,
            "a redraw shows the leaf glass"
        );
        if let Some(Control::TrackBar(t)) = s.pages[p.index()]
            .item_mut("glass_intensity")
            .map(|i| &mut i.control)
        {
            t.set(80);
        }
        read_one(&s, "glass_intensity", &mut got);
        follow(&mut s, &got);
        assert_eq!(got.glass_intensity, 80);
        assert!(drawn_in_pending(&s, &got), "the intensity");
        tick(&mut s, &mut got, p, "glass_transparency", false);
        assert!(drawn_in_pending(&s, &got), "the transparency");
    }

    #[test]
    fn the_sample_draws_the_pending_scheme() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let p = Page::Appearance;
        let draw = |s: &PropSheet| {
            let mut c = Canvas::new(s.size.0 as u32, s.size.1 as u32);
            form::draw(
                &mut c,
                &text,
                &Theme::default().paint(),
                &s.pages[p.index()],
                false,
                px,
            );
            c
        };
        let s = with(&Settings {
            color_scheme: "Brick".into(),
            ..Settings::default()
        });
        let r = s.on_page(appearancepage::sample_rect(Area::MIN));
        let brick = reprisekit::schemes::scheme("Brick").expect("Brick is a Classic scheme");
        assert_eq!(draw(&s).get(r.x + 3, r.y + 3), brick.background.argb());
        let g = with(&Settings {
            interface: "glass".into(),
            background: "#102030".into(),
            ..Settings::default()
        });
        assert_eq!(
            draw(&g).get(r.x + 3, r.y + 3),
            Color::rgb(0x10, 0x20, 0x30).argb()
        );
    }

    /// The lists the host draws over the Startup and Keyboard pages move with
    /// their pages: right of the sheet's list, inside the page, flush with the buttons under them.
    #[test]
    fn the_side_lists_move_with_their_pages() {
        let text = TextRenderer::load(&FontConfig::default());
        let s = built();
        let page = s.layout(&text, Theme::default().fonts.size_px).page;
        let list = s.on_page(list(Area::MIN));
        assert_eq!((list.x, list.y, list.width), (166, 44, 358));
        assert!(list.x >= page.x && list.right() <= page.right() && list.bottom() <= page.bottom());
        let remove = s.pages[Page::Startup.index()].item(REMOVE).unwrap().rect;
        assert_eq!(
            remove.right(),
            list.right(),
            "Remove is flush with the list"
        );
        let shortcuts = s.on_page(shortcuts(Area::MIN));
        let delete = s.pages[Page::Keyboard.index()]
            .item(SC_REMOVE)
            .unwrap()
            .rect;
        assert_eq!(delete.right(), shortcuts.right());
    }

    fn themes() -> Vec<String> {
        vec!["hicolor".into(), "Adwaita".into()]
    }

    fn reg() -> Vec<Layout> {
        reprisefs::xkb::parse(
            "<layoutList>\
             <layout><configItem><name>us</name><shortDescription>en</shortDescription>\
             <description>English (US)</description></configItem></layout>\
             <layout><configItem><name>hu</name><shortDescription>hu</shortDescription>\
             <description>Hungarian</description></configItem></layout>\
             </layoutList>",
        )
    }

    fn cursor_themes() -> Vec<String> {
        vec!["Adwaita".into(), "breeze_cursors".into()]
    }

    fn gtk_themes() -> Vec<String> {
        vec!["Adwaita".into(), "Arc-Dark".into()]
    }

    fn first_ink(c: &Canvas, r: Rect) -> Option<i32> {
        (r.x..r.right()).find(|&x| (r.y..r.bottom()).any(|y| c.get(x, y) & 0xFF < 0xA0))
    }

    /// The text under a check box or radio starts at that choice's label column in
    /// every look: the shortcuts note, the Desktops page's indented rows and the
    /// Night Light times row under "At fixed times". That row takes the check
    /// box's column, so under Classic and Blue, whose radio label sits a pixel left
    /// of a check box's, it starts a pixel right of the radio's.
    #[test]
    fn lines_under_a_choice_start_at_its_label_in_every_look() {
        if !reprisekit::text::test_tahoma() {
            return;
        }
        use reprisekit::interface::Interface;
        let text = TextRenderer::load(&FontConfig::default());
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = reprisekit::interfaces::theme_for(i);
            let cm = theme.look.controls();
            for (form, choice, under, past) in [
                (
                    accessibility(Area::MIN),
                    "key_shortcuts",
                    "shortcuts_note",
                    0,
                ),
                (desktops(Area::MIN), "use_desktops", "desktops_label", 0),
                (
                    night_light(Area::MIN),
                    "nl_fixed",
                    "nl_from_label",
                    cm.check_label - cm.radio_label,
                ),
            ] {
                let mut c = Canvas::new(420, 420);
                c.fill_rect(c.bounds(), theme.colors.button_face);
                form::draw(&mut c, &text, &theme.paint(), &form, false, 11);
                let at = |id: &str| {
                    let it = form.item(id).unwrap();
                    let r = it.placed(&cm);
                    Rect::new(r.x - 4, r.y, 60, 13)
                };
                let box_x = form.item(choice).unwrap().rect.x;
                let label = Rect {
                    x: box_x + 14,
                    ..at(choice)
                };
                assert_eq!(
                    first_ink(&c, at(under)),
                    first_ink(&c, label).map(|x| x + past),
                    "{i:?}: {under} starts {past} px past {choice}'s label"
                );
            }
        }
    }

    fn built() -> PropSheet {
        with(&Settings::default())
    }

    fn rect(sheet: &PropSheet, page: Page, id: &str) -> Rect {
        let r = sheet.pages[page.index()]
            .item(id)
            .unwrap_or_else(|| panic!("{id} is on {}", page.label()))
            .placed(&reprisekit::controls::metrics::ControlMetrics::CLASSIC);
        let (dx, dy) = sheet.page_offset();
        Rect::new(r.x - dx, r.y - dy, r.width, r.height)
    }

    fn inside_page(sheet: &PropSheet, page: Page) {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let p = sheet.layout(&text, px).page;
        for item in &sheet.pages[page.index()].items {
            let r = item.placed(&reprisekit::controls::metrics::ControlMetrics::CLASSIC);
            if matches!(
                item.id,
                propsheet::OK | propsheet::CANCEL | propsheet::APPLY
            ) {
                continue;
            }
            assert!(
                r.x >= p.x && r.y >= p.y && r.right() <= p.right() && r.bottom() <= p.bottom(),
                "{} at {r:?} escapes the page {p:?}",
                item.id
            );
        }
    }

    #[test]
    fn the_start_menu_page_sits_inside_its_page() {
        inside_page(&built(), Page::StartMenu);
    }

    #[test]
    fn the_accessibility_page_geometry() {
        let s = built();
        let p = Page::Accessibility;
        assert_eq!(rect(&s, p, "text_size"), Rect::new(94, 44, 80, 21));
        assert_eq!(rect(&s, p, "sticky_group"), Rect::new(20, 70, 358, 108));
        assert_eq!(rect(&s, p, "sticky_keys"), Rect::new(32, 109, 13, 13));
        assert_eq!(rect(&s, p, "sticky_lock"), Rect::new(32, 131, 13, 13));
        assert_eq!(
            rect(&s, p, "sticky_two_keys_off"),
            Rect::new(32, 153, 13, 13)
        );
        assert_eq!(rect(&s, p, "slow_group"), Rect::new(20, 184, 358, 117));
        assert_eq!(rect(&s, p, "slow_keys"), Rect::new(32, 223, 13, 13));
        assert_eq!(rect(&s, p, "slow_delay_value"), Rect::new(286, 247, 80, 13));
        assert_eq!(
            rect(&s, p, "slow_keys_delay"),
            Rect::new(78, 268, 238, trackbar::HEIGHT)
        );
        assert_eq!(rect(&s, p, "mouse_keys_group"), Rect::new(20, 307, 358, 72));
        assert_eq!(rect(&s, p, "mouse_keys"), Rect::new(32, 325, 13, 13));
        assert_eq!(
            rect(&s, p, "mouse_keys_num_lock"),
            Rect::new(199, 325, 13, 13)
        );
        assert_eq!(
            rect(&s, p, "mouse_keys_speed"),
            Rect::new(72, 347, 119, trackbar::HEIGHT)
        );
        assert_eq!(
            rect(&s, p, "mouse_keys_acceleration"),
            Rect::new(269, 347, 97, trackbar::HEIGHT)
        );
        assert_eq!(rect(&s, p, "key_shortcuts"), Rect::new(20, 387, 13, 13));
        inside_page(&s, p);
    }

    #[test]
    fn accessibility_values_go_in_and_come_back_out() {
        let want = Settings {
            sticky_keys: true,
            sticky_lock: false,
            sticky_two_keys_off: true,
            slow_keys: false,
            slow_keys_delay: 15,
            key_shortcuts: false,
            mouse_keys: false,
            mouse_keys_num_lock: false,
            mouse_keys_speed: 700,
            mouse_keys_acceleration: 20,
            text_size: 175,
            ..Settings::default()
        };
        let s = with(&want);
        let page = &s.pages[Page::Accessibility.index()];
        assert_eq!(page.track_pos("slow_keys_delay"), Some(12));
        assert_eq!(page.track_pos("mouse_keys_speed"), Some(6));
        assert_eq!(page.track_pos("mouse_keys_acceleration"), Some(6));
        assert_eq!(page.combo_selected("text_size"), Some(3));
        assert_eq!(page.checked("mouse_keys_num_lock"), Some(false));
        assert_eq!(page.checked("sticky_lock"), Some(false));
        assert_eq!(page.checked("key_shortcuts"), Some(false));
        match &page.item("slow_delay_value").unwrap().control {
            Control::Label { text, .. } => assert_eq!(text, "1.5 seconds"),
            _ => panic!("a label"),
        }
        let enabled = |id: &str| match &page.item(id).unwrap().control {
            Control::Check { enabled, .. } | Control::Label { enabled, .. } => *enabled,
            Control::TrackBar(t) => t.enabled,
            _ => panic!("{id} greys"),
        };
        assert!(enabled("sticky_lock") && enabled("sticky_two_keys_off"));
        assert!(!enabled("slow_keys_delay") && !enabled("slow_delay_label"));
        assert!(!enabled("mouse_keys_speed") && !enabled("mouse_keys_num_lock"));
        let mut got = Settings::default();
        read_all(&s, &mut got);
        assert_eq!(got, want);
    }

    #[test]
    fn the_vision_page_geometry() {
        let s = built();
        let p = Page::Vision;
        assert_eq!(rect(&s, p, "cf_group"), Rect::new(20, 44, 358, 110));
        assert_eq!(rect(&s, p, "color_filter"), Rect::new(106, 62, 150, 21));
        assert_eq!(rect(&s, p, "cf_preview"), Rect::new(266, 60, 100, 28));
        assert_eq!(
            rect(&s, p, "color_filter_intensity"),
            Rect::new(96, 96, 222, trackbar::HEIGHT)
        );
        assert_eq!(rect(&s, p, "cf_note"), Rect::new(32, 132, 334, 13));
        assert_eq!(rect(&s, p, "mag_group"), Rect::new(20, 164, 358, 93));
        assert_eq!(rect(&s, p, "magnifier_follow"), Rect::new(32, 184, 13, 13));
        assert_eq!(rect(&s, p, "mag_note"), Rect::new(32, 206, 334, 39));
        inside_page(&s, p);
        if !reprisekit::text::test_tahoma() {
            return;
        }
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let page = &s.pages[p.index()];
        let Control::Lines(lines) = &page.item("mag_note").unwrap().control else {
            panic!("the note is lines");
        };
        let check = "When zoomed in, follow the text cursor and focus";
        for line in lines.iter().map(String::as_str).chain([check]) {
            let w = text.measure(line, false, px);
            assert!(w <= 334 - 18, "{line:?} is {w} px, past the group");
        }
    }

    /// The filter combo stands for its name, the trackbar for the percent, the
    /// magnifier's check box for "pointer+focus"; the intensity greys with no correction.
    #[test]
    fn color_filter_values_go_in_and_come_back_out() {
        let want = Settings {
            color_filter: "tritanopia".into(),
            color_filter_intensity: 40,
            magnifier_follow: "pointer".into(),
            ..Settings::default()
        };
        let mut s = with(&want);
        let page = &s.pages[Page::Vision.index()];
        assert_eq!(page.checked("magnifier_follow"), Some(false));
        assert_eq!(page.combo_selected("color_filter"), Some(5));
        assert_eq!(page.track_pos("color_filter_intensity"), Some(40));
        match &page.item("cf_intensity_value").unwrap().control {
            Control::Label { text, enabled, .. } => {
                assert_eq!(text, "40%");
                assert!(*enabled);
            }
            _ => panic!("a label"),
        }
        let mut got = Settings::default();
        read_all(&s, &mut got);
        assert_eq!(got, want);
        let gray = Settings {
            color_filter: "grayscale".into(),
            ..want
        };
        follow(&mut s, &gray);
        match &s.pages[Page::Vision.index()]
            .item("color_filter_intensity")
            .unwrap()
            .control
        {
            Control::TrackBar(t) => assert!(!t.enabled, "grayscale takes no intensity"),
            _ => panic!("a trackbar"),
        }
    }

    #[test]
    fn the_color_filter_preview_draws_the_pending_filter() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let at = Page::Vision.index();
        let mut want = Settings {
            color_filter: "grayscale".into(),
            ..Settings::default()
        };
        let mut s = with(&want);
        let r = s.on_page(crate::visionpage::preview_rect(Area::MIN));
        let pixel = |s: &PropSheet, x: i32, y: i32| {
            let mut c = Canvas::new(s.size.0 as u32, s.size.1 as u32);
            form::draw(
                &mut c,
                &text,
                &Theme::default().paint(),
                &s.pages[at],
                false,
                px,
            );
            c.get(r.x + x, r.y + y)
        };
        assert_eq!(pixel(&s, 0, 0), 0xff80_8080, "the frame's outer shadow");
        assert_eq!(pixel(&s, 2, 2), 0xffd0_3030, "red as it is");
        assert_eq!(pixel(&s, 2, 16), 0xff52_5252, "red in grayscale");
        assert_eq!(pixel(&s, 2 + 4 * 16, 16), 0xff5e_5e5e, "blue in grayscale");
        want.color_filter = "invert".into();
        follow(&mut s, &want);
        assert_eq!(pixel(&s, 2, 16), 0xff2f_cfcf, "red inverted");
        want.color_filter = "none".into();
        follow(&mut s, &want);
        assert_eq!(pixel(&s, 2, 16), 0xffd0_3030, "no filter: red again");
    }

    #[test]
    fn the_advanced_page_geometry() {
        let s = built();
        let p = Page::Advanced;
        assert!(
            s.pages[p.index()].item("terminal").is_none(),
            "the terminal is picked on Default Programs"
        );
        assert_eq!(rect(&s, p, "task_manager"), Rect::new(20, 60, 358, 20));
        assert_eq!(rect(&s, p, "adaptive_sync"), Rect::new(20, 88, 13, 13));
        assert_eq!(rect(&s, p, "clipboard_history"), Rect::new(20, 110, 13, 13));
        assert_eq!(rect(&s, p, "screenshot_save"), Rect::new(20, 132, 13, 13));
        assert_eq!(rect(&s, p, "recent_files"), Rect::new(20, 154, 13, 13));
        assert_eq!(rect(&s, p, "capture_border"), Rect::new(20, 176, 13, 13));
        assert_eq!(rect(&s, p, PRIVACY_LOCATION), Rect::new(20, 198, 13, 13));
        assert_eq!(locations(Area::MIN), Rect::new(20, 220, 275, 68));
        assert_eq!(rect(&s, p, WATER), Rect::new(20, 344, 13, 13));
        assert_eq!(
            rect(&s, p, WATER_STYLE),
            Rect::new(132, 340, 100, combo::COMBO_H)
        );
        assert_eq!(rect(&s, p, FORGET), Rect::new(303, 220, 75, 23));
        assert!(
            s.pages[p.index()].item("desktop_icons").is_none(),
            "desktop_icons is on Background"
        );
        assert_eq!(rect(&s, p, DEBUG_OVERLAY), Rect::new(20, 300, 13, 13));
        assert_eq!(rect(&s, p, DEBUG_IN_CAPTURES), Rect::new(38, 322, 13, 13));
        assert_eq!(rect(&s, p, THUMBS), Rect::new(20, 392, 13, 13));
        assert_eq!(rect(&s, p, THUMBS_MB), Rect::new(112, 414, 50, 20));
        assert_eq!(rect(&s, p, THUMBS_CLEAR), Rect::new(303, 412, 75, 23));
        assert_eq!(rect(&s, p, "note"), Rect::new(20, 444, 275, 26));
        assert_eq!(rect(&s, p, EDIT_FILE), Rect::new(303, 444, 75, 23));
        for id in ["banner_label", "startmenu_banner", "start_icon"] {
            assert!(s.pages[p.index()].item(id).is_none(), "{id} is on Taskbar");
        }
        inside_page(&s, p);
    }

    #[test]
    fn water_styles_list_the_fluids() {
        let keys: Vec<_> = WATER_STYLES.iter().map(|c| c.0).collect();
        assert_eq!(
            keys,
            ["calm", "lively", "rain", "ink", "glow", "sparks", "neon"]
        );
        let labels: Vec<_> = WATER_STYLES.iter().skip(3).map(|c| c.1).collect();
        assert_eq!(labels, ["Ink", "Glow", "Sparks", "Neon"]);
        for (key, _) in WATER_STYLES {
            assert!(reprisekit::water::Water::parse(key).is_some(), "{key}");
        }
    }

    #[test]
    fn water_note_sits_under_the_style_combo_and_greys_when_off() {
        let mut s = built();
        let p = Page::Advanced;
        let combo = rect(&s, p, WATER_STYLE);
        let (a, b) = (rect(&s, p, "water_note"), rect(&s, p, "water_note_2"));
        assert!(
            a.y >= combo.bottom() && b.y >= a.bottom(),
            "{a:?} {b:?} under {combo:?}"
        );
        let page = &s.pages[p.index()];
        let Control::Label { text, .. } = &page.item("water_note").unwrap().control else {
            panic!("the note is a label");
        };
        assert!(text.starts_with("With Ink, Glow, Sparks or Neon"));
        inside_page(&s, p);
        let mut want = Settings {
            wallpaper_water: "off".into(),
            ..Settings::default()
        };
        follow(&mut s, &want);
        assert!(!enabled_on(&s, p, "water_note") && !enabled_on(&s, p, "water_note_2"));
        want.wallpaper_water = "ink".into();
        follow(&mut s, &want);
        assert!(enabled_on(&s, p, "water_note") && enabled_on(&s, p, "water_note_2"));
    }

    #[test]
    fn the_water_style_greys_while_water_is_off() {
        let mut s = built();
        let mut want = Settings {
            wallpaper_water: "off".into(),
            ..Settings::default()
        };
        follow(&mut s, &want);
        assert!(
            !enabled_on(&s, Page::Advanced, WATER_STYLE),
            "greyed while off"
        );
        want.wallpaper_water = "rain".into();
        follow(&mut s, &want);
        assert!(enabled_on(&s, Page::Advanced, WATER_STYLE));
        write_forms(&mut s, &want);
        let page = &s.pages[Page::Advanced.index()];
        assert_eq!(
            page.combo_selected(WATER_STYLE),
            Some(2),
            "Rain is the third row"
        );
        assert!(matches!(
            page.item(WATER).map(|i| &i.control),
            Some(Control::Check { checked: true, .. })
        ));
        let mut got = Settings::default();
        tick(&mut s, &mut got, Page::Advanced, WATER, false);
        assert_eq!(got.wallpaper_water, "off");
        tick(&mut s, &mut got, Page::Advanced, WATER, true);
        assert_eq!(got.wallpaper_water, "rain");
    }

    fn thumbs_mb(s: &PropSheet) -> &UpDown {
        match &s.pages[Page::Advanced.index()]
            .item(THUMBS_MB)
            .unwrap()
            .control
        {
            Control::UpDown(u) => u,
            _ => panic!("{THUMBS_MB} is an up-down"),
        }
    }

    #[test]
    fn the_thumbnail_cache_row_reads_and_writes_its_keys() {
        let mut got = crate::model::load("[thumbnail_cache]\nenabled = false\nmax_mb = 64\n");
        let mut s = with(&got);
        let p = Page::Advanced;
        assert_eq!(thumbs_mb(&s).value(), 64);
        for id in THUMBS_SIZE {
            assert!(!enabled_on(&s, p, id), "{id} greyed with the cache off");
        }
        let base = got.clone();
        tick(&mut s, &mut got, p, THUMBS, true);
        for id in THUMBS_SIZE {
            assert!(enabled_on(&s, p, id), "{id} lit with the cache on");
        }
        if let Some(Control::UpDown(u)) = s.pages[p.index()]
            .item_mut(THUMBS_MB)
            .map(|i| &mut i.control)
        {
            u.set(128);
        }
        read_one(&s, THUMBS_MB, &mut got);
        let text = "[thumbnail_cache]\nenabled = false\nmax_mb = 64\n";
        let out = crate::model::save(text, &got, &base).expect("parses");
        assert_eq!(
            out, "[thumbnail_cache]\nmax_mb = 128\n",
            "on is the default, so its key goes"
        );
        let back = crate::model::load(&out);
        assert_eq!((back.thumbnail_cache, back.thumbnail_max_mb), (true, 128));
    }

    #[test]
    fn the_thumbnail_cache_size_keeps_to_the_range_the_cache_reads() {
        let mut got = crate::model::load("[thumbnail_cache]\nmax_mb = 9000\n");
        let mut s = with(&got);
        let u = thumbs_mb(&s);
        assert_eq!((u.min, u.max), (1, 4096));
        assert_eq!(u.value(), 4096, "shown clamped");
        assert_eq!(got.thumbnail_max_mb, 9000, "left in the file until touched");
        if let Some(Control::UpDown(u)) = s.pages[Page::Advanced.index()]
            .item_mut(THUMBS_MB)
            .map(|i| &mut i.control)
        {
            u.set(0);
        }
        read_one(&s, THUMBS_MB, &mut got);
        assert_eq!(got.thumbnail_max_mb, 1);
        s = with(&Settings::default());
        assert_eq!(thumbs_mb(&s).value(), 512, "the cache's default");
    }

    #[test]
    fn clear_greys_while_it_runs_then_says_what_went() {
        let mut s = built();
        let page = &mut s.pages[Page::Advanced.index()];
        let clear_lit = |page: &Form| {
            matches!(
                page.item(THUMBS_CLEAR).unwrap().control,
                Control::Button { enabled: true, .. }
            )
        };
        let line = |page: &Form| match &page.item(THUMBS_CLEARED).unwrap().control {
            Control::Label { text, .. } => text.clone(),
            _ => panic!("a label"),
        };
        assert!(clear_lit(page));
        assert_eq!(line(page), "");
        show_clearing(page, Clearing::Running);
        assert!(!clear_lit(page));
        assert_eq!(line(page), "Clearing...");
        let done = |files, bytes| Clearing::Done(Cleared { files, bytes });
        show_clearing(page, done(214, 1_384_448));
        assert!(clear_lit(page));
        assert_eq!(line(page), "Removed 214 files, 1.32 MB.");
        show_clearing(page, done(1, 900));
        assert_eq!(line(page), "Removed 1 file, 900 bytes.");
        show_clearing(page, done(0, 0));
        assert_eq!(line(page), "Nothing to remove.");
        show_clearing(page, Clearing::Failed);
        assert!(clear_lit(page));
        assert_eq!(line(page), "Couldn't clear the thumbnails.");
    }

    /// The Clear row idle, clearing and done, Classic at 100%; every result line fits.
    #[test]
    fn the_clear_row_pixels() {
        if !reprisekit::text::test_tahoma() {
            return;
        }
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let mut page = advanced(Area::MIN);
        let line = page.item(THUMBS_CLEARED).unwrap().rect;
        let widest = Clearing::Done(Cleared {
            files: 99_999,
            bytes: 999 << 30,
        });
        for state in [widest, Clearing::Failed] {
            let line_text = cleared_text(state);
            assert!(
                text.measure(&line_text, false, px) <= line.width,
                "{line_text}"
            );
        }
        let clear = page.item(THUMBS_CLEAR).unwrap().rect;
        let row = Rect::new(line.x, line.y, line.width, clear.bottom() - line.y);
        let hash = |page: &Form| {
            let mut c = Canvas::new(row.right() as u32 + 8, row.bottom() as u32 + 8);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            form::draw(&mut c, &text, &theme.paint(), page, false, px);
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for y in row.y..row.bottom() {
                for x in row.x..row.right() {
                    h = (h ^ u64::from(c.get(x, y))).wrapping_mul(0x100_0000_01b3);
                }
            }
            h
        };
        let idle = hash(&page);
        show_clearing(&mut page, Clearing::Running);
        let clearing = hash(&page);
        show_clearing(
            &mut page,
            Clearing::Done(Cleared {
                files: 214,
                bytes: 1_384_448,
            }),
        );
        let done = hash(&page);
        assert_eq!(
            [idle, clearing, done],
            [
                7_897_640_068_597_891_350,
                2_251_566_719_493_853_932,
                14_478_241_142_069_230_046
            ],
            "idle, clearing, done"
        );
    }

    #[test]
    fn the_debug_overlay_checks() {
        let mut got = Settings::default();
        let mut s = with(&got);
        let p = Page::Advanced;
        assert!(
            !enabled_on(&s, p, DEBUG_IN_CAPTURES),
            "greyed with the overlay off"
        );
        tick(&mut s, &mut got, p, DEBUG_OVERLAY, true);
        assert!(got.debug_overlay);
        assert!(enabled_on(&s, p, DEBUG_IN_CAPTURES));
        tick(&mut s, &mut got, p, DEBUG_IN_CAPTURES, true);
        assert!(got.debug_overlay_in_captures);
    }

    #[test]
    fn the_location_check_and_list() {
        let mut s = built();
        let mut got = Settings::default();
        assert!(got.location, "programs may ask by default");
        tick(&mut s, &mut got, Page::Advanced, PRIVACY_LOCATION, false);
        assert!(!got.location);
        let mut lv = new_location_list();
        fit_location_columns(
            &mut lv,
            &reprisekit::controls::metrics::ControlMetrics::CLASSIC,
            locations(Area::MIN),
        );
        let widths: Vec<i32> = lv.columns.iter().map(|c| c.width).collect();
        assert_eq!(widths, [185, 70]);
        fill_locations(
            &mut lv,
            &[("Maps".into(), true), ("org.example.Weather".into(), false)],
        );
        let cells: Vec<&[String]> = lv.rows.iter().map(|r| r.cells.as_slice()).collect();
        assert_eq!(
            cells,
            [
                ["Maps", "Allowed"].map(String::from).as_slice(),
                ["org.example.Weather", "Denied"]
                    .map(String::from)
                    .as_slice(),
            ]
        );
    }

    #[test]
    fn the_background_page_geometry() {
        let s = built();
        let p = Page::Background;
        assert_eq!(
            rect(&s, p, backgroundpage::MONITOR),
            Rect::new(20, 44, 191, 168)
        );
        assert_eq!(rect(&s, p, "wallpaper_label"), Rect::new(219, 44, 66, 13));
        assert_eq!(
            rect(&s, p, backgroundpage::MODE),
            Rect::new(289, 40, 89, 21)
        );
        assert_eq!(
            rect(&s, p, backgroundpage::WALLPAPER),
            Rect::new(219, 66, 159, 191)
        );
        assert_eq!(
            rect(&s, p, backgroundpage::BROWSE),
            Rect::new(303, 264, 75, 23)
        );
        assert_eq!(rect(&s, p, "slideshow_group"), Rect::new(20, 393, 358, 70));
        assert_eq!(
            rect(&s, p, backgroundpage::INTERVAL),
            Rect::new(166, 413, 120, 21)
        );
        assert_eq!(rect(&s, p, "slideshow_shuffle"), Rect::new(32, 439, 13, 13));
        assert_eq!(rect(&s, p, "position_label"), Rect::new(219, 310, 75, 13));
        assert_eq!(
            rect(&s, p, "wallpaper_position"),
            Rect::new(219, 326, 75, 21)
        );
        assert_eq!(rect(&s, p, "color_label"), Rect::new(303, 310, 75, 13));
        assert_eq!(rect(&s, p, "background"), Rect::new(303, 326, 75, 21));
        assert_eq!(rect(&s, p, "icons_group"), Rect::new(20, 222, 191, 160));
        assert_eq!(rect(&s, p, "extensions_label"), Rect::new(32, 334, 167, 13));
        assert_eq!(
            rect(&s, p, backgroundpage::EXTENSIONS),
            Rect::new(32, 350, 167, 21)
        );
        for (i, id) in ["documents", "computer", "home", "desktop_icons"]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                rect(&s, p, id),
                Rect::new(32, 244 + 22 * i as i32, 13, 13),
                "{id}"
            );
        }
        inside_page(&s, p);
    }

    #[test]
    fn unticking_an_icon_keeps_the_recycle_bin() {
        let p = Page::Background;
        let mut got = Settings::default();
        let mut s = with(&got);
        tick(&mut s, &mut got, p, "home", false);
        assert_eq!(got.system_icons, ["documents", "computer", "recycle-bin"]);
        tick(&mut s, &mut got, p, "documents", false);
        tick(&mut s, &mut got, p, "home", true);
        assert_eq!(
            got.system_icons,
            ["computer", "recycle-bin", "home"],
            "a ticked icon goes at the end"
        );
    }

    #[test]
    fn the_position_greys_without_a_wallpaper() {
        let p = Page::Background;
        let s = built();
        assert!(!enabled_on(&s, p, "wallpaper_position") && !enabled_on(&s, p, "position_label"));
        let s = with(&Settings {
            wallpaper: "/w/a.png".into(),
            ..Settings::default()
        });
        assert!(enabled_on(&s, p, "wallpaper_position") && enabled_on(&s, p, "position_label"));
    }

    #[test]
    fn the_extensions_combo_writes_its_key_and_greys_without_the_files() {
        let p = Page::Background;
        let mut got = Settings::default();
        let mut s = with(&got);
        assert!(enabled_on(&s, p, backgroundpage::EXTENSIONS));
        pick(&mut s, &mut got, p, backgroundpage::EXTENSIONS, 1);
        assert_eq!(got.desktop_extensions, "hide_launchers");
        pick(&mut s, &mut got, p, backgroundpage::EXTENSIONS, 2);
        assert_eq!(got.desktop_extensions, "show");
        tick(&mut s, &mut got, p, "desktop_icons", false);
        assert!(
            !enabled_on(&s, p, backgroundpage::EXTENSIONS)
                && !enabled_on(&s, p, "extensions_label")
        );
        tick(&mut s, &mut got, p, "desktop_icons", true);
        assert!(enabled_on(&s, p, backgroundpage::EXTENSIONS));
    }

    /// The Color button shows the desktop colour the interface and scheme give until one is picked, and follows a scheme picked meanwhile.
    #[test]
    fn the_colour_button_shows_the_desktop_the_look_gives() {
        let colour = |s: &PropSheet| match &s.pages[Page::Background.index()]
            .item("background")
            .unwrap()
            .control
        {
            Control::Color(b) => b.color,
            _ => panic!("background is a colour button"),
        };
        let mut got = Settings::default();
        let mut s = with(&got);
        assert_eq!(colour(&s), Theme::default().colors.background);
        let brick = rows(&s, Page::Appearance, appearancepage::CLASSIC_SCHEME)
            .0
            .iter()
            .position(|n| n == "Brick")
            .unwrap();
        pick(
            &mut s,
            &mut got,
            Page::Appearance,
            appearancepage::CLASSIC_SCHEME,
            brick,
        );
        assert_eq!(
            colour(&s),
            reprisekit::schemes::scheme("Brick").unwrap().background
        );
        let s = with(&Settings {
            background: "#336699".into(),
            ..Settings::default()
        });
        assert_eq!(colour(&s), Color::rgb(0x33, 0x66, 0x99));
    }

    #[test]
    fn the_monitor_shows_the_colour_then_the_picture() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let at = Page::Background.index();
        let mut want = Settings {
            background: "#102030".into(),
            ..Settings::default()
        };
        let mut s = with(&want);
        let r = s.on_page(backgroundpage::MONITOR_RECT);
        let (sx, sy, _, _) = reprisekit::classic::assets::MONITOR_SCREEN;
        let at_screen = |s: &PropSheet, x: i32, y: i32| {
            let mut c = Canvas::new(s.size.0 as u32, s.size.1 as u32);
            form::draw(
                &mut c,
                &text,
                &Theme::default().paint(),
                &s.pages[at],
                false,
                px,
            );
            c.get(r.x + sx + x, r.y + sy + y)
        };
        let bg = 0xFF10_2030;
        backgroundpage::show(&mut s.pages[at], &want, "", None);
        assert_eq!(at_screen(&s, 76, 56), bg, "no wallpaper: the colour");
        // An 8 px red picture from a 960 px wide file on a 1920 px screen is centred at half the screen's width: 76 px of the 152.
        want.wallpaper = "/w/red.png".into();
        want.wallpaper_position = "center".into();
        let mut red = Canvas::new(8, 8);
        red.fill_rect(red.bounds(), Color::rgb(255, 0, 0));
        let pic = backgroundpage::Picture {
            path: "/w/red.png".into(),
            canvas: Some(red),
            source_w: 960,
        };
        backgroundpage::show(&mut s.pages[at], &want, "/w/red.png", Some(&pic));
        assert_eq!(
            at_screen(&s, 76, 56),
            0xFFFF_0000,
            "the middle of the screen"
        );
        assert_eq!(
            at_screen(&s, 2, 2),
            bg,
            "the colour round a centred picture"
        );
        want.wallpaper = "/w/blue.png".into();
        backgroundpage::show(&mut s.pages[at], &want, "/w/blue.png", Some(&pic));
        assert_eq!(at_screen(&s, 76, 56), bg);
    }

    #[test]
    fn the_fonts_and_icons_page_geometry() {
        let s = built();
        assert_eq!(
            rect(&s, Page::FontsAndIcons, "fonts_group"),
            Rect::new(20, 44, 358, 84)
        );
        assert_eq!(
            rect(&s, Page::FontsAndIcons, "font_family"),
            Rect::new(32, 78, 163, 20)
        );
        assert_eq!(
            rect(&s, Page::FontsAndIcons, "font_shell_dlg"),
            Rect::new(203, 78, 163, 20)
        );
        assert_eq!(
            rect(&s, Page::FontsAndIcons, "font_size_px"),
            Rect::new(80, 104, 50, 20)
        );
        assert_eq!(
            rect(&s, Page::FontsAndIcons, "font_antialias"),
            Rect::new(192, 108, 13, 13)
        );
        assert_eq!(
            rect(&s, Page::FontsAndIcons, "icons_group"),
            Rect::new(20, 138, 358, 56)
        );
        assert_eq!(
            rect(&s, Page::FontsAndIcons, "icon_theme"),
            Rect::new(92, 158, 274, 21)
        );
        assert_eq!(
            rect(&s, Page::FontsAndIcons, "windows_group"),
            Rect::new(20, 204, 358, 92)
        );
        assert_eq!(
            rect(&s, Page::FontsAndIcons, "taskbar_height"),
            Rect::new(146, 274, 50, 20)
        );
        assert_eq!(
            rect(&s, Page::FontsAndIcons, "toolkits_group"),
            Rect::new(20, 306, 358, 114)
        );
        assert_eq!(
            rect(&s, Page::FontsAndIcons, "gtk_theme"),
            Rect::new(120, 326, 246, 21)
        );
        assert_eq!(
            rect(&s, Page::FontsAndIcons, "qt_platform_theme"),
            Rect::new(120, 352, 246, 21)
        );
        assert_eq!(
            rect(&s, Page::FontsAndIcons, "smoothing_label"),
            Rect::new(32, 382, 84, 13)
        );
        assert_eq!(
            rect(&s, Page::FontsAndIcons, "font_smoothing"),
            Rect::new(120, 378, 246, 21)
        );
        assert_eq!(
            rect(&s, Page::FontsAndIcons, "gtk4_colors"),
            Rect::new(32, 404, 13, 13)
        );
        let fi = &s.pages[Page::FontsAndIcons.index()];
        assert!(
            fi.item("menus_group").is_none(),
            "the Menus group is on Effects"
        );
        inside_page(&s, Page::FontsAndIcons);
    }

    #[test]
    fn the_keyboard_page_geometry() {
        let s = built();
        assert_eq!(
            rect(&s, Page::Keyboard, "keyboard_group"),
            Rect::new(20, 44, 358, 190)
        );
        assert_eq!(
            rect(&s, Page::Keyboard, KB_LAYOUTS),
            Rect::new(116, 62, 169, 72)
        );
        assert_eq!(rect(&s, Page::Keyboard, KB_ADD), Rect::new(291, 62, 75, 23));
        assert_eq!(
            rect(&s, Page::Keyboard, KB_REMOVE),
            Rect::new(291, 89, 75, 23)
        );
        assert_eq!(rect(&s, Page::Keyboard, KB_UP), Rect::new(291, 116, 75, 23));
        assert_eq!(
            rect(&s, Page::Keyboard, "kb_model"),
            Rect::new(116, 146, 250, 20)
        );
        assert_eq!(
            rect(&s, Page::Keyboard, "kb_options"),
            Rect::new(116, 172, 250, 20)
        );
        assert_eq!(
            rect(&s, Page::Keyboard, "repeat_rate"),
            Rect::new(116, 198, 50, 20)
        );
        assert_eq!(
            rect(&s, Page::Keyboard, "repeat_delay"),
            Rect::new(250, 198, 50, 20)
        );
        assert_eq!(
            rect(&s, Page::Keyboard, "shortcuts_group"),
            Rect::new(20, 242, 358, 138)
        );
        assert_eq!(shortcuts(Area::MIN), Rect::new(32, 260, 334, 92));
        assert_eq!(
            rect(&s, Page::Keyboard, SC_ADD),
            Rect::new(129, 356, 75, 23)
        );
        assert_eq!(
            rect(&s, Page::Keyboard, SC_EDIT),
            Rect::new(210, 356, 75, 23)
        );
        assert_eq!(
            rect(&s, Page::Keyboard, SC_REMOVE),
            Rect::new(291, 356, 75, 23)
        );
        inside_page(&s, Page::Keyboard);

        let lv = new_shortcut_list();
        let lay = listview::layout(
            &lv,
            shortcuts(Area::MIN),
            &reprisekit::controls::metrics::ControlMetrics::CLASSIC,
        );
        assert_eq!(lay.rows, 4, "four rows of 17 px under the header");
        // The default list is longer than four rows, so the vertical bar is up; at 16 or 17 px the columns leave it room and no horizontal one appears.
        for (scroll, action) in [(16, 194), (17, 193)] {
            let cm = reprisekit::controls::metrics::ControlMetrics {
                scroll,
                ..reprisekit::controls::metrics::ControlMetrics::CLASSIC
            };
            let mut full = new_shortcut_list();
            fill_shortcuts(&mut full, &Settings::default(), &|a| a.into());
            fit_shortcut_columns(&mut full, &cm, shortcuts(Area::MIN));
            assert_eq!(full.columns[1].width, action);
            let lay = listview::layout(&full, shortcuts(Area::MIN), &cm);
            assert!(lay.scrollbar.is_some());
            assert!(lay.hscroll.is_none(), "no horizontal bar at {scroll}");
            assert_eq!(lay.rows, 4);
        }
    }

    #[test]
    fn the_layouts_list_names_each_layout_and_its_buttons_follow_it() {
        let two = Settings {
            kb_layout: "us,hu".into(),
            ..Settings::default()
        };
        let mut s = with(&two);
        let rows = |s: &PropSheet| match &s.pages[Page::Keyboard.index()]
            .item(KB_LAYOUTS)
            .unwrap()
            .control
        {
            Control::ListBox { list, .. } => list.items.clone(),
            _ => panic!("a list box"),
        };
        let enabled = |s: &PropSheet, id: &str| match &s.pages[Page::Keyboard.index()]
            .item(id)
            .unwrap()
            .control
        {
            Control::Button { enabled, .. } => *enabled,
            _ => panic!("{id} is a button"),
        };
        assert_eq!(rows(&s), vec!["English (US)", "Hungarian"]);
        assert!(enabled(&s, KB_ADD));
        assert!(!enabled(&s, KB_REMOVE), "nothing picked");
        assert!(!enabled(&s, KB_UP));
        fill_layouts(&mut s, &two, &reg(), Some(1));
        assert!(enabled(&s, KB_REMOVE));
        assert!(enabled(&s, KB_UP));
        s.pages[Page::Keyboard.index()].focus_id(KB_UP);
        fill_layouts(&mut s, &two, &reg(), Some(0));
        assert!(!enabled(&s, KB_UP), "the first cannot move up");
        assert_eq!(
            s.pages[Page::Keyboard.index()].focus(),
            s.pages[Page::Keyboard.index()].index_of(KB_LAYOUTS),
            "the focus leaves the greyed button for the list"
        );
        let four = Settings {
            kb_layout: "us,hu,de,fr".into(),
            ..Settings::default()
        };
        fill_layouts(&mut s, &four, &reg(), None);
        assert!(!enabled(&s, KB_ADD), "four is the most");
        assert_eq!(rows(&s)[2], "de", "an unknown code shows as itself");
    }

    #[test]
    fn the_shortcut_list_shows_keys_and_action_with_the_spawn_command() {
        let mut lv = new_shortcut_list();
        let s = Settings {
            shortcuts: vec![
                Shortcut::new("Super+Return", "terminal", ""),
                Shortcut::new("Super+t", "spawn", "foot"),
            ],
            ..Settings::default()
        };
        fill_shortcuts(&mut lv, &s, &|a| a.into());
        assert_eq!(lv.rows.len(), 2);
        assert_eq!(lv.rows[0].cells, vec!["Super+Return", "terminal"]);
        assert_eq!(lv.rows[1].cells, vec!["Super+t", "spawn foot"]);
        assert!(lv.checks.is_none());
    }

    #[test]
    fn program_rows_follow_the_binds() {
        let s = crate::model::load(
            "[[global_shortcuts]]\napp = \"t\"\nid = \"talk\"\ndescription = \"Hold to talk\"\nkeys = \"\"\n[portal]\nshortcuts_denied = [\"n\"]\n",
        );
        let mut l = new_shortcut_list();
        fill_shortcuts(&mut l, &s, &|a| {
            if a == "t" { "Taltos".into() } else { a.into() }
        });
        let n = s.shortcuts.len();
        assert_eq!(l.rows[n].cells, ["None", "Taltos: Hold to talk"]);
        assert_eq!(l.rows[n + 1].cells, ["", "n: not allowed"]);
    }

    /// The Motion group, twice: a "Select a pointer speed:" label, Slow and Fast
    /// around the trackbar, and "Enhance pointer precision" under it (label 8 px
    /// under the title, trackbar row 15 px under that, check 28 px under the
    /// trackbar's top), and each group's "Reverse scrolling direction" check 22 px
    /// under the last row.
    #[test]
    fn the_mouse_page_geometry() {
        let s = built();
        assert_eq!(
            rect(&s, Page::Mouse, "mouse_group"),
            Rect::new(20, 44, 358, 152)
        );
        assert_eq!(
            rect(&s, Page::Mouse, "double_click_ms"),
            Rect::new(166, 64, 50, 20)
        );
        assert_eq!(
            rect(&s, Page::Mouse, "mouse_speed_label"),
            Rect::new(32, 94, 334, 13)
        );
        assert_eq!(
            rect(&s, Page::Mouse, "mouse_slow"),
            Rect::new(32, 115, 26, 13)
        );
        assert_eq!(
            rect(&s, Page::Mouse, "mouse_speed"),
            Rect::new(60, 109, 160, trackbar::HEIGHT)
        );
        assert_eq!(
            rect(&s, Page::Mouse, "mouse_fast"),
            Rect::new(224, 115, 26, 13)
        );
        assert_eq!(
            rect(&s, Page::Mouse, "mouse_accel"),
            Rect::new(32, 143, 13, 13)
        );
        assert_eq!(
            rect(&s, Page::Mouse, "mouse_natural"),
            Rect::new(32, 165, 13, 13)
        );

        assert_eq!(
            rect(&s, Page::Mouse, "touchpad_group"),
            Rect::new(20, 206, 358, 218)
        );
        assert_eq!(
            rect(&s, Page::Mouse, "touchpad_speed"),
            Rect::new(60, 245, 160, trackbar::HEIGHT)
        );
        assert_eq!(
            rect(&s, Page::Mouse, "touchpad_accel"),
            Rect::new(32, 279, 13, 13)
        );
        assert_eq!(
            rect(&s, Page::Mouse, "tap_to_click"),
            Rect::new(32, 305, 13, 13)
        );
        assert_eq!(
            rect(&s, Page::Mouse, "two_finger_scroll"),
            Rect::new(32, 327, 13, 13)
        );
        assert_eq!(
            rect(&s, Page::Mouse, "two_finger_tap_right_click"),
            Rect::new(32, 349, 13, 13)
        );
        assert_eq!(
            rect(&s, Page::Mouse, "disable_while_typing"),
            Rect::new(32, 371, 13, 13)
        );
        assert_eq!(
            rect(&s, Page::Mouse, "touchpad_natural"),
            Rect::new(32, 393, 13, 13)
        );
        inside_page(&s, Page::Mouse);
    }

    #[test]
    fn the_speed_trackbars_run_from_minus_one_to_one_in_tenths() {
        let want = Settings {
            mouse_speed: -10,
            touchpad_speed: 3,
            ..Settings::default()
        };
        let s = with(&want);
        let pos = |id: &str| match &s.pages[Page::Mouse.index()].item(id).unwrap().control {
            Control::TrackBar(t) => (t.pos, t.count),
            _ => panic!("{id} is a trackbar"),
        };
        assert_eq!(pos("mouse_speed"), (0, 21));
        assert_eq!(pos("touchpad_speed"), (13, 21));
        let mut got = Settings::default();
        read_all(&s, &mut got);
        assert_eq!(got, want);
    }

    #[test]
    fn two_finger_tap_follows_tap_to_click() {
        let mut s = with(&Settings::default());
        let enabled = |s: &PropSheet| match &s.pages[Page::Mouse.index()]
            .item("two_finger_tap_right_click")
            .unwrap()
            .control
        {
            Control::Check { enabled, .. } => *enabled,
            _ => panic!("a check"),
        };
        assert!(enabled(&s));
        let off = Settings {
            tap_to_click: false,
            ..Settings::default()
        };
        write(&mut s, &off, &reg());
        assert!(!enabled(&s));
    }

    #[test]
    fn the_power_page_geometry() {
        let s = built();
        let p = Page::Power;
        assert_eq!(rect(&s, p, "turnoff_group"), Rect::new(20, 44, 358, 126));
        assert_eq!(rect(&s, p, "standby_label"), Rect::new(32, 66, 70, 13));
        assert_eq!(rect(&s, p, "standby"), Rect::new(106, 62, 260, 20));
        assert_eq!(rect(&s, p, "poweroff"), Rect::new(106, 88, 260, 20));
        assert_eq!(rect(&s, p, "reboot"), Rect::new(106, 114, 260, 20));
        assert_eq!(rect(&s, p, "hibernate"), Rect::new(106, 140, 260, 20));
        assert_eq!(rect(&s, p, "idle_group"), Rect::new(20, 180, 358, 196));
        assert_eq!(rect(&s, p, "lock_after"), Rect::new(150, 200, 50, 20));
        assert_eq!(rect(&s, p, "blank_after"), Rect::new(150, 252, 50, 20));
        assert_eq!(rect(&s, p, "lock_fade"), Rect::new(150, 278, 50, 20));
        assert_eq!(rect(&s, p, "idle_note"), Rect::new(32, 308, 334, 13));
        assert_eq!(rect(&s, p, "lock_on_sleep"), Rect::new(32, 330, 13, 13));
        assert_eq!(rect(&s, p, "lock_wallpaper"), Rect::new(32, 352, 13, 13));
        inside_page(&s, p);
    }

    #[test]
    fn the_switcher_page_geometry() {
        let s = built();
        let p = Page::Switcher;
        assert_eq!(rect(&s, p, "switcher_group"), Rect::new(20, 44, 358, 168));
        assert_eq!(rect(&s, p, "switcher_style"), Rect::new(146, 64, 130, 21));
        assert_eq!(rect(&s, p, "switcher_size"), Rect::new(146, 90, 130, 21));
        assert_eq!(rect(&s, p, "switcher_preview"), Rect::new(32, 118, 13, 13));
        assert_eq!(rect(&s, p, "switcher_desktop"), Rect::new(32, 140, 13, 13));
        assert_eq!(rect(&s, p, "switcher_peek"), Rect::new(32, 162, 13, 13));
        assert_eq!(rect(&s, p, "switcher_flip"), Rect::new(32, 184, 13, 13));
        assert_eq!(rect(&s, p, "overview_group"), Rect::new(20, 224, 358, 76));
        assert_eq!(rect(&s, p, "hot_corner"), Rect::new(146, 268, 130, 21));
        inside_page(&s, p);
    }

    #[test]
    fn switcher_values_go_in_and_come_back_out() {
        let want = Settings {
            switcher_style: "thumbnails".into(),
            switcher_size: "large".into(),
            switcher_preview: false,
            switcher_desktop: true,
            switcher_peek: true,
            switcher_flip: true,
            hot_corner: "bottom-right".into(),
            ..Settings::default()
        };
        let s = with(&want);
        let page = &s.pages[Page::Switcher.index()];
        assert_eq!(page.checked("switcher_desktop"), Some(true));
        assert_eq!(page.checked("switcher_peek"), Some(true));
        assert_eq!(page.checked("switcher_flip"), Some(true));
        let mut got = Settings::default();
        read_all(&s, &mut got);
        assert_eq!(got, want);
    }

    #[test]
    fn the_desktops_page_geometry() {
        let s = built();
        let p = Page::Desktops;
        assert_eq!(rect(&s, p, "desktops_group"), Rect::new(20, 44, 358, 168));
        assert_eq!(rect(&s, p, "use_desktops"), Rect::new(32, 64, 13, 13));
        assert_eq!(rect(&s, p, "desktops_label"), Rect::new(50, 88, 206, 13));
        assert_eq!(rect(&s, p, "desktops"), Rect::new(260, 84, 50, 20));
        assert_eq!(
            rect(&s, p, "desktop_animation"),
            Rect::new(164, 110, 130, 21)
        );
        assert_eq!(
            rect(&s, p, "desktop_animation_ms"),
            Rect::new(164, 136, 50, 20)
        );
        assert_eq!(rect(&s, p, "duration_units"), Rect::new(220, 140, 60, 13));
        assert_eq!(rect(&s, p, "pager_mark_label"), Rect::new(50, 166, 190, 13));
        assert_eq!(rect(&s, p, "pager_mark"), Rect::new(50, 184, 134, 21));
        assert_eq!(rect(&s, p, "previews_group"), Rect::new(20, 222, 358, 76));
        assert_eq!(rect(&s, p, "preview_taskbar"), Rect::new(32, 246, 13, 13));
        assert_eq!(rect(&s, p, "previews_fps"), Rect::new(146, 268, 50, 20));
        assert_eq!(rect(&s, p, "fps_units"), Rect::new(202, 272, 100, 13));
        for id in ["session_restore", "session_end_processes"] {
            assert!(s.pages[p.index()].item(id).is_none(), "{id} is on Startup");
        }
        inside_page(&s, p);
    }

    fn enabled_on(s: &PropSheet, page: Page, id: &str) -> bool {
        match &s.pages[page.index()].item(id).unwrap().control {
            Control::Check { enabled, .. }
            | Control::Label { enabled, .. }
            | Control::Edit { enabled, .. } => *enabled,
            Control::Combo(c) => c.enabled,
            Control::UpDown(u) => u.enabled,
            Control::TrackBar(t) => t.enabled,
            _ => panic!("{id} has no greyed state here"),
        }
    }

    #[test]
    fn the_effects_page_geometry() {
        let s = built();
        let p = Page::Effects;
        assert_eq!(p.label(), "Effects");
        assert_eq!(rect(&s, p, "window_animation"), Rect::new(GX, 66, 13, 13));
        assert_eq!(rect(&s, p, "open_label"), Rect::new(50, 92, 70, 13));
        assert_eq!(rect(&s, p, "window_open_ms"), Rect::new(124, 88, 50, 20));
        assert_eq!(rect(&s, p, "close_label"), Rect::new(190, 92, 70, 13));
        assert_eq!(rect(&s, p, "window_close_ms"), Rect::new(264, 88, 50, 20));
        assert_eq!(rect(&s, p, "hover_fade"), Rect::new(GX, 122, 13, 13));
        assert_eq!(rect(&s, p, "in_label"), Rect::new(50, 148, 70, 13));
        assert_eq!(rect(&s, p, "hover_in_ms"), Rect::new(124, 144, 50, 20));
        assert_eq!(rect(&s, p, "out_label"), Rect::new(190, 148, 70, 13));
        assert_eq!(rect(&s, p, "hover_out_ms"), Rect::new(264, 144, 50, 20));
        assert_eq!(rect(&s, p, "session_fade"), Rect::new(GX, 178, 13, 13));
        assert_eq!(rect(&s, p, "menus_group"), Rect::new(20, 212, 358, 74));
        assert_eq!(rect(&s, p, "menu_shadow"), Rect::new(96, 230, 50, 20));
        assert_eq!(rect(&s, p, "menu_hover_ms"), Rect::new(246, 230, 50, 20));
        assert_eq!(rect(&s, p, "menu_fade_ms"), Rect::new(96, 256, 50, 20));
        assert_eq!(rect(&s, p, "taskbar_group"), Rect::new(20, 296, 358, 48));
        assert_eq!(rect(&s, p, "taskbar_slide"), Rect::new(GX, 318, 13, 13));
        inside_page(&s, p);
    }

    #[test]
    fn the_menu_timings_go_in_and_come_back_out_on_effects() {
        let want = Settings {
            menu_shadow: 4,
            menu_hover_ms: 250,
            menu_fade_ms: 0,
            ..Settings::default()
        };
        let s = with(&want);
        let fx = &s.pages[Page::Effects.index()];
        let value = |id: &str| match &fx
            .item(id)
            .unwrap_or_else(|| panic!("{id} is on Effects"))
            .control
        {
            Control::UpDown(u) => u.value(),
            _ => panic!("{id} is an up-down"),
        };
        assert_eq!(
            (
                value("menu_shadow"),
                value("menu_hover_ms"),
                value("menu_fade_ms")
            ),
            (4, 250, 0)
        );
        let mut got = Settings::default();
        read_all(&s, &mut got);
        assert_eq!(got, want);
    }

    fn set_check(s: &mut PropSheet, got: &mut Settings, id: &str, on: bool) {
        if let Some(Control::Check { checked, .. }) = s.pages[Page::Effects.index()]
            .item_mut(id)
            .map(|i| &mut i.control)
        {
            *checked = on;
        }
        read_one(s, id, got);
        follow(s, got);
    }

    #[test]
    fn the_effect_spins_grey_with_their_check() {
        let mut got = Settings::for_interface("glass");
        let mut s = with(&got);
        let p = Page::Effects;
        assert!(enabled_on(&s, p, "window_open_ms") && enabled_on(&s, p, "hover_out_ms"));
        set_check(&mut s, &mut got, "window_animation", false);
        assert!(!enabled_on(&s, p, "window_open_ms") && !enabled_on(&s, p, "window_close_ms"));
        assert!(
            enabled_on(&s, p, "hover_in_ms"),
            "the other group keeps going"
        );
        set_check(&mut s, &mut got, "hover_fade", false);
        assert!(!enabled_on(&s, p, "hover_in_ms") && !enabled_on(&s, p, "hover_out_ms"));
    }

    #[test]
    fn the_taskbar_slide_check_goes_in_and_comes_back_out() {
        let mut got = Settings::for_interface("glass");
        let mut s = with(&got);
        let checked = |s: &PropSheet| match &s.pages[Page::Effects.index()]
            .item("taskbar_slide")
            .unwrap()
            .control
        {
            Control::Check { checked, .. } => *checked,
            _ => panic!("taskbar_slide is a check"),
        };
        assert!(checked(&s), "Glass slides its task buttons");
        set_check(&mut s, &mut got, "taskbar_slide", false);
        assert!(!got.taskbar_slide);
        assert!(!checked(&with(&Settings::for_interface("classic"))));
    }

    /// One desktop unticks "Use virtual desktops" and greys the count, animation
    /// and duration; the count shows the default so a tick starts from it.
    /// Ticking writes the count back, unticking writes 1.
    #[test]
    fn use_virtual_desktops_sets_one_and_greys_the_rest() {
        let one = Settings {
            desktops: 1,
            desktop_animation: "slide".into(),
            ..Settings::default()
        };
        let mut s = with(&one);
        let p = Page::Desktops;
        assert_eq!(s.pages[p.index()].checked("use_desktops"), Some(false));
        for id in [
            "desktops",
            "desktops_label",
            "desktop_animation",
            "desktop_animation_ms",
        ] {
            assert!(!enabled_on(&s, p, id), "{id} greys with one desktop");
        }
        match &s.pages[p.index()].item("desktops").unwrap().control {
            Control::UpDown(u) => assert_eq!((u.value(), u.min, u.max), (4, 2, 16)),
            _ => panic!("an up-down"),
        }

        let mut got = one.clone();
        if let Some(Control::Check { checked, .. }) = s.pages[p.index()]
            .item_mut("use_desktops")
            .map(|i| &mut i.control)
        {
            *checked = true;
        }
        read_one(&s, "use_desktops", &mut got);
        assert_eq!(got.desktops, 4, "ticked: the count the up-down shows");
        follow(&mut s, &got);
        for id in ["desktops", "desktop_animation", "desktop_animation_ms"] {
            assert!(enabled_on(&s, p, id), "{id} comes back");
        }

        if let Some(Control::Check { checked, .. }) = s.pages[p.index()]
            .item_mut("use_desktops")
            .map(|i| &mut i.control)
        {
            *checked = false;
        }
        read_one(&s, "use_desktops", &mut got);
        assert_eq!(got.desktops, 1, "unticked: one desktop");
    }

    #[test]
    fn the_duration_and_the_refresh_rate_follow_what_uses_them() {
        let p = Page::Desktops;
        let still = Settings {
            switcher_preview: false,
            preview_taskbar: false,
            ..Settings::default()
        };
        let mut s = with(&still);
        assert!(enabled_on(&s, p, "desktop_animation"));
        assert!(!enabled_on(&s, p, "desktop_animation_ms"), "no animation");
        assert!(!enabled_on(&s, p, "duration_units"));
        for id in FPS_CONTROLS {
            assert!(!enabled_on(&s, p, id), "{id} greys with no live picture");
        }
        let thumbs = Settings {
            switcher_style: "thumbnails".into(),
            desktop_animation: "fade".into(),
            ..still.clone()
        };
        follow(&mut s, &thumbs);
        assert!(enabled_on(&s, p, "previews_fps"), "thumbnails are live");
        assert!(enabled_on(&s, p, "desktop_animation_ms"));
        let taskbar = Settings {
            preview_taskbar: true,
            ..still
        };
        follow(&mut s, &taskbar);
        assert!(enabled_on(&s, p, "previews_fps"));
    }

    /// The combos show words, and the config's values go in and come back out through them.
    #[test]
    fn named_combos_show_labels_and_carry_values() {
        let want = Settings {
            switcher_style: "thumbnails".into(),
            desktop_animation: "slide-vertical".into(),
            ..Settings::default()
        };
        let s = with(&want);
        let shown = |page: Page, id: &str| match &s.pages[page.index()].item(id).unwrap().control {
            Control::Combo(c) => c.items[c.selected].text.clone(),
            _ => panic!("{id} is a combo"),
        };
        assert_eq!(shown(Page::Switcher, "switcher_style"), "Thumbnails");
        assert_eq!(
            shown(Page::Desktops, "desktop_animation"),
            "Slide vertically"
        );
        let mut got = Settings::default();
        read_all(&s, &mut got);
        assert_eq!(got, want);

        let odd = Settings {
            desktop_animation: "spin".into(),
            ..Settings::default()
        };
        let s = with(&odd);
        let mut got = Settings::default();
        read_all(&s, &mut got);
        assert_eq!(got.desktop_animation, "spin", "an unknown value survives");
    }

    #[test]
    fn the_pager_mark_greys_outside_glass() {
        let p = Page::Desktops;
        let with_interface = |interface: &str, desktops: i64| Settings {
            interface: interface.into(),
            desktops,
            ..Settings::default()
        };
        let mut s = with(&with_interface("classic", 4));
        for id in ["pager_mark_label", "pager_mark"] {
            assert!(!enabled_on(&s, p, id), "{id} greys outside Glass");
        }
        follow(&mut s, &with_interface("glass", 4));
        for id in ["pager_mark_label", "pager_mark"] {
            assert!(enabled_on(&s, p, id), "{id} enables on Glass");
        }
        follow(&mut s, &with_interface("glass", 1));
        for id in ["pager_mark_label", "pager_mark"] {
            assert!(!enabled_on(&s, p, id), "{id} greys with one desktop");
        }
    }

    #[test]
    fn the_snap_bar_and_distance_grey_with_drag_snapping_off() {
        let p = Page::Windows;
        let off = Settings {
            snap_enabled: false,
            ..Settings::default()
        };
        let mut s = with(&off);
        for id in DRAG_SNAP_CONTROLS {
            assert!(!enabled_on(&s, p, id), "{id} greys: it needs drags");
        }
        assert_eq!(s.pages[p.index()].checked("snap_bar"), Some(true));
        follow(&mut s, &Settings::default());
        for id in DRAG_SNAP_CONTROLS {
            assert!(enabled_on(&s, p, id), "{id} follows drag snapping back on");
        }
    }

    #[test]
    fn the_flip_check_greys_outside_glass() {
        let p = Page::Switcher;
        let on = |interface: &str| Settings {
            interface: interface.into(),
            switcher_flip: true,
            ..Settings::default()
        };
        let mut s = with(&on("classic"));
        assert!(!enabled_on(&s, p, "switcher_flip"), "greyed under Classic");
        assert_eq!(s.pages[p.index()].checked("switcher_flip"), Some(true));
        follow(&mut s, &on("blue"));
        assert!(!enabled_on(&s, p, "switcher_flip"), "greyed under Blue");
        follow(&mut s, &on("glass"));
        assert!(enabled_on(&s, p, "switcher_flip"), "enabled under Glass");
    }

    #[test]
    fn the_pager_mark_combo_carries_its_value() {
        let want = Settings {
            pager_mark: "tile".into(),
            ..Settings::default()
        };
        let s = with(&want);
        match &s.pages[Page::Desktops.index()]
            .item("pager_mark")
            .unwrap()
            .control
        {
            Control::Combo(c) => assert_eq!(c.items[c.selected].text, "Glass tile"),
            _ => panic!("a combo"),
        }
        let odd = Settings {
            pager_mark: "box".into(),
            ..Settings::default()
        };
        let s = with(&odd);
        let mut got = Settings::default();
        read_all(&s, &mut got);
        assert_eq!(got.pager_mark, "box", "an unknown value survives");
    }

    #[test]
    fn the_taskbar_page_geometry() {
        let s = built();
        let p = Page::Taskbar;
        assert_eq!(rect(&s, p, "location_label"), Rect::new(20, 48, 100, 13));
        assert_eq!(rect(&s, p, "taskbar_position"), Rect::new(124, 44, 120, 21));
        assert_eq!(rect(&s, p, "buttons_label"), Rect::new(20, 76, 100, 13));
        assert_eq!(rect(&s, p, "taskbar_buttons"), Rect::new(124, 72, 230, 21));
        for (i, id) in [
            "taskbar_pinned",
            "taskbar_small_icons",
            "taskbar_show_desktop",
            "taskbar_peek",
            "taskbar_tray_overflow",
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                rect(&s, p, id),
                Rect::new(20, 108 + 22 * i as i32, 13, 13),
                "{id} is row {i}"
            );
        }
        assert_eq!(rect(&s, p, "taskbar_note"), Rect::new(20, 216, 358, 26));
        assert_eq!(rect(&s, p, "taskbar_auto_hide"), Rect::new(20, 250, 13, 13));
        for (i, id) in REVEAL_CONTROLS.into_iter().enumerate() {
            assert_eq!(
                rect(&s, p, id),
                Rect::new(38, 272 + 22 * i as i32, 13, 13),
                "{id} sits under the auto-hide label"
            );
        }
        assert!(
            s.pages[p.index()].item("recent_files").is_none(),
            "recent_files is on Advanced"
        );
        assert_eq!(
            rect(&s, p, "start_group"),
            Rect::new(20, 338, 358, 94),
            "the Start Button group ends 4 px above the page's foot"
        );
        assert_eq!(rect(&s, p, "start_icon"), Rect::new(32, 360, 13, 13));
        assert_eq!(rect(&s, p, "banner_label"), Rect::new(32, 384, 110, 13));
        assert_eq!(
            rect(&s, p, "startmenu_banner"),
            Rect::new(146, 380, 220, 20)
        );
        assert_eq!(
            rect(&s, p, "startmenu_style_label"),
            Rect::new(32, 412, 110, 13)
        );
        assert_eq!(rect(&s, p, "startmenu_style"), Rect::new(146, 408, 220, 21));
        inside_page(&s, p);
    }

    #[test]
    fn taskbar_values_go_in_and_come_back_out() {
        let want = Settings {
            taskbar_position: "top".into(),
            taskbar_buttons: "combine".into(),
            taskbar_pinned: true,
            taskbar_peek: true,
            taskbar_tray_overflow: true,
            recent_files: false,
            start_icon: false,
            startmenu_banner: "Reprise".into(),
            startmenu_style: "classic".into(),
            taskbar_auto_hide: true,
            taskbar_reveal_on_edge: false,
            taskbar_reveal_on_attention: false,
            ..Settings::default()
        };
        let s = with(&want);
        match &s.pages[Page::Taskbar.index()]
            .item("taskbar_buttons")
            .unwrap()
            .control
        {
            Control::Combo(c) => assert_eq!(c.items[c.selected].text, "Combine, icons only"),
            _ => panic!("taskbar_buttons is a combo"),
        }
        match &s.pages[Page::Taskbar.index()]
            .item("taskbar_position")
            .unwrap()
            .control
        {
            Control::Combo(c) => assert_eq!(c.items[c.selected].text, "Top"),
            _ => panic!("taskbar_position is a combo"),
        }
        let tb = &s.pages[Page::Taskbar.index()];
        assert_eq!(tb.checked("start_icon"), Some(false));
        assert_eq!(tb.checked("taskbar_auto_hide"), Some(true));
        assert_eq!(tb.checked("taskbar_reveal_on_edge"), Some(false));
        assert_eq!(tb.checked("taskbar_reveal_on_keys"), Some(true));
        let mut got = Settings::default();
        read_all(&s, &mut got);
        assert_eq!(got, want);
    }

    #[test]
    fn the_location_combo_lists_bottom_top_left_and_right() {
        let want = Settings {
            taskbar_position: "right".into(),
            ..Settings::default()
        };
        let s = with(&want);
        match &s.pages[Page::Taskbar.index()]
            .item("taskbar_position")
            .unwrap()
            .control
        {
            Control::Combo(c) => {
                let texts: Vec<&str> = c.items.iter().map(|i| i.text.as_str()).collect();
                assert_eq!(texts, ["Bottom", "Top", "Left", "Right"]);
                assert_eq!(c.items[c.selected].text, "Right");
            }
            _ => panic!("taskbar_position is a combo"),
        }
        let mut got = Settings::default();
        read_all(&s, &mut got);
        assert_eq!(
            got.taskbar_position, "right",
            "read back as the config's name"
        );
    }

    #[test]
    fn the_reveal_check_boxes_grey_while_auto_hide_is_off() {
        let mut got = Settings::default();
        let mut s = with(&got);
        let p = Page::Taskbar;
        assert!(enabled_on(&s, p, "taskbar_auto_hide"));
        for id in REVEAL_CONTROLS {
            assert!(!enabled_on(&s, p, id), "{id} greys with auto-hide off");
        }
        if let Some(Control::Check { checked, .. }) = s.pages[p.index()]
            .item_mut("taskbar_auto_hide")
            .map(|i| &mut i.control)
        {
            *checked = true;
        }
        read_one(&s, "taskbar_auto_hide", &mut got);
        follow(&mut s, &got);
        assert!(got.taskbar_auto_hide);
        for id in REVEAL_CONTROLS {
            assert!(enabled_on(&s, p, id), "{id} comes back with auto-hide on");
        }
    }

    #[test]
    fn the_launcher_page_geometry() {
        let s = built();
        let p = Page::Launcher;
        assert_eq!(
            rect(&s, p, "launcher_search_group"),
            Rect::new(20, 44, 358, 86)
        );
        assert_eq!(
            rect(&s, p, "launcher_search_note"),
            Rect::new(32, 66, 334, 26)
        );
        assert_eq!(rect(&s, p, "launcher_files"), Rect::new(32, 104, 13, 13));
        inside_page(&s, p);
    }

    #[test]
    fn launcher_values_go_in_and_come_back_out() {
        let want = Settings {
            launcher_files: false,
            ..Settings::default()
        };
        let s = with(&want);
        assert_eq!(
            s.pages[Page::Launcher.index()].checked("launcher_files"),
            Some(false)
        );
        let mut got = Settings::default();
        read_all(&s, &mut got);
        assert_eq!(got, want);
    }

    #[test]
    fn the_windows_page_geometry() {
        let s = built();
        let p = Page::Windows;
        assert_eq!(rect(&s, p, "snap_group"), Rect::new(20, 44, 358, 172));
        assert_eq!(rect(&s, p, "snap_enabled"), Rect::new(32, 66, 13, 13));
        assert_eq!(rect(&s, p, "snap_picker"), Rect::new(32, 90, 13, 13));
        assert_eq!(rect(&s, p, "snap_bar"), Rect::new(32, 114, 13, 13));
        assert_eq!(
            rect(&s, p, "snap_distance_label"),
            Rect::new(32, 142, 150, 13)
        );
        assert_eq!(rect(&s, p, "snap_distance").y, 138);
        assert_eq!(
            rect(&s, p, "snap_distance_units"),
            Rect::new(242, 142, 30, 13)
        );
        assert_eq!(rect(&s, p, "snap_layouts"), Rect::new(120, 166, 246, 20));
        assert_eq!(rect(&s, p, "tabs_group"), Rect::new(20, 228, 358, 96));
        assert_eq!(rect(&s, p, "window_tabs"), Rect::new(32, 250, 13, 13));
        assert_eq!(rect(&s, p, "remember_windows"), Rect::new(32, 274, 13, 13));
        assert_eq!(rect(&s, p, "window_shake"), Rect::new(32, 298, 13, 13));
        inside_page(&s, p);
    }

    #[test]
    fn the_windows_page_reads_back_tabs_shaking_the_picker_the_bar_the_distance_and_the_layouts() {
        let want = Settings {
            window_tabs: false,
            snap_picker: false,
            snap_bar: false,
            snap_layouts: "1|1 2-1:2".into(),
            snap_distance: 15,
            remember_windows: false,
            window_shake: true,
            ..Settings::default()
        };
        let s = with(&want);
        let p = &s.pages[Page::Windows.index()];
        assert_eq!(p.checked("window_tabs"), Some(false));
        assert_eq!(p.checked("window_shake"), Some(true));
        assert_eq!(p.checked("snap_picker"), Some(false));
        assert_eq!(p.checked("snap_bar"), Some(false));
        let mut got = Settings::default();
        read_all(&s, &mut got);
        assert_eq!(got, want);
    }

    #[test]
    fn the_startup_page_geometry() {
        let s = built();
        let p = Page::Startup;
        assert_eq!(rect(&s, p, ADD), Rect::new(222, 260, 75, 23));
        assert_eq!(rect(&s, p, REMOVE), Rect::new(303, 260, 75, 23));
        assert_eq!(rect(&s, p, "session_group"), Rect::new(20, 292, 358, 66));
        assert_eq!(rect(&s, p, "session_restore"), Rect::new(32, 314, 13, 13));
        assert_eq!(
            rect(&s, p, "session_end_processes"),
            Rect::new(32, 336, 13, 13)
        );
        inside_page(&s, p);

        assert_eq!(list(Area::MIN), Rect::new(20, 44, 358, 208));
        let lv = new_list();
        let lay = listview::layout(
            &lv,
            list(Area::MIN),
            &reprisekit::controls::metrics::ControlMetrics::CLASSIC,
        );
        assert_eq!(lay.header, Rect::new(22, 46, 354, listview::HEADER_H));
        assert_eq!(lay.rows, 11, "11 rows of 17 px under the header");
        assert_eq!(
            COLUMNS.iter().map(|c| c.1).sum::<i32>(),
            lay.header.width - 16,
            "the columns leave the 16 px bar its room"
        );
    }

    #[test]
    fn a_startup_list_that_scrolls_draws_only_the_vertical_bar() {
        use reprisekit::interface::Interface;
        let text = TextRenderer::load(&FontConfig::default());
        let entries: Vec<Entry> = (0..30)
            .map(|i| Entry {
                id: format!("item{i}"),
                name: format!("Item {i}"),
                exec: format!("/usr/bin/item{i} --tray"),
                exec_line: format!("/usr/bin/item{i} --tray"),
                hidden: false,
                only_show_in: Vec::new(),
                not_show_in: Vec::new(),
                try_exec: None,
                path: None,
                terminal: false,
                under_systemd: false,
                user: true,
            })
            .collect();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for pct in [100, 150, 200] {
                let theme = reprisekit::interfaces::theme_for(i).with_text_size(pct);
                let cm = theme.controls();
                let r = theme.text.rect(list(Area::MIN));
                let mut lv = new_list();
                fill_list(&mut lv, &entries, &[true; 30]);
                fit_list_columns(&mut lv, &cm, r);
                let lay = listview::layout(&lv, r, &cm);
                let at = format!("{i:?} {pct}%");
                assert!(lay.hscroll.is_none(), "{at}: no horizontal bar");
                let bar = lay.scrollbar.expect("a vertical bar");
                assert_eq!(bar.right(), r.right() - 2, "{at}");
                let mut c = Canvas::new((r.right() + 2) as u32, (r.bottom() + 2) as u32);
                draw_list(&mut c, &text, &theme, &lv, r, theme.fonts.size_px, 1);
                let window = theme.colors.window.argb();
                assert_eq!(
                    c.get(lay.body.x + lay.body.width / 2, r.bottom() - 3),
                    window,
                    "{at}: the last row runs down to the frame"
                );
                assert_ne!(
                    c.get(bar.x + bar.width / 2, bar.bottom() - 3),
                    window,
                    "{at}: the bar"
                );
            }
        }
    }

    #[test]
    fn the_night_light_page_geometry() {
        let s = built();
        let p = Page::NightLight;
        assert_eq!(rect(&s, p, "nl_enabled"), Rect::new(20, 44, 13, 13));
        assert_eq!(rect(&s, p, "nl_color_group"), Rect::new(20, 70, 358, 76));
        assert_eq!(rect(&s, p, "nl_temp_value"), Rect::new(286, 90, 80, 13));
        assert_eq!(
            rect(&s, p, "nl_temperature"),
            Rect::new(78, 111, 238, trackbar::HEIGHT)
        );
        assert_eq!(rect(&s, p, "nl_cool"), Rect::new(322, 117, 44, 13));
        assert_eq!(
            rect(&s, p, "nl_schedule_group"),
            Rect::new(20, 156, 358, 104)
        );
        assert_eq!(rect(&s, p, "nl_sun"), Rect::new(32, 178, 12, 12));
        assert_eq!(rect(&s, p, "nl_fixed"), Rect::new(32, 200, 12, 12));
        assert_eq!(rect(&s, p, "nl_from"), Rect::new(90, 226, 70, 20));
        assert_eq!(rect(&s, p, "nl_to"), Rect::new(210, 226, 70, 20));
        inside_page(&s, p);
    }

    #[test]
    fn night_light_values_go_in_and_come_back_out() {
        let want = Settings {
            nl_enabled: true,
            nl_temperature: 3400,
            nl_schedule: "fixed".into(),
            nl_from: "20:30".into(),
            nl_to: "06:05".into(),
            ..Settings::default()
        };
        let s = with(&want);
        let page = &s.pages[Page::NightLight.index()];
        assert_eq!(page.track_pos("nl_temperature"), Some(9));
        assert_eq!(page.checked("nl_fixed"), Some(true));
        assert_eq!(page.checked("nl_sun"), Some(false));
        match &page.item("nl_from").unwrap().control {
            Control::Time(tf) => assert_eq!(tf.text().0, "20:30"),
            _ => panic!("nl_from is a time field"),
        }
        match &page.item("nl_temp_value").unwrap().control {
            Control::Label { text, .. } => assert_eq!(text, "3400 K"),
            _ => panic!("a label"),
        }
        let mut got = Settings::default();
        read_all(&s, &mut got);
        assert_eq!(got, want);
    }

    #[test]
    fn the_night_light_controls_follow_the_check_box_and_the_schedule() {
        let enabled = |s: &PropSheet, id: &str| match &s.pages[Page::NightLight.index()]
            .item(id)
            .unwrap()
            .control
        {
            Control::Label { enabled, .. } | Control::Radio { enabled, .. } => *enabled,
            Control::TrackBar(t) => t.enabled,
            Control::Time(t) => t.enabled,
            _ => panic!("{id} has no greyed state here"),
        };
        let off = with(&Settings::default());
        for id in ["nl_temperature", "nl_warm", "nl_sun", "nl_from"] {
            assert!(!enabled(&off, id), "{id} greys while off");
        }
        let sun = Settings {
            nl_enabled: true,
            ..Settings::default()
        };
        let s = with(&sun);
        assert!(enabled(&s, "nl_temperature"));
        assert!(enabled(&s, "nl_fixed"));
        assert!(!enabled(&s, "nl_from"), "sunset needs no times");
        assert!(!enabled(&s, "nl_to_label"));
        let fixed = Settings {
            nl_schedule: "fixed".into(),
            ..sun
        };
        let s = with(&fixed);
        assert!(enabled(&s, "nl_from"));
        assert!(enabled(&s, "nl_to_label"));
    }

    #[test]
    fn the_switcher_preview_greys_with_thumbnails() {
        let enabled = |s: &PropSheet| match &s.pages[Page::Switcher.index()]
            .item("switcher_preview")
            .unwrap()
            .control
        {
            Control::Check { enabled, .. } => *enabled,
            _ => panic!("a check box"),
        };
        let mut want = Settings {
            switcher_style: "thumbnails".into(),
            ..Settings::default()
        };
        let mut s = with(&want);
        assert!(!enabled(&s), "thumbnails grey the preview check box");
        assert_eq!(
            s.pages[Page::Switcher.index()].checked("switcher_preview"),
            Some(true)
        );
        want.switcher_style = "icons".into();
        follow(&mut s, &want);
        assert!(enabled(&s), "icons bring the preview check box back");
    }

    #[test]
    fn values_go_in_and_come_back_out() {
        let want = Settings {
            desktop_icons: false,
            desktop_extensions: "show".into(),
            wallpaper_position: "tile".into(),
            background: "#336699".into(),
            system_icons: vec!["computer".into(), "recycle-bin".into()],
            start_icon: false,
            adaptive_sync: true,
            preview_taskbar: false,
            previews_fps: 24,
            switcher_style: "thumbnails".into(),
            screenshot_save: false,
            clipboard_history: false,
            desktops: 9,
            desktop_animation: "zoom".into(),
            desktop_animation_ms: 400,
            remember_windows: false,
            session_restore: true,
            session_end_processes: false,
            hibernate: "zzz".into(),
            lock_on_sleep: false,
            taskbar_height: 32,
            icon_theme: "hicolor".into(),
            font_family: "Verdana".into(),
            color_scheme: "Brick".into(),
            ..Settings::default()
        };
        let sheet = with(&want);

        let mut got = Settings::default();
        read_all(&sheet, &mut got);
        assert_eq!(got, want);

        let glass = Settings {
            interface: "glass".into(),
            glass_color: "ruby".into(),
            glass_intensity: 70,
            glass_transparency: false,
            glass_blur: 4,
            glass_streaks: false,
            glass_fast_drag: false,
            glass_blur_frames: false,
            ..Settings::default()
        };
        let sheet = with(&glass);
        let mut got = Settings {
            interface: "glass".into(),
            ..Settings::default()
        };
        read_all(&sheet, &mut got);
        assert_eq!(got, glass);
    }

    #[test]
    fn only_the_control_that_changed_is_read_back() {
        // Past the idle up-down's maximum: the control has to clamp it to show it at
        // all, but that clamp must not reach `Settings` and be written over a key the user never touched.
        let want = Settings {
            lock_after: 99_999_999,
            ..Settings::default()
        };
        let sheet = with(&want);

        let mut got = want.clone();
        read_one(&sheet, "taskbar_height", &mut got);
        assert_eq!(
            got.lock_after, 99_999_999,
            "an untouched field keeps what the file said"
        );

        // Naming it is the user having worked it, and then the clamp is what they see and what gets written.
        read_one(&sheet, "lock_after", &mut got);
        assert_eq!(got.lock_after, 99_999);
    }

    #[test]
    fn the_pointers_page_geometry() {
        let s = built();
        let p = Page::Pointers;
        assert_eq!(rect(&s, p, "cursor_group"), Rect::new(20, 44, 358, 84));
        assert_eq!(rect(&s, p, "cursor_theme"), Rect::new(92, 62, 274, 21));
        assert_eq!(rect(&s, p, "cursor_size"), Rect::new(92, 92, 110, 21));
        assert_eq!(rect(&s, p, CURSOR_PREVIEW), Rect::new(32, 160, 334, 208));
        assert_eq!(preview_size(Area::MIN), (330, 204));
    }

    #[test]
    fn an_empty_cursor_theme_is_the_default_row() {
        let sheet = with(&Settings::default());
        let page = &sheet.pages[Page::Pointers.index()];
        assert_eq!(page.combo_selected("cursor_theme"), Some(0));
        assert_eq!(page.combo_selected("cursor_size"), Some(0));
        let mut got = Settings {
            cursor_theme: "stale".into(),
            cursor_size: 7,
            ..Settings::default()
        };
        read_all(&sheet, &mut got);
        assert_eq!(got.cursor_theme, "", "(Default) reads back as no theme");
        assert_eq!(got.cursor_size, 0, "Default reads back as size 0");
    }

    #[test]
    fn automatic_toolkits_are_the_first_rows() {
        let sheet = with(&Settings::default());
        let page = &sheet.pages[Page::FontsAndIcons.index()];
        assert_eq!(page.combo_selected("gtk_theme"), Some(0));
        assert_eq!(page.combo_selected("qt_platform_theme"), Some(0));
        let mut got = Settings {
            gtk_theme: "stale".into(),
            qt_platform_theme: "stale".into(),
            ..Settings::default()
        };
        read_all(&sheet, &mut got);
        assert_eq!(
            (got.gtk_theme.as_str(), got.qt_platform_theme.as_str()),
            ("auto", "auto")
        );
        let odd = Settings {
            gtk_theme: "Gone".into(),
            qt_platform_theme: "lxqt".into(),
            ..Settings::default()
        };
        let mut got = Settings::default();
        read_all(&with(&odd), &mut got);
        assert_eq!(
            (got.gtk_theme.as_str(), got.qt_platform_theme.as_str()),
            ("Gone", "lxqt")
        );
    }

    /// "Reprise (follows the look)" is each toolkit combo's second row and reads
    /// back as "reprise"; the Qt one is greyed without qt6ct or qt5ct.
    #[test]
    fn reprise_toolkits_are_the_second_rows_and_qt_needs_a_tool() {
        let want = Settings {
            gtk_theme: TOOLKIT_REPRISE.into(),
            qt_platform_theme: TOOLKIT_REPRISE.into(),
            ..Settings::default()
        };
        let sheet = with(&want);
        let page = &sheet.pages[Page::FontsAndIcons.index()];
        assert_eq!(page.combo_selected("gtk_theme"), Some(1));
        assert_eq!(page.combo_selected("qt_platform_theme"), Some(1));
        let mut got = Settings::default();
        read_all(&sheet, &mut got);
        assert_eq!(
            (got.gtk_theme.as_str(), got.qt_platform_theme.as_str()),
            ("reprise", "reprise")
        );
        let qt_row = |tools| {
            let page = fonts_and_icons(Area::MIN, &[], &[], tools);
            match &page.item("qt_platform_theme").unwrap().control {
                Control::Combo(c) => (c.items[1].text.clone(), c.items[1].greyed),
                _ => unreachable!("the Qt style is a combo"),
            }
        };
        assert_eq!(qt_row(true), (REPRISE_TOOLKIT_ROW.to_string(), false));
        assert_eq!(qt_row(false), (REPRISE_TOOLKIT_ROW.to_string(), true));
    }

    #[test]
    fn font_smoothing_starts_on_grayscale_and_reads_back_its_pick() {
        let sheet = with(&Settings::default());
        let page = &sheet.pages[Page::FontsAndIcons.index()];
        assert_eq!(page.combo_selected("font_smoothing"), Some(0));
        let none = Settings {
            font_smoothing: "none".into(),
            ..Settings::default()
        };
        let sheet = with(&none);
        assert_eq!(
            sheet.pages[Page::FontsAndIcons.index()].combo_selected("font_smoothing"),
            Some(2)
        );
        let mut got = Settings::default();
        read_all(&sheet, &mut got);
        assert_eq!(got.font_smoothing, "none");
    }

    #[test]
    fn a_cursor_theme_and_size_the_lists_lack_still_show() {
        let want = Settings {
            cursor_theme: "gone".into(),
            cursor_size: 40,
            ..Settings::default()
        };
        let sheet = with(&want);
        let mut got = Settings::default();
        read_all(&sheet, &mut got);
        assert_eq!((got.cursor_theme.as_str(), got.cursor_size), ("gone", 40));
    }

    #[test]
    fn an_unlisted_icon_theme_still_shows() {
        let want = Settings {
            icon_theme: "gone".into(),
            ..Settings::default()
        };
        let sheet = with(&want);
        let mut got = Settings::default();
        read_all(&sheet, &mut got);
        assert_eq!(got.icon_theme, "gone");
    }

    /// Writes target/reprise-settings-<page>.png for each page.
    #[test]
    #[ignore]
    fn render_the_pages() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let two = Settings {
            kb_layout: "us,hu".into(),
            ..Settings::default()
        };
        let mut sheet = with(&two);
        fill_layouts(&mut sheet, &two, &reg(), Some(1));
        let pic =
            crate::cursors::preview("Adwaita", 32, preview_size(Area::MIN), theme.colors.window);
        sheet.pages[Page::Pointers.index()].set_picture(CURSOR_PREVIEW, pic);
        backgroundpage::show(&mut sheet.pages[Page::Background.index()], &two, "", None);
        let mut lv = new_list();
        let entries: Vec<Entry> = [
            (
                "reprise-sound",
                "Sounds and Audio Devices",
                "reprise-sound --tray",
                false,
            ),
            ("nm-applet", "Network", "nm-applet", false),
            ("mine", "My Script", "/home/user/bin/mine", true),
        ]
        .iter()
        .map(|(id, name, exec, user)| Entry {
            id: (*id).into(),
            name: (*name).into(),
            exec: (*exec).into(),
            exec_line: (*exec).into(),
            hidden: false,
            only_show_in: Vec::new(),
            not_show_in: Vec::new(),
            try_exec: None,
            path: None,
            terminal: false,
            under_systemd: false,
            user: *user,
        })
        .collect();
        fill_list(&mut lv, &entries, &[true, false, true]);
        lv.selection.set_only(Some(0));
        let keyboard = crate::model::load(
            "[[global_shortcuts]]\napp = \"org.example.Talk\"\nid = \"talk\"\n\
             description = \"Hold to talk\"\nkeys = \"Pause\"\n\
             [portal]\nshortcuts_denied = [\"org.example.Notes\"]\n",
        );
        let mut sc = new_shortcut_list();
        fill_shortcuts(&mut sc, &keyboard, &|a| {
            a.strip_prefix("org.example.").unwrap_or(a).into()
        });
        let n = keyboard.shortcuts.len();
        sc.top = n - 2;
        sc.selection.set_only(Some(n));

        let mut shown = crate::displaypage::test_head("DP-1");
        shown.adaptive_sync = repriseapp::heads::AdaptiveSync::Off;
        let display = crate::displaypage::Display::with_heads(vec![
            shown,
            crate::displaypage::test_head("HDMI-A-1"),
        ]);
        crate::displaypage::refresh(&mut sheet, &display);
        show_clearing(
            &mut sheet.pages[Page::Advanced.index()],
            Clearing::Done(Cleared {
                files: 214,
                bytes: 1_384_448,
            }),
        );
        for p in Page::all() {
            sheet.tabs.open = p.index();
            sheet.reveal_open();
            let mut c = Canvas::new(sheet.size.0 as u32, sheet.size.1 as u32);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            propsheet::draw(&mut c, &text, &theme.paint(), &sheet, true, px);
            if p == Page::Startup {
                draw_list(
                    &mut c,
                    &text,
                    &theme,
                    &lv,
                    sheet.on_page(list(Area::MIN)),
                    px,
                    1,
                );
            }
            if p == Page::Keyboard {
                draw_shortcuts(
                    &mut c,
                    &text,
                    &theme,
                    &sc,
                    sheet.on_page(shortcuts(Area::MIN)),
                    px,
                    1,
                );
            }
            reprisekit::raster::write_preview(&c, &format!("reprise-settings-{}.png", p.slug()));
        }
        let mut glass = with(&Settings {
            interface: "glass".into(),
            ..two.clone()
        });
        glass.tabs.open = Page::Appearance.index();
        let mut c = Canvas::new(glass.size.0 as u32, glass.size.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        propsheet::draw(&mut c, &text, &theme.paint(), &glass, true, px);
        reprisekit::raster::write_preview(&c, "reprise-settings-appearance-glass.png");
    }
}
