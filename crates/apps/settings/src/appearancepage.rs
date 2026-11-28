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
    GLASS_FRAMES, GLASS_MATERIALS, GLASS_STYLES, INTERFACES, MODES, Settings, text_to_list,
};
use crate::pages::{Area, COMBO_MAX, TRACK_MAX, X, choice, time_edit};
use crate::registry::Page;
use repriseconf::config::{AppearanceMode, CLASSIC_DARK_SCHEME};
use reprisekit::tr;

pub const SAMPLE: &str = "sample";
/// The sample's rect at every window size; the rows under it stay put.
pub const SAMPLE_RECT: Rect = Rect::new(X + 4, 44, Area::MIN.w - 8, 188);
/// The scheme combo's id under each interface: each lists its own names, and the id says whose.
pub const CLASSIC_SCHEME: &str = "classic_scheme";
pub const BLUE_SCHEME: &str = "blue_scheme";
pub const GLASS_COLOR: &str = "glass_color";
/// The last glass colour row while the file's colour names no swatch.
pub fn custom_row() -> String {
    tr!("appearancepage-custom")
}
const COMBO_W: i32 = 196;
const RIGHT_X: i32 = 232;
const RIGHT_W: i32 = 146;

fn columns(a: Area) -> (i32, i32, i32) {
    let dw = a.dw().min(2 * (COMBO_MAX - COMBO_W));
    let half = dw / 2;
    (COMBO_W + half, RIGHT_X + half, RIGHT_W + dw - half)
}
const INTENSITY_POSITIONS: usize = 101;
const TIME_W: i32 = 62;
/// The "hours" mode's times, greyed under any other mode.
pub const DARK_HOURS: [&str; 4] = ["dark_hours_label", "dark_from", "dark_to_label", "dark_to"];
/// Classic's dark scheme combo, greyed under Light.
pub const DARK_SCHEME: &str = "dark_scheme";
const DARK_SCHEME_LABEL: &str = "dark_scheme_label";
const BLUR_POSITIONS: usize = glass::BLUR_MAX as usize;

/// The interface `s` names; an unknown one is Classic, as the compositor reads it.
pub fn interface(s: &Settings) -> Interface {
    Interface::parse(&s.interface).unwrap_or_default()
}

/// `s`'s `[glass]` as reprisekit's settings, dark while `s`'s mode draws dark ([`mode_dark`]); a
/// colour neither a swatch nor "#RRGGBB" is the default's. Blur, streaks and `blur_rate` show
/// only in the desktop's backdrop, which the sample has none of, so it takes their defaults.
pub fn glass(s: &Settings) -> glass::Settings {
    let d = glass::Settings::default();
    glass::Settings {
        color: glass::swatch(&s.glass_color)
            .or_else(|| Color::parse(&s.glass_color))
            .unwrap_or(d.color),
        mode: match mode_dark(s) {
            true => glass::Mode::Dark,
            false => glass::Mode::Light,
        },
        material: glass::Material::parse(&s.glass_material).unwrap_or_default(),
        frame: glass::Frame::parse(&s.glass_frame).unwrap_or_default(),
        style: glass::Style::parse(&s.glass_style).unwrap_or_default(),
        notification: glass::NotifyStyle::parse(&s.glass_notification_style).unwrap_or_default(),
        intensity: s.glass_intensity.clamp(0, 100) as u8,
        transparency: s.glass_transparency,
        fast_drag: s.glass_fast_drag,
        blur_frames: s.glass_blur_frames,
        ..d
    }
}

/// The Classic scheme `s` wears while dark.
pub fn classic_dark(s: &Settings) -> &str {
    match s.dark_color_scheme.as_str() {
        "" => CLASSIC_DARK_SCHEME,
        name => name,
    }
}

/// Whether `s`'s mode is "night" or "hours", whose state now only the compositor knows.
fn scheduled(s: &Settings) -> bool {
    s.mode == AppearanceMode::Night.as_str() || s.mode == AppearanceMode::Hours.as_str()
}

/// Whether `s`'s mode draws dark: Dark, or a schedule while the session is dark.
pub fn mode_dark(s: &Settings) -> bool {
    s.mode == AppearanceMode::Dark.as_str() || scheduled(s) && repriseapp::session_dark()
}

