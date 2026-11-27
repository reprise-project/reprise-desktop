use repriseconf::recording::Codec;
use reprisekit::tr;

/// A row's text, looked up when the page is built.
pub type Label = fn() -> String;

/// A combo's items: the value the config takes, then the text the combo
/// shows for it.
pub type Choices = [(&'static str, Label)];

/// The taskbar button modes. Mirrors `TaskbarButtons::parse` in
/// `repriseconf::config`.
pub const TASKBAR_BUTTONS: &Choices = &[
    ("combine", || tr!("choices-taskbar-buttons-combine")),
    ("when_full", || tr!("choices-taskbar-buttons-when-full")),
    ("never", || tr!("choices-taskbar-buttons-never")),
    ("group", || tr!("choices-taskbar-buttons-group")),
];

/// What a title bar double click does. Mirrors `TitleDoubleClick::parse` in `repriseconf::config`.
pub const TITLE_DOUBLE_CLICKS: &Choices = &[
    ("maximize", || tr!("choices-title-double-clicks-maximize")),
    ("shade", || tr!("choices-title-double-clicks-shade")),
    ("minimize", || tr!("choices-title-double-clicks-minimize")),
    ("none", || tr!("choices-title-double-clicks-none")),
];

/// The Start menu styles. Mirrors `StartMenuStyle::parse` in `repriseconf::config`.
pub const STARTMENU_STYLES: &Choices = &[
    ("glass", || tr!("choices-startmenu-styles-glass")),
    ("blue", || tr!("choices-startmenu-styles-blue")),
    ("classic", || tr!("choices-startmenu-styles-classic")),
];

/// What the Glass Start menu's power button does. Mirrors `PowerButton::from_name` in
/// `crates/reprisekit/src/startpanel/mod.rs`.
pub const STARTMENU_POWERS: &Choices = &[
    ("shut_down", || tr!("choices-startmenu-powers-shut-down")),
    ("log_off", || tr!("choices-startmenu-powers-log-off")),
    ("lock", || tr!("choices-startmenu-powers-lock")),
    ("restart", || tr!("choices-startmenu-powers-restart")),
];

/// The edges the taskbar can sit on. Mirrors `Edge::name` in
/// `crates/reprisekit/src/taskbar.rs`.
pub const TASKBAR_POSITIONS: &Choices = &[
    ("bottom", || tr!("choices-taskbar-positions-bottom")),
    ("top", || tr!("choices-taskbar-positions-top")),
    ("left", || tr!("choices-taskbar-positions-left")),
    ("right", || tr!("choices-taskbar-positions-right")),
];

/// What pressing a clickpad clicks. Mirrors `ClickMethod::parse` in `repriseconf::config`.
pub const CLICK_METHODS: &Choices = &[
    ("default", || tr!("choices-click-methods-default")),
    ("areas", || tr!("choices-click-methods-areas")),
    ("clickfinger", || tr!("choices-click-methods-clickfinger")),
];

/// `[gestures] fingers`' rows.
pub const GESTURE_FINGERS: &[(i64, Label)] = &[
    (3, || tr!("choices-gesture-fingers-3")),
    (4, || tr!("choices-gesture-fingers-4")),
];

/// A drop-down that edits part of `[keyboard] options`: each row an xkb option id ("" for none),
/// and an id prefix whose other ids it shows as themselves and replaces when a row is picked.
pub struct XkbPick {
    pub rows: &'static Choices,
    pub family: Option<&'static str>,
}

impl XkbPick {
    fn owns(&self, id: &str) -> bool {
        !id.is_empty()
            && (self.rows.iter().any(|(v, _)| *v == id)
                || self.family.is_some_and(|f| id.starts_with(f)))
    }

    /// The id of this drop-down `options` holds, "" for none.
    pub fn get<'a>(&self, options: &'a str) -> &'a str {
        (options.split(',').map(str::trim))
            .find(|id| self.owns(id))
            .unwrap_or("")
    }

    /// `options` holding `id` in place of this drop-down's ids; every other id stays as it was.
    pub fn set(&self, options: &str, id: &str) -> String {
        if self.get(options) == id {
            return options.to_string();
        }
        let mut ids: Vec<&str> = (options.split(',').map(str::trim))
            .filter(|o| !o.is_empty() && !self.owns(o))
            .collect();
        if !id.is_empty() {
            ids.push(id);
        }
        ids.join(",")
    }
}

