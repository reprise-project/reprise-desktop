//! The `org.freedesktop.impl.portal.Settings` backend, served from the compositor's bus connection so GTK, Qt and browsers learn through xdg-desktop-portal whether the look is dark and what its accent is.
//! Three namespaces are answered, everything else is NotFound (`reprise-portals.conf` names no other Settings backend): `org.freedesktop.appearance`, `org.reprise.appearance` (the whole look, for Reprise's programs) and `org.gnome.desktop.interface` (theme, icons, cursor, font, font smoothing and overlay scrolling for GTK3, which the session points at the portal via `GTK_USE_PORTAL`).

use crate::config::{Config, FontSmoothing};
use reprisebus::appearance as look;
use reprisekit::blue::Variant;
use reprisekit::interface::Interface;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

pub const SETTINGS_NAME: &str = "org.freedesktop.impl.portal.desktop.reprise-settings";
pub const SETTINGS_PATH: &str = "/org/freedesktop/portal/desktop";
const NAMESPACE: &str = "org.freedesktop.appearance";
const GNOME: &str = "org.gnome.desktop.interface";

/// What the three namespaces say about the look. The freedesktop one is
/// in the portal's own units: `color_scheme` 1 prefer dark / 2 prefer
/// light, `contrast` 1 for the High Contrast schemes, `accent` sRGB in
/// 0..1. The Reprise one is the look as the environment hands it to a
/// program. The GNOME one is GTK3's theme, icons, cursor, font and font
/// smoothing.
#[derive(Clone, Debug, PartialEq)]
pub struct Appearance {
    pub color_scheme: u32,
    pub contrast: u32,
    pub accent: (f64, f64, f64),
    pub interface: Interface,
    pub blue_variant: Variant,
    pub colors: String,
    pub icon_theme: String,
    pub shell_dlg: String,
    /// `[fonts] family`, joined as [`look::FONT_FAMILY`] carries it.
    pub font_family: String,
    pub glass_mode: reprisekit::glass::Mode,
    pub glass_material: reprisekit::glass::Material,
    /// `[cursor] theme`, else `XCURSOR_THEME`, else "default".
    pub cursor_theme: String,
    /// `[cursor] size` in pixels, else `XCURSOR_SIZE`, else 24.
    pub cursor_size: i32,
    /// GTK's font name (family list and size in points) at the text size of
    /// 100%: GTK scales it by `text-scaling-factor`.
    pub font_name: String,
    pub text_size: u32,
    pub double_click_ms: i32,
    /// `[toolkits] gtk_theme`, "auto", "reprise" or a theme's name.
    pub gtk_theme: String,
    /// The theme directory holding the generated GTK theme
    /// (`crate::toolkits`), which "reprise" names; None until written.
    pub gtk_generated: Option<&'static str>,
    pub font_smoothing: FontSmoothing,
}

impl Appearance {
    /// Read the scheme: dark per `ColorScheme::is_dark`, high contrast
    /// when the scheme is one of the High Contrast four, the accent the
    /// glass colour under Glass, else the active caption's.
    pub fn of(cfg: &Config) -> Appearance {
        let colors = &cfg.theme.colors;
        let high_contrast =
            reprisekit::schemes::name_of(colors).is_some_and(|n| n.starts_with("High Contrast"));
        let a = if cfg.interface == Interface::Glass {
            cfg.glass.color
        } else {
            colors.active_title
        };
        let (cursor_theme, cursor_size) = cfg.cursor.effective();
        Appearance {
            color_scheme: if colors.is_dark() { 1 } else { 2 },
            contrast: high_contrast as u32,
            accent: (a.r as f64 / 255.0, a.g as f64 / 255.0, a.b as f64 / 255.0),
            interface: cfg.interface,
            blue_variant: cfg.blue_variant,
            colors: colors.to_env(),
            icon_theme: cfg.icons.theme.clone(),
            shell_dlg: cfg.theme.fonts.shell_dlg.clone().unwrap_or_default(),
            font_family: cfg.theme.fonts.family.join(","),
            glass_mode: cfg.glass.mode,
            glass_material: cfg.glass.material,
            cursor_theme,
            cursor_size: cursor_size as i32,
            font_name: font_name(&cfg.theme.fonts, cfg.accessibility.text_factor()),
            text_size: cfg.accessibility.text_size,
            double_click_ms: cfg.theme.metrics.double_click_ms,
            gtk_theme: cfg.toolkits.gtk_theme.clone(),
            gtk_generated: None,
            font_smoothing: cfg.toolkits.font_smoothing,
        }
    }