pub fn theme(s: &Settings) -> Theme {
    let i = interface(s);
    let g = glass(s);
    let dark = mode_dark(s);
    let scheme = match (i, dark) {
        (Interface::Classic, true) => classic_dark(s),
        _ => &s.color_scheme,
    };
    let background = Color::parse(&s.background);
    let mut t = reprisekit::interfaces::sample_theme(i, scheme, dark, &g, background);
    let picked_over = match dark {
        true => {
            let light = glass::Settings {
                mode: glass::Mode::Light,
                ..g
            };
            reprisekit::interfaces::sample_theme(i, &s.color_scheme, false, &light, background)
                .colors
        }
        false => t.colors.clone(),
    };
    let pins: Vec<(&str, Color)> = (s.colors.iter())
        .filter_map(|(name, value)| Some((name.as_str(), Color::parse(value)?)))
        .collect();
    // A name that is no colour fails the compositor's read; the sample draws the others.
    let _ = t.colors.pin(&picked_over, &pins);
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
    t.with_text_size_own_bar(s.text_size.clamp(100, 200) as u32)
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

/// Classic's schemes, the High Contrast ones last.
fn classic_rows() -> impl Iterator<Item = &'static str> {
    let contrast = |n: &&str| n.starts_with("High Contrast");
    (schemes::names().filter(move |n| !contrast(n))).chain(schemes::names().filter(contrast))
}

