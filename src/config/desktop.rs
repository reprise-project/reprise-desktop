use reprisekit::interface::Interface;
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq)]
pub struct SnapConfig {
    pub enabled: bool,
    pub picker: bool,
    pub bar: bool,
    pub layouts: Vec<crate::snap::ZoneLayout>,
    /// Logical px from the usable area's edge where a drag snaps; corners reach twice as far.
    pub distance: i32,
}

pub const SNAP_DISTANCE_MAX: i32 = 100;

impl Default for SnapConfig {
    fn default() -> Self {
        SnapConfig {
            enabled: true,
            picker: true,
            bar: true,
            layouts: Vec::new(),
            distance: crate::snap::BAND,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenshotConfig {
    pub save: bool,
}

impl Default for ScreenshotConfig {
    fn default() -> Self {
        ScreenshotConfig { save: true }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebugConfig {
    pub overlay: bool,
    pub overlay_in_captures: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewsConfig {
    pub taskbar: bool,
    pub fps: u32,
}

/// Ceiling of `[previews] fps`: a common output's refresh.
pub const PREVIEW_FPS_MAX: u32 = 60;

impl Default for PreviewsConfig {
    fn default() -> Self {
        PreviewsConfig {
            taskbar: true,
            fps: 10,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HotCorner {
    #[default]
    None,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl HotCorner {
    pub const ALL: [HotCorner; 5] = [
        HotCorner::None,
        HotCorner::TopLeft,
        HotCorner::TopRight,
        HotCorner::BottomLeft,
        HotCorner::BottomRight,
    ];

    pub fn name(self) -> &'static str {
        match self {
            HotCorner::None => "none",
            HotCorner::TopLeft => "top-left",
            HotCorner::TopRight => "top-right",
            HotCorner::BottomLeft => "bottom-left",
            HotCorner::BottomRight => "bottom-right",
        }
    }

    pub fn from_name(s: &str) -> Option<HotCorner> {
        HotCorner::ALL.into_iter().find(|c| c.name() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GesturesConfig {
    pub enabled: bool,
    pub fingers: u32,
    /// libinput swipe units for the whole way.
    pub distance: u32,
    /// Percent of the way past which a released swipe commits.
    pub commit_at: u32,
    /// Swipe units per second that make a release a flick.
    pub flick_speed: u32,
}

impl Default for GesturesConfig {
    fn default() -> Self {
        GesturesConfig {
            enabled: true,
            fingers: 3,
            distance: 400,
            commit_at: 30,
            flick_speed: 300,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SwitcherStyle {
    #[default]
    Icons,
    Thumbnails,
}

impl FromStr for SwitcherStyle {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "icons" => Ok(SwitcherStyle::Icons),
            "thumbnails" => Ok(SwitcherStyle::Thumbnails),
            other => Err(format!(
                "switcher.style {other:?} is not \"icons\" or \"thumbnails\""
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwitcherConfig {
    pub style: SwitcherStyle,
    pub preview: bool,
    pub desktop: bool,
    pub peek: bool,
    pub flip: bool,
    pub size: reprisekit::switcher::Size,
}

impl SwitcherConfig {
    /// `interface`'s switcher defaults, kept in sync with `Settings::for_interface` in reprise-settings.
    pub fn default_for(interface: Interface) -> SwitcherConfig {
        let glass = interface == Interface::Glass;
        SwitcherConfig {
            style: if glass {
                SwitcherStyle::Thumbnails
            } else {
                SwitcherStyle::Icons
            },
            preview: true,
            desktop: glass,
            peek: glass,
            flip: glass,
            size: reprisekit::switcher::Size::Small,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionConfig {
    pub restore: bool,
    pub exclude: Vec<String>,
    /// Log Off ends every app scope Reprise started and every process left in the login session,
    /// windowless ones too.
    pub end_processes: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            restore: false,
            exclude: Vec::new(),
            end_processes: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardConfig {
    pub history: bool,
    pub terminal_apps: Vec<String>,
    /// Sandboxed (Flatpak) app ids that may use the clipboard managers'
    /// data control protocols, which read the clipboard at any time.
    pub trusted_apps: Vec<String>,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        ClipboardConfig {
            history: true,
            terminal_apps: [
                "reprise.terminal",
                "foot",
                "kitty",
                "Alacritty",
                "org.wezfurlong.wezterm",
                "com.mitchellh.ghostty",
                "org.gnome.Console",
                "konsole",
                "xterm",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            trusted_apps: Vec::new(),
        }
    }
}

pub const WORKSPACES_MAX: usize = 16;
