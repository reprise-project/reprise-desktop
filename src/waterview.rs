//! Desktop water's glue: a field per output and lock cover, stepped before each frame.
//!
//! A field steps only while it moves: once its [`Energy`] is idle it is flattened and left alone.
//! A covered desktop pane (fullscreen, lock, saver) steps nothing and gets no rain. A pane whose
//! GL work failed draws nothing and its wallpaper buffers show again. Fluid styles stir desktops.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reprisefx::fluid::{FluidFx, FluidPool};
use reprisefx::water::{Pool, WaterFx};
use reprisekit::fluid::{Force, Hands, Profile, Source};
use reprisekit::water::{
    Clock, Energy, Preset, Queue, REFERENCE_HZ, Rain, Rate, SPLASHES_PER_STEP, STIRRED_SECS,
    Splash, Step, Water, field_point, field_size, rain_steps_per_frame,
};
use reprisescene::{BufferId, CustomElement, CustomId, CustomNode, Input, TreeId};
use smithay::{
    backend::allocator::Fourcc,
    backend::renderer::{
        ImportMem,
        element::Id,
        gles::{GlesError, GlesRenderer, GlesTexture},
        utils::CommitCounter,
    },
    output::Output,
    utils::{Logical, Physical, Point, Rectangle, Scale, Size},
};

use crate::compositor::State;
use crate::dragview::DragSource;
use crate::input::{Hit, Interaction};
use crate::raster::{Canvas, Rect};
use crate::theme::Color;

/// How far the normals pass amplifies the field's slopes before they are cut to 8 bits.
const SLOPE_GAIN: f32 = 6.0;

/// Which picture a field ripples: an output's desktop or its lock cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Pane {
    Desktop,
    Lock,
}

type Key = (String, Pane);

/// Every pane's field and picture, and the renderer's programs.
#[derive(Default)]
pub struct WaterCtx {
    /// None before the backend starts or when the programs did not compile: every pane then fails
    /// and the plain wallpaper shows.
    fx: RefCell<Option<WaterFx>>,
    /// As `fx`, for the fluid styles' panes.
    fluid_fx: RefCell<Option<FluidFx>>,
    panes: RefCell<HashMap<Key, PaneState>>,
    style: Cell<Water>,
    /// The output whose desktop pane each held source stirs.
    stirs: RefCell<HashMap<Source, String>>,
    rate: Cell<Rate>,
    /// A pane failed since `water_frame` last looked: its buffers show again.
    fell: Cell<bool>,
    /// The dragged window's moves since the last frame.
    edge: Cell<EdgeDrag>,
}

/// What a pane runs: ripples, or a fluid style's sim and the hands stirring it.
enum Sim {
    Ripples(Pool),
    Fluid {
        pool: Box<FluidPool>,
        hands: Hands,
        /// Ink's light (0) or dark (1) look; 0 for the other profiles.
        flavour: usize,
        last_input: Option<Instant>,
    },
}

impl Sim {
    fn for_pane(style: Water, pane: Pane) -> Sim {
        match style.fluid().filter(|_| pane == Pane::Desktop) {
            Some(profile) => Sim::Fluid {
                pool: Box::new(FluidPool::new()),
                hands: Hands::new(profile, 0),
                flavour: 0,
                last_input: None,
            },
            None => Sim::Ripples(Pool::new()),
        }
    }
}

/// The flavour a pane starting from empty takes: Ink alternates, the others have one.
fn next_flavour(profile: Profile, flavour: usize) -> usize {
    match profile {
        Profile::Ink => 1 - flavour.min(1),
        _ => 0,
    }
}

struct PaneState {
    sim: Sim,
    id: Id,
    prev_id: Id,
    pending: Option<Canvas>,
    texture: Option<GlesTexture>,
    previous: Option<GlesTexture>,
    fade: f32,
    /// Bumped for every change of picture or fade, never reset: the elements' commit adds it to the
    /// field's.
    wall_commit: CommitCounter,
    solid: Option<Color>,
    queue: Queue,
    energy: Energy,
    clock: Clock,
    rain: Rain,
    obstacles: Vec<Rectangle<i32, Physical>>,
    /// Covered as of the last `prepare`: frames drawn meanwhile step nothing.
    covered: bool,
    /// The last step; None steps 0 s next (just made, or uncovered).
    last: Option<Instant>,
    /// The step rate of the last step.
    hz: f32,
    /// When the last splash other than rain was queued.
    stirred: Option<Instant>,
    running: bool,
    size: Size<i32, Physical>,
    node: Option<CustomId>,
    node_size: Size<i32, Logical>,
    failed: bool,
    /// The fluid's clock start for `Hands` and the shaders: when the pane was made or last faded.
    epoch: Instant,
    /// The level shown as of the last step (`reprisekit::fluid::shown_level`).
    level: f32,
}

impl PaneState {
    fn new(seed: u32, sim: Sim) -> PaneState {
        PaneState {
            sim,
            id: Id::new(),
            prev_id: Id::new(),
            pending: None,
            texture: None,
            previous: None,
            fade: 1.0,
            wall_commit: CommitCounter::default(),
            solid: None,
            queue: Queue::default(),
            energy: Energy::default(),
            clock: Clock::default(),
            rain: Rain::new(seed),
            obstacles: Vec::new(),
            last: None,
            hz: REFERENCE_HZ,
            stirred: None,
            covered: false,
            running: false,
            size: Size::default(),
            node: None,
            node_size: Size::default(),
            failed: false,
            epoch: Instant::now(),
            level: 0.0,
        }
    }

    fn bump(&mut self) {
        self.wall_commit.increment();
    }

    fn import_pending(&mut self, r: &mut GlesRenderer) -> Result<(), GlesError> {
        if let Some(c) = self.pending.take() {
            self.texture = Some(import(r, &c)?);
            self.bump();
        }
        Ok(())
    }

    /// The steps at `hz` that `now` is due; none while stopped.
    fn due_steps(&mut self, hz: f32, now: Instant) -> u32 {
        let dt = self
            .last
            .map_or(0.0, |l| now.saturating_duration_since(l).as_secs_f32());
        self.last = Some(now);
        if self.running {
            self.clock.steps(dt, hz)
        } else {
            0
        }
    }

    /// As `due_steps`, without catching up on late frames (`Clock::steps_no_catch_up`).
    fn due_steps_no_catch_up(&mut self, hz: f32, refresh: f32, now: Instant) -> u32 {
        let dt = self
            .last
            .map_or(0.0, |l| now.saturating_duration_since(l).as_secs_f32());
        self.last = Some(now);
        if self.running {
            self.clock.steps_no_catch_up(dt, hz, refresh)
        } else {
            0
        }
    }

    /// Ms since `epoch`, the clock `Hands` runs on.
    fn ms(&self, now: Instant) -> u32 {
        now.saturating_duration_since(self.epoch).as_millis() as u32
    }

    /// Import a pending picture and, unless covered, run the steps at `hz` that `now` is due and
    /// remake the normals when the field moved.
    fn advance(
        &mut self,
        r: &mut GlesRenderer,
        fx: &WaterFx,
        preset: &Preset,
        hz: f32,
        rains: bool,
        now: Instant,
    ) -> Result<(), GlesError> {
        self.import_pending(r)?;
        let Sim::Ripples(pool) = &mut self.sim else {
            return Ok(());
        };
        let (w, h) = field_size((self.size.w, self.size.h));
        pool.fit(r, fx, (w, h).into())?;
        if self.covered {
            return Ok(());
        }
        let n = self.due_steps(hz, now);
        let Sim::Ripples(pool) = &mut self.sim else {
            return Ok(());
        };
        let step = Step::new(preset.damping, hz);
        for _ in 0..n {
            let splashes: Vec<Splash> = self
                .queue
                .take(SPLASHES_PER_STEP, hz)
                .into_iter()
                .map(|s| {
                    self.energy.add(&s);
                    Splash {
                        depth: s.depth * preset.gain,
                        ..s
                    }
                })
                .collect();
            pool.step(r, fx, &splashes, &self.obstacles, step)?;
            self.energy.step(step.damping);
        }
        if n > 0 {
            pool.shade(r, fx, SLOPE_GAIN)?;
        }
        if self.running && self.energy.idle() && self.queue.is_empty() && !rains {
            pool.flatten(r, fx)?;
            self.running = false;
        }
        Ok(())
    }

