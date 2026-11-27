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
    EFFECTS_MS_MAX, FONT_SMOOTHINGS, MENU_ANIMATIONS, QT_PLATFORM_THEMES, reprise_toolkit_row,
};
use reprisekit::tr;

/// `qt_tools` is whether qt6ct or qt5ct is installed, which the Qt combo's Reprise row needs; without either it is greyed.
pub fn fonts_and_icons(
    a: Area,
    icon_themes: &[String],
    gtk_themes: &[String],
    qt_tools: bool,
) -> Form {
    let gw = a.gw();
    let mut items = vec![group(
        a,
        "fonts_group",
        44,
        102,
        &tr!("pages-appearance-fonts"),
    )];
    let half = (gw - 8) / 2;
    items.extend(field(
        "family_label",
        "font_family",
        GX,
        62,
        half,
        &tr!("pages-appearance-family-comma-separated"),
    ));
    items.extend(field(
        "shell_dlg_label",
        "font_shell_dlg",
        a.gright() - half,
        62,
        half,
        &tr!("pages-appearance-second-dialog-font"),
    ));
    items.extend(spin(
        "size_label",
        "font_size_px",
        GX,
        108,
        44,
        &tr!("pages-appearance-size"),
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
    items.push(group(
        a,
        "smoothing_group",
        154,
        72,
        &tr!("pages-appearance-font-smoothing"),
    ));
    items.push(Item::label_at(
        "reprise_text_label",
        GX,
        176,
        84,
        &tr!("pages-appearance-reprise-text"),
    ));
    items.push(Item::check(
        "font_antialias",
        GX + 88,
        176,
        &tr!("pages-appearance-smooth-the-edges"),
    ));
    items.push(Item::label_at(
        "smoothing_label",
        GX,
        202,
        84,
        &tr!("pages-appearance-other-programs"),
    ));
    items.push(choice("font_smoothing", GX + 88, 198, cw, FONT_SMOOTHINGS));

    items.push(group(
        a,
        "icons_group",
        234,
        48,
        &tr!("pages-appearance-icons"),
    ));
    items.push(Item::label_at(
        "theme_label",
        GX,
        258,
        56,
        &tr!("pages-appearance-theme"),
    ));
    items.push(combo(
        "icon_theme",
        GX + 60,
        254,
        a.combo(Area::MIN.gw() - 60),
        icon_themes,
        icon_themes.len().clamp(1, 8),
    ));

    items.push(group(
        a,
        "windows_group",
        290,
        46,
        &tr!("pages-appearance-window-frames"),
    ));
    items.extend(spin(
        "frame_label",
        "frame_width",
        GX,
        312,
        76,
        &tr!("pages-appearance-frame-width"),
        (0, 32),
    ));
    items.push(Item::label_at("frame_units", GX + 136, 312, 20, "px"));
    items.extend(spin(
        "caption_label",
        "caption_height",
        GX + 164,
        312,
        88,
        &tr!("pages-appearance-caption-height"),
        (12, 128),
    ));
    items.push(Item::label_at("caption_units", GX + 312, 312, 20, "px"));

    let mut gtk: Vec<String> = GTK_FIXED_ROWS.iter().map(|(_, row)| row()).collect();
    gtk.extend(gtk_themes.iter().cloned());
    items.push(group(
        a,
        "toolkits_group",
        344,
        86,
        &tr!("pages-appearance-programs"),
    ));
    items.push(Item::label_at(
        "gtk_label",
        GX,
        368,
        84,
        &tr!("pages-appearance-gtk-theme"),
    ));
    items.push(combo(
        "gtk_theme",
        GX + 88,
        364,
        cw,
        &gtk,
        gtk.len().clamp(1, 8),
    ));
    items.push(Item::label_at(
        "qt_label",
        GX,
        394,
        84,
        &tr!("pages-appearance-qt-style"),
    ));
    let mut qt = choice("qt_platform_theme", GX + 88, 390, cw, QT_PLATFORM_THEMES);
    if let Control::Combo(c) = &mut qt.control
        && let Some(row) = c.items.iter_mut().find(|i| i.text == reprise_toolkit_row())
    {
        row.greyed = !qt_tools;
    }
    items.push(qt);
    items.push(Item::check(
        "gtk4_colors",
        GX,
        416,
        &tr!("pages-appearance-gtk4-and-libadwaita-in-the-looks"),
    ));
    items.push(Item::new(
        "toolkits_note",
        Rect::new(X, 434, a.w, LINE_H),
        Control::Lines(vec![tr!("pages-appearance-qt-and-gtk4-programs-take-a")]),
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
        &tr!("pages-appearance-warm-the-screens-colors-at-night"),
    )];
    items.push(group(
        a,
        "nl_color_group",
        70,
        76,
        &tr!("pages-appearance-color-temperature"),
    ));
    items.push(Item::label_at(
        "nl_temp_label",
        GX,
        90,
        200,
        &tr!("pages-appearance-screen-color-at-night"),
    ));
    items.extend(ends_track(
        a,
        ["nl_warm", "nl_temperature", "nl_cool", "nl_temp_value"],
        &tr!("pages-appearance-color-temperature-2"),
        117,
        27,
        (
            &tr!("pages-appearance-warmer"),
            &tr!("pages-appearance-cooler"),
        ),
        GX,
        TrackBar::new(0, KELVIN_STEPS),
    ));
    items.push(group(
        a,
        "nl_schedule_group",
        156,
        104,
        &tr!("pages-appearance-schedule"),
    ));
    items.push(radio(
        "nl_sun",
        "nl_schedule",
        GX,
        178,
        &tr!("pages-appearance-from-sunset-to-sunrise"),
    ));
    items.push(radio(
        "nl_fixed",
        "nl_schedule",
        GX,
        200,
        &tr!("pages-appearance-at-fixed-times"),
    ));
    // The times row starts at the choices' label column: the check box's, which is the radio's too in looks that draw them alike.
    let times = time(
        "nl_from_label",
        "nl_from",
        GX,
        230,
        36,
        &tr!("pages-appearance-from"),
    )
    .into_iter()
    .chain(time(
        "nl_to_label",
        "nl_to",
        GX + 132,
        230,
        24,
        &tr!("pages-appearance-to"),
    ));
    items.extend(times.map(Item::under_check));
    items.push(Item::new(
        "nl_note",
        Rect::new(X, 272, a.w, 4 * LINE_H),
        Control::Lines(vec![
            tr!("pages-appearance-sunset-and-sunrise-are-worked-out"),
            tr!("pages-appearance-the-latitude-and-longitude-in-config"),
            tr!("pages-appearance-fades-over-30-minutes-the-tray"),
        ]),
    ));
    Form::new(items)
}

/// The Effects page: reduce motion over every other motion, then window animations and the fade of
/// hovered buttons, each a check with its two durations indented under it, the screen's fade and the
/// lock screen's motion; then the menus' shadow, timings and opening.
pub fn effects(a: Area) -> Form {
    let mut items = vec![group(
        a,
        "effects_group",
        44,
        202,
        &tr!("pages-appearance-animation"),
    )];
    items.push(Item::check(
        "reduce_motion",
        GX,
        66,
        &tr!("pages-appearance-reduce-motion-everywhere"),
    ));
    items.push(Item::check(
        "window_animation",
        GX,
        88,
        &tr!("pages-appearance-animate-windows-opening-and-closing"),
    ));
    let mut anim = Vec::new();
    anim.extend(ms_spin(
        ["open_label", "window_open_ms", "open_units"],
        GX,
        114,
        &tr!("pages-appearance-open"),
    ));
    anim.extend(ms_spin(
        ["close_label", "window_close_ms", "close_units"],
        GX + 140,
        114,
        &tr!("pages-appearance-close"),
    ));
    items.extend(anim.into_iter().map(Item::under_check));
    items.push(Item::check(
        "hover_fade",
        GX,
        144,
        &tr!("pages-appearance-fade-button-highlights"),
    ));
    let mut hover = Vec::new();
    hover.extend(ms_spin(
        ["in_label", "hover_in_ms", "in_units"],
        GX,
        170,
        &tr!("pages-appearance-in"),
    ));
    hover.extend(ms_spin(
        ["out_label", "hover_out_ms", "out_units"],
        GX + 140,
        170,
        &tr!("pages-appearance-out"),
    ));
    items.extend(hover.into_iter().map(Item::under_check));
    items.push(Item::check(
        "session_fade",
        GX,
        200,
        &tr!("pages-appearance-fade-the-screen-at-log-on"),
    ));
    items.push(Item::check(
        "logon_animation",
        GX,
        222,
        &tr!("pages-appearance-animate-the-lock-screen"),
    ));
    items.push(group(
        a,
        "menus_group",
        256,
        74,
        &tr!("pages-appearance-menus"),
    ));
    items.extend(spin(
        "shadow_label",
        "menu_shadow",
        GX,
        278,
        60,
        &tr!("pages-appearance-shadow"),
        (0, 32),
    ));
    items.push(Item::label_at("shadow_units", GX + 118, 278, 20, "px"));
    items.extend(spin(
        "hover_label",
        "menu_hover_ms",
        GX + 140,
        278,
        70,
        &tr!("pages-appearance-hover"),
        (0, 9999),
    ));
    items.push(Item::label_at("menu_hover_units", GX + 268, 278, 20, "ms"));
    items.extend(spin(
        "fade_label",
        "menu_fade_ms",
        GX,
        304,
        60,
        &tr!("pages-appearance-time"),
        (0, 9999),
    ));
    items.push(Item::label_at("menu_fade_units", GX + 118, 304, 20, "ms"));
    items.push(Item::label_at(
        "menu_animation_label",
        GX + 140,
        304,
        70,
        &tr!("pages-appearance-animation-2"),
    ));
    items.push(choice(
        "menu_animation",
        GX + 214,
        300,
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

/// The Effects page's motion controls, greyed while reduce motion is on.
pub(super) const MOTION_CONTROLS: [&str; 8] = [
    "window_animation",
    "hover_fade",
    "session_fade",
    "logon_animation",
    "fade_label",
    "menu_fade_ms",
    "menu_fade_units",
    "menu_animation_label",
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
