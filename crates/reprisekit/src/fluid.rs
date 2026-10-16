//! Fluid water styles' model: each profile's palettes, backdrop and flow constants.

/// Velocity and pressure grid, as a divisor of the output's physical size.
pub const SIM_DIV: i32 = 8;
/// Dye grid, as a divisor of the output's physical size.
pub const DYE_DIV: i32 = 2;
/// Jacobi iterations a step; the pressure carries over as the next step's first guess.
pub const PRESSURE_ITERS: u32 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Ink,
    Glow,
    Sparks,
    Neon,
}

/// Colours are RGB 0..1: palettes are dye, which adds up linearly; backdrops are as displayed.
pub struct Look {
    /// One palette per flavour.
    pub palettes: &'static [&'static [[f32; 3]]],
    /// Backdrop colour per flavour.
    pub backdrop: [[f32; 3]; 2],
    /// How far the wallpaper goes toward the backdrop at full level.
    pub dim: f32,
    pub bloom: f32,
    /// Seconds for the velocity to halve.
    pub velocity_half_life: f32,
    /// Seconds for the dye to halve.
    pub dye_half_life: f32,
    pub vorticity: f32,
    /// Splat radius as a fraction of the output's shorter side.
    pub splat_radius: f32,
    /// The chance a dye texel at a splat's centre gains a glitter point.
    pub glitter: f32,
}

static INK: Look = Look {
    palettes: &[
        &[
            [0.02, 0.30, 1.00],
            [1.00, 0.42, 0.02],
            [0.90, 0.05, 0.55],
            [0.00, 0.70, 0.80],
            [0.45, 0.10, 1.00],
        ],
        &[
            [1.00, 0.80, 0.10],
            [1.00, 0.20, 0.60],
            [0.55, 0.25, 1.00],
            [0.20, 0.75, 1.00],
            [1.00, 0.45, 0.10],
        ],
    ],
    backdrop: [[0.93, 0.95, 0.97], [0.32, 0.24, 0.36]],
    dim: 0.85,
    bloom: 0.5,
    velocity_half_life: 12.0,
    dye_half_life: 30.0,
    vorticity: 10.0,
    splat_radius: 0.04,
    glitter: 0.0,
};

static GLOW: Look = Look {
    palettes: &[&[
        [1.00, 0.95, 0.80],
        [1.00, 0.75, 0.25],
        [0.30, 0.55, 1.00],
        [0.85, 0.90, 1.00],
    ]],
    backdrop: [[0.10, 0.09, 0.12]; 2],
    dim: 0.8,
    bloom: 0.6,
    velocity_half_life: 10.0,
    dye_half_life: 30.0,
    vorticity: 16.0,
    splat_radius: 0.035,
    glitter: 0.0,
};

static SPARKS: Look = Look {
    palettes: &[&[
        [0.25, 0.55, 1.00],
        [1.00, 0.50, 0.20],
        [0.45, 0.75, 1.00],
        [1.00, 0.35, 0.15],
    ]],
    backdrop: [[0.06, 0.07, 0.10]; 2],
    dim: 0.85,
    bloom: 1.2,
    velocity_half_life: 9.0,
    dye_half_life: 30.0,
    vorticity: 14.0,
    splat_radius: 0.03,
    glitter: 0.2,
};

static NEON: Look = Look {
    palettes: &[&[
        [1.00, 0.00, 0.60],
        [0.00, 1.00, 0.90],
        [0.60, 0.10, 1.00],
        [1.00, 0.90, 0.00],
        [0.10, 0.45, 1.00],
    ]],
    backdrop: [[0.00, 0.01, 0.05]; 2],
    dim: 0.9,
    bloom: 0.9,
    velocity_half_life: 10.0,
    dye_half_life: 30.0,
    vorticity: 12.0,
    splat_radius: 0.03,
    glitter: 0.5,
};

impl Profile {
    pub fn look(self) -> &'static Look {
        match self {
            Profile::Ink => &INK,
            Profile::Glow => &GLOW,
            Profile::Sparks => &SPARKS,
            Profile::Neon => &NEON,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Pointer,
    Finger(u32),
}

/// Positions are logical px of the output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Force {
    /// Dye and velocity along the segment `from` to `at`; a point when they are equal.
    Splat {
        from: (f32, f32),
        at: (f32, f32),
        /// Logical px/s.
        vel: (f32, f32),
        /// A multiple of `Look::splat_radius`.
        radius: f32,
        colour: [f32; 3],
        amount: f32,
    },
    Swirl {
        centre: (f32, f32),
        /// Logical px.
        radius: f32,
        /// Radians/s, positive counter-clockwise on screen.
        spin: f32,
    },
    Clear,
}