/// Settings > Keyboard's Compose key.
pub const COMPOSE_KEY: XkbPick = XkbPick {
    rows: &[
        ("", || tr!("choices-compose-key-none")),
        ("compose:ralt", || tr!("choices-compose-key-compose-ralt")),
        ("compose:rwin", || tr!("choices-compose-key-compose-rwin")),
        ("compose:menu", || tr!("choices-compose-key-compose-menu")),
        ("compose:rctrl", || tr!("choices-compose-key-compose-rctrl")),
        ("compose:caps", || tr!("choices-compose-key-compose-caps")),
        ("compose:lwin", || tr!("choices-compose-key-compose-lwin")),
        ("compose:prsc", || tr!("choices-compose-key-compose-prsc")),
        ("compose:sclk", || tr!("choices-compose-key-compose-sclk")),
        ("compose:paus", || tr!("choices-compose-key-compose-paus")),
        ("compose:ins", || tr!("choices-compose-key-compose-ins")),
    ],
    family: Some("compose:"),
};

/// Settings > Keyboard's Caps Lock key.
pub const CAPS_LOCK: XkbPick = XkbPick {
    rows: &[
        ("", || tr!("choices-caps-lock-none")),
        ("ctrl:nocaps", || tr!("choices-caps-lock-ctrl-nocaps")),
        ("caps:escape", || tr!("choices-caps-lock-caps-escape")),
        ("caps:backspace", || tr!("choices-caps-lock-caps-backspace")),
        ("caps:none", || tr!("choices-caps-lock-caps-none")),
    ],
    family: None,
};

/// What flashes on the bell. Mirrors `VisualBell::parse` in `repriseconf::config`.
pub const VISUAL_BELLS: &Choices = &[
    ("none", || tr!("choices-visual-bells-none")),
    ("window", || tr!("choices-visual-bells-window")),
    ("screen", || tr!("choices-visual-bells-screen")),
];

pub const SWITCHER_STYLES: &Choices = &[
    ("icons", || tr!("choices-switcher-styles-icons")),
    ("thumbnails", || tr!("choices-switcher-styles-thumbnails")),
];

pub const SWITCHER_SIZES: &Choices = &[
    ("small", || tr!("choices-switcher-sizes-small")),
    ("medium", || tr!("choices-switcher-sizes-medium")),
    ("large", || tr!("choices-switcher-sizes-large")),
    ("huge", || tr!("choices-switcher-sizes-huge")),
];

/// The overview's hot corner combo. Mirrors `HotCorner::name` in
/// `repriseconf::config`.
pub const HOT_CORNERS: &Choices = &[
    ("none", || tr!("choices-hot-corners-none")),
    ("top-left", || tr!("choices-hot-corners-top-left")),
    ("top-right", || tr!("choices-hot-corners-top-right")),
    ("bottom-left", || tr!("choices-hot-corners-bottom-left")),
    ("bottom-right", || tr!("choices-hot-corners-bottom-right")),
];

/// The desktop switch animations. Mirrors `SwitchStyle::name` in
/// `repriseconf::config`.
pub const SWITCH_ANIMATIONS: &Choices = &[
    ("none", || tr!("choices-switch-animations-none")),
    ("slide", || tr!("choices-switch-animations-slide")),
    ("slide-vertical", || {
        tr!("choices-switch-animations-slide-vertical")
    }),
    ("fade", || tr!("choices-switch-animations-fade")),
    ("zoom", || tr!("choices-switch-animations-zoom")),
    ("wipe", || tr!("choices-switch-animations-wipe")),
];

/// What the lock shows of the media playing. Mirrors `LockMedia::name` in `repriseconf::config`.
pub const LOCK_MEDIA: &Choices = &[
    ("title", || tr!("choices-lock-media-title")),
    ("controls", || tr!("choices-lock-media-controls")),
    ("off", || tr!("choices-lock-media-off")),
];

/// How menus open. Mirrors `MenuAnimation::name` in `repriseconf::config`.
pub const MENU_ANIMATIONS: &Choices = &[
    ("fade", || tr!("choices-menu-animations-fade")),
    ("slide", || tr!("choices-menu-animations-slide")),
    ("none", || tr!("choices-menu-animations-none")),
];

/// How the Glass taskbar marks the current desktop. Mirrors
/// `PagerMark::name` in `crates/reprisekit/src/taskbar.rs`.
pub const PAGER_MARKS: &Choices = &[
    ("light", || tr!("choices-pager-marks-light")),
    ("tile", || tr!("choices-pager-marks-tile")),
];

