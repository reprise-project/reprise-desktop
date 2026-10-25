//! Windows that want attention: a task button flashes for `[taskbar] attention_flash`, then stays
//! lit until its window is focused. One timer runs while any button flashes, redrawing the bars.

use std::time::{Duration, Instant};

use reprisekit::taskbar::Attention;
use smithay::reexports::calloop::{
    RegistrationToken,
    timer::{TimeoutAction, Timer},
};

use crate::compositor::State;
use crate::window::WindowId;

/// The flash timer and when it fires.
#[derive(Default)]
pub struct Flash {
    timer: Option<(RegistrationToken, Instant)>,
}

impl State {
    /// `w` asks for attention: its button starts flashing, again if it already asked.
    pub fn ask_attention(&mut self, w: WindowId) {
        let now = Instant::now();
        let Some(win) = self.win_mut(w) else {
            return;
        };
        win.attention = Some(now);
        arm(self, now);
    }

    /// `w` no longer wants attention; true if it did. An X11 window loses its demands-attention
    /// state too.
    pub fn clear_attention(&mut self, w: WindowId) -> bool {
        let Some(win) = self.win_mut(w) else {
            return false;
        };
        let had = win.attention.take().is_some();
        if let Some(x) = win.x11()
            && x.demands_attention()
            && let Err(e) = x.set_demands_attention(false)
        {
            log::debug!("clear demands-attention of {w:?}: {e}");
        }
        had
    }

    /// An X11 window's urgency (WM_HINTS UrgencyHint or `_NET_WM_STATE_DEMANDS_ATTENTION`) is
    /// `urgent` now: becoming urgent asks for attention unless the window has focus. Withdrawing
    /// it leaves the button lit until the user focuses the window.
    pub fn x11_urgency(&mut self, w: WindowId, urgent: bool) {
        let focused = self.focused == Some(w);
        let Some(win) = self.win_mut(w) else {
            return;
        };
        let rose = urgent && !win.urgent;
        win.urgent = urgent;
        if rose && win.mapped && !focused {
            log::info!("{w:?} ({:?}) is urgent", win.app_id());
            self.ask_attention(w);
            self.redraw_taskbars();
            self.redraw_all();
        }
    }
}

pub fn flash_for(state: &State) -> Duration {
    Duration::from_secs(state.config.taskbar.attention_flash.into())
}

/// The phase of a window that began asking `since` (None: it does not), at `now`.
pub fn phase(since: Option<Instant>, now: Instant, flash_for: Duration) -> Attention {
    since.map_or(Attention::None, |t| {
        Attention::after(now.saturating_duration_since(t), flash_for)
    })
}

/// When the next of `asks` (each window's ask) changes phase, seen at `now`; None once none flashes.
fn next_change(
    asks: impl Iterator<Item = Instant>,
    now: Instant,
    flash_for: Duration,
) -> Option<Instant> {
    asks.filter_map(|t| {
        Attention::next_change(now.saturating_duration_since(t), flash_for).map(|d| t + d)
    })
    .min()
}

/// Sets the timer for the next phase change of any flashing button, or drops it when none flashes.
fn arm(state: &mut State, now: Instant) {
    let asks = state.windows.iter().filter_map(|w| w.attention);
    let due = next_change(asks, now, flash_for(state));
    if state.attention_flash.timer.as_ref().map(|&(_, at)| at) == due {
        return;
    }
    if let Some((t, _)) = state.attention_flash.timer.take() {
        state.handle.remove(t);
    }
    let Some(at) = due else {
        return;
    };
    let token = state
        .handle
        .insert_source(Timer::from_deadline(at), |_, _, state| {
            state.attention_flash.timer = None;
            state.redraw_taskbars();
            arm(state, Instant::now());
            TimeoutAction::Drop
        });
    match token {
        Ok(t) => state.attention_flash.timer = Some((t, at)),
        Err(e) => log::warn!("attention flash timer: {}", e.error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Harness;

    const FOR: Duration = Duration::from_secs(30);

    #[test]
    fn a_window_that_never_asked_shows_nothing() {
        assert_eq!(phase(None, Instant::now(), FOR), Attention::None);
    }

    #[test]
    fn the_timer_follows_the_soonest_flashing_window_and_stops_once_all_are_steady() {
        let now = Instant::now();
        let ms = Duration::from_millis;
        let asks = [now - ms(200), now - ms(1300)];
        assert_eq!(next_change(asks.into_iter(), now, FOR), Some(now + ms(200)));
        let steady = [now - FOR, now - 2 * FOR];
        assert_eq!(next_change(steady.into_iter(), now, FOR), None);
        assert_eq!(
            next_change(asks.into_iter(), now, Duration::ZERO),
            None,
            "no flash time runs no timer"
        );
    }

    fn two_windows() -> (Harness, crate::harness::Toplevel, crate::harness::Toplevel) {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let a = h.map_toplevel(&mut c, "test.a", 300, 200);
        let b = h.map_toplevel(&mut c, "test.b", 300, 200);
        h.dispatch();
        assert_eq!(h.state.focused, Some(b.id));
        (h, a, b)
    }

    #[test]
    fn an_ask_flashes_until_its_time_then_stays_lit_and_focus_clears_it() {
        let (mut h, a, _) = two_windows();
        h.state.ask_attention(a.id);
        let now = |h: &Harness| {
            let asked = h.state.win(a.id).unwrap().attention;
            phase(asked, Instant::now(), flash_for(&h.state))
        };
        assert_eq!(now(&h), Attention::FlashOn);
        assert!(
            h.state.attention_flash.timer.is_some(),
            "a flash runs the timer"
        );

        let since = h.state.win(a.id).unwrap().attention.unwrap();
        h.state.win_mut(a.id).unwrap().attention = Some(since - FOR);
        arm(&mut h.state, Instant::now());
        assert_eq!(now(&h), Attention::Steady);
        assert!(
            h.state.attention_flash.timer.is_none(),
            "nothing flashes, no timer"
        );

        h.state.ask_attention(a.id);
        assert_eq!(now(&h), Attention::FlashOn, "asking again flashes again");

        h.state.focus_window(a.id);
        assert_eq!(now(&h), Attention::None);
        arm(&mut h.state, Instant::now());
        assert!(h.state.attention_flash.timer.is_none());
    }

    #[test]
    fn x11_urgency_asks_once_as_it_rises_and_its_withdrawal_keeps_the_button_lit() {
        let (mut h, a, b) = two_windows();
        h.state.x11_urgency(a.id, true);
        let asked = h.state.win(a.id).unwrap().attention;
        assert!(asked.is_some(), "urgency asks for attention");
        h.state.x11_urgency(a.id, true);
        assert_eq!(
            h.state.win(a.id).unwrap().attention,
            asked,
            "staying urgent asks no more"
        );
        h.state.x11_urgency(a.id, false);
        assert_eq!(
            h.state.win(a.id).unwrap().attention,
            asked,
            "withdrawn, it stays lit"
        );
        h.state.x11_urgency(b.id, true);
        assert!(
            h.state.win(b.id).unwrap().attention.is_none(),
            "the focused window does not ask"
        );
    }

    #[test]
    fn a_flash_time_of_zero_lights_the_button_at_once() {
        let (mut h, a, _) = two_windows();
        h.state.config.taskbar.attention_flash = 0;
        h.state.ask_attention(a.id);
        let asked = h.state.win(a.id).unwrap().attention;
        assert_eq!(
            phase(asked, Instant::now(), flash_for(&h.state)),
            Attention::Steady
        );
        assert!(h.state.attention_flash.timer.is_none());
    }
}