    /// As `advance`, for a fluid pane: the due steps at `hz` on `refresh` Hz frames, no catch-up,
    /// the hands' forces in the first, a shade after them, and once faded a cleared, stopped pane.
    fn advance_fluid(
        &mut self,
        r: &mut GlesRenderer,
        fx: &FluidFx,
        profile: Profile,
        (hz, refresh): (f32, f32),
        scale: f64,
        now: Instant,
    ) -> Result<(), GlesError> {
        self.import_pending(r)?;
        let Sim::Fluid { pool, .. } = &mut self.sim else {
            return Ok(());
        };
        pool.fit(r, fx, self.size)?;
        if self.covered {
            return Ok(());
        }
        let dt = self
            .last
            .map_or(0.0, |l| now.saturating_duration_since(l).as_secs_f32());
        let n = self.due_steps_no_catch_up(hz, refresh, now);
        let ms = self.ms(now);
        let time_s = now.saturating_duration_since(self.epoch).as_secs_f32();
        let Sim::Fluid {
            pool,
            hands,
            flavour,
            last_input,
        } = &mut self.sim
        else {
            return Ok(());
        };
        let look = profile.look();
        let mut cleared = false;
        for i in 0..n {
            let forces = if i == 0 { hands.frame(ms) } else { Vec::new() };
            cleared |= forces.contains(&Force::Clear);
            pool.step(r, fx, &forces, scale, look, 1.0 / hz)?;
        }
        if cleared {
            *flavour = next_flavour(profile, *flavour);
            hands.set_flavour(*flavour);
        }
        if n > 0 {
            pool.shade(r, fx, profile, *flavour, time_s)?;
        }
        let idle = idle_s(last_input, hands.holding(), now);
        let level = if self.running && !hands.active() && reprisekit::fluid::faded(idle) {
            pool.clear(r, fx)?;
            *flavour = next_flavour(profile, *flavour);
            *hands = Hands::new(profile, *flavour);
            *last_input = None;
            self.epoch = now;
            self.running = false;
            0.0
        } else if self.running {
            reprisekit::fluid::shown_level(self.level, reprisekit::fluid::level(idle), dt)
        } else {
            self.level
        };
        // Damage follows the commit: a fade between steps must still redraw.
        if level != self.level {
            self.level = level;
            self.bump();
        }
        Ok(())
    }
}

/// Idle seconds at `now` since `last_input`; a held source is input now.
fn idle_s(last_input: &mut Option<Instant>, holding: bool, now: Instant) -> f32 {
    if holding {
        *last_input = Some(now);
    }
    last_input.map_or(f32::INFINITY, |t| {
        now.saturating_duration_since(t).as_secs_f32()
    })
}

fn refresh_hz(output: &Output) -> f32 {
    output
        .current_mode()
        .map(|m| m.refresh as f32 / 1000.0)
        .filter(|&hz| hz > 0.0)
        .unwrap_or(REFERENCE_HZ)
}

fn import(r: &mut GlesRenderer, c: &Canvas) -> Result<GlesTexture, GlesError> {
    let (w, h) = c.device_size();
    let bytes: Vec<u8> = c.pixels.iter().flat_map(|p| p.to_le_bytes()).collect();
    r.import_memory(&bytes, Fourcc::Argb8888, (w as i32, h as i32).into(), false)
}

fn seed(n: usize) -> u32 {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    t ^ (n as u32).wrapping_mul(0x9E37_79B9)
}

impl WaterCtx {
    pub fn new() -> Rc<WaterCtx> {
        Rc::new(WaterCtx::default())
    }

    /// Compile the programs on `renderer`, the one every output draws with.
    pub fn set_renderer(&self, renderer: &mut GlesRenderer) {
        let fx = WaterFx::new(renderer)
            .map_err(|e| log::warn!("water: no ripples, the shaders did not compile: {e}"))
            .ok();
        *self.fx.borrow_mut() = fx;
        let fluid = FluidFx::new(renderer)
            .map_err(|e| match e {
                GlesError::GLExtensionNotSupported(needs) => {
                    log::warn!("water: no fluid, the GPU lacks {}", needs.join(", "))
                }
                e => log::warn!("water: no fluid, its GL setup failed: {e}"),
            })
            .ok();
        *self.fluid_fx.borrow_mut() = fluid;
    }

    /// Store `style` and keep a desktop pane per desktop key, dropping the rest (all of them on a
    /// fluid or profile change, lock panes too when off). Returns the dropped nodes.
    pub fn configure(
        &self,
        style: Water,
        desktops: impl Iterator<Item = (String, Pane)>,
    ) -> Vec<CustomId> {
        let rebuild = self.style.replace(style).fluid() != style.fluid();
        if rebuild {
            self.stirs.borrow_mut().clear();
        }
        let mut panes = self.panes.borrow_mut();
        if style == Water::Off {
            return panes.drain().filter_map(|(_, p)| p.node).collect();
        }
        let keep: Vec<Key> = desktops.filter(|(_, p)| *p == Pane::Desktop).collect();
        let mut dropped = Vec::new();
        panes.retain(|k, p| {
            let stays = k.1 != Pane::Desktop || (keep.contains(k) && !rebuild);
            if !stays {
                dropped.extend(p.node);
            }
            stays
        });
        for k in keep {
            let n = panes.len();
            let sim = Sim::for_pane(style, k.1);
            panes
                .entry(k)
                .or_insert_with(|| PaneState::new(seed(n), sim));
        }
        dropped
    }

    /// `[wallpaper] water` as of the last `configure`.
    pub fn style(&self) -> Water {
        self.style.get()
    }

    /// Make `pane` of `output_name` unless it exists. The lock screen's way in: `configure` makes
    /// desktop panes only.
    pub fn add_pane(&self, output_name: &str, pane: Pane) {
        let mut panes = self.panes.borrow_mut();
        let n = panes.len();
        let sim = Sim::for_pane(self.style.get(), pane);
        panes
            .entry((output_name.to_string(), pane))
            .or_insert_with(|| PaneState::new(seed(n), sim));
    }

    /// Drop `pane` of `output_name`; returns its node, if it had one.
    pub fn drop_pane(&self, output_name: &str, pane: Pane) -> Option<CustomId> {
        self.panes
            .borrow_mut()
            .remove(&(output_name.to_string(), pane))?
            .node
    }

    /// Store `[wallpaper] water_rate`; the next step runs at it.
    pub fn set_rate(&self, rate: Rate) {
        self.rate.set(rate);
    }

    /// The ripples' preset of `pane`: the lock screen ripples with Calm's damping under any style,
    /// and as Calm under a fluid one. None while off, and for a fluid desktop.
    fn preset_for(&self, pane: Pane) -> Option<Preset> {
        let style = self.style.get();
        let calm = Water::Calm.preset().expect("Calm has a preset");
        match pane {
            Pane::Desktop => style.preset(),
            Pane::Lock if style == Water::Off => None,
            Pane::Lock => Some(Preset {
                damping: calm.damping,
                ..style.preset().unwrap_or(calm)
            }),
        }
    }

    /// Every pane's key, sorted.
    pub fn pane_keys(&self) -> Vec<Key> {
        let mut keys: Vec<Key> = self.panes.borrow().keys().cloned().collect();
        keys.sort();
        keys
    }

