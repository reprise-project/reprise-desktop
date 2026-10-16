use reprisekit::blue;
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::propsheet::PropSheet;
use reprisekit::glass;
use reprisekit::interface::Interface;
use reprisekit::raster::Rect;
use reprisekit::schemes;
use reprisekit::theme::{Color, ColorScheme, FontConfig, Theme};
use reprisekit::trackbar::{self, TrackBar};

use crate::model::{
    GLASS_FRAMES, GLASS_MATERIALS, GLASS_MODES, GLASS_NOTIFY_STYLES, INTERFACES, Settings,
    text_to_list,
};
use crate::pages::{Area, COMBO_MAX, TRACK_MAX, X, choice};
use crate::registry::Page;

pub const SAMPLE: &str = "sample";
pub fn sample_rect(a: Area) -> Rect {
    Rect::new(X + 4, 44, a.w - 8, 188 + a.dh)
}
/// The scheme combo's id under each interface: each lists its own names, and the id says whose.
pub const CLASSIC_SCHEME: &str = "classic_scheme";
pub const BLUE_SCHEME: &str = "blue_scheme";
pub const GLASS_COLOR: &str = "glass_color";
/// The last glass colour row while the file's colour names no swatch.
pub const CUSTOM_ROW: &str = "Custom";
const COMBO_W: i32 = 196;
const RIGHT_X: i32 = 232;
const RIGHT_W: i32 = 146;

fn columns(a: Area) -> (i32, i32, i32) {
    let dw = a.dw().min(2 * (COMBO_MAX - COMBO_W));
    let half = dw / 2;
    (COMBO_W + half, RIGHT_X + half, RIGHT_W + dw - half)
}
const INTENSITY_POSITIONS: usize = 101;
const BLUR_POSITIONS: usize = glass::BLUR_MAX as usize;

/// The interface `s` names; an unknown one is Classic, as the compositor reads it.
pub fn interface(s: &Settings) -> Interface {
    Interface::parse(&s.interface).unwrap_or_default()
}

/// `s`'s `[glass]` as reprisekit's settings; a colour neither a swatch nor "#RRGGBB" is the
/// default's. Blur, streaks and `blur_rate` show only in the desktop's backdrop, which the sample
/// has none of, so it takes their defaults.
pub fn glass(s: &Settings) -> glass::Settings {
    let d = glass::Settings::default();
    glass::Settings {
        color: glass::swatch(&s.glass_color)
            .or_else(|| Color::parse(&s.glass_color))
            .unwrap_or(d.color),
        mode: glass::Mode::parse(&s.glass_mode).unwrap_or_default(),
        material: glass::Material::parse(&s.glass_material).unwrap_or_default(),
        frame: glass::Frame::parse(&s.glass_frame).unwrap_or_default(),
        notification: glass::NotifyStyle::parse(&s.glass_notification_style).unwrap_or_default(),
        intensity: s.glass_intensity.clamp(0, 100) as u8,
        transparency: s.glass_transparency,
        fast_drag: s.glass_fast_drag,
        blur_frames: s.glass_blur_frames,
        ..d
    }
}

pub fn theme(s: &Settings) -> Theme {
    let i = interface(s);
    let g = glass(s);
    let mut t =
        reprisekit::interfaces::sample_theme(i, &s.color_scheme, &g, Color::parse(&s.background));
    if i == Interface::Glass && s.taskbar_small_icons {
        reprisekit::interfaces::small_bar(&mut t, &g);
    }
    let family = text_to_list(&s.font_family);
    if !family.is_empty() {
        t.fonts.family = family;
    }
    t.fonts.size_px = s.font_size_px.clamp(
        FontConfig::SIZE_PX_MIN as i64,
        FontConfig::SIZE_PX_MAX as i64,
    ) as u32;
    t.fonts.antialias = s.font_antialias;
    t
}

/// The Classic scheme an absent `color_scheme` stands for.
pub fn classic_default() -> &'static str {
    schemes::name_of(&ColorScheme::DEFAULT).expect("the default scheme has a name")
}

/// The scheme combo's id under `i`.
pub fn scheme_id(i: Interface) -> &'static str {
    match i {
        Interface::Classic => CLASSIC_SCHEME,
        Interface::Blue => BLUE_SCHEME,
        Interface::Glass => GLASS_COLOR,
    }
}

