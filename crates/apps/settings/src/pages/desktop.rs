//! Taskbar, Search, Window Switcher, Virtual Desktops and Window Behavior.

use super::{Area, GX, TRAY_CUSTOMIZE, X, choice, edit, group, spin};
use reprisekit::classic::widgets::{BUTTON_H, LINE_H};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::raster::Rect;
use reprisekit::trackbar::{self, TrackBar};

use crate::model::{
    DESKTOPS_MAX, DIM_INACTIVE_MAX, HIDDEN_FRAME_RATE_MAX, HOT_CORNERS, PAGER_MARKS,
    PREVIEW_FPS_MAX, SNAP_DISTANCE_MAX, SWITCH_ANIMATIONS, SWITCH_MS_MAX, SWITCHER_SIZES,
    SWITCHER_STYLES, Shortcut, TASKBAR_BUTTONS, TASKBAR_POSITIONS, TITLE_DOUBLE_CLICKS, keys_for,
};

const CUSTOMIZE_W: i32 = 84;

/// How windows snap, tab and come back: the Snapping group holds
/// `[window_snapping]`'s drag snapping and how near an edge it starts (greyed
/// with drag snapping off), the snap layouts on the maximize button, the snap
/// bar a drag shows (greyed too) and the layouts they offer; Tabs and Placement
/// holds `[windows]`, shaking, the title bar double click, dimming and the hidden frame rate
/// included.
pub fn window_behavior(a: Area) -> Form {
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
        group(a, "tabs_group", 228, 210, "Tabs and Placement"),
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
        Item::button(
            FORGET_FRAMES,
            Rect::new(GX, 296, 110, BUTTON_H),
            "&Forget Positions",
            true,
            false,
        )
        .under_check(),
        Item::check(
            "window_shake",
            GX,
            328,
            "&Shake a window by its title bar to minimize the others",
        ),
        Item::label_at(
            "title_double_click_label",
            GX,
            356,
            150,
            "Double-clic&king a title bar:",
        ),
        choice(
            "title_double_click",
            GX + 154,
            352,
            120,
            TITLE_DOUBLE_CLICKS,
        ),
    ]);
    items.extend(spin(
        "dim_inactive_label",
        "dim_inactive",
        GX,
        384,
        230,
        "Dim &windows that don't have the focus:",
        (0, DIM_INACTIVE_MAX as i32),
    ));
    items.push(Item::label_at("dim_inactive_units", GX + 290, 384, 20, "%"));
    items.extend(spin(
        "hidden_frame_rate_label",
        "hidden_frame_rate",
        GX,
        412,
        170,
        "Frame &rate of hidden windows:",
        (0, HIDDEN_FRAME_RATE_MAX as i32),
    ));
    items.push(Item::label_at(
        "hidden_frame_rate_units",
        GX + 230,
        412,
        gw - 230,
        "(0: no limit)",
    ));
    Form::new(items)
}

/// The Window Behavior page's Forget Positions under the remember check: the compositor drops every
/// window place it kept, for every set of screens.
pub const FORGET_FRAMES: &str = "forget_window_frames";

/// The Window Behavior page controls that grey with drag snapping off.
pub(super) const DRAG_SNAP_CONTROLS: [&str; 4] = [
    "snap_bar",
    "snap_distance_label",
    "snap_distance",
    "snap_distance_units",
];

/// The virtual desktop controls that grey with one desktop; the duration's also grey without an
/// animation.
pub(super) const DESKTOP_CONTROLS: [&str; 10] = [
    "desktop_names_label",
    DESKTOP_NAMES,
    "desktop_names_note",
    "desktops_label",
    "desktops",
    "animation_label",
    "desktop_animation",
    "pager_mark_label",
    "pager_mark",
    "wheel_on_desktop",
];

pub(super) const DURATION_CONTROLS: [&str; 3] =
    ["duration_label", "desktop_animation_ms", "duration_units"];

/// Under the Virtual Desktops group and beside the Taskbar page's slide: blank, or the matching
/// note while reduce motion greys those rows.
pub(super) const DESKTOP_MOTION_NOTE: &str = "desktop_animation_note";
pub(super) const SLIDE_MOTION_NOTE: &str = "taskbar_slide_note";
pub(super) const MOTION_NOTE: &str = "Off while Reduce motion is on (Effects page)";
pub(super) const SLIDE_MOTION_TEXT: &str = "(Reduce motion)";

/// The Taskbar page's window picture rate, greyed while nothing shows a live picture.
pub(super) const FPS_CONTROLS: [&str; 3] = ["fps_label", "previews_fps", "fps_units"];

/// The desktops' names, apart by commas.
pub const DESKTOP_NAMES: &str = "desktop_names";

