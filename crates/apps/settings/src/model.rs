//! `config.toml` as the sheet sees it: one flat [`Settings`] of every key the pages edit, read from the text and written back with `toml_edit` so comments, key order and untouched tables survive.
//!
//! The defaults mirror the compositor's own (`Config::default` and the per-section defaults in `src/config.rs`, `Metrics` and `FontConfig` in `reprisekit::theme`, `glass::Settings`). This crate cannot depend on the compositor, so the lists change together; `src/config.rs` carries the same note above `Config`.

use reprisekit::interface::Interface;
use shortcuts::default_shortcuts;

mod choices;
mod fields;
mod io;
mod shortcuts;

pub use choices::*;
pub(crate) use fields::text_to_list;
pub use io::{load, rebase, save, switch_interface, validate};
pub use shortcuts::{
    ACTIONS, ProgramShortcut, Row, Shortcut, check_keys, check_program_keys, default_chord, forget,
    program_text, row,
};

/// Every key the sheet edits, with the compositor's default in place of a
/// key the file leaves out. An empty `task_manager` means the key is
/// absent (the taskbar menu then runs `top` in the terminal).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub terminal: String,
    /// `script_terminal`: one of [`SCRIPT_ENDS`]' values.
    pub script_terminal: String,
    /// `script_open`: one of [`SCRIPT_OPENS`]' values.
    pub script_open: String,
    pub task_manager: String,
    pub startmenu_banner: String,
    /// `startmenu_style`: one of [`STARTMENU_STYLES`]' values. Glass under
    /// the Glass interface, Classic otherwise (`for_interface`).
    pub startmenu_style: String,
    pub desktop_icons: bool,
    /// `desktop_extensions`: one of [`DESKTOP_EXTENSIONS`]' values.
    pub desktop_extensions: String,
    pub start_icon: bool,
    pub adaptive_sync: bool,
    /// `[switcher] preview`: a live picture of the selected window in the
    /// icon style.
    pub switcher_preview: bool,
    pub switcher_desktop: bool,
    /// `[switcher] peek`: the selected window shows through while Alt+Tab
    /// is held.
    pub switcher_peek: bool,
    /// `[switcher] flip`: the `flip` bind opens the 3D switcher under
    /// Glass.
    pub switcher_flip: bool,
    pub preview_taskbar: bool,
    /// `[previews] fps`: how often a live picture is redrawn, 1 to
    /// [`PREVIEW_FPS_MAX`].
    pub previews_fps: i64,
    /// `[switcher] style`: "icons" or "thumbnails".
    pub switcher_style: String,
    /// `[switcher] size`: "small", "medium", "large" or "huge".
    pub switcher_size: String,
    /// `[overview] hot_corner`: "none" or the corner, as [`HOT_CORNERS`]
    /// names them.
    pub hot_corner: String,
    pub screenshot_save: bool,
    pub clipboard_history: bool,

    /// `[appearance] interface`: "classic", "blue" or "glass".
    /// [`Settings::for_interface`] leaves it at Classic, the compositor's
    /// default, so the key is written for either of the others.
    pub interface: String,
    /// `[appearance] color_scheme`: a Classic scheme's name or a Blue
    /// variant's; empty for the interface's own.
    pub color_scheme: String,
    /// `[glass] color`: a swatch's name or "#RRGGBB".
    pub glass_color: String,
    /// `[glass] mode`: "light" or "dark".
    pub glass_mode: String,
    /// `[glass] material`: one of [`GLASS_MATERIALS`]' values.
    pub glass_material: String,
    /// `[glass] frame`: one of [`GLASS_FRAMES`]' values.
    pub glass_frame: String,
    /// `[glass] notification_style`: one of [`GLASS_NOTIFY_STYLES`]' values.
    pub glass_notification_style: String,
    /// Not a key: `glass_color` as the file had it when it names no
    /// swatch ([`glass_swatch`]), the colour the Appearance page's Custom
    /// row stands for; empty for none.
    pub glass_custom: String,
    /// `[glass] intensity`, 0 to 100.
    pub glass_intensity: i64,
    pub glass_transparency: bool,
    /// `[glass] blur`, 1 to `glass::BLUR_MAX`.
    pub glass_blur: i64,
    /// `[glass] streaks`.
    pub glass_streaks: bool,
    pub glass_fast_drag: bool,
    /// `[glass] blur_frames`.
    pub glass_blur_frames: bool,
    /// `[glass] orb_glow`, 0 to 100.
    pub glass_orb_glow: i64,
    /// `[wallpaper] image`: a picture's path or a `reprise:` built-in;
    /// empty for none. Blue and Glass default to their own built-in.
    pub wallpaper: String,
    /// `[wallpaper] position`: one of [`POSITIONS`]' values.
    pub wallpaper_position: String,
    /// `[wallpaper] water`: "off" or one of [`WATER_STYLES`]' values.
    pub wallpaper_water: String,
    /// `[drives] on_insert`: one of [`DRIVE_INSERTS`]' values.
    pub drive_insert: String,
    /// `[wallpaper] slideshow`: the folder whose pictures take turns;
    /// empty shows `wallpaper` alone.
    pub slideshow: String,
    /// `[wallpaper] interval`: minutes per slideshow picture.
    pub slideshow_interval: i64,
    /// `[wallpaper] shuffle`.
    pub slideshow_shuffle: bool,
    /// `[screensaver] name`: a `reprisekit::saver::Kind` name.
    pub saver: String,
    /// `[screensaver]` speed, stars, shapes, cycle_colors, folder and
    /// photo_seconds; `saver_folder` empty is the Pictures folder.
    pub saver_speed: i64,
    pub saver_stars: i64,
    pub saver_shapes: i64,
    pub saver_cycle: bool,
    pub saver_folder: String,
    pub saver_photo_seconds: i64,
    /// `[colors] background`, the desktop colour; empty for the one the
    /// interface and its scheme give.
    pub background: String,
    /// `system_icons`: the desktop's system icons, in order.
    pub system_icons: Vec<String>,

    /// `[taskbar] buttons`: one of [`TASKBAR_BUTTONS`]' values. It and the
    /// five flags after it default to the file's interface's own; see
    /// [`Settings::for_interface`].
    pub taskbar_buttons: String,
    pub taskbar_pinned: bool,
    pub taskbar_small_icons: bool,
    /// `[taskbar] show_desktop_button`.
    pub taskbar_show_desktop: bool,
    pub taskbar_peek: bool,
    pub taskbar_tray_overflow: bool,
    /// `[taskbar] position`: one of [`TASKBAR_POSITIONS`]' values, the
    /// bottom under every interface.
    pub taskbar_position: String,
    /// `[taskbar] auto_hide` and its three reveal triggers: off and on
    /// under every interface.
    pub taskbar_auto_hide: bool,
    pub taskbar_reveal_on_edge: bool,
    pub taskbar_reveal_on_keys: bool,
    pub taskbar_reveal_on_attention: bool,
    /// `[privacy] recent_files`.
    pub recent_files: bool,
    /// `[privacy] location`: off, every program asking for the location
    /// is refused without a prompt.
    pub location: bool,
    /// `[privacy] capture_border`: the red border around what is shared.
    pub capture_border: bool,
    /// `[debug] overlay`: the stats panel in each screen's corner.
    pub debug_overlay: bool,
    /// `[debug] overlay_in_captures`: screenshots and screen sharing show
    /// the panel too.
    pub debug_overlay_in_captures: bool,
    /// `[launcher] search_files`.
    pub launcher_files: bool,
    /// `[thumbnail_cache] enabled` and `max_mb`, the cap in MiB
    /// ([`repriseconf::ThumbnailCache::MAX_MB`]; the up-down clamps a value outside it).
    pub thumbnail_cache: bool,
    pub thumbnail_max_mb: i64,

    /// `[workspaces] count`, 1 to [`DESKTOPS_MAX`]; 1 turns virtual
    /// desktops off.
    pub desktops: i64,
    /// `[workspaces] animation`: one of [`SWITCH_ANIMATIONS`]' names.
    pub desktop_animation: String,
    pub desktop_animation_ms: i64,
    /// `[workspaces] pager`: one of [`PAGER_MARKS`]' names; only the Glass
    /// taskbar reads it.
    pub pager_mark: String,
    /// `[windows] remember`.
    pub remember_windows: bool,
    /// `[windows] tabs`.
    pub window_tabs: bool,
    /// `[windows] shake`; "auto" reads as the interface's default.
    pub window_shake: bool,
    /// `[session] restore`.
    pub session_restore: bool,
    /// `[session] end_processes`.
    pub session_end_processes: bool,

    pub font_family: String,
    pub font_size_px: i64,
    pub font_antialias: bool,
    /// `[fonts] shell_dlg`, the second dialog font; empty for none.
    pub font_shell_dlg: String,
    pub icon_theme: String,
    /// `[toolkits] gtk_theme`: "auto" or a GTK theme's name.
    pub gtk_theme: String,
    /// `[toolkits] qt_platform_theme`: "auto" or a Qt platform theme.
    pub qt_platform_theme: String,
    /// `[toolkits] font_smoothing`, one of `FONT_SMOOTHINGS`.
    pub font_smoothing: String,
    /// `[toolkits] gtk4_colors`: GTK4 and libadwaita programs take the
    /// look's colours.
    pub gtk4_colors: bool,
    /// `[cursor] theme`; empty keeps the session's theme.
    pub cursor_theme: String,
    /// `[cursor] size` in pixels; 0 keeps the session's size.
    pub cursor_size: i64,

    pub frame_width: i64,
    pub caption_height: i64,
    pub taskbar_height: i64,
    pub menu_shadow: i64,
    pub menu_hover_ms: i64,
    pub menu_fade_ms: i64,
    /// `[effects] menu_animation`: one of [`MENU_ANIMATIONS`]' names.
    pub menu_animation: String,
    pub window_animation: bool,
    pub window_open_ms: i64,
    pub window_close_ms: i64,
    pub hover_fade: bool,
    pub hover_in_ms: i64,
    pub hover_out_ms: i64,
    pub taskbar_slide: bool,
    pub session_fade: bool,
    pub double_click_ms: i64,

    pub kb_layout: String,
    pub kb_variant: String,
    pub kb_model: String,
    pub kb_options: String,
    pub repeat_rate: i64,
    pub repeat_delay: i64,

    pub standby: String,
    pub poweroff: String,
    pub reboot: String,
    pub hibernate: String,
    pub lock_on_sleep: bool,
    /// `[wallpaper] lock_screen`: off under Classic, on under Blue and
    /// Glass ([`Settings::for_interface`]).
    pub lock_wallpaper: bool,

    pub lock_after: i64,
    pub screen_off_after: i64,
    pub blank_after: i64,
    pub lock_fade: i64,

    /// libinput's -1..1 speed in tenths, -10..=10, so the trackbar's steps
    /// and `Eq` both work.
    pub mouse_speed: i64,
    pub mouse_accel: bool,
    pub mouse_natural: bool,
    pub touchpad_speed: i64,
    pub touchpad_accel: bool,
    pub touchpad_natural: bool,
    pub tap_to_click: bool,
    pub two_finger_scroll: bool,
    pub two_finger_tap_right_click: bool,
    pub disable_while_typing: bool,

    pub snap_enabled: bool,
    /// `[window_snapping] picker`: the snap layouts on a resting maximize
    /// button.
    pub snap_picker: bool,
    /// `[window_snapping] bar`: the snap layouts in a bar at the top while
    /// a title bar is dragged toward it.
    pub snap_bar: bool,
    /// `[window_snapping] layouts`: the specs apart by spaces or commas,
    /// empty for the built-in six.
    pub snap_layouts: String,
    /// `[window_snapping] distance`: px from a screen edge where a drag snaps.
    pub snap_distance: i64,

    pub nl_enabled: bool,
    /// Kelvin at night.
    pub nl_temperature: i64,
    /// `sun` or `fixed`.
    pub nl_schedule: String,
    /// "21:00": the fixed schedule's local times.
    pub nl_from: String,
    pub nl_to: String,

    pub sticky_keys: bool,
    pub sticky_lock: bool,
    pub sticky_two_keys_off: bool,
    pub slow_keys: bool,
    /// Tenths of a second, 3 to 20.
    pub slow_keys_delay: i64,
    pub key_shortcuts: bool,
    pub mouse_keys: bool,
    pub mouse_keys_num_lock: bool,
    /// Pixels a second.
    pub mouse_keys_speed: i64,
    /// Tenths of a second, 0 to 50.
    pub mouse_keys_acceleration: i64,
    /// Percent, one of `TEXT_SIZES`.
    pub text_size: i64,

    /// `[accessibility] color_filter`: a `colorfilter::Filter` name.
    pub color_filter: String,
    /// Percent, 0 to 100.
    pub color_filter_intensity: i64,
    /// `pointer` or `pointer+focus`.
    pub magnifier_follow: String,

    /// Every bind the compositor runs with: the file's `[[keybinds]]`
    /// layered over the defaults. Saved as the entries that differ from them.
    pub shortcuts: Vec<Shortcut>,
    /// `[[global_shortcuts]]` and `[portal] shortcuts_denied`: written by
    /// reprise-portal; the sheet edits keys and forgets programs.
    pub programs: Vec<ProgramShortcut>,
    pub denied: Vec<String>,

    /// The whole `[[startmenu]]` tree, or the default menu when the file
    /// has none; written whole, as `shortcuts` is.
    pub startmenu: Vec<crate::startmenu::Entry>,

    /// `[[tablet_pads]]`' own keys, in file order; none by default.
    pub tablet_pads: Vec<crate::tabletpage::PadEntry>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            terminal: reprisefs::apps::DEFAULT_TERMINAL.into(),
            script_terminal: "stay".into(),
            script_open: "run".into(),
            task_manager: String::new(),
            startmenu_banner: "Reprise Desktop".into(),
            startmenu_style: "classic".into(),
            desktop_icons: true,
            desktop_extensions: "hide".into(),
            start_icon: false,
            adaptive_sync: false,
            switcher_preview: true,
            switcher_desktop: false,
            switcher_peek: false,
            switcher_flip: false,
            preview_taskbar: true,
            previews_fps: 10,
            switcher_style: "icons".into(),
            switcher_size: "small".into(),
            hot_corner: "none".into(),
            screenshot_save: true,
            clipboard_history: true,

            interface: "classic".into(),
            color_scheme: String::new(),
            glass_color: "sky".into(),
            glass_mode: "light".into(),
            glass_material: "frosted".into(),
            glass_frame: "rounded".into(),
            glass_notification_style: "lozenge".into(),
            glass_custom: String::new(),
            glass_intensity: 42,
            glass_transparency: true,
            glass_blur: 2,
            glass_streaks: true,
            glass_fast_drag: true,
            glass_blur_frames: true,
            glass_orb_glow: 100,
            wallpaper: String::new(),
            wallpaper_position: "stretch".into(),
            wallpaper_water: "off".into(),
            drive_insert: "ask".into(),
            slideshow: String::new(),
            slideshow_interval: 30,
            slideshow_shuffle: false,
            saver: "blank".into(),
            saver_speed: 5,
            saver_stars: 200,
            saver_shapes: 2,
            saver_cycle: true,
            saver_folder: String::new(),
            saver_photo_seconds: 10,
            background: String::new(),
            system_icons: SYSTEM_ICONS
                .iter()
                .chain([&RECYCLE_BIN])
                .map(|n| n.to_string())
                .collect(),

            taskbar_buttons: "when_full".into(),
            taskbar_pinned: false,
            taskbar_small_icons: false,
            taskbar_show_desktop: false,
            taskbar_peek: false,
            taskbar_tray_overflow: true,
            taskbar_position: "bottom".into(),
            taskbar_auto_hide: false,
            taskbar_reveal_on_edge: true,
            taskbar_reveal_on_keys: true,
            taskbar_reveal_on_attention: true,
            recent_files: true,
            location: true,
            capture_border: true,
            debug_overlay: false,
            debug_overlay_in_captures: false,
            launcher_files: true,
            thumbnail_cache: repriseconf::ThumbnailCache::DEFAULT.enabled,
            thumbnail_max_mb: repriseconf::ThumbnailCache::DEFAULT.max_mb as i64,

            desktops: DESKTOPS_DEFAULT,
            desktop_animation: "none".into(),
            desktop_animation_ms: 200,
            pager_mark: "light".into(),
            remember_windows: true,
            window_tabs: true,
            window_shake: false,
            session_restore: false,
            session_end_processes: true,

            font_family: "Tahoma, DejaVu Sans".into(),
            font_size_px: 11,
            font_antialias: false,
            font_shell_dlg: String::new(),
            icon_theme: "hicolor".into(),
            gtk_theme: TOOLKIT_AUTO.into(),
            qt_platform_theme: TOOLKIT_AUTO.into(),
            font_smoothing: "grayscale".into(),
            gtk4_colors: false,
            cursor_theme: String::new(),
            cursor_size: 0,

            frame_width: 4,
            caption_height: 18,
            taskbar_height: 28,
            menu_shadow: 4,
            menu_hover_ms: 400,
            menu_fade_ms: 150,
            menu_animation: "fade".into(),
            window_animation: false,
            window_open_ms: 250,
            window_close_ms: 200,
            hover_fade: false,
            hover_in_ms: 120,
            hover_out_ms: 250,
            taskbar_slide: false,
            session_fade: true,
            double_click_ms: 500,

            kb_layout: String::new(),
            kb_variant: String::new(),
            kb_model: String::new(),
            kb_options: String::new(),
            repeat_rate: 25,
            repeat_delay: 600,

            standby: "systemctl suspend".into(),
            poweroff: "systemctl poweroff".into(),
            reboot: "systemctl reboot".into(),
            hibernate: "systemctl hibernate".into(),
            lock_on_sleep: true,
            lock_wallpaper: false,

            lock_after: 600,
            screen_off_after: 900,
            blank_after: 0,
            lock_fade: 0,

            mouse_speed: 0,
            mouse_accel: true,
            mouse_natural: false,
            touchpad_speed: 0,
            touchpad_accel: true,
            touchpad_natural: false,
            tap_to_click: true,
            two_finger_scroll: true,
            two_finger_tap_right_click: true,
            disable_while_typing: true,

            snap_enabled: true,
            snap_picker: true,
            snap_bar: true,
            snap_layouts: String::new(),
            snap_distance: SNAP_DISTANCE_DEFAULT,

            nl_enabled: false,
            nl_temperature: 4000,
            nl_schedule: "sun".into(),
            nl_from: "21:00".into(),
            nl_to: "07:00".into(),

            sticky_keys: false,
            sticky_lock: true,
            sticky_two_keys_off: true,
            slow_keys: false,
            slow_keys_delay: 10,
            key_shortcuts: true,
            mouse_keys: false,
            mouse_keys_num_lock: true,
            mouse_keys_speed: 400,
            mouse_keys_acceleration: 10,
            text_size: 100,

            color_filter: "none".into(),
            color_filter_intensity: 100,
            magnifier_follow: "pointer+focus".into(),

            shortcuts: default_shortcuts(),
            programs: Vec::new(),
            denied: Vec::new(),
            startmenu: crate::startmenu::default_menu(),
            tablet_pads: Vec::new(),
        }
    }
}

