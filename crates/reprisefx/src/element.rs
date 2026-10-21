//! The glass render element and the handle a host keeps per piece of glass.

use std::cell::{Cell, RefCell};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::{Arc, mpsc};

use reprisekit::backdrop::Lift;
use reprisekit::glass::tint::{self, Surface};
use reprisekit::glass::{Material, Settings, streak};
use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform,
};
use smithay::backend::renderer::utils::{
    CommitCounter, DamageBag, DamageSet, DamageSnapshot, OpaqueRegions,
};
use smithay::backend::renderer::{Frame, FrameContext, ImportDma, ImportMem, Texture};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Size, Transform};

use crate::blur::{self, Capture, Levels, Plan, Quality, Streaks};
use crate::lens::Lens;
use crate::picture::Source;
use crate::shaders::Programs;
use crate::space::Space;

/// The programs and streak textures shared by every glass element of one renderer. Make it once per `GlesRenderer`; cloning shares it.
#[derive(Clone, Debug)]
pub struct GlassFx(Rc<Shared>);

#[derive(Debug)]
struct Shared {
    programs: Programs,
    /// The streak textures last made, newest first, one per output size, scale and gain.
    streaks: RefCell<Vec<(StreakKey, GlesTexture)>>,
    /// Streak pictures being made on a thread of their own ([`GlassFx::on_streaks`]).
    jobs: RefCell<Vec<(StreakKey, Job)>>,
    wake: RefCell<Option<Wake>>,
    fill: RefCell<Option<Filler>>,
}

#[derive(Clone)]
struct Wake(Arc<dyn Fn() + Send + Sync>);

impl std::fmt::Debug for Wake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Wake")
    }
}

#[derive(Clone)]
struct Filler(Fill);

impl std::fmt::Debug for Filler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Filler")
    }
}

/// Copies a `w`x`h` ARGB8888 picture, rows of `w * 4` bytes, into a dma-buf the renderer can import.
pub type Fill = Arc<dyn Fn(&[u8], (u32, u32)) -> Option<Dmabuf> + Send + Sync>;

#[derive(Debug)]
enum Job {
    Making(mpsc::Receiver<Made>),
    Made(Made),
}

/// A streak picture made on its thread: its bytes, and a dma-buf holding them when one was filled.
#[derive(Debug)]
struct Made {
    bytes: Vec<u8>,
    dmabuf: Option<Dmabuf>,
}

type StreakKey = (Size<i32, Physical>, u64);

const STREAK_TEXTURES: usize = 4;

impl GlassFx {
    /// Compile the shaders on `renderer`; elements made from this draw only with that renderer.
    pub fn new(renderer: &mut GlesRenderer) -> Result<GlassFx, GlesError> {
        Ok(GlassFx(Rc::new(Shared {
            programs: Programs::compile(renderer)?,
            streaks: RefCell::new(Vec::new()),
            jobs: RefCell::new(Vec::new()),
            wake: RefCell::new(None),
            fill: RefCell::new(None),
        })))
    }

    /// Make each streak picture on a thread of its own and call `wake` there once it is made: glass
    /// draws without streaks until a frame after that. Without it they are made in the frame that
    /// first needs them.
    pub fn on_streaks(&self, wake: impl Fn() + Send + Sync + 'static) {
        *self.0.wake.borrow_mut() = Some(Wake(Arc::new(wake)));
    }

    /// Copy each streak picture made on its thread into a dma-buf with `fill` there, which the frame
    /// imports as it stands. A picture `fill` returns None for, or the renderer refuses, is uploaded.
    pub fn fill_streaks(&self, fill: Fill) {
        *self.0.fill.borrow_mut() = Some(Filler(fill));
    }

