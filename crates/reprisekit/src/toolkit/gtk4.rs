//! The `gtk-4.0/gtk.css` that `[toolkits] gtk4_colors = true` hands GTK4 and
//! libadwaita programs: its named colours (as `@define-color` and `--var`) set
//! from the scheme in force. Pure text.
//!
//! Pages take the face, lists and cards the paper, popovers the menu, header bars
//! the caption (Glass: the face), the accent the selection. The scheme's own pairs
//! stand unless under 2.5:1; colours Reprise picks read at 4.5:1 (header text on
//! an inactive caption 3:1), else black, white or the active caption stands in.
//! Windows are square under every interface since Reprise draws their frame;
//! Classic squares dialogs, sheets and popovers too. Where paper and face match
//! (high-contrast schemes), cards and sheets get a 1 px text-colour edge.

use crate::interface::Interface;
use crate::theme::{Color, ColorScheme};

pub const MARKER: &str = "/* Written by Reprise for [toolkits] gtk4_colors = true; Reprise rewrites it when the look changes and removes it when the key is off. */";

const READABLE: f64 = 4.5;
const READABLE_BACKDROP: f64 = 3.0;
/// The least contrast a pair the scheme makes itself keeps: Glass's white on
/// 3399FF (2.9:1) stands, High Contrast #2's white on cyan doesn't.
const OWN_PAIR: f64 = 2.5;
/// Under this contrast between a card's paper and its face, cards, sheets and
/// popovers get their own edge; the high-contrast schemes sit at 1:1.
const CARD_GROUND: f64 = 1.05;
/// The least contrast a border keeps against the face and the paper; High Contrast Black's
/// black shadow on black falls back to its grey highlight.
const BORDER_LEAST: f64 = 1.5;

const SQUARE_WINDOWS: &[&str] = &[
    "window.dialog.message.csd",
    "window.messagedialog.csd",
    "window.dialog-window.alert.csd",
    "window.dialog-window.alert .body-scrolled-window > viewport",
];

const CLASSIC_SQUARE: &[&str] = &[
    "dialog-host > dialog.alert sheet",
    "dialog-host > dialog.alert sheet .body-scrolled-window > viewport",
    "floating-sheet > sheet",
    "bottom-sheet > sheet",
    "bottom-sheet > sheet > outline",
    "bottom-sheet > sheet > stack > widget",
    "bottom-sheet > sheet > stack > button",
    "bottom-sheet > sheet > stack > button > widget",
    "popover > contents",
    "menubar > item popover.menu popover.menu > contents",
    "colorchooser .popover.osd",
];

const SHADOW_EDGED: &[&str] = &[
    "list.boxed-list",
    "list.content",
    "list.boxed-list-separate > row",
    ".card",
    "floating-sheet > sheet",
    "bottom-sheet > sheet",
];

const BORDER_EDGED: &[&str] = &["popover > arrow", "popover > contents"];

const BLACK: Color = Color::rgb(0, 0, 0);
const WHITE: Color = Color::rgb(255, 255, 255);

const DESTRUCTIVE_LIGHT: Color = Color::rgb(0xe0, 0x1b, 0x24);
const DESTRUCTIVE_DARK: Color = Color::rgb(0xc0, 0x1c, 0x28);
const DESTRUCTIVE_TEXT_LIGHT: [Color; 2] =
    [Color::rgb(0xc0, 0x1c, 0x28), Color::rgb(0xa5, 0x1d, 0x2d)];
const DESTRUCTIVE_TEXT_DARK: [Color; 2] =
    [Color::rgb(0xf6, 0x61, 0x51), Color::rgb(0xff, 0x7b, 0x63)];

fn readable(inks: &[Color], grounds: &[Color], least: f64) -> Color {
    let worst = |ink: Color| {
        grounds
            .iter()
            .map(|g| ink.contrast(*g))
            .fold(f64::INFINITY, f64::min)
    };
    inks.iter()
        .copied()
        .find(|&ink| worst(ink) >= least)
        .unwrap_or_else(|| {
            if worst(BLACK) >= worst(WHITE) {
                BLACK
            } else {
                WHITE
            }
        })
}

