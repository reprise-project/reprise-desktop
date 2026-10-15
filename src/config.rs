//! `config.toml`: parsing, validation and defaults for every section.

use crate::keys::{MOD_ALT, MOD_CTRL, MOD_LOGO, MOD_SHIFT};
use crate::theme::{Color, FontConfig, Theme};
use crate::windowrules::{WindowRule, WindowState};
use reprisekit::interface::Interface;
use serde::Deserialize;
use smithay::input::keyboard::XkbConfig;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Spawn(String),
    /// An argument vector run with no shell (a Quick Launch application entry, `reprisefs::apps::launch_argv`),
    /// and the desktop entry id it came from.
    Exec {
        argv: Vec<std::ffi::OsString>,
        app: String,
    },
    Terminal,
    Close,
    Cycle,
    Flip,
    Exit,
    Maximize,
    Minimize,
    Lock,
    StartMenu,
    Run,
    LogOff,
    TurnOff,
    WindowMenu,
    ShowDesktop,
    /// Hide or show the focused window's border and caption; it keeps its outer rect.
    ToggleFrame,
    AlwaysOnTop,
    Center,
    Tab(bool),
    Desktop(crate::desktops::Step),
    CarryToDesktop(crate::desktops::Step),
    AllDesktops,
    DoNotDisturb,
    NightLight,
    Snap(crate::snap::Dir),
    SnapLayouts,
    Zoom(crate::magnifier::ZoomStep),
    ColorFilter,
    Launcher,
    Overview,
    FocusTaskbar,
    FocusTray,
    ClipboardHistory,
    EmojiPicker,
    NextLayout,
    Screenshot,
    ScreenshotWindow,
    ScreenshotRegion,
    ScreenshotContent,
    VolumeUp,
    VolumeDown,
    Mute,
    MicMute,
    BrightnessUp,
    BrightnessDown,
    KbdBrightnessUp,
    KbdBrightnessDown,
    PlayPause,
    Next,
    Prev,
    Stop,
    Wifi,
}

