//! Auto-hide for one taskbar (`[taskbar] auto_hide`), with no clock or scene of its own: the host hands it `Signals`, `Rules` and the time.
//! A bar goes `Shown -> Hiding -> Hidden -> Revealing -> Shown`. A hold keeps a shown bar shown but never brings a hidden one back;
//! over a focused fullscreen window only the Start key does. A slide turns back from wherever it is.

use std::time::{Duration, Instant};

pub const EDGE_REST: Duration = Duration::from_millis(300);
pub const HIDE_DELAY: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Shown,
    Hiding,
    Hidden,
    Revealing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rules {
    pub edge: bool,
    pub keys: bool,
    pub attention: bool,
    pub slide_ms: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Signals {
    /// The pointer, a client drag's too, is on the bar's reveal line.
    pub on_line: bool,
    pub over_bar: bool,
    /// A surface the bar owns is open: the Start menu, a tray or SNI menu,
    /// the calendar, a preview, a jump list or group list, the hidden
    /// icons' flyout, the notification history, a context menu, or a
    /// client popup a press on its tray opened.
    pub owned_open: bool,
    pub dragging: bool,
    pub switcher: bool,
    /// A window on the current desktop asks for attention on this output,
    /// or a balloon is up there.
    pub attention: bool,
    pub fullscreen: bool,
}

#[derive(Clone, Debug)]
pub struct AutoHide {
    phase: Phase,
    from: f32,
    since: Instant,
    ms: u32,
    line_since: Option<Instant>,
    free_since: Option<Instant>,
}

impl AutoHide {
    pub fn new(now: Instant) -> AutoHide {
        AutoHide {
            phase: Phase::Shown,
            from: 0.0,
            since: now,
            ms: 0,
            line_since: None,
            free_since: None,
        }
    }

    #[cfg(test)]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn headed_out(&self) -> bool {
        matches!(self.phase, Phase::Hiding | Phase::Hidden)
    }

    pub fn hidden(&self, now: Instant) -> f32 {
        let step = if self.ms == 0 {
            1.0
        } else {
            now.saturating_duration_since(self.since).as_micros() as f32 / (self.ms as f32 * 1000.0)
        };
        match self.phase {
            Phase::Shown => 0.0,
            Phase::Hidden => 1.0,
            Phase::Hiding => (self.from + step).min(1.0),
            Phase::Revealing => (self.from - step).max(0.0),
        }
    }

    pub fn moving(&self, now: Instant) -> bool {
        self.slide_end().is_some_and(|end| now < end)
    }

    /// A pointer button went down on the reveal line: with `reveal_on_edge`
    /// on and no fullscreen window over the bar, it shows. The host steps
    /// `update` right after.
    pub fn pressed(&mut self, r: Rules, fullscreen: bool, now: Instant) {
        if r.edge && !fullscreen {
            self.reveal(r, now);
        }
    }

    /// The Start key, Ctrl+Esc (`start`) or Alt+Tab acted on this bar's
    /// output: with `reveal_on_keys` on it shows, over a fullscreen window
    /// only for the Start key. The host steps `update` right after.
    pub fn keyed(&mut self, r: Rules, start: bool, fullscreen: bool, now: Instant) {
        if r.keys && (start || !fullscreen) {
            self.reveal(r, now);
        }
    }

    /// A bar sliding in lands shown at once, for a key that acts on where it
    /// is headed. Any other phase stays.
    pub fn land(&mut self, now: Instant) {
        if self.phase == Phase::Revealing {
            self.settle(Phase::Shown, now);
        }
    }

    pub fn update(&mut self, s: Signals, r: Rules, now: Instant) {
        self.retime(r, now);
        if self.slide_end().is_some_and(|end| now >= end) {
            let done = if self.phase == Phase::Hiding {
                Phase::Hidden
            } else {
                Phase::Shown
            };
            self.settle(done, now);
        }
        self.line_since = if s.on_line {
            Some(self.line_since.unwrap_or(now))
        } else {
            None
        };
        let open = !s.fullscreen;
        let rested = self
            .line_since
            .is_some_and(|t| now.saturating_duration_since(t) >= EDGE_REST);
        let keys = r.keys && s.switcher && open;
        let attention = r.attention && s.attention && open;
        let trigger = (r.edge && open && rested) || keys || attention;
        let held = s.over_bar || s.owned_open || s.dragging || keys || attention;
        match self.phase {
            Phase::Hidden if trigger => self.reveal(r, now),
            Phase::Hiding if trigger || (s.over_bar && open) => self.reveal(r, now),
            Phase::Shown if held => self.free_since = None,
            Phase::Shown => {
                let free = *self.free_since.get_or_insert(now);
                if now.saturating_duration_since(free) >= HIDE_DELAY {
                    self.slide(Phase::Hiding, r, now);
                }
            }
            _ => {}
        }
    }

    pub fn wake(&self, r: Rules, now: Instant) -> Option<Instant> {
        let rest = self
            .line_since
            .filter(|_| r.edge && self.headed_out())
            .map(|t| t + EDGE_REST);
        let delay = self
            .free_since
            .filter(|_| self.phase == Phase::Shown)
            .map(|t| t + HIDE_DELAY);
        [rest, delay, self.slide_end()]
            .into_iter()
            .flatten()
            .filter(|&t| t > now)
            .min()
    }

    fn slide_end(&self) -> Option<Instant> {
        let left = match self.phase {
            Phase::Hiding => 1.0 - self.from,
            Phase::Revealing => self.from,
            Phase::Shown | Phase::Hidden => return None,
        };
        let us = (left * self.ms as f32 * 1000.0).round() as u64;
        Some(self.since + Duration::from_micros(us))
    }

    fn reveal(&mut self, r: Rules, now: Instant) {
        self.free_since = None;
        if self.headed_out() {
            self.slide(Phase::Revealing, r, now);
        }
    }

    fn slide(&mut self, to: Phase, r: Rules, now: Instant) {
        self.from = self.hidden(now);
        self.since = now;
        self.ms = r.slide_ms;
        self.phase = to;
        self.free_since = None;
        if self.slide_end().is_some_and(|end| now >= end) {
            let done = if to == Phase::Hiding {
                Phase::Hidden
            } else {
                Phase::Shown
            };
            self.settle(done, now);
        }
    }

    fn settle(&mut self, to: Phase, now: Instant) {
        self.phase = to;
        self.from = if to == Phase::Hidden { 1.0 } else { 0.0 };
        self.since = now;
    }

    fn retime(&mut self, r: Rules, now: Instant) {
        if r.slide_ms != self.ms {
            self.from = self.hidden(now);
            self.since = now;
            self.ms = r.slide_ms;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64) -> Instant {
        static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *T0.get_or_init(Instant::now) + Duration::from_millis(ms)
    }

    fn rules() -> Rules {
        Rules {
            edge: true,
            keys: true,
            attention: true,
            slide_ms: 200,
        }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn hidden_bar(r: Rules) -> AutoHide {
        let mut a = AutoHide::new(at(0));
        for ms in [0, 500, 700] {
            a.update(Signals::default(), r, at(ms));
        }
        assert_eq!(a.phase(), Phase::Hidden);
        a
    }

    #[test]
    fn a_new_bar_hides_half_a_second_after_nothing_holds_it() {
        let r = rules();
        let mut a = AutoHide::new(at(0));
        a.update(Signals::default(), r, at(0));
        assert_eq!(a.phase(), Phase::Shown);
        assert_eq!(a.wake(r, at(0)), Some(at(500)), "it wakes for the delay");
        a.update(Signals::default(), r, at(499));
        assert_eq!(a.phase(), Phase::Shown, "not before the delay");
        a.update(Signals::default(), r, at(500));
        assert_eq!(a.phase(), Phase::Hiding);
        assert!(a.headed_out());
    }

    #[test]
    fn the_bar_slides_out_over_the_hide_time() {
        let r = rules();
        let mut a = AutoHide::new(at(0));
        a.update(Signals::default(), r, at(0));
        a.update(Signals::default(), r, at(500));
        assert!(near(a.hidden(at(500)), 0.0));
        assert!(near(a.hidden(at(600)), 0.5), "halfway at half the time");
        assert!(a.moving(at(600)));
        assert!(near(a.hidden(at(700)), 1.0));
        assert!(!a.moving(at(700)));
        assert_eq!(
            a.wake(r, at(500)),
            Some(at(700)),
            "it wakes at the slide's end"
        );
        a.update(Signals::default(), r, at(700));
        assert_eq!(a.phase(), Phase::Hidden);
        assert_eq!(a.wake(r, at(700)), None, "hidden, only the host wakes it");
    }

    #[test]
    fn zero_ms_hides_and_shows_at_once() {
        let r = Rules {
            slide_ms: 0,
            ..rules()
        };
        let mut a = AutoHide::new(at(0));
        a.update(Signals::default(), r, at(0));
        a.update(Signals::default(), r, at(500));
        assert_eq!(a.phase(), Phase::Hidden);
        assert!(near(a.hidden(at(500)), 1.0));
        a.pressed(r, false, at(600));
        assert_eq!(a.phase(), Phase::Shown);
        assert!(near(a.hidden(at(600)), 0.0));
    }

    #[test]
    fn resting_on_the_line_for_300_ms_reveals_it() {
        let r = rules();
        let line = Signals {
            on_line: true,
            ..Signals::default()
        };
        let mut a = hidden_bar(r);
        a.update(line, r, at(1000));
        assert_eq!(a.phase(), Phase::Hidden);
        assert_eq!(
            a.wake(r, at(1000)),
            Some(at(1300)),
            "it wakes when the rest is up"
        );
        a.update(line, r, at(1299));
        assert_eq!(a.phase(), Phase::Hidden, "not before 300 ms");
        a.update(line, r, at(1300));
        assert_eq!(a.phase(), Phase::Revealing);
        let mut a = hidden_bar(r);
        a.update(line, r, at(2000));
        a.update(Signals::default(), r, at(2100));
        a.update(line, r, at(2200));
        a.update(line, r, at(2499));
        assert_eq!(a.phase(), Phase::Hidden, "the rest runs from 2200");
        a.update(line, r, at(2500));
        assert_eq!(a.phase(), Phase::Revealing);
    }

    #[test]
    fn the_edge_trigger_off_ignores_the_line_and_presses() {
        let r = Rules {
            edge: false,
            ..rules()
        };
        let line = Signals {
            on_line: true,
            ..Signals::default()
        };
        let mut a = hidden_bar(r);
        a.update(line, r, at(1000));
        assert_eq!(a.wake(r, at(1000)), None, "no rest to wait for");
        a.update(line, r, at(1400));
        a.pressed(r, false, at(1500));
        assert_eq!(a.phase(), Phase::Hidden);
    }

    #[test]
    fn a_press_on_the_line_reveals_at_once() {
        let r = rules();
        let mut a = hidden_bar(r);
        a.pressed(r, false, at(1000));
        assert_eq!(a.phase(), Phase::Revealing);
        assert!(near(a.hidden(at(1100)), 0.5));
        a.update(Signals::default(), r, at(1200));
        assert_eq!(a.phase(), Phase::Shown);
    }

    #[test]
    fn a_reveal_turns_the_bar_toward_the_screen_before_it_moves() {
        let r = rules();
        let mut a = hidden_bar(r);
        assert!(a.headed_out());
        a.keyed(r, true, false, at(1000));
        assert!(!a.headed_out(), "a menu opened now docks on the shown bar");
        assert!(
            near(a.hidden(at(1000)), 1.0),
            "while it is still drawn hidden"
        );
    }

    #[test]
    fn a_bar_sliding_in_lands_shown_at_once_and_a_hiding_one_stays() {
        let r = rules();
        let mut a = hidden_bar(r);
        a.keyed(r, true, false, at(1000));
        assert!(a.hidden(at(1050)) > 0.0, "still sliding in");
        a.land(at(1050));
        assert_eq!(a.phase(), Phase::Shown);
        assert_eq!(a.hidden(at(1050)), 0.0, "drawn shown");
        assert!(!a.moving(at(1050)));

        let mut b = AutoHide::new(at(0));
        b.update(Signals::default(), r, at(0));
        b.update(Signals::default(), r, at(500));
        assert_eq!(b.phase(), Phase::Hiding);
        b.land(at(550));
        assert_eq!(b.phase(), Phase::Hiding, "a hiding bar keeps sliding out");
    }

    #[test]
    fn the_start_key_reveals_and_holds_until_the_menu_closes() {
        let r = rules();
        let menu = Signals {
            owned_open: true,
            ..Signals::default()
        };
        let mut a = hidden_bar(r);
        a.keyed(r, true, false, at(1000));
        assert_eq!(a.phase(), Phase::Revealing);
        a.update(menu, r, at(1200));
        assert_eq!(a.phase(), Phase::Shown);
        a.update(menu, r, at(3000));
        assert_eq!(a.phase(), Phase::Shown, "the open menu holds it");
        a.update(Signals::default(), r, at(3000));
        a.update(Signals::default(), r, at(3500));
        assert_eq!(a.phase(), Phase::Hiding, "half a second after it closed");
    }

    #[test]
    fn alt_tab_reveals_and_holds_until_the_switcher_closes() {
        let r = rules();
        let switcher = Signals {
            switcher: true,
            ..Signals::default()
        };
        let mut a = hidden_bar(r);
        a.keyed(r, false, false, at(1000));
        assert_eq!(a.phase(), Phase::Revealing);
        a.update(switcher, r, at(1200));
        a.update(switcher, r, at(4000));
        assert_eq!(a.phase(), Phase::Shown, "the panel holds it");
        a.update(Signals::default(), r, at(4000));
        a.update(Signals::default(), r, at(4500));
        assert_eq!(a.phase(), Phase::Hiding);
        let mut a = hidden_bar(r);
        a.update(switcher, r, at(1000));
        assert_eq!(a.phase(), Phase::Revealing);
    }

    #[test]
    fn the_keys_trigger_off_leaves_the_bar_hidden() {
        let r = Rules {
            keys: false,
            ..rules()
        };
        let mut a = hidden_bar(r);
        a.keyed(r, true, false, at(1000));
        a.keyed(r, false, false, at(1000));
        a.update(
            Signals {
                switcher: true,
                ..Signals::default()
            },
            r,
            at(1100),
        );
        assert_eq!(a.phase(), Phase::Hidden);
    }

    #[test]
    fn attention_reveals_and_holds_until_it_ends() {
        let r = rules();
        let asks = Signals {
            attention: true,
            ..Signals::default()
        };
        let mut a = hidden_bar(r);
        a.update(asks, r, at(1000));
        assert_eq!(a.phase(), Phase::Revealing);
        a.update(asks, r, at(1200));
        a.update(asks, r, at(10_000));
        assert_eq!(a.phase(), Phase::Shown, "held while the window asks");
        a.update(Signals::default(), r, at(10_000));
        a.update(Signals::default(), r, at(10_500));
        assert_eq!(
            a.phase(),
            Phase::Hiding,
            "half a second after it was answered"
        );
    }

    #[test]
    fn the_attention_trigger_off_leaves_the_bar_hidden() {
        let r = Rules {
            attention: false,
            ..rules()
        };
        let mut a = hidden_bar(r);
        a.update(
            Signals {
                attention: true,
                ..Signals::default()
            },
            r,
            at(1000),
        );
        assert_eq!(a.phase(), Phase::Hidden);
    }

    #[test]
    fn each_hold_keeps_a_shown_bar_shown_until_it_lets_go() {
        let r = rules();
        let holds = [
            Signals {
                over_bar: true,
                ..Signals::default()
            },
            Signals {
                owned_open: true,
                ..Signals::default()
            },
            Signals {
                dragging: true,
                ..Signals::default()
            },
            Signals {
                switcher: true,
                ..Signals::default()
            },
            Signals {
                attention: true,
                ..Signals::default()
            },
        ];
        for s in holds {
            let mut a = AutoHide::new(at(0));
            a.update(s, r, at(0));
            a.update(s, r, at(5000));
            assert_eq!(a.phase(), Phase::Shown, "{s:?} holds it");
            a.update(Signals::default(), r, at(5000));
            a.update(Signals::default(), r, at(5499));
            assert_eq!(
                a.phase(),
                Phase::Shown,
                "{s:?}: the delay runs from the let-go"
            );
            a.update(Signals::default(), r, at(5500));
            assert_eq!(a.phase(), Phase::Hiding, "{s:?}");
        }
    }

    #[test]
    fn a_hold_does_not_bring_a_hidden_bar_back() {
        let r = rules();
        for s in [
            Signals {
                over_bar: true,
                ..Signals::default()
            },
            Signals {
                owned_open: true,
                ..Signals::default()
            },
            Signals {
                dragging: true,
                ..Signals::default()
            },
        ] {
            let mut a = hidden_bar(r);
            a.update(s, r, at(1000));
            a.update(s, r, at(2000));
            assert_eq!(a.phase(), Phase::Hidden, "{s:?}");
        }
    }

    #[test]
    fn the_pointer_catching_a_hiding_bar_turns_it_back_from_where_it_is() {
        let r = rules();
        let mut a = AutoHide::new(at(0));
        a.update(Signals::default(), r, at(0));
        a.update(Signals::default(), r, at(500));
        a.update(
            Signals {
                over_bar: true,
                ..Signals::default()
            },
            r,
            at(600),
        );
        assert_eq!(a.phase(), Phase::Revealing);
        assert!(near(a.hidden(at(600)), 0.5), "it turns where it is");
        assert!(near(a.hidden(at(650)), 0.25));
        assert_eq!(a.wake(r, at(600)), Some(at(700)), "half a slide back");
        a.update(
            Signals {
                over_bar: true,
                ..Signals::default()
            },
            r,
            at(700),
        );
        assert_eq!(a.phase(), Phase::Shown);
    }

    #[test]
    fn over_a_fullscreen_window_only_the_start_key_reveals() {
        let r = rules();
        let all = Signals {
            on_line: true,
            switcher: true,
            attention: true,
            fullscreen: true,
            ..Signals::default()
        };
        let mut a = hidden_bar(r);
        a.update(all, r, at(1000));
        a.update(all, r, at(1400));
        a.pressed(r, true, at(1400));
        a.keyed(r, false, true, at(1400));
        assert_eq!(
            a.phase(),
            Phase::Hidden,
            "the line, attention, Alt+Tab and a press do nothing"
        );
        assert_eq!(a.wake(r, at(1400)), None, "and nothing is due");
        a.keyed(r, true, true, at(1500));
        assert_eq!(a.phase(), Phase::Revealing, "the Start key does");
    }

    #[test]
    fn a_new_hide_time_mid_slide_keeps_the_bar_where_it_is() {
        let r = rules();
        let mut a = AutoHide::new(at(0));
        a.update(Signals::default(), r, at(0));
        a.update(Signals::default(), r, at(500));
        let slow = Rules {
            slide_ms: 1000,
            ..r
        };
        a.update(Signals::default(), slow, at(600));
        assert!(near(a.hidden(at(600)), 0.5), "no jump");
        assert!(near(a.hidden(at(850)), 0.75), "the rest at the new pace");
        assert_eq!(a.wake(slow, at(600)), Some(at(1100)));
    }
}