pub fn colors(cs: &ColorScheme, interface: Interface) -> Vec<(&'static str, Color)> {
    let (caption, caption_text, inactive) = if interface == Interface::Glass {
        (cs.button_face, cs.button_text, cs.button_face)
    } else {
        (cs.active_title, cs.title_text, cs.inactive_title)
    };
    let header_text = readable(&[caption_text], &[caption], OWN_PAIR);
    let least = READABLE_BACKDROP.min(header_text.contrast(caption));
    let backdrop = if header_text.contrast(inactive) >= least {
        inactive
    } else {
        caption
    };
    let paper = [cs.window, cs.button_face];
    let (destructive, destructive_text) = if cs.is_dark() {
        (DESTRUCTIVE_DARK, DESTRUCTIVE_TEXT_DARK)
    } else {
        (DESTRUCTIVE_LIGHT, DESTRUCTIVE_TEXT_LIGHT)
    };
    let mut colors = vec![
        ("accent_bg_color", cs.hilight),
        (
            "accent_fg_color",
            readable(&[cs.hilight_text], &[cs.hilight], OWN_PAIR),
        ),
        (
            "accent_color",
            readable(
                &[cs.hilight, cs.hot_light, cs.window_text],
                &paper,
                READABLE,
            ),
        ),
        ("destructive_bg_color", destructive),
        (
            "destructive_fg_color",
            readable(&[WHITE], &[destructive], READABLE),
        ),
        (
            "destructive_color",
            readable(
                &[destructive_text[0], destructive_text[1], cs.window_text],
                &paper,
                READABLE,
            ),
        ),
        ("window_bg_color", cs.button_face),
        ("window_fg_color", cs.button_text),
        ("view_bg_color", cs.window),
        ("view_fg_color", cs.window_text),
        ("headerbar_bg_color", caption),
        ("headerbar_fg_color", header_text),
        ("headerbar_border_color", header_text),
        ("headerbar_backdrop_color", backdrop),
        ("sidebar_bg_color", cs.window),
        ("sidebar_fg_color", cs.window_text),
        ("sidebar_backdrop_color", cs.window),
        ("sidebar_border_color", border(cs)),
        ("secondary_sidebar_bg_color", cs.window),
        ("secondary_sidebar_fg_color", cs.window_text),
        ("secondary_sidebar_backdrop_color", cs.window),
        ("secondary_sidebar_border_color", border(cs)),
        ("card_bg_color", cs.window),
        ("card_fg_color", cs.window_text),
        ("dialog_bg_color", cs.button_face),
        ("dialog_fg_color", cs.button_text),
        ("popover_bg_color", cs.menu),
        ("popover_fg_color", cs.menu_text),
        ("thumbnail_bg_color", cs.window),
        ("thumbnail_fg_color", cs.window_text),
        ("border_color", border(cs)),
    ];
    if edge(cs).is_some() {
        // Boxed-list rows part on the card's shade, a faint black that vanishes on black paper.
        colors.push(("card_shade_color", border(cs)));
    }
    colors
}

/// The scheme's border colour: its shadow, unless that vanishes on the face or the paper.
pub(super) fn border(cs: &ColorScheme) -> Color {
    readable(
        &[cs.button_shadow, cs.button_hilight, cs.window_text],
        &[cs.button_face, cs.window],
        BORDER_LEAST,
    )
}

fn edge(cs: &ColorScheme) -> Option<Color> {
    (cs.window.contrast(cs.button_face) < CARD_GROUND).then(|| {
        readable(
            &[cs.window_text, cs.button_text],
            &[cs.window, cs.button_face, cs.menu],
            READABLE,
        )
    })
}

