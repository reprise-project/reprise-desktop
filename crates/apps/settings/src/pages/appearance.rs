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
    let mut items = vec![group(a, "fonts_group", 44, 84, "Fonts")];
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
    items.push(Item::check("font_antialias", GX + 160, 108, "Antia&lias"));

    items.push(group(a, "icons_group", 138, 56, "Icons"));
    items.push(Item::label_at("theme_label", GX, 162, 56, "&Theme:"));
    items.push(combo(
        "icon_theme",
        GX + 60,
        158,
        a.combo(Area::MIN.gw() - 60),
        icon_themes,
        icon_themes.len().clamp(1, 8),
    ));

    items.push(group(a, "windows_group", 204, 92, "Windows"));
    items.extend(spin(
        "frame_label",
        "frame_width",
        GX,
        226,
        110,
        "Frame &width:",
        (0, 32),
    ));
    items.extend(spin(
        "caption_label",
        "caption_height",
        GX,
        252,
        110,
        "Caption &height:",
        (12, 128),
    ));
    items.extend(spin(
        "taskbar_label",
        "taskbar_height",
        GX,
        278,
        110,
        "Tas&kbar height:",
        (8, 256),
    ));

    // GTK and Qt look: `(Automatic)` and Automatic follow the look's darkness,
    // the Reprise rows its colours. Their font smoothing is apart from the Fonts
    // group's Antialias, which is Reprise's own text.
    let mut gtk: Vec<String> = GTK_FIXED_ROWS
        .iter()
        .map(|(_, row)| row.to_string())
        .collect();
    gtk.extend(gtk_themes.iter().cloned());
    let cw = a.combo(Area::MIN.gw() - 88);
    items.push(group(a, "toolkits_group", 306, 114, "Programs"));
    items.push(Item::label_at("gtk_label", GX, 330, 84, "GTK th&eme:"));
    items.push(combo(
        "gtk_theme",
        GX + 88,
        326,
        cw,
        &gtk,
        gtk.len().clamp(1, 8),
    ));
    items.push(Item::label_at("qt_label", GX, 356, 84, "&Qt style:"));
    let mut qt = choice("qt_platform_theme", GX + 88, 352, cw, QT_PLATFORM_THEMES);
    if let Control::Combo(c) = &mut qt.control
        && let Some(row) = c.items.iter_mut().find(|i| i.text == REPRISE_TOOLKIT_ROW)
    {
        row.greyed = !qt_tools;
    }
    items.push(qt);
    items.push(Item::label_at(
        "smoothing_label",
        GX,
        382,
        84,
        "Fo&nt smoothing:",
    ));
    items.push(choice("font_smoothing", GX + 88, 378, cw, FONT_SMOOTHINGS));
    items.push(Item::check(
        "gtk4_colors",
        GX,
        404,
        "GTK&4 and libadwaita in the look's colours",
    ));
    items.push(Item::new(
        "toolkits_note",
        Rect::new(X, 422, a.w, LINE_H),
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
        117,
        27,
        ("Warmer", "Cooler"),
        GX + 46,
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
/// indented under it, and the screen's fade; then the menus' shadow, timings and opening, then the task buttons' slide.
pub fn effects(a: Area) -> Form {
    let range = (0, EFFECTS_MS_MAX as i32);
    let mut items = vec![group(a, "effects_group", 44, 158, "Windows")];
    items.push(Item::check(
        "window_animation",
        GX,
        66,
        "&Animate windows opening and closing",
    ));
    let mut anim = Vec::new();
    anim.extend(spin(
        "open_label",
        "window_open_ms",
        GX,
        92,
        70,
        "&Open ms:",
        range,
    ));
    anim.extend(spin(
        "close_label",
        "window_close_ms",
        GX + 140,
        92,
        70,
        "&Close ms:",
        range,
    ));
    items.extend(anim.into_iter().map(Item::under_check));
    items.push(Item::check(
        "hover_fade",
        GX,
        122,
        "Fade button &highlights",
    ));
    let mut hover = Vec::new();
    hover.extend(spin(
        "in_label",
        "hover_in_ms",
        GX,
        148,
        70,
        "&In ms:",
        range,
    ));
    hover.extend(spin(
        "out_label",
        "hover_out_ms",
        GX + 140,
        148,
        70,
        "O&ut ms:",
        range,
    ));
    items.extend(hover.into_iter().map(Item::under_check));
    items.push(Item::check(
        "session_fade",
        GX,
        178,
        "&Fade the screen at log on and log off",
    ));
    items.push(group(a, "menus_group", 212, 74, "Menus"));
    items.extend(spin(
        "shadow_label",
        "menu_shadow",
        GX,
        234,
        60,
        "&Shadow:",
        (0, 32),
    ));
    items.extend(spin(
        "hover_label",
        "menu_hover_ms",
        GX + 140,
        234,
        70,
        "Ho&ver ms:",
        (0, 9999),
    ));
    items.extend(spin(
        "fade_label",
        "menu_fade_ms",
        GX,
        260,
        60,
        "Ti&me ms:",
        (0, 9999),
    ));
    items.push(Item::label_at(
        "menu_animation_label",
        GX + 140,
        260,
        70,
        "A&nimation:",
    ));
    items.push(choice(
        "menu_animation",
        GX + 214,
        256,
        110,
        MENU_ANIMATIONS,
    ));
    items.push(group(a, "taskbar_group", 296, 48, "Taskbar"));

    items.push(Item::check(
        "taskbar_slide",
        GX,
        318,
        "Slide &taskbar buttons as they are dragged",
    ));
    Form::new(items)
}

pub(super) const ANIM_CONTROLS: [&str; 4] = [
    "open_label",
    "window_open_ms",
    "close_label",
    "window_close_ms",
];

pub(super) const HOVER_CONTROLS: [&str; 4] =
    ["in_label", "hover_in_ms", "out_label", "hover_out_ms"];