    /// Whether glass over `output` at `scale` draws streaks now: shown and made. Starts making them
    /// when not.
    pub(crate) fn streaks_ready(
        &self,
        s: &Settings,
        output: Size<i32, Physical>,
        scale: f64,
    ) -> bool {
        if !streak::shown(s) {
            return false;
        }
        let key = (output, scale.to_bits());
        if self.0.streaks.borrow().iter().any(|(k, _)| *k == key) {
            return true;
        }
        let Some(wake) = self.0.wake.borrow().clone() else {
            return true;
        };
        let mut jobs = self.0.jobs.borrow_mut();
        match jobs.iter().position(|(k, _)| *k == key) {
            Some(i) => {
                if let Job::Making(rx) = &jobs[i].1 {
                    match rx.try_recv() {
                        Ok(bytes) => jobs[i].1 = Job::Made(bytes),
                        Err(mpsc::TryRecvError::Empty) => return false,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            log::warn!("glass: the streaks thread stopped; made in the frame");
                            jobs.remove(i);
                        }
                    }
                }
                true
            }
            None => {
                let (tx, rx) = mpsc::channel();
                let px = streak_px(output);
                let fill = self.0.fill.borrow().clone();
                let spawned = std::thread::Builder::new()
                    .name("glass-streaks".into())
                    .spawn(move || {
                        let bytes = streak_bytes(px, scale);
                        let size = (px.0 as u32, px.1 as u32);
                        let dmabuf = fill.and_then(|f| (f.0)(&bytes, size));
                        if tx.send(Made { bytes, dmabuf }).is_ok() {
                            (wake.0)();
                        }
                    });
                match spawned {
                    Ok(_) => {
                        jobs.insert(0, (key, Job::Making(rx)));
                        jobs.truncate(STREAK_TEXTURES);
                        false
                    }
                    Err(e) => {
                        log::warn!("glass: streaks made in the frame, no thread: {e}");
                        true
                    }
                }
            }
        }
    }

    pub(crate) fn blur_program(&self) -> &GlesTexProgram {
        &self.0.programs.blur
    }

    /// The streaks over an output of `output` physical pixels at `scale`, at half resolution
    /// (`streak::pixels`); made on first use and kept. None when `s` shows none.
    pub(crate) fn streaks(
        &self,
        renderer: &mut GlesRenderer,
        s: &Settings,
        output: Size<i32, Physical>,
        scale: f64,
    ) -> Result<Option<GlesTexture>, GlesError> {
        if !streak::shown(s) {
            return Ok(None);
        }
        let key = (output, scale.to_bits());
        let mut cache = self.0.streaks.borrow_mut();
        if let Some(i) = cache.iter().position(|(k, _)| *k == key) {
            let hit = cache.remove(i);
            let t = hit.1.clone();
            cache.insert(0, hit);
            return Ok(Some(t));
        }
        let px = streak_px(output);
        let made = {
            let mut jobs = self.0.jobs.borrow_mut();
            let i = jobs
                .iter()
                .position(|(k, j)| *k == key && matches!(j, Job::Made(_)));
            i.map(|i| jobs.remove(i).1)
        };
        let made = match made {
            Some(Job::Made(made)) => made,
            _ => Made {
                bytes: streak_bytes(px, scale),
                dmabuf: None,
            },
        };
        let imported = made.dmabuf.and_then(|d| {
            renderer
                .import_dmabuf(&d, None)
                .map_err(|e| {
                    log::warn!("glass: streaks uploaded from now on, the dma-buf was refused: {e}");
                    *self.0.fill.borrow_mut() = None;
                })
                .ok()
        });
        let t = match imported {
            Some(t) => t,
            None => renderer.import_memory(&made.bytes, Fourcc::Argb8888, px.into(), false)?,
        };
        cache.insert(0, (key, t.clone()));
        cache.truncate(STREAK_TEXTURES);
        Ok(Some(t))
    }
}

/// The streak picture's size for an output: half its resolution.
fn streak_px(output: Size<i32, Physical>) -> (i32, i32) {
    ((output.w + 1) / 2, (output.h + 1) / 2)
}

/// The streak picture of `px` pixels for an output at `scale`, as ARGB8888 bytes.
fn streak_bytes(px: (i32, i32), scale: f64) -> Vec<u8> {
    let p = streak::picture(px, (scale / 2.0) as f32);
    p.iter().flat_map(|v| v.to_le_bytes()).collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub rect: Rectangle<i32, Physical>,
    /// Corner radii in physical pixels: top-left, top-right, bottom-right, bottom-left.
    pub radii: [f32; 4],
}