/// Every gesture threshold; distances are logical px, times ms.
pub struct Feel {
    pub tap_ms: u32,
    pub tap_slop: f32,
    pub hold_ms: u32,
    pub hold_slop: f32,
    pub flick_speed: f32,
    pub flick_boost: f32,
    pub flick_window_ms: u32,
    pub twist_deg: f32,
    pub double_ms: u32,
    pub double_slop: f32,
    pub mouse_drift_px: f32,
    /// Dye a moving finger or held pointer lays per second.
    pub stroke_dye: f32,
    /// Dye the pointer lays per second moving with no button held.
    pub hover_dye: f32,
    /// Dye a held, still finger pours per second.
    pub pour_dye: f32,
    /// Seconds a pour takes to spread from a stroke's width to its widest.
    pub pour_grow_s: f32,
}

pub const FEEL: Feel = Feel {
    tap_ms: 200,
    tap_slop: 8.0,
    hold_ms: 300,
    hold_slop: 4.0,
    flick_speed: 1500.0,
    flick_boost: 3.0,
    flick_window_ms: 50,
    twist_deg: 20.0,
    double_ms: 350,
    double_slop: 30.0,
    mouse_drift_px: 600.0,
    stroke_dye: 15.0,
    hover_dye: 2.0,
    pour_dye: 2.5,
    pour_grow_s: 0.7,
};

/// Idle seconds before the fluid starts to fade.
pub const DRIFT_S: f32 = 60.0;
/// Seconds the fade takes after the drift.
pub const FADE_S: f32 = 60.0;

/// Fluid and wallpaper dim strength after `since_input_s` idle seconds: 1.0 through the drift,
/// then a smoothstep down to 0.0.
pub fn level(since_input_s: f32) -> f32 {
    let x = ((since_input_s - DRIFT_S) / FADE_S).clamp(0.0, 1.0);
    1.0 - x * x * (3.0 - 2.0 * x)
}

/// True once the fade has finished and the pane can stop rendering.
pub fn faded(since_input_s: f32) -> bool {
    level(since_input_s) == 0.0
}

/// Seconds the shown level takes to rise from 0.0 to 1.0.
pub const RISE_S: f32 = 0.3;

/// The level to show `dt_s` after showing `shown`, heading for `target`: it rises over
/// [`RISE_S`] and falls at once.
pub fn shown_level(shown: f32, target: f32, dt_s: f32) -> f32 {
    if target > shown {
        (shown + dt_s / RISE_S).min(target)
    } else {
        target
    }
}

const STROKE_RADIUS: f32 = 1.0;
/// The longest motion span, ms, one splat lays dye for: a finger still for longer lays none.
const LONGEST_SPAN_MS: u32 = 100;
const BURST_RADIUS: f32 = 3.0;
/// A pour's widest; more than a burst's, as a pour lays its dye thinner.
const POUR_RADIUS: f32 = 4.0;
const HOVER_RADIUS: f32 = 0.5;

#[derive(Clone, Copy)]
enum Tint {
    Fixed([f32; 3]),
    /// Palette position; the pointer's drifts along the stroke.
    Drift(f32),
}

struct Held {
    source: Source,
    tint: Tint,
    start: (f32, f32),
    start_ms: u32,
    last: (f32, f32),
    left_tap_slop: bool,
    hold_anchor: (f32, f32),
    hold_since: u32,
    /// The last frame that poured, since the hold began.
    poured: Option<u32>,
    last_ms: u32,
    samples: Vec<((f32, f32), u32)>,
    /// Where the next stroke splat starts; stays put while motion shares a timestamp.
    seg_from: ((f32, f32), u32),
}

struct Twist {
    prev_angle: f32,
    total: f32,
    prev_ms: u32,
    turning: bool,
}

/// Turns pointer and finger events into forces; the caller supplies every timestamp.
pub struct Hands {
    palettes: &'static [&'static [[f32; 3]]],
    palette: &'static [[f32; 3]],
    held: Vec<Held>,
    queue: Vec<(Option<Source>, Force)>,
    next_finger: usize,
    pointer_base: f32,
    pointer_pos: f32,
    last_hover: Option<((f32, f32), u32)>,
    /// Hover motion since `last_hover`, splatted by the next frame.
    hover_to: Option<((f32, f32), u32)>,
    last_tap: Option<((f32, f32), u32)>,
    twist: Option<Twist>,
    /// The latest event's ms, where `cancel` restarts the twist.
    latest_ms: u32,
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// The dye `per_s` lays over `ms`, up to [`LONGEST_SPAN_MS`].
fn laid(per_s: f32, ms: u32) -> f32 {
    per_s * ms.min(LONGEST_SPAN_MS) as f32 / 1000.0
}

/// Logical px/s over `ms`; still for no time.
fn velocity(from: (f32, f32), to: (f32, f32), ms: u32) -> (f32, f32) {
    if ms == 0 {
        return (0.0, 0.0);
    }
    let k = 1000.0 / ms as f32;
    ((to.0 - from.0) * k, (to.1 - from.1) * k)
}