    /// Whether the look in force is dark: a dark scheme or dark Glass.
    fn dark(&self) -> bool {
        self.color_scheme == 1
    }

    pub fn live(state: &crate::compositor::State) -> Appearance {
        Appearance {
            gtk_generated: state.toolkits.gtk_theme(),
            ..Appearance::of(&state.config)
        }
    }

    /// GTK's theme: the configured one, the generated one for "reprise",
    /// else Adwaita for the scheme's darkness. The GTK portal backend gets
    /// it too (`spawn::theme_portal_gtk`).
    pub fn gtk_theme(&self) -> &str {
        crate::config::resolve_gtk_theme(&self.gtk_theme, self.dark(), self.gtk_generated)
    }

    /// Whether GTK reads the generated theme with Classic's edges
    /// (`reprisekit::toolkit::gtk3_css`).
    fn classic_gtk_theme(&self) -> bool {
        self.interface == Interface::Classic
            && self.gtk_theme == crate::config::TOOLKIT_REPRISE
            && self.gtk_generated.is_some()
    }

    fn keys(&self) -> Vec<(&'static str, &'static str, Value<'static>)> {
        let (antialiasing, hinting, rgba_order) = self.font_smoothing.gnome_keys();
        vec![
            (NAMESPACE, "color-scheme", Value::U32(self.color_scheme)),
            (NAMESPACE, "contrast", Value::U32(self.contrast)),
            (NAMESPACE, "accent-color", Value::from(self.accent)),
            (
                look::NAMESPACE,
                look::INTERFACE,
                Value::from(self.interface.as_str()),
            ),
            (
                look::NAMESPACE,
                look::BLUE_VARIANT,
                Value::from(self.blue_variant.as_str()),
            ),
            (
                look::NAMESPACE,
                look::COLORS,
                Value::from(self.colors.clone()),
            ),
            (
                look::NAMESPACE,
                look::ICON_THEME,
                Value::from(self.icon_theme.clone()),
            ),
            (
                look::NAMESPACE,
                look::SHELL_DLG,
                Value::from(self.shell_dlg.clone()),
            ),
            (
                look::NAMESPACE,
                look::FONT_FAMILY,
                Value::from(self.font_family.clone()),
            ),
            (
                look::NAMESPACE,
                look::GLASS_MODE,
                Value::from(self.glass_mode.as_str()),
            ),
            (
                look::NAMESPACE,
                look::GLASS_MATERIAL,
                Value::from(self.glass_material.as_str()),
            ),
            (
                look::NAMESPACE,
                look::TEXT_SIZE,
                Value::from(self.text_size.to_string()),
            ),
            (
                look::NAMESPACE,
                look::DOUBLE_CLICK_MS,
                Value::from(self.double_click_ms.to_string()),
            ),
            (
                GNOME,
                "color-scheme",
                Value::from(if self.dark() {
                    "prefer-dark"
                } else {
                    "default"
                }),
            ),
            (
                GNOME,
                "gtk-theme",
                Value::from(self.gtk_theme().to_string()),
            ),
            // Classic's generated theme draws its scroll bars beside the
            // content, never over it.
            (
                GNOME,
                "overlay-scrolling",
                Value::Bool(!self.classic_gtk_theme()),
            ),
            (GNOME, "icon-theme", Value::from(self.icon_theme.clone())),
            (
                GNOME,
                "cursor-theme",
                Value::from(self.cursor_theme.clone()),
            ),
            (GNOME, "cursor-size", Value::I32(self.cursor_size)),
            (GNOME, "font-name", Value::from(self.font_name.clone())),
            // GTK3 leaves text unsmoothed and unhinted at 0 dpi until the
            // portal names these, so every one is answered.
            (GNOME, "font-antialiasing", Value::from(antialiasing)),
            (GNOME, "font-hinting", Value::from(hinting)),
            (GNOME, "font-rgba-order", Value::from(rgba_order)),
            (
                GNOME,
                "text-scaling-factor",
                Value::F64(self.text_size as f64 / 100.0),
            ),
        ]
    }

    fn get(&self, namespace: &str, key: &str) -> Option<Value<'static>> {
        self.keys()
            .into_iter()
            .find(|(n, k, _)| *n == namespace && *k == key)
            .map(|(_, _, v)| v)
    }
}

