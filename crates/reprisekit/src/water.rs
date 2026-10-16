//! Desktop water's model: the `[wallpaper] water` value and presets, the splashes the desktop's events
//! make in the height field, the estimate of motion left in it, the step rate and clock, and Rain's
//! drops. Field coordinates are texels of a field 1/[`FIELD_DIV`] of the output's physical size; heights
//! run -1..1, negative pushing the water down. `reprisefx::water` steps the field itself.

use crate::saver::Rng;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Water {
    #[default]
    Off,
    Calm,
    Lively,
    Rain,
    Ink,
    Glow,
    Sparks,
    Neon,
}

impl Water {
    pub fn parse(s: &str) -> Option<Water> {
        match s.to_ascii_lowercase().as_str() {
            "off" => Some(Water::Off),
            "calm" => Some(Water::Calm),
            "lively" => Some(Water::Lively),
            "rain" => Some(Water::Rain),
            "ink" => Some(Water::Ink),
            "glow" => Some(Water::Glow),
            "sparks" => Some(Water::Sparks),
            "neon" => Some(Water::Neon),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Water::Off => "off",
            Water::Calm => "calm",
            Water::Lively => "lively",
            Water::Rain => "rain",
            Water::Ink => "ink",
            Water::Glow => "glow",
            Water::Sparks => "sparks",
            Water::Neon => "neon",
        }
    }

    pub fn preset(self) -> Option<Preset> {
        let calm = Preset {
            damping: 0.985,
            refraction: 14.0,
            gain: 1.0,
            shine: 0.2,
            rain: None,
        };
        match self {
            Water::Off | Water::Ink | Water::Glow | Water::Sparks | Water::Neon => None,
            Water::Calm => Some(calm),
            Water::Lively => Some(Preset {
                damping: 0.995,
                refraction: 1.5 * calm.refraction,
                gain: 1.4,
                shine: 0.26,
                ..calm
            }),
            Water::Rain => Some(Preset {
                rain: Some((0.4, 1.2)),
                ..calm
            }),
        }
    }

    pub fn fluid(self) -> Option<crate::fluid::Profile> {
        use crate::fluid::Profile;
        match self {
            Water::Ink => Some(Profile::Ink),
            Water::Glow => Some(Profile::Glow),
            Water::Sparks => Some(Profile::Sparks),
            Water::Neon => Some(Profile::Neon),
            _ => None,
        }
    }

    pub fn is_fluid(self) -> bool {
        self.fluid().is_some()
    }
}

/// `[wallpaper] water_rate`: how often the field steps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rate {
    #[default]
    Refresh,
    Fixed(u32),
}

impl Rate {
    /// "refresh" in any case, or a whole number of steps a second from 30 to 240.
    pub fn parse(s: &str) -> Option<Rate> {
        if s.eq_ignore_ascii_case("refresh") {
            return Some(Rate::Refresh);
        }
        s.parse()
            .ok()
            .filter(|hz| (30..=240).contains(hz))
            .map(Rate::Fixed)
    }

    pub fn hz(self, refresh_hz: f32) -> f32 {
        match self {
            Rate::Refresh => refresh_hz,
            Rate::Fixed(hz) => hz as f32,
        }
    }
}

/// How the field moves and looks. The numbers are for steps at [`REFERENCE_HZ`]; [`Step::new`] scales them to any rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preset {
    pub damping: f32,
    /// The largest offset of a wallpaper lookup, physical pixels, at the steepest slope the normals keep.
    pub refraction: f32,
    pub gain: f32,
    pub shine: f32,
    pub rain: Option<(f32, f32)>,
}

pub const FIELD_DIV: i32 = 4;
/// The step rate the presets, wave speed and wake depths are set at.
pub const REFERENCE_HZ: f32 = 60.0;
/// Reference steps' worth of time one frame may run after a stall; the rest is dropped.
pub const MAX_CATCH_UP: u32 = 4;
pub const SPLASHES_PER_STEP: usize = 4;
/// The frames a second rain draws while nothing else stirs the water; each runs the steps owed
/// since the last, so the ripples keep their speed.
pub const RAIN_FPS: f32 = 30.0;
/// Seconds after a wake, click or dragged edge during which rain draws every step again.
pub const STIRRED_SECS: f32 = 2.0;
pub const QUEUE_MAX: usize = 32;

