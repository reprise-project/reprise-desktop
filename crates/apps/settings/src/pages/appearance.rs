//! Night Light, Fonts & Icons and Effects, the Appearance pages built here.

use super::{
    Area, GTK_FIXED_ROWS, GX, KELVIN_STEPS, X, choice, combo, ends_track, field, group, radio,
    spin, time,
};
use reprisekit::classic::widgets::LINE_H;
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::raster::Rect;
use reprisekit::theme::FontConfig;
use reprisekit::trackbar::TrackBar;

use crate::model::{
    EFFECTS_MS_MAX, FONT_SMOOTHINGS, MENU_ANIMATIONS, QT_PLATFORM_THEMES, REPRISE_TOOLKIT_ROW,
};

/// `qt_tools` is whether qt6ct or qt5ct is installed, which the Qt combo's Reprise row needs; without either it is greyed.
pub fn fonts_and_icons(
    a: Area,
    icon_themes: &[String],
    gtk_themes: &[String],
    qt_tools: bool,
) -> Form {
    let gw = a.gw();
    let mut items = vec![group(a, "fonts_group", 44, 102, "Fonts")];
    let half = (gw - 8) / 2;
    items.extend(field(
        "family_label",
        "font_family",
        GX,
        62,
        half,
        "&Family (comma separated):",
    ));
    items.extend(field(
        "shell_dlg_label",
        "font_shell_dlg",
        a.gright() - half,
        62,
        half,
        "Second dialo&g font:",
    ));
    items.extend(spin(
        "size_label",
        "font_size_px",
        GX,
        108,
        44,
        "Si&ze:",
        (
            FontConfig::SIZE_PX_MIN as i32,
            FontConfig::SIZE_PX_MAX as i32,
        ),
    ));
    items.push(Item::label_at("font_size_units", GX + 104, 108, 20, "px"));
    items.push(Item::new(
        "fonts_note",
        Rect::new(GX, 126, gw, LINE_H),
        Control::Lines(vec![text_size_note()]),
    ));

    let cw = a.combo(Area::MIN.gw() - 88);
    items.push(group(a, "smoothing_group", 154, 72, "Font Smoothing"));
    items.push(Item::label_at(
        "reprise_text_label",
        GX,
        176,
        84,
        "Reprise text:",
    ));
    items.push(Item::check(
        "font_antialias",
        GX + 88,
        176,
        "S&mooth the edges",
    ));
    items.push(Item::label_at(
        "smoothing_label",
        GX,
        202,
        84,
        "&Other programs:",
    ));
    items.push(choice("font_smoothing", GX + 88, 198, cw, FONT_SMOOTHINGS));

    items.push(group(a, "icons_group", 234, 48, "Icons"));
    items.push(Item::label_at("theme_label", GX, 258, 56, "&Theme:"));
    items.push(combo(
        "icon_theme",
        GX + 60,
        254,
        a.combo(Area::MIN.gw() - 60),
        icon_themes,
        icon_themes.len().clamp(1, 8),
    ));

    items.push(group(a, "windows_group", 290, 46, "Window Frames"));
    items.extend(spin(
        "frame_label",
        "frame_width",
        GX,
        312,
        76,
        "Frame &width:",
        (0, 32),
    ));
    items.push(Item::label_at("frame_units", GX + 136, 312, 20, "px"));
    items.extend(spin(
        "caption_label",
        "caption_height",
        GX + 164,
        312,
        88,
        "Caption &height:",
        (12, 128),
    ));
    items.push(Item::label_at("caption_units", GX + 312, 312, 20, "px"));

    let mut gtk: Vec<String> = GTK_FIXED_ROWS
        .iter()
        .map(|(_, row)| row.to_string())
        .collect();
    gtk.extend(gtk_themes.iter().cloned());
    items.push(group(a, "toolkits_group", 344, 86, "Programs"));
    items.push(Item::label_at("gtk_label", GX, 368, 84, "GTK th&eme:"));
    items.push(combo(
        "gtk_theme",
        GX + 88,
        364,
        cw,
        &gtk,
        gtk.len().clamp(1, 8),
    ));
    items.push(Item::label_at("qt_label", GX, 394, 84, "&Qt style:"));
    let mut qt = choice("qt_platform_theme", GX + 88, 390, cw, QT_PLATFORM_THEMES);
    if let Control::Combo(c) = &mut qt.control
        && let Some(row) = c.items.iter_mut().find(|i| i.text == REPRISE_TOOLKIT_ROW)
    {
        row.greyed = !qt_tools;
    }
    items.push(qt);
    items.push(Item::check(
        "gtk4_colors",
        GX,
        416,
        "GTK&4 and libadwaita in the look's colours",
    ));
    items.push(Item::new(
        "toolkits_note",
        Rect::new(X, 434, a.w, LINE_H),
        Control::Lines(vec![
            "Qt and GTK4 programs take a new style when they next start.".into(),
        ]),
    ));

    Form::new(items)
}

/// The night light's controls that grey with its check box, and those that also need the fixed schedule.
pub(super) const NL_CONTROLS: [&str; 7] = [
    "nl_temp_label",
    "nl_temp_value",
    "nl_warm",
    "nl_temperature",
    "nl_cool",
    "nl_sun",
    "nl_fixed",
];

pub(super) const NL_TIMES: [&str; 4] = ["nl_from_label", "nl_from", "nl_to_label", "nl_to"];

