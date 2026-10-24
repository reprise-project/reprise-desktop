use reprisekit::interface::Interface;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconsConfig {
    pub theme: String,
}

impl Default for IconsConfig {
    fn default() -> Self {
        IconsConfig {
            theme: "hicolor".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolkitsConfig {
    pub gtk_theme: String,
    pub qt_platform_theme: String,
    pub font_smoothing: FontSmoothing,
    pub gtk4_colors: bool,
}

pub const TOOLKIT_AUTO: &str = "auto";

impl Default for ToolkitsConfig {
    fn default() -> Self {
        ToolkitsConfig {
            gtk_theme: TOOLKIT_AUTO.into(),
            qt_platform_theme: TOOLKIT_AUTO.into(),
            font_smoothing: FontSmoothing::Grayscale,
            gtk4_colors: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FontSmoothing {
    #[default]
    Grayscale,
    Subpixel,
    None,
}

impl FontSmoothing {
    pub fn parse(s: &str) -> Option<FontSmoothing> {
        match s {
            "grayscale" => Some(FontSmoothing::Grayscale),
            "subpixel" => Some(FontSmoothing::Subpixel),
            "none" => Some(FontSmoothing::None),
            _ => None,
        }
    }

    pub fn gnome_keys(self) -> (&'static str, &'static str, &'static str) {
        match self {
            FontSmoothing::Grayscale => ("grayscale", "slight", "rgb"),
            FontSmoothing::Subpixel => ("rgba", "slight", "rgb"),
            FontSmoothing::None => ("none", "full", "rgb"),
        }
    }
}

pub const TOOLKIT_REPRISE: &str = "reprise";

/// The GTK theme for `configured`: "reprise" is the `generated` slot, "auto" (or
/// "reprise" with nothing written) is Adwaita or its dark variant; else the named theme.
pub fn resolve_gtk_theme<'a>(
    configured: &'a str,
    dark: bool,
    generated: Option<&'static str>,
) -> &'a str {
    match (configured, generated) {
        (TOOLKIT_REPRISE, Some(g)) => g,
        (TOOLKIT_AUTO | TOOLKIT_REPRISE, _) if dark => "Adwaita-dark",
        (TOOLKIT_AUTO | TOOLKIT_REPRISE, _) => "Adwaita",
        (t, _) => t,
    }
}

/// `QT_QPA_PLATFORMTHEME` for `configured`: "reprise" is the `generated` tool,
/// "auto" (or "reprise" with no tool) is the portal's under a dark look and None
/// under a light one; else the named platform theme.
pub fn resolve_qt<'a>(
    configured: &'a str,
    dark: bool,
    generated: Option<&'static str>,
) -> Option<&'a str> {
    match (configured, generated) {
        (TOOLKIT_REPRISE, Some(g)) => Some(g),
        (TOOLKIT_AUTO | TOOLKIT_REPRISE, _) if dark => Some("xdgdesktopportal"),
        (TOOLKIT_AUTO | TOOLKIT_REPRISE, _) => None,
        (t, _) => Some(t),
    }
}

/// `[cursor]`: the XCursor theme and size. An empty theme or size 0 keeps the
/// session's `XCURSOR_THEME` / `XCURSOR_SIZE`, else `default` at 24 px.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CursorConfig {
    pub theme: String,
    pub size: u32,
}

pub const CURSOR_SIZE_MAX: u32 = 256;

impl CursorConfig {
    pub fn resolve(&self, env: impl Fn(&str) -> Option<String>) -> (String, u32) {
        let theme = Some(self.theme.clone())
            .filter(|t| !t.is_empty())
            .or_else(|| env("XCURSOR_THEME").filter(|t| !t.is_empty()))
            .unwrap_or_else(|| "default".into());
        let size = Some(self.size)
            .filter(|s| *s > 0)
            .or_else(|| {
                env("XCURSOR_SIZE")
                    .and_then(|s| s.trim().parse().ok())
                    .filter(|s| (1..=CURSOR_SIZE_MAX).contains(s))
            })
            .unwrap_or(24);
        (theme, size)
    }

    pub fn effective(&self) -> (String, u32) {
        self.resolve(|k| std::env::var(k).ok())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskbarButtons {
    Combine,
    WhenFull,
    Never,
    Group,
}

impl TaskbarButtons {
    pub fn parse(s: &str) -> Option<TaskbarButtons> {
        match s {
            "combine" => Some(TaskbarButtons::Combine),
            "when_full" => Some(TaskbarButtons::WhenFull),
            "never" => Some(TaskbarButtons::Never),
            "group" => Some(TaskbarButtons::Group),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartMenuStyle {
    Classic,
    Glass,
}

impl StartMenuStyle {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "classic" => Some(Self::Classic),
            "glass" => Some(Self::Glass),
            _ => None,
        }
    }
    pub fn default_for(i: Interface) -> Self {
        if i == Interface::Glass {
            Self::Glass
        } else {
            Self::Classic
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TaskbarConfig {
    pub position: reprisekit::taskbar::Edge,
    pub buttons: TaskbarButtons,
    pub pinned: bool,
    pub small_icons: bool,
    pub show_desktop_button: bool,
    pub peek: bool,
    pub tray_overflow: bool,
    pub auto_hide: bool,
    pub reveal_on_edge: bool,
    pub reveal_on_keys: bool,
    pub reveal_on_attention: bool,
    /// 0 takes the look's own; clamped where used (`barplace::thickness`).
    pub side_width: i32,
}

impl TaskbarConfig {
    /// `interface`'s taskbar defaults, kept in sync with `Settings::default` and
    /// `Settings::for_interface` in reprise-settings.
    pub fn default_for(interface: Interface) -> TaskbarConfig {
        let glass = interface == Interface::Glass;
        TaskbarConfig {
            position: reprisekit::taskbar::Edge::Bottom,
            buttons: if glass {
                TaskbarButtons::Combine
            } else {
                TaskbarButtons::WhenFull
            },
            pinned: glass,
            small_icons: false,
            show_desktop_button: glass,
            peek: glass,
            tray_overflow: true,
            auto_hide: false,
            reveal_on_edge: true,
            reveal_on_keys: true,
            reveal_on_attention: true,
            side_width: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WallpaperConfig {
    /// A picture file, or a built-in's `reprise:` URI (`builtin_path`).
    pub image: Option<PathBuf>,
    pub position: crate::wallpaper::Position,
    pub lock_screen: bool,
    pub slideshow: Option<PathBuf>,
    pub interval: u32,
    pub shuffle: bool,
    pub water: reprisekit::water::Water,
    pub water_rate: reprisekit::water::Rate,
}

impl Default for WallpaperConfig {
    fn default() -> WallpaperConfig {
        WallpaperConfig {
            image: None,
            position: Default::default(),
            lock_screen: false,
            slideshow: None,
            interval: 30,
            shuffle: false,
            water: Default::default(),
            water_rate: Default::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScreensaverConfig {
    pub kind: reprisekit::saver::Kind,
    pub options: reprisekit::saver::Options,
    pub folder: Option<PathBuf>,
}
