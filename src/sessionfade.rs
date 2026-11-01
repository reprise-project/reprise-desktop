//! The screen fading in from black at session start and out to black at its end (`[effects]
//! session_fade`): a black rectangle over each output's screen frames, so no capture shows it.

use std::time::{Duration, Instant};

use smithay::{
    backend::renderer::{
        element::{Id, Kind, solid::SolidColorRenderElement},
        utils::CommitCounter,
    },
    output::Output,
    reexports::calloop::timer::{TimeoutAction, Timer},
    utils::Rectangle,
};

use crate::compositor::State;

/// How long a fade out's action waits past its end, so the black frame reaches the screen first.
const LAST_FRAME: Duration = Duration::from_millis(50);
/// How long a starting output stays black waiting for its wallpaper before it fades in anyway.
const WAIT_CAP: Duration = Duration::from_secs(2);
/// The most a fade in moves on between two of its frames on screen, so an output whose frames
/// stall (its own or another output's modeset) resumes the fade where the screen left it.
const STEP: Duration = Duration::from_millis(50);
/// How long a fade in stays black after a modeset's first frame is on screen: the kernel
/// reports that flip done while a monitor relocking to the new signal may still show nothing.
const SETTLE: Duration = Duration::from_secs(1);

/// The fade out's black from start to end, linear in alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Ramp {
    started: Instant,
    ms: u32,
}

impl Ramp {
    fn length(&self) -> Duration {
        Duration::from_millis(self.ms.into())
    }

    fn done(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.started) >= self.length()
    }

    fn alpha(&self, now: Instant) -> f32 {
        let t = if self.done(now) {
            1.0
        } else {
            now.saturating_duration_since(self.started).as_secs_f64() * 1000.0 / f64::from(self.ms)
        };
        t as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum In {
    /// Black from this instant until the output's first full frame, then a fade of this many ms.
    Waiting(Instant, u32),
    /// The full frame went out under the black: the fade starts once a frame is on screen and
    /// the screen has settled after its last modeset.
    Drawn(u32),
    /// A fade of `ms`, `shown` of it on screen by the frame presented at `last`.
    Fading {
        ms: u32,
        shown: Duration,
        last: Instant,
    },
}

impl In {
    /// How much of a fade in the screen shows by `now`.
    fn shown(shown: Duration, last: Instant, now: Instant) -> Duration {
        shown + now.saturating_duration_since(last).min(STEP)
    }
}

/// When each step of an output's fade in happened, for the log line once it has faded in.
#[derive(Clone, Debug, PartialEq)]
pub struct Timeline {
    armed: Instant,
    /// When its wallpaper was on it, and whether the wait ran out instead.
    ready: Option<(Instant, bool)>,
    modesets: Vec<Instant>,
    first_flip: Option<Instant>,
    started: Option<Instant>,
    ended: Option<Instant>,
}

impl std::fmt::Display for Timeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ms = |t: Instant| t.saturating_duration_since(self.armed).as_millis();
        let at = |t: Option<Instant>| t.map_or("-".to_string(), |t| format!("{} ms", ms(t)));
        let modesets: Vec<String> = self
            .modesets
            .iter()
            .map(|t| format!("{}", ms(*t)))
            .collect();
        write!(
            f,
            "wallpaper {}{}, modesets at [{}] ms, first flip {}, fade {} to {}",
            at(self.ready.map(|(t, _)| t)),
            if self.ready.is_some_and(|(_, capped)| capped) {
                " (gave up waiting)"
            } else {
                ""
            },
            modesets.join(", "),
            at(self.first_flip),
            at(self.started),
            at(self.ended),
        )
    }
}

/// One starting output's fade in.
struct Entry<K> {
    key: K,
    state: In,
    /// A frame carrying a modeset went out and has not reached the screen yet.
    modeset: bool,
    /// The earliest a presented frame starts the fade: the settle after the last modeset.
    settled: Option<Instant>,
    log: Timeline,
}

