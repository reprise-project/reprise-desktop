//! Taskbar, Launcher, Switcher, Desktops and Windows.

use super::{Area, GX, TRAY_CUSTOMIZE, X, button, choice, edit, group, spin};
use reprisekit::classic::widgets::{BUTTON_W, LINE_H};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::raster::Rect;

use crate::model::{
    DESKTOPS_MAX, HOT_CORNERS, PAGER_MARKS, PREVIEW_FPS_MAX, SNAP_DISTANCE_MAX, STARTMENU_STYLES,
    SWITCH_ANIMATIONS, SWITCH_MS_MAX, SWITCHER_SIZES, SWITCHER_STYLES, TASKBAR_BUTTONS,
    TASKBAR_POSITIONS,
};

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
pub(super) const DRAG_SNAP_CONTROLS: [&str; 4] = [
    "snap_bar",
    "snap_distance_label",
    "snap_distance",
    "snap_distance_units",
];

/// The virtual desktop controls that grey with one desktop; the duration's also grey without an
/// animation, and the preview rate's while nothing shows a live picture.
pub(super) const DESKTOP_CONTROLS: [&str; 6] = [
    "desktops_label",
    "desktops",
    "animation_label",
    "desktop_animation",
    "pager_mark_label",
    "pager_mark",
];

pub(super) const DURATION_CONTROLS: [&str; 3] =
    ["duration_label", "desktop_animation_ms", "duration_units"];

pub(super) const FPS_CONTROLS: [&str; 3] = ["fps_label", "previews_fps", "fps_units"];

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
pub(super) const REVEAL_CONTROLS: [&str; 3] = [
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
