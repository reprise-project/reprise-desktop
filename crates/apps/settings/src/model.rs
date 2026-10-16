//! `config.toml` as the sheet sees it: one flat [`Settings`] of every key the pages edit, read from the text and written back with `toml_edit` so comments, key order and untouched tables survive.
//!
//! The defaults mirror the compositor's own (`Config::default` and the per-section defaults in `src/config.rs`, `Metrics` and `FontConfig` in `reprisekit::theme`, `glass::Settings`). This crate cannot depend on the compositor, so the lists change together; `src/config.rs` carries the same note above `Config`.

use repriseconf::{Key, Lens, get, key as k, lens, remove, set};
use reprisekit::interface::Interface;
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value};

/// One `[[keybinds]]` entry as the sheet edits it: the key string the
/// compositor parses ("Super+Shift+b"), the action name and, for `spawn`,
/// the command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub keys: String,
    pub action: String,
    pub command: String,
}

impl Shortcut {
    pub fn new(keys: &str, action: &str, command: &str) -> Shortcut {
        Shortcut {
            keys: keys.into(),
            action: action.into(),
            command: command.into(),
        }
    }
}

/// One `[[global_shortcuts]]` entry: a key a program bound through the
/// portal; `keys` is empty when unassigned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramShortcut {
    pub app: String,
    pub id: String,
    pub description: String,
    pub keys: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Bind(usize),
    Program(usize),
    Denied(usize),
}

/// The compositor's action names, `spawn` first since it is the one with
/// a command. Mirrors `Action::name` in `src/config.rs`.
pub const ACTIONS: [&str; 60] = [
    "spawn",
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
    "snap-left",
    "snap-right",
    "snap-up",
    "snap-down",
    "snap-layouts",
    "zoom-in",
    "zoom-out",
    "zoom-reset",
    "color-filter",
    "desktop-prev",
    "desktop-next",
    "desktop-carry-prev",
    "desktop-carry-next",
    "all-desktops",
    "screenshot",
    "screenshot-window",
    "screenshot-region",
    "screenshot-content",
    "volume-up",
    "volume-down",
    "mute",
    "mic-mute",
    "brightness-up",
    "brightness-down",
    "kbd-brightness-up",
    "kbd-brightness-down",
    "play-pause",
    "next",
    "prev",
    "stop",
    "wifi",
];

/// The compositor's default binds (`repriseconf::keybind::DEFAULTS`), which
/// a file's `[[keybinds]]` layer over.
fn default_shortcuts() -> Vec<Shortcut> {
    (repriseconf::keybind::DEFAULTS.iter())
        .map(|(k, a)| Shortcut::new(k, a, ""))
        .collect()
}

pub fn check_keys(keys: &str) -> Result<(), &'static str> {
    repriseconf::keybind::check(keys)
}

/// Why a program could not have `keys`, as `Err`: keys that would not
/// bind, or keys the compositor never gives a program (modifiers alone,
/// or kept by the desktop). Empty keys are fine: unassigned. `shown`
/// names the keys in the reason.
pub fn check_program_keys(keys: &str, shown: &str) -> Result<(), String> {
    if keys.is_empty() {
        return Ok(());
    }
    check_keys(keys)?;
    if repriseconf::keybind::modifier_only(keys) {
        return Err("A program shortcut ends in a key that is not a modifier.".into());
    }
    if repriseconf::keybind::reserved(keys) {
        return Err(format!("{shown} is kept by the desktop."));
    }
    Ok(())
}

pub fn program_text(p: &ProgramShortcut, name: &str) -> String {
    let what = if p.description.is_empty() {
        &p.id
    } else {
        &p.description
    };
    format!("{name}: {what}")
}

/// Every key the sheet edits, with the compositor's default in place of a
/// key the file leaves out. An empty `task_manager` means the key is
/// absent (the taskbar menu then runs `top` in the terminal).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub terminal: String,
    /// `script_terminal`: one of [`SCRIPT_ENDS`]' values.
    pub script_terminal: String,
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
    pub window_animation: bool,
    pub window_open_ms: i64,
    pub window_close_ms: i64,
    pub hover_fade: bool,
    pub hover_in_ms: i64,
    pub hover_out_ms: i64,
    pub taskbar_slide: bool,
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
            window_animation: false,
            window_open_ms: 250,
            window_close_ms: 200,
            hover_fade: false,
            hover_in_ms: 120,
            hover_out_ms: 250,
            taskbar_slide: false,
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

/// A combo's items: the value the config takes, then the text the combo
/// shows for it.
pub type Choices = [(&'static str, &'static str)];

/// The taskbar button modes. Mirrors `TaskbarButtons::parse` in
/// `src/config.rs`.
pub const TASKBAR_BUTTONS: &Choices = &[
    ("combine", "Combine, icons only"),
    ("when_full", "Combine when the taskbar is full"),
    ("never", "Never combine"),
    ("group", "Group, never combine"),
];

/// The Start menu styles. Mirrors `StartMenuStyle::parse` in `src/config.rs`.
pub const STARTMENU_STYLES: &Choices = &[("glass", "Glass"), ("classic", "Classic")];

/// The edges the taskbar can sit on. Mirrors `Edge::name` in
/// `crates/reprisekit/src/taskbar.rs`.
pub const TASKBAR_POSITIONS: &Choices = &[
    ("bottom", "Bottom"),
    ("top", "Top"),
    ("left", "Left"),
    ("right", "Right"),
];

pub const SWITCHER_STYLES: &Choices = &[("icons", "Icons"), ("thumbnails", "Thumbnails")];

pub const SWITCHER_SIZES: &Choices = &[
    ("small", "Small"),
    ("medium", "Medium"),
    ("large", "Large"),
    ("huge", "Huge"),
];

/// The overview's hot corner combo. Mirrors `HotCorner::name` in
/// `src/config.rs`.
pub const HOT_CORNERS: &Choices = &[
    ("none", "None"),
    ("top-left", "Top left"),
    ("top-right", "Top right"),
    ("bottom-left", "Bottom left"),
    ("bottom-right", "Bottom right"),
];

/// The desktop switch animations. Mirrors `SwitchStyle::name` in
/// `src/config.rs`.
pub const SWITCH_ANIMATIONS: &Choices = &[
    ("none", "None"),
    ("slide", "Slide"),
    ("slide-vertical", "Slide vertically"),
    ("fade", "Fade"),
    ("zoom", "Zoom"),
    ("wipe", "Wipe"),
];

/// How the Glass taskbar marks the current desktop. Mirrors
/// `PagerMark::name` in `crates/reprisekit/src/taskbar.rs`.
pub const PAGER_MARKS: &Choices = &[("light", "Light under its number"), ("tile", "Glass tile")];

/// The text sizes `[accessibility] text_size` takes. Mirrors
/// `TEXT_SIZE` in `src/config.rs`.
pub const TEXT_SIZES: &[(i64, &str)] = &[
    (100, "100%"),
    (125, "125%"),
    (150, "150%"),
    (175, "175%"),
    (200, "200%"),
];

pub const CURSOR_SIZES: &[(i64, &str)] = &[
    (0, "Default"),
    (24, "24 px"),
    (32, "32 px"),
    (48, "48 px"),
    (64, "64 px"),
    (96, "96 px"),
];

/// The value of a `[toolkits]` key that follows the look in force.
/// Mirrors `TOOLKIT_AUTO` in `src/config.rs`.
pub const TOOLKIT_AUTO: &str = "auto";

/// The value of a `[toolkits]` key that recolours GTK or Qt programs in
/// the look's own colours. Mirrors `TOOLKIT_REPRISE` in `src/config.rs`.
pub const TOOLKIT_REPRISE: &str = "reprise";

pub const REPRISE_TOOLKIT_ROW: &str = "Reprise (follows the look)";

/// `[toolkits] qt_platform_theme`: "auto", "reprise", then the Qt
/// platform theme plugins a desktop Linux ships. Any other name shows as
/// itself.
pub const QT_PLATFORM_THEMES: &Choices = &[
    (TOOLKIT_AUTO, "Automatic (desktop portal when dark)"),
    (TOOLKIT_REPRISE, REPRISE_TOOLKIT_ROW),
    ("xdgdesktopportal", "Desktop portal"),
    ("qt6ct", "qt6ct"),
    ("qt5ct", "qt5ct"),
    ("kde", "KDE Plasma"),
    ("gtk3", "GTK 3"),
];

/// `[toolkits] font_smoothing`: how GTK and the browsers smooth their
/// text. Mirrors `FontSmoothing::parse` in `src/config.rs`.
pub const FONT_SMOOTHINGS: &Choices = &[
    ("grayscale", "Grayscale"),
    ("subpixel", "Subpixel (RGB)"),
    ("none", "None"),
];

/// `[glass] mode`. Mirrors `glass::Mode::as_str`.
pub const GLASS_MODES: &Choices = &[("light", "Light"), ("dark", "Dark")];

/// `[glass] material`. Mirrors `glass::Material::as_str`.
pub const GLASS_MATERIALS: &Choices = &[
    ("frosted", "Frosted"),
    ("smoked", "Smoked"),
    ("molten", "Molten"),
];

/// `[glass] frame`. Mirrors `glass::Frame::as_str`.
pub const GLASS_FRAMES: &Choices = &[("rounded", "Rounded"), ("square", "Square")];

/// The Interface combo's items. Mirrors `Interface::parse` in reprisekit.
pub const INTERFACES: &Choices = &[
    ("classic", "Reprise Classic"),
    ("blue", "Reprise Blue"),
    ("glass", "Reprise Glass"),
];

pub const WALLPAPER_MODES: &Choices = &[("picture", "Picture"), ("slideshow", "Slideshow")];

pub const SLIDE_INTERVALS: &[(i64, &str)] = &[
    (1, "1 minute"),
    (5, "5 minutes"),
    (10, "10 minutes"),
    (30, "30 minutes"),
    (60, "1 hour"),
    (360, "6 hours"),
    (1440, "1 day"),
];