/// What an output's frame asks of the loop after it is drawn or presented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    Idle,
    /// The fade moves on: draw the next frame.
    Frame,
    /// The fade waits: draw again after this long at the latest.
    After(Duration),
}

/// The fades over outputs keyed by `K`; a fade out runs `T` once it is black.
pub struct Fade<K, T> {
    /// The fade in's length until the first starting output has faded in: an output first
    /// drawn after that does not fade.
    opening: Option<u32>,
    ins: Vec<Entry<K>>,
    /// Outputs that faded in since the last `take_faded`.
    faded: Vec<(K, Timeline)>,
    out: Option<(Ramp, Option<T>)>,
}

impl<K: PartialEq + Clone, T> Fade<K, T> {
    /// `in_ms` 0 is no fade in.
    pub fn new(in_ms: u32) -> Self {
        Fade {
            opening: (in_ms > 0).then_some(in_ms),
            ins: Vec::new(),
            faded: Vec::new(),
            out: None,
        }
    }

    fn entry(&mut self, key: &K) -> Option<&mut Entry<K>> {
        self.ins.iter_mut().find(|e| e.key == *key)
    }

    /// The black's alpha over `key`'s frame at `now`. `ready`: the output's wallpaper is on it,
    /// so the fade in may start.
    pub fn frame(&mut self, key: &K, ready: bool, now: Instant) -> (f32, Next) {
        let mut next = Next::Idle;
        let i = match self.ins.iter().position(|e| e.key == *key) {
            Some(i) => Some(i),
            None => self.opening.map(|ms| {
                self.ins.push(Entry {
                    key: key.clone(),
                    state: In::Waiting(now, ms),
                    modeset: false,
                    settled: None,
                    log: Timeline {
                        armed: now,
                        ready: None,
                        modesets: Vec::new(),
                        first_flip: None,
                        started: None,
                        ended: None,
                    },
                });
                next = Next::After(WAIT_CAP);
                self.ins.len() - 1
            }),
        };
        let mut shown = 0.0;
        if let Some(i) = i {
            let e = &mut self.ins[i];
            if let In::Waiting(since, ms) = e.state {
                let capped = now.saturating_duration_since(since) >= WAIT_CAP;
                if ready || capped {
                    e.state = In::Drawn(ms);
                    e.log.ready = Some((now, !ready));
                }
            }
            match e.state {
                In::Waiting(..) | In::Drawn(_) => shown = 1.0,
                In::Fading {
                    ms,
                    shown: by,
                    last,
                } => {
                    let t = In::shown(by, last, now).as_secs_f64() * 1000.0 / f64::from(ms);
                    if t >= 1.0 {
                        let mut e = self.ins.remove(i);
                        e.log.ended = Some(now);
                        self.faded.push((e.key, e.log));
                        self.opening = None;
                    } else {
                        shown = (1.0 - t) as f32;
                        next = Next::Frame;
                    }
                }
            }
        }
        if let Some((r, _)) = &self.out {
            shown = f32::max(shown, r.alpha(now));
            if !r.done(now) {
                next = Next::Frame;
            }
        }
        (shown, next)
    }

    /// A frame of `key`'s carrying a modeset went out at `now`: its fade in, even one under way,
    /// starts over once that frame is on screen and the screen has settled. None when `key`
    /// has no fade in; true when a fade under way went back to black.
    pub fn modeset(&mut self, key: &K, now: Instant) -> Option<bool> {
        let e = self.entry(key)?;
        e.modeset = true;
        e.log.modesets.push(now);
        match e.state {
            In::Fading { ms, .. } => {
                e.state = In::Drawn(ms);
                e.log.started = None;
                Some(true)
            }
            In::Waiting(..) | In::Drawn(_) => Some(false),
        }
    }

