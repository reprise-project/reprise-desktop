//! Keyboard, Mouse, Touchpad, Cursor, Text & Keys and Pointer & Alerts.

use super::{
    Area, BOUNCE_DELAY, CURSOR_DEFAULT_ROW, CURSOR_PREVIEW, DELAY_STEPS, DOUBLE_CLICK, GX, KB_ADD,
    KB_LAYOUTS, KB_REMOVE, KB_SWITCH_NOTE, KB_UP, MK_ACCEL, MK_SPEED, SC_ADD, SC_EDIT, SC_REMOVE,
    SCROLL_SPEED, SPEED_STEPS, X, button, choice, combo, edit, ends_track, group, shortcuts, spin,
    updown,
};
use crate::model::{CLICK_METHODS, CURSOR_SIZES, TEXT_SIZES, VISUAL_BELLS};
use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, LINE_H};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::listbox::ListBox;
use reprisekit::raster::Rect;
use reprisekit::trackbar::{self, TrackBar};

/// The Layouts list: three 17 px rows inside its 2 px frame, between the label column and the button column.
const LAYOUTS_Y: i32 = 62;

fn layouts(a: Area) -> Rect {
    Rect::new(
        GX + 84,
        LAYOUTS_Y,
        a.gw() - 84 - BUTTON_W - 6,
        3 * reprisekit::controls::listbox::ROW_H + 4,
    )
}

pub fn keyboard(a: Area) -> Form {
    let gw = a.gw();
    let mut items = vec![group(a, "keyboard_group", 44, 210, "Keyboard")];
    let row = |label_id, id, y, text| {
        [
            Item::label_at(label_id, GX, y, 80, text),
            edit(id, GX + 84, y - 4, gw - 84),
        ]
    };
    // The layouts in switching order, an empty list showing the system default greyed; each button 4 px under the last, beside the list.
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
    let list = layouts(a);
    items.push(Item::label_at(
        KB_SWITCH_NOTE,
        list.x,
        list.bottom() + 5,
        list.width,
        "",
    ));
    items.push(Item::check(
        "layout_per_window",
        GX + 84,
        146,
        "Remember a layout for each &window",
    ));
    items.extend(row("model_label", "kb_model", 170, "&Model:"));
    items.extend(row("options_label", "kb_options", 196, "&Options:"));
    items.extend(spin(
        "rate_label",
        "repeat_rate",
        GX,
        222,
        80,
        "&Repeat rate:",
        (1, 255),
    ));
    items.push(Item::label_at("rate_units", GX + 140, 222, 50, "a second"));
    items.extend(spin(
        "delay_label",
        "repeat_delay",
        GX + 196,
        222,
        40,
        "Dela&y:",
        (0, 9999),
    ));
    items.push(Item::label_at("delay_units", GX + 296, 222, 20, "ms"));
    // The Shortcuts list is a `ListView` the host draws over the page; the page carries its buttons only.
    items.push(group(a, "shortcuts_group", 262, 138 + a.dh, "Shortcuts"));
    items.push(Item::check(
        "shortcut_overlay",
        GX,
        280,
        "Hold Super to &show them after",
    ));
    let (min, max) = repriseconf::config::ShortcutOverlay::DELAY_MS.into_inner();
    items.push(updown(
        "shortcut_overlay_ms",
        GX + 218,
        276,
        min as i32,
        max as i32,
    ));
    items.push(Item::label_at(
        "shortcut_overlay_ms_units",
        GX + 272,
        280,
        20,
        "ms",
    ));
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
    let y = 408 + a.dh;
    items.push(group(a, "osd_group", y, 40, "On-screen display"));
    items.push(Item::check("osd_caps_lock", GX, y + 18, "Show &Caps Lock"));
    items.push(Item::check(
        "osd_num_lock",
        GX + a.gw() / 2,
        y + 18,
        "Show &Num Lock",
    ));
    Form::new(items)
}

