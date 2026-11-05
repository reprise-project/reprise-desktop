//! Sounds, Power, Startup, Remote Desktop, Advanced and Backup.

use super::{
    ADD, Area, BACKUP_EXPORT, BACKUP_IMPORT, BACKUP_STATUS, DEBUG_IN_CAPTURES, DEBUG_OVERLAY,
    EDIT_FILE, FORGET, GX, PRIVACY_LOCATION, REMOVE, THUMBNAILERS, THUMBS, THUMBS_CLEAR,
    THUMBS_CLEARED, THUMBS_MB, THUMBS_SIZE, WATER, WATER_STYLE, X, button, choice, combo, edit,
    ends_track, field, group, list, locations, spin,
};
use crate::model::{FRAME_RATES, REMOTE_ENCODERS, REMOTE_QUALITIES, WATER_STYLES};
use crate::notifypage;
use reprisebus::remote::{Client, Status};
use repriseconf::ThumbnailCache;
use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_W, LINE_H};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::listbox::{self, ListBox};
use reprisekit::raster::Rect;
use reprisekit::trackbar::TrackBar;

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
            40,
            "Si&ze:",
            (*mb.start() as i32, *mb.end() as i32),
        )
        .map(Item::under_check),
    );
    items.push(Item::label_at(THUMBS_SIZE[2], X + 98, 396 + dh, 20, "MB").under_check());
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

/// The Remote Desktop page's switch, acted on at once rather than through Apply.
pub const REMOTE_ALLOW: &str = "allow";
const REMOTE_STATUS: &str = "status";
const ENCODER: &str = "encoder";
pub(super) const REMOTE_PORT: &str = "port";
pub(super) const REMOTE_QUALITY: &str = "quality";
pub(super) const QUALITY_VALUE: &str = "quality_value";
pub(super) const REMOTE_FRAME_RATE: &str = "frame_rate";
pub(super) const REMOTE_ENCODER: &str = "video_encoder";
pub(super) const REMOTE_SOUND: &str = "remote_sound";
/// The certificate's fingerprint: a read-only edit, so it can be selected and copied.
const REMOTE_FINGERPRINT: &str = "fingerprint";
pub const REMOTE_REGENERATE: &str = "regenerate";
const CLIENTS: &str = "clients";
pub const REMOTE_DISCONNECT: &str = "disconnect";

/// What greys with the switch off; New Certificate and Disconnect also need the server answering.
const REMOTE_CONTROLS: [&str; 16] = [
    REMOTE_STATUS,
    ENCODER,
    "port_label",
    REMOTE_PORT,
    "quality_label",
    "quality_speed",
    REMOTE_QUALITY,
    "quality_sharp",
    QUALITY_VALUE,
    "frame_rate_label",
    REMOTE_FRAME_RATE,
    "video_encoder_label",
    REMOTE_ENCODER,
    REMOTE_SOUND,
    "fingerprint_label",
    REMOTE_FINGERPRINT,
];

const NEW_CERTIFICATE_W: i32 = 110;

/// The Remote Desktop page as `status` and `enabled` (the server's unit) leave it; the host
/// follows the server with [`show_remote`].
pub fn remote_desktop(a: Area, status: Option<&Status>, enabled: bool) -> Form {
    let gw = a.gw();
    let mut items = vec![
        Item::check(
            REMOTE_ALLOW,
            X,
            44,
            "Allow remote &connections to this computer",
        ),
        Item::label_at(REMOTE_STATUS, X, 64, a.w - 18, "").under_check(),
        Item::label_at(ENCODER, X, 80, a.w - 18, "").under_check(),
        group(a, "connection_group", 104, 183, "Connection"),
    ];
    items.extend(spin(
        "port_label",
        REMOTE_PORT,
        GX,
        128,
        114,
        "&Port:",
        (1024, 65535),
    ));
    items.push(Item::label_at(
        "quality_label",
        GX,
        154,
        200,
        "Picture &quality:",
    ));
    let (left, right) = (REMOTE_QUALITIES[0].1, REMOTE_QUALITIES[2].1);
    items.extend(ends_track(
        a,
        [
            "quality_speed",
            REMOTE_QUALITY,
            "quality_sharp",
            QUALITY_VALUE,
        ],
        "Picture quality",
        181,
        27,
        (left, right),
        GX + 46,
        TrackBar::new(1, REMOTE_QUALITIES.len()),
    ));
    items.push(Item::label_at(
        "frame_rate_label",
        GX,
        214,
        114,
        "Frame &rate:",
    ));
    let rates: Vec<&str> = FRAME_RATES.iter().map(|r| r.1).collect();
    items.push(Item::combo(
        REMOTE_FRAME_RATE,
        GX + 118,
        210,
        160,
        &rates,
        rates.len(),
    ));
    items.push(Item::label_at(
        "video_encoder_label",
        GX,
        240,
        114,
        "Video &encoder:",
    ));
    let encoders: Vec<&str> = REMOTE_ENCODERS.iter().map(|e| e.1).collect();
    items.push(Item::combo(
        REMOTE_ENCODER,
        GX + 118,
        236,
        160,
        &encoders,
        encoders.len(),
    ));
    items.push(Item::check(
        REMOTE_SOUND,
        GX,
        266,
        "Play &sound on the remote computer",
    ));
    items.push(group(a, "certificate_group", 297, 66, "Certificate"));
    items.push(Item::label_at(
        "fingerprint_label",
        GX,
        317,
        gw,
        "&Fingerprint (SHA-256):",
    ));
    items.push(edit(REMOTE_FINGERPRINT, GX, 333, gw - NEW_CERTIFICATE_W - 6).read_only());
    items.push(Item::button_at(
        REMOTE_REGENERATE,
        a.gright() - NEW_CERTIFICATE_W,
        331,
        NEW_CERTIFICATE_W,
        "&New Certificate...",
        true,
    ));
    items.push(group(a, "client_group", 373, 102, "Connected Computers"));
    items.push(Item::new(
        CLIENTS,
        Rect::new(GX, 393, gw - BUTTON_W - 6, 4 * listbox::ROW_H + 4),
        Control::ListBox {
            list: ListBox::new(Vec::new(), None),
            icons: Vec::new(),
        },
    ));
    items.push(button(
        REMOTE_DISCONNECT,
        a.gright() - BUTTON_W,
        393,
        "&Disconnect",
    ));
    let mut page = Form::new(items);
    show_remote(&mut page, status, enabled, None, None);
    page
}

