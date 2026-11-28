use super::{Lookup, Page};
use reprisekit::i18n::{Strings, Value};

pub(super) const SETTINGS_EN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../apps/settings/locale/en.ftl"
));

/// Settings' strings: a control's search term is the label its page draws.
pub(super) static SETTINGS: Strings = Strings::new("settings", SETTINGS_EN, cfg!(test));

/// Each labelled control on `p` as (control id, label as shown), the label drawn from settings'
/// message through `tr` with its access key dropped.
pub(super) fn of(p: Page, tr: Lookup) -> Vec<(&'static str, String)> {
    (ids(p).iter())
        .map(|&(id, message)| {
            let label = tr(message, &args(message));
            (id, reprisekit::text::split_mnemonic(&label).0)
        })
        .collect()
}

/// The variables of `message`, valued as Settings draws them, less the user's name before Home.
fn args(message: &str) -> Vec<(&'static str, Value<'static>)> {
    let chord = |name, english| vec![(name, Value::from(reprisekit::keycap::shown_chord(english)))];
    match message {
        "pages-desktop-alt-tab-shows" | "pages-desktop-show-the-desktop-in-alt-tab" => {
            chord("keys", "Alt+Tab")
        }
        "pages-input-hold-super-to-show-them-after" => chord("super_key", "Super"),
        "pages-system-paste-with-ctrl-shift-v-into" => chord("keys", "Ctrl+Shift+V"),
        "backgroundpage-home-owned" => vec![("owner", Value::from(""))],
        _ => Vec::new(),
    }
}