fn at_position(palette: &[[f32; 3]], pos: f32) -> [f32; 3] {
    let n = palette.len();
    let i = pos.floor();
    let t = pos - i;
    let a = palette[(i as usize) % n];
    let b = palette[(i as usize + 1) % n];
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)
}

impl Hands {
    /// `flavour` indexes `Look::palettes`, clamped to the last one.
    pub fn new(profile: Profile, flavour: usize) -> Hands {
        let palettes = profile.look().palettes;
        Hands {
            palettes,
            palette: palettes[flavour.min(palettes.len() - 1)],
            held: Vec::new(),
            queue: Vec::new(),
            next_finger: 0,
            pointer_base: 0.0,
            pointer_pos: 0.0,
            last_hover: None,
            hover_to: None,
            last_tap: None,
            twist: None,
            latest_ms: 0,
        }
    }

    /// Later colours come from `flavour`'s palette; held sources go on.
    pub fn set_flavour(&mut self, flavour: usize) {
        self.palette = self.palettes[flavour.min(self.palettes.len() - 1)];
    }

    /// Queues a [`Force::Clear`] for the next frame; the next tap starts a new pair.
    pub fn clear(&mut self, ms: u32) {
        self.latest_ms = ms;
        self.last_tap = None;
        self.queue.push((None, Force::Clear));
    }

