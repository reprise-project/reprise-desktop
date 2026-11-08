//! Sound, Notifications, Startup, Remote Desktop, Privacy, Advanced and Backup.

use super::{
    ADD, AUDIO_DEVICES, Area, BACKUP_EXPORT, BACKUP_IMPORT, BACKUP_STATUS, DEBUG_IN_CAPTURES,
    DEBUG_OVERLAY, EDIT_FILE, FORGET, GX, PRIVACY_LOCATION, REMOVE, THUMBNAILERS, THUMBS,
    THUMBS_CLEAR, THUMBS_CLEARED, THUMBS_MB, THUMBS_SIZE, X, button, choice, combo, edit,
    ends_track, field, group, list, locations, radio, spin, time,
};
use crate::model::{
    FRAME_RATES, GLASS_NOTIFY_STYLES, LOCK_GRACES, REMOTE_ENCODERS, REMOTE_QUALITIES, Shortcut,
    keys_for,
};
use crate::notifypage;
use reprisebus::remote::{Client, Status};
use repriseconf::ThumbnailCache;
use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_W, LINE_H};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::listbox::{self, ListBox};
use reprisekit::raster::Rect;
use reprisekit::trackbar::TrackBar;

/// The task manager, the switches with no page of their own, the Turn Off commands, the low
/// disk space warning, the thumbnail cache, the debug overlay and the config file itself. The
/// terminal is picked on Default Programs. Nothing on the page grows with its height.
pub fn advanced(a: Area) -> Form {
    let gw = a.gw();
    let mut items = Vec::new();
    items.extend(field(
        "task_manager_label",
        "task_manager",
        X,
        44,
        a.w,
        "Task &manager (empty runs Task Manager):",
    ));
    items.push(Item::check(
        "screenshot_save",
        X,
        90,
        "Save each screenshot as a file in &Pictures/Screenshots",
    ));
    items.push(Item::check(
        "restart_notice",
        X,
        108,
        "Tell me when an &update needs a restart",
    ));

    let half = (gw - 8) / 2;
    let command = |label_id, id, x, y, text| {
        [
            Item::label_at(label_id, x, y, 60, text),
            edit(id, x + 62, y - 4, half - 62),
        ]
    };
    let right = GX + half + 8;
    items.push(group(a, "turnoff_group", 130, 74, "Turn Off Computer"));
    items.extend(command("standby_label", "standby", GX, 152, "Stand &by:"));
    items.extend(command(
        "poweroff_label",
        "poweroff",
        right,
        152,
        "Tur&n off:",
    ));
    items.extend(command("reboot_label", "reboot", GX, 178, "Rest&art:"));
    items.extend(command(
        "hibernate_label",
        "hibernate",
        right,
        178,
        "&Hibernate:",
    ));

    items.push(group(a, "storage_group", 212, 66, "Low Disk Space"));
    items.extend(spin(
        "low_space_label",
        "low_space_percent",
        GX,
        234,
        114,
        "Warn bel&ow:",
        (0, repriseconf::LOW_SPACE_MAX.into()),
    ));
    items.push(Item::label_at(
        "low_space_units",
        GX + 174,
        234,
        gw - 174,
        "% free",
    ));
    items.push(Item::label_at(
        "low_space_note",
        GX,
        254,
        gw,
        "For the disks of your home folder and /. 0 turns it off.",
    ));

    items.push(group(a, "thumbs_group", 286, 92, "Thumbnails"));
    items.push(Item::check(THUMBS, GX, 308, "Keep &thumbnails of pictures"));
    let cleared_x = GX + 184;
    items.push(Item::value_at(
        THUMBS_CLEARED,
        cleared_x,
        308,
        a.gright() - cleared_x,
        "",
    ));
    items.push(button(THUMBS_CLEAR, a.gright() - BUTTON_W, 326, "Clea&r"));
    let mb = ThumbnailCache::MAX_MB;
    items.extend(
        spin(
            THUMBS_SIZE[0],
            THUMBS_MB,
            GX,
            332,
            40,
            "Si&ze:",
            (*mb.start() as i32, *mb.end() as i32),
        )
        .map(Item::under_check),
    );
    items.push(Item::label_at(THUMBS_SIZE[2], GX + 98, 332, 20, "MB").under_check());
    items.push(Item::check(
        THUMBNAILERS,
        GX,
        356,
        "Ma&ke thumbnails of documents and videos",
    ));

    items.push(Item::check(
        DEBUG_OVERLAY,
        X,
        386,
        "Show a &debug overlay (frame rate, render times)",
    ));
    items.push(
        Item::check(
            DEBUG_IN_CAPTURES,
            X,
            404,
            "Show it in screenshots and screen &sharing",
        )
        .under_check(),
    );

    items.push(Item::new(
        "note",
        Rect::new(X, 422, a.w - BUTTON_W - 8, 2 * LINE_H),
        Control::Lines(vec![
            "Keys no page sets are edited in".into(),
            "config.toml itself.".into(),
        ]),
    ));
    items.push(button(
        EDIT_FILE,
        a.right() - BUTTON_W,
        422,
        "&Edit File...",
    ));
    Form::new(items)
}

