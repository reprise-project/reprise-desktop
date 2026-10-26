use reprisekit::interface::Interface;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SwitchStyle {
    #[default]
    None,
    Slide,
    SlideVertical,
    Fade,
    Zoom,
    Wipe,
}

impl SwitchStyle {
    pub const ALL: [SwitchStyle; 6] = [
        SwitchStyle::None,
        SwitchStyle::Slide,
        SwitchStyle::SlideVertical,
        SwitchStyle::Fade,
        SwitchStyle::Zoom,
        SwitchStyle::Wipe,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SwitchStyle::None => "none",
            SwitchStyle::Slide => "slide",
            SwitchStyle::SlideVertical => "slide-vertical",
            SwitchStyle::Fade => "fade",
            SwitchStyle::Zoom => "zoom",
            SwitchStyle::Wipe => "wipe",
        }
    }

    pub fn from_name(s: &str) -> Option<SwitchStyle> {
        SwitchStyle::ALL.into_iter().find(|v| v.name() == s)
    }
}

pub const SWITCH_MS_MAX: u32 = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwitchAnimation {
    pub style: SwitchStyle,
    pub ms: u32,
}

impl Default for SwitchAnimation {
    fn default() -> Self {
        SwitchAnimation {
            style: SwitchStyle::default(),
            ms: 200,
        }
    }
}

impl SwitchAnimation {
    pub fn enabled(&self) -> bool {
        self.style != SwitchStyle::None && self.ms > 0
    }
}

pub const EFFECTS_MS_MAX: u32 = 2000;

/// How a menu or the Start panel opens over `[metrics] menu_fade_ms`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MenuAnimation {
    None,
    #[default]
    Fade,
    /// It unrolls from the side facing what opened it.
    Slide,
}

impl MenuAnimation {
    pub const ALL: [MenuAnimation; 3] = [
        MenuAnimation::None,
        MenuAnimation::Fade,
        MenuAnimation::Slide,
    ];

    pub fn name(self) -> &'static str {
        match self {
            MenuAnimation::None => "none",
            MenuAnimation::Fade => "fade",
            MenuAnimation::Slide => "slide",
        }
    }

    pub fn from_name(s: &str) -> Option<MenuAnimation> {
        MenuAnimation::ALL.into_iter().find(|v| v.name() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectsConfig {
    pub window_animation: bool,
    pub window_open_ms: u32,
    pub window_close_ms: u32,
    pub window_scale: f64,
    pub hover_fade: bool,
    pub hover_in_ms: u32,
    pub hover_out_ms: u32,
    pub taskbar_slide: bool,
    pub taskbar_hide_ms: u32,
    /// The same under every interface.
    pub menu_animation: MenuAnimation,
    /// On under every interface, unlike the switches above.
    pub session_fade: bool,
    pub session_fade_in_ms: u32,
    pub session_fade_out_ms: u32,
}

impl EffectsConfig {
    pub fn default_for(interface: Interface) -> EffectsConfig {
        let glass = interface == Interface::Glass;
        EffectsConfig {
            window_animation: glass,
            window_open_ms: 250,
            window_close_ms: 200,
            window_scale: 0.95,
            hover_fade: glass,
            hover_in_ms: 120,
            hover_out_ms: 250,
            taskbar_slide: glass,
            taskbar_hide_ms: 200,
            menu_animation: MenuAnimation::Fade,
            session_fade: true,
            session_fade_in_ms: 600,
            session_fade_out_ms: 400,
        }
    }

    pub fn opens(&self) -> bool {
        self.window_animation && self.window_open_ms > 0
    }

    pub fn closes(&self) -> bool {
        self.window_animation && self.window_close_ms > 0
    }

    /// (in, out): both 0, instant, while `hover_fade` is off.
    pub fn hover_ms(&self) -> (u32, u32) {
        if self.hover_fade {
            (self.hover_in_ms, self.hover_out_ms)
        } else {
            (0, 0)
        }
    }

    /// (in, out) of the screen's fade at session start and end: both 0 while
    /// `session_fade` is off.
    pub fn session_fade_ms(&self) -> (u32, u32) {
        if self.session_fade {
            (self.session_fade_in_ms, self.session_fade_out_ms)
        } else {
            (0, 0)
        }
    }
}
