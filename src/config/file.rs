//! The `config.toml` schema as serde reads it, before validation.

use super::{
    AccessibilityConfig, Action, MOUSE_KEYS_ACCELERATION, MOUSE_KEYS_SPEED, MenuEntry, MenuTarget,
    NIGHT_KELVIN, NightLightConfig, NightSchedule, QuickLaunchEntry, SLOW_KEYS_DELAY, TEXT_SIZE,
    TEXT_SIZE_STEP, WORKSPACES_MAX, parse_hhmm,
};
use crate::windowrules::{WindowRule, WindowState};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileFonts {
    pub(super) family: Option<Vec<String>>,
    pub(super) size_px: Option<u32>,
    pub(super) antialias: Option<bool>,
    pub(super) shell_dlg: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileKeybind {
    pub(super) keys: String,
    pub(super) action: String,
    pub(super) command: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileGlobalShortcut {
    pub(super) app: String,
    pub(super) id: String,
    #[serde(default)]
    pub(super) description: String,
    #[serde(default)]
    pub(super) keys: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileQuickLaunch {
    pub(super) label: String,
    pub(super) icon: Option<String>,
    pub(super) command: Option<String>,
    pub(super) action: Option<String>,
}

impl FileQuickLaunch {
    pub(super) fn parse(&self) -> Result<QuickLaunchEntry, String> {
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
pub(super) struct FileMenuEntry {
    pub(super) label: Option<String>,
    pub(super) icon: Option<String>,
    pub(super) command: Option<String>,
    pub(super) action: Option<String>,
    pub(super) separator: Option<bool>,
    pub(super) submenu: Option<String>,
    pub(super) items: Option<Vec<FileMenuEntry>>,
    pub(super) tip: Option<String>,
}

impl FileMenuEntry {
    pub(super) fn parse(&self) -> Result<MenuEntry, String> {
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
pub(super) struct FileKeyboard {
    pub(super) rules: Option<String>,
    pub(super) model: Option<String>,
    pub(super) layout: Option<String>,
    pub(super) variant: Option<String>,
    pub(super) options: Option<String>,
    pub(super) repeat_rate: Option<i32>,
    pub(super) repeat_delay: Option<i32>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileMouse {
    pub(super) speed: Option<f64>,
    pub(super) acceleration: Option<bool>,
    pub(super) natural_scroll: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileTouchpad {
    pub(super) speed: Option<f64>,
    pub(super) acceleration: Option<bool>,
    pub(super) natural_scroll: Option<bool>,
    pub(super) tap_to_click: Option<bool>,
    pub(super) two_finger_scroll: Option<bool>,
    pub(super) two_finger_tap_right_click: Option<bool>,
    pub(super) disable_while_typing: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileTouchscreen {
    pub(super) output: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileIdle {
    pub(super) lock_after: Option<i64>,
    pub(super) screen_off_after: Option<i64>,
    pub(super) blank_after: Option<i64>,
    pub(super) suspend_after: Option<i64>,
    pub(super) lock_fade: Option<i64>,
    pub(super) battery: Option<FileIdleBattery>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileIdleBattery {
    pub(super) lock_after: Option<i64>,
    pub(super) screen_off_after: Option<i64>,
    pub(super) blank_after: Option<i64>,
    pub(super) suspend_after: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FilePower {
    pub(super) standby: Option<String>,
    pub(super) poweroff: Option<String>,
    pub(super) reboot: Option<String>,
    pub(super) hibernate: Option<String>,
    pub(super) lid_close: Option<String>,
    pub(super) power_button: Option<String>,
    pub(super) sleep_button: Option<String>,
    pub(super) battery_low: Option<i64>,
    pub(super) battery_critical: Option<i64>,
    pub(super) critical_action: Option<String>,
    pub(super) always_show_icon: Option<bool>,
    pub(super) lock_on_sleep: Option<bool>,
    pub(super) locker: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileOutput {
    pub(super) name: String,
    pub(super) scale: Option<f64>,
    pub(super) mode: Option<String>,
    pub(super) adaptive_sync: Option<bool>,
    pub(super) transform: Option<String>,
    /// An output's snap grid, read only to be ignored.
    #[allow(dead_code)]
    pub(super) window_snapping: Option<serde::de::IgnoredAny>,
    pub(super) position: Option<[i64; 2]>,
    pub(super) primary: Option<bool>,
    /// Retired: the renderer is OpenGL ES and drives no HDR or ICC
    /// profile. Read only to name it in [`Config::warn_retired`].
    pub(super) color_profile: Option<serde::de::IgnoredAny>,
    pub(super) hdr: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileSnapping {
    pub(super) enabled: Option<bool>,
    pub(super) picker: Option<bool>,
    pub(super) bar: Option<bool>,
    pub(super) layouts: Option<Vec<String>>,
    pub(super) distance: Option<i64>,
    /// The snap grid's keys, read only to be ignored: the grid is gone.
    #[allow(dead_code)]
    pub(super) threshold: Option<serde::de::IgnoredAny>,
    #[allow(dead_code)]
    pub(super) columns: Option<serde::de::IgnoredAny>,
    #[allow(dead_code)]
    pub(super) rows: Option<serde::de::IgnoredAny>,
    #[allow(dead_code)]
    pub(super) column_weights: Option<serde::de::IgnoredAny>,
    #[allow(dead_code)]
    pub(super) row_weights: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileScreenshot {
    pub(super) save: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FilePreviews {
    pub(super) taskbar: Option<bool>,
    pub(super) fps: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileOverview {
    pub(super) hot_corner: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileGestures {
    pub(super) enabled: Option<bool>,
    pub(super) fingers: Option<i64>,
    pub(super) distance: Option<i64>,
    pub(super) commit_at: Option<i64>,
    pub(super) flick_speed: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileSwitcher {
    pub(super) style: Option<String>,
    pub(super) size: Option<String>,
    pub(super) preview: Option<bool>,
    pub(super) desktop: Option<bool>,
    pub(super) peek: Option<bool>,
    pub(super) flip: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FilePrivacy {
    pub(super) recent_files: Option<bool>,
    pub(super) location: Option<bool>,
    pub(super) capture_border: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileDebug {
    pub(super) overlay: Option<bool>,
    pub(super) overlay_in_captures: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileLauncher {
    pub(super) search_files: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileClipboard {
    pub(super) history: Option<bool>,
    pub(super) terminal_apps: Option<Vec<String>>,
    pub(super) trusted_apps: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileWorkspaces {
    pub(super) count: Option<i64>,
    pub(super) animation: Option<String>,
    pub(super) animation_ms: Option<i64>,
    pub(super) pager: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileEffects {
    pub(super) window_animation: Option<bool>,
    pub(super) window_open_ms: Option<i64>,
    pub(super) window_close_ms: Option<i64>,
    pub(super) window_scale: Option<f64>,
    pub(super) hover_fade: Option<bool>,
    pub(super) hover_in_ms: Option<i64>,
    pub(super) hover_out_ms: Option<i64>,
    pub(super) taskbar_slide: Option<bool>,
    pub(super) taskbar_hide_ms: Option<i64>,
    pub(super) menu_animation: Option<String>,
    pub(super) session_fade: Option<bool>,
    pub(super) session_fade_in_ms: Option<i64>,
    pub(super) session_fade_out_ms: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileWindows {
    pub(super) remember: Option<bool>,
    pub(super) tabs: Option<bool>,
    pub(super) shake: Option<AutoFlag>,
}

/// A key that takes true, false or "auto" (the interface's default).
#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum AutoFlag {
    Flag(bool),
    Word(String),
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileSession {
    pub(super) restore: Option<bool>,
    pub(super) exclude: Option<Vec<String>>,
    pub(super) end_processes: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileWindowRule {
    pub(super) app_id: String,
    pub(super) title: Option<String>,
    pub(super) size: Option<[i64; 2]>,
    pub(super) position: Option<[i64; 2]>,
    pub(super) output: Option<String>,
    pub(super) state: Option<String>,
    pub(super) desktop: Option<i64>,
    pub(super) always_on_top: Option<bool>,
    pub(super) sticky: Option<bool>,
    pub(super) tab_group: Option<String>,
}

impl FileWindowRule {
    pub(super) fn parse(self, at: &str) -> Result<WindowRule, String> {
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

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileNightLight {
    pub(super) enabled: Option<bool>,
    pub(super) temperature: Option<i64>,
    pub(super) schedule: Option<String>,
    pub(super) from: Option<String>,
    pub(super) to: Option<String>,
    pub(super) latitude: Option<f64>,
    pub(super) longitude: Option<f64>,
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
pub(super) struct FileAccessibility {
    pub(super) sticky_keys: Option<bool>,
    pub(super) sticky_lock: Option<bool>,
    pub(super) sticky_two_keys_off: Option<bool>,
    pub(super) slow_keys: Option<bool>,
    pub(super) slow_keys_delay: Option<f64>,
    pub(super) shortcuts: Option<bool>,
    pub(super) color_filter: Option<String>,
    pub(super) color_filter_intensity: Option<i64>,
    pub(super) magnifier_follow: Option<String>,
    pub(super) mouse_keys: Option<bool>,
    pub(super) mouse_keys_speed: Option<u32>,
    pub(super) mouse_keys_acceleration: Option<f64>,
    pub(super) mouse_keys_num_lock: Option<bool>,
    pub(super) text_size: Option<u32>,
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
pub(super) struct FileIcons {
    pub(super) theme: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileToolkits {
    pub(super) gtk_theme: Option<String>,
    pub(super) qt_platform_theme: Option<String>,
    pub(super) font_smoothing: Option<String>,
    pub(super) gtk4_colors: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileCursor {
    pub(super) theme: Option<String>,
    pub(super) size: Option<u32>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileAppearance {
    pub(super) interface: Option<String>,
    pub(super) color_scheme: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileGlass {
    pub(super) color: Option<String>,
    pub(super) intensity: Option<u8>,
    pub(super) transparency: Option<bool>,
    pub(super) blur: Option<u8>,
    pub(super) streaks: Option<bool>,
    pub(super) fast_drag: Option<bool>,
    pub(super) blur_frames: Option<bool>,
    pub(super) blur_rate: Option<u16>,
    pub(super) orb_glow: Option<u8>,
    pub(super) mode: Option<String>,
    pub(super) material: Option<String>,
    pub(super) frame: Option<String>,
    pub(super) notification_style: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileTaskbar {
    pub(super) position: Option<String>,
    pub(super) buttons: Option<String>,
    pub(super) pinned: Option<bool>,
    pub(super) small_icons: Option<bool>,
    pub(super) show_desktop_button: Option<bool>,
    pub(super) peek: Option<bool>,
    pub(super) tray_overflow: Option<bool>,
    pub(super) auto_hide: Option<bool>,
    pub(super) reveal_on_edge: Option<bool>,
    pub(super) reveal_on_keys: Option<bool>,
    pub(super) reveal_on_attention: Option<bool>,
    pub(super) attention_flash: Option<i64>,
    pub(super) side_width: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileWallpaper {
    pub(super) image: Option<String>,
    pub(super) position: Option<String>,
    pub(super) lock_screen: Option<bool>,
    pub(super) slideshow: Option<String>,
    pub(super) interval: Option<i64>,
    pub(super) shuffle: Option<bool>,
    pub(super) water: Option<String>,
    pub(super) water_rate: Option<toml::Value>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileScreensaver {
    pub(super) name: Option<String>,
    pub(super) speed: Option<i64>,
    pub(super) stars: Option<i64>,
    pub(super) shapes: Option<i64>,
    pub(super) cycle_colors: Option<bool>,
    pub(super) folder: Option<String>,
    pub(super) photo_seconds: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FileConfig {
    pub(super) terminal: Option<String>,
    pub(super) script_terminal: Option<String>,
    pub(super) script_open: Option<String>,
    pub(super) task_manager: Option<String>,
    /// Retired, as `FileOutput::hdr`.
    pub(super) renderer: Option<serde::de::IgnoredAny>,
    pub(super) colors: Option<HashMap<String, String>>,
    pub(super) metrics: Option<HashMap<String, i32>>,
    pub(super) fonts: Option<FileFonts>,
    pub(super) keybinds: Option<Vec<FileKeybind>>,
    pub(super) global_shortcuts: Option<Vec<FileGlobalShortcut>>,
    /// `[portal]` is read by reprise-portal; accepted here so the file parses.
    #[allow(dead_code)]
    pub(super) portal: Option<serde::de::IgnoredAny>,
    /// `[recycle_bin]` is read by `reprisefs::bincap::Policy`; accepted here so the file parses.
    #[allow(dead_code)]
    pub(super) recycle_bin: Option<serde::de::IgnoredAny>,
    /// `[drives]` is read by reprise-drives; accepted here so the file parses.
    #[allow(dead_code)]
    pub(super) drives: Option<serde::de::IgnoredAny>,
    /// `[thumbnail_cache]` is read by `repriseconf::thumbnail_cache`; accepted here so the file parses.
    #[allow(dead_code)]
    pub(super) thumbnail_cache: Option<serde::de::IgnoredAny>,
    pub(super) startmenu: Option<Vec<FileMenuEntry>>,
    pub(super) quicklaunch: Option<Vec<FileQuickLaunch>>,
    pub(super) startmenu_banner: Option<String>,
    pub(super) startmenu_style: Option<String>,
    pub(super) keyboard: Option<FileKeyboard>,
    pub(super) mouse: Option<FileMouse>,
    pub(super) touchpad: Option<FileTouchpad>,
    pub(super) touchscreen: Option<FileTouchscreen>,
    pub(super) idle: Option<FileIdle>,
    pub(super) power: Option<FilePower>,
    pub(super) desktop_icons: Option<bool>,
    pub(super) desktop_extensions: Option<String>,
    pub(super) start_icon: Option<bool>,
    pub(super) system_icons: Option<Vec<String>>,
    pub(super) wallpaper: Option<FileWallpaper>,
    pub(super) screensaver: Option<FileScreensaver>,
    pub(super) outputs: Option<Vec<FileOutput>>,
    pub(super) adaptive_sync: Option<bool>,
    pub(super) window_snapping: Option<FileSnapping>,
    pub(super) screenshot: Option<FileScreenshot>,
    pub(super) previews: Option<FilePreviews>,
    pub(super) switcher: Option<FileSwitcher>,
    pub(super) overview: Option<FileOverview>,
    pub(super) gestures: Option<FileGestures>,
    pub(super) clipboard: Option<FileClipboard>,
    pub(super) privacy: Option<FilePrivacy>,
    pub(super) debug: Option<FileDebug>,
    pub(super) launcher: Option<FileLauncher>,
    pub(super) night_light: Option<FileNightLight>,
    pub(super) workspaces: Option<FileWorkspaces>,
    pub(super) effects: Option<FileEffects>,
    pub(super) accessibility: Option<FileAccessibility>,
    pub(super) windows: Option<FileWindows>,
    pub(super) session: Option<FileSession>,
    pub(super) window_rules: Option<Vec<FileWindowRule>>,
    pub(super) tablet_pads: Option<Vec<crate::tabletpad::FilePad>>,
    pub(super) icons: Option<FileIcons>,
    pub(super) cursor: Option<FileCursor>,
    pub(super) toolkits: Option<FileToolkits>,
    pub(super) appearance: Option<FileAppearance>,
    pub(super) glass: Option<FileGlass>,
    pub(super) taskbar: Option<FileTaskbar>,
}
