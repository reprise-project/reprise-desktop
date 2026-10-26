#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyboardConfig {
    pub rules: String,
    pub model: String,
    pub layout: String,
    pub variant: String,
    pub options: String,
    pub repeat_rate: i32,
    pub repeat_delay: i32,
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
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PointerConfig {
    pub speed: f64,
    pub acceleration: bool,
    pub natural_scroll: bool,
}

impl Default for PointerConfig {
    fn default() -> Self {
        PointerConfig {
            speed: 0.0,
            acceleration: true,
            natural_scroll: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TouchpadConfig {
    pub pointer: PointerConfig,
    pub tap_to_click: bool,
    pub two_finger_scroll: bool,
    pub two_finger_tap_right_click: bool,
    pub disable_while_typing: bool,
}

impl Default for TouchpadConfig {
    fn default() -> Self {
        TouchpadConfig {
            pointer: PointerConfig::default(),
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
