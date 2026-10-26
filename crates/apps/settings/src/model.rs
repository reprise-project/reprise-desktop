//! `config.toml` as the sheet sees it: one flat [`Settings`] of every key the pages edit, read from the text and written back with `toml_edit` so comments, key order and untouched tables survive.
//!
//! The defaults are [`repriseconf::config::Config`]'s, the ones the compositor runs with.

use repriseconf::config::{FontSmoothing, StartMenuStyle, TaskbarButtons, switcher_size};
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
        Settings::for_interface("classic")
    }
}

/// The value `choices` lists for `v`, as `parse` reads the values.
fn named<T: PartialEq>(choices: &Choices, parse: impl Fn(&str) -> Option<T>, v: &T) -> String {
    (choices.iter())
        .find(|(name, _)| parse(name).as_ref() == Some(v))
        .map(|(name, _)| name.to_string())
        .expect("every default is one of the sheet's choices")
}

fn tenths(v: f64) -> i64 {
    (v * 10.0).round() as i64
}

impl Settings {
    /// The defaults under `[appearance] interface` = `interface`: the compositor's own
    /// ([`repriseconf::config::Config::default_for`]), with the wallpaper as the file names it.
    /// `interface` itself stays Classic's, the compositor's default, so the key is written for
    /// either of the others.
    pub fn for_interface(interface: &str) -> Settings {
        let interface = Interface::parse(interface).unwrap_or(Interface::Classic);
        let c = repriseconf::config::Config::default_for(interface);
        let m = &c.theme.metrics;
        let fonts = &c.theme.fonts;
        let a = &c.accessibility;
        let fx = &c.effects;
        let hhmm = |min: u32| format!("{:02}:{:02}", min / 60, min % 60);
        Settings {
            terminal: c.terminal.clone(),
            script_terminal: named(
                SCRIPT_ENDS,
                reprisefs::program::ScriptEnd::parse,
                &c.script_terminal,
            ),
            script_open: named(
                SCRIPT_OPENS,
                reprisefs::program::ScriptOpen::parse,
                &c.script_open,
            ),
            task_manager: c.task_manager.clone().unwrap_or_default(),
            startmenu_banner: c.startmenu_banner.clone(),
            startmenu_style: named(STARTMENU_STYLES, StartMenuStyle::parse, &c.startmenu_style),
            desktop_icons: c.desktop_icons,
            desktop_extensions: named(
                DESKTOP_EXTENSIONS,
                reprisefs::names::Extensions::parse,
                &c.desktop_extensions,
            ),
            start_icon: c.start_icon,
            adaptive_sync: c.adaptive_sync,
            switcher_preview: c.switcher.preview,
            switcher_desktop: c.switcher.desktop,
            switcher_peek: c.switcher.peek,
            switcher_flip: c.switcher.flip,
            preview_taskbar: c.previews.taskbar,
            previews_fps: c.previews.fps.into(),
            switcher_style: named(SWITCHER_STYLES, |v| v.parse().ok(), &c.switcher.style),
            switcher_size: named(SWITCHER_SIZES, switcher_size, &c.switcher.size),
            hot_corner: c.hot_corner.name().into(),
            screenshot_save: c.screenshot.save,
            clipboard_history: c.clipboard.history,

            interface: Interface::Classic.as_str().into(),
            color_scheme: c.color_scheme.clone().unwrap_or_default(),
            glass_color: reprisekit::glass::swatch_name(c.glass.color)
                .expect("the default glass colour is a swatch")
                .into(),
            glass_mode: c.glass.mode.as_str().into(),
            glass_material: c.glass.material.as_str().into(),
            glass_frame: c.glass.frame.as_str().into(),
            glass_notification_style: c.glass.notification.as_str().into(),
            glass_custom: String::new(),
            glass_intensity: c.glass.intensity.into(),
            glass_transparency: c.glass.transparency,
            glass_blur: c.glass.blur.into(),
            glass_streaks: c.glass.streaks,
            glass_fast_drag: c.glass.fast_drag,
            glass_blur_frames: c.glass.blur_frames,
            glass_orb_glow: c.glass.orb_glow.into(),
            // The built-in as the file names it; the config holds its tinted variant.
            wallpaper: reprisekit::interfaces::default_wallpaper(interface)
                .map(|b| b.uri())
                .unwrap_or_default(),
            wallpaper_position: named(
                POSITIONS,
                reprisekit::wallpaper::Position::parse,
                &c.wallpaper.position,
            ),
            wallpaper_water: c.wallpaper.water.name().into(),
            drive_insert: repriseconf::DRIVES_ON_INSERT.into(),
            slideshow: String::new(),
            slideshow_interval: c.wallpaper.interval.into(),
            slideshow_shuffle: c.wallpaper.shuffle,
            saver: c.screensaver.kind.name().into(),
            saver_speed: c.screensaver.options.speed.into(),
            saver_stars: c.screensaver.options.stars.into(),
            saver_shapes: c.screensaver.options.shapes.into(),
            saver_cycle: c.screensaver.options.cycle_colors,
            saver_folder: String::new(),
            saver_photo_seconds: c.screensaver.options.photo_seconds.into(),
            background: String::new(),
            system_icons: c.system_icons.clone(),

            taskbar_buttons: named(TASKBAR_BUTTONS, TaskbarButtons::parse, &c.taskbar.buttons),
            taskbar_pinned: c.taskbar.pinned,
            taskbar_small_icons: c.taskbar.small_icons,
            taskbar_show_desktop: c.taskbar.show_desktop_button,
            taskbar_peek: c.taskbar.peek,
            taskbar_tray_overflow: c.taskbar.tray_overflow,
            taskbar_position: c.taskbar.position.name().into(),
            taskbar_auto_hide: c.taskbar.auto_hide,
            taskbar_reveal_on_edge: c.taskbar.reveal_on_edge,
            taskbar_reveal_on_keys: c.taskbar.reveal_on_keys,
            taskbar_reveal_on_attention: c.taskbar.reveal_on_attention,
            recent_files: c.recent_files,
            location: c.location,
            capture_border: c.capture_border,
            debug_overlay: c.debug.overlay,
            debug_overlay_in_captures: c.debug.overlay_in_captures,
            launcher_files: c.launcher_files,
            thumbnail_cache: repriseconf::ThumbnailCache::DEFAULT.enabled,
            thumbnail_max_mb: repriseconf::ThumbnailCache::DEFAULT.max_mb as i64,

            desktops: c.workspaces as i64,
            desktop_animation: c.workspace_animation.style.name().into(),
            desktop_animation_ms: c.workspace_animation.ms.into(),
            pager_mark: c.pager_mark.name().into(),
            remember_windows: c.remember_windows,
            window_tabs: c.window_tabs,
            window_shake: c.window_shake,
            session_restore: c.session.restore,
            session_end_processes: c.session.end_processes,

            font_family: fonts.family.join(", "),
            font_size_px: fonts.size_px.into(),
            font_antialias: fonts.antialias,
            font_shell_dlg: fonts.shell_dlg.clone().unwrap_or_default(),
            icon_theme: c.icons.theme.clone(),
            gtk_theme: c.toolkits.gtk_theme.clone(),
            qt_platform_theme: c.toolkits.qt_platform_theme.clone(),
            font_smoothing: named(
                FONT_SMOOTHINGS,
                FontSmoothing::parse,
                &c.toolkits.font_smoothing,
            ),
            gtk4_colors: c.toolkits.gtk4_colors,
            cursor_theme: c.cursor.theme.clone(),
            cursor_size: c.cursor.size.into(),

            frame_width: m.frame_width.into(),
            caption_height: m.caption_height.into(),
            taskbar_height: m.taskbar_height.into(),
            menu_shadow: m.menu_shadow.into(),
            menu_hover_ms: m.menu_hover_ms.into(),
            menu_fade_ms: m.menu_fade_ms.into(),
            menu_animation: fx.menu_animation.name().into(),
            window_animation: fx.window_animation,
            window_open_ms: fx.window_open_ms.into(),
            window_close_ms: fx.window_close_ms.into(),
            hover_fade: fx.hover_fade,
            hover_in_ms: fx.hover_in_ms.into(),
            hover_out_ms: fx.hover_out_ms.into(),
            taskbar_slide: fx.taskbar_slide,
            session_fade: fx.session_fade,
            double_click_ms: m.double_click_ms.into(),

            kb_layout: c.keyboard.layout.clone(),
            kb_variant: c.keyboard.variant.clone(),
            kb_model: c.keyboard.model.clone(),
            kb_options: c.keyboard.options.clone(),
            repeat_rate: c.keyboard.repeat_rate.into(),
            repeat_delay: c.keyboard.repeat_delay.into(),

            standby: c.power.standby.clone(),
            poweroff: c.power.poweroff.clone(),
            reboot: c.power.reboot.clone(),
            hibernate: c.power.hibernate.clone(),
            lock_on_sleep: c.power.lock_on_sleep,
            lock_wallpaper: c.wallpaper.lock_screen,

            lock_after: c.idle.lock_after.into(),
            screen_off_after: c.idle.screen_off_after.into(),
            blank_after: c.idle.blank_after.into(),
            lock_fade: c.idle.lock_fade.into(),

            mouse_speed: tenths(c.mouse.speed),
            mouse_accel: c.mouse.acceleration,
            mouse_natural: c.mouse.natural_scroll,
            touchpad_speed: tenths(c.touchpad.pointer.speed),
            touchpad_accel: c.touchpad.pointer.acceleration,
            touchpad_natural: c.touchpad.pointer.natural_scroll,
            tap_to_click: c.touchpad.tap_to_click,
            two_finger_scroll: c.touchpad.two_finger_scroll,
            two_finger_tap_right_click: c.touchpad.two_finger_tap_right_click,
            disable_while_typing: c.touchpad.disable_while_typing,

            snap_enabled: c.window_snapping.enabled,
            snap_picker: c.window_snapping.picker,
            snap_bar: c.window_snapping.bar,
            snap_layouts: String::new(),
            snap_distance: c.window_snapping.distance.into(),

            nl_enabled: c.night_light.enabled,
            nl_temperature: c.night_light.temperature.into(),
            nl_schedule: c.night_light.schedule.name().into(),
            nl_from: hhmm(c.night_light.from),
            nl_to: hhmm(c.night_light.to),

            sticky_keys: a.sticky_keys,
            sticky_lock: a.sticky_lock,
            sticky_two_keys_off: a.sticky_two_keys_off,
            slow_keys: a.slow_keys,
            slow_keys_delay: (a.slow_keys_delay / 100).into(),
            key_shortcuts: a.shortcuts,
            mouse_keys: a.mouse_keys,
            mouse_keys_num_lock: a.mouse_keys_num_lock,
            mouse_keys_speed: a.mouse_keys_speed.into(),
            mouse_keys_acceleration: (a.mouse_keys_acceleration / 100).into(),
            text_size: a.text_size.into(),

            color_filter: a.color_filter.name().into(),
            color_filter_intensity: a.color_filter_intensity.into(),
            magnifier_follow: a.magnifier_follow.name().into(),

            shortcuts: default_shortcuts(),
            programs: Vec::new(),
            denied: Vec::new(),
            startmenu: crate::startmenu::default_menu(),
            tablet_pads: Vec::new(),
        }
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
