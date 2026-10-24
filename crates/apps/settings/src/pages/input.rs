//! Keyboard, Mouse, Pointers and Accessibility.

use super::{
    Area, CURSOR_DEFAULT_ROW, CURSOR_PREVIEW, DELAY_STEPS, GX, KB_ADD, KB_LAYOUTS, KB_REMOVE,
    KB_UP, MK_ACCEL, MK_SPEED, SC_ADD, SC_EDIT, SC_REMOVE, SPEED_STEPS, TRACK_MAX, X, button,
    combo, edit, ends_track, group, shortcuts, spin,
};
use crate::model::{CURSOR_SIZES, TEXT_SIZES};
use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, LINE_H};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::listbox::ListBox;
use reprisekit::raster::Rect;
use reprisekit::trackbar::{self, TrackBar};

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

/// The StickyKeys controls that grey with its check box, the SlowKeys ones and the MouseKeys ones.
pub(super) const STICKY_CONTROLS: [&str; 2] = ["sticky_lock", "sticky_two_keys_off"];

pub(super) const MOUSE_KEYS_CONTROLS: [&str; 5] = [
    "mouse_keys_num_lock",
    "mk_speed_label",
    "mouse_keys_speed",
    "mk_accel_label",
    "mouse_keys_acceleration",
];

pub(super) const SLOW_CONTROLS: [&str; 5] = [
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
