#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerAction {
    Nothing,
    ScreenOff,
    Lock,
    Standby,
    Hibernate,
    TurnOff,
    Ask,
}

impl PowerAction {
    pub(super) const NAMES: [(&'static str, PowerAction); 7] = [
        ("nothing", PowerAction::Nothing),
        ("screen_off", PowerAction::ScreenOff),
        ("lock", PowerAction::Lock),
        ("standby", PowerAction::Standby),
        ("hibernate", PowerAction::Hibernate),
        ("turnoff", PowerAction::TurnOff),
        ("ask", PowerAction::Ask),
    ];

    pub fn parse(s: &str) -> Result<PowerAction, String> {
        Self::NAMES
            .iter()
            .find(|(n, _)| *n == s)
            .map(|(_, a)| *a)
            .ok_or_else(|| {
                let names: Vec<&str> = Self::NAMES.iter().map(|(n, _)| *n).collect();
                format!("unknown power action {s:?}; one of {}", names.join(", "))
            })
    }

    pub fn name(self) -> &'static str {
        Self::NAMES
            .iter()
            .find(|(_, a)| *a == self)
            .map(|(n, _)| *n)
            .expect("every action has a name")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerConfig {
    pub standby: String,
    pub poweroff: String,
    pub reboot: String,
    pub hibernate: String,
    pub lid_close: PowerAction,
    pub power_button: PowerAction,
    pub sleep_button: PowerAction,
    /// Percent. reprise-power raises the battery alarms; the compositor only carries these.
    pub battery_low: u32,
    pub battery_critical: u32,
    pub critical_action: PowerAction,
    pub always_show_icon: bool,
    pub lock_on_sleep: bool,
    /// Command for an ext-session-lock-v1 locker; empty keeps the built-in dialog (`sessionlock`).
    pub locker: String,
}

impl Default for PowerConfig {
    fn default() -> Self {
        PowerConfig {
            standby: "systemctl suspend".into(),
            poweroff: "systemctl poweroff".into(),
            reboot: "systemctl reboot".into(),
            hibernate: "systemctl hibernate".into(),
            lid_close: PowerAction::Standby,
            power_button: PowerAction::Ask,
            sleep_button: PowerAction::Standby,
            battery_low: 10,
            battery_critical: 3,
            critical_action: PowerAction::Hibernate,
            always_show_icon: false,
            lock_on_sleep: true,
            locker: String::new(),
        }
    }
}

/// Idle timeouts in seconds; zero disables the step. All pause while a client holds an idle inhibitor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdleConfig {
    pub lock_after: u32,
    pub screen_off_after: u32,
    pub blank_after: u32,
    pub suspend_after: u32,
    /// Seconds to dim before `lock_after`; any input brings it back. Zero locks
    /// without a dim; meaningless with `lock_after` 0.
    pub lock_fade: u32,
}

impl Default for IdleConfig {
    fn default() -> Self {
        IdleConfig {
            lock_after: 600,
            screen_off_after: 900,
            blank_after: 0,
            suspend_after: 0,
            lock_fade: 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdleOverrides {
    pub lock_after: Option<u32>,
    pub screen_off_after: Option<u32>,
    pub blank_after: Option<u32>,
    pub suspend_after: Option<u32>,
}

impl IdleOverrides {
    pub(super) fn apply(&self, base: &IdleConfig) -> IdleConfig {
        IdleConfig {
            lock_after: self.lock_after.unwrap_or(base.lock_after),
            screen_off_after: self.screen_off_after.unwrap_or(base.screen_off_after),
            blank_after: self.blank_after.unwrap_or(base.blank_after),
            suspend_after: self.suspend_after.unwrap_or(base.suspend_after),
            lock_fade: base.lock_fade,
        }
    }
}
