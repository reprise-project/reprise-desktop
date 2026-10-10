//! The GTK3 theme and Qt palette `[toolkits] ... = "reprise"` hands GTK and Qt programs, generated
//! from the colour scheme in force. Pure text: the compositor writes it where GTK, qt6ct and qt5ct
//! read it. GTK3 gets recoloured Adwaita plus `gtk3.css`; Qt gets a qt5ct/qt6ct scheme of the 21
//! `QPalette` roles.

use crate::interface::Interface;
use crate::theme::{Color, ColorScheme, FontConfig};

pub mod gtk4;

const GTK3_RULES: &str = include_str!("toolkit/gtk3.css");

/// The first line of every file Reprise writes for qt5ct/qt6ct; a `qt6ct.conf` without it is the
/// user's own and stays.
pub const QT_CONF_MARKER: &str = "; Written by Reprise for [toolkits] qt_platform_theme = \"reprise\"; Reprise rewrites it when the look changes.";

fn hex(c: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

pub fn gtk3_css(cs: &ColorScheme, interface: Interface) -> String {
    let mut out = recoloured_adwaita(cs, interface);
    if interface == Interface::Classic {
        out += &crate::classic::gtk3::bevels(cs);
    }
    out
}

pub fn gtk3_assets(cs: &ColorScheme, interface: Interface) -> Vec<(String, Vec<u8>)> {
    if interface != Interface::Classic {
        return Vec::new();
    }
    let dir = crate::classic::gtk3::ASSETS;
    crate::classic::gtk3::assets(cs)
        .into_iter()
        .map(|(name, bytes)| (format!("{dir}/{name}"), bytes))
        .collect()
}

/// Adwaita (dark sheet when the scheme is dark), then the named colours and rules that read them.
/// Glass has no caption colour, so its header bars take the face.
fn recoloured_adwaita(cs: &ColorScheme, interface: Interface) -> String {
    let adwaita = if cs.is_dark() {
        "gtk-contained-dark.css"
    } else {
        "gtk-contained.css"
    };
    let glass = interface == Interface::Glass;
    let (caption, caption_text, inactive, inactive_text) = if glass {
        (
            cs.button_face,
            cs.button_text,
            cs.button_face,
            cs.button_text,
        )
    } else {
        (
            cs.active_title,
            cs.title_text,
            cs.inactive_title,
            cs.inactive_title_text,
        )
    };
    let named = [
        ("theme_bg_color", cs.button_face),
        ("theme_fg_color", cs.button_text),
        ("theme_base_color", cs.window),
        ("theme_text_color", cs.window_text),
        ("theme_selected_bg_color", cs.hilight),
        ("theme_selected_fg_color", cs.hilight_text),
        ("insensitive_bg_color", cs.button_face),
        ("insensitive_fg_color", cs.gray_text),
        ("insensitive_base_color", cs.button_face),
        ("theme_unfocused_fg_color", cs.button_text),
        ("theme_unfocused_text_color", cs.window_text),
        ("theme_unfocused_bg_color", cs.button_face),
        ("theme_unfocused_base_color", cs.window),
        ("theme_unfocused_selected_bg_color", cs.hilight),
        ("theme_unfocused_selected_fg_color", cs.hilight_text),
        ("unfocused_insensitive_color", cs.gray_text),
        ("borders", cs.button_shadow),
        ("unfocused_borders", cs.button_shadow),
        ("link_color", cs.hot_light),
        ("content_view_bg", cs.window),
        ("text_view_bg", cs.window),
        ("wm_title", caption_text),
        ("wm_unfocused_title", inactive_text),
        ("reprise_hilight", cs.button_hilight),
        ("reprise_shadow", cs.button_shadow),
        ("reprise_dk_shadow", cs.button_dk_shadow),
        ("reprise_menu", cs.menu),
        ("reprise_menu_text", cs.menu_text),
        ("reprise_menu_hilight", cs.menu_hilight),
        ("reprise_menu_hilight_text", cs.hilight_text),
        ("reprise_menu_bar", cs.menu_bar),
        ("reprise_tooltip", cs.info_bk),
        ("reprise_tooltip_text", cs.info_text),
        ("reprise_caption", caption),
        ("reprise_caption_text", caption_text),
        ("reprise_inactive_caption", inactive),
        ("reprise_inactive_caption_text", inactive_text),
    ];
    let mut out = format!(
        "/* Reprise: generated from the look in force and rewritten when it changes. */\n\
         @import url(\"resource:///org/gtk/libgtk/theme/Adwaita/{adwaita}\");\n\n"
    );
    for (name, c) in named {
        out += &format!("@define-color {name} {};\n", hex(c));
    }
    out.push('\n');
    out += GTK3_RULES;
    let gradient = match interface {
        Interface::Classic => Some("to right"),
        Interface::Blue => Some("to top"),
        Interface::Glass => None,
    };
    if let Some(dir) = gradient {
        out += &format!(
            "\n/* The caption's gradient */\n\
             .titlebar:not(headerbar), headerbar {{\n  \
             background-image: linear-gradient({dir}, {}, {});\n}}\n\
             .titlebar:backdrop:not(headerbar), headerbar:backdrop {{\n  \
             background-image: linear-gradient({dir}, {}, {});\n}}\n",
            hex(cs.active_title),
            hex(cs.gradient_active_title),
            hex(cs.inactive_title),
            hex(cs.gradient_inactive_title),
        );
    }
    out
}

fn qt_roles(cs: &ColorScheme, disabled: bool) -> [Color; 21] {
    let grey = |c: Color| if disabled { cs.gray_text } else { c };
    let f = cs.button_face;
    let mid = Color::rgb(
        (f.r as u16 * 2 / 3) as u8,
        (f.g as u16 * 2 / 3) as u8,
        (f.b as u16 * 2 / 3) as u8,
    );
    [
        grey(cs.button_text),                  // WindowText
        cs.button_face,                        // Button
        cs.button_hilight,                     // Light
        cs.button_light,                       // Midlight
        cs.button_shadow,                      // Dark
        mid,                                   // Mid
        grey(cs.window_text),                  // Text
        cs.button_hilight,                     // BrightText
        grey(cs.button_text),                  // ButtonText
        cs.window,                             // Base
        cs.button_face,                        // Window
        cs.button_dk_shadow,                   // Shadow
        cs.hilight,                            // Highlight
        cs.hilight_text,                       // HighlightedText
        cs.hot_light,                          // Link
        cs.hot_light,                          // LinkVisited
        cs.window.toward(cs.button_face, 128), // AlternateBase
        Color::rgb(0, 0, 0),                   // NoRole
        cs.info_bk,                            // ToolTipBase
        cs.info_text,                          // ToolTipText
        cs.gray_text,                          // PlaceholderText
    ]
}

pub fn qt_colors(cs: &ColorScheme) -> String {
    let line = |disabled| {
        qt_roles(cs, disabled)
            .iter()
            .map(|c| format!("#ff{:02x}{:02x}{:02x}", c.r, c.g, c.b))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "{QT_CONF_MARKER}\n[ColorScheme]\nactive_colors={}\ndisabled_colors={}\ninactive_colors={}\n",
        line(false),
        line(true),
        line(false),
    )
}

pub fn qt_conf(colors: &str, icon_theme: &str, font: Option<&FontConfig>) -> String {
    let mut conf = format!(
        "{QT_CONF_MARKER}\n[Appearance]\ncolor_scheme_path={colors}\ncustom_palette=true\nicon_theme={icon_theme}\nstyle=Fusion\n"
    );
    if let Some(f) = font {
        let family = f.family.first().map_or("Sans", String::as_str);
        let pt = f64::from(f.size_px) * 0.75;
        conf += &format!("[Fonts]\ngeneral=\"{family},{pt}\"\n");
    }
    conf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line<'a>(text: &'a str, start: &str) -> &'a str {
        text.lines()
            .find(|l| l.starts_with(start))
            .unwrap_or_else(|| panic!("no line starting {start:?}"))
    }

    #[test]
    fn standard_gtk3_lays_its_colours_over_light_adwaita() {
        let css = gtk3_css(&ColorScheme::default(), Interface::Classic);
        assert_eq!(
            css.lines().nth(1),
            Some("@import url(\"resource:///org/gtk/libgtk/theme/Adwaita/gtk-contained.css\");")
        );
        for want in [
            "@define-color theme_bg_color #d4d0c8;",
            "@define-color theme_fg_color #000000;",
            "@define-color theme_base_color #ffffff;",
            "@define-color theme_text_color #000000;",
            "@define-color theme_selected_bg_color #0a246a;",
            "@define-color theme_selected_fg_color #ffffff;",
            "@define-color insensitive_fg_color #808080;",
            "@define-color borders #808080;",
            "@define-color link_color #000080;",
            "@define-color reprise_menu #d4d0c8;",
            "@define-color reprise_tooltip #ffffe1;",
            "@define-color reprise_caption #0a246a;",
            "@define-color reprise_caption_text #ffffff;",
        ] {
            assert!(css.lines().any(|l| l == want), "missing {want}");
        }
        assert!(
            css.contains("background-image: linear-gradient(to right, #0a246a, #a6caf0);"),
            "the caption's gradient left to right"
        );
        assert!(css.contains(GTK3_RULES));
    }

    #[test]
    fn a_dark_scheme_takes_adwaitas_dark_sheet() {
        let cs = crate::schemes::scheme("Tokyo Night").unwrap();
        let css = gtk3_css(&cs, Interface::Classic);
        assert!(css.contains("Adwaita/gtk-contained-dark.css"));
        assert_eq!(
            line(&css, "@define-color theme_base_color"),
            format!("@define-color theme_base_color {};", hex(cs.window))
        );
        let glass = gtk3_css(&cs, Interface::Glass);
        assert!(!glass.contains("linear-gradient(to right"));
        assert_eq!(
            line(&glass, "@define-color reprise_caption "),
            format!("@define-color reprise_caption {};", hex(cs.button_face))
        );
    }

    #[test]
    fn the_rules_read_only_defined_colours() {
        let css = gtk3_css(&ColorScheme::default(), Interface::Classic);
        let defined: Vec<&str> = css
            .lines()
            .filter_map(|l| l.strip_prefix("@define-color "))
            .filter_map(|l| l.split(' ').next())
            .collect();
        for word in GTK3_RULES.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '@')) {
            if let Some(name) = word.strip_prefix('@') {
                assert!(defined.contains(&name), "@{name} is not defined");
            }
        }
    }

    #[test]
    fn a_marked_scales_knob_takes_the_face() {
        let css = gtk3_css(&ColorScheme::default(), Interface::Blue);
        for orient in ["horizontal", "vertical"] {
            for (side, other) in [("before", "after"), ("after", "before")] {
                for state in [
                    "",
                    ":hover",
                    ":active",
                    ":backdrop",
                    ":disabled",
                    ":backdrop:disabled",
                ] {
                    let sel =
                        format!("scale.{orient}.marks-{side}:not(.marks-{other}) slider{state}");
                    assert!(
                        css.contains(&format!("{sel},")) || css.contains(&format!("{sel} {{")),
                        "no rule for {sel}"
                    );
                }
            }
        }
        assert!(css.contains("  background-image: image(@theme_bg_color);\n  box-shadow"));
    }

    fn fnv(text: &str) -> u64 {
        text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
        })
    }

    #[test]
    fn blue_and_glass_sheets_are_pinned() {
        let tokyo = crate::schemes::scheme("Tokyo Night").unwrap();
        let std = ColorScheme::default();
        let got = [
            fnv(&gtk3_css(&std, Interface::Blue)),
            fnv(&gtk3_css(&std, Interface::Glass)),
            fnv(&gtk3_css(&tokyo, Interface::Glass)),
        ];
        assert_eq!(
            got,
            [
                0xb5f8_6215_bb74_2f37,
                0x8ef7_17bc_ea9c_4680,
                0xfeee_8e55_72ee_7ede
            ],
            "{got:#x?}"
        );
    }

    #[test]
    fn standard_qt_palette() {
        let text = qt_colors(&ColorScheme::default());
        assert_eq!(text.lines().next(), Some(QT_CONF_MARKER));
        assert_eq!(
            line(&text, "active_colors="),
            "active_colors=#ff000000, #ffd4d0c8, #ffffffff, #ffd4d0c8, #ff808080, \
             #ff8d8a85, #ff000000, #ffffffff, #ff000000, #ffffffff, #ffd4d0c8, \
             #ff404040, #ff0a246a, #ffffffff, #ff000080, #ff000080, #ffe9e7e3, \
             #ff000000, #ffffffe1, #ff000000, #ff808080"
        );
        assert_eq!(
            line(&text, "inactive_colors=")["inactive_".len()..],
            line(&text, "active_colors=")["active_".len()..]
        );
        let disabled: Vec<&str> = line(&text, "disabled_colors=")["disabled_colors=".len()..]
            .split(", ")
            .collect();
        assert_eq!(disabled.len(), 21);
        assert_eq!(
            (disabled[0], disabled[6], disabled[8]),
            ("#ff808080", "#ff808080", "#ff808080")
        );
        assert_eq!(disabled[9], "#ffffffff", "the paper stays");
    }

    #[test]
    fn a_dark_scheme_gives_qt_its_paper_and_text() {
        let cs = crate::schemes::scheme("Tokyo Night").unwrap();
        let roles: Vec<String> = line(&qt_colors(&cs), "active_colors=")["active_colors=".len()..]
            .split(", ")
            .map(String::from)
            .collect();
        let argb = |c: Color| format!("#ff{:02x}{:02x}{:02x}", c.r, c.g, c.b);
        assert_eq!(roles[9], argb(cs.window), "Base");
        assert_eq!(roles[6], argb(cs.window_text), "Text");
        assert_eq!(roles[10], argb(cs.button_face), "Window");
    }

    #[test]
    fn qt_conf_names_the_palette_and_the_font_under_the_marker() {
        assert_eq!(
            qt_conf("/home/u/.config/qt6ct/colors/Reprise.conf", "Papirus", None),
            format!(
                "{QT_CONF_MARKER}\n[Appearance]\ncolor_scheme_path=/home/u/.config/qt6ct/colors/Reprise.conf\n\
                 custom_palette=true\nicon_theme=Papirus\nstyle=Fusion\n"
            )
        );
        let big = FontConfig {
            size_px: 17,
            ..FontConfig::default()
        };
        assert!(
            qt_conf("c", "i", Some(&big))
                .ends_with("style=Fusion\n[Fonts]\ngeneral=\"Tahoma,12.75\"\n")
        );
    }
}
