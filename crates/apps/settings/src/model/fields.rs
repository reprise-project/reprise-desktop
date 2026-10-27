use super::Settings;
use repriseconf::{Key, Lens, key as k, lens};
use toml_edit::{Array, Item, Value};

/// One field's key and how to move it in and out of a `Settings`. The
/// list is the single place a key name appears, so `load` and `save`
/// cannot drift apart.
pub(super) enum Field {
    Text(Key, Lens<Settings, String>),
    Int(Key, Lens<Settings, i64>),
    Flag(Key, Lens<Settings, bool>),
    /// A comma-separated `Settings` string against a TOML array of
    /// strings (`fonts.family`).
    List(Key, Lens<Settings, String>),
    /// A `Settings` string of words apart by spaces or commas against a
    /// TOML array of strings (`window_snapping.layouts`).
    Words(Key, Lens<Settings, String>),
    /// A `Settings` integer in tenths against a TOML float (`mouse.speed`),
    /// read back into its range of tenths.
    Tenths(Key, Lens<Settings, i64>, (i64, i64)),
    Names(Key, Lens<Settings, Vec<String>>),
}

pub(super) const INTERFACE: Key = k("appearance", "interface");

pub(super) fn fields() -> Vec<Field> {
    vec![
        Field::Text(k("", "terminal"), lens!(terminal)),
        Field::Text(k("", "script_terminal"), lens!(script_terminal)),
        Field::Text(k("", "script_open"), lens!(script_open)),
        Field::Flag(k("", "confirm_scripts"), lens!(confirm_scripts)),
        Field::Text(k("", "task_manager"), lens!(task_manager)),
        Field::Text(k("", "startmenu_banner"), lens!(startmenu_banner)),
        Field::Text(k("", "startmenu_style"), lens!(startmenu_style)),
        Field::Flag(k("", "desktop_icons"), lens!(desktop_icons)),
        Field::Text(k("", "desktop_extensions"), lens!(desktop_extensions)),
        Field::Flag(k("", "start_icon"), lens!(start_icon)),
        Field::Flag(k("", "adaptive_sync"), lens!(adaptive_sync)),
        Field::Flag(k("previews", "taskbar"), lens!(preview_taskbar)),
        Field::Int(k("previews", "fps"), lens!(previews_fps)),
        Field::Text(k("switcher", "style"), lens!(switcher_style)),
        Field::Text(k("switcher", "size"), lens!(switcher_size)),
        Field::Flag(k("switcher", "preview"), lens!(switcher_preview)),
        Field::Flag(k("switcher", "desktop"), lens!(switcher_desktop)),
        Field::Flag(k("switcher", "peek"), lens!(switcher_peek)),
        Field::Flag(k("switcher", "flip"), lens!(switcher_flip)),
        Field::Text(k("overview", "hot_corner"), lens!(hot_corner)),
        Field::Flag(k("screenshot", "save"), lens!(screenshot_save)),
        Field::Flag(k("clipboard", "history"), lens!(clipboard_history)),
        Field::Text(INTERFACE, lens!(interface)),
        Field::Text(k("appearance", "color_scheme"), lens!(color_scheme)),
        Field::Text(k("glass", "color"), lens!(glass_color)),
        Field::Text(k("glass", "mode"), lens!(glass_mode)),
        Field::Text(k("glass", "material"), lens!(glass_material)),
        Field::Text(k("glass", "frame"), lens!(glass_frame)),
        Field::Text(
            k("glass", "notification_style"),
            lens!(glass_notification_style),
        ),
        Field::Int(k("glass", "intensity"), lens!(glass_intensity)),
        Field::Flag(k("glass", "transparency"), lens!(glass_transparency)),
        Field::Int(k("glass", "blur"), lens!(glass_blur)),
        Field::Flag(k("glass", "streaks"), lens!(glass_streaks)),
        Field::Flag(k("glass", "fast_drag"), lens!(glass_fast_drag)),
        Field::Flag(k("glass", "blur_frames"), lens!(glass_blur_frames)),
        Field::Int(k("glass", "orb_glow"), lens!(glass_orb_glow)),
        Field::Text(k("wallpaper", "image"), lens!(wallpaper)),
        Field::Text(k("wallpaper", "position"), lens!(wallpaper_position)),
        Field::Text(k("wallpaper", "water"), lens!(wallpaper_water)),
        Field::Text(k("drives", "on_insert"), lens!(drive_insert)),
        Field::Text(k("wallpaper", "slideshow"), lens!(slideshow)),
        Field::Int(k("wallpaper", "interval"), lens!(slideshow_interval)),
        Field::Flag(k("wallpaper", "shuffle"), lens!(slideshow_shuffle)),
        Field::Text(k("screensaver", "name"), lens!(saver)),
        Field::Int(k("screensaver", "speed"), lens!(saver_speed)),
        Field::Int(k("screensaver", "stars"), lens!(saver_stars)),
        Field::Int(k("screensaver", "shapes"), lens!(saver_shapes)),
        Field::Flag(k("screensaver", "cycle_colors"), lens!(saver_cycle)),
        Field::Text(k("screensaver", "folder"), lens!(saver_folder)),
        Field::Int(
            k("screensaver", "photo_seconds"),
            lens!(saver_photo_seconds),
        ),
        Field::Text(k("colors", "background"), lens!(background)),
        Field::Names(k("", "system_icons"), lens!(system_icons)),
        Field::Text(k("taskbar", "buttons"), lens!(taskbar_buttons)),
        Field::Flag(k("taskbar", "pinned"), lens!(taskbar_pinned)),
        Field::Flag(k("taskbar", "small_icons"), lens!(taskbar_small_icons)),
        Field::Flag(
            k("taskbar", "show_desktop_button"),
            lens!(taskbar_show_desktop),
        ),
        Field::Flag(k("taskbar", "peek"), lens!(taskbar_peek)),
        Field::Flag(k("taskbar", "tray_overflow"), lens!(taskbar_tray_overflow)),
        Field::Flag(k("notifications", "sound"), lens!(notification_sound)),
        Field::Text(
            k("notifications", "sound_theme"),
            lens!(notification_sound_theme),
        ),
        Field::Text(k("taskbar", "position"), lens!(taskbar_position)),
        Field::Flag(k("taskbar", "auto_hide"), lens!(taskbar_auto_hide)),
        Field::Flag(
            k("taskbar", "reveal_on_edge"),
            lens!(taskbar_reveal_on_edge),
        ),
        Field::Flag(
            k("taskbar", "reveal_on_keys"),
            lens!(taskbar_reveal_on_keys),
        ),
        Field::Flag(
            k("taskbar", "reveal_on_attention"),
            lens!(taskbar_reveal_on_attention),
        ),
        Field::Flag(k("privacy", "recent_files"), lens!(recent_files)),
        Field::Flag(k("privacy", "location"), lens!(location)),
        Field::Flag(k("privacy", "capture_border"), lens!(capture_border)),
        Field::Flag(k("debug", "overlay"), lens!(debug_overlay)),
        Field::Flag(
            k("debug", "overlay_in_captures"),
            lens!(debug_overlay_in_captures),
        ),
        Field::Flag(k("launcher", "search_files"), lens!(launcher_files)),
        Field::Flag(k("thumbnail_cache", "enabled"), lens!(thumbnail_cache)),
        Field::Int(k("thumbnail_cache", "max_mb"), lens!(thumbnail_max_mb)),
        Field::Int(k("workspaces", "count"), lens!(desktops)),
        Field::Text(k("workspaces", "animation"), lens!(desktop_animation)),
        Field::Int(k("workspaces", "animation_ms"), lens!(desktop_animation_ms)),
        Field::Text(k("workspaces", "pager"), lens!(pager_mark)),
        Field::Flag(k("windows", "remember"), lens!(remember_windows)),
        Field::Flag(k("windows", "tabs"), lens!(window_tabs)),
        Field::Flag(k("windows", "shake"), lens!(window_shake)),
        Field::Flag(k("session", "restore"), lens!(session_restore)),
        Field::Flag(k("session", "end_processes"), lens!(session_end_processes)),
        Field::List(k("fonts", "family"), lens!(font_family)),
        Field::Int(k("fonts", "size_px"), lens!(font_size_px)),
        Field::Flag(k("fonts", "antialias"), lens!(font_antialias)),
        Field::Text(k("fonts", "shell_dlg"), lens!(font_shell_dlg)),
        Field::Text(k("icons", "theme"), lens!(icon_theme)),
        Field::Text(k("toolkits", "gtk_theme"), lens!(gtk_theme)),
        Field::Text(k("toolkits", "qt_platform_theme"), lens!(qt_platform_theme)),
        Field::Text(k("toolkits", "font_smoothing"), lens!(font_smoothing)),
        Field::Flag(k("toolkits", "gtk4_colors"), lens!(gtk4_colors)),
        Field::Text(k("cursor", "theme"), lens!(cursor_theme)),
        Field::Int(k("cursor", "size"), lens!(cursor_size)),
        Field::Int(k("metrics", "frame_width"), lens!(frame_width)),
        Field::Int(k("metrics", "caption_height"), lens!(caption_height)),
        Field::Int(k("metrics", "taskbar_height"), lens!(taskbar_height)),
        Field::Int(k("metrics", "menu_shadow"), lens!(menu_shadow)),
        Field::Int(k("metrics", "menu_hover_ms"), lens!(menu_hover_ms)),
        Field::Int(k("metrics", "menu_fade_ms"), lens!(menu_fade_ms)),
        Field::Text(k("effects", "menu_animation"), lens!(menu_animation)),
        Field::Flag(k("effects", "window_animation"), lens!(window_animation)),
        Field::Int(k("effects", "window_open_ms"), lens!(window_open_ms)),
        Field::Int(k("effects", "window_close_ms"), lens!(window_close_ms)),
        Field::Flag(k("effects", "hover_fade"), lens!(hover_fade)),
        Field::Int(k("effects", "hover_in_ms"), lens!(hover_in_ms)),
        Field::Int(k("effects", "hover_out_ms"), lens!(hover_out_ms)),
        Field::Flag(k("effects", "taskbar_slide"), lens!(taskbar_slide)),
        Field::Flag(k("effects", "session_fade"), lens!(session_fade)),
        Field::Int(k("metrics", "double_click_ms"), lens!(double_click_ms)),
        Field::Text(k("keyboard", "layout"), lens!(kb_layout)),
        Field::Text(k("keyboard", "variant"), lens!(kb_variant)),
        Field::Text(k("keyboard", "model"), lens!(kb_model)),
        Field::Text(k("keyboard", "options"), lens!(kb_options)),
        Field::Int(k("keyboard", "repeat_rate"), lens!(repeat_rate)),
        Field::Int(k("keyboard", "repeat_delay"), lens!(repeat_delay)),
        Field::Text(k("power", "standby"), lens!(standby)),
        Field::Text(k("power", "poweroff"), lens!(poweroff)),
        Field::Text(k("power", "reboot"), lens!(reboot)),
        Field::Text(k("power", "hibernate"), lens!(hibernate)),
        Field::Flag(k("power", "lock_on_sleep"), lens!(lock_on_sleep)),
        Field::Flag(k("wallpaper", "lock_screen"), lens!(lock_wallpaper)),
        Field::Int(k("idle", "lock_after"), lens!(lock_after)),
        Field::Int(k("idle", "screen_off_after"), lens!(screen_off_after)),
        Field::Int(k("idle", "blank_after"), lens!(blank_after)),
        Field::Int(k("idle", "lock_fade"), lens!(lock_fade)),
        Field::Tenths(k("mouse", "speed"), lens!(mouse_speed), (-10, 10)),
        Field::Flag(k("mouse", "acceleration"), lens!(mouse_accel)),
        Field::Flag(k("mouse", "natural_scroll"), lens!(mouse_natural)),
        Field::Tenths(k("touchpad", "speed"), lens!(touchpad_speed), (-10, 10)),
        Field::Flag(k("touchpad", "acceleration"), lens!(touchpad_accel)),
        Field::Flag(k("touchpad", "natural_scroll"), lens!(touchpad_natural)),
        Field::Flag(k("touchpad", "tap_to_click"), lens!(tap_to_click)),
        Field::Flag(k("touchpad", "two_finger_scroll"), lens!(two_finger_scroll)),
        Field::Flag(
            k("touchpad", "two_finger_tap_right_click"),
            lens!(two_finger_tap_right_click),
        ),
        Field::Flag(
            k("touchpad", "disable_while_typing"),
            lens!(disable_while_typing),
        ),
        Field::Flag(k("window_snapping", "enabled"), lens!(snap_enabled)),
        Field::Flag(k("window_snapping", "picker"), lens!(snap_picker)),
        Field::Flag(k("window_snapping", "bar"), lens!(snap_bar)),
        Field::Words(k("window_snapping", "layouts"), lens!(snap_layouts)),
        Field::Int(k("window_snapping", "distance"), lens!(snap_distance)),
        Field::Flag(k("night_light", "enabled"), lens!(nl_enabled)),
        Field::Int(k("night_light", "temperature"), lens!(nl_temperature)),
        Field::Text(k("night_light", "schedule"), lens!(nl_schedule)),
        Field::Text(k("night_light", "from"), lens!(nl_from)),
        Field::Text(k("night_light", "to"), lens!(nl_to)),
        Field::Flag(k("accessibility", "sticky_keys"), lens!(sticky_keys)),
        Field::Flag(k("accessibility", "sticky_lock"), lens!(sticky_lock)),
        Field::Flag(
            k("accessibility", "sticky_two_keys_off"),
            lens!(sticky_two_keys_off),
        ),
        Field::Flag(k("accessibility", "slow_keys"), lens!(slow_keys)),
        Field::Tenths(
            k("accessibility", "slow_keys_delay"),
            lens!(slow_keys_delay),
            (3, 20),
        ),
        Field::Flag(k("accessibility", "shortcuts"), lens!(key_shortcuts)),
        Field::Text(k("accessibility", "color_filter"), lens!(color_filter)),
        Field::Int(
            k("accessibility", "color_filter_intensity"),
            lens!(color_filter_intensity),
        ),
        Field::Text(
            k("accessibility", "magnifier_follow"),
            lens!(magnifier_follow),
        ),
        Field::Flag(k("accessibility", "mouse_keys"), lens!(mouse_keys)),
        Field::Flag(
            k("accessibility", "mouse_keys_num_lock"),
            lens!(mouse_keys_num_lock),
        ),
        Field::Int(
            k("accessibility", "mouse_keys_speed"),
            lens!(mouse_keys_speed),
        ),
        Field::Tenths(
            k("accessibility", "mouse_keys_acceleration"),
            lens!(mouse_keys_acceleration),
            (0, 50),
        ),
        Field::Int(k("accessibility", "text_size"), lens!(text_size)),
    ]
}