/// The wallpaper Position combo's items. Mirrors `Position::parse` in
/// reprisekit.
pub const POSITIONS: &Choices = &[
    ("center", "Center"),
    ("tile", "Tile"),
    ("stretch", "Stretch"),
];

/// Settings > Advanced's desktop water styles; unchecked is "off".
/// Mirrors `reprisekit::water::Water::parse`.
pub const WATER_STYLES: &Choices = &[("calm", "Calm"), ("lively", "Lively"), ("rain", "Rain")];

/// The Background page's file name extensions combo. Mirrors `Extensions::parse` in
/// `reprisefs::names`.
pub const DESKTOP_EXTENSIONS: &Choices = &[
    ("hide", "Hide"),
    ("hide_launchers", "Hide for launchers only"),
    ("show", "Show"),
];

/// Default Programs' plugged-in drive combo. Mirrors `OnInsert::parse` in reprise-drives.
pub const DRIVE_INSERTS: &Choices = &[
    ("ask", "Ask what to do"),
    ("mount", "Mount it"),
    ("open", "Mount it and open a window"),
    ("nothing", "Do nothing"),
];

/// Default Programs' script combo. Mirrors `ScriptEnd::parse` in `reprisefs::program`.
pub const SCRIPT_ENDS: &Choices = &[
    ("stay", "Keep its window open"),
    ("close", "Close its window"),
];

/// The system icons the Background page ticks, in the desktop's order,
/// and the Recycle Bin, which always follows them. Mirror
/// `Config::default`'s `system_icons`.
pub const SYSTEM_ICONS: [&str; 3] = ["documents", "computer", "home"];
pub const RECYCLE_BIN: &str = "recycle-bin";

/// `icons` with `name` ticked or not, in the order the file had: an
/// untick drops `name`, a tick adds it at the end when it is missing,
/// and the Recycle Bin is added at the end when it had gone. Every other
/// name, one the desktop does not know included, keeps its place.
pub fn system_icons_with(icons: &[String], name: &str, on: bool) -> Vec<String> {
    let mut out: Vec<String> = icons.iter().filter(|i| on || *i != name).cloned().collect();
    if on && !out.iter().any(|i| i == name) {
        out.push(name.to_string());
    }
    if !out.iter().any(|i| i == RECYCLE_BIN) {
        out.push(RECYCLE_BIN.to_string());
    }
    out
}

/// The swatch `[glass] color = v` stands for, by name or by a colour
/// equal to a swatch's; None for any other colour.
pub fn glass_swatch(v: &str) -> Option<&'static str> {
    reprisekit::glass::swatch(v)
        .or_else(|| reprisekit::theme::Color::parse(v))
        .and_then(reprisekit::glass::swatch_name)
}

/// `[workspaces] count`'s default and ceiling. Mirror `Config::default`
/// and `WORKSPACES_MAX` in `src/config.rs`.
pub const DESKTOPS_DEFAULT: i64 = 4;
pub const DESKTOPS_MAX: i64 = 16;

/// Longest `[workspaces] animation_ms`. Mirrors `SWITCH_MS_MAX`.
pub const SWITCH_MS_MAX: i64 = 2000;

/// Longest `[effects]` duration. Mirrors `EFFECTS_MS_MAX`.
pub const EFFECTS_MS_MAX: i64 = 2000;

/// Most `[previews] fps`. Mirrors `PREVIEW_FPS_MAX`.
pub const PREVIEW_FPS_MAX: i64 = 60;