    /// A frame of `key`'s reached the screen at `now`: a fade in waiting for that starts, a
    /// running one moves on. `Next::Frame` when one started, `Next::After` while the screen
    /// settles after a modeset; the caller draws the next frame then.
    pub fn presented(&mut self, key: &K, now: Instant) -> Next {
        let Some(e) = self.entry(key) else {
            return Next::Idle;
        };
        e.log.first_flip.get_or_insert(now);
        if std::mem::take(&mut e.modeset) {
            e.settled = Some(now + SETTLE);
        }
        match e.state {
            In::Drawn(ms) => match e.settled.filter(|t| *t > now) {
                Some(t) => Next::After(t - now),
                None => {
                    e.state = In::Fading {
                        ms,
                        shown: Duration::ZERO,
                        last: now,
                    };
                    e.log.started = Some(now);
                    Next::Frame
                }
            },
            In::Fading { ms, shown, last } => {
                e.state = In::Fading {
                    ms,
                    shown: In::shown(shown, last, now),
                    last: now,
                };
                Next::Idle
            }
            In::Waiting(..) => Next::Idle,
        }
    }

    /// The outputs that faded in since the last call, with their timelines.
    pub fn take_faded(&mut self) -> Vec<(K, Timeline)> {
        std::mem::take(&mut self.faded)
    }

    /// Fade every output to black over `ms`, then hand `then` back from `ended`. Returns `then`
    /// to run at once when `ms` is 0 or a fade out is already under way.
    pub fn fade_out(&mut self, ms: u32, then: T, now: Instant) -> Option<T> {
        if ms == 0 || self.out.is_some() {
            return Some(then);
        }
        self.out = Some((Ramp { started: now, ms }, Some(then)));
        None
    }

    /// The fade out's action once its black is on screen; the black stays.
    pub fn ended(&mut self, now: Instant) -> Option<T> {
        let (r, then) = self.out.as_mut()?;
        if now.saturating_duration_since(r.started) < r.length() + LAST_FRAME {
            return None;
        }
        then.take()
    }

    /// How long until `ended` hands the action back.
    pub fn out_left(&self, now: Instant) -> Option<Duration> {
        let (r, then) = self.out.as_ref()?;
        then.as_ref()?;
        Some((r.started + r.length() + LAST_FRAME).saturating_duration_since(now))
    }

    /// Drop a fade out and its action: the desktop shows again at once.
    pub fn cancel_out(&mut self) -> bool {
        self.out.take().is_some()
    }
}

pub type Then = Box<dyn FnOnce(&mut State)>;

/// The compositor's fades and the black rectangle drawn for each output.
pub struct SessionFade {
    fade: Fade<Output, Then>,
    drawn: Vec<(Output, Id, CommitCounter, f32)>,
}

impl SessionFade {
    pub fn new(in_ms: u32) -> SessionFade {
        SessionFade {
            fade: Fade::new(in_ms),
            drawn: Vec::new(),
        }
    }

    #[cfg(test)]
    pub fn fading_out(&self) -> bool {
        self.fade.out_left(Instant::now()).is_some()
    }
}

/// The black over `output`'s next screen frame, if any, and whether the frame after it is due
/// at once (`Next::Frame`).
pub fn element(state: &mut State, output: &Output) -> (Option<SolidColorRenderElement>, Next) {
    let ready = state.wallpaper_ready(output);
    let now = Instant::now();
    let (alpha, next) = state.session_fade.fade.frame(output, ready, now);
    if let Next::After(wait) = next {
        redraw_after(state, output, wait);
    }
    log_faded(state);
    let f = &mut state.session_fade;
    let at = f.drawn.iter().position(|(o, ..)| o == output);
    if alpha <= 0.0 {
        if let Some(i) = at {
            f.drawn.remove(i);
        }
        return (None, next);
    }
    let i = at.unwrap_or_else(|| {
        f.drawn
            .push((output.clone(), Id::new(), CommitCounter::default(), 0.0));
        f.drawn.len() - 1
    });
    let (_, id, commit, last) = &mut f.drawn[i];
    if *last != alpha {
        commit.increment();
        *last = alpha;
    }
    let Some(geo) = reprisescene::output_rect(output) else {
        return (None, next);
    };
    let size = geo
        .size
        .to_f64()
        .to_physical_precise_ceil(output.current_scale().fractional_scale());
    let e = SolidColorRenderElement::new(
        id.clone(),
        Rectangle::from_size(size),
        *commit,
        [0.0, 0.0, 0.0, alpha],
        Kind::Unspecified,
    );
    (Some(e), next)
}

