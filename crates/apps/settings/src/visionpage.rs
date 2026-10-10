//! The Vision page: the screen's colour filter (`[accessibility] color_filter` and
//! `color_filter_intensity`) with a preview of what it does to six colours, and the magnifier's
//! following (`[accessibility] magnifier_follow`) with the keys that zoom. The intensity greys unless a
//! colour-blind correction is picked; the preview follows the pending choice, the zoom keys' note the
//! pending `[[keybinds]]`.

use reprisekit::classic::widgets::LINE_H;
use reprisekit::colorfilter::{self, Filter};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::theme::Color;
use reprisekit::trackbar::{self, TrackBar};

use crate::model::{Choices, Settings, Shortcut};
use crate::pages::{Area, GX, choice, group};

/// The filters the combo lists, in the order of `colorfilter::Filter::ALL`.
pub const FILTERS: &Choices = &[
    ("none", "None"),
    ("grayscale", "Grayscale"),
    ("invert", "Inverted"),
    ("deuteranopia", "Red-green (deuteranopia)"),
    ("protanopia", "Red-green (protanopia)"),
    ("tritanopia", "Blue-yellow (tritanopia)"),
];

pub const PREVIEW: &str = "cf_preview";

/// The colours the preview shows: red, orange, yellow, green, blue and purple, each `CELL` wide, over themselves filtered.
const SAMPLES: [Color; 6] = [
    Color::rgb(0xd0, 0x30, 0x30),
    Color::rgb(0xe8, 0x88, 0x20),
    Color::rgb(0xe8, 0xd0, 0x30),
    Color::rgb(0x30, 0xa0, 0x40),
    Color::rgb(0x30, 0x60, 0xd0),
    Color::rgb(0x90, 0x40, 0xb0),
];
const CELL: (i32, i32) = (16, 12);

const TOP: i32 = 44;
const MAG_TOP: i32 = TOP + 120;

/// The preview's rect: the swatch in a 2 px sunken frame, at the group's right.
pub fn preview_rect(a: Area) -> Rect {
    let w = CELL.0 * SAMPLES.len() as i32 + 4;
    Rect::new(a.gright() - w, TOP + 16, w, 2 * CELL.1 + 4)
}

const INTENSITY: [&str; 3] = [
    "cf_intensity_label",
    "color_filter_intensity",
    "cf_intensity_value",
];

pub fn page(a: Area) -> Form {
    let gw = a.gw();
    let track = Rect::new(
        GX + 64,
        TOP + 52,
        a.track(Area::MIN.gw() - 64 - 48),
        trackbar::HEIGHT,
    );
    Form::new(vec![
        group(a, "cf_group", TOP, 110, "Color Filter"),
        Item::label_at("color_filter_label", GX, TOP + 22, 70, "Color fi&lter:"),
        choice("color_filter", GX + 74, TOP + 18, 150, FILTERS),
        Item::new(
            PREVIEW,
            preview_rect(a),
            Control::Picture {
                canvas: Some(swatch(Filter::None, 1.0)),
                frame: 2,
            },
        ),
        Item::label_at("cf_intensity_label", GX, TOP + 58, 60, "&Intensity:"),
        Item::new(
            "color_filter_intensity",
            track,
            Control::TrackBar(TrackBar::new(0, 101)),
        ),
        Item::value_at("cf_intensity_value", track.right() + 4, TOP + 58, 44, ""),
        Item::new(
            "cf_note",
            Rect::new(GX, TOP + 88, gw, LINE_H),
            Control::Lines(vec![
                "Screenshots and screen sharing see the colors as they are.".into(),
            ]),
        ),
        group(a, "mag_group", MAG_TOP, 93, "Magnifier"),
        Item::check(
            "magnifier_follow",
            GX,
            MAG_TOP + 20,
            "When zoomed in, &follow the text cursor and focus",
        ),
        Item::new(
            MAG_NOTE,
            Rect::new(GX, MAG_TOP + 42, gw, NOTE_LINES as i32 * LINE_H),
            Control::Lines(Vec::new()),
        ),
    ])
}

const MAG_NOTE: &str = "mag_note";
/// The zoom note's most lines, and the characters a line holds: the default note's first line, which fits the group in every look.
const NOTE_LINES: usize = 3;
const NOTE_COLS: usize = 58;

pub fn show_zoom_keys(page: &mut Form, binds: &[Shortcut]) {
    page.set_lines(MAG_NOTE, &zoom_note(binds));
}

/// The zoom note's lines: the keys bound to zoom-in, zoom-out and zoom-reset, then where to change
/// them. A note too long for the group drops the pointer to the Keyboard page first.
fn zoom_note(binds: &[Shortcut]) -> Vec<String> {
    let keys = |action: &str| {
        let shown: Vec<String> = (binds.iter())
            .filter(|b| b.action == action)
            .map(|b| shown_keys(&b.keys))
            .collect();
        (!shown.is_empty()).then(|| shown.join(" or "))
    };
    let parts: Vec<String> = [
        ("zoom-in", "zooms in, up to 8x"),
        ("zoom-out", "zooms out"),
        ("zoom-reset", "goes back to 1x"),
    ]
    .into_iter()
    .filter_map(|(action, does)| keys(action).map(|k| format!("{k} {does}")))
    .collect();
    let sentence = match parts.as_slice() {
        [] => return vec!["No keys zoom. The Keyboard page binds them.".into()],
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    };
    let wrap = |body: &str| reprisekit::text::wrap_chars(body, NOTE_COLS);
    let lines = wrap(&format!(
        "{sentence}. The Keyboard page changes these keys."
    ));
    let mut lines = if lines.len() <= NOTE_LINES {
        lines
    } else {
        wrap(&format!("{sentence}."))
    };
    lines.truncate(NOTE_LINES);
    lines
}

