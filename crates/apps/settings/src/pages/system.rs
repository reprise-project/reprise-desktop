//! Sounds, Power, Startup, Advanced and Backup.

use super::{
    ADD, Area, BACKUP_EXPORT, BACKUP_IMPORT, BACKUP_STATUS, DEBUG_IN_CAPTURES, DEBUG_OVERLAY,
    EDIT_FILE, FORGET, GX, PRIVACY_LOCATION, REMOVE, THUMBNAILERS, THUMBS, THUMBS_CLEAR,
    THUMBS_CLEARED, THUMBS_MB, THUMBS_SIZE, WATER, WATER_STYLE, X, button, choice, combo, edit,
    field, group, list, locations, spin,
};
use crate::model::WATER_STYLES;
use crate::notifypage;
use reprisebus::charge::ChargeLimit;
use repriseconf::ThumbnailCache;
use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_W, LINE_H};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::listbox::{self, ListBox};
use reprisekit::raster::Rect;

/// The task manager, the switches with no page of their own, the location
/// answers programs were given (a [`ListView`](reprisekit::controls::listview::ListView) the host draws over the page at
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
        "Task &manager (empty runs Task Manager):",
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
        278 + dh,
        "Show a &debug overlay (frame rate, render times)",
    ));
    items.push(
        Item::check(
            DEBUG_IN_CAPTURES,
            X,
            300 + dh,
            "Show it in screenshots and screen &sharing",
        )
        .under_check(),
    );
    items.push(Item::check(WATER, X, 322 + dh, "Desktop &water"));
    items.push(choice(WATER_STYLE, X + 112, 318 + dh, 100, WATER_STYLES));
    items.push(Item::label_at(
        "water_note",
        X + 12,
        342 + dh,
        a.w - 12,
        "With Ink, Glow, Sparks or Neon, dragging on the desktop",
    ));
    items.push(Item::label_at(
        "water_note_2",
        X + 12,
        355 + dh,
        a.w - 12,
        "stirs the fluid instead of drawing a selection box.",
    ));
    items.push(Item::check(
        THUMBS,
        X,
        370 + dh,
        "Keep &thumbnails of pictures",
    ));
    let mb = ThumbnailCache::MAX_MB;
    items.extend(
        spin(
            THUMBS_SIZE[0],
            THUMBS_MB,
            X,
            396 + dh,
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
        370 + dh,
        a.right() - cleared_x,
        "",
    ));
    items.push(button(
        THUMBS_CLEAR,
        a.right() - BUTTON_W,
        390 + dh,
        "Clea&r",
    ));
    items.push(Item::check(
        THUMBNAILERS,
        X,
        418 + dh,
        "Ma&ke thumbnails of documents and videos",
    ));
    items.push(Item::new(
        "note",
        Rect::new(X, 440 + dh, a.w - BUTTON_W - 8, 2 * LINE_H),
        Control::Lines(vec![
            "Keys no page sets are edited in".into(),
            "config.toml itself.".into(),
        ]),
    ));
    items.push(button(
        EDIT_FILE,
        a.right() - BUTTON_W,
        440 + dh,
        "&Edit File...",
    ));
    Form::new(items)
}

/// What Export and Import carry, one line per [`ENTRIES`](crate::backup::ENTRIES) row, what they
/// leave out, the two buttons and the line saying what the last one did.
pub fn backup(a: Area) -> Form {
    let entries = crate::backup::ENTRIES;
    let list_h = LINE_H * entries.len() as i32;
    let note_y = 66 + list_h + 10;
    let buttons_y = note_y + 3 * LINE_H + 10;
    Form::new(vec![
        Item::label_at(
            "backup_intro",
            X,
            44,
            a.w,
            "Export saves these in one file; Import puts them back:",
        ),
        Item::new(
            "backup_list",
            Rect::new(GX, 66, a.w - 12, list_h),
            Control::Lines(entries.iter().map(|e| e.what.to_string()).collect()),
        ),
        Item::new(
            "backup_left_out",
            Rect::new(X, note_y, a.w, 3 * LINE_H),
            Control::Lines(vec![
                "Not included: the programs a session reopens, the Run".into(),
                "and clipboard histories, and the location answers given".into(),
                "on this computer.".into(),
            ]),
        ),
        button(BACKUP_EXPORT, X, buttons_y, "&Export..."),
        button(
            BACKUP_IMPORT,
            X + BUTTON_W + BUTTON_GAP,
            buttons_y,
            "&Import...",
        ),
        Item::new(
            BACKUP_STATUS,
            Rect::new(X, buttons_y + 34, a.w, 2 * LINE_H),
            Control::Lines(Vec::new()),
        ),
    ])
}