/// GTK's font name for `f`, a Pango description: every family,
/// comma-separated, at its size in points (px · 3/4 at 96 dpi), so Pango
/// falls to the next family when one is missing ("<UI face>, DejaVu Sans
/// 8.25" for both at 11 px); "Sans" when the list is empty. `f` is at the
/// text size `factor`, which GTK applies itself, so the name gives the
/// size before it, to a hundredth of a point.
fn font_name(f: &reprisekit::theme::FontConfig, factor: f64) -> String {
    let families = if f.family.is_empty() {
        "Sans".to_string()
    } else {
        f.family.join(", ")
    };
    let pt = (f64::from(f.size_px) * 0.75 / factor * 100.0).round() / 100.0;
    format!("{families} {pt}")
}

#[derive(zbus::DBusError, Debug)]
#[zbus(prefix = "org.freedesktop.portal.Error")]
enum PortalError {
    #[zbus(error)]
    ZBus(zbus::Error),
    NotFound(String),
}

/// A `ReadAll` pattern matches a namespace when it is the namespace, or
/// a `prefix*` glob of it; no patterns at all means every namespace.
fn matches(patterns: &[String], namespace: &str) -> bool {
    patterns.is_empty()
        || patterns.iter().any(|p| match p.strip_suffix('*') {
            Some(prefix) => namespace.starts_with(prefix),
            None => p == namespace,
        })
}

pub struct Settings {
    pub current: Arc<Mutex<Appearance>>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Settings")]
impl Settings {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    fn read_all(&self, namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        let mut out: HashMap<String, HashMap<String, OwnedValue>> = HashMap::new();
        let keys = self
            .current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys();
        for (namespace, key, value) in keys {
            if matches(&namespaces, namespace)
                && let Ok(v) = value.try_to_owned()
            {
                out.entry(namespace.to_string())
                    .or_default()
                    .insert(key.to_string(), v);
            }
        }
        out
    }

    fn read_one(&self, namespace: &str, key: &str) -> Result<OwnedValue, PortalError> {
        self.lookup(namespace, key)
    }

    /// Version 1's reader. The frontend's own `Read` is the one that
    /// wraps the value in a second variant; the backend returns it bare,
    /// as xdg-desktop-portal-gtk does.
    fn read(&self, namespace: &str, key: &str) -> Result<OwnedValue, PortalError> {
        self.lookup(namespace, key)
    }

