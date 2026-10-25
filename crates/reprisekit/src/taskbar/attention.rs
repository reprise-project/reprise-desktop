//! A task button whose window wants attention: it flashes for a while (`[taskbar] attention_flash`),
//! then stays lit until the window is focused.

use std::time::Duration;

/// How long a flash stays lit, and how long it stays dark after.
pub const FLASH_HALF: Duration = Duration::from_millis(500);

/// What a task button shows of its windows' wish for attention.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Attention {
    #[default]
    None,
    /// Flashing, in its lit half.
    FlashOn,
    /// Flashing, in its dark half: drawn as it would be without attention.
    FlashOff,
    /// Done flashing; lit until the window is focused.
    Steady,
}

impl Attention {
    /// The phase `elapsed` after the window began to ask, for a button that flashes for `flash_for`.
    pub fn after(elapsed: Duration, flash_for: Duration) -> Attention {
        if elapsed >= flash_for {
            return Attention::Steady;
        }
        if (elapsed.as_millis() / FLASH_HALF.as_millis()).is_multiple_of(2) {
            Attention::FlashOn
        } else {
            Attention::FlashOff
        }
    }

    /// How long after the window began to ask the phase next changes, seen at `elapsed`; None once
    /// steady.
    pub fn next_change(elapsed: Duration, flash_for: Duration) -> Option<Duration> {
        if elapsed >= flash_for {
            return None;
        }
        let half = FLASH_HALF.as_millis();
        let next = (elapsed.as_millis() / half + 1) * half;
        Some(Duration::from_millis(next as u64).min(flash_for))
    }

    /// Whether a window wants attention, lit or not.
    pub fn wants(self) -> bool {
        self != Attention::None
    }

    /// Whether the button is drawn in its attention colours now.
    pub fn lit(self) -> bool {
        matches!(self, Attention::FlashOn | Attention::Steady)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOR: Duration = Duration::from_secs(30);

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    #[test]
    fn a_new_ask_starts_lit_and_alternates_every_half_second() {
        assert_eq!(Attention::after(ms(0), FOR), Attention::FlashOn);
        assert_eq!(Attention::after(ms(499), FOR), Attention::FlashOn);
        assert_eq!(Attention::after(ms(500), FOR), Attention::FlashOff);
        assert_eq!(Attention::after(ms(999), FOR), Attention::FlashOff);
        assert_eq!(Attention::after(ms(1000), FOR), Attention::FlashOn);
    }

    #[test]
    fn it_flashes_for_its_time_then_stays_lit() {
        let lit_halves = (0..)
            .map(|k| Attention::after(ms(k * 500 + 250), FOR))
            .take_while(|a| *a != Attention::Steady)
            .filter(|a| *a == Attention::FlashOn)
            .count();
        assert_eq!(lit_halves, 30, "one lit half a second for 30 s");
        assert_eq!(Attention::after(ms(29_999), FOR), Attention::FlashOff);
        assert_eq!(Attention::after(FOR, FOR), Attention::Steady);
        assert_eq!(
            Attention::after(Duration::from_secs(3600), FOR),
            Attention::Steady
        );
    }

    #[test]
    fn no_flash_time_lights_it_at_once() {
        assert_eq!(Attention::after(ms(0), Duration::ZERO), Attention::Steady);
        assert_eq!(Attention::next_change(ms(0), Duration::ZERO), None);
    }

    #[test]
    fn the_next_change_is_the_next_half_boundary_until_steady() {
        assert_eq!(Attention::next_change(ms(0), FOR), Some(ms(500)));
        assert_eq!(Attention::next_change(ms(730), FOR), Some(ms(1000)));
        assert_eq!(Attention::next_change(ms(29_999), FOR), Some(FOR));
        assert_eq!(Attention::next_change(FOR, FOR), None);
        assert_eq!(
            Attention::next_change(ms(1200), ms(1300)),
            Some(ms(1300)),
            "a flash time off the half boundary ends where it says"
        );
    }

    #[test]
    fn only_lit_phases_draw_the_attention_colours() {
        let lit = [
            Attention::None,
            Attention::FlashOn,
            Attention::FlashOff,
            Attention::Steady,
        ]
        .map(Attention::lit);
        assert_eq!(lit, [false, true, false, true]);
        assert!(
            Attention::FlashOff.wants(),
            "a dark half still wants attention"
        );
        assert!(!Attention::None.wants());
    }
}