    /// Lay `s` into the field of `pane` on `output_name` at its next step.
    pub fn splash(&self, output_name: &str, pane: Pane, s: Splash) {
        self.queue(output_name, pane, |q| q.push(s));
    }

    /// Lay a dragged window's edge `s` for this frame into the desktop field of `output_name`,
    /// adding to the frames' since its last step.
    pub fn splash_edge(&self, output_name: &str, s: Splash) {
        self.queue(output_name, Pane::Desktop, |q| q.push_edge(s));
    }

    fn queue(&self, output_name: &str, pane: Pane, push: impl FnOnce(&mut Queue)) {
        if let Some(p) = self
            .panes
            .borrow_mut()
            .get_mut(&(output_name.to_string(), pane))
            .filter(|p| matches!(p.sim, Sim::Ripples(_)))
        {
            if !p.running {
                p.last = None;
            }
            p.stirred = Some(Instant::now());
            push(&mut p.queue);
            p.running = true;
        }
    }

    fn takes_stir(&self, output_name: &str) -> bool {
        self.fluid_fx.borrow().is_some()
            && self
                .panes
                .borrow()
                .get(&(output_name.to_string(), Pane::Desktop))
                .is_some_and(|p| !p.covered && !p.failed && matches!(p.sim, Sim::Fluid { .. }))
    }

    /// Feed the hands of `output_name`'s fluid desktop pane at ms on their clock (`input` restarts
    /// a stopped pane). False, dropping the input, without a live uncovered pane.
    fn stir(&self, output_name: &str, input: bool, feed: impl FnOnce(&mut Hands, u32)) -> bool {
        let mut panes = self.panes.borrow_mut();
        let Some(p) = panes
            .get_mut(&(output_name.to_string(), Pane::Desktop))
            .filter(|p| !p.covered && !p.failed)
        else {
            return false;
        };
        let now = Instant::now();
        let ms = p.ms(now);
        let Sim::Fluid {
            hands, last_input, ..
        } = &mut p.sim
        else {
            return false;
        };
        feed(hands, ms);
        if input {
            *last_input = Some(now);
            if !p.running {
                p.last = None;
                p.running = true;
            }
        }
        true
    }

    /// Hold `field_rects` (field texels) flat in the desktop field of `output_name`.
    pub fn set_obstacles(&self, output_name: &str, field_rects: Vec<Rectangle<i32, Physical>>) {
        if let Some(p) = self
            .panes
            .borrow_mut()
            .get_mut(&(output_name.to_string(), Pane::Desktop))
        {
            p.obstacles = field_rects;
        }
    }

    /// Before `output`'s frame: note whether its desktop is `covered` and lay Rain's due drops on an
    /// uncovered desktop pane.
    pub fn prepare(&self, output: &Output, covered: bool, now: Instant) {
        self.prepare_named(&output.name(), covered, now);
    }

    fn prepare_named(&self, name: &str, covered: bool, now: Instant) {
        let rain = self.style.get().preset().and_then(|p| p.rain);
        for ((n, pane), p) in self.panes.borrow_mut().iter_mut() {
            if n != name || p.failed {
                continue;
            }
            p.covered = *pane == Pane::Desktop && covered;
            if p.covered {
                p.last = None;
                if let Sim::Fluid { hands, .. } = &mut p.sim {
                    hands.cancel_all();
                }
                continue;
            }
            let rains = *pane == Pane::Desktop && rain.is_some();
            // Until the first step sizes it a drop would land at 0,0.
            if let Some(range) = rain.filter(|_| rains && !p.size.is_empty()) {
                let dt = p
                    .last
                    .map_or(0.0, |l| now.saturating_duration_since(l).as_secs_f32());
                let field = field_size((p.size.w, p.size.h));
                if let Some(s) = p.rain.tick(dt, range, field) {
                    p.queue.push(s);
                    p.running = true;
                }
            }
        }
    }

    /// When a pane of `output` next changes: a fade at once, a moving field at its next step, rain
    /// that nothing else stirred at its next [`reprisekit::water::RAIN_FPS`] frame. None while every
    /// pane is still, covered or failed.
    pub fn next_step(&self, output: &Output) -> Option<Instant> {
        self.next_step_named(&output.name(), Instant::now())
    }

    fn next_step_named(&self, name: &str, now: Instant) -> Option<Instant> {
        let rain = self.style.get().preset().is_some_and(|p| p.rain.is_some());
        self.panes
            .borrow()
            .iter()
            .filter(|((n, _), p)| n == name && !p.failed && !p.covered)
            .filter_map(|((_, pane), p)| {
                if p.pending.is_some() || (p.previous.is_some() && p.fade < 1.0) {
                    return Some(now);
                }
                let rains = *pane == Pane::Desktop && rain;
                if !(p.running || rains || !p.queue.is_empty()) {
                    return None;
                }
                let calm = p
                    .stirred
                    .is_none_or(|t| now.saturating_duration_since(t).as_secs_f32() >= STIRRED_SECS);
                let steps = if rains && calm {
                    rain_steps_per_frame(p.hz)
                } else {
                    1
                };
                let last = p.last.unwrap_or(now);
                Some(last + Duration::from_secs_f32(p.clock.until_steps(steps, p.hz)))
            })
            .min()
    }

    /// Step `output`'s fields for the frame about to be built. True when a pane failed: its node
    /// draws nothing from now on.
    pub fn step(&self, renderer: &mut GlesRenderer, output: &Output) -> bool {
        let style = self.style.get();
        if style == Water::Off {
            return false;
        }
        let size = output
            .current_mode()
            .map(|m| output.current_transform().transform_size(m.size))
            .unwrap_or_default();
        let name = output.name();
        let refresh = refresh_hz(output);
        let hz = self.rate.get().hz(refresh);
        let now = Instant::now();
        let scale = output.current_scale().fractional_scale();
        let fx = self.fx.borrow();
        let fluid_fx = self.fluid_fx.borrow();
        let mut fell = false;
        for ((n, pane), p) in self.panes.borrow_mut().iter_mut() {
            if *n != name || p.failed {
                continue;
            }
            p.size = size;
            p.hz = hz;
            let done = match (&p.sim, style.fluid(), self.preset_for(*pane)) {
                (Sim::Fluid { .. }, Some(profile), _) => fluid_fx
                    .as_ref()
                    .map(|fx| p.advance_fluid(renderer, fx, profile, (hz, refresh), scale, now)),
                (Sim::Ripples(_), _, Some(preset)) => fx.as_ref().map(|fx| {
                    let rains = *pane == Pane::Desktop && preset.rain.is_some();
                    p.advance(renderer, fx, &preset, hz, rains, now)
                }),
                _ => None,
            };
            let Some(done) = done else {
                p.failed = true;
                fell = true;
                continue;
            };
            if let Err(e) = done {
                log::warn!("water: {e}");
                p.failed = true;
                fell = true;
            }
        }
        if fell {
            self.fell.set(true);
        }
        fell
    }
}

/// A pane's scene node: the wallpaper through its field, sized to the output, never taking the
/// pointer.
pub struct WaterNode {
    ctx: Rc<WaterCtx>,
    key: Key,
    size: Size<i32, Logical>,
}

impl WaterNode {
    pub fn new(ctx: &Rc<WaterCtx>, key: Key, size: Size<i32, Logical>) -> WaterNode {
        WaterNode {
            ctx: ctx.clone(),
            key,
            size,
        }
    }
}

impl CustomNode<GlesRenderer> for WaterNode {
    fn size(&self) -> Size<i32, Logical> {
        self.size
    }

    fn in_backdrop(&self) -> bool {
        true
    }