/// The sound theme picked from `sound_themes`, the events that play from it, the volume's reach,
/// then each program's notification switches ([`notifypage`]). The program list takes the page's
/// extra height.
pub fn sounds(a: Area, sound_themes: &[String]) -> Form {
    Form::new(vec![
        group(a, "sound_theme_group", 44, 52, "Sound Theme"),
        Item::label_at(SOUND_CONTROLS[0], GX, 70, 80, "Sound th&eme:"),
        combo(
            SOUND_CONTROLS[1],
            GX + 84,
            66,
            a.combo(200),
            sound_themes,
            sound_themes.len().clamp(1, 8),
        ),
        group(a, "sound_events_group", 106, 74, "Play Sounds"),
        Item::check(
            NOTIFICATION_SOUND,
            GX,
            128,
            "&Play a sound for notifications",
        ),
        Item::check(BELL, GX, 154, "Play a &sound when a program rings the bell"),
        group(a, "volume_group", 190, 74, "Volume"),
        Item::check(ALLOW_BOOST, GX, 212, "Allow volume &above 100%"),
        Item::label_at(
            "allow_boost_note",
            GX + 18,
            232,
            a.gw() - 18,
            "Sliders and the volume keys go on to 150%.",
        ),
        group(a, "notify_group", 274, 144 + a.dh, "Program Notifications"),
        Item::new(
            notifypage::PROGRAMS,
            Rect::new(
                GX,
                294,
                a.gw(),
                notifypage::ROWS as i32 * listbox::ROW_H + 4 + a.dh,
            ),
            Control::ListBox {
                list: ListBox::new(Vec::new(), None),
                icons: Vec::new(),
            },
        ),
        Item::check(notifypage::BALLOONS, GX, 391 + a.dh, "Show &balloons").enabled(false),
        Item::check(notifypage::SOUNDS, GX + 140, 391 + a.dh, "Play s&ounds").enabled(false),
    ])
}

pub(super) const NOTIFICATION_SOUND: &str = "notification_sound";
pub(super) const BELL: &str = "bell";
pub(super) const ALLOW_BOOST: &str = "allow_boost";

/// The sound theme's label and combo, greyed while nothing plays a sound.
pub(super) const SOUND_CONTROLS: [&str; 2] = ["sound_theme_label", "sound_theme"];

/// The battery charge limit's check box, there only while UPower offers a limit.
pub const CHARGE_LIMIT: &str = "charge_limit";

pub fn charge_label(limit: &ChargeLimit) -> String {
    match limit.end {
        Some(end) => format!("Limit charging to protect the &battery (stops at {end}%)"),
        None => "Limit charging to protect the &battery".to_string(),
    }
}

/// The Power page; `charge` adds the charge limit's check box under Low Disk Space.
pub fn power(a: Area, charge: Option<&ChargeLimit>) -> Form {
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

    items.push(group(a, "storage_group", 388, 74, "Low Disk Space"));
    items.extend(spin(
        "low_space_label",
        "low_space_percent",
        GX,
        412,
        114,
        "Warn bel&ow:",
        (0, repriseconf::LOW_SPACE_MAX.into()),
    ));
    items.push(Item::label_at(
        "low_space_units",
        GX + 174,
        412,
        gw - 174,
        "% free",
    ));
    items.push(Item::label_at(
        "low_space_note",
        GX,
        438,
        gw,
        "For the disks of your home folder and /. 0 turns it off.",
    ));
    if let Some(limit) = charge {
        items.push(Item::check(CHARGE_LIMIT, X, 472, &charge_label(limit)).checked(limit.enabled));
    }
    Form::new(items)
}

/// The Startup page: the autostart list (a [`ListView`](reprisekit::controls::listview::ListView) the host draws over
/// the page and routes the pointer to, see `main.rs`), its two buttons, and
/// the Session group: what the next login gets back of this one, and what Log
/// Off ends, then whether Software Updates says when the system needs a restart. The list takes the
/// page's extra height.
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
        group(a, "updates_group", 368 + dh, 44, "Software Updates"),
        Item::check(
            "restart_notice",
            GX,
            390 + dh,
            "&Tell me when an update needs a restart",
        ),
    ])
}
