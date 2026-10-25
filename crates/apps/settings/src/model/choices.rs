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

/// How menus open. Mirrors `MenuAnimation::name` in `src/config.rs`.
pub const MENU_ANIMATIONS: &Choices = &[("fade", "Fade"), ("slide", "Slide"), ("none", "None")];

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

/// `[glass] notification_style`, the values of `glass::NotifyStyle::as_str`.
pub const GLASS_NOTIFY_STYLES: &Choices = &[
    ("lozenge", "Lozenge"),
    ("halo", "Halo"),
    ("droplet", "Droplet"),
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
pub const WATER_STYLES: &Choices = &[
    ("calm", "Calm"),
    ("lively", "Lively"),
    ("rain", "Rain"),
    ("ink", "Ink"),
    ("glow", "Glow"),
    ("sparks", "Sparks"),
    ("neon", "Neon"),
];

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

/// Default Programs' script opening combo. Mirrors `ScriptOpen::parse` in `reprisefs::program`.
pub const SCRIPT_OPENS: &Choices = &[("run", "Run it"), ("terminal", "Run it in the terminal")];

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

/// `[window_snapping] distance`'s default and ceiling. Mirror `snap::BAND` and
/// `SNAP_DISTANCE_MAX` in `src/config.rs`.
pub const SNAP_DISTANCE_DEFAULT: i64 = 30;

pub const SNAP_DISTANCE_MAX: i64 = 100;

/// Longest `[workspaces] animation_ms`. Mirrors `SWITCH_MS_MAX`.
pub const SWITCH_MS_MAX: i64 = 2000;

/// Longest `[effects]` duration. Mirrors `EFFECTS_MS_MAX`.
pub const EFFECTS_MS_MAX: i64 = 2000;

/// Most `[previews] fps`. Mirrors `PREVIEW_FPS_MAX`.
pub const PREVIEW_FPS_MAX: i64 = 60;