fn scheme_rows(i: Interface, custom: bool) -> Vec<String> {
    match i {
        Interface::Classic => classic_rows().map(String::from).collect(),
        Interface::Blue => blue::Variant::ALL
            .iter()
            .map(|v| v.name().to_string())
            .collect(),
        Interface::Glass => glass::swatch_names()
            .map(title)
            .chain(custom.then(|| custom_row().to_string()))
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
/// Custom row while `s.glass_custom` is set. Under Glass the Style list shares the scheme's row.
pub fn page(s: &Settings, a: Area) -> Form {
    let i = interface(s);
    let glass = i == Interface::Glass;
    let rows = scheme_rows(i, !s.glass_custom.is_empty());
    let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
    let (lw, rx, rw) = columns(a);
    let half_w = (lw - 16) / 2;
    let frame_x = X + half_w + 16;
    let tw = rw.min(TRACK_MAX);
    let scheme_w = if glass { half_w } else { lw };
    let mut items = vec![
        Item::new(
            SAMPLE,
            SAMPLE_RECT,
            Control::Sample {
                themes: false,
                theme: theme(s),
                icon: None,
            },
        ),
        Item::label_at(
            "interface_label",
            X,
            242,
            lw,
            &tr!("appearancepage-interface"),
        ),
        choice("interface", X, 258, lw, INTERFACES),
        Item::label_at(
            "scheme_label",
            X,
            288,
            scheme_w,
            &tr!("appearancepage-color-scheme"),
        ),
        Item::combo(scheme_id(i), X, 304, scheme_w, &refs, refs.len().min(8)),
        Item::label_at("mode_label", rx, 242, rw, &tr!("appearancepage-mode")),
        choice("mode", rx, 258, rw, MODES),
    ];
    let dark_hours = |hours_y: i32| {
        [
            Item::label_at(
                DARK_HOURS[0],
                rx,
                hours_y,
                rw,
                &tr!("appearancepage-dark-hours"),
            ),
            time_edit(DARK_HOURS[1], rx, hours_y + 16, TIME_W),
            Item::label_at(
                DARK_HOURS[2],
                rx + TIME_W + 4,
                hours_y + 20,
                14,
                &tr!("appearancepage-dark-hours-to"),
            ),
            time_edit(DARK_HOURS[3], rx + TIME_W + 22, hours_y + 16, TIME_W)
                .named(tr!("appearancepage-dark-until")),
        ]
    };
    if i != Interface::Glass {
        items.extend(dark_hours(288));
    }
    if i == Interface::Classic {
        let dark: Vec<&str> = classic_rows()
            .filter(|n| schemes::scheme(n).is_some_and(|c| c.is_dark()))
            .collect();
        items.extend([
            Item::label_at(
                DARK_SCHEME_LABEL,
                X,
                334,
                lw,
                &tr!("appearancepage-dark-color-scheme"),
            ),
            Item::combo(DARK_SCHEME, X, 350, lw, &dark, dark.len().min(8)),
        ]);
    }
    if glass {
        items.extend([
            Item::label_at(
                "style_label",
                frame_x,
                288,
                half_w,
                &tr!("appearancepage-style"),
            ),
            choice("glass_style", frame_x, 304, half_w, GLASS_STYLES),
            Item::label_at(
                "material_label",
                X,
                334,
                half_w,
                &tr!("appearancepage-material"),
            ),
            choice("glass_material", X, 350, half_w, GLASS_MATERIALS),
            Item::label_at(
                "frame_label",
                frame_x,
                334,
                half_w,
                &tr!("appearancepage-frame"),
            ),
            choice("glass_frame", frame_x, 350, half_w, GLASS_FRAMES),
            Item::check(
                "glass_transparency",
                X,
                384,
                &tr!("appearancepage-enable-transparency"),
            ),
            Item::check(
                "glass_fast_drag",
                X,
                406,
                &tr!("appearancepage-faster-glass-while-dragging"),
            ),
            Item::check(
                "glass_blur_frames",
                X,
                428,
                &tr!("appearancepage-show-frames-behind-glass"),
            ),
            Item::check(
                "glass_streaks",
                rx,
                384,
                &tr!("appearancepage-show-reflections"),
            ),
            Item::label_at(
                "intensity_label",
                rx,
                288,
                rw,
                &tr!("appearancepage-color-intensity"),
            ),
            Item::new(
                "glass_intensity",
                Rect::new(rx, 304, tw, trackbar::HEIGHT),
                Control::TrackBar(TrackBar::new(42, INTENSITY_POSITIONS).paged(10, 5)),
            ),
            Item::label_at("blur_label", rx, 334, rw, &tr!("appearancepage-blur")),
            Item::new(
                "glass_blur",
                Rect::new(rx, 350, tw, trackbar::HEIGHT),
                Control::TrackBar(TrackBar::new(1, BLUR_POSITIONS).paged(1, 1)),
            ),
        ]);
        // Under the intensity and blur that fill the column below Mode.
        items.extend(dark_hours(406));
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
    for id in DARK_HOURS {
        form.set_enabled(id, s.mode == AppearanceMode::Hours.as_str());
    }
    for id in [DARK_SCHEME_LABEL, DARK_SCHEME] {
        form.set_enabled(id, s.mode != AppearanceMode::Light.as_str());
    }
    // Set every time: `Theme`'s equality cannot see the glass settings (the look compares by name).
    if let Some(Control::Sample { theme: t, .. }) = form.item_mut(SAMPLE).map(|i| &mut i.control) {
        *t = theme(s);
    }
}

/// Under a schedule, draw the sample in the session's look now, which can turn with no event;
/// runs before each draw.
pub fn follow_session(sheet: &mut PropSheet, s: &Settings) {
    if !scheduled(s) {
        return;
    }
    let form = &mut sheet.pages[Page::Appearance.index()];
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

    #[test]
    fn a_dark_onyx_sample_on_the_small_bar_draws_its_controls_dark() {
        let onyx = |mode: &str| Settings {
            interface: "glass".into(),
            glass_style: "onyx".into(),
            taskbar_small_icons: true,
            mode: mode.into(),
            ..Settings::default()
        };
        let name = |s: &Settings| theme(s).look.style().map(|st| st.name);
        assert_eq!(name(&onyx("dark")), Some("glass-dark"), "mode dark");
        repriseapp::set_session_dark(true);
        assert_eq!(
            name(&onyx("hours")),
            Some("glass-dark"),
            "hours, inside them"
        );
        repriseapp::set_session_dark(false);
        assert_eq!(name(&onyx("hours")), Some("glass"), "hours, outside them");
    }

    #[test]
    fn the_sample_lays_the_files_colors_over_the_picked_scheme() {
        let s = crate::model::load(
            "[appearance]\ncolor_scheme = \"Storm\"\n[colors]\nactive_title = \"#000080\"\nbackground = \"#102030\"\n",
        );
        assert_eq!(
            s.colors,
            [("active_title".to_string(), "#000080".to_string())]
        );
        let t = theme(&s);
        assert_eq!(t.colors.active_title, Color::rgb(0, 0, 0x80));
        assert_eq!(t.colors.background, Color::rgb(0x10, 0x20, 0x30));
        let storm = reprisekit::schemes::scheme("Storm").unwrap();
        assert_eq!(t.colors.inactive_title, storm.inactive_title);
    }

    #[test]
    fn at_200_percent_the_sample_fills_the_room_above_the_interface_row() {
        let f = page(&Settings::default(), Area::MIN);
        let room = |pct| {
            f.drawn_with(&Theme::default().with_text_size(pct).paint());
            let sample = f.rect(SAMPLE).expect("the page has a sample");
            let label = f.rect("interface_label").expect("the page has a label");
            (sample.height, label.y - sample.bottom())
        };
        assert_eq!(room(100), (188, 10));
        assert_eq!(room(200), (376, 20));
    }

    #[test]
    fn a_maximized_window_keeps_the_sample_and_the_rows_under_it_in_place() {
        let at = |a: Area| {
            let f = page(&Settings::default(), a);
            let y = |id: &str| f.item(id).expect("on the page").rect.y;
            (
                f.item(SAMPLE).unwrap().rect,
                y("interface_label"),
                y("mode"),
            )
        };
        let large = Area { w: 1500, dh: 600 };
        assert_eq!(at(large), at(Area::MIN));
    }

    #[test]
    fn the_sample_draws_at_the_picked_text_size() {
        for i in ["classic", "blue", "glass"] {
            let at = |pct| {
                theme(&Settings {
                    interface: i.into(),
                    text_size: pct,
                    ..Settings::default()
                })
            };
            let (small, big) = (at(100), at(200));
            assert!(big.fonts.size_px > small.fonts.size_px, "{i}");
            assert!(
                big.metrics.caption_height > small.metrics.caption_height,
                "{i}"
            );
        }
    }
}