impl Action {
    /// The config name of a compositor action (the inverse of `from_parts`);
    /// None for `Spawn` and `Exec`, which are commands, not names.
    pub fn name(&self) -> Option<&'static str> {
        Some(match self {
            Action::Spawn(_) | Action::Exec { .. } => return None,
            Action::Terminal => "terminal",
            Action::Close => "close",
            Action::Cycle => "cycle",
            Action::Flip => "flip",
            Action::Exit => "exit",
            Action::Maximize => "maximize",
            Action::Minimize => "minimize",
            Action::Lock => "lock",
            Action::StartMenu => "startmenu",
            Action::Run => "run",
            Action::LogOff => "logoff",
            Action::TurnOff => "turnoff",
            Action::WindowMenu => "windowmenu",
            Action::ShowDesktop => "showdesktop",
            Action::ToggleFrame => "toggleframe",
            Action::AlwaysOnTop => "alwaysontop",
            Action::Center => "center",
            Action::Tab(false) => "tab-next",
            Action::Tab(true) => "tab-prev",
            Action::Desktop(crate::desktops::Step::Prev) => "desktop-prev",
            Action::Desktop(crate::desktops::Step::Next) => "desktop-next",
            Action::CarryToDesktop(crate::desktops::Step::Prev) => "desktop-carry-prev",
            Action::CarryToDesktop(crate::desktops::Step::Next) => "desktop-carry-next",
            Action::AllDesktops => "all-desktops",
            Action::DoNotDisturb => "do-not-disturb",
            Action::NightLight => "night-light",
            Action::Snap(crate::snap::Dir::Left) => "snap-left",
            Action::Snap(crate::snap::Dir::Right) => "snap-right",
            Action::Snap(crate::snap::Dir::Up) => "snap-up",
            Action::Snap(crate::snap::Dir::Down) => "snap-down",
            Action::SnapLayouts => "snap-layouts",
            Action::Zoom(crate::magnifier::ZoomStep::In) => "zoom-in",
            Action::Zoom(crate::magnifier::ZoomStep::Out) => "zoom-out",
            Action::Zoom(crate::magnifier::ZoomStep::Reset) => "zoom-reset",
            Action::ColorFilter => "color-filter",
            Action::Launcher => "launcher",
            Action::Overview => "overview",
            Action::FocusTaskbar => "focus-taskbar",
            Action::FocusTray => "focus-tray",
            Action::ClipboardHistory => "clipboard-history",
            Action::EmojiPicker => "emoji-picker",
            Action::NextLayout => "next-layout",
            Action::Screenshot => "screenshot",
            Action::ScreenshotWindow => "screenshot-window",
            Action::ScreenshotRegion => "screenshot-region",
            Action::ScreenshotContent => "screenshot-content",
            Action::VolumeUp => "volume-up",
            Action::VolumeDown => "volume-down",
            Action::Mute => "mute",
            Action::MicMute => "mic-mute",
            Action::BrightnessUp => "brightness-up",
            Action::BrightnessDown => "brightness-down",
            Action::KbdBrightnessUp => "kbd-brightness-up",
            Action::KbdBrightnessDown => "kbd-brightness-down",
            Action::PlayPause => "play-pause",
            Action::Next => "next",
            Action::Prev => "prev",
            Action::Stop => "stop",
            Action::Wifi => "wifi",
        })
    }

    /// What a launcher file for this action holds: the command of a
    /// `Spawn`, else the action's name. None for an `Exec`, which only a
    /// launcher file already holds.
    pub fn launch(&self) -> Option<reprisefs::apps::Launch<'_>> {
        Some(match self {
            Action::Spawn(cmd) => reprisefs::apps::Launch::Exec(cmd),
            Action::Exec { .. } => return None,
            a => reprisefs::apps::Launch::Action(a.name().unwrap_or_default()),
        })
    }

    pub fn from_parts(action: &str, command: Option<&str>) -> Result<Action, String> {
        Ok(match action {
            "spawn" => Action::Spawn(command.ok_or("spawn needs a command")?.to_string()),
            "terminal" => Action::Terminal,
            "close" => Action::Close,
            "cycle" => Action::Cycle,
            "flip" => Action::Flip,
            "exit" => Action::Exit,
            "maximize" => Action::Maximize,
            "minimize" => Action::Minimize,
            "lock" => Action::Lock,
            "startmenu" => Action::StartMenu,
            "run" => Action::Run,
            "logoff" => Action::LogOff,
            "turnoff" => Action::TurnOff,
            "windowmenu" => Action::WindowMenu,
            "showdesktop" => Action::ShowDesktop,
            "toggleframe" => Action::ToggleFrame,
            "alwaysontop" => Action::AlwaysOnTop,
            "center" => Action::Center,
            "tab-next" => Action::Tab(false),
            "tab-prev" => Action::Tab(true),
            "desktop-prev" => Action::Desktop(crate::desktops::Step::Prev),
            "desktop-next" => Action::Desktop(crate::desktops::Step::Next),
            "desktop-carry-prev" => Action::CarryToDesktop(crate::desktops::Step::Prev),
            "desktop-carry-next" => Action::CarryToDesktop(crate::desktops::Step::Next),
            "all-desktops" => Action::AllDesktops,
            "do-not-disturb" => Action::DoNotDisturb,
            "night-light" => Action::NightLight,
            "snap-left" => Action::Snap(crate::snap::Dir::Left),
            "snap-right" => Action::Snap(crate::snap::Dir::Right),
            "snap-up" => Action::Snap(crate::snap::Dir::Up),
            "snap-down" => Action::Snap(crate::snap::Dir::Down),
            "snap-layouts" => Action::SnapLayouts,
            "zoom-in" => Action::Zoom(crate::magnifier::ZoomStep::In),
            "zoom-out" => Action::Zoom(crate::magnifier::ZoomStep::Out),
            "zoom-reset" => Action::Zoom(crate::magnifier::ZoomStep::Reset),
            "color-filter" => Action::ColorFilter,
            "launcher" => Action::Launcher,
            "overview" => Action::Overview,
            "focus-taskbar" => Action::FocusTaskbar,
            "focus-tray" => Action::FocusTray,
            "clipboard-history" => Action::ClipboardHistory,
            "emoji-picker" => Action::EmojiPicker,
            "next-layout" => Action::NextLayout,
            "screenshot" => Action::Screenshot,
            "screenshot-window" => Action::ScreenshotWindow,
            "screenshot-region" => Action::ScreenshotRegion,
            "screenshot-content" => Action::ScreenshotContent,
            "volume-up" => Action::VolumeUp,
            "volume-down" => Action::VolumeDown,
            "mute" => Action::Mute,
            "mic-mute" => Action::MicMute,
            "brightness-up" => Action::BrightnessUp,
            "brightness-down" => Action::BrightnessDown,
            "kbd-brightness-up" => Action::KbdBrightnessUp,
            "kbd-brightness-down" => Action::KbdBrightnessDown,
            "play-pause" => Action::PlayPause,
            "next" => Action::Next,
            "prev" => Action::Prev,
            "stop" => Action::Stop,
            "wifi" => Action::Wifi,
            other => return Err(format!("unknown action {other:?}")),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keybind {
    pub mods: u32,
    pub keysym: u32,
    pub action: Action,
}

/// One `[[global_shortcuts]]` entry bound through the portal (reprise-portal
/// writes these; Settings edits them). `chord` is None when `keys` is empty,
/// unparsable or modifier-only; the entry stays and takes no key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalShortcut {
    pub app: String,
    pub id: String,
    pub description: String,
    pub keys: String,
    pub chord: Option<(u32, u32)>,
}

impl Keybind {
    /// A modifier-only chord ("Shift+Alt_L") fires on release via `keyroute::ChordState`.
    pub fn is_chord(&self) -> bool {
        crate::keyroute::modifier_key(self.keysym).is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickLaunchEntry {
    pub label: String,
    pub icon: Option<String>,
    pub action: Action,
}

fn default_quicklaunch() -> Vec<QuickLaunchEntry> {
    let entry = |label: &str, icon: &str, action: Action| QuickLaunchEntry {
        label: label.into(),
        icon: Some(icon.into()),
        action,
    };
    vec![
        entry(
            "Launch Web Browser",
            "web-browser",
            Action::Spawn("xdg-open https://duckduckgo.com/".into()),
        ),
        entry("Show Desktop", "user-desktop", Action::ShowDesktop),
        entry("Terminal", "utilities-terminal", Action::Terminal),
    ]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuTarget {
    Action(Action),
    Programs,
    Settings,
    Documents,
    Submenu(Vec<MenuEntry>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEntry {
    Separator,
    Item {
        label: String,
        icon: Option<String>,
        target: MenuTarget,
        tip: Option<String>,
    },
}

impl MenuEntry {
    pub(crate) fn item(label: &str, icon: &str, target: MenuTarget) -> MenuEntry {
        MenuEntry::Item {
            label: label.into(),
            icon: Some(icon.into()),
            target,
            tip: None,
        }
    }

    pub(crate) fn spawn(label: &str, icon: &str, cmd: &str) -> MenuEntry {
        MenuEntry::item(label, icon, MenuTarget::Action(Action::Spawn(cmd.into())))
    }

    pub(crate) fn tipped(self, tip: &str) -> MenuEntry {
        match self {
            MenuEntry::Item {
                label,
                icon,
                target,
                ..
            } => MenuEntry::Item {
                label,
                icon,
                target,
                tip: Some(tip.into()),
            },
            sep => sep,
        }
    }

    pub fn tip(&self) -> Option<&str> {
        match self {
            MenuEntry::Item { tip, .. } => tip.as_deref(),
            MenuEntry::Separator => None,
        }
    }

    pub fn opens_submenu(&self) -> bool {
        !matches!(
            self,
            MenuEntry::Separator
                | MenuEntry::Item {
                    target: MenuTarget::Action(_),
                    ..
                }
        )
    }
}

pub(crate) const PRINTERS: &str = "reprise-printers";
pub(crate) const PRINTERS_TIP: &str =
    "Add, remove, and configure local and network printers, and see what is printing.";
pub(crate) const HELP: &str = "xdg-open https://github.com/reprise-project/reprise-desktop";
pub(crate) const CONTROL_PANEL_TIP: &str = "Provides options for you to customize the appearance \
     and functionality of your computer, add or remove programs, and set up network connections \
     and user accounts.";

/// The default Start menu. Kept in sync with `default_menu` in
/// `crates/apps/settings/src/startmenu.rs`; both are held to `config.example.toml`'s block.
fn default_startmenu() -> Vec<MenuEntry> {
    use MenuTarget as T;
    vec![
        MenuEntry::item("&Programs", "applications-other", T::Programs),
        MenuEntry::item("&Documents", "folder-documents", T::Documents),
        MenuEntry::item(
            "&Settings",
            "preferences-system",
            T::Submenu(vec![
                MenuEntry::item("&Control Panel", "preferences-other", T::Settings)
                    .tipped(CONTROL_PANEL_TIP),
                MenuEntry::Separator,
                MenuEntry::spawn(
                    "&Network Connections",
                    "network-workgroup",
                    "nm-connection-editor",
                )
                .tipped(
                    "Displays existing network connections on this computer and helps you \
                     create new ones",
                ),
                MenuEntry::spawn("&Printers", "printer", PRINTERS).tipped(PRINTERS_TIP),
                MenuEntry::spawn(
                    "&Taskbar and Start Menu",
                    "preferences-desktop",
                    "reprise-settings --page taskbar",
                )
                .tipped(
                    "Customize the Start Menu and the taskbar, such as the types of items to \
                     be displayed and how they should appear.",
                ),
            ]),
        ),
        MenuEntry::item(
            "Sear&ch",
            "system-search",
            T::Submenu(vec![
                MenuEntry::spawn(
                    "&For Files or Folders...",
                    "system-search",
                    "reprise-explorer \"$HOME\"",
                ),
                MenuEntry::spawn(
                    "On the &Internet...",
                    "web-browser",
                    "xdg-open https://duckduckgo.com/",
                ),
            ]),
        ),
        MenuEntry::spawn("&Help and Support", "help-browser", HELP),
        MenuEntry::item("&Run...", "system-run", T::Action(Action::Run)),
        MenuEntry::Separator,
        MenuEntry::item(
            "&Log Off {user}...",
            "gnome-session-logout",
            T::Action(Action::LogOff),
        ),
        MenuEntry::item(
            "T&urn Off Computer...",
            "system-shutdown",
            T::Action(Action::TurnOff),
        ),
    ]
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutputConfig {
    pub name: String,
    pub scale: f32,
    pub mode: Option<OutputMode>,
    pub adaptive_sync: Option<bool>,
    pub transform: Transform,
    pub position: Option<(i32, i32)>,
    pub primary: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SnapConfig {
    pub enabled: bool,
    pub picker: bool,
    pub bar: bool,
    pub layouts: Vec<crate::snap::ZoneLayout>,
}

impl Default for SnapConfig {
    fn default() -> Self {
        SnapConfig {
            enabled: true,
            picker: true,
            bar: true,
            layouts: Vec::new(),
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

#[derive(Clone, Debug, PartialEq)]
pub struct NightLightConfig {
    pub enabled: bool,
    pub temperature: u32,
    pub schedule: NightSchedule,
    /// Minutes after local midnight.
    pub from: u32,
    pub to: u32,
    /// (latitude, longitude): degrees north and east.
    pub location: Option<(f64, f64)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SwitchStyle {
    #[default]
    None,
    Slide,
    SlideVertical,
    Fade,
    Zoom,
    Wipe,
}

impl SwitchStyle {
    pub const ALL: [SwitchStyle; 6] = [
        SwitchStyle::None,
        SwitchStyle::Slide,
        SwitchStyle::SlideVertical,
        SwitchStyle::Fade,
        SwitchStyle::Zoom,
        SwitchStyle::Wipe,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SwitchStyle::None => "none",
            SwitchStyle::Slide => "slide",
            SwitchStyle::SlideVertical => "slide-vertical",
            SwitchStyle::Fade => "fade",
            SwitchStyle::Zoom => "zoom",
            SwitchStyle::Wipe => "wipe",
        }
    }

    pub fn from_name(s: &str) -> Option<SwitchStyle> {
        SwitchStyle::ALL.into_iter().find(|v| v.name() == s)
    }
}

pub const SWITCH_MS_MAX: u32 = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwitchAnimation {
    pub style: SwitchStyle,
    pub ms: u32,
}

impl Default for SwitchAnimation {
    fn default() -> Self {
        SwitchAnimation {
            style: SwitchStyle::default(),
            ms: 200,
        }
    }
}

impl SwitchAnimation {
    pub fn enabled(&self) -> bool {
        self.style != SwitchStyle::None && self.ms > 0
    }
}

pub const EFFECTS_MS_MAX: u32 = 2000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectsConfig {
    pub window_animation: bool,
    pub window_open_ms: u32,
    pub window_close_ms: u32,
    pub window_scale: f64,
    pub hover_fade: bool,
    pub hover_in_ms: u32,
    pub hover_out_ms: u32,
    pub taskbar_slide: bool,
    pub taskbar_hide_ms: u32,
}

impl EffectsConfig {
    pub fn default_for(interface: Interface) -> EffectsConfig {
        let glass = interface == Interface::Glass;
        EffectsConfig {
            window_animation: glass,
            window_open_ms: 250,
            window_close_ms: 200,
            window_scale: 0.95,
            hover_fade: glass,
            hover_in_ms: 120,
            hover_out_ms: 250,
            taskbar_slide: glass,
            taskbar_hide_ms: 200,
        }
    }

    pub fn opens(&self) -> bool {
        self.window_animation && self.window_open_ms > 0
    }

    pub fn closes(&self) -> bool {
        self.window_animation && self.window_close_ms > 0
    }

    /// (in, out): both 0, instant, while `hover_fade` is off.
    pub fn hover_ms(&self) -> (u32, u32) {
        if self.hover_fade {
            (self.hover_in_ms, self.hover_out_ms)
        } else {
            (0, 0)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NightSchedule {
    Sun,
    Fixed,
}

impl Default for NightLightConfig {
    fn default() -> Self {
        NightLightConfig {
            enabled: false,
            temperature: 4000,
            schedule: NightSchedule::Sun,
            from: 21 * 60,
            to: 7 * 60,
            location: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccessibilityConfig {
    pub color_filter: reprisekit::colorfilter::Filter,
    pub color_filter_intensity: u8,
    pub magnifier_follow: crate::magnifier::Follow,
    pub sticky_keys: bool,
    pub sticky_lock: bool,
    pub sticky_two_keys_off: bool,
    pub slow_keys: bool,
    /// Milliseconds; the file gives seconds.
    pub slow_keys_delay: u32,
    pub shortcuts: bool,
    pub mouse_keys: bool,
    pub mouse_keys_speed: u32,
    /// Milliseconds; the file gives seconds.
    pub mouse_keys_acceleration: u32,
    pub mouse_keys_num_lock: bool,
    pub text_size: u32,
}

impl Default for AccessibilityConfig {
    fn default() -> Self {
        AccessibilityConfig {
            color_filter: reprisekit::colorfilter::Filter::None,
            color_filter_intensity: 100,
            magnifier_follow: crate::magnifier::Follow::default(),
            sticky_keys: false,
            sticky_lock: true,
            sticky_two_keys_off: true,
            slow_keys: false,
            slow_keys_delay: 1000,
            shortcuts: true,
            mouse_keys: false,
            mouse_keys_speed: 400,
            mouse_keys_acceleration: 1000,
            mouse_keys_num_lock: true,
            text_size: 100,
        }
    }
}

impl AccessibilityConfig {
    pub fn text_factor(&self) -> f64 {
        self.text_size as f64 / 100.0
    }
}

pub const SLOW_KEYS_DELAY: std::ops::RangeInclusive<f64> = 0.3..=2.0;

pub const MOUSE_KEYS_SPEED: std::ops::RangeInclusive<u32> = 50..=2000;

pub const MOUSE_KEYS_ACCELERATION: std::ops::RangeInclusive<f64> = 0.0..=5.0;

pub const TEXT_SIZE: std::ops::RangeInclusive<u32> = 100..=200;
pub const TEXT_SIZE_STEP: u32 = 25;

pub const NIGHT_KELVIN: std::ops::RangeInclusive<u32> = 1000..=6500;

pub fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    let digits = |t: &str| t.bytes().all(|b| b.is_ascii_digit());
    if !(1..=2).contains(&h.len()) || m.len() != 2 || !digits(h) || !digits(m) {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
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

/// An output transform by its wlr-randr name (`transform = "90"`); the
/// discriminants are the `wl_output_transform` values, rotations counter-clockwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum Transform {
    #[default]
    Normal = 0,
    Rotate90 = 1,
    Rotate180 = 2,
    Rotate270 = 3,
    Flipped = 4,
    Flipped90 = 5,
    Flipped180 = 6,
    Flipped270 = 7,
}

impl Transform {
    pub const ALL: [Transform; 8] = [
        Transform::Normal,
        Transform::Rotate90,
        Transform::Rotate180,
        Transform::Rotate270,
        Transform::Flipped,
        Transform::Flipped90,
        Transform::Flipped180,
        Transform::Flipped270,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Transform::Normal => "normal",
            Transform::Rotate90 => "90",
            Transform::Rotate180 => "180",
            Transform::Rotate270 => "270",
            Transform::Flipped => "flipped",
            Transform::Flipped90 => "flipped-90",
            Transform::Flipped180 => "flipped-180",
            Transform::Flipped270 => "flipped-270",
        }
    }

    pub fn parse(s: &str) -> Option<Transform> {
        Transform::ALL.into_iter().find(|t| t.name() == s)
    }
}

/// A mode request from `mode = "1920x1080"` or `"1920x1080@59.94"`; the refresh
/// (above 0, at most 1000 Hz) is in mHz like Smithay's `output::Mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputMode {
    pub width: i32,
    pub height: i32,
    pub refresh_mhz: Option<i32>,
}

impl OutputMode {
    pub fn parse(s: &str) -> Result<OutputMode, String> {
        let bad = || format!("mode {s:?} is not WIDTHxHEIGHT[@HZ]");
        let (size, hz) = match s.split_once('@') {
            Some((size, hz)) => (size, Some(hz)),
            None => (s, None),
        };
        let (w, h) = size.split_once('x').ok_or_else(bad)?;
        let width: i32 = w.trim().parse().map_err(|_| bad())?;
        let height: i32 = h.trim().parse().map_err(|_| bad())?;
        if width <= 0 || height <= 0 {
            return Err(bad());
        }
        let refresh_mhz = match hz {
            Some(hz) => {
                let hz: f64 = hz.trim().parse().map_err(|_| bad())?;
                // 1000 Hz bounds the mHz value well inside i32.
                if !hz.is_finite() || hz <= 0.0 || hz > 1000.0 {
                    return Err(bad());
                }
                Some((hz * 1000.0).round() as i32)
            }
            None => None,
        };
        Ok(OutputMode {
            width,
            height,
            refresh_mhz,
        })
    }

    /// Index of the best match among `modes` (in the output's order): same
    /// size, then the closest refresh when one was asked for, else the
    /// preferred one, else the highest refresh. None when no mode has the
    /// size.
    pub fn pick(&self, modes: &[OfferedMode]) -> Option<usize> {
        let same_size = modes
            .iter()
            .enumerate()
            .filter(|(_, m)| m.width == self.width && m.height == self.height);
        match self.refresh_mhz {
            Some(want) => same_size.min_by_key(|(_, m)| (m.refresh_mhz - want).abs()),
            None => same_size.max_by_key(|(_, m)| (m.preferred, m.refresh_mhz)),
        }
        .map(|(i, _)| i)
    }
}

impl std::fmt::Display for OutputMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.width, self.height)?;
        match self.refresh_mhz {
            Some(mhz) => write!(f, "@{}", hz_string(mhz)),
            None => Ok(()),
        }
    }
}

pub fn hz_string(refresh_mhz: i32) -> String {
    let hz = format!("{:.3}", refresh_mhz as f64 / 1000.0);
    hz.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OfferedMode {
    pub width: i32,
    pub height: i32,
    pub refresh_mhz: i32,
    pub preferred: bool,
}

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

/// The compositor's whole configuration.
///
/// **The defaults are duplicated in `reprise-settings`** (`crates/apps/settings/src/model.rs`),
/// which cannot depend on this crate: change a default in both in the same commit.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub terminal: String,
    pub script_terminal: reprisefs::program::ScriptEnd,
    pub task_manager: Option<String>,
    /// Retired keys (`renderer`, `outputs.<name>.hdr`, `.color_profile`), kept so
    /// an older file loads; [`Config::warn_retired`] names them.
    pub retired: Vec<String>,
    /// Includes `[accessibility] text_size`.
    pub theme: Theme,
    /// `theme` before the text size, for what keeps 100% text: the desktop icon grid and the
    /// dialogs that do not grow with it (`Dialog::grows`).
    pub base_theme: Theme,
    pub keybinds: Vec<Keybind>,
    pub global_shortcuts: Vec<GlobalShortcut>,
    pub startmenu: Vec<MenuEntry>,
    pub quicklaunch: Vec<QuickLaunchEntry>,
    pub startmenu_banner: String,
    pub startmenu_style: StartMenuStyle,
    pub keyboard: KeyboardConfig,
    pub mouse: PointerConfig,
    pub touchpad: TouchpadConfig,
    pub touchscreen: TouchscreenConfig,
    pub idle: IdleConfig,
    pub idle_battery: IdleOverrides,
    pub power: PowerConfig,
    pub desktop_icons: bool,
    pub desktop_extensions: reprisefs::names::Extensions,
    pub start_icon: bool,
    /// The desktop's system icons, in order: `documents`, `computer`, `home`,
    /// `recycle-bin`. Old `my-documents`/`my-computer` read as the new names.
    pub system_icons: Vec<String>,
    pub wallpaper: WallpaperConfig,
    pub screensaver: ScreensaverConfig,
    pub outputs: Vec<OutputConfig>,
    pub adaptive_sync: bool,
    pub window_snapping: SnapConfig,
    pub screenshot: ScreenshotConfig,
    pub previews: PreviewsConfig,
    pub switcher: SwitcherConfig,
    pub hot_corner: HotCorner,
    pub gestures: GesturesConfig,
    pub clipboard: ClipboardConfig,
    pub recent_files: bool,
    pub location: bool,
    pub debug: DebugConfig,
    pub launcher_files: bool,
    pub night_light: NightLightConfig,
    pub workspaces: usize,
    pub workspace_animation: SwitchAnimation,
    pub pager_mark: reprisekit::taskbar::PagerMark,
    pub effects: EffectsConfig,
    pub accessibility: AccessibilityConfig,
    pub remember_windows: bool,
    pub window_tabs: bool,
    pub session: SessionConfig,
    pub window_rules: Vec<WindowRule>,
    pub tablet_pads: Vec<crate::tabletpad::PadConfig>,
    pub icons: IconsConfig,
    pub cursor: CursorConfig,
    pub toolkits: ToolkitsConfig,
    pub interface: Interface,
    pub color_scheme: Option<String>,
    pub blue_variant: reprisekit::blue::Variant,
    pub glass: reprisekit::glass::Settings,
    pub taskbar: TaskbarConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyboardConfig {
    pub rules: String,
    pub model: String,
    pub layout: String,
    pub variant: String,
    pub options: String,
    pub repeat_rate: i32,
    pub repeat_delay: i32,
}

impl Default for KeyboardConfig {
    fn default() -> Self {
        KeyboardConfig {
            rules: String::new(),
            model: String::new(),
            layout: String::new(),
            variant: String::new(),
            options: String::new(),
            repeat_rate: 25,
            repeat_delay: 600,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PointerConfig {
    pub speed: f64,
    pub acceleration: bool,
    pub natural_scroll: bool,
}

impl Default for PointerConfig {
    fn default() -> Self {
        PointerConfig {
            speed: 0.0,
            acceleration: true,
            natural_scroll: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TouchpadConfig {
    pub pointer: PointerConfig,
    pub tap_to_click: bool,
    pub two_finger_scroll: bool,
    pub two_finger_tap_right_click: bool,
    pub disable_while_typing: bool,
}

impl Default for TouchpadConfig {
    fn default() -> Self {
        TouchpadConfig {
            pointer: PointerConfig::default(),
            tap_to_click: true,
            two_finger_scroll: true,
            two_finger_tap_right_click: true,
            disable_while_typing: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TouchscreenConfig {
    pub output: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerAction {
    Nothing,
    ScreenOff,
    Lock,
    Standby,
    Hibernate,
    TurnOff,
    Ask,
}

impl PowerAction {
    const NAMES: [(&'static str, PowerAction); 7] = [
        ("nothing", PowerAction::Nothing),
        ("screen_off", PowerAction::ScreenOff),
        ("lock", PowerAction::Lock),
        ("standby", PowerAction::Standby),
        ("hibernate", PowerAction::Hibernate),
        ("turnoff", PowerAction::TurnOff),
        ("ask", PowerAction::Ask),
    ];

    pub fn parse(s: &str) -> Result<PowerAction, String> {
        Self::NAMES
            .iter()
            .find(|(n, _)| *n == s)
            .map(|(_, a)| *a)
            .ok_or_else(|| {
                let names: Vec<&str> = Self::NAMES.iter().map(|(n, _)| *n).collect();
                format!("unknown power action {s:?}; one of {}", names.join(", "))
            })
    }

    pub fn name(self) -> &'static str {
        Self::NAMES
            .iter()
            .find(|(_, a)| *a == self)
            .map(|(n, _)| *n)
            .expect("every action has a name")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerConfig {
    pub standby: String,
    pub poweroff: String,
    pub reboot: String,
    pub hibernate: String,
    pub lid_close: PowerAction,
    pub power_button: PowerAction,
    pub sleep_button: PowerAction,
    /// Percent. reprise-power raises the battery alarms; the compositor only carries these.
    pub battery_low: u32,
    pub battery_critical: u32,
    pub critical_action: PowerAction,
    pub always_show_icon: bool,
    pub lock_on_sleep: bool,
    /// Command for an ext-session-lock-v1 locker; empty keeps the built-in dialog (`sessionlock`).
    pub locker: String,
}

impl Default for PowerConfig {
    fn default() -> Self {
        PowerConfig {
            standby: "systemctl suspend".into(),
            poweroff: "systemctl poweroff".into(),
            reboot: "systemctl reboot".into(),
            hibernate: "systemctl hibernate".into(),
            lid_close: PowerAction::Standby,
            power_button: PowerAction::Ask,
            sleep_button: PowerAction::Standby,
            battery_low: 10,
            battery_critical: 3,
            critical_action: PowerAction::Hibernate,
            always_show_icon: false,
            lock_on_sleep: true,
            locker: String::new(),
        }
    }
}

/// Idle timeouts in seconds; zero disables the step. All pause while a client holds an idle inhibitor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdleConfig {
    pub lock_after: u32,
    pub screen_off_after: u32,
    pub blank_after: u32,
    pub suspend_after: u32,
    /// Seconds to dim before `lock_after`; any input brings it back. Zero locks
    /// without a dim; meaningless with `lock_after` 0.
    pub lock_fade: u32,
}

impl Default for IdleConfig {
    fn default() -> Self {
        IdleConfig {
            lock_after: 600,
            screen_off_after: 900,
            blank_after: 0,
            suspend_after: 0,
            lock_fade: 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdleOverrides {
    pub lock_after: Option<u32>,
    pub screen_off_after: Option<u32>,
    pub blank_after: Option<u32>,
    pub suspend_after: Option<u32>,
}

impl IdleOverrides {
    fn apply(&self, base: &IdleConfig) -> IdleConfig {
        IdleConfig {
            lock_after: self.lock_after.unwrap_or(base.lock_after),
            screen_off_after: self.screen_off_after.unwrap_or(base.screen_off_after),
            blank_after: self.blank_after.unwrap_or(base.blank_after),
            suspend_after: self.suspend_after.unwrap_or(base.suspend_after),
            lock_fade: base.lock_fade,
        }
    }
}

impl KeyboardConfig {
    pub fn xkb_config(&self) -> XkbConfig<'_> {
        XkbConfig {
            rules: &self.rules,
            model: &self.model,
            layout: &self.layout,
            variant: &self.variant,
            options: Some(self.options.clone()).filter(|o| !o.is_empty()),
        }
    }
}

/// `[[global_shortcuts]]` as entries. A repeated (app, id) keeps the first and
/// unbindable keys leave the entry unassigned; both are logged, neither fails the file.
fn global_shortcuts(list: Vec<FileGlobalShortcut>) -> Vec<GlobalShortcut> {
    let mut out: Vec<GlobalShortcut> = Vec::new();
    for e in list {
        if out.iter().any(|o| o.app == e.app && o.id == e.id) {
            log::warn!(
                "global_shortcuts: {} {} is listed twice; the first counts",
                e.app,
                e.id
            );
            continue;
        }
        let chord = if e.keys.is_empty() {
            None
        } else if repriseconf::keybind::modifier_only(&e.keys) {
            log::warn!(
                "global_shortcuts: {} {}: {:?} is modifier-only",
                e.app,
                e.id,
                e.keys
            );
            None
        } else {
            parse_keybind(&e.keys)
                .inspect_err(|err| log::warn!("global_shortcuts: {} {}: {err}", e.app, e.id))
                .ok()
        };
        out.push(GlobalShortcut {
            app: e.app,
            id: e.id,
            description: e.description,
            keys: e.keys,
            chord,
        });
    }
    out
}

/// "Mod+Mod+key" as (`keys::MOD_*` bits, lowercase keysym).
pub fn parse_keybind(s: &str) -> Result<(u32, u32), String> {
    let mut mods = 0u32;
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    let (key, modparts) = parts.split_last().ok_or("empty keybind")?;
    if key.is_empty() {
        return Err(format!("keybind {s:?} has no key"));
    }
    for m in modparts {
        mods |= match m.to_ascii_lowercase().as_str() {
            "super" | "logo" | "mod4" | "win" => MOD_LOGO,
            "ctrl" | "control" => MOD_CTRL,
            "alt" | "mod1" => MOD_ALT,
            "shift" => MOD_SHIFT,
            other => return Err(format!("unknown modifier {other:?} in {s:?}")),
        };
    }
    let sym = crate::keys::from_name(key).ok_or_else(|| format!("unknown key {key:?} in {s:?}"))?;
    Ok((mods, crate::keys::to_lower(sym)))
}

fn bind(keys: &str, action: Action) -> Keybind {
    let (mods, keysym) = parse_keybind(keys).expect("default keybind");
    Keybind {
        mods,
        keysym,
        action,
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            terminal: reprisefs::apps::DEFAULT_TERMINAL.into(),
            script_terminal: reprisefs::program::ScriptEnd::Stay,
            task_manager: None,
            retired: Vec::new(),
            theme: Theme::default(),
            base_theme: Theme::default(),
            keybinds: repriseconf::keybind::DEFAULTS
                .iter()
                .map(|(keys, action)| {
                    bind(
                        keys,
                        Action::from_parts(action, None).expect("default action"),
                    )
                })
                .collect(),
            global_shortcuts: Vec::new(),
            startmenu: default_startmenu(),
            quicklaunch: default_quicklaunch(),
            startmenu_banner: "Reprise Desktop".into(),
            startmenu_style: StartMenuStyle::Classic,
            keyboard: KeyboardConfig::default(),
            mouse: PointerConfig::default(),
            touchpad: TouchpadConfig::default(),
            touchscreen: TouchscreenConfig::default(),
            idle: IdleConfig::default(),
            idle_battery: IdleOverrides::default(),
            power: PowerConfig::default(),
            desktop_icons: true,
            desktop_extensions: reprisefs::names::Extensions::Hide,
            start_icon: false,
            system_icons: ["documents", "computer", "home", "recycle-bin"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            wallpaper: WallpaperConfig::default(),
            screensaver: ScreensaverConfig::default(),
            outputs: Vec::new(),
            adaptive_sync: false,
            window_snapping: SnapConfig::default(),
            screenshot: ScreenshotConfig::default(),
            previews: PreviewsConfig::default(),
            switcher: SwitcherConfig::default_for(Interface::Classic),
            hot_corner: HotCorner::None,
            gestures: GesturesConfig::default(),
            clipboard: ClipboardConfig::default(),
            recent_files: true,
            location: true,
            debug: DebugConfig::default(),
            launcher_files: true,
            night_light: NightLightConfig::default(),
            workspaces: 4,
            workspace_animation: SwitchAnimation::default(),
            pager_mark: reprisekit::taskbar::PagerMark::default(),
            effects: EffectsConfig::default_for(Interface::Classic),
            accessibility: AccessibilityConfig::default(),
            remember_windows: true,
            window_tabs: true,
            session: SessionConfig::default(),
            window_rules: Vec::new(),
            tablet_pads: Vec::new(),
            icons: IconsConfig::default(),
            cursor: CursorConfig::default(),
            toolkits: ToolkitsConfig::default(),
            interface: Interface::Classic,
            color_scheme: None,
            blue_variant: reprisekit::blue::Variant::Blue,
            glass: reprisekit::glass::Settings::default(),
            taskbar: TaskbarConfig::default_for(Interface::Classic),
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileFonts {
    family: Option<Vec<String>>,
    size_px: Option<u32>,
    antialias: Option<bool>,
    shell_dlg: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileKeybind {
    keys: String,
    action: String,
    command: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileGlobalShortcut {
    app: String,
    id: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    keys: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileQuickLaunch {
    label: String,
    icon: Option<String>,
    command: Option<String>,
    action: Option<String>,
}

impl FileQuickLaunch {
    fn parse(&self) -> Result<QuickLaunchEntry, String> {
        let action = match (&self.action, &self.command) {
            (Some(a), cmd) => Action::from_parts(a, cmd.as_deref())?,
            (None, Some(cmd)) => Action::Spawn(cmd.clone()),
            (None, None) => {
                return Err(format!(
                    "quicklaunch entry {:?} needs a command or action",
                    self.label
                ));
            }
        };
        Ok(QuickLaunchEntry {
            label: self.label.clone(),
            icon: self.icon.clone().filter(|s| !s.is_empty()),
            action,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileMenuEntry {
    label: Option<String>,
    icon: Option<String>,
    command: Option<String>,
    action: Option<String>,
    separator: Option<bool>,
    submenu: Option<String>,
    items: Option<Vec<FileMenuEntry>>,
    tip: Option<String>,
}

impl FileMenuEntry {
    fn parse(&self) -> Result<MenuEntry, String> {
        if self.separator == Some(true) {
            return Ok(MenuEntry::Separator);
        }
        let label = self
            .label
            .clone()
            .ok_or("startmenu entry needs a label or separator = true")?;
        let target = match (&self.action, &self.command, &self.submenu, &self.items) {
            (Some(a), cmd, None, None) => {
                MenuTarget::Action(Action::from_parts(a, cmd.as_deref())?)
            }
            (None, Some(cmd), None, None) => MenuTarget::Action(Action::Spawn(cmd.clone())),
            (None, None, Some(kind), None) => match kind.as_str() {
                "programs" => MenuTarget::Programs,
                "settings" => MenuTarget::Settings,
                "documents" => MenuTarget::Documents,
                other => {
                    return Err(format!(
                        "startmenu entry {label:?}: unknown submenu {other:?}, expected programs|settings|documents"
                    ));
                }
            },
            (None, None, None, Some(items)) => MenuTarget::Submenu(
                items
                    .iter()
                    .map(FileMenuEntry::parse)
                    .collect::<Result<Vec<_>, String>>()?,
            ),
            // A label alone is a folder whose rows were all taken out.
            (None, None, None, None) => MenuTarget::Submenu(Vec::new()),
            _ => {
                return Err(format!(
                    "startmenu entry {label:?}: command/action, submenu and items are mutually exclusive"
                ));
            }
        };
        Ok(MenuEntry::Item {
            label,
            icon: self.icon.clone().filter(|s| !s.is_empty()),
            target,
            tip: self.tip.clone().filter(|s| !s.is_empty()),
        })
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileKeyboard {
    rules: Option<String>,
    model: Option<String>,
    layout: Option<String>,
    variant: Option<String>,
    options: Option<String>,
    repeat_rate: Option<i32>,
    repeat_delay: Option<i32>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileMouse {
    speed: Option<f64>,
    acceleration: Option<bool>,
    natural_scroll: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileTouchpad {
    speed: Option<f64>,
    acceleration: Option<bool>,
    natural_scroll: Option<bool>,
    tap_to_click: Option<bool>,
    two_finger_scroll: Option<bool>,
    two_finger_tap_right_click: Option<bool>,
    disable_while_typing: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileTouchscreen {
    output: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileIdle {
    lock_after: Option<i64>,
    screen_off_after: Option<i64>,
    blank_after: Option<i64>,
    suspend_after: Option<i64>,
    lock_fade: Option<i64>,
    battery: Option<FileIdleBattery>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileIdleBattery {
    lock_after: Option<i64>,
    screen_off_after: Option<i64>,
    blank_after: Option<i64>,
    suspend_after: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilePower {
    standby: Option<String>,
    poweroff: Option<String>,
    reboot: Option<String>,
    hibernate: Option<String>,
    lid_close: Option<String>,
    power_button: Option<String>,
    sleep_button: Option<String>,
    battery_low: Option<i64>,
    battery_critical: Option<i64>,
    critical_action: Option<String>,
    always_show_icon: Option<bool>,
    lock_on_sleep: Option<bool>,
    locker: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileOutput {
    name: String,
    scale: Option<f64>,
    mode: Option<String>,
    adaptive_sync: Option<bool>,
    transform: Option<String>,
    /// An output's snap grid, read only to be ignored.
    #[allow(dead_code)]
    window_snapping: Option<serde::de::IgnoredAny>,
    position: Option<[i64; 2]>,
    primary: Option<bool>,
    /// Retired: the renderer is OpenGL ES and drives no HDR or ICC
    /// profile. Read only to name it in [`Config::warn_retired`].
    color_profile: Option<serde::de::IgnoredAny>,
    hdr: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileSnapping {
    enabled: Option<bool>,
    picker: Option<bool>,
    bar: Option<bool>,
    layouts: Option<Vec<String>>,
    /// The snap grid's keys, read only to be ignored: the grid is gone.
    #[allow(dead_code)]
    threshold: Option<serde::de::IgnoredAny>,
    #[allow(dead_code)]
    columns: Option<serde::de::IgnoredAny>,
    #[allow(dead_code)]
    rows: Option<serde::de::IgnoredAny>,
    #[allow(dead_code)]
    column_weights: Option<serde::de::IgnoredAny>,
    #[allow(dead_code)]
    row_weights: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileScreenshot {
    save: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FilePreviews {
    taskbar: Option<bool>,
    fps: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileOverview {
    hot_corner: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileGestures {
    enabled: Option<bool>,
    fingers: Option<i64>,
    distance: Option<i64>,
    commit_at: Option<i64>,
    flick_speed: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileSwitcher {
    style: Option<String>,
    size: Option<String>,
    preview: Option<bool>,
    desktop: Option<bool>,
    peek: Option<bool>,
    flip: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FilePrivacy {
    recent_files: Option<bool>,
    location: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileDebug {
    overlay: Option<bool>,
    overlay_in_captures: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileLauncher {
    search_files: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileClipboard {
    history: Option<bool>,
    terminal_apps: Option<Vec<String>>,
    trusted_apps: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileWorkspaces {
    count: Option<i64>,
    animation: Option<String>,
    animation_ms: Option<i64>,
    pager: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileEffects {
    window_animation: Option<bool>,
    window_open_ms: Option<i64>,
    window_close_ms: Option<i64>,
    window_scale: Option<f64>,
    hover_fade: Option<bool>,
    hover_in_ms: Option<i64>,
    hover_out_ms: Option<i64>,
    taskbar_slide: Option<bool>,
    taskbar_hide_ms: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileWindows {
    remember: Option<bool>,
    tabs: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileSession {
    restore: Option<bool>,
    exclude: Option<Vec<String>>,
    end_processes: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileWindowRule {
    app_id: String,
    title: Option<String>,
    size: Option<[i64; 2]>,
    position: Option<[i64; 2]>,
    output: Option<String>,
    state: Option<String>,
    desktop: Option<i64>,
    always_on_top: Option<bool>,
    sticky: Option<bool>,
    tab_group: Option<String>,
}

impl FileWindowRule {
    fn parse(self, at: &str) -> Result<WindowRule, String> {
        if self.app_id.is_empty() {
            return Err(format!("{at}: app_id is empty"));
        }
        let size = match self.size {
            None => None,
            Some([w, h]) if (1..=32767).contains(&w) && (1..=32767).contains(&h) => {
                Some((w as i32, h as i32))
            }
            Some([w, h]) => {
                return Err(format!(
                    "{at}: size must be two numbers from 1 to 32767, got [{w}, {h}]"
                ));
            }
        };
        let span = -32768..=32767;
        let position = match self.position {
            None => None,
            Some([x, y]) if span.contains(&x) && span.contains(&y) => Some((x as i32, y as i32)),
            Some([x, y]) => {
                return Err(format!(
                    "{at}: position must be two numbers from -32768 to 32767, got [{x}, {y}]"
                ));
            }
        };
        let state = self
            .state
            .map(|s| {
                WindowState::parse(&s).ok_or_else(|| {
                    format!(
                        "{at}: state {s:?} is not one of {}",
                        WindowState::ALL.map(WindowState::name).join(", ")
                    )
                })
            })
            .transpose()?;
        let desktop = self
            .desktop
            .map(|n| {
                usize::try_from(n)
                    .ok()
                    .filter(|n| (1..=WORKSPACES_MAX).contains(n))
                    .map(|n| n - 1)
                    .ok_or_else(|| {
                        format!("{at}: desktop must be between 1 and {WORKSPACES_MAX}, got {n}")
                    })
            })
            .transpose()?;
        Ok(WindowRule {
            app_id: self.app_id,
            title: self.title,
            size,
            position,
            output: self.output,
            state,
            desktop,
            always_on_top: self.always_on_top,
            sticky: self.sticky,
            tab_group: self.tab_group,
        })
    }
}

pub const WORKSPACES_MAX: usize = 16;

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileNightLight {
    enabled: Option<bool>,
    temperature: Option<i64>,
    schedule: Option<String>,
    from: Option<String>,
    to: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
}

impl FileNightLight {
    fn apply(self, cfg: &mut NightLightConfig) -> Result<(), String> {
        if let Some(v) = self.enabled {
            cfg.enabled = v;
        }
        if let Some(t) = self.temperature {
            let (lo, hi) = (*NIGHT_KELVIN.start(), *NIGHT_KELVIN.end());
            if !(lo as i64..=hi as i64).contains(&t) {
                return Err(format!(
                    "night_light.temperature must be between {lo} and {hi}, got {t}"
                ));
            }
            cfg.temperature = t as u32;
        }
        if let Some(s) = self.schedule {
            cfg.schedule = match s.as_str() {
                "sun" => NightSchedule::Sun,
                "fixed" => NightSchedule::Fixed,
                other => {
                    return Err(format!(
                        "night_light.schedule {other:?} is not \"sun\" or \"fixed\""
                    ));
                }
            };
        }
        for (key, text, slot) in [
            ("from", self.from, &mut cfg.from),
            ("to", self.to, &mut cfg.to),
        ] {
            if let Some(t) = text {
                *slot = parse_hhmm(&t).ok_or_else(|| {
                    format!("night_light.{key} {t:?} is not a time like \"21:00\"")
                })?;
            }
        }
        if cfg.from == cfg.to {
            return Err("night_light.from and night_light.to must differ".into());
        }
        match (self.latitude, self.longitude) {
            (None, None) => {}
            (Some(lat), Some(lon)) => {
                if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
                    return Err(format!(
                        "night_light: latitude must be within -90..90 and longitude \
                         within -180..180, got {lat}, {lon}"
                    ));
                }
                cfg.location = Some((lat, lon));
            }
            _ => return Err("night_light: latitude and longitude go together".into()),
        }
        Ok(())
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileAccessibility {
    sticky_keys: Option<bool>,
    sticky_lock: Option<bool>,
    sticky_two_keys_off: Option<bool>,
    slow_keys: Option<bool>,
    slow_keys_delay: Option<f64>,
    shortcuts: Option<bool>,
    color_filter: Option<String>,
    color_filter_intensity: Option<i64>,
    magnifier_follow: Option<String>,
    mouse_keys: Option<bool>,
    mouse_keys_speed: Option<u32>,
    mouse_keys_acceleration: Option<f64>,
    mouse_keys_num_lock: Option<bool>,
    text_size: Option<u32>,
}

impl FileAccessibility {
    fn apply(self, cfg: &mut AccessibilityConfig) -> Result<(), String> {
        if let Some(name) = &self.color_filter {
            cfg.color_filter = reprisekit::colorfilter::Filter::parse(name).ok_or_else(|| {
                let names: Vec<&str> = reprisekit::colorfilter::Filter::ALL
                    .iter()
                    .map(|f| f.name())
                    .collect();
                format!(
                    "accessibility.color_filter must be one of {}, got {name:?}",
                    names.join(", ")
                )
            })?;
        }
        if let Some(p) = self.color_filter_intensity {
            cfg.color_filter_intensity =
                u8::try_from(p).ok().filter(|p| *p <= 100).ok_or_else(|| {
                    format!("accessibility.color_filter_intensity must be 0 to 100, got {p}")
                })?;
        }
        if let Some(name) = &self.magnifier_follow {
            cfg.magnifier_follow = crate::magnifier::Follow::parse(name).ok_or_else(|| {
                format!(
                    "accessibility.magnifier_follow must be \"pointer\" or \"pointer+focus\", got {name:?}"
                )
            })?;
        }
        for (v, slot) in [
            (self.sticky_keys, &mut cfg.sticky_keys),
            (self.sticky_lock, &mut cfg.sticky_lock),
            (self.sticky_two_keys_off, &mut cfg.sticky_two_keys_off),
            (self.slow_keys, &mut cfg.slow_keys),
            (self.shortcuts, &mut cfg.shortcuts),
            (self.mouse_keys, &mut cfg.mouse_keys),
            (self.mouse_keys_num_lock, &mut cfg.mouse_keys_num_lock),
        ] {
            if let Some(v) = v {
                *slot = v;
            }
        }
        if let Some(d) = self.slow_keys_delay {
            // A hair of slack, so 0.3 written as 0.30000000000000004 passes.
            let (lo, hi) = (*SLOW_KEYS_DELAY.start(), *SLOW_KEYS_DELAY.end());
            if !(lo - 1e-9..=hi + 1e-9).contains(&d) {
                return Err(format!(
                    "accessibility.slow_keys_delay must be between {lo} and {hi} seconds, got {d}"
                ));
            }
            cfg.slow_keys_delay = (d * 1000.0).round() as u32;
        }
        if let Some(v) = self.mouse_keys_speed {
            if !MOUSE_KEYS_SPEED.contains(&v) {
                return Err(format!(
                    "accessibility.mouse_keys_speed must be between {} and {} pixels a second, got {v}",
                    MOUSE_KEYS_SPEED.start(),
                    MOUSE_KEYS_SPEED.end()
                ));
            }
            cfg.mouse_keys_speed = v;
        }
        if let Some(a) = self.mouse_keys_acceleration {
            let (lo, hi) = (
                *MOUSE_KEYS_ACCELERATION.start(),
                *MOUSE_KEYS_ACCELERATION.end(),
            );
            if !(lo - 1e-9..=hi + 1e-9).contains(&a) {
                return Err(format!(
                    "accessibility.mouse_keys_acceleration must be between {lo} and {hi} seconds, got {a}"
                ));
            }
            cfg.mouse_keys_acceleration = (a.max(0.0) * 1000.0).round() as u32;
        }
        if let Some(t) = self.text_size {
            if !TEXT_SIZE.contains(&t) || t % TEXT_SIZE_STEP != 0 {
                return Err(format!(
                    "accessibility.text_size must be one of 100, 125, 150, 175 or 200, got {t}"
                ));
            }
            cfg.text_size = t;
        }
        Ok(())
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileIcons {
    theme: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileToolkits {
    gtk_theme: Option<String>,
    qt_platform_theme: Option<String>,
    font_smoothing: Option<String>,
    gtk4_colors: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileCursor {
    theme: Option<String>,
    size: Option<u32>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileAppearance {
    interface: Option<String>,
    color_scheme: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileGlass {
    color: Option<String>,
    intensity: Option<u8>,
    transparency: Option<bool>,
    blur: Option<u8>,
    streaks: Option<bool>,
    fast_drag: Option<bool>,
    blur_frames: Option<bool>,
    blur_rate: Option<u16>,
    orb_glow: Option<u8>,
    mode: Option<String>,
    material: Option<String>,
    frame: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileTaskbar {
    position: Option<String>,
    buttons: Option<String>,
    pinned: Option<bool>,
    small_icons: Option<bool>,
    show_desktop_button: Option<bool>,
    peek: Option<bool>,
    tray_overflow: Option<bool>,
    auto_hide: Option<bool>,
    reveal_on_edge: Option<bool>,
    reveal_on_keys: Option<bool>,
    reveal_on_attention: Option<bool>,
    side_width: Option<i64>,
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

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileWallpaper {
    image: Option<String>,
    position: Option<String>,
    lock_screen: Option<bool>,
    slideshow: Option<String>,
    interval: Option<i64>,
    shuffle: Option<bool>,
    water: Option<String>,
    water_rate: Option<toml::Value>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScreensaverConfig {
    pub kind: reprisekit::saver::Kind,
    pub options: reprisekit::saver::Options,
    pub folder: Option<PathBuf>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileScreensaver {
    name: Option<String>,
    speed: Option<i64>,
    stars: Option<i64>,
    shapes: Option<i64>,
    cycle_colors: Option<bool>,
    folder: Option<String>,
    photo_seconds: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    terminal: Option<String>,
    script_terminal: Option<String>,
    task_manager: Option<String>,
    /// Retired, as `FileOutput::hdr`.
    renderer: Option<serde::de::IgnoredAny>,
    colors: Option<HashMap<String, String>>,
    metrics: Option<HashMap<String, i32>>,
    fonts: Option<FileFonts>,
    keybinds: Option<Vec<FileKeybind>>,
    global_shortcuts: Option<Vec<FileGlobalShortcut>>,
    /// `[portal]` is read by reprise-portal; accepted here so the file parses.
    #[allow(dead_code)]
    portal: Option<serde::de::IgnoredAny>,
    /// `[recycle_bin]` is read by `reprisefs::bincap::Policy`; accepted here so the file parses.
    #[allow(dead_code)]
    recycle_bin: Option<serde::de::IgnoredAny>,
    /// `[drives]` is read by reprise-drives; accepted here so the file parses.
    #[allow(dead_code)]
    drives: Option<serde::de::IgnoredAny>,
    /// `[thumbnail_cache]` is read by `repriseconf::thumbnail_cache`; accepted here so the file parses.
    #[allow(dead_code)]
    thumbnail_cache: Option<serde::de::IgnoredAny>,
    startmenu: Option<Vec<FileMenuEntry>>,
    quicklaunch: Option<Vec<FileQuickLaunch>>,
    startmenu_banner: Option<String>,
    startmenu_style: Option<String>,
    keyboard: Option<FileKeyboard>,
    mouse: Option<FileMouse>,
    touchpad: Option<FileTouchpad>,
    touchscreen: Option<FileTouchscreen>,
    idle: Option<FileIdle>,
    power: Option<FilePower>,
    desktop_icons: Option<bool>,
    desktop_extensions: Option<String>,
    start_icon: Option<bool>,
    system_icons: Option<Vec<String>>,
    wallpaper: Option<FileWallpaper>,
    screensaver: Option<FileScreensaver>,
    outputs: Option<Vec<FileOutput>>,
    adaptive_sync: Option<bool>,
    window_snapping: Option<FileSnapping>,
    screenshot: Option<FileScreenshot>,
    previews: Option<FilePreviews>,
    switcher: Option<FileSwitcher>,
    overview: Option<FileOverview>,
    gestures: Option<FileGestures>,
    clipboard: Option<FileClipboard>,
    privacy: Option<FilePrivacy>,
    debug: Option<FileDebug>,
    launcher: Option<FileLauncher>,
    night_light: Option<FileNightLight>,
    workspaces: Option<FileWorkspaces>,
    effects: Option<FileEffects>,
    accessibility: Option<FileAccessibility>,
    windows: Option<FileWindows>,
    session: Option<FileSession>,
    window_rules: Option<Vec<FileWindowRule>>,
    tablet_pads: Option<Vec<crate::tabletpad::FilePad>>,
    icons: Option<FileIcons>,
    cursor: Option<FileCursor>,
    toolkits: Option<FileToolkits>,
    appearance: Option<FileAppearance>,
    glass: Option<FileGlass>,
    taskbar: Option<FileTaskbar>,
}

fn apply_pointer(
    cfg: &mut PointerConfig,
    section: &str,
    speed: Option<f64>,
    acceleration: Option<bool>,
    natural_scroll: Option<bool>,
) -> Result<(), String> {
    if let Some(v) = speed {
        if !(-1.0..=1.0).contains(&v) {
            return Err(format!("{section}.speed must be between -1 and 1"));
        }
        cfg.speed = v;
    }
    if let Some(v) = acceleration {
        cfg.acceleration = v;
    }
    if let Some(v) = natural_scroll {
        cfg.natural_scroll = v;
    }
    Ok(())
}

/// Largest `[metrics]` value, pixels or milliseconds: everything past it
/// is a typo, and the consumers do their arithmetic unclamped.
const METRIC_MAX: i32 = 10_000;

fn home_path(s: &str, home: Option<&std::ffi::OsStr>) -> Result<PathBuf, String> {
    let path = match (s.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => Path::new(home).join(rest),
        (Some(_), None) => return Err(format!("{s:?} starts with ~/ and HOME is not set")),
        (None, _) => PathBuf::from(s),
    };
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!("{s:?} must be an absolute path or start with ~/"))
    }
}

fn wallpaper_image(s: &str, home: Option<&std::ffi::OsStr>) -> Result<Option<PathBuf>, String> {
    if s.is_empty() {
        return Ok(None);
    }
    if s.starts_with("~/") {
        return home_path(s, home)
            .map(Some)
            .map_err(|e| format!("wallpaper.image {e}"));
    }
    Ok(Some(PathBuf::from(s)))
}

/// A glass-tinted built-in resolves to a concrete variant, so a new glass colour
/// or mode is a new path and the wallpaper redraws.
fn builtin_path(
    b: reprisekit::wallpapers::Builtin,
    glass: &reprisekit::glass::Settings,
) -> PathBuf {
    PathBuf::from(reprisekit::interfaces::resolve_wallpaper(b, glass).uri())
}

const UNREADABLE_NOTICE: &str = "Reprise could not read config.toml and started with the \
                                 defaults. The session log names the error.";

impl Config {
    pub fn default_path() -> PathBuf {
        repriseconf::config_path()
    }

    /// The whole file or an error: what a live reload and a config write check against.
    pub fn parse(text: &str) -> Result<Config, String> {
        Config::from_file(toml::from_str(text).map_err(|e| e.to_string())?)
    }

    /// `text` with every top-level key that fails on its own left at its default.
    /// Returns the config and the dropped keys; Err only when `text` is not TOML.
    pub fn parse_lenient(text: &str) -> Result<(Config, Vec<(String, String)>), String> {
        let mut table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
        let from_table = |t: toml::Table| {
            Config::from_file(t.try_into().map_err(|e: toml::de::Error| e.to_string())?)
        };
        let mut dropped = Vec::new();
        for (key, value) in &table {
            let alone = toml::Table::from_iter([(key.clone(), value.clone())]);
            if let Err(e) = from_table(alone) {
                dropped.push((key.clone(), e.trim_end().to_string()));
            }
        }
        for (key, _) in &dropped {
            table.remove(key);
        }
        Ok((from_table(table)?, dropped))
    }

    fn from_file(file: FileConfig) -> Result<Config, String> {
        let mut cfg = Config::default();
        if let Some(t) = file.terminal {
            cfg.terminal = t;
        }
        if let Some(v) = file.script_terminal {
            cfg.script_terminal = reprisefs::program::ScriptEnd::parse(&v)
                .ok_or_else(|| format!("script_terminal = {v:?} is not stay or close"))?;
        }
        cfg.task_manager = file.task_manager;
        if file.renderer.is_some() {
            cfg.retired.push("renderer".into());
        }
        let appearance = file.appearance.unwrap_or_default();
        if let Some(s) = appearance.interface {
            match Interface::parse(&s) {
                Some(i) => cfg.interface = i,
                None => log::warn!("[appearance] interface = {s:?} is unknown; using classic"),
            }
        }
        cfg.color_scheme = appearance.color_scheme;
        if cfg.interface == Interface::Blue
            && let Some(name) = cfg.color_scheme.as_deref()
        {
            match reprisekit::blue::Variant::from_name(name) {
                Some(v) => cfg.blue_variant = v,
                None => {
                    log::warn!(
                        "[appearance] color_scheme = {name:?} is not a Blue scheme; using Blue"
                    )
                }
            }
        }
        if let Some(g) = file.glass {
            if let Some(name) = g.color {
                let color = if name.starts_with('#') {
                    Color::parse(&name)
                } else {
                    reprisekit::glass::swatch(&name)
                };
                cfg.glass.color = color.ok_or_else(|| {
                    format!("[glass] color = {name:?} is neither a colour name nor #RRGGBB")
                })?;
            }
            if let Some(i) = g.intensity {
                if i > 100 {
                    return Err(format!("[glass] intensity must be 0 to 100, got {i}"));
                }
                cfg.glass.intensity = i;
            }
            if let Some(t) = g.transparency {
                cfg.glass.transparency = t;
            }
            if let Some(b) = g.blur {
                let max = reprisekit::glass::BLUR_MAX;
                if !(1..=max).contains(&b) {
                    return Err(format!("[glass] blur must be 1 to {max}, got {b}"));
                }
                cfg.glass.blur = b;
            }
            if let Some(on) = g.streaks {
                cfg.glass.streaks = on;
            }
            if let Some(f) = g.fast_drag {
                cfg.glass.fast_drag = f;
            }
            if let Some(f) = g.blur_frames {
                cfg.glass.blur_frames = f;
            }
            if let Some(r) = g.blur_rate {
                cfg.glass.blur_rate = r;
            }
            if let Some(o) = g.orb_glow {
                if o > 100 {
                    return Err(format!("[glass] orb_glow must be 0 to 100, got {o}"));
                }
                cfg.glass.orb_glow = o;
            }
            if let Some(m) = g.mode {
                cfg.glass.mode = reprisekit::glass::Mode::parse(&m)
                    .ok_or_else(|| format!("[glass] mode = {m:?} is not light or dark"))?;
            }
            if let Some(m) = g.material {
                cfg.glass.material = reprisekit::glass::Material::parse(&m).ok_or_else(|| {
                    format!("[glass] material = {m:?} is not frosted, smoked or molten")
                })?;
            }
            if let Some(f) = g.frame {
                cfg.glass.frame = reprisekit::glass::Frame::parse(&f)
                    .ok_or_else(|| format!("[glass] frame = {f:?} is not rounded or square"))?;
            }
        }
        cfg.taskbar = TaskbarConfig::default_for(cfg.interface);
        cfg.startmenu_style = StartMenuStyle::default_for(cfg.interface);
        if let Some(s) = &file.startmenu_style {
            cfg.startmenu_style = StartMenuStyle::parse(s)
                .ok_or_else(|| format!("startmenu_style = {s:?} is not classic or glass"))?;
        }
        cfg.switcher = SwitcherConfig::default_for(cfg.interface);
        cfg.effects = EffectsConfig::default_for(cfg.interface);
        if let Some(t) = file.taskbar {
            if let Some(p) = t.position {
                cfg.taskbar.position =
                    reprisekit::taskbar::Edge::from_name(&p).ok_or_else(|| {
                        format!("[taskbar] position = {p:?} is not bottom, top, left or right")
                    })?;
            }
            if let Some(b) = t.buttons {
                cfg.taskbar.buttons = TaskbarButtons::parse(&b).ok_or_else(|| {
                    format!("[taskbar] buttons = {b:?} is not combine, when_full, never or group")
                })?;
            }
            let flags = [
                (t.pinned, &mut cfg.taskbar.pinned),
                (t.small_icons, &mut cfg.taskbar.small_icons),
                (t.show_desktop_button, &mut cfg.taskbar.show_desktop_button),
                (t.peek, &mut cfg.taskbar.peek),
                (t.tray_overflow, &mut cfg.taskbar.tray_overflow),
                (t.auto_hide, &mut cfg.taskbar.auto_hide),
                (t.reveal_on_edge, &mut cfg.taskbar.reveal_on_edge),
                (t.reveal_on_keys, &mut cfg.taskbar.reveal_on_keys),
                (t.reveal_on_attention, &mut cfg.taskbar.reveal_on_attention),
            ];
            for (v, at) in flags {
                if let Some(v) = v {
                    *at = v;
                }
            }
            if let Some(w) = t.side_width {
                cfg.taskbar.side_width =
                    i32::try_from(w).ok().filter(|w| *w >= 0).ok_or_else(|| {
                        format!(
                            "[taskbar] side_width must be between 0 and {}, got {w}",
                            i32::MAX
                        )
                    })?;
            }
        }
        cfg.theme = reprisekit::interfaces::theme_with(cfg.interface, cfg.blue_variant, &cfg.glass);
        // Classic and Blue already run small icons at their usual height. A
        // `[metrics] taskbar_height` below still wins over the small bar.
        if cfg.interface == Interface::Glass && cfg.taskbar.small_icons {
            reprisekit::interfaces::small_bar(&mut cfg.theme, &cfg.glass);
        }
        if cfg.interface == Interface::Classic
            && let Some(name) = cfg.color_scheme.as_deref()
        {
            match reprisekit::schemes::scheme(name) {
                Some(colors) => cfg.theme.colors = colors,
                None => log::warn!("[appearance] color_scheme = {name:?} is not a Classic scheme"),
            }
        }
        for (name, value) in file.colors.unwrap_or_default() {
            let c =
                Color::parse(&value).ok_or_else(|| format!("bad color {value:?} for {name}"))?;
            if !cfg.theme.colors.set(&name, c) {
                return Err(format!("unknown color name {name:?}"));
            }
        }
        for (name, value) in file.metrics.unwrap_or_default() {
            if !(0..=METRIC_MAX).contains(&value) {
                return Err(format!(
                    "metrics.{name} must be between 0 and {METRIC_MAX}, got {value}"
                ));
            }
            if !cfg.theme.metrics.set(&name, value) {
                return Err(format!("unknown metric name {name:?}"));
            }
        }
        // The tray centres an icon in a well 6 px inside the bar.
        let min_taskbar = reprisekit::icon::ICON_SIZE + 6;
        if cfg.theme.metrics.taskbar_height < min_taskbar {
            return Err(format!(
                "metrics.taskbar_height must be >= {min_taskbar}, got {}",
                cfg.theme.metrics.taskbar_height
            ));
        }
        if let Some(f) = file.fonts {
            if let Some(fam) = f.family {
                cfg.theme.fonts.family = fam;
            }
            if let Some(s) = f.size_px {
                let (min, max) = (FontConfig::SIZE_PX_MIN, FontConfig::SIZE_PX_MAX);
                if !(min..=max).contains(&s) {
                    return Err(format!(
                        "fonts.size_px must be between {min} and {max}, got {s}"
                    ));
                }
                cfg.theme.fonts.size_px = s;
            }
            if let Some(a) = f.antialias {
                cfg.theme.fonts.antialias = a;
            }
            cfg.theme.fonts.shell_dlg = f.shell_dlg.filter(|s| !s.trim().is_empty());
        }
        if let Some(binds) = file.keybinds {
            for b in &binds {
                parse_keybind(&b.keys)?;
            }
            let layered = repriseconf::keybind::layered(
                binds,
                |b| (&b.keys, &b.action),
                |keys, action| FileKeybind {
                    keys: keys.into(),
                    action: action.into(),
                    command: None,
                },
            );
            cfg.keybinds = layered
                .iter()
                .map(|b| {
                    let (mods, keysym) = parse_keybind(&b.keys)?;
                    Ok(Keybind {
                        mods,
                        keysym,
                        action: Action::from_parts(&b.action, b.command.as_deref())?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
        }
        if let Some(list) = file.global_shortcuts {
            cfg.global_shortcuts = global_shortcuts(list);
        }
        if let Some(entries) = file.startmenu {
            cfg.startmenu = entries
                .iter()
                .map(FileMenuEntry::parse)
                .collect::<Result<Vec<_>, String>>()?;
        }
        if let Some(entries) = file.quicklaunch {
            cfg.quicklaunch = entries
                .iter()
                .map(FileQuickLaunch::parse)
                .collect::<Result<Vec<_>, String>>()?;
        }
        if let Some(b) = file.startmenu_banner {
            cfg.startmenu_banner = b;
        }
        if let Some(v) = file.desktop_icons {
            cfg.desktop_icons = v;
        }
        if let Some(v) = file.desktop_extensions {
            cfg.desktop_extensions = reprisefs::names::Extensions::parse(&v).ok_or_else(|| {
                format!("desktop_extensions = {v:?} is not hide, hide_launchers or show")
            })?;
        }
        if let Some(v) = file.start_icon {
            cfg.start_icon = v;
        }
        if let Some(v) = file.system_icons {
            cfg.system_icons =
                repriseconf::system_icons::current_list(v.iter().map(String::as_str));
        }
        cfg.wallpaper.lock_screen = cfg.interface != Interface::Classic;
        let mut image_set = false;
        if let Some(w) = file.wallpaper {
            if let Some(v) = w.lock_screen {
                cfg.wallpaper.lock_screen = v;
            }
            image_set = w.image.is_some();
            cfg.wallpaper.image = match w.image {
                Some(s) => match reprisekit::wallpapers::Builtin::from_uri(&s) {
                    Some(b) => Some(builtin_path(
                        b.map_err(|e| format!("wallpaper.image {e}"))?,
                        &cfg.glass,
                    )),
                    None => wallpaper_image(&s, std::env::var_os("HOME").as_deref())?,
                },
                None => None,
            };
            if let Some(p) = w.position {
                cfg.wallpaper.position =
                    crate::wallpaper::Position::parse(&p).ok_or_else(|| {
                        format!("wallpaper.position {p:?} is not center, tile or stretch")
                    })?;
            }
            cfg.wallpaper.slideshow = w
                .slideshow
                .map(|s| wallpaper_image(&s, std::env::var_os("HOME").as_deref()))
                .transpose()
                .map_err(|e| e.replace("wallpaper.image", "wallpaper.slideshow"))?
                .flatten();
            if let Some(v) = w.interval {
                if !(1..=1440).contains(&v) {
                    return Err("wallpaper.interval must be 1 to 1440 minutes".into());
                }
                cfg.wallpaper.interval = v as u32;
            }
            if let Some(v) = w.shuffle {
                cfg.wallpaper.shuffle = v;
            }
            if let Some(v) = w.water {
                cfg.wallpaper.water = reprisekit::water::Water::parse(&v).ok_or_else(|| {
                    format!("wallpaper.water {v:?} is not off, calm, lively or rain")
                })?;
            }
            if let Some(v) = w.water_rate {
                let rate = match &v {
                    toml::Value::String(s) => reprisekit::water::Rate::parse(s),
                    toml::Value::Integer(n) => reprisekit::water::Rate::parse(&n.to_string()),
                    _ => None,
                };
                cfg.wallpaper.water_rate = rate.ok_or_else(|| {
                    format!("wallpaper.water_rate {v} is not \"refresh\" or 30 to 240")
                })?;
            }
        }
        if !image_set {
            cfg.wallpaper.image = reprisekit::interfaces::default_wallpaper(cfg.interface)
                .map(|b| builtin_path(b, &cfg.glass));
        }
        if cfg
            .wallpaper
            .image
            .as_deref()
            .and_then(crate::wallpaper::builtin)
            .is_some()
        {
            cfg.wallpaper.position = crate::wallpaper::Position::Stretch;
        }
        if let Some(f) = file.screensaver {
            let ss = &mut cfg.screensaver;
            if let Some(n) = f.name {
                ss.kind = reprisekit::saver::Kind::parse(&n).ok_or_else(|| {
                    format!(
                        "screensaver.name {n:?} is not blank, starfield, mystify, logo or photos"
                    )
                })?;
            }
            let num = |key: &str, v: Option<i64>, (lo, hi): (u32, u32)| match v {
                Some(v) if v < lo as i64 || v > hi as i64 => {
                    Err(format!("screensaver.{key} must be {lo} to {hi}"))
                }
                v => Ok(v.map(|v| v as u32)),
            };
            use reprisekit::saver::Options as O;
            let o = &mut ss.options;
            if let Some(v) = num("speed", f.speed, O::SPEED)? {
                o.speed = v;
            }
            if let Some(v) = num("stars", f.stars, O::STARS)? {
                o.stars = v;
            }
            if let Some(v) = num("shapes", f.shapes, O::SHAPES)? {
                o.shapes = v;
            }
            if let Some(v) = num("photo_seconds", f.photo_seconds, O::PHOTO_SECONDS)? {
                o.photo_seconds = v;
            }
            if let Some(v) = f.cycle_colors {
                o.cycle_colors = v;
            }
            ss.folder = f
                .folder
                .map(|s| wallpaper_image(&s, std::env::var_os("HOME").as_deref()))
                .transpose()
                .map_err(|e| e.replace("wallpaper.image", "screensaver.folder"))?
                .flatten();
        }
        if let Some(i) = file.icons
            && let Some(t) = i.theme
        {
            cfg.icons.theme = t;
        }
        if let Some(t) = file.toolkits {
            if let Some(v) = t.gtk_theme {
                cfg.toolkits.gtk_theme = v;
            }
            if let Some(v) = t.qt_platform_theme {
                cfg.toolkits.qt_platform_theme = v;
            }
            if let Some(v) = t.gtk4_colors {
                cfg.toolkits.gtk4_colors = v;
            }
            if let Some(s) = t.font_smoothing {
                match FontSmoothing::parse(&s) {
                    Some(f) => cfg.toolkits.font_smoothing = f,
                    None => {
                        log::warn!("[toolkits] font_smoothing = {s:?} is unknown; using grayscale")
                    }
                }
            }
        }
        if let Some(c) = file.cursor {
            if let Some(t) = c.theme {
                cfg.cursor.theme = t;
            }
            if let Some(v) = c.size {
                if v > CURSOR_SIZE_MAX {
                    return Err(format!("cursor.size must be 0 to {CURSOR_SIZE_MAX}"));
                }
                cfg.cursor.size = v;
            }
        }
        if let Some(v) = file.adaptive_sync {
            cfg.adaptive_sync = v;
        }
        if let Some(s) = file.window_snapping {
            if let Some(v) = s.enabled {
                cfg.window_snapping.enabled = v;
            }
            if let Some(v) = s.picker {
                cfg.window_snapping.picker = v;
            }
            if let Some(v) = s.bar {
                cfg.window_snapping.bar = v;
            }
            if let Some(specs) = s.layouts {
                if specs.len() > crate::snap::ZONE_MAX {
                    return Err(format!(
                        "window_snapping: layouts has at most {} entries, got {}",
                        crate::snap::ZONE_MAX,
                        specs.len()
                    ));
                }
                cfg.window_snapping.layouts = (specs.iter())
                    .map(|s| crate::snap::ZoneLayout::parse(s))
                    .collect::<Result<_, _>>()
                    .map_err(|e| format!("window_snapping: layouts: {e}"))?;
            }
        }
        if let Some(v) = file.screenshot.and_then(|s| s.save) {
            cfg.screenshot.save = v;
        }
        if let Some(p) = file.previews {
            if let Some(v) = p.taskbar {
                cfg.previews.taskbar = v;
            }
            if let Some(n) = p.fps {
                if !(1..=PREVIEW_FPS_MAX as i64).contains(&n) {
                    return Err(format!(
                        "previews.fps must be between 1 and {PREVIEW_FPS_MAX}, got {n}"
                    ));
                }
                cfg.previews.fps = n as u32;
            }
        }
        if let Some(s) = file.switcher {
            if let Some(v) = s.style {
                cfg.switcher.style = v.parse()?;
            }
            if let Some(v) = s.size {
                use reprisekit::switcher::Size;
                cfg.switcher.size = match v.as_str() {
                    "small" => Size::Small,
                    "medium" => Size::Medium,
                    "large" => Size::Large,
                    "huge" => Size::Huge,
                    other => {
                        return Err(format!(
                            "switcher.size {other:?} is not \"small\", \"medium\", \"large\" or \"huge\""
                        ));
                    }
                };
            }
            let flags = [
                (s.preview, &mut cfg.switcher.preview),
                (s.desktop, &mut cfg.switcher.desktop),
                (s.peek, &mut cfg.switcher.peek),
                (s.flip, &mut cfg.switcher.flip),
            ];
            for (v, at) in flags {
                if let Some(v) = v {
                    *at = v;
                }
            }
        }
        if let Some(v) = file.overview.and_then(|o| o.hot_corner) {
            cfg.hot_corner = HotCorner::from_name(&v).ok_or_else(|| {
                format!(
                    "overview.hot_corner {v:?} is not \"none\", \"top-left\", \"top-right\", \"bottom-left\" or \"bottom-right\""
                )
            })?;
        }
        if let Some(g) = file.gestures {
            if let Some(v) = g.enabled {
                cfg.gestures.enabled = v;
            }
            if let Some(n) = g.fingers {
                if n != 3 && n != 4 {
                    return Err(format!("gestures.fingers must be 3 or 4, got {n}"));
                }
                cfg.gestures.fingers = n as u32;
            }
            let ranged =
                |key: &str, v: Option<i64>, lo: u32, hi: u32| -> Result<Option<u32>, String> {
                    v.map(|n| {
                        u32::try_from(n)
                            .ok()
                            .filter(|n| (lo..=hi).contains(n))
                            .ok_or_else(|| format!("gestures.{key} must be {lo} to {hi}, got {n}"))
                    })
                    .transpose()
                };
            if let Some(n) = ranged("distance", g.distance, 100, 2000)? {
                cfg.gestures.distance = n;
            }
            if let Some(n) = ranged("commit_at", g.commit_at, 5, 95)? {
                cfg.gestures.commit_at = n;
            }
            if let Some(n) = ranged("flick_speed", g.flick_speed, 50, 5000)? {
                cfg.gestures.flick_speed = n;
            }
        }
        if let Some(c) = file.clipboard {
            if let Some(v) = c.history {
                cfg.clipboard.history = v;
            }
            if let Some(v) = c.terminal_apps {
                cfg.clipboard.terminal_apps = v;
            }
            if let Some(v) = c.trusted_apps {
                cfg.clipboard.trusted_apps = v;
            }
        }
        if let Some(p) = file.privacy {
            if let Some(v) = p.recent_files {
                cfg.recent_files = v;
            }
            if let Some(v) = p.location {
                cfg.location = v;
            }
        }
        if let Some(d) = file.debug {
            if let Some(v) = d.overlay {
                cfg.debug.overlay = v;
            }
            if let Some(v) = d.overlay_in_captures {
                cfg.debug.overlay_in_captures = v;
            }
        }
        if let Some(v) = file.launcher.and_then(|l| l.search_files) {
            cfg.launcher_files = v;
        }
        if let Some(n) = file.night_light {
            n.apply(&mut cfg.night_light)?;
        }
        if let Some(w) = file.workspaces {
            if let Some(n) = w.count {
                cfg.workspaces = usize::try_from(n)
                    .ok()
                    .filter(|n| (1..=WORKSPACES_MAX).contains(n))
                    .ok_or_else(|| {
                        format!("workspaces: count must be between 1 and {WORKSPACES_MAX}, got {n}")
                    })?;
            }
            if let Some(a) = w.animation {
                cfg.workspace_animation.style = SwitchStyle::from_name(&a).ok_or_else(|| {
                    format!(
                        "workspaces.animation {a:?} is not one of {}",
                        SwitchStyle::ALL
                            .iter()
                            .map(|s| format!("{:?}", s.name()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
            }
            if let Some(p) = w.pager {
                use reprisekit::taskbar::PagerMark;
                cfg.pager_mark = PagerMark::from_name(&p).ok_or_else(|| {
                    format!(
                        "workspaces.pager {p:?} is not one of {}",
                        PagerMark::ALL
                            .iter()
                            .map(|m| format!("{:?}", m.name()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
            }
            if let Some(ms) = w.animation_ms {
                cfg.workspace_animation.ms = u32::try_from(ms)
                    .ok()
                    .filter(|ms| *ms <= SWITCH_MS_MAX)
                    .ok_or_else(|| {
                        format!(
                            "workspaces: animation_ms must be between 0 and {SWITCH_MS_MAX}, got {ms}"
                        )
                    })?;
            }
        }
        if let Some(e) = file.effects {
            let ms = |v: Option<i64>, key: &str, at: &mut u32| -> Result<(), String> {
                if let Some(v) = v {
                    *at = u32::try_from(v)
                        .ok()
                        .filter(|v| *v <= EFFECTS_MS_MAX)
                        .ok_or_else(|| {
                            format!(
                                "effects: {key} must be between 0 and {EFFECTS_MS_MAX}, got {v}"
                            )
                        })?;
                }
                Ok(())
            };
            let fx = &mut cfg.effects;
            if let Some(v) = e.window_animation {
                fx.window_animation = v;
            }
            if let Some(v) = e.hover_fade {
                fx.hover_fade = v;
            }
            if let Some(v) = e.taskbar_slide {
                fx.taskbar_slide = v;
            }
            ms(e.window_open_ms, "window_open_ms", &mut fx.window_open_ms)?;
            ms(
                e.window_close_ms,
                "window_close_ms",
                &mut fx.window_close_ms,
            )?;
            ms(e.hover_in_ms, "hover_in_ms", &mut fx.hover_in_ms)?;
            ms(e.hover_out_ms, "hover_out_ms", &mut fx.hover_out_ms)?;
            ms(
                e.taskbar_hide_ms,
                "taskbar_hide_ms",
                &mut fx.taskbar_hide_ms,
            )?;
            if let Some(s) = e.window_scale {
                if !(0.8..=1.0).contains(&s) {
                    return Err(format!(
                        "effects: window_scale must be between 0.8 and 1.0, got {s}"
                    ));
                }
                fx.window_scale = s;
            }
        }
        if let Some(a) = file.accessibility {
            a.apply(&mut cfg.accessibility)?;
        }
        if let Some(w) = file.windows {
            if let Some(v) = w.remember {
                cfg.remember_windows = v;
            }
            if let Some(v) = w.tabs {
                cfg.window_tabs = v;
            }
        }
        if let Some(s) = file.session {
            if let Some(v) = s.restore {
                cfg.session.restore = v;
            }
            if let Some(v) = s.exclude {
                cfg.session.exclude = v;
            }
            if let Some(v) = s.end_processes {
                cfg.session.end_processes = v;
            }
        }
        if let Some(rules) = file.window_rules {
            cfg.window_rules = rules
                .into_iter()
                .enumerate()
                .map(|(i, r)| {
                    let at = format!("window_rules #{} ({:?})", i + 1, r.app_id);
                    r.parse(&at)
                })
                .collect::<Result<_, _>>()?;
        }
        if let Some(pads) = file.tablet_pads {
            cfg.tablet_pads = pads
                .into_iter()
                .enumerate()
                .map(|(i, p)| p.parse(&format!("tablet_pads #{}", i + 1)))
                .collect::<Result<_, _>>()?;
        }
        if let Some(outs) = file.outputs {
            cfg.outputs = outs
                .into_iter()
                .map(|o| {
                    let asked = o.scale.unwrap_or(1.0);
                    if !(1.0..=4.0).contains(&asked) {
                        return Err(format!(
                            "outputs.{}: scale must be between 1 and 4, got {asked}",
                            o.name
                        ));
                    }
                    let scale = crate::output::quarter_scale(asked as f32);
                    if f64::from(scale) != asked {
                        log::warn!(
                            "outputs.{}: scale {asked} is not a multiple of 0.25; using {scale}",
                            o.name
                        );
                    }
                    let mode = o
                        .mode
                        .as_deref()
                        .map(OutputMode::parse)
                        .transpose()
                        .map_err(|e| format!("outputs.{}: {e}", o.name))?;
                    let transform = match o.transform.as_deref() {
                        None => Transform::Normal,
                        Some(s) => Transform::parse(s).ok_or_else(|| {
                            format!(
                                "outputs.{}: transform {s:?} is not one of {}",
                                o.name,
                                Transform::ALL.map(Transform::name).join(", ")
                            )
                        })?,
                    };
                    if o.color_profile.is_some() {
                        cfg.retired
                            .push(format!("outputs.{}.color_profile", o.name));
                    }
                    if o.hdr.is_some() {
                        cfg.retired.push(format!("outputs.{}.hdr", o.name));
                    }
                    let position = o
                        .position
                        .map(|[x, y]| {
                            let max = i64::from(crate::outputmgmt::MAX_POSITION);
                            if (-max..=max).contains(&x) && (-max..=max).contains(&y) {
                                Ok((x as i32, y as i32))
                            } else {
                                Err(format!(
                                    "outputs.{}: position must be within {max} of 0, got [{x}, {y}]",
                                    o.name
                                ))
                            }
                        })
                        .transpose()?;
                    Ok(OutputConfig {
                        name: o.name,
                        scale,
                        mode,
                        adaptive_sync: o.adaptive_sync,
                        transform,
                        position,
                        primary: o.primary.unwrap_or(false),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
        }
        if let Some(k) = file.keyboard {
            if let Some(v) = k.rules {
                cfg.keyboard.rules = v;
            }
            if let Some(v) = k.model {
                cfg.keyboard.model = v;
            }
            if let Some(v) = k.layout {
                cfg.keyboard.layout = v;
            }
            if let Some(v) = k.variant {
                cfg.keyboard.variant = v;
            }
            if let Some(v) = k.options {
                cfg.keyboard.options = v;
            }
            if let Some(v) = k.repeat_rate {
                if v < 0 {
                    return Err("keyboard.repeat_rate must be >= 0".into());
                }
                cfg.keyboard.repeat_rate = v;
            }
            if let Some(v) = k.repeat_delay {
                if v < 0 {
                    return Err("keyboard.repeat_delay must be >= 0".into());
                }
                cfg.keyboard.repeat_delay = v;
            }
        }
        if let Some(m) = file.mouse {
            apply_pointer(
                &mut cfg.mouse,
                "mouse",
                m.speed,
                m.acceleration,
                m.natural_scroll,
            )?;
        }
        if let Some(t) = file.touchpad {
            apply_pointer(
                &mut cfg.touchpad.pointer,
                "touchpad",
                t.speed,
                t.acceleration,
                t.natural_scroll,
            )?;
            if let Some(v) = t.tap_to_click {
                cfg.touchpad.tap_to_click = v;
            }
            if let Some(v) = t.two_finger_scroll {
                cfg.touchpad.two_finger_scroll = v;
            }
            if let Some(v) = t.two_finger_tap_right_click {
                cfg.touchpad.two_finger_tap_right_click = v;
            }
            if let Some(v) = t.disable_while_typing {
                cfg.touchpad.disable_while_typing = v;
            }
        }
        if let Some(t) = file.touchscreen {
            cfg.touchscreen.output = t.output.filter(|o| !o.is_empty());
        }
        if let Some(p) = file.power {
            if let Some(v) = p.standby {
                cfg.power.standby = v;
            }
            if let Some(v) = p.poweroff {
                cfg.power.poweroff = v;
            }
            if let Some(v) = p.reboot {
                cfg.power.reboot = v;
            }
            if let Some(v) = p.hibernate {
                cfg.power.hibernate = v;
            }
            let action = |name: &str, v: Option<String>| -> Result<Option<PowerAction>, String> {
                v.map(|s| PowerAction::parse(&s).map_err(|e| format!("power.{name}: {e}")))
                    .transpose()
            };
            if let Some(a) = action("lid_close", p.lid_close)? {
                cfg.power.lid_close = a;
            }
            if let Some(a) = action("power_button", p.power_button)? {
                cfg.power.power_button = a;
            }
            if let Some(a) = action("sleep_button", p.sleep_button)? {
                cfg.power.sleep_button = a;
            }
            if let Some(v) = p.lock_on_sleep {
                cfg.power.lock_on_sleep = v;
            }
            if let Some(v) = p.locker {
                cfg.power.locker = v;
            }
            if let Some(a) = action("critical_action", p.critical_action)? {
                if matches!(
                    a,
                    PowerAction::Ask | PowerAction::ScreenOff | PowerAction::Lock
                ) {
                    return Err(format!(
                        "power.critical_action cannot be {:?}: nothing, standby, hibernate or turnoff",
                        a.name()
                    ));
                }
                cfg.power.critical_action = a;
            }
            let percent = |name: &str, v: Option<i64>| -> Result<Option<u32>, String> {
                match v {
                    None => Ok(None),
                    Some(s) if (0..=100).contains(&s) => Ok(Some(s as u32)),
                    Some(s) => Err(format!("power.{name} must be 0..=100, got {s}")),
                }
            };
            if let Some(v) = percent("battery_low", p.battery_low)? {
                cfg.power.battery_low = v;
            }
            if let Some(v) = percent("battery_critical", p.battery_critical)? {
                cfg.power.battery_critical = v;
            }
            if let Some(v) = p.always_show_icon {
                cfg.power.always_show_icon = v;
            }
        }
        if let Some(i) = file.idle {
            let seconds = |name: &str, v: Option<i64>| -> Result<Option<u32>, String> {
                v.map(|s| {
                    u32::try_from(s)
                        .map_err(|_| format!("idle.{name} must be 0..={}, got {s}", u32::MAX))
                })
                .transpose()
            };
            if let Some(v) = seconds("lock_after", i.lock_after)? {
                cfg.idle.lock_after = v;
            }
            if let Some(v) = seconds("screen_off_after", i.screen_off_after)? {
                cfg.idle.screen_off_after = v;
            }
            if let Some(v) = seconds("blank_after", i.blank_after)? {
                cfg.idle.blank_after = v;
            }
            if let Some(v) = seconds("suspend_after", i.suspend_after)? {
                cfg.idle.suspend_after = v;
            }
            if let Some(v) = seconds("lock_fade", i.lock_fade)? {
                cfg.idle.lock_fade = v;
            }
            if let Some(b) = i.battery {
                cfg.idle_battery = IdleOverrides {
                    lock_after: seconds("battery.lock_after", b.lock_after)?,
                    screen_off_after: seconds("battery.screen_off_after", b.screen_off_after)?,
                    blank_after: seconds("battery.blank_after", b.blank_after)?,
                    suspend_after: seconds("battery.suspend_after", b.suspend_after)?,
                };
            }
        }
        // Last: the text size scales the theme `[metrics]` and `[fonts]` made.
        cfg.base_theme = cfg.theme.clone();
        cfg.theme = cfg.theme.with_text_size(cfg.accessibility.text_size);
        Ok(cfg)
    }

    /// Load for startup. A top-level key that does not parse is dropped to its
    /// default and a non-TOML file yields all defaults. Returns a notice when anything was dropped.
    pub fn load(path: Option<&Path>) -> (Config, Option<String>) {
        let path = path
            .map(Path::to_path_buf)
            .unwrap_or_else(Config::default_path);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                log::info!("no config at {}; using defaults", path.display());
                return (Config::default(), None);
            }
            Err(e) => {
                log::error!("config {}: {e}; using defaults", path.display());
                return (Config::default(), Some(UNREADABLE_NOTICE.to_string()));
            }
        };
        match Config::parse_lenient(&text) {
            Ok((c, dropped)) if dropped.is_empty() => {
                log::info!("loaded config {}", path.display());
                (c, None)
            }
            Ok((c, dropped)) => {
                for (key, e) in &dropped {
                    log::warn!("config {}: {key}: {e}; using its defaults", path.display());
                }
                let keys: Vec<&str> = dropped.iter().map(|(k, _)| k.as_str()).collect();
                let notice = format!(
                    "Reprise ignored {} in config.toml and used the defaults there. \
                     The session log names the errors.",
                    keys.join(", ")
                );
                (c, Some(notice))
            }
            Err(e) => {
                log::error!("config {}: {e}; using defaults", path.display());
                (Config::default(), Some(UNREADABLE_NOTICE.to_string()))
            }
        }
    }

    pub fn warn_retired(&self) {
        if !self.retired.is_empty() {
            log::warn!(
                "config: {} no longer has an effect; Reprise renders with OpenGL ES \
                 and drives no HDR or colour profile",
                self.retired.join(", ")
            );
        }
    }

    pub fn scale_for_output_name(&self, name: &str) -> f32 {
        self.outputs
            .iter()
            .find(|o| o.name == name)
            .map(|o| o.scale)
            .unwrap_or(1.0)
    }

    pub fn mode_for_output_name(&self, name: &str) -> Option<OutputMode> {
        self.outputs
            .iter()
            .find(|o| o.name == name)
            .and_then(|o| o.mode)
    }

    pub fn adaptive_sync_for_output_name(&self, name: &str) -> bool {
        self.outputs
            .iter()
            .find(|o| o.name == name)
            .and_then(|o| o.adaptive_sync)
            .unwrap_or(self.adaptive_sync)
    }

    pub fn position_for_output_name(&self, name: &str) -> Option<(i32, i32)> {
        self.outputs
            .iter()
            .find(|o| o.name == name)
            .and_then(|o| o.position)
    }

    pub fn primary_output_name(&self) -> Option<&str> {
        self.outputs
            .iter()
            .find(|o| o.primary)
            .map(|o| o.name.as_str())
    }

    pub fn transform_for_output_name(&self, name: &str) -> Transform {
        self.outputs
            .iter()
            .find(|o| o.name == name)
            .map(|o| o.transform)
            .unwrap_or_default()
    }
}

impl Config {
    pub fn idle_for(&self, on_battery: bool) -> IdleConfig {
        if on_battery {
            self.idle_battery.apply(&self.idle)
        } else {
            self.idle.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn global_shortcuts_parse_with_their_chord() {
        let cfg = Config::parse(
            "[[global_shortcuts]]\napp = \"hu.taltos.Taltos\"\nid = \"talk\"\ndescription = \"Hold to talk\"\nkeys = \"Pause\"\n\
             [[global_shortcuts]]\napp = \"hu.taltos.Taltos\"\nid = \"mute\"\nkeys = \"\"\n",
        )
        .unwrap();
        let g = &cfg.global_shortcuts;
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].chord, Some(parse_keybind("Pause").unwrap()));
        assert_eq!(g[0].description, "Hold to talk");
        assert_eq!(g[1].chord, None, "empty keys are unassigned");
        assert_eq!(g[1].description, "", "description is optional");
    }

    #[test]
    fn a_global_shortcut_with_bad_keys_is_unassigned_and_the_file_still_loads() {
        let cfg = Config::parse(
            "[[global_shortcuts]]\napp = \"a\"\nid = \"x\"\nkeys = \"Ctrl+Nope\"\n\
             [[global_shortcuts]]\napp = \"a\"\nid = \"y\"\nkeys = \"Shift_L\"\n",
        )
        .unwrap();
        assert_eq!(cfg.global_shortcuts[0].chord, None);
        assert_eq!(
            cfg.global_shortcuts[0].keys, "Ctrl+Nope",
            "the text is kept"
        );
        assert_eq!(
            cfg.global_shortcuts[1].chord, None,
            "a modifier-only chord takes no key"
        );
    }

    #[test]
    fn a_repeated_global_shortcut_keeps_the_first() {
        let cfg = Config::parse(
            "[[global_shortcuts]]\napp = \"a\"\nid = \"x\"\nkeys = \"F9\"\n\
             [[global_shortcuts]]\napp = \"a\"\nid = \"x\"\nkeys = \"F10\"\n",
        )
        .unwrap();
        assert_eq!(cfg.global_shortcuts.len(), 1);
        assert_eq!(cfg.global_shortcuts[0].keys, "F9");
    }

    #[test]
    fn the_portal_table_belongs_to_the_portal() {
        assert!(Config::parse("[portal]\nshortcuts_denied = [\"com.example.App\"]\n").is_ok());
    }

    #[test]
    fn the_thumbnail_cache_table_parses() {
        assert!(Config::parse("[thumbnail_cache]\nenabled = false\nmax_mb = 64\n").is_ok());
    }

    use super::*;
    use crate::keys::*;
    use crate::theme::Color;

    #[test]
    fn color_scheme_picks_classic_colours() {
        let want = reprisekit::schemes::scheme("Brick").expect("Brick is a Classic scheme");
        let c = Config::parse("[appearance]\ncolor_scheme = \"Brick\"\n").unwrap();
        assert_eq!(c.theme.colors, want);
    }

    #[test]
    fn colors_table_applies_over_the_scheme() {
        let c = Config::parse(
            "[appearance]\ncolor_scheme = \"Brick\"\n[colors]\nbackground = \"#102030\"\n",
        )
        .unwrap();
        assert_eq!(c.theme.colors.background, Color::rgb(0x10, 0x20, 0x30));
    }

    #[test]
    fn color_scheme_picks_the_blue_variant() {
        let c =
            Config::parse("[appearance]\ninterface = \"blue\"\ncolor_scheme = \"Olive Green\"\n")
                .unwrap();
        assert_eq!(c.blue_variant, reprisekit::blue::Variant::Olive);
        let c = Config::parse("[appearance]\ninterface = \"blue\"\ncolor_scheme = \"Obsidian\"\n")
            .unwrap();
        assert_eq!(c.blue_variant, reprisekit::blue::Variant::Obsidian);
    }

    #[test]
    fn a_scheme_of_another_interface_falls_back() {
        let blue = Config::parse("[appearance]\ninterface = \"blue\"\ncolor_scheme = \"Brick\"\n")
            .unwrap();
        assert_eq!(blue.blue_variant, reprisekit::blue::Variant::Blue);
        let classic = Config::parse("[appearance]\ncolor_scheme = \"Olive Green\"\n").unwrap();
        assert_eq!(classic.theme.colors, Config::default().theme.colors);
    }

    #[test]
    fn interface_defaults_to_classic_and_parses() {
        assert_eq!(Config::parse("").unwrap().interface, Interface::Classic);
        let c = Config::parse("[appearance]\ninterface = \"classic\"\n").unwrap();
        assert_eq!(c.interface, Interface::Classic);
    }

    #[test]
    fn blue_interface_rebases_metrics_and_colors() {
        let c = Config::parse("[appearance]\ninterface = \"blue\"\n").unwrap();
        assert_eq!(c.interface, Interface::Blue);
        assert_eq!(c.theme.metrics.caption_height, 26);
        assert_eq!(
            c.theme.colors.background,
            reprisekit::blue::Variant::Blue.scheme().system.background
        );
        assert!(matches!(c.theme.look, reprisekit::look::Look::Styled(_)));
    }

    #[test]
    fn user_metrics_apply_over_the_blue_base() {
        let c = Config::parse(
            "[appearance]\ninterface = \"blue\"\n[metrics]\nframe_width = 6\n[colors]\nbackground = \"#102030\"\n",
        )
        .unwrap();
        assert_eq!(c.theme.metrics.frame_width, 6);
        assert_eq!(c.theme.metrics.caption_height, 26);
        assert_eq!(c.theme.colors.background, Color::rgb(0x10, 0x20, 0x30));
    }

    #[test]
    fn classic_config_theme_is_unchanged() {
        assert_eq!(Config::parse("").unwrap().theme, Theme::default());
    }

    #[test]
    fn unknown_interface_falls_back_to_classic() {
        let c = Config::parse("[appearance]\ninterface = \"frosted\"\n").unwrap();
        assert_eq!(c.interface, Interface::Classic);
    }

    #[test]
    fn glass_material_and_frame_parse_their_names() {
        use reprisekit::glass::{Frame, Material};
        let c = Config::parse("").unwrap();
        assert_eq!(
            (c.glass.material, c.glass.frame),
            (Material::Frosted, Frame::Rounded)
        );
        let c = Config::parse("[glass]\nmaterial = \"smoked\"\nframe = \"square\"\n").unwrap();
        assert_eq!(
            (c.glass.material, c.glass.frame),
            (Material::Smoked, Frame::Square)
        );
        assert_eq!(
            Config::parse("[glass]\nmaterial = \"molten\"\n")
                .unwrap()
                .glass
                .material,
            Material::Molten
        );
        assert!(Config::parse("[glass]\nmaterial = \"glassy\"\n").is_err());
        assert!(Config::parse("[glass]\nframe = \"round\"\n").is_err());
    }

    #[test]
    fn glass_mode_parses_light_and_dark() {
        use reprisekit::glass::Mode;
        assert_eq!(Config::parse("").unwrap().glass.mode, Mode::Light);
        assert_eq!(
            Config::parse("[glass]\nmode = \"dark\"\n")
                .unwrap()
                .glass
                .mode,
            Mode::Dark
        );
        assert_eq!(
            Config::parse("[glass]\nmode = \"light\"\n")
                .unwrap()
                .glass
                .mode,
            Mode::Light
        );
        assert!(
            Config::parse("[glass]\nmode = \"dim\"\n").is_err(),
            "a mode other than light or dark is an error"
        );
    }

    #[test]
    fn glass_interface_and_settings_parse() {
        let c = Config::parse(
            "[appearance]\ninterface = \"glass\"\n[glass]\ncolor = \"#102030\"\nintensity = 80\ntransparency = false\n",
        )
        .unwrap();
        assert_eq!(c.interface, Interface::Glass);
        assert_eq!(c.glass.intensity, 80);
        assert!(!c.glass.transparency);
        assert_eq!(c.theme.metrics.frame_width, 8);
        let d = Config::parse("[glass]\ncolor = \"sky\"\n").unwrap();
        assert_eq!(d.glass, reprisekit::glass::Settings::default());
        assert!(
            d.glass.fast_drag,
            "the faster glass while dragging is on by default"
        );
        let e = Config::parse("[glass]\nfast_drag = false\n").unwrap();
        assert!(!e.glass.fast_drag);
        assert!(d.glass.blur_frames, "frames behind glass show by default");
        let f = Config::parse("[glass]\nblur_frames = false\n").unwrap();
        assert!(!f.glass.blur_frames);
        assert_eq!(
            d.glass.blur_rate, 30,
            "client content re-blurs 30 times a second"
        );
        let r = Config::parse("[glass]\nblur_rate = 0\n").unwrap();
        assert_eq!(r.glass.blur_rate, 0);
        assert!(
            Config::parse("[glass]\nblur_rate = -1\n").is_err(),
            "the rate is not negative"
        );
        assert!(
            Config::parse("[glass]\nintensity = 101\n").is_err(),
            "intensity is 0..100"
        );
        assert_eq!(d.glass.orb_glow, 100, "the orb glows fully by default");
        for o in [0, 50, 100] {
            let c = Config::parse(&format!("[glass]\norb_glow = {o}\n")).unwrap();
            assert_eq!(c.glass.orb_glow, o);
        }
        for bad in [-1, 101] {
            assert!(
                Config::parse(&format!("[glass]\norb_glow = {bad}\n")).is_err(),
                "orb_glow is 0..100, not {bad}"
            );
        }
        assert_eq!((d.glass.blur, d.glass.streaks), (2, true));
        let b = Config::parse("[glass]\nblur = 5\nstreaks = false\n").unwrap();
        assert_eq!((b.glass.blur, b.glass.streaks), (5, false));
        for bad in [0, 6] {
            assert!(
                Config::parse(&format!("[glass]\nblur = {bad}\n")).is_err(),
                "blur is 1..5, not {bad}"
            );
        }
        assert!(
            Config::parse("[glass]\ncolor = \"mauve-ish\"\n").is_err(),
            "unknown colour name"
        );
    }

    #[test]
    fn side_width_takes_its_key_and_refuses_a_negative() {
        for i in ["classic", "blue", "glass"] {
            let head = format!("[appearance]\ninterface = \"{i}\"\n");
            assert_eq!(
                Config::parse(&head).unwrap().taskbar.side_width,
                0,
                "{i}: the look's default"
            );
        }
        let c = Config::parse("[taskbar]\nside_width = 120\n").unwrap();
        assert_eq!(c.taskbar.side_width, 120);
        let c = Config::parse("[taskbar]\nside_width = 5000\n").unwrap();
        assert_eq!(
            c.taskbar.side_width, 5000,
            "clamped where it is used, not here"
        );
        for w in ["-1", "2147483648"] {
            let err = Config::parse(&format!("[taskbar]\nside_width = {w}\n")).unwrap_err();
            assert_eq!(
                err,
                format!("[taskbar] side_width must be between 0 and 2147483647, got {w}")
            );
        }
        assert!(Config::parse("[taskbar]\nside_width = \"wide\"\n").is_err());
    }

    #[test]
    fn tray_overflow_is_on_by_default_in_every_look() {
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            assert!(TaskbarConfig::default_for(i).tray_overflow, "{i:?}");
        }
    }

    #[test]
    fn taskbar_defaults_follow_the_interface() {
        let classic = Config::parse("").unwrap().taskbar;
        assert_eq!(classic, TaskbarConfig::default_for(Interface::Classic));
        assert_eq!(classic.buttons, TaskbarButtons::WhenFull);
        assert!(
            !classic.pinned
                && !classic.small_icons
                && !classic.show_desktop_button
                && !classic.peek,
            "Classic turns every taskbar extra off but the tray arrow"
        );
        assert!(classic.tray_overflow);
        let blue = Config::parse("[appearance]\ninterface = \"blue\"\n").unwrap();
        assert_eq!(blue.taskbar, classic, "Blue keeps Classic's taskbar");
        let glass = Config::parse("[appearance]\ninterface = \"glass\"\n")
            .unwrap()
            .taskbar;
        assert_eq!(
            glass,
            TaskbarConfig {
                position: reprisekit::taskbar::Edge::Bottom,
                buttons: TaskbarButtons::Combine,
                pinned: true,
                small_icons: false,
                show_desktop_button: true,
                peek: true,
                tray_overflow: true,
                auto_hide: false,
                reveal_on_edge: true,
                reveal_on_keys: true,
                reveal_on_attention: true,
                side_width: 0,
            }
        );
        assert_eq!(glass, TaskbarConfig::default_for(Interface::Glass));
    }

    #[test]
    fn taskbar_keys_override_the_interface_defaults() {
        let c = Config::parse(
            "[appearance]\ninterface = \"glass\"\n[taskbar]\nbuttons = \"never\"\npeek = false\nsmall_icons = true\n",
        )
        .unwrap();
        assert_eq!(c.taskbar.buttons, TaskbarButtons::Never);
        assert!(!c.taskbar.peek);
        assert!(c.taskbar.small_icons);
        assert!(c.taskbar.pinned, "an absent key keeps Glass's default");
        let c = Config::parse("[taskbar]\nbuttons = \"combine\"\ntray_overflow = true\n").unwrap();
        assert_eq!(c.taskbar.buttons, TaskbarButtons::Combine);
        assert!(c.taskbar.tray_overflow);
        assert!(!c.taskbar.pinned && !c.taskbar.peek);
        let w = Config::parse("[taskbar]\nbuttons = \"when_full\"\n").unwrap();
        assert_eq!(w.taskbar.buttons, TaskbarButtons::WhenFull);
        let g = Config::parse("[taskbar]\nbuttons = \"group\"\n").unwrap();
        assert_eq!(g.taskbar.buttons, TaskbarButtons::Group);
    }

    #[test]
    fn the_taskbar_sits_on_any_edge_position_names() {
        use reprisekit::taskbar::Edge;
        for i in ["classic", "blue", "glass"] {
            let head = format!("[appearance]\ninterface = \"{i}\"\n");
            let c = Config::parse(&head).unwrap();
            assert_eq!(
                c.taskbar.position,
                Edge::Bottom,
                "{i} puts the bar at the bottom"
            );
            let c = Config::parse(&format!("{head}[taskbar]\nposition = \"top\"\n")).unwrap();
            assert_eq!(c.taskbar.position, Edge::Top, "{i} takes the key");
        }
        let c = Config::parse("[taskbar]\nposition = \"bottom\"\n").unwrap();
        assert_eq!(c.taskbar.position, Edge::Bottom);
        let err = Config::parse("[taskbar]\nposition = \"middle\"\n").unwrap_err();
        assert!(
            err.contains("\"middle\""),
            "the error names the value: {err}"
        );
        for edge in ["bottom", "top", "left", "right"] {
            assert!(err.contains(edge), "the error names {edge}: {err}");
        }
        assert!(Config::parse("[taskbar]\nposition = 1\n").is_err());
        for (name, edge) in [("left", Edge::Left), ("right", Edge::Right)] {
            let c = Config::parse(&format!("[taskbar]\nposition = \"{name}\"\n")).unwrap();
            assert_eq!(c.taskbar.position, edge, "{name}");
        }
    }

    #[test]
    fn auto_hide_is_off_and_its_triggers_on_under_every_interface() {
        for i in ["classic", "blue", "glass"] {
            let c = Config::parse(&format!("[appearance]\ninterface = \"{i}\"\n")).unwrap();
            let t = &c.taskbar;
            assert!(!t.auto_hide, "{i} keeps the bar up");
            assert!(
                t.reveal_on_edge && t.reveal_on_keys && t.reveal_on_attention,
                "{i} turns every trigger on"
            );
            assert_eq!(c.effects.taskbar_hide_ms, 200, "{i} slides it over 200 ms");
        }
    }

    #[test]
    fn auto_hide_and_its_triggers_take_their_keys() {
        let c = Config::parse(
            "[taskbar]\nauto_hide = true\nreveal_on_edge = false\nreveal_on_keys = false\n\
             reveal_on_attention = false\n[effects]\ntaskbar_hide_ms = 0\n",
        )
        .unwrap();
        assert!(c.taskbar.auto_hide);
        assert!(!c.taskbar.reveal_on_edge && !c.taskbar.reveal_on_keys);
        assert!(!c.taskbar.reveal_on_attention);
        assert_eq!(c.effects.taskbar_hide_ms, 0, "0 hides and shows at once");
        assert!(Config::parse("[taskbar]\nauto_hide = \"yes\"\n").is_err());
        let err = Config::parse("[effects]\ntaskbar_hide_ms = 2001\n").unwrap_err();
        assert!(
            err.contains("taskbar_hide_ms"),
            "the error names the key: {err}"
        );
        assert!(Config::parse("[effects]\ntaskbar_hide_ms = -1\n").is_err());
    }

    #[test]
    fn small_icons_shrinks_the_glass_bar_and_the_usable_area() {
        let full = |b: bool| {
            Config::parse(&format!(
                "[appearance]\ninterface = \"glass\"\n[taskbar]\nsmall_icons = {b}\n"
            ))
            .unwrap()
            .theme
            .metrics
            .taskbar_height
        };
        assert_eq!(full(false), 40, "Glass's own bar height, unshrunk");
        assert_eq!(full(true), reprisekit::glass::SMALL_TASKBAR_HEIGHT);

        let screen = reprisekit::raster::Rect::new(0, 0, 1920, 1080);
        let usable = |h| {
            let p = crate::barplace::place(screen, reprisekit::taskbar::Edge::Bottom, h);
            crate::barplace::usable(screen, &p)
        };
        let usable_off = usable(full(false));
        let usable_on = usable(full(true));
        assert_eq!(usable_off.height, 1040);
        assert_eq!(usable_on.height, 1050, "10 px taller, the bar's own shrink");

        let classic = |b: bool| {
            Config::parse(&format!("[taskbar]\nsmall_icons = {b}\n"))
                .unwrap()
                .theme
                .metrics
                .taskbar_height
        };
        assert_eq!(classic(false), classic(true), "Classic ignores the key");
        let blue = |b: bool| {
            Config::parse(&format!(
                "[appearance]\ninterface = \"blue\"\n[taskbar]\nsmall_icons = {b}\n"
            ))
            .unwrap()
            .theme
            .metrics
            .taskbar_height
        };
        assert_eq!(blue(false), blue(true), "Blue ignores the key");

        let c = Config::parse(
            "[appearance]\ninterface = \"glass\"\n[taskbar]\nsmall_icons = true\n[metrics]\ntaskbar_height = 36\n",
        )
        .unwrap();
        assert_eq!(c.theme.metrics.taskbar_height, 36);
    }

    #[test]
    fn small_icons_under_glass_take_the_small_bars_style() {
        let glass = |b: bool| {
            Config::parse(&format!(
                "[appearance]\ninterface = \"glass\"\n[taskbar]\nsmall_icons = {b}\n"
            ))
            .unwrap()
        };
        let (full, small) = (glass(false), glass(true));
        let settings = &full.glass;
        let style = |c: &Config| match &c.theme.look {
            reprisekit::look::Look::Styled(s) => s.clone(),
            reprisekit::look::Look::Classic => panic!("Glass is a styled look"),
        };
        // A shared, capped cache may rebuild either style in between, so compare
        // pixels rather than pointers.
        let pixels = |s: &std::sync::Arc<reprisekit::style::Style>| {
            s.part(reprisekit::style::Part::Caption)
                .unwrap()
                .atlas
                .pixels
                .clone()
        };
        assert_eq!(
            pixels(&style(&full)),
            pixels(&reprisekit::glass::style(settings))
        );
        assert_eq!(
            pixels(&style(&small)),
            pixels(&reprisekit::glass::small_bar_style(settings)),
            "small icons take the small bar's style"
        );
    }

    #[test]
    fn taskbar_rejects_unknown_buttons_and_keys() {
        assert!(Config::parse("[taskbar]\nbuttons = \"sometimes\"\n").is_err());
        assert!(Config::parse("[taskbar]\nbuttons = true\n").is_err());
        assert!(
            Config::parse("[taskbar]\nautohide = true\n").is_err(),
            "the key is auto_hide"
        );
    }

    #[test]
    fn appearance_rejects_unknown_keys() {
        assert!(Config::parse("[appearance]\nfoo = 1\n").is_err());
    }

    #[test]
    fn idle_section_parses_and_defaults() {
        let c = Config::parse("[idle]\nlock_after = 10\n").unwrap();
        assert_eq!(c.idle.lock_after, 10);
        assert_eq!(c.idle.screen_off_after, 900);
        let c =
            Config::parse("[idle]\nlock_after = 0\nscreen_off_after = 0\nlock_fade = 3\n").unwrap();
        assert_eq!(
            c.idle,
            IdleConfig {
                lock_after: 0,
                screen_off_after: 0,
                blank_after: 0,
                suspend_after: 0,
                lock_fade: 3,
            }
        );
        assert!(Config::parse("[idle]\nlock_after = -1\n").is_err());
        assert_eq!(
            Config::parse("[idle]\nlock_after = 4294967296\n").unwrap_err(),
            "idle.lock_after must be 0..=4294967295, got 4294967296"
        );
        assert!(Config::parse("[idle]\nbogus = 1\n").is_err());
        assert_eq!(Config::default().idle, IdleConfig::default());
    }

    #[test]
    fn idle_battery_overrides_fall_back_to_idle() {
        let c =
            Config::parse("[idle]\nscreen_off_after = 100\n[idle.battery]\nsuspend_after = 5\n")
                .unwrap();
        assert_eq!(
            c.idle_for(true),
            IdleConfig {
                lock_after: 600,
                screen_off_after: 100,
                blank_after: 0,
                suspend_after: 5,
                lock_fade: 0,
            }
        );
        assert_eq!(c.idle_for(false).suspend_after, 0);
        assert_eq!(c.idle_for(false).screen_off_after, 100);
        assert!(Config::parse("[idle.battery]\nsuspend_after = -1\n").is_err());
        assert!(Config::parse("[idle.battery]\nbogus = 1\n").is_err());
        assert_eq!(Config::default().idle_for(true), IdleConfig::default());
    }

    #[test]
    fn power_actions_parse_by_name() {
        for (name, action) in PowerAction::NAMES {
            assert_eq!(PowerAction::parse(name), Ok(action));
            assert_eq!(action.name(), name);
        }
        assert!(PowerAction::parse("bogus").is_err());
    }

    #[test]
    fn power_keys_parse() {
        let c = Config::parse(
            "[power]\nhibernate = \"h\"\nlid_close = \"screen_off\"\npower_button = \"nothing\"\n\
             sleep_button = \"hibernate\"\nbattery_low = 20\nbattery_critical = 5\n\
             critical_action = \"turnoff\"\nalways_show_icon = true\n",
        )
        .unwrap();
        assert_eq!(c.power.hibernate, "h");
        assert_eq!(c.power.lid_close, PowerAction::ScreenOff);
        assert_eq!(c.power.power_button, PowerAction::Nothing);
        assert_eq!(c.power.sleep_button, PowerAction::Hibernate);
        assert_eq!(c.power.battery_low, 20);
        assert_eq!(c.power.battery_critical, 5);
        assert_eq!(c.power.critical_action, PowerAction::TurnOff);
        assert!(c.power.always_show_icon);
        assert!(Config::parse("[power]\nlid_close = \"bogus\"\n").is_err());
        assert!(Config::parse("[power]\ncritical_action = \"ask\"\n").is_err());
        assert!(Config::parse("[power]\nbattery_low = 101\n").is_err());
        let d = Config::default().power;
        assert_eq!(d.power_button, PowerAction::Ask);
        assert_eq!(d.lid_close, PowerAction::Standby);
        assert!(d.lock_on_sleep);
        let c = Config::parse("[power]\nlock_on_sleep = false\n").unwrap();
        assert!(!c.power.lock_on_sleep);
        assert_eq!(d.locker, "");
        let c = Config::parse("[power]\nlocker = \"swaylock -f\"\n").unwrap();
        assert_eq!(c.power.locker, "swaylock -f");
    }

    #[test]
    fn icons_section_defaults_and_parses() {
        assert_eq!(Config::default().icons.theme, "hicolor");
        let c = Config::parse("[icons]\ntheme = \"Adwaita\"\n").unwrap();
        assert_eq!(c.icons.theme, "Adwaita");
        assert!(Config::parse("[icons]\nnope = 1\n").is_err());
    }

    #[test]
    fn toolkits_default_to_auto_and_resolve_against_the_look() {
        assert_eq!(Config::default().toolkits, ToolkitsConfig::default());
        let auto = &Config::default().toolkits;
        assert_eq!(resolve_gtk_theme(&auto.gtk_theme, false, None), "Adwaita");
        assert_eq!(
            resolve_gtk_theme(&auto.gtk_theme, true, None),
            "Adwaita-dark"
        );
        assert_eq!(resolve_qt(&auto.qt_platform_theme, false, None), None);
        assert_eq!(
            resolve_qt(&auto.qt_platform_theme, true, None),
            Some("xdgdesktopportal")
        );
        let c = Config::parse("[toolkits]\ngtk_theme = \"Arc\"\nqt_platform_theme = \"qt6ct\"\n")
            .unwrap();
        assert_eq!(resolve_gtk_theme(&c.toolkits.gtk_theme, true, None), "Arc");
        assert_eq!(
            resolve_qt(&c.toolkits.qt_platform_theme, false, None),
            Some("qt6ct")
        );
        assert!(Config::parse("[toolkits]\nnope = 1\n").is_err());
    }

    #[test]
    fn reprise_resolves_to_what_was_generated_else_as_auto() {
        let c =
            Config::parse("[toolkits]\ngtk_theme = \"reprise\"\nqt_platform_theme = \"reprise\"\n")
                .unwrap();
        let (gtk, qt) = (&c.toolkits.gtk_theme, &c.toolkits.qt_platform_theme);
        assert_eq!(
            resolve_gtk_theme(gtk, true, Some("Reprise-alt")),
            "Reprise-alt"
        );
        assert_eq!(resolve_gtk_theme(gtk, true, None), "Adwaita-dark");
        assert_eq!(resolve_gtk_theme(gtk, false, None), "Adwaita");
        assert_eq!(resolve_qt(qt, false, Some("qt6ct")), Some("qt6ct"));
        assert_eq!(resolve_qt(qt, true, None), Some("xdgdesktopportal"));
        assert_eq!(resolve_qt(qt, false, None), None);
        assert_eq!(
            resolve_gtk_theme("Arc", false, Some("Reprise")),
            "Arc",
            "a named theme ignores the generated one"
        );
    }

    #[test]
    fn font_smoothing_defaults_to_grayscale_and_an_unknown_value_keeps_it() {
        assert_eq!(
            Config::default().toolkits.font_smoothing,
            FontSmoothing::Grayscale
        );
        for (s, f) in [
            ("grayscale", FontSmoothing::Grayscale),
            ("subpixel", FontSmoothing::Subpixel),
            ("none", FontSmoothing::None),
            ("crisp", FontSmoothing::Grayscale),
        ] {
            let c = Config::parse(&format!("[toolkits]\nfont_smoothing = \"{s}\"\n")).unwrap();
            assert_eq!(c.toolkits.font_smoothing, f, "{s}");
        }
    }

    #[test]
    fn cursor_section_parses_and_bounds_the_size() {
        let c = Config::parse("[cursor]\ntheme = \"Adwaita\"\nsize = 32\n").unwrap();
        assert_eq!(
            c.cursor,
            CursorConfig {
                theme: "Adwaita".into(),
                size: 32
            }
        );
        assert!(Config::parse("[cursor]\nsize = 257\n").is_err());
        assert!(Config::parse("[cursor]\nnope = 1\n").is_err());
    }

    #[test]
    fn an_unset_cursor_falls_back_to_the_session_then_to_default_24() {
        let none = |_: &str| None;
        assert_eq!(
            CursorConfig::default().resolve(none),
            ("default".to_string(), 24)
        );
        let session = |k: &str| match k {
            "XCURSOR_THEME" => Some("breeze_cursors".to_string()),
            "XCURSOR_SIZE" => Some("48".to_string()),
            _ => None,
        };
        assert_eq!(
            CursorConfig::default().resolve(session),
            ("breeze_cursors".to_string(), 48)
        );
        let set = CursorConfig {
            theme: "Adwaita".into(),
            size: 32,
        };
        assert_eq!(set.resolve(session), ("Adwaita".to_string(), 32));
        let junk = |k: &str| (k == "XCURSOR_SIZE").then(|| "huge".to_string());
        assert_eq!(CursorConfig::default().resolve(junk).1, 24);
    }

    /// The action the first bind on `keys` runs, as a key press finds it.
    fn action_on(c: &Config, keys: &str) -> Option<Action> {
        let (mods, keysym) = parse_keybind(keys).unwrap();
        (c.keybinds.iter())
            .find(|b| b.mods == mods && b.keysym == keysym)
            .map(|b| b.action.clone())
    }

    #[test]
    fn a_keybinds_list_layers_over_the_defaults() {
        let defaults = Config::default().keybinds;
        let c = Config::parse("[[keybinds]]\nkeys = \"Super+F9\"\naction = \"lock\"\n").unwrap();
        assert_eq!(c.keybinds[..defaults.len()], defaults[..]);
        assert_eq!(action_on(&c, "Super+F9"), Some(Action::Lock));

        let c = Config::parse("[[keybinds]]\nkeys = \"super+Q\"\naction = \"minimize\"\n").unwrap();
        assert_eq!(c.keybinds.len(), defaults.len());
        assert_eq!(action_on(&c, "Super+q"), Some(Action::Minimize));

        let c = Config::parse("[[keybinds]]\nkeys = \"Super+Tab\"\naction = \"none\"\n").unwrap();
        assert_eq!(c.keybinds.len(), defaults.len() - 1);
        assert_eq!(action_on(&c, "Super+Tab"), None);
        assert!(
            Config::parse("[[keybinds]]\nkeys = \"Super+Nope\"\naction = \"none\"\n").is_err(),
            "a none entry's keys still have to parse"
        );
    }

    #[test]
    fn an_old_full_keybinds_list_gains_super_tab() {
        let text: String = (repriseconf::keybind::DEFAULTS.iter())
            .filter(|(keys, _)| *keys != "Super+Tab")
            .map(|(keys, action)| {
                format!("[[keybinds]]\nkeys = \"{keys}\"\naction = \"{action}\"\n")
            })
            .collect();
        let c = Config::parse(&text).unwrap();
        assert_eq!(c.keybinds, Config::default().keybinds);
        assert_eq!(action_on(&c, "Super+Tab"), Some(Action::Flip));
    }

    #[test]
    fn keybind_parsing() {
        let (mods, sym) = parse_keybind("Super+Shift+E").unwrap();
        assert_eq!(mods, MOD_LOGO | MOD_SHIFT);
        assert_eq!(sym, KEY_e);
        let (mods, sym) = parse_keybind("Ctrl+Alt+F3").unwrap();
        assert_eq!(mods, MOD_CTRL | MOD_ALT);
        assert_eq!(sym, KEY_F3);
        let (mods, sym) = parse_keybind("Super+Return").unwrap();
        assert_eq!(mods, MOD_LOGO);
        assert_eq!(sym, KEY_Return);
        assert!(parse_keybind("Bogus+x").is_err());
        assert_eq!(
            parse_keybind("Win+x").unwrap(),
            parse_keybind("Super+x").unwrap(),
            "win is Super, as the settings sheet and its key field take it"
        );
        // The settings sheet's check trims, so this side must too.
        assert_eq!(
            parse_keybind("Super + t").unwrap(),
            parse_keybind("Super+t").unwrap()
        );
        assert!(parse_keybind("Super+NoSuchKey").is_err());
        assert!(parse_keybind("Super+").is_err());
    }

    #[test]
    fn defaults() {
        let c = Config::default();
        assert_eq!(c.terminal, "alacritty");
        assert!(c.retired.is_empty());
        assert_eq!(c.keybinds.len(), 40);
        assert!(matches!(c.keybinds[0].action, Action::Terminal));
        assert_eq!(c.startmenu.len(), 9);
        assert_eq!(c.startmenu_banner, "Reprise Desktop");
        assert!(matches!(
            &c.startmenu[0],
            MenuEntry::Item { label, target: MenuTarget::Programs, .. } if label == "&Programs"
        ));
        assert!(matches!(c.startmenu[6], MenuEntry::Separator));
        assert!(matches!(
            &c.startmenu[7],
            MenuEntry::Item {
                target: MenuTarget::Action(Action::LogOff),
                ..
            }
        ));
        assert_eq!(c.power, PowerConfig::default());
        assert_eq!(c.quicklaunch.len(), 3);
        assert_eq!(c.quicklaunch[1].action, Action::ShowDesktop);
    }

    #[test]
    fn action_names_round_trip() {
        for name in [
            "terminal",
            "close",
            "cycle",
            "flip",
            "exit",
            "maximize",
            "minimize",
            "lock",
            "startmenu",
            "run",
            "logoff",
            "turnoff",
            "windowmenu",
            "showdesktop",
            "toggleframe",
            "alwaysontop",
            "launcher",
            "overview",
            "snap-layouts",
            "focus-taskbar",
            "focus-tray",
            "clipboard-history",
            "emoji-picker",
            "next-layout",
            "center",
            "tab-next",
            "tab-prev",
            "do-not-disturb",
            "night-light",
            "desktop-prev",
            "desktop-next",
            "desktop-carry-prev",
            "desktop-carry-next",
            "all-desktops",
            "screenshot",
            "screenshot-window",
            "screenshot-region",
            "screenshot-content",
            "zoom-in",
            "zoom-out",
            "zoom-reset",
            "color-filter",
        ] {
            let a = Action::from_parts(name, None).unwrap();
            assert_eq!(a.name(), Some(name));
        }
        assert_eq!(Action::Spawn("x".into()).name(), None);
    }

    #[test]
    fn toggleframe_is_bound_by_default() {
        let (mods, keysym) = parse_keybind("Super+Shift+b").unwrap();
        let cfg = Config::default();
        assert!(
            cfg.keybinds
                .iter()
                .any(|b| b.mods == mods && b.keysym == keysym && b.action == Action::ToggleFrame)
        );
    }

    #[test]
    fn center_is_bound_to_super_c_by_default() {
        let (mods, keysym) = parse_keybind("Super+c").unwrap();
        let cfg = Config::default();
        assert!(
            cfg.keybinds
                .iter()
                .any(|b| b.mods == mods && b.keysym == keysym && b.action == Action::Center)
        );
    }

    #[test]
    fn print_screen_is_bound_by_default() {
        let cfg = Config::default();
        for (keys, action) in [
            ("Print", Action::Screenshot),
            ("Alt+Sys_Req", Action::ScreenshotWindow),
            ("Super+Shift+w", Action::ScreenshotWindow),
            ("Shift+Print", Action::ScreenshotRegion),
            ("Ctrl+Alt+Sys_Req", Action::ScreenshotContent),
        ] {
            let (mods, keysym) = parse_keybind(keys).unwrap();
            assert!(
                cfg.keybinds
                    .iter()
                    .any(|b| b.mods == mods && b.keysym == keysym && b.action == action),
                "{keys} copies with {action:?}"
            );
        }
    }

    #[test]
    fn previews_default_on_and_each_turns_off() {
        assert_eq!(
            Config::default().previews,
            PreviewsConfig {
                taskbar: true,
                fps: 10
            }
        );
        let c = Config::parse("[previews]\ntaskbar = false\n").unwrap();
        assert!(!c.previews.taskbar);
        assert_eq!(
            Config::parse("[previews]\nfps = 30\n")
                .unwrap()
                .previews
                .fps,
            30
        );
        assert!(Config::parse("[previews]\nfps = 0\n").is_err());
        assert!(Config::parse("[previews]\nfps = 61\n").is_err());
        assert!(Config::parse("[previews]\nfps = 12.5\n").is_err());
        assert!(Config::parse("[previews]\ncolour = true\n").is_err());
        assert!(
            Config::parse("[previews]\nswitcher = false\n").is_err(),
            "the Alt+Tab preview lives in [switcher] now"
        );
    }

    #[test]
    fn switcher_keys_follow_the_interface() {
        let classic = Config::default().switcher;
        assert_eq!(
            classic,
            SwitcherConfig {
                style: SwitcherStyle::Icons,
                preview: true,
                desktop: false,
                peek: false,
                flip: false,
                size: reprisekit::switcher::Size::Small,
            }
        );
        let glass = Config::parse("[appearance]\ninterface = \"glass\"\n")
            .unwrap()
            .switcher;
        assert_eq!(
            glass,
            SwitcherConfig {
                style: SwitcherStyle::Thumbnails,
                preview: true,
                desktop: true,
                peek: true,
                flip: true,
                size: reprisekit::switcher::Size::Small,
            }
        );
        let blue = Config::parse("[appearance]\ninterface = \"blue\"\n")
            .unwrap()
            .switcher;
        assert_eq!(blue, classic);
    }

    #[test]
    fn switcher_keys_override_the_interface() {
        let c = Config::parse(
            "[appearance]\ninterface = \"glass\"\n[switcher]\nstyle = \"icons\"\n\
             preview = false\ndesktop = false\npeek = false\nflip = false\n",
        )
        .unwrap();
        assert_eq!(
            c.switcher,
            SwitcherConfig {
                style: SwitcherStyle::Icons,
                preview: false,
                desktop: false,
                peek: false,
                flip: false,
                size: reprisekit::switcher::Size::Small,
            }
        );
        let c = Config::parse("[switcher]\nstyle = \"thumbnails\"\ndesktop = true\n").unwrap();
        assert_eq!(
            (c.switcher.style, c.switcher.desktop),
            (SwitcherStyle::Thumbnails, true)
        );
        assert!(Config::parse("[switcher]\nstyle = \"cards\"\n").is_err());
        assert_eq!(
            Config::parse("[switcher]\nsize = \"large\"\n")
                .unwrap()
                .switcher
                .size,
            reprisekit::switcher::Size::Large
        );
        assert_eq!(
            Config::parse("[switcher]\nsize = \"huge\"\n")
                .unwrap()
                .switcher
                .size,
            reprisekit::switcher::Size::Huge
        );
        assert!(Config::parse("[switcher]\nsize = \"giant\"\n").is_err());
    }

    #[test]
    fn the_hot_corner_is_off_until_a_corner_is_named() {
        assert_eq!(Config::default().hot_corner, HotCorner::None);
        let c = Config::parse("[overview]\nhot_corner = \"top-left\"\n").unwrap();
        assert_eq!(c.hot_corner, HotCorner::TopLeft);
        assert!(Config::parse("[overview]\nhot_corner = \"middle\"\n").is_err());
        for h in HotCorner::ALL {
            assert_eq!(HotCorner::from_name(h.name()), Some(h));
        }
    }

    #[test]
    fn gestures_take_three_fingers_until_told_four() {
        let d = Config::default().gestures;
        assert_eq!((d.enabled, d.fingers), (true, 3));
        let c = Config::parse("[gestures]\nenabled = false\nfingers = 4\n").unwrap();
        assert_eq!((c.gestures.enabled, c.gestures.fingers), (false, 4));
        assert!(Config::parse("[gestures]\nfingers = 2\n").is_err());
    }

    #[test]
    fn gesture_tuning_defaults_and_ranges() {
        let d = Config::default().gestures;
        assert_eq!((d.distance, d.commit_at, d.flick_speed), (400, 30, 300));
        let c = Config::parse("[gestures]\ndistance = 600\ncommit_at = 50\nflick_speed = 800\n")
            .unwrap()
            .gestures;
        assert_eq!((c.distance, c.commit_at, c.flick_speed), (600, 50, 800));
        for bad in [
            "distance = 99",
            "distance = 2001",
            "commit_at = 4",
            "commit_at = 96",
            "flick_speed = 49",
            "flick_speed = 5001",
            "flick_speed = -1",
        ] {
            assert!(
                Config::parse(&format!("[gestures]\n{bad}\n")).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn screenshot_save_defaults_on_and_is_configurable() {
        assert!(Config::default().screenshot.save);
        let c = Config::parse("[screenshot]\nsave = false\n").unwrap();
        assert!(!c.screenshot.save);
        assert!(Config::parse("[screenshot]\nfolder = \"x\"\n").is_err());
    }

    #[test]
    fn night_light_is_off_by_default_and_every_key_is_checked() {
        let d = Config::default().night_light;
        assert!(!d.enabled);
        assert_eq!((d.temperature, d.schedule), (4000, NightSchedule::Sun));
        assert_eq!((d.from, d.to, d.location), (21 * 60, 7 * 60, None));
        let c = Config::parse(
            "[night_light]\nenabled = true\ntemperature = 3400\nschedule = \"fixed\"\n\
             from = \"20:30\"\nto = \"6:05\"\nlatitude = 47.5\nlongitude = 19.08\n",
        )
        .unwrap()
        .night_light;
        assert!(c.enabled);
        assert_eq!((c.temperature, c.schedule), (3400, NightSchedule::Fixed));
        assert_eq!((c.from, c.to), (20 * 60 + 30, 6 * 60 + 5));
        assert_eq!(c.location, Some((47.5, 19.08)));
        for bad in [
            "temperature = 900",
            "temperature = 7000",
            "schedule = \"moon\"",
            "from = \"25:00\"",
            "to = \"7\"",
            "from = \"07:00\"",
            "latitude = 47.5",
            "latitude = 91.0\nlongitude = 0.0",
            "brightness = 1",
        ] {
            assert!(
                Config::parse(&format!("[night_light]\n{bad}\n")).is_err(),
                "{bad} is refused"
            );
        }
        let bind = Config::parse("[[keybinds]]\nkeys = \"Super+n\"\naction = \"night-light\"\n");
        assert_eq!(
            action_on(&bind.unwrap(), "Super+n"),
            Some(Action::NightLight)
        );
    }

    #[test]
    fn no_colour_filter_by_default_and_each_one_parses() {
        use crate::magnifier::Follow;
        use reprisekit::colorfilter::Filter;
        let d = Config::default().accessibility;
        assert_eq!(d.color_filter, Filter::None);
        assert_eq!(d.color_filter_intensity, 100);
        assert_eq!(d.magnifier_follow, Follow::PointerAndFocus);
        let c = Config::parse(
            "[accessibility]\ncolor_filter = \"tritanopia\"\n\
             color_filter_intensity = 40\nmagnifier_follow = \"pointer\"\n",
        )
        .unwrap()
        .accessibility;
        assert_eq!(c.color_filter, Filter::Tritanopia);
        assert_eq!(c.color_filter_intensity, 40);
        assert_eq!(c.magnifier_follow, Follow::Pointer);
        for bad in [
            "color_filter = \"sepia\"",
            "color_filter_intensity = 101",
            "color_filter_intensity = -1",
            "magnifier_follow = \"caret\"",
        ] {
            assert!(
                Config::parse(&format!("[accessibility]\n{bad}\n")).is_err(),
                "{bad} is refused"
            );
        }
        let (mods, keysym) = parse_keybind("Super+Ctrl+c").unwrap();
        assert!(
            Config::default()
                .keybinds
                .iter()
                .any(|b| b.mods == mods && b.keysym == keysym && b.action == Action::ColorFilter),
            "Super+Ctrl+C toggles the filter"
        );
    }

    #[test]
    fn the_key_aids_are_off_with_their_shortcuts_on_by_default() {
        let d = Config::default().accessibility;
        assert!(!d.sticky_keys && !d.slow_keys);
        assert!(d.sticky_lock && d.sticky_two_keys_off && d.shortcuts);
        assert_eq!(d.slow_keys_delay, 1000);
        let c = Config::parse(
            "[accessibility]\nsticky_keys = true\nsticky_lock = false\n\
             sticky_two_keys_off = false\nslow_keys = true\nslow_keys_delay = 0.3\n\
             shortcuts = false\n",
        )
        .unwrap()
        .accessibility;
        assert!(c.sticky_keys && c.slow_keys);
        assert!(!c.sticky_lock && !c.sticky_two_keys_off && !c.shortcuts);
        assert_eq!(c.slow_keys_delay, 300);
        for bad in [
            "slow_keys_delay = 0.2",
            "slow_keys_delay = 2.5",
            "sticky = true",
        ] {
            assert!(
                Config::parse(&format!("[accessibility]\n{bad}\n")).is_err(),
                "{bad} is refused"
            );
        }
    }

    #[test]
    fn mouse_keys_and_the_text_size_parse_within_their_ranges() {
        let d = Config::default().accessibility;
        assert!(!d.mouse_keys && d.mouse_keys_num_lock);
        assert_eq!((d.mouse_keys_speed, d.mouse_keys_acceleration), (400, 1000));
        assert_eq!(d.text_size, 100);
        let c = Config::parse(
            "[accessibility]\nmouse_keys = true\nmouse_keys_speed = 800\n\
             mouse_keys_acceleration = 0.5\nmouse_keys_num_lock = false\ntext_size = 150\n",
        )
        .unwrap()
        .accessibility;
        assert!(c.mouse_keys && !c.mouse_keys_num_lock);
        assert_eq!((c.mouse_keys_speed, c.mouse_keys_acceleration), (800, 500));
        assert_eq!(c.text_size, 150);
        for bad in [
            "mouse_keys_speed = 10",
            "mouse_keys_speed = 5000",
            "mouse_keys_acceleration = -1.0",
            "mouse_keys_acceleration = 6.0",
            "text_size = 75",
            "text_size = 140",
            "text_size = 225",
        ] {
            assert!(
                Config::parse(&format!("[accessibility]\n{bad}\n")).is_err(),
                "{bad} is refused"
            );
        }
    }

    #[test]
    fn clipboard_history_is_on_bound_to_super_v_and_configurable() {
        let cfg = Config::default();
        assert!(cfg.clipboard.history);
        assert!(cfg.clipboard.terminal_apps.contains(&"foot".to_string()));
        let (mods, keysym) = parse_keybind("Super+v").unwrap();
        assert!(
            cfg.keybinds.iter().any(|b| b.mods == mods
                && b.keysym == keysym
                && b.action == Action::ClipboardHistory)
        );
        let c = Config::parse("[clipboard]\nhistory = false\nterminal_apps = [\"st\"]\n").unwrap();
        assert!(!c.clipboard.history);
        assert_eq!(c.clipboard.terminal_apps, vec!["st".to_string()]);
        assert!(cfg.clipboard.trusted_apps.is_empty());
        let c = Config::parse("[clipboard]\ntrusted_apps = [\"me.proton.Pass\"]\n").unwrap();
        assert_eq!(c.clipboard.trusted_apps, vec!["me.proton.Pass".to_string()]);
        assert!(Config::parse("[clipboard]\nsize = 3\n").is_err());
    }

    #[test]
    fn the_emoji_picker_is_bound_to_super_period() {
        let (mods, keysym) = parse_keybind("Super+period").unwrap();
        assert!(
            Config::default()
                .keybinds
                .iter()
                .any(|b| b.mods == mods && b.keysym == keysym && b.action == Action::EmojiPicker)
        );
    }

    #[test]
    fn effects_follow_the_interface_and_the_keys_override_them() {
        let classic = Config::parse("").unwrap().effects;
        assert_eq!(classic, EffectsConfig::default_for(Interface::Classic));
        assert!(
            !classic.window_animation && !classic.hover_fade,
            "Classic stays still"
        );
        assert_eq!(
            (classic.window_open_ms, classic.window_close_ms),
            (250, 200)
        );
        assert_eq!(classic.window_scale, 0.95);
        assert_eq!((classic.hover_in_ms, classic.hover_out_ms), (120, 250));
        let blue = Config::parse("[appearance]\ninterface = \"blue\"\n")
            .unwrap()
            .effects;
        assert_eq!(blue, classic, "Blue is still by default");
        let glass = Config::parse("[appearance]\ninterface = \"glass\"\n")
            .unwrap()
            .effects;
        assert!(
            glass.window_animation && glass.hover_fade,
            "Glass moves by default"
        );
        assert!(glass.opens() && glass.closes());
        assert_eq!(glass.hover_ms(), (120, 250));
        let on_blue = Config::parse(
            "[appearance]\ninterface = \"blue\"\n[effects]\nwindow_animation = true\nhover_fade = true\nwindow_open_ms = 400\nwindow_scale = 0.9\n",
        )
        .unwrap()
        .effects;
        assert!(on_blue.window_animation && on_blue.hover_fade);
        assert_eq!(on_blue.window_open_ms, 400);
        assert_eq!(on_blue.window_scale, 0.9);
        let off = Config::parse(
            "[appearance]\ninterface = \"glass\"\n[effects]\nhover_fade = false\nwindow_close_ms = 0\n",
        )
        .unwrap()
        .effects;
        assert_eq!(off.hover_ms(), (0, 0), "no fade: both instant");
        assert!(off.opens() && !off.closes(), "0 ms is no close animation");
    }

    #[test]
    fn task_buttons_slide_on_glass_by_default_and_the_key_overrides_it() {
        let parse = |s: &str| Config::parse(s).unwrap().effects.taskbar_slide;
        assert!(!parse(""), "Classic stays still");
        assert!(
            !parse("[appearance]\ninterface = \"blue\"\n"),
            "so does Blue"
        );
        assert!(parse("[appearance]\ninterface = \"glass\"\n"));
        assert!(parse("[effects]\ntaskbar_slide = true\n"));
        assert!(!parse(
            "[appearance]\ninterface = \"glass\"\n[effects]\ntaskbar_slide = false\n"
        ));
    }

    #[test]
    fn effects_values_out_of_range_are_errors() {
        for bad in [
            "window_open_ms = -1",
            "window_open_ms = 2001",
            "window_close_ms = 2001",
            "hover_in_ms = 2001",
            "hover_out_ms = -5",
            "window_scale = 0.79",
            "window_scale = 1.01",
            "window_scale = nan",
            "wobble = true",
        ] {
            assert!(
                Config::parse(&format!("[effects]\n{bad}\n")).is_err(),
                "{bad}"
            );
        }
        assert!(
            Config::parse("[effects]\nwindow_scale = 0.8\nhover_in_ms = 0\nhover_out_ms = 2000\n")
                .is_ok()
        );
    }

    #[test]
    fn desktop_switches_are_instant_by_default_and_the_animation_is_checked() {
        let cfg = Config::default();
        assert_eq!(cfg.workspace_animation.style, SwitchStyle::None);
        assert_eq!(cfg.workspace_animation.ms, 200);
        assert!(!cfg.workspace_animation.enabled(), "none is no animation");
        for style in SwitchStyle::ALL {
            let c =
                Config::parse(&format!("[workspaces]\nanimation = {:?}\n", style.name())).unwrap();
            assert_eq!(c.workspace_animation.style, style);
        }
        let c = Config::parse("[workspaces]\nanimation = \"fade\"\nanimation_ms = 0\n").unwrap();
        assert_eq!(c.workspace_animation.ms, 0);
        assert!(!c.workspace_animation.enabled(), "0 ms is no animation");
        let c = Config::parse("[workspaces]\nanimation = \"none\"\n").unwrap();
        assert!(!c.workspace_animation.enabled());
        let c = Config::parse("[workspaces]\nanimation_ms = 350\n").unwrap();
        assert_eq!(c.workspaces, 4);
        assert_eq!(c.workspace_animation.style, SwitchStyle::None);
        assert_eq!(c.workspace_animation.ms, 350);
        assert!(Config::parse("[workspaces]\nanimation = \"spin\"\n").is_err());
        assert!(Config::parse("[workspaces]\nanimation_ms = -1\n").is_err());
        assert!(Config::parse("[workspaces]\nanimation_ms = 2001\n").is_err());
    }

    #[test]
    fn the_pager_mark_defaults_to_light_and_rejects_others() {
        use reprisekit::taskbar::PagerMark;
        assert_eq!(Config::default().pager_mark, PagerMark::Light);
        let c = Config::parse("[workspaces]\npager = \"tile\"\n").unwrap();
        assert_eq!(c.pager_mark, PagerMark::Tile);
        let c = Config::parse("[workspaces]\npager = \"light\"\n").unwrap();
        assert_eq!(c.pager_mark, PagerMark::Light);
        let err = Config::parse("[workspaces]\npager = \"box\"\n").unwrap_err();
        assert!(
            err.contains("\"light\"") && err.contains("\"tile\""),
            "the error names both choices: {err}"
        );
    }

    #[test]
    fn four_desktops_by_default_and_count_is_checked() {
        use crate::desktops::Step;
        let cfg = Config::default();
        assert_eq!(cfg.workspaces, 4);
        for (keys, action) in [
            ("Super+Ctrl+Left", Action::Desktop(Step::Prev)),
            ("Super+Ctrl+Right", Action::Desktop(Step::Next)),
            ("Super+Ctrl+Shift+Left", Action::CarryToDesktop(Step::Prev)),
            ("Super+Ctrl+Shift+Right", Action::CarryToDesktop(Step::Next)),
        ] {
            let (mods, keysym) = parse_keybind(keys).unwrap();
            assert!(
                cfg.keybinds
                    .iter()
                    .any(|b| b.mods == mods && b.keysym == keysym && b.action == action),
                "{keys} runs {action:?}"
            );
        }
        assert_eq!(
            Config::parse("[workspaces]\ncount = 1\n")
                .unwrap()
                .workspaces,
            1
        );
        assert_eq!(
            Config::parse("[workspaces]\ncount = 16\n")
                .unwrap()
                .workspaces,
            16
        );
        assert!(Config::parse("[workspaces]\ncount = 0\n").is_err());
        assert!(Config::parse("[workspaces]\ncount = 17\n").is_err());
        assert!(Config::parse("[workspaces]\nnames = []\n").is_err());
    }

    #[test]
    fn the_tab_keys_are_bound_by_default() {
        let cfg = Config::default();
        for (keys, action) in [
            ("Super+grave", Action::Tab(false)),
            ("Super+Page_Down", Action::Tab(false)),
            ("Super+Page_Up", Action::Tab(true)),
        ] {
            let (mods, keysym) = parse_keybind(keys).unwrap();
            assert!(
                cfg.keybinds
                    .iter()
                    .any(|b| b.mods == mods && b.keysym == keysym && b.action == action),
                "{keys} runs {action:?}"
            );
        }
    }

    #[test]
    fn launcher_is_bound_to_super_space_by_default() {
        let (mods, keysym) = parse_keybind("Super+space").unwrap();
        let cfg = Config::default();
        assert!(
            cfg.keybinds
                .iter()
                .any(|b| b.mods == mods && b.keysym == keysym && b.action == Action::Launcher)
        );
    }

    #[test]
    fn media_actions_round_trip_through_their_names() {
        for a in [
            Action::VolumeUp,
            Action::VolumeDown,
            Action::Mute,
            Action::MicMute,
            Action::BrightnessUp,
            Action::BrightnessDown,
            Action::KbdBrightnessUp,
            Action::KbdBrightnessDown,
            Action::PlayPause,
            Action::Next,
            Action::Prev,
            Action::Stop,
            Action::Wifi,
        ] {
            assert_eq!(Action::from_parts(a.name().unwrap(), None), Ok(a));
        }
    }

    #[test]
    fn quicklaunch_entries_parse_and_an_empty_list_hides_the_toolbar() {
        let c = Config::parse(
            "[[quicklaunch]]\nlabel = \"Files\"\nicon = \"folder\"\ncommand = \"nautilus\"\n\n[[quicklaunch]]\nlabel = \"Desk\"\naction = \"showdesktop\"\n",
        )
        .unwrap();
        assert_eq!(
            c.quicklaunch,
            vec![
                QuickLaunchEntry {
                    label: "Files".into(),
                    icon: Some("folder".into()),
                    action: Action::Spawn("nautilus".into()),
                },
                QuickLaunchEntry {
                    label: "Desk".into(),
                    icon: None,
                    action: Action::ShowDesktop,
                },
            ]
        );
        assert!(
            Config::parse("quicklaunch = []\n")
                .unwrap()
                .quicklaunch
                .is_empty()
        );
        assert!(Config::parse("[[quicklaunch]]\nlabel = \"Nothing\"\n").is_err());
    }

    #[test]
    fn dialog_actions_and_power_commands_parse() {
        let c = Config::parse(
            "[power]\nreboot = \"loginctl reboot\"\n\n[[keybinds]]\nkeys = \"Super+F4\"\naction = \"turnoff\"\n\n[[startmenu]]\nlabel = \"Go\"\naction = \"run\"\n\n[[startmenu]]\nlabel = \"Bye\"\naction = \"logoff\"\n",
        )
        .unwrap();
        assert_eq!(c.power.reboot, "loginctl reboot");
        assert_eq!(c.power.standby, "systemctl suspend");
        assert_eq!(action_on(&c, "Super+F4"), Some(Action::TurnOff));
        assert!(matches!(
            &c.startmenu[0],
            MenuEntry::Item {
                target: MenuTarget::Action(Action::Run),
                ..
            }
        ));
        assert!(matches!(
            &c.startmenu[1],
            MenuEntry::Item {
                target: MenuTarget::Action(Action::LogOff),
                ..
            }
        ));
        assert!(Config::parse("[power]\nhalt = \"x\"\n").is_err());
    }

    #[test]
    fn lock_action_and_default_bind() {
        let c = Config::default();
        assert!(
            c.keybinds
                .iter()
                .any(|b| b.action == Action::Lock && b.mods == MOD_LOGO && b.keysym == KEY_l)
        );
        let c = Config::parse("[[keybinds]]\nkeys = \"Super+F12\"\naction = \"lock\"\n").unwrap();
        assert_eq!(action_on(&c, "Super+F12"), Some(Action::Lock));
    }

    #[test]
    fn full_file() {
        let text = r##"
terminal = "kitty"

[colors]
active_title = "#000000"
background = "1 2 3"

[metrics]
caption_height = 20

[fonts]
family = ["DejaVu Sans"]
size_px = 12
antialias = true

[[keybinds]]
keys = "Super+t"
action = "spawn"
command = "kitty"

[[keybinds]]
keys = "Super+w"
action = "close"

[[startmenu]]
label = "Kitty"
command = "kitty"

[[startmenu]]
separator = true

[[startmenu]]
label = "Quit"
action = "exit"
"##;
        let c = Config::parse(text).unwrap();
        assert_eq!(c.terminal, "kitty");
        assert_eq!(c.theme.colors.active_title, Color::rgb(0, 0, 0));
        assert_eq!(c.theme.colors.background, Color::rgb(1, 2, 3));
        assert_eq!(c.theme.colors.button_face, Color::rgb(212, 208, 200));
        assert_eq!(c.theme.metrics.caption_height, 20);
        assert_eq!(c.theme.metrics.frame_width, 4);
        assert_eq!(c.theme.fonts.family, vec!["DejaVu Sans".to_string()]);
        assert!(c.theme.fonts.antialias);
        assert_eq!(c.keybinds.len(), 40);
        assert_eq!(
            action_on(&c, "Super+t"),
            Some(Action::Spawn("kitty".into()))
        );
        assert_eq!(action_on(&c, "Super+w"), Some(Action::Close));
        assert_eq!(c.startmenu.len(), 3);
        assert!(matches!(
            &c.startmenu[0],
            MenuEntry::Item { target: MenuTarget::Action(Action::Spawn(s)), icon: None, .. } if s == "kitty"
        ));
        assert!(matches!(c.startmenu[1], MenuEntry::Separator));
    }

    #[test]
    fn the_start_menu_style_follows_the_interface_unless_set() {
        let glass = Config::parse("[appearance]\ninterface = \"glass\"\n").unwrap();
        assert_eq!(glass.startmenu_style, StartMenuStyle::Glass);
        let classic = Config::parse("").unwrap();
        assert_eq!(classic.startmenu_style, StartMenuStyle::Classic);
        let set =
            Config::parse("startmenu_style = \"classic\"\n[appearance]\ninterface = \"glass\"\n")
                .unwrap();
        assert_eq!(set.startmenu_style, StartMenuStyle::Classic);
    }

    #[test]
    fn an_unknown_start_menu_style_is_an_error() {
        let err = Config::parse("startmenu_style = \"round\"\n").unwrap_err();
        assert!(err.contains("startmenu_style"), "{err}");
    }

    #[test]
    fn startmenu_entries_nest_and_name_dynamic_submenus() {
        let text = r##"
startmenu_banner = "Custom Banner"

[[startmenu]]
label = "Programs"
icon = "applications-other"
submenu = "programs"

[[startmenu]]
label = "Settings"
[[startmenu.items]]
label = "Control Panel"
submenu = "settings"
[[startmenu.items]]
label = "Docs"
icon = ""
submenu = "documents"
[[startmenu.items]]
separator = true
[[startmenu.items]]
label = "Editor"
command = "gedit"
"##;
        let c = Config::parse(text).unwrap();
        assert_eq!(c.startmenu_banner, "Custom Banner");
        assert!(matches!(
            &c.startmenu[0],
            MenuEntry::Item { icon: Some(i), target: MenuTarget::Programs, .. } if i == "applications-other"
        ));
        let MenuEntry::Item {
            target: MenuTarget::Submenu(items),
            ..
        } = &c.startmenu[1]
        else {
            panic!("expected a static submenu");
        };
        assert_eq!(items.len(), 4);
        assert!(matches!(
            items[0],
            MenuEntry::Item {
                target: MenuTarget::Settings,
                ..
            }
        ));
        assert!(matches!(
            items[1],
            MenuEntry::Item {
                icon: None,
                target: MenuTarget::Documents,
                ..
            }
        ));
        assert!(matches!(items[2], MenuEntry::Separator));
        assert!(matches!(
            &items[3],
            MenuEntry::Item { target: MenuTarget::Action(Action::Spawn(s)), .. } if s == "gedit"
        ));
        assert!(c.startmenu[0].opens_submenu());
        assert!(!items[3].opens_submenu());
        assert!(!items[2].opens_submenu());
    }

    #[test]
    fn startmenu_entry_rejects_ambiguous_or_unknown_targets() {
        let err = Config::parse("[[startmenu]]\nlabel = \"x\"\nsubmenu = \"bogus\"\n").unwrap_err();
        assert!(err.contains("unknown submenu"), "{err}");
        let err = Config::parse(
            "[[startmenu]]\nlabel = \"x\"\ncommand = \"a\"\nsubmenu = \"programs\"\n",
        )
        .unwrap_err();
        assert!(err.contains("mutually exclusive"), "{err}");
    }

    #[test]
    fn a_startmenu_folder_without_items_is_an_empty_folder() {
        for text in [
            "[[startmenu]]\nlabel = \"x\"\n",
            "[[startmenu]]\nlabel = \"x\"\nitems = []\n",
        ] {
            let c = Config::parse(text).unwrap();
            assert!(
                matches!(
                    &c.startmenu[..],
                    [MenuEntry::Item { target: MenuTarget::Submenu(items), .. }] if items.is_empty()
                ),
                "{text:?} is one empty folder"
            );
        }
    }

    #[test]
    fn partial_file_keeps_defaults() {
        let c = Config::parse("terminal = \"foot\"\n").unwrap();
        assert_eq!(c.terminal, "foot");
        assert_eq!(c.keybinds.len(), 40);
        assert_eq!(c.theme, crate::theme::Theme::default());
    }

    #[test]
    fn errors_are_reported() {
        assert!(Config::parse("terminal = ").is_err());
        assert!(Config::parse("[colors]\nactive_title = \"zzz\"\n").is_err());
        assert!(Config::parse("[colors]\nnope = \"#000000\"\n").is_err());
        assert!(Config::parse("[[keybinds]]\nkeys = \"Super+x\"\naction = \"dance\"\n").is_err());
        assert!(Config::parse("[[keybinds]]\nkeys = \"Super+x\"\naction = \"spawn\"\n").is_err());
    }

    #[test]
    fn load_missing_file_gives_defaults() {
        let (c, notice) = Config::load(Some(std::path::Path::new("/nonexistent/reprise.toml")));
        assert_eq!(c.terminal, "alacritty");
        assert_eq!(notice, None, "a missing file is not worth a notice");
    }

    #[test]
    fn load_malformed_file_falls_back() {
        let path =
            std::env::temp_dir().join(format!("reprise-bad-config-{}.toml", std::process::id()));
        std::fs::write(&path, "terminal = \n[colors]\nactive_title = \"zzz\"\n").unwrap();
        let (c, notice) = Config::load(Some(&path));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(c, Config::default());
        assert!(notice.is_some(), "a file that is not TOML gets a notice");
    }

    #[test]
    fn load_unreadable_file_gives_defaults_and_a_notice() {
        let dir = std::env::temp_dir();
        let (c, notice) = Config::load(Some(&dir));
        assert_eq!(c, Config::default());
        assert!(notice.is_some(), "a path that cannot be read gets a notice");
    }

    /// An unknown key or bad value costs only its own top-level key at login; `Config::parse` refuses the same file whole.
    #[test]
    fn load_drops_only_the_sections_that_fail() {
        let text = "terminal = \"foot\"\n\
                    future_key = 1\n\
                    [idle]\nlock_after = 0\n\
                    [power]\nlid_close = \"standby\"\nnewer_key = true\n\
                    [[outputs]]\nname = \"Virtual-1\"\nmode = \"1920x1080@60\"\n\
                    [[keybinds]]\nkeys = \"Super+x\"\naction = \"dance\"\n";
        assert!(Config::parse(text).is_err());
        let (c, dropped) = Config::parse_lenient(text).unwrap();
        let keys: Vec<&str> = dropped.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["future_key", "keybinds", "power"]);
        assert_eq!(c.terminal, "foot");
        assert_eq!(c.idle.lock_after, 0);
        assert_eq!(c.outputs.len(), 1, "outputs keeps Virtual-1");
        assert_eq!(c.power, Config::default().power, "power is at its defaults");
        assert_eq!(
            c.keybinds,
            Config::default().keybinds,
            "keybinds are at their defaults"
        );

        let path =
            std::env::temp_dir().join(format!("reprise-skew-config-{}.toml", std::process::id()));
        std::fs::write(&path, text).unwrap();
        let (loaded, notice) = Config::load(Some(&path));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(loaded, c);
        let notice = notice.expect("dropped keys get a notice");
        assert!(notice.contains("future_key, keybinds, power"), "{notice}");
    }

    #[test]
    fn keyboard_section_parses_and_defaults() {
        let c = Config::parse("[keyboard]\nlayout = \"hu\"\nrepeat_rate = 30\n").unwrap();
        assert_eq!(c.keyboard.layout, "hu");
        assert_eq!(c.keyboard.repeat_rate, 30);
        assert_eq!(c.keyboard.repeat_delay, 600);
        let xkb = c.keyboard.xkb_config();
        assert_eq!(xkb.rules, "");
        assert_eq!(xkb.layout, "hu");
        assert_eq!(xkb.options, None, "no options picks the defaults");
        assert_eq!(Config::default().keyboard.layout, "");
    }

    #[test]
    fn recent_files_default_on_and_privacy_turns_them_off() {
        assert!(Config::default().recent_files);
        assert!(
            !Config::parse("[privacy]\nrecent_files = false\n")
                .unwrap()
                .recent_files
        );
    }

    #[test]
    fn location_default_on_and_privacy_turns_it_off() {
        assert!(Config::default().location);
        assert!(
            !Config::parse("[privacy]\nlocation = false\n")
                .unwrap()
                .location
        );
    }

    #[test]
    fn the_debug_overlay_is_off_and_left_out_of_captures_by_default() {
        assert_eq!(Config::default().debug, DebugConfig::default());
        assert!(!Config::default().debug.overlay);
        let d = Config::parse("[debug]\noverlay = true\noverlay_in_captures = true\n")
            .unwrap()
            .debug;
        assert!(d.overlay && d.overlay_in_captures);
    }

    #[test]
    fn the_launcher_searches_files_unless_turned_off() {
        assert!(Config::default().launcher_files);
        assert!(
            !Config::parse("[launcher]\nsearch_files = false\n")
                .unwrap()
                .launcher_files
        );
    }

    #[test]
    fn script_terminal_defaults_to_stay_and_reads_close() {
        use reprisefs::program::ScriptEnd;
        assert_eq!(Config::default().script_terminal, ScriptEnd::Stay);
        let c = Config::parse("script_terminal = \"close\"\n").unwrap();
        assert_eq!(c.script_terminal, ScriptEnd::Close);
        let c = Config::parse("script_terminal = \"stay\"\n").unwrap();
        assert_eq!(c.script_terminal, ScriptEnd::Stay);
        assert!(Config::parse("script_terminal = \"open\"\n").is_err());
    }

    #[test]
    fn desktop_extensions_default_to_hide_and_read_each_mode() {
        use reprisefs::names::Extensions;
        assert_eq!(Config::default().desktop_extensions, Extensions::Hide);
        for (text, want) in [
            ("hide", Extensions::Hide),
            ("hide_launchers", Extensions::HideLaunchers),
            ("show", Extensions::Show),
        ] {
            let c = Config::parse(&format!("desktop_extensions = \"{text}\"\n")).unwrap();
            assert_eq!(c.desktop_extensions, want, "{text}");
        }
        let err = Config::parse("desktop_extensions = \"all\"\n").unwrap_err();
        assert!(err.contains("hide, hide_launchers or show"), "{err}");
    }

    #[test]
    fn desktop_icons_defaults_on_and_is_configurable() {
        assert!(Config::default().desktop_icons);
        assert_eq!(Config::default().system_icons.len(), 4);
        assert_eq!(
            Config::parse("system_icons = [\"recycle-bin\"]\n")
                .unwrap()
                .system_icons,
            ["recycle-bin"]
        );
        assert!(
            Config::parse("system_icons = []\n")
                .unwrap()
                .system_icons
                .is_empty()
        );
        assert_eq!(
            Config::parse(
                "system_icons = [\"my-computer\", \"my-network-places\", \"my-documents\"]\n"
            )
            .unwrap()
            .system_icons,
            ["computer", "documents"],
            "an older file's names read as the current ones"
        );
        assert!(
            !Config::parse("desktop_icons = false\n")
                .unwrap()
                .desktop_icons
        );
    }

    #[test]
    fn start_icon_defaults_to_the_reprise_mark_and_is_configurable() {
        assert!(!Config::default().start_icon);
        assert!(Config::parse("start_icon = true\n").unwrap().start_icon);
    }

    #[test]
    fn keyboard_repeat_rejects_negative() {
        assert!(Config::parse("[keyboard]\nrepeat_rate = -1\n").is_err());
        assert!(Config::parse("[keyboard]\nrepeat_delay = -1\n").is_err());
        assert_eq!(
            Config::parse("[keyboard]\nrepeat_rate = 0\n")
                .unwrap()
                .keyboard
                .repeat_rate,
            0
        );
    }

    #[test]
    fn pointer_sections_parse_and_default() {
        let d = Config::default();
        assert_eq!(
            d.mouse,
            PointerConfig {
                speed: 0.0,
                acceleration: true,
                natural_scroll: false,
            }
        );
        assert_eq!(
            d.touchpad,
            TouchpadConfig {
                pointer: PointerConfig {
                    speed: 0.0,
                    acceleration: true,
                    natural_scroll: false,
                },
                tap_to_click: true,
                two_finger_scroll: true,
                two_finger_tap_right_click: true,
                disable_while_typing: true,
            }
        );
        let c = Config::parse(
            "[mouse]\nspeed = -0.5\nacceleration = false\nnatural_scroll = true\n\
             [touchpad]\nspeed = 1\ntap_to_click = false\ntwo_finger_scroll = false\n\
             disable_while_typing = false\n",
        )
        .unwrap();
        assert_eq!(c.mouse.speed, -0.5);
        assert!(!c.mouse.acceleration);
        assert!(c.mouse.natural_scroll);
        assert_eq!(c.touchpad.pointer.speed, 1.0);
        assert!(
            !c.touchpad.pointer.natural_scroll,
            "each class keeps its own direction"
        );
        assert!(c.touchpad.pointer.acceleration);
        assert!(!c.touchpad.tap_to_click);
        assert!(!c.touchpad.two_finger_scroll);
        assert!(c.touchpad.two_finger_tap_right_click);
        assert!(!c.touchpad.disable_while_typing);
        let c = Config::parse("[touchpad]\nnatural_scroll = true\n").unwrap();
        assert!(c.touchpad.pointer.natural_scroll);
        assert!(!c.mouse.natural_scroll);
    }

    #[test]
    fn touchscreen_output_parses() {
        assert_eq!(Config::default().touchscreen.output, None);
        let c = Config::parse("[touchscreen]\noutput = \"DP-2\"\n").unwrap();
        assert_eq!(c.touchscreen.output.as_deref(), Some("DP-2"));
        let c = Config::parse("[touchscreen]\noutput = \"\"\n").unwrap();
        assert_eq!(c.touchscreen.output, None);
        assert!(Config::parse("[touchscreen]\nmap = \"DP-2\"\n").is_err());
    }

    #[test]
    fn pointer_speed_rejects_out_of_range() {
        assert!(Config::parse("[mouse]\nspeed = 1.5\n").is_err());
        assert!(Config::parse("[touchpad]\nspeed = -2\n").is_err());
    }

    #[test]
    fn example_config_parses() {
        let text = include_str!("../config.example.toml");
        let c = Config::parse(text).unwrap();
        assert_eq!(
            Config::parse_lenient(text).unwrap(),
            (c.clone(), Vec::new()),
            "the login parse reads the example the same, dropping nothing"
        );
        assert_eq!(c.startmenu, Config::default().startmenu);
        assert_eq!(c.keybinds, Config::default().keybinds);
        assert!(c.outputs.is_empty());
    }

    #[test]
    fn session_restore_is_off_by_default_and_reads_its_exclusions() {
        let d = Config::default();
        assert!(!d.session.restore && d.session.exclude.is_empty());
        let c =
            Config::parse("[session]\nrestore = true\nexclude = [\"foot\", \"org.*\"]\n").unwrap();
        assert_eq!(
            c.session,
            SessionConfig {
                restore: true,
                exclude: vec!["foot".into(), "org.*".into()],
                end_processes: true,
            }
        );
        assert!(Config::parse("[session]\nrestor = true\n").is_err());
    }

    #[test]
    fn log_off_ends_leftover_processes_unless_turned_off() {
        assert!(Config::default().session.end_processes);
        let c = Config::parse("[session]\nend_processes = false\n").unwrap();
        assert!(!c.session.end_processes);
    }

    #[test]
    fn window_rules_parse_in_file_order_with_desktops_counted_from_one() {
        let c = Config::parse(
            "[windows]\nremember = false\n\n\
             [[window_rules]]\napp_id = \"foot\"\nsize = [800, 600]\nposition = [-10, 40]\n\
             output = \"DP-2\"\nstate = \"maximized\"\ndesktop = 2\nalways_on_top = true\n\
             sticky = false\ntab_group = \"shells\"\n\n\
             [[window_rules]]\napp_id = \"firefox\"\ntitle = \"*Mail*\"\n",
        )
        .unwrap();
        assert!(!c.remember_windows);
        assert_eq!(
            c.window_rules,
            [
                WindowRule {
                    app_id: "foot".into(),
                    title: None,
                    size: Some((800, 600)),
                    position: Some((-10, 40)),
                    output: Some("DP-2".into()),
                    state: Some(WindowState::Maximized),
                    desktop: Some(1),
                    always_on_top: Some(true),
                    sticky: Some(false),
                    tab_group: Some("shells".into()),
                },
                WindowRule {
                    app_id: "firefox".into(),
                    title: Some("*Mail*".into()),
                    ..WindowRule::default()
                },
            ]
        );
        let d = Config::default();
        assert!(d.remember_windows && d.window_rules.is_empty());
    }

    #[test]
    fn window_tabs_are_on_unless_the_file_turns_them_off() {
        assert!(Config::default().window_tabs);
        let c = Config::parse("[windows]\ntabs = false\n").unwrap();
        assert!(!c.window_tabs, "tabs = false turns tab groups off");
        assert!(c.remember_windows, "remember keeps its default");
    }

    #[test]
    fn a_bad_window_rule_names_its_number_app_and_key() {
        let err = |text: &str| Config::parse(text).unwrap_err();
        let one =
            |body: &str| format!("[[window_rules]]\napp_id = \"x\"\n\n[[window_rules]]\n{body}");
        assert_eq!(
            err(&one("app_id = \"foot\"\nstate = \"big\"\n")),
            "window_rules #2 (\"foot\"): state \"big\" is not one of \
             normal, maximized, minimized, fullscreen"
        );
        assert_eq!(
            err(&one("app_id = \"foot\"\nsize = [0, 600]\n")),
            "window_rules #2 (\"foot\"): size must be two numbers from 1 to 32767, got [0, 600]"
        );
        assert_eq!(
            err(&one("app_id = \"foot\"\nposition = [0, 40000]\n")),
            "window_rules #2 (\"foot\"): position must be two numbers from -32768 to 32767, \
             got [0, 40000]"
        );
        assert_eq!(
            err(&one("app_id = \"foot\"\ndesktop = 0\n")),
            "window_rules #2 (\"foot\"): desktop must be between 1 and 16, got 0"
        );
        assert_eq!(
            err(&one("app_id = \"\"\n")),
            "window_rules #2 (\"\"): app_id is empty"
        );
        assert!(Config::parse("[[window_rules]]\ntitle = \"x\"\n").is_err());
        assert!(Config::parse("[[window_rules]]\napp_id = \"x\"\nsize = [1]\n").is_err());
        assert!(Config::parse("[[window_rules]]\napp_id = \"x\"\nwidth = 3\n").is_err());
        assert!(Config::parse("[windows]\nremember = 1\n").is_err());
    }

    #[test]
    fn retired_keys_still_load_and_are_named() {
        let c = Config::parse(
            "renderer = \"vulkan\"\n\n[[outputs]]\nname = \"DP-1\"\ncolor_profile = \"dell.icc\"\nhdr = \"yes\"\n\n[[outputs]]\nname = \"eDP-1\"\nhdr = true\n",
        )
        .expect("retired keys parse whatever their value");
        assert_eq!(
            c.retired,
            [
                "renderer",
                "outputs.DP-1.color_profile",
                "outputs.DP-1.hdr",
                "outputs.eDP-1.hdr"
            ]
        );
        assert!(
            Config::parse("terminal = \"foot\"\n")
                .unwrap()
                .retired
                .is_empty()
        );
    }

    #[test]
    fn a_home_path_is_absolute_or_under_home() {
        let home = Some(std::ffi::OsStr::new("/home/ann"));
        assert_eq!(
            home_path("~/Pictures/a.png", home),
            Ok(PathBuf::from("/home/ann/Pictures/a.png"))
        );
        assert_eq!(home_path("/a.png", None), Ok(PathBuf::from("/a.png")));
        assert!(home_path("~/a.png", None).is_err());
        assert!(home_path("pics/a.png", home).is_err());
        assert!(home_path("~user/a.png", home).is_err());
    }

    #[test]
    fn outputs_section_parses_and_validates_scale() {
        let c = Config::parse(
            "[[outputs]]\nname = \"DP-1\"\nscale = 2\n\n[[outputs]]\nname = \"HDMI-A-1\"\nscale = 1\n",
        )
        .unwrap();
        assert_eq!(c.outputs.len(), 2);
        assert_eq!(c.outputs[0].name, "DP-1");
        assert_eq!(c.outputs[0].scale, 2.0);
        assert_eq!(c.scale_for_output_name("DP-1"), 2.0);
        assert_eq!(c.scale_for_output_name("eDP-1"), 1.0);
        assert!(Config::default().outputs.is_empty());
        assert!(Config::parse("[[outputs]]\nname = \"DP-1\"\nscale = 0\n").is_err());
        assert!(Config::parse("[[outputs]]\nname = \"DP-1\"\nscale = 5\n").is_err());
        assert!(Config::parse("[[outputs]]\nname = \"DP-1\"\nscale = 0.5\n").is_err());
        assert!(Config::parse("[[outputs]]\nname = \"DP-1\"\nscale = 4.5\n").is_err());
        let at = |s: &str| {
            Config::parse(&format!("[[outputs]]\nname = \"DP-1\"\nscale = {s}\n"))
                .unwrap()
                .scale_for_output_name("DP-1")
        };
        assert_eq!(at("1.5"), 1.5);
        assert_eq!(at("1.75"), 1.75);
        assert_eq!(at("1.3"), 1.25, "rounded to the nearest quarter");
        assert_eq!(at("3.9"), 4.0);
        assert!(Config::parse("[[outputs]]\nscale = 2\n").is_err());
    }

    #[test]
    fn adaptive_sync_is_global_with_a_per_output_override() {
        let c = Config::default();
        assert!(!c.adaptive_sync_for_output_name("DP-1"));
        let c = Config::parse(
            "adaptive_sync = true\n\n[[outputs]]\nname = \"DP-1\"\nadaptive_sync = false\n\n[[outputs]]\nname = \"DP-2\"\n",
        )
        .unwrap();
        assert!(c.adaptive_sync);
        assert!(!c.adaptive_sync_for_output_name("DP-1"));
        assert!(c.adaptive_sync_for_output_name("DP-2"));
        assert!(c.adaptive_sync_for_output_name("HDMI-A-1"));
        let c = Config::parse("[[outputs]]\nname = \"DP-1\"\nadaptive_sync = true\n").unwrap();
        assert!(c.adaptive_sync_for_output_name("DP-1"));
        assert!(!c.adaptive_sync_for_output_name("DP-2"));
    }

    #[test]
    fn window_snapping_is_on_by_default_and_the_old_grid_keys_are_ignored() {
        let c = Config::parse("").unwrap();
        assert!(c.window_snapping.enabled, "drag snapping is on");
        assert!(c.window_snapping.picker, "the picker is on");
        assert!(c.window_snapping.layouts.is_empty(), "built-in layouts");
        let old = Config::parse(
            "[window_snapping]\nthreshold = 20\ncolumns = 2\ncolumn_weights = [1, 1]\n\
             rows = 2\nrow_weights = [1]\n\n[[outputs]]\nname = \"DP-2\"\n\
             [outputs.window_snapping]\ncolumn_weights = [1, 2, 1]\nbogus = 1\n",
        )
        .expect("the grid's keys parse and are ignored");
        assert!(old.window_snapping.enabled);
        assert!(
            Config::parse("[window_snapping]\nbogus = 1\n").is_err(),
            "a typo still fails"
        );
    }

    #[test]
    fn the_snap_bar_is_on_by_default_and_can_be_turned_off() {
        assert!(Config::parse("").unwrap().window_snapping.bar);
        let c = Config::parse("[window_snapping]\nbar = false\n").unwrap();
        assert!(!c.window_snapping.bar);
    }

    #[test]
    fn the_snap_layout_picker_reads_its_switch_and_layouts() {
        let c =
            Config::parse("[window_snapping]\npicker = false\nlayouts = [\"1|1\", \"3|1:2\"]\n")
                .unwrap();
        assert!(!c.window_snapping.picker);
        assert_eq!(c.window_snapping.layouts.len(), 2);
        assert_eq!(c.window_snapping.layouts[1].bands, vec![(3.0, 1), (1.0, 2)]);
        let bad = Config::parse("[window_snapping]\nlayouts = [\"1|x\"]\n").unwrap_err();
        assert!(bad.contains("window_snapping: layouts"), "{bad}");
        let seven = ["\"1\""; 7].join(", ");
        let many = format!("[window_snapping]\nlayouts = [{seven}]\n");
        assert!(Config::parse(&many).is_err());
    }

    #[test]
    fn the_second_dialog_font_is_optional() {
        let font = |text: &str| Config::parse(text).unwrap().theme.fonts.shell_dlg;
        assert_eq!(font(""), None);
        assert_eq!(font("[fonts]\nsize_px = 11\n"), None);
        assert_eq!(font("[fonts]\nshell_dlg = \"\"\n"), None);
        assert_eq!(
            font("[fonts]\nshell_dlg = \"Some Sans\"\n").as_deref(),
            Some("Some Sans")
        );
        assert_eq!(
            font("[fonts]\nshell_dlg = \"/opt/fonts/dlg.ttf\"\n").as_deref(),
            Some("/opt/fonts/dlg.ttf")
        );
    }

    #[test]
    fn font_size_is_range_checked_at_parse() {
        let err = |t: &str| Config::parse(t).unwrap_err();
        assert_eq!(
            err("[fonts]\nsize_px = 60000\n"),
            "fonts.size_px must be between 6 and 72, got 60000"
        );
        assert_eq!(
            err("[fonts]\nsize_px = 5\n"),
            "fonts.size_px must be between 6 and 72, got 5"
        );
        for px in [6, 72] {
            let cfg = Config::parse(&format!("[fonts]\nsize_px = {px}\n")).unwrap();
            assert_eq!(cfg.theme.fonts.size_px, px);
        }
    }

    #[test]
    fn metrics_are_range_checked_at_parse() {
        let err = |t: &str| Config::parse(t).unwrap_err();
        assert_eq!(
            err("[metrics]\ndouble_click_ms = -1\n"),
            "metrics.double_click_ms must be between 0 and 10000, got -1"
        );
        assert_eq!(
            err("[metrics]\nframe_width = 10001\n"),
            "metrics.frame_width must be between 0 and 10000, got 10001"
        );
        assert_eq!(
            err("[metrics]\ntaskbar_height = 21\n"),
            "metrics.taskbar_height must be >= 22, got 21"
        );
        assert_eq!(
            err("[metrics]\nbogus = 1\n"),
            "unknown metric name \"bogus\""
        );
        assert_eq!(
            Config::parse("[metrics]\nmenu_shadow = 0\n")
                .unwrap()
                .theme
                .metrics
                .menu_shadow,
            0
        );
    }

    #[test]
    fn output_position_and_primary_parse() {
        let c = Config::parse(
            "[[outputs]]\nname = \"DP-1\"\nposition = [-1280, 56]\n\n\
             [[outputs]]\nname = \"HDMI-A-1\"\nprimary = true\n\n\
             [[outputs]]\nname = \"DP-2\"\nprimary = true\n",
        )
        .unwrap();
        assert_eq!(c.position_for_output_name("DP-1"), Some((-1280, 56)));
        assert_eq!(c.position_for_output_name("HDMI-A-1"), None);
        assert_eq!(c.primary_output_name(), Some("HDMI-A-1"), "the first wins");
        assert_eq!(Config::parse("").unwrap().primary_output_name(), None);
        let far = Config::parse("[[outputs]]\nname = \"DP-1\"\nposition = [0, 99999999]\n");
        assert!(far.unwrap_err().contains("position must be within"));
    }

    #[test]
    fn output_transform_parses_by_wlr_randr_name() {
        let c = Config::parse("[[outputs]]\nname = \"DP-1\"\ntransform = \"90\"\n").unwrap();
        assert_eq!(c.transform_for_output_name("DP-1"), Transform::Rotate90);
        assert_eq!(c.transform_for_output_name("DP-2"), Transform::Normal);
        for (name, t) in [
            ("normal", Transform::Normal),
            ("180", Transform::Rotate180),
            ("270", Transform::Rotate270),
            ("flipped", Transform::Flipped),
            ("flipped-90", Transform::Flipped90),
            ("flipped-180", Transform::Flipped180),
            ("flipped-270", Transform::Flipped270),
        ] {
            assert_eq!(Transform::parse(name), Some(t), "{name}");
            assert_eq!(t.name(), name);
        }
        assert!(Config::parse("[[outputs]]\nname = \"DP-1\"\ntransform = \"45\"\n").is_err());
    }

    #[test]
    fn an_output_mode_prints_the_spelling_it_parses_from() {
        let mode = |width, height, refresh_mhz| OutputMode {
            width,
            height,
            refresh_mhz,
        };
        for (mode, text) in [
            (mode(1920, 1080, Some(60_000)), "1920x1080@60"),
            (mode(1920, 1080, Some(59_940)), "1920x1080@59.94"),
            (mode(2560, 1440, Some(74_973)), "2560x1440@74.973"),
            (mode(1280, 800, None), "1280x800"),
        ] {
            assert_eq!(mode.to_string(), text);
            assert_eq!(OutputMode::parse(text), Ok(mode), "{text} parses back");
        }
    }

    #[test]
    fn output_mode_parses_and_picks() {
        let c =
            Config::parse("[[outputs]]\nname = \"DP-1\"\nmode = \"1920x1080@59.94\"\n").unwrap();
        let m = c.mode_for_output_name("DP-1").unwrap();
        assert_eq!(
            (m.width, m.height, m.refresh_mhz),
            (1920, 1080, Some(59940))
        );
        assert_eq!(c.mode_for_output_name("DP-2"), None);
        assert_eq!(c.outputs[0].scale, 1.0);
        for bad in [
            "1920",
            "0x1080",
            "1920x1080@0",
            "axb",
            "1920x1080@fast",
            "1920x1080@inf",
            "1920x1080@1e9",
        ] {
            let text = format!("[[outputs]]\nname = \"DP-1\"\nmode = \"{bad}\"\n");
            assert!(Config::parse(&text).is_err(), "{bad}");
        }

        let offered = |width, height, refresh_mhz, preferred| OfferedMode {
            width,
            height,
            refresh_mhz,
            preferred,
        };
        let modes = [
            offered(2560, 1440, 144_000, true),
            offered(1920, 1080, 60_000, false),
            offered(1920, 1080, 120_000, false),
            offered(1920, 1080, 59_940, false),
        ];
        let any = OutputMode::parse("1920x1080").unwrap();
        assert_eq!(
            any.pick(&modes),
            Some(2),
            "highest refresh without a request"
        );
        assert_eq!(m.pick(&modes), Some(3), "closest refresh to the request");
        assert_eq!(
            OutputMode::parse("1920x1080@75").unwrap().pick(&modes),
            Some(1)
        );
        assert_eq!(OutputMode::parse("1280x720").unwrap().pick(&modes), None);
        let pref = [
            offered(1920, 1080, 120_000, false),
            offered(1920, 1080, 60_000, true),
        ];
        assert_eq!(any.pick(&pref), Some(1));
    }

    #[test]
    fn wallpaper_reads_image_and_position() {
        let c = Config::parse("[wallpaper]\nimage = \"/w/a.jpg\"\nposition = \"tile\"\n").unwrap();
        assert_eq!(c.wallpaper.image.as_deref(), Some(Path::new("/w/a.jpg")));
        assert_eq!(c.wallpaper.position, crate::wallpaper::Position::Tile);
    }

    #[test]
    fn wallpaper_defaults_to_none_stretched() {
        let c = Config::default();
        assert_eq!(c.wallpaper.image, None);
        assert_eq!(c.wallpaper.position, crate::wallpaper::Position::Stretch);
    }

    #[test]
    fn wallpaper_reads_a_builtin_and_fills_the_screen_with_it() {
        let c =
            Config::parse("[wallpaper]\nimage = \"reprise:terraces/frost\"\nposition = \"tile\"\n")
                .unwrap();
        assert_eq!(
            c.wallpaper.image.as_deref(),
            Some(Path::new("reprise:terraces/frost"))
        );
        assert_eq!(c.wallpaper.position, crate::wallpaper::Position::Stretch);
        let c = Config::parse("[wallpaper]\nimage = \"reprise:reeded/hue-30-dark\"\n").unwrap();
        assert_eq!(
            c.wallpaper.image.as_deref(),
            Some(Path::new("reprise:reeded/hue-30-dark"))
        );
    }

    #[test]
    fn wallpaper_refuses_a_builtin_it_does_not_have() {
        for (bad, says) in [
            ("reprise:lake", "no built-in wallpaper \"lake\""),
            ("reprise:tarn/noon", "tarn has no variant \"noon\""),
            ("reprise:tarn/hue-20", "tarn has no variant"),
            ("reprise:flow/hue-400", "flow has no variant"),
        ] {
            let e = Config::parse(&format!("[wallpaper]\nimage = \"{bad}\"\n")).unwrap_err();
            assert!(
                e.contains("wallpaper.image") && e.contains(says),
                "{bad}: {e}"
            );
        }
    }

    #[test]
    fn wallpaper_defaults_to_each_interfaces_own() {
        let image = |toml: &str| Config::parse(toml).unwrap().wallpaper.image;
        assert_eq!(image(""), None, "Classic keeps its desktop colour");
        assert_eq!(
            image("[appearance]\ninterface = \"blue\"\n"),
            Some(PathBuf::from("reprise:tarn/blue"))
        );
        let glass = image("[appearance]\ninterface = \"glass\"\n").unwrap();
        assert!(
            glass.to_str().unwrap().starts_with("reprise:flow/hue-"),
            "{glass:?}"
        );
        assert_ne!(
            image("[appearance]\ninterface = \"glass\"\n[glass]\ncolor = \"ruby\"\n"),
            Some(glass),
            "another glass colour, another tint"
        );
        assert_eq!(
            image("[appearance]\ninterface = \"glass\"\n[glass]\nmode = \"dark\"\n"),
            Some(PathBuf::from("reprise:flow/night"))
        );
        assert_eq!(
            image("[wallpaper]\nimage = \"reprise:flow/glass\"\n[glass]\nmode = \"dark\"\n"),
            Some(PathBuf::from("reprise:flow/night"))
        );
        assert_eq!(
            image("[appearance]\ninterface = \"blue\"\n[wallpaper]\nimage = \"/w/a.jpg\"\n"),
            Some(PathBuf::from("/w/a.jpg"))
        );
        assert_eq!(
            image("[appearance]\ninterface = \"glass\"\n[wallpaper]\nimage = \"\"\n"),
            None
        );
    }

    #[test]
    fn wallpaper_image_that_is_missing_still_parses() {
        let c = Config::parse("[wallpaper]\nimage = \"/nowhere/none.png\"\n").unwrap();
        assert_eq!(
            c.wallpaper.image.as_deref(),
            Some(Path::new("/nowhere/none.png"))
        );
    }

    #[test]
    fn wallpaper_image_expands_home_and_empty_is_none() {
        let home = Some(std::ffi::OsStr::new("/home/u"));
        assert_eq!(
            wallpaper_image("~/Pictures/a.png", home),
            Ok(Some(PathBuf::from("/home/u/Pictures/a.png")))
        );
        assert!(wallpaper_image("~/a.png", None).is_err());
        assert_eq!(wallpaper_image("", home), Ok(None));
        assert_eq!(
            wallpaper_image("/w/a.png", home),
            Ok(Some(PathBuf::from("/w/a.png")))
        );
        let c = Config::parse("[wallpaper]\nimage = \"\"\n").unwrap();
        assert_eq!(c.wallpaper.image, None);
    }

    #[test]
    fn wallpaper_water_parses_and_defaults_off() {
        use reprisekit::water::Water;
        let w = |toml: &str| Config::parse(toml).map(|c| c.wallpaper.water);
        assert_eq!(w(""), Ok(Water::Off));
        assert_eq!(
            w("[appearance]\ninterface = \"glass\"\n"),
            Ok(Water::Off),
            "off under Glass too"
        );
        assert_eq!(w("[wallpaper]\nwater = \"calm\"\n"), Ok(Water::Calm));
        assert_eq!(w("[wallpaper]\nwater = \"Lively\"\n"), Ok(Water::Lively));
        assert_eq!(w("[wallpaper]\nwater = \"rain\"\n"), Ok(Water::Rain));
        assert!(w("[wallpaper]\nwater = \"storm\"\n").is_err());
    }

    #[test]
    fn wallpaper_water_rate_parses_and_defaults_to_refresh() {
        use reprisekit::water::Rate;
        let r = |toml: &str| Config::parse(toml).map(|c| c.wallpaper.water_rate);
        assert_eq!(r(""), Ok(Rate::Refresh));
        assert_eq!(
            r("[appearance]\ninterface = \"glass\"\n"),
            Ok(Rate::Refresh),
            "refresh under Glass too"
        );
        assert_eq!(
            r("[wallpaper]\nwater_rate = \"Refresh\"\n"),
            Ok(Rate::Refresh)
        );
        assert_eq!(r("[wallpaper]\nwater_rate = 144\n"), Ok(Rate::Fixed(144)));
        assert_eq!(r("[wallpaper]\nwater_rate = 30\n"), Ok(Rate::Fixed(30)));
        for bad in ["29", "241", "\"fast\"", "60.5"] {
            let e = r(&format!("[wallpaper]\nwater_rate = {bad}\n")).unwrap_err();
            assert!(e.contains("wallpaper.water_rate"), "{bad}: {e}");
        }
    }

    #[test]
    fn wallpaper_lock_screen_defaults_by_interface() {
        let on = |toml: &str| Config::parse(toml).unwrap().wallpaper.lock_screen;
        assert!(!on(""), "off under Classic");
        assert!(on("[appearance]\ninterface = \"blue\"\n"), "on under Blue");
        assert!(
            on("[appearance]\ninterface = \"glass\"\n"),
            "on under Glass"
        );
        assert!(on("[wallpaper]\nlock_screen = true\n"));
        assert!(!on(
            "[appearance]\ninterface = \"glass\"\n[wallpaper]\nlock_screen = false\n"
        ));
    }

    #[test]
    fn wallpaper_rejects_an_unknown_position() {
        assert!(Config::parse("[wallpaper]\nposition = \"fill\"\n").is_err());
    }

    #[test]
    fn wallpaper_slideshow_keys() {
        let w = Config::default().wallpaper;
        assert_eq!((w.slideshow, w.interval, w.shuffle), (None, 30, false));
        let c =
            Config::parse("[wallpaper]\nslideshow = \"/srv/pics\"\ninterval = 1\nshuffle = true\n")
                .unwrap();
        assert_eq!(c.wallpaper.slideshow, Some(PathBuf::from("/srv/pics")));
        assert_eq!((c.wallpaper.interval, c.wallpaper.shuffle), (1, true));
        assert_eq!(
            Config::parse("[wallpaper]\nslideshow = \"\"\n")
                .unwrap()
                .wallpaper
                .slideshow,
            None
        );
        assert!(Config::parse("[wallpaper]\ninterval = 0\n").is_err());
    }

    #[test]
    fn screensaver_keys() {
        use reprisekit::saver::Kind;
        let d = Config::default().screensaver;
        assert_eq!(d.kind, Kind::Blank);
        assert_eq!(d.options.speed, 5);
        let c = Config::parse(
            "[screensaver]\nname = \"mystify\"\nspeed = 9\nshapes = 3\ncycle_colors = false\nfolder = \"/srv/pics\"\nphoto_seconds = 20\nstars = 50\n",
        )
        .unwrap()
        .screensaver;
        assert_eq!(c.kind, Kind::Mystify);
        assert_eq!(
            (c.options.speed, c.options.shapes, c.options.stars),
            (9, 3, 50)
        );
        assert_eq!(
            (c.options.cycle_colors, c.options.photo_seconds),
            (false, 20)
        );
        assert_eq!(c.folder, Some(PathBuf::from("/srv/pics")));
        assert!(Config::parse("[screensaver]\nname = \"toasters\"\n").is_err());
        assert!(Config::parse("[screensaver]\nspeed = 11\n").is_err());
    }
}