pub fn night_light(a: Area) -> Form {
    let mut items = vec![Item::check(
        "nl_enabled",
        X,
        44,
        "&Warm the screen's colors at night",
    )];
    items.push(group(a, "nl_color_group", 70, 76, "Color Temperature"));
    items.push(Item::label_at(
        "nl_temp_label",
        GX,
        90,
        200,
        "Screen &color at night:",
    ));
    items.extend(ends_track(
        a,
        ["nl_warm", "nl_temperature", "nl_cool", "nl_temp_value"],
        "Color temperature",
        117,
        27,
        ("Warmer", "Cooler"),
        GX,
        TrackBar::new(0, KELVIN_STEPS),
    ));
    items.push(group(a, "nl_schedule_group", 156, 104, "Schedule"));
    items.push(radio(
        "nl_sun",
        "nl_schedule",
        GX,
        178,
        "From &sunset to sunrise",
    ));
    items.push(radio("nl_fixed", "nl_schedule", GX, 200, "At &fixed times"));
    // The times row starts at the choices' label column: the check box's, which is the radio's too in looks that draw them alike.
    let times = time("nl_from_label", "nl_from", GX, 230, 36, "F&rom:")
        .into_iter()
        .chain(time("nl_to_label", "nl_to", GX + 132, 230, 24, "&To:"));
    items.extend(times.map(Item::under_check));
    items.push(Item::new(
        "nl_note",
        Rect::new(X, 272, a.w, 4 * LINE_H),
        Control::Lines(vec![
            "Sunset and sunrise are worked out for the time zone's city, or for".into(),
            "the latitude and longitude in config.toml. Each change of color".into(),
            "fades over 30 minutes. The tray icon pauses it until morning.".into(),
        ]),
    ));
    Form::new(items)
}

/// The Effects page: window animations and the fade of hovered buttons, each a check with its two durations
/// indented under it, the screen's fade and the lock screen's motion; then the menus' shadow, timings
/// and opening.
pub fn effects(a: Area) -> Form {
    let mut items = vec![group(a, "effects_group", 44, 180, "Animation")];
    items.push(Item::check(
        "window_animation",
        GX,
        66,
        "&Animate windows opening and closing",
    ));
    let mut anim = Vec::new();
    anim.extend(ms_spin(
        ["open_label", "window_open_ms", "open_units"],
        GX,
        92,
        "&Open:",
    ));
    anim.extend(ms_spin(
        ["close_label", "window_close_ms", "close_units"],
        GX + 140,
        92,
        "&Close:",
    ));
    items.extend(anim.into_iter().map(Item::under_check));
    items.push(Item::check(
        "hover_fade",
        GX,
        122,
        "Fade button &highlights",
    ));
    let mut hover = Vec::new();
    hover.extend(ms_spin(
        ["in_label", "hover_in_ms", "in_units"],
        GX,
        148,
        "&In:",
    ));
    hover.extend(ms_spin(
        ["out_label", "hover_out_ms", "out_units"],
        GX + 140,
        148,
        "O&ut:",
    ));
    items.extend(hover.into_iter().map(Item::under_check));
    items.push(Item::check(
        "session_fade",
        GX,
        178,
        "&Fade the screen at log on and log off",
    ));
    items.push(Item::check(
        "logon_animation",
        GX,
        200,
        "Animate the &lock screen",
    ));
    items.push(group(a, "menus_group", 234, 74, "Menus"));
    items.extend(spin(
        "shadow_label",
        "menu_shadow",
        GX,
        256,
        60,
        "&Shadow:",
        (0, 32),
    ));
    items.push(Item::label_at("shadow_units", GX + 118, 256, 20, "px"));
    items.extend(spin(
        "hover_label",
        "menu_hover_ms",
        GX + 140,
        256,
        70,
        "Ho&ver:",
        (0, 9999),
    ));
    items.push(Item::label_at("menu_hover_units", GX + 268, 256, 20, "ms"));
    items.extend(spin(
        "fade_label",
        "menu_fade_ms",
        GX,
        282,
        60,
        "Ti&me:",
        (0, 9999),
    ));
    items.push(Item::label_at("menu_fade_units", GX + 118, 282, 20, "ms"));
    items.push(Item::label_at(
        "menu_animation_label",
        GX + 140,
        282,
        70,
        "A&nimation:",
    ));
    items.push(choice(
        "menu_animation",
        GX + 214,
        278,
        110,
        MENU_ANIMATIONS,
    ));
    Form::new(items)
}

/// A 40 px label, an up-down of milliseconds and its "ms" unit, starting at `x`.
fn ms_spin(ids: [&'static str; 3], x: i32, y: i32, text: &str) -> [Item; 3] {
    let [label, updown] = spin(ids[0], ids[1], x, y, 40, text, (0, EFFECTS_MS_MAX as i32));
    [label, updown, Item::label_at(ids[2], x + 98, y, 20, "ms")]
}

pub(super) const ANIM_CONTROLS: [&str; 6] = [
    "open_label",
    "window_open_ms",
    "open_units",
    "close_label",
    "window_close_ms",
    "close_units",
];

pub(super) const HOVER_CONTROLS: [&str; 6] = [
    "in_label",
    "hover_in_ms",
    "in_units",
    "out_label",
    "hover_out_ms",
    "out_units",
];

/// The note sending a reader to the setting that enlarges all text.
pub fn text_size_note() -> String {
    format!(
        "Text size on the {} page enlarges all text.",
        crate::registry::Page::TextKeys.label()
    )
}