/// `keys` as the note names them: a key that types a character shows it ("Super+equal" reads "Super+=").
fn shown_keys(keys: &str) -> String {
    let (mods, key) = keys.rsplit_once('+').unwrap_or(("", keys));
    let key = key.trim();
    let typed = repriseconf::keybind::keysym(key)
        .filter(|sym| (0x21..=0x7e).contains(sym))
        .and_then(char::from_u32);
    let key = typed.map_or_else(|| key.to_string(), String::from);
    if mods.is_empty() {
        key
    } else {
        format!("{}+{key}", mods.trim_end())
    }
}

/// The preview's bitmap: the sample colours over the same colours through `filter` at `intensity` (0 to 1).
pub fn swatch(filter: Filter, intensity: f64) -> Canvas {
    let (w, h) = CELL;
    let mut c = Canvas::new((w * SAMPLES.len() as i32) as u32, (2 * h) as u32);
    let m = colorfilter::matrix(filter, intensity);
    for (i, &colour) in SAMPLES.iter().enumerate() {
        let x = i as i32 * w;
        c.fill_rect(Rect::new(x, 0, w, h), colour);
        let shown = m.map_or(colour, |m| {
            let [r, g, b] = m.apply([colour.r, colour.g, colour.b].map(|v| f32::from(v) / 255.0));
            let q = |v: f32| (v * 255.0).round() as u8;
            Color::rgb(q(r), q(g), q(b))
        });
        c.fill_rect(Rect::new(x, h, w, h), shown);
    }
    c
}

/// Grey the intensity unless a correction is picked, show its percent, and draw the preview through the pending filter.
pub fn follow(page: &mut Form, s: &Settings) {
    let filter = Filter::parse(&s.color_filter).unwrap_or_default();
    for id in INTENSITY {
        page.set_enabled(id, filter.takes_intensity());
    }
    let percent = s.color_filter_intensity.clamp(0, 100);
    page.set_text("cf_intensity_value", &format!("{percent}%"));
    if let Some(item) = page.item_mut(PREVIEW) {
        item.control = Control::Picture {
            canvas: Some(swatch(filter, percent as f64 / 100.0)),
            frame: 2,
        };
    }
    show_zoom_keys(page, &s.shortcuts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_combo_lists_every_filter_in_order() {
        let names: Vec<&str> = FILTERS.iter().map(|c| c.0).collect();
        let want: Vec<&str> = Filter::ALL.iter().map(|f| f.name()).collect();
        assert_eq!(names, want);
        for (name, label) in FILTERS {
            assert_eq!(Filter::parse(name).unwrap().label(), *label);
        }
    }

    #[test]
    fn the_swatch_shows_the_colours_over_their_filtered_selves() {
        let c = swatch(Filter::Grayscale, 1.0);
        assert_eq!((c.width, c.height), (96, 24));
        assert_eq!(c.get(0, 0), 0xffd0_3030, "red above");
        assert_eq!(c.get(0, 12), 0xff52_5252, "grey below");
        assert_eq!(c.get(95, 23), 0xff59_5959, "purple's grey in the last cell");
        let none = swatch(Filter::None, 1.0);
        assert_eq!(none.get(20, 20), none.get(20, 2), "no filter: the same");
    }

    #[test]
    fn the_zoom_note_names_the_default_keys() {
        let binds: Vec<Shortcut> = (repriseconf::keybind::DEFAULTS.iter())
            .map(|(k, a)| Shortcut::new(k, a, ""))
            .collect();
        assert_eq!(
            zoom_note(&binds),
            [
                "Super+= zooms in, up to 8x, Super+- zooms out and Super+0",
                "goes back to 1x. The Keyboard page changes these keys.",
            ]
        );
    }

    #[test]
    fn the_zoom_note_follows_rebound_and_unbound_keys() {
        let binds = [
            Shortcut::new("Ctrl+Alt+plus", "zoom-in", ""),
            Shortcut::new("Super+KP_Add", "zoom-in", ""),
            Shortcut::new("Ctrl+Alt+Home", "zoom-reset", ""),
        ];
        assert_eq!(
            zoom_note(&binds),
            [
                "Ctrl+Alt++ or Super+KP_Add zooms in, up to 8x and",
                "Ctrl+Alt+Home goes back to 1x. The Keyboard page changes",
                "these keys.",
            ]
        );
        assert_eq!(
            zoom_note(&[Shortcut::new("Super+Return", "terminal", "")]),
            ["No keys zoom. The Keyboard page binds them."]
        );
    }

    #[test]
    fn a_zoom_note_too_long_drops_the_keyboard_page_and_keeps_three_lines() {
        let long = "Super+Ctrl+Alt+Shift+XF86AudioRaiseVolume";
        let binds = [
            Shortcut::new(long, "zoom-in", ""),
            Shortcut::new(long, "zoom-out", ""),
            Shortcut::new(long, "zoom-reset", ""),
        ];
        let lines = zoom_note(&binds);
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|l| !l.contains("Keyboard")), "{lines:?}");
    }
}
