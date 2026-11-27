//! Taskbar, Search, Window Switcher, Virtual Desktops and Window Behavior.

use super::{Area, GX, TRAY_CUSTOMIZE, X, button, choice, edit, group, spin};
use crate::providerpage;
use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W, LINE_H};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::listbox::{self, ListBox};
use reprisekit::raster::Rect;
use reprisekit::trackbar::{self, TrackBar};

use crate::model::{
    DESKTOPS_MAX, DIM_INACTIVE_MAX, HIDDEN_FRAME_RATE_MAX, HOT_CORNERS, PAGER_MARKS,
    PREVIEW_FPS_MAX, SNAP_DISTANCE_MAX, SWITCH_ANIMATIONS, SWITCH_MS_MAX, SWITCHER_SIZES,
    SWITCHER_STYLES, Shortcut, TASKBAR_BUTTONS, TASKBAR_POSITIONS, TITLE_DOUBLE_CLICKS, keys_for,
};
use reprisekit::tr;

const CUSTOMIZE_W: i32 = 84;
/// The Taskbar page's first part check box, under the note on what follows the interface.
const CHECKS_Y: i32 = 118;

/// The Alt+Tab chord as the switcher's labels name it.
fn alt_tab() -> String {
    reprisekit::keycap::shown_chord("Alt+Tab")
}

