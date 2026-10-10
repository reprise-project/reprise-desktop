//! Desktop water's glue: a field per output and lock cover, stepped before each frame.
//!
//! A field steps only while it moves: once its [`Energy`] is idle it is flattened and left alone.
//! A covered desktop pane (fullscreen, lock, saver) steps nothing and gets no rain. A pane whose
//! GL work failed draws nothing and its wallpaper buffers show again.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use reprisefx::water::{Pool, WaterFx};
use reprisekit::water::{
    Clock, Energy, Preset, Queue, REFERENCE_HZ, Rain, Rate, SPLASHES_PER_STEP, Splash, Step, Water,
    field_point, field_size,
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
use crate::input::Hit;
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
    panes: RefCell<HashMap<Key, PaneState>>,
    preset: Cell<Option<Preset>>,
    rate: Cell<Rate>,
    /// A pane failed since `water_frame` last looked: its buffers show again.
    fell: Cell<bool>,
    /// The dragged window's moves since the last frame.
    edge: Cell<EdgeDrag>,
}

struct PaneState {
    pool: Pool,
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
    /// Covered as of the last `wants_frame`: frames drawn meanwhile step nothing.
    covered: bool,
    /// The last step; None steps 0 s next (just made, or uncovered).
    last: Option<Instant>,
    running: bool,
    size: Size<i32, Physical>,
    node: Option<CustomId>,
    node_size: Size<i32, Logical>,
    failed: bool,
}

impl PaneState {
    fn new(seed: u32) -> PaneState {
        PaneState {
            pool: Pool::new(),
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
            covered: false,
            running: false,
            size: Size::default(),
            node: None,
            node_size: Size::default(),
            failed: false,
        }
    }