pub fn virtual_desktops(a: Area) -> Form {
    let mut items = vec![group(a, "desktops_group", 44, 244, "Virtual Desktops")];
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
    indented.push(Item::check(
        "wheel_on_desktop",
        ix,
        214,
        "Switch desktops with the wheel over the des&ktop",
    ));
    let iw = a.gw() - 18;
    indented.push(Item::label_at(
        "desktop_names_label",
        ix,
        242,
        90,
        "Desktop na&mes:",
    ));
    indented.push(edit(DESKTOP_NAMES, ix + 94, 238, iw - 94));
    indented.push(Item::label_at(
        "desktop_names_note",
        ix,
        264,
        iw,
        "Apart by commas; an empty one keeps its number",
    ));
    items.extend(indented.into_iter().map(Item::under_check));
    items.push(Item::label_at(DESKTOP_MOTION_NOTE, X, 296, a.w, ""));
    Form::new(items)
}

/// The edge the bar sits on, its height and how windows become task buttons, then one check box
/// per optional part of the taskbar, auto-hide with its three triggers indented under it, the live
/// window pictures and their rate, and the Start button's logo and glow.
pub fn taskbar(a: Area) -> Form {
    let mut items = vec![
        Item::label_at("location_label", X, 48, 100, "Taskbar l&ocation:"),
        choice("taskbar_position", X + 104, 44, 120, TASKBAR_POSITIONS),
    ];
    items.extend(spin(
        "taskbar_height_label",
        "taskbar_height",
        X + 234,
        48,
        44,
        "Hei&ght:",
        (8, 256),
    ));
    items.extend([
        Item::label_at("taskbar_height_units", X + 338, 48, 20, "px"),
        Item::label_at("buttons_label", X, 76, 100, "Taskbar &buttons:"),
        choice("taskbar_buttons", X + 104, 72, 230, TASKBAR_BUTTONS),
    ]);
    let checks = [
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
        ("taskbar_badges", "Show unread &counts on task buttons"),
        ("taskbar_tray_overflow", "Hide &idle notification icons"),
        (
            "taskbar_network_meter",
            "Show n&etwork speed beside the clock",
        ),
        (
            "taskbar_slide",
            "Slide taskbar buttons as they are d&ragged",
        ),
    ];
    let slide_y = 100 + 18 * (checks.len() as i32 - 1);
    for (i, (id, text)) in checks.into_iter().enumerate() {
        items.push(Item::check(id, X, 100 + 18 * i as i32, text));
    }
    items.push(Item::label_at(
        SLIDE_MOTION_NOTE,
        X + 262,
        slide_y,
        a.w - 262,
        "",
    ));
    items.push(Item::button(
        TRAY_CUSTOMIZE,
        Rect::new(a.right() - CUSTOMIZE_W, 167, CUSTOMIZE_W, BUTTON_H),
        "C&ustomize...",
        true,
        false,
    ));
    items.push(Item::new(
        "taskbar_note",
        Rect::new(X, 228, a.w, 3 * LINE_H),
        Control::Lines(vec![
            "Left unchanged, these follow the interface: Glass combines".into(),
            "buttons and turns on the desktop button and Peek. Small".into(),
            "icons shorten the Glass taskbar only.".into(),
        ]),
    ));
    items.push(Item::check(
        "taskbar_auto_hide",
        X,
        272,
        "Auto-&hide the taskbar",
    ));
    let reveals = [
        "Show when the pointer &touches the edge",
        "Show for the Start menu and window s&witching",
        "Show when a window needs &attention",
    ];
    for (i, (id, text)) in REVEAL_CONTROLS.into_iter().zip(reveals).enumerate() {
        items.push(Item::check(id, X, 290 + 18 * i as i32, text).under_check());
    }
    items.push(Item::check(
        "preview_taskbar",
        X,
        344,
        "Show live window &pictures over taskbar buttons",
    ));
    let [fps_label, fps] = spin(
        "fps_label",
        "previews_fps",
        X,
        364,
        110,
        "Re&fresh rate:",
        (1, PREVIEW_FPS_MAX as i32),
    );
    // Under the check, 18 px in: the units' width gives that back to stay on the page.
    let fps_units = Item::label_at(
        "fps_units",
        X + 170,
        364,
        a.w - 188,
        "times a second, Alt+Tab too",
    );
    items.extend([fps_label, fps, fps_units].map(Item::under_check));
    items.extend([
        Item::check(
            "start_icon",
            X,
            386,
            "Show the icon theme's &logo on the Start button",
        ),
        Item::label_at("orb_glow_label", X, 410, 110, "Start butto&n glow:"),
        Item::new(
            "glass_orb_glow",
            Rect::new(X + 114, 404, a.track(150), trackbar::HEIGHT),
            Control::TrackBar(TrackBar::new(100, 101).paged(10, 5)),
        ),
        Item::label_at(
            "orb_glow_note",
            X,
            434,
            a.w,
            "The glow shows under Glass only.",
        ),
    ]);
    Form::new(items)
}

/// The Start button glow's rows, greyed outside Glass.
pub(super) const GLOW_CONTROLS: [&str; 2] = ["orb_glow_label", "glass_orb_glow"];