/// The text sizes `[accessibility] text_size` takes. Mirrors
/// `TEXT_SIZE` in `repriseconf::config`.
pub const TEXT_SIZES: &[(i64, Label)] = &[
    (100, || tr!("choices-text-sizes-100")),
    (125, || tr!("choices-text-sizes-125")),
    (150, || tr!("choices-text-sizes-150")),
    (175, || tr!("choices-text-sizes-175")),
    (200, || tr!("choices-text-sizes-200")),
];

pub const CURSOR_SIZES: &[(i64, Label)] = &[
    (0, || tr!("choices-cursor-sizes-0")),
    (24, || tr!("choices-cursor-sizes-24")),
    (32, || tr!("choices-cursor-sizes-32")),
    (48, || tr!("choices-cursor-sizes-48")),
    (64, || tr!("choices-cursor-sizes-64")),
    (96, || tr!("choices-cursor-sizes-96")),
];

pub use repriseconf::config::{TOOLKIT_AUTO, TOOLKIT_REPRISE};

pub fn reprise_toolkit_row() -> String {
    tr!("choices-reprise-toolkit-row")
}

/// `[toolkits] qt_platform_theme`: "auto", "reprise", then the Qt
/// platform theme plugins a desktop Linux ships. Any other name shows as
/// itself.
pub const QT_PLATFORM_THEMES: &Choices = &[
    (TOOLKIT_AUTO, || {
        tr!("choices-qt-platform-themes-toolkit-auto")
    }),
    (TOOLKIT_REPRISE, reprise_toolkit_row),
    ("xdgdesktopportal", || {
        tr!("choices-qt-platform-themes-xdgdesktopportal")
    }),
    ("qt6ct", || tr!("choices-qt-platform-themes-qt6ct")),
    ("qt5ct", || tr!("choices-qt-platform-themes-qt5ct")),
    ("kde", || tr!("choices-qt-platform-themes-kde")),
    ("gtk3", || tr!("choices-qt-platform-themes-gtk3")),
];

/// `[toolkits] font_smoothing`: how GTK and the browsers smooth their
/// text. Mirrors `FontSmoothing::parse` in `repriseconf::config`.
pub const FONT_SMOOTHINGS: &Choices = &[
    ("grayscale", || tr!("choices-font-smoothings-grayscale")),
    ("subpixel", || tr!("choices-font-smoothings-subpixel")),
    ("none", || tr!("choices-font-smoothings-none")),
];

/// `[glass] mode`: `glass::Mode::as_str`, then `DarkSchedule::name`.
pub const GLASS_MODES: &Choices = &[
    ("light", || tr!("choices-glass-modes-light")),
    ("dark", || tr!("choices-glass-modes-dark")),
    ("night", || tr!("choices-glass-modes-night")),
    ("hours", || tr!("choices-glass-modes-hours")),
];

/// `[glass] material`. Mirrors `glass::Material::as_str`.
pub const GLASS_MATERIALS: &Choices = &[
    ("frosted", || tr!("choices-glass-materials-frosted")),
    ("smoked", || tr!("choices-glass-materials-smoked")),
    ("molten", || tr!("choices-glass-materials-molten")),
];

/// `[glass] notification_style`, the values of `glass::NotifyStyle::as_str`.
pub const GLASS_NOTIFY_STYLES: &Choices = &[
    ("lozenge", || tr!("choices-glass-notify-styles-lozenge")),
    ("halo", || tr!("choices-glass-notify-styles-halo")),
    ("droplet", || tr!("choices-glass-notify-styles-droplet")),
];

/// `[glass] frame`. Mirrors `glass::Frame::as_str`.
pub const GLASS_FRAMES: &Choices = &[
    ("rounded", || tr!("choices-glass-frames-rounded")),
    ("square", || tr!("choices-glass-frames-square")),
];

/// The Interface combo's items. Mirrors `Interface::parse` in reprisekit.
pub const INTERFACES: &Choices = &[
    ("classic", || tr!("choices-interfaces-classic")),
    ("blue", || tr!("choices-interfaces-blue")),
    ("glass", || tr!("choices-interfaces-glass")),
];