    fn elements(
        &self,
        _renderer: &mut GlesRenderer,
        location: Point<f64, Physical>,
        scale: Scale<f64>,
        alpha: f32,
    ) -> Vec<CustomElement<GlesRenderer>> {
        let panes = self.ctx.panes.borrow();
        let Some(p) = panes.get(&self.key).filter(|p| !p.failed) else {
            return Vec::new();
        };
        let geometry = Rectangle::new(
            location.to_i32_round(),
            self.size.to_physical_precise_round(scale),
        );
        let pool = match &p.sim {
            Sim::Ripples(pool) => pool,
            Sim::Fluid { pool, flavour, .. } => {
                let fx = self.ctx.fluid_fx.borrow();
                let (Some(fx), Some(profile), Some(t)) =
                    (fx.as_ref(), self.ctx.style.get().fluid(), &p.texture)
                else {
                    return Vec::new();
                };
                let look = profile.look();
                return pool
                    .element(
                        fx,
                        &p.id,
                        t,
                        p.wall_commit,
                        geometry,
                        look,
                        *flavour,
                        p.level,
                    )
                    .map(CustomElement::new)
                    .into_iter()
                    .collect();
            }
        };
        let fx = self.ctx.fx.borrow();
        let (Some(fx), Some(preset)) = (fx.as_ref(), self.ctx.preset_for(self.key.1)) else {
            return Vec::new();
        };
        let element = |id: &Id, t: &GlesTexture, a: f32| {
            pool.element(
                fx,
                id,
                t,
                p.wall_commit,
                geometry,
                preset.refraction * scale.x as f32,
                preset.shine,
                a,
            )
        };
        let fading = p.previous.as_ref().filter(|_| p.fade < 1.0);
        let mut out = Vec::new();
        if let Some(t) = &p.texture {
            let a = if fading.is_some() {
                alpha * p.fade
            } else {
                alpha
            };
            out.extend(element(&p.id, t, a));
        }
        if let Some(t) = fading {
            out.extend(element(&p.prev_id, t, alpha));
        }
        out.into_iter().map(CustomElement::new).collect()
    }
}

impl State {
    /// Whether `[wallpaper] water` is on, rippling or fluid.
    pub fn water_on(&self) -> bool {
        self.config.wallpaper.water != Water::Off
    }

    /// Whether `[wallpaper] water` is a ripple style: windows, icons and clicks splash it.
    pub fn ripples_on(&self) -> bool {
        self.config.wallpaper.water.preset().is_some()
    }

    /// Whether `[wallpaper] water` is a fluid style: the desktop's pointer and fingers stir it.
    pub fn fluid_on(&self) -> bool {
        self.config.wallpaper.water.is_fluid()
    }

    /// Follow `[wallpaper] water` and the outputs: a desktop pane and node per output while on,
    /// none while off, and the wallpaper buffers hidden under every node that draws. Runs on every
    /// `refit_wallpaper` and reload; cheap when nothing changed.
    pub fn water_configured(&mut self) {
        self.water.set_rate(self.config.wallpaper.water_rate);
        let style = self.config.wallpaper.water;
        if self.water.style().fluid() != style.fluid() {
            self.ignore_stirring_fingers();
            if self.interaction == Interaction::Stir {
                self.interaction = Interaction::Idle;
                self.end_marquee();
            }
        }
        if style == Water::Off {
            if self.water.pane_keys().is_empty() {
                return;
            }
            for node in self.water.configure(Water::Off, std::iter::empty()) {
                self.scene.destroy(node);
            }
            self.water_buffers();
            self.redraw_all();
            return;
        }
        let outputs: Vec<Output> = self.scene.outputs().to_vec();
        let keys: Vec<Key> = outputs
            .iter()
            .filter(|o| self.layout_box(Some(o)).width > 0)
            .map(|o| (o.name(), Pane::Desktop))
            .collect();
        let mut changed = false;
        for node in self.water.configure(style, keys.into_iter()) {
            self.scene.destroy(node);
            changed = true;
        }
        let solid = self.wallpaper.as_ref().is_none_or(|w| w.shows_nothing());
        let bg = self.config.theme.colors.background;
        for out in outputs {
            let b = self.layout_box(Some(&out));
            let key = (out.name(), Pane::Desktop);
            let size = Size::from((b.width, b.height));
            let (node, made, unpictured, shows) = {
                let panes = self.water.panes.borrow();
                let Some(p) = panes.get(&key) else {
                    continue;
                };
                let unpictured = p.texture.is_none() && p.pending.is_none();
                (p.node, p.node_size, unpictured, p.solid)
            };
            let node = match node {
                Some(n) if made == size => n,
                old => {
                    if let Some(n) = old {
                        self.scene.destroy(n);
                    }
                    let n = self.scene.custom_create(
                        self.layers.background,
                        Box::new(WaterNode::new(&self.water, key.clone(), size)),
                    );
                    self.scene.set_input(n, Input::Passthrough);
                    self.scene.lower_to_bottom(n);
                    if let Some(p) = self.water.panes.borrow_mut().get_mut(&key) {
                        p.node = Some(n);
                        p.node_size = size;
                    }
                    changed = true;
                    n
                }
            };
            if self.scene.position(node) != Some((b.x, b.y).into()) {
                self.scene.set_position(node, b.x, b.y);
                changed = true;
            }
            let canvas = if solid {
                None
            } else if unpictured {
                let scale = self.draw_scale_at(Some(&out));
                self.wallpaper_canvas(&out, scale)
            } else {
                continue;
            };
            match canvas {
                Some(c) => self.water_wallpaper(&out, Pane::Desktop, &c, false),
                None if shows != Some(bg) => {
                    let mut c = Canvas::new(1, 1);
                    c.fill_rect(c.bounds(), bg);
                    self.water_wallpaper(&out, Pane::Desktop, &c, false);
                    if let Some(p) = self.water.panes.borrow_mut().get_mut(&key) {
                        p.solid = Some(bg);
                    }
                }
                None => continue,
            }
            changed = true;
        }
        self.water_buffers();
        if changed {
            self.water_obstacles();
            self.redraw_all();
        }
    }

    /// Show each output's wallpaper buffer unless its desktop water draws, and the buffers faded
    /// from unless every output's does.
    fn water_buffers(&mut self) {
        let Some((shown, fading)) = self.wallpaper.as_ref().map(|w| w.buffers()) else {
            return;
        };
        let panes = self.water.panes.borrow();
        let draws = |out: &Output| {
            panes
                .get(&(out.name(), Pane::Desktop))
                .is_some_and(|p| p.node.is_some() && !p.failed)
        };
        let mut every = !shown.is_empty();
        for (out, buffer) in &shown {
            let water = draws(out);
            every &= water;
            show(&mut self.scene, *buffer, !water);
        }
        for b in fading {
            show(&mut self.scene, b, !every);
        }
    }

    /// Before `output`'s frame is built: show the wallpaper again under a pane that failed and note
    /// what covers the desktop. The frame's end asks for the next step's (`WaterCtx::next_step`).
    pub fn water_frame(&mut self, output: &Output) {
        if !self.water_on() {
            return;
        }
        if self.water.fell.take() {
            self.water_buffers();
            crate::lockview::refit(self);
            self.redraw_all();
        }
        let b = self.layout_box(Some(output));
        let covered = self.is_locked() || crate::saverview::running(self) || self.fullscreen_on(b);
        self.water.prepare(output, covered, Instant::now());
    }

    /// Hand `pane` of `out` the picture it draws, composed at the output's size (imported at its
    /// next step). With `fading_in` the picture showing stays under it until `water_fade` reaches 1.
    pub fn water_wallpaper(&mut self, out: &Output, pane: Pane, canvas: &Canvas, fading_in: bool) {
        {
            let mut panes = self.water.panes.borrow_mut();
            let Some(p) = panes.get_mut(&(out.name(), pane)) else {
                return;
            };
            if fading_in {
                if let Some(t) = p.texture.take() {
                    p.previous = Some(t);
                }
                p.fade = 0.0;
            } else {
                p.previous = None;
                p.fade = 1.0;
            }
            p.pending = Some(canvas.clone());
            p.solid = None;
            p.bump();
        }
        crate::backend::schedule(self, out);
    }