/// One field's key and how to move it in and out of a `Settings`. The
/// list is the single place a key name appears, so `load` and `save`
/// cannot drift apart.
enum Field {
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

const INTERFACE: Key = k("appearance", "interface");

fn fields() -> Vec<Field> {
    vec![
        Field::Text(k("", "terminal"), lens!(terminal)),
        Field::Text(k("", "script_terminal"), lens!(script_terminal)),
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
        Field::Flag(k("effects", "window_animation"), lens!(window_animation)),
        Field::Int(k("effects", "window_open_ms"), lens!(window_open_ms)),
        Field::Int(k("effects", "window_close_ms"), lens!(window_close_ms)),
        Field::Flag(k("effects", "hover_fade"), lens!(hover_fade)),
        Field::Int(k("effects", "hover_in_ms"), lens!(hover_in_ms)),
        Field::Int(k("effects", "hover_out_ms"), lens!(hover_out_ms)),
        Field::Flag(k("effects", "taskbar_slide"), lens!(taskbar_slide)),
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
    fn key(&self) -> &Key {
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

    fn value(&self, s: &Settings) -> Value {
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

    fn same(&self, a: &Settings, b: &Settings) -> bool {
        match self {
            Field::Text(_, at) | Field::List(_, at) | Field::Words(_, at) => at.get(a) == at.get(b),
            Field::Int(_, at) | Field::Tenths(_, at, _) => at.get(a) == at.get(b),
            Field::Flag(_, at) => at.get(a) == at.get(b),
            Field::Names(_, at) => at.get(a) == at.get(b),
        }
    }

    fn copy(&self, from: &Settings, to: &mut Settings) {
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

fn list_to_text(item: &Item) -> Option<String> {
    let arr = item.as_array()?;
    Some(
        arr.iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    )
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    (text.split([',', ' ', '\t'])).filter(|w| !w.is_empty())
}

pub(crate) fn text_to_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn defaults_in(doc: &DocumentMut) -> Settings {
    Settings::for_interface(get(doc, &INTERFACE).and_then(Item::as_str).unwrap_or(""))
}

/// Read every key the sheet knows out of `text`, leaving the rest of the
/// defaults of its interface in place. A file that does not parse gives
/// the defaults whole, as the compositor's own `Config::load` does.
pub fn load(text: &str) -> Settings {
    let Ok(doc) = text.parse::<DocumentMut>() else {
        log::warn!("config.toml does not parse; showing defaults");
        return Settings::default();
    };
    let mut s = defaults_in(&doc);
    for f in fields() {
        let Some(item) = get(&doc, f.key()) else {
            continue;
        };
        match &f {
            Field::Text(_, at) => {
                if let Some(v) = item.as_str() {
                    *at.get_mut(&mut s) = v.to_string();
                }
            }
            Field::Int(_, at) => {
                if let Some(v) = item.as_integer() {
                    *at.get_mut(&mut s) = v;
                }
            }
            Field::Flag(_, at) => {
                if let Some(v) = item.as_bool() {
                    *at.get_mut(&mut s) = v;
                }
            }
            Field::List(_, at) => {
                if let Some(v) = list_to_text(item) {
                    *at.get_mut(&mut s) = v;
                }
            }
            Field::Words(_, at) => {
                if let Some(arr) = item.as_array() {
                    let w: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
                    *at.get_mut(&mut s) = w.join(" ");
                }
            }
            Field::Tenths(_, at, (lo, hi)) => {
                // `speed = 1` is a valid float to the compositor too.
                let v = item
                    .as_float()
                    .or_else(|| item.as_integer().map(|i| i as f64));
                if let Some(v) = v {
                    *at.get_mut(&mut s) = ((v * 10.0).round() as i64).clamp(*lo, *hi);
                }
            }
            // An older file's icon names read as the current ones.
            Field::Names(_, at) => {
                if let Some(arr) = item.as_array() {
                    *at.get_mut(&mut s) = repriseconf::system_icons::current_list(
                        arr.iter().filter_map(|v| v.as_str()),
                    );
                }
            }
        }
    }
    if let Some(list) = get(&doc, &KEYBINDS).and_then(read_shortcuts) {
        s.shortcuts = repriseconf::keybind::layered(
            list,
            |b| (&b.keys, &b.action),
            |k, a| Shortcut::new(k, a, ""),
        );
    }
    if let Some(list) = get(&doc, &GLOBALS).and_then(Item::as_array_of_tables) {
        s.programs = read_programs(list);
    }
    if let Some(arr) = get(&doc, &DENIED).and_then(Item::as_array) {
        s.denied = arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::to_string)
            .collect();
    }
    if let Some(menu) = crate::startmenu::read(&doc) {
        s.startmenu = menu;
    }
    s.tablet_pads = crate::tabletpage::read(&doc);
    if glass_swatch(&s.glass_color).is_none() {
        s.glass_custom = s.glass_color.clone();
    }
    s
}

const KEYBINDS: Key = k("", "keybinds");

/// `[[keybinds]]` (or an inline `keybinds = [{...}]`) as shortcuts; None
/// when the item is neither. An entry without `keys` or `action` is
/// skipped (the compositor would reject the file).
fn read_shortcuts(item: &Item) -> Option<Vec<Shortcut>> {
    let tables: Vec<&dyn toml_edit::TableLike> = if let Some(a) = item.as_array_of_tables() {
        a.iter().map(|t| t as &dyn toml_edit::TableLike).collect()
    } else {
        item.as_array()?
            .iter()
            .filter_map(|v| v.as_inline_table().map(|t| t as &dyn toml_edit::TableLike))
            .collect()
    };
    let text = |t: &dyn toml_edit::TableLike, name: &str| {
        t.get(name).and_then(Item::as_str).map(str::to_string)
    };
    Some(
        tables
            .iter()
            .filter_map(|t| {
                Some(Shortcut {
                    keys: text(*t, "keys")?,
                    action: text(*t, "action")?,
                    command: text(*t, "command").unwrap_or_default(),
                })
            })
            .collect(),
    )
}

/// The `[[keybinds]]` entries that layer over the defaults into `binds`:
/// each default chord `binds` holds otherwise (or "none" when it holds
/// nothing there), then the binds on no default chord.
fn keybind_entries(binds: &[Shortcut]) -> Vec<Shortcut> {
    use repriseconf::keybind::{DEFAULTS, NONE, same};
    let mut out = Vec::new();
    for &(keys, action) in DEFAULTS {
        let mine: Vec<&Shortcut> = binds.iter().filter(|b| same(&b.keys, keys)).collect();
        match mine[..] {
            [] => out.push(Shortcut::new(keys, NONE, "")),
            [b] if *b == Shortcut::new(keys, action, "") => {}
            _ => out.extend(mine.into_iter().cloned()),
        }
    }
    let on_a_default = |b: &Shortcut| DEFAULTS.iter().any(|(k, _)| same(&b.keys, k));
    out.extend(binds.iter().filter(|b| !on_a_default(b)).cloned());
    out
}

fn shortcut_tables(list: &[Shortcut]) -> ArrayOfTables {
    let mut out = ArrayOfTables::new();
    for sc in list {
        let mut t = Table::new();
        t["keys"] = toml_edit::value(&sc.keys);
        t["action"] = toml_edit::value(&sc.action);
        if sc.action == "spawn" {
            t["command"] = toml_edit::value(&sc.command);
        }
        out.push(t);
    }
    out
}

const GLOBALS: Key = k("", "global_shortcuts");
const DENIED: Key = k("portal", "shortcuts_denied");

fn read_programs(list: &ArrayOfTables) -> Vec<ProgramShortcut> {
    let text = |t: &Table, name: &str| t.get(name).and_then(Item::as_str).map(str::to_string);
    list.iter()
        .filter_map(|t| {
            Some(ProgramShortcut {
                app: text(t, "app")?,
                id: text(t, "id")?,
                description: text(t, "description").unwrap_or_default(),
                keys: text(t, "keys").unwrap_or_default(),
            })
        })
        .collect()
}

fn program_tables(list: &[ProgramShortcut]) -> ArrayOfTables {
    let mut out = ArrayOfTables::new();
    for p in list {
        let mut t = Table::new();
        t["app"] = toml_edit::value(&p.app);
        t["id"] = toml_edit::value(&p.id);
        t["description"] = toml_edit::value(&p.description);
        t["keys"] = toml_edit::value(&p.keys);
        out.push(t);
    }
    out
}

/// `want`'s edits since `base`, applied to what the file holds now
/// (`have`): the portal may have added programs while the sheet was open.
/// The sheet only changes keys and removes entries.
fn merge_programs(
    want: &[ProgramShortcut],
    base: &[ProgramShortcut],
    have: &[ProgramShortcut],
) -> Vec<ProgramShortcut> {
    let same = |a: &ProgramShortcut, b: &ProgramShortcut| a.app == b.app && a.id == b.id;
    have.iter()
        .filter(|h| !base.iter().any(|b| same(b, h)) || want.iter().any(|w| same(w, h)))
        .map(|h| {
            match (
                want.iter().find(|w| same(w, h)),
                base.iter().find(|b| same(b, h)),
            ) {
                (Some(w), Some(b)) if w != b => w.clone(),
                _ => h.clone(),
            }
        })
        .collect()
}

fn merge_denied(want: &[String], base: &[String], have: &[String]) -> Vec<String> {
    have.iter()
        .filter(|h| !base.contains(h) || want.contains(h))
        .cloned()
        .collect()
}

/// What the Shortcuts list's row `i` is: the desktop binds, then each
/// program shortcut, then each denied program.
pub fn row(s: &Settings, i: usize) -> Option<Row> {
    let (b, p) = (s.shortcuts.len(), s.programs.len());
    if i < b {
        Some(Row::Bind(i))
    } else if i < b + p {
        Some(Row::Program(i - b))
    } else if i < b + p + s.denied.len() {
        Some(Row::Denied(i - b - p))
    } else {
        None
    }
}

/// Delete on a row: a bind goes; a program forgets every shortcut of
/// that program, or its refusal, so it asks again.
pub fn forget(s: &mut Settings, r: Row) {
    match r {
        Row::Bind(i) => {
            s.shortcuts.remove(i);
        }
        Row::Program(i) => {
            let app = s.programs[i].app.clone();
            s.programs.retain(|p| p.app != app);
        }
        Row::Denied(i) => {
            s.denied.remove(i);
        }
    }
}

/// Write into `text` the fields the sheet moved, `want` against `base`, touching as little as possible: keys the sheet did not move are left alone even if the file changed under it, keys moved back to the interface's default are removed (and tables left empty), tables are created only when a key needs one.
///
/// `Err` when `text` does not parse; the caller must not write then.
pub fn save(text: &str, want: &Settings, base: &Settings) -> Result<String, String> {
    let mut doc = text.parse::<DocumentMut>().map_err(|e| e.to_string())?;
    // What the file says today, with the defaults standing in for the
    // keys it leaves out.
    let have = load(text);
    // The defaults of the interface the file names once this save lands:
    // the sheet's when it moved it, else the one on disk.
    let moved = want.interface != base.interface && want.interface != have.interface;
    let def = Settings::for_interface(if moved {
        &want.interface
    } else {
        &have.interface
    });
    // The tables a key was removed from.
    let mut shrunk: Vec<&'static str> = Vec::new();
    let mut drop_key = |doc: &mut DocumentMut, key: &Key| {
        if get(doc, key).is_some() {
            remove(doc, key);
            shrunk.push(key.table);
        }
    };
    for f in fields() {
        if f.same(want, base) || f.same(want, &have) {
            continue;
        }
        if f.same(want, &def) {
            drop_key(&mut doc, f.key());
        } else {
            set(&mut doc, f.key(), f.value(want)).map_err(|e| e.to_string())?;
        }
    }
    for table in shrunk {
        remove_if_empty(&mut doc, table);
    }
    // The list is written whole or not at all: a table's comments do not
    // survive a rewrite anyway.
    if want.shortcuts != base.shortcuts && want.shortcuts != have.shortcuts {
        let entries = keybind_entries(&want.shortcuts);
        if entries.is_empty() {
            remove(&mut doc, &KEYBINDS);
        } else {
            doc[KEYBINDS.name] = Item::ArrayOfTables(shortcut_tables(&entries));
        }
    }
    // Merged per entry: the portal writes these too.
    if want.programs != base.programs {
        let list = merge_programs(&want.programs, &base.programs, &have.programs);
        if list.is_empty() {
            remove(&mut doc, &GLOBALS);
        } else {
            doc[GLOBALS.name] = Item::ArrayOfTables(program_tables(&list));
        }
    }
    if want.denied != base.denied {
        let list = merge_denied(&want.denied, &base.denied, &have.denied);
        if list.is_empty() {
            remove(&mut doc, &DENIED);
            remove_if_empty(&mut doc, DENIED.table);
        } else {
            let arr: Array = list.iter().map(String::as_str).collect();
            set(&mut doc, &DENIED, Value::Array(arr)).map_err(|e| e.to_string())?;
        }
    }
    crate::startmenu::write(&mut doc, &want.startmenu, &base.startmenu, &have.startmenu);
    crate::tabletpage::write(&mut doc, &want.tablet_pads, &base.tablet_pads);
    Ok(doc.to_string())
}

/// `config.toml` changed under the sheet: every value `want` still holds as `base` (what the sheet loaded or last saved) takes `fresh`'s, the file as it reads now; a value the user moved stays pending.
pub fn rebase(want: &mut Settings, base: &Settings, fresh: &Settings) {
    fn follow<T: PartialEq + Clone>(want: &mut T, base: &T, fresh: &T) {
        if want == base {
            *want = fresh.clone();
        }
    }
    for f in fields() {
        if f.same(want, base) {
            f.copy(fresh, want);
        }
    }
    follow(
        &mut want.glass_custom,
        &base.glass_custom,
        &fresh.glass_custom,
    );
    follow(&mut want.shortcuts, &base.shortcuts, &fresh.shortcuts);
    follow(&mut want.programs, &base.programs, &fresh.programs);
    follow(&mut want.denied, &base.denied, &fresh.denied);
    follow(&mut want.startmenu, &base.startmenu, &fresh.startmenu);
    follow(&mut want.tablet_pads, &base.tablet_pads, &fresh.tablet_pads);
}

/// The sheet's interface moved from `from` to `s.interface`: the scheme goes back to the new interface's own, and every field `text` leaves unset that held `from`'s default takes the new one's. Moved back to `loaded`'s interface, the scheme and glass colour are `loaded`'s again.
pub fn switch_interface(s: &mut Settings, from: &str, text: &str, loaded: &Settings) {
    let doc = text
        .parse::<DocumentMut>()
        .unwrap_or_else(|_| DocumentMut::new());
    let (old, new) = (
        Settings::for_interface(from),
        Settings::for_interface(&s.interface),
    );
    for f in fields() {
        if get(&doc, f.key()).is_none() && f.same(s, &old) {
            f.copy(&new, s);
        }
    }
    if s.interface == loaded.interface {
        s.color_scheme = loaded.color_scheme.clone();
        s.glass_color = loaded.glass_color.clone();
    } else {
        s.color_scheme.clear();
    }
}

/// Remove the table at `path` (dotted for a nested one) when nothing is left in it.
fn remove_if_empty(doc: &mut DocumentMut, path: &'static str) {
    let (parent, name) = path.rsplit_once('.').unwrap_or(("", path));
    let key = k(parent, name);
    let empty = get(doc, &key)
        .and_then(Item::as_table_like)
        .is_some_and(|t| t.is_empty());
    if !name.is_empty() && empty {
        remove(doc, &key);
    }
}

/// The compositor's own parse of each spec in `text`, at most `ZONE_MAX` of them.
fn check_snap_layouts(text: &str) -> Result<(), String> {
    use reprisekit::snaplayouts::{ZONE_MAX, ZoneLayout};
    let specs: Vec<&str> = words(text).collect();
    if specs.len() > ZONE_MAX {
        return Err(format!(
            "Snap layouts: at most {ZONE_MAX} layouts, got {}.",
            specs.len()
        ));
    }
    for spec in specs {
        ZoneLayout::parse(spec).map_err(|e| format!("Snap layouts: {e}."))?;
    }
    Ok(())
}

/// What Apply refuses before writing: a bind's key that will not bind, or a program key changed from `base` that fails `check_program_keys`.
pub fn validate(s: &Settings, base: &Settings) -> Result<(), String> {
    if s.nl_from == s.nl_to {
        return Err("The night light's From and To times must differ.".into());
    }
    for sc in &s.shortcuts {
        check_keys(&sc.keys)?;
        if !ACTIONS.contains(&sc.action.as_str()) {
            return Err("A shortcut's action is not one the compositor knows.".into());
        }
        if sc.action == "spawn" && sc.command.trim().is_empty() {
            return Err("A spawn shortcut needs a command.".into());
        }
    }
    check_snap_layouts(&s.snap_layouts)?;
    let as_loaded = |p: &ProgramShortcut| {
        (base.programs.iter()).any(|b| b.app == p.app && b.id == p.id && b.keys == p.keys)
    };
    for p in s.programs.iter().filter(|p| !as_loaded(p)) {
        if let Err(why) = check_program_keys(&p.keys, &p.keys) {
            let name = if p.description.is_empty() {
                &p.id
            } else {
                &p.description
            };
            return Err(format!("{name}: {why}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_list_adds_moves_and_removes_through_layout_and_variant() {
        let mut s = Settings::default();
        assert!(s.layouts().is_empty(), "the system default");
        assert!(s.add_layout("us", ""));
        assert!(s.add_layout("hu", "qwerty"));
        assert_eq!(
            (s.kb_layout.as_str(), s.kb_variant.as_str()),
            ("us,hu", ",qwerty")
        );
        assert!(!s.move_layout_up(0));
        assert!(s.move_layout_up(1));
        assert_eq!(
            (s.kb_layout.as_str(), s.kb_variant.as_str()),
            ("hu,us", "qwerty,")
        );
        s.remove_layout(0);
        assert_eq!((s.kb_layout.as_str(), s.kb_variant.as_str()), ("us", ""));
        assert!(s.add_layout("de", ""));
        assert!(s.add_layout("fr", ""));
        assert!(s.add_layout("it", ""));
        assert!(!s.add_layout("es", ""), "four at most");
        assert_eq!(s.kb_layout, "us,de,fr,it");
        for _ in 0..4 {
            s.remove_layout(0);
        }
        assert_eq!((s.kb_layout.as_str(), s.kb_variant.as_str()), ("", ""));
    }

    #[test]
    fn banner_default_matches_the_compositor() {
        assert_eq!(Settings::default().startmenu_banner, "Reprise Desktop");
    }

    /// A commented file with a default key and a non-default one: what `save` must leave alone.
    const SAMPLE: &str = "\
# Reprise configuration.

terminal = \"foot\"           # what Super+Return runs
desktop_icons = true          # the default, spelled out

[fonts]
family = [\"Tahoma\", \"DejaVu Sans\"]
size_px = 12

[idle]                        # seconds, 0 disables
lock_after = 0
";

    /// Comments above the keys they document, as `config.example.toml` writes them.
    const COMMENTED: &str = "\
# Reprise configuration. Every key is optional.

terminal = \"foot\"
# task_manager = \"gnome-system-monitor\"   # default: top in the terminal
desktop_icons = false         # the desktop shows no icons

[metrics]
frame_width = 5
# how tall the taskbar is
taskbar_height = 32

[fonts]
size_px = 12
";

    fn saved(text: &str, want: &Settings) -> String {
        save(text, want, &load(text)).expect("the sample parses")
    }

    #[test]
    fn effects_defaults_follow_the_files_interface() {
        let classic = load("");
        assert!(!classic.window_animation && !classic.hover_fade);
        assert_eq!(
            (classic.window_open_ms, classic.window_close_ms),
            (250, 200)
        );
        assert_eq!((classic.hover_in_ms, classic.hover_out_ms), (120, 250));
        assert!(!classic.taskbar_slide);
        let glass = load("[appearance]\ninterface = \"glass\"\n");
        assert!(glass.window_animation && glass.hover_fade && glass.taskbar_slide);
    }

    #[test]
    fn effects_are_written_against_the_interfaces_defaults() {
        let text = "[appearance]\ninterface = \"glass\"\n";
        let mut want = load(text);
        want.hover_fade = false;
        want.window_open_ms = 400;
        let out = saved(text, &want);
        assert!(out.contains("[effects]"), "{out}");
        assert!(out.contains("hover_fade = false"), "{out}");
        assert!(out.contains("window_open_ms = 400"), "{out}");
        assert!(
            !out.contains("window_animation"),
            "Glass's default stays unwritten: {out}"
        );
        assert_eq!(load(&out), want);
        let base = want.clone();
        want.hover_fade = true;
        want.window_open_ms = 250;
        let out = save(&out, &want, &base).expect("parses");
        assert!(
            !out.contains("hover_fade") && !out.contains("window_open_ms"),
            "{out}"
        );
    }

    #[test]
    fn a_table_whose_last_key_goes_back_to_its_default_goes_too() {
        let text = "[appearance]\ninterface = \"glass\"\n";
        let mut want = load(text);
        want.hover_fade = false;
        let out = saved(text, &want);
        assert!(out.contains("[effects]"), "{out}");
        let base = want.clone();
        want.hover_fade = true;
        let out = save(&out, &want, &base).expect("parses");
        assert_eq!(out, text, "no bare [effects] is left");
    }

    #[test]
    fn save_carries_a_removed_keys_comments_on_to_the_next_key() {
        let mut s = load(COMMENTED);
        s.desktop_icons = true;
        let out = saved(COMMENTED, &s);

        assert_eq!(
            out,
            COMMENTED.replace(
                "desktop_icons = false         # the desktop shows no icons\n",
                ""
            ),
            "byte-identical but for the removed key's own line"
        );
        assert!(out.starts_with("# Reprise configuration. Every key is optional.\n"));
        assert!(out.contains("# task_manager = \"gnome-system-monitor\""));
    }

    #[test]
    fn save_carries_the_comments_of_a_tables_last_key_past_the_table() {
        let mut s = load(COMMENTED);
        s.taskbar_height = 28;
        let out = saved(COMMENTED, &s);

        assert_eq!(
            out,
            COMMENTED.replace("taskbar_height = 32\n", ""),
            "the comment lands above the next table, not in the bin"
        );
    }

    #[test]
    fn save_carries_the_comments_of_the_last_key_in_the_file() {
        let text =
            "[fonts]\nsize_px = 12\n# the font size above is the only key here\nantialias = true\n";
        let mut s = load(text);
        s.font_antialias = false;
        let out = save(text, &s, &load(text)).expect("parses");

        assert_eq!(out, text.replace("antialias = true\n", ""));
    }

    #[test]
    fn rebase_takes_the_files_values_and_keeps_the_ones_the_sheet_moved() {
        let base = load("");
        let mut want = base.clone();
        want.sticky_keys = true;
        let edited = "[accessibility]\nmouse_keys = true\nsticky_keys = false\n";
        let fresh = load(edited);
        rebase(&mut want, &base, &fresh);
        assert!(want.mouse_keys, "an untouched value follows the file");
        assert!(want.sticky_keys, "a moved value stays pending");
        let out = save(edited, &want, &fresh).expect("parses");
        assert_eq!(
            out, "[accessibility]\nmouse_keys = true\nsticky_keys = true\n",
            "Apply writes only the moved value"
        );
    }

    #[test]
    fn save_leaves_a_key_an_editor_changed_meanwhile() {
        let opened = "terminal = \"foot\"\n[fonts]\nsize_px = 12\n";
        let base = load(opened);
        let mut want = base.clone();
        want.font_size_px = 14;
        let edited = "terminal = \"kitty\"\n[fonts]\nsize_px = 12\n";

        let out = save(edited, &want, &base).expect("parses");
        assert!(out.contains("terminal = \"kitty\""), "{out}");
        assert!(out.contains("size_px = 14"), "{out}");
    }

    #[test]
    fn save_keeps_glass_blur_rate_when_it_writes_the_glass_table() {
        let text =
            "[appearance]\ninterface = \"glass\"\n\n[glass]\nblur_rate = 0\nintensity = 30\n";
        let mut want = load(text);
        want.glass_intensity = 75;
        want.glass_fast_drag = false;
        want.glass_blur_frames = false;
        want.glass_orb_glow = 50;

        let out = saved(text, &want);
        assert!(out.contains("blur_rate = 0\n"), "{out}");
        assert!(out.contains("orb_glow = 50"), "{out}");
        assert_eq!(load(&out).glass_orb_glow, 50, "the glow reads back");
        assert!(out.contains("intensity = 75"), "{out}");
        assert!(out.contains("fast_drag = false"), "{out}");
        assert!(out.contains("blur_frames = false"), "{out}");
        assert!(!load(&out).glass_blur_frames, "it reads back off");
    }

    #[test]
    fn save_refuses_a_file_that_does_not_parse() {
        let broken = "terminal = \"foot\"\n[metrics\nframe_width = 4\n";
        let mut s = load(broken);
        s.terminal = "kitty".into();

        let err = save(broken, &s, &Settings::default())
            .expect_err("a broken file is not ours to rewrite");
        assert!(!err.is_empty());
    }

    #[test]
    fn wallpaper_water_round_trips() {
        for v in ["calm", "lively", "rain", "off"] {
            let mut s = load("");
            s.wallpaper_water = v.into();
            let out = saved("", &s);
            if v != "off" {
                assert!(out.contains(&format!("water = \"{v}\"")), "{out}");
            }
            assert_eq!(load(&out).wallpaper_water, v);
        }
        assert_eq!(load("").wallpaper_water, "off");
    }

    #[test]
    fn load_reads_the_keys_it_knows() {
        let s = load(SAMPLE);
        assert_eq!(s.terminal, "foot");
        assert_eq!(s.font_family, "Tahoma, DejaVu Sans");
        assert_eq!(s.font_size_px, 12);
        assert_eq!(s.lock_after, 0);
    }

    #[test]
    fn load_falls_back_to_the_compositors_defaults() {
        let s = load("");
        assert_eq!(s, Settings::default());
        assert_eq!(s.terminal, "alacritty");
        assert_eq!(s.taskbar_height, 28);
        assert_eq!(s.screen_off_after, 900);
        assert_eq!(s.task_manager, "", "an absent key is the empty string");
    }

    #[test]
    fn load_ignores_a_file_that_does_not_parse() {
        assert_eq!(load("terminal = "), Settings::default());
    }

    #[test]
    fn save_leaves_an_unchanged_file_byte_identical() {
        let s = load(SAMPLE);
        assert_eq!(saved(SAMPLE, &s), SAMPLE);
    }

    #[test]
    fn save_rewrites_only_the_key_that_changed() {
        let mut s = load(SAMPLE);
        s.terminal = "kitty".into();
        let out = saved(SAMPLE, &s);
        assert_eq!(out, SAMPLE.replace("\"foot\"", "\"kitty\""));
        assert!(
            out.contains("# what Super+Return runs"),
            "the trailing comment survives: {out}"
        );
    }

    #[test]
    fn save_removes_a_key_that_is_back_to_its_default() {
        let mut s = load(SAMPLE);
        s.font_size_px = 11;
        let out = saved(SAMPLE, &s);
        assert!(!out.contains("size_px"), "{out}");
        assert!(out.contains("family = [\"Tahoma\", \"DejaVu Sans\"]"));
    }

    #[test]
    fn save_creates_a_table_only_when_a_key_needs_one() {
        let mut s = load(SAMPLE);
        let untouched = saved(SAMPLE, &s);
        assert!(!untouched.contains("[power]"), "nothing needed it");

        s.poweroff = "loginctl poweroff".into();
        let out = saved(SAMPLE, &s);
        assert!(out.contains("[power]"));
        assert!(out.contains("poweroff = \"loginctl poweroff\""));
        assert!(!out.contains("standby"), "the defaults stay out: {out}");
    }

    #[test]
    fn save_writes_the_font_family_back_as_an_array() {
        let mut s = load(SAMPLE);
        s.font_family = "Verdana, Tahoma".into();
        let out = saved(SAMPLE, &s);
        assert!(out.contains("family = [\"Verdana\", \"Tahoma\"]"), "{out}");
        assert_eq!(load(&out).font_family, "Verdana, Tahoma");
    }

    #[test]
    fn save_starts_a_file_from_nothing() {
        let s = Settings {
            taskbar_height: 32,
            ..Settings::default()
        };
        let out = saved("", &s);
        assert_eq!(out, "[metrics]\ntaskbar_height = 32\n");
        assert_eq!(load(&out).taskbar_height, 32);
    }

    #[test]
    fn glass_mode_saves_dark_and_drops_light() {
        let base = Settings::default();
        let want = Settings {
            glass_mode: "dark".into(),
            ..base.clone()
        };
        let text = save("", &want, &base).unwrap();
        assert!(text.contains("mode = \"dark\""), "dark is written: {text}");
        assert_eq!(load(&text).glass_mode, "dark");
        let back = save(&text, &base, &want).unwrap();
        assert!(
            !back.contains("mode ="),
            "light, the default, drops the key: {back}"
        );
    }

    #[test]
    fn glass_material_and_frame_save_their_names_and_drop_the_defaults() {
        let base = Settings::default();
        let want = Settings {
            glass_material: "smoked".into(),
            glass_frame: "square".into(),
            ..base.clone()
        };
        let text = save("", &want, &base).unwrap();
        assert_eq!(text, "[glass]\nmaterial = \"smoked\"\nframe = \"square\"\n");
        let back = load(&text);
        assert_eq!(
            (back.glass_material.as_str(), back.glass_frame.as_str()),
            ("smoked", "square")
        );
        assert_eq!(
            save(&text, &base, &want).unwrap(),
            "",
            "frosted and rounded, the defaults, drop both keys"
        );
    }

    #[test]
    fn appearance_defaults_match_the_compositor() {
        let d = Settings::default();
        assert_eq!(
            (d.interface.as_str(), d.color_scheme.as_str()),
            ("classic", "")
        );
        let g = reprisekit::glass::Settings::default();
        assert_eq!(reprisekit::glass::swatch(&d.glass_color), Some(g.color));
        assert_eq!(d.glass_intensity, i64::from(g.intensity));
        assert_eq!(d.glass_orb_glow, i64::from(g.orb_glow));
        assert_eq!(
            (d.glass_transparency, d.glass_fast_drag, d.glass_blur_frames),
            (g.transparency, g.fast_drag, g.blur_frames)
        );
        assert_eq!(
            (d.glass_blur, d.glass_streaks),
            (i64::from(g.blur), g.streaks)
        );
        assert_eq!(reprisekit::glass::Mode::parse(&d.glass_mode), Some(g.mode));
        assert_eq!(
            reprisekit::glass::Material::parse(&d.glass_material),
            Some(g.material)
        );
        assert_eq!(
            reprisekit::glass::Frame::parse(&d.glass_frame),
            Some(g.frame)
        );
        assert_eq!(d.glass_custom, "");
        assert_eq!((d.wallpaper.as_str(), d.background.as_str()), ("", ""));
        assert_eq!(
            reprisekit::wallpaper::Position::parse(&d.wallpaper_position),
            Some(reprisekit::wallpaper::Position::default())
        );
        assert_eq!(
            d.system_icons,
            ["documents", "computer", "home", "recycle-bin"]
        );
    }

    #[test]
    fn a_glass_colour_no_swatch_names_is_the_custom_one() {
        assert_eq!(
            load("[glass]\ncolor = \"#102030\"\n").glass_custom,
            "#102030"
        );
        assert_eq!(load("[glass]\ncolor = \"ruby\"\n").glass_custom, "");
        // Sky's own colour spelled out is Sky, not a custom colour.
        assert_eq!(load("[glass]\ncolor = \"#70AFFC\"\n").glass_custom, "");
        assert_eq!(glass_swatch("#70AFFC"), Some("sky"));
        assert_eq!(glass_swatch("leaf"), Some("leaf"));
        assert_eq!(glass_swatch("#102030"), None);
    }

    #[test]
    fn system_icons_keep_their_order_and_the_bin() {
        let have: Vec<String> = vec!["computer".into(), "recycle-bin".into()];
        assert_eq!(
            system_icons_with(&have, "documents", true),
            ["computer", "recycle-bin", "documents"]
        );
        assert_eq!(system_icons_with(&have, "computer", false), ["recycle-bin"]);
        assert_eq!(system_icons_with(&have, "computer", true), have);
        // A list without the bin gets it back at the end; a name the
        // desktop does not know stays where it was.
        let odd: Vec<String> = vec!["home".into(), "nowhere".into()];
        assert_eq!(
            system_icons_with(&odd, "computer", true),
            ["home", "nowhere", "computer", "recycle-bin"]
        );
    }

    #[test]
    fn a_hand_ordered_icon_list_keeps_its_order() {
        let hand: Vec<String> = ["computer", "recycle-bin", "documents", "custom-x"]
            .map(String::from)
            .to_vec();
        let off = system_icons_with(&hand, "computer", false);
        assert_eq!(off, ["recycle-bin", "documents", "custom-x"]);
        assert_eq!(
            system_icons_with(&off, "computer", true),
            ["recycle-bin", "documents", "custom-x", "computer"]
        );
        let off = system_icons_with(&hand, "documents", false);
        assert_eq!(
            system_icons_with(&off, "documents", true),
            ["computer", "recycle-bin", "custom-x", "documents"]
        );
    }

    #[test]
    fn older_icon_names_load_as_the_current_ones() {
        let s = load("system_icons = [\"my-computer\", \"my-network-places\", \"recycle-bin\"]\n");
        assert_eq!(s.system_icons, ["computer", "recycle-bin"]);
    }

    #[test]
    fn the_start_menu_style_defaults_follow_the_interface() {
        assert_eq!(Settings::for_interface("glass").startmenu_style, "glass");
        assert_eq!(
            Settings::for_interface("classic").startmenu_style,
            "classic"
        );
        assert_eq!(Settings::for_interface("blue").startmenu_style, "classic");
    }

    #[test]
    fn the_lock_screen_wallpaper_defaults_on_under_blue_and_glass() {
        assert!(!Settings::for_interface("").lock_wallpaper);
        assert!(!Settings::for_interface("classic").lock_wallpaper);
        assert!(Settings::for_interface("blue").lock_wallpaper);
        assert!(Settings::for_interface("glass").lock_wallpaper);
    }

    #[test]
    fn a_new_interface_resets_the_scheme_and_the_unset_defaults() {
        let text = "[appearance]\ncolor_scheme = \"Brick\"\n[taskbar]\npinned = false\n";
        let loaded = load(text);
        let mut s = loaded.clone();
        s.background = "#102030".into();
        s.interface = "glass".into();
        switch_interface(&mut s, "classic", text, &loaded);
        assert_eq!(s.color_scheme, "", "the scheme goes back to Glass's own");
        assert_eq!(s.background, "#102030", "the desktop colour is its own key");
        assert_eq!(s.taskbar_buttons, "combine", "unset: Glass's default");
        assert!(s.taskbar_peek && s.window_animation && s.hover_fade && s.taskbar_slide);
        assert!(!s.taskbar_pinned, "the file sets it");
        s.interface = "classic".into();
        switch_interface(&mut s, "glass", text, &loaded);
        assert_eq!(s.taskbar_buttons, "when_full");
        assert!(!s.taskbar_peek && !s.window_animation && !s.hover_fade && !s.taskbar_slide);
    }

    #[test]
    fn picking_the_interface_back_keeps_the_colour_scheme() {
        let text = "[appearance]\ninterface = \"blue\"\ncolor_scheme = \"Olive Green\"\n\
                    [glass]\ncolor = \"ruby\"\n";
        let loaded = load(text);
        let mut want = loaded.clone();
        want.interface = "classic".into();
        switch_interface(&mut want, "blue", text, &loaded);
        assert_eq!(want.color_scheme, "", "Classic's own scheme");
        want.interface = "blue".into();
        switch_interface(&mut want, "classic", text, &loaded);
        assert_eq!(want.color_scheme, "Olive Green");
        assert_eq!(want.glass_color, "ruby");
        assert_eq!(want, loaded, "OK has nothing to write");
        assert_eq!(save(text, &want, &loaded).expect("parses"), text);
    }

    #[test]
    fn a_switched_interface_is_written_against_its_own_defaults() {
        let base = load("");
        let mut want = base.clone();
        want.interface = "glass".into();
        switch_interface(&mut want, "classic", "", &base);
        let out = save("", &want, &base).expect("parses");
        assert_eq!(
            out, "[appearance]\ninterface = \"glass\"\n",
            "no Glass default is pinned"
        );
        assert_eq!(load(&out), want);
        let base = want.clone();
        want.interface = "classic".into();
        switch_interface(&mut want, "glass", &out, &base);
        let out = save(&out, &want, &base).expect("parses");
        assert_eq!(
            out, "",
            "Classic is the default, so the key and its table go"
        );
    }

    #[test]
    fn switcher_defaults_follow_the_interface() {
        let g = Settings::for_interface("glass");
        assert_eq!(g.switcher_style, "thumbnails");
        assert!(g.switcher_desktop && g.switcher_peek && g.switcher_preview && g.switcher_flip);
        let c = Settings::for_interface("classic");
        assert_eq!(c.switcher_style, "icons");
        assert!(!c.switcher_desktop && !c.switcher_peek && c.switcher_preview && !c.switcher_flip);
    }

    #[test]
    fn window_shake_follows_the_interface_and_auto_reads_as_its_default() {
        assert!(Settings::for_interface("glass").window_shake);
        assert!(!Settings::for_interface("classic").window_shake);
        assert!(!Settings::for_interface("blue").window_shake);
        let glass = "[appearance]\ninterface = \"glass\"\n";
        assert!(load(&format!("{glass}[windows]\nshake = \"auto\"\n")).window_shake);
        assert!(!load(&format!("{glass}[windows]\nshake = false\n")).window_shake);
        let base = load(glass);
        let off = Settings {
            window_shake: false,
            ..base.clone()
        };
        let text = save(glass, &off, &base).unwrap();
        assert!(text.contains("shake = false"), "{text}");
        assert!(!save(&text, &base, &off).unwrap().contains("shake"));
    }

    #[test]
    fn save_round_trips_every_field() {
        let s = Settings {
            terminal: "kitty".into(),
            script_terminal: "close".into(),
            task_manager: "htop".into(),
            startmenu_banner: "Reprise".into(),
            startmenu_style: "classic".into(),
            desktop_icons: false,
            desktop_extensions: "hide_launchers".into(),
            start_icon: false,
            adaptive_sync: true,
            switcher_preview: false,
            switcher_desktop: true,
            switcher_peek: true,
            switcher_flip: true,
            preview_taskbar: false,
            previews_fps: 30,
            switcher_style: "thumbnails".into(),
            switcher_size: "medium".into(),
            hot_corner: "top-right".into(),
            screenshot_save: false,
            clipboard_history: false,
            interface: "blue".into(),
            color_scheme: "Olive Green".into(),
            glass_color: "#102030".into(),
            glass_mode: "dark".into(),
            glass_material: "smoked".into(),
            glass_frame: "square".into(),
            glass_custom: "#102030".into(),
            glass_intensity: 70,
            glass_transparency: false,
            glass_blur: 4,
            glass_streaks: false,
            glass_fast_drag: false,
            glass_blur_frames: false,
            glass_orb_glow: 35,
            wallpaper: "/usr/share/backgrounds/a.png".into(),
            wallpaper_position: "tile".into(),
            wallpaper_water: "rain".into(),
            drive_insert: "open".into(),
            slideshow: "~/Pictures/Slides".into(),
            slideshow_interval: 5,
            slideshow_shuffle: true,
            saver: "photos".into(),
            saver_speed: 8,
            saver_stars: 400,
            saver_shapes: 3,
            saver_cycle: false,
            saver_folder: "/srv/photos".into(),
            saver_photo_seconds: 20,
            background: "#336699".into(),
            system_icons: vec!["computer".into(), "recycle-bin".into()],
            taskbar_buttons: "never".into(),
            taskbar_pinned: true,
            taskbar_small_icons: true,
            taskbar_show_desktop: true,
            taskbar_peek: true,
            taskbar_tray_overflow: true,
            taskbar_position: "top".into(),
            taskbar_auto_hide: true,
            taskbar_reveal_on_edge: false,
            taskbar_reveal_on_keys: false,
            taskbar_reveal_on_attention: false,
            recent_files: false,
            location: false,
            capture_border: false,
            debug_overlay: true,
            debug_overlay_in_captures: true,
            launcher_files: false,
            thumbnail_cache: false,
            thumbnail_max_mb: 64,
            desktops: 1,
            desktop_animation: "slide-vertical".into(),
            desktop_animation_ms: 350,
            pager_mark: "tile".into(),
            remember_windows: false,
            window_tabs: false,
            window_shake: true,
            session_restore: true,
            session_end_processes: false,
            font_family: "Verdana".into(),
            font_size_px: 13,
            font_antialias: true,
            font_shell_dlg: "/opt/fonts/dlg.ttf".into(),
            icon_theme: "hicolor".into(),
            gtk_theme: "Arc-Dark".into(),
            qt_platform_theme: "qt6ct".into(),
            font_smoothing: "subpixel".into(),
            gtk4_colors: true,
            cursor_theme: "Adwaita".into(),
            cursor_size: 48,
            frame_width: 5,
            caption_height: 20,
            taskbar_height: 32,
            menu_shadow: 0,
            menu_hover_ms: 300,
            menu_fade_ms: 0,
            window_animation: true,
            window_open_ms: 300,
            window_close_ms: 180,
            hover_fade: true,
            hover_in_ms: 90,
            hover_out_ms: 400,
            taskbar_slide: true,
            double_click_ms: 400,
            kb_layout: "hu".into(),
            kb_variant: "nodeadkeys".into(),
            kb_model: "pc105".into(),
            kb_options: "ctrl:nocaps".into(),
            repeat_rate: 30,
            repeat_delay: 500,
            standby: "loginctl suspend".into(),
            poweroff: "loginctl poweroff".into(),
            reboot: "loginctl reboot".into(),
            hibernate: "loginctl hibernate".into(),
            lock_on_sleep: false,
            lock_wallpaper: true,
            lock_after: 300,
            screen_off_after: 0,
            blank_after: 120,
            lock_fade: 4,
            mouse_speed: -5,
            mouse_accel: false,
            mouse_natural: true,
            touchpad_speed: 10,
            touchpad_accel: false,
            touchpad_natural: true,
            tap_to_click: false,
            two_finger_scroll: false,
            two_finger_tap_right_click: false,
            disable_while_typing: false,
            snap_enabled: false,
            snap_picker: false,
            snap_bar: false,
            snap_layouts: "1|1 2-1:2".into(),
            nl_enabled: true,
            nl_temperature: 3400,
            nl_schedule: "fixed".into(),
            nl_from: "20:30".into(),
            nl_to: "06:00".into(),
            sticky_keys: true,
            sticky_lock: false,
            sticky_two_keys_off: false,
            slow_keys: true,
            slow_keys_delay: 3,
            key_shortcuts: false,
            color_filter: "protanopia".into(),
            color_filter_intensity: 60,
            magnifier_follow: "pointer".into(),
            mouse_keys: true,
            mouse_keys_num_lock: false,
            mouse_keys_speed: 900,
            mouse_keys_acceleration: 25,
            text_size: 150,
            shortcuts: vec![Shortcut::new("Super+t", "spawn", "foot")],
            // The portal adds these; the sheet only edits what is there.
            programs: Vec::new(),
            denied: Vec::new(),
            startmenu: vec![crate::startmenu::Entry::item(
                "&Web",
                "web-browser",
                crate::startmenu::Target::Command("firefox".into()),
            )],
            tablet_pads: vec![crate::tabletpage::PadEntry {
                name: "Wacom*".into(),
                buttons: [(1, "zoom-in".to_string())].into(),
                ring_up: "ctrl+plus".into(),
                ..Default::default()
            }],
        };
        let out = saved(SAMPLE, &s);
        assert_eq!(load(&out), s);
    }

    #[test]
    fn snap_defaults_match_the_compositor() {
        let d = Settings::default();
        assert!(d.snap_enabled, "drag snapping is on");
        assert!(d.snap_picker);
        assert!(d.snap_bar);
        assert_eq!(d.snap_layouts, "", "the built-in six");
    }

    #[test]
    fn snap_layouts_write_as_an_array_and_read_back() {
        let want = Settings {
            snap_picker: false,
            snap_layouts: "1|1,2|1  1:2|1:2".into(),
            ..Settings::default()
        };
        let out = saved("", &want);
        assert!(
            out.contains(r#"layouts = ["1|1", "2|1", "1:2|1:2"]"#),
            "{out}"
        );
        assert!(out.contains("picker = false"), "{out}");
        let got = load(&out);
        assert!(!got.snap_picker);
        assert_eq!(got.snap_layouts, "1|1 2|1 1:2|1:2");
        let cleared = Settings {
            snap_layouts: String::new(),
            ..got.clone()
        };
        let out = save(&out, &cleared, &got).unwrap();
        assert!(!out.contains("layouts"), "empty is the default: {out}");
    }

    #[test]
    fn validate_refuses_a_snap_layout_the_compositor_would() {
        let base = Settings::default();
        let with = |l: &str| Settings {
            snap_layouts: l.into(),
            ..Settings::default()
        };
        assert_eq!(validate(&with("1|1 2-1:2"), &base), Ok(()));
        assert_eq!(
            validate(&with("1|1 1|0"), &base),
            Err(r#"Snap layouts: "1|0": "0" is not a weight > 0."#.into())
        );
        assert_eq!(
            validate(&with("1|1-1"), &base),
            Err(r#"Snap layouts: "1|1-1" mixes | and -."#.into())
        );
        assert_eq!(
            validate(&with("1 1 1 1 1 1 1"), &base),
            Err("Snap layouts: at most 6 layouts, got 7.".into())
        );
    }

    #[test]
    fn shortcuts_default_to_the_compositors_list() {
        let s = load("");
        assert_eq!(s.shortcuts.len(), 40);
        assert_eq!(
            s.shortcuts[0],
            Shortcut::new("Super+Return", "terminal", "")
        );
        assert_eq!(
            s.shortcuts[10],
            Shortcut::new("Super+Shift+b", "toggleframe", "")
        );
        assert_eq!(
            s.shortcuts[11],
            Shortcut::new("Super+space", "launcher", "")
        );
        assert_eq!(s.shortcuts[12], Shortcut::new("Super+c", "center", ""));
        assert_eq!(
            s.shortcuts[16],
            Shortcut::new("Super+Down", "snap-down", "")
        );
        assert_eq!(
            s.shortcuts[20],
            Shortcut::new("Super+Ctrl+Shift+Right", "desktop-carry-next", "")
        );
        assert_eq!(
            s.shortcuts[22],
            Shortcut::new("Alt+Sys_Req", "screenshot-window", "")
        );
        assert_eq!(validate(&s, &Settings::default()), Ok(()));
    }

    #[test]
    fn a_keybinds_table_in_the_file_layers_over_the_defaults() {
        let text = "[[keybinds]]\nkeys = \"Super+t\"\naction = \"spawn\"\ncommand = \"foot\"\n\n\
                    [[keybinds]]\nkeys = \"Super+q\"\naction = \"none\"\n\n\
                    [[keybinds]]\nkeys = \"Super+F9\"\naction = \"lock\"\n";
        let s = load(text);
        let mut want = default_shortcuts();
        let t = want.iter().position(|b| b.keys == "Super+t").unwrap();
        want[t] = Shortcut::new("Super+t", "spawn", "foot");
        want.retain(|b| b.keys != "Super+q");
        want.push(Shortcut::new("Super+F9", "lock", ""));
        assert_eq!(s.shortcuts, want);
    }

    #[test]
    fn shortcuts_save_what_differs_from_the_defaults_and_go_when_back_to_default() {
        let mut s = load(SAMPLE);
        s.shortcuts.push(Shortcut::new("Super+F9", "spawn", "foot"));
        let out = saved(SAMPLE, &s);
        assert!(
            out.starts_with(SAMPLE),
            "the rest of the file is untouched: {out}"
        );
        assert_eq!(
            out[SAMPLE.len()..].trim(),
            "[[keybinds]]\nkeys = \"Super+F9\"\naction = \"spawn\"\ncommand = \"foot\"",
            "no default is copied into the file"
        );
        assert_eq!(load(&out), s);

        let mut removed = s.clone();
        removed.shortcuts.retain(|b| b.keys != "Super+q");
        removed.shortcuts[2] = Shortcut::new("Alt+Tab", "flip", "");
        let out = save(&out, &removed, &s).unwrap();
        assert!(
            out.contains("[[keybinds]]\nkeys = \"Super+q\"\naction = \"none\"\n"),
            "a removed default is written as none: {out}"
        );
        assert!(out.contains("[[keybinds]]\nkeys = \"Alt+Tab\"\naction = \"flip\"\n"));
        assert_eq!(load(&out), removed);

        let out = save(&out, &load(SAMPLE), &removed).unwrap();
        assert!(!out.contains("keybinds"), "{out}");
    }

    #[test]
    fn a_files_none_survives_a_rewrite_of_its_keybinds() {
        let text = "[[keybinds]]\nkeys = \"Super+l\"\naction = \"none\"\n";
        let base = load(text);
        let mut want = base.clone();
        want.shortcuts.push(Shortcut::new("Super+F9", "lock", ""));
        let out = save(text, &want, &base).unwrap();
        assert!(
            out.contains("[[keybinds]]\nkeys = \"Super+l\"\naction = \"none\"\n"),
            "{out}"
        );
        assert_eq!(load(&out), want);
    }

    #[test]
    fn validate_refuses_a_shortcut_it_cannot_bind() {
        let mut s = Settings::default();
        s.shortcuts.push(Shortcut::new("Hyper+x", "close", ""));
        assert!(validate(&s, &Settings::default()).is_err());
        s.shortcuts.pop();
        s.shortcuts.push(Shortcut::new("Super+", "close", ""));
        assert!(validate(&s, &Settings::default()).is_err());
        s.shortcuts.pop();
        s.shortcuts.push(Shortcut::new("Super+t", "spawn", ""));
        assert!(validate(&s, &Settings::default()).is_err());
        s.shortcuts.pop();
        s.shortcuts.push(Shortcut::new("Super+t", "nosuch", ""));
        assert!(validate(&s, &Settings::default()).is_err());
    }

    #[test]
    fn desktop_and_preview_defaults_match_the_compositor() {
        let d = Settings::default();
        assert_eq!(d.desktops, 4);
        assert_eq!(d.desktop_animation, "none");
        assert_eq!(d.desktop_animation_ms, 200);
        assert_eq!(d.previews_fps, 10);
        assert!(d.screenshot_save && d.clipboard_history && d.remember_windows);
        assert!(d.window_tabs);
        assert!(!d.session_restore);
        assert!(d.session_end_processes);
        assert_eq!(d.hibernate, "systemctl hibernate");
        assert!(d.lock_on_sleep);
    }

    #[test]
    fn workspaces_and_previews_round_trip_and_drop_the_defaults() {
        let text = "[workspaces]\ncount = 6\nanimation = \"wipe\"\n[previews]\nfps = 24\n";
        let s = load(text);
        assert_eq!(s.desktops, 6);
        assert_eq!(s.desktop_animation, "wipe");
        assert_eq!(s.desktop_animation_ms, 200, "absent: the default");
        assert_eq!(s.previews_fps, 24);

        let mut want = s.clone();
        want.desktops = 1;
        want.desktop_animation_ms = 500;
        want.previews_fps = 10;
        let out = saved(text, &want);
        assert_eq!(
            out, "[workspaces]\ncount = 1\nanimation = \"wipe\"\nanimation_ms = 500\n",
            "the emptied [previews] goes"
        );
        assert_eq!(load(&out), want);

        let base = want.clone();
        want.desktops = 4;
        want.desktop_animation = "none".into();
        want.desktop_animation_ms = 200;
        let out = save(&out, &want, &base).expect("parses");
        assert_eq!(out, "", "the emptied [workspaces] goes too");
    }

    #[test]
    fn the_desktop_extensions_round_trip_and_the_default_leaves_the_file() {
        assert_eq!(load("").desktop_extensions, "hide");
        let base = load("");
        let mut show = base.clone();
        show.desktop_extensions = "show".into();
        let text = saved("", &show);
        assert_eq!(text, "desktop_extensions = \"show\"\n");
        assert_eq!(load(&text).desktop_extensions, "show");
        let mut launchers = load(&text);
        launchers.desktop_extensions = "hide_launchers".into();
        assert_eq!(
            saved(&text, &launchers),
            "desktop_extensions = \"hide_launchers\"\n"
        );
        let mut hide = load(&text);
        hide.desktop_extensions = "hide".into();
        assert_eq!(saved(&text, &hide), "", "the default removes the key");
    }

    #[test]
    fn the_script_end_round_trips_and_the_default_leaves_the_file() {
        assert_eq!(load("").script_terminal, "stay");
        let foot = "terminal = \"foot\"\n";
        let mut close = load(foot);
        close.script_terminal = "close".into();
        let text = saved(foot, &close);
        assert_eq!(text, "terminal = \"foot\"\nscript_terminal = \"close\"\n");
        assert_eq!(load(&text).script_terminal, "close");
        let mut stay = load(&text);
        stay.script_terminal = "stay".into();
        assert_eq!(
            saved(&text, &stay),
            "terminal = \"foot\"\n",
            "the default removes the key"
        );
    }

    #[test]
    fn the_drive_insert_mode_round_trips_and_the_default_leaves_the_file() {
        assert_eq!(load("").drive_insert, "ask");
        let text = "[drives]\non_insert = \"nothing\"\n";
        let s = load(text);
        assert_eq!(s.drive_insert, "nothing");
        assert_eq!(saved(text, &s), text);
        let mut open = s.clone();
        open.drive_insert = "open".into();
        assert_eq!(saved(text, &open), "[drives]\non_insert = \"open\"\n");
        let mut ask = s;
        ask.drive_insert = "ask".into();
        assert_eq!(
            saved(text, &ask),
            "",
            "the default removes the key and its table"
        );
    }

    #[test]
    fn the_pager_mark_round_trips() {
        assert_eq!(load("").pager_mark, "light");
        let text = "[workspaces]\npager = \"tile\"\n";
        let s = load(text);
        assert_eq!(s.pager_mark, "tile");
        assert_eq!(saved(text, &s), text);
        let mut light = s.clone();
        light.pager_mark = "light".into();
        assert_eq!(saved(text, &light), "", "the default leaves the file");
        let odd = "[workspaces]\npager = \"box\"\n";
        let s = load(odd);
        assert_eq!(s.pager_mark, "box");
        assert_eq!(saved(odd, &s), odd, "an unknown value survives");
    }

    #[test]
    fn taskbar_defaults_follow_the_files_interface() {
        for text in ["", "[appearance]\ninterface = \"blue\"\n"] {
            let s = load(text);
            assert_eq!(s.taskbar_buttons, "when_full", "{text:?}");
            assert!(
                !s.taskbar_pinned
                    && !s.taskbar_small_icons
                    && !s.taskbar_show_desktop
                    && !s.taskbar_peek,
                "{text:?} turns every extra off but the tray arrow"
            );
            assert!(s.taskbar_tray_overflow, "{text:?}");
        }
        let g = load("[appearance]\ninterface = \"glass\"\n");
        assert_eq!(g.taskbar_buttons, "combine");
        assert!(g.taskbar_pinned && g.taskbar_show_desktop && g.taskbar_peek);
        assert!(g.taskbar_tray_overflow && !g.taskbar_small_icons);
        let set = load(
            "[appearance]\ninterface = \"glass\"\n[taskbar]\nbuttons = \"never\"\npeek = false\n",
        );
        assert_eq!(set.taskbar_buttons, "never");
        assert!(!set.taskbar_peek && set.taskbar_pinned);
    }

    #[test]
    fn taskbar_keys_are_written_against_the_interfaces_defaults() {
        let glass = "[appearance]\ninterface = \"glass\"\n";
        let mut want = load(glass);
        want.taskbar_buttons = "when_full".into();
        want.taskbar_small_icons = true;
        let out = saved(glass, &want);
        assert_eq!(
            out,
            "[appearance]\ninterface = \"glass\"\n\n[taskbar]\nbuttons = \"when_full\"\nsmall_icons = true\n"
        );
        assert_eq!(load(&out), want);
        let base = want.clone();
        want.taskbar_buttons = "combine".into();
        let out = save(&out, &want, &base).expect("parses");
        assert!(!out.contains("buttons"), "back to Glass's default: {out}");

        // The same value under Classic is not its default, so it is written.
        let mut classic = load("");
        classic.taskbar_buttons = "combine".into();
        classic.taskbar_peek = true;
        let out = saved("", &classic);
        assert!(out.contains("buttons = \"combine\""), "{out}");
        assert!(out.contains("peek = true"), "{out}");
    }

    /// Keys are written against the defaults of the interface in the file at Apply, which another writer may have switched.
    #[test]
    fn taskbar_keys_are_written_against_the_interface_on_disk_at_apply() {
        let glass = "[appearance]\ninterface = \"glass\"\n[taskbar]\nbuttons = \"never\"\n";
        let base = load(glass);
        let mut want = base.clone();
        want.taskbar_buttons = "combine".into();
        // Switched to Classic meanwhile: combine is not Classic's default,
        // so it is written rather than dropped.
        let classic = "[appearance]\ninterface = \"classic\"\n[taskbar]\nbuttons = \"never\"\n";
        let out = save(classic, &want, &base).expect("parses");
        assert!(out.contains("buttons = \"combine\""), "{out}");
        assert_eq!(load(&out).taskbar_buttons, "combine");
        // Still Glass: combine is its default, so the key goes.
        let out = save(glass, &want, &base).expect("parses");
        assert!(!out.contains("buttons"), "{out}");
    }

    #[test]
    fn the_taskbar_position_is_written_only_off_the_bottom() {
        for i in ["classic", "blue", "glass"] {
            for edge in ["top", "left", "right"] {
                let text = format!("[appearance]\ninterface = \"{i}\"\n");
                let s = load(&text);
                assert_eq!(s.taskbar_position, "bottom", "{i}");
                let mut moved = s.clone();
                moved.taskbar_position = edge.into();
                let out = saved(&text, &moved);
                assert_eq!(
                    out,
                    format!("{text}\n[taskbar]\nposition = \"{edge}\"\n"),
                    "{i} {edge}"
                );
                assert_eq!(load(&out), moved);
                let back = save(&out, &s, &moved).expect("parses");
                assert!(!back.contains("position"), "back at the bottom: {back}");
                assert_eq!(load(&back), s);
            }
        }
    }

    #[test]
    fn the_taskbar_positions_are_the_edge_names() {
        use reprisekit::taskbar::Edge;
        let values: Vec<&str> = TASKBAR_POSITIONS.iter().map(|c| c.0).collect();
        let names: Vec<&str> = Edge::ALL.iter().map(|e| e.name()).collect();
        assert_eq!(values, names);
        assert_eq!(Settings::default().taskbar_position, Edge::default().name());
    }

    #[test]
    fn the_auto_hide_keys_are_written_only_off_their_defaults() {
        for i in ["classic", "blue", "glass"] {
            let text = format!("[appearance]\ninterface = \"{i}\"\n");
            let s = load(&text);
            assert!(!s.taskbar_auto_hide, "{i}");
            assert!(
                s.taskbar_reveal_on_edge
                    && s.taskbar_reveal_on_keys
                    && s.taskbar_reveal_on_attention,
                "{i}"
            );
            let mut want = s.clone();
            want.taskbar_auto_hide = true;
            want.taskbar_reveal_on_keys = false;
            let out = saved(&text, &want);
            assert_eq!(
                out,
                format!("{text}\n[taskbar]\nauto_hide = true\nreveal_on_keys = false\n"),
                "{i}"
            );
            assert_eq!(load(&out), want);
            let back = save(&out, &s, &want).expect("parses");
            assert!(
                !back.contains("auto_hide") && !back.contains("reveal_on"),
                "back at the defaults: {back}"
            );
        }
    }

    #[test]
    fn session_and_power_flags_write_their_own_tables() {
        let mut s = load("");
        s.session_restore = true;
        s.lock_on_sleep = false;
        s.hibernate = "zzz".into();
        s.clipboard_history = false;
        let out = saved("", &s);
        assert!(out.contains("[session]\nrestore = true\n"), "{out}");
        assert!(out.contains("[clipboard]\nhistory = false\n"), "{out}");
        assert!(out.contains("hibernate = \"zzz\"\n"), "{out}");
        assert!(out.contains("lock_on_sleep = false\n"), "{out}");
        assert_eq!(load(&out), s);
    }

    #[test]
    fn pointer_speed_is_tenths_of_libinputs_float() {
        let text = "[mouse]\nspeed = -0.5\n[touchpad]\nspeed = 1\n";
        let s = load(text);
        assert_eq!(s.mouse_speed, -5);
        assert_eq!(s.touchpad_speed, 10);
        let mut s = load("");
        s.mouse_speed = 3;
        let out = saved("", &s);
        assert_eq!(out, "[mouse]\nspeed = 0.3\n");
        s.mouse_speed = 0;
        assert!(
            !saved(&out, &s).contains("speed"),
            "the default leaves the file"
        );
    }

    const PROGRAMS: &str = "[[global_shortcuts]]\napp = \"t\"\nid = \"talk\"\ndescription = \"Hold to talk\"\nkeys = \"Pause\"\n\
        [[global_shortcuts]]\napp = \"t\"\nid = \"mute\"\nkeys = \"\"\n\
        [portal]\nshortcuts_denied = [\"n\"]\n";

    fn p(app: &str, id: &str, desc: &str, keys: &str) -> ProgramShortcut {
        ProgramShortcut {
            app: app.into(),
            id: id.into(),
            description: desc.into(),
            keys: keys.into(),
        }
    }

    #[test]
    fn program_shortcuts_and_denials_load() {
        let s = load(PROGRAMS);
        assert_eq!(
            s.programs,
            [
                p("t", "talk", "Hold to talk", "Pause"),
                p("t", "mute", "", "")
            ]
        );
        assert_eq!(s.denied, ["n"]);
    }

    #[test]
    fn rows_run_binds_then_programs_then_denials() {
        let s = load(PROGRAMS);
        let n = s.shortcuts.len();
        assert_eq!(row(&s, 0), Some(Row::Bind(0)));
        assert_eq!(row(&s, n), Some(Row::Program(0)));
        assert_eq!(row(&s, n + 2), Some(Row::Denied(0)));
        assert_eq!(row(&s, n + 3), None);
    }

    #[test]
    fn forgetting_a_program_drops_all_its_shortcuts_or_its_denial() {
        let mut s = load(PROGRAMS);
        forget(&mut s, Row::Program(1));
        assert!(s.programs.is_empty(), "both of t's go");
        forget(&mut s, Row::Denied(0));
        assert!(s.denied.is_empty());
        let out = saved(PROGRAMS, &s);
        assert!(!out.contains("global_shortcuts"), "{out}");
        assert!(!out.contains("shortcuts_denied"), "{out}");
        assert!(!out.contains("[portal]"), "no bare header is left: {out}");
    }

    #[test]
    fn a_changed_program_key_saves_in_place() {
        let mut s = load(PROGRAMS);
        s.programs[0].keys = "F9".into();
        let out = saved(PROGRAMS, &s);
        assert_eq!(load(&out).programs[0].keys, "F9");
        assert_eq!(load(&out).programs[1], p("t", "mute", "", ""));
    }

    #[test]
    fn a_program_bound_while_the_sheet_was_open_survives_a_save() {
        let base = load(PROGRAMS);
        let mut want = base.clone();
        want.programs[0].keys = "F9".into();
        let now =
            format!("{PROGRAMS}[[global_shortcuts]]\napp = \"v\"\nid = \"ptt\"\nkeys = \"F10\"\n");
        let out = save(&now, &want, &base).unwrap();
        let after = load(&out);
        assert_eq!(after.programs[0].keys, "F9");
        assert!(
            after
                .programs
                .iter()
                .any(|e| e.app == "v" && e.keys == "F10"),
            "{out}"
        );
    }

    #[test]
    fn a_program_key_may_be_empty_but_not_wrong() {
        let base = load(PROGRAMS);
        let mut s = base.clone();
        assert!(validate(&s, &base).is_ok());
        s.programs[0].keys = "Ctrl+Nope".into();
        assert!(validate(&s, &base).is_err());
        s.programs[0].keys = "Ctrl+Shift_L".into();
        assert_eq!(
            validate(&s, &base),
            Err("Hold to talk: A program shortcut ends in a key that is not a modifier.".into())
        );
        s.programs[0].keys = "Super+Shift+Escape".into();
        assert_eq!(
            validate(&s, &base),
            Err("Hold to talk: Super+Shift+Escape is kept by the desktop.".into())
        );
    }

    #[test]
    fn a_bad_program_key_from_the_file_does_not_block_apply() {
        let base = load(&PROGRAMS.replace("keys = \"Pause\"", "keys = \"Ctrl+Nope\""));
        let mut s = base.clone();
        s.desktops = 2;
        assert_eq!(validate(&s, &base), Ok(()), "the entry is as loaded");
        s.programs[0].keys = "Ctrl+Shift_L".into();
        assert!(validate(&s, &base).is_err(), "an edited entry is checked");
        s.programs[0].keys = "F9".into();
        assert_eq!(validate(&s, &base), Ok(()));
    }
}