pub const WALLPAPER_MODES: &Choices = &[
    ("picture", || tr!("choices-wallpaper-modes-picture")),
    ("slideshow", || tr!("choices-wallpaper-modes-slideshow")),
];

/// The Remote Desktop quality trackbar's stops, left to right, and the name above it.
pub const REMOTE_QUALITIES: &Choices = &[
    ("speed", || tr!("choices-remote-qualities-speed")),
    ("balanced", || tr!("choices-remote-qualities-balanced")),
    ("sharp", || tr!("choices-remote-qualities-sharp")),
];

/// The [`REMOTE_QUALITIES`] row `v` names; a name the server does not know reads as its default.
pub fn remote_quality_row(v: &str) -> usize {
    let row = |name: &str| REMOTE_QUALITIES.iter().position(|(q, _)| *q == name);
    row(v)
        .or_else(|| row(repriseconf::remote::RemoteDesktop::default().quality.name()))
        .unwrap_or_default()
}

/// The Remote Desktop video encoders; the server greys those that do not work there.
pub const REMOTE_ENCODERS: &Choices = &[
    ("auto", || tr!("choices-remote-encoders-auto")),
    ("vaapi", || tr!("choices-remote-encoders-vaapi")),
    ("nvenc", || tr!("choices-remote-encoders-nvenc")),
    ("vulkan", || tr!("choices-remote-encoders-vulkan")),
    ("software", || tr!("choices-remote-encoders-software")),
    ("none", || tr!("choices-remote-encoders-none")),
];

/// The screen recording encoders: Remote Desktop's, less its RemoteFX row.
pub const ENCODERS: &Choices = REMOTE_ENCODERS.split_at(5).0;

/// The [`ENCODERS`] row for the CPU encoder of `[recording] codec` = `codec`.
pub fn software_row(codec: &str) -> String {
    let codec = Codec::from_name(codec).unwrap_or(Codec::H264);
    tr!("choices-software-row", codec = codec.software_name())
}

/// `[screenshot] format`. Mirrors `ShotFormat::extension` in `repriseconf::config`.
pub const SHOT_FORMATS: &Choices = &[
    ("png", || tr!("choices-shot-formats-png")),
    ("jpg", || tr!("choices-shot-formats-jpg")),
    ("jxl", || tr!("choices-shot-formats-jxl")),
];

/// `[screenshot] png_compression`. Mirrors `PngCompression::name` in `repriseconf::config`.
pub const PNG_COMPRESSIONS: &Choices = &[
    ("fast", || tr!("choices-png-compressions-fast")),
    ("default", || tr!("choices-png-compressions-default")),
    ("best", || tr!("choices-png-compressions-best")),
];

/// `[recording] codec`. Mirrors `recording::Codec::name`.
pub const VIDEO_CODECS: &Choices = &[
    ("h264", || tr!("choices-video-codecs-h264")),
    ("h265", || tr!("choices-video-codecs-h265")),
    ("av1", || tr!("choices-video-codecs-av1")),
];

/// `[recording] rate`. Mirrors `recording::RateMode::name`.
pub const RATE_MODES: &Choices = &[
    ("quality", || tr!("choices-rate-modes-quality")),
    ("average", || tr!("choices-rate-modes-average")),
    ("constant", || tr!("choices-rate-modes-constant")),
];

/// `[recording] frame_rate`'s rows; 0 is the screen's refresh rate.
pub const RECORD_FRAME_RATES: &[(i64, Label)] = &[
    (0, || tr!("choices-record-frame-rates-0")),
    (60, || tr!("choices-record-frame-rates-60")),
    (30, || tr!("choices-record-frame-rates-30")),
    (24, || tr!("choices-record-frame-rates-24")),
    (15, || tr!("choices-record-frame-rates-15")),
];

/// `[recording] audio_codec`. Mirrors `recording::AudioCodec::name`.
pub const AUDIO_CODECS: &Choices = &[
    ("aac", || tr!("choices-audio-codecs-aac")),
    ("opus", || tr!("choices-audio-codecs-opus")),
    ("flac", || tr!("choices-audio-codecs-flac")),
];

pub const FRAME_RATES: &[(i64, Label)] = &[
    (30, || tr!("choices-frame-rates-30")),
    (60, || tr!("choices-frame-rates-60")),
    (120, || tr!("choices-frame-rates-120")),
    (144, || tr!("choices-frame-rates-144")),
];

