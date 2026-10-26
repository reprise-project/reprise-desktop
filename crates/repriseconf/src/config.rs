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
pub use parse::lenient;
pub use power::*;
use reprisekit::interface::Interface;
use reprisekit::theme::Theme;

/// `config.toml` validated, every key the file leaves out at its default.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub terminal: String,
    pub script_terminal: reprisefs::program::ScriptEnd,
    pub script_open: reprisefs::program::ScriptOpen,
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
    pub capture_border: bool,
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
    /// `[windows] shake`, resolved against the interface.
    pub window_shake: bool,
    pub session: SessionConfig,
    pub icons: IconsConfig,
    pub cursor: CursorConfig,
    pub toolkits: ToolkitsConfig,
    pub interface: Interface,
    pub color_scheme: Option<String>,
    pub blue_variant: reprisekit::blue::Variant,
    pub glass: reprisekit::glass::Settings,
    pub taskbar: TaskbarConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            terminal: reprisefs::apps::DEFAULT_TERMINAL.into(),
            script_terminal: reprisefs::program::ScriptEnd::Stay,
            script_open: reprisefs::program::ScriptOpen::Run,
            task_manager: None,
            retired: Vec::new(),
            theme: Theme::default(),
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
            capture_border: true,
            debug: DebugConfig::default(),
            launcher_files: true,
            night_light: NightLightConfig::default(),
            workspaces: WORKSPACES_DEFAULT,
            workspace_animation: SwitchAnimation::default(),
            pager_mark: reprisekit::taskbar::PagerMark::default(),
            effects: EffectsConfig::default_for(Interface::Classic),
            accessibility: AccessibilityConfig::default(),
            remember_windows: true,
            window_tabs: true,
            window_shake: false,
            session: SessionConfig::default(),
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