pub const WAKE_RADIUS: f32 = 3.0;
pub const WAKE_DEPTH_MAX: f32 = 0.35;
/// The longest pointer move, in logical px, that lays a wake: 256 px between two motion events is over
/// 15000 px/s at 60 reports a second. A longer move is a jump (a touch, a tablet coming near, a warp)
/// with no path between, and lays nothing.
pub const WAKE_JUMP: f64 = 256.0;
/// A wake's depth per square root of its length in texels, one reference step long: a slow pointer
/// still shows, a fast one nears the cap slowly.
const WAKE_PER_ROOT_TEXEL: f32 = 0.05;
const CLICK_RADIUS: f32 = 6.0;
const CLICK_DEPTH: f32 = -0.8;
const RING_RADIUS: f32 = 2.0;
const RING_DEPTH: f32 = -0.25;
const DROP_RADIUS: f32 = 3.0;
const DROP_DEPTH: f32 = -0.6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Splash {
    pub from: (f32, f32),
    pub to: (f32, f32),
    pub radius: f32,
    pub depth: f32,
}

impl Splash {
    pub fn disc(at: (f32, f32), radius: f32, depth: f32) -> Splash {
        Splash {
            from: at,
            to: at,
            radius,
            depth,
        }
    }

    pub fn segment(from: (f32, f32), to: (f32, f32), radius: f32, depth: f32) -> Splash {
        Splash {
            from,
            to,
            radius,
            depth,
        }
    }

    /// A pointer's wake: [`Queue::take`] sets its depth for the step rate.
    fn is_wake(&self) -> bool {
        self.radius == WAKE_RADIUS && self.from != self.to
    }

    fn length(&self) -> f32 {
        (self.to.0 - self.from.0).hypot(self.to.1 - self.from.1)
    }

    pub fn energy(&self) -> f32 {
        let area =
            std::f32::consts::PI * self.radius * self.radius + 2.0 * self.radius * self.length();
        self.depth * self.depth * area
    }
}

/// An output-local logical point in field texels, for an output at `scale`.
pub fn field_point(local: (f64, f64), scale: f64) -> (f32, f32) {
    let k = scale / f64::from(FIELD_DIV);
    ((local.0 * k) as f32, (local.1 * k) as f32)
}

pub fn field_size(physical: (i32, i32)) -> (i32, i32) {
    let up = |v: i32| (v.max(1) + FIELD_DIV - 1) / FIELD_DIV;
    (up(physical.0), up(physical.1))
}

/// The pointer's wake from `from` to `to` (output-local logical); None when it did not move or jumped
/// further than [`WAKE_JUMP`]. Faster motion presses deeper, as the square root of the distance, up to [`WAKE_DEPTH_MAX`].
pub fn wake(from: (f64, f64), to: (f64, f64), scale: f64) -> Option<Splash> {
    if (to.0 - from.0).hypot(to.1 - from.1) > WAKE_JUMP {
        return None;
    }
    let s = Splash::segment(
        field_point(from, scale),
        field_point(to, scale),
        WAKE_RADIUS,
        0.0,
    );
    let len = s.length();
    (len > 0.0).then(|| Splash {
        depth: wake_depth(len, REFERENCE_HZ),
        ..s
    })
}

/// The depth of a wake `len` texels long, the pointer's travel over one step at `hz`. A texel on the
/// path collects about `(WAKE_RADIUS + len) / len` wakes, so depth is scaled to collect as much at any
/// rate as at [`REFERENCE_HZ`] for the same pointer speed.
fn wake_depth(len: f32, hz: f32) -> f32 {
    let at_ref = len * (hz / REFERENCE_HZ);
    let depth = (at_ref.sqrt() * WAKE_PER_ROOT_TEXEL).min(WAKE_DEPTH_MAX);
    -(depth * ((WAKE_RADIUS + at_ref) * len) / (at_ref * (WAKE_RADIUS + len)))
}

pub fn click(at: (f64, f64), scale: f64) -> Splash {
    Splash::disc(field_point(at, scale), CLICK_RADIUS, CLICK_DEPTH)
}

/// A window's frame `(x, y, w, h)` appearing or going: a shallow line along each edge.
pub fn ring(rect: (f64, f64, f64, f64), scale: f64) -> [Splash; 4] {
    let (x, y, w, h) = rect;
    let p = |px, py| field_point((px, py), scale);
    let side = |a, b| Splash::segment(a, b, RING_RADIUS, RING_DEPTH);
    [
        side(p(x, y), p(x + w, y)),
        side(p(x + w, y), p(x + w, y + h)),
        side(p(x + w, y + h), p(x, y + h)),
        side(p(x, y + h), p(x, y)),
    ]
}

/// The longest gap, seconds, [`edge`] counts between two frames of a drag: a window held still then moved moved over one frame.
const EDGE_GAP_MAX: f32 = 0.05;

