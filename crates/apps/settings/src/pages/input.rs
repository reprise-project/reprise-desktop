//! Keyboard, Mouse, Touchpad, Cursor, Text & Keys and Pointer & Alerts.

use super::{
    Area, BOUNCE_DELAY, CURSOR_PREVIEW, DELAY_STEPS, DOUBLE_CLICK, GX, KB_ADD, KB_CAPS, KB_COMPOSE,
    KB_LAYOUTS, KB_REMOVE, KB_SWITCH_NOTE, KB_UP, MK_ACCEL, MK_SPEED, SC_ADD, SC_EDIT,
    SC_OVERLAY_MS, SC_REMOVE, SCROLL_SPEED, SPEED_STEPS, X, button, choice, combo, combo_of,
    cursor_default_row, edit, ends_track, group, shortcuts, spin, updown,
};
use crate::model::{
    ATTENTION_FLASH_MAX, CAPS_LOCK, CLICK_METHODS, COMPOSE_KEY, CURSOR_SIZES, GESTURE_FINGERS,
    TEXT_SIZES, VISUAL_BELLS,
};
use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, LINE_H};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::listbox::ListBox;
use reprisekit::raster::Rect;
use reprisekit::tr;
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
    let mut items = vec![group(
        a,
        "keyboard_group",
        44,
        210,
        &tr!("pages-input-keyboard"),
    )];
    // The layouts in switching order, an empty list showing the system default greyed; each button 4 px under the last, beside the list.
    items.push(Item::label_at(
        "layouts_label",
        GX,
        LAYOUTS_Y + 4,
        80,
        &tr!("pages-input-layouts"),
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
    items.push(button(KB_ADD, bx, LAYOUTS_Y, &tr!("pages-input-add")));
    items.push(button(
        KB_REMOVE,
        bx,
        LAYOUTS_Y + step,
        &tr!("pages-input-remove"),
    ));
    items.push(button(
        KB_UP,
        bx,
        LAYOUTS_Y + 2 * step,
        &tr!("pages-input-move-up"),
    ));
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
        &tr!("pages-input-remember-a-layout-for-each-window"),
    ));
    // Two columns, sharing the extra width: the Compose and Caps Lock drop-downs over the model and
    // the other options.
    let (x1, x2) = (GX + 84, GX + 182 + a.dw() / 2);
    items.push(Item::label_at(
        "compose_label",
        GX,
        170,
        80,
        &tr!("pages-input-compose-key"),
    ));
    items.push(choice(KB_COMPOSE, x1, 166, 90, COMPOSE_KEY.rows));
    items.push(Item::label_at(
        "caps_label",
        x2,
        170,
        56,
        &tr!("pages-input-caps-lock"),
    ));
    items.push(choice(KB_CAPS, x2 + 60, 166, 92, CAPS_LOCK.rows));
    items.push(Item::label_at(
        "model_label",
        GX,
        196,
        80,
        &tr!("pages-input-model"),
    ));
    items.push(edit("kb_model", x1, 192, 90 + a.dw() / 2));
    items.push(Item::label_at(
        "options_label",
        x2,
        196,
        56,
        &tr!("pages-input-options"),
    ));
    items.push(edit("kb_options", x2 + 60, 192, a.gright() - x2 - 60));
    items.extend(spin(
        "rate_label",
        "repeat_rate",
        GX,
        222,
        80,
        &tr!("pages-input-repeat-rate"),
        (1, repriseconf::config::REPEAT_RATE_MAX),
    ));
    items.push(Item::label_at(
        "rate_units",
        GX + 140,
        222,
        50,
        &tr!("pages-input-a-second"),
    ));
    items.extend(spin(
        "delay_label",
        "repeat_delay",
        GX + 196,
        222,
        40,
        &tr!("pages-input-delay"),
        (0, 9999),
    ));
    items.push(Item::label_at(
        "delay_units",
        GX + 296,
        222,
        20,
        &tr!("units-ms"),
    ));
    // The Shortcuts list is a `ListView` the host draws over the page; the page carries its buttons only.
    items.push(group(
        a,
        "shortcuts_group",
        262,
        138 + a.dh,
        &tr!("pages-input-shortcuts"),
    ));
    items.push(Item::check(
        "shortcut_overlay",
        GX,
        280,
        &tr!("pages-input-hold-super-to-show-them-after"),
    ));
    let (min, max) = repriseconf::config::ShortcutOverlay::DELAY_MS.into_inner();
    items.push(updown(SC_OVERLAY_MS, GX + 218, 276, min as i32, max as i32));
    items.push(Item::label_at(
        "shortcut_overlay_ms_units",
        GX + 272,
        280,
        20,
        &tr!("units-ms"),
    ));
    let y = shortcuts(a).bottom() + 4;
    let r = shortcuts(a).right();
    items.push(button(
        SC_ADD,
        r - 3 * BUTTON_W - 2 * BUTTON_GAP,
        y,
        &tr!("pages-input-add-2"),
    ));
    items.push(button(
        SC_EDIT,
        r - 2 * BUTTON_W - BUTTON_GAP,
        y,
        &tr!("pages-input-edit"),
    ));
    items.push(button(
        SC_REMOVE,
        r - BUTTON_W,
        y,
        &tr!("pages-input-delete"),
    ));
    let y = 408 + a.dh;
    items.push(group(
        a,
        "osd_group",
        y,
        40,
        &tr!("pages-input-on-screen-display"),
    ));
    items.push(Item::check(
        "osd_caps_lock",
        GX,
        y + 18,
        &tr!("pages-input-show-caps-lock"),
    ));
    items.push(Item::check(
        "osd_num_lock",
        GX + a.gw() / 2,
        y + 18,
        &tr!("pages-input-show-num-lock"),
    ));
    Form::new(items)
}