    /// The slideshow's fade stands at `t` (0..1): the new picture over the old one at that opacity
    /// on every desktop pane; at 1 the old goes.
    pub fn water_fade(&mut self, t: f32) {
        for ((_, pane), p) in self.water.panes.borrow_mut().iter_mut() {
            if *pane != Pane::Desktop || p.previous.is_none() {
                continue;
            }
            p.fade = t.clamp(0.0, 1.0);
            if p.fade >= 1.0 {
                p.previous = None;
            }
            p.bump();
        }
    }

    /// The pointer moved from `from` to `to` (layout): a wake in the water under it when it moves
    /// nothing else over bare desktop. The only pointer-motion hook, the lock screen's included;
    /// warps and tablet jumps lay no wake (`reprisekit::water::WAKE_JUMP`).
    pub fn water_pointer(&mut self, from: Point<f64, Logical>, to: Point<f64, Logical>) {
        if !self.water_on() {
            return;
        }
        if self.is_locked() {
            if self.lock_splash_at(to).is_some() {
                self.water_wake(Pane::Lock, from, to);
            }
            return;
        }
        if !self.pointer_free() || !matches!(self.scene_hit(to), Hit::Desktop { index: None }) {
            return;
        }
        if self.fluid_on() {
            self.stir_hover(to);
        } else {
            self.water_wake(Pane::Desktop, from, to);
        }
    }

    /// A press on the lock screen at `at` (layout): a click in the lock water there unless the
    /// dialog is over it.
    pub fn water_lock_click(&mut self, at: Point<f64, Logical>) {
        if !self.water_on() {
            return;
        }
        let Some(out) = self.lock_splash_at(at) else {
            return;
        };
        let b = self.layout_box(Some(&out));
        let scale = out.current_scale().fractional_scale();
        let local = (at.x - f64::from(b.x), at.y - f64::from(b.y));
        self.water.splash(
            &out.name(),
            Pane::Lock,
            reprisekit::water::click(local, scale),
        );
        crate::backend::schedule(self, &out);
    }

    fn lock_splash_at(&self, at: Point<f64, Logical>) -> Option<Output> {
        let l = self.lock.active.as_ref()?;
        let out = self.output_at(at.x, at.y)?;
        let lo = l.outputs.iter().find(|lo| lo.output == out)?;
        let dialog = l.dialog_shown().then_some(lo.dialog_rect);
        lock_splash_allowed(dialog, (at.x, at.y)).then_some(out)
    }

    /// Give `out`'s lock cover its water: the lock pane, and its node in the cover's `tree` just
    /// under `wall`, sized to the output. `wall` hides while the pane draws.
    pub fn water_lock_cover(&mut self, out: &Output, tree: TreeId, wall: BufferId) {
        let key = (out.name(), Pane::Lock);
        self.water.add_pane(&key.0, Pane::Lock);
        let b = self.layout_box(Some(out));
        let size = Size::from((b.width, b.height));
        let (node, made, failed) = {
            let panes = self.water.panes.borrow();
            let Some(p) = panes.get(&key) else {
                return;
            };
            (p.node, p.node_size, p.failed)
        };
        match node {
            Some(_) if made == size => {}
            old => {
                if let Some(n) = old {
                    self.scene.destroy(n);
                }
                let n = self.scene.custom_create(
                    tree,
                    Box::new(WaterNode::new(&self.water, key.clone(), size)),
                );
                self.scene.set_input(n, Input::Passthrough);
                self.scene.place_below(n, wall);
                if let Some(p) = self.water.panes.borrow_mut().get_mut(&key) {
                    p.node = Some(n);
                    p.node_size = size;
                }
            }
        }
        show(&mut self.scene, wall, failed);
    }

    /// Take the water off `out`'s lock cover: its pane and node go and `wall` shows.
    pub fn water_lock_uncover(&mut self, out: &Output, wall: BufferId) {
        if let Some(n) = self.water.drop_pane(&out.name(), Pane::Lock) {
            self.scene.destroy(n);
        }
        show(&mut self.scene, wall, true);
    }

    /// A press on bare desktop, or an icon drop there (`water_icon_drop`), at `at`.
    pub fn water_click(&mut self, at: Point<f64, Logical>) {
        let area = Rect::new(at.x.floor() as i32, at.y.floor() as i32, 1, 1);
        self.water_lay(area, false, |o, scale| {
            vec![reprisekit::water::click((at.x - o.0, at.y - o.1), scale)]
        });
    }

    /// Desktop icons dragged with the image now at `to`: the drag's first placement (`from` None)
    /// lets the dragged icons' water move, the rest leave a wake from `from` while `to` is over the
    /// desktop.
    pub fn water_icon_drag(&mut self, from: Option<Point<f64, Logical>>, to: Point<f64, Logical>) {
        if !self.ripples_on() {
            return;
        }
        match from {
            None => self.water_obstacles(),
            Some(from) if self.over_desktop(to) => self.water_wake(Pane::Desktop, from, to),
            Some(_) => {}
        }
    }

    /// An icon drag dropped at `at`: a click in the water there when it fell on the desktop.
    pub fn water_icon_drop(&mut self, at: Point<f64, Logical>) {
        if self.ripples_on() && self.over_desktop(at) {
            self.water_click(at);
        }
    }

    fn over_desktop(&self, at: Point<f64, Logical>) -> bool {
        matches!(self.scene_hit(at), Hit::Desktop { .. })
    }

    /// A window's frame `rect` appearing, going or minimizing: a ring along its edges.
    pub fn water_ring(&mut self, rect: Rect) {
        self.water_lay(rect, false, |o, scale| {
            let r = (
                f64::from(rect.x) - o.0,
                f64::from(rect.y) - o.1,
                f64::from(rect.width),
                f64::from(rect.height),
            );
            reprisekit::water::ring(r, scale).to_vec()
        });
    }

    /// A window being dragged moves away from `from`: remembered until the frame settles
    /// (`water_edge_settle`), however many moves come first.
    pub fn water_edge_moved(&mut self, from: Rect) {
        if self.ripples_on() {
            let mut e = self.water.edge.get();
            e.moved(from);
            self.water.edge.set(e);
        }
    }

    /// The dragged window's frame is `to` for this frame: the wake from where it was at the last
    /// one, over the time since. None (no drag moved it) forgets a move a drag ended with.
    pub fn water_edge_settle(&mut self, to: Option<Rect>) {
        if !self.ripples_on() {
            return;
        }
        let mut e = self.water.edge.get();
        let lay = e.settle(to, Instant::now());
        self.water.edge.set(e);
        if let Some((from, to, dt)) = lay {
            self.water_edge(from, to, dt);
        }
    }

    fn water_edge(&mut self, from: Rect, to: Rect, dt: f32) {
        self.water_lay(to, true, |o, scale| {
            let local = |r: Rect| {
                (
                    f64::from(r.x) - o.0,
                    f64::from(r.y) - o.1,
                    f64::from(r.width),
                    f64::from(r.height),
                )
            };
            reprisekit::water::edge(local(from), local(to), scale, dt)
                .into_iter()
                .collect()
        });
    }