pub(super) const CLIPBOARD_HISTORY: &str = "clipboard_history";

/// The clipboard history check's label, naming the keys that open the history.
pub(super) fn clipboard_label(binds: &[Shortcut]) -> String {
    match keys_for(binds, "clipboard-history") {
        Some(k) => format!(
            "Keep a &clipboard history ({} opens it)",
            reprisekit::text::escape_mnemonic(&k)
        ),
        None => "Keep a &clipboard history".into(),
    }
}

/// The location check, its answers (a `ListView` the host draws at [`locations`]) with
/// Forget, the recent files and clipboard histories, and the border around a shared screen.
pub fn privacy(a: Area) -> Form {
    let dh = a.dh;
    Form::new(vec![
        group(a, "location_group", 44, 168 + dh, "Location"),
        Item::check(
            PRIVACY_LOCATION,
            GX,
            66,
            "Let programs ask for your &location",
        ),
        button(FORGET, a.gright() - BUTTON_W, locations(a).y, "F&orget"),
        group(a, "history_group", 222 + dh, 70, "History"),
        Item::check(
            "recent_files",
            GX,
            244 + dh,
            "Show recently used &files in menus and file dialogs",
        ),
        Item::check(CLIPBOARD_HISTORY, GX, 266 + dh, &clipboard_label(&[])),
        group(a, "sharing_group", 302 + dh, 48, "Screen Sharing"),
        Item::check(
            "capture_border",
            GX,
            324 + dh,
            "Show a red &border around what is being shared",
        ),
    ])
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

/// The button that opens Sounds and Audio Devices, the sound theme picked from `sound_themes`, the bell and
/// the volume's reach.
pub fn sound(a: Area, sound_themes: &[String]) -> Form {
    Form::new(vec![
        Item::new(
            "audio_devices_note",
            Rect::new(X, 44, a.w - 118, 2 * LINE_H),
            Control::Lines(vec![
                "To pick speakers and microphones or".into(),
                "set each program's volume, open".into(),
            ]),
        ),
        Item::button_at(
            AUDIO_DEVICES,
            a.right() - 110,
            44,
            110,
            "Au&dio Devices...",
            true,
        ),
        group(a, "sound_theme_group", 80, 52, "Sound Theme"),
        Item::label_at(SOUND_CONTROLS[0], GX, 106, 80, "Sound th&eme:"),
        combo(
            SOUND_CONTROLS[1],
            GX + 84,
            102,
            a.combo(200),
            sound_themes,
            sound_themes.len().clamp(1, 8),
        ),
        group(a, "sound_events_group", 142, 48, "Play Sounds"),
        Item::check(BELL, GX, 164, "Play a &sound when a program rings the bell"),
        group(a, "volume_group", 200, 74, "Volume"),
        Item::check(ALLOW_BOOST, GX, 222, "Allow volume &above 100%"),
        Item::label_at(
            "allow_boost_note",
            GX + 18,
            242,
            a.gw() - 18,
            "Sliders and the volume keys go on to 150%.",
        ),
    ])
}

/// The notification sound and balloon style, Do Not Disturb's triggers, then each program's
/// switches ([`notifypage`]). The program list takes the page's extra height.
pub fn notifications(a: Area) -> Form {
    let mut items = vec![
        group(a, "balloons_group", 44, 92, "Balloons"),
        Item::check(
            NOTIFICATION_SOUND,
            GX,
            66,
            "&Play a sound for notifications",
        ),
        Item::label_at("notify_style_label", GX, 92, 84, "Balloon st&yle:"),
        choice(
            "glass_notification_style",
            GX + 88,
            88,
            120,
            GLASS_NOTIFY_STYLES,
        ),
        Item::label_at(
            notifypage::STYLE_NOTE,
            GX,
            114,
            a.gw(),
            "Only Glass draws balloons in a style.",
        ),
        group(a, "dnd_group", 146, 116, "Do Not Disturb"),
        Item::check(
            notifypage::AUTO_DND,
            GX,
            168,
            "Hold balloons during &full screen and screen sharing",
        ),
        Item::check(
            notifypage::QUIET,
            GX,
            192,
            "Hold balloons during &quiet hours",
        ),
    ];
    let [from_label, from, to_label, to] = notifypage::QUIET_TIMES;
    let times = time(from_label, from, GX, 216, 36, "F&rom:")
        .into_iter()
        .chain(time(to_label, to, GX + 132, 216, 24, "&To:"));
    items.extend(times.map(Item::under_check));
    items.extend([
        Item::label_at(
            "quiet_note",
            GX,
            242,
            a.gw(),
            "Held balloons show afterwards; critical ones always show.",
        ),
        group(a, "notify_group", 272, 144 + a.dh, "Program Notifications"),
        Item::new(
            notifypage::PROGRAMS,
            Rect::new(
                GX,
                292,
                a.gw(),
                notifypage::ROWS as i32 * listbox::ROW_H + 4 + a.dh,
            ),
            Control::ListBox {
                list: ListBox::new(Vec::new(), None),
                icons: Vec::new(),
            },
        )
        .named("Program Notifications"),
        Item::check(notifypage::BALLOONS, GX, 389 + a.dh, "Show &balloons").enabled(false),
        Item::check(notifypage::SOUNDS, GX + 140, 389 + a.dh, "Play s&ounds").enabled(false),
    ]);
    Form::new(items)
}

pub(super) const NOTIFICATION_SOUND: &str = "notification_sound";
pub(super) const BELL: &str = "bell";
pub(super) const ALLOW_BOOST: &str = "allow_boost";

/// The sound theme's label and combo.
pub(super) const SOUND_CONTROLS: [&str; 2] = ["sound_theme_label", "sound_theme"];

/// The Startup page: the autostart list (a [`ListView`](reprisekit::controls::listview::ListView) the host draws over
/// the page and routes the pointer to, see `main.rs`), its two buttons, and
/// the Session group: what the next login gets back of this one, and what Log
/// Off ends. The list takes the page's extra height.
pub fn startup(a: Area) -> Form {
    let list = list(a);
    let (y, dh) = (list.bottom() + 8, a.dh);
    Form::new(vec![
        button(ADD, list.right() - 2 * BUTTON_W - BUTTON_GAP, y, "&Add..."),
        button(REMOVE, list.right() - BUTTON_W, y, "&Remove"),
        group(a, "session_group", 343 + dh, 66, "Session"),
        Item::check(
            "session_restore",
            GX,
            365 + dh,
            "Re&open the programs that were open at the last log off",
        ),
        Item::check(
            "session_end_processes",
            GX,
            387 + dh,
            "&End the programs still running when I log off",
        ),
    ])
}

/// The Remote Desktop page's switch, acted on at once rather than through Apply.
pub const REMOTE_ALLOW: &str = "allow";
const REMOTE_NOTE: &str = "allow_note";
const REMOTE_STATUS: &str = "status";
const ENCODER: &str = "encoder";
pub(super) const REMOTE_PORT: &str = "port";
pub(super) const REMOTE_QUALITY: &str = "quality";
pub(super) const QUALITY_VALUE: &str = "quality_value";
pub(super) const REMOTE_FRAME_RATE: &str = "frame_rate";
pub(super) const REMOTE_ENCODER: &str = "video_encoder";
pub(super) const REMOTE_SOUND: &str = "remote_sound";
pub(super) const REMOTE_LOCK_ON_LEAVE: &str = "remote_lock";
pub(super) const REMOTE_LOCK_GRACE: &str = "lock_grace";
/// What greys with the lock check box off.
const LOCK_GRACE_CONTROLS: [&str; 2] = ["lock_grace_label", REMOTE_LOCK_GRACE];
pub(super) const REMOTE_BORDER: &str = "remote_border";
pub(super) const REMOTE_SCREEN_SHOW: &str = "remote_screen_show";
pub(super) const REMOTE_SCREEN_LOCK: &str = "remote_screen_lock";
/// Says why the Lock radio is greyed; empty while no external locker is set.
pub(super) const REMOTE_SCREEN_NOTE: &str = "remote_screen_note";
pub(super) const REMOTE_SECOND_JOIN: &str = "second_client_join";
pub(super) const REMOTE_SECOND_REPLACE: &str = "second_client_replace";
/// Stands in for [`REMOTE_ALLOW`] while the login manager runs remote logins.
const MANAGED_NOTE: &str = "managed_note";
pub(super) const LOCKER_NOTE: &str = "Lock mode is off while an external screen locker is set.";
/// The certificate's fingerprint: a read-only edit, so it can be selected and copied.
const REMOTE_FINGERPRINT: &str = "fingerprint";
pub const REMOTE_REGENERATE: &str = "regenerate";
const CLIENTS: &str = "clients";
pub const REMOTE_DISCONNECT: &str = "disconnect";
/// Opens User Accounts on your Remote Desktop password, on a [`managed`] page.
pub const RD_PASSWORD_ACCOUNTS: &str = "rd_password_accounts";
/// Top of the Remote Desktop password row on a [`managed`] page.
const RD_PASSWORD_Y: i32 = 62;
const ACCOUNTS_W: i32 = 110;

/// What greys with the switch off; New Certificate and Disconnect also need the server answering.
const REMOTE_CONTROLS: [&str; 24] = [
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
    REMOTE_LOCK_ON_LEAVE,
    REMOTE_BORDER,
    "remote_screen_label",
    REMOTE_SCREEN_SHOW,
    REMOTE_SCREEN_LOCK,
    "fingerprint_label",
    REMOTE_FINGERPRINT,
    "second_client_label",
    REMOTE_SECOND_JOIN,
    REMOTE_SECOND_REPLACE,
];

/// What greys while the login manager's port is used.
const PORT_CONTROLS: [&str; 2] = ["port_label", REMOTE_PORT];

const NEW_CERTIFICATE_W: i32 = 110;
/// Where the second client's radios start, right of their label.
const SECOND_CLIENT_X: i32 = X + 182;
const LOCK_GRACE_W: i32 = 96;
/// Where the encoder's column starts, right of the frame rate's and the sound check box's.
const ENCODER_X: i32 = X + 224;
const ENCODER_W: i32 = Area::MIN.w - (ENCODER_X - X);

/// The Remote Desktop page as `status` and `enabled` (the server's unit) leave it, laid out for
/// the login manager when `machine` is given; the host follows the server with [`show_remote`].
pub fn remote_desktop(
    a: Area,
    status: Option<&Status>,
    enabled: bool,
    machine: Option<&str>,
) -> Form {
    let port = |y| {
        spin(
            "port_label",
            REMOTE_PORT,
            a.right() - 86,
            y,
            32,
            "&Port:",
            (1024, 65535),
        )
    };
    let mut items = Vec::new();
    if machine.is_some() {
        items.push(Item::label_at(
            MANAGED_NOTE,
            X,
            44,
            a.w,
            "Remote logins are set on the Login Manager page.",
        ));
        let button_x = a.right() - ACCOUNTS_W;
        items.extend([
            Item::lines(
                "rd_password_label",
                Rect::new(X, RD_PASSWORD_Y - 1, button_x - X - 6, 2 * LINE_H),
                &["Your Remote Desktop password is set in User Accounts."],
            ),
            Item::button_at(
                RD_PASSWORD_ACCOUNTS,
                button_x,
                RD_PASSWORD_Y,
                ACCOUNTS_W,
                "&User Accounts...",
                true,
            ),
        ]);
    } else {
        items.push(Item::check(
            REMOTE_ALLOW,
            X,
            44,
            "Allow remote &connections to this computer",
        ));
        items.extend(port(44));
        items.extend([
            Item::label_at(
                REMOTE_NOTE,
                X,
                64,
                a.w - 18,
                "Turning this on or off takes effect at once.",
            )
            .under_check(),
            Item::label_at(REMOTE_STATUS, X, 80, a.w - 18, "").under_check(),
        ]);
    }
    items.extend([
        Item::label_at("fingerprint_label", X, 104, 126, "&Fingerprint (SHA-256):"),
        edit(
            REMOTE_FINGERPRINT,
            X + 130,
            100,
            a.w - 130 - NEW_CERTIFICATE_W - 6,
        )
        .read_only(),
        Item::button_at(
            REMOTE_REGENERATE,
            a.right() - NEW_CERTIFICATE_W,
            98,
            NEW_CERTIFICATE_W,
            "&New Certificate...",
            true,
        ),
        Item::label_at("quality_label", X, 130, 200, "Picture &quality:"),
    ]);
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
        157,
        27,
        (left, right),
        X,
        TrackBar::new(1, REMOTE_QUALITIES.len()),
    ));
    let rates: Vec<&str> = FRAME_RATES.iter().map(|r| r.1).collect();
    let encoders: Vec<&str> = REMOTE_ENCODERS.iter().map(|e| e.1).collect();
    items.extend([
        Item::label_at("frame_rate_label", X, 184, 160, "Frame &rate:"),
        Item::combo(REMOTE_FRAME_RATE, X, 200, 160, &rates, rates.len()),
        Item::label_at(
            "video_encoder_label",
            ENCODER_X,
            184,
            ENCODER_W,
            "Video &encoder:",
        ),
        Item::combo(
            REMOTE_ENCODER,
            ENCODER_X,
            200,
            ENCODER_W,
            &encoders,
            encoders.len(),
        ),
        Item::check(REMOTE_SOUND, X, 230, "Play &sound on the remote computer"),
        Item::label_at(ENCODER, ENCODER_X, 230, a.right() - ENCODER_X, ""),
        Item::label_at("clients_label", X, 252, 200, "Connected computers:"),
        Item::new(
            CLIENTS,
            Rect::new(X, 268, a.w - BUTTON_W - 6, 2 * listbox::ROW_H + 4),
            Control::ListBox {
                list: ListBox::new(Vec::new(), None),
                icons: Vec::new(),
            },
        ),
        button(REMOTE_DISCONNECT, a.right() - BUTTON_W, 268, "&Disconnect"),
        Item::check(
            REMOTE_LOCK_ON_LEAVE,
            X,
            312,
            "&Lock this computer when the last one disconnects",
        ),
    ]);
    let grace_x = a.right() - LOCK_GRACE_W;
    let graces: Vec<&str> = LOCK_GRACES.iter().map(|g| g.1).collect();
    items.extend([
        Item::label_at(
            "lock_grace_label",
            X,
            333,
            grace_x - 6 - X - 18,
            "&Wait before locking on a screen change:",
        )
        .under_check(),
        Item::combo(
            REMOTE_LOCK_GRACE,
            grace_x,
            329,
            LOCK_GRACE_W,
            &graces,
            graces.len(),
        ),
        Item::check(
            REMOTE_BORDER,
            X,
            355,
            "Show a red &border while a remote computer is connected",
        ),
        Item::label_at(
            "remote_screen_label",
            X,
            377,
            a.w,
            "While connected, this computer's screen:",
        ),
        radio(REMOTE_SCREEN_SHOW, "remote_screen", X, 395, "Stays &on"),
        radio(
            REMOTE_SCREEN_LOCK,
            "remote_screen",
            X + 80,
            395,
            "Loc&ks and fits the remote window",
        ),
        Item::label_at(REMOTE_SCREEN_NOTE, X + 18, 413, a.w - 18, ""),
        Item::label_at(
            "second_client_label",
            X,
            431,
            SECOND_CLIENT_X - X - 6,
            "When another client connects:",
        ),
        radio(
            REMOTE_SECOND_JOIN,
            "second_client",
            SECOND_CLIENT_X,
            431,
            "&Join it",
        ),
        radio(
            REMOTE_SECOND_REPLACE,
            "second_client",
            SECOND_CLIENT_X + 70,
            431,
            "Replace &it",
        ),
    ]);
    let mut page = Form::new(items);
    show_remote(&mut page, status, enabled, None, None, machine);
    page
}