fn redraw_after(state: &mut State, output: &Output, wait: Duration) {
    let o = output.clone();
    let armed = state
        .handle
        .insert_source(Timer::from_duration(wait), move |_, _, state| {
            crate::backend::schedule(state, &o);
            TimeoutAction::Drop
        });
    if let Err(e) = armed {
        log::warn!("session fade: no timer: {}", e.error);
    }
}

fn log_faded(state: &mut State) {
    for (output, timeline) in state.session_fade.fade.take_faded() {
        log::info!("session fade: {} faded in: {timeline}", output.name());
    }
}

/// A frame of `output`'s reached the screen (`backend::flipped`).
pub fn presented(state: &mut State, output: &Output) {
    match state.session_fade.fade.presented(output, Instant::now()) {
        Next::Idle => {}
        Next::Frame => {
            log::debug!("session fade: {} fades in", output.name());
            crate::backend::schedule(state, output);
        }
        Next::After(wait) => redraw_after(state, output, wait),
    }
}

/// A frame of `output`'s was rendered: an `empty` one changed nothing, so the screen already
/// shows it.
pub fn rendered(state: &mut State, output: &Output, empty: bool) {
    if empty {
        presented(state, output);
    }
}

/// A modeset on `output` went out: a link retrain, or a frame carrying a new mode or the first.
pub fn modeset(state: &mut State, output: &Output) {
    let Some(reverted) = state.session_fade.fade.modeset(output, Instant::now()) else {
        return;
    };
    if reverted {
        log::info!(
            "session fade: {} modeset during its fade in; black until it settles",
            output.name()
        );
    }
    // A retrain is followed by no frame of its own; the settle starts at the next one shown.
    crate::backend::schedule(state, output);
}

/// Fade every output to black over `[effects] session_fade_out_ms`, then run `then`; at once
/// with the fade off.
pub fn out(state: &mut State, then: Then) {
    let (_, ms) = state.config.effects.session_fade_ms();
    let now = Instant::now();
    if let Some(then) = state.session_fade.fade.fade_out(ms, then, now) {
        then(state);
        return;
    }
    log::debug!("session fade: out over {ms} ms");
    let wait = state.session_fade.fade.out_left(now).unwrap_or_default();
    let armed = state
        .handle
        .insert_source(Timer::from_duration(wait), |_, _, state| {
            let now = Instant::now();
            if let Some(then) = state.session_fade.fade.ended(now) {
                log::debug!("session fade: out done");
                then(state);
                return TimeoutAction::Drop;
            }
            state
                .session_fade
                .fade
                .out_left(now)
                .map_or(TimeoutAction::Drop, TimeoutAction::ToDuration)
        });
    if let Err(e) = armed {
        log::warn!("session fade: no timer: {}", e.error);
        if let Some((_, Some(then))) = state.session_fade.fade.out.take() {
            then(state);
        }
        return;
    }
    state.redraw_all();
}