fn title(name: &str) -> String {
    let mut c = name.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn scheme_rows(i: Interface, custom: bool) -> Vec<String> {
    match i {
        Interface::Classic => schemes::names().map(String::from).collect(),
        Interface::Blue => blue::Variant::ALL
            .iter()
            .map(|v| v.name().to_string())
            .collect(),
        Interface::Glass => glass::swatch_names()
            .map(title)
            .chain(custom.then(|| CUSTOM_ROW.to_string()))
            .collect(),
    }
}

/// The glass colour list's row for `value`: its swatch's, else the Custom row, the last of `rows`.
pub fn glass_row(value: &str, rows: usize) -> usize {
    crate::model::glass_swatch(value)
        .and_then(|n| glass::swatch_names().position(|m| m == n))
        .unwrap_or(rows.saturating_sub(1))
}

/// The page for `s`'s interface in `a`, before any value is pushed in; the glass colour list has a
/// Custom row while `s.glass_custom` is set.
pub fn page(s: &Settings, a: Area) -> Form {
    let i = interface(s);
    let rows = scheme_rows(i, !s.glass_custom.is_empty());
    let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
    let (lw, rx, rw) = columns(a);
    let half_w = (lw - 16) / 2;
    let frame_x = X + half_w + 16;
    let tw = rw.min(TRACK_MAX);
    let y = |y: i32| y + a.dh;
    let mut items = vec![
        Item::new(
            SAMPLE,
            sample_rect(a),
            Control::Sample {
                themes: false,
                theme: theme(s),
                icon: None,
            },
        ),
        Item::label_at("interface_label", X, y(242), lw, "&Interface:"),
        choice("interface", X, y(258), lw, INTERFACES),
        Item::label_at("scheme_label", X, y(288), lw, "&Color scheme:"),
        Item::combo(scheme_id(i), X, y(304), lw, &refs, refs.len().min(8)),
    ];
    if i == Interface::Glass {
        items.extend([
            Item::label_at("mode_label", rx, y(242), rw, "&Mode:"),
            choice("glass_mode", rx, y(258), rw, GLASS_MODES),
            Item::label_at("material_label", X, y(334), half_w, "M&aterial:"),
            choice("glass_material", X, y(350), half_w, GLASS_MATERIALS),
            Item::label_at("frame_label", frame_x, y(334), half_w, "&Frame:"),
            choice("glass_frame", frame_x, y(350), half_w, GLASS_FRAMES),
            Item::check("glass_transparency", X, y(384), "Enable &transparency"),
            Item::check("glass_fast_drag", X, y(406), "Faster &glass while dragging"),
            Item::check("glass_blur_frames", X, y(428), "Sho&w frames behind glass"),
            Item::label_at("notify_label", X, y(452), half_w, "N&otifications:"),
            choice(
                "glass_notification_style",
                X,
                y(468),
                half_w,
                GLASS_NOTIFY_STYLES,
            ),
            Item::check("glass_streaks", rx, y(384), "Show &reflections"),
            Item::label_at("intensity_label", rx, y(288), rw, "Color i&ntensity:"),
            Item::new(
                "glass_intensity",
                Rect::new(rx, y(304), tw, trackbar::HEIGHT),
                Control::TrackBar(TrackBar::new(42, INTENSITY_POSITIONS).paged(10, 5)),
            ),
            Item::label_at("blur_label", rx, y(334), rw, "&Blur:"),
            Item::new(
                "glass_blur",
                Rect::new(rx, y(350), tw, trackbar::HEIGHT),
                Control::TrackBar(TrackBar::new(1, BLUR_POSITIONS).paged(1, 1)),
            ),
            Item::label_at("orb_glow_label", rx, y(406), rw, "&Start button glow:"),
            Item::new(
                "glass_orb_glow",
                Rect::new(rx, y(422), tw, trackbar::HEIGHT),
                Control::TrackBar(TrackBar::new(100, INTENSITY_POSITIONS).paged(10, 5)),
            ),
        ]);
    }
    Form::new(items)
}

/// Rebuild the page when it was built for another interface than `s`'s, keeping the focus, grey
/// the backdrop rows while the glass is opaque, and draw the sample in `s`'s look.
pub fn follow(sheet: &mut PropSheet, s: &Settings) {
    let at = Page::Appearance.index();
    let id = scheme_id(interface(s));
    if sheet.pages[at].item(id).is_none() {
        let old = &sheet.pages[at];
        let focused = old.focus().and_then(|f| old.items.get(f)).map(|i| i.id);
        sheet.set_page(at, page(s, Area::of(sheet)));

        crate::pages::write_form(&mut sheet.pages[at], s);
        match focused {
            Some(CLASSIC_SCHEME | BLUE_SCHEME | GLASS_COLOR) => sheet.pages[at].focus_id(id),
            Some(f) if sheet.pages[at].item(f).is_some() => sheet.pages[at].focus_id(f),
            _ => {}
        }
    }
    let form = &mut sheet.pages[at];
    for id in [
        "glass_fast_drag",
        "glass_blur_frames",
        "blur_label",
        "glass_blur",
    ] {
        form.set_enabled(id, s.glass_transparency);
    }
    let frosted =
        glass::Material::parse(&s.glass_material).unwrap_or_default() == glass::Material::Frosted;
    form.set_enabled("glass_streaks", s.glass_transparency && frosted);
    // Set every time: `Theme`'s equality cannot see the glass settings (the look compares by name).
    if let Some(Control::Sample { theme: t, .. }) = form.item_mut(SAMPLE).map(|i| &mut i.control) {
        *t = theme(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::interfaces::theme_for;

    #[test]
    fn the_sample_takes_the_small_bar_and_the_fonts() {
        let small = Settings {
            interface: "glass".into(),
            taskbar_small_icons: true,
            ..Settings::default()
        };
        assert_eq!(
            theme(&small).metrics.taskbar_height,
            glass::SMALL_TASKBAR_HEIGHT
        );
        let classic = Settings {
            taskbar_small_icons: true,
            ..Settings::default()
        };
        assert_eq!(
            theme(&classic).metrics.taskbar_height,
            theme_for(Interface::Classic).metrics.taskbar_height
        );
        let fonts = Settings {
            font_family: "Verdana, DejaVu Sans".into(),
            font_size_px: 14,
            font_antialias: true,
            ..Settings::default()
        };
        let f = theme(&fonts).fonts;
        assert_eq!(
            (f.family, f.size_px, f.antialias),
            (vec!["Verdana".to_string(), "DejaVu Sans".into()], 14, true)
        );
    }
}