    /// Hold the desktop icons' cells flat in the first output's desktop field, where they lie,
    /// leaving out the icons a placed drag carries; the other outputs' fields get none.
    pub fn water_obstacles(&mut self) {
        if !self.ripples_on() {
            return;
        }
        let first = self.first_output();
        let rects = match (&first, self.desktop.as_ref()) {
            (Some(out), Some(view)) => {
                let dragged: Vec<usize> = self
                    .drags
                    .armed
                    .as_ref()
                    .filter(|d| d.water_last.is_some())
                    .map(|d| {
                        d.sources
                            .iter()
                            .filter_map(|s| match s {
                                DragSource::Icon { id, .. } => {
                                    crate::desktop::find(&view.entries, id)
                                }
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let b = self.layout_box(Some(out));
                let scale = out.current_scale().fractional_scale();
                obstacle_rects(&view.cells, &dragged, (b.x, b.y), scale)
            }
            _ => Vec::new(),
        };
        let first = first.map(|o| o.name());
        for (name, pane) in self.water.pane_keys() {
            if pane != Pane::Desktop {
                continue;
            }
            let field = if Some(&name) == first.as_ref() {
                rects.clone()
            } else {
                Vec::new()
            };
            self.water.set_obstacles(&name, field);
        }
    }

    /// Source `s` goes down at `pos` (layout) on a fluid desktop.
    pub fn stir_down(&mut self, s: Source, pos: Point<f64, Logical>) {
        let Some((out, at)) = self.stir_at(pos) else {
            return;
        };
        let name = out.name();
        if self.water.stir(&name, true, |h, ms| h.down(s, at, ms)) {
            self.water.stirs.borrow_mut().insert(s, name);
            crate::backend::schedule(self, &out);
        }
    }

    /// Held source `s` moves to `pos` (layout), stirring the pane it went down on.
    pub fn stir_move(&mut self, s: Source, pos: Point<f64, Logical>) {
        let Some(out) = self.stirred_output(s) else {
            return;
        };
        let b = self.layout_box(Some(&out));
        let at = local(b, pos);
        if self
            .water
            .stir(&out.name(), true, |h, ms| h.moved(s, at, ms))
        {
            crate::backend::schedule(self, &out);
        }
    }

    /// Held source `s` comes up: a tap bursts, a flick throws.
    pub fn stir_up(&mut self, s: Source) {
        let out = self.stirred_output(s);
        self.water.stirs.borrow_mut().remove(&s);
        let Some(out) = out else {
            return;
        };
        if self.water.stir(&out.name(), true, |h, ms| h.up(s, ms)) {
            crate::backend::schedule(self, &out);
        }
    }

    /// Held source `s` ends without a tap or flick.
    pub fn stir_cancel(&mut self, s: Source) {
        if let Some(out) = self.stirred_output(s) {
            self.water.stir(&out.name(), false, |h, _| h.cancel(s));
        }
        self.water.stirs.borrow_mut().remove(&s);
    }

    /// The pointer moves over bare desktop at `pos` (layout) with no button held.
    pub fn stir_hover(&mut self, pos: Point<f64, Logical>) {
        let Some((out, at)) = self.stir_at(pos) else {
            return;
        };
        if self.water.stir(&out.name(), true, |h, ms| h.hover(at, ms)) {
            crate::backend::schedule(self, &out);
        }
    }

    /// Clear the fluid of the output under `pos` (layout).
    pub fn stir_clear(&mut self, pos: Point<f64, Logical>) {
        let Some((out, _)) = self.stir_at(pos) else {
            return;
        };
        if self.water.stir(&out.name(), true, |h, ms| h.clear(ms)) {
            crate::backend::schedule(self, &out);
        }
    }

    /// Whether a source going down at `pos` (layout) would stir: a fluid desktop pane is there,
    /// drawing and uncovered, and the GPU runs the fluid.
    pub fn stir_takes(&self, pos: Point<f64, Logical>) -> bool {
        self.stir_at(pos)
            .is_some_and(|(out, _)| self.water.takes_stir(&out.name()))
    }

    fn stir_at(&self, pos: Point<f64, Logical>) -> Option<(Output, (f32, f32))> {
        if !self.fluid_on() {
            return None;
        }
        let out = self.output_at(pos.x, pos.y)?;
        let at = local(self.layout_box(Some(&out)), pos);
        Some((out, at))
    }

    fn stirred_output(&self, s: Source) -> Option<Output> {
        let stirs = self.water.stirs.borrow();
        let name = stirs.get(&s)?;
        self.scene
            .outputs()
            .iter()
            .find(|o| o.name() == *name)
            .cloned()
    }

    fn water_wake(&mut self, pane: Pane, from: Point<f64, Logical>, to: Point<f64, Logical>) {
        let Some(out) = self.output_at(to.x, to.y) else {
            return;
        };
        let b = self.layout_box(Some(&out));
        let scale = out.current_scale().fractional_scale();
        let origin = (f64::from(b.x), f64::from(b.y));
        if let Some(s) = local_wake(origin, scale, (from.x, from.y), (to.x, to.y)) {
            self.water.splash(&out.name(), pane, s);
            // A scene ask: a pointer frame alone would skip the render.
            crate::backend::schedule(self, &out);
        }
    }

    fn water_lay(&mut self, area: Rect, edge: bool, make: impl Fn((f64, f64), f64) -> Vec<Splash>) {
        if !self.ripples_on() {
            return;
        }
        let outputs: Vec<Output> = self.scene.outputs().to_vec();
        for out in outputs {
            let b = self.layout_box(Some(&out));
            let touch = b.intersect(&area);
            if touch.width <= 0 || touch.height <= 0 {
                continue;
            }
            let scale = out.current_scale().fractional_scale();
            for s in make((f64::from(b.x), f64::from(b.y)), scale) {
                if edge {
                    self.water.splash_edge(&out.name(), s);
                } else {
                    self.water.splash(&out.name(), Pane::Desktop, s);
                }
            }
            crate::backend::schedule(self, &out);
        }
    }
}

/// Every output's frame settles it, so its wake follows the time between frames of any output.
#[derive(Clone, Copy, Debug, Default)]
struct EdgeDrag {
    from: Option<Rect>,
    settled: Option<Instant>,
}

impl EdgeDrag {
    /// The dragged frame moves away from `from`; only the first move since the last frame counts.
    fn moved(&mut self, from: Rect) {
        self.from.get_or_insert(from);
    }

    fn settle(&mut self, to: Option<Rect>, now: Instant) -> Option<(Rect, Rect, f32)> {
        let last = self.settled.replace(now);
        let (from, to) = (self.from.take()?, to?);
        let dt = last.map_or(1.0 / REFERENCE_HZ, |l| {
            now.saturating_duration_since(l).as_secs_f32()
        });
        Some((from, to, dt))
    }
}

/// Only beside the built-in dialog; None (a locker holds the lock or is starting) keeps it still.
fn lock_splash_allowed(dialog: Option<Rect>, at: (f64, f64)) -> bool {
    dialog.is_some_and(|d| !d.contains(at.0.floor() as i32, at.1.floor() as i32))
}

/// `pos` (layout) in logical px of the output at `b`.
fn local(b: Rect, pos: Point<f64, Logical>) -> (f32, f32) {
    (
        (pos.x - f64::from(b.x)) as f32,
        (pos.y - f64::from(b.y)) as f32,
    )
}

fn local_wake(origin: (f64, f64), scale: f64, from: (f64, f64), to: (f64, f64)) -> Option<Splash> {
    reprisekit::water::wake(
        (from.0 - origin.0, from.1 - origin.1),
        (to.0 - origin.0, to.1 - origin.1),
        scale,
    )
}

fn obstacle_rects(
    cells: &[Rect],
    skip: &[usize],
    origin: (i32, i32),
    scale: f64,
) -> Vec<Rectangle<i32, Physical>> {
    cells
        .iter()
        .enumerate()
        .filter(|(i, _)| !skip.contains(i))
        .map(|(_, c)| {
            let local = |x: i32, y: i32| {
                field_point((f64::from(x - origin.0), f64::from(y - origin.1)), scale)
            };
            let (x0, y0) = local(c.x, c.y);
            let (x1, y1) = local(c.right(), c.bottom());
            let (x0, y0) = (x0.floor() as i32, y0.floor() as i32);
            let (x1, y1) = (x1.ceil() as i32, y1.ceil() as i32);
            Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into())
        })
        .collect()
}

fn show(scene: &mut crate::compositor::DeskScene, b: reprisescene::BufferId, on: bool) {
    if scene.enabled(b) != on {
        scene.set_enabled(b, on);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::water::{Splash, Water};

    /// The frame `DP-1`'s water asks for after one begun `covered` or not.
    fn wants(ctx: &WaterCtx, covered: bool) -> Option<Instant> {
        let now = Instant::now();
        ctx.prepare_named("DP-1", covered, now);
        ctx.next_step_named("DP-1", now)
    }

    fn ctx_with(outputs: &[&str]) -> Rc<WaterCtx> {
        let ctx = WaterCtx::new();
        ctx.configure(
            Water::Calm,
            outputs.iter().map(|o| (o.to_string(), Pane::Desktop)),
        );
        ctx
    }

    #[test]
    fn icon_cells_become_field_obstacles() {
        let cells = [Rect::new(10, 10, 75, 75), Rect::new(10, 95, 75, 75)];
        let r = obstacle_rects(&cells, &[], (0, 0), 1.0);
        assert_eq!(
            r[0],
            Rectangle::new((2, 2).into(), (20, 20).into()),
            "rounded outward"
        );
        assert_eq!(r.len(), 2);
        let dragged = obstacle_rects(&cells, &[1], (0, 0), 1.0);
        assert_eq!(dragged.len(), 1, "the dragged icon is no obstacle");
        let scaled = obstacle_rects(&cells, &[], (0, 0), 2.0);
        assert_eq!(scaled[0], Rectangle::new((5, 5).into(), (38, 38).into()));
    }

    #[test]
    fn output_local_points_feed_the_field() {
        let s = local_wake((1920.0, 0.0), 1.0, (1930.0, 10.0), (1970.0, 10.0)).unwrap();
        assert_eq!(s.from, (2.5, 2.5));
        assert_eq!(s.to, (12.5, 2.5));
    }

    #[test]
    fn panes_follow_outputs() {
        let ctx = ctx_with(&["DP-1", "HDMI-A-1"]);
        assert_eq!(ctx.pane_keys().len(), 2);
        ctx.configure(
            Water::Calm,
            [("DP-1".to_string(), Pane::Desktop)].into_iter(),
        );
        assert_eq!(ctx.pane_keys(), vec![("DP-1".to_string(), Pane::Desktop)]);
    }

    #[test]
    fn turning_water_off_drops_every_pane() {
        let ctx = ctx_with(&["DP-1"]);
        ctx.splash("DP-1", Pane::Desktop, Splash::disc((1.0, 1.0), 2.0, -0.5));
        ctx.configure(Water::Off, std::iter::empty());
        assert!(ctx.pane_keys().is_empty());
        assert_eq!(wants(&ctx, false), None, "nothing left to step");
    }

    #[test]
    fn a_splash_asks_for_a_frame() {
        let ctx = ctx_with(&["DP-1"]);
        assert_eq!(wants(&ctx, false), None, "still water asks for nothing");
        ctx.splash("DP-1", Pane::Desktop, Splash::disc((1.0, 1.0), 2.0, -0.5));
        assert!(wants(&ctx, false).is_some());
    }

    #[test]
    fn a_moving_field_asks_for_the_frame_of_its_next_step() {
        let ctx = ctx_with(&["DP-1"]);
        ctx.splash("DP-1", Pane::Desktop, Splash::disc((1.0, 1.0), 2.0, -0.5));
        let t0 = Instant::now();
        {
            let mut panes = ctx.panes.borrow_mut();
            let p = panes.get_mut(&("DP-1".to_string(), Pane::Desktop)).unwrap();
            p.hz = 30.0;
            p.last = Some(t0);
            assert_eq!(p.clock.steps(1.0 / 30.0, 30.0), 1);
        }
        let at = ctx.next_step_named("DP-1", t0).unwrap();
        let wait = at.duration_since(t0).as_secs_f32();
        assert!(
            (wait - 0.5 / 30.0).abs() < 1e-4,
            "a step at 30 Hz is due when the clock owes one, not at the next refresh: {wait}"
        );
    }

    #[test]
    fn rain_alone_asks_for_frames_at_rain_fps_and_a_splash_for_every_step() {
        let ctx = WaterCtx::new();
        ctx.configure(
            Water::Rain,
            [("DP-1".to_string(), Pane::Desktop)].into_iter(),
        );
        let key = ("DP-1".to_string(), Pane::Desktop);
        let t0 = Instant::now();
        let wait = |ctx: &WaterCtx| {
            {
                let mut panes = ctx.panes.borrow_mut();
                let p = panes.get_mut(&key).unwrap();
                p.hz = 60.0;
                p.last = Some(t0);
                p.clock = Clock::default();
            }
            let at = ctx.next_step_named("DP-1", t0).unwrap();
            at.duration_since(t0).as_secs_f32()
        };
        let rain = wait(&ctx);
        assert!(
            (rain - 1.5 / 60.0).abs() < 1e-4,
            "rain's next frame runs two 60 Hz steps: {rain}"
        );
        ctx.splash("DP-1", Pane::Desktop, Splash::disc((1.0, 1.0), 2.0, -0.5));
        let stirred = wait(&ctx);
        assert!(
            (stirred - 0.5 / 60.0).abs() < 1e-4,
            "a splash on the rain asks for every step: {stirred}"
        );
    }

    #[test]
    fn a_window_drag_lays_one_edge_a_frame() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + std::time::Duration::from_millis(ms);
        let r = |x: i32| Rect::new(x, 100, 80, 60);
        let mut e = EdgeDrag::default();
        assert_eq!(e.settle(None, at(0)), None, "no drag, no wake");
        for x in 1..=10 {
            e.moved(r(x - 1));
        }
        let (from, to, dt) = e.settle(Some(r(10)), at(16)).expect("one wake");
        assert_eq!(
            (from, to),
            (r(0), r(10)),
            "from where the last frame left it"
        );
        assert!((dt - 0.016).abs() < 1e-6, "over the time since: {dt}");
        assert_eq!(e.settle(Some(r(10)), at(20)), None, "a frame with no move");
        e.moved(r(10));
        let (_, _, dt) = e.settle(Some(r(12)), at(22)).unwrap();
        assert!((dt - 0.002).abs() < 1e-6, "another output's frame: {dt}");
        e.moved(r(12));
        assert_eq!(e.settle(None, at(30)), None, "a drag that ended");
        assert_eq!(e.settle(Some(r(20)), at(40)), None, "forgets its move");
    }

    #[test]
    fn a_dragged_edge_waits_as_one_until_the_step() {
        let ctx = ctx_with(&["DP-1"]);
        let edge = |x: f32| Splash::segment((x, 0.0), (x, 10.0), 2.0, -0.1);
        for x in 0..5 {
            ctx.splash_edge("DP-1", edge(x as f32));
        }
        let panes = ctx.panes.borrow();
        let q = &panes[&("DP-1".to_string(), Pane::Desktop)].queue;
        assert_eq!(q.len(), 1, "five frames' edges wait as one");
    }

    #[test]
    fn the_lock_dialog_keeps_the_water_out() {
        let dialog = Some(Rect::new(800, 400, 320, 200));
        assert!(
            !lock_splash_allowed(dialog, (900.0, 500.0)),
            "over the dialog"
        );
        assert!(
            lock_splash_allowed(dialog, (100.0, 100.0)),
            "over the wallpaper"
        );
        assert!(
            !lock_splash_allowed(None, (100.0, 100.0)),
            "a locker's own screen has no water"
        );
    }

    #[test]
    fn the_lock_pane_settles_like_calm_under_any_preset() {
        let ctx = WaterCtx::new();
        ctx.configure(
            Water::Lively,
            [("DP-1".to_string(), Pane::Desktop)].into_iter(),
        );
        ctx.add_pane("DP-1", Pane::Lock);
        assert_eq!(
            ctx.preset_for(Pane::Lock).unwrap().damping,
            Water::Calm.preset().unwrap().damping
        );
        assert_eq!(
            ctx.preset_for(Pane::Desktop).unwrap().damping,
            Water::Lively.preset().unwrap().damping
        );
    }

    #[test]
    fn a_lock_pane_outlives_a_refit_and_goes_with_its_cover() {
        let ctx = ctx_with(&["DP-1"]);
        ctx.add_pane("DP-1", Pane::Lock);
        ctx.configure(
            Water::Calm,
            [("DP-1".to_string(), Pane::Desktop)].into_iter(),
        );
        assert!(
            ctx.pane_keys().contains(&("DP-1".to_string(), Pane::Lock)),
            "a slideshow tick or an output change while locked keeps it"
        );
        ctx.drop_pane("DP-1", Pane::Lock);
        assert_eq!(ctx.pane_keys(), vec![("DP-1".to_string(), Pane::Desktop)]);
    }

    fn dp1() -> Key {
        ("DP-1".to_string(), Pane::Desktop)
    }

    fn is_fluid(ctx: &WaterCtx, key: &Key) -> bool {
        matches!(ctx.panes.borrow()[key].sim, Sim::Fluid { .. })
    }

    #[test]
    fn a_fluid_style_ripples_the_lock_pane_as_calm() {
        let ctx = WaterCtx::new();
        ctx.configure(Water::Ink, [dp1()].into_iter());
        ctx.add_pane("DP-1", Pane::Lock);
        assert!(is_fluid(&ctx, &dp1()));
        assert!(!is_fluid(&ctx, &("DP-1".to_string(), Pane::Lock)));
        assert_eq!(ctx.preset_for(Pane::Lock), Water::Calm.preset());
        assert_eq!(ctx.preset_for(Pane::Desktop), None);
    }

    #[test]
    fn switching_between_ripples_and_fluid_remakes_the_desktop_panes() {
        let ctx = ctx_with(&["DP-1"]);
        ctx.add_pane("DP-1", Pane::Lock);
        ctx.configure(Water::Glow, [dp1()].into_iter());
        assert!(is_fluid(&ctx, &dp1()), "ripples to fluid");
        assert_eq!(ctx.pane_keys().len(), 2, "the lock pane stays");
        ctx.stir("DP-1", true, |h, ms| {
            h.down(Source::Pointer, (5.0, 5.0), ms)
        });
        ctx.configure(Water::Neon, [dp1()].into_iter());
        assert!(
            !ctx.panes.borrow()[&dp1()].running,
            "another profile starts from empty"
        );
        ctx.configure(Water::Lively, [dp1()].into_iter());
        assert!(!is_fluid(&ctx, &dp1()), "fluid to ripples");
    }

    #[test]
    fn a_stir_starts_a_stopped_fluid_pane() {
        let ctx = WaterCtx::new();
        ctx.configure(Water::Sparks, [dp1()].into_iter());
        assert_eq!(wants(&ctx, false), None, "an empty fluid asks for nothing");
        assert!(ctx.stir("DP-1", true, |h, ms| h.hover((5.0, 5.0), ms)));
        assert!(wants(&ctx, false).is_some());
    }

    #[test]
    fn a_cancel_is_no_input() {
        let ctx = WaterCtx::new();
        ctx.configure(Water::Sparks, [dp1()].into_iter());
        assert!(ctx.stir("DP-1", false, |h, _| h.cancel(Source::Pointer)));
        assert_eq!(wants(&ctx, false), None);
    }

    #[test]
    fn a_covered_fluid_pane_lets_go_and_drops_stirs() {
        let ctx = WaterCtx::new();
        ctx.configure(Water::Ink, [dp1()].into_iter());
        ctx.stir("DP-1", true, |h, ms| {
            h.down(Source::Finger(0), (5.0, 5.0), ms)
        });
        wants(&ctx, true);
        let active = |ctx: &WaterCtx| match &ctx.panes.borrow()[&dp1()].sim {
            Sim::Fluid { hands, .. } => hands.active(),
            Sim::Ripples(_) => unreachable!(),
        };
        assert!(!active(&ctx), "held sources are cancelled");
        assert!(!ctx.stir("DP-1", true, |h, ms| h.hover((1.0, 1.0), ms)));
        assert!(!active(&ctx), "no stir while covered");
    }

    #[test]
    fn a_fluid_pane_without_the_programs_or_failed_takes_no_stir() {
        let ctx = WaterCtx::new();
        ctx.configure(Water::Ink, [dp1()].into_iter());
        assert!(!ctx.takes_stir("DP-1"), "no FluidFx: the marquee stays");
        ctx.panes.borrow_mut().get_mut(&dp1()).unwrap().failed = true;
        assert!(!ctx.takes_stir("DP-1"), "a failed pane");
    }

    #[test]
    fn a_held_source_keeps_the_fluid_from_fading() {
        let now = Instant::now();
        let long_ago = now - Duration::from_secs(200);
        let mut last = Some(long_ago);
        assert_eq!(idle_s(&mut last, true, now), 0.0);
        assert_eq!(last, Some(now), "a hold counts as input now");
        let mut last = Some(long_ago);
        assert_eq!(idle_s(&mut last, false, now), 200.0);
        assert_eq!(idle_s(&mut None, false, now), f32::INFINITY);
    }

    #[test]
    fn splashes_pass_fluid_panes_by() {
        let ctx = WaterCtx::new();
        ctx.configure(Water::Glow, [dp1()].into_iter());
        ctx.splash("DP-1", Pane::Desktop, Splash::disc((1.0, 1.0), 2.0, -0.5));
        ctx.splash_edge("DP-1", Splash::disc((1.0, 1.0), 2.0, -0.5));
        assert_eq!(wants(&ctx, false), None);
    }

    #[test]
    fn ink_alternates_its_flavour_and_the_others_keep_one() {
        assert_eq!(next_flavour(Profile::Ink, 0), 1);
        assert_eq!(next_flavour(Profile::Ink, 1), 0);
        for p in [Profile::Glow, Profile::Sparks, Profile::Neon] {
            assert_eq!(next_flavour(p, 0), 0, "{p:?}");
        }
    }

    #[test]
    fn rain_waits_for_the_pane_size() {
        let ctx = WaterCtx::new();
        ctx.configure(
            Water::Rain,
            [("DP-1".to_string(), Pane::Desktop)].into_iter(),
        );
        let key = ("DP-1".to_string(), Pane::Desktop);
        assert!(wants(&ctx, false).is_some(), "rain asks the first step");
        assert!(
            ctx.panes.borrow()[&key].queue.is_empty(),
            "no drop before the first step sizes the pane"
        );
        ctx.panes.borrow_mut().get_mut(&key).unwrap().size = Size::from((1920, 1080));
        wants(&ctx, false);
        assert_eq!(
            ctx.panes.borrow()[&key].queue.len(),
            1,
            "the first drop falls once it has a size"
        );
    }

    #[test]
    fn rain_pauses_when_covered() {
        let ctx = WaterCtx::new();
        ctx.configure(
            Water::Rain,
            [("DP-1".to_string(), Pane::Desktop)].into_iter(),
        );
        assert!(
            wants(&ctx, false).is_some(),
            "rain keeps the desktop moving"
        );
        assert_eq!(
            wants(&ctx, true),
            None,
            "but not under a fullscreen window, drop queued or not"
        );
        ctx.splash("DP-1", Pane::Desktop, Splash::disc((1.0, 1.0), 2.0, -0.5));
        assert_eq!(wants(&ctx, true), None, "nor for a splash while covered");
    }
}