/// Bring the desktop back after a session end that did not happen.
pub fn restore(state: &mut State) {
    if state.session_fade.fade.cancel_out() {
        log::debug!("session fade: out cancelled");
        state.redraw_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Present `key`'s frames every 16 ms over (`from`, `to`]; the black at `to`.
    fn frames<T>(f: &mut Fade<u8, T>, key: u8, from: Instant, to: Instant) -> f32 {
        let mut at = from;
        while at + ms(16) <= to {
            at += ms(16);
            f.frame(&key, true, at);
            f.presented(&key, at);
        }
        f.frame(&key, true, to).0
    }

    #[test]
    fn a_starting_output_stays_black_until_ready_then_fades_in() {
        let t0 = Instant::now();
        let mut f: Fade<u8, ()> = Fade::new(600);
        assert_eq!(f.frame(&1, false, t0), (1.0, Next::After(WAIT_CAP)));
        assert_eq!(f.frame(&1, false, t0 + ms(500)), (1.0, Next::Idle));
        assert_eq!(f.frame(&1, true, t0 + ms(1000)), (1.0, Next::Idle));
        assert_eq!(f.presented(&1, t0 + ms(1000)), Next::Frame);
        assert_eq!(f.frame(&1, true, t0 + ms(1016)).1, Next::Frame);
        let half = frames(&mut f, 1, t0 + ms(1000), t0 + ms(1300));
        assert!((half - 0.5).abs() < 1e-3, "{half}");
        assert_eq!(frames(&mut f, 1, t0 + ms(1300), t0 + ms(1600)), 0.0);
        assert_eq!(f.frame(&1, true, t0 + ms(1616)), (0.0, Next::Idle));
        assert_eq!(
            f.frame(&2, true, t0 + ms(1700)),
            (0.0, Next::Idle),
            "a later output shows at once"
        );
    }

    #[test]
    fn a_wallpaper_that_never_lands_fades_in_after_the_cap() {
        let t0 = Instant::now();
        let mut f: Fade<u8, ()> = Fade::new(600);
        f.frame(&1, false, t0);
        assert_eq!(f.frame(&1, false, t0 + WAIT_CAP), (1.0, Next::Idle));
        assert_eq!(f.presented(&1, t0 + WAIT_CAP), Next::Frame);
        assert_eq!(
            frames(&mut f, 1, t0 + WAIT_CAP, t0 + WAIT_CAP + ms(608)),
            0.0
        );
    }

    #[test]
    fn the_fade_in_starts_when_its_first_frame_reaches_the_screen() {
        let t0 = Instant::now();
        let mut f: Fade<u8, ()> = Fade::new(600);
        assert_eq!(f.frame(&1, true, t0).0, 1.0);
        assert_eq!(f.presented(&2, t0), Next::Idle, "another output's frame");
        let modeset = ms(700);
        assert_eq!(
            f.frame(&1, true, t0 + modeset).0,
            1.0,
            "still black while the first modeset is out"
        );
        assert_eq!(f.presented(&1, t0 + modeset), Next::Frame);
        assert_eq!(f.presented(&1, t0 + modeset), Next::Idle, "starts once");
        let half = frames(&mut f, 1, t0 + modeset, t0 + modeset + ms(300));
        assert!((half - 0.5).abs() < 1e-3, "{half}");
    }

    #[test]
    fn a_stalled_output_resumes_its_fade_in_where_the_screen_left_it() {
        let t0 = Instant::now();
        let mut f: Fade<u8, ()> = Fade::new(600);
        f.frame(&1, true, t0);
        f.presented(&1, t0);
        let before = frames(&mut f, 1, t0, t0 + ms(160));
        assert!((before - (1.0 - 160.0 / 600.0)).abs() < 1e-3, "{before}");
        let stall = ms(450);
        let (after, next) = f.frame(&1, true, t0 + ms(160) + stall);
        assert!(
            (after - (1.0 - 210.0 / 600.0)).abs() < 1e-3,
            "one step on after the stall: {after}"
        );
        assert_eq!(next, Next::Frame);
    }

    #[test]
    fn every_starting_output_fades_in_on_its_own() {
        let t0 = Instant::now();
        let mut f: Fade<u8, ()> = Fade::new(600);
        f.frame(&1, true, t0);
        f.frame(&2, false, t0);
        f.presented(&1, t0);
        f.presented(&2, t0);
        assert_eq!(frames(&mut f, 1, t0, t0 + ms(608)), 0.0);
        assert_eq!(f.frame(&2, false, t0 + ms(700)).0, 1.0, "still waiting");
        assert_eq!(f.frame(&2, true, t0 + ms(800)), (1.0, Next::Idle));
        assert_eq!(f.presented(&2, t0 + ms(816)), Next::Frame);
    }

    #[test]
    fn zero_ms_is_no_fade_in() {
        let mut f: Fade<u8, ()> = Fade::new(0);
        assert_eq!(f.frame(&1, false, Instant::now()), (0.0, Next::Idle));
    }

    #[test]
    fn the_action_runs_only_once_the_fade_out_is_black_and_the_black_stays() {
        let t0 = Instant::now();
        let mut f: Fade<u8, &str> = Fade::new(0);
        assert_eq!(f.fade_out(400, "stop", t0), None);
        let (a, next) = f.frame(&1, true, t0 + ms(100));
        assert!((a - 0.25).abs() < 1e-3, "{a}");
        assert_eq!(next, Next::Frame);
        assert_eq!(f.ended(t0 + ms(400)), None, "the black frame goes up first");
        assert_eq!(f.out_left(t0 + ms(400)), Some(LAST_FRAME));
        assert_eq!(f.frame(&1, true, t0 + ms(400)), (1.0, Next::Idle));
        assert_eq!(f.ended(t0 + ms(400) + LAST_FRAME), Some("stop"));
        assert_eq!(f.ended(t0 + ms(900)), None, "once");
        assert_eq!(f.out_left(t0 + ms(900)), None);
        assert_eq!(f.frame(&1, true, t0 + ms(2000)).0, 1.0);
    }

    #[test]
    fn zero_ms_or_a_second_end_runs_at_once() {
        let t0 = Instant::now();
        let mut f: Fade<u8, &str> = Fade::new(0);
        assert_eq!(f.fade_out(0, "stop", t0), Some("stop"));
        assert_eq!(f.frame(&1, true, t0).0, 0.0, "no black");
        assert_eq!(f.fade_out(400, "reboot", t0), None);
        assert_eq!(f.fade_out(400, "signal", t0 + ms(100)), Some("signal"));
        assert_eq!(f.ended(t0 + ms(450)), Some("reboot"));
    }

    #[test]
    fn a_cancelled_fade_out_shows_the_desktop_at_once_and_never_acts() {
        let t0 = Instant::now();
        let mut f: Fade<u8, &str> = Fade::new(0);
        f.fade_out(400, "reboot", t0);
        assert!(f.cancel_out());
        assert_eq!(f.frame(&1, true, t0 + ms(200)), (0.0, Next::Idle));
        assert_eq!(f.ended(t0 + ms(500)), None);
        assert!(!f.cancel_out());
    }

    #[test]
    fn a_fade_out_over_a_fade_in_keeps_the_darker() {
        let t0 = Instant::now();
        let mut f: Fade<u8, &str> = Fade::new(400);
        f.frame(&1, true, t0);
        f.presented(&1, t0);
        f.fade_out(400, "stop", t0 + ms(100));
        let a = frames(&mut f, 1, t0, t0 + ms(200));
        assert!((a - 0.5).abs() < 1e-3, "the fade in's half: {a}");
        let (a, _) = f.frame(&1, true, t0 + ms(400));
        assert!((a - 0.75).abs() < 1e-3, "the fade out's: {a}");
    }

    #[test]
    fn the_fade_in_waits_a_settle_after_its_modeset_reaches_the_screen() {
        let t0 = Instant::now();
        let mut f: Fade<u8, ()> = Fade::new(600);
        f.frame(&1, true, t0);
        assert_eq!(f.modeset(&1, t0), Some(false));
        let flip = t0 + ms(700);
        assert_eq!(f.presented(&1, flip), Next::After(SETTLE));
        assert_eq!(f.frame(&1, true, flip + ms(500)), (1.0, Next::Idle));
        assert_eq!(
            f.presented(&1, flip + ms(500)),
            Next::After(SETTLE - ms(500))
        );
        assert_eq!(f.presented(&1, flip + SETTLE), Next::Frame);
        let half = frames(&mut f, 1, flip + SETTLE, flip + SETTLE + ms(300));
        assert!((half - 0.5).abs() < 1e-3, "{half}");
    }

    #[test]
    fn a_modeset_mid_fade_goes_back_to_black_and_fades_the_whole_length() {
        let t0 = Instant::now();
        let mut f: Fade<u8, ()> = Fade::new(600);
        f.frame(&1, true, t0);
        f.presented(&1, t0);
        frames(&mut f, 1, t0, t0 + ms(208));
        assert_eq!(f.modeset(&1, t0 + ms(208)), Some(true));
        assert_eq!(f.frame(&1, true, t0 + ms(210)), (1.0, Next::Idle));
        let flip = t0 + ms(400);
        assert_eq!(f.presented(&1, flip), Next::After(SETTLE));
        assert_eq!(f.presented(&1, flip + SETTLE), Next::Frame);
        let half = frames(&mut f, 1, flip + SETTLE, flip + SETTLE + ms(300));
        assert!((half - 0.5).abs() < 1e-3, "{half}");
    }

    #[test]
    fn a_modeset_while_waiting_for_the_wallpaper_still_settles() {
        let t0 = Instant::now();
        let mut f: Fade<u8, ()> = Fade::new(600);
        f.frame(&1, false, t0);
        f.modeset(&1, t0);
        assert_eq!(
            f.presented(&1, t0 + ms(100)),
            Next::Idle,
            "no wallpaper yet"
        );
        assert_eq!(f.frame(&1, true, t0 + ms(400)), (1.0, Next::Idle));
        assert_eq!(f.presented(&1, t0 + ms(400)), Next::After(ms(700)));
        assert_eq!(f.presented(&1, t0 + ms(100) + SETTLE), Next::Frame);
    }

    #[test]
    fn a_wallpaper_ready_after_the_settle_fades_in_at_once() {
        let t0 = Instant::now();
        let mut f: Fade<u8, ()> = Fade::new(600);
        f.frame(&1, false, t0);
        f.modeset(&1, t0);
        f.presented(&1, t0 + ms(100));
        let late = t0 + ms(1500);
        assert_eq!(f.frame(&1, true, late), (1.0, Next::Idle));
        assert_eq!(f.presented(&1, late), Next::Frame);
    }

    #[test]
    fn an_output_added_again_during_the_opening_fades_in_whole() {
        let t0 = Instant::now();
        let mut f: Fade<u8, ()> = Fade::new(600);
        f.frame(&1, true, t0);
        f.modeset(&1, t0);
        f.presented(&1, t0 + ms(100));
        let again = t0 + ms(800);
        assert_eq!(f.frame(&2, true, again), (1.0, Next::After(WAIT_CAP)));
        f.modeset(&2, again);
        assert_eq!(f.presented(&2, again + ms(100)), Next::After(SETTLE));
        let start = again + ms(100) + SETTLE;
        assert_eq!(f.presented(&2, start), Next::Frame);
        assert_eq!(frames(&mut f, 2, start, start + ms(608)), 0.0);
        assert_eq!(
            f.frame(&3, true, start + ms(700)),
            (0.0, Next::Idle),
            "an output first drawn after the opening shows at once"
        );
    }

    #[test]
    fn a_faded_output_hands_over_its_timeline_once() {
        let t0 = Instant::now();
        let mut f: Fade<u8, ()> = Fade::new(600);
        f.frame(&1, false, t0);
        f.modeset(&1, t0 + ms(5));
        f.presented(&1, t0 + ms(40));
        f.frame(&1, true, t0 + ms(200));
        let start = t0 + ms(40) + SETTLE;
        f.presented(&1, start);
        frames(&mut f, 1, start, start + ms(608));
        let faded = f.take_faded();
        assert_eq!(faded.len(), 1);
        let (key, line) = &faded[0];
        assert_eq!(*key, 1);
        assert_eq!(
            line.to_string(),
            "wallpaper 200 ms, modesets at [5] ms, first flip 40 ms, fade 1040 ms to 1648 ms"
        );
        assert!(f.take_faded().is_empty());
    }
}