impl Settings {
    /// The defaults under `[appearance] interface` = `interface`: Glass combines its task buttons, turns the taskbar extras (but not small icons) and the window animation, hover fade and button slide on, shows Alt+Tab as thumbnails with the desktop and peek, and Super+Tab as the 3D switcher. Blue and Glass show the wallpaper on the lock screen. `interface` itself stays Classic's, the compositor's default.
    pub fn for_interface(interface: &str) -> Settings {
        let mut s = Settings {
            lock_wallpaper: matches!(interface, "blue" | "glass"),
            wallpaper: Interface::parse(interface)
                .and_then(reprisekit::interfaces::default_wallpaper)
                .map(|b| b.uri())
                .unwrap_or_default(),
            ..Settings::default()
        };
        if interface == "glass" {
            s.taskbar_buttons = "combine".into();
            s.startmenu_style = "glass".into();
            s.taskbar_pinned = true;
            s.taskbar_show_desktop = true;
            s.taskbar_peek = true;
            s.window_animation = true;
            s.hover_fade = true;
            s.taskbar_slide = true;
            s.switcher_style = "thumbnails".into();
            s.switcher_desktop = true;
            s.switcher_peek = true;
            s.switcher_flip = true;
            s.window_shake = true;
        }
        s
    }

    pub fn layouts(&self) -> Vec<(String, String)> {
        reprisefs::xkb::split(&self.kb_layout, &self.kb_variant)
    }

    fn set_layouts(&mut self, pairs: &[(String, String)]) {
        (self.kb_layout, self.kb_variant) = reprisefs::xkb::join(pairs);
    }

    pub fn add_layout(&mut self, layout: &str, variant: &str) -> bool {
        let mut pairs = self.layouts();
        if pairs.len() >= reprisefs::xkb::MAX_LAYOUTS {
            return false;
        }
        pairs.push((layout.to_string(), variant.to_string()));
        self.set_layouts(&pairs);
        true
    }

    pub fn remove_layout(&mut self, i: usize) {
        let mut pairs = self.layouts();
        if i < pairs.len() {
            pairs.remove(i);
            self.set_layouts(&pairs);
        }
    }

    pub fn move_layout_up(&mut self, i: usize) -> bool {
        let mut pairs = self.layouts();
        if i == 0 || i >= pairs.len() {
            return false;
        }
        pairs.swap(i - 1, i);
        self.set_layouts(&pairs);
        true
    }
}

#[cfg(test)]
mod tests;
