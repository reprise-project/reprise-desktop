//! Power, Startup and Advanced.

use super::{
    ADD, Area, DEBUG_IN_CAPTURES, DEBUG_OVERLAY, EDIT_FILE, FORGET, GX, PRIVACY_LOCATION, REMOVE,
    THUMBS, THUMBS_CLEAR, THUMBS_CLEARED, THUMBS_MB, THUMBS_SIZE, WATER, WATER_STYLE, X, button,
    choice, edit, field, group, list, locations, spin,
};
use crate::model::WATER_STYLES;
use repriseconf::ThumbnailCache;
use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_W, LINE_H};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::raster::Rect;

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