impl Shape {
    pub fn rect(rect: Rectangle<i32, Physical>) -> Shape {
        Shape {
            rect,
            radii: [0.0; 4],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GlassFrame<'a> {
    /// The glass, output-local physical pixels. Shapes should not overlap: each draws on its own.
    pub shapes: &'a [Shape],
    pub holes: &'a [Rectangle<i32, Physical>],
    pub settings: &'a Settings,
    /// Whose chrome lies over the glass: a focused or unfocused frame, or the taskbar; the backdrop is lifted for its glass
    /// (`tint::lift`). None draws the blurred backdrop alone.
    pub surface: Option<Surface>,
    /// The output's size in physical pixels and its scale (the streaks are fixed to it).
    pub output: Size<i32, Physical>,
    pub scale: f64,
    pub alpha: f32,
    pub quality: Quality,
    /// Keep the last blur although what lies under the glass changed: `[glass] blur_rate`'s cap is up
    /// ([`Glass::is_stale`]). A move, resize or new blur draws anyway. Only for glass that copies the frame.
    pub hold: bool,
    /// The output's blurred backdrop to draw from (`Picture::source`); None copies and blurs the frame.
    /// With one the shapes must be output-local.
    pub source: Option<&'a Source>,
    /// Output pixels per pixel of the target: 1 on the output, [`blur::backdrop_downscale`] in a
    /// backdrop (glass beneath other glass). Shapes, holes, output and scale stay the output's. Above 1 the
    /// glass needs a source.
    pub downscale: i32,
}

/// The handle a host keeps for one piece of glass (a window's frame, the taskbar). Its element keeps
/// one Id, so the damage tracker keeps its blur from frame to frame.
#[derive(Debug)]
pub struct Glass {
    id: Id,
    /// What changed in the element, local to its geometry: a new [`key`] or a refresh resets it (whole
    /// element), a source's changes under the glass add to it.
    damage: RefCell<DamageBag<i32, Physical>>,
    /// How far this glass caught up with each source's changes: the source's id and its commit.
    seen: RefCell<Vec<(u64, CommitCounter)>>,
    key: Cell<Option<u64>>,
    stale: Rc<Cell<bool>>,
    /// [`Glass::refresh`] asked for a new blur: the next element does not hold.
    refresh: Cell<bool>,
}

impl Default for Glass {
    fn default() -> Glass {
        Glass::new()
    }
}

impl Glass {
    pub fn new() -> Glass {
        Glass {
            id: Id::new(),
            damage: RefCell::new(DamageBag::new(DAMAGE_AGE)),
            seen: RefCell::new(Vec::new()),
            key: Cell::new(None),
            stale: Rc::new(Cell::new(false)),
            refresh: Cell::new(false),
        }
    }

    /// The element for this frame; None when the glass is opaque or has no shapes.
    pub fn element(&self, fx: &GlassFx, f: &GlassFrame<'_>) -> Option<GlassElement> {
        let k = f.downscale.max(1);
        if !f.settings.transparency || f.shapes.is_empty() || (k > 1 && f.source.is_none()) {
            return None;
        }
        let material = f.settings.material;
        let lenses: Vec<Option<Lens>> = f
            .shapes
            .iter()
            .map(|s| Lens::new(material, s.rect, f.output, f.scale))
            .collect();
        let bend = lenses.iter().flatten().map(Lens::reach).max().unwrap_or(0);
        // Glass that blurs the frame reaches past its shapes so the frame there is drawn before it is
        // copied; glass over a source draws only its shapes.
        let reach = match f.source {
            Some(_) => 0,
            None => blur::reach(f.settings.blur, material, f.quality, f.scale) + bend,
        };
        let bounds = f.shapes.iter().map(|s| s.rect).reduce(|a, b| a.merge(b))?;
        if bounds.is_empty() {
            return None;
        }
        let area = Rectangle::new(
            (bounds.loc.x - reach, bounds.loc.y - reach).into(),
            (bounds.size.w + 2 * reach, bounds.size.h + 2 * reach).into(),
        );
        let geometry = shrink(area, k);
        let plan = Plan::new(f.settings.blur, material, f.quality, f.scale);
        let lift = f.surface.map_or(Lift::NONE, |s| tint::lift(f.settings, s));
        let streaks = fx.streaks_ready(f.settings, f.output, f.scale);
        let drawn = key(f, area, plan, lift, &lenses, streaks);
        let hold = f.hold && !self.refresh.replace(false);
        let caught_up = f.source.map(|s| self.catch_up(s, area, k, bend));
        if self.key.get() != Some(drawn) {
            if self.key.get().is_some() {
                self.bump();
            }
            self.key.set(Some(drawn));
        } else if let Some(Some(changed)) = caught_up {
            self.damage.borrow_mut().add(changed);
        } else if let Some(None) = caught_up {
            self.bump();
        }
        Some(GlassElement {
            fx: fx.clone(),
            id: self.id.clone(),
            damage: self.damage.borrow().snapshot(),
            key: drawn,
            geometry,
            downscale: k,
            shapes: f.shapes.to_vec(),
            lenses,
            holes: f.holes.to_vec(),
            lift,
            plan,
            settings: *f.settings,
            streaks,
            output: f.output,
            scale: f.scale,
            alpha: f.alpha,
            hold,
            stale: self.stale.clone(),
            source: f.source.cloned(),
        })
    }

    /// Where `source` changed under glass over `area` (output pixels; its lens reads `bend` past its
    /// shapes) since this glass last drew from it, in target pixels (`downscale` output pixels each)
    /// local to the element's geometry; None when too far behind to say.
    fn catch_up(
        &self,
        source: &Source,
        area: Rectangle<i32, Physical>,
        downscale: i32,
        bend: i32,
    ) -> Option<Vec<Rectangle<i32, Physical>>> {
        let geometry = shrink(area, downscale);
        let now = source.damage.current_commit();
        let mut seen = self.seen.borrow_mut();
        let last = match seen.iter_mut().find(|(id, _)| *id == source.id) {
            Some((_, c)) => std::mem::replace(c, now),
            None => {
                // First drawn from this picture: the element is new to the output's tracker, which draws it whole.
                seen.push((source.id, now));
                return Some(Vec::new());
            }
        };
        let changed = source.damage.damage_since(Some(last))?;
        Some(
            changed
                .iter()
                .filter_map(|r| {
                    let grown = Rectangle::new(
                        (r.loc.x - bend, r.loc.y - bend).into(),
                        (r.size.w + 2 * bend, r.size.h + 2 * bend).into(),
                    );
                    grown.intersection(area)
                })
                .map(|r| shrink(r, downscale))
                .map(|r| Rectangle::new(r.loc - geometry.loc, r.size))
                .collect(),
        )
    }

    fn bump(&self) {
        self.damage.borrow_mut().reset();
    }

    /// Blur again in the next frame, held or not: called when `blur_rate`'s cap lets a held glass catch
    /// up. Schedule a frame on the glass's outputs after it.
    pub fn refresh(&self) {
        self.bump();
        self.refresh.set(true);
    }

    pub fn is_stale(&self) -> bool {
        self.stale.get()
    }
}

/// The target pixels `r` (output pixels) touches when each target pixel is `k` output pixels.
fn shrink(r: Rectangle<i32, Physical>, k: i32) -> Rectangle<i32, Physical> {
    let (x0, y0) = (r.loc.x.div_euclid(k), r.loc.y.div_euclid(k));
    let x1 = (r.loc.x + r.size.w + k - 1).div_euclid(k);
    let y1 = (r.loc.y + r.size.h + k - 1).div_euclid(k);
    Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into())
}

/// The target pixels `r` (output pixels) covers whole when each target pixel is `k` output pixels.
fn shrink_in(r: Rectangle<i32, Physical>, k: i32) -> Rectangle<i32, Physical> {
    let x0 = (r.loc.x + k - 1).div_euclid(k);
    let y0 = (r.loc.y + k - 1).div_euclid(k);
    let (x1, y1) = (
        (r.loc.x + r.size.w).div_euclid(k),
        (r.loc.y + r.size.h).div_euclid(k),
    );
    Rectangle::new((x0, y0).into(), ((x1 - x0).max(0), (y1 - y0).max(0)).into())
}

/// How many changes a damage tracker can catch up on; one further behind draws the element whole.
const DAMAGE_AGE: usize = 8;

/// Everything about a frame's glass that changes its pixels, except where it is: shapes and holes from
/// the element's corner, the lenses (edges on the output's border do not bend), lift, blur, streaks and
/// which source (or the frame) is blurred. The damage tracker sees moves itself.
fn key(
    f: &GlassFrame<'_>,
    geometry: Rectangle<i32, Physical>,
    plan: Plan,
    lift: Lift,
    lenses: &[Option<Lens>],
    streaks: bool,
) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    let local = |r: Rectangle<i32, Physical>| {
        let at = r.loc - geometry.loc;
        [at.x, at.y, r.size.w, r.size.h]
    };
    for s in f.shapes {
        local(s.rect).hash(&mut h);
        s.radii.map(f32::to_bits).hash(&mut h);
    }
    for l in lenses.iter().flatten() {
        [l.band, l.strength, l.rim].map(f32::to_bits).hash(&mut h);
        l.area.map(f32::to_bits).hash(&mut h);
    }
    0xFFu8.hash(&mut h);
    for r in f.holes {
        local(*r).hash(&mut h);
    }
    lift.colour.map(f32::to_bits).hash(&mut h);
    [lift.mean, lift.gain].map(f32::to_bits).hash(&mut h);
    (plan.factor, plan.sigma.to_bits(), plan.pairs.to_bits()).hash(&mut h);
    let s = f.settings;
    (
        streaks,
        s.material,
        f.output.w,
        f.output.h,
        f.scale.to_bits(),
        f.source.map(|s| s.id),
        f.downscale,
    )
        .hash(&mut h);
    h.finish()
}