/// Each labelled control on `p` as (control id, id of the settings message drawing its label).
pub(super) fn ids(p: Page) -> &'static [(&'static str, &'static str)] {
    match p {
        Page::Appearance => &[
            ("interface", "appearancepage-interface"),
            ("glass_color", "appearancepage-color-scheme"),
            ("mode", "appearancepage-mode"),
            ("glass_material", "appearancepage-material"),
            ("glass_frame", "appearancepage-frame"),
            ("glass_style", "appearancepage-style"),
            ("glass_transparency", "appearancepage-enable-transparency"),
            (
                "glass_fast_drag",
                "appearancepage-faster-glass-while-dragging",
            ),
            (
                "glass_blur_frames",
                "appearancepage-show-frames-behind-glass",
            ),
            ("glass_streaks", "appearancepage-show-reflections"),
            ("glass_intensity", "appearancepage-color-intensity"),
            ("glass_blur", "appearancepage-blur"),
            ("dark_from", "appearancepage-dark-hours"),
            ("dark_to", "appearancepage-dark-until"),
            ("classic_scheme", "appearancepage-color-scheme"),
            ("blue_scheme", "appearancepage-color-scheme"),
            ("dark_scheme", "appearancepage-dark-color-scheme"),
        ],
        Page::Background => &[
            ("wallpaper_mode", "backgroundpage-background"),
            ("browse", "backgroundpage-browse"),
            ("wallpaper_position", "backgroundpage-position"),
            ("background", "backgroundpage-color"),
            ("wallpaper_for", "backgroundpage-use-for"),
            ("documents", "backgroundpage-documents"),
            ("computer", "backgroundpage-computer"),
            ("home", "backgroundpage-home-owned"),
            ("desktop_icons", "backgroundpage-show-the-files-in-desktop"),
            ("desktop_extensions", "backgroundpage-file-name-extensions"),
            ("slideshow_interval", "backgroundpage-change-picture-every"),
            ("slideshow_shuffle", "backgroundpage-shuffle"),
            ("widget_clock", "backgroundpage-clock"),
            ("widget_calendar", "backgroundpage-calendar"),
            ("widget_meter", "backgroundpage-system-meter"),
            ("wallpaper_water", "backgroundpage-desktop-water"),
            ("water_style", "backgroundpage-water-style"),
        ],
        Page::ScreenSaverLock => &[
            ("saver", "saverpage-screen-saver"),
            ("saver_settings", "saverpage-settings"),
            ("saver_preview", "saverpage-preview"),
            ("saver_wait", "saverpage-wait"),
            ("lock_after", "saverpage-lock-after"),
            ("lock_fade", "saverpage-fade-before-lock"),
            ("lock_on_sleep", "saverpage-lock-the-computer-when-it-goes"),
            ("away_device", "saverpage-lock-when-this-device-disconnects"),
            ("away_after_s", "saverpage-after"),
            ("lock_media", "saverpage-now-playing"),
        ],
        Page::Display => &[
            ("primary", "displaypage-use-as-the-primary-display"),
            ("identify", "displaypage-identify"),
            ("resolution", "displaypage-screen-resolution-2"),
            ("refresh_rate", "displaypage-screen-refresh-rate"),
            ("orientation", "displaypage-orientation"),
            ("scale", "displaypage-scale"),
            ("vrr", "displaypage-variable-refresh-rate"),
            ("forget_layouts", "displaypage-forget-saved-layouts"),
            ("multiple_displays", "displaypage-multiple-displays"),
            ("auto_rotate", "displaypage-rotate-automatically"),
            ("icc_profile", "displaypage-color-profile"),
        ],
        Page::NightLight => &[
            (
                "nl_enabled",
                "pages-appearance-warm-the-screens-colors-at-night",
            ),
            ("nl_temperature", "pages-appearance-color-temperature-2"),
            ("nl_sun", "pages-appearance-from-sunset-to-sunrise"),
            ("nl_fixed", "pages-appearance-at-fixed-times"),
            ("nl_from", "pages-appearance-from"),
            ("nl_to", "pages-appearance-to"),
        ],
        Page::FontsAndIcons => &[
            ("font_family", "pages-appearance-family-comma-separated"),
            ("font_shell_dlg", "pages-appearance-second-dialog-font"),
            ("font_size_px", "pages-appearance-size"),
            ("font_antialias", "pages-appearance-smooth-the-edges"),
            ("font_smoothing", "pages-appearance-other-programs"),
            ("icon_theme", "pages-appearance-theme"),
            ("frame_width", "pages-appearance-frame-width"),
            ("caption_height", "pages-appearance-caption-height"),
            ("gtk_theme", "pages-appearance-gtk-theme"),
            ("qt_platform_theme", "pages-appearance-qt-style"),
            (
                "gtk4_colors",
                "pages-appearance-gtk4-and-libadwaita-in-the-looks",
            ),
        ],
        Page::Effects => &[
            ("reduce_motion", "pages-appearance-reduce-motion-everywhere"),
            (
                "window_animation",
                "pages-appearance-animate-windows-opening-and-closing",
            ),
            ("window_open_ms", "pages-appearance-open"),
            ("window_close_ms", "pages-appearance-close"),
            ("hover_fade", "pages-appearance-fade-button-highlights"),
            ("hover_in_ms", "pages-appearance-in"),
            ("hover_out_ms", "pages-appearance-out"),
            ("session_fade", "pages-appearance-fade-the-screen-at-log-on"),
            (
                "logon_animation",
                "pages-appearance-animate-the-lock-screen",
            ),
            ("menu_shadow", "pages-appearance-shadow"),
            ("menu_hover_ms", "pages-appearance-hover"),
            ("menu_fade_ms", "pages-appearance-time"),
            ("menu_animation", "pages-appearance-animation-2"),
        ],
        Page::Taskbar => &[
            ("taskbar_position", "pages-desktop-taskbar-location"),
            ("taskbar_height", "pages-desktop-height"),
            ("taskbar_buttons", "pages-desktop-taskbar-buttons"),
            (
                "taskbar_small_icons",
                "pages-desktop-use-small-icons-on-a-shorter",
            ),
            (
                "taskbar_show_desktop",
                "pages-desktop-show-desktop-button-at-the-taskbars",
            ),
            ("taskbar_peek", "pages-desktop-peek-at-the-desktop-or-a"),
            (
                "taskbar_badges",
                "pages-desktop-show-unread-counts-on-task-buttons",
            ),
            (
                "taskbar_per_monitor",
                "pages-desktop-show-each-windows-button-only-on",
            ),
            (
                "taskbar_tray_overflow",
                "pages-desktop-hide-idle-notification-icons",
            ),
            (
                "taskbar_network_meter",
                "pages-desktop-show-network-speed-beside-the-clock",
            ),
            (
                "taskbar_slide",
                "pages-desktop-slide-taskbar-buttons-as-they-are",
            ),
            (
                "taskbar_calendar_events",
                "pages-desktop-show-calendar-events",
            ),
            ("taskbar_tray_customize", "pages-desktop-customize"),
            ("taskbar_auto_hide", "pages-desktop-auto-hide-the-taskbar"),
            (
                "taskbar_reveal_on_edge",
                "pages-desktop-show-when-the-pointer-touches-the",
            ),
            (
                "taskbar_reveal_on_keys",
                "pages-desktop-show-for-the-start-menu-and",
            ),
            (
                "taskbar_reveal_on_attention",
                "pages-desktop-show-when-a-window-needs-attention",
            ),
            (
                "preview_taskbar",
                "pages-desktop-show-live-window-pictures-over-taskbar",
            ),
            ("previews_fps", "pages-desktop-refresh-rate"),
            ("start_icon", "pages-desktop-show-the-icon-themes-logo-on"),
            ("glass_orb_glow", "pages-desktop-start-button-glow"),
        ],
        Page::StartMenu => &[
            ("startmenu_style", "menupage-start-menu-style"),
            ("startmenu_banner", "menupage-banner"),
            ("sm_tree", "menupage-start-menu-2"),
            ("sm_add", "menupage-add"),
            ("sm_folder", "menupage-new-folder-2"),
            ("sm_rename", "menupage-rename"),
            ("sm_remove", "menupage-remove"),
            ("sm_up", "menupage-move-up"),
            ("sm_down", "menupage-move-down"),
            ("sm_place_documents", "menupage-place-documents"),
            ("sm_place_pictures", "menupage-place-pictures"),
            ("sm_place_music", "menupage-place-music"),
            ("sm_place_computer", "menupage-place-computer"),
            ("sm_place_control_panel", "menupage-place-control-panel"),
            ("sm_place_printers", "menupage-place-printers"),
            ("sm_place_run", "menupage-place-run"),
            ("startmenu_power", "menupage-power-button"),
            ("startmenu_programs", "menupage-frequent-programs"),
            ("sm_clear_recent", "menupage-clear-recent-items"),
            (
                "taskbar_pinned",
                "quickpage-show-the-quick-launch-programs-as",
            ),
            ("ql_add", "quickpage-add"),
            ("ql_remove", "quickpage-remove"),
            ("ql_up", "quickpage-move-up"),
            ("ql_down", "quickpage-move-down"),
        ],
        Page::Search => &[
            (
                "launcher_files",
                "pages-desktop-also-find-files-and-folders-in",
            ),
            ("launcher_windows", "pages-desktop-also-find-open-windows"),
            (
                "launcher_contents",
                "pages-desktop-find-files-by-their-contents",
            ),
            ("search_providers", "pages-desktop-program-results"),
            ("search_providers_up", "pages-desktop-move-up"),
            ("search_providers_down", "pages-desktop-move-down"),
        ],
        Page::Switcher => &[
            ("switcher_style", "pages-desktop-alt-tab-shows"),
            ("switcher_size", "pages-desktop-picture-size"),
            (
                "switcher_preview",
                "pages-desktop-show-a-live-preview-of-the",
            ),
            (
                "switcher_desktop",
                "pages-desktop-show-the-desktop-in-alt-tab",
            ),
            ("switcher_peek", "pages-desktop-peek-at-the-selected-window"),
            ("switcher_flip", "pages-desktop-show-windows-in-3d"),
            ("hot_corner", "pages-desktop-hot-corner"),
        ],
        Page::VirtualDesktops => &[
            ("use_desktops", "pages-desktop-use-virtual-desktops"),
            ("desktops", "pages-desktop-number-of-desktops"),
            ("desktop_animation", "pages-desktop-switch-animation"),
            ("desktop_animation_ms", "pages-desktop-duration"),
            (
                "pager_mark",
                "pages-desktop-current-desktop-on-the-glass-taskbar",
            ),
            (
                "wheel_on_desktop",
                "pages-desktop-switch-desktops-with-the-wheel-over",
            ),
            ("desktop_names", "pages-desktop-desktop-names"),
        ],
        Page::WindowBehavior => &[
            (
                "snap_enabled",
                "pages-desktop-snap-a-dragged-window-to-screen",
            ),
            (
                "snap_picker",
                "pages-desktop-show-snap-layouts-when-resting-on",
            ),
            ("snap_bar", "pages-desktop-show-snap-layouts-in-a-bar"),
            ("snap_distance", "pages-desktop-snap-distance-from-edges"),
            ("snap_layouts", "pages-desktop-custom-layouts"),
            ("window_tabs", "pages-desktop-drop-a-window-on-a-title"),
            (
                "remember_windows",
                "pages-desktop-remember-where-each-programs-window-was",
            ),
            ("forget_window_frames", "pages-desktop-forget-positions"),
            ("window_shake", "pages-desktop-shake-a-window-by-its-title"),
            (
                "title_double_click",
                "pages-desktop-double-clicking-a-title-bar",
            ),
            (
                "dim_inactive",
                "pages-desktop-dim-windows-that-dont-have-the",
            ),
            (
                "hidden_frame_rate",
                "pages-desktop-frame-rate-of-hidden-windows",
            ),
        ],
        Page::WindowRules => &[
            ("window_rules", "rulespage-window-rules"),
            ("wr_add", "rulespage-add"),
            ("wr_edit", "rulespage-edit"),
            ("wr_remove", "rulespage-remove"),
            ("wr_up", "rulespage-move-up"),
            ("wr_down", "rulespage-move-down"),
        ],
        Page::Keyboard => &[
            ("kb_layouts", "pages-input-layouts"),
            ("kb_add", "pages-input-add"),
            ("kb_remove", "pages-input-remove"),
            ("kb_up", "pages-input-move-up"),
            (
                "layout_per_window",
                "pages-input-remember-a-layout-for-each-window",
            ),
            ("kb_compose", "pages-input-compose-key"),
            ("kb_caps", "pages-input-caps-lock"),
            ("kb_model", "pages-input-model"),
            ("kb_options", "pages-input-options"),
            ("repeat_rate", "pages-input-repeat-rate"),
            ("repeat_delay", "pages-input-delay"),
            (
                "shortcut_overlay",
                "pages-input-hold-super-to-show-them-after",
            ),
            ("sc_add", "pages-input-add-2"),
            ("sc_edit", "pages-input-edit"),
            ("sc_remove", "pages-input-delete"),
            ("osd_caps_lock", "pages-input-show-caps-lock"),
            ("osd_num_lock", "pages-input-show-num-lock"),
        ],
        Page::Mouse => &[
            ("double_click_ms", "pages-input-double-click-speed-2"),
            ("mouse_speed", "pages-input-pointer-speed"),
            ("mouse_accel", "pages-input-enhance-pointer-precision"),
            (
                "mouse_middle",
                "pages-input-press-both-buttons-for-a-middle",
            ),
            (
                "mouse_left_handed",
                "pages-input-switch-primary-and-secondary-buttons",
            ),
            ("mouse_scroll_speed", "pages-input-scroll-speed"),
            ("mouse_natural", "pages-input-reverse-scrolling-direction"),
        ],
        Page::Touchpad => &[
            ("touchpad_enabled", "pages-input-use-the-touchpad"),
            ("gestures_enabled", "pages-input-swipe-with"),
            ("gesture_fingers", "pages-input-swipe-fingers"),
            (
                "disable_with_mouse",
                "pages-input-turn-it-off-while-a-mouse",
            ),
            ("touchpad_speed", "pages-input-touchpad-speed"),
            ("touchpad_accel", "pages-input-enhance-pointer-precision"),
            ("disable_while_typing", "pages-input-disable-while-typing"),
            ("tap_to_click", "pages-input-tap-to-click"),
            (
                "two_finger_tap_right_click",
                "pages-input-two-finger-tap-is-a-right",
            ),
            (
                "touchpad_left_handed",
                "pages-input-switch-primary-and-secondary-buttons",
            ),
            ("click_method", "pages-input-click-method"),
            (
                "two_finger_scroll",
                "pages-input-scroll-with-two-fingers-edge-scrolling",
            ),
            ("touchpad_scroll_speed", "pages-input-scroll-speed"),
            (
                "touchpad_natural",
                "pages-input-reverse-scrolling-direction-2",
            ),
        ],
        Page::Tablet => &[
            ("tablet_pad", "tabletpage-pad"),
            ("tablet_ring_step", "tabletpage-ring-step"),
            ("tablet_strip_step", "tabletpage-strip-step"),
        ],
        Page::Thunderbolt => &[
            ("thunderbolt_devices", "pages-thunderbolt-devices"),
            ("thunderbolt_authorize", "pages-thunderbolt-authorize"),
            ("thunderbolt_forget", "pages-thunderbolt-forget"),
        ],
        Page::Cursor => &[
            ("cursor_theme", "pages-input-scheme"),
            ("cursor_size", "pages-input-size"),
        ],
        Page::TextKeys => &[
            ("text_size", "pages-input-text-size"),
            ("sticky_keys", "pages-input-use-stickykeys"),
            ("sticky_lock", "pages-input-lock-a-modifier-key-when-it"),
            (
                "sticky_two_keys_off",
                "pages-input-turn-stickykeys-off-when-two-keys",
            ),
            ("slow_keys", "pages-input-use-slowkeys"),
            ("slow_keys_delay", "pages-input-slowkeys-delay"),
            ("bounce_keys", "pages-input-use-bouncekeys"),
            ("bounce_keys_delay", "pages-input-bouncekeys-delay"),
            ("key_shortcuts", "pages-input-use-the-keyboard-shortcuts"),
        ],
        Page::PointerAlerts => &[
            ("mouse_keys", "pages-input-use-mousekeys"),
            (
                "mouse_keys_num_lock",
                "pages-input-only-while-num-lock-is-on",
            ),
            ("mouse_keys_speed", "pages-input-mousekeys-speed"),
            (
                "mouse_keys_acceleration",
                "pages-input-mousekeys-acceleration",
            ),
            (
                "locate_pointer",
                "pages-input-show-where-the-pointer-is-when",
            ),
            ("visual_bell", "pages-input-flash-on-the-bell"),
            (
                "taskbar_attention_flash",
                "pages-input-flash-a-taskbar-button-for-attention",
            ),
        ],
        Page::MagnifierFilters => &[
            ("color_filter", "magnifierpage-color-filter-2"),
            ("color_filter_intensity", "magnifierpage-intensity"),
            (
                "magnifier_follow",
                "magnifierpage-when-zoomed-in-follow-the-text",
            ),
            (
                "vision_reduce_motion",
                "magnifierpage-reduce-motion-everywhere",
            ),
            ("screen_reader", "magnifierpage-use-the-screen-reader"),
            (
                "screen_reader_logon",
                "magnifierpage-start-the-screen-reader-when-i",
            ),
        ],
        Page::Sound => &[
            ("audio_devices", "pages-system-audio-devices"),
            ("sound_theme", "pages-system-sound-theme-2"),
            ("bell", "pages-system-play-a-sound-when-a-program"),
            (
                "sound_events",
                "pages-system-play-sounds-for-desktop-events",
            ),
            ("allow_boost", "pages-system-allow-volume-above-100"),
        ],
        Page::ScreenCapture => &[
            (
                "screenshot_save",
                "capturepage-save-in-pictures-screenshots",
            ),
            ("screenshot_delay", "capturepage-screenshot-delay"),
            ("screenshot_format", "capturepage-format"),
            ("screenshot_png_compression", "capturepage-png-compression"),
            ("screenshot_jpg_quality", "capturepage-jpg-quality"),
            ("screenshot_jxl_lossless", "capturepage-lossless"),
            ("screenshot_jxl_effort", "capturepage-effort"),
            ("screenshot_jxl_quality", "capturepage-jpeg-xl-quality"),
            ("recording_codec", "capturepage-codec"),
            ("recording_encoder", "capturepage-encoder"),
            ("recording_rate", "capturepage-rate-control"),
            ("recording_quality", "capturepage-recording-quality"),
            ("recording_bitrate", "capturepage-bitrate"),
            ("recording_frame_rate", "capturepage-frame-rate"),
            ("recording_keyframe_interval", "capturepage-keyframe-after"),
            ("screenshot_record_sound", "capturepage-record-what-plays"),
            (
                "screenshot_record_microphone",
                "capturepage-record-the-microphone",
            ),
            ("recording_audio_codec", "capturepage-audio-codec"),
            ("recording_audio_bitrate", "capturepage-audio-bitrate"),
            ("recording_audio_vbr", "capturepage-variable-bitrate"),
            ("recording_flac_compression", "capturepage-flac-compression"),
        ],
        Page::Notifications => &[
            (
                "notification_sound",
                "pages-system-play-a-sound-for-notifications",
            ),
            ("glass_notification_style", "pages-system-balloon-style"),
            (
                "auto_dnd",
                "pages-system-hold-balloons-during-full-screen-and",
            ),
            (
                "quiet_hours",
                "pages-system-hold-balloons-during-quiet-hours",
            ),
            ("quiet_from", "pages-system-from"),
            ("quiet_to", "pages-system-to"),
            ("notify_programs", "pages-system-program-notifications"),
            ("notify_balloons", "pages-system-show-balloons"),
            ("notify_sounds", "pages-system-play-sounds-2"),
        ],
        Page::Startup => &[
            ("add", "pages-system-add"),
            ("remove", "pages-system-remove"),
            (
                "session_restore",
                "pages-system-reopen-the-programs-that-were-open",
            ),
            ("session_exclude", "pages-system-except-app-ids"),
            (
                "session_end_processes",
                "pages-system-end-the-programs-still-running-when",
            ),
        ],
        Page::DefaultPrograms => &[
            ("dp_browser", "defaultspage-web-browser"),
            ("dp_mail", "defaultspage-mail"),
            ("dp_music", "defaultspage-music"),
            ("dp_video", "defaultspage-video"),
            ("dp_photos", "defaultspage-photos"),
            ("dp_editor", "defaultspage-text-editor"),
            ("dp_files", "defaultspage-file-manager"),
            ("dp_terminal", "defaultspage-terminal"),
            ("dp_terminal_args", "defaultspage-terminal-arguments"),
            ("dp_drive_insert", "defaultspage-when-a-drive-is-plugged-in"),
            ("dp_script_open", "defaultspage-when-a-script-is-opened"),
            ("dp_script_end", "defaultspage-when-a-script-ends"),
            (
                "dp_confirm_scripts",
                "defaultspage-ask-before-running-a-program-the",
            ),
            ("dp_filter", "defaultspage-filter"),
            ("dp_change", "defaultspage-change"),
            ("dp_restore", "defaultspage-restore-default"),
        ],
        Page::Network => &[
            ("airplane", "pages-network-airplane-mode"),
            ("proxy_none", "pages-network-no-proxy"),
            ("proxy_manual", "pages-network-manual-configuration"),
            ("proxy_http", "pages-network-http"),
            ("proxy_http_port", "pages-network-port"),
            ("proxy_https", "pages-network-https"),
            ("proxy_https_port", "pages-network-port"),
            ("proxy_ftp", "pages-network-ftp"),
            ("proxy_ftp_port", "pages-network-port"),
            ("proxy_socks", "pages-network-socks"),
            ("proxy_socks_port", "pages-network-port"),
            (
                "proxy_ignore",
                "pages-network-ignore-hosts-separated-by-commas",
            ),
            ("proxy_auto", "pages-network-automatic-configuration"),
            ("proxy_pac", "pages-network-script-url"),
        ],
        Page::RemoteDesktop => &[
            (
                "allow",
                "pages-system-allow-remote-connections-to-this-computer",
            ),
            ("port", "pages-system-port"),
            ("print", "pages-system-certificate"),
            ("fingerprint", "pages-system-sha-256-fingerprint"),
            ("regenerate", "pages-system-new-certificate"),
            ("quality", "pages-system-picture-quality-2"),
            ("frame_rate", "pages-system-frame-rate"),
            ("video_encoder", "pages-system-video-encoder"),
            (
                "remote_sound",
                "pages-system-play-sound-on-the-remote-computer",
            ),
            ("clients", "pages-system-connected-computers"),
            ("disconnect", "pages-system-disconnect"),
            (
                "remote_lock",
                "pages-system-lock-this-computer-when-the-last",
            ),
            ("lock_grace", "pages-system-wait-before-locking-on-a-screen"),
            ("remote_border", "pages-system-show-a-red-border-while-a"),
            ("remote_screen_show", "pages-system-stays-on"),
            (
                "remote_screen_lock",
                "pages-system-locks-and-fits-the-remote-window",
            ),
            ("second_client_join", "pages-system-join-it"),
            ("second_client_replace", "pages-system-replace-it"),
        ],
        Page::Privacy => &[
            (
                "privacy_location",
                "pages-system-let-programs-ask-for-your-location",
            ),
            ("permission_forget", "pages-system-forget"),
            ("clipboard_apps", "pages-system-clipboard-access"),
            (
                "recent_files",
                "pages-system-show-recently-used-files-in-menus",
            ),
            ("clipboard_history", "pages-system-keep-a-clipboard-history"),
            (
                "clipboard_terminal_apps",
                "pages-system-paste-with-ctrl-shift-v-into",
            ),
            (
                "capture_border",
                "pages-system-show-a-red-border-around-what",
            ),
        ],
        Page::RegionLanguage => &[
            ("region_language", "regionpage-language"),
            ("region_formats", "regionpage-formats"),
            ("region_system", "regionpage-apply-to-the-system"),
        ],
        Page::LoginManager => &[
            ("login_interface", "pages-login-interface"),
            ("login_show_users", "pages-login-show-users"),
            ("login_animation", "pages-login-animate-the-screen"),
            ("login_autologin", "pages-login-log-on-automatically-as"),
            ("login_background", "pages-login-background"),
            ("login_browse", "pages-login-browse"),
            ("login_use_displays", "pages-login-use-my-display-settings"),
            ("login_reset_displays", "pages-login-reset"),
            (
                "login_remote",
                "pages-login-allow-remote-logins-to-this-computer",
            ),
            ("login_port", "pages-login-port"),
            ("login_sessions", "pages-login-sessions-at-most"),
            (
                "login_end_after",
                "pages-login-end-disconnected-sessions-after",
            ),
            ("login_fingerprint", "pages-login-certificate-fingerprint"),
            ("login_thumbprint", "pages-login-thumbprint-sha-1"),
        ],
        Page::Advanced => &[
            (
                "task_manager",
                "pages-system-task-manager-empty-runs-task-manager",
            ),
            (
                "restart_notice",
                "pages-system-tell-me-when-an-update-needs",
            ),
            ("standby", "pages-system-stand-by"),
            ("poweroff", "pages-system-turn-off"),
            ("reboot", "pages-system-restart"),
            ("hibernate", "pages-system-hibernate"),
            ("low_space_percent", "pages-system-warn-below"),
            (
                "thumbnail_cache",
                "pages-system-keep-thumbnails-of-pictures",
            ),
            ("thumbnail_max_mb", "pages-system-size"),
            ("thumbnail_clear", "pages-system-clear"),
            (
                "external_thumbnailers",
                "pages-system-make-thumbnails-of-documents-and-videos",
            ),
            (
                "debug_overlay",
                "pages-system-show-a-debug-overlay-frame-rate",
            ),
            (
                "debug_overlay_in_captures",
                "pages-system-show-it-in-screenshots-and-screen",
            ),
            ("edit_file", "pages-system-edit-file"),
        ],
        Page::Backup => &[
            ("backup_export", "pages-system-export"),
            ("backup_import", "pages-system-import"),
            ("backup_from_desktop", "pages-system-from-another-desktop"),
        ],
    }
}