/// How long the Remote Desktop lock waits when the last viewer's screen went.
pub const LOCK_GRACES: &[(i64, Label)] = &[
    (0, || tr!("choices-lock-graces-0")),
    (5, || tr!("choices-lock-graces-5")),
    (10, || tr!("choices-lock-graces-10")),
    (30, || tr!("choices-lock-graces-30")),
];

pub const SLIDE_INTERVALS: &[(i64, Label)] = &[
    (1, || tr!("choices-slide-intervals-1")),
    (5, || tr!("choices-slide-intervals-5")),
    (10, || tr!("choices-slide-intervals-10")),
    (30, || tr!("choices-slide-intervals-30")),
    (60, || tr!("choices-slide-intervals-60")),
    (360, || tr!("choices-slide-intervals-360")),
    (1440, || tr!("choices-slide-intervals-1440")),
];

/// The wallpaper Position combo's items. Mirrors `Position::parse` in
/// reprisekit.
pub const POSITIONS: &Choices = &[
    ("fill", || tr!("choices-positions-fill")),
    ("fit", || tr!("choices-positions-fit")),
    ("stretch", || tr!("choices-positions-stretch")),
    ("tile", || tr!("choices-positions-tile")),
    ("center", || tr!("choices-positions-center")),
];

/// Settings > Background's desktop water styles; unchecked is "off".
/// Mirrors `reprisekit::water::Water::parse`.
pub const WATER_STYLES: &Choices = &[
    ("calm", || tr!("choices-water-styles-calm")),
    ("lively", || tr!("choices-water-styles-lively")),
    ("rain", || tr!("choices-water-styles-rain")),
    ("ink", || tr!("choices-water-styles-ink")),
    ("glow", || tr!("choices-water-styles-glow")),
    ("sparks", || tr!("choices-water-styles-sparks")),
    ("neon", || tr!("choices-water-styles-neon")),
];

/// The Background page's file name extensions combo. Mirrors `Extensions::parse` in
/// `reprisefs::names`.
pub const DESKTOP_EXTENSIONS: &Choices = &[
    ("hide", || tr!("choices-desktop-extensions-hide")),
    ("hide_launchers", || {
        tr!("choices-desktop-extensions-hide-launchers")
    }),
    ("show", || tr!("choices-desktop-extensions-show")),
];

/// Default Programs' plugged-in drive combo. Mirrors `OnInsert::parse` in reprise-drives.
pub const DRIVE_INSERTS: &Choices = &[
    ("ask", || tr!("choices-drive-inserts-ask")),
    ("mount", || tr!("choices-drive-inserts-mount")),
    ("open", || tr!("choices-drive-inserts-open")),
    ("nothing", || tr!("choices-drive-inserts-nothing")),
];

/// Default Programs' script opening combo. Mirrors `ScriptOpen::parse` in `reprisefs::program`.
pub const SCRIPT_OPENS: &Choices = &[
    ("run", || tr!("choices-script-opens-run")),
    ("terminal", || tr!("choices-script-opens-terminal")),
];

/// Default Programs' script combo. Mirrors `ScriptEnd::parse` in `reprisefs::program`.
pub const SCRIPT_ENDS: &Choices = &[
    ("stay", || tr!("choices-script-ends-stay")),
    ("close", || tr!("choices-script-ends-close")),
];

/// The Recycle Bin, which always follows the other system icons.
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

pub const DESKTOPS_DEFAULT: i64 = repriseconf::config::WORKSPACES_DEFAULT as i64;

pub const DESKTOPS_MAX: i64 = repriseconf::config::WORKSPACES_MAX as i64;

pub const DIM_INACTIVE_MAX: i64 = repriseconf::config::DIM_INACTIVE_MAX as i64;

pub const HIDDEN_FRAME_RATE_MAX: i64 = repriseconf::config::HIDDEN_FRAME_RATE_MAX as i64;

pub const ATTENTION_FLASH_MAX: i64 = repriseconf::config::ATTENTION_FLASH_MAX as i64;

pub const SNAP_DISTANCE_MAX: i64 = repriseconf::config::SNAP_DISTANCE_MAX as i64;

pub const SWITCH_MS_MAX: i64 = repriseconf::config::SWITCH_MS_MAX as i64;

pub const EFFECTS_MS_MAX: i64 = repriseconf::config::EFFECTS_MS_MAX as i64;

pub const PREVIEW_FPS_MAX: i64 = repriseconf::config::PREVIEW_FPS_MAX as i64;
