//! `config.toml`: parsing, validation and defaults for every section, read by the compositor and
//! by reprise-settings alike. The compositor adds the parts built from its own types (binds, menus,
//! window rules, tablet pads) on top.

mod accessibility;
mod appearance;
mod desktop;
mod effects;
pub mod file;
mod input;
mod output;
mod parse;
mod power;

#[cfg(test)]
mod tests;

pub use accessibility::*;
pub use appearance::*;
pub use desktop::*;
pub use effects::*;
pub use input::*;
pub use output::*;
pub use parse::{Dropped, DroppedKey, lenient, lenient_over};
pub use power::*;
use reprisekit::interface::Interface;
use reprisekit::theme::Theme;

/// `config.toml` validated, every key the file leaves out at its default.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub terminal: String,
    pub script_terminal: reprisefs::program::ScriptEnd,
    pub script_open: reprisefs::program::ScriptOpen,
    pub confirm_scripts: bool,
    pub task_manager: Option<String>,
    /// Retired keys (`renderer`, `outputs.<name>.hdr`, `.color_profile`), kept so
    /// an older file loads; [`Config::warn_retired`] names them.
    pub retired: Vec<String>,
    /// Includes `[accessibility] text_size`.
    pub theme: Theme,
    pub startmenu_banner: String,
    pub startmenu_style: StartMenuStyle,
    pub keyboard: KeyboardConfig,
    pub mouse: PointerConfig,
    pub touchpad: TouchpadConfig,
    pub touchscreen: TouchscreenConfig,
    pub tablets: Vec<TabletConfig>,
    pub idle: IdleConfig,
    pub idle_battery: IdleOverrides,
    pub power: PowerConfig,
    pub lock: LockConfig,
    pub desktop_icons: bool,
    pub desktop_extensions: reprisefs::names::Extensions,
    pub start_icon: bool,
    /// The desktop's system icons, in order: `documents`, `computer`, `home`,
    /// `recycle-bin`. Old `my-documents`/`my-computer` read as the new names.
    pub system_icons: Vec<String>,
    /// The widgets standing on the desktop, each once, in the order listed.
    pub desktop_widgets: Vec<reprisekit::controls::deskwidget::Kind>,
    pub wallpaper: WallpaperConfig,
    pub screensaver: ScreensaverConfig,
    /// `[display] gpu`: the DRM device whose connectors are driven, read once at startup.
    pub gpu: Option<std::path::PathBuf>,
    /// `[display] all_gpus`: open every GPU of the seat, not only the one that draws; read once
    /// at startup.
    pub all_gpus: bool,
    pub outputs: Vec<OutputConfig>,
    pub adaptive_sync: bool,
    pub window_snapping: SnapConfig,
    pub screenshot: ScreenshotConfig,
    pub previews: PreviewsConfig,
    pub osd: OsdConfig,
    pub switcher: SwitcherConfig,
    pub hot_corner: HotCorner,
    pub gestures: GesturesConfig,
    pub clipboard: ClipboardConfig,
    pub notifications: NotificationsConfig,
    pub sounds: SoundsConfig,
    pub recent_files: bool,
    pub location: bool,
    pub capture_border: bool,
    /// `[remote_desktop]`; the compositor reads its `capture_border`.
    pub remote_desktop: crate::remote::RemoteDesktop,
    /// `[updates] restart_notice`: Software Updates says when the running system needs a restart.
    pub restart_notice: bool,
    /// `[time_zone] automatic`: the compositor sets the time zone from the location.
    pub auto_time_zone: bool,
    pub debug: DebugConfig,
    pub shortcut_overlay: ShortcutOverlay,
    pub launcher_files: bool,
    /// `[launcher] windows`: the launcher finds open windows.
    pub launcher_windows: bool,
    /// `[launcher] providers`: programs' search providers by desktop id, in the order their
    /// sections show ([`reprisefs::searchproviders::arrange`]).
    pub launcher_providers: Vec<String>,
    /// `[launcher] providers_off`: the providers never asked.
    pub launcher_providers_off: Vec<String>,
    pub night_light: NightLightConfig,
    pub workspaces: usize,
    /// `[workspaces] names`: entry n names desktop n; see [`desktop_name`].
    pub workspace_names: Vec<String>,
    pub workspace_animation: SwitchAnimation,
    pub pager_mark: reprisekit::taskbar::PagerMark,
    /// `[workspaces] wheel_on_desktop`: the wheel over the desktop steps desktops as over the pager.
    pub wheel_on_desktop: bool,
    pub effects: EffectsConfig,
    pub accessibility: AccessibilityConfig,
    pub remember_windows: bool,
    pub window_tabs: bool,
    /// `[windows] shake`, resolved against the interface.
    pub window_shake: bool,
    pub title_double_click: TitleDoubleClick,
    /// `[windows] dim_inactive`: how much darker, in percent, a window without the focus draws.
    pub dim_inactive: u8,
    /// `[windows] hidden_frame_rate`: frames a second at most for a fifo or timed commit no
    /// output shows; 0 is the refresh rate.
    pub hidden_frame_rate: u32,
    pub session: SessionConfig,
    pub icons: IconsConfig,
    pub cursor: CursorConfig,
    pub toolkits: ToolkitsConfig,
    pub interface: Interface,
    pub color_scheme: Option<String>,
    pub blue_variant: reprisekit::blue::Variant,
    pub glass: reprisekit::glass::Settings,
    /// `glass.mode` is the mode in force; under a schedule the compositor sets it (`darkmode`).
    pub glass_dark: GlassDark,
    pub taskbar: TaskbarConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            terminal: reprisefs::apps::DEFAULT_TERMINAL.into(),
            script_terminal: reprisefs::program::ScriptEnd::Stay,
            script_open: reprisefs::program::ScriptOpen::Run,
            confirm_scripts: true,
            task_manager: None,
            retired: Vec::new(),
            theme: Theme::default(),
            startmenu_banner: "Reprise Desktop".into(),
            startmenu_style: StartMenuStyle::Classic,
            keyboard: KeyboardConfig::default(),
            mouse: PointerConfig::default(),
            touchpad: TouchpadConfig::default(),
            touchscreen: TouchscreenConfig::default(),
            tablets: Vec::new(),
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
            desktop_widgets: Vec::new(),
            wallpaper: WallpaperConfig::default(),
            screensaver: ScreensaverConfig::default(),
            gpu: None,
            all_gpus: false,
            outputs: Vec::new(),
            adaptive_sync: false,
            window_snapping: SnapConfig::default(),
            screenshot: ScreenshotConfig::default(),
            previews: PreviewsConfig::default(),
            osd: OsdConfig::default(),
            lock: LockConfig::default(),
            switcher: SwitcherConfig::default_for(Interface::Classic),
            hot_corner: HotCorner::None,
            gestures: GesturesConfig::default(),
            clipboard: ClipboardConfig::default(),
            notifications: NotificationsConfig::default(),
            sounds: SoundsConfig::default(),
            recent_files: true,
            location: true,
            capture_border: true,
            remote_desktop: crate::remote::RemoteDesktop::default(),
            restart_notice: true,
            auto_time_zone: false,
            debug: DebugConfig::default(),
            shortcut_overlay: ShortcutOverlay::default(),
            launcher_files: true,
            launcher_windows: true,
            launcher_providers: Vec::new(),
            launcher_providers_off: Vec::new(),
            night_light: NightLightConfig::default(),
            workspaces: WORKSPACES_DEFAULT,
            workspace_names: Vec::new(),
            workspace_animation: SwitchAnimation::default(),
            pager_mark: reprisekit::taskbar::PagerMark::default(),
            wheel_on_desktop: false,
            effects: EffectsConfig::default_for(Interface::Classic),
            accessibility: AccessibilityConfig::default(),
            remember_windows: true,
            window_tabs: true,
            window_shake: false,
            title_double_click: TitleDoubleClick::Maximize,
            dim_inactive: 0,
            hidden_frame_rate: 0,
            session: SessionConfig::default(),
            icons: IconsConfig::default(),
            cursor: CursorConfig::default(),
            toolkits: ToolkitsConfig::default(),
            interface: Interface::Classic,
            color_scheme: None,
            blue_variant: reprisekit::blue::Variant::Blue,
            glass: reprisekit::glass::Settings::default(),
            glass_dark: GlassDark::default(),
            taskbar: TaskbarConfig::default_for(Interface::Classic),
        }
    }
}

