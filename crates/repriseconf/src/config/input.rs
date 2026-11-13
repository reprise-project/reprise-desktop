#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyboardConfig {
    pub rules: String,
    pub model: String,
    pub layout: String,
    pub variant: String,
    pub options: String,
    pub repeat_rate: i32,
    pub repeat_delay: i32,
    /// Each window keeps the layout last active while it had the keyboard.
    pub layout_per_window: bool,
}

impl Default for KeyboardConfig {
    fn default() -> Self {
        KeyboardConfig {
            rules: String::new(),
            model: String::new(),
            layout: String::new(),
            variant: String::new(),
            options: String::new(),
            repeat_rate: 25,
            repeat_delay: 600,
            layout_per_window: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PointerConfig {
    pub speed: f64,
    pub acceleration: bool,
    pub natural_scroll: bool,
    /// What a scroll is multiplied by before programs see it, [`PointerConfig::SCROLL_SPEED`].
    pub scroll_speed: f64,
    /// Left and right pressed together are a middle click.
    pub middle_emulation: bool,
}

impl PointerConfig {
    pub const SCROLL_SPEED: std::ops::RangeInclusive<f64> = 0.1..=10.0;
}

impl Default for PointerConfig {
    fn default() -> Self {
        PointerConfig {
            speed: 0.0,
            acceleration: true,
            natural_scroll: false,
            scroll_speed: 1.0,
            middle_emulation: false,
        }
    }
}

/// `[touchpad] click_method`: what a press on a clickpad clicks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClickMethod {
    /// The pad's own default.
    #[default]
    Default,
    /// The bottom strip is split into left, middle and right buttons.
    Areas,
    /// One, two or three fingers on the pad click left, right or middle.
    Clickfinger,
}

impl ClickMethod {
    pub const ALL: [ClickMethod; 3] = [
        ClickMethod::Default,
        ClickMethod::Areas,
        ClickMethod::Clickfinger,
    ];

    pub fn parse(s: &str) -> Option<ClickMethod> {
        ClickMethod::ALL.into_iter().find(|m| m.name() == s)
    }

    pub fn name(self) -> &'static str {
        match self {
            ClickMethod::Default => "default",
            ClickMethod::Areas => "areas",
            ClickMethod::Clickfinger => "clickfinger",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TouchpadConfig {
    pub pointer: PointerConfig,
    /// Off, the pad moves nothing; the touchpad keys flip it.
    pub enabled: bool,
    /// The pad is off while a mouse is plugged in.
    pub disable_with_mouse: bool,
    pub click_method: ClickMethod,
    pub tap_to_click: bool,
    pub two_finger_scroll: bool,
    pub two_finger_tap_right_click: bool,
    pub disable_while_typing: bool,
}

impl Default for TouchpadConfig {
    fn default() -> Self {
        TouchpadConfig {
            pointer: PointerConfig::default(),
            enabled: true,
            disable_with_mouse: false,
            click_method: ClickMethod::Default,
            tap_to_click: true,
            two_finger_scroll: true,
            two_finger_tap_right_click: true,
            disable_while_typing: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TouchscreenConfig {
    pub output: Option<String>,
}

/// A `[[tablets]]` entry: the first whose `name` matches a tablet maps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabletConfig {
    /// A glob over the tablet's device name; `*` when left out.
    pub name: String,
    pub output: Option<String>,
    /// Map the tablet to the largest rect of its own shape inside the area, centred.
    pub keep_aspect: bool,
}

/// The `[[tablets]]` entry for the tablet called `name`.
pub fn find_tablet<'a>(list: &'a [TabletConfig], name: &str) -> Option<&'a TabletConfig> {
    list.iter().find(|t| crate::glob::glob(&t.name, name))
}

/// `[shortcut_overlay]`: the keyboard shortcut sheet holding Super alone shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortcutOverlay {
    pub enabled: bool,
    /// How long Super is held alone before the sheet shows, in ms.
    pub delay_ms: u32,
}

impl ShortcutOverlay {
    pub const DELAY_MS: std::ops::RangeInclusive<u32> = 200..=5000;
}

impl Default for ShortcutOverlay {
    fn default() -> Self {
        ShortcutOverlay {
            enabled: true,
            delay_ms: 1000,
        }
    }
}