/// How windows snap, tab and come back: the Snapping group holds
/// `[window_snapping]`'s drag snapping and how near an edge it starts (greyed
/// with drag snapping off), the snap layouts on the maximize button, the snap
/// bar a drag shows (greyed too) and the layouts they offer; Tabs and Placement
/// holds `[windows]`, shaking, the title bar double click, dimming and the hidden frame rate
/// included.
pub fn window_behavior(a: Area) -> Form {
    let gw = a.gw();
    let mut items = vec![
        group(a, "snap_group", 44, 172, &tr!("pages-desktop-snapping")),
        Item::check(
            "snap_enabled",
            GX,
            66,
            &tr!("pages-desktop-snap-a-dragged-window-to-screen"),
        ),
        Item::check(
            "snap_picker",
            GX,
            90,
            &tr!("pages-desktop-show-snap-layouts-when-resting-on"),
        ),
        Item::check(
            "snap_bar",
            GX,
            114,
            &tr!("pages-desktop-show-snap-layouts-in-a-bar"),
        ),
    ];
    items.extend(spin(
        "snap_distance_label",
        "snap_distance",
        GX,
        142,
        150,
        &tr!("pages-desktop-snap-distance-from-edges"),
        (1, SNAP_DISTANCE_MAX as i32),
    ));
    items.extend([
        Item::label_at("snap_distance_units", GX + 210, 142, 30, &tr!("units-px")),
        Item::label_at(
            "snap_layouts_label",
            GX,
            170,
            84,
            &tr!("pages-desktop-custom-layouts"),
        ),
        edit("snap_layouts", GX + 88, 166, gw - 88),
        Item::label_at(
            "snap_layouts_note",
            GX,
            192,
            gw,
            &tr!("pages-desktop-empty-for-the-built-in-six"),
        ),
        group(
            a,
            "tabs_group",
            228,
            210,
            &tr!("pages-desktop-tabs-and-placement"),
        ),
        Item::check(
            "window_tabs",
            GX,
            250,
            &tr!("pages-desktop-drop-a-window-on-a-title"),
        ),
        Item::check(
            "remember_windows",
            GX,
            274,
            &tr!("pages-desktop-remember-where-each-programs-window-was"),
        ),
        Item::button(
            FORGET_FRAMES,
            Rect::new(GX, 296, 110, BUTTON_H),
            &tr!("pages-desktop-forget-positions"),
            true,
            false,
        )
        .under_check(),
        Item::check(
            "window_shake",
            GX,
            328,
            &tr!("pages-desktop-shake-a-window-by-its-title"),
        ),
        Item::label_at(
            "title_double_click_label",
            GX,
            356,
            150,
            &tr!("pages-desktop-double-clicking-a-title-bar"),
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
        &tr!("pages-desktop-dim-windows-that-dont-have-the"),
        (0, DIM_INACTIVE_MAX as i32),
    ));
    items.push(Item::label_at("dim_inactive_units", GX + 290, 384, 20, "%"));
    items.extend(spin(
        "hidden_frame_rate_label",
        "hidden_frame_rate",
        GX,
        412,
        170,
        &tr!("pages-desktop-frame-rate-of-hidden-windows"),
        (0, HIDDEN_FRAME_RATE_MAX as i32),
    ));
    items.push(Item::label_at(
        "hidden_frame_rate_units",
        GX + 230,
        412,
        gw - 230,
        &tr!("pages-desktop-0-no-limit"),
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
pub(super) fn motion_note() -> String {
    tr!("pages-desktop-off-while-reduce-motion-is-on")
}
pub(super) fn slide_motion_text() -> String {
    tr!("pages-desktop-reduce-motion")
}

/// The Taskbar page's window picture rate, greyed while nothing shows a live picture.
pub(super) const FPS_CONTROLS: [&str; 3] = ["fps_label", "previews_fps", "fps_units"];

/// The desktops' names, apart by commas.
pub const DESKTOP_NAMES: &str = "desktop_names";

pub fn virtual_desktops(a: Area) -> Form {
    let mut items = vec![group(
        a,
        "desktops_group",
        44,
        244,
        &tr!("pages-desktop-virtual-desktops"),
    )];
    items.push(Item::check(
        "use_desktops",
        GX,
        64,
        &tr!("pages-desktop-use-virtual-desktops"),
    ));
    // Indented under the check box they depend on: `ix` is the box's x, and each item starts at its label's column.
    let ix = GX;
    let mut indented = Vec::new();
    indented.extend(spin(
        "desktops_label",
        "desktops",
        ix,
        88,
        206,
        &tr!("pages-desktop-number-of-desktops"),
        (2, DESKTOPS_MAX as i32),
    ));
    indented.push(Item::label_at(
        "animation_label",
        ix,
        114,
        110,
        &tr!("pages-desktop-switch-animation"),
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
        &tr!("pages-desktop-duration"),
        (0, SWITCH_MS_MAX as i32),
    ));
    indented.push(Item::label_at(
        "duration_units",
        ix + 170,
        140,
        60,
        &tr!("units-ms"),
    ));
    indented.push(Item::label_at(
        "pager_mark_label",
        ix,
        166,
        190,
        &tr!("pages-desktop-current-desktop-on-the-glass-taskbar"),
    ));
    indented.push(choice("pager_mark", ix, 184, 134, PAGER_MARKS));
    indented.push(Item::check(
        "wheel_on_desktop",
        ix,
        214,
        &tr!("pages-desktop-switch-desktops-with-the-wheel-over"),
    ));
    let iw = a.gw() - 18;
    indented.push(Item::label_at(
        "desktop_names_label",
        ix,
        242,
        90,
        &tr!("pages-desktop-desktop-names"),
    ));
    indented.push(edit(DESKTOP_NAMES, ix + 94, 238, iw - 94));
    indented.push(Item::label_at(
        "desktop_names_note",
        ix,
        264,
        iw,
        &tr!("pages-desktop-apart-by-commas-an-empty-one"),
    ));
    items.extend(indented.into_iter().map(Item::under_check));
    items.push(Item::label_at(DESKTOP_MOTION_NOTE, X, 296, a.w, ""));
    Form::new(items)
}

/// The edge the bar sits on, its height and how windows become task buttons, the note that the
/// page follows the interface, then one check box per optional part of the taskbar (Customize...
/// beside the idle icons' row, which stays put on a wide page) with the calendar events' note
/// ([`show_events`]), auto-hide with its three triggers indented under it, the live window
/// pictures and their rate, and the Start button's logo and glow.
pub fn taskbar(a: Area) -> Form {
    let mut items = vec![
        Item::label_at(
            "location_label",
            X,
            48,
            100,
            &tr!("pages-desktop-taskbar-location"),
        ),
        choice("taskbar_position", X + 104, 44, 120, TASKBAR_POSITIONS),
    ];
    items.extend(spin(
        "taskbar_height_label",
        "taskbar_height",
        X + 234,
        48,
        44,
        &tr!("pages-desktop-height"),
        (8, 256),
    ));
    items.extend([
        Item::label_at("taskbar_height_units", X + 338, 48, 20, &tr!("units-px")),
        Item::label_at(
            "buttons_label",
            X,
            76,
            100,
            &tr!("pages-desktop-taskbar-buttons"),
        ),
        choice("taskbar_buttons", X + 104, 72, 230, TASKBAR_BUTTONS),
    ]);
    let checks = [
        (
            "taskbar_small_icons",
            &tr!("pages-desktop-use-small-icons-on-a-shorter"),
        ),
        (
            "taskbar_show_desktop",
            &tr!("pages-desktop-show-desktop-button-at-the-taskbars"),
        ),
        (
            "taskbar_peek",
            &tr!("pages-desktop-peek-at-the-desktop-or-a"),
        ),
        (
            "taskbar_badges",
            &tr!("pages-desktop-show-unread-counts-on-task-buttons"),
        ),
        (
            "taskbar_per_monitor",
            &tr!("pages-desktop-show-each-windows-button-only-on"),
        ),
        (
            "taskbar_tray_overflow",
            &tr!("pages-desktop-hide-idle-notification-icons"),
        ),
        (
            "taskbar_network_meter",
            &tr!("pages-desktop-show-network-speed-beside-the-clock"),
        ),
        (
            "taskbar_slide",
            &tr!("pages-desktop-slide-taskbar-buttons-as-they-are"),
        ),
    ];
    items.push(Item::new(
        "taskbar_note",
        Rect::new(X, 100, a.w, LINE_H),
        Control::Lines(vec![tr!(
            "pages-desktop-left-unchanged-the-settings-on-this"
        )]),
    ));
    let row_y = |i: usize| CHECKS_Y + 18 * i as i32;
    let tray = checks.iter().position(|c| c.0 == "taskbar_tray_overflow");
    let tray_y = row_y(tray.expect("the idle icons have a row"));
    let slide_y = row_y(checks.len() - 1);
    for (i, (id, text)) in checks.into_iter().enumerate() {
        items.push(Item::check(id, X, row_y(i), text));
    }
    items.push(Item::label_at(
        SLIDE_MOTION_NOTE,
        X + 262,
        slide_y,
        a.w - 262,
        "",
    ));
    items.push(Item::check(
        EVENTS_CHECK,
        X,
        slide_y + 18,
        &tr!("pages-desktop-show-calendar-events"),
    ));
    items.push(Item::label_at(
        EVENTS_NOTE,
        X + EVENTS_NOTE_X,
        slide_y + 18,
        a.w - EVENTS_NOTE_X,
        "",
    ));
    items.push(Item::button(
        TRAY_CUSTOMIZE,
        Rect::new(
            Area::MIN.right() - CUSTOMIZE_W,
            tray_y - 5,
            CUSTOMIZE_W,
            BUTTON_H,
        ),
        &tr!("pages-desktop-customize"),
        true,
        false,
    ));
    items.push(Item::check(
        "taskbar_auto_hide",
        X,
        280,
        &tr!("pages-desktop-auto-hide-the-taskbar"),
    ));
    let reveals = [
        &tr!("pages-desktop-show-when-the-pointer-touches-the"),
        &tr!("pages-desktop-show-for-the-start-menu-and"),
        &tr!("pages-desktop-show-when-a-window-needs-attention"),
    ];
    for (i, (id, text)) in REVEAL_CONTROLS.into_iter().zip(reveals).enumerate() {
        items.push(Item::check(id, X, 298 + 18 * i as i32, text).under_check());
    }
    items.push(Item::check(
        "preview_taskbar",
        X,
        352,
        &tr!("pages-desktop-show-live-window-pictures-over-taskbar"),
    ));
    let [fps_label, fps] = spin(
        "fps_label",
        "previews_fps",
        X,
        372,
        110,
        &tr!("pages-desktop-refresh-rate"),
        (1, PREVIEW_FPS_MAX as i32),
    );
    // Under the check, 18 px in: the units' width gives that back to stay on the page.
    let fps_units = Item::label_at(
        "fps_units",
        X + 170,
        372,
        a.w - 188,
        &tr!("pages-desktop-times-a-second-alt-tab-too", keys = alt_tab()),
    );
    items.extend([fps_label, fps, fps_units].map(Item::under_check));
    let glow_w = a.track(150);
    items.extend([
        Item::check(
            "start_icon",
            X,
            394,
            &tr!("pages-desktop-show-the-icon-themes-logo-on"),
        ),
        Item::label_at(
            "orb_glow_label",
            X,
            418,
            110,
            &tr!("pages-desktop-start-button-glow"),
        ),
        Item::new(
            "glass_orb_glow",
            Rect::new(X + 114, 412, glow_w, trackbar::HEIGHT),
            Control::TrackBar(TrackBar::new(100, 101).paged(10, 5)),
        ),
        Item::label_at(
            "orb_glow_note",
            X + GLOW_NOTE_X + glow_w,
            418,
            a.w - GLOW_NOTE_X - glow_w,
            &tr!("pages-desktop-glass-only"),
        ),
    ]);
    Form::new(items)
}

/// The glow note's left edge, less the slider's width.
const GLOW_NOTE_X: i32 = 122;

/// The Start button glow's rows, greyed outside Glass.
pub(super) const GLOW_CONTROLS: [&str; 2] = ["orb_glow_label", "glass_orb_glow"];

/// The auto-hide triggers' check boxes, indented under "Auto-hide the taskbar" and greyed while it is off.
pub(super) const REVEAL_CONTROLS: [&str; 3] = [
    "taskbar_reveal_on_edge",
    "taskbar_reveal_on_keys",
    "taskbar_reveal_on_attention",
];

pub const SEARCH_NOTE: &str = "launcher_search_note";
pub const CONTENTS: &str = "launcher_contents";
pub const CONTENTS_NOTE: &str = "launcher_contents_note";
pub const EVENTS_NOTE: &str = "taskbar_events_note";
const EVENTS_CHECK: &str = "taskbar_calendar_events";
/// The events note's left edge, right of its check box's label.
const EVENTS_NOTE_X: i32 = 160;
pub const FLIP_NOTE: &str = "flip_note";
pub const OVERVIEW_NOTE: &str = "overview_note";
/// The search note's most lines, and the characters a line holds.
const SEARCH_LINES: usize = 3;
const SEARCH_COLS: usize = 55;
/// The characters a line of the indented content note holds.
const CONTENTS_COLS: usize = 52;

/// The Search page: the Search group's note naming the launcher's keys, its checks for files
/// and folders, open windows and contents with the content note under the last, then the Program
/// Results list ([`crate::providerpage`]), which takes the extra height. [`super::show_keys`]
/// fills the first note, [`show_contents`] the second.
pub fn search(a: Area) -> Form {
    let programs_y = 228;
    let list_y = programs_y + 22 + 2 * LINE_H + 4;
    let list_h = providerpage::ROWS as i32 * listbox::ROW_H + 4 + a.dh;
    let list_w = a.gw() - BUTTON_W - 6;
    let programs_end = list_y + list_h + 10;
    let bx = a.gright() - BUTTON_W;
    Form::new(vec![
        group(
            a,
            "launcher_search_group",
            44,
            174,
            &tr!("pages-desktop-search"),
        ),
        Item::new(
            SEARCH_NOTE,
            Rect::new(GX, 66, a.gw(), SEARCH_LINES as i32 * LINE_H),
            Control::Lines(search_note(&[])),
        ),
        Item::check(
            "launcher_files",
            GX,
            117,
            &tr!("pages-desktop-also-find-files-and-folders-in"),
        ),
        Item::check(
            "launcher_windows",
            GX,
            139,
            &tr!("pages-desktop-also-find-open-windows"),
        ),
        Item::check(
            CONTENTS,
            GX,
            161,
            &tr!("pages-desktop-find-files-by-their-contents"),
        ),
        Item::new(
            CONTENTS_NOTE,
            Rect::new(GX + 18, 179, a.gw() - 18, 2 * LINE_H),
            Control::Lines(Vec::new()),
        ),
        group(
            a,
            "search_providers_group",
            programs_y,
            programs_end - programs_y,
            &tr!("pages-desktop-program-results"),
        ),
        Item::new(
            "search_providers_note",
            Rect::new(GX, programs_y + 22, a.gw(), 2 * LINE_H),
            Control::Lines(vec![
                tr!("pages-desktop-a-ticked-program-is-sent-what"),
                tr!("pages-desktop-results-under-its-name-in-this"),
            ]),
        ),
        Item::new(
            providerpage::LIST,
            Rect::new(GX, list_y, list_w, list_h),
            Control::ListBox {
                list: ListBox::new(Vec::new(), None),
                icons: Vec::new(),
            },
        )
        .named(tr!("pages-desktop-program-results")),
        button(providerpage::UP, bx, list_y, &tr!("pages-desktop-move-up")),
        button(
            providerpage::DOWN,
            bx,
            list_y + BUTTON_H + 4,
            &tr!("pages-desktop-move-down"),
        ),
    ])
}

/// The content search note: what turning the check on starts, or greyed with the check while
/// localsearch is not installed; blank until `installed` is read.
pub fn show_contents(page: &mut Form, installed: Option<bool>) {
    let note = match installed {
        None => "",
        Some(true) => &tr!("pages-desktop-the-first-search-starts-localsearch-which"),
        Some(false) => &tr!("pages-desktop-install-localsearch-to-find-files-by"),
    };
    let lines = match note.is_empty() {
        true => Vec::new(),
        false => reprisekit::text::wrap_chars(note, CONTENTS_COLS),
    };
    if let Some(it) = page.item_mut(CONTENTS_NOTE) {
        it.control = match installed {
            Some(false) => Control::GreyedLines(lines),
            _ => Control::Lines(lines),
        };
    }
    let files = page.checked("launcher_files") != Some(false);
    follow_contents(page, files);
}

/// Greys the content search check while file search is off or the note says localsearch is
/// missing.
pub(super) fn follow_contents(page: &mut Form, files: bool) {
    let missing = matches!(
        page.item(CONTENTS_NOTE).map(|i| &i.control),
        Some(Control::GreyedLines(_))
    );
    page.set_enabled(CONTENTS, files && !missing);
}

/// The calendar events' note right of its check box: what to install, greyed with the check box,
/// while evolution-data-server is not installed; blank otherwise.
pub fn show_events(page: &mut Form, installed: Option<bool>) {
    let note = match installed {
        Some(false) => &tr!("pages-desktop-install-evolution-data-server"),
        _ => "",
    };
    page.set_text(EVENTS_NOTE, note);
    page.set_enabled(EVENTS_NOTE, installed != Some(false));
    page.set_enabled(EVENTS_CHECK, installed != Some(false));
}

/// The Search note's lines: the keys bound to the launcher, what it finds, and that the Start
/// menu's search follows the checks. Keys too long for the note drop the Start menu sentence.
pub fn search_note(binds: &[Shortcut]) -> Vec<String> {
    let finds = tr!("pages-desktop-finds-installed-programs-as-you-type");
    let start = tr!("pages-desktop-the-start-menus-search-box-follows");
    let wrap = |body: &str| reprisekit::text::wrap_chars(body, SEARCH_COLS);
    let Some(keys) = keys_for(binds, "launcher") else {
        return wrap(&tr!(
            "pages-desktop-search-no-key",
            finds = finds.clone(),
            start = start.clone()
        ));
    };
    let opens = tr!("pages-desktop-search-opens", keys = keys, finds = finds);
    let mut lines = wrap(&tr!(
        "pages-desktop-search-opens-start",
        opens = opens.clone(),
        start = start
    ));
    if lines.len() > SEARCH_LINES {
        lines = wrap(&opens);
    }
    lines.truncate(SEARCH_LINES);
    lines
}

/// The 3D switcher check's note: the keys bound to it, and that it runs under Glass alone.
pub fn flip_note(binds: &[Shortcut]) -> String {
    let keys = keys_for(binds, "flip").unwrap_or_else(|| tr!("pages-desktop-no-key"));
    tr!("pages-desktop-flip-note", keys = keys)
}

/// The Window Overview note, naming the keys bound to the overview.
pub fn overview_note(binds: &[Shortcut]) -> String {
    keys_for(binds, "overview").map_or_else(
        || tr!("pages-desktop-every-window-of-this-desktop-side"),
        |k| tr!("pages-desktop-overview-keys", keys = k),
    )
}

/// The Window Switcher page: what Alt+Tab shows and how large its pictures
/// are, the desktop entry and peek with it, the 3D switcher, and the window
/// overview's hot corner under it. [`super::show_keys`] fills the notes.
pub fn switcher(a: Area) -> Form {
    Form::new(vec![
        group(
            a,
            "switcher_group",
            44,
            186,
            &tr!("pages-desktop-alt-tab", keys = alt_tab()),
        ),
        Item::label_at(
            "switcher_style_label",
            GX,
            68,
            110,
            &tr!("pages-desktop-alt-tab-shows", keys = alt_tab()),
        ),
        choice("switcher_style", GX + 114, 64, 130, SWITCHER_STYLES),
        Item::label_at(
            "switcher_size_label",
            GX,
            94,
            110,
            &tr!("pages-desktop-picture-size"),
        ),
        choice("switcher_size", GX + 114, 90, 130, SWITCHER_SIZES),
        Item::check(
            "switcher_preview",
            GX,
            118,
            &tr!("pages-desktop-show-a-live-preview-of-the"),
        ),
        Item::check(
            "switcher_desktop",
            GX,
            140,
            &tr!(
                "pages-desktop-show-the-desktop-in-alt-tab",
                keys = alt_tab()
            ),
        ),
        Item::check(
            "switcher_peek",
            GX,
            162,
            &tr!("pages-desktop-peek-at-the-selected-window"),
        ),
        Item::check(
            "switcher_flip",
            GX,
            184,
            &tr!("pages-desktop-show-windows-in-3d"),
        ),
        Item::label_at(FLIP_NOTE, GX + 18, 202, a.gw() - 18, &flip_note(&[])),
        group(
            a,
            "overview_group",
            240,
            76,
            &tr!("pages-desktop-window-overview"),
        ),
        Item::new(
            OVERVIEW_NOTE,
            Rect::new(GX, 262, a.gw(), LINE_H),
            Control::Lines(vec![overview_note(&[])]),
        ),
        Item::label_at(
            "hot_corner_label",
            GX,
            288,
            110,
            &tr!("pages-desktop-hot-corner"),
        ),
        choice("hot_corner", GX + 114, 284, 130, HOT_CORNERS),
    ])
}