    #[zbus(signal)]
    pub async fn setting_changed(
        emitter: &SignalEmitter<'_>,
        namespace: &str,
        key: &str,
        value: Value<'_>,
    ) -> zbus::Result<()>;
}

impl Settings {
    fn lookup(&self, namespace: &str, key: &str) -> Result<OwnedValue, PortalError> {
        if ![NAMESPACE, GNOME, look::NAMESPACE].contains(&namespace) {
            return Err(PortalError::NotFound(format!("namespace {namespace}")));
        }
        match self
            .current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(namespace, key)
        {
            Some(v) => Ok(v.try_to_owned().map_err(zbus::Error::from)?),
            None => Err(PortalError::NotFound(format!("key {key}"))),
        }
    }
}

/// Take `next` as the current appearance and signal every key whose
/// value moved. Called on the D-Bus thread with the interface served.
pub async fn update(conn: &zbus::Connection, current: &Mutex<Appearance>, next: Appearance) {
    let prev = std::mem::replace(
        &mut *current.lock().unwrap_or_else(|e| e.into_inner()),
        next.clone(),
    );
    if prev == next {
        return;
    }
    let Ok(iface) = conn
        .object_server()
        .interface::<_, Settings>(SETTINGS_PATH)
        .await
    else {
        return;
    };
    let emitter = iface.signal_emitter();
    for (namespace, key, value) in changed(&prev, &next) {
        if let Err(e) = Settings::setting_changed(emitter, namespace, key, value).await {
            log::warn!("settings portal: SettingChanged {key}: {e}");
        }
    }
}

fn changed(
    prev: &Appearance,
    next: &Appearance,
) -> Vec<(&'static str, &'static str, Value<'static>)> {
    next.keys()
        .into_iter()
        .zip(prev.keys())
        .filter(|((_, _, value), (_, _, was))| value != was)
        .map(|(now, _)| now)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::theme::ColorScheme;

    fn of(colors: &ColorScheme, interface: Interface) -> Appearance {
        let mut c = crate::config::Config::default();
        c.theme.colors = colors.clone();
        c.interface = interface;
        Appearance::of(&c)
    }

    #[test]
    fn light_schemes_read_light_and_the_dark_ones_dark() {
        let standard = of(&ColorScheme::default(), Interface::Classic);
        assert_eq!(standard.color_scheme, 2);
        assert_eq!(standard.contrast, 0);
        assert_eq!(standard.accent, (10.0 / 255.0, 36.0 / 255.0, 106.0 / 255.0));
        for name in [
            "Tokyo Night",
            "Dracula",
            "Ayu Dark",
            "Nord",
            "High Contrast Black",
        ] {
            let a = of(
                &reprisekit::schemes::scheme(name).unwrap(),
                Interface::Classic,
            );
            assert_eq!(a.color_scheme, 1, "{name}");
        }
        for name in ["Brick", "Desert", "High Contrast White"] {
            let a = of(
                &reprisekit::schemes::scheme(name).unwrap(),
                Interface::Classic,
            );
            assert_eq!(a.color_scheme, 2, "{name}");
        }
        assert_eq!(
            of(
                &reprisekit::schemes::scheme("High Contrast #1").unwrap(),
                Interface::Classic
            )
            .contrast,
            1
        );
    }

    #[test]
    fn namespace_patterns() {
        assert!(matches(&[], NAMESPACE));
        assert!(matches(&["org.freedesktop.*".into()], NAMESPACE));
        assert!(matches(&[NAMESPACE.into()], NAMESPACE));
        assert!(!matches(&["org.gnome.*".into()], NAMESPACE));
    }

    #[test]
    fn read_all_and_lookup() {
        let s = Settings {
            current: Arc::new(Mutex::new(of(&ColorScheme::default(), Interface::Classic))),
        };
        let all = s.read_all(vec![]);
        assert_eq!(all[NAMESPACE].len(), 3);
        let gnome = s.read_all(vec!["org.gnome.*".into()]);
        assert_eq!(gnome.keys().collect::<Vec<_>>(), [GNOME]);
        assert!(s.lookup(NAMESPACE, "color-scheme").is_ok());
        assert!(s.lookup(NAMESPACE, "font").is_err());
        assert!(s.lookup("org.gnome.desktop", "color-scheme").is_err());
        assert!(s.lookup(GNOME, "color-scheme").is_ok());
    }

    #[test]
    fn the_reprise_namespace_carries_the_look_as_the_environment_does() {
        let colors = reprisekit::schemes::scheme("Tokyo Night").unwrap();
        let s = Settings {
            current: Arc::new(Mutex::new({
                let mut c = crate::config::Config::default();
                c.theme.colors = colors.clone();
                c.interface = Interface::Blue;
                c.blue_variant = Variant::Silver;
                c.icons.theme = "Papirus".into();
                c.theme.fonts.shell_dlg = Some("/usr/share/wine/fonts/sserife.fon".into());
                c.theme.fonts.family = vec!["DejaVu Sans".into(), "Noto Sans".into()];
                Appearance::of(&c)
            })),
        };
        assert_eq!(
            s.read_all(vec![look::NAMESPACE.into()])[look::NAMESPACE].len(),
            10
        );
        let text = |key| String::try_from(s.lookup(look::NAMESPACE, key).unwrap()).unwrap();
        assert_eq!(text(look::INTERFACE), "blue");
        assert_eq!(text(look::BLUE_VARIANT), "silver");
        assert_eq!(text(look::COLORS), colors.to_env());
        assert_eq!(text(look::ICON_THEME), "Papirus");
        assert_eq!(text(look::SHELL_DLG), "/usr/share/wine/fonts/sserife.fon");
        assert_eq!(text(look::FONT_FAMILY), "DejaVu Sans,Noto Sans");
        assert_eq!(text(look::GLASS_MODE), "light");
        assert_eq!(text(look::GLASS_MATERIAL), "frosted");
        assert_eq!(text(look::TEXT_SIZE), "100");
        assert_eq!(text(look::DOUBLE_CLICK_MS), "500");
        assert!(s.lookup(look::NAMESPACE, "color-scheme").is_err());
        let slow = crate::config::Config::parse("[metrics]\ndouble_click_ms = 900\n").unwrap();
        assert_eq!(
            Appearance::of(&slow).get(look::NAMESPACE, look::DOUBLE_CLICK_MS),
            Some(Value::from("900"))
        );
    }

    #[test]
    fn glass_accents_its_glass_colour_and_other_interfaces_their_caption() {
        let mut c = crate::config::Config::default();
        c.glass.color = reprisekit::theme::Color::rgb(0x80, 0x20, 0x40);
        c.interface = Interface::Glass;
        assert_eq!(
            Appearance::of(&c).accent,
            (128.0 / 255.0, 32.0 / 255.0, 64.0 / 255.0)
        );
        c.interface = Interface::Classic;
        let a = c.theme.colors.active_title;
        assert_eq!(
            Appearance::of(&c).accent,
            (a.r as f64 / 255.0, a.g as f64 / 255.0, a.b as f64 / 255.0),
            "Classic keeps its caption colour"
        );
    }

    #[test]
    fn gtk_reads_the_theme_icons_cursor_and_font_light_or_dark() {
        let text = |s: &Settings, key| String::try_from(s.lookup(GNOME, key).unwrap()).unwrap();
        let light = Settings {
            current: Arc::new(Mutex::new(of(&ColorScheme::default(), Interface::Classic))),
        };
        assert_eq!(
            light.read_all(vec![GNOME.into()])[GNOME].len(),
            11,
            "every key GTK takes from here"
        );
        assert_eq!(text(&light, "color-scheme"), "default");
        assert_eq!(text(&light, "gtk-theme"), "Adwaita");
        let dark = Settings {
            current: Arc::new(Mutex::new(of(
                &reprisekit::schemes::scheme("Tokyo Night").unwrap(),
                Interface::Classic,
            ))),
        };
        assert_eq!(text(&dark, "color-scheme"), "prefer-dark");
        assert_eq!(text(&dark, "gtk-theme"), "Adwaita-dark");
        let mut c = crate::config::Config::default();
        c.theme.colors = reprisekit::schemes::scheme("Tokyo Night").unwrap();
        c.toolkits.gtk_theme = "Arc".into();
        assert_eq!(
            Appearance::of(&c).gtk_theme(),
            "Arc",
            "a configured theme stands, dark or not"
        );
        let mut c = crate::config::Config::default();
        c.theme.fonts.family = vec!["DejaVu Sans".into()];
        c.theme.fonts.size_px = 12;
        let a = Appearance::of(&c);
        assert_eq!(a.font_name, "DejaVu Sans 9", "12 px is 9 pt");
        c.theme.fonts.family = vec!["Tahoma".into(), "DejaVu Sans".into()];
        c.theme.fonts.size_px = 11;
        assert_eq!(
            Appearance::of(&c).font_name,
            "Tahoma, DejaVu Sans 8.25",
            "every family, comma-separated, then the size"
        );
        c.theme.fonts.family = vec![];
        assert_eq!(Appearance::of(&c).font_name, "Sans 8.25");
        let large = crate::config::Config::parse("[accessibility]\ntext_size = 150\n").unwrap();
        assert_eq!(large.theme.fonts.size_px, 17);
        let big = Appearance::of(&large);
        assert_eq!(big.font_name, "Tahoma, DejaVu Sans 8.5");
        assert_eq!(big.get(GNOME, "text-scaling-factor"), Some(Value::F64(1.5)));
        assert_eq!(
            big.get(look::NAMESPACE, look::TEXT_SIZE),
            Some(Value::from("150"))
        );
        assert_eq!(
            (a.cursor_theme.clone(), a.cursor_size as u32),
            c.cursor.effective()
        );
        assert!(
            light
                .lookup("org.gnome.desktop.wm.preferences", "theme")
                .is_err(),
            "other GNOME namespaces stay NotFound"
        );
    }

    #[test]
    fn reprise_names_the_generated_slot_and_a_move_signals_it() {
        let mut c = crate::config::Config::default();
        c.toolkits.gtk_theme = "reprise".into();
        assert_eq!(Appearance::of(&c).gtk_theme(), "Adwaita");
        let before = Appearance {
            gtk_generated: Some("Reprise"),
            ..Appearance::of(&c)
        };
        let after = Appearance {
            gtk_generated: Some("Reprise-alt"),
            ..before.clone()
        };
        let s = Settings {
            current: Arc::new(Mutex::new(after.clone())),
        };
        assert_eq!(
            String::try_from(s.lookup(GNOME, "gtk-theme").unwrap()).unwrap(),
            "Reprise-alt"
        );
        let moved: Vec<_> = changed(&before, &after)
            .into_iter()
            .map(|(_, k, _)| k)
            .collect();
        assert_eq!(moved, ["gtk-theme"]);
    }

    #[test]
    fn classics_generated_theme_turns_overlay_scrolling_off() {
        let overlay = |a: &Appearance| a.get(GNOME, "overlay-scrolling").unwrap();
        let mut c = crate::config::Config::default();
        assert_eq!(overlay(&Appearance::of(&c)), Value::Bool(true), "Adwaita");
        c.toolkits.gtk_theme = "reprise".into();
        assert_eq!(
            overlay(&Appearance::of(&c)),
            Value::Bool(true),
            "nothing written yet"
        );
        let classic = Appearance {
            gtk_generated: Some("Reprise"),
            ..Appearance::of(&c)
        };
        assert_eq!(overlay(&classic), Value::Bool(false));
        c.interface = Interface::Blue;
        let blue = Appearance {
            gtk_generated: Some("Reprise-alt"),
            ..Appearance::of(&c)
        };
        assert_eq!(overlay(&blue), Value::Bool(true));
        assert!(
            changed(&classic, &blue)
                .iter()
                .any(|&(_, k, ref v)| k == "overlay-scrolling" && *v == Value::Bool(true))
        );
    }

    #[test]
    fn gtk_reads_the_font_smoothing_and_a_change_signals_it() {
        let read = |a: &Appearance, key| match a.get(GNOME, key).unwrap() {
            Value::Str(s) => s.to_string(),
            v => panic!("{key} is {v:?}"),
        };
        let mut c = crate::config::Config::default();
        let grayscale = Appearance::of(&c);
        assert_eq!(read(&grayscale, "font-antialiasing"), "grayscale");
        assert_eq!(read(&grayscale, "font-hinting"), "slight");
        assert_eq!(read(&grayscale, "font-rgba-order"), "rgb");
        assert_eq!(
            grayscale.get(GNOME, "text-scaling-factor"),
            Some(Value::F64(1.0))
        );
        c.toolkits.font_smoothing = FontSmoothing::Subpixel;
        let subpixel = Appearance::of(&c);
        assert_eq!(read(&subpixel, "font-antialiasing"), "rgba");
        assert_eq!(read(&subpixel, "font-hinting"), "slight");
        c.toolkits.font_smoothing = FontSmoothing::None;
        let none = Appearance::of(&c);
        assert_eq!(read(&none, "font-antialiasing"), "none");
        assert_eq!(
            read(&none, "font-hinting"),
            "full",
            "monochrome glyphs are fully hinted"
        );
        assert_eq!(
            changed(&grayscale, &none),
            [
                (GNOME, "font-antialiasing", Value::from("none")),
                (GNOME, "font-hinting", Value::from("full")),
            ]
        );
    }

    #[test]
    fn dark_glass_reads_dark_everywhere() {
        let c = crate::config::Config::parse(
            "[appearance]\ninterface = \"glass\"\n[glass]\nmode = \"dark\"\n",
        )
        .unwrap();
        let a = Appearance::of(&c);
        assert_eq!(a.color_scheme, 1, "dark Glass is dark to the portal");
        assert!(a.dark());
        assert_eq!(a.gtk_theme(), "Adwaita-dark");
        let s = Settings {
            current: Arc::new(Mutex::new(a)),
        };
        assert_eq!(
            String::try_from(s.lookup(look::NAMESPACE, look::GLASS_MODE).unwrap()).unwrap(),
            "dark"
        );
    }

    #[test]
    fn a_light_to_dark_glass_switch_signals_the_keys_that_moved() {
        let glass = |mode| {
            let c = crate::config::Config::parse(&format!(
                "[appearance]\ninterface = \"glass\"\n[glass]\nmode = \"{mode}\"\n"
            ))
            .unwrap();
            Appearance::of(&c)
        };
        let (light, dark) = (glass("light"), glass("dark"));
        assert!(!light.dark() && dark.dark());
        assert_eq!(
            changed(&light, &dark),
            [
                (NAMESPACE, "color-scheme", Value::U32(1)),
                (
                    look::NAMESPACE,
                    look::COLORS,
                    Value::from(dark.colors.clone())
                ),
                (look::NAMESPACE, look::GLASS_MODE, Value::from("dark")),
                (GNOME, "color-scheme", Value::from("prefer-dark")),
                (GNOME, "gtk-theme", Value::from("Adwaita-dark")),
            ],
            "the scheme, the colours and the mode move; the accent, icons, cursor and font stay"
        );
        assert!(
            changed(&dark, &dark).is_empty(),
            "nothing moved, nothing signalled"
        );
    }
}