/// Glass over one piece of chrome on one output: a framebuffer effect that copies what the frame holds
/// under it, blurs it and draws it, lifted for the chrome's glass, into its shapes.
#[derive(Debug, Clone)]
pub struct GlassElement {
    fx: GlassFx,
    id: Id,
    damage: DamageSnapshot<i32, Physical>,
    key: u64,
    /// The shapes' bounds, grown by the blur's reach and the lens's when the glass blurs the frame, in
    /// target pixels.
    geometry: Rectangle<i32, Physical>,
    downscale: i32,
    shapes: Vec<Shape>,
    lenses: Vec<Option<Lens>>,
    holes: Vec<Rectangle<i32, Physical>>,
    lift: Lift,
    plan: Plan,
    settings: Settings,
    /// Whether the streaks are drawn: shown and made ([`GlassFx::streaks_ready`]).
    streaks: bool,
    output: Size<i32, Physical>,
    scale: f64,
    alpha: f32,
    hold: bool,
    stale: Rc<Cell<bool>>,
    source: Option<Source>,
}

impl Element for GlassElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.damage.current_commit()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size(
            (
                f64::from(self.geometry.size.w),
                f64::from(self.geometry.size.h),
            )
                .into(),
        )
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.damage.damage_since(commit).unwrap_or_else(|| {
            DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)])
        })
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }

    /// Glass over a source draws from its picture like any texture; without one it copies the frame beneath it.
    fn is_framebuffer_effect(&self) -> bool {
        self.source.is_none()
    }
}