    fn bump(&mut self) {
        self.wall_commit.increment();
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
        let (w, h) = field_size((self.size.w, self.size.h));
        self.pool.fit(r, fx, (w, h).into())?;
        if let Some(c) = self.pending.take() {
            self.texture = Some(import(r, &c)?);
            self.bump();
        }
        if self.covered {
            return Ok(());
        }
        let dt = self
            .last
            .map_or(0.0, |l| now.saturating_duration_since(l).as_secs_f32());
        self.last = Some(now);
        let n = if self.running {
            self.clock.steps(dt, hz)
        } else {
            0
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
            self.pool.step(r, fx, &splashes, &self.obstacles, step)?;
            self.energy.step(step.damping);
        }
        if n > 0 {
            self.pool.shade(r, fx, SLOPE_GAIN)?;
        }
        if self.running && self.energy.idle() && self.queue.is_empty() && !rains {
            self.pool.flatten(r, fx)?;
            self.running = false;
        }
        Ok(())
    }
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
    }

    /// Store `preset` and keep a desktop pane for each of `desktops` (desktop keys), dropping the
    /// other desktop panes; lock panes stay. None drops every pane. Returns the dropped nodes.
    pub fn configure(
        &self,
        preset: Option<Preset>,
        desktops: impl Iterator<Item = (String, Pane)>,
    ) -> Vec<CustomId> {
        self.preset.set(preset);
        let mut panes = self.panes.borrow_mut();
        if preset.is_none() {
            return panes.drain().filter_map(|(_, p)| p.node).collect();
        }
        let keep: Vec<Key> = desktops.filter(|(_, p)| *p == Pane::Desktop).collect();
        let mut dropped = Vec::new();
        panes.retain(|k, p| {
            let stays = k.1 != Pane::Desktop || keep.contains(k);
            if !stays {
                dropped.extend(p.node);
            }
            stays
        });
        for k in keep {
            let n = panes.len();
            panes.entry(k).or_insert_with(|| PaneState::new(seed(n)));
        }
        dropped
    }

    /// Make `pane` of `output_name` unless it exists. The lock screen's way in: `configure` makes
    /// desktop panes only.
    pub fn add_pane(&self, output_name: &str, pane: Pane) {
        let mut panes = self.panes.borrow_mut();
        let n = panes.len();
        panes
            .entry((output_name.to_string(), pane))
            .or_insert_with(|| PaneState::new(seed(n)));
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

    /// The lock screen always ripples with Calm's damping, whatever the preset.
    pub fn damping_for(&self, pane: Pane) -> f32 {
        let calm = Water::Calm.preset().expect("Calm has a preset").damping;
        match pane {
            Pane::Desktop => self.preset.get().map_or(calm, |p| p.damping),
            Pane::Lock => calm,
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
        {
            if !p.running {
                p.last = None;
            }
            push(&mut p.queue);
            p.running = true;
        }
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

    /// Whether a pane of `output` moves and wants the next frame. An uncovered desktop pane under
    /// Rain gets its due drops here.
    pub fn wants_frame(&self, output: &Output, covered: bool, now: Instant) -> bool {
        self.wants(&output.name(), covered, now)
    }

    fn wants(&self, name: &str, covered: bool, now: Instant) -> bool {
        let rain = self.preset.get().and_then(|p| p.rain);
        let mut want = false;
        for ((n, pane), p) in self.panes.borrow_mut().iter_mut() {
            if n != name || p.failed {
                continue;
            }
            p.covered = *pane == Pane::Desktop && covered;
            if p.covered {
                p.last = None;
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
            want |=
                !p.queue.is_empty() || p.running || (p.previous.is_some() && p.fade < 1.0) || rains;
        }
        want
    }

    #[cfg(test)]
    fn wants_frame_named(&self, name: &str, covered: bool) -> bool {
        self.wants(name, covered, Instant::now())
    }

    /// Step `output`'s fields for the frame about to be built. True when a pane failed: its node
    /// draws nothing from now on.
    pub fn step(&self, renderer: &mut GlesRenderer, output: &Output) -> bool {
        let Some(preset) = self.preset.get() else {
            return false;
        };
        let size = output
            .current_mode()
            .map(|m| output.current_transform().transform_size(m.size))
            .unwrap_or_default();
        let name = output.name();
        let hz = self.rate.get().hz(refresh_hz(output));
        let now = Instant::now();
        let fx = self.fx.borrow();
        let mut fell = false;
        for ((n, pane), p) in self.panes.borrow_mut().iter_mut() {
            if *n != name || p.failed {
                continue;
            }
            p.size = size;
            let rains = *pane == Pane::Desktop && preset.rain.is_some();
            let Some(fx) = fx.as_ref() else {
                p.failed = true;
                fell = true;
                continue;
            };
            let preset = Preset {
                damping: self.damping_for(*pane),
                ..preset
            };
            if let Err(e) = p.advance(renderer, fx, &preset, hz, rains, now) {
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
        let fx = self.ctx.fx.borrow();
        let (Some(fx), Some(preset)) = (fx.as_ref(), self.ctx.preset.get()) else {
            return Vec::new();
        };
        let panes = self.ctx.panes.borrow();
        let Some(p) = panes.get(&self.key).filter(|p| !p.failed) else {
            return Vec::new();
        };
        let geometry = Rectangle::new(
            location.to_i32_round(),
            self.size.to_physical_precise_round(scale),
        );
        let element = |id: &Id, t: &GlesTexture, a: f32| {
            p.pool.element(
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
    /// Whether `[wallpaper] water` is on.
    pub fn water_on(&self) -> bool {
        self.config.wallpaper.water.preset().is_some()
    }

    /// Follow `[wallpaper] water` and the outputs: a desktop pane and node per output while on,
    /// none while off, and the wallpaper buffers hidden under every node that draws. Runs on every
    /// `refit_wallpaper` and reload; cheap when nothing changed.
    pub fn water_configured(&mut self) {
        self.water.set_rate(self.config.wallpaper.water_rate);
        let Some(preset) = self.config.wallpaper.water.preset() else {
            if self.water.pane_keys().is_empty() {
                return;
            }
            for node in self.water.configure(None, std::iter::empty()) {
                self.scene.destroy(node);
            }
            self.water_buffers();
            self.redraw_all();
            return;
        };
        let outputs: Vec<Output> = self.scene.outputs().to_vec();
        let keys: Vec<Key> = outputs
            .iter()
            .filter(|o| self.layout_box(Some(o)).width > 0)
            .map(|o| (o.name(), Pane::Desktop))
            .collect();
        let mut changed = false;
        for node in self.water.configure(Some(preset), keys.into_iter()) {
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

    /// Ask for the next frame on `output` while its water moves.
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
        if self.water.wants_frame(output, covered, Instant::now()) {
            crate::backend::schedule(self, output);
        }
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
        self.water_wake(Pane::Desktop, from, to);
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
        if !self.water_on() {
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
        if self.water_on() && self.over_desktop(at) {
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
        if self.water_on() {
            let mut e = self.water.edge.get();
            e.moved(from);
            self.water.edge.set(e);
        }
    }

    /// The dragged window's frame is `to` for this frame: the wake from where it was at the last
    /// one, over the time since. None (no drag moved it) forgets a move a drag ended with.
    pub fn water_edge_settle(&mut self, to: Option<Rect>) {
        if !self.water_on() {
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
        if !self.water_on() {
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
        if !self.water_on() {
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

    fn ctx_with(outputs: &[&str]) -> Rc<WaterCtx> {
        let ctx = WaterCtx::new();
        ctx.configure(
            Water::Calm.preset(),
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
            Water::Calm.preset(),
            [("DP-1".to_string(), Pane::Desktop)].into_iter(),
        );
        assert_eq!(ctx.pane_keys(), vec![("DP-1".to_string(), Pane::Desktop)]);
    }

    #[test]
    fn turning_water_off_drops_every_pane() {
        let ctx = ctx_with(&["DP-1"]);
        ctx.splash("DP-1", Pane::Desktop, Splash::disc((1.0, 1.0), 2.0, -0.5));
        ctx.configure(None, std::iter::empty());
        assert!(ctx.pane_keys().is_empty());
        assert!(
            !ctx.wants_frame_named("DP-1", false),
            "nothing left to step"
        );
    }

    #[test]
    fn a_splash_asks_for_a_frame() {
        let ctx = ctx_with(&["DP-1"]);
        assert!(
            !ctx.wants_frame_named("DP-1", false),
            "still water asks for nothing"
        );
        ctx.splash("DP-1", Pane::Desktop, Splash::disc((1.0, 1.0), 2.0, -0.5));
        assert!(ctx.wants_frame_named("DP-1", false));
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
            Water::Lively.preset(),
            [("DP-1".to_string(), Pane::Desktop)].into_iter(),
        );
        ctx.add_pane("DP-1", Pane::Lock);
        assert_eq!(
            ctx.damping_for(Pane::Lock),
            Water::Calm.preset().unwrap().damping
        );
        assert_eq!(
            ctx.damping_for(Pane::Desktop),
            Water::Lively.preset().unwrap().damping
        );
    }

    #[test]
    fn a_lock_pane_outlives_a_refit_and_goes_with_its_cover() {
        let ctx = ctx_with(&["DP-1"]);
        ctx.add_pane("DP-1", Pane::Lock);
        ctx.configure(
            Water::Calm.preset(),
            [("DP-1".to_string(), Pane::Desktop)].into_iter(),
        );
        assert!(
            ctx.pane_keys().contains(&("DP-1".to_string(), Pane::Lock)),
            "a slideshow tick or an output change while locked keeps it"
        );
        ctx.drop_pane("DP-1", Pane::Lock);
        assert_eq!(ctx.pane_keys(), vec![("DP-1".to_string(), Pane::Desktop)]);
    }

    #[test]
    fn rain_waits_for_the_pane_size() {
        let ctx = WaterCtx::new();
        ctx.configure(
            Water::Rain.preset(),
            [("DP-1".to_string(), Pane::Desktop)].into_iter(),
        );
        let key = ("DP-1".to_string(), Pane::Desktop);
        assert!(
            ctx.wants_frame_named("DP-1", false),
            "rain asks the first step"
        );
        assert!(
            ctx.panes.borrow()[&key].queue.is_empty(),
            "no drop before the first step sizes the pane"
        );
        ctx.panes.borrow_mut().get_mut(&key).unwrap().size = Size::from((1920, 1080));
        ctx.wants_frame_named("DP-1", false);
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
            Water::Rain.preset(),
            [("DP-1".to_string(), Pane::Desktop)].into_iter(),
        );
        assert!(
            ctx.wants_frame_named("DP-1", false),
            "rain keeps the desktop moving"
        );
        assert!(
            !ctx.wants_frame_named("DP-1", true),
            "but not under a fullscreen window, drop queued or not"
        );
        ctx.splash("DP-1", Pane::Desktop, Splash::disc((1.0, 1.0), 2.0, -0.5));
        assert!(
            !ctx.wants_frame_named("DP-1", true),
            "nor for a splash while covered"
        );
    }
}