/// A pointer speed: a "Select a pointer speed:" label, Slow and Fast around a
/// trackbar, and "Enhance pointer precision" under it. `y` is the label's row;
/// the items span 62 px. `ids` are the label, Slow, the trackbar, Fast and the
/// check, and the trackbar is `name` to a reader.
fn motion(a: Area, y: i32, ids: [&'static str; 5], name: &str) -> [Item; 5] {
    let [label_id, slow_id, track_id, fast_id, check_id] = ids;
    let track = Rect::new(GX + 28, y + 15, a.track(160), trackbar::HEIGHT);
    [
        Item::label_at(
            label_id,
            GX,
            y,
            a.gw(),
            &tr!("pages-input-select-a-pointer-speed"),
        ),
        Item::label_at(slow_id, GX, y + 21, 26, &tr!("pages-input-slow")),
        Item::new(
            track_id,
            track,
            Control::TrackBar(TrackBar::new(SPEED_STEPS / 2, SPEED_STEPS)),
        )
        .named(name),
        Item::label_at(
            fast_id,
            track.right() + 4,
            y + 21,
            26,
            &tr!("pages-input-fast"),
        ),
        Item::check(
            check_id,
            GX,
            y + 49,
            &tr!("pages-input-enhance-pointer-precision"),
        ),
    ]
}

/// A scroll speed: its label at `y`, Slow and Fast around a trackbar under it, the percent at the
/// label's right. `ids` are the label, Slow, the trackbar, Fast and the percent.
fn scroll_speed(a: Area, y: i32, ids: [&'static str; 5]) -> Vec<Item> {
    let [label_id, slow_id, track_id, fast_id, value_id] = ids;
    let mut items = vec![Item::label_at(
        label_id,
        GX,
        y,
        200,
        &tr!("pages-input-scroll-speed-2"),
    )];
    items.extend(ends_track(
        a,
        [slow_id, track_id, fast_id, value_id],
        &tr!("pages-input-scroll-speed"),
        y + 27,
        27,
        (&tr!("pages-input-slow"), &tr!("pages-input-fast")),
        GX,
        TrackBar::new(0, SCROLL_SPEED.count),
    ));
    items
}

pub fn mouse(a: Area) -> Form {
    let mut items = vec![group(a, "mouse_group", 44, 200, &tr!("pages-input-mouse"))];
    items.push(Item::label_at(
        "double_click_label",
        GX,
        68,
        200,
        &tr!("pages-input-double-click-speed"),
    ));
    items.extend(ends_track(
        a,
        [
            "double_click_slow",
            "double_click_ms",
            "double_click_fast",
            "double_click_value",
        ],
        &tr!("pages-input-double-click-speed-2"),
        95,
        27,
        (&tr!("pages-input-slow"), &tr!("pages-input-fast")),
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
        &tr!("pages-input-pointer-speed"),
    ));
    items.push(Item::check(
        "mouse_middle",
        GX,
        191,
        &tr!("pages-input-press-both-buttons-for-a-middle"),
    ));
    items.push(Item::check(
        "mouse_left_handed",
        GX,
        213,
        &tr!("pages-input-switch-primary-and-secondary-buttons"),
    ));
    items.push(group(a, "wheel_group", 254, 106, &tr!("pages-input-wheel")));
    items.extend(scroll_speed(
        a,
        278,
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
        330,
        &tr!("pages-input-reverse-scrolling-direction"),
    ));
    Form::new(items)
}

/// The Touchpad page's controls that grey while "Use the touchpad" is unticked.
pub(super) const TOUCHPAD_CONTROLS: [&str; 21] = [
    "gestures_enabled",
    "gesture_fingers",
    "disable_with_mouse",
    "touchpad_speed_label",
    "touchpad_slow",
    "touchpad_speed",
    "touchpad_fast",
    "touchpad_accel",
    "disable_while_typing",
    "tap_to_click",
    "two_finger_tap_right_click",
    "touchpad_left_handed",
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
    let x2 = GX + a.gw() / 2;
    let mut items = vec![
        group(a, "touchpad_group", 44, 144, &tr!("pages-input-touchpad")),
        Item::check(
            "touchpad_enabled",
            GX,
            66,
            &tr!("pages-input-use-the-touchpad"),
        ),
        Item::check("gestures_enabled", x2, 66, &tr!("pages-input-swipe-with")),
        combo_of("gesture_fingers", x2 + 88, 62, 78, GESTURE_FINGERS)
            .named(&tr!("pages-input-swipe-fingers")),
        Item::check(
            "disable_with_mouse",
            GX,
            88,
            &tr!("pages-input-turn-it-off-while-a-mouse"),
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
        &tr!("pages-input-touchpad-speed"),
    ));
    items.push(Item::check(
        "disable_while_typing",
        x2,
        163,
        &tr!("pages-input-disable-while-typing"),
    ));

    items.push(group(
        a,
        "clicking_group",
        194,
        121,
        &tr!("pages-input-clicking"),
    ));
    items.push(Item::check(
        "tap_to_click",
        GX,
        216,
        &tr!("pages-input-tap-to-click"),
    ));
    items.push(Item::check(
        "two_finger_tap_right_click",
        GX,
        238,
        &tr!("pages-input-two-finger-tap-is-a-right"),
    ));
    items.push(Item::check(
        "touchpad_left_handed",
        GX,
        260,
        &tr!("pages-input-switch-primary-and-secondary-buttons"),
    ));
    items.push(Item::label_at(
        "click_method_label",
        GX,
        286,
        90,
        &tr!("pages-input-click-method"),
    ));
    items.push(choice("click_method", GX + 94, 282, 150, CLICK_METHODS));

    items.push(group(
        a,
        "touchpad_scroll_group",
        321,
        123,
        &tr!("pages-input-scrolling"),
    ));
    items.push(Item::check(
        "two_finger_scroll",
        GX,
        343,
        &tr!("pages-input-scroll-with-two-fingers-edge-scrolling"),
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
        &tr!("pages-input-reverse-scrolling-direction-2"),
    ));
    Form::new(items)
}

/// The pointer theme and size, and a preview of the theme's pointers the host
/// draws into `CURSOR_PREVIEW` (inside is [`preview_size`]), which takes the page's extra width and height.
pub fn cursor(a: Area, cursor_themes: &[String]) -> Form {
    let mut themes = vec![cursor_default_row()];
    themes.extend(cursor_themes.iter().cloned());
    Form::new(vec![
        group(a, "cursor_group", 44, 84, &tr!("pages-input-pointer-theme")),
        Item::label_at("cursor_theme_label", GX, 66, 56, &tr!("pages-input-scheme")),
        combo(
            "cursor_theme",
            GX + 60,
            62,
            a.combo(Area::MIN.gw() - 60),
            &themes,
            themes.len().clamp(1, 8),
        ),
        Item::label_at("cursor_size_label", GX, 96, 56, &tr!("pages-input-size")),
        combo_of("cursor_size", GX + 60, 92, 110, CURSOR_SIZES),
        group(
            a,
            "cursor_preview_group",
            138,
            242 + a.dh,
            &tr!("pages-input-preview"),
        ),
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
    let mut items = vec![
        Item::label_at("text_size_label", X, 48, 70, &tr!("pages-input-text-size")),
        combo_of("text_size", X + 74, 44, 80, TEXT_SIZES),
    ];
    items.push(group(
        a,
        "sticky_group",
        70,
        103,
        &tr!("pages-input-stickykeys"),
    ));
    items.push(note(
        "sticky_note",
        88,
        &tr!("pages-input-shift-ctrl-alt-and-the-super"),
    ));
    items.push(Item::check(
        "sticky_keys",
        GX,
        109,
        &tr!("pages-input-use-stickykeys"),
    ));
    items.push(Item::check(
        "sticky_lock",
        GX,
        131,
        &tr!("pages-input-lock-a-modifier-key-when-it"),
    ));
    items.push(Item::check(
        "sticky_two_keys_off",
        GX,
        153,
        &tr!("pages-input-turn-stickykeys-off-when-two-keys"),
    ));
    items.extend(delay_group(
        a,
        179,
        ["slow_group", "slow_keys", "slow_delay_label"],
        [
            &tr!("pages-input-slowkeys"),
            &tr!("pages-input-use-slowkeys"),
            &tr!("pages-input-keys-must-be-held-down-for"),
        ],
        Some(("slow_note", &tr!("pages-input-a-key-counts-only-once-it"))),
        [
            "slow_short",
            "slow_keys_delay",
            "slow_long",
            "slow_delay_value",
        ],
        &tr!("pages-input-slowkeys-delay"),
        TrackBar::new(0, DELAY_STEPS),
    ));
    items.extend(delay_group(
        a,
        300,
        ["bounce_group", "bounce_keys", "bounce_delay_label"],
        [
            &tr!("pages-input-bouncekeys"),
            &tr!("pages-input-use-bouncekeys"),
            &tr!("pages-input-ignore-a-key-pressed-again-within"),
        ],
        None,
        [
            "bounce_short",
            "bounce_keys_delay",
            "bounce_long",
            "bounce_delay_value",
        ],
        &tr!("pages-input-bouncekeys-delay"),
        TrackBar::new(0, BOUNCE_DELAY.count),
    ));
    items.push(Item::check(
        "key_shortcuts",
        X,
        400,
        &tr!("pages-input-use-the-keyboard-shortcuts"),
    ));
    items.push(
        Item::new(
            "shortcuts_note",
            Rect::new(X, 418, a.w - 18, 2 * LINE_H),
            Control::Lines(vec![
                tr!("pages-input-shift-5-times-toggles-stickykeys-right"),
                tr!("pages-input-slowkeys-and-left-alt-left-shift"),
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
    name: &str,
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
        (&tr!("pages-input-shorter"), &tr!("pages-input-longer")),
        GX,
        bar,
    ));
    items
}

/// The Pointer & Alerts page: MouseKeys, the Ctrl tap that shows where the pointer is, what
/// flashes when a program rings the bell, and how long a button asking for attention flashes.
pub fn pointer_and_alerts(a: Area) -> Form {
    let gw = a.gw();
    // The keypad moves the pointer: the switch and Num Lock's share a row, and the
    // speed and acceleration each take a row, Slow and Fast around the trackbar.
    let half = gw / 2;
    let mut items = vec![
        group(a, "mouse_keys_group", 44, 98, &tr!("pages-input-mousekeys")),
        Item::check("mouse_keys", GX, 62, &tr!("pages-input-use-mousekeys")),
        Item::check(
            "mouse_keys_num_lock",
            GX + half,
            62,
            &tr!("pages-input-only-while-num-lock-is-on"),
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
        &tr!("pages-input-speed"),
        &tr!("pages-input-mousekeys-speed"),
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
        &tr!("pages-input-acceleration"),
        &tr!("pages-input-mousekeys-acceleration"),
        MK_ACCEL.count,
    ));
    items.extend([
        group(
            a,
            "locate_group",
            148,
            62,
            &tr!("pages-input-pointer-location"),
        ),
        Item::check(
            "locate_pointer",
            GX,
            166,
            &tr!("pages-input-show-where-the-pointer-is-when"),
        ),
        Item::label_at(
            "locate_note",
            GX,
            186,
            gw,
            &tr!("pages-input-rings-grow-round-the-pointer-wherever"),
        ),
        group(
            a,
            "visual_group",
            216,
            134,
            &tr!("pages-input-visual-alerts"),
        ),
        Item::label_at(
            "visual_note",
            GX,
            234,
            gw,
            &tr!("pages-input-for-when-the-bell-cant-be"),
        ),
        Item::label_at(
            "visual_bell_label",
            GX,
            256,
            110,
            &tr!("pages-input-flash-on-the-bell"),
        ),
        choice("visual_bell", GX + 114, 252, 150, VISUAL_BELLS),
        Item::label_at(
            "visual_sound_note",
            GX,
            278,
            gw,
            &tr!("pages-input-the-sound-page-sets-whether-the"),
        ),
    ]);
    items.extend(spin(
        "attention_flash_label",
        "taskbar_attention_flash",
        GX,
        306,
        206,
        &tr!("pages-input-flash-a-taskbar-button-for-attention"),
        (0, ATTENTION_FLASH_MAX as i32),
    ));
    items.extend([
        Item::label_at("attention_flash_units", GX + 264, 306, gw - 264, "seconds"),
        Item::label_at(
            "attention_flash_note",
            GX,
            328,
            gw,
            &tr!("pages-input-then-it-stays-lit-until-you"),
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
    label: &str,
    name: &str,
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
        Item::label_at(slow_id, GX + 84, y, 30, &tr!("pages-input-slow")),
        Item::new(track_id, track, Control::TrackBar(TrackBar::new(0, count))).named(name),
        Item::label_at(fast_id, track.right() + 4, y, 30, &tr!("pages-input-fast")),
    ]
}
