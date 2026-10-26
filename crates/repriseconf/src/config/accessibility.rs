#[derive(Clone, Debug, PartialEq)]
pub struct NightLightConfig {
    pub enabled: bool,
    pub temperature: u32,
    pub schedule: NightSchedule,
    /// Minutes after local midnight.
    pub from: u32,
    pub to: u32,
    /// (latitude, longitude): degrees north and east.
    pub location: Option<(f64, f64)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NightSchedule {
    Sun,
    Fixed,
}

impl NightSchedule {
    pub fn name(self) -> &'static str {
        match self {
            NightSchedule::Sun => "sun",
            NightSchedule::Fixed => "fixed",
        }
    }

    pub fn parse(s: &str) -> Option<NightSchedule> {
        [NightSchedule::Sun, NightSchedule::Fixed]
            .into_iter()
            .find(|n| n.name() == s)
    }
}

impl Default for NightLightConfig {
    fn default() -> Self {
        NightLightConfig {
            enabled: false,
            temperature: 4000,
            schedule: NightSchedule::Sun,
            from: 21 * 60,
            to: 7 * 60,
            location: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccessibilityConfig {
    pub color_filter: reprisekit::colorfilter::Filter,
    pub color_filter_intensity: u8,
    pub magnifier_follow: Follow,
    pub sticky_keys: bool,
    pub sticky_lock: bool,
    pub sticky_two_keys_off: bool,
    pub slow_keys: bool,
    /// Milliseconds; the file gives seconds.
    pub slow_keys_delay: u32,
    pub shortcuts: bool,
    pub mouse_keys: bool,
    pub mouse_keys_speed: u32,
    /// Milliseconds; the file gives seconds.
    pub mouse_keys_acceleration: u32,
    pub mouse_keys_num_lock: bool,
    pub text_size: u32,
}

impl Default for AccessibilityConfig {
    fn default() -> Self {
        AccessibilityConfig {
            color_filter: reprisekit::colorfilter::Filter::None,
            color_filter_intensity: 100,
            magnifier_follow: Follow::default(),
            sticky_keys: false,
            sticky_lock: true,
            sticky_two_keys_off: true,
            slow_keys: false,
            slow_keys_delay: 1000,
            shortcuts: true,
            mouse_keys: false,
            mouse_keys_speed: 400,
            mouse_keys_acceleration: 1000,
            mouse_keys_num_lock: true,
            text_size: 100,
        }
    }
}

impl AccessibilityConfig {
    pub fn text_factor(&self) -> f64 {
        self.text_size as f64 / 100.0
    }
}

pub const SLOW_KEYS_DELAY: std::ops::RangeInclusive<f64> = 0.3..=2.0;

pub const MOUSE_KEYS_SPEED: std::ops::RangeInclusive<u32> = 50..=2000;

pub const MOUSE_KEYS_ACCELERATION: std::ops::RangeInclusive<f64> = 0.0..=5.0;

pub const TEXT_SIZE: std::ops::RangeInclusive<u32> = 100..=200;
pub const TEXT_SIZE_STEP: u32 = 25;

pub const NIGHT_KELVIN: std::ops::RangeInclusive<u32> = 1000..=6500;

pub fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    let digits = |t: &str| t.bytes().all(|b| b.is_ascii_digit());
    if !(1..=2).contains(&h.len()) || m.len() != 2 || !digits(h) || !digits(m) {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// What the magnifier's view follows: `[accessibility] magnifier_follow`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Follow {
    Pointer,
    #[default]
    PointerAndFocus,
}

impl Follow {
    pub fn name(self) -> &'static str {
        match self {
            Follow::Pointer => "pointer",
            Follow::PointerAndFocus => "pointer+focus",
        }
    }

    pub fn parse(name: &str) -> Option<Follow> {
        [Follow::Pointer, Follow::PointerAndFocus]
            .into_iter()
            .find(|f| f.name() == name)
    }

    pub fn focus(self) -> bool {
        self == Follow::PointerAndFocus
    }
}