impl RenderElement<GlesRenderer> for GlassElement {
    fn capture_framebuffer(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), GlesError> {
        // The element may be drawn away from where it was made (a region capture shifts everything):
        // `dst` is where it is in this frame, the shapes keep their place relative to it. The frame lies turned.
        if self.source.is_some() {
            return Ok(());
        }
        let space = Space::new(frame.transformation(), frame.output_size());
        let Some(region) = blur::capture_rect(space.in_fb(dst), space.fb_size(), self.plan.factor)
        else {
            return Ok(());
        };
        let levels = cache.get_or_insert::<RefCell<Levels>, _>(Default::default);
        let mut levels = levels.borrow_mut();
        let drawn = (self.key, self.plan, space);
        if self.hold
            && levels.picture.is_some()
            && levels.region == region
            && levels.drawn == Some(drawn)
        {
            self.stale.set(true);
            return Ok(());
        }
        let streaks = {
            let mut guard = frame.renderer();
            let renderer: &mut GlesRenderer = guard.as_mut();
            let streaks = if self.streaks {
                (self.fx).streaks(renderer, &self.settings, self.output, self.scale)?
            } else {
                None
            };
            levels.prepare(renderer, region, self.plan)?;
            streaks
        };
        // The streaks are fixed to the output: the part the capture shows, from where the element was made.
        let shown = space.in_frame(region);
        let on_output = Rectangle::new(shown.loc - dst.loc + self.geometry.loc, shown.size);
        let c = Capture {
            region,
            plan: self.plan,
            blur: &self.fx.0.programs.blur,
            streaks: streaks.as_ref().map(|t| Streaks {
                texture: t,
                output: self.output,
                shown: on_output,
                transform: space.toward_fb(),
            }),
        };
        levels.drawn = None;
        levels.capture(frame, &c)?;
        levels.drawn = Some(drawn);
        self.stale.set(false);
        Ok(())
    }

    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        // The picture and the output pixels it lies over (`c`): a source's upright over the whole output,
        // a capture's in framebuffer orientation.
        let levels = cache.and_then(|c| c.get::<RefCell<Levels>>());
        let levels = levels.map(RefCell::borrow);
        let (picture, c) = match (&self.source, &levels) {
            (Some(s), _) => (&s.picture, s.extent()),
            (None, Some(l)) => match (&l.picture, l.drawn) {
                (Some(p), Some(_)) => (p, l.region),
                _ => return Ok(()),
            },
            (None, None) => return Ok(()),
        };
        let space = Space::new(frame.transformation(), frame.output_size());
        // `src` is the part of the geometry drawn (one buffer unit per target pixel): a crop moves
        // `dst` by as much, and the shapes stay where they are.
        let cut: Point<i32, Physical> = (src.loc.x.round() as i32, src.loc.y.round() as i32).into();
        let offset = dst.loc - cut - self.geometry.loc;
        let tex = picture.size();
        let kx = f64::from(tex.w) / f64::from(c.size.w);
        let ky = f64::from(tex.h) / f64::from(c.size.h);
        let material = match self.settings.material {
            Material::Frosted => 0.0f32,
            Material::Smoked => 1.0,
            Material::Molten => 2.0,
        };
        let k = self.downscale;
        let holes = self.holes.iter().map(|h| {
            let h = shrink_in(*h, k);
            Rectangle::new(h.loc + offset, h.size)
        });
        for (shape, lens) in self.shapes.iter().zip(&self.lenses) {
            let t = shrink(shape.rect, k);
            let s = Rectangle::new(t.loc + offset, t.size);
            let parts: Vec<Rectangle<i32, Physical>> = damage
                .iter()
                .filter_map(|d| Rectangle::new(d.loc + dst.loc, d.size).intersection(s))
                .collect();
            let parts = Rectangle::subtract_rects_many_in_place(parts, holes.clone());
            let parts: Vec<Rectangle<i32, Physical>> = parts
                .into_iter()
                .filter(|r| !r.is_empty())
                .map(|r| Rectangle::new(r.loc - s.loc, r.size))
                .collect();
            if parts.is_empty() {
                continue;
            }
            // The shape where the picture holds it, the output pixels the drawn target pixels span
            // (the shape's unless shrunk), and the space its distance field runs in (the shader's
            // coordinates run along the picture).
            let (b, span, turn, radii) = match self.source {
                Some(_) => (
                    shape.rect,
                    Rectangle::new(t.loc.upscale(k), t.size.upscale(k)),
                    Transform::Normal,
                    shape.radii,
                ),
                None => {
                    let b = space.in_fb(s);
                    (b, b, space.upright(), space.radii_in_fb(shape.radii))
                }
            };
            let src = Rectangle::new(
                (
                    f64::from(span.loc.x - c.loc.x) * kx,
                    f64::from(span.loc.y - c.loc.y) * ky,
                )
                    .into(),
                (f64::from(span.size.w) * kx, f64::from(span.size.h) * ky).into(),
            );
            let uniforms = [
                Uniform::new(
                    "cap",
                    [
                        (c.loc.x - b.loc.x) as f32,
                        (c.loc.y - b.loc.y) as f32,
                        c.size.w as f32,
                        c.size.h as f32,
                    ],
                ),
                Uniform::new("size", [b.size.w as f32, b.size.h as f32]),
                Uniform::new("radii", radii),
                Uniform::new("colour", self.lift.colour),
                Uniform::new("lift", [self.lift.mean, self.lift.gain]),
                Uniform::new("material", material),
                Uniform::new(
                    "lens",
                    lens.map_or([0.0, 0.0, 0.0, 1.0], |l| {
                        [l.band, l.strength, l.rim, l.scale]
                    }),
                ),
                Uniform::new("lens_box", lens.map_or([0.0; 4], |l| l.area)),
            ];
            frame.render_texture_from_to(
                picture,
                src,
                s,
                &parts,
                &[],
                turn,
                self.alpha,
                Some(&self.fx.0.programs.glass),
                &uniforms,
            )?;
        }
        Ok(())
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