    fn colour(&self, tint: Tint) -> [f32; 3] {
        match tint {
            Tint::Fixed(c) => c,
            Tint::Drift(p) => at_position(self.palette, p),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn splat(
        &mut self,
        s: Option<Source>,
        from: (f32, f32),
        at: (f32, f32),
        vel: (f32, f32),
        radius: f32,
        tint: Tint,
        amount: f32,
    ) {
        let colour = self.colour(tint);
        self.queue.push((
            s,
            Force::Splat {
                from,
                at,
                vel,
                radius,
                colour,
                amount,
            },
        ));
    }

    fn index(&self, s: Source) -> Option<usize> {
        self.held.iter().position(|h| h.source == s)
    }

    pub fn down(&mut self, s: Source, at: (f32, f32), ms: u32) {
        self.latest_ms = ms;
        if self.index(s).is_some() {
            return;
        }
        let tint = match s {
            Source::Pointer => {
                let base = self.pointer_base;
                self.pointer_base += 1.0;
                self.pointer_pos = base;
                self.flush_hover();
                self.last_hover = None;
                Tint::Drift(base)
            }
            Source::Finger(_) => {
                let c = self.palette[self.next_finger % self.palette.len()];
                self.next_finger += 1;
                Tint::Fixed(c)
            }
        };
        self.held.push(Held {
            source: s,
            tint,
            start: at,
            start_ms: ms,
            last: at,
            last_ms: ms,
            left_tap_slop: false,
            hold_anchor: at,
            hold_since: ms,
            poured: None,
            samples: vec![(at, ms)],
            seg_from: (at, ms),
        });
        self.retwist(ms);
    }

    pub fn moved(&mut self, s: Source, at: (f32, f32), ms: u32) {
        self.latest_ms = ms;
        let Some(i) = self.index(s) else { return };
        let h = &mut self.held[i];
        let step = dist(h.last, at);
        h.last = at;
        h.last_ms = h.last_ms.max(ms);
        h.samples.push((at, ms));
        // `up` reads the flick window and the one point before it.
        let cutoff = h.last_ms.saturating_sub(FEEL.flick_window_ms);
        if let Some(k) = h.samples.iter().rposition(|p| p.1 < cutoff) {
            h.samples.drain(..k);
        }
        if dist(h.start, at) > FEEL.tap_slop {
            h.left_tap_slop = true;
        }
        if dist(h.hold_anchor, at) > FEEL.hold_slop {
            h.hold_anchor = at;
            h.hold_since = ms;
            h.poured = None;
        }
        if let Tint::Drift(p) = &mut h.tint {
            *p += step / FEEL.mouse_drift_px;
            self.pointer_pos = *p;
        }
    }

    /// Queues held source `i`'s one splat for its motion since the last, if it moved on in time.
    fn stroke(&mut self, i: usize) {
        let h = &mut self.held[i];
        let ((from, from_ms), (to, to_ms)) = (h.seg_from, (h.last, h.last_ms));
        if to_ms <= from_ms {
            return;
        }
        h.seg_from = (to, to_ms);
        let (s, tint) = (h.source, h.tint);
        let vel = velocity(from, to, to_ms - from_ms);
        let dye = laid(FEEL.stroke_dye, to_ms - from_ms);
        self.splat(Some(s), from, to, vel, STROKE_RADIUS, tint, dye);
    }

    /// The pointer moving with no button held.
    pub fn hover(&mut self, at: (f32, f32), ms: u32) {
        self.latest_ms = ms;
        let ms = self
            .hover_to
            .or(self.last_hover)
            .map_or(ms, |(_, t)| ms.max(t));
        if self.last_hover.is_none() {
            self.last_hover = Some((at, ms));
        }
        self.hover_to = Some((at, ms));
    }

    /// Queues the hover motion since the last splat as one splat.
    fn flush_hover(&mut self) {
        let Some((to, to_ms)) = self.hover_to.take() else {
            return;
        };
        let (from, from_ms) = self.last_hover.unwrap_or((to, to_ms));
        self.last_hover = Some((to, to_ms));
        let span = to_ms.saturating_sub(from_ms);
        if span == 0 {
            return;
        }
        let vel = velocity(from, to, span);
        let tint = Tint::Drift(self.pointer_pos);
        let dye = laid(FEEL.hover_dye, span);
        self.splat(None, from, to, vel, HOVER_RADIUS, tint, dye);
    }

    pub fn up(&mut self, s: Source, ms: u32) {
        self.latest_ms = ms;
        let Some(i) = self.index(s) else { return };
        self.stroke(i);
        let h = self.held.remove(i);
        self.retwist(ms);
        let burst = ms.saturating_sub(h.start_ms) <= FEEL.tap_ms && !h.left_tap_slop;
        if burst {
            let double = self.last_tap.take().filter(|&(p, t)| {
                ms.saturating_sub(t) <= FEEL.double_ms && dist(p, h.start) <= FEEL.double_slop
            });
            if double.is_some() {
                self.queue.push((None, Force::Clear));
            } else {
                self.last_tap = Some((h.start, ms));
                let at = h.start;
                self.splat(None, at, at, (0.0, 0.0), BURST_RADIUS, h.tint, 1.0);
            }
            return;
        }
        let window: Vec<_> = h
            .samples
            .iter()
            .filter(|p| p.1 + FEEL.flick_window_ms >= ms)
            .collect();
        let from = match window.first() {
            Some(&&f) if window.len() >= 2 => Some(f),
            Some(&&f) => h.samples.iter().rev().find(|p| p.1 < f.1).copied(),
            None => None,
        };
        let to = h.samples.last().copied();
        if let (Some(a), Some(b)) = (from, to) {
            let dt = b.1.saturating_sub(a.1);
            if dt > 0 && !window.is_empty() {
                let k = 1000.0 / dt as f32;
                let vel = ((b.0.0 - a.0.0) * k, (b.0.1 - a.0.1) * k);
                if vel.0.hypot(vel.1) > FEEL.flick_speed {
                    let boosted = (vel.0 * FEEL.flick_boost, vel.1 * FEEL.flick_boost);
                    let at = h.last;
                    self.splat(None, at, at, boosted, STROKE_RADIUS, h.tint, 1.0);
                }
            }
        }
    }

    /// Ends a source without firing a tap or flick; its undrained forces are dropped.
    pub fn cancel(&mut self, s: Source) {
        if let Some(i) = self.index(s) {
            self.held.remove(i);
            self.queue.retain(|(from, _)| *from != Some(s));
            self.retwist(self.latest_ms);
        }
    }

    pub fn cancel_all(&mut self) {
        self.held.clear();
        self.queue.clear();
        self.hover_to = None;
        self.twist = None;
    }

    fn fingers(&self) -> Vec<&Held> {
        self.held
            .iter()
            .filter(|h| h.source != Source::Pointer)
            .collect()
    }

    fn angle(&self) -> Option<f32> {
        match self.fingers()[..] {
            [a, b] => Some((b.last.1 - a.last.1).atan2(b.last.0 - a.last.0)),
            _ => None,
        }
    }

    /// Starts tracking the twist afresh whenever the set of fingers changes.
    fn retwist(&mut self, ms: u32) {
        self.twist = self.angle().map(|a| Twist {
            prev_angle: a,
            total: 0.0,
            prev_ms: ms,
            turning: false,
        });
    }

    /// Drains the forces since the last call: gestures in the order they happened, then one
    /// stroke splat per moving source.
    pub fn frame(&mut self, ms: u32) -> Vec<Force> {
        for i in 0..self.held.len() {
            self.stroke(i);
        }
        self.flush_hover();
        let mut out: Vec<Force> = self.queue.drain(..).map(|(_, f)| f).collect();
        let mut pours = Vec::new();
        for h in &mut self.held {
            let start = h.hold_since + FEEL.hold_ms;
            let from = h.poured.unwrap_or(start);
            if ms <= from || ms < start {
                continue;
            }
            h.poured = Some(ms);
            let grown = ((ms - start) as f32 / 1000.0 / FEEL.pour_grow_s).min(1.0);
            let radius = STROKE_RADIUS + (POUR_RADIUS - STROKE_RADIUS) * grown;
            pours.push((h.last, h.tint, radius, laid(FEEL.pour_dye, ms - from)));
        }
        for (at, tint, radius, amount) in pours {
            out.push(Force::Splat {
                from: at,
                at,
                vel: (0.0, 0.0),
                radius,
                colour: self.colour(tint),
                amount,
            });
        }
        if let (Some(angle), [a, b]) = (self.angle(), &self.fingers()[..]) {
            let (centre, radius) = (
                ((a.last.0 + b.last.0) / 2.0, (a.last.1 + b.last.1) / 2.0),
                dist(a.last, b.last) / 2.0,
            );
            let t = self
                .twist
                .as_mut()
                .expect("two fingers always have a twist");
            let delta = (angle - t.prev_angle + std::f32::consts::PI)
                .rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            t.total += delta;
            t.prev_angle = angle;
            let dt = ms.saturating_sub(t.prev_ms);
            t.prev_ms = ms;
            if t.total.abs().to_degrees() > FEEL.twist_deg {
                t.turning = true;
            }
            if t.turning && delta != 0.0 && dt > 0 {
                // Screen y points down, so a positive angle change turns clockwise.
                let spin = -delta * 1000.0 / dt as f32;
                out.push(Force::Swirl {
                    centre,
                    radius,
                    spin,
                });
            }
        }
        out
    }

    /// True while a source is held or forces are waiting to be drained.
    pub fn active(&self) -> bool {
        !self.held.is_empty() || !self.queue.is_empty() || self.hover_to.is_some()
    }

    /// True while a pointer button or finger is down.
    pub fn holding(&self) -> bool {
        !self.held.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Profile; 4] = [Profile::Ink, Profile::Glow, Profile::Sparks, Profile::Neon];

    #[test]
    fn every_look_has_vivid_palettes() {
        for p in ALL {
            for pal in p.look().palettes {
                assert!(pal.len() >= 4, "{p:?} palette too short");
                for c in *pal {
                    let max = c.iter().cloned().fold(0.0, f32::max);
                    assert!(max >= 0.6, "{p:?} colour {c:?} is muddy");
                }
            }
        }
    }

    #[test]
    fn ink_has_two_flavours_others_one() {
        assert_eq!(Profile::Ink.look().palettes.len(), 2);
        assert_eq!(Profile::Ink.look().backdrop.len(), 2);
        for p in [Profile::Glow, Profile::Sparks, Profile::Neon] {
            assert_eq!(p.look().palettes.len(), 1, "{p:?}");
        }
    }

    #[test]
    fn dye_stays_visible_through_the_drift() {
        for p in ALL {
            assert!(p.look().dye_half_life >= 20.0, "{p:?}");
        }
    }

    #[test]
    fn drift_outlasts_a_minute() {
        for p in ALL {
            assert!(p.look().velocity_half_life >= 8.0, "{p:?}");
        }
    }

    #[test]
    fn level_holds_through_the_drift() {
        assert_eq!(level(0.0), 1.0);
        assert_eq!(level(59.9), 1.0);
    }

    #[test]
    fn level_falls_smoothly() {
        let mid = level(DRIFT_S + FADE_S / 2.0);
        assert!((0.4..0.6).contains(&mid), "level at midpoint {mid}");
        let samples: Vec<f32> = (60..=120).map(|s| level(s as f32)).collect();
        for w in samples.windows(2) {
            assert!(w[1] <= w[0], "level rose from {} to {}", w[0], w[1]);
        }
    }

    #[test]
    fn the_shown_level_rises_over_rise_s_and_falls_at_once() {
        assert_eq!(shown_level(0.0, 1.0, 0.0), 0.0);
        assert!((shown_level(0.0, 1.0, RISE_S / 2.0) - 0.5).abs() < 1e-6);
        assert_eq!(shown_level(0.5, 1.0, RISE_S), 1.0, "stops at the target");
        assert_eq!(shown_level(0.2, 0.6, RISE_S), 0.6);
        assert_eq!(shown_level(1.0, 0.7, 0.0), 0.7, "a fade shows at once");
    }

    #[test]
    fn faded_after_two_minutes() {
        assert!(!faded(119.9));
        assert!(faded(120.0));
        assert!(faded(500.0));
    }

    fn forces(h: &mut Hands, ms: u32) -> Vec<Force> {
        h.frame(ms)
    }

    type Splat<'a> = (
        &'a (f32, f32),
        &'a (f32, f32),
        f32,
        [f32; 3],
        f32,
        (f32, f32),
    );

    /// Each splat's (at, vel, radius, colour, amount, from).
    fn splats(f: &[Force]) -> Vec<Splat<'_>> {
        f.iter()
            .filter_map(|f| match f {
                Force::Splat {
                    from,
                    at,
                    vel,
                    radius,
                    colour,
                    amount,
                } => Some((at, vel, *radius, *colour, *amount, *from)),
                _ => None,
            })
            .collect()
    }