fn hex(c: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

fn rule(selectors: &[&str], declaration: &str) -> String {
    format!("\n{} {{\n  {declaration}\n}}\n", selectors.join(",\n"))
}

pub fn css(cs: &ColorScheme, interface: Interface) -> String {
    let colors = colors(cs, interface);
    let mut out = format!("{MARKER}\n\n");
    for (name, c) in &colors {
        out += &format!("@define-color {name} {};\n", hex(*c));
    }
    out += "\n:root {\n";
    for (name, c) in &colors {
        out += &format!("  --{}: {};\n", name.replace('_', "-"), hex(*c));
    }
    out += "  --window-radius: 0px;\n}\n";
    out += &rule(SQUARE_WINDOWS, "border-radius: 0;");
    if interface == Interface::Classic {
        out += &rule(CLASSIC_SQUARE, "border-radius: 0;");
    }
    if let Some(edge) = edge(cs) {
        let edge = hex(edge);
        out += &rule(SHADOW_EDGED, &format!("box-shadow: 0 0 0 1px {edge};"));
        out += &rule(BORDER_EDGED, &format!("border-color: {edge};"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{blue, glass, interfaces, schemes};

    fn get(cs: &ColorScheme, interface: Interface, name: &str) -> Color {
        colors(cs, interface)
            .into_iter()
            .find(|(n, _)| *n == name)
            .unwrap_or_else(|| panic!("no {name}"))
            .1
    }

    #[test]
    fn standard_classic() {
        let css = css(&ColorScheme::default(), Interface::Classic);
        assert_eq!(css.lines().next(), Some(MARKER));
        for want in [
            "@define-color window_bg_color #d4d0c8;",
            "@define-color window_fg_color #000000;",
            "@define-color view_bg_color #ffffff;",
            "@define-color view_fg_color #000000;",
            "@define-color headerbar_bg_color #0a246a;",
            "@define-color headerbar_fg_color #ffffff;",
            "@define-color headerbar_backdrop_color #808080;",
            "@define-color accent_bg_color #0a246a;",
            "@define-color accent_fg_color #ffffff;",
            "@define-color accent_color #0a246a;",
            "@define-color destructive_bg_color #e01b24;",
            "@define-color destructive_color #a51d2d;",
            "@define-color card_bg_color #ffffff;",
            "@define-color sidebar_bg_color #ffffff;",
            "@define-color popover_bg_color #d4d0c8;",
            "@define-color dialog_bg_color #d4d0c8;",
            "@define-color border_color #808080;",
            "  --window-bg-color: #d4d0c8;",
            "  --headerbar-bg-color: #0a246a;",
            "  --accent-color: #0a246a;",
            "  --border-color: #808080;",
        ] {
            assert!(css.lines().any(|l| l == want), "missing {want}");
        }
        assert!(css.ends_with("}\n"));
    }

    #[test]
    fn each_colour_is_a_name_and_a_variable() {
        let css = css(&ColorScheme::default(), Interface::Blue);
        for (name, c) in colors(&ColorScheme::default(), Interface::Blue) {
            let var = format!("  --{}: {};", name.replace('_', "-"), hex(c));
            assert!(css.lines().any(|l| l == var), "missing {var}");
            let named = format!("@define-color {name} {};", hex(c));
            assert!(css.lines().any(|l| l == named), "missing {named}");
        }
    }

    #[test]
    fn glass_dark_takes_its_face_for_header_bars() {
        let settings = glass::Settings {
            mode: glass::Mode::Dark,
            ..Default::default()
        };
        let cs = interfaces::theme_with(Interface::Glass, blue::Variant::Blue, &settings).colors;
        assert!(cs.is_dark());
        let at = |n| get(&cs, Interface::Glass, n);
        assert_eq!(at("window_bg_color"), cs.button_face);
        assert_eq!(at("view_bg_color"), cs.window);
        assert_eq!(at("headerbar_bg_color"), cs.button_face);
        assert_eq!(at("headerbar_fg_color"), cs.button_text);
        assert_eq!(at("headerbar_backdrop_color"), cs.button_face);
        assert_eq!(
            (at("accent_bg_color"), at("accent_fg_color")),
            (cs.hilight, cs.hilight_text),
            "white on the selection's blue stands"
        );
        assert_eq!(at("destructive_bg_color"), DESTRUCTIVE_DARK);
        assert_eq!(
            at("destructive_color"),
            DESTRUCTIVE_TEXT_DARK[1],
            "F66151 does not read on its face"
        );
    }

    #[test]
    fn blue_header_bars_take_the_caption() {
        let cs = interfaces::blue_theme(blue::Variant::Silver).colors;
        assert_eq!(
            get(&cs, Interface::Blue, "headerbar_bg_color"),
            cs.active_title
        );
    }

    #[test]
    fn high_contrast_pairs_stay_readable() {
        let hc2 = schemes::scheme("High Contrast #2").unwrap();
        assert_eq!(get(&hc2, Interface::Classic, "headerbar_fg_color"), BLACK);
        assert_eq!(get(&hc2, Interface::Classic, "accent_fg_color"), BLACK);
        assert_eq!(
            get(&hc2, Interface::Classic, "headerbar_backdrop_color"),
            hc2.active_title,
            "black does not read on its blue inactive caption"
        );
        let hcb = schemes::scheme("High Contrast Black").unwrap();
        assert_eq!(get(&hcb, Interface::Classic, "accent_color"), WHITE);
    }

    #[test]
    fn high_contrast_black_parts_its_sidebars_and_list_rows_in_grey() {
        let hcb = schemes::scheme("High Contrast Black").unwrap();
        for name in ["border_color", "sidebar_border_color", "card_shade_color"] {
            assert_eq!(
                get(&hcb, Interface::Classic, name),
                hcb.button_hilight,
                "{name}"
            );
        }
        let std = ColorScheme::default();
        assert_eq!(
            get(&std, Interface::Classic, "border_color"),
            std.button_shadow
        );
        assert!(
            !colors(&std, Interface::Classic)
                .iter()
                .any(|(n, _)| *n == "card_shade_color"),
            "the card's own shade stands"
        );
    }

    #[test]
    fn windows_are_square_and_classic_squares_popovers() {
        let cs = ColorScheme::default();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let css = css(&cs, i);
            assert!(
                css.contains("  --border-color: #808080;\n  --window-radius: 0px;\n}\n"),
                "{i:?}: :root ends with a window radius of 0"
            );
            assert!(
                css.contains(
                    "\nwindow.dialog.message.csd,\nwindow.messagedialog.csd,\n\
                     window.dialog-window.alert.csd,\n\
                     window.dialog-window.alert .body-scrolled-window > viewport {\n  \
                     border-radius: 0;\n}\n"
                ),
                "{i:?}: message and alert windows are square"
            );
            assert_eq!(
                css.contains("\npopover > contents,\n"),
                i == Interface::Classic,
                "{i:?}: popovers are square under Classic only"
            );
        }
        let classic = css(&cs, Interface::Classic);
        for sel in [
            "dialog-host > dialog.alert sheet,",
            "floating-sheet > sheet,",
            "bottom-sheet > sheet,",
            "popover > contents,",
            "colorchooser .popover.osd {",
        ] {
            assert!(classic.lines().any(|l| l == sel), "Classic squares {sel}");
        }
    }

    #[test]
    fn high_contrast_surfaces_get_an_edge_in_the_text_colour() {
        let hcb = css(
            &schemes::scheme("High Contrast Black").unwrap(),
            Interface::Classic,
        );
        assert!(hcb.ends_with(
            "\nlist.boxed-list,\nlist.content,\nlist.boxed-list-separate > row,\n.card,\n\
             floating-sheet > sheet,\nbottom-sheet > sheet {\n  \
             box-shadow: 0 0 0 1px #ffffff;\n}\n\
             \npopover > arrow,\npopover > contents {\n  border-color: #ffffff;\n}\n"
        ));
        let hcw = css(
            &schemes::scheme("High Contrast White").unwrap(),
            Interface::Classic,
        );
        assert!(hcw.contains("bottom-sheet > sheet {\n  box-shadow: 0 0 0 1px #000000;\n}\n"));
        assert!(hcw.ends_with("popover > contents {\n  border-color: #000000;\n}\n"));
        for name in ["High Contrast #1", "High Contrast #2"] {
            let cs = schemes::scheme(name).unwrap();
            assert_eq!(edge(&cs), Some(cs.window_text), "{name}");
        }
    }

    #[test]
    fn only_high_contrast_draws_edges() {
        let mut edged: Vec<String> = looks()
            .into_iter()
            .filter(|(name, cs, i)| {
                let css = css(cs, *i);
                let drawn = css.contains("  box-shadow:") || css.contains("  border-color:");
                assert_eq!(edge(cs).is_some(), drawn, "{name}");
                drawn
            })
            .map(|(name, _, _)| name)
            .collect();
        edged.sort();
        assert_eq!(
            edged,
            [
                "High Contrast #1",
                "High Contrast #2",
                "High Contrast Black",
                "High Contrast White"
            ]
        );
    }

    fn looks() -> Vec<(String, ColorScheme, Interface)> {
        let mut looks: Vec<(String, ColorScheme, Interface)> = schemes::names()
            .map(|n| {
                (
                    n.to_string(),
                    schemes::scheme(n).unwrap(),
                    Interface::Classic,
                )
            })
            .collect();
        for v in blue::Variant::ALL {
            looks.push((
                v.name().into(),
                interfaces::blue_theme(v).colors,
                Interface::Blue,
            ));
        }
        for mode in [glass::Mode::Light, glass::Mode::Dark] {
            let settings = glass::Settings {
                mode,
                ..Default::default()
            };
            let cs = interfaces::theme_with(Interface::Glass, blue::Variant::Blue, &settings);
            looks.push((format!("Glass {mode:?}"), cs.colors, Interface::Glass));
        }
        looks
    }

    #[test]
    fn every_look_keeps_its_picked_pairs_readable() {
        for (name, cs, i) in &looks() {
            let at = |n| get(cs, *i, n);
            for (ink, ground, least) in [
                ("accent_fg_color", "accent_bg_color", OWN_PAIR),
                ("accent_color", "view_bg_color", READABLE),
                ("accent_color", "window_bg_color", READABLE),
                ("destructive_fg_color", "destructive_bg_color", READABLE),
                ("destructive_color", "view_bg_color", READABLE),
                ("headerbar_fg_color", "headerbar_bg_color", OWN_PAIR),
                ("headerbar_fg_color", "headerbar_backdrop_color", OWN_PAIR),
            ] {
                let ratio = at(ink).contrast(at(ground));
                assert!(
                    ratio >= least,
                    "{name}: {ink} on {ground} reads at {ratio:.2}"
                );
            }
        }
    }
}