/// A pointer speed: a "Select a pointer speed:" label, Slow and Fast around a
/// trackbar, and "Enhance pointer precision" under it. `y` is the label's row;
/// the items span 62 px. `ids` are the label, Slow, the trackbar, Fast and the
/// check, and the trackbar is `name` to a reader.
fn motion(a: Area, y: i32, ids: [&'static str; 5], name: &'static str) -> [Item; 5] {
    let [label_id, slow_id, track_id, fast_id, check_id] = ids;
    let track = Rect::new(GX + 28, y + 15, a.track(160), trackbar::HEIGHT);
    [
        Item::label_at(label_id, GX, y, a.gw(), "Select a &pointer speed:"),
        Item::label_at(slow_id, GX, y + 21, 26, "Slow"),
        Item::new(
            track_id,
            track,
            Control::TrackBar(TrackBar::new(SPEED_STEPS / 2, SPEED_STEPS)),
        )
        .named(name),
        Item::label_at(fast_id, track.right() + 4, y + 21, 26, "Fast"),
        Item::check(check_id, GX, y + 49, "&Enhance pointer precision"),
    ]
}

/// A scroll speed: its label at `y`, Slow and Fast around a trackbar under it, the percent at the
/// label's right. `ids` are the label, Slow, the trackbar, Fast and the percent.
fn scroll_speed(a: Area, y: i32, ids: [&'static str; 5]) -> Vec<Item> {
    let [label_id, slow_id, track_id, fast_id, value_id] = ids;
    let mut items = vec![Item::label_at(label_id, GX, y, 200, "Scro&ll speed:")];
    items.extend(ends_track(
        a,
        [slow_id, track_id, fast_id, value_id],
        "Scroll speed",
        y + 27,
        27,
        ("Slow", "Fast"),
        GX,
        TrackBar::new(0, SCROLL_SPEED.count),
    ));
    items
}

pub fn mouse(a: Area) -> Form {
    let mut items = vec![group(a, "mouse_group", 44, 178, "Mouse")];
    items.push(Item::label_at(
        "double_click_label",
        GX,
        68,
        200,
        "Double-click &speed:",
    ));
    items.extend(ends_track(
        a,
        [
            "double_click_slow",
            "double_click_ms",
            "double_click_fast",
            "double_click_value",
        ],
        "Double-click speed",
        95,
        27,
        ("Slow", "Fast"),
        GX,
        TrackBar::new(0, DOUBLE_CLICK.count),
    ));
    items.extend(motion(
        a,
        120,
        [
            "mouse_speed_label",
            "mouse_slow",
            "mouse_speed",
            "mouse_fast",
            "mouse_accel",
        ],
        "Pointer speed",
    ));
    items.push(Item::check(
        "mouse_middle",
        GX,
        191,
        "Press both buttons for a &middle click",
    ));
    items.push(group(a, "wheel_group", 232, 106, "Wheel"));
    items.extend(scroll_speed(
        a,
        256,
        [
            "mouse_scroll_label",
            "mouse_scroll_slow",
            "mouse_scroll_speed",
            "mouse_scroll_fast",
            "mouse_scroll_value",
        ],
    ));
    items.push(Item::check(
        "mouse_natural",
        GX,
        308,
        "Re&verse scrolling direction",
    ));
    Form::new(items)
}

/// The Touchpad page's controls that grey while "Use the touchpad" is unticked.
pub(super) const TOUCHPAD_CONTROLS: [&str; 18] = [
    "disable_with_mouse",
    "touchpad_speed_label",
    "touchpad_slow",
    "touchpad_speed",
    "touchpad_fast",
    "touchpad_accel",
    "disable_while_typing",
    "tap_to_click",
    "two_finger_tap_right_click",
    "click_method_label",
    "click_method",
    "two_finger_scroll",
    "touchpad_scroll_label",
    "touchpad_scroll_slow",
    "touchpad_scroll_speed",
    "touchpad_scroll_fast",
    "touchpad_scroll_value",
    "touchpad_natural",
];

pub fn touchpad(a: Area) -> Form {
    let mut items = vec![
        group(a, "touchpad_group", 44, 166, "Touchpad"),
        Item::check("touchpad_enabled", GX, 66, "&Use the touchpad"),
        Item::check(
            "disable_with_mouse",
            GX,
            88,
            "Turn it off while a &mouse is plugged in",
        ),
    ];
    items.extend(motion(
        a,
        114,
        [
            "touchpad_speed_label",
            "touchpad_slow",
            "touchpad_speed",
            "touchpad_fast",
            "touchpad_accel",
        ],
        "Touchpad speed",
    ));
    items.push(Item::check(
        "disable_while_typing",
        GX,
        185,
        "&Disable while typing",
    ));

    items.push(group(a, "clicking_group", 216, 99, "Clicking"));
    items.push(Item::check("tap_to_click", GX, 238, "&Tap to click"));
    items.push(Item::check(
        "two_finger_tap_right_click",
        GX,
        260,
        "Two-finger tap is a &right click (middle when off)",
    ));
    items.push(Item::label_at(
        "click_method_label",
        GX,
        286,
        90,
        "Click met&hod:",
    ));
    items.push(choice("click_method", GX + 94, 282, 150, CLICK_METHODS));

    items.push(group(a, "touchpad_scroll_group", 321, 123, "Scrolling"));
    items.push(Item::check(
        "two_finger_scroll",
        GX,
        343,
        "Scroll with two &fingers (edge scrolling when off)",
    ));
    items.extend(scroll_speed(
        a,
        367,
        [
            "touchpad_scroll_label",
            "touchpad_scroll_slow",
            "touchpad_scroll_speed",
            "touchpad_scroll_fast",
            "touchpad_scroll_value",
        ],
    ));
    items.push(Item::check(
        "touchpad_natural",
        GX,
        419,
        "Reverse scr&olling direction",
    ));
    Form::new(items)
}

/// The pointer theme and size, and a preview of the theme's pointers the host
/// draws into `CURSOR_PREVIEW` (inside is [`preview_size`]), which takes the page's extra width and height.
pub fn cursor(a: Area, cursor_themes: &[String]) -> Form {
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

/// The StickyKeys controls that grey with its check box, the SlowKeys, BounceKeys and MouseKeys ones.
pub(super) const STICKY_CONTROLS: [&str; 2] = ["sticky_lock", "sticky_two_keys_off"];

pub(super) const MOUSE_KEYS_CONTROLS: [&str; 9] = [
    "mouse_keys_num_lock",
    "mk_speed_label",
    "mk_speed_slow",
    "mouse_keys_speed",
    "mk_speed_fast",
    "mk_accel_label",
    "mk_accel_slow",
    "mouse_keys_acceleration",
    "mk_accel_fast",
];

pub(super) const SLOW_CONTROLS: [&str; 5] = [
    "slow_delay_label",
    "slow_delay_value",
    "slow_short",
    "slow_keys_delay",
    "slow_long",
];

pub(super) const BOUNCE_CONTROLS: [&str; 5] = [
    "bounce_delay_label",
    "bounce_delay_value",
    "bounce_short",
    "bounce_keys_delay",
    "bounce_long",
];

/// The Text & Keys page: the text size on top, then StickyKeys, SlowKeys and BounceKeys, each in
/// its group, and the shortcuts that turn the aids on and off.
pub fn text_and_keys(a: Area) -> Form {
    let gw = a.gw();
    let note = |id, y, text: &str| Item::label_at(id, GX, y, gw, text);
    let sizes: Vec<&str> = TEXT_SIZES.iter().map(|c| c.1).collect();
    let mut items = vec![
        Item::label_at("text_size_label", X, 48, 70, "&Text size:"),
        Item::combo("text_size", X + 74, 44, 80, &sizes, sizes.len()),
    ];
    items.push(group(a, "sticky_group", 70, 103, "StickyKeys"));
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
    items.extend(delay_group(
        a,
        179,
        ["slow_group", "slow_keys", "slow_delay_label"],
        ["SlowKeys", "U&se SlowKeys", "&Keys must be held down for:"],
        Some((
            "slow_note",
            "A key counts only once it has been held down for the delay.",
        )),
        [
            "slow_short",
            "slow_keys_delay",
            "slow_long",
            "slow_delay_value",
        ],
        "SlowKeys delay",
        TrackBar::new(0, DELAY_STEPS),
    ));
    items.extend(delay_group(
        a,
        300,
        ["bounce_group", "bounce_keys", "bounce_delay_label"],
        [
            "BounceKeys",
            "Use &BounceKeys",
            "&Ignore a key pressed again within:",
        ],
        None,
        [
            "bounce_short",
            "bounce_keys_delay",
            "bounce_long",
            "bounce_delay_value",
        ],
        "BounceKeys delay",
        TrackBar::new(0, BOUNCE_DELAY.count),
    ));
    items.push(Item::check(
        "key_shortcuts",
        X,
        400,
        "Use the keyboard s&hortcuts",
    ));
    items.push(
        Item::new(
            "shortcuts_note",
            Rect::new(X, 418, a.w - 18, 2 * LINE_H),
            Control::Lines(vec![
                "Shift 5 times toggles StickyKeys, right Shift held 8 seconds".into(),
                "SlowKeys, and left Alt+left Shift+Num Lock MouseKeys.".into(),
            ]),
        )
        .under_check(),
    );
    Form::new(items)
}

/// A group at `y` for a key aid with a delay: its note (id and text) if any, its check box, the
/// delay's label and a Shorter-Longer trackbar `name` to a reader; 94 px tall, 115 with a note.
/// `ids` and `texts` are the group, check box and label; `track` are the trackbar row's ids as
/// [`ends_track`] takes them.
#[allow(clippy::too_many_arguments)]
fn delay_group(
    a: Area,
    y: i32,
    ids: [&'static str; 3],
    texts: [&str; 3],
    note: Option<(&'static str, &str)>,
    track: [&'static str; 4],
    name: &'static str,
    bar: TrackBar,
) -> Vec<Item> {
    let [group_id, check_id, label_id] = ids;
    let [title, check, label] = texts;
    let dy = if note.is_some() { 21 } else { 0 };
    let mut items = vec![group(a, group_id, y, 94 + dy, title)];
    if let Some((id, text)) = note {
        items.push(Item::label_at(id, GX, y + 18, a.gw(), text));
    }
    items.push(Item::check(check_id, GX, y + 18 + dy, check));
    items.push(Item::label_at(label_id, GX, y + 40 + dy, 200, label));
    items.extend(ends_track(
        a,
        track,
        name,
        y + 67 + dy,
        27,
        ("Shorter", "Longer"),
        GX,
        bar,
    ));
    items
}

/// The Pointer & Alerts page: MouseKeys, the Ctrl tap that shows where the pointer is, and what
/// flashes when a program rings the bell.
pub fn pointer_and_alerts(a: Area) -> Form {
    let gw = a.gw();
    // The keypad moves the pointer: the switch and Num Lock's share a row, and the
    // speed and acceleration each take a row, Slow and Fast around the trackbar.
    let half = gw / 2;
    let mut items = vec![
        group(a, "mouse_keys_group", 44, 98, "MouseKeys"),
        Item::check("mouse_keys", GX, 62, "Use &MouseKeys"),
        Item::check(
            "mouse_keys_num_lock",
            GX + half,
            62,
            "Only while &Num Lock is on",
        ),
    ];
    items.extend(mouse_keys_track(
        a,
        88,
        [
            "mk_speed_label",
            "mk_speed_slow",
            "mouse_keys_speed",
            "mk_speed_fast",
        ],
        "S&peed:",
        "MouseKeys speed",
        MK_SPEED.count,
    ));
    items.extend(mouse_keys_track(
        a,
        116,
        [
            "mk_accel_label",
            "mk_accel_slow",
            "mouse_keys_acceleration",
            "mk_accel_fast",
        ],
        "&Acceleration:",
        "MouseKeys acceleration",
        MK_ACCEL.count,
    ));
    items.extend([
        group(a, "locate_group", 148, 62, "Pointer Location"),
        Item::check(
            "locate_pointer",
            GX,
            166,
            "Show where the pointer is when I tap &Ctrl",
        ),
        Item::label_at(
            "locate_note",
            GX,
            186,
            gw,
            "Rings grow round the pointer, wherever it is.",
        ),
        group(a, "visual_group", 216, 82, "Visual Alerts"),
        Item::label_at(
            "visual_note",
            GX,
            234,
            gw,
            "For when the bell can't be heard.",
        ),
        Item::label_at("visual_bell_label", GX, 256, 110, "&Flash on the bell:"),
        choice("visual_bell", GX + 114, 252, 150, VISUAL_BELLS),
        Item::label_at(
            "visual_sound_note",
            GX,
            278,
            gw,
            "The Sound page sets whether the bell sounds too.",
        ),
    ]);
    Form::new(items)
}

/// A MouseKeys trackbar row at text row `y`: its label, then Slow, the trackbar `name` to a
/// reader, and Fast. `ids` are the label, Slow, the trackbar and Fast.
fn mouse_keys_track(
    a: Area,
    y: i32,
    ids: [&'static str; 4],
    label: &'static str,
    name: &'static str,
    count: usize,
) -> [Item; 4] {
    let [label_id, slow_id, track_id, fast_id] = ids;
    let track = Rect::new(
        GX + 116,
        y - 6,
        a.track(Area::MIN.gw() - 150),
        trackbar::HEIGHT,
    );
    [
        Item::label_at(label_id, GX, y, 80, label),
        Item::label_at(slow_id, GX + 84, y, 30, "Slow"),
        Item::new(track_id, track, Control::TrackBar(TrackBar::new(0, count))).named(name),
        Item::label_at(fast_id, track.right() + 4, y, 30, "Fast"),
    ]
}