impl Field {
    pub(super) fn key(&self) -> &Key {
        match self {
            Field::Text(k, _)
            | Field::Int(k, _)
            | Field::Flag(k, _)
            | Field::List(k, _)
            | Field::Words(k, _)
            | Field::Tenths(k, _, _)
            | Field::Names(k, _) => k,
        }
    }

    pub(super) fn value(&self, s: &Settings) -> Value {
        match self {
            Field::Text(_, at) => Value::from(at.get(s).as_str()),
            Field::Int(_, at) => Value::from(*at.get(s)),
            Field::Flag(_, at) => Value::from(*at.get(s)),
            Field::List(_, at) => {
                Value::Array(text_to_list(at.get(s)).into_iter().collect::<Array>())
            }
            Field::Words(_, at) => Value::Array(words(at.get(s)).collect::<Array>()),
            Field::Tenths(_, at, _) => Value::from(*at.get(s) as f64 / 10.0),
            Field::Names(_, at) => {
                Value::Array(at.get(s).iter().map(String::as_str).collect::<Array>())
            }
        }
    }

    pub(super) fn same(&self, a: &Settings, b: &Settings) -> bool {
        match self {
            Field::Text(_, at) | Field::List(_, at) | Field::Words(_, at) => at.get(a) == at.get(b),
            Field::Int(_, at) | Field::Tenths(_, at, _) => at.get(a) == at.get(b),
            Field::Flag(_, at) => at.get(a) == at.get(b),
            Field::Names(_, at) => at.get(a) == at.get(b),
        }
    }

    pub(super) fn copy(&self, from: &Settings, to: &mut Settings) {
        match self {
            Field::Text(_, at) | Field::List(_, at) | Field::Words(_, at) => {
                *at.get_mut(to) = at.get(from).clone()
            }
            Field::Int(_, at) | Field::Tenths(_, at, _) => *at.get_mut(to) = *at.get(from),
            Field::Flag(_, at) => *at.get_mut(to) = *at.get(from),
            Field::Names(_, at) => *at.get_mut(to) = at.get(from).clone(),
        }
    }
}

pub(super) fn list_to_text(item: &Item) -> Option<String> {
    let arr = item.as_array()?;
    Some(
        arr.iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    )
}

pub(super) fn words(text: &str) -> impl Iterator<Item = &str> {
    (text.split([',', ' ', '\t'])).filter(|w| !w.is_empty())
}

pub(crate) fn text_to_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}