impl Config {
    /// How menus open: None when `menu_fade_ms` is 0 whatever `[effects] menu_animation` says.
    pub fn menu_opening(&self) -> MenuAnimation {
        match self.theme.metrics.menu_fade_ms {
            0 => MenuAnimation::None,
            _ => self.effects.menu_animation,
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

    fn output(&self, name: &str) -> Option<&OutputConfig> {
        self.outputs.iter().find(|o| o.name == name)
    }

    pub fn scale_for_output_name(&self, name: &str) -> f32 {
        self.output(name).map(|o| o.scale).unwrap_or(1.0)
    }

    pub fn mode_for_output_name(&self, name: &str) -> Option<OutputMode> {
        self.output(name).and_then(|o| o.mode)
    }

    pub fn adaptive_sync_for_output_name(&self, name: &str) -> bool {
        self.output(name)
            .and_then(|o| o.adaptive_sync)
            .unwrap_or(self.adaptive_sync)
    }

    pub fn position_for_output_name(&self, name: &str) -> Option<(i32, i32)> {
        self.output(name).and_then(|o| o.position)
    }

    pub fn primary_output_name(&self) -> Option<&str> {
        self.outputs
            .iter()
            .find(|o| o.primary)
            .map(|o| o.name.as_str())
    }

    pub fn transform_for_output_name(&self, name: &str) -> Transform {
        self.output(name).map(|o| o.transform).unwrap_or_default()
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
