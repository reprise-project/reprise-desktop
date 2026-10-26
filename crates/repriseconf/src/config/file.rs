//! The `config.toml` schema as serde reads it, before validation.

use super::{
    AccessibilityConfig, Follow, MOUSE_KEYS_ACCELERATION, MOUSE_KEYS_SPEED, NIGHT_KELVIN,
    NightLightConfig, NightSchedule, SLOW_KEYS_DELAY, TEXT_SIZE, TEXT_SIZE_STEP, parse_hhmm,
};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileFonts {
    pub family: Option<Vec<String>>,
    pub size_px: Option<u32>,
    pub antialias: Option<bool>,
    pub shell_dlg: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileKeybind {
    pub keys: String,
    pub action: String,
    pub command: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileGlobalShortcut {
    pub app: String,
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub keys: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileQuickLaunch {
    pub label: String,
    pub icon: Option<String>,
    pub command: Option<String>,
    pub action: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileMenuEntry {
    pub label: Option<String>,
    pub icon: Option<String>,
    pub command: Option<String>,
    pub action: Option<String>,
    pub separator: Option<bool>,
    pub submenu: Option<String>,
    pub items: Option<Vec<FileMenuEntry>>,
    pub tip: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileKeyboard {
    pub rules: Option<String>,
    pub model: Option<String>,
    pub layout: Option<String>,
    pub variant: Option<String>,
    pub options: Option<String>,
    pub repeat_rate: Option<i32>,
    pub repeat_delay: Option<i32>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileMouse {
    pub speed: Option<f64>,
    pub acceleration: Option<bool>,
    pub natural_scroll: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileTouchpad {
    pub speed: Option<f64>,
    pub acceleration: Option<bool>,
    pub natural_scroll: Option<bool>,
    pub tap_to_click: Option<bool>,
    pub two_finger_scroll: Option<bool>,
    pub two_finger_tap_right_click: Option<bool>,
    pub disable_while_typing: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileTouchscreen {
    pub output: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileIdle {
    pub lock_after: Option<i64>,
    pub screen_off_after: Option<i64>,
    pub blank_after: Option<i64>,
    pub suspend_after: Option<i64>,
    pub lock_fade: Option<i64>,
    pub battery: Option<FileIdleBattery>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileIdleBattery {
    pub lock_after: Option<i64>,
    pub screen_off_after: Option<i64>,
    pub blank_after: Option<i64>,
    pub suspend_after: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilePower {
    pub standby: Option<String>,
    pub poweroff: Option<String>,
    pub reboot: Option<String>,
    pub hibernate: Option<String>,
    pub lid_close: Option<String>,
    pub power_button: Option<String>,
    pub sleep_button: Option<String>,
    pub battery_low: Option<i64>,
    pub battery_critical: Option<i64>,
    pub critical_action: Option<String>,
    pub always_show_icon: Option<bool>,
    pub lock_on_sleep: Option<bool>,
    pub locker: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileOutput {
    pub name: String,
    pub scale: Option<f64>,
    pub mode: Option<String>,
    pub adaptive_sync: Option<bool>,
    pub transform: Option<String>,
    /// An output's snap grid, read only to be ignored.
    #[allow(dead_code)]
    pub window_snapping: Option<serde::de::IgnoredAny>,
    pub position: Option<[i64; 2]>,
    pub primary: Option<bool>,
    /// Retired: the renderer is OpenGL ES and drives no HDR or ICC
    /// profile. Read only to name it in [`Config::warn_retired`].
    pub color_profile: Option<serde::de::IgnoredAny>,
    pub hdr: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileSnapping {
    pub enabled: Option<bool>,
    pub picker: Option<bool>,
    pub bar: Option<bool>,
    pub layouts: Option<Vec<String>>,
    pub distance: Option<i64>,
    /// The snap grid's keys, read only to be ignored: the grid is gone.
    #[allow(dead_code)]
    pub threshold: Option<serde::de::IgnoredAny>,
    #[allow(dead_code)]
    pub columns: Option<serde::de::IgnoredAny>,
    #[allow(dead_code)]
    pub rows: Option<serde::de::IgnoredAny>,
    #[allow(dead_code)]
    pub column_weights: Option<serde::de::IgnoredAny>,
    #[allow(dead_code)]
    pub row_weights: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileScreenshot {
    pub save: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FilePreviews {
    pub taskbar: Option<bool>,
    pub fps: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileOverview {
    pub hot_corner: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileGestures {
    pub enabled: Option<bool>,
    pub fingers: Option<i64>,
    pub distance: Option<i64>,
    pub commit_at: Option<i64>,
    pub flick_speed: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileSwitcher {
    pub style: Option<String>,
    pub size: Option<String>,
    pub preview: Option<bool>,
    pub desktop: Option<bool>,
    pub peek: Option<bool>,
    pub flip: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FilePrivacy {
    pub recent_files: Option<bool>,
    pub location: Option<bool>,
    pub capture_border: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileDebug {
    pub overlay: Option<bool>,
    pub overlay_in_captures: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileLauncher {
    pub search_files: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileClipboard {
    pub history: Option<bool>,
    pub terminal_apps: Option<Vec<String>>,
    pub trusted_apps: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileWorkspaces {
    pub count: Option<i64>,
    pub animation: Option<String>,
    pub animation_ms: Option<i64>,
    pub pager: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileEffects {
    pub window_animation: Option<bool>,
    pub window_open_ms: Option<i64>,
    pub window_close_ms: Option<i64>,
    pub window_scale: Option<f64>,
    pub hover_fade: Option<bool>,
    pub hover_in_ms: Option<i64>,
    pub hover_out_ms: Option<i64>,
    pub taskbar_slide: Option<bool>,
    pub taskbar_hide_ms: Option<i64>,
    pub menu_animation: Option<String>,
    pub session_fade: Option<bool>,
    pub session_fade_in_ms: Option<i64>,
    pub session_fade_out_ms: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileWindows {
    pub remember: Option<bool>,
    pub tabs: Option<bool>,
    pub shake: Option<AutoFlag>,
}

/// A key that takes true, false or "auto" (the interface's default).
#[derive(Deserialize)]
#[serde(untagged)]
pub enum AutoFlag {
    Flag(bool),
    Word(String),
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileSession {
    pub restore: Option<bool>,
    pub exclude: Option<Vec<String>>,
    pub end_processes: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileWindowRule {
    pub app_id: String,
    pub title: Option<String>,
    pub size: Option<[i64; 2]>,
    pub position: Option<[i64; 2]>,
    pub output: Option<String>,
    pub state: Option<String>,
    pub desktop: Option<i64>,
    pub always_on_top: Option<bool>,
    pub sticky: Option<bool>,
    pub tab_group: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileNightLight {
    pub enabled: Option<bool>,
    pub temperature: Option<i64>,
    pub schedule: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

impl FileNightLight {
    pub(super) fn apply(self, cfg: &mut NightLightConfig) -> Result<(), String> {
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
            cfg.schedule = NightSchedule::parse(&s)
                .ok_or_else(|| format!("night_light.schedule {s:?} is not \"sun\" or \"fixed\""))?;
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
pub struct FileAccessibility {
    pub sticky_keys: Option<bool>,
    pub sticky_lock: Option<bool>,
    pub sticky_two_keys_off: Option<bool>,
    pub slow_keys: Option<bool>,
    pub slow_keys_delay: Option<f64>,
    pub shortcuts: Option<bool>,
    pub color_filter: Option<String>,
    pub color_filter_intensity: Option<i64>,
    pub magnifier_follow: Option<String>,
    pub mouse_keys: Option<bool>,
    pub mouse_keys_speed: Option<u32>,
    pub mouse_keys_acceleration: Option<f64>,
    pub mouse_keys_num_lock: Option<bool>,
    pub text_size: Option<u32>,
}

impl FileAccessibility {
    pub(super) fn apply(self, cfg: &mut AccessibilityConfig) -> Result<(), String> {
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
            cfg.magnifier_follow = Follow::parse(name).ok_or_else(|| {
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
pub struct FileIcons {
    pub theme: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileToolkits {
    pub gtk_theme: Option<String>,
    pub qt_platform_theme: Option<String>,
    pub font_smoothing: Option<String>,
    pub gtk4_colors: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileCursor {
    pub theme: Option<String>,
    pub size: Option<u32>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileAppearance {
    pub interface: Option<String>,
    pub color_scheme: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileGlass {
    pub color: Option<String>,
    pub intensity: Option<u8>,
    pub transparency: Option<bool>,
    pub blur: Option<u8>,
    pub streaks: Option<bool>,
    pub fast_drag: Option<bool>,
    pub blur_frames: Option<bool>,
    pub blur_rate: Option<u16>,
    pub orb_glow: Option<u8>,
    pub mode: Option<String>,
    pub material: Option<String>,
    pub frame: Option<String>,
    pub notification_style: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileTaskbar {
    pub position: Option<String>,
    pub buttons: Option<String>,
    pub pinned: Option<bool>,
    pub small_icons: Option<bool>,
    pub show_desktop_button: Option<bool>,
    pub peek: Option<bool>,
    pub tray_overflow: Option<bool>,
    pub auto_hide: Option<bool>,
    pub reveal_on_edge: Option<bool>,
    pub reveal_on_keys: Option<bool>,
    pub reveal_on_attention: Option<bool>,
    pub attention_flash: Option<i64>,
    pub side_width: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileWallpaper {
    pub image: Option<String>,
    pub position: Option<String>,
    pub lock_screen: Option<bool>,
    pub slideshow: Option<String>,
    pub interval: Option<i64>,
    pub shuffle: Option<bool>,
    pub water: Option<String>,
    pub water_rate: Option<toml::Value>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileScreensaver {
    pub name: Option<String>,
    pub speed: Option<i64>,
    pub stars: Option<i64>,
    pub shapes: Option<i64>,
    pub cycle_colors: Option<bool>,
    pub folder: Option<String>,
    pub photo_seconds: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub terminal: Option<String>,
    pub script_terminal: Option<String>,
    pub script_open: Option<String>,
    pub task_manager: Option<String>,
    /// Retired, as `FileOutput::hdr`.
    pub renderer: Option<serde::de::IgnoredAny>,
    pub colors: Option<HashMap<String, String>>,
    pub metrics: Option<HashMap<String, i32>>,
    pub fonts: Option<FileFonts>,
    pub keybinds: Option<Vec<FileKeybind>>,
    pub global_shortcuts: Option<Vec<FileGlobalShortcut>>,
    /// `[portal]` is read by reprise-portal; accepted here so the file parses.
    #[allow(dead_code)]
    pub portal: Option<serde::de::IgnoredAny>,
    /// `[recycle_bin]` is read by `reprisefs::bincap::Policy`; accepted here so the file parses.
    #[allow(dead_code)]
    pub recycle_bin: Option<serde::de::IgnoredAny>,
    /// `[drives]` is read by reprise-drives; accepted here so the file parses.
    #[allow(dead_code)]
    pub drives: Option<serde::de::IgnoredAny>,
    /// `[thumbnail_cache]` is read by `crate::thumbnail_cache`; accepted here so the file parses.
    #[allow(dead_code)]
    pub thumbnail_cache: Option<serde::de::IgnoredAny>,
    pub startmenu: Option<Vec<FileMenuEntry>>,
    pub quicklaunch: Option<Vec<FileQuickLaunch>>,
    pub startmenu_banner: Option<String>,
    pub startmenu_style: Option<String>,
    pub keyboard: Option<FileKeyboard>,
    pub mouse: Option<FileMouse>,
    pub touchpad: Option<FileTouchpad>,
    pub touchscreen: Option<FileTouchscreen>,
    pub idle: Option<FileIdle>,
    pub power: Option<FilePower>,
    pub desktop_icons: Option<bool>,
    pub desktop_extensions: Option<String>,
    pub start_icon: Option<bool>,
    pub system_icons: Option<Vec<String>>,
    pub wallpaper: Option<FileWallpaper>,
    pub screensaver: Option<FileScreensaver>,
    pub outputs: Option<Vec<FileOutput>>,
    pub adaptive_sync: Option<bool>,
    pub window_snapping: Option<FileSnapping>,
    pub screenshot: Option<FileScreenshot>,
    pub previews: Option<FilePreviews>,
    pub switcher: Option<FileSwitcher>,
    pub overview: Option<FileOverview>,
    pub gestures: Option<FileGestures>,
    pub clipboard: Option<FileClipboard>,
    pub privacy: Option<FilePrivacy>,
    pub debug: Option<FileDebug>,
    pub launcher: Option<FileLauncher>,
    pub night_light: Option<FileNightLight>,
    pub workspaces: Option<FileWorkspaces>,
    pub effects: Option<FileEffects>,
    pub accessibility: Option<FileAccessibility>,
    pub windows: Option<FileWindows>,
    pub session: Option<FileSession>,
    pub window_rules: Option<Vec<FileWindowRule>>,
    /// Deserialized by the compositor, whose types the pads' bindings are.
    pub tablet_pads: Option<Vec<toml::Value>>,
    pub icons: Option<FileIcons>,
    pub cursor: Option<FileCursor>,
    pub toolkits: Option<FileToolkits>,
    pub appearance: Option<FileAppearance>,
    pub glass: Option<FileGlass>,
    pub taskbar: Option<FileTaskbar>,
}