/// A window frame dragged from `from` to `to` (both `(x, y, w, h)`) over the `dt` seconds since the
/// last: a wake along the leading edge, the pointer's wake for the same speed times the reference steps
/// `dt` spans, so a swept texel collects as much at any frame rate. None when it did not move.
pub fn edge(
    from: (f64, f64, f64, f64),
    to: (f64, f64, f64, f64),
    scale: f64,
    dt: f32,
) -> Option<Splash> {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let steps = REFERENCE_HZ * dt.clamp(0.001, EDGE_GAP_MAX);
    let per_step = f64::from(steps);
    let depth = wake((0.0, 0.0), (dx / per_step, dy / per_step), scale)?.depth * steps;
    let (x, y, w, h) = to;
    let (a, b) = if dx.abs() >= dy.abs() {
        let ex = if dx > 0.0 { x + w } else { x };
        ((ex, y), (ex, y + h))
    } else {
        let ey = if dy > 0.0 { y + h } else { y };
        ((x, ey), (x + w, ey))
    };
    Some(Splash::segment(
        field_point(a, scale),
        field_point(b, scale),
        RING_RADIUS,
        depth,
    ))
}

/// Splashes waiting for the next step, oldest first. A wake starting where the last ended and not
/// turning a right angle or more joins it, as deep as a wake that long, so a 1000 Hz mouse queues one
/// segment per step. A dragged window's edge waits on its own: frames between steps add up into one.
#[derive(Clone, Debug, Default)]
pub struct Queue {
    splashes: Vec<Splash>,
    edge: Option<Splash>,
}

impl Queue {
    pub fn push(&mut self, s: Splash) {
        if let Some(last) = self.splashes.last_mut()
            && last.radius == WAKE_RADIUS
            && s.radius == WAKE_RADIUS
            && last.to == s.from
            && last.from != last.to
            && s.from != s.to
            && runs_on(last, &s)
        {
            last.to = s.to;
            last.depth = wake_depth(last.length(), REFERENCE_HZ);
            return;
        }
        if self.splashes.len() == QUEUE_MAX {
            self.splashes.remove(0);
        }
        self.splashes.push(s);
    }

    /// A dragged window's [`edge`] for one frame, as deep as every frame's since the last step together.
    pub fn push_edge(&mut self, s: Splash) {
        let depth = self.edge.map_or(0.0, |e| e.depth) + s.depth;
        self.edge = Some(Splash { depth, ..s });
    }

    /// The waiting edge, then the oldest of the rest, `n` in all, removed for a step at `hz`: each wake as deep as its length makes it at that rate.
    pub fn take(&mut self, n: usize, hz: f32) -> Vec<Splash> {
        let edge = self.edge.take_if(|_| n > 0);
        let rest = (n - usize::from(edge.is_some())).min(self.splashes.len());
        edge.into_iter()
            .chain(self.splashes.drain(..rest).map(|s| {
                if s.is_wake() {
                    Splash {
                        depth: wake_depth(s.length(), hz),
                        ..s
                    }
                } else {
                    s
                }
            }))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.splashes.len() + usize::from(self.edge.is_some())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn runs_on(last: &Splash, s: &Splash) -> bool {
    let a = (last.to.0 - last.from.0, last.to.1 - last.from.1);
    let b = (s.to.0 - s.from.0, s.to.1 - s.from.1);
    a.0 * b.0 + a.1 * b.1 > 0.0
}

/// An estimate of the motion left in the field, kept without reading it back: splashes add their
/// energy, each step takes its damping squared. It ignores `preset.gain` so each preset settles in its own time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Energy(pub f32);

impl Energy {
    /// Below this the field is still enough to flatten and stop stepping: 3% of a click's energy.
    pub const IDLE: f32 =
        0.03 * CLICK_DEPTH * CLICK_DEPTH * std::f32::consts::PI * CLICK_RADIUS * CLICK_RADIUS;

    pub fn add(&mut self, s: &Splash) {
        self.0 += s.energy();
    }

    pub fn step(&mut self, damping: f32) {
        self.0 *= damping * damping;
    }

    pub fn idle(&self) -> bool {
        self.0 < Energy::IDLE
    }
}

/// One step's constants at a step rate, from a preset's damping at [`REFERENCE_HZ`]: waves fall and travel as far a second at any rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub damping: f32,
    /// The wave speed squared, texels a step: 0.5 at the reference rate, falling with the square of the
    /// rate above it; below it stays at 0.5 (the most that is stable) and waves slow with the rate.
    pub c2: f32,
}

impl Step {
    pub fn new(damping: f32, hz: f32) -> Step {
        let k = REFERENCE_HZ / hz;
        Step {
            damping: damping.powf(k),
            c2: (0.5 * k * k).min(0.5),
        }
    }
}

/// The step clock: how many steps at `hz` a frame `dt` seconds after the last runs. It keeps half a
/// step in hand, so frames at the step rate with up to half a frame of jitter each run one step.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    owed: f32,
}

impl Default for Clock {
    fn default() -> Clock {
        Clock { owed: 0.5 }
    }
}

impl Clock {
    pub fn steps(&mut self, dt: f32, hz: f32) -> u32 {
        self.owed += dt.max(0.0) * hz;
        let n = self.owed.floor();
        self.owed -= n;
        let cap = (MAX_CATCH_UP as f32 * hz / REFERENCE_HZ).ceil() as u32;
        if n as u32 > cap {
            self.owed = 0.5;
            return cap;
        }
        n as u32
    }