/// The auto-hide triggers' check boxes, indented under "Auto-hide the taskbar" and greyed while it is off.
pub(super) const REVEAL_CONTROLS: [&str; 3] = [
    "taskbar_reveal_on_edge",
    "taskbar_reveal_on_keys",
    "taskbar_reveal_on_attention",
];

pub const SEARCH_NOTE: &str = "launcher_search_note";
pub const CONTENTS_NOTE: &str = "launcher_contents_note";
pub const FLIP_NOTE: &str = "flip_note";
pub const OVERVIEW_NOTE: &str = "overview_note";
/// The search note's most lines, and the characters a line holds.
const SEARCH_LINES: usize = 3;
const SEARCH_COLS: usize = 55;

/// The Search page: the Search group's note naming the launcher's keys, its checks for files
/// and folders and for open windows, and the note on content search. [`super::show_keys`] fills
/// the first note, [`show_contents`] the second.
pub fn search(a: Area) -> Form {
    Form::new(vec![
        group(a, "launcher_search_group", 44, 156, "Search"),
        Item::new(
            SEARCH_NOTE,
            Rect::new(GX, 66, a.gw(), SEARCH_LINES as i32 * LINE_H),
            Control::Lines(search_note(&[])),
        ),
        Item::check(
            "launcher_files",
            GX,
            117,
            "Also find &files and folders in my home folder",
        ),
        Item::check("launcher_windows", GX, 139, "Also find open &windows"),
        Item::new(
            CONTENTS_NOTE,
            Rect::new(GX, 161, a.gw(), 2 * LINE_H),
            Control::Lines(Vec::new()),
        ),
    ])
}

/// The content search note: what finding files by their contents takes, greyed while localsearch
/// is not installed; blank until `installed` is read.
pub fn show_contents(page: &mut Form, installed: Option<bool>) {
    let note = match installed {
        None => "",
        Some(true) => "Files are also found by their contents through localsearch.",
        Some(false) => "Install localsearch to also find files by their contents.",
    };
    let lines = match note.is_empty() {
        true => Vec::new(),
        false => reprisekit::text::wrap_chars(note, SEARCH_COLS),
    };
    if let Some(it) = page.item_mut(CONTENTS_NOTE) {
        it.control = match installed {
            Some(false) => Control::GreyedLines(lines),
            _ => Control::Lines(lines),
        };
    }
}

/// The Search note's lines: the keys bound to the launcher, what it finds, and that the Start
/// menu's search follows the checks. Keys too long for the note drop the Start menu sentence.
pub fn search_note(binds: &[Shortcut]) -> Vec<String> {
    const FINDS: &str = "finds installed programs as you type their names.";
    const START: &str = "The Start menu's search box follows these too.";
    let wrap = |body: &str| reprisekit::text::wrap_chars(body, SEARCH_COLS);
    let Some(keys) = keys_for(binds, "launcher") else {
        return wrap(&format!("The launcher {FINDS} {START}"));
    };
    let opens = format!("{keys} opens the launcher, which {FINDS}");
    let mut lines = wrap(&format!("{opens} {START}"));
    if lines.len() > SEARCH_LINES {
        lines = wrap(&opens);
    }
    lines.truncate(SEARCH_LINES);
    lines
}

/// The 3D switcher check's note: the keys bound to it, and that it runs under Glass alone.
pub fn flip_note(binds: &[Shortcut]) -> String {
    let keys = keys_for(binds, "flip").unwrap_or_else(|| "No key".into());
    format!("{keys} shows them; Glass only.")
}

/// The Window Overview note, naming the keys bound to the overview.
pub fn overview_note(binds: &[Shortcut]) -> String {
    keys_for(binds, "overview").map_or_else(
        || "Every window of this desktop side by side.".into(),
        |k| format!("{k} shows this desktop's windows side by side."),
    )
}

/// The Window Switcher page: what Alt+Tab shows and how large its pictures
/// are, the desktop entry and peek with it, the 3D switcher, and the window
/// overview's hot corner under it. [`super::show_keys`] fills the notes.
pub fn switcher(a: Area) -> Form {
    Form::new(vec![
        group(a, "switcher_group", 44, 186, "Alt+Tab"),
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
        Item::check("switcher_flip", GX, 184, "Show windows in &3D"),
        Item::label_at(FLIP_NOTE, GX + 18, 202, a.gw() - 18, &flip_note(&[])),
        group(a, "overview_group", 240, 76, "Window Overview"),
        Item::new(
            OVERVIEW_NOTE,
            Rect::new(GX, 262, a.gw(), LINE_H),
            Control::Lines(vec![overview_note(&[])]),
        ),
        Item::label_at("hot_corner_label", GX, 288, 110, "Hot &corner:"),
        choice("hot_corner", GX + 114, 284, 130, HOT_CORNERS),
    ])
}