/// Whether `page` was built for the login manager's remote logins.
pub fn managed(page: &Form) -> bool {
    page.item(MANAGED_NOTE).is_some()
}

/// Put the server's state on the page in place, so an open drop-down survives an unchanged poll;
/// `machine` (on a [`managed`] page) lights it whatever the unit says. Clients keep `keep` picked.
pub fn show_remote(
    page: &mut Form,
    status: Option<&Status>,
    enabled: bool,
    address: Option<&str>,
    keep: Option<&str>,
    machine: Option<&str>,
) {
    let on = enabled || machine.is_some();
    page.set_check(REMOTE_ALLOW, enabled);
    page.set_enabled(REMOTE_ALLOW, true);
    for id in REMOTE_CONTROLS {
        page.set_enabled(id, on);
    }
    for id in PORT_CONTROLS {
        page.set_enabled(id, enabled && machine.is_none());
    }
    let line = match machine {
        Some(_) => String::new(),
        None => remote_status(status, enabled, address),
    };
    page.set_text(REMOTE_STATUS, &line);
    let shown = status.filter(|_| on);
    let encoder = shown.map_or(String::new(), |s| format!("Video: {}", s.encoder));
    page.set_text(ENCODER, &encoder);
    let offered = shown.map_or(&[][..], |s| s.encoders.as_slice());
    if let Some(Control::Combo(c)) = page.item_mut(REMOTE_ENCODER).map(|i| &mut i.control) {
        for (item, (value, _)) in c.items.iter_mut().zip(REMOTE_ENCODERS) {
            item.greyed = *value != "auto" && !offered.iter().any(|o| o == value);
        }
    }
    let fingerprint = machine.unwrap_or(shown.map_or("", |s| s.fingerprint.as_str()));
    page.show_edit_text(REMOTE_FINGERPRINT, fingerprint);
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
    page.set_enabled(REMOTE_REGENERATE, shown.is_some() && machine.is_none());
    page.set_enabled(REMOTE_DISCONNECT, picked);
    follow_lock(page);
    follow_screen(page);
}

/// Grey the lock's wait unless the lock check box is ticked and lit.
pub(super) fn follow_lock(page: &mut Form) {
    let lit = matches!(
        page.item(REMOTE_LOCK_ON_LEAVE).map(|i| &i.control),
        Some(Control::Check {
            checked: true,
            enabled: true,
            ..
        })
    );
    for id in LOCK_GRACE_CONTROLS {
        page.set_enabled(id, lit);
    }
}

/// Grey the Lock radio while the Stays on radio is greyed or [`REMOTE_SCREEN_NOTE`] shows
/// [`LOCKER_NOTE`].
pub(super) fn follow_screen(page: &mut Form) {
    let lit = matches!(
        page.item(REMOTE_SCREEN_SHOW).map(|i| &i.control),
        Some(Control::Radio { enabled: true, .. })
    );
    let held = matches!(
        page.item(REMOTE_SCREEN_NOTE).map(|i| &i.control),
        Some(Control::Label { text, .. }) if text == LOCKER_NOTE
    );
    page.set_enabled(REMOTE_SCREEN_LOCK, lit && !held);
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
