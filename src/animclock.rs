//! Shell animations posed on the frame clock: each running one steps in every frame's `advance`
//! at that frame's time and asks the next frame on the outputs it moves on, until it settles.

use std::time::{Duration, Instant};

use smithay::reexports::calloop::{
    RegistrationToken,
    timer::{TimeoutAction, Timer},
};

use crate::compositor::State;

/// How long the running animations go without a frame before a timer steps them, so one still
/// ends while its outputs render nothing (switched off, away, or showing none of it). The harness
/// draws no frames, so in tests the timer stands in for them at about a frame's pace.
const FALLBACK: Duration = Duration::from_millis(if cfg!(test) { 16 } else { 250 });

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anim {
    /// Caption buttons and Start fading lit or dark (`hoverfade`).
    Hover,
    /// Task buttons sliding and floating in a drag (`taskslide`).
    TaskSlide,
    /// The tray arrow's run of hidden icons sliding out or back (`trayreveal`).
    TrayReveal,
    /// Menus fading or unrolling in (`menuview`).
    MenuOpen,
    /// The Start panel fading or unrolling in, and its pictures crossfading (`startpanelview`).
    StartPanel,
    /// Notification balloons sliding and Droplet beads in motion (`notifyview`).
    Notify,
    /// The wallpaper fading in over the picture before (`wallpaperview`).
    Wallpaper,
    /// The idle dim darkening the screens (`idle`).
    IdleDim,
    /// The greyed still under a Welcome panel fading in (`dialogview`).
    DialogFade,
}

impl Anim {
    /// Poses `self` at `now` and, while it moves, asks the next frame on the outputs it is on;
    /// whether it still moves.
    fn step(self, state: &mut State, now: Instant) -> bool {
        match self {
            Anim::Hover => state.hover_step(now),
            Anim::TaskSlide => state.slide_step(now),
            Anim::TrayReveal => crate::trayreveal::step(state, now),
            Anim::MenuOpen => state.menu_open_step(now),
            Anim::StartPanel => state.start_panel_step(now),
            Anim::Notify => crate::notifyview::motion::step(state, now),
            Anim::Wallpaper => state.wallpaper_fade_step(now),
            Anim::IdleDim => crate::idle::dim_step(state, now),
            Anim::DialogFade => state.dialog_fade_step(now),
        }
    }
}

#[derive(Default)]
pub struct AnimClock {
    running: Vec<Anim>,
    stepped: Option<Instant>,
    fallback: Option<RegistrationToken>,
    /// A step is queued for the loop's next turn.
    kicked: bool,
}

#[cfg(test)]
impl AnimClock {
    pub fn running(&self, a: Anim) -> bool {
        self.running.contains(&a)
    }
}

impl State {
    /// `a` steps on the loop's next turn, which asks its first frame, and then on every frame
    /// until it settles.
    pub fn animate(&mut self, a: Anim) {
        if !self.anim_clock.running.contains(&a) {
            self.anim_clock.running.push(a);
        }
        arm_fallback(self);
        if !self.anim_clock.kicked {
            self.anim_clock.kicked = true;
            self.insert_idle_woken(|state| {
                state.anim_clock.kicked = false;
                step(state, Instant::now());
            });
        }
    }
}

/// Steps every running animation at the frame's time: from `backend::advance`.
pub fn frame(state: &mut State) {
    step(state, Instant::now());
}

/// Steps every running animation at `now` and keeps those still moving.
pub fn step(state: &mut State, now: Instant) {
    if state.anim_clock.running.is_empty() {
        return;
    }
    state.anim_clock.stepped = Some(now);
    for a in std::mem::take(&mut state.anim_clock.running) {
        // A step may start its own or another animation again (`animate`).
        if a.step(state, now) && !state.anim_clock.running.contains(&a) {
            state.anim_clock.running.push(a);
        }
    }
}

fn arm_fallback(state: &mut State) {
    if state.anim_clock.fallback.is_some() {
        return;
    }
    let token = state
        .handle
        .insert_source(Timer::from_duration(FALLBACK), |_, _, state| {
            let now = Instant::now();
            let quiet =
                (state.anim_clock.stepped).map_or(FALLBACK, |t| now.saturating_duration_since(t));
            if quiet < FALLBACK && !state.anim_clock.running.is_empty() {
                return TimeoutAction::ToDuration(FALLBACK - quiet);
            }
            step(state, now);
            if state.anim_clock.running.is_empty() {
                state.anim_clock.fallback = None;
                TimeoutAction::Drop
            } else {
                TimeoutAction::ToDuration(FALLBACK)
            }
        });
    match token {
        Ok(t) => state.anim_clock.fallback = Some(t),
        Err(e) => log::warn!("animation fallback timer: {}", e.error),
    }
}

/// Asserts that `a` runs, that a frame at `from`, a time before its end, asks the next one on
/// exactly the outputs named `on`, and that a frame at `end` settles it, leaving nothing to ask on
/// the frame after. `from` is a time the test took, so a slow host cannot step past the end.
#[cfg(test)]
pub fn assert_steps_until(state: &mut State, a: Anim, on: &[&str], from: Instant, end: Instant) {
    assert!(state.anim_clock.running(a), "{a:?} runs");
    crate::backend::take_asks();
    step(state, from);
    assert_eq!(
        crate::backend::take_asked_outputs(),
        on,
        "{a:?} asks the next frame"
    );
    assert!(
        state.anim_clock.running(a),
        "{a:?} still runs before its end"
    );
    step(state, end);
    assert!(!state.anim_clock.running(a), "{a:?} settles at its end");
    crate::backend::take_asks();
    step(state, end + Duration::from_millis(1));
    assert_eq!(
        crate::backend::take_asks(),
        [],
        "a frame after its end asks none"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_running_animation_steps_on_the_fallback_when_no_frame_does() {
        let mut config = crate::config::Config::default();
        config.effects.hover_fade = true;
        let mut h = crate::harness::Harness::with_config(config);
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "test.app", 300, 200);
        h.state.win_mut(t.id).unwrap().hot = Some(reprisekit::frame::CaptionButton::Close);
        h.state.glow_to_hot(t.id);
        assert!(h.state.anim_clock.running(Anim::Hover));
        h.until("the fade settles with no frame drawn", |s| {
            !s.anim_clock.running(Anim::Hover)
        });
        assert_eq!(
            h.state.win(t.id).unwrap().glow[2].level(Instant::now()),
            1.0,
            "the close button's glow ends lit"
        );
    }
}