    fn hands() -> Hands {
        Hands::new(Profile::Neon, 0)
    }

    #[test]
    fn hands_drag_emits_splats_along_path() {
        let mut h = hands();
        h.down(Source::Pointer, (0.0, 0.0), 0);
        h.moved(Source::Pointer, (100.0, 0.0), 100);
        let f = forces(&mut h, 100);
        let s = splats(&f);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].5, *s[0].0), ((0.0, 0.0), (100.0, 0.0)));
        assert!((s[0].1.0 - 1000.0).abs() < 1.0, "vel.x {}", s[0].1.0);
        assert_eq!(s[0].1.1, 0.0);
    }

    #[test]
    fn a_frame_merges_a_sources_motion_into_one_splat() {
        let mut h = hands();
        h.down(Source::Finger(0), (0.0, 0.0), 0);
        for i in 1..=16u32 {
            h.moved(Source::Finger(0), (i as f32 * 5.0, (i % 2) as f32), i);
        }
        let f = forces(&mut h, 16);
        let s = splats(&f);
        assert_eq!(s.len(), 1, "16 events, one splat");
        assert_eq!((s[0].5, *s[0].0), ((0.0, 0.0), (80.0, 0.0)));
        assert!((s[0].1.0 - 5000.0).abs() < 1.0, "vel.x {}", s[0].1.0);
        h.moved(Source::Finger(0), (90.0, 0.0), 20);
        let f = forces(&mut h, 20);
        let s = splats(&f);
        assert_eq!(
            (s[0].5, *s[0].0),
            ((80.0, 0.0), (90.0, 0.0)),
            "the next frame goes on"
        );
    }

    #[test]
    fn ten_fingers_at_a_thousand_hertz_make_ten_splats_a_frame() {
        let mut h = hands();
        for i in 0..10u32 {
            h.down(Source::Finger(i), (i as f32 * 50.0, 0.0), 0);
        }
        for ms in 1..=16u32 {
            for i in 0..10u32 {
                h.moved(Source::Finger(i), (i as f32 * 50.0, ms as f32), ms);
            }
        }
        assert_eq!(splats(&forces(&mut h, 16)).len(), 10);
    }

    #[test]
    fn lifting_a_finger_keeps_its_last_motion() {
        let mut h = hands();
        h.down(Source::Finger(0), (0.0, 0.0), 0);
        h.moved(Source::Finger(0), (300.0, 0.0), 300);
        h.up(Source::Finger(0), 400);
        let f = forces(&mut h, 400);
        let s = splats(&f);
        assert_eq!(s.len(), 1);
        assert_eq!(*s[0].0, (300.0, 0.0));
    }

    #[test]
    fn hands_ten_fingers_ten_colours() {
        let mut h = hands();
        for i in 0..10u32 {
            h.down(Source::Finger(i), (i as f32 * 50.0, 0.0), 0);
        }
        for i in 0..10u32 {
            h.moved(Source::Finger(i), (i as f32 * 50.0, 100.0), 100);
        }
        let f = forces(&mut h, 100);
        let s = splats(&f);
        assert_eq!(s.len(), 10);
        for w in s.windows(2) {
            assert_ne!(w[0].3, w[1].3);
        }
    }

    #[test]
    fn hands_pointer_colour_drifts_and_advances_per_press() {
        let mut h = hands();
        h.down(Source::Pointer, (0.0, 0.0), 0);
        h.moved(Source::Pointer, (1.0, 0.0), 10);
        let first = splats(&forces(&mut h, 10))[0].3;
        h.moved(Source::Pointer, (1.0 + FEEL.mouse_drift_px, 0.0), 20);
        let later = splats(&forces(&mut h, 20))[0].3;
        assert_ne!(first, later);
        h.up(Source::Pointer, 30);
        forces(&mut h, 30);
        h.down(Source::Pointer, (0.0, 0.0), 1000);
        h.moved(Source::Pointer, (1.0, 0.0), 1010);
        let second = splats(&forces(&mut h, 1010))[0].3;
        assert_ne!(first, second);
    }

    #[test]
    fn hands_tap_bursts() {
        let mut h = hands();
        h.down(Source::Finger(0), (10.0, 10.0), 0);
        h.up(Source::Finger(0), 100);
        let f = forces(&mut h, 100);
        let s = splats(&f);
        assert_eq!(s.len(), 1);
        assert_eq!(*s[0].1, (0.0, 0.0));
        assert_eq!(s[0].2, 3.0);
    }

    #[test]
    fn hands_hold_pours_each_frame() {
        let mut h = hands();
        h.down(Source::Finger(0), (10.0, 10.0), 0);
        assert!(forces(&mut h, FEEL.hold_ms - 1).is_empty());
        let mut n = 0;
        for i in 0..4 {
            n += splats(&forces(&mut h, FEEL.hold_ms + i * 16)).len();
        }
        assert_eq!(n, 3, "one a frame once the hold has lasted");
    }

    /// The dye a stroke lays in one second along the same path, with motion events at `event_hz`
    /// and frames at `frame_hz`.
    fn dye_in_a_second(event_hz: u32, frame_hz: u32) -> f32 {
        let mut h = hands();
        h.down(Source::Finger(0), (0.0, 0.0), 0);
        let (mut total, mut next_frame) = (0.0, 0.0);
        for ms in 1..=1000u32 {
            if ms * event_hz % 1000 < event_hz {
                h.moved(Source::Finger(0), (ms as f32 * 0.5, 0.0), ms);
            }
            if ms as f32 >= next_frame {
                next_frame += 1000.0 / frame_hz as f32;
                total += splats(&forces(&mut h, ms)).iter().map(|s| s.4).sum::<f32>();
            }
        }
        total
    }

    #[test]
    fn a_strokes_dye_follows_its_time_not_its_events() {
        let mouse = dye_in_a_second(60, 60);
        assert!(
            (mouse - FEEL.stroke_dye).abs() < 0.05 * FEEL.stroke_dye,
            "{mouse}"
        );
        for (event_hz, frame_hz) in [(1000, 60), (120, 60), (1000, 144), (60, 144)] {
            let got = dye_in_a_second(event_hz, frame_hz);
            assert!(
                (got - mouse).abs() < 0.05 * mouse,
                "{event_hz} Hz events, {frame_hz} Hz frames: {got} against {mouse}"
            );
        }
    }

    /// Dye at `d` look splat radii from the centre of `splats` all put at one point.
    fn dye_at(splats: &[Splat<'_>], d: f32) -> f32 {
        splats
            .iter()
            .map(|s| s.4 * (-(d * d) / (s.2 * s.2)).exp())
            .sum()
    }

    /// How far out, in look splat radii, `splats` lay at least `dye`.
    fn reach(splats: &[Splat<'_>], dye: f32) -> f32 {
        (0..400)
            .map(|i| i as f32 * 0.025)
            .take_while(|&d| dye_at(splats, d) >= dye)
            .last()
            .unwrap_or(0.0)
    }

    #[test]
    fn a_second_held_pours_a_blob_as_big_as_a_tap() {
        let mut h = hands();
        h.down(Source::Finger(0), (10.0, 10.0), 0);
        h.up(Source::Finger(0), 60);
        let tap = forces(&mut h, 60);
        let mut h = hands();
        h.down(Source::Finger(1), (10.0, 10.0), 1000);
        let mut poured = Vec::new();
        for ms in (1000..=2000).step_by(16) {
            poured.extend(forces(&mut h, ms));
        }
        for dye in [0.1, 0.3] {
            let (t, p) = (reach(&splats(&tap), dye), reach(&splats(&poured), dye));
            assert!(p >= t, "dye {dye}: the pour reaches {p} radii, the tap {t}");
        }
    }

    #[test]
    fn hands_flick_boosts_velocity() {
        let mut h = hands();
        h.down(Source::Finger(0), (0.0, 0.0), 0);
        h.moved(Source::Finger(0), (100.0, 0.0), 40);
        forces(&mut h, 40);
        h.up(Source::Finger(0), 50);
        let f = forces(&mut h, 50);
        let s = splats(&f);
        assert_eq!(s.len(), 1);
        assert!(
            (s[0].1.0 - 2500.0 * FEEL.flick_boost).abs() < 1.0,
            "{}",
            s[0].1.0
        );
    }

    #[test]
    fn a_long_drag_keeps_only_the_flick_window_and_one_point_before() {
        let mut h = hands();
        h.down(Source::Finger(0), (0.0, 0.0), 0);
        for t in 1..=1000 {
            h.moved(Source::Finger(0), (t as f32 * 20.0, 0.0), t * 10);
            forces(&mut h, t * 10);
        }
        let kept = &h.held[0].samples;
        let window = (FEEL.flick_window_ms / 10 + 1) as usize;
        assert_eq!(kept.len(), window + 1, "{kept:?}");
        h.up(Source::Finger(0), 10_000);
        let f = forces(&mut h, 10_000);
        let s = splats(&f);
        assert_eq!(s.len(), 1, "the flick still fires");
        assert!(
            (s[0].1.0 - 2000.0 * FEEL.flick_boost).abs() < 1.0,
            "{}",
            s[0].1.0
        );
    }

    #[test]
    fn holding_lasts_from_down_to_up() {
        let mut h = hands();
        assert!(!h.holding());
        h.down(Source::Pointer, (0.0, 0.0), 0);
        forces(&mut h, 2000);
        assert!(h.holding(), "a still hold");
        h.up(Source::Pointer, 2100);
        assert!(!h.holding());
    }

    #[test]
    fn hands_twist_swirls_with_sign() {
        let mut h = hands();
        h.down(Source::Finger(0), (-50.0, 0.0), 0);
        h.down(Source::Finger(1), (50.0, 0.0), 0);
        h.moved(Source::Finger(0), (-45.0, -30.0), 100);
        h.moved(Source::Finger(1), (45.0, 30.0), 100);
        let f = forces(&mut h, 100);
        let sw: Vec<_> = f
            .iter()
            .filter_map(|f| match f {
                Force::Swirl {
                    centre,
                    radius,
                    spin,
                } => Some((*centre, *radius, *spin)),
                _ => None,
            })
            .collect();
        assert_eq!(sw.len(), 1);
        assert!(sw[0].2 < 0.0, "spin {}", sw[0].2);
        assert!((sw[0].1 - 0.5 * (90.0f32.powi(2) + 60.0f32.powi(2)).sqrt()).abs() < 0.01);
        assert!(sw[0].0.0.abs() < 0.01 && sw[0].0.1.abs() < 0.01);
    }

    #[test]
    fn hands_no_swirl_below_twist_threshold() {
        let mut h = hands();
        h.down(Source::Finger(0), (-50.0, 0.0), 0);
        h.down(Source::Finger(1), (50.0, 0.0), 0);
        h.moved(Source::Finger(1), (50.0, 5.0), 100);
        let f = forces(&mut h, 100);
        assert!(f.iter().all(|f| !matches!(f, Force::Swirl { .. })));
    }

    #[test]
    fn hands_double_tap_clears() {
        let mut h = hands();
        h.down(Source::Finger(0), (10.0, 10.0), 0);
        h.up(Source::Finger(0), 50);
        h.down(Source::Finger(1), (15.0, 12.0), 150);
        h.up(Source::Finger(1), 200);
        let f = forces(&mut h, 200);
        assert_eq!(f.iter().filter(|f| matches!(f, Force::Clear)).count(), 1);
        assert_eq!(splats(&f).len(), 1, "only the first burst remains");
    }

    #[test]
    fn hands_hover_velocity_is_px_per_second() {
        let mut h = hands();
        h.hover((0.0, 0.0), 0);
        h.hover((50.0, 0.0), 50);
        h.hover((100.0, 0.0), 100);
        let f = forces(&mut h, 100);
        let s = splats(&f);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].5, *s[0].0), ((0.0, 0.0), (100.0, 0.0)));
        assert!((s[0].1.0 - 1000.0).abs() < 1.0, "vel.x {}", s[0].1.0);
    }

    #[test]
    fn hands_same_timestamp_motion_has_no_absurd_velocity() {
        let mut h = hands();
        h.down(Source::Pointer, (0.0, 0.0), 0);
        h.moved(Source::Pointer, (50.0, 0.0), 0);
        h.moved(Source::Pointer, (100.0, 0.0), 100);
        h.hover((0.0, 0.0), 200);
        h.hover((50.0, 0.0), 200);
        let f = forces(&mut h, 200);
        for s in splats(&f) {
            assert!(s.1.0.abs() <= 1000.0 + 1.0, "vel.x {}", s.1.0);
        }
    }

    #[test]
    fn hands_hover_is_faint() {
        let mut h = hands();
        h.hover((5.0, 5.0), 0);
        h.hover((25.0, 5.0), 16);
        let f = forces(&mut h, 16);
        let s = splats(&f);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].4, FEEL.hover_dye * 0.016);
        assert!(!h.active());
    }

    #[test]
    fn set_flavour_keeps_the_held_stroke_in_the_new_palette() {
        let mut h = Hands::new(Profile::Ink, 0);
        h.down(Source::Pointer, (0.0, 0.0), 0);
        h.set_flavour(1);
        h.moved(Source::Pointer, (10.0, 0.0), 10);
        let f = forces(&mut h, 10);
        let s = splats(&f);
        assert_eq!(s.len(), 1, "the held stroke goes on");
        let dark = Profile::Ink.look().palettes[1];
        assert_eq!(s[0].3, at_position(dark, 10.0 / FEEL.mouse_drift_px));
        assert!(h.active());
    }

    #[test]
    fn clear_queues_one_clear() {
        let mut h = hands();
        h.clear(500);
        assert!(h.active());
        assert!(matches!(forces(&mut h, 500)[..], [Force::Clear]));
    }

    #[test]
    fn a_tap_after_clear_bursts_without_pairing_with_the_tap_before() {
        let mut h = hands();
        h.down(Source::Pointer, (10.0, 10.0), 0);
        h.up(Source::Pointer, 50);
        h.clear(100);
        h.down(Source::Pointer, (10.0, 10.0), 150);
        h.up(Source::Pointer, 200);
        let f = forces(&mut h, 200);
        assert_eq!(f.iter().filter(|f| matches!(f, Force::Clear)).count(), 1);
        assert_eq!(splats(&f).len(), 2, "both taps burst");
    }

    #[test]
    fn cancel_fires_nothing() {
        let mut h = hands();
        h.down(Source::Finger(0), (0.0, 0.0), 0);
        h.moved(Source::Finger(0), (200.0, 0.0), 40);
        h.cancel(Source::Finger(0));
        assert!(forces(&mut h, 50).is_empty());
        h.down(Source::Finger(1), (0.0, 0.0), 100);
        h.cancel(Source::Finger(1));
        h.up(Source::Finger(1), 150);
        assert!(forces(&mut h, 150).is_empty());
    }
}