    /// As `steps`, for a sim too costly to catch up: up to the steps a frame at `refresh` Hz runs
    /// at `hz`, at most two; a frame owing more runs one and drops the rest.
    pub fn steps_no_catch_up(&mut self, dt: f32, hz: f32, refresh: f32) -> u32 {
        let most = ((hz / refresh).ceil() as u32).clamp(1, 2);
        match self.steps(dt, hz) {
            n if n <= most => n,
            _ => {
                self.owed = 0.5;
                1
            }
        }
    }

    /// Seconds from the last `steps` until a frame there runs a step at `hz`.
    pub fn until_step(&self, hz: f32) -> f32 {
        self.until_steps(1, hz)
    }

    /// Seconds from the last `steps` until a frame there runs `n` steps at `hz`.
    pub fn until_steps(&self, n: u32, hz: f32) -> f32 {
        (n as f32 - self.owed).max(0.0) / hz
    }
}

/// The steps at `hz` each frame of rain runs: as many as keep it at [`RAIN_FPS`] or above.
pub fn rain_steps_per_frame(hz: f32) -> u32 {
    ((hz / RAIN_FPS).floor() as u32).max(1)
}

#[derive(Clone, Debug)]
pub struct Rain {
    rng: Rng,
    /// Seconds until the next drop; the first falls on the first tick.
    wait: f32,
}

impl Rain {
    pub fn new(seed: u32) -> Rain {
        Rain {
            rng: Rng::new(seed),
            wait: 0.0,
        }
    }