/// Put the server's state on the page in place, so an open drop-down survives an unchanged poll;
/// encoders the server lacks grey. Changed clients keep `keep` selected, else the first.
pub fn show_remote(
    page: &mut Form,
    status: Option<&Status>,
    enabled: bool,
    address: Option<&str>,
    keep: Option<&str>,
) {
    page.set_check(REMOTE_ALLOW, enabled);
    page.set_enabled(REMOTE_ALLOW, true);
    for id in REMOTE_CONTROLS {
        page.set_enabled(id, enabled);
    }
    page.set_text(REMOTE_STATUS, &remote_status(status, enabled, address));
    let shown = status.filter(|_| enabled);
    let encoder = shown.map_or(String::new(), |s| format!("Video: {}", s.encoder));
    page.set_text(ENCODER, &encoder);
    let offered = shown.map_or(&[][..], |s| s.encoders.as_slice());
    if let Some(Control::Combo(c)) = page.item_mut(REMOTE_ENCODER).map(|i| &mut i.control) {
        for (item, (value, _)) in c.items.iter_mut().zip(REMOTE_ENCODERS) {
            item.greyed = *value != "auto" && !offered.iter().any(|o| o == value);
        }
    }
    let fingerprint = shown.map_or("", |s| s.fingerprint.as_str());
    if let Some(Control::Edit { edit, first, .. }) =
        page.item_mut(REMOTE_FINGERPRINT).map(|i| &mut i.control)
        && edit.text != fingerprint
    {
        edit.set_selected(fingerprint);
        edit.set_selection(0, 0);
        first.set(0);
    }
    let clients = shown.map_or(&[][..], |s| s.clients.as_slice());
    let items: Vec<String> = clients
        .iter()
        .map(|c| client_address(&c.address, clients))
        .collect();
    let detail: Vec<String> = clients
        .iter()
        .map(|c| {
            let since = reprisefs::listing::date_text(c.since as i64, reprisefs::locale::is_12h());
            format!("{}, since {since}", c.user)
        })
        .collect();
    let mut picked = false;
    if let Some(Control::ListBox { list, .. }) = page.item_mut(CLIENTS).map(|i| &mut i.control) {
        if list.items != items || list.detail != detail {
            let kept = keep.and_then(|a| clients.iter().position(|c| c.address == a));
            let first = (!clients.is_empty()).then_some(0);
            *list = ListBox::new(items, kept.or(first));
            list.detail = detail;
        }
        picked = list.selected.is_some();
    }
    page.set_enabled(REMOTE_REGENERATE, shown.is_some());
    page.set_enabled(REMOTE_DISCONNECT, picked);
}

/// The address of the client selected on the page, as `status` lists it.
pub fn selected_client<'a>(page: &Form, status: Option<&'a Status>) -> Option<&'a str> {
    let Control::ListBox { list, .. } = &page.item(CLIENTS)?.control else {
        return None;
    };
    let client = status?.clients.get(list.selected?)?;
    Some(client.address.as_str())
}

/// The status line: off, not answering, listening on its port at `address`, or why it cannot listen.
pub(super) fn remote_status(
    status: Option<&Status>,
    enabled: bool,
    address: Option<&str>,
) -> String {
    let Some(s) = status.filter(|_| enabled) else {
        return if enabled { "Not answering" } else { "Off" }.into();
    };
    if let Some(why) = s.listening.strip_prefix("error: ") {
        return format!("Not listening: {why}");
    }
    let port = s.listening.rsplit_once(':').map_or("", |(_, p)| p);
    match address {
        Some(at) => format!("Listening on port {port} at {at}"),
        None => format!("Listening on port {port}"),
    }
}

/// A client's address, without its port unless another of `clients` has the same IP.
fn client_address(address: &str, clients: &[Client]) -> String {
    let ip = |a: &str| a.parse::<std::net::SocketAddr>().ok().map(|a| a.ip());
    let Some(own) = ip(address) else {
        return address.to_string();
    };
    let shared = clients
        .iter()
        .filter(|c| ip(&c.address) == Some(own))
        .count()
        > 1;
    if shared {
        address.to_string()
    } else {
        own.to_string()
    }
}