    pub fn tick(&mut self, dt: f32, range: (f32, f32), field: (i32, i32)) -> Option<Splash> {
        self.wait -= dt;
        if self.wait > 0.0 {
            return None;
        }
        self.wait = self.rng.range(range.0, range.1);
        let at = (
            self.rng.range(0.0, field.0 as f32),
            self.rng.range(0.0, field.1 as f32),
        );
        Some(Splash::disc(at, DROP_RADIUS, DROP_DEPTH))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fluid_styles_parse_and_round_trip() {
        for s in ["ink", "glow", "sparks", "neon", "INK"] {
            assert_eq!(Water::parse(s).unwrap().name(), s.to_ascii_lowercase());
        }
    }

    #[test]
    fn fluid_styles_have_no_ripple_preset() {
        use crate::fluid::Profile;
        assert!(Water::Ink.preset().is_none());
        assert_eq!(Water::Ink.fluid(), Some(Profile::Ink));
        assert!(Water::Ink.is_fluid());
        assert!(Water::Calm.fluid().is_none());
        assert!(!Water::Calm.is_fluid());
    }

    use super::*;

    #[test]
    fn values_parse_and_name_back() {
        for w in [Water::Off, Water::Calm, Water::Lively, Water::Rain] {
            assert_eq!(Water::parse(w.name()), Some(w));
        }
        assert_eq!(Water::parse("CALM"), Some(Water::Calm));
        assert_eq!(Water::parse("storm"), None);
        assert_eq!(Water::default(), Water::Off);
        assert!(Water::Off.preset().is_none());
    }

    #[test]
    fn presets_hold_their_numbers() {
        let calm = Water::Calm.preset().unwrap();
        let lively = Water::Lively.preset().unwrap();
        let rain = Water::Rain.preset().unwrap();
        assert_eq!(calm.damping, 0.985);
        assert_eq!(lively.damping, 0.995);
        assert_eq!((calm.refraction, calm.shine), (14.0, 0.2));
        assert_eq!(lively.refraction, 1.5 * calm.refraction);
        assert_eq!((lively.gain, lively.shine), (1.4, 0.26));
        assert_eq!(rain.damping, calm.damping);
        assert_eq!(rain.rain, Some((0.4, 1.2)));
        assert_eq!(calm.rain, None);
    }

    /// Seconds from one click until the energy estimate goes idle, stepping at `hz`.
    fn settle(p: &Preset, hz: f32) -> f32 {
        let mut e = Energy::default();
        e.add(&click((100.0, 100.0), 1.0));
        let d = Step::new(p.damping, hz).damping;
        let mut steps = 0;
        while !e.idle() {
            e.step(d);
            steps += 1;
            assert!(steps < 10_000, "never idles");
        }
        steps as f32 / hz
    }

    #[test]
    fn a_click_settles_in_each_presets_time() {
        let calm = settle(&Water::Calm.preset().unwrap(), REFERENCE_HZ);
        assert!(
            (1.5..=2.5).contains(&calm),
            "calm settles in about 2 s: {calm}"
        );
        let lively = settle(&Water::Lively.preset().unwrap(), REFERENCE_HZ);
        assert!(
            (4.5..=6.5).contains(&lively),
            "lively settles in about 5 s: {lively}"
        );
    }

    #[test]
    fn a_click_settles_in_the_same_time_at_any_rate() {
        for w in [Water::Calm, Water::Lively, Water::Rain] {
            let p = w.preset().unwrap();
            let at60 = settle(&p, 60.0);
            for hz in [30.0, 144.0] {
                let t = settle(&p, hz);
                assert!(
                    (t - at60).abs() <= 1.0 / hz + 1.0 / 60.0,
                    "{w:?} settles in {at60} s at 60 Hz and {t} s at {hz} Hz"
                );
            }
        }
    }

    #[test]
    fn a_step_at_sixty_hertz_is_the_preset() {
        let s = Step::new(0.985, 60.0);
        assert_eq!(s.damping, 0.985);
        assert_eq!(s.c2, 0.5);
        let fast = Step::new(0.985, 120.0);
        assert!((fast.damping - 0.985f32.sqrt()).abs() < 1e-6);
        assert_eq!(fast.c2, 0.125, "a quarter: waves cross half as far a step");
        assert_eq!(Step::new(0.985, 30.0).c2, 0.5, "the stable limit holds");
    }

    #[test]
    fn rates_parse_and_give_their_hertz() {
        assert_eq!(Rate::parse("refresh"), Some(Rate::Refresh));
        assert_eq!(Rate::parse("Refresh"), Some(Rate::Refresh));
        assert_eq!(Rate::parse("30"), Some(Rate::Fixed(30)));
        assert_eq!(Rate::parse("240"), Some(Rate::Fixed(240)));
        for bad in ["29", "241", "fast", "", "-60", "60.5"] {
            assert_eq!(Rate::parse(bad), None, "{bad:?} is no rate");
        }
        assert_eq!(Rate::default(), Rate::Refresh);
        assert_eq!(Rate::Refresh.hz(144.0), 144.0);
        assert_eq!(Rate::Fixed(90).hz(144.0), 90.0);
    }

    #[test]
    fn the_clock_steps_at_its_rate() {
        let mut c = Clock::default();
        let n: u32 = (0..144).map(|_| c.steps(1.0 / 144.0, 60.0)).sum();
        assert!(
            (59..=61).contains(&n),
            "60 Hz steps under 144 Hz frames: {n}"
        );
        let mut c = Clock::default();
        let n: Vec<u32> = (0..144).map(|_| c.steps(1.0 / 144.0, 144.0)).collect();
        assert_eq!(n.iter().sum::<u32>(), 144, "144 Hz: 144 steps in a second");
        assert!(n.iter().all(|&k| k == 1), "one a frame: {n:?}");
    }

    #[test]
    fn the_clock_steps_once_a_frame_through_jitter() {
        let mut c = Clock::default();
        for i in 0..240 {
            let jitter = if i % 2 == 0 { 0.7 } else { 1.3 };
            assert_eq!(c.steps(jitter / 144.0, 144.0), 1, "frame {i}");
        }
    }

    #[test]
    fn the_clock_caps_catch_up_steps() {
        let mut c = Clock::default();
        assert_eq!(c.steps(5.0, 60.0), 4, "a 5 s stall steps 4 times at 60 Hz");
        assert_eq!(c.steps(0.0, 60.0), 0, "and drops the rest");
        let mut c = Clock::default();
        assert_eq!(c.steps(5.0, 144.0), 10, "the same 67 ms at 144 Hz");
        let mut c = Clock::default();
        assert_eq!(c.steps(5.0, 30.0), 2, "and at 30 Hz");
    }

    #[test]
    fn a_late_frame_runs_one_step_without_catch_up() {
        let mut c = Clock::default();
        assert_eq!(c.steps_no_catch_up(1.0 / 60.0, 60.0, 60.0), 1);
        assert_eq!(
            c.steps_no_catch_up(2.0 / 60.0, 60.0, 60.0),
            1,
            "one frame late"
        );
        assert_eq!(
            c.steps_no_catch_up(1.0 / 60.0, 60.0, 60.0),
            1,
            "the late time is dropped"
        );
        let n: Vec<u32> = (0..30)
            .map(|_| c.steps_no_catch_up(1.0 / 30.0, 60.0, 60.0))
            .collect();
        assert!(
            n.iter().all(|&k| k == 1),
            "30 fps frames at a 60 Hz rate: {n:?}"
        );
        assert_eq!(c.steps_no_catch_up(5.0, 60.0, 60.0), 1, "a stall");
        assert_eq!(c.steps_no_catch_up(0.0, 60.0, 60.0), 0, "drops the rest");
    }

    #[test]
    fn a_rate_above_refresh_runs_two_steps_a_frame_at_most() {
        let mut c = Clock::default();
        let n: Vec<u32> = (0..60)
            .map(|_| c.steps_no_catch_up(1.0 / 60.0, 120.0, 60.0))
            .collect();
        assert!(n.iter().all(|&k| k == 2), "120 Hz on 60 Hz frames: {n:?}");
        let n: u32 = (0..60)
            .map(|_| c.steps_no_catch_up(1.0 / 60.0, 240.0, 60.0))
            .sum();
        assert_eq!(
            n, 60,
            "240 Hz on 60 Hz frames: four due is too many, one a frame"
        );
        let mut c = Clock::default();
        let n: Vec<u32> = (0..4)
            .map(|_| c.steps_no_catch_up(1.0 / 60.0, 30.0, 60.0))
            .collect();
        assert_eq!(n, [1, 0, 1, 0], "a 30 Hz rate steps every other frame");
    }

    #[test]
    fn the_clock_keeps_every_step_through_a_missed_frame() {
        let mut c = Clock::default();
        let n: u32 = (0..60)
            .map(|i| c.steps(if i % 3 == 0 { 2.0 } else { 1.0 } / 60.0, 240.0))
            .sum();
        assert_eq!(n, 320, "240 Hz on 60 Hz frames, every third one late");
    }

    #[test]
    fn frames_at_until_step_each_run_one_step() {
        for hz in [30.0, 45.0, 60.0, 144.0] {
            let mut c = Clock::default();
            let mut dt = 0.0;
            for i in 0..200 {
                let n = c.steps(dt, hz);
                assert!(i == 0 || n == 1, "{hz} Hz frame {i} ran {n} steps");
                dt = c.until_step(hz) + 1e-4;
                assert!(dt <= 1.0 / hz + 2e-4, "{hz} Hz waits {dt} s");
            }
        }
        let mut c = Clock::default();
        c.steps(0.0, 30.0);
        assert_eq!(c.until_step(30.0), 0.5 / 30.0, "half a step in hand");
        assert_eq!(
            c.steps(0.4 / 30.0, 30.0),
            0,
            "a frame before it steps nothing"
        );
    }

    #[test]
    fn rain_frames_run_the_steps_of_rain_fps() {
        assert_eq!(rain_steps_per_frame(60.0), 2);
        assert_eq!(rain_steps_per_frame(120.0), 4);
        assert_eq!(
            rain_steps_per_frame(144.0),
            4,
            "36 frames a second, not under 30"
        );
        assert_eq!(rain_steps_per_frame(30.0), 1);
        assert_eq!(rain_steps_per_frame(45.0), 1);
        for hz in [60.0, 120.0, 144.0] {
            let n = rain_steps_per_frame(hz);
            let mut c = Clock::default();
            let mut dt = 0.0;
            for i in 0..200 {
                let ran = c.steps(dt, hz);
                assert!(
                    i == 0 || ran == n,
                    "{hz} Hz frame {i} ran {ran} steps, not {n}"
                );
                dt = c.until_steps(n, hz) + 1e-4;
            }
        }
    }

    #[test]
    fn field_points_scale_and_divide() {
        assert_eq!(field_point((40.0, 80.0), 1.0), (10.0, 20.0));
        assert_eq!(field_point((40.0, 80.0), 1.5), (15.0, 30.0));
        assert_eq!(field_size((1920, 1080)), (480, 270));
        assert_eq!(field_size((1366, 768)), (342, 192), "rounds up");
    }

    #[test]
    fn a_wake_needs_motion_and_caps_its_depth() {
        assert!(wake((10.0, 10.0), (10.0, 10.0), 1.0).is_none());
        let slow = wake((0.0, 0.0), (4.0, 0.0), 1.0).unwrap();
        let fast = wake((0.0, 0.0), (200.0, 0.0), 1.0).unwrap();
        assert!(slow.depth.abs() < fast.depth.abs());
        assert!(fast.depth.abs() <= WAKE_DEPTH_MAX);
        assert_eq!(fast.from, (0.0, 0.0));
        assert_eq!(fast.to, (50.0, 0.0));
    }

    #[test]
    fn a_pointer_jump_lays_no_wake() {
        // A touch, a tablet coming near or a warp moves the pointer without the path between.
        assert!(wake((0.0, 0.0), (WAKE_JUMP + 1.0, 0.0), 1.0).is_none());
        assert!(wake((0.0, 0.0), (900.0, 700.0), 2.0).is_none());
        assert!(
            wake((0.0, 0.0), (WAKE_JUMP, 0.0), 2.0).is_some(),
            "the fastest flick between two reports still wakes, at any scale"
        );
    }

    #[test]
    fn a_ring_follows_the_rect() {
        let r = ring((40.0, 40.0, 80.0, 40.0), 1.0);
        assert_eq!(r[0].from, (10.0, 10.0));
        assert_eq!(r[0].to, (30.0, 10.0));
        assert_eq!(r[2].from, (30.0, 20.0));
        assert!(
            r.iter()
                .all(|s| s.depth.abs() < click((0.0, 0.0), 1.0).depth.abs())
        );
    }

    #[test]
    fn a_fast_reporting_mouse_presses_as_deep() {
        let mut q = Queue::default();
        for i in 0..16 {
            let x = f64::from(i) * 1.25;
            q.push(wake((x * 4.0, 0.0), ((x + 1.25) * 4.0, 0.0), 1.0).unwrap());
        }
        let one = wake((0.0, 0.0), (80.0, 0.0), 1.0).unwrap();
        assert_eq!(q.take(QUEUE_MAX, REFERENCE_HZ), vec![one]);
    }

    #[test]
    fn a_jittery_pointer_still_joins() {
        let mut q = Queue::default();
        q.push(wake((0.0, 0.0), (4.0, 0.0), 1.0).unwrap());
        q.push(wake((4.0, 0.0), (8.0, 4.0), 1.0).unwrap());
        assert_eq!(q.len(), 1, "a 45 degree step joins the wake");
    }

    #[test]
    fn slow_and_fast_wakes_both_show() {
        let slow = wake((0.0, 0.0), (5.0, 0.0), 1.0).unwrap().depth;
        let fast = wake((0.0, 0.0), (20.0, 0.0), 1.0).unwrap().depth;
        assert!(slow <= -0.05, "a slow pointer presses 0.05 or more: {slow}");
        assert!(fast >= -0.12, "a fast one 0.12 or less: {fast}");
    }

    /// What `splashes` lay into the texel at `p`: the step program's capsule profile, summed.
    fn laid(splashes: &[Splash], p: (f32, f32)) -> f32 {
        splashes
            .iter()
            .map(|s| {
                let ab = (s.to.0 - s.from.0, s.to.1 - s.from.1);
                let l = ab.0 * ab.0 + ab.1 * ab.1;
                let ap = (p.0 - s.from.0, p.1 - s.from.1);
                let t = if l > 0.0 {
                    ((ap.0 * ab.0 + ap.1 * ab.1) / l).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let d = (ap.0 - ab.0 * t).hypot(ap.1 - ab.1 * t);
                let k = (d / s.radius).min(1.0);
                s.depth * 0.5 * (1.0 + (std::f32::consts::PI * k).cos())
            })
            .sum()
    }

    /// The depth a pointer moving right at `speed` px/s for half a second, reported at 1000 Hz, lays
    /// into the texels along its path, averaged over the middle; the queue drained by steps at `hz`.
    fn wake_at(speed: f64, hz: f64) -> f32 {
        let mut q = Queue::default();
        let mut all = Vec::new();
        let mut next = 0.0;
        for ms in 0..500 {
            let t = f64::from(ms) / 1000.0;
            let x = 40.0 + speed * t;
            q.push(wake((x, 40.0), (x + speed / 1000.0, 40.0), 1.0).unwrap());
            if t >= next {
                all.extend(q.take(SPLASHES_PER_STEP, hz as f32));
                next += 1.0 / hz;
            }
        }
        all.extend(q.take(QUEUE_MAX, hz as f32));
        let (from, to) = ((40.0 + speed / 8.0) / 4.0, (40.0 + speed * 3.0 / 8.0) / 4.0);
        let n = 200;
        (0..n)
            .map(|i| {
                let x = from + (to - from) * f64::from(i) / f64::from(n);
                laid(&all, (x as f32, 10.0))
            })
            .sum::<f32>()
            / n as f32
    }

    #[test]
    fn a_wake_lays_as_deep_at_any_rate() {
        for speed in [300.0, 1200.0] {
            let at60 = wake_at(speed, 60.0);
            assert!(at60 < -0.05, "{speed} px/s lays {at60}");
            for hz in [30.0, 144.0, 240.0] {
                let d = wake_at(speed, hz);
                assert!(
                    (d / at60 - 1.0).abs() < 0.15,
                    "{speed} px/s lays {at60} at 60 Hz and {d} at {hz} Hz"
                );
            }
        }
    }

    /// The depth a window frame dragged right at 600 px/s for half a second lays into the texels its
    /// leading edge sweeps, averaged over the middle, with frames `gaps` seconds apart (repeating).
    fn edge_wake_at(gaps: &[f64]) -> f32 {
        let speed = 600.0;
        let frame = |t: f64| (100.0 + speed * t, 100.0, 80.0, 60.0);
        let mut all = Vec::new();
        let mut t = 0.0;
        for gap in gaps.iter().cycle() {
            if t + gap > 0.5 {
                break;
            }
            all.extend(edge(frame(t), frame(t + gap), 1.0, *gap as f32));
            t += gap;
        }
        let n = 200;
        (0..n)
            .map(|i| {
                let x = (255.0 + 150.0 * f64::from(i) / f64::from(n)) / 4.0;
                laid(&all, (x as f32, 32.5))
            })
            .sum::<f32>()
            / n as f32
    }

    #[test]
    fn a_dragged_window_lays_as_deep_at_any_frame_rate() {
        let at60 = edge_wake_at(&[1.0 / 60.0]);
        assert!(at60 < -0.05, "a 600 px/s drag lays {at60}");
        for (what, gaps) in [
            ("144 Hz", vec![1.0 / 144.0]),
            ("30 Hz", vec![1.0 / 30.0]),
            ("two outputs' frames", vec![0.002, 1.0 / 60.0 - 0.002]),
        ] {
            let d = edge_wake_at(&gaps);
            assert!(
                (d / at60 - 1.0).abs() < 0.15,
                "{at60} at 60 Hz and {d} at {what}"
            );
        }
    }

    /// [`edge_wake_at`] with every frame's edge queued and the queue drained by steps at `step_hz`, as the compositor does.
    fn queued_edge_wake_at(frame_hz: f64, step_hz: f64) -> f32 {
        let speed = 600.0;
        let frame = |t: f64| (100.0 + speed * t, 100.0, 80.0, 60.0);
        let mut q = Queue::default();
        let mut all = Vec::new();
        let gap = 1.0 / frame_hz;
        let (mut t, mut next) = (0.0, 0.0);
        while t + gap <= 0.5 {
            q.push_edge(edge(frame(t), frame(t + gap), 1.0, gap as f32).unwrap());
            t += gap;
            while next <= t {
                all.extend(q.take(SPLASHES_PER_STEP, step_hz as f32));
                assert!(q.len() <= 1, "at most one frame's edge waits");
                next += 1.0 / step_hz;
            }
        }
        all.extend(q.take(QUEUE_MAX, step_hz as f32));
        let n = 200;
        (0..n)
            .map(|i| {
                let x = (255.0 + 150.0 * f64::from(i) / f64::from(n)) / 4.0;
                laid(&all, (x as f32, 32.5))
            })
            .sum::<f32>()
            / n as f32
    }

    #[test]
    fn a_dragged_window_lays_as_deep_through_a_slower_step() {
        let at60 = edge_wake_at(&[1.0 / 60.0]);
        for (frames, steps) in [(144.0, 30.0), (240.0, 60.0), (60.0, 60.0)] {
            let d = queued_edge_wake_at(frames, steps);
            assert!(
                (d / at60 - 1.0).abs() < 0.15,
                "{at60} laid directly, {d} with {frames} Hz frames and {steps} Hz steps"
            );
        }
    }

    #[test]
    fn a_click_is_laid_once_at_any_rate() {
        let mut q = Queue::default();
        q.push(click((40.0, 40.0), 1.0));
        assert_eq!(
            q.take(SPLASHES_PER_STEP, 144.0),
            vec![click((40.0, 40.0), 1.0)]
        );
        assert!(q.take(SPLASHES_PER_STEP, 144.0).is_empty());
    }

    #[test]
    fn a_ring_queues_all_four_sides() {
        let mut q = Queue::default();
        for s in ring((40.0, 40.0, 80.0, 40.0), 1.0) {
            q.push(s);
        }
        assert_eq!(q.len(), 4, "the corners keep the sides apart");
    }

    #[test]
    fn the_queue_merges_a_pointer_storm() {
        let mut q = Queue::default();
        // Steps of 1/8 texel, exact in binary, so each segment starts exactly where the last ended.
        for i in 0..1000 {
            let x = i as f32 * 0.125;
            q.push(Splash::segment(
                (x, 0.0),
                (x + 0.125, 0.0),
                WAKE_RADIUS,
                -0.01,
            ));
        }
        assert!(q.len() <= QUEUE_MAX);
        let all = q.take(QUEUE_MAX, REFERENCE_HZ);
        assert_eq!(
            all.first().unwrap().from,
            (0.0, 0.0),
            "the path still starts where it did"
        );
        assert_eq!(
            all.last().unwrap().to,
            (125.0, 0.0),
            "and ends where it did"
        );
    }

    #[test]
    fn rain_falls_inside_its_interval_and_field() {
        let mut r = Rain::new(7);
        let mut t = 0.0;
        let mut last = 0.0;
        let mut drops = 0;
        while t < 60.0 {
            t += 1.0 / 60.0;
            if let Some(s) = r.tick(1.0 / 60.0, (0.4, 1.2), (480, 270)) {
                let gap = t - last;
                assert!(drops == 0 || (0.39..=1.22).contains(&gap), "gap {gap}");
                assert!((0.0..480.0).contains(&s.from.0) && (0.0..270.0).contains(&s.from.1));
                last = t;
                drops += 1;
            }
        }
        assert!((50..=150).contains(&drops), "{drops} drops a minute");
    }
}
